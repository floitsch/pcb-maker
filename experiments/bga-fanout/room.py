#!/usr/bin/env python3
# Copyright (C) 2026 Toit contributors.
# Use of this source code is governed by an MIT-style license that can be
# found in the LICENSE file.
"""The room a placement keeps around a grid array (docs/placer.md, "Room
around grid arrays (2026-11-04)").

    room.py --reference U30 NAME=BOARD.kicad_pcb [NAME=BOARD ...] [--json OUT]

For each board (KiCad's pcbnew) and the array `--reference` (a grid as
census.py finds it), in the array's frame and from its ball field (the box
of the ball centres grown by half a pitch):

- per side (the array's own side and the far side): the other parts by the
  distance of their nearest pad from the field (inside it, then bands of
  0.5 mm), the capacitors among them that share a net with the array (its
  decoupling), and the parts whose courtyard reaches into the field;
- the channels: per outer-ring ball (rings 0 to `--surface-rings` less
  one), straight out from its edge of the field within half a pitch of
  its column or row, the free length on the array's side before another
  part's pad or courtyard (cut at 10 mm): the room its surface escape has;
- the far side under the field: the other parts' pads that cover a
  diagonal gap of an inner ball (where a dog-bone via would go), and those
  in the depopulated middle;
- the vias: per net of the array's balls, the nearest via of the net
  outside the field (the first via of its escape), by its distance from
  the field, split by the ring of the net's ball (the outer rings' and the
  inner rings').
"""

import argparse
import json
import math
import os
import statistics
import sys

sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
import census  # noqa: E402

MM = census.MM
BANDS = [0.0, 0.5, 1.0, 1.5, 2.0, 3.0, 4.0, 6.0]


def box_distance(a, b):
    """The gap between two boxes (x0, y0, x1, y1); 0 where they meet."""
    dx = max(a[0] - b[2], b[0] - a[2], 0.0)
    dy = max(a[1] - b[3], b[1] - a[3], 0.0)
    return math.hypot(dx, dy)


def local_box(fp, bbox):
    """A board bounding box (pcbnew BOX2I) in `fp`'s frame, as the box of
    its turned corners (mm)."""
    corners = [
        (bbox.GetLeft(), bbox.GetTop()),
        (bbox.GetRight(), bbox.GetTop()),
        (bbox.GetLeft(), bbox.GetBottom()),
        (bbox.GetRight(), bbox.GetBottom()),
    ]
    points = [census.local(fp, corner) for corner in corners]
    xs = [p[0] for p in points]
    ys = [p[1] for p in points]
    return (min(xs), min(ys), max(xs), max(ys))


def courtyard_box(board, fp, side):
    import pcbnew

    layer = pcbnew.F_CrtYd if side == "F" else pcbnew.B_CrtYd
    try:
        poly = fp.GetCourtyard(layer)
        if poly.OutlineCount() > 0:
            return poly.BBox()
    except Exception:
        pass
    return None


def side_of(board, fp):
    return "B" if fp.IsFlipped() else "F"


def measure(path, reference, surface_rings):
    import pcbnew

    board = census.load(path)
    array = board.FindFootprintByReference(reference)
    grid = census.grid_of(array)
    if grid is None:
        raise SystemExit(f"{path}: {reference} is no grid array")
    pitch = grid["pitch"]
    half = pitch / 2.0
    box = grid["box"]
    field = (box[0] - half, box[1] - half, box[2] + half, box[3] + half)
    own_side = side_of(board, array)
    far = "B" if own_side == "F" else "F"
    array_nets = {pad.GetNetname() for pad in array.Pads() if pad.GetNetCode() > 0}
    occupied = {(round((p[0] - box[0]) / pitch), round((p[1] - box[1]) / pitch)) for p in grid["points"]}
    columns = round((box[2] - box[0]) / pitch) + 1
    rows = round((box[3] - box[1]) / pitch) + 1
    middle = [
        (x, y)
        for x in range(columns)
        for y in range(rows)
        if (x, y) not in occupied
    ]

    parts = []
    for fp in board.GetFootprints():
        if fp.GetReference() == reference:
            continue
        pads = [local_box(array, pad.GetBoundingBox()) for pad in fp.Pads()]
        if not pads:
            continue
        side = side_of(board, fp)
        court = courtyard_box(board, fp, side)
        court = local_box(array, court) if court is not None else None
        nets = {pad.GetNetname() for pad in fp.Pads() if pad.GetNetCode() > 0}
        through = any(pad.GetAttribute() in (pcbnew.PAD_ATTRIB_PTH, pcbnew.PAD_ATTRIB_NPTH) for pad in fp.Pads())
        parts.append(
            {
                "reference": fp.GetReference(),
                "side": side,
                "through": through,
                "pads": pads,
                "courtyard": court,
                "pad_distance": min(box_distance(field, pad) for pad in pads),
                "courtyard_distance": box_distance(field, court) if court else None,
                "decoupling": fp.GetReference().startswith("C") and bool(nets & array_nets),
                "fixed": fp.IsLocked(),
            }
        )

    def bands(selected):
        counts = {"inside": 0}
        for low, high in zip(BANDS, BANDS[1:]):
            counts[f"{low:g}-{high:g}"] = 0
        for part in selected:
            d = part["pad_distance"]
            if d <= 0.0:
                counts["inside"] += 1
                continue
            for low, high in zip(BANDS, BANDS[1:]):
                if low < d <= high:
                    counts[f"{low:g}-{high:g}"] += 1
        return counts

    sides = {}
    for name, side in (("own", own_side), ("far", far)):
        here = [p for p in parts if p["side"] == side or p["through"]]
        sides[name] = {
            "layer": side,
            "parts_by_pad_distance": bands(here),
            "decoupling_by_pad_distance": bands([p for p in here if p["decoupling"]]),
            "nearest_pad_mm": round(min((p["pad_distance"] for p in here), default=99.0), 3),
            "courtyards_into_field": sorted(
                p["reference"] for p in here if p["courtyard"] and box_distance(field, p["courtyard"]) <= 0.0
            ),
        }

    # The channels of the outer rings on the array's side.
    own = [p for p in parts if p["side"] == own_side or p["through"]]
    channels = []
    limit = 10.0
    for point, ring in zip(grid["points"], grid["rings"]):
        if ring >= surface_rings:
            continue
        x, y = point
        reach = {
            "W": x - box[0],
            "E": box[2] - x,
            "N": y - box[1],
            "S": box[3] - y,
        }
        direction = min(reach, key=reach.get)
        if direction in ("W", "E"):
            lane = (y - half, y + half)
            start = field[0] if direction == "W" else field[2]
        else:
            lane = (x - half, x + half)
            start = field[1] if direction == "N" else field[3]
        free = limit
        for part in own:
            for item in part["pads"] + ([part["courtyard"]] if part["courtyard"] else []):
                if direction in ("W", "E"):
                    if item[3] < lane[0] or item[1] > lane[1]:
                        continue
                    d = start - item[2] if direction == "W" else item[0] - start
                else:
                    if item[2] < lane[0] or item[0] > lane[1]:
                        continue
                    d = start - item[3] if direction == "N" else item[1] - start
                if d > -pitch:
                    free = min(free, max(d, 0.0))
        channels.append({"ring": ring, "direction": direction, "free_mm": round(free, 3)})

    # The far side under the field: pads over an inner ball's diagonal gaps
    # (any of its four) and pads in the depopulated middle.
    far_parts = [p for p in parts if p["side"] == far and not p["through"]]
    gaps = set()
    for point, ring in zip(grid["points"], grid["rings"]):
        if ring < surface_rings:
            continue
        for sx in (-1, 1):
            for sy in (-1, 1):
                gaps.add((round(point[0] + sx * half, 4), round(point[1] + sy * half, 4)))
    via_half = 0.2 + 0.15  # a 0.4 mm via and the class clearance
    over_gaps = set()
    in_middle = set()
    under = set()
    for part in far_parts:
        for pad in part["pads"]:
            if box_distance(field, pad) > 0.0:
                continue
            under.add(part["reference"])
            for gx, gy in gaps:
                if box_distance(pad, (gx - via_half, gy - via_half, gx + via_half, gy + via_half)) <= 0.0:
                    over_gaps.add(part["reference"])
            centre = ((pad[0] + pad[2]) / 2.0, (pad[1] + pad[3]) / 2.0)
            cell = (round((centre[0] - box[0]) / pitch), round((centre[1] - box[1]) / pitch))
            if cell in middle:
                in_middle.add(part["reference"])

    # The first via of each net outside the field.
    net_ring = {}
    for pad, ring in zip(grid["pads"], grid["rings"]):
        name = pad.GetNetname()
        if pad.GetNetCode() > 0:
            net_ring[name] = min(net_ring.get(name, 99), ring)
    first_via = {}
    for track in board.GetTracks():
        if track.GetClass() not in ("PCB_VIA", "VIA"):
            continue
        name = track.GetNetname()
        if name not in net_ring:
            continue
        at = census.local(array, (track.GetPosition().x, track.GetPosition().y))
        r = track.GetWidth(pcbnew.F_Cu) / MM / 2.0 if hasattr(track, "GetWidth") else 0.2
        d = box_distance(field, (at[0] - r, at[1] - r, at[0] + r, at[1] + r))
        if d <= 0.0:
            continue
        first_via[name] = min(first_via.get(name, 99.0), d)

    def summary(values):
        if not values:
            return None
        values = sorted(values)
        return {
            "n": len(values),
            "p10": round(values[len(values) // 10], 3),
            "median": round(statistics.median(values), 3),
            "p90": round(values[(9 * len(values)) // 10], 3),
        }

    outer_nets = [n for n, r in net_ring.items() if r < surface_rings and n in first_via]
    inner_nets = [n for n, r in net_ring.items() if r >= surface_rings and n in first_via]
    free = [c["free_mm"] for c in channels]
    return {
        "path": path,
        "side": own_side,
        "pitch": round(pitch, 4),
        "grid": [rows, columns],
        "field_mm": [round(field[2] - field[0], 3), round(field[3] - field[1], 3)],
        "sides": sides,
        "channels": {
            "balls": len(channels),
            "free_mm": summary(free),
            "under_1mm": sum(1 for f in free if f < 1.0),
            "under_2mm": sum(1 for f in free if f < 2.0),
            "by_ring": {
                str(ring): summary([c["free_mm"] for c in channels if c["ring"] == ring])
                for ring in range(surface_rings)
            },
        },
        "far_under_field": {
            "parts": sorted(under),
            "over_inner_gaps": sorted(over_gaps),
            "in_middle": sorted(in_middle),
            "middle_cells": len(middle),
        },
        "first_via_outside_mm": {
            "outer_ring_nets": summary([first_via[n] for n in outer_nets]),
            "inner_ring_nets": summary([first_via[n] for n in inner_nets]),
        },
    }


def main():
    parser = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    parser.add_argument("boards", nargs="+", help="NAME=PATH")
    parser.add_argument("--reference", required=True)
    parser.add_argument("--surface-rings", type=int, default=2)
    parser.add_argument("--json")
    args = parser.parse_args()
    result = {}
    for entry in args.boards:
        name, path = entry.split("=", 1)
        result[name] = measure(path, args.reference, args.surface_rings)
    text = json.dumps(result, indent=1)
    if args.json:
        with open(args.json, "w") as handle:
            handle.write(text + "\n")
    print(text)


if __name__ == "__main__":
    main()
