//! Serving the apps on TCP listeners until the shutdown future resolves, then answering the
//! requests in flight before returning, for ten seconds at most (spec §8.3; appendix A.6 of the
//! plan).

use std::error::Error;
use std::fmt;
use std::future::{Future, IntoFuture};
use std::io;
use std::net::SocketAddr;
use std::pin::pin;
use std::time::Duration;

use axum::Router;
use tokio::net::TcpListener;
use tokio::sync::{oneshot, watch};
use tokio::time::timeout;

use crate::app::HttpApp;
use crate::internal::InternalApp;

/// How long the requests in flight have to finish once the shutdown future resolved (a Call:
/// spec §8.3 sets none). Without it, a client that never ends its request head would hold the
/// process open, since hyper waits for the head of a first request and axum sets no timer on
/// it; a header-read timeout against slow clients is left to the proxy in front (BAR-12).
pub(crate) const DRAIN_LIMIT: Duration = Duration::from_secs(10);

/// A bound TCP listener, with the name its errors give it: `public` or `internal`, say.
#[derive(Debug)]
pub struct Listener {
    name: &'static str,
    tcp: TcpListener,
}

impl Listener {
    /// The address it is bound to, with the port the system picked when it was given 0.
    pub fn local_addr(&self) -> Result<SocketAddr, ServeError> {
        let name = self.name;
        self.tcp.local_addr().map_err(|source| {
            ServeError::new(format!("the {name} listener has no local address"), source)
        })
    }
}

/// Binds `addr`; `name` says which listener it is, in the error.
pub async fn bind(name: &'static str, addr: SocketAddr) -> Result<Listener, ServeError> {
    match TcpListener::bind(addr).await {
        Ok(tcp) => Ok(Listener { name, tcp }),
        Err(source) => Err(ServeError::new(
            format!("the {name} listener could not bind {addr}"),
            source,
        )),
    }
}

/// Serves `app` on `public` and its internal app on `internal` until `shutdown` resolves.
/// Both then stop taking connections, and it returns once every request in flight on either
/// is answered, or once `DRAIN_LIMIT` has passed, with a warning: it still answers `Ok`, and
/// the connections left are dropped with the runtime. It binds nothing: the caller binds both
/// listeners first, so that a port already taken stops the binary before either serves.
pub async fn serve_api(
    app: HttpApp,
    public: Listener,
    internal: Listener,
    shutdown: impl Future<Output = ()> + Send + 'static,
) -> Result<(), ServeError> {
    // One shutdown future, two servers: a watch channel tells both.
    let (stop, stopping) = watch::channel(false);
    let signal = async move {
        shutdown.await;
        stop.send_replace(true);
        Ok(())
    };
    tokio::try_join!(
        serve(public, app.router(), stopped(stopping.clone())),
        serve(internal, app.internal().router(), stopped(stopping)),
        signal,
    )?;
    Ok(())
}

/// Serves `app` on `internal` until `shutdown` resolves, then stops taking connections and
/// returns once every request in flight is answered, or once `DRAIN_LIMIT` has passed, as
/// `serve_api` does: the internal port of a binary that has no public one.
pub async fn serve_internal(
    app: InternalApp,
    internal: Listener,
    shutdown: impl Future<Output = ()> + Send + 'static,
) -> Result<(), ServeError> {
    serve(internal, app.router(), shutdown).await
}

/// Resolves once `stopping` turns true, or once its sender is gone.
async fn stopped(mut stopping: watch::Receiver<bool>) {
    // An error says the sender is gone, which stops the server too.
    let _ = stopping.wait_for(|stop| *stop).await;
}

async fn serve(
    listener: Listener,
    router: Router,
    shutdown: impl Future<Output = ()> + Send + 'static,
) -> Result<(), ServeError> {
    let Listener { name, tcp } = listener;
    // axum takes the shutdown future whole; this one also says when it resolved, which is when
    // the deadline starts.
    let (began, beginning) = oneshot::channel();
    let shutdown = async move {
        shutdown.await;
        let _ = began.send(());
    };
    // axum's future answers `Ok` once `shutdown` resolved and every connection is closed.
    let mut serving = pin!(
        axum::serve(tcp, router)
            .with_graceful_shutdown(shutdown)
            .into_future()
    );
    let served = tokio::select! {
        served = &mut serving => served,
        _ = beginning => match timeout(DRAIN_LIMIT, &mut serving).await {
            Ok(served) => served,
            Err(_) => {
                tracing::warn!(
                    listener = name,
                    waited_seconds = DRAIN_LIMIT.as_secs(),
                    "stopped waiting for the requests in flight"
                );
                Ok(())
            }
        },
    };
    served.map_err(|source| ServeError::new(format!("the {name} listener stopped serving"), source))
}

/// A listener could not be bound or served on. The message names the listener and, for a
/// bind, the address; the cause is the [`source`](Error::source).
#[derive(Debug)]
pub struct ServeError {
    what: String,
    source: io::Error,
}

impl ServeError {
    fn new(what: String, source: io::Error) -> Self {
        Self { what, source }
    }
}

impl fmt::Display for ServeError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.what)
    }
}

impl Error for ServeError {
    fn source(&self) -> Option<&(dyn Error + 'static)> {
        Some(&self.source)
    }
}
