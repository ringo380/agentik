#!/usr/bin/env bash
# The full local gate - run before pushing. The pre-push hook calls this.
#
# This is everything the old five-job ci.yml did, minus the parts a Mac can
# actually run: fmt, clippy, check, and the test suite. What stayed in CI is
# the one thing this machine cannot do natively - proving Linux behavior.
#
# Usage:
#   ./scripts/check.sh          full gate
#   ./scripts/check.sh --fast   skip the release build
set -euo pipefail

cd "$(dirname "$0")/.."

FAST=0
[ "${1:-}" = "--fast" ] && FAST=1

# rustup installs into ~/.cargo/bin, which is missing from any environment that
# doesn't source the user's profile (git hooks, launchd, a stripped PATH). Add
# it here so the gate never half-runs and reports success.
[ -d "$HOME/.cargo/bin" ] && PATH="$HOME/.cargo/bin:$PATH"
command -v cargo >/dev/null 2>&1 || { echo "error: cargo not found in PATH" >&2; exit 1; }

echo "==> fmt"
cargo fmt --all -- --check

echo "==> clippy"
cargo clippy --all-targets -- -D warnings

echo "==> check"
cargo check --all-targets

echo "==> test"
cargo test --all

if [ "$FAST" -eq 0 ]; then
  echo "==> build (release)"
  cargo build --release
fi

echo "==> OK"
