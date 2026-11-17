//! The time limit of a request (spec §6.4): an answer that has not started five seconds after
//! the request arrived, the reading of its body included, is a 503 `REQUEST_TIMEOUT` that is
//! logged, counted and carries the request id, and its handler is dropped, a handler that panics
//! as it is dropped included. A request whose body is still arriving then is the client's
//! slowness instead: a 408 `REQUEST_BODY_TIMEOUT` that closes the connection and logs no
//! failure. A body that streams out after the head is not limited. Every test pauses tokio's
//! clock, so none of them waits.

use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::Duration;

use axum::body::{Body, Bytes, to_bytes};
use axum::extract::{Path, State};
use axum::http::header::CONTENT_TYPE;
use axum::http::{Request, StatusCode};
use axum::response::{IntoResponse, Response};
use http_body_util::{BodyExt, Full};
use serde_json::{Value, json};
use tokio::time::Instant;
use tower::ServiceExt;
use tracing::Level;
use utoipa_axum::routes;

use super::support::{
    assert_error, fixture_metrics, fixture_routes, logged_json, send_logged_json,
};
use crate::middleware::REQUEST_TIME_LIMIT;
use crate::testing::{TestClient, TestRequest};
use crate::{
    Access, ApiError, ApiResult, ApiRouter, ApiSuccess, HttpApp, Metrics, NoData, SuccessEnvelope,
};

/// A wait far over the limit, for the handler that must not be waited for.
const OVER_THE_LIMIT: &str = "/api/v1/wait/60000";

/// What the streaming route sends at once, before its body ends.
const STREAMED: &str = "the first and only part of the body";

/// What the value the fragile route holds panics with as it is dropped: it must reach the log
/// and never a body.
const DROP_PANIC_TEXT: &str = "drop-panic-secret from the fixture";

/// Counts the runs of the waiting handler: those that began, and those still going. A run stops
/// going when it ends or when it is dropped.
#[derive(Clone, Default)]
struct Runs {
    began: Arc<AtomicUsize>,
    going: Arc<AtomicUsize>,
}

impl Runs {
    fn begin(&self) -> Going {
        self.began.fetch_add(1, Ordering::SeqCst);
        self.going.fetch_add(1, Ordering::SeqCst);
        Going(Arc::clone(&self.going))
    }

    fn began(&self) -> usize {
        self.began.load(Ordering::SeqCst)
    }

    fn going(&self) -> usize {
        self.going.load(Ordering::SeqCst)
    }
}

/// Held by a run of the waiting handler for as long as the run goes.
struct Going(Arc<AtomicUsize>);

impl Drop for Going {
    fn drop(&mut self) {
        self.0.fetch_sub(1, Ordering::SeqCst);
    }
}

/// Answer once `ms` milliseconds have passed.
#[utoipa::path(
    get,
    path = "/wait/{ms}",
    tag = "fixtures",
    params(("ms" = u64, Path, description = "How long to wait, in milliseconds")),
    responses((status = 200, description = "Answered late", body = SuccessEnvelope<NoData>)),
)]
async fn wait(State(runs): State<Runs>, Path(ms): Path<u64>) -> ApiResult<NoData> {
    let _going = runs.begin();
    tokio::time::sleep(Duration::from_millis(ms)).await;
    Ok(ApiSuccess::ok("Answered late", NoData))
}

/// Read the body to its end, then answer once `ms` milliseconds have passed.
#[utoipa::path(
    post,
    path = "/wait/{ms}",
    tag = "fixtures",
    params(("ms" = u64, Path, description = "How long to wait once the body is read, in milliseconds")),
    responses((status = 200, description = "Answered late", body = SuccessEnvelope<NoData>)),
)]
async fn wait_after_the_body(Path(ms): Path<u64>, _body: Bytes) -> ApiResult<NoData> {
    tokio::time::sleep(Duration::from_millis(ms)).await;
    Ok(ApiSuccess::ok("Answered late", NoData))
}

/// Panics as it is dropped, as a value whose clean-up fails might.
struct PanicsWhenDropped;

impl Drop for PanicsWhenDropped {
    fn drop(&mut self) {
        panic!("{DROP_PANIC_TEXT}");
    }
}

/// Answer once a minute has passed, holding meanwhile a value that panics as it is dropped.
#[utoipa::path(
    get,
    path = "/fragile",
    tag = "fixtures",
    responses((status = 200, description = "Answered late", body = SuccessEnvelope<NoData>)),
)]
async fn fragile() -> ApiResult<NoData> {
    let _held = PanicsWhenDropped;
    tokio::time::sleep(Duration::from_secs(60)).await;
    Ok(ApiSuccess::ok("Answered late", NoData))
}

/// Answer at once with a body that ends only once `ms` milliseconds have passed.
#[utoipa::path(
    get,
    path = "/stream/{ms}",
    tag = "fixtures",
    params(("ms" = u64, Path, description = "When the body ends, in milliseconds")),
    responses(
        (status = 200, description = "Plain text that streams", body = SuccessEnvelope<NoData>),
    ),
)]
async fn stream(Path(ms): Path<u64>) -> Response {
    let body = ending_after(STREAMED, Duration::from_millis(ms));
    ([(CONTENT_TYPE, "text/plain")], body).into_response()
}

/// `first` at once, then the end of the body once `after` has passed: what a handler that
/// streams its answer sends, or a client that sends its body slowly.
fn ending_after(first: &'static str, after: Duration) -> Body {
    // http-body-util has no body that pauses, but it has one that awaits a future before it
    // ends: the future of its trailers, which here waits, then gives none.
    let end = async move {
        tokio::time::sleep(after).await;
        None
    };
    Body::new(Full::new(Bytes::from_static(first.as_bytes())).with_trailers(end))
}

/// A request to the length route whose body sends five bytes at once and ends six seconds later,
/// past the limit. `TestRequest` sends a body whole, so this request goes to the router itself.
fn slow_body(request_id: &str) -> Request<Body> {
    Request::post("/api/v1/length")
        .header("x-request-id", request_id)
        .body(ending_after("12345", Duration::from_secs(6)))
        .unwrap()
}

/// The fixture app with the routes of these tests, its waiting runs counted in `runs` and its
/// answers in `metrics`.
fn app_with(runs: Runs, metrics: Metrics) -> HttpApp {
    let routes = ApiRouter::new()
        .route(routes!(wait, wait_after_the_body), Access::public(), &[])
        .route(routes!(fragile), Access::public(), &[])
        .route(routes!(stream), Access::public(), &[])
        .with_state(runs);
    HttpApp::builder(metrics)
        .routes(fixture_routes().merge(routes))
        .build()
        .unwrap()
}

fn client() -> TestClient {
    TestClient::new(&app_with(Runs::default(), fixture_metrics()))
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

#[tokio::test(start_paused = true)]
async fn a_handler_slower_than_the_limit_is_answered_503_after_five_seconds_with_the_request_id() {
    let request = TestRequest::new("GET", OVER_THE_LIMIT).header("x-request-id", "slow-handler");
    let started = Instant::now();

    let response = client().send(request).await;

    assert_eq!(REQUEST_TIME_LIMIT, Duration::from_secs(5));
    assert_eq!(started.elapsed(), REQUEST_TIME_LIMIT);
    assert_error(&response, 503, ApiError::REQUEST_TIMEOUT);
    assert_eq!(response.json()["message"], "Service unavailable");
    assert_eq!(response.header("x-request-id"), Some("slow-handler"));
}

#[tokio::test(start_paused = true)]
async fn a_request_that_times_out_is_logged_once_as_failed_then_as_finished() {
    let request = TestRequest::new("GET", OVER_THE_LIMIT).header("x-request-id", "slow-log");

    let (response, lines) = send_logged_json(&client(), request).await;

    assert_eq!(response.status, 503);
    assert_eq!(
        events(&lines),
        [("ERROR", "request failed"), ("INFO", "request finished")]
    );
    let failed = &lines[0]["fields"];
    assert_eq!(failed["code"], ApiError::REQUEST_TIMEOUT);
    assert_eq!(failed["status"], 503);
    assert_eq!(failed["detail"], "no answer within 5 seconds");
    assert_eq!(lines[1]["fields"]["status"], 503);
    for line in &lines {
        assert_eq!(line["spans"][0]["request_id"], "slow-log", "{line}");
        assert_eq!(line["spans"][0]["path"], "/api/v1/wait/{ms}", "{line}");
    }
}

#[tokio::test(start_paused = true)]
async fn a_request_that_times_out_is_counted_with_status_503() {
    let metrics = fixture_metrics();
    let client = TestClient::new(&app_with(Runs::default(), metrics.clone()));

    client.get(OVER_THE_LIMIT).await;

    let text = metrics.render().unwrap();
    let counted = r#"http_requests_total{method="GET",path="/api/v1/wait/{ms}",status="503"} 1"#;
    assert!(text.contains(counted), "{text}");
}

#[tokio::test(start_paused = true)]
async fn the_handler_of_a_request_that_times_out_is_dropped() {
    let runs = Runs::default();
    let client = TestClient::new(&app_with(runs.clone(), fixture_metrics()));

    let response = client.get(OVER_THE_LIMIT).await;

    assert_eq!(response.status, 503);
    assert_eq!(runs.began(), 1);
    assert_eq!(runs.going(), 0);
}

#[tokio::test(start_paused = true)]
async fn a_handler_panicking_when_dropped_still_gets_the_503_with_the_request_id_and_is_counted() {
    let metrics = fixture_metrics();
    let client = TestClient::new(&app_with(Runs::default(), metrics.clone()));
    let request = TestRequest::new("GET", "/api/v1/fragile").header("x-request-id", "fragile");

    let response = client.send(request).await;

    assert_error(&response, 503, ApiError::REQUEST_TIMEOUT);
    assert_eq!(response.json()["message"], "Service unavailable");
    assert_eq!(response.header("x-request-id"), Some("fragile"));
    assert!(!String::from_utf8_lossy(response.bytes()).contains(DROP_PANIC_TEXT));
    let text = metrics.render().unwrap();
    let counted = r#"http_requests_total{method="GET",path="/api/v1/fragile",status="503"} 1"#;
    assert!(text.contains(counted), "{text}");
}

#[tokio::test(start_paused = true)]
async fn a_handler_panicking_when_dropped_is_logged_once_as_failed_with_the_panic() {
    let request = TestRequest::new("GET", "/api/v1/fragile").header("x-request-id", "fragile-log");

    let (response, lines) = send_logged_json(&client(), request).await;

    assert_eq!(response.status, 503);
    assert_eq!(
        events(&lines),
        [("ERROR", "request failed"), ("INFO", "request finished")]
    );
    let failed = &lines[0]["fields"];
    assert_eq!(failed["code"], ApiError::REQUEST_TIMEOUT);
    let detail = format!(
        "no answer within 5 seconds, and the handler panicked as it was dropped: \
         {DROP_PANIC_TEXT}"
    );
    assert_eq!(failed["detail"], detail);
    assert_eq!(lines[1]["fields"]["status"], 503);
    for line in &lines {
        assert_eq!(line["spans"][0]["request_id"], "fragile-log", "{line}");
    }
}

#[tokio::test(start_paused = true)]
async fn a_body_the_client_sends_slowly_counts_against_the_limit() {
    let app = app_with(Runs::default(), fixture_metrics());
    let started = Instant::now();

    let Ok(response) = app.router().oneshot(slow_body("slow-body")).await;

    assert_eq!(started.elapsed(), REQUEST_TIME_LIMIT);
    assert_eq!(response.status(), StatusCode::REQUEST_TIMEOUT);
    assert_eq!(response.headers()["x-request-id"], "slow-body");
    assert_eq!(response.headers()["connection"], "close");
    assert_eq!(response.headers()["content-type"], "application/json");
    let body = to_bytes(response.into_body(), usize::MAX).await.unwrap();
    let body: Value = serde_json::from_slice(&body).unwrap();
    assert_eq!(body["success"], false);
    assert_eq!(body["code"], 408);
    assert_eq!(body["error"], json!({ "code": "REQUEST_BODY_TIMEOUT" }));
}

#[tokio::test(start_paused = true)]
async fn a_body_still_arriving_at_the_limit_logs_no_failure_and_is_counted_at_408() {
    let metrics = fixture_metrics();
    let app = app_with(Runs::default(), metrics.clone());
    let sent = app.router().oneshot(slow_body("slow-body-log"));

    let (Ok(response), lines) = logged_json(Level::TRACE, sent).await;

    assert_eq!(response.status(), StatusCode::REQUEST_TIMEOUT);
    assert_eq!(events(&lines), [("INFO", "request finished")]);
    assert_eq!(lines[0]["fields"]["status"], 408);
    assert_eq!(lines[0]["spans"][0]["request_id"], "slow-body-log");
    let text = metrics.render().unwrap();
    let counted = r#"http_requests_total{method="POST",path="/api/v1/length",status="408"} 1"#;
    assert!(text.contains(counted), "{text}");
}

#[tokio::test(start_paused = true)]
async fn a_body_sent_whole_then_a_slow_handler_gets_the_503() {
    for request in [
        TestRequest::new("POST", OVER_THE_LIMIT).body("12345"),
        // A stream tells its end only by giving no more, never by its length.
        TestRequest::new("POST", OVER_THE_LIMIT)
            .body("12345")
            .streamed(),
    ] {
        let response = client().send(request).await;

        assert_error(&response, 503, ApiError::REQUEST_TIMEOUT);
        assert_eq!(response.header("connection"), None);
    }
}

#[tokio::test(start_paused = true)]
async fn a_request_without_a_body_and_a_slow_handler_gets_the_503() {
    // The waiting handler never reads the body: one that is empty from the start has arrived.
    let response = client().get(OVER_THE_LIMIT).await;

    assert_error(&response, 503, ApiError::REQUEST_TIMEOUT);
    assert_eq!(response.header("connection"), None);
}

#[tokio::test(start_paused = true)]
async fn a_body_that_streams_out_after_the_head_is_not_cut() {
    let started = Instant::now();

    // The head comes at once, and the body ends six seconds later.
    let response = client().get("/api/v1/stream/6000").await;

    assert_eq!(started.elapsed(), Duration::from_secs(6));
    assert_eq!(response.status, 200);
    assert_eq!(response.bytes(), STREAMED.as_bytes());
}

#[tokio::test(start_paused = true)]
async fn a_handler_under_the_limit_is_untouched() {
    let started = Instant::now();

    let response = client().get("/api/v1/wait/4999").await;

    assert_eq!(started.elapsed(), Duration::from_millis(4999));
    assert_eq!(response.status, 200);
    assert_eq!(
        response.json(),
        json!({ "success": true, "message": "Answered late", "data": null, "code": 200 })
    );
}
