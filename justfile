# Project commands. `just --list` prints these with their descriptions.

# Show the available recipes.
default:
    @just --list

# Install dev tools used by this project
setup:
    #!/usr/bin/env bash
    set -euo pipefail
    if command -v cargo-binstall >/dev/null; then
        cargo binstall -y cargo-nextest cargo-machete
    else
        cargo install --locked cargo-nextest cargo-machete
    fi

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
machete: (need "cargo-machete")
    cargo machete

# pkg is a package name from Cargo.toml (rrc-sup), not a directory (sup).
# Everything after pkg is forwarded to nextest verbatim.
#
#   just test                          all workspace packages
#   just test rrc-sup                  one package
#   just test rrc-sup --no-capture     one package, extra nextest flags
[doc('Run the test suite')]
test pkg="" *args: (need "cargo-nextest")
    cargo nextest run {{ if pkg == "" { "--workspace" } else { "-p" + pkg } }} {{ args }}

# Supervise the services in DIR until interrupted.
run dir="examples/services":
    cargo run -p rrc-sup -- {{ dir }}

# Point git at the repository's hooks (one-time, per clone).
hooks:
    ./scripts/setup-hooks.sh

[private]
need tool:
    @{{ assert(shell("command -v $1 || true", tool) != "", tool + " not found -- run `just setup`") }}
