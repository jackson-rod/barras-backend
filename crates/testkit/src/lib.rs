//! Test-only helpers for the workspace's tests; only dev-dependencies take this crate
//! (`testkit-dev-only`). It holds:
//!
//! - containers: [`PostgresContainer`], a throwaway Postgres 18 that one test starts, reads with
//!   `psql` inside it, and that is removed when the test ends, for the `int_` and `e2e_` tests,
//!   and [`LoopbackRelay`], a port of this machine that leads to one, or to two in turn, as a
//!   relay to another machine would;
//! - fixtures: the files of the repository ([`repository_file`]) and the variables of an env
//!   file such as `.env.example` ([`env_file`]);
//! - the process helper ([`process`]): a workspace binary started with a clean environment,
//!   stopped with SIGTERM, and what it printed.
//!
//! It reads no environment variable, apart from the coverage profile the process helper hands
//! on, so a test reaches only the containers it started, never the local database.
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)] // test support: a step that fails must stop the test that took it

mod env_file;
mod postgres;
// The binaries stop on SIGTERM, a Unix signal, and Barras builds and runs only on Unix.
#[cfg(unix)]
pub mod process;
mod repository_file;

pub use env_file::env_file;
pub use postgres::{LoopbackRelay, PostgresContainer};
pub use repository_file::repository_file;
