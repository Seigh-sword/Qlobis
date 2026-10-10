//! Per-target code generation.
//!
//! Simple targets (Python, PHP, Kotlin, Web) transpile directly from the AST;
//! the native target goes `AST -> IR -> optimized IR -> C++`, and the build
//! system then invokes an external C++ compiler.

mod common;
mod kotlin;
mod native;
mod php;
mod python;
mod web;

pub use native::cpp_type_of;

use qlobis_tyck::{CheckOutput, ModuleInput, Target};

/// One generated file (path relative to the target output directory).
pub struct GenFile {
    pub path: String,
    pub contents: String,
}

/// Everything a backend needs.
pub struct GenInput<'a> {
    pub modules: &'a [ModuleInput],
    pub checked: &'a CheckOutput,
    pub entry: &'a (String, String),
    pub package: &'a str,
    pub release: bool,
}

/// Generate all files for `target`.
pub fn generate(target: Target, input: &GenInput) -> Vec<GenFile> {
    match target {
        Target::Python => python::generate(input),
        Target::Php => php::generate(input),
        Target::Kotlin => kotlin::generate(input),
        Target::Web => web::generate(input),
        Target::Native => native::generate(input),
    }
}
