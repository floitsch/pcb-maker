#!/usr/bin/env python3
# Copyright (C) 2026 Toit contributors.
# Use of this source code is governed by an MIT-style license that can be
# found in the LICENSE file.
"""Seed-repeated pairs: per board, the median and range over seeds of
KiCad unconnected, errors by type, vias, length, HPWL of the final board
and CPU seconds, for two sets of run.py outputs.

    pairs.py --off build/room-pair-a-off-s1 ... --on build/room-pair-a-on-s1 ...

HPWL: the half perimeter of every net's pads on the routed board (all
nets with two pads or more, unweighted), with pcbnew.
"""

import argparse
import json
import statistics
import sys
from pathlib import Path


def hpwl(board_path):
    import pcbnew
    board = pcbnew.LoadBoard(str(board_path))
    boxes = {}
    for fp in board.GetFootprints():
        for pad in fp.Pads():
            net = pad.GetNetname()
            if not net or net.startswith("unconnected-"):
                continue
            p = pad.GetPosition()
            x, y = pcbnew.ToMM(p.x), pcbnew.ToMM(p.y)
            b = boxes.setdefault(net, [x, y, x, y, 0])
            b[0], b[1], b[2], b[3] = min(b[0], x), min(b[1], y), max(b[2], x), max(b[3], y)
            b[4] += 1
    return sum(b[2] - b[0] + b[3] - b[1] for b in boxes.values() if b[4] > 1)


def rows(directories):
    result = {}
    for directory in directories:
        log = Path(str(directory) + ".log")
        if not log.exists():
            continue
        for line in log.read_text().splitlines():
            if not line.startswith("{"):
                continue
            row = json.loads(line)
            task = Path(directory) / row["name"]
            placed = task / "layout/placed/board-placer.json"
            if placed.exists() and "native" in row:
                board_id = json.loads(placed.read_text())["board_id"]
                board = task / f"layout/result/{board_id}.kicad_pcb"
                if board.exists():
                    try:
                        row["hpwl"] = hpwl(board)
                    except Exception:  # noqa: BLE001
                        pass
            result.setdefault(row["name"], []).append(row)
    return result


def summary(values):
    values = [v for v in values if v is not None]
    if not values:
        return "-"
    return f"{statistics.median(values):.0f} [{min(values):.0f}-{max(values):.0f}]"


def errors(rows_):
    out = {}
    for row in rows_:
        for kind, count in (row.get("native") or {}).get("errors", {}).items():
            out.setdefault(kind, []).append(count)
    return ", ".join(f"{k} {statistics.median(v + [0] * (len(rows_) - len(v))):.0f}" for k, v in sorted(out.items())) or "-"


def main():
    parser = argparse.ArgumentParser()
    parser.add_argument("--off", nargs="+", required=True)
    parser.add_argument("--on", nargs="+", required=True)
    arguments = parser.parse_args()
    off, on = rows(arguments.off), rows(arguments.on)
    keys = [("unconnected", lambda r: (r.get("native") or {}).get("unconnected")), ("vias", lambda r: r.get("vias")),
            ("length", lambda r: r.get("length_mm")), ("hpwl", lambda r: r.get("hpwl")),
            ("cpu", lambda r: r.get("cpu_seconds"))]
    for name in sorted(set(off) | set(on)):
        a, b = off.get(name, []), on.get(name, [])
        print(f"{name}  (seeds off {len(a)}, on {len(b)}; passes off {sum(r.get('pass', False) for r in a)}, "
              f"on {sum(r.get('pass', False) for r in b)})")
        for key, get in keys:
            print(f"   {key:12} off {summary([get(r) for r in a]):24} on {summary([get(r) for r in b]):24}")
        print(f"   {'errors':12} off {errors(a):24} on {errors(b)}")
        failed = [r["name"] for r in a + b if "error" in r]
        if failed:
            print(f"   failures: {[r.get('error') for r in a + b if 'error' in r]}")
    # Paired CPU ratio (on / off) per board and seed, matched by order.
    ratios = []
    for name in off:
        for x, y in zip(off.get(name, []), on.get(name, [])):
            if x.get("cpu_seconds") and y.get("cpu_seconds"):
                ratios.append(y["cpu_seconds"] / x["cpu_seconds"])
    if ratios:
        print(f"CPU on/off: median {statistics.median(ratios):.2f}, geomean "
              f"{statistics.geometric_mean(ratios):.2f}, range {min(ratios):.2f}-{max(ratios):.2f} over {len(ratios)} pairs")


if __name__ == "__main__":
    sys.exit(main())
