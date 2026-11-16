# Testing

Source: [ADR-0017](../adr/0017-definition-of-done.md) and [ADR-0027](../adr/0027-test-tiers-and-coverage.md). Change the ADR and this file in the same pull request; if they disagree, the ADR wins.

This restates the foundation design, sections 9 and 6.14, and the BAR-3 spec, section 9.

## Tests per layer

A change is done only when the same pull request contains tests for every layer it touches:

- Domain types are tested directly.
- Use cases are tested with fake ports.
- Mappers round trip in both directions.
- Routes are tested through e2e against the real app.
- Every adapter to an outside system has a contract test.
- Errors are asserted by `code`, never by message.

## Tiers

A test target's file name decides its tier, so a new test has nothing to register (ADR-0027).

| Tier | Where | Runs in | Needs |
| --- | --- | --- | --- |
| Unit and in-process | `#[cfg(test)]` modules, and every test target under `tests/` not named below | `just test` | Nothing outside the process |
| e2e | `tests/e2e_*.rs`: a whole binary, its app in-process through the HTTP adapter's `TestClient`, or its real process through the testkit's `BinaryCommand` | `just test-int` | From BAR-4, Docker |
| int | `tests/int_*.rs`: one adapter against a real outside system, from BAR-4 | `just test-int` | Docker, in throwaway Postgres 18 and Redis 8 containers |

- `just test` runs the doctests after the tests; nextest does not run them.
- `just test-cov` runs every test of both tiers once, under coverage, then the thresholds. `just verify` and CI run it, and the doctests apart.
- An adapter's tests that need its `testing` feature live in `src/tests/`, declared `#[cfg(test)] mod tests;`. Under `tests/` they would need a dependency of the adapter on itself, which `check-arch` refuses.
- A test still running after 30 seconds is reported as slow; after two minutes it is killed and fails under its own name (`.config/nextest.toml`).

## Coverage

**Call** (section 9; BAR-3 spec §9.4).

- At least 95% of the production lines of `crates/kernel` and of each crate under `crates/modules/`, each on its own.
- At least 85% of the production lines of the measured crates together.

How it is measured (ADR-0027):

- `just test-cov` runs every test once with cargo-llvm-cov, writing a fresh report, then `cargo xtask check-coverage` reads it with the sources. It prints each crate's figure, cut and not rounded, and fails under either bar, or when the kernel or a module crate has no figure.
- Production lines leave out every item marked exactly `#[cfg(test)]`, at any depth, with its attributes and doc comments. Test support under `#[cfg(any(test, feature = "…"))]` counts as production.
- Measured: the files under the `src/` of the kernel, the modules, the adapters, `crates/bootstrap`, `crates/xtask` and the apps. A binary that a process test starts is measured too.
- Not measured: `barras-testkit`, the generated `workspace-hack`, a crate's `tests/`, `benches/`, `examples/` and `build.rs`, and the doctests, which a stable toolchain cannot instrument.
- A source that holds a function body outside test code must appear in the report, so a crate whose tests never run fails.
- Test code is inline under `#[cfg(test)]`, in a `tests.rs` file, or under a `tests/` folder in `src/`. `check-coverage` refuses every shape that would hide code from the measure; `CLAUDE.md` lists them and ADR-0027 says why.
