//! Errors shared by the xtask checks.

use std::path::{Path, PathBuf};

use thiserror::Error;

#[derive(Debug, Error)]
pub enum XtaskError {
    #[error("could not read {}: {source}", path.display())]
    Io {
        path: PathBuf,
        source: std::io::Error,
    },
    #[error("cargo metadata failed: {0}")]
    Metadata(String),
    #[error("invalid pattern {pattern:?}: {message}")]
    Pattern { pattern: String, message: String },
    #[error("the coverage report is malformed at line {line}: {problem}")]
    Lcov { line: usize, problem: &'static str },
    #[error(
        "the coverage report is older than {}: it measures line {line}, which the file does not have",
        path.display()
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
