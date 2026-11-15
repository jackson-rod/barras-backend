# ADR-0023: Subscriptions through the stores

Status: Accepted
Date: 2026-11-15

## Context

- Apple and Google require in-app purchase for digital subscriptions (section 3).
- The owner decided on two plans: Lite is free, and a free trial of Pro is offered at sign-up. That Barras is a product to charge for is a **Call** of the designer: the owner did not answer.
- The server must be the only authority on what a user may use.

## Decision

- Pro is bought through the stores, with RevenueCat (`react-native-purchases`) in the app. RevenueCat is a **Call** of the designer.
- The backend keeps plans, subscriptions and `subscription_events` (store events, idempotent), and takes store events by webhook.
- A plan change updates entitlements (ADR-0010) and sends the realtime event.
- Switch to Lite: the app opens the store's subscription screen, and Lite starts when the paid period ends. This is a **Call**.
- The trial's length and mechanism are decided in the billing spec.

## Consequences

- Cost: the stores' rules apply to every purchase, and Pro cannot be sold outside them.
- The store webhook needs the owner's accounts (F-13) before it can be finished.
- The store adapter has a contract test (ADR-0012).
