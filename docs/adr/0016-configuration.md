# ADR-0016: Configuration

Status: Accepted
Date: 2026-11-15

## Context

- A service that boots with a missing variable fails later and in a confusing place.
- Scattered reads of the environment make the real configuration impossible to list.

## Decision

- Each binary loads one typed configuration from the environment at start and refuses to boot when a variable is missing.
- Every variable is listed in `.env.example`.
- Only the configuration module reads the environment.

## Consequences

- Cost: every new variable needs a typed field and an `.env.example` entry in the same pull request.
- A new developer can run the service from the repository `README.md` by copying `.env.example`.
- The spec names no automated check for the rule that only the configuration module reads the environment: review enforces it.
- Pointer: clippy now denies the four ways of reading the environment, `std::env::var`, `var_os`, `vars` and `vars_os` (`clippy.toml`), and `Environment::from_process` of `barras-bootstrap` is the one reader a binary has (ADR-0026). The test-support exception is the testkit's process helper, which reads `LLVM_PROFILE_FILE` to pass it to each binary a test starts, so that the binary's coverage is measured (ADR-0027). Review still covers the rest, such as a library that reads a variable itself (ADR-0029).
- Pointer: every variable that holds a credential is a `Secret` of `barras-bootstrap`. `Debug` and `Display` print it as `[redacted]`, it implements no `Serialize`, and only `expose()` reaches its value, so a configuration that derives `Debug` prints no password (BAR-4 spec §4.3). `barras-api` reads `API_DATABASE_URL` and `API_DATABASE_READ_URL`, both `Secret`s, and `API_DATABASE_POOL_SIZE`; its `migrate` mode reads only the first, and `seed` and `reset` read only `APP_ENV` and the first. A database URL names its host, port, user, password and database and has no query string, so that none of its parts comes from the `PG*` variables or `~/.pgpass`; sqlx still reads those variables itself for the settings no URL of ours carries (ADR-0030).
