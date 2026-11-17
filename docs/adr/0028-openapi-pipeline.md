# ADR-0028: The OpenAPI pipeline

Status: Accepted
Date: 2026-11-16

## Context

- `openapi.json` is the contract with the app and must never drift from the code (ADR-0002), and every route carries the metadata the definition of done asks for (ADR-0017).
- The generator cannot live in `crates/xtask`, which may depend on no workspace crate (`tool-isolation`, ADR-0024). From BAR-5 the document also carries the realtime events, which live in another adapter, and only a composition root sees both adapters.
- utoipa keeps extensions in a `HashMap`, so its own JSON changes from run to run.
- The owner decided on 2026-11-15 that the generator is a mode of `barras-api` (BAR-3 spec §3.2, Decided), and approved the BAR-3 spec, which holds the rest of this record, the same day (spec §7, §9.3, §12).
- **Calls of the implementation**, made while building BAR-3 after that approval: `build_app` takes no configuration, so the document needs no environment; an operationId is unique across the API; a schema name is registered once; and `ApiRouter` refuses at build time a route the document cannot describe.

## Decision

- `barras-api openapi` prints the document. `barras-api openapi --check <file>` fails, naming `just openapi`, unless the file holds the document byte for byte. Neither reads an environment variable or opens a socket.
- The document comes from `build_app(Dependencies::offline())`, the one function that wires the app the binary serves, so there is no second list of routes. `build_app` takes the dependencies, never the configuration.
- `openapi.json` is committed at the repository root in a canonical form: every object's keys sorted, pretty-printed, one final newline (`canonical_json`). Two runs give the same bytes.
- `just openapi` rewrites it through a temporary file, so a failed build leaves it whole. `just openapi-check` fails when it differs; `just verify` and CI's `checks` job run it.
- The root of the document comes from `document_base`: `info.title` is `Barras API`, written by hand, and `info.version` is the workspace version, which the HTTP adapter inherits as its own; nothing else comes from a crate's metadata. `x-common-error-codes` lists, sorted, the seven codes any route can return: `INTERNAL_ERROR`, `MALFORMED_REQUEST`, `METHOD_NOT_ALLOWED`, `NOT_ACCEPTABLE`, `PAYLOAD_TOO_LARGE`, `ROUTE_NOT_FOUND` and `UNSUPPORTED_MEDIA_TYPE`.
- Every operation under `/api/v1` has (BAR-3 spec §7.2):
  - an `operationId`, unique across the API; one tag; a summary, which utoipa takes from the first line of the handler's doc comment;
  - a 2xx response whose body is the success envelope around a named schema, and none without it, so no 204 (ADR-0029); every 4xx and 5xx response, and `default`, with the `ErrorEnvelope` body; a description on every response;
  - `x-error-codes`: the codes of its own it can return, sorted, each once, none of the common ones. It may be empty and must be present;
  - `x-permission`: the permission it needs, or `public`; `x-plan-feature`: the `Feature` code it needs, or `none`.
- `ApiRouter` is the only door to the public API. `ApiRouter::route(routes!(handler), access, codes)` takes handlers that carry `#[utoipa::path]`, an `Access` value (the permission and the plan feature) and the route's own codes, and writes the three `x-` keys from them, over any key of the same name the handler wrote. No method of it takes a bare axum `MethodRouter`, and its fields are private, so no module of the HTTP adapter can build one around an axum router of its own.
- `ApiRouter` adds nothing, and records a problem, for a route that names other than one path, that documents no operation, that brings a method its path already has, or that brings a schema under a name already registered with another shape; `merge` does the same for two routers. `HttpAppBuilder::build` then refuses the app with every problem, so a binary that would serve such a route stops before it listens, and `just openapi` fails.
- A schema name is registered once: when two modules have a type of the same name, one of them takes `#[schema(as = …)]`.
- The guard against a route without metadata has three layers (BAR-3 spec §9.3):
  1. `ApiRouter`, above.
  2. `metadata_problems`, which an e2e test runs on the document. It names the method, the path and each item an operation lacks, an operation with more than one tag, every operationId used twice, and a root without the title, the version or `x-common-error-codes` above.
  3. `TestClient`, which fails a test when a route the document lacks, by method and template, served its request; and when a route the document has answers an error envelope whose `error.code` neither its `x-error-codes` nor `x-common-error-codes` lists, or whose status its operation does not document, as itself, as its range (`4XX`) or with `default`, unless a common code has that status (400, 404, 405, 406, 413, 415, 500).
- Every route of BAR-3 is `public` and `none`. BAR-17 adds the constructors of `Access` that take a permission and a feature; the guard it adds reads the same value, so what a route documents and what it enforces cannot differ.

## Consequences

- Cost: every route change is followed by `just openapi` and a commit of `openapi.json`. A stale file fails `just openapi-check`, the e2e test that compares it, `just verify` and CI.
- The guards of `TestClient` trip only on what a test requests. What holds without them: only `ApiRouter::route` adds a route to the public API, and `undocumented_route`, the one way around it, exists only behind the HTTP adapter's `testing` feature, which `testing-dev-only` (ADR-0024) keeps out of every build but the tests. What does not: `route` takes the tuple `routes!` makes, and a tuple built by hand can serve a method its document lacks, as `router.rs` says; only the route guard sees that.
- `metadata_problems` cannot know which codes and statuses a handler really returns: that each one is documented, the 422 of a route with a validating extractor included (ADR-0029), is left to review, and to `TestClient` for the answers the tests receive.
- From BAR-5 the realtime events join the same document (ADR-0002, ADR-0011).
- Pointer: `x-common-error-codes` lists nine codes, `REQUEST_BODY_TIMEOUT` and `REQUEST_TIMEOUT` with the seven above. `TestClient` exempts a common code from the status rule only at the status the adapter answers it at, no longer any status that some common code has: a route's own code at 503, say, needs its 503 documented (ADR-0031).
