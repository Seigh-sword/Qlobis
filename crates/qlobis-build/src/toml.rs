//! Hand-rolled TOML-subset parser for `qlobconfig.toml` and
//! `qlobregistry.toml`. Supports:
//!
//! * `[table]` and `[table.sub]` headers
//! * `key = "string"`, `key = 123`, `key = true`, `key = 1.5`
//! * `key = ["a", "b"]` arrays
//! * `key = { k = v, ... }` inline tables
//! * `#` comments

use std::collections::HashMap;

#[derive(Debug, Clone, PartialEq)]
pub enum Value {
    Str(String),
    Int(i64),
    Float(f64),
    Bool(bool),
    Array(Vec<Value>),
    Table(HashMap<String, Value>),
}

impl Value {
    pub fn as_str(&self) -> Option<&str> {
        match self {
            Value::Str(s) => Some(s),
            _ => None,
        }
    }
    pub fn as_int(&self) -> Option<i64> {
        match self {
            Value::Int(i) => Some(*i),
            _ => None,
        }
    }
    pub fn as_bool(&self) -> Option<bool> {
        match self {
            Value::Bool(b) => Some(*b),
            _ => None,
        }
    }
    pub fn as_array(&self) -> Option<&[Value]> {
        match self {
            Value::Array(a) => Some(a),
            _ => None,
        }
    }
    pub fn as_table(&self) -> Option<&HashMap<String, Value>> {
        match self {
            Value::Table(t) => Some(t),
            _ => None,
        }
    }
}

/// A parsed TOML document: nested tables.
pub type Table = HashMap<String, Value>;

pub fn parse_toml(src: &str) -> Result<Table, String> {
    let mut root: Table = Table::new();
    let mut path: Vec<String> = Vec::new();
    for (lineno, raw) in src.lines().enumerate() {
        let line = strip_comment(raw).trim().to_string();
        if line.is_empty() {
            continue;
        }
        if line.starts_with('[') {
            if !line.ends_with(']') {
                return Err(format!("line {}: unterminated table header", lineno + 1));
            }
            let inner = &line[1..line.len() - 1];
            path = inner
                .split('.')
                .map(|s| s.trim().trim_matches('"').to_string())
                .collect();
            // Ensure the table exists.
            let mut cur = &mut root;
            for part in &path {
                let entry = cur
                    .entry(part.clone())
                    .or_insert_with(|| Value::Table(HashMap::new()));
                if !matches!(entry, Value::Table(_)) {
                    *entry = Value::Table(HashMap::new());
                }
                cur = match entry {
                    Value::Table(t) => t,
                    _ => unreachable!(),
                };
            }
            continue;
        }
        let eq = line.find('=').ok_or_else(|| {
            format!("line {}: expected `key = value`", lineno + 1)
        })?;
        let key = line[..eq].trim().trim_matches('"').to_string();
        let value_src = line[eq + 1..].trim();
        let value = parse_value(value_src, lineno + 1)?;
        let mut cur = &mut root;
        for part in &path {
            let entry = cur
                .entry(part.clone())
                .or_insert_with(|| Value::Table(HashMap::new()));
            cur = match entry {
                Value::Table(t) => t,
                _ => return Err(format!("line {}: `{}` is not a table", lineno + 1, part)),
            };
        }
        cur.insert(key, value);
    }
    Ok(root)
}

fn strip_comment(line: &str) -> &str {
    let mut in_str = false;
    for (i, c) in line.char_indices() {
        match c {
            '"' => in_str = !in_str,
            '#' if !in_str => return &line[..i],
            _ => {}
        }
    }
    line
}

fn parse_value(src: &str, lineno: usize) -> Result<Value, String> {
    let s = src.trim();
    if s.is_empty() {
        return Err(format!("line {}: missing value", lineno));
    }
    if s == "true" {
        return Ok(Value::Bool(true));
    }
    if s == "false" {
        return Ok(Value::Bool(false));
    }
    if s.starts_with('"') {
        let end = s.rfind('"').ok_or_else(|| {
            format!("line {}: unterminated string", lineno)
        })?;
        let inner = &s[1..end];
        return Ok(Value::Str(unescape(inner)));
    }
    if s.starts_with('\'') {
        let end = s.rfind('\'').ok_or_else(|| {
            format!("line {}: unterminated string", lineno)
        })?;
        return Ok(Value::Str(s[1..end].to_string()));
    }
    if s.starts_with('[') {
        if !s.ends_with(']') {
            return Err(format!("line {}: unterminated array", lineno));
        }
        let inner = &s[1..s.len() - 1];
        let mut out = Vec::new();
        for part in split_top(inner, ',') {
            let p = part.trim();
            if p.is_empty() {
                continue;
            }
            out.push(parse_value(p, lineno)?);
        }
        return Ok(Value::Array(out));
    }
    if s.starts_with('{') {
        if !s.ends_with('}') {
            return Err(format!("line {}: unterminated inline table", lineno));
        }
        let inner = &s[1..s.len() - 1];
        let mut t = HashMap::new();
        for part in split_top(inner, ',') {
            let p = part.trim();
            if p.is_empty() {
                continue;
            }
            let eq = p.find('=').ok_or_else(|| {
                format!("line {}: expected `key = value` in inline table", lineno)
            })?;
            let k = p[..eq].trim().trim_matches('"').to_string();
            let v = parse_value(p[eq + 1..].trim(), lineno)?;
            t.insert(k, v);
        }
        return Ok(Value::Table(t));
    }
    // number
    if let Ok(i) = s.parse::<i64>() {
        return Ok(Value::Int(i));
    }
    if let Ok(f) = s.parse::<f64>() {
        return Ok(Value::Float(f));
    }
    Err(format!("line {}: cannot parse value `{}`", lineno, s))
}

/// Split on `sep` at bracket/brace/quote depth zero.
fn split_top(s: &str, sep: char) -> Vec<String> {
    let mut out = Vec::new();
    let mut depth = 0i32;
    let mut in_str = false;
    let mut cur = String::new();
    for c in s.chars() {
        match c {
            '"' => {
                in_str = !in_str;
                cur.push(c);
            }
            '[' | '{' if !in_str => {
                depth += 1;
                cur.push(c);
            }
            ']' | '}' if !in_str => {
                depth -= 1;
                cur.push(c);
            }
            c if c == sep && depth == 0 && !in_str => {
                out.push(std::mem::take(&mut cur));
            }
            c => cur.push(c),
        }
    }
    out.push(cur);
    out
}

fn unescape(s: &str) -> String {
    let mut out = String::new();
    let mut chars = s.chars();
    while let Some(c) = chars.next() {
        if c == '\\' {
            match chars.next() {
                Some('n') => out.push('\n'),
                Some('t') => out.push('\t'),
                Some('r') => out.push('\r'),
                Some('\\') => out.push('\\'),
                Some('"') => out.push('"'),
                Some(other) => {
                    out.push('\\');
                    out.push(other);
                }
                None => out.push('\\'),
            }
        } else {
            out.push(c);
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_config_shape() {
        let t = parse_toml(
            r#"
# demo config
[package]
name = "demo"
version = "0.1.0"

[targets]
python = true
native = true

[build]
sources = ["src", "extra"]
profile = "release"

[dependencies]
mathx = { git = "https://github.com/a/b", tag = "v1" }
"#,
        )
        .unwrap();
        let pkg = t["package"].as_table().unwrap();
        assert_eq!(pkg["name"].as_str(), Some("demo"));
        let targets = t["targets"].as_table().unwrap();
        assert_eq!(targets["python"].as_bool(), Some(true));
        assert_eq!(targets.get("web"), None);
        let build = t["build"].as_table().unwrap();
        assert_eq!(
            build["sources"].as_array().unwrap()[0].as_str(),
            Some("src")
        );
        let deps = t["dependencies"].as_table().unwrap();
        let mathx = deps["mathx"].as_table().unwrap();
        assert_eq!(mathx["git"].as_str(), Some("https://github.com/a/b"));
    }

    #[test]
    fn parses_nested_tables() {
        let t = parse_toml("[a.b]\nx = 1\n[a]\ny = 2\n").unwrap();
        assert_eq!(t["a"].as_table().unwrap()["y"].as_int(), Some(2));
        assert_eq!(
            t["a"].as_table().unwrap()["b"].as_table().unwrap()["x"].as_int(),
            Some(1)
        );
    }
}
