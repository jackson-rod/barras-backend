#![allow(clippy::unwrap_used, clippy::expect_used)] // integration test crate: every item is test code

use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Command, Output};

/// How many lines of every fixture file sit in its test module, all of them run. Counted
/// as production lines, they would lift a kernel at 94% to its bar.
const TEST_LINES: usize = 20;

/// A temporary workspace and the LCOV report of a test run over it.
struct Workspace {
    dir: tempfile::TempDir,
    lcov: String,
}

impl Workspace {
    fn new() -> Self {
        Self {
            dir: tempfile::tempdir().unwrap(),
            lcov: String::new(),
        }
    }

    /// Adds a source file with `found` production lines, the first `hit` of them run, and
    /// an inline test module after them.
    fn file(mut self, relative: &str, hit: usize, found: usize) -> Self {
        let mut source = vec!["// production"; found];
        source.extend(["#[cfg(test)]", "mod tests {"]);
        source.extend(vec!["    // test"; TEST_LINES]);
        source.push("}");
        let path = self.dir.path().join(relative);
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(&path, source.join("\n") + "\n").unwrap();

        self.lcov.push_str(&format!("SF:{}\n", path.display()));
        for line in 1..=found {
            self.lcov
                .push_str(&format!("DA:{line},{}\n", u8::from(line <= hit)));
        }
        let first_test_line = found + 3;
        for line in first_test_line..first_test_line + TEST_LINES {
            self.lcov.push_str(&format!("DA:{line},1\n"));
        }
        // The sums are wrong on purpose: the lines are counted, never taken from them.
        self.lcov.push_str("LF:0\nLH:0\nend_of_record\n");
        self
    }

    fn report(&self) -> PathBuf {
        let path = self.dir.path().join("lcov.info");
        fs::write(&path, &self.lcov).unwrap();
        path
    }

    fn check(&self) -> Checked {
        check(&[&self.report(), self.dir.path()])
    }
}

struct Checked {
    code: Option<i32>,
    stdout: String,
    stderr: String,
}

fn check(args: &[&Path]) -> Checked {
    checked(
        Command::new(env!("CARGO_BIN_EXE_xtask"))
            .arg("check-coverage")
            .args(args)
            .output()
            .unwrap(),
    )
}

fn checked(output: Output) -> Checked {
    Checked {
        code: output.status.code(),
        stdout: String::from_utf8_lossy(&output.stdout).into_owned(),
        stderr: String::from_utf8_lossy(&output.stderr).into_owned(),
    }
}

#[test]
fn a_report_at_exactly_both_bars_passes_and_prints_every_figure() {
    let run = Workspace::new()
        .file("crates/kernel/src/money.rs", 95, 100)
        .file("apps/api/src/main.rs", 75, 100)
        .check();
    assert_eq!(run.code, Some(0), "{}", run.stderr);
    assert_eq!(
        run.stdout,
        "apps/api: 75.0% (75/100)\ncrates/kernel: 95.0% (95/100)\ntotal: 85.0% (170/200)\n"
    );
    assert_eq!(run.stderr, "");
}

#[test]
fn a_kernel_at_94_percent_fails_and_is_named() {
    let run = Workspace::new()
        .file("crates/kernel/src/money.rs", 94, 100)
        .file("apps/api/src/main.rs", 100, 100)
        .check();
    assert_eq!(run.code, Some(1), "{}", run.stdout);
    assert!(
        run.stderr.starts_with("crates/kernel: 94.0% (94/100)"),
        "{}",
        run.stderr
    );
    assert!(run.stderr.contains("95%"), "{}", run.stderr);
    assert!(!run.stderr.contains("total"), "{}", run.stderr);
    assert!(
        run.stdout.contains("total: 97.0% (194/200)"),
        "{}",
        run.stdout
    );
}

#[test]
fn a_module_under_its_bar_fails_and_is_named() {
    let run = Workspace::new()
        .file("crates/kernel/src/money.rs", 100, 100)
        .file("crates/modules/goals/src/domain/goal.rs", 94, 100)
        .check();
    assert_eq!(run.code, Some(1), "{}", run.stdout);
    assert!(
        run.stderr
            .starts_with("crates/modules/goals: 94.0% (94/100)"),
        "{}",
        run.stderr
    );
}

#[test]
fn a_total_at_84_percent_fails() {
    let run = Workspace::new()
        .file("crates/kernel/src/money.rs", 100, 100)
        .file("apps/api/src/main.rs", 68, 100)
        .check();
    assert_eq!(run.code, Some(1), "{}", run.stdout);
    assert!(
        run.stderr.starts_with("total: 84.0% (168/200)"),
        "{}",
        run.stderr
    );
    assert!(run.stderr.contains("85%"), "{}", run.stderr);
    assert!(!run.stderr.contains("crates/kernel"), "{}", run.stderr);
}

#[test]
fn a_missing_report_fails() {
    let workspace = Workspace::new();
    let missing = workspace.dir.path().join("lcov.info");
    let run = check(&[&missing, workspace.dir.path()]);
    assert_eq!(run.code, Some(1), "{}", run.stdout);
    assert!(run.stderr.contains("lcov.info"), "{}", run.stderr);
    assert_eq!(run.stdout, "");
}

#[test]
fn a_report_cut_short_fails() {
    let mut workspace = Workspace::new()
        .file("crates/kernel/src/money.rs", 100, 100)
        .file("apps/api/src/main.rs", 100, 100);
    let whole = workspace.lcov.len();
    workspace.lcov.truncate(whole - "end_of_record\n".len());
    let run = workspace.check();
    assert_eq!(run.code, Some(1), "{}", run.stdout);
    assert_eq!(run.stdout, "");
}

#[test]
fn a_report_that_measures_nothing_fails() {
    let run = Workspace::new().check();
    assert_eq!(run.code, Some(1), "{}", run.stdout);
    assert!(run.stderr.starts_with("total: "), "{}", run.stderr);
}

#[test]
fn without_a_root_the_workspace_is_this_repository_wherever_the_command_runs() {
    // A report about another workspace measures nothing here, even from inside it.
    let other = Workspace::new()
        .file("crates/kernel/src/money.rs", 100, 100)
        .file("apps/api/src/main.rs", 100, 100);
    let run = checked(
        Command::new(env!("CARGO_BIN_EXE_xtask"))
            .arg("check-coverage")
            .arg(other.report())
            .current_dir(other.dir.path())
            .output()
            .unwrap(),
    );
    assert_eq!(run.code, Some(1), "{}", run.stdout);
    assert!(run.stderr.starts_with("total: "), "{}", run.stderr);

    // A report about this crate's own first line is measured.
    let own_file = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("src/main.rs")
        .canonicalize()
        .unwrap();
    fs::write(
        other.dir.path().join("own.info"),
        format!("SF:{}\nDA:1,1\nend_of_record\n", own_file.display()),
    )
    .unwrap();
    let run = checked(
        Command::new(env!("CARGO_BIN_EXE_xtask"))
            .args(["check-coverage", "own.info"])
            .current_dir(other.dir.path())
            .output()
            .unwrap(),
    );
    assert_eq!(run.code, Some(0), "{}", run.stderr);
    assert_eq!(
        run.stdout,
        "crates/xtask: 100.0% (1/1)\ntotal: 100.0% (1/1)\n"
    );
}

#[test]
fn the_report_is_a_required_argument() {
    let run = check(&[]);
    assert_eq!(run.code, Some(2), "{}", run.stdout);
    assert!(
        run.stderr.contains("check-coverage <lcov-file>"),
        "{}",
        run.stderr
    );
}
