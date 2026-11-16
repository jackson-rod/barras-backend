//! The middleware every public answer goes through, and the fallbacks it wraps.

mod fallback;
mod matched_route;
mod request_id;
mod safety_net;

use axum::Router;
use axum::extract::DefaultBodyLimit;
use axum::middleware::{from_fn, map_response};
use tower::ServiceBuilder;
use tower::util::MapRequestLayer;
use tower_http::catch_panic::CatchPanicLayer;
use tower_http::compression::CompressionLayer;
use tower_http::compression::predicate::{NotForContentType, Predicate, SizeAbove};
use tower_http::request_id::{PropagateRequestIdLayer, SetRequestIdLayer};

pub use matched_route::MatchedRoute;

/// The largest request body read, in bytes (spec §6.4).
const BODY_LIMIT: usize = 1024 * 1024;

/// The smallest response body compressed, in bytes (spec §6.4).
const COMPRESS_FROM: u64 = 1024;

/// `router` with its fallbacks, wrapped in the stack, outermost layer first.
pub(crate) fn wrap(router: Router) -> Router {
    router
        .fallback(fallback::route_not_found)
        // It patches only the routes registered before it, so it is the last routing call.
        .method_not_allowed_fallback(fallback::method_not_allowed)
        // `layer`, not `route_layer`, so that the answers of the fallbacks go through the stack
        // too. axum applies it inside each route, after routing, where the template is known.
        .layer(
            ServiceBuilder::new()
                // The next layer keeps whatever id the header holds.
                .layer(MapRequestLayer::new(request_id::drop_invalid_request_id))
                .layer(SetRequestIdLayer::x_request_id(
                    request_id::MakeRequestUuidV7,
                ))
                // Outside every layer that writes an answer of its own, so that each carries
                // the id: the panic handler's, the safety net's.
                .layer(PropagateRequestIdLayer::x_request_id())
                .layer(from_fn(matched_route::record_matched_route))
                .layer(map_response(safety_net::envelope_safety_net))
                // An `Accept-Encoding` that refuses every encoding, such as `identity;q=0`,
                // still runs the handler: this layer then changes only the status of its answer
                // to 406, and the safety net above replaces the body.
                .layer(CompressionLayer::new().compress_when(compress_when()))
                .layer(CatchPanicLayer::custom(fallback::handle_panic))
                // It only sets the limit: the extractor that reads the body answers 413.
                .layer(DefaultBodyLimit::max(BODY_LIMIT)),
        )
}

/// tower-http's three default exclusions, with bodies under 1 KiB sent as they are (spec
/// §6.4). A body of unknown length is compressed.
fn compress_when() -> impl Predicate {
    SizeAbove::new(COMPRESS_FROM)
        .and(NotForContentType::GRPC)
        .and(NotForContentType::IMAGES)
        .and(NotForContentType::SSE)
}
