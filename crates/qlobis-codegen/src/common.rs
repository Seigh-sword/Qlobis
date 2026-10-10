//! Shared helpers for the AST-based backends.

use qlobis_ast::*;
use qlobis_tyck::{CheckOutput, ModuleInput, Target};

/// Does this function apply to `target`?
pub fn applies(f: &FuncDef, target: Target) -> bool {
    match &f.target {
        None => true,
        Some(names) => names.iter().any(|n| Target::parse(&n.name) == Some(target)),
    }
}

/// Namespace -> module path map for one module.
pub fn import_map(m: &ModuleInput) -> Vec<(String, String)> {
    let mut out = Vec::new();
    for imp in &m.program.imports {
        if imp.path.is_empty() {
            continue;
        }
        let path = imp.path.replace('/', "::");
        let ns = imp
            .alias
            .as_ref()
            .map(|a| a.name.clone())
            .unwrap_or_else(|| {
                imp.path
                    .rsplit(['/', ':'])
                    .find(|s| !s.is_empty())
                    .unwrap_or(&imp.path)
                    .to_string()
            });
        let ns = ns.rsplit("::").next().unwrap_or(&ns).to_string();
        out.push((ns, path));
    }
    out
}

/// The variant of a function group that applies to `target`.
#[allow(dead_code)]
pub fn active_variant<'a>(
    m: &'a ModuleInput,
    name: &str,
    target: Target,
) -> Option<&'a FuncDef> {
    for item in &m.program.items {
        if let Item::Func(f) = item {
            if f.name.name == name && applies(f, target) {
                return Some(f);
            }
        }
    }
    None
}

/// Local name scope tracking (params, lets, loop vars, catch vars).
#[derive(Default)]
pub struct Scope {
    frames: Vec<Vec<String>>,
}

impl Scope {
    pub fn new() -> Self {
        Scope { frames: vec![Vec::new()] }
    }

    pub fn push(&mut self) {
        self.frames.push(Vec::new());
    }

    pub fn pop(&mut self) {
        self.frames.pop();
    }

    pub fn declare(&mut self, name: &str) {
        if let Some(f) = self.frames.last_mut() {
            f.push(name.to_string());
        }
    }

    pub fn contains(&self, name: &str) -> bool {
        self.frames.iter().any(|f| f.iter().any(|n| n == name))
    }
}

/// Common emission context fields shared by backends.
pub struct Ctx<'a> {
    pub modules: &'a [ModuleInput],
    pub checked: &'a CheckOutput,
    pub target: Target,
    pub imports: Vec<(String, String)>,
    pub module: String,
    pub scope: Scope,
    pub out: String,
    pub indent: usize,
    /// Backend-specific extras (Kotlin uses this for function-reference imports).
    pub fn_imports: Vec<(String, String)>,
    /// True while emitting an `async` function body.
    pub in_async: bool,
}

impl<'a> Ctx<'a> {
    pub fn new(modules: &'a [ModuleInput], checked: &'a CheckOutput, target: Target, m: &ModuleInput) -> Self {
        Ctx {
            modules,
            checked,
            target,
            imports: import_map(m),
            module: m.path.clone(),
            scope: Scope::new(),
            out: String::new(),
            indent: 0,
            fn_imports: Vec::new(),
            in_async: false,
        }
    }

    pub fn ty(&self, e: &Expr) -> Type {
        self.checked.type_of(e.span())
    }

    pub fn ty_of(&self, span: Span) -> Type {
        self.checked.type_of(span)
    }

    pub fn line(&mut self, s: &str) {
        for _ in 0..self.indent {
            self.out.push_str("    ");
        }
        self.out.push_str(s);
        self.out.push('\n');
    }

    pub fn raw(&mut self, s: &str) {
        self.out.push_str(s);
    }

    /// Resolve a namespace to its module path.
    pub fn ns_module(&self, ns: &str) -> Option<String> {
        self.imports
            .iter()
            .find(|(n, _)| n == ns)
            .map(|(_, p)| p.clone())
    }
}

/// Should this name be emitted as a local reference?
pub fn is_local(ctx: &Ctx, name: &str) -> bool {
    ctx.scope.contains(name)
}

/// True when `name` resolves to an async function (free or method) near `module`.
pub fn is_async_fn(checked: &qlobis_tyck::CheckOutput, module: &str, name: &str) -> bool {
    checked
        .funcs
        .iter()
        .any(|f| f.name == name && (f.module == module || f.self_type.is_some()) && f.is_async)
}
