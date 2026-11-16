//! Which route served a request, recorded on its response for the test client's route guard.

use axum::extract::{MatchedPath, Request};
use axum::http::Method;
use axum::response::Response;

use super::fallback::ServedByFallback;

/// The route that served a request: its method and its path template, spelled as the OpenAPI
/// document spells them (`GET`, `/api/v1/goals/{id}`). It is on the response of every request
/// a route served, an extractor's rejection included, and absent when a fallback answered: no
/// route matched the path (404) or the method (405).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MatchedRoute {
    pub method: Method,
    pub template: String,
}

impl MatchedRoute {
    /// The route whose path `request` matched, read before the request is handed on. Only a
    /// layer that `Router::layer` runs inside each route, after routing, knows the template.
    pub(super) fn of(request: &Request) -> Option<Self> {
        request
            .extensions()
            .get::<MatchedPath>()
            .map(|template| Self {
                method: request.method().clone(),
                template: template.as_str().to_owned(),
            })
    }

    /// Puts `served` on `response`, unless a fallback answered it. Only this says which route
    /// served a request, whatever a handler wrote.
    pub(super) fn record(served: Option<Self>, response: &mut Response) {
        let extensions = response.extensions_mut();
        extensions.remove::<Self>();
        if let Some(served) = served
            && extensions.get::<ServedByFallback>().is_none()
        {
            extensions.insert(served);
        }
    }
}
