//! Serving on real sockets (spec §8.3): both listeners answer; once the shutdown future
//! resolves, no new connection is taken and the requests in flight are answered before the
//! server returns, for ten seconds at most; a port already taken is an error that names the
//! listener and the address.

use std::error::Error;
use std::io;
use std::net::SocketAddr;
use std::sync::Arc;
use std::time::Duration;

use serde_json::{Value, json};
use tokio::net::TcpStream;
use tokio::sync::{Notify, oneshot};
use tokio::task::JoinHandle;
use tokio::time::{Instant, timeout};
use tracing::Level;

use super::support::{fixture_metrics, logged_json};
use crate::server::DRAIN_LIMIT;
use crate::testing::fetch;
use crate::{
    ApiError, Criticality, HealthProbe, HttpApp, InternalApp, Listener, bind, serve_api,
    serve_internal,
};

/// A listener on a port the system picks, and the address it got.
async fn listener(name: &'static str) -> (Listener, SocketAddr) {
    let listener = bind(name, SocketAddr::from(([127, 0, 0, 1], 0)))
        .await
        .unwrap();
    let addr = listener.local_addr().unwrap();
    (listener, addr)
}

/// A shutdown future, and what resolves it.
fn shutdown() -> (
    oneshot::Sender<()>,
    impl Future<Output = ()> + Send + 'static,
) {
    let (stop, stopped) = oneshot::channel::<()>();
    (stop, async {
        let _ = stopped.await;
    })
}

/// Holds a request in flight: its probe says when the check has started, then answers only
/// once the test releases it.
#[derive(Default)]
struct Gate {
    entered: Arc<Notify>,
    release: Arc<Notify>,
}

impl Gate {
    fn probe(&self) -> HealthProbe {
        let (entered, release) = (Arc::clone(&self.entered), Arc::clone(&self.release));
        HealthProbe::new("gate", Criticality::Critical, move || {
            let (entered, release) = (Arc::clone(&entered), Arc::clone(&release));
            async move {
                entered.notify_one();
                release.notified().await;
                Ok(())
            }
        })
    }
}

/// What `task` returned, failing the test when it is still running five seconds on.
async fn finished<T>(task: JoinHandle<T>) -> T {
    let Ok(joined) = timeout(Duration::from_secs(5), task).await else {
        panic!("still running five seconds on");
    };
    joined.unwrap()
}

/// A connection to `addr` that sends the start of a request head and never the rest, as a slow
/// client does. hyper keeps the first request of a connection open until its head is whole, so
/// only the deadline ends it. On a paused clock each sleep first polls the sockets, so the
/// server has accepted the connection and read the bytes when this returns.
async fn hold_a_partial_head(addr: SocketAddr, path: &str) -> TcpStream {
    let stream = TcpStream::connect(addr).await.unwrap();
    let head = format!("GET {path} HTTP/1.1\r\nHost: barras\r\n");
    stream.writable().await.unwrap();
    assert_eq!(stream.try_write(head.as_bytes()).unwrap(), head.len());
    for _ in 0..10 {
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
    stream
}

/// What `server` returned once `stop` resolved its shutdown, how long that took, and the
/// warnings logged meanwhile.
async fn stopped_holding<E>(
    stop: oneshot::Sender<()>,
    server: JoinHandle<Result<(), E>>,
) -> (Result<(), E>, Duration, Vec<Value>) {
    let ((outcome, waited), lines) = logged_json(Level::WARN, async {
        stop.send(()).unwrap();
        let started = Instant::now();
        let Ok(joined) = timeout(Duration::from_secs(60), server).await else {
            panic!("still running a minute after the shutdown");
        };
        (joined.unwrap(), started.elapsed())
    })
    .await;
    (outcome, waited, lines)
}

/// Waits until `addr` refuses new connections, as a server does once its shutdown began.
async fn refused(addr: SocketAddr) {
    for _ in 0..100 {
        // An attempt that races the listener's close can wait a second for the system to
        // retry it; a new attempt is refused at once.
        let attempt = timeout(Duration::from_millis(50), TcpStream::connect(addr)).await;
        if let Ok(Err(error)) = attempt
            && error.kind() == io::ErrorKind::ConnectionRefused
        {
            return;
        }
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
    panic!("{addr} still takes connections six seconds after the shutdown");
}

#[tokio::test]
async fn serve_api_answers_on_both_listeners() {
    let app = HttpApp::builder(fixture_metrics()).build().unwrap();
    let (public, public_addr) = listener("public").await;
    let (internal, internal_addr) = listener("internal").await;
    let (stop, stopped) = shutdown();
    let server = tokio::spawn(serve_api(app, public, internal, stopped));

    let live = fetch(public_addr, "/api/v1/health/live").await;
    let ready = fetch(public_addr, "/api/v1/health").await;
    let not_public = fetch(public_addr, "/metrics").await;
    let health = fetch(internal_addr, "/health").await;
    let not_internal = fetch(internal_addr, "/api/v1/health").await;
    let metrics = fetch(internal_addr, "/metrics").await;

    assert_eq!(live.status, 200);
    assert_eq!(live.json()["data"], json!({ "status": "ok" }));
    assert!(live.header("x-request-id").is_some());
    assert_eq!(ready.status, 200);
    assert_eq!(not_public.status, 404);
    assert_eq!(
        not_public.error_code().as_deref(),
        Some(ApiError::ROUTE_NOT_FOUND)
    );
    assert_eq!(health.status, 200);
    assert_eq!(health.json()["service"], "barras-fixture");
    assert_eq!(not_internal.status, 404);
    assert_eq!(metrics.status, 200);
    // The public requests, counted in the registry the internal port renders; never its own.
    let text = String::from_utf8_lossy(metrics.bytes()).into_owned();
    let live_counted =
        r#"http_requests_total{method="GET",path="/api/v1/health/live",status="200"} 1"#;
    assert!(text.contains(live_counted), "{text}");
    assert!(!text.contains(r#"path="/metrics""#), "{text}");
    stop.send(()).unwrap();
    assert!(finished(server).await.is_ok());
}

#[tokio::test]
async fn after_the_shutdown_serve_api_answers_the_request_in_flight_and_returns_ok() {
    let gate = Gate::default();
    let app = HttpApp::builder(fixture_metrics())
        .probe(gate.probe())
        .build()
        .unwrap();
    let (public, public_addr) = listener("public").await;
    let (internal, internal_addr) = listener("internal").await;
    let (stop, stopped) = shutdown();
    let server = tokio::spawn(serve_api(app, public, internal, stopped));
    let in_flight = tokio::spawn(fetch(public_addr, "/api/v1/health"));
    gate.entered.notified().await;

    stop.send(()).unwrap();
    refused(public_addr).await;
    refused(internal_addr).await;
    assert!(!server.is_finished());
    gate.release.notify_one();

    let response = finished(in_flight).await;
    assert_eq!(response.status, 200);
    assert_eq!(response.json()["data"], json!({ "status": "ok" }));
    assert!(finished(server).await.is_ok());
}

#[tokio::test]
async fn after_the_shutdown_serve_internal_answers_the_request_in_flight_and_returns_ok() {
    let gate = Gate::default();
    let app = InternalApp::new(fixture_metrics(), vec![gate.probe()]);
    let (internal, addr) = listener("internal").await;
    let (stop, stopped) = shutdown();
    let server = tokio::spawn(serve_internal(app, internal, stopped));
    let metrics = fetch(addr, "/metrics").await;
    assert_eq!(metrics.status, 200);
    let in_flight = tokio::spawn(fetch(addr, "/health"));
    gate.entered.notified().await;

    stop.send(()).unwrap();
    refused(addr).await;
    assert!(!server.is_finished());
    gate.release.notify_one();

    let response = finished(in_flight).await;
    assert_eq!(response.status, 200);
    assert_eq!(response.json()["dependencies"][0]["status"], "up");
    assert!(finished(server).await.is_ok());
}

#[tokio::test(start_paused = true)]
async fn after_the_shutdown_serve_api_waits_ten_seconds_at_most_for_the_requests_in_flight() {
    let app = HttpApp::builder(fixture_metrics()).build().unwrap();
    let (public, public_addr) = listener("public").await;
    let (internal, _) = listener("internal").await;
    let (stop, stopped) = shutdown();
    let server = tokio::spawn(serve_api(app, public, internal, stopped));
    let _slow = hold_a_partial_head(public_addr, "/api/v1/health/live").await;

    let (outcome, waited, lines) = stopped_holding(stop, server).await;

    assert!(outcome.is_ok());
    assert_eq!(DRAIN_LIMIT, Duration::from_secs(10));
    assert_eq!(waited, DRAIN_LIMIT);
    assert_eq!(lines.len(), 1, "{lines:?}");
    let fields = &lines[0]["fields"];
    assert_eq!(lines[0]["level"], "WARN");
    assert_eq!(
        fields["message"],
        "stopped waiting for the requests in flight"
    );
    assert_eq!(fields["listener"], "public");
    assert_eq!(fields["waited_seconds"], 10);
}

#[tokio::test(start_paused = true)]
async fn after_the_shutdown_serve_internal_waits_ten_seconds_at_most_for_the_requests_in_flight() {
    let app = InternalApp::new(fixture_metrics(), vec![]);
    let (internal, addr) = listener("internal").await;
    let (stop, stopped) = shutdown();
    let server = tokio::spawn(serve_internal(app, internal, stopped));
    let _slow = hold_a_partial_head(addr, "/health").await;

    let (outcome, waited, lines) = stopped_holding(stop, server).await;

    assert!(outcome.is_ok());
    assert_eq!(waited, DRAIN_LIMIT);
    assert_eq!(lines.len(), 1, "{lines:?}");
    assert_eq!(lines[0]["fields"]["listener"], "internal");
    assert_eq!(lines[0]["fields"]["waited_seconds"], 10);
}

#[tokio::test]
async fn binding_a_port_already_taken_names_the_listener_and_the_address() {
    let (_taken, addr) = listener("public").await;

    let Err(error) = bind("internal", addr).await else {
        panic!("{addr} was bound twice");
    };

    assert_eq!(
        error.to_string(),
        format!("the internal listener could not bind {addr}")
    );
    let cause = error.source().unwrap().downcast_ref::<io::Error>().unwrap();
    assert_eq!(cause.kind(), io::ErrorKind::AddrInUse);
}
