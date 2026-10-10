//! Documentation generator: extracts `///` doc comments and produces
//! Markdown documentation per module plus an index.

use qlobis_ast::*;
use qlobis_tyck::ModuleInput;
use std::fmt::Write as _;

/// Generate Markdown documentation for every module. Returns
/// `(relative_path, contents)` pairs.
pub fn generate_docs(modules: &[ModuleInput], package: &str, version: &str) -> Vec<(String, String)> {
    let mut out = Vec::new();
    let mut index = format!("# {}\n\nVersion {}\n\n## Modules\n\n", package, version);
    for m in modules {
        let mut md = String::new();
        let _ = writeln!(md, "# module `{}`\n", m.path);
        for item in &m.program.items {
            match item {
                Item::Func(f) => {
                    write_func(&mut md, f, "");
                }
                Item::Struct(s) => {
                    for d in &s.doc {
                        let _ = writeln!(md, "{}", d);
                    }
                    let _ = writeln!(md, "## struct `{}`\n", s.name.name);
                    if !s.doc.is_empty() {
                        let _ = writeln!(md, "{}\n", s.doc.join(" "));
                    }
                    let _ = writeln!(md, "| field | type |");
                    let _ = writeln!(md, "| --- | --- |");
                    for f in &s.fields {
                        let _ = writeln!(
                            md,
                            "| `{}` | `{}` |",
                            f.name.name,
                            type_text(&f.ty)
                        );
                    }
                    let _ = writeln!(md);
                }
                Item::Trait(t) => {
                    let _ = writeln!(md, "## trait `{}`\n", t.name.name);
                    if !t.doc.is_empty() {
                        let _ = writeln!(md, "{}\n", t.doc.join(" "));
                    }
                    for f in &t.methods {
                        write_func(&mut md, f, "### ");
                    }
                }
                Item::Impl(im) => {
                    let _ = writeln!(md, "## impl `{}`\n", type_text(&im.self_ty));
                    for f in &im.methods {
                        write_func(&mut md, f, "### ");
                    }
                }
            }
        }
        let fname = format!("{}.md", m.path.replace("::", "__").replace('/', "__"));
        let _ = writeln!(index, "- [{}]({})", m.path, fname);
        out.push((fname, md));
    }
    out.push(("index.md".to_string(), index));
    out
}

fn write_func(md: &mut String, f: &FuncDef, level: &str) {
    if !f.doc.is_empty() {
        for d in &f.doc {
            let _ = writeln!(md, "{}", d);
        }
    }
    let params: Vec<String> = f
        .params
        .iter()
        .filter(|p| !p.is_self)
        .map(|p| {
            format!(
                "{}: {}",
                p.name.name,
                p.ty.as_ref().map(type_text).unwrap_or_else(|| "any".into())
            )
        })
        .collect();
    let ret = f
        .ret
        .as_ref()
        .map(type_text)
        .unwrap_or_else(|| "none".to_string());
    let async_kw = if f.is_async { "async " } else { "" };
    let _ = writeln!(
        md,
        "{}{}defin {}({}) -> {}\n",
        level, async_kw, f.name.name, params.join(", "), ret
    );
    if !f.doc.is_empty() {
        let _ = writeln!(md, "{}\n", f.doc.join(" "));
    }
}

pub fn type_text(t: &TypeExpr) -> String {
    match t {
        TypeExpr::Named { name, args, .. } => {
            if args.is_empty() {
                name.clone()
            } else {
                format!(
                    "{}<{}>",
                    name,
                    args.iter().map(type_text).collect::<Vec<_>>().join(", ")
                )
            }
        }
        TypeExpr::Fn { params, ret, .. } => {
            format!(
                "fn({}) -> {}",
                params.iter().map(type_text).collect::<Vec<_>>().join(", "),
                type_text(ret)
            )
        }
    }
}

/// Write docs into `out_dir`, creating it.
pub fn write_docs(
    out_dir: &std::path::Path,
    modules: &[ModuleInput],
    package: &str,
    version: &str,
) -> Result<Vec<std::path::PathBuf>, String> {
    std::fs::create_dir_all(out_dir)
        .map_err(|e| format!("cannot create {}: {}", out_dir.display(), e))?;
    let mut written = Vec::new();
    for (name, contents) in generate_docs(modules, package, version) {
        let p = out_dir.join(name);
        std::fs::write(&p, contents)
            .map_err(|e| format!("cannot write {}: {}", p.display(), e))?;
        written.push(p);
    }
    Ok(written)
}

#[cfg(test)]
mod tests {
    use super::*;
    use qlobis_tyck::check;

    #[test]
    fn docs_capture_signatures() {
        let src = "/// Add two numbers.\ndefin add(a: int, b: int) -> int {\n  return a + b\n}\n\n/// A point.\nstruct Point {\n  x: int\n  y: int\n}\n";
        let mut diags = Diagnostics::new();
        let program = qlobis_parser::parse_file(0, src, &mut diags);
        let modules = vec![ModuleInput {
            path: "p::main".into(),
            file: 0,
            program,
            is_stdlib: false,
        }];
        let _ = check(&modules, "p", &[qlobis_tyck::Target::Python], &mut diags);
        let docs = generate_docs(&modules, "p", "1.0.0");
        let md = docs.iter().find(|(n, _)| n == "p__main.md").unwrap();
        assert!(md.1.contains("defin add(a: int, b: int) -> int"), "{}", md.1);
        assert!(md.1.contains("struct `Point`"), "{}", md.1);
        assert!(md.1.contains("Add two numbers."), "{}", md.1);
        let index = docs.iter().find(|(n, _)| n == "index.md").unwrap();
        assert!(index.1.contains("p::main"));
    }
}
