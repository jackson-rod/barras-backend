//! The request id (spec §6.3): the caller's when it is well formed, a new UUID v7 otherwise.

use axum::http::{HeaderName, HeaderValue, Request};
use tower_http::request_id::{MakeRequestId, RequestId};
use uuid::Uuid;

const X_REQUEST_ID: HeaderName = HeaderName::from_static("x-request-id");
const MAX_LENGTH: usize = 64;

/// Removes every `x-request-id` the caller sent unless there is exactly one and it is 1 to
/// 64 characters of `A-Z a-z 0-9 . _ -`: the layer that sets the id keeps whatever the header
/// holds, and an id goes into every log line of its request.
pub(crate) fn drop_invalid_request_id<B>(mut request: Request<B>) -> Request<B> {
    let mut sent = request.headers().get_all(X_REQUEST_ID).iter();
    let well_formed = match (sent.next(), sent.next()) {
        (Some(id), None) => is_well_formed(id),
        (None, _) => return request,
        (Some(_), Some(_)) => false,
    };
    if !well_formed {
        request.headers_mut().remove(X_REQUEST_ID);
    }
    request
}

fn is_well_formed(id: &HeaderValue) -> bool {
    let id = id.as_bytes();
    (1..=MAX_LENGTH).contains(&id.len())
        && id
            .iter()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'.' | b'_' | b'-'))
}

/// Makes the id of a request that came without a well-formed one: a UUID v7, as every id of
/// Barras is (ADR-0008).
#[derive(Debug, Clone, Copy, Default)]
pub(crate) struct MakeRequestUuidV7;

impl MakeRequestId for MakeRequestUuidV7 {
    fn make_request_id<B>(&mut self, _request: &Request<B>) -> Option<RequestId> {
        let id = Uuid::now_v7().hyphenated().to_string();
        HeaderValue::try_from(id).ok().map(RequestId::new)
    }
}
