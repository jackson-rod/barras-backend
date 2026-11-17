//! The PostgreSQL adapter: the one way the workspace reaches Postgres (ADR-0007). It holds the
//! rule a database URL follows ([`DatabaseUrl`]); the primary and read pools with their
//! readiness checks ([`Pools`]), which the composition root wraps in its health probes; and the
//! migrations of `migrations/`, embedded at compile time, with the run that applies them
//! ([`migrate`]). The repositories that implement the modules' persistence ports join it with
//! the first table.
//!
//! Its SQL is checked at compile time against the metadata committed in `.sqlx/`, which `just
//! db-prepare` rewrites and `just db-check` holds to the queries and the migrations.
//!
//! It takes credentials as plain values from the composition root and never logs, displays or
//! returns them: no type of it that holds sqlx's connect options has `Debug`, since theirs
//! prints the password, and every failure it reports before a connection is made is a sentence
//! of a fixed set ([`DatabaseUnavailable`]), never sqlx's text.

mod database_url;
mod guard;
mod migrate;
mod pools;
mod seed;
mod unavailable;

pub use database_url::{DatabaseUrl, UrlProblem};
pub use guard::{NotLocal, ensure_local};
pub use migrate::{AppliedMigration, MigrateError, MigrationFailure, migrate, migrate_with};
pub use pools::Pools;
pub use seed::{AppliedSeed, ErrorPosition, SeedError, SeedFailure, seed};
pub use unavailable::DatabaseUnavailable;

// Taken for its features, not code; named so that cargo never reports it as an unused dependency.
use workspace_hack as _;
