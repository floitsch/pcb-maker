#!/bin/bash
# Copyright (C) 2026 Toit contributors.
# One placement (place-kicad-board, 200 s of work, the layout's share of a
# third) or one layout (the project's layout.json, a third of the budget)
# of a permuted source, four threads.
#   run.sh <binary tag> place|layout <source dir> <board id> <out dir>
set -u
tag=$1; kind=$2; source=$3; board=$4; out=$5
binary=/home/flo/programming/pcb-maker/build/bin/pcb-maker-$tag
export RAYON_NUM_THREADS=4
rm -rf "$out"
if [ "$kind" = place ]; then
  echo '{"constraints": "constraints.json"}' > "$source/placer.json"
  PCB_PLACER_WORK_SECONDS=200 "$binary" place-kicad-board "$source" "$board" "$out" "$source/placer.json" > "$out.log" 2>&1
else
  "$binary" layout-kicad-board "$source" "$board" "$out" auto "$source/layout.json" > "$out.log" 2>&1
fi
echo "$tag $kind $out exit $?"
