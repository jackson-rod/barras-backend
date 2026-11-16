# ADR-0024: Enforcement of the layer rule

Status: Proposed
Date: 2026-11-15

## Context

- ADR-0003 sets who may depend on whom and says checks enforce it. A name on a list proves little on its own: a `git` or `path` entry, another registry, or a `[patch]` table can put other code behind an allowed name, and a crate folder missing from `members` is never built or checked.
- Tests need a few crates that the lists of ADR-0003 leave out, such as a mocking library.
- Everything below is a **Call** of the designer. The table of spec §6.4 lets `apps/*` depend on everything, and the owner has not confirmed the stricter rule.

## Decision

- An allowed name counts only from its expected source: `barras-kernel` only as the workspace path dependency, every other allowed name only from crates.io. A workspace crate is known by its package name, whatever its source.
- The kernel's normal and build dependencies come only from `thiserror`, `uuid`, `time` and `rust_decimal`. A module's come only from `barras-kernel`, those four and `async-trait`.
- The dev dependencies of the kernel and of a module may also use `tokio`, `mockall` and `insta`.
- No `[patch]` or `[replace]` table in the root `Cargo.toml`, and no `[patch]` table in `.cargo/config.toml`. Needing one changes this rule through a new ADR.
- Every folder directly under `apps/`, `crates/modules/` and `crates/adapters/` that holds a `Cargo.toml`, and `crates/kernel`, `crates/testkit` and `crates/xtask`, is a workspace member.
- The cargo-deny bans let only these crates depend directly on a technology crate: `sqlx` only `barras-adapter-postgres` and `barras-testkit`; `axum` only `barras-adapter-http` and `barras-adapter-realtime`; `reqwest` only `barras-adapter-providers`; `redis` only `barras-adapter-redis` and `barras-testkit`. So `apps/*` reach these crates through the adapters, never directly.

## Consequences

- Cost: a crate from git or another registry, or a local patch, waits for an ADR, even for a quick fix.
- An app that needs a type of one of the four technology crates gets it through its adapter, which re-exports what the composition root needs.
- `cargo xtask check-arch` enforces every rule above but the last, and names the rule each failure breaks, such as `module-allowlist`, `no-patch` or `not-a-member`. `cargo deny check bans` enforces the last. `just check-arch` runs both.
