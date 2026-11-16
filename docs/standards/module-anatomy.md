# Module anatomy

Source: [ADR-0003](../adr/0003-hexagonal-layers-enforced-by-crates.md), [ADR-0006](../adr/0006-errors-as-typed-results.md) and [ADR-0024](../adr/0024-enforcement-of-the-layer-rule.md). Change the ADR and this file in the same pull request; if they disagree, the ADR wins.

This restates the foundation design, sections 6.3 to 6.6. Where this file and `code-standard.md` disagree on structure or naming, this file wins.

## Naming

A module's package is `barras-<module>` and its folder is `crates/modules/<module>`, in snake_case.

## Module tree

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

## Adapter mirror paths

Adapters mirror the modules:

- `crates/adapters/postgres/src/<module>/<name>_repository.rs`, with an explicit mapper in both directions.
- `crates/adapters/http/src/<module>/`, with `routes.rs` and `dto/<action>.rs`.

A handler is thin: validate the DTO, convert it into the use case's input, call one use case, map the output.

## Dependency rule

The "May depend on" column is the design rule (ADR-0003). The "Enforced by" column names what a check enforces (ADR-0024); review enforces the rest.

| Crate | May depend on | Enforced by |
| --- | --- | --- |
| `crates/kernel` | `std` and the small pure crates `thiserror`, `uuid`, `time`, `rust_decimal` | `check-arch`: normal and build dependencies only from that list; dev dependencies from it plus `tokio`, `mockall`, `insta`; each only from crates.io (`kernel-allowlist`, `kernel-dev-allowlist`) |
| `crates/modules/<m>` | `barras-kernel`, the same pure crates and `async-trait`. Never another module, an adapter, axum, sqlx, reqwest, redis, tokio's runtime or serde_json | `check-arch`: normal and build dependencies only from that list; dev dependencies from it plus `tokio`, `mockall`, `insta`; `barras-kernel` only as the workspace crate, every other name only from crates.io (`module-allowlist`, `module-dev-allowlist`) |
| `crates/adapters/*` | `barras-kernel`, the module crates and its own technology. Never another adapter | `check-arch`: no other adapter, whatever its source (`adapter-isolation`). cargo-deny bans: a technology crate only in its own adapter. Any other library is left to review |
| `apps/*` | everything. The only place where a concrete adapter meets a port | cargo-deny bans: no direct `sqlx`, `axum`, `reqwest` or `redis`. `check-arch`: `barras-testkit` only as a dev dependency (`testkit-dev-only`) |
| `crates/testkit` | test-only helpers: containers, fixtures, fakes | cargo-deny bans: `sqlx` and `redis` allowed directly, `axum` and `reqwest` not. `check-arch` sets no list; every crate that uses it takes it only as a dev dependency (`testkit-dev-only`) |
| `crates/xtask` | no workspace crate | `check-arch`: no workspace crate, whatever its source (`tool-isolation`) |

A module that needs something from another module declares its own port. The composition root supplies a bridge that calls the other module's use case. Example: `capital` declares `ExchangeRates`; the bridge calls `indicators`.

## Enforcement

- Cargo enforces most of this: a module crate cannot import what its `Cargo.toml` does not list.
- `just check-arch` runs two checks: `cargo deny check bans`, then `cargo xtask check-arch` ([ADR-0024](../adr/0024-enforcement-of-the-layer-rule.md)).
- `cargo xtask check-arch` reads `cargo metadata` and fails on each `check-arch` entry of the "Enforced by" column, and also on: a crate outside the folders of the table (`unknown-location`); a folder under `apps/`, `crates/modules/` or `crates/adapters/`, or `crates/kernel`, `crates/testkit` or `crates/xtask`, that holds a `Cargo.toml` but is not a workspace member (`not-a-member`); a `[patch]` or `[replace]` table in the root `Cargo.toml`, or a `[patch]` table in `.cargo/config.toml` (`no-patch`).
- An allowed name counts only from its expected source: `barras-kernel` only as the workspace crate, every other allowed name only from crates.io. A workspace crate is known by its package name, so a `git` copy of one is still caught.
- `cargo deny check bans` lets only these crates depend directly on a technology crate: `sqlx` only `barras-adapter-postgres` and `barras-testkit`; `axum` only `barras-adapter-http` and `barras-adapter-realtime`; `reqwest` only `barras-adapter-providers`; `redis` only `barras-adapter-redis` and `barras-testkit`. An app reaches them through its adapter.

## Reject on sight

- A framework type in a module crate.
- A database row type in a use case signature.
- A handler calling a repository.
- A module depending on another module.
- `unwrap`, `expect` or `panic!` outside tests.

## Ports and wiring

**Call** (section 6.5).

- Ports are traits. Use cases hold `Arc<dyn Port>`; `main.rs` builds every adapter and use case by hand. Dynamic dispatch costs nothing measurable next to a database call and compiles faster than generics.
- Async port traits use `async-trait`. If native async traits are usable behind `dyn` on the pinned toolchain, the builder may use them instead.

## Errors

- Every use case and port returns `Result<T, <Module>Error>`. There are no exceptions and no `anyhow` outside `main.rs`.
- Each variant has a stable `SCREAMING_SNAKE_CASE` code and an `ErrorKind` (validation, unauthenticated, forbidden, not found, conflict, rate limited, unavailable, internal). The code is public API: renaming it is a breaking change.
- The domain does not know HTTP. Only the `http` adapter maps `ErrorKind` to a status.
- Never swallow an error. Handle it, wrap it with its source, or return it.
