# ADR-0015: Logging, health and metrics

Status: Accepted
Date: 2026-11-15

## Context

- Failures in production must be diagnosable, but logs must not leak credentials, tokens or request bodies.
- Public health endpoints must not expose the state of internal dependencies.

## Decision

- Use `tracing` with JSON output in every deployed environment. Log at boundaries and on failure.
- Never log credentials, tokens or request bodies. `println!` and `dbg!` are denied by lint.
- `GET /api/v1/health` (ready: `ok`, `degraded`, or 503) and `GET /api/v1/health/live` are public.
- Per-dependency detail and Prometheus metrics are only on an internal port.
- A 5xx answers with a generic message and the detail goes to the logs (ADR-0005).

## Consequences

- Cost: a second, internal port to expose and protect.
- clippy denies `println!` and `dbg!` outside tests (`just lint`).
- No automated check of the log content is named in the spec: the rule on credentials and tokens is enforced by review.
