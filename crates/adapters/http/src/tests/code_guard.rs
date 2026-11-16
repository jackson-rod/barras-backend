//! The guard on the error answers of a documented route: its `error.code` is one that its
//! operation lists in `x-error-codes` or the document lists in `x-common-error-codes`, and its
//! status is one that its operation documents or that any route can answer; otherwise the test
//! that received it fails, naming the route, the code and the status.

use std::error::Error;
use std::fmt;

use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use barras_kernel::{AppError, ErrorKind};
use utoipa_axum::router::UtoipaMethodRouter;
use utoipa_axum::routes;

use super::support::{assert_error, fixture_metrics};
use crate::testing::{TestClient, TestRequest};
use crate::{
    Access, ApiError, ApiResult, ApiRouter, ErrorEnvelope, HttpApp, NoData, SuccessEnvelope,
};

/// The code of [`Taken`].
const FIXTURE_TAKEN: &str = "FIXTURE_TAKEN";

/// A failure of a module the adapter knows nothing about: someone else holds the fixture.
#[derive(Debug)]
struct Taken;

impl fmt::Display for Taken {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("the fixture is taken")
    }
}

impl Error for Taken {}

impl AppError for Taken {
    fn code(&self) -> &'static str {
        FIXTURE_TAKEN
    }

    fn kind(&self) -> ErrorKind {
        ErrorKind::Conflict
    }
}

/// Take the fixture.
#[utoipa::path(
    post,
    path = "/taken",
    tag = "fixtures",
    responses(
        (status = 200, description = "Never sent", body = SuccessEnvelope<NoData>),
        (status = 409, description = "Always", body = ErrorEnvelope),
    ),
)]
async fn take() -> ApiResult<NoData> {
    Err(Taken.into())
}

/// Take the fixture, documenting no 409.
#[utoipa::path(
    post,
    path = "/quietly-taken",
    tag = "fixtures",
    responses((status = 200, description = "Never sent", body = SuccessEnvelope<NoData>)),
)]
async fn take_quietly() -> ApiResult<NoData> {
    Err(Taken.into())
}

/// Take the fixture, documenting every status but 200 at once.
#[utoipa::path(
    post,
    path = "/taken-by-default",
    tag = "fixtures",
    responses(
        (status = 200, description = "Never sent", body = SuccessEnvelope<NoData>),
        (status = "default", description = "Any failure", body = ErrorEnvelope),
    ),
)]
async fn take_by_default() -> ApiResult<NoData> {
    Err(Taken.into())
}

/// Take the fixture, documenting every client error at once.
#[utoipa::path(
    post,
    path = "/taken-in-range",
    tag = "fixtures",
    responses(
        (status = 200, description = "Never sent", body = SuccessEnvelope<NoData>),
        (status = "4XX", description = "Any client error", body = ErrorEnvelope),
    ),
)]
async fn take_in_range() -> ApiResult<NoData> {
    Err(Taken.into())
}

/// Refuse a body that cannot be read, as an extractor does, documenting only the success.
#[utoipa::path(
    post,
    path = "/unread",
    tag = "fixtures",
    responses((status = 200, description = "Never sent", body = SuccessEnvelope<NoData>)),
)]
async fn refuse_unread() -> ApiResult<NoData> {
    Err(ApiError::malformed_request(
        "Request body could not be read",
    ))
}

/// Answer 418 in plain text, as a library layer might, documenting only the success: the
/// safety net makes it a `MALFORMED_REQUEST` envelope at 418.
#[utoipa::path(
    get,
    path = "/teapot",
    tag = "fixtures",
    responses((status = 200, description = "Never sent", body = SuccessEnvelope<NoData>)),
)]
async fn teapot() -> Response {
    (StatusCode::IM_A_TEAPOT, "short and stout").into_response()
}

/// A client of an app with `route` alone, registered with `codes` as its own error codes.
fn client_of(route: UtoipaMethodRouter, codes: &'static [&'static str]) -> TestClient {
    let routes = ApiRouter::new().route(route, Access::public(), codes);
    let app = HttpApp::builder(fixture_metrics())
        .routes(routes)
        .build()
        .unwrap();
    TestClient::new(&app)
}

fn post(path: &str) -> TestRequest {
    TestRequest::new("POST", path)
}

#[tokio::test]
async fn a_code_the_route_lists_at_a_status_its_operation_documents_passes() {
    let client = client_of(routes!(take), &[FIXTURE_TAKEN]);

    let response = client.send(post("/api/v1/taken")).await;

    assert_error(&response, 409, FIXTURE_TAKEN);
}

#[tokio::test]
#[should_panic(
    expected = "POST /api/v1/taken answered 409 FIXTURE_TAKEN, a code that neither its \
                x-error-codes nor x-common-error-codes lists"
)]
async fn a_code_the_route_does_not_list_fails_the_test() {
    let client = client_of(routes!(take), &[]);

    client.send(post("/api/v1/taken")).await;
}

#[tokio::test]
#[should_panic(
    expected = "POST /api/v1/quietly-taken answered 409 FIXTURE_TAKEN, a status that its \
                operation does not document"
)]
async fn a_status_the_operation_does_not_document_fails_the_test() {
    let client = client_of(routes!(take_quietly), &[FIXTURE_TAKEN]);

    client.send(post("/api/v1/quietly-taken")).await;
}

#[tokio::test]
async fn a_common_code_at_a_status_any_route_can_answer_needs_nothing_documented() {
    let client = client_of(routes!(refuse_unread), &[]);

    let response = client.send(post("/api/v1/unread")).await;

    assert_error(&response, 400, ApiError::MALFORMED_REQUEST);
}

#[tokio::test]
#[should_panic(
    expected = "GET /api/v1/teapot answered 418 MALFORMED_REQUEST, a status that its operation \
                does not document"
)]
async fn a_common_code_at_any_other_status_needs_that_status_documented() {
    let client = client_of(routes!(teapot), &[]);

    client.get("/api/v1/teapot").await;
}

#[tokio::test]
async fn default_and_a_range_document_every_status_they_cover() {
    for (route, path) in [
        (routes!(take_by_default), "/api/v1/taken-by-default"),
        (routes!(take_in_range), "/api/v1/taken-in-range"),
    ] {
        let client = client_of(route, &[FIXTURE_TAKEN]);

        let response = client.send(post(path)).await;

        assert_error(&response, 409, FIXTURE_TAKEN);
    }
}
