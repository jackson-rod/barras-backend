//! The answers no handler gives: no route for the path, no route for the method, and a
//! handler that panicked.

use std::any::Any;

use axum::response::{IntoResponse, Response};

use crate::error::ApiError;

/// Marks the answer of a fallback, so that no route is recorded as having served it.
#[derive(Debug, Clone, Copy)]
pub(crate) struct ServedByFallback;

/// 404 `ROUTE_NOT_FOUND`: no route matches the path.
pub(crate) async fn route_not_found() -> Response {
    fallback_answer(ApiError::route_not_found())
}

/// 405 `METHOD_NOT_ALLOWED`: the path has routes, none for the method. axum adds the `allow`
/// header to this answer as it leaves the route.
pub(crate) async fn method_not_allowed() -> Response {
    fallback_answer(ApiError::method_not_allowed())
}

fn fallback_answer(error: ApiError) -> Response {
    let mut response = error.into_response();
    response.extensions_mut().insert(ServedByFallback);
    response
}

/// 500 `INTERNAL_ERROR`. The panic's text goes to the log with the answer, never into it.
pub(crate) fn handle_panic(payload: Box<dyn Any + Send + 'static>) -> Response {
    let text = payload
        .downcast_ref::<String>()
        .map(String::as_str)
        .or_else(|| payload.downcast_ref::<&str>().copied())
        .unwrap_or("a payload that is not text");
    ApiError::internal(format!("a handler panicked: {text}")).into_response()
}
