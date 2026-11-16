//! The command line of `barras-ingest`. With no argument it runs, configured by the
//! environment; any argument is answered with the usage.

use std::fmt::Display;
use std::process::ExitCode;

use barras_bootstrap::{Environment, with_sources};
use tokio::runtime;

use crate::{IngestConfig, RunError, run};

const USAGE: &str = "usage: barras-ingest   run the ingestor, configured by the environment";

/// The exit status of a command line it does not understand, as with most tools.
const USAGE_ERROR: u8 = 2;

/// Does what `args`, the arguments after the binary's name, ask for, and returns the exit status.
pub fn main_with_args(args: &[String]) -> ExitCode {
    match args {
        [] => serve(),
        _ => {
            report(&USAGE);
            ExitCode::from(USAGE_ERROR)
        }
    }
}

/// Reads the configuration, then serves until SIGTERM or Ctrl-C. A configuration with a
/// problem stops it before the runtime starts, and so before any socket is opened.
fn serve() -> ExitCode {
    let config = match IngestConfig::from_env(Environment::from_process()) {
        Ok(config) => config,
        Err(error) => return fail(&error),
    };
    let runtime = match runtime::Builder::new_multi_thread().enable_all().build() {
        Ok(runtime) => runtime,
        Err(error) => return fail(&format!("the tokio runtime could not start: {error}")),
    };
    match runtime.block_on(run(config)) {
        Ok(()) => ExitCode::SUCCESS,
        // The logs are what failed, so standard error is the only place left to say so.
        Err(error @ RunError::Logs(_)) => fail(&with_sources(&error)),
        Err(error) => {
            tracing::error!(error = %with_sources(&error), "barras-ingest stopped on a failure");
            ExitCode::FAILURE
        }
    }
}

fn fail(message: &dyn Display) -> ExitCode {
    report(message);
    ExitCode::FAILURE
}

#[allow(clippy::print_stderr)] // the logs are not up: serving starts them only once the configuration is read, and they may fail to start
fn report(message: &dyn Display) {
    eprintln!("{message}");
}
