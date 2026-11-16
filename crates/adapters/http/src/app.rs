//! The public application: the documented routes under `/api/v1`, wrapped in the middleware
//! every answer goes through, and the OpenAPI document built from the same routes.

use axum::Router;
#[cfg(any(test, feature = "testing"))]
use axum::routing::MethodRouter;
use utoipa::openapi::OpenApi;
use utoipa_axum::router::OpenApiRouter;

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

/// The public API, ready to serve, with its OpenAPI document.
pub struct HttpApp {
    service: ServiceInfo,
    router: Router,
    document: OpenApi,
}

impl HttpApp {
    pub fn builder(service: ServiceInfo) -> HttpAppBuilder {
        HttpAppBuilder {
            service,
            routes: ApiRouter::new(),
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

    #[allow(dead_code)] // the server reads it; until it exists, only the test client does
    pub(crate) fn router(&self) -> Router {
        self.router.clone()
    }
}

/// Gathers the routes of the app. Only [`ApiRouter`] adds one, so every route is documented.
pub struct HttpAppBuilder {
    service: ServiceInfo,
    routes: ApiRouter,
    #[cfg(any(test, feature = "testing"))]
    undocumented: Router,
}

impl HttpAppBuilder {
    /// Adds `routes`, served and documented under `/api/v1`.
    pub fn routes(mut self, routes: ApiRouter) -> Self {
        self.routes = self.routes.merge(routes);
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

    pub fn build(self) -> HttpApp {
        // Merging documents drops the top-level `info` and extensions, so the routes are nested
        // into the base rather than the base merged into them.
        let (router, document) = OpenApiRouter::with_openapi(document_base())
            .nest(API_PREFIX, self.routes.0)
            .split_for_parts();
        #[cfg(any(test, feature = "testing"))]
        let router = router.merge(self.undocumented);
        HttpApp {
            service: self.service,
            router: middleware::wrap(router),
            document,
        }
    }
}
