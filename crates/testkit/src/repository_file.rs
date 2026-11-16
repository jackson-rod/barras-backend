//! Where a test finds a file of the repository.

use std::path::{Path, PathBuf};

/// The file or folder `name` at the root of the repository: two folders above this crate, which
/// lives in `crates/testkit` (`check-arch`).
pub fn repository_file(name: &str) -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../..")
        .join(name)
}

#[cfg(test)]
mod tests {
    use std::fs;

    use super::*;

    #[test]
    fn a_file_at_the_root_of_the_repository_is_found() {
        let manifest = fs::read_to_string(repository_file("Cargo.toml")).unwrap();

        assert!(manifest.contains("[workspace]"), "{manifest}");
    }
}
