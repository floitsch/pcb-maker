#!/usr/bin/env python3
# Copyright (C) 2026 Toit contributors.
"""Does the planted solution's tile occupancy say where routing is hard?

    experiments/planted/tiles.py <measure-dir> <sets-dir> [--json out.json]

Per measured board: the planted board's track utilisation per 1.6 mm tile
(summed over layers), our routed board's, their Spearman correlation over
the tiles either uses, and where our router's negotiation hot spots (the
ten nodes with the most conflict history, from board-router.json) fall in
the planted utilisation distribution (percentile among used tiles; 0.5 is
no relation, near 1 means the router fights where the planted solution is
dense, i.e. where the slack is small)."""

import argparse
import json
import math
import sys
from pathlib import Path

import numpy as np

HERE = Path(__file__).resolve().parent
sys.path.insert(0, str(HERE))
from stats import TILE, board_outline, mm  # noqa: E402


def tile_grid(path, frame=None):
    import pcbnew
    board = pcbnew.LoadBoard(str(path))
    clearance = mm(board.GetDesignSettings().m_NetSettings.GetDefaultNetclass().GetClearance())
    outlines, _ = board_outline(board)
    if frame is None:
        xs = [p[0] for o in outlines for p in o]
        ys = [p[1] for o in outlines for p in o]
        x0, y0 = min(xs), min(ys)
        nx = int(math.ceil((max(xs) - x0) / TILE))
        ny = int(math.ceil((max(ys) - y0) / TILE))
        frame = (x0, y0, nx, ny)
    x0, y0, nx, ny = frame
    grid = np.zeros((ny, nx))
    for item in board.GetTracks():
        if item.GetClass() != "PCB_TRACK":
            continue
        s, e = item.GetStart(), item.GetEnd()
        ax, ay, bx, by = mm(s.x), mm(s.y), mm(e.x), mm(e.y)
        length = math.hypot(bx - ax, by - ay)
        n = max(1, int(length / 0.05))
        t = (np.arange(n) + 0.5) / n
        ix = np.clip(((ax + t * (bx - ax) - x0) / TILE).astype(int), 0, nx - 1)
        iy = np.clip(((ay + t * (by - ay) - y0) / TILE).astype(int), 0, ny - 1)
        np.add.at(grid, (iy, ix), length / n * (mm(item.GetWidth()) + clearance))
    return grid / (TILE * TILE), frame


def spearman(a, b):
    ra = np.argsort(np.argsort(a))
    rb = np.argsort(np.argsort(b))
    if ra.std() == 0 or rb.std() == 0:
        return None
    return float(np.corrcoef(ra, rb)[0, 1])


def main():
    parser = argparse.ArgumentParser()
    parser.add_argument("measure", type=Path)
    parser.add_argument("sets", type=Path)
    parser.add_argument("--json")
    arguments = parser.parse_args()
    rows = []
    for line in open(arguments.measure / "results.jsonl"):
        row = json.loads(line)
        name = row["name"]
        routed = arguments.measure / name / "routed" / "planted.kicad_pcb"
        planted = arguments.sets / name / "planted.kicad_pcb"
        report = arguments.measure / name / "routed" / "board-router.json"
        if not (routed.exists() and planted.exists() and report.exists()):
            continue
        p, frame = tile_grid(planted)
        o, _ = tile_grid(routed, frame)
        used = (p > 0) | (o > 0)
        hot = json.loads(report.read_text()).get("diagnostics", {}).get("hot_spots", [])
        x0, y0, nx, ny = frame
        ranks = []
        used_values = np.sort(p[p > 0])
        for spot in hot:
            ix = min(nx - 1, max(0, int((spot["at"][0] - x0) / TILE)))
            iy = min(ny - 1, max(0, int((spot["at"][1] - y0) / TILE)))
            if len(used_values):
                ranks.append(float(np.searchsorted(used_values, p[iy, ix], side="right") / len(used_values)))
        rows.append({"name": name, "mode": (row.get("knobs") or {}).get("mode"),
                     "level": (row.get("knobs") or {}).get("level"), "complete": row.get("complete"),
                     "tile_spearman": spearman(p[used], o[used]) if used.sum() > 2 else None,
                     "hot_spot_planted_percentile": round(float(np.mean(ranks)), 3) if ranks else None,
                     "planted_p99": round(float(np.quantile(p[p > 0], 0.99)), 3) if (p > 0).any() else None,
                     "ours_p99": round(float(np.quantile(o[o > 0], 0.99)), 3) if (o > 0).any() else None})
        print(json.dumps(rows[-1]), flush=True)
    if arguments.json:
        Path(arguments.json).write_text(json.dumps(rows, indent=1))


if __name__ == "__main__":
    main()
