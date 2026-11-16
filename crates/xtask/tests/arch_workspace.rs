#![allow(clippy::unwrap_used, clippy::expect_used)] // integration test crate: every item is test code

use std::fs;
use std::path::Path;

use xtask::arch::{Violation, check_arch, check_workspace, load_workspace, workspace_root};
use xtask::error::{LayoutProblem, XtaskError};

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
        [
            ("barras-identity", "barras-kernel", "module-allowlist"),
            ("barras-identity", "barras-kernel", "path-outside-members"),
        ]
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
        [
            ("barras-identity", "barras-kernel", "module-allowlist"),
            ("barras-identity", "barras-kernel", "path-outside-members"),
        ]
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

/// A workspace of the members given as (folder, package name, extra manifest text), with
/// `root_extra` added to the root manifest.
fn workspace(members: &[(&str, &str, &str)], root_extra: &str) -> tempfile::TempDir {
    let root = tempfile::tempdir().unwrap();
    let listed: Vec<String> = members
        .iter()
        .map(|(dir, _, _)| format!("\"{dir}\""))
        .collect();
    write(
        root.path(),
        "Cargo.toml",
        &format!(
            "[workspace]\nresolver = \"3\"\nmembers = [{}]\n{root_extra}",
            listed.join(", ")
        ),
    );
    for (dir, name, extra) in members {
        stub_package(root.path(), dir, name, extra);
    }
    root
}

/// `[dependencies]` lines that take each (name, path) as a path dependency.
fn by_path(dependencies: &[(&str, &str)]) -> String {
    let lines: Vec<String> = dependencies
        .iter()
        .map(|(name, path)| format!("{name} = {{ path = \"{path}\" }}\n"))
        .collect();
    format!("\n[dependencies]\n{}", lines.concat())
}

#[test]
fn a_renamed_bootstrap_is_still_known_by_its_folder() {
    let root = workspace(
        &[
            ("crates/kernel", "barras-kernel", ""),
            ("crates/bootstrap", "barras-startup", ""),
            (
                "crates/adapters/http",
                "barras-adapter-http",
                &by_path(&[("barras-startup", "../../bootstrap")]),
            ),
        ],
        "",
    );
    assert_eq!(
        sorted_rules(&check_workspace(root.path()).unwrap()),
        [
            (
                "barras-adapter-http",
                "barras-startup",
                "bootstrap-apps-only"
            ),
            ("crates/bootstrap", "barras-startup", "package-name"),
        ]
    );
}

#[test]
fn a_renamed_testkit_is_still_known_by_its_folder() {
    let root = workspace(
        &[
            ("crates/kernel", "barras-kernel", ""),
            ("crates/testkit", "barras-fixtures", ""),
            (
                "apps/api",
                "barras-api",
                &by_path(&[("barras-fixtures", "../../crates/testkit")]),
            ),
        ],
        "",
    );
    assert_eq!(
        sorted_rules(&check_workspace(root.path()).unwrap()),
        [
            ("barras-api", "barras-fixtures", "testkit-dev-only"),
            ("crates/testkit", "barras-fixtures", "package-name"),
        ]
    );
}

#[test]
fn a_crate_outside_the_members_cannot_pass_members_on() {
    let shim = [("shim", "../../vendor/shim")];
    let root = workspace(
        &[
            ("apps/api", "barras-api", &by_path(&shim)),
            ("crates/kernel", "barras-kernel", ""),
            ("crates/bootstrap", "barras-bootstrap", &by_path(&shim)),
            (
                "crates/adapters/http",
                "barras-adapter-http",
                &by_path(&[("shim", "../../../vendor/shim")]),
            ),
            ("crates/adapters/postgres", "barras-adapter-postgres", ""),
            ("crates/testkit", "barras-testkit", ""),
            ("crates/xtask", "xtask", &by_path(&shim)),
        ],
        "exclude = [\"vendor\"]\n",
    );
    // `exclude` keeps the shim out of the members, so cargo builds it as a plain path
    // dependency, and what it re-exports reaches the four crates with no rule seeing it.
    stub_package(
        root.path(),
        "vendor/shim",
        "shim",
        &by_path(&[
            ("barras-kernel", "../../crates/kernel"),
            ("barras-adapter-postgres", "../../crates/adapters/postgres"),
            ("barras-testkit", "../../crates/testkit"),
        ]),
    );
    write(
        root.path(),
        "vendor/shim/src/lib.rs",
        "pub use barras_adapter_postgres;\npub use barras_kernel;\npub use barras_testkit;\n",
    );
    assert_eq!(
        sorted_rules(&check_workspace(root.path()).unwrap()),
        [
            ("barras-adapter-http", "shim", "path-outside-members"),
            ("barras-api", "shim", "path-outside-members"),
            ("barras-bootstrap", "shim", "path-outside-members"),
            ("xtask", "shim", "path-outside-members"),
        ]
    );
}

#[test]
fn no_crate_may_depend_on_an_app() {
    let root = workspace(
        &[
            ("apps/api", "barras-api", ""),
            (
                "apps/glue",
                "barras-glue",
                &by_path(&[
                    ("barras-adapter-postgres", "../../crates/adapters/postgres"),
                    ("barras-bootstrap", "../../crates/bootstrap"),
                ]),
            ),
            ("crates/kernel", "barras-kernel", ""),
            ("crates/bootstrap", "barras-bootstrap", ""),
            (
                "crates/adapters/http",
                "barras-adapter-http",
                &by_path(&[("barras-glue", "../../../apps/glue")]),
            ),
            ("crates/adapters/postgres", "barras-adapter-postgres", ""),
        ],
        "",
    );
    assert_eq!(
        rules(&check_workspace(root.path()).unwrap()),
        [("barras-adapter-http", "barras-glue", "apps-are-roots")]
    );
}

/// The kernel, the HTTP adapter and a hack crate shaped as cargo-hakari writes it, with
/// `hack_extra` after its `### END HAKARI SECTION` line and `hack_lib` as its `src/lib.rs`.
fn workspace_with_a_hack(hack_extra: &str, hack_lib: &str) -> tempfile::TempDir {
    let root = workspace(
        &[
            (
                "crates/kernel",
                "barras-kernel",
                &by_path(&[("workspace-hack", "../workspace-hack")]),
            ),
            ("crates/adapters/http", "barras-adapter-http", ""),
            (
                "crates/workspace-hack",
                "workspace-hack",
                &format!(
                    "description = \"workspace-hack package, managed by hakari\"\npublish = false\n\n### BEGIN HAKARI SECTION\n### END HAKARI SECTION\n{hack_extra}"
                ),
            ),
        ],
        "",
    );
    write(root.path(), "crates/workspace-hack/src/lib.rs", hack_lib);
    write(
        root.path(),
        "crates/workspace-hack/build.rs",
        "// A build script is required for cargo to consider build dependencies.\nfn main() {}\n",
    );
    root
}

const HAKARI_LIB: &str = "// This is a stub lib.rs.\n";

#[test]
fn the_hack_as_cargo_hakari_writes_it_is_clean() {
    let root = workspace_with_a_hack("\n[dependencies]\nregex = \"1.13.1\"\n", HAKARI_LIB);
    assert_eq!(check_workspace(root.path()).unwrap(), []);
}

#[test]
fn the_hack_may_take_nothing_by_path_nor_carry_code() {
    let root = workspace_with_a_hack(
        "\n[dependencies]\nbarras-adapter-http = { path = \"../adapters/http\" }\n",
        "// This is a stub lib.rs.\npub use barras_adapter_http;\n",
    );
    write(
        root.path(),
        "crates/kernel/src/lib.rs",
        "pub use workspace_hack::barras_adapter_http;\n",
    );
    assert_eq!(
        sorted_rules(&check_workspace(root.path()).unwrap()),
        [
            ("workspace-hack", "barras-adapter-http", "hack-isolation"),
            (
                "workspace-hack",
                "crates/workspace-hack/src/lib.rs",
                "hack-is-a-stub"
            ),
        ]
    );
}

#[test]
fn the_hack_may_not_pass_on_a_crate_from_crates_io() {
    let root = workspace_with_a_hack(
        "\n[dependencies]\nregex = \"1.13.1\"\n",
        "// This is a stub lib.rs.\npub use regex;\n\npub fn business_rule(amount: u64) -> u64 {\n    amount / 100\n}\n",
    );
    assert_eq!(
        rules(&check_workspace(root.path()).unwrap()),
        [(
            "workspace-hack",
            "crates/workspace-hack/src/lib.rs",
            "hack-is-a-stub"
        )]
    );
}

#[test]
fn the_hack_has_only_its_library_at_src_lib_rs_and_an_empty_build_script() {
    for (hack_extra, file, content, offender) in [
        (
            "\n[lib]\npath = \"src/other.rs\"\n",
            "src/other.rs",
            HAKARI_LIB,
            "crates/workspace-hack/src/other.rs",
        ),
        (
            "\n[[bin]]\nname = \"tool\"\npath = \"src/bin/tool.rs\"\n",
            "src/bin/tool.rs",
            "fn main() {}\n",
            "crates/workspace-hack/src/bin/tool.rs",
        ),
        (
            "",
            "build.rs",
            "fn main() {\n    println!(\"cargo::rustc-cfg=hacked\");\n}\n",
            "crates/workspace-hack/build.rs",
        ),
        (
            "",
            "src/extra.rs",
            HAKARI_LIB,
            "crates/workspace-hack/src/extra.rs",
        ),
    ] {
        let root = workspace_with_a_hack(hack_extra, HAKARI_LIB);
        write(
            root.path(),
            &format!("crates/workspace-hack/{file}"),
            content,
        );
        assert_eq!(
            rules(&check_workspace(root.path()).unwrap()),
            [("workspace-hack", offender, "hack-is-a-stub")],
            "{offender}"
        );
    }
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

#[cfg(unix)]
#[test]
fn a_symlink_in_the_hack_folder_is_refused_wherever_it_points() {
    // The hack's `src` points at a folder outside the crate, so cargo builds the library
    // from there and no check reads it through the crate folder.
    let root = workspace_with_a_hack("\n[dependencies]\nregex = \"1.13.1\"\n", HAKARI_LIB);
    fs::remove_dir_all(root.path().join("crates/workspace-hack/src")).unwrap();
    write(
        root.path(),
        "vendor/hack-src/lib.rs",
        "// This is a stub lib.rs.\npub use regex;\n\npub fn smuggled() {}\n",
    );
    std::os::unix::fs::symlink(
        "../../vendor/hack-src",
        root.path().join("crates/workspace-hack/src"),
    )
    .unwrap();
    assert_eq!(
        rules(&check_workspace(root.path()).unwrap()),
        [(
            "workspace-hack",
            "crates/workspace-hack/src",
            "hack-is-a-stub"
        )]
    );

    let root = workspace_with_a_hack("", HAKARI_LIB);
    std::os::unix::fs::symlink(
        "/etc/hosts",
        root.path().join("crates/workspace-hack/NOTES"),
    )
    .unwrap();
    assert_eq!(
        rules(&check_workspace(root.path()).unwrap()),
        [(
            "workspace-hack",
            "crates/workspace-hack/NOTES",
            "hack-is-a-stub"
        )]
    );
}

#[cfg(unix)]
#[test]
fn a_fifo_in_the_hack_folder_is_refused_without_waiting() {
    let root = workspace_with_a_hack("", HAKARI_LIB);
    let made = std::process::Command::new("mkfifo")
        .arg(root.path().join("crates/workspace-hack/src/pipe"))
        .status()
        .unwrap();
    assert!(made.success());
    assert_eq!(
        rules(&check_workspace(root.path()).unwrap()),
        [(
            "workspace-hack",
            "crates/workspace-hack/src/pipe",
            "hack-is-a-stub"
        )]
    );
}

#[test]
fn the_kernel_and_the_modules_have_no_build_script() {
    let root = workspace(
        &[
            ("crates/kernel", "barras-kernel", ""),
            ("crates/modules/pricing", "barras-pricing", ""),
            ("crates/bootstrap", "barras-bootstrap", ""),
            ("crates/adapters/http", "barras-adapter-http", ""),
            ("apps/api", "barras-api", ""),
        ],
        "",
    );
    let script = "fn main() {\n    if !cfg!(debug_assertions) {\n        println!(\"cargo::rustc-cfg=shipping\");\n    }\n}\n";
    for folder in [
        "crates/kernel",
        "crates/modules/pricing",
        "crates/bootstrap",
        "crates/adapters/http",
        "apps/api",
    ] {
        write(root.path(), &format!("{folder}/build.rs"), script);
    }
    assert_eq!(
        sorted_rules(&check_workspace(root.path()).unwrap()),
        [
            ("barras-kernel", "crates/kernel/build.rs", "no-build-script"),
            (
                "barras-pricing",
                "crates/modules/pricing/build.rs",
                "no-build-script"
            ),
        ]
    );
}

/// The kernel and the HTTP adapter, each with a `testing` feature that the other members
/// may turn on, as the real workspace has them.
const KERNEL_WITH_TESTING: (&str, &str, &str) = (
    "crates/kernel",
    "barras-kernel",
    "\n[features]\ntesting = []\n",
);

#[test]
fn testing_features_turned_on_in_dev_dependencies_only_are_clean() {
    let root = workspace(
        &[
            KERNEL_WITH_TESTING,
            (
                "crates/adapters/http",
                "barras-adapter-http",
                "\n[features]\ntesting = [\"barras-kernel/testing\"]\n\n[dependencies]\nbarras-kernel.workspace = true\n\n[dev-dependencies]\nbarras-kernel = { workspace = true, features = [\"testing\"] }\n",
            ),
            (
                "apps/api",
                "barras-api",
                "\n[dependencies]\nbarras-adapter-http.workspace = true\n\n[dev-dependencies]\nbarras-adapter-http = { workspace = true, features = [\"testing\"] }\n",
            ),
        ],
        "\n[workspace.dependencies]\nbarras-kernel = { path = \"crates/kernel\" }\nbarras-adapter-http = { path = \"crates/adapters/http\" }\n",
    );
    assert_eq!(check_workspace(root.path()).unwrap(), []);
}

#[test]
fn a_testing_feature_turned_on_outside_a_dev_dependency_is_refused() {
    // The workspace entry turns the kernel's on in every crate that inherits it, and cargo
    // reports it on each of them.
    let root = workspace(
        &[
            KERNEL_WITH_TESTING,
            (
                "crates/adapters/http",
                "barras-adapter-http",
                "\n[features]\ntesting = []\n\n[dependencies]\nbarras-kernel.workspace = true\n",
            ),
            (
                "apps/api",
                "barras-api",
                "\n[dependencies]\nbarras-adapter-http = { path = \"../../crates/adapters/http\", features = [\"testing\"] }\n\n[build-dependencies]\nbarras-adapter-http = { path = \"../../crates/adapters/http\", features = [\"testing\"] }\n\n[dev-dependencies]\nbarras-kernel.workspace = true\n",
            ),
            (
                "crates/testkit",
                "barras-testkit",
                "\n[dependencies]\nbarras-kernel = { path = \"../kernel\", features = [\"testing\"] }\n",
            ),
        ],
        "\n[workspace.dependencies]\nbarras-kernel = { path = \"crates/kernel\", features = [\"testing\"] }\n",
    );
    assert_eq!(
        sorted_rules(&check_workspace(root.path()).unwrap()),
        [
            ("barras-adapter-http", "barras-kernel", "testing-dev-only"),
            ("barras-api", "barras-adapter-http", "testing-dev-only"),
            ("barras-api", "barras-adapter-http", "testing-dev-only"),
            ("barras-testkit", "barras-kernel", "testing-dev-only"),
        ]
    );
}

#[test]
fn a_feature_other_than_testing_may_not_turn_a_testing_feature_on() {
    // `cargo build --workspace` turns on every member's default features, so `default`, or
    // a feature that some normal dependency turns on, would carry test code into the build.
    let root = workspace(
        &[
            (
                "crates/kernel",
                "barras-kernel",
                "\n[features]\ndefault = [\"testing\"]\ntesting = []\n",
            ),
            (
                "crates/adapters/http",
                "barras-adapter-http",
                "\n[features]\ntesting = []\n",
            ),
            (
                "apps/api",
                "barras-api",
                "\n[features]\nclock = [\"kernel/testing\"]\nclient = [\"dep:http\", \"http?/testing\"]\n\n[dependencies]\nkernel = { package = \"barras-kernel\", path = \"../../crates/kernel\", default-features = false }\nhttp = { package = \"barras-adapter-http\", path = \"../../crates/adapters/http\", optional = true }\n",
            ),
        ],
        "",
    );
    assert_eq!(
        sorted_rules(&check_workspace(root.path()).unwrap()),
        [
            (
                "barras-api",
                "[features] client = http?/testing",
                "testing-dev-only"
            ),
            (
                "barras-api",
                "[features] clock = kernel/testing",
                "testing-dev-only"
            ),
            (
                "barras-kernel",
                "[features] default = testing",
                "testing-dev-only"
            ),
        ]
    );
}

#[cfg(unix)]
#[test]
fn a_crate_folder_that_is_a_symlink_is_refused() {
    let root = workspace(&[("crates/kernel", "barras-kernel", "")], "");
    stub_package(root.path(), "hidden/sym", "barras-sym", "");
    fs::create_dir_all(root.path().join("crates/modules")).unwrap();
    std::os::unix::fs::symlink("../../hidden/sym", root.path().join("crates/modules/sym")).unwrap();
    write(
        root.path(),
        "Cargo.toml",
        "[workspace]\nresolver = \"3\"\nmembers = [\"crates/kernel\", \"crates/modules/sym\"]\n",
    );
    let error = check_workspace(root.path()).unwrap_err();
    assert!(
        matches!(&error, XtaskError::Layout { path, problem: LayoutProblem::Symlink }
            if path == Path::new("crates/modules/sym")),
        "{error}"
    );
}
