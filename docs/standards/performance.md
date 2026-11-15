# Performance

Source: [ADR-0014](../adr/0014-performance-budgets.md). Change the ADR and this file in the same pull request; if they disagree, the ADR wins.

## Budgets

Starting budgets. **Call**; the first load test tunes them through an ADR.

| Budget | Target |
| --- | --- |
| `GET /api/v1/dashboard` server time | p95 ≤ 80 ms, p99 ≤ 200 ms |
| Any other read | p95 ≤ 150 ms |
| Any write | p95 ≤ 200 ms |
| Response body | ≤ 64 KB compressed |
| Tick from source to phone | p95 ≤ 1 s |
| Incremental `cargo check` after a one-line change in a module | ≤ 5 s |
| `just test` (unit) | ≤ 30 s |
| `just verify`, and CI | ≤ 10 min |

## Rules

- Home loads in one request: `GET /api/v1/dashboard` returns everything the screen needs.
- One query per read model, or an explicit batch. Never one query per row.
- Nothing builds a multi-megabyte JSON in a request. Exports stream.
- Release builds use thin LTO and one codegen unit. The allocator is chosen by benchmark.
- Build speed: many small crates so Cargo compiles in parallel; versions pinned once in the workspace; cargo-hakari to unify features; minimal feature flags; sccache and cargo-chef in CI; cargo-nextest. Cranelift is not used because it needs a nightly toolchain.
- Hot paths have criterion benchmarks. A nightly load test checks the budgets.

## Sizing for launch

**Call**: orders of magnitude, refined in the markets spec.

- **Users:** 10,000 registered and 1,000 concurrent websocket connections. The load test checks the budgets at 200 requests per second on the dashboard. One API node must hold 10,000 connections.
- **Instruments:** a curated catalog, not everything an exchange lists. About 50 crypto instruments (the largest Binance USDT pairs plus Buda's COP pairs) and the Colombian listings (about 57).
- **History kept:** 1-minute bars for 30 days, hourly bars for 2 years, daily bars without limit. Back-filled from the source where the source allows it.
