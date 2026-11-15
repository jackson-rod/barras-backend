#![allow(clippy::print_stdout, clippy::print_stderr)] // a CLI: its output is the product

use std::process::{Command, ExitCode};

use xtask::{arch, git_rules, structure};

const USAGE: &str = "subcommands:\n  check-branch [name]\n  check-commit-msg <file>\n  check-commits <range>\n  check-arch\n  check-structure";

fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let args: Vec<&str> = args.iter().map(String::as_str).collect();
    let result = match args.as_slice() {
        ["check-branch"] => check_current_branch(),
        ["check-branch", name] => git_rules::check_branch_name(name).map_err(|e| vec![e]),
        ["check-commit-msg", file] => check_commit_file(file),
        ["check-commits", range] => check_commit_range(range),
        ["check-arch"] => check_architecture(),
        ["check-structure"] => check_repository_structure(),
        _ => {
            eprintln!("{USAGE}");
            return ExitCode::from(2);
        }
    };
    match result {
        Ok(()) => ExitCode::SUCCESS,
        Err(problems) => {
            for problem in problems {
                eprintln!("{problem}");
            }
            ExitCode::from(1)
        }
    }
}

type Problems = Result<(), Vec<String>>;

fn git(args: &[&str]) -> Result<String, String> {
    let output = Command::new("git")
        .args(args)
        .output()
        .map_err(|e| format!("could not run git: {e}"))?;
    if output.status.success() {
        Ok(String::from_utf8_lossy(&output.stdout).into_owned())
    } else {
        Err(format!(
            "git {} failed: {}",
            args.join(" "),
            String::from_utf8_lossy(&output.stderr).trim()
        ))
    }
}

fn check_current_branch() -> Problems {
    let raw = git(&["rev-parse", "--abbrev-ref", "HEAD"]).map_err(|e| vec![e])?;
    match git_rules::branch_to_check(&raw) {
        None => Ok(()),
        Some(name) => git_rules::check_branch_name(name).map_err(|e| vec![e]),
    }
}

fn check_commit_file(file: &str) -> Problems {
    let raw =
        std::fs::read_to_string(file).map_err(|e| vec![format!("could not read {file}: {e}")])?;
    git_rules::check_commit_message(&git_rules::strip_git_noise(&raw)).map_err(|e| vec![e])
}

fn check_commit_range(range: &str) -> Problems {
    let hashes = git(&["log", "--format=%H", range]).map_err(|e| vec![e])?;
    let mut problems = Vec::new();
    for hash in hashes.lines() {
        match git(&["log", "-1", "--format=%B", hash]) {
            Ok(message) => {
                if let Err(e) = git_rules::check_commit_message(&message) {
                    problems.push(format!("{}: {e}", hash.get(..7).unwrap_or(hash)));
                }
            }
            Err(e) => problems.push(e),
        }
    }
    if problems.is_empty() {
        Ok(())
    } else {
        Err(problems)
    }
}

fn check_architecture() -> Problems {
    let crates = arch::load_workspace(&arch::workspace_root()).map_err(|e| vec![e.to_string()])?;
    let violations = arch::check_arch(&crates);
    if violations.is_empty() {
        return Ok(());
    }
    Err(violations
        .iter()
        .map(|v| format!("{}: {} ({})", v.krate, v.dependency, v.rule))
        .collect())
}

fn check_repository_structure() -> Problems {
    let problems =
        structure::check_structure(&arch::workspace_root()).map_err(|e| vec![e.to_string()])?;
    if problems.is_empty() {
        Ok(())
    } else {
        Err(problems)
    }
}
