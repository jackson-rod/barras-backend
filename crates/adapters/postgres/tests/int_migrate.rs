//! `migrate` against a real Postgres, in a container of each test's own: what it applies, what
//! it refuses before applying anything, and that a refusal leaves no lock behind.
#![allow(clippy::unwrap_used, clippy::expect_used)] // an integration test: a step that fails must fail the test

use std::borrow::Cow;
use std::fs;
use std::path::Path;
use std::time::Duration;

use barras_adapter_postgres::{
    DatabaseUnavailable, DatabaseUrl, MigrateError, MigrationFailure, migrate, migrate_with,
};
use barras_testkit::{PostgresContainer, closed_port};
use sqlx::migrate::{Migration, MigrationType, Migrator};
use sqlx::{Connection, PgConnection, SqlSafeStr};

/// What a test keeps secret; no error may show it.
const PASSWORD: &str = "hunter2-secret";

/// Migrations of a test's own, each creating what its SQL says, described as `probe <version>`.
fn migrations(list: &[(i64, &'static str)]) -> Migrator {
    Migrator::with_migrations(
        list.iter()
            .map(|&(version, sql)| {
                Migration::new(
                    version,
                    Cow::Owned(format!("probe {version}")),
                    MigrationType::Simple,
                    sql.into_sql_str(),
                    false,
                )
            })
            .collect(),
    )
}

fn url_of(container: &PostgresContainer) -> DatabaseUrl {
    DatabaseUrl::parse(container.url()).unwrap()
}

async fn connect(container: &PostgresContainer) -> PgConnection {
    PgConnection::connect(container.url()).await.unwrap()
}

async fn table_exists(container: &PostgresContainer, table: &str) -> bool {
    sqlx::query_scalar("SELECT to_regclass($1) IS NOT NULL")
        .bind(format!("public.{table}"))
        .fetch_one(&mut connect(container).await)
        .await
        .unwrap()
}

/// The versions sqlx's history records, in order.
async fn history(container: &PostgresContainer) -> Vec<i64> {
    sqlx::query_scalar("SELECT version FROM public._sqlx_migrations ORDER BY version")
        .fetch_all(&mut connect(container).await)
        .await
        .unwrap()
}

#[tokio::test]
async fn every_migration_of_the_repository_is_applied_and_named() {
    let container = PostgresContainer::start().await;
    let mut files: Vec<String> =
        fs::read_dir(Path::new(env!("CARGO_MANIFEST_DIR")).join("../../../migrations"))
            .unwrap()
            .map(|entry| entry.unwrap().file_name().into_string().unwrap())
            .filter(|name| !name.starts_with('.'))
            .collect();
    files.sort();

    let applied = migrate(&url_of(&container)).await.unwrap();

    let applied: Vec<&str> = applied.iter().map(|migration| migration.file()).collect();
    assert_eq!(applied, files);
    let schemas: i64 =
        sqlx::query_scalar("SELECT count(*) FROM pg_namespace WHERE nspname = 'barras'")
            .fetch_one(&mut connect(&container).await)
            .await
            .unwrap();
    assert_eq!(schemas, 1);
}

#[tokio::test]
async fn a_second_run_has_nothing_to_apply() {
    let container = PostgresContainer::start().await;
    let url = url_of(&container);
    migrate(&url).await.unwrap();

    let applied = migrate(&url).await.unwrap();

    assert_eq!(applied, []);
}

#[tokio::test]
async fn an_edited_migration_is_refused_before_anything_is_applied() {
    let container = PostgresContainer::start().await;
    let url = url_of(&container);
    let first = migrations(&[
        (10, "CREATE TABLE public.probe_10 (id int)"),
        (20, "CREATE TABLE public.probe_20 (id int)"),
    ]);
    migrate_with(&url, &first).await.unwrap();
    // 15 is pending and older than the edited 20: sqlx's own run applies it before it sees the
    // edit (plan, appendix A.3).
    let edited = migrations(&[
        (10, "CREATE TABLE public.probe_10 (id int)"),
        (15, "CREATE TABLE public.probe_15 (id int)"),
        (20, "CREATE TABLE public.probe_20 (id bigint)"),
    ]);

    let refused = migrate_with(&url, &edited).await;

    assert_eq!(refused, Err(MigrateError::Edited { version: 20 }));
    assert!(!table_exists(&container, "probe_15").await);
    assert_eq!(history(&container).await, [10, 20]);
}

#[tokio::test]
async fn an_applied_migration_the_binary_lacks_is_refused_before_anything_is_applied() {
    let container = PostgresContainer::start().await;
    let url = url_of(&container);
    migrate_with(
        &url,
        &migrations(&[
            (10, "CREATE TABLE public.probe_10 (id int)"),
            (20, "CREATE TABLE public.probe_20 (id int)"),
        ]),
    )
    .await
    .unwrap();
    let without_20 = migrations(&[
        (10, "CREATE TABLE public.probe_10 (id int)"),
        (30, "CREATE TABLE public.probe_30 (id int)"),
    ]);

    let refused = migrate_with(&url, &without_20).await;

    assert_eq!(refused, Err(MigrateError::Missing { version: 20 }));
    assert!(!table_exists(&container, "probe_30").await);
    assert_eq!(history(&container).await, [10, 20]);
}

#[tokio::test]
async fn a_migration_recorded_as_failed_is_refused_before_anything_is_applied() {
    let container = PostgresContainer::start().await;
    let url = url_of(&container);
    migrate_with(
        &url,
        &migrations(&[(10, "CREATE TABLE public.probe_10 (id int)")]),
    )
    .await
    .unwrap();
    // sqlx 0.9.0 only ever inserts `success = TRUE`, so no run of it writes this row: it stands
    // for one that a person or another tool wrote, which a run must refuse all the same.
    sqlx::query(
        "INSERT INTO public._sqlx_migrations \
         (version, description, success, checksum, execution_time) \
         VALUES (20, 'probe 20', false, '\\x00', -1)",
    )
    .execute(&mut connect(&container).await)
    .await
    .unwrap();
    let next = migrations(&[
        (10, "CREATE TABLE public.probe_10 (id int)"),
        (20, "CREATE TABLE public.probe_20 (id int)"),
        (30, "CREATE TABLE public.probe_30 (id int)"),
    ]);

    let refused = migrate_with(&url, &next).await;

    assert_eq!(refused, Err(MigrateError::Failed { version: 20 }));
    assert!(!table_exists(&container, "probe_30").await);
}

#[tokio::test]
async fn a_run_after_a_refusal_is_not_held_up_by_the_lock_of_the_refused_one() {
    let container = PostgresContainer::start().await;
    let url = url_of(&container);
    let first = migrations(&[(10, "CREATE TABLE public.probe_10 (id int)")]);
    migrate_with(&url, &first).await.unwrap();
    let edited = migrations(&[(10, "CREATE TABLE public.probe_10 (id bigint)")]);
    assert_eq!(
        migrate_with(&url, &edited).await,
        Err(MigrateError::Edited { version: 10 })
    );

    // sqlx's advisory lock lasts as long as the session that took it.
    let next = tokio::time::timeout(Duration::from_secs(10), migrate_with(&url, &first)).await;

    assert_eq!(next.expect("the run waited for a lock"), Ok(Vec::new()));
}

#[tokio::test]
async fn a_failing_migration_is_named_with_the_server_message_and_the_ones_before_it_stay() {
    let container = PostgresContainer::start().await;
    let failing = migrations(&[
        (10, "CREATE TABLE public.probe_10 (id int)"),
        (20, "CREATE TABLE public.probe_20 (id int"),
        (30, "CREATE TABLE public.probe_30 (id int)"),
    ]);

    let error = migrate_with(&url_of(&container), &failing)
        .await
        .unwrap_err();

    let MigrateError::Migration {
        applied,
        file,
        failure,
    } = error
    else {
        panic!("{error:?}");
    };
    let applied: Vec<&str> = applied.iter().map(|migration| migration.file()).collect();
    assert_eq!(applied, ["10_probe_10.sql"]);
    assert_eq!(file, "20_probe_20.sql");
    assert_eq!(
        failure,
        MigrationFailure::Sql {
            sqlstate: "42601".to_owned(),
            message: "syntax error at end of input".to_owned(),
        }
    );
    assert!(table_exists(&container, "probe_10").await);
    assert!(!table_exists(&container, "probe_20").await);
    assert_eq!(history(&container).await, [10]);
}

#[tokio::test]
async fn a_failing_migration_whose_message_would_quote_a_table_s_data_is_named_by_its_sqlstate() {
    let container = PostgresContainer::start().await;
    let failing = migrations(&[
        (
            10,
            "CREATE TABLE public.probe (value text); \
             INSERT INTO public.probe VALUES ('hunter2-secret')",
        ),
        // The server's message quotes the value that does not convert.
        (
            20,
            "ALTER TABLE public.probe ALTER COLUMN value TYPE int USING value::int",
        ),
    ]);

    let error = migrate_with(&url_of(&container), &failing)
        .await
        .unwrap_err();

    let MigrateError::Migration { file, failure, .. } = &error else {
        panic!("{error:?}");
    };
    assert_eq!(file, "20_probe_20.sql");
    assert_eq!(
        failure,
        &MigrationFailure::Server {
            sqlstate: Some("22P02".to_owned())
        }
    );
    assert_eq!(
        error.to_string(),
        "migration 20_probe_20.sql failed with SQLSTATE 22P02"
    );
    assert!(!format!("{error:?}").contains(PASSWORD), "{error:?}");
}

#[tokio::test]
async fn a_migration_runs_without_the_statement_timeout_of_the_pools() {
    let container = PostgresContainer::start().await;
    let probe = migrations(&[(
        10,
        "CREATE TABLE public.probe AS SELECT current_setting('statement_timeout') AS value",
    )]);

    migrate_with(&url_of(&container), &probe).await.unwrap();

    let timeout: String = sqlx::query_scalar("SELECT value FROM public.probe")
        .fetch_one(&mut connect(&container).await)
        .await
        .unwrap();
    assert_eq!(timeout, "0");
}

#[tokio::test]
async fn a_database_that_does_not_answer_is_named_in_a_fixed_sentence() {
    let url = format!(
        "postgres://barras:{PASSWORD}@127.0.0.1:{}/barras",
        closed_port()
    );

    let error = migrate(&DatabaseUrl::parse(&url).unwrap())
        .await
        .unwrap_err();

    assert_eq!(
        error,
        MigrateError::Unavailable(DatabaseUnavailable::ConnectionFailed(
            std::io::ErrorKind::ConnectionRefused
        ))
    );
    assert_eq!(
        error.to_string(),
        "cannot connect to the database: the connection failed: connection refused"
    );
}

#[tokio::test]
async fn a_refused_password_is_named_by_its_sqlstate_alone() {
    let container = PostgresContainer::start().await;
    let wrong = container
        .url()
        .replace("barras:barras@", &format!("barras:{PASSWORD}@"));

    let error = migrate(&DatabaseUrl::parse(&wrong).unwrap())
        .await
        .unwrap_err();

    assert_eq!(
        error.to_string(),
        "cannot connect to the database: the server refused it with SQLSTATE 28P01"
    );
}
