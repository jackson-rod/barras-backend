//! The two shapes of every response body on the public port, and the success a handler
//! returns.

use axum::http::header::CONTENT_TYPE;
use axum::http::{HeaderValue, StatusCode};
use axum::response::{IntoResponse, Response};
use serde::Serialize;
use utoipa::ToSchema;
use utoipa::openapi::schema::{ObjectBuilder, Type};
use utoipa::openapi::{RefOr, Schema};

use crate::error::{ApiError, FieldErrors};

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

/// A successful answer, sent as a [`SuccessEnvelope`] whose `code` is its status. `data`
/// of `()` is sent as `null`: a route with nothing to return still answers 200, never 204.
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
        json_response(self.status, &envelope).unwrap_or_else(|error| {
            ApiError::internal(format!(
                "the response data could not be serialized: {error}"
            ))
            .into_response()
        })
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

pub(crate) fn json_response(
    status: StatusCode,
    body: &impl Serialize,
) -> Result<Response, serde_json::Error> {
    let bytes = serde_json::to_vec(body)?;
    let content_type = [(CONTENT_TYPE, HeaderValue::from_static("application/json"))];
    Ok((status, content_type, bytes).into_response())
}

#[cfg(test)]
mod tests {
    use http_body_util::BodyExt;
    use serde_json::{Value, json};
    use utoipa::PartialSchema;

    use super::*;

    #[derive(Serialize, ToSchema)]
    struct Readiness {
        status: &'static str,
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
        let (status, _, body) = read(ApiSuccess::ok("Settings saved", ()).into_response()).await;

        assert_eq!(status, StatusCode::OK);
        assert_eq!(
            body,
            json!({ "success": true, "message": "Settings saved", "data": null, "code": 200 })
        );
    }

    #[tokio::test]
    async fn data_that_cannot_be_serialized_answers_an_internal_error() {
        let success = ApiSuccess::ok("Never sent", Unserializable);

        let (status, content_type, body) = read(success.into_response()).await;

        assert_eq!(status, StatusCode::INTERNAL_SERVER_ERROR);
        assert_eq!(content_type.as_deref(), Some("application/json"));
        assert_eq!(body["code"], 500);
        assert_eq!(body["error"]["code"], "INTERNAL_ERROR");
        assert!(
            !body.to_string().contains("unserializable-secret"),
            "{body}"
        );
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
