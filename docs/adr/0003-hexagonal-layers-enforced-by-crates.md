# ADR-0003: Hexagonal layers enforced by crates

Status: Accepted
Date: 2026-11-15

## Context

- The owner decided on a hexagonal backend, and the spec sets a goal that architecture rules are enforced by the compiler and by checks, not by convention (hexagonal: Decided, section 1; goal 8: section 2).
- Splitting the code into many small crates also lets Cargo compile in parallel, which serves the fast feedback loop.
- The two wiring rules below are **Calls** of the designer, not confirmed by the owner: ports are `Arc<dyn Port>`, and async ports use `async-trait`.

## Decision

- One crate per bounded context under `crates/modules/<module>`, holding domain, ports and use cases.
- A module crate depends only on `kernel`, small pure crates and `async-trait`. Never on another module, an adapter, axum, sqlx, reqwest, redis, tokio's runtime or serde_json.
- An adapter depends on `kernel`, the module crates and its own technology, never on another adapter. Only `apps/*` meet a concrete adapter and a port.
- A module that needs another module declares its own port. The composition root supplies a bridge that calls the other module's use case.
- Use cases hold `Arc<dyn Port>`, and `main.rs` builds every adapter and use case by hand. The builder may use native async traits if they work behind `dyn` on the pinned toolchain.

## Consequences

- Cost: a bridge and a port for each cross-module need, and hand-written wiring. Dynamic dispatch costs nothing measurable next to a database call.
- Cargo rejects an import that `Cargo.toml` does not list. `cargo deny check bans` restricts who may depend on sqlx, axum, reqwest and redis.
- `cargo xtask check-arch` fails on any dependency outside a crate's allowlist. Both run in `just check-arch`.
