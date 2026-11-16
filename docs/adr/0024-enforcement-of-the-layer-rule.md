# ADR-0024: Enforcement of the layer rule

Status: Accepted
Date: 2026-11-15

## Context

- ADR-0003 sets who may depend on whom and says checks enforce it. A name on a list proves little on its own: a `git` or `path` entry, another registry, or a `[patch]` table can put other code behind an allowed name, and a crate folder missing from `members` is never built or checked.
- Tests need a few crates that the lists of ADR-0003 leave out, such as a mocking library.
- The table of spec §6.4 lets `apps/*` depend on everything, so the stricter rule below began as a **Call** of the designer. The owner confirmed it on 2026-11-15, in the review of the BAR-3 spec: `apps/*` and `barras-testkit` never depend directly on `axum` or `reqwest` (BAR-3 spec §3.1, Decided).
- Approving the BAR-3 spec the same day accepted the amendments it lists: `barras-kernel` only at its member's path, the `workspace-hack` exception, and the bans on `axum-core`, `hyper`, `hyper-util` and the alternative crates (BAR-3 spec §3.6, §3.7, §12). cargo-hakari was adopted then (BAR-3 spec §3.7, Decided): it generates a crate that the members depend on, `xtask` and `barras-testkit` aside, and that the lists must accept without letting code in.
- The reviews of the gates while building BAR-3, after that approval, found ways to slip the first rules. The rules that close them are **Calls of the implementation**: `package-name`, `path-outside-members`, `apps-are-roots`, `hack-isolation`, `hack-is-a-stub`, `no-build-script`, `testing-dev-only`, the ban on `utoipa-axum` itself, the ban on axum's `json`, `query` and `form` features, `[graph] all-features`, the testkit and the four third-party crates left out of hakari's traversal, and the comparison of the release build in `just hakari-check`.

## Decision

### Members

- A dependency on a workspace member is recognised by the member's package name and judged by the member's folder, never by a name written in a rule.
- The package in each known folder has its expected name: `crates/kernel` is `barras-kernel`, `crates/modules/<m>` is `barras-<m>`, `crates/adapters/<a>` is `barras-adapter-<a>`, `apps/<a>` is `barras-<a>`, `crates/bootstrap` is `barras-bootstrap`, `crates/testkit` is `barras-testkit`, `crates/workspace-hack` is `workspace-hack` and `crates/xtask` is `xtask`. The cargo-deny bans name crates, so this ties each name to its folder, and renaming a package slips no rule.
- Every folder directly under `apps/`, `crates/modules/` and `crates/adapters/` that holds a `Cargo.toml`, and `crates/kernel`, `crates/bootstrap`, `crates/testkit`, `crates/workspace-hack` and `crates/xtask`, is a workspace member. A crate in any other folder is refused.
- A path dependency whose folder is not a member's folder is refused, in any crate and any kind of dependency: a crate outside the members is never checked, so it could pass any member on.
- No crate depends on a crate under `apps/`, in any kind of dependency.
- No `[patch]` or `[replace]` table in the root `Cargo.toml`, and no `[patch]` table in `.cargo/config.toml`. Needing one changes this rule through a new ADR.

### The lists

- An allowed member counts only as the path dependency at that member's own folder: `barras-kernel` at `crates/kernel`, `workspace-hack` at `crates/workspace-hack`. Every other allowed name counts only from crates.io.
- The kernel's normal and build dependencies come only from `thiserror`, `uuid`, `time`, `rust_decimal` and `workspace-hack`. A module's come only from `barras-kernel`, those five and `async-trait`.
- The dev dependencies of the kernel and of a module may also use `tokio`, `mockall` and `insta`.
- The kernel and the modules have no build script: the code it generated would escape the coverage measure (ADR-0027), and pure domain code needs none.
- An adapter depends on no other adapter. `xtask` depends on no workspace member, `workspace-hack` included. `barras-testkit` is only ever a dev dependency. ADR-0026 adds the rules of `crates/bootstrap`.

### Test support

- A member's test support lives behind a feature named `testing`, such as the kernel's `FixedClock` and the HTTP adapter's test client. A crate turns on a member's `testing` feature only in a dev-dependency.
- A crate's own `[features]` table turns on `testing`, its own or a member's (`<dep>/testing`, `<dep>?/testing`), only from its own `testing` feature, never from `default` or any other feature.
- The testkit gets no exemption: `cargo build --workspace` unifies the features of every member it builds, so a normal dependency of the testkit that turned the feature on would carry the test support into the release binaries.

### The generated crate

- cargo-hakari generates `crates/workspace-hack` from the dependency tree and `.config/hakari.toml`: one crate that depends on the union of the third-party dependencies and their features, so that two commands asking for different features do not build a shared dependency twice. Every member depends on it but the two left out of hakari's traversal, `xtask` and `barras-testkit`.
- It may carry no code: exactly one library, at `src/lib.rs`, that holds doc comments at most; at most one build script, at `build.rs`, that holds only `fn main() {}`; and in its folder no other `.rs` file, no symlink and no entry that is neither a file nor a folder. Every dependency it has comes from crates.io. The lists accept it only because of these two rules.
- Each member that depends on it names it once, with `use workspace_hack as _;` in `lib.rs`, or in `main.rs` for a binary without a library, so that cargo never reports it as an unused dependency and the lint stays on for every real one. The generated crate allows that lint for itself.
- `xtask` and `barras-testkit` are left out of hakari's traversal. The repository tool neither depends on a member nor shapes what the others build. hakari counts a member's normal dependencies as production, so through the generated crate the testkit's would reach the binaries' release builds.
- hakari simulates every member's builds with its dev-dependencies and all its features, `testing` included, and has no option to leave either out, so the generated crate could carry a feature only tests ask for into every release build. Four third-party crates are left out of its traversal (`[traversal-excludes]`), each for its own reason: `garde`, whose `derive` and `email` only the HTTP adapter's test fixtures use; `hyper-util`, whose `client-legacy` only the test client uses; `tokio`, whose `test-util` pauses the clock in the HTTP adapter's tests; and `hyper`, a banned crate, so that the generated crate never takes it directly. A crate that gains such a feature joins that list, never `[final-excludes]`, which would drop only its line: hakari would still unify the rest as if it were there.
- `just hakari-check` holds this. On each platform of `.config/hakari.toml`, it compares the crates and features of the workspace's release build, as `cargo tree` resolves them without dev units, with the same build in a temporary copy of the workspace whose members do not depend on the generated crate, and fails on any difference, naming the crate and the feature. It reads `Cargo.lock` with `--locked` first, so a stale lock fails it instead of being rewritten by cargo-hakari's own plain `cargo metadata`, and it writes nothing in the repository.
- When a banned crate enters the generated crate, `workspace-hack` joins that ban's wrappers in `deny.toml`, with a comment that says why. A crate still cannot `use` what its own `Cargo.toml` does not name, so the layer rule holds.

### The cargo-deny bans

- Only these crates depend directly on a technology crate:
  - `sqlx`: `barras-adapter-postgres` and `barras-testkit`.
  - `axum`: `barras-adapter-http`, `barras-adapter-realtime`, and `utoipa-axum`, which builds the router and the OpenAPI document from the same handlers.
  - `axum-core`, `hyper` and `hyper-util`: the HTTP and realtime adapters, and the third-party crates that wrap them (`axum`; `hyper-util` for `hyper`), so that no crate serves HTTP, or calls out, past the `axum` ban.
  - `utoipa-axum`: `barras-adapter-http`, so that no crate reaches `axum` through it.
  - `reqwest`: `barras-adapter-providers`.
  - `redis`: `barras-adapter-redis` and `barras-testkit`.
- So `apps/*` reach these crates through the adapters, never directly.
- No crate, an adapter included, depends on an alternative to the crates the ADRs chose: the web frameworks `actix-web`, `rocket`, `warp`, `poem`, `salvo` and `tide`; the database layers `diesel`, `sea-orm` and `tokio-postgres`; the HTTP clients `ureq`, `surf`, `isahc` and `curl`.
- No crate turns on axum's `json`, `query` or `form` feature, nor its `default`, which turns on all three (`[[bans.features]]`). They bring axum's own `Json`, `Query` and `Form` extractors, whose rejections are not envelopes and quote the caller's input; a handler takes its input through the HTTP adapter's validating extractors (ADR-0029). `default-features = false` in every crate keeps them off, and the ban keeps them off whatever a crate writes.
- A `*` version requirement is refused (`wildcards = "deny"`). A path dependency between our own unpublished crates has no version to give, and is allowed (`allow-wildcard-paths`).
- cargo-deny reads the graph with every feature of every member turned on (`[graph] all-features = true`), so a banned crate cannot sit unseen behind an optional dependency that no default feature turns on.

## Consequences

- Cost: a crate from git or another registry, or a local patch, waits for an ADR, even for a quick fix.
- Cost of cargo-hakari: on a cold build every crate, the kernel included, waits for the union of the third-party dependencies; an incremental check is not affected. The four third-party crates left out of the traversal are not unified: a test build and a release build each compile them as cargo resolves them, as without hakari. Three gates needed an exception for the generated crate: the lists of `check-arch`, cargo's unused-dependency lint and the cargo-deny bans.
- An app that needs a type of a technology crate gets it through its adapter, which re-exports what the composition root needs.
- `cargo xtask check-arch` reads `cargo metadata` and the manifests, and names the rule each failure breaks: `unknown-location`, `not-a-member`, `package-name`, `path-outside-members`, `apps-are-roots`, `no-patch`, `kernel-allowlist`, `kernel-dev-allowlist`, `module-allowlist`, `module-dev-allowlist`, `no-build-script`, `adapter-isolation`, `tool-isolation`, `testkit-dev-only`, `testing-dev-only`, `hack-isolation` and `hack-is-a-stub`, with `bootstrap-isolation` and `bootstrap-apps-only` of ADR-0026. `cargo deny check bans` enforces the bans, the feature ban included. `just check-arch` runs both.
- `just lint` can fail on a member that does not name the generated crate. `just hakari` regenerates the crate and every member's line to it; `just hakari-check` fails while either is out of step, or while the generated crate changes a release build. `just verify` runs it, and CI in a step of its own, so that no other failure hides it.
