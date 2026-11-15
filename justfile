set shell := ["bash", "-euo", "pipefail", "-c"]

default:
    @just --list

setup:
    rustup show active-toolchain
    cargo install --locked cargo-nextest@0.9.152 cargo-deny@0.20.2
    lefthook install

dev:
    cargo run -p barras-api

dev-ingest:
    cargo run -p barras-ingest

lint:
    cargo clippy --workspace --all-targets --locked -- -D warnings

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

test:
    cargo nextest run --workspace --locked

build:
    cargo build --workspace --release --locked

verify: fmt-check lint deny check-arch check-structure test build
