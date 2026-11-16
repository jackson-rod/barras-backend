//! HTTP adapter exposing the application over REST.

mod envelope;
mod error;

pub use envelope::{ApiResult, ApiSuccess, ErrorDetail, ErrorEnvelope, SuccessEnvelope};
pub use error::{ApiError, FieldErrors};
