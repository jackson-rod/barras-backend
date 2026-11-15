# ADR-0022: Informational suggestions and chatbot

Status: Accepted
Date: 2026-11-15

## Context

- Under Decreto 661 of 2018, advice tailored to the user is a regulated activity reserved to supervised entities. A suggestion or chatbot answer that reads as such is an anti-goal.
- Generic trend lists marked "informativo, no constituye asesoría" are defensible; suggestions personalised from the user's own finances are not. A lawyer reviews this before launch.
- The definitions of suggestion and chatbot are **Calls** of the designer, not confirmed by the owner.

## Decision

- A suggestion is an instrument highlighted because it is trending, ranked by price movement, volume and news mentions. It is informational and the same for every user.
- Lite sees the top 8; Pro sees all. Suggestions carry the disclaimer.
- The chatbot (Pro) explains the market, the news and the user's own numbers. It never recommends buying or selling.
- Barras does not buy or sell anything.

## Consequences

- Cost: no personalised ranking, which a user might expect.
- The spec names no automated check: the guardrails are built in the chatbot ticket (S-06), and the legal review is ticket L-01.
- Drift toward personal advice is a launch risk (risk 5).
