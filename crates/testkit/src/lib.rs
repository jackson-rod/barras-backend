//! Shared test helpers and fixtures for the workspace tests.
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)] // test support: a step that fails must stop the test that took it

mod env_file;
// The binaries stop on SIGTERM, a Unix signal, and Barras builds and runs only on Unix.
#[cfg(unix)]
mod process;

pub use env_file::env_file;
#[cfg(unix)]
pub use process::{BinaryCommand, RunningBinary};
