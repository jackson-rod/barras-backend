//! The error answer of the public API: the adapter's own failures, and any `AppError`
//! mapped from its kind.

use std::collections::BTreeMap;
use std::error::Error;

use axum::http::header::WWW_AUTHENTICATE;
use axum::http::{HeaderValue, StatusCode};
use axum::response::{IntoResponse, Response};
use barras_kernel::{AppError, ErrorKind};

use crate::envelope::{ErrorEnvelope, json_response};

const BEARER: HeaderValue = HeaderValue::from_static("Bearer");

/// Each field path that failed validation (`address.zip`, `items[2].amount`) with what is
/// expected of it. A message never repeats the value that was sent.
pub type FieldErrors = BTreeMap<String, Vec<String>>;

/// An error answer, sent as an [`ErrorEnvelope`] whose `code` is its status: a stable
/// `error.code`, a message for developers and, on a validation failure only, the fields
/// that failed.
///
/// A 5xx answers with a generic message and writes its detail to the log at `error` level;
/// the body never carries the detail. `ApiError` does not implement `AppError`: the blanket
/// `From` would then overlap the standard `From<T> for T`.
#[derive(Debug)]
pub struct ApiError {
    status: StatusCode,
    code: &'static str,
    message: String,
    fields: Option<FieldErrors>,
    /// For the log only.
    detail: Option<String>,
}

impl ApiError {
    /// The code of [`ApiError::validation`].
    pub const VALIDATION_FAILED: &'static str = "VALIDATION_FAILED";
    /// The code of [`ApiError::malformed_request`].
    pub const MALFORMED_REQUEST: &'static str = "MALFORMED_REQUEST";
    /// The code of [`ApiError::unsupported_media_type`].
    pub const UNSUPPORTED_MEDIA_TYPE: &'static str = "UNSUPPORTED_MEDIA_TYPE";
    /// The code of [`ApiError::payload_too_large`].
    pub const PAYLOAD_TOO_LARGE: &'static str = "PAYLOAD_TOO_LARGE";
    /// The code of [`ApiError::route_not_found`].
    pub const ROUTE_NOT_FOUND: &'static str = "ROUTE_NOT_FOUND";
    /// The code of [`ApiError::method_not_allowed`].
    pub const METHOD_NOT_ALLOWED: &'static str = "METHOD_NOT_ALLOWED";
    /// The code of [`ApiError::not_acceptable`].
    pub const NOT_ACCEPTABLE: &'static str = "NOT_ACCEPTABLE";
    /// The code of [`ApiError::service_unavailable`].
    pub const SERVICE_UNAVAILABLE: &'static str = "SERVICE_UNAVAILABLE";
    /// The code of [`ApiError::internal`].
    pub const INTERNAL_ERROR: &'static str = "INTERNAL_ERROR";

    /// 422 `VALIDATION_FAILED`, with every field that failed in `error.fields`.
    pub fn validation(fields: FieldErrors) -> Self {
        Self {
            fields: Some(fields),
            ..Self::client(
                StatusCode::UNPROCESSABLE_ENTITY,
                Self::VALIDATION_FAILED,
                "Request validation failed",
            )
        }
    }

    /// 400 `MALFORMED_REQUEST`: the body or the query string cannot be read. The message is
    /// fixed text, so it cannot repeat what was sent.
    pub fn malformed_request(message: &'static str) -> Self {
        Self::client(StatusCode::BAD_REQUEST, Self::MALFORMED_REQUEST, message)
    }

    /// 415 `UNSUPPORTED_MEDIA_TYPE`: the body is not sent as JSON.
    pub fn unsupported_media_type() -> Self {
        Self::client(
            StatusCode::UNSUPPORTED_MEDIA_TYPE,
            Self::UNSUPPORTED_MEDIA_TYPE,
            "Request body must be JSON",
        )
    }

    /// 413 `PAYLOAD_TOO_LARGE`: the body is over the size limit.
    pub fn payload_too_large() -> Self {
        Self::client(
            StatusCode::PAYLOAD_TOO_LARGE,
            Self::PAYLOAD_TOO_LARGE,
            "Request body is too large",
        )
    }

    /// 404 `ROUTE_NOT_FOUND`: no route matches the path.
    pub fn route_not_found() -> Self {
        Self::client(
            StatusCode::NOT_FOUND,
            Self::ROUTE_NOT_FOUND,
            "Route not found",
        )
    }

    /// 405 `METHOD_NOT_ALLOWED`: the path exists, the method does not.
    pub fn method_not_allowed() -> Self {
        Self::client(
            StatusCode::METHOD_NOT_ALLOWED,
            Self::METHOD_NOT_ALLOWED,
            "Method not allowed",
        )
    }

    /// 406 `NOT_ACCEPTABLE`: `Accept-Encoding` refuses every encoding the server has, the
    /// plain body included.
    pub fn not_acceptable() -> Self {
        Self::client(
            StatusCode::NOT_ACCEPTABLE,
            Self::NOT_ACCEPTABLE,
            "No acceptable content encoding",
        )
    }

    /// 503 `SERVICE_UNAVAILABLE`, with a generic message. `detail` goes only to the log.
    pub fn service_unavailable(detail: String) -> Self {
        Self::server(
            StatusCode::SERVICE_UNAVAILABLE,
            Self::SERVICE_UNAVAILABLE,
            detail,
        )
    }

    /// 500 `INTERNAL_ERROR`, with a generic message. `detail` goes only to the log.
    pub fn internal(detail: String) -> Self {
        Self::server(
            StatusCode::INTERNAL_SERVER_ERROR,
            Self::INTERNAL_ERROR,
            detail,
        )
    }

    /// The answer to an error a library layer gave without saying more than its status, at
    /// that status: `MALFORMED_REQUEST` for a 4xx, and for a 5xx `INTERNAL_ERROR`, whose
    /// detail goes to the log.
    pub(crate) fn unexplained(status: StatusCode) -> Self {
        if status.is_server_error() {
            let detail = format!("a layer answered {status} without an envelope");
            Self::server(status, Self::INTERNAL_ERROR, detail)
        } else {
            Self::client(
                status,
                Self::MALFORMED_REQUEST,
                "Request could not be handled",
            )
        }
    }

    /// The HTTP status, which is also the top-level `code` of the envelope.
    pub fn status(&self) -> StatusCode {
        self.status
    }

    /// The stable `SCREAMING_SNAKE_CASE` code sent as `error.code`.
    pub fn code(&self) -> &'static str {
        self.code
    }

    fn client(status: StatusCode, code: &'static str, message: impl Into<String>) -> Self {
        Self {
            status,
            code,
            message: message.into(),
            fields: None,
            detail: None,
        }
    }

    // ADR-0005: a 5xx says no more than this, because its detail may hold SQL or the words
    // of a dependency.
    fn server(status: StatusCode, code: &'static str, detail: String) -> Self {
        let message = if status == StatusCode::SERVICE_UNAVAILABLE {
            "Service unavailable"
        } else {
            "Internal server error"
        };
        Self {
            status,
            code,
            message: message.to_owned(),
            fields: None,
            detail: Some(detail),
        }
    }
}

/// Each kind answers its status of spec §6.2, with the code of the variant. A 5xx
/// (`Internal`, `Unavailable`) keeps the error's text and its chain of sources as the
/// detail for the log; any other kind answers the error's `Display` as its message.
///
/// A 4xx body carries that `Display` verbatim, so the message of a module's error must never
/// interpolate a source or a value the caller sent.
impl<E: AppError> From<E> for ApiError {
    fn from(error: E) -> Self {
        let status = status_of(error.kind());
        if status.is_server_error() {
            Self::server(status, error.code(), with_sources(&error))
        } else {
            Self::client(status, error.code(), error.to_string())
        }
    }
}

fn status_of(kind: ErrorKind) -> StatusCode {
    match kind {
        ErrorKind::Validation => StatusCode::UNPROCESSABLE_ENTITY,
        ErrorKind::Unauthenticated => StatusCode::UNAUTHORIZED,
        ErrorKind::Forbidden => StatusCode::FORBIDDEN,
        ErrorKind::NotFound => StatusCode::NOT_FOUND,
        ErrorKind::Conflict => StatusCode::CONFLICT,
        ErrorKind::RateLimited => StatusCode::TOO_MANY_REQUESTS,
        ErrorKind::Unavailable => StatusCode::SERVICE_UNAVAILABLE,
        ErrorKind::Internal => StatusCode::INTERNAL_SERVER_ERROR,
    }
}

/// The error's text, then the text of each of its sources in turn, joined by `": "`: at most
/// sixteen, because an error whose sources lead back to itself would never end.
pub(crate) fn with_sources(error: &(dyn Error + 'static)) -> String {
    std::iter::successors(Some(error), |&error| error.source())
        .take(16)
        .map(ToString::to_string)
        .collect::<Vec<_>>()
        .join(": ")
}

/// The detail of a 5xx goes to the log at `error` level, with its code and its status, and
/// the envelope goes out without it. A 401 also names the scheme to authenticate with, as RFC
/// 9110 §15.5.2 requires.
impl IntoResponse for ApiError {
    fn into_response(self) -> Response {
        if let Some(detail) = &self.detail {
            log_server_failure(self.code, self.status, detail);
        }
        let envelope = ErrorEnvelope::new(self.status, self.message, self.code, self.fields);
        let mut response = json_response(self.status, &envelope);
        if response.status() == StatusCode::UNAUTHORIZED {
            response.headers_mut().insert(WWW_AUTHENTICATE, BEARER);
        }
        response
    }
}

/// The one log event of a failure on the server's side: what was answered, and the detail
/// that no body carries.
pub(crate) fn log_server_failure(code: &str, status: StatusCode, detail: &str) {
    tracing::error!(code, status = status.as_u16(), detail = %detail, "request failed");
}

#[cfg(test)]
mod tests {
    use std::fmt;
    use std::io;
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::sync::{Arc, Mutex};

    use axum::http::header::{CONTENT_TYPE, WWW_AUTHENTICATE};
    use barras_kernel::{Currency, KernelError};
    use http_body_util::BodyExt;
    use serde_json::{Value, json};
    use tracing_subscriber::fmt::MakeWriter;
    use tracing_subscriber::util::SubscriberInitExt;

    use super::*;
    use crate::envelope::{ApiResult, ApiSuccess};

    const EVERY_KIND: [ErrorKind; 8] = [
        ErrorKind::Validation,
        ErrorKind::Unauthenticated,
        ErrorKind::Forbidden,
        ErrorKind::NotFound,
        ErrorKind::Conflict,
        ErrorKind::RateLimited,
        ErrorKind::Unavailable,
        ErrorKind::Internal,
    ];

    /// The table of spec §6.2. The `match` has no wildcard arm, so a new kind does not
    /// compile until it has its row here; list it in `EVERY_KIND` as well.
    fn expected_status(kind: ErrorKind) -> u16 {
        match kind {
            ErrorKind::Validation => 422,
            ErrorKind::Unauthenticated => 401,
            ErrorKind::Forbidden => 403,
            ErrorKind::NotFound => 404,
            ErrorKind::Conflict => 409,
            ErrorKind::RateLimited => 429,
            ErrorKind::Unavailable => 503,
            ErrorKind::Internal => 500,
        }
    }

    const SERVER_KINDS: [(ErrorKind, &str); 2] = [
        (ErrorKind::Internal, "Internal server error"),
        (ErrorKind::Unavailable, "Service unavailable"),
    ];

    /// The error of a module this adapter knows nothing about.
    #[derive(Debug)]
    struct SaveFailed {
        kind: ErrorKind,
        source: Option<Cause>,
    }

    impl SaveFailed {
        fn of(kind: ErrorKind) -> Self {
            Self { kind, source: None }
        }

        /// Caused by "connection lost", itself caused by "secret-sql-detail".
        fn with_secret_cause(kind: ErrorKind) -> Self {
            let sql = Cause {
                text: "secret-sql-detail",
                source: None,
            };
            let connection = Cause {
                text: "connection lost",
                source: Some(Box::new(sql)),
            };
            Self {
                kind,
                source: Some(connection),
            }
        }
    }

    impl fmt::Display for SaveFailed {
        fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
            f.write_str("the goal could not be saved")
        }
    }

    impl Error for SaveFailed {
        fn source(&self) -> Option<&(dyn Error + 'static)> {
            self.source
                .as_ref()
                .map(|cause| cause as &(dyn Error + 'static))
        }
    }

    impl AppError for SaveFailed {
        fn code(&self) -> &'static str {
            "GOAL_NOT_SAVED"
        }

        fn kind(&self) -> ErrorKind {
            self.kind
        }
    }

    /// An error that names itself as its own source, a thousand times, so that a test of the
    /// walk along the sources ends even when the walk is not cut.
    #[derive(Debug, Default)]
    struct Looping {
        calls: AtomicUsize,
    }

    impl fmt::Display for Looping {
        fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
            f.write_str("looping")
        }
    }

    impl Error for Looping {
        fn source(&self) -> Option<&(dyn Error + 'static)> {
            let calls = self.calls.fetch_add(1, Ordering::Relaxed);
            (calls < 1_000).then_some(self as &(dyn Error + 'static))
        }
    }

    impl AppError for Looping {
        fn code(&self) -> &'static str {
            "LOOPING"
        }

        fn kind(&self) -> ErrorKind {
            ErrorKind::Internal
        }
    }

    #[derive(Debug)]
    struct Cause {
        text: &'static str,
        source: Option<Box<Cause>>,
    }

    impl fmt::Display for Cause {
        fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
            f.write_str(self.text)
        }
    }

    impl Error for Cause {
        fn source(&self) -> Option<&(dyn Error + 'static)> {
            self.source
                .as_deref()
                .map(|cause| cause as &(dyn Error + 'static))
        }
    }

    /// Keeps what a subscriber writes, for the test that holds a clone.
    #[derive(Clone, Default)]
    struct Capture(Arc<Mutex<Vec<u8>>>);

    impl Capture {
        fn lines(&self) -> Vec<String> {
            let written = self.0.lock().unwrap().clone();
            let text = String::from_utf8(written).unwrap();
            text.lines().map(str::to_owned).collect()
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

    /// Builds the error of one case of a test.
    type MakeError = fn() -> ApiError;

    /// The response to the error `make` builds, and the lines logged while it was built and
    /// answered, each as `LEVEL message field=value…`. The subscriber is the default of this
    /// thread only, so tests running side by side do not see each other's events.
    fn respond_logged(make: impl FnOnce() -> ApiError) -> (Response, Vec<String>) {
        let capture = Capture::default();
        let response = {
            let _default = tracing_subscriber::fmt()
                .with_writer(capture.clone())
                .with_ansi(false)
                .without_time()
                .with_target(false)
                .with_max_level(tracing::Level::TRACE)
                .finish()
                .set_default();
            make().into_response()
        };
        (response, capture.lines())
    }

    /// The status, the content type and the JSON body of a response.
    async fn read(response: Response) -> (StatusCode, Option<String>, Value) {
        let status = response.status();
        let content_type = response
            .headers()
            .get(CONTENT_TYPE)
            .map(|value| value.to_str().unwrap().to_owned());
        let bytes = response.into_body().collect().await.unwrap().to_bytes();
        let body = serde_json::from_slice(&bytes)
            .unwrap_or_else(|error| panic!("{error}: {}", String::from_utf8_lossy(&bytes)));
        (status, content_type, body)
    }

    /// The keys of a JSON object, sorted.
    fn keys(value: &Value) -> Vec<&str> {
        let mut keys: Vec<&str> = value
            .as_object()
            .unwrap()
            .keys()
            .map(String::as_str)
            .collect();
        keys.sort_unstable();
        keys
    }

    const ENVELOPE_KEYS: [&str; 5] = ["code", "data", "error", "message", "success"];

    #[tokio::test]
    async fn every_kind_answers_its_status_and_the_code_of_the_variant() {
        for kind in EVERY_KIND {
            let error = ApiError::from(SaveFailed::of(kind));
            assert_eq!(error.status().as_u16(), expected_status(kind), "{kind:?}");
            assert_eq!(error.code(), "GOAL_NOT_SAVED", "{kind:?}");

            let (status, content_type, body) = read(error.into_response()).await;

            assert_eq!(status.as_u16(), expected_status(kind), "{kind:?}");
            assert_eq!(content_type.as_deref(), Some("application/json"));
            assert_eq!(keys(&body), ENVELOPE_KEYS, "{kind:?}");
            assert_eq!(body["success"], false);
            assert_eq!(body["data"], Value::Null);
            assert_eq!(body["code"], expected_status(kind), "{kind:?}");
            assert_eq!(
                body["error"],
                json!({ "code": "GOAL_NOT_SAVED" }),
                "{kind:?}"
            );
        }
    }

    #[tokio::test]
    async fn a_client_error_says_what_failed_in_its_message() {
        let client_kinds = EVERY_KIND
            .into_iter()
            .filter(|kind| expected_status(*kind) < 500);
        for kind in client_kinds {
            let error = SaveFailed::of(kind);
            let display = error.to_string();

            let (_, _, body) = read(ApiError::from(error).into_response()).await;

            assert_eq!(body["message"], display.as_str(), "{kind:?}");
        }
    }

    #[tokio::test]
    async fn a_server_error_answers_a_generic_message() {
        for (kind, generic) in SERVER_KINDS {
            let (_, _, body) = read(ApiError::from(SaveFailed::of(kind)).into_response()).await;
            assert_eq!(body["message"], generic, "{kind:?}");
        }
        for (error, generic) in [
            (
                ApiError::internal("handler panicked".to_owned()),
                "Internal server error",
            ),
            (
                ApiError::service_unavailable("critical probe failed".to_owned()),
                "Service unavailable",
            ),
        ] {
            let (_, _, body) = read(error.into_response()).await;
            assert_eq!(body["message"], generic);
        }
    }

    #[tokio::test]
    async fn a_server_error_logs_its_code_its_status_and_its_sources_and_sends_none() {
        for (kind, status) in [(ErrorKind::Internal, 500), (ErrorKind::Unavailable, 503)] {
            let (response, lines) =
                respond_logged(|| ApiError::from(SaveFailed::with_secret_cause(kind)));
            let (_, _, body) = read(response).await;

            let sent = body.to_string();
            assert!(!sent.contains("secret-sql-detail"), "{sent}");
            assert!(!sent.contains("connection lost"), "{sent}");
            assert!(!sent.contains("the goal could not be saved"), "{sent}");
            assert_eq!(
                lines,
                [format!(
                    "ERROR request failed code=\"GOAL_NOT_SAVED\" status={status} \
                     detail=the goal could not be saved: connection lost: secret-sql-detail"
                )]
            );
        }
    }

    #[tokio::test]
    async fn the_detail_given_to_a_server_failure_goes_only_to_the_log() {
        let cases: [(MakeError, &str); 2] = [
            (
                || ApiError::internal("secret-sql-detail".to_owned()),
                "ERROR request failed code=\"INTERNAL_ERROR\" status=500 detail=secret-sql-detail",
            ),
            (
                || ApiError::service_unavailable("secret-sql-detail".to_owned()),
                "ERROR request failed code=\"SERVICE_UNAVAILABLE\" status=503 \
                 detail=secret-sql-detail",
            ),
        ];
        for (make, logged) in cases {
            let (response, lines) = respond_logged(make);
            let (_, _, body) = read(response).await;

            assert!(!body.to_string().contains("secret-sql-detail"), "{body}");
            assert_eq!(lines, [logged]);
        }
    }

    #[test]
    fn a_chain_of_sources_is_cut_after_sixteen_errors() {
        let (_, lines) = respond_logged(|| ApiError::from(Looping::default()));

        let detail = ["looping"; 16].join(": ");
        assert_eq!(
            lines,
            [format!(
                "ERROR request failed code=\"LOOPING\" status=500 detail={detail}"
            )]
        );
    }

    #[test]
    fn a_client_error_writes_no_log() {
        let cases: [MakeError; 3] = [
            || ApiError::from(SaveFailed::with_secret_cause(ErrorKind::Conflict)),
            || {
                let message = vec!["is required".to_owned()];
                ApiError::validation(FieldErrors::from([("email".to_owned(), message)]))
            },
            ApiError::route_not_found,
        ];
        for make in cases {
            let (response, lines) = respond_logged(make);

            assert!(response.status().is_client_error(), "{}", response.status());
            assert!(lines.is_empty(), "{lines:?}");
        }
    }

    #[tokio::test]
    async fn a_client_error_sends_its_message_and_never_its_sources() {
        let client_kinds = EVERY_KIND
            .into_iter()
            .filter(|kind| expected_status(*kind) < 500);
        for kind in client_kinds {
            let error = ApiError::from(SaveFailed::with_secret_cause(kind));

            let (_, _, body) = read(error.into_response()).await;

            let sent = body.to_string();
            assert!(!sent.contains("secret-sql-detail"), "{kind:?}: {sent}");
            assert!(!sent.contains("connection lost"), "{kind:?}: {sent}");
            assert_eq!(body["message"], "the goal could not be saved", "{kind:?}");
        }
    }

    #[test]
    fn only_an_unauthenticated_answer_asks_for_a_bearer_token() {
        let challenged: Vec<(ErrorKind, String)> = EVERY_KIND
            .into_iter()
            .filter_map(|kind| {
                let response = ApiError::from(SaveFailed::of(kind)).into_response();
                let challenge = response.headers().get(WWW_AUTHENTICATE)?;
                Some((kind, challenge.to_str().unwrap().to_owned()))
            })
            .collect();

        assert_eq!(
            challenged,
            [(ErrorKind::Unauthenticated, "Bearer".to_owned())]
        );
    }

    #[tokio::test]
    async fn the_adapter_failures_answer_their_own_codes() {
        let fields = FieldErrors::from([("email".to_owned(), vec!["is required".to_owned()])]);
        for (error, status, code) in [
            (ApiError::validation(fields), 422, "VALIDATION_FAILED"),
            (
                ApiError::malformed_request("Request body is not valid JSON"),
                400,
                "MALFORMED_REQUEST",
            ),
            (
                ApiError::unsupported_media_type(),
                415,
                "UNSUPPORTED_MEDIA_TYPE",
            ),
            (ApiError::payload_too_large(), 413, "PAYLOAD_TOO_LARGE"),
            (ApiError::route_not_found(), 404, "ROUTE_NOT_FOUND"),
            (ApiError::method_not_allowed(), 405, "METHOD_NOT_ALLOWED"),
            (ApiError::not_acceptable(), 406, "NOT_ACCEPTABLE"),
            (
                ApiError::service_unavailable("critical probe failed".to_owned()),
                503,
                "SERVICE_UNAVAILABLE",
            ),
            (
                ApiError::internal("handler panicked".to_owned()),
                500,
                "INTERNAL_ERROR",
            ),
        ] {
            assert_eq!(error.status().as_u16(), status, "{code}");
            assert_eq!(error.code(), code);

            let (response_status, content_type, body) = read(error.into_response()).await;

            assert_eq!(response_status.as_u16(), status, "{code}");
            assert_eq!(content_type.as_deref(), Some("application/json"));
            assert_eq!(keys(&body), ENVELOPE_KEYS, "{code}");
            assert_eq!(body["success"], false);
            assert_eq!(body["data"], Value::Null);
            assert_eq!(body["code"], status, "{code}");
            assert_eq!(body["error"]["code"], code);
            let has_fields = body["error"].get("fields").is_some();
            assert_eq!(has_fields, code == "VALIDATION_FAILED", "{code}");
            assert!(!body["message"].as_str().unwrap().is_empty(), "{code}");
        }
    }

    #[tokio::test]
    async fn a_validation_failure_lists_every_field_with_its_messages() {
        let fields = FieldErrors::from([
            (
                "email".to_owned(),
                vec!["is not a valid email address".to_owned()],
            ),
            (
                "tags[1]".to_owned(),
                vec!["is too long".to_owned(), "is not valid".to_owned()],
            ),
        ]);

        let (status, _, body) = read(ApiError::validation(fields).into_response()).await;

        assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY);
        assert_eq!(
            body["error"],
            json!({
                "code": "VALIDATION_FAILED",
                "fields": {
                    "email": ["is not a valid email address"],
                    "tags[1]": ["is too long", "is not valid"]
                }
            })
        );
    }

    #[tokio::test]
    async fn an_unexplained_client_status_is_a_malformed_request_at_that_status() {
        for status in [StatusCode::BAD_REQUEST, StatusCode::IM_A_TEAPOT] {
            let (response, lines) = respond_logged(|| ApiError::unexplained(status));

            let (answered, _, body) = read(response).await;
            assert_eq!(answered, status);
            assert_eq!(body["code"], status.as_u16());
            assert_eq!(body["error"], json!({ "code": "MALFORMED_REQUEST" }));
            assert!(lines.is_empty(), "{lines:?}");
        }
    }

    #[tokio::test]
    async fn an_unexplained_server_status_is_an_internal_error_at_that_status_and_logged() {
        for (status, message) in [
            (StatusCode::INTERNAL_SERVER_ERROR, "Internal server error"),
            (StatusCode::SERVICE_UNAVAILABLE, "Service unavailable"),
        ] {
            let (response, lines) = respond_logged(|| ApiError::unexplained(status));

            let (answered, _, body) = read(response).await;
            assert_eq!(answered, status);
            assert_eq!(body["code"], status.as_u16());
            assert_eq!(body["message"], message);
            assert_eq!(body["error"], json!({ "code": "INTERNAL_ERROR" }));
            assert_eq!(lines.len(), 1, "{lines:?}");
            let expected = format!(
                "ERROR request failed code=\"INTERNAL_ERROR\" status={}",
                status.as_u16()
            );
            assert!(lines[0].starts_with(&expected), "{lines:?}");
        }
    }

    #[test]
    fn the_codes_of_the_adapter_are_public_and_never_change() {
        assert_eq!(
            [
                ApiError::VALIDATION_FAILED,
                ApiError::MALFORMED_REQUEST,
                ApiError::UNSUPPORTED_MEDIA_TYPE,
                ApiError::PAYLOAD_TOO_LARGE,
                ApiError::ROUTE_NOT_FOUND,
                ApiError::METHOD_NOT_ALLOWED,
                ApiError::NOT_ACCEPTABLE,
                ApiError::SERVICE_UNAVAILABLE,
                ApiError::INTERNAL_ERROR,
            ],
            [
                "VALIDATION_FAILED",
                "MALFORMED_REQUEST",
                "UNSUPPORTED_MEDIA_TYPE",
                "PAYLOAD_TOO_LARGE",
                "ROUTE_NOT_FOUND",
                "METHOD_NOT_ALLOWED",
                "NOT_ACCEPTABLE",
                "SERVICE_UNAVAILABLE",
                "INTERNAL_ERROR",
            ]
        );
    }

    #[tokio::test]
    async fn an_internal_error_is_the_exact_generic_envelope() {
        let (status, _, body) =
            read(ApiError::internal("disk full".to_owned()).into_response()).await;

        assert_eq!(status, StatusCode::INTERNAL_SERVER_ERROR);
        assert_eq!(
            body,
            json!({
                "success": false,
                "message": "Internal server error",
                "data": null,
                "code": 500,
                "error": { "code": "INTERNAL_ERROR" }
            })
        );
    }

    #[test]
    fn the_question_mark_turns_a_kernel_error_into_its_answer() {
        fn currency(code: &str) -> ApiResult<&'static str> {
            Currency::new(code)?;
            Ok(ApiSuccess::ok("Currency read", "ok"))
        }

        let error = currency("cop").unwrap_err();

        assert_eq!(error.status(), StatusCode::UNPROCESSABLE_ENTITY);
        assert_eq!(error.code(), "INVALID_CURRENCY");
        assert!(currency("COP").is_ok());
    }

    #[test]
    fn a_kernel_error_of_kind_internal_is_a_500_with_its_own_code() {
        let error = ApiError::from(KernelError::AmountOverflow);

        assert_eq!(error.status(), StatusCode::INTERNAL_SERVER_ERROR);
        assert_eq!(error.code(), "AMOUNT_OVERFLOW");
    }
}
