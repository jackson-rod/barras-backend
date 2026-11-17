//! The time limit of a request (spec §6.4): an answer that has not started five seconds after the
//! request arrived is replaced by a 503 `REQUEST_TIMEOUT`, and the handler is dropped.

use std::panic::{AssertUnwindSafe, catch_unwind};
use std::time::Duration;

use axum::extract::Request;
use axum::middleware::Next;
use axum::response::{IntoResponse, Response};
use tokio::time::timeout;

use super::fallback::panic_text;
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
    // awaited: a body that streams out after it is not limited. Boxed and awaited through a
    // reference, so that the future outlives the timer and this layer chooses where it is
    // dropped.
    let mut handler = Box::pin(next.run(request));
    if let Ok(response) = timeout(REQUEST_TIME_LIMIT, &mut handler).await {
        return response;
    }
    // The future is dropped wherever it was waiting. This layer is outside the panic handler, so
    // a value of the handler that panics as it is dropped would unwind through every layer
    // above it, and the client would get no answer: no request id, no log line, no count.
    let drop_panic = catch_unwind(AssertUnwindSafe(move || drop(handler))).err();
    timed_out(drop_panic.as_deref().map(panic_text))
}

/// The answer to a request that has had none within the limit, chosen here alone so that what
/// a timeout answers changes in one place. The detail names the limit and, as the panic
/// handler's names its panic, what the handler panicked with as it was dropped, when it did;
/// the request span it is logged in already names the route and the request id.
fn timed_out(drop_panic: Option<&str>) -> Response {
    let mut detail = format!("no answer within {} seconds", REQUEST_TIME_LIMIT.as_secs());
    if let Some(text) = drop_panic {
        detail.push_str(", and the handler panicked as it was dropped: ");
        detail.push_str(text);
    }
    ApiError::request_timeout(detail).into_response()
}
