# ADR-0005: The HTTP contract

Status: Accepted
Date: 2026-11-15

## Context

- The app and the backend are built in separate repositories (ADR-0002), so the shape of every response must be uniform and stable.
- The app shows its own translated text, so it needs a stable code rather than a message to branch on.
- Responses must never leak SQL, stack traces, tokens or another user's data, and money must not be a floating-point number.

## Decision

- Every route is under `/api/v1`.
- Success is `{ success: true, message, data, code }`. Error is `{ success: false, message, data: null, code, error: { code, fields? } }`.
- The top-level `code` is the HTTP status as a number. `error.code` is the stable `SCREAMING_SNAKE_CASE` string of ADR-0006.
- Validation failures answer 422 with `error.fields`, a map from field path to messages.
- `message` is English and for developers; the app chooses its text by `error.code`.
- Messages never contain SQL, stack traces, tokens or another user's data. A 5xx answers with a generic message and the detail goes to the logs.
- Money and any integer above 2^53 travel as strings.

## Consequences

- Cost: every client reads the envelope, and changing it breaks both repositories.
- The end-to-end suite fails on a route with no OpenAPI metadata, and `just verify` fails if `openapi.json` drifts (ADR-0002).
- Tests assert errors by `code`, never by message (ADR-0017).
