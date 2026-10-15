# Copyright (C) 2026 Toit contributors.
"""KiCad's connection_width findings, found and named.

    python3 -I necks.py [--jobs N] [--json out.json] [--quiet] <board.kicad_pcb | directory>...

A directory holds one `.kicad_pcb` (a result directory with `drc.json`, or
a harvested source). Boards whose project sets no minimum connection width
(`min_connection` 0, KiCad's default) have no such check and are skipped
without loading.

KiCad's side (pcbnew/drc/drc_test_provider_connection_width.cpp, 10.0.7):
the zones are filled as `kicad-cli pcb drc --refill-zones` fills them; per
net and copper layer every zone fill, track, flashed via and flashed pad is
turned into polygons and united (`Fracture`, which simplifies first and
joins holes to their outline by slits); then POLYGON_TEST walks each
outline: at every concave vertex it looks for another vertex of the same
outline (not a neighbour by index) closer than the minimum width less
twice (DRC epsilon + max error + extra clearance), seen from inside on both
sides, such that the outline bulges wider than that limit in x and in y on
both sides of the cut. Each pair is one finding, its width the distance of
the two vertices, its place their midpoint. The check is reproduced here
vertex for vertex (VERTEX_SET's list, z-order and fracture skipping) so
that each finding has a place; with `drc.json` beside the board the count
per layer and the widths are compared with KiCad's report.

Each neck is named by its sides (what cut the copper at each of the two
vertices: another net's pad, track or via, the board edge, the net's own
thermal relief, a hatch hole, the zone's outline) and by what of the net
lies at its middle (a spoke or the thermal ring of a pad of the net, the
pad itself, a track or via of the net, plain fill). A "gap" is a pair
across empty space: KiCad's test is local (the vertices see each other
from inside), so a slot narrower than the limit between two parts of one
outline counts as a neck as well.

Usage for the census of 2026-10-15: the designer boards of the tasks with
`min_connection` set (benchmarks/real/external/github/<name>) and our
results (build/github-layout-v16/<name>/layout/result, the tiers' result
directories); `--json` keeps every neck with its vertices.
"""

import argparse
import concurrent.futures
import glob
import json
import math
import multiprocessing
import os
import re
import shutil
import subprocess
import sys
import tempfile

import pcbnew

NM = 1.0e6
ARC_HIGH_DEF = 5000  # KiCad's ARC_HIGH_DEF, 0.005 mm


# --- KiCad's POLYGON_TEST (VERTEX_SET) -------------------------------------

class Vertex:
    __slots__ = ("i", "x", "y", "prev", "next", "prevZ", "nextZ", "z")

    def __init__(self, i, x, y):
        self.i, self.x, self.y = i, x, y
        self.prev = self.next = self.prevZ = self.nextZ = None
        self.z = 0


def same_point(a, b):
    return a is not None and b is not None and a.x == b.x and a.y == b.y


def area(p, q, r):
    return (q.y - p.y) * (r.x - q.x) - (q.x - p.x) * (r.y - q.y)


class PolygonTest:
    def __init__(self, limit):
        self.limit = limit

    def z_order(self, x, y):
        bx, by, bw, bh = self.bbox
        lx = min(max((x - bx) / bw, 0.0), 1.0) if bw else 0.0
        ly = min(max((y - by) / bh, 0.0), 1.0) if bh else 0.0
        xi = int(lx * 32767.0)
        yi = int(ly * 32767.0)
        xi = (xi | (xi << 8)) & 0x00FF00FF
        xi = (xi | (xi << 4)) & 0x0F0F0F0F
        xi = (xi | (xi << 2)) & 0x33333333
        xi = (xi | (xi << 1)) & 0x55555555
        yi = (yi | (yi << 8)) & 0x00FF00FF
        yi = (yi | (yi << 4)) & 0x0F0F0F0F
        yi = (yi | (yi << 2)) & 0x33333333
        yi = (yi | (yi << 1)) & 0x55555555
        return xi | (yi << 1)

    def next_outline(self, a):
        nz, pz = a.nextZ, a.prevZ
        if same_point(a, nz) and same_point(a.next, nz.prev) and a.y == a.next.y:
            return nz.next
        if same_point(a, pz) and same_point(a.next, pz.prev) and a.y == a.next.y:
            return pz.next
        return a.next

    def prev_outline(self, a):
        nz, pz = a.nextZ, a.prevZ
        if same_point(a, nz) and a.y == a.prev.y:
            return nz.prev
        if same_point(a, pz) and a.y == a.prev.y:
            return pz.prev
        return a.prev

    def locally_inside(self, a, b):
        an = self.next_outline(a)
        ap = self.prev_outline(a)
        if area(ap, a, an) < 0:
            return area(a, b, an) >= 0 and area(a, ap, b) >= 0
        return area(a, b, ap) < 0 or area(a, an, b) < 0

    def substantial(self, a, b):
        limit = self.limit
        total = self.total
        for forward, step in ((True, self.next_outline), (False, self.prev_outline)):
            x_change = y_change = False
            checked = 0
            p0 = a
            p = step(p0)
            while (not same_point(p, b) and not same_point(p, a) and checked < total
                   and not (x_change and y_change)):
                if abs(p.x - p0.x) > limit:
                    x_change = True
                if abs(p.y - p0.y) > limit:
                    y_change = True
                p = step(p)
                checked += 1
            if checked >= total:
                return False
            if forward:
                if not same_point(p, a) and (not x_change or not y_change):
                    return False
            else:
                return same_point(p, a) or (x_change and y_change)
        return False

    def kink(self, a):
        if self.locally_inside(a.prev, a.next):
            return None
        limit = self.limit
        max_z = self.z_order(a.x + limit, a.y + limit)
        min_z = self.z_order(a.x - limit, a.y - limit)
        limit2 = float(limit) * float(limit)
        best, best_d2 = None, float("inf")
        for forward in (True, False):
            p = a.nextZ if forward else a.prevZ
            while p is not None and (p.z <= max_z if forward else p.z >= min_z):
                dx, dy = p.x - a.x, p.y - a.y
                d2 = dx * dx + dy * dy
                if (abs(p.i - a.i) > 1 and d2 < limit2 and d2 < best_d2 and d2 > 0
                        and self.locally_inside(p, a) and self.substantial(p, a) and self.substantial(a, p)):
                    best, best_d2 = p, d2
                p = p.nextZ if forward else p.prevZ
        return best

    def find_pairs(self, points):
        """`points` the outline's (x, y) in nanometres: the set of index pairs."""
        count = len(points)
        if count < 3:
            return set()
        xs = [p[0] for p in points]
        ys = [p[1] for p in points]
        self.bbox = (min(xs), min(ys), max(xs) - min(xs), max(ys) - min(ys))
        total = 0.0
        for i in range(count):
            x1, y1 = points[i]
            x2, y2 = points[(i + 1) % count]
            total += (float(x2) - float(x1)) * (float(y2) + float(y1))
        order = range(count - 1, -1, -1) if total > 0.0 else range(count)
        vertices = []
        tail = None
        last = None
        for i in order:
            x, y = points[i]
            if last is None or (x - last[0]) ** 2 + (y - last[1]) ** 2 > 0:
                v = Vertex(i, float(x), float(y))
                if tail is None:
                    v.prev = v.next = v
                else:
                    v.next = tail.next
                    v.prev = tail
                    tail.next.prev = v
                    tail.next = v
                vertices.append(v)
                tail = v
                last = (x, y)
        self.total = len(vertices)
        front = vertices[0]
        if tail is not front and tail.x == tail.next.x and tail.y == tail.next.y:
            # KiCad removes the head here; a closed chain never repeats it
            pass
        # updateList: drop consecutive duplicates, z-order, sort
        p = front.next
        while p is not front:
            if p.x == p.next.x and p.y == p.next.y:
                p = p.prev
                q = p.next
                q.next.prev = q.prev
                q.prev.next = q.next
                if p is p.next:
                    break
            p.z = self.z_order(p.x, p.y)
            p = p.next
        front.z = self.z_order(front.x, front.y)
        queue = [front]
        p = front.next
        while p is not None and p is not front:
            queue.append(p)
            p = p.next
        queue.sort(key=lambda v: (v.z, v.x, v.y, v.i))
        previous = None
        for v in queue:
            v.prevZ = previous
            if previous is not None:
                previous.nextZ = v
            previous = v
        previous.nextZ = None
        hits = set()
        all_hits = set()
        p = front.next
        while p is not front:
            if not all_hits or id(p) not in all_hits:
                match = self.kink(p)
                if match is not None and id(match) not in all_hits and (p.i, match.i) not in hits:
                    hits.add((p.i, match.i))
                    for v in (p, match, p.next, p.prev, match.next, match.prev):
                        all_hits.add(id(v))
            p = p.next
        return hits


# --- the board ---------------------------------------------------------------

def project_min_connection(board_path):
    project = board_path[:-len(".kicad_pcb")] + ".kicad_pro"
    try:
        settings = json.load(open(project))
        return float(settings["board"]["design_settings"]["rules"].get("min_connection", 0.0))
    except (OSError, KeyError, ValueError):
        return 0.0


def custom_rule_widths(board_path):
    rules = board_path[:-len(".kicad_pcb")] + ".kicad_dru"
    if not os.path.exists(rules):
        return []
    text = open(rules).read()
    return re.findall(r"\(constraint\s+connection_width[^)]*\)", text)


def items_on_layer(board, zones, layer):
    """(item, kind) of every copper item on the layer, as the check collects them."""
    result = []
    for zone in zones:
        if zone.IsOnLayer(layer):
            result.append((zone, "zone"))
    for track in board.GetTracks():
        if track.Type() == pcbnew.PCB_VIA_T:
            if track.FlashLayer(layer):
                result.append((track, "via"))
        elif track.IsOnLayer(layer):
            result.append((track, "track"))
    for footprint in board.GetFootprints():
        for pad in footprint.Pads():
            if pad.FlashLayer(layer):
                result.append((pad, "pad"))
    return result


def outline_distance(poly, x, y):
    """Distance from a point to the edges of a polygon set, nm."""
    best = float("inf")
    for index in range(poly.OutlineCount()):
        points = [(p.x, p.y) for p in poly.COutline(index).CPoints()]
        for k in range(len(points)):
            (ax, ay), (bx, by) = points[k], points[(k + 1) % len(points)]
            dx, dy = bx - ax, by - ay
            length2 = dx * dx + dy * dy
            t = 0.0 if length2 == 0 else max(0.0, min(1.0, ((x - ax) * dx + (y - ay) * dy) / length2))
            best = min(best, math.hypot(ax + t * dx - x, ay + t * dy - y))
    return best


def label(item, kind):
    if kind == "pad":
        return f"pad {item.GetParentFootprint().GetReference()}.{item.GetNumber()}"
    return kind


def poly_distance(poly, x, y):
    """Distance from a point to the polygon set's copper (0 inside), nm."""
    return math.sqrt(poly.SquaredDistance(pcbnew.VECTOR2I(int(x), int(y))))


class Shapes:
    """Polygons of a layer's items, built once."""

    def __init__(self, layer):
        self.layer = layer
        self.cache = {}

    def of(self, item):
        key = id(item)
        if key not in self.cache:
            poly = pcbnew.SHAPE_POLY_SET()
            item.TransformShapeToPolygon(poly, self.layer, 0, ARC_HIGH_DEF, pcbnew.ERROR_OUTSIDE)
            self.cache[key] = (item, poly)
        return self.cache[key][1]


def thermal_settings(pad, zone):
    gap = pad.GetLocalThermalGapOverride() if hasattr(pad, "GetLocalThermalGapOverride") else None
    if not gap:
        gap = zone.GetThermalReliefGap()
    width = zone.GetThermalReliefSpokeWidth()
    return gap, width


def zone_name(zone):
    if zone.IsTeardropArea():
        return f"teardrop {zone.GetNetname() or '(no net)'}"
    return f"zone {zone.GetNetname() or '(no net)'}"


def classify(board, zones, layer, netcode, items, shapes, ax, ay, bx, by):
    """What cut each side of the neck and what of the net lies at its middle.

    A side is the nearest edge of something that keeps the fill away: another
    net's pad, track, via, teardrop or zone (within 0.5 mm), the net's own
    thermal relief (a thermal pad's copper at its gap), the zone's outline or
    the board's edge; failing those, in a hatched zone, a hatch hole."""
    mx, my = (ax + bx) / 2, (ay + by) / 2
    reach = 1.5 * NM
    box = pcbnew.BOX2I(pcbnew.VECTOR2I(int(mx - reach), int(my - reach)), pcbnew.VECTOR2L(int(2 * reach), int(2 * reach)))
    near = [(item, kind) for item, kind in items if box.Intersects(item.GetBoundingBox())]
    own_zones = [zone for zone in zones if zone.GetNetCode() == netcode and zone.IsOnLayer(layer)
                 and not zone.IsTeardropArea()]
    zone = own_zones[0] if own_zones else None
    hatched = zone is not None and zone.GetFillMode() == pcbnew.ZONE_FILL_MODE_HATCH_PATTERN
    edge = board.GetBoardEdgesBoundingBox()

    def side(x, y):
        candidates = []
        for item, kind in near:
            if item.GetNetCode() == netcode:
                if kind == "pad" and zone is not None:
                    d = poly_distance(shapes.of(item), x, y)
                    gap, _ = thermal_settings(item, zone)
                    if d > 0 and abs(d - gap) < 0.03 * NM:
                        candidates.append((abs(d - gap), f"relief of own {label(item, kind)}"))
                continue
            if kind == "zone":
                filled = item.GetFilledPolysList(layer)
                if not filled.OutlineCount():
                    continue
                d = poly_distance(filled, x, y)
                name = zone_name(item)
            else:
                d = poly_distance(shapes.of(item), x, y)
                name = f"{label(item, kind)} {item.GetNetname() or '(no net)'}"
            if d < 0.5 * NM:
                candidates.append((d, name))
        if zone is not None:
            d = outline_distance(zone.Outline(), x, y)
            if d < 0.03 * NM:
                candidates.append((d, "zone outline"))
        if edge.GetWidth() > 0:
            d = min(x - edge.GetX(), edge.GetRight() - x, y - edge.GetY(), edge.GetBottom() - y)
            if d < 0.6 * NM:
                candidates.append((d, "board edge"))
        candidates.sort()
        if not candidates:
            return "hatch hole" if hatched else "unknown"
        distance, name = candidates[0]
        if hatched and not name.startswith("relief") and distance > 0.45 * NM:
            return "hatch hole"
        return f"{name} @{distance / NM:.3f}"

    middle = "fill"
    for item, kind in near:
        if item.GetNetCode() != netcode or kind == "zone":
            continue
        d = poly_distance(shapes.of(item), mx, my)
        if d == 0:
            middle = f"own {label(item, kind)}"
            break
        if kind == "pad" and zone is not None:
            gap, width = thermal_settings(item, zone)
            if d <= gap + width + 0.01 * NM:
                centre = item.ShapePos(layer)
                angle = math.radians(item.GetOrientation().AsDegrees() + item.GetThermalSpokeAngle().AsDegrees())
                dx, dy = mx - centre.x, my - centre.y
                across = [abs(-dx * math.sin(angle + k * math.pi / 2) + dy * math.cos(angle + k * math.pi / 2)) for k in range(2)]
                in_spoke = any(across[k] <= width / 2 + 0.01 * NM for k in range(2))
                middle = (f"spoke of {label(item, kind)}" if in_spoke
                          else f"ring of {label(item, kind)}" if hatched and d >= gap - 0.01 * NM
                          else f"relief of {label(item, kind)}")
                break
    teardrop = any(kind == "zone" and item.IsTeardropArea() and item.GetNetCode() != netcode
                   and item.GetFilledPolysList(layer).OutlineCount()
                   and poly_distance(item.GetFilledPolysList(layer), mx, my) < 0.5 * NM
                   for item, kind in near)
    return mside(side(ax, ay), side(bx, by)), middle, teardrop


def mside(a, b):
    return " | ".join(sorted([a, b]))


def kicad_violations(drc_path):
    rows = []
    for violation in json.load(open(drc_path)).get("violations", []):
        if violation["type"] != "connection_width":
            continue
        match = re.search(r"actual ([0-9.]+) mm\) \((.*)\)$", violation["description"])
        rows.append({"width": float(match.group(1)) if match else None, "layer": match.group(2) if match else None,
                     "severity": violation.get("severity"),
                     "items": [item["description"] for item in violation.get("items", [])]})
    return rows


def kicad_fill(path, work):
    """The board filled by kicad-cli's DRC (`--refill-zones`, which also
    rebuilds teardrops), saved in `work`: (saved board, its drc.json)."""
    base = os.path.basename(path)[:-len(".kicad_pcb")]
    for extension in (".kicad_pcb", ".kicad_pro", ".kicad_dru"):
        source = path[:-len(".kicad_pcb")] + extension
        if os.path.exists(source):
            shutil.copy(source, os.path.join(work, "board" + extension))
    report = os.path.join(work, "drc.json")
    try:
        subprocess.run(["kicad-cli", "pcb", "drc", "--refill-zones", "--save-board", "--severity-all",
                        "--format", "json", "-o", report, "board.kicad_pcb"], cwd=work,
                       stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL, timeout=900)
    except subprocess.TimeoutExpired:
        return None, None
    return os.path.join(work, "board.kicad_pcb"), report if os.path.exists(report) else None


def check(path, quiet=False):
    """The board's necks, KiCad's own findings (from its DRC on the same
    fill) and, with a drc.json beside the board, the findings recorded there."""
    minimum_mm = project_min_connection(path)
    result = {"board": path, "min_connection": minimum_mm, "necks": [], "custom_rules": custom_rule_widths(path)}
    drc_path = os.path.join(os.path.dirname(path), "drc.json")
    if os.path.exists(drc_path):
        result["recorded"] = kicad_violations(drc_path)
    if minimum_mm <= 0 and not result["custom_rules"]:
        return result
    with tempfile.TemporaryDirectory(prefix="necks-") as work:
        filled, report = kicad_fill(path, work)
        if report is None:
            result["error"] = "kicad-cli did not finish"
            return result
        result["kicad"] = kicad_violations(report)
        board = pcbnew.LoadBoard(filled)
        result.update(necks_of(board))
    return result


def necks_of(board):
    settings = board.GetDesignSettings()
    minimum = settings.m_MinConn
    epsilon = (settings.GetDRCEpsilon() + settings.m_MaxError + int(0.0005 * NM)) * 2
    limit = minimum - epsilon
    rows = []
    if limit <= 0:
        return {"necks": rows}
    zones = [zone for zone in board.Zones() if not zone.GetIsRuleArea()]
    for layer in board.GetEnabledLayers().CuStack():
        layer_name = board.GetLayerName(layer)
        items = items_on_layer(board, zones, layer)
        shapes = Shapes(layer)
        by_net = {}
        for item, kind in items:
            by_net.setdefault(item.GetNetCode(), []).append((item, kind))
        for netcode, members in by_net.items():
            poly = pcbnew.SHAPE_POLY_SET()
            for item, kind in members:
                item.TransformShapeToPolygon(poly, layer, 0, ARC_HIGH_DEF, pcbnew.ERROR_OUTSIDE)
            poly.Fracture()
            for index in range(poly.OutlineCount()):
                points = [(p.x, p.y) for p in poly.COutline(index).CPoints()]
                test = PolygonTest(limit)
                for first, second in sorted(test.find_pairs(points)):
                    ax, ay = points[first]
                    bx, by = points[second]
                    width = math.hypot(ax - bx, ay - by)
                    sides, middle, teardrop = classify(board, zones, layer, netcode, items, shapes, ax, ay, bx, by)
                    # KiCad pairs vertices across a narrow gap between two
                    # parts of one outline too (its test is local).
                    gap = not poly.Contains(pcbnew.VECTOR2I(int((ax + bx) / 2), int((ay + by) / 2)))
                    net = board.FindNet(netcode)
                    rows.append({
                        "net": net.GetNetname() if net else str(netcode), "layer": layer_name,
                        "width": round(width / NM, 4),
                        "x": round((ax + bx) / 2 / NM, 3), "y": round((ay + by) / 2 / NM, 3),
                        "a": [round(ax / NM, 4), round(ay / NM, 4)], "b": [round(bx / NM, 4), round(by / NM, 4)],
                        "sides": sides, "middle": middle, "teardrop": teardrop, "gap": gap,
                    })
    return {"necks": rows}


def find_board(argument):
    if argument.endswith(".kicad_pcb"):
        return argument
    boards = glob.glob(os.path.join(argument, "*.kicad_pcb"))
    return boards[0] if len(boards) == 1 else None


def kind_of(neck):
    """A short class of the neck for the summary."""
    sides, middle = neck["sides"], neck["middle"]
    if neck.get("gap"):
        kind = "gap"
    elif middle.startswith("spoke"):
        kind = "spoke"
    elif middle.startswith("ring"):
        kind = "thermal ring"
    elif middle.startswith("relief"):
        kind = "pad relief"
    elif middle.startswith("own track"):
        kind = "track"
    elif middle.startswith("own via"):
        kind = "via"
    elif middle.startswith("own pad"):
        kind = "pad copper"
    elif "via " in sides:
        kind = "fill by a via"
    elif "track " in sides:
        kind = "fill by a track"
    elif sides.count("pad ") == 2:
        kind = "fill between pads"
    elif "pad " in sides:
        kind = "fill by a pad"
    else:
        kind = "fill"
    return kind + (" (teardrop)" if neck.get("teardrop") else "")


def widths(rows):
    return sorted((row["layer"], row["width"]) for row in rows)


def main():
    parser = argparse.ArgumentParser()
    parser.add_argument("paths", nargs="+")
    parser.add_argument("--jobs", type=int, default=1)
    parser.add_argument("--json")
    parser.add_argument("--quiet", action="store_true")
    args = parser.parse_args()
    boards = [board for board in (find_board(path) for path in args.paths) if board]
    # pcbnew's threads deadlock forked workers: spawn them.
    context = multiprocessing.get_context("spawn")
    with concurrent.futures.ProcessPoolExecutor(max_workers=args.jobs, mp_context=context) as pool:
        results = list(pool.map(check, boards))
    if args.json:
        json.dump(results, open(args.json, "w"), indent=1)
    summary = {}
    agree = disagree = 0
    for result in results:
        necks = result["necks"]
        kicad = result.get("kicad")
        recorded = result.get("recorded")
        if not necks and not kicad and not recorded:
            continue
        line = f"{result['board']}  min {result['min_connection']} mm  ours {len(necks)}"
        if kicad is not None:
            severity = kicad[0]["severity"] if kicad else "-"
            line += f"  KiCad {len(kicad)} ({severity})"
        if recorded is not None:
            line += f"  recorded {len(recorded)}"
        print(line)
        if kicad is not None:
            if widths(necks) == widths(kicad):
                agree += 1
            else:
                disagree += 1
                print(f"  MISMATCH ours {widths(necks)} KiCad {widths(kicad)}")
        for neck in necks:
            summary[kind_of(neck)] = summary.get(kind_of(neck), 0) + 1
            if not args.quiet:
                print(f"  {neck['net']:12} {neck['layer']:6} {neck['width']:.4f} at ({neck['x']:.3f}, {neck['y']:.3f})"
                      f"  {kind_of(neck):24} middle: {neck['middle']}; sides: {neck['sides']}")
    print(f"boards agreeing with KiCad: {agree}, not: {disagree}")
    print("summary:", json.dumps(summary, sort_keys=True))


if __name__ == "__main__":
    main()
