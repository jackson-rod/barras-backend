//! The Postgres container of a test, checked through the Docker command line: where it listens,
//! how it runs, and that it goes away with the test; and the relay that reaches one through a
//! port of this machine.
#![allow(clippy::unwrap_used, clippy::expect_used)] // an integration test: a step that fails must fail the test

use std::io::{Read, Write};
use std::net::TcpStream;
use std::process::Command;

use barras_testkit::{LoopbackRelay, PostgresContainer};

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
    assert_eq!(container.psql("SHOW fsync"), "off");
    assert_eq!(
        container.psql(
            "SELECT count(*) FROM information_schema.tables \
             WHERE table_schema NOT IN ('pg_catalog', 'information_schema')"
        ),
        "0"
    );
}

#[tokio::test]
async fn a_container_starts_as_the_local_server_unless_it_stands_for_another() {
    let local = PostgresContainer::start().await;
    let elsewhere = PostgresContainer::start_as_another_server().await;

    assert_eq!(local.psql("SHOW cluster_name"), "barras-local");
    assert_eq!(elsewhere.psql("SHOW cluster_name"), "");
    assert_eq!(elsewhere.psql("SHOW fsync"), "off");
}

#[tokio::test]
async fn a_relay_answers_for_its_target_on_a_loopback_port() {
    let target = PostgresContainer::start_as_another_server().await;
    let relay = LoopbackRelay::start(&target).await;

    let published = docker(&["port", relay.id(), "5432/tcp"]);
    let port = published.strip_prefix("127.0.0.1:").unwrap();
    assert_eq!(
        relay.url(),
        format!("postgres://barras:barras@127.0.0.1:{port}/barras")
    );
    assert_eq!(published.lines().count(), 1, "{published}");
    assert_eq!(
        docker(&[
            "inspect",
            "--format",
            r#"{{index .Config.Labels "barras-test"}}"#,
            relay.id()
        ]),
        "1"
    );
    // The first message of the protocol, which asks for TLS: a Postgres without it answers `N`.
    let mut stream = TcpStream::connect(format!("127.0.0.1:{port}")).unwrap();
    stream
        .write_all(&[0, 0, 0, 8, 0x04, 0xd2, 0x16, 0x2f])
        .unwrap();
    let mut answer = [0];
    stream.read_exact(&mut answer).unwrap();
    assert_eq!(&answer, b"N");
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
