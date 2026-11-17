# BAR-4: Database foundation

Date: 2026-11-16. Status: approved by the owner on 2026-11-16.

Ticket: BAR-4 (F-04), <https://trello.com/c/0u3rUF8j>. Depends on BAR-3. Design: [foundation design](2026-11-15-barras-foundation-design.md) §6.2, §6.9, §6.13, §6.14 and §8; [ADR-0007](../adr/0007-data-access.md), [ADR-0008](../adr/0008-money-time-and-ids.md).

**Decided** marks what the owner said. **Call** marks a decision of this spec that the owner has not confirmed one by one; approving the spec accepts them.

## 1. Intent

Every module that stores data builds on one way to reach Postgres: SQL that the compiler checks against the schema, a schema that changes only through migrations, and a local database a developer can throw away and rebuild without fear of touching another one. This ticket builds that once, before the first table exists.

After this ticket:

- A repository's SQL is checked at compile time against the migrated schema, offline, and CI fails when the committed metadata no longer matches.
- The schema changes only through forward migrations, which the same binary applies in every environment: `barras-api migrate`.
- A developer rebuilds and seeds the local database with one command, which refuses any database that is not on their machine.
- Integration tests run against throwaway Postgres 18 containers and never touch the local database.
- `barras-api` reports its database in readiness, and no request holds a connection or a worker without limit.

## 2. Scope

In scope:

1. `barras-adapter-postgres`: the primary and read pools, their readiness checks, the embedded migrations, the seed runner, the local-database guard and the reset.
2. `migrations/` with the first migration, which creates the schema `barras`.
3. `.sqlx/`, the offline query metadata: `just db-prepare` writes it, `just db-check` fails when it is stale, `just verify` and CI run the check.
4. `barras-api`: the pools and their probes when it serves, and three new modes, `migrate`, `seed` and `reset` (section 3.2).
5. `barras-bootstrap`: `Secret<T>` for every variable that holds a credential.
6. `barras-adapter-http`: a request timeout with its own error code, and one readiness run shared by concurrent requests.
7. `barras-testkit`: Postgres 18 containers for the `int_` and `e2e_` tests, each test with a database of its own.
8. `compose.yaml` for the local database (section 3.1) and the commands `db-up`, `db-down`, `db-migrate`, `db-deploy`, `db-status`, `db-check`, `db-seed`, `db-reset` and `db-prepare`.
9. Repository checks: migration versions unique, seed file names, the bans and licences the new dependencies need, the workspace-hack.
10. CI: sqlx-cli, Docker for the tests, `db-check`.
11. The items carried to this card from BAR-2 and BAR-3 (section 14).
12. Docs: `CLAUDE.md`, `README.md` (Docker becomes required), `CONTRIBUTING.md`, the standards, the schema document, the ADRs of section 15.

Not in this ticket:

- Any table of a module. The first tables come with identity (BAR-15). So the repository holds no seed file yet: the seed runner is proven by its tests with fixture files, and `just db-reset` applies no seed until BAR-15 adds the demo user.
- `barras-ingest` and the database. It reads no database variable until the ingest foundation (BAR-6), which adds its own (`INGEST_` prefix, section 5).
- Mapping a database failure to a module error. The first repository (BAR-15) decides it, with the rule of section 6.4.
- Redis (BAR-5). TLS to the database, roles and grants, backups and the deployed database (BAR-12, the hosting decision).
- Metrics of the pools.

## 3. Decisions taken with the owner

The owner answered these four questions on 2026-11-16, before this spec was written. Each changes what gets built.

### 3.1 The local database runs in Docker Compose (**Decided**)

`compose.yaml` at the repository root holds one service, Postgres 18 from the same pinned image as the tests and CI (section 10). It listens on `127.0.0.1:5433`, so that it never clashes with a native server on 5432, as on the owner's machine. `just db-up` starts it and waits until it accepts connections; `just db-down` stops it and keeps its data.

### 3.2 The database commands are modes of `barras-api` (**Decided**)

As `barras-api openapi` is (BAR-3 §3.2):

- `barras-api migrate` applies the migrations the binary carries. It is the deploy step, so production needs no sqlx-cli and no `migrations/` folder.
- `barras-api seed <folder>` and `barras-api reset <folder>` load seed data and rebuild the database. Both refuse to run unless `APP_ENV=local` and the database is on this machine (section 9.3).
- sqlx-cli stays a developer tool: it creates migration files, shows their status, and writes and checks the offline metadata.

### 3.3 Migrations are forward-only (**Decided**)

A migration has no script that undoes it. A mistake is fixed by a new migration, and a change that would break the running code goes in two steps: expand (add the new shape, keep the old), then contract (remove the old) in a later release. This settles the question BAR-2 left on the card: `check-structure` keeps refusing `.up.sql` and `.down.sql` names.

### 3.4 The advisories check stays in every pull request (**Decided**)

`just deny` keeps checking advisories on every pull request, and a new advisory blocks until it is fixed (usually `cargo update -p <crate>`) or ignored in `deny.toml` with a reason and a date, in a pull request of its own. There is no scheduled-only run.

## 4. Crates and layers

The dependency table of ADR-0024 does not change: sqlx lives in `barras-adapter-postgres` (and `barras-testkit`, for the tests); `barras-api` wires them.

### 4.1 `barras-adapter-postgres`

- `Pools`: the primary pool and the read pool, built without connecting (section 6.1). When the two URLs are equal, one pool serves both roles.
- A readiness check per pool, `check_primary` and `check_read`, each one compile-time checked query, answering `Ok` or an error whose text holds no credential (section 6.3). The adapter does not know the HTTP adapter's `HealthProbe`; `barras-api` wraps each check in one.
- The migrations, embedded in the crate from `migrations/` (section 7), and `migrate`, which applies the pending ones.
- `seed`, `reset` and the local-database guard (section 9).
- It takes credentials as plain values from the composition root and never logs, displays or returns them. No type of ours that holds sqlx's connect options derives `Debug`: `PgConnectOptions` derives it and prints the password.
- Every failure it reports before a connection is made is a fixed sentence of its own, with the SQLSTATE when the server gave one, never sqlx's text: that text can carry the server's message, which names the user on an authentication failure, and one SCRAM failure prints a character of the password. Once connected, a failing migration or seed shows the server's message, which is about the SQL, with the file's name.

### 4.2 `barras-api`

- When it serves: the configuration of section 5, the pools, one probe per pool in `Dependencies`, and the pools closed after the server stops, so that Postgres sees each connection end.
- The modes of section 3.2. Each reads only the variables it uses (`migrate` reads `API_DATABASE_URL`; `seed` and `reset` read `APP_ENV` and `API_DATABASE_URL`), names every missing or invalid one at once, as the server does, and prints what it did on standard output.

```
usage: barras-api                          serve the API, configured by the environment
       barras-api openapi                  print the OpenAPI document
       barras-api openapi --check <file>   fail unless <file> holds the document the code generates
       barras-api migrate                  apply the pending migrations to API_DATABASE_URL
       barras-api seed <folder>            load the seed files of <folder> into the local database
       barras-api reset <folder>           rebuild the local database: drop, create, migrate, seed
```

### 4.3 `barras-bootstrap`

`Secret<T>`: a value that `Debug` and `Display` print as `[redacted]`, read from the environment like any other (`FromEnvValue`), and reached only through `expose()`. Every variable that holds a credential is one, so a configuration that derives `Debug` cannot print a password. It implements no `Serialize`.

### 4.4 `barras-adapter-http`

- A request timeout with the code `REQUEST_TIMEOUT` (section 6.4).
- Readiness shared: concurrent requests share one run of the probes, and a result less than one second old is reused (section 6.3).

### 4.5 `barras-testkit`

Postgres 18 containers through testcontainers, with the image of `compose.yaml`, and a database per test (section 11.2). It reads no environment variable, so it can only reach the containers it starts.

### 4.6 `workspace-hack` and the bans

- sqlx joins the workspace-hack through `barras-adapter-postgres`. `just hakari-check` already fails if a feature only the tests use would reach a release build; the testkit stays out of hakari's traversal (ADR-0024).
- testcontainers talks to Docker through crates that the bans reserve for the HTTP adapters (`hyper`, `hyper-util`). Each one that needs it becomes a wrapper in `deny.toml`, with a reason that says it is the Docker client of the test containers. Only the testkit depends on them.

## 5. Configuration

New variables, read only by `barras-api`, so they take its prefix (BAR-3 §8.1):

| Variable | What it holds | In `.env.example` |
| --- | --- | --- |
| `API_DATABASE_URL` | The primary database: a `postgres://` URL with its credentials. A `Secret` | `postgres://barras:barras@127.0.0.1:5433/barras` |
| `API_DATABASE_READ_URL` | The read database: a replica when there is one, the primary's URL when there is not. A `Secret` | the same URL |
| `API_DATABASE_POOL_SIZE` | The most connections each pool opens; a whole number from 1 | `10` |

- Every variable stays required, with no default in code (BAR-3 §8.1). An environment with no replica gives the read URL the primary's value, and then the binary opens one pool (section 6.1).
- A URL the driver cannot read is reported at start like any invalid variable: by its name, never its value.
- A URL names its host, port, user, password and database, or it is invalid. sqlx fills whatever a URL leaves out from the `PG*` environment variables and `~/.pgpass`, which would be a second place where the configuration lives (ADR-0016). sqlx still reads `PGOPTIONS`, `PGSSLMODE` and the others for what no URL of ours carries; no environment of ours sets them, and ADR-0030 records it as a limit.
- The local values are those of `compose.yaml`; they hold only characters a shell reads the same way, as `env_file` requires.
- Each binary keeps its own variables (**Call**): `barras-ingest` gets `INGEST_DATABASE_URL` and its own pool size in BAR-6. Its jobs write where the API reads, but the two can then use different database roles and pool sizes.

## 6. Pools, readiness and timeouts

### 6.1 Pools (**Call**)

- Built lazily: the binary opens no connection at start, serves at once, and readiness says whether the database answers. A database that is down when the binary starts is a readiness failure, not a start-up failure, so an instance recovers on its own when the database comes back.
- Each connection names itself to Postgres with `application_name=barras-api`, so `pg_stat_activity` shows who holds it.
- Getting a connection, opening it included, gives up after 2 seconds (sqlx's default is 30); until then sqlx retries a refused connection.
- A statement on a serving pool is cancelled by Postgres after 5 seconds (`statement_timeout`), the request timeout, so no statement outlives its request for long. `migrate`, `seed` and `reset` connect without that limit: a migration may build an index for longer.
- Idle connections close after 10 minutes and every connection after 30 minutes. These are sqlx's defaults, written out so that an upgrade cannot change them unseen. No connection is kept open when idle (`min_connections` 0).

### 6.2 SQL logged

sqlx logs every statement at `debug`, which `LOG_LEVEL=info` drops, and a statement slower than 250 ms at `warn`, with its SQL and its duration (**Call**). The SQL text is code, not data; the plan confirms that sqlx never logs the values bound to a statement.

### 6.3 Readiness

- `barras-api` registers one probe per pool: `postgres` for the primary and `postgres-read` for the read pool, both `Critical`, because the app's screens read from both. With one pool there is one probe.
- A check runs one compile-time checked query, `SELECT 1`, through the pool, within the probe's 2 seconds (BAR-3).
- Its error is one sentence from a fixed set (section 4.1), such as `no connection within 2 seconds` (the database cannot be reached, or every connection is busy) or `the server refused it with SQLSTATE 28P01`. It never holds a URL, a user name or a password (the card's carry-over note). The internal `/health` and the log show it; the public route never does.
- Readiness shared (**Call**): `GET /api/v1/health` is public, and in BAR-3 every request ran every probe. Now concurrent requests share one run, and a result less than one second old is answered again, on the public port and the internal one. So the public route cannot be used to load the database.

### 6.4 Request timeout (**Call**)

- A request whose answer has not started within 5 seconds is answered 503 with the envelope and `error.code` `REQUEST_TIMEOUT`, and its handler is dropped. 5 seconds is far above every budget of ADR-0014 and under the 10-second drain (ADR-0029), so a request in flight at SIGTERM ends before the drain does.
- The time counts from the request's arrival to the head of the answer, reading the body included. A body that streams out is not limited.
- `REQUEST_TIMEOUT` joins the codes every route can return (`x-common-error-codes`, now eight), and 503 joins the statuses `TestClient` accepts without the operation documenting them. `openapi.json` changes with it.
- Its place in the stack of BAR-3 (plan, appendix A.3): inside the request log, `PropagateRequestId`, the metrics and the safety net, so that a timeout is logged, counted and carries the request id; outside compression, `CatchPanic` and the body limit, so that it bounds everything a handler does.
- A timeout is a 5xx, so it is logged once as `request failed`, like any other.
- The rule for repositories, for BAR-15 to apply: a database that cannot be reached, a pool with no free connection and a cancelled statement are `ErrorKind::Unavailable`.

## 7. Migrations

- They live in `migrations/` at the repository root, one file per migration, named `YYYYMMDDHHMMSS_snake_case.sql` (BAR-2's rule). `check-structure` now also fails when two files share a version (the BAR-2 carry-over), and a name that is not that pattern, such as `.up.sql` and `.down.sql`, keeps failing (section 3.3).
- `just db-migrate <name>` creates the file through sqlx-cli, with the timestamp of the moment, and refuses a name that is not snake_case.
- They are embedded in `barras-adapter-postgres`, so a binary carries the migrations it was built with. A new migration file rebuilds the crate.
- `barras-api migrate` applies the pending ones in order, each in its own transaction, under sqlx's advisory lock so that two deploys cannot interleave. sqlx-cli's own `migrate run` takes no such lock, one more reason deploys use the binary. It prints one line per migration it applies, or that there is nothing to apply. It fails, before applying anything, when a migration already applied differs from its file (it was edited after it ran) or is missing from the binary.
- A migration that cannot run inside a transaction, such as `CREATE INDEX CONCURRENTLY`, starts with sqlx's `-- no-transaction` line and a comment that says why.
- The first migration creates the schema `barras`, with a comment that every table lives in it.
- SQL names the schema of every table it touches (`barras.users`) (**Call**). No query depends on a connection's `search_path`, which `cargo sqlx prepare` would not share.
- sqlx keeps its history table, `_sqlx_migrations`, in `public`, its default (**Call**). Moving it into `barras` takes a `sqlx.toml` in the adapter's folder and another where sqlx-cli runs, which must agree, plus the sqlx feature that reads them; no `sqlx.toml` exists.
- SQL that looks odd carries a comment that says why (ADR-0007).

## 8. Offline query metadata

- `.sqlx/` at the repository root holds what the compile-time checked queries need, and is committed (ADR-0007).
- Every cargo command compiles with `SQLX_OFFLINE=true`, set in `.cargo/config.toml` (**Call**). So no build connects to a database, whatever `DATABASE_URL` or a `.env` file holds: the sqlx macros read `.env` at compile time, and `.env` names the local database.
- `just db-prepare` starts a throwaway Postgres 18 container, applies every migration to it, rewrites `.sqlx/` with `cargo sqlx prepare`, and removes the container. Run it after changing a query or adding a migration, and commit what it changes.
- `just db-check` does the same, but with `--check`: it fails when a migration does not apply to an empty database, or when `.sqlx/` differs from what the current queries and schema produce. It also fails on a file in `.sqlx/` that no query produces any more, which `cargo sqlx prepare --check` only warns about. This is the card's "CI fails when `.sqlx` is stale".
- Neither ever reads `API_DATABASE_URL` or touches the local database.
- `just verify` runs `db-check` after `openapi-check`, and so does CI's `checks` job.
- The readiness query of section 6.3 is the first checked query, so `.sqlx/` and its check are real from this ticket on.
- A query is a compile-time checked macro (`query!`, `query_as!`, `query_scalar!`). Only SQL the compiler cannot see takes a string at run time: a seed file, and the statements that drop and create a database. sqlx 0.9 makes each of those say so with `AssertSqlSafe`, so review finds them all.

## 9. Seeds, reset and the local-database guard

### 9.1 Seeds (**Call**)

- A seed is a SQL file in `seeds/`, named like a migration (`YYYYMMDDHHMMSS_snake_case.sql`; `check-structure` checks it). `barras-api seed <folder>` applies every `.sql` file of the folder in name order, in one transaction: a file that fails rolls the whole run back, and the error names the file and the SQLSTATE. A folder that does not exist counts as empty.
- Seeds are local development data only, such as the demo user. Reference data that every environment needs (the role `trader`, the permission catalog, the plans, the currencies, the banks and exchanges of design §8) never goes in a seed: it comes in a migration or from its module's code, since seeds never reach production.
- A seed assumes a database just migrated. Run twice, the first conflict rolls the whole run back and changes nothing; `just db-reset` is the way to start again.
- Seeds are read from the folder when the command runs, never embedded, so the binary never carries the demo user's data.

### 9.2 Reset

`barras-api reset <folder>`: the guard, then drop the database the URL names (ending the other sessions on it), create it, apply every migration, and apply the seeds. It prints each step.

### 9.3 The local-database guard

`seed` and `reset` run only when both hold, and say which does not, before any connection:

1. `APP_ENV` is `local`.
2. The database is on this machine: its address is a Unix socket, a loopback address (`127.0.0.0/8`, `::1`), or `localhost` resolving only to loopback addresses.

The guard judges the address sqlx would connect to, from the URL as sqlx reads it, and refuses a URL whose target it cannot judge, such as several hosts or a separate host address parameter. Its message names the reason, never the URL. The container on `127.0.0.1:5433` passes; a database on another machine never does, whatever `APP_ENV` says.

## 10. Local database

- `compose.yaml` (section 3.1): the image, pinned by tag and digest; port `127.0.0.1:5433`; user, password and database `barras`; a named volume for the data; a health check, so that `docker compose up --wait` returns once Postgres accepts connections.
- The volume is mounted at `/var/lib/postgresql`, where the 18 image keeps its data (`/var/lib/postgresql/18/docker`). Older guides mount `/var/lib/postgresql/data`, and the 18 image refuses to start with anything mounted there.
- The image is Postgres 18 on Debian, not Alpine (**Call**): its collation comes from glibc, as on a managed Postgres. Today that is `18.7-trixie`. `compose.yaml` is the one place that names it: the testkit and the throwaway container of `db-check` and `db-prepare` read it from there.
- The local routine: `just db-up`, then `just db-reset`, then `just dev`. `just dev` serves while the database is down, with readiness at 503.

## 11. Tests

### 11.1 Tiers

- `int_` targets in `barras-adapter-postgres` run against a real Postgres: the pools and the reuse of one pool, the readiness checks up and down, `migrate` (pending, nothing pending, an edited migration refused, an unknown one refused), `seed` (order, one transaction, a failing file), `reset`, and the guard's refusals.
- `e2e_` targets in `barras-api`: the process tests start a container. Serving with the database up, readiness answers `ok`; with the database unreachable, the public route answers 503 and the internal `/health` names `postgres` with its fixed sentence and no credential. The modes `migrate`, `seed` and `reset` run as processes, and a guard refusal never connects.
- Unit tests: `Secret`, the configuration, the guard's decisions, the seed files' selection and order, the timeout layer, the shared readiness run.
- `just test` stays free of Docker. `just test-int` and `just test-cov`, and so `just verify` and the pre-push hook, need Docker running (ADR-0027 said so from this ticket).

### 11.2 Containers

- Each test starts its own container and gets a migrated database in it that no other test touches (**Call**). nextest runs every test in a process of its own, and a container shared by a process would never be removed: a value held in a `static` is never dropped. Sharing a container across processes exists in testcontainers and in nextest (setup scripts), but both are experimental.
- A test that ends, passed or failed, removes its container: testcontainers removes a container when its handle is dropped, a panic included. A test that nextest's time limit stops gets SIGTERM first, and testcontainers' `watchdog` feature removes its containers then. Only SIGKILL leaves one behind; every container carries a label that finds it.
- Throwaway containers run with `fsync=off`, since their data never outlives them. The local database of `compose.yaml` keeps it on.
- Docker not running fails the test with a message that says so. A test that skipped itself would hide a failure.
- No test reaches the local database: the testkit reads no environment variable, and the process tests hand each binary its whole environment, as BAR-3's helper does.

### 11.3 Coverage

The adapter's `int_` tests count, as every tier does (ADR-0027). `barras-adapter-postgres` is measured from this ticket on.

## 12. Commands and CI

| Command | What it does |
| --- | --- |
| `just db-up` | Starts the local Postgres of `compose.yaml` and waits until it accepts connections |
| `just db-down` | Stops it; its data stays in the volume |
| `just db-migrate <name>` | Creates `migrations/<timestamp>_<name>.sql` with sqlx-cli; refuses a name that is not snake_case |
| `just db-deploy` | `barras-api migrate` against the database of `.env` |
| `just db-status` | sqlx-cli's `migrate info` against the database of `.env`: applied, pending, and changed after they ran |
| `just db-check` | A throwaway container: every migration from empty, then fails when `.sqlx/` is stale |
| `just db-seed` | `barras-api seed seeds` against the database of `.env`; local only |
| `just db-reset` | `barras-api reset seeds` against the database of `.env`; local only |
| `just db-prepare` | A throwaway container: every migration, then rewrites `.sqlx/` |

- `just setup` also installs sqlx-cli 0.9.0, with Postgres support only.
- `just verify`: `check-arch`, `check-structure`, `hakari-check`, `fmt-check`, `deny`, `lint`, `openapi-check`, `db-check`, `test-cov`, `test-doc`, `build`.
- CI, `checks` job: sqlx-cli built from source at its pinned version, as cargo-hakari is, because install-action has no manifest for it; then `just db-check` after the OpenAPI step. `test` job: the Postgres image pulled in a step of its own before the tests, so that a slow pull is not charged to a test's time limit.
- The plan measures `just verify` and the pre-push hook on the real tree (the BAR-2 carry-over) and CI's first run, against ADR-0014's 10 minutes.

## 13. Done when

| Card item | Met by |
| --- | --- |
| `just db-reset` rebuilds and seeds a local database | `barras-api reset` in the process tests, against a container, with fixture seeds; and once on the owner's machine against `compose.yaml`, recorded in the plan. The repository has no seed file until BAR-15 (section 2) |
| Integration tests run against a throwaway container and never touch the local database | Section 11.2: every URL a test uses comes from a container it started; the testkit reads no environment variable, which clippy denies outside BAR-3's two exceptions; the process tests hand each binary its whole environment, as BAR-3's helper is tested to do |
| CI fails when `.sqlx` is stale | `just db-check` in CI's `checks` job. Shown by mutation on a scratch branch: a query changed without `just db-prepare`, and a migration that changes a column a query reads |

Plus the definition of done of `CLAUDE.md`.

## 14. Carried over from BAR-2 and BAR-3

| Note on the card | Where |
| --- | --- |
| Decide whether migrations are reversible (BAR-2) | Section 3.3: forward-only |
| Duplicate migration versions are not detected (BAR-2) | Section 7: `check-structure` |
| Measure `just verify` and the pre-push hook on a real tree (BAR-2) | Section 12: the plan's measurements |
| Advisories on a schedule? (BAR-2) | Section 3.4: they stay in every pull request |
| The testkit's crate doc (BAR-2) | Section 4.5: it names containers, fixtures and the process helper, and says the crate is test-only |
| A `Secret` type before the first secret variable (BAR-3) | Section 4.3 |
| A server-side request timeout (BAR-3) | Section 6.4 |
| Docker starts here (BAR-3) | Sections 11 and 12; the README's Docker row becomes "required" |
| A readiness probe for Postgres (BAR-3) | Section 6.3 |
| Public health runs every probe on every request (BAR-3) | Section 6.3: one shared run, reused for one second |
| A probe's error text holds no credential (BAR-3) | Section 6.3 |

## 15. Decisions to record

Approving this spec accepts these:

| ADR | Content |
| --- | --- |
| ADR-0030 (new) | The database foundation: sections 3.1 to 3.3, the pools and their limits (6.1), URLs that name all five parts and the `PG*` limit, schema-qualified SQL, migrations applied by `barras-api migrate` with sqlx's history table in `public`, `SQLX_OFFLINE` and `db-check`, seeds as local SQL files, the local-database guard, the image named once in `compose.yaml`, a container per test |
| ADR-0031 (new) | The request timeout and `REQUEST_TIMEOUT`; readiness shared and reused for one second; the readiness error text |

ADR-0007, ADR-0016, ADR-0024, ADR-0027 and ADR-0029 keep their Decisions; each Consequence that is no longer accurate gets a pointer: ADR-0007 to `barras-api migrate` and `db-check`; ADR-0016 to `Secret`; ADR-0024 to the wrappers for the Docker client; ADR-0027 to the `int_` tier, now real; ADR-0029 to the eighth common code.

No ADR describes the advisories check, and section 3.4 keeps it as it is, so the decision is written where the check lives: a comment in `deny.toml`'s `[advisories]` and a line in `CONTRIBUTING.md` on what to do when a new advisory blocks.

## 16. Left to the plan

- The exact versions and features, starting from what a reading of the published crates found on 2026-11-16: sqlx 0.9.0 with `default-features = false` and `postgres`, `runtime-tokio`, `macros`, `migrate`, plus `uuid`, `time`, `rust_decimal` when the first table needs them (its minimum Rust is 1.94); sqlx-cli 0.9.0 with `--no-default-features --features postgres`; the Postgres image's tag and digest, and the registry CI pulls it from.
- testcontainers: 0.28.0 with a Postgres image definition of our own, or 0.27 with testcontainers-modules 0.15, whose Postgres module still requires 0.27 and defaults to `11-alpine`. Either way the image comes from `compose.yaml`.
- Whether `cargo install --locked` works for sqlx-cli 0.9.0: its changelog says no, while the published crate holds a lock file.
- The licences and wrappers `deny.toml` needs, read from the real dependency tree, and the regenerated workspace-hack.
- What spikes prove first, because sqlx 0.9 is six months old and its 0.8 behaviour cannot be assumed: the offline build with `SQLX_OFFLINE=true` in `.cargo/config.toml` while a `.env` names a database (the macros parse every `.env` from the crate's folder up); `cargo sqlx prepare --check` against a throwaway database, which runs cargo with `SQLX_OFFLINE=false`, a value the config must not override; the build script that makes a new migration file rebuild the embedded migrations; a multi-statement seed file through `raw_sql`; how sqlx reads `host`, `hostaddr` and the `PG*` fallbacks, for the guard and for the five parts a URL must name; that sqlx logs no bound values.
- The middleware layer for the timeout, against BAR-3's stack.

## 17. Risks

1. sqlx 0.9.0 is recent. The plan spikes every behaviour of section 16 before building on it.
2. A container per test makes the `int_` and `e2e_` tiers slower as tests are added: each one starts Postgres from nothing. The plan measures the cost per test; if it threatens `just test-int` or the 10-minute budget, a container shared across a run is the next step, through one of the two experimental features of section 11.2, and the owner decides.
3. `just verify` now needs Docker running and compiles the workspace once more for `db-check`. A developer without Docker running cannot push; the README says so.
4. CI pulls the Postgres image on every run. Docker Hub limits anonymous pulls; the plan picks a registry that does not.
5. sqlx-cli is large to build, and install-action has no manifest for it, so CI builds it from source on every run, as it does cargo-hakari, with sccache's help. The plan measures it.
6. A test process killed with SIGKILL cannot clean up after itself, and its container stays running until removed by its label. nextest sends SIGTERM first, which the `watchdog` feature handles.
7. Pools built lazily move a wrong password from a start-up failure to a readiness failure. The probe's fixed sentence names the class of failure, and the instance never becomes ready.
