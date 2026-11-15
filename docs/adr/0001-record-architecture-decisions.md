# ADR-0001: Record architecture decisions

Status: Accepted
Date: 2026-11-15

## Context

- The design fixes many decisions at once (spec section 10), and the owner wants a spec-driven plan with ADRs in `docs/` (Decided).
- Without a written record, a later change or a new agent re-decides what was already settled, or invents a second pattern in one module.
- The statuses and the supersede rule below are a **Call** of the designer, taken from the structure check in the BAR-2 plan, not confirmed by the owner.
- The working rules say what to do when something is not covered: read the ADR for the area, and if none exists, stop and ask or propose one.

## Decision

- Record every decision about architecture, the contract with the app or the working rules as an ADR in `docs/adr/`, named `NNNN-title.md`, numbered without gaps.
- Each ADR has the sections Context, Decision and Consequences, and a Status line.
- The statuses are `Proposed`, `Accepted` and `Superseded by ADR-NNNN`. A new ADR is `Proposed` until the owner accepts it.
- A decision is changed by a new ADR. The ADR it replaces gets the status `Superseded by ADR-NNNN` and its Decision is not rewritten.
- List every ADR in the index `docs/adr/README.md`.
- A pull request that adds or changes a decision includes its ADR.
- An ADR outranks `CLAUDE.md`.

## Consequences

- Each decision costs a short document, written in the same pull request as the change.
- `just check-structure` verifies the ADR index. Definition of done item 3 (ADR-0017) requires the ADR in the same pull request.
- The first ADRs record the decisions of the foundation design, so the design document can be superseded without losing them.
