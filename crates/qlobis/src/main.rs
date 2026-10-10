//! `qlobis` command-line interface.

mod ansi;

use ansi::{bold, green, red, yellow};
use qlobis_build::config::{self, BuildConfig, Profile};
use qlobis_build::registry::{self, Registry};
use qlobis_build::{docs, fmt, lint, lsp, Program};
use qlobis_tyck::Target;
use std::path::{Path, PathBuf};
use std::process::exit;

const USAGE: &str = "\
qlobis — multi-target language compiler

USAGE:
    qlobis <command> [options]

COMMANDS:
    build               Compile the project to all configured targets
    run [--target T]    Build and run (python|php|web|native; default python)
    check               Parse + typecheck without generating code
    new <name>          Create a new project skeleton
    add <name> [--git URL] [--tag T]   Add a dependency (registry or git)
    publish             Package and record this project for the registry
    lint                Run the linter
    fmt [--check]       Format (or check formatting of) project sources
    docs                Generate Markdown API documentation
    lsp                 Run the language server on stdio
    version             Print version information
    clean               Remove build outputs

OPTIONS:
    --target <T>        python | php | web | kotlin | native
    --release           Build with the release profile
    --root <dir>        Project root (default: search upward)
    -h, --help          Show this help
";

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let code = run(&args);
    exit(code);
}

fn run(args: &[String]) -> i32 {
    if args.is_empty() || args[0] == "-h" || args[0] == "--help" || args[0] == "help" {
        print!("{}", USAGE);
        return 0;
    }
    let cmd = args[0].as_str();
    let rest = &args[1..];
    match cmd {
        "build" => cmd_build(rest),
        "run" => cmd_run(rest),
        "check" => cmd_check(rest),
        "new" => cmd_new(rest),
        "add" => cmd_add(rest),
        "publish" => cmd_publish(rest),
        "lint" => cmd_lint(rest),
        "fmt" => cmd_fmt(rest),
        "docs" => cmd_docs(rest),
        "lsp" => lsp::serve(),
        "version" | "--version" | "-V" => {
            println!("qlobis {}", env!("CARGO_PKG_VERSION"));
            0
        }
        "clean" => cmd_clean(rest),
        other => {
            eprintln!("{} unknown command `{}`\n", red("error:"), other);
            eprint!("{}", USAGE);
            2
        }
    }
}

// ---------------------------------------------------------------------------
// shared option handling
// ---------------------------------------------------------------------------

struct Opts {
    root: PathBuf,
    release: bool,
    target: Option<Target>,
    check: bool,
    positional: Vec<String>,
    git: Option<String>,
    tag: Option<String>,
}

fn parse_opts(args: &[String]) -> Result<Opts, String> {
    let mut root = None;
    let mut release = false;
    let mut target = None;
    let mut check = false;
    let mut positional = Vec::new();
    let mut git = None;
    let mut tag = None;
    let mut i = 0;
    while i < args.len() {
        match args[i].as_str() {
            "--root" => {
                i += 1;
                root = Some(PathBuf::from(args.get(i).ok_or("--root needs a value")?));
            }
            "--release" => release = true,
            "--check" => check = true,
            "--target" => {
                i += 1;
                let t = args.get(i).ok_or("--target needs a value")?;
                target = Some(parse_target(t)?);
            }
            "--git" => {
                i += 1;
                git = Some(args.get(i).ok_or("--git needs a value")?.clone());
            }
            "--tag" => {
                i += 1;
                tag = Some(args.get(i).ok_or("--tag needs a value")?.clone());
            }
            "-h" | "--help" => {
                print!("{}", USAGE);
                exit(0);
            }
            a if a.starts_with('-') => {
                return Err(format!("unknown option `{}`", a));
            }
            a => positional.push(a.to_string()),
        }
        i += 1;
    }
    let root = match root {
        Some(r) => r,
        None => config::find_project_root(&std::env::current_dir().unwrap_or_default())
            .ok_or("not inside a qlobis project (no qlobconfig.toml found); use `qlobis new`")?,
    };
    Ok(Opts {
        root,
        release,
        target,
        check,
        positional,
        git,
        tag,
    })
}

fn parse_target(s: &str) -> Result<Target, String> {
    Target::parse(s).ok_or_else(|| {
        format!(
            "unknown target `{}` (expected python|php|web|kotlin|native)",
            s
        )
    })
}

fn load(opts: &Opts) -> Result<(BuildConfig, Program, Vec<(String, PathBuf)>), String> {
    let mut cfg = config::load_config(&opts.root)?;
    if opts.release {
        cfg.profile = Profile::Release;
    }
    if let Some(t) = opts.target {
        cfg.targets = vec![t];
    }
    let registry = Registry::load(&opts.root)?;
    let resolved = registry::resolve(&cfg.deps, &registry)?;
    registry::write_lock(&opts.root, &resolved)?;
    let extra: Vec<(String, PathBuf)> = resolved
        .into_iter()
        .map(|d| (d.name.clone(), d.root.clone()))
        .collect();
    let prog = qlobis_build::load_program(&cfg, &extra)?;
    Ok((cfg, prog, extra))
}

// ---------------------------------------------------------------------------
// commands
// ---------------------------------------------------------------------------

fn cmd_build(args: &[String]) -> i32 {
    let opts = match parse_opts(args) {
        Ok(o) => o,
        Err(e) => return fail(&e),
    };
    let (cfg, prog, _) = match load(&opts) {
        Ok(x) => x,
        Err(e) => return fail(&e),
    };
    match qlobis_build::build(&cfg, &prog) {
        Ok(report) => {
            println!(
                "{} built {} target(s), {} file(s)",
                green("ok:"),
                report.targets.len(),
                report.files_written.len()
            );
            for t in &report.targets {
                println!("  {} -> {}", t.as_str(), cfg.target_dir(*t).display());
            }
            0
        }
        Err(e) => fail(&e),
    }
}

fn cmd_run(args: &[String]) -> i32 {
    let opts = match parse_opts(args) {
        Ok(o) => o,
        Err(e) => return fail(&e),
    };
    let target = opts.target.unwrap_or(Target::Python);
    let (mut cfg, prog, _) = match load(&opts) {
        Ok(x) => x,
        Err(e) => return fail(&e),
    };
    // `run` builds only the target it executes.
    cfg.targets = vec![target];
    match qlobis_build::run(&cfg, &prog, target) {
        Ok(code) => code,
        Err(e) => fail(&e),
    }
}

fn cmd_check(args: &[String]) -> i32 {
    let opts = match parse_opts(args) {
        Ok(o) => o,
        Err(e) => return fail(&e),
    };
    let (cfg, prog, _) = match load(&opts) {
        Ok(x) => x,
        Err(e) => return fail(&e),
    };
    let _ = qlobis_build::check_only(&cfg, &prog);
    println!("{} checked {} module(s)", green("ok:"), prog.modules.len());
    0
}

fn cmd_new(args: &[String]) -> i32 {
    let name = match args.first() {
        Some(n) if !n.starts_with('-') => n.clone(),
        _ => return fail("usage: qlobis new <name>"),
    };
    let root = PathBuf::from(&name);
    if root.exists() {
        return fail(&format!("`{}` already exists", name));
    }
    if let Err(e) = make_project(&root, &name) {
        return fail(&e);
    }
    println!("{} created project `{}`", green("ok:"), name);
    println!("  cd {} && qlobis run", name);
    0
}

fn make_project(root: &Path, name: &str) -> Result<(), String> {
    std::fs::create_dir_all(root.join("src")).map_err(|e| e.to_string())?;
    std::fs::create_dir_all(root.join("stdlib")).map_err(|e| e.to_string())?;
    std::fs::write(
        root.join("qlobconfig.toml"),
        format!(
            "[package]\nname = \"{name}\"\nversion = \"0.1.0\"\n\n[targets]\npython = true\nweb = true\nnative = true\n\n[build]\nsources = [\"src\"]\nprofile = \"debug\"\n",
            name = name
        ),
    )
    .map_err(|e| e.to_string())?;
    std::fs::write(
        root.join("qlobregistry.toml"),
        "# Third-party packages for this project.\n[packages]\n",
    )
    .map_err(|e| e.to_string())?;
    std::fs::write(
        root.join("src/main.qbs"),
        format!(
            "@module.import(\"std::io\")\n\n/// Project entry point.\ndefin main() {{\n  io.println(\"hello from {name}!\")\n}}\n",
            name = name
        ),
    )
    .map_err(|e| e.to_string())?;
    std::fs::write(
        root.join("stdlib/io.qbs"),
        "/// Write a line to standard output.\ndefin println<T>(v: T) {\n  __println(str(v))\n}\n\n/// Write to standard output without a newline.\ndefin print<T>(v: T) {\n  __print(str(v))\n}\n",
    )
    .map_err(|e| e.to_string())?;
    std::fs::write(root.join(".gitignore"), "/qlob\n/qlobis.lock\n").map_err(|e| e.to_string())?;
    Ok(())
}

fn cmd_add(args: &[String]) -> i32 {
    let opts = match parse_opts(args) {
        Ok(o) => o,
        Err(e) => return fail(&e),
    };
    let Some(name) = opts.positional.first() else {
        return fail("usage: qlobis add <name> [--git URL] [--tag T]");
    };
    let mut entry = format!("{} = {{", name);
    if let Some(g) = &opts.git {
        entry.push_str(&format!(" git = \"{}\"", g));
        if let Some(t) = &opts.tag {
            entry.push_str(&format!(", tag = \"{}\"", t));
        }
    }
    entry.push_str(" }\n");
    let cfg_path = opts.root.join("qlobconfig.toml");
    let mut src = std::fs::read_to_string(&cfg_path).unwrap_or_default();
    if src.contains("[dependencies]") {
        src.push_str(&entry);
    } else {
        src.push_str(&format!("\n[dependencies]\n{}", entry));
    }
    std::fs::write(&cfg_path, &src)
        .map_err(|e| e.to_string())
        .unwrap();
    // Fetch now so the cache is warm and errors surface early.
    let cfg = match config::load_config(&opts.root) {
        Ok(c) => c,
        Err(e) => return fail(&e),
    };
    let registry = Registry::load(&opts.root).unwrap_or_default();
    match registry::resolve(&cfg.deps, &registry) {
        Ok(deps) => {
            let d = deps.iter().find(|d| d.name == *name);
            println!("{} added `{}`", green("ok:"), name);
            if let Some(d) = d {
                println!("  source: {}", d.root.display());
            }
            0
        }
        Err(e) => fail(&e),
    }
}

fn cmd_publish(args: &[String]) -> i32 {
    let opts = match parse_opts(args) {
        Ok(o) => o,
        Err(e) => return fail(&e),
    };
    // Build everything to prove the package compiles, then record metadata.
    let (cfg, prog, _) = match load(&opts) {
        Ok(x) => x,
        Err(e) => return fail(&e),
    };
    if let Err(e) = qlobis_build::build(&cfg, &prog) {
        return fail(&format!("package does not build: {}", e));
    }
    let pkg_dir = dirs_publish_dir().join(format!("{}-{}", cfg.name, cfg.version));
    if let Err(e) = std::fs::create_dir_all(&pkg_dir) {
        return fail(&e.to_string());
    }
    for s in &cfg.sources {
        let src = cfg.root.join(s);
        if src.is_dir() {
            if let Err(e) = copy_dir(&src, &pkg_dir.join(s)) {
                return fail(&e);
            }
        }
    }
    let meta = format!(
        "[package]\nname = \"{}\"\nversion = \"{}\"\n\n[published]\npath = \"{}\"\n",
        cfg.name,
        cfg.version,
        pkg_dir.display()
    );
    let reg_path = opts.root.join("qlobregistry.toml");
    let mut reg = std::fs::read_to_string(&reg_path).unwrap_or_default();
    if !reg.contains("[packages]") {
        reg.push_str("\n[packages]\n");
    }
    reg.push_str(&format!(
        "{} = {{ path = \"{}\", description = \"published {}\" }}\n",
        cfg.name,
        pkg_dir.display(),
        cfg.version
    ));
    let _ = meta;
    if let Err(e) = std::fs::write(&reg_path, reg) {
        return fail(&e.to_string());
    }
    println!(
        "{} published `{} {}` to {}",
        green("ok:"),
        cfg.name,
        cfg.version,
        pkg_dir.display()
    );
    0
}

fn dirs_publish_dir() -> PathBuf {
    registry::qlobis_home().join("packages")
}

fn copy_dir(src: &Path, dst: &Path) -> Result<(), String> {
    std::fs::create_dir_all(dst).map_err(|e| e.to_string())?;
    for e in std::fs::read_dir(src).map_err(|e| e.to_string())? {
        let e = e.map_err(|e| e.to_string())?;
        let to = dst.join(e.file_name());
        if e.path().is_dir() {
            copy_dir(&e.path(), &to)?;
        } else {
            std::fs::copy(e.path(), &to).map_err(|e| e.to_string())?;
        }
    }
    Ok(())
}

fn cmd_lint(args: &[String]) -> i32 {
    let opts = match parse_opts(args) {
        Ok(o) => o,
        Err(e) => return fail(&e),
    };
    let (_cfg, prog, _) = match load(&opts) {
        Ok(x) => x,
        Err(e) => return fail(&e),
    };
    let findings = lint::lint(&prog.modules, &prog.checked, &prog.map);
    if findings.is_empty() {
        println!("{} no warnings", green("ok:"));
        0
    } else {
        for f in &findings {
            println!("{} {}", yellow("warning:"), f);
        }
        println!("{}", lint::render(&findings));
        1
    }
}

fn cmd_fmt(args: &[String]) -> i32 {
    let opts = match parse_opts(args) {
        Ok(o) => o,
        Err(e) => return fail(&e),
    };
    let mut sources = Vec::new();
    let cfg = config::load_config(&opts.root);
    if let Ok(cfg) = &cfg {
        for s in &cfg.sources {
            let dir = opts.root.join(s);
            collect_qbs(&dir, &mut sources);
        }
    } else {
        collect_qbs(&opts.root, &mut sources);
    }
    let mut unformatted = 0;
    for f in &sources {
        let src = std::fs::read_to_string(f).unwrap_or_default();
        match fmt::format_source(&src) {
            Ok(out) => {
                if out != src {
                    unformatted += 1;
                    if opts.check {
                        println!("{}", f.display());
                    } else if let Err(e) = std::fs::write(f, &out) {
                        return fail(&e.to_string());
                    }
                }
            }
            Err(e) => {
                eprintln!("{} {}: {}", red("error:"), f.display(), e);
                return 1;
            }
        }
    }
    if opts.check {
        if unformatted == 0 {
            println!("{} all files formatted", green("ok:"));
            0
        } else {
            println!(
                "{} {} file(s) need formatting (run `qlobis fmt`)",
                yellow("warning:"),
                unformatted
            );
            1
        }
    } else {
        println!("{} formatted {} file(s)", green("ok:"), sources.len());
        0
    }
}

fn collect_qbs(dir: &Path, out: &mut Vec<PathBuf>) {
    if let Ok(rd) = std::fs::read_dir(dir) {
        for e in rd.flatten() {
            let p = e.path();
            if p.is_dir() {
                collect_qbs(&p, out);
            } else if p.extension().map(|x| x == "qbs").unwrap_or(false) {
                out.push(p);
            }
        }
    }
}

fn cmd_docs(args: &[String]) -> i32 {
    let opts = match parse_opts(args) {
        Ok(o) => o,
        Err(e) => return fail(&e),
    };
    let (cfg, prog, _) = match load(&opts) {
        Ok(x) => x,
        Err(e) => return fail(&e),
    };
    let out_dir = cfg.output_dir().join("docs");
    match docs::write_docs(&out_dir, &prog.modules, &cfg.name, &cfg.version) {
        Ok(written) => {
            println!(
                "{} docs in {} ({} file(s))",
                green("ok:"),
                out_dir.display(),
                written.len()
            );
            0
        }
        Err(e) => fail(&e),
    }
}

fn cmd_clean(args: &[String]) -> i32 {
    let opts = match parse_opts(args) {
        Ok(o) => o,
        Err(e) => return fail(&e),
    };
    let out = opts.root.join("qlob");
    if out.exists() {
        match std::fs::remove_dir_all(&out) {
            Ok(()) => println!("{} removed {}", green("ok:"), out.display()),
            Err(e) => return fail(&e.to_string()),
        }
    } else {
        println!("{} nothing to clean", green("ok:"));
    }
    0
}

fn fail(msg: &str) -> i32 {
    eprintln!("{} {}", red("error:"), msg);
    1
}

#[allow(dead_code)]
fn info(msg: &str) {
    println!("{} {}", bold("info:"), msg);
}
