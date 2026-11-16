set shell := ["bash", "-euo", "pipefail", "-c"]

default:
    @just --list

setup:
    rustup toolchain install
    cargo install --locked cargo-nextest@0.9.152 cargo-deny@0.20.2 cargo-hakari@0.9.39 cargo-llvm-cov@0.9.1 watchexec-cli@2.8.0
    lefthook install

# Runs barras-api with the variables of .env; restarts it when a Rust source or a Cargo.toml changes
dev: (_watch "barras-api")

# Runs barras-ingest with the variables of .env; restarts it when a Rust source or a Cargo.toml changes
dev-ingest: (_watch "barras-ingest")

# A binary refuses to start without its variables, so a missing .env is named before anything
# builds. watchexec stops the binary with SIGTERM before each restart, so every reload goes
# through the clean shutdown.
_watch $binary:
    #!/usr/bin/env bash
    set -euo pipefail
    if [[ ! -f .env ]]; then
        echo 'copy .env.example to .env' >&2
        exit 1
    fi
    set -a
    source .env
    set +a
    exec watchexec --restart --filter '*.rs' --filter 'Cargo.toml' --ignore 'target/**' -- cargo run -p "$binary"

lint:
    cargo clippy --workspace --all-targets --locked --config 'build.warnings="deny"' -- -D warnings

fmt:
    cargo fmt --all

fmt-check:
    cargo fmt --all --check

deny:
    cargo deny --locked check

check-arch:
    cargo deny --locked check bans
    cargo xtask check-arch

check-structure:
    cargo xtask check-structure

# crates/workspace-hack is generated from the dependency tree and .config/hakari.toml (ADR-0024).
# cargo hakari reads the tree with a plain cargo metadata, which runs no build script but would
# rewrite a stale Cargo.lock.

# Regenerates crates/workspace-hack and every member's line to it, after any dependency change
hakari:
    cargo hakari generate
    cargo hakari manage-deps --yes

# Fails when crates/workspace-hack or a member's line to it is out of step, and fixes neither
hakari-check:
    cargo hakari generate --diff
    cargo hakari manage-deps --dry-run
    cargo hakari verify

# Rewrites openapi.json from the code; through a temporary file, so a failed build leaves it whole
openapi:
    mkdir -p target
    cargo run --locked --quiet -p barras-api -- openapi > target/openapi.json.new
    mv target/openapi.json.new openapi.json

openapi-check:
    cargo run --locked --quiet -p barras-api -- openapi --check openapi.json

# The tiers go by the test target's file name, so a new test has nothing to register.

# Unit tests and every test target not named e2e_ or int_, then the doctests; no Docker
test:
    cargo nextest run --workspace --locked -E 'not binary(/^(e2e|int)_/)'
    just test-doc

# Doctests run apart: nextest does not run them, and cargo-llvm-cov cannot measure them on stable
test-doc:
    cargo test --workspace --doc --locked

# The test targets named e2e_ or int_: a whole binary, in process or as a real process; from BAR-4 also adapters against real systems
test-int:
    cargo nextest run --workspace --locked -E 'binary(/^(e2e|int)_/)'

# The report is written afresh on every run and measures the whole workspace: -p, --exclude or
# an ignore regex would take code out of the measure. --no-cfg-coverage leaves cfg(coverage)
# unset, so no code can be compiled out of the instrumented build. cargo-llvm-cov does not
# create the folder of --output-path.

# Every test once under coverage, then the thresholds of ADR-0017
test-cov:
    mkdir -p target/coverage
    cargo llvm-cov nextest --workspace --locked --no-cfg-coverage --lcov --output-path target/coverage/lcov.info
    just _no-stray-profiles
    cargo xtask check-coverage target/coverage/lcov.info

# A process that lost LLVM_PROFILE_FILE writes its coverage profile into its working folder
# instead of target/, and its lines then count as never run, while the thresholds can still
# pass. Checked before them, so that a figure a lost profile lowered fails with its cause.
_no-stray-profiles:
    #!/usr/bin/env bash
    set -euo pipefail
    stray="$(find . \( -path ./target -o -path ./.git \) -prune -o -name '*.profraw' -print)"
    if [[ -n "$stray" ]]; then
        printf 'coverage profiles outside target/, from a process that lost LLVM_PROFILE_FILE; delete them once that is fixed:\n%s\n' "$stray" >&2
        exit 1
    fi

build:
    cargo build --workspace --release --locked

# In CI's order. check-arch, check-structure, hakari-check, fmt-check and deny compile no
# member, so they read the workspace before any build script of the change runs: cargo xtask
# builds only xtask, and cargo metadata runs no build script. hakari-check comes after
# check-arch, whose --locked fails on a stale Cargo.lock before cargo hakari's own cargo
# metadata could rewrite it.

# What CI runs, apart from the branch, commit and pull request checks and the tree check
verify: check-arch check-structure hakari-check fmt-check deny lint openapi-check test-cov test-doc build
