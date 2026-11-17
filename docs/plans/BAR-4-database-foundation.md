# BAR-4: Database foundation, implementation plan

Date: 2026-11-16. Spec: [`docs/specs/BAR-4-database-foundation.md`](../specs/BAR-4-database-foundation.md), approved by the owner on 2026-11-16. Branch: `feat/BAR-4-database-foundation`.

The spec says what and why; this plan says in which order, with which tests, and what each task leaves behind. Where the plan and the spec disagree, the spec wins until a refinement below says otherwise.

## How the work runs

- Test first: each step writes the failing test, sees it fail for the right reason, then writes the code.
- Each task is built by a subagent and reviewed by `agent-skills:code-reviewer` before the next task builds on it. A task that touches CI, the hooks, the justfile's gates, `deny.toml` or the local-database guard is also reviewed by `agent-skills:security-auditor`. The findings are fixed, or answered with a reason, before the task counts as done. One whole-branch review closes the plan (Task 8).
- Tasks 1 and 2 need no database and run in parallel with the spikes of Task 0. Task 3 starts once Task 0 has answered.
- A subagent in a worktree of its own cannot commit: the hooks need the toolchain on PATH, and the worktree's guard refuses a git command that sets it. It leaves its work staged, and for a task of several commits it marks each one with `git write-tree` and saves the diff between two marks as a patch. The session applies the patches in order on the branch and commits each one through the hooks.
- Docker Desktop runs for Task 0 and from Task 3 on.
- Commits read `type(BAR-4): lower-case description`, go through the hooks, and carry no attribution (ADR-0018). Nothing is pushed until the owner says so.

## Refinements of the spec

A decision made during the work that changes what the spec says is numbered here, with where it is recorded.

From the spikes (appendix A, 2026-11-16):

1. **Spec §7, an edited migration.** sqlx checks an applied migration's checksum only when its turn comes, after applying every older pending one (A.3). So `barras-api migrate` first compares every applied migration in `_sqlx_migrations` with the binary's, and refuses before it applies anything: the spec's promise holds. It runs on a connection of its own, closed after the run whatever happened, because sqlx's advisory lock outlives a refusal on a pooled connection.
2. **Spec §8, a `.env` that cannot be read.** No build connects to a database (A.1), but the sqlx macros parse every `.env` from the crate's folder up to `/`, and one that dotenvy cannot read fails every build, offline too. `CONTRIBUTING.md` says so (Task 7); the value rule of §5 already keeps `.env.example` readable.
3. **Spec §6.2, statements logged.** sqlx's event for each statement has no `message`, which every log line must have (A.6), so it is turned off (`log_statements(Off)`). The slow-statement event has one and stays, at `warn` from 250 ms, set explicitly: sqlx's default is 1 second.
4. **Spec §5 and §9.3, what a URL names.** sqlx's getters fill a missing part from the `PG*` variables or `~/.pgpass` and show no password (A.5), so the rule reads the URL itself with the `url` crate: scheme `postgres` or `postgresql`; host, port, user, password and database all present; none of the query keys `host`, `hostaddr`, `port`, `user`, `password`, `dbname`; and no IPv6 address in brackets, which sqlx 0.9 cannot connect to (`localhost` reaches `::1`). The guard reads `get_socket()` before `get_host()`.
5. **Spec §9.1, a seed's failure.** It is built from the file's name, the SQLSTATE, the server's message and the line and column of its position, never from sqlx's `Display`, whose "at line N" is a line of Postgres's C source (A.4). A seed that ends the transaction itself (`COMMIT`) would escape the one-transaction rule, so the runner checks after each file that its transaction is still open, and fails naming the file otherwise.
6. **Spec §6.4, the rule for repositories.** A database error's detail can quote a value bound to a statement (A.4, A.6: `Key (id)=(1) already exists.`, `invalid input syntax for type integer: "…"`). A repository's error never carries sqlx's text into an answer or a log line; BAR-15 applies it with the mapping of §6.4.

Decided by the owner on 2026-11-16, after the spikes and the review of Task 2:

7. **Spec §6.4, a request whose body is slow.** A request whose body has not finished arriving when the 5 seconds pass is answered 408, with the envelope, the code `REQUEST_BODY_TIMEOUT` and `connection: close`. It is the client's slowness, not the server's: no `request failed` event, counted as a 4xx. A request whose body arrived, or that has none, still gets 503 `REQUEST_TIMEOUT`. The timeout layer watches the request body, and the 408 goes only to a request whose handler was waiting for body data that had not come. A handler that never read the body, or stopped reading it, is a slow server and gets the 503: a first version answered 408 to any body not read to its end, and the review of Task 2 showed a slow server answered as a slow client that way. `REQUEST_BODY_TIMEOUT` is the ninth common code.
8. **The refinements 1 to 6 above** were put to the owner, who said to go on with them.

From the review and the security audit of Task 3 (2026-11-16):

9. **Spec §7, sqlx-cli and the history table.** sqlx-cli names the history table without a schema, and with a user and a schema both called `barras` the default `search_path` makes its `migrate info` read and create `barras._sqlx_migrations`. So a `sqlx.toml` at the repository root holds `[migrate] table-name = "public._sqlx_migrations"`, the name the binary's migrator uses, and sqlx-cli is installed with its `sqlx-toml` feature. The adapter's folder has no `sqlx.toml`, so the macros need no feature. A test fails when the two names differ.
10. **Spec §5 and refinement 4, query parameters.** A URL carries no query parameter. sqlx logs the value of a key it does not know at `warn`, and `options` or `application_name` would contest the binary's own settings; BAR-12 allows what TLS needs. A host holding `,` is refused too: sqlx reads several hosts as one name and never connects.
11. **Spec §4.1, a failing migration's text.** It shows the server's message only for the SQLSTATE classes 42 (syntax or access rule) and 0A (not supported); for any other it gives the SQLSTATE and the file, since the message can quote the data of a table and migrations run against production. A failing seed keeps the server's message: seeds run only against local demo data.
12. **Spec §8, a build started outside the checkout.** cargo reads `.cargo/config.toml` from the folder it runs in, so a build started elsewhere would miss `SQLX_OFFLINE`. The adapter's build script sets it for the adapter's own compilation when it is unset; the explicit `false` of `cargo sqlx prepare` still wins.
13. **Spec §4.6 and ADR-0024, what `just build` builds.** `cargo build --workspace` unifies the features of every member it builds, so the testkit's testcontainers turned on hyper-util's `http2` and `server-auto` in `barras-api`, whose public port then answered cleartext HTTP/2. `just build` leaves out `barras-testkit` and `xtask`, and `hakari-check` compares the release graph of that same selection.
14. **Spec §3.4, the ignored advisory.** RUSTSEC-2026-0440 comes with testcontainers, which this branch adds, so its ignore cannot land in an earlier pull request of its own; it is put to the owner. It is scoped: `astral-tokio-tar` is banned with testcontainers as its only wrapper, and `unused-ignored-advisory = "deny"` fails the check once the ignore matches nothing.

Choices the spec left to the plan, made from the spikes: testcontainers 0.28.0 with a Postgres image of our own (A.7); the image `postgres:18.7-trixie@sha256:2ea8630df176ad090ec92136dee850397c5af8781e4bcf2e4162593691c61bbc`, written with tag and digest in `compose.yaml`, the testkit and CI (A.9); CI pulls it from `mirror.gcr.io` and tags it, with Docker Hub as the fallback, to be confirmed by the first CI run (A.9); sqlx-cli installed with `--locked` (A.8).

## Task 0: Spikes (gate)

In a scratch workspace outside the repository, with the exact versions of the spec. Each spike writes what it found in appendix A: the answer, the version, and the command or code that showed it. A spike that contradicts the spec stops the plan, and the owner hears it before anything is built on it.

- **S1, offline build.** A crate with one `query_scalar!` and `.cargo/config.toml` holding `[env] SQLX_OFFLINE = "true"`, next to a `.env` that names an unreachable `DATABASE_URL`: it builds from `.sqlx/` without trying to connect; without `.sqlx/` it fails naming the missing query. A malformed `.env` up the tree: does it fail the build even offline?
- **S2, prepare and check.** Against a throwaway Postgres 18 container, with the config of S1 present: `cargo sqlx prepare --workspace` writes `.sqlx/` (the config's `SQLX_OFFLINE=true` does not override the `false` sqlx-cli sets); `--check` passes; then fails on a changed query, and on a migration that changes the type of a column a query reads; an extra file only warns, so write the comparison `db-check` adds (file names of `.sqlx/` against `target/sqlx-prepare-check`).
- **S3, embedded migrations.** `sqlx::migrate!` with a relative path that reaches the repository's `migrations/` from `crates/adapters/postgres`, and a `build.rs` with `cargo::rerun-if-changed`: a new migration file rebuilds the crate. `Migrator::run` on an edited migration and on a missing one: the error variants and their text.
- **S4, seed files.** `raw_sql(AssertSqlSafe(text))` with several statements inside one transaction; a failing statement's error: SQLSTATE, message, position; the transaction rolled back.
- **S5, what a URL means to sqlx.** `PgConnectOptions` from URLs with `host`, `hostaddr`, a socket folder as host, IPv6, and with parts missing, run in a child process whose environment sets `PGHOST`, `PGPORT`, `PGUSER`, `PGPASSWORD`, `PGDATABASE` and `PGHOSTADDR`: which getters show the target sqlx connects to, and how to tell that a URL left a part out (the spec makes that invalid).
- **S6, statement logs.** What sqlx logs at `debug` and for a slow statement at `warn`, through `tracing`: the fields, and that no bound value appears.
- **S7, containers.** testcontainers 0.28.0 with a Postgres image definition of our own, against 0.27 with testcontainers-modules 0.15: pick one. Image `postgres:18.7-trixie` by digest, `fsync=off`, a label, the `watchdog` feature: the container is removed on a normal end, on a panic, and on SIGTERM. Start time for one container, and for eight at once.
- **S8, sqlx-cli.** `cargo install sqlx-cli@0.9.0 --no-default-features --features postgres`, with and without `--locked`: which works, and how long a cold build takes.
- **S9, the registry.** The digest of `postgres:18.7-trixie` on Docker Hub and on a mirror without Docker Hub's anonymous limits (`mirror.gcr.io`, `public.ecr.aws/docker/library`): CI pulls the same image by digest from a mirror, or the plan says why not.

## Task 1: `Secret<T>` and the pool size in `barras-bootstrap`

- Files: `crates/bootstrap/src/secret.rs` (new), `crates/bootstrap/src/environment.rs`, `crates/bootstrap/src/lib.rs`.
- `Secret<T>`: `Debug` and `Display` print `[redacted]`, inside a derived `Debug` too; `expose(&self) -> &T`; `Clone`, `PartialEq`, `Eq` when `T` has them; no `Serialize`; `FromEnvValue for Secret<T>` wherever `T: FromEnvValue`, and an invalid value is reported without its text, as every variable is.
- `FromEnvValue for NonZeroU32`: digits only, no sign, no blanks; `0` is invalid; a number above `u32::MAX` is invalid.
- `FromEnvValue for String`, for a value with no structure of its own: any value; an empty one already counts as missing (BAR-3 §8.1).
- Tests: unit tests for each rule above, with a secret value that must never appear in any output.
- Commit: `feat(BAR-4): add a secret type and whole numbers to the configuration`.

## Task 2: The request timeout and the shared readiness run in `barras-adapter-http`

- Files: `crates/adapters/http/src/middleware/` (the timeout layer), `src/error.rs` (`REQUEST_TIMEOUT`), `src/openapi.rs` (`COMMON_ERROR_CODES`), `src/testing/test_client.rs` (503 among the common statuses), `src/health/` (the shared run), `src/tests/`, `openapi.json`.
- Timeout (spec §6.4): 5 seconds, a named constant; 503, `REQUEST_TIMEOUT`, the envelope; the layer inside the safety net and outside compression, so inside the request log, `PropagateRequestId` and the metrics. The implementer picks between tower-http's `TimeoutLayer` and an `axum::middleware::from_fn` with `tokio::time::timeout`, and says why in a comment.
- Tests, with tokio's paused clock: a handler slower than 5 s answers the 503 envelope with the request id, is logged once as `request failed` and counted with status 503; a body that the client sends slowly counts against the limit; a body that streams out after the head is not cut; a handler under the limit is untouched.
- Readiness (spec §6.3): concurrent requests share one run; a result younger than one second is reused; the public and the internal routes share it. Tests: two concurrent requests run each check once; a request within one second reuses the result; one after a second runs the checks again; a check that panics or times out is still reported as before.
- `just openapi`: the document gains the eighth common code; `e2e_openapi.rs` passes.
- Commits: `feat(BAR-4): answer a request that takes longer than five seconds`, `feat(BAR-4): share one readiness run between concurrent requests`.

## Task 3: The postgres adapter, the first migration, the offline metadata and the containers

Built on appendix A.

- **Workspace:** sqlx in `[workspace.dependencies]` with `default-features = false` and the features of S1 to S6; testcontainers (S7) for the testkit; `SQLX_OFFLINE = "true"` in `.cargo/config.toml`, with the comment of spec §8. `just hakari`; the wrappers and licences `cargo deny` asks for, each with its reason (spec §4.6).
- **`compose.yaml`:** spec §10, with the image of S7 and S9.
- **`migrations/`:** the first migration, `<timestamp>_create_schema_barras.sql` (spec §7).
- **`barras-adapter-postgres`:** `build.rs` (S3); the embedded migrations and `migrate`; `Pools` with the settings of spec §6.1 and one pool when the URLs are equal; `check_primary` and `check_read` with `query_scalar!("SELECT 1 …")`; the database URL rule of spec §5 (S5); the fixed sentences of spec §4.1; no `Debug` on what holds the connect options.
- **`barras-testkit`:** a Postgres container per test (spec §11.2): the image read from `compose.yaml`, `fsync=off`, a label, `watchdog`, the URL of a database ready to use; a clear failure when Docker is not running. The crate doc names containers, fixtures and the process helper and says the crate is test-only (the BAR-2 note).
- **`justfile`:** `_throwaway-db` (start, wait, run a command with `DATABASE_URL` set, remove), `db-prepare`, `db-check` (with the file comparison of S2), sqlx-cli in `setup` (S8). `.sqlx/` written by `just db-prepare` and committed.
- **Tests:** `int_` targets in the adapter: a pool opens nothing until used; equal URLs make one pool; `application_name` shows in `pg_stat_activity`; `statement_timeout` cancels `pg_sleep(6)` with SQLSTATE 57014; a check passes against the container and fails with its fixed sentence against a closed port, within the limits; `migrate` applies the pending migration, then has nothing to apply, refuses an edited migration and an unknown applied one. Unit tests: the URL rule and the sentences, with a password that never appears.
- Commits: one per part, such as `build(BAR-4): add sqlx and the offline metadata`, `feat(BAR-4): add the database pools and their readiness checks`, `feat(BAR-4): embed the migrations and apply them`, `test(BAR-4): start a postgres container per test`.

## Task 4: Seeds, reset and the local-database guard

- Files: `crates/adapters/postgres/src/{guard,seed,reset}.rs` and their tests.
- Guard (spec §9.3): allowed: a Unix socket, `127.0.0.0/8`, `::1`, `localhost` resolving only to loopback addresses; refused: any other host, a host whose resolution includes a non-loopback address, a URL whose target S5 shows the guard cannot judge. Each refusal is a reason with a fixed message. Unit tests over a table of URLs; one test with a name that does not resolve.
- Seeds (spec §9.1): every `.sql` file of the folder in name order, other files ignored, a missing folder empty; one transaction; a failure names the file and the SQLSTATE and keeps the server's message. `int_` tests: order, a failing second file leaves nothing of the first, a second run changes nothing, an empty or missing folder.
- Reset (spec §9.2): guard, drop with the other sessions ended, create, migrate, seed. `int_` test: a database with an extra table and a session open comes back with only the migrations and the seeds.
- Commits: `feat(BAR-4): refuse a database that is not on this machine`, `feat(BAR-4): load the seed files in one transaction`, `feat(BAR-4): rebuild the local database`.

## Task 5: `barras-api`: the pools, the probes and the modes

- **Configuration** (spec §5): `API_DATABASE_URL`, `API_DATABASE_READ_URL` (a `Secret`, checked by the adapter's URL rule), `API_DATABASE_POOL_SIZE`; `ApiConfig::VARIABLES`; `.env.example`.
- **Serving:** the pools built inside the runtime; the probes `postgres` and `postgres-read`, one when the URLs are equal; the pools closed after the server stops.
- **Modes** (spec §3.2, §4.2): `migrate`, `seed <folder>`, `reset <folder>`; each reads only its variables; `seed` and `reset` check `APP_ENV=local`, then the guard, before any connection; output lines on standard output; failures through `report_failure`, never with a credential.
- **Tests:** unit tests for the configuration and the command line. `e2e_` process tests, each with its container: serving with the database up, readiness `ok` and the internal `/health` listing `postgres`; with the database unreachable, the public 503 and the internal fixed sentence, and no password anywhere in the output; `migrate` (applies, then nothing); `seed` with fixture files; `reset`; the refusals for `APP_ENV=staging` and for a host on another machine, fast and without a connection. The in-process e2e tests keep their fake probes; the `.env.example` check covers the new variables.
- Commits: `feat(BAR-4): read the database settings`, `feat(BAR-4): report the database in readiness`, `feat(BAR-4): add the migrate, seed and reset modes`.

## Task 6: The commands, the checks and CI

- **`justfile`:** `db-up`, `db-down`, `db-migrate <name>` (a name that is not snake_case is refused before sqlx-cli runs), `db-deploy`, `db-status`, `db-seed`, `db-reset`, reading `.env` as `dev` does; `verify` with `db-check` after `openapi-check`.
- **`check-structure`:** two migrations with the same version fail; a file in `seeds/` must be named `YYYYMMDDHHMMSS_snake_case.sql`. Tests in `crates/xtask`.
- **CI:** `checks` job: build sqlx-cli at its pinned version, then `just db-check` with a timeout; `test` job: pull the Postgres image in its own step (S9); the tree check unchanged.
- **`deny.toml`:** the advisories comment of spec §3.4.
- Commits: `build(BAR-4): add the database commands`, `feat(BAR-4): check migration versions and seed names`, `ci(BAR-4): check the offline metadata and pull the postgres image`.

## Task 7: Docs

- ADR-0030 and ADR-0031 (spec §15), `Accepted`, with their rows in the index; the pointers in ADR-0007, ADR-0016, ADR-0024, ADR-0027 and ADR-0029, and in ADR-0028, whose seven common codes and status rule changed (refinement 7; the review of Task 2: a common code is exempt only at its own status).
- `CLAUDE.md`'s rules of the API: the nine common codes in the table (`REQUEST_TIMEOUT`, `REQUEST_BODY_TIMEOUT`), and `TestClient`'s status rule as it now is.
- `CLAUDE.md`: the `db-*` commands, the variables, the data access rules (schema-qualified SQL, forward-only migrations, `.sqlx/` and `just db-prepare`, seeds), the test tiers with Docker, the layout (`migrations/`, `.sqlx/`, `seeds/` and `compose.yaml` now exist).
- `README.md`: Docker required, sqlx-cli, the local routine (`db-up`, `db-reset`, `dev`); an existing checkout re-runs `just setup`.
- `CONTRIBUTING.md`: the database workflow; what to do when a new advisory blocks; a `.env` that sqlx can parse.
- `docs/standards/testing.md`: the `int_` tier with containers. `docs/schema/barras-schema.md`: seeds are local data; reference data comes in migrations or code.
- Commit: `docs(BAR-4): record the database foundation`.

## Task 8: Whole-branch verification and review (gate)

- `just verify` from a clean clone, cold and warm; the pre-push hook; `just test-int`; the cost of one container per test (the BAR-2 note).
- The card's "Done when": `just db-up` and `just db-reset` against `compose.yaml` on the owner's machine; on a scratch branch, a query changed without `just db-prepare`, and a migration that changes a column a query reads, each failing `just db-check`.
- A whole-branch `agent-skills:code-reviewer` and `agent-skills:security-auditor`.
- The card to EN PRUEBA; stop for the owner's decision to push.

**Measurements** (Apple M4; budgets from ADR-0014):

| Measure | Result | Budget |
| --- | --- | --- |
| `just verify` from a clean clone, cold | to measure | 10 min |
| `just verify`, warm | to measure | 10 min |
| pre-push hook | to measure | 10 min |
| `just test`, warm | to measure | 30 s |
| `just test-int`, warm | to measure | none |
| one Postgres container, start to ready | to measure | none |
| CI | read it from the first run of the pull request | 10 min |

## Appendix A: what the spikes found

Run on 2026-11-16 in a scratch workspace outside the repository, on an Apple M4 (10 cores) with Docker Desktop (engine 29.9.2, containerd image store). Toolchain: Rust 1.100.0, cargo 1.100.0, edition 2024 (the repository's `rust-toolchain.toml`). Crates: sqlx 0.9.0 (sqlx-core, sqlx-macros-core, sqlx-postgres 0.9.0; `rust-version` 1.94.0; dotenvy 0.15.7; url 2.5.8), sqlx-cli 0.9.0, testcontainers 0.28.0 (bollard 0.21.1) and 0.27.3 with testcontainers-modules 0.15.0 (bollard 0.20.2), tokio 1.54.1, tracing-subscriber 0.3.23, cargo-nextest 0.9.152. Postgres: `postgres:18.7-trixie` by digest, which reports `PostgreSQL 18.7 (Debian 18.7-1.pgdg13+2)`.

sqlx's features everywhere: `default-features = false`, `postgres`, `runtime-tokio`, `macros`, `migrate`. `<scratch>` is the scratch workspace; its `scripts/` hold every script below and `logs/` the full output.

Contradictions with the spec are marked **Contradicts**.

### A.1 S1, offline build

**Answer.** With `[env] SQLX_OFFLINE = "true"` in `.cargo/config.toml`, the crate builds from `.sqlx/` and never connects, whatever `DATABASE_URL` the `.env` names. Without `.sqlx/` it fails at the macro's call site: rustc gives the file, line and column and prints that line of code; the message itself names no query. A `.env` that dotenvy cannot parse fails the build even offline, wherever it sits from the crate's folder up to `/`, above the repository included.

**Evidence.** `.env` held `DATABASE_URL=postgres://spike:spike@127.0.0.1:<port>/spike`, where `<port>` was a listener that logs every connection. Each case touched `src/lib.rs` (one `sqlx::query_scalar!("SELECT 1")`) and ran `cargo build -p spike-postgres --lib` (`scripts/s1.sh`):

| Case | Result | Connections to the listener |
| --- | --- | --- |
| config + `.sqlx/` | `Finished`, exit 0 | 0 |
| config, no `.sqlx/` | exit 101: ``error: `SQLX_OFFLINE=true` but there is no cached data for this query, run `cargo sqlx prepare` to update the query cache or unset `SQLX_OFFLINE` --> crates/adapters/postgres/src/lib.rs:9:5`` | 0 |
| control: `.sqlx/` present, config removed | exit 101: `error communicating with database: Connection reset by peer` | 1 |
| config + `.sqlx/`, one bad line added to the workspace root's `.env` (two levels above the crate): `NOT A VALID LINE`, `KEY="unterminated`, `PASSWORD=two words` | each exit 101: `error: error reading dotenv file "<scratch>/spikes/.env"` | 0 |
| the same with the bad `.env` in the workspace's parent folder | exit 101: `error reading dotenv file "<scratch>/.env"` | 0 |
| the same with the bad `.env` in the crate's own folder | exit 101 | 0 |

The message names the file but not the bad line. The control shows that the config line is what keeps a build offline: without it, the macros connect to the `.env`'s database even when `.sqlx/` is complete.

Read in the source (`sqlx-macros-core-0.9.0/src/query/metadata.rs`, `load_env`): the macros parse every `.env` in `CARGO_MANIFEST_DIR`'s ancestors, up to `/`, before they look at `SQLX_OFFLINE`; a process variable wins over any `.env`; between two `.env` files that both set `DATABASE_URL`, the one higher up the tree wins. `.sqlx/` is looked up in `SQLX_OFFLINE_DIR`, then the crate's folder, then the workspace root.

**Contradicts §8 in part** ("So no build connects to a database, whatever `DATABASE_URL` or a `.env` file holds"): no build connects, true; but a `.env` that dotenvy cannot read breaks every build, offline included, and that `.env` can be outside the repository (a parent folder, the home folder). §16 already says the macros parse every `.env` up the tree; the CONTRIBUTING line of Task 7 should say "any `.env` above the checkout" too. `PASSWORD=two words` (an unquoted blank) is one such line, which the value rule of §5 already forbids.

### A.2 S2, prepare and check

**Answer.** With the config of S1 present, `cargo sqlx prepare --workspace` writes `.sqlx/`: cargo's `[env]` does not replace a variable the process already has, so sqlx-cli's `SQLX_OFFLINE=false` wins. Only `force = true` in the config would break it. `--check` passes when nothing changed, fails on a changed query, and fails on a migration that changes the type of a column a query reads. A file in `.sqlx/` that no query produces only warns, with exit 0; the comparison below makes it fail.

**Evidence** (`scripts/s2.sh`, `scripts/s2d2.sh`; `DATABASE_URL` set to a throwaway Postgres 18.7 container, migrations applied with `sqlx migrate run`):

```
$ cargo sqlx prepare --workspace          # config: SQLX_OFFLINE = "true"
query data written to .sqlx in the workspace root; please check this into version control
.sqlx/query-e004ebd5b5532a4b85984a62f8ad48a81aa3460c1ca07701f386135d72cdecf5.json   ("SELECT 1", Int4)

$ cargo sqlx prepare --workspace --check   # nothing changed                       -> exit 0
# control, config SQLX_OFFLINE = { value = "true", force = true }:
$ cargo sqlx prepare --workspace           -> `SQLX_OFFLINE=true` but there is no cached data ... ; exit 1
# "SELECT 1" changed to "SELECT 2" without prepare:
$ cargo sqlx prepare --workspace --check   -> error: prepare check failed: .sqlx is missing one or more queries; you should re-run sqlx prepare ; exit 1
# migration ALTER TABLE barras.probe ALTER COLUMN amount TYPE int8, read by query_scalar!("SELECT amount FROM barras.probe WHERE id = $1"):
#   a) the code expects i32:   -> error[E0308]: expected `Result<i32, Error>`, found `Result<i64, Error>` ; exit 1
#   b) the code compiles with either type (.map(i64::from)):
$ cargo sqlx prepare --workspace --check   -> error: prepare check failed: one or more query files differ (query-33977863….json); you should re-run sqlx prepare ; exit 1
#      what differs: "type_info": "Int4"  ->  "type_info": "Int8"
# an extra file .sqlx/query-0000….json:
$ cargo sqlx prepare --workspace --check   -> warning: potentially unused queries found in .sqlx; you may want to re-run sqlx prepare ; exit 0
```

The comparison `db-check` adds, run after `--check` has passed (sqlx-cli 0.9.0 writes the regenerated files to `<target dir>/sqlx-prepare-check` and leaves them there; it empties that folder at the start of each run):

```bash
# bash: process substitution. Use `cargo metadata`'s target_directory if CARGO_TARGET_DIR may be set.
extra=$(comm -23 <(cd .sqlx && ls query-*.json | sort) <(cd target/sqlx-prepare-check && ls query-*.json | sort))
if [ -n "$extra" ]; then echo "in .sqlx/ but produced by no query: $extra"; exit 1; fi
```

Output: with the extra file, `in .sqlx/ but produced by no query: query-0000….json`, exit 1; after removing it, exit 0. It also flags the old file of a changed query (case "SELECT 2").

### A.3 S3, embedded migrations

**Answer.** `sqlx::migrate!("../../../migrations")` in `crates/adapters/postgres` embeds the root's `migrations/`. On stable Rust the macro cannot ask cargo to watch the folder (`proc_macro::tracked::path` only under `sqlx_macros_unstable`), so `build.rs` with `cargo::rerun-if-changed=../../../migrations` is needed: a new or edited file rebuilds the crate; without it the binary keeps the old list. `Migrator::run` refuses an edited migration with `MigrateError::VersionMismatch` and a missing one with `MigrateError::VersionMissing`.

**Evidence** (`scripts/s3.sh`):

```
add 20261116000010_s3_first.sql      -> build: spike-postgres compiled again
edit it                              -> build: spike-postgres compiled again
without build.rs, add 20261116000020 -> build: nothing compiled (fresh); the binary still embeds up to 20261116000010
```

| Case | `run` | Display |
| --- | --- | --- |
| 20261116000020 edited after it ran | `Err(VersionMismatch(20261116000020))` | `migration 20261116000020 was previously applied but has been modified` |
| 20261116000010 applied, missing from the binary | `Err(VersionMissing(20261116000010))` | `migration 20261116000010 was previously applied but is missing in the resolved migrations` |
| 20 edited, new pending 30 | `Err(VersionMismatch(20…20))`; 30 not applied | |
| 20 edited, new pending 15 (older than the newest applied) | `Err(VersionMismatch(20…20))`, **after applying 15** (`history: 20261116000015 s3 between success=true`, table `s3_between` created) | |

Read in the source (`sqlx-core-0.9.0/src/migrate/migrator.rs`, `run_direct`): the missing check runs before anything is applied; the checksum check runs inside the loop that applies, in version order; nothing refuses a pending migration older than the newest applied.

After every refusal, sqlx's advisory lock was still held: `pg_locks` showed it granted to the pool's idle connection (`[(168, "3516080584")]`), because `run_direct` returns before it unlocks and the connection goes back to the pool. It ends when that connection closes. `barras-api migrate` exits, so it is safe there; a test that refuses and then migrates again through another connection would wait. Running `migrate` on a dedicated `PgConnection`, closed afterwards, avoids it.

**Contradicts §7** ("It fails, before applying anything, when a migration already applied differs from its file … or is missing from the binary"): true for a missing one; for an edited one, every pending migration with a lower version is applied first. Such a migration is out of order (older than the newest applied), which sqlx accepts silently. Either the spec drops "before applying anything" for edited ones, or `migrate` checks the checksums against `_sqlx_migrations` itself (or refuses a pending version older than the newest applied) before calling `run`.

### A.4 S4, seed files

**Answer.** `sqlx::raw_sql(AssertSqlSafe(text))` runs a whole file of several statements through the simple query protocol, inside an open transaction; several files share it. A failing statement returns a database error with the SQLSTATE (`code()`), the server's message (`message()`) and, for parse and name errors, `PgDatabaseError::position()`: `Original(n)`, a 1-based character offset into the file's text, from which a line and column follow. `rollback` removes everything, earlier files included. Two traps: the `Display` of the error ends with `at line N`, and N is a line of Postgres's C source; and a seed file that holds `COMMIT` ends the transaction early.

**Evidence** (`scripts/s4.sh`, each case on a fresh database with schema `barras`):

```
two files, the second fails while it executes:
  20261116000001_ok.sql: ok, rows_affected=2
  20261116000002_fail_undefined_table.sql: Err
    Display: error returned from database: relation "barras.does_not_exist" does not exist at line 1477
    code(): Some("42P01")   message(): "relation \"barras.does_not_exist\" does not exist"
    server source file/line: Some("parse_relation.c")/Some(1477)
    position(): Original(127) = line 3, column 13 of the file
  rolled back; tables in barras afterwards: []        <- the first file's table is gone too
unique violation:   code 23505, message "duplicate key value violates unique constraint \"seed_c_pkey\"",
                    detail "Key (id)=(1) already exists.", position None
syntax error in the third statement:  code 42601, message "syntax error at or near \";\"", position line 3, column 36
a file that holds COMMIT, then a failing statement:  code 42P01; after rollback the table seed_leak is still there;
                    server log: WARNING:  there is no transaction in progress
```

For Task 4: build the failure from the file name, `code()`, `message()` and the line and column of `position()`, never from `Display`. `detail()` can carry values from the file (`Key (id)=(1)`), which is fine for local seeds but worth remembering for repositories. A seed must not hold `BEGIN`, `COMMIT` or `ROLLBACK`; `check-structure` or the runner can refuse them if the plan wants the one-transaction rule enforced.

### A.5 S5, what a URL means to sqlx

**Answer.** The target sqlx connects to is shown by three getters together, in this order: `get_socket()` set means the Unix socket `<socket>/.s.PGSQL.<get_port()>`; otherwise a `get_host()` starting with `/` means a socket in that folder; otherwise TCP to `get_host()`:`get_port()`. `get_host()` alone can mislead. No getter shows the password, and no getter can tell whether a part came from the URL: `PGHOST`, `PGHOSTADDR`, `PGPORT`, `PGUSER`, `PGPASSWORD`, `PGDATABASE`, `~/.pgpass` and defaults fill whatever the URL leaves out, at parse time. So the rule of §5 has to read the URL itself, with the `url` crate sqlx uses.

**Evidence.** `s5-url` listens on TCP 127.0.0.1, TCP [::1] and a Unix socket, on two free ports P and Q, and runs itself as a child per URL with a cleared environment: none; all six `PG*` (`PGHOST=<socket folder>`, `PGPORT=Q`, `PGUSER=env_user`, `PGPASSWORD=env_password`, `PGDATABASE=env_db`, `PGHOSTADDR=::1`); the same without `PGHOSTADDR`; and only a `~/.pgpass` (`HOME` is a scratch folder, so no real `.pgpass` is read). The child prints the getters, then connects; the listeners show where. It only connected to its own listeners.

| URL | Getters (no `PG*`) | Connected to |
| --- | --- | --- |
| `postgres://u:p@127.0.0.1:P/db` | host `127.0.0.1`, port P, socket None | TCP 127.0.0.1:P; same in every environment |
| `postgres://u:p@localhost:P/db` | host `localhost` | TCP [::1]:P (resolved, ::1 first) |
| `postgres://u:p@[::1]:P/db` | host `"[::1]"` (brackets kept) | nothing: `failed to lookup address information: nodename nor servname provided` |
| `postgres://u:p@%2Ftmp%2F…%2Fbs5:P/db` | socket `/tmp/…/bs5`; host shows the default (`/private/tmp`), or `::1` with `PGHOSTADDR` | the Unix socket |
| `postgres://u:p@127.0.0.1:P/db?host=/tmp/…/bs5` | **host `127.0.0.1`**, socket `/tmp/…/bs5` | the Unix socket |
| `…@db.example.com:P/db?hostaddr=127.0.0.1` | host `127.0.0.1` (the name is dropped) | TCP 127.0.0.1:P |
| `…@db.example.com:P/db?hostaddr=::1` | host `::1` | TCP [::1]:P |
| `…@127.0.0.1:P/db?port=Q` | port Q | TCP 127.0.0.1:Q |
| `…@127.0.0.1:P/db?user=u2&password=p2&dbname=db2` | user `u2`, database `db2`, password `p2` (Debug) | |
| `…@127.0.0.1:P,127.0.0.1:Q/db` | parse error `invalid port number` | |
| `…@127.0.0.1,10.0.0.1/db` | host `"127.0.0.1,10.0.0.1"`, verbatim | not tried |
| `postgres://u:p@127.0.0.1/db` | port 5432; with `PG*`: Q | with `PG*`: TCP 127.0.0.1:Q |
| `postgres://u@127.0.0.1:P/db`, `postgres://u:@…` | password (Debug) None; `env_password` with `PG*`; `pgpass_password` with `~/.pgpass` | |
| `postgres://127.0.0.1:P/db` | user = a default (`anonymous` in the cleared environment); `env_user` with `PG*` | |
| `postgres://u:p@127.0.0.1:P` | database None; `env_db` with `PG*` | |
| `postgres:///db` | no `PG*`: host `/private/tmp`, port 5432, i.e. the socket of the native server on 5432; with `PG*`: `::1`:Q (`PGHOSTADDR` beats `PGHOST`), without `PGHOSTADDR`: the `PGHOST` socket | with `PG*`: TCP [::1]:Q / the socket |
| `postgres://u:p@:P/db` | parse error `empty host` | |
| `mysql://u:p@127.0.0.1:P/db` | parses: the scheme is not checked | TCP 127.0.0.1:P |

The check on the URL itself (`url::Url`: scheme `postgres` or `postgresql`, non-empty `host_str()`, `port()`, non-empty `username()`, non-empty `password()`, a path after `/`, and none of the query keys `host`, `hostaddr`, `port`, `user`, `password`, `dbname`) named every missing or overridden part above, in every environment. `u:@` counts as no password. Only the full URLs passed.

Notes for the guard (§9.3): read `get_socket()` before `get_host()`; with all five parts in the URL and none of those query keys, no `PG*` variable changes the target (rows 1 and 2); `?hostaddr=` is visible in `get_host()`, so refusing it is a choice, not a necessity; an unknown query parameter is ignored with a `tracing` warning. A URL with an IPv6 literal (`[::1]`) passes a loopback check but sqlx 0.9.0 on tokio cannot connect to it; `localhost` and `?hostaddr=::1` work.

**Not a contradiction of §5's text, but the rule cannot rest on the getters:** the password has no getter (it shows only in the derived `Debug`, which §4.1 forbids), and every getter shows a `PG*` or `~/.pgpass` value as if the URL had named it.

### A.6 S6, statement logs

**Answer.** sqlx logs each statement through `tracing`, target `sqlx::query`, at `debug` by default, and a statement slower than the threshold at `warn`. sqlx 0.9.0's default threshold is 1 second, so §6.2's 250 ms needs `log_slow_statements(LevelFilter::Warn, Duration::from_millis(250))` on the connect options. No bound value appears in any event. The value can appear in the error the statement returns.

**Evidence** (`s6_logs`, JSON lines; five distinct bound values; `grep` for them in the logs: 0 matches in both modes):

```
{"level":"DEBUG","target":"sqlx::query","fields":{"summary":"SELECT $1::text AS secret, …",
  "db.statement":"\n\nSELECT $1::text AS secret, $2::int4 AS number\n","rows_affected":1,"rows_returned":1,
  "elapsed":"1.348ms","elapsed_secs":0.001348}}
{"level":"WARN","target":"sqlx::query","fields":{"message":"slow statement: execution time exceeded alert threshold",
  "summary":"SELECT $1::text FROM pg_sleep(0.3)","db.statement":"","rows_affected":1,"rows_returned":1,
  "elapsed":"311.795792ms","elapsed_secs":0.311795792,"slow_threshold":"250ms"}}
defaults: the same 312 ms statement only at DEBUG (threshold 1 s)
{"level":"INFO","target":"sqlx::postgres::notice","fields":{"message":"table \"no_such_table\" does not exist, skipping"}}
returned error, not logged by sqlx: invalid input syntax for type integer: "FAILING-BOUND-SECRET" at line 615
```

Fields: `summary` (the first four words, with ` …` when cut), `db.statement` (the whole SQL, or empty when it has four words or fewer), `rows_affected`, `rows_returned`, `elapsed`, `elapsed_secs`; the slow event adds `message` and `slow_threshold`. The SQL is not reformatted. The `debug` event has no `message` field, unlike the rule of `CLAUDE.md` that every event has one; it shows only at `LOG_LEVEL=debug` or `trace`. Server notices (`NOTICE`) are logged at `info` under `sqlx::postgres::notice`, so `LOG_LEVEL=info` shows them, such as a `DROP DATABASE IF EXISTS` in `reset`; a server `WARNING` at `warn` (read in the source, `sqlx-postgres-0.9.0/src/connection/stream.rs`). A seed's SQL, demo data included, goes into `db.statement` at `debug`, and at `warn` when slow.

### A.7 S7, containers

**Answer.** Pick testcontainers 0.28.0 with a Postgres image definition of our own (about 35 lines): it behaves exactly as 0.27.3 with testcontainers-modules 0.15.0, is the current release, has a smaller tree, and the module brings nothing we keep (we replace its tag, user, password and database; its `fsync=off` and its two ready conditions are the lines we wrote). Both remove the container on a normal end, on a panic, on nextest's SIGTERM, on a plain SIGTERM and on SIGINT, with the `watchdog` feature; only SIGKILL leaves one, which its label finds.

**Evidence** (`s7-tc28`, `s7-tc27`, `scripts/s7-run.sh`; each test in its own process under nextest, containers labelled `barras-spike=1` and `spike-case=<case>`, checked with `docker ps -a --filter label=…` afterwards):

```rust
impl Image for Postgres18 {
    fn name(&self) -> &str { "postgres" }
    fn tag(&self) -> &str { "18.7-trixie@sha256:2ea8630df176ad090ec92136dee850397c5af8781e4bcf2e4162593691c61bbc" }
    fn ready_conditions(&self) -> Vec<WaitFor> { vec![
        WaitFor::message_on_stderr("database system is ready to accept connections"),
        WaitFor::message_on_stdout("database system is ready to accept connections")] }
    fn env_vars(&self) -> … { [("POSTGRES_USER","barras"), ("POSTGRES_PASSWORD","barras"), ("POSTGRES_DB","barras")] }
    fn cmd(&self) -> … { ["postgres", "-c", "fsync=off"] }
    fn expose_ports(&self) -> &[ContainerPort] { &[ContainerPort::Tcp(5432)] }
}
// Postgres18.with_label("barras-spike", "1").start().await
// testcontainers = { version = "=0.28.0", default-features = false, features = ["watchdog"] }
```

| Case (same result for both versions) | Test outcome | Left behind |
| --- | --- | --- |
| normal end | PASS | none |
| panic after start | FAIL | none |
| nextest time limit (`slow-timeout` 5 s × 2, grace 10 s) | `TIMEOUT [10.142s]`; nextest returned after 11 s | none |
| `kill -TERM` | exit 143 after 0.18 s | none |
| `kill -INT` | exit 130 after 0.17 s | none |
| `kill -KILL` | exit 137 | **one**, removed by its label |

Every container answered `SELECT 1` and `SHOW fsync` = `off` at once after the ready conditions. The image by tag and digest works: the first start pulled `postgres:18.7-trixie@sha256:…` from Docker Hub itself.

Times, image present, measured from `start()` (0.28.0; 0.27.3 within 10%):

| Measure | Result |
| --- | --- |
| one container, start to ready | 0.61 to 0.65 s (5 runs) |
| one container, start to first `SELECT 1` | 0.65 to 0.69 s |
| removal on drop | 0.10 to 0.14 s |
| eight at once, all eight answering | 1.29 to 1.30 s (2 rounds; 0.27.3: 1.27 to 1.38 s) |
| eight removed on drop, one after the other | 0.86 to 0.89 s |
| first start with the pull from Docker Hub | 6.2 s |

Docker unreachable (`DOCKER_HOST` at a missing socket): the test fails at once with `Client(Init(SocketNotFoundError("…")))`; the testkit can turn that into its own sentence.

Trees (normal dependencies, sqlx included): 279 crates for 0.28.0, 291 for 0.27.3 with the module; no licence outside the repository's allow list. Both reach `hyper` 1.12.0 and `hyper-util` 0.1.21. The crates that depend directly on `hyper` are `bollard`, `hyper-rustls`, `hyper-timeout`, `hyper-util`, `hyperlocal` and `tonic`; on `hyper-util`, the same minus itself. Those are the wrappers §4.6 means. testcontainers itself reads `DOCKER_HOST`, `DOCKER_CONFIG`, `DOCKER_AUTH_CONFIG`, `DOCKER_CERT_PATH`, `DOCKER_TLS_VERIFY`, `DOCKER_DEFAULT_PLATFORM` and `TESTCONTAINERS_COMMAND` (`keep` turns off removal on drop), and finds Docker Desktop's socket under `~/.docker/run/`. The testkit's own code can still read none (§4.5); none of these can point a test at the local database.

### A.8 S8, sqlx-cli

**Answer.** Both work and do the same thing: `cargo install sqlx-cli@0.9.0 --no-default-features --features postgres`, with or without `--locked`. The published crate holds a `Cargo.lock`, and cargo 1.100 uses a package's lock file by default (`cargo help install`: "By default, Cargo uses the Cargo.lock file included with the package if it is available … The --locked flag does not currently change dependency resolution for cargo install"). sqlx's changelog ("`cargo install --locked sqlx-cli` will no longer work") does not hold for this crate and this cargo. A cold build takes 24 s on the M4.

**Evidence** (`scripts/s8.sh`, each build in a fresh temporary target folder, installed under `<scratch>/spikes/tools`):

```
--locked:     Finished `release` profile [optimized] target(s) in 24.40s   real 24.56 user 126.43   (40 crates downloaded)
no --locked:  Finished `release` profile [optimized] target(s) in 18.90s   real 19.05 user 125.62   (nothing to download)
both: 166 crates compiled, the same names and versions (diff empty), e.g. tokio v1.52.1 from the packaged lock (1.54.1 is newest)
both: warning: package `chacha20 v0.10.0` in Cargo.lock is yanked in registry `crates-io` (also der 0.8.0, crypto-bigint 0.7.3, spin 0.9.8)
$ sqlx --version  ->  sqlx-cli 0.9.0
```

The yanked packages only warn; sqlx-cli is not part of the workspace, so `deny.toml`'s `yanked = "deny"` does not see them. Keep `--locked` in `just setup` and CI, like the other tools. CI's runners have fewer cores: the build needs about 126 s of CPU time, so expect around a minute there (an estimate, to read from the first run).

### A.9 S9, the registry

**Answer.** The digest is the same on all three registries: index `sha256:2ea8630df176ad090ec92136dee850397c5af8781e4bcf2e4162593691c61bbc` (built 2026-11-12 from docker-library/postgres `185fae35…`, `18/trixie`). CI can pull that digest from `mirror.gcr.io` and tag it `postgres:18.7-trixie`, so the testkit, which names Docker Hub's reference, finds it without a pull. Docker Hub stays the fallback.

**Evidence:**

```
$ docker buildx imagetools inspect <registry>/postgres:18.7-trixie
  docker.io/library, mirror.gcr.io/library, public.ecr.aws/docker/library:
  Digest: sha256:2ea8630df176ad090ec92136dee850397c5af8781e4bcf2e4162593691c61bbc
  linux/amd64    sha256:c7fedce47eff6a78cb35dad6b627f0d684107f75ab15c0870e40c093766ba452
  linux/arm64/v8 sha256:ede0133b5b300f7609629c2bd8ac2cf8cbaf830527200ea2cbecf80f4481f2d3
Docker Hub, anonymous manifest HEAD:   ratelimit-limit: 100;w=3600   (per address)
mirror.gcr.io, same HEAD:              no rate-limit header
public.ecr.aws, imagetools inspect:    429 Too Many Requests / toomanyrequests: Rate exceeded
  10 back-to-back HEADs: 200 200 200 200 200 200 429 200 200 200;   one per 1.2 s: all 200
$ docker pull <registry>/postgres@sha256:2ea8…   -> works from all three (mirror.gcr.io cold: 12 s here)
mirror.gcr.io is a cache: postgres:10.0-alpine 200, 9.6.1 200, but 8.4.22 404 (Docker Hub: 200)
$ docker pull mirror.gcr.io/library/postgres@sha256:2ea8… ; docker tag mirror.gcr.io/library/postgres@sha256:2ea8… postgres:18.7-trixie
$ docker image inspect postgres:18.7-trixie@sha256:2ea8…   -> sha256:2ea8…   (before the tag: No such image)
$ docker run --rm --pull=never postgres:18.7-trixie@sha256:2ea8… postgres --version  -> postgres (PostgreSQL) 18.7 (Debian 18.7-1.pgdg13+2)
```

The image's configuration matches §10: `PGDATA=/var/lib/postgresql/18/docker`, `VOLUME /var/lib/postgresql`, `STOPSIGNAL SIGINT`. Pulled by tag and digest (as testcontainers does), the image is stored under the name `postgres:18.7-trixie@sha256:…`: `docker image inspect postgres:18.7-trixie` alone then answers "No such image", so `compose.yaml`, the testkit and CI should all write the same tag-and-digest reference.

So CI's `test` job: one step pulls `mirror.gcr.io/library/postgres@<digest>` and tags it `postgres:18.7-trixie`, falling back to `docker pull postgres:18.7-trixie@<digest>` if the mirror answers 404, which it may for an image it has not cached. ECR Public is not the pick: it throttles anonymous bursts. The re-tag was verified on Docker Desktop's containerd image store; whether the runner's image store resolves `postgres:18.7-trixie@<digest>` the same way is not verified here and shows in the first CI run (the other way is `registry-mirrors` in the runner's daemon configuration).

### Not run

- S5's IPv6 case was run against local listeners only, never against a Postgres published on `[::1]`.
- The image store question of A.9 on a GitHub runner, and the CI times of A.7 and A.8.
