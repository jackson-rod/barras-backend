# ADR-0010: Authorisation and entitlements

Status: Accepted
Date: 2026-11-15

## Context

- The server must be the only authority on what a user may use. Pro unlocked by the app alone is an anti-goal.
- Reading the database on every request to authorise would slow the read path.
- The spec makes these a set of **Calls** of the designer, not confirmed by the owner.

## Decision

- The access token carries the role and the plan's entitlements, so no request reads the database to authorise.
- When a plan changes, the server sends a realtime event and the app refreshes its token.
- A new user starts on Lite. Entitlements can carry an expiry, so a Pro trial ends on its own without a store event.
- Permissions are a catalog in code, seeded into `permissions` and granted to roles through `role_permissions`.
- Each route declares the permission and the plan feature it needs.

## Consequences

- Cost: an entitlement in a token is as old as the token until the app refreshes it, so the realtime event is part of the design.
- Definition of done item 2 (ADR-0017) requires the permission and plan feature in each route's OpenAPI metadata, and the end-to-end suite fails on a route without metadata.
- The app reads entitlements from the session and never decides them (ADR-0002).
- "Seeded" above means written into every environment by a migration or by the module's code; it is not the `seeds/` of ADR-0030, which holds local development data only and never reaches production.
