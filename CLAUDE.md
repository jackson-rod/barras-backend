# Working rules

These rules bind every contributor, human or agent. They restate the accepted ADRs in [`docs/adr/`](docs/adr/README.md), the foundation design ([the spec](docs/specs/2026-11-15-barras-foundation-design.md)) and the checks this repository runs. Most rules cite their source in parentheses: the ADR they restate, a section of the spec or a standard, or the check that enforces them. When a rule here and an ADR disagree, the ADR wins, and this file is fixed in the same pull request. Everything in this repository is in English: code, comments, commits, pull requests and docs (ADR-0018).

## What this repository is

- The backend of Barras, a mobile app for people in Colombia who invest. The app lives in the separate repository `barras-frontend`; the committed `openapi.json` is the contract between the two (ADR-0002).
- One Cargo workspace builds two binaries, each a composition root only, under `apps/` (ADR-0004):
  - `barras-api`: axum. HTTP under `/api/v1` and the websocket gateway in one process. Stateless, so it scales horizontally.
  - `barras-ingest`: the long-lived feed connections and the scheduled jobs that talk to outside sources.
  - The chatbot runs inside `barras-api` and reaches the language model through a port.
- Each app has a library target, and its `main.rs` only hands it the command line, so the tests reach the same code as the binary. Both build and run only on Unix: they stop on SIGTERM and SIGINT (ADR-0026).

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
- A ticket starts with a spec in `docs/specs/BAR-<n>-<topic>.md`. No code before the owner approves it (spec §12). Its implementation plan is `docs/plans/BAR-<n>-<topic>.md` (BAR-3 spec §3.5).

## Commands

Every command runs from the repository root through `just`.

| Command | What it does |
| --- | --- |
| `just` | Lists the recipes (the `default` recipe) |
| `just setup` | Installs the toolchain that `rust-toolchain.toml` names, with its components (`rustup toolchain install`); installs cargo-nextest 0.9.152, cargo-deny 0.20.2, cargo-hakari 0.9.39, cargo-llvm-cov 0.9.1 and watchexec 2.8.0 (`cargo install --locked`); installs the git hooks |
| `just dev` | Fails with `copy .env.example to .env` when `.env` is missing. Otherwise exports the variables of `.env` and runs `barras-api` under watchexec, which restarts it with SIGTERM whenever a `.rs` file or a `Cargo.toml` changes |
| `just dev-ingest` | The same for `barras-ingest` |
| `just fmt` | Formats the workspace with rustfmt |
| `just fmt-check` | Fails on unformatted code |
| `just lint` | clippy on every crate and target; any warning fails, cargo's own too (`build.warnings="deny"`), such as a member without `[lints]` or a dependency no code uses |
| `just deny` | cargo-deny: advisories, licences, bans and sources. Needs the network |
| `just check-arch` | `cargo deny check bans`, then `cargo xtask check-arch` |
| `just check-structure` | `cargo xtask check-structure`: ADRs, module crates, migration names |
| `just hakari` | Regenerates `crates/workspace-hack` (`cargo hakari generate`) and every member's line to it (`cargo hakari manage-deps`) |
| `just hakari-check` | Reads `Cargo.lock` with `--locked`, so a stale one fails it rather than being rewritten. Fails when `crates/workspace-hack` or a member's line to it is out of step with the dependency tree, or when, on a platform of `.config/hakari.toml`, the workspace's release build compiles a crate or a feature it would not compile without the generated crate, naming each. Writes nothing in the repository |
| `just openapi` | Rewrites `openapi.json` from `barras-api openapi`, through a temporary file, so a failed build leaves it whole |
| `just openapi-check` | `barras-api openapi --check openapi.json`: fails when the file is not the document the code generates |
| `just test` | With cargo-nextest, every test target not named `e2e_` or `int_`, then `just test-doc`; no Docker |
| `just test-doc` | The doctests, with `cargo test --doc`: nextest does not run them, and cargo-llvm-cov cannot measure them on stable |
| `just test-int` | With cargo-nextest, the test targets named `e2e_` or `int_`; from BAR-4 it needs Docker |
| `just test-cov` | Every test once under cargo-llvm-cov, into a fresh `target/coverage/lcov.info`; fails on a `.profraw` file outside `target/`; then `cargo xtask check-coverage` holds the thresholds |
| `just build` | Release build of the workspace |
| `just verify` | `check-arch`, `check-structure`, `hakari-check`, `fmt-check`, `deny`, `lint`, `openapi-check`, `test-cov`, `test-doc`, `build`, in that order, which is CI's: the first five compile no member, so they read the workspace before any build script of the change runs. It is what CI runs, apart from the branch, commit and pull request checks and the tree check |

- `cargo xtask` subcommands: `check-branch [name]`, `check-commit-msg <file>`, `check-commits <range>`, `check-pr <file>`, `check-arch`, `check-structure` and `check-coverage <lcov-file> [workspace-root]`, whose root is this repository unless a test gives another. Each exits 1 with one line per problem on standard error, and 2 on a command line it does not know; `check-coverage` also prints each crate's figure.
- The database commands of spec §6.14 (`db-*`) join this table with the ticket that creates them.

Git hooks (lefthook, installed by `just setup`; ADR-0018): `pre-commit` runs `just fmt-check`, `just lint`, `just check-arch` and `just check-structure`; `commit-msg` runs `cargo xtask check-commit-msg`; `pre-push` runs `cargo xtask check-branch` and `just verify`, so it needs the network, as `just deny` does, and every tool `just setup` installs. Never bypass them: `--no-verify` is not allowed, and neither is `LEFTHOOK=0`, which turns lefthook off.

## Repository layout

```
Cargo.toml              workspace; [workspace.dependencies] pins each version once; [workspace.lints]; edition 2024
Cargo.lock              committed; the checks and builds run with --locked
rust-toolchain.toml     Rust 1.100.0 with rustfmt, clippy and llvm-tools
clippy.toml             test exemptions and disallowed methods
deny.toml               cargo-deny: licences, advisories, sources, and the bans that guard the layers
justfile                every command
lefthook.yml            git hooks
openapi.json            the OpenAPI document, written by `just openapi`: the contract with the app
.env.example            every variable the binaries read, with a value for a developer machine
.cargo/config.toml      the `cargo xtask` alias
.config/
  hakari.toml           how cargo-hakari generates crates/workspace-hack
  nextest.toml          the time limit of a test
.github/                CI workflow and pull request template
apps/
  api/                  binary barras-api: composition root only
  ingest/               binary barras-ingest: composition root only
crates/
  kernel/               shared kernel: AppError, ErrorKind, Money, Currency, ids, Page and Cursor,
                        Entitlements, Clock port
  bootstrap/            the start-up the binaries share: configuration, logs, panic hook, runtime,
                        shutdown signal, failures told on standard error
  modules/<module>/     one crate per bounded context: domain, ports and use cases
  adapters/
    postgres/           sqlx repositories and mappers
    http/               axum: routers, DTOs, extractors, envelope, error mapping, OpenAPI, health,
                        internal port, metrics, server; the test client behind the `testing` feature
    realtime/           websocket gateway
    redis/              pub/sub, cache, rate limiter
    providers/          outbound clients: exchanges, feeds, DANE, Apple, Google, store, model
  testkit/              test-only helpers: the process helper, the env file reader; containers, fixtures, fakes
  workspace-hack/       generated by cargo-hakari: unifies the features of third-party crates; holds no code
  xtask/                repository checks: architecture, structure, coverage, commit and branch names
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

- A module's package is `barras-<module>` ([module anatomy](docs/standards/module-anatomy.md); `package-name`, `check-arch`).
- `cargo xtask check-structure` fails a module whose folder name is not snake_case, or that lacks `README.md`, `src/lib.rs`, `src/domain/errors.rs`, `src/ports/` or `src/application/`.
- It also fails a `src/ports/` or `src/application/` with no `.rs` file directly inside: git does not track an empty folder, so the module would fail in CI.
- It also fails any `.rs` file directly in `src/domain/` (other than `mod.rs` and `errors.rs`) or directly in `src/application/` (other than `mod.rs`) that contains no `#[cfg(test)]`. Hidden files are ignored.
- The hooks check the working tree, not the commit: a file that exists on disk but is not committed passes locally and fails in CI.
- Adapters mirror the modules: `crates/adapters/postgres/src/<module>/<name>_repository.rs` with an explicit mapper in both directions, and `crates/adapters/http/src/<module>/` with `routes.rs` and `dto/<action>.rs` (spec §6.3).

### Dependency rule

- The "May depend on" column is the design rule (ADR-0003). The "Enforced by" column names what a check enforces (ADR-0024, ADR-0026); review enforces the rest.
- `just check-arch` runs two checks: `cargo deny check bans`, then `cargo xtask check-arch`, which reads `cargo metadata` and the manifests. Each failure names the rule it breaks.
- A dependency on a workspace member is recognised by the member's package name and judged by the member's folder, never by a name written in a rule. The package in each known folder has its expected name: `barras-kernel`, `barras-<module>`, `barras-adapter-<name>`, `barras-<app>`, `barras-bootstrap`, `barras-testkit`, `workspace-hack` and `xtask` (`package-name`). So renaming a package slips no rule, and a `git` copy of a member is caught as the member is (`check-arch`; ADR-0024).
- An allowed member counts only as the path dependency at its own folder: `barras-kernel` at `crates/kernel`, `workspace-hack` at `crates/workspace-hack`. Every other allowed name counts only from crates.io (`check-arch`; ADR-0024).
- For every crate, in any kind of dependency: no path dependency outside the members' folders (`path-outside-members`), and no dependency on a crate under `apps/` (`apps-are-roots`) (`check-arch`; ADR-0024).
- A member's `testing` feature is turned on only in a dev-dependency, and in a crate's own `[features]` only its `testing` feature may turn on `testing`, `<dep>/testing` or `<dep>?/testing`, never `default` or another feature. The testkit is no exception (`testing-dev-only`, `check-arch`; ADR-0024).
- No `[patch]` or `[replace]` table in the root `Cargo.toml`, and no `[patch]` table in `.cargo/config.toml` (`no-patch`, `check-arch`; ADR-0024).
- Every folder directly under `apps/`, `crates/modules/` and `crates/adapters/` that holds a `Cargo.toml`, and `crates/kernel`, `crates/bootstrap`, `crates/testkit`, `crates/workspace-hack` and `crates/xtask`, is listed in `members` (`not-a-member`); a crate in any other folder fails (`unknown-location`) (`check-arch`; ADR-0024).
- `cargo deny check bans` lets only these crates depend directly on a technology crate; every other crate, apps included, fails, and an app reaches them through its adapter (`deny.toml`; ADR-0024):
  - `sqlx`: `barras-adapter-postgres` and `barras-testkit`.
  - `axum`: `barras-adapter-http` and `barras-adapter-realtime`, and the third-party `utoipa-axum`.
  - `axum-core`, `hyper` and `hyper-util`: the HTTP and realtime adapters, and the third-party crates that wrap them (`axum`; `hyper-util` for `hyper`).
  - `utoipa-axum`: `barras-adapter-http`.
  - `reqwest`: `barras-adapter-providers`.
  - `redis`: `barras-adapter-redis` and `barras-testkit`.
- No crate, an adapter included, depends on `actix-web`, `rocket`, `warp`, `poem`, `salvo`, `tide`, `diesel`, `sea-orm`, `tokio-postgres`, `ureq`, `surf`, `isahc` or `curl`: each would be a second way to do what one adapter owns. A `*` version requirement fails, a path dependency between our crates aside. cargo-deny reads every feature of every member, so a banned crate cannot hide behind an optional dependency (`deny.toml`; ADR-0024).
- No crate turns on axum's `json`, `query` or `form` feature, nor its `default`, which turns on all three: they bring axum's own `Json`, `Query` and `Form` extractors, whose rejections are not envelopes and quote the caller's input (`[[bans.features]]` in `deny.toml`; ADR-0024, ADR-0029).

| Crate | May depend on | Enforced by |
| --- | --- | --- |
| `crates/kernel` | `std` and the small pure crates `thiserror`, `uuid`, `time`, `rust_decimal` | `check-arch`: normal and build dependencies only from that list and `workspace-hack`; dev dependencies from them plus `tokio`, `mockall`, `insta` (`kernel-allowlist`, `kernel-dev-allowlist`); no build script (`no-build-script`) |
| `crates/modules/<m>` | `barras-kernel`, the same pure crates and `async-trait`. Never another module, an adapter, axum, sqlx, reqwest, redis, tokio's runtime or serde_json | `check-arch`: normal and build dependencies only from that list and `workspace-hack`; dev dependencies from them plus `tokio`, `mockall`, `insta` (`module-allowlist`, `module-dev-allowlist`); no build script (`no-build-script`) |
| `crates/adapters/*` | `barras-kernel`, the module crates and its own technology. Never another adapter | `check-arch`: no other adapter (`adapter-isolation`). cargo-deny bans: a technology crate only in its own adapter. Any other library is left to review |
| `crates/bootstrap` | the start-up the binaries share: configuration, logs, panic hook, runtime, shutdown signal, failures told on standard error. No workspace crate but `workspace-hack` | `check-arch`: no other member (`bootstrap-isolation`); only `apps/*` depend on it, in any kind (`bootstrap-apps-only`). cargo-deny bans: no technology crate. Any other library is left to review (ADR-0026) |
| `apps/*` | everything. The only place where a concrete adapter meets a port | cargo-deny bans: no direct `sqlx`, `axum`, `axum-core`, `hyper`, `hyper-util`, `utoipa-axum`, `reqwest` or `redis`. `check-arch`: `barras-testkit` only as a dev dependency (`testkit-dev-only`); no crate depends on an app (`apps-are-roots`) |
| `crates/testkit` | test-only helpers: containers, fixtures, fakes, the process helper | cargo-deny bans: `sqlx` and `redis` allowed directly, the other technology crates not. `check-arch` sets no list; every crate that uses it takes it only as a dev dependency (`testkit-dev-only`) |
| `crates/workspace-hack` | generated by cargo-hakari: third-party crates and their features, no code | `check-arch`: every dependency from crates.io (`hack-isolation`); one library at `src/lib.rs` holding doc comments at most, at most a `build.rs` holding `fn main() {}`, nothing else (`hack-is-a-stub`). `just hakari-check`: in step with the dependency tree, and changing no release build |
| `crates/xtask` | no workspace crate | `check-arch`: no member, `workspace-hack` included (`tool-isolation`) |

Reject on sight: a framework type in a module crate; a database row type in a use case signature; a handler calling a repository; a module depending on another module; `unwrap`, `expect` or `panic!` outside tests (ADR-0003, ADR-0006).

### Adding a crate or a dependency

- List the crate in `members` of the root `Cargo.toml`. The list is explicit, with no globs (`Cargo.toml`). A crate folder of the table left out of it fails (`not-a-member`, `check-arch`).
- Its folder decides its layer: `crates/kernel`, `crates/modules/<module>`, `crates/adapters/<name>`, `apps/<name>`, `crates/bootstrap`, `crates/testkit`, `crates/workspace-hack` or `crates/xtask`. A crate anywhere else fails (`unknown-location`, `check-arch`).
- Its folder also decides its package name: `barras-<module>` for a module, `barras-adapter-<name>` for an adapter, `barras-<name>` for an app (`package-name`, `check-arch`).
- Give its `Cargo.toml` the fields every member has, under that name:

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
- After adding a member, or changing a dependency or its features, run `just hakari` and commit what it changes: `crates/workspace-hack/Cargo.toml`, the members' `workspace-hack.workspace = true` lines and `Cargo.lock`. `just hakari-check` fails while the generated crate or a member's line to it would change; `just verify` and CI run it (ADR-0024).
- Every member that depends on the generated crate takes whatever it holds into its release build. `just hakari-check` fails, naming the crate and the feature, when the generated crate changes what the workspace's release build compiles. When only tests ask for that feature, the crate joins `third-party` under `[traversal-excludes]` in `.config/hakari.toml`, never `[final-excludes]`; then run `just hakari`. Only hakari writes `crates/workspace-hack/Cargo.toml`, its `[lints.cargo]` table aside (ADR-0024).
- `barras-testkit` and `xtask` are left out of hakari's traversal (`workspace-members` under `[traversal-excludes]`): they take no line to the generated crate, and their dependencies never shape it (ADR-0024).
- A new member that depends on the generated crate names it once, in `lib.rs`, or in `main.rs` when it has no library. Without that line, cargo can report it as a dependency no code uses, which fails `just lint` (ADR-0024):

```rust
// Taken for its features, not code; named so that cargo never reports it as an unused dependency.
use workspace_hack as _;
```

- If `cargo deny check bans` names `workspace-hack` under a ban, add it to that ban's wrappers in `deny.toml`, with a comment that says why (ADR-0024).
- A member's test support lives behind a feature named `testing`, under `#[cfg(any(test, feature = "testing"))]`, and other crates turn it on only in their dev-dependencies (`testing-dev-only`, `check-arch`).
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
- JSON field names are camelCase (ADR-0029).
- A route with nothing to return answers 200 with `data: null` and documents `SuccessEnvelope<NoData>`; never 204 (ADR-0029; `metadata_problems`).
- Money and any integer above 2^53 travel as strings (ADR-0005, ADR-0008).
- Every answer the application gives on the public port carries `x-request-id`: the caller's when it is 1 to 64 characters of `A-Z a-z 0-9 . _ -`, a new UUID v7 otherwise. hyper answers a request head it cannot read itself (400, 414 or 431), with an empty body: no envelope, no request id, nothing logged or counted (ADR-0029).
- A request body is a JSON object, sent as `application/json`, of at most 1 MiB. An answer of 1 KiB or more is compressed with gzip or brotli when `Accept-Encoding` allows (ADR-0029).
- The HTTP adapter's own codes, besides those of the module errors (ADR-0029):

| `error.code` | Status | When |
| --- | --- | --- |
| `VALIDATION_FAILED` | 422 | A body or query field is missing, has the wrong type or breaks a rule; `error.fields` says which |
| `MALFORMED_REQUEST` | 400 | The body is not UTF-8, not JSON or not a JSON object, or the query string cannot be read |
| `UNSUPPORTED_MEDIA_TYPE` | 415 | A body not sent as `application/json` or `application/<x>+json` |
| `PAYLOAD_TOO_LARGE` | 413 | A body over 1 MiB |
| `NOT_ACCEPTABLE` | 406 | `Accept-Encoding` refuses every encoding, the plain body included |
| `ROUTE_NOT_FOUND` | 404 | No route matches the path |
| `METHOD_NOT_ALLOWED` | 405 | The path exists, the method does not |
| `SERVICE_UNAVAILABLE` | 503 | `GET /api/v1/health` with a critical probe failing |
| `INTERNAL_ERROR` | 500 | A handler panicked |

- `openapi.json` is generated from the code and committed with the change: run `just openapi` after any route change, or `just openapi-check` fails. It is the contract with the app, from BAR-5 including the realtime event names and payloads (ADR-0002, ADR-0028).
- Add a route only with `ApiRouter::route(routes!(handler), access, codes)`: its `#[utoipa::path]` handlers, an `Access` value with the permission and the plan feature it needs (`Access::public()` for none), and its own error codes, without the seven codes every route can return (`x-common-error-codes`). The HTTP adapter's crate doc shows one (ADR-0028).
- Only `ApiRouter::route` adds a route to the public API: `ApiRouter`'s fields are private, and `undocumented_route`, which serves a route the document lacks, exists only behind the adapter's `testing` feature. The gap left: `route` takes the tuple `routes!` makes, and a tuple built by hand can serve a method its document lacks, which only `TestClient`'s route guard sees, on a request a test sends (ADR-0028).
- The handler's `#[utoipa::path]` has one tag, a summary (the first line of its doc comment), a description on every response, `SuccessEnvelope<T>` on its 2xx and `ErrorEnvelope` on every error status it can return. `metadata_problems`, run by an e2e test, fails an operation that lacks one or has more than one tag, and an operationId used twice (ADR-0017, ADR-0028).
- `TestClient` fails a test when a documented route answers an error envelope whose `error.code` neither its `x-error-codes` nor `x-common-error-codes` lists, or whose status its operation does not document (as itself, as its range such as `4XX`, or with `default`), the statuses of the common codes aside: 400, 404, 405, 406, 413, 415 and 500 (ADR-0028).
- A schema name is registered once: when two modules have a type of the same name, one takes `#[schema(as = …)]`. The app refuses to build with a route the document cannot describe (ADR-0028).

### Errors

- Every use case and port returns `Result<T, <Module>Error>`. No exceptions, and no `anyhow` outside `main.rs` (ADR-0006).
- `<Module>Error` is one enum per module, in `src/domain/errors.rs`, with one variant per failure case (ADR-0006; spec §6.3).
- Each variant has a stable `SCREAMING_SNAKE_CASE` code and an `ErrorKind`: validation, unauthenticated, forbidden, not found, conflict, rate limited, unavailable or internal (ADR-0006).
- The code is public API: renaming it is a breaking change (ADR-0006).
- The domain does not know HTTP. Only the `http` adapter maps `ErrorKind` to a status. A handler returns `ApiResult<T>`, and `?` turns any `AppError` into its answer (ADR-0006, ADR-0029).
- The `Display` of a module error never interpolates a source or a value the caller sent: a 4xx body carries it verbatim. A 5xx answers a generic message and logs the error with its chain of sources (ADR-0029).
- Never swallow an error. Handle it, wrap it with its source, or return it (ADR-0006).

### Validation

- A handler is thin: validate the DTO, convert it into the use case's input, call one use case, map the output (spec §6.3).
- A handler takes its body with `ValidatedJson<T>` and its query with `ValidatedQuery<T>`, never axum's `Json` or `Query`, whose rejections are not envelopes. Every crate that takes axum keeps `default-features = false`, and cargo-deny refuses axum's `json`, `query` and `form` features, so those extractors exist nowhere (ADR-0029; `deny.toml`).
- A route that takes a validating extractor lists `VALIDATION_FAILED` among its codes and documents a 422 (ADR-0029; review).
- Validate at the boundary. Inner layers trust their inputs (code standard).
- A domain type owns its invariants: its constructor returns `Result` (spec §6.3).
- A validation failure answers 422 `VALIDATION_FAILED` with `error.fields`, a map from each field's JSON path (`address.zip`, `tags[1]`) to messages. A module error of kind `Validation` answers 422 with its own code and no `error.fields` (ADR-0005, ADR-0029).
- Every DTO, body and query alike, takes `#[serde(rename_all = "camelCase")]` and never renames a field one by one: garde 0.23 parses `#[garde(rename(…))]` but never applies it (ADR-0029).
- A DTO holds no map keyed by caller data, such as a `HashMap<String, _>`: its keys would come back as field keys. A list of objects takes its place (ADR-0029).
- An error message states what failed and on which input, never the value the caller sent, which may be a secret. A field's message comes from a fixed set (`is required`, `expected <type>`, `has the wrong type`, `is not one of the accepted values`, `has a field it does not accept`, `is not valid`) or is garde's rule message. A DTO type's own `Deserialize` or `TryFrom` error and a garde `custom` rule's message reach the client as written, so they never quote their input (code standard; ADR-0029).
- A handler that validates a JSON body itself keys its errors by paths in that body, JSON names in camelCase. `TestClient` fails a test whose 422 has a key with `_` in a name or, answering a JSON body, a key that names nothing that was sent and no field missing from it (ADR-0029).

### Authentication and authorisation

- Barras owns identity. Email and password use argon2id, with a verification code by email (ADR-0009).
- Apple and Google sign-in verify the provider's ID token against the provider's keys: issuer, audience, nonce and expiry. The same verified email links to one user (ADR-0009).
- The app sends `Authorization: Bearer <access token>`. Access tokens are signed JWTs that live 15 minutes (ADR-0009). A 401 carries `WWW-Authenticate: Bearer` (ADR-0029).
- Refresh tokens rotate on every use and are stored hashed. Reuse of an old one revokes the session. No cookies, no CSRF (ADR-0009).
- The access token carries the role and the plan's entitlements; no request reads the database to authorise. When a plan changes, the server sends a realtime event and the app refreshes its token (ADR-0010).
- A new user starts on Lite. Entitlements can carry an expiry, so a Pro trial ends on its own (ADR-0010).
- Permissions are a catalog in code, seeded into `permissions` and granted to roles through `role_permissions`. Each route declares the permission and the plan feature it needs, through its `Access` value (ADR-0010, ADR-0028).
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
- `APP_ENV=local` writes readable lines; any other value writes one JSON object per line with exactly the keys `timestamp` (RFC 3339, UTC), `level`, `target`, `fields` (the event's fields, `message` included) and, inside a span, `spans`. Whatever an event or a span records stays inside `fields` or `spans` (ADR-0026).
- Every event has a message, as in `tracing::info!(%addr, "listening")`: the process tests read `fields.message` of each line.
- Never log credentials, tokens or request bodies (ADR-0015). The request log records the method, the route template (or `unmatched`), the request id, the status and the duration, never a header, the query string or the body (ADR-0029).
- A 5xx logs one `error` event, `request failed`, with its code, status and detail; a handler panic also logs the panic hook's event (ADR-0026, ADR-0029).
- A panic message never formats a value the caller sent: every panic is logged with its payload, by the panic hook and, for a handler's, in the detail of `request failed` (ADR-0026, ADR-0029).
- `GET /api/v1/health` (ready: `ok`, `degraded`, or 503) and `GET /api/v1/health/live` are public and never name a dependency. Every probe runs at once, with 2 seconds to answer (ADR-0015, ADR-0029).
- Per-dependency detail and Prometheus metrics are only on an internal port: `GET /health` and `GET /metrics`, with no `/api/v1` prefix, no envelope and none of the public middleware, and not in `openapi.json` (ADR-0015, ADR-0029).
- The metrics live in a registry the composition root creates and hands over; nothing is global. A label takes bounded values: the route template, never the path (ADR-0029).
- Each binary loads one typed configuration from the environment at start and refuses to boot when a variable is missing or not valid. It prints one line per problem to standard error, all at once, naming the variable and what is expected and never the value, and exits non-zero before it opens a socket (ADR-0016, ADR-0026).
- Every variable is required; there is no default in code. An empty value counts as missing (BAR-3 spec §8.1).
- The variables: `APP_ENV` (`local`, `staging` or `production`) and `LOG_LEVEL` (`trace`, `debug`, `info`, `warn` or `error`), read by both binaries; `API_BIND_ADDR` and `API_INTERNAL_BIND_ADDR`, read by `barras-api`; `INGEST_INTERNAL_BIND_ADDR`, read by `barras-ingest`.
- Every variable is listed in `.env.example`, with a value that works on a developer machine (ADR-0016). The e2e tests check it both ways: each binary's variables are in the file, and each variable in the file is read by `barras-api` or starts with `INGEST_`, and is read by `barras-ingest` or starts with `API_`. So a variable that one binary alone reads takes its prefix. A value holds only letters, digits and `. _ : / @ % + , = [ ] -`, which a shell and a dotenv loader read the same way (`env_file` in `barras-testkit`).
- Only `Environment::from_process` of `barras-bootstrap` reads the environment of a binary (ADR-0016, ADR-0026). clippy denies `std::env::var`, `std::env::var_os`, `std::env::vars` and `std::env::vars_os` (`clippy.toml`); that function carries one `#[allow]`. The other is the test-support exception: the testkit's process helper reads `LLVM_PROFILE_FILE` and passes it to each binary it starts, so that the binary's coverage is measured (`crates/testkit/src/process.rs`; ADR-0027). Review covers the rest, such as a library that reads a variable itself: `build_runtime` of `barras-bootstrap` gives tokio its worker count, so it never reads `TOKIO_WORKER_THREADS` (ADR-0029).
- After SIGTERM or Ctrl-C a binary stops accepting connections, gives the requests in flight at most 10 seconds, and exits 0 (ADR-0029).

## Definition of done

A change is done only when the same pull request contains (ADR-0017):

1. **Tests per layer.** Domain types tested directly. Use cases with fake ports. Mappers round trip in both directions. Routes through e2e tests against the real app. Every adapter to an outside system with a contract test. Errors asserted by `code`, never by message.
2. **OpenAPI metadata on every route**, including the error codes it can return and the permission and plan feature it needs.
3. **Docs.** The module `README.md` is current. A new or changed decision is an ADR, listed in the index.
4. **`just verify` is green** locally and in CI.

- The pull request template carries the four items as checkboxes (`.github/pull_request_template.md`).

### Test tiers

- A test target's file name decides its tier, so a new test has nothing to register (ADR-0027):
  - `tests/e2e_*.rs` drives a whole binary: its app in-process with the HTTP adapter's `TestClient`, or its real process with the testkit's `BinaryCommand`.
  - `tests/int_*.rs` drives one adapter against a real outside system (from BAR-4).
  - `just test` runs every other test; `just test-int` runs the `e2e_` and `int_` targets.
- An adapter's tests that need its `testing` feature live in `src/tests/`, declared `#[cfg(test)] mod tests;`: under `tests/` they would need a dependency of the adapter on itself (ADR-0027).
- A test still running after two minutes is killed and fails under its own name (`.config/nextest.toml`).

### Coverage

- At least 95% of the production lines of `crates/kernel` and of each module crate, each on its own, and 85% of the production lines of the measured crates together (ADR-0017, ADR-0027).
- `just test-cov` measures every test once with cargo-llvm-cov; then `cargo xtask check-coverage` prints each crate's figure and fails under either bar. `just verify` and CI run it.
- Production lines leave out every item marked exactly `#[cfg(test)]`, at any depth. Measured: the `src/` of the kernel, the modules, the adapters, `crates/bootstrap`, `crates/xtask` and the apps, a binary started by a process test included. Not measured: `barras-testkit`, `workspace-hack`, `tests/`, `benches/`, `examples/`, `build.rs` and the doctests (ADR-0027).
- A source that holds a function body outside test code must be in the report, so a crate whose tests never run fails (ADR-0027).
- `check-coverage` refuses these shapes in a measured `src/`, naming the file and the line (ADR-0027):
  - Outside test code: `#[cfg(test)]` on anything but an item; a `cfg` that names `test` in another form than `test` or `any(test, feature = "…")`; a `cfg` or `cfg_attr` predicate built from anything but `test`, `feature = "…"`, `debug_assertions`, `unix`, `windows` and the `target_*` keys with `all`, `any` and `not`, or with `not` around a feature or `debug_assertions`; a predicate that names `coverage`, a `coverage(…)` attribute, a `cfg_attr` that applies `cfg`; a test attribute (`#[test]`, `#[tokio::test]`) outside a `#[cfg(test)]` item; a module named `tests`, or an out-of-line one named `*_tests`, without `#[cfg(test)]`; the keyword `mod` or a test attribute in a `macro_rules!` body.
  - Anywhere: `#[path]`; `include!` other than `include!(concat!(env!("OUT_DIR"), "…"))`, and a `use` that names `include`; an out-of-line `#[cfg(test)]` module not named `tests`, or an out-of-line module inside test code that is not named `tests` and does not sit in a module named `tests`.
  - Files: a `*_tests.rs` or `*-tests.rs` file, an `examples` or `benches` folder, a symlink or a special file; a source over 1 MiB; brackets nested deeper than 256 levels.
- So test code is inline under `#[cfg(test)]`, in a `tests.rs` file, or under a `tests/` folder in `src/`. Such a file is read only for `#[path]` and `include` (ADR-0027).
- The kernel and the modules have no build script (`no-build-script`, `check-arch`; ADR-0024).

## Code style

- Follow [`docs/standards/code-standard.md`](docs/standards/code-standard.md): comments, duplication, naming, size, error handling. Where it and the module anatomy disagree on structure or naming, the module anatomy wins (spec §9).
- Comment why, never what (code standard).
- `unsafe` is forbidden (`unsafe_code = "forbid"` in `Cargo.toml`; spec §9).
- clippy runs with zero warnings. It denies `unwrap`, `expect`, `panic!`, `dbg!` and printing to stdout or stderr outside tests, and `todo!`, `unimplemented!` and reading the environment with `std::env::var`, `var_os`, `vars` or `vars_os` everywhere (`[workspace.lints]`, `clippy.toml`; spec §9).
- Every crate inherits the workspace lints with `[lints] workspace = true` (`Cargo.toml`). `crates/workspace-hack` is the exception: it holds no code, and its own `[lints.cargo]` table allows cargo's unused-dependency lint (ADR-0024).
- No `#[allow]` without the reason on the same line, as in `#![allow(clippy::print_stdout, clippy::print_stderr)] // a CLI: its output is the product` (spec §9).
- clippy's test exemption covers only `#[test]` functions and `#[cfg(test)]` modules. So an integration-test crate (`tests/*.rs`), `barras-testkit` and a member's `testing` module need a crate- or module-level `#![allow(clippy::unwrap_used, clippy::expect_used)]`, its reason on the same line, to use `unwrap` or `expect` in their helpers, as the files in `crates/xtask/tests/` do (`clippy.toml`).
- Files are snake_case (spec §9). Format with `just fmt`.

## Git and pull requests

- Every change belongs to a ticket on the Trello board BARRAS. The card number is the ticket id `BAR-<n>`. One ticket per branch (ADR-0018).
- **Branch:** from `develop`, named `type/BAR-<n>-short-slug`, at most 50 characters. The slug is lower-case letters and digits in words joined by single hyphens. Types: `feat`, `fix`, `refactor`, `perf`, `test`, `docs`, `chore`, `ci`, `build`. `main`, `develop` and a detached `HEAD` pass the check (ADR-0018; `cargo xtask check-branch` in `pre-push` and CI).
- **Commit subject:** `type(BAR-<n>): short description starting in lower case`, at most 100 characters, with the same types. Only the merge and revert subjects that git and GitHub write are exempt: `Merge pull request #<n> from …`, `Merge branch '…'…`, `Merge remote-tracking branch '…'…`, `Merge tag '…'…`, `Merge <hex> into <hex>` and `Revert "…"…` (ADR-0018; `cargo xtask check-commit-msg` in `commit-msg`, `cargo xtask check-commits` on every commit of a pull request in CI).
- **No AI attribution** in commits or pull requests: no trailer that credits an AI tool (`Co-authored-by:`, `Signed-off-by:`, `Generated-by:`, `Assisted-by:`), no "Generated with" or "Generated by" line that names one, no link to an AI tool's session (ADR-0018). The `commit-msg` hook and CI catch the patterns and the tool names listed in `crates/xtask/src/git_rules.rs`; review catches the rest.
- **No skip-CI marker** in a commit message or in a pull request's title or description: `[skip ci]`, `[ci skip]`, `[no ci]`, `[skip actions]` and `[actions skip]`, in any case and with any spaces inside the brackets, and a `skip-checks: true` trailer wherever it stands. GitHub would start no CI run, so no check would see the change (ADR-0018; `check-commit-msg`, `check-commits`, `check-pr`).
- Pull requests target `develop`. `main` and `develop` are protected: no direct push, no force push. Merge only when the checks pass; GitHub enforces that only once `checks`, `test` and `build` are required status checks on both branches, an open owner decision. The owner merges, by squash (ADR-0018, ADR-0025).
- **Pull request title and description:** a squash merge makes the title the subject of the commit on `develop` and may take the description as its body. So the title follows the commit subject rule and the description the attribution and skip-CI rules. GitHub pre-fills the title from the branch name when the branch has more than one commit: replace it. CI checks both on every pull request, and again when either is edited (ADR-0025; `cargo xtask check-pr` in CI).
- Agents never push, open pull requests or touch production unless the owner asks in that conversation (ADR-0018).
- **Move the ticket's card as its state changes** (set by the owner; ADR-0025):
  - `EN PROCESO` when work starts.
  - `EN PRUEBA` when it is implemented and under review, or its pull request is open.
  - `APROBADA` and `PRODUCCIÓN` only after the owner approves or the pull request is merged, never on the contributor's own judgment.

## When something is not covered

- Read the ADR for the area. If the decision does not exist, stop and ask, or propose an ADR. Do not invent a new pattern in one module (ADR-0001).
- An ADR is `docs/adr/NNNN-kebab-case-title.md` with the next free number: numbers start at 0001, are unique and leave no gap. Its first line is `# ADR-NNNN: Title` (ADR-0001; `check-structure`).
- Its status line is `Status: Proposed` until the owner accepts it, then `Status: Accepted`, or `Status: Superseded by ADR-NNNN` (ADR-0001; `check-structure`).
- It has the sections Context, Decision and Consequences (ADR-0001).
- Add its row to the index [`docs/adr/README.md`](docs/adr/README.md) in the same pull request. The index links no ADR that does not exist (ADR-0001; `check-structure`).
- A decision changes only through a new ADR. The old one gets `Superseded by ADR-NNNN`, and its Decision is not rewritten (ADR-0001).
