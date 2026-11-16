//! The internal port (spec §5): what every probe said and the metrics, for operators and
//! Prometheus. No `/api/v1` prefix, no envelope, nothing in the OpenAPI document, and none of
//! the public middleware, so its requests are neither counted nor logged as public ones.

use std::sync::Arc;

use axum::Router;
use axum::extract::State;
use axum::http::header::CONTENT_TYPE;
use axum::http::{HeaderValue, StatusCode};
use axum::response::{IntoResponse, Response};
use axum::routing::get;
use serde::Serialize;

use crate::envelope::json_response;
use crate::error::with_sources;
use crate::health::{HealthProbe, ProbeReport, Readiness, check_all};
use crate::metrics::Metrics;

const PROMETHEUS_TEXT: HeaderValue = HeaderValue::from_static(prometheus::TEXT_FORMAT);

/// `GET /health` and `GET /metrics`, for the internal port of either binary.
#[derive(Clone)]
pub struct InternalApp {
    router: Router,
}

/// What the two routes answer from.
#[derive(Clone)]
struct Internal {
    metrics: Metrics,
    probes: Arc<[HealthProbe]>,
}

impl InternalApp {
    /// Answers `GET /health` from `probes`, under the name and the version of the service the
    /// metrics were made for, and `GET /metrics` from `metrics`: the registry the public app
    /// records into, when the binary has one.
    pub fn new(metrics: Metrics, probes: Vec<HealthProbe>) -> Self {
        let internal = Internal {
            metrics,
            probes: probes.into(),
        };
        let router = Router::new()
            .route("/health", get(health))
            .route("/metrics", get(metrics_text))
            .with_state(internal);
        Self { router }
    }

    pub(crate) fn router(&self) -> Router {
        self.router.clone()
    }
}

/// 200, or 503 when a critical dependency is down, with every probe's answer.
async fn health(State(internal): State<Internal>) -> Response {
    let reports = check_all(&internal.probes).await;
    let readiness = Readiness::of(&reports);
    let status = match readiness {
        Readiness::Unavailable => StatusCode::SERVICE_UNAVAILABLE,
        Readiness::Ok | Readiness::Degraded => StatusCode::OK,
    };
    let service = internal.metrics.service();
    let body = Health {
        status: readiness,
        service: service.name,
        version: service.version,
        dependencies: reports.iter().map(Dependency::of).collect(),
    };
    json_response(status, &body)
}

/// The Prometheus text, or 500 with no body when it cannot be written: the cause goes to the
/// log, with every source, since the step alone does not say what went wrong.
async fn metrics_text(State(internal): State<Internal>) -> Response {
    match internal.metrics.render() {
        Ok(text) => ([(CONTENT_TYPE, PROMETHEUS_TEXT)], text).into_response(),
        Err(error) => {
            tracing::error!(error = %with_sources(&error), "the metrics could not be served");
            StatusCode::INTERNAL_SERVER_ERROR.into_response()
        }
    }
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct Health<'a> {
    status: Readiness,
    service: &'static str,
    version: &'static str,
    dependencies: Vec<Dependency<'a>>,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct Dependency<'a> {
    name: &'static str,
    critical: bool,
    status: DependencyStatus,
    latency_ms: u64,
    #[serde(skip_serializing_if = "Option::is_none")]
    error: Option<&'a str>,
}

#[derive(Serialize)]
#[serde(rename_all = "lowercase")]
enum DependencyStatus {
    Up,
    Down,
}

impl<'a> Dependency<'a> {
    fn of(report: &'a ProbeReport) -> Self {
        let status = match report.error {
            Some(_) => DependencyStatus::Down,
            None => DependencyStatus::Up,
        };
        Self {
            name: report.name,
            critical: report.critical,
            status,
            // A probe answers within two seconds, so the milliseconds always fit.
            latency_ms: u64::try_from(report.latency.as_millis()).unwrap_or(u64::MAX),
            error: report.error.as_deref(),
        }
    }
}
