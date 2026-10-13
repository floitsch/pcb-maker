#!/usr/bin/env python3
# Copyright (C) 2026 Toit contributors.
"""Summarises stats.py outputs side by side: per set of boards, the median
over boards of each board-level statistic.

    experiments/planted/summary.py name=stats.json [name=stats.json ...] [--markdown]"""

import json
import sys

import numpy as np

KEYS = [
    ("layers", lambda s: s["layers"]),
    ("parts", lambda s: s["parts"]),
    ("area/part mm2", lambda s: s["area_per_part_mm2"]),
    ("nets", lambda s: s["nets"]),
    ("pour nets", lambda s: s["pour_nets"]),
    ("pads/net mean", lambda s: (s["pads_per_net"] or {}).get("mean")),
    ("pads/net p90", lambda s: (s["pads_per_net"] or {}).get("p90")),
    ("nets/part mean", lambda s: (s["nets_per_part"] or {}).get("mean")),
    ("detour median", lambda s: (s["detour"] or {}).get("p50")),
    ("detour p90", lambda s: (s["detour"] or {}).get("p90")),
    ("length/MST", lambda s: s["length_over_mst"]),
    ("vias/net mean", lambda s: (s["vias_per_net"] or {}).get("mean")),
    ("one-layer nets", lambda s: s["one_layer_nets"]),
    ("tiles used", lambda s: s["tiles_used"]),
    ("tile util p50", lambda s: (s["tile_utilisation"] or {}).get("p50")),
    ("tile util p90", lambda s: (s["tile_utilisation"] or {}).get("p90")),
    ("tile util p99", lambda s: (s["tile_utilisation"] or {}).get("p99")),
    ("tiles > 0.5", lambda s: s["tiles_over_0_5"]),
    ("used-tile util p50", lambda s: (s.get("tile_utilisation_used") or {}).get("p50")),
    ("used-tile util p90", lambda s: (s.get("tile_utilisation_used") or {}).get("p90")),
]


def load(path):
    data = json.load(open(path))
    if isinstance(data, dict):
        data = [data]
    return [d for d in data if "error" not in d]


def column(rows, getter):
    values = []
    for r in rows:
        try:
            v = getter(r)
        except (KeyError, TypeError):
            v = None
        if v is not None:
            values.append(v)
    if not values:
        return "-"
    a = np.array(values, dtype=float)
    return f"{np.median(a):.3g} [{np.quantile(a, 0.25):.3g}-{np.quantile(a, 0.75):.3g}]"


def main():
    sets = []
    for argument in sys.argv[1:]:
        if argument.startswith("--"):
            continue
        name, path = argument.split("=", 1)
        sets.append((name, load(path)))
    header = ["statistic"] + [f"{n} ({len(r)})" for n, r in sets]
    lines = ["| " + " | ".join(header) + " |", "|" + "---|" * len(header)]
    for key, getter in KEYS:
        lines.append("| " + " | ".join([key] + [column(r, getter) for _, r in sets]) + " |")
    print("\n".join(lines))


if __name__ == "__main__":
    main()
