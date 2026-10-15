#!/bin/bash
# Copyright (C) 2026 Toit contributors.
# Alternates, per board, kicad-cli (empty library tables, --refill-zones) and
# a warm daemon_drc.py round, on the same CPUs: the daemon's saving per call
# under the same load. Usage: pair.sh <cpus> <nolib-config-home> <out> <board.kicad_pcb>...
cpus=$1; config=$2; out=$3; shift 3
here=$(dirname "$(readlink -f "$0")")
mkdir -p "$out"
for board in "$@"; do
  dir=$(dirname "$board"); pcb=$(basename "$board")
  for i in 1 2; do
    (cd "$dir" && KICAD_CONFIG_HOME=$config python3 "$here/timed.py" taskset -c "$cpus" kicad-cli pcb drc --format json --severity-all --refill-zones --output "$out/pair.json" "$pcb" | tail -1 | sed "s|^|cli $pcb |")
    taskset -c "$cpus" python3 "$here/daemon_drc.py" "$board" --rounds 1 --out "$out" 2>/dev/null | grep '"rounds"' | python3 -c "import json,sys; r=json.load(sys.stdin); print('daemon', '$pcb', json.dumps(r['rounds'][0]))"
  done
done
