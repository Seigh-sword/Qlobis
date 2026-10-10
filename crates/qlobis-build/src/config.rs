//! Build configuration (`qlobconfig.toml`).

use crate::toml::{parse_toml, Table, Value};
use qlobis_tyck::Target;
use std::path::{Path, PathBuf};

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Profile {
    Debug,
    Release,
}

impl Profile {
    pub fn parse(s: &str) -> Option<Profile> {
        match s {
            "debug" => Some(Profile::Debug),
            "release" => Some(Profile::Release),
            _ => None,
        }
    }
    /// Native optimization level.
    pub fn opt_level(&self) -> &'static str {
        match self {
            Profile::Debug => "-O0",
            Profile::Release => "-O2",
        }
    }
}

#[derive(Debug, Clone)]
pub struct Dependency {
    pub name: String,
    /// Git URL (required for registry dependencies).
    pub git: Option<String>,
    /// Tag / branch / rev to check out.
    pub tag: Option<String>,
    pub rev: Option<String>,
    /// Local path dependency.
    pub path: Option<String>,
}

#[derive(Debug, Clone)]
pub struct BuildConfig {
    pub name: String,
    pub version: String,
    /// Source directories (relative to the project root).
    pub sources: Vec<String>,
    /// Stdlib source directory.
    pub stdlib: Option<String>,
    /// Output directory root.
    pub output: String,
    pub profile: Profile,
    pub targets: Vec<Target>,
    pub deps: Vec<Dependency>,
    /// Project root (directory containing qlobconfig.toml).
    pub root: PathBuf,
}

impl BuildConfig {
    pub fn output_dir(&self) -> PathBuf {
        self.root.join(&self.output)
    }

    /// Where a target's files land: `qlob/python`, `qlob/web`,
    /// `qlob/mobile/<platform>`, `qlob/native/<os>/<arch>`.
    pub fn target_dir(&self, t: Target) -> PathBuf {
        let out = self.output_dir();
        match t {
            Target::Python => out.join("python"),
            Target::Php => out.join("php"),
            Target::Web => out.join("web"),
            Target::Kotlin => out.join("mobile").join("android"),
            Target::Native => out
                .join("native")
                .join(std::env::consts::OS)
                .join(std::env::consts::ARCH),
        }
    }

    /// The entry module: `<package>::main`, function `main`.
    pub fn entry(&self) -> (String, String) {
        (format!("{}::main", self.name), "main".to_string())
    }
}

fn get_str(t: &Table, k: &str) -> Option<String> {
    t.get(k).and_then(|v| v.as_str()).map(|s| s.to_string())
}

/// Parse `qlobconfig.toml` text for the project rooted at `root`.
pub fn parse_config(src: &str, root: &Path) -> Result<BuildConfig, String> {
    let t = parse_toml(src)?;
    let pkg = t
        .get("package")
        .and_then(|v| v.as_table())
        .ok_or("[package] table is required")?;
    let name = get_str(pkg, "name").ok_or("[package].name is required")?;
    let version = get_str(pkg, "version").unwrap_or_else(|| "0.0.0".to_string());

    let build = t.get("build").and_then(|v| v.as_table());
    let sources: Vec<String> = build
        .and_then(|b| b.get("sources"))
        .and_then(|v| v.as_array())
        .map(|a| {
            a.iter()
                .filter_map(|v| v.as_str().map(|s| s.to_string()))
                .collect()
        })
        .unwrap_or_else(|| vec!["src".to_string()]);
    let stdlib = build.and_then(|b| get_str(b, "stdlib"));
    let output = build
        .and_then(|b| get_str(b, "output"))
        .unwrap_or_else(|| "qlob".to_string());
    let profile = build
        .and_then(|b| get_str(b, "profile"))
        .and_then(|s| Profile::parse(&s))
        .unwrap_or(Profile::Debug);

    // Targets: `[targets] python = true` or `all = true`.
    let mut targets = Vec::new();
    if let Some(tt) = t.get("targets").and_then(|v| v.as_table()) {
        let all = tt.get("all").and_then(|v| v.as_bool()).unwrap_or(false);
        if all {
            targets = Target::ALL.to_vec();
        } else {
            // Canonical order regardless of key order in the file.
            for (name, t) in [
                ("python", Target::Python),
                ("php", Target::Php),
                ("web", Target::Web),
                ("kotlin", Target::Kotlin),
                ("mobile", Target::Kotlin),
                ("native", Target::Native),
            ] {
                if tt.get(name).and_then(|v| v.as_bool()) == Some(true) && !targets.contains(&t) {
                    targets.push(t);
                }
            }
            for k in tt.keys() {
                if !matches!(
                    k.as_str(),
                    "python" | "php" | "web" | "kotlin" | "mobile" | "native" | "all"
                ) {
                    return Err(format!("[targets]: unknown target `{}`", k));
                }
            }
        }
    }
    if targets.is_empty() {
        targets = vec![Target::Python];
    }

    let mut deps = Vec::new();
    if let Some(dt) = t.get("dependencies").and_then(|v| v.as_table()) {
        for (k, v) in dt {
            match v {
                Value::Table(inner) => {
                    deps.push(Dependency {
                        name: k.clone(),
                        git: get_str(inner, "git"),
                        tag: get_str(inner, "tag"),
                        rev: get_str(inner, "rev"),
                        path: get_str(inner, "path"),
                    });
                }
                Value::Str(p) => {
                    deps.push(Dependency {
                        name: k.clone(),
                        git: None,
                        tag: None,
                        rev: None,
                        path: Some(p.clone()),
                    });
                }
                _ => return Err(format!("bad dependency entry `{}`", k)),
            }
        }
    }

    Ok(BuildConfig {
        name,
        version,
        sources,
        stdlib,
        output,
        profile,
        targets,
        deps,
        root: root.to_path_buf(),
    })
}

/// Find the project root walking up from `start` looking for qlobconfig.toml.
pub fn find_project_root(start: &Path) -> Option<PathBuf> {
    let mut cur = Some(start);
    while let Some(dir) = cur {
        if dir.join("qlobconfig.toml").exists() {
            return Some(dir.to_path_buf());
        }
        cur = dir.parent();
    }
    None
}

pub fn load_config(root: &Path) -> Result<BuildConfig, String> {
    let path = root.join("qlobconfig.toml");
    let src = std::fs::read_to_string(&path)
        .map_err(|e| format!("cannot read {}: {}", path.display(), e))?;
    parse_config(&src, root)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_full_config() {
        let root = Path::new("/proj");
        let cfg = parse_config(
            r#"
[package]
name = "demo"
version = "1.2.3"

[build]
sources = ["src"]
stdlib = "vendor/stdlib"
output = "out"
profile = "release"

[targets]
python = true
native = true

[dependencies]
mathx = { git = "https://github.com/a/b", tag = "v9" }
localx = "libs/localx"
"#,
            root,
        )
        .unwrap();
        assert_eq!(cfg.name, "demo");
        assert_eq!(cfg.version, "1.2.3");
        assert_eq!(cfg.profile, Profile::Release);
        assert_eq!(cfg.targets, vec![Target::Python, Target::Native]);
        assert_eq!(cfg.deps.len(), 2);
        assert_eq!(cfg.entry().0, "demo::main");
        assert_eq!(
            cfg.target_dir(Target::Native),
            root.join("out/native")
                .join(std::env::consts::OS)
                .join(std::env::consts::ARCH)
        );
    }

    #[test]
    fn defaults_when_minimal() {
        let cfg = parse_config(
            "[package]\nname = \"x\"\n",
            Path::new("/p"),
        )
        .unwrap();
        assert_eq!(cfg.sources, vec!["src"]);
        assert_eq!(cfg.targets, vec![Target::Python]);
        assert_eq!(cfg.profile, Profile::Debug);
    }
}
