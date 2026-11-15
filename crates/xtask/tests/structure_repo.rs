#![allow(clippy::unwrap_used, clippy::expect_used)] // integration test crate: every item is test code

use xtask::arch::workspace_root;
use xtask::structure::check_structure;

#[test]
fn the_real_repository_is_clean_from_any_directory() {
    // workspace_root() does not depend on the current directory
    std::env::set_current_dir(workspace_root().join("crates/kernel")).unwrap();
    assert_eq!(
        check_structure(&workspace_root()).unwrap(),
        Vec::<String>::new()
    );
}
