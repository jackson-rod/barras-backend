# BAR-4: Database foundation, implementation plan

Date: 2026-11-16. Spec: [`docs/specs/BAR-4-database-foundation.md`](../specs/BAR-4-database-foundation.md), approved by the owner on 2026-11-16. Branch: `feat/BAR-4-database-foundation`.

The spec says what and why; this plan says in which order, with which tests, and what each task leaves behind. Where the plan and the spec disagree, the spec wins until a refinement below says otherwise.

## How the work runs

- Test first: each step writes the failing test, sees it fail for the right reason, then writes the code.
- Each task is built by a subagent and reviewed by `agent-skills:code-reviewer` before the next task builds on it. A task that touches CI, the hooks, the justfile's gates, `deny.toml` or the local-database guard is also reviewed by `agent-skills:security-auditor`. The findings are fixed, or answered with a reason, before the task counts as done. One whole-branch review closes the plan (Task 8).
- Tasks 1 and 2 need no database and run in parallel with the spikes of Task 0. Task 3 starts once Task 0 has answered.
- Docker Desktop runs for Task 0 and from Task 3 on.
- Commits read `type(BAR-4): lower-case description`, go through the hooks, and carry no attribution (ADR-0018). Nothing is pushed until the owner says so.

## Refinements of the spec

None yet. A decision made during the work that changes what the spec says is numbered here, with where it is recorded.

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

- ADR-0030 and ADR-0031 (spec §15), `Accepted`, with their rows in the index; the pointers in ADR-0007, ADR-0016, ADR-0024, ADR-0027 and ADR-0029.
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

Filled by Task 0.
