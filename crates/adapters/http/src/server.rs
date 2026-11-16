//! Serving the apps on TCP listeners until the shutdown future resolves, then answering the
//! requests in flight before returning (spec §8.3; appendix A.6 of the plan).

use std::error::Error;
use std::fmt;
use std::future::Future;
use std::io;
use std::net::SocketAddr;

use axum::Router;
use tokio::net::TcpListener;
use tokio::sync::watch;

use crate::app::HttpApp;
use crate::internal::InternalApp;

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
/// is answered. It binds nothing: the caller binds both listeners first, so that a port
/// already taken stops the binary before either serves.
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
/// returns once every request in flight is answered: the internal port of a binary that has
/// no public one.
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
    // axum's future answers `Ok` once `shutdown` resolved and every connection is closed.
    axum::serve(tcp, router)
        .with_graceful_shutdown(shutdown)
        .await
        .map_err(|source| ServeError::new(format!("the {name} listener stopped serving"), source))
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
