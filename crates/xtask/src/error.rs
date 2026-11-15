//! Errors shared by the xtask checks.

use thiserror::Error;

#[derive(Debug, Error)]
pub enum XtaskError {
    #[error("io error: {0}")]
    Io(#[from] std::io::Error),
    #[error("cargo metadata failed: {0}")]
    Metadata(String),
    #[error("git failed: {0}")]
    Git(String),
}
