set shell := ["bash", "-euo", "pipefail", "-c"]

default:
    @just --list

setup:
    rustup toolchain install
    cargo install --locked cargo-nextest@0.9.152 cargo-deny@0.20.2 cargo-hakari@0.9.39 cargo-llvm-cov@0.9.1 watchexec-cli@2.8.0
    cargo install --locked sqlx-cli@0.9.0 --no-default-features --features postgres
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
# rewrite a stale Cargo.lock: hakari-check reads it with --locked first, so it fails on one
# instead.

# Regenerates crates/workspace-hack and every member's line to it, after any dependency change
hakari:
    cargo hakari generate
    cargo hakari manage-deps --yes

# Fails when crates/workspace-hack or a member's line to it is stale, or the hack changes a release build; writes nothing
hakari-check:
    cargo metadata --locked --format-version 1 >/dev/null
    cargo hakari generate --diff
    cargo hakari manage-deps --dry-run
    cargo hakari verify
    just _hack-release-graph

# Every member depends on crates/workspace-hack, so whatever it holds reaches each member's
# release build, while hakari unifies the features of test builds too. For each platform of
# .config/hakari.toml, this compares the crates and features of the workspace's release build,
# as cargo tree resolves them without dev units, with the same build in a copy of the workspace
# whose members do not depend on the hack, and fails on any difference, naming the crate and
# what the hack adds to it. The copy is the files git tracks or would track, in a temporary
# folder, so nothing is written in the repository.
_hack-release-graph:
    #!/usr/bin/env bash
    set -euo pipefail
    root="$(pwd -P)"
    platforms="$(awk '
        /^[[:space:]]*platforms[[:space:]]*=/ { listing = 1 }
        listing {
            line = $0
            sub(/#.*/, "", line)
            while (match(line, /"[^"]*"/)) {
                print substr(line, RSTART + 1, RLENGTH - 2)
                line = substr(line, RSTART + RLENGTH)
            }
            if (index(line, "]")) exit
        }' .config/hakari.toml)"
    if [[ -z "$platforms" ]]; then
        echo '.config/hakari.toml names no platform to compare the release build on' >&2
        exit 1
    fi
    copy="$(mktemp -d)"
    trap 'rm -rf "$copy"' EXIT
    copy="$(cd "$copy" && pwd -P)"
    git ls-files -z --cached --others --exclude-standard \
        | while IFS= read -r -d '' file; do if [[ -e $file ]]; then printf '%s\0' "$file"; fi; done \
        | tar --null -T - -cf - | tar -xf - -C "$copy"
    (cd "$copy" && cargo hakari remove-deps --yes --quiet)
    # One line per crate and set of features; the members, the hack among them, are left out.
    graph() {
        (cd "$1" && cargo tree --locked --workspace --exclude workspace-hack -e normal,build \
            --target "$2" --prefix none -f '{p} [{f}]') \
            | sed 's/ (\*)$//' | awk -v member="($1/" 'index($0, member) == 0' | sort -u
    }
    failed=0
    for target in $platforms; do
        with="$(graph "$root" "$target")"
        without="$(graph "$copy" "$target")"
        if [[ "$with" == "$without" ]]; then
            continue
        fi
        failed=1
        awk -v target="$target" '
            function crate(line) { sub(/ \[[^]]*\]$/, "", line); return line }
            function features(line) { sub(/^.* \[/, "", line); sub(/\]$/, "", line); return line }
            FNR == NR { before[crate($0)] = before[crate($0)] "," features($0) ","; next }
            {
                name = crate($0)
                if (!(name in before)) {
                    printf "%s: the workspace-hack brings %s into the release build, with [%s]\n", target, name, features($0)
                    next
                }
                count = split(features($0), list, ",")
                added = ""
                for (i = 1; i <= count; i++)
                    if (index(before[name], "," list[i] ",") == 0) added = added (added == "" ? "" : ", ") list[i]
                if (added != "") printf "%s: the workspace-hack adds %s to %s in the release build\n", target, added, name
            }' <(printf '%s\n' "$without") <(printf '%s\n' "$with") >&2
        diff <(printf '%s\n' "$without") <(printf '%s\n' "$with") | grep '^[<>]' | sed 's/^</  without:/; s/^>/  with:   /' >&2 || true
    done
    if [[ $failed -ne 0 ]]; then
        echo 'A release build must not change: if only tests ask for this, list the crate under third-party in [traversal-excludes] of .config/hakari.toml and run just hakari; only hakari writes crates/workspace-hack/Cargo.toml.' >&2
    fi
    exit "$failed"

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
# builds only xtask, and cargo metadata and cargo tree run no build script.

# What CI runs, apart from the branch, commit and pull request checks and the tree check
verify: check-arch check-structure hakari-check fmt-check deny lint openapi-check test-cov test-doc build
