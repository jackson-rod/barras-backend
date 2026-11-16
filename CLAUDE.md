# Working rules

These rules bind every contributor, human or agent. They restate the accepted ADRs in [`docs/adr/`](docs/adr/README.md), the foundation design ([the spec](docs/specs/2026-11-15-barras-foundation-design.md)) and the checks this repository runs. Most rules cite their source in parentheses: the ADR they restate, a section of the spec or a standard, or the check that enforces them. When a rule here and an ADR disagree, the ADR wins, and this file is fixed in the same pull request. Everything in this repository is in English: code, comments, commits, pull requests and docs (ADR-0018).

## What this repository is

- The backend of Barras, a mobile app for people in Colombia who invest. The app lives in the separate repository `barras-frontend`; the committed `openapi.json` is the contract between the two (ADR-0002).
- One Cargo workspace builds two binaries, each a composition root only, under `apps/` (ADR-0004):
  - `barras-api`: axum. HTTP under `/api/v1` and the websocket gateway in one process. Stateless, so it scales horizontally.
  - `barras-ingest`: the long-lived feed connections and the scheduled jobs that talk to outside sources.
  - The chatbot runs inside `barras-api` and reaches the language model through a port.

```
app ⇄ barras-api ⇄ Postgres 18 (schema `barras`)
          ▲
        Redis 8 (live ticks, latest-quote cache, replay buffer, rate limits)
          ▲
     barras-ingest ⇄ Binance, Buda, El Espectador, datos.gov.co, DANE, equities vendor
```

**Performance is the product** (ADR-0014; details in [`docs/standards/performance.md`](docs/standards/performance.md)):

- Home loads in one request: `GET /api/v1/dashboard` returns everything the screen needs.
- One query per read model, or an explicit batch. Never one query per row.
- Nothing builds a multi-megabyte JSON in a request. Exports stream.
- Server time: `GET /api/v1/dashboard` p95 ≤ 80 ms and p99 ≤ 200 ms; any other read p95 ≤ 150 ms; any write p95 ≤ 200 ms.
- Response body ≤ 64 KB compressed. Tick from source to phone p95 ≤ 1 s.
- Feedback loop: incremental `cargo check` after a one-line change in a module ≤ 5 s; `just test` ≤ 30 s; `just verify` and CI ≤ 10 min.
- Release builds use thin LTO and one codegen unit (`[profile.release]` in `Cargo.toml`). A hot path has a criterion benchmark.

**Never ship** what makes Barras a failure even if it works (spec §2):

- A suggestion or a chatbot answer that reads as advice tailored to the user (ADR-0022).
- News article text (ADR-0021).
- Money as a floating-point number (ADR-0008).
- Pro unlocked by the app alone, without the server agreeing (ADR-0010).
- A slow Home screen or a multi-megabyte JSON response (ADR-0014).
- A split into more services before a measured need (ADR-0004).

**Where the plan lives:**

- [`docs/README.md`](docs/README.md) is the plan; start there. It links the spec, the [ADR index](docs/adr/README.md), the standards in `docs/standards/`, the [schema](docs/schema/barras-schema.md) and the [work plan](docs/backlog/work-plan.md), which mirrors the Trello board BARRAS.
- A ticket starts with a spec in `docs/specs/BAR-<n>-<topic>.md`. No code before the owner approves it (spec §12).

## Commands

Every command runs from the repository root through `just`.

| Command | What it does |
| --- | --- |
| `just` | Lists the recipes (the `default` recipe) |
| `just setup` | Installs the toolchain that `rust-toolchain.toml` names (`rustup toolchain install`), installs cargo-nextest 0.9.152 and cargo-deny 0.20.2, installs the git hooks |
| `just dev` | Runs `barras-api` (`cargo run -p barras-api`) |
| `just dev-ingest` | Runs `barras-ingest` (`cargo run -p barras-ingest`) |
| `just fmt` | Formats the workspace with rustfmt |
| `just fmt-check` | Fails on unformatted code |
| `just lint` | clippy on every crate and target; any warning fails, cargo's own too (`build.warnings="deny"`), such as a member without `[lints]` |
| `just deny` | cargo-deny: advisories, licences, bans and sources. Needs the network |
| `just check-arch` | `cargo deny check bans`, then `cargo xtask check-arch` |
| `just check-structure` | `cargo xtask check-structure`: ADRs, module crates, migration names |
| `just test` | Unit tests with cargo-nextest, then the doctests with `cargo test --doc`, which nextest does not run; no Docker |
| `just build` | Release build of the workspace |
| `just verify` | `fmt-check`, `lint`, `deny`, `check-arch`, `check-structure`, `test`, `build`, in that order: what CI runs, apart from the branch, commit and pull request checks |

- `cargo xtask` subcommands: `check-branch [name]`, `check-commit-msg <file>`, `check-commits <range>`, `check-pr <file>`, `check-arch`, `check-structure`.
- The other commands of spec §6.14 (integration tests, coverage, database, OpenAPI) arrive with later tickets and are added to this table when they exist.

Git hooks (lefthook, installed by `just setup`; ADR-0018): `pre-commit` runs `just fmt-check`, `just lint`, `just check-arch` and `just check-structure`; `commit-msg` runs `cargo xtask check-commit-msg`; `pre-push` runs `cargo xtask check-branch` and `just verify`, so it needs the network, as `just deny` does. Never bypass them: `--no-verify` is not allowed, and neither is `LEFTHOOK=0`, which turns lefthook off.

## Repository layout

```
Cargo.toml              workspace; [workspace.dependencies] pins each version once; [workspace.lints]; edition 2024
Cargo.lock              committed; the checks and builds run with --locked
rust-toolchain.toml     Rust 1.100.0 with rustfmt and clippy
clippy.toml             test exemptions and disallowed methods
deny.toml               cargo-deny: licences, advisories, sources, and the bans that guard the layers
justfile                every command
lefthook.yml            git hooks
.cargo/config.toml      the `cargo xtask` alias
.github/                CI workflow and pull request template
apps/
  api/                  binary barras-api: composition root only
  ingest/               binary barras-ingest: composition root only
crates/
  kernel/               shared kernel: AppError, ErrorKind, Money, Currency, ids, Page and Cursor,
                        Entitlements, Clock port
  modules/<module>/     one crate per bounded context: domain, ports and use cases
  adapters/
    postgres/           sqlx repositories and mappers
    http/               axum routers, DTOs, extractors, envelope, error mapping, OpenAPI
    realtime/           websocket gateway
    redis/              pub/sub, cache, rate limiter
    providers/          outbound clients: exchanges, feeds, DANE, Apple, Google, store, model
  testkit/              test-only helpers: containers, fixtures, fakes
  xtask/                repository checks: architecture, structure, commit and branch names
migrations/             plain SQL migrations (sqlx)
.sqlx/                  offline query metadata, committed
docs/                   the plan: specs, plans, ADRs, standards, schema, analysis, backlog
```

- Modules: `identity`, `settings`, `billing`, `capital`, `budgets`, `goals`, `markets`, `news`, `indicators`, `assistant` (spec §6.2).
- `crates/modules/`, `migrations/` and `.sqlx/` do not exist yet; the first ticket that needs one creates it.

## Module anatomy

```
crates/modules/<module>/        snake_case; plural for collections
  README.md                     routes, use cases, ports, owned and read tables (required)
  Cargo.toml                    no web, database, HTTP-client or runtime dependency
  src/
    lib.rs                      the module's public surface
    domain/
      <entity>.rs               struct or enum that owns its invariants; constructors return Result
      errors.rs                 one enum <Module>Error, one variant per failure case
    ports/
      <name>_<role>.rs          a trait named for the role: UserRepository, QuoteFeed
    application/
      <use_case>.rs             one struct, one `execute`, its own input and output types
      services/                 only for a decision shared by two or more use cases of the module
```

- A module's package is `barras-<module>` ([module anatomy](docs/standards/module-anatomy.md)).
- `cargo xtask check-structure` fails a module whose folder name is not snake_case, or that lacks `README.md`, `src/lib.rs`, `src/domain/errors.rs`, `src/ports/` or `src/application/`.
- It also fails a `src/ports/` or `src/application/` with no `.rs` file directly inside: git does not track an empty folder, so the module would fail in CI.
- It also fails any `.rs` file directly in `src/domain/` (other than `mod.rs` and `errors.rs`) or directly in `src/application/` (other than `mod.rs`) that contains no `#[cfg(test)]`. Hidden files are ignored.
- The hooks check the working tree, not the commit: a file that exists on disk but is not committed passes locally and fails in CI.
- Adapters mirror the modules: `crates/adapters/postgres/src/<module>/<name>_repository.rs` with an explicit mapper in both directions, and `crates/adapters/http/src/<module>/` with `routes.rs` and `dto/<action>.rs` (spec §6.3).

### Dependency rule

- The "May depend on" column is the design rule (ADR-0003). The "Enforced by" column names what a check enforces (ADR-0024); review enforces the rest.
- `just check-arch` runs two checks. `cargo xtask check-arch` reads `cargo metadata` and fails: any kernel or module dependency outside its list, any adapter dependency on another adapter, `barras-testkit` taken as anything but a dev dependency, a workspace crate in `xtask`'s dependencies, and a crate outside the folders of the table (`unknown-location`).
- In those lists, an allowed name counts only from its expected source: `barras-kernel` only as the workspace crate, every other allowed name only from crates.io. A workspace crate is known by its package name, so a `git` copy of one is still caught (`check-arch`; ADR-0024).
- No `[patch]` or `[replace]` table in the root `Cargo.toml`, and no `[patch]` table in `.cargo/config.toml` (`no-patch`, `check-arch`; ADR-0024).
- Every folder directly under `apps/`, `crates/modules/` and `crates/adapters/` that holds a `Cargo.toml`, and `crates/kernel`, `crates/testkit` and `crates/xtask`, is listed in `members` (`not-a-member`, `check-arch`; ADR-0024).
- `cargo deny check bans` lets only these crates depend directly on a technology crate: `sqlx` only `barras-adapter-postgres` and `barras-testkit`; `axum` only `barras-adapter-http` and `barras-adapter-realtime`; `reqwest` only `barras-adapter-providers`; `redis` only `barras-adapter-redis` and `barras-testkit`. Every other crate, apps included, fails; an app reaches them through its adapter (`deny.toml`; ADR-0024).

| Crate | May depend on | Enforced by |
| --- | --- | --- |
| `crates/kernel` | `std` and the small pure crates `thiserror`, `uuid`, `time`, `rust_decimal` | `check-arch`: normal and build dependencies only from that list; dev dependencies from it plus `tokio`, `mockall`, `insta`; each only from crates.io (`kernel-allowlist`, `kernel-dev-allowlist`) |
| `crates/modules/<m>` | `barras-kernel`, the same pure crates and `async-trait`. Never another module, an adapter, axum, sqlx, reqwest, redis, tokio's runtime or serde_json | `check-arch`: normal and build dependencies only from that list; dev dependencies from it plus `tokio`, `mockall`, `insta`; `barras-kernel` only as the workspace crate, every other name only from crates.io (`module-allowlist`, `module-dev-allowlist`) |
| `crates/adapters/*` | `barras-kernel`, the module crates and its own technology. Never another adapter | `check-arch`: no other adapter, whatever its source (`adapter-isolation`). cargo-deny bans: a technology crate only in its own adapter. Any other library is left to review |
| `apps/*` | everything. The only place where a concrete adapter meets a port | cargo-deny bans: no direct `sqlx`, `axum`, `reqwest` or `redis`. `check-arch`: `barras-testkit` only as a dev dependency (`testkit-dev-only`) |
| `crates/testkit` | test-only helpers: containers, fixtures, fakes | cargo-deny bans: `sqlx` and `redis` allowed directly, `axum` and `reqwest` not. `check-arch` sets no list; every crate that uses it takes it only as a dev dependency (`testkit-dev-only`) |
| `crates/xtask` | no workspace crate | `check-arch`: no workspace crate, whatever its source (`tool-isolation`) |

Reject on sight: a framework type in a module crate; a database row type in a use case signature; a handler calling a repository; a module depending on another module; `unwrap`, `expect` or `panic!` outside tests (ADR-0003, ADR-0006).

### Adding a crate or a dependency

- List the crate in `members` of the root `Cargo.toml`. The list is explicit, with no globs (`Cargo.toml`). A crate folder under `apps/`, `crates/modules/` or `crates/adapters/`, or at `crates/kernel`, `crates/testkit` or `crates/xtask`, left out of it fails (`not-a-member`, `check-arch`).
- Its folder decides its layer: `crates/kernel`, `crates/modules/<module>`, `crates/adapters/<name>`, `apps/<name>`, `crates/testkit` or `crates/xtask`. A crate anywhere else fails (`unknown-location`, `check-arch`).
- Give its `Cargo.toml` the fields every member has, under its own `name` (a module is `barras-<module>`):

```toml
[package]
name = "barras-<module>"
version.workspace = true
edition.workspace = true
rust-version.workspace = true
publish.workspace = true

[lints]
workspace = true
```

- A module crate has the files and folders `check-structure` requires (see Module anatomy).
- Declare each dependency once, with its version, in `[workspace.dependencies]` of the root `Cargo.toml`, and take it in the crate with `<name>.workspace = true`. Enable only the features you use (spec §6.2, spec §6.12).
- A workspace crate is declared there too, with its `path`, as in `barras-kernel = { path = "crates/kernel" }`, and taken with `barras-kernel.workspace = true` like any other dependency.
- Refresh `Cargo.lock` with a plain `cargo build` before committing. The checks run with `--locked`, so the hooks fail on a stale lock file (`justfile`, `.cargo/config.toml`).

### Ports and wiring

- Ports are traits named for the role. Use cases hold `Arc<dyn Port>`; `main.rs` builds every adapter and use case by hand (ADR-0003).
- Async port traits use `async-trait`. Native async traits may replace it if they work behind `dyn` on the pinned toolchain (ADR-0003).
- A module that needs another module declares its own port; the composition root supplies a bridge that calls the other module's use case. Example: `capital` declares `ExchangeRates`; the bridge calls `indicators` (ADR-0003).

## Rules of the API

### Contract with the app

- Every route is under `/api/v1` (ADR-0005).
- Success: `{ success: true, message, data, code }`. Error: `{ success: false, message, data: null, code, error: { code, fields? } }` (ADR-0005).
- The top-level `code` is the HTTP status as a number. `error.code` is the stable `SCREAMING_SNAKE_CASE` string of the error variant (ADR-0005, ADR-0006).
- `message` is English and for developers. The app shows its own translated text chosen by `error.code` (ADR-0005).
- Messages never contain SQL, stack traces, tokens or another user's data. A 5xx answers with a generic message; the detail goes to the logs (ADR-0005).
- Money and any integer above 2^53 travel as strings (ADR-0005, ADR-0008).
- Generate `openapi.json` with utoipa and commit it with the change. It is the contract with the app, including the realtime event names and payloads, and it never drifts from the code (ADR-0002).
- Every route has OpenAPI metadata: the error codes it can return and the permission and plan feature it needs (ADR-0017).

### Errors

- Every use case and port returns `Result<T, <Module>Error>`. No exceptions, and no `anyhow` outside `main.rs` (ADR-0006).
- `<Module>Error` is one enum per module, in `src/domain/errors.rs`, with one variant per failure case (ADR-0006; spec §6.3).
- Each variant has a stable `SCREAMING_SNAKE_CASE` code and an `ErrorKind`: validation, unauthenticated, forbidden, not found, conflict, rate limited, unavailable or internal (ADR-0006).
- The code is public API: renaming it is a breaking change (ADR-0006).
- The domain does not know HTTP. Only the `http` adapter maps `ErrorKind` to a status (ADR-0006).
- Never swallow an error. Handle it, wrap it with its source, or return it (ADR-0006).

### Validation

- A handler is thin: validate the DTO, convert it into the use case's input, call one use case, map the output (spec §6.3).
- Validate at the boundary. Inner layers trust their inputs (code standard).
- A domain type owns its invariants: its constructor returns `Result` (spec §6.3).
- A validation failure answers 422 with `error.fields`, a map from field path to messages (ADR-0005).
- An error message states what failed and with what input, and never leaks secrets, tokens or full payloads (code standard).

### Authentication and authorisation

- Barras owns identity. Email and password use argon2id, with a verification code by email (ADR-0009).
- Apple and Google sign-in verify the provider's ID token against the provider's keys: issuer, audience, nonce and expiry. The same verified email links to one user (ADR-0009).
- The app sends `Authorization: Bearer <access token>`. Access tokens are signed JWTs that live 15 minutes (ADR-0009).
- Refresh tokens rotate on every use and are stored hashed. Reuse of an old one revokes the session. No cookies, no CSRF (ADR-0009).
- The access token carries the role and the plan's entitlements; no request reads the database to authorise. When a plan changes, the server sends a realtime event and the app refreshes its token (ADR-0010).
- A new user starts on Lite. Entitlements can carry an expiry, so a Pro trial ends on its own (ADR-0010).
- Permissions are a catalog in code, seeded into `permissions` and granted to roles through `role_permissions`. Each route declares the permission and the plan feature it needs (ADR-0010).
- The server is the only authority on what a user may use. Pro is bought through the stores; store events arrive by webhook and are stored idempotently in `subscription_events` (ADR-0010, ADR-0023).

### Data access

- sqlx 0.9 with hand-written SQL, checked at compile time in offline mode. No ORM. Commit the offline query metadata in `.sqlx/` (ADR-0007).
- Two pools: primary for writes and for reads that follow a write; a read pool for read models (a replica when one exists) (ADR-0007).
- Read models aggregate in SQL. Lists use cursor pagination, never `OFFSET`, with at most 50 items per page (ADR-0007).
- Each repository has an explicit mapper in both directions (ADR-0007).
- The schema changes only through migrations in `migrations/` (ADR-0007). Every non-hidden entry in `migrations/` must be named `YYYYMMDDHHMMSS_snake_case.sql` (`check-structure`).
- SQL that looks odd (a partial index, a CHECK) carries a comment that says why (ADR-0007).
- Money is `rust_decimal::Decimal` in Rust and `numeric` in Postgres, always with its currency. Fiat uses `numeric(20,2)`; quantities use `numeric(38,18)` (ADR-0008).
- Timestamps are `timestamptz` in UTC. The user's time zone applies only when presenting or grouping, such as a budget's month (ADR-0008).
- Ids are UUID version 7 (ADR-0008).

### Every outside system is a port

- Every system Barras does not own is reached through a port, implemented by an outbound adapter in `crates/adapters/providers/` (ADR-0012).
- Each outbound adapter has an explicit timeout, retries with backoff and jitter, maps failures to typed errors, shows the attribution its source requires, and has a contract test with recorded responses (ADR-0012).
- Each ingestion job runs on one instance at a time under its own Postgres advisory lock, and records each run in `ingest_runs` (ADR-0013). The jobs and their frequencies are listed in ADR-0013.
- Sources are those of ADR-0020. Colombian stocks are end-of-day only until a licence is bought. Yahoo is not used (ADR-0020).
- News is headline and link only: store title, link, date and section, never the body (ADR-0021).
- Suggestions are informational, the same for every user, and carry the disclaimer. The chatbot never recommends buying or selling. Lite sees the top 8 suggestions and the 8 latest headlines; Pro sees all (ADR-0021, ADR-0022).

### Realtime

- One websocket endpoint in `barras-api`. The first frame authenticates with the access token; tokens never appear in a URL (ADR-0011).
- Channels: `user:{id}` (private changes and the chatbot stream), `market:{symbol}` (ticks), `trends` (ranking changes) (ADR-0011).
- Envelope: `{ id, type, ts, data }`. A user event says what changed and the app refetches it; a lost event only delays freshness (ADR-0011).
- The app resumes with `lastEventId`; the replay buffer is a capped Redis stream (ADR-0011).
- Ticks are coalesced to at most one update per second per instrument per connection. On subscribe the app gets the current quote first (ADR-0011).
- A slow connection loses old ticks and never blocks the others (ADR-0011).
- `barras-ingest` publishes ticks to Redis; each API node subscribes once and fans out in memory (ADR-0011).

### Logging, observability and configuration

- `tracing` with JSON output in every deployed environment. Log at boundaries and on failure (ADR-0015).
- Never log credentials, tokens or request bodies (ADR-0015).
- `GET /api/v1/health` (ready: `ok`, `degraded`, or 503) and `GET /api/v1/health/live` are public. Per-dependency detail and Prometheus metrics are only on an internal port (ADR-0015).
- Each binary loads one typed configuration from the environment at start and refuses to boot when a variable is missing (ADR-0016).
- Every variable is listed in `.env.example` (ADR-0016).
- Only the configuration module reads the environment (ADR-0016). clippy denies `std::env::var` (`clippy.toml`); review enforces the rest of the rule, including `std::env::var_os` and `std::env::vars`.

## Definition of done

A change is done only when the same pull request contains (ADR-0017):

1. **Tests per layer.** Domain types tested directly. Use cases with fake ports. Mappers round trip in both directions. Routes through e2e tests against the real app. Every adapter to an outside system with a contract test. Errors asserted by `code`, never by message.
2. **OpenAPI metadata on every route**, including the error codes it can return and the permission and plan feature it needs.
3. **Docs.** The module `README.md` is current. A new or changed decision is an ADR, listed in the index.
4. **`just verify` is green** locally and in CI.

- Coverage: at least 95% of lines in module crates, 85% overall (ADR-0017).
- The pull request template carries the four items as checkboxes (`.github/pull_request_template.md`).

## Code style

- Follow [`docs/standards/code-standard.md`](docs/standards/code-standard.md): comments, duplication, naming, size, error handling. Where it and the module anatomy disagree on structure or naming, the module anatomy wins (spec §9).
- Comment why, never what (code standard).
- `unsafe` is forbidden (`unsafe_code = "forbid"` in `Cargo.toml`; spec §9).
- clippy runs with zero warnings. It denies `unwrap`, `expect`, `panic!`, `dbg!` and printing to stdout or stderr outside tests, and `todo!`, `unimplemented!` and `std::env::var` everywhere (`[workspace.lints]`, `clippy.toml`; spec §9).
- Every crate inherits the workspace lints with `[lints] workspace = true` (`Cargo.toml`).
- No `#[allow]` without the reason on the same line, as in `#![allow(clippy::print_stdout, clippy::print_stderr)] // a CLI: its output is the product` (spec §9).
- clippy's test exemption covers only `#[test]` functions and `#[cfg(test)]` modules. So an integration-test crate (`tests/*.rs`) and `barras-testkit` need a crate-level `#![allow(clippy::unwrap_used, clippy::expect_used)]`, its reason on the same line, to use `unwrap` or `expect` in their helpers, as the files in `crates/xtask/tests/` do (`clippy.toml`).
- Files are snake_case (spec §9). Format with `just fmt`.

## Git and pull requests

- Every change belongs to a ticket on the Trello board BARRAS. The card number is the ticket id `BAR-<n>`. One ticket per branch (ADR-0018).
- **Branch:** from `develop`, named `type/BAR-<n>-short-slug`, at most 50 characters. The slug is lower-case letters and digits in words joined by single hyphens. Types: `feat`, `fix`, `refactor`, `perf`, `test`, `docs`, `chore`, `ci`, `build`. `main`, `develop` and a detached `HEAD` pass the check (ADR-0018; `cargo xtask check-branch` in `pre-push` and CI).
- **Commit subject:** `type(BAR-<n>): short description starting in lower case`, at most 100 characters, with the same types. Only the merge and revert subjects that git and GitHub write are exempt: `Merge pull request #<n> from …`, `Merge branch '…'…`, `Merge remote-tracking branch '…'…`, `Merge tag '…'…`, `Merge <hex> into <hex>` and `Revert "…"…` (ADR-0018; `cargo xtask check-commit-msg` in `commit-msg`, `cargo xtask check-commits` on every commit of a pull request in CI).
- **No AI attribution** in commits or pull requests: no trailer that credits an AI tool (`Co-authored-by:`, `Signed-off-by:`, `Generated-by:`, `Assisted-by:`), no "Generated with" or "Generated by" line that names one, no link to an AI tool's session (ADR-0018). The `commit-msg` hook and CI catch the patterns and the tool names listed in `crates/xtask/src/git_rules.rs`; review catches the rest.
- Pull requests target `develop`. `main` and `develop` are protected: no direct push, no force push, merge only when the checks pass. The owner merges, by squash (ADR-0018, ADR-0025).
- **Pull request title and description:** a squash merge makes the title the subject of the commit on `develop` and may take the description as its body. So the title follows the commit subject rule and the description the attribution rule. CI checks both on every pull request, and again when either is edited (ADR-0025; `cargo xtask check-pr` in CI).
- Agents never push, open pull requests or touch production unless the owner asks in that conversation (ADR-0018).
- **Move the ticket's card as its state changes** (set by the owner; ADR-0025):
  - `EN PROCESO` when work starts.
  - `EN PRUEBA` when it is implemented and under review, or its pull request is open.
  - `APROBADA` and `PRODUCCIÓN` only after the owner approves or the pull request is merged, never on the contributor's own judgment.

## When something is not covered here

- Read the ADR for the area. If the decision does not exist, stop and ask, or propose an ADR. Do not invent a new pattern in one module (ADR-0001).
- An ADR is `docs/adr/NNNN-kebab-case-title.md` with the next free number: numbers start at 0001, are unique and leave no gap. Its first line is `# ADR-NNNN: Title` (ADR-0001; `check-structure`).
- Its status line is `Status: Proposed` until the owner accepts it, then `Status: Accepted`, or `Status: Superseded by ADR-NNNN` (ADR-0001; `check-structure`).
- It has the sections Context, Decision and Consequences (ADR-0001).
- Add its row to the index [`docs/adr/README.md`](docs/adr/README.md) in the same pull request. The index links no ADR that does not exist (ADR-0001; `check-structure`).
- A decision changes only through a new ADR. The old one gets `Superseded by ADR-NNNN`, and its Decision is not rewritten (ADR-0001).
