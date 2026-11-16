# BAR-3: Core contract

Date: 2026-11-15. Status: approved by the owner on 2026-11-15.

Ticket: BAR-3 (F-03), <https://trello.com/c/eAkMTw1P>. Depends on BAR-2. Design: [foundation design](2026-11-15-barras-foundation-design.md) §6.6, §6.7, §6.12, §6.13 and §9.

**Decided** marks what the owner said. **Call** marks a decision of this spec that the owner has not confirmed one by one; approving the spec accepts them.

## 1. Intent

Every module and every route of Barras builds on one contract: how an error is named, what a response looks like, how a binary reads its configuration, what it logs, how it reports its health, and how the app learns the API. This ticket builds that contract once, with the checks that keep it true, before the first module exists.

After this ticket:

- A module author returns a typed error and gets the right HTTP status and envelope without writing any HTTP code.
- A route cannot be added to the public API without OpenAPI metadata, and `openapi.json` cannot drift from the code.
- Both binaries start from a typed configuration, log JSON, answer health and metrics, and stop cleanly.
- `just verify` fails when coverage drops under the thresholds.

## 2. Scope

In scope:

1. `barras-kernel`: `ErrorKind`, `AppError`, `Currency`, `Money`, typed ids (UUID v7), `PageLimit`, `Cursor`, `PageRequest`, `Page`, `Plan`, `Feature`, `Entitlements`, the `Clock` port.
2. `barras-adapter-http`: success and error envelopes, `ErrorKind` to HTTP status, validating extractors, request id, compression, request logging, HTTP metrics, health routes, the internal router, the OpenAPI document, the server, and test helpers.
3. `barras-bootstrap`, the start-up shared by the binaries: typed configuration from the environment, logging, shutdown signal (section 4.3).
4. `barras-api` and `barras-ingest` as real processes: `build_app()`, two listeners for the API, one for the ingestor, `barras-api openapi`.
5. `openapi.json` committed at the repository root, `just openapi`, `just openapi-check`.
6. The e2e harness and the guard that fails on a route without OpenAPI metadata.
7. `just test-int`, `just test-cov`, the coverage thresholds, cargo-hakari, `just dev` with reload, CI.
8. The fixes to the repository checks that section 3.6 assigns to this ticket.
9. Docs: `CLAUDE.md`, `README.md`, `CONTRIBUTING.md`, the standards, the ADRs of section 12.

Not in this ticket:

- Postgres and Redis (BAR-4, BAR-5). No container is started (section 3.4).
- Authentication, the permission catalog and route guards (BAR-15, BAR-17). This ticket only fixes where a route declares its permission and plan feature (section 7.2).
- Realtime event names and payloads in `openapi.json` (BAR-5).
- A money DTO. The kernel gives `Money` its string form; the first route that returns money adds the DTO.
- A server-side request timeout and rate limiting. Neither is on the card. The timeout belongs with the first slow dependency (BAR-4); it is named here so that it is not forgotten.
- Container images and deployment (BAR-12). OpenTelemetry export.

## 3. Decisions taken with the owner

The owner answered these six questions on 2026-11-15, in the review of this spec. Each changes what gets built.

### 3.1 ADR-0024 stands as written (**Decided**)

`apps/*` and `barras-testkit` never depend directly on `axum` or `reqwest`. The e2e tests live in `apps/api/tests/` and must build the real app, send it requests and read the answers, so the HTTP adapter exports what they need: a `serve` function that takes the listeners, and, behind a `testing` feature, a `TestClient` that calls the app in-process and a small client for a real socket. No app and no test names an axum type. ADR-0024 becomes `Accepted` in this ticket, with the amendments of section 12.

Not taken: relaxing the ban for `barras-testkit`, which would add an exception and bring `reqwest` into the build long before the first outbound adapter.

### 3.2 The OpenAPI generator is a mode of `barras-api` (**Decided**)

It cannot live in `crates/xtask`: the `tool-isolation` rule bars workspace crates there. `barras-api openapi` prints the document; `barras-api openapi --check openapi.json` fails when the file is stale. The mode needs no environment variable and opens no socket. The document comes from the same function that builds the router the binary serves, so there is no second list of routes. From BAR-5 the document also carries the realtime events, which live in another adapter, and only the composition root sees both adapters.

Not taken: a second binary inside `barras-adapter-http`, which sees only the HTTP adapter.

### 3.3 Shared start-up code lives in a new crate, `barras-bootstrap` (**Decided**)

Reading the environment into a typed configuration, starting the logs and waiting for the shutdown signal are the same in `barras-api` and `barras-ingest`, and no row of the dependency table fitted them. `crates/bootstrap` gets its own row: it depends on no workspace crate, and only `apps/*` may depend on it. `check-arch` enforces both (`bootstrap-isolation`, `bootstrap-apps-only`), and ADR-0026 records the row.

Not taken: an adapter crate, which would implement no port; and one copy in each binary, kept in step by review.

### 3.4 Docker is first needed in BAR-4, not here (**Decided**)

This ticket has no database and no Redis, so its e2e tests build the app in-process or start the binary itself, and start no container. `just test-int` is created here with the rule that selects its tests (section 9.1); BAR-4 adds the containers behind the same command. The README row "Docker, from BAR-4" stays true.

### 3.5 Specs and plans are named `BAR-<n>-<topic>.md` (**Decided**)

`docs/specs/BAR-<n>-<topic>.md` and `docs/plans/BAR-<n>-<topic>.md`, as `CLAUDE.md`, `CONTRIBUTING.md` and the foundation design §12 already say. The foundation design and the kickoff plan keep their dated names: neither belongs to one ticket.

Which repository holds the spec of a ticket that touches backend and frontend is not needed here and is asked before BAR-14.

### 3.6 The deferred fixes to the checks are split (**Decided**)

The BAR-2 reviews left a list on the card. This ticket takes the fixes that change the same files as the contract, or that would otherwise widen a gap:

1. `deny.toml`: `utoipa-axum` as a wrapper of `axum`; the licences the new dependency tree needs; `unmaintained = "workspace"` if a transitive crate forces it; `wildcards = "deny"` with `allow-wildcard-paths`; bans on `axum-core`, `hyper` and `hyper-util` outside the HTTP and realtime adapters (this ticket is where a bypass of the `axum` ban first becomes possible), and a plain deny list of alternative web, database and HTTP-client crates.
2. CI: in the `checks` job the pull request text step moves after the code checks, so a bad title no longer hides them.
3. `check-arch`: `barras-kernel` counts only at the workspace member's path. The kernel becomes real in this ticket.
4. clippy: `std::env::var_os`, `std::env::vars` and `std::env::vars_os` join `std::env::var` in the disallowed methods. This is the configuration ticket.
5. Documents: ADR-0003 (Consequences) and the design §6.4 say `check-arch` fails on any dependency outside a crate's allowlist, which is true only of the kernel and the modules; ADR-0016 says no check enforces the environment rule; the README rows and its two present-state sentences; CONTRIBUTING on the pre-filled pull request title; `llvm-tools` in `rust-toolchain.toml` (coverage needs it). `rust-src` is not added: only editors use it, and the README says how to add it locally.

The rest goes to a new ticket of its own, BAR-92 (F-14, `fix`, pure `xtask`; Trello numbered the new card 92, not 62), because nothing here depends on it and this ticket is already the largest of the foundations:

- `no-patch` with a real TOML parser, and the legacy `.cargo/config`.
- Attribution: whole-word names, `Co-authored-by :`, a quoted `> Co-authored-by:` line, "Made with <tool>".
- `check-pr` never exempts a title written like a merge subject.
- `check-commits` passes `--end-of-options`; `check-branch HEAD` given explicitly; tests through the binary for the other subcommands.
- `check-structure`: nested folders, and `Superseded by` an ADR that does not exist.
- `not-a-member`: the double space and the missing hint.

### 3.7 cargo-hakari is adopted in this ticket (**Decided**)

A spike on a copy of the repository measured what it costs, and the owner chose to adopt it now, as the card says:

- Three gates need an exception: `check-arch` (a new folder, and a path dependency in every allowlist), cargo's own `unused_dependencies` lint under `just lint`, and the cargo-deny bans once a banned crate enters the generated crate.
- The kernel then waits for every third-party dependency on a cold build. In the spike's copy, with only tokio, serde and uuid added, that was 18 crates against 5 and 2.6 s against 0.8 s; it grows once axum and sqlx arrive. An incremental check is not affected.
- What it saves is building a shared dependency a second time when two commands ask for different features. That second build happens once and stays cached.

Section 4.6 says how it is wired.

## 4. Crates and layers

### 4.1 `barras-kernel`

No I/O. Dependencies: `thiserror`, `uuid` (feature `v7`), `time`, `rust_decimal`. No serde: the wire form of a kernel type is the adapter's job.

| Item | Behaviour |
| --- | --- |
| `ErrorKind` | `Validation`, `Unauthenticated`, `Forbidden`, `NotFound`, `Conflict`, `RateLimited`, `Unavailable`, `Internal` |
| `AppError` | A trait every `<Module>Error` implements: `code() -> &'static str` (stable `SCREAMING_SNAKE_CASE`) and `kind() -> ErrorKind`. Its `Display` text is the developer message |
| `KernelError` | The kernel's own error enum; implements `AppError`. Its codes are in section 6.2 |
| `Currency` | An ISO 4217 alphabetic code: exactly three upper-case ASCII letters. `Currency::new` returns `Result`; `Currency::COP` and `Currency::USD` are constants. Which currencies Barras offers is data (BAR-19), not this type |
| `Money` | A `rust_decimal::Decimal` amount with its `Currency`, sized as `numeric(20,2)`: at most 2 decimal places and 18 integer digits, kept at scale 2. May be negative (a loss, a change). `Money::new` and `Money::parse(amount, currency)` return `Result`. `checked_add` and `checked_sub` fail on two currencies or on overflow. The amount prints as a plain decimal string with two places (`"1234.50"`), which is its wire form. No multiplication, division or conversion: the capital spec owns rounding |
| Typed ids | `define_id!(Name)` makes a newtype over a UUID: `generate()` (UUID v7), `from_uuid`, `as_uuid`, `parse(&str)` returning `Result`, `Display` in the canonical hyphenated form. The kernel defines `UserId`, which every module refers to; a module defines its own ids with the macro |
| `PageLimit` | 1 to 50; default 20 (**Call**). The constructor returns `Result` |
| `Cursor` | An opaque token: 1 to 512 characters of `A-Z a-z 0-9 - _`. What it encodes is the repository's business |
| `PageRequest`, `Page<T>` | `PageRequest { limit, after: Option<Cursor> }`. `Page<T> { items, next: Option<Cursor> }` with `map` |
| `Plan`, `Feature` | `Plan`: `Lite`, `Pro`. `Feature`: `Assistant`, `AllSuggestions`, `AllNews`, `FullHistory`, `StockTracker`, each with a stable code (`ASSISTANT`, …). `Plan::features()` is the catalog of spec §4 |
| `Entitlements` | A plan and an optional expiry. `allows(feature, now)` and `effective_plan(now)`: after the expiry the plan counts as Lite, so a trial ends on its own |
| `Clock` | The port `now() -> OffsetDateTime` (UTC). `SystemClock` implements it. `FixedClock` is behind the feature `testing`, for the tests of modules, which may not depend on `barras-testkit` |

### 4.2 `barras-adapter-http`

Owns axum. Depends on `barras-kernel` and its own technology. Its public surface:

| Part | What it gives |
| --- | --- |
| Envelope | `ApiSuccess<T>`, `ApiError`, `ApiResult<T>`. `ApiError` is built from any `AppError`, so a handler writes `use_case.execute(input).await?` |
| Extractors | `ValidatedJson<T>` and `ValidatedQuery<T>`. Handlers use these, never axum's own `Json` or `Query`, whose rejections are not envelopes |
| Router | `ApiRouter`: the only way to add a route to the public API. It takes only handlers that carry `#[utoipa::path]`, each together with its access declaration (the permission and the plan feature it needs) and its own error codes, as typed values. A route without them does not compile |
| Middleware | Request id, request log, HTTP metrics, compression, body limit, panic to envelope, 404 and 405 envelopes, and a safety net that wraps any response a library layer produced on its own |
| Health | `HealthProbe` (name, critical or not, an async check), the two public routes, the internal detail |
| Internal router | `GET /health` and `GET /metrics`, for the internal port of either binary. The metrics live in a registry the composition root creates and hands over; nothing is global |
| OpenAPI | The document and its canonical JSON form |
| Server | `serve(...)`: serves the routers on the listeners it is given until the shutdown signal, then finishes in-flight requests |
| `testing` (feature) | `TestClient` (in-process), a client for a real socket, and the OpenAPI guard of section 9.3 |

Handlers stay thin (design §6.3). JSON field names are camelCase, as `lastEventId` in the design §6.10 (**Call**).

### 4.3 `barras-bootstrap` (new)

Depends on no workspace crate. Only `apps/*` depend on it.

- `Environment`: a reader over a set of name and value pairs. `required::<T>(name)` parses a value and records every problem instead of stopping at the first. Tests pass their own pairs; nothing in a test touches the process environment (setting it is `unsafe` in edition 2024, and `unsafe` is forbidden).
- Logging: `init_logging(env, level)`. JSON in every environment but `local`.
- `shutdown_signal()`: resolves on SIGTERM or Ctrl-C.

### 4.4 `barras-api` and `barras-ingest`

Each gets a library target, so tests reach the same code as `main.rs`, and `main.rs` only calls it.

- `barras-api`: `ApiConfig`; `Dependencies` (in this ticket: the health probes; BAR-4 adds the pools); `build_app(&ApiConfig, Dependencies)`, the one function that wires the application; `run()`; the `openapi` mode.
- `barras-ingest`: `IngestConfig`; `run()`: logs, serves its internal port, waits for the shutdown signal. The job runner is BAR-6.

### 4.5 `barras-testkit`

Gains `process`: start a workspace binary with a clean environment, wait for a port, send SIGTERM, read its output, kill it on drop; and a free local port. Used by the process tests of both binaries.

### 4.6 `workspace-hack` (cargo-hakari)

`crates/workspace-hack` is generated by `cargo hakari` and depends on the union of the third-party dependencies and their features. Every member depends on it except `xtask`, which is left out of the traversal so the repository tool neither depends on a workspace crate nor shapes what the others build.

- Declared once in `[workspace.dependencies]` and taken with `workspace-hack.workspace = true`, like every other dependency.
- `check-arch` knows the folder (no `unknown-location`), accepts `workspace-hack` as a path dependency in the kernel's and the modules' lists, and still refuses it in `xtask` (`tool-isolation`).
- cargo 1.100 warns about a dependency no code uses, and `just lint` denies warnings. Each managed crate names the dependency once (`use workspace_hack as _;`), so the lint stays on for every real dependency. The generated crate allows the lint for itself.
- cargo-deny lists `workspace-hack` as a wrapper of each banned crate that enters it. A crate still cannot `use` what its own `Cargo.toml` does not name, so the layer rule holds.
- `just hakari` regenerates it after a dependency change. `just hakari-check` fails when it is stale; `just verify` and CI run it.

## 5. Routes

Public port, all under `/api/v1`:

| Route | Success | Error |
| --- | --- | --- |
| `GET /api/v1/health/live` | 200, `data: { status: "ok" }` while the process runs | none of its own |
| `GET /api/v1/health` | 200, `data: { status: "ok" }` when every probe passes; `"degraded"` when only non-critical probes fail | 503 `SERVICE_UNAVAILABLE` when a critical probe fails |

- The public health answers never name a dependency (ADR-0015).
- Each probe runs with a 2 second limit, all at once; a probe that does not answer in time counts as failed (**Call**).
- With no probe registered, which is the case until BAR-4, ready is `ok`.

Internal port, no `/api/v1` prefix, no envelope, not in `openapi.json` (**Call**: it is for operators and Prometheus, not for the app):

| Route | Answer |
| --- | --- |
| `GET /health` | 200 or 503 with `{ status, service, version, dependencies: [{ name, critical, status: "up" or "down", latencyMs, error? }] }` |
| `GET /metrics` | Prometheus text format |

Schema changes: none.

## 6. The HTTP contract in detail

### 6.1 Envelopes

Exactly the shapes of ADR-0005:

```json
{ "success": true, "message": "Service is ready", "data": { "status": "ok" }, "code": 200 }
```

```json
{ "success": false, "message": "Request validation failed", "data": null, "code": 422,
  "error": { "code": "VALIDATION_FAILED", "fields": { "email": ["is not a valid email address"] } } }
```

- Every response the application produces on the public port is an envelope, including an unknown route, a wrong method, a rejected body and a panic. A safety net wraps any response that a library layer produced on its own. One case is out of reach: bytes that are not HTTP at all are answered by the server library with a bare 400 before any of our code runs.
- A request body is a JSON object.
- A route with nothing to return answers 200 with `data: null`. No 204 (**Call**: one shape for the client).
- `error.fields` appears only on a validation failure. Its keys are field paths (`address.zip`, `items[2].amount`). A field message says what is expected and never repeats the value that was sent, which may be a password.
- For kinds `Internal` and `Unavailable` the message is generic and the detail, with the chain of sources, goes to the log at `error` level. `error.code` is still the variant's code.

### 6.2 Status and error codes

`ErrorKind` to status, mapped only in this adapter:

| Kind | Status |
| --- | --- |
| `Validation` | 422 |
| `Unauthenticated` | 401 |
| `Forbidden` | 403 |
| `NotFound` | 404 |
| `Conflict` | 409 |
| `RateLimited` | 429 |
| `Unavailable` | 503 |
| `Internal` | 500 |

Codes of the HTTP adapter itself:

| `error.code` | Status | When |
| --- | --- | --- |
| `VALIDATION_FAILED` | 422 | A body or query field is missing, has the wrong type or breaks a rule. `error.fields` is present |
| `MALFORMED_REQUEST` | 400 | The body is not JSON, or the query string cannot be read |
| `UNSUPPORTED_MEDIA_TYPE` | 415 | A body without `Content-Type: application/json` |
| `PAYLOAD_TOO_LARGE` | 413 | A body over 1 MiB (**Call**; a route may set its own limit later) |
| `NOT_ACCEPTABLE` | 406 | `Accept-Encoding` refuses every encoding the server has, the plain body included |
| `ROUTE_NOT_FOUND` | 404 | No route matches the path |
| `METHOD_NOT_ALLOWED` | 405 | The path exists, the method does not |
| `SERVICE_UNAVAILABLE` | 503 | `GET /api/v1/health` with a critical probe failing |
| `INTERNAL_ERROR` | 500 | A handler panicked |

Codes of `KernelError`:

| Code | Kind | When |
| --- | --- | --- |
| `INVALID_CURRENCY` | `Validation` | Not three upper-case ASCII letters |
| `INVALID_AMOUNT` | `Validation` | Not a decimal, more than 2 decimal places, or more than 18 integer digits |
| `INVALID_ID` | `Validation` | Not a UUID in canonical form |
| `INVALID_PAGE_LIMIT` | `Validation` | Outside 1 to 50 |
| `INVALID_CURSOR` | `Validation` | Empty, over 512 characters, or a character outside the set |
| `CURRENCY_MISMATCH` | `Internal` | Arithmetic on two currencies: a bug in the caller, never the user's fault |
| `AMOUNT_OVERFLOW` | `Internal` | A sum leaves the `numeric(20,2)` range |

All of these are public API from the day they ship (ADR-0006).

### 6.3 Request id

- Header `x-request-id`. An incoming value is kept when it is 1 to 64 characters of `A-Z a-z 0-9 . _ -`; otherwise the server makes a UUID v7.
- It is on every response, error envelopes included, and on every log event of the request.

### 6.4 Compression and limits

- gzip and brotli, chosen by `Accept-Encoding`. Bodies under 1 KiB are sent as they are (**Call**).
- Request bodies are limited to 1 MiB.

### 6.5 Request log

- One `info` event per finished request: method, the matched route template, status, duration, request id. A 5xx adds an `error` event with the cause.
- Never a header, a query string or a body (ADR-0015). A test proves it with an `Authorization` header, a query string and a body that must not appear in the captured output.

### 6.6 Metrics

| Metric | Type | Labels |
| --- | --- | --- |
| `http_requests_total` | counter | `method`, `path` (the matched template, or `unmatched`), `status` |
| `http_request_duration_seconds` | histogram | `method`, `path`; buckets at 5, 10, 25, 50, 80, 100, 150, 200 and 500 ms, 1, 2.5 and 5 s, so the budgets of ADR-0014 fall on bucket edges |
| `barras_build_info` | gauge, always 1 | `service`, `version` |

- The `path` label is the route template, never the raw path, so its values are bounded.
- Requests to the internal port are not counted.
- Library: the `prometheus` crate with an explicit registry (**Call**). The tooling analysis named `metrics` with its exporter; the spike showed it is a process-wide global, costs about 200 ms in every test that builds the app, and prints in an unstable order. A registry passed by hand fits a codebase that wires everything in `main.rs`.

## 7. OpenAPI

### 7.1 Pipeline

- The document is built with utoipa from the same `ApiRouter` values that serve the routes.
- `openapi.json` is committed at the repository root, in a canonical form: pretty-printed, every object's keys sorted, a final newline. Two runs give the same bytes; utoipa's own output does not, which the spike measured.
- The document's `info` is written by hand: the title `Barras API` and the workspace version. Nothing comes from a crate's metadata.
- `just openapi` rewrites it. `just openapi-check` fails when it differs, with the hint to run `just openapi`. `just verify` and the CI `checks` job run the check.

### 7.2 What "OpenAPI metadata" means

Every operation under `/api/v1` has:

1. An `operationId`, one tag and a summary.
2. A success response whose body is the success envelope around a named schema.
3. Every error status it can return, each with the error envelope.
4. `x-error-codes`: the `error.code` values of its own that it can return, sorted. May be empty; must be present. The codes any route can return (`ROUTE_NOT_FOUND`, `METHOD_NOT_ALLOWED`, `MALFORMED_REQUEST`, `UNSUPPORTED_MEDIA_TYPE`, `PAYLOAD_TOO_LARGE`, `NOT_ACCEPTABLE`, `INTERNAL_ERROR`) are listed once, at the top of the document, as `x-common-error-codes`.
5. `x-permission`: the permission it needs, or `public`.
6. `x-plan-feature`: the `Feature` code it needs, or `none`.

Items 4 to 6 are not written by hand in the document: `ApiRouter` writes them from the typed values a route is registered with (section 4.2). In this ticket every route is `public` and `none`. BAR-17 attaches the guard from the same values, so what a route documents and what it enforces cannot differ.

## 8. Configuration and logs

### 8.1 Variables

| Variable | Read by | Value |
| --- | --- | --- |
| `APP_ENV` | both | `local`, `staging` or `production` |
| `LOG_LEVEL` | both | `trace`, `debug`, `info`, `warn` or `error` |
| `API_BIND_ADDR` | `barras-api` | A socket address, such as `127.0.0.1:8080` |
| `API_INTERNAL_BIND_ADDR` | `barras-api` | A socket address, such as `127.0.0.1:9080` |
| `INGEST_INTERNAL_BIND_ADDR` | `barras-ingest` | A socket address, such as `127.0.0.1:9081` |

- Every variable is required. There is no default in code (**Call**: a default is a second place where configuration lives). An empty value counts as missing.
- A binary with a missing or invalid variable prints one line per problem to standard error, all problems at once, and exits with a non-zero status before it opens a socket. A line names the variable and what is expected. It never repeats the value, which may be a secret.
- `.env.example` lists every variable with a value that works on a developer machine. A test loads each binary's configuration from it, so the example cannot go stale.
- The binaries read only the process environment. `just dev` loads `.env` before it starts them.
- `barras-api openapi` reads no variable.

### 8.2 Logs

- `APP_ENV=local`: readable lines. Any other value: one JSON object per line with the timestamp (UTC), level, target, message, and the fields of the current spans.
- `LOG_LEVEL` sets the lowest level written.

### 8.3 Process

- `barras-api` listens on two addresses: the public API and the internal port. `barras-ingest` listens on its internal port.
- On SIGTERM or Ctrl-C a binary stops accepting connections, finishes the requests in flight and exits with status 0.

## 9. Tests

### 9.1 Tiers

| Command | Runs | Needs |
| --- | --- | --- |
| `just test` | Unit tests and the integration tests that need nothing outside the process, then the doctests | No Docker |
| `just test-int` | The test targets whose file name starts with `e2e_` or `int_` | From BAR-4, Docker |
| `just test-cov` | Both, under coverage, then the thresholds | The same as `just test-int` |

- `e2e_*.rs` tests drive a whole binary: its app in-process or its real process. `int_*.rs` tests drive one adapter against a real outside system (from BAR-4). The name is the rule, so nothing has to be registered (**Call**).

### 9.2 Per layer

| Layer | Tests |
| --- | --- |
| Kernel | Every type directly: each constructor's accepted and refused inputs, the money arithmetic, the expiry of entitlements, the stability of every code |
| Bootstrap | The environment reader (missing, empty, invalid, every problem reported, no value repeated); the JSON log line parses and carries the span fields |
| HTTP adapter | In-process against a fixture router built with the same `ApiRouter` and middleware as the app: each `ErrorKind` to its status and envelope; `error.fields` for a wrong type and for a broken rule; malformed JSON; 404, 405, 413, 415 and a panic as envelopes; the request id kept, replaced and present on errors; gzip, brotli and a small body left alone; the metrics text; the log that leaks nothing; the canonical JSON; the guard of section 9.3 |
| `barras-api` e2e | In-process: ready `ok`, `degraded` and 503 (with fake probes), live, an unknown route, a wrong method, the request id; every operation has the metadata of section 7.2; `openapi.json` equals the generated document. Real process: refuses to start with variables missing and names each one; starts from the values of `.env.example`; health and metrics answer on both ports; SIGTERM ends it with status 0; `openapi` prints the document with an empty environment; `openapi --check` fails on a stale file |
| `barras-ingest` e2e | Real process: refuses to start with a variable missing; health and metrics answer on its internal port; SIGTERM ends it with status 0 |
| `xtask` | `check-coverage` through the library and the binary; the new `check-arch` rules |

Errors are asserted by `code`, never by message (ADR-0017).

### 9.3 The guard against a route without OpenAPI metadata

Three layers, so the guard does not depend on someone remembering:

1. `ApiRouter` accepts only documented handlers (section 4.2).
2. One e2e test walks the generated document and fails on any operation that lacks an item of section 7.2, naming the operation and the item.
3. `TestClient` checks every response it receives: when the request was served by a route (method and template) that the document does not have, the test fails with the route's name. So any e2e test that touches an undocumented route fails.

A test of the guard itself mounts an undocumented route in a fixture app and expects the failure.

### 9.4 Coverage

Measured with cargo-llvm-cov over the source of the workspace crates, without `barras-testkit` and the generated `workspace-hack`. `xtask` counts.

- Only production lines count (**Call**). cargo-llvm-cov leaves out files under `tests/` but counts the inline `#[cfg(test)]` modules, which are about half of today's lines and are always covered, so they would lift every figure. `check-coverage` reads the per-line report and leaves those modules out. For `xtask` today that is an estimated 84%, against the 91% the tool reports with the test modules counted.
- At least 95% of lines in `crates/kernel` and in each crate under `crates/modules/`, each measured on its own. The kernel is held to the modules' bar (**Call**: it is pure domain code, and until the first module exists the 95% bar would otherwise check nothing).
- At least 85% of lines overall.
- Code run in a child process counts, so the real-process tests of section 9.2 cover `main.rs`.
- Doctests run, but outside the measure: instrumenting them needs a nightly toolchain.
- `cargo xtask check-coverage <report>` fails under either bar, naming the crate and its figure. `just test-cov` runs it, and `just verify` runs `just test-cov`.

## 10. Commands and CI

| Command | After this ticket |
| --- | --- |
| `just setup` | Also installs cargo-llvm-cov, cargo-hakari and watchexec |
| `just dev`, `just dev-ingest` | Load `.env`, run the binary, and restart it with watchexec when a Rust source or a `Cargo.toml` changes. The restart sends SIGTERM, so it exercises the clean shutdown |
| `just test`, `just test-int`, `just test-cov` | Section 9.1 |
| `just openapi`, `just openapi-check` | Section 7.1 |
| `just hakari`, `just hakari-check` | Regenerate or verify `workspace-hack` |
| `just verify` | `fmt-check`, `lint`, `deny`, `check-arch`, `check-structure`, `hakari-check`, `openapi-check`, `test-cov`, the doctests, `build` |

- CI `checks`: adds `hakari-check` and `openapi-check`; the pull request text step runs last. CI `test`: runs `just test-cov` and the doctests. CI `build`: unchanged.
- The hooks are unchanged: `pre-push` runs `just verify`.
- Budget: `just verify` and CI stay within 10 minutes (ADR-0014). The plan measures it.

## 11. Done when

| Card item | Met by |
| --- | --- |
| A sample route returns both envelopes and is covered by e2e | `GET /api/v1/health`: 200 success envelope, and 503 error envelope with a failing critical probe, in `apps/api/tests/e2e_*.rs`. The envelopes for validation and for each `ErrorKind` are covered in the adapter's tests against a fixture router, because no real route takes a body yet |
| The e2e suite fails on a route without OpenAPI metadata | Section 9.3, with the test of the guard itself |
| Health and metrics answer on their ports | The real-process tests of both binaries |
| `just verify` fails under the coverage thresholds | `just verify` runs `just test-cov`; `check-coverage` is tested with reports under each bar |

Plus the definition of done of `CLAUDE.md`.

## 12. Decisions to record

Approving this spec accepts these:

| ADR | Content |
| --- | --- |
| ADR-0024, amended, then `Accepted` | The `workspace-hack` exception; `barras-kernel` only at the member's path; the bans on `axum-core`, `hyper`, `hyper-util` and on alternative crates |
| ADR-0026 (new) | The `barras-bootstrap` crate and its row in the dependency table |
| ADR-0027 (new) | Test tiers by file name; coverage measured per crate; the kernel at 95%; what is left out of the measure |
| ADR-0028 (new) | The OpenAPI pipeline: the `barras-api openapi` mode, the canonical file, the metadata keys of section 7.2, `ApiRouter` as the only door |
| ADR-0029 (new) | Details of the HTTP contract that ADR-0005 leaves open: the adapter's own codes and statuses, camelCase, no 204, the request id header, the internal port without envelope |

ADR-0003, ADR-0015, ADR-0016 and ADR-0017 keep their Decisions; where a Consequence is no longer accurate it gets a pointer to the ADR that refines it.

## 13. Left to the plan

- The exact versions and features, starting from those a spike compiled together on 2026-11-15: axum 0.8.9, tokio 1.54.1, tower 0.5.3, tower-http 0.7.1, utoipa 6.0.0, utoipa-axum 0.3.0, tracing-subscriber 0.3.23, garde 0.23.0 (validation), prometheus 0.14.0, serde_path_to_error 0.1.20, hyper-util 0.1.21; cargo-llvm-cov 0.9.1, cargo-hakari 0.9.39, watchexec 2.8.0.
- The licences and wrappers `deny.toml` needs, read from the real dependency tree.
- The order of the middleware layers, which the spike worked out and the plan quotes.

## 14. Risks

1. utoipa 6 and utoipa-axum 0.3 are recent releases. A spike compiled and ran every part this spec relies on; what it found is already in sections 6 and 7. The typed route declaration of section 4.2 was not part of that spike, so the plan proves it first.
2. cargo-hakari makes every crate, the kernel included, wait for the union of third-party dependencies on a cold build (section 3.7). The 5 second budget is for an incremental check and is not affected; the plan measures both once the real dependencies are in.
3. `just verify` now builds the workspace three ways (checks, coverage, release). If it passes 10 minutes, the plan says what moves out of `verify`, and the owner decides.
4. A real-process test picks a free port and then starts the binary, so another process can take the port in between. The helper retries with a new port.
5. An `Accept-Encoding` that refuses everything is answered 406 after the handler has run, so a write would already have happened. It is a quirk of the compression layer; no client of ours sends such a header, and the plan notes it where the layer is configured.
