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
- Pointer: clippy now denies the four ways of reading the environment, `std::env::var`, `var_os`, `vars` and `vars_os` (`clippy.toml`), and `Environment::from_process` of `barras-bootstrap` is the one reader allowed (ADR-0026). Review still covers the rest, such as a library that reads a variable itself (ADR-0029).
