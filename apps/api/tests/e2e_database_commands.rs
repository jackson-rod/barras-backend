//! The database commands of the real `barras-api` binary (BAR-4 spec §3.2, §4.2), each against a
//! container of its test's own: `migrate` applies every migration the binary carries, then has
//! nothing to apply, and names a migration that fails with its SQLSTATE; `seed` loads the SQL
//! files of a folder in name order; `reset` rebuilds the database with only the migrations and the
//! seeds. Each reads only its variables and names every one missing or not valid at once; `seed`
//! and `reset` refuse, before any connection, outside `local` and for a database on another
//! machine, and take an option where their folder goes for a request for the usage; and no
//! output shows a password.
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)] // integration test crate: every item is test code

use std::fs;
use std::io;
use std::net::{Ipv4Addr, TcpListener};
use std::path::{Path, PathBuf};
use std::process::{Command, Output};
use std::time::{Duration, Instant};

use barras_api::{ApiConfig, LocalDatabaseConfig, MigrateConfig};
use barras_testkit::process::{BinaryCommand, describe, text};
use barras_testkit::{PostgresContainer, repository_file};

const BINARY: &str = env!("CARGO_BIN_EXE_barras-api");

/// How long a command may take before the test fails: far longer than any takes, for an
/// instrumented build on a busy machine.
const PATIENCE: Duration = Duration::from_secs(30);

/// How long a refusal may take. It comes before any connection, while a connection to an address
/// that never answers waits for the system's TCP timeout, more than a minute.
const AT_ONCE: Duration = Duration::from_secs(10);

/// The password of every database URL a test gives the binary, which no output may show.
const PASSWORD: &str = "hunter2-secret";

/// What a database URL must look like, as the problem line of either variable says it.
const URL_EXPECTED: &str = "expected a postgres:// URL that names its user, password, host, \
                            port and database, without a query string, an IPv6 address in \
                            brackets or several hosts";

/// Runs `barras-api` with `args` to its end, with `variables` as its whole environment; the last
/// value given for a name wins.
fn run(args: &[&str], variables: &[(&str, &str)]) -> Output {
    let command = args
        .iter()
        .fold(BinaryCommand::new(BINARY), |command, arg| command.arg(arg));
    variables
        .iter()
        .fold(command, |command, (name, value)| command.env(name, value))
        .run_within(PATIENCE)
}

/// Every variable the server reads, each set to a value none of them accepts, then `variables`
/// over them: a command that read one it does not use would stop on it.
fn not_valid_but(variables: &[(&'static str, &'static str)]) -> Vec<(&'static str, &'static str)> {
    ApiConfig::VARIABLES
        .iter()
        .map(|&name| (name, "not-valid"))
        .chain(variables.iter().copied())
        .collect()
}

/// What `psql`, run inside the container, prints for `sql`, trimmed. A test reads the database
/// through the container: only the postgres adapter and the testkit may take a driver.
fn psql(container: &PostgresContainer, sql: &str) -> String {
    let output = Command::new("docker")
        .args([
            "exec",
            container.id(),
            "psql",
            "--username=barras",
            "--dbname=barras",
        ])
        .args(["--quiet", "--tuples-only", "--no-align", "--command", sql])
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "psql {sql}: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    text(&output.stdout).trim().to_owned()
}

/// The files of the repository's `migrations/` folder, in name order: every migration the binary
/// carries, since it embeds them when it is built.
fn migration_files() -> Vec<String> {
    let mut files: Vec<String> = fs::read_dir(repository_file("migrations"))
        .unwrap()
        .map(|entry| entry.unwrap().file_name().into_string().unwrap())
        .filter(|name| !name.starts_with('.'))
        .collect();
    files.sort();
    files
}

/// One line of standard output per migration the binary carries.
fn every_migration_applied() -> Vec<String> {
    migration_files()
        .iter()
        .map(|file| format!("applied migration {file}"))
        .collect()
}

/// A folder of seed files that one test writes in the target folder, removed when it ends.
struct SeedFolder(PathBuf);

impl SeedFolder {
    fn new(test: &str) -> Self {
        let path = Path::new(env!("CARGO_TARGET_TMPDIR")).join(format!(
            "e2e_database_commands-{test}-{}",
            std::process::id()
        ));
        // A run killed before its end leaves its folder behind, and process ids come back.
        if path.exists() {
            fs::remove_dir_all(&path).unwrap();
        }
        fs::create_dir_all(&path).unwrap();
        Self(path)
    }

    fn with(self, name: &str, content: &str) -> Self {
        fs::write(self.0.join(name), content).unwrap();
        self
    }

    fn path(&self) -> &str {
        self.0.to_str().unwrap()
    }
}

impl Drop for SeedFolder {
    fn drop(&mut self) {
        // A folder left behind only takes room in the target folder, and a test that unwinds
        // must not panic a second time.
        let _ = fs::remove_dir_all(&self.0);
    }
}

#[tokio::test]
async fn migrate_applies_every_migration_the_binary_carries_then_has_nothing_to_apply() {
    let container = PostgresContainer::start().await;
    let mut variables = not_valid_but(&[]);
    variables.push(("API_DATABASE_URL", container.url()));

    let first = run(&["migrate"], &variables);
    let second = run(&["migrate"], &variables);

    assert_eq!(first.status.code(), Some(0), "{}", describe(&first));
    let lines: Vec<&str> = text(&first.stdout).lines().collect();
    assert_eq!(lines, every_migration_applied());
    assert_eq!(text(&first.stderr), "");
    assert_eq!(second.status.code(), Some(0), "{}", describe(&second));
    assert_eq!(text(&second.stdout), "no migration to apply\n");
    assert_eq!(text(&second.stderr), "");
    let schema = "SELECT count(*) FROM pg_namespace WHERE nspname = 'barras'";
    assert_eq!(psql(&container, schema), "1");
}

#[tokio::test]
async fn a_migration_that_fails_is_named_on_standard_error_with_its_sqlstate() {
    let container = PostgresContainer::start().await;
    // What the first migration creates, made by hand, so that the migration fails.
    psql(&container, "CREATE SCHEMA barras");
    // The tests' password, which the failure must not show.
    psql(
        &container,
        &format!("ALTER USER barras PASSWORD '{PASSWORD}'"),
    );
    let url = container
        .url()
        .replace(":barras@", &format!(":{PASSWORD}@"));

    let output = run(&["migrate"], &[("API_DATABASE_URL", &url)]);

    assert_eq!(output.status.code(), Some(1), "{}", describe(&output));
    let first = &migration_files()[0];
    assert_eq!(
        text(&output.stderr),
        format!("migration {first} failed with SQLSTATE 42P06: schema \"barras\" already exists\n")
    );
    // It was the first, so no line tells of a migration applied before it.
    assert_eq!(text(&output.stdout), "");
    assert!(!text(&output.stderr).contains(PASSWORD));
}

#[tokio::test]
async fn seed_loads_the_sql_files_of_the_folder_in_name_order() {
    let container = PostgresContainer::start().await;
    let migrated = run(&["migrate"], &[("API_DATABASE_URL", container.url())]);
    assert_eq!(migrated.status.code(), Some(0), "{}", describe(&migrated));
    let seeds = SeedFolder::new("seed")
        .with(
            "20261116000002_rows.sql",
            "INSERT INTO barras.demo (id, name) VALUES (1, 'first'), (2, 'second');",
        )
        .with(
            "20261116000001_table.sql",
            "CREATE TABLE barras.demo (id int PRIMARY KEY, name text NOT NULL);",
        )
        .with("README.md", "Not a seed: only the .sql files are.");
    let missing = SeedFolder::new("seed-missing");
    let missing = format!("{}/no-such-folder", missing.path());
    let mut variables = not_valid_but(&[("APP_ENV", "local")]);
    variables.push(("API_DATABASE_URL", container.url()));

    let none = run(&["seed", &missing], &variables);
    let seeded = run(&["seed", seeds.path()], &variables);
    let again = run(&["seed", seeds.path()], &variables);

    assert_eq!(none.status.code(), Some(0), "{}", describe(&none));
    assert_eq!(text(&none.stdout), "no seed to apply\n");
    assert_eq!(seeded.status.code(), Some(0), "{}", describe(&seeded));
    assert_eq!(
        text(&seeded.stdout),
        "applied seed 20261116000001_table.sql\napplied seed 20261116000002_rows.sql\n"
    );
    assert_eq!(text(&seeded.stderr), "");
    let names = "SELECT string_agg(name, ',' ORDER BY id) FROM barras.demo";
    assert_eq!(psql(&container, names), "first,second");
    // A seed assumes a database just migrated: run again, its first file fails, and the run says
    // so on standard error, with the file and the SQLSTATE, and applies nothing.
    assert_eq!(again.status.code(), Some(1), "{}", describe(&again));
    let failure = text(&again.stderr);
    assert!(
        failure.starts_with("seed 20261116000001_table.sql failed with SQLSTATE 42P07"),
        "{failure}"
    );
    assert!(failure.ends_with("; nothing was applied\n"), "{failure}");
    assert_eq!(text(&again.stdout), "");
    assert_eq!(psql(&container, names), "first,second");
}

#[tokio::test]
async fn reset_rebuilds_the_database_with_only_the_migrations_and_the_seeds() {
    let container = PostgresContainer::start().await;
    let migrated = run(&["migrate"], &[("API_DATABASE_URL", container.url())]);
    assert_eq!(migrated.status.code(), Some(0), "{}", describe(&migrated));
    psql(&container, "CREATE TABLE public.left_over (id int)");
    let seeds = SeedFolder::new("reset").with(
        "20261116000001_demo.sql",
        "CREATE TABLE barras.demo (id int PRIMARY KEY);\nINSERT INTO barras.demo VALUES (1), (2);",
    );
    let mut variables = not_valid_but(&[("APP_ENV", "local")]);
    variables.push(("API_DATABASE_URL", container.url()));
    // The same server, naming a database it does not have yet.
    let (server, _) = container.url().rsplit_once('/').unwrap();
    let fresh = format!("{server}/fresh");

    let output = run(&["reset", seeds.path()], &variables);
    let created = run(
        &["reset", seeds.path()],
        &[("APP_ENV", "local"), ("API_DATABASE_URL", &fresh)],
    );

    assert_eq!(output.status.code(), Some(0), "{}", describe(&output));
    let mut steps = vec![
        "dropped the database barras".to_owned(),
        "created the database barras".to_owned(),
    ];
    steps.extend(every_migration_applied());
    steps.push("applied seed 20261116000001_demo.sql".to_owned());
    let lines: Vec<&str> = text(&output.stdout).lines().collect();
    assert_eq!(lines, steps);
    assert_eq!(text(&output.stderr), "");
    assert_eq!(created.status.code(), Some(0), "{}", describe(&created));
    let lines: Vec<&str> = text(&created.stdout).lines().collect();
    assert_eq!(
        lines[..2],
        [
            "found no database fresh to drop",
            "created the database fresh"
        ]
    );
    let tables = "SELECT string_agg(table_schema || '.' || table_name, ',' \
                  ORDER BY table_schema, table_name) FROM information_schema.tables \
                  WHERE table_schema NOT IN ('pg_catalog', 'information_schema')";
    assert_eq!(
        psql(&container, tables),
        "barras.demo,public._sqlx_migrations"
    );
    assert_eq!(psql(&container, "SELECT count(*) FROM barras.demo"), "2");
}

#[tokio::test]
async fn an_option_where_the_folder_goes_prints_the_usage_and_changes_nothing() {
    let container = PostgresContainer::start().await;
    psql(&container, "CREATE TABLE public.left_over (id int)");
    // Everything a reset needs, so that only the command line stops one.
    let variables = [("APP_ENV", "local"), ("API_DATABASE_URL", container.url())];

    let outputs: Vec<_> = [["reset", "--help"], ["reset", "-h"], ["seed", "-h"]]
        .into_iter()
        .map(|args| (args, run(&args, &variables)))
        .collect();

    // Read as a folder, `--help` would have had the database dropped, the table with it.
    let tables = "SELECT string_agg(table_schema || '.' || table_name, ',' \
                  ORDER BY table_schema, table_name) FROM information_schema.tables \
                  WHERE table_schema NOT IN ('pg_catalog', 'information_schema')";
    assert_eq!(psql(&container, tables), "public.left_over");
    for (args, output) in outputs {
        assert_eq!(
            output.status.code(),
            Some(2),
            "{args:?}: {}",
            describe(&output)
        );
        let stderr = text(&output.stderr);
        assert!(
            stderr.starts_with("usage: barras-api"),
            "{args:?}: {stderr}"
        );
        assert_eq!(text(&output.stdout), "", "{args:?}");
    }
}

#[test]
fn migrate_names_its_one_variable_when_it_is_missing() {
    let output = run(&["migrate"], &[]);

    assert_eq!(output.status.code(), Some(1), "{}", describe(&output));
    let lines: Vec<String> = MigrateConfig::VARIABLES
        .iter()
        .map(|name| format!("{name} is missing: {URL_EXPECTED}\n"))
        .collect();
    assert_eq!(text(&output.stderr), lines.concat());
    assert_eq!(text(&output.stdout), "");
}

#[test]
fn seed_and_reset_name_both_their_variables_at_once() {
    for command in ["seed", "reset"] {
        let output = run(&[command, "seeds"], &[]);

        assert_eq!(output.status.code(), Some(1), "{}", describe(&output));
        let lines: Vec<&str> = text(&output.stderr).lines().collect();
        assert_eq!(
            lines.len(),
            LocalDatabaseConfig::VARIABLES.len(),
            "{lines:?}"
        );
        for (line, name) in lines.iter().zip(LocalDatabaseConfig::VARIABLES) {
            assert!(
                line.starts_with(&format!("{name} is missing: expected ")),
                "{line}"
            );
        }
        assert_eq!(text(&output.stdout), "");
    }
}

#[test]
fn a_database_url_that_is_not_valid_is_named_and_never_shown() {
    let no_port = format!("postgres://barras:{PASSWORD}@127.0.0.1/barras");

    let migrating = run(&["migrate"], &[("API_DATABASE_URL", &no_port)]);
    let seeding = run(
        &["seed", "seeds"],
        &[("APP_ENV", "local"), ("API_DATABASE_URL", &no_port)],
    );

    for output in [migrating, seeding] {
        assert_eq!(output.status.code(), Some(1), "{}", describe(&output));
        assert_eq!(
            text(&output.stderr),
            format!("API_DATABASE_URL is not valid: {URL_EXPECTED}\n")
        );
        assert_eq!(text(&output.stdout), "");
    }
}

#[test]
fn seed_and_reset_refuse_outside_local_before_any_connection() {
    // Where the database would be: a listener of the test's own, which keeps whatever connects.
    let listener = TcpListener::bind((Ipv4Addr::LOCALHOST, 0)).unwrap();
    listener.set_nonblocking(true).unwrap();
    let url = format!(
        "postgres://barras:{PASSWORD}@{}/barras",
        listener.local_addr().unwrap()
    );

    for (command, refused) in [
        ("seed", "seeding refused: APP_ENV is not local\n"),
        ("reset", "reset refused: APP_ENV is not local\n"),
    ] {
        for app_env in ["staging", "production"] {
            let started = Instant::now();
            let output = run(
                &[command, "seeds"],
                &[("APP_ENV", app_env), ("API_DATABASE_URL", &url)],
            );

            assert!(started.elapsed() < AT_ONCE, "{command} with {app_env}");
            assert_eq!(output.status.code(), Some(1), "{}", describe(&output));
            assert_eq!(text(&output.stderr), refused);
            assert_eq!(text(&output.stdout), "");
        }
    }
    let connected = listener.accept().err().map(|error| error.kind());
    assert_eq!(
        connected,
        Some(io::ErrorKind::WouldBlock),
        "a command connected"
    );
}

#[test]
fn seed_and_reset_refuse_a_database_on_another_machine_before_any_connection() {
    // TEST-NET-1 (RFC 5737), where no machine answers.
    let url = format!("postgres://barras:{PASSWORD}@192.0.2.10:5432/barras");

    for (command, refused) in [("seed", "seeding refused"), ("reset", "reset refused")] {
        let started = Instant::now();
        let output = run(
            &[command, "seeds"],
            &[("APP_ENV", "local"), ("API_DATABASE_URL", &url)],
        );

        assert!(started.elapsed() < AT_ONCE, "{command}");
        assert_eq!(output.status.code(), Some(1), "{}", describe(&output));
        let stderr = text(&output.stderr);
        let reason = format!("{refused}: the database may be on another machine: ");
        assert!(stderr.starts_with(&reason), "{stderr}");
        assert!(!stderr.contains("192.0.2.10"), "{stderr}");
        assert!(!stderr.contains(PASSWORD), "{stderr}");
        assert_eq!(text(&output.stdout), "");
    }
}
