//! The time limit of a request (spec §6.4): an answer that has not started five seconds after the
//! request arrived is replaced, and the handler is dropped. The replacement is a 408
//! `REQUEST_BODY_TIMEOUT` when the request's body was still arriving, a 503 `REQUEST_TIMEOUT`
//! otherwise.

use std::panic::{AssertUnwindSafe, catch_unwind};
use std::pin::Pin;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::task::{Context, Poll};
use std::time::Duration;

use axum::body::{Body, Bytes, HttpBody};
use axum::extract::Request;
use axum::http::HeaderValue;
use axum::http::header::CONNECTION;
use axum::middleware::Next;
use axum::response::{IntoResponse, Response};
use http_body::{Frame, SizeHint};
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
    let (request, body_arrived) = watch_body(request);
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
    timed_out(&body_arrived, drop_panic.as_deref().map(panic_text))
}

/// The answer to a request that has had none within the limit, chosen here alone so that what
/// a timeout answers changes in one place.
fn timed_out(body_arrived: &BodyArrived, drop_panic: Option<&str>) -> Response {
    // A body has arrived once the handler has read it to its end, or when there was none. So a
    // handler that never reads a body it was sent counts as still waiting for it, and its
    // request gets the 408: every handler of ours reads its body, through `ValidatedJson`.
    if !body_arrived.get() {
        // The client's slowness, not the server's: a 4xx, which logs no failure, so a drop panic
        // shows only in the panic hook's event. `connection: close`, as RFC 9110 §15.5.9 advises,
        // so that the server stops waiting for the rest of the body.
        let mut response = ApiError::request_body_timeout().into_response();
        let close = HeaderValue::from_static("close");
        response.headers_mut().insert(CONNECTION, close);
        return response;
    }
    // The detail names the limit and, as the panic handler's names its panic, what the handler
    // panicked with as it was dropped, when it did; the request span it is logged in already
    // names the route and the request id.
    let mut detail = format!("no answer within {} seconds", REQUEST_TIME_LIMIT.as_secs());
    if let Some(text) = drop_panic {
        detail.push_str(", and the handler panicked as it was dropped: ");
        detail.push_str(text);
    }
    ApiError::request_timeout(detail).into_response()
}

/// Whether a request's body has wholly arrived: set by the body the handler reads, as it reaches
/// its end, and read by this layer when the limit passes.
#[derive(Clone)]
struct BodyArrived(Arc<AtomicBool>);

impl BodyArrived {
    fn get(&self) -> bool {
        self.0.load(Ordering::Acquire)
    }

    fn set(&self) {
        self.0.store(true, Ordering::Release);
    }
}

/// `request` with its body watched, and what tells whether that body has arrived. A body that
/// is empty from the start has.
fn watch_body(request: Request) -> (Request, BodyArrived) {
    let (parts, body) = request.into_parts();
    let body_arrived = BodyArrived(Arc::new(AtomicBool::new(body.is_end_stream())));
    let watched = WatchedBody {
        inner: body,
        arrived: body_arrived.clone(),
    };
    (Request::from_parts(parts, Body::new(watched)), body_arrived)
}

/// A request's body as the handler reads it: it says what the body says of its length and its
/// end, and records in `arrived` when that end comes.
struct WatchedBody {
    inner: Body,
    arrived: BodyArrived,
}

impl HttpBody for WatchedBody {
    type Data = Bytes;
    type Error = axum::Error;

    fn poll_frame(
        mut self: Pin<&mut Self>,
        cx: &mut Context<'_>,
    ) -> Poll<Option<Result<Frame<Bytes>, axum::Error>>> {
        let polled = Pin::new(&mut self.inner).poll_frame(cx);
        // A body tells its end either way: by giving no more frames, or by saying, from its last
        // frame on, that its stream has ended, after which a reader may stop polling it. The
        // body changes only when it is polled, here, so `is_end_stream` need record nothing.
        if matches!(polled, Poll::Ready(None)) || self.inner.is_end_stream() {
            self.arrived.set();
        }
        polled
    }

    fn is_end_stream(&self) -> bool {
        self.inner.is_end_stream()
    }

    fn size_hint(&self) -> SizeHint {
        self.inner.size_hint()
    }
}

#[cfg(test)]
mod tests {
    use http_body_util::BodyExt;

    use super::*;

    /// `body` as the handler reads it, and what tells whether it has arrived.
    fn watched(body: Body) -> (Body, BodyArrived) {
        let (request, body_arrived) = watch_body(Request::new(body));
        (request.into_body(), body_arrived)
    }

    #[test]
    fn a_body_empty_from_the_start_has_arrived() {
        let (body, body_arrived) = watched(Body::empty());

        assert!(body.is_end_stream());
        assert!(body_arrived.get());
    }

    #[tokio::test]
    async fn a_body_of_known_length_has_arrived_with_its_last_frame() {
        let (mut body, body_arrived) = watched(Body::from("12345"));
        assert_eq!(body.size_hint().exact(), Some(5));
        assert!(!body.is_end_stream());
        assert!(!body_arrived.get());

        let frame = body.frame().await.unwrap().unwrap();

        // Its length tells its end, so a reader may stop here, without asking for more.
        assert_eq!(frame.into_data().unwrap(), "12345");
        assert!(body.is_end_stream());
        assert!(body_arrived.get());
    }

    #[tokio::test]
    async fn a_stream_has_arrived_only_once_it_gives_no_more() {
        let stream = Body::from("12345").into_data_stream();
        let (mut body, body_arrived) = watched(Body::from_stream(stream));

        body.frame().await.unwrap().unwrap();
        assert!(!body_arrived.get());
        let end = body.frame().await;

        assert!(end.is_none());
        assert!(body_arrived.get());
    }
}
