//! The dependency rules of each layer, over the crates as `cargo metadata` reports them.

use std::path::Path;

use super::{CrateInfo, Dep, DepKind, Origin, Violation, violation};
use crate::error::escaped;
use crate::layout::{HACK_DIR, KERNEL_DIR, Layer, layer_of};

/// From crates.io, for the normal and build dependencies of the kernel.
const KERNEL_ALLOWED: &[&str] = &["thiserror", "uuid", "time", "rust_decimal"];
/// From crates.io, for the normal and build dependencies of a module.
const MODULE_ALLOWED: &[&str] = &["thiserror", "uuid", "time", "rust_decimal", "async-trait"];
/// From crates.io, for dev dependencies, on top of the crate's own list.
const DEV_EXTRA: &[&str] = &["tokio", "mockall", "insta"];
/// The members the kernel may depend on, by folder, each only as a path dependency there.
const KERNEL_MEMBERS: &[&str] = &[HACK_DIR];
/// The members a module may depend on, by folder, each only as a path dependency there.
const MODULE_MEMBERS: &[&str] = &[KERNEL_DIR, HACK_DIR];

/// The package name the crate in the known folder `dir` must have. The cargo-deny bans
/// name crates, so a name that drifted from its folder would slip them.
pub fn expected_name(dir: &Path) -> Option<String> {
    let last = dir.file_name()?.to_str()?;
    Some(match layer_of(dir)? {
        Layer::Kernel => "barras-kernel".to_owned(),
        Layer::Module | Layer::App => format!("barras-{last}"),
        Layer::Adapter => format!("barras-adapter-{last}"),
        Layer::Bootstrap => "barras-bootstrap".to_owned(),
        Layer::Testkit => "barras-testkit".to_owned(),
        Layer::Tool => "xtask".to_owned(),
        Layer::WorkspaceHack => "workspace-hack".to_owned(),
    })
}

/// Violations come in input order of the crates, then in order of their dependencies.
pub fn check_arch(crates: &[CrateInfo]) -> Vec<Violation> {
    let mut violations = Vec::new();
    for krate in crates {
        match expected_name(&krate.dir) {
            None => violations.push(violation(krate, "", "unknown-location")),
            Some(expected) if expected != krate.name => violations.push(Violation {
                krate: escaped(&krate.dir),
                dependency: krate.name.clone(),
                rule: "package-name",
            }),
            Some(_) => {}
        }
        let layer = layer_of(&krate.dir);
        for dep in &krate.deps {
            for rule in broken_rules(crates, layer, dep) {
                violations.push(violation(krate, &dep.name, rule));
            }
        }
    }
    violations
}

/// The rules that `dep`, a dependency of a crate in `layer`, breaks. A dependency on a
/// member is known by the member it resolves to, found by package name and judged by its
/// folder, so renaming a package cannot slip a rule.
fn broken_rules(crates: &[CrateInfo], layer: Option<Layer>, dep: &Dep) -> Vec<&'static str> {
    let member = crates.iter().find(|c| c.name == dep.name);
    let member_layer = member.and_then(|m| layer_of(&m.dir));
    let mut broken = Vec::new();
    broken.extend(layer_rule(layer, dep, member));
    // A crate outside the members is never checked, so it could pass any member on.
    if dep.origin == Origin::Path
        && !crates
            .iter()
            .any(|c| dep.path.as_deref() == Some(c.dir.as_path()))
    {
        broken.push("path-outside-members");
    }
    if member_layer == Some(Layer::App) {
        broken.push("apps-are-roots");
    }
    if member_layer == Some(Layer::Testkit) && dep.kind != DepKind::Dev {
        broken.push("testkit-dev-only");
    }
    if member_layer == Some(Layer::Bootstrap) && layer != Some(Layer::App) {
        broken.push("bootstrap-apps-only");
    }
    broken
}

/// The rule of the crate's own layer that `dep` breaks, if any. `member` is the member
/// `dep` resolves to.
fn layer_rule(layer: Option<Layer>, dep: &Dep, member: Option<&CrateInfo>) -> Option<&'static str> {
    let dev = dep.kind == DepKind::Dev;
    let (rule, fits) = match layer? {
        Layer::Kernel if dev => (
            "kernel-dev-allowlist",
            allowed(&[KERNEL_ALLOWED, DEV_EXTRA], KERNEL_MEMBERS, dep, member),
        ),
        Layer::Kernel => (
            "kernel-allowlist",
            allowed(&[KERNEL_ALLOWED], KERNEL_MEMBERS, dep, member),
        ),
        Layer::Module if dev => (
            "module-dev-allowlist",
            allowed(&[MODULE_ALLOWED, DEV_EXTRA], MODULE_MEMBERS, dep, member),
        ),
        Layer::Module => (
            "module-allowlist",
            allowed(&[MODULE_ALLOWED], MODULE_MEMBERS, dep, member),
        ),
        Layer::Adapter => (
            "adapter-isolation",
            member.and_then(|m| layer_of(&m.dir)) != Some(Layer::Adapter),
        ),
        Layer::Bootstrap => (
            "bootstrap-isolation",
            member.is_none_or(|m| is_path_at(dep, m, HACK_DIR)),
        ),
        Layer::Tool => ("tool-isolation", member.is_none()),
        Layer::WorkspaceHack => ("hack-isolation", dep.origin == Origin::CratesIo),
        Layer::App | Layer::Testkit => return None,
    };
    (!fits).then_some(rule)
}

/// A dependency on a member counts only when that member's folder is in `members` and the
/// dependency takes it by path from there: a copy kept in any other folder, or taken from
/// git or a registry, has the same name and is other code. Any other name counts only from
/// crates.io, or a `path` or `git` entry, or another registry, could pass a foreign crate
/// off under an allowed name. `[patch]` and `[replace]` are stopped by `check_patches`.
fn allowed(lists: &[&[&str]], members: &[&str], dep: &Dep, member: Option<&CrateInfo>) -> bool {
    match member {
        Some(member) => members.iter().any(|folder| is_path_at(dep, member, folder)),
        None => {
            dep.origin == Origin::CratesIo
                && lists.iter().any(|list| list.contains(&dep.name.as_str()))
        }
    }
}

/// `member`, which `dep` resolves to, sits at `folder`, and `dep` takes it by path from there.
fn is_path_at(dep: &Dep, member: &CrateInfo, folder: &str) -> bool {
    let folder = Path::new(folder);
    member.dir == folder && dep.origin == Origin::Path && dep.path.as_deref() == Some(folder)
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;

    use super::*;
    use crate::arch::DepKind::{Build, Dev, Normal};

    const CRATES_IO: &str = "registry+https://github.com/rust-lang/crates.io-index";
    const GIT: &str = "git+https://github.com/example/fork?branch=main#0123456789abcdef";
    const OTHER_REGISTRY: &str = "sparse+https://registry.example.com/index/";
    /// Stands for the folder of the member of that name until `workspace` resolves it.
    const MEMBER_FOLDER: &str = "<the member's folder>";

    /// `true` marks a path dependency at the folder of the member of that name, `false` one
    /// from crates.io.
    fn krate(name: &str, dir: &str, deps: &[(&str, DepKind, bool)]) -> CrateInfo {
        let sources: Vec<(&str, DepKind, Option<&str>)> = deps
            .iter()
            .map(|(name, kind, is_path)| (*name, *kind, (!is_path).then_some(CRATES_IO)))
            .collect();
        krate_from(name, dir, &sources)
    }

    /// Each dependency carries the `source` that `cargo metadata` would report: `None` for
    /// a path, which is then the folder of the member of that name.
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
                    path: source.is_none().then(|| PathBuf::from(MEMBER_FOLDER)),
                })
                .collect(),
            targets: Vec::new(),
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
            targets: Vec::new(),
        }
    }

    /// The crates as `load_workspace` reads them: a path dependency on a member's name
    /// points at that member's folder, and one on any other name at a folder outside.
    fn workspace<const N: usize>(crates: [CrateInfo; N]) -> Vec<CrateInfo> {
        let folders: Vec<(String, PathBuf)> = crates
            .iter()
            .map(|c| (c.name.clone(), c.dir.clone()))
            .collect();
        crates
            .into_iter()
            .map(|mut krate| {
                for dep in &mut krate.deps {
                    if dep.path.as_deref() == Some(Path::new(MEMBER_FOLDER)) {
                        dep.path = Some(
                            folders
                                .iter()
                                .find(|(name, _)| *name == dep.name)
                                .map_or_else(
                                    || Path::new("/outside").join(&dep.name),
                                    |(_, dir)| dir.clone(),
                                ),
                        );
                    }
                }
                krate
            })
            .collect()
    }

    fn rules(violations: &[Violation]) -> Vec<(&str, &str, &str)> {
        violations
            .iter()
            .map(|v| (v.krate.as_str(), v.dependency.as_str(), v.rule))
            .collect()
    }

    #[test]
    fn the_empty_skeleton_is_clean() {
        let crates = workspace([
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
        ]);
        assert_eq!(check_arch(&crates), []);
    }

    #[test]
    fn a_module_cannot_reach_a_framework_or_another_module() {
        let crates = workspace([
            krate("barras-capital", "crates/modules/capital", &[]),
            krate(
                "barras-identity",
                "crates/modules/identity",
                &[
                    ("sqlx", Normal, false),
                    ("axum", Normal, false),
                    ("barras-capital", Normal, true),
                    ("sqlx", Dev, false),
                ],
            ),
        ]);
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
        let crates = workspace([
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
            krate("barras-testkit", "crates/testkit", &[]),
            krate("xtask", "crates/xtask", &[("barras-kernel", Normal, true)]),
            krate("barras-misc", "crates/misc", &[]),
        ]);
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
    fn each_known_folder_holds_the_package_of_its_name() {
        let named = [
            ("barras-kernel", "crates/kernel"),
            ("barras-identity", "crates/modules/identity"),
            ("barras-adapter-http", "crates/adapters/http"),
            ("barras-api", "apps/api"),
            ("barras-bootstrap", "crates/bootstrap"),
            ("barras-testkit", "crates/testkit"),
            ("workspace-hack", "crates/workspace-hack"),
            ("xtask", "crates/xtask"),
        ];
        let crates = named.map(|(name, dir)| krate(name, dir, &[]));
        assert_eq!(check_arch(&crates), []);
        let misnamed = [
            ("barras-core", "crates/kernel"),
            ("identity", "crates/modules/identity"),
            ("barras-http", "crates/adapters/http"),
            ("api", "apps/api"),
            ("barras-startup", "crates/bootstrap"),
            ("barras-fixtures", "crates/testkit"),
            ("hack", "crates/workspace-hack"),
            ("barras-xtask", "crates/xtask"),
        ];
        let crates = misnamed.map(|(name, dir)| krate(name, dir, &[]));
        assert_eq!(
            rules(&check_arch(&crates)),
            misnamed.map(|(name, dir)| (dir, name, "package-name"))
        );
    }

    #[test]
    fn a_path_dependency_outside_the_members_is_a_violation_in_any_crate_and_kind() {
        let mut crates = workspace([
            krate("barras-kernel", "crates/kernel", &[]),
            krate_at(
                "barras-api",
                "apps/api",
                &[
                    ("shim", Normal, "vendor/shim"),
                    ("shim", Dev, "/outside/vendor/shim"),
                    ("barras-kernel", Build, "crates/kernel"),
                ],
            ),
            krate_at(
                "barras-testkit",
                "crates/testkit",
                &[("shim", Build, "vendor/shim")],
            ),
        ]);
        // A path dependency whose folder cargo did not report is outside every member too.
        crates[1].deps.push(Dep {
            name: "shim".to_owned(),
            kind: Normal,
            origin: Origin::Path,
            path: None,
        });
        assert_eq!(
            rules(&check_arch(&crates)),
            [
                ("barras-api", "shim", "path-outside-members"),
                ("barras-api", "shim", "path-outside-members"),
                ("barras-api", "shim", "path-outside-members"),
                ("barras-testkit", "shim", "path-outside-members"),
            ]
        );
    }

    #[test]
    fn no_crate_may_depend_on_an_app_in_any_kind() {
        let crates = workspace([
            krate("barras-api", "apps/api", &[("barras-ingest", Dev, true)]),
            krate("barras-ingest", "apps/ingest", &[]),
            krate(
                "barras-adapter-http",
                "crates/adapters/http",
                &[("barras-api", Normal, true)],
            ),
            krate(
                "barras-testkit",
                "crates/testkit",
                &[("barras-ingest", Build, true)],
            ),
            krate_from(
                "barras-bootstrap",
                "crates/bootstrap",
                &[("barras-api", Normal, Some(GIT))],
            ),
        ]);
        assert_eq!(
            rules(&check_arch(&crates)),
            [
                ("barras-api", "barras-ingest", "apps-are-roots"),
                ("barras-adapter-http", "barras-api", "apps-are-roots"),
                ("barras-testkit", "barras-ingest", "apps-are-roots"),
                ("barras-bootstrap", "barras-api", "bootstrap-isolation"),
                ("barras-bootstrap", "barras-api", "apps-are-roots"),
            ]
        );
    }

    #[test]
    fn a_renamed_member_is_judged_by_its_folder() {
        let crates = workspace([
            krate("barras-core", "crates/kernel", &[]),
            krate("barras-startup", "crates/bootstrap", &[]),
            krate("barras-fixtures", "crates/testkit", &[]),
            krate("barras-db", "crates/adapters/postgres", &[]),
            krate(
                "barras-identity",
                "crates/modules/identity",
                &[("barras-core", Normal, true)],
            ),
            krate(
                "barras-adapter-http",
                "crates/adapters/http",
                &[
                    ("barras-startup", Normal, true),
                    ("barras-db", Normal, true),
                ],
            ),
            krate(
                "barras-api",
                "apps/api",
                &[
                    ("barras-startup", Normal, true),
                    ("barras-fixtures", Normal, true),
                ],
            ),
        ]);
        assert_eq!(
            rules(&check_arch(&crates)),
            [
                ("crates/kernel", "barras-core", "package-name"),
                ("crates/bootstrap", "barras-startup", "package-name"),
                ("crates/testkit", "barras-fixtures", "package-name"),
                ("crates/adapters/postgres", "barras-db", "package-name"),
                (
                    "barras-adapter-http",
                    "barras-startup",
                    "bootstrap-apps-only"
                ),
                ("barras-adapter-http", "barras-db", "adapter-isolation"),
                ("barras-api", "barras-fixtures", "testkit-dev-only"),
            ]
        );
    }

    #[test]
    fn an_allowed_name_from_the_wrong_source_is_a_violation() {
        let crates = workspace([
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
        ]);
        assert_eq!(
            rules(&check_arch(&crates)),
            [
                ("barras-kernel", "uuid", "kernel-allowlist"),
                ("barras-kernel", "uuid", "path-outside-members"),
                ("barras-identity", "barras-kernel", "module-allowlist"),
                ("barras-identity", "time", "module-allowlist"),
                ("barras-identity", "time", "path-outside-members"),
                ("barras-identity", "tokio", "module-dev-allowlist"),
                ("barras-identity", "tokio", "path-outside-members"),
            ]
        );
    }

    #[test]
    fn an_allowed_name_counts_only_from_crates_io() {
        let crates = workspace([
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
                    ("workspace-hack", Normal, Some(CRATES_IO)),
                ],
            ),
        ]);
        assert_eq!(
            rules(&check_arch(&crates)),
            [
                ("barras-kernel", "thiserror", "kernel-allowlist"),
                ("barras-identity", "uuid", "module-allowlist"),
                ("barras-identity", "time", "module-allowlist"),
                ("barras-identity", "workspace-hack", "module-allowlist"),
            ]
        );
    }

    #[test]
    fn a_workspace_crate_is_known_by_its_name_whatever_its_source() {
        let crates = workspace([
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
        ]);
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
        let crates = workspace([
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
        ]);
        assert_eq!(
            rules(&check_arch(&crates)),
            [
                ("barras-goals", "barras-kernel", "module-allowlist"),
                ("barras-goals", "barras-kernel", "path-outside-members"),
                ("barras-goals", "barras-kernel", "module-allowlist"),
                ("barras-goals", "barras-kernel", "path-outside-members"),
                ("barras-goals", "barras-kernel", "module-dev-allowlist"),
                ("barras-goals", "barras-kernel", "path-outside-members"),
            ]
        );
    }

    #[test]
    fn a_path_dependency_without_its_folder_never_counts() {
        let mut crates = workspace([
            krate("barras-kernel", "crates/kernel", &[]),
            krate_at(
                "barras-identity",
                "crates/modules/identity",
                &[("barras-kernel", Normal, "crates/kernel")],
            ),
        ]);
        crates[1].deps[0].path = None;
        assert_eq!(
            rules(&check_arch(&crates)),
            [
                ("barras-identity", "barras-kernel", "module-allowlist"),
                ("barras-identity", "barras-kernel", "path-outside-members"),
            ]
        );
    }

    #[test]
    fn the_hack_is_allowed_in_the_kernel_and_the_modules_from_its_own_folder() {
        let crates = workspace([
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
        ]);
        assert_eq!(check_arch(&crates), []);
    }

    #[test]
    fn the_hack_counts_only_from_its_own_folder() {
        let crates = workspace([
            krate("workspace-hack", "crates/workspace-hack", &[]),
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
                &[("workspace-hack", Normal, "vendor/workspace-hack")],
            ),
        ]);
        assert_eq!(
            rules(&check_arch(&crates)),
            [
                ("barras-kernel", "workspace-hack", "kernel-allowlist"),
                ("barras-kernel", "workspace-hack", "kernel-dev-allowlist"),
                ("barras-identity", "workspace-hack", "module-allowlist"),
                ("barras-identity", "workspace-hack", "path-outside-members"),
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
        let crates = workspace([
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
        ]);
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
        let crates = workspace([
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
        ]);
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
        assert_eq!(
            rules(&check_arch(&workspace([hack.clone(), from_another_folder]))),
            [
                ("barras-bootstrap", "workspace-hack", "bootstrap-isolation"),
                ("barras-bootstrap", "workspace-hack", "path-outside-members"),
            ]
        );
        let from_git = krate_from(
            "barras-bootstrap",
            "crates/bootstrap",
            &[("workspace-hack", Normal, Some(GIT))],
        );
        assert_eq!(
            rules(&check_arch(&workspace([hack, from_git]))),
            [("barras-bootstrap", "workspace-hack", "bootstrap-isolation")]
        );
    }

    #[test]
    fn only_an_app_may_depend_on_bootstrap() {
        let crates = workspace([
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
        ]);
        assert_eq!(
            rules(&check_arch(&crates)),
            [
                (
                    "barras-adapter-http",
                    "barras-bootstrap",
                    "bootstrap-apps-only"
                ),
                ("barras-testkit", "barras-bootstrap", "bootstrap-apps-only"),
                ("workspace-hack", "barras-bootstrap", "hack-isolation"),
                ("workspace-hack", "barras-bootstrap", "bootstrap-apps-only"),
            ]
        );
    }

    #[test]
    fn bootstrap_in_a_crate_with_its_own_rule_breaks_both() {
        let crates = workspace([
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
        ]);
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
        let crates = workspace([
            krate("workspace-hack", "crates/workspace-hack", &[]),
            krate_at(
                "xtask",
                "crates/xtask",
                &[("workspace-hack", Normal, "crates/workspace-hack")],
            ),
        ]);
        assert_eq!(
            rules(&check_arch(&crates)),
            [("xtask", "workspace-hack", "tool-isolation")]
        );
    }

    #[test]
    fn every_dependency_of_the_hack_comes_from_crates_io() {
        let crates = workspace([
            krate("barras-adapter-http", "crates/adapters/http", &[]),
            krate_from(
                "workspace-hack",
                "crates/workspace-hack",
                &[
                    ("tokio", Normal, Some(CRATES_IO)),
                    ("cc", Build, Some(CRATES_IO)),
                    ("barras-adapter-http", Normal, None),
                    ("regex", Build, Some(GIT)),
                    ("serde", Dev, Some(OTHER_REGISTRY)),
                ],
            ),
        ]);
        assert_eq!(
            rules(&check_arch(&crates)),
            [
                ("workspace-hack", "barras-adapter-http", "hack-isolation"),
                ("workspace-hack", "regex", "hack-isolation"),
                ("workspace-hack", "serde", "hack-isolation"),
            ]
        );
    }

    #[test]
    fn build_and_dev_dependencies_follow_the_rules_too() {
        let crates = workspace([
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
            krate("barras-testkit", "crates/testkit", &[]),
        ]);
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
