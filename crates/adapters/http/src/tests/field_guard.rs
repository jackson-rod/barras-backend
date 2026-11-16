//! The guard on the keys of a validation failure: on a 422 `VALIDATION_FAILED` to a JSON body,
//! every key of `error.fields` names a member of the body that was sent, a parameter of the
//! query string, or a field missing from an object of the body; otherwise the test that sent
//! it fails, naming the key.

use garde::Validate;
use serde::Deserialize;
use utoipa::ToSchema;
use utoipa_axum::routes;

use super::support::{SERVICE, client, fixture_metrics};
use crate::testing::{TestClient, TestRequest};
use crate::{
    Access, ApiError, ApiResult, ApiRouter, ApiSuccess, ErrorEnvelope, FieldErrors, HttpApp,
    NoData, SuccessEnvelope, ValidatedJson, ValidatedQuery,
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

/// Refuse whatever was sent with three keys: a field reported missing from a nested object, an
/// element of an array, and a field reported missing from the root.
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
        ("address.zip".to_owned(), vec!["is required".to_owned()]),
        ("tags[1]".to_owned(), vec!["is too long".to_owned()]),
        ("email".to_owned(), vec!["is required".to_owned()]),
    ]);
    Err(ApiError::validation(fields))
}

/// How many notes to keep, read from the query string.
#[derive(Debug, Deserialize, Validate)]
#[serde(rename_all = "camelCase")]
struct NotesKept {
    #[garde(range(min = 1, max = 50))]
    limit: u32,
}

/// A note to save.
#[derive(Debug, Deserialize, Validate, ToSchema)]
#[serde(rename_all = "camelCase")]
struct NewNote {
    #[garde(length(min = 1))]
    text: String,
}

/// Save a note, read from the body and the query string.
#[utoipa::path(
    post,
    path = "/notes",
    tag = "fixtures",
    request_body = NewNote,
    params(("limit" = u32, Query, description = "How many notes to keep")),
    responses(
        (status = 200, description = "Saved", body = SuccessEnvelope<NoData>),
        (status = 422, description = "A field failed validation", body = ErrorEnvelope),
    ),
)]
async fn save_note(
    ValidatedQuery(_kept): ValidatedQuery<NotesKept>,
    ValidatedJson(_note): ValidatedJson<NewNote>,
) -> ApiResult<NoData> {
    Ok(ApiSuccess::ok("Note saved", NoData))
}

/// A contact whose phone a rule requires, though its type lets it be absent.
#[derive(Debug, Deserialize, Validate, ToSchema)]
#[serde(rename_all = "camelCase")]
struct NewContact {
    #[garde(required)]
    phone: Option<String>,
}

/// Save a contact.
#[utoipa::path(
    post,
    path = "/contacts",
    tag = "fixtures",
    request_body = NewContact,
    responses(
        (status = 200, description = "Saved", body = SuccessEnvelope<NoData>),
        (status = 422, description = "A field failed validation", body = ErrorEnvelope),
    ),
)]
async fn save_contact(ValidatedJson(_contact): ValidatedJson<NewContact>) -> ApiResult<NoData> {
    Ok(ApiSuccess::ok("Contact saved", NoData))
}

/// An app with the routes of this file, documented as every route is.
fn refusing_client() -> TestClient {
    let codes = &[ApiError::VALIDATION_FAILED];
    let routes = ApiRouter::new()
        .route(routes!(misnamed), Access::public(), codes)
        .route(routes!(refusing), Access::public(), codes)
        .route(routes!(save_note), Access::public(), codes)
        .route(routes!(save_contact), Access::public(), codes);
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
async fn a_missing_field_whose_parent_was_not_sent_fails_the_test() {
    let body = serde_json::json!({ "tags": ["saver", "investor"] });

    refusing_client()
        .send(TestRequest::new("POST", "/api/v1/refusing").json(&body))
        .await;
}

#[tokio::test]
async fn a_key_that_names_a_parameter_of_the_query_passes() {
    let body = serde_json::json!({ "text": "a" });
    let request = TestRequest::new("POST", "/api/v1/notes?limit=abc").json(&body);

    let response = refusing_client().send(request).await;

    assert_eq!(response.status, 422);
    let fields = response.json()["error"]["fields"].clone();
    assert_eq!(
        fields.as_object().unwrap().keys().collect::<Vec<_>>(),
        ["limit"]
    );
}

#[tokio::test]
async fn a_field_a_rule_reports_though_it_was_not_sent_passes() {
    let request = TestRequest::new("POST", "/api/v1/contacts").json(&serde_json::json!({}));

    let response = refusing_client().send(request).await;

    assert_eq!(response.status, 422);
    assert_eq!(
        response.json()["error"]["fields"],
        serde_json::json!({ "phone": ["not set"] })
    );
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
