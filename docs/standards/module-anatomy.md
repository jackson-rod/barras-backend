# Module anatomy

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

| Crate | May depend on |
| --- | --- |
| `kernel` | `std` and small pure crates (thiserror, uuid, time, rust_decimal) |
| `modules/<m>` | `kernel` and the same pure crates, plus `async-trait`. Never another module, an adapter, axum, sqlx, reqwest, redis, tokio's runtime or serde_json |
| `adapters/*` | `kernel`, the module crates, and its own technology. Never another adapter |
| `apps/*` | everything. The only place where a concrete adapter meets a port |

A module that needs something from another module declares its own port. The composition root supplies a bridge that calls the other module's use case. Example: `capital` declares `ExchangeRates`; the bridge calls `indicators`.

## Enforcement

- Cargo enforces most of this: a module crate cannot import what its `Cargo.toml` does not list.
- `cargo deny check bans` restricts which crates may depend on sqlx, axum, reqwest and redis.
- `cargo xtask check-arch` reads `cargo metadata` and fails on any dependency outside each crate's allowlist.

## Reject on sight

- A framework type in a module crate.
- A database row type in a use case signature.
- A handler calling a repository.
- A module depending on another module.
- `unwrap`, `expect` or `panic!` outside tests.

## Ports and wiring

- Ports are traits. Use cases hold `Arc<dyn Port>`; `main.rs` builds every adapter and use case by hand. Dynamic dispatch costs nothing measurable next to a database call and compiles faster than generics.
- Async port traits use `async-trait`. If native async traits are usable behind `dyn` on the pinned toolchain, the builder may use them instead.

## Errors

- Every use case and port returns `Result<T, <Module>Error>`. There are no exceptions and no `anyhow` outside `main.rs`.
- Each variant has a stable `SCREAMING_SNAKE_CASE` code and an `ErrorKind` (validation, unauthenticated, forbidden, not found, conflict, rate limited, unavailable, internal). The code is public API: renaming it is a breaking change.
- The domain does not know HTTP. Only the `http` adapter maps `ErrorKind` to a status.
- Never swallow an error. Handle it, wrap it with its source, or return it.
