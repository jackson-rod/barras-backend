# Contributing

How we work day to day. The rules themselves are in [CLAUDE.md](CLAUDE.md) and bind every contributor, human or agent; this file is the routine around them. To set up your machine, follow [README.md](README.md).

## Tickets

- Every piece of work is a card on the Trello board BARRAS. The card number is the ticket id: card 15 is `BAR-15`, and `BAR-15` goes into the branch name and every commit subject.
- The order of work is in [`docs/backlog/work-plan.md`](docs/backlog/work-plan.md), which mirrors the board.
- Move the card as its state changes:

| Column | When |
| --- | --- |
| `EN PROCESO` | When work starts |
| `EN PRUEBA` | When it is implemented and under review, or its pull request is open |
| `APROBADA`, `PRODUCCIÓN` | Only after the owner approves or the pull request is merged; never on your own judgment |

## From ticket to code

1. The ticket starts with a spec in `docs/specs/BAR-<n>-<topic>.md`: intent, behaviour, routes, schema changes, error codes, tests.
2. The owner approves the spec.
3. An implementation plan is written from the spec, in `docs/plans/BAR-<n>-<topic>.md`.
4. The code is written test first, and `just verify` passes.
5. The pull request updates the module `README.md`, the [schema doc](docs/schema/barras-schema.md) and any ADR it touches.

What "done" means is the definition of done in [CLAUDE.md](CLAUDE.md#definition-of-done).

## Branches and commits

The full rules are in [CLAUDE.md](CLAUDE.md#git-and-pull-requests); the git hooks and CI check them.

| | Good | Bad |
| --- | --- | --- |
| Branch | `feat/BAR-15-email-sign-up` | `feature/bar-15-Email_SignUp`: `feature` is not a type, `bar` is lower case, the slug has capitals and an underscore |
| Commit subject | `feat(BAR-15): add email sign-up with a verification code` | `Added email sign-up`: no type, no ticket, starts with a capital |
| Pull request title | `feat(BAR-15): add email sign-up with a verification code` | `BAR-15 Email sign-up`: no type, the ticket is not in parentheses, the description starts with a capital |

- A pull request is merged by squash, so its title becomes the subject of the one commit that lands on `develop` and follows the commit subject rule. Its description may become that commit's body, so it carries no AI attribution either.
- `git commit --fixup` is refused by the `commit-msg` hook, because a `fixup!` subject does not match the subject rule. Write an ordinary commit instead.
- A commit message, a pull request title and a pull request description never hold a marker that makes GitHub skip CI: `[skip ci]`, `[ci skip]`, `[no ci]`, `[skip actions]`, `[actions skip]` or a `skip-checks: true` trailer. The `commit-msg` hook refuses them, and CI refuses them in every commit of the branch and in the pull request's text. To write about one in a message, describe it in words.
- After adding or changing a dependency, refresh `Cargo.lock` with a plain `cargo build` before committing: `cargo xtask` and `cargo deny` run with `--locked`, so the hooks fail on a stale lock file.
- After adding a crate, or changing a dependency or its features, run `just hakari` and commit what it changes: `crates/workspace-hack/Cargo.toml`, the `workspace-hack` lines of the members and `Cargo.lock`. `just hakari-check` fails until then, in `just verify` and in CI.
- After adding or changing a route, run `just openapi` and commit `openapi.json`. `just openapi-check` fails until then, in `just verify` and in CI.

## Pull requests

1. Push your branch. The `pre-push` hook checks the branch name and runs `just verify`.
2. Open the pull request against `develop`. When the branch has more than one commit, GitHub pre-fills the title from the branch name, which `cargo xtask check-pr` refuses: replace it with `type(BAR-n): description`, as in the table above. Fill in the template: the ticket id and its Trello card, what and why, the four definition of done checkboxes, notes for the reviewer.
3. Move the card to `EN PRUEBA`.
4. CI runs `checks`, `test` and `build`. All three must pass. `checks` also checks the branch name, every commit message of the branch (not only the last one), and the pull request title and description. Editing the title or the description runs CI again. The README says what [each job runs](README.md#branches-pull-requests-and-ci).
5. The owner reviews and merges by squash.
6. After the owner approves or merges, the card moves on as the table under [Tickets](#tickets) says.
