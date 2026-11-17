//! `reset` against a real Postgres, in a container of each test's own: the database rebuilt with
//! only the migrations and the seeds, whatever it held, whoever was connected to it and whatever
//! its name holds; the server that answers; and the state each failure leaves it in.
#![allow(clippy::unwrap_used, clippy::expect_used)] // an integration test: a step that fails must fail the test

use std::fs;
use std::io;
use std::net::TcpListener;
use std::path::{Path, PathBuf};

use barras_adapter_postgres::{
    AppliedMigration, AppliedSeed, DatabaseUnavailable, DatabaseUrl, MigrateError,
    MigrationFailure, NotLocal, ResetError, SeedError, SeedFailure, migrate, reset,
};
use barras_testkit::{LoopbackRelay, PostgresContainer};
use sqlx::postgres::PgConnectOptions;
use sqlx::{Connection, PgConnection};

/// What a test keeps secret; no error may show it.
const PASSWORD: &str = "hunter2-secret";

/// A folder of seed files that one test writes in the target folder, removed when it ends.
struct SeedFolder(PathBuf);

impl SeedFolder {
    fn new(test: &str) -> Self {
        let path = Path::new(env!("CARGO_TARGET_TMPDIR"))
            .join(format!("int_reset-{test}-{}", std::process::id()));
        // A run killed before its end leaves its folder behind, and process ids come back.
        if path.exists() {
            fs::remove_dir_all(&path).unwrap();
        }
        fs::create_dir_all(&path).unwrap();
        Self(path)
    }

    fn with(self, name: &str, content: impl AsRef<[u8]>) -> Self {
        fs::write(self.0.join(name), content).unwrap();
        self
    }

    fn path(&self) -> &Path {
        &self.0
    }
}

impl Drop for SeedFolder {
    fn drop(&mut self) {
        // A folder left behind only takes room in the target folder, and a test that unwinds
        // must not panic a second time.
        let _ = fs::remove_dir_all(&self.0);
    }
}

fn url_of(container: &PostgresContainer) -> DatabaseUrl {
    DatabaseUrl::parse(container.url()).unwrap()
}

/// The container's URL, naming `database` instead of `barras`.
fn url_with_database(container: &PostgresContainer, database: &str) -> String {
    let (server, _) = container.url().rsplit_once('/').unwrap();
    format!("{server}/{database}")
}

async fn connect(url: &str) -> PgConnection {
    PgConnection::connect(url).await.unwrap()
}

/// Every table of the database outside the system's schemas, as `schema.table`.
async fn tables(url: &str) -> Vec<String> {
    sqlx::query_scalar(
        "SELECT table_schema || '.' || table_name FROM information_schema.tables \
         WHERE table_schema NOT IN ('pg_catalog', 'information_schema') ORDER BY 1",
    )
    .fetch_all(&mut connect(url).await)
    .await
    .unwrap()
}

/// The files of the repository's `migrations/` folder, in name order.
fn migration_files() -> Vec<String> {
    let mut files: Vec<String> =
        fs::read_dir(Path::new(env!("CARGO_MANIFEST_DIR")).join("../../../migrations"))
            .unwrap()
            .map(|entry| entry.unwrap().file_name().into_string().unwrap())
            .filter(|name| !name.starts_with('.'))
            .collect();
    files.sort();
    files
}

fn migrations_of(applied: &[AppliedMigration]) -> Vec<&str> {
    applied.iter().map(AppliedMigration::file).collect()
}

fn seeds_of(applied: &[AppliedSeed]) -> Vec<&str> {
    applied.iter().map(AppliedSeed::file).collect()
}

/// A port of this machine where nothing listens.
fn closed_port() -> u16 {
    TcpListener::bind("127.0.0.1:0")
        .unwrap()
        .local_addr()
        .unwrap()
        .port()
}

#[tokio::test]
async fn reset_rebuilds_a_database_with_an_extra_table_and_another_session_open() {
    let container = PostgresContainer::start().await;
    let url = url_of(&container);
    // The database as a developer leaves it: migrated, with a table no migration made, and a
    // session that holds a lock in it inside a transaction, as a server under `just dev` can.
    migrate(&url).await.unwrap();
    sqlx::raw_sql(
        "CREATE TABLE public.left_over (id int); INSERT INTO public.left_over VALUES (1);",
    )
    .execute(&mut connect(container.url()).await)
    .await
    .unwrap();
    let options: PgConnectOptions = container.url().parse().unwrap();
    let mut other = PgConnection::connect_with(&options.application_name("other-session"))
        .await
        .unwrap();
    sqlx::raw_sql("BEGIN; LOCK TABLE public.left_over IN ACCESS EXCLUSIVE MODE;")
        .execute(&mut other)
        .await
        .unwrap();
    let seeds = SeedFolder::new("rebuild").with(
        "20261116000001_demo.sql",
        "CREATE TABLE barras.demo (id int PRIMARY KEY);\nINSERT INTO barras.demo VALUES (1), (2);",
    );

    let rebuilt = reset(&url, seeds.path()).await.unwrap();

    assert_eq!(rebuilt.database(), "barras");
    assert!(rebuilt.dropped());
    assert_eq!(migrations_of(rebuilt.migrations()), migration_files());
    assert_eq!(seeds_of(rebuilt.seeds()), ["20261116000001_demo.sql"]);
    assert_eq!(
        tables(container.url()).await,
        ["barras.demo", "public._sqlx_migrations"]
    );
    let ids: Vec<i32> = sqlx::query_scalar("SELECT id FROM barras.demo ORDER BY id")
        .fetch_all(&mut connect(container.url()).await)
        .await
        .unwrap();
    assert_eq!(ids, [1, 2]);
    // Ended, not waited for.
    assert!(sqlx::raw_sql("SELECT 1").execute(&mut other).await.is_err());
}

#[tokio::test]
async fn a_server_behind_a_loopback_relay_that_is_not_the_local_one_keeps_its_database() {
    // What a developer reaches staging through, with `ssh -L` or `kubectl port-forward`: a port
    // of this machine, which the guard allows, before a server that is not the local one.
    let elsewhere = PostgresContainer::start_as_another_server().await;
    sqlx::raw_sql("CREATE TABLE public.precious (id int); INSERT INTO public.precious VALUES (1);")
        .execute(&mut connect(elsewhere.url()).await)
        .await
        .unwrap();
    let relay = LoopbackRelay::start(&elsewhere).await;
    let seeds = SeedFolder::new("relay").with(
        "20261116000001_demo.sql",
        "CREATE TABLE barras.demo (id int);",
    );

    let error = reset(&DatabaseUrl::parse(relay.url()).unwrap(), seeds.path())
        .await
        .unwrap_err();

    assert_eq!(error, ResetError::NotLocal(NotLocal::OtherServer));
    let sentence = error.to_string();
    assert!(sentence.contains("cluster_name"), "{sentence}");
    assert!(!sentence.contains(relay.url()), "{sentence}");
    assert_eq!(tables(elsewhere.url()).await, ["public.precious"]);
}

#[tokio::test]
async fn reset_reaches_the_local_server_through_localhost_at_every_step() {
    // The container listens on 127.0.0.1 alone, and `localhost` may resolve to `::1` first: the
    // maintenance database, the migrations and the seeds are each reached at an address the
    // guard judged.
    let container = PostgresContainer::start().await;
    let through_localhost = container.url().replace("@127.0.0.1:", "@localhost:");
    let seeds = SeedFolder::new("localhost").with(
        "20261116000001_demo.sql",
        "CREATE TABLE barras.demo (id int);",
    );

    let rebuilt = reset(
        &DatabaseUrl::parse(&through_localhost).unwrap(),
        seeds.path(),
    )
    .await
    .unwrap();

    assert!(rebuilt.dropped());
    assert_eq!(seeds_of(rebuilt.seeds()), ["20261116000001_demo.sql"]);
    assert_eq!(
        tables(container.url()).await,
        ["barras.demo", "public._sqlx_migrations"]
    );
}

#[tokio::test]
async fn reset_creates_a_database_that_does_not_exist_yet() {
    let container = PostgresContainer::start().await;
    let fresh = url_with_database(&container, "fresh");
    let folder = SeedFolder::new("fresh");

    let rebuilt = reset(
        &DatabaseUrl::parse(&fresh).unwrap(),
        &folder.path().join("missing"),
    )
    .await
    .unwrap();

    assert_eq!(rebuilt.database(), "fresh");
    assert!(!rebuilt.dropped());
    assert_eq!(migrations_of(rebuilt.migrations()), migration_files());
    assert_eq!(rebuilt.seeds(), []);
    assert_eq!(tables(&fresh).await, ["public._sqlx_migrations"]);
}

#[tokio::test]
async fn reset_rebuilds_the_maintenance_database_itself_from_another_one() {
    let container = PostgresContainer::start().await;
    // No session can drop the database it is connected to.
    let postgres = url_with_database(&container, "postgres");
    sqlx::raw_sql("CREATE TABLE public.left_over (id int);")
        .execute(&mut connect(&postgres).await)
        .await
        .unwrap();
    let folder = SeedFolder::new("maintenance");

    let rebuilt = reset(
        &DatabaseUrl::parse(&postgres).unwrap(),
        &folder.path().join("missing"),
    )
    .await
    .unwrap();

    assert_eq!(rebuilt.database(), "postgres");
    assert!(rebuilt.dropped());
    assert_eq!(tables(&postgres).await, ["public._sqlx_migrations"]);
}

#[tokio::test]
async fn a_seed_file_that_cannot_be_read_leaves_the_database_as_it_was() {
    let container = PostgresContainer::start().await;
    sqlx::raw_sql("CREATE TABLE public.left_over (id int);")
        .execute(&mut connect(container.url()).await)
        .await
        .unwrap();
    // `é` in Latin-1, which is not UTF-8.
    let seeds = SeedFolder::new("unreadable").with("20261116000001_latin1.sql", b"SELECT '\xe9';");

    let error = reset(&url_of(&container), seeds.path()).await.unwrap_err();

    assert_eq!(
        error,
        ResetError::SeedsUnreadable(SeedError::File {
            file: "20261116000001_latin1.sql".to_owned(),
            kind: io::ErrorKind::InvalidData,
        })
    );
    assert_eq!(tables(container.url()).await, ["public.left_over"]);
}

#[tokio::test]
async fn a_failing_seed_leaves_the_database_created_again_and_migrated() {
    let container = PostgresContainer::start().await;
    sqlx::raw_sql("CREATE TABLE public.left_over (id int);")
        .execute(&mut connect(container.url()).await)
        .await
        .unwrap();
    let seeds = SeedFolder::new("failing").with(
        "20261116000001_demo.sql",
        "CREATE TABLE barras.demo (id int);\nINSERT INTO barras.nowhere VALUES (1);",
    );

    let error = reset(&url_of(&container), seeds.path()).await.unwrap_err();

    let ResetError::Seed(SeedError::Failed {
        file,
        failure: SeedFailure::Server { sqlstate, .. },
    }) = error
    else {
        panic!("{error:?}");
    };
    assert_eq!(file, "20261116000001_demo.sql");
    assert_eq!(sqlstate.as_deref(), Some("42P01"));
    assert_eq!(tables(container.url()).await, ["public._sqlx_migrations"]);
}

#[tokio::test]
async fn a_database_the_user_may_not_drop_is_left_as_it_was() {
    let container = PostgresContainer::start().await;
    // A user that owns nothing: it reaches the maintenance database, as every user may, and may
    // not drop `barras`.
    sqlx::raw_sql(
        "CREATE ROLE visitor LOGIN PASSWORD 'visitor'; CREATE TABLE public.left_over (id int);",
    )
    .execute(&mut connect(container.url()).await)
    .await
    .unwrap();
    let visitor = container
        .url()
        .replace("barras:barras@", "visitor:visitor@");
    let folder = SeedFolder::new("not-owner");

    let error = reset(&DatabaseUrl::parse(&visitor).unwrap(), folder.path())
        .await
        .unwrap_err();

    assert_eq!(
        error,
        ResetError::Drop {
            database: "barras".to_owned(),
            cause: DatabaseUnavailable::Refused {
                sqlstate: Some("42501".to_owned())
            },
        }
    );
    assert_eq!(tables(container.url()).await, ["public.left_over"]);
}

#[tokio::test]
async fn a_database_dropped_that_the_user_may_not_create_again_is_gone() {
    let container = PostgresContainer::start().await;
    let mut superuser = connect(container.url()).await;
    // A user that owns a database, so may drop it, and may not create one. `CREATE DATABASE`
    // runs alone: it refuses to run inside the transaction of several statements.
    sqlx::raw_sql("CREATE ROLE visitor LOGIN PASSWORD 'visitor'")
        .execute(&mut superuser)
        .await
        .unwrap();
    sqlx::raw_sql("CREATE DATABASE visited OWNER visitor")
        .execute(&mut superuser)
        .await
        .unwrap();
    let visited =
        url_with_database(&container, "visited").replace("barras:barras@", "visitor:visitor@");
    let folder = SeedFolder::new("not-creator");

    let error = reset(&DatabaseUrl::parse(&visited).unwrap(), folder.path())
        .await
        .unwrap_err();

    assert_eq!(
        error,
        ResetError::Create {
            database: "visited".to_owned(),
            dropped: true,
            cause: DatabaseUnavailable::Refused {
                sqlstate: Some("42501".to_owned())
            },
        }
    );
    let exists: bool =
        sqlx::query_scalar("SELECT EXISTS (SELECT FROM pg_database WHERE datname = 'visited')")
            .fetch_one(&mut superuser)
            .await
            .unwrap();
    assert!(!exists);
}

#[tokio::test]
async fn a_database_that_did_not_exist_and_may_not_be_created_is_not_said_to_be_dropped() {
    let container = PostgresContainer::start().await;
    // A user that may reach the server, and may create no database.
    sqlx::raw_sql("CREATE ROLE visitor LOGIN PASSWORD 'visitor'")
        .execute(&mut connect(container.url()).await)
        .await
        .unwrap();
    let fresh =
        url_with_database(&container, "fresh").replace("barras:barras@", "visitor:visitor@");
    let folder = SeedFolder::new("not-creator-fresh");

    let error = reset(&DatabaseUrl::parse(&fresh).unwrap(), folder.path())
        .await
        .unwrap_err();

    assert_eq!(
        error,
        ResetError::Create {
            database: "fresh".to_owned(),
            dropped: false,
            cause: DatabaseUnavailable::Refused {
                sqlstate: Some("42501".to_owned())
            },
        }
    );
    assert_eq!(
        error.to_string(),
        "cannot create the database fresh, which did not exist: the server refused it with \
         SQLSTATE 42501"
    );
}

#[tokio::test]
async fn a_failing_migration_is_not_said_to_leave_the_database_empty() {
    let container = PostgresContainer::start().await;
    // Every database created from now on holds the schema that the first migration creates, so
    // that migration fails.
    let mut template = connect(&url_with_database(&container, "template1")).await;
    sqlx::raw_sql("CREATE SCHEMA barras")
        .execute(&mut template)
        .await
        .unwrap();
    // No session may be connected to the template of a database being created.
    template.close().await.unwrap();
    let folder = SeedFolder::new("failing-migration");

    let error = reset(
        &DatabaseUrl::parse(&url_with_database(&container, "fresh")).unwrap(),
        folder.path(),
    )
    .await
    .unwrap_err();

    let ResetError::Migrate(MigrateError::Migration {
        applied,
        file,
        failure,
    }) = &error
    else {
        panic!("{error:?}");
    };
    assert!(applied.is_empty(), "{applied:?}");
    assert_eq!(file, &migration_files()[0]);
    assert_eq!(
        failure,
        &MigrationFailure::Sql {
            sqlstate: "42P06".to_owned(),
            message: "schema \"barras\" already exists".to_owned(),
        }
    );
    let sentence = error.to_string();
    assert!(!sentence.contains("empty"), "{sentence}");
    assert_eq!(
        sentence,
        format!(
            "the database was created, and keeps the migrations applied before the failure; \
             migration {file} failed with SQLSTATE 42P06: schema \"barras\" already exists"
        )
    );
}

#[tokio::test]
async fn reset_creates_and_drops_databases_whose_names_need_quoting() {
    let container = PostgresContainer::start().await;
    let folder = SeedFolder::new("quoted");
    // 63 bytes, the most Postgres keeps of a name, in 32 characters.
    let longest = format!("{}x", "ñ".repeat(31));
    let longest_encoded = format!("{}x", "%C3%B1".repeat(31));
    let names = [
        ("Barras Dev", "Barras%20Dev"),
        ("we\"ird", "we%22ird"),
        // Written to end the statement and drop another database, and one name all the same.
        (
            "x\" WITH (FORCE); DROP DATABASE postgres; --",
            "x%22%20WITH%20(FORCE)%3B%20DROP%20DATABASE%20postgres%3B%20--",
        ),
        (longest.as_str(), longest_encoded.as_str()),
    ];

    for (name, encoded) in &names {
        let url = url_with_database(&container, encoded);
        let parsed = DatabaseUrl::parse(&url).unwrap();

        let created = reset(&parsed, &folder.path().join("missing"))
            .await
            .unwrap();
        let rebuilt = reset(&parsed, &folder.path().join("missing"))
            .await
            .unwrap();

        assert_eq!(created.database(), *name);
        assert!(!created.dropped(), "{name}");
        assert!(rebuilt.dropped(), "{name}");
        assert_eq!(tables(&url).await, ["public._sqlx_migrations"], "{name}");
    }
    let databases: Vec<String> = sqlx::query_scalar(
        "SELECT datname FROM pg_database WHERE NOT datistemplate ORDER BY datname COLLATE \"C\"",
    )
    .fetch_all(&mut connect(container.url()).await)
    .await
    .unwrap();
    let mut expected: Vec<&str> = names.iter().map(|(name, _)| *name).collect();
    expected.extend(["barras", "postgres"]);
    expected.sort_unstable();
    assert_eq!(databases, expected);
}

#[tokio::test]
async fn a_server_that_does_not_answer_is_named_in_a_fixed_sentence() {
    let url = DatabaseUrl::parse(&format!(
        "postgres://barras:{PASSWORD}@127.0.0.1:{}/barras",
        closed_port()
    ))
    .unwrap();
    let folder = SeedFolder::new("unreachable");

    let error = reset(&url, folder.path()).await.unwrap_err();

    assert_eq!(
        error,
        ResetError::Unavailable(DatabaseUnavailable::ConnectionFailed(
            io::ErrorKind::ConnectionRefused
        ))
    );
    assert!(!error.to_string().contains(PASSWORD), "{error}");
}
