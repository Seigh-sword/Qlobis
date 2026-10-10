//! Cross-backend end-to-end tests: one source program, six targets.
//!
//! * Python / Web / Native outputs are *executed* (python3 / node / g++)
//!   and must agree on the program's exact stdout.
//! * PHP / Kotlin outputs are checked against golden expectations (no
//!   toolchain is available in the test environment).

use qlobis_codegen::{generate, GenInput};
use qlobis_tyck::{check, ModuleInput, Target};
use std::path::PathBuf;
use std::process::Command;

/// Minimal `std::io` module, standing in for the stdlib.
const STD_IO: &str = r#"
defin println<T>(v: T) {
    __println(str(v))
}

defin print<T>(v: T) {
    __print(str(v))
}
"#;

const SRC: &str = r#"
@module.import("std::io")

struct Point {
  x: int
  y: int
}

impl Point {
  defin sum(self) -> int {
    return self.x + self.y
  }
}

defin fact(n: int) -> int {
  if n <= 1 {
    return 1
  }
  return n * fact(n - 1)
}

async defin fetch() -> int {
  return 21
}

async defin main() {
  io.println("hello")
  let n = 7
  let d = n / 2
  io.println(d)
  io.println("n={n}")
  let xs = [1, 2, 3]
  xs.append(4)
  io.println(xs)
  let m = {"a": 1}
  m.put("b", 2)
  io.println(m.get("a") + m.get("b"))
  let p = Point(2, 3)
  io.println(p.sum())
  try {
    throw "boom"
  } throw err {
    io.println("caught: " + err)
  }
  let total = 0
  for x in xs {
    total = total + x
  }
  io.println(total)
  let i = 0
  while i < 3 {
    i = i + 1
  }
  io.println(i)
  io.println(fact(5))
  io.println(await fetch())
}
"#;

const EXPECTED: &str = "hello\n3\nn=7\n[1, 2, 3, 4]\n3\n5\ncaught: boom\n10\n3\n120\n21\n";

fn gen(target: Target, src: &str) -> Vec<qlobis_codegen::GenFile> {
    let mut diags = qlobis_ast::Diagnostics::new();
    let mut map = qlobis_ast::SourceMap::new();
    map.add_file("std/io.qbs", STD_IO);
    map.add_file("test.qbs", src);
    let std_program = qlobis_parser::parse_file(0, STD_IO, &mut diags);
    let program = qlobis_parser::parse_file(1, src, &mut diags);
    let modules = vec![
        ModuleInput {
            path: "std::io".to_string(),
            file: 0,
            program: std_program,
            is_stdlib: true,
        },
        ModuleInput {
            path: "test_pkg::main".to_string(),
            file: 1,
            program,
            is_stdlib: false,
        },
    ];
    let out = check(&modules, "test_pkg", &[target], &mut diags);
    assert!(!diags.has_errors(), "{}", diags.render(&map, false));
    generate(
        target,
        &GenInput {
            modules: &modules,
            checked: &out,
            entry: &("test_pkg::main".to_string(), "main".to_string()),
            package: "test_pkg",
            release: false,
        },
    )
}

fn write_files(dir: &std::path::Path, files: &[qlobis_codegen::GenFile]) {
    std::fs::create_dir_all(dir).unwrap();
    for f in files {
        let path = dir.join(&f.path);
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent).unwrap();
        }
        std::fs::write(path, &f.contents).unwrap();
    }
}

fn out_dir(name: &str) -> PathBuf {
    let d = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../../target/e2e")
        .join(name);
    std::fs::create_dir_all(&d).unwrap();
    d
}

fn have(cmd: &str) -> bool {
    Command::new(cmd)
        .arg("--version")
        .output()
        .map(|o| o.status.success())
        .unwrap_or(false)
}

#[test]
fn python_executes_and_matches() {
    if !have("python3") {
        eprintln!("skip: python3 not available");
        return;
    }
    let files = gen(Target::Python, SRC);
    let dir = out_dir("python");
    write_files(&dir, &files);
    let out = Command::new("python3")
        .arg("main_qlobis.py")
        .current_dir(&dir)
        .output()
        .expect("run python3");
    let stdout = String::from_utf8_lossy(&out.stdout);
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(
        out.status.success(),
        "python failed:\n{}\n{}",
        stdout,
        stderr
    );
    assert_eq!(stdout, EXPECTED, "python stdout mismatch (stderr: {})", stderr);
}

#[test]
fn web_executes_and_matches() {
    if !have("node") {
        eprintln!("skip: node not available");
        return;
    }
    let files = gen(Target::Web, SRC);
    let dir = out_dir("web");
    write_files(&dir, &files);
    // Drive the generated program from Node (the browser shell is app.js).
    let runner = format!(
        r#"const fs = require('fs');
eval(fs.readFileSync('js/program.js', 'utf8'));
(async () => {{
  try {{
    const r = await qlob_test_pkg__main_main();
    if (typeof r === 'number') process.exitCode = r;
  }} catch (e) {{
    console.error('error: ' + (e && e.message ? e.message : e));
    process.exitCode = 1;
  }}
}})();
"#
    );
    std::fs::write(dir.join("runner.js"), runner).unwrap();
    let out = Command::new("node")
        .arg("runner.js")
        .current_dir(&dir)
        .output()
        .expect("run node");
    let stdout = String::from_utf8_lossy(&out.stdout);
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(out.status.success(), "node failed:\n{}\n{}", stdout, stderr);
    assert_eq!(stdout, EXPECTED, "web stdout mismatch (stderr: {})", stderr);
}

#[test]
fn native_executes_and_matches() {
    if !have("g++") {
        eprintln!("skip: g++ not available");
        return;
    }
    let files = gen(Target::Native, SRC);
    let dir = out_dir("native");
    write_files(&dir, &files);
    let compile = Command::new("g++")
        .args([
            "-std=c++17",
            "-O1",
            "-pthread",
            "-o",
            "prog",
            "program.cpp",
        ])
        .current_dir(&dir)
        .output()
        .expect("run g++");
    let c_err = String::from_utf8_lossy(&compile.stderr);
    assert!(
        compile.status.success(),
        "g++ failed:\n{}",
        c_err
    );
    let out = Command::new(dir.join("prog"))
        .current_dir(&dir)
        .output()
        .expect("run prog");
    let stdout = String::from_utf8_lossy(&out.stdout);
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(out.status.success(), "prog failed:\n{}\n{}", stdout, stderr);
    assert_eq!(stdout, EXPECTED, "native stdout mismatch (stderr: {})", stderr);
}

#[test]
fn php_golden() {
    let files = gen(Target::Php, SRC);
    let main = files
        .iter()
        .find(|f| f.path == "test_pkg__main.php")
        .expect("module file")
        .contents
        .clone();
    assert!(main.contains("namespace qlobis\\test_pkg__main;"), "{}", main);
    assert!(main.contains("function main()"), "{}", main);
    assert!(main.contains("class Point"), "{}", main);
    assert!(main.contains("function sum()"), "{}", main);
    assert!(main.contains("qlob_idiv($n, 2)"), "{}", main);
    assert!(main.contains("catch (\\Throwable"), "{}", main);
    let index = files
        .iter()
        .find(|f| f.path == "index.php")
        .expect("index.php")
        .contents
        .clone();
    assert!(index.contains("qlobis_runtime.php"), "{}", index);
    assert!(index.contains("\\qlobis\\test_pkg__main\\main"), "{}", index);
    let runtime = files
        .iter()
        .find(|f| f.path == "qlobis_runtime.php")
        .expect("runtime")
        .contents
        .clone();
    assert!(runtime.contains("function qlob_idiv"), "{}", runtime);
}

#[test]
fn kotlin_golden() {
    let files = gen(Target::Kotlin, SRC);
    let main = files
        .iter()
        .find(|f| f.path == "test_pkg__main.kt")
        .expect("module file")
        .contents
        .clone();
    assert!(
        main.contains("package qlobis.generated.test_pkg__main"),
        "{}",
        main
    );
    assert!(main.contains("data class Point("), "{}", main);
    assert!(main.contains("fun sum()"), "{}", main);
    assert!(main.contains("suspend fun fetch()"), "{}", main);
    assert!(main.contains("QR.idiv(n, 2L)"), "{}", main);
    // `main` is suspend: async calls appear plain (no runBlocking wrapper).
    assert!(main.contains("fetch()"), "{}", main);
    assert!(main.contains("import qlobis.generated.QlobRuntime as QR"), "{}", main);
    let main_kt = files
        .iter()
        .find(|f| f.path == "Main.kt")
        .expect("Main.kt")
        .contents
        .clone();
    assert!(main_kt.contains("fun main()"), "{}", main_kt);
    assert!(main_kt.contains("runBlocking"), "{}", main_kt);
    let runtime = files
        .iter()
        .find(|f| f.path == "QlobRuntime.kt")
        .expect("QlobRuntime.kt")
        .contents
        .clone();
    assert!(runtime.contains("fun idiv"), "{}", runtime);
}

#[test]
fn tsx_and_js_flavors_agree() {
    let files = gen(Target::Web, SRC);
    let ts = files
        .iter()
        .find(|f| f.path == "src/program.ts")
        .expect("ts")
        .contents
        .clone();
    let js = files
        .iter()
        .find(|f| f.path == "js/program.js")
        .expect("js")
        .contents
        .clone();
    assert!(ts.contains("class Point"), "{}", ts);
    assert!(js.contains("class Point"), "{}", js);
    assert!(ts.contains("async function qlob_test_pkg__main_main"), "{}", ts);
    assert!(js.contains("async function qlob_test_pkg__main_main"), "{}", js);
    // The TS flavor carries types; the JS flavor must not.
    assert!(ts.contains("x: number"), "{}", ts);
    assert!(!js.contains("x: number"), "{}", js);
    let app = files
        .iter()
        .find(|f| f.path == "src/App.tsx")
        .expect("App.tsx")
        .contents
        .clone();
    assert!(app.contains("qlob_test_pkg__main_main"), "{}", app);
    let index = files
        .iter()
        .find(|f| f.path == "index.html")
        .expect("index.html")
        .contents
        .clone();
    assert!(index.contains("js/program.js"), "{}", index);
    assert!(files.iter().any(|f| f.path == "styles.css"));
    assert!(files.iter().any(|f| f.path == "package.json"));
    assert!(files.iter().any(|f| f.path == "tsconfig.json"));
}

#[test]
fn escape_hatches_pass_through_per_target() {
    // Calls to unknown identifiers are target-specific escape hatches:
    // they lower to raw host calls and are emitted verbatim.
    let src = r#"
@module.import("std::io")

@target(python)
defin now() -> str {
    return py_now()
}

@target(web)
defin now() -> str {
    return js_now()
}

@target(native)
defin now() -> str {
    return cpp_now()
}

defin main() {
    io.println(now())
}
"#;
    for (target, marker) in [
        (Target::Python, "py_now()"),
        (Target::Web, "js_now()"),
        (Target::Native, "cpp_now()"),
    ] {
        let files = gen(target, src);
        let all: String = files.iter().map(|f| f.contents.clone()).collect();
        assert!(
            all.contains(marker),
            "target {} missing raw passthrough {}:\n{}",
            target.as_str(),
            marker,
            all
        );
        // The other targets' escape hatches must NOT leak through.
        for (other, other_marker) in [
            (Target::Python, "py_now()"),
            (Target::Web, "js_now()"),
            (Target::Native, "cpp_now()"),
        ] {
            if other.as_str() != target.as_str() {
                assert!(
                    !all.contains(other_marker),
                    "target {} leaked {}:\n{}",
                    target.as_str(),
                    other_marker,
                    all
                );
            }
        }
    }
}
