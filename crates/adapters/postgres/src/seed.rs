//! The seed runner: the SQL files of a folder, applied in one transaction (spec §9.1).

use std::fmt;
use std::fs;
use std::io;
use std::path::Path;

use sqlx::postgres::{PgDatabaseError, PgErrorPosition};
use sqlx::{AssertSqlSafe, Connection, PgConnection};

use crate::unavailable::sqlstate_of;
use crate::{DatabaseUnavailable, DatabaseUrl, NotLocal, ensure_local};

/// Applies the seed files of `folder` to the database `url` names, on this machine only, and
/// returns them in the order they ran.
///
/// A seed file is a `.sql` file of the folder; the files run in name order, each whole, in one
/// transaction, so that a file that fails leaves nothing of the run behind. Other files are
/// ignored, and a folder that does not exist holds none. Every file is read before the first
/// one runs, and with nothing to apply nothing connects.
///
/// The guard comes first ([`ensure_local`]): seeds are local development data, so a database
/// that may be on another machine is refused before the folder is read. The run has a
/// connection of its own, without the pools' statement timeout.
pub async fn seed(url: &DatabaseUrl, folder: &Path) -> Result<Vec<AppliedSeed>, SeedError> {
    ensure_local(url).map_err(SeedError::NotLocal)?;
    let files = read_seed_files(folder)?;
    apply(url, &files).await
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

/// Applies `files` in order, in one transaction on a connection of its own, which is closed
/// afterwards whatever happened: a run that failed ends with its session, and the server rolls
/// back what it left open.
pub(crate) async fn apply(
    url: &DatabaseUrl,
    files: &[SeedFile],
) -> Result<Vec<AppliedSeed>, SeedError> {
    if files.is_empty() {
        return Ok(Vec::new());
    }
    let mut connection = PgConnection::connect_with(&url.connect_options())
        .await
        .map_err(|error| SeedError::Unavailable(DatabaseUnavailable::from(&error)))?;
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

async fn apply_in_one_transaction(
    connection: &mut PgConnection,
    files: &[SeedFile],
) -> Result<Vec<AppliedSeed>, SeedError> {
    let unusable = |error: sqlx::Error| SeedError::Unavailable(DatabaseUnavailable::from(&error));
    let mut transaction = connection.begin().await.map_err(unusable)?;
    // `SET LOCAL` as a function: the marker lasts as long as the transaction, so a file that
    // ends it with `COMMIT` or `ROLLBACK` takes the marker with it (plan, refinement 5).
    sqlx::query_scalar!("SELECT set_config('barras.seed_run', 'on', true)")
        .fetch_one(&mut *transaction)
        .await
        .map_err(unusable)?;
    let mut applied = Vec::new();
    for file in files {
        let ran = sqlx::raw_sql(AssertSqlSafe(file.sql.as_str()))
            .execute(&mut *transaction)
            .await;
        let open = transaction_is_open(&mut transaction).await;
        let name = || file.name.clone();
        match (ran, open) {
            // What the file did before it ended the transaction stays, whether it failed after
            // or not, so that comes first.
            (_, Ok(false)) => return Err(SeedError::EndedTransaction { file: name() }),
            (Err(error), _) => {
                return Err(SeedError::Failed {
                    file: name(),
                    failure: SeedFailure::of(&error, Some(&file.sql)),
                });
            }
            (Ok(_), Err(error)) => {
                return Err(SeedError::Failed {
                    file: name(),
                    failure: SeedFailure::of(&error, None),
                });
            }
            (Ok(_), Ok(true)) => applied.push(AppliedSeed { file: name() }),
        }
    }
    transaction
        .commit()
        .await
        .map_err(|error| SeedError::Commit(SeedFailure::of(&error, None)))?;
    Ok(applied)
}

/// Whether the run's transaction is still open, by its marker. A transaction that a statement
/// aborted refuses the question with an error.
async fn transaction_is_open(connection: &mut PgConnection) -> Result<bool, sqlx::Error> {
    sqlx::query_scalar!(
        r#"SELECT current_setting('barras.seed_run', true) IS NOT DISTINCT FROM 'on' AS "open!""#
    )
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
    /// The database may be on another machine: nothing was read or applied.
    NotLocal(NotLocal),
    /// The folder could not be read: nothing was applied.
    Folder(io::ErrorKind),
    /// A seed file could not be read, such as one that is not UTF-8: nothing was applied.
    File { file: String, kind: io::ErrorKind },
    /// The database could not be used before the first file ran: nothing was applied.
    Unavailable(DatabaseUnavailable),
    /// A seed file failed, and the run's transaction with it: nothing was applied.
    Failed { file: String, failure: SeedFailure },
    /// A seed file ended the run's transaction itself, with `COMMIT` or `ROLLBACK`: what it and
    /// the files before it did may have stayed, and `just db-reset` starts again.
    EndedTransaction { file: String },
    /// Every file ran, and the transaction failed when it was committed, as a deferred
    /// constraint makes it: nothing was applied.
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
    /// The database stopped answering.
    Unavailable(DatabaseUnavailable),
}

impl fmt::Display for SeedError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::NotLocal(reason) => write!(f, "seeding refused: {reason}"),
            Self::Folder(kind) => write!(f, "cannot read the seed folder: {kind}"),
            Self::File { file, kind } => write!(f, "cannot read seed {file}: {kind}"),
            Self::Unavailable(cause) => write!(f, "cannot seed the database: {cause}"),
            Self::Failed { file, failure } => {
                write!(f, "seed {file} failed")?;
                write_failure(f, failure)?;
                f.write_str("; nothing was applied")
            }
            Self::EndedTransaction { file } => write!(
                f,
                "seed {file} ended the run's transaction itself, so what it and the seeds before \
                 it did may have stayed: take its COMMIT or ROLLBACK out, then run just db-reset"
            ),
            Self::Commit(failure) => {
                f.write_str("the seeds failed when committed")?;
                write_failure(f, failure)?;
                f.write_str("; nothing was applied")
            }
        }
    }
}

/// The rest of a sentence that says something failed: with what SQLSTATE, where, and why.
fn write_failure(f: &mut fmt::Formatter<'_>, failure: &SeedFailure) -> fmt::Result {
    match failure {
        SeedFailure::Server {
            sqlstate,
            message,
            position,
        } => {
            if let Some(sqlstate) = sqlstate {
                write!(f, " with SQLSTATE {sqlstate}")?;
            }
            if let Some(ErrorPosition { line, column }) = position {
                write!(f, " at line {line}, column {column}")?;
            }
            write!(f, ": {message}")
        }
        SeedFailure::Unavailable(cause) => write!(f, ": {cause}"),
    }
}

impl std::error::Error for SeedError {}

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
                 nothing was applied",
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
        ] {
            assert_eq!(error.to_string(), sentence);
        }
    }
}
