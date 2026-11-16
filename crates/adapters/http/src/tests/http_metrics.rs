//! The HTTP metrics (spec §6.6): every answer the app gives is counted and timed under its
//! method, its route template and its status, in the registry the app was built with.

use serde_json::{Value, json};

use super::support::{SERVICE, fixture_app_with, fixture_metrics, send_logged_json};
use crate::Metrics;
use crate::testing::{TestClient, TestRequest};

const MIB: usize = 1024 * 1024;

/// A client of the fixture app, and the metrics the app records into.
fn recorded() -> (TestClient, Metrics) {
    let metrics = fixture_metrics();
    let client = TestClient::new(&fixture_app_with(metrics.clone()));
    (client, metrics)
}

/// The samples of the metric `name` that `metrics` renders, without the `# HELP` and
/// `# TYPE` lines.
fn samples(metrics: &Metrics, name: &str) -> Vec<String> {
    metrics
        .render()
        .unwrap()
        .lines()
        .filter(|line| line.split(['{', ' ']).next() == Some(name))
        .map(str::to_owned)
        .collect()
}

/// The value of the label `name` in the sample `line`.
fn label<'line>(line: &'line str, name: &str) -> &'line str {
    let (_, after) = line.split_once(&format!("{name}=\"")).unwrap();
    after.split('"').next().unwrap()
}

fn get(path: &str) -> TestRequest {
    TestRequest::new("GET", path)
}

#[tokio::test]
async fn a_request_a_route_served_is_counted_under_its_template() {
    let (client, metrics) = recorded();

    client.get("/api/v1/bare/200").await;

    assert_eq!(
        samples(&metrics, "http_requests_total"),
        [r#"http_requests_total{method="GET",path="/api/v1/bare/{status}",status="200"} 1"#]
    );
}

#[tokio::test]
async fn a_path_no_route_has_is_counted_as_unmatched() {
    let (client, metrics) = recorded();

    for path in ["/api/v1/nowhere", "/nowhere", "/api/v1/small/extra"] {
        client.get(path).await;
    }

    assert_eq!(
        samples(&metrics, "http_requests_total"),
        [r#"http_requests_total{method="GET",path="unmatched",status="404"} 3"#]
    );
}

#[tokio::test]
async fn a_method_the_path_lacks_is_counted_under_the_template_it_matched() {
    let (client, metrics) = recorded();

    client
        .send(TestRequest::new("DELETE", "/api/v1/small"))
        .await;

    assert_eq!(
        samples(&metrics, "http_requests_total"),
        [r#"http_requests_total{method="DELETE",path="/api/v1/small",status="405"} 1"#]
    );
}

#[tokio::test]
async fn each_standard_method_is_counted_under_its_own_name() {
    let (client, metrics) = recorded();
    let mut standard = [
        "GET", "HEAD", "POST", "PUT", "PATCH", "DELETE", "OPTIONS", "TRACE", "CONNECT",
    ];

    for method in standard {
        client.send(TestRequest::new(method, "/api/v1/small")).await;
    }

    let counted = samples(&metrics, "http_requests_total");
    let mut methods: Vec<&str> = counted.iter().map(|line| label(line, "method")).collect();
    methods.sort_unstable();
    standard.sort_unstable();
    assert_eq!(methods, standard);
}

#[tokio::test]
async fn a_method_outside_the_standard_set_is_counted_as_other() {
    let (client, metrics) = recorded();

    for method in ["PURGE", "get", "X-SECRET-METHOD"] {
        client.send(TestRequest::new(method, "/api/v1/small")).await;
    }

    assert_eq!(
        samples(&metrics, "http_requests_total"),
        [r#"http_requests_total{method="other",path="/api/v1/small",status="405"} 3"#]
    );
}

#[tokio::test]
async fn the_status_counted_is_the_one_the_caller_got() {
    let (client, metrics) = recorded();
    let refused = get("/api/v1/small").header("accept-encoding", "identity;q=0");
    let too_large = TestRequest::new("POST", "/api/v1/length").body(vec![b'a'; MIB + 1]);

    for request in [refused, get("/api/v1/panic/formatted"), too_large] {
        client.send(request).await;
    }

    assert_eq!(
        samples(&metrics, "http_requests_total"),
        [
            r#"http_requests_total{method="GET",path="/api/v1/panic/{payload}",status="500"} 1"#,
            r#"http_requests_total{method="GET",path="/api/v1/small",status="406"} 1"#,
            r#"http_requests_total{method="POST",path="/api/v1/length",status="413"} 1"#,
        ]
    );
}

#[tokio::test]
async fn every_request_is_timed_in_the_buckets_of_the_spec() {
    let (client, metrics) = recorded();

    client.get("/api/v1/small").await;

    let buckets = samples(&metrics, "http_request_duration_seconds_bucket");
    let edges: Vec<&str> = buckets.iter().map(|line| label(line, "le")).collect();
    assert_eq!(
        edges,
        [
            "0.005", "0.01", "0.025", "0.05", "0.08", "0.1", "0.15", "0.2", "0.5", "1", "2.5", "5",
            "+Inf"
        ]
    );
    for line in &buckets {
        let route =
            r#"http_request_duration_seconds_bucket{method="GET",path="/api/v1/small",le=""#;
        assert!(line.starts_with(route), "{line}");
    }
    assert_eq!(
        samples(&metrics, "http_request_duration_seconds_count"),
        [r#"http_request_duration_seconds_count{method="GET",path="/api/v1/small"} 1"#]
    );
}

#[tokio::test]
async fn two_metrics_values_do_not_see_each_others_counts() {
    let (first, first_metrics) = recorded();
    let (_second, second_metrics) = recorded();

    first.get("/api/v1/small").await;

    assert_eq!(samples(&first_metrics, "http_requests_total").len(), 1);
    assert!(samples(&second_metrics, "http_requests_total").is_empty());
}

#[tokio::test]
async fn a_request_the_metrics_cannot_record_is_answered_and_logged_once() {
    let metrics = Metrics::unrecordable(SERVICE).unwrap();
    let client = TestClient::new(&fixture_app_with(metrics));

    let (response, lines) = send_logged_json(&client, get("/api/v1/small")).await;

    assert_eq!(response.status, 200);
    assert_eq!(response.json()["data"], json!({ "text": "hello" }));
    let warnings: Vec<&Value> = lines
        .iter()
        .filter(|line| line["level"] == "WARN")
        .collect();
    assert_eq!(warnings.len(), 1, "{lines:?}");
    let fields = warnings[0]["fields"].as_object().unwrap();
    let mut keys: Vec<&str> = fields.keys().map(String::as_str).collect();
    keys.sort_unstable();
    assert_eq!(keys, ["error", "message"]);
    let error = fields["error"].as_str().unwrap();
    for value in ["GET", "/api/v1/small", "200"] {
        assert!(!error.contains(value), "{error}");
    }
}
