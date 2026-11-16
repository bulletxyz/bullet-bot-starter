# Strategies for the Bullet bot, built into WASM components a bot runs as plugins.
# `just` lists the recipes.

default:
    @just --list

# Every strategy (or the named ones) to dist/<name>.wasm, with its sha256.
build *names:
    scripts/build.sh {{names}}

# Native tests: the SDK, the examples, the built-ins and their parity replay.
test:
    cargo test --locked --workspace

# A new strategy in strategies/<name>/, from templates/strategy.
new name:
    scripts/new-strategy.sh {{name}}

# Clippy over everything, warnings as errors.
lint:
    cargo clippy --locked --workspace --all-targets -- -D warnings

fmt:
    cargo fmt --all

# What CI runs: format check, lint, tests, build.
ci:
    cargo fmt --all --check
    just lint
    just test
    just build
