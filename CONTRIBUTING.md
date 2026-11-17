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
- A commit message, a pull request title and a pull request description never hold a marker that makes GitHub skip CI: `[skip ci]`, `[ci skip]`, `[no ci]`, `[skip actions]`, `[actions skip]` or a `skip-checks: true` trailer. The `commit-msg` hook refuses them in a commit message. CI refuses them in the pull request's title and description and in the commits of the branch, but only in a run that starts: GitHub starts none for a pull request whose head commit holds one, so there nothing but the hook refuses it. To write about one in a message, describe it in words.
- After adding or changing a dependency, refresh `Cargo.lock` with a plain `cargo build` before committing: `cargo xtask` and `cargo deny` run with `--locked`, so the hooks fail on a stale lock file.
- After adding a crate, or changing a dependency or its features, run `just hakari` and commit what it changes: `crates/workspace-hack/Cargo.toml`, the `workspace-hack` lines of the members and `Cargo.lock`. `just hakari-check`, in `just verify` and in CI, fails while either would change, and when the generated crate changes what a release build compiles: a crate to which only tests ask for more then joins `third-party` under `[traversal-excludes]` in `.config/hakari.toml`.
- After adding or changing a route, run `just openapi` and commit `openapi.json`. `just openapi-check` fails until then, in `just verify` and in CI.

## The database

The rules are in [CLAUDE.md](CLAUDE.md#data-access) and [ADR-0030](docs/adr/0030-database-foundation.md); the [README](README.md#the-local-database) sets up the local database.

- A new migration: `just db-migrate <name>`, with a snake_case name such as `create_users`, creates `migrations/<timestamp>_<name>.sql`. Write its SQL, naming the schema of every table (`barras.users`). `just db-deploy` applies it to the database of `.env`, and `just db-status` shows what is applied and what is pending.
- Migrations are forward-only: fix a mistake with a new migration. Never edit one that has been merged: `barras-api migrate` refuses a database that ran it before the change. One you are still writing on your branch you may edit; `just db-reset` then rebuilds your local database, which ran the old text.
- After changing a query or adding a migration, run `just db-prepare` and commit what it changes in `.sqlx/`. `just db-check` fails until then, in `just verify` and in CI. Both start a throwaway Postgres of their own, so the local database may be down, but Docker must be running.
- Seeds hold local development data, in `seeds/`, named like migrations. Reference data that every environment needs goes in a migration or in its module's code, never in a seed. A seed never ends the transaction the files run in, with `COMMIT` or `ROLLBACK`: the run fails on it.

## When a new advisory blocks

`just deny` checks the advisories in every pull request, so an advisory published today fails every pull request, yours included, until someone deals with it (ADR-0030, and the comment of `[advisories]` in `deny.toml`):

1. Fix it in a pull request of its own, usually with `cargo update -p <crate>`, which rewrites `Cargo.lock`.
2. When no fix exists, ignore it in `[advisories]` of `deny.toml`, with a reason and the date, in a pull request of its own. Ban its crate in `[bans]` with the crates that bring it as its only wrappers, so that the ignore covers no other crate.
3. The one exception is an advisory that your pull request's own new dependency brings. An ignore that matches no crate fails the check, so it cannot land first: ignore the advisory in that pull request, scoped by a ban whose only wrapper is that dependency.

sqlx-cli is a tool, not a member of the workspace, so `just deny` never reads its dependencies: it is built from the lock file it was published with. When you change its pinned version, in `just setup` and in CI, run `cargo deny check` on the lock of the new version before the change lands. ADR-0030 lists what the lock of 0.9.0 holds, and why it is accepted.

## Your `.env`

- `just dev` and the `db-*` commands that connect export the variables of `.env`, your copy of `.env.example`. When `.env.example` gains a variable, add it to your `.env`: a binary refuses to start without it, naming it.
- The sqlx query macros read every `.env` from the crate's folder up to `/` when they compile, offline too. One that their parser cannot read fails every build, with `error reading dotenv file` and its path, not the line, and that includes a `.env` above the checkout, such as one in your home folder. Keep each line a comment or `NAME=value`, with a value of letters, digits and `. _ : / @ % + , = [ ] -` only, as `.env.example` has: a blank or an unclosed quote in a value breaks it.

## Pull requests

1. Push your branch. The `pre-push` hook checks the branch name and runs `just verify`, which needs the network, and Docker running: `just db-check` and the `int_` and `e2e_` tests start Postgres containers.
2. Open the pull request against `develop`. When the branch has more than one commit, GitHub pre-fills the title from the branch name, which `cargo xtask check-pr` refuses: replace it with `type(BAR-n): description`, as in the table above. Fill in the template: the ticket id and its Trello card, what and why, the four definition of done checkboxes, notes for the reviewer.
3. Move the card to `EN PRUEBA`.
4. CI runs `checks`, `test` and `build`. All three must pass. `checks` also checks the branch name, every commit message of the branch (not only the last one), and the pull request title and description. Editing the title or the description runs CI again. The README says what [each job runs](README.md#branches-pull-requests-and-ci).
5. The owner reviews and merges by squash.
6. After the owner approves or merges, the card moves on as the table under [Tickets](#tickets) says.
