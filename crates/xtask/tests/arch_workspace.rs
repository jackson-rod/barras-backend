#![allow(clippy::unwrap_used, clippy::expect_used)] // integration test crate: every item is test code

use std::fs;
use std::path::Path;

use xtask::arch::{Violation, check_arch, check_workspace, load_workspace, workspace_root};

fn write(root: &Path, relative: &str, content: &str) {
    let path = root.join(relative);
    fs::create_dir_all(path.parent().unwrap()).unwrap();
    fs::write(path, content).unwrap();
}

fn stub_package(root: &Path, dir: &str, name: &str, extra: &str) {
    write(
        root,
        &format!("{dir}/Cargo.toml"),
        &format!("[package]\nname = \"{name}\"\nversion = \"0.1.0\"\nedition = \"2024\"\n{extra}"),
    );
    write(root, &format!("{dir}/src/lib.rs"), "");
}

fn temp_workspace_with_hidden_sqlx() -> tempfile::TempDir {
    let root = tempfile::tempdir().unwrap();
    write(
        root.path(),
        "Cargo.toml",
        "[workspace]\nresolver = \"3\"\nmembers = [\"crates/modules/identity\", \"vendor/sqlx\"]\n",
    );
    stub_package(root.path(), "vendor/sqlx", "sqlx", "");
    stub_package(
        root.path(),
        "crates/modules/identity",
        "barras-identity",
        "\n[target.'cfg(unix)'.dependencies]\ndb = { package = \"sqlx\", path = \"../../../vendor/sqlx\", optional = true }\n",
    );
    root
}

/// One member, `crates/kernel`, and a stub `uuid` at `vendor/uuid` for a patch to point at.
fn temp_workspace(manifest_extra: &str) -> tempfile::TempDir {
    let root = tempfile::tempdir().unwrap();
    write(
        root.path(),
        "Cargo.toml",
        &format!("[workspace]\nresolver = \"3\"\nmembers = [\"crates/kernel\"]\n{manifest_extra}"),
    );
    stub_package(root.path(), "crates/kernel", "barras-kernel", "");
    stub_package(root.path(), "vendor/uuid", "uuid", "");
    root
}

/// `[dependencies]` lines that take each name from the workspace table.
fn inherited(names: &[&str]) -> String {
    let lines: Vec<String> = names
        .iter()
        .map(|name| format!("{name}.workspace = true\n"))
        .collect();
    format!("\n[dependencies]\n{}", lines.concat())
}

/// The members this ticket adds, wired as cargo-hakari wires them: the kernel, a module,
/// `crates/bootstrap`, `crates/workspace-hack` and an app, every dependency inherited.
/// `in_bootstrap` and `in_testkit` are extra dependencies of those two crates.
fn temp_workspace_with_bootstrap(in_bootstrap: &[&str], in_testkit: &[&str]) -> tempfile::TempDir {
    let root = tempfile::tempdir().unwrap();
    write(
        root.path(),
        "Cargo.toml",
        "[workspace]\nresolver = \"3\"\nmembers = [\"apps/api\", \"crates/kernel\", \"crates/modules/identity\", \"crates/bootstrap\", \"crates/testkit\", \"crates/workspace-hack\"]\n\n[workspace.dependencies]\nbarras-kernel = { path = \"crates/kernel\" }\nbarras-identity = { path = \"crates/modules/identity\" }\nbarras-bootstrap = { path = \"crates/bootstrap\" }\nworkspace-hack = { version = \"0.1\", path = \"crates/workspace-hack\" }\n",
    );
    let hack = "workspace-hack";
    stub_package(root.path(), "crates/workspace-hack", hack, "");
    stub_package(
        root.path(),
        "crates/kernel",
        "barras-kernel",
        &inherited(&[hack]),
    );
    stub_package(
        root.path(),
        "crates/modules/identity",
        "barras-identity",
        &inherited(&["barras-kernel", hack]),
    );
    stub_package(
        root.path(),
        "crates/bootstrap",
        "barras-bootstrap",
        &inherited(&[in_bootstrap, &[hack]].concat()),
    );
    stub_package(
        root.path(),
        "crates/testkit",
        "barras-testkit",
        &inherited(&[in_testkit, &[hack]].concat()),
    );
    stub_package(
        root.path(),
        "apps/api",
        "barras-api",
        &inherited(&["barras-bootstrap", "barras-identity", "barras-kernel", hack]),
    );
    root
}

fn rules(violations: &[Violation]) -> Vec<(&str, &str, &str)> {
    violations
        .iter()
        .map(|v| (v.krate.as_str(), v.dependency.as_str(), v.rule))
        .collect()
}

/// For a workspace of several members: cargo decides the order it reports them in.
fn sorted_rules(violations: &[Violation]) -> Vec<(&str, &str, &str)> {
    let mut found = rules(violations);
    found.sort_unstable();
    found
}

#[test]
fn the_real_workspace_has_no_patch_and_no_stray_crate() {
    assert_eq!(
        check_workspace(&workspace_root()).unwrap(),
        Vec::<Violation>::new()
    );
}

#[test]
fn a_patch_in_the_root_manifest_is_a_violation() {
    let root = temp_workspace("\n[patch.crates-io]\nuuid = { path = \"vendor/uuid\" }\n");
    assert_eq!(
        rules(&check_workspace(root.path()).unwrap()),
        [("Cargo.toml", "[patch.crates-io]", "no-patch")]
    );
}

#[test]
fn a_patch_in_the_cargo_config_is_a_violation() {
    let root = temp_workspace("");
    write(
        root.path(),
        ".cargo/config.toml",
        "[patch.crates-io]\nuuid = { path = \"vendor/uuid\" }\n",
    );
    assert_eq!(
        rules(&check_workspace(root.path()).unwrap()),
        [(".cargo/config.toml", "[patch.crates-io]", "no-patch")]
    );
}

#[test]
fn a_crate_folder_that_is_not_a_member_is_a_violation() {
    let root = temp_workspace("");
    stub_package(root.path(), "crates/modules/ghost", "barras-ghost", "");
    // Hidden entries are ignored, like everywhere else in the checks.
    stub_package(root.path(), "crates/modules/.scratch", "barras-scratch", "");
    assert_eq!(
        rules(&check_workspace(root.path()).unwrap()),
        [("crates/modules/ghost", "", "not-a-member")]
    );
}

#[test]
fn the_real_workspace_is_clean() {
    let crates = load_workspace(&workspace_root()).unwrap();
    assert_eq!(check_arch(&crates).len(), 0);
}

#[test]
fn a_renamed_optional_target_specific_dependency_is_still_caught() {
    // A temp workspace with `crates/modules/identity` (package barras-identity) and a local
    // stub package named `sqlx` at `vendor/sqlx`. The module declares it as:
    //   [target.'cfg(unix)'.dependencies]
    //   db = { package = "sqlx", path = "../../../vendor/sqlx", optional = true }
    // Each stub crate needs an empty src/lib.rs, or cargo refuses the manifest.
    let root = temp_workspace_with_hidden_sqlx();
    let violations = check_arch(&load_workspace(root.path()).unwrap());
    assert!(violations.iter().any(|v| v.krate == "barras-identity"
        && v.dependency == "sqlx"
        && v.rule == "module-allowlist"));
}

#[test]
fn a_kernel_taken_from_another_folder_is_refused() {
    // The copy builds: `exclude` keeps it out of the workspace, and its own version keeps
    // the two packages apart in the lock file.
    let root = tempfile::tempdir().unwrap();
    write(
        root.path(),
        "Cargo.toml",
        "[workspace]\nresolver = \"3\"\nmembers = [\"crates/kernel\", \"crates/modules/identity\"]\nexclude = [\"vendor\"]\n",
    );
    stub_package(root.path(), "crates/kernel", "barras-kernel", "");
    write(
        root.path(),
        "vendor/kernel/Cargo.toml",
        "[package]\nname = \"barras-kernel\"\nversion = \"9.9.9\"\nedition = \"2024\"\n",
    );
    write(root.path(), "vendor/kernel/src/lib.rs", "");
    stub_package(
        root.path(),
        "crates/modules/identity",
        "barras-identity",
        "\n[dependencies]\nbarras-kernel = { path = \"../../../vendor/kernel\" }\n",
    );
    assert_eq!(
        rules(&check_workspace(root.path()).unwrap()),
        [("barras-identity", "barras-kernel", "module-allowlist")]
    );
}

#[test]
fn a_kernel_taken_from_outside_the_workspace_is_refused() {
    let parent = tempfile::tempdir().unwrap();
    let root = parent.path().join("workspace");
    write(
        &root,
        "Cargo.toml",
        "[workspace]\nresolver = \"3\"\nmembers = [\"crates/kernel\", \"crates/modules/identity\"]\n",
    );
    stub_package(&root, "crates/kernel", "barras-kernel", "");
    // The same tail as the member's folder, so only the part before it tells them apart.
    write(
        parent.path(),
        "crates/kernel/Cargo.toml",
        "[package]\nname = \"barras-kernel\"\nversion = \"9.9.9\"\nedition = \"2024\"\n",
    );
    write(parent.path(), "crates/kernel/src/lib.rs", "");
    stub_package(
        &root,
        "crates/modules/identity",
        "barras-identity",
        "\n[dependencies]\nbarras-kernel = { path = \"../../../../crates/kernel\" }\n",
    );
    assert_eq!(
        rules(&check_workspace(&root).unwrap()),
        [("barras-identity", "barras-kernel", "module-allowlist")]
    );
}

#[test]
fn a_kernel_taken_from_its_own_folder_is_accepted_however_the_path_is_written() {
    let root = tempfile::tempdir().unwrap();
    write(
        root.path(),
        "Cargo.toml",
        "[workspace]\nresolver = \"3\"\nmembers = [\"crates/kernel\", \"crates/modules/identity\"]\n",
    );
    stub_package(root.path(), "crates/kernel", "barras-kernel", "");
    stub_package(
        root.path(),
        "crates/modules/identity",
        "barras-identity",
        "\n[dependencies]\nbarras-kernel = { path = \"../identity/../../kernel/\" }\n",
    );
    assert_eq!(check_workspace(root.path()).unwrap(), []);
}

#[test]
fn bootstrap_and_the_hack_fit_in_a_clean_workspace() {
    let root = temp_workspace_with_bootstrap(&[], &[]);
    assert_eq!(check_workspace(root.path()).unwrap(), []);
}

#[test]
fn the_bootstrap_rules_hold_through_cargo_metadata() {
    let root = temp_workspace_with_bootstrap(&["barras-kernel"], &["barras-bootstrap"]);
    assert_eq!(
        sorted_rules(&check_workspace(root.path()).unwrap()),
        [
            ("barras-bootstrap", "barras-kernel", "bootstrap-isolation"),
            ("barras-testkit", "barras-bootstrap", "bootstrap-apps-only"),
        ]
    );
}

#[test]
fn bootstrap_and_the_hack_must_be_members_when_they_hold_a_manifest() {
    let root = temp_workspace("");
    stub_package(root.path(), "crates/bootstrap", "barras-bootstrap", "");
    stub_package(root.path(), "crates/workspace-hack", "workspace-hack", "");
    assert_eq!(
        rules(&check_workspace(root.path()).unwrap()),
        [
            ("crates/bootstrap", "", "not-a-member"),
            ("crates/workspace-hack", "", "not-a-member"),
        ]
    );
}
