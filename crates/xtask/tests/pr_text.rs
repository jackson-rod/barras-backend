#![allow(clippy::unwrap_used, clippy::expect_used)] // integration test crate: every item is test code

use std::fs;
use std::process::Command;

/// Runs `xtask check-pr` on a file that holds `text`, as CI writes the title and description.
fn check_pr(text: &str) -> (Option<i32>, String) {
    let dir = tempfile::tempdir().unwrap();
    let file = dir.path().join("pull-request.txt");
    fs::write(&file, text).unwrap();
    let output = Command::new(env!("CARGO_BIN_EXE_xtask"))
        .arg("check-pr")
        .arg(&file)
        .output()
        .unwrap();
    (
        output.status.code(),
        String::from_utf8_lossy(&output.stderr).into_owned(),
    )
}

#[test]
fn a_good_title_and_a_markdown_description_pass() {
    let (code, stderr) = check_pr(
        "build(BAR-2): add the workspace skeleton\n\n## Ticket\n\nBAR-2:\n\n## What and why\n\nThe workspace and its checks.\n",
    );
    assert_eq!(code, Some(0), "{stderr}");
}

#[test]
fn a_title_that_breaks_the_subject_rule_fails() {
    let (code, stderr) = check_pr("Build/bar 2 workspace skeleton\n\n## Ticket\n\nBAR-2:\n");
    assert_eq!(code, Some(1), "{stderr}");
}

#[test]
fn attribution_under_a_markdown_heading_fails() {
    let (code, stderr) = check_pr(
        "build(BAR-2): add the workspace skeleton\n\n## Notes\n\n🤖 Generated with [Claude Code](https://claude.com/claude-code)\n",
    );
    assert_eq!(code, Some(1), "{stderr}");
}
