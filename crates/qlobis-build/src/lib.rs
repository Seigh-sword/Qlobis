//! Build orchestration: discover sources, typecheck, generate every target,
//! and (for native) invoke the external C++ toolchain.

pub mod config;
pub mod docs;
pub mod fmt;
pub mod lint;
pub mod lsp;
pub mod registry;
pub mod toml;

use config::{BuildConfig, Profile};
use qlobis_ast::{Diagnostics, SourceMap};
use qlobis_codegen::{generate, GenFile, GenInput};
use qlobis_tyck::{check, CheckOutput, ModuleInput, Target};
use std::path::{Path, PathBuf};

/// Everything the compiler saw, kept for later stages (run, lint, docs).
pub struct Program {
    pub modules: Vec<ModuleInput>,
    pub map: SourceMap,
    pub checked: CheckOutput,
}

/// Parse + typecheck the whole source tree (project sources, stdlib, deps).
pub fn load_program(cfg: &BuildConfig, extra_roots: &[(String, PathBuf)]) -> Result<Program, String> {
    let mut map = SourceMap::new();
    let mut diags = Diagnostics::new();
    let mut modules: Vec<ModuleInput> = Vec::new();

    // Stdlib first so user modules can shadow deliberately.
    let stdlib_dir = cfg
        .stdlib
        .clone()
        .map(|s| cfg.root.join(s))
        .unwrap_or_else(|| cfg.root.join("stdlib"));
    if stdlib_dir.is_dir() {
        collect_modules(&stdlib_dir, "std", true, &cfg.name, &mut map, &mut diags, &mut modules)?;
    }
    // Dependency packages.
    for (dep_name, root) in extra_roots {
        let src_dir = root.join("src");
        let dir = if src_dir.is_dir() { &src_dir } else { root };
        collect_modules(dir, dep_name, false, dep_name, &mut map, &mut diags, &mut modules)?;
    }
    // Project sources.
    for s in &cfg.sources {
        let dir = cfg.root.join(s);
        if !dir.is_dir() {
            return Err(format!("source directory `{}` not found", dir.display()));
        }
        collect_modules(&dir, &cfg.name, false, &cfg.name, &mut map, &mut diags, &mut modules)?;
    }

    let checked = check(&modules, &cfg.name, &cfg.targets, &mut diags);
    if diags.has_errors() {
        return Err(diags.render(&map, true));
    }
    Ok(Program {
        modules,
        map,
        checked,
    })
}

/// Walk `dir` collecting `.qbs` files as modules rooted at `package_root`
/// (module paths are `pkg::a::b` for `a/b.qbs`).
fn collect_modules(
    dir: &Path,
    package_root: &str,
    is_stdlib: bool,
    _pkg: &str,
    map: &mut SourceMap,
    diags: &mut Diagnostics,
    out: &mut Vec<ModuleInput>,
) -> Result<(), String> {
    let mut files = Vec::new();
    collect_files(dir, &mut files)?;
    files.sort();
    for f in files {
        let rel = f
            .strip_prefix(dir)
            .map_err(|_| format!("bad source path {}", f.display()))?;
        let mut parts: Vec<String> = Vec::new();
        for c in rel.components() {
            let s = c.as_os_str().to_string_lossy().to_string();
            if let Some(stem) = s.strip_suffix(".qbs") {
                if stem != "mod" {
                    parts.push(stem.replace('-', "_"));
                }
            } else {
                parts.push(s.replace('-', "_"));
            }
        }
        let module_path = std::iter::once(package_root.to_string())
            .chain(parts)
            .collect::<Vec<_>>()
            .join("::");
        let src = std::fs::read_to_string(&f)
            .map_err(|e| format!("cannot read {}: {}", f.display(), e))?;
        let file = map.add_file(f.display().to_string(), &src);
        let program = qlobis_parser::parse_file(file, &src, diags);
        out.push(ModuleInput {
            path: module_path,
            file,
            program,
            is_stdlib,
        });
    }
    Ok(())
}

fn collect_files(dir: &Path, out: &mut Vec<PathBuf>) -> Result<(), String> {
    let entries = std::fs::read_dir(dir)
        .map_err(|e| format!("cannot read dir {}: {}", dir.display(), e))?;
    for e in entries {
        let e = e.map_err(|e| format!("io error: {}", e))?;
        let p = e.path();
        if p.is_dir() {
            collect_files(&p, out)?;
        } else if p.extension().map(|x| x == "qbs").unwrap_or(false) {
            out.push(p);
        }
    }
    Ok(())
}

/// Result of a build: files written per target.
pub struct BuildReport {
    pub files_written: Vec<PathBuf>,
    pub targets: Vec<Target>,
}

/// Generate all configured targets and write them to their output dirs.
pub fn build(cfg: &BuildConfig, prog: &Program) -> Result<BuildReport, String> {
    let mut files_written = Vec::new();
    for &target in &cfg.targets {
        let files = generate(
            target,
            &GenInput {
                modules: &prog.modules,
                checked: &prog.checked,
                entry: &cfg.entry(),
                package: &cfg.name,
                release: cfg.profile == Profile::Release,
            },
        );
        let dir = cfg.target_dir(target);
        write_files(&dir, &files, &mut files_written)?;
        if target == Target::Native {
            compile_native(cfg, &dir)?;
        }
    }
    Ok(BuildReport {
        files_written,
        targets: cfg.targets.clone(),
    })
}

fn write_files(
    dir: &Path,
    files: &[GenFile],
    written: &mut Vec<PathBuf>,
) -> Result<(), String> {
    std::fs::create_dir_all(dir)
        .map_err(|e| format!("cannot create {}: {}", dir.display(), e))?;
    for f in files {
        let path = dir.join(&f.path);
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)
                .map_err(|e| format!("cannot create {}: {}", parent.display(), e))?;
        }
        std::fs::write(&path, &f.contents)
            .map_err(|e| format!("cannot write {}: {}", path.display(), e))?;
        written.push(path);
    }
    Ok(())
}

/// Invoke the external C++ toolchain on the generated native sources.
pub fn compile_native(cfg: &BuildConfig, dir: &Path) -> Result<PathBuf, String> {
    let (compiler, mut args): (&str, Vec<String>) = if cfg!(windows) {
        ("cl", vec!["/std:c++17".into(), "/EHsc".into()])
    } else if have("clang++") {
        ("clang++", vec!["-std=c++17".into(), "-pthread".into()])
    } else {
        ("g++", vec!["-std=c++17".into(), "-pthread".into()])
    };
    args.push(cfg.profile.opt_level().to_string());
    if cfg.profile == Profile::Debug {
        args.push("-g".to_string());
    }
    let exe = if cfg!(windows) { "app.exe" } else { "app" };
    let out = dir.join(exe);
    if cfg!(windows) {
        args.push(format!("/Fe:{}", out.display()));
        args.push("program.cpp".to_string());
    } else {
        args.push("-o".to_string());
        args.push(out.display().to_string());
        args.push("program.cpp".to_string());
    }
    let st = std::process::Command::new(compiler)
        .args(&args)
        .current_dir(dir)
        .output()
        .map_err(|e| {
            format!(
                "cannot run `{}` (install clang++/g++ or MSVC): {}",
                compiler, e
            )
        })?;
    if !st.status.success() {
        return Err(format!(
            "{} failed:\n{}",
            compiler,
            String::from_utf8_lossy(&st.stderr)
        ));
    }
    Ok(out)
}

fn have(cmd: &str) -> bool {
    std::process::Command::new(cmd)
        .arg("--version")
        .output()
        .map(|o| o.status.success())
        .unwrap_or(false)
}

/// Check-only (no code generation): parse + typecheck with diagnostics.
pub fn check_only(cfg: &BuildConfig, prog: &Program) -> Result<(), String> {
    if prog.checked.entry.is_none() && cfg.targets.iter().any(|t| *t == Target::Native) {
        // Entry is validated per-target by codegen; nothing extra to do here.
    }
    Ok(())
}

/// `qlobis run`: build the Python target and execute `main_qlobis.py`, or
/// run the native binary when `--target native`.
pub fn run(cfg: &BuildConfig, prog: &Program, target: Target) -> Result<i32, String> {
    let report = build(cfg, prog)?;
    let _ = report;
    let dir = cfg.target_dir(target);
    let (cmd, args): (String, Vec<String>) = match target {
        Target::Python => ("python3".into(), vec!["main_qlobis.py".into()]),
        Target::Php => ("php".into(), vec!["index.php".into()]),
        Target::Web => ("node".into(), vec!["runner.js".into()]),
        Target::Kotlin => {
            return Err("run: kotlin target needs a device/emulator; build only".into())
        }
        Target::Native => {
            let exe = if cfg!(windows) { "app.exe" } else { "app" };
            (dir.join(exe).display().to_string(), vec![])
        }
    };
    if target == Target::Web && !dir.join("runner.js").exists() {
        write_node_runner(&dir)?;
    }
    let st = std::process::Command::new(&cmd)
        .args(&args)
        .current_dir(&dir)
        .status()
        .map_err(|e| format!("cannot run `{}`: {}", cmd, e))?;
    Ok(st.code().unwrap_or(1))
}

fn write_node_runner(dir: &Path) -> Result<(), String> {
    // Find the entry function from the generated program.
    let prog_js = std::fs::read_to_string(dir.join("js/program.js"))
        .map_err(|e| format!("cannot read program.js: {}", e))?;
    let entry = prog_js
        .lines()
        .find_map(|l| {
            let l = l.trim_start();
            l.strip_prefix("async function qlob_").and_then(|rest| {
                let name = rest.split('(').next()?;
                Some(format!("qlob_{}", name))
            })
        })
        .unwrap_or_else(|| "main".to_string());
    let runner = format!(
        "const fs = require('fs');\neval(fs.readFileSync('js/program.js', 'utf8'));\n(async () => {{\n  try {{\n    const r = await {entry}();\n    if (typeof r === 'number') process.exitCode = r;\n  }} catch (e) {{\n    console.error('error: ' + (e && e.message ? e.message : e));\n    process.exitCode = 1;\n  }}\n}})();\n",
        entry = entry
    );
    std::fs::write(dir.join("runner.js"), runner)
        .map_err(|e| format!("cannot write runner.js: {}", e))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn end_to_end_project_build() {
        let root = std::env::temp_dir().join("qlobis-e2e-project");
        let _ = std::fs::remove_dir_all(&root);
        std::fs::create_dir_all(root.join("src")).unwrap();
        std::fs::create_dir_all(root.join("stdlib")).unwrap();
        std::fs::write(
            root.join("qlobconfig.toml"),
            r#"
[package]
name = "demo"
version = "0.1.0"

[targets]
python = true
native = true

[build]
profile = "release"
"#,
        )
        .unwrap();
        std::fs::write(
            root.join("stdlib/io.qbs"),
            "defin println<T>(v: T) {\n  __println(str(v))\n}\n",
        )
        .unwrap();
        std::fs::write(
            root.join("src/main.qbs"),
            r#"@module.import("std::io")

defin main() {
  io.println("from demo")
}
"#,
        )
        .unwrap();
        let cfg = config::load_config(&root).unwrap();
        let prog = load_program(&cfg, &[]).unwrap();
        let report = build(&cfg, &prog).unwrap();
        assert!(report.files_written.len() > 3);
        assert!(root.join("qlob/python/main_qlobis.py").exists());
        assert!(root.join("qlob/native").join(std::env::consts::OS).join(std::env::consts::ARCH).join("app").exists());
    }
}
