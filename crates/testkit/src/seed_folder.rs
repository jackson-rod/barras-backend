//! A folder of seed files that one test writes.

use std::fs;
use std::path::{Path, PathBuf};

/// A folder of seed files that one test writes, removed when the value is dropped, also while a
/// failed test unwinds.
#[derive(Debug)]
pub struct SeedFolder(PathBuf);

impl SeedFolder {
    /// An empty folder in `parent`, named after `test` and this process. `parent` is the test
    /// target's `env!("CARGO_TARGET_TMPDIR")`: cargo sets it only while it compiles a test
    /// target, and this crate reads no environment variable to find it. A folder of the same name
    /// is emptied first: a run killed before its end leaves its folder behind, and process ids
    /// come back.
    pub fn new(parent: impl AsRef<Path>, test: &str) -> Self {
        let path = parent
            .as_ref()
            .join(format!("{test}-{}", std::process::id()));
        if path.exists() {
            fs::remove_dir_all(&path)
                .unwrap_or_else(|error| panic!("cannot empty {}: {error}", path.display()));
        }
        fs::create_dir_all(&path)
            .unwrap_or_else(|error| panic!("cannot create {}: {error}", path.display()));
        Self(path)
    }

    /// The folder, with a file `name` that holds `content`.
    pub fn with(self, name: &str, content: impl AsRef<[u8]>) -> Self {
        let file = self.0.join(name);
        fs::write(&file, content)
            .unwrap_or_else(|error| panic!("cannot write {}: {error}", file.display()));
        self
    }

    /// Where the folder is.
    pub fn path(&self) -> &Path {
        &self.0
    }
}

impl Drop for SeedFolder {
    fn drop(&mut self) {
        // A folder left behind only takes room in the target folder, and a test that unwinds
        // must not panic a second time.
        let _ = fs::remove_dir_all(&self.0);
    }
}
