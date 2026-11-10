#!/usr/bin/env python3
# Copyright (C) 2026 Toit contributors.
# Use of this source code is governed by an MIT-style license that can be
# found in the LICENSE file.
"""What the designer's placement does that ours does not: placement metrics
of two placements of one board, side by side (docs/placer.md, "Where the gap to the designer comes from").

    metrics.py run --sweep build/nightly/2026-11-03/sweep \\
        --designer build/nightly/2026-11-03/designer --first-open build/teach1-fr/first-open.json \\
        --out build/teach1-gap BOARD...

Per board the designer's placement (the designer tier's `placed` board) and
ours (the sweep's `layout/placed` board) are read with KiCad's pcbnew
(`extract`, cached as JSON) and measured (`measure`, plain Python on the
extracted parts; the tests use it without pcbnew). The parts both
placements hold at the same pose are the fixed ones; the metrics that judge
choices (orientation, alignment, spacing, connectors, the far side) count
the moved parts only. Writes `<out>/tables.md` (a table per board and the
summary over the boards) and `<out>/metrics.json`.

The metrics, per placement:

- `hpwl_mm`, `hpwl_<bucket>_mm`: half perimeter of each net's pins (nets of
  two pins or more), in all and by net size (2 pins, 3-8, 9 or more, the
  pour nets); `octilinear_mm`: each signal net's minimum spanning tree in
  the octilinear metric (max + (sqrt 2 - 1) min), summed;
- `crossings`: the placer's ratsnest crossings (each signal net's minimum
  spanning tree, Prim's from its first pin; pairs of lines of different
  nets that properly cross); signal nets are those of two pins or more not
  carried by a pour;
- `pin_row_crossings`: ratsnest lines of another net crossing the line
  between two adjacent SMD pads of one part (docs/quality.md's measure on
  the ratsnest instead of the tracks);
- `density_p90`, `density_max`, `density_cv`: body area over the board's
  area per cell of a 10 x 10 grid over the outline's box (cells at least
  half inside the outline; the fuller side; a frame part, one whose body
  covers a quarter of the board, by its pads);
- `connector_edge_mm`: moved connectors' median body-to-outline distance;
- `decoupling_mm`, `decoupling_within_3mm`: each decoupling capacitor's
  distance (pad centres) to the nearest supply pin of its rail on a part of
  eight pads or more: median, and the share within 3 mm;
- `orientation_agreement`: of the moved parts of one footprint (groups of
  two or more), the share at the group's most common angle (two-pad parts
  modulo 180 degrees);
- `row_alignment`: of the moved parts of one footprint (groups of three or
  more), the share whose centre shares an x or a y (0.05 mm) with another
  of the group;
- `far_side`: moved parts on the back;
- `gap_median_mm`, `gap_p10_mm`, `gap_under_0_25`, `gap_under_1`: each moved
  part's gap from its body to the nearest other body on its side (a
  through-hole part's pads on the other side; a frame part by its pads);
- `faced_pins_1mm`: pads of parts with eight pads or more facing (outward,
  within 1 mm) another part's body;
- `first_open`: the first route's open terminals (firstroute.py).
"""

import argparse
import json
import math
import statistics
import sys
from pathlib import Path

ROOT = Path(__file__).resolve().parents[2]
TASKS = ROOT / "benchmarks/agent-tasks/tasks/v7/tasks.json"
NM = 1e-6
CONNECTOR_PREFIXES = ("J", "P", "CN", "USB", "CON", "X")
CONNECTOR_WORDS = ("connector", "conn_", "header", "usb", "jack", "socket", "terminal", "pinheader", "pinsocket")
GROUND_WORDS = ("GND", "VSS", "0V", "AGND", "DGND", "PGND")


# --- extraction (pcbnew) ---------------------------------------------------------


def extract(path):
    """The board's parts, pads, outline, pour nets and copper layers as a
    plain dict (mm), read by KiCad's pcbnew."""
    import pcbnew  # noqa: PLC0415  (only the extraction needs KiCad)

    board = pcbnew.LoadBoard(str(path))
    outlines = pcbnew.SHAPE_POLY_SET()
    board.GetBoardPolygonOutlines(outlines, True)
    polygons = []
    for index in range(outlines.OutlineCount()):
        outline = outlines.Outline(index)
        polygons.append([[outline.CPoint(i).x * NM, outline.CPoint(i).y * NM] for i in range(outline.PointCount())])
        for hole in range(outlines.HoleCount(index)):
            ring = outlines.Hole(index, hole)
            polygons.append([[ring.CPoint(i).x * NM, ring.CPoint(i).y * NM] for i in range(ring.PointCount())])
    pours = sorted({zone.GetNetname() for zone in board.Zones()
                    if not zone.GetIsRuleArea() and zone.GetNetname() and zone.IsOnCopperLayer()})
    footprints = []
    for footprint in board.GetFootprints():
        footprint.BuildCourtyardCaches()
        back = footprint.IsFlipped()
        courtyard = footprint.GetCourtyard(pcbnew.B_CrtYd if back else pcbnew.F_CrtYd)
        boxes = []
        if courtyard.OutlineCount():
            box = courtyard.BBox()
            boxes.append([box.GetLeft() * NM, box.GetTop() * NM, box.GetRight() * NM, box.GetBottom() * NM])
        pads = []
        through = False
        for pad in footprint.Pads():
            box = pad.GetBoundingBox()
            smd = pad.GetAttribute() == pcbnew.PAD_ATTRIB_SMD
            through = through or pad.GetAttribute() == pcbnew.PAD_ATTRIB_PTH
            position = pad.GetPosition()
            pads.append({"number": pad.GetNumber(), "net": pad.GetNetname() or None,
                         "x": position.x * NM, "y": position.y * NM, "smd": smd,
                         "box": [box.GetLeft() * NM, box.GetTop() * NM, box.GetRight() * NM, box.GetBottom() * NM]})
            boxes.append(pads[-1]["box"])
        if not boxes:
            continue
        position = footprint.GetPosition()
        footprints.append({
            "reference": footprint.GetReference(), "footprint": footprint.GetFPIDAsString(),
            "value": footprint.GetValue(), "x": position.x * NM, "y": position.y * NM,
            "angle": footprint.GetOrientationDegrees() % 360.0, "back": back, "through_hole": through,
            "courtyard": bool(courtyard.OutlineCount()),
            "body": [min(b[0] for b in boxes), min(b[1] for b in boxes), max(b[2] for b in boxes),
                     max(b[3] for b in boxes)],
            "pads": pads})
    return {"copper_layers": board.GetCopperLayerCount(), "outline": polygons, "pour_nets": pours,
            "footprints": footprints}


def extracted(path, cache):
    cache = Path(cache)
    if cache.exists() and cache.stat().st_mtime >= Path(path).stat().st_mtime:
        return json.loads(cache.read_text())
    data = extract(path)
    cache.parent.mkdir(parents=True, exist_ok=True)
    cache.write_text(json.dumps(data))
    return data


# --- geometry ----------------------------------------------------------------------


def orientation(a, b, c):
    return (b[0] - a[0]) * (c[1] - a[1]) - (b[1] - a[1]) * (c[0] - a[0])


def lines_cross(p, q, r, s):
    """Whether segments pq and rs properly cross (touching and collinear
    segments do not), as crossings::lines_cross."""
    d1, d2 = orientation(p, q, r), orientation(p, q, s)
    d3, d4 = orientation(r, s, p), orientation(r, s, q)
    return ((d1 > 0 > d2) or (d1 < 0 < d2)) and ((d3 > 0 > d4) or (d3 < 0 < d4))


def spanning_tree(points, metric=math.dist):
    """Prim's minimum spanning tree from the first point (a tie to the
    earlier point); the tree's lines as index pairs."""
    n = len(points)
    if n < 2:
        return []
    inside = [False] * n
    best = [math.inf] * n
    parent = [-1] * n
    best[0] = 0.0
    lines = []
    for _ in range(n):
        u = min((i for i in range(n) if not inside[i]), key=lambda i: (best[i], i))
        inside[u] = True
        if parent[u] >= 0:
            lines.append((parent[u], u))
        for v in range(n):
            if not inside[v]:
                d = metric(points[u], points[v])
                if d < best[v]:
                    best[v], parent[v] = d, u
    return lines


def octilinear(a, b):
    dx, dy = abs(a[0] - b[0]), abs(a[1] - b[1])
    return max(dx, dy) + (math.sqrt(2.0) - 1.0) * min(dx, dy)


class SegmentGrid:
    """Segments in a uniform grid by their boxes, for crossing queries."""

    def __init__(self, segments, cell):
        self.cell = max(cell, 1e-3)
        self.cells = {}
        self.segments = segments
        for index, (p, q, _) in enumerate(segments):
            for key in self.keys(p, q):
                self.cells.setdefault(key, []).append(index)

    def keys(self, p, q):
        x0, x1 = sorted((p[0], q[0]))
        y0, y1 = sorted((p[1], q[1]))
        for i in range(math.floor(x0 / self.cell), math.floor(x1 / self.cell) + 1):
            for j in range(math.floor(y0 / self.cell), math.floor(y1 / self.cell) + 1):
                yield i, j

    def candidates(self, p, q):
        seen = set()
        for key in self.keys(p, q):
            for index in self.cells.get(key, ()):
                if index not in seen:
                    seen.add(index)
                    yield index


def box_gap(a, b):
    dx = max(a[0] - b[2], b[0] - a[2], 0.0)
    dy = max(a[1] - b[3], b[1] - a[3], 0.0)
    return math.hypot(dx, dy)


def point_segment(p, a, b):
    ax, ay = b[0] - a[0], b[1] - a[1]
    length = ax * ax + ay * ay
    t = 0.0 if length == 0 else max(0.0, min(1.0, ((p[0] - a[0]) * ax + (p[1] - a[1]) * ay) / length))
    return math.dist(p, (a[0] + t * ax, a[1] + t * ay))


def box_outline_distance(box, polygons):
    """The least distance from a box to the outline's edges (0 where an edge
    meets the box)."""
    corners = [(box[0], box[1]), (box[2], box[1]), (box[2], box[3]), (box[0], box[3])]
    sides = list(zip(corners, corners[1:] + corners[:1]))
    best = math.inf
    for polygon in polygons:
        for a, b in zip(polygon, polygon[1:] + polygon[:1]):
            if box[0] <= a[0] <= box[2] and box[1] <= a[1] <= box[3]:
                return 0.0
            if any(lines_cross(a, b, c, d) for c, d in sides):
                return 0.0
            best = min(best, point_segment(a, *sides[0]), point_segment(a, *sides[1]),
                       point_segment(a, *sides[2]), point_segment(a, *sides[3]),
                       *(point_segment(c, a, b) for c in corners))
    return best


def inside_polygon(point, polygon):
    x, y = point
    inside = False
    for (x1, y1), (x2, y2) in zip(polygon, polygon[1:] + polygon[:1]):
        if (y1 > y) != (y2 > y) and x < x1 + (y - y1) * (x2 - x1) / (y2 - y1):
            inside = not inside
    return inside


def inside_outline(point, polygons):
    """Even-odd over every ring (outer outlines and holes)."""
    return sum(inside_polygon(point, polygon) for polygon in polygons) % 2 == 1


def quantile(values, q):
    if not values:
        return None
    values = sorted(values)
    position = (len(values) - 1) * q
    low = math.floor(position)
    high = min(low + 1, len(values) - 1)
    return values[low] + (values[high] - values[low]) * (position - low)


# --- the metrics -----------------------------------------------------------------------


def is_connector(footprint):
    reference = footprint["reference"].upper()
    prefix = reference.rstrip("0123456789")
    name = footprint["footprint"].lower()
    return prefix in CONNECTOR_PREFIXES or any(word in name for word in CONNECTOR_WORDS)


def is_ground(net):
    return net is not None and any(word in net.upper() for word in GROUND_WORDS)


def net_pins(board):
    """{net: [(x, y)]} over every pad with a net."""
    nets = {}
    for footprint in board["footprints"]:
        for pad in footprint["pads"]:
            if pad["net"]:
                nets.setdefault(pad["net"], []).append((pad["x"], pad["y"]))
    return nets


def ratsnest(board):
    """The signal nets' minimum spanning trees: [(p, q, net)]."""
    pours = set(board["pour_nets"])
    lines = []
    for net, pins in net_pins(board).items():
        if net in pours or len(pins) < 2:
            continue
        for a, b in spanning_tree(pins):
            lines.append((pins[a], pins[b], net))
    return lines


def crossing_count(lines):
    if not lines:
        return 0
    mean = statistics.mean(max(abs(p[0] - q[0]), abs(p[1] - q[1])) for p, q, _ in lines)
    grid = SegmentGrid(lines, max(mean, 0.5))
    count = 0
    for index, (p, q, net) in enumerate(lines):
        for other in grid.candidates(p, q):
            if other <= index:
                continue
            r, s, other_net = lines[other]
            if other_net != net and lines_cross(p, q, r, s):
                count += 1
    return count


def pin_row_pairs(footprint):
    """Adjacent SMD pad pairs of a part of three SMD pads or more (at most
    1.25 times the closer pad's nearest-neighbour distance apart)."""
    pads = [pad for pad in footprint["pads"] if pad["smd"]]
    if len(pads) < 3:
        return []
    points = [(pad["x"], pad["y"]) for pad in pads]
    nearest = []
    for i, p in enumerate(points):
        nearest.append(min((math.dist(p, q) for j, q in enumerate(points) if j != i and math.dist(p, q) > 1e-6),
                           default=math.inf))
    pairs = []
    for i in range(len(points)):
        for j in range(i + 1, len(points)):
            d = math.dist(points[i], points[j])
            if 1e-6 < d <= 1.25 * min(nearest[i], nearest[j]):
                pairs.append((pads[i], pads[j]))
    return pairs


def pin_row_crossings(board, lines):
    if not lines:
        return 0
    mean = statistics.mean(max(abs(p[0] - q[0]), abs(p[1] - q[1])) for p, q, _ in lines)
    grid = SegmentGrid(lines, max(mean, 0.5))
    count = 0
    for footprint in board["footprints"]:
        for a, b in pin_row_pairs(footprint):
            p, q = (a["x"], a["y"]), (b["x"], b["y"])
            nets = set()
            for index in grid.candidates(p, q):
                r, s, net = lines[index]
                if net in (a["net"], b["net"]) or net in nets:
                    continue
                if lines_cross(p, q, r, s):
                    nets.add(net)
            count += len(nets)
    return count


def bucket(net, count, pours):
    if net in pours:
        return "pour"
    if count == 2:
        return "2"
    return "3_8" if count <= 8 else "9plus"


def wirelength(board):
    pours = set(board["pour_nets"])
    result = {"hpwl_mm": 0.0, "octilinear_mm": 0.0}
    for name in ("2", "3_8", "9plus", "pour"):
        result[f"hpwl_{name}_mm"] = 0.0
    for net, pins in net_pins(board).items():
        if len(pins) < 2:
            continue
        xs, ys = [p[0] for p in pins], [p[1] for p in pins]
        hpwl = (max(xs) - min(xs)) + (max(ys) - min(ys))
        result["hpwl_mm"] += hpwl
        result[f"hpwl_{bucket(net, len(pins), pours)}_mm"] += hpwl
        if net not in pours:
            result["octilinear_mm"] += sum(octilinear(pins[a], pins[b]) for a, b in spanning_tree(pins, octilinear))
    return result


def outline_box(board):
    points = [p for polygon in board["outline"] for p in polygon]
    if not points:
        bodies = [f["body"] for f in board["footprints"]]
        return [min(b[0] for b in bodies), min(b[1] for b in bodies), max(b[2] for b in bodies),
                max(b[3] for b in bodies)]
    return [min(p[0] for p in points), min(p[1] for p in points), max(p[0] for p in points),
            max(p[1] for p in points)]


def density(board, cells=10, samples=4):
    box = outline_box(board)
    width, height = (box[2] - box[0]) / cells, (box[3] - box[1]) / cells
    values = []
    for i in range(cells):
        for j in range(cells):
            cell = [box[0] + i * width, box[1] + j * height, box[0] + (i + 1) * width, box[1] + (j + 1) * height]
            if board["outline"]:
                inside = sum(inside_outline((cell[0] + (a + 0.5) * width / samples, cell[1] + (b + 0.5) * height
                                             / samples), board["outline"])
                             for a in range(samples) for b in range(samples)) / samples ** 2
            else:
                inside = 1.0
            if inside < 0.5:
                continue
            sides = []
            for back in (False, True):
                area = 0.0
                for footprint in board["footprints"]:
                    if footprint["back"] != back:
                        continue
                    for b in shapes(footprint, back, board):
                        dx = min(b[2], cell[2]) - max(b[0], cell[0])
                        dy = min(b[3], cell[3]) - max(b[1], cell[1])
                        if dx > 0 and dy > 0:
                            area += dx * dy
                sides.append(area)
            values.append(max(sides) / (width * height * inside))
    if not values:
        return {"density_p90": None, "density_max": None, "density_cv": None}
    mean = statistics.mean(values)
    return {"density_p90": quantile(values, 0.9), "density_max": max(values),
            "density_cv": statistics.pstdev(values) / mean if mean else None}


def decoupling(board):
    pins_by_net = {}
    for footprint in board["footprints"]:
        if len(footprint["pads"]) >= 8 and not is_connector(footprint):
            for pad in footprint["pads"]:
                if pad["net"]:
                    pins_by_net.setdefault(pad["net"], []).append((pad["x"], pad["y"]))
    distances = []
    for footprint in board["footprints"]:
        if not footprint["reference"].upper().startswith("C") or len(footprint["pads"]) != 2:
            continue
        nets = [pad["net"] for pad in footprint["pads"]]
        if not any(is_ground(net) for net in nets):
            continue
        for pad in footprint["pads"]:
            if pad["net"] and not is_ground(pad["net"]) and pad["net"] in pins_by_net:
                distances.append(min(math.dist((pad["x"], pad["y"]), p) for p in pins_by_net[pad["net"]]))
    if not distances:
        return {"decoupling_mm": None, "decoupling_within_3mm": None}
    return {"decoupling_mm": statistics.median(distances),
            "decoupling_within_3mm": sum(d <= 3.0 for d in distances) / len(distances)}


def angle_key(footprint):
    modulo = 180.0 if len(footprint["pads"]) == 2 else 360.0
    return round(footprint["angle"] % modulo) % round(modulo)


def groups(footprints, least):
    by = {}
    for footprint in footprints:
        by.setdefault((footprint["footprint"], footprint["back"]), []).append(footprint)
    return [group for group in by.values() if len(group) >= least]


def orientation_agreement(moved):
    total = agreeing = 0
    for group in groups(moved, 2):
        keys = [angle_key(f) for f in group]
        agreeing += max(keys.count(k) for k in set(keys))
        total += len(group)
    return agreeing / total if total else None


def row_alignment(moved, tolerance=0.05):
    total = aligned = 0
    for group in groups(moved, 3):
        for footprint in group:
            total += 1
            if any(other is not footprint and (abs(other["x"] - footprint["x"]) <= tolerance
                                               or abs(other["y"] - footprint["y"]) <= tolerance)
                   for other in group):
                aligned += 1
    return aligned / total if total else None


def box_area(box):
    return max(box[2] - box[0], 0.0) * max(box[3] - box[1], 0.0)


def is_frame(footprint, board):
    """A part whose body covers a quarter of the outline's box or more (the
    OpenESC boards' stack connector, pads at the board's corners): it blocks
    with its pads only."""
    return box_area(footprint["body"]) >= 0.25 * box_area(outline_box(board))


def shapes(footprint, back, board):
    """The boxes a part blocks the side `back` with, as the placer's bodies
    do: its body on its own side, its pads on the other where it has
    through-hole pads; a frame with its pads alone."""
    pads = [pad["box"] for pad in footprint["pads"]]
    if is_frame(footprint, board):
        return pads if footprint["through_hole"] or footprint["back"] == back else []
    if footprint["back"] == back:
        return [footprint["body"]]
    return pads if footprint["through_hole"] else []


def shapes_gap(a, b):
    return min((box_gap(x, y) for x in a for y in b), default=math.inf)


def gaps(board, moved_references):
    footprints = board["footprints"]
    result = []
    for footprint in footprints:
        if footprint["reference"] not in moved_references:
            continue
        own = shapes(footprint, footprint["back"], board)
        best = math.inf
        for other in footprints:
            if other is footprint:
                continue
            if box_gap(footprint["body"], other["body"]) >= best:
                continue
            best = min(best, shapes_gap(own, shapes(other, footprint["back"], board)))
        if best < math.inf:
            result.append(best)
    if not result:
        return {"gap_median_mm": None, "gap_p10_mm": None, "gap_under_0_25": None, "gap_under_1": None}
    return {"gap_median_mm": statistics.median(result), "gap_p10_mm": quantile(result, 0.1),
            "gap_under_0_25": sum(g < 0.25 for g in result) / len(result),
            "gap_under_1": sum(g < 1.0 for g in result) / len(result)}


def faced_pins(board, reach=1.0):
    """Pads of parts with eight pads or more whose way out (the body side
    nearest the pad, outward, as wide as the pad, `reach` deep) meets
    another colliding part's body."""
    footprints = board["footprints"]
    count = 0
    for footprint in footprints:
        if len(footprint["pads"]) < 8:
            continue
        if is_frame(footprint, board):
            continue
        body = footprint["body"]
        others = [box for o in footprints if o is not footprint and box_gap(o["body"], body) < reach
                  for box in shapes(o, footprint["back"], board)]
        if not others:
            continue
        for pad in footprint["pads"]:
            x0, y0, x1, y1 = pad["box"]
            sides = [(pad["x"] - body[0], "w"), (body[2] - pad["x"], "e"), (pad["y"] - body[1], "n"),
                     (body[3] - pad["y"], "s")]
            side = min(sides)[1]
            if side == "w":
                ray = [body[0] - reach, y0, body[0], y1]
            elif side == "e":
                ray = [body[2], y0, body[2] + reach, y1]
            elif side == "n":
                ray = [x0, body[1] - reach, x1, body[1]]
            else:
                ray = [x0, body[3], x1, body[3] + reach]
            if any(min(ray[2], o[2]) - max(ray[0], o[0]) > 0 and min(ray[3], o[3]) - max(ray[1], o[1]) > 0
                   for o in others):
                count += 1
    return count


def moved_references(designer, ours, tolerance=1e-3):
    """References whose pose differs between the two placements."""
    other = {f["reference"]: f for f in ours["footprints"]}
    moved = set()
    for footprint in designer["footprints"]:
        twin = other.get(footprint["reference"])
        if twin is None:
            continue
        if (abs(twin["x"] - footprint["x"]) > tolerance or abs(twin["y"] - footprint["y"]) > tolerance
                or abs((twin["angle"] - footprint["angle"] + 180) % 360 - 180) > 0.01
                or twin["back"] != footprint["back"]):
            moved.add(footprint["reference"])
    return moved


def measure(board, moved):
    """Every metric of one placement; `moved` the references the two
    placements put differently."""
    lines = ratsnest(board)
    result = wirelength(board)
    result["crossings"] = crossing_count(lines)
    result["pin_row_crossings"] = pin_row_crossings(board, lines)
    result.update(density(board))
    moved_parts = [f for f in board["footprints"] if f["reference"] in moved]
    connectors = [box_outline_distance(f["body"], board["outline"]) for f in moved_parts if is_connector(f)]
    result["connector_edge_mm"] = statistics.median(connectors) if connectors and board["outline"] else None
    result.update(decoupling(board))
    result["orientation_agreement"] = orientation_agreement(moved_parts)
    result["row_alignment"] = row_alignment(moved_parts)
    result["far_side"] = sum(f["back"] for f in moved_parts)
    result.update(gaps(board, moved))
    result["faced_pins_1mm"] = faced_pins(board)
    return result


# --- the comparison -------------------------------------------------------------------------

METRICS = ["first_open", "hpwl_mm", "hpwl_2_mm", "hpwl_3_8_mm", "hpwl_9plus_mm", "hpwl_pour_mm", "octilinear_mm",
           "crossings", "pin_row_crossings", "density_p90", "density_max", "density_cv", "connector_edge_mm",
           "decoupling_mm", "decoupling_within_3mm", "orientation_agreement", "row_alignment", "far_side",
           "gap_median_mm", "gap_p10_mm", "gap_under_0_25", "gap_under_1", "faced_pins_1mm"]


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
    pairs = [(x, y) for x, y in zip(xs, ys) if x is not None and y is not None]
    if len(pairs) < 3:
        return None
    rx, ry = ranks([p[0] for p in pairs]), ranks([p[1] for p in pairs])
    mx, my = statistics.mean(rx), statistics.mean(ry)
    sx = math.sqrt(sum((r - mx) ** 2 for r in rx))
    sy = math.sqrt(sum((r - my) ** 2 for r in ry))
    if sx == 0 or sy == 0:
        return None
    return sum((a - mx) * (b - my) for a, b in zip(rx, ry)) / (sx * sy)


def relative(designer, ours):
    """ours / designer - 1, or None where it is not defined."""
    if designer is None or ours is None:
        return None
    if designer == 0:
        return 0.0 if ours == 0 else None
    return ours / designer - 1.0


def summarise(rows):
    """Per metric over the boards: how often ours is above the designer's,
    the median relative difference, the Spearman over every placement
    (metric normalised by the board's mean of its two placements, against
    the first route's open terminals per connection) and over the boards'
    paired differences."""
    summary = {}
    for metric in METRICS:
        above = below = equal = 0
        rel = []
        xs, ys, dxs, dys = [], [], [], []
        for row in rows:
            d, o = row["designer"].get(metric), row["ours"].get(metric)
            if d is None or o is None:
                continue
            above += o > d
            below += o < d
            equal += o == d
            r = relative(d, o)
            if r is not None:
                rel.append(r)
            mean = (d + o) / 2.0
            fd, fo = row["designer"].get("first_open"), row["ours"].get("first_open")
            if fd is None or fo is None:
                continue
            connections = row["connections"] or 1
            for value, first in ((d, fd), (o, fo)):
                xs.append(value / mean if mean else 1.0)
                ys.append(first / connections)
            dxs.append((o - d) / mean if mean else 0.0)
            dys.append((fo - fd) / connections)
        summary[metric] = {"ours_above": above, "ours_below": below, "equal": equal,
                           "median_relative": statistics.median(rel) if rel else None,
                           "spearman_placements": spearman(xs, ys) if metric != "first_open" else None,
                           "spearman_differences": spearman(dxs, dys) if metric != "first_open" else None,
                           "placements": len(xs)}
    return summary


def fmt(value):
    if value is None:
        return "-"
    if isinstance(value, bool):
        return str(value)
    if isinstance(value, int):
        return str(value)
    if abs(value) >= 100:
        return f"{value:.0f}"
    if abs(value) >= 10:
        return f"{value:.1f}"
    return f"{value:.2f}"


def tables(rows, summary):
    out = []
    for row in rows:
        out.append(f"### {row['board']}\n")
        out.append(f"{row['connections']} connections, {row['moved']} parts moved of {row['parts']}; "
                   f"layout open {row.get('layout_open')}, designer-route open {row.get('designer_open')}.\n")
        out.append("| metric | designer | ours | ours / designer |\n| --- | ---: | ---: | ---: |")
        for metric in METRICS:
            d, o = row["designer"].get(metric), row["ours"].get(metric)
            r = relative(d, o)
            out.append(f"| {metric} | {fmt(d)} | {fmt(o)} | {'-' if r is None else f'{1 + r:.2f}'} |")
        out.append("")
    out.append(f"### Summary over the {len(rows)} boards\n")
    out.append("| metric | ours above / below / equal | median ours / designer | Spearman, placements | "
               "Spearman, paired differences |\n| --- | --- | ---: | ---: | ---: |")
    for metric in METRICS:
        s = summary[metric]
        rel = s["median_relative"]
        out.append(f"| {metric} | {s['ours_above']} / {s['ours_below']} / {s['equal']} | "
                   f"{'-' if rel is None else f'{1 + rel:.2f}'} | {fmt(s['spearman_placements'])} | "
                   f"{fmt(s['spearman_differences'])} |")
    return "\n".join(out) + "\n"


def board_ids():
    return {task["name"]: task["board_id"] for task in json.loads(TASKS.read_text())["tasks"]}


def connections(board):
    pins = net_pins(board)
    return sum(len(p) - 1 for p in pins.values() if len(p) >= 2)


def gap_rows(path):
    """{board: (layout open, designer open)} from a gap.txt."""
    result = {}
    if path and Path(path).exists():
        for line in Path(path).read_text().splitlines():
            words = line.split()
            if len(words) == 6 and words[1].isdigit():
                result[words[0]] = (int(words[2]), int(words[3]))
    return result


def run(arguments):
    ids = board_ids()
    out = Path(arguments.out)
    first = json.loads(Path(arguments.first_open).read_text()) if arguments.first_open else {}
    gap = gap_rows(arguments.gap)
    rows = []
    for name in arguments.boards:
        board_id = ids[name]
        designer = extracted(Path(arguments.designer) / name / "placed" / f"{board_id}.kicad_pcb",
                             out / "extract" / "designer" / f"{name}.json")
        ours = extracted(Path(arguments.sweep) / name / "layout/placed" / f"{board_id}.kicad_pcb",
                         out / "extract" / "ours" / f"{name}.json")
        moved = moved_references(designer, ours)
        row = {"board": name, "parts": len(designer["footprints"]), "moved": len(moved),
               "connections": connections(designer), "designer": measure(designer, moved),
               "ours": measure(ours, moved)}
        row["designer"]["first_open"] = first.get(f"designer|{name}")
        row["ours"]["first_open"] = first.get(f"ours|{name}")
        if name in gap:
            row["layout_open"], row["designer_open"] = gap[name]
        rows.append(row)
        print(f"{name}: measured ({len(moved)} moved)", file=sys.stderr, flush=True)
    summary = summarise(rows)
    out.mkdir(parents=True, exist_ok=True)
    (out / "metrics.json").write_text(json.dumps({"rows": rows, "summary": summary}, indent=1))
    (out / "tables.md").write_text(tables(rows, summary))
    print(tables(rows[-0:] and [], summary))


def main():
    parser = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    sub = parser.add_subparsers(dest="command", required=True)
    r = sub.add_parser("run")
    r.add_argument("--sweep", required=True)
    r.add_argument("--designer", required=True)
    r.add_argument("--first-open")
    r.add_argument("--gap")
    r.add_argument("--out", required=True)
    r.add_argument("boards", nargs="+")
    arguments = parser.parse_args()
    run(arguments)


if __name__ == "__main__":
    main()
