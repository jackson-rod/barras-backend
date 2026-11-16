//! Where the crates of this workspace sit: the one list of crate folders that every check
//! reads, so that no check can learn of a folder another one misses.

use std::fs;
use std::path::{Path, PathBuf};

use crate::error::XtaskError;
use crate::walk::{EntryKind, kind_at};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Layer {
    Kernel,
    Module,
    Adapter,
    App,
    Bootstrap,
    Testkit,
    Tool,
    WorkspaceHack,
}

pub const KERNEL_DIR: &str = "crates/kernel";
/// The crate cargo-hakari generates. Every member but `xtask` depends on it.
pub const HACK_DIR: &str = "crates/workspace-hack";

/// The folders that hold one crate each.
const FOLDERS: [(&str, Layer); 5] = [
    ("crates/bootstrap", Layer::Bootstrap),
    (KERNEL_DIR, Layer::Kernel),
    ("crates/testkit", Layer::Testkit),
    (HACK_DIR, Layer::WorkspaceHack),
    ("crates/xtask", Layer::Tool),
];

/// The folders whose every subfolder holds a crate.
pub const PARENTS: [(&str, Layer); 3] = [
    ("apps", Layer::App),
    ("crates/modules", Layer::Module),
    ("crates/adapters", Layer::Adapter),
];

/// The layer of the crate folder `dir`, relative to the workspace root, if it is one.
pub fn layer_of(dir: &Path) -> Option<Layer> {
    if let Some((_, layer)) = FOLDERS.iter().find(|(folder, _)| dir == Path::new(folder)) {
        return Some(*layer);
    }
    let parent = dir.parent()?;
    dir.file_name()?;
    PARENTS
        .iter()
        .find(|(folder, _)| parent == Path::new(folder))
        .map(|(_, layer)| *layer)
}

/// Every place where a crate could sit in the workspace at `root`, whether it holds one
/// or not, with what it is on disk: each entry directly inside a folder of `PARENTS`, in
/// name order, then each folder of `FOLDERS` that exists. Hidden entries are left out:
/// Finder and editors drop them into any folder on their own.
pub fn crate_folders(root: &Path) -> Result<Vec<(PathBuf, EntryKind)>, XtaskError> {
    let mut found = Vec::new();
    for (parent, _) in PARENTS {
        let dir = root.join(parent);
        if !dir.is_dir() {
            continue;
        }
        let mut children = Vec::new();
        for entry in fs::read_dir(&dir).map_err(XtaskError::io(&dir))? {
            let entry = entry.map_err(XtaskError::io(&dir))?;
            if entry.file_name().to_string_lossy().starts_with('.') {
                continue;
            }
            let kind = EntryKind::of(entry.file_type().map_err(XtaskError::io(&entry.path()))?);
            children.push((Path::new(parent).join(entry.file_name()), kind));
        }
        children.sort_by(|(left, _), (right, _)| left.cmp(right));
        found.extend(children);
    }
    for (folder, _) in FOLDERS {
        if let Some(kind) = kind_at(root, Path::new(folder))? {
            found.push((PathBuf::from(folder), kind));
        }
    }
    Ok(found)
}

/// Whether `dir` is one of the folders that hold one crate each, rather than a folder
/// inside one of the parents.
pub fn is_fixed_folder(dir: &Path) -> bool {
    FOLDERS.iter().any(|(folder, _)| dir == Path::new(folder))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn each_known_folder_has_its_layer() {
        for (folder, layer) in [
            ("crates/kernel", Layer::Kernel),
            ("crates/modules/identity", Layer::Module),
            ("crates/adapters/http", Layer::Adapter),
            ("apps/api", Layer::App),
            ("crates/bootstrap", Layer::Bootstrap),
            ("crates/testkit", Layer::Testkit),
            ("crates/xtask", Layer::Tool),
            ("crates/workspace-hack", Layer::WorkspaceHack),
        ] {
            assert_eq!(layer_of(Path::new(folder)), Some(layer), "{folder}");
        }
    }

    #[test]
    fn any_other_folder_has_none() {
        for folder in [
            "crates",
            "crates/modules",
            "apps",
            "crates/misc",
            "crates/kernel/src",
            "crates/modules/identity/src",
            "crates/bootstrap/nested",
            "crates/workspace-hacks",
            "vendor/uuid",
            "",
        ] {
            assert_eq!(layer_of(Path::new(folder)), None, "{folder}");
        }
    }

    #[test]
    fn the_crate_folders_are_the_known_places_that_exist_hidden_entries_aside() {
        let root = tempfile::tempdir().unwrap();
        for folder in [
            "crates/kernel",
            "crates/modules/goals",
            "crates/modules/.scratch",
            "apps/api",
        ] {
            fs::create_dir_all(root.path().join(folder)).unwrap();
        }
        fs::write(root.path().join("apps/README.md"), "").unwrap();
        assert_eq!(
            crate_folders(root.path()).unwrap(),
            [
                ("apps/README.md", EntryKind::File),
                ("apps/api", EntryKind::Folder),
                ("crates/modules/goals", EntryKind::Folder),
                ("crates/kernel", EntryKind::Folder),
            ]
            .map(|(folder, kind)| (PathBuf::from(folder), kind))
        );
    }
}
