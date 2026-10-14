#!/usr/bin/env python3
# Copyright (C) 2026 Toit contributors.
# Use of this source code is governed by an MIT-style license that can be
# found in the LICENSE file.
"""Routing halos granted against routing used, per part, on routed layouts.

    halo_use.py <task-output-dir>... [--json out.json] [--probe-dir DIR]

Each task directory is a run.py output (layout/result/<board>.kicad_pcb and
layout/placed/board-placer.json). Per footprint of the final board:

- granted: the halo the placer gave it (the pin-count rule,
  ceil(pins / 8) x 0.65 mm up to 4 mm, times the placement's scale for its
  class: `halo_scale` for parts with 10 or more pins, `small_halo_scale`
  for the others);
- ring: the body box (courtyard and pads) grown by the granted halo, less
  the body; band: the body grown by a fixed 1 mm, less the body;
- used: track area (length x (width + clearance)) and via area
  ((diameter + clearance)^2 per via, every layer) inside, over the area
  times the copper layers (the share of the routing capacity used);
- with --probe-dir (export-congestion-sample outputs, one <task>/source.json
  per task): the probe's conflict and history in tiles the band touches.

Prints per board and per pin-count bucket: halo area granted, the share of
it used, and how much is granted where (almost) nothing is routed.
"""

import argparse
import json
import math
import sys
from pathlib import Path

import pcbnew

BUCKETS = [(1, 2, "2"), (3, 4, "3-4"), (5, 9, "5-9"), (10, 16, "10-16"), (17, 32, "17-32"), (33, 10**6, "33+")]


def bucket(pins):
    for low, high, name in BUCKETS:
        if low <= pins <= high:
            return name
    return "0"


def clip(a, b, box):
    """Length of segment a-b inside box (x0, y0, x1, y1), Liang-Barsky."""
    (x0, y0), (x1, y1) = a, b
    dx, dy = x1 - x0, y1 - y0
    t0, t1 = 0.0, 1.0
    for p, q in ((-dx, x0 - box[0]), (dx, box[2] - x0), (-dy, y0 - box[1]), (dy, box[3] - y0)):
        if p == 0:
            if q < 0:
                return 0.0
            continue
        r = q / p
        if p < 0:
            t0 = max(t0, r)
        else:
            t1 = min(t1, r)
        if t0 > t1:
            return 0.0
    return (t1 - t0) * math.hypot(dx, dy)


def grow(box, by):
    return (box[0] - by, box[1] - by, box[2] + by, box[3] + by)


def area(box):
    return max(0.0, box[2] - box[0]) * max(0.0, box[3] - box[1])


def inside(point, box):
    return box[0] <= point[0] <= box[2] and box[1] <= point[1] <= box[3]


def mm_box(box2i):
    return (pcbnew.ToMM(box2i.GetLeft()), pcbnew.ToMM(box2i.GetTop()), pcbnew.ToMM(box2i.GetRight()),
            pcbnew.ToMM(box2i.GetBottom()))


def load_probe(path):
    header = json.loads(path.read_text())
    data = path.with_suffix(".bin").read_bytes()
    import array
    out = {"frame": header["frame"], "layers": header["layers"]}
    for name in ("conflict", "history", "usage"):
        entry = header["arrays"].get(name)
        if not entry:
            continue
        values = array.array("f")
        count = 1
        for size in entry["shape"]:
            count *= size
        values.frombytes(data[entry["offset"]:entry["offset"] + 4 * count])
        planes, ty, tx = entry["shape"]
        summed = [0.0] * (ty * tx)
        for plane in range(planes):
            for tile in range(ty * tx):
                summed[tile] += values[plane * ty * tx + tile]
        out[name] = summed
    return out


def probe_in(probe, box, body):
    """Probe values (summed over planes) in tiles, weighted by the share of
    each tile that the band (box less body) covers."""
    frame = probe["frame"]
    tile_mm = frame["pitch"] * frame["tile"]
    ox = frame["origin"][0] - frame["pitch"] / 2
    oy = frame["origin"][1] - frame["pitch"] / 2
    result = {}
    tx0 = max(0, int((box[0] - ox) // tile_mm))
    tx1 = min(frame["tiles_x"] - 1, int((box[2] - ox) // tile_mm))
    ty0 = max(0, int((box[1] - oy) // tile_mm))
    ty1 = min(frame["tiles_y"] - 1, int((box[3] - oy) // tile_mm))
    for name in ("conflict", "history"):
        if name not in probe:
            continue
        total = 0.0
        for ty in range(ty0, ty1 + 1):
            for tx in range(tx0, tx1 + 1):
                tile = (ox + tx * tile_mm, oy + ty * tile_mm, ox + (tx + 1) * tile_mm, oy + (ty + 1) * tile_mm)
                both = (max(tile[0], box[0]), max(tile[1], box[1]), min(tile[2], box[2]), min(tile[3], box[3]))
                inner = (max(tile[0], body[0]), max(tile[1], body[1]), min(tile[2], body[2]), min(tile[3], body[3]))
                share = (area(both) - area(inner)) / (tile_mm * tile_mm)
                if share > 0:
                    total += share * probe[name][ty * frame["tiles_x"] + tx]
        result[name] = total
    return result


def measure(task_dir, probe_dir=None):
    task_dir = Path(task_dir)
    placed = json.loads((task_dir / "layout/placed/board-placer.json").read_text())
    board_id = placed["board_id"]
    layout_cfg = json.loads((task_dir / "layout.json").read_text()) if (task_dir / "layout.json").exists() else {}
    halo_factor = layout_cfg.get("placer", {}).get("halo_scale", 1.0)
    board = pcbnew.LoadBoard(str(task_dir / f"layout/result/{board_id}.kicad_pcb"))
    layers = board.GetCopperLayerCount()
    fixed = {fp["reference"]: fp["fixed"] for fp in placed["footprints"]}
    scales = (placed["small_halo_scale"], placed["halo_scale"])
    probe = None
    if probe_dir:
        path = Path(probe_dir) / task_dir.name / "source.json"
        if path.exists():
            probe = load_probe(path)
    # Tracks and vias once.
    segments = []
    vias = []
    for item in board.GetTracks():
        kind = item.GetClass()
        if kind == "PCB_VIA":
            position = item.GetPosition()
            size = pcbnew.ToMM(item.GetWidth(pcbnew.F_Cu)) + pcbnew.ToMM(item.GetOwnClearance(pcbnew.F_Cu))
            vias.append(((pcbnew.ToMM(position.x), pcbnew.ToMM(position.y)), size * size, item.GetNetname()))
        else:
            start, end = item.GetStart(), item.GetEnd()
            pitch = pcbnew.ToMM(item.GetWidth()) + pcbnew.ToMM(item.GetOwnClearance(item.GetLayer()))
            segments.append(((pcbnew.ToMM(start.x), pcbnew.ToMM(start.y)), (pcbnew.ToMM(end.x), pcbnew.ToMM(end.y)),
                             pitch, item.GetNetname()))
    # RUDY from the pads (what a placer sees before routing): each net's
    # half perimeter times the track pitch over its box (at least 1.6 mm a
    # side), as a density.
    pitches = sorted(pitch for _, _, pitch, _ in segments) or [0.45]
    pitch = pitches[len(pitches) // 2]
    pins_of = {}
    for fp in board.GetFootprints():
        for pad in fp.Pads():
            net = pad.GetNetname()
            if net and not net.startswith("unconnected-"):
                position = pad.GetPosition()
                pins_of.setdefault(net, []).append((pcbnew.ToMM(position.x), pcbnew.ToMM(position.y)))
    poured = {zone.GetNetname() for zone in board.Zones() if not zone.GetIsRuleArea()}
    rudy = []
    for net, points in pins_of.items():
        if len(points) < 2 or net in poured:
            continue
        xs, ys = [p[0] for p in points], [p[1] for p in points]
        box = [min(xs), min(ys), max(xs), max(ys)]
        hpwl = box[2] - box[0] + box[3] - box[1]
        for axis in (0, 1):
            missing = 1.6 - (box[axis + 2] - box[axis])
            if missing > 0:
                box[axis] -= missing / 2
                box[axis + 2] += missing / 2
        rudy.append((tuple(box), hpwl * pitch / area(box)))
    outline = mm_box(board.GetBoardEdgesBoundingBox())
    board_area = area(outline)
    total_track = sum(clip(a, b, outline) * p for a, b, p, _ in segments) + sum(
        size * layers for _, size, _ in vias)
    mean_use = total_track / (board_area * layers)

    def rudy_in(outer, body):
        total = 0.0
        for box, density in rudy:
            both = (max(box[0], outer[0]), max(box[1], outer[1]), min(box[2], outer[2]), min(box[3], outer[3]))
            inner = (max(box[0], body[0]), max(box[1], body[1]), min(box[2], body[2]), min(box[3], body[3]))
            total += density * (area(both) - area(inner))
        return total / max(area(outer) - area(body), 1e-9) / layers

    parts = []
    for fp in board.GetFootprints():
        reference = fp.GetReference()
        nets = set()
        pins = 0
        pad_box = None
        for pad in fp.Pads():
            net = pad.GetNetname()
            box = mm_box(pad.GetBoundingBox())
            pad_box = box if pad_box is None else (min(pad_box[0], box[0]), min(pad_box[1], box[1]),
                                                   max(pad_box[2], box[2]), max(pad_box[3], box[3]))
            if net and not net.startswith("unconnected-"):
                pins += 1
                nets.add(net)
        if pins == 0 or pad_box is None:
            continue
        layer = pcbnew.B_CrtYd if fp.IsFlipped() else pcbnew.F_CrtYd
        courtyard = fp.GetCourtyard(layer)
        body = pad_box
        if courtyard.OutlineCount():
            c = mm_box(courtyard.BBox())
            body = (min(body[0], c[0]), min(body[1], c[1]), max(body[2], c[2]), max(body[3], c[3]))
        requested = min(math.ceil(pins / 8) * 0.65, 4.0) * halo_factor
        granted = requested * scales[1 if pins >= 10 else 0]

        def used(outer):
            track = own = 0.0
            for a, b, pitch, net in segments:
                length = clip(a, b, outer) - clip(a, b, body)
                if length > 1e-9:
                    track += length * pitch
                    if net in nets:
                        own += length * pitch
            via_area = 0.0
            via_count = 0
            for point, size, net in vias:
                if inside(point, outer) and not inside(point, body):
                    via_area += size * layers
                    via_count += 1
            capacity = (area(outer) - area(body)) * layers
            return capacity, track + via_area, own, via_count

        ring = grow(body, granted)
        band = grow(body, 1.0)
        ring_capacity, ring_used, ring_own, ring_vias = used(ring) if granted > 0 else (0.0, 0.0, 0.0, 0)
        band_capacity, band_used, band_own, band_vias = used(band)
        entry = {
            "ref": reference, "pins": pins, "bucket": bucket(pins), "fixed": fixed.get(reference, True),
            "requested": requested, "granted": granted,
            "ring_area": ring_capacity / layers, "ring_use": ring_used / ring_capacity if ring_capacity else None,
            "ring_own": ring_own / ring_capacity if ring_capacity else None, "ring_vias": ring_vias,
            "band_area": band_capacity / layers, "band_use": band_used / band_capacity, "band_own": band_own / band_capacity,
            "band_vias": band_vias, "band_rudy": rudy_in(band, body),
            "body_area": area(body),
        }
        if probe:
            entry.update({f"probe_{k}": v for k, v in probe_in(probe, band, body).items()})
        parts.append(entry)
    return {"task": task_dir.name, "board": board_id, "layers": layers, "scales": scales, "mean_use": mean_use,
            "parts": parts}


def main():
    parser = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    parser.add_argument("tasks", nargs="+")
    parser.add_argument("--json", type=Path)
    parser.add_argument("--probe-dir", type=Path)
    arguments = parser.parse_args()
    results = []
    for task in arguments.tasks:
        if not (Path(task) / "layout/result").exists():
            continue
        try:
            results.append(measure(task, arguments.probe_dir))
        except Exception as error:  # noqa: BLE001
            print(f"{task}: {error}", file=sys.stderr)
    if arguments.json:
        arguments.json.write_text(json.dumps(results))
    for result in results:
        movable = [p for p in result["parts"] if not p["fixed"]]
        granted = sum(p["ring_area"] for p in movable)
        used = sum(p["ring_area"] * (p["ring_use"] or 0) for p in movable)
        idle = sum(p["ring_area"] for p in movable if p["granted"] > 0 and (p["ring_use"] or 0) < 0.05)
        print(f"{result['task'][:50]:50} L{result['layers']} scales {result['scales']} movable {len(movable):4d} "
              f"halo area {granted:8.0f} mm2, used {100 * used / max(granted, 1e-9):5.1f} %, idle (<5 %) "
              f"{100 * idle / max(granted, 1e-9):5.1f} %")


if __name__ == "__main__":
    sys.exit(main())
