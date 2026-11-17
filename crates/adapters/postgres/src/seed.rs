//! The seed runner: the SQL files of a folder, applied in one transaction (spec §9.1).

use std::fmt;
use std::fs;
use std::io;
use std::path::Path;

use sqlx::postgres::{PgDatabaseError, PgErrorPosition};
use sqlx::{AssertSqlSafe, Connection, PgConnection};

use crate::guard::{LocalDatabase, ServerCheckFailed, ensure_local};
use crate::unavailable::sqlstate_of;
use crate::{DatabaseUnavailable, DatabaseUrl, NotLocal};

/// Applies the seed files of `folder` to the database `url` names, on this machine only, and
/// returns them in the order they ran.
///
/// A seed file is a `.sql` file of the folder; the files run in name order, each whole, in one
/// transaction, so that a file that fails leaves nothing of the run behind. Other files are
/// ignored, and a folder that does not exist holds none. Every file is read before the first
/// one runs, and with nothing to apply nothing connects.
///
/// The local-database guard comes first: seeds are local development data, so a database that
/// may be on another machine is refused ([`NotLocal`]) before the folder is read. The run
/// connects to the address the guard judged, on a connection of its own without the pools'
/// statement timeout, and refuses a server that did not start as the local one before the first
/// file runs.
pub async fn seed(url: &DatabaseUrl, folder: &Path) -> Result<Vec<AppliedSeed>, SeedError> {
    let local = ensure_local(url).map_err(SeedError::NotLocal)?;
    let files = read_seed_files(folder)?;
    apply(&local, &files).await
}

/// A seed file, read whole before the run connects.
pub(crate) struct SeedFile {
    name: String,
    sql: String,
}

/// The seed files of `folder` in the order they apply, each read whole: a file that cannot be
/// read stops the run before anything is applied.
pub(crate) fn read_seed_files(folder: &Path) -> Result<Vec<SeedFile>, SeedError> {
    let entries = match fs::read_dir(folder) {
        Ok(entries) => entries,
        Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(Vec::new()),
        Err(error) => return Err(SeedError::Folder(error.kind())),
    };
    let mut names = Vec::new();
    for entry in entries {
        let entry = entry.map_err(|error| SeedError::Folder(error.kind()))?;
        // A name that is not UTF-8 is no seed's, since seeds are named like migrations; and
        // `is_file` follows a symbolic link, as reading the file does.
        if let Ok(name) = entry.file_name().into_string()
            && entry.path().is_file()
        {
            names.push(name);
        }
    }
    seed_files(names)
        .into_iter()
        .map(|name| match fs::read_to_string(folder.join(&name)) {
            Ok(sql) => Ok(SeedFile { name, sql }),
            Err(error) => Err(SeedError::File {
                file: name,
                kind: error.kind(),
            }),
        })
        .collect()
}

/// Of the names of a folder's files, the seed files' in the order they apply: those that end in
/// `.sql`, by name, so by the timestamp they start with. A hidden file is no seed: editors leave
/// them next to the files they edit, and the structure check ignores them too.
fn seed_files(names: Vec<String>) -> Vec<String> {
    let mut seeds: Vec<String> = names
        .into_iter()
        .filter(|name| name.ends_with(".sql") && !name.starts_with('.'))
        .collect();
    seeds.sort_unstable();
    seeds
}

/// Applies `files` in order, in one transaction on a connection of its own to the local server,
/// which is closed afterwards whatever happened: a run that failed ends with its session, and the
/// server rolls back what it left open.
pub(crate) async fn apply(
    local: &LocalDatabase,
    files: &[SeedFile],
) -> Result<Vec<AppliedSeed>, SeedError> {
    if files.is_empty() {
        return Ok(Vec::new());
    }
    let mut connection = local.connect().await.map_err(SeedError::from)?;
    let outcome = apply_in_one_transaction(&mut connection, files).await;
    if let Err(error) = connection.close().await {
        // A connection that cannot close cleanly is dropped instead, which closes its socket:
        // the session ends either way, so the outcome stands.
        tracing::warn!(
            error = %DatabaseUnavailable::from(&error),
            "the connection that seeded did not close cleanly"
        );
    }
    outcome
}

/// The run's transaction, a savepoint before each file, and the commit once every file ran and
/// left the transaction as it found it (plan, refinements 5 and 16).
///
/// The savepoint is always named `barras_seed_file`: the query macros take no name from a
/// variable, and only the latest savepoint of a name counts.
async fn apply_in_one_transaction(
    connection: &mut PgConnection,
    files: &[SeedFile],
) -> Result<Vec<AppliedSeed>, SeedError> {
    let unusable = |error: sqlx::Error| SeedError::Unavailable(DatabaseUnavailable::from(&error));
    let mut transaction = connection.begin().await.map_err(unusable)?;
    // Asked once the transaction is open, the server gives it an id that no later transaction
    // takes: a file that ends it, then opens another or runs outside any, leaves another id.
    let run = transaction_id(&mut transaction).await.map_err(unusable)?;
    let mut applied = Vec::new();
    for file in files {
        sqlx::query!("SAVEPOINT barras_seed_file")
            .execute(&mut *transaction)
            .await
            .map_err(unusable)?;
        let ran = sqlx::raw_sql(AssertSqlSafe(file.sql.as_str()))
            .execute(&mut *transaction)
            .await;
        let verdict = match ran {
            Ok(_) => verdict_after_success(&mut transaction, &run).await,
            Err(error @ sqlx::Error::Database(_)) => {
                let failure = SeedFailure::of(&error, Some(&file.sql));
                verdict_after_failure(&mut transaction, &run, failure).await
            }
            // An error that is not the server's leaves the connection where the run cannot tell
            // its state: after a `COPY … FROM stdin`, the server waits for data that the run never
            // sends, and any question would wait as long. So nothing more is asked, and the run
            // closes the connection (plan, refinement 16).
            Err(error) => Verdict::Lost(DatabaseUnavailable::from(&error)),
        };
        let file = file.name.clone();
        match verdict {
            Verdict::Applied => {
                sqlx::query!("RELEASE SAVEPOINT barras_seed_file")
                    .execute(&mut *transaction)
                    .await
                    .map_err(unusable)?;
                applied.push(AppliedSeed { file });
            }
            Verdict::Refused(failure) => return Err(SeedError::Failed { file, failure }),
            Verdict::Ended => return Err(SeedError::EndedTransaction { file }),
            Verdict::Lost(cause) => {
                return Err(SeedError::Failed {
                    file,
                    failure: SeedFailure::Unavailable(cause),
                });
            }
        }
    }
    transaction
        .commit()
        .await
        .map_err(|error| SeedError::Commit(SeedFailure::of(&error, None)))?;
    Ok(applied)
}

/// What became of the run's transaction once a file ran.
enum Verdict {
    /// The file ran, and left the run's transaction open.
    Applied,
    /// The file failed, and the run's transaction survived it: rolled back with the run, nothing
    /// of it stays.
    Refused(SeedFailure),
    /// The file ended the run's transaction itself, with `COMMIT` or `ROLLBACK`.
    Ended,
    /// The run cannot tell: the connection failed, or was left where no question gets an answer.
    Lost(DatabaseUnavailable),
}

/// After a file that ran: whether it left the run's transaction open.
async fn verdict_after_success(transaction: &mut PgConnection, run: &str) -> Verdict {
    match is_the_run_s(transaction, run).await {
        Ok(true) => Verdict::Applied,
        Ok(false) => Verdict::Ended,
        Err(cause) => Verdict::Lost(cause),
    }
}

/// After a file that failed: whether the run's transaction survived it. A transaction that an
/// error aborted answers nothing but its end or a rollback to a savepoint, so the rollback comes
/// first. In a transaction the file opened, the savepoint does not exist (3B001), and outside any
/// transaction there is none to roll back (25P01): either way the file ended the run's. Where the
/// rollback works, the id still has the last word, since the file may have set a savepoint of the
/// same name in a transaction of its own.
async fn verdict_after_failure(
    transaction: &mut PgConnection,
    run: &str,
    failure: SeedFailure,
) -> Verdict {
    let rolled_back = sqlx::query!("ROLLBACK TO SAVEPOINT barras_seed_file")
        .execute(&mut *transaction)
        .await;
    match rolled_back {
        Ok(_) => match is_the_run_s(transaction, run).await {
            Ok(true) => Verdict::Refused(failure),
            Ok(false) => Verdict::Ended,
            Err(cause) => Verdict::Lost(cause),
        },
        Err(sqlx::Error::Database(refusal))
            if matches!(refusal.code().as_deref(), Some("3B001" | "25P01")) =>
        {
            Verdict::Ended
        }
        Err(error) => Verdict::Lost(DatabaseUnavailable::from(&error)),
    }
}

/// Whether the transaction `connection` is in is the run's, whose id is `run`.
async fn is_the_run_s(
    connection: &mut PgConnection,
    run: &str,
) -> Result<bool, DatabaseUnavailable> {
    transaction_id(connection)
        .await
        .map(|id| id == run)
        .map_err(|error| DatabaseUnavailable::from(&error))
}

/// The id of the transaction `connection` is in, given one now if it has none. A statement
/// outside any transaction runs in one of its own, with an id of its own. Named with its schema,
/// so that no function a file creates stands in for it.
async fn transaction_id(connection: &mut PgConnection) -> Result<String, sqlx::Error> {
    sqlx::query_scalar!(r#"SELECT pg_catalog.pg_current_xact_id()::text AS "id!""#)
        .fetch_one(connection)
        .await
}

impl SeedFailure {
    /// The failure `error` reports, its position read in `sql`, the text the server ran, when
    /// it was a file's. Never sqlx's text: its "at line N" is a line of the server's C source
    /// (plan, appendix A.4).
    fn of(error: &sqlx::Error, sql: Option<&str>) -> Self {
        let sqlx::Error::Database(refusal) = error else {
            return Self::Unavailable(DatabaseUnavailable::from(error));
        };
        // An error inside a function's body points into that body's own text instead.
        let position = match (refusal.try_downcast_ref::<PgDatabaseError>(), sql) {
            (Some(refusal), Some(sql)) => match refusal.position() {
                Some(PgErrorPosition::Original(character)) => line_and_column(sql, character),
                _ => None,
            },
            _ => None,
        };
        Self::Server {
            sqlstate: sqlstate_of(refusal.as_ref()),
            message: refusal.message().to_owned(),
            position,
        }
    }
}

/// The line and the column of the character at `position` of `text`, which counts characters
/// from 1, as the server's error position does. The position just past the end is where the
/// server points an error at the end of the input.
fn line_and_column(text: &str, position: usize) -> Option<ErrorPosition> {
    let mut characters = text.chars();
    let mut at = ErrorPosition { line: 1, column: 1 };
    for _ in 1..position {
        match characters.next()? {
            '\n' => {
                at = ErrorPosition {
                    line: at.line + 1,
                    column: 1,
                }
            }
            _ => at.column += 1,
        }
    }
    (position > 0).then_some(at)
}

/// A seed file that a run applied.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AppliedSeed {
    file: String,
}

impl AppliedSeed {
    /// Its name in the seed folder.
    pub fn file(&self) -> &str {
        &self.file
    }
}

/// Where in a seed file the server's error points: the line and the column, both counted from 1,
/// in characters.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ErrorPosition {
    pub line: usize,
    pub column: usize,
}

/// Why a seed run stopped.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SeedError {
    /// The database may be on another machine, by its address or by the server that answered
    /// there: nothing was applied.
    NotLocal(NotLocal),
    /// The folder could not be read: nothing was applied.
    Folder(io::ErrorKind),
    /// A seed file could not be read, such as one that is not UTF-8: nothing was applied.
    File { file: String, kind: io::ErrorKind },
    /// The database could not be used outside a seed file, as before the first one ran: nothing
    /// was applied.
    Unavailable(DatabaseUnavailable),
    /// A seed file failed. Refused by the server, it left the run's transaction to be rolled
    /// back: nothing was applied. When the connection failed instead, or was left where no
    /// question gets an answer, as a `COPY … FROM stdin` leaves it, the run closed it, which rolls
    /// back the run's transaction unless the file had ended that itself.
    Failed { file: String, failure: SeedFailure },
    /// A seed file ended the run's transaction itself, with `COMMIT` or `ROLLBACK`: what it and
    /// the files before it did may have stayed, and `just db-reset` starts again.
    EndedTransaction { file: String },
    /// Every file ran, and the commit failed. Refused by the server, as a deferred constraint
    /// makes it: nothing was applied. When the connection failed instead, the seeds may have been
    /// applied.
    Commit(SeedFailure),
}

/// What stopped a seed file, or the commit of a run.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SeedFailure {
    /// The server refused it: its SQLSTATE, its message, and where in the file its error points
    /// when the server says. Seeds run only against a database on this machine, so the message,
    /// which can quote the data, stays (plan, refinement 11).
    Server {
        sqlstate: Option<String>,
        message: String,
        position: Option<ErrorPosition>,
    },
    /// The connection failed, or was left where no question gets an answer: what became of the
    /// run's transaction cannot be told.
    Unavailable(DatabaseUnavailable),
}

impl fmt::Display for SeedError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::NotLocal(reason) => write!(f, "seeding refused: {reason}"),
            Self::Folder(kind) => write!(f, "cannot read the seed folder: {kind}"),
            Self::File { file, kind } => write!(f, "cannot read seed {file}: {kind}"),
            Self::Unavailable(cause) => write!(f, "cannot seed the database: {cause}"),
            Self::Failed {
                file,
                failure:
                    SeedFailure::Server {
                        sqlstate,
                        message,
                        position,
                    },
            } => {
                write!(f, "seed {file} failed")?;
                write_refusal(f, sqlstate.as_deref(), message, *position)?;
                f.write_str("; nothing was applied")
            }
            Self::Failed {
                file,
                failure: SeedFailure::Unavailable(cause),
            } => write!(
                f,
                "seed {file} failed: {cause}; the run closed its connection, which rolls back its \
                 transaction unless the file ended it itself: take out any COMMIT, ROLLBACK or \
                 COPY, then run just db-reset"
            ),
            Self::EndedTransaction { file } => write!(
                f,
                "seed {file} ended the run's transaction itself, so what it and the seeds before \
                 it did may have stayed: take its COMMIT or ROLLBACK out, then run just db-reset"
            ),
            Self::Commit(SeedFailure::Server {
                sqlstate,
                message,
                position,
            }) => {
                f.write_str("the seeds failed when committed")?;
                write_refusal(f, sqlstate.as_deref(), message, *position)?;
                f.write_str("; nothing was applied")
            }
            Self::Commit(SeedFailure::Unavailable(cause)) => write!(
                f,
                "the commit of the seeds did not complete: {cause}; whether they were applied \
                 cannot be told, so run just db-reset"
            ),
        }
    }
}

/// The rest of a sentence that says the server refused something: with what SQLSTATE, where,
/// and why.
fn write_refusal(
    f: &mut fmt::Formatter<'_>,
    sqlstate: Option<&str>,
    message: &str,
    position: Option<ErrorPosition>,
) -> fmt::Result {
    if let Some(sqlstate) = sqlstate {
        write!(f, " with SQLSTATE {sqlstate}")?;
    }
    if let Some(ErrorPosition { line, column }) = position {
        write!(f, " at line {line}, column {column}")?;
    }
    write!(f, ": {message}")
}

impl std::error::Error for SeedError {}

impl From<ServerCheckFailed> for SeedError {
    fn from(failure: ServerCheckFailed) -> Self {
        match failure {
            ServerCheckFailed::NotLocal(reason) => Self::NotLocal(reason),
            ServerCheckFailed::Unavailable(cause) => Self::Unavailable(cause),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn names(list: &[&str]) -> Vec<String> {
        list.iter().map(|name| (*name).to_owned()).collect()
    }

    fn at(line: usize, column: usize) -> Option<ErrorPosition> {
        Some(ErrorPosition { line, column })
    }

    #[test]
    fn the_seed_files_are_the_sql_files_in_name_order() {
        let selected = seed_files(names(&[
            "20261116000003_third.sql",
            "README.md",
            "20261116000001_first.sql",
            "20261116000010_tenth.sql",
            "notes.txt",
            "20261116000002_second.sql",
            "20261116000004_fourth.sql.bak",
            "20261116000005_fifth.SQL",
            "20261116000006_sixth.sql~",
        ]));

        assert_eq!(
            selected,
            [
                "20261116000001_first.sql",
                "20261116000002_second.sql",
                "20261116000003_third.sql",
                "20261116000010_tenth.sql",
            ]
        );
    }

    #[test]
    fn a_hidden_file_is_no_seed() {
        // What an editor leaves next to the file it edits, which the structure check ignores too.
        let selected = seed_files(names(&[
            ".20261116000001_first.sql.swp",
            ".#20261116000001_first.sql",
            ".sql",
            "20261116000001_first.sql",
        ]));

        assert_eq!(selected, ["20261116000001_first.sql"]);
    }

    #[test]
    fn a_folder_that_does_not_exist_holds_no_seed() {
        let missing = Path::new(env!("CARGO_MANIFEST_DIR")).join("no-such-folder");

        assert!(read_seed_files(&missing).unwrap().is_empty());
    }

    #[test]
    fn a_seed_folder_that_is_a_file_is_refused_by_the_kind_of_its_failure() {
        let file = Path::new(env!("CARGO_MANIFEST_DIR")).join("Cargo.toml");

        assert_eq!(
            read_seed_files(&file).err(),
            Some(SeedError::Folder(io::ErrorKind::NotADirectory))
        );
    }

    #[test]
    fn a_position_is_a_line_and_a_column_counted_from_one() {
        let text = "CREATE TABLE a ();\nINSERT INTO b VALUES (1);";

        assert_eq!(line_and_column(text, 1), at(1, 1));
        assert_eq!(line_and_column(text, 18), at(1, 18));
        // The line break ends the line it is on.
        assert_eq!(line_and_column(text, 19), at(1, 19));
        assert_eq!(line_and_column(text, 32), at(2, 13));
    }

    #[test]
    fn a_position_counts_characters_not_bytes() {
        // `ñ` and `ú` take two bytes each, and one character each for the server.
        let text = "INSERT INTO t VALUES ('ñandú');\nSELEC 1;";

        assert_eq!(line_and_column(text, 33), at(2, 1));
    }

    #[test]
    fn a_carriage_return_is_a_character_of_its_line() {
        let text = "SELECT 1;\r\nSELEC 2;";

        assert_eq!(line_and_column(text, 10), at(1, 10));
        assert_eq!(line_and_column(text, 12), at(2, 1));
    }

    #[test]
    fn the_position_just_past_the_end_is_where_the_text_ends() {
        // Where the server points an error at the end of the input.
        assert_eq!(line_and_column("CREATE TABLE t (id int", 23), at(1, 23));
        assert_eq!(line_and_column("SELECT 1;\n", 11), at(2, 1));
    }

    #[test]
    fn a_position_outside_the_text_has_no_line() {
        assert_eq!(line_and_column("SELECT 1", 0), None);
        assert_eq!(line_and_column("SELECT 1", 10), None);
        assert_eq!(line_and_column("", 2), None);
    }

    #[tokio::test]
    async fn a_database_that_may_be_on_another_machine_is_refused_before_the_folder_is_read() {
        let url = DatabaseUrl::parse("postgres://barras:hunter2-secret@db.example.com:5432/barras")
            .unwrap();
        // Read, this folder would fail the run with another error: it is a file.
        let folder = Path::new(env!("CARGO_MANIFEST_DIR")).join("Cargo.toml");

        let refused = seed(&url, &folder).await;

        assert_eq!(refused, Err(SeedError::NotLocal(NotLocal::OtherName)));
    }

    #[test]
    fn each_error_is_a_sentence_that_names_the_file() {
        let refusal = |position| SeedFailure::Server {
            sqlstate: Some("42P01".to_owned()),
            message: "relation \"barras.nowhere\" does not exist".to_owned(),
            position,
        };
        for (error, sentence) in [
            (
                SeedError::NotLocal(NotLocal::OtherName),
                "seeding refused: the database may be on another machine: its host is a name \
                 other than localhost",
            ),
            (
                SeedError::Folder(io::ErrorKind::PermissionDenied),
                "cannot read the seed folder: permission denied",
            ),
            (
                SeedError::File {
                    file: "20261116000001_demo.sql".to_owned(),
                    kind: io::ErrorKind::InvalidData,
                },
                "cannot read seed 20261116000001_demo.sql: invalid data",
            ),
            (
                SeedError::Unavailable(DatabaseUnavailable::Refused {
                    sqlstate: Some("28P01".to_owned()),
                }),
                "cannot seed the database: the server refused it with SQLSTATE 28P01",
            ),
            (
                SeedError::Failed {
                    file: "20261116000002_demo.sql".to_owned(),
                    failure: refusal(at(3, 13)),
                },
                "seed 20261116000002_demo.sql failed with SQLSTATE 42P01 at line 3, column 13: \
                 relation \"barras.nowhere\" does not exist; nothing was applied",
            ),
            (
                SeedError::Failed {
                    file: "20261116000002_demo.sql".to_owned(),
                    failure: refusal(None),
                },
                "seed 20261116000002_demo.sql failed with SQLSTATE 42P01: relation \
                 \"barras.nowhere\" does not exist; nothing was applied",
            ),
            (
                SeedError::Failed {
                    file: "20261116000002_demo.sql".to_owned(),
                    failure: SeedFailure::Server {
                        sqlstate: None,
                        message: "it went wrong".to_owned(),
                        position: None,
                    },
                },
                "seed 20261116000002_demo.sql failed: it went wrong; nothing was applied",
            ),
            (
                SeedError::Failed {
                    file: "20261116000002_demo.sql".to_owned(),
                    failure: SeedFailure::Unavailable(DatabaseUnavailable::ConnectionFailed(
                        io::ErrorKind::ConnectionReset,
                    )),
                },
                "seed 20261116000002_demo.sql failed: the connection failed: connection reset; \
                 the run closed its connection, which rolls back its transaction unless the file \
                 ended it itself: take out any COMMIT, ROLLBACK or COPY, then run just db-reset",
            ),
            (
                SeedError::Failed {
                    file: "20261116000002_demo.sql".to_owned(),
                    failure: SeedFailure::Unavailable(DatabaseUnavailable::ProtocolViolated),
                },
                "seed 20261116000002_demo.sql failed: the server answered outside the protocol; \
                 the run closed its connection, which rolls back its transaction unless the file \
                 ended it itself: take out any COMMIT, ROLLBACK or COPY, then run just db-reset",
            ),
            (
                SeedError::EndedTransaction {
                    file: "20261116000002_demo.sql".to_owned(),
                },
                "seed 20261116000002_demo.sql ended the run's transaction itself, so what it and \
                 the seeds before it did may have stayed: take its COMMIT or ROLLBACK out, then \
                 run just db-reset",
            ),
            (
                SeedError::Commit(SeedFailure::Server {
                    sqlstate: Some("23503".to_owned()),
                    message: "insert or update on table \"child\" violates foreign key \
                              constraint \"child_parent_fkey\""
                        .to_owned(),
                    position: None,
                }),
                "the seeds failed when committed with SQLSTATE 23503: insert or update on table \
                 \"child\" violates foreign key constraint \"child_parent_fkey\"; nothing was \
                 applied",
            ),
            (
                SeedError::Commit(SeedFailure::Unavailable(
                    DatabaseUnavailable::ConnectionFailed(io::ErrorKind::ConnectionReset),
                )),
                "the commit of the seeds did not complete: the connection failed: connection \
                 reset; whether they were applied cannot be told, so run just db-reset",
            ),
        ] {
            assert_eq!(error.to_string(), sentence);
        }
    }
}
