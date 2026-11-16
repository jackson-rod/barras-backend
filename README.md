# barras-backend

Barras is a mobile app for people in Colombia who invest: their money and the market in one place. This repository is its backend, one Rust Cargo workspace whose hexagonal layers are separate crates. It builds two binaries: `barras-api` serves the app over HTTP and websockets, and `barras-ingest` reads the outside data sources. Today it holds the workspace skeleton, the repository checks, the git hooks and CI; no product code exists yet. The rules every contributor follows are in [CLAUDE.md](CLAUDE.md), and how we work day to day is in [CONTRIBUTING.md](CONTRIBUTING.md).

> **Keep this file current.** A new developer must be able to run the project on the first try from this README. Whenever a step, a service, a port or a command changes, change this README in the same pull request.

## What you need

| Tool | Version | Why |
| --- | --- | --- |
| Rust, through rustup | 1.100.0 with rustfmt and clippy, pinned by `rust-toolchain.toml` | Compiles, formats and lints the workspace |
| just | Set up with 1.59.0; a newer one is expected to work | Runs every command in the `justfile` |
| lefthook | Set up with 2.2.2; a newer one is expected to work | Runs the git hooks in `lefthook.yml` |
| cargo-nextest | 0.9.152, installed by `just setup` | Runs the tests (`just test`) |
| cargo-deny | 0.20.2, installed by `just setup` | Checks advisories, licences, sources and the layer bans (`just deny`, `just check-arch`) |
| Postgres | 18, from BAR-4 | The database |
| Redis | 8, from BAR-4 | Live ticks, latest-quote cache, replay buffer, rate limits |
| Docker | from BAR-4 | Throwaway Postgres and Redis containers for integration tests |

- Install rustup, just and lefthook yourself. On macOS with Homebrew: `brew install rustup just lefthook`.
- Homebrew's rustup is keg-only: `cargo` is not on your `PATH` until you add `$(brew --prefix rustup)/bin` to it. Tools installed by `cargo install` (cargo-nextest, cargo-deny) live in `$HOME/.cargo/bin`, which must be on the `PATH` too.
- Today no database, Redis, Docker or `.env` file is needed.

## First run

```bash
git clone https://github.com/jackson-rod/barras-backend.git  # get the code
cd barras-backend                                           # every command runs from the repository root
git switch develop                                          # the integration branch: every work branch starts here
just setup                                                  # installs Rust 1.100.0 as rust-toolchain.toml pins it, cargo-nextest and cargo-deny, then the git hooks
just verify                                                 # format, lint, deny, architecture, structure, tests, release build; must end without error
```

The first `just setup` compiles cargo-nextest and cargo-deny from source and takes a few minutes.

## Everyday commands

| Command | What it does |
| --- | --- |
| `just verify` | The checks, tests and release build that CI runs; run it before you push (the `pre-push` hook runs it too) |
| `just test` | Unit tests with cargo-nextest; no Docker |
| `just fmt` | Formats the code |
| `just lint` | clippy; any warning fails |
| `just check-arch` | The dependency rule between crates |
| `just check-structure` | ADRs, module crates, migration names |
| `just dev` / `just dev-ingest` | Runs `barras-api` / `barras-ingest` |

`just` lists every recipe, and [CLAUDE.md](CLAUDE.md#commands) describes each one.

## Branches, pull requests and CI

- Work happens on a branch from `develop` named after its ticket. The pull request targets `develop`, and the owner merges it. `main` and `develop` are protected. The rules are in [CLAUDE.md](CLAUDE.md#git-and-pull-requests); the routine is in [CONTRIBUTING.md](CONTRIBUTING.md).
- CI (`.github/workflows/ci.yml`) runs three jobs in parallel on every pull request and on every push to `develop` and `main`. All three must pass before a merge.

| Check | What it runs |
| --- | --- |
| `checks` | On a pull request, the branch name (`cargo xtask check-branch`) and every commit message of the branch (`cargo xtask check-commits`). Always `just fmt-check`, `just lint`, `just deny`, `just check-arch` and `just check-structure` |
| `test` | `just test` |
| `build` | `just build` |

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
| `crates/adapters/` | Adapters: `postgres`, `http`, `realtime`, `redis`, `providers` |
| `crates/testkit/` | Test-only helpers |
| `crates/xtask/` | The repository checks behind `cargo xtask` |
| `justfile` | Every command |
| `lefthook.yml` | The git hooks |
| `Cargo.toml` | The workspace: members, versions pinned once, lints, release profile |
| `rust-toolchain.toml` | The pinned Rust toolchain |
| `clippy.toml` | clippy settings: test exemptions, disallowed methods |
| `deny.toml` | cargo-deny policy: advisories, licences, sources, layer bans |
| `.cargo/config.toml` | The `cargo xtask` alias |
| `.github/workflows/ci.yml` | The CI workflow |
| `.github/pull_request_template.md` | The pull request template |
| [`LICENSE`](LICENSE) | MIT licence |

Module crates go in `crates/modules/<module>/`; none exists yet.

## Stack

| Part | Choice | Decided in |
| --- | --- | --- |
| Language | Rust 1.100.0, edition 2024 | `rust-toolchain.toml`, `Cargo.toml` |
| Shape | One Cargo workspace; hexagonal layers as crates; two binaries | [ADR-0003](docs/adr/0003-hexagonal-layers-enforced-by-crates.md), [ADR-0004](docs/adr/0004-two-deployables-one-workspace.md) |
| HTTP and websockets | axum | [ADR-0004](docs/adr/0004-two-deployables-one-workspace.md) |
| Database | Postgres 18; sqlx 0.9 with hand-written SQL | [ADR-0007](docs/adr/0007-data-access.md), [spec §6.1](docs/specs/2026-11-15-barras-foundation-design.md) |
| Live data | Redis 8 | [ADR-0004](docs/adr/0004-two-deployables-one-workspace.md) |
| Contract with the app | `openapi.json` generated with utoipa | [ADR-0002](docs/adr/0002-two-repositories-openapi-contract.md) |
| Logging | `tracing`, JSON output | [ADR-0015](docs/adr/0015-observability.md) |
| Tooling | just, lefthook, cargo-nextest, cargo-deny, `cargo xtask`, GitHub Actions | [spec §6.14](docs/specs/2026-11-15-barras-foundation-design.md) |

Only the toolchain and the tooling are in use today; the rest arrives with the tickets that need it.
