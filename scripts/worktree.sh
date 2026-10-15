#!/usr/bin/env bash
# Copyright (C) 2026 Toit contributors.
#
# Creates a git worktree for a package of work, branched from main, beside
# the main checkout (../pcb-maker-<name>), and links the main checkout's
# untracked shared directories into it:
#
#   benchmarks/real/external  the harvested boards and their schematics
#   build                     logs, sweeps, tiers and the snapshots in build/bin
#   artifacts                 retained outputs
#
# so that run.py, the tiers and the snapshots work there as in the main
# checkout. target/ is the worktree's own (its first build takes minutes).
#
#   bash scripts/worktree.sh <name> <branch>
#   bash scripts/worktree.sh docs quality-docs   # ../pcb-maker-docs on quality-docs

set -euo pipefail

if [ $# -ne 2 ]; then
  echo "usage: $0 <name> <branch>   (creates ../pcb-maker-<name> on a new branch from main)" >&2
  exit 2
fi
name="$1"
branch="$2"

here="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
# The main checkout is the first entry of the worktree list, wherever this
# script runs from.
# (awk reads to the end: an early exit would break git's pipe under pipefail.)
main="$(git -C "$here" worktree list --porcelain | awk '/^worktree / && !found { print substr($0, 10); found = 1 }')"
target="$(dirname "$main")/pcb-maker-$name"

if [ -e "$target" ]; then
  echo "$target exists" >&2
  exit 1
fi

git -C "$main" worktree add "$target" -b "$branch" main

for path in benchmarks/real/external build artifacts; do
  if [ -e "$main/$path" ]; then
    mkdir -p "$(dirname "$target/$path")"
    ln -s "$main/$path" "$target/$path"
    echo "linked $path -> $main/$path"
  else
    echo "not linked: $main/$path does not exist"
  fi
done

highest="$(ls "$main/build/bin" 2>/dev/null | sed -n 's/^pcb-maker-x\([0-9][0-9]*\)$/\1/p' | sort -n | tail -1)"
highest="${highest:-0}"
next=$(( (highest / 10 + 1) * 10 ))
cat <<MESSAGE

Worktree: $target (branch $branch, from main $(git -C "$main" rev-parse --short main))

Snapshot numbering: snapshots live in the shared build/bin as
pcb-maker-x<n> and are never reused. List build/bin, take a free range
above the highest number in use (now x$highest; for example
x$next-x$((next + 9))), and say which range is yours in your hand-off so
that agents working side by side do not collide. Build and copy one with
  bash scripts/snapshot.sh <n>
(it refuses a number that is taken). Before committing:
  bash scripts/check.sh
MESSAGE
