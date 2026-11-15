# ADR-0012: Every outside system is a port

Status: Accepted
Date: 2026-11-15

## Context

- Barras depends on systems it does not own: exchanges, news feeds, official data, Apple, Google, a store and a language model.
- Their terms and uptime are outside the project's control, and several terms are unverified (spec section 3).
- A module that called them directly would break the layers of ADR-0003 and be untestable.

## Decision

- Every system Barras does not own is reached through a port, implemented by an outbound adapter under `crates/adapters/providers/`.
- Each outbound adapter has an explicit timeout and retries with backoff and jitter.
- Each maps failures to typed errors (ADR-0006).
- Each shows the attribution its source requires.
- Each has a contract test with recorded responses.

## Consequences

- Cost: a port, an adapter, recorded fixtures and an attribution rule for every new source.
- Definition of done item 1 (ADR-0017) requires the contract test, so a pull request without it is not done.
- The default adapter for the chatbot's model is Anthropic's Claude API, a **Call** of the designer; the model is chosen in the assistant spec.
- Replacing a provider means a new adapter, not a change to a module.
