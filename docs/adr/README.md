# Architecture decision records

An architecture decision record (ADR) records one decision: the forces behind it, the rule it sets and what it costs. An ADR outranks `CLAUDE.md`: if they disagree, the ADR wins. To add one, take the next free number, create `NNNN-title.md` in this folder with the sections Context, Decision and Consequences and the line `Status: Accepted`, and add a row to the table below in the same pull request.

| ADR | Title | Status |
| --- | --- | --- |
| [0001](0001-record-architecture-decisions.md) | Record architecture decisions | Accepted |
| [0002](0002-two-repositories-openapi-contract.md) | Two repositories with OpenAPI as the contract | Accepted |
| [0003](0003-hexagonal-layers-enforced-by-crates.md) | Hexagonal layers enforced by crates | Accepted |
| [0004](0004-two-deployables-one-workspace.md) | Two deployables from one workspace | Accepted |
| [0005](0005-http-contract.md) | The HTTP contract | Accepted |
| [0006](0006-errors-as-typed-results.md) | Errors as typed results | Accepted |
| [0007](0007-data-access.md) | Data access with sqlx and hand-written SQL | Accepted |
| [0008](0008-money-time-and-ids.md) | Money, time and ids | Accepted |
| [0009](0009-authentication.md) | Authentication | Accepted |
| [0010](0010-authorisation-and-entitlements.md) | Authorisation and entitlements | Accepted |
| [0011](0011-realtime-gateway.md) | The realtime gateway | Accepted |
| [0012](0012-outside-systems-are-ports.md) | Every outside system is a port | Accepted |
| [0013](0013-ingestion-jobs.md) | Ingestion jobs under advisory locks | Accepted |
| [0014](0014-performance-budgets.md) | Performance budgets and the dashboard read path | Accepted |
| [0015](0015-observability.md) | Logging, health and metrics | Accepted |
| [0016](0016-configuration.md) | Configuration | Accepted |
| [0017](0017-definition-of-done.md) | Definition of done | Accepted |
| [0018](0018-git-workflow.md) | Git workflow and ticket ids | Accepted |
| [0019](0019-frontend-stack-and-anatomy.md) | Frontend stack and feature anatomy | Accepted |
| [0020](0020-market-data-sources.md) | Market data sources | Accepted |
| [0021](0021-news-headline-and-link.md) | News as headline and link | Accepted |
| [0022](0022-informational-suggestions.md) | Informational suggestions and chatbot | Accepted |
| [0023](0023-subscriptions-through-the-stores.md) | Subscriptions through the stores | Accepted |
