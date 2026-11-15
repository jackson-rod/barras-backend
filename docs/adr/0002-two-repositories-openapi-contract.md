# ADR-0002: Two repositories with OpenAPI as the contract

Status: Accepted
Date: 2026-11-15

## Context

- The owner decided on two repositories: `barras-backend` (Rust) and `barras-frontend` (React Native with TypeScript) (Decided, section 1).
- The two sides must not drift: the app shows its own translated text chosen by `error.code`, and it never decides entitlements.
- The contract covers the realtime event names and payloads as well as the HTTP routes.

## Decision

- The backend generates `openapi.json` with utoipa and commits it. It is the contract between the repositories, including the realtime events.
- The frontend vendors it as `api/openapi.json` at a pinned version, and `bun run api:generate` produces its types in `src/generated/`.
- The app reads entitlements from the session and never decides them.
- Product-wide documents (the plan, the schema, the contract ADRs) live in `barras-backend`; `barras-frontend` links to them.

## Consequences

- A contract change needs two pull requests: one to publish the new `openapi.json`, one to vendor it in the app.
- `just verify` fails if the committed `openapi.json` drifts from the code. The end-to-end suite fails on a route with no OpenAPI metadata.
- The frontend CI fails when the generated types are stale.
