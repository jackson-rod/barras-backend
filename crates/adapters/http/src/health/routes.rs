//! The two public health routes (spec §5). Neither answer names a dependency: what each probe
//! said is for the internal port and, on a 503, for the log.

use std::sync::Arc;

use axum::extract::State;
use utoipa_axum::routes;

use super::dto::HealthStatus;
use super::probe::{HealthProbe, ProbeReport, Readiness, check_all};
use crate::envelope::{ApiResult, ApiSuccess, ErrorEnvelope, SuccessEnvelope};
use crate::error::ApiError;
use crate::router::{Access, ApiRouter};

/// The probes readiness runs, shared by every request.
#[derive(Clone)]
struct Probes(Arc<[HealthProbe]>);

/// Whether the process is running.
///
/// No probe runs: the answer is `ok` for as long as the process can answer at all.
#[utoipa::path(
    get,
    path = "/health/live",
    tag = "health",
    responses(
        (status = 200, description = "The process is running", body = SuccessEnvelope<HealthStatus>),
    ),
)]
async fn liveness() -> ApiResult<HealthStatus> {
    Ok(ApiSuccess::ok("Service is alive", HealthStatus::OK))
}

/// Whether the service can take traffic.
///
/// Every probe of a dependency runs at once, each with two seconds to answer. The answer is
/// `ok` when every probe passes and `degraded` when only probes of optional dependencies fail.
#[utoipa::path(
    get,
    path = "/health",
    tag = "health",
    responses(
        (
            status = 200,
            description = "The service can take traffic: `ok`, or `degraded` while a dependency it can do without is down",
            body = SuccessEnvelope<HealthStatus>,
        ),
        (status = 503, description = "A dependency the service needs is down", body = ErrorEnvelope),
    ),
)]
async fn readiness(State(Probes(probes)): State<Probes>) -> ApiResult<HealthStatus> {
    let reports = check_all(&probes).await;
    match Readiness::of(&reports) {
        Readiness::Ok => Ok(ApiSuccess::ok("Service is ready", HealthStatus::OK)),
        Readiness::Degraded => {
            // The answer names no dependency, so the log is where an operator sees which.
            tracing::warn!(detail = %down(&reports, false), "service degraded");
            Ok(ApiSuccess::ok(
                "Service is ready but degraded",
                HealthStatus::DEGRADED,
            ))
        }
        Readiness::Unavailable => Err(ApiError::service_unavailable(down(&reports, true))),
    }
}

/// The log's account of a 503, given `critical`, or of a degraded service: each dependency of
/// that kind that is down, and why.
fn down(reports: &[ProbeReport], critical: bool) -> String {
    let kind = if critical { "critical" } else { "optional" };
    let failed: Vec<String> = reports
        .iter()
        .filter(|report| report.critical == critical)
        .filter_map(|report| Some(format!("{} ({})", report.name, report.error.as_deref()?)))
        .collect();
    format!("{kind} dependencies are down: {}", failed.join(", "))
}

/// `GET /health/live` and `GET /health`, the second answering from `probes`.
pub(crate) fn health_routes(probes: Vec<HealthProbe>) -> ApiRouter {
    ApiRouter::new()
        .route(routes!(liveness), Access::public(), &[])
        .route(
            routes!(readiness),
            Access::public(),
            &[ApiError::SERVICE_UNAVAILABLE],
        )
        .with_state(Probes(probes.into()))
}
