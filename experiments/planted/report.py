#!/usr/bin/env python3
# Copyright (C) 2026 Toit contributors.
"""Tables of measure.py results per approach and difficulty level.

    experiments/planted/report.py <work-dir>/results.jsonl [...]"""

import json
import sys
from collections import defaultdict

import numpy as np


def median(values):
    values = [v for v in values if v is not None]
    return float(np.median(values)) if values else None


def fmt(value, digits=2):
    return "-" if value is None else f"{value:.{digits}f}"


def main():
    rows = []
    for path in sys.argv[1:]:
        for line in open(path):
            if line.strip():
                rows.append(json.loads(line))
    # The last row per board wins (reruns append).
    latest = {}
    for row in rows:
        latest[row["name"]] = row
    groups = defaultdict(list)
    for row in latest.values():
        knobs = row.get("knobs") or {}
        groups[(knobs.get("mode") or "?", knobs.get("level") or "?")].append(row)
    print("| approach | level | boards | complete | nets routed | length ours/planted | vias ours/planted | "
          "per-net length median | nets shorter / longer | CPU s median (max) |")
    print("|---|---|---:|---:|---:|---:|---:|---:|---:|---:|")
    failures = []
    better = []
    for (mode, level), items in sorted(groups.items()):
        done = [r for r in items if "ours" in r]
        complete = sum(1 for r in done if r.get("complete"))
        routed = sum(r["ours"]["nets_routed"] for r in done)
        total = sum(r["ours"]["nets"] for r in done)
        # Totals over complete boards only (an incomplete board's length is
        # not comparable).
        full = [r for r in done if r.get("complete")]
        lr = [r["ours"]["length_mm"] / r["planted"]["length_mm"] for r in full if r["planted"]["length_mm"] > 0]
        vr = [(r["ours"]["vias"], r["planted"]["vias"]) for r in full]
        via_ratio = (sum(a for a, _ in vr) / sum(b for _, b in vr)) if vr and sum(b for _, b in vr) else None
        pn = [r["per_net"]["length_ratio_median"] for r in full if "per_net" in r]
        shorter = sum(r["per_net"]["shorter"] for r in full if "per_net" in r)
        longer = sum(r["per_net"]["longer"] for r in full if "per_net" in r)
        cpu = [r["cpu_seconds"] for r in items if "cpu_seconds" in r]
        print(f"| {mode} | {level} | {len(items)} | {complete}/{len(done)} | {routed}/{total} | "
              f"{fmt(median(lr))} | {fmt(via_ratio)} | {fmt(median(pn))} | {shorter} / {longer} | "
              f"{fmt(median(cpu), 0)} ({fmt(max(cpu) if cpu else None, 0)}) |")
        for r in items:
            if not r.get("complete"):
                failures.append(r)
            elif r["ours"]["length_mm"] < r["planted"]["length_mm"] and r["ours"]["vias"] <= r["planted"]["vias"]:
                better.append(r)
    print()
    print("Boards our router did not complete (feasible by construction):")
    for r in sorted(failures, key=lambda r: r["name"]):
        ours = r.get("ours") or {}
        native = ours.get("native") or {}
        print(f"- {r['name']}: {ours.get('nets_routed')}/{ours.get('nets')} nets, KiCad unconnected "
              f"{native.get('unconnected')}, errors {native.get('errors')}, exit {r.get('exit')}, "
              f"{r.get('error', '')}")
    print()
    print(f"Complete boards where ours is shorter with no more vias than planted: {len(better)} of "
          f"{sum(1 for r in latest.values() if r.get('complete'))}")


if __name__ == "__main__":
    main()
