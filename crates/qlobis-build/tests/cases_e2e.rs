//! Golden-file integration tests: run every `tests/cases/*.qbs` with the
//! bundled stdlib and compare against `tests/cases/*.expected`.
//!
//! `tests/run.sh` does the same through the CLI binary.

use qlobis_codegen::{generate, GenInput};
use qlobis_tyck::{check, ModuleInput, Target};
use std::path::PathBuf;

fn repo_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../..")
}

fn stdlib_modules(
    map: &mut qlobis_ast::SourceMap,
    diags: &mut qlobis_ast::Diagnostics,
) -> Vec<ModuleInput> {
    let dir = repo_root().join("stdlib");
    let mut modules = Vec::new();
    let mut files: Vec<_> = std::fs::read_dir(&dir)
        .unwrap()
        .flatten()
        .map(|e| e.path())
        .filter(|p| p.extension().map(|x| x == "qbs").unwrap_or(false))
        .collect();
    files.sort();
    for f in files {
        let stem = f.file_stem().unwrap().to_string_lossy().to_string();
        let src = std::fs::read_to_string(&f).unwrap();
        let file = map.add_file(f.display().to_string(), &src);
        let program = qlobis_parser::parse_file(file, &src, diags);
        modules.push(ModuleInput {
            path: format!("std::{}", stem.replace('-', "_")),
            file,
            program,
            is_stdlib: true,
        });
    }
    modules
}

#[test]
fn golden_cases_execute_and_match() {
    if std::process::Command::new("python3")
        .arg("--version")
        .output()
        .map(|o| !o.status.success())
        .unwrap_or(true)
    {
        eprintln!("skip: python3 not available");
        return;
    }
    let cases_dir = repo_root().join("tests/cases");
    let mut cases: Vec<PathBuf> = std::fs::read_dir(&cases_dir)
        .expect("tests/cases present")
        .flatten()
        .map(|e| e.path())
        .filter(|p| p.extension().map(|x| x == "qbs").unwrap_or(false))
        .collect();
    cases.sort();
    assert!(!cases.is_empty(), "no cases found");

    for case in cases {
        let name = case.file_stem().unwrap().to_string_lossy().to_string();
        let src = std::fs::read_to_string(&case).unwrap();
        let expected_path = cases_dir.join(format!("{}.expected", name));
        let expected = std::fs::read_to_string(&expected_path)
            .unwrap_or_else(|_| panic!("missing {}", expected_path.display()));

        let mut map = qlobis_ast::SourceMap::new();
        let mut diags = qlobis_ast::Diagnostics::new();
        let mut modules = stdlib_modules(&mut map, &mut diags);
        let file = map.add_file(case.display().to_string(), &src);
        let program = qlobis_parser::parse_file(file, &src, &mut diags);
        modules.push(ModuleInput {
            path: "case::main".to_string(),
            file,
            program,
            is_stdlib: false,
        });
        let checked = check(&modules, "case", &[Target::Python], &mut diags);
        assert!(
            !diags.has_errors(),
            "case `{}` does not typecheck:\n{}",
            name,
            diags.render(&map, true)
        );
        let files = generate(
            Target::Python,
            &GenInput {
                modules: &modules,
                checked: &checked,
                entry: &("case::main".to_string(), "main".to_string()),
                package: "case",
                release: false,
            },
        );
        let dir = std::env::temp_dir().join(format!("qlobis-case-{}", name));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        for f in &files {
            let p = dir.join(&f.path);
            if let Some(parent) = p.parent() {
                std::fs::create_dir_all(parent).unwrap();
            }
            std::fs::write(p, &f.contents).unwrap();
        }
        let out = std::process::Command::new("python3")
            .arg("main_qlobis.py")
            .current_dir(&dir)
            .output()
            .unwrap();
        let stdout = String::from_utf8_lossy(&out.stdout).to_string();
        let stderr = String::from_utf8_lossy(&out.stderr).to_string();
        assert!(
            out.status.success(),
            "case `{}` crashed:\n{}\n{}",
            name,
            stdout,
            stderr
        );
        assert_eq!(
            stdout, expected,
            "case `{}` output mismatch (stderr: {})",
            name, stderr
        );
    }
}
