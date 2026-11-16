#![allow(clippy::print_stdout, clippy::print_stderr)] // a CLI: its output is the product

use std::path::Path;
use std::process::{Command, ExitCode};

use xtask::{arch, coverage, git_rules, structure};

const USAGE: &str = "subcommands:\n  check-branch [name]\n  check-commit-msg <file>\n  check-commits <range>\n  check-pr <file>\n  check-arch\n  check-structure\n  check-coverage <lcov-file> [workspace-root]";

fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let args: Vec<&str> = args.iter().map(String::as_str).collect();
    let result = match args.as_slice() {
        ["check-branch"] => check_current_branch(),
        ["check-branch", name] => git_rules::check_branch_name(name).map_err(|e| vec![e]),
        ["check-commit-msg", file] => check_commit_file(file),
        ["check-commits", range] => check_commit_range(range),
        ["check-pr", file] => check_pr_file(file),
        ["check-arch"] => check_architecture(),
        ["check-structure"] => check_repository_structure(),
        ["check-coverage", report] => check_coverage_report(report, &arch::workspace_root()),
        // The root is an argument so that the tests of this binary can measure a
        // temporary workspace. `just test-cov` gives none.
        ["check-coverage", report, root] => check_coverage_report(report, Path::new(root)),
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

/// The title on the first line, a blank line, then the description: a squash merge turns
/// them into the commit that lands on `develop`. Taken as it is, because the description
/// is Markdown and a `## Heading` line is content, not a git comment.
fn check_pr_file(file: &str) -> Problems {
    let text =
        std::fs::read_to_string(file).map_err(|e| vec![format!("could not read {file}: {e}")])?;
    git_rules::check_commit_message(&text).map_err(|e| vec![e])
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
    let violations =
        arch::check_workspace(&arch::workspace_root()).map_err(|e| vec![e.to_string()])?;
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

/// Prints every figure before failing, so a failed run still shows where each crate stands.
fn check_coverage_report(report: &str, root: &Path) -> Problems {
    let measured = coverage::measure_coverage_file(root, Path::new(report))
        .map_err(|e| vec![e.to_string()])?;
    for line in measured.summary() {
        println!("{line}");
    }
    let failures = coverage::judge(&measured);
    if failures.is_empty() {
        Ok(())
    } else {
        Err(failures.iter().map(ToString::to_string).collect())
    }
}
