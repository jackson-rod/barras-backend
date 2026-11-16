# ADR-0026: The process start-up crate

Status: Accepted
Date: 2026-11-16

## Context

- `barras-api` and `barras-ingest` start the same way: they read the environment into a typed configuration (ADR-0016), start the logs (ADR-0015) and wait for the signal that stops them. No row of the dependency table of ADR-0003 fits that code: it implements no port, so it is no adapter, and the kernel does no I/O.
- The owner decided on 2026-11-15, in the review of the BAR-3 spec, that this code lives in a crate of its own with its own row (BAR-3 spec §3.3, Decided). The owner approved that spec, which holds the rest of this record, the same day (spec §12).
- Two parts are **Calls of the implementation**, made while building BAR-3 after that approval: the shutdown signal is installed before anything is served and awaited later, where the spec named one function, so that failing to install it is a start-up error; and Barras builds and runs only on Unix.
- Not taken: an adapter crate, which would implement no port; and a copy of the code in each binary, kept in step by review.

## Decision

- `crates/bootstrap`, the package `barras-bootstrap`, holds the start-up the binaries share:
  - `Environment`. `from_process` is the one reader of the process environment; `from_pairs` gives a test, or the check of `.env.example`, its own pairs, so no test touches the process environment. `required::<T>(name)` reads one variable and records every problem instead of stopping at the first; `finish` returns the configuration, or every problem at once.
  - `init_logging`: readable lines when `APP_ENV` is `local`, one JSON object per line otherwise (ADR-0029).
  - `install_panic_hook`: a panic is logged as an `error` event, inside the spans current where it happened, rather than as the default hook's raw line on standard error.
  - `ShutdownSignal`: `install()` listens for SIGTERM and SIGINT, and `recv()` waits for the next one.
  - `with_sources`: an error's text followed by that of each of its sources, which a binary prints when it stops on a failure.
- It depends on no workspace member but `workspace-hack` (`bootstrap-isolation`).
- Only the crates under `apps/` depend on it, in any kind of dependency (`bootstrap-apps-only`).
- Barras builds and runs only on Unix: the binaries stop on SIGTERM and SIGINT, so `ShutdownSignal` and the testkit's process helper exist only on Unix.

## Consequences

- Cost: one more row in the dependency table, with a rule for each side of it.
- `cargo xtask check-arch` enforces both rules, and requires the package at `crates/bootstrap` to be named `barras-bootstrap` (`package-name`, ADR-0024). The cargo-deny bans apply to it as to any crate that is not an adapter.
- An adapter cannot use it, so the HTTP adapter keeps its own copy of the error chain it writes to the log.
- The binaries do not compile for Windows.
- `CLAUDE.md` and `docs/standards/module-anatomy.md` carry its row.
