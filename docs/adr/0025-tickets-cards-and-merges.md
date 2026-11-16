# ADR-0025: Tickets, cards and merges

Status: Proposed
Date: 2026-11-15

## Context

- ADR-0018 sets the branch and commit rules and forbids AI attribution, but says nothing about a ticket's card or about how a pull request lands on `develop`.
- The card rule is the owner's, given on 2026-11-15 (Decided).
- The owner merged the first ticket by squash. A squash merge takes the pull request title as the subject of the commit on `develop`, and may take the description as its body, so neither passes through the `commit-msg` hook.
- That the title follows the commit subject rule is a **Call** of the designer, not confirmed by the owner.

## Decision

- The card of a ticket moves as its state changes: `EN PROCESO` when work starts; `EN PRUEBA` when it is implemented and under review, or its pull request is open; `APROBADA` and `PRODUCCIÓN` only after the owner approves or the pull request is merged, never on the contributor's own judgment.
- Pull requests are merged by squash.
- The pull request title is the commit subject, so it follows the commit subject rule of ADR-0018: `type(BAR-<n>): description`.
- The description follows the no-AI-attribution rule of ADR-0018.
- CI checks the title and the description of every pull request.

## Consequences

- Cost: each change of state is a manual move of the card.
- `develop` gets one commit per pull request; the commits of the branch stay readable only in the pull request.
- CI runs `cargo xtask check-pr` on the title and the description, and runs again when either is edited.
