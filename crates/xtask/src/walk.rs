//! The one walk over a folder that every check uses. It never follows a symlink, so a
//! link can neither lead a check out of the folder it reads nor hide what it points at.

use std::fs::{self, FileType};
use std::path::{Path, PathBuf};

use crate::error::XtaskError;

/// What an entry is on disk, a symlink never followed.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum EntryKind {
    File,
    Folder,
    Symlink,
    /// A FIFO, a socket or a device.
    Special,
}

impl EntryKind {
    pub fn of(file_type: FileType) -> Self {
        if file_type.is_symlink() {
            Self::Symlink
        } else if file_type.is_dir() {
            Self::Folder
        } else if file_type.is_file() {
            Self::File
        } else {
            Self::Special
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Entry {
    /// Relative to the root the walk was given.
    pub path: PathBuf,
    pub kind: EntryKind,
}

impl Entry {
    /// `None` when the name is not UTF-8.
    pub fn name(&self) -> Option<&str> {
        self.path.file_name()?.to_str()
    }
}

/// Every entry under `root/folder`, each folder's entries in name order. A folder is
/// entered only when `enter` says so, and a symlink never is. The walk keeps its own list
/// of folders to visit rather than recursing, so no depth of folders can exhaust the stack.
pub fn walk(
    root: &Path,
    folder: &Path,
    enter: impl Fn(&Entry) -> bool,
) -> Result<Vec<Entry>, XtaskError> {
    let mut found = Vec::new();
    let mut pending = vec![folder.to_path_buf()];
    while let Some(dir) = pending.pop() {
        let absolute = root.join(&dir);
        let mut listed = Vec::new();
        for entry in fs::read_dir(&absolute).map_err(XtaskError::io(&absolute))? {
            let entry = entry.map_err(XtaskError::io(&absolute))?;
            let kind = EntryKind::of(entry.file_type().map_err(XtaskError::io(&entry.path()))?);
            listed.push(Entry {
                path: dir.join(entry.file_name()),
                kind,
            });
        }
        listed.sort_by(|left, right| left.path.cmp(&right.path));
        // Pushed last first, so that the folders are visited in name order too.
        for entry in listed.iter().rev() {
            if entry.kind == EntryKind::Folder && enter(entry) {
                pending.push(entry.path.clone());
            }
        }
        found.extend(listed);
    }
    Ok(found)
}

/// What `root/relative` is on disk, a symlink never followed; `None` when it does not exist.
pub fn kind_at(root: &Path, relative: &Path) -> Result<Option<EntryKind>, XtaskError> {
    let absolute = root.join(relative);
    match fs::symlink_metadata(&absolute) {
        Ok(found) => Ok(Some(EntryKind::of(found.file_type()))),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(None),
        Err(error) => Err(XtaskError::io(&absolute)(error)),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn touch(root: &Path, relative: &str) {
        let path = root.join(relative);
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(path, "").unwrap();
    }

    fn listed(entries: &[Entry]) -> Vec<(String, EntryKind)> {
        entries
            .iter()
            .map(|entry| (entry.path.display().to_string(), entry.kind))
            .collect()
    }

    #[test]
    fn every_entry_is_listed_and_only_the_folders_it_is_told_to_are_entered() {
        let root = tempfile::tempdir().unwrap();
        for file in ["src/lib.rs", "src/deep/x.rs", "skip/y.rs", "b.txt"] {
            touch(root.path(), file);
        }
        let entries = walk(root.path(), Path::new(""), |entry| {
            entry.name() != Some("skip")
        })
        .unwrap();
        let mut found = listed(&entries);
        found.sort();
        assert_eq!(
            found,
            [
                ("b.txt".to_owned(), EntryKind::File),
                ("skip".to_owned(), EntryKind::Folder),
                ("src".to_owned(), EntryKind::Folder),
                ("src/deep".to_owned(), EntryKind::Folder),
                ("src/deep/x.rs".to_owned(), EntryKind::File),
                ("src/lib.rs".to_owned(), EntryKind::File),
            ]
        );
    }

    #[cfg(unix)]
    #[test]
    fn a_symlink_and_a_fifo_are_listed_for_what_they_are_and_never_entered() {
        let root = tempfile::tempdir().unwrap();
        let elsewhere = tempfile::tempdir().unwrap();
        touch(elsewhere.path(), "never.rs");
        std::os::unix::fs::symlink(elsewhere.path(), root.path().join("linked")).unwrap();
        let made = std::process::Command::new("mkfifo")
            .arg(root.path().join("fifo"))
            .status()
            .unwrap();
        assert!(made.success());
        let entries = walk(root.path(), Path::new(""), |_| true).unwrap();
        assert_eq!(
            listed(&entries),
            [
                ("fifo".to_owned(), EntryKind::Special),
                ("linked".to_owned(), EntryKind::Symlink),
            ]
        );
        assert_eq!(
            kind_at(root.path(), Path::new("linked")).unwrap(),
            Some(EntryKind::Symlink)
        );
        assert_eq!(kind_at(root.path(), Path::new("gone")).unwrap(), None);
    }

    #[test]
    fn a_walk_from_a_subfolder_names_entries_from_the_root() {
        let root = tempfile::tempdir().unwrap();
        touch(root.path(), "crates/kernel/src/lib.rs");
        let entries = walk(root.path(), Path::new("crates/kernel/src"), |_| true).unwrap();
        assert_eq!(
            listed(&entries),
            [("crates/kernel/src/lib.rs".to_owned(), EntryKind::File)]
        );
    }
}
