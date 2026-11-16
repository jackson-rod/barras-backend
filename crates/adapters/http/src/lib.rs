//! HTTP adapter exposing the application over REST.

mod envelope;
mod error;
mod openapi;
mod router;

pub use envelope::{ApiResult, ApiSuccess, ErrorDetail, ErrorEnvelope, SuccessEnvelope};
pub use error::{ApiError, FieldErrors};
pub use openapi::{COMMON_ERROR_CODES, canonical_json, metadata_problems};
pub use router::{Access, ApiRouter};
