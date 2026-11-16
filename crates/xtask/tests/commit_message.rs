#![allow(clippy::unwrap_used, clippy::expect_used)] // integration test crate: every item is test code

use std::fs;
use std::process::Command;

/// Runs `xtask check-commit-msg` on a file that holds `text`, as the `commit-msg` hook does
/// with the file git writes.
fn check_commit_msg(text: &str) -> (Option<i32>, String) {
    let dir = tempfile::tempdir().unwrap();
    let file = dir.path().join("COMMIT_EDITMSG");
    fs::write(&file, text).unwrap();
    let output = Command::new(env!("CARGO_BIN_EXE_xtask"))
        .arg("check-commit-msg")
        .arg(&file)
        .output()
        .unwrap();
    (
        output.status.code(),
        String::from_utf8_lossy(&output.stderr).into_owned(),
    )
}

#[test]
fn a_good_message_passes_with_the_comments_git_adds() {
    let (code, stderr) = check_commit_msg(
        "docs(BAR-3): fix a typo in the readme\n\n# Please enter the commit message for your changes.\n",
    );
    assert_eq!(code, Some(0), "{stderr}");
}

#[test]
fn a_marker_that_skips_ci_fails_in_the_subject_the_body_or_a_trailer() {
    for text in [
        "docs(BAR-3): fix a typo in the readme [skip ci]\n",
        "docs(BAR-3): fix a typo in the readme\n\nNothing to build [Actions Skip].\n",
        "docs(BAR-3): fix a typo in the readme\n\n\nskip-checks:true\n",
    ] {
        let (code, stderr) = check_commit_msg(text);
        assert_eq!(code, Some(1), "{text}");
        assert!(stderr.contains("skip-CI marker rule"), "{stderr}");
    }
}

#[test]
fn a_marker_in_a_comment_git_drops_is_not_part_of_the_message() {
    let (code, stderr) = check_commit_msg(
        "docs(BAR-3): fix a typo in the readme\n\n# [skip ci] would skip the run\n",
    );
    assert_eq!(code, Some(0), "{stderr}");
}
