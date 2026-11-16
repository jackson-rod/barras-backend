//! The last word on what a library layer answered on its own: every 4xx or 5xx that is not
//! JSON, and every 406, becomes an error envelope (spec §6.1).

use axum::BoxError;
use axum::body::{Body, Bytes, HttpBody};
use axum::http::header::{
    CONTENT_ENCODING, CONTENT_LENGTH, CONTENT_RANGE, CONTENT_TYPE, ETAG, LAST_MODIFIED,
};
use axum::http::{HeaderMap, HeaderName, Response, StatusCode};
use axum::response::IntoResponse;

use crate::error::ApiError;

/// The headers that describe the body a layer sent, so none of them is true of the envelope
/// sent in its place: a client would read the envelope as compressed, as a range, or as the
/// version of a resource a cache holds.
const BODY_HEADERS: [HeaderName; 6] = [
    CONTENT_TYPE,
    CONTENT_LENGTH,
    CONTENT_ENCODING,
    CONTENT_RANGE,
    ETAG,
    LAST_MODIFIED,
];

/// Generic over the body, because it receives the body of the layer below it, not axum's.
pub(crate) async fn envelope_safety_net<B>(response: Response<B>) -> Response<Body>
where
    B: HttpBody<Data = Bytes> + Send + 'static,
    B::Error: Into<BoxError>,
{
    let status = response.status();
    let is_error = status.is_client_error() || status.is_server_error();
    // The compression layer makes a 406 of any answer by changing its status alone, so a 406
    // is replaced even when its body is an envelope.
    let replace =
        status == StatusCode::NOT_ACCEPTABLE || (is_error && !is_json(response.headers()));
    if !replace {
        return response.map(Body::new);
    }
    let (mut parts, _unsent) = response.into_parts();
    let (envelope, body) = replacement(status).into_response().into_parts();
    parts.status = envelope.status;
    for name in &BODY_HEADERS {
        parts.headers.remove(name);
    }
    for (name, value) in &envelope.headers {
        parts.headers.insert(name, value.clone());
    }
    Response::from_parts(parts, body)
}

fn replacement(status: StatusCode) -> ApiError {
    match status {
        StatusCode::PAYLOAD_TOO_LARGE => ApiError::payload_too_large(),
        StatusCode::UNSUPPORTED_MEDIA_TYPE => ApiError::unsupported_media_type(),
        StatusCode::NOT_ACCEPTABLE => ApiError::not_acceptable(),
        status => ApiError::unexplained(status),
    }
}

/// Every envelope is `application/json`, parameters aside.
fn is_json(headers: &HeaderMap) -> bool {
    let Some(Ok(content_type)) = headers.get(CONTENT_TYPE).map(|value| value.to_str()) else {
        return false;
    };
    let essence = content_type.split(';').next().unwrap_or_default().trim();
    essence.eq_ignore_ascii_case("application/json")
}

#[cfg(test)]
mod tests {
    use axum::http::HeaderValue;

    use super::*;

    #[test]
    fn only_application_json_counts_as_an_envelope() {
        let cases: [(Option<&[u8]>, bool); 7] = [
            (None, false),
            (Some(b"application/json"), true),
            (Some(b"application/json; charset=utf-8"), true),
            (Some(b"Application/JSON"), true),
            (Some(b"application/problem+json"), false),
            (Some(b"text/plain; charset=utf-8"), false),
            (Some(b"application/json\xff"), false),
        ];
        for (content_type, expected) in cases {
            let mut headers = HeaderMap::new();
            if let Some(content_type) = content_type {
                let value = HeaderValue::from_bytes(content_type).unwrap();
                headers.insert(CONTENT_TYPE, value);
            }

            assert_eq!(is_json(&headers), expected, "{content_type:?}");
        }
    }
}
