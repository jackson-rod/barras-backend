# ADR-0031: Request timeout and shared readiness

Status: Accepted
Date: 2026-11-17

## Context

- BAR-3 left these notes on the card of BAR-4 (BAR-4 spec §14): no server-side time limit bounded a request; `GET /api/v1/health`, which is public, ran every probe on every request, so it could be used to load the dependencies; and the text of a probe's error, which the internal port and the log show, had to hold no credential once a probe checks a database.
- ADR-0029 fixes the adapter's own codes, the stack of middleware and readiness, and gives the requests in flight 10 seconds at shutdown. ADR-0028 lists the codes any route can return, and how `TestClient` holds a route's error answers to its document.
- The owner approved the BAR-4 spec on 2026-11-16. Its **Calls** are accepted with it: the time limit, its code and its place in the stack, and readiness shared and reused for a second (spec §6.3, §6.4, §15).
- The owner decided the same day, after the spikes and the review of the timeout, that a request whose body is still arriving at the limit is the client's slowness, not the server's (plan, refinement 7). The review then narrowed it to a handler waiting for body data that has not come, as refinement 7 now reads: a first version answered 408 to any body not read to its end, and so answered a slow server as a slow client.
- **Calls of the implementation**, made while building it: a middleware of our own rather than tower-http's `TimeoutLayer`; an answer even for a handler that panics as it is dropped; and, from the review of the timeout, a common code exempt from `TestClient`'s status rule only at its own status.

## Decision

### The time limit

- A request has 5 seconds from its arrival to the head of its answer, the reading of its body included (`REQUEST_TIME_LIMIT`). A body that streams out after the head is not limited. 5 seconds is far above every budget of ADR-0014, and under the 10 seconds a shutdown gives the requests in flight (ADR-0029), so a request in flight at SIGTERM ends before that wait does.
- When the limit passes, the handler is dropped wherever it was waiting, and one place chooses the answer:
  - 408, with the envelope, `error.code` `REQUEST_BODY_TIMEOUT` and `connection: close`, only when the handler was then waiting for more of a body that the client had not finished sending: its last poll of the body found nothing, before the body's end. That is the client's slowness. It is a 4xx, so it logs no `request failed` event, and it is counted as a 408.
  - 503, with the envelope and `error.code` `REQUEST_TIMEOUT`, for every other request: one with no body, one whose body was read to its end, and one whose handler never read its body or stopped reading it, such as a slow extractor that runs before the body is read, or a handler that reads a chunk and stalls. That time is the server's. Like any 5xx, it answers a generic message and logs one `request failed` event, whose detail names the limit.
- A panic raised while the handler is dropped is caught, so the client still gets its answer, with its request id, logged and counted. The 503's detail names the panic; for a 408, only the panic hook's event shows it.
- The layer is an `axum::middleware::from_fn` with tokio's `timeout`. tower-http's `TimeoutLayer` answers with an empty body, which the safety net could only make an `INTERNAL_ERROR` envelope of.
- Its place in the stack of ADR-0029: inside the request log, `PropagateRequestId`, the metrics and the safety net, so that a timeout is logged, counted and carries the request id; outside compression, `CatchPanic` and the body limit, so that it bounds all a handler does.
- The internal port has none of the public middleware (ADR-0029), so it has no time limit.

### The common codes

- `REQUEST_TIMEOUT` and `REQUEST_BODY_TIMEOUT` join the codes any route can return. `x-common-error-codes` lists nine, sorted: `INTERNAL_ERROR`, `MALFORMED_REQUEST`, `METHOD_NOT_ALLOWED`, `NOT_ACCEPTABLE`, `PAYLOAD_TOO_LARGE`, `REQUEST_BODY_TIMEOUT`, `REQUEST_TIMEOUT`, `ROUTE_NOT_FOUND` and `UNSUPPORTED_MEDIA_TYPE`. A route never lists one of them among its own codes.
- `TestClient` exempts a common code from its status rule only at the status the adapter answers it at: `INTERNAL_ERROR` 500, `MALFORMED_REQUEST` 400, `METHOD_NOT_ALLOWED` 405, `NOT_ACCEPTABLE` 406, `PAYLOAD_TOO_LARGE` 413, `REQUEST_BODY_TIMEOUT` 408, `REQUEST_TIMEOUT` 503, `ROUTE_NOT_FOUND` 404 and `UNSUPPORTED_MEDIA_TYPE` 415. Every other error answer of a documented route needs its status documented, as itself, as its range or with `default`. That includes a route's own code at one of those statuses, such as a module error of kind `Unavailable` at 503, and a common code at another code's status, such as the `INTERNAL_ERROR` the safety net makes of a layer's bare 503.

### Readiness shared

- Readiness requests share one run of the probes. A request that comes while a run goes waits for its reports rather than starting another, and the reports of a run that ended less than a second ago are given again. The public `GET /api/v1/health` and the internal `GET /health` of a binary share the same runs.
- A run is a task of its own, which records its reports even when the request that started it goes away, as when the client closes its connection: leaving starts runs no faster than waiting. So however often the public route is asked, the dependencies see at most one run a second.
- Within a run, every probe still runs at once with the others, with 2 seconds to answer, and one that does not answer in time, or whose check panics, counts as failed (ADR-0029).

### What a probe reports

- A probe's error is one sentence of a fixed set. It never holds a URL, a user name, a password, or a driver's or a server's text, which can name the user when a password is refused, and in one SCRAM failure prints a character of the password. The internal `/health` and the log show it; the public routes never do.
- The HTTP adapter's own sentences: `did not answer within 2 seconds`, and `stopped without an answer` for a check that panicked.
- The sentences of the database probes of ADR-0030, the postgres adapter's `DatabaseUnavailable`: `no connection within 2 seconds`, when the database cannot be reached or every connection is busy; `the pool is closed`; `the server refused it with SQLSTATE <code>`, or `the server refused it` when the server gave no code; `the connection failed: <kind>`, where the kind is the standard library's fixed description of an `io::ErrorKind`, never the operating system's text; `TLS could not be set up`; `the server answered outside the protocol`; and `the driver failed`.

## Consequences

- Cost: every route can answer 503 `REQUEST_TIMEOUT` and 408 `REQUEST_BODY_TIMEOUT` without documenting them, so the app handles both on any route. `openapi.json` changed with the two codes.
- A request that timed out may have done part of its work: its handler is dropped wherever it was. A statement on a serving pool is cancelled by Postgres after the same 5 seconds (ADR-0030), so it does not outlive its request for long.
- The 408 rests on the handler's last poll of the body: a handler that got a chunk and stopped polling is a slow server, even if more of the body was still to come.
- The limit counts from the moment hyper hands the request over, once it has read the head. A client slow to send its head is not this layer's: a header-read timeout belongs to the deployment's proxy (ADR-0029, BAR-12).
- Readiness can be up to a second behind, plus the length of a run: a database that has just gone down still reads as up until the next run.
- `HealthProbe::new` takes any text as a probe's error, so review holds a probe added later to a fixed set.
- ADR-0028 and ADR-0029 keep their Decisions; each points here.
