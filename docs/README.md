# Barras docs

Barras is a mobile app for people in Colombia who invest. It shows their money and the market in one place.
It covers capital in Colombian pesos with a switch to other currencies, and trends for crypto and stocks with news and official figures.
It also covers income and expense records, budgets per category, and savings goals.
Two plans, Lite and Pro, are offered; Lite is free. The backend is Rust with a hexagonal architecture, Postgres and websockets.

## How we work

1. A ticket starts with a spec in `docs/specs/BAR-<n>-<topic>.md`: intent, behaviour, routes, schema changes, error codes, tests.
2. The owner approves the spec.
3. An implementation plan is written from the spec, in `docs/plans/BAR-<n>-<topic>.md`.
4. The code is written test first, and `verify` passes.
5. The pull request updates the module README, the schema doc and any ADR it touches.

Product-wide documents live in this repository (spec section 10). The frontend's own docs are at [barras-frontend docs](https://github.com/jackson-rod/barras-frontend/tree/develop/docs).

## Where things are

| Folder | What it holds | Entry file |
| --- | --- | --- |
| `specs/` | One spec per module or feature, approved before any code | [2026-11-15-barras-foundation-design.md](specs/2026-11-15-barras-foundation-design.md); per ticket, [BAR-3-core-contract.md](specs/BAR-3-core-contract.md) |
| `plans/` | Implementation plans written from approved specs | [2026-11-15-bar-1-bar-2-foundations-kickoff.md](plans/2026-11-15-bar-1-bar-2-foundations-kickoff.md); per ticket, [BAR-3-core-contract.md](plans/BAR-3-core-contract.md) |
| `adr/` | Architecture decision records: an index and `NNNN-title.md` files | [adr/README.md](adr/README.md) |
| `analysis/` | Data sources, licensing, spikes, benchmarks | [data-sources.md](analysis/data-sources.md) |
| `schema/` | The database schema, table by table | [barras-schema.md](schema/barras-schema.md) |
| `standards/` | Code standard, module anatomy, performance and testing | [code-standard.md](standards/code-standard.md) |
| `backlog/` | The order of work; mirrors Trello | [work-plan.md](backlog/work-plan.md) |
| `runbooks/` | Operational runbooks | none yet |

## Start here

- [Foundation design](specs/2026-11-15-barras-foundation-design.md)
- [Work plan](backlog/work-plan.md)
- [ADR index](adr/README.md)
