# ADR-0019: Frontend stack and feature anatomy

Status: Accepted
Date: 2026-11-15

## Context

- Expo is the one sensible way to build the app: the faster-bundler alternative, Re.Pack with Rspack, does not properly support Expo today and would cost Expo Router and EAS builds (section 5).
- The stack below is a set of **Calls** of the designer, not confirmed by the owner. Compile speed and a 2.0 s cold start to Home are goals.

## Decision

- Expo SDK 58 with Expo Router, development builds, EAS and React Compiler on. TypeScript strict with `noUncheckedIndexedAccess`; TypeScript 7 runs the typecheck.
- oxlint and oxfmt, bun, TanStack Query persisted to MMKV, Zustand, tokens in expo-secure-store, Unistyles 3, FlashList, Reanimated, i18next, react-hook-form with zod, RevenueCat, Jest and Maestro.
- The chart library is chosen in the spike F-11; writing our own on Skia is the fallback.
- `app/` holds routes only. `src/core` holds the contract, `src/features/<feature>` (kebab-case) holds `domain`, `application`, `infrastructure`, `ui` and `index.ts`, and `src/generated` holds the generated types.
- Layers import only what the spec's table allows. A route or another feature imports only a feature's `index.ts`, and `src/core` imports nothing in `features`.

## Consequences

- Cost: Expo pins TypeScript 6, so two TypeScript versions are installed.
- dependency-cruiser enforces the layers (`bun run check:arch`), running on TypeScript 6.
- The app never decides entitlements (ADR-0002).
