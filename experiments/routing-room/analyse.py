#!/usr/bin/env python3
# Copyright (C) 2026 Toit contributors.
# Use of this source code is governed by an MIT-style license that can be
# found in the LICENSE file.
"""Tables from halo_use.py's JSON: halo granted against used per board and
per pin bucket, and how well pins, RUDY and the probe's history predict
where the routing goes (Spearman per board).

    analyse.py halo_use.json [--boards NAME...]
"""

import argparse
import json
import statistics
import sys
from pathlib import Path

BUCKETS = ["2", "3-4", "5-9", "10-16", "17-32", "33+"]
IDLE = 0.05


def ranks(values):
    order = sorted(range(len(values)), key=lambda i: values[i])
    result = [0.0] * len(values)
    i = 0
    while i < len(order):
        j = i
        while j + 1 < len(order) and values[order[j + 1]] == values[order[i]]:
            j += 1
        for k in range(i, j + 1):
            result[order[k]] = (i + j) / 2
        i = j + 1
    return result


def spearman(a, b):
    if len(a) < 5:
        return None
    ra, rb = ranks(a), ranks(b)
    ma, mb = statistics.mean(ra), statistics.mean(rb)
    num = sum((x - ma) * (y - mb) for x, y in zip(ra, rb))
    den = (sum((x - ma) ** 2 for x in ra) * sum((y - mb) ** 2 for y in rb)) ** 0.5
    return num / den if den else None


def fmt(value, digits=2):
    return "-" if value is None else f"{value:.{digits}f}"


def main():
    parser = argparse.ArgumentParser()
    parser.add_argument("json", type=Path)
    parser.add_argument("--boards", nargs="*")
    arguments = parser.parse_args()
    results = json.loads(arguments.json.read_text())
    if arguments.boards:
        results = [r for r in results if any(b in r["task"] for b in arguments.boards)]
    print("Per board (movable parts): halo area granted (mm2, one layer), share of its capacity used, granted area "
          "where under 5 % is used; band (1 mm) use relative to the board mean for parts with and without halo;\n"
          "Spearman of band use against pins, band RUDY, probe history")
    print(f"{'board':42} {'L':>2} {'parts':>5} {'granted':>8} {'used%':>6} {'idle%':>6} {'board%':>6} "
          f"{'bandH':>6} {'band0':>6} {'r_pins':>6} {'r_rudy':>6} {'r_hist':>6} {'r_conf':>6}")
    by_bucket = {b: {"n": 0, "granted": 0.0, "used": 0.0, "idle": 0.0, "band": [], "with": 0} for b in BUCKETS}
    for result in results:
        movable = [p for p in result["parts"] if not p["fixed"]]
        if not movable:
            continue
        mean = result["mean_use"] or 1e-9
        granted = sum(p["ring_area"] for p in movable)
        used = sum(p["ring_area"] * (p["ring_use"] or 0) for p in movable)
        idle = sum(p["ring_area"] for p in movable if p["granted"] > 0 and (p["ring_use"] or 0) < IDLE)
        with_halo = [p["band_use"] / mean for p in movable if p["granted"] > 0]
        without = [p["band_use"] / mean for p in movable if p["granted"] == 0]
        band = [p["band_use"] for p in movable]
        r_pins = spearman([p["pins"] for p in movable], band)
        r_rudy = spearman([p["band_rudy"] for p in movable], band)
        r_hist = spearman([p.get("probe_history", 0) for p in movable], band) if "probe_history" in movable[0] else None
        r_conf = spearman([p.get("probe_conflict", 0) for p in movable], band) if "probe_conflict" in movable[0] else None
        print(f"{result['task'][:42]:42} {result['layers']:>2} {len(movable):>5} {granted:>8.0f} "
              f"{100 * used / max(granted, 1e-9):>6.1f} {100 * idle / max(granted, 1e-9):>6.1f} {100 * mean:>6.1f} "
              f"{fmt(statistics.median(with_halo) if with_halo else None):>6} {fmt(statistics.median(without) if without else None):>6} "
              f"{fmt(r_pins):>6} {fmt(r_rudy):>6} {fmt(r_hist):>6} {fmt(r_conf):>6}")
        for p in movable:
            entry = by_bucket.get(p["bucket"])
            if entry is None:
                continue
            entry["n"] += 1
            entry["band"].append(p["band_use"] / mean)
            if p["granted"] > 0:
                entry["with"] += 1
                entry["granted"] += p["ring_area"]
                entry["used"] += p["ring_area"] * (p["ring_use"] or 0)
                if (p["ring_use"] or 0) < IDLE:
                    entry["idle"] += p["ring_area"]
    print()
    print("Per pin bucket (all boards above, movable parts)")
    print(f"{'pins':>6} {'parts':>6} {'w/halo':>6} {'granted':>8} {'used%':>6} {'idle%':>6} {'band/mean med':>13} {'p25':>5} {'p75':>5}")
    for name in BUCKETS:
        entry = by_bucket[name]
        if not entry["n"]:
            continue
        band = sorted(entry["band"])
        q = lambda f: band[min(len(band) - 1, int(f * len(band)))]
        print(f"{name:>6} {entry['n']:>6} {entry['with']:>6} {entry['granted']:>8.0f} "
              f"{100 * entry['used'] / max(entry['granted'], 1e-9):>6.1f} {100 * entry['idle'] / max(entry['granted'], 1e-9):>6.1f} "
              f"{statistics.median(band):>13.2f} {q(0.25):>5.2f} {q(0.75):>5.2f}")


if __name__ == "__main__":
    sys.exit(main())
