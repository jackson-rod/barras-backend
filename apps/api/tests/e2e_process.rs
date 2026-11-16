//! The real `barras-api` binary, started as an operator starts it (spec §8, §9.2): it refuses to
//! start without its configuration, naming each variable and never a value; it serves the public
//! API and the internal port on the addresses it is given, logs JSON outside `local`, and stops
//! with status 0 on SIGTERM; `openapi` prints the document with no configuration at all, and
//! `openapi --check` fails on a stale file.
//!
//! A binary that was started is always ended with `terminate` or run to its end, never dropped:
//! a drop kills it, and a killed binary writes no coverage profile.
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)] // integration test crate: every item is test code

use std::fs;
use std::net::{SocketAddr, TcpListener};
use std::path::{Path, PathBuf};
use std::process::{Command, Output, Stdio};
use std::time::Duration;

use barras_adapter_http::testing::{TestResponse, fetch};
use barras_api::ApiConfig;
use barras_testkit::{BinaryCommand, RunningBinary, env_file, free_local_addr};
use serde_json::Value;

const BINARY: &str = env!("CARGO_BIN_EXE_barras-api");

/// How long a start or a stop may take before the test fails: far longer than either takes, for
/// an instrumented build on a busy machine.
const PATIENCE: Duration = Duration::from_secs(30);

/// How many times the binary is started on new addresses when another process took one of them
/// between the moment it was found free and the moment the binary bound it.
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

/// A binary that listens on both its addresses.
struct Serving {
    binary: RunningBinary,
    public: SocketAddr,
    internal: SocketAddr,
}

/// Starts the binary with the values of `.env.example`, `variables` over them and two free
/// addresses, and waits until both addresses listen. A binary that stopped because another
/// process took one of its addresses in between is started again on new ones.
fn serve(variables: &[(&str, &str)]) -> Serving {
    for _ in 0..ATTEMPTS {
        let public = free_local_addr();
        let internal = free_local_addr();
        let command = variables
            .iter()
            .fold(with_example_values(), |command, (name, value)| {
                command.env(name, value)
            })
            .env("API_BIND_ADDR", public.to_string())
            .env("API_INTERNAL_BIND_ADDR", internal.to_string());
        let mut binary = command.spawn();
        if binary.wait_until_listening(public, PATIENCE)
            && binary.wait_until_listening(internal, PATIENCE)
        {
            return Serving {
                binary,
                public,
                internal,
            };
        }
        let output = binary.terminate(PATIENCE);
        assert!(
            text(&output.stdout).contains("could not bind"),
            "the binary did not listen: {}",
            describe(&output)
        );
    }
    panic!("another process took the binary's addresses {ATTEMPTS} times in a row");
}

#[test]
fn an_empty_environment_stops_the_binary_naming_every_variable() {
    let output = BinaryCommand::new(BINARY).run();

    assert_eq!(output.status.code(), Some(1), "{}", describe(&output));
    let lines: Vec<&str> = text(&output.stderr).lines().collect();
    assert_eq!(lines.len(), ApiConfig::VARIABLES.len(), "{lines:?}");
    for (line, name) in lines.iter().zip(ApiConfig::VARIABLES) {
        assert!(
            line.starts_with(&format!("{name} is missing: expected ")),
            "{line}"
        );
    }
    // Nothing logged, so nothing listened.
    assert_eq!(text(&output.stdout), "");
}

#[test]
fn a_log_level_that_is_not_valid_stops_the_binary_before_either_address_listens() {
    let public = free_local_addr();
    let internal = free_local_addr();
    let mut binary = with_example_values()
        .env("LOG_LEVEL", "loud")
        .env("API_BIND_ADDR", public.to_string())
        .env("API_INTERNAL_BIND_ADDR", internal.to_string())
        .spawn();

    let listened = binary.wait_until_listening(public, PATIENCE)
        || binary.wait_until_listening(internal, PATIENCE);
    let output = binary.terminate(PATIENCE);

    assert!(!listened, "{}", describe(&output));
    assert_eq!(output.status.code(), Some(1), "{}", describe(&output));
    let stderr = text(&output.stderr);
    assert_eq!(
        stderr,
        "LOG_LEVEL is not valid: expected one of trace, debug, info, warn, error\n"
    );
    assert!(!stderr.contains("loud"), "{stderr}");
}

#[test]
fn the_values_of_env_example_serve_the_public_api_and_the_internal_port() {
    let serving = serve(&[]);

    let ready = get(serving.public, "/api/v1/health");
    let live = get(serving.public, "/api/v1/health/live");
    let metrics_on_public = get(serving.public, "/metrics");
    let health = get(serving.internal, "/health");
    let metrics = get(serving.internal, "/metrics");
    let output = serving.binary.terminate(PATIENCE);

    assert_eq!(ready.status, 200);
    assert_eq!(ready.json()["data"]["status"], "ok");
    assert_eq!(live.status, 200);
    assert_eq!(live.json()["data"]["status"], "ok");
    assert_eq!(metrics_on_public.status, 404);
    assert_eq!(
        metrics_on_public.error_code().as_deref(),
        Some("ROUTE_NOT_FOUND")
    );
    assert_eq!(health.status, 200);
    assert_eq!(health.json()["service"], "barras-api");
    assert_eq!(health.json()["status"], "ok");
    assert_eq!(metrics.status, 200);
    let metrics = text(metrics.bytes());
    let build_info = format!(
        r#"barras_build_info{{service="barras-api",version="{}"}} 1"#,
        env!("CARGO_PKG_VERSION")
    );
    assert!(metrics.contains(&build_info), "{metrics}");
    let ready_counted = r#"http_requests_total{method="GET",path="/api/v1/health",status="200"} 1"#;
    assert!(metrics.contains(ready_counted), "{metrics}");
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
    let serving = serve(&[("APP_ENV", "staging")]);
    let (public, internal) = (serving.public, serving.internal);

    let ready = get(public, "/api/v1/health");
    let output = serving.binary.terminate(PATIENCE);

    assert_eq!(ready.status, 200);
    assert_eq!(output.status.code(), Some(0), "{}", describe(&output));
    let lines = json_lines(&output.stdout);
    let messages: Vec<&str> = lines
        .iter()
        .map(|line| line["fields"]["message"].as_str().unwrap())
        .collect();
    assert_eq!(messages.first(), Some(&"listening"), "{messages:?}");
    assert!(
        messages.ends_with(&["shutting down", "stopped"]),
        "{messages:?}"
    );
    assert!(messages.len() > 3, "the request left no line: {messages:?}");
    assert_eq!(lines[0]["fields"]["public_addr"], public.to_string());
    assert_eq!(lines[0]["fields"]["internal_addr"], internal.to_string());
}

#[test]
fn an_address_already_taken_stops_the_binary_and_the_log_says_why() {
    let taken = TcpListener::bind((std::net::Ipv4Addr::LOCALHOST, 0)).unwrap();
    let public = taken.local_addr().unwrap();

    let output = with_example_values()
        .env("APP_ENV", "staging")
        .env("API_BIND_ADDR", public.to_string())
        .env("API_INTERNAL_BIND_ADDR", free_local_addr().to_string())
        .run();

    assert_eq!(output.status.code(), Some(1), "{}", describe(&output));
    let lines = json_lines(&output.stdout);
    let failure = lines.last().unwrap();
    assert_eq!(failure["level"], "ERROR", "{failure}");
    // The listener and the address, then the cause the system gave.
    let error = failure["fields"]["error"].as_str().unwrap();
    let listener = format!("the public listener could not bind {public}: ");
    assert!(error.starts_with(&listener), "{error}");
    assert!(error.contains("Address already in use"), "{error}");
    assert_eq!(text(&output.stderr), "");
}

#[test]
fn openapi_prints_the_committed_document_with_an_empty_environment() {
    let output = BinaryCommand::new(BINARY).arg("openapi").run();

    assert_eq!(output.status.code(), Some(0), "{}", describe(&output));
    let committed = fs::read(repository_file("openapi.json")).unwrap();
    assert!(
        output.stdout == committed,
        "the document printed is not openapi.json: run `just openapi`"
    );
    assert_eq!(text(&output.stderr), "");
}

#[test]
fn openapi_reports_a_document_it_cannot_write_instead_of_panicking() {
    // A pipe nobody reads from any more: every write to it fails.
    let (reader, writer) = std::io::pipe().unwrap();
    drop(reader);

    // `BinaryCommand` captures standard output, so this one is started by hand. `openapi` reads
    // no variable, so the environment the test runs in changes nothing.
    let output = Command::new(BINARY)
        .arg("openapi")
        .stdout(Stdio::from(writer))
        .output()
        .unwrap();

    assert_eq!(output.status.code(), Some(1), "{}", describe(&output));
    let stderr = text(&output.stderr);
    assert!(
        stderr.starts_with("the OpenAPI document could not be written: "),
        "{stderr}"
    );
}

#[test]
fn openapi_check_fails_on_a_file_it_cannot_read_and_says_how_to_write_it() {
    let output = BinaryCommand::new(BINARY)
        .arg("openapi")
        .arg("--check")
        .arg("/nonexistent/openapi.json")
        .run();

    assert_eq!(output.status.code(), Some(1), "{}", describe(&output));
    let stderr = text(&output.stderr);
    assert!(stderr.contains("could not be read"), "{stderr}");
    assert!(stderr.contains("run `just openapi`"), "{stderr}");
}

#[test]
fn openapi_check_fails_on_a_stale_file_and_says_how_to_update_it() {
    let committed = fs::read_to_string(repository_file("openapi.json")).unwrap();
    let stale = Path::new(env!("CARGO_TARGET_TMPDIR"))
        .join(format!("stale-openapi-{}.json", std::process::id()));
    fs::write(
        &stale,
        committed.replacen("Barras API", "Barras API, stale", 1),
    )
    .unwrap();

    let output = BinaryCommand::new(BINARY)
        .arg("openapi")
        .arg("--check")
        .arg(&stale)
        .run();

    fs::remove_file(&stale).unwrap();
    assert_eq!(output.status.code(), Some(1), "{}", describe(&output));
    let stderr = text(&output.stderr);
    assert!(stderr.contains("run `just openapi`"), "{stderr}");
    assert_eq!(text(&output.stdout), "");
}

#[test]
fn openapi_check_passes_on_the_committed_file() {
    let output = BinaryCommand::new(BINARY)
        .arg("openapi")
        .arg("--check")
        .arg(repository_file("openapi.json"))
        .run();

    assert_eq!(output.status.code(), Some(0), "{}", describe(&output));
    assert_eq!(text(&output.stderr), "");
}

#[test]
fn a_command_line_it_does_not_know_prints_the_usage_and_exits_with_status_2() {
    for args in [&["serve"][..], &["openapi", "--check"]] {
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
            stderr.starts_with("usage: barras-api"),
            "{args:?}: {stderr}"
        );
    }
}

#[test]
fn an_argument_that_is_not_unicode_is_a_command_line_it_does_not_know() {
    use std::os::unix::ffi::OsStrExt;

    let output = BinaryCommand::new(BINARY)
        .arg(std::ffi::OsStr::from_bytes(b"open\xffapi"))
        .run();

    assert_eq!(output.status.code(), Some(2), "{}", describe(&output));
    assert!(text(&output.stderr).starts_with("usage: barras-api"));
}
