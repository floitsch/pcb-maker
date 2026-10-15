#!/usr/bin/env bash
# Copyright (C) 2026 Toit contributors.
#
# Builds the release binary of the checkout this script lives in and copies
# it to build/bin/pcb-maker-x<n>, the snapshot benchmarks and logs name.
# Refuses a number that is taken: snapshots are never overwritten, a log
# that names x<n> must keep meaning that binary.
#
#   bash scripts/snapshot.sh 1180
#
# Pick n from your own free range (scripts/worktree.sh prints the rule).
# A cargo test of the library crates does not refresh
# target/release/pcb-maker, so this always builds first.

set -euo pipefail

if [ $# -ne 1 ] || ! [[ "$1" =~ ^[0-9]+$ ]]; then
  echo "usage: $0 <n>   (copies the release binary to build/bin/pcb-maker-x<n>)" >&2
  exit 2
fi

root="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$root"
destination="build/bin/pcb-maker-x$1"

if [ -e "$destination" ]; then
  echo "$destination exists; pick another number (ls build/bin)" >&2
  exit 1
fi

cargo build --release
mkdir -p build/bin
# -n: never replace a snapshot another agent made while this one built.
cp -n target/release/pcb-maker "$destination"
if ! cmp -s target/release/pcb-maker "$destination"; then
  echo "$destination appeared while building and is another binary; pick another number" >&2
  exit 1
fi

commit="$(git rev-parse --short HEAD)"
dirty=""
git diff --quiet HEAD -- || dirty=" with uncommitted changes"
echo "$destination: $commit$dirty"
