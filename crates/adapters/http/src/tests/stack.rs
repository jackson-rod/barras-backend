//! The stack every public answer goes through (spec §6.1 to §6.4): the two fallbacks, the
//! panic handler, the safety net, the request id, compression and the body limit.

use serde_json::json;
use uuid::Uuid;

use super::support::{
    BARE_TEXT, CONTENT_SIZE, KEPT_HEADER, PANIC_TEXT, SERVICE, assert_error, client, fixture_app,
    send_logged,
};
use crate::testing::{TestRequest, TestResponse};
use crate::{ApiError, COMMON_ERROR_CODES, MatchedRoute, canonical_json};

const MIB: usize = 1024 * 1024;

fn request_id(response: &TestResponse) -> &str {
    response.header("x-request-id").unwrap()
}

fn is_uuid_v7(id: &str) -> bool {
    Uuid::try_parse(id).is_ok_and(|uuid| uuid.get_version_num() == 7)
}

fn get(path: &str) -> TestRequest {
    TestRequest::new("GET", path)
}

#[tokio::test]
async fn an_unknown_path_under_the_prefix_is_route_not_found() {
    let response = client().get("/api/v1/nowhere").await;

    assert_error(&response, 404, ApiError::ROUTE_NOT_FOUND);
}

#[tokio::test]
async fn an_unknown_path_outside_the_prefix_is_route_not_found() {
    for path in ["/", "/nowhere", "/api/v2/small"] {
        let response = client().get(path).await;

        assert_error(&response, 404, ApiError::ROUTE_NOT_FOUND);
    }
}

#[tokio::test]
async fn a_known_path_with_another_method_is_method_not_allowed_and_keeps_allow() {
    let response = client()
        .send(TestRequest::new("DELETE", "/api/v1/small"))
        .await;

    assert_error(&response, 405, ApiError::METHOD_NOT_ALLOWED);
    assert_eq!(response.header("allow"), Some("GET,HEAD"));
}

#[tokio::test]
async fn a_panic_answers_the_generic_internal_error_and_only_the_log_has_its_text() {
    let (response, lines) = send_logged(&client(), get("/api/v1/panic/formatted")).await;

    assert_error(&response, 500, ApiError::INTERNAL_ERROR);
    assert_eq!(response.json()["message"], "Internal server error");
    assert!(!String::from_utf8_lossy(response.bytes()).contains(PANIC_TEXT));
    let failures: Vec<&String> = lines
        .iter()
        .filter(|line| line.contains("request failed"))
        .collect();
    assert_eq!(failures.len(), 1, "{lines:?}");
    assert!(failures[0].starts_with("ERROR"), "{lines:?}");
    assert!(failures[0].contains(PANIC_TEXT), "{lines:?}");
    assert!(failures[0].contains("code=\"INTERNAL_ERROR\""), "{lines:?}");
}

#[tokio::test]
async fn the_log_says_what_any_kind_of_panic_carried() {
    for (payload, logged) in [
        ("literal", "literal-secret from the fixture"),
        ("number", "a payload that is not text"),
    ] {
        let request = get(&format!("/api/v1/panic/{payload}"));

        let (response, lines) = send_logged(&client(), request).await;

        assert_error(&response, 500, ApiError::INTERNAL_ERROR);
        assert!(!String::from_utf8_lossy(response.bytes()).contains(logged));
        let failure = lines.iter().find(|line| line.contains("request failed"));
        assert!(
            failure.is_some_and(|line| line.contains(logged)),
            "{lines:?}"
        );
    }
}

#[tokio::test]
async fn a_bare_client_error_becomes_the_envelope_its_status_maps_to() {
    for (status, code) in [
        (400, ApiError::MALFORMED_REQUEST),
        (413, ApiError::PAYLOAD_TOO_LARGE),
        (415, ApiError::UNSUPPORTED_MEDIA_TYPE),
        (406, ApiError::NOT_ACCEPTABLE),
        (418, ApiError::MALFORMED_REQUEST),
        (429, ApiError::MALFORMED_REQUEST),
    ] {
        let response = client().get(&format!("/api/v1/bare/{status}")).await;

        assert_error(&response, status, code);
        assert!(!String::from_utf8_lossy(response.bytes()).contains(BARE_TEXT));
    }
}

#[tokio::test]
async fn a_bare_server_error_becomes_an_internal_error_logged_once() {
    for status in [500, 503] {
        let request = get(&format!("/api/v1/bare/{status}"));

        let (response, lines) = send_logged(&client(), request).await;

        assert_error(&response, status, ApiError::INTERNAL_ERROR);
        assert!(!String::from_utf8_lossy(response.bytes()).contains(BARE_TEXT));
        let failures: Vec<&String> = lines
            .iter()
            .filter(|line| line.contains("request failed"))
            .collect();
        assert_eq!(failures.len(), 1, "{lines:?}");
        assert!(
            failures[0].contains(&format!("status={status}")),
            "{lines:?}"
        );
    }
}

#[tokio::test]
async fn the_safety_net_keeps_the_headers_but_the_type_and_the_length() {
    let response = client().get("/api/v1/bare/418").await;

    assert_eq!(response.header(KEPT_HEADER), Some("kept"));
    assert_eq!(response.header("content-type"), Some("application/json"));
    let length = response.bytes().len().to_string();
    assert_eq!(response.header("content-length"), Some(length.as_str()));
}

#[tokio::test]
async fn the_safety_net_drops_the_range_and_the_validators_of_the_body_it_replaces() {
    let response = client().get("/api/v1/bare/400").await;

    assert_error(&response, 400, ApiError::MALFORMED_REQUEST);
    for name in ["content-range", "etag", "last-modified"] {
        assert_eq!(response.header(name), None, "{name}");
    }
}

#[tokio::test]
async fn a_replaced_body_that_was_compressed_comes_back_as_a_plain_envelope() {
    // The same content with a success status is compressed, so the 4xx is compressed too
    // before the safety net replaces it.
    let success = client()
        .send(get("/api/v1/content/200/text").header("accept-encoding", "gzip"))
        .await;
    assert_eq!(success.header("content-encoding"), Some("gzip"));

    let response = client()
        .send(get("/api/v1/content/400/text").header("accept-encoding", "gzip"))
        .await;

    assert_error(&response, 400, ApiError::MALFORMED_REQUEST);
    assert_eq!(response.header("content-encoding"), None);
}

#[tokio::test]
async fn a_rejection_axum_writes_on_its_own_becomes_an_envelope() {
    let response = client().get("/api/v1/bare/not-a-number").await;

    assert_error(&response, 400, ApiError::MALFORMED_REQUEST);
}

#[tokio::test]
async fn a_bare_answer_that_is_not_an_error_is_left_alone() {
    let response = client().get("/api/v1/bare/200").await;

    assert_eq!(response.status, 200);
    assert_eq!(response.bytes(), BARE_TEXT.as_bytes());
}

#[tokio::test]
#[should_panic(expected = "the body is not JSON")]
async fn reading_a_body_that_is_not_json_as_json_fails_the_test() {
    client().get("/api/v1/bare/200").await.json();
}

#[tokio::test]
async fn an_error_envelope_passes_the_safety_net_untouched() {
    let response = client().get("/api/v1/refused").await;

    assert_error(&response, 422, ApiError::VALIDATION_FAILED);
    assert_eq!(
        response.json()["error"],
        json!({ "code": "VALIDATION_FAILED", "fields": { "name": ["is required"] } })
    );
}

#[tokio::test]
async fn an_accept_encoding_that_refuses_every_encoding_is_not_acceptable() {
    for path in ["/api/v1/small", "/api/v1/big", "/api/v1/refused"] {
        let request = get(path).header("accept-encoding", "identity;q=0");

        let response = client().send(request).await;

        assert_error(&response, 406, ApiError::NOT_ACCEPTABLE);
        assert_eq!(response.header("vary"), Some("accept-encoding"));
    }
}

#[tokio::test]
async fn no_request_id_sent_gives_a_uuid_v7() {
    let response = client().get("/api/v1/small").await;

    let id = request_id(&response);
    assert!(is_uuid_v7(id), "{id}");
}

#[tokio::test]
async fn each_request_gets_its_own_id() {
    let client = client();

    let first = client.get("/api/v1/small").await;
    let second = client.get("/api/v1/small").await;

    assert_ne!(request_id(&first), request_id(&second));
}

#[tokio::test]
async fn a_well_formed_request_id_is_kept() {
    for id in ["abc-123._XYZ", "a", &"7".repeat(64)] {
        let request = get("/api/v1/small").header("x-request-id", id);

        let response = client().send(request).await;

        assert_eq!(request_id(&response), id);
    }
}

#[tokio::test]
async fn a_malformed_request_id_is_replaced_by_a_uuid_v7() {
    for id in [
        &"7".repeat(65),
        "has space",
        "semi;colon",
        "",
        "slash/es",
        "é",
    ] {
        let request = get("/api/v1/small").header("x-request-id", id);

        let response = client().send(request).await;

        let kept = request_id(&response);
        assert!(is_uuid_v7(kept), "{id:?} gave {kept:?}");
    }
}

#[tokio::test]
async fn two_request_ids_are_replaced_by_a_uuid_v7() {
    let request = get("/api/v1/small")
        .header("x-request-id", "first")
        .header("x-request-id", "second");

    let response = client().send(request).await;

    let kept = request_id(&response);
    assert!(is_uuid_v7(kept), "{kept:?}");
}

#[tokio::test]
async fn every_answer_carries_the_request_id() {
    let too_large = TestRequest::new("POST", "/api/v1/length").body(vec![b'a'; MIB + 1]);
    for (request, status) in [
        (get("/api/v1/nowhere"), 404),
        (TestRequest::new("DELETE", "/api/v1/small"), 405),
        (too_large, 413),
        (get("/api/v1/panic/formatted"), 500),
        (get("/api/v1/bare/400"), 400),
        (
            get("/api/v1/small").header("accept-encoding", "identity;q=0"),
            406,
        ),
    ] {
        let response = client()
            .send(request.header("x-request-id", "sent-by-the-test"))
            .await;

        assert_eq!(response.status, status);
        assert_eq!(request_id(&response), "sent-by-the-test", "{status}");
    }
}

#[tokio::test]
async fn a_big_body_is_gzipped_when_the_client_accepts_gzip() {
    let plain = client().get("/api/v1/big").await;
    assert!(plain.bytes().len() > 4096, "{}", plain.bytes().len());
    assert_eq!(plain.header("content-encoding"), None);

    let response = client()
        .send(get("/api/v1/big").header("accept-encoding", "gzip"))
        .await;

    assert_eq!(response.status, 200);
    assert_eq!(response.header("content-encoding"), Some("gzip"));
    assert_eq!(response.header("vary"), Some("accept-encoding"));
    assert_eq!(response.bytes()[..2], [0x1f, 0x8b]);
    assert!(response.bytes().len() < plain.bytes().len());
}

#[tokio::test]
async fn a_big_body_is_brotli_when_the_client_accepts_br() {
    let plain = client().get("/api/v1/big").await;

    let response = client()
        .send(get("/api/v1/big").header("accept-encoding", "br"))
        .await;

    assert_eq!(response.status, 200);
    assert_eq!(response.header("content-encoding"), Some("br"));
    assert_ne!(response.bytes()[0], b'{');
    assert!(response.bytes().len() < plain.bytes().len());
}

#[tokio::test]
async fn images_grpc_and_event_streams_are_sent_as_they_are() {
    for (kind, encoding) in [
        ("text", Some("gzip")),
        ("image", None),
        ("grpc", None),
        ("sse", None),
    ] {
        let request = get(&format!("/api/v1/content/200/{kind}")).header("accept-encoding", "gzip");

        let response = client().send(request).await;

        assert_eq!(response.status, 200, "{kind}");
        assert_eq!(response.header("content-encoding"), encoding, "{kind}");
        if encoding.is_none() {
            assert_eq!(response.bytes().len(), CONTENT_SIZE, "{kind}");
        }
    }
}

#[tokio::test]
async fn a_body_under_one_kib_is_sent_as_it_is() {
    let response = client()
        .send(get("/api/v1/small").header("accept-encoding", "gzip, br"))
        .await;

    assert_eq!(response.status, 200);
    assert!(response.bytes().len() < 1024);
    assert_eq!(response.header("content-encoding"), None);
    assert_eq!(response.json()["data"], json!({ "text": "hello" }));
}

#[tokio::test]
async fn a_body_of_one_mib_is_read() {
    let request = TestRequest::new("POST", "/api/v1/length").body(vec![b'a'; MIB]);

    let response = client().send(request).await;

    assert_eq!(response.status, 200);
    assert_eq!(response.json()["data"], json!({ "bytes": MIB }));
}

#[tokio::test]
async fn a_json_body_reaches_the_handler_whole() {
    let request = TestRequest::new("POST", "/api/v1/length").json(&json!({ "key": "value" }));

    let response = client().send(request).await;

    assert_eq!(response.status, 200);
    assert_eq!(response.json()["data"], json!({ "bytes": 15 }));
}

#[tokio::test]
async fn a_body_over_one_mib_is_payload_too_large() {
    let request = TestRequest::new("POST", "/api/v1/length").body(vec![b'a'; MIB + 1]);

    let response = client().send(request).await;

    assert_error(&response, 413, ApiError::PAYLOAD_TOO_LARGE);
}

#[tokio::test]
async fn the_route_that_served_a_request_is_on_its_response() {
    for (request, method, template) in [
        (get("/api/v1/small"), "GET", "/api/v1/small"),
        (get("/api/v1/bare/200"), "GET", "/api/v1/bare/{status}"),
        (
            TestRequest::new("HEAD", "/api/v1/small"),
            "HEAD",
            "/api/v1/small",
        ),
        (
            TestRequest::new("POST", "/api/v1/length").body(vec![b'a'; MIB + 1]),
            "POST",
            "/api/v1/length",
        ),
    ] {
        let response = client().send(request).await;

        let served = response.matched_route().unwrap();
        assert_eq!(served.method.as_str(), method, "{template}");
        assert_eq!(served.template, template);
    }
}

#[tokio::test]
async fn no_route_is_on_the_answer_of_a_fallback() {
    for request in [
        get("/api/v1/nowhere"),
        get("/nowhere"),
        TestRequest::new("DELETE", "/api/v1/small"),
    ] {
        let response = client().send(request).await;

        let served: Option<&MatchedRoute> = response.matched_route();
        assert!(served.is_none(), "{served:?}");
    }
}

#[test]
fn the_document_is_the_base_with_every_route_under_the_prefix() {
    let app = fixture_app();

    let document = serde_json::to_value(app.openapi()).unwrap();

    assert_eq!(document["info"]["title"], "Barras API");
    assert_eq!(document["x-common-error-codes"], json!(COMMON_ERROR_CODES));
    let paths: Vec<&str> = document["paths"]
        .as_object()
        .unwrap()
        .keys()
        .map(String::as_str)
        .collect();
    assert_eq!(
        paths,
        [
            "/api/v1/bare/{status}",
            "/api/v1/big",
            "/api/v1/content/{status}/{kind}",
            "/api/v1/length",
            "/api/v1/members",
            "/api/v1/panic/{payload}",
            "/api/v1/profiles",
            "/api/v1/refused",
            "/api/v1/small",
        ]
    );
}

#[test]
fn the_json_of_the_app_is_its_canonical_document() {
    let app = fixture_app();

    assert_eq!(
        app.openapi_json().unwrap(),
        canonical_json(app.openapi()).unwrap()
    );
}

#[test]
fn the_app_knows_the_service_it_was_built_for() {
    assert_eq!(fixture_app().service(), SERVICE);
}
