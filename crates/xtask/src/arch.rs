//! The layer rule: who may depend on whom, checked against `cargo metadata`.

use std::path::{Path, PathBuf};

use cargo_metadata::{DependencyKind, MetadataCommand};

use crate::error::XtaskError;

const KERNEL_ALLOWED: &[&str] = &["thiserror", "uuid", "time", "rust_decimal"];
const MODULE_ALLOWED: &[&str] = &[
    "barras-kernel",
    "thiserror",
    "uuid",
    "time",
    "rust_decimal",
    "async-trait",
];
const DEV_EXTRA: &[&str] = &["tokio", "mockall", "insta"];
const KERNEL: &str = "barras-kernel";
const TESTKIT: &str = "barras-testkit";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Layer {
    Kernel,
    Module,
    Adapter,
    App,
    Testkit,
    Tool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DepKind {
    Normal,
    Dev,
    Build,
}

#[derive(Debug, Clone)]
pub struct Dep {
    /// The real package name, never the local rename.
    pub name: String,
    pub kind: DepKind,
    pub is_workspace: bool,
}

#[derive(Debug, Clone)]
pub struct CrateInfo {
    pub name: String,
    /// Relative to the workspace root.
    pub dir: PathBuf,
    pub deps: Vec<Dep>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Violation {
    pub krate: String,
    pub dependency: String,
    pub rule: &'static str,
}

pub fn layer_of(dir: &Path) -> Option<Layer> {
    let parts: Vec<&str> = dir.iter().filter_map(|p| p.to_str()).collect();
    match parts.as_slice() {
        ["crates", "kernel"] => Some(Layer::Kernel),
        ["crates", "modules", _] => Some(Layer::Module),
        ["crates", "adapters", _] => Some(Layer::Adapter),
        ["apps", _] => Some(Layer::App),
        ["crates", "testkit"] => Some(Layer::Testkit),
        ["crates", "xtask"] => Some(Layer::Tool),
        _ => None,
    }
}

/// Violations come in input order of the crates, then in order of their dependencies.
pub fn check_arch(crates: &[CrateInfo]) -> Vec<Violation> {
    let mut violations = Vec::new();
    for krate in crates {
        let Some(layer) = layer_of(&krate.dir) else {
            violations.push(violation(krate, "", "unknown-location"));
            continue;
        };
        for dep in &krate.deps {
            let name = dep.name.as_str();
            let not_dev = dep.kind != DepKind::Dev;
            match layer {
                Layer::Kernel if not_dev && !allowed(&[KERNEL_ALLOWED], dep) => {
                    violations.push(violation(krate, name, "kernel-allowlist"));
                }
                Layer::Kernel if !not_dev && !allowed(&[KERNEL_ALLOWED, DEV_EXTRA], dep) => {
                    violations.push(violation(krate, name, "kernel-dev-allowlist"));
                }
                Layer::Module if not_dev && !allowed(&[MODULE_ALLOWED], dep) => {
                    violations.push(violation(krate, name, "module-allowlist"));
                }
                Layer::Module if !not_dev && !allowed(&[MODULE_ALLOWED, DEV_EXTRA], dep) => {
                    violations.push(violation(krate, name, "module-dev-allowlist"));
                }
                Layer::Adapter if depends_on_adapter(crates, name) => {
                    violations.push(violation(krate, name, "adapter-isolation"));
                }
                Layer::Tool if dep.is_workspace => {
                    violations.push(violation(krate, name, "tool-isolation"));
                }
                _ => {}
            }
            if name == TESTKIT && dep.kind != DepKind::Dev {
                violations.push(violation(krate, name, "testkit-dev-only"));
            }
        }
    }
    violations
}

/// An allowlisted name counts only from its expected source: `barras-kernel` is a
/// workspace path, everything else comes from the registry. Otherwise a `path`, `git`
/// or `[patch]` entry could pass a foreign crate off under an allowed name.
fn allowed(lists: &[&[&str]], dep: &Dep) -> bool {
    lists.iter().any(|list| list.contains(&dep.name.as_str()))
        && dep.is_workspace == (dep.name == KERNEL)
}

fn violation(krate: &CrateInfo, dependency: &str, rule: &'static str) -> Violation {
    Violation {
        krate: krate.name.clone(),
        dependency: dependency.to_owned(),
        rule,
    }
}

fn depends_on_adapter(crates: &[CrateInfo], dependency: &str) -> bool {
    crates
        .iter()
        .any(|c| c.name == dependency && layer_of(&c.dir) == Some(Layer::Adapter))
}

pub fn load_workspace(root: &Path) -> Result<Vec<CrateInfo>, XtaskError> {
    let metadata = MetadataCommand::new()
        .current_dir(root)
        .manifest_path(root.join("Cargo.toml"))
        .no_deps()
        .exec()
        .map_err(|e| XtaskError::Metadata(e.to_string()))?;
    // Taken from the metadata, not from `root`: cargo may report a canonicalised path.
    let workspace_root = metadata.workspace_root.as_std_path();
    let mut crates = Vec::new();
    for package in &metadata.packages {
        let manifest_dir = package
            .manifest_path
            .as_std_path()
            .parent()
            .ok_or_else(|| {
                XtaskError::Metadata(format!("manifest of {} has no directory", package.name))
            })?;
        let dir = manifest_dir
            .strip_prefix(workspace_root)
            .map_err(|e| XtaskError::Metadata(format!("{}: {e}", package.name)))?
            .to_path_buf();
        let deps = package
            .dependencies
            .iter()
            .map(|d| Dep {
                name: d.name.clone(),
                kind: match d.kind {
                    DependencyKind::Development => DepKind::Dev,
                    DependencyKind::Build => DepKind::Build,
                    _ => DepKind::Normal,
                },
                is_workspace: d.path.is_some(),
            })
            .collect();
        crates.push(CrateInfo {
            name: package.name.to_string(),
            dir,
            deps,
        });
    }
    Ok(crates)
}

pub fn workspace_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../..")
}

#[cfg(test)]
mod tests {
    use super::DepKind::{Build, Dev, Normal};
    use super::*;

    fn krate(name: &str, dir: &str, deps: &[(&str, DepKind, bool)]) -> CrateInfo {
        CrateInfo {
            name: name.to_owned(),
            dir: PathBuf::from(dir),
            deps: deps
                .iter()
                .map(|(name, kind, is_workspace)| Dep {
                    name: (*name).to_owned(),
                    kind: *kind,
                    is_workspace: *is_workspace,
                })
                .collect(),
        }
    }

    fn rules(violations: &[Violation]) -> Vec<(&str, &str, &str)> {
        violations
            .iter()
            .map(|v| (v.krate.as_str(), v.dependency.as_str(), v.rule))
            .collect()
    }

    #[test]
    fn the_empty_skeleton_is_clean() {
        let crates = [
            krate(
                "barras-kernel",
                "crates/kernel",
                &[("thiserror", Normal, false)],
            ),
            krate(
                "barras-identity",
                "crates/modules/identity",
                &[
                    ("barras-kernel", Normal, true),
                    ("async-trait", Normal, false),
                    ("tokio", Dev, false),
                ],
            ),
            krate(
                "barras-adapter-postgres",
                "crates/adapters/postgres",
                &[("barras-identity", Normal, true), ("sqlx", Normal, false)],
            ),
            krate(
                "barras-api",
                "apps/api",
                &[
                    ("barras-adapter-postgres", Normal, true),
                    ("barras-testkit", Dev, true),
                ],
            ),
            krate(
                "barras-testkit",
                "crates/testkit",
                &[("sqlx", Normal, false)],
            ),
            krate("xtask", "crates/xtask", &[("regex", Normal, false)]),
        ];
        assert!(check_arch(&crates).is_empty());
    }

    #[test]
    fn a_module_cannot_reach_a_framework_or_another_module() {
        let crates = [krate(
            "barras-identity",
            "crates/modules/identity",
            &[
                ("sqlx", Normal, false),
                ("axum", Normal, false),
                ("barras-capital", Normal, true),
                ("sqlx", Dev, false),
            ],
        )];
        assert_eq!(
            rules(&check_arch(&crates)),
            [
                ("barras-identity", "sqlx", "module-allowlist"),
                ("barras-identity", "axum", "module-allowlist"),
                ("barras-identity", "barras-capital", "module-allowlist"),
                ("barras-identity", "sqlx", "module-dev-allowlist"),
            ]
        );
    }

    #[test]
    fn the_other_rules() {
        let crates = [
            krate(
                "barras-kernel",
                "crates/kernel",
                &[("serde", Normal, false)],
            ),
            krate("barras-adapter-postgres", "crates/adapters/postgres", &[]),
            krate(
                "barras-adapter-http",
                "crates/adapters/http",
                &[("barras-adapter-postgres", Normal, true)],
            ),
            krate(
                "barras-api",
                "apps/api",
                &[("barras-testkit", Normal, true)],
            ),
            krate("xtask", "crates/xtask", &[("barras-kernel", Normal, true)]),
            krate("barras-misc", "crates/misc", &[]),
        ];
        assert_eq!(
            rules(&check_arch(&crates)),
            [
                ("barras-kernel", "serde", "kernel-allowlist"),
                (
                    "barras-adapter-http",
                    "barras-adapter-postgres",
                    "adapter-isolation"
                ),
                ("barras-api", "barras-testkit", "testkit-dev-only"),
                ("xtask", "barras-kernel", "tool-isolation"),
                ("barras-misc", "", "unknown-location"),
            ]
        );
    }

    #[test]
    fn an_allowed_name_from_the_wrong_source_is_a_violation() {
        let crates = [
            krate("barras-kernel", "crates/kernel", &[("uuid", Normal, true)]),
            krate(
                "barras-identity",
                "crates/modules/identity",
                &[
                    ("barras-kernel", Normal, false),
                    ("time", Normal, true),
                    ("tokio", Dev, true),
                ],
            ),
        ];
        assert_eq!(
            rules(&check_arch(&crates)),
            [
                ("barras-kernel", "uuid", "kernel-allowlist"),
                ("barras-identity", "barras-kernel", "module-allowlist"),
                ("barras-identity", "time", "module-allowlist"),
                ("barras-identity", "tokio", "module-dev-allowlist"),
            ]
        );
    }

    #[test]
    fn build_and_dev_dependencies_follow_the_rules_too() {
        let crates = [
            krate(
                "barras-kernel",
                "crates/kernel",
                &[
                    ("cc", Build, false),
                    ("sqlx", Dev, false),
                    ("insta", Dev, false),
                ],
            ),
            krate(
                "barras-identity",
                "crates/modules/identity",
                &[("cc", Build, false)],
            ),
            krate("barras-api", "apps/api", &[("barras-testkit", Build, true)]),
        ];
        assert_eq!(
            rules(&check_arch(&crates)),
            [
                ("barras-kernel", "cc", "kernel-allowlist"),
                ("barras-kernel", "sqlx", "kernel-dev-allowlist"),
                ("barras-identity", "cc", "module-allowlist"),
                ("barras-api", "barras-testkit", "testkit-dev-only"),
            ]
        );
    }
}
