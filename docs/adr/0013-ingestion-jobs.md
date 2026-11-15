# ADR-0013: Ingestion jobs under advisory locks

Status: Accepted
Date: 2026-11-15

## Context

- `barras-ingest` runs long-lived feed connections and scheduled jobs (ADR-0004), and may run on more than one instance.
- Two instances running the same job would duplicate work and data.
- Some sources are fragile: DANE publishes files, not an API, so its adapter breaks when the file layout changes (risk 4).

## Decision

- Each job runs on one instance at a time, under its own Postgres advisory lock.
- Each run is recorded in `ingest_runs`.
- Jobs and frequencies: crypto ticks and 1-minute bars live (Binance, Buda); Colombian stocks daily at end of day; news headlines every 5 minutes; news tagging with each news run; TRM and currency rates daily; DANE figures monthly; bank rates daily; trend scores every minute.
- News stores title, link, date and section, never the body (ADR-0021).
- Every source is a port (ADR-0012).

## Consequences

- Cost: a lock and a run record for every job, and operational care when a source changes shape.
- `ingest_runs` shows what ran and what failed, and the ingest foundation ticket (F-06) provides the job runner and an adapter template with contract tests.
- The Colombian stocks vendor is chosen in the licensing spike (F-10).
