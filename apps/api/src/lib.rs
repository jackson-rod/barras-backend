//! `barras-api`: the public API under `/api/v1` and its internal port, in one process. This is
//! the composition root; `main.rs` only hands it the command line, so the tests reach the same
//! code as the binary.

// Taken for its features, not code; named so that cargo never reports it as an unused dependency.
use workspace_hack as _;

pub mod cli;
mod config;

use std::error::Error;
use std::fmt;
use std::io;

use barras_adapter_http::{
    BuildError, HealthProbe, HttpApp, HttpAppBuilder, Metrics, MetricsError, ServeError,
    ServiceInfo, bind, serve_api,
};
use barras_bootstrap::{LoggingError, ShutdownSignal, init_logging, install_panic_hook};

pub use config::{ApiConfig, DatabaseUrlValue};

/// The binary as it names itself in its metrics and on its internal port.
const SERVICE: ServiceInfo = ServiceInfo {
    name: "barras-api",
    version: env!("CARGO_PKG_VERSION"),
};

/// What the wiring takes from outside the app: the registry its metrics go to, and a probe for
/// each dependency it reports on. BAR-4 adds the pools.
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
/// and returns once the requests in flight are answered.
pub async fn run(config: ApiConfig) -> Result<(), RunError> {
    init_logging(config.env, config.log_level).map_err(RunError::Logs)?;
    install_panic_hook();
    let metrics = Metrics::new(SERVICE).map_err(RunError::Metrics)?;
    // No probe until BAR-4 brings the first dependency.
    let dependencies = Dependencies {
        metrics,
        probes: Vec::new(),
    };
    let app = build_app(dependencies).map_err(RunError::App)?;
    // Before anything is bound, so that failing to listen for the signals stops the binary at
    // start rather than once it serves.
    let mut signal = ShutdownSignal::install().map_err(RunError::Signal)?;
    let public = bind("public", config.bind_addr)
        .await
        .map_err(RunError::Serve)?;
    let internal = bind("internal", config.internal_bind_addr)
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
        .map_err(RunError::Serve)?;
    tracing::info!("stopped");
    Ok(())
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
    use super::*;

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
