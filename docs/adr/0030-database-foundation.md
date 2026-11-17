# ADR-0030: Database foundation

Status: Accepted
Date: 2026-11-17

## Context

- ADR-0007 chose sqlx 0.9 with hand-written SQL checked at compile time, two pools, and a schema that changes only through migrations. BAR-4 builds that once, before the first table exists, so that every module that stores data reaches Postgres one way (BAR-4 spec §1).
- The owner answered four questions on 2026-11-16, before the spec was written (spec §3, **Decided**): the local database runs in Docker Compose; the database commands are modes of `barras-api`; migrations are forward-only; and the advisories check stays in every pull request.
- The owner approved the spec the same day. Its **Calls** are accepted with it (spec §15): the pools and their limits, the statement logs, schema-qualified SQL, sqlx's history table in `public`, `SQLX_OFFLINE` for every build, seeds as local SQL files, the Debian image named once, a container per test, and a binary's own database variables.
- sqlx 0.9 was six months old, so spikes tried each behaviour the spec relied on before anything was built on it (plan, appendix A). The owner accepted the refinements they led to (plan, refinements 1 to 6, and 8), and the ignore of one advisory in this pull request (refinement 14).
- The rest are **Calls of the implementation**, made in the reviews and security audits of the adapter, the seeds and the guard, after that approval (plan, refinements 9 to 13, 15 and 16): the history table named for sqlx-cli, no query string in a URL, a failing migration's message kept only for errors in its SQL, `SQLX_OFFLINE` in the adapter's build script, `just build` without the testkit and `xtask`, the server's `cluster_name`, and the seed transaction checked by its id. So are two choices of the work on the commands and CI: CI's pull through a registry mirror, where the plan had chosen a pull and a tag; and, from its security audit, the acceptance of the advisories in sqlx-cli's own lock file.

## Decision

### The local database

- `compose.yaml` at the repository root holds one service, `postgres`: Postgres 18 on `127.0.0.1:5433`, so that it never clashes with a native server on 5432; user, password and database `barras`; a named volume at `/var/lib/postgresql`, where the 18 image keeps its data; and a health check over TCP. `just db-up` starts it and waits until it accepts connections; `just db-down` stops it and keeps its data.
- The image is named once, on the `image:` line of that service, pinned by tag and digest: `postgres:18.7-trixie@sha256:2ea8630df176ad090ec92136dee850397c5af8781e4bcf2e4162593691c61bbc`. Debian, not Alpine: its collation comes from glibc, as on a managed Postgres. The testkit's containers, the throwaway database of `just db-prepare` and `just db-check`, and CI's pulls read the image from that line, by the service's name. The testkit and the throwaway database refuse an image without both a tag and a digest.
- The Postgres of `compose.yaml`, the testkit's containers and the throwaway database start with `-c cluster_name=barras-local`, the setting the local-database guard requires (below).

### The database commands

- `barras-api migrate` applies the migrations the binary carries. `barras-api seed <folder>` loads the seed files of a folder into the local database, and `barras-api reset <folder>` rebuilds it.
- Each reads only the variables it uses: `migrate` reads `API_DATABASE_URL`; `seed` and `reset` read `APP_ENV` and `API_DATABASE_URL`. Each names every missing or invalid one at once, prints one line per step on standard output, and says on standard error why it stopped, never with a URL, a password or the driver's text. None starts the logs.
- `just db-deploy`, `just db-seed` and `just db-reset` run them against the database of `.env`, the last two with the folder `seeds`.
- sqlx-cli 0.9.0 stays a developer tool, installed by `just setup` with `--locked`, Postgres support alone and its `sqlx-toml` feature. It creates migration files (`just db-migrate`), shows their status (`just db-status`), and applies the migrations to the throwaway database and writes or checks the offline metadata (`just db-prepare`, `just db-check`). Its `migrate info`, behind `just db-status`, is no pure read: it creates `public._sqlx_migrations` in a database that lacks it.

### Configuration

- `barras-api` reads `API_DATABASE_URL`, the primary database; `API_DATABASE_READ_URL`, a replica when there is one and the primary's URL when there is not; and `API_DATABASE_POOL_SIZE`, the most connections each pool opens, a whole number from 1. All three are required. Both URLs are `Secret`s of `barras-bootstrap` (spec §4.3), so a configuration that derives `Debug` prints neither.
- A database URL names its host, port, user, password and database, or it is invalid, and it is reported at start by the variable's name, never its value. sqlx fills whatever a URL leaves out from the `PG*` variables and `~/.pgpass`, which would be a second place where the configuration lives (ADR-0016), and its getters show such a part as if the URL had named it (plan, appendix A.5). So the rule reads the URL itself, with the `url` crate sqlx reads it with (plan, refinements 4 and 10):
  - the scheme is `postgres` or `postgresql`;
  - it names one host, with no comma, written or percent-encoded, and no IPv6 address in brackets, which sqlx 0.9 cannot connect to (`localhost` reaches `::1`);
  - it names a port, a user, a password that is not empty, and a database;
  - it has no query string, not even an empty one: through one, sqlx would take a part from a key the rule does not read, `options` or `application_name` would contest the binary's own settings, and sqlx logs the value of a key it does not know.
- Each binary keeps its own variables: `barras-ingest` reads none of these, and BAR-6 is to give it `INGEST_DATABASE_URL` and a pool size of its own (spec §5).

### The pools

- `Pools` of `barras-adapter-postgres` holds the primary pool, for writes and the reads that follow a write, and the read pool, for the read models (ADR-0007). When the two URLs are the same, as the `url` crate writes them out, one pool serves both roles.
- They are built without connecting. `barras-api` opens no connection at start and serves at once; a database that is down is a readiness failure, not a start-up failure, and the instance recovers on its own when it comes back.
- Each pool opens at most `API_DATABASE_POOL_SIZE` connections and keeps none open while idle (`min_connections` 0). Getting a connection, opening it included, gives up after 2 seconds, where sqlx's default is 30. Idle connections close after 10 minutes and every connection after 30 minutes: sqlx's defaults, written out so that an upgrade cannot change them unseen.
- Each connection of a pool names itself to Postgres with `application_name=barras-api`, so `pg_stat_activity` shows who holds it, and starts with `statement_timeout` at 5 seconds, the request timeout of ADR-0031, so that no statement outlives its request for long. `migrate`, `seed` and `reset` work on a connection of their own, without that limit: a migration may build an index for longer.
- `barras-api` closes the pools after the server stops, so that Postgres sees each connection end.
- Each pool has a readiness check, one compile-time checked `SELECT 1`. `barras-api` wraps the two in critical probes, `postgres` and `postgres-read`, or `postgres` alone when one pool serves both roles. What a probe reports is ADR-0031's.
- sqlx's event for each statement has no message, which every log event must have, so it is turned off (plan, refinement 3). A statement slower than 250 ms is logged at `warn`, under the target `sqlx::query`, with its SQL and its duration; sqlx's own threshold is 1 second. sqlx puts no value bound to a statement in either event (plan, appendix A.6).
- The adapter takes credentials as plain values from the composition root and never logs, displays or returns them. No type of it that holds sqlx's connect options has `Debug`, since theirs prints the password. Every failure it reports before a connection is made is a sentence of a fixed set, with the SQLSTATE when the server gave one, never sqlx's text, which can carry the server's message.

### SQL and migrations

- SQL names the schema of every table it touches (`barras.users`). No query depends on a connection's `search_path`, which `cargo sqlx prepare` would not share. The first migration creates the schema `barras`, with a comment that every table lives in it.
- A query is a compile-time checked macro: `query!`, `query_as!` or `query_scalar!`. Only SQL the compiler cannot see takes a string at run time, and sqlx makes each say so with `AssertSqlSafe`: a seed file, and the statements that drop and create a database.
- Migrations live in `migrations/`, one file each, named `YYYYMMDDHHMMSS_snake_case.sql`. `check-structure` refuses any other name, `.up.sql` and `.down.sql` included, and two files that share a version. `just db-migrate <name>` creates one with sqlx-cli, versioned by the moment in UTC, and refuses a name that is not snake_case before any file exists.
- Migrations are forward-only (spec §3.3, **Decided**). A migration has no script that undoes it. A mistake is fixed by a new migration. A change that would break the running code goes in two steps: expand, which adds the new shape and keeps the old, then contract, which removes the old in a later release.
- The adapter embeds the migrations when it compiles, and its build script makes a new or edited file rebuild it. So a binary carries the migrations it was built with, and production needs neither sqlx-cli nor `migrations/`.
- `barras-api migrate` is the deploy step of every environment, whatever `APP_ENV` says. On a connection of its own, under sqlx's advisory lock, so that two deploys never interleave, it reads the whole history first, and refuses, before applying anything, a migration recorded as failed, one applied whose file has changed since, and one applied that the binary does not carry. sqlx's own run checks an applied migration only when its turn comes, after applying every older pending one (plan, refinement 1). Then it applies the pending ones in order, each in its own transaction, unless its file starts with sqlx's `-- no-transaction`; such a file says why in a comment. The connection is closed afterwards, whatever happened: the lock lasts as long as the session.
- A failing migration is named by its file and its SQLSTATE. Its message shows only for the SQLSTATE classes 42, a syntax error or a broken access rule, and 0A, a feature not supported: any other message can quote the data of a table, and migrations run against production (plan, refinement 11). The migrations applied before it in the run stay applied.
- sqlx keeps its history table in `public`, named with its schema: `public._sqlx_migrations`. With a user and a schema both named `barras`, Postgres would look for the table without a schema in `barras` first, and an empty history there would show every migration as pending again. The binary names it in `migrate.rs` (`HISTORY`), and the `sqlx.toml` at the repository root names it for sqlx-cli, which reads that file only with its `sqlx-toml` feature; a test fails when the two names differ. The adapter's folder has no `sqlx.toml`, so the query macros need no feature (plan, refinement 9).

### Offline metadata

- `.sqlx/` at the repository root holds what the compile-time checked queries need, and is committed (ADR-0007). It is real from this ticket on: the readiness check, the guard, the seeds and the reset each run checked queries.
- Every cargo command compiles with `SQLX_OFFLINE=true`, set in `.cargo/config.toml`, so no build connects to a database, whatever `DATABASE_URL` or a `.env` file holds. cargo reads that file from the folder it runs in, so the adapter's build script sets the variable for the adapter's own compilation when it is unset (plan, refinement 12). A value already set wins, such as the `false` with which `cargo sqlx prepare` compiles against its database; so the config never uses `force = true`.
- `just db-prepare` starts a throwaway Postgres from the image of `compose.yaml`, on a port of `127.0.0.1` that Docker picks, with `fsync` off and the local `cluster_name`. It applies every migration with sqlx-cli, rewrites `.sqlx/` with `cargo sqlx prepare`, and removes the container, whatever happened. It runs after a query changes or a migration is added, and what it changes is committed.
- `just db-check` does the same with `--check`. It fails when a migration does not apply to an empty database, and when `.sqlx/` differs from what the queries and the migrations produce. It also fails on a file of `.sqlx/` that no query produces, which `cargo sqlx prepare --check` only warns about: it compares the names of the files with those sqlx-cli regenerated. `just verify` runs it after `openapi-check`, and so does CI's `checks` job, bounded to 10 minutes.
- Neither reads `API_DATABASE_URL` or touches the local database.

### Seeds and reset

- A seed is a SQL file in `seeds/`, named like a migration. `check-structure` checks the names, and refuses two files that share a version.
- Seeds are local development data only, such as the demo user. Reference data that every environment needs, such as the role `trader`, the permission catalog, the plans, the currencies, the banks and the exchanges, never goes in a seed: it comes in a migration or from its module's code, since seeds never reach production.
- `barras-api seed <folder>` reads every `.sql` file of the folder, hidden ones aside, before it connects, and applies them in name order, in one transaction. A file that fails rolls the whole run back, and the error names the file, the SQLSTATE, the line and column when the server points at one, and the server's message, which stays because seeds run only against local data. A folder that does not exist holds no seed. Seeds are read when the command runs, never embedded, so the binary never carries the demo user's data.
- The run reads its transaction's id (`pg_current_xact_id()`) after `BEGIN`, and requires the same id after each file, with a savepoint before each file. So a file that ends the run's transaction itself, with `COMMIT` or `ROLLBACK`, fails the run, which names it (plan, refinements 5 and 16).
- `barras-api reset <folder>`: the guard; a database name of at most 63 bytes, since Postgres would cut a longer one short and name another database; and the seed files read. Then, from the server's maintenance database, the database is dropped `WITH (FORCE)`, which ends the other sessions on it, and created again. Then every migration is applied, and the seeds. It prints each step, and each of its errors says what state the database was left in.

### The local-database guard

- `seed` and `reset` change a database only when all three hold, each checked before the next:
  1. `APP_ENV` is `local`. `barras-api` checks it before anything connects.
  2. The database is on this machine. Its address, read as sqlx reads the URL, the socket before the host, is a Unix socket, an address of `127.0.0.0/8` or `::1`, or `localhost` resolving to loopback addresses alone. Every other host is refused, a name that resolves to this machine included. `localhost` is resolved once, and every step connects to the addresses judged, so no later answer of the resolver can send a run elsewhere.
  3. The server that answers there started as the local one: its `cluster_name` is `barras-local`. A server takes that setting only when it starts. This is the first question on the connection, before anything changes, because a port of this machine can be the first hop to another one, as `ssh -L`, `kubectl port-forward` or a cloud SQL proxy makes it (plan, refinement 15).
- Each refusal is a sentence of a fixed set, which never quotes the URL. A Postgres of one's own, such as a native one, is seeded or reset only once its `postgresql.conf` holds `cluster_name = 'barras-local'` and it has restarted.
- `migrate` has no guard: it is the deploy step.

### Tests

- `barras-testkit` starts a Postgres 18 container per test, with testcontainers 0.28, from the image of `compose.yaml`: `fsync` off, since its data never outlives it; the local `cluster_name`; its port published on `127.0.0.1` alone; and an empty database `barras` that no other test touches. A test that needs the schema applies the migrations itself.
- One container per test, because nextest runs every test in a process of its own, and a container held in a `static` would never be removed. Sharing one across processes exists in testcontainers and in nextest, both experimental.
- Dropping a container removes it, also while a failed test unwinds. testcontainers' `watchdog` feature removes it when the test is stopped with SIGTERM, as nextest does at its time limit, or SIGINT. Only SIGKILL leaves one behind. Each test container carries the label `barras-test=1`, and the throwaway database of the justfile the label `barras-throwaway=1`.
- Docker not running fails the test, with a sentence that says the `int_` and `e2e_` tests need it: a test that skipped itself would hide a failure.
- No test reaches the local database. Every URL a test uses comes from a container it started; the testkit reads no environment variable, apart from the coverage profile its process helper hands on; and the process tests hand each binary its whole environment.
- testcontainers talks to Docker through crates that the bans keep for the HTTP adapters (ADR-0024). `deny.toml` bans `testcontainers` with the testkit as its only wrapper, and each crate of the Docker client under it with its parents on that path as its wrappers. So only the testkit reaches them, and no crate reaches `hyper`, `hyper-util` or `axum` through them.

### The release build and the workspace-hack

- `just build` builds the workspace without `barras-testkit` and `xtask`, which only the tests and the repository's checks use. cargo unifies the features of every package it builds: with the testkit, the binaries took testcontainers' features of `hyper-util`, among them `http2` and `server-auto`, and the public port of `barras-api` answered cleartext HTTP/2 (plan, refinement 13). `just hakari-check` compares the release build of that same selection.
- sqlx joins the workspace-hack through the adapter. `sqlx-macros` joins the third-party crates left out of hakari's traversal: hakari unifies a crate built for both the compiler's host and the target, so the features the query macros run on at compile time would otherwise reach the release build.

### Advisories

- `just deny` checks the advisories in every pull request, in CI and in `just verify`, never on a schedule alone (spec §3.4, **Decided**). A new advisory blocks every pull request until it is fixed, usually with `cargo update -p <crate>`, or ignored in `deny.toml` with a reason and a date, in a pull request of its own, so that the fix or the ignore is reviewed for itself.
- The one exception is an advisory that a pull request's own new dependency brings. An ignore that matches no crate fails the check (`unused-ignored-advisory = "deny"`), so it cannot land before that dependency. It is ignored in that pull request instead, scoped by a ban in `[bans]` whose only wrapper is that dependency.
- RUSTSEC-2026-0440 is ignored on that ground, since 2026-11-16. `astral-tokio-tar` 0.6, whose maintainers ended it on 2026-11-03, comes with testcontainers 0.28.0, the latest release, and no release of testcontainers takes another tar crate. It builds the archives of the files a test copies into a container, which no test does, and only test builds take it. `deny.toml` bans it with testcontainers as its only wrapper, so the ignore covers no other crate that would take it (plan, refinement 14).
- sqlx-cli is a tool, not a member, so `deny.toml` never sees its dependencies. It is built from the lock file it was published with, which cargo 1.100 uses whatever `--locked` says (plan, appendix A.8). The security audit of the commands found advisories in that lock: RUSTSEC-2026-0385, a timing flaw in ctutils 0.4.2; anyhow, event-listener, rustix and tokio marked unsound; backoff and instant unmaintained; and a yanked chacha20. None is reachable on the connections sqlx-cli makes, to the throwaway database and to the local one, so they are accepted. Whenever the pinned version of sqlx-cli changes, the lock of the new version is audited with `cargo deny check` before it lands.

## Consequences

- Cost: Docker is required. `just db-up`, `just db-prepare`, `just db-check`, `just test-int` and `just test-cov`, and so `just verify` and the `pre-push` hook, need it running; `just test` does not. A developer without Docker running cannot push.
- Cost: `just verify` and CI's `checks` job compile the workspace once more, for `db-check`. CI builds sqlx-cli from its crates.io source on every run, as it does cargo-hakari, since install-action has no manifest for it. The `checks` and `test` jobs each pull the Postgres image in a step of their own, bounded to 5 minutes, through `mirror.gcr.io`, set as the runner's registry mirror because Docker Hub limits the pulls of an anonymous address; Docker falls back to Docker Hub when the mirror fails or lacks the image.
- Cost: a container per test makes the `int_` and `e2e_` tiers slower as tests are added, since each test starts Postgres from nothing. If that threatens `just test-int` or the 10 minutes of ADR-0014, a container shared across a run, through one of the experimental features, is the next step, and the owner decides (spec §17).
- Cost: every changed query and every new migration is followed by `just db-prepare` and a commit of `.sqlx/`; `just db-check` fails until then.
- Cost: a migration cannot be undone by a script. A mistake that reached a database takes a new migration, and a breaking change takes two releases.
- Cost: no statement is logged at any level unless it is slow, `debug` included.
- Cost: a statement on a serving pool that needs more than 5 seconds is cancelled by Postgres, with SQLSTATE 57014.
- With a replica, an instance can open twice `API_DATABASE_POOL_SIZE` connections, one pool's worth to each database.
- Pools built lazily move a wrong password from a start-up failure to a readiness failure: the instance starts and never becomes ready, its probe naming the class of failure. `just dev` serves while the local database is down, with readiness at 503.
- A `.env` that dotenvy cannot parse fails every build, offline too: the query macros parse every `.env` from the crate's folder up to `/`, above the repository included, before they look at `SQLX_OFFLINE`, and the error names the file but not the line (plan, refinement 2). The value rule of `.env.example` keeps that file readable; `CONTRIBUTING.md` says what to do.
- The history table's name is written twice, in `migrate.rs` and in `sqlx.toml`, and only a test keeps the two equal.
- The testkit reads the image from `compose.yaml` with a parser of its own: the `postgres` service names its image on one `image:` line.
- Only the selection of `just build` is the release build: a plain `cargo build --workspace --release` builds the testkit's features into the binaries.
- Limit: the URL rule covers the five parts it names. sqlx still reads the `PG*` variables itself, past `Environment::from_process` (ADR-0016), for the settings that no URL of ours carries, such as `PGSSLMODE` and `PGOPTIONS`; no environment of ours sets them. TLS to the database comes with BAR-12, which decides what a URL may carry for it.
- Limit: `barras-api migrate` applies a pending migration older than the newest applied one, out of order, as sqlx does, and nothing refuses it: a branch merged late with an old version runs after newer migrations. A `-- no-transaction` migration that fails can leave part of its work behind, such as an invalid index, which the next run meets again.
- Limit: the seed transaction is checked, not enforced. A file that ends it with `COMMIT` or `ROLLBACK` is named, but what it and the files before it did may stay. A file that leaves the connection mid-protocol, as `COPY … FROM stdin` does, fails without that check and closes the connection. `just db-reset` starts again.
- Limit: the guard's last word is a setting, not a proof. A server that started with `cluster_name = 'barras-local'` passes wherever it runs once a port of this machine reaches it, such as the Postgres of another developer's `compose.yaml` behind `ssh -L`.
- Limit: a test killed with SIGKILL leaves its container running until it is removed by its label.
- The ignore of RUSTSEC-2026-0440 stays until testcontainers drops `astral-tokio-tar`; the check fails then, until the ignore and its ban go.
- Limit: no check reads sqlx-cli's lock between two changes of its version. An advisory published against one of its crates blocks nothing; only the audit at the next version change, or a reader of the advisory, finds it.
- Left to BAR-15, the first repository: mapping a database failure to a module error, with the rule of spec §6.4, that an unreachable database, a pool with no free connection and a cancelled statement are `ErrorKind::Unavailable`. A repository's error never carries sqlx's text into an answer or a log line, since a database error's detail can quote a value bound to a statement (plan, refinement 6). The repository holds no seed file until BAR-15 adds the demo user, so `just db-reset` applies none.
- ADR-0007 keeps its Decision: this record says how it is built. ADR-0016, ADR-0024 and ADR-0027 point here.
