//! Reading a file that someone else wrote: never a FIFO, a device or a symlink, and
//! never more than a known number of bytes.

use std::fs::{self, File, Metadata};
use std::io::{self, Read};
use std::path::Path;

use crate::error::XtaskError;

/// The text of the regular file at `path`, which must hold at most `limit` bytes.
pub fn read_regular_file(path: &Path, limit: u64) -> Result<String, XtaskError> {
    let not_regular = || XtaskError::NotARegularFile {
        path: path.to_path_buf(),
    };
    let too_large = || XtaskError::TooLarge {
        path: path.to_path_buf(),
        limit,
    };
    // Looked at before opening: opening a FIFO waits, for ever, for a writer.
    let before = fs::symlink_metadata(path).map_err(XtaskError::io(path))?;
    if !before.file_type().is_file() {
        return Err(not_regular());
    }
    if before.len() > limit {
        return Err(too_large());
    }
    let file = File::open(path).map_err(XtaskError::io(path))?;
    // The path may have been swapped between the look and the open.
    let opened = file.metadata().map_err(XtaskError::io(path))?;
    if !opened.file_type().is_file() || !same_file(&before, &opened) {
        return Err(not_regular());
    }
    let mut bytes = Vec::new();
    file.take(limit.saturating_add(1))
        .read_to_end(&mut bytes)
        .map_err(XtaskError::io(path))?;
    if u64::try_from(bytes.len()).map_or(true, |read| read > limit) {
        return Err(too_large());
    }
    String::from_utf8(bytes).map_err(|_| {
        XtaskError::io(path)(io::Error::new(
            io::ErrorKind::InvalidData,
            "the file is not UTF-8",
        ))
    })
}

#[cfg(unix)]
fn same_file(before: &Metadata, opened: &Metadata) -> bool {
    use std::os::unix::fs::MetadataExt;
    before.dev() == opened.dev() && before.ino() == opened.ino()
}

#[cfg(not(unix))]
fn same_file(_before: &Metadata, _opened: &Metadata) -> bool {
    true
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_regular_file_is_read() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("lib.rs");
        fs::write(&path, "fn main() {}\n").unwrap();
        assert_eq!(read_regular_file(&path, 100).unwrap(), "fn main() {}\n");
    }

    #[test]
    fn a_file_over_the_limit_is_refused() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("lib.rs");
        fs::write(&path, "0123456789").unwrap();
        assert_eq!(read_regular_file(&path, 10).unwrap(), "0123456789");
        let error = read_regular_file(&path, 9).unwrap_err();
        assert!(
            matches!(&error, XtaskError::TooLarge { limit: 9, .. }),
            "{error}"
        );
    }

    #[test]
    fn a_folder_a_missing_file_and_bytes_that_are_not_utf8_are_refused() {
        let dir = tempfile::tempdir().unwrap();
        let error = read_regular_file(dir.path(), 100).unwrap_err();
        assert!(
            matches!(error, XtaskError::NotARegularFile { .. }),
            "{error}"
        );
        let error = read_regular_file(&dir.path().join("gone.rs"), 100).unwrap_err();
        assert!(matches!(error, XtaskError::Io { .. }), "{error}");
        let path = dir.path().join("bytes.rs");
        fs::write(&path, [0xff, 0xfe]).unwrap();
        let error = read_regular_file(&path, 100).unwrap_err();
        assert!(matches!(error, XtaskError::Io { .. }), "{error}");
    }

    #[cfg(unix)]
    #[test]
    fn a_symlink_and_a_fifo_are_refused_without_waiting() {
        let dir = tempfile::tempdir().unwrap();
        let target = dir.path().join("target.rs");
        fs::write(&target, "fn main() {}\n").unwrap();
        let link = dir.path().join("link.rs");
        std::os::unix::fs::symlink(&target, &link).unwrap();
        let error = read_regular_file(&link, 100).unwrap_err();
        assert!(
            matches!(error, XtaskError::NotARegularFile { .. }),
            "{error}"
        );
        let fifo = dir.path().join("fifo.rs");
        let made = std::process::Command::new("mkfifo")
            .arg(&fifo)
            .status()
            .unwrap();
        assert!(made.success());
        let error = read_regular_file(&fifo, 100).unwrap_err();
        assert!(
            matches!(error, XtaskError::NotARegularFile { .. }),
            "{error}"
        );
    }
}
