#!/usr/bin/env python3
# Copyright (C) 2026 Toit contributors.
# Use of this source code is governed by an MIT-style license that can be
# found in the LICENSE file.
"""Reads measure.py's traces: per board the weighted wirelength after each
placer stage against the designer's, how crowded the global placement is,
and which parts move farthest and why.

    analyse.py <measure-dir> [--parts N] [--seeds all|kept]
"""

import argparse
import json
import math
import re
import statistics
import sys
from pathlib import Path


def distance(a, b):
    return math.hypot(a[0] - b[0], a[1] - b[1])


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("directory", type=Path)
    parser.add_argument("--parts", type=int, default=0, help="list the N farthest movers per board")
    parser.add_argument("--markdown", action="store_true")
    arguments = parser.parse_args()
    rows = []
    for trace in sorted(arguments.directory.glob("*/trace.json")):
        name = trace.parent.name
        data = json.loads(trace.read_text())
        log = (trace.parent / "place.log").read_text()
        illegal_designer = re.search(r"designer: (\d+) movable parts illegal", log)
        seeds = [seed for seed in data["seeds"] if seed["unplaced"] + seed["illegal"] == 0] or data["seeds"]
        kept = data["seeds"][0]
        stages = kept["stages"]
        def at(seed, stage):
            return seed["wirelength"][seed["stages"].index(stage)] if stage in seed["stages"] else float("nan")
        designer = kept["designer_wirelength"] or float("nan")
        movable = len(kept["parts"])
        # Medians over the legal seeds: one seed is noise.
        median = {stage: statistics.median(at(seed, stage) for seed in seeds) for stage in ["global", "annealed", "legal", "final"]}
        far = [part for part in kept["parts"] if "annealed" in kept["stages"]]
        index = {stage: kept["stages"].index(stage) for stage in kept["stages"]}
        moves_anneal = [distance(p["centers"][index["global"]], p["centers"][index["annealed"]]) for p in far] if "annealed" in index else []
        moves_legal = [distance(p["centers"][index["annealed"]], p["centers"][index["legal"]]) for p in far] if "annealed" in index else []
        moves_refine = [distance(p["centers"][index["legal"]], p["centers"][index["final"]]) for p in kept["parts"]]
        to_designer = [distance(p["centers"][index["final"]], p["designer_center"]) for p in kept["parts"] if "designer_center" in p]
        relaxation = kept["relaxation"]
        rows.append({
            "name": name, "movable": movable, "designer": designer, "median": median,
            "kept": {stage: at(kept, stage) for stage in ["global", "annealed", "legal", "final"]},
            "crowding": kept["crowding"], "relaxation": relaxation,
            "illegal_designer": int(illegal_designer.group(1)) if illegal_designer else None,
            "moves_anneal": statistics.median(moves_anneal) if moves_anneal else float("nan"),
            "moves_legal_p90": sorted(moves_legal)[int(0.9 * (len(moves_legal) - 1))] if moves_legal else float("nan"),
            "moves_legal_max": max(moves_legal) if moves_legal else float("nan"),
            "legal_moved": sum(move > 0.5 for move in moves_legal),
            "refine_moved": sum(move > 0.01 for move in moves_refine),
            "to_designer": statistics.median(to_designer) if to_designer else float("nan"),
            "kept_seed": kept,
        })
    header = ["board", "mov", "level", "glob", "anneal", "legal", "final", "designer", "fin/des", "glob conf", "glob ovl", "ann conf", "leg moved", "leg p90", "leg max", "ref moved", "des illegal"]
    if arguments.markdown:
        print("| " + " | ".join(header) + " |")
        print("|" + "---|" * len(header))
    for row in rows:
        r = row["relaxation"]
        level = f"h{r['halo_scale']:.2g}/{r['small_halo_scale']:.2g} s{r['spacing']:.2g} g{r['grid']:.2g}{' T' if r['tight'] else ''}{' C' if r['courtyard_spacing'] else ''}"
        m = row["kept"]
        crowd = row["crowding"]
        cells = [
            row["name"].split("__")[-1][:22], row["movable"], level,
            f"{m['global']:.0f}", f"{m['annealed']:.0f}", f"{m['legal']:.0f}", f"{m['final']:.0f}",
            f"{row['designer']:.0f}", f"{m['final'] / row['designer']:.2f}" if row["designer"] == row["designer"] else "-",
            crowd["global"]["conflicted"], f"{crowd['global']['overlap_share']:.2f}",
            crowd.get("annealed", {}).get("conflicted", "-"),
            row["legal_moved"], f"{row['moves_legal_p90']:.1f}", f"{row['moves_legal_max']:.1f}", row["refine_moved"],
            row["illegal_designer"],
        ]
        if arguments.markdown:
            print("| " + " | ".join(str(cell) for cell in cells) + " |")
        else:
            print(" ".join(f"{str(cell):>8}" if i else f"{str(cell):24}" for i, cell in enumerate(cells)))
        if arguments.parts:
            kept = row["kept_seed"]
            index = {stage: kept["stages"].index(stage) for stage in kept["stages"]}
            if "annealed" not in index:
                continue
            parts = sorted(kept["parts"], key=lambda p: -distance(p["centers"][index["annealed"]], p["centers"][index["legal"]]))
            for part in parts[:arguments.parts]:
                move = distance(part["centers"][index["annealed"]], part["centers"][index["legal"]])
                if move < 0.5:
                    break
                cost = part["net_cost"]
                print(f"    {part['reference']:8} pins {part['pins']:3} body {part['body'][0]:.1f}x{part['body'][1]:.1f} halo {part['halo']:.2f} "
                      f"legal move {move:5.1f} mm, net cost ann {cost[index['annealed']]:.0f} -> legal {cost[index['legal']]:.0f} -> final {cost[index['final']]:.0f}"
                      f" (designer {part.get('designer_net_cost', float('nan')):.0f}); met: {'; '.join(part.get('met_annealed', [])[:4])}")
    totals = {stage: sum(row["kept"][stage] for row in rows) for stage in ["global", "annealed", "legal", "final"]}
    designer = sum(row["designer"] for row in rows if row["designer"] == row["designer"])
    print("totals:", {stage: round(value) for stage, value in totals.items()}, "designer", round(designer))


if __name__ == "__main__":
    sys.exit(main())
