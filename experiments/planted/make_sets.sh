#!/bin/sh
# Copyright (C) 2026 Toit contributors.
# The synthetic planted sets: grow and maze, five difficulty levels, four
# seeds each, every board checked by KiCad's DRC.
#   experiments/planted/make_sets.sh <out-root> [seeds]
cd "$(dirname "$0")/../.."
out=${1:-build/planted/sets}
seeds=${2:-"1 2 3 4"}
mkdir -p "$out"
level() {
  case $1 in
    L2-easy)   echo "--layers 2 --parts 40 --area-per-part 60 --corridor 2 --wind 0.15 --blocker-density 0.5" ;;
    L2-medium) echo "--layers 2 --parts 40 --area-per-part 40 --corridor 1 --wind 0.3 --detour-max 1.8 --blocker-density 1.0" ;;
    L2-tight)  echo "--layers 2 --parts 40 --area-per-part 28 --corridor 0 --wind 0.5 --detour-max 2.2 --blocker-density 2.0" ;;
    L4-medium) echo "--layers 4 --parts 60 --area-per-part 30 --corridor 1 --wind 0.3 --detour-max 1.8 --blocker-density 1.0" ;;
    L4-tight)  echo "--layers 4 --parts 60 --area-per-part 20 --corridor 0 --wind 0.5 --detour-max 2.2 --blocker-density 2.0" ;;
    L2-dense)  echo "--layers 2 --parts 120 --area-per-part 20 --corridor 0 --wind 0.3 --blocker-density 2.0 --mix passive=0.8,small=0.1,ic=0.05,connector=0.03,tht=0.02" ;;
    L4-dense)  echo "--layers 4 --parts 150 --area-per-part 15 --corridor 0 --wind 0.3 --blocker-density 2.0 --mix passive=0.8,small=0.1,ic=0.05,connector=0.03,tht=0.02" ;;
  esac
}
modes=${MODES:-"grow maze"}
levels=${LEVELS:-"L2-easy L2-medium L2-tight L4-medium L4-tight L2-dense L4-dense"}
for mode in $modes; do
  for lv in $levels; do
    for seed in $seeds; do
      knobs=$(level $lv)
      # Grow boards get no netless blockers: their parts are the blockers.
      [ $mode = grow ] && knobs=$(echo "$knobs" | sed 's/--blocker-density [0-9.]*//')
      d=$out/$mode-$lv-s$seed
      echo "taskset -c 24-31 nice -n 10 python3 experiments/planted/generate.py $d --mode $mode --level $lv --seed $seed $knobs --drc > $d.log 2>&1"
    done
  done
done | xargs -P 4 -I{} sh -c "{}"
