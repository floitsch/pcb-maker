#!/usr/bin/env python3
# Copyright (C) 2026 Florian Loitsch.
"""Tables of the cut census (cuts.py) for docs/experiments/cut-census.md.

    report.py census --dir DIR      # the designer's against ours per board
    report.py seeds --dir DIR       # the measures against the first route

`census` reads `<dir>/units-r*.json` (one per ring radius) and the
extracted boards under `<dir>/extract/`, and recomputes the gaps per pair
of parts so that they can be restricted to the pairs with at least one
moved part (a footprint whose pose differs between the designer's
placement and ours). `seeds` reads `<dir>/units-r*.json` of placements
that carry their first route's open terminals (`first_open`) and gives the
Spearman correlation per board of each measure with it, and its mean over
the boards, beside the placer's objective and crossings.
"""

import argparse
import json
import math
import statistics
import sys
from pathlib import Path

import numpy as np

sys.path.insert(0, str(Path(__file__).resolve().parent))

import cuts  # noqa: E402


def short(board):
    return board.split("__")[-1]


def ranks(values):
    order = sorted(range(len(values)), key=lambda i: values[i])
    result = [0.0] * len(values)
    i = 0
    while i < len(order):
        j = i
        while j + 1 < len(order) and values[order[j + 1]] == values[order[i]]:
            j += 1
        for k in range(i, j + 1):
            result[order[k]] = (i + j) / 2.0
        i = j + 1
    return result


def spearman(xs, ys):
    if len(xs) < 3 or len(set(xs)) < 2 or len(set(ys)) < 2:
        return None
    rx, ry = ranks(xs), ranks(ys)
    mx, my = statistics.mean(rx), statistics.mean(ry)
    num = sum((a - mx) * (b - my) for a, b in zip(rx, ry))
    den = math.sqrt(sum((a - mx) ** 2 for a in rx) * sum((b - my) ** 2 for b in ry))
    return num / den if den else None


def load_units(directory):
    by_ring = {}
    for path in sorted(Path(directory).glob("units-r*.json")):
        ring = float(path.stem[len("units-r"):])
        by_ring[ring] = {(row["board"], row["label"]): row for row in json.loads(path.read_text())}
    return by_ring


def poses(data):
    return {f["reference"]: f["at"] for f in data["footprints"]}


def moved(a, b, tolerance=1e-3):
    pa, pb = poses(a), poses(b)
    result = set()
    for reference, at in pa.items():
        other = pb.get(reference)
        if other is None or abs(at[0] - other[0]) > tolerance or abs(at[1] - other[1]) > tolerance or \
                abs(((at[2] - other[2]) + 180) % 360 - 180) > 0.01:
            result.add(reference)
    return result


def gap_rows(data, moved_refs):
    board = cuts.Board(data)
    references = [f["reference"] for f in data["footprints"]]
    found = cuts.cuts(board)
    every = cuts.gap_stats(board, found)
    near_moved = [c for c in found if references[c[1]] in moved_refs or references[c[2]] in moved_refs]
    return every, cuts.gap_stats(board, near_moved) if near_moved else None


def pct(value):
    return "" if value is None else f"{100 * value:.0f}"


def census(arguments):
    directory = Path(arguments.dir)
    by_ring = load_units(directory)
    rings = sorted(by_ring)
    main = by_ring[2.5] if 2.5 in by_ring else by_ring[rings[0]]
    boards = sorted({board for board, _ in main})
    result = {}
    gap_lines = ["| board | rules w / s | cuts D / O | moved-part cuts D / O | wasted 0.1 mm D / O (base) | "
                 "top quarter D / O | bottom quarter D / O | mean phase D / O | no track (gap < Q_1) D / O |", "| --- " * 9 + "|"]
    flow_lines = ["| board | pins | " + " | ".join(f"planar r{r:g} D / O | vias r{r:g} D / O" for r in rings)
                  + " | parts short (r2.5 planar) D / O |", "| --- " * (3 + 2 * len(rings)) + "|"]
    histograms = {}
    for board in boards:
        extract_dir = directory / "extract" / board
        designer = json.loads((extract_dir / "designer.json").read_text())
        ours = json.loads((extract_dir / "ours-first.json").read_text())
        moved_refs = moved(designer, ours)
        d_all, d_moved = gap_rows(designer, moved_refs)
        o_all, o_moved = gap_rows(ours, moved_refs)
        rules = main[(board, "designer")]["rules"]
        base = d_all["wasted_01_base"]
        gap_lines.append(
            f"| {short(board)} | {rules['track']:.2f} / {rules['clearance']:.2f} | {d_all['usable']} / {o_all['usable']} | "
            f"{d_moved['usable'] if d_moved else 0} / {o_moved['usable'] if o_moved else 0} | "
            f"{pct(d_moved and d_moved['wasted_01'])} / {pct(o_moved and o_moved['wasted_01'])} ({pct(base)}) | "
            f"{pct(d_moved and d_moved['wasted_q'])} / {pct(o_moved and o_moved['wasted_q'])} | "
            f"{pct(d_moved and d_moved['tight_q'])} / {pct(o_moved and o_moved['tight_q'])} | "
            f"{(d_moved or {}).get('phase_mean', 0):.2f} / {(o_moved or {}).get('phase_mean', 0):.2f} | "
            f"{pct(d_all['by_tracks'][0] / max(1, d_all['usable']))} / {pct(o_all['by_tracks'][0] / max(1, o_all['usable']))} |")
        histograms[board] = {"designer": d_moved and d_moved["phase_histogram"],
                             "ours": o_moved and o_moved["phase_histogram"]}
        cells = []
        for ring in rings:
            d = by_ring[ring].get((board, "designer"))
            o = by_ring[ring].get((board, "ours-first"))
            if d and o:
                cells.append(f"{d['deficit_planar']} / {o['deficit_planar']} | {d['deficit_vias']} / {o['deficit_vias']}")
            else:
                cells.append(" | ")
        d, o = main[(board, "designer")], main[(board, "ours-first")]
        flow_lines.append(f"| {short(board)} | {d['pins']} | " + " | ".join(cells)
                          + f" | {d['parts_short_planar']} / {o['parts_short_planar']} |")
        result[board] = {"moved": len(moved_refs), "gaps_all": {"designer": d_all, "ours": o_all},
                         "gaps_moved": {"designer": d_moved, "ours": o_moved},
                         "flow": {f"r{ring:g}": {label: {k: by_ring[ring][(board, label)][k]
                                                         for k in ("pins", "deficit_planar", "deficit_vias",
                                                                   "parts_short_planar", "parts_short_vias")}
                                                 for label in ("designer", "ours-placed", "ours-first")
                                                 if (board, label) in by_ring[ring]}
                                  for ring in rings},
                         "parts": {label: main[(board, label)]["parts"] for label in ("designer", "ours-first")}}
    # Pooled phase histogram over the boards (moved-part cuts).
    pooled = {"designer": np.zeros(8), "ours": np.zeros(8)}
    for h in histograms.values():
        for label in pooled:
            if h[label]:
                pooled[label] += np.array(h[label])
    hist_lines = ["| phase bin (share of a track pitch above the quantum below) | " +
                  " | ".join(f"{i / 8:.3f}-{(i + 1) / 8:.3f}" for i in range(8)) + " |", "| --- " * 9 + "|"]
    for label in pooled:
        total = pooled[label].sum()
        hist_lines.append(f"| {label} (n = {int(total)}) | " + " | ".join(f"{100 * v / total:.1f}" for v in pooled[label]) + " |")
    text = "\n".join(["## Gaps (cuts with a moved part)", "", *gap_lines, "", "## Pooled phase histogram", "",
                      *hist_lines, "", "## Escape deficits", "", *flow_lines, ""])
    (directory / "census.md").write_text(text)
    (directory / "census.json").write_text(json.dumps(result, indent=1))
    print(text)


def seeds(arguments):
    directory = Path(arguments.dir)
    by_ring = load_units(directory)
    lines = []
    summary = {}
    for ring, units in sorted(by_ring.items()):
        boards = sorted({board for board, _ in units})
        measures = {
            "deficit_planar": lambda r: r["deficit_planar"],
            "deficit_vias": lambda r: r["deficit_vias"],
            "parts_short_planar": lambda r: r["parts_short_planar"],
            "closed": lambda r: r["gaps"]["by_tracks"][0] / max(1, r["gaps"]["usable"]),
            "wasted_q": lambda r: r["gaps"]["wasted_q"],
            "wasted_01": lambda r: r["gaps"]["wasted_01"],
            "phase_mean": lambda r: r["gaps"]["phase_mean"],
            "wasted_mm": lambda r: r["gaps"]["wasted_mm"],
            "objective": lambda r: r.get("objective"),
            "crossings": lambda r: r.get("crossings"),
        }
        header = "| board | n | first_open range | " + " | ".join(measures) + " |"
        lines += [f"## Ring {ring:g} mm: Spearman with first-route open terminals", "", header,
                  "| --- " * (3 + len(measures)) + "|"]
        per = {name: [] for name in measures}
        for board in boards:
            rows = [r for (b, _), r in units.items() if b == board and r.get("first_open") is not None]
            first = [r["first_open"] for r in rows]
            cells = []
            for name, get in measures.items():
                values = [get(r) for r in rows]
                rho = spearman(values, first) if all(v is not None for v in values) else None
                if rho is not None:
                    per[name].append(rho)
                cells.append("" if rho is None else f"{rho:+.2f}")
            spread = {name: (min(get(r) for r in rows), max(get(r) for r in rows))
                      for name, get in measures.items() if name.startswith("deficit")}
            lines.append(f"| {short(board)} | {len(rows)} | {min(first)}-{max(first)} | " + " | ".join(cells) + " |")
            summary.setdefault(f"r{ring:g}", {})[board] = {"spread": spread}
        lines.append("| mean over boards | | | " + " | ".join(
            f"{statistics.mean(v):+.2f} ({len(v)})" if v else "" for v in per.values()) + " |")
        lines.append("")
        summary.setdefault(f"r{ring:g}", {})["mean"] = {k: (statistics.mean(v) if v else None) for k, v in per.items()}
    text = "\n".join(lines)
    (directory / "seeds.md").write_text(text)
    (directory / "seeds.json").write_text(json.dumps(summary, indent=1))
    print(text)


def main():
    parser = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    sub = parser.add_subparsers(dest="command", required=True)
    for name in ("census", "seeds"):
        p = sub.add_parser(name)
        p.add_argument("--dir", required=True)
    arguments = parser.parse_args()
    {"census": census, "seeds": seeds}[arguments.command](arguments)


if __name__ == "__main__":
    sys.exit(main())
