//! The real `barras-ingest` binary, started as an operator starts it (spec §8, §9.2): it refuses
//! to start without its configuration, naming each variable and never a value; it serves its
//! internal port on the address it is given, logs JSON outside `local`, and stops with status 0
//! on SIGTERM. `.env.example` holds a configuration it accepts.
//!
//! A binary that was started is always ended with `terminate` or run to its end, never dropped:
//! a drop kills it, and a killed binary writes no coverage profile.
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)] // integration test crate: every item is test code

use std::net::{Ipv4Addr, SocketAddr, TcpListener};
use std::path::{Path, PathBuf};
use std::process::Output;
use std::time::Duration;

use barras_adapter_http::testing::{TestResponse, fetch};
use barras_bootstrap::{AppEnv, Environment, LogLevel};
use barras_ingest::IngestConfig;
use barras_testkit::{BinaryCommand, RunningBinary, env_file, free_local_addr};
use serde_json::{Value, json};

const BINARY: &str = env!("CARGO_BIN_EXE_barras-ingest");

/// How long a start or a stop may take before the test fails: far longer than either takes, for
/// an instrumented build on a busy machine.
const PATIENCE: Duration = Duration::from_secs(30);

/// How many times the binary is started on a new address when another process took it between
/// the moment it was found free and the moment the binary bound it.
const ATTEMPTS: usize = 3;

fn repository_file(name: &str) -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../..")
        .join(name)
}

/// The binary with the variables of `.env.example` as its whole environment.
fn with_example_values() -> BinaryCommand {
    env_file(repository_file(".env.example"))
        .into_iter()
        .fold(BinaryCommand::new(BINARY), |command, (name, value)| {
            command.env(name, value)
        })
}

fn text(bytes: &[u8]) -> &str {
    std::str::from_utf8(bytes).unwrap()
}

/// The exit status and everything the binary printed, for the message of a failed assertion.
fn describe(output: &Output) -> String {
    format!(
        "{}\nstandard output:\n{}\nstandard error:\n{}",
        output.status,
        text(&output.stdout),
        text(&output.stderr)
    )
}

/// Each line of `log`, parsed as JSON; the test fails on a line that is not.
fn json_lines(log: &[u8]) -> Vec<Value> {
    text(log)
        .lines()
        .map(|line| serde_json::from_str(line).unwrap_or_else(|error| panic!("{error}: {line:?}")))
        .collect()
}

/// `GET path` on `addr`, from a test that has no runtime of its own.
fn get(addr: SocketAddr, path: &str) -> TestResponse {
    tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .unwrap()
        .block_on(fetch(addr, path))
}

/// A binary that listens on its internal address.
struct Serving {
    binary: RunningBinary,
    internal: SocketAddr,
}

/// Starts the binary with the values of `.env.example`, `variables` over them and a free
/// address, and waits until the address listens. A binary that stopped because another process
/// took its address in between is started again on a new one.
fn serve(variables: &[(&str, &str)]) -> Serving {
    for _ in 0..ATTEMPTS {
        let internal = free_local_addr();
        let command = variables
            .iter()
            .fold(with_example_values(), |command, (name, value)| {
                command.env(name, value)
            })
            .env("INGEST_INTERNAL_BIND_ADDR", internal.to_string());
        let mut binary = command.spawn();
        if binary.wait_until_listening(internal, PATIENCE) {
            return Serving { binary, internal };
        }
        let output = binary.terminate(PATIENCE);
        assert!(
            text(&output.stdout).contains("could not bind"),
            "the binary did not listen: {}",
            describe(&output)
        );
    }
    panic!("another process took the binary's address {ATTEMPTS} times in a row");
}

#[test]
fn an_empty_environment_stops_the_binary_naming_every_variable() {
    let output = BinaryCommand::new(BINARY).run();

    assert_eq!(output.status.code(), Some(1), "{}", describe(&output));
    let lines: Vec<&str> = text(&output.stderr).lines().collect();
    assert_eq!(lines.len(), IngestConfig::VARIABLES.len(), "{lines:?}");
    for (line, name) in lines.iter().zip(IngestConfig::VARIABLES) {
        assert!(
            line.starts_with(&format!("{name} is missing: expected ")),
            "{line}"
        );
    }
    // Nothing logged, so nothing listened.
    assert_eq!(text(&output.stdout), "");
}

#[test]
fn an_app_env_that_is_not_valid_stops_the_binary_before_its_address_listens() {
    let internal = free_local_addr();
    let mut binary = with_example_values()
        .env("APP_ENV", "development")
        .env("INGEST_INTERNAL_BIND_ADDR", internal.to_string())
        .spawn();

    let listened = binary.wait_until_listening(internal, PATIENCE);
    let output = binary.terminate(PATIENCE);

    assert!(!listened, "{}", describe(&output));
    assert_eq!(output.status.code(), Some(1), "{}", describe(&output));
    let stderr = text(&output.stderr);
    assert_eq!(
        stderr,
        "APP_ENV is not valid: expected one of local, staging, production\n"
    );
    assert!(!stderr.contains("development"), "{stderr}");
}

#[test]
fn the_values_of_env_example_serve_health_and_metrics_on_the_internal_port() {
    let serving = serve(&[]);

    let health = get(serving.internal, "/health");
    let metrics = get(serving.internal, "/metrics");
    let public_route = get(serving.internal, "/api/v1/health");
    let output = serving.binary.terminate(PATIENCE);

    assert_eq!(health.status, 200);
    let health = health.json();
    assert_eq!(health["status"], "ok");
    assert_eq!(health["service"], "barras-ingest");
    assert_eq!(health["version"], env!("CARGO_PKG_VERSION"));
    // No probe until BAR-4 brings the first dependency.
    assert_eq!(health["dependencies"], json!([]));
    assert_eq!(metrics.status, 200);
    let metrics = text(metrics.bytes());
    let build_info = format!(
        r#"barras_build_info{{service="barras-ingest",version="{}"}} 1"#,
        env!("CARGO_PKG_VERSION")
    );
    assert!(metrics.contains(&build_info), "{metrics}");
    // The ingestor has no public API.
    assert_eq!(public_route.status, 404);
    assert_eq!(output.status.code(), Some(0), "{}", describe(&output));
}

#[test]
fn sigterm_stops_the_binary_with_status_0() {
    let serving = serve(&[]);

    let output = serving.binary.terminate(PATIENCE);

    assert_eq!(output.status.code(), Some(0), "{}", describe(&output));
    assert_eq!(text(&output.stderr), "");
}

#[test]
fn outside_local_every_line_of_the_log_is_json_from_listening_to_stopped() {
    let serving = serve(&[("APP_ENV", "production")]);
    let internal = serving.internal;

    let health = get(internal, "/health");
    let output = serving.binary.terminate(PATIENCE);

    assert_eq!(health.status, 200);
    assert_eq!(output.status.code(), Some(0), "{}", describe(&output));
    let lines = json_lines(&output.stdout);
    let messages: Vec<&str> = lines
        .iter()
        .map(|line| line["fields"]["message"].as_str().unwrap())
        .collect();
    // The internal port logs no request, so a quiet run writes these three lines and no other.
    assert_eq!(messages, ["listening", "shutting down", "stopped"]);
    assert_eq!(lines[0]["fields"]["internal_addr"], internal.to_string());
}

#[test]
fn an_address_already_taken_stops_the_binary_and_the_log_says_why() {
    let taken = TcpListener::bind((Ipv4Addr::LOCALHOST, 0)).unwrap();
    let internal = taken.local_addr().unwrap();

    let output = with_example_values()
        .env("APP_ENV", "staging")
        .env("INGEST_INTERNAL_BIND_ADDR", internal.to_string())
        .run();

    assert_eq!(output.status.code(), Some(1), "{}", describe(&output));
    let lines = json_lines(&output.stdout);
    let failure = lines.last().unwrap();
    assert_eq!(failure["level"], "ERROR", "{failure}");
    assert_eq!(
        failure["fields"]["message"], "barras-ingest stopped on a failure",
        "{failure}"
    );
    // The listener and the address, then the cause the system gave.
    let error = failure["fields"]["error"].as_str().unwrap();
    let listener = format!("the internal listener could not bind {internal}: ");
    assert!(error.starts_with(&listener), "{error}");
    assert!(error.contains("Address already in use"), "{error}");
    assert_eq!(text(&output.stderr), "");
}

#[test]
fn a_command_line_it_does_not_know_prints_the_usage_and_exits_with_status_2() {
    for args in [&["serve"][..], &["--help"], &["openapi"]] {
        let output = args
            .iter()
            .fold(BinaryCommand::new(BINARY), |command, arg| command.arg(arg))
            .run();

        assert_eq!(
            output.status.code(),
            Some(2),
            "{args:?}: {}",
            describe(&output)
        );
        let stderr = text(&output.stderr);
        assert!(
            stderr.starts_with("usage: barras-ingest"),
            "{args:?}: {stderr}"
        );
        assert_eq!(text(&output.stdout), "", "{args:?}");
    }
}

#[test]
fn an_argument_that_is_not_unicode_is_a_command_line_it_does_not_know() {
    use std::os::unix::ffi::OsStrExt;

    let output = BinaryCommand::new(BINARY)
        .arg(std::ffi::OsStr::from_bytes(b"ser\xffve"))
        .run();

    assert_eq!(output.status.code(), Some(2), "{}", describe(&output));
    assert!(text(&output.stderr).starts_with("usage: barras-ingest"));
}

#[test]
fn the_configuration_loads_from_the_values_of_env_example() {
    let variables = env_file(repository_file(".env.example"));

    let config = IngestConfig::from_env(Environment::from_pairs(variables)).unwrap();

    assert_eq!(
        config,
        IngestConfig {
            env: AppEnv::Local,
            log_level: LogLevel::Info,
            internal_bind_addr: "127.0.0.1:9081".parse().unwrap(),
        }
    );
}

#[test]
fn every_variable_of_the_configuration_is_in_env_example() {
    let written: Vec<String> = env_file(repository_file(".env.example"))
        .into_iter()
        .map(|(name, _)| name)
        .collect();

    for name in IngestConfig::VARIABLES {
        assert!(
            written.iter().any(|written| written == name),
            "{name} is not in .env.example"
        );
    }
}
