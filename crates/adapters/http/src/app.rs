//! The public application: the documented routes under `/api/v1`, wrapped in the middleware
//! every answer goes through, and the OpenAPI document built from the same routes; with it,
//! the internal app of the same binary.

use std::error::Error;
use std::fmt;

use axum::Router;
#[cfg(any(test, feature = "testing"))]
use axum::routing::MethodRouter;
use utoipa::openapi::OpenApi;
use utoipa_axum::router::OpenApiRouter;

use crate::health::{HealthProbe, health_routes};
use crate::internal::InternalApp;
use crate::metrics::Metrics;
use crate::middleware;
use crate::openapi::{canonical_json, document_base};
use crate::router::ApiRouter;

/// Every public route lives under it (ADR-0005).
const API_PREFIX: &str = "/api/v1";

/// The binary that serves the app, as it names itself to operators.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ServiceInfo {
    pub name: &'static str,
    pub version: &'static str,
}

/// The public API, ready to serve, with its OpenAPI document and the internal app.
pub struct HttpApp {
    service: ServiceInfo,
    router: Router,
    document: OpenApi,
    internal: InternalApp,
}

impl HttpApp {
    /// An app that records every answer it gives in `metrics`, and serves them on its internal
    /// port.
    pub fn builder(service: ServiceInfo, metrics: Metrics) -> HttpAppBuilder {
        HttpAppBuilder {
            service,
            metrics,
            routes: ApiRouter::new(),
            probes: Vec::new(),
            #[cfg(any(test, feature = "testing"))]
            undocumented: Router::new(),
        }
    }

    pub fn service(&self) -> ServiceInfo {
        self.service
    }

    /// The document of every route the app serves under `/api/v1`.
    pub fn openapi(&self) -> &OpenApi {
        &self.document
    }

    /// The document as it is committed: see [`canonical_json`].
    pub fn openapi_json(&self) -> Result<String, serde_json::Error> {
        canonical_json(&self.document)
    }

    /// The internal port's app: the detail of the same probes, and the metrics.
    pub fn internal(&self) -> &InternalApp {
        &self.internal
    }

    pub(crate) fn router(&self) -> Router {
        self.router.clone()
    }
}

/// Gathers the routes of the app. Only [`ApiRouter`] adds one, so every route is documented.
/// The two health routes are always there.
pub struct HttpAppBuilder {
    service: ServiceInfo,
    metrics: Metrics,
    routes: ApiRouter,
    probes: Vec<HealthProbe>,
    #[cfg(any(test, feature = "testing"))]
    undocumented: Router,
}

impl HttpAppBuilder {
    /// Adds `routes`, served and documented under `/api/v1`.
    pub fn routes(mut self, routes: ApiRouter) -> Self {
        self.routes = self.routes.merge(routes);
        self
    }

    /// Adds a dependency for readiness to check, on the public port and on the internal one.
    pub fn probe(mut self, probe: HealthProbe) -> Self {
        self.probes.push(probe);
        self
    }

    /// Serves `route` at `path` under `/api/v1`, with the middleware of every route, and
    /// leaves it out of the document: the bad case the test client's route guard exists for.
    #[cfg(any(test, feature = "testing"))]
    pub fn undocumented_route(mut self, path: &str, route: MethodRouter) -> Self {
        self.undocumented = self
            .undocumented
            .route(&format!("{API_PREFIX}{path}"), route);
        self
    }

    /// The app, or every problem `ApiRouter` recorded about a route the document cannot
    /// describe: an app that would serve such a route is not built.
    pub fn build(self) -> Result<HttpApp, BuildError> {
        let routes = health_routes(self.probes.clone()).merge(self.routes);
        if !routes.problems.is_empty() {
            return Err(BuildError {
                problems: routes.problems,
            });
        }
        // Merging documents drops the top-level `info` and extensions, so the routes are nested
        // into the base rather than the base merged into them.
        let (router, document) = OpenApiRouter::with_openapi(document_base())
            .nest(API_PREFIX, routes.router)
            .split_for_parts();
        #[cfg(any(test, feature = "testing"))]
        let router = router.merge(self.undocumented);
        let internal = InternalApp::new(self.service, self.metrics.clone(), self.probes);
        Ok(HttpApp {
            service: self.service,
            router: middleware::wrap(router, self.metrics),
            document,
            internal,
        })
    }
}

/// The routes of an app that the OpenAPI document cannot describe, one problem each.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BuildError {
    problems: Vec<String>,
}

impl BuildError {
    pub fn problems(&self) -> &[String] {
        &self.problems
    }
}

impl fmt::Display for BuildError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("the OpenAPI document cannot describe every route:")?;
        for problem in &self.problems {
            write!(f, "\n{problem}")?;
        }
        Ok(())
    }
}

impl Error for BuildError {}
