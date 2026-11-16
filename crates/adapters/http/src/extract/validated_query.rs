//! A query string, read into its type and checked against its rules before the handler sees
//! it.

use axum::extract::FromRequestParts;
use axum::http::request::Parts;
use garde::Validate;
use serde::de::DeserializeOwned;

use super::{broken_rules, refused_value};
use crate::error::ApiError;

const UNREADABLE: &str = "Query string cannot be read";

/// The query string read into `T` and checked against every rule `T` declares. A handler takes
/// its query with it, never with axum's `Query`, whose rejections are not envelopes.
///
/// A parameter that is missing, has the wrong type or breaks a rule answers 422
/// `VALIDATION_FAILED` under its name. A query string `T` cannot be read from, though no
/// parameter is to blame, such as one that gives a parameter twice, answers 400
/// `MALFORMED_REQUEST`.
#[derive(Debug)]
pub struct ValidatedQuery<T>(pub T);

impl<T, S> FromRequestParts<S> for ValidatedQuery<T>
where
    T: DeserializeOwned + Validate,
    T::Context: Default,
    S: Send + Sync,
{
    type Rejection = ApiError;

    async fn from_request_parts(parts: &mut Parts, _state: &S) -> Result<Self, ApiError> {
        let query = parts.uri.query().unwrap_or_default();
        // Read as axum's `Query` reads it, except that the path of a refused value is kept
        // where this code can see it, not inside a boxed error.
        let pairs = serde_urlencoded::Deserializer::new(form_urlencoded::parse(query.as_bytes()));
        let value: T = serde_path_to_error::deserialize(pairs)
            .map_err(|error| refused_value(error.path(), &error.inner().to_string(), UNREADABLE))?;
        value.validate().map_err(|report| broken_rules(&report))?;
        Ok(Self(value))
    }
}
