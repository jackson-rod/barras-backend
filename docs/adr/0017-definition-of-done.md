# ADR-0017: Definition of done

Status: Accepted
Date: 2026-11-15

## Context

- The working rules adapt the owner's earlier `CLAUDE.md` (Decided, section 1), and apply to both repositories.
- Architecture rules must be enforced by checks, not convention (section 2, goal 8).
- The coverage thresholds are a **Call** of the designer, not confirmed by the owner.

## Decision

A change is done only when the same pull request contains:

1. Tests per layer: domain types directly, use cases with fake ports, mappers round trip in both directions, routes through end-to-end tests against the real app, a contract test for every adapter to an outside system, and errors asserted by `code`, never by message.
2. OpenAPI metadata on every route, including the error codes it can return and the permission and plan feature it needs.
3. Docs: the module `README.md` is current, and a new or changed decision is an ADR listed in the index.
4. `verify` green locally and in CI.

Coverage: 95% of lines in module crates and in frontend `domain` and `application`; 85% overall.

## Consequences

- Cost: tests, metadata and docs are part of every change.
- `just verify` runs everything CI runs, and `just test-cov` applies the coverage thresholds.
- `just check-structure` checks the module README, tests per layer and the ADR index.
- Pointer: ADR-0027 says how coverage is measured: production lines only, crate by crate, with the kernel held to the modules' bar. `just verify` runs what CI runs apart from the branch, commit and pull request checks and the tree check (`justfile`).
