//! Split-licensing guard (D18).
//!
//! Only `jidr-ui` and `jidr-gui` are GPLv3. Every other workspace crate is
//! Apache-2.0 and must never link Slint (GPLv3) or the GPL crates, through
//! normal or build dependencies. Dev-dependencies don't ship, so they're ignored.

use std::collections::{BTreeSet, HashMap, VecDeque};

use anyhow::{Context, Result, bail};
use serde::Deserialize;

/// Workspace crates licensed GPL-3.0-only.
pub const GPL_CRATES: &[&str] = &["jidr-ui", "jidr-gui"];
const GPL_LICENSE: &str = "GPL-3.0-only";
const APACHE_LICENSE: &str = "Apache-2.0";

/// The subset of `cargo metadata --format-version 1` we need.
#[derive(Debug, Deserialize)]
pub struct Metadata {
    pub packages: Vec<Package>,
    pub workspace_members: Vec<String>,
    pub resolve: Option<Resolve>,
}

#[derive(Debug, Deserialize)]
pub struct Package {
    pub id: String,
    pub name: String,
    pub license: Option<String>,
}

#[derive(Debug, Deserialize)]
pub struct Resolve {
    pub nodes: Vec<Node>,
}

#[derive(Debug, Deserialize)]
pub struct Node {
    pub id: String,
    #[serde(default)]
    pub deps: Vec<NodeDep>,
}

#[derive(Debug, Deserialize)]
pub struct NodeDep {
    pub pkg: String,
    #[serde(default)]
    pub dep_kinds: Vec<DepKind>,
}

#[derive(Debug, Deserialize)]
pub struct DepKind {
    /// `null` = normal, `"build"`, or `"dev"`.
    pub kind: Option<String>,
}

impl NodeDep {
    fn ships(&self) -> bool {
        // Old cargo versions omit dep_kinds; treat that as a normal dependency.
        self.dep_kinds.is_empty()
            || self
                .dep_kinds
                .iter()
                .any(|k| k.kind.as_deref() != Some("dev"))
    }
}

fn is_gpl_package(name: &str) -> bool {
    name == "slint" || name.starts_with("i-slint-") || GPL_CRATES.contains(&name)
}

/// Returns every violation found; an empty list means the check passed.
pub fn violations(meta: &Metadata) -> Result<Vec<String>> {
    let resolve = meta
        .resolve
        .as_ref()
        .context("cargo metadata has no resolve graph")?;
    let names: HashMap<&str, &Package> = meta.packages.iter().map(|p| (p.id.as_str(), p)).collect();
    let edges: HashMap<&str, Vec<&str>> = resolve
        .nodes
        .iter()
        .map(|n| {
            let deps = n
                .deps
                .iter()
                .filter(|d| d.ships())
                .map(|d| d.pkg.as_str())
                .collect();
            (n.id.as_str(), deps)
        })
        .collect();

    let mut out = Vec::new();
    for member in &meta.workspace_members {
        let pkg = names
            .get(member.as_str())
            .with_context(|| format!("workspace member {member} missing from packages"))?;
        let gpl = GPL_CRATES.contains(&pkg.name.as_str());
        let want = if gpl { GPL_LICENSE } else { APACHE_LICENSE };
        if pkg.license.as_deref() != Some(want) {
            out.push(format!(
                "{}: license is {:?}, expected {want:?} (D18)",
                pkg.name, pkg.license
            ));
        }
        if gpl {
            continue;
        }
        // BFS over shipped dependencies, remembering the path for the report.
        let mut seen = BTreeSet::new();
        let mut queue = VecDeque::from([(member.as_str(), vec![pkg.name.clone()])]);
        while let Some((id, path)) = queue.pop_front() {
            for dep in edges.get(id).into_iter().flatten() {
                if !seen.insert(*dep) {
                    continue;
                }
                let Some(dep_pkg) = names.get(dep) else {
                    continue;
                };
                let mut next = path.clone();
                next.push(dep_pkg.name.clone());
                if is_gpl_package(&dep_pkg.name) {
                    out.push(format!(
                        "{}: Apache-2.0 crate depends on GPL crate `{}` via {}",
                        pkg.name,
                        dep_pkg.name,
                        next.join(" -> ")
                    ));
                } else {
                    queue.push_back((dep, next));
                }
            }
        }
    }
    Ok(out)
}

/// Runs `cargo metadata` on the workspace and fails on any violation.
pub fn run() -> Result<()> {
    let output = std::process::Command::new(crate::cargo())
        .args(["metadata", "--format-version", "1", "--locked"])
        .current_dir(crate::workspace_root())
        .output()
        .context("running cargo metadata")?;
    if !output.status.success() {
        bail!(
            "cargo metadata failed: {}",
            String::from_utf8_lossy(&output.stderr)
        );
    }
    let meta: Metadata =
        serde_json::from_slice(&output.stdout).context("parsing cargo metadata output")?;
    let found = violations(&meta)?;
    if found.is_empty() {
        println!(
            "license-check: ok ({} workspace crates)",
            meta.workspace_members.len()
        );
        return Ok(());
    }
    for v in &found {
        println!("license-check: {v}");
    }
    bail!("license-check failed with {} violation(s)", found.len())
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Builds metadata for a tiny workspace. `deps` entries are `(from, to, kind)`.
    fn meta(
        members: &[(&str, &str)],
        externals: &[&str],
        deps: &[(&str, &str, Option<&str>)],
    ) -> Metadata {
        let mut packages: Vec<Package> = members
            .iter()
            .map(|(n, l)| Package {
                id: n.to_string(),
                name: n.to_string(),
                license: Some(l.to_string()),
            })
            .collect();
        packages.extend(externals.iter().map(|n| Package {
            id: n.to_string(),
            name: n.to_string(),
            license: Some("MIT".into()),
        }));
        let nodes = packages
            .iter()
            .map(|p| Node {
                id: p.id.clone(),
                deps: deps
                    .iter()
                    .filter(|(from, _, _)| *from == p.id)
                    .map(|(_, to, kind)| NodeDep {
                        pkg: to.to_string(),
                        dep_kinds: vec![DepKind {
                            kind: kind.map(str::to_string),
                        }],
                    })
                    .collect(),
            })
            .collect();
        Metadata {
            workspace_members: members.iter().map(|(n, _)| n.to_string()).collect(),
            packages,
            resolve: Some(Resolve { nodes }),
        }
    }

    const BASE: &[(&str, &str)] = &[
        ("jidr", "Apache-2.0"),
        ("jidr-client", "Apache-2.0"),
        ("jidr-ui", "GPL-3.0-only"),
        ("jidr-gui", "GPL-3.0-only"),
    ];

    #[test]
    fn clean_workspace_passes() {
        let m = meta(
            BASE,
            &["slint", "serde"],
            &[
                ("jidr", "jidr-client", None),
                ("jidr-client", "serde", None),
                ("jidr-gui", "jidr-ui", None),
                ("jidr-ui", "slint", None),
                ("jidr-ui", "jidr-client", None),
            ],
        );
        assert_eq!(violations(&m).unwrap(), Vec::<String>::new());
    }

    #[test]
    fn slint_added_to_cli_fails() {
        let m = meta(BASE, &["slint"], &[("jidr", "slint", None)]);
        let v = violations(&m).unwrap();
        assert_eq!(v.len(), 1, "{v:?}");
        assert!(v[0].contains("jidr -> slint"), "{v:?}");
    }

    #[test]
    fn transitive_slint_internals_fail() {
        let m = meta(
            BASE,
            &["helper", "i-slint-core"],
            &[
                ("jidr-client", "helper", None),
                ("helper", "i-slint-core", Some("build")),
            ],
        );
        let v = violations(&m).unwrap();
        assert!(
            v.iter()
                .any(|s| s.contains("jidr-client -> helper -> i-slint-core")),
            "{v:?}"
        );
    }

    #[test]
    fn apache_crate_depending_on_gui_crate_fails() {
        let m = meta(BASE, &[], &[("jidr", "jidr-ui", None)]);
        let v = violations(&m).unwrap();
        assert!(v.iter().any(|s| s.contains("GPL crate `jidr-ui`")), "{v:?}");
    }

    #[test]
    fn dev_dependency_on_slint_is_allowed() {
        let m = meta(BASE, &["slint"], &[("jidr", "slint", Some("dev"))]);
        assert!(violations(&m).unwrap().is_empty());
    }

    #[test]
    fn wrong_license_field_fails() {
        let m = meta(&[("jidr", "MIT"), ("jidr-ui", "Apache-2.0")], &[], &[]);
        let v = violations(&m).unwrap();
        assert_eq!(v.len(), 2, "{v:?}");
    }

    /// The real workspace must pass.
    #[test]
    fn current_workspace_passes() {
        run().unwrap();
    }
}
