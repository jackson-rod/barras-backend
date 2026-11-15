# Work plan

The order of work. It mirrors the Trello board BARRAS. `BAR` is the Trello card number and the ticket id used in branch names and commit subjects.

The Trello board BARRAS has status columns: ACT - DEV, ACT - DISEÑO, EN PROCESO, EN PRUEBA, APROBADA, PRODUCCIÓN. Cards are created in ACT - DEV, and design cards in ACT - DISEÑO, in the order below. Each title starts with its stage and repository so the order survives moving between columns. Ticket titles are in English, like the branches made from them.

Repository tags: BE backend, FE frontend, DOC docs, DS design, OWN the owner.

| BAR | # | Stage | Repo | Ticket |
| --- | --- | --- | --- | --- |
| 1 | F-01 | Foundations | DOC | Seed `docs/` in both repositories: plan, standards, ADR index and first ADRs, schema, data-source analysis |
| 2 | F-02 | Foundations | BE | Workspace skeleton: toolchain, crates, lints, justfile, hooks, cargo-deny, xtask checks, CI, CLAUDE.md, README |
| 3 | F-03 | Foundations | BE | Core contract: errors, envelope, validation, configuration, logging, health, metrics, OpenAPI pipeline, e2e harness |
| 4 | F-04 | Foundations | BE | Database foundation: pools, migrations, `barras` schema, seed and reset, offline metadata check |
| 5 | F-05 | Foundations | BE | Redis and realtime foundation: pub/sub, websocket gateway, channels, replay |
| 6 | F-06 | Foundations | BE | Ingest foundation: job runner, advisory locks, `ingest_runs`, adapter template with contract tests |
| 7 | F-07 | Foundations | FE | App skeleton: Expo, router, TypeScript 7 typecheck, oxlint, bun, architecture check, Jest, Maestro, CI, CLAUDE.md, README |
| 8 | F-08 | Foundations | FE | Core contract: API client, generated types, query client with persistence, realtime client, i18n, configuration, secure storage |
| 9 | F-09 | Foundations | DS | Design system and screen designs delivered by the owner's designer; mapped to theme tokens |
| 10 | F-10 | Foundations | DOC | Spike: data licensing (Colombian stocks vendor, Binance and Buda display terms, El Espectador permission) |
| 11 | F-11 | Foundations | FE | Spike: candlestick chart library and its performance on a mid-range Android |
| 12 | F-12 | Foundations | BE | Hosting and environments ADR; container images; staging deploy |
| 13 | F-13 | Foundations | OWN | Accounts and identifiers: Apple Developer, Google Play, Expo, RevenueCat, bundle id, domain |
| 14 | A-01 | Sign up and sign in | DOC | Identity spec |
| 15 | A-02 | Sign up and sign in | BE | Users, roles and permissions; email sign-up with verification; sign-in; token refresh; sign-out; password reset |
| 16 | A-03 | Sign up and sign in | BE | Sign in with Apple and Google; account linking |
| 17 | A-04 | Sign up and sign in | BE | Authorisation: permission catalog, route guards, entitlements in the token |
| 18 | A-05 | Sign up and sign in | FE | Sign-in, sign-up, verify-email and forgot-password screens; session and token refresh |
| 19 | O-01 | Onboarding | BE | User settings and currencies: preferences and onboarding state |
| 20 | O-02 | Onboarding | FE | Onboarding slides, first preferences, first-visit guides per section |
| 21 | H-01 | Home | BE | TRM and currency-rate ingestion; currency conversion |
| 22 | H-02 | Home | DOC | Capital spec: accounts, investments, valuation, snapshots and the dashboard read model |
| 23 | H-03 | Home | BE | Banks, exchanges and accounts |
| 24 | H-04 | Home | BE | Investments and movements by class; valuation |
| 25 | H-05 | Home | BE | Daily capital snapshots |
| 26 | H-06 | Home | BE | Dashboard read model and its performance test |
| 27 | H-07 | Home | FE | Home: capital card with currency switch, change, breakdown, latest activity, live updates |
| 28 | P-01 | Profile and menu | BE | Profile, sessions, password change, account deletion |
| 29 | P-02 | Profile and menu | FE | Perfil: personalisation, plan, settings, security and privacy |
| 30 | P-03 | Profile and menu | FE | Menú: preferences, currency, notifications, language |
| 31 | P-04 | Profile and menu | BE+FE | Push notifications: device tokens, preferences, sender |
| 32 | T-01 | Trends | DOC | Markets, news and indicators spec |
| 33 | T-02 | Trends | BE | Instruments and exchanges catalog |
| 34 | T-03 | Trends | BE | Crypto ingestion: ticks, latest quotes, 1-minute bars |
| 35 | T-04 | Trends | BE | Colombian stocks end-of-day ingestion (only if F-10 ends with a licence) |
| 36 | T-05 | Trends | BE | Markets API: list, detail, bars, favourites, realtime channels |
| 37 | T-06 | Trends | BE | News ingestion and API, with the Lite limit |
| 38 | T-07 | Trends | BE | Trend scoring and suggestions API, with the Lite limit and the disclaimer |
| 39 | T-08 | Trends | BE | DANE and bank-rate ingestion; indicators API |
| 40 | T-09 | Trends | FE | Tendencias list: live prices, favourites, filters |
| 41 | T-10 | Trends | FE | Instrument detail: bars chart, news, favourite |
| 42 | T-11 | Trends | FE | News and suggestions with the Lite limit; official figures |
| 43 | C-01 | Capital | DOC | Income, expenses and budgets spec |
| 44 | C-02 | Capital | BE | Income, expenses and categories |
| 45 | C-03 | Capital | BE | Budgets per category and their progress |
| 46 | C-04 | Capital | FE | Capital overview; register an investment and its movements |
| 47 | C-05 | Capital | FE | Register an expense or income; categories |
| 48 | C-06 | Capital | FE | Budgets: create per category, progress |
| 49 | C-07 | Capital | BE+FE | Scan a receipt into an expense draft |
| 50 | G-01 | Goals | DOC | Goals spec |
| 51 | G-02 | Goals | BE | Goals: create, contribute from an account, progress and time, close, achieved |
| 52 | G-03 | Goals | FE | Metas screens |
| 53 | S-01 | Pro | DOC | Billing and assistant spec |
| 54 | S-02 | Pro | BE | Plans, store webhook, subscription state, entitlement change event |
| 55 | S-03 | Pro | FE | Paywall, Pro trial offer after onboarding, purchase and restore, switch to Lite, gated features |
| 56 | S-04 | Pro | BE+FE | Full history for bars and trends |
| 57 | S-05 | Pro | BE+FE | Custom stock tracker: watchlists and price alerts |
| 58 | S-06 | Pro | BE | Chatbot: conversations, streaming, context, guardrails, quotas |
| 59 | S-07 | Pro | FE | Chatbot in Tendencias |
| 60 | L-01 | Launch | DOC | Terms, privacy policy, disclaimers; legal review of suggestions and the chatbot |
| 61 | L-02 | Launch | BE+FE | Load test against the budgets; store listings and submission |

Notes on the order (**Calls**):

- **Home comes before Capital in the flow, but it reads capital data.** So the backend for accounts, investments and snapshots is built in the Home stage (H-02 to H-05), before the dashboard read model. The screens that record that data stay in the Capital stage. Until they land, Home shows its empty state and developers use the demo seed.
- **Home gains market-driven change when Trends lands.** Before that, investments are valued at the last price the user entered.
- **Colombian stock prices are conditional.** If F-10 ends without a licence, the first version ships with crypto, news and official figures. Stocks stay in the catalog so the user can record them, valued at the last price the user entered.
- **News is firm.** Headline and link need no permission.
- **The Pro trial is offered at the end of onboarding.** Until the Pro stage lands, onboarding skips the offer.
- **F-09 is an outside dependency.** The designs come from the owner's designer; frontend screen tickets from A-05 onward wait for the screens they need.
- **F-13 belongs to the owner.** Sign in with Apple and Google (A-03), store builds (F-07), hosting (F-12) and the store webhook (S-02) cannot be finished without those accounts, and Apple enrolment takes time.
