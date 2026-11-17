//! `barras-api`: the public API under `/api/v1` and its internal port, in one process, and the
//! commands that migrate, seed and rebuild its database. This is the composition root; `main.rs`
//! only hands it the command line, so the tests reach the same code as the binary.

// Taken for its features, not code; named so that cargo never reports it as an unused dependency.
use workspace_hack as _;

pub mod cli;
mod config;
mod database_commands;

use std::error::Error;
use std::fmt;
use std::io;
use std::net::SocketAddr;

use barras_adapter_http::{
    BuildError, Criticality, HealthProbe, HttpApp, HttpAppBuilder, Metrics, MetricsError,
    ServeError, ServiceInfo, bind, serve_api,
};
use barras_adapter_postgres::Pools;
use barras_bootstrap::{LoggingError, ShutdownSignal, init_logging, install_panic_hook};

pub use config::{ApiConfig, DatabaseUrlValue, LocalDatabaseConfig, MigrateConfig};

/// The binary as it names itself in its metrics, on its internal port and to Postgres.
const SERVICE: ServiceInfo = ServiceInfo {
    name: "barras-api",
    version: env!("CARGO_PKG_VERSION"),
};

/// What the wiring takes from outside the app: the registry its metrics go to, and a probe for
/// each dependency it reports on, one per database pool when it serves.
pub struct Dependencies {
    pub metrics: Metrics,
    pub probes: Vec<HealthProbe>,
}

impl Dependencies {
    /// What the wiring needs when nothing is served, as for the OpenAPI document: metrics of
    /// its own, and no probe.
    pub fn offline() -> Result<Self, MetricsError> {
        Ok(Self {
            metrics: Metrics::new(SERVICE)?,
            probes: Vec::new(),
        })
    }
}

/// The one function that wires the application: the routes of the public API, the probes of
/// its dependencies, and the internal app, which shares the metrics and the probes.
pub fn build_app(dependencies: Dependencies) -> Result<HttpApp, BuildError> {
    let Dependencies { metrics, probes } = dependencies;
    probes
        .into_iter()
        .fold(HttpApp::builder(metrics), HttpAppBuilder::probe)
        .build()
}

/// The OpenAPI document of the app, as `openapi.json` holds it. It reads no configuration and
/// opens no socket.
pub fn openapi_json() -> Result<String, RunError> {
    let dependencies = Dependencies::offline().map_err(RunError::Metrics)?;
    let app = build_app(dependencies).map_err(RunError::App)?;
    app.openapi_json().map_err(RunError::Document)
}

/// Starts the logs, then serves the public API and the internal port until SIGTERM or Ctrl-C,
/// and returns once the requests in flight are answered and the database pools are closed.
pub async fn run(config: ApiConfig) -> Result<(), RunError> {
    init_logging(config.env, config.log_level).map_err(RunError::Logs)?;
    install_panic_hook();
    let metrics = Metrics::new(SERVICE).map_err(RunError::Metrics)?;
    // Inside the runtime, since each pool starts a task on it, and without connecting: a database
    // that is down at start leaves the service unready, not stopped (spec §6.1).
    let pools = Pools::new(
        SERVICE.name,
        config.database_url.expose().get(),
        config.database_read_url.expose().get(),
        config.database_pool_size,
    );
    let dependencies = Dependencies {
        metrics,
        probes: database_probes(&pools),
    };
    let served = serve(dependencies, config.bind_addr, config.internal_bind_addr).await;
    // Whatever ended the serving, so that Postgres sees each connection end rather than the
    // process vanish from under it.
    pools.close().await;
    served?;
    tracing::info!("stopped");
    Ok(())
}

/// Serves the app that `dependencies` wire, on the two addresses, until SIGTERM or Ctrl-C, and
/// returns once the requests in flight are answered.
async fn serve(
    dependencies: Dependencies,
    bind_addr: SocketAddr,
    internal_bind_addr: SocketAddr,
) -> Result<(), RunError> {
    let app = build_app(dependencies).map_err(RunError::App)?;
    // Before anything is bound, so that failing to listen for the signals stops the binary at
    // start rather than once it serves.
    let mut signal = ShutdownSignal::install().map_err(RunError::Signal)?;
    let public = bind("public", bind_addr).await.map_err(RunError::Serve)?;
    let internal = bind("internal", internal_bind_addr)
        .await
        .map_err(RunError::Serve)?;
    let public_addr = public.local_addr().map_err(RunError::Serve)?;
    let internal_addr = internal.local_addr().map_err(RunError::Serve)?;
    tracing::info!(%public_addr, %internal_addr, "listening");
    let shutdown = async move {
        signal.recv().await;
        tracing::info!("shutting down");
    };
    serve_api(app, public, internal, shutdown)
        .await
        .map_err(RunError::Serve)
}

/// One probe per pool (spec §6.3): `postgres` for the primary and `postgres-read` for the read
/// pool, both critical, since the app's screens read from both; only the first when the primary
/// pool serves the reads too. Each answers with the adapter's sentence, which holds no
/// credential.
fn database_probes(pools: &Pools) -> Vec<HealthProbe> {
    let primary = pools.clone();
    let primary = HealthProbe::new("postgres", Criticality::Critical, move || {
        let pools = primary.clone();
        async move {
            pools
                .check_primary()
                .await
                .map_err(|cause| cause.to_string())
        }
    });
    let read = pools.clone();
    let read = HealthProbe::new("postgres-read", Criticality::Critical, move || {
        let pools = read.clone();
        async move { pools.check_read().await.map_err(|cause| cause.to_string()) }
    });
    if pools.read_is_primary() {
        vec![primary]
    } else {
        vec![primary, read]
    }
}

/// Why the binary stopped on a failure. Its text says what failed, and its sources say why.
#[derive(Debug)]
pub enum RunError {
    /// The logs could not start, so the failure is in no log.
    Logs(LoggingError),
    Metrics(MetricsError),
    /// A route the OpenAPI document cannot describe.
    App(BuildError),
    /// The OpenAPI document could not be written as JSON.
    Document(serde_json::Error),
    /// SIGTERM and SIGINT could not be listened for.
    Signal(io::Error),
    /// A listener could not be bound, or stopped serving.
    Serve(ServeError),
}

impl fmt::Display for RunError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Logs(error) => error.fmt(f),
            Self::Metrics(error) => error.fmt(f),
            Self::App(error) => error.fmt(f),
            Self::Document(_) => f.write_str("the OpenAPI document could not be written as JSON"),
            Self::Signal(_) => f.write_str("SIGTERM and SIGINT could not be listened for"),
            Self::Serve(error) => error.fmt(f),
        }
    }
}

/// A failure whose own error names the step is that error, its sources included; the others
/// name the step and keep the error as their source.
impl Error for RunError {
    fn source(&self) -> Option<&(dyn Error + 'static)> {
        match self {
            Self::Logs(error) => error.source(),
            Self::Metrics(error) => error.source(),
            Self::App(error) => error.source(),
            Self::Document(error) => Some(error),
            Self::Signal(error) => Some(error),
            Self::Serve(error) => error.source(),
        }
    }
}

#[cfg(test)]
mod tests {
    use std::num::NonZeroU32;

    use barras_adapter_http::testing::TestClient;
    use barras_adapter_postgres::DatabaseUrl;
    use serde_json::Value;

    use super::*;

    /// Databases nothing connects to: the pools are closed before any probe runs.
    const PRIMARY: &str = "postgres://barras:hunter2-secret@127.0.0.1:5999/barras";
    const REPLICA: &str = "postgres://barras:hunter2-secret@replica.example:5432/barras";

    /// What the internal port says of each dependency of the app whose probes are those of the
    /// pools to `primary` and `read`. The pools are closed first, so that every check fails at
    /// once, without a connection.
    async fn dependencies_reported(primary: &str, read: &str) -> Vec<Value> {
        let primary = DatabaseUrl::parse(primary).unwrap();
        let read = DatabaseUrl::parse(read).unwrap();
        let pools = Pools::new(SERVICE.name, &primary, &read, NonZeroU32::MIN);
        pools.close().await;
        let dependencies = Dependencies {
            probes: database_probes(&pools),
            ..Dependencies::offline().unwrap()
        };
        let app = build_app(dependencies).unwrap();

        let health = TestClient::internal(app.internal()).get("/health").await;

        assert_eq!(health.status, 503);
        let body = String::from_utf8_lossy(health.bytes());
        assert!(!body.contains("hunter2"), "{body}");
        health.json()["dependencies"].as_array().unwrap().clone()
    }

    fn names(dependencies: &[Value]) -> Vec<&str> {
        dependencies
            .iter()
            .map(|dependency| dependency["name"].as_str().unwrap())
            .collect()
    }

    #[tokio::test]
    async fn equal_urls_make_one_critical_probe_named_postgres() {
        let reported = dependencies_reported(PRIMARY, PRIMARY).await;

        assert_eq!(names(&reported), ["postgres"]);
        assert_eq!(reported[0]["critical"], true);
        assert_eq!(reported[0]["error"], "the pool is closed");
    }

    #[tokio::test]
    async fn different_urls_make_a_critical_probe_for_each_pool() {
        let reported = dependencies_reported(PRIMARY, REPLICA).await;

        assert_eq!(names(&reported), ["postgres", "postgres-read"]);
        for dependency in &reported {
            assert_eq!(dependency["critical"], true, "{dependency}");
            // The adapter's sentence, as the check gives it.
            assert_eq!(dependency["error"], "the pool is closed", "{dependency}");
        }
    }

    #[test]
    fn a_failure_whose_cause_names_no_step_names_it_and_keeps_the_cause() {
        let signal = RunError::Signal(io::Error::from(io::ErrorKind::PermissionDenied));
        let document = RunError::Document(serde_json::from_str::<u8>("x").unwrap_err());

        assert_eq!(
            signal.to_string(),
            "SIGTERM and SIGINT could not be listened for"
        );
        assert!(signal.source().unwrap().is::<io::Error>());
        assert_eq!(
            document.to_string(),
            "the OpenAPI document could not be written as JSON"
        );
        assert!(document.source().unwrap().is::<serde_json::Error>());
    }

    #[test]
    fn a_failure_whose_error_names_the_step_reads_as_that_error_sources_included() {
        use barras_bootstrap::{AppEnv, LogLevel};

        // Whichever call comes second fails: a process has one global subscriber.
        let _first = init_logging(AppEnv::Local, LogLevel::Error);
        let logs = init_logging(AppEnv::Local, LogLevel::Error).unwrap_err();
        let (text, cause) = (logs.to_string(), logs.source().unwrap().to_string());

        let failure = RunError::Logs(logs);

        assert_eq!(failure.to_string(), text);
        assert_eq!(failure.source().unwrap().to_string(), cause);
    }
}
