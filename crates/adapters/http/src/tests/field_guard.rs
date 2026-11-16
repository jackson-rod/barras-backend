//! The guard on the keys of a validation failure: on a 422 `VALIDATION_FAILED` to a JSON body,
//! every key of `error.fields` names a member of the body that was sent or, for a field
//! reported missing, its parent; otherwise the test that sent it fails, naming the key.

use utoipa_axum::routes;

use super::support::{SERVICE, client, fixture_metrics};
use crate::testing::{TestClient, TestRequest};
use crate::{
    Access, ApiError, ApiResult, ApiRouter, ErrorEnvelope, FieldErrors, HttpApp, NoData,
    SuccessEnvelope,
};

/// Refuse whatever was sent, keyed as a careless check would: by a Rust name.
#[utoipa::path(
    post,
    path = "/misnamed",
    tag = "fixtures",
    responses(
        (status = 200, description = "Never sent", body = SuccessEnvelope<NoData>),
        (status = 422, description = "Always", body = ErrorEnvelope),
    ),
)]
async fn misnamed() -> ApiResult<NoData> {
    let fields = FieldErrors::from([("display_name".to_owned(), vec!["is too long".to_owned()])]);
    Err(ApiError::validation(fields))
}

/// Refuse whatever was sent with three keys: a nested member, an element of an array, and a
/// field reported missing from the root.
#[utoipa::path(
    post,
    path = "/refusing",
    tag = "fixtures",
    responses(
        (status = 200, description = "Never sent", body = SuccessEnvelope<NoData>),
        (status = 422, description = "Always", body = ErrorEnvelope),
    ),
)]
async fn refusing() -> ApiResult<NoData> {
    let fields = FieldErrors::from([
        ("address.zip".to_owned(), vec!["is not valid".to_owned()]),
        ("tags[1]".to_owned(), vec!["is too long".to_owned()]),
        ("email".to_owned(), vec!["is required".to_owned()]),
    ]);
    Err(ApiError::validation(fields))
}

/// An app with the two refusing routes, documented as every route is.
fn refusing_client() -> TestClient {
    let routes = ApiRouter::new()
        .route(
            routes!(misnamed),
            Access::public(),
            &[ApiError::VALIDATION_FAILED],
        )
        .route(
            routes!(refusing),
            Access::public(),
            &[ApiError::VALIDATION_FAILED],
        );
    let app = HttpApp::builder(SERVICE, fixture_metrics())
        .routes(routes)
        .build()
        .unwrap();
    TestClient::new(&app)
}

#[tokio::test]
#[should_panic(expected = "display_name")]
async fn a_key_that_names_no_member_of_the_body_fails_the_test() {
    let request = TestRequest::new("POST", "/api/v1/misnamed")
        .json(&serde_json::json!({ "displayName": "Ana" }));

    refusing_client().send(request).await;
}

#[tokio::test]
async fn keys_that_name_members_or_the_parent_of_a_missing_field_pass() {
    let body = serde_json::json!({ "address": { "zip": 1 }, "tags": ["saver", "investor"] });

    let response = refusing_client()
        .send(TestRequest::new("POST", "/api/v1/refusing").json(&body))
        .await;

    assert_eq!(response.status, 422);
}

#[tokio::test]
#[should_panic(expected = "tags[1]")]
async fn an_index_past_the_end_of_the_array_sent_fails_the_test() {
    let body = serde_json::json!({ "address": { "zip": 1 }, "tags": ["saver"] });

    refusing_client()
        .send(TestRequest::new("POST", "/api/v1/refusing").json(&body))
        .await;
}

#[tokio::test]
#[should_panic(expected = "address.zip")]
async fn a_key_reported_wrong_must_name_a_member_even_when_its_parent_was_sent() {
    let body = serde_json::json!({ "address": {}, "tags": ["saver", "investor"] });

    refusing_client()
        .send(TestRequest::new("POST", "/api/v1/refusing").json(&body))
        .await;
}

#[tokio::test]
async fn a_request_that_sent_no_json_is_not_held_to_a_body() {
    let response = refusing_client()
        .send(TestRequest::new("POST", "/api/v1/misnamed").body("display_name"))
        .await;

    assert_eq!(response.status, 422);
}

#[tokio::test]
async fn the_rules_of_the_fixture_app_key_each_failure_by_a_member_of_the_body() {
    let body = serde_json::json!({ "displayName": "", "strictNamed": { "maxLevel": 11 } });

    let response = client()
        .send(TestRequest::new("POST", "/api/v1/profiles").json(&body))
        .await;

    assert_eq!(response.status, 422);
    let fields = response.json()["error"]["fields"].clone();
    let keys: Vec<&String> = fields.as_object().unwrap().keys().collect();
    assert_eq!(keys, ["displayName", "strictNamed.maxLevel"]);
}
