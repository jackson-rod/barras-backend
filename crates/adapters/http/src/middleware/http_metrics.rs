//! The HTTP metrics of spec §6.6, recorded as each answer leaves, with the route that served
//! it. The labels take few values: the method from a fixed set, the route template rather
//! than the path, and the status.

use axum::extract::{MatchedPath, Request, State};
use axum::http::Method;
use axum::middleware::Next;
use axum::response::Response;
use tokio::time::Instant;

use super::matched_route::MatchedRoute;
use crate::metrics::Metrics;

/// The `path` of a request no route matched, the 404 of the fallback: the path itself would
/// make a label value of anything a caller types.
const UNMATCHED: &str = "unmatched";

/// The `method` of a request whose method is not one of HTTP's standard nine.
const OTHER_METHOD: &str = "other";

pub(super) async fn record_http_metrics(
    State(metrics): State<Metrics>,
    request: Request,
    next: Next,
) -> Response {
    let method = method_label(request.method());
    // Read from the request: a 405's path matched a route, which its response does not carry.
    let served = MatchedRoute::of(&request);
    // tokio's clock, which reads the system's unless a test paused it to time a handler.
    let started = Instant::now();
    let mut response = next.run(request).await;
    // Up to the head of the answer: a body that streams is not waited for.
    let elapsed = started.elapsed();
    let path = served
        .as_ref()
        .map_or(UNMATCHED, |route| route.template.as_str());
    metrics.record_request(method, path, response.status(), elapsed);
    MatchedRoute::record(served, &mut response);
    response
}

/// The method as the metrics and the request log name it: a caller may send any token as a
/// method, so every method outside the standard set is one value.
pub(super) fn method_label(method: &Method) -> &'static str {
    match *method {
        Method::GET => "GET",
        Method::HEAD => "HEAD",
        Method::POST => "POST",
        Method::PUT => "PUT",
        Method::PATCH => "PATCH",
        Method::DELETE => "DELETE",
        Method::OPTIONS => "OPTIONS",
        Method::TRACE => "TRACE",
        Method::CONNECT => "CONNECT",
        _ => OTHER_METHOD,
    }
}

/// The route template `request` matched, as the metrics and the request log name it, or
/// `unmatched`.
pub(super) fn path_label(request: &Request) -> &str {
    request
        .extensions()
        .get::<MatchedPath>()
        .map_or(UNMATCHED, MatchedPath::as_str)
}
