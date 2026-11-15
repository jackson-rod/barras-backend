# ADR-0011: The realtime gateway

Status: Accepted
Date: 2026-11-15

## Context

- Prices and the user's own data must reach the phone fast: a tick from source to phone has a p95 budget of 1 s (ADR-0014).
- One API node must hold 10,000 connections, and a slow connection must not hurt the others.
- The spec makes these a set of **Calls** of the designer, not confirmed by the owner.

## Decision

- One websocket endpoint in `barras-api`. The first frame authenticates with the access token, so tokens never appear in a URL.
- Channels: `user:{id}` (private changes and the chatbot stream), `market:{symbol}` (ticks) and `trends` (ranking changes).
- Envelope: `{ id, type, ts, data }`. User events say what changed and the app refetches it; a lost event only delays freshness.
- The app resumes with `lastEventId`; the replay buffer is a capped Redis stream.
- Ticks are coalesced to at most one update per second per instrument per connection. On subscribe the app gets the current quote first.
- A slow connection loses old ticks and never blocks the others.
- `barras-ingest` publishes ticks to Redis; each API node subscribes once and fans out in memory.

## Consequences

- Cost: Redis is on the live path, and the app must refetch on user events.
- Event names and payloads are part of `openapi.json` (ADR-0002).
- The nightly load test checks the budgets (ADR-0014).
