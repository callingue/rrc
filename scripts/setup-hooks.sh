#!/bin/sh
#
# Points git at the version-controlled hooks in .githooks/ (one-time, per clone).
# Hooks in .git/hooks/ are not tracked, which is why they live here instead.
#
#   ./scripts/setup-hooks.sh

set -eu
cd "$(dirname "$0")/.."

git config core.hooksPath .githooks
chmod +x .githooks/*

echo "Git hooks enabled (core.hooksPath -> .githooks)."
echo "  pre-commit: cargo fmt --all --check"
echo "  pre-push:   cargo clippy -D warnings, then cargo nextest run --workspace"
echo
echo "Bypass once with --no-verify. Disable with: git config --unset core.hooksPath"
