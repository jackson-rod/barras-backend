//! The request log of spec §6.5: a span for each request, and one `info` event when it is
//! answered.

use std::time::Duration;

use axum::body::Body;
use axum::http::{Request, Response};
use tower_http::request_id::RequestId;
use tower_http::trace::{HttpMakeClassifier, TraceLayer};
use tracing::Span;

use super::http_metrics::{method_label, path_label};

type MakeSpan = fn(&Request<Body>) -> Span;
type OnResponse = fn(&Response<Body>, Duration, &Span);

/// The trace layer with our span and our one event, and none of tower-http's own: its span
/// records the whole URI, query string included; its failure event would log the cause of a
/// 5xx a second time, after `ApiError` did; and its other events would only repeat ours.
pub(super) fn request_log() -> TraceLayer<HttpMakeClassifier, MakeSpan, (), OnResponse, (), (), ()>
{
    TraceLayer::new_for_http()
        .make_span_with(request_span as MakeSpan)
        .on_request(())
        .on_response(request_finished as OnResponse)
        .on_body_chunk(())
        .on_eos(())
        .on_failure(())
}

/// Only what the server chose: the method as the metrics name it, the route template, and
/// the request id, which the layer above checked or made. Never a header, the query string or
/// the body: any of them may carry a credential.
fn request_span(request: &Request<Body>) -> Span {
    let request_id = request
        .extensions()
        .get::<RequestId>()
        .and_then(|id| id.header_value().to_str().ok());
    // At `error` level, so that no filter turns it off: under `LOG_LEVEL=warn` or `error` an
    // `info` span is disabled, and the `request failed` event of a failing request would lose
    // its method, its path and its request id with it. A layer that reads the level of spans,
    // such as an OpenTelemetry or a Sentry one, will see every request span as an error.
    tracing::error_span!(
        "request",
        method = method_label(request.method()),
        path = path_label(request),
        request_id,
    )
}

/// The one `info` event of an answered request. Its duration runs to the head of the answer,
/// as the metrics measure it: a body that streams is not waited for.
fn request_finished(response: &Response<Body>, latency: Duration, span: &Span) {
    tracing::info!(
        parent: span,
        status = response.status().as_u16(),
        duration_ms = latency.as_secs_f64() * 1000.0,
        "request finished"
    );
}
