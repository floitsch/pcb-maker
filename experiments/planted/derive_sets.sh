#!/bin/sh
# Copyright (C) 2026 Toit contributors.
# Derived planted boards: blockers into our v14 passing results (ours) and
# into the designers' boards of the same tasks (designer), at blocker
# densities 0 (the source as it is) and 2.
#   experiments/planted/derive_sets.sh <out-root>
cd "$(dirname "$0")/../.."
out=${1:-build/planted/sets/derived}
mkdir -p "$out"
for name in 0xCB-dev__0xCB-1337__pcb CDFER__Business-Cards__USB_Cable_Tester__PCB ISSUIUC__ISS-PCB__BAGEL-MK1 \
            ISSUIUC__ISS-PCB__MIDAS-MK2 hackclub__OnBoard__MotionCubeViewAllForces headblockhead__slab-pcb__slab-pcb-left \
            siderakb__ergo-snm-keyboard__ErgoSNM_keyboard 0xCB-dev__0xCB-1337__1337-v4.0; do
  ours=$(ls build/github-layout-v14/$name/layout/result/*.kicad_pcb | head -1)
  designer=$(ls benchmarks/real/external/github/$name/*.kicad_pcb | head -1)
  for density in 0 2; do
    echo "ours|$ours|$out/ours-d$density-$name|$density"
    echo "designer|$designer|$out/designer-d$density-$name|$density"
  done
done | while IFS='|' read kind source dir density; do
  printf '%s\0%s\0%s\0%s\0' "$kind" "$source" "$dir" "$density"
done | xargs -0 -n 4 -P 4 sh -c 'taskset -c 24-31 nice -n 10 python3 experiments/planted/derive.py "$1" "$2" --source-kind "$0" --density "$3" --level "d$3" > "$2.log" 2>&1'
