#!/usr/bin/env bash
# Copyright (C) 2026 Toit contributors.
#
# The test set to run before a commit, in the checkout this script lives in.
#
#   bash scripts/check.sh                   # everything (the default; CI's nightly job: --full)
#   bash scripts/check.sh --fast            # the gate: about 30 s of tests once built
#   bash scripts/check.sh --full            # the same as no flag, said explicitly
#   bash scripts/check.sh --no-walkthrough  # everything but the agent walkthrough
#
# Every mode runs:
#   - the unit and integration tests of every crate of the workspace
#     (pcb-router, pcb-kicad, pcb-placer, pcb-global, pcb-congestion, pcb-topo,
#     tools/config-reference) and the root binary's own unit tests;
#   - the root package's CLI tests: exit codes, native verification, the
#     router's fingerprints at 1 and 4 threads, the generated config
#     reference against the code, hostile input files, and the fixture
#     tests on the committed boards of tests/fixtures (docs/handover.md,
#     "Before a commit");
#   - the benchmark runner's, the harvest's, the expansion intake's and
#     the nightly's Python tests (benchmarks/agent-tasks, benchmarks/github,
#     benchmarks/expansion, scripts);
#   - clippy over the whole workspace and every target, warnings as errors
#     (CI's lint gate);
#   - a release build last, and prints each step's wall time and where the
#     binary is.
# The default (and --full) adds the slow tests that depend on the
# machine's load: the agent walkthrough (three layouts with KiCad in the
# loop, 30 to 80 s) and the placer's tests/placement.rs (8 to 20 s).
#
# kicad-cli (KiCad 10) on the PATH runs the tests that need it (the CLI
# test's KiCad case, the walkthrough, the fixture tests of KiCad's
# connectivity and pours); without it they say so and pass, and this
# script says so first. Run it with the machine's load low, and under the
# machine's KiCad lock where there is one (the tests run kicad-cli).

set -euo pipefail

mode=full
walkthrough=(--test agent_walkthrough)
case "${1:-}" in
  --fast) mode=fast; walkthrough=() ;;
  --full | "") ;;
  --no-walkthrough) walkthrough=() ;;
  *) echo "usage: $0 [--fast | --full | --no-walkthrough]" >&2; exit 2 ;;
esac

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

if command -v kicad-cli > /dev/null 2>&1; then
  echo "kicad-cli $(kicad-cli version 2> /dev/null || echo '(version unknown)'): the KiCad tests run"
else
  echo "kicad-cli is not on the PATH: the KiCad tests skip (cli_exit_codes' KiCad case, the walkthrough,"
  echo "the fixture tests kicad_agrees_with_the_routers_connectivity_on_f1 and the_routed_f1s_pours_are_kicads)"
fi
echo

# The placer's placement.rs lays boards out on a work budget: --fast
# leaves it to the full run.
placer=(-p pcb-placer)
if [ "$mode" = fast ]; then
  placer=()
  step cargo test --release -p pcb-placer --lib --test orientation
fi
step cargo test --release -p pcb-router -p pcb-kicad -p pcb-global ${placer[@]+"${placer[@]}"} \
  -p pcb-congestion -p pcb-topo -p config-reference
step cargo test --release -p pcb-maker --bins --test cli_exit_codes ${walkthrough[@]+"${walkthrough[@]}"} \
  --test native_verification --test router_fingerprint --test config_reference --test hostile_inputs --test fixtures
step python3 -m pytest -q benchmarks/agent-tasks
step python3 -m pytest -q benchmarks/github
step python3 -m pytest -q benchmarks/expansion
step python3 -m pytest -q scripts
step cargo clippy --release --workspace --all-targets -- -D warnings
# A cargo test of the library crates does not refresh
# target/release/pcb-maker: build it last so the binary matches the tree
# (after the CLI tests this takes seconds).
step cargo build --release

echo "all checks passed ($mode) on $(git rev-parse --short HEAD)$(git diff --quiet HEAD -- || echo ' (with uncommitted changes)')"
echo "binary: $root/target/release/pcb-maker"
echo "snapshot it with: bash scripts/snapshot.sh <n>"
