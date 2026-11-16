//! The two shapes of every response body on the public port, and the success a handler
//! returns.

use std::borrow::Cow;

use axum::http::header::CONTENT_TYPE;
use axum::http::{HeaderValue, StatusCode};
use axum::response::{IntoResponse, Response};
use serde::Serialize;
use utoipa::openapi::schema::{ObjectBuilder, Type};
use utoipa::openapi::{RefOr, Schema};
use utoipa::{PartialSchema, ToSchema};

use crate::error::{ApiError, FieldErrors, log_server_failure};

const APPLICATION_JSON: HeaderValue = HeaderValue::from_static("application/json");

/// What `ApiError::internal` sends, written out so that the answer to a body serde_json
/// refuses cannot fail in turn. A test holds it equal to that body.
const INTERNAL_ERROR_BODY: &str = r#"{"success":false,"message":"Internal server error","data":null,"code":500,"error":{"code":"INTERNAL_ERROR"}}"#;

/// The body of every successful response. `success` is always true and `code` is always the
/// HTTP status of the response.
#[derive(Debug, Serialize, ToSchema)]
pub struct SuccessEnvelope<T> {
    success: bool,
    message: String,
    data: T,
    code: u16,
}

/// The body of every error response. `success` is always false, `data` is always null and
/// `code` is always the HTTP status of the response.
#[derive(Debug, Serialize, ToSchema)]
pub struct ErrorEnvelope {
    success: bool,
    message: String,
    #[schema(schema_with = null_schema)]
    data: (),
    code: u16,
    error: ErrorDetail,
}

/// What failed: the stable code to choose a text by and, on a validation failure only, the
/// messages for each field path.
#[derive(Debug, Serialize, ToSchema)]
pub struct ErrorDetail {
    code: &'static str,
    // utoipa makes every `Option` nullable, but this key is absent or a map, never null.
    // It cannot see through a type alias either, and would refer to a component named
    // after the aliased type (`BTreeMap`); `inline` writes the map's own schema here.
    #[serde(skip_serializing_if = "Option::is_none")]
    #[schema(inline, nullable = false)]
    fields: Option<FieldErrors>,
}

// utoipa's own schema for `()` is `{"default": null}`, which accepts any value.
fn null_schema() -> RefOr<Schema> {
    ObjectBuilder::new().schema_type(Type::Null).into()
}

/// The `data` of a route with nothing to return, sent as `null`. A route documents its
/// success as `SuccessEnvelope<NoData>`: `#[utoipa::path]` refuses `SuccessEnvelope<()>`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
pub struct NoData;

// Derived, the schema of a unit struct is `{"default": null}`, which accepts any value.
impl PartialSchema for NoData {
    fn schema() -> RefOr<Schema> {
        null_schema()
    }
}

impl ToSchema for NoData {
    fn name() -> Cow<'static, str> {
        Cow::Borrowed("NoData")
    }
}

impl ErrorEnvelope {
    pub(crate) fn new(
        status: StatusCode,
        message: String,
        code: &'static str,
        fields: Option<FieldErrors>,
    ) -> Self {
        Self {
            success: false,
            message,
            data: (),
            code: status.as_u16(),
            error: ErrorDetail { code, fields },
        }
    }
}

/// A successful answer, sent as a [`SuccessEnvelope`] whose `code` is its status. A route
/// with nothing to return answers [`NoData`], sent as `null`, with 200: never 204.
#[derive(Debug)]
pub struct ApiSuccess<T> {
    status: StatusCode,
    message: String,
    data: T,
}

impl<T> ApiSuccess<T> {
    /// 200 OK. The message is for developers; the app shows its own text.
    pub fn ok(message: impl Into<String>, data: T) -> Self {
        Self {
            status: StatusCode::OK,
            message: message.into(),
            data,
        }
    }

    /// 201 Created. The message is for developers; the app shows its own text.
    pub fn created(message: impl Into<String>, data: T) -> Self {
        Self {
            status: StatusCode::CREATED,
            message: message.into(),
            data,
        }
    }
}

/// Data that cannot be serialized answers 500 `INTERNAL_ERROR`, with the cause in the log
/// only.
impl<T: Serialize> IntoResponse for ApiSuccess<T> {
    fn into_response(self) -> Response {
        let envelope = SuccessEnvelope {
            success: true,
            message: self.message,
            data: self.data,
            code: self.status.as_u16(),
        };
        json_response(self.status, &envelope)
    }
}

/// What a handler returns. `?` turns any `AppError` into its error answer:
///
/// ```
/// use barras_adapter_http::{ApiResult, ApiSuccess};
/// use barras_kernel::Currency;
///
/// fn read_currency(code: &str) -> ApiResult<String> {
///     let currency = Currency::new(code)?;
///     Ok(ApiSuccess::ok("Currency read", currency.as_str().to_owned()))
/// }
///
/// assert_eq!(read_currency("cop").unwrap_err().code(), "INVALID_CURRENCY");
/// ```
pub type ApiResult<T> = Result<ApiSuccess<T>, ApiError>;

/// `body` as a JSON response. A body serde_json refuses, which only the data of a success can
/// be, answers the fixed `INTERNAL_ERROR` envelope instead, with the cause in the log only.
pub(crate) fn json_response(status: StatusCode, body: &impl Serialize) -> Response {
    let content_type = [(CONTENT_TYPE, APPLICATION_JSON)];
    match serde_json::to_vec(body) {
        Ok(bytes) => (status, content_type, bytes).into_response(),
        Err(error) => {
            let status = StatusCode::INTERNAL_SERVER_ERROR;
            let detail = format!("the response body could not be serialized: {error}");
            log_server_failure(ApiError::INTERNAL_ERROR, status, &detail);
            (status, content_type, INTERNAL_ERROR_BODY).into_response()
        }
    }
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeMap;
    use std::io;
    use std::sync::{Arc, Mutex};

    use http_body_util::BodyExt;
    use serde_json::{Value, json};
    use tracing_subscriber::fmt::MakeWriter;
    use tracing_subscriber::util::SubscriberInitExt;
    use utoipa::PartialSchema;
    use utoipa_axum::router::UtoipaMethodRouter;
    use utoipa_axum::routes;

    use super::*;

    #[derive(Serialize, ToSchema)]
    struct Readiness {
        status: &'static str,
    }

    /// Save the settings, with nothing to return.
    #[utoipa::path(
        put,
        path = "/settings",
        responses((status = 200, description = "Saved", body = SuccessEnvelope<NoData>)),
    )]
    async fn save_settings() -> ApiResult<NoData> {
        Ok(ApiSuccess::ok("Settings saved", NoData))
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

    /// The response `make` builds, and the lines logged meanwhile, each as
    /// `LEVEL message field=value…`, on this thread only.
    fn respond_logged(make: impl FnOnce() -> Response) -> (Response, Vec<String>) {
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
            make()
        };
        let written = capture.0.lock().unwrap().clone();
        let lines = String::from_utf8(written).unwrap();
        (response, lines.lines().map(str::to_owned).collect())
    }

    async fn body_text(response: Response) -> String {
        let bytes = response.into_body().collect().await.unwrap().to_bytes();
        String::from_utf8(bytes.to_vec()).unwrap()
    }

    /// Data that serde_json refuses, with a reason that must not reach the client.
    struct Unserializable;

    impl Serialize for Unserializable {
        fn serialize<S: serde::Serializer>(&self, _serializer: S) -> Result<S::Ok, S::Error> {
            Err(serde::ser::Error::custom("unserializable-secret"))
        }
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

    fn schema_of<T: PartialSchema>() -> Value {
        serde_json::to_value(T::schema()).unwrap()
    }

    #[tokio::test]
    async fn a_success_is_the_exact_envelope_with_the_status_as_its_code() {
        let success = ApiSuccess::ok("Service is ready", Readiness { status: "ok" });

        let (status, content_type, body) = read(success.into_response()).await;

        assert_eq!(status, StatusCode::OK);
        assert_eq!(content_type.as_deref(), Some("application/json"));
        assert_eq!(
            body,
            json!({
                "success": true,
                "message": "Service is ready",
                "data": { "status": "ok" },
                "code": 200
            })
        );
    }

    #[tokio::test]
    async fn created_answers_201_and_says_so_in_the_body() {
        let success = ApiSuccess::created("Goal created", Readiness { status: "ok" });

        let (status, content_type, body) = read(success.into_response()).await;

        assert_eq!(status, StatusCode::CREATED);
        assert_eq!(content_type.as_deref(), Some("application/json"));
        assert_eq!(
            body,
            json!({
                "success": true,
                "message": "Goal created",
                "data": { "status": "ok" },
                "code": 201
            })
        );
    }

    #[tokio::test]
    async fn nothing_to_return_is_200_with_data_null() {
        let success = ApiSuccess::ok("Settings saved", NoData);

        let (status, _, body) = read(success.into_response()).await;

        assert_eq!(status, StatusCode::OK);
        assert_eq!(
            body,
            json!({ "success": true, "message": "Settings saved", "data": null, "code": 200 })
        );
    }

    #[test]
    fn no_data_is_null_on_the_wire_and_in_its_schema() {
        assert_eq!(serde_json::to_value(NoData).unwrap(), Value::Null);
        assert_eq!(schema_of::<NoData>(), json!({ "type": "null" }));
        assert_eq!(NoData::name(), "NoData");
    }

    #[test]
    fn a_route_with_nothing_to_return_documents_its_data_as_null() {
        let (schemas, paths, _): UtoipaMethodRouter = routes!(save_settings);

        let schemas: BTreeMap<String, Value> = schemas
            .into_iter()
            .map(|(name, schema)| (name, serde_json::to_value(schema).unwrap()))
            .collect();
        let names: Vec<&str> = schemas.keys().map(String::as_str).collect();
        assert_eq!(names, ["NoData", "SuccessEnvelope_NoData"]);
        assert_eq!(
            schemas["SuccessEnvelope_NoData"]["properties"]["data"],
            json!({ "type": "null" })
        );
        let paths = serde_json::to_value(paths).unwrap();
        assert_eq!(
            paths["/settings"]["put"]["responses"]["200"]["content"]["application/json"]["schema"],
            json!({ "$ref": "#/components/schemas/SuccessEnvelope_NoData" })
        );
    }

    #[tokio::test]
    async fn data_that_cannot_be_serialized_answers_the_fixed_internal_error() {
        let (response, lines) =
            respond_logged(|| ApiSuccess::ok("Never sent", Unserializable).into_response());

        assert_eq!(response.status(), StatusCode::INTERNAL_SERVER_ERROR);
        let content_type = response.headers().get(CONTENT_TYPE).cloned();
        assert_eq!(content_type, Some(APPLICATION_JSON));
        assert_eq!(body_text(response).await, INTERNAL_ERROR_BODY);
        assert_eq!(
            lines,
            ["ERROR request failed code=\"INTERNAL_ERROR\" status=500 \
                 detail=the response body could not be serialized: unserializable-secret"]
        );
    }

    #[tokio::test]
    async fn the_fixed_internal_error_is_the_body_an_internal_error_sends() {
        let response = ApiError::internal("disk full".to_owned()).into_response();

        assert_eq!(body_text(response).await, INTERNAL_ERROR_BODY);
    }

    #[test]
    fn an_error_envelope_without_fields_has_data_null_and_no_fields_key() {
        let envelope = ErrorEnvelope::new(
            StatusCode::NOT_FOUND,
            "Route not found".to_owned(),
            "ROUTE_NOT_FOUND",
            None,
        );

        assert_eq!(
            serde_json::to_value(envelope).unwrap(),
            json!({
                "success": false,
                "message": "Route not found",
                "data": null,
                "code": 404,
                "error": { "code": "ROUTE_NOT_FOUND" }
            })
        );
    }

    #[test]
    fn an_error_envelope_with_fields_is_the_validation_shape_of_the_spec() {
        let fields = FieldErrors::from([(
            "email".to_owned(),
            vec!["is not a valid email address".to_owned()],
        )]);
        let envelope = ErrorEnvelope::new(
            StatusCode::UNPROCESSABLE_ENTITY,
            "Request validation failed".to_owned(),
            "VALIDATION_FAILED",
            Some(fields),
        );

        assert_eq!(
            serde_json::to_value(envelope).unwrap(),
            json!({
                "success": false,
                "message": "Request validation failed",
                "data": null,
                "code": 422,
                "error": {
                    "code": "VALIDATION_FAILED",
                    "fields": { "email": ["is not a valid email address"] }
                }
            })
        );
    }

    #[test]
    fn the_schema_of_error_data_is_null_and_data_is_required() {
        let schema = schema_of::<ErrorEnvelope>();

        assert_eq!(schema["properties"]["data"], json!({ "type": "null" }));
        assert_eq!(
            schema["required"],
            json!(["success", "message", "data", "code", "error"])
        );
    }

    #[test]
    fn the_schema_of_error_fields_is_an_optional_map_of_messages_and_never_null() {
        let schema = schema_of::<ErrorDetail>();

        assert_eq!(schema["required"], json!(["code"]));
        assert_eq!(
            schema["properties"]["fields"],
            json!({
                "type": "object",
                "propertyNames": { "type": "string" },
                "additionalProperties": { "type": "array", "items": { "type": "string" } }
            }),
            "{schema}"
        );
    }

    #[test]
    fn the_schema_of_a_success_envelope_requires_its_four_keys() {
        let schema = schema_of::<SuccessEnvelope<Readiness>>();

        assert_eq!(
            schema["required"],
            json!(["success", "message", "data", "code"])
        );
        assert!(schema["properties"]["data"].is_object(), "{schema}");
    }
}
