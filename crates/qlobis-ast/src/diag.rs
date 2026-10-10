//! Rust-quality diagnostic rendering: multi-span labels, colored snippets,
//! notes and actionable help (including `did you mean ...?` suggestions).

use crate::{SourceMap, Span};
use std::fmt::Write as _;

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum Severity {
    Error,
    Warning,
    Note,
}

impl Severity {
    fn as_str(self) -> &'static str {
        match self {
            Severity::Error => "error",
            Severity::Warning => "warning",
            Severity::Note => "note",
        }
    }

    fn color(self) -> &'static str {
        match self {
            Severity::Error => RED,
            Severity::Warning => YELLOW,
            Severity::Note => CYAN,
        }
    }
}

/// A labeled source span. The first label of a diagnostic is the primary one.
#[derive(Clone, Debug)]
pub struct Label {
    pub span: Span,
    pub message: String,
    pub primary: bool,
}

impl Label {
    pub fn primary(span: Span, message: impl Into<String>) -> Self {
        Label {
            span,
            message: message.into(),
            primary: true,
        }
    }

    pub fn secondary(span: Span, message: impl Into<String>) -> Self {
        Label {
            span,
            message: message.into(),
            primary: false,
        }
    }
}

/// One compiler diagnostic.
#[derive(Clone, Debug)]
pub struct Diagnostic {
    pub severity: Severity,
    /// Optional stable error code such as `E0201`.
    pub code: Option<&'static str>,
    pub message: String,
    pub labels: Vec<Label>,
    pub notes: Vec<String>,
    pub helps: Vec<String>,
}

impl Diagnostic {
    pub fn error(message: impl Into<String>) -> Self {
        Diagnostic {
            severity: Severity::Error,
            code: None,
            message: message.into(),
            labels: Vec::new(),
            notes: Vec::new(),
            helps: Vec::new(),
        }
    }

    pub fn warning(message: impl Into<String>) -> Self {
        Diagnostic {
            severity: Severity::Warning,
            code: None,
            message: message.into(),
            labels: Vec::new(),
            notes: Vec::new(),
            helps: Vec::new(),
        }
    }

    pub fn code(mut self, code: &'static str) -> Self {
        self.code = Some(code);
        self
    }

    pub fn with_label(mut self, label: Label) -> Self {
        self.labels.push(label);
        self
    }

    pub fn primary(mut self, span: Span, message: impl Into<String>) -> Self {
        self.labels.push(Label::primary(span, message));
        self
    }

    pub fn secondary(mut self, span: Span, message: impl Into<String>) -> Self {
        self.labels.push(Label::secondary(span, message));
        self
    }

    pub fn note(mut self, note: impl Into<String>) -> Self {
        self.notes.push(note.into());
        self
    }

    pub fn help(mut self, help: impl Into<String>) -> Self {
        self.helps.push(help.into());
        self
    }

    /// `help: a similar name exists: \`foo\`` style suggestion.
    pub fn suggest(self, candidate: &str) -> Self {
        self.help(format!("did you mean `{}`?", candidate))
    }

    pub fn is_error(&self) -> bool {
        self.severity == Severity::Error
    }
}

/// An ordered collection of diagnostics.
#[derive(Default, Debug)]
pub struct Diagnostics {
    pub items: Vec<Diagnostic>,
}

impl Diagnostics {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn push(&mut self, d: Diagnostic) {
        self.items.push(d);
    }

    pub fn error_count(&self) -> usize {
        self.items.iter().filter(|d| d.is_error()).count()
    }

    pub fn warning_count(&self) -> usize {
        self.items
            .iter()
            .filter(|d| d.severity == Severity::Warning)
            .count()
    }

    pub fn has_errors(&self) -> bool {
        self.error_count() > 0
    }

    /// Merge another collection into this one.
    pub fn extend(&mut self, other: Diagnostics) {
        self.items.extend(other.items);
    }

    /// Render every diagnostic against `map` (sorted by source position).
    pub fn render(&self, map: &SourceMap, color: bool) -> String {
        let mut items: Vec<&Diagnostic> = self.items.iter().collect();
        items.sort_by_key(|d| {
            d.labels
                .iter()
                .find(|l| l.primary)
                .or_else(|| d.labels.first())
                .map(|l| (l.span.file, l.span.start))
                .unwrap_or((u32::MAX, u32::MAX))
        });
        let mut out = String::new();
        for (i, d) in items.iter().enumerate() {
            if i > 0 {
                out.push('\n');
            }
            out.push_str(&render_diagnostic(d, map, color));
        }
        out
    }
}

// ---------------------------------------------------------------------------
// Coloring
// ---------------------------------------------------------------------------

pub const RED: &str = "\x1b[31m";
pub const YELLOW: &str = "\x1b[33m";
pub const CYAN: &str = "\x1b[36m";
pub const GREEN: &str = "\x1b[32m";
pub const BLUE: &str = "\x1b[34m";
pub const BOLD: &str = "\x1b[1m";
pub const DIM: &str = "\x1b[2m";
pub const RESET: &str = "\x1b[0m";

/// Should colored output be used? Honors `NO_COLOR`, `CLICOLOR_FORCE`, `TERM`.
pub fn colors_enabled() -> bool {
    if std::env::var_os("NO_COLOR").is_some() {
        return false;
    }
    if std::env::var_os("CLICOLOR_FORCE").is_some() {
        return true;
    }
    if std::env::var("TERM").map(|t| t == "dumb").unwrap_or(false) {
        return false;
    }
    true
}

fn paint(color: &str, on: bool, text: &str) -> String {
    if on {
        format!("{color}{text}{RESET}")
    } else {
        text.to_string()
    }
}

// ---------------------------------------------------------------------------
// Rendering
// ---------------------------------------------------------------------------

fn render_diagnostic(d: &Diagnostic, map: &SourceMap, color: bool) -> String {
    let mut out = String::new();
    // Header: `error[E0201]: message`
    let sev = paint(d.severity.color(), color, d.severity.as_str());
    let code = match d.code {
        Some(c) => format!("[{}]", c),
        None => String::new(),
    };
    let _ = writeln!(out, "{}{}: {}", sev, paint(BOLD, color, &code), d.message);

    // Group labels by file, keeping original order within a file.
    let mut by_file: Vec<(u32, Vec<&Label>)> = Vec::new();
    for label in &d.labels {
        match by_file.iter_mut().find(|(f, _)| *f == label.span.file) {
            Some((_, v)) => v.push(label),
            None => by_file.push((label.span.file, vec![label])),
        }
    }

    for (fid, labels) in by_file {
        let file = map.file(fid);
        // Sort labels by position for display.
        let mut labels = labels;
        labels.sort_by_key(|l| (l.span.start, l.span.end));

        // Layout: gutter width based on the largest displayed line number.
        let mut line_entries: Vec<(u32, Vec<&Label>)> = Vec::new();
        for l in &labels {
            let start_lc = file.line_col(l.span.start);
            let end_lc = file.line_col(l.span.end.saturating_sub(1).max(l.span.start));
            for line in start_lc.line..=end_lc.line {
                match line_entries.iter_mut().find(|(ln, _)| *ln == line) {
                    Some((_, v)) => v.push(l),
                    None => line_entries.push((line, vec![l])),
                }
            }
        }
        line_entries.sort_by_key(|(ln, _)| *ln);

        let gutter_w = line_entries
            .last()
            .map(|(ln, _)| ln.to_string().len())
            .unwrap_or(1);
        let gutter_blank = " ".repeat(gutter_w);
        let bar = paint(DIM, color, "|");

        let _ = writeln!(
            out,
            "{} {} {}",
            gutter_blank,
            bar,
            paint(DIM, color, &format!("--> {}", file.name))
        );
        let _ = writeln!(out, "{} {}", gutter_blank, bar);

        // Emit lines; group close consecutive lines but allow gaps.
        let mut prev_line: i64 = -1;
        for (line, line_labels) in &line_entries {
            if prev_line >= 0 && *line as i64 > prev_line + 1 {
                let _ = writeln!(out, "{} {}", paint(DIM, color, "..."), bar);
            }
            let text = file.line_text(*line);
            let ln = paint(DIM, color, &format!("{:>width$}", line, width = gutter_w));
            let _ = writeln!(out, "{} {} {}", ln, bar, text);

            // Caret line: place markers for every label on this line.
            let mut marks: Vec<(usize, usize, bool, &str)> = Vec::new(); // col0, len, primary, msg
            for l in line_labels {
                let s = file.line_col(l.span.start);
                let e = file.line_col(if l.span.end > l.span.start {
                    l.span.end - 1
                } else {
                    l.span.start
                });
                if s.line != *line && e.line != *line {
                    continue;
                }
                let (from, to) = if s.line == *line && e.line == *line {
                    (s.col as usize, e.col as usize)
                } else if s.line == *line {
                    (s.col as usize, text.chars().count().max(s.col as usize))
                } else {
                    (1, e.col as usize)
                };
                let len = to.saturating_sub(from).max(1);
                marks.push((from, len, l.primary, l.message.as_str()));
            }
            if !marks.is_empty() {
                let mut caret = String::new();
                let mut cursor = 1usize;
                for (from, len, primary, msg) in &marks {
                    while cursor < *from {
                        caret.push(' ');
                        cursor += 1;
                    }
                    let ch = if *primary { '^' } else { '-' };
                    let mark_color = if *primary { d.severity.color() } else { BLUE };
                    caret.push_str(&paint(mark_color, color, &ch.to_string().repeat(*len)));
                    cursor += len;
                    if !msg.is_empty() {
                        caret.push(' ');
                        caret.push_str(&paint(mark_color, color, msg));
                        cursor += msg.chars().count() + 1;
                    }
                }
                let _ = writeln!(out, "{} {} {}", gutter_blank, bar, caret);
            }
            prev_line = *line as i64;
        }

        // Extra labels living outside the rendered line range get a summary.
        let _ = writeln!(out, "{} {}", gutter_blank, bar);
    }

    for n in &d.notes {
        let _ = writeln!(out, "  {} {}", paint(CYAN, color, "note:"), n);
    }
    for h in &d.helps {
        let _ = writeln!(out, "  {} {}", paint(GREEN, color, "help:"), h);
    }
    out
}

/// Format `error: ...` style one-liners (used by the CLI for aggregated
/// build-failure summaries).
pub fn render_message(sev: Severity, message: &str, color: bool) -> String {
    format!("{}: {}", paint(sev.color(), color, sev.as_str()), message)
}

/// Levenshtein distance, used for `did you mean` suggestions.
pub fn levenshtein(a: &str, b: &str) -> usize {
    let a: Vec<char> = a.chars().collect();
    let b: Vec<char> = b.chars().collect();
    let mut prev: Vec<usize> = (0..=b.len()).collect();
    let mut cur = vec![0usize; b.len() + 1];
    for (i, ca) in a.iter().enumerate() {
        cur[0] = i + 1;
        for (j, cb) in b.iter().enumerate() {
            let cost = if ca == cb { 0 } else { 1 };
            cur[j + 1] = (prev[j] + cost).min(prev[j + 1] + 1).min(cur[j] + 1);
        }
        std::mem::swap(&mut prev, &mut cur);
    }
    prev[b.len()]
}

/// Pick the closest candidate to `name` within an edit-distance budget.
pub fn suggest_name<'a, I>(name: &str, candidates: I) -> Option<&'a str>
where
    I: IntoIterator<Item = &'a str>,
{
    let mut best: Option<(usize, &str)> = None;
    let budget = (name.chars().count() / 3).max(1) + 1;
    for cand in candidates {
        let d = levenshtein(name, cand);
        if d <= budget && best.map(|(bd, _)| d < bd).unwrap_or(true) {
            best = Some((d, cand));
        }
    }
    best.map(|(_, c)| c)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::LineCol;

    #[test]
    fn line_col_lookup() {
        let mut map = SourceMap::new();
        let id = map.add_file("t.qbs", "abc\ndef\nghi");
        assert_eq!(map.file(id).line_col(0), LineCol { line: 1, col: 1 });
        assert_eq!(map.file(id).line_col(4), LineCol { line: 2, col: 1 });
        assert_eq!(map.file(id).line_col(6), LineCol { line: 2, col: 3 });
    }

    #[test]
    fn render_contains_caret_and_help() {
        let mut map = SourceMap::new();
        let id = map.add_file("demo.qbs", "let x = 1\n");
        let mut ds = Diagnostics::new();
        ds.push(
            Diagnostic::error("cannot find value `y`")
                .code("E0425")
                .primary(Span::new(id, 4, 5), "not found in this scope")
                .help("did you mean `x`?"),
        );
        let out = ds.render(&map, false);
        assert!(out.contains("error[E0425]"));
        assert!(out.contains("demo.qbs"));
        assert!(out.contains('^'));
        assert!(out.contains("help: did you mean `x`?"));
    }

    #[test]
    fn suggest_close_names() {
        let names = ["println", "print_line", "read_file"];
        assert_eq!(
            suggest_name("println", names.iter().copied()),
            Some("println")
        );
        assert_eq!(
            suggest_name("prinln", names.iter().copied()),
            Some("println")
        );
        assert_eq!(suggest_name("zzzzzzzz", names.iter().copied()), None);
    }
}
