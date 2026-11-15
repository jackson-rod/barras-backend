# Testing

This restates the foundation design, sections 9 and 6.14.

## Tests per layer

A change is done only when the same pull request contains tests for every layer it touches:

- Domain types are tested directly.
- Use cases are tested with fake ports.
- Mappers round trip in both directions.
- Routes are tested through e2e against the real app.
- Every adapter to an outside system has a contract test.
- Errors are asserted by `code`, never by message.

## Coverage

- 95% of lines in module crates.
- 85% overall.

## Where tests run

- Unit tests need no Docker (`just test`).
- Integration and e2e tests run against throwaway Postgres 18 and Redis 8 containers (`just test-int`).
- `just test-cov` runs both, with coverage thresholds.
