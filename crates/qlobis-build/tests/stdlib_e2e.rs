//! Stdlib validation: load every `stdlib/*.qbs` module together with a
//! usage program and EXECUTE the result (python + native when available).

use qlobis_codegen::{generate, GenInput};
use qlobis_tyck::{check, ModuleInput, Target};
use std::path::PathBuf;

fn stdlib_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../stdlib")
}

fn load_stdlib(
    map: &mut qlobis_ast::SourceMap,
    diags: &mut qlobis_ast::Diagnostics,
) -> Vec<ModuleInput> {
    let dir = stdlib_root();
    let mut modules = Vec::new();
    let mut files: Vec<_> = std::fs::read_dir(&dir)
        .expect("stdlib/ present")
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

const USAGE: &str = r#"
@module.import("std::io")
@module.import("std::string")
@module.import("std::math")
@module.import("std::collections")
@module.import("std::crypto")
@module.import("std::threading")

defin double(x: int) -> int {
    return x * 2
}

defin add(a: int, b: int) -> int {
    return a + b
}

defin less(a: int, b: int) -> int {
    return a - b
}

defin is_big(x: int) -> bool {
    return x > 2
}

async defin fetch_num() -> int {
    return 20
}

async defin main() {
    io.println("== stdlib e2e ==")
    io.println(string.upper("abc"))
    io.println(string.pad_left("7", 3, "0"))
    io.println(string.reverse("qlobis"))
    io.println(string.count("banana", "an"))
    io.println(string.ord("A"))
    io.println(string.chr(66))
    io.println(math.abs(-5))
    io.println(math.pow(2, 10))
    io.println(math.sqrt(16.0))
    io.println(math.gcd(12, 18))
    io.println(math.is_prime(97))
    let xs = [3, 1, 2]
    io.println(collections.map(xs, double))
    io.println(collections.filter(xs, is_big))
    io.println(collections.reduce(xs, 0, add))
    io.println(collections.sum(xs))
    io.println(collections.sorted(xs, less))
    io.println(collections.unique([1, 1, 2, 2, 3]))
    io.println(collections.range(0, 4))
    let m = {"a": 1}
    m.put("b", 2)
    io.println(collections.keys(m))
    io.println(collections.get_or(m, "z", -1))
    io.println(crypto.fnv1a("hi"))
    io.println(crypto.hex_encode("AB"))
    io.println(crypto.hex_decode("4142"))
    io.println(crypto.base64_encode("hello"))
    io.println(crypto.base64_decode("aGVsbG8="))
    io.println(crypto.rot13("hello"))
    io.println(crypto.xor_cipher("abc", "k"))
    io.println(threading.parallel_map(xs, double))
    io.println(threading.thread_count() >= 1)
    io.println(await fetch_num() + 1)
}
"#;

#[test]
fn stdlib_typechecks_and_executes() {
    let mut map = qlobis_ast::SourceMap::new();
    let mut diags = qlobis_ast::Diagnostics::new();
    let mut modules = load_stdlib(&mut map, &mut diags);
    let file = map.add_file("usage.qbs", USAGE);
    let program = qlobis_parser::parse_file(file, USAGE, &mut diags);
    modules.push(ModuleInput {
        path: "app::main".to_string(),
        file,
        program,
        is_stdlib: false,
    });
    let checked = check(
        &modules,
        "app",
        &[Target::Python, Target::Native],
        &mut diags,
    );
    assert!(
        !diags.has_errors(),
        "stdlib type errors:\n{}",
        diags.render(&map, true)
    );

    let files = generate(
        Target::Python,
        &GenInput {
            modules: &modules,
            checked: &checked,
            entry: &("app::main".to_string(), "main".to_string()),
            package: "app",
            release: false,
        },
    );
    let dir = std::env::temp_dir().join("qlobis-stdlib-e2e");
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
        .expect("python3 available");
    let stdout = String::from_utf8_lossy(&out.stdout).to_string();
    let stderr = String::from_utf8_lossy(&out.stderr).to_string();
    assert!(
        out.status.success(),
        "python failed:\n{}\n{}",
        stdout,
        stderr
    );

    let expect = "\
== stdlib e2e ==
ABC
007
sibolq
2
65
B
5
1024
4.0
6
true
[6, 2, 4]
[3]
6
6
[1, 2, 3]
[1, 2, 3]
[0, 1, 2, 3]
[a, b]
-1
";
    assert!(
        stdout.starts_with(expect),
        "stdlib output mismatch:\n--- got ---\n{}\n--- want ---\n{}",
        stdout,
        expect
    );
    // The crypto/threading/async tail must run without errors.
    assert!(stdout.contains("aGVsbG8="), "{}", stdout); // base64_decode
    assert!(stdout.contains("hello"), "{}", stdout); // base64 round trip
    assert!(stdout.contains("uryyb"), "{}", stdout); // rot13
    assert!(stdout.contains("[6, 2, 4]"), "{}", stdout); // parallel_map
    assert!(stdout.contains("21"), "{}", stdout); // await
}
