//! The migrations of `migrations/`, embedded in the crate, and the run that applies them.

use std::fmt;

use sqlx::migrate::{Migrate, Migration, Migrator};
use sqlx::{Connection, PgConnection};

use crate::unavailable::sqlstate_of;
use crate::{DatabaseUnavailable, DatabaseUrl};

/// The migrations of the repository's `migrations/` folder as the crate was built with them: a
/// new or edited file rebuilds the crate (`build.rs`), so a binary carries the migrations it was
/// built with, and production needs no `migrations/` folder.
static MIGRATIONS: Migrator = sqlx::migrate!("../../../migrations");

/// Where sqlx records the migrations it applied: `public`, its default (spec §7), named
/// outright. sqlx names the table alone, and Postgres looks for it in the schema named like the
/// user before `public`, so once the first migration has created `barras`, a user named
/// `barras`, as the local database's is, would get a new, empty history there and see every
/// migration as pending again.
const HISTORY: &str = "public._sqlx_migrations";

/// Applies the pending migrations this binary carries, in order, each in its own transaction
/// unless its file starts with sqlx's `-- no-transaction`, and returns them.
///
/// It refuses, before applying anything, when the history records a migration that failed, or
/// one already applied that differs from its file (edited after it ran) or that the binary does
/// not carry. sqlx's own run checks an applied migration only when its turn comes, after
/// applying every older pending one (plan, refinement 1).
///
/// It runs under sqlx's advisory lock, so that two deploys never interleave, on a connection of
/// its own without the pools' statement timeout, since a migration may build an index for
/// longer. The connection is closed after the run whatever happened: the lock lasts as long as
/// the session that took it.
pub async fn migrate(url: &DatabaseUrl) -> Result<Vec<AppliedMigration>, MigrateError> {
    migrate_with(url, &MIGRATIONS).await
}

/// [`migrate`] with `migrations` instead of the embedded ones, for the tests that need
/// migrations of their own.
pub async fn migrate_with(
    url: &DatabaseUrl,
    migrations: &Migrator,
) -> Result<Vec<AppliedMigration>, MigrateError> {
    let mut connection = PgConnection::connect_with(&url.connect_options())
        .await
        .map_err(|error| MigrateError::Unavailable(DatabaseUnavailable::from(&error)))?;
    let outcome = apply_pending(&mut connection, migrations).await;
    if let Err(error) = connection.close().await {
        // A connection that cannot close cleanly is dropped instead, which closes its socket:
        // the session, and the lock with it, ends either way, so the outcome stands.
        tracing::warn!(
            error = %DatabaseUnavailable::from(&error),
            "the connection that migrated did not close cleanly"
        );
    }
    outcome
}

/// The lock, then the whole history checked against `migrations`, and only then the pending
/// ones applied, in order. The lock is left to the end of the session, which follows at once.
async fn apply_pending(
    connection: &mut PgConnection,
    migrations: &Migrator,
) -> Result<Vec<AppliedMigration>, MigrateError> {
    connection.lock().await.map_err(history_failure)?;
    connection
        .ensure_migrations_table(HISTORY)
        .await
        .map_err(history_failure)?;
    if let Some(version) = connection
        .dirty_version(HISTORY)
        .await
        .map_err(history_failure)?
    {
        return Err(MigrateError::Failed { version });
    }
    let history = connection
        .list_applied_migrations(HISTORY)
        .await
        .map_err(history_failure)?;
    refuse_changed(&history, migrations)?;
    let mut applied = Vec::new();
    for migration in up_migrations(migrations) {
        if history.iter().any(|row| row.version == migration.version) {
            continue;
        }
        if let Err(error) = connection.apply(HISTORY, migration).await {
            return Err(migration_failure(applied, migration, &error));
        }
        applied.push(AppliedMigration::of(migration));
    }
    Ok(applied)
}

/// The migrations that move the schema forward: a migration has no script that undoes it (spec
/// §3.3), and `check-structure` refuses the names of one.
fn up_migrations(migrations: &Migrator) -> impl Iterator<Item = &Migration> {
    migrations
        .iter()
        .filter(|migration| !migration.migration_type.is_down_migration())
}

/// The first migration of `history` that was edited after it ran, or that `migrations` lacks.
fn refuse_changed(
    history: &[sqlx::migrate::AppliedMigration],
    migrations: &Migrator,
) -> Result<(), MigrateError> {
    for row in history {
        match up_migrations(migrations).find(|migration| migration.version == row.version) {
            None => {
                return Err(MigrateError::Missing {
                    version: row.version,
                });
            }
            Some(migration) if migration.checksum != row.checksum => {
                return Err(MigrateError::Edited {
                    version: row.version,
                });
            }
            Some(_) => {}
        }
    }
    Ok(())
}

/// A failure of the lock or of the history table, before any migration ran.
fn history_failure(error: sqlx::migrate::MigrateError) -> MigrateError {
    MigrateError::History(match &error {
        sqlx::migrate::MigrateError::Execute(cause) => DatabaseUnavailable::from(cause),
        _ => DatabaseUnavailable::DriverFailed,
    })
}

/// A failure of `migration`, after the ones in `applied`. A refusal of its SQL keeps the
/// server's message, which is about the SQL; any other failure is a fixed sentence.
fn migration_failure(
    applied: Vec<AppliedMigration>,
    migration: &Migration,
    error: &sqlx::migrate::MigrateError,
) -> MigrateError {
    let failure = match error {
        sqlx::migrate::MigrateError::ExecuteMigration(cause, _)
        | sqlx::migrate::MigrateError::Execute(cause) => match cause {
            sqlx::Error::Database(refusal) => MigrationFailure::Server {
                sqlstate: sqlstate_of(refusal.as_ref()),
                message: refusal.message().to_owned(),
            },
            other => MigrationFailure::Unavailable(DatabaseUnavailable::from(other)),
        },
        _ => MigrationFailure::Unavailable(DatabaseUnavailable::DriverFailed),
    };
    MigrateError::Migration {
        applied,
        file: AppliedMigration::of(migration).file,
        failure,
    }
}

/// A migration that a run applied.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AppliedMigration {
    version: i64,
    file: String,
}

impl AppliedMigration {
    fn of(migration: &Migration) -> Self {
        // sqlx reads the description from the file's name, each `_` turned into a space.
        let description = migration.description.replace(' ', "_");
        Self {
            version: migration.version,
            file: format!("{}_{description}.sql", migration.version),
        }
    }

    /// Its version, the timestamp its file's name starts with.
    pub fn version(&self) -> i64 {
        self.version
    }

    /// The name of its file in `migrations/`.
    pub fn file(&self) -> &str {
        &self.file
    }
}

/// Why a migration run stopped.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum MigrateError {
    /// No connection: nothing ran.
    Unavailable(DatabaseUnavailable),
    /// sqlx's history table, `public._sqlx_migrations`, or its lock, could not be read or
    /// written: nothing was applied.
    History(DatabaseUnavailable),
    /// The history records a migration that failed when it last ran: nothing was applied.
    Failed { version: i64 },
    /// A migration already applied differs from its file, which was edited after it ran: nothing
    /// was applied.
    Edited { version: i64 },
    /// A migration already applied is not among the binary's: nothing was applied.
    Missing { version: i64 },
    /// A migration failed, and nothing of it stays. The ones before it in the run stay applied,
    /// and the ones after it did not run.
    Migration {
        applied: Vec<AppliedMigration>,
        file: String,
        failure: MigrationFailure,
    },
}

/// What stopped a migration.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum MigrationFailure {
    /// The server refused its SQL: its SQLSTATE, and its message, which is about the SQL.
    Server {
        sqlstate: Option<String>,
        message: String,
    },
    /// The database stopped answering.
    Unavailable(DatabaseUnavailable),
}

impl fmt::Display for MigrateError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Unavailable(cause) => write!(f, "cannot connect to the database: {cause}"),
            Self::History(cause) => write!(
                f,
                "cannot read or write the migration history, {HISTORY}: {cause}"
            ),
            Self::Failed { version } => write!(
                f,
                "migration {version} is recorded as failed in {HISTORY}: nothing was applied"
            ),
            Self::Edited { version } => write!(
                f,
                "migration {version} was applied, and its file has changed since: nothing was \
                 applied"
            ),
            Self::Missing { version } => write!(
                f,
                "migration {version} was applied, and this binary does not carry it: nothing was \
                 applied"
            ),
            Self::Migration { file, failure, .. } => match failure {
                MigrationFailure::Server {
                    sqlstate: Some(sqlstate),
                    message,
                } => write!(
                    f,
                    "migration {file} failed with SQLSTATE {sqlstate}: {message}"
                ),
                MigrationFailure::Server {
                    sqlstate: None,
                    message,
                } => write!(f, "migration {file} failed: {message}"),
                MigrationFailure::Unavailable(cause) => {
                    write!(f, "migration {file} failed: {cause}")
                }
            },
        }
    }
}

impl std::error::Error for MigrateError {}

#[cfg(test)]
mod tests {
    use std::borrow::Cow;
    use std::collections::BTreeSet;
    use std::fs;
    use std::path::Path;

    use sqlx::SqlSafeStr;
    use sqlx::migrate::{Migration, MigrationType};

    use super::*;

    fn migration(version: i64, sql: &'static str) -> Migration {
        Migration::new(
            version,
            Cow::Owned(format!("probe {version}")),
            MigrationType::Simple,
            sql.into_sql_str(),
            false,
        )
    }

    #[test]
    fn every_file_of_the_migrations_folder_is_embedded() {
        let folder = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../../migrations");
        let on_disk: BTreeSet<String> = fs::read_dir(folder)
            .unwrap()
            .map(|entry| entry.unwrap().file_name().into_string().unwrap())
            .filter(|name| !name.starts_with('.'))
            .collect();

        let embedded: BTreeSet<String> = MIGRATIONS
            .iter()
            .map(|migration| AppliedMigration::of(migration).file)
            .collect();

        assert!(
            embedded.contains("20261117024143_create_schema_barras.sql"),
            "{embedded:?}"
        );
        assert_eq!(embedded, on_disk);
    }

    #[test]
    fn an_applied_migration_is_named_by_its_file() {
        let applied = AppliedMigration::of(&migration(20261116120000, "SELECT 1"));

        assert_eq!(applied.version(), 20261116120000);
        assert_eq!(applied.file(), "20261116120000_probe_20261116120000.sql");
    }

    #[test]
    fn a_history_that_matches_the_migrations_is_accepted() {
        let migrations = Migrator::with_migrations(vec![
            migration(10, "SELECT 10"),
            migration(20, "SELECT 20"),
            migration(30, "SELECT 30"),
        ]);
        let history = [applied_as(&migration(10, "SELECT 10"))];

        assert_eq!(refuse_changed(&history, &migrations), Ok(()));
    }

    #[test]
    fn a_history_with_an_edited_migration_is_refused() {
        let migrations =
            Migrator::with_migrations(vec![migration(10, "SELECT 10"), migration(20, "SELECT 2")]);
        let history = [
            applied_as(&migration(10, "SELECT 10")),
            applied_as(&migration(20, "SELECT 20")),
        ];

        assert_eq!(
            refuse_changed(&history, &migrations),
            Err(MigrateError::Edited { version: 20 })
        );
    }

    #[test]
    fn a_history_with_a_migration_the_binary_lacks_is_refused() {
        let migrations = Migrator::with_migrations(vec![migration(10, "SELECT 10")]);
        let history = [
            applied_as(&migration(10, "SELECT 10")),
            applied_as(&migration(15, "SELECT 15")),
        ];

        assert_eq!(
            refuse_changed(&history, &migrations),
            Err(MigrateError::Missing { version: 15 })
        );
    }

    fn applied_as(migration: &Migration) -> sqlx::migrate::AppliedMigration {
        sqlx::migrate::AppliedMigration {
            version: migration.version,
            checksum: migration.checksum.clone(),
        }
    }

    #[test]
    fn each_error_is_a_sentence_that_names_the_migration() {
        let first = AppliedMigration::of(&migration(10, "SELECT 10"));
        for (error, sentence) in [
            (
                MigrateError::Unavailable(DatabaseUnavailable::Refused {
                    sqlstate: Some("28P01".to_owned()),
                }),
                "cannot connect to the database: the server refused it with SQLSTATE 28P01",
            ),
            (
                MigrateError::History(DatabaseUnavailable::Refused {
                    sqlstate: Some("42501".to_owned()),
                }),
                "cannot read or write the migration history, public._sqlx_migrations: the \
                 server refused it with SQLSTATE 42501",
            ),
            (
                MigrateError::Failed { version: 20 },
                "migration 20 is recorded as failed in public._sqlx_migrations: nothing was \
                 applied",
            ),
            (
                MigrateError::Edited { version: 20 },
                "migration 20 was applied, and its file has changed since: nothing was applied",
            ),
            (
                MigrateError::Missing { version: 20 },
                "migration 20 was applied, and this binary does not carry it: nothing was \
                 applied",
            ),
            (
                MigrateError::Migration {
                    applied: vec![first.clone()],
                    file: "20_probe_20.sql".to_owned(),
                    failure: MigrationFailure::Server {
                        sqlstate: Some("42601".to_owned()),
                        message: "syntax error at end of input".to_owned(),
                    },
                },
                "migration 20_probe_20.sql failed with SQLSTATE 42601: syntax error at end of \
                 input",
            ),
            (
                MigrateError::Migration {
                    applied: Vec::new(),
                    file: "20_probe_20.sql".to_owned(),
                    failure: MigrationFailure::Server {
                        sqlstate: None,
                        message: "it went wrong".to_owned(),
                    },
                },
                "migration 20_probe_20.sql failed: it went wrong",
            ),
            (
                MigrateError::Migration {
                    applied: vec![first],
                    file: "20_probe_20.sql".to_owned(),
                    failure: MigrationFailure::Unavailable(DatabaseUnavailable::ConnectionFailed(
                        std::io::ErrorKind::ConnectionReset,
                    )),
                },
                "migration 20_probe_20.sql failed: the connection failed: connection reset",
            ),
        ] {
            assert_eq!(error.to_string(), sentence);
        }
    }
}
