//! Language Server Protocol implementation (JSON-RPC over stdio).
//!
//! Supported: `initialize`, `initialized`, `textDocument/didOpen`,
//! `textDocument/didChange`, `textDocument/didSave`, `textDocument/hover`,
//! `textDocument/documentSymbol`, `textDocument/formatting`,
//! `textDocument/publishDiagnostics` (push), `shutdown`, `exit`.
//!
//! The JSON layer is hand-rolled (std-only workspace).

use qlobis_ast::{Diagnostics, SourceMap, Span};
use qlobis_tyck::{check, ModuleInput, Target};
use std::collections::HashMap;
use std::io::{Read, Write};

/// Serve LSP on stdin/stdout until `exit`. Returns the process exit code.
pub fn serve() -> i32 {
    let stdin = std::io::stdin();
    let mut stdin = stdin.lock();
    let mut docs: HashMap<String, String> = HashMap::new();
    let mut exit_code = 0;
    loop {
        match read_message(&mut stdin) {
            None => break,
            Some(msg) => {
                let v = parse_json(&msg).unwrap_or(Json::Null);
                let method = get_str(&v, "method").unwrap_or_default();
                let id = v.get("id").cloned();
                match method.as_str() {
                    "initialize" => {
                        let caps = format!(
                            r#"{{"capabilities":{{"textDocumentSync":1,"hoverProvider":true,"documentSymbolProvider":true,"documentFormattingProvider":true}},"serverInfo":{{"name":"qlobis-lsp","version":"0.1.0"}}}}"#
                        );
                        respond(id, &caps);
                    }
                    "initialized" | "workspace/didChangeConfiguration" => {}
                    "shutdown" => {
                        respond(id, "null");
                    }
                    "exit" => {
                        exit_code = 0;
                        break;
                    }
                    "textDocument/didOpen" => {
                        if let Some(uri) = get_path(&v, "textDocument", "uri") {
                            let text = get_nested_str(&v, "textDocument", "text")
                                .unwrap_or_default();
                            docs.insert(uri.clone(), text.clone());
                            let diags = analyze(&docs, &uri);
                            publish(&uri, &diags);
                        }
                    }
                    "textDocument/didChange" => {
                        if let Some(uri) = get_path(&v, "textDocument", "uri") {
                            if let Some(text) = first_change_text(&v) {
                                docs.insert(uri.clone(), text.clone());
                                let diags = analyze(&docs, &uri);
                                publish(&uri, &diags);
                            }
                        }
                    }
                    "textDocument/didSave" => {
                        if let Some(uri) = get_path(&v, "textDocument", "uri") {
                            let diags = analyze(&docs, &uri);
                            publish(&uri, &diags);
                        }
                    }
                    "textDocument/hover" => {
                        let result = hover(&v, &docs);
                        respond(id, &result);
                    }
                    "textDocument/documentSymbol" => {
                        let result = document_symbols(&v, &docs);
                        respond(id, &result);
                    }
                    "textDocument/formatting" => {
                        let result = formatting(&v, &docs);
                        respond(id, &result);
                    }
                    _ => {
                        if id.is_some() {
                            respond(id, "null");
                        }
                    }
                }
            }
        }
    }
    exit_code
}

// ---------------------------------------------------------------------------
// tiny JSON value
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, PartialEq)]
pub enum Json {
    Null,
    Bool(bool),
    Num(f64),
    Str(String),
    Arr(Vec<Json>),
    Obj(Vec<(String, Json)>),
}

impl Json {
    fn get(&self, key: &str) -> Option<&Json> {
        match self {
            Json::Obj(kvs) => kvs.iter().find(|(k, _)| k == key).map(|(_, v)| v),
            _ => None,
        }
    }
    fn as_str(&self) -> Option<&str> {
        match self {
            Json::Str(s) => Some(s),
            _ => None,
        }
    }
    fn as_arr(&self) -> Option<&[Json]> {
        match self {
            Json::Arr(a) => Some(a),
            _ => None,
        }
    }
}

pub fn parse_json(s: &str) -> Option<Json> {
    let b: Vec<char> = s.chars().collect();
    let mut i = 0;
    let v = parse_value(&b, &mut i)?;
    Some(v)
}

fn skip_ws(b: &[char], i: &mut usize) {
    while *i < b.len() && b[*i].is_whitespace() {
        *i += 1;
    }
}

fn parse_value(b: &[char], i: &mut usize) -> Option<Json> {
    skip_ws(b, i);
    match b.get(*i)? {
        '{' => {
            *i += 1;
            let mut out = Vec::new();
            skip_ws(b, i);
            if b.get(*i) == Some(&'}') {
                *i += 1;
                return Some(Json::Obj(out));
            }
            loop {
                skip_ws(b, i);
                let key = match parse_value(b, i)? {
                    Json::Str(s) => s,
                    _ => return None,
                };
                skip_ws(b, i);
                if b.get(*i) != Some(&':') {
                    return None;
                }
                *i += 1;
                let val = parse_value(b, i)?;
                out.push((key, val));
                skip_ws(b, i);
                match b.get(*i) {
                    Some(',') => {
                        *i += 1;
                    }
                    Some('}') => {
                        *i += 1;
                        return Some(Json::Obj(out));
                    }
                    _ => return None,
                }
            }
        }
        '[' => {
            *i += 1;
            let mut out = Vec::new();
            skip_ws(b, i);
            if b.get(*i) == Some(&']') {
                *i += 1;
                return Some(Json::Arr(out));
            }
            loop {
                out.push(parse_value(b, i)?);
                skip_ws(b, i);
                match b.get(*i) {
                    Some(',') => {
                        *i += 1;
                    }
                    Some(']') => {
                        *i += 1;
                        return Some(Json::Arr(out));
                    }
                    _ => return None,
                }
            }
        }
        '"' => {
            *i += 1;
            let mut s = String::new();
            while *i < b.len() {
                let c = b[*i];
                *i += 1;
                match c {
                    '"' => return Some(Json::Str(s)),
                    '\\' => {
                        let e = *b.get(*i)?;
                        *i += 1;
                        match e {
                            'n' => s.push('\n'),
                            't' => s.push('\t'),
                            'r' => s.push('\r'),
                            '"' => s.push('"'),
                            '\\' => s.push('\\'),
                            '/' => s.push('/'),
                            'u' => {
                                let mut hex = String::new();
                                for _ in 0..4 {
                                    hex.push(*b.get(*i)?);
                                    *i += 1;
                                }
                                let n = u32::from_str_radix(&hex, 16).ok()?;
                                s.push(char::from_u32(n).unwrap_or('\u{fffd}'));
                            }
                            other => s.push(other),
                        }
                    }
                    c => s.push(c),
                }
            }
            None
        }
        't' => {
            *i += 4;
            Some(Json::Bool(true))
        }
        'f' => {
            *i += 5;
            Some(Json::Bool(false))
        }
        'n' => {
            *i += 4;
            Some(Json::Null)
        }
        _ => {
            let start = *i;
            while *i < b.len() && (b[*i].is_ascii_digit() || "+-.eE".contains(b[*i])) {
                *i += 1;
            }
            let text: String = b[start..*i].iter().collect();
            text.parse::<f64>().ok().map(Json::Num)
        }
    }
}

pub fn json_escape(s: &str) -> String {
    let mut out = String::new();
    for c in s.chars() {
        match c {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            c if (c as u32) < 0x20 => out.push_str(&format!("\\u{:04x}", c as u32)),
            c => out.push(c),
        }
    }
    out
}

// ---------------------------------------------------------------------------
// message framing + helpers
// ---------------------------------------------------------------------------

fn read_message(stdin: &mut impl Read) -> Option<String> {
    let mut header = Vec::new();
    let mut byte = [0u8; 1];
    // Read until \r\n\r\n.
    let mut state = 0;
    loop {
        let n = stdin.read(&mut byte).ok()?;
        if n == 0 {
            return None;
        }
        header.push(byte[0]);
        match byte[0] {
            b'\r' if state == 0 || state == 2 => state += 1,
            b'\n' if state == 1 => state += 1,
            b'\r' if state == 3 => state = 1,
            _ => state = 0,
        }
        if state == 3 {
            state = 0;
            // peek for final \n
            break;
        }
    }
    let header = String::from_utf8_lossy(&header).to_string();
    let mut len = 0usize;
    for line in header.lines() {
        if let Some(v) = line.strip_prefix("Content-Length:") {
            len = v.trim().parse().unwrap_or(0);
        }
    }
    if len == 0 {
        return Some(String::new());
    }
    let mut buf = vec![0u8; len];
    stdin.read_exact(&mut buf).ok()?;
    Some(String::from_utf8_lossy(&buf).to_string())
}

fn send(content: &str) {
    let stdout = std::io::stdout();
    let mut out = stdout.lock();
    let _ = write!(out, "Content-Length: {}\r\n\r\n{}", content.len(), content);
    let _ = out.flush();
}

fn respond(id: Option<Json>, result: &str) {
    let id_text = match id {
        Some(Json::Num(n)) => format!("{}", n as i64),
        Some(Json::Str(s)) => format!("\"{}\"", json_escape(&s)),
        _ => "null".to_string(),
    };
    send(&format!(r#"{{"jsonrpc":"2.0","id":{},"result":{}}}"#, id_text, result));
}

fn publish(uri: &str, diagnostics: &str) {
    send(&format!(
        r#"{{"jsonrpc":"2.0","method":"textDocument/publishDiagnostics","params":{{"uri":"{}","diagnostics":{}}}}}"#,
        json_escape(uri),
        diagnostics
    ));
}

fn get_str(v: &Json, key: &str) -> Option<String> {
    v.get(key).and_then(|j| j.as_str()).map(String::from)
}

fn get_path(v: &Json, a: &str, b: &str) -> Option<String> {
    v.get(a).and_then(|j| get_str(j, b))
}

fn get_nested_str(v: &Json, a: &str, b: &str) -> Option<String> {
    get_path(v, a, b)
}

fn first_change_text(v: &Json) -> Option<String> {
    let arr = v.get("params")?.get("contentChanges")?.as_arr()?;
    // Full-document sync: the first change carries the whole text.
    for c in arr {
        if let Some(t) = get_str(c, "text") {
            return Some(t);
        }
    }
    None
}

// ---------------------------------------------------------------------------
// analysis
// ---------------------------------------------------------------------------

fn analyze(docs: &HashMap<String, String>, uri: &str) -> String {
    let empty = String::new();
    let text = docs.get(uri).unwrap_or(&empty);
    let mut map = SourceMap::new();
    map.add_file(uri, text);
    let mut diags = Diagnostics::new();
    let program = qlobis_parser::parse_file(0, text, &mut diags);
    let modules = vec![ModuleInput {
        path: "main".into(),
        file: 0,
        program,
        is_stdlib: false,
    }];
    let _ = check(&modules, "main", &[Target::Python], &mut diags);
    let mut out = String::from("[");
    let mut first = true;
    for d in &diags.items {
        let Some(label) = d.labels.first() else {
            continue;
        };
        let start = map.line_col(label.span);
        let end = map.line_col(Span::new(label.span.file, label.span.end, label.span.end));
        let severity = match d.severity {
            qlobis_ast::Severity::Error => 1,
            qlobis_ast::Severity::Warning => 2,
            _ => 3,
        };
        let mut msg = d.message.clone();
        for h in &d.helps {
            msg.push_str("\nhelp: ");
            msg.push_str(h);
        }
        if !first {
            out.push(',');
        }
        first = false;
        out.push_str(&format!(
            r#"{{"range":{{"start":{{"line":{},"character":{}}},"end":{{"line":{},"character":{}}}}},"severity":{},"source":"qlobis","message":"{}"}}"#,
            start.line.saturating_sub(1),
            start.col.saturating_sub(1),
            end.line.saturating_sub(1),
            end.col.saturating_sub(1),
            severity,
            json_escape(&msg),
        ));
    }
    out.push(']');
    out
}

fn hover(v: &Json, docs: &HashMap<String, String>) -> String {
    let uri = match get_path(v, "params", "uri") {
        Some(u) => u,
        None => return "null".to_string(),
    };
    let text = match docs.get(&uri) {
        Some(t) => t,
        None => return "null".to_string(),
    };
    // Line/char of the hover position.
    let params = match v.get("params") {
        Some(p) => p,
        None => return "null".to_string(),
    };
    let (line, col) = match (params.get("position"),) {
        (Some(pos),) => (
            get_str(pos, "line").and_then(|s| s.parse::<usize>().ok()).or_else(|| {
                match pos.get("line") {
                    Some(Json::Num(n)) => Some(*n as usize),
                    _ => None,
                }
            }),
            match pos.get("character") {
                Some(Json::Num(n)) => Some(*n as usize),
                _ => None,
            },
        ),
        _ => (None, None),
    };
    let (Some(line), Some(col)) = (line, col) else {
        return "null".to_string();
    };
    // Find the identifier under the cursor and describe its checked type.
    let mut map = SourceMap::new();
    map.add_file(&uri, text);
    let offset = line_col_to_offset(text, line, col);
    let word = word_at(text, offset);
    if word.is_empty() {
        return "null".to_string();
    }
    let mut diags = Diagnostics::new();
    let program = qlobis_parser::parse_file(0, text, &mut diags);
    let modules = vec![ModuleInput {
        path: "main".into(),
        file: 0,
        program,
        is_stdlib: false,
    }];
    let checked = check(&modules, "main", &[Target::Python], &mut diags);
    // Report the declaration kind via a simple scan.
    let mut desc = format!("**{}**", word);
    for m in &modules {
        for item in &m.program.items {
            if let qlobis_ast::Item::Func(f) = item {
                if f.name.name == word {
                    let ret = f
                        .ret
                        .as_ref()
                        .map(crate::docs::type_text)
                        .unwrap_or_else(|| "none".into());
                    desc = format!("```qlobis\ndefin {}(...) -> {}\n```", word, ret);
                }
            }
        }
    }
    let _ = &checked;
    format!(
        r#"{{"contents":{{"kind":"markdown","value":"{}"}}}}"#,
        json_escape(&desc)
    )
}

fn document_symbols(v: &Json, docs: &HashMap<String, String>) -> String {
    let uri = match get_path(v, "params", "uri") {
        Some(u) => u,
        None => return "[]".to_string(),
    };
    let text = match docs.get(&uri) {
        Some(t) => t,
        None => return "[]".to_string(),
    };
    let mut diags = Diagnostics::new();
    let program = qlobis_parser::parse_file(0, text, &mut diags);
    let mut out = String::from("[");
    let mut first = true;
    for item in &program.items {
        let (name, kind, span) = match item {
            qlobis_ast::Item::Func(f) => (f.name.name.clone(), 12u32, f.name.span),
            qlobis_ast::Item::Struct(s) => (s.name.name.clone(), 23, s.name.span),
            qlobis_ast::Item::Trait(t) => (t.name.name.clone(), 5, t.name.span),
            qlobis_ast::Item::Impl(_) => continue,
        };
        let start = line_col_of(text, span.start);
        let end = line_col_of(text, span.end);
        if !first {
            out.push(',');
        }
        first = false;
        out.push_str(&format!(
            r#"{{"name":"{}","kind":{},"range":{{"start":{{"line":{},"character":{}}},"end":{{"line":{},"character":{}}}}},"selectionRange":{{"start":{{"line":{},"character":{}}},"end":{{"line":{},"character":{}}}}}}}"#,
            json_escape(&name),
            kind,
            start.0, start.1, end.0, end.1,
            start.0, start.1, end.0, end.1
        ));
    }
    out.push(']');
    out
}

fn formatting(v: &Json, docs: &HashMap<String, String>) -> String {
    let uri = match get_path(v, "params", "uri") {
        Some(u) => u,
        None => return "null".to_string(),
    };
    let text = match docs.get(&uri) {
        Some(t) => t,
        None => return "null".to_string(),
    };
    match crate::fmt::format_source(text) {
        Ok(formatted) => {
            if &formatted == text {
                return "[]".to_string();
            }
            let line_count = text.lines().count();
            format!(
                r#"[{{"range":{{"start":{{"line":0,"character":0}},"end":{{"line":{},"character":0}}}},"newText":"{}"}}]"#,
                line_count,
                json_escape(&formatted)
            )
        }
        Err(_) => "null".to_string(),
    }
}

fn line_col_to_offset(text: &str, line: usize, col: usize) -> usize {
    let mut off = 0;
    for (i, l) in text.lines().enumerate() {
        if i == line {
            return off + col.min(l.len());
        }
        off += l.len() + 1;
    }
    off
}

fn line_col_of(text: &str, offset: u32) -> (usize, usize) {
    let mut off = 0usize;
    for (i, l) in text.lines().enumerate() {
        let end = off + l.len();
        if (offset as usize) <= end {
            return (i, offset as usize - off);
        }
        off = end + 1;
    }
    (0, 0)
}

fn word_at(text: &str, offset: usize) -> String {
    let b: Vec<char> = text.chars().collect();
    let mut start = offset.min(b.len());
    let mut end = start;
    while start > 0 && (b[start - 1].is_alphanumeric() || b[start - 1] == '_') {
        start -= 1;
    }
    while end < b.len() && (b[end].is_alphanumeric() || b[end] == '_') {
        end += 1;
    }
    b[start..end].iter().collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn json_round_trip() {
        let v = parse_json(r#"{"a": [1, 2, "x"], "b": {"c": true}, "d": null}"#).unwrap();
        assert_eq!(get_str(&v, "a"), None);
        assert_eq!(v.get("a").unwrap().as_arr().unwrap().len(), 3);
        assert_eq!(v.get("b").unwrap().get("c"), Some(&Json::Bool(true)));
    }

    #[test]
    fn json_escapes() {
        assert_eq!(json_escape("a\"b\n"), "a\\\"b\\n");
    }

    #[test]
    fn word_lookup() {
        assert_eq!(word_at("let count = 1", 5), "count");
        assert_eq!(word_at("let count = 1", 0), "let");
    }
}
