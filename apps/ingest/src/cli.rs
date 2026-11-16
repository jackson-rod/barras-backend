//! The command line of `barras-ingest`. With no argument it runs, configured by the
//! environment; any argument is answered with the usage.

use std::process::ExitCode;

use barras_bootstrap::{Environment, build_runtime, report_failure, report_usage, with_sources};

use crate::{IngestConfig, RunError, run};

const USAGE: &str = "usage: barras-ingest   run the ingestor, configured by the environment";

/// Does what `args`, the arguments after the binary's name, ask for, and returns the exit status.
pub fn main_with_args(args: &[String]) -> ExitCode {
    match args {
        [] => serve(),
        _ => report_usage(&USAGE),
    }
}

/// Reads the configuration, then serves until SIGTERM or Ctrl-C. A configuration with a
/// problem stops it before the runtime starts, and so before any socket is opened.
fn serve() -> ExitCode {
    let config = match IngestConfig::from_env(Environment::from_process()) {
        Ok(config) => config,
        Err(error) => return report_failure(&error),
    };
    let runtime = match build_runtime() {
        Ok(runtime) => runtime,
        Err(error) => return report_failure(&with_sources(&error)),
    };
    match runtime.block_on(run(config)) {
        Ok(()) => ExitCode::SUCCESS,
        // The logs are what failed, so standard error is the only place left to say so.
        Err(error @ RunError::Logs(_)) => report_failure(&with_sources(&error)),
        Err(error) => {
            tracing::error!(error = %with_sources(&error), "barras-ingest stopped on a failure");
            ExitCode::FAILURE
        }
    }
}
