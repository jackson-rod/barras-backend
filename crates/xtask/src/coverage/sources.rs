//! The sources the measure reads: every `.rs` file under the `src/` of each measured
//! crate, whether the report names it or not.

use std::collections::BTreeMap;
use std::io;
use std::path::{Path, PathBuf};
use std::thread;

use super::source::{Finding, Source};
use crate::error::{LayoutProblem, XtaskError};
use crate::layout::{Layer, crate_folders, layer_of};
use crate::regular_file::read_regular_file;
use crate::walk::{EntryKind, is_folder, walk};

/// A hand-written source is far smaller; anything larger is refused unread.
const SOURCE_LIMIT: u64 = 1024 * 1024;

/// The folders, relative to `root`, of the measured crates: those of `crates/kernel`,
/// `crates/modules/<m>`, `crates/adapters/<a>`, `crates/bootstrap`, `crates/xtask` and
/// `apps/<a>` that hold a `Cargo.toml`.
pub fn measured_crates(root: &Path) -> Result<Vec<PathBuf>, XtaskError> {
    let mut crates: Vec<PathBuf> = crate_folders(root)?
        .into_iter()
        .filter(|folder| {
            !matches!(
                layer_of(folder),
                Some(Layer::Testkit | Layer::WorkspaceHack)
            )
        })
        .filter(|folder| root.join(folder).join("Cargo.toml").is_file())
        .collect();
    crates.sort();
    Ok(crates)
}

/// Every source under the `src/` of each of `crates`, by its path relative to `root`. A
/// `tests.rs` file or a file under a `tests` folder is a test module, which cargo-llvm-cov
/// leaves out of the report: it is only checked for `include` and `#[path]`, and is not
/// one of the sources. A hidden entry is not read: no module can name it, and a report
/// that measures one is refused as a file that was not read.
pub fn read_sources(
    root: &Path,
    crates: &[PathBuf],
) -> Result<BTreeMap<PathBuf, Source>, XtaskError> {
    let mut sources = BTreeMap::new();
    for folder in crates {
        let src = folder.join("src");
        if !is_folder(root, &src)? {
            continue;
        }
        let entries = walk(root, &src, |entry| {
            entry.name().is_some_and(|name| !name.starts_with('.'))
        })?;
        for entry in entries {
            let name = utf8_name(&entry.path)?;
            if name.starts_with('.') {
                continue;
            }
            let layout = |problem| XtaskError::Layout {
                path: entry.path.clone(),
                problem,
            };
            let test_module = is_test_module(&src, &entry.path);
            match entry.kind {
                EntryKind::Symlink => return Err(layout(LayoutProblem::Symlink)),
                EntryKind::Special => return Err(layout(LayoutProblem::SpecialFile)),
                EntryKind::Folder if !test_module && matches!(name, "examples" | "benches") => {
                    return Err(layout(LayoutProblem::DroppedFolder));
                }
                EntryKind::Folder => {}
                EntryKind::File if !name.ends_with(".rs") => {}
                EntryKind::File
                    if !test_module
                        && (name.ends_with("_tests.rs") || name.ends_with("-tests.rs")) =>
                {
                    return Err(layout(LayoutProblem::DroppedFileName));
                }
                EntryKind::File => {
                    let text = read_regular_file(&root.join(&entry.path), SOURCE_LIMIT)?;
                    let at_its_line = |finding: Finding| XtaskError::Source {
                        path: entry.path.clone(),
                        line: finding.line,
                        problem: finding.problem,
                    };
                    if test_module {
                        on_a_large_stack(|| Source::check_test_module(&text))?
                            .map_err(at_its_line)?;
                    } else {
                        let source =
                            on_a_large_stack(|| Source::read(&text))?.map_err(at_its_line)?;
                        sources.insert(entry.path, source);
                    }
                }
            }
        }
    }
    Ok(sources)
}

/// A `tests.rs` file, or anything under a folder named `tests`, inside `src`.
fn is_test_module(src: &Path, path: &Path) -> bool {
    let inner = path.strip_prefix(src).unwrap_or(path);
    inner.iter().any(|part| part == "tests")
        || inner.file_name().is_some_and(|name| name == "tests.rs")
}

/// syn parses by recursion, a few frames for each level of nesting: 256 nested blocks take
/// about 8 MiB of stack in a debug build, more than a thread is given by default.
const PARSE_STACK: usize = 64 * 1024 * 1024;

/// The result of `work`, run on a thread of its own with a large stack.
fn on_a_large_stack<T: Send>(work: impl FnOnce() -> T + Send) -> Result<T, XtaskError> {
    thread::scope(|scope| {
        thread::Builder::new()
            .stack_size(PARSE_STACK)
            .spawn_scoped(scope, work)
            .map_err(XtaskError::ParseThread)?
            .join()
            .map_err(|_| XtaskError::ParseThread(io::Error::other("the parse stopped short")))
    })
}

fn utf8_name(path: &Path) -> Result<&str, XtaskError> {
    path.file_name()
        .and_then(|name| name.to_str())
        .ok_or_else(|| XtaskError::Layout {
            path: path.to_path_buf(),
            problem: LayoutProblem::NotUtf8Name,
        })
}

#[cfg(test)]
mod tests {
    use std::fs;

    use super::*;
    use crate::error::SourceProblem;

    fn write(root: &Path, relative: &str, text: &str) {
        let path = root.join(relative);
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(path, text).unwrap();
    }

    fn workspace(files: &[&str]) -> tempfile::TempDir {
        let root = tempfile::tempdir().unwrap();
        for file in files {
            write(root.path(), file, "pub fn production() {}\n");
        }
        root
    }

    fn read_all(root: &Path) -> Result<BTreeMap<PathBuf, Source>, XtaskError> {
        read_sources(root, &measured_crates(root)?)
    }

    #[test]
    fn the_measured_crates_are_the_known_folders_that_hold_a_manifest() {
        let root = workspace(&[
            "apps/api/Cargo.toml",
            "apps/notes/README.md",
            "apps/.scratch/Cargo.toml",
            "crates/kernel/Cargo.toml",
            "crates/modules/goals/Cargo.toml",
            "crates/adapters/http/Cargo.toml",
            "crates/bootstrap/Cargo.toml",
            "crates/xtask/Cargo.toml",
            "crates/testkit/Cargo.toml",
            "crates/workspace-hack/Cargo.toml",
            "crates/misc/Cargo.toml",
        ]);
        assert_eq!(
            measured_crates(root.path()).unwrap(),
            [
                "apps/api",
                "crates/adapters/http",
                "crates/bootstrap",
                "crates/kernel",
                "crates/modules/goals",
                "crates/xtask",
            ]
            .map(PathBuf::from)
        );
    }

    #[test]
    fn every_rs_file_under_src_is_read_but_test_modules_and_hidden_entries() {
        let root = workspace(&[
            "crates/kernel/Cargo.toml",
            "crates/kernel/src/lib.rs",
            "crates/kernel/src/money/mod.rs",
            "crates/kernel/src/money/parse.rs",
            "crates/kernel/src/README.md",
            "crates/kernel/build.rs",
            "crates/kernel/tests/it.rs",
            "apps/api/Cargo.toml",
            "apps/api/src/main.rs",
        ]);
        for test_module in [
            "crates/kernel/src/tests.rs",
            "crates/kernel/src/money/tests/helpers.rs",
        ] {
            // Read for `include` and `#[path]` only: nothing else here is refused.
            write(
                root.path(),
                test_module,
                "#[cfg(not(coverage))]\nfn helper() {}\n",
            );
        }
        for hidden in [
            "crates/kernel/src/.hidden.rs",
            "crates/kernel/src/.cache/x.rs",
        ] {
            write(root.path(), hidden, "fn not_parsed( {\n");
        }
        let read: Vec<PathBuf> = read_all(root.path()).unwrap().into_keys().collect();
        assert_eq!(
            read,
            [
                "apps/api/src/main.rs",
                "crates/kernel/src/lib.rs",
                "crates/kernel/src/money/mod.rs",
                "crates/kernel/src/money/parse.rs",
            ]
            .map(PathBuf::from)
        );
    }

    #[test]
    fn a_file_or_a_folder_that_cargo_llvm_cov_leaves_out_is_refused() {
        for (file, offender, problem) in [
            (
                "crates/kernel/src/ab_tests.rs",
                "crates/kernel/src/ab_tests.rs",
                LayoutProblem::DroppedFileName,
            ),
            (
                "crates/kernel/src/money/x-tests.rs",
                "crates/kernel/src/money/x-tests.rs",
                LayoutProblem::DroppedFileName,
            ),
            (
                "crates/kernel/src/examples/mod.rs",
                "crates/kernel/src/examples",
                LayoutProblem::DroppedFolder,
            ),
            (
                "crates/kernel/src/money/benches/x.rs",
                "crates/kernel/src/money/benches",
                LayoutProblem::DroppedFolder,
            ),
        ] {
            let root = workspace(&["crates/kernel/Cargo.toml", "crates/kernel/src/lib.rs", file]);
            let error = read_all(root.path()).unwrap_err();
            assert!(
                matches!(&error, XtaskError::Layout { path, problem: found }
                    if path == Path::new(offender) && *found == problem),
                "{file}: {error}"
            );
        }
    }

    #[test]
    fn a_source_that_breaks_a_rule_is_refused_at_its_line() {
        let root = workspace(&["crates/kernel/Cargo.toml"]);
        write(
            root.path(),
            "crates/kernel/src/lib.rs",
            "pub fn f() {}\n#[cfg(not(coverage))]\npub fn g() {}\n",
        );
        let error = read_all(root.path()).unwrap_err();
        assert!(
            matches!(&error, XtaskError::Source { path, line: 2, problem: SourceProblem::NamesCoverage }
                if path == Path::new("crates/kernel/src/lib.rs")),
            "{error}"
        );
    }

    #[test]
    fn a_source_over_1_mib_is_refused_unread() {
        let root = workspace(&["crates/kernel/Cargo.toml", "crates/kernel/src/lib.rs"]);
        let big = root.path().join("crates/kernel/src/big.rs");
        fs::File::create(&big)
            .unwrap()
            .set_len(SOURCE_LIMIT + 1)
            .unwrap();
        let error = read_all(root.path()).unwrap_err();
        assert!(
            matches!(&error, XtaskError::TooLarge { path, .. } if *path == big),
            "{error}"
        );
    }

    #[cfg(unix)]
    #[test]
    fn a_symlink_or_a_fifo_under_a_measured_src_is_refused() {
        use std::os::unix::fs::symlink;
        let link_to_a_file = |root: &Path| {
            symlink("/etc/hosts", root.join("crates/kernel/src/link.rs")).unwrap();
            "crates/kernel/src/link.rs"
        };
        let link_to_a_folder = |root: &Path| {
            symlink("/etc", root.join("crates/kernel/src/linked")).unwrap();
            "crates/kernel/src/linked"
        };
        let fifo = |root: &Path| {
            let made = std::process::Command::new("mkfifo")
                .arg(root.join("crates/kernel/src/fifo.rs"))
                .status()
                .unwrap();
            assert!(made.success());
            "crates/kernel/src/fifo.rs"
        };
        for (make, problem) in [
            (
                &link_to_a_file as &dyn Fn(&Path) -> &'static str,
                LayoutProblem::Symlink,
            ),
            (&link_to_a_folder, LayoutProblem::Symlink),
            (&fifo, LayoutProblem::SpecialFile),
        ] {
            let root = workspace(&["crates/kernel/Cargo.toml", "crates/kernel/src/lib.rs"]);
            let offender = make(root.path());
            let error = read_all(root.path()).unwrap_err();
            assert!(
                matches!(&error, XtaskError::Layout { path, problem: found }
                    if path == Path::new(offender) && *found == problem),
                "{offender}: {error}"
            );
        }
    }

    #[cfg(unix)]
    #[test]
    fn a_crate_folder_or_its_src_that_is_a_symlink_is_refused() {
        use std::os::unix::fs::symlink;
        let elsewhere = workspace(&["Cargo.toml", "src/lib.rs"]);
        let root = workspace(&["crates/kernel/Cargo.toml"]);
        fs::create_dir_all(root.path().join("crates/modules")).unwrap();
        symlink(elsewhere.path(), root.path().join("crates/modules/goals")).unwrap();
        let error = measured_crates(root.path()).unwrap_err();
        assert!(
            matches!(&error, XtaskError::Layout { path, problem: LayoutProblem::Symlink }
                if path == Path::new("crates/modules/goals")),
            "{error}"
        );
        let root = workspace(&["crates/kernel/Cargo.toml"]);
        symlink(
            elsewhere.path().join("src"),
            root.path().join("crates/kernel/src"),
        )
        .unwrap();
        let error = read_all(root.path()).unwrap_err();
        assert!(
            matches!(&error, XtaskError::Layout { path, problem: LayoutProblem::Symlink }
                if path == Path::new("crates/kernel/src")),
            "{error}"
        );
    }
}
