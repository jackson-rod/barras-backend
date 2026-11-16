//! The public routes of `barras-api`, in-process, through the app `build_app` wires (spec §5):
//! readiness with no probe, with an optional dependency down and with a critical one down;
//! liveness; an unknown route; a wrong method; the request id. This is the card's "a sample
//! route returns both envelopes": the success envelope of a 200 and the error envelope of a 503.
#![allow(clippy::unwrap_used, clippy::expect_used)] // integration test crate: every item is test code

use std::collections::BTreeSet;

use barras_adapter_http::testing::{TestClient, TestRequest, TestResponse};
use barras_adapter_http::{Criticality, HealthProbe};
use barras_api::{Dependencies, build_app};
use serde_json::{Value, json};

/// A client of the app as the binary wires it, with `probes` for its dependencies.
fn client(probes: Vec<HealthProbe>) -> TestClient {
    let dependencies = Dependencies {
        probes,
        ..Dependencies::offline().unwrap()
    };
    TestClient::new(&build_app(dependencies).unwrap())
}

fn up(name: &'static str, criticality: Criticality) -> HealthProbe {
    HealthProbe::new(name, criticality, || async { Ok(()) })
}

fn down(name: &'static str, criticality: Criticality) -> HealthProbe {
    HealthProbe::new(name, criticality, || async {
        Err("connection refused".to_owned())
    })
}

/// Fails unless `response` is the error envelope, exactly, with `status` and `code`.
fn assert_error_envelope(response: &TestResponse, status: u16, code: &str) {
    assert_eq!(response.status, status);
    let body = response.json();
    let keys: BTreeSet<&str> = body
        .as_object()
        .unwrap()
        .keys()
        .map(String::as_str)
        .collect();
    assert_eq!(
        keys,
        BTreeSet::from(["code", "data", "error", "message", "success"])
    );
    assert_eq!(body["success"], false);
    assert!(body["message"].is_string(), "{body}");
    assert_eq!(body["data"], Value::Null);
    assert_eq!(body["code"], status);
    assert_eq!(body["error"], json!({ "code": code }));
}

#[tokio::test]
async fn with_no_probe_ready_is_ok_in_the_success_envelope() {
    let response = client(vec![]).get("/api/v1/health").await;

    assert_eq!(response.status, 200);
    assert_eq!(
        response.json(),
        json!({ "success": true, "message": "Service is ready", "data": { "status": "ok" }, "code": 200 })
    );
}

#[tokio::test]
async fn ready_is_degraded_while_an_optional_dependency_is_down() {
    let probes = vec![
        up("database", Criticality::Critical),
        down("cache", Criticality::Optional),
    ];

    let response = client(probes).get("/api/v1/health").await;

    assert_eq!(response.status, 200);
    assert_eq!(response.json()["success"], true);
    assert_eq!(response.json()["data"], json!({ "status": "degraded" }));
}

#[tokio::test]
async fn ready_is_503_in_the_error_envelope_while_a_critical_dependency_is_down() {
    let probes = vec![
        down("database", Criticality::Critical),
        up("cache", Criticality::Optional),
    ];

    let response = client(probes).get("/api/v1/health").await;

    assert_error_envelope(&response, 503, "SERVICE_UNAVAILABLE");
    let body = String::from_utf8_lossy(response.bytes());
    assert!(!body.contains("database"), "{body}");
    assert!(!body.contains("connection refused"), "{body}");
}

#[tokio::test]
async fn live_is_ok_even_while_a_critical_dependency_is_down() {
    let probes = vec![down("database", Criticality::Critical)];

    let response = client(probes).get("/api/v1/health/live").await;

    assert_eq!(response.status, 200);
    assert_eq!(
        response.json(),
        json!({ "success": true, "message": "Service is alive", "data": { "status": "ok" }, "code": 200 })
    );
}

#[tokio::test]
async fn an_unknown_route_is_route_not_found_in_the_error_envelope() {
    let client = client(vec![]);

    for path in ["/api/v1/unknown", "/unknown", "/metrics"] {
        let response = client.get(path).await;

        assert_error_envelope(&response, 404, "ROUTE_NOT_FOUND");
    }
}

#[tokio::test]
async fn a_wrong_method_is_method_not_allowed_and_names_the_allowed_ones() {
    let request = TestRequest::new("DELETE", "/api/v1/health");

    let response = client(vec![]).send(request).await;

    assert_error_envelope(&response, 405, "METHOD_NOT_ALLOWED");
    let allow = response.header("allow").unwrap();
    assert!(allow.contains("GET"), "{allow}");
}

#[tokio::test]
async fn every_answer_carries_a_new_request_id_when_none_was_sent() {
    let client = client(vec![down("database", Criticality::Critical)]);

    for (method, path, status) in [
        ("GET", "/api/v1/health/live", 200),
        ("GET", "/api/v1/health", 503),
        ("GET", "/api/v1/unknown", 404),
        ("DELETE", "/api/v1/health", 405),
    ] {
        let response = client.send(TestRequest::new(method, path)).await;

        assert_eq!(response.status, status, "{method} {path}");
        let id = response.header("x-request-id").unwrap();
        // A UUID version 7: the version is the first digit of the third group.
        assert_eq!(id.len(), 36, "{id}");
        assert_eq!(id.split('-').nth(2).unwrap().get(..1), Some("7"), "{id}");
    }
}

#[tokio::test]
async fn a_valid_request_id_from_the_caller_is_kept() {
    let request = TestRequest::new("GET", "/api/v1/health/live").header("x-request-id", "app-4f2c");

    let response = client(vec![]).send(request).await;

    assert_eq!(response.header("x-request-id"), Some("app-4f2c"));
}
