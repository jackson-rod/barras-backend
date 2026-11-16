//! The command line of `barras-api`. With no argument it serves; `openapi` prints the OpenAPI
//! document and `openapi --check <file>` compares a file with it, both without reading the
//! environment or opening a socket.

use std::error::Error;
use std::fmt::Display;
use std::fs;
use std::io::{self, Write};
use std::process::ExitCode;

use barras_bootstrap::Environment;
use tokio::runtime;

use crate::{ApiConfig, RunError, openapi_json, run};

const USAGE: &str = "\
usage: barras-api                          serve the API, configured by the environment
       barras-api openapi                  print the OpenAPI document
       barras-api openapi --check <file>   fail unless <file> holds the document the code generates";

/// The exit status of a command line it does not understand, as with most tools.
const USAGE_ERROR: u8 = 2;

/// What the arguments ask for.
#[derive(Debug, PartialEq, Eq)]
enum Command<'a> {
    Serve,
    PrintOpenApi,
    CheckOpenApi(&'a str),
    Usage,
}

impl<'a> Command<'a> {
    fn parse(args: &'a [String]) -> Self {
        match args.iter().map(String::as_str).collect::<Vec<_>>()[..] {
            [] => Self::Serve,
            ["openapi"] => Self::PrintOpenApi,
            ["openapi", "--check", file] => Self::CheckOpenApi(file),
            _ => Self::Usage,
        }
    }
}

/// Does what `args`, the arguments after the binary's name, ask for, and returns the exit status.
pub fn main_with_args(args: &[String]) -> ExitCode {
    match Command::parse(args) {
        Command::Serve => serve(),
        Command::PrintOpenApi => print_openapi(),
        Command::CheckOpenApi(file) => check_openapi(file),
        Command::Usage => {
            report(&USAGE);
            ExitCode::from(USAGE_ERROR)
        }
    }
}

/// Reads the configuration, then serves until SIGTERM or Ctrl-C. A configuration with a
/// problem stops it before the runtime starts, and so before any socket is opened.
fn serve() -> ExitCode {
    let config = match ApiConfig::from_env(Environment::from_process()) {
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
            tracing::error!(error = %with_sources(&error), "barras-api stopped on a failure");
            ExitCode::FAILURE
        }
    }
}

fn print_openapi() -> ExitCode {
    let document = match openapi_json() {
        Ok(document) => document,
        Err(error) => return fail(&with_sources(&error)),
    };
    // `print!` panics when standard output is closed; this reports it like any other failure.
    let mut stdout = io::stdout().lock();
    match stdout
        .write_all(document.as_bytes())
        .and_then(|()| stdout.flush())
    {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => fail(&format!(
            "the OpenAPI document could not be written: {error}"
        )),
    }
}

/// Succeeds only when `file` holds, byte for byte, the document the code generates.
fn check_openapi(file: &str) -> ExitCode {
    let document = match openapi_json() {
        Ok(document) => document,
        Err(error) => return fail(&with_sources(&error)),
    };
    match fs::read(file) {
        Ok(committed) if committed == document.as_bytes() => ExitCode::SUCCESS,
        Ok(_) => fail(&format!(
            "{file} is not the document the code generates: run `just openapi`"
        )),
        Err(error) => fail(&format!(
            "{file} could not be read ({error}): run `just openapi`"
        )),
    }
}

fn fail(message: &dyn Display) -> ExitCode {
    report(message);
    ExitCode::FAILURE
}

#[allow(clippy::print_stderr)] // the logs are not up yet: serving starts them once the configuration is read, and no other mode has any
fn report(message: &dyn Display) {
    eprintln!("{message}");
}

/// The error's text, then that of each of its sources in turn, joined by `": "`: at most
/// sixteen, since an error whose sources led back to itself would never end.
fn with_sources(error: &(dyn Error + 'static)) -> String {
    std::iter::successors(Some(error), |&error| error.source())
        .take(16)
        .map(ToString::to_string)
        .collect::<Vec<_>>()
        .join(": ")
}

#[cfg(test)]
mod tests {
    use super::*;

    fn args(words: &[&str]) -> Vec<String> {
        words.iter().map(|&word| word.to_owned()).collect()
    }

    #[test]
    fn no_argument_serves() {
        assert_eq!(Command::parse(&args(&[])), Command::Serve);
    }

    #[test]
    fn openapi_prints_the_document_and_check_compares_a_file_with_it() {
        let print = args(&["openapi"]);
        let check = args(&["openapi", "--check", "openapi.json"]);

        assert_eq!(Command::parse(&print), Command::PrintOpenApi);
        assert_eq!(
            Command::parse(&check),
            Command::CheckOpenApi("openapi.json")
        );
    }

    #[test]
    fn any_other_command_line_is_answered_with_the_usage() {
        for words in [
            &["serve"][..],
            &["--help"],
            &["openapi", "--check"],
            &["openapi", "openapi.json"],
            &["openapi", "--check", "openapi.json", "extra"],
            &["--check", "openapi.json", "openapi"],
            &["OPENAPI"],
        ] {
            assert_eq!(Command::parse(&args(words)), Command::Usage, "{words:?}");
        }
    }

    #[test]
    fn a_failure_is_reported_with_every_source_in_turn() {
        let cause = io::Error::other("connection reset");
        let failure = RunError::Signal(cause);

        assert_eq!(
            with_sources(&failure),
            "SIGTERM and SIGINT could not be listened for: connection reset"
        );
    }
}
