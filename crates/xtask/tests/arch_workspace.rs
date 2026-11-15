#![allow(clippy::unwrap_used, clippy::expect_used)] // integration test crate: every item is test code

use std::fs;
use std::path::Path;

use xtask::arch::{check_arch, load_workspace, workspace_root};

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
