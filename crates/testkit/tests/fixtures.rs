//! The fixtures that need no container: a folder of seed files that a test writes, and a port of
//! this machine where nothing listens.
#![allow(clippy::unwrap_used, clippy::expect_used)] // an integration test: a step that fails must fail the test

use std::fs;
use std::io;
use std::net::{Ipv4Addr, TcpStream};
use std::path::Path;

use barras_testkit::{SeedFolder, closed_port};

#[test]
fn a_seed_folder_holds_what_the_test_writes_and_goes_away_with_its_value() {
    let folder = SeedFolder::new(env!("CARGO_TARGET_TMPDIR"), "fixtures-written")
        .with("20261116000001_first.sql", "SELECT 1;")
        .with("20261116000002_bytes.sql", b"SELECT '\xe9';");
    let path = folder.path().to_owned();

    assert_eq!(path.parent(), Some(Path::new(env!("CARGO_TARGET_TMPDIR"))));
    assert_eq!(
        path.file_name().unwrap().to_str().unwrap(),
        format!("fixtures-written-{}", std::process::id())
    );
    assert_eq!(
        fs::read_to_string(path.join("20261116000001_first.sql")).unwrap(),
        "SELECT 1;"
    );
    assert_eq!(
        fs::read(path.join("20261116000002_bytes.sql")).unwrap(),
        b"SELECT '\xe9';"
    );

    drop(folder);

    assert!(!path.exists());
}

#[test]
fn a_seed_folder_that_a_killed_run_left_behind_is_emptied_first() {
    let left = Path::new(env!("CARGO_TARGET_TMPDIR"))
        .join(format!("fixtures-left-{}", std::process::id()));
    fs::create_dir_all(&left).unwrap();
    fs::write(left.join("20261116000001_stale.sql"), "SELECT 1;").unwrap();

    let folder = SeedFolder::new(env!("CARGO_TARGET_TMPDIR"), "fixtures-left");

    assert_eq!(folder.path(), left);
    assert_eq!(fs::read_dir(folder.path()).unwrap().count(), 0);
}

#[test]
fn nothing_answers_at_a_closed_port() {
    let refused = TcpStream::connect((Ipv4Addr::LOCALHOST, closed_port())).unwrap_err();

    assert_eq!(refused.kind(), io::ErrorKind::ConnectionRefused);
}
