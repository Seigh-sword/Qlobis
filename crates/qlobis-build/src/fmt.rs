//! Source formatter: canonical formatting driven by the lexer's token
//! stream (stable layout, standard spacing, two-space indentation).

use qlobis_ast::Diagnostics;
use qlobis_lexer::{lex, StrRawPart, Tok};

/// Format one source file. Lexical errors are reported and the input is
/// returned unchanged.
pub fn format_source(src: &str) -> Result<String, String> {
    let mut diags = Diagnostics::new();
    let res = lex(0, src, 0, &mut diags);
    if diags.has_errors() {
        return Err("cannot format: lexical errors present".to_string());
    }
    let mut out = String::new();
    let mut indent = 0usize;
    let mut line_start = true;
    let mut prev: Option<Tok> = None;
    let mut prev_end: usize = 0;

    for t in &res.tokens {
        if matches!(t.tok, Tok::Eof) {
            continue;
        }
        let text = tok_text(&t.tok, src);
        let start = t.span.start as usize;

        // A newline in the gap forces a line break (semicolons optional).
        let gap = &src[prev_end.min(src.len())..start.min(src.len())];
        let had_newline = gap.contains('\n');

        if !line_start {
            if had_newline {
                out.push('\n');
                line_start = true;
            } else if space_between(prev.as_ref(), &t.tok) {
                out.push(' ');
            }
        }
        if line_start {
            // Close braces sit one level out.
            if matches!(t.tok, Tok::RBrace) {
                for _ in 0..indent.saturating_sub(1) {
                    out.push_str("  ");
                }
            } else {
                for _ in 0..indent {
                    out.push_str("  ");
                }
            }
            line_start = false;
        }
        out.push_str(&text);
        prev_end = t.span.end as usize;

        match &t.tok {
            Tok::LBrace => {
                indent += 1;
                out.push('\n');
                line_start = true;
            }
            Tok::RBrace => {
                indent = indent.saturating_sub(1);
                out.push('\n');
                line_start = true;
            }
            Tok::Semi => {
                out.push('\n');
                line_start = true;
            }
            _ => {}
        }
        prev = Some(t.tok.clone());
    }
    // Trailing newline, collapse runs of blank lines.
    let mut result = String::new();
    let mut blanks = 0;
    for l in out.lines() {
        if l.trim().is_empty() {
            blanks += 1;
            if blanks > 1 {
                continue;
            }
        } else {
            blanks = 0;
        }
        result.push_str(l.trim_end());
        result.push('\n');
    }
    Ok(result)
}

fn tok_text(tok: &Tok, src: &str) -> String {
    match tok {
        Tok::Int(v) => format!("{}", v),
        Tok::Float(v) => format!("{}", v),
        Tok::Str(parts) => {
            let mut s = String::from("\"");
            for p in parts {
                match p {
                    StrRawPart::Lit(l) => s.push_str(&escape(l)),
                    StrRawPart::Interp { src: inner, .. } => {
                        s.push('{');
                        s.push_str(inner.trim());
                        s.push('}');
                    }
                }
            }
            s.push('"');
            s
        }
        Tok::Ident(name) => name.clone(),
        other => keyword_text(other).to_string(),
    }
}

fn escape(s: &str) -> String {
    let mut out = String::new();
    for c in s.chars() {
        match c {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '{' => out.push_str("\\{"),
            '}' => out.push_str("\\}"),
            '\n' => out.push_str("\\n"),
            '\t' => out.push_str("\\t"),
            '\r' => out.push_str("\\r"),
            c => out.push(c),
        }
    }
    out
}

fn keyword_text(tok: &Tok) -> &'static str {
    match tok {
        Tok::Int(_) | Tok::Float(_) | Tok::Str(_) | Tok::Ident(_) => "",
        Tok::Defin => "defin",
        Tok::Let => "let",
        Tok::Struct => "struct",
        Tok::Trait => "trait",
        Tok::Impl => "impl",
        Tok::For => "for",
        Tok::In => "in",
        Tok::While => "while",
        Tok::If => "if",
        Tok::Else => "else",
        Tok::Return => "return",
        Tok::Try => "try",
        Tok::Throw => "throw",
        Tok::Break => "break",
        Tok::Continue => "continue",
        Tok::True => "true",
        Tok::False => "false",
        Tok::None => "none",
        Tok::And => "and",
        Tok::Or => "or",
        Tok::Not => "not",
        Tok::Async => "async",
        Tok::Await => "await",
        Tok::SelfVal => "self",
        Tok::At => "@",
        Tok::Dot => ".",
        Tok::Comma => ",",
        Tok::Colon => ":",
        Tok::Semi => ";",
        Tok::LParen => "(",
        Tok::RParen => ")",
        Tok::LBrace => "{",
        Tok::RBrace => "}",
        Tok::LBracket => "[",
        Tok::RBracket => "]",
        Tok::Plus => "+",
        Tok::Minus => "-",
        Tok::Star => "*",
        Tok::Slash => "/",
        Tok::Percent => "%",
        Tok::Assign => "=",
        Tok::Eq => "==",
        Tok::Ne => "!=",
        Tok::Lt => "<",
        Tok::Gt => ">",
        Tok::Le => "<=",
        Tok::Ge => ">=",
        Tok::Amp => "&",
        Tok::Pipe => "|",
        Tok::Caret => "^",
        Tok::Tilde => "~",
        Tok::Shl => "<<",
        Tok::Shr => ">>",
        Tok::Arrow => "->",
        Tok::PlusAssign => "+=",
        Tok::MinusAssign => "-=",
        Tok::StarAssign => "*=",
        Tok::SlashAssign => "/=",
        Tok::PercentAssign => "%=",
        Tok::Bang => "!",
        Tok::Eof => "",
    }
}

fn space_between(prev: Option<&Tok>, next: &Tok) -> bool {
    let Some(p) = prev else {
        return false;
    };
    // No space after these.
    if matches!(p, Tok::At | Tok::Dot | Tok::LParen | Tok::LBracket) {
        return false;
    }
    // No space before these.
    if matches!(
        next,
        Tok::Dot | Tok::Comma | Tok::Semi | Tok::RParen | Tok::RBracket | Tok::LParen | Tok::LBracket
    ) {
        // Calls and indexing: `f(...)` / `xs[i]` — no space after the callee.
        return matches!(next, Tok::LParen | Tok::LBracket)
            && !matches!(p, Tok::Ident(_) | Tok::RParen | Tok::RBracket | Tok::RBrace);
    }
    if matches!(next, Tok::Colon) {
        return false;
    }
    if matches!(p, Tok::Bang) {
        return false;
    }
    true
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn formats_basic_program() {
        let out = format_source(
            "defin main(){let x=1\nif x>0{\nreturn\n}\n}\n",
        )
        .unwrap();
        assert!(out.contains("defin main() {"), "{}", out);
        assert!(out.contains("  let x = 1"), "{}", out);
        assert!(out.contains("  if x > 0 {"), "{}", out);
        assert!(out.contains("    return"), "{}", out);
    }

    #[test]
    fn idempotent_on_formatted_input() {
        let once = format_source("defin f(a: int) -> int {\n  return a + 1\n}\n").unwrap();
        let twice = format_source(&once).unwrap();
        assert_eq!(once, twice);
    }
}
