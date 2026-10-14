#!/usr/bin/env python3
# Copyright (C) 2026 Toit contributors.
# Use of this source code is governed by an MIT-style license that can be
# found in the LICENSE file.
"""Sums two runner logs over the boards both have: passes, KiCad
unconnected, errors by type, vias, length, the placer's wirelength
(global, legal, final from board-placer.json) and CPU seconds as paired
relative differences; lists the boards whose placement is byte-identical.

    tier_summary.py <before-dir> <after-dir>   (the run directories; their .log beside them)
"""

import collections
import json
import statistics
import sys
from pathlib import Path


def rows(directory):
    log = Path(str(directory) + ".log")
    result = {}
    for line in log.read_text().splitlines():
        if line.startswith("{"):
            row = json.loads(line)
            result[row["name"]] = row
    return result


def placer(directory, name):
    path = directory / name / "layout/placed/board-placer.json"
    if not path.exists():
        return None
    data = json.loads(path.read_text())
    return data["wirelength_global_mm"], data["wirelength_legal_mm"], data["wirelength_final_mm"]


def placed_board(directory, name):
    placed = directory / name / "layout/placed"
    boards = list(placed.glob("*.kicad_pcb")) if placed.exists() else []
    return boards[0].read_bytes() if boards else None


def main():
    before, after = Path(sys.argv[1]), Path(sys.argv[2])
    a, b = rows(before), rows(after)
    names = sorted(set(a) & set(b))
    total = collections.defaultdict(lambda: [0.0, 0.0])
    errors = [collections.Counter(), collections.Counter()]
    ratios = []
    identical = []
    for name in names:
        for side, row in enumerate((a[name], b[name])):
            native = row.get("native") or {}
            total["passes"][side] += bool(row.get("pass"))
            total["unconnected"][side] += native.get("unconnected", 0) or 0
            total["vias"][side] += row.get("vias", 0) or 0
            total["length"][side] += row.get("length_mm", 0) or 0
            for kind, count in (native.get("errors") or {}).items():
                errors[side][kind] += count
            if "error" in row:
                errors[side]["layout error"] += 1
        wa, wb = placer(before, name), placer(after, name)
        if wa and wb:
            for index, stage in enumerate(["hpwl global", "hpwl legal", "hpwl final"]):
                total[stage][0] += wa[index]
                total[stage][1] += wb[index]
        if a[name].get("cpu_seconds") and b[name].get("cpu_seconds"):
            ratios.append(b[name]["cpu_seconds"] / a[name]["cpu_seconds"])
        if placed_board(before, name) is not None and placed_board(before, name) == placed_board(after, name):
            identical.append(name)
    print(f"{len(names)} boards in both")
    for key, (x, y) in total.items():
        print(f"{key:12} {x:10.0f} -> {y:10.0f}" + (f" ({y / x - 1:+.1%})" if x else ""))
    print("errors before", dict(errors[0]))
    print("errors after ", dict(errors[1]))
    if ratios:
        print(f"cpu seconds after/before: median {statistics.median(ratios):.2f}, geometric mean "
              f"{statistics.geometric_mean(ratios):.2f}")
    print(f"placement byte-identical on {len(identical)}: {', '.join(n.split('__')[-1] for n in identical)}")


if __name__ == "__main__":
    sys.exit(main())
