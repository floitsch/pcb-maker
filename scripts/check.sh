#!/usr/bin/env bash
# Copyright (C) 2026 Toit contributors.
#
# The test set to run before a commit, in the checkout this script lives in:
# the unit and integration tests of the three core crates, the root
# package's three CLI tests, the benchmark runner's Python tests, then a
# release build. Prints each step's wall time and where the binary is.
# Needs kicad-cli on the PATH for the CLI tests (without it they say so and
# pass). Run it with the machine's load low: the walkthrough test lays out
# boards on a work budget.
#
#   bash scripts/check.sh

set -euo pipefail

root="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$root"

step() {
  local started
  started=$(date +%s)
  echo "== $*"
  "$@"
  echo "== $(($(date +%s) - started)) s: $*"
  echo
}

step cargo test --release -p pcb-router -p pcb-kicad -p pcb-placer
step cargo test --release -p pcb-maker --test cli_exit_codes --test agent_walkthrough --test native_verification
step python3 -m pytest -q benchmarks/agent-tasks
# A cargo test of the library crates does not refresh
# target/release/pcb-maker: build it last so the binary matches the tree
# (after the CLI tests this takes seconds).
step cargo build --release

echo "all checks passed on $(git rev-parse --short HEAD)$(git diff --quiet HEAD -- || echo ' (with uncommitted changes)')"
echo "binary: $root/target/release/pcb-maker"
echo "snapshot it with: bash scripts/snapshot.sh <n>"
