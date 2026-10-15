#!/usr/bin/env python3
# Copyright (C) 2026 Toit contributors.
"""Other footprints' pads inside a footprint's solder-mask openings.

    python3 -I pads_under_openings.py board.kicad_pcb [...] [--json out.json] [--quiet]

A footprint's openings on a side are its graphics on that side's mask layer
(F.Mask, B.Mask): filled polygons, rectangles and circles by their area,
lines, arcs and unfilled outlines by their stroke. A pad of another
footprint is under an opening when its box (copper or mask on that side)
meets the opening's shape. KiCad reports solder_mask_bridge for each such
pad of another net than the opening's (same-net pairs are exempt; not
told apart here). Pads of the footprint itself are not counted.
"""

import argparse
import json
import math
import sys
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parent.parent / "bodies"))
from courtless_census import child, children, head, layer_of, layers_of, parse, xy  # noqa: E402


def reference_of(footprint):
    for item in footprint:
        if head(item) == "fp_text" and len(item) > 2 and item[1] == "reference":
            return item[2]
        if head(item) == "property" and len(item) > 2 and item[1] == "Reference":
            return item[2]
    return "?"


def at_of(node):
    at = child(node, "at")
    angle = float(at[3]) if len(at) > 3 and at[3] not in ("unlocked",) else 0.0
    return float(at[1]), float(at[2]), angle


def to_board(at, point):
    x, y, angle = at
    radians = math.radians(angle)
    cos, sin = math.cos(radians), math.sin(radians)
    lx, ly = point
    return (x + lx * cos + ly * sin, y - lx * sin + ly * cos)


def segment_polygon(a, b, width):
    """A stroke as a rectangle around the segment (ends squared)."""
    half = max(width, 0.001) / 2.0
    dx, dy = b[0] - a[0], b[1] - a[1]
    length = math.hypot(dx, dy)
    if length < 1.0e-9:
        return [(a[0] - half, a[1] - half), (a[0] + half, a[1] - half), (a[0] + half, a[1] + half), (a[0] - half, a[1] + half)]
    ux, uy = dx / length * half, dy / length * half
    nx, ny = -uy, ux
    return [(a[0] - ux + nx, a[1] - uy + ny), (b[0] + ux + nx, b[1] + uy + ny), (b[0] + ux - nx, b[1] + uy - ny), (a[0] - ux - nx, a[1] - uy - ny)]


def circle_points(center, radius, count=32):
    return [(center[0] + radius * math.cos(2 * math.pi * k / count), center[1] + radius * math.sin(2 * math.pi * k / count)) for k in range(count)]


def stroke_width(item):
    stroke = child(item, "stroke")
    if stroke and child(stroke, "width"):
        return float(child(stroke, "width")[1])
    if child(item, "width"):
        return float(child(item, "width")[1])
    return 0.0


def filled(item):
    fill = child(item, "fill")
    if fill is None:
        # Before KiCad 7, an fp_poly is filled; rectangles and circles not.
        return head(item) == "fp_poly"
    return len(fill) > 1 and fill[1] in ("yes", "solid")


def opening_polygons(item):
    """Polygons (own frame) a mask graphic opens."""
    kind = head(item)
    width = stroke_width(item)
    outline = []
    closed = True
    if kind == "fp_rect":
        a, b = xy(item, "start"), xy(item, "end")
        outline = [a, (b[0], a[1]), b, (a[0], b[1])]
    elif kind == "fp_poly":
        pts = child(item, "pts")
        outline = [(float(p[1]), float(p[2])) for p in pts[1:] if head(p) == "xy"] if pts else []
    elif kind == "fp_circle":
        center, end = xy(item, "center"), xy(item, "end")
        outline = circle_points(center, math.dist(center, end))
    elif kind == "fp_line":
        outline, closed = [xy(item, "start"), xy(item, "end")], False
    elif kind == "fp_arc":
        outline, closed = [p for p in (xy(item, "start"), xy(item, "mid"), xy(item, "end")) if p], False
    if len(outline) < 2:
        return []
    polygons = []
    if closed and filled(item) and len(outline) >= 3:
        if width > 0:
            # The stroke widens the filled shape: approximated by the
            # strokes around it.
            pass
        polygons.append(outline)
    pairs = list(zip(outline, outline[1:] + ([outline[0]] if closed else [])))
    if width > 0 or not polygons:
        polygons.extend(segment_polygon(a, b, width) for a, b in pairs)
    return polygons


def point_in_polygon(point, polygon):
    inside = False
    for index in range(len(polygon)):
        a, b = polygon[index], polygon[(index + 1) % len(polygon)]
        if (a[1] > point[1]) != (b[1] > point[1]):
            x = a[0] + (point[1] - a[1]) / (b[1] - a[1]) * (b[0] - a[0])
            if point[0] < x:
                inside = not inside
    return inside


def segments_cross(p, q, r, s):
    def orient(a, b, c):
        return (b[0] - a[0]) * (c[1] - a[1]) - (b[1] - a[1]) * (c[0] - a[0])

    d1, d2, d3, d4 = orient(r, s, p), orient(r, s, q), orient(p, q, r), orient(p, q, s)
    return (d1 > 0) != (d2 > 0) and (d3 > 0) != (d4 > 0) and d1 != 0 and d2 != 0 and d3 != 0 and d4 != 0


def polygon_meets_box(polygon, box, eps=1.0e-4):
    """Whether a polygon and a box (shrunk by `eps`: touching does not
    count) share area."""
    box = (box[0] + eps, box[1] + eps, box[2] - eps, box[3] - eps)
    if box[0] >= box[2] or box[1] >= box[3]:
        return False
    xs, ys = [p[0] for p in polygon], [p[1] for p in polygon]
    if max(xs) <= box[0] or min(xs) >= box[2] or max(ys) <= box[1] or min(ys) >= box[3]:
        return False
    corners = [(box[0], box[1]), (box[2], box[1]), (box[2], box[3]), (box[0], box[3])]
    if any(point_in_polygon(c, polygon) for c in corners):
        return True
    if any(box[0] < p[0] < box[2] and box[1] < p[1] < box[3] for p in polygon):
        return True
    edges = list(zip(corners, corners[1:] + corners[:1]))
    for a, b in zip(polygon, polygon[1:] + polygon[:1]):
        for c, d in edges:
            if segments_cross(a, b, c, d):
                return True
    return False


def pad_box(pad, footprint_at):
    """The pad's box in board coordinates (pad angles are absolute)."""
    px, py, angle = at_of(pad)
    size = child(pad, "size")
    sx, sy = float(size[1]), float(size[2])
    if head(child(pad, "primitives") or []) == "primitives":
        sx = sy = max(sx, sy)
    center = to_board(footprint_at, (px, py))
    turn = math.radians(angle)
    hx = (abs(math.cos(turn)) * sx + abs(math.sin(turn)) * sy) / 2
    hy = (abs(math.sin(turn)) * sx + abs(math.cos(turn)) * sy) / 2
    return (center[0] - hx, center[1] - hy, center[0] + hx, center[1] + hy)


def pad_sides(pad):
    sides = set()
    for layer in layers_of(pad):
        if layer in ("*.Cu", "*.Mask", "F&B.Cu"):
            sides |= {"F", "B"}
        elif layer in ("F.Cu", "F.Mask"):
            sides.add("F")
        elif layer in ("B.Cu", "B.Mask"):
            sides.add("B")
    return sides


def pad_net(pad):
    net = child(pad, "net")
    if not net or len(net) < 2:
        return ""
    return net[-1] if len(net) > 2 else net[1]


def census(path):
    tree = parse(Path(path).read_text(errors="replace"))
    footprints = []
    for footprint in (item for item in tree if head(item) == "footprint"):
        at = at_of(footprint)
        openings = {"F": [], "B": []}
        for item in footprint:
            layer = layer_of(item) if isinstance(item, list) else None
            if layer in ("F.Mask", "B.Mask") and head(item) in ("fp_line", "fp_rect", "fp_arc", "fp_circle", "fp_poly"):
                for polygon in opening_polygons(item):
                    openings[layer[0]].append([to_board(at, p) for p in polygon])
        pads = [(pad_box(pad, at), pad_sides(pad), pad_net(pad)) for pad in children(footprint, "pad") if child(pad, "size")]
        footprints.append({"reference": reference_of(footprint), "openings": openings, "pads": pads})
    findings = []
    for owner in footprints:
        for side, polygons in owner["openings"].items():
            if not polygons:
                continue
            for other in footprints:
                if other is owner:
                    continue
                under = [
                    (box, net)
                    for box, sides, net in other["pads"]
                    if side in sides and any(polygon_meets_box(polygon, box) for polygon in polygons)
                ]
                if under:
                    findings.append({
                        "opening": owner["reference"],
                        "side": side,
                        "part": other["reference"],
                        "pads": len(under),
                        "nets": sorted({net for _, net in under}),
                    })
    return findings


def main():
    parser = argparse.ArgumentParser()
    parser.add_argument("boards", nargs="+")
    parser.add_argument("--json")
    parser.add_argument("--quiet", action="store_true")
    args = parser.parse_args()
    result = {}
    for board in args.boards:
        findings = census(board)
        result[board] = findings
        pads = sum(f["pads"] for f in findings)
        parts = len({(f["part"]) for f in findings})
        print(f"{board}: {pads} pads of {parts} parts under other footprints' mask openings")
        if not args.quiet:
            for f in findings:
                print(f"  {f['opening']} {f['side']}.Mask: {f['part']} {f['pads']} pads {f['nets']}")
    if args.json:
        Path(args.json).write_text(json.dumps(result, indent=1))


if __name__ == "__main__":
    main()
