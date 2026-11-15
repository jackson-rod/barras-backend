# Foundations Kickoff (BAR-1 and BAR-2) Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Both repositories carry the approved plan, standards and decisions (BAR-1), and the backend is a Cargo workspace whose layer, structure and git rules are enforced by checks and CI before any feature exists (BAR-2).

**Architecture:** BAR-1 is documentation only: the foundation design is committed and split into standards, ADRs, schema and analysis. BAR-2 creates a workspace of placeholder crates and an `xtask` crate that holds the repository checks as pure, tested functions, then wires them into `just`, lefthook and GitHub Actions.

**Tech Stack:** Rust 1.100.0 (edition 2024), Cargo workspace, cargo-nextest, cargo-deny, just, lefthook, GitHub Actions.

**Spec:** `docs/specs/2026-11-15-barras-foundation-design.md`

## Global Constraints

- Everything in the repositories is in English: code, comments, commits, pull requests, docs.
- Rust toolchain `1.100.0`, edition `2024`.
- Branch: `type/BAR-<n>-short-slug`, at most 50 characters, from `develop`. Types: `feat`, `fix`, `refactor`, `perf`, `test`, `docs`, `chore`, `ci`, `build`.
- Commit subject: `type(BAR-<n>): short description starting in lower case`. One ticket per branch.
- No AI attribution in commits or pull requests: no `Co-Authored-By` naming Claude or Anthropic, no "Generated with", no session links.
- Agents never push, open pull requests or change GitHub settings unless the owner asks in that conversation.
- `unsafe` is forbidden. clippy runs with zero warnings and denies `unwrap`, `expect`, `panic!`, `dbg!` and printing to stdout outside tests. No `#[allow]` without the reason on the same line.
- Comment why, never what (`docs/standards/code-standard.md`). Rust files are snake_case.
- A module crate may depend only on `barras-kernel`, `thiserror`, `uuid`, `time`, `rust_decimal` and `async-trait`.
- `just verify` and CI finish in 10 minutes or less.
- The Trello card number is the ticket id. Cards were created in spec §11 order, so F-01 is BAR-1 and L-02 is BAR-61.

## Review Focus

1. **A commit message file with git's own noise.** With `git commit -v`, or on Windows, the file has `#` comment lines, a scissors line followed by a diff, or CRLF endings. The check must judge only the message. Pinned in Task 5.
2. **A detached HEAD.** CI checks out a merge ref, so the current branch reads `HEAD`. The branch check must skip it, not fail. Pinned in Task 5.
3. **A renamed dependency.** `db = { package = "sqlx" }` in a module crate must still be caught by its real name. Pinned in Task 6.
4. **An optional or target-specific dependency.** `[target.'cfg(unix)'.dependencies]` or `optional = true` must not hide a forbidden dependency. Pinned in Task 6.
5. **Running a check from a subdirectory.** `cargo xtask check-structure` run inside `crates/kernel` must still find `docs/adr` at the repository root. Pinned in Task 7.

## Before starting

The session folder holds the inputs this plan copies into the repositories:

- `docs/specs/2026-11-15-barras-foundation-design.md` (the spec)
- `docs/plans/2026-11-15-bar-1-bar-2-foundations-kickoff.md` (this plan)
- `docs/standards/code-standard.md` (the owner's text)
- `docs/analysis/data-sources.md` and `docs/analysis/tooling-2026-11.md`

Three things need the owner, and nothing else in the plan can replace them:

- **A folder** for the two clones. Below it is called `<work>`.
- **A `develop` branch** on both GitHub repositories. Both have only `main`. Creating it means a push.
- **Rust.** It is not installed on this machine. Installing it is a download (rustup, from rust-lang.org).

The owner also sets, in GitHub, `develop` as the default branch and protection on `main` and `develop` (no direct push, no force push, merge only with green checks).

---

# Part A: BAR-1, seed the docs

Branch in each repository: `docs/BAR-1-seed-docs`.

### Task 1: Backend docs: plan, standards, schema, analysis, backlog

**Files** (in `<work>/barras-backend`):
- Create: `docs/README.md`
- Create (copied unchanged from the session folder): `docs/specs/2026-11-15-barras-foundation-design.md`, `docs/plans/2026-11-15-bar-1-bar-2-foundations-kickoff.md`, `docs/standards/code-standard.md`, `docs/analysis/data-sources.md`, `docs/analysis/tooling-2026-11.md`
- Create: `docs/standards/module-anatomy.md`, `docs/standards/performance.md`, `docs/standards/testing.md`
- Create: `docs/schema/barras-schema.md`, `docs/backlog/work-plan.md`

**Interfaces:**
- Consumes: the five input files listed in "Before starting".
- Produces: the `docs/` tree that Task 2 adds `docs/adr/` to, and that Task 7's structure check reads.

- [ ] **Step 1: Clone and branch**

```bash
cd <work>
git clone git@github.com:jackson-rod/barras-backend.git
cd barras-backend
git switch develop          # exists once the owner has asked for it to be created and pushed
git switch -c docs/BAR-1-seed-docs
```

Expected: `git branch --show-current` prints `docs/BAR-1-seed-docs`.

- [ ] **Step 2: Copy the five input files unchanged**

Verify each with `diff <session-folder>/<path> <work>/barras-backend/<path>`. Expected: no output.

- [ ] **Step 3: Write `docs/README.md`**

Sections, in this order:
1. `# Barras docs`: what Barras is, from spec §1, in five lines at most.
2. `## How we work`: the five steps of spec §12, unchanged.
3. `## Where things are`: one table row per folder of spec §10, each linking to its entry file.
4. `## Start here`: links to the foundation design, `backlog/work-plan.md` and `adr/README.md`.

- [ ] **Step 4: Write `docs/standards/module-anatomy.md`**

It restates spec §6.3, §6.4, §6.5 and §6.6 and must contain: the module tree; the adapter mirror paths; the dependency table; the three enforcement mechanisms; the "Reject on sight" list; ports as traits held as `Arc<dyn Port>`; the error rules. Add the naming rule this plan fixes: a module's package is `barras-<module>` and its folder is `crates/modules/<module>` in snake_case.

- [ ] **Step 5: Write `docs/standards/performance.md`**

The budget table, the rules and the launch sizing of spec §6.12, unchanged, under `## Budgets`, `## Rules` and `## Sizing for launch`.

- [ ] **Step 6: Write `docs/standards/testing.md`**

From spec §9: tests per layer (definition of done, item 1), the coverage numbers (95% of lines in module crates, 85% overall), and "assert errors by `code`, never by message". From spec §6.14: unit tests need no Docker; integration and e2e tests run against throwaway Postgres 18 and Redis 8 containers.

- [ ] **Step 7: Write `docs/schema/barras-schema.md`**

- `## Conventions`: ids are UUID v7; timestamps are `timestamptz` in UTC; fiat is `numeric(20,2)` and quantities `numeric(38,18)`, always with a currency (spec §6.9).
- One `##` per module of spec §8, with one `###` per table carrying the purpose given in the spec and the line "Columns are defined in this module's spec."
- `## Seeds`: the seed list of spec §8.

- [ ] **Step 8: Write `docs/backlog/work-plan.md`**

The table of spec §11 with a new first column `BAR`, numbered 1 to 61 in row order, followed by the "Notes on the order" of spec §11.

- [ ] **Step 9: Verify**

```bash
test "$(grep -c '^| [0-9]' docs/backlog/work-plan.md)" = 61 && echo rows-ok
grep -rnE 'TBD|TODO' docs --exclude-dir=plans --exclude-dir=specs ; test $? = 1 && echo no-placeholders
```

Expected: `rows-ok` and `no-placeholders`. The plan and the spec are excluded because they are copies, and this plan quotes those two words.

- [ ] **Step 10: Commit**

```bash
git add docs
git commit -m "docs(BAR-1): add the plan, standards, schema and analysis"
```

### Task 2: Backend ADRs and their index

**Files:**
- Create: `docs/adr/README.md`
- Create: `docs/adr/0001-…md` to `docs/adr/0023-…md`, named by the table below

**Interfaces:**
- Produces: the ADR file format and index format that `check_adrs` (Task 7) enforces.

Every ADR uses exactly this shape:

```markdown
# ADR-0003: Hexagonal layers enforced by crates

Status: Accepted
Date: 2026-11-15

## Context

## Decision

## Consequences
```

`README.md` holds one table row per ADR: `| [0003](0003-hexagonal-layers-enforced-by-crates.md) | Hexagonal layers enforced by crates | Accepted |`.

| File | Title | Records spec |
| --- | --- | --- |
| `0001-record-architecture-decisions.md` | Record architecture decisions | §10 |
| `0002-two-repositories-openapi-contract.md` | Two repositories with OpenAPI as the contract | §1, §6.7, §7.3 |
| `0003-hexagonal-layers-enforced-by-crates.md` | Hexagonal layers enforced by crates | §6.2 to §6.5 |
| `0004-two-deployables-one-workspace.md` | Two deployables from one workspace | §5, §6.1 |
| `0005-http-contract.md` | The HTTP contract | §6.7 |
| `0006-errors-as-typed-results.md` | Errors as typed results | §6.6 |
| `0007-data-access.md` | Data access with sqlx and hand-written SQL | §6.9 |
| `0008-money-time-and-ids.md` | Money, time and ids | §6.9 |
| `0009-authentication.md` | Authentication | §6.8 |
| `0010-authorisation-and-entitlements.md` | Authorisation and entitlements | §6.8 |
| `0011-realtime-gateway.md` | The realtime gateway | §6.10 |
| `0012-outside-systems-are-ports.md` | Every outside system is a port | §6.11 |
| `0013-ingestion-jobs.md` | Ingestion jobs under advisory locks | §6.1, §6.11 |
| `0014-performance-budgets.md` | Performance budgets and the dashboard read path | §6.12 |
| `0015-observability.md` | Logging, health and metrics | §6.13 |
| `0016-configuration.md` | Configuration | §6.13 |
| `0017-definition-of-done.md` | Definition of done | §9 |
| `0018-git-workflow.md` | Git workflow and ticket ids | §9 |
| `0019-frontend-stack-and-anatomy.md` | Frontend stack and feature anatomy | §5, §7 |
| `0020-market-data-sources.md` | Market data sources | §3, §11 |
| `0021-news-headline-and-link.md` | News as headline and link | §2, §3 |
| `0022-informational-suggestions.md` | Informational suggestions and chatbot | §2, §3, §4 |
| `0023-subscriptions-through-the-stores.md` | Subscriptions through the stores | §3, §4, §6.8 |

- [ ] **Step 1: Write the 23 ADRs**

Each has at most 40 lines. Context states the forces the spec gives. Decision restates the spec's rule in the imperative. Consequences say what it costs and which check enforces it. An ADR adds no decision the spec does not contain; where the spec marks a decision as a **Call**, the ADR says so in Context.

- [ ] **Step 2: Write `docs/adr/README.md`**

A one-paragraph introduction (what an ADR is, that an ADR outranks `CLAUDE.md`, how to add one) and the index table.

- [ ] **Step 3: Verify**

```bash
ls docs/adr/0*.md | wc -l                                   # 23
for f in docs/adr/0*.md; do sed -n 3p "$f" | grep -qx 'Status: Accepted' || echo "bad status: $f"; done
for f in docs/adr/0*.md; do grep -q "($(basename "$f"))" docs/adr/README.md || echo "not indexed: $f"; done
```

Expected: `23`, and no other output.

- [ ] **Step 4: Commit**

```bash
git add docs/adr
git commit -m "docs(BAR-1): record the foundation decisions as ADRs"
```

### Task 3: Frontend docs seed

**Files** (in `<work>/barras-frontend`, branch `docs/BAR-1-seed-docs` from `develop`):
- Create: `docs/README.md`
- Create (copied unchanged): `docs/standards/code-standard.md`
- Create: `docs/standards/module-anatomy.md`, `docs/standards/performance.md`, `docs/standards/testing.md`
- Create: `docs/adr/README.md`, `docs/adr/0001-record-architecture-decisions.md`, `docs/adr/0002-frontend-stack-and-feature-anatomy.md`, `docs/adr/0003-backend-openapi-is-the-contract.md`

**Interfaces:**
- Consumes: the ADR and index formats of Task 2.

- [ ] **Step 1: Clone and branch** as in Task 1, with `barras-frontend`.

- [ ] **Step 2: Write `docs/README.md`**

Three sections: what this repository is (the Barras mobile app; spec §7); `## How we work` (the five steps of spec §12); `## Product-wide docs`, linking to the foundation design, the schema and the ADR index in `https://github.com/jackson-rod/barras-backend/tree/develop/docs`.

- [ ] **Step 3: Write the three standards**

- `module-anatomy.md`: the layout, the feature list and the import table of spec §7.2.
- `performance.md`: the budget table of spec §7.3.
- `testing.md`: Jest with jest-expo and React Native Testing Library for `domain`, `application` and `ui`; Maestro for end-to-end flows (spec §7.1); coverage of 95% of lines in `domain` and `application`, 85% overall (spec §9).

- [ ] **Step 4: Write the three ADRs and the index**

`0002` records spec §5 (frontend) and §7. `0003` records spec §6.7 and §7.3: the vendored `api/openapi.json`, generated types, and that the app never decides entitlements.

- [ ] **Step 5: Verify**

```bash
ls docs/adr/0*.md | wc -l                                   # 3
diff docs/standards/code-standard.md ../barras-backend/docs/standards/code-standard.md && echo same
```

Expected: `3` and `same`.

- [ ] **Step 6: Commit**

```bash
git add docs
git commit -m "docs(BAR-1): seed the docs"
```

- [ ] **Step 7: Stop and ask the owner** whether to push both `docs/BAR-1-seed-docs` branches and open the pull requests into `develop`.

---

# Part B: BAR-2, backend workspace skeleton

Branch: `build/BAR-2-workspace-skeleton`, from `develop` after BAR-1 is merged. Task 7's test on the real repository reads `docs/adr`, so it fails without BAR-1.

Deferred, and written on their Trello cards: `.env.example`, `just test-int`, `just test-cov` with the coverage thresholds and cargo-hakari arrive with BAR-3; the database commands with BAR-4.

### Task 4: Workspace, toolchain and lints

**Files:**
- Create: `rust-toolchain.toml`, `Cargo.toml`, `Cargo.lock`, `clippy.toml`, `.cargo/config.toml`, `.gitignore`
- Create: `apps/api/Cargo.toml`, `apps/api/src/main.rs`, `apps/ingest/Cargo.toml`, `apps/ingest/src/main.rs`
- Create: `crates/kernel/Cargo.toml`, `crates/kernel/src/lib.rs`
- Create: `crates/adapters/{postgres,http,realtime,redis,providers}/Cargo.toml` and `src/lib.rs`
- Create: `crates/testkit/Cargo.toml`, `crates/testkit/src/lib.rs`
- Create: `crates/xtask/Cargo.toml`, `crates/xtask/src/main.rs`

**Interfaces:**
- Produces these package names, which Tasks 6 and 8 rely on:

| Path | Package | Kind |
| --- | --- | --- |
| `apps/api` | `barras-api` | binary |
| `apps/ingest` | `barras-ingest` | binary |
| `crates/kernel` | `barras-kernel` | library |
| `crates/adapters/postgres` | `barras-adapter-postgres` | library |
| `crates/adapters/http` | `barras-adapter-http` | library |
| `crates/adapters/realtime` | `barras-adapter-realtime` | library |
| `crates/adapters/redis` | `barras-adapter-redis` | library |
| `crates/adapters/providers` | `barras-adapter-providers` | library |
| `crates/testkit` | `barras-testkit` | library |
| `crates/xtask` | `xtask` | binary |

- [ ] **Step 1: Install the toolchain** (a download; needs the owner's go-ahead)

Install rustup from rust-lang.org, then `just` and `lefthook` (Homebrew). Expected after Step 2: `rustc --version` prints `rustc 1.100.0`.

- [ ] **Step 2: Write `rust-toolchain.toml`**

```toml
[toolchain]
channel = "1.100.0"
components = ["rustfmt", "clippy"]
profile = "minimal"
```

- [ ] **Step 3: Write the workspace `Cargo.toml`**

```toml
[workspace]
resolver = "3"
members = [
  "apps/api", "apps/ingest",
  "crates/kernel",
  "crates/adapters/postgres", "crates/adapters/http", "crates/adapters/realtime",
  "crates/adapters/redis", "crates/adapters/providers",
  "crates/testkit", "crates/xtask",
]

[workspace.package]
version = "0.1.0"
edition = "2024"
rust-version = "1.100"
publish = false

[workspace.dependencies]

[workspace.lints.rust]
unsafe_code = "forbid"

[workspace.lints.clippy]
unwrap_used = "deny"
expect_used = "deny"
panic = "deny"
todo = "deny"
unimplemented = "deny"
dbg_macro = "deny"
print_stdout = "deny"
print_stderr = "deny"
disallowed_methods = "deny"

[profile.dev]
debug = "line-tables-only"

[profile.release]
lto = "thin"
codegen-units = 1
```

Module crates are added to `members` one by one as they are created. `[workspace.dependencies]` gains an entry the first time a crate needs it; member crates always write `name.workspace = true`.

- [ ] **Step 4: Write `clippy.toml` and `.cargo/config.toml`**

```toml
# clippy.toml
allow-unwrap-in-tests = true
allow-expect-in-tests = true
allow-panic-in-tests = true
allow-dbg-in-tests = true
allow-print-in-tests = true
disallowed-methods = [
  { path = "std::env::var", reason = "read configuration through the typed Config of the binary" },
]
```

```toml
# .cargo/config.toml
[alias]
xtask = "run --quiet --package xtask --"
```

- [ ] **Step 5: Write the member crates**

Every member `Cargo.toml` has `version.workspace = true`, `edition.workspace = true`, `rust-version.workspace = true`, `publish.workspace = true`, `[lints] workspace = true`, and no dependencies yet. Each library's `src/lib.rs` holds only a one-line `//!` crate doc saying what the crate is for. Each binary's `main.rs` is `fn main() -> std::process::ExitCode { std::process::ExitCode::SUCCESS }`.

- [ ] **Step 6: Write `.gitignore`**

```gitignore
target/
.env
.env.*
!.env.example
.DS_Store
lcov.info
*.profraw
```

- [ ] **Step 7: Verify the build and the lints**

```bash
cargo build --workspace
cargo fmt --all --check
cargo clippy --workspace --all-targets --locked -- -D warnings
```

Expected: all three exit 0, and `Cargo.lock` now exists.

- [ ] **Step 8: Prove a denied lint fires**

Add `pub fn probe() -> u8 { None::<u8>.unwrap() }` to `crates/kernel/src/lib.rs` and run the clippy command of Step 7. Expected: FAIL naming `clippy::unwrap_used`. Remove the line and run it again. Expected: exit 0.

- [ ] **Step 9: Commit**

```bash
git add -A
git commit -m "build(BAR-2): add the cargo workspace, toolchain and lints"
```

### Task 5: xtask: branch and commit rules

**Files:**
- Create: `crates/xtask/src/git_rules.rs` (tests in the same file)
- Modify: `crates/xtask/src/main.rs`, `crates/xtask/Cargo.toml`, `Cargo.toml` (`[workspace.dependencies]`: `regex`)

**Interfaces:**
- Produces:
  - `pub fn check_branch_name(name: &str) -> Result<(), String>`
  - `pub fn branch_to_check(raw: &str) -> Option<&str>`: `None` for `HEAD`, otherwise the trimmed name
  - `pub fn strip_git_noise(raw: &str) -> String`
  - `pub fn check_commit_message(message: &str) -> Result<(), String>`
  - CLI: `cargo xtask check-branch [name]`, `cargo xtask check-commit-msg <file>`, `cargo xtask check-commits <range>`; exit code 1 and one line per problem on stderr

- [ ] **Step 1: Write the failing tests**

```rust
#[test]
fn branch_names() {
    let exactly_50 = format!("feat/BAR-2-{}", "a".repeat(39));
    let over_50 = format!("feat/BAR-2-{}", "a".repeat(40));
    for ok in ["main", "develop", "build/BAR-2-workspace-skeleton", "feat/BAR-1700-trade-log-cursor", exactly_50.as_str()] {
        assert!(check_branch_name(ok).is_ok(), "{ok}");
    }
    for bad in ["feature/BAR-2-x", "feat/bar-2-x", "feat/BAR-2_x", "feat/BAR-2-X", "feat/BAR-x", "feat/BAR-2-", "BAR-2-x", over_50.as_str()] {
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
    for bad in ["add the workspace", "build: add the workspace", "build(BAR-2): Add the workspace", "build(bar-2): add", "wip(BAR-2): add", too_long.as_str()] {
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
fn git_noise_is_not_part_of_the_message() {
    let raw = "build(BAR-2): add the workspace\r\n\r\n# Please enter the commit message\r\n# ------------------------ >8 ------------------------\r\ndiff --git a/x b/x\r\n+Generated with a tool\r\n";
    assert_eq!(strip_git_noise(raw), "build(BAR-2): add the workspace\n");
    assert!(check_commit_message(&strip_git_noise(raw)).is_ok());
}
```

- [ ] **Step 2: Run them to see them fail**

Run: `cargo nextest run -p xtask`
Expected: FAIL to compile, the four functions are not defined.

- [ ] **Step 3: Implement the four functions in `git_rules.rs`**

- Branch pattern: `^(feat|fix|refactor|perf|test|docs|chore|ci|build)/BAR-[0-9]+-[a-z0-9]+(-[a-z0-9]+)*$`; `main` and `develop` pass; length at most 50.
- Subject pattern: `^(feat|fix|refactor|perf|test|docs|chore|ci|build)\(BAR-[0-9]+\): [a-z].*$`; at most 100 characters; subjects starting with `Merge ` or `Revert "` pass.
- Attribution, case-insensitive, anywhere in the message: a `co-authored-by:` line containing `claude` or `anthropic`; `generated with`; `claude.ai/`; `claude.com/`.
- `strip_git_noise`: normalise CRLF to LF, cut at the first line containing `>8`, drop lines starting with `#`, trim trailing blank lines, end with one `\n`.

Each error message names the rule and shows the offending text.

- [ ] **Step 4: Wire the three subcommands in `main.rs`**

- `check-branch [name]`: without an argument, read `git rev-parse --abbrev-ref HEAD`; pass it through `branch_to_check`; `None` exits 0.
- `check-commit-msg <file>`: read the file, `strip_git_noise`, `check_commit_message`.
- `check-commits <range>`: for each hash of `git log --format=%H <range>`, check `git log -1 --format=%B <hash>`.

`main.rs` starts with `#![allow(clippy::print_stdout, clippy::print_stderr)] // a CLI: its output is the product`. An unknown subcommand prints the list and exits 2.

- [ ] **Step 5: Run the tests**

Run: `cargo nextest run -p xtask`
Expected: PASS, 5 tests.

- [ ] **Step 6: Try the CLI**

```bash
cargo xtask check-branch; echo "exit $?"                 # exit 0 on build/BAR-2-workspace-skeleton
cargo xtask check-branch feature/x; echo "exit $?"       # exit 1
```

- [ ] **Step 7: Commit**

```bash
git add -A
git commit -m "feat(BAR-2): check branch names and commit messages"
```

### Task 6: xtask: check-arch

**Files:**
- Create: `crates/xtask/src/arch.rs` (unit tests in the same file)
- Create: `crates/xtask/tests/arch_workspace.rs`
- Modify: `crates/xtask/src/main.rs`, `crates/xtask/Cargo.toml`, `Cargo.toml` (`cargo_metadata`, `thiserror`; dev: `tempfile`)

**Interfaces:**
- Consumes: the package names of Task 4.
- Produces:

```rust
pub enum Layer { Kernel, Module, Adapter, App, Testkit, Tool }
pub enum DepKind { Normal, Dev, Build }
pub struct Dep { pub name: String, pub kind: DepKind, pub is_workspace: bool }  // name is the real package name, never the rename
pub struct CrateInfo { pub name: String, pub dir: PathBuf, pub deps: Vec<Dep> } // dir is relative to the workspace root
pub struct Violation { pub krate: String, pub dependency: String, pub rule: &'static str }

pub fn layer_of(dir: &Path) -> Option<Layer>
pub fn check_arch(crates: &[CrateInfo]) -> Vec<Violation>
pub fn load_workspace(root: &Path) -> Result<Vec<CrateInfo>, XtaskError>
pub fn workspace_root() -> PathBuf
```

- `XtaskError` (in `main.rs` or `error.rs`): `Io(std::io::Error)`, `Metadata(String)`, `Git(String)`, with thiserror.
- CLI: `cargo xtask check-arch`.

Rules, with the `rule` string each violation carries:

| Layer (by `dir`) | Rule | `rule` |
| --- | --- | --- |
| `crates/kernel` | normal and build deps only from `thiserror`, `uuid`, `time`, `rust_decimal` | `kernel-allowlist` |
| `crates/modules/<name>` | normal and build deps only from `barras-kernel` and the kernel list plus `async-trait` | `module-allowlist` |
| `crates/modules/<name>` | dev deps only from the module list plus `tokio`, `mockall`, `insta` | `module-dev-allowlist` |
| `crates/adapters/<name>` | no dependency of any kind on another `crates/adapters/*` crate | `adapter-isolation` |
| any | `barras-testkit` only as a dev dependency | `testkit-dev-only` |
| `crates/xtask` | no workspace dependency | `tool-isolation` |
| anything else except `apps/<name>` and `crates/testkit` | the crate itself is the violation, with an empty `dependency` | `unknown-location` |

- [ ] **Step 1: Write the failing unit tests in `arch.rs`**

A helper `krate(name, dir, deps: &[(&str, DepKind, bool)]) -> CrateInfo` builds inputs. `rules(&violations)` returns the `(krate, dependency, rule)` triples.

```rust
#[test]
fn the_empty_skeleton_is_clean() {
    let crates = [
        krate("barras-kernel", "crates/kernel", &[("thiserror", Normal, false)]),
        krate("barras-identity", "crates/modules/identity", &[("barras-kernel", Normal, true), ("async-trait", Normal, false), ("tokio", Dev, false)]),
        krate("barras-adapter-postgres", "crates/adapters/postgres", &[("barras-identity", Normal, true), ("sqlx", Normal, false)]),
        krate("barras-api", "apps/api", &[("barras-adapter-postgres", Normal, true), ("barras-testkit", Dev, true)]),
        krate("barras-testkit", "crates/testkit", &[("sqlx", Normal, false)]),
        krate("xtask", "crates/xtask", &[("regex", Normal, false)]),
    ];
    assert!(check_arch(&crates).is_empty());
}

#[test]
fn a_module_cannot_reach_a_framework_or_another_module() {
    let crates = [krate("barras-identity", "crates/modules/identity", &[
        ("sqlx", Normal, false), ("axum", Normal, false), ("barras-capital", Normal, true), ("sqlx", Dev, false),
    ])];
    assert_eq!(rules(&check_arch(&crates)), [
        ("barras-identity", "sqlx", "module-allowlist"),
        ("barras-identity", "axum", "module-allowlist"),
        ("barras-identity", "barras-capital", "module-allowlist"),
        ("barras-identity", "sqlx", "module-dev-allowlist"),
    ]);
}

#[test]
fn the_other_rules() {
    let crates = [
        krate("barras-kernel", "crates/kernel", &[("serde", Normal, false)]),
        krate("barras-adapter-postgres", "crates/adapters/postgres", &[]),
        krate("barras-adapter-http", "crates/adapters/http", &[("barras-adapter-postgres", Normal, true)]),
        krate("barras-api", "apps/api", &[("barras-testkit", Normal, true)]),
        krate("xtask", "crates/xtask", &[("barras-kernel", Normal, true)]),
        krate("barras-misc", "crates/misc", &[]),
    ];
    assert_eq!(rules(&check_arch(&crates)), [
        ("barras-kernel", "serde", "kernel-allowlist"),
        ("barras-adapter-http", "barras-adapter-postgres", "adapter-isolation"),
        ("barras-api", "barras-testkit", "testkit-dev-only"),
        ("xtask", "barras-kernel", "tool-isolation"),
        ("barras-misc", "", "unknown-location"),
    ]);
}
```

- [ ] **Step 2: Write the failing workspace tests in `tests/arch_workspace.rs`**

```rust
#[test]
fn the_real_workspace_is_clean() {
    let crates = load_workspace(&workspace_root()).unwrap();
    assert_eq!(check_arch(&crates).len(), 0);
}

#[test]
fn a_renamed_optional_target_specific_dependency_is_still_caught() {
    // A temp workspace with `crates/modules/identity` (package barras-identity) and a local
    // stub package named `sqlx` at `vendor/sqlx`. The module declares it as:
    //   [target.'cfg(unix)'.dependencies]
    //   db = { package = "sqlx", path = "../../../vendor/sqlx", optional = true }
    // Each stub crate needs an empty src/lib.rs, or cargo refuses the manifest.
    let root = temp_workspace_with_hidden_sqlx();
    let violations = check_arch(&load_workspace(root.path()).unwrap());
    assert!(violations.iter().any(|v| v.krate == "barras-identity" && v.dependency == "sqlx" && v.rule == "module-allowlist"));
}
```

- [ ] **Step 3: Run them to see them fail**

Run: `cargo nextest run -p xtask`
Expected: FAIL to compile, `arch` is not defined.

- [ ] **Step 4: Implement `arch.rs`**

- `load_workspace` runs `cargo_metadata::MetadataCommand` with `no_deps()` in `root`. `Dep.name` comes from the dependency's `name` (the real package), never from `rename`. `is_workspace` is true when the dependency has a `path`. Optional and target-specific dependencies are kept like any other.
- `workspace_root` is two levels above `env!("CARGO_MANIFEST_DIR")`.
- `check_arch` reports violations in input order, then dependency order.
- A dependency's layer is found by looking its name up among `crates`. `barras-testkit` is recognised by its package name.

- [ ] **Step 5: Wire `check-arch` in `main.rs`**

Print one line per violation as `<krate>: <dependency> (<rule>)` and exit 1; exit 0 when there are none.

- [ ] **Step 6: Run the tests and the command**

Run: `cargo nextest run -p xtask && cargo xtask check-arch; echo "exit $?"`
Expected: PASS, and `exit 0`.

- [ ] **Step 7: Commit**

```bash
git add -A
git commit -m "feat(BAR-2): check the dependency rule between crates"
```

### Task 7: xtask: check-structure

**Files:**
- Create: `crates/xtask/src/structure.rs` (unit tests in the same file)
- Create: `crates/xtask/tests/structure_repo.rs`
- Modify: `crates/xtask/src/main.rs`

**Interfaces:**
- Consumes: `XtaskError` and `workspace_root()` from Task 6; the ADR format of Task 2.
- Produces (each returns one message per violation):

```rust
pub fn check_adrs(adr_dir: &Path) -> Result<Vec<String>, XtaskError>
pub fn check_modules(modules_dir: &Path) -> Result<Vec<String>, XtaskError>
pub fn check_migrations(migrations_dir: &Path) -> Result<Vec<String>, XtaskError>
pub fn check_structure(repo_root: &Path) -> Result<Vec<String>, XtaskError>
```

- CLI: `cargo xtask check-structure`.

Rules:

- **ADRs** (`docs/adr`, required): every file other than `README.md` is named `^\d{4}-[a-z0-9]+(-[a-z0-9]+)*\.md$`; line 1 is `# ADR-NNNN: ` with the file's number; one line is `Status: Proposed`, `Status: Accepted` or `Status: Superseded by ADR-NNNN`; numbers run from 0001 with no gap; every file is linked from `README.md` as `(filename)`; `README.md` links no ADR file that does not exist. Lines are compared after trimming a trailing `\r`.
- **Modules** (`crates/modules`, optional): every directory is snake_case and has `README.md`, `src/lib.rs`, `src/domain/errors.rs`, a `src/ports` directory and a `src/application` directory. Every `.rs` file directly in `src/application` other than `mod.rs`, and every `.rs` file directly in `src/domain` other than `mod.rs` and `errors.rs`, contains `#[cfg(test)]`.
- **Migrations** (`migrations`, optional): every file is named `^\d{14}_[a-z0-9_]+\.sql$`.

- [ ] **Step 1: Write the failing unit tests in `structure.rs`**

Helpers: `adr(n, slug)` returns valid ADR text; `write(root, path, text)` creates a file under a `tempfile::TempDir`.

```rust
#[test]
fn a_valid_adr_folder_is_clean() {
    // 0001-a.md and 0002-b.md, both valid, both linked from README.md
    assert!(check_adrs(dir.path()).unwrap().is_empty());
}

#[test]
fn adr_violations_are_reported() {
    // README.md links 0001-a.md and 0009-ghost.md.
    // Files: 0001-a.md (valid, CRLF line endings), 0003-c.md (valid, not linked),
    //        0004-d.md with "Status: Draft", 0005-e.md whose first line is "# Something",
    //        Notes.md
    let found = check_adrs(dir.path()).unwrap().join("\n");
    for expected in ["0003-c.md is not listed", "0009-ghost.md does not exist", "0002 is missing", "0004-d.md: status", "0005-e.md: title", "Notes.md: name"] {
        assert!(found.contains(expected), "{expected}\n{found}");
    }
    assert!(!found.contains("0001-a.md"));
}

#[test]
fn module_violations_are_reported() {
    // identity/: complete; src/application/register_user.rs and src/domain/user.rs both contain #[cfg(test)]
    // capital/: no README.md, no src/domain/errors.rs; src/application/add_account.rs without #[cfg(test)]
    // BadName/: empty
    let found = check_modules(dir.path()).unwrap().join("\n");
    for expected in ["capital: README.md", "capital: src/domain/errors.rs", "capital: src/application/add_account.rs has no tests", "BadName: name"] {
        assert!(found.contains(expected), "{expected}\n{found}");
    }
    assert!(!found.contains("identity"));
}

#[test]
fn migration_names() {
    // 20261115120000_create_users.sql, create_users.sql, 20261115120000_CreateUsers.sql
    let found = check_migrations(dir.path()).unwrap();
    assert_eq!(found.len(), 2);
}

#[test]
fn missing_optional_folders_are_fine_but_adrs_are_required() {
    // an empty repo root
    let found = check_structure(dir.path()).unwrap().join("\n");
    assert!(found.contains("docs/adr is missing"));
    assert!(!found.contains("modules"));
    assert!(!found.contains("migrations"));
}
```

- [ ] **Step 2: Write the failing repository test in `tests/structure_repo.rs`**

```rust
#[test]
fn the_real_repository_is_clean_from_any_directory() {
    // workspace_root() does not depend on the current directory
    std::env::set_current_dir(workspace_root().join("crates/kernel")).unwrap();
    assert_eq!(check_structure(&workspace_root()).unwrap(), Vec::<String>::new());
}
```

- [ ] **Step 3: Run them to see them fail**

Run: `cargo nextest run -p xtask`
Expected: FAIL to compile, `structure` is not defined.

- [ ] **Step 4: Implement `structure.rs` and wire `check-structure` in `main.rs`**

`main.rs` calls `check_structure(&workspace_root())`, prints one line per violation and exits 1; exit 0 when there are none.

- [ ] **Step 5: Run the tests and the command from a subdirectory**

Run: `cargo nextest run -p xtask && (cd crates/kernel && cargo xtask check-structure; echo "exit $?")`
Expected: PASS, and `exit 0`.

- [ ] **Step 6: Commit**

```bash
git add -A
git commit -m "feat(BAR-2): check the structure of docs, modules and migrations"
```

### Task 8: Commands, hooks, cargo-deny and CI

**Files:**
- Create: `justfile`, `lefthook.yml`, `deny.toml`
- Create: `.github/workflows/ci.yml`, `.github/pull_request_template.md`

**Interfaces:**
- Consumes: the `cargo xtask` subcommands of Tasks 5 to 7; the package names of Task 4.
- Produces: the `just` recipes that `CLAUDE.md` and `README.md` (Task 9) document.

- [ ] **Step 1: Write the `justfile`**

```just
set shell := ["bash", "-euo", "pipefail", "-c"]

default:
    @just --list

setup:
    rustup show active-toolchain
    cargo install --locked cargo-nextest cargo-deny
    lefthook install

dev:
    cargo run -p barras-api

dev-ingest:
    cargo run -p barras-ingest

lint:
    cargo clippy --workspace --all-targets --locked -- -D warnings

fmt:
    cargo fmt --all

fmt-check:
    cargo fmt --all --check

deny:
    cargo deny check

check-arch:
    cargo deny check bans
    cargo xtask check-arch

check-structure:
    cargo xtask check-structure

test:
    cargo nextest run --workspace --locked

build:
    cargo build --workspace --release --locked

verify: fmt-check lint deny check-arch check-structure test build
```

- [ ] **Step 2: Write `deny.toml`**

It must express, in the syntax of the installed cargo-deny:
- Licences allowed: `MIT`, `Apache-2.0`, `Apache-2.0 WITH LLVM-exception`, `BSD-2-Clause`, `BSD-3-Clause`, `ISC`, `Unicode-3.0`, `Zlib`. Private (unpublished) workspace crates are ignored.
- Sources: unknown registries and unknown git sources are denied.
- Bans, each with its allowed direct dependents (`wrappers`):

| Banned crate | May be depended on directly by |
| --- | --- |
| `sqlx` | `barras-adapter-postgres`, `barras-testkit` |
| `axum` | `barras-adapter-http`, `barras-adapter-realtime` |
| `reqwest` | `barras-adapter-providers` |
| `redis` | `barras-adapter-redis`, `barras-testkit` |

When a third-party crate that legitimately wraps one of these arrives later (for example `utoipa-axum`), it is added to that row with a comment that says why.

- [ ] **Step 3: Write `lefthook.yml`**

```yaml
pre-commit:
  commands:
    fmt:
      run: just fmt-check
    lint:
      run: just lint
    arch:
      run: just check-arch
    structure:
      run: just check-structure
commit-msg:
  commands:
    message:
      run: cargo xtask check-commit-msg {1}
pre-push:
  commands:
    branch:
      run: cargo xtask check-branch
    verify:
      run: just verify
```

- [ ] **Step 4: Write `.github/workflows/ci.yml`**

Requirements:
- Triggers: `pull_request`, and `push` to `develop` and `main`. One run per ref; a newer run cancels the older.
- Three parallel jobs named exactly `checks`, `test` and `build`, on `ubuntu-latest`.
- Every job: checkout, the toolchain from `rust-toolchain.toml`, sccache as `RUSTC_WRAPPER`.
- `checks`: checkout with full history. On pull requests only, `cargo xtask check-branch "$BRANCH_NAME"` and `cargo xtask check-commits "origin/$BASE_REF..HEAD"`. Both values reach the shell through `env:` (`BRANCH_NAME: ${{ github.head_ref }}`, `BASE_REF: ${{ github.base_ref }}`), never by writing `${{ … }}` inside `run:`, because a branch name is chosen by whoever opens the pull request. Then `just fmt-check lint deny check-arch check-structure`.
- `test`: `just test`.
- `build`: `just build`.
- Tools (`just`, `cargo-nextest`, `cargo-deny`) come from a prebuilt-binary install action. Each action is pinned to its current major version.

- [ ] **Step 5: Write `.github/pull_request_template.md`**

Sections: `## Ticket` (BAR-n and the Trello link), `## What and why`, `## Definition of done` with the four checkboxes of spec §9, `## Notes for the reviewer`.

- [ ] **Step 6: Install the hooks and run everything**

```bash
just setup
just verify; echo "exit $?"
```

Expected: `exit 0`. Note the wall-clock time; it must be under 10 minutes.

- [ ] **Step 7: Prove the hooks reject**

```bash
git commit --allow-empty -m "added stuff"; echo "exit $?"          # exit 1, the message names the subject rule
git commit --allow-empty -m "build(BAR-2): add the commands" -m "Co-Authored-By: Claude <noreply@anthropic.com>"; echo "exit $?"   # exit 1
```

Expected: both commits are refused and `git log -1` still shows the Task 7 commit.

- [ ] **Step 8: Commit**

```bash
git add -A
git commit -m "ci(BAR-2): add the commands, git hooks, cargo-deny and the workflow"
```

### Task 9: CLAUDE.md, README.md, CONTRIBUTING.md

**Files:**
- Create: `CLAUDE.md`, `CONTRIBUTING.md`
- Modify: `README.md` (today it holds only a title)

**Interfaces:**
- Consumes: the recipes of Task 8; the docs of Part A.

- [ ] **Step 1: Write `CLAUDE.md`**

It opens with: these rules bind every contributor, human or agent; they restate the accepted ADRs in `docs/adr/`; when a rule and an ADR disagree, the ADR wins and this file is fixed in the same pull request; everything in this repository is in English. Then, in this order:

| Section | Content |
| --- | --- |
| What this repository is | The Barras backend; two deployables from one workspace (spec §6.1); "Performance is the product" with the dashboard rules of spec §6.12; where the plan lives |
| Commands | A table of the recipes that exist today (Task 8), and the hooks paragraph of spec §6.14, ending "Never bypass them (`--no-verify` is not allowed)" |
| Repository layout | The tree of spec §6.2 |
| Module anatomy | The tree of spec §6.3, the dependency table and "Reject on sight" of spec §6.4, ports and wiring of §6.5 |
| Rules of the API | Subsections: contract with the app (§6.7); errors (§6.6); validation; data access (§6.9); every outside system is a port (§6.11); realtime (§6.10); logging and observability, and configuration (§6.13) |
| Definition of done | Spec §9, the four items |
| Code style | Spec §9 code style, pointing to `docs/standards/code-standard.md` |
| Git and pull requests | Spec §9, including no AI attribution and the rule for agents |
| When something is not covered here | Spec §9, last paragraph |

Rules for features that do not exist yet (realtime, data access) are written as rules, in the present tense, with their ADR number.

- [ ] **Step 2: Write `README.md`**

Sections: one-paragraph description and the "keep this current" note; `## What you need` (Rust 1.100 through rustup, just, lefthook, cargo-nextest, cargo-deny, each with the version installed in Task 4 and Task 8; Postgres 18, Redis 8 and Docker marked "from BAR-4"); `## First run` (`git clone`, `just setup`, `just verify`); `## Everyday commands`; `## Branches, pull requests and CI` with the three checks; `## Where things are`; `## Stack`.

- [ ] **Step 3: Write `CONTRIBUTING.md`**

How we work day to day: tickets live on the Trello board BARRAS and the card number is the `BAR-<n>` id; the spec-driven workflow of spec §12; branch and commit rules with one good and one bad example each; how a pull request is opened and merged.

- [ ] **Step 4: Verify**

```bash
for recipe in $(just --summary); do grep -q "just $recipe" CLAUDE.md || echo "missing in CLAUDE.md: $recipe"; done
just verify; echo "exit $?"
```

Expected: no "missing" line (the `default` recipe may be reported and is ignored), and `exit 0`.

- [ ] **Step 5: Commit**

```bash
git add -A
git commit -m "docs(BAR-2): add CLAUDE.md, README and CONTRIBUTING"
```

- [ ] **Step 6: Stop and ask the owner** whether to push `build/BAR-2-workspace-skeleton` and open the pull request. CI is only proven once that pull request runs: record that the three jobs `checks`, `test` and `build` are green, and fix the workflow on the same branch if one is not.
