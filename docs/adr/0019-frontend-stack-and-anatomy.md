# ADR-0019: Frontend stack and feature anatomy

Status: Accepted
Date: 2026-11-15

## Context

- Expo is the one sensible way to build the app: the faster-bundler alternative, Re.Pack with Rspack, does not properly support Expo today and would cost Expo Router and EAS builds (section 5).
- The stack is a set of **Calls** of the designer, not confirmed by the owner. Compile speed and the 2.0 s cold start to Home, a budget (section 7.3), shape it.
- The frontend has its own standards, ADRs and UI specs (section 10). Copying them here would let the two copies drift, because a pull request touches one repository.

## Decision

- The app is built with Expo SDK 58 on React Native with TypeScript. Re.Pack was set aside.
- The full stack, the feature anatomy and the import rules are decided and maintained in the frontend repository: [frontend ADR-0002](https://github.com/jackson-rod/barras-frontend/blob/develop/docs/adr/0002-frontend-stack-and-feature-anatomy.md) and [module anatomy](https://github.com/jackson-rod/barras-frontend/blob/develop/docs/standards/module-anatomy.md).

## Consequences

- A change to the frontend stack or anatomy is made in the frontend ADR, not here.
- This ADR changes only if the headline choice changes.
