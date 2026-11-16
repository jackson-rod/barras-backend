//! HTTP adapter exposing the application over REST.
//!
//! # Adding a route
//!
//! A module's routes live in `src/<module>/routes.rs` of this crate, and its DTOs in
//! `src/<module>/dto/`. A handler is thin: it takes its body with [`ValidatedJson`] and its
//! query with [`ValidatedQuery`], calls one use case, and answers an [`ApiResult`], into which
//! `?` turns the module's error. Its `#[utoipa::path]` has one tag, a summary (the first line
//! of its doc comment), a description on every response, `SuccessEnvelope<T>` on its 2xx and
//! `ErrorEnvelope` on every error status it can return. [`ApiRouter::route`] adds the handlers
//! of one path, gathered by `routes!`, with their [`Access`] and the route's own error codes:
//! those of the module errors it can return, and `VALIDATION_FAILED` when it takes a validating
//! extractor, never one of [`COMMON_ERROR_CODES`].
//!
//! ```
//! use barras_adapter_http::{
//!     Access, ApiError, ApiResult, ApiRouter, ApiSuccess, ErrorEnvelope, SuccessEnvelope,
//!     ValidatedJson,
//! };
//! use garde::Validate;
//! use serde::{Deserialize, Serialize};
//! use utoipa::ToSchema;
//! use utoipa_axum::routes;
//!
//! /// A goal to create.
//! #[derive(Deserialize, Validate, ToSchema)]
//! #[serde(rename_all = "camelCase")]
//! struct NewGoal {
//!     #[garde(length(min = 1, max = 80))]
//!     display_name: String,
//! }
//!
//! /// A goal.
//! #[derive(Serialize, ToSchema)]
//! #[serde(rename_all = "camelCase")]
//! struct Goal {
//!     display_name: String,
//! }
//!
//! /// Create a goal.
//! #[utoipa::path(
//!     post,
//!     path = "/goals",
//!     tag = "goals",
//!     request_body = NewGoal,
//!     responses(
//!         (status = 201, description = "The goal created", body = SuccessEnvelope<Goal>),
//!         (status = 422, description = "A field failed validation", body = ErrorEnvelope),
//!     ),
//! )]
//! async fn create_goal(ValidatedJson(goal): ValidatedJson<NewGoal>) -> ApiResult<Goal> {
//!     // A real handler hands the DTO to its use case, and `?` answers the module's error.
//!     let goal = Goal { display_name: goal.display_name };
//!     Ok(ApiSuccess::created("Goal created", goal))
//! }
//!
//! let routes: ApiRouter = ApiRouter::new().route(
//!     routes!(create_goal),
//!     Access::public(),
//!     &[ApiError::VALIDATION_FAILED],
//! );
//! # use barras_adapter_http::{HttpApp, Metrics, ServiceInfo, metadata_problems};
//! # let metrics = Metrics::new(ServiceInfo { name: "barras-api", version: "0.1.0" }).unwrap();
//! # let app = HttpApp::builder(metrics).routes(routes).build().unwrap();
//! # assert_eq!(metadata_problems(app.openapi()), Vec::<String>::new());
//! ```
//!
//! The app takes the router with [`HttpAppBuilder::routes`], in `build_app` of `barras-api`.
//! Then run `just openapi` and commit `openapi.json`: `just openapi-check` fails until then.

// Taken for its features, not code; named so that cargo never reports it as an unused dependency.
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
pub use envelope::{ApiResult, ApiSuccess, ErrorEnvelope, NoData, SuccessEnvelope};
pub use error::{ApiError, FieldErrors};
pub use extract::{ValidatedJson, ValidatedQuery};
pub use health::{Criticality, HealthProbe};
pub use internal::InternalApp;
pub use metrics::{Metrics, MetricsError};
// What the test client reads; the metrics layer records it on every build.
#[cfg(any(test, feature = "testing"))]
pub use middleware::MatchedRoute;
pub use openapi::{COMMON_ERROR_CODES, canonical_json, metadata_problems};
pub use router::{Access, ApiRouter};
pub use server::{Listener, ServeError, bind, serve_api, serve_internal};
