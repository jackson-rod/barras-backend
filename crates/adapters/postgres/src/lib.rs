//! The PostgreSQL adapter: the one way the workspace reaches Postgres (ADR-0007). It holds the
//! rule a database URL follows ([`DatabaseUrl`]) and the migrations of `migrations/`, embedded
//! at compile time, with the run that applies them ([`migrate`]); the repositories that
//! implement the modules' persistence ports join it with the first table.
//!
//! It takes credentials as plain values from the composition root and never logs, displays or
//! returns them: no type of it that holds sqlx's connect options has `Debug`, since theirs
//! prints the password, and every failure it reports before a connection is made is a sentence
//! of a fixed set ([`DatabaseUnavailable`]), never sqlx's text.

mod database_url;
mod migrate;
mod unavailable;

pub use database_url::{DatabaseUrl, UrlProblem};
pub use migrate::{AppliedMigration, MigrateError, MigrationFailure, migrate, migrate_with};
pub use unavailable::DatabaseUnavailable;

// Taken for its features, not code; named so that cargo never reports it as an unused dependency.
use workspace_hack as _;
