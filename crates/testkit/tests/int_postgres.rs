//! The Postgres container of a test, checked through the Docker command line: where it listens,
//! how it runs, and that it goes away with the test.
#![allow(clippy::unwrap_used, clippy::expect_used)] // an integration test: a step that fails must fail the test

use std::process::Command;

use barras_testkit::PostgresContainer;

/// What `docker` printed, trimmed; the test fails when the command does.
fn docker(args: &[&str]) -> String {
    let output = Command::new("docker").args(args).output().unwrap();
    assert!(
        output.status.success(),
        "docker {args:?}: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    String::from_utf8(output.stdout).unwrap().trim().to_owned()
}

/// One value, through `psql` inside the container: no client on this machine is needed.
fn query(container: &PostgresContainer, sql: &str) -> String {
    docker(&[
        "exec",
        container.id(),
        "psql",
        "--username=barras",
        "--dbname=barras",
        "--no-align",
        "--tuples-only",
        "--command",
        sql,
    ])
}

#[tokio::test]
async fn a_container_serves_an_empty_barras_database_on_a_loopback_port() {
    let container = PostgresContainer::start().await;

    let published = docker(&["port", container.id(), "5432/tcp"]);
    let port = published.strip_prefix("127.0.0.1:").unwrap();
    assert_eq!(
        container.url(),
        format!("postgres://barras:barras@127.0.0.1:{port}/barras")
    );
    // Only the loopback address: a container with a known password must not answer the network.
    assert_eq!(published.lines().count(), 1, "{published}");
    assert_eq!(
        docker(&[
            "inspect",
            "--format",
            r#"{{index .Config.Labels "barras-test"}}"#,
            container.id()
        ]),
        "1"
    );
    assert_eq!(query(&container, "SHOW fsync"), "off");
    assert_eq!(
        query(
            &container,
            "SELECT count(*) FROM information_schema.tables \
             WHERE table_schema NOT IN ('pg_catalog', 'information_schema')"
        ),
        "0"
    );
}

#[tokio::test]
async fn a_dropped_container_is_removed() {
    let container = PostgresContainer::start().await;
    let id = container.id().to_owned();

    drop(container);

    let left = docker(&[
        "ps",
        "--all",
        "--quiet",
        "--no-trunc",
        "--filter",
        &format!("id={id}"),
    ]);
    assert_eq!(left, "");
}
