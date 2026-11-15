# ADR-0006: Errors as typed results

Status: Accepted
Date: 2026-11-15

## Context

- The app branches on `error.code` (ADR-0005), so failures must be explicit, named and stable.
- The domain must not know HTTP, or the hexagonal layers (ADR-0003) would leak.
- Swallowed errors and panics in a request path hide failures from the logs.

## Decision

- Every use case and port returns `Result<T, <Module>Error>`. There are no exceptions, and no `anyhow` outside `main.rs`.
- `<Module>Error` is one enum per module with one variant per failure case.
- Each variant has a stable `SCREAMING_SNAKE_CASE` code and an `ErrorKind`: validation, unauthenticated, forbidden, not found, conflict, rate limited, unavailable or internal.
- The code is public API: renaming it is a breaking change.
- Only the `http` adapter maps `ErrorKind` to a status.
- Never swallow an error: handle it, wrap it with its source, or return it.

## Consequences

- Cost: a variant, a code and a mapping for each failure case, and a breaking-change discipline on codes.
- clippy denies `unwrap`, `expect` and `panic!` outside tests, and runs with zero warnings (`just lint`).
- Tests assert errors by `code` (ADR-0017).
