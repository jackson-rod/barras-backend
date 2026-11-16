//! Which route served a request, recorded on its response for the test client's route guard.

use axum::extract::{MatchedPath, Request};
use axum::http::Method;
use axum::middleware::Next;
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

pub(crate) async fn record_matched_route(request: Request, next: Next) -> Response {
    // `Router::layer` runs this inside each route, after routing, so the template is known.
    let served = request
        .extensions()
        .get::<MatchedPath>()
        .map(|template| MatchedRoute {
            method: request.method().clone(),
            template: template.as_str().to_owned(),
        });
    let mut response = next.run(request).await;
    let extensions = response.extensions_mut();
    // Only this layer says which route served a request, whatever a handler wrote.
    extensions.remove::<MatchedRoute>();
    if let Some(served) = served
        && extensions.get::<ServedByFallback>().is_none()
    {
        extensions.insert(served);
    }
    response
}
