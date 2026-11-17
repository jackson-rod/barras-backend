//! The database commands of `barras-api` (spec §3.2, §4.2): `migrate` applies the migrations the
//! binary carries, `seed` loads seed files into the local database, and `reset` rebuilds it.
//!
//! Each reads only the variables it uses, says on standard output what each step did, one line
//! each, and on standard error why it stopped, in sentences of a fixed set: never a URL, a
//! password or the driver's text. None starts the logs, since none reads `LOG_LEVEL`: what a
//! command prints is its whole report.

use std::fmt::Display;
use std::future::Future;
use std::io::{self, Write};
use std::path::Path;
use std::process::ExitCode;

use barras_adapter_postgres::{AppliedMigration, AppliedSeed, MigrateError, Reset, ResetError};
use barras_bootstrap::{AppEnv, Environment, build_runtime, report_failure, with_sources};

use crate::config::{LocalDatabaseConfig, MigrateConfig};

/// What `seed` and `reset` answer outside a local environment, before anything connects. The
/// adapter refuses a database that may be on another machine on its own.
const SEED_REFUSED: &str = "seeding refused: APP_ENV is not local";
const RESET_REFUSED: &str = "reset refused: APP_ENV is not local";

/// What a command did, one line per step, and how it ended: `Err` with why it stopped.
type Report<E> = (Vec<String>, Result<(), E>);

/// `barras-api migrate`: applies the pending migrations to `API_DATABASE_URL`, whatever
/// `APP_ENV` is, since it is the deploy step of every environment.
pub(crate) fn migrate() -> ExitCode {
    let config = match MigrateConfig::from_env(Environment::from_process()) {
        Ok(config) => config,
        Err(error) => return report_failure(&error),
    };
    run(async move {
        match barras_adapter_postgres::migrate(config.database_url.expose().get()).await {
            Ok(applied) => (
                migration_lines(applied.iter().map(AppliedMigration::file)),
                Ok(()),
            ),
            Err(error) => (applied_before(&error), Err(error)),
        }
    })
}

/// `barras-api seed <folder>`: loads the seed files of `folder` into the local database.
pub(crate) fn seed(folder: &Path) -> ExitCode {
    let config = match local_database(SEED_REFUSED) {
        Ok(config) => config,
        Err(refused) => return refused,
    };
    run(async move {
        match barras_adapter_postgres::seed(config.database_url.expose().get(), folder).await {
            Ok(applied) => (seed_lines(applied.iter().map(AppliedSeed::file)), Ok(())),
            // A run that failed applied nothing, or its error says what may have stayed.
            Err(error) => (Vec::new(), Err(error)),
        }
    })
}

/// `barras-api reset <folder>`: rebuilds the local database, then loads the seed files of
/// `folder` into it.
pub(crate) fn reset(seeds: &Path) -> ExitCode {
    let config = match local_database(RESET_REFUSED) {
        Ok(config) => config,
        Err(refused) => return refused,
    };
    run(async move {
        match barras_adapter_postgres::reset(config.database_url.expose().get(), seeds).await {
            Ok(reset) => (reset_lines(&reset), Ok(())),
            Err(error) => (reset_kept(&error), Err(error)),
        }
    })
}

/// The configuration of `seed` and `reset`. Every problem with its variables, or else `refused`
/// when `APP_ENV` is not `local`, is said on standard error before anything connects, and the
/// exit status of the failure comes back instead.
fn local_database(refused: &str) -> Result<LocalDatabaseConfig, ExitCode> {
    let config = LocalDatabaseConfig::from_env(Environment::from_process())
        .map_err(|error| report_failure(&error))?;
    if config.env != AppEnv::Local {
        return Err(report_failure(&refused));
    }
    Ok(config)
}

/// Runs `command` on a runtime of its own, writes the lines of what it did on standard output,
/// then, when it failed, why on standard error.
fn run<E: Display>(command: impl Future<Output = Report<E>>) -> ExitCode {
    let runtime = match build_runtime() {
        Ok(runtime) => runtime,
        Err(error) => return report_failure(&with_sources(&error)),
    };
    let (lines, outcome) = runtime.block_on(command);
    let written = write_lines(io::stdout().lock(), &lines);
    match outcome {
        Ok(()) => written,
        Err(error) => report_failure(&error),
    }
}

/// What `migrate` did: one line per migration applied, in order, or one that says there was
/// none to apply.
fn migration_lines<'a>(files: impl IntoIterator<Item = &'a str>) -> Vec<String> {
    or_none(applied("migration", files), "no migration to apply")
}

/// The migrations a run that failed applied before it stopped: each runs in a transaction of its
/// own, so they stay applied (spec §7).
fn applied_before(error: &MigrateError) -> Vec<String> {
    match error {
        MigrateError::Migration { applied: done, .. } => {
            applied("migration", done.iter().map(AppliedMigration::file))
        }
        _ => Vec::new(),
    }
}

/// What a reset that failed did that stays: the migrations applied before one that failed, as
/// [`applied_before`] tells them for `migrate`. The error of any other step says alone what state
/// the database was left in.
fn reset_kept(error: &ResetError) -> Vec<String> {
    match error {
        ResetError::Migrate(error) => applied_before(error),
        _ => Vec::new(),
    }
}

/// What `seed` did, as [`migration_lines`] says it of the migrations.
fn seed_lines<'a>(files: impl IntoIterator<Item = &'a str>) -> Vec<String> {
    or_none(applied("seed", files), "no seed to apply")
}

/// One line per file applied, a `migration` or a `seed`, in order.
fn applied<'a>(what: &str, files: impl IntoIterator<Item = &'a str>) -> Vec<String> {
    files
        .into_iter()
        .map(|file| format!("applied {what} {file}"))
        .collect()
}

/// `lines`, or `none` alone when there is no line, so that a step with nothing to do says so.
fn or_none(lines: Vec<String>, none: &str) -> Vec<String> {
    if lines.is_empty() {
        vec![none.to_owned()]
    } else {
        lines
    }
}

/// What `reset` did, step by step: the drop, the creation, the migrations, then the seeds.
fn reset_lines(reset: &Reset) -> Vec<String> {
    let database = reset.database();
    let dropped = if reset.dropped() {
        format!("dropped the database {database}")
    } else {
        format!("found no database {database} to drop")
    };
    let migrations = reset.migrations().iter().map(AppliedMigration::file);
    let seeds = reset.seeds().iter().map(AppliedSeed::file);
    [dropped, format!("created the database {database}")]
        .into_iter()
        .chain(migration_lines(migrations))
        .chain(seed_lines(seeds))
        .collect()
}

/// Writes `lines` to `out`, each on a line of its own. `println!` would panic on a standard
/// output that was closed; this reports it like any other failure, though the work the lines
/// tell of is done.
fn write_lines(mut out: impl Write, lines: &[String]) -> ExitCode {
    let written = lines
        .iter()
        .try_for_each(|line| writeln!(out, "{line}"))
        .and_then(|()| out.flush());
    match written {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => report_failure(&format!("what was done could not be written: {error}")),
    }
}

#[cfg(test)]
mod tests {
    use barras_adapter_postgres::{MigrationFailure, SeedError};

    use super::*;

    /// A standard output that was closed: every write fails.
    struct Closed;

    impl Write for Closed {
        fn write(&mut self, _bytes: &[u8]) -> io::Result<usize> {
            Err(io::Error::from(io::ErrorKind::BrokenPipe))
        }

        fn flush(&mut self) -> io::Result<()> {
            Err(io::Error::from(io::ErrorKind::BrokenPipe))
        }
    }

    #[test]
    fn each_migration_applied_is_a_line_in_order() {
        let files = [
            "20261117024143_create_schema_barras.sql",
            "20261201000000_x.sql",
        ];

        assert_eq!(
            migration_lines(files),
            [
                "applied migration 20261117024143_create_schema_barras.sql",
                "applied migration 20261201000000_x.sql",
            ]
        );
    }

    #[test]
    fn with_no_migration_applied_the_one_line_says_so() {
        assert_eq!(migration_lines([]), ["no migration to apply"]);
    }

    #[test]
    fn a_run_that_stopped_before_applying_a_migration_tells_none_and_no_line_of_none() {
        // A failed run's lines tell what stays applied; `no migration to apply` would tell what
        // the run never found out.
        let failed_first = MigrateError::Migration {
            applied: Vec::new(),
            file: "20261117024143_create_schema_barras.sql".to_owned(),
            failure: MigrationFailure::Server { sqlstate: None },
        };
        let refused = MigrateError::Edited {
            version: 20261117024143,
        };

        for stopped in [failed_first, refused] {
            assert_eq!(applied_before(&stopped), Vec::<String>::new(), "{stopped}");
        }
    }

    #[test]
    fn a_reset_whose_later_migration_failed_tells_each_migration_applied_before_it() {
        let failed = ResetError::Migrate(MigrateError::Migration {
            applied: vec![
                AppliedMigration::new(20261117024143, "create_schema_barras"),
                AppliedMigration::new(20261201000000, "x"),
            ],
            file: "20261202000000_y.sql".to_owned(),
            failure: MigrationFailure::Server {
                sqlstate: Some("23505".to_owned()),
            },
        });

        assert_eq!(
            reset_kept(&failed),
            [
                "applied migration 20261117024143_create_schema_barras.sql",
                "applied migration 20261201000000_x.sql",
            ]
        );
    }

    #[test]
    fn a_reset_that_failed_at_another_step_tells_no_line() {
        // Its sentence alone says what state the database was left in: as it was, or created and
        // migrated.
        for failed in [
            ResetError::NameTooLong,
            ResetError::Seed(SeedError::EndedTransaction {
                file: "20261116000001_demo.sql".to_owned(),
            }),
        ] {
            assert_eq!(reset_kept(&failed), Vec::<String>::new(), "{failed}");
        }
    }

    #[test]
    fn each_seed_applied_is_a_line_in_order() {
        let files = ["20261116000001_first.sql", "20261116000002_second.sql"];

        assert_eq!(
            seed_lines(files),
            [
                "applied seed 20261116000001_first.sql",
                "applied seed 20261116000002_second.sql",
            ]
        );
    }

    #[test]
    fn with_no_seed_applied_the_one_line_says_so() {
        assert_eq!(seed_lines([]), ["no seed to apply"]);
    }

    #[test]
    fn the_lines_are_written_each_on_a_line_of_its_own() {
        let mut written = Vec::new();

        let status = write_lines(&mut written, &migration_lines(["1_a.sql", "2_b.sql"]));

        assert_eq!(status, ExitCode::SUCCESS);
        assert_eq!(
            String::from_utf8(written).unwrap(),
            "applied migration 1_a.sql\napplied migration 2_b.sql\n"
        );
    }

    #[test]
    fn lines_that_cannot_be_written_are_a_failure_rather_than_a_panic() {
        assert_eq!(write_lines(Closed, &seed_lines([])), ExitCode::FAILURE);
    }
}
