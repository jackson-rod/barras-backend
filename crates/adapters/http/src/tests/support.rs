//! The fixture app the tests run against, its routes documented and registered with
//! `ApiRouter` as the real app's are, and a writer that keeps what the app logs.

use std::io;
use std::sync::{Arc, Mutex};

use axum::body::Bytes;
use axum::extract::Path;
use axum::http::header::CONTENT_LENGTH;
use axum::http::{HeaderName, HeaderValue, StatusCode};
use axum::response::{IntoResponse, Response};
use garde::Validate;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use tracing_subscriber::fmt::MakeWriter;
use tracing_subscriber::util::SubscriberInitExt;
use utoipa::ToSchema;
use utoipa_axum::routes;

use crate::testing::{TestClient, TestRequest, TestResponse};
use crate::{
    Access, ApiError, ApiResult, ApiRouter, ApiSuccess, ErrorEnvelope, FieldErrors, HttpApp,
    NoData, ServiceInfo, SuccessEnvelope, ValidatedJson, ValidatedQuery,
};

pub(super) const SERVICE: ServiceInfo = ServiceInfo {
    name: "barras-fixture",
    version: "1.2.3",
};

/// What the panic route panics with: it must reach the log and never a body.
pub(super) const PANIC_TEXT: &str = "panic-secret from the fixture";

/// What the bare route answers, as a library layer would: plain text, no envelope.
pub(super) const BARE_TEXT: &str = "bare-secret from a library layer";

/// A header the bare route sets, which the safety net keeps.
pub(super) const KEPT_HEADER: &str = "x-fixture";

/// What the small route answers: far under the size from which a body is compressed.
#[derive(Serialize, ToSchema)]
struct Greeting {
    text: &'static str,
}

/// What the big route answers: over 4 KiB of JSON.
#[derive(Serialize, ToSchema)]
struct Catalog {
    items: Vec<String>,
}

/// What the length route answers.
#[derive(Serialize, ToSchema)]
struct Length {
    bytes: usize,
}

/// Read the greeting.
#[utoipa::path(
    get,
    path = "/small",
    tag = "fixtures",
    responses((status = 200, description = "The greeting", body = SuccessEnvelope<Greeting>)),
)]
async fn small() -> ApiResult<Greeting> {
    Ok(ApiSuccess::ok("Greeting read", Greeting { text: "hello" }))
}

/// Read the catalog.
#[utoipa::path(
    get,
    path = "/big",
    tag = "fixtures",
    responses((status = 200, description = "The catalog", body = SuccessEnvelope<Catalog>)),
)]
async fn big() -> ApiResult<Catalog> {
    let items = (0..200)
        .map(|number| format!("catalog item number {number}"))
        .collect();
    Ok(ApiSuccess::ok("Catalog read", Catalog { items }))
}

/// Give up, with a payload of the kind asked for: `formatted` text (a `String`), `literal`
/// text (a `&'static str`) or a `number`.
#[utoipa::path(
    get,
    path = "/panic/{payload}",
    tag = "fixtures",
    params(("payload" = String, Path, description = "The kind of payload")),
    responses((status = 200, description = "Never sent", body = SuccessEnvelope<NoData>)),
)]
async fn explode(Path(payload): Path<String>) -> ApiResult<NoData> {
    match payload.as_str() {
        "literal" => panic!("literal-secret from the fixture"),
        "number" => std::panic::panic_any(7_u8),
        _ => panic!("{PANIC_TEXT}"),
    }
}

/// Count the bytes of the body.
#[utoipa::path(
    post,
    path = "/length",
    tag = "fixtures",
    responses(
        (status = 200, description = "The length", body = SuccessEnvelope<Length>),
        (status = 413, description = "Over the limit", body = ErrorEnvelope),
    ),
)]
async fn length(body: Bytes) -> ApiResult<Length> {
    let length = Length { bytes: body.len() };
    Ok(ApiSuccess::ok("Body measured", length))
}

/// Answer the status asked for as a library layer does: plain text, with a length.
#[utoipa::path(
    get,
    path = "/bare/{status}",
    tag = "fixtures",
    params(("status" = u16, Path, description = "The status to answer")),
    responses((status = 200, description = "Plain text", body = SuccessEnvelope<NoData>)),
)]
async fn bare(Path(status): Path<u16>) -> Response {
    let headers = [
        (
            HeaderName::from_static(KEPT_HEADER),
            HeaderValue::from_static("kept"),
        ),
        (CONTENT_LENGTH, HeaderValue::from(BARE_TEXT.len())),
    ];
    (StatusCode::from_u16(status).unwrap(), headers, BARE_TEXT).into_response()
}

/// Refuse, with an error envelope of the app's own.
#[utoipa::path(
    get,
    path = "/refused",
    tag = "fixtures",
    responses(
        (status = 200, description = "Never sent", body = SuccessEnvelope<NoData>),
        (status = 422, description = "Always", body = ErrorEnvelope),
    ),
)]
async fn refused() -> ApiResult<NoData> {
    let fields = FieldErrors::from([("name".to_owned(), vec!["is required".to_owned()])]);
    Err(ApiError::validation(fields))
}

/// A member to create: a body with every kind of field the extractor reports on, a nested
/// object and an array included.
#[derive(Debug, Deserialize, Serialize, Validate, ToSchema)]
pub(super) struct NewMember {
    #[garde(email)]
    email: String,
    #[garde(length(min = 3, max = 20), alphanumeric)]
    username: String,
    #[garde(dive)]
    address: Address,
    #[garde(inner(length(min = 1, max = 12)))]
    tags: Vec<String>,
}

/// Where a member lives.
#[derive(Debug, Deserialize, Serialize, Validate, ToSchema)]
struct Address {
    #[garde(length(min = 1, max = 80))]
    street: String,
    #[garde(range(min = 100_000, max = 999_999))]
    zip: u32,
}

/// Create a member, answering it as the handler received it.
#[utoipa::path(
    post,
    path = "/members",
    tag = "fixtures",
    request_body = NewMember,
    responses(
        (status = 200, description = "The member read", body = SuccessEnvelope<NewMember>),
        (status = 422, description = "A field failed validation", body = ErrorEnvelope),
    ),
)]
async fn create_member(ValidatedJson(member): ValidatedJson<NewMember>) -> ApiResult<NewMember> {
    Ok(ApiSuccess::ok("Member read", member))
}

/// Which members to list, read from the query string.
#[derive(Debug, Deserialize, Serialize, Validate, ToSchema)]
struct Listing {
    #[garde(range(min = 1, max = 50))]
    limit: u32,
    #[garde(length(min = 1, max = 12))]
    name: Option<String>,
}

/// List members, answering the listing as the handler received it.
#[utoipa::path(
    get,
    path = "/members",
    tag = "fixtures",
    params(
        ("limit" = u32, Query, description = "How many members to list"),
        ("name" = Option<String>, Query, description = "The name to look for"),
    ),
    responses(
        (status = 200, description = "The listing read", body = SuccessEnvelope<Listing>),
        (status = 422, description = "A parameter failed validation", body = ErrorEnvelope),
    ),
)]
async fn list_members(ValidatedQuery(listing): ValidatedQuery<Listing>) -> ApiResult<Listing> {
    Ok(ApiSuccess::ok("Listing read", listing))
}

pub(super) fn fixture_routes() -> ApiRouter {
    ApiRouter::new()
        .route(routes!(small), Access::public(), &[])
        .route(routes!(big), Access::public(), &[])
        .route(routes!(explode), Access::public(), &[])
        .route(routes!(length), Access::public(), &[])
        .route(routes!(bare), Access::public(), &[])
        .route(
            routes!(refused),
            Access::public(),
            &[ApiError::VALIDATION_FAILED],
        )
        .route(
            routes!(list_members, create_member),
            Access::public(),
            &[ApiError::VALIDATION_FAILED],
        )
}

pub(super) fn fixture_app() -> HttpApp {
    HttpApp::builder(SERVICE)
        .routes(fixture_routes())
        .build()
        .unwrap()
}

pub(super) fn client() -> TestClient {
    TestClient::new(&fixture_app())
}

/// An error envelope with `status` as its status and its top-level code, and `code` as
/// `error.code`.
pub(super) fn assert_error(response: &TestResponse, status: u16, code: &str) {
    let text = String::from_utf8_lossy(response.bytes()).into_owned();
    assert_eq!(response.status, status, "{text}");
    assert_eq!(response.header("content-type"), Some("application/json"));
    let body: Value = response.json();
    assert_eq!(body["success"], false, "{text}");
    assert_eq!(body["data"], Value::Null, "{text}");
    assert_eq!(body["code"], status, "{text}");
    assert_eq!(response.error_code().as_deref(), Some(code), "{text}");
}

/// Keeps what a subscriber writes, for the test that holds a clone.
#[derive(Clone, Default)]
struct Capture(Arc<Mutex<Vec<u8>>>);

impl io::Write for Capture {
    fn write(&mut self, bytes: &[u8]) -> io::Result<usize> {
        self.0.lock().unwrap().extend_from_slice(bytes);
        Ok(bytes.len())
    }

    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}

impl<'writer> MakeWriter<'writer> for Capture {
    type Writer = Self;

    fn make_writer(&'writer self) -> Self::Writer {
        self.clone()
    }
}

/// The answer to `request`, and the lines the app logged meanwhile. The subscriber is the
/// default of this thread only, so the test runs on tokio's current-thread runtime.
pub(super) async fn send_logged(
    client: &TestClient,
    request: TestRequest,
) -> (TestResponse, Vec<String>) {
    let capture = Capture::default();
    let response = {
        let _default = tracing_subscriber::fmt()
            .with_writer(capture.clone())
            .with_ansi(false)
            .without_time()
            .with_target(false)
            .finish()
            .set_default();
        client.send(request).await
    };
    let written = capture.0.lock().unwrap().clone();
    let text = String::from_utf8(written).unwrap();
    (response, text.lines().map(str::to_owned).collect())
}
