//! Shared data structures for the Qlobis compiler: source spans, the source
//! map, the diagnostic engine, the abstract syntax tree, and the semantic type
//! representation used from type checking through code generation.

pub mod builtins;
pub mod diag;
pub mod node;
pub mod ty;

pub use builtins::{builtin_method, intrinsic};
pub use diag::{Diagnostic, Diagnostics, Label, Severity};
pub use node::*;
pub use ty::Type;

use std::fmt;

/// Identifier of a source file inside a [`SourceMap`].
pub type FileId = u32;

/// A half-open byte range `[start, end)` inside one source file.
#[derive(Clone, Copy, PartialEq, Eq, Hash)]
pub struct Span {
    pub file: FileId,
    pub start: u32,
    pub end: u32,
}

impl Span {
    pub fn new(file: FileId, start: u32, end: u32) -> Self {
        Span { file, start, end }
    }

    /// A zero-width span at `pos`, useful for "expected X here" diagnostics.
    pub fn at(file: FileId, pos: u32) -> Self {
        Span {
            file,
            start: pos,
            end: pos,
        }
    }

    /// Smallest span covering both `self` and `other`.
    pub fn merge(self, other: Span) -> Span {
        Span {
            file: self.file,
            start: self.start.min(other.start),
            end: self.end.max(other.end),
        }
    }

    pub fn is_empty(self) -> bool {
        self.start >= self.end
    }
}

impl fmt::Debug for Span {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}..{}@{}", self.start, self.end, self.file)
    }
}

/// 1-based line/column position, for humans.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct LineCol {
    pub line: u32,
    pub col: u32,
}

/// One loaded source file.
pub struct SourceFile {
    pub id: FileId,
    /// Display name (usually the path relative to the project root).
    pub name: String,
    pub src: String,
    /// Byte offsets where each line starts (line 0 at offset 0).
    line_starts: Vec<u32>,
}

impl SourceFile {
    pub fn new(id: FileId, name: String, src: String) -> Self {
        let mut line_starts = vec![0u32];
        for (i, b) in src.bytes().enumerate() {
            if b == b'\n' {
                line_starts.push(i as u32 + 1);
            }
        }
        SourceFile {
            id,
            name,
            src,
            line_starts,
        }
    }

    /// Convert a byte offset into a 1-based line/column pair.
    pub fn line_col(&self, offset: u32) -> LineCol {
        let idx = match self.line_starts.binary_search(&offset) {
            Ok(i) => i,
            Err(i) => i.saturating_sub(1),
        };
        let line_start = self.line_starts[idx];
        LineCol {
            line: idx as u32 + 1,
            col: offset.saturating_sub(line_start) + 1,
        }
    }

    /// The text of a 1-based line, without its trailing newline.
    pub fn line_text(&self, line: u32) -> &str {
        let idx = line.saturating_sub(1) as usize;
        if idx >= self.line_starts.len() {
            return "";
        }
        let start = self.line_starts[idx] as usize;
        let end = self
            .line_starts
            .get(idx + 1)
            .map(|&e| e as usize)
            .unwrap_or(self.src.len());
        self.src[start..end].trim_end_matches(['\n', '\r'])
    }
}

/// All source files loaded for one compilation.
#[derive(Default)]
pub struct SourceMap {
    files: Vec<SourceFile>,
}

impl SourceMap {
    pub fn new() -> Self {
        SourceMap::default()
    }

    /// Register a file and return its id.
    pub fn add_file(&mut self, name: impl Into<String>, src: impl Into<String>) -> FileId {
        let id = self.files.len() as FileId;
        self.files
            .push(SourceFile::new(id, name.into(), src.into()));
        id
    }

    pub fn file(&self, id: FileId) -> &SourceFile {
        &self.files[id as usize]
    }

    pub fn files(&self) -> &[SourceFile] {
        &self.files
    }

    pub fn line_col(&self, span: Span) -> LineCol {
        self.file(span.file).line_col(span.start)
    }

    /// `path:line:col` for a span.
    pub fn location(&self, span: Span) -> String {
        let f = self.file(span.file);
        let lc = f.line_col(span.start);
        format!("{}:{}:{}", f.name, lc.line, lc.col)
    }
}
