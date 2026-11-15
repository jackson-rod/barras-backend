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
