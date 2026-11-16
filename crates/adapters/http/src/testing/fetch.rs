//! A client for a real socket, without reqwest: what the tests of a served app, or of a running
//! binary, send their requests with (appendix A.6 of the plan).

use std::net::SocketAddr;
use std::time::Duration;

use axum::body::{Body, to_bytes};
use hyper_util::client::legacy::Client;
use hyper_util::rt::TokioExecutor;
use tokio::time::timeout;

use super::test_client::TestResponse;

/// How long `fetch` waits for a whole answer: far longer than any route takes, so that only a
/// server that does not answer reaches it.
const ANSWER_LIMIT: Duration = Duration::from_secs(30);

/// `GET path` to the server at `addr`, its answer read whole. The test fails when the request
/// cannot be sent, when the answer cannot be read, or when it is not whole within thirty
/// seconds, rather than wait for ever.
pub async fn fetch(addr: SocketAddr, path: &str) -> TestResponse {
    let url = format!("http://{addr}{path}");
    let exchange = async {
        let client = Client::builder(TokioExecutor::new()).build_http::<Body>();
        let response = client
            .get(url.parse().unwrap())
            .await
            .unwrap_or_else(|error| panic!("GET {url}: {error}"));
        let (parts, body) = response.into_parts();
        let body = to_bytes(Body::new(body), usize::MAX).await.unwrap();
        TestResponse::new(parts, body)
    };
    let limit = ANSWER_LIMIT.as_secs();
    timeout(ANSWER_LIMIT, exchange)
        .await
        .unwrap_or_else(|_| panic!("GET {url}: no whole answer within {limit} seconds"))
}

#[cfg(test)]
mod tests {
    use std::net::{Ipv4Addr, TcpListener};

    use super::*;

    // The paused clock moves on whenever the runtime has nothing else to do, so the wait for an
    // answer that never comes takes no time at all.
    #[tokio::test(start_paused = true)]
    #[should_panic(expected = "no whole answer within 30 seconds")]
    async fn a_server_that_never_answers_fails_the_test_instead_of_hanging_it() {
        // Bound, so the connection is made, and never accepted, so nothing ever answers.
        let silent = TcpListener::bind((Ipv4Addr::LOCALHOST, 0)).unwrap();

        fetch(silent.local_addr().unwrap(), "/health").await;
    }
}
