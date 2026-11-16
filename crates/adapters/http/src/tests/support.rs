//! The fixture app the tests run against, its routes documented and registered with
//! `ApiRouter` as the real app's are, and a writer that keeps what the app logs.

use std::io;
use std::sync::{Arc, Mutex};

use axum::body::Bytes;
use axum::extract::Path;
use axum::http::header::CONTENT_LENGTH;
use axum::http::{HeaderName, HeaderValue, StatusCode};
use axum::response::{IntoResponse, Response};
use serde::Serialize;
use tracing_subscriber::fmt::MakeWriter;
use tracing_subscriber::util::SubscriberInitExt;
use utoipa::ToSchema;
use utoipa_axum::routes;

use crate::testing::{TestClient, TestRequest, TestResponse};
use crate::{
    Access, ApiError, ApiResult, ApiRouter, ApiSuccess, ErrorEnvelope, FieldErrors, HttpApp,
    NoData, ServiceInfo, SuccessEnvelope,
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
}

pub(super) fn fixture_app() -> HttpApp {
    HttpApp::builder(SERVICE).routes(fixture_routes()).build()
}

pub(super) fn client() -> TestClient {
    TestClient::new(&fixture_app())
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
