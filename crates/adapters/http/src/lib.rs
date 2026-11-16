//! HTTP adapter exposing the application over REST.

mod app;
mod envelope;
mod error;
mod middleware;
mod openapi;
mod router;
#[cfg(any(test, feature = "testing"))]
pub mod testing;
#[cfg(test)]
mod tests;

pub use app::{BuildError, HttpApp, HttpAppBuilder, ServiceInfo};
pub use envelope::{ApiResult, ApiSuccess, ErrorDetail, ErrorEnvelope, NoData, SuccessEnvelope};
pub use error::{ApiError, FieldErrors};
pub use middleware::MatchedRoute;
pub use openapi::{COMMON_ERROR_CODES, canonical_json, metadata_problems};
pub use router::{Access, ApiRouter};
