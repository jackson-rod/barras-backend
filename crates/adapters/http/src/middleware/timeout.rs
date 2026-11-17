//! The time limit of a request (spec §6.4): an answer that has not started five seconds after the
//! request arrived is replaced, and the handler is dropped. The replacement is a 408
//! `REQUEST_BODY_TIMEOUT` when the handler was then waiting for more of a body that the client
//! had not finished sending, a 503 `REQUEST_TIMEOUT` otherwise.

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
    let (request, waiting_for_client) = watch_body(request);
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
    timed_out(&waiting_for_client, drop_panic.as_deref().map(panic_text))
}

/// The answer to a request that has had none within the limit, chosen here alone so that what
/// a timeout answers changes in one place.
fn timed_out(waiting_for_client: &WaitingForClient, drop_panic: Option<&str>) -> Response {
    // The client is the slow one only when the handler was waiting for more of a body that the
    // client had not finished sending. Any other request is the server's slowness: one with no
    // body, one whose body was read to its end, and one whose handler never read its body or
    // stopped reading it, such as a slow extractor that runs before the body is read, or a
    // handler that reads a chunk and then stalls. That handler was not waiting for the client,
    // so a client that sent faster would not have had its answer any sooner: the time is the
    // server's, and the 503 logs it as a failure, where a 408 would log nothing and blame the
    // client.
    if waiting_for_client.get() {
        // A 4xx, which logs no failure, so a drop panic shows only in the panic hook's event.
        //
        // Dropping the handler drops the body unfinished, which already closes the connection:
        // hyper 1.12 drains what has come in of the body and, when its end is not there, stops
        // reading and closes the connection once this answer is written
        // (`poll_drain_or_close_read`). `connection: close` tells the client so, as RFC 9110
        // §15.5.9 advises, and also closes the connection when the rest of the body had come in
        // but was never read: hyper drains that and would keep the connection open.
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

/// Whether the handler is waiting for the client to send more of the body: set by the body the
/// handler reads, at each poll, and read by this layer when the limit passes.
#[derive(Clone, Default)]
struct WaitingForClient(Arc<AtomicBool>);

impl WaitingForClient {
    fn get(&self) -> bool {
        self.0.load(Ordering::Acquire)
    }

    fn set(&self, waiting: bool) {
        self.0.store(waiting, Ordering::Release);
    }
}

/// `request` with its body watched, and what tells whether its handler is waiting for the
/// client. A body never polled is not waited for.
fn watch_body(request: Request) -> (Request, WaitingForClient) {
    let (parts, body) = request.into_parts();
    let waiting_for_client = WaitingForClient::default();
    let watched = WatchedBody {
        inner: body,
        waiting_for_client: waiting_for_client.clone(),
    };
    let request = Request::from_parts(parts, Body::new(watched));
    (request, waiting_for_client)
}

/// A request's body as the handler reads it: it says what the body says of its length and its
/// end, and records at each poll whether the handler is left waiting for the client.
struct WatchedBody {
    inner: Body,
    waiting_for_client: WaitingForClient,
}

impl HttpBody for WatchedBody {
    type Data = Bytes;
    type Error = axum::Error;

    fn poll_frame(
        mut self: Pin<&mut Self>,
        cx: &mut Context<'_>,
    ) -> Poll<Option<Result<Frame<Bytes>, axum::Error>>> {
        let polled = Pin::new(&mut self.inner).poll_frame(cx);
        // Each poll overwrites the last: a handler that got a frame and polls no more, whether it
        // stalls or wants no more, is not waiting for the client. A poll that found nothing waits
        // for the client only before the body's end, since a body of known length says that its
        // stream has ended once its last byte is read, and may still answer `Pending` before its
        // `None`.
        let waiting = polled.is_pending() && !self.inner.is_end_stream();
        self.waiting_for_client.set(waiting);
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
    use std::convert::Infallible;
    use std::task::Waker;

    use tokio::sync::mpsc;

    use super::*;

    /// `body` as the handler reads it, and what tells whether the handler waits for the client.
    fn watched(body: Body) -> (Body, WaitingForClient) {
        let (request, waiting_for_client) = watch_body(Request::new(body));
        (request.into_body(), waiting_for_client)
    }

    /// One poll of `body`, as the handler makes it, with a waker that does nothing: each test
    /// polls again itself.
    fn poll(body: &mut Body) -> Poll<Option<Result<Frame<Bytes>, axum::Error>>> {
        Pin::new(body).poll_frame(&mut Context::from_waker(Waker::noop()))
    }

    /// A body that the test sends a chunk at a time, as a client sends one at its own pace: a
    /// poll finds nothing until the test sends a chunk, and the body ends once the test drops
    /// its sender.
    struct Sent(mpsc::UnboundedReceiver<Bytes>);

    impl HttpBody for Sent {
        type Data = Bytes;
        type Error = Infallible;

        fn poll_frame(
            mut self: Pin<&mut Self>,
            cx: &mut Context<'_>,
        ) -> Poll<Option<Result<Frame<Bytes>, Infallible>>> {
            let chunk = self.0.poll_recv(cx);
            chunk.map(|chunk| chunk.map(|bytes| Ok(Frame::data(bytes))))
        }
    }

    /// A body that says its stream has ended and still answers `Pending`, as one of known length
    /// may between its last byte and its `None`.
    struct EndedButOpen;

    impl HttpBody for EndedButOpen {
        type Data = Bytes;
        type Error = Infallible;

        fn poll_frame(
            self: Pin<&mut Self>,
            _cx: &mut Context<'_>,
        ) -> Poll<Option<Result<Frame<Bytes>, Infallible>>> {
            Poll::Pending
        }

        fn is_end_stream(&self) -> bool {
            true
        }
    }

    #[test]
    fn a_body_never_polled_is_not_waited_for() {
        let (_body, waiting_for_client) = watched(Body::from("12345"));

        assert!(!waiting_for_client.get());
    }

    #[test]
    fn the_last_poll_tells_whether_the_handler_waits_for_the_client() {
        let (sender, chunks) = mpsc::unbounded_channel();
        let (mut body, waiting_for_client) = watched(Body::new(Sent(chunks)));

        assert!(poll(&mut body).is_pending());
        assert!(waiting_for_client.get());
        sender.send(Bytes::from_static(b"12345")).unwrap();
        assert!(matches!(poll(&mut body), Poll::Ready(Some(Ok(_)))));
        assert!(!waiting_for_client.get());
        assert!(poll(&mut body).is_pending());
        assert!(waiting_for_client.get());
        drop(sender);
        assert!(matches!(poll(&mut body), Poll::Ready(None)));
        assert!(!waiting_for_client.get());
    }

    #[test]
    fn a_body_of_known_length_is_not_waited_for_once_its_last_frame_is_read() {
        let (mut body, waiting_for_client) = watched(Body::from("12345"));
        assert_eq!(body.size_hint().exact(), Some(5));
        assert!(!body.is_end_stream());

        assert!(matches!(poll(&mut body), Poll::Ready(Some(Ok(_)))));

        // Its length tells its end, so a reader may stop here, without asking for more.
        assert!(body.is_end_stream());
        assert!(!waiting_for_client.get());
    }

    #[test]
    fn a_body_that_says_it_has_ended_is_not_waited_for_even_while_it_answers_pending() {
        let (mut body, waiting_for_client) = watched(Body::new(EndedButOpen));

        assert!(poll(&mut body).is_pending());
        assert!(!waiting_for_client.get());
    }
}
