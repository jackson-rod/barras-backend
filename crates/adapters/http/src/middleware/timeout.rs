//! The time limit of a request (spec §6.4): an answer that has not started five seconds after the
//! request arrived is replaced by a 503 `REQUEST_TIMEOUT`, and the handler is dropped.

use std::time::Duration;

use axum::extract::Request;
use axum::middleware::Next;
use axum::response::{IntoResponse, Response};
use tokio::time::timeout;

use crate::error::ApiError;

/// How long a request has from its arrival to the head of its answer, the reading of its body
/// included: far above every budget of ADR-0014, and under the ten seconds a shutdown waits
/// for the requests in flight, so that a request in flight at SIGTERM ends before that wait
/// does (spec §6.4).
pub(crate) const REQUEST_TIME_LIMIT: Duration = Duration::from_secs(5);

// An `axum::middleware::from_fn` with tokio's `timeout` rather than tower-http's
// `TimeoutLayer`: that layer answers its status with an empty body, and the safety net can only
// make an `INTERNAL_ERROR` envelope of a bare 503, which says nothing of its cause. Here the
// answer is the envelope with its own code, and `ApiError` logs it once, as it does every 5xx.
pub(super) async fn limit_request_time(request: Request, next: Next) -> Response {
    // The handler reads the body inside this future, so the reading counts. Only the head is
    // awaited: a body that streams out after it is not limited.
    match timeout(REQUEST_TIME_LIMIT, next.run(request)).await {
        Ok(response) => response,
        // The handler's future is dropped with the timer's, wherever it was waiting.
        Err(_elapsed) => ApiError::request_timeout(REQUEST_TIME_LIMIT).into_response(),
    }
}
