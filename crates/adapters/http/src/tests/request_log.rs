//! The request log (spec §6.5): one `info` event for each answered request, inside a span that
//! names the method, the route template and the request id, and never a header, the query
//! string or the body, any of which may carry a credential.

use serde_json::{Value, json};
use tracing::Level;

use super::support::{PANIC_TEXT, client, send_logged_json, send_logged_json_at};
use crate::testing::TestRequest;

const HEADER_SECRET: &str = "header-secret";
const QUERY_SECRET: &str = "query-secret";
const BODY_SECRET: &str = "body-secret";
const PATH_SECRET: &str = "path-secret";

/// The keys of a JSON object, sorted.
fn keys(value: &Value) -> Vec<&str> {
    let mut keys: Vec<&str> = value
        .as_object()
        .unwrap()
        .keys()
        .map(String::as_str)
        .collect();
    keys.sort_unstable();
    keys
}

/// Every line, as one text to search.
fn whole(lines: &[Value]) -> String {
    lines
        .iter()
        .map(Value::to_string)
        .collect::<Vec<_>>()
        .join("\n")
}

/// The level and the message of each line.
fn events(lines: &[Value]) -> Vec<(&str, &str)> {
    lines
        .iter()
        .map(|line| {
            let level = line["level"].as_str().unwrap();
            (level, line["fields"]["message"].as_str().unwrap())
        })
        .collect()
}

/// `path` with a query string and a body that carry a secret each, and a bearer token.
fn with_secrets(method: &str, path: &str) -> TestRequest {
    TestRequest::new(method, &format!("{path}?token={QUERY_SECRET}"))
        .header("authorization", &format!("Bearer {HEADER_SECRET}"))
        .header("content-type", "application/json")
        .body(format!(r#"{{"password":"{BODY_SECRET}"}}"#))
}

#[tokio::test]
async fn an_answered_request_logs_one_info_event_inside_its_span() {
    let request = TestRequest::new("POST", "/api/v1/length")
        .header("x-request-id", "log-probe-1")
        .body("12345");

    let (response, lines) = send_logged_json(&client(), request).await;

    assert_eq!(response.status, 200);
    assert_eq!(events(&lines), [("INFO", "request finished")]);
    let fields = &lines[0]["fields"];
    assert_eq!(keys(fields), ["duration_ms", "message", "status"]);
    assert_eq!(fields["status"], 200);
    assert!(
        fields["duration_ms"].as_f64().is_some_and(|ms| ms >= 0.0),
        "{fields}"
    );
    assert_eq!(
        lines[0]["spans"],
        json!([{
            "name": "request",
            "method": "POST",
            "path": "/api/v1/length",
            "request_id": "log-probe-1"
        }])
    );
}

#[tokio::test]
async fn the_span_names_the_method_as_the_metrics_do_and_the_route_template() {
    for (method, path, logged_method, logged_path, status) in [
        (
            "GET",
            "/api/v1/bare/200",
            "GET",
            "/api/v1/bare/{status}",
            200,
        ),
        ("HEAD", "/api/v1/small", "HEAD", "/api/v1/small", 200),
        ("GET", "/api/v1/nowhere", "GET", "unmatched", 404),
        ("DELETE", "/api/v1/small", "DELETE", "/api/v1/small", 405),
        ("PURGE", "/api/v1/small", "other", "/api/v1/small", 405),
    ] {
        let request = TestRequest::new(method, path);

        let (_, lines) = send_logged_json(&client(), request).await;

        assert_eq!(events(&lines), [("INFO", "request finished")], "{path}");
        assert_eq!(lines[0]["fields"]["status"], status, "{path}");
        let span = &lines[0]["spans"][0];
        assert_eq!(span["method"], logged_method, "{method} {path}");
        assert_eq!(span["path"], logged_path, "{method} {path}");
    }
}

#[tokio::test]
async fn the_id_made_for_a_request_is_the_id_in_its_log() {
    let (response, lines) =
        send_logged_json(&client(), TestRequest::new("GET", "/api/v1/small")).await;

    let made = response.header("x-request-id").unwrap();
    assert_eq!(lines[0]["spans"][0]["request_id"], made);
}

#[tokio::test]
async fn a_server_failure_is_logged_once_with_its_cause_then_as_finished() {
    for (path, status) in [("/api/v1/panic/formatted", 500), ("/api/v1/bare/503", 503)] {
        let request = TestRequest::new("GET", path).header("x-request-id", "log-probe-2");

        let (response, lines) = send_logged_json(&client(), request).await;

        assert_eq!(response.status, status);
        assert_eq!(
            events(&lines),
            [("ERROR", "request failed"), ("INFO", "request finished")],
            "{path}"
        );
        assert_eq!(lines[0]["fields"]["status"], status);
        assert_eq!(lines[1]["fields"]["status"], status);
        for line in &lines {
            assert_eq!(line["spans"][0]["request_id"], "log-probe-2", "{line}");
        }
    }
}

#[tokio::test]
async fn a_server_failure_keeps_its_span_when_only_warnings_or_errors_are_logged() {
    for level in [Level::WARN, Level::ERROR] {
        for (path, template, status) in [
            ("/api/v1/panic/formatted", "/api/v1/panic/{payload}", 500),
            ("/api/v1/bare/503", "/api/v1/bare/{status}", 503),
        ] {
            let request = TestRequest::new("GET", path).header("x-request-id", "log-probe-4");

            let (response, lines) = send_logged_json_at(&client(), request, level).await;

            assert_eq!(response.status, status);
            assert_eq!(
                events(&lines),
                [("ERROR", "request failed")],
                "{level} {path}"
            );
            assert_eq!(
                lines[0]["spans"],
                json!([{
                    "name": "request",
                    "method": "GET",
                    "path": template,
                    "request_id": "log-probe-4"
                }]),
                "{level} {path}"
            );
        }
    }
}

#[tokio::test]
async fn the_cause_of_a_panic_is_in_the_failure_event_only() {
    let request = TestRequest::new("GET", "/api/v1/panic/formatted");

    let (_, lines) = send_logged_json(&client(), request).await;

    let detail = lines[0]["fields"]["detail"].as_str().unwrap();
    assert!(detail.contains(PANIC_TEXT), "{detail}");
    assert!(!lines[1].to_string().contains(PANIC_TEXT), "{}", lines[1]);
}

#[tokio::test]
async fn nothing_the_caller_sent_reaches_the_log() {
    let request = with_secrets("POST", "/api/v1/length").header("x-request-id", "log-probe-3");

    let (response, lines) = send_logged_json(&client(), request).await;

    assert_eq!(response.status, 200);
    let logged = whole(&lines);
    for secret in [HEADER_SECRET, QUERY_SECRET, BODY_SECRET] {
        assert!(!logged.contains(secret), "{secret} in {logged}");
    }
    for named in ["POST", "/api/v1/length", "log-probe-3"] {
        assert!(logged.contains(named), "{named} not in {logged}");
    }
}

#[tokio::test]
async fn nothing_the_caller_sent_reaches_the_log_whatever_the_answer() {
    for (method, path, status) in [
        ("GET", format!("/api/v1/panic/{PATH_SECRET}"), 500),
        ("GET", format!("/api/v1/{PATH_SECRET}"), 404),
        ("DELETE", "/api/v1/small".to_owned(), 405),
        ("POST", "/api/v1/members".to_owned(), 422),
    ] {
        let (response, lines) = send_logged_json(&client(), with_secrets(method, &path)).await;

        assert_eq!(response.status, status, "{path}");
        let logged = whole(&lines);
        assert!(logged.contains("request finished"), "{logged}");
        for secret in [HEADER_SECRET, QUERY_SECRET, BODY_SECRET, PATH_SECRET] {
            assert!(!logged.contains(secret), "{secret} in {logged}");
        }
    }
}
