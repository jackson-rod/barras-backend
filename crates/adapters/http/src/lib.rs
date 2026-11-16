//! HTTP adapter exposing the application over REST.

// Taken for its features, not code; named so cargo's unused-dependency lint lets `just lint` pass.
use workspace_hack as _;

mod app;
mod envelope;
mod error;
mod extract;
mod health;
mod internal;
mod metrics;
mod middleware;
mod openapi;
mod router;
mod server;
#[cfg(any(test, feature = "testing"))]
pub mod testing;
#[cfg(test)]
mod tests;

pub use app::{BuildError, HttpApp, HttpAppBuilder, ServiceInfo};
pub use envelope::{ApiResult, ApiSuccess, ErrorDetail, ErrorEnvelope, NoData, SuccessEnvelope};
pub use error::{ApiError, FieldErrors};
pub use extract::{ValidatedJson, ValidatedQuery};
pub use health::{Criticality, HealthProbe};
pub use internal::InternalApp;
pub use metrics::{Metrics, MetricsError};
pub use middleware::MatchedRoute;
pub use openapi::{COMMON_ERROR_CODES, canonical_json, metadata_problems};
pub use router::{Access, ApiRouter};
pub use server::{Listener, ServeError, bind, serve_api, serve_internal};
