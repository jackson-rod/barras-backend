# barras-backend

Barras is a mobile app for people in Colombia who invest: their money and the market in one place. This repository is its backend, one Rust Cargo workspace whose hexagonal layers are separate crates. It builds two binaries: `barras-api` serves the app over HTTP and websockets, and `barras-ingest` reads the outside data sources. The [work plan](docs/backlog/work-plan.md) lists the tickets that build it, in order. The rules every contributor follows are in [CLAUDE.md](CLAUDE.md), and how we work day to day is in [CONTRIBUTING.md](CONTRIBUTING.md).

> **Keep this file current.** A new developer must be able to run the project on the first try from this README. Whenever a step, a service, a port or a command changes, change this README in the same pull request.

## What you need

| Tool | Version | Why |
| --- | --- | --- |
| Rust, through rustup | Rust 1.100.0 with rustfmt, clippy and llvm-tools, pinned by `rust-toolchain.toml`; rustup 1.28.0 or newer | Compiles, formats, lints and measures the workspace. `just setup` runs `rustup toolchain install` with no argument, which installs the pinned toolchain from rustup 1.28.0 on |
| just | Set up with 1.59.0; a newer one is expected to work | Runs every command in the `justfile` |
| lefthook | Set up with 2.2.2; a newer one is expected to work | Runs the git hooks in `lefthook.yml` |
| cargo-nextest | 0.9.152, installed by `just setup` | Runs the tests |
| cargo-deny | 0.20.2, installed by `just setup` | Checks advisories, licences, sources and the layer bans (`just deny`, `just check-arch`) |
| cargo-hakari | 0.9.39, installed by `just setup` | Generates `crates/workspace-hack` (`just hakari`, `just hakari-check`) |
| cargo-llvm-cov | 0.9.1, installed by `just setup` | Measures coverage (`just test-cov`) |
| watchexec | 2.8.0, installed by `just setup` | Restarts a binary when the code changes (`just dev`, `just dev-ingest`) |
| sqlx-cli | 0.9.0, installed by `just setup`, with Postgres support and its `sqlx-toml` feature | Creates migrations and shows their status (`just db-migrate`, `just db-status`); applies them to a throwaway database and writes or checks `.sqlx/` (`just db-prepare`, `just db-check`) |
| Docker | Required, with `docker compose`; Docker Desktop has both | Runs the local Postgres of `compose.yaml` (`just db-up`), the throwaway Postgres of `just db-prepare` and `just db-check`, and the containers of the `int_` and `e2e_` tests; Redis from BAR-5 |
| Postgres | 18, run by Docker from `compose.yaml`: nothing to install | The database |
| Redis | 8, from BAR-5 | Live ticks, latest-quote cache, replay buffer, rate limits |

- Install rustup, just, lefthook and Docker yourself. On macOS with Homebrew: `brew install rustup just lefthook`, and Docker Desktop from Docker's site.
- Homebrew's rustup is keg-only: `cargo` is not on your `PATH` until you add `$(brew --prefix rustup)/bin` to it. Tools installed by `cargo install` (cargo-nextest, cargo-deny, cargo-hakari, cargo-llvm-cov, watchexec, sqlx-cli) live in `$HOME/.cargo/bin`, which must be on the `PATH` too.
- In a checkout you already have, run `just setup` again whenever the `setup` recipe changes: `just verify`, and so the `pre-push` hook, needs every tool it installs. Since BAR-4 it installs sqlx-cli.
- An editor's Rust support may want the source of the standard library. `rustup component add rust-src`, run inside the repository, adds it to the pinned toolchain; nothing else needs it, so `rust-toolchain.toml` leaves it out.

## First run

```bash
git clone https://github.com/jackson-rod/barras-backend.git  # get the code
cd barras-backend                                           # every command runs from the repository root
git switch develop                                          # the integration branch: every work branch starts here
just setup                                                  # installs the pinned Rust toolchain, the cargo tools, sqlx-cli and the git hooks
just verify                                                 # every check, the tests under coverage and the release build; must end without error
cp .env.example .env                                        # the variables the binaries read, with values for this machine
just db-up                                                  # starts the local Postgres of compose.yaml, on 127.0.0.1:5433
just db-reset                                               # rebuilds it: every migration, then the seeds
just dev                                                    # runs barras-api, and restarts it when the code changes
```

The first `just setup` compiles the cargo tools from source and takes several minutes. `just verify` needs the network, for cargo-deny's advisory database, and Docker running, for the throwaway Postgres containers of `just db-check` and of the tests.

With the values of `.env.example`, `barras-api` listens on two addresses:

| Address | What answers |
| --- | --- |
| `http://127.0.0.1:8080/api/v1` | The public API. `GET /api/v1/health/live` answers while the process runs; `GET /api/v1/health` answers whether it can take traffic |
| `http://127.0.0.1:9080` | The internal port, for operators: `GET /health`, the health of each dependency, and `GET /metrics`, the Prometheus metrics |

`just dev-ingest` runs `barras-ingest`, whose internal port, `http://127.0.0.1:9081`, answers the same `GET /health` and `GET /metrics`. A binary that misses a variable, or reads one it cannot use, names each such variable and does not start; `just dev` stops before it builds anything when `.env` is missing.

## The local database

- The local database is the `postgres` service of `compose.yaml`: Postgres 18 in Docker, on `127.0.0.1:5433`, so that it never clashes with a Postgres of your own on 5432. Its user, password and database are all `barras`, as `.env.example` says.
- The routine: `just db-up` starts it and waits until it accepts connections; `just db-reset` drops the database and creates it again, with every migration and then the seeds; `just dev` serves against it. `just dev` serves while the database is down too: `GET /api/v1/health` then answers 503, and the internal `GET /health` says why.
- `just db-down` stops it, and its data stays in a Docker volume. It does not start on its own when Docker or the machine restarts: run `just db-up` again.
- A checkout from before the database: run `just setup` again, which installs sqlx-cli; add the three `API_DATABASE_*` variables of `.env.example` to your `.env`, since `barras-api` refuses to start without them; then `just db-up`.
- A Postgres of your own instead, such as a native one: point `API_DATABASE_URL` and `API_DATABASE_READ_URL` at it. `just db-seed` and `just db-reset` change a database only when `APP_ENV` is `local`, the database is on this machine, and its server started with `cluster_name = 'barras-local'`: add that line to its `postgresql.conf`, then restart it. A port of this machine can lead to another machine, so they refuse any other server.
- Seeds are local development data. There is none until BAR-15 adds a demo user, so `just db-reset` applies only the migrations.

## Everyday commands

| Command | What it does |
| --- | --- |
| `just verify` | The checks, the tests under coverage, the doctests and the release build that CI runs; run it before you push (the `pre-push` hook runs it too). Needs the network and Docker running |
| `just test` | The tests that need nothing outside the process, then the doctests; no Docker |
| `just test-int` | The `e2e_` and `int_` test targets: a whole binary, in process or as a real process, and the adapters against throwaway Postgres containers; needs Docker running |
| `just test-cov` | Every test once under coverage, then the coverage thresholds; needs Docker running |
| `just fmt` | Formats the code |
| `just lint` | clippy; any warning fails, cargo's own too |
| `just check-arch` | The dependency rule between crates |
| `just check-structure` | ADRs, module crates, and the names and versions of migrations and seeds |
| `just hakari` | Regenerates `crates/workspace-hack`; run it after any dependency or feature change |
| `just openapi` | Rewrites `openapi.json`; run it after any route change |
| `just dev` / `just dev-ingest` | Runs `barras-api` / `barras-ingest` with the variables of `.env`, and restarts it when the code changes |
| `just db-up` / `just db-down` | Starts the local Postgres and waits until it accepts connections / stops it, keeping its data |
| `just db-reset` | Rebuilds the database of `.env`: drop, create, every migration, the seeds; only a local database |
| `just db-seed` | Loads the seeds into the database of `.env`; only a local database |
| `just db-migrate <name>` | Creates a migration, `migrations/<timestamp>_<name>.sql`; the name is snake_case |
| `just db-deploy` | Applies the pending migrations to the database of `.env` |
| `just db-status` | Which migrations the database of `.env` has applied, which are pending, and which changed after they ran. It creates the history table, `public._sqlx_migrations`, in a database that lacks it |
| `just db-prepare` | Rewrites `.sqlx/` against a throwaway Postgres; run it after changing a query or adding a migration |
| `just db-check` | Fails when `.sqlx/` is stale, or when a migration does not apply to an empty database |

`just` lists every recipe, and [CLAUDE.md](CLAUDE.md#commands) describes each one.

## Branches, pull requests and CI

- Work happens on a branch from `develop` named after its ticket. The pull request targets `develop`, and the owner merges it. `main` and `develop` are protected. The rules are in [CLAUDE.md](CLAUDE.md#git-and-pull-requests); the routine is in [CONTRIBUTING.md](CONTRIBUTING.md).
- CI (`.github/workflows/ci.yml`) runs three jobs in parallel on every pull request, again when its title or description is edited, and on every push to `develop` and `main`. All three must pass before a merge. On a pull request a newer run cancels the older one; on `develop` and `main` it does not.

| Job | What it runs, in order |
| --- | --- |
| `checks` | Once the tools are installed, the Postgres image of `compose.yaml`, pulled in a step bounded to 5 minutes. On a pull request, the branch name (`cargo xtask check-branch`) and every commit message of the branch (`cargo xtask check-commits`). Then `just check-arch check-structure`; `just hakari-check`, which also fails when the generated `crates/workspace-hack` would change what a release build compiles; `just fmt-check` and `just deny`. None of them compiles a member, so no build script of the change runs before they have read the workspace. On a pull request, its title and description (`cargo xtask check-pr`). Last, `just lint`, then `just openapi-check` and `just db-check`, each bounded to 10 minutes |
| `test` | Once the tools are installed, the Postgres image of `compose.yaml`, pulled in a step bounded to 5 minutes, so that a slow pull is charged to no test. Then `just test-cov`, then `just test-doc`, bounded to 5 minutes |
| `build` | `just build` |

- Every check runs even when one before it failed, so one finding never hides another.
- Every job ends by failing when a step changed a tracked file or wrote any file outside `target/`, ignored ones included, such as `.env` or `lcov.info`. This catches mistakes, not a hostile build script, which can undo its writes first.
- Every action is pinned to a commit: `actions/checkout` v7.0.1, `actions-rust-lang/setup-rust-toolchain` v2.0.1, `mozilla-actions/sccache-action` v0.0.11, which installs sccache v0.19.0, and `taiki-e/install-action` v2.90.5. The toolchain comes from `rust-toolchain.toml`.
- `taiki-e/install-action` installs just 1.59.0, cargo-deny 0.20.2, cargo-nextest 0.9.152 and cargo-llvm-cov 0.9.1, each checked against the checksum in its manifest. `fallback: none` makes a tool or a version missing from that manifest fail the step, instead of being downloaded with no checksum. cargo-hakari 0.9.39 and sqlx-cli 0.9.0, which the manifest lacks, are built from their crates.io source with `cargo install --locked`, sqlx-cli with the features `just setup` gives it.
- Docker Hub limits the pulls of an anonymous address, and a runner shares its address. So the `checks` and `test` jobs first set `mirror.gcr.io`, Google's cache of Docker Hub, as the runner's registry mirror; Docker falls back to Docker Hub when the mirror fails or lacks the image.
- sccache caches the compiled crates in GitHub's cache. A pull request only reads it; pushes to `develop` and `main` write to it. The cache token stays in the environment of every later step, so a hostile build script could still write to the pull request's own cache: the run on `develop` after the merge is the authoritative result ([ADR-0027](docs/adr/0027-test-tiers-and-coverage.md)).
- CI compiles as `just verify` does: the toolchain action's own cache and its default of denying cargo warnings are off. Each job stops after 20 minutes, and the workflow's token may only read the repository's contents.

## Where things are

| Path | What it holds |
| --- | --- |
| [`CLAUDE.md`](CLAUDE.md) | The rules every contributor follows, human or agent |
| [`CONTRIBUTING.md`](CONTRIBUTING.md) | Tickets, specs, branches, commits and pull requests, day to day |
| [`docs/README.md`](docs/README.md) | The plan: start here for specs, plans, ADRs, standards, schema, analysis and the work plan |
| [`docs/adr/README.md`](docs/adr/README.md) | The index of architecture decision records |
| `apps/api/` | Binary `barras-api`: composition root only |
| `apps/ingest/` | Binary `barras-ingest`: composition root only |
| `crates/kernel/` | The shared kernel |
| `crates/bootstrap/` | The start-up both binaries share: configuration, logs, panic hook, runtime, shutdown signal, failures told on standard error |
| `crates/adapters/` | Adapters: `postgres`, `http`, `realtime`, `redis`, `providers` |
| `crates/testkit/` | Test-only helpers, among them a Postgres container per test |
| `crates/workspace-hack/` | Generated by cargo-hakari; only its `[lints.cargo]` table is written by hand |
| `crates/xtask/` | The repository checks behind `cargo xtask` |
| `migrations/` | The SQL migrations, forward-only, which `barras-api migrate` applies |
| `.sqlx/` | The offline query metadata, written by `just db-prepare` and committed |
| `openapi.json` | The OpenAPI document of the public API, generated by `just openapi`: the contract with the app |
| `compose.yaml` | The local Postgres, and the one place that names its image |
| `sqlx.toml` | sqlx-cli's name for the history table of the migrations |
| `.env.example` | Every variable the binaries read, with a value that works on a developer machine |
| `justfile` | Every command |
| `lefthook.yml` | The git hooks |
| `Cargo.toml` | The workspace: members, versions pinned once, lints, release profile |
| `rust-toolchain.toml` | The pinned Rust toolchain |
| `clippy.toml` | clippy settings: test exemptions, disallowed methods |
| `deny.toml` | cargo-deny policy: advisories, licences, sources, layer bans |
| `.cargo/config.toml` | The `cargo xtask` alias, and `SQLX_OFFLINE=true` for every cargo command |
| `.config/hakari.toml` | How cargo-hakari generates `crates/workspace-hack` |
| `.config/nextest.toml` | The time limit of a test |
| `.github/workflows/ci.yml` | The CI workflow |
| `.github/pull_request_template.md` | The pull request template |
| [`LICENSE`](LICENSE) | MIT licence |

Module crates go in `crates/modules/<module>/`, as [CLAUDE.md](CLAUDE.md#module-anatomy) describes.

## Stack

| Part | Choice | Decided in |
| --- | --- | --- |
| Language | Rust 1.100.0, edition 2024; builds and runs on Unix only | `rust-toolchain.toml`, `Cargo.toml`, [ADR-0026](docs/adr/0026-process-start-up-crate.md) |
| Shape | One Cargo workspace; hexagonal layers as crates; two binaries | [ADR-0003](docs/adr/0003-hexagonal-layers-enforced-by-crates.md), [ADR-0004](docs/adr/0004-two-deployables-one-workspace.md) |
| HTTP and websockets | axum | [ADR-0004](docs/adr/0004-two-deployables-one-workspace.md) |
| Validation | garde, behind the HTTP adapter's extractors | [ADR-0029](docs/adr/0029-http-contract-details.md) |
| Database | Postgres 18; sqlx 0.9 with hand-written SQL; forward-only migrations applied by `barras-api migrate`; the local database in Docker Compose | [ADR-0007](docs/adr/0007-data-access.md), [ADR-0030](docs/adr/0030-database-foundation.md), [spec §6.1](docs/specs/2026-11-15-barras-foundation-design.md) |
| Live data | Redis 8 | [ADR-0004](docs/adr/0004-two-deployables-one-workspace.md) |
| Contract with the app | `openapi.json` generated with utoipa by `barras-api openapi` | [ADR-0002](docs/adr/0002-two-repositories-openapi-contract.md), [ADR-0028](docs/adr/0028-openapi-pipeline.md) |
| Logging | `tracing`, JSON output | [ADR-0015](docs/adr/0015-observability.md) |
| Metrics | `prometheus`, in a registry each binary creates | [ADR-0029](docs/adr/0029-http-contract-details.md) |
| Tooling | just, lefthook, cargo-nextest, cargo-deny, cargo-hakari, cargo-llvm-cov, watchexec, sqlx-cli, Docker with testcontainers, `cargo xtask`, GitHub Actions | [spec §6.14](docs/specs/2026-11-15-barras-foundation-design.md), [ADR-0024](docs/adr/0024-enforcement-of-the-layer-rule.md), [ADR-0027](docs/adr/0027-test-tiers-and-coverage.md), [ADR-0030](docs/adr/0030-database-foundation.md) |

Each part arrives with the first ticket that needs it; the [work plan](docs/backlog/work-plan.md) gives their order.
