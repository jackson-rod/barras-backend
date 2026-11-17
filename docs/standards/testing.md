# Testing

Source: [ADR-0017](../adr/0017-definition-of-done.md), [ADR-0027](../adr/0027-test-tiers-and-coverage.md) and [ADR-0030](../adr/0030-database-foundation.md). Change the ADR and this file in the same pull request; if they disagree, the ADR wins.

This restates the foundation design, sections 9 and 6.14, the BAR-3 spec, section 9, and the BAR-4 spec, section 11.

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
| e2e | `tests/e2e_*.rs`: a whole binary, its app in-process through the HTTP adapter's `TestClient`, or its real process through the testkit's `BinaryCommand` | `just test-int` | Docker, for the process tests that give the binary a database |
| int | `tests/int_*.rs`: one adapter against a real outside system | `just test-int` | Docker: a throwaway Postgres 18 container per test; Redis 8 from BAR-5 |

- `just test` runs the doctests after the tests; nextest does not run them. It needs no Docker.
- `just test-cov` runs every test of both tiers once, under coverage, then the thresholds. `just verify` and CI run it, and the doctests apart.
- `just test-int` and `just test-cov`, and so `just verify` and the `pre-push` hook, need Docker running.
- An adapter's tests that need its `testing` feature live in `src/tests/`, declared `#[cfg(test)] mod tests;`. Under `tests/` they would need a dependency of the adapter on itself, which `check-arch` refuses.
- A test still running after 30 seconds is reported as slow; after two minutes it is killed and fails under its own name (`.config/nextest.toml`).

## Containers

- A test that needs a database starts its own container with `PostgresContainer::start()` of `barras-testkit`: Postgres 18 from the image of the `postgres` service of `compose.yaml`, with `fsync` off, since its data never outlives it, the `cluster_name` of the local database, its port published on `127.0.0.1` alone, and an empty database `barras` that no other test touches. The test applies the migrations it needs, and reaches the database through `url()`.
- One container per test, never one shared: nextest runs every test in a process of its own, and a container held in a `static` would never be removed.
- Dropping the container removes it, also while a failed test unwinds; it must be dropped inside the tokio runtime that started it. testcontainers' `watchdog` removes it when the test is stopped with SIGTERM, as nextest does at its time limit, or with SIGINT. Only SIGKILL leaves one behind: the label `barras-test=1` finds it, and `barras-throwaway=1` the throwaway database of `just db-prepare` and `just db-check`.
- Docker not running fails the test with a sentence that says so. No test skips itself: that would hide a failure.
- No test reaches the local database. Every URL a test uses comes from a container it started; the testkit reads no environment variable, apart from the coverage profile its process helper hands on; and the process tests hand each binary its whole environment.
- `LoopbackRelay` of the testkit reaches a container through a port of `127.0.0.1`, as `ssh -L` would, and `PostgresContainer::start_as_another_server()` starts one without the local `cluster_name`. Together they stand for a server on another machine behind a port of this one, which `seed` and `reset` must refuse.

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
