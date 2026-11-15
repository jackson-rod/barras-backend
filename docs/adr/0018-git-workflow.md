# ADR-0018: Git workflow and ticket ids

Status: Accepted
Date: 2026-11-15

## Context

- The working rules come from the owner's earlier project, adapted to Rust (Decided, section 1). The Trello card number is the ticket id.
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
