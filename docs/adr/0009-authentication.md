# ADR-0009: Authentication

Status: Accepted
Date: 2026-11-15

## Context

- Users register with email, Apple or Google. Apple requires Sign in with Apple when Google sign-in is offered, and in-app account deletion (section 3).
- The client is a mobile app, so there is no browser, no cookies and no CSRF to defend against.
- The spec makes these a set of **Calls** of the designer, not confirmed by the owner.

## Decision

- Barras owns identity. Email and password use argon2id, with a verification code by email.
- Apple and Google sign-in verify the provider's ID token against the provider's keys: issuer, audience, nonce and expiry. The same verified email links to one user.
- The app sends `Authorization: Bearer <access token>`. Access tokens are signed JWTs that live 15 minutes.
- Refresh tokens rotate on every use and are stored hashed. Reuse of an old one revokes the session.
- No cookies and no CSRF.

## Consequences

- Cost: the backend owns passwords, sessions and email verification, and must keep the provider keys current.
- Every adapter to Apple or Google has a contract test with recorded responses (ADR-0012).
- Tokens are never logged (ADR-0015) and never appear in a URL (ADR-0011).
- Entitlements travel in the token (ADR-0010).
