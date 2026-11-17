set shell := ["bash", "-euo", "pipefail", "-c"]

default:
    @just --list

# sqlx-cli takes its history table's name from sqlx.toml, and only with its sqlx-toml feature:
# without it, every sqlx-cli command refuses to run beside that file. Every sqlx-cli command here
# runs with --no-dotenv: sqlx-cli would otherwise read a .env on its own, from this folder or one
# above, and print any line of it that it cannot parse, a password included. The commands that
# connect take DATABASE_URL from their environment instead.

setup:
    rustup toolchain install
    cargo install --locked cargo-nextest@0.9.152 cargo-deny@0.20.2 cargo-hakari@0.9.39 cargo-llvm-cov@0.9.1 watchexec-cli@2.8.0
    cargo install --locked sqlx-cli@0.9.0 --no-default-features --features postgres,sqlx-toml
    lefthook install

# Runs barras-api with the variables of .env; restarts it when a Rust source or a Cargo.toml changes
dev: (_watch "barras-api")

# Runs barras-ingest with the variables of .env; restarts it when a Rust source or a Cargo.toml changes
dev-ingest: (_watch "barras-ingest")

# watchexec stops the binary with SIGTERM before each restart, so every reload goes through the
# clean shutdown.
_watch binary: (_with-env "watchexec" "--restart" "--filter" "*.rs" "--filter" "Cargo.toml" "--ignore" "target/**" "--" "cargo" "run" "-p" binary)

# Runs the given command in place of this script, with the variables of .env exported. A binary
# refuses to start without its variables, so a missing .env is named before anything builds.
[positional-arguments]
_with-env +command:
    #!/usr/bin/env bash
    set -euo pipefail
    if [[ ! -f .env ]]; then
        echo 'copy .env.example to .env' >&2
        exit 1
    fi
    set -a
    # With a slash: bash's source looks a bare name up in the folders of PATH before this one, so
    # a .env in any of them would be read in place of the project's.
    source ./.env
    set +a
    exec "$@"

# The local database is the postgres service of compose.yaml, on 127.0.0.1:5433 (ADR-0007). The
# commands below that connect read their database from .env, as just dev does, so they act on
# whatever database it names: barras-api seed and reset refuse one that is not on this machine.

# compose.yaml interpolates no variable, so these read neither a compose.override.yaml, a
# COMPOSE_FILE nor .env: compose would otherwise parse .env and print a line it cannot read,
# password included.

# Starts the local Postgres of compose.yaml and waits until it accepts connections
db-up:
    docker compose --file compose.yaml --env-file /dev/null up --detach --wait postgres

# Stops the local Postgres; its data stays in the volume
db-down:
    docker compose --file compose.yaml --env-file /dev/null stop postgres

# Migrations are forward-only (spec §3.3), so the file is a simple one, with no script that
# undoes it, and its version is the moment in UTC. sqlx-cli takes any description and turns its
# blanks into underscores, so a name check-structure would refuse is refused first, before a file
# exists: snake_case as check-structure reads a module's name. `migrate add` refuses --no-dotenv,
# which sqlx-cli takes before the subcommand instead.

# Creates migrations/<UTC timestamp>_<name>.sql; the name is snake_case, such as create_users
db-migrate $name:
    #!/usr/bin/env bash
    set -euo pipefail
    snake_case='^[a-z][a-z0-9]*(_[a-z0-9]+)*$'
    if [[ ! $name =~ $snake_case ]]; then
        echo 'the name of a migration is snake_case, such as create_users: lower-case letters and digits, in words joined by single underscores' >&2
        exit 1
    fi
    exec sqlx --no-dotenv migrate add --source migrations --simple --timestamp "$name"

# Applies the pending migrations to the database of .env with barras-api migrate, the deploy step
db-deploy: (_with-env "cargo" "run" "--locked" "--quiet" "-p" "barras-api" "--" "migrate")

# Shows which migrations the database of .env has applied, which are pending, and which changed after they ran
db-status: (_with-env "just" "_migrate-info")

# sqlx-cli reads its database from DATABASE_URL, which .env does not set: it is mapped from
# API_DATABASE_URL into sqlx-cli's environment alone, never into an argument or the output, so
# that no process list or log shows it. The history table's name comes from sqlx.toml.
_migrate-info:
    #!/usr/bin/env bash
    set -euo pipefail
    if [[ -z ${API_DATABASE_URL:-} ]]; then
        echo '.env sets no API_DATABASE_URL' >&2
        exit 1
    fi
    export DATABASE_URL="$API_DATABASE_URL"
    exec sqlx migrate info --source migrations --no-dotenv

# Loads the seed files of seeds/ into the database of .env, which must be on this machine
db-seed: (_with-env "cargo" "run" "--locked" "--quiet" "-p" "barras-api" "--" "seed" "seeds")

# Rebuilds the database of .env, which must be on this machine: drop, create, migrate, seed
db-reset: (_with-env "cargo" "run" "--locked" "--quiet" "-p" "barras-api" "--" "reset" "seeds")

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
# .config/hakari.toml, this compares the crates and features of the release build of just build,
# the packages of release_packages, as cargo tree resolves them without dev units, with the same
# build in a copy of the workspace whose members do not depend on the hack, and fails on any
# difference, naming the crate and what the hack adds to it. The copy is the files git tracks or
# would track, in a temporary folder, so nothing is written in the repository.
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
        (cd "$1" && cargo tree --locked {{release_packages}} --exclude workspace-hack -e normal,build \
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

# The offline query metadata in .sqlx/ is read against a throwaway database, never the local one
# nor API_DATABASE_URL, so that what it holds follows from the migrations and the queries alone.

# Rewrites .sqlx/ from the queries, against a throwaway database with every migration applied
db-prepare: (_throwaway-db "cargo" "sqlx" "prepare" "--workspace" "--no-dotenv" "--" "--locked")

# Fails when a migration does not apply to an empty database, or when .sqlx/ is not what the queries and the migrations produce
db-check: (_throwaway-db "just" "_sqlx-check")

# cargo sqlx prepare --check fails on a query missing from .sqlx/ and on a file that differs,
# but only warns about a file that no query produces any more. sqlx-cli leaves what it
# regenerated in <target dir>/sqlx-prepare-check, so comparing the file names makes that fail
# too: it would otherwise stay committed after its query changed or went away.
_sqlx-check:
    #!/usr/bin/env bash
    set -euo pipefail
    cargo sqlx prepare --workspace --check --no-dotenv -- --locked
    target="$(cargo metadata --locked --format-version 1 --no-deps | sed -n 's/.*"target_directory":"\([^"]*\)".*/\1/p')"
    extra="$(comm -23 <(cd .sqlx && ls query-*.json | sort) <(cd "$target/sqlx-prepare-check" && ls query-*.json | sort))"
    if [[ -n "$extra" ]]; then
        printf 'in .sqlx/ but produced by no query, so run just db-prepare:\n%s\n' "$extra" >&2
        exit 1
    fi

# A Postgres from the image of compose.yaml's postgres service, on a port of 127.0.0.1 that Docker
# picks, with fsync off since its data never outlives it, the cluster name every local Postgres
# starts with (LOCAL_CLUSTER_NAME in crates/adapters/postgres/src/guard.rs, whose tests read this
# file), and every migration applied: runs the given command with DATABASE_URL set to it, then
# removes the container, whatever happened. The label barras-throwaway=1 finds one that a killed
# run left behind.
[positional-arguments]
_throwaway-db +command:
    #!/usr/bin/env bash
    set -euo pipefail
    # Read by the service, as the testkit reads it, so that another service changes nothing; and
    # from compose.yaml alone, as the testkit does: --file leaves out a compose.override.yaml and
    # a COMPOSE_FILE variable, and --env-file /dev/null leaves out .env, which compose would read
    # too, printing a line it cannot parse, a password included.
    image="$(docker compose --file compose.yaml --env-file /dev/null config --images postgres)"
    # Pinned as the testkit requires: a digest, and a tag after the name's last slash.
    named="${image%@sha256:*}"
    if [[ $named == "$image" || ${named##*/} != *:* ]]; then
        echo 'the postgres service of compose.yaml must name its image pinned by tag and digest' >&2
        exit 1
    fi
    container="$(docker create --label barras-throwaway=1 --publish 127.0.0.1::5432 \
        --env POSTGRES_USER=barras --env POSTGRES_PASSWORD=barras --env POSTGRES_DB=barras \
        "$image" postgres -c fsync=off -c cluster_name=barras-local)"
    # Set before the container starts, so that nothing leaves it running; a signal ends the
    # script through it too, once the command it runs has been stopped.
    trap 'docker rm --force --volumes "$container" >/dev/null' EXIT
    command=''
    stop() {
        if [[ -n $command ]]; then
            kill -TERM -- "-$command" 2>/dev/null || true
        fi
        exit "$1"
    }
    trap 'stop 130' INT
    trap 'stop 143' TERM
    docker start "$container" >/dev/null
    # Over TCP: while the image creates the database, a temporary server answers on its Unix
    # socket alone, then stops.
    deadline=$((SECONDS + 60))
    until docker exec "$container" pg_isready --host=127.0.0.1 --username=barras --dbname=barras >/dev/null 2>&1; do
        if (( SECONDS >= deadline )); then
            echo 'the throwaway database did not accept connections within 60 seconds' >&2
            exit 1
        fi
        sleep 0.2
    done
    port="$(docker port "$container" 5432/tcp)"
    export DATABASE_URL="postgres://barras:barras@127.0.0.1:${port##*:}/barras"
    sqlx migrate run --source migrations --no-dotenv
    # bash runs a trap only after the command in the foreground ends, so the command runs in the
    # background while the script waits for it: a signal then reaches the trap at once. set -m
    # gives the command a process group of its own, which the trap stops whole, with the
    # compilers cargo started and not only the first process.
    set -m
    "$@" &
    command=$!
    set +m
    wait "$command"

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

# What a release build builds: the workspace without barras-testkit and xtask, which only the
# tests and the repository's checks use. cargo unifies the features of every package it builds,
# so with the testkit the binaries took testcontainers' features, such as hyper-util's http2 and
# server-auto, and barras-api's public port answered cleartext HTTP/2 (plan, refinement 13).
# _hack-release-graph compares the release build of this same selection.
release_packages := "--workspace --exclude barras-testkit --exclude xtask"

build:
    cargo build {{release_packages}} --release --locked

# In CI's order. check-arch, check-structure, hakari-check, fmt-check and deny compile no
# member, so they read the workspace before any build script of the change runs: cargo xtask
# builds only xtask, and cargo metadata and cargo tree run no build script.

# What CI runs, apart from the branch, commit and pull request checks and the tree check
verify: check-arch check-structure hakari-check fmt-check deny lint openapi-check db-check test-cov test-doc build
