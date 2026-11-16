//! `barras-ingest`: the feed connections and the scheduled jobs (BAR-6), and the internal port
//! that reports on them. This is the composition root; `main.rs` only hands it the command line,
//! so the tests reach the same code as the binary.

pub mod cli;
mod config;

use std::error::Error;
use std::fmt;
use std::io;

use barras_adapter_http::{
    InternalApp, Metrics, MetricsError, ServeError, ServiceInfo, bind, serve_internal,
};
use barras_bootstrap::{LoggingError, ShutdownSignal, init_logging, install_panic_hook};

pub use config::IngestConfig;

/// The binary as it names itself in its metrics and on its internal port.
const SERVICE: ServiceInfo = ServiceInfo {
    name: "barras-ingest",
    version: env!("CARGO_PKG_VERSION"),
};

/// Starts the logs, then serves the internal port until SIGTERM or Ctrl-C, and returns once the
/// requests in flight are answered.
pub async fn run(config: IngestConfig) -> Result<(), RunError> {
    init_logging(config.env, config.log_level).map_err(RunError::Logs)?;
    install_panic_hook();
    let metrics = Metrics::new(SERVICE).map_err(RunError::Metrics)?;
    // No probe until BAR-4 brings the first dependency.
    let app = InternalApp::new(SERVICE, metrics, Vec::new());
    // Before anything is bound, so that failing to listen for the signals stops the binary at
    // start rather than once it serves.
    let mut signal = ShutdownSignal::install().map_err(RunError::Signal)?;
    let internal = bind("internal", config.internal_bind_addr)
        .await
        .map_err(RunError::Serve)?;
    let internal_addr = internal.local_addr().map_err(RunError::Serve)?;
    tracing::info!(%internal_addr, "listening");
    let shutdown = async move {
        signal.recv().await;
        tracing::info!("shutting down");
    };
    serve_internal(app, internal, shutdown)
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
    /// SIGTERM and SIGINT could not be listened for.
    Signal(io::Error),
    /// The listener could not be bound, or stopped serving.
    Serve(ServeError),
}

impl fmt::Display for RunError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Logs(error) => error.fmt(f),
            Self::Metrics(error) => error.fmt(f),
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
            Self::Signal(error) => Some(error),
            Self::Serve(error) => error.source(),
        }
    }
}

#[cfg(test)]
mod tests {
    use barras_bootstrap::with_sources;

    use super::*;

    #[test]
    fn a_failure_whose_cause_names_no_step_names_it_and_keeps_the_cause() {
        let signal = RunError::Signal(io::Error::other("connection reset"));

        assert_eq!(
            signal.to_string(),
            "SIGTERM and SIGINT could not be listened for"
        );
        assert!(signal.source().unwrap().is::<io::Error>());
        assert_eq!(
            with_sources(&signal),
            "SIGTERM and SIGINT could not be listened for: connection reset"
        );
    }

    #[test]
    fn a_failure_whose_error_names_the_step_reads_as_that_error_sources_included() {
        use barras_bootstrap::{AppEnv, LogLevel};

        // Whichever call comes second fails: a process has one global subscriber.
        let _first = init_logging(AppEnv::Local, LogLevel::Error);
        let logs = init_logging(AppEnv::Local, LogLevel::Error).unwrap_err();
        let whole = with_sources(&logs);

        let failure = RunError::Logs(logs);

        assert_eq!(with_sources(&failure), whole);
    }
}
