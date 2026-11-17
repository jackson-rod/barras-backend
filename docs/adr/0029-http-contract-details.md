# ADR-0029: Details of the HTTP contract

Status: Accepted
Date: 2026-11-16

## Context

- ADR-0005 fixes the envelopes, the codes and the rule that messages never leak. It leaves open what the HTTP adapter answers on its own, the shape of the JSON, the request id, the limits and the internal port of ADR-0015.
- serde quotes the value it refuses in its messages, and a field may hold a password, so the text of an error cannot reach the caller as it is.
- The owner approved the BAR-3 spec, which holds these details, on 2026-11-15 (spec §5, §6, §8, §12). Its **Calls** are accepted with it: camelCase, no 204, the body limit, compression from 1 KiB, the internal port without envelope, the `prometheus` registry and the two-second limit of a probe.
- The rest are **Calls of the implementation**, made in the reviews of the adapter while building BAR-3, after that approval: the field messages and the rules on DTOs, how a body is judged to be JSON, the `WWW-Authenticate` header, the request span at `error` level, and the 10-second limit on the requests in flight at shutdown, which refines spec §8.3.

## Decision

### Codes and statuses

- Only the HTTP adapter maps an `ErrorKind` to a status: `Validation` 422, `Unauthenticated` 401, `Forbidden` 403, `NotFound` 404, `Conflict` 409, `RateLimited` 429, `Unavailable` 503, `Internal` 500 (ADR-0006).
- The adapter's own codes:

| `error.code` | Status | When |
| --- | --- | --- |
| `VALIDATION_FAILED` | 422 | A body or query field is missing, has the wrong type or breaks a rule. `error.fields` says which |
| `MALFORMED_REQUEST` | 400 | The body is not UTF-8, not JSON or not a JSON object, or the query string cannot be read |
| `UNSUPPORTED_MEDIA_TYPE` | 415 | A body not sent as `application/json` or `application/<x>+json` |
| `PAYLOAD_TOO_LARGE` | 413 | A body over 1 MiB |
| `NOT_ACCEPTABLE` | 406 | `Accept-Encoding` refuses every encoding the server has, the plain body included |
| `ROUTE_NOT_FOUND` | 404 | No route matches the path |
| `METHOD_NOT_ALLOWED` | 405 | The path exists, the method does not. The `allow` header is kept |
| `SERVICE_UNAVAILABLE` | 503 | `GET /api/v1/health` with a critical probe failing |
| `INTERNAL_ERROR` | 500 | A handler panicked |

- `error.fields` comes only with `VALIDATION_FAILED`. A module error of kind `Validation`, such as the kernel's `INVALID_CURRENCY`, answers 422 with its own code and no `error.fields`.
- A 4xx from a module error answers the error's `Display` as its message, verbatim. A 5xx answers a generic message, and the error's text with its chain of sources goes to the log at `error` level (ADR-0005).
- A 401 carries `WWW-Authenticate: Bearer`, as RFC 9110 §15.5.2 requires.
- Every answer the application produces on the public port is an envelope. A safety net replaces any 4xx or 5xx that a library layer answered without a JSON body, and every 406, with an envelope at the same status: 413, 415 and 406 get their codes above, any other 4xx `MALFORMED_REQUEST` and any 5xx `INTERNAL_ERROR`. A bare 429 from a future rate-limit layer is mapped to a code of its own when that layer lands.
- hyper, the server library, answers a request whose head it cannot read itself, before any of our code runs: 400 for a head that is not HTTP, 414 for a URI longer than 65,534 bytes, and 431 for more than 100 headers or a head larger than its buffer of 408 KiB. That answer has an empty body, so no envelope and no request id, and it is neither logged nor counted. "Every answer is an envelope" covers what the application produces.

### Shape

- JSON field names are camelCase, in bodies, in queries and in the keys of `error.fields`.
- A route with nothing to return answers 200 with `data: null`, and documents `SuccessEnvelope<NoData>`. Never 204: one shape for the client, and `metadata_problems` refuses a 2xx without the success envelope (ADR-0028).
- A request body is a JSON object.

### Request id, compression and limits

- `x-request-id`: the caller's value is kept when exactly one is sent and it is 1 to 64 characters of `A-Z a-z 0-9 . _ -`; otherwise the server makes a UUID v7. It is on every answer the application gives on the public port, a 404, a 405, a 413 and a panic included, though not on hyper's own (above), and in the span of every log event of the request.
- An answer of 1 KiB or more, or of unknown length, is compressed with gzip or brotli as `Accept-Encoding` chooses; never for gRPC, images or server-sent events. An `Accept-Encoding` that refuses every encoding still runs the handler before the answer becomes 406.
- A request body is limited to 1 MiB.

### Validation

- A handler takes its body with `ValidatedJson<T>` and its query with `ValidatedQuery<T>`, never with axum's `Json` or `Query`, whose rejections are not envelopes. Every crate that takes axum keeps `default-features = false`, so the stock `Json`, `Query` and `Form` extractors exist nowhere.
- A route that takes either lists `VALIDATION_FAILED` among its codes and documents a 422.
- Whether a body is JSON is decided by a strict parse of the whole of it, which must also be UTF-8 throughout (RFC 8259 §8.1). A body that is not JSON answers 400 even when a field before the fault was wrong. A lone surrogate escape, nesting deeper than serde_json reads, or a number past `f64` (`1e400`) is accepted in a member the DTO does not read; in a field it reads, it answers 422 at that field, or 400 for the nesting.
- serde stops at the first value it refuses; garde reports every broken rule. Each key of `error.fields` is the JSON path of its field (`address.zip`, `tags[1]`): serde's paths already are, and garde's are converted to camelCase segment by segment.
- A missing field is keyed at its own path; in a body, once a tree of the body confirms that it is missing (below). A field that an object does not accept (`deny_unknown_fields`) is keyed at that object, never under its own name, which is the caller's text; at the root it answers 400. A query that gives a parameter twice, or one the DTO does not accept, answers 400.
- A field's message comes from a fixed set, never from serde's text: `is required`; `expected <type>` for the primitive types serde names in fixed words, `has the wrong type` for any other; `is not one of the accepted values`; `has a field it does not accept`; `is not valid`. A broken garde rule answers garde's message, which names what the rule expects, never the value it refused.
- Every DTO, body and query alike, takes `#[serde(rename_all = "camelCase")]` and never renames a field one by one: garde 0.23 parses `#[garde(rename(…))]` but never applies it, so a field renamed by hand would be reported under its Rust name.
- A DTO holds no map keyed by caller data, such as a `HashMap<String, _>`: a key would come back as a key of `error.fields`. A list of objects, each with its key as a field, takes its place.
- Nothing the caller sent comes back. A DTO type's own `Deserialize` or `TryFrom` error, the message of a garde `custom` rule and the `Display` of a module error never quote their input, because each reaches the client as written.
- A handler that validates a JSON body itself keys its errors by paths in that body, JSON names in camelCase. `TestClient` fails a test whose 422 `VALIDATION_FAILED` has a key with `_` in a name or, answering a JSON body, a key that names no member of the body, no query parameter and no field missing from an object of the body.
- To confirm a missing field, the extractor builds a tree of the body, and a tree takes many times the size of its text: a body of 1 MiB peaked at about 24 MiB. So it builds one only for a body of at most 64 KiB. In a larger body the field is not confirmed: the key is the path of the object that lacks it, with `is not valid`, and at the root, where no field is left to name, the body answers 400.

### Logs and metrics

- The request log is a span named `request`, with `method`, `path` (the route template, or `unmatched`) and `request_id`, and one `info` event, `request finished`, with `status` and `duration_ms`. It never records a header, the query string or the body (ADR-0015); a test sends a secret in all three and finds none in the captured output.
- The span is created at `error` level, so that under any `LOG_LEVEL` the events of a failing request keep its method, its path and its request id.
- A 5xx logs one `error` event, `request failed`, with `code`, `status` and `detail`. A handler panic shows twice: the panic hook's event (ADR-0026) and the `request failed` event of its 500.
- The metrics of BAR-3 spec §6.6 live in a `prometheus` registry the composition root creates and hands to the app; nothing is global. They are `http_requests_total` (`method`, `path`, `status`), `http_request_duration_seconds` (`method`, `path`; buckets at 5, 10, 25, 50, 80, 100, 150, 200 and 500 ms, 1, 2.5 and 5 s) and `barras_build_info` (`service`, `version`; always 1). `path` is the route template or `unmatched`, and a method outside the nine standard ones is `other`, so the labels take bounded values. A failure to record is logged and never fails the request.
- The duration, in the log and in the histogram, runs to the head of the answer: a body that streams is not waited for.

### Health and the internal port

- `GET /api/v1/health/live` answers `ok` while the process runs. `GET /api/v1/health` runs every probe at once, each with 2 seconds to answer; a probe that does not answer in time, or whose check panics, counts as failed. It answers `ok` when every probe passes; `degraded` when only optional probes fail, and logs a `warn` naming them; 503 `SERVICE_UNAVAILABLE` when a critical one fails, with the failed critical probes in the detail of `request failed`. The public answers never name a dependency (ADR-0015).
- The internal port has no `/api/v1` prefix, no envelope and none of the public middleware, and is not in `openapi.json`; its requests are neither counted nor logged as public ones. `GET /health` answers 200, or 503 when a critical probe fails, with `status` (`ok`, `degraded` or `unavailable`), `service`, `version` and `dependencies`: per probe its `name`, `critical`, `status` (`up` or `down`), `latencyMs` and, when it is down, `error`. `GET /metrics` answers the Prometheus text format.

### Shutdown and the runtime

- After SIGTERM or Ctrl-C a binary stops accepting connections and gives the requests in flight at most 10 seconds. Then it stops waiting, logs a `warn`, and exits 0. A header-read timeout against slow clients belongs to the deployment's proxy (BAR-12).
- The binaries build their runtime with `build_runtime` of `barras-bootstrap`, which gives tokio its worker count, one per processor, so tokio never reads `TOKIO_WORKER_THREADS` (ADR-0016, ADR-0026).

## Consequences

- Cost: a DTO cannot rename a field one by one or hold a map keyed by caller data, and its own errors, its validators and every module error's `Display` are written so that they never quote their input.
- The app can rely on one shape for every answer of the public port, a request id on each, and camelCase everywhere.
- The guard on the keys of `error.fields`, like the route guard, lives in `TestClient`, so it holds only for what the tests send; review covers the rest (ADR-0028).
- A tracing layer that reads the level of spans, such as an OpenTelemetry one, will see every request span as an error.
- A request still running 10 seconds after the signal is cut off with its connection.
- Pointer: ADR-0031 limits a request to 5 seconds, with two codes of the adapter, `REQUEST_TIMEOUT` (503) and `REQUEST_BODY_TIMEOUT` (408), which join the table above and the common codes; its layer sits between the safety net and compression. Readiness requests share one run of the probes, on both ports, whose reports are given again for a second, and each probe's error is a sentence of a fixed set.
