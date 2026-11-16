//! The layer rule: who may depend on whom, checked against `cargo metadata`.

use std::fs;
use std::path::{Path, PathBuf};

use cargo_metadata::{DependencyKind, MetadataCommand};

use crate::error::XtaskError;
use crate::structure::entries;

const KERNEL_ALLOWED: &[&str] = &["thiserror", "uuid", "time", "rust_decimal", HACK];
const MODULE_ALLOWED: &[&str] = &[
    KERNEL,
    "thiserror",
    "uuid",
    "time",
    "rust_decimal",
    "async-trait",
    HACK,
];
const DEV_EXTRA: &[&str] = &["tokio", "mockall", "insta"];
const KERNEL: &str = "barras-kernel";
const KERNEL_DIR: &str = "crates/kernel";
const BOOTSTRAP: &str = "barras-bootstrap";
const TESTKIT: &str = "barras-testkit";
/// The crate cargo-hakari generates. Every member but `xtask` depends on it.
const HACK: &str = "workspace-hack";
const HACK_DIR: &str = "crates/workspace-hack";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Layer {
    Kernel,
    Module,
    Adapter,
    App,
    Bootstrap,
    Testkit,
    Tool,
    WorkspaceHack,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DepKind {
    Normal,
    Dev,
    Build,
}

/// Where a dependency comes from.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Origin {
    /// A path dependency.
    Path,
    CratesIo,
    /// A git repository or a registry other than crates.io.
    Elsewhere,
}

impl Origin {
    /// Classifies the `source` that `cargo metadata` reports, which is absent for a path dependency.
    pub fn of(source: Option<&cargo_metadata::Source>) -> Self {
        match source {
            None => Self::Path,
            Some(source) if source.is_crates_io() => Self::CratesIo,
            Some(_) => Self::Elsewhere,
        }
    }
}

#[derive(Debug, Clone)]
pub struct Dep {
    /// The real package name, never the local rename.
    pub name: String,
    pub kind: DepKind,
    pub origin: Origin,
    /// The folder of a path dependency: relative to the workspace root when it is inside
    /// it, as cargo reports it otherwise.
    pub path: Option<PathBuf>,
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
        ["crates", "bootstrap"] => Some(Layer::Bootstrap),
        ["crates", "testkit"] => Some(Layer::Testkit),
        ["crates", "xtask"] => Some(Layer::Tool),
        ["crates", "workspace-hack"] => Some(Layer::WorkspaceHack),
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
                Layer::Bootstrap if is_member(crates, name) && !is_the_hack(dep) => {
                    violations.push(violation(krate, name, "bootstrap-isolation"));
                }
                Layer::Tool if is_member(crates, name) => {
                    violations.push(violation(krate, name, "tool-isolation"));
                }
                _ => {}
            }
            if name == TESTKIT && dep.kind != DepKind::Dev {
                violations.push(violation(krate, name, "testkit-dev-only"));
            }
            if name == BOOTSTRAP && layer != Layer::App {
                violations.push(violation(krate, name, "bootstrap-apps-only"));
            }
        }
    }
    violations
}

/// An allowlisted name counts only from its expected source: `barras-kernel` and
/// `workspace-hack` are the path dependency at that member's own folder, everything else
/// comes from crates.io. Otherwise a `path` or `git` entry, or another registry, could
/// pass a foreign crate off under an allowed name. `[patch]` and `[replace]` are stopped
/// by `check_patches`.
fn allowed(lists: &[&[&str]], dep: &Dep) -> bool {
    let from_expected_source = match member_folder(&dep.name) {
        Some(folder) => is_path_to(dep, folder),
        None => dep.origin == Origin::CratesIo,
    };
    lists.iter().any(|list| list.contains(&dep.name.as_str())) && from_expected_source
}

/// The folder of each workspace crate that an allowlist names.
fn member_folder(name: &str) -> Option<&'static str> {
    match name {
        KERNEL => Some(KERNEL_DIR),
        HACK => Some(HACK_DIR),
        _ => None,
    }
}

/// A path dependency taken from `folder`, which is relative to the workspace root. A
/// copy of the crate kept in any other folder has the same name and is other code.
fn is_path_to(dep: &Dep, folder: &str) -> bool {
    dep.path.as_deref() == Some(Path::new(folder))
}

fn is_the_hack(dep: &Dep) -> bool {
    dep.name == HACK && is_path_to(dep, HACK_DIR)
}

/// A workspace crate is known by its package name, so a `git` copy of one is still caught.
fn is_member(crates: &[CrateInfo], dependency: &str) -> bool {
    crates.iter().any(|c| c.name == dependency)
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

/// Every rule of `check-arch` over the workspace at `root`.
pub fn check_workspace(root: &Path) -> Result<Vec<Violation>, XtaskError> {
    let crates = load_workspace(root)?;
    let mut violations = check_arch(&crates);
    violations.extend(check_members(root, &crates)?);
    violations.extend(check_patches(root)?);
    Ok(violations)
}

/// A crate folder that `members` does not list is never built and never checked.
fn check_members(root: &Path, crates: &[CrateInfo]) -> Result<Vec<Violation>, XtaskError> {
    let mut folders = Vec::new();
    for parent in ["apps", "crates/modules", "crates/adapters"] {
        let dir = root.join(parent);
        if !dir.is_dir() {
            continue;
        }
        for (name, is_dir) in entries(&dir)? {
            if is_dir {
                folders.push(format!("{parent}/{name}"));
            }
        }
    }
    folders.extend(
        [
            "crates/bootstrap",
            "crates/kernel",
            "crates/testkit",
            "crates/workspace-hack",
            "crates/xtask",
        ]
        .map(String::from),
    );
    Ok(folders
        .into_iter()
        .filter(|folder| root.join(folder).join("Cargo.toml").is_file())
        .filter(|folder| !crates.iter().any(|c| c.dir.as_path() == Path::new(folder)))
        .map(|folder| Violation {
            krate: folder,
            dependency: String::new(),
            rule: "not-a-member",
        })
        .collect())
}

/// `[patch]` and `[replace]` swap a crate for other code without touching any dependency
/// line, so the allowlists cannot see them. Needing one is a decision for an ADR.
fn check_patches(root: &Path) -> Result<Vec<Violation>, XtaskError> {
    let mut violations = Vec::new();
    for (file, tables) in [
        ("Cargo.toml", &["patch", "replace"][..]),
        (".cargo/config.toml", &["patch"][..]),
    ] {
        let path = root.join(file);
        if !path.is_file() {
            continue;
        }
        let text = fs::read_to_string(&path).map_err(XtaskError::io(&path))?;
        for found in top_level_tables(&text, tables) {
            violations.push(Violation {
                krate: file.to_owned(),
                dependency: found,
                rule: "no-patch",
            });
        }
    }
    Ok(violations)
}

/// The tables among `names` that a TOML text defines at its top level, as written: a
/// `[name]` or `[name.x]` header, or a `name` or `name.x` key before the first header.
/// A line scan rather than a TOML parser, so the check needs no new dependency.
fn top_level_tables(toml: &str, names: &[&str]) -> Vec<String> {
    let mut found = Vec::new();
    let mut before_first_header = true;
    for line in toml.lines().map(str::trim) {
        let (key, shown) = if let Some(header) = line.strip_prefix('[') {
            before_first_header = false;
            let end = line.find(']').map_or(line.len(), |i| i + 1);
            (
                header.trim_start_matches('['),
                line.get(..end).unwrap_or(line),
            )
        } else if before_first_header {
            (line, line.split('=').next().unwrap_or(line).trim_end())
        } else {
            continue;
        };
        if names.contains(&first_key(key)) {
            found.push(shown.to_owned());
        }
    }
    found
}

/// The first part of a dotted TOML key, unquoted: `patch` for `patch.crates-io`,
/// `"patch".crates-io` or `patch . crates-io`.
fn first_key(key: &str) -> &str {
    let key = key.trim_start();
    for quote in ['"', '\''] {
        if let Some(rest) = key.strip_prefix(quote) {
            return rest.split(quote).next().unwrap_or(rest);
        }
    }
    key.split(['.', ']', '=', ' ', '\t']).next().unwrap_or(key)
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
                origin: Origin::of(d.source.as_ref()),
                // Cargo spells it like the root and normalises it, so a plain prefix strip
                // is enough. A folder outside the root keeps its whole path and so equals
                // no member's folder.
                path: d.path.as_ref().map(|path| {
                    let path = path.as_std_path();
                    path.strip_prefix(workspace_root)
                        .unwrap_or(path)
                        .to_path_buf()
                }),
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

    const CRATES_IO: &str = "registry+https://github.com/rust-lang/crates.io-index";

    /// `true` marks a path dependency at its usual folder, `false` one from crates.io.
    fn krate(name: &str, dir: &str, deps: &[(&str, DepKind, bool)]) -> CrateInfo {
        let sources: Vec<(&str, DepKind, Option<&str>)> = deps
            .iter()
            .map(|(name, kind, is_path)| (*name, *kind, (!is_path).then_some(CRATES_IO)))
            .collect();
        krate_from(name, dir, &sources)
    }

    /// Each dependency carries the `source` that `cargo metadata` would report: `None` for
    /// a path, which is then the crate's usual folder.
    fn krate_from(name: &str, dir: &str, deps: &[(&str, DepKind, Option<&str>)]) -> CrateInfo {
        CrateInfo {
            name: name.to_owned(),
            dir: PathBuf::from(dir),
            deps: deps
                .iter()
                .map(|(name, kind, source)| Dep {
                    name: (*name).to_owned(),
                    kind: *kind,
                    origin: Origin::of(
                        source
                            .map(|repr| cargo_metadata::Source {
                                repr: repr.to_owned(),
                            })
                            .as_ref(),
                    ),
                    path: source.is_none().then(|| usual_folder(name)),
                })
                .collect(),
        }
    }

    /// Path dependencies only, each from the folder given.
    fn krate_at(name: &str, dir: &str, deps: &[(&str, DepKind, &str)]) -> CrateInfo {
        CrateInfo {
            name: name.to_owned(),
            dir: PathBuf::from(dir),
            deps: deps
                .iter()
                .map(|(name, kind, path)| Dep {
                    name: (*name).to_owned(),
                    kind: *kind,
                    origin: Origin::Path,
                    path: Some(PathBuf::from(path)),
                })
                .collect(),
        }
    }

    /// Where this workspace keeps the two crates an allowlist takes by path; any other
    /// name gets a folder no rule looks at.
    fn usual_folder(name: &str) -> PathBuf {
        match name {
            "barras-kernel" => PathBuf::from("crates/kernel"),
            "workspace-hack" => PathBuf::from("crates/workspace-hack"),
            other => Path::new("elsewhere").join(other),
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

    const GIT: &str = "git+https://github.com/example/fork?branch=main#0123456789abcdef";
    const OTHER_REGISTRY: &str = "sparse+https://registry.example.com/index/";

    #[test]
    fn an_allowed_name_counts_only_from_crates_io() {
        let crates = [
            krate_from(
                "barras-kernel",
                "crates/kernel",
                &[("thiserror", Normal, Some(GIT))],
            ),
            krate_from(
                "barras-identity",
                "crates/modules/identity",
                &[
                    ("uuid", Normal, Some(GIT)),
                    ("time", Normal, Some(OTHER_REGISTRY)),
                ],
            ),
        ];
        assert_eq!(
            rules(&check_arch(&crates)),
            [
                ("barras-kernel", "thiserror", "kernel-allowlist"),
                ("barras-identity", "uuid", "module-allowlist"),
                ("barras-identity", "time", "module-allowlist"),
            ]
        );
    }

    #[test]
    fn a_workspace_crate_is_known_by_its_name_whatever_its_source() {
        let crates = [
            krate_from("barras-kernel", "crates/kernel", &[]),
            krate_from("barras-adapter-postgres", "crates/adapters/postgres", &[]),
            krate_from(
                "barras-adapter-http",
                "crates/adapters/http",
                &[("barras-adapter-postgres", Normal, Some(GIT))],
            ),
            krate_from(
                "xtask",
                "crates/xtask",
                &[("barras-kernel", Normal, Some(GIT))],
            ),
        ];
        assert_eq!(
            rules(&check_arch(&crates)),
            [
                (
                    "barras-adapter-http",
                    "barras-adapter-postgres",
                    "adapter-isolation"
                ),
                ("xtask", "barras-kernel", "tool-isolation"),
            ]
        );
    }

    #[test]
    fn the_kernel_counts_only_from_its_own_folder() {
        let crates = [
            krate("barras-kernel", "crates/kernel", &[]),
            krate_at(
                "barras-identity",
                "crates/modules/identity",
                &[("barras-kernel", Normal, "crates/kernel")],
            ),
            krate_at(
                "barras-goals",
                "crates/modules/goals",
                &[
                    ("barras-kernel", Normal, "vendor/kernel"),
                    ("barras-kernel", Build, "crates/kernel/copy"),
                    ("barras-kernel", Dev, "/outside/crates/kernel"),
                ],
            ),
        ];
        assert_eq!(
            rules(&check_arch(&crates)),
            [
                ("barras-goals", "barras-kernel", "module-allowlist"),
                ("barras-goals", "barras-kernel", "module-allowlist"),
                ("barras-goals", "barras-kernel", "module-dev-allowlist"),
            ]
        );
    }

    #[test]
    fn a_path_dependency_without_its_folder_never_counts() {
        let mut module = krate_at(
            "barras-identity",
            "crates/modules/identity",
            &[("barras-kernel", Normal, "crates/kernel")],
        );
        module.deps[0].path = None;
        assert_eq!(
            rules(&check_arch(&[module])),
            [("barras-identity", "barras-kernel", "module-allowlist")]
        );
    }

    #[test]
    fn the_hack_is_allowed_in_the_kernel_and_the_modules_from_its_own_folder() {
        let crates = [
            krate("workspace-hack", "crates/workspace-hack", &[]),
            krate_at(
                "barras-kernel",
                "crates/kernel",
                &[
                    ("workspace-hack", Normal, "crates/workspace-hack"),
                    ("workspace-hack", Dev, "crates/workspace-hack"),
                ],
            ),
            krate_at(
                "barras-identity",
                "crates/modules/identity",
                &[
                    ("workspace-hack", Normal, "crates/workspace-hack"),
                    ("workspace-hack", Build, "crates/workspace-hack"),
                    ("workspace-hack", Dev, "crates/workspace-hack"),
                ],
            ),
        ];
        assert_eq!(check_arch(&crates), []);
    }

    #[test]
    fn the_hack_counts_only_from_its_own_folder() {
        let crates = [
            krate_from(
                "barras-kernel",
                "crates/kernel",
                &[
                    ("workspace-hack", Normal, Some(CRATES_IO)),
                    ("workspace-hack", Dev, Some(GIT)),
                ],
            ),
            krate_at(
                "barras-identity",
                "crates/modules/identity",
                &[
                    ("workspace-hack", Normal, "vendor/workspace-hack"),
                    ("workspace-hack", Dev, "crates/kernel"),
                ],
            ),
        ];
        assert_eq!(
            rules(&check_arch(&crates)),
            [
                ("barras-kernel", "workspace-hack", "kernel-allowlist"),
                ("barras-kernel", "workspace-hack", "kernel-dev-allowlist"),
                ("barras-identity", "workspace-hack", "module-allowlist"),
                ("barras-identity", "workspace-hack", "module-dev-allowlist"),
            ]
        );
    }

    #[test]
    fn bootstrap_and_the_hack_are_known_folders() {
        assert_eq!(
            layer_of(Path::new("crates/bootstrap")),
            Some(Layer::Bootstrap)
        );
        assert_eq!(
            layer_of(Path::new("crates/workspace-hack")),
            Some(Layer::WorkspaceHack)
        );
        let crates = [
            krate(
                "barras-bootstrap",
                "crates/bootstrap",
                &[("tokio", Normal, false), ("workspace-hack", Normal, true)],
            ),
            krate(
                "workspace-hack",
                "crates/workspace-hack",
                &[("tokio", Normal, false), ("axum", Normal, false)],
            ),
            krate("barras-nested", "crates/bootstrap/nested", &[]),
            krate("barras-hacks", "crates/workspace-hacks", &[]),
        ];
        assert_eq!(
            rules(&check_arch(&crates)),
            [
                ("barras-nested", "", "unknown-location"),
                ("barras-hacks", "", "unknown-location"),
            ]
        );
    }

    #[test]
    fn bootstrap_depends_on_no_workspace_crate_but_the_hack() {
        let crates = [
            krate("barras-kernel", "crates/kernel", &[]),
            krate("barras-adapter-http", "crates/adapters/http", &[]),
            krate("workspace-hack", "crates/workspace-hack", &[]),
            krate_from(
                "barras-bootstrap",
                "crates/bootstrap",
                &[
                    ("tracing", Normal, Some(CRATES_IO)),
                    ("workspace-hack", Normal, None),
                    ("barras-kernel", Normal, None),
                    ("barras-adapter-http", Dev, None),
                    ("barras-kernel", Build, Some(GIT)),
                ],
            ),
        ];
        assert_eq!(
            rules(&check_arch(&crates)),
            [
                ("barras-bootstrap", "barras-kernel", "bootstrap-isolation"),
                (
                    "barras-bootstrap",
                    "barras-adapter-http",
                    "bootstrap-isolation"
                ),
                ("barras-bootstrap", "barras-kernel", "bootstrap-isolation"),
            ]
        );
    }

    #[test]
    fn bootstrap_takes_the_hack_only_from_its_own_folder() {
        let hack = krate("workspace-hack", "crates/workspace-hack", &[]);
        let from_another_folder = krate_at(
            "barras-bootstrap",
            "crates/bootstrap",
            &[("workspace-hack", Normal, "vendor/workspace-hack")],
        );
        let from_git = krate_from(
            "barras-bootstrap",
            "crates/bootstrap",
            &[("workspace-hack", Normal, Some(GIT))],
        );
        for bootstrap in [from_another_folder, from_git] {
            assert_eq!(
                rules(&check_arch(&[hack.clone(), bootstrap])),
                [("barras-bootstrap", "workspace-hack", "bootstrap-isolation")]
            );
        }
    }

    #[test]
    fn only_an_app_may_depend_on_bootstrap() {
        let crates = [
            krate("barras-bootstrap", "crates/bootstrap", &[]),
            krate(
                "barras-api",
                "apps/api",
                &[("barras-bootstrap", Normal, true)],
            ),
            krate(
                "barras-ingest",
                "apps/ingest",
                &[("barras-bootstrap", Dev, true)],
            ),
            krate(
                "barras-adapter-http",
                "crates/adapters/http",
                &[("barras-bootstrap", Normal, true)],
            ),
            krate(
                "barras-testkit",
                "crates/testkit",
                &[("barras-bootstrap", Dev, true)],
            ),
            krate_from(
                "workspace-hack",
                "crates/workspace-hack",
                &[("barras-bootstrap", Build, Some(GIT))],
            ),
        ];
        assert_eq!(
            rules(&check_arch(&crates)),
            [
                (
                    "barras-adapter-http",
                    "barras-bootstrap",
                    "bootstrap-apps-only"
                ),
                ("barras-testkit", "barras-bootstrap", "bootstrap-apps-only"),
                ("workspace-hack", "barras-bootstrap", "bootstrap-apps-only"),
            ]
        );
    }

    #[test]
    fn bootstrap_in_a_crate_with_its_own_rule_breaks_both() {
        let crates = [
            krate("barras-bootstrap", "crates/bootstrap", &[]),
            krate(
                "barras-kernel",
                "crates/kernel",
                &[("barras-bootstrap", Dev, true)],
            ),
            krate(
                "barras-identity",
                "crates/modules/identity",
                &[("barras-bootstrap", Normal, true)],
            ),
            krate(
                "xtask",
                "crates/xtask",
                &[("barras-bootstrap", Normal, true)],
            ),
        ];
        assert_eq!(
            rules(&check_arch(&crates)),
            [
                ("barras-kernel", "barras-bootstrap", "kernel-dev-allowlist"),
                ("barras-kernel", "barras-bootstrap", "bootstrap-apps-only"),
                ("barras-identity", "barras-bootstrap", "module-allowlist"),
                ("barras-identity", "barras-bootstrap", "bootstrap-apps-only"),
                ("xtask", "barras-bootstrap", "tool-isolation"),
                ("xtask", "barras-bootstrap", "bootstrap-apps-only"),
            ]
        );
    }

    #[test]
    fn the_tool_still_may_not_depend_on_the_hack() {
        let crates = [
            krate("workspace-hack", "crates/workspace-hack", &[]),
            krate_at(
                "xtask",
                "crates/xtask",
                &[("workspace-hack", Normal, "crates/workspace-hack")],
            ),
        ];
        assert_eq!(
            rules(&check_arch(&crates)),
            [("xtask", "workspace-hack", "tool-isolation")]
        );
    }

    #[test]
    fn patch_and_replace_tables_are_found_in_each_toml_form() {
        let manifest = "patch.crates-io.uuid = { path = \"x\" }\n\n[package]\nname = \"x\"\n[patches]\n[workspace.patch]\n[ patch . \"https://github.com/example/fork\" ] # a fork\n[replace]\n\"uuid:1.0.0\" = { path = \"y\" }\n";
        assert_eq!(
            top_level_tables(manifest, &["patch", "replace"]),
            [
                "patch.crates-io.uuid",
                "[ patch . \"https://github.com/example/fork\" ]",
                "[replace]",
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
