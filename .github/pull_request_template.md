<!-- The title becomes the commit on develop (the pull request is merged by squash) and must read type(BAR-n): description, as in feat(BAR-15): add email sign-up with a verification code. -->

## Ticket

BAR-n:
Trello:

## What and why

## Definition of done

- [ ] Tests per layer: domain types tested directly, use cases with fake ports, mappers round trip, routes through e2e, every adapter to an outside system with a contract test, errors asserted by `code`.
- [ ] OpenAPI metadata on every route, including its error codes and the permission and plan feature it needs.
- [ ] Docs: the module `README.md` is current, and a new or changed decision is an ADR listed in the index.
- [ ] `just verify` is green locally and in CI.

## Notes for the reviewer
