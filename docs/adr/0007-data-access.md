# ADR-0007: Data access with sqlx and hand-written SQL

Status: Accepted
Date: 2026-11-15

## Context

- Performance is the most important quality (Decided, section 1), and the dashboard must answer within its budget (ADR-0014).
- The spec makes these a set of **Calls** of the designer, not confirmed by the owner.
- A query per row, an offset scan or a multi-megabyte result would break the read path.

## Decision

- Use sqlx 0.9 with hand-written SQL, checked at compile time in offline mode. No ORM.
- Keep two pools: primary for writes and for reads that follow a write, and a read pool for read models (a replica when one exists).
- Read models aggregate in SQL. Lists use cursor pagination, never `OFFSET`, with at most 50 items per page.
- The schema changes only through migrations. SQL that looks odd, such as a partial index or a CHECK, carries a comment that says why.
- Each repository has an explicit mapper in both directions (see [module anatomy](../standards/module-anatomy.md)).

## Consequences

- Cost: more SQL to write and review than with an ORM, and the offline metadata in `.sqlx/` must be committed and refreshed (`just db-prepare`).
- CI checks the sqlx metadata, and `just check-structure` checks migration names. `just db-check` verifies migrations.
- Money, time and ids follow ADR-0008.
- Pointer: ADR-0030 builds this record. The migrations are forward-only, embedded in `barras-adapter-postgres` and applied by `barras-api migrate`, the deploy step, which applies nothing when its history holds a migration recorded as failed, changed since it ran, or missing from the binary; sqlx-cli only creates them and shows their status. `just db-check` applies every migration to a throwaway database and fails when `.sqlx/` is stale; `just verify` and CI's `checks` job run it, and `just db-prepare` rewrites `.sqlx/` against the same kind of throwaway database.
