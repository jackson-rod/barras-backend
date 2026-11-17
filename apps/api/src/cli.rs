//! The command line of `barras-api`. With no argument it serves; `openapi` prints the OpenAPI
//! document and `openapi --check <file>` compares a file with it, both without reading the
//! environment or opening a socket; `migrate`, `seed <folder>` and `reset <folder>` are the
//! database commands. None but serving starts the logs, so each says on standard error what
//! failed, as serving does until its logs are up.

use std::fs;
use std::io::{self, Write};
use std::path::Path;
use std::process::ExitCode;

use barras_bootstrap::{Environment, build_runtime, report_failure, report_usage, with_sources};

use crate::database_commands;
use crate::{ApiConfig, RunError, openapi_json, run};

const USAGE: &str = "\
usage: barras-api                          serve the API, configured by the environment
       barras-api openapi                  print the OpenAPI document
       barras-api openapi --check <file>   fail unless <file> holds the document the code generates
       barras-api migrate                  apply the pending migrations to API_DATABASE_URL
       barras-api seed <folder>            load the seed files of <folder> into the local database
       barras-api reset <folder>           rebuild the local database: drop, create, migrate, seed";

/// What the arguments ask for.
#[derive(Debug, PartialEq, Eq)]
enum Command<'a> {
    Serve,
    PrintOpenApi,
    CheckOpenApi(&'a str),
    Migrate,
    Seed(&'a str),
    Reset(&'a str),
    Usage,
}

impl<'a> Command<'a> {
    fn parse(args: &'a [String]) -> Self {
        match args.iter().map(String::as_str).collect::<Vec<_>>()[..] {
            [] => Self::Serve,
            ["openapi"] => Self::PrintOpenApi,
            ["openapi", "--check", file] => Self::CheckOpenApi(file),
            ["migrate"] => Self::Migrate,
            ["seed", folder] if !is_option(folder) => Self::Seed(folder),
            ["reset", folder] if !is_option(folder) => Self::Reset(folder),
            _ => Self::Usage,
        }
    }
}

/// Whether `word`, where a folder goes, is an option such as `--help` or `-h` instead: read as a
/// folder, `reset --help` would rebuild the database. A folder whose name starts with `-` is
/// still reached as `./-name`.
fn is_option(word: &str) -> bool {
    word.starts_with('-')
}

/// Does what `args`, the arguments after the binary's name, ask for, and returns the exit status.
pub fn main_with_args(args: &[String]) -> ExitCode {
    match Command::parse(args) {
        Command::Serve => serve(),
        Command::PrintOpenApi => print_openapi(),
        Command::CheckOpenApi(file) => check_openapi(file),
        Command::Migrate => database_commands::migrate(),
        Command::Seed(folder) => database_commands::seed(Path::new(folder)),
        Command::Reset(folder) => database_commands::reset(Path::new(folder)),
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
    fn migrate_seed_and_reset_are_the_database_commands() {
        let migrate = args(&["migrate"]);
        let seed = args(&["seed", "seeds"]);
        let reset = args(&["reset", "fixtures/seeds"]);

        assert_eq!(Command::parse(&migrate), Command::Migrate);
        assert_eq!(Command::parse(&seed), Command::Seed("seeds"));
        assert_eq!(Command::parse(&reset), Command::Reset("fixtures/seeds"));
    }

    #[test]
    fn a_folder_with_a_dash_after_its_first_character_is_still_a_folder() {
        let seed = args(&["seed", "demo-seeds"]);
        let reset = args(&["reset", "./-seeds"]);

        assert_eq!(Command::parse(&seed), Command::Seed("demo-seeds"));
        assert_eq!(Command::parse(&reset), Command::Reset("./-seeds"));
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
            &["migrate", "now"],
            &["seed"],
            &["seed", "seeds", "more-seeds"],
            &["reset"],
            &["reset", "seeds", "--force"],
            &["seeds", "seed"],
            &["Migrate"],
            // An option where the folder goes: taken for a folder, `reset --help` would rebuild
            // the database.
            &["reset", "--help"],
            &["reset", "-h"],
            &["seed", "-h"],
            &["seed", "--help"],
        ] {
            assert_eq!(Command::parse(&args(words)), Command::Usage, "{words:?}");
        }
    }

    #[test]
    fn the_usage_names_every_command_line_it_knows() {
        let forms: Vec<&str> = USAGE
            .lines()
            .map(|line| {
                let line = line.trim_start_matches("usage:").trim_start();
                line.split("   ").next().unwrap().trim()
            })
            .collect();

        assert_eq!(
            forms,
            [
                "barras-api",
                "barras-api openapi",
                "barras-api openapi --check <file>",
                "barras-api migrate",
                "barras-api seed <folder>",
                "barras-api reset <folder>",
            ]
        );
    }
}
