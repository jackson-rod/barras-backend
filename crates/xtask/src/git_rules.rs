//! Branch-name and commit-message rules, checked by git hooks and CI.

use regex::Regex;

const TYPES: &str = "feat|fix|refactor|perf|test|docs|chore|ci|build";
const MAX_BRANCH_LEN: usize = 50;
const MAX_SUBJECT_LEN: usize = 100;

fn matches(pattern: &str, text: &str) -> Result<bool, String> {
    // Built per call: the checks run once per process, and a bad pattern then
    // surfaces as an error instead of a panic.
    Regex::new(pattern)
        .map(|re| re.is_match(text))
        .map_err(|e| format!("internal error: invalid pattern {pattern:?}: {e}"))
}

/// Accepts `main`, `develop` and `type/BAR-<n>-short-slug` of at most 50 characters.
pub fn check_branch_name(name: &str) -> Result<(), String> {
    if name == "main" || name == "develop" {
        return Ok(());
    }
    let length = name.chars().count();
    if length > MAX_BRANCH_LEN {
        return Err(format!(
            "branch name rule: at most {MAX_BRANCH_LEN} characters, got {length}: {name:?}"
        ));
    }
    let pattern = format!("^({TYPES})/BAR-[0-9]+-[a-z0-9]+(-[a-z0-9]+)*$");
    if matches(&pattern, name)? {
        Ok(())
    } else {
        Err(format!(
            "branch name rule: expected type/BAR-<n>-short-slug with type one of {}, got {name:?}",
            TYPES.replace('|', ", ")
        ))
    }
}

/// `None` for a detached `HEAD`, otherwise the trimmed branch name.
pub fn branch_to_check(raw: &str) -> Option<&str> {
    let name = raw.trim();
    (name != "HEAD").then_some(name)
}

/// Removes what git adds to a commit message file: CRLF, comment lines and the scissors diff.
pub fn strip_git_noise(raw: &str) -> String {
    let normalised = raw.replace("\r\n", "\n");
    let kept: Vec<&str> = normalised
        .lines()
        // Only git's scissors line cuts; ">8" in ordinary text must not hide the trailers after it.
        .take_while(|line| !(line.starts_with('#') && line.contains(">8")))
        .filter(|line| !line.starts_with('#'))
        .collect();
    let mut text = kept.join("\n");
    text.truncate(text.trim_end().len());
    text.push('\n');
    text
}

/// Checks the subject format, and rejects AI attribution and the markers that skip CI
/// anywhere in the message.
pub fn check_commit_message(message: &str) -> Result<(), String> {
    let subject = message.lines().next().unwrap_or("");
    check_subject(subject)?;
    check_attribution(message)?;
    check_skip_markers(message)
}

/// The markers that make GitHub start no workflow run: the five bracketed ones, and the
/// skip-checks trailer. A run that never starts never fails, so no check would see the
/// change, and a squash merge carries the title and may carry the description onto
/// develop. GitHub does not document its matching to the letter, so case and the spaces
/// inside are ignored, and the trailer counts anywhere.
const SKIP_MARKERS: &str = concat!(
    r"(?i)\[\s*(skip\s+ci|ci\s+skip|no\s+ci|skip\s+actions|actions\s+skip)\s*\]",
    r"|skip-checks\s*:\s*true",
);

fn check_skip_markers(message: &str) -> Result<(), String> {
    let found = Regex::new(SKIP_MARKERS)
        .map_err(|e| format!("internal error: invalid pattern {SKIP_MARKERS:?}: {e}"))?
        .find(message);
    match found {
        Some(marker) => Err(format!(
            "no skip-CI marker rule: GitHub starts no CI run for a message that holds {:?}; remove it",
            marker.as_str()
        )),
        None => Ok(()),
    }
}

/// The merge and revert subjects that git and GitHub write themselves. Only these skip the
/// subject rule, so "Merge the two config loaders" cannot pass as one.
const GENERATED_SUBJECT: &str = concat!(
    r"^(Merge pull request #[0-9]+ from \S.*",
    r"|Merge (branch|remote-tracking branch|tag) '[^']+'.*",
    r"|Merge [0-9a-f]{4,40} into [0-9a-f]{4,40}",
    r#"|Revert ".+".*)$"#,
);

fn check_subject(subject: &str) -> Result<(), String> {
    if matches(GENERATED_SUBJECT, subject)? {
        return Ok(());
    }
    let length = subject.chars().count();
    if length > MAX_SUBJECT_LEN {
        return Err(format!(
            "commit subject rule: at most {MAX_SUBJECT_LEN} characters, got {length}: {subject:?}"
        ));
    }
    let pattern = format!(r"^({TYPES})\(BAR-[0-9]+\): [a-z].*$");
    if matches(&pattern, subject)? {
        Ok(())
    } else {
        Err(format!(
            "commit subject rule: expected type(BAR-<n>): description starting in lower case, got {subject:?}"
        ))
    }
}

/// Lower case. A person who shares one of these names is refused too; that cost is accepted.
const AI_NAMES: &[&str] = &[
    "claude",
    "anthropic",
    "copilot",
    "cursor",
    "codex",
    "chatgpt",
    "openai",
    "gemini",
    "devin",
];
const TRAILERS: &[&str] = &[
    "co-authored-by:",
    "signed-off-by:",
    "generated-by:",
    "assisted-by:",
];
const SESSION_LINKS: &[&str] = &[
    "claude.ai/code",
    "claude.ai/share",
    "claude.com/claude-code",
];

/// Attribution is a trailer or a "generated with/by" line that names an AI tool, or a link
/// to a tool's session. Anything broader would refuse honest sentences such as
/// "openapi.json is generated with utoipa".
fn check_attribution(message: &str) -> Result<(), String> {
    for line in message.lines() {
        let lower = line.to_lowercase();
        let trailer = TRAILERS
            .iter()
            .find_map(|trailer| lower.trim_start().strip_prefix(trailer))
            .is_some_and(names_ai);
        let sentence = lower.trim_start_matches(|c: char| !c.is_alphabetic());
        let generated = (sentence.starts_with("generated with")
            || sentence.starts_with("generated by"))
            && names_ai(&lower);
        let session = SESSION_LINKS.iter().any(|link| lower.contains(link));
        if trailer || generated || session {
            return Err(format!("no AI attribution rule: remove {line:?}"));
        }
    }
    Ok(())
}

fn names_ai(lower: &str) -> bool {
    AI_NAMES.iter().any(|name| lower.contains(name))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn branch_names() {
        let exactly_50 = format!("feat/BAR-2-{}", "a".repeat(39));
        let over_50 = format!("feat/BAR-2-{}", "a".repeat(40));
        for ok in [
            "main",
            "develop",
            "build/BAR-2-workspace-skeleton",
            "feat/BAR-1700-trade-log-cursor",
            exactly_50.as_str(),
        ] {
            assert!(check_branch_name(ok).is_ok(), "{ok}");
        }
        for bad in [
            "feature/BAR-2-x",
            "feat/bar-2-x",
            "feat/BAR-2_x",
            "feat/BAR-2-X",
            "feat/BAR-x",
            "feat/BAR-2-",
            "BAR-2-x",
            over_50.as_str(),
        ] {
            assert!(check_branch_name(bad).is_err(), "{bad}");
        }
    }

    #[test]
    fn a_detached_head_is_skipped() {
        assert_eq!(branch_to_check("HEAD\n"), None);
        assert_eq!(branch_to_check("feat/BAR-2-x\n"), Some("feat/BAR-2-x"));
    }

    #[test]
    fn commit_subjects() {
        for ok in [
            "build(BAR-2): add the cargo workspace",
            "feat(BAR-1700): serve the trade log one page per request",
            "Merge pull request #3 from jackson-rod/docs/BAR-1-seed-docs",
            "Merge 1a2b3c into 4d5e6f",
            "Revert \"build(BAR-2): add the cargo workspace\"",
        ] {
            assert!(check_commit_message(ok).is_ok(), "{ok}");
        }
        let too_long = format!("docs(BAR-1): {}", "a".repeat(88)); // 101 characters
        for bad in [
            "add the workspace",
            "build: add the workspace",
            "build(BAR-2): Add the workspace",
            "build(bar-2): add",
            "wip(BAR-2): add",
            too_long.as_str(),
        ] {
            assert!(check_commit_message(bad).is_err(), "{bad}");
        }
    }

    #[test]
    fn only_real_merge_and_revert_subjects_are_exempt() {
        for ok in [
            "Merge pull request #3 from jackson-rod/docs/BAR-1-seed-docs",
            "Merge branch 'develop' into build/BAR-2-workspace-skeleton",
            "Merge remote-tracking branch 'origin/develop' into develop",
            "Merge 1a2b3c4 into 4d5e6f7",
            "Revert \"build(BAR-2): add the cargo workspace\"",
        ] {
            assert!(check_commit_message(ok).is_ok(), "{ok}");
        }
        for bad in [
            "Merge the two config loaders into one",
            "Revert the kernel change",
        ] {
            assert!(check_commit_message(bad).is_err(), "{bad}");
        }
    }

    #[test]
    fn ai_attribution_is_rejected() {
        for body in [
            "Co-Authored-By: Claude <noreply@anthropic.com>",
            "co-authored-by: Claude Fable 5.2 <noreply@anthropic.com>",
            "🤖 Generated with [Claude Code](https://claude.com/claude-code)",
            "Session: https://claude.ai/code/session_123",
        ] {
            let message = format!("build(BAR-2): add the workspace\n\n{body}\n");
            assert!(check_commit_message(&message).is_err(), "{body}");
        }
        assert!(check_commit_message("docs(BAR-1): explain the ADR format\n\nCo-Authored-By: Ana Gomez <ana@example.com>\n").is_ok());
    }

    #[test]
    fn ordinary_sentences_are_not_attribution() {
        for body in [
            "openapi.json is generated with utoipa and committed.",
            "The client is generated with openapi-typescript.",
            "See https://docs.claude.com/en/api/messages for the request shape.",
            "Generated with the release profile to measure the real cost.",
        ] {
            let message = format!("docs(BAR-3): describe the contract pipeline\n\n{body}\n");
            assert!(check_commit_message(&message).is_ok(), "{body}");
        }
    }

    #[test]
    fn attribution_in_its_other_forms_is_rejected() {
        for body in [
            "  Co-Authored-By: Claude <noreply@anthropic.com>",
            "Co-authored-by: Copilot <copilot@example.com>",
            "Co-authored-by: Cursor Agent <agent@example.com>",
            "Generated-by: Claude Code",
            "Assisted-by: Claude",
            "Signed-off-by: Claude <noreply@anthropic.com>",
            "Generated by Claude Code",
            "🤖 Generated with [Claude Code](https://claude.com/claude-code)",
            "https://claude.ai/code/session_123",
            "https://claude.ai/share/abc",
        ] {
            let message = format!("build(BAR-2): add the workspace\n\n{body}\n");
            assert!(check_commit_message(&message).is_err(), "{body}");
        }
    }

    #[test]
    fn a_marker_that_makes_github_skip_the_run_is_refused_anywhere_in_any_case() {
        for marker in [
            "[skip ci]",
            "[ci skip]",
            "[no ci]",
            "[skip actions]",
            "[actions skip]",
            "[Skip CI]",
            "[NO CI]",
            "[ skip  actions ]",
        ] {
            for message in [
                format!("docs(BAR-3): fix a typo in the readme {marker}\n"),
                format!(
                    "docs(BAR-3): fix a typo in the readme\n\nNothing to build here {marker}.\n"
                ),
            ] {
                assert!(check_commit_message(&message).is_err(), "{message}");
            }
        }
        for trailer in ["skip-checks: true", "skip-checks:true", "Skip-Checks: TRUE"] {
            let as_a_trailer = format!("docs(BAR-3): fix a typo in the readme\n\n\n{trailer}\n");
            let in_the_body = format!("docs(BAR-3): fix a typo\n\n{trailer} and more.\n");
            for message in [as_a_trailer, in_the_body] {
                assert!(check_commit_message(&message).is_err(), "{message}");
            }
        }
    }

    #[test]
    fn words_about_skipping_ci_are_not_a_marker() {
        for body in [
            "The docs job could skip CI work, but it does not.",
            "skip-checks: false",
            "A [skip] list and a [ci] note.",
        ] {
            let message = format!("docs(BAR-3): explain the CI\n\n{body}\n");
            assert!(check_commit_message(&message).is_ok(), "{body}");
        }
    }

    #[test]
    fn git_noise_is_not_part_of_the_message() {
        let raw = "build(BAR-2): add the workspace\r\n\r\n# Please enter the commit message\r\n# ------------------------ >8 ------------------------\r\ndiff --git a/x b/x\r\n+Generated with a tool\r\n";
        assert_eq!(strip_git_noise(raw), "build(BAR-2): add the workspace\n");
        assert!(check_commit_message(&strip_git_noise(raw)).is_ok());
    }

    #[test]
    fn only_the_scissors_line_cuts_the_message() {
        let raw = "perf(BAR-2): cache the quote lookup\n\np99 latency >8s before this change\n\nCo-Authored-By: Claude <noreply@anthropic.com>\n";
        let message = strip_git_noise(raw);
        assert!(message.contains("p99 latency >8s before this change"));
        assert!(check_commit_message(&message).is_err());
    }
}
