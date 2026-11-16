//! The validating extractors (spec §6.1, §6.2): a handler that takes its input with
//! `ValidatedJson` or `ValidatedQuery` sees it only once it was read whole, a body as one JSON
//! object, and passed every rule; anything else answers an error envelope, and no answer
//! repeats a value that was sent.

use std::collections::BTreeMap;
use std::io;

use axum::body::{Body, Bytes};
use axum::extract::FromRequest;
use axum::http::Request;
use garde::Validate;
use http_body_util::{BodyExt, Full, Limited};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use utoipa::ToSchema;
use utoipa_axum::routes;

use super::support::{NewMember, assert_error, client, fixture_metrics};
use crate::extract::{
    HAS_THE_WRONG_TYPE, HAS_UNKNOWN_FIELD, IS_NOT_ACCEPTED, IS_NOT_VALID, IS_REQUIRED,
};
use crate::testing::{TestClient, TestRequest, TestResponse};
use crate::{
    Access, ApiError, ApiResult, ApiRouter, ApiSuccess, ErrorEnvelope, HttpApp, NoData,
    SuccessEnvelope, ValidatedJson, ValidatedQuery,
};

const KIB: usize = 1024;
const MIB: usize = 1024 * KIB;

/// What a test sends where nothing must come back: it could be a password or a token. It is
/// spelled as an identifier, as many tokens are, so that the shape of a name cannot be what
/// keeps it out of an answer.
const SECRET: &str = "hunter2_Secret_Value";

/// A query string read whole into a map, through a conversion whose error carelessly names, as
/// a missing field, the first value the caller sent.
#[derive(Debug, Deserialize, Validate)]
#[serde(try_from = "BTreeMap<String, String>")]
struct ValueEcho {}

impl TryFrom<BTreeMap<String, String>> for ValueEcho {
    type Error = String;

    fn try_from(pairs: BTreeMap<String, String>) -> Result<Self, String> {
        let first = pairs.into_values().next().unwrap_or_default();
        Err(format!("missing field `{first}`"))
    }
}

/// As `ValueEcho`, naming the first parameter's name instead of its value.
#[derive(Debug, Deserialize, Serialize, Validate)]
#[serde(try_from = "BTreeMap<String, String>")]
struct NameEcho {}

impl TryFrom<BTreeMap<String, String>> for NameEcho {
    type Error = String;

    fn try_from(pairs: BTreeMap<String, String>) -> Result<Self, String> {
        let first = pairs.into_keys().next().unwrap_or_default();
        Err(format!("missing field `{first}`"))
    }
}

/// Read the query string, through a careless conversion that quotes a value.
#[utoipa::path(
    get,
    path = "/echoes/values",
    tag = "fixtures",
    params(("a" = Option<String>, Query, description = "Anything")),
    responses(
        (status = 200, description = "Never sent", body = SuccessEnvelope<NoData>),
        (status = 422, description = "A parameter failed validation", body = ErrorEnvelope),
    ),
)]
async fn echo_values(ValidatedQuery(_pairs): ValidatedQuery<ValueEcho>) -> ApiResult<NoData> {
    Ok(ApiSuccess::ok("Read", NoData))
}

/// Read the query string, through a careless conversion that quotes a name.
#[utoipa::path(
    get,
    path = "/echoes/names",
    tag = "fixtures",
    params(("a" = Option<String>, Query, description = "Anything")),
    responses(
        (status = 200, description = "Never sent", body = SuccessEnvelope<NoData>),
        (status = 422, description = "A parameter failed validation", body = ErrorEnvelope),
    ),
)]
async fn echo_names(ValidatedQuery(_pairs): ValidatedQuery<NameEcho>) -> ApiResult<NoData> {
    Ok(ApiSuccess::ok("Read", NoData))
}

/// A body with a member read through the careless conversion that quotes a name.
#[derive(Debug, Deserialize, Serialize, Validate, ToSchema)]
#[serde(rename_all = "camelCase")]
struct Names {
    #[garde(skip)]
    #[schema(value_type = Option<Object>)]
    names: Option<NameEcho>,
}

/// Read the body, through a careless conversion that quotes a name.
#[utoipa::path(
    post,
    path = "/echoes/names",
    tag = "fixtures",
    request_body = Names,
    responses(
        (status = 200, description = "Never sent", body = SuccessEnvelope<Names>),
        (status = 422, description = "A field failed validation", body = ErrorEnvelope),
    ),
)]
async fn echo_names_in_a_body(ValidatedJson(names): ValidatedJson<Names>) -> ApiResult<Names> {
    Ok(ApiSuccess::ok("Read", names))
}

/// A client of an app with the echo routes.
fn echo_client() -> TestClient {
    let codes = &[ApiError::VALIDATION_FAILED];
    let routes = ApiRouter::new()
        .route(routes!(echo_values), Access::public(), codes)
        .route(
            routes!(echo_names, echo_names_in_a_body),
            Access::public(),
            codes,
        );
    let app = HttpApp::builder(fixture_metrics())
        .routes(routes)
        .build()
        .unwrap();
    TestClient::new(&app)
}

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
    let nested = format!("{}{}", "[".repeat(200), "]".repeat(200));
    let deep = format!(r#"{{"extra": {nested}}}"#);
    let deep_beside_a_wrong_field = format!(r#"{{"email": 7, "extra": {nested}}}"#);
    for body in [
        Vec::new(),
        b" \r\n\t".to_vec(),
        br#"{"email": "ana@example.com", "username": "an"#.to_vec(),
        br#"{email: "ana@example.com"}"#.to_vec(),
        format!("{member} trailing").into_bytes(),
        format!("{member}{member}").into_bytes(),
        // Even in a field the DTO does not have: ill-formed UTF-8 in a value or a key, and,
        // where a field is refused too, nesting past serde_json's limit.
        b"{\"text\": \"a\xffb\"}".to_vec(),
        b"{\"te\xffxt\": 1}".to_vec(),
        deep.into_bytes(),
        deep_beside_a_wrong_field.into_bytes(),
    ] {
        let shown = String::from_utf8_lossy(&body).into_owned();

        let response = client().send(post_raw("application/json", body)).await;

        assert_eq!(response.status, 400, "{shown}");
        assert_error(&response, 400, ApiError::MALFORMED_REQUEST);
    }
}

#[tokio::test]
async fn ill_formed_utf8_is_malformed_even_in_a_member_nobody_reads() {
    // RFC 8259 §8.1: JSON text is UTF-8. The typed read skips a member the DTO does not have
    // without looking at its bytes.
    let member = member().to_string();
    let open = member.strip_suffix('}').unwrap();
    for extra in [&b"\"x\": \"a\xffb\""[..], b"\"x\": {\"k\xff\": 1}"] {
        let mut body = format!("{open}, ").into_bytes();
        body.extend_from_slice(extra);
        body.push(b'}');

        let response = client().send(post_raw("application/json", body)).await;

        assert_error(&response, 400, ApiError::MALFORMED_REQUEST);
    }
}

#[tokio::test]
async fn what_the_grammar_allows_in_a_member_nobody_reads_is_accepted() {
    let member = member().to_string();
    let open = member.strip_suffix('}').unwrap();
    let deep = format!("{}{}", "[".repeat(200), "]".repeat(200));
    for extra in [r#""\ud800""#, "1e400", &deep] {
        let body = format!(r#"{open}, "x": {extra}}}"#);

        let response = client().send(post_raw("application/json", body)).await;

        assert_eq!(response.status, 200, "{extra}");
    }
}

#[tokio::test]
async fn a_value_the_grammar_allows_and_a_field_cannot_hold_is_reported_at_that_field() {
    let member = member().to_string();
    for (from, to, field) in [
        (
            r#""email":"ana@example.com""#,
            r#""email":"\ud800""#,
            "email",
        ),
        (r#""zip":110111"#, r#""zip":1e400"#, "address.zip"),
    ] {
        assert!(member.contains(from), "{member}");
        let body = member.replace(from, to);

        let response = client().send(post_raw("application/json", body)).await;

        assert_eq!(failed_fields(&response), [field], "{to}");
        let messages = &response.json()["error"]["fields"][field];
        assert_eq!(messages, &json!([IS_NOT_VALID]));
    }
}

#[tokio::test]
async fn a_missing_field_is_not_believed_beside_what_the_grammar_allows_and_no_tree_holds() {
    let without_email = member_without("/email").to_string();
    let open = without_email.strip_suffix('}').unwrap();
    for extra in [r#""\ud800""#, "1e400"] {
        let body = format!(r#"{open}, "x": {extra}}}"#);

        let response = client().send(post_raw("application/json", body)).await;

        // At the root, a field that cannot be named leaves nothing to answer for.
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
        assert_eq!(
            response.json()["error"]["fields"][field],
            json!([IS_REQUIRED])
        );
    }
}

/// `body` with a member nobody reads that brings its JSON to exactly `size` bytes.
fn padded_to(size: usize, mut body: Value) -> String {
    body["padding"] = json!("");
    let unpadded = body.to_string().len();
    body["padding"] = json!("a".repeat(size - unpadded));
    let padded = body.to_string();
    assert_eq!(padded.len(), size);
    padded
}

#[tokio::test]
async fn a_missing_field_is_confirmed_only_in_a_body_of_at_most_64_kib() {
    let at_most = 64 * KIB;
    for (at, size, field, message) in [
        ("/email", at_most, "email", IS_REQUIRED),
        ("/address/zip", at_most, "address.zip", IS_REQUIRED),
        // Unconfirmed, the field is no key: the object that lacks it is not valid.
        ("/address/zip", at_most + 1, "address", IS_NOT_VALID),
    ] {
        let body = padded_to(size, member_without(at));

        let response = client().send(post_raw("application/json", body)).await;

        assert_eq!(failed_fields(&response), [field], "{at} in {size} bytes");
        let messages = &response.json()["error"]["fields"][field];
        assert_eq!(messages, &json!([message]), "{at} in {size} bytes");
    }
    // Unconfirmed at the root, no field is left to answer for.
    let body = padded_to(at_most + 1, member_without("/email"));

    let response = client().send(post_raw("application/json", body)).await;

    assert_error(&response, 400, ApiError::MALFORMED_REQUEST);
}

#[tokio::test]
async fn a_field_missing_from_an_element_of_an_array_is_reported_at_its_path() {
    let body = json!({
        "displayName": "Ana",
        "addresses": [{ "street": "Calle 10", "zip": 110_111 }, { "street": "Calle 11" }]
    });

    let response = client().send(post_profile(&body)).await;

    assert_eq!(failed_fields(&response), ["addresses[1].zip"]);
    let messages = &response.json()["error"]["fields"]["addresses[1].zip"];
    assert_eq!(messages, &json!([IS_REQUIRED]));
}

#[tokio::test]
async fn a_field_missing_from_a_variant_is_reported_under_the_variant() {
    let body = json!({ "displayName": "Ana", "plan": { "monthly": {} } });

    let response = client().send(post_profile(&body)).await;

    assert_eq!(failed_fields(&response), ["plan.monthly.amount"]);
    let messages = &response.json()["error"]["fields"]["plan.monthly.amount"];
    assert_eq!(messages, &json!([IS_REQUIRED]));
}

#[tokio::test]
async fn a_missing_field_named_after_a_value_that_was_sent_is_not_believed() {
    for value in ["hunter2", "Password123", SECRET] {
        let body = json!({ "displayName": "Ana", "lookup": { "a": value } });
        let in_a_body = client().send(post_profile(&body)).await;
        let path = format!("/api/v1/echoes/values?a={value}");
        let in_a_query = echo_client().send(TestRequest::new("GET", &path)).await;

        assert_eq!(failed_fields(&in_a_body), ["lookup"], "{value}");
        let messages = &in_a_body.json()["error"]["fields"]["lookup"];
        assert_eq!(messages, &json!([IS_NOT_VALID]));
        // At the root of a query string, no field is left to answer for.
        assert_error(&in_a_query, 400, ApiError::MALFORMED_REQUEST);
        for response in [&in_a_body, &in_a_query] {
            let sent = String::from_utf8_lossy(response.bytes()).into_owned();
            assert!(!sent.contains(value), "{sent}");
        }
    }
}

#[tokio::test]
async fn a_missing_parameter_named_after_one_that_was_sent_is_not_believed() {
    let response = echo_client()
        .send(TestRequest::new("GET", "/api/v1/echoes/names?limit=5"))
        .await;

    assert_error(&response, 400, ApiError::MALFORMED_REQUEST);
}

#[tokio::test]
async fn a_wrong_type_names_what_was_expected_only_when_it_is_a_primitive() {
    for (body, field, message) in [
        (
            member_with("/address/zip", json!("110111")),
            "address.zip",
            "expected u32",
        ),
        (
            member_with("/address/zip", json!(-1)),
            "address.zip",
            "expected u32",
        ),
        (
            member_with("/email", json!(7)),
            "email",
            "expected a string",
        ),
        (
            member_with("/tags", json!("saver")),
            "tags",
            "expected a sequence",
        ),
        // serde would say `expected struct Address`: a name of the code, not of the contract.
        (
            member_with("/address", json!("Calle 10")),
            "address",
            HAS_THE_WRONG_TYPE,
        ),
    ] {
        let response = client().send(post(&body)).await;

        assert_eq!(failed_fields(&response), [field], "{body}");
        let messages = &response.json()["error"]["fields"][field];
        assert_eq!(messages, &json!([message]), "{body}");
    }
}

#[tokio::test]
async fn an_unknown_variant_is_not_one_of_the_accepted_values() {
    let body = json!({ "displayName": "Ana", "kind": "trader" });

    let response = client().send(post_profile(&body)).await;

    assert_eq!(failed_fields(&response), ["kind"]);
    let messages = &response.json()["error"]["fields"]["kind"];
    assert_eq!(messages, &json!([IS_NOT_ACCEPTED]));
}

#[tokio::test]
async fn an_error_of_a_type_of_its_own_that_reads_as_a_missing_field_is_not_believed() {
    for request in [
        post_profile(&json!({ "displayName": "Ana", "echo": "missing field `nickname`" })),
        get_profiles("echo=missing%20field%20%60nickname%60"),
    ] {
        let response = client().send(request).await;

        assert_eq!(failed_fields(&response), ["echo"]);
        let messages = &response.json()["error"]["fields"]["echo"];
        assert_eq!(messages, &json!([IS_NOT_VALID]));
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
        "pair": [1, 2],
        "kind": "saver",
        "target": { "type": "saving", "amount": 5 }
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
        assert_eq!(messages, &json!([HAS_UNKNOWN_FIELD]));
    }
}

#[tokio::test]
async fn an_unknown_field_inside_an_internally_tagged_enum_is_reported_one_level_up() {
    // serde reads the enum's object whole before it knows the variant, so the path stops at
    // the enum; the object that holds it here is the body itself.
    let body = format!(
        r#"{{"displayName": "Ana", "target": {{"type": "saving", "amount": 5, "{SECRET}": 1}}}}"#
    );

    let response = client().send(post_profile_raw(body)).await;

    assert_error(&response, 400, ApiError::MALFORMED_REQUEST);
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
        assert_eq!(messages, &json!([IS_NOT_VALID]));
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
    // A type of the DTO's own whose error starts with the value, and one whose error is the
    // value: the caller then writes the words the extractor reads.
    let serde_words = format!("invalid type: x, expected {SECRET}");
    let missing_words = format!("missing field `{SECRET}`");
    let target = format!(r#"{{"type": "saving", "amount": 5, "{SECRET}": 1}}"#);
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
        (
            post_profile(&json!({ "displayName": "Ana", "code": serde_words })),
            422,
        ),
        (
            post_profile(&json!({ "displayName": "Ana", "echo": missing_words })),
            422,
        ),
        (
            get_profiles(&format!(
                "code=invalid%20type%3A%20x%2C%20expected%20{SECRET}"
            )),
            422,
        ),
        (
            get_profiles(&format!("echo=missing%20field%20%60{SECRET}%60")),
            422,
        ),
        (
            post_profile(&json!({ "displayName": "Ana", "kind": SECRET })),
            422,
        ),
        // A type of the DTO's own that names, as a missing field, a value that was sent.
        (
            post_profile(&json!({ "displayName": "Ana", "lookup": { "a": SECRET } })),
            422,
        ),
        (
            post_profile_raw(format!(r#"{{"displayName": "Ana", "target": {target}}}"#)),
            400,
        ),
    ] {
        let response = client().send(request).await;

        let body = String::from_utf8_lossy(response.bytes()).into_owned();
        assert_eq!(response.status, status, "{body}");
        assert!(!body.contains(SECRET), "{body}");
    }
    // The same, through a type that quotes what it read, value or name, of a query string or
    // of a body.
    let names = json!({ "names": { SECRET: "x" } });
    for (request, status) in [
        (
            TestRequest::new("GET", &format!("/api/v1/echoes/values?a={SECRET}")),
            400,
        ),
        (
            TestRequest::new("GET", &format!("/api/v1/echoes/names?{SECRET}=1")),
            400,
        ),
        (
            TestRequest::new("POST", "/api/v1/echoes/names").json(&names),
            422,
        ),
    ] {
        let response = echo_client().send(request).await;

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
