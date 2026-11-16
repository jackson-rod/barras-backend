# ADR-0018: Git workflow and ticket ids

Status: Accepted
Date: 2026-11-15

## Context

- The working rules come from the owner's earlier project, adapted to Rust (Decided, section 1). The Trello card number is the ticket id.
- That everything in the repositories is in English is a **Call** of the designer, not confirmed by the owner.
- `main` and `develop` carry released and integrated work, so nothing may reach them unchecked.

## Decision

- Branch from `develop` as `type/BAR-<n>-short-slug`, at most 50 characters. `<n>` is the Trello card number. Types: `feat`, `fix`, `refactor`, `perf`, `test`, `docs`, `chore`, `ci`, `build`.
- Commit subject: `type(BAR-<n>): short description starting in lower case`. One ticket per branch.
- No AI attribution in commits or pull requests.
- Pull requests target `develop`. `main` and `develop` are protected: no direct push, no force push, merge only when the checks pass.
- Agents never push, open pull requests or touch production unless the owner asks in that conversation.
- Everything in the repositories is in English.

## Consequences

- Cost: every change needs a ticket before it has a branch.
- lefthook runs the checks: `commit-msg` checks the message and `pre-push` checks the branch name and runs `just verify`. Bypassing the hooks is not allowed.
- CI repeats the branch name and commit message checks, and `xtask` implements them.
- Pointer: the commit and pull request checks also refuse the markers that make GitHub start no CI run: `[skip ci]`, `[ci skip]`, `[no ci]`, `[skip actions]` and `[actions skip]`, in any case and with any spaces inside the brackets, and a `skip-checks: true` trailer wherever it stands (`crates/xtask/src/git_rules.rs`). CI refuses one only in a run that starts: when the head commit of a pull request holds one, GitHub starts none, so there only the `commit-msg` hook refuses it, and required checks, below, would stay pending and block the merge.
- Pointer: GitHub enforces "merge only when the checks pass" only once `checks`, `test` and `build` are required status checks on `develop` and `main`, an open owner decision; until then it rests on whoever merges.
