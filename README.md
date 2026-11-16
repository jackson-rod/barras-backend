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
| Postgres | 18, from BAR-4 | The database |
| Redis | 8, from BAR-5 | Live ticks, latest-quote cache, replay buffer, rate limits |
| Docker | from BAR-4 | Throwaway containers for integration tests: Postgres from BAR-4, Redis from BAR-5 |

- Install rustup, just and lefthook yourself. On macOS with Homebrew: `brew install rustup just lefthook`.
- Homebrew's rustup is keg-only: `cargo` is not on your `PATH` until you add `$(brew --prefix rustup)/bin` to it. Tools installed by `cargo install` (cargo-nextest, cargo-deny, cargo-hakari, cargo-llvm-cov, watchexec) live in `$HOME/.cargo/bin`, which must be on the `PATH` too.
- In a checkout you already have, run `just setup` again whenever the `setup` recipe changes: `just verify`, and so the `pre-push` hook, needs every tool it installs.
- An editor's Rust support may want the source of the standard library. `rustup component add rust-src`, run inside the repository, adds it to the pinned toolchain; nothing else needs it, so `rust-toolchain.toml` leaves it out.

## First run

```bash
git clone https://github.com/jackson-rod/barras-backend.git  # get the code
cd barras-backend                                           # every command runs from the repository root
git switch develop                                          # the integration branch: every work branch starts here
just setup                                                  # installs the pinned Rust toolchain, the cargo tools and the git hooks
just verify                                                 # every check, the tests under coverage and the release build; must end without error
cp .env.example .env                                        # the variables the binaries read, with values for this machine
just dev                                                    # runs barras-api, and restarts it when the code changes
```

The first `just setup` compiles the cargo tools from source and takes several minutes. `just verify` needs the network, for cargo-deny's advisory database.

With the values of `.env.example`, `barras-api` listens on two addresses:

| Address | What answers |
| --- | --- |
| `http://127.0.0.1:8080/api/v1` | The public API. `GET /api/v1/health/live` answers while the process runs; `GET /api/v1/health` answers whether it can take traffic |
| `http://127.0.0.1:9080` | The internal port, for operators: `GET /health`, the health of each dependency, and `GET /metrics`, the Prometheus metrics |

`just dev-ingest` runs `barras-ingest`, whose internal port, `http://127.0.0.1:9081`, answers the same `GET /health` and `GET /metrics`. A binary that misses a variable, or reads one it cannot use, names each such variable and does not start; `just dev` stops before it builds anything when `.env` is missing.

## Everyday commands

| Command | What it does |
| --- | --- |
| `just verify` | The checks, the tests under coverage, the doctests and the release build that CI runs; run it before you push (the `pre-push` hook runs it too) |
| `just test` | The tests that need nothing outside the process, then the doctests; no Docker |
| `just test-int` | The `e2e_` and `int_` test targets: a whole binary, in process or as a real process; Docker from BAR-4 |
| `just test-cov` | Every test once under coverage, then the coverage thresholds |
| `just fmt` | Formats the code |
| `just lint` | clippy; any warning fails, cargo's own too |
| `just check-arch` | The dependency rule between crates |
| `just check-structure` | ADRs, module crates, migration names |
| `just hakari` | Regenerates `crates/workspace-hack`; run it after any dependency or feature change |
| `just openapi` | Rewrites `openapi.json`; run it after any route change |
| `just dev` / `just dev-ingest` | Runs `barras-api` / `barras-ingest` with the variables of `.env`, and restarts it when the code changes |

`just` lists every recipe, and [CLAUDE.md](CLAUDE.md#commands) describes each one.

## Branches, pull requests and CI

- Work happens on a branch from `develop` named after its ticket. The pull request targets `develop`, and the owner merges it. `main` and `develop` are protected. The rules are in [CLAUDE.md](CLAUDE.md#git-and-pull-requests); the routine is in [CONTRIBUTING.md](CONTRIBUTING.md).
- CI (`.github/workflows/ci.yml`) runs three jobs in parallel on every pull request, again when its title or description is edited, and on every push to `develop` and `main`. All three must pass before a merge. On a pull request a newer run cancels the older one; on `develop` and `main` it does not.

| Job | What it runs, in order |
| --- | --- |
| `checks` | On a pull request, the branch name (`cargo xtask check-branch`) and every commit message of the branch (`cargo xtask check-commits`). Then `just check-arch check-structure`; `just hakari-check`, which also fails when the generated `crates/workspace-hack` would change what a release build compiles; `just fmt-check` and `just deny`. None of them compiles a member, so no build script of the change runs before they have read the workspace. On a pull request, its title and description (`cargo xtask check-pr`). Last, `just lint` and `just openapi-check`, the latter bounded to 10 minutes |
| `test` | `just test-cov`, then `just test-doc`, bounded to 5 minutes |
| `build` | `just build` |

- Every check runs even when one before it failed, so one finding never hides another.
- Every job ends by failing when a step changed a tracked file or wrote any file outside `target/`, ignored ones included, such as `.env` or `lcov.info`. This catches mistakes, not a hostile build script, which can undo its writes first.
- Every action is pinned to a commit: `actions/checkout` v7.0.1, `actions-rust-lang/setup-rust-toolchain` v2.0.1, `mozilla-actions/sccache-action` v0.0.11, which installs sccache v0.19.0, and `taiki-e/install-action` v2.90.5. The toolchain comes from `rust-toolchain.toml`.
- `taiki-e/install-action` installs just 1.59.0, cargo-deny 0.20.2, cargo-nextest 0.9.152 and cargo-llvm-cov 0.9.1, each checked against the checksum in its manifest. `fallback: none` makes a tool or a version missing from that manifest fail the step, instead of being downloaded with no checksum. cargo-hakari 0.9.39, which the manifest lacks, is built from its crates.io source with `cargo install --locked`.
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
| `crates/testkit/` | Test-only helpers |
| `crates/workspace-hack/` | Generated by cargo-hakari; only its `[lints.cargo]` table is written by hand |
| `crates/xtask/` | The repository checks behind `cargo xtask` |
| `openapi.json` | The OpenAPI document of the public API, generated by `just openapi`: the contract with the app |
| `.env.example` | Every variable the binaries read, with a value that works on a developer machine |
| `justfile` | Every command |
| `lefthook.yml` | The git hooks |
| `Cargo.toml` | The workspace: members, versions pinned once, lints, release profile |
| `rust-toolchain.toml` | The pinned Rust toolchain |
| `clippy.toml` | clippy settings: test exemptions, disallowed methods |
| `deny.toml` | cargo-deny policy: advisories, licences, sources, layer bans |
| `.cargo/config.toml` | The `cargo xtask` alias |
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
| Database | Postgres 18; sqlx 0.9 with hand-written SQL | [ADR-0007](docs/adr/0007-data-access.md), [spec §6.1](docs/specs/2026-11-15-barras-foundation-design.md) |
| Live data | Redis 8 | [ADR-0004](docs/adr/0004-two-deployables-one-workspace.md) |
| Contract with the app | `openapi.json` generated with utoipa by `barras-api openapi` | [ADR-0002](docs/adr/0002-two-repositories-openapi-contract.md), [ADR-0028](docs/adr/0028-openapi-pipeline.md) |
| Logging | `tracing`, JSON output | [ADR-0015](docs/adr/0015-observability.md) |
| Metrics | `prometheus`, in a registry each binary creates | [ADR-0029](docs/adr/0029-http-contract-details.md) |
| Tooling | just, lefthook, cargo-nextest, cargo-deny, cargo-hakari, cargo-llvm-cov, watchexec, `cargo xtask`, GitHub Actions | [spec §6.14](docs/specs/2026-11-15-barras-foundation-design.md), [ADR-0024](docs/adr/0024-enforcement-of-the-layer-rule.md), [ADR-0027](docs/adr/0027-test-tiers-and-coverage.md) |

Each part arrives with the first ticket that needs it; the [work plan](docs/backlog/work-plan.md) gives their order.
