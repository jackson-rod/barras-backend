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
/// migration as pending again. sqlx-cli takes the same name from the repository's `sqlx.toml`,
/// and a test fails when the two differ.
const HISTORY: &str = "public._sqlx_migrations";

/// Applies the pending migrations this binary carries, in order, each in its own transaction
/// unless its file starts with sqlx's `-- no-transaction`, and returns them.
///
/// It refuses, before applying anything, when the history marks a migration as failed, or holds
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
    let connection = PgConnection::connect_with(&url.connect_options())
        .await
        .map_err(|error| MigrateError::Unavailable(DatabaseUnavailable::from(&error)))?;
    run(connection, migrations).await
}

/// [`migrate`] on `connection`, as `reset` migrates the database it has just created, on a
/// connection whose server has shown that it is the local one.
pub(crate) async fn migrate_on(
    connection: PgConnection,
) -> Result<Vec<AppliedMigration>, MigrateError> {
    run(connection, &MIGRATIONS).await
}

/// The run of [`migrate`] on `connection`, which is closed afterwards whatever happened.
async fn run(
    mut connection: PgConnection,
    migrations: &Migrator,
) -> Result<Vec<AppliedMigration>, MigrateError> {
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

/// A failure of `migration`, after the ones in `applied`: the server's answer as
/// [`server_failure`] keeps it, or a fixed sentence when the server gave none.
fn migration_failure(
    applied: Vec<AppliedMigration>,
    migration: &Migration,
    error: &sqlx::migrate::MigrateError,
) -> MigrateError {
    let failure = match error {
        sqlx::migrate::MigrateError::ExecuteMigration(cause, _)
        | sqlx::migrate::MigrateError::Execute(cause) => match cause {
            sqlx::Error::Database(refusal) => {
                server_failure(sqlstate_of(refusal.as_ref()), refusal.message())
            }
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

/// What a migration's failure keeps of the server's answer: its SQLSTATE, and its message only
/// for the classes about the SQL itself, 42 and 0A. Any other message can quote the data of a
/// table, as a value that does not convert does, and migrations run against production (plan,
/// refinement 11).
fn server_failure(sqlstate: Option<String>, message: &str) -> MigrationFailure {
    match sqlstate {
        Some(sqlstate) if sqlstate.starts_with("42") || sqlstate.starts_with("0A") => {
            MigrationFailure::Sql {
                sqlstate,
                message: message.to_owned(),
            }
        }
        sqlstate => MigrationFailure::Server { sqlstate },
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
        Self::named(migration.version, &migration.description.replace(' ', "_"))
    }

    /// The migration of `version` from the file `<version>_<description>.sql`.
    fn named(version: i64, description: &str) -> Self {
        Self {
            version,
            file: format!("{version}_{description}.sql"),
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

#[cfg(any(test, feature = "testing"))]
impl AppliedMigration {
    /// The migration of `version` from the file `<version>_<description>.sql`, as a run reports
    /// it: for the tests of a crate that tells what a run applied, which have no run to get one
    /// from.
    pub fn new(version: i64, description: &str) -> Self {
        Self::named(version, description)
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
    /// The history marks a migration as failed (`success = false`), which sqlx 0.9 never writes
    /// on Postgres, though a person or another tool can: nothing was applied.
    Failed { version: i64 },
    /// A migration already applied differs from its file, which was edited after it ran: nothing
    /// was applied.
    Edited { version: i64 },
    /// A migration already applied is not among the binary's: nothing was applied.
    Missing { version: i64 },
    /// A migration failed. One that runs inside its transaction, as every migration does unless
    /// its file starts with `-- no-transaction`, leaves nothing behind when its SQL fails. A
    /// `-- no-transaction` migration can leave part of its work, such as the invalid index of a
    /// `CREATE INDEX CONCURRENTLY` that failed, which the next run meets again: sqlx records a
    /// migration only once its SQL has succeeded. The ones before it in the run stay applied,
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
    /// The server refused the SQL itself, with a SQLSTATE of class 42 (a syntax error or an
    /// access rule broken) or 0A (a feature it does not support): its SQLSTATE, and its message,
    /// which is about the SQL.
    Sql { sqlstate: String, message: String },
    /// The server failed it in any other way, as when the data of a table breaks a constraint or
    /// does not convert: its SQLSTATE alone, when it gave one. Such a message can quote that
    /// data, and migrations run against production (plan, refinement 11).
    Server { sqlstate: Option<String> },
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
                MigrationFailure::Sql { sqlstate, message } => write!(
                    f,
                    "migration {file} failed with SQLSTATE {sqlstate}: {message}"
                ),
                MigrationFailure::Server {
                    sqlstate: Some(sqlstate),
                } => write!(f, "migration {file} failed with SQLSTATE {sqlstate}"),
                MigrationFailure::Server { sqlstate: None } => {
                    write!(f, "migration {file} failed: the server refused it")
                }
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

    /// The `table-name` lines of the `[migrate]` table of a `sqlx.toml`, each value without its
    /// quotes. The file holds plain `key = "value"` lines, so reading it line by line is enough.
    fn history_tables_named_in(sqlx_toml: &str) -> Vec<&str> {
        let mut table = "";
        let mut names = Vec::new();
        for line in sqlx_toml.lines().map(str::trim) {
            if let Some(header) = line.strip_prefix('[') {
                table = header.split(']').next().unwrap_or_default().trim();
            } else if let Some((key, value)) = line.split_once('=')
                && table == "migrate"
                && key.trim() == "table-name"
            {
                let quoted = value
                    .trim()
                    .strip_prefix('"')
                    .and_then(|rest| rest.split_once('"'));
                names.push(quoted.map_or(value.trim(), |(name, _)| name));
            }
        }
        names
    }

    #[test]
    fn sqlx_cli_names_the_history_table_as_the_binary_does() {
        // sqlx-cli reads the name from the repository's `sqlx.toml`; were the two to differ,
        // `sqlx migrate info` would read one history and `barras-api migrate` write another.
        let sqlx_toml = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../../sqlx.toml");
        let sqlx_toml = fs::read_to_string(&sqlx_toml)
            .unwrap_or_else(|error| panic!("cannot read {}: {error}", sqlx_toml.display()));

        assert_eq!(history_tables_named_in(&sqlx_toml), [HISTORY]);
    }

    #[test]
    fn an_applied_migration_is_named_by_its_file() {
        let applied = AppliedMigration::of(&migration(20261116120000, "SELECT 1"));

        assert_eq!(applied.version(), 20261116120000);
        assert_eq!(applied.file(), "20261116120000_probe_20261116120000.sql");
    }

    #[test]
    fn a_migration_made_for_a_test_is_the_one_a_run_reports() {
        let run = AppliedMigration::of(&migration(20261116120000, "SELECT 1"));

        let made = AppliedMigration::new(20261116120000, "probe_20261116120000");

        assert_eq!(made, run);
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
                    failure: MigrationFailure::Sql {
                        sqlstate: "42601".to_owned(),
                        message: "syntax error at end of input".to_owned(),
                    },
                },
                "migration 20_probe_20.sql failed with SQLSTATE 42601: syntax error at end of \
                 input",
            ),
            (
                MigrateError::Migration {
                    applied: vec![first.clone()],
                    file: "20_probe_20.sql".to_owned(),
                    failure: MigrationFailure::Server {
                        sqlstate: Some("22P02".to_owned()),
                    },
                },
                "migration 20_probe_20.sql failed with SQLSTATE 22P02",
            ),
            (
                MigrateError::Migration {
                    applied: Vec::new(),
                    file: "20_probe_20.sql".to_owned(),
                    failure: MigrationFailure::Server { sqlstate: None },
                },
                "migration 20_probe_20.sql failed: the server refused it",
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

    #[test]
    fn only_a_refusal_of_the_sql_itself_keeps_the_server_message() {
        // What a value that does not convert answers: the message quotes the table's data.
        let message = "invalid input syntax for type integer: \"hunter2-secret\"";
        for sqlstate in ["42601", "42P01", "42501", "0A000"] {
            assert_eq!(
                server_failure(Some(sqlstate.to_owned()), message),
                MigrationFailure::Sql {
                    sqlstate: sqlstate.to_owned(),
                    message: message.to_owned(),
                }
            );
        }
        for sqlstate in [
            "22P02", "23505", "23514", "57014", "40P01", "XX000", "0B000", "04200",
        ] {
            assert_eq!(
                server_failure(Some(sqlstate.to_owned()), message),
                MigrationFailure::Server {
                    sqlstate: Some(sqlstate.to_owned())
                }
            );
        }
        assert_eq!(
            server_failure(None, message),
            MigrationFailure::Server { sqlstate: None }
        );
    }
}
