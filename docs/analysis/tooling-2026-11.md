# Tooling Check, November 2026

Versions read on 2026-11-15 from crates.io, the npm registry and the Rust stable channel manifest. Qualitative notes come from release notes and docs. Pin what is current at scaffold time, starting from these.

## Rust backend

| Tool | Latest stable | Use | Caveat |
| --- | --- | --- | --- |
| Rust and edition | 1.100.0 (2026-11-12), edition 2024 | yes | Cargo 1.100 adds a built-in unused-dependencies lint |
| axum | 0.8.9 | yes | 0.9 is unreleased, with breaking changes queued |
| tokio, hyper | 1.54.1, 1.12.0 | yes | none |
| tower, tower-http | 0.5.3, 0.7.1 | yes | axum 0.8.9's own optional dependency is tower-http ^0.6.8 |
| sqlx and sqlx-cli | 0.9.0 | yes | Offline mode (`cargo sqlx prepare`, `.sqlx`, `--check`) is still the documented path. MSRV 1.94. New `sqlx.toml`. `query()` takes only `&'static str` or `AssertSqlSafe` |
| Postgres alternatives | clorinde 2.0.0, tokio-postgres 0.7.18, diesel 2.4.0 with diesel-async 0.10.0 | maybe | Clorinde generates code ahead of time but has tiny adoption |
| Websockets | axum `ws` (tokio-tungstenite ^0.29) | yes | fastwebsockets is stale. For fan-out: `tokio::sync::broadcast` per node plus Redis (1.8.2) or NATS across nodes |
| OpenAPI | utoipa 6.0.0 with utoipa-axum 0.3.0 | yes | Targets axum ^0.8.4; MSRV 1.88. aide 0.15.1 is a maybe |
| Errors and tracing | thiserror 2.0.21, anyhow 1.0.104, tracing 0.1.44 | yes | none |
| OpenTelemetry and metrics | opentelemetry 0.33.1 with tracing-opentelemetry 0.34.0; metrics 0.24.6 with its Prometheus exporter 0.18.3 | yes | OpenTelemetry is pre-1.0; pin matching versions |
| Configuration and validation | config 0.15.27; garde 0.23.0 or validator 0.21.0 | yes | figment has had no release since May 2024 |
| Auth | argon2 0.6.0 with password-hash 0.6.1; jsonwebtoken 12.0.0 | yes | jsonwebtoken 12 is new; pick the `aws_lc_rs` or `rust_crypto` backend |
| Apple and Google ID tokens | jsonwebtoken with jsonwebtoken-jwks-cache 0.4.0 or jwks_client_rs 0.6.14; openidconnect 4.1.0 | maybe | No dominant crate; issuer, audience and nonce checks are ours |
| Testing | cargo-nextest 0.9.152, testcontainers 0.28.0, mockall 0.15.0, insta 1.49.0, cargo-llvm-cov 0.9.1 | yes | none |
| Linkers | lld (default on x86_64 Linux since 1.90); mold 3.0.0; wild 0.11.0 | yes on Linux | On macOS use Apple's default linker |
| Cranelift | nightly only | no | Not in the stable manifest |
| Build caches | sccache 0.19.0, cargo-chef 0.1.78, cargo-hakari 0.9.39 | yes | Since 1.99 Cargo disables incremental builds when `CI` is set |
| Lint and supply chain | clippy and rustfmt (bundled); cargo-deny 0.20.2; cargo-machete 0.9.2 or cargo-shear 1.14.1 | yes | cargo-udeps needs nightly |
| Architecture rules | cargo_pup 0.1.9 | no | Needs a pinned nightly. Crate boundaries plus cargo-deny bans are the stable option |

## React Native app

| Tool | Latest stable | Use | Caveat |
| --- | --- | --- | --- |
| React Native and Expo SDK | 0.88.1 and SDK 58 (58.0.25) | yes | React 19.3. New Architecture is the only one since 0.82. Hermes V1 is default. Node 22.13 or later |
| React Compiler | 1.0.0 | yes | On in Expo's default template |
| TypeScript | 7.0.2 (native, in Go) | for the typecheck | 8 to 12 times faster. No programmatic API until 7.1. Expo templates pin `~6.0.3` |
| Metro | 0.88.0 | yes | SDK 58 has an experimental Rust transformer, disabled by custom Babel plugins |
| Re.Pack with Rspack | 5.4.3 and 2.3.2 | no with Expo | Docs cover only Expo Modules in bare apps |
| oxlint and oxfmt | 1.93.0 and 0.78.0 | yes | Type-aware linting is stable through oxlint-tsgolint. Hooks rules are native. JS plugins are alpha |
| Biome | 2.6.8 | maybe | Type-aware rules are still in nursery |
| ESLint and Prettier | 10.15.0 and 3.10.1 | maybe | typescript-eslint requires TypeScript below 6.1 |
| Package manager | bun 1.4.4, pnpm 12.21.0 | yes | Both are supported; EAS picks by lockfile |
| Unit tests | Jest 30.5.2 with jest-expo and React Native Testing Library 14.1.0 | yes | Vitest has no official React Native support |
| End to end | Maestro CLI 2.12.1 | yes | Detox is the alternative |
| Core libraries | expo-router 58, TanStack Query 5.107, Zustand 5.1, MMKV 4.3, expo-secure-store, zod 4.6, react-hook-form 7.92, Reanimated 4.7, i18next 27 with expo-localization | yes | MMKV needs Nitro Modules and a development build |
| Styling | Unistyles 3.8.1, NativeWind 4.2.7, Tamagui 3.3.3 | maybe | NativeWind 5 is still a release candidate |
| Lists | FlashList 2.3.3, Legend List 3.7.2 | yes | none |
| Candlestick charts | victory-native 43.0.1 (Skia), react-native-livechart 4.30.2 (Skia), wagmi-charts 3.0.1 (SVG) | spike needed | Skia moved to the unscoped `react-native-skia` package; Expo 58 bundles `@shopify/react-native-skia` 2.14.0 |
| Sign-in | expo-apple-authentication 58; google-signin 17.0.0 | yes | The free Google package uses the legacy Android SDK |
| Subscriptions | react-native-purchases 11.0.2 (RevenueCat); expo-iap 6.4.4 | yes; maybe | RevenueCat gives server validation and webhooks |
| Import boundaries | dependency-cruiser 18.7.2, eslint-plugin-boundaries 7.4.3 | maybe | dependency-cruiser needs TypeScript 7.1 to run on TypeScript 7, so it runs on TypeScript 6 |

## Could not be verified

- Whether lld is the default linker on aarch64 Linux.
- mold 3.0 on macOS.
- Static Hermes: no official shipping status.
- Any Expo statement on TypeScript 7.
- Re.Pack with EAS Build and Update.
- eslint-plugin-boundaries under oxlint.
- The `Candlestick` documentation of victory-native 43.
- React Compiler lint rules under oxlint.

## Sources

Rust 1.100 release notes; axum, sqlx, utoipa and jsonwebtoken changelogs; the Expo SDK 58 changelog; the TypeScript 7.0 announcement; the oxlint type-aware linting announcement; the Biome linter domains page; the Re.Pack Expo Modules guide; the Expo guide on bun.
