#!/usr/bin/env python3
# Copyright (C) 2026 Toit contributors.
"""Summarises exported samples: counts, timings, label statistics.

    experiments/congestion/stats.py build/cdata/samples"""

import collections
import json
import sys
from pathlib import Path

import numpy as np

sys.path.insert(0, str(Path(__file__).resolve().parent))
from dataset import read_sample  # noqa: E402


def main():
    root = Path(sys.argv[1])
    rows = []
    for path in root.glob("*/*.json"):
        header, arrays = read_sample(path)
        if "probe" not in header:
            continue
        inside = arrays["features"][header["channels"].index("inside")] > 0
        layers = header["layers"]
        row = {"board": path.parent.name, "variant": path.stem, "layers": layers,
               "tiles": int(inside.sum()), "unfinished": header["probe"]["unfinished"],
               "routable": header["probe"]["routable_nets"],
               "lower": header["seconds_lower"], "features": header["seconds_features"],
               "probe": header["probe"]["seconds"], "expansions": header["probe"]["expansions"]}
        for name in ("conflict", "history", "tile_history"):
            values = arrays[name][:layers][:, inside]
            row[f"{name}_nonzero"] = float((values > 0).mean())
            row[f"{name}_sum"] = float(values.sum())
        rows.append(row)
    kinds = collections.Counter(r["variant"].split("-")[0].rstrip("0123456789") for r in rows)
    boards = {r["board"] for r in rows}
    print(f"{len(rows)} samples of {len(boards)} boards; variants: {dict(kinds)}")
    print(f"layers: {dict(collections.Counter(r['layers'] for r in rows))}")
    for key in ("tiles", "lower", "features", "probe", "expansions", "unfinished"):
        values = np.array([r[key] for r in rows], dtype=float)
        print(f"{key:>12}: median {np.median(values):.3g}, mean {values.mean():.3g}, p90 {np.percentile(values, 90):.3g}, max {values.max():.3g}")
    zero = sum(r["unfinished"] == 0 for r in rows)
    print(f"unfinished = 0 in {zero} samples ({zero / len(rows):.0%}); share of routable nets unfinished: "
          f"median {np.median([r['unfinished'] / max(r['routable'], 1) for r in rows]):.2f}")
    for name in ("conflict", "history", "tile_history"):
        nonzero = np.array([r[f"{name}_nonzero"] for r in rows])
        print(f"{name:>12}: share of board tiles (per layer) > 0: median {np.median(nonzero):.3f}, mean {nonzero.mean():.3f}")
    varied = 0
    for board in boards:
        values = {r["unfinished"] for r in rows if r["board"] == board}
        varied += len(values) > 1
    print(f"boards whose placements differ in unfinished nets: {varied} of {len(boards)}")


if __name__ == "__main__":
    main()
