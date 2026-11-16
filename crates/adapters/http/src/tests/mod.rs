//! The tests of the whole app: requests sent in-process through `TestClient` to a fixture app
//! built with `ApiRouter`, as the real app is.
//!
//! They live inside the crate rather than under `tests/` because they need the `testing`
//! module. An integration test would get it only through a dev dependency of this crate on
//! itself, which `check-arch` refuses as an adapter depending on an adapter.

mod http_metrics;
mod request_log;
mod route_guard;
mod stack;
mod support;
mod validation;
