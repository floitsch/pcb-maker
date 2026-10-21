#!/usr/bin/env bash
# Copyright (C) 2026 Toit contributors.
#
# Housekeeping: lists, and with --yes removes, what the work has left
# behind beside the main checkout:
#
#   worktrees  ../pcb-maker-<name> (scripts/worktree.sh) whose HEAD main
#              contains, idle for --keep-days (their index, HEAD, reflog,
#              top directory and release build), with nothing uncommitted
#              (the linked build, artifacts and benchmarks/real/external
#              symlinks aside) and no process working in them; removed
#              with their symlinks
#              (never the linked directories), `git worktree remove` and
#              `git branch -d` (merged branches only).
#   snapshots  build/bin/pcb-maker-* that no committed file on main names
#              (pcb-maker-x1170 counts as named by "x1170" anywhere in the
#              tree: the docs, docs/handover.md's references and its tier
#              logs, the changelog, tier files), that no running process
#              uses or names, and older than --keep-days (default 7: a
#              snapshot of work in progress is not in the docs yet).
#
# It always prints what it would remove (and what it keeps, with why,
# under --verbose) first; without --yes it removes nothing.
#
#   bash scripts/prune.sh                 # list only
#   bash scripts/prune.sh --verbose       # also what is kept and why
#   bash scripts/prune.sh --yes           # remove what it lists

set -euo pipefail

yes=0
verbose=0
keep_days=7
while [ $# -gt 0 ]; do
  case "$1" in
    --yes) yes=1 ;;
    --verbose) verbose=1 ;;
    --keep-days) shift; keep_days="$1" ;;
    -h|--help) sed -n '3,28p' "$0" | sed 's/^# \{0,1\}//'; exit 0 ;;
    *) echo "unknown argument: $1" >&2; exit 2 ;;
  esac
  shift
done

here="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
# The main checkout is the first entry of the worktree list (worktree.sh).
main="$(git -C "$here" worktree list --porcelain | awk '/^worktree / && !found { print substr($0, 10); found = 1 }')"
main_head="$(git -C "$main" rev-parse main)"
linked="build artifacts benchmarks/real/external"

say_keep() {
  if [ "$verbose" = 1 ]; then echo "  keep $1: $2"; fi
}

# Working directories and command lines of the running processes (one
# line each), to keep what is in use.
in_use="$(for pid in /proc/[0-9]*; do
  cwd="$(readlink "$pid/cwd" 2>/dev/null || true)"
  exe="$(readlink "$pid/exe" 2>/dev/null || true)"
  cmd="$(tr '\0' ' ' < "$pid/cmdline" 2>/dev/null || true)"
  printf '%s\t%s\t%s\n' "$cwd" "$exe" "$cmd"
done)"

echo "== worktrees (main at ${main_head:0:7})"
remove_worktrees=()
while IFS=$'\t' read -r path head branch; do
  [ "$path" = "$main" ] && continue
  name="${branch:-detached ${head:0:7}}"
  if [ ! -d "$path" ]; then
    say_keep "$path" "missing (git worktree prune clears it)"
    continue
  fi
  if ! git -C "$main" merge-base --is-ancestor "$head" "$main_head" 2>/dev/null; then
    say_keep "$path" "$name not merged into main"
    continue
  fi
  # Recent activity: the worktree's index, HEAD and reflog in its git
  # directory, its top directory and its release build (a worktree made
  # from main's head yesterday is "merged" before its first commit).
  gitdir="$(git -C "$path" rev-parse --absolute-git-dir 2>/dev/null || echo "$path")"
  newest=0
  for item in "$gitdir/index" "$gitdir/HEAD" "$gitdir/logs/HEAD" "$path" "$path/target/release"; do
    if [ -e "$item" ]; then
      stamp=$(stat -c %Y "$item")
      if [ "$stamp" -gt "$newest" ]; then newest=$stamp; fi
    fi
  done
  idle_days=$(( ($(date +%s) - newest) / 86400 ))
  if [ "$idle_days" -lt "$keep_days" ]; then
    say_keep "$path" "$name merged, but active $idle_days days ago (under $keep_days)"
    continue
  fi
  # Uncommitted work, the linked shared directories aside (no optional
  # locks: the listing must not refresh the index it reads the age of).
  dirty="$(git --no-optional-locks -C "$path" status --porcelain --untracked-files=normal 2>/dev/null \
    | grep -v -E '^\?\? (build|artifacts|benchmarks/real/external)/?$' || true)"
  if [ -n "$dirty" ]; then
    say_keep "$path" "$name merged, but with uncommitted changes ($(echo "$dirty" | wc -l) paths)"
    continue
  fi
  user="$(printf '%s\n' "$in_use" | awk -F'\t' -v p="$path" '$1 == p || index($1, p "/") == 1 { print; exit }')"
  if [ -n "$user" ]; then
    say_keep "$path" "$name merged, but a process works in it"
    continue
  fi
  echo "  would remove $path ($name, merged into main)"
  remove_worktrees+=("$path|$branch")
done < <(git -C "$main" worktree list --porcelain | awk '
  /^worktree / { if (path != "") print path "\t" head "\t" branch; path = substr($0, 10); head = ""; branch = "" }
  /^HEAD / { head = substr($0, 6) }
  /^branch / { branch = substr($0, 8); sub("^refs/heads/", "", branch) }
  END { if (path != "") print path "\t" head "\t" branch }')

echo "== snapshots in $main/build/bin"
# Every "x<digits>" and "pcb-maker-<name>" a committed file on main names.
named="$(git -C "$main" grep -ohE '(\bx[0-9]+\b|pcb-maker-[A-Za-z0-9_.-]+)' main -- 2>/dev/null \
  | sed -e 's/^main://' | sort -u)"
remove_snapshots=()
freed=0
now="$(date +%s)"
for binary in "$main"/build/bin/pcb-maker-*; do
  [ -f "$binary" ] || continue
  file="$(basename "$binary")"
  tag="${file#pcb-maker-}"
  if printf '%s\n' "$named" | grep -qxF -e "$file" -e "$tag"; then
    say_keep "$file" "named in a committed file"
    continue
  fi
  age_days=$(( (now - $(stat -c %Y "$binary")) / 86400 ))
  if [ "$age_days" -lt "$keep_days" ]; then
    say_keep "$file" "$age_days days old (under $keep_days)"
    continue
  fi
  if printf '%s\n' "$in_use" | grep -qF "$file"; then
    say_keep "$file" "a running process uses or names it"
    continue
  fi
  size=$(stat -c %s "$binary")
  freed=$((freed + size))
  echo "  would remove build/bin/$file ($age_days days old, $((size / 1048576)) MB)"
  remove_snapshots+=("$binary")
done

echo
echo "${#remove_worktrees[@]} worktrees and ${#remove_snapshots[@]} snapshots ($((freed / 1073741824)) GB) to remove"
if [ "$yes" != 1 ]; then
  echo "nothing removed (pass --yes to remove them)"
  exit 0
fi

for entry in "${remove_worktrees[@]}"; do
  path="${entry%%|*}"
  branch="${entry#*|}"
  # The links to the main checkout's shared directories go first (never
  # what they point to): git keeps a worktree with untracked files.
  for link in $linked; do
    if [ -L "$path/$link" ]; then rm "$path/$link"; fi
  done
  git -C "$main" worktree remove "$path"
  if [ -n "$branch" ]; then git -C "$main" branch -d "$branch"; fi
  echo "removed $path"
done
for binary in "${remove_snapshots[@]}"; do
  rm -- "$binary"
  echo "removed $binary"
done
