# Barras: Foundation Design

Date: 2026-11-15. Status: draft, waiting for the product owner's approval.

This is the foundation design for Barras. It fixes the architecture, the working rules, the repository layouts, the first schema, the module map and the ordered backlog. It does not design each feature in detail: every module gets its own spec in `docs/specs/` before it is built (section 12).

Throughout this document, **Decided** marks what the product owner said. **Call** marks a decision the designer made and the owner has not confirmed. Facts about tool versions and data sources were checked on live pages on 2026-11-15 unless marked *unverified*.

## 1. Intent

Barras is a mobile app for people in Colombia who invest. It shows their money and the market in one place: capital in Colombian pesos with a switch to other currencies, trends for crypto and stocks with news and official figures, income and expense records, budgets per category, and savings goals.

**Call (the why):** Barras is a product to launch and charge for through two plans, Lite and Pro. The owner was asked and did not answer; this is the working assumption.

**Decided:**

- The name is Barras ("bars", as in price bars). The target is Colombia.
- Two repositories: `barras-backend` (Rust) and `barras-frontend` (React Native with TypeScript).
- Backend: Rust, hexagonal architecture, Postgres, websockets.
- Performance is the most important quality: the product must be very fast, and so must the compile and feedback loop.
- One role: Trader. Two plans: Lite and Pro. Pro adds the chatbot, removes the cap of 8 suggestions and 8 news items, adds a custom stock tracker and history.
- Sources named: Bancolombia, Davivienda, Nu; exchanges and the stock market; El Espectador; DANE.
- The working rules come from the owner's earlier NestJS project (`CLAUDE.md`, `README.md`, code standard), adapted to Rust.
- `docs/` holds a spec-driven plan with ADRs, analysis, schema and standards.
- Backend shape: one workspace, two services (approach B, section 5).
- Trello tickets are created after this design is approved, ordered by the user flow, and written in English.
- Lite is free. A free trial of Pro is offered when the user signs up.
- Home shows the change of total capital and also the change of each class and each investment.
- Visual design comes from the owner's existing designs or designer, not from this project's builder.

## 2. Goals, non-goals, anti-goals

### Goals

1. A person registers with email, Apple or Google, goes through a guided onboarding, and lands on Home.
2. Home shows total capital in COP with a switch to the user's display currency, the percentage change, and the latest investment activity.
3. Tendencias shows crypto and stocks, each with its price bars and latest news; the user can favourite instruments.
4. Capital records investments (crypto, stocks, bank), expenses and income, and budgets per category.
5. Metas: create a goal, add money to it, see where that money is held, close it, see achieved goals and time.
6. Perfil and Menú: profile, plan, settings, security and privacy; preferences, currency, notifications, language.
7. Lite and Pro work end to end, with the server as the only authority on what a user may use.
8. The architecture rules are enforced by the compiler and by checks, not by convention.
9. Both repositories can be run by a new developer on the first try from their `README.md`.

### Non-goals for the first version

- Connecting to bank accounts. No regulated open-finance API will be usable in Colombia before about 2028; banks are a catalog and the user records balances by hand.
- Real-time prices for Colombian stocks. The exchange (BVC, now nuam) licenses real-time data by contract only.
- Buying or selling anything through Barras.
- A web app. iOS and Android only.
- An admin back office. Operational tasks are CLI commands.

### Anti-goals (these make Barras a failure even if it works)

- A suggestion or a chatbot answer that reads as advice tailored to the user. Under Decreto 661 of 2018 that is a regulated activity reserved to supervised entities.
- Reproducing news article text. El Espectador's terms forbid reproduction and commercial use without written authorisation.
- A slow Home screen, a multi-megabyte JSON response, or money stored as a floating-point number.
- Pro unlocked by the app alone, without the server agreeing.
- Splitting the backend into many services before a measured need.

## 3. Constraints

| Area | Constraint |
| --- | --- |
| Language | Everything in the repositories is in English: code, comments, commits, pull requests, docs. The app's interface is Spanish (es-CO) first, with English as the second language. **Call.** |
| Stores | Apple and Google require in-app purchase for digital subscriptions. Apple requires Sign in with Apple when Google sign-in is offered, and in-app account deletion. |
| Regulation | Generic trend lists marked "informativo, no constituye asesoría" are defensible. Suggestions personalised from the user's own finances are not. A lawyer reviews this before launch. Personal data falls under Ley 1581 of 2012 (*unverified*, from the designer's own knowledge). |
| News | El Espectador: headline and link only. The only live feed is `https://www.elespectador.com/arc/outboundfeeds/discover/category/economia/`. |
| Colombian stocks | End-of-day prices only until a licence is bought. Vendors that cover the exchange give end-of-day data; the one found with display rights lists about USD 1,099 per month. Yahoo has no redistribution right and is not used. |
| Crypto | Binance public websocket (`wss://data-stream.binance.vision`, no key) and Buda (COP pairs: BTC, ETH, SOL, USDC, USDT, LTC, BCH). Their terms for showing data in a commercial app are *unverified*. |
| Official figures | TRM: free daily API on datos.gov.co (dataset `32sa-8pi3`, CC BY-SA 4.0). DANE: no API; monthly XLSX files, cited as "Fuente: DANE"; OECD SDMX is the fallback. Bank CDT and savings rates per entity: datos.gov.co dataset `axk9-g2nh`, daily. |
| Developer machine | Node 24, Postgres 18 and Docker are installed. Rust, bun and pnpm are not. |
| Repositories | Both exist on GitHub with only `LICENSE` and `README.md` on `main`. |

## 4. Product scope

### Screens and navigation

```
(auth)        Sign in · Sign up · Verify email · Forgot password
(onboarding)  4 to 5 explanation slides · first preferences (currency, language)
(tabs)        Home · Tendencias · Capital · Metas · Perfil
              Menú (preferences, currency, notifications, language) opens from the header
```

- **Onboarding** also leaves a short first-visit guide in each section, so the user is accompanied the first time they open Tendencias, Capital and Metas.
- **Home:** total capital with currency switch; its percentage change over 24 hours, 7 days or 30 days; a breakdown by class (crypto, stocks, bank) and by investment, each with its own change; latest investment activity. Total capital is the sum of the user's account balances and the current value of their investments.
- **Tendencias:** instrument list with live price and change; instrument detail with bars chart, its latest news and a favourite toggle; suggestions; news feed; official figures (TRM, DANE, bank rates); chatbot (Pro).
- **Capital:** investments by class, register an investment and its movements, register an expense or income, budgets per category. Scanning a receipt into an expense draft comes last.
- **Metas:** create, contribute from an account, progress and time, close, achieved list.
- **Perfil:** personalise the profile, see and change the plan, settings, security (password, sessions) and privacy (delete account).

Visual design is not part of this document. The owner supplies it; F-09 tracks its delivery, and every frontend screen ticket depends on it.

### Definitions that the owner has not confirmed (**Calls**)

- **Suggestion:** an instrument Barras highlights because it is trending, ranked by price movement, volume and news mentions. It is informational and the same for every user. Lite sees the top 8; Pro sees all.
- **News limit:** Lite sees the 8 latest headlines; Pro sees all, paginated.
- **Chatbot (Pro):** explains the market, the news and the user's own numbers. It never recommends buying or selling.
- **History (Pro):** Lite sees the last 7 days of bars; Pro sees the full history, plus trend history.
- **Custom stock tracker (Pro):** user-made watchlists with price alerts by push notification.
- **Banks and exchanges:** catalogs. The user picks one to say where money or an investment is held. Barras also shows the published CDT and savings rates of Bancolombia, Davivienda and Nu.
- **Switch to Lite:** the app opens the store's subscription screen; Lite starts when the paid period ends.
- **Change of capital:** the current value compared with the daily snapshot at the start of the chosen period.
- **Pro trial:** offered at the end of onboarding. Its length (starting proposal: 7 days) and its mechanism (a store introductory offer or a trial granted by the server) are decided in the billing spec.
- **Display currencies (seed list):** COP, USD, EUR, GBP, MXN, BRL.

### Plans

| Capability | Lite | Pro |
| --- | --- | --- |
| Price | free | paid, with a free trial offered at sign-up |
| Capital, budgets, goals | yes | yes |
| Trends, favourites, live crypto prices | yes | yes |
| Suggestions | top 8 | all |
| News | latest 8 | all |
| Bars history | 7 days | full |
| Chatbot | no | yes |
| Custom stock tracker | no | yes |

## 5. Approaches

### Backend shape

| | A. One service | B. One workspace, two services | C. Microservices per domain |
| --- | --- | --- | --- |
| What it is | One binary serves HTTP and websockets and runs the data ingestion inside | Same repository and modules; `barras-api` serves users, `barras-ingest` talks to external sources | Identity, capital, markets, news and chatbot as separate services with messaging and a gateway |
| For | Simplest and cheapest to deploy | The user's read path is isolated and fast; the fragile part (external sources) lives and deploys apart | Each domain scales and deploys alone |
| Against | Every API deploy drops the exchange connections; an ingestion spike hurts user latency | Two deployments, and Redis as an extra piece | Home would join data over the network; operations multiply for a small team |

**Chosen by the owner: B.** It protects the read path, and because every module is hexagonal, moving one to its own service later means moving a crate, not rewriting.

### Frontend

There is one sensible way: **Expo SDK 58** (React Native 0.88, Hermes V1, New Architecture, React Compiler on). The faster-bundler alternative, Re.Pack with Rspack, does not properly support Expo today and would cost Expo Router and EAS builds. Compile speed comes from the TypeScript 7 native compiler for typechecking, oxlint and oxfmt instead of ESLint and Prettier, and Metro.

## 6. Backend architecture (`barras-backend`)

### 6.1 What runs

```
app ⇄ barras-api ⇄ Postgres 18 (schema `barras`)
          ▲
        Redis 8 (live ticks, latest-quote cache, replay buffer, rate limits)
          ▲
     barras-ingest ⇄ Binance, Buda, El Espectador, datos.gov.co, DANE, equities vendor
```

- **`barras-api`:** axum. HTTP under `/api/v1` and the websocket gateway in one process. Stateless, so it scales horizontally.
- **`barras-ingest`:** long-lived feed connections and scheduled jobs. Each job runs on one instance at a time under its own Postgres advisory lock, and records each run in `ingest_runs`.
- The chatbot runs inside `barras-api` and reaches the language model through a port. **Call:** the default adapter is Anthropic's Claude API; the model is chosen in the assistant spec.

### 6.2 Workspace layout

```
Cargo.toml              workspace; [workspace.dependencies] pins each version once; [workspace.lints]
rust-toolchain.toml     Rust 1.100.0, edition 2024
deny.toml               cargo-deny: licences, advisories, and the dependency bans that guard the layers
justfile                every command
lefthook.yml            git hooks
apps/
  api/                  binary barras-api: composition root only
  ingest/               binary barras-ingest: composition root only
crates/
  kernel/               shared kernel: AppError, ErrorKind, Money, Currency, ids, Page and Cursor,
                        Entitlements, Clock port
  modules/<module>/     one crate per bounded context: domain, ports and use cases
  adapters/
    postgres/           sqlx repositories and mappers
    http/               axum routers, DTOs, extractors, envelope, error mapping, OpenAPI
    realtime/           websocket gateway
    redis/              pub/sub, cache, rate limiter
    providers/          outbound clients: exchanges, feeds, DANE, Apple, Google, store, model
  testkit/              test-only helpers: containers, fixtures, fakes
  xtask/                repository checks: architecture, structure, commit and branch names
migrations/             plain SQL migrations (sqlx)
.sqlx/                  offline query metadata, committed
docs/
```

Modules: `identity`, `settings`, `billing`, `capital`, `budgets`, `goals`, `markets`, `news`, `indicators`, `assistant`.

### 6.3 Module anatomy

```
crates/modules/<module>/        snake_case; plural for collections
  README.md                     routes, use cases, ports, owned and read tables (required)
  Cargo.toml                    no web, database, HTTP-client or runtime dependency
  src/
    lib.rs                      the module's public surface
    domain/
      <entity>.rs               struct or enum that owns its invariants; constructors return Result
      errors.rs                 one enum <Module>Error, one variant per failure case
    ports/
      <name>_<role>.rs          a trait named for the role: UserRepository, QuoteFeed
    application/
      <use_case>.rs             one struct, one `execute`, its own input and output types
      services/                 only for a decision shared by two or more use cases of the module
```

Adapters mirror the modules: `crates/adapters/postgres/src/<module>/<name>_repository.rs` with an explicit mapper in both directions, and `crates/adapters/http/src/<module>/` with `routes.rs` and `dto/<action>.rs`. A handler is thin: validate the DTO, convert it into the use case's input, call one use case, map the output.

### 6.4 Dependency rule

| Crate | May depend on |
| --- | --- |
| `kernel` | `std` and small pure crates (thiserror, uuid, time, rust_decimal) |
| `modules/<m>` | `kernel` and the same pure crates, plus `async-trait`. Never another module, an adapter, axum, sqlx, reqwest, redis, tokio's runtime or serde_json |
| `adapters/*` | `kernel`, the module crates, and its own technology. Never another adapter |
| `apps/*` | everything. The only place where a concrete adapter meets a port |

- Cargo enforces most of this: a module crate cannot import what its `Cargo.toml` does not list.
- `cargo deny check bans` restricts which crates may depend on sqlx, axum, reqwest and redis.
- `cargo xtask check-arch` reads `cargo metadata` and fails on any dependency outside each crate's allowlist. Only the kernel and the modules have an allowlist; ADR-0024 lists what it checks for every layer.
- A module that needs something from another module declares its own port. The composition root supplies a bridge that calls the other module's use case. Example: `capital` declares `ExchangeRates`; the bridge calls `indicators`.

Reject on sight: a framework type in a module crate; a database row type in a use case signature; a handler calling a repository; a module depending on another module; `unwrap`, `expect` or `panic!` outside tests.

### 6.5 Ports and wiring (**Calls**)

- Ports are traits. Use cases hold `Arc<dyn Port>`; `main.rs` builds every adapter and use case by hand. Dynamic dispatch costs nothing measurable next to a database call and compiles faster than generics.
- Async port traits use `async-trait`. If native async traits are usable behind `dyn` on the pinned toolchain, the builder may use them instead.

### 6.6 Errors

- Every use case and port returns `Result<T, <Module>Error>`. There are no exceptions and no `anyhow` outside `main.rs`.
- Each variant has a stable `SCREAMING_SNAKE_CASE` code and an `ErrorKind` (validation, unauthenticated, forbidden, not found, conflict, rate limited, unavailable, internal). The code is public API: renaming it is a breaking change.
- The domain does not know HTTP. Only the `http` adapter maps `ErrorKind` to a status.
- Never swallow an error. Handle it, wrap it with its source, or return it.

### 6.7 Contract with the app

- Every route is under `/api/v1`.
- Success: `{ success: true, message, data, code }`. Error: `{ success: false, message, data: null, code, error: { code, fields? } }`. The top-level `code` is the HTTP status as a number. `error.code` is the stable `SCREAMING_SNAKE_CASE` string from section 6.6. Validation failures answer 422 with `error.fields` (field path to messages).
- `message` is English and for developers. The app shows its own translated text chosen by `error.code`.
- Messages never contain SQL, stack traces, tokens or another user's data. A 5xx answers with a generic message; the detail goes to the logs.
- Money and any integer above 2^53 travel as strings.
- The backend generates `openapi.json` with utoipa and commits it; `just verify` fails if it drifts. It is the contract between the two repositories, including the realtime event names and payloads. The e2e suite fails on a route with no OpenAPI metadata.

### 6.8 Authentication and authorisation (**Calls**)

- Barras owns identity. Email and password (argon2id) with a verification code by email; Apple and Google by verifying their ID token against the provider's keys (issuer, audience, nonce, expiry). The same verified email links to one user.
- The app sends `Authorization: Bearer <access token>`. Access tokens are signed JWTs that live 15 minutes. Refresh tokens rotate on every use, are stored hashed, and reuse of an old one revokes the session. No cookies and no CSRF, because there is no browser.
- The token carries the role and the plan's entitlements, so no request reads the database to authorise. When a plan changes, the server sends a realtime event and the app refreshes its token.
- A new user starts on Lite. Entitlements can carry an expiry, so a Pro trial ends on its own without a store event.
- Permissions are a catalog in code, seeded into `permissions` and granted to roles through `role_permissions`. Each route declares the permission and the plan feature it needs.

### 6.9 Data access (**Calls**)

- sqlx 0.9 with hand-written SQL, checked at compile time in offline mode. No ORM.
- Two pools: primary for writes and for reads that follow a write; a read pool for read models (a replica when one exists).
- Read models aggregate in SQL. Lists use cursor pagination, never `OFFSET`, with at most 50 items per page.
- The schema changes only through migrations. SQL that looks odd (a partial index, a CHECK) carries a comment that says why.
- Money is `rust_decimal::Decimal` in Rust and `numeric` in Postgres, always with its currency. Fiat uses `numeric(20,2)`; quantities use `numeric(38,18)`.
- Timestamps are `timestamptz` in UTC. The user's time zone applies only when presenting or grouping, such as a budget's month.
- Ids are UUID version 7.

### 6.10 Realtime (**Calls**)

- One websocket endpoint in `barras-api`. The first frame authenticates with the access token, so tokens never appear in a URL.
- Channels: `user:{id}` (private changes and the chatbot stream), `market:{symbol}` (ticks), `trends` (ranking changes).
- Envelope: `{ id, type, ts, data }`. User events say what changed and the app refetches it; a lost event only delays freshness. The app resumes with `lastEventId`; the replay buffer is a capped Redis stream.
- Ticks are coalesced to at most one update per second per instrument per connection. On subscribe the app gets the current quote first. A slow connection loses old ticks, never blocks the others.
- `barras-ingest` publishes ticks to Redis; each API node subscribes once and fans out in memory.

### 6.11 Every system Barras does not own is a port

Each outbound adapter has an explicit timeout, retries with backoff and jitter, maps failures to typed errors, shows required attribution, and has a contract test with recorded responses.

| Job in `barras-ingest` | Source | Frequency |
| --- | --- | --- |
| Crypto ticks and 1-minute bars | Binance and Buda websockets | live |
| Colombian stocks | vendor chosen in the licensing spike (F-10) | daily, end of day |
| News headlines | El Espectador feed; stores title, link, date, section; never the body | every 5 minutes |
| News tagging | matches headlines to instruments by alias | with each news run |
| TRM and other currency rates | datos.gov.co; `open.er-api.com` for other pairs | daily |
| DANE figures | monthly XLSX; OECD SDMX fallback | monthly |
| Bank rates | datos.gov.co `axk9-g2nh` | daily |
| Trend scores | computed from price change, volume and news mentions | every minute |

### 6.12 Performance rules

Starting budgets. **Call**; the first load test tunes them through an ADR.

| Budget | Target |
| --- | --- |
| `GET /api/v1/dashboard` server time | p95 ≤ 80 ms, p99 ≤ 200 ms |
| Any other read | p95 ≤ 150 ms |
| Any write | p95 ≤ 200 ms |
| Response body | ≤ 64 KB compressed |
| Tick from source to phone | p95 ≤ 1 s |
| Incremental `cargo check` after a one-line change in a module | ≤ 5 s |
| `just test` (unit) | ≤ 30 s |
| `just verify`, and CI | ≤ 10 min |

- Home loads in one request: `GET /api/v1/dashboard` returns everything the screen needs.
- One query per read model, or an explicit batch. Never one query per row.
- Nothing builds a multi-megabyte JSON in a request. Exports stream.
- Release builds use thin LTO and one codegen unit. The allocator is chosen by benchmark.
- Build speed: many small crates so Cargo compiles in parallel; versions pinned once in the workspace; cargo-hakari to unify features; minimal feature flags; sccache and cargo-chef in CI; cargo-nextest. Cranelift is not used because it needs a nightly toolchain.
- Hot paths have criterion benchmarks. A nightly load test checks the budgets.

Sizing for launch (**Call**; orders of magnitude, refined in the markets spec):

- **Users:** 10,000 registered and 1,000 concurrent websocket connections. The load test checks the budgets at 200 requests per second on the dashboard. One API node must hold 10,000 connections.
- **Instruments:** a curated catalog, not everything an exchange lists. About 50 crypto instruments (the largest Binance USDT pairs plus Buda's COP pairs) and the Colombian listings (about 57).
- **History kept:** 1-minute bars for 30 days, hourly bars for 2 years, daily bars without limit. Back-filled from the source where the source allows it.

### 6.13 Logging, health, configuration

- `tracing` with JSON output in every deployed environment. Log at boundaries and on failure. Never log credentials, tokens or request bodies. `println!` and `dbg!` are denied by lint.
- `GET /api/v1/health` (ready: `ok`, `degraded`, or 503) and `/api/v1/health/live` are public. Per-dependency detail and Prometheus metrics are only on an internal port.
- Each binary loads one typed configuration from the environment at start and refuses to boot when a variable is missing. Every variable is in `.env.example`. Only the configuration module reads the environment.

### 6.14 Commands

Everything runs from the repository root through `just`.

| Command | What it does |
| --- | --- |
| `just setup` | Installs the pinned toolchain, the cargo tools and the git hooks |
| `just dev` / `just dev-ingest` | Runs the API or the ingestor locally with reload |
| `just lint` / `just fmt` / `just fmt-check` | clippy with zero warnings; rustfmt |
| `just check-arch` | cargo-deny bans and `xtask check-arch` |
| `just check-structure` | Module README, tests per layer, ADR index, migration names |
| `just test` | Unit tests, no Docker |
| `just test-int` | Integration and e2e tests against throwaway Postgres 18 and Redis 8 containers |
| `just test-cov` | Both, with coverage thresholds |
| `just db-migrate <name>` / `db-deploy` / `db-status` / `db-check` | Create, apply, inspect and verify migrations |
| `just db-seed` / `just db-reset` | Seed or rebuild the local database; refuse a database that is not on this machine |
| `just db-prepare` | Refresh the sqlx offline metadata |
| `just openapi` | Regenerate `openapi.json` |
| `just verify` | Everything CI runs, in CI order |

Git hooks (lefthook): `pre-commit` runs format check, clippy, architecture and structure checks; `commit-msg` checks the message; `pre-push` checks the branch name and runs `just verify`. Bypassing them is not allowed.

## 7. Frontend architecture (`barras-frontend`)

### 7.1 Stack (**Calls**)

- Expo SDK 58 with Expo Router, development builds and EAS. React Compiler on.
- TypeScript strict with `noUncheckedIndexedAccess`. TypeScript 6 stays installed for Expo's tooling; TypeScript 7 runs the typecheck.
- oxlint (type-aware) and oxfmt. bun as package manager and script runner.
- TanStack Query for server state, persisted to MMKV so Home paints from cache on a cold start. Realtime events patch or invalidate that cache. Zustand for the little client state there is. Tokens in expo-secure-store.
- Unistyles 3 for styling, FlashList for lists, Reanimated for animation, i18next with expo-localization, react-hook-form with zod.
- Charts: a Skia candlestick chart. The library is chosen in a spike (F-11); writing our own on Skia is the fallback, since bars are the centre of the product.
- RevenueCat (`react-native-purchases`) for subscriptions.
- Jest with jest-expo and React Native Testing Library; Maestro for end-to-end flows.

### 7.2 Layout and feature anatomy

```
app/                     Expo Router routes only: thin screens that compose features
src/
  core/                  the core contract: API client (envelope, errors by code), session, realtime client,
                         configuration schema, i18n, theme, query client, storage
  features/<feature>/    kebab-case
    README.md
    domain/              types and pure functions; no React, no IO
    application/         use-case hooks that orchestrate ports; query keys
    infrastructure/      API adapters and mappers (DTO to domain and back), realtime subscriptions, device APIs
    ui/                  components
    index.ts             the feature's public surface
  shared/ui/             design-system primitives
  generated/             types generated from the backend's openapi.json
locales/                 es.json, en.json
```

Features: `auth`, `onboarding`, `home`, `trends`, `news`, `capital`, `budgets`, `goals`, `profile`, `settings`, `billing`, `assistant`.

| Layer | May import from |
| --- | --- |
| `domain` | itself and pure libraries |
| `application` | its feature's `domain`, and `src/core` |
| `infrastructure` | its feature's `domain`, `src/core`, `src/generated` |
| `ui` | its feature's `application` and `domain`, `src/shared/ui`, `src/core` |
| `app/` routes | a feature's `index.ts` only |
| another feature | only its `index.ts` |
| `src/core` | nothing in `features` |

dependency-cruiser enforces this (`bun run check:arch`), running on TypeScript 6.

### 7.3 Contract and performance

- `api/openapi.json` is vendored from the backend at a pinned version. `bun run api:generate` produces the types; CI fails when they are stale.
- The app never decides entitlements; it reads them from the session.

| Budget | Target |
| --- | --- |
| Cold start to an interactive Home, mid-range Android | ≤ 2.0 s |
| Screen transition to first frame | ≤ 100 ms |
| Lists and chart pan or zoom | 60 fps |
| Typecheck | ≤ 5 s |
| Lint | ≤ 2 s |
| Unit tests | ≤ 30 s |

### 7.4 Commands

`bun run start`, `ios`, `android`, `lint`, `format`, `typecheck`, `check:arch`, `check:structure`, `test`, `test:cov`, `e2e`, `api:generate`, `verify`. The same lefthook hooks as the backend.

## 8. Initial schema (schema `barras`)

A starting point, built from the owner's table list. Each module's spec refines its tables; `docs/schema/barras-schema.md` documents them table by table.

| Module | Tables |
| --- | --- |
| identity | `users`, `user_identities` (password, Apple, Google), `sessions` (hashed refresh tokens), `email_challenges`, `roles`, `permissions`, `role_permissions` |
| settings | `user_settings` (display currency, language, time zone, notification preferences, onboarding completed), `currencies`, `device_tokens` |
| billing | `plans`, `subscriptions`, `subscription_events` (store events, idempotent) |
| capital | `banks`, `exchanges`, `accounts` (where money is held), `categories`, `transactions` (income and expense), `investments`, `investment_movements`, `capital_snapshots` (one row per user per day, total and per class, written by a daily job), `receipts` |
| budgets | `budgets` (per category and period; spending is computed from `transactions`) |
| goals | `goals`, `goal_contributions` (each linked to the account that holds the money) |
| markets | `instruments`, `instrument_aliases`, `market_bars` (partitioned by month), `quotes_latest`, `trends`, `trend_history`, `favorites`, `watchlists`, `watchlist_items` |
| news | `news_sources`, `news_articles` (no body), `news_article_instruments` |
| indicators | `indicator_series`, `indicator_observations`, `fx_rates`, `bank_rates` |
| assistant | `assistant_conversations`, `assistant_messages` |
| operations | `ingest_runs` |

Seeds: the role `trader` and the permission catalog; the plans; the currency list; Bancolombia, Davivienda and Nu; Binance, Buda and the Colombian stock exchange; a demo user for local development.

## 9. Working rules (both repositories)

These adapt the owner's earlier `CLAUDE.md`. Each repository gets its own `CLAUDE.md`, `README.md` and `CONTRIBUTING.md` that restate them.

### Definition of done

A change is done only when the same pull request contains:

1. **Tests per layer.** Domain types tested directly. Use cases with fake ports. Mappers round trip in both directions. Routes through e2e against the real app. Every adapter to an outside system with a contract test. Errors asserted by `code`, never by message.
2. **OpenAPI metadata on every route**, including the error codes it can return and the permission and plan feature it needs.
3. **Docs.** The module `README.md` is current. A new or changed decision is an ADR, listed in the index.
4. **`verify` is green** locally and in CI.

Coverage (**Call**): 95% of lines in module crates and in frontend `domain` and `application`; 85% overall.

### Code style

`docs/standards/code-standard.md` is the owner's code standard, unchanged. Its text is saved next to this design, at `docs/standards/code-standard.md`, and is copied as is into both repositories. It is stack-agnostic. Where it and the module anatomy disagree on structure or naming, the module anatomy wins, as the standard itself says.

The earlier project's `CLAUDE.md` and `README.md` were given in the design conversation. They describe another project, so they are not copied into these repositories. Everything adopted from them is restated in sections 6, 7 and 9.

On top of the code standard:

- Rust: `unsafe` is forbidden. clippy runs with zero warnings and denies `unwrap`, `expect`, `panic!`, `dbg!` and printing to stdout outside tests. No `#[allow]` without the reason on the same line.
- TypeScript: no `any`, no non-null assertion to silence the compiler, no disabled lint rule without the reason on the same line.
- Files are snake_case in Rust and kebab-case in TypeScript.

### Git and pull requests

- Branch from `develop`: `type/BAR-<n>-short-slug`, at most 50 characters. `<n>` is the Trello card number. Types: `feat`, `fix`, `refactor`, `perf`, `test`, `docs`, `chore`, `ci`, `build`.
- Commit subject: `type(BAR-<n>): short description starting in lower case`. One ticket per branch.
- No AI attribution in commits or pull requests (carried over from the earlier project).
- Pull requests target `develop`. `main` and `develop` are protected: no direct push, no force push, merge only when the checks pass.
- Agents never push, open pull requests or touch production unless the owner asks in that conversation.

### CI

Three jobs in parallel on every pull request and on pushes to `develop` and `main`: `checks` (branch name, commit messages, format, lint, architecture, structure, and on the backend the sqlx metadata and OpenAPI drift), `test` (unit, integration and e2e with coverage), `build`.

### When something is not covered

Read the ADR for the area. If the decision does not exist, stop and ask, or propose an ADR. Do not invent a new pattern in one module.

## 10. Documentation layout

```
docs/
  README.md             the plan; start here
  specs/                one spec per module or feature, approved before any code
  adr/                  README.md (index) and NNNN-title.md
  analysis/             data sources, licensing, spikes, benchmarks
  schema/               barras-schema.md
  standards/            code-standard.md, module-anatomy.md, performance.md, testing.md
  backlog/work-plan.md  the order of work; mirrors Trello
  runbooks/
```

Product-wide documents (the plan, the schema, the contract ADRs) live in `barras-backend`. `barras-frontend` has the same folders for its own standards, ADRs and UI specs, and links to the backend for the contract.

First ADRs, each recording a decision in this document: architecture decision records; two repositories with OpenAPI as the contract; hexagonal layers enforced by crates; two deployables from one workspace; the HTTP contract; errors as typed results; data access; money, time and ids; authentication; authorisation and entitlements; realtime; outside systems as ports; ingestion jobs; performance budgets; observability; configuration; definition of done; git workflow; frontend stack and anatomy; market data sources; news as headline and link; informational suggestions and chatbot; subscriptions through the stores.

## 11. Backlog, ordered by the user flow

The Trello board BARRAS has status columns: ACT - DEV, ACT - DISEÑO, EN PROCESO, EN PRUEBA, APROBADA, PRODUCCIÓN. Cards are created in ACT - DEV, and design cards in ACT - DISEÑO, in the order below. Each title starts with its stage and repository so the order survives moving between columns. Ticket titles are in English, like the branches made from them.

Repository tags: BE backend, FE frontend, DOC docs, DS design, OWN the owner.

| # | Stage | Repo | Ticket |
| --- | --- | --- | --- |
| F-01 | Foundations | DOC | Seed `docs/` in both repositories: plan, standards, ADR index and first ADRs, schema, data-source analysis |
| F-02 | Foundations | BE | Workspace skeleton: toolchain, crates, lints, justfile, hooks, cargo-deny, xtask checks, CI, CLAUDE.md, README |
| F-03 | Foundations | BE | Core contract: errors, envelope, validation, configuration, logging, health, metrics, OpenAPI pipeline, e2e harness |
| F-04 | Foundations | BE | Database foundation: pools, migrations, `barras` schema, seed and reset, offline metadata check |
| F-05 | Foundations | BE | Redis and realtime foundation: pub/sub, websocket gateway, channels, replay |
| F-06 | Foundations | BE | Ingest foundation: job runner, advisory locks, `ingest_runs`, adapter template with contract tests |
| F-07 | Foundations | FE | App skeleton: Expo, router, TypeScript 7 typecheck, oxlint, bun, architecture check, Jest, Maestro, CI, CLAUDE.md, README |
| F-08 | Foundations | FE | Core contract: API client, generated types, query client with persistence, realtime client, i18n, configuration, secure storage |
| F-09 | Foundations | DS | Design system and screen designs delivered by the owner's designer; mapped to theme tokens |
| F-10 | Foundations | DOC | Spike: data licensing (Colombian stocks vendor, Binance and Buda display terms, El Espectador permission) |
| F-11 | Foundations | FE | Spike: candlestick chart library and its performance on a mid-range Android |
| F-12 | Foundations | BE | Hosting and environments ADR; container images; staging deploy |
| F-13 | Foundations | OWN | Accounts and identifiers: Apple Developer, Google Play, Expo, RevenueCat, bundle id, domain |
| A-01 | Sign up and sign in | DOC | Identity spec |
| A-02 | Sign up and sign in | BE | Users, roles and permissions; email sign-up with verification; sign-in; token refresh; sign-out; password reset |
| A-03 | Sign up and sign in | BE | Sign in with Apple and Google; account linking |
| A-04 | Sign up and sign in | BE | Authorisation: permission catalog, route guards, entitlements in the token |
| A-05 | Sign up and sign in | FE | Sign-in, sign-up, verify-email and forgot-password screens; session and token refresh |
| O-01 | Onboarding | BE | User settings and currencies: preferences and onboarding state |
| O-02 | Onboarding | FE | Onboarding slides, first preferences, first-visit guides per section |
| H-01 | Home | BE | TRM and currency-rate ingestion; currency conversion |
| H-02 | Home | DOC | Capital spec: accounts, investments, valuation, snapshots and the dashboard read model |
| H-03 | Home | BE | Banks, exchanges and accounts |
| H-04 | Home | BE | Investments and movements by class; valuation |
| H-05 | Home | BE | Daily capital snapshots |
| H-06 | Home | BE | Dashboard read model and its performance test |
| H-07 | Home | FE | Home: capital card with currency switch, change, breakdown, latest activity, live updates |
| P-01 | Profile and menu | BE | Profile, sessions, password change, account deletion |
| P-02 | Profile and menu | FE | Perfil: personalisation, plan, settings, security and privacy |
| P-03 | Profile and menu | FE | Menú: preferences, currency, notifications, language |
| P-04 | Profile and menu | BE+FE | Push notifications: device tokens, preferences, sender |
| T-01 | Trends | DOC | Markets, news and indicators spec |
| T-02 | Trends | BE | Instruments and exchanges catalog |
| T-03 | Trends | BE | Crypto ingestion: ticks, latest quotes, 1-minute bars |
| T-04 | Trends | BE | Colombian stocks end-of-day ingestion (only if F-10 ends with a licence) |
| T-05 | Trends | BE | Markets API: list, detail, bars, favourites, realtime channels |
| T-06 | Trends | BE | News ingestion and API, with the Lite limit |
| T-07 | Trends | BE | Trend scoring and suggestions API, with the Lite limit and the disclaimer |
| T-08 | Trends | BE | DANE and bank-rate ingestion; indicators API |
| T-09 | Trends | FE | Tendencias list: live prices, favourites, filters |
| T-10 | Trends | FE | Instrument detail: bars chart, news, favourite |
| T-11 | Trends | FE | News and suggestions with the Lite limit; official figures |
| C-01 | Capital | DOC | Income, expenses and budgets spec |
| C-02 | Capital | BE | Income, expenses and categories |
| C-03 | Capital | BE | Budgets per category and their progress |
| C-04 | Capital | FE | Capital overview; register an investment and its movements |
| C-05 | Capital | FE | Register an expense or income; categories |
| C-06 | Capital | FE | Budgets: create per category, progress |
| C-07 | Capital | BE+FE | Scan a receipt into an expense draft |
| G-01 | Goals | DOC | Goals spec |
| G-02 | Goals | BE | Goals: create, contribute from an account, progress and time, close, achieved |
| G-03 | Goals | FE | Metas screens |
| S-01 | Pro | DOC | Billing and assistant spec |
| S-02 | Pro | BE | Plans, store webhook, subscription state, entitlement change event |
| S-03 | Pro | FE | Paywall, Pro trial offer after onboarding, purchase and restore, switch to Lite, gated features |
| S-04 | Pro | BE+FE | Full history for bars and trends |
| S-05 | Pro | BE+FE | Custom stock tracker: watchlists and price alerts |
| S-06 | Pro | BE | Chatbot: conversations, streaming, context, guardrails, quotas |
| S-07 | Pro | FE | Chatbot in Tendencias |
| L-01 | Launch | DOC | Terms, privacy policy, disclaimers; legal review of suggestions and the chatbot |
| L-02 | Launch | BE+FE | Load test against the budgets; store listings and submission |

Notes on the order (**Calls**):

- **Home comes before Capital in the flow, but it reads capital data.** So the backend for accounts, investments and snapshots is built in the Home stage (H-02 to H-05), before the dashboard read model. The screens that record that data stay in the Capital stage. Until they land, Home shows its empty state and developers use the demo seed.
- **Home gains market-driven change when Trends lands.** Before that, investments are valued at the last price the user entered.
- **Colombian stock prices are conditional.** If F-10 ends without a licence, the first version ships with crypto, news and official figures. Stocks stay in the catalog so the user can record them, valued at the last price the user entered.
- **News is firm.** Headline and link need no permission.
- **The Pro trial is offered at the end of onboarding.** Until the Pro stage lands, onboarding skips the offer.
- **F-09 is an outside dependency.** The designs come from the owner's designer; frontend screen tickets from A-05 onward wait for the screens they need.
- **F-13 belongs to the owner.** Sign in with Apple and Google (A-03), store builds (F-07), hosting (F-12) and the store webhook (S-02) cannot be finished without those accounts, and Apple enrolment takes time.

## 12. Spec-driven workflow

1. A ticket starts with a spec in `docs/specs/BAR-<n>-<topic>.md`: intent, behaviour, routes, schema changes, error codes, tests.
2. The owner approves the spec.
3. An implementation plan is written from the spec.
4. The code is written test first, and `verify` passes.
5. The pull request updates the module README, the schema doc and any ADR it touches.

## 13. Left to the builder

- Exact dependency versions at scaffold time, starting from those verified on 2026-11-15: Rust 1.100, axum 0.8, tokio 1.54, sqlx 0.9, utoipa 6, jsonwebtoken 12, argon2 0.6, cargo-nextest 0.9, testcontainers 0.28; Expo SDK 58, React Native 0.88, TypeScript 6.0 and 7.0, oxlint 1.9x, bun 1.4, Jest 30.
- `async-trait` or native async traits.
- Hand-written fakes or mockall, case by case.
- Hosting provider and region (ADR in F-12).
- Email provider and push provider (identity and settings specs).
- The Colombian stocks vendor (after F-10) and the chart library (after F-11).
- The language model (assistant spec).
- The memory allocator, by benchmark.

## 14. Risks

1. No licensed news feed exists; El Espectador may refuse permission for more than headline and link.
2. Intraday Colombian stock prices need an exchange licence of unknown cost.
3. Whether Binance and Buda allow their data to be shown in a commercial app is unverified.
4. DANE publishes files, not an API; the adapter breaks when the file layout changes.
5. Suggestions or chatbot answers that drift toward personal advice become a regulated activity.
6. Tooling edges: Expo pins TypeScript 6 while the typecheck uses TypeScript 7; jsonwebtoken 12 and sqlx 0.9 are recent releases; the Skia package used by chart libraries may not match the one Expo bundles.
