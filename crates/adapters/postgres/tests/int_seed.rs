//! `seed` against a real Postgres, in a container of each test's own: the order of the files,
//! one transaction for the run, a failing file, a second run, a file that ends the transaction,
//! and folders with nothing to apply.
#![allow(clippy::unwrap_used, clippy::expect_used)] // an integration test: a step that fails must fail the test

use std::fs;
use std::io;
use std::net::TcpListener;
use std::path::{Path, PathBuf};

use barras_adapter_postgres::{
    AppliedSeed, DatabaseUnavailable, DatabaseUrl, ErrorPosition, NotLocal, SeedError, SeedFailure,
    seed,
};
use barras_testkit::{LoopbackRelay, PostgresContainer};
use sqlx::{Connection, PgConnection};

/// What a test keeps secret; no error may show it.
const PASSWORD: &str = "hunter2-secret";

/// A folder of seed files that one test writes in the target folder, removed when it ends.
struct SeedFolder(PathBuf);

impl SeedFolder {
    fn new(test: &str) -> Self {
        let path = Path::new(env!("CARGO_TARGET_TMPDIR"))
            .join(format!("int_seed-{test}-{}", std::process::id()));
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

async fn connect(container: &PostgresContainer) -> PgConnection {
    PgConnection::connect(container.url()).await.unwrap()
}

async fn execute(container: &PostgresContainer, sql: &'static str) {
    sqlx::raw_sql(sql)
        .execute(&mut connect(container).await)
        .await
        .unwrap();
}

async fn table_exists(container: &PostgresContainer, table: &str) -> bool {
    sqlx::query_scalar("SELECT to_regclass($1) IS NOT NULL")
        .bind(format!("public.{table}"))
        .fetch_one(&mut connect(container).await)
        .await
        .unwrap()
}

async fn count(container: &PostgresContainer, sql: &'static str) -> i64 {
    sqlx::query_scalar(sql)
        .fetch_one(&mut connect(container).await)
        .await
        .unwrap()
}

fn files(applied: &[AppliedSeed]) -> Vec<&str> {
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
async fn every_sql_file_of_the_folder_applies_in_name_order_and_other_files_are_ignored() {
    let container = PostgresContainer::start().await;
    // Written out of order, as a folder lists its files in no order of its own.
    let folder = SeedFolder::new("order")
        .with(
            "20261116000003_third.sql",
            "INSERT INTO public.ran (file) VALUES ('third');",
        )
        .with(
            "20261116000001_first.sql",
            "CREATE TABLE public.ran (step serial PRIMARY KEY, file text NOT NULL);\n\
             INSERT INTO public.ran (file) VALUES ('first');",
        )
        .with(
            "20261116000002_second.sql",
            "INSERT INTO public.ran (file) VALUES ('second');",
        )
        // Nothing to run in it, as in a seed whose statements are all commented out.
        .with("20261116000004_empty.sql", "")
        .with(
            "README.md",
            "INSERT INTO public.ran (file) VALUES ('readme');",
        )
        .with(
            "20261116000005_backup.sql.bak",
            "INSERT INTO public.ran (file) VALUES ('backup');",
        );
    fs::create_dir(folder.path().join("20261116000006_folder.sql")).unwrap();

    let applied = seed(&url_of(&container), folder.path()).await.unwrap();

    assert_eq!(
        files(&applied),
        [
            "20261116000001_first.sql",
            "20261116000002_second.sql",
            "20261116000003_third.sql",
            "20261116000004_empty.sql",
        ]
    );
    let ran: Vec<String> = sqlx::query_scalar("SELECT file FROM public.ran ORDER BY step")
        .fetch_all(&mut connect(&container).await)
        .await
        .unwrap();
    assert_eq!(ran, ["first", "second", "third"]);
}

#[tokio::test]
async fn a_failing_file_leaves_nothing_of_the_files_before_it_and_is_named_with_its_line() {
    let container = PostgresContainer::start().await;
    let folder = SeedFolder::new("failing")
        .with(
            "20261116000001_first.sql",
            "CREATE TABLE public.first (id int PRIMARY KEY);\nINSERT INTO public.first VALUES (1);",
        )
        .with(
            "20261116000002_second.sql",
            "INSERT INTO public.first VALUES (2);\n\nINSERT INTO public.nowhere VALUES (3);",
        )
        .with(
            "20261116000003_third.sql",
            "CREATE TABLE public.third (id int);",
        );

    let error = seed(&url_of(&container), folder.path()).await.unwrap_err();

    assert_eq!(
        error,
        SeedError::Failed {
            file: "20261116000002_second.sql".to_owned(),
            failure: SeedFailure::Server {
                sqlstate: Some("42P01".to_owned()),
                message: "relation \"public.nowhere\" does not exist".to_owned(),
                position: Some(ErrorPosition {
                    line: 3,
                    column: 13
                }),
            },
        }
    );
    // Not sqlx's text, whose "at line N" is a line of the server's C source.
    assert_eq!(
        error.to_string(),
        "seed 20261116000002_second.sql failed with SQLSTATE 42P01 at line 3, column 13: \
         relation \"public.nowhere\" does not exist; nothing was applied"
    );
    assert!(!table_exists(&container, "first").await);
    assert!(!table_exists(&container, "third").await);
}

#[tokio::test]
async fn a_second_run_on_a_seeded_database_fails_and_changes_nothing() {
    let container = PostgresContainer::start().await;
    let url = url_of(&container);
    // The tables a migration would have created: seeds assume a database just migrated.
    execute(
        &container,
        "CREATE TABLE public.visits (id serial PRIMARY KEY);
         CREATE TABLE public.demo (id int PRIMARY KEY, name text NOT NULL);",
    )
    .await;
    // The first file succeeds on every run; the second conflicts with what the first run left.
    let folder = SeedFolder::new("twice")
        .with(
            "20261116000001_visit.sql",
            "INSERT INTO public.visits DEFAULT VALUES;",
        )
        .with(
            "20261116000002_demo.sql",
            "INSERT INTO public.demo VALUES (1, 'demo');",
        );
    seed(&url, folder.path()).await.unwrap();

    let error = seed(&url, folder.path()).await.unwrap_err();

    let SeedError::Failed { file, failure } = error else {
        panic!("{error:?}");
    };
    assert_eq!(file, "20261116000002_demo.sql");
    let SeedFailure::Server { sqlstate, .. } = failure else {
        panic!("{failure:?}");
    };
    assert_eq!(sqlstate.as_deref(), Some("23505"));
    assert_eq!(
        count(&container, "SELECT count(*) FROM public.visits").await,
        1
    );
    assert_eq!(
        count(&container, "SELECT count(*) FROM public.demo").await,
        1
    );
}

#[tokio::test]
async fn a_server_that_is_not_the_local_one_is_refused_behind_a_loopback_relay_or_on_its_own_port()
{
    let elsewhere = PostgresContainer::start_as_another_server().await;
    let relay = LoopbackRelay::start(&elsewhere).await;
    let folder = SeedFolder::new("elsewhere").with(
        "20261116000001_demo.sql",
        "CREATE TABLE public.demo (id int);",
    );

    for url in [relay.url(), elsewhere.url()] {
        let refused = seed(&DatabaseUrl::parse(url).unwrap(), folder.path()).await;

        assert_eq!(refused, Err(SeedError::NotLocal(NotLocal::OtherServer)));
    }
    assert!(!table_exists(&elsewhere, "demo").await);
}

#[tokio::test]
async fn a_relay_to_the_local_server_is_allowed() {
    // The guard judges the server that answers, not the way to it.
    let container = PostgresContainer::start().await;
    let relay = LoopbackRelay::start(&container).await;
    let folder = SeedFolder::new("relayed").with(
        "20261116000001_demo.sql",
        "CREATE TABLE public.demo (id int);",
    );

    let applied = seed(&DatabaseUrl::parse(relay.url()).unwrap(), folder.path())
        .await
        .unwrap();

    assert_eq!(files(&applied), ["20261116000001_demo.sql"]);
    assert!(table_exists(&container, "demo").await);
}

#[tokio::test]
async fn through_localhost_the_seeds_reach_the_server_at_an_address_it_resolves_to() {
    // The container listens on 127.0.0.1 alone, and `localhost` may resolve to `::1` first: the
    // run tries the addresses the guard judged in turn, as a connection to the name would.
    let container = PostgresContainer::start().await;
    let through_localhost = container.url().replace("@127.0.0.1:", "@localhost:");
    let folder = SeedFolder::new("localhost").with(
        "20261116000001_demo.sql",
        "CREATE TABLE public.demo (id int);",
    );

    let applied = seed(
        &DatabaseUrl::parse(&through_localhost).unwrap(),
        folder.path(),
    )
    .await
    .unwrap();

    assert_eq!(files(&applied), ["20261116000001_demo.sql"]);
    assert!(table_exists(&container, "demo").await);
}

#[tokio::test]
async fn a_missing_folder_or_one_without_seed_files_applies_nothing_and_connects_to_nothing() {
    // Nothing listens at this port: with nothing to apply, the run never connects.
    let url = DatabaseUrl::parse(&format!(
        "postgres://barras:{PASSWORD}@127.0.0.1:{}/barras",
        closed_port()
    ))
    .unwrap();
    let without_seeds = SeedFolder::new("empty").with("README.md", "The seeds come with BAR-15.");
    let missing = without_seeds.path().join("missing");

    assert_eq!(seed(&url, &missing).await, Ok(Vec::new()));
    assert_eq!(seed(&url, without_seeds.path()).await, Ok(Vec::new()));
}

#[tokio::test]
async fn a_file_that_commits_is_named_and_the_run_says_to_reset() {
    let container = PostgresContainer::start().await;
    let folder = SeedFolder::new("commit")
        .with(
            "20261116000001_first.sql",
            "CREATE TABLE public.first (id int);",
        )
        .with(
            "20261116000002_commits.sql",
            "INSERT INTO public.first VALUES (1);\nCOMMIT;",
        )
        .with(
            "20261116000003_third.sql",
            "CREATE TABLE public.third (id int);",
        );

    let error = seed(&url_of(&container), folder.path()).await.unwrap_err();

    assert_eq!(
        error,
        SeedError::EndedTransaction {
            file: "20261116000002_commits.sql".to_owned()
        }
    );
    assert!(error.to_string().ends_with("run just db-reset"), "{error}");
    // What ran up to the COMMIT stayed, which is why the run has to start again.
    assert!(table_exists(&container, "first").await);
    assert!(!table_exists(&container, "third").await);
}

#[tokio::test]
async fn a_file_that_commits_and_then_fails_is_named_for_ending_the_transaction() {
    let container = PostgresContainer::start().await;
    let folder = SeedFolder::new("commit-then-fail")
        .with(
            "20261116000001_first.sql",
            "CREATE TABLE public.first (id int);",
        )
        .with(
            "20261116000002_commits.sql",
            "COMMIT;\nINSERT INTO public.nowhere VALUES (1);",
        );

    let error = seed(&url_of(&container), folder.path()).await.unwrap_err();

    // Its failure rolled back nothing that ran before the COMMIT.
    assert_eq!(
        error,
        SeedError::EndedTransaction {
            file: "20261116000002_commits.sql".to_owned()
        }
    );
    assert!(table_exists(&container, "first").await);
}

#[tokio::test]
async fn a_file_that_ends_the_transaction_in_any_other_way_is_named_too() {
    let container = PostgresContainer::start().await;
    for (number, ending) in [
        (1, "ROLLBACK;"),
        // A transaction open again is not the run's.
        (2, "COMMIT;\nBEGIN;"),
    ] {
        let folder = SeedFolder::new(&format!("ending-{number}"))
            .with("20261116000001_ends.sql", format!("SELECT 1;\n{ending}"));

        let error = seed(&url_of(&container), folder.path()).await.unwrap_err();

        assert_eq!(
            error,
            SeedError::EndedTransaction {
                file: "20261116000001_ends.sql".to_owned()
            },
            "{ending}"
        );
    }
}

#[tokio::test]
async fn a_constraint_checked_at_commit_fails_the_run_and_leaves_nothing() {
    let container = PostgresContainer::start().await;
    let folder = SeedFolder::new("deferred").with(
        "20261116000001_orphan.sql",
        "CREATE TABLE public.parent (id int PRIMARY KEY);
         CREATE TABLE public.child (
             parent int REFERENCES public.parent DEFERRABLE INITIALLY DEFERRED
         );
         INSERT INTO public.child VALUES (1);",
    );

    let error = seed(&url_of(&container), folder.path()).await.unwrap_err();

    let SeedError::Commit(SeedFailure::Server {
        sqlstate, position, ..
    }) = error
    else {
        panic!("{error:?}");
    };
    assert_eq!(sqlstate.as_deref(), Some("23503"));
    assert_eq!(position, None);
    assert!(!table_exists(&container, "parent").await);
}

#[tokio::test]
async fn a_file_that_cannot_be_read_stops_the_run_before_it_connects() {
    // Nothing listens at this port: a run that connected would fail with another error.
    let url = DatabaseUrl::parse(&format!(
        "postgres://barras:{PASSWORD}@127.0.0.1:{}/barras",
        closed_port()
    ))
    .unwrap();
    let folder = SeedFolder::new("latin1")
        .with("20261116000001_first.sql", "SELECT 1;")
        // `é` in Latin-1, which is not UTF-8.
        .with("20261116000002_latin1.sql", b"SELECT '\xe9';");

    let error = seed(&url, folder.path()).await.unwrap_err();

    assert_eq!(
        error,
        SeedError::File {
            file: "20261116000002_latin1.sql".to_owned(),
            kind: io::ErrorKind::InvalidData,
        }
    );
}

#[tokio::test]
async fn a_database_that_does_not_answer_is_named_in_a_fixed_sentence() {
    let url = DatabaseUrl::parse(&format!(
        "postgres://barras:{PASSWORD}@127.0.0.1:{}/barras",
        closed_port()
    ))
    .unwrap();
    let folder = SeedFolder::new("unreachable").with("20261116000001_first.sql", "SELECT 1;");

    let error = seed(&url, folder.path()).await.unwrap_err();

    assert_eq!(
        error,
        SeedError::Unavailable(DatabaseUnavailable::ConnectionFailed(
            io::ErrorKind::ConnectionRefused
        ))
    );
    assert_eq!(
        error.to_string(),
        "cannot seed the database: the connection failed: connection refused"
    );
}

#[tokio::test]
async fn a_refused_password_is_named_by_its_sqlstate_alone() {
    let container = PostgresContainer::start().await;
    let wrong = container
        .url()
        .replace("barras:barras@", &format!("barras:{PASSWORD}@"));
    let folder = SeedFolder::new("password").with("20261116000001_first.sql", "SELECT 1;");

    let error = seed(&DatabaseUrl::parse(&wrong).unwrap(), folder.path())
        .await
        .unwrap_err();

    assert_eq!(
        error.to_string(),
        "cannot seed the database: the server refused it with SQLSTATE 28P01"
    );
    assert!(!format!("{error:?}").contains(PASSWORD));
}
