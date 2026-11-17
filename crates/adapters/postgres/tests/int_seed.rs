//! `seed` against a real Postgres, in a container of each test's own: the order of the files,
//! one transaction for the run, a failing file, a second run, the ways a file can end the
//! transaction, a file that leaves the connection waiting, the server that answers, and folders
//! with nothing to apply.
#![allow(clippy::unwrap_used, clippy::expect_used)] // an integration test: a step that fails must fail the test

use std::fs;
use std::io;
use std::time::{Duration, Instant};

use barras_adapter_postgres::{
    AppliedSeed, DatabaseUnavailable, DatabaseUrl, ErrorPosition, NotLocal, SeedError, SeedFailure,
    seed,
};
use barras_testkit::{LoopbackRelay, PostgresContainer, SeedFolder, closed_port};
use sqlx::{Connection, PgConnection};

/// What a test keeps secret; no error may show it.
const PASSWORD: &str = "hunter2-secret";

/// A folder of seed files that the test `test` writes in the target folder.
fn seed_folder(test: &str) -> SeedFolder {
    SeedFolder::new(env!("CARGO_TARGET_TMPDIR"), &format!("int_seed-{test}"))
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

#[tokio::test]
async fn every_sql_file_of_the_folder_applies_in_name_order_and_other_files_are_ignored() {
    let container = PostgresContainer::start().await;
    // Written out of order, as a folder lists its files in no order of its own.
    let folder = seed_folder("order")
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
    let folder = seed_folder("failing")
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
    let folder = seed_folder("twice")
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
    let folder = seed_folder("elsewhere").with(
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
    let folder = seed_folder("relayed").with(
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
    let folder = seed_folder("localhost").with(
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
    let without_seeds = seed_folder("empty").with("README.md", "The seeds come with BAR-15.");
    let missing = without_seeds.path().join("missing");

    assert_eq!(seed(&url, &missing).await, Ok(Vec::new()));
    assert_eq!(seed(&url, without_seeds.path()).await, Ok(Vec::new()));
}

#[tokio::test]
async fn a_file_that_commits_is_named_and_the_run_says_to_reset() {
    let container = PostgresContainer::start().await;
    let folder = seed_folder("commit")
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
    let folder = seed_folder("commit-then-fail")
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
        let folder = seed_folder(&format!("ending-{number}"))
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

/// Seeds a folder whose first file creates `public.first`, whose second inserts a row there and
/// ends the run's transaction as `second` does, and whose third fails, as it does when it runs.
/// The run must name the second for ending the transaction, and never say that nothing was
/// applied, since the row it committed stays.
async fn assert_named_for_ending_the_transaction(test: &str, second: &str) {
    let container = PostgresContainer::start().await;
    let folder = seed_folder(test)
        .with(
            "20261116000001_first.sql",
            "CREATE TABLE public.first (id int);",
        )
        .with("20261116000002_ends.sql", second)
        .with(
            "20261116000003_fails.sql",
            "INSERT INTO public.nowhere VALUES (3);",
        );

    let error = seed(&url_of(&container), folder.path()).await.unwrap_err();

    assert_eq!(
        error,
        SeedError::EndedTransaction {
            file: "20261116000002_ends.sql".to_owned()
        },
        "{second}"
    );
    assert!(
        !error.to_string().contains("nothing was applied"),
        "{error}"
    );
    assert_eq!(
        count(&container, "SELECT count(*) FROM public.first").await,
        1,
        "{second}"
    );
}

#[tokio::test]
async fn a_file_that_commits_then_fails_in_a_transaction_of_its_own_is_named_for_ending_the_run_s()
{
    assert_named_for_ending_the_transaction(
        "commit-begin-fail",
        "BEGIN;\nINSERT INTO public.first VALUES (1);\nCOMMIT;\nBEGIN;\n\
         INSERT INTO public.nowhere VALUES (2);",
    )
    .await;
}

#[tokio::test]
async fn a_file_that_commits_and_chains_then_fails_is_named_for_ending_the_transaction() {
    assert_named_for_ending_the_transaction(
        "commit-and-chain",
        "INSERT INTO public.first VALUES (1);\nCOMMIT AND CHAIN;\n\
         INSERT INTO public.nowhere VALUES (2);",
    )
    .await;
}

#[tokio::test]
async fn a_file_that_sets_in_a_transaction_of_its_own_what_marked_the_run_s_is_named_for_it() {
    // The run once marked its transaction with this setting, which a file can set again.
    assert_named_for_ending_the_transaction(
        "forged-local-marker",
        "INSERT INTO public.first VALUES (1);\nCOMMIT;\nBEGIN;\n\
         SELECT set_config('barras.seed_run', 'on', true);",
    )
    .await;
}

#[tokio::test]
async fn a_file_that_sets_for_the_session_what_marked_the_run_s_transaction_is_named_for_it() {
    // Set for the session, the old marker outlived the transaction that the file committed.
    assert_named_for_ending_the_transaction(
        "forged-session-marker",
        "INSERT INTO public.first VALUES (1);\nSET barras.seed_run = 'on';\nCOMMIT;",
    )
    .await;
}

#[tokio::test]
async fn a_file_that_fails_under_a_savepoint_named_as_the_run_s_in_its_own_transaction_is_named() {
    // Rolling back to the savepoint works in the file's transaction too: its id tells them apart.
    assert_named_for_ending_the_transaction(
        "forged-savepoint",
        "INSERT INTO public.first VALUES (1);\nCOMMIT;\nBEGIN;\nSAVEPOINT barras_seed_file;\n\
         INSERT INTO public.nowhere VALUES (2);",
    )
    .await;
}

#[tokio::test]
async fn a_file_may_roll_back_to_savepoints_of_its_own() {
    let container = PostgresContainer::start().await;
    let folder = seed_folder("own-savepoints")
        .with(
            "20261116000001_first.sql",
            "CREATE TABLE public.first (id int);\nSAVEPOINT mine;\n\
             INSERT INTO public.first VALUES (1);\nROLLBACK TO SAVEPOINT mine;\n\
             INSERT INTO public.first VALUES (2);",
        )
        .with(
            "20261116000002_second.sql",
            "INSERT INTO public.first VALUES (3);",
        );

    let applied = seed(&url_of(&container), folder.path()).await.unwrap();

    assert_eq!(
        files(&applied),
        ["20261116000001_first.sql", "20261116000002_second.sql"]
    );
    let ids: Vec<i32> = sqlx::query_scalar("SELECT id FROM public.first ORDER BY id")
        .fetch_all(&mut connect(&container).await)
        .await
        .unwrap();
    assert_eq!(ids, [2, 3]);
}

#[tokio::test]
async fn a_file_that_leaves_the_connection_waiting_for_copy_data_fails_at_once() {
    // `COPY … FROM stdin` waits for data that the run never sends, and so would any question the
    // run asked after it.
    let container = PostgresContainer::start().await;
    let file = "CREATE TABLE public.copied (id int);\nCOPY public.copied FROM stdin;";
    let folder = seed_folder("copy").with("20261116000001_copy.sql", file);

    let error = tokio::time::timeout(
        Duration::from_secs(5),
        seed(&url_of(&container), folder.path()),
    )
    .await
    .expect("the run waited on its connection")
    .unwrap_err();

    assert_eq!(
        error,
        SeedError::Failed {
            file: "20261116000001_copy.sql".to_owned(),
            failure: SeedFailure::Unavailable(DatabaseUnavailable::ProtocolViolated),
        }
    );
    // The run cannot tell whether the file ended its transaction before the COPY.
    assert!(
        !error.to_string().contains("nothing was applied"),
        "{error}"
    );
    // Closed: the session that ran the file ends, and its transaction with it.
    let deadline = Instant::now() + Duration::from_secs(5);
    while count_sessions_running(&container, file).await != 0 {
        assert!(
            Instant::now() < deadline,
            "the session of the run outlived it"
        );
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
    assert!(!table_exists(&container, "copied").await);
}

#[tokio::test]
async fn a_file_that_ends_its_own_session_after_a_commit_is_never_said_to_have_applied_nothing() {
    let container = PostgresContainer::start().await;
    let folder = seed_folder("terminate")
        .with(
            "20261116000001_first.sql",
            "CREATE TABLE public.first (id int);",
        )
        .with(
            "20261116000002_ends.sql",
            "INSERT INTO public.first VALUES (1);\nCOMMIT;\n\
             SELECT pg_terminate_backend(pg_backend_pid());",
        );

    let error = seed(&url_of(&container), folder.path()).await.unwrap_err();

    // The session is gone, and the run cannot ask what became of its transaction.
    let SeedError::Failed {
        file,
        failure: SeedFailure::Unavailable(_),
    } = &error
    else {
        panic!("{error:?}");
    };
    assert_eq!(file, "20261116000002_ends.sql");
    assert!(
        !error.to_string().contains("nothing was applied"),
        "{error}"
    );
    assert_eq!(
        count(&container, "SELECT count(*) FROM public.first").await,
        1
    );
}

/// The sessions whose last statement is `sql`, as Postgres counts them.
async fn count_sessions_running(container: &PostgresContainer, sql: &str) -> i64 {
    sqlx::query_scalar("SELECT count(*) FROM pg_stat_activity WHERE query = $1")
        .bind(sql)
        .fetch_one(&mut connect(container).await)
        .await
        .unwrap()
}

#[tokio::test]
async fn a_position_after_characters_of_several_bytes_counts_characters() {
    let container = PostgresContainer::start().await;
    // `🦀` takes four bytes and `ñ` two; the server counts each as one character.
    let folder = seed_folder("multibyte").with(
        "20261116000001_crabs.sql",
        "SELECT '🦀🦀ñ' AS text; SELEC 1;",
    );

    let error = seed(&url_of(&container), folder.path()).await.unwrap_err();

    let SeedError::Failed {
        failure: SeedFailure::Server {
            sqlstate, position, ..
        },
        ..
    } = error
    else {
        panic!("{error:?}");
    };
    assert_eq!(sqlstate.as_deref(), Some("42601"));
    assert_eq!(
        position,
        Some(ErrorPosition {
            line: 1,
            column: 23
        })
    );
}

#[tokio::test]
async fn a_constraint_checked_at_commit_fails_the_run_and_leaves_nothing() {
    let container = PostgresContainer::start().await;
    let folder = seed_folder("deferred").with(
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
    let folder = seed_folder("latin1")
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
    let folder = seed_folder("unreachable").with("20261116000001_first.sql", "SELECT 1;");

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
    let folder = seed_folder("password").with("20261116000001_first.sql", "SELECT 1;");

    let error = seed(&DatabaseUrl::parse(&wrong).unwrap(), folder.path())
        .await
        .unwrap_err();

    assert_eq!(
        error.to_string(),
        "cannot seed the database: the server refused it with SQLSTATE 28P01"
    );
    assert!(!format!("{error:?}").contains(PASSWORD));
}
