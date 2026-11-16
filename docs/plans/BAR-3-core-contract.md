# BAR-3 Core Contract Implementation Plan

> **For agentic workers:** implement task by task, test first. Load the matching skill of the `agent-skills` plugin before a task (`test-driven-development` and `incremental-implementation` always; the task names any other). Each task ends with a review by `agent-skills:code-reviewer`; a task marked **gate** also gets `agent-skills:security-auditor`. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** The contract every module and route builds on exists and is enforced: typed errors, the response envelopes, typed configuration, JSON logs, health and metrics, the OpenAPI pipeline, the e2e harness and the coverage thresholds.

**Architecture:** The kernel gains the shared domain types. The HTTP adapter owns axum and exports everything the composition roots and the tests need, so no app names an axum type. A new crate, `barras-bootstrap`, holds the start-up both binaries share. Both binaries become real processes built from a library target that `main.rs` and the tests share.

**Tech stack:** Rust 1.100.0, axum 0.8.9, tokio 1.54.1, tower-http 0.7.1, utoipa 6.0.0 with utoipa-axum 0.3.0, garde 0.23.0, prometheus 0.14.0, tracing-subscriber 0.3.23, cargo-llvm-cov 0.9.1, cargo-hakari 0.9.39, watchexec 2.8.0.

**Spec:** [`docs/specs/BAR-3-core-contract.md`](../specs/BAR-3-core-contract.md), approved by the owner on 2026-11-15. Where this plan and the spec disagree, the spec wins, except for the two refinements listed under "Refinements of the spec".

## Global constraints

- Branch `feat/BAR-3-core-contract`, already created from `develop`. Never push, never open a pull request, never change git config, never bypass a hook (`--no-verify`, `LEFTHOOK=0`).
- Commit subject: `type(BAR-3): description starting in lower case`, at most 100 characters. No AI attribution anywhere in a commit: no `Co-authored-by`, no "Generated with".
- Everything in the repository is in English.
- `unsafe` is forbidden. clippy denies `unwrap`, `expect`, `panic!`, `dbg!` and printing outside tests, and `todo!` and `unimplemented!` everywhere. No `#[allow]` without the reason on the same line. An integration-test file starts with `#![allow(clippy::unwrap_used, clippy::expect_used)] // integration test crate: every item is test code`.
- A dependency is declared once in `[workspace.dependencies]` with its version, and taken with `<name>.workspace = true`, adding only the features the crate uses. Cargo 1.100 warns about a dependency no code uses and `just lint` denies warnings, so a commit never declares a dependency it does not use yet.
- After a dependency change run a plain `cargo build` to refresh `Cargo.lock`: the checks run with `--locked`.
- Comment why, never what. One primary export per file, the file named after it.
- Errors are asserted by `code`, never by message.
- A message, a log line or an error never repeats a value the caller sent.
- Shell commands need `export PATH="/opt/homebrew/opt/rustup/bin:$HOME/.cargo/bin:$PATH"`.

## Review focus

1. **A value echoed back.** serde quotes the offending value in its messages (`invalid type: string "abc"`). `error.fields`, configuration errors and logs must never carry it. Pinned in Tasks 3 and 7.
2. **The request id on every response.** A 404, a 405, a 413 and a panic must carry `x-request-id`. It depends on the order of the layers (appendix A.3). Pinned in Task 6.
3. **A route that skips the door.** Nothing but `ApiRouter::route` may add a route to the public API, and the guard must fail on one that does. Pinned in Tasks 5 and 6.
4. **A child process without its coverage file.** The process helper clears the environment; it must pass `LLVM_PROFILE_FILE` through, or the binaries' `main.rs` is measured as never run and a stray `.profraw` lands in the crate folder. Pinned in Task 10.
5. **Secrets in the request log.** tower-http's default span records the whole URI, query string included. The span is ours: method, route template, request id. Pinned in Task 8.
6. **Unstable bytes.** utoipa keeps extensions in a `HashMap`; `openapi.json` must be byte-identical between runs. Pinned in Task 5.
7. **Thresholds that measure tests.** cargo-llvm-cov counts inline `#[cfg(test)]` modules as covered lines. `check-coverage` must leave them out. Pinned in Task 2.

## Refinements of the spec

1. The spec writes `build_app(&ApiConfig, Dependencies)`. Nothing in this ticket's wiring reads the configuration, and `barras-api openapi` must build the app with no environment at all, so the function is `build_app(Dependencies)`. A later ticket that needs a setting in the wiring adds it to `Dependencies`.
2. The spec gives `barras-bootstrap` a function `shutdown_signal()`. The handlers must be installed before anything is served, so that a failure is a start-up error: it is `ShutdownSignal::install()` followed by `recv()`.

## Task order

| # | Task | Reviewers |
| --- | --- | --- |
| 1 | Kernel types | code |
| 2 | `xtask`: layer rules and `check-coverage` | code, **gate** |
| 3 | `barras-bootstrap` | code |
| 4 | HTTP adapter: envelopes and errors | code |
| 5 | HTTP adapter: `ApiRouter` and the OpenAPI document | code |
| 6 | HTTP adapter: the app, its middleware and the test client | code |
| 7 | HTTP adapter: validating extractors | code |
| 8 | HTTP adapter: request log and metrics | code |
| 9 | HTTP adapter: health, internal port, server | code |
| 10 | `barras-testkit`: process helper | code |
| 11 | `barras-api`: configuration, wiring, OpenAPI mode, e2e | code |
| 12 | `barras-ingest`: process and e2e | code |
| 13 | Commands, CI and cargo-deny | code, **gate** |
| 14 | cargo-hakari | code, **gate** |
| 15 | Docs and ADRs | code |
| 16 | Whole-branch verification and review | code, **gate** |

Tasks run one at a time, in this order, in the one working tree. Each task ends with `just fmt-check`, `just lint`, `just check-arch`, `just check-structure` and `just test` green (the hooks run the first four on every commit), then its commit.

---

### Task 1: Kernel types

Skill: `api-and-interface-design`.

**Files**
- Modify: `Cargo.toml` (`uuid`, `time`, `rust_decimal` in `[workspace.dependencies]`), `crates/kernel/Cargo.toml`, `crates/kernel/src/lib.rs`
- Create: `crates/kernel/src/{error,currency,money,id,page,entitlements,clock}.rs`

**Dependencies:** `thiserror`, `uuid` (feature `v7`), `time`, `rust_decimal` (`default-features = false`, feature `std`: no serde in the kernel). A feature `testing` with no dependency.

**Interfaces** (spec §4.1, §6.2)

```rust
// error.rs
pub enum ErrorKind { Validation, Unauthenticated, Forbidden, NotFound, Conflict, RateLimited, Unavailable, Internal }
pub trait AppError: std::error::Error + Send + Sync + 'static {
    fn code(&self) -> &'static str;
    fn kind(&self) -> ErrorKind;
}
pub enum KernelError { InvalidCurrency, InvalidAmount, InvalidId, InvalidPageLimit, InvalidCursor,
                       CurrencyMismatch { left: Currency, right: Currency }, AmountOverflow }

// currency.rs
impl Currency { pub const COP: Self; pub const USD: Self;
                pub fn new(code: &str) -> Result<Self, KernelError>; pub fn as_str(&self) -> &str; }

// money.rs
impl Money { pub fn new(amount: Decimal, currency: Currency) -> Result<Self, KernelError>;
             pub fn parse(amount: &str, currency: &str) -> Result<Self, KernelError>;
             pub fn zero(currency: Currency) -> Self;
             pub fn amount(&self) -> Decimal; pub fn currency(&self) -> Currency; pub fn is_negative(&self) -> bool;
             pub fn checked_add(self, other: Self) -> Result<Self, KernelError>;
             pub fn checked_sub(self, other: Self) -> Result<Self, KernelError>; }

// id.rs
define_id!(UserId);   // generate(), from_uuid, as_uuid, parse(&str) -> Result<Self, KernelError>, Display

// page.rs
impl PageLimit { pub const DEFAULT: Self; pub const MAX: u32 = 50; pub fn new(value: u32) -> Result<Self, KernelError>; pub fn get(self) -> u32; }
impl Cursor { pub fn new(token: impl Into<String>) -> Result<Self, KernelError>; pub fn as_str(&self) -> &str; }
pub struct PageRequest { pub limit: PageLimit, pub after: Option<Cursor> }
pub struct Page<T> { pub items: Vec<T>, pub next: Option<Cursor> }   // + map

// entitlements.rs
pub enum Plan { Lite, Pro }                     // code(), features() -> &'static [Feature]
pub enum Feature { Assistant, AllSuggestions, AllNews, FullHistory, StockTracker }   // code(), ALL
impl Entitlements { pub fn new(plan: Plan, expires_at: Option<OffsetDateTime>) -> Self; pub fn lite() -> Self;
                    pub fn effective_plan(&self, now: OffsetDateTime) -> Plan;
                    pub fn allows(&self, feature: Feature, now: OffsetDateTime) -> bool; }

// clock.rs
pub trait Clock: Send + Sync { fn now(&self) -> OffsetDateTime; }
pub struct SystemClock;
#[cfg(any(test, feature = "testing"))] pub struct FixedClock;   // new(at), set(at), advance(duration)
```

**Rules the tests pin**
- Every `KernelError` code and kind equals the table of spec §6.2. No `Display` text contains an input value.
- `Currency`: exactly three ASCII upper-case letters; `"cop"`, `"CO"`, `"COPX"`, `"C0P"` and non-ASCII are refused.
- `Money::parse`: an optional `-`, 1 to 18 digits, optionally `.` and 1 or 2 digits. `"1234.5"` is kept as `1234.50`. Refused: an exponent, a leading `+`, spaces, 3 decimal places, 19 integer digits, an empty string. `Money::new` refuses a scale above 2 and an amount of 10^18 or more in absolute value, and normalises to scale 2.
- `checked_add` and `checked_sub`: two currencies give `CURRENCY_MISMATCH`; leaving the range gives `AMOUNT_OVERFLOW`.
- An id is generated as UUID version 7; `parse` accepts only the 36-character hyphenated lower-case form and any version; `Display` round trips.
- `PageLimit`: 0 and 51 refused, 1 and 50 accepted, default 20. `Cursor`: empty, 513 characters and a character outside `A-Z a-z 0-9 - _` refused.
- `Entitlements`: Pro with no expiry allows every feature; Pro expired at exactly `now` counts as Lite; Lite allows none of the five.
- `FixedClock` returns what it was given and moves only when told.

**Steps**
- [ ] Write the tests of one file, see them fail, implement, see them pass; one file at a time in the order above.
- [ ] `cargo xtask check-arch` is clean (the kernel's list is `thiserror`, `uuid`, `time`, `rust_decimal`).
- [ ] Commit: `feat(BAR-3): add the kernel types`

---

### Task 2: `xtask`: layer rules and `check-coverage` (gate)

Skill: `security-and-hardening`.

**Files**
- Modify: `Cargo.toml` (`syn`, `proc-macro2` in `[workspace.dependencies]`), `crates/xtask/Cargo.toml`, `crates/xtask/src/{arch,main,lib,error}.rs`, `crates/xtask/tests/arch_workspace.rs`
- Create: `crates/xtask/src/coverage.rs` (submodules under `crates/xtask/src/coverage/` when it grows), `crates/xtask/tests/coverage_cli.rs`

A code review and a security audit of the first version (commits `e978670` and `f749706`) found ways to pass both gates wrongly: test lines counted as covered production lines, production lines dropped as test lines, a crate left out of the report skipping its bar, and layer rules defeated by a renamed package, a crate outside the workspace or the generated `workspace-hack` crate. A re-review and a re-audit of those fixes (`a26a23f`, `e7bb417`) found more. What follows is the end state, both rounds included.

**What the gates guard against.** They must not pass by mistake: any shape an honest contributor, human or agent, could write that hides code from the measure or slips a layer rule is refused or measured. They do not guard against a hostile author: the change's own code runs in the same CI job before the gates finish, a test or a build script can rewrite the report or the tool, and the gate files belong to the change. The owner reads every change to them; CODEOWNERS on them is an open owner decision.

**Part A: `check-arch`**

A dependency on a workspace member is known by the member it resolves to (by package name, then by that member's folder), never by a name written in the rule, so renaming a package cannot slip a rule.

| Rule | Behaviour |
| --- | --- |
| `package-name` (new) | The package in each known folder has its expected name: `crates/kernel` is `barras-kernel`, `crates/modules/<m>` is `barras-<m>`, `crates/adapters/<a>` is `barras-adapter-<a>`, `apps/<a>` is `barras-<a>`, `crates/bootstrap` is `barras-bootstrap`, `crates/testkit` is `barras-testkit`, `crates/workspace-hack` is `workspace-hack`, `crates/xtask` is `xtask`. The cargo-deny bans name crates, so this keeps each name tied to its folder |
| `path-outside-members` (new) | Any crate, any dependency kind: a path dependency whose folder is not a workspace member's folder |
| `apps-are-roots` (new) | No crate depends on a crate under `apps/`, in any kind |
| `kernel-allowlist`, `module-allowlist` (changed) | `barras-kernel` and `workspace-hack` count only as the path dependency at that member's own folder |
| `bootstrap-isolation` (new) | `crates/bootstrap` depends on no workspace member except `workspace-hack`, in any kind |
| `bootstrap-apps-only` (new) | Only a crate under `apps/` may depend on the member at `crates/bootstrap`, in any kind |
| `testkit-dev-only` (changed) | Decided by the member at `crates/testkit`, not by its name |
| `hack-isolation` (new) | Every dependency of `crates/workspace-hack`, in any kind, comes from crates.io |
| `hack-is-a-stub` (new) | `crates/workspace-hack` has exactly one library target, at `src/lib.rs`, and at most one build script, at `build.rs`; `src/lib.rs` holds no item and no attribute but doc comments; `build.rs` holds only `fn main() {}`; the folder holds no other `.rs` file, no symlink and no entry that is neither a regular file nor a folder. The allowlists accept this crate only because it cannot carry code |
| `no-build-script` (new) | `crates/kernel` and each `crates/modules/<m>` have no build script: code it generates would escape the coverage measure, and pure domain code needs none |
| `unknown-location`, `not-a-member` (changed) | `crates/bootstrap` and `crates/workspace-hack` are known folders, and must be members when they hold a `Cargo.toml` |
| `tool-isolation` (unchanged) | `xtask` may not depend on `workspace-hack` either |

`crates/workspace-hack` does not exist until Task 14; the tests build it as a fixture, and the real workspace stays clean. One folder walker, which refuses symlinks and special entries, and one list of crate folders serve both `check-arch` and `check-coverage`.

**Part B: `check-coverage <lcov-file> [workspace-root]`**

The optional root lets the tests run the binary on a temporary workspace. Without it the root is this repository, whatever the current directory.

*The report.* LCOV is parsed by hand, strictly: an `SF` that is relative or holds `..`, a `DA` outside a record, a record inside another, a report cut short, and a `DA` line past the end of its source file are errors. A file named by two records counts a line once, as hit if either record hit it. Every non-empty line is a known LCOV record at column 0 (`TN`, `SF`, `FN`, `FNDA`, `FNF`, `FNH`, `DA`, `LF`, `LH`, `BRDA`, `BRF`, `BRH`, `VER`, `end_of_record`, or whatever subset cargo-llvm-cov 0.9.1 really writes); anything else is an error. Every error that comes from the report names it and the line, and a path taken from the report is printed escaped. The report and each source file are read only when they are regular files (not a symlink, a FIFO or a device) and under a size cap: 64 MiB for the report, 1 MiB for a source file.

*Which lines count.*
- An `SF` outside the root, under the spelling given or the canonical one, is ignored. An `SF` inside the root but outside every known crate folder and outside `target/` is an error, so code included from elsewhere cannot escape the bars. A missing source file inside the root is an error, under either spelling.
- Only files under a crate's `src/` are measured. The rest of the crate folder (`tests/`, `benches/`, `examples/`, `build.rs`) is not. `crates/testkit` and `crates/workspace-hack` are not measured.
- Every `.rs` file under the `src/` of each measured crate is parsed with `syn`, whether the report names it or not, and a file that does not parse is an error. Line positions come from `proc-macro2` with its `span-locations` feature.
- A source file under a measured `src/` that holds a function body outside test items must be named by the report. cargo-llvm-cov drops some paths by default (`tests.rs`, `*_tests.rs`, any path with `/rustc/<hex>/`, …), and a crate whose tests never run is absent from the report; neither can escape the bars this way, and every crate with code has a figure.
- A source nested deeper than 256 groups is an error naming the file, found by a walk that does not recurse, before the parse; the parse runs on a thread with a large stack.
- An item that carries exactly `#[cfg(test)]` is test code at any depth: in a file, in an inline module, in an `impl`, `trait` or `extern` block. The lines of its whole span, its attributes and doc comments included, are left out. `#[cfg(any(test, feature = "<name>"))]`, test support that other crates' tests use, counts as production.
- These are errors, each naming the file and the line, because each would hide code from the measure:
  - a `cfg` predicate that names `test` in any other form (`not(test)`, `all(test, …)`, …), or `#[cfg(test)]` anywhere but on an item;
  - a `cfg` or `cfg_attr` predicate that uses anything but `test` (as the test mark), `feature = "…"`, `unix`, `windows` and the `target_*` keys, combined with `all`, `any` and `not`, or that puts `not` around a `feature`: code that only a release build (`debug_assertions`), a custom cfg or a missing feature compiles is never measured;
  - a `cfg` or `cfg_attr` predicate that names `coverage`, and any `coverage(…)` attribute;
  - outside test items, the identifiers `cfg`, `cfg_attr`, `coverage`, `test` or `include`, or the keyword `mod`, inside a `macro_rules!` body or a macro call: macros are not expanded, so they could otherwise produce every shape this list refuses;
  - the identifier `include` anywhere, test code included, except in `include!(concat!(env!("OUT_DIR"), "<literal without ..>"))`: an included file is measured, or dropped, under its own path;
  - a `cfg_attr` whose attributes include `cfg`;
  - an out-of-line `#[cfg(test)] mod <name>;` whose name is not `tests`;
  - an out-of-line `mod <name>;` without `#[cfg(test)]` whose name is `tests` or ends in `_tests`: cargo-llvm-cov drops such files, so production code in them would escape;
  - a `#[path]` attribute on a module;
  - a file under a measured `src/` named `*_tests.rs` or `*-tests.rs`, or a folder there named `examples` or `benches`.
- A `tests.rs` file or a `tests/` folder inside `src/` is a test module, which cargo-llvm-cov leaves out of the report. It is parsed only for `include` and `#[path]`, so it cannot pull production code into a test build.

*The bars.* Lines are grouped by crate folder: `crates/kernel`, `crates/modules/<m>`, `crates/adapters/<a>`, `crates/bootstrap`, `crates/xtask`, `apps/<a>`. They are compared with integers (`hit * 100 >= bar * found`): 95 for `crates/kernel` and for each `crates/modules/<m>`, 85 for the total of the measured crates. `crates/kernel` and every `crates/modules/<m>` that holds a `Cargo.toml` must have a figure: absent from the report, or present with no production line, is a failure. Any other crate with no counted line has no figure. A report with no measured line at all fails. Failures are typed (`UnderBar`, `NotInReport`, `NothingMeasured`) with a `Display`, and the tests assert the variant. The tool prints one line per crate (`crates/kernel: 97.3% (220/226)`) and the total, cut and not rounded, so 94.99% never reads as 95.0%. Any failure exits 1.

*Accepted limits*, recorded in ADR-0027 (Task 15), with the threat model above:
- A stale report is caught only when it names a line past a file's end, so `just test-cov` always regenerates it; the report comes from the change's own code and can be forged.
- The total averages across crates. Trivially covered production lines, and test support under `any(test, feature = "…")`, count as production: only review sees padding.
- Code generated under `target/` and code a macro expands are not measured as such; a dependency's proc macro can emit any attribute, and only adapters and apps can take such dependencies.
- Code compiled out of the coverage build by a feature the workspace build leaves off is not measured.
- Test attributes not named `test` (`#[rstest]`, `#[test_case]`) are not recognised as test marks; the kernel's and the modules' dev dependencies allow none of them.
- A file swapped for a FIFO between the check and the open is a residual race.

**Steps**
- [x] Part A, first version: `feat(BAR-3): teach check-arch the bootstrap and workspace-hack folders` (`e978670`).
- [x] Part B, first version: `feat(BAR-3): add the check-coverage subcommand` (`f749706`).
- [x] Review fixes to Part A, test first: each rule above gets a test that fails on the first version. Commit: `fix(BAR-3): close the check-arch rules found open in review`
- [x] Review fixes to Part B, test first, same way. Commit: `fix(BAR-3): read sources with syn in check-coverage and fail closed` (`e7bb417`; Part A was `a26a23f`).
- [ ] Second round, test first: `refactor(BAR-3): share one folder walker and one list of crate folders in xtask` (no change in behaviour), `fix(BAR-3): refuse symlinks and build scripts where check-arch cannot see code`, `fix(BAR-3): close the remaining ways around check-coverage`.

---

### Task 3: `barras-bootstrap`

Skill: `observability-and-instrumentation`.

**Files**
- Create: `crates/bootstrap/Cargo.toml`, `crates/bootstrap/src/{lib,environment,logging,shutdown}.rs`
- Modify: `Cargo.toml` (`members`; `barras-bootstrap`, `tokio`, `tracing`, `tracing-subscriber` in `[workspace.dependencies]`), `clippy.toml`

**Interfaces** (spec §4.3, §8)

```rust
// environment.rs
pub enum AppEnv { Local, Staging, Production }
pub enum LogLevel { Trace, Debug, Info, Warn, Error }
pub trait FromEnvValue: Sized { const EXPECTED: &'static str; fn from_env_value(raw: &str) -> Option<Self>; }
// impls: AppEnv, LogLevel, std::net::SocketAddr
pub struct Environment { /* name -> value, and the problems found so far */ }
impl Environment {
    pub fn from_process() -> Self;                                  // the one reader of the process environment
    pub fn from_pairs<I, K, V>(pairs: I) -> Self;                   // tests and `.env.example`
    pub fn required<T: FromEnvValue>(&mut self, name: &'static str) -> Option<T>;
    pub fn finish<T>(self, config: Option<T>) -> Result<T, ConfigError>;   // the values, or every problem
}
pub struct ConfigError;   // problems() -> &[ConfigProblem] (name, ProblemKind::{Missing, Invalid}); Display: one line per problem

// logging.rs
pub fn init_logging(env: AppEnv, level: LogLevel) -> Result<(), LoggingError>;
pub fn build_subscriber<W>(env: AppEnv, level: LogLevel, writer: W) -> impl tracing::Subscriber + Send + Sync;

// shutdown.rs (Unix only: install() registers SIGTERM and SIGINT inside the tokio runtime)
pub struct ShutdownSignal;
impl ShutdownSignal { pub fn install() -> Result<Self, std::io::Error>; pub async fn recv(&mut self); }   // install() outside a runtime: an error, not a panic
```

**Rules the tests pin**
- A missing name and an empty value are both "missing". An unparsable value is "not valid". Every problem is recorded; `finish` returns all of them, in the order asked.
- A problem line is `<NAME> is missing: expected <EXPECTED>` or `<NAME> is not valid: expected <EXPECTED>`. With a value such as `hunter2-secret` set for a variable, the error text does not contain it.
- `from_process` reads only the names a configuration asks for, and a value that is not Unicode is not valid. It carries the one `#[allow(clippy::disallowed_methods)]`, with its reason.
- `clippy.toml` disallows `std::env::var`, `std::env::var_os`, `std::env::vars` and `std::env::vars_os`. `xtask` reads `std::env::args`, which stays allowed.
- JSON format: every line is one JSON object with exactly the top-level keys `timestamp`, `level`, `target`, `fields` and, inside a span, `spans`. The event's fields, `message` included, are under `fields` and the spans' fields under `spans`, so no field can shadow a top-level key; a test parses each line into a struct that refuses unknown and duplicate keys. An event inside a span with `request_id` carries it under `spans`. `Local` gives readable lines, not JSON. A level below `level` is not written. The format is A.5.
- `init_logging` twice returns an error, not a panic.
- `finish` fails on any recorded problem, whatever `config` is: a variable that was read but left out of the configuration still stops the start-up. A value that is not valid Unicode is `Invalid`, even for a type that accepts any text.
- `ShutdownSignal`: after `install()`, a test sends SIGTERM, and another SIGINT, to its own process with the `kill` program, and `recv` returns (nextest runs each test in its own process, and tokio's handler is in place); a signal sent before `recv` is kept. `install()` outside a tokio runtime is an error, not a panic. Tokio never removes its handlers, so a second signal during the drain is ignored until `recv` is called again; say so in the doc of `recv`.

**Steps**
- [ ] Create the crate with the member fields of `CLAUDE.md`; `cargo xtask check-arch` is clean.
- [ ] `environment.rs`, test first; then the `clippy.toml` change.
- [ ] `logging.rs`, test first with the capture writer of A.5.
- [ ] `shutdown.rs` as in A.6.
- [ ] Commit: `feat(BAR-3): add the bootstrap crate for configuration, logs and shutdown`

---

### Task 4: HTTP adapter: envelopes and errors

Skill: `api-and-interface-design`.

**Files**
- Modify: `Cargo.toml` (`axum`, `serde`, `serde_json`, `utoipa`, `http-body-util` in `[workspace.dependencies]`; `barras-kernel` there too if absent), `crates/adapters/http/Cargo.toml`, `crates/adapters/http/src/lib.rs`
- Create: `crates/adapters/http/src/{envelope,error}.rs`

**Interfaces** (spec §6.1, §6.2)

```rust
// envelope.rs
#[derive(Serialize, ToSchema)] pub struct SuccessEnvelope<T> { success: bool, message: String, data: T, code: u16 }
#[derive(Serialize, ToSchema)] pub struct ErrorEnvelope { success: bool, message: String, data: (), code: u16, error: ErrorDetail }
#[derive(Serialize, ToSchema)] pub struct ErrorDetail { code: String, fields: Option<FieldErrors> }   // absent, never null
pub struct ApiSuccess<T> { /* status, message, data */ }
impl<T> ApiSuccess<T> { pub fn ok(message: impl Into<String>, data: T) -> Self; pub fn created(message: impl Into<String>, data: T) -> Self; }
impl<T: Serialize> IntoResponse for ApiSuccess<T>
pub type ApiResult<T> = Result<ApiSuccess<T>, ApiError>;

// error.rs
pub type FieldErrors = BTreeMap<String, Vec<String>>;
pub struct ApiError { /* status, code, message, fields, detail for the log */ }
impl ApiError {
    pub fn validation(fields: FieldErrors) -> Self;      // 422 VALIDATION_FAILED
    pub fn malformed_request(message: &'static str) -> Self;   // 400
    pub fn unsupported_media_type() -> Self;             // 415
    pub fn payload_too_large() -> Self;                  // 413
    pub fn route_not_found() -> Self;                    // 404
    pub fn method_not_allowed() -> Self;                 // 405
    pub fn not_acceptable() -> Self;                     // 406
    pub fn service_unavailable(detail: String) -> Self;  // 503 SERVICE_UNAVAILABLE
    pub fn internal(detail: String) -> Self;             // 500 INTERNAL_ERROR
    pub fn status(&self) -> StatusCode; pub fn code(&self) -> &str;
}
impl<E: AppError> From<E> for ApiError
impl IntoResponse for ApiError
```

**Rules the tests pin**
- The exact JSON of both envelopes: the key set, `data: null` on an error, `error.fields` absent unless it is a validation failure, the top-level `code` equal to the HTTP status.
- `From<E: AppError>`: each `ErrorKind` gives the status of spec §6.2; `error.code` is the variant's code. For `Internal` and `Unavailable` the message is a fixed generic sentence and the error's own text, with its chain of sources, is kept as the detail; for the other kinds the message is the error's `Display`.
- `IntoResponse` writes the detail to the log at `error` level and never into the body. A test with a fixture error whose source says `secret-sql-detail` asserts the body lacks it and the captured log has it.
- `ApiError` does not implement `AppError`: the blanket `From` depends on that.
- The schema of `ErrorEnvelope.data` is `{"type":"null"}` (A.2).

**Steps**
- [ ] `envelope.rs`, then `error.rs`, each test first. The unit tests call `into_response()` and read the body with `http_body_util::BodyExt::collect`.
- [ ] Commit: `feat(BAR-3): add the response envelopes and the error mapping`

---

### Task 5: HTTP adapter: `ApiRouter` and the OpenAPI document

Skill: `api-and-interface-design`.

**Files**
- Modify: `Cargo.toml` (`utoipa-axum`), `deny.toml` (`utoipa-axum` as a wrapper of `axum`, with its reason), `crates/adapters/http/Cargo.toml`, `crates/adapters/http/src/lib.rs`
- Create: `crates/adapters/http/src/{router,openapi}.rs`

**Interfaces** (spec §4.2, §7)

```rust
// router.rs
pub struct Access { /* permission: Option<&'static str>, feature: Option<Feature> */ }
impl Access { pub const fn public() -> Self; }          // BAR-17 adds the constructors that take a permission and a feature
pub struct ApiRouter<S = ()>(/* utoipa_axum::router::OpenApiRouter<S> */);
impl<S: Clone + Send + Sync + 'static> ApiRouter<S> {
    pub fn new() -> Self;
    pub fn route(self, route: UtoipaMethodRouter<S>, access: Access, error_codes: &'static [&'static str]) -> Self;
    pub fn merge(self, other: Self) -> Self;
    pub fn with_state<S2>(self, state: S) -> ApiRouter<S2>;
}

// openapi.rs
pub const COMMON_ERROR_CODES: &[&str];                  // the seven of spec §7.2, sorted
pub fn canonical_json(document: &OpenApi) -> Result<String, serde_json::Error>;
pub fn metadata_problems(document: &OpenApi) -> Vec<String>;
pub(crate) fn document_base() -> OpenApi;               // info: "Barras API" and the workspace version; x-common-error-codes
pub(crate) fn operations(document: &OpenApi) -> Vec<(&str, &'static str, &Operation)>;
```

**Rules the tests pin**
- `route` writes `x-permission` (`public`), `x-plan-feature` (`none`) and `x-error-codes` (sorted) on every operation of the route it is given (A.1). `ApiRouter` has no method that takes a bare `MethodRouter`.
- `canonical_json`: `serde_json::to_value`, `sort_all_objects()`, pretty-printed, one final newline. Two calls give the same bytes for a document whose operations carry three `x-` keys; so do two processes (the e2e of Task 11 compares with the committed file).
- `document_base`: `info.title` is `Barras API`, `info.version` is `CARGO_PKG_VERSION`, no `license`, no `contact`, no `description` taken from a crate. Sub-routers start from `OpenApiRouter::default()`, never `new()` (A.1).
- `metadata_problems` returns, for a fixture document, one line per missing item of spec §7.2, each naming the method, the path and the item: no `operationId`; no tag; no summary; no 2xx response whose body refers to a `SuccessEnvelope_` component; a 4xx or 5xx response whose body is not `ErrorEnvelope`; a response without `description`; a missing `x-error-codes`, `x-permission` or `x-plan-feature`. A complete document gives an empty list.

**Steps**
- [ ] `router.rs`, test first, with a fixture handler carrying `#[utoipa::path]`.
- [ ] `openapi.rs`, test first.
- [ ] `cargo deny --locked check bans` is clean.
- [ ] Commit: `feat(BAR-3): add the documented router and the canonical OpenAPI document`

---

### Task 6: HTTP adapter: the app, its middleware and the test client

**Files**
- Modify: `Cargo.toml` (`tower`, `tower-http`, `http`, `http-body`, `bytes`, `tokio`, `uuid` as needed), `crates/adapters/http/Cargo.toml` (feature `testing`), `crates/adapters/http/src/lib.rs`
- Create: `crates/adapters/http/src/app.rs`, `crates/adapters/http/src/middleware/{mod,request_id,fallback,safety_net}.rs`, `crates/adapters/http/src/testing/{mod,test_client}.rs`, `crates/adapters/http/tests/{support/mod.rs,stack.rs,route_guard.rs}`

**Interfaces**

```rust
// app.rs
#[derive(Clone, Copy)] pub struct ServiceInfo { pub name: &'static str, pub version: &'static str }
pub struct HttpApp { /* public router, document; Task 9 adds the internal app */ }
impl HttpApp {
    pub fn builder(service: ServiceInfo) -> HttpAppBuilder;
    pub fn openapi(&self) -> &OpenApi;
    pub fn openapi_json(&self) -> Result<String, serde_json::Error>;
}
impl HttpAppBuilder {
    pub fn routes(self, routes: ApiRouter) -> Self;      // nested under /api/v1
    #[cfg(feature = "testing")] pub fn undocumented_route(self, path: &str, route: MethodRouter) -> Self;
    pub fn build(self) -> HttpApp;
}

// middleware: a response extension the test client reads
#[derive(Clone)] pub struct MatchedRoute { pub method: Method, pub template: String }

// testing/test_client.rs (feature "testing")
pub struct TestClient;
impl TestClient {
    pub fn new(app: &HttpApp) -> Self;
    pub async fn get(&self, path: &str) -> TestResponse;
    pub async fn send(&self, request: TestRequest) -> TestResponse;   // method, path, headers, JSON or raw body
}
pub struct TestResponse { pub status: u16, /* headers, body */ }
impl TestResponse { pub fn header(&self, name: &str) -> Option<&str>; pub fn bytes(&self) -> &[u8];
                    pub fn json(&self) -> serde_json::Value; pub fn error_code(&self) -> Option<String>; }
```

**The stack** is A.3, in that order, applied with `Router::layer` so the fallback is covered. This task builds all of it except the request log and the metrics (Task 8); it leaves their two places marked by the order of the code, not by a comment that says "todo".

**Rules the tests pin** (in `tests/stack.rs`, in-process against a fixture app from `tests/support`, built with `ApiRouter` like the real one)
- An unknown path under and outside `/api/v1`: 404 `ROUTE_NOT_FOUND`. A known path with the wrong method: 405 `METHOD_NOT_ALLOWED`, with the `allow` header kept.
- A handler that panics: 500 `INTERNAL_ERROR`, a generic message, the panic text only in the log.
- A response that a library layer produced on its own (a fixture route using axum's stock `Json` extractor without a content type): the safety net turns it into an envelope with the code its status maps to: 400 `MALFORMED_REQUEST`, 413 `PAYLOAD_TOO_LARGE`, 415 `UNSUPPORTED_MEDIA_TYPE`, 406 `NOT_ACCEPTABLE`, any other 4xx `MALFORMED_REQUEST`, any 5xx `INTERNAL_ERROR`. A response that is already an envelope passes untouched, 406 excepted (A.3).
- Request id: none sent gives a UUID v7; a valid one is kept; an invalid one (65 characters; a space; a semicolon) is replaced. It is on the 404, the 405, the 413 and the panic response.
- Compression: a body of 4 KiB with `accept-encoding: gzip` comes back with `content-encoding: gzip` and the gzip magic bytes; with `br`, brotli; a body under 1 KiB is not compressed; `identity;q=0` gives 406 `NOT_ACCEPTABLE`.
- A body over 1 MiB to a route that reads its body is refused with 413 (Task 7 gives it the right code through the extractor; here the safety net does).
- `MatchedRoute` is on the response of a matched route and absent on a fallback response.
- `tests/route_guard.rs`: `TestClient` panics, naming `GET /api/v1/hidden`, when a request is served by a route added with `undocumented_route`; it does not panic for a documented route, for a fallback, for a 405, or for `HEAD` on a documented `GET`.

`TestClient` is test support compiled as library code, so its module carries `#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]` with the reason on the same line.

**Steps**
- [ ] `tests/support`: a fixture app with a small success route, a big success route, a panic route, a stock-extractor route. See the tests fail to compile, then fail.
- [ ] `app.rs` and the fallbacks; then the safety net; then the request id; then compression and the body limit; `TestClient` and `MatchedRoute` along the way.
- [ ] `route_guard.rs`, test first.
- [ ] Commit: `feat(BAR-3): add the HTTP app, its middleware and the test client`

---

### Task 7: HTTP adapter: validating extractors

**Files**
- Modify: `Cargo.toml` (`garde`, `serde_path_to_error`; `axum` feature `query`), `crates/adapters/http/Cargo.toml`, `crates/adapters/http/src/lib.rs`, `crates/adapters/http/tests/support/mod.rs`
- Create: `crates/adapters/http/src/extract/{mod,validated_json,validated_query}.rs`, `crates/adapters/http/tests/validation.rs`

**Interfaces:** `pub struct ValidatedJson<T>(pub T)` and `pub struct ValidatedQuery<T>(pub T)`, both `FromRequest`/`FromRequestParts` with `Rejection = ApiError`, for `T: DeserializeOwned + garde::Validate` with `T::Context: Default`. The shape is A.4.

**Rules the tests pin** (fixture route taking a body with `email`, `username`, a nested `address { street, zip }` and `tags`)
- Valid: 200. `application/json; charset=utf-8` and `application/merge-patch+json` are accepted.
- No content type, or `text/plain`: 415 `UNSUPPORTED_MEDIA_TYPE`.
- Truncated JSON, not JSON, trailing characters, an empty body: 400 `MALFORMED_REQUEST`.
- A body that is a JSON array or a scalar: 400 `MALFORMED_REQUEST`, even when the array would deserialise positionally.
- A wrong type at `address.zip`: 422 `VALIDATION_FAILED` with the key `address.zip`. A missing field: the key is the field's own path (`email`, `address.zip`), not its parent's. `null` for a required field and a wrong type inside an array (`tags[1]`) likewise.
- Broken rules: every one is reported, with the keys `email`, `username`, `address.street`, `tags[1]`.
- No message repeats a value: the test sends `hunter2-secret` as a wrong-typed value and as a rule-breaking value, and asserts the whole response body lacks it. Messages for serde failures are our own (`is required`; `expected <what serde expected>`; otherwise `is not valid`), built from the part of serde's message after `, expected `.
- A body over 1 MiB: 413 `PAYLOAD_TOO_LARGE`, for a known length and for a streamed body.
- Query: `?limit=abc` on a numeric field gives 422 with the key `limit`; a broken rule gives 422 with its key; a query string that cannot be parsed at all gives 400 `MALFORMED_REQUEST`.

**Steps**
- [ ] `tests/validation.rs` first, then `validated_json.rs`, then `validated_query.rs`.
- [ ] Commit: `feat(BAR-3): add the validating extractors`

---

### Task 8: HTTP adapter: request log and metrics

Skill: `observability-and-instrumentation`.

**Files**
- Modify: `Cargo.toml` (`prometheus` with `default-features = false`, `tracing`; `tower-http` feature `trace`), `deny.toml` if a licence is new, `crates/adapters/http/Cargo.toml`, `crates/adapters/http/src/{lib,app}.rs`, `crates/adapters/http/src/middleware/mod.rs`
- Create: `crates/adapters/http/src/metrics.rs`, `crates/adapters/http/src/middleware/{request_log,http_metrics}.rs`, `crates/adapters/http/tests/{request_log.rs,http_metrics.rs}`

**Interfaces**

```rust
// metrics.rs
#[derive(Clone)] pub struct Metrics { /* Registry, IntCounterVec, HistogramVec */ }
impl Metrics {
    pub fn new(service: ServiceInfo) -> Result<Self, MetricsError>;   // registers the three metrics of spec §6.6
    pub fn render(&self) -> Result<String, MetricsError>;             // Prometheus text
}
// app.rs: the builder now takes the metrics
pub fn builder(service: ServiceInfo, metrics: Metrics) -> HttpAppBuilder;
```

**Rules the tests pin**
- Metrics: after a request to a matched route, `http_requests_total{method="GET",path="/api/v1/…",status="200"} 1` is in the text; an unknown path counts under `path="unmatched"`; the histogram has the twelve bucket edges of spec §6.6; `barras_build_info{service=…,version=…} 1` is present. The recording calls use the fallible `get_metric_with_label_values`, never the form that panics.
- Two `Metrics` values in one process do not see each other's counts: nothing is global.
- Request log, captured with the writer of A.5 on a current-thread runtime: one `info` event per finished request with the status and the duration, inside a span with `method`, `path` (the template, or `unmatched`) and `request_id`. tower-http's own `on_failure` event is switched off; the cause of a 5xx is logged once, by `ApiError` or by the panic handler.
- Nothing leaks: a request with `authorization: Bearer header-secret`, `?token=query-secret` and a body holding `body-secret` leaves none of the three in the captured output.

**Steps**
- [ ] `metrics.rs`, test first; then `http_metrics.rs` in its place in the stack; then `request_log.rs`.
- [ ] `just deny` is clean.
- [ ] Commit: `feat(BAR-3): add the request log and the HTTP metrics`

---

### Task 9: HTTP adapter: health, internal port, server

**Files**
- Modify: `Cargo.toml` (`hyper-util`, optional, for `testing`), `crates/adapters/http/Cargo.toml`, `crates/adapters/http/src/{lib,app}.rs`, `crates/adapters/http/src/testing/mod.rs`
- Create: `crates/adapters/http/src/health/{mod,probe,routes}.rs`, `crates/adapters/http/src/health/dto/health_status.rs`, `crates/adapters/http/src/{internal,server}.rs`, `crates/adapters/http/src/testing/fetch.rs`, `crates/adapters/http/tests/{health.rs,server.rs}`

**Interfaces** (spec §5)

```rust
// health/probe.rs
pub enum Criticality { Critical, Optional }
#[derive(Clone)] pub struct HealthProbe { /* name, criticality, check */ }
impl HealthProbe {
    pub fn new<F, Fut>(name: &'static str, criticality: Criticality, check: F) -> Self
    where F: Fn() -> Fut + Send + Sync + 'static, Fut: Future<Output = Result<(), String>> + Send + 'static;
}
// app.rs
impl HttpAppBuilder { pub fn probe(self, probe: HealthProbe) -> Self; }
impl HttpApp { pub fn internal(&self) -> &InternalApp; }
// internal.rs
pub struct InternalApp;
impl InternalApp { pub fn new(service: ServiceInfo, metrics: Metrics, probes: Vec<HealthProbe>) -> Self; }
// server.rs
pub struct Listener; impl Listener { pub fn local_addr(&self) -> Result<SocketAddr, ServeError>; }
pub async fn bind(name: &'static str, addr: SocketAddr) -> Result<Listener, ServeError>;
pub async fn serve_api(app: HttpApp, public: Listener, internal: Listener,
                       shutdown: impl Future<Output = ()> + Send + 'static) -> Result<(), ServeError>;
pub async fn serve_internal(app: InternalApp, internal: Listener,
                            shutdown: impl Future<Output = ()> + Send + 'static) -> Result<(), ServeError>;
// testing/fetch.rs
pub async fn fetch(addr: SocketAddr, path: &str) -> TestResponse;      // GET over a real socket
impl TestClient { pub fn internal(app: &InternalApp) -> Self; }       // no route guard: the internal routes are not in the document
```

The two public routes are registered with `ApiRouter::route(routes!(…), Access::public(), …)`: `GET /health/live` with no code of its own, `GET /health` with `SERVICE_UNAVAILABLE`. Both have a summary, the tag `health`, described responses and the envelope schemas.

**Rules the tests pin**
- `tests/health.rs`, in-process: no probe gives `ok`; an optional probe failing gives 200 `degraded`; a critical probe failing gives 503 `SERVICE_UNAVAILABLE`; a probe that never answers counts as failed after 2 seconds (the test pauses tokio's clock; it does not wait). The public bodies never contain a probe's name or its error.
- Internal `GET /health`: 200 or 503 with `status`, `service`, `version` and one entry per probe (`name`, `critical`, `status` of `up` or `down`, `latencyMs`, and `error` only when down). No envelope.
- Internal `GET /metrics`: the text of `Metrics::render`, content type `text/plain; version=0.0.4`. Requests to the internal router are not counted in `http_requests_total`.
- The internal routes do not exist on the public router, and the public ones do not exist on the internal router.
- `tests/server.rs`, real sockets on `127.0.0.1:0`: both listeners answer through `fetch`; after the shutdown future resolves, a request already in flight completes and `serve_api` returns `Ok`; `bind` on a taken port returns an error that names the listener and the address.
- `metadata_problems(app.openapi())` is empty for an app with only the health routes.

**Steps**
- [ ] `probe.rs` and the readiness logic, test first; then the public routes and DTO; then `internal.rs`; then `server.rs` (A.6) and `fetch.rs`.
- [ ] Commit: `feat(BAR-3): add health, the internal port and the server`

---

### Task 10: `barras-testkit`: process helper

**Files**
- Modify: `crates/testkit/src/lib.rs`
- Create: `crates/testkit/src/process.rs`

**Interfaces**

```rust
pub fn free_local_addr() -> SocketAddr;                       // 127.0.0.1 with a port the OS just handed out
pub struct BinaryCommand;                                     // new(path), arg, env; starts from an EMPTY environment
impl BinaryCommand { pub fn run(self) -> Output; pub fn spawn(self) -> RunningBinary; }
pub struct RunningBinary;                                     // killed on drop
impl RunningBinary {
    pub fn wait_until_listening(&mut self, addr: SocketAddr, timeout: Duration) -> bool;   // false when the process exited or time ran out
    pub fn terminate(self, timeout: Duration) -> Output;      // SIGTERM, then the exit status and what it printed
}
```

**Rules the tests pin** (unit tests that drive `/bin/sh` and `/bin/sleep`, Unix only like the rest of the repository)
- The child sees only the variables it was given, plus `LLVM_PROFILE_FILE` when the parent has it: without that the coverage of a child process is lost (Review focus 4).
- `terminate` delivers SIGTERM (a shell `trap` proves it) and returns the status and the output. A process that ignores the signal is killed when the timeout ends.
- A dropped `RunningBinary` leaves no process behind.
- `wait_until_listening` returns `false` at once when the process has already exited.

The crate is test support, so it starts with `#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]` and its reason. SIGTERM is sent with the `kill` program: no `unsafe`, no new dependency.

**Steps**
- [ ] Test first, then implement.
- [ ] Commit: `test(BAR-3): add the process helper to the testkit`

---

### Task 11: `barras-api`: configuration, wiring, OpenAPI mode, e2e

**Files**
- Modify: `apps/api/Cargo.toml`, `apps/api/src/main.rs`, `justfile` (`openapi`, `openapi-check`), `.gitignore` if needed
- Create: `apps/api/src/{lib,config,cli}.rs`, `apps/api/tests/{e2e_health.rs,e2e_openapi.rs,e2e_process.rs}`, `.env.example`, `openapi.json`

**Interfaces** (spec §4.4, §8)

```rust
// config.rs
pub struct ApiConfig { pub env: AppEnv, pub log_level: LogLevel, pub bind_addr: SocketAddr, pub internal_bind_addr: SocketAddr }
impl ApiConfig { pub const VARIABLES: &[&str]; pub fn from_env(env: Environment) -> Result<Self, ConfigError>; }
// lib.rs
pub struct Dependencies { pub probes: Vec<HealthProbe> }
impl Dependencies { pub fn offline() -> Self; }               // what the wiring needs when nothing is served; BAR-4 adds the pools
pub fn build_app(dependencies: Dependencies) -> Result<HttpApp, BuildError>;
pub fn openapi_json() -> Result<String, BuildError>;          // build_app(Dependencies::offline()), canonical
pub async fn run(config: ApiConfig) -> Result<(), RunError>;  // logs, bind both, serve until the signal
// cli.rs
pub fn main_with_args(args: &[String]) -> ExitCode;           // no argument: serve; `openapi`; `openapi --check <file>`
```

`ApiConfig::from_env` reads each variable with `Environment::required`, builds the struct with `?` inside a helper closure (`(|| Some(ApiConfig { env: env?, … }))()`), and hands that to `Environment::finish`, which reports every problem at once.

`main.rs` is `fn main() -> ExitCode { barras_api::cli::main_with_args(&args) }` and nothing else. The serve mode loads the configuration, and on a `ConfigError` prints its lines to standard error (one `#[allow(clippy::print_stderr)]`, with its reason: the logs are not up, the configuration they need has just failed) and returns a non-zero status before any socket is opened. It then builds the tokio runtime by hand and blocks on `run`.

**Rules the tests pin**
- `e2e_health.rs` (in-process, `TestClient` on `build_app`): the health cases of spec §5 with fake probes passed through `Dependencies`; an unknown route; a wrong method; the request id. This is the card's "sample route returns both envelopes".
- `e2e_openapi.rs`: `metadata_problems` is empty; the committed `openapi.json` equals `openapi_json()` byte for byte; `ApiConfig::from_env` succeeds on the pairs read from `.env.example`, and every name in `ApiConfig::VARIABLES` is in that file.
- `e2e_process.rs` (the real binary, `env!("CARGO_BIN_EXE_barras-api")`, the helper of Task 10, `fetch` of Task 9):
  - an empty environment: non-zero status, standard error names each of the four variables, neither address ever listens;
  - `LOG_LEVEL=loud`: non-zero status, the line says what is expected and does not contain `loud`;
  - the values of `.env.example` with the two addresses replaced by free ones: `/api/v1/health` and `/api/v1/health/live` answer on the public address, `/health` and `/metrics` on the internal one, `/metrics` on the public one is 404; with `APP_ENV=staging` every line of standard output parses as JSON;
  - SIGTERM: status 0;
  - `openapi` with an empty environment prints exactly `openapi.json`;
  - `openapi --check <stale copy>`: non-zero status and a line that says `just openapi`; `--check` on the committed file: status 0.
  - If the binary exits before it listens because the port was taken in between, the test starts over with new addresses, up to three times.
- `.env.example` lists `APP_ENV`, `LOG_LEVEL`, `API_BIND_ADDR`, `API_INTERNAL_BIND_ADDR` and `INGEST_INTERNAL_BIND_ADDR`, each with a comment and a value that works on a developer machine (`local`, `info`, `127.0.0.1:8080`, `127.0.0.1:9080`, `127.0.0.1:9081`).

**Commands**

```make
openapi:
    cargo run --locked --quiet -p barras-api -- openapi > openapi.json
openapi-check:
    cargo run --locked --quiet -p barras-api -- openapi --check openapi.json
```

`openapi` must not truncate the file when the build fails: write to a temporary file and move it.

**Steps**
- [ ] `config.rs`, test first (unit tests with `Environment::from_pairs`).
- [ ] `lib.rs` and `e2e_health.rs`; then `cli.rs`, `main.rs`, `.env.example` and `e2e_process.rs`; then `openapi.json` with `just openapi` and `e2e_openapi.rs`.
- [ ] Commit: `feat(BAR-3): make barras-api a real process with its OpenAPI mode`

---

### Task 12: `barras-ingest`: process and e2e

**Files**
- Modify: `apps/ingest/Cargo.toml`, `apps/ingest/src/main.rs`
- Create: `apps/ingest/src/{lib,config}.rs`, `apps/ingest/tests/e2e_process.rs`

**Interfaces:** `IngestConfig { env, log_level, internal_bind_addr }` with `VARIABLES` and `from_env`; `pub async fn run(config: IngestConfig) -> Result<(), RunError>`; `pub fn main_with_args(args: &[String]) -> ExitCode`. It builds `Metrics` and an `InternalApp` with no probe and serves it with `serve_internal`. The job runner is BAR-6.

**Rules the tests pin:** an empty environment gives a non-zero status and names the three variables; with the values of `.env.example` and a free address, `/health` and `/metrics` answer and `barras_build_info{service="barras-ingest"…}` is in the metrics; SIGTERM gives status 0; `IngestConfig` loads from `.env.example`.

Two binaries now share a few lines of `main` (print the configuration error, build the runtime). That is two occurrences: leave them (code standard, "Duplication").

**Steps**
- [ ] Test first, then implement.
- [ ] Commit: `feat(BAR-3): make barras-ingest a real process with its internal port`

---

### Task 13: Commands, CI and cargo-deny (gate)

Skills: `ci-cd-and-automation`, `security-and-hardening`.

**Files**
- Modify: `justfile`, `rust-toolchain.toml`, `.github/workflows/ci.yml`, `deny.toml`, `.gitignore` if needed

**`justfile`**

| Recipe | Body |
| --- | --- |
| `setup` | Also installs `cargo-llvm-cov@0.9.1` and `watchexec-cli@2.8.0` |
| `dev`, `dev-ingest` | Fail with "copy .env.example to .env" when `.env` is missing; export its variables; `watchexec --restart --filter '*.rs' --filter 'Cargo.toml' --ignore 'target/**' -- cargo run -p <binary>` |
| `test` | `cargo nextest run --workspace --locked -E 'not binary(/^(e2e\|int)_/)'`, then `test-doc` |
| `test-doc` | `cargo test --workspace --doc --locked` |
| `test-int` | `cargo nextest run --workspace --locked -E 'binary(/^(e2e\|int)_/)'` |
| `test-cov` | `mkdir -p target/coverage`, then `cargo llvm-cov nextest --workspace --locked --no-cfg-coverage --lcov --output-path target/coverage/lcov.info`, then `cargo xtask check-coverage target/coverage/lcov.info` |
| `verify` | `fmt-check check-arch check-structure lint deny openapi-check test-cov test-doc build` (Task 14 adds `hakari-check` after `check-structure`) |

The two filters as they are written in the `justfile` (the table escapes the bar only for Markdown):

```make
test:
    cargo nextest run --workspace --locked -E 'not binary(/^(e2e|int)_/)'
    just test-doc
test-int:
    cargo nextest run --workspace --locked -E 'binary(/^(e2e|int)_/)'
```

- The two filters are valid only while at least one `e2e_` or `int_` test target exists (A.7); Tasks 11 and 12 created them.
- `test-cov` measures the whole workspace on every run and regenerates the report: never `-p`, `--exclude`, `--ignore-filename-regex` or `--no-default-ignore-filename-regex`, each of which takes code out of the measure. `--no-cfg-coverage` leaves `cfg(coverage)` unset, so no code can be compiled out of the instrumented build; `check-coverage` refuses such a predicate too. cargo-llvm-cov does not create the folder of `--output-path`.
- `rust-toolchain.toml`: `components = ["rustfmt", "clippy", "llvm-tools"]`. A wrong component name is only a warning, so check `rustup component list --installed`.

**CI**
- `checks`: the step "Check the pull request title and description" moves after "Format, lint, deny, architecture and structure", and that code step gains `openapi-check`. The branch-name and commit-message steps stay first: they do not hide the code checks, because the code step runs with `if: ${{ !cancelled() }}`. Every value written by whoever opens the pull request still reaches the shell only through `env`.
- `test`: installs `cargo-nextest@0.9.152` and `cargo-llvm-cov@0.9.1` through the pinned `install-action`, and runs `just test-cov test-doc`.
- `checks` runs `check-arch` and `check-structure` before anything compiles the workspace: `cargo xtask` builds only xtask and `cargo metadata --no-deps` runs no build script, so a build script in the change cannot rewrite a check before it runs. The local `verify` keeps the same order.
- Every job ends with a step that fails when a tracked file changed or an untracked one appeared (`git status --porcelain` is not empty), so a build script or a test that rewrites the tree is caught.
- `build`: unchanged otherwise. No new permission, no new secret, no new third-party action.

**`deny.toml`**
- `wildcards = "deny"` and `allow-wildcard-paths = true`.
- Bans with wrappers, each with its reason: `axum-core`, `hyper` and `hyper-util`. The wrappers are the HTTP and realtime adapters plus the third-party crates that depend on each directly today, read from `cargo tree -i <crate>`.
- A plain deny list, each with its reason: the web frameworks `actix-web`, `rocket`, `warp`, `poem`, `salvo`, `tide`; the database layers `diesel`, `sea-orm`, `tokio-postgres`; the HTTP clients `ureq`, `surf`, `isahc`, `curl`.
- The licences the tree needs, each one read from `cargo deny check licenses` and added only when it is permissive. A copyleft or unknown licence is not added: stop and report it.
- `unmaintained = "workspace"` only if `cargo deny check advisories` names a transitive crate we cannot replace; say which in a comment.

**Tests:** for each new ban, add the crate temporarily to an app's `Cargo.toml`, see `cargo deny check bans` fail, and remove it. `just verify` passes, and fails when the kernel is under its bar: remove one whole test module of the kernel temporarily and see `check-coverage` name `crates/kernel` (every production line of the kernel is run, so one test is not enough).

**Steps**
- [ ] `justfile` and `rust-toolchain.toml`; `just test`, `just test-int`, `just test-cov` each run and pass.
- [ ] `ci.yml`.
- [ ] `deny.toml`, with the negative checks above.
- [ ] `just verify` is green. Note its wall time, cold and warm.
- [ ] Commits: `build(BAR-3): split the tests into tiers and add the coverage thresholds`, `ci(BAR-3): run coverage and the OpenAPI check, and check the pull request text last`, `build(BAR-3): tighten the cargo-deny bans`

---

### Task 14: cargo-hakari (gate)

Skills: `ci-cd-and-automation`, `security-and-hardening`.

**Files**
- Create: `.config/hakari.toml`, `crates/workspace-hack/` (generated)
- Modify: `Cargo.toml` (`members`; `workspace-hack` in `[workspace.dependencies]`), every member's `Cargo.toml` but `xtask`'s, one source file per managed crate, `deny.toml`, `justfile`, `.github/workflows/ci.yml`

**Steps** (A.8 has the commands and what each one writes)
- [ ] `cargo install --locked cargo-hakari@0.9.39`; add it to `just setup`.
- [ ] `cargo hakari init crates/workspace-hack --package-name workspace-hack --yes`.
- [ ] `.config/hakari.toml`: `resolver = "3"`; `platforms = ["aarch64-apple-darwin", "x86_64-unknown-linux-gnu"]`; `workspace-hack-line-style = "workspace-dotted"`; `[traversal-excludes] workspace-members = ["xtask"]`.
- [ ] Root `Cargo.toml`: `workspace-hack = { version = "0.1", path = "crates/workspace-hack" }` in `[workspace.dependencies]`.
- [ ] `cargo hakari generate`, `cargo hakari manage-deps --yes`, `cargo hakari verify`.
- [ ] After the `### END HAKARI SECTION` marker of `crates/workspace-hack/Cargo.toml`: `[lints.cargo]` with `unused_dependencies = "allow"`, and a comment that says why.
- [ ] Each managed crate names the dependency once: `use workspace_hack as _;` in its `lib.rs`, with a one-line comment that says why. Check whether a package with a library and a binary needs it in both.
- [ ] `deny.toml`: `workspace-hack` as a wrapper of each banned crate that entered it, read from `cargo deny --locked check bans`.
- [ ] `justfile`: `hakari` (`generate`, then `manage-deps --yes`) and `hakari-check` (`generate --diff`, `manage-deps --dry-run`, `verify`); `verify` gains `hakari-check` after `check-structure`.
- [ ] CI `checks`: installs `cargo-hakari@0.9.39` and runs `hakari-check` in the code step.
- [ ] `just verify` is green; `cargo xtask check-arch` is clean with the rules of Task 2, `hack-isolation` and `hack-is-a-stub` included; `xtask`'s `Cargo.toml` is untouched. If a generated file breaks `hack-is-a-stub`, stop and report: do not loosen the rule.
- [ ] Negative check: add a feature to one crate's `tokio`, see `just hakari-check` fail, run `just hakari`, see it pass, revert.
- [ ] Measure and note: cold `cargo check -p barras-kernel`, and an incremental `cargo check` after a one-line change in the kernel (budget: 5 s).
- [ ] Commit: `build(BAR-3): adopt cargo-hakari`

---

### Task 15: Docs and ADRs

Skill: `documentation-and-adrs`.

**Files**
- Modify: `CLAUDE.md`, `README.md`, `CONTRIBUTING.md`, `docs/README.md`, `docs/adr/README.md`, `docs/adr/{0003,0015,0016,0017,0024}-*.md`, `docs/standards/{testing,module-anatomy}.md`, `docs/specs/2026-11-15-barras-foundation-design.md` (§6.4, one sentence), `docs/backlog/work-plan.md`
- Create: `docs/adr/0026-process-start-up-crate.md`, `docs/adr/0027-test-tiers-and-coverage.md`, `docs/adr/0028-openapi-pipeline.md`, `docs/adr/0029-http-contract-details.md`

**ADRs.** Each has Context, Decision and Consequences, `Date: 2026-11-15` and `Status: Accepted`: the owner accepted them by approving the spec (§12).

| ADR | Decision |
| --- | --- |
| 0024 (amended, then `Accepted`) | Adds: members known by their folder, and `package-name`; `path-outside-members` and `apps-are-roots`; `barras-kernel` only at the member's path; the `workspace-hack` exception (a path dependency in the kernel's and modules' lists, a wrapper in the bans, `xtask` left out) and the two rules that make it safe, `hack-isolation` and `hack-is-a-stub`; `no-build-script` for the kernel and the modules; the bans on `axum-core`, `hyper`, `hyper-util` and the deny list of alternatives. Its Context says the owner confirmed the stricter rule on 2026-11-15 |
| 0026 | `barras-bootstrap`: what it holds, its row in the dependency table, the two rules; Barras builds and runs only on Unix, because the shutdown signal is SIGTERM and SIGINT |
| 0027 | Tiers by file name (`e2e_`, `int_`); coverage over production lines, per crate; the kernel at 95%; what is not measured; doctests outside the measure; how `check-coverage` reads the sources (`syn`, `#[cfg(test)]` items) and the shapes it refuses; what the gates guard against and what they do not; its accepted limits (Task 2) |
| 0028 | `barras-api openapi` and `--check`; the canonical file; the metadata of spec §7.2; `ApiRouter` as the only door; the three-layer guard |
| 0029 | The adapter's own codes and statuses; camelCase; no 204; `x-request-id`; the body limit and compression; the internal port without envelope; metrics from an explicit registry |

ADR-0003, ADR-0015, ADR-0016 and ADR-0017 keep their Decisions. A Consequence that is no longer accurate gets a pointer: ADR-0003 and the design §6.4 (`check-arch` has lists only for the kernel and the modules: ADR-0024); ADR-0015 (the internal port: ADR-0029); ADR-0016 (clippy now denies the four ways of reading the environment); ADR-0017 (how coverage is measured: ADR-0027).

**`CLAUDE.md`:** the commands table and the `cargo xtask` subcommands; the layout (`crates/bootstrap`, `crates/workspace-hack`, `.config/hakari.toml`, `openapi.json`, `.env.example`); the dependency table (the two new rows, the kernel path rule, the new bans); "Adding a crate or a dependency" (`just hakari`, the `use workspace_hack as _;` line); the API rules (the adapter's codes, handlers use `ApiRouter` and the validating extractors only, `just openapi` after a route change); logging and configuration (the four disallowed methods, no default in code, the shape of a JSON log line and that every event has a message); members known by their folder, not by their package name (ADR-0024); the definition of done (how coverage is measured, the tier names).

**`README.md`:** what you need (cargo-llvm-cov, cargo-hakari, watchexec; how to add `rust-src` for an editor; the rustup version that accepts `rustup toolchain install` without arguments, only if verified against rustup's own documentation, otherwise leave the row as it is); first run (`cp .env.example .env`, `just dev`, the two addresses and what answers on each); everyday commands; the CI table; where things are; the `just lint` row says cargo's own warnings are denied; the two present-state sentences are rewritten so they do not go stale again. The Docker row stays "from BAR-4".

**`CONTRIBUTING.md`:** GitHub pre-fills the pull request title from the branch name and it must be replaced with `type(BAR-n): description`; after a dependency change run `just hakari`; after a route change run `just openapi`.

**Others:** `docs/standards/testing.md` (tiers, coverage); `docs/standards/module-anatomy.md` (the table rows, in step with `CLAUDE.md`); `docs/README.md` (this spec and plan in the table); `docs/backlog/work-plan.md` (a row for BAR-92, F-14, "Harden the repository checks").

**Steps**
- [ ] Write the ADRs and the index rows; `just check-structure` is clean.
- [ ] Update each document against the code as it is now, not against this plan: read the `justfile`, `deny.toml` and the crates first.
- [ ] Commit: `docs(BAR-3): record the core contract in the ADRs, the rules and the README`

---

### Task 16: Whole-branch verification and review (gate)

- [ ] `just verify` from a clean tree, cold and warm; note both times. Over 10 minutes: stop and report what could move out of `verify` (spec §14, risk 3).
- [ ] Walk the four "Done when" items of the card against spec §11 and name the test that proves each.
- [ ] Whole-branch review: `agent-skills:code-reviewer` on `develop..HEAD`, and `agent-skills:security-auditor` on `justfile`, `lefthook.yml`, `.github/`, `deny.toml`, `clippy.toml`, `crates/xtask/` and `.config/`.
- [ ] Fix what the reviews find, each fix test first, each in its own commit.
- [ ] Move the card to `EN PRUEBA`. Stop: the owner decides the push.

---

## Appendix A: what the spikes verified

Two spikes compiled and ran these on 2026-11-15, on Rust 1.100.0, before this plan was written. Quote them; do not rediscover them.

### A.1 utoipa 6 and utoipa-axum 0.3

- `UtoipaMethodRouter<S>` is the tuple `(Vec<(String, RefOr<Schema>)>, Paths, MethodRouter<S>)` that `routes!(handler)` returns. `ApiRouter::route` takes it apart, writes the extensions and hands it to `OpenApiRouter::routes`:

```rust
let (schemas, mut paths, method_router) = route;
for item in paths.paths.values_mut() {
    for operation in [item.get.as_mut(), item.put.as_mut(), item.post.as_mut(), item.delete.as_mut(),
                      item.options.as_mut(), item.head.as_mut(), item.patch.as_mut(), item.trace.as_mut()]
        .into_iter().flatten()
    {
        let extensions = ExtensionsBuilder::new()            // utoipa::openapi::extensions
            .add("x-permission", permission)
            .add("x-plan-feature", feature)
            .add("x-error-codes", codes.clone())
            .build();
        match operation.extensions.as_mut() {
            Some(existing) => existing.merge(extensions),
            None => operation.extensions = Some(extensions),
        }
    }
}
Self(self.0.routes((schemas, paths, method_router)))
```

- `OpenApiRouter::new()` stamps utoipa-axum's own title, author and version into `info`. Use `OpenApiRouter::with_openapi(document_base())` at the root and `OpenApiRouter::default()` below it.
- A crate without a `license` field makes `#[derive(OpenApi)]` emit `"license": {"name": ""}`. Build `info` by hand.
- `split_for_parts()` gives `(axum::Router<S>, OpenApi)`. `OpenApiRouter` has no `method_not_allowed_fallback`: call it on the `axum::Router`.
- OpenAPI paths and axum 0.8 templates share the `{id}` syntax, so a document's `(method, path)` compares directly with `MatchedPath`.
- A response declared without `description` is emitted without one, which OpenAPI 3.1 does not allow. Always write it.

### A.2 Schemas

- `SuccessEnvelope<HealthStatus>` becomes the component `SuccessEnvelope_HealthStatus`, with `T` inlined.
- A `()` field needs `#[schema(schema_with = null_schema)]`, where `null_schema` returns `ObjectBuilder::new().schema_type(Type::Null).into()`, to read `{"type":"null"}`.
- `Option` with `skip_serializing_if` is absent on the wire and nullable in the schema.
- `to_pretty_json()` is not stable: `Extensions` is a `HashMap`. `serde_json::to_value`, then `sort_all_objects()`, then `to_string_pretty` is, also with serde_json's `preserve_order` switched on by another crate.

### A.3 The middleware stack

Outermost first, all through `Router::layer`:

```rust
ServiceBuilder::new()
    .layer(MapRequestLayer::new(drop_invalid_request_id))        // SetRequestId keeps whatever is in the header
    .layer(SetRequestIdLayer::x_request_id(MakeRequestUuidV7))   // header and extension on the request
    .layer(request_log)                                          // its span sees the id
    .layer(PropagateRequestIdLayer::x_request_id())              // onto every response from below
    .layer(http_metrics)                                         // sees the final status; adds MatchedRoute
    .layer(middleware::map_response(envelope_safety_net))
    .layer(CompressionLayer::new().compress_when(compress_when))
    .layer(CatchPanicLayer::custom(handle_panic))
    .layer(DefaultBodyLimit::max(1024 * 1024))
```

- `Router::layer` wraps the fallback; `route_layer` does not.
- `method_not_allowed_fallback` patches only the routes registered before it, on the router it is called on: it is the last routing call on the outermost router.
- `PropagateRequestId` must sit outside `CatchPanic`, or a panic response has no id. The request log must sit inside `SetRequestId`, or its span has none.
- `compress_when`: `SizeAbove::new(1024)` (a `u64`, compared with `>=`) and the three defaults `NotForContentType::{GRPC, IMAGES, SSE}`. A body of unknown length is always compressed.
- The safety net is generic over the body type, because `map_response` receives the inner layer's body: `async fn envelope_safety_net<B>(res: Response<B>) -> Response where B: http_body::Body<Data = Bytes> + Send + 'static, B::Error: Into<BoxError>`. A non-generic function fails to compile with an error that points at the outer layer.
- An `Accept-Encoding` that refuses everything (`identity;q=0`) makes the compression layer run the handler and then overwrite only the status with 406. The safety net sits outside it and replaces the body when the status is 406, envelope or not. The handler has still run; say so in a comment where the layer is configured.
- `DefaultBodyLimit` does nothing by itself: the extractor that buffers the body produces the 413.
- `MatchedPath` (axum feature `matched-path`) is the full template for a nested route, is present on a 405 and on an extractor rejection, and is absent when the fallback answers.
- `DefaultMakeSpan` records the whole URI. Write the span: `tracing::info_span!("request", method = %req.method(), path, request_id)`. `make_span_with` and `on_request` take `&Request<Body>`; `on_response` must take `&Response<_>`.
- Bytes that are not HTTP are answered by hyper with a bare 400. Nothing to do.

### A.4 Validation

```rust
async fn from_request(req: Request, state: &S) -> Result<Self, ApiError> {
    if !is_json_content_type(req.headers()) { return Err(ApiError::unsupported_media_type()); }
    let bytes = Bytes::from_request(req, state).await?;              // honours DefaultBodyLimit; 413 by its status
    let mut de = serde_json::Deserializer::from_slice(&bytes);
    let value: T = serde_path_to_error::deserialize(&mut de).map_err(serde_failure)?;
    de.end().map_err(|_| ApiError::malformed_request(NOT_JSON))?;    // trailing characters
    value.validate().map_err(|report| garde_failure(&report))?;
    Ok(Self(value))
}
```

- `serde_json::error::Category::Data` is a validation failure; `Syntax`, `Eof` and `Io` are malformed.
- A missing field is reported at the parent's path (`.` at the root), with the message ``missing field `name` ``: build the key from both.
- serde stops at its first error; garde reports every rule. `garde::Report::iter()` yields `&(Path, Error)`; `Path` prints as `address.zip` and `tags[1]`, like `serde_path_to_error::Path`.
- garde needs an attribute on every field, `#[garde(skip)]` included, and `#[garde(dive)]` on a nested struct. Features: `derive`, and `email` for that rule.
- A derived struct also deserialises from a JSON array, by position. Check that the first byte that is not white space is `{` before deserialising.
- `BytesRejection` is non-exhaustive: tell 413 from the rest by `rejection.status()`.

### A.5 Logs

```rust
match env {
    AppEnv::Local => fmt::layer().with_ansi(false).with_writer(writer).boxed(),
    _ => fmt::layer().json().with_current_span(false).with_span_list(true).with_writer(writer).boxed(),
}
// tracing_subscriber::registry().with(layer).with(level_filter); try_init() for the process
```

- The default timer already prints RFC 3339 in UTC. tracing-subscriber needs the feature `json`.
- The event's fields, `message` included, are under `fields`, and the spans' fields under `spans`; `spans[0]` is the request span. Nothing an event or a span writes can collide with a top-level key. `flatten_event(true)`, tried first, let a field named `level` or `timestamp` write a second key of the same name, which parsers read as the line's own.
- Capture in a test: a `Clone` wrapper over `Arc<Mutex<Vec<u8>>>` that implements `io::Write` and `MakeWriter`; `tracing::subscriber::set_default` on a current-thread runtime. It is thread-local: a multi-thread runtime loses the events.

### A.6 Serving and stopping

- Install the SIGTERM handler before binding, bind both listeners before serving either, then `tokio::try_join!` the two `axum::serve(listener, router).with_graceful_shutdown(…)` futures and the future that waits for the signal and flips a `tokio::sync::watch` channel. Each server waits with `shutdown.wait_for(|stop| *stop)`; a dropped sender also stops it.
- SIGTERM and Ctrl-C both drain the requests in flight and end with status 0. There is no drain deadline; none is added in this ticket.
- tokio features: `net`, `signal`, `sync`, `macros`, `rt-multi-thread`. This passes clippy with `unwrap`, `expect` and `panic` denied.
- A client without reqwest: `hyper_util::client::legacy::Client::builder(TokioExecutor::new()).build_http()` with the features `client-legacy`, `http1`, `tokio`; the body through `http_body_util::BodyExt::collect`. In-process: `tower::ServiceExt::oneshot` on a clone of the router (tower feature `util`).

### A.7 Coverage and the test tiers

- `cargo llvm-cov nextest --workspace --locked --lcov --output-path <file>` runs every test once and writes the report. Artefacts go to `target/llvm-cov-target`. The toolchain component is `llvm-tools`.
- Files under `tests/` are not counted; inline `#[cfg(test)]` modules are. `#[coverage(off)]` needs nightly.
- A binary started through `CARGO_BIN_EXE_<name>` is measured, through the `LLVM_PROFILE_FILE` variable the tool sets.
- Doctests cannot be measured on stable: `cargo test --doc` stays a separate step.
- nextest refuses a `binary()` filter that matches no test target, exit 94, and `--no-tests=pass` does not help. With one matching target the filter and its complement both work.
- Coverage on `develop` before this ticket: 91% with the test modules counted, about 84% of production lines, `xtask/src/main.rs` at 26%.

### A.8 cargo-hakari

- `init` adds the member to the root `Cargo.toml` and writes `.config/hakari.toml` and the crate. `manage-deps` adds one line to every managed member. `generate` and `manage-deps` update `Cargo.lock` themselves.
- The `workspace-dotted` line style needs the root `[workspace.dependencies]` entry written by hand first. If the style is changed after `manage-deps` ran, run `cargo hakari remove-deps --yes` and `manage-deps --yes` again.
- `[traversal-excludes]` keeps `xtask`'s needs out of the generated crate; `[final-excludes]` would not.
- Text after the `### END HAKARI SECTION` marker survives `generate`.
- Cargo 1.100 reports `unused dependency` for `workspace-hack` in every member and `missing [lints]` for the generated crate; `just lint` turns both into errors. The two remedies are in Task 14.
- `cargo deny check bans` fails with `unmatched-wrapper` once a banned crate is in the generated crate, until `workspace-hack` is one of its wrappers.
- `generate --diff`, `manage-deps --dry-run` and `verify` exit 0 in step and 1 out of step, and write nothing.
- Measured in a copy with tokio, serde and uuid: a cold `cargo check -p barras-kernel` builds 18 crates in 2.6 s with the dependency, 5 crates in 0.8 s without.
