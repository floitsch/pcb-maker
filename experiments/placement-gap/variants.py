#!/usr/bin/env python3
# Copyright (C) 2026 Toit contributors.
# Use of this source code is governed by an MIT-style license that can be
# found in the LICENSE file.
"""Stage 0's table for several variants of one placer switch against the
designer: the metrics.json of `metrics.py run` per variant (its `ours` the
variant's placement), side by side (docs/placer.md, "Where the gap to the designer comes from", "Net weights").

    variants.py --metric hpwl_mm --metric crossings steep=build/teach2-gap/steep sqrt=build/teach2-gap/sqrt

Per board each variant's metric over the designer's, and per variant over
the boards: the median ratio, the geometric mean ratio, and on how many
boards it is above the designer's.
"""

import argparse
import json
import math
import statistics
from pathlib import Path

DEFAULT = ["hpwl_mm", "hpwl_2_mm", "hpwl_3_8_mm", "hpwl_9plus_mm", "hpwl_pour_mm", "octilinear_mm", "crossings",
           "pin_row_crossings", "connector_edge_mm", "gap_under_0_25", "gap_median_mm"]


def short(board):
    return board.split("__")[-1]


def main():
    parser = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    parser.add_argument("--metric", action="append")
    parser.add_argument("variants", nargs="+", help="NAME=DIR (a metrics.py --out)")
    arguments = parser.parse_args()
    metrics = arguments.metric or DEFAULT
    rows = {}
    names = []
    for variant in arguments.variants:
        name, directory = variant.split("=", 1)
        names.append(name)
        for row in json.loads((Path(directory) / "metrics.json").read_text())["rows"]:
            rows.setdefault(row["board"], {})[name] = row
    boards = [b for b in rows if all(n in rows[b] for n in names)]
    for metric in metrics:
        print(f"\n#### {metric}: designer, then each variant / designer\n")
        print("| board | designer | " + " | ".join(names) + " |")
        print("| --- | ---: | " + " | ".join("---:" for _ in names) + " |")
        ratios = {n: [] for n in names}
        above = {n: 0 for n in names}
        for board in boards:
            d = rows[board][names[0]]["designer"].get(metric)
            cells = []
            for n in names:
                o = rows[board][n]["ours"].get(metric)
                if d is None or o is None:
                    cells.append("-")
                    continue
                above[n] += o > d
                if d > 0 and o > 0:
                    ratios[n].append(o / d)
                cells.append(f"{o / d:.2f}" if d else f"{o:.2f} (designer 0)")
            print(f"| {short(board)} | {'-' if d is None else round(d, 2)} | " + " | ".join(cells) + " |")
        med = [f"{statistics.median(ratios[n]):.3f}" if ratios[n] else "-" for n in names]
        geo = [f"{math.exp(statistics.mean(math.log(r) for r in ratios[n])):.3f}" if ratios[n] else "-" for n in names]
        print("| median ratio | | " + " | ".join(med) + " |")
        print("| geometric mean | | " + " | ".join(geo) + " |")
        print(f"| above the designer (of {len(boards)}) | | " + " | ".join(str(above[n]) for n in names) + " |")


if __name__ == "__main__":
    main()
