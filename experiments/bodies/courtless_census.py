#!/usr/bin/env python3
# Copyright (C) 2026 Toit contributors.
"""Census of courtyard-less footprints in the harvested task boards.

For every footprint with pads and no courtyard (on either side), reports
whether it draws a body on its Fab layer, the box of that outline against
the box of its pads, and whether the designer put another part's pads or
body inside that box on the same side (a rule blocking the Fab box would
forbid the designer's own placement there).

    experiments/bodies/courtless_census.py build/github-tasks-v6/tasks.json [--json out.json]
"""
import argparse
import json
import math
import re
import sys
from pathlib import Path

TOKEN = re.compile(r'\(|\)|"(?:[^"\\]|\\.)*"|[^\s()]+')


def parse(text):
    stack = [[]]
    for match in TOKEN.finditer(text):
        token = match.group(0)
        if token == "(":
            stack.append([])
        elif token == ")":
            done = stack.pop()
            stack[-1].append(done)
        else:
            stack[-1].append(token[1:-1] if token.startswith('"') else token)
    return stack[0][0]


def head(node):
    return node[0] if isinstance(node, list) and node else None


def child(node, name):
    for item in node:
        if head(item) == name:
            return item
    return None


def children(node, name):
    return [item for item in node if head(item) == name]


def layer_of(node):
    layer = child(node, "layer")
    return layer[1] if layer else None


def layers_of(node):
    layers = child(node, "layers")
    return layers[1:] if layers else []


def xy(node, name):
    item = child(node, name)
    return (float(item[1]), float(item[2])) if item else None


def graphic_points(item):
    kind = head(item)
    width = 0.0
    stroke = child(item, "stroke")
    if stroke and child(stroke, "width"):
        width = float(child(stroke, "width")[1])
    elif child(item, "width"):
        width = float(child(item, "width")[1])
    points = []
    if kind in ("fp_line", "fp_rect"):
        points += [xy(item, "start"), xy(item, "end")]
    elif kind == "fp_arc":
        points += [p for p in (xy(item, "start"), xy(item, "mid"), xy(item, "end")) if p]
    elif kind == "fp_circle":
        center, end = xy(item, "center"), xy(item, "end")
        radius = math.dist(center, end)
        points += [(center[0] - radius, center[1] - radius), (center[0] + radius, center[1] + radius)]
    elif kind == "fp_poly":
        pts = child(item, "pts")
        points += [(float(p[1]), float(p[2])) for p in pts[1:] if head(p) == "xy"] if pts else []
    points = [p for p in points if p]
    return [(x + dx, y + dy) for x, y in points for dx, dy in ((-width / 2, -width / 2), (width / 2, width / 2))]


def bbox(points):
    if not points:
        return None
    xs = [p[0] for p in points]
    ys = [p[1] for p in points]
    return (min(xs), min(ys), max(xs), max(ys))


def pad_points(pad, footprint_angle):
    at = child(pad, "at")
    px, py = float(at[1]), float(at[2])
    angle = float(at[3]) if len(at) > 3 and at[3] != "unlocked" else 0.0
    size = child(pad, "size")
    sx, sy = float(size[1]), float(size[2])
    turn = math.radians(-(angle - footprint_angle))
    sin, cos = math.sin(turn), math.cos(turn)
    hx = (abs(cos) * sx + abs(sin) * sy) / 2
    hy = (abs(sin) * sx + abs(cos) * sy) / 2
    return [(px - hx, py - hy), (px + hx, py + hy)]


def to_board(at, point):
    x, y, angle = at
    radians = math.radians(angle)
    cos, sin = math.cos(radians), math.sin(radians)
    lx, ly = point
    return (x + lx * cos + ly * sin, y - lx * sin + ly * cos)


def board_box(at, box):
    corners = [(box[0], box[1]), (box[2], box[1]), (box[0], box[3]), (box[2], box[3])]
    return bbox([to_board(at, corner) for corner in corners])


def to_local(at, point):
    x, y, angle = at
    radians = math.radians(angle)
    cos, sin = math.cos(radians), math.sin(radians)
    dx, dy = point[0] - x, point[1] - y
    return (dx * cos - dy * sin, dx * sin + dy * cos)


def holds(info, box, other):
    """Whether the centre of one of `other`'s pads lies inside `box` (the
    footprint's own frame)."""
    for pad in other["pad_boxes_board"]:
        center = to_local(info["at"], ((pad[0] + pad[2]) / 2, (pad[1] + pad[3]) / 2))
        if box[0] < center[0] < box[2] and box[1] < center[1] < box[3]:
            return True
    return False


def shrink(box, by):
    return (box[0] + by, box[1] + by, box[2] - by, box[3] - by)


def intersects(a, b):
    return a[0] < b[2] and b[0] < a[2] and a[1] < b[3] and b[1] < a[3]


def area(box):
    return max(0.0, box[2] - box[0]) * max(0.0, box[3] - box[1])


def footprint_info(footprint):
    at_node = child(footprint, "at")
    at = (float(at_node[1]), float(at_node[2]), float(at_node[3]) if len(at_node) > 3 else 0.0)
    reference = None
    for item in footprint:
        if head(item) == "fp_text" and len(item) > 2 and item[1] == "reference":
            reference = item[2]
        if head(item) == "property" and len(item) > 2 and item[1] == "Reference":
            reference = item[2]
    side = "B" if layer_of(footprint) == "B.Cu" else "F"
    graphics = [item for item in footprint if head(item) in ("fp_line", "fp_rect", "fp_arc", "fp_circle", "fp_poly")]
    courtyard = [g for g in graphics if (layer_of(g) or "").endswith(".CrtYd")]
    fab_own = [g for g in graphics if layer_of(g) == f"{side}.Fab"]
    fab_any = [g for g in graphics if (layer_of(g) or "").endswith(".Fab")]
    copper_pads = []
    holes = []
    for pad in children(footprint, "pad"):
        layers = layers_of(pad)
        kind = pad[2] if len(pad) > 2 else ""
        if any(layer.endswith(".Cu") for layer in layers):
            copper_pads.append(pad)
        if kind in ("thru_hole", "np_thru_hole"):
            holes.append(pad)
    pad_box = bbox([p for pad in copper_pads for p in pad_points(pad, at[2])])
    court_box = bbox([p for g in courtyard for p in graphic_points(g)])
    fab_box = bbox([p for g in fab_own for p in graphic_points(g)])
    return {
        "reference": reference,
        "library": footprint[1],
        "at": at,
        "side": side,
        "courtyard": bool(courtyard),
        "court_box": court_box,
        "fab_count": len(fab_own),
        "fab_any": len(fab_any),
        "fab_box": fab_box,
        "pads": len(copper_pads),
        "pad_box": pad_box,
        "smd_only": not holes,
        "pad_boxes_board": [board_box(at, bbox(pad_points(pad, at[2]))) for pad in copper_pads],
        "board_pad_box": board_box(at, pad_box) if pad_box else None,
        "board_body": board_box(at, court_box or pad_box) if (court_box or pad_box) else None,
        "board_fab": board_box(at, fab_box) if fab_box else None,
        "through": bool(holes),
        "pad_list": [
            [round(float(child(pad, "at")[1]), 4), round(float(child(pad, "at")[2]), 4)]
            + [round(v, 4) for v in bbox(pad_points(pad, at[2]))]
            + [pad[2] if len(pad) > 2 else ""]
            for pad in copper_pads
        ],
    }


def census(board_path, fixed):
    tree = parse(board_path.read_text(errors="replace"))
    footprints = [footprint_info(item) for item in tree if head(item) == "footprint"]
    rows = []
    for info in footprints:
        if info["courtyard"] or not info["pads"]:
            continue
        row = {key: info[key] for key in ("reference", "library", "side", "pads", "fab_count", "fab_any", "through")}
        row["fixed"] = info["reference"] in fixed
        row["pad_list"] = info["pad_list"]
        row["pad_box_size"] = [round(info["pad_box"][2] - info["pad_box"][0], 2), round(info["pad_box"][3] - info["pad_box"][1], 2)]
        if info["fab_box"]:
            fab, pads = info["fab_box"], info["pad_box"]
            row["fab_box_size"] = [round(fab[2] - fab[0], 2), round(fab[3] - fab[1], 2)]
            row["fab_over_pads"] = round(area(fab) / max(area(pads), 1e-9), 2)
            # Who the designer put inside the Fab box, on the same side
            # (pads of SMD parts, or through-hole parts on either side).
            intruders = []
            for other in footprints:
                if other is info:
                    continue
                same_side = other["side"] == info["side"] or (other["through"] and info["through"])
                if not same_side:
                    continue
                if holds(info, info["fab_box"], other):
                    intruders.append(other["reference"])
            row["inside_fab"] = intruders
        # Others inside the pad box (what blocking the whole pad box forbids).
        row["inside_pad_box"] = [
            other["reference"]
            for other in footprints
            if other is not info
            and (other["side"] == info["side"] or (other["through"] and info["through"]))
            and holds(info, info["pad_box"], other)
        ]
        rows.append(row)
    return len(footprints), rows


def main():
    parser = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    parser.add_argument("tasks", type=Path)
    parser.add_argument("--json", type=Path)
    arguments = parser.parse_args()
    tasks = json.loads(arguments.tasks.read_text())["tasks"]
    out = {}
    total = 0
    for task in tasks:
        directory = Path(task["directory"])
        board = directory / f"{task['board_id']}.kicad_pcb"
        if not board.exists():
            print(f"missing {board}", file=sys.stderr)
            continue
        constraints = json.loads((arguments.tasks.parent / task["constraints"]).read_text())
        fixed = set(constraints.get("fixed", []))
        count, rows = census(board, fixed)
        total += count
        out[task["name"]] = {"footprints": count, "courtless": rows}
    courtless = [row for board in out.values() for row in board["courtless"]]
    with_fab = [row for row in courtless if row["fab_count"]]
    movable = [row for row in courtless if not row["fixed"]]
    print(f"boards {len(out)}, footprints {total}, courtyard-less with pads {len(courtless)} "
          f"({len(movable)} movable) on {sum(1 for b in out.values() if b['courtless'])} boards")
    print(f"  with an own-side Fab outline: {len(with_fab)} ({sum(1 for r in with_fab if not r['fixed'])} movable); "
          f"Fab on any side: {sum(1 for r in courtless if r['fab_any'])}")
    print(f"  Fab box holds a designer's part: {sum(1 for r in with_fab if r['inside_fab'])}")
    print(f"  pad box holds a designer's part: {sum(1 for r in courtless if r['inside_pad_box'])}")
    if arguments.json:
        arguments.json.write_text(json.dumps(out, indent=1))


if __name__ == "__main__":
    main()
