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
    #[error("the coverage report is malformed at line {line}: {problem}")]
    Lcov { line: usize, problem: &'static str },
    #[error(
        "the coverage report is older than {}: it measures line {line}, which the file does not have",
        escaped(path)
    )]
    StaleReport { path: PathBuf, line: usize },
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
            XtaskError::StaleReport {
                path: path.clone(),
                line: 2,
            },
        ] {
            let message = error.to_string();
            assert!(!message.contains('\u{1b}'), "{message}");
            assert!(message.contains("\\u{1b}[31mgone.rs"), "{message}");
        }
    }
}
