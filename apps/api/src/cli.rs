//! The command line of `barras-api`. With no argument it serves; `openapi` prints the OpenAPI
//! document and `openapi --check <file>` compares a file with it, both without reading the
//! environment or opening a socket. Neither starts the logs, so each says on standard error
//! what failed, as serving does until its logs are up.

use std::fs;
use std::io::{self, Write};
use std::process::ExitCode;

use barras_bootstrap::{Environment, build_runtime, report_failure, report_usage, with_sources};

use crate::{ApiConfig, RunError, openapi_json, run};

const USAGE: &str = "\
usage: barras-api                          serve the API, configured by the environment
       barras-api openapi                  print the OpenAPI document
       barras-api openapi --check <file>   fail unless <file> holds the document the code generates";

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
        Command::Usage => report_usage(&USAGE),
    }
}

/// Reads the configuration, then serves until SIGTERM or Ctrl-C. A configuration with a
/// problem stops it before the runtime starts, and so before any socket is opened.
fn serve() -> ExitCode {
    let config = match ApiConfig::from_env(Environment::from_process()) {
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
            tracing::error!(error = %with_sources(&error), "barras-api stopped on a failure");
            ExitCode::FAILURE
        }
    }
}

fn print_openapi() -> ExitCode {
    let document = match openapi_json() {
        Ok(document) => document,
        Err(error) => return report_failure(&with_sources(&error)),
    };
    // `print!` panics when standard output is closed; this reports it like any other failure.
    let mut stdout = io::stdout().lock();
    match stdout
        .write_all(document.as_bytes())
        .and_then(|()| stdout.flush())
    {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => report_failure(&format!(
            "the OpenAPI document could not be written: {error}"
        )),
    }
}

/// Succeeds only when `file` holds, byte for byte, the document the code generates.
fn check_openapi(file: &str) -> ExitCode {
    let document = match openapi_json() {
        Ok(document) => document,
        Err(error) => return report_failure(&with_sources(&error)),
    };
    match fs::read(file) {
        Ok(committed) if committed == document.as_bytes() => ExitCode::SUCCESS,
        Ok(_) => report_failure(&format!(
            "{file} is not the document the code generates: run `just openapi`"
        )),
        Err(error) => report_failure(&format!(
            "{file} could not be read ({error}): run `just openapi`"
        )),
    }
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
}
