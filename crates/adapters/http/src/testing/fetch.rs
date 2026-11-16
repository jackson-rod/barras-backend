//! A client for a real socket, without reqwest: what the tests of a served app, or of a running
//! binary, send their requests with (appendix A.6 of the plan).

use std::net::SocketAddr;

use axum::body::{Body, to_bytes};
use hyper_util::client::legacy::Client;
use hyper_util::rt::TokioExecutor;

use super::test_client::TestResponse;

/// `GET path` to the server at `addr`, its answer read whole. The test fails when the request
/// cannot be sent or the answer cannot be read.
pub async fn fetch(addr: SocketAddr, path: &str) -> TestResponse {
    let url = format!("http://{addr}{path}");
    let client = Client::builder(TokioExecutor::new()).build_http::<Body>();
    let response = client
        .get(url.parse().unwrap())
        .await
        .unwrap_or_else(|error| panic!("GET {url}: {error}"));
    let (parts, body) = response.into_parts();
    let body = to_bytes(Body::new(body), usize::MAX).await.unwrap();
    TestResponse::new(parts, body)
}
