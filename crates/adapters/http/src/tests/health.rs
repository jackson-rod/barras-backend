//! Health (spec §5): the two public routes, whose answers never name a dependency, and the
//! internal port, which details every probe and serves the metrics. Both ports answer from one
//! run of the probes, shared by the requests that come while it goes, and given again for a
//! second (BAR-4 spec §6.3).

use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::Duration;

use serde_json::{Value, json};
use tokio::time::Instant;

use super::support::{SERVICE, assert_error, fixture_metrics, send_logged_json};
use crate::testing::{TestClient, TestRequest, TestResponse};
use crate::{ApiError, Criticality, HealthProbe, HttpApp, InternalApp, Metrics, metadata_problems};

/// A probe name and an error that must never reach a public answer.
const SECRET_NAME: &str = "probe-secret-name";
const SECRET_ERROR: &str = "password=hunter2-secret refused";

fn up(name: &'static str, criticality: Criticality) -> HealthProbe {
    HealthProbe::new(name, criticality, || async { Ok(()) })
}

fn down(name: &'static str, criticality: Criticality, error: &'static str) -> HealthProbe {
    HealthProbe::new(
        name,
        criticality,
        move || async move { Err(error.to_owned()) },
    )
}

/// A probe that answers `answer` once `delay` has passed.
fn after(
    delay: Duration,
    name: &'static str,
    criticality: Criticality,
    answer: Result<(), &'static str>,
) -> HealthProbe {
    HealthProbe::new(name, criticality, move || async move {
        tokio::time::sleep(delay).await;
        answer.map_err(str::to_owned)
    })
}

/// A probe that never answers.
fn silent(name: &'static str, criticality: Criticality) -> HealthProbe {
    HealthProbe::new(name, criticality, std::future::pending)
}

/// A probe whose check panics.
fn panicking(name: &'static str, criticality: Criticality) -> HealthProbe {
    HealthProbe::new(name, criticality, || async { panic!("probe-panic-secret") })
}

/// A check that panics as soon as it is called, before it has a future to give.
fn panic_when_called() -> std::future::Ready<Result<(), String>> {
    panic!("probe-panic-secret")
}

/// A critical probe that passes once `delay` has passed, counting each run of its check in
/// `runs`.
fn counted(name: &'static str, runs: &Arc<AtomicUsize>, delay: Duration) -> HealthProbe {
    let runs = Arc::clone(runs);
    HealthProbe::new(name, Criticality::Critical, move || {
        runs.fetch_add(1, Ordering::SeqCst);
        async move {
            tokio::time::sleep(delay).await;
            Ok(())
        }
    })
}

/// A counter of the runs of a check.
fn counter() -> Arc<AtomicUsize> {
    Arc::new(AtomicUsize::new(0))
}

fn count(runs: &Arc<AtomicUsize>) -> usize {
    runs.load(Ordering::SeqCst)
}

fn app_with(probes: Vec<HealthProbe>) -> HttpApp {
    let builder = HttpApp::builder(fixture_metrics());
    let builder = probes
        .into_iter()
        .fold(builder, |builder, probe| builder.probe(probe));
    builder.build().unwrap()
}

/// A client of the public routes of an app with `probes`.
fn public_client(probes: Vec<HealthProbe>) -> TestClient {
    TestClient::new(&app_with(probes))
}

/// A client of the internal port of an app with `probes`.
fn internal_client(probes: Vec<HealthProbe>) -> TestClient {
    TestClient::internal(app_with(probes).internal())
}

fn text(response: &TestResponse) -> String {
    String::from_utf8_lossy(response.bytes()).into_owned()
}

#[tokio::test]
async fn with_no_probe_ready_is_ok() {
    let response = public_client(vec![]).get("/api/v1/health").await;

    assert_eq!(response.status, 200, "{}", text(&response));
    assert_eq!(
        response.json(),
        json!({ "success": true, "message": "Service is ready", "data": { "status": "ok" }, "code": 200 })
    );
}

#[tokio::test]
async fn ready_is_ok_when_every_probe_passes() {
    let probes = vec![
        up("database", Criticality::Critical),
        up("cache", Criticality::Optional),
    ];

    let response = public_client(probes).get("/api/v1/health").await;

    assert_eq!(response.status, 200);
    assert_eq!(response.json()["data"], json!({ "status": "ok" }));
}

#[tokio::test]
async fn an_optional_probe_failing_is_degraded() {
    let probes = vec![
        up("database", Criticality::Critical),
        down("cache", Criticality::Optional, "connection refused"),
    ];

    let response = public_client(probes).get("/api/v1/health").await;

    assert_eq!(response.status, 200);
    assert_eq!(
        response.json(),
        json!({
            "success": true,
            "message": "Service is ready but degraded",
            "data": { "status": "degraded" },
            "code": 200
        })
    );
}

#[tokio::test]
async fn a_critical_probe_failing_is_service_unavailable() {
    let probes = vec![
        down("database", Criticality::Critical, "connection refused"),
        up("cache", Criticality::Optional),
    ];

    let response = public_client(probes).get("/api/v1/health").await;

    assert_error(&response, 503, ApiError::SERVICE_UNAVAILABLE);
}

#[tokio::test]
async fn live_is_ok_whatever_the_probes_say() {
    let probes = vec![
        down("database", Criticality::Critical, "connection refused"),
        silent("cache", Criticality::Critical),
    ];

    let response = public_client(probes).get("/api/v1/health/live").await;

    assert_eq!(response.status, 200);
    assert_eq!(
        response.json(),
        json!({ "success": true, "message": "Service is alive", "data": { "status": "ok" }, "code": 200 })
    );
}

#[tokio::test(start_paused = true)]
async fn a_probe_that_never_answers_fails_after_two_seconds() {
    for (criticality, status) in [(Criticality::Critical, 503), (Criticality::Optional, 200)] {
        let client = public_client(vec![silent("database", criticality)]);
        let started = Instant::now();

        let response = client.get("/api/v1/health").await;

        assert_eq!(started.elapsed(), Duration::from_secs(2), "{criticality:?}");
        assert_eq!(response.status, status, "{criticality:?}");
        if status == 200 {
            assert_eq!(response.json()["data"], json!({ "status": "degraded" }));
        }
    }
}

#[tokio::test(start_paused = true)]
async fn the_probes_run_at_once() {
    let second = Duration::from_millis(1500);
    let probes = vec![
        after(second, "database", Criticality::Critical, Ok(())),
        after(second, "cache", Criticality::Critical, Ok(())),
    ];
    let client = public_client(probes);
    let started = Instant::now();

    let response = client.get("/api/v1/health").await;

    // One after the other, they would take three seconds.
    assert_eq!(started.elapsed(), second);
    assert_eq!(response.status, 200);
    assert_eq!(response.json()["data"], json!({ "status": "ok" }));
}

#[tokio::test(start_paused = true)]
async fn the_public_answers_never_name_a_probe_or_its_error() {
    for probes in [
        vec![down(SECRET_NAME, Criticality::Optional, SECRET_ERROR)],
        vec![down(SECRET_NAME, Criticality::Critical, SECRET_ERROR)],
        vec![silent(SECRET_NAME, Criticality::Critical)],
        vec![panicking(SECRET_NAME, Criticality::Optional)],
    ] {
        let client = public_client(probes);

        for path in ["/api/v1/health", "/api/v1/health/live"] {
            let response = client.get(path).await;

            let sent = text(&response);
            for secret in [
                SECRET_NAME,
                SECRET_ERROR,
                "hunter2-secret",
                "probe-panic-secret",
            ] {
                assert!(!sent.contains(secret), "{path}: {sent}");
            }
            for header in ["content-type", "x-request-id"] {
                let value = response.header(header).unwrap_or_default();
                assert!(!value.contains(SECRET_NAME), "{path}: {header}: {value}");
            }
        }
    }
}

#[tokio::test(start_paused = true)]
async fn a_silent_probe_beside_one_that_answers_fails_alone_after_two_seconds() {
    let probes = || {
        vec![
            silent("cache", Criticality::Optional),
            up("database", Criticality::Critical),
        ]
    };
    let started = Instant::now();

    let public = public_client(probes()).get("/api/v1/health").await;
    let internal = internal_client(probes()).get("/health").await;

    assert_eq!(started.elapsed(), Duration::from_secs(4));
    assert_eq!(public.status, 200);
    assert_eq!(public.json()["data"], json!({ "status": "degraded" }));
    let dependencies = internal.json()["dependencies"].clone();
    assert_eq!(dependencies[0]["status"], "down");
    assert_eq!(dependencies[0]["latencyMs"], 2000);
    assert_eq!(dependencies[1]["status"], "up");
    assert_eq!(dependencies[1]["latencyMs"], 0);
}

#[tokio::test]
async fn a_check_that_panics_when_called_counts_as_failed_on_both_ports() {
    for (criticality, status) in [(Criticality::Optional, 200), (Criticality::Critical, 503)] {
        let app = app_with(vec![
            HealthProbe::new("database", criticality, panic_when_called),
            up("cache", Criticality::Optional),
        ]);

        let public = TestClient::new(&app).get("/api/v1/health").await;
        let internal = TestClient::internal(app.internal()).get("/health").await;

        if status == 200 {
            assert_eq!(public.status, 200, "{}", text(&public));
            assert_eq!(public.json()["data"], json!({ "status": "degraded" }));
        } else {
            assert_error(&public, 503, ApiError::SERVICE_UNAVAILABLE);
        }
        assert_eq!(internal.status, status, "{}", text(&internal));
        let database = &internal.json()["dependencies"][0];
        assert_eq!(database["status"], "down", "{criticality:?}");
        assert_eq!(database["error"], "stopped without an answer");
        assert_eq!(internal.json()["dependencies"][1]["status"], "up");
    }
}

#[tokio::test]
async fn a_degraded_service_warns_of_each_optional_dependency_down_and_why() {
    let client = public_client(vec![
        down("cache", Criticality::Optional, "connection refused"),
        down("search", Criticality::Optional, "timed out"),
        up("database", Criticality::Critical),
    ]);
    // tracing decides whether anyone listens to a call site when it is first reached, and
    // while one subscriber alone is registered it asks only the reaching thread's: a test with
    // none, reaching this warning first on another thread, could silence it here. Reached once
    // before, it is decided again when this test's subscriber is registered.
    client.get("/api/v1/health").await;

    let (response, lines) =
        send_logged_json(&client, TestRequest::new("GET", "/api/v1/health")).await;

    assert_eq!(response.status, 200);
    let warnings: Vec<&Value> = lines
        .iter()
        .filter(|line| line["level"] == "WARN")
        .collect();
    assert_eq!(warnings.len(), 1, "{lines:?}");
    assert_eq!(warnings[0]["fields"]["message"], "service degraded");
    assert_eq!(
        warnings[0]["fields"]["detail"],
        "optional dependencies are down: cache (connection refused), search (timed out)"
    );
    assert!(
        warnings[0]["spans"][0]["request_id"].is_string(),
        "{}",
        warnings[0]
    );
}

#[tokio::test]
async fn an_unavailable_service_logs_each_critical_dependency_down_and_why() {
    let client = public_client(vec![
        down("database", Criticality::Critical, "connection refused"),
        down("ledger", Criticality::Critical, "disk full"),
        down("cache", Criticality::Optional, "cache-error-not-logged"),
        up("queue", Criticality::Critical),
    ]);

    let (response, lines) =
        send_logged_json(&client, TestRequest::new("GET", "/api/v1/health")).await;

    assert_eq!(response.status, 503);
    let failures: Vec<&Value> = lines
        .iter()
        .filter(|line| line["fields"]["message"] == "request failed")
        .collect();
    assert_eq!(failures.len(), 1, "{lines:?}");
    assert_eq!(failures[0]["fields"]["code"], ApiError::SERVICE_UNAVAILABLE);
    assert_eq!(
        failures[0]["fields"]["detail"],
        "critical dependencies are down: database (connection refused), ledger (disk full)"
    );
}

#[tokio::test(start_paused = true)]
async fn the_internal_health_details_every_probe_in_the_order_given() {
    let client = internal_client(vec![
        after(
            Duration::from_millis(30),
            "database",
            Criticality::Critical,
            Ok(()),
        ),
        after(
            Duration::from_millis(40),
            "cache",
            Criticality::Optional,
            Err("connection refused"),
        ),
        up("queue", Criticality::Optional),
    ]);

    let response = client.get("/health").await;

    assert_eq!(response.status, 200);
    assert_eq!(response.header("content-type"), Some("application/json"));
    assert_eq!(
        response.json(),
        json!({
            "status": "degraded",
            "service": "barras-fixture",
            "version": "1.2.3",
            "dependencies": [
                { "name": "database", "critical": true, "status": "up", "latencyMs": 30 },
                {
                    "name": "cache",
                    "critical": false,
                    "status": "down",
                    "latencyMs": 40,
                    "error": "connection refused"
                },
                { "name": "queue", "critical": false, "status": "up", "latencyMs": 0 },
            ]
        })
    );
}

#[tokio::test]
async fn the_internal_health_is_503_when_a_critical_probe_fails() {
    let client = internal_client(vec![
        down("database", Criticality::Critical, "connection refused"),
        up("cache", Criticality::Optional),
    ]);

    let response = client.get("/health").await;

    assert_eq!(response.status, 503);
    let body = response.json();
    assert_eq!(body["status"], "unavailable");
    assert_eq!(body["dependencies"][0]["status"], "down");
    assert_eq!(body["dependencies"][0]["error"], "connection refused");
    assert_eq!(body["dependencies"][1]["status"], "up");
    assert_eq!(body["dependencies"][1].get("error"), None);
}

#[tokio::test]
async fn the_internal_health_with_no_probe_is_ok() {
    let response = internal_client(vec![]).get("/health").await;

    assert_eq!(response.status, 200);
    assert_eq!(
        response.json(),
        json!({ "status": "ok", "service": "barras-fixture", "version": "1.2.3", "dependencies": [] })
    );
}

#[tokio::test(start_paused = true)]
async fn the_internal_health_says_a_silent_probe_ran_out_of_time() {
    let response = internal_client(vec![silent("database", Criticality::Optional)])
        .get("/health")
        .await;

    assert_eq!(response.status, 200);
    assert_eq!(
        response.json()["dependencies"],
        json!([{
            "name": "database",
            "critical": false,
            "status": "down",
            "latencyMs": 2000,
            "error": "did not answer within 2 seconds"
        }])
    );
}

#[tokio::test]
async fn a_probe_whose_check_panics_is_down_and_the_others_still_answer() {
    let client = internal_client(vec![
        panicking("database", Criticality::Critical),
        up("cache", Criticality::Optional),
    ]);

    let response = client.get("/health").await;

    assert_eq!(response.status, 503);
    let body = response.json();
    assert_eq!(body["dependencies"][0]["status"], "down");
    assert_eq!(
        body["dependencies"][0]["error"],
        "stopped without an answer"
    );
    assert_eq!(body["dependencies"][1]["status"], "up");
    assert!(!text(&response).contains("probe-panic-secret"));
}

#[tokio::test]
async fn the_internal_metrics_are_the_text_of_the_registry() {
    let metrics = fixture_metrics();
    let app = HttpApp::builder(metrics.clone()).build().unwrap();
    let client = TestClient::internal(app.internal());

    let response = client.get("/metrics").await;

    assert_eq!(response.status, 200);
    assert_eq!(
        response.header("content-type"),
        Some("text/plain; version=0.0.4")
    );
    assert_eq!(text(&response), metrics.render().unwrap());
    assert!(
        text(&response)
            .contains(r#"barras_build_info{service="barras-fixture",version="1.2.3"} 1"#)
    );
}

#[tokio::test]
async fn a_request_to_the_internal_port_is_not_counted() {
    let metrics = fixture_metrics();
    let app = HttpApp::builder(metrics.clone()).build().unwrap();
    let internal = TestClient::internal(app.internal());

    for path in ["/health", "/metrics", "/nowhere"] {
        internal.get(path).await;
    }

    assert!(
        !metrics.render().unwrap().contains("http_requests_total"),
        "{}",
        metrics.render().unwrap()
    );
    TestClient::new(&app).get("/api/v1/health").await;
    assert!(metrics.render().unwrap().contains("http_requests_total"));
}

#[tokio::test]
async fn an_internal_app_built_alone_serves_both_routes() {
    let probes = vec![up("database", Criticality::Critical)];
    let internal = InternalApp::new(fixture_metrics(), probes);
    let client = TestClient::internal(&internal);

    let health = client.get("/health").await;
    let metrics = client.get("/metrics").await;

    assert_eq!(health.status, 200);
    assert_eq!(health.json()["dependencies"][0]["name"], "database");
    assert_eq!(metrics.status, 200);
}

#[tokio::test]
async fn metrics_that_cannot_be_rendered_answer_500_and_log_the_whole_chain() {
    let metrics = Metrics::unrenderable(SERVICE).unwrap();
    let client = TestClient::internal(&InternalApp::new(metrics, vec![]));

    let (response, lines) = send_logged_json(&client, TestRequest::new("GET", "/metrics")).await;

    assert_eq!(response.status, 500);
    assert!(response.bytes().is_empty(), "{}", text(&response));
    assert_eq!(lines.len(), 1, "{lines:?}");
    assert_eq!(lines[0]["level"], "ERROR");
    assert_eq!(
        lines[0]["fields"]["message"],
        "the metrics could not be served"
    );
    let error = lines[0]["fields"]["error"].as_str().unwrap();
    assert!(
        error.starts_with("the metrics could not be written as text: "),
        "{error}"
    );
    assert!(error.contains("MetricFamily has no name"), "{error}");
}

#[tokio::test]
async fn the_public_routes_are_not_on_the_internal_port() {
    let client = internal_client(vec![]);

    for path in ["/api/v1/health", "/api/v1/health/live", "/health/live", "/"] {
        let response = client.get(path).await;

        assert_eq!(response.status, 404, "{path}");
    }
}

#[tokio::test]
async fn the_internal_routes_are_not_on_the_public_port() {
    let client = public_client(vec![]);

    for path in ["/health", "/metrics", "/api/v1/metrics"] {
        let response = client.get(path).await;

        assert_error(&response, 404, ApiError::ROUTE_NOT_FOUND);
    }
}

#[test]
fn an_app_with_only_the_health_routes_has_complete_metadata() {
    let app = app_with(vec![]);

    assert_eq!(metadata_problems(app.openapi()), Vec::<String>::new());
    let document = serde_json::to_value(app.openapi()).unwrap();
    let paths: Vec<&str> = document["paths"]
        .as_object()
        .unwrap()
        .keys()
        .map(String::as_str)
        .collect();
    assert_eq!(paths, ["/api/v1/health", "/api/v1/health/live"]);
}

#[test]
fn the_health_routes_document_their_tag_their_codes_and_their_answers() {
    let document = serde_json::to_value(app_with(vec![]).openapi()).unwrap();

    let live = &document["paths"]["/api/v1/health/live"]["get"];
    let ready = &document["paths"]["/api/v1/health"]["get"];
    for operation in [live, ready] {
        assert_eq!(operation["tags"], json!(["health"]), "{operation}");
        assert_eq!(operation["x-permission"], "public");
        assert_eq!(operation["x-plan-feature"], "none");
        assert_eq!(
            operation["responses"]["200"]["content"]["application/json"]["schema"],
            json!({ "$ref": "#/components/schemas/SuccessEnvelope_HealthStatus" })
        );
    }
    assert_eq!(live["x-error-codes"], json!([]));
    assert_eq!(ready["x-error-codes"], json!(["SERVICE_UNAVAILABLE"]));
    assert_eq!(
        ready["responses"]["503"]["content"]["application/json"]["schema"],
        json!({ "$ref": "#/components/schemas/ErrorEnvelope" })
    );
    assert_eq!(live["responses"].as_object().unwrap().len(), 1);
    let status = &document["components"]["schemas"]["HealthStatus"]["properties"]["status"];
    assert_eq!(
        status,
        &json!({
            "type": "string",
            "enum": ["ok", "degraded"],
            "description": "`ok`, or `degraded` while a dependency the service can do without is down."
        })
    );
}

#[tokio::test(start_paused = true)]
async fn two_requests_at_the_same_time_run_each_check_once() {
    let (database, cache) = (counter(), counter());
    let delay = Duration::from_millis(100);
    let client = public_client(vec![
        counted("database", &database, delay),
        counted("cache", &cache, delay),
    ]);
    let started = Instant::now();

    let (first, second) = tokio::join!(client.get("/api/v1/health"), client.get("/api/v1/health"));

    assert_eq!((count(&database), count(&cache)), (1, 1));
    // The second waited for the run the first started, and none after it.
    assert_eq!(started.elapsed(), delay);
    for response in [first, second] {
        assert_eq!(response.status, 200, "{}", text(&response));
        assert_eq!(response.json()["data"], json!({ "status": "ok" }));
    }
}

#[tokio::test(start_paused = true)]
async fn a_request_less_than_a_second_after_a_run_is_answered_from_it() {
    let runs = counter();
    let client = public_client(vec![counted("database", &runs, Duration::ZERO)]);
    client.get("/api/v1/health").await;

    tokio::time::sleep(Duration::from_millis(999)).await;
    let response = client.get("/api/v1/health").await;

    assert_eq!(count(&runs), 1);
    assert_eq!(response.status, 200);
    assert_eq!(response.json()["data"], json!({ "status": "ok" }));
}

#[tokio::test(start_paused = true)]
async fn a_request_a_second_after_a_run_runs_the_checks_again() {
    let runs = counter();
    let client = public_client(vec![counted("database", &runs, Duration::ZERO)]);
    client.get("/api/v1/health").await;

    tokio::time::sleep(Duration::from_secs(1)).await;
    let response = client.get("/api/v1/health").await;

    assert_eq!(count(&runs), 2);
    assert_eq!(response.status, 200);
}

#[tokio::test(start_paused = true)]
async fn the_public_and_the_internal_routes_share_a_run() {
    let runs = counter();
    let app = app_with(vec![counted("database", &runs, Duration::ZERO)]);

    let public = TestClient::new(&app).get("/api/v1/health").await;
    let internal = TestClient::internal(app.internal()).get("/health").await;

    assert_eq!(count(&runs), 1);
    assert_eq!(public.status, 200);
    assert_eq!(internal.status, 200);
    assert_eq!(internal.json()["dependencies"][0]["status"], "up");
}

#[tokio::test(start_paused = true)]
async fn a_shared_run_still_reports_a_check_that_panics_or_never_answers() {
    let runs = counter();
    let app = app_with(vec![
        silent("cache", Criticality::Optional),
        panicking("search", Criticality::Optional),
        counted("database", &runs, Duration::ZERO),
    ]);
    let (public_port, internal_port) =
        (TestClient::new(&app), TestClient::internal(app.internal()));
    let started = Instant::now();

    let (public, internal) = tokio::join!(
        public_port.get("/api/v1/health"),
        internal_port.get("/health")
    );

    assert_eq!(count(&runs), 1);
    assert_eq!(started.elapsed(), Duration::from_secs(2));
    assert_eq!(public.status, 200, "{}", text(&public));
    assert_eq!(public.json()["data"], json!({ "status": "degraded" }));
    assert_eq!(internal.status, 200);
    // A check that panicked took no longer than the whole run, the one time known of it.
    assert_eq!(
        internal.json()["dependencies"],
        json!([
            {
                "name": "cache",
                "critical": false,
                "status": "down",
                "latencyMs": 2000,
                "error": "did not answer within 2 seconds"
            },
            {
                "name": "search",
                "critical": false,
                "status": "down",
                "latencyMs": 2000,
                "error": "stopped without an answer"
            },
            { "name": "database", "critical": true, "status": "up", "latencyMs": 0 },
        ])
    );
}

#[tokio::test(start_paused = true)]
async fn a_run_goes_on_when_its_request_goes_away_and_answers_the_next_one() {
    let runs = counter();
    let delay = Duration::from_millis(100);
    let client = public_client(vec![counted("database", &runs, delay)]);
    let started = Instant::now();
    // As a client that closes its connection does, which drops its request.
    let gone = tokio::time::timeout(delay / 2, client.get("/api/v1/health")).await;
    assert!(gone.is_err(), "the first request was answered");

    let next = tokio::time::timeout(Duration::from_secs(10), client.get("/api/v1/health")).await;

    let response = next.expect("the next request had no answer");
    // Leaving starts runs no faster than waiting does.
    assert_eq!(count(&runs), 1);
    assert_eq!(started.elapsed(), delay);
    assert_eq!(response.status, 200);
    assert_eq!(response.json()["data"], json!({ "status": "ok" }));
}
