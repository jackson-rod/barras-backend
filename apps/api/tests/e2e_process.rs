//! The real `barras-api` binary, started as an operator starts it (spec §8, §9.2): it refuses to
//! start without its configuration, naming each variable and never a value; it serves the public
//! API and the internal port on the addresses it is given, reports its database in readiness
//! without ever showing a credential (BAR-4 spec §6.3), logs JSON outside `local`, and stops with
//! status 0 on SIGTERM, its database connections closed; `openapi` prints the document and reads
//! no variable, and `openapi --check` fails on a stale file. Neither a closed standard error nor
//! a worker count for tokio in the environment turns a run into a panic.
//!
//! Every address the binary binds is `127.0.0.1:0`: the system picks a free port, so no other
//! process can take it first, and the test reads the ports from the `listening` line. A binary
//! that was started is always ended within a deadline, by `terminate` or `run_within`, never
//! dropped: a drop kills it, and a killed binary writes no coverage profile.
//!
//! No test reaches the local database that `.env.example` names: a binary that serves gets a
//! container of its test's own, or a port where nothing listens.
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)] // integration test crate: every item is test code

use std::fs;
use std::net::{Ipv4Addr, SocketAddr, TcpListener};
use std::path::Path;
use std::thread;
use std::time::{Duration, Instant};

use barras_adapter_http::testing::{TestResponse, fetch};
use barras_api::ApiConfig;
use barras_testkit::process::{
    BinaryCommand, RunningBinary, address_in, closed_pipe, describe, is_listening, json_lines,
    text, with_example_values,
};
use barras_testkit::{PostgresContainer, repository_file};
use serde_json::Value;

const BINARY: &str = env!("CARGO_BIN_EXE_barras-api");

/// How long a start, a run or a stop may take before the test fails: far longer than any takes,
/// for an instrumented build on a busy machine.
const PATIENCE: Duration = Duration::from_secs(30);

/// A free port of the loopback interface, which the system picks when the binary binds it.
const ANY_PORT: &str = "127.0.0.1:0";

/// The password of every database URL a test gives the binary, which no output may show.
const PASSWORD: &str = "hunter2-secret";

/// What a probe of a database that cannot be reached answers. The pool gives up on a connection
/// after two seconds and the probe on an answer after two seconds too, so whichever of the two
/// the runtime's timer ends first gives the sentence; each is one of a fixed set.
const NO_ANSWER_IN_TIME: [&str; 2] = [
    "no connection within 2 seconds",
    "did not answer within 2 seconds",
];

/// How many sessions the binary has in a container's server: each of its connections names
/// itself after it.
const BINARY_SESSIONS: &str =
    "SELECT count(*) FROM pg_stat_activity WHERE application_name = 'barras-api'";

/// `GET path` on `addr`, from a test that has no runtime of its own.
fn get(addr: SocketAddr, path: &str) -> TestResponse {
    tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .unwrap()
        .block_on(fetch(addr, path))
}

/// A port of this machine where nothing listens.
fn closed_port() -> u16 {
    TcpListener::bind((Ipv4Addr::LOCALHOST, 0))
        .unwrap()
        .local_addr()
        .unwrap()
        .port()
}

/// A database URL at a port where nothing listens: the pools are built without connecting, so
/// the binary serves with it, and its readiness says the database is down.
fn unreachable_database() -> String {
    format!(
        "postgres://barras:{PASSWORD}@127.0.0.1:{}/barras",
        closed_port()
    )
}

/// The binary with the values of `.env.example`, and both database URLs at `database` instead of
/// the local database the file names.
fn with_database(database: &str) -> BinaryCommand {
    with_example_values(BINARY)
        .env("API_DATABASE_URL", database)
        .env("API_DATABASE_READ_URL", database)
}

/// A binary that listens on both its addresses.
struct Serving {
    binary: RunningBinary,
    public: SocketAddr,
    internal: SocketAddr,
}

/// Starts the binary with the values of `.env.example`, both database URLs at `database` and
/// `variables` over them, and any free port for both addresses, and waits until it logs where it
/// listens.
fn serve(database: &str, variables: &[(&str, &str)]) -> Serving {
    let binary = variables
        .iter()
        .fold(with_database(database), |command, (name, value)| {
            command.env(name, value)
        })
        .env("API_BIND_ADDR", ANY_PORT)
        .env("API_INTERNAL_BIND_ADDR", ANY_PORT)
        .spawn();
    let Some(listening) = binary.wait_for_line(is_listening, PATIENCE) else {
        let output = binary.terminate(PATIENCE);
        panic!(
            "the binary did not log where it listens: {}",
            describe(&output)
        );
    };
    Serving {
        public: address_in(&listening, "public_addr"),
        internal: address_in(&listening, "internal_addr"),
        binary,
    }
}

/// Waits until the binary has no session left in `container`'s server. A session's server
/// process records how the session ended before it leaves `pg_stat_activity`, so what
/// `pg_stat_database` says of the binary's sessions is then whole.
fn wait_until_no_session_is_left(container: &PostgresContainer) {
    let deadline = Instant::now() + PATIENCE;
    while container.psql(BINARY_SESSIONS) != "0" {
        assert!(
            Instant::now() < deadline,
            "the binary's sessions were still open after {PATIENCE:?}"
        );
        thread::sleep(Duration::from_millis(50));
    }
}

#[test]
fn an_empty_environment_stops_the_binary_naming_every_variable() {
    let output = BinaryCommand::new(BINARY).run_within(PATIENCE);

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
fn a_log_level_that_is_not_valid_stops_the_binary_before_it_listens() {
    let output = with_database(&unreachable_database())
        .env("LOG_LEVEL", "loud")
        .env("API_BIND_ADDR", ANY_PORT)
        .env("API_INTERNAL_BIND_ADDR", ANY_PORT)
        .run_within(PATIENCE);

    assert_eq!(output.status.code(), Some(1), "{}", describe(&output));
    let stderr = text(&output.stderr);
    assert_eq!(
        stderr,
        "LOG_LEVEL is not valid: expected one of trace, debug, info, warn, error\n"
    );
    assert!(!stderr.contains("loud"), "{stderr}");
    // Nothing logged: the logs did not start, and the listeners come after them.
    assert_eq!(text(&output.stdout), "");
}

// A runtime of the test's own, which the container must be dropped in, so each request is sent
// with `fetch` on it.
#[tokio::test]
async fn with_the_database_up_the_binary_is_ready_and_its_internal_port_lists_each_pool() {
    let container = PostgresContainer::start().await;
    // Another spelling of the same database, so that the read pool is a pool of its own.
    let read = container.url().replace("postgres://", "postgresql://");
    let serving = serve(container.url(), &[("API_DATABASE_READ_URL", &read)]);

    let ready = fetch(serving.public, "/api/v1/health").await;
    let live = fetch(serving.public, "/api/v1/health/live").await;
    let metrics_on_public = fetch(serving.public, "/metrics").await;
    let health = fetch(serving.internal, "/health").await;
    let metrics = fetch(serving.internal, "/metrics").await;
    let connections = container.psql(BINARY_SESSIONS);
    let output = serving.binary.terminate(PATIENCE);
    wait_until_no_session_is_left(&container);
    let abandoned =
        container.psql("SELECT sessions_abandoned FROM pg_stat_database WHERE datname = 'barras'");

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
    let dependencies = health.json()["dependencies"].clone();
    let reported: Vec<(&str, &Value, &str)> = dependencies
        .as_array()
        .unwrap()
        .iter()
        .map(|dependency| {
            let name = dependency["name"].as_str().unwrap();
            (
                name,
                &dependency["critical"],
                dependency["status"].as_str().unwrap(),
            )
        })
        .collect();
    let critical = Value::Bool(true);
    assert_eq!(
        reported,
        [
            ("postgres", &critical, "up"),
            ("postgres-read", &critical, "up")
        ]
    );
    // The connection each pool's check opened, named after the binary, and kept for the next.
    assert_eq!(connections, "2");
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
    // The pools closed both connections before the binary exited. A connection that the end of
    // the process cut instead would count as abandoned (spec §4.2).
    assert_eq!(abandoned, "0");
}

#[test]
fn with_the_database_at_a_closed_port_readiness_is_503_and_no_output_shows_the_password() {
    let serving = serve(&unreachable_database(), &[("APP_ENV", "staging")]);

    let ready = get(serving.public, "/api/v1/health");
    // Within the second that the public request's run is given again.
    let health = get(serving.internal, "/health");
    let output = serving.binary.terminate(PATIENCE);

    assert_eq!(ready.status, 503);
    assert_eq!(ready.error_code().as_deref(), Some("SERVICE_UNAVAILABLE"));
    let public = text(ready.bytes());
    assert!(!public.contains("postgres"), "{public}");
    assert_eq!(health.status, 503);
    let dependencies = health.json()["dependencies"].clone();
    assert_eq!(dependencies.as_array().unwrap().len(), 1, "{dependencies}");
    assert_eq!(dependencies[0]["name"], "postgres");
    assert_eq!(dependencies[0]["status"], "down");
    let error = dependencies[0]["error"].as_str().unwrap();
    assert!(NO_ANSWER_IN_TIME.contains(&error), "{error}");
    // The log names the probe and its sentence, as the public answer never does.
    let lines = json_lines(&output.stdout);
    let failed = lines
        .iter()
        .find(|line| line["fields"]["message"] == "request failed")
        .unwrap();
    let detail = failed["fields"]["detail"].as_str().unwrap();
    let named = NO_ANSWER_IN_TIME
        .map(|sentence| format!("critical dependencies are down: postgres ({sentence})"));
    assert!(named.iter().any(|named| named == detail), "{detail}");
    assert_eq!(output.status.code(), Some(0), "{}", describe(&output));
    for printed in [&output.stdout, &output.stderr] {
        assert!(!text(printed).contains(PASSWORD), "{}", describe(&output));
    }
}

#[test]
fn sigterm_stops_the_binary_with_status_0() {
    let serving = serve(&unreachable_database(), &[]);

    let output = serving.binary.terminate(PATIENCE);

    assert_eq!(output.status.code(), Some(0), "{}", describe(&output));
    assert_eq!(text(&output.stderr), "");
}

#[test]
fn outside_local_every_line_of_the_log_is_json_from_listening_to_stopped() {
    let serving = serve(&unreachable_database(), &[("APP_ENV", "staging")]);

    let live = get(serving.public, "/api/v1/health/live");
    let output = serving.binary.terminate(PATIENCE);

    assert_eq!(live.status, 200);
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
}

#[test]
fn a_worker_count_for_tokio_in_the_environment_is_never_read() {
    let serving = serve(&unreachable_database(), &[("TOKIO_WORKER_THREADS", "abc")]);

    let live = get(serving.public, "/api/v1/health/live");
    let output = serving.binary.terminate(PATIENCE);

    assert_eq!(live.status, 200);
    assert_eq!(output.status.code(), Some(0), "{}", describe(&output));
    assert_eq!(text(&output.stderr), "");
}

#[test]
fn an_address_already_taken_stops_the_binary_and_the_log_says_why() {
    let taken = TcpListener::bind((Ipv4Addr::LOCALHOST, 0)).unwrap();
    let public = taken.local_addr().unwrap();

    let output = with_database(&unreachable_database())
        .env("APP_ENV", "staging")
        .env("API_BIND_ADDR", public.to_string())
        .env("API_INTERNAL_BIND_ADDR", ANY_PORT)
        .run_within(PATIENCE);

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
fn a_closed_standard_error_does_not_turn_a_failure_into_a_panic() {
    // The configuration's problems, a database command's, then the usage, each with nowhere to
    // go.
    for (args, status) in [(&[][..], 1), (&["migrate"][..], 1), (&["--help"][..], 2)] {
        let output = args
            .iter()
            .fold(BinaryCommand::new(BINARY), |command, arg| command.arg(arg))
            .stderr(closed_pipe())
            .run_within(PATIENCE);

        assert_eq!(
            output.status.code(),
            Some(status),
            "{args:?}: {}",
            describe(&output)
        );
    }
    // Neither the document nor the failure to write it can be written.
    let output = BinaryCommand::new(BINARY)
        .arg("openapi")
        .stdout(closed_pipe())
        .stderr(closed_pipe())
        .run_within(PATIENCE);

    assert_eq!(output.status.code(), Some(1), "{}", describe(&output));
}

#[test]
fn openapi_prints_the_committed_document_with_an_empty_environment() {
    let output = BinaryCommand::new(BINARY)
        .arg("openapi")
        .run_within(PATIENCE);

    assert_eq!(output.status.code(), Some(0), "{}", describe(&output));
    let committed = fs::read(repository_file("openapi.json")).unwrap();
    assert!(
        output.stdout == committed,
        "the document printed is not openapi.json: run `just openapi`"
    );
    assert_eq!(text(&output.stderr), "");
}

#[test]
fn openapi_reads_no_variable_whatever_its_value() {
    let output = ApiConfig::VARIABLES
        .iter()
        .fold(
            BinaryCommand::new(BINARY).arg("openapi"),
            |command, name| command.env(name, "not-valid"),
        )
        .env("TOKIO_WORKER_THREADS", "abc")
        .run_within(PATIENCE);

    assert_eq!(output.status.code(), Some(0), "{}", describe(&output));
    let committed = fs::read(repository_file("openapi.json")).unwrap();
    assert!(
        output.stdout == committed,
        "the document printed is not openapi.json: {}",
        describe(&output)
    );
    assert_eq!(text(&output.stderr), "");
}

#[test]
fn openapi_reports_a_document_it_cannot_write_instead_of_panicking() {
    let output = BinaryCommand::new(BINARY)
        .arg("openapi")
        .stdout(closed_pipe())
        .run_within(PATIENCE);

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
        .run_within(PATIENCE);

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
        .run_within(PATIENCE);

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
        .run_within(PATIENCE);

    assert_eq!(output.status.code(), Some(0), "{}", describe(&output));
    assert_eq!(text(&output.stderr), "");
}

#[test]
fn a_command_line_it_does_not_know_prints_the_usage_and_exits_with_status_2() {
    for args in [
        &["serve"][..],
        &["openapi", "--check"],
        &["migrate", "now"],
        &["seed"],
        &["reset", "seeds", "more-seeds"],
        &["reset", "--help"],
        &["reset", "-h"],
        &["seed", "-h"],
    ] {
        let output = args
            .iter()
            .fold(BinaryCommand::new(BINARY), |command, arg| command.arg(arg))
            .run_within(PATIENCE);

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
        .run_within(PATIENCE);

    assert_eq!(output.status.code(), Some(2), "{}", describe(&output));
    assert!(text(&output.stderr).starts_with("usage: barras-api"));
}
