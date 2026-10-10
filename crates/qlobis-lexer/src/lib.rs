//! Qlobis tokenizer.
//!
//! Produces a flat token stream with accurate spans. String interpolation is
//! captured as raw sub-sources (`{expr}` segments) which the parser re-lexes
//! recursively so spans inside interpolations stay correct.

use qlobis_ast::diag::{Diagnostic, Diagnostics};
use qlobis_ast::{FileId, Span};

#[derive(Clone, Debug, PartialEq)]
pub enum StrRawPart {
    Lit(String),
    /// Raw source text of `{ ... }` plus its span in the enclosing file.
    Interp { src: String, span: Span },
}

#[derive(Clone, Debug, PartialEq)]
pub enum Tok {
    // literals & names
    Int(i64),
    Float(f64),
    Str(Vec<StrRawPart>),
    Ident(String),
    // keywords
    Defin,
    Let,
    If,
    Else,
    While,
    For,
    In,
    Return,
    Try,
    Throw,
    True,
    False,
    None,
    And,
    Or,
    Not,
    Struct,
    Trait,
    Impl,
    SelfVal,
    Async,
    Await,
    Break,
    Continue,
    // punctuation
    LParen,
    RParen,
    LBrace,
    RBrace,
    LBracket,
    RBracket,
    Comma,
    Dot,
    Colon,
    Semi,
    Arrow,
    At,
    // operators
    Assign,
    PlusAssign,
    MinusAssign,
    StarAssign,
    SlashAssign,
    PercentAssign,
    Plus,
    Minus,
    Star,
    Slash,
    Percent,
    Amp,
    Pipe,
    Caret,
    Tilde,
    Shl,
    Shr,
    Eq,
    Ne,
    Lt,
    Le,
    Gt,
    Ge,
    Bang,
    Eof,
}

impl Tok {
    pub fn describe(&self) -> String {
        match self {
            Tok::Int(v) => format!("integer `{}`", v),
            Tok::Float(v) => format!("float `{}`", v),
            Tok::Str(_) => "string literal".to_string(),
            Tok::Ident(s) => format!("identifier `{}`", s),
            Tok::Defin => "`defin`".into(),
            Tok::Let => "`let`".into(),
            Tok::If => "`if`".into(),
            Tok::Else => "`else`".into(),
            Tok::While => "`while`".into(),
            Tok::For => "`for`".into(),
            Tok::In => "`in`".into(),
            Tok::Return => "`return`".into(),
            Tok::Try => "`try`".into(),
            Tok::Throw => "`throw`".into(),
            Tok::True => "`true`".into(),
            Tok::False => "`false`".into(),
            Tok::None => "`none`".into(),
            Tok::And => "`and`".into(),
            Tok::Or => "`or`".into(),
            Tok::Not => "`not`".into(),
            Tok::Struct => "`struct`".into(),
            Tok::Trait => "`trait`".into(),
            Tok::Impl => "`impl`".into(),
            Tok::SelfVal => "`self`".into(),
            Tok::Async => "`async`".into(),
            Tok::Await => "`await`".into(),
            Tok::Break => "`break`".into(),
            Tok::Continue => "`continue`".into(),
            Tok::LParen => "`(`".into(),
            Tok::RParen => "`)`".into(),
            Tok::LBrace => "`{`".into(),
            Tok::RBrace => "`}`".into(),
            Tok::LBracket => "`[`".into(),
            Tok::RBracket => "`]`".into(),
            Tok::Comma => "`,`".into(),
            Tok::Dot => "`.`".into(),
            Tok::Colon => "`:`".into(),
            Tok::Semi => "`;`".into(),
            Tok::Arrow => "`->`".into(),
            Tok::At => "`@`".into(),
            Tok::Assign => "`=`".into(),
            Tok::PlusAssign => "`+=`".into(),
            Tok::MinusAssign => "`-=`".into(),
            Tok::StarAssign => "`*=`".into(),
            Tok::SlashAssign => "`/=`".into(),
            Tok::PercentAssign => "`%=`".into(),
            Tok::Plus => "`+`".into(),
            Tok::Minus => "`-`".into(),
            Tok::Star => "`*`".into(),
            Tok::Slash => "`/`".into(),
            Tok::Percent => "`%`".into(),
            Tok::Amp => "`&`".into(),
            Tok::Pipe => "`|`".into(),
            Tok::Caret => "`^`".into(),
            Tok::Tilde => "`~`".into(),
            Tok::Shl => "`<<`".into(),
            Tok::Shr => "`>>`".into(),
            Tok::Eq => "`==`".into(),
            Tok::Ne => "`!=`".into(),
            Tok::Lt => "`<`".into(),
            Tok::Le => "`<=`".into(),
            Tok::Gt => "`>`".into(),
            Tok::Ge => "`>=`".into(),
            Tok::Bang => "`!`".into(),
            Tok::Eof => "end of file".into(),
        }
    }
}

#[derive(Clone, Debug)]
pub struct Token {
    pub tok: Tok,
    pub span: Span,
}

/// A doc comment (`///`) attached to the following item, with its span.
#[derive(Clone, Debug)]
pub struct DocComment {
    pub text: String,
    pub span: Span,
}

pub struct LexResult {
    pub tokens: Vec<Token>,
    /// Doc comments keyed by the byte offset of the token they precede.
    pub docs: Vec<(u32, DocComment)>,
}

/// Tokenize `src`. `file` is the file id used in all spans. `offset` shifts
/// spans so that recursively lexed interpolation segments keep global spans.
pub fn lex(file: FileId, src: &str, offset: u32, diags: &mut Diagnostics) -> LexResult {
    Lexer { src: src.as_bytes(), file, offset, pos: 0, diags, docs: Vec::new() }.run()
}

struct Lexer<'a> {
    src: &'a [u8],
    file: FileId,
    offset: u32,
    pos: usize,
    diags: &'a mut Diagnostics,
    docs: Vec<(u32, DocComment)>,
}

impl<'a> Lexer<'a> {
    fn span(&self, start: usize, end: usize) -> Span {
        Span::new(self.file, self.offset + start as u32, self.offset + end as u32)
    }

    fn peek(&self) -> Option<u8> {
        self.src.get(self.pos).copied()
    }

    fn peek2(&self) -> Option<u8> {
        self.src.get(self.pos + 1).copied()
    }

    fn bump(&mut self) -> Option<u8> {
        let b = self.peek();
        if b.is_some() {
            self.pos += 1;
        }
        b
    }

    fn error(&mut self, span: Span, msg: impl Into<String>) {
        self.diags.push(Diagnostic::error(msg).code("E0100").primary(span, "here"));
    }

    fn run(mut self) -> LexResult {
        let mut tokens = Vec::new();
        while let Some(b) = self.peek() {
            match b {
                b' ' | b'\t' | b'\r' | b'\n' => {
                    self.pos += 1;
                }
                b'/' if self.peek2() == Some(b'/') => {
                    let start = self.pos;
                    // `///` (but not `////...`) starts a doc comment.
                    if self.src.get(self.pos + 2).copied() == Some(b'/')
                        && self.src.get(self.pos + 3).copied() != Some(b'/')
                    {
                        // Doc comment: consume to end of line.
                        while let Some(c) = self.peek() {
                            if c == b'\n' {
                                break;
                            }
                            self.pos += 1;
                        }
                        let raw = &self.src[start + 3..self.pos];
                        let text = String::from_utf8_lossy(raw).trim().to_string();
                        let span = self.span(start, self.pos);
                        self.docs.push((self.offset + start as u32, DocComment { text, span }));
                    } else {
                        while let Some(c) = self.peek() {
                            if c == b'\n' {
                                break;
                            }
                            self.pos += 1;
                        }
                    }
                }
                b'/' if self.peek2() == Some(b'*') => {
                    let start = self.pos;
                    self.pos += 2;
                    let mut depth = 1;
                    while depth > 0 {
                        match self.peek() {
                            None => {
                                let sp = self.span(start, self.pos);
                                self.error(sp, "unterminated block comment");
                                break;
                            }
                            Some(b'/') if self.peek2() == Some(b'*') => {
                                self.pos += 2;
                                depth += 1;
                            }
                            Some(b'*') if self.peek2() == Some(b'/') => {
                                self.pos += 2;
                                depth -= 1;
                            }
                            Some(_) => self.pos += 1,
                        }
                    }
                }
                b'0'..=b'9' => {
                    let start = self.pos;
                    // hex / binary prefixes
                    if b == b'0' && matches!(self.peek2(), Some(b'x') | Some(b'X')) {
                        self.pos += 2;
                        let hs = self.pos;
                        while matches!(self.peek(), Some(c) if c.is_ascii_hexdigit() || c == b'_') {
                            self.pos += 1;
                        }
                        let text: String = std::str::from_utf8(&self.src[hs..self.pos])
                            .unwrap_or("")
                            .chars()
                            .filter(|c| *c != '_')
                            .collect();
                        let span = self.span(start, self.pos);
                        match i64::from_str_radix(&text, 16) {
                            Ok(v) => tokens.push(Token { tok: Tok::Int(v), span }),
                            Err(_) => self.error(span, "hexadecimal literal out of range for `int`"),
                        }
                        continue;
                    }
                    while matches!(self.peek(), Some(c) if c.is_ascii_digit() || c == b'_') {
                        self.pos += 1;
                    }
                    let mut is_float = false;
                    if self.peek() == Some(b'.') && matches!(self.peek2(), Some(c) if c.is_ascii_digit())
                    {
                        is_float = true;
                        self.pos += 1;
                        while matches!(self.peek(), Some(c) if c.is_ascii_digit() || c == b'_') {
                            self.pos += 1;
                        }
                    }
                    if matches!(self.peek(), Some(b'e') | Some(b'E')) {
                        let save = self.pos;
                        self.pos += 1;
                        if matches!(self.peek(), Some(b'+') | Some(b'-')) {
                            self.pos += 1;
                        }
                        if matches!(self.peek(), Some(c) if c.is_ascii_digit()) {
                            is_float = true;
                            while matches!(self.peek(), Some(c) if c.is_ascii_digit() || c == b'_') {
                                self.pos += 1;
                            }
                        } else {
                            self.pos = save;
                        }
                    }
                    let raw: String = std::str::from_utf8(&self.src[start..self.pos])
                        .unwrap_or("")
                        .chars()
                        .filter(|c| *c != '_')
                        .collect();
                    let span = self.span(start, self.pos);
                    if is_float {
                        match raw.parse::<f64>() {
                            Ok(v) => tokens.push(Token { tok: Tok::Float(v), span }),
                            Err(_) => self.error(span, "invalid float literal"),
                        }
                    } else {
                        match raw.parse::<i64>() {
                            Ok(v) => tokens.push(Token { tok: Tok::Int(v), span }),
                            Err(_) => self.error(span, "integer literal out of range for `int`"),
                        }
                    }
                }
                b'"' => {
                    let start = self.pos;
                    let tok = self.lex_string();
                    tokens.push(Token { tok, span: self.span(start, self.pos) });
                }
                b'a'..=b'z' | b'A'..=b'Z' | b'_' => {
                    let start = self.pos;
                    while matches!(self.peek(), Some(c) if c.is_ascii_alphanumeric() || c == b'_') {
                        self.pos += 1;
                    }
                    let word = std::str::from_utf8(&self.src[start..self.pos]).unwrap_or("").to_string();
                    let span = self.span(start, self.pos);
                    let tok = match word.as_str() {
                        "defin" => Tok::Defin,
                        "let" => Tok::Let,
                        "if" => Tok::If,
                        "else" => Tok::Else,
                        "while" => Tok::While,
                        "for" => Tok::For,
                        "in" => Tok::In,
                        "return" => Tok::Return,
                        "try" => Tok::Try,
                        "throw" => Tok::Throw,
                        "true" => Tok::True,
                        "false" => Tok::False,
                        "none" => Tok::None,
                        "and" => Tok::And,
                        "or" => Tok::Or,
                        "not" => Tok::Not,
                        "struct" => Tok::Struct,
                        "trait" => Tok::Trait,
                        "impl" => Tok::Impl,
                        "self" => Tok::SelfVal,
                        "async" => Tok::Async,
                        "await" => Tok::Await,
                        "break" => Tok::Break,
                        "continue" => Tok::Continue,
                        _ => Tok::Ident(word),
                    };
                    tokens.push(Token { tok, span });
                }
                _ => {
                    let start = self.pos;
                    let (tok, ok) = self.lex_punct();
                    let span = self.span(start, self.pos);
                    if !ok {
                        let ch = self.src[start..self.pos.min(self.src.len())]
                            .iter()
                            .map(|&b| b as char)
                            .collect::<String>();
                        let sp = self.span(start, start + 1);
                        self.pos += 1; // skip the bad byte
                        self.diags.push(
                            Diagnostic::error(format!("unexpected character `{}`", ch))
                                .code("E0101")
                                .primary(sp, "not valid in Qlobis source"),
                        );
                        continue;
                    }
                    tokens.push(Token { tok, span });
                }
            }
        }
        let end = self.src.len();
        tokens.push(Token { tok: Tok::Eof, span: self.span(end, end) });
        LexResult { tokens, docs: self.docs }
    }

    fn lex_punct(&mut self) -> (Tok, bool) {
        let b = self.peek().unwrap();
        macro_rules! two {
            ($second:expr, $two:expr, $one:expr) => {
                if self.peek2() == Some($second) {
                    self.pos += 2;
                    return ($two, true);
                }
            };
        }
        match b {
            b'(' => {
                self.pos += 1;
                (Tok::LParen, true)
            }
            b')' => {
                self.pos += 1;
                (Tok::RParen, true)
            }
            b'{' => {
                self.pos += 1;
                (Tok::LBrace, true)
            }
            b'}' => {
                self.pos += 1;
                (Tok::RBrace, true)
            }
            b'[' => {
                self.pos += 1;
                (Tok::LBracket, true)
            }
            b']' => {
                self.pos += 1;
                (Tok::RBracket, true)
            }
            b',' => {
                self.pos += 1;
                (Tok::Comma, true)
            }
            b'.' => {
                self.pos += 1;
                (Tok::Dot, true)
            }
            b':' => {
                self.pos += 1;
                (Tok::Colon, true)
            }
            b';' => {
                self.pos += 1;
                (Tok::Semi, true)
            }
            b'@' => {
                self.pos += 1;
                (Tok::At, true)
            }
            b'~' => {
                self.pos += 1;
                (Tok::Tilde, true)
            }
            b'-' => {
                two!(b'>', Tok::Arrow, Tok::Minus);
                two!(b'=', Tok::MinusAssign, Tok::Minus);
                self.pos += 1;
                (Tok::Minus, true)
            }
            b'+' => {
                two!(b'=', Tok::PlusAssign, Tok::Plus);
                self.pos += 1;
                (Tok::Plus, true)
            }
            b'*' => {
                two!(b'=', Tok::StarAssign, Tok::Star);
                self.pos += 1;
                (Tok::Star, true)
            }
            b'/' => {
                two!(b'=', Tok::SlashAssign, Tok::Slash);
                self.pos += 1;
                (Tok::Slash, true)
            }
            b'%' => {
                two!(b'=', Tok::PercentAssign, Tok::Percent);
                self.pos += 1;
                (Tok::Percent, true)
            }
            b'=' => {
                two!(b'=', Tok::Eq, Tok::Assign);
                self.pos += 1;
                (Tok::Assign, true)
            }
            b'!' => {
                two!(b'=', Tok::Ne, Tok::Bang);
                self.pos += 1;
                (Tok::Bang, true)
            }
            b'<' => {
                two!(b'<', Tok::Shl, Tok::Lt);
                two!(b'=', Tok::Le, Tok::Lt);
                self.pos += 1;
                (Tok::Lt, true)
            }
            b'>' => {
                two!(b'>', Tok::Shr, Tok::Gt);
                two!(b'=', Tok::Ge, Tok::Gt);
                self.pos += 1;
                (Tok::Gt, true)
            }
            b'&' => {
                self.pos += 1;
                (Tok::Amp, true)
            }
            b'|' => {
                self.pos += 1;
                (Tok::Pipe, true)
            }
            b'^' => {
                self.pos += 1;
                (Tok::Caret, true)
            }
            _ => (Tok::Eof, false),
        }
    }

    /// Lex a double-quoted string with `{expr}` interpolation.
    fn lex_string(&mut self) -> Tok {
        self.pos += 1; // opening quote
        let mut parts: Vec<StrRawPart> = Vec::new();
        let mut lit = String::new();
        loop {
            match self.peek() {
                None | Some(b'\n') => {
                    let sp = self.span(self.pos.saturating_sub(1), self.pos);
                    self.error(sp, "unterminated string literal");
                    break;
                }
                Some(b'"') => {
                    self.pos += 1;
                    break;
                }
                Some(b'\\') => {
                    self.pos += 1;
                    match self.bump() {
                        Some(b'n') => lit.push('\n'),
                        Some(b't') => lit.push('\t'),
                        Some(b'r') => lit.push('\r'),
                        Some(b'0') => lit.push('\0'),
                        Some(b'\\') => lit.push('\\'),
                        Some(b'"') => lit.push('"'),
                        Some(b'\'') => lit.push('\''),
                        Some(b'{') => lit.push('{'),
                        Some(b'}') => lit.push('}'),
                        Some(other) => {
                            let sp = self.span(self.pos - 2, self.pos);
                            self.error(
                                sp,
                                format!("unknown escape sequence `\\{}`", other as char),
                            );
                            lit.push(other as char);
                        }
                        None => {}
                    }
                }
                Some(b'{') => {
                    if !lit.is_empty() {
                        parts.push(StrRawPart::Lit(std::mem::take(&mut lit)));
                    }
                    let expr_start = self.pos;
                    self.pos += 1;
                    // Scan a balanced { ... } region, respecting nested strings.
                    let mut depth = 1;
                    let seg_start = self.pos;
                    while depth > 0 {
                        match self.peek() {
                            None | Some(b'\n') => {
                                let sp = self.span(expr_start, self.pos);
                                self.error(sp, "unterminated interpolation in string literal");
                                break;
                            }
                            Some(b'{') => depth += 1,
                            Some(b'}') => {
                                depth -= 1;
                                if depth == 0 {
                                    break;
                                }
                            }
                            Some(b'"') => {
                                // Skip nested string literal.
                                self.pos += 1;
                                while let Some(c) = self.peek() {
                                    if c == b'\\' {
                                        self.pos += 2;
                                        continue;
                                    }
                                    if c == b'"' || c == b'\n' {
                                        break;
                                    }
                                    self.pos += 1;
                                }
                                if self.peek() == Some(b'"') {
                                    self.pos += 1;
                                }
                                continue;
                            }
                            _ => {}
                        }
                        self.pos += 1;
                    }
                    let seg_end = self.pos;
                    if self.peek() == Some(b'}') {
                        self.pos += 1;
                    }
                    let src = std::str::from_utf8(&self.src[seg_start..seg_end])
                        .unwrap_or("")
                        .to_string();
                    parts.push(StrRawPart::Interp {
                        src,
                        span: self.span(seg_start, seg_end),
                    });
                }
                Some(_) => {
                    // Fast-path UTF-8 bytes into the literal part.
                    let start = self.pos;
                    while let Some(c) = self.peek() {
                        if c == b'"' || c == b'\\' || c == b'{' {
                            break;
                        }
                        self.pos += 1;
                    }
                    lit.push_str(std::str::from_utf8(&self.src[start..self.pos]).unwrap_or(""));
                }
            }
        }
        if !lit.is_empty() || parts.is_empty() {
            parts.push(StrRawPart::Lit(lit));
        }
        Tok::Str(parts)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use qlobis_ast::diag::Diagnostics;

    fn toks(src: &str) -> Vec<Tok> {
        let mut d = Diagnostics::new();
        let r = lex(0, src, 0, &mut d);
        assert!(!d.has_errors(), "unexpected lex errors: {:?}", d.items);
        r.tokens.into_iter().map(|t| t.tok).collect()
    }

    #[test]
    fn keywords_and_idents() {
        let t = toks("defin main x _y");
        assert!(matches!(t[0], Tok::Defin));
        assert!(matches!(t[1], Tok::Ident(ref s) if s == "main"));
        assert!(matches!(t[2], Tok::Ident(ref s) if s == "x"));
        assert!(matches!(t[3], Tok::Ident(ref s) if s == "_y"));
        assert!(matches!(t[4], Tok::Eof));
    }

    #[test]
    fn numbers() {
        let t = toks("1 2.5 1e3 0x10 1_000");
        assert!(matches!(t[0], Tok::Int(1)));
        assert!(matches!(t[1], Tok::Float(f) if (f - 2.5).abs() < 1e-9));
        assert!(matches!(t[2], Tok::Float(f) if (f - 1000.0).abs() < 1e-9));
        assert!(matches!(t[3], Tok::Int(16)));
        assert!(matches!(t[4], Tok::Int(1000)));
    }

    #[test]
    fn int_then_member() {
        // `x.foo` after an int must lex as Int, Dot, Ident (not a float).
        let t = toks("1.foo");
        assert!(matches!(t[0], Tok::Int(1)));
        assert!(matches!(t[1], Tok::Dot));
        assert!(matches!(t[2], Tok::Ident(_)));
    }

    #[test]
    fn strings_and_interpolation() {
        let t = toks(r#""hello {name}!""#);
        match &t[0] {
            Tok::Str(parts) => {
                assert_eq!(parts.len(), 3);
                assert!(matches!(&parts[0], StrRawPart::Lit(s) if s == "hello "));
                assert!(matches!(&parts[1], StrRawPart::Interp { src, .. } if src == "name"));
                assert!(matches!(&parts[2], StrRawPart::Lit(s) if s == "!"));
            }
            other => panic!("expected string, got {:?}", other),
        }
    }

    #[test]
    fn escapes_and_nested_strings() {
        let t = toks(r#""a\n{f("x")}b""#);
        match &t[0] {
            Tok::Str(parts) => {
                assert_eq!(parts.len(), 3);
                assert!(matches!(&parts[0], StrRawPart::Lit(s) if s == "a\n"));
                assert!(matches!(&parts[1], StrRawPart::Interp { src, .. } if src == r#"f("x")"#));
                assert!(matches!(&parts[2], StrRawPart::Lit(s) if s == "b"));
            }
            other => panic!("expected string, got {:?}", other),
        }
    }

    #[test]
    fn operators() {
        let t = toks("-> == != <= >= << >> += -= *= /= %= and or not");
        assert!(matches!(t[0], Tok::Arrow));
        assert!(matches!(t[1], Tok::Eq));
        assert!(matches!(t[2], Tok::Ne));
        assert!(matches!(t[3], Tok::Le));
        assert!(matches!(t[4], Tok::Ge));
        assert!(matches!(t[5], Tok::Shl));
        assert!(matches!(t[6], Tok::Shr));
        assert!(matches!(t[7], Tok::PlusAssign));
        assert!(matches!(t[8], Tok::MinusAssign));
        assert!(matches!(t[9], Tok::StarAssign));
        assert!(matches!(t[10], Tok::SlashAssign));
        assert!(matches!(t[11], Tok::PercentAssign));
        assert!(matches!(t[12], Tok::And));
        assert!(matches!(t[13], Tok::Or));
        assert!(matches!(t[14], Tok::Not));
    }

    #[test]
    fn comments_and_docs() {
        let mut d = Diagnostics::new();
        let r = lex(0, "/// docs here\n// plain\n/* block */ defin", 0, &mut d);
        assert!(!d.has_errors());
        assert_eq!(r.docs.len(), 1);
        assert_eq!(r.docs[0].1.text, "docs here");
        assert!(matches!(r.tokens[0].tok, Tok::Defin));
    }

    #[test]
    fn unterminated_string_reports_error() {
        let mut d = Diagnostics::new();
        lex(0, "\"abc", 0, &mut d);
        assert!(d.has_errors());
    }
}
