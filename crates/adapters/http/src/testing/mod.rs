//! Test support for the crates that build an app: a client that sends requests to it
//! in-process.
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)] // test support: a step that fails must stop the test that took it

mod test_client;

pub use test_client::{TestClient, TestRequest, TestResponse};
