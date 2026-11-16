//! Errors shared by the xtask checks.

use std::path::{Path, PathBuf};

use thiserror::Error;

#[derive(Debug, Error)]
pub enum XtaskError {
    #[error("could not read {}: {source}", escaped(path))]
    Io {
        path: PathBuf,
        source: std::io::Error,
    },
    #[error(
        "{} is not a regular file: a symlink, a FIFO, a device or a folder is not read",
        escaped(path)
    )]
    NotARegularFile { path: PathBuf },
    #[error("{} holds more than {limit} bytes", escaped(path))]
    TooLarge { path: PathBuf, limit: u64 },
    #[error("cargo metadata failed: {0}")]
    Metadata(String),
    #[error("invalid pattern {pattern:?}: {message}")]
    Pattern { pattern: String, message: String },
    #[error(
        "the coverage report {} is malformed at line {line}: {problem}",
        escaped(report)
    )]
    Lcov {
        report: PathBuf,
        line: usize,
        problem: &'static str,
    },
    #[error(
        "the coverage report measures line {line} of {}, past the end of the file, which has {length} lines",
        escaped(path)
    )]
    StaleReport {
        path: PathBuf,
        line: usize,
        length: usize,
    },
    #[error(
        "the coverage report measures {}, inside the workspace but outside the src/ of every crate and outside target/: code included from there would escape every bar",
        escaped(path)
    )]
    StrayFile { path: PathBuf },
    #[error(
        "the coverage report measures {}, a file under src/ that check-coverage does not read: it reads the .rs files, hidden ones aside, of crates that hold a Cargo.toml",
        escaped(path)
    )]
    NotRead { path: PathBuf },
    #[error("{}: {problem}", escaped(path))]
    Layout {
        path: PathBuf,
        problem: LayoutProblem,
    },
    #[error("{}:{line}: {problem}", escaped(path))]
    Source {
        path: PathBuf,
        line: usize,
        problem: SourceProblem,
    },
    /// An error about what the record at `line` of the report names.
    #[error("{}:{line}: {error}", escaped(report))]
    InReport {
        report: PathBuf,
        line: usize,
        error: Box<XtaskError>,
    },
    #[error(
        "{} holds a function body, and the coverage report {} does not name it: its tests never ran, or cargo-llvm-cov dropped its path",
        escaped(path),
        escaped(report)
    )]
    Unreported { report: PathBuf, path: PathBuf },
    #[error("could not parse on a thread of its own: {0}")]
    ParseThread(std::io::Error),
}

/// A file or a folder under a measured `src/` that would hide code from the measure, or
/// that check-coverage does not read.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Error)]
pub enum LayoutProblem {
    #[error(
        "a file named `*_tests.rs` or `*-tests.rs`, which cargo-llvm-cov leaves out of the report"
    )]
    DroppedFileName,
    #[error(
        "a folder named `examples` or `benches`, which cargo-llvm-cov leaves out of the report"
    )]
    DroppedFolder,
    #[error("a symlink, which check-coverage does not follow")]
    Symlink,
    #[error("a FIFO, a socket or a device, which check-coverage does not read")]
    SpecialFile,
    #[error("a name that is not UTF-8")]
    NotUtf8Name,
}

/// A shape in a measured source that check-coverage refuses, because it would hide code
/// from the measure or count test code as production.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Error)]
pub enum SourceProblem {
    #[error("the file does not parse as Rust")]
    Unparsable,
    #[error(
        "`#[cfg(test)]` stands on something that is not an item, so its lines cannot be left out"
    )]
    CfgTestNotOnItem,
    #[error(
        "a `cfg` predicate names `test` in a form other than `test` or `any(test, feature = \"…\")`"
    )]
    TestInOtherForm,
    #[error("a `cfg` predicate names `coverage`, which could take code out of the measured build")]
    NamesCoverage,
    #[error("a `coverage(…)` attribute, which takes code out of the measure")]
    CoverageAttribute,
    #[error("a `cfg_attr` that applies `cfg`, which could take code out of the measured build")]
    CfgInCfgAttr,
    #[error(
        "an out-of-line `#[cfg(test)]` module must be named `tests`, the file name cargo-llvm-cov leaves out"
    )]
    OutOfLineTestModule,
    #[error(
        "a module named `tests` or `*_tests` without `#[cfg(test)]`: cargo-llvm-cov leaves such files out, so production code in them would escape"
    )]
    UnmarkedTestsModule,
    #[error(
        "an out-of-line module in test code must be named `tests` or sit in a module named `tests`, or its file is measured as production"
    )]
    StrayTestModuleFile,
    #[error("a `#[path]` attribute, which can point a module at any file")]
    PathAttribute,
    #[error(
        "`include` other than in `include!(concat!(env!(\"OUT_DIR\"), \"…\"))`: an included file is measured, or dropped, under its own path"
    )]
    Include,
    #[error("nested deeper than 256 groups, more than check-coverage reads")]
    TooDeep,
    #[error(
        "a `cfg` predicate uses more than `test`, `feature = \"…\"`, `unix`, `windows` and the `target_*` keys, or puts `not` around a feature: code only a release build, a custom cfg or a missing feature compiles is never measured"
    )]
    CfgOutsideAllowlist,
    #[error(
        "a macro names `cfg`, `cfg_attr`, `coverage`, `test`, `include` or `mod`: macros are not expanded, so it could produce any shape check-coverage refuses"
    )]
    MacroCouldHideCode,
    #[error("a test function outside a `#[cfg(test)]` item, whose lines would count as production")]
    TestOutsideTestCode,
}

impl XtaskError {
    /// For `map_err`. There is no `From<io::Error>` on purpose: an I/O failure has to
    /// say which path it was reading, or the message is useless.
    pub fn io(path: &Path) -> impl FnOnce(std::io::Error) -> Self {
        move |source| Self::Io {
            path: path.to_path_buf(),
            source,
        }
    }
}

/// `path` as it is safe to print: a file name chosen by someone else could hold control
/// characters that rewrite the terminal, so every character that is not printable is
/// shown as its escape, and so is `\`, to keep the escapes unambiguous.
pub fn escaped(path: &Path) -> String {
    let mut shown = String::new();
    for character in path.to_string_lossy().chars() {
        let escape: String = character.escape_debug().collect();
        let harmless = escape.chars().count() == 1 || matches!(character, '"' | '\'');
        if harmless {
            shown.push(character);
        } else {
            shown.push_str(&escape);
        }
    }
    shown
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn control_characters_in_a_path_are_printed_escaped() {
        assert_eq!(
            escaped(Path::new("/w/src/\u{1b}]0;owned\u{7}\u{1b}[2J.rs")),
            "/w/src/\\u{1b}]0;owned\\u{7}\\u{1b}[2J.rs"
        );
        assert_eq!(
            escaped(Path::new("a\tb\nc\rd\\e\u{202e}f")),
            "a\\tb\\nc\\rd\\\\e\\u{202e}f"
        );
    }

    #[test]
    fn a_plain_path_is_printed_as_it_is() {
        assert_eq!(
            escaped(Path::new("/work/crates/kernel/src/money.rs")),
            "/work/crates/kernel/src/money.rs"
        );
        assert_eq!(escaped(Path::new("año/\"x\"/'y'")), "año/\"x\"/'y'");
    }

    #[test]
    fn every_path_in_an_error_is_printed_escaped() {
        let path = PathBuf::from("/w/\u{1b}[31mgone.rs");
        for error in [
            XtaskError::io(&path)(std::io::Error::other("gone")),
            XtaskError::NotARegularFile { path: path.clone() },
            XtaskError::TooLarge {
                path: path.clone(),
                limit: 1,
            },
            XtaskError::Lcov {
                report: path.clone(),
                line: 1,
                problem: "a record starts inside another",
            },
            XtaskError::StaleReport {
                path: path.clone(),
                line: 2,
                length: 1,
            },
            XtaskError::StrayFile { path: path.clone() },
            XtaskError::NotRead { path: path.clone() },
            XtaskError::Layout {
                path: path.clone(),
                problem: LayoutProblem::Symlink,
            },
            XtaskError::Source {
                path: path.clone(),
                line: 3,
                problem: SourceProblem::PathAttribute,
            },
            XtaskError::InReport {
                report: path.clone(),
                line: 4,
                error: Box::new(XtaskError::StrayFile { path: path.clone() }),
            },
            XtaskError::Unreported {
                report: path.clone(),
                path: path.clone(),
            },
        ] {
            let message = error.to_string();
            assert!(!message.contains('\u{1b}'), "{message}");
            assert!(message.contains("\\u{1b}[31mgone.rs"), "{message}");
        }
    }
}
