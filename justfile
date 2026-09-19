# Project commands. `just --list` prints these with their descriptions.

# Show the available recipes.
default:
    @just --list

# Everything CI checks, in the order CI runs it.
ci: fmt-check lint machete test

# Reformat the workspace.
fmt:
    cargo fmt --all

# Fail if anything is unformatted.
fmt-check:
    cargo fmt --all --check

# Lint, treating warnings as errors. Extra arguments go to clippy: `just lint --fix`.
lint *args:
    cargo clippy --workspace --all-targets {{ args }} -- -D warnings

# Report dependencies that are declared but never used.
machete:
    cargo machete

# Run the test suite. Extra arguments go to cargo test: `just test simple`.
test *args:
    cargo test --workspace {{ args }}

# Supervise the services in DIR until interrupted.
run dir="examples/services":
    cargo run -p rrc-sup -- {{ dir }}

# Point git at the repository's hooks (one-time, per clone).
hooks:
    ./scripts/setup-hooks.sh
