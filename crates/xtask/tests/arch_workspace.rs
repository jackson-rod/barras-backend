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

fn rules(violations: &[Violation]) -> Vec<(&str, &str, &str)> {
    violations
        .iter()
        .map(|v| (v.krate.as_str(), v.dependency.as_str(), v.rule))
        .collect()
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
