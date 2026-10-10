//! Linter: AST-level checks producing Rust-quality diagnostics with spans.

use qlobis_ast::*;
use qlobis_tyck::{CheckOutput, ModuleInput};
use std::collections::HashSet;

/// Run all lints over the parsed modules. Returns human-readable findings
/// (empty when clean).
pub fn lint(modules: &[ModuleInput], checked: &CheckOutput, map: &SourceMap) -> Vec<String> {
    let mut out = Vec::new();
    for m in modules {
        let mut l = Linter {
            m,
            checked,
            findings: Vec::new(),
        };
        l.run();
        for (span, msg) in l.findings {
            out.push(format!("{}: warning: {}", map.location(span), msg));
        }
    }
    out
}

struct Linter<'a> {
    m: &'a ModuleInput,
    #[allow(dead_code)]
    checked: &'a CheckOutput,
    findings: Vec<(Span, String)>,
}

impl<'a> Linter<'a> {
    fn warn(&mut self, span: Span, msg: impl Into<String>) {
        self.findings.push((span, msg.into()));
    }

    fn run(&mut self) {
        let mut used_imports: HashSet<String> = HashSet::new();
        for item in &self.m.program.items {
            match item {
                Item::Func(f) => self.func(f, &mut used_imports),
                Item::Struct(s) => {
                    if !is_snake_or_pascal(&s.name.name, true) {
                        self.warn(
                            s.name.span,
                            format!("struct `{}` should be PascalCase", s.name.name),
                        );
                    }
                    for f in &s.fields {
                        if !is_snake_or_pascal(&f.name.name, false) {
                            self.warn(
                                f.name.span,
                                format!("field `{}` should be snake_case", f.name.name),
                            );
                        }
                    }
                }
                Item::Trait(t) => {
                    if !is_snake_or_pascal(&t.name.name, true) {
                        self.warn(
                            t.name.span,
                            format!("trait `{}` should be PascalCase", t.name.name),
                        );
                    }
                }
                Item::Impl(im) => {
                    for f in &im.methods {
                        let mut u = HashSet::new();
                        self.func(f, &mut u);
                    }
                }
            }
        }
        for imp in &self.m.program.imports {
            let last = imp
                .path
                .rsplit(['/', ':'])
                .find(|s| !s.is_empty())
                .unwrap_or(&imp.path);
            let last = last.rsplit("::").next().unwrap_or(last);
            if !used_imports.contains(last) {
                self.warn(imp.span, format!("unused import `{}`", imp.path));
            }
        }
    }

    fn func(&mut self, f: &FuncDef, used: &mut HashSet<String>) {
        if !is_snake_or_pascal(&f.name.name, false) {
            self.warn(
                f.name.span,
                format!("function `{}` should be snake_case", f.name.name),
            );
        }
        let mut assigned: HashSet<String> = HashSet::new();
        let mut read: HashSet<String> = HashSet::new();
        if let Some(body) = &f.body {
            self.block(body, used, &mut assigned, &mut read, true);
        }
        // Variables assigned once and never read.
        for name in &assigned {
            if !read.contains(name) {
                // Best-effort: no span here, skip noisy cases.
            }
        }
    }

    fn block(
        &mut self,
        b: &Block,
        used: &mut HashSet<String>,
        assigned: &mut HashSet<String>,
        read: &mut HashSet<String>,
        _top: bool,
    ) {
        for s in &b.stmts {
            self.stmt(s, used, assigned, read);
        }
    }

    fn stmt(
        &mut self,
        s: &Stmt,
        used: &mut HashSet<String>,
        assigned: &mut HashSet<String>,
        read: &mut HashSet<String>,
    ) {
        match s {
            Stmt::Let { name, value, .. } => {
                self.expr(value, used, read);
                assigned.insert(name.name.clone());
            }
            Stmt::Assign { target, value, .. } => {
                self.expr(target, used, read);
                self.expr(value, used, read);
            }
            Stmt::If {
                cond,
                then,
                otherwise,
                ..
            } => {
                self.expr(cond, used, read);
                self.block(then, used, assigned, read, false);
                if let Some(o) = otherwise {
                    self.block(o, used, assigned, read, false);
                }
                if then.stmts.is_empty()
                    && otherwise
                        .as_ref()
                        .map(|o| o.stmts.is_empty())
                        .unwrap_or(true)
                {
                    self.warn(s.span(), "empty `if` body");
                }
            }
            Stmt::While { cond, body, .. } => {
                self.expr(cond, used, read);
                self.block(body, used, assigned, read, false);
            }
            Stmt::For { iter, body, .. } => {
                self.expr(iter, used, read);
                self.block(body, used, assigned, read, false);
            }
            Stmt::Return { value, .. } => {
                if let Some(v) = value {
                    self.expr(v, used, read);
                }
            }
            Stmt::Throw { value, .. } => self.expr(value, used, read),
            Stmt::Try { body, catch, .. } => {
                self.block(body, used, assigned, read, false);
                if let Some(c) = catch {
                    self.block(&c.body, used, assigned, read, false);
                }
            }
            Stmt::Block(b) => self.block(b, used, assigned, read, false),
            Stmt::Expr { expr, .. } => self.expr(expr, used, read),
            _ => {}
        }
    }

    fn expr(&mut self, e: &Expr, used: &mut HashSet<String>, read: &mut HashSet<String>) {
        match e {
            Expr::Ident(id) => {
                read.insert(id.name.clone());
            }
            Expr::Member { recv, name, .. } => {
                if let Expr::Ident(ns) = recv.as_ref() {
                    used.insert(ns.name.clone());
                }
                self.expr(recv, used, read);
                let _ = name;
            }
            Expr::Method {
                recv, name, args, ..
            } => {
                if let Expr::Ident(ns) = recv.as_ref() {
                    used.insert(ns.name.clone());
                }
                self.expr(recv, used, read);
                for a in args {
                    self.expr(a, used, read);
                }
                let _ = name;
            }
            Expr::Call { func, args, .. } => {
                self.expr(func, used, read);
                for a in args {
                    self.expr(a, used, read);
                }
            }
            Expr::Binary { lhs, rhs, .. } => {
                self.expr(lhs, used, read);
                self.expr(rhs, used, read);
            }
            Expr::Unary { expr, .. } | Expr::Await { expr, .. } => {
                self.expr(expr, used, read);
            }
            Expr::Index { recv, index, .. } => {
                self.expr(recv, used, read);
                self.expr(index, used, read);
            }
            Expr::List(items, _) => {
                for i in items {
                    self.expr(i, used, read);
                }
            }
            Expr::Map(entries, _) => {
                for (k, v) in entries {
                    self.expr(k, used, read);
                    self.expr(v, used, read);
                }
            }
            Expr::Str(parts, _) => {
                for p in parts {
                    if let StrPart::Interp(inner) = p {
                        self.expr(inner, used, read);
                    }
                }
            }
            _ => {}
        }
    }
}

fn is_snake_or_pascal(name: &str, pascal: bool) -> bool {
    if name.starts_with("__") {
        return true;
    }
    let mut chars = name.chars();
    let Some(first) = chars.next() else {
        return true;
    };
    if pascal {
        first.is_uppercase()
    } else {
        first.is_lowercase() || first == '_'
    }
}

/// Format a lint-clean report string for CLI display.
pub fn render(findings: &[String]) -> String {
    if findings.is_empty() {
        "no warnings".to_string()
    } else {
        format!("{} warning(s):\n{}", findings.len(), findings.join("\n"))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use qlobis_tyck::{check, Target};

    fn lint_src(src: &str) -> Vec<String> {
        let mut diags = Diagnostics::new();
        let mut map = SourceMap::new();
        map.add_file("t.qbs", src);
        let program = qlobis_parser::parse_file(0, src, &mut diags);
        let modules = vec![ModuleInput {
            path: "p::main".into(),
            file: 0,
            program,
            is_stdlib: false,
        }];
        let checked = check(&modules, "p", &[Target::Python], &mut diags);
        lint(&modules, &checked, &map)
    }

    #[test]
    fn warns_unused_import() {
        let out = lint_src("@module.import(\"std::io\")\n\ndefin main() {\n  let x = 1\n}\n");
        assert!(out.iter().any(|w| w.contains("unused import")), "{:?}", out);
    }

    #[test]
    fn clean_program_has_no_warnings() {
        let out = lint_src(
            "defin main() {\n  let count = 1\n  let y = count + 1\n  __println(str(y))\n}\n",
        );
        assert!(out.is_empty(), "{:?}", out);
    }

    #[test]
    fn warns_empty_if() {
        let out = lint_src("defin main() {\n  if true {\n  }\n}\n");
        assert!(out.iter().any(|w| w.contains("empty `if`")), "{:?}", out);
    }
}
