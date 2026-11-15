# ADR-0014: Performance budgets and the dashboard read path

Status: Accepted
Date: 2026-11-15

## Context

- The owner decided that performance is the most important quality: the product must be very fast, and so must the compile and feedback loop.
- A slow Home screen and a multi-megabyte JSON response are anti-goals.
- The budgets are a **Call** of the designer, not confirmed by the owner. The first load test tunes them through an ADR.

## Decision

- Server time: `GET /api/v1/dashboard` p95 at most 80 ms and p99 at most 200 ms; any other read p95 at most 150 ms; any write p95 at most 200 ms.
- Response body at most 64 KB compressed. A tick from source to phone p95 at most 1 s.
- Developer loop: incremental `cargo check` after a one-line module change at most 5 s; `just test` at most 30 s; `just verify` and CI at most 10 min.
- Home loads in one request: `GET /api/v1/dashboard` returns everything the screen needs.
- One query per read model, or an explicit batch, never one query per row. Nothing builds a multi-megabyte JSON in a request; exports stream.
- Release builds use thin LTO and one codegen unit.

## Consequences

- Cost: the dashboard read model is shaped for one screen, and build speed needs many small crates and extra tooling.
- Hot paths have criterion benchmarks, and a nightly load test checks the budgets.
- The allocator is chosen by benchmark.
