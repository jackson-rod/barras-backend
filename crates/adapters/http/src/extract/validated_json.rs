//! A JSON body, read whole and checked against its type's rules before the handler sees it.

use axum::body::Bytes;
use axum::extract::{FromRequest, Request};
use axum::http::header::CONTENT_TYPE;
use axum::http::{HeaderMap, StatusCode};
use garde::Validate;
use serde::de::{DeserializeOwned, IgnoredAny};

use super::{broken_rules, refused_value};
use crate::error::ApiError;

const NOT_JSON: &str = "Request body is not valid JSON";
const NOT_AN_OBJECT: &str = "Request body is not a JSON object";
const UNREADABLE: &str = "Request body could not be read";
const UNFIT: &str = "Request body cannot be read as the object this route takes";

/// The request body, a JSON object, read into `T` and checked against every rule `T` declares.
/// A handler takes its body with it, never with axum's `Json`, whose rejections are not
/// envelopes.
///
/// A body not sent as JSON (`application/json` or `application/<x>+json`, parameters aside)
/// answers 415 `UNSUPPORTED_MEDIA_TYPE`; one over the size limit, 413 `PAYLOAD_TOO_LARGE`; one
/// that is not a JSON object, 400 `MALFORMED_REQUEST`; and one whose fields are missing, have
/// the wrong type or break a rule, 422 `VALIDATION_FAILED` with each such field.
#[derive(Debug)]
pub struct ValidatedJson<T>(pub T);

impl<T, S> FromRequest<S> for ValidatedJson<T>
where
    T: DeserializeOwned + Validate,
    T::Context: Default,
    S: Send + Sync,
{
    type Rejection = ApiError;

    async fn from_request(request: Request, state: &S) -> Result<Self, ApiError> {
        if !is_json(request.headers()) {
            return Err(ApiError::unsupported_media_type());
        }
        let bytes = Bytes::from_request(request, state)
            .await
            .map_err(|rejection| {
                // The rejection type is non-exhaustive: only its status tells the size limit
                // from a body that broke off.
                if rejection.status() == StatusCode::PAYLOAD_TOO_LARGE {
                    ApiError::payload_too_large()
                } else {
                    ApiError::malformed_request(UNREADABLE)
                }
            })?;
        // A derived struct also reads a JSON array, by position.
        if first_token(&bytes) != Some(b'{') {
            return Err(ApiError::malformed_request(NOT_AN_OBJECT));
        }
        let mut deserializer = serde_json::Deserializer::from_slice(&bytes);
        let value: T = serde_path_to_error::deserialize(&mut deserializer).map_err(|error| {
            // serde stops at the first value it refuses, which may come before the place where
            // the body stops being JSON; a body that is not JSON is malformed, whatever it holds.
            // Only this path reads the body twice.
            if error.inner().is_data() && serde_json::from_slice::<IgnoredAny>(&bytes).is_ok() {
                refused_value(error.path(), &without_position(error.inner()), UNFIT)
            } else {
                ApiError::malformed_request(NOT_JSON)
            }
        })?;
        deserializer
            .end()
            .map_err(|_| ApiError::malformed_request(NOT_JSON))?;
        value.validate().map_err(|report| broken_rules(&report))?;
        Ok(Self(value))
    }
}

/// `application/json` or `application/<x>+json`, in any case, its parameters aside.
fn is_json(headers: &HeaderMap) -> bool {
    let Some(Ok(content_type)) = headers.get(CONTENT_TYPE).map(|value| value.to_str()) else {
        return false;
    };
    let essence = content_type.split(';').next().unwrap_or_default().trim();
    let Some((kind, subtype)) = essence.split_once('/') else {
        return false;
    };
    let has_json_suffix = subtype
        .rsplit_once('+')
        .is_some_and(|(name, suffix)| !name.is_empty() && suffix.eq_ignore_ascii_case("json"));
    kind.eq_ignore_ascii_case("application")
        && (subtype.eq_ignore_ascii_case("json") || has_json_suffix)
}

/// The first byte of `bytes` that is not JSON white space.
fn first_token(bytes: &[u8]) -> Option<u8> {
    bytes
        .iter()
        .copied()
        .find(|byte| !matches!(byte, b' ' | b'\t' | b'\n' | b'\r'))
}

/// serde_json's text for `error` without the position it appends.
fn without_position(error: &serde_json::Error) -> String {
    let text = error.to_string();
    let position = format!(" at line {} column {}", error.line(), error.column());
    text.strip_suffix(&position).unwrap_or(&text).to_owned()
}
