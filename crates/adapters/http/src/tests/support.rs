//! The fixture app the tests run against, its routes documented and registered with
//! `ApiRouter` as the real app's are, and a writer that keeps what the app logs.

use std::collections::BTreeMap;
use std::io;
use std::sync::{Arc, Mutex};

use axum::body::Bytes;
use axum::extract::Path;
use axum::http::header::{CONTENT_LENGTH, CONTENT_RANGE, CONTENT_TYPE, ETAG, LAST_MODIFIED};
use axum::http::{HeaderName, HeaderValue, StatusCode};
use axum::response::{IntoResponse, Response};
use garde::Validate;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use tracing::Level;
use tracing_subscriber::fmt::MakeWriter;
use tracing_subscriber::util::SubscriberInitExt;
use utoipa::ToSchema;
use utoipa_axum::routes;

use crate::testing::{TestClient, TestRequest, TestResponse};
use crate::{
    Access, ApiError, ApiResult, ApiRouter, ApiSuccess, ErrorEnvelope, FieldErrors, HttpApp,
    Metrics, NoData, ServiceInfo, SuccessEnvelope, ValidatedJson, ValidatedQuery,
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

/// How many bytes the content route answers: over the size from which a body is compressed.
pub(super) const CONTENT_SIZE: usize = 2048;

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

/// Answer the status asked for as a library layer does: plain text, with a length, a range,
/// and the validators of a cache.
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
        (CONTENT_RANGE, HeaderValue::from_static("bytes 0-31/32")),
        (ETAG, HeaderValue::from_static("\"bare\"")),
        (
            LAST_MODIFIED,
            HeaderValue::from_static("Mon, 16 Nov 2026 00:00:00 GMT"),
        ),
    ];
    (StatusCode::from_u16(status).unwrap(), headers, BARE_TEXT).into_response()
}

/// Answer the status asked for with `CONTENT_SIZE` bytes of the kind of content asked for, as
/// a library layer does: `image` (PNG), `grpc`, `sse` (an event stream) or `text`.
#[utoipa::path(
    get,
    path = "/content/{status}/{kind}",
    tag = "fixtures",
    params(
        ("status" = u16, Path, description = "The status to answer"),
        ("kind" = String, Path, description = "The kind of content"),
    ),
    responses((status = 200, description = "Content of that kind", body = SuccessEnvelope<NoData>)),
)]
async fn content(Path((status, kind)): Path<(u16, String)>) -> Response {
    let content_type = match kind.as_str() {
        "image" => "image/png",
        "grpc" => "application/grpc",
        "sse" => "text/event-stream",
        _ => "text/plain",
    };
    let status = StatusCode::from_u16(status).unwrap();
    let headers = [(CONTENT_TYPE, HeaderValue::from_static(content_type))];
    (status, headers, vec![b'a'; CONTENT_SIZE]).into_response()
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
#[serde(rename_all = "camelCase")]
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

/// A profile to create: fields whose Rust names are not their JSON names, and the shapes whose
/// refusal could carry back what was sent: objects that refuse a field they do not name, types
/// whose own errors quote what was sent, a tuple, three enums and a list of objects.
#[derive(Debug, Deserialize, Serialize, Validate, ToSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct NewProfile {
    #[garde(length(min = 1, max = 20))]
    display_name: String,
    #[garde(dive)]
    #[serde(skip_serializing_if = "Option::is_none")]
    strict_named: Option<StrictNamed>,
    #[garde(skip)]
    #[serde(skip_serializing_if = "Option::is_none")]
    strict: Option<Strict>,
    #[garde(skip)]
    #[serde(skip_serializing_if = "Option::is_none")]
    #[schema(value_type = Option<String>)]
    nickname: Option<Nickname>,
    #[garde(skip)]
    #[serde(skip_serializing_if = "Option::is_none")]
    #[schema(value_type = Option<Vec<u32>>)]
    pair: Option<(u32, u32)>,
    #[garde(skip)]
    #[serde(skip_serializing_if = "Option::is_none")]
    kind: Option<ProfileKind>,
    #[garde(skip)]
    #[serde(skip_serializing_if = "Option::is_none")]
    target: Option<Target>,
    #[garde(skip)]
    #[serde(skip_serializing_if = "Option::is_none")]
    #[schema(value_type = Option<String>)]
    code: Option<Code>,
    #[garde(skip)]
    #[serde(skip_serializing_if = "Option::is_none")]
    #[schema(value_type = Option<String>)]
    echo: Option<Echo>,
    #[garde(skip)]
    #[serde(skip_serializing_if = "Option::is_none")]
    #[schema(value_type = Option<Object>)]
    lookup: Option<Lookup>,
    #[garde(dive)]
    #[serde(skip_serializing_if = "Option::is_none")]
    addresses: Option<Vec<Address>>,
    #[garde(skip)]
    #[serde(skip_serializing_if = "Option::is_none")]
    plan: Option<SavingPlan>,
}

/// How a member saves, externally tagged: an object whose one key is the variant.
#[derive(Debug, Deserialize, Serialize, ToSchema)]
#[serde(rename_all = "camelCase")]
enum SavingPlan {
    Monthly { amount: u32 },
}

/// What a member does with money.
#[derive(Debug, Deserialize, Serialize, ToSchema)]
#[serde(rename_all = "camelCase")]
enum ProfileKind {
    Saver,
    Investor,
}

/// A target, internally tagged: serde reads its object whole before it knows the variant.
#[derive(Debug, Deserialize, Serialize, ToSchema)]
#[serde(tag = "type", rename_all = "camelCase", deny_unknown_fields)]
enum Target {
    Saving { amount: u32 },
}

/// Read through a conversion whose error carelessly starts with the value.
#[derive(Debug, Deserialize, Serialize)]
#[serde(try_from = "String")]
struct Code(String);

impl TryFrom<String> for Code {
    type Error = String;

    fn try_from(value: String) -> Result<Self, String> {
        Err(format!("{value} is not a valid code"))
    }
}

/// Read through a conversion whose error carelessly is the value itself.
#[derive(Debug, Deserialize, Serialize)]
#[serde(try_from = "String")]
struct Echo(String);

impl TryFrom<String> for Echo {
    type Error = String;

    fn try_from(value: String) -> Result<Self, String> {
        Err(value)
    }
}

/// Read from an object through a conversion whose error carelessly names, as a field it
/// lacks, the first value the object holds.
#[derive(Debug, Deserialize, Serialize)]
#[serde(try_from = "BTreeMap<String, String>")]
struct Lookup(BTreeMap<String, String>);

impl TryFrom<BTreeMap<String, String>> for Lookup {
    type Error = String;

    fn try_from(entries: BTreeMap<String, String>) -> Result<Self, String> {
        let first = entries.into_values().next().unwrap_or_default();
        Err(format!("missing field `{first}`"))
    }
}

/// An object that refuses a field it does not name.
#[derive(Debug, Deserialize, Serialize, Validate, ToSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct StrictNamed {
    #[garde(inner(range(max = 10)))]
    #[serde(skip_serializing_if = "Option::is_none")]
    max_level: Option<u8>,
}

/// An object that takes no field at all.
#[derive(Debug, Deserialize, Serialize, ToSchema)]
#[serde(deny_unknown_fields)]
struct Strict {}

/// Lower-case letters, read through a conversion whose error carelessly quotes the value
/// after the words serde writes before what it expected.
#[derive(Debug, Deserialize, Serialize)]
#[serde(try_from = "String")]
struct Nickname(String);

impl TryFrom<String> for Nickname {
    type Error = String;

    fn try_from(value: String) -> Result<Self, String> {
        if value.bytes().all(|byte| byte.is_ascii_lowercase()) {
            Ok(Self(value))
        } else {
            Err(format!("not a nickname, expected {value} to be lower-case"))
        }
    }
}

/// Create a profile, answering it as the handler received it.
#[utoipa::path(
    post,
    path = "/profiles",
    tag = "fixtures",
    request_body = NewProfile,
    responses(
        (status = 200, description = "The profile read", body = SuccessEnvelope<NewProfile>),
        (status = 422, description = "A field failed validation", body = ErrorEnvelope),
    ),
)]
async fn create_profile(
    ValidatedJson(profile): ValidatedJson<NewProfile>,
) -> ApiResult<NewProfile> {
    Ok(ApiSuccess::ok("Profile read", profile))
}

/// Which profiles to find, read from the query string.
#[derive(Debug, Deserialize, Serialize, Validate, ToSchema)]
#[serde(rename_all = "camelCase")]
struct ProfileQuery {
    #[garde(skip)]
    #[schema(value_type = Option<String>)]
    nickname: Option<Nickname>,
    #[garde(skip)]
    #[serde(skip_serializing_if = "Option::is_none")]
    #[schema(value_type = Option<String>)]
    code: Option<Code>,
    #[garde(skip)]
    #[serde(skip_serializing_if = "Option::is_none")]
    #[schema(value_type = Option<String>)]
    echo: Option<Echo>,
}

/// Find profiles, answering the query as the handler received it.
#[utoipa::path(
    get,
    path = "/profiles",
    tag = "fixtures",
    params(
        ("nickname" = Option<String>, Query, description = "The nickname to look for"),
        ("code" = Option<String>, Query, description = "The code to look for"),
        ("echo" = Option<String>, Query, description = "Anything, which is refused"),
    ),
    responses(
        (status = 200, description = "The query read", body = SuccessEnvelope<ProfileQuery>),
        (status = 422, description = "A parameter failed validation", body = ErrorEnvelope),
    ),
)]
async fn find_profiles(
    ValidatedQuery(query): ValidatedQuery<ProfileQuery>,
) -> ApiResult<ProfileQuery> {
    Ok(ApiSuccess::ok("Query read", query))
}

pub(super) fn fixture_routes() -> ApiRouter {
    ApiRouter::new()
        .route(routes!(small), Access::public(), &[])
        .route(routes!(big), Access::public(), &[])
        .route(routes!(explode), Access::public(), &[])
        .route(routes!(length), Access::public(), &[])
        .route(routes!(bare), Access::public(), &[])
        .route(routes!(content), Access::public(), &[])
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
        .route(
            routes!(find_profiles, create_profile),
            Access::public(),
            &[ApiError::VALIDATION_FAILED],
        )
}

/// Metrics of their own, for one app.
pub(super) fn fixture_metrics() -> Metrics {
    Metrics::new(SERVICE).unwrap()
}

pub(super) fn fixture_app() -> HttpApp {
    fixture_app_with(fixture_metrics())
}

/// The fixture app, recording what it answers in `metrics`.
pub(super) fn fixture_app_with(metrics: Metrics) -> HttpApp {
    HttpApp::builder(SERVICE, metrics)
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

impl Capture {
    fn text(&self) -> String {
        let written = self.0.lock().unwrap().clone();
        String::from_utf8(written).unwrap()
    }
}

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
    (
        response,
        capture.text().lines().map(str::to_owned).collect(),
    )
}

/// The answer to `request`, and each line the app logged meanwhile, at any level, parsed: the
/// JSON of every deployed environment (appendix A.5 of the plan), so that a test reads the
/// fields as an operator queries them. As for `send_logged`, the subscriber is the default of
/// this thread only.
pub(super) async fn send_logged_json(
    client: &TestClient,
    request: TestRequest,
) -> (TestResponse, Vec<Value>) {
    send_logged_json_at(client, request, Level::TRACE).await
}

/// As `send_logged_json`, with only what `level` and the levels above it let through, as
/// `LOG_LEVEL` does.
pub(super) async fn send_logged_json_at(
    client: &TestClient,
    request: TestRequest,
    level: Level,
) -> (TestResponse, Vec<Value>) {
    logged_json(level, client.send(request)).await
}

/// What `future` gives, and each line logged meanwhile at `level` or above, parsed, as for
/// `send_logged_json`. The subscriber is the default of this thread only, which on tokio's
/// current-thread runtime is also where every task the test started runs.
pub(super) async fn logged_json<T>(
    level: Level,
    future: impl Future<Output = T>,
) -> (T, Vec<Value>) {
    let capture = Capture::default();
    let output = {
        let _default = tracing_subscriber::fmt()
            .json()
            .with_current_span(false)
            .with_span_list(true)
            .with_max_level(level)
            .with_writer(capture.clone())
            .finish()
            .set_default();
        future.await
    };
    let lines = capture
        .text()
        .lines()
        .map(|line| serde_json::from_str(line).unwrap_or_else(|error| panic!("{error}: {line}")))
        .collect();
    (output, lines)
}
