#!/usr/bin/env python3
# Copyright (C) 2026 Toit contributors.
"""Routing statistics of a KiCad board, the same for planted, routed and
designers' boards.

    experiments/planted/stats.py board.kicad_pcb [...] [--json out.json]

Per net (two or more pads): pads, parts, copper length, vias, the
Euclidean minimum spanning tree over its pad centres, and the detour ratio
length / MST. Nets with a copper zone (pours) are counted but left out of
the length, detour and via statistics: their copper is mostly fill.
Per board: area per part, nets per part, and track utilisation on 1.6 mm
tiles (the router's 16-node tiles at 0.1 mm): per copper layer and tile
inside the outline, the track length in the tile times (width + clearance)
over the tile's area; 1.0 is a tile packed with parallel tracks."""

import argparse
import json
import math
import sys

import numpy as np

TILE = 1.6


def mm(value):
    return value / 1e6


def mst_length(points):
    count = len(points)
    if count < 2:
        return 0.0
    pts = np.array(points)
    inside = np.zeros(count, dtype=bool)
    inside[0] = True
    best = np.hypot(*(pts - pts[0]).T)
    total = 0.0
    for _ in range(count - 1):
        candidates = np.where(inside, np.inf, best)
        k = int(np.argmin(candidates))
        total += candidates[k]
        inside[k] = True
        best = np.minimum(best, np.hypot(*(pts - pts[k]).T))
    return float(total)


def point_in_polygon(x, y, poly):
    inside = np.zeros(x.shape, dtype=bool)
    count = len(poly)
    for index in range(count):
        ax, ay = poly[index]
        bx, by = poly[(index + 1) % count]
        crosses = (ay > y) != (by > y)
        with np.errstate(divide="ignore", invalid="ignore"):
            xcross = ax + (y - ay) * (bx - ax) / (by - ay)
        inside ^= crosses & (x < xcross)
    return inside


def board_outline(board):
    import pcbnew
    polys = pcbnew.SHAPE_POLY_SET()
    board.GetBoardPolygonOutlines(polys, True)
    outlines = []
    area = 0.0
    for index in range(polys.OutlineCount()):
        line = polys.Outline(index)
        points = [(mm(line.CPoint(i).x), mm(line.CPoint(i).y)) for i in range(line.PointCount())]
        outlines.append(points)
        a = 0.0
        for i in range(len(points)):
            x0, y0 = points[i]
            x1, y1 = points[(i + 1) % len(points)]
            a += x0 * y1 - x1 * y0
        area += abs(a) / 2
        # Holes in the outline (cut-outs) reduce the area.
        for h in range(polys.HoleCount(index)):
            hole = polys.Hole(index, h)
            hp = [(mm(hole.CPoint(i).x), mm(hole.CPoint(i).y)) for i in range(hole.PointCount())]
            a = 0.0
            for i in range(len(hp)):
                x0, y0 = hp[i]
                x1, y1 = hp[(i + 1) % len(hp)]
                a += x0 * y1 - x1 * y0
            area -= abs(a) / 2
    return outlines, area


def copper_layers(board):
    import pcbnew
    return [layer for layer in board.GetEnabledLayers().CuStack()]


def board_stats(path, include_nets=True):
    import pcbnew
    board = pcbnew.LoadBoard(str(path))
    if board is None:
        raise RuntimeError(f"cannot load {path}")
    layers = copper_layers(board)
    layer_index = {layer: i for i, layer in enumerate(layers)}
    settings = board.GetDesignSettings()
    clearance = mm(settings.m_NetSettings.GetDefaultNetclass().GetClearance())
    outlines, area = board_outline(board)

    zone_nets = set()
    for zone in board.Zones():
        if zone.GetIsRuleArea():
            continue
        if zone.GetNetCode() > 0:
            zone_nets.add(zone.GetNetname())

    nets = {}
    parts = 0
    pads_total = 0
    part_nets = []
    for fp in board.GetFootprints():
        names = set()
        pads = list(fp.Pads())
        if not pads:
            continue
        parts += 1
        for pad in pads:
            pads_total += 1
            name = pad.GetNetname()
            if pad.GetNetCode() <= 0 or name.startswith("unconnected-"):
                continue
            net = nets.setdefault(name, {"pads": 0, "parts": set(), "points": [], "length": 0.0, "vias": 0})
            net["pads"] += 1
            net["parts"].add(fp.GetReference() + "@" + str(id(fp)))
            p = pad.GetPosition()
            net["points"].append((mm(p.x), mm(p.y)))
            names.add(name)
        part_nets.append(names)

    tracks = []
    for item in board.GetTracks():
        name = item.GetNetname()
        cls = item.GetClass()
        if cls == "PCB_VIA":
            if name in nets:
                nets[name]["vias"] += 1
            continue
        length = mm(item.GetLength())
        if name in nets:
            nets[name]["length"] += length
        if cls == "PCB_ARC":
            # Chord sampling is good enough for tile statistics.
            s, m, e = item.GetStart(), item.GetMid(), item.GetEnd()
            pts = [(mm(s.x), mm(s.y)), (mm(m.x), mm(m.y)), (mm(e.x), mm(e.y))]
        else:
            s, e = item.GetStart(), item.GetEnd()
            pts = [(mm(s.x), mm(s.y)), (mm(e.x), mm(e.y))]
        layer = item.GetLayer()
        if layer in layer_index:
            tracks.append((layer_index[layer], pts, mm(item.GetWidth())))

    # Nets with at least two pads are connections to make.
    multi = {name: net for name, net in nets.items() if net["pads"] >= 2}
    rows = []
    for name, net in multi.items():
        mst = mst_length(net["points"])
        pour = name in zone_nets
        row = {"net": name, "pads": net["pads"], "parts": len(net["parts"]), "mst_mm": round(mst, 3),
               "length_mm": round(net["length"], 3), "vias": net["vias"], "pour": pour}
        if not pour and net["length"] > 0 and mst > 2.0:
            row["detour"] = round(net["length"] / mst, 4)
        rows.append(row)

    # Track utilisation per tile.
    xs = [p[0] for o in outlines for p in o]
    ys = [p[1] for o in outlines for p in o]
    utilisation = []
    if xs:
        x0, y0 = min(xs), min(ys)
        nx = int(math.ceil((max(xs) - x0) / TILE))
        ny = int(math.ceil((max(ys) - y0) / TILE))
        grid = np.zeros((len(layers), ny, nx))
        for layer, pts, width in tracks:
            for (ax, ay), (bx, by) in zip(pts, pts[1:]):
                length = math.hypot(bx - ax, by - ay)
                n = max(1, int(length / 0.05))
                t = (np.arange(n) + 0.5) / n
                sx = ax + t * (bx - ax)
                sy = ay + t * (by - ay)
                ix = np.clip(((sx - x0) / TILE).astype(int), 0, nx - 1)
                iy = np.clip(((sy - y0) / TILE).astype(int), 0, ny - 1)
                np.add.at(grid[layer], (iy, ix), length / n * (width + clearance))
        cx, cy = np.meshgrid(x0 + (np.arange(nx) + 0.5) * TILE, y0 + (np.arange(ny) + 0.5) * TILE)
        inside = np.zeros(cx.shape, dtype=bool)
        for o in outlines:
            inside ^= point_in_polygon(cx, cy, o)
        for layer in range(len(layers)):
            utilisation.extend((grid[layer][inside] / (TILE * TILE)).tolist())

    signal = [r for r in rows if not r["pour"]]
    detours = [r["detour"] for r in signal if "detour" in r]
    routed = [r for r in signal if r["length_mm"] > 0]
    util = np.array(utilisation) if utilisation else np.zeros(1)
    nets_per_part = [len(n) for n in part_nets]
    summary = {
        "board": str(path),
        "layers": len(layers),
        "area_mm2": round(area, 1),
        "parts": parts,
        "pads": pads_total,
        "area_per_part_mm2": round(area / parts, 2) if parts else None,
        "nets": len(rows),
        "pour_nets": sum(1 for r in rows if r["pour"]),
        "clearance_mm": clearance,
        "pads_per_net": summarize([r["pads"] for r in rows]),
        "nets_per_part": summarize(nets_per_part),
        "detour": summarize(detours),
        "vias_per_net": summarize([r["vias"] for r in routed]),
        "one_layer_nets": round(sum(1 for r in routed if r["vias"] == 0) / len(routed), 3) if routed else None,
        "length_over_mst": round(sum(r["length_mm"] for r in routed) / max(1e-9, sum(r["mst_mm"] for r in routed)), 3) if routed else None,
        "tile_utilisation": summarize(util.tolist(), quantiles=(0.5, 0.75, 0.9, 0.95, 0.99)),
        "tiles_over_0_5": round(float((util > 0.5).mean()), 4),
        "tile_utilisation_used": summarize(util[util > 0].tolist(), quantiles=(0.5, 0.9, 0.99)),
        "tiles_used": round(float((util > 0).mean()), 4),
    }
    if include_nets:
        summary["per_net"] = rows
    return summary


def summarize(values, quantiles=(0.1, 0.25, 0.5, 0.75, 0.9)):
    if not values:
        return None
    a = np.array(values, dtype=float)
    out = {"n": int(a.size), "mean": round(float(a.mean()), 4)}
    for q in quantiles:
        out[f"p{int(q * 100)}"] = round(float(np.quantile(a, q)), 4)
    out["max"] = round(float(a.max()), 4)
    return out


def main():
    parser = argparse.ArgumentParser()
    parser.add_argument("boards", nargs="*")
    parser.add_argument("--list", help="a file with one board path per line")
    parser.add_argument("--json")
    parser.add_argument("--no-nets", action="store_true")
    arguments = parser.parse_args()
    results = []
    boards = list(arguments.boards)
    if arguments.list:
        boards += [line.strip() for line in open(arguments.list) if line.strip()]
    for path in boards:
        try:
            results.append(board_stats(path, not arguments.no_nets))
        except Exception as error:  # noqa: BLE001 - a bad board is a row, not a crash
            results.append({"board": path, "error": str(error)})
            print(f"{path}: {error}", file=sys.stderr)
    text = json.dumps(results if len(results) > 1 else results[0], indent=1)
    if arguments.json:
        with open(arguments.json, "w") as out:
            out.write(text)
    else:
        print(text)


if __name__ == "__main__":
    main()
