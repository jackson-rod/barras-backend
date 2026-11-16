//! The validating extractors (spec §6.1, §6.2): a handler that takes its input with
//! `ValidatedJson` or `ValidatedQuery` sees it only once it was read whole, a body as one JSON
//! object, and passed every rule; anything else answers an error envelope, and no answer
//! repeats a value that was sent.

use std::io;

use axum::body::{Body, Bytes};
use axum::extract::FromRequest;
use axum::http::Request;
use http_body_util::{BodyExt, Full, Limited};
use serde_json::{Value, json};

use super::support::{NewMember, assert_error, client};
use crate::testing::{TestRequest, TestResponse};
use crate::{ApiError, ValidatedJson};

const MIB: usize = 1024 * 1024;

/// What a test sends where nothing must come back: it could be a password.
const SECRET: &str = "hunter2-secret";

/// A member every rule of the fixture accepts.
fn member() -> Value {
    json!({
        "email": "ana@example.com",
        "username": "ana",
        "address": { "street": "Calle 10", "zip": 110_111 },
        "tags": ["saver", "investor"]
    })
}

/// `member()` with the value at the JSON pointer `at` replaced by `value`.
fn member_with(at: &str, value: Value) -> Value {
    let mut body = member();
    *body.pointer_mut(at).unwrap() = value;
    body
}

/// `member()` without the field at the JSON pointer `at`.
fn member_without(at: &str) -> Value {
    let mut body = member();
    let (parent, field) = at.rsplit_once('/').unwrap();
    let object = body.pointer_mut(parent).unwrap().as_object_mut().unwrap();
    object.remove(field).unwrap();
    body
}

/// `body` as JSON to the members route.
fn post(body: &Value) -> TestRequest {
    TestRequest::new("POST", "/api/v1/members").json(body)
}

/// `body` as it is to the members route, sent as `content_type`.
fn post_raw(content_type: &str, body: impl Into<Bytes>) -> TestRequest {
    TestRequest::new("POST", "/api/v1/members")
        .header("content-type", content_type)
        .body(body)
}

/// The members listing, asked for with `query`.
fn get_members(query: &str) -> TestRequest {
    TestRequest::new("GET", &format!("/api/v1/members?{query}"))
}

/// `body` as JSON to the profiles route.
fn post_profile(body: &Value) -> TestRequest {
    TestRequest::new("POST", "/api/v1/profiles").json(body)
}

/// `body`, written by hand as JSON, to the profiles route.
fn post_profile_raw(body: String) -> TestRequest {
    TestRequest::new("POST", "/api/v1/profiles")
        .header("content-type", "application/json")
        .body(body)
}

/// The profiles search, asked for with `query`.
fn get_profiles(query: &str) -> TestRequest {
    TestRequest::new("GET", &format!("/api/v1/profiles?{query}"))
}

/// The keys of `error.fields`, in order, of what must be a validation failure that gives each
/// field at least one message.
fn failed_fields(response: &TestResponse) -> Vec<String> {
    assert_error(response, 422, ApiError::VALIDATION_FAILED);
    let body = response.json();
    let fields = body["error"]["fields"].as_object().unwrap();
    for (field, messages) in fields {
        let count = messages.as_array().map_or(0, Vec::len);
        assert!(count > 0, "{field} has no message: {body}");
    }
    fields.keys().cloned().collect()
}

#[tokio::test]
async fn a_valid_body_reaches_the_handler_whole() {
    let response = client().send(post(&member())).await;

    assert_eq!(response.status, 200);
    assert_eq!(response.json()["data"], member());
}

#[tokio::test]
async fn json_with_parameters_or_a_json_suffix_is_accepted() {
    for content_type in [
        "application/json; charset=utf-8",
        "application/merge-patch+json",
        "Application/JSON",
    ] {
        let response = client()
            .send(post_raw(content_type, member().to_string()))
            .await;

        assert_eq!(response.status, 200, "{content_type}");
    }
}

#[tokio::test]
async fn a_body_not_sent_as_json_is_unsupported_media_type() {
    let body = member().to_string();
    let mut requests = vec![TestRequest::new("POST", "/api/v1/members").body(body.clone())];
    for content_type in [
        "json",
        "text/plain",
        "text/json",
        "application/xml",
        "application/jsonx",
        "application/+json",
        "application/json+xml",
    ] {
        requests.push(post_raw(content_type, body.clone()));
    }
    for request in requests {
        let response = client().send(request).await;

        assert_error(&response, 415, ApiError::UNSUPPORTED_MEDIA_TYPE);
    }
}

#[tokio::test]
async fn the_media_type_is_refused_before_the_body_is_read() {
    let request = post_raw("text/plain", vec![b' '; MIB + 1]);

    let response = client().send(request).await;

    assert_error(&response, 415, ApiError::UNSUPPORTED_MEDIA_TYPE);
}

#[tokio::test]
async fn a_body_that_is_not_json_is_malformed() {
    let member = member().to_string();
    for body in [
        String::new(),
        " \r\n\t".to_owned(),
        r#"{"email": "ana@example.com", "username": "an"#.to_owned(),
        r#"{email: "ana@example.com"}"#.to_owned(),
        format!("{member} trailing"),
        format!("{member}{member}"),
    ] {
        let response = client()
            .send(post_raw("application/json", body.clone()))
            .await;

        assert_error(&response, 400, ApiError::MALFORMED_REQUEST);
    }
}

#[tokio::test]
async fn a_body_that_is_not_json_is_malformed_even_when_a_field_before_the_fault_is_wrong() {
    for body in [
        r#"{"address": {"street": "Calle 10", "zip": "110111"}, "email": "#,
        r#"{"email": "ana@example.com"} trailing"#,
    ] {
        let response = client()
            .send(post_raw("application/json", body.to_owned()))
            .await;

        assert_error(&response, 400, ApiError::MALFORMED_REQUEST);
    }
}

#[tokio::test]
async fn a_body_that_is_not_an_object_is_malformed() {
    // The derived struct would read this array by position, every rule passed.
    let positional = json!([
        "ana@example.com",
        "ana",
        { "street": "Calle 10", "zip": 110_111 },
        ["saver"]
    ]);
    for body in [
        positional.to_string(),
        format!(" \n{positional}"),
        "[]".to_owned(),
        "\"ana@example.com\"".to_owned(),
        "42".to_owned(),
        "true".to_owned(),
        "null".to_owned(),
    ] {
        let response = client()
            .send(post_raw("application/json", body.clone()))
            .await;

        assert_error(&response, 400, ApiError::MALFORMED_REQUEST);
    }
}

#[tokio::test]
async fn white_space_before_the_object_is_allowed() {
    let body = format!(" \r\n\t{}", member());

    let response = client().send(post_raw("application/json", body)).await;

    assert_eq!(response.status, 200);
}

#[tokio::test]
async fn a_key_given_twice_at_the_root_is_malformed() {
    let body = r#"{"email": "ana@example.com", "email": "eva@example.com", "username": "ana",
        "address": {"street": "Calle 10", "zip": 110111}, "tags": []}"#;

    let response = client()
        .send(post_raw("application/json", body.to_owned()))
        .await;

    assert_error(&response, 400, ApiError::MALFORMED_REQUEST);
}

#[tokio::test]
async fn a_wrong_type_is_reported_at_its_path() {
    for (body, field) in [
        (member_with("/address/zip", json!("110111")), "address.zip"),
        (member_with("/address/zip", json!(-1)), "address.zip"),
        (member_with("/email", json!(7)), "email"),
        (member_with("/address", json!("Calle 10")), "address"),
        (member_with("/tags", json!("saver")), "tags"),
    ] {
        let response = client().send(post(&body)).await;

        assert_eq!(failed_fields(&response), [field], "{body}");
    }
}

#[tokio::test]
async fn a_missing_field_is_reported_at_its_own_path_not_its_parents() {
    for (at, field) in [
        ("/email", "email"),
        ("/address/zip", "address.zip"),
        ("/address", "address"),
        ("/tags", "tags"),
    ] {
        let response = client().send(post(&member_without(at))).await;

        assert_eq!(failed_fields(&response), [field], "{at}");
    }
}

#[tokio::test]
async fn null_for_a_required_field_is_reported_at_its_path() {
    for (at, field) in [("/email", "email"), ("/address/zip", "address.zip")] {
        let response = client().send(post(&member_with(at, Value::Null))).await;

        assert_eq!(failed_fields(&response), [field], "{at}");
    }
}

#[tokio::test]
async fn a_wrong_type_inside_an_array_is_reported_at_its_index() {
    let response = client().send(post(&member_with("/tags/1", json!(7)))).await;

    assert_eq!(failed_fields(&response), ["tags[1]"]);
}

#[tokio::test]
async fn every_broken_rule_is_reported_with_all_its_messages() {
    let body = json!({
        "email": "not-an-email",
        "username": "a!",
        "address": { "street": "", "zip": 110_111 },
        "tags": ["saver", ""]
    });

    let response = client().send(post(&body)).await;

    assert_eq!(
        failed_fields(&response),
        ["address.street", "email", "tags[1]", "username"]
    );
    let fields = &response.json()["error"]["fields"];
    let counts: Vec<usize> = ["address.street", "email", "tags[1]", "username"]
        .iter()
        .map(|field| fields[field].as_array().unwrap().len())
        .collect();
    // `a!` is both too short and not alphanumeric.
    assert_eq!(counts, [1, 1, 1, 2]);
}

#[tokio::test]
async fn a_valid_profile_reaches_the_handler_whole() {
    let profile = json!({
        "displayName": "Ana",
        "strictNamed": { "maxLevel": 3 },
        "strict": {},
        "nickname": "ana",
        "pair": [1, 2]
    });

    let response = client().send(post_profile(&profile)).await;

    assert_eq!(response.status, 200);
    assert_eq!(response.json()["data"], profile);
}

#[tokio::test]
async fn a_field_is_reported_under_its_json_name_whichever_check_refused_it() {
    for (body, field) in [
        // garde names a field by its Rust name, serde by its JSON name.
        (json!({ "displayName": "" }), "displayName"),
        (json!({ "displayName": 7 }), "displayName"),
        (
            json!({ "displayName": "Ana", "strictNamed": { "maxLevel": 11 } }),
            "strictNamed.maxLevel",
        ),
        (
            json!({ "displayName": "Ana", "strictNamed": { "maxLevel": "high" } }),
            "strictNamed.maxLevel",
        ),
    ] {
        let response = client().send(post_profile(&body)).await;

        assert_eq!(failed_fields(&response), [field], "{body}");
    }
}

#[tokio::test]
async fn an_unknown_field_is_reported_at_the_object_that_refuses_it() {
    for (body, field) in [
        (
            format!(r#"{{"displayName": "Ana", "strictNamed": {{"{SECRET}": 1}}}}"#),
            "strictNamed",
        ),
        (
            format!(r#"{{"displayName": "Ana", "strict": {{"x, expected {SECRET}": 1}}}}"#),
            "strict",
        ),
    ] {
        let response = client().send(post_profile_raw(body.clone())).await;

        assert_eq!(failed_fields(&response), [field], "{body}");
        let messages = &response.json()["error"]["fields"][field];
        assert_eq!(messages, &json!(["has a field it does not accept"]));
    }
}

#[tokio::test]
async fn an_unknown_field_at_the_root_is_malformed() {
    let body = format!(r#"{{"displayName": "Ana", "{SECRET}": 1}}"#);

    let response = client().send(post_profile_raw(body)).await;

    assert_error(&response, 400, ApiError::MALFORMED_REQUEST);
}

#[tokio::test]
async fn the_error_of_a_type_of_its_own_is_reported_as_not_valid() {
    for request in [
        post_profile(&json!({ "displayName": "Ana", "nickname": SECRET })),
        get_profiles(&format!("nickname={SECRET}")),
    ] {
        let response = client().send(request).await;

        assert_eq!(failed_fields(&response), ["nickname"]);
        let messages = &response.json()["error"]["fields"]["nickname"];
        assert_eq!(messages, &json!(["is not valid"]));
    }
}

#[tokio::test]
async fn an_array_of_the_wrong_length_for_its_tuple_is_reported_at_its_field() {
    // serde_json reads the elements past those a tuple takes as trailing characters, though
    // the body is JSON.
    for pair in [json!([1, 2, 3]), json!([1])] {
        let body = json!({ "displayName": "Ana", "pair": pair });

        let response = client().send(post_profile(&body)).await;

        assert_eq!(failed_fields(&response), ["pair"], "{pair}");
    }
}

#[tokio::test]
async fn no_answer_repeats_a_value_that_was_sent() {
    // serde's own words inside the value, as if to end its quote early.
    let quoting = format!("x\", expected {SECRET}");
    let truncated = format!(r#"{{"email": "{SECRET}""#);
    // No DTO holds a map keyed by what the caller sends, so a body shaped as one names no
    // field that could carry its keys back.
    let mut with_a_map = member();
    with_a_map["map"] = Value::Object([(SECRET.to_owned(), json!("x"))].into_iter().collect());
    for (request, status) in [
        (post(&member_with("/address/zip", json!(SECRET))), 422),
        (post(&member_with("/address/zip", json!(quoting))), 422),
        (post(&member_with("/email", json!(SECRET))), 422),
        (post(&member_with("/username", json!(SECRET))), 422),
        (post(&member_with("/tags/1", json!(SECRET))), 422),
        (post_raw("application/json", truncated), 400),
        (get_members(&format!("limit={SECRET}")), 422),
        (get_members(&format!("limit=5&name={SECRET}")), 422),
        (
            post_profile_raw(format!(
                r#"{{"displayName": "Ana", "strictNamed": {{"{SECRET}": 1}}}}"#
            )),
            422,
        ),
        (
            post_profile_raw(format!(
                r#"{{"displayName": "Ana", "strict": {{"x, expected {SECRET}": 1}}}}"#
            )),
            422,
        ),
        (
            post_profile_raw(format!(r#"{{"displayName": "Ana", "{SECRET}": 1}}"#)),
            400,
        ),
        (
            post_profile(&json!({ "displayName": "Ana", "nickname": SECRET })),
            422,
        ),
        (get_profiles(&format!("nickname={SECRET}")), 422),
        (post(&with_a_map), 200),
        (
            post_profile_raw(format!(
                r#"{{"displayName": "Ana", "map": {{"{SECRET}": "x"}}}}"#
            )),
            400,
        ),
        (
            post_profile_raw(format!(
                r#"{{"displayName": "Ana", "rangedMap": {{"{SECRET}": {{"v": 0}}}}}}"#
            )),
            400,
        ),
    ] {
        let response = client().send(request).await;

        let body = String::from_utf8_lossy(response.bytes()).into_owned();
        assert_eq!(response.status, status, "{body}");
        assert!(!body.contains(SECRET), "{body}");
    }
}

#[tokio::test]
async fn a_body_over_one_mib_is_payload_too_large_whether_its_length_is_known_or_not() {
    let too_large = vec![b' '; MIB + 1];
    let known = post_raw("application/json", too_large.clone())
        .header("content-length", &too_large.len().to_string());
    let streamed = post_raw("application/json", too_large).streamed();
    for request in [known, streamed] {
        let response = client().send(request).await;

        assert_error(&response, 413, ApiError::PAYLOAD_TOO_LARGE);
    }
}

#[tokio::test]
async fn a_body_of_one_mib_is_read_whether_its_length_is_known_or_not() {
    let mut body = member().to_string().into_bytes();
    body.resize(MIB, b' ');
    let known = post_raw("application/json", body.clone());
    let streamed = post_raw("application/json", body).streamed();
    for request in [known, streamed] {
        let response = client().send(request).await;

        assert_eq!(response.status, 200);
        assert_eq!(response.json()["data"], member());
    }
}

#[tokio::test]
async fn a_body_that_breaks_off_while_it_is_read_is_malformed() {
    // No client sends this: the body fails on its first chunk, as when a connection drops.
    let failing = Limited::new(Full::new(Bytes::from_static(b"{}")), 0)
        .map_err(|_| io::Error::other("the connection was lost"));
    let request = Request::post("/api/v1/members")
        .header("content-type", "application/json")
        .body(Body::new(failing))
        .unwrap();

    let refused = ValidatedJson::<NewMember>::from_request(request, &())
        .await
        .unwrap_err();

    assert_eq!(refused.status().as_u16(), 400);
    assert_eq!(refused.code(), ApiError::MALFORMED_REQUEST);
}

#[tokio::test]
async fn a_valid_query_reaches_the_handler() {
    for (query, data) in [
        ("limit=10&name=ana", json!({ "limit": 10, "name": "ana" })),
        ("name=an%61&limit=10", json!({ "limit": 10, "name": "ana" })),
        ("limit=10", json!({ "limit": 10, "name": null })),
    ] {
        let response = client().send(get_members(query)).await;

        assert_eq!(response.status, 200, "{query}");
        assert_eq!(response.json()["data"], data, "{query}");
    }
}

#[tokio::test]
async fn a_parameter_of_the_wrong_type_is_reported_under_its_name() {
    for query in ["limit=abc", "limit=-1", "limit=1.5", "limit=", "limit"] {
        let response = client().send(get_members(query)).await;

        assert_eq!(failed_fields(&response), ["limit"], "{query}");
    }
}

#[tokio::test]
async fn a_missing_parameter_is_reported_under_its_name() {
    for path in ["/api/v1/members", "/api/v1/members?name=ana"] {
        let response = client().get(path).await;

        assert_eq!(failed_fields(&response), ["limit"], "{path}");
    }
}

#[tokio::test]
async fn a_parameter_that_breaks_a_rule_is_reported_under_its_name() {
    for (query, fields) in [
        ("limit=0", vec!["limit"]),
        ("limit=51", vec!["limit"]),
        ("limit=10&name=", vec!["name"]),
        ("limit=0&name=", vec!["limit", "name"]),
    ] {
        let response = client().send(get_members(query)).await;

        assert_eq!(failed_fields(&response), fields, "{query}");
    }
}

#[tokio::test]
async fn a_query_string_that_cannot_be_read_is_malformed() {
    // form-urlencoded reads any text, so what cannot be read is a query serde refuses as a
    // whole, naming no parameter to answer for: one that gives a parameter twice.
    for query in ["limit=5&limit=6", "limit=5&name=ana&limit=5"] {
        let response = client().send(get_members(query)).await;

        assert_error(&response, 400, ApiError::MALFORMED_REQUEST);
    }
}
