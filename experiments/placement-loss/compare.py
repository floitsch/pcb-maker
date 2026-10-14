#!/usr/bin/env python3
# Copyright (C) 2026 Toit contributors.
# Use of this source code is governed by an MIT-style license that can be
# found in the LICENSE file.
"""Compares two measure.py runs board by board: the kept placement's
weighted wirelength after legalization and at the end, the median over the
legal seeds, the parts legalization moved, and the placement work.

    compare.py <before-dir> <after-dir>
"""

import json
import math
import statistics
import sys
from pathlib import Path


def summary(trace):
    data = json.loads(trace.read_text())
    seeds = [seed for seed in data["seeds"] if seed["unplaced"] + seed["illegal"] == 0]
    kept = data["seeds"][0]
    def at(seed, stage):
        return seed["wirelength"][seed["stages"].index(stage)]
    index = {stage: kept["stages"].index(stage) for stage in kept["stages"]}
    moved = 0
    if "annealed" in index:
        moved = sum(math.dist(p["centers"][index["annealed"]], p["centers"][index["legal"]]) > 0.5 for p in kept["parts"])
    return {
        "legal": at(kept, "legal"), "final": at(kept, "final"),
        "median_final": statistics.median(at(seed, "final") for seed in seeds) if seeds else float("nan"),
        "legal_seeds": len(seeds), "seeds": len(data["seeds"]),
        "moved": moved, "work": sum(seed["work"] for seed in data["seeds"]) / 150e6,
        "designer": kept["designer_wirelength"] or float("nan"),
        "illegal": kept["unplaced"] + kept["illegal"],
    }


def main():
    before, after = Path(sys.argv[1]), Path(sys.argv[2])
    totals = [0.0, 0.0, 0.0, 0.0]
    print(f"{'board':26} {'legal':>13} {'final':>13} {'median final':>15} {'moved':>9} {'work s':>13} {'designer':>8} legal seeds")
    for trace in sorted(after.glob("*/trace.json")):
        other = before / trace.parent.name / "trace.json"
        if not other.exists():
            continue
        a, b = summary(other), summary(trace)
        name = trace.parent.name.split("__")[-1][:26]
        print(f"{name:26} {a['legal']:6.0f} {b['legal']:6.0f} {a['final']:6.0f} {b['final']:6.0f} {a['median_final']:7.0f} {b['median_final']:7.0f} "
              f"{a['moved']:4} {b['moved']:4} {a['work']:6.1f} {b['work']:6.1f} {a['designer']:8.0f} {a['legal_seeds']:2}/{b['legal_seeds']:2}"
              f"{'  ILLEGAL' if b['illegal'] and not a['illegal'] else ''}")
        totals = [totals[0] + a["final"], totals[1] + b["final"], totals[2] + a["median_final"], totals[3] + b["median_final"]]
    if totals[0]:
        print(f"final {totals[0]:.0f} -> {totals[1]:.0f} ({totals[1] / totals[0] - 1:+.1%}); median {totals[2]:.0f} -> {totals[3]:.0f}")


if __name__ == "__main__":
    sys.exit(main())
