//! The pools and their readiness checks against a real Postgres, in a container of each test's
//! own: what each connection tells Postgres, the statement timeout, the checks up and down, the
//! statements logged, and the pools closed.
#![allow(clippy::unwrap_used, clippy::expect_used)] // an integration test: a step that fails must fail the test

use std::io::{self, Write};
use std::net::TcpListener;
use std::num::NonZeroU32;
use std::sync::{Arc, Mutex, PoisonError};
use std::time::{Duration, Instant};

use barras_adapter_postgres::{DatabaseUnavailable, DatabaseUrl, Pools};
use barras_testkit::PostgresContainer;
use serde_json::Value;
use sqlx::{Connection, PgConnection};

/// What a test keeps secret; no error may show it.
const PASSWORD: &str = "hunter2-secret";

/// How a test's connections name themselves, so that `pg_stat_activity` tells them apart.
const APPLICATION: &str = "barras-api";

fn pools_for(primary: &str, read: &str) -> Pools {
    Pools::new(
        APPLICATION,
        &DatabaseUrl::parse(primary).unwrap(),
        &DatabaseUrl::parse(read).unwrap(),
        NonZeroU32::new(4).unwrap(),
    )
}

fn pools_of(container: &PostgresContainer) -> Pools {
    pools_for(container.url(), container.url())
}

/// The connections that name themselves `barras-api`, as Postgres counts them.
async fn named_connections(container: &PostgresContainer) -> i64 {
    sqlx::query_scalar("SELECT count(*) FROM pg_stat_activity WHERE application_name = $1")
        .bind(APPLICATION)
        .fetch_one(&mut PgConnection::connect(container.url()).await.unwrap())
        .await
        .unwrap()
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
async fn a_pool_opens_its_first_connection_when_a_check_needs_it() {
    let container = PostgresContainer::start().await;
    let pools = pools_of(&container);
    assert_eq!(named_connections(&container).await, 0);

    pools.check_primary().await.unwrap();

    assert_eq!(pools.primary().size(), 1);
    assert_eq!(named_connections(&container).await, 1);
}

#[tokio::test]
async fn both_checks_pass_against_a_database_that_answers() {
    let container = PostgresContainer::start().await;
    // Another spelling of the same database, so that each role has a pool of its own.
    let read_url = container.url().replace("postgres://", "postgresql://");
    let pools = pools_for(container.url(), &read_url);
    assert!(!pools.read_is_primary());

    assert_eq!(pools.check_primary().await, Ok(()));
    assert_eq!(pools.check_read().await, Ok(()));

    assert_eq!(pools.primary().size(), 1);
    assert_eq!(pools.read().size(), 1);
}

#[tokio::test]
async fn with_equal_urls_the_read_check_goes_through_the_primary_pool() {
    let container = PostgresContainer::start().await;
    let pools = pools_of(&container);

    pools.check_read().await.unwrap();

    assert!(pools.read_is_primary());
    assert_eq!(pools.primary().size(), 1);
    assert_eq!(named_connections(&container).await, 1);
}

#[tokio::test]
async fn each_connection_names_itself_and_carries_the_statement_timeout() {
    let container = PostgresContainer::start().await;
    let pools = pools_of(&container);

    let (name, timeout): (String, String) = sqlx::query_as(
        "SELECT current_setting('application_name'), current_setting('statement_timeout')",
    )
    .fetch_one(pools.primary())
    .await
    .unwrap();

    assert_eq!(name, APPLICATION);
    assert_eq!(timeout, "5s");
}

#[tokio::test]
async fn a_statement_longer_than_five_seconds_is_cancelled_by_postgres() {
    let container = PostgresContainer::start().await;
    let pools = pools_of(&container);
    let started = Instant::now();

    let error = sqlx::query("SELECT pg_sleep(6)")
        .execute(pools.primary())
        .await
        .unwrap_err();

    let elapsed = started.elapsed();
    assert_eq!(
        DatabaseUnavailable::from(&error),
        DatabaseUnavailable::Refused {
            sqlstate: Some("57014".to_owned())
        }
    );
    assert!(
        elapsed >= Duration::from_secs(5) && elapsed < Duration::from_secs(6),
        "{elapsed:?}"
    );
}

#[tokio::test]
async fn a_check_against_a_closed_port_fails_once_the_two_seconds_have_passed() {
    // sqlx retries a refused connection until the acquire timeout.
    let url = format!(
        "postgres://barras:{PASSWORD}@127.0.0.1:{}/barras",
        closed_port()
    );
    let pools = pools_for(&url, &url);
    let started = Instant::now();

    let checked = pools.check_primary().await;

    let elapsed = started.elapsed();
    assert_eq!(checked, Err(DatabaseUnavailable::NoConnectionInTime));
    assert_eq!(
        checked.unwrap_err().to_string(),
        "no connection within 2 seconds"
    );
    assert!(
        elapsed >= Duration::from_secs(2) && elapsed < Duration::from_secs(3),
        "{elapsed:?}"
    );
}

#[tokio::test]
async fn a_refused_password_fails_the_check_with_its_sqlstate_and_no_credential() {
    let container = PostgresContainer::start().await;
    let wrong = container
        .url()
        .replace("barras:barras@", &format!("barras:{PASSWORD}@"));
    let pools = pools_for(&wrong, &wrong);

    let error = pools.check_read().await.unwrap_err();

    let sentence = error.to_string();
    assert_eq!(sentence, "the server refused it with SQLSTATE 28P01");
    assert!(!format!("{error:?}").contains(PASSWORD));
}

#[tokio::test]
async fn close_ends_every_connection_and_the_checks_after_it_fail() {
    let container = PostgresContainer::start().await;
    let pools = pools_of(&container);
    pools.check_primary().await.unwrap();
    assert_eq!(named_connections(&container).await, 1);

    pools.close().await;

    // Postgres takes a moment to notice that a connection has ended.
    let deadline = Instant::now() + Duration::from_secs(5);
    while named_connections(&container).await != 0 {
        assert!(Instant::now() < deadline, "a connection outlived close");
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
    assert_eq!(
        pools.check_primary().await,
        Err(DatabaseUnavailable::PoolClosed)
    );
}

/// Every line a subscriber wrote, shared with the test that reads them.
#[derive(Clone, Default)]
struct Captured(Arc<Mutex<Vec<u8>>>);

impl Write for Captured {
    fn write(&mut self, bytes: &[u8]) -> io::Result<usize> {
        self.0
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .extend_from_slice(bytes);
        Ok(bytes.len())
    }

    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}

#[tokio::test]
async fn only_a_statement_slower_than_250_milliseconds_is_logged_and_at_warn() {
    let container = PostgresContainer::start().await;
    let pools = pools_of(&container);
    let captured = Captured::default();
    let writer = captured.clone();
    let subscriber = tracing_subscriber::fmt()
        .json()
        .with_max_level(tracing::Level::TRACE)
        .with_writer(move || writer.clone())
        .finish();
    let _default = tracing::subscriber::set_default(subscriber);

    pools.check_primary().await.unwrap();
    // Under the threshold, so not logged: with the 300 ms one alone, a threshold of a few
    // milliseconds would pass too.
    sqlx::query("SELECT pg_sleep(0.15)")
        .execute(pools.primary())
        .await
        .unwrap();
    sqlx::query("SELECT pg_sleep(0.3)")
        .execute(pools.primary())
        .await
        .unwrap();

    let text = String::from_utf8(captured.0.lock().unwrap().clone()).unwrap();
    let statements: Vec<Value> = text
        .lines()
        .map(|line| serde_json::from_str::<Value>(line).unwrap())
        .filter(|line| line["target"] == "sqlx::query")
        .collect();
    // The check's `SELECT 1` may be slow too on a busy machine, and then is logged the same way;
    // logged at any other level, it would be the event without a message.
    for statement in &statements {
        assert_eq!(statement["level"], "WARN", "{text}");
        assert_eq!(
            statement["fields"]["message"],
            "slow statement: execution time exceeded alert threshold"
        );
    }
    let logged = |sql: &str| {
        statements
            .iter()
            .filter(|statement| statement["fields"]["summary"] == sql)
            .count()
    };
    assert_eq!(logged("SELECT pg_sleep(0.15)"), 0, "{text}");
    assert_eq!(logged("SELECT pg_sleep(0.3)"), 1, "{text}");
}
