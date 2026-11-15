# ADR-0008: Money, time and ids

Status: Accepted
Date: 2026-11-15

## Context

- Money stored as a floating-point number is an anti-goal that makes Barras a failure even if it works (section 2).
- Capital is shown in COP with a switch to other currencies, so an amount is meaningless without its currency.
- The spec makes these a set of **Calls** of the designer, not confirmed by the owner.

## Decision

- Money is `rust_decimal::Decimal` in Rust and `numeric` in Postgres, always with its currency. Fiat uses `numeric(20,2)`; quantities use `numeric(38,18)`.
- Timestamps are `timestamptz` in UTC. The user's time zone applies only when presenting or grouping, such as a budget's month.
- Ids are UUID version 7.
- On the wire money travels as a string (ADR-0005).
- `Money`, `Currency` and the ids live in the shared `kernel` crate.

## Consequences

- Cost: every amount carries a currency, and every conversion is explicit.
- Month and day boundaries depend on the user's time zone, so they are computed at presentation or grouping time, never stored in local time.
- The spec names no dedicated automated check for these rules: they are enforced by the shared types, by the schema and by review.
