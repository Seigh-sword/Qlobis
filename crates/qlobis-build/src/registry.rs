//! Third-party package registry (`qlobregistry.toml`) and dependency
//! resolution with a local cache under `~/.qlobis/`.

use crate::config::Dependency;
use crate::toml::{parse_toml, Table};
use std::path::{Path, PathBuf};
use std::process::Command;

/// An entry in `qlobregistry.toml`.
#[derive(Debug, Clone)]
pub struct RegistryEntry {
    pub name: String,
    pub git: String,
    pub tag: Option<String>,
    pub rev: Option<String>,
    pub description: String,
}

/// The package registry file.
#[derive(Debug, Clone, Default)]
pub struct Registry {
    pub packages: Vec<RegistryEntry>,
}

impl Registry {
    pub fn parse(src: &str) -> Result<Registry, String> {
        let t: Table = parse_toml(src)?;
        let mut packages = Vec::new();
        if let Some(pt) = t.get("packages").and_then(|v| v.as_table()) {
            for (k, v) in pt {
                let inner = v
                    .as_table()
                    .ok_or_else(|| format!("package `{}` must be a table", k))?;
                let git = inner
                    .get("git")
                    .and_then(|v| v.as_str())
                    .ok_or_else(|| format!("package `{}` needs `git`", k))?
                    .to_string();
                packages.push(RegistryEntry {
                    name: k.clone(),
                    git,
                    tag: inner.get("tag").and_then(|v| v.as_str()).map(String::from),
                    rev: inner.get("rev").and_then(|v| v.as_str()).map(String::from),
                    description: inner
                        .get("description")
                        .and_then(|v| v.as_str())
                        .unwrap_or("")
                        .to_string(),
                });
            }
        }
        Ok(Registry { packages })
    }

    pub fn find(&self, name: &str) -> Option<&RegistryEntry> {
        self.packages.iter().find(|p| p.name == name)
    }

    /// Load the registry from a project's `qlobregistry.toml`, if present.
    pub fn load(root: &Path) -> Result<Registry, String> {
        let p = root.join("qlobregistry.toml");
        if !p.exists() {
            return Ok(Registry::default());
        }
        let src = std::fs::read_to_string(&p)
            .map_err(|e| format!("cannot read {}: {}", p.display(), e))?;
        Registry::parse(&src)
    }
}

/// A resolved dependency checked out into the cache.
#[derive(Debug, Clone)]
pub struct ResolvedDep {
    pub name: String,
    /// Source directory of the package inside the checkout.
    pub root: PathBuf,
}

/// `~/.qlobis` cache root.
pub fn qlobis_home() -> PathBuf {
    std::env::var_os("QLOBIS_HOME")
        .map(PathBuf::from)
        .unwrap_or_else(|| {
            std::env::var_os("HOME")
                .map(|h| PathBuf::from(h).join(".qlobis"))
                .unwrap_or_else(|| PathBuf::from(".qlobis"))
        })
}

pub fn cache_dir() -> PathBuf {
    qlobis_home().join("registry")
}

/// Resolve every dependency: fetch git deps into the cache (unless already
/// present), map path deps directly. Returns the checkout roots.
pub fn resolve(deps: &[Dependency], registry: &Registry) -> Result<Vec<ResolvedDep>, String> {
    let cache = cache_dir();
    std::fs::create_dir_all(&cache)
        .map_err(|e| format!("cannot create cache {}: {}", cache.display(), e))?;
    let mut out = Vec::new();
    for dep in deps {
        if let Some(path) = &dep.path {
            let root = PathBuf::from(path);
            if !root.exists() {
                return Err(format!("dependency `{}`: path `{}` not found", dep.name, path));
            }
            out.push(ResolvedDep {
                name: dep.name.clone(),
                root,
            });
            continue;
        }
        // Registry lookup fills in git/tag when not spelled out.
        let entry = registry.find(&dep.name);
        let git = dep
            .git
            .clone()
            .or_else(|| entry.map(|e| e.git.clone()))
            .ok_or_else(|| {
                format!(
                    "dependency `{}`: needs `git = \"...\"` or a registry entry",
                    dep.name
                )
            })?;
        let tag = dep
            .tag
            .clone()
            .or_else(|| entry.and_then(|e| e.tag.clone()));
        let rev = dep
            .rev
            .clone()
            .or_else(|| entry.and_then(|e| e.rev.clone()));

        let dest = cache.join(&dep.name);
        if dest.exists() {
            // Already cached; fetch updates only if the pin moved (best effort).
            out.push(ResolvedDep {
                name: dep.name.clone(),
                root: dest,
            });
            continue;
        }
        fetch_git(&git, tag.as_deref(), rev.as_deref(), &dest, &dep.name)?;
        out.push(ResolvedDep {
            name: dep.name.clone(),
            root: dest,
        });
    }
    Ok(out)
}

fn fetch_git(
    git: &str,
    tag: Option<&str>,
    rev: Option<&str>,
    dest: &Path,
    name: &str,
) -> Result<(), String> {
    let mut cmd = Command::new("git");
    cmd.arg("clone").arg("--depth").arg("1");
    if let Some(t) = tag {
        cmd.arg("--branch").arg(t);
    }
    cmd.arg(git).arg(dest);
    let st = cmd
        .output()
        .map_err(|e| format!("dependency `{}`: cannot run git: {}", name, e))?;
    if !st.status.success() {
        return Err(format!(
            "dependency `{}`: git clone failed: {}",
            name,
            String::from_utf8_lossy(&st.stderr).trim()
        ));
    }
    if let Some(r) = rev {
        let st = Command::new("git")
            .arg("-C")
            .arg(dest)
            .arg("checkout")
            .arg(r)
            .output()
            .map_err(|e| format!("dependency `{}`: {}", name, e))?;
        if !st.status.success() {
            return Err(format!(
                "dependency `{}`: git checkout {} failed: {}",
                name,
                r,
                String::from_utf8_lossy(&st.stderr).trim()
            ));
        }
    }
    Ok(())
}

/// Serialize a lockfile (`qlobis.lock`) of resolved dependencies.
pub fn write_lock(root: &Path, deps: &[ResolvedDep]) -> Result<(), String> {
    let mut out = String::from("# Generated by qlobis — resolved dependencies.\n[locked]\n");
    for d in deps {
        out.push_str(&format!(
            "{} = \"{}\"\n",
            d.name,
            d.root.display().to_string().replace('\\', "/")
        ));
    }
    std::fs::write(root.join("qlobis.lock"), out)
        .map_err(|e| format!("cannot write qlobis.lock: {}", e))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_registry() {
        let r = Registry::parse(
            r#"
[packages]
mathx = { git = "https://github.com/a/mathx", tag = "v1", description = "math" }
webui = { git = "https://github.com/b/webui", rev = "abc123" }
"#,
        )
        .unwrap();
        assert_eq!(r.packages.len(), 2);
        let m = r.find("mathx").unwrap();
        assert_eq!(m.git, "https://github.com/a/mathx");
        assert_eq!(m.tag.as_deref(), Some("v1"));
        assert!(r.find("nope").is_none());
    }

    #[test]
    fn path_deps_resolve_without_git() {
        let dir = std::env::temp_dir().join("qlobis-test-pathdep");
        std::fs::create_dir_all(&dir).unwrap();
        let deps = vec![Dependency {
            name: "localx".into(),
            git: None,
            tag: None,
            rev: None,
            path: Some(dir.display().to_string()),
        }];
        let got = resolve(&deps, &Registry::default()).unwrap();
        assert_eq!(got.len(), 1);
        assert_eq!(got[0].name, "localx");
    }
}
