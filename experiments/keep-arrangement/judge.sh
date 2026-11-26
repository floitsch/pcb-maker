#!/usr/bin/env bash
# Copyright (C) 2026 Florian Loitsch.
#
# A placer change judged by full layouts at several placer seeds, the two
# binaries side by side (docs/experiments/keep-arrangement.md):
#
#   judge.sh short|sweep <off-binary> <on-binary> <out-dir> [seed...]
#
# short: run.py's short tier (12 boards). sweep: the 25 boards of
# sweep-subset.json (tasks v7, the sweep's settings), fixed before any
# result. Per seed (default 1 101 201: the race places seeds N to N + 15)
# both sides run at once, each 3 boards x 2 threads, into
# <out-dir>/<set>-s<seed>-off and -on; each run's exit code goes to
# <dir>.exit. A run that has its .exit is not run again. Before a seed
# starts the script waits while the 1-minute load is above 20, less than
# 16 GB are available, or the nightly's window is open (from 01:45 until
# build/nightly/<today>/summary.md exists, at most until 10:00).
set -euo pipefail

set_name="$1"; off="$2"; on="$3"; out="$4"; shift 4
seeds=("$@")
[ ${#seeds[@]} -gt 0 ] || seeds=(1 101 201)
here="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
mkdir -p "$out"

free() {
  local load available minutes summary
  load="$(cut -d' ' -f1 /proc/loadavg)"
  available="$(awk '/^MemAvailable:/ { print int($2 / 1e6) }' /proc/meminfo)"
  minutes=$((10#$(date +%H) * 60 + 10#$(date +%M)))
  summary="$here/build/nightly/$(date +%F)/summary.md"
  if [ "$minutes" -ge 105 ] && [ "$minutes" -lt 600 ] && [ ! -f "$summary" ]; then return 1; fi
  awk -v busy="$load" -v room="$available" 'BEGIN { exit !(busy <= 20 && room >= 16) }'
}

arguments=()
case "$set_name" in
  short) arguments=(--tier short) ;;
  sweep)
    mapfile -t names < <(python3 -c "
import json, sys
subset = json.load(open(sys.argv[1]))
print('\n'.join(subset['failing'] + subset['passing']))" "$here/experiments/keep-arrangement/sweep-subset.json")
    arguments=(--tasks "$here/benchmarks/agent-tasks/tasks/v7/tasks.json" --quality --order peak-desc --only "${names[@]}")
    ;;
  *) echo "unknown set $set_name" >&2; exit 2 ;;
esac

side() {
  local directory="$1" binary="$2" seed="$3" status
  [ -f "$directory.exit" ] && return 0
  set +e
  (cd "$here" && python3 benchmarks/agent-tasks/run.py "$directory" "${arguments[@]}" --jobs 3 --threads 2 \
    --binary "$binary" --placer "{\"tuning\": {\"seed\": $seed}}") > "$directory.log" 2> "$directory.err"
  status=$?
  set -e
  echo "$status" > "$directory.exit"
  echo "$(date +%T) $directory: exit $status; $(grep -E 'tasks pass' "$directory.log" | tail -1)"
}

for seed in "${seeds[@]}"; do
  until free; do sleep 60; done
  echo "$(date +%T) $set_name seed $seed"
  side "$out/$set_name-s$seed-off" "$off" "$seed" &
  sleep 20
  side "$out/$set_name-s$seed-on" "$on" "$seed" &
  wait
done
echo "$(date +%T) $set_name done"
