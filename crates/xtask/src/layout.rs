//! Where the crates of this workspace sit: the one list of crate folders that every check
//! reads, so that no check can learn of a folder another one misses.

use std::path::{Path, PathBuf};

use crate::error::{LayoutProblem, XtaskError};
use crate::walk::{EntryKind, is_folder, walk};

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
const PARENTS: [(&str, Layer); 3] = [
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

/// Every folder of the workspace at `root` where a crate could sit, whether it holds one
/// or not: each folder directly inside a folder of `PARENTS`, in name order, then each
/// folder of `FOLDERS` that exists. A symlink or a special entry in any of these places is
/// refused, so a crate's code always lies inside the workspace. Hidden entries are left
/// out: Finder and editors drop them into any folder on their own, and check-arch finds a
/// crate there through `members` anyway.
pub fn crate_folders(root: &Path) -> Result<Vec<PathBuf>, XtaskError> {
    let mut found = Vec::new();
    for (parent, _) in PARENTS {
        if !is_folder(root, Path::new(parent))? {
            continue;
        }
        for entry in walk(root, Path::new(parent), |_| false)? {
            let refused = |problem| XtaskError::Layout {
                path: entry.path.clone(),
                problem,
            };
            let name = entry
                .name()
                .ok_or_else(|| refused(LayoutProblem::NotUtf8Name))?;
            if name.starts_with('.') {
                continue;
            }
            match entry.kind {
                EntryKind::Folder => found.push(entry.path),
                EntryKind::File => {}
                EntryKind::Symlink => return Err(refused(LayoutProblem::Symlink)),
                EntryKind::Special => return Err(refused(LayoutProblem::SpecialFile)),
            }
        }
    }
    for (folder, _) in FOLDERS {
        if is_folder(root, Path::new(folder))? {
            found.push(PathBuf::from(folder));
        }
    }
    Ok(found)
}

#[cfg(test)]
mod tests {
    use std::fs;

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
    fn the_crate_folders_are_the_known_folders_that_exist_hidden_entries_aside() {
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
            ["apps/api", "crates/modules/goals", "crates/kernel"].map(PathBuf::from)
        );
    }

    #[cfg(unix)]
    #[test]
    fn a_symlink_or_a_special_entry_where_a_crate_could_sit_is_refused() {
        use std::os::unix::fs::symlink;
        let elsewhere = tempfile::tempdir().unwrap();
        for (place, make_a_fifo) in [
            ("crates/modules/goals", false),
            ("crates/kernel", false),
            ("apps", false),
            ("crates/adapters/pipe", true),
        ] {
            let root = tempfile::tempdir().unwrap();
            let path = root.path().join(place);
            fs::create_dir_all(path.parent().unwrap()).unwrap();
            let problem = if make_a_fifo {
                let made = std::process::Command::new("mkfifo")
                    .arg(&path)
                    .status()
                    .unwrap();
                assert!(made.success());
                LayoutProblem::SpecialFile
            } else {
                symlink(elsewhere.path(), &path).unwrap();
                LayoutProblem::Symlink
            };
            let error = crate_folders(root.path()).unwrap_err();
            assert!(
                matches!(&error, XtaskError::Layout { path, problem: found }
                    if path == Path::new(place) && *found == problem),
                "{place}: {error}"
            );
        }
    }
}
