#!/bin/sh
# Copyright (C) 2026 Toit contributors.
# Downloads the PCBench boards and the PCBWorld preparation chain (both pinned)
# and turns the boards into the D3 benchmark set with the local KiCad.
#
#   benchmarks/pcbench/fetch.sh [--limit N] [--workers N]
#
# Nothing downloaded here is committed: everything lands under
# benchmarks/real/external/ (gitignored). Needs kicad-cli and a python3 that
# imports pcbnew (KiCad 9 or 10).
set -e
LIMIT=""
WORKERS=8
while [ $# -gt 0 ]; do
  case "$1" in
    --limit) LIMIT="--limit $2"; shift 2 ;;
    --workers) WORKERS="$2"; shift 2 ;;
    *) echo "unknown argument: $1" >&2; exit 2 ;;
  esac
done
ROOT=$(cd "$(dirname "$0")/../.." && pwd)
EXTERNAL="$ROOT/benchmarks/real/external"
DOWNLOADS="$EXTERNAL/_downloads"
mkdir -p "$DOWNLOADS"
if [ ! -d "$DOWNLOADS/PCBench" ]; then
  git clone -q https://github.com/PCBench/PCBench.git "$DOWNLOADS/PCBench"
fi
git -C "$DOWNLOADS/PCBench" checkout -q dec3be75cbdef74787625f9043c7391cd473bb64
if [ ! -d "$DOWNLOADS/PCBWorld" ]; then
  git clone -q https://github.com/LGAI-Research/PCBWorld.git "$DOWNLOADS/PCBWorld"
fi
git -C "$DOWNLOADS/PCBWorld" checkout -q b3d62f5c37e7528670d112e03d9a90029f23f4f3

WORK="$EXTERNAL/pcbench-work"
export KICAD_CLI="${KICAD_CLI:-$(command -v kicad-cli)}"
export PCBNEW_PYTHON="${PCBNEW_PYTHON:-/usr/bin/python3}"
export PCBENCH_PCBS_ROOT="$DOWNLOADS/PCBench/PCBs"
export PCBENCH_V9_ROOT="$WORK/v9"
export PCBENCH_NEWDRC_OUT="$WORK/newdrc"
export PCBENCH_SORTED_OUT="$WORK/sorted"
PREP="$DOWNLOADS/PCBWorld/tools/datagen/pcbench_prep"
# Step 0: KiCad 5 -> current format plus a derived .kicad_pro. Step 1: the
# rule patches of PCBWorld's D3 and the DRC filter. Step 2: per-net widths
# folded into net classes (the "guide" board D3 routes); guide.py redoes
# PCBWorld's make_guide.py for KiCad 10 files, which name nets instead of
# numbering them.
python3 "$PREP/convert_v9.py" $LIMIT --workers "$WORKERS" 2>&1 | grep -v PROPERTY_ENUM | tail -3
python3 "$PREP/drc_fix_v9.py" --workers "$WORKERS" | tail -3
python3 "$ROOT/benchmarks/pcbench/guide.py" --workers "$WORKERS"
python3 "$ROOT/benchmarks/pcbench/manifest.py"
