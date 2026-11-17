//! The local database rebuilt from nothing: dropped, created, migrated and seeded (spec §9.2).

use std::fmt;
use std::path::Path;

use sqlx::postgres::PgConnectOptions;
use sqlx::{AssertSqlSafe, Connection, PgConnection};

use crate::seed::{apply, read_seed_files};
use crate::{
    AppliedMigration, AppliedSeed, DatabaseUnavailable, DatabaseUrl, MigrateError, NotLocal,
    SeedError, ensure_local, migrate,
};

/// Rebuilds the database `url` names, on this machine only, and returns what each step did.
///
/// The guard comes first ([`ensure_local`]). Then the seed files of `seeds` are read, so that
/// one that cannot be read leaves the database as it was. Then, from the server's maintenance
/// database, the database is dropped, every other session on it ended, and created again,
/// empty. Then every migration this binary carries is applied ([`migrate`]), and the seed files
/// after them, in one transaction, as [`seed`](crate::seed) applies them.
pub async fn reset(url: &DatabaseUrl, seeds: &Path) -> Result<Reset, ResetError> {
    ensure_local(url).map_err(ResetError::NotLocal)?;
    let files = read_seed_files(seeds).map_err(ResetError::SeedsUnreadable)?;
    let options = url.connect_options();
    // A URL of the rule names its database; without one, Postgres takes the user's name.
    let database = options
        .get_database()
        .unwrap_or(options.get_username())
        .to_owned();
    let dropped = recreate(&options, &database).await?;
    let migrations = migrate(url).await.map_err(ResetError::Migrate)?;
    let seeds = apply(url, &files).await.map_err(ResetError::Seed)?;
    Ok(Reset {
        database,
        dropped,
        migrations,
        seeds,
    })
}

/// Drops `database` when it exists, ending every other session on it, and creates it again,
/// empty, on a connection of its own to the server's maintenance database, which is closed
/// afterwards whatever happened. Returns whether `database` existed.
async fn recreate(options: &PgConnectOptions, database: &str) -> Result<bool, ResetError> {
    // As createdb does: `postgres`, unless that is the database rebuilt.
    let maintenance = if database == "postgres" {
        "template1"
    } else {
        "postgres"
    };
    let mut connection = PgConnection::connect_with(&options.clone().database(maintenance))
        .await
        .map_err(|error| ResetError::Unavailable(DatabaseUnavailable::from(&error)))?;
    let outcome = drop_and_create(&mut connection, database).await;
    if let Err(error) = connection.close().await {
        // A connection that cannot close cleanly is dropped instead, which closes its socket:
        // the session ends either way, so the outcome stands.
        tracing::warn!(
            error = %DatabaseUnavailable::from(&error),
            "the connection that rebuilt the database did not close cleanly"
        );
    }
    outcome
}

async fn drop_and_create(
    connection: &mut PgConnection,
    database: &str,
) -> Result<bool, ResetError> {
    let cause = |error: sqlx::Error| DatabaseUnavailable::from(&error);
    let refused_drop = |error| ResetError::Drop {
        database: database.to_owned(),
        cause: cause(error),
    };
    let existed = sqlx::query_scalar!(
        r#"SELECT EXISTS (SELECT FROM pg_database WHERE datname = $1::text) AS "existed!""#,
        database
    )
    .fetch_one(&mut *connection)
    .await
    .map_err(refused_drop)?;
    let name = quoted(database);
    if existed {
        // Alone, `DROP DATABASE` refuses while another session is connected, such as a server
        // under `just dev`; `FORCE` ends those sessions first. `IF EXISTS`, since a session
        // elsewhere may drop it after the question.
        sqlx::raw_sql(AssertSqlSafe(format!(
            "DROP DATABASE IF EXISTS {name} WITH (FORCE)"
        )))
        .execute(&mut *connection)
        .await
        .map_err(refused_drop)?;
    }
    sqlx::raw_sql(AssertSqlSafe(format!("CREATE DATABASE {name}")))
        .execute(&mut *connection)
        .await
        .map_err(|error| ResetError::Create {
            database: database.to_owned(),
            cause: cause(error),
        })?;
    Ok(existed)
}

/// `name` as one SQL identifier, whatever it holds: inside double quotes, with each of its own
/// doubled. The name comes from the URL, and these statements take no parameter for it.
fn quoted(name: &str) -> String {
    format!("\"{}\"", name.replace('"', "\"\""))
}

/// What a reset did, step by step, for the caller to tell.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Reset {
    database: String,
    dropped: bool,
    migrations: Vec<AppliedMigration>,
    seeds: Vec<AppliedSeed>,
}

impl Reset {
    /// The database rebuilt, by the name the URL gives it.
    pub fn database(&self) -> &str {
        &self.database
    }

    /// Whether it existed, and so was dropped, its other sessions ended, before it was created.
    pub fn dropped(&self) -> bool {
        self.dropped
    }

    /// The migrations applied to it: every one this binary carries.
    pub fn migrations(&self) -> &[AppliedMigration] {
        &self.migrations
    }

    /// The seed files applied to it, in the order they ran.
    pub fn seeds(&self) -> &[AppliedSeed] {
        &self.seeds
    }
}

/// Why a reset stopped, and what state it left the database in.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ResetError {
    /// The database may be on another machine: nothing was read or touched.
    NotLocal(NotLocal),
    /// The seed files could not be read: the database was left as it was.
    SeedsUnreadable(SeedError),
    /// The server's maintenance database could not be used: the database was left as it was.
    Unavailable(DatabaseUnavailable),
    /// The database could not be dropped: it was left as it was.
    Drop {
        database: String,
        cause: DatabaseUnavailable,
    },
    /// The database was dropped and could not be created again: it no longer exists.
    Create {
        database: String,
        cause: DatabaseUnavailable,
    },
    /// The database was created again, and the migrations failed as the error says.
    Migrate(MigrateError),
    /// The database was created again and migrated, and the seeds failed as the error says.
    Seed(SeedError),
}

impl fmt::Display for ResetError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::NotLocal(reason) => write!(f, "reset refused: {reason}"),
            Self::SeedsUnreadable(error) => {
                write!(f, "{error}; the database was left as it was")
            }
            Self::Unavailable(cause) => write!(
                f,
                "cannot connect to the server's maintenance database: {cause}; the database was \
                 left as it was"
            ),
            Self::Drop { database, cause } => {
                write!(f, "cannot drop the database {database}: {cause}")
            }
            Self::Create { database, cause } => write!(
                f,
                "dropped the database {database}, and cannot create it again: {cause}"
            ),
            Self::Migrate(error) => write!(f, "the database was created again, empty; {error}"),
            Self::Seed(error) => write!(f, "the database was created again and migrated; {error}"),
        }
    }
}

impl std::error::Error for ResetError {}

#[cfg(test)]
mod tests {
    use std::io;

    use super::*;

    #[test]
    fn a_name_is_quoted_as_one_identifier_whatever_it_holds() {
        for (name, identifier) in [
            ("barras", r#""barras""#),
            ("Barras Dev", r#""Barras Dev""#),
            (r#"we"ird"#, r#""we""ird""#),
            (
                r#"x" WITH (FORCE); DROP DATABASE postgres; --"#,
                r#""x"" WITH (FORCE); DROP DATABASE postgres; --""#,
            ),
        ] {
            assert_eq!(quoted(name), identifier, "{name}");
        }
    }

    #[tokio::test]
    async fn a_database_that_may_be_on_another_machine_is_refused_before_the_seeds_are_read() {
        let url = DatabaseUrl::parse("postgres://barras:hunter2-secret@db.example.com:5432/barras")
            .unwrap();
        // Read, this folder would fail the reset with another error: it is a file.
        let seeds = Path::new(env!("CARGO_MANIFEST_DIR")).join("Cargo.toml");

        let refused = reset(&url, &seeds).await;

        assert_eq!(refused, Err(ResetError::NotLocal(NotLocal::OtherName)));
    }

    #[test]
    fn each_error_is_a_sentence_that_says_what_state_the_database_is_in() {
        let refused = |sqlstate: &str| DatabaseUnavailable::Refused {
            sqlstate: Some(sqlstate.to_owned()),
        };
        for (error, sentence) in [
            (
                ResetError::NotLocal(NotLocal::OtherAddress),
                "reset refused: the database may be on another machine: its host is an address \
                 outside 127.0.0.0/8 and ::1",
            ),
            (
                ResetError::SeedsUnreadable(SeedError::File {
                    file: "20261116000001_demo.sql".to_owned(),
                    kind: io::ErrorKind::InvalidData,
                }),
                "cannot read seed 20261116000001_demo.sql: invalid data; the database was left \
                 as it was",
            ),
            (
                ResetError::Unavailable(DatabaseUnavailable::ConnectionFailed(
                    io::ErrorKind::ConnectionRefused,
                )),
                "cannot connect to the server's maintenance database: the connection failed: \
                 connection refused; the database was left as it was",
            ),
            (
                ResetError::Drop {
                    database: "barras".to_owned(),
                    cause: refused("42501"),
                },
                "cannot drop the database barras: the server refused it with SQLSTATE 42501",
            ),
            (
                ResetError::Create {
                    database: "barras".to_owned(),
                    cause: refused("42501"),
                },
                "dropped the database barras, and cannot create it again: the server refused it \
                 with SQLSTATE 42501",
            ),
            (
                ResetError::Migrate(MigrateError::Unavailable(refused("53300"))),
                "the database was created again, empty; cannot connect to the database: the \
                 server refused it with SQLSTATE 53300",
            ),
            (
                ResetError::Seed(SeedError::EndedTransaction {
                    file: "20261116000001_demo.sql".to_owned(),
                }),
                "the database was created again and migrated; seed 20261116000001_demo.sql ended \
                 the run's transaction itself, so what it and the seeds before it did may have \
                 stayed: take its COMMIT or ROLLBACK out, then run just db-reset",
            ),
        ] {
            assert_eq!(error.to_string(), sentence);
        }
    }
}
