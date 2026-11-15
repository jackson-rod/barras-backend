# ADR-0004: Two deployables from one workspace

Status: Accepted
Date: 2026-11-15

## Context

- The owner chose approach B among three: one service, one workspace with two services, or microservices per domain (Decided, section 5).
- One service would drop the exchange connections on every API deploy, and an ingestion spike would hurt user latency.
- Microservices would make Home join data over the network and multiply operations for a small team. Splitting before a measured need is an anti-goal.

## Decision

- One workspace builds two binaries: `barras-api` and `barras-ingest`. Both are composition roots only, under `apps/`.
- `barras-api` is axum, serves HTTP under `/api/v1` and the websocket gateway in one process, and is stateless so it scales horizontally.
- `barras-ingest` holds the long-lived feed connections and the scheduled jobs, and talks to the external sources.
- Redis 8 carries live ticks, the latest-quote cache, the replay buffer and rate limits between them.
- The chatbot runs inside `barras-api` and reaches the language model through a port.

## Consequences

- Cost: two deployments and Redis as an extra piece.
- Benefit: the user's read path is isolated, and the fragile part (external sources) deploys apart.
- Because every module is hexagonal (ADR-0003), moving one to its own service later means moving a crate. `cargo xtask check-arch` keeps that true.
