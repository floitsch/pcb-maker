#!/usr/bin/env python3
# Copyright (C) 2026 Toit contributors.
# Use of this source code is governed by an MIT-style license that can be
# found in the LICENSE file.
"""How the balls of the task boards' BGAs escape (docs/router.md, "Prepared
fan-outs for BGAs (2026-11-04)").

    census.py find   [--tasks benchmarks/agent-tasks/tasks/v7/tasks.json] --out DIR
    census.py escape --out DIR [--sweep build/nightly/2026-11-04/sweep]
                     [--designer build/nightly/2026-11-04/designer] [--board KEY=PATH ...]
    census.py table  --out DIR

`find` reads every task's designer board with KiCad's pcbnew and lists the
grid-array footprints: SMD pads (an exposed pad left out) of which at least
three rows and three columns hold three pads or more (a WLCSP-9 too), the
pads round or square and of one size, the pitch (the median
nearest-neighbour distance of the pad centres) at most 1.27 mm. Writes
`DIR/bgas.json`.

`escape` classifies every ball of a net (two pads or more on the board) of
those footprints on the designer's own board, on the sweep's result board
(our layout) and on the designer tier's route (our route on the designer's
placement), and on any extra board given as `name=path`:

- `via_in_pad`: a via of the net with its centre inside the ball;
- `dogbone`: a track on the ball's layer from the ball to a via of the net
  inside the ball field (the box of the ball centres grown by half a pitch),
  `diagonal` when the via sits in a diagonal gap (both offsets between 0.3
  and 0.7 pitches), with the stub's length along the tracks and the layers
  the via leaves on (tracks there, or a zone of the net filled at the via);
- `surface`: a track on the ball's layer that leaves the field before any
  via, with the length to the first node outside;
- `joined`: a track to another ball of the same net (nothing else first);
- `zone`: no track, a zone of the net filled over the ball;
- `stub`: tracks touch the ball but reach none of the above;
- `open`: no copper of the net touches the ball.

The ring of a ball is its distance from the field's box in pitches (0: the
outer ring). Writes `DIR/escapes.json` and, with KiCad's DRC report beside a
board, the unconnected items naming the footprint's pads or nets.
`table` writes `DIR/tables.md`.
"""

import argparse
import json
import math
import os
import statistics
import sys
from collections import Counter, defaultdict

MM = 1.0e6


def load(path):
    import pcbnew

    return pcbnew.LoadBoard(path)


def local(fp, point):
    """A board point in the footprint's frame (mm): translated and turned
    back by the footprint's orientation."""
    position = fp.GetPosition()
    angle = math.radians(fp.GetOrientationDegrees())
    dx = (point[0] - position.x) / MM
    dy = (point[1] - position.y) / MM
    # KiCad's y points down and its angles turn counter-clockwise on screen.
    c, s = math.cos(angle), math.sin(angle)
    return (dx * c - dy * s, dx * s + dy * c)


def smd_pads(fp):
    import pcbnew

    return [
        pad
        for pad in fp.Pads()
        if pad.GetAttribute() in (pcbnew.PAD_ATTRIB_SMD, pcbnew.PAD_ATTRIB_CONN)
    ]


def pad_area(pad):
    import pcbnew

    size = pad.GetSize(pad.GetPrincipalLayer()) if hasattr(pad, "GetPrincipalLayer") else pad.GetSize()
    return size.x * size.y / MM / MM


def cluster(values, tolerance):
    """Sorted cluster centres of `values` (groups closer than `tolerance`)."""
    groups = []
    for value in sorted(values):
        if groups and value - groups[-1][-1] <= tolerance:
            groups[-1].append(value)
        else:
            groups.append([value])
    return [sum(group) / len(group) for group in groups], groups


def grid_of(fp):
    """The footprint's ball grid or None: balls (pad objects), pitch, rows,
    columns, the field's box (local mm)."""
    pads = smd_pads(fp)
    if len(pads) < 9:
        return None
    areas = [pad_area(pad) for pad in pads]
    largest = max(areas)
    others = sorted(areas)[:-1]
    if others and largest >= 2.0 * max(others):
        pads = [pad for pad, area in zip(pads, areas) if area < largest]
    # Balls and lands are round or square; a QFN's or a connector's pads
    # are long (and an exposed pad split into squares lies inside a frame
    # of long pads).
    def squat(pad):
        size = pad.GetSize(pad.GetPrincipalLayer())
        return max(size.x, size.y) <= 1.5 * min(size.x, size.y)

    if sum(1 for pad in pads if squat(pad)) < 0.75 * len(pads):
        return None
    # Balls of one size (a DFN's exposed pad cut in pieces is not).
    middle = statistics.median(pad_area(pad) for pad in pads)
    if sum(1 for pad in pads if middle / 1.5 <= pad_area(pad) <= middle * 1.5) < 0.75 * len(pads):
        return None
    points = [local(fp, (pad.GetPosition().x, pad.GetPosition().y)) for pad in pads]
    nearest = []
    for i, a in enumerate(points):
        best = min(
            (math.dist(a, b) for j, b in enumerate(points) if j != i and math.dist(a, b) > 1.0e-3),
            default=None,
        )
        if best is not None:
            nearest.append(best)
    if not nearest:
        return None
    pitch = statistics.median(nearest)
    if pitch > 1.27 + 1.0e-6:
        return None
    tolerance = 0.3 * pitch
    for axis in (0, 1):
        _, groups = cluster([p[axis] for p in points], tolerance)
        if sum(1 for group in groups if len(group) >= 3) < 3:
            return None
    xs = [p[0] for p in points]
    ys = [p[1] for p in points]
    box = (min(xs), min(ys), max(xs), max(ys))
    rows = len(cluster(ys, tolerance)[0])
    columns = len(cluster(xs, tolerance)[0])
    rings = [
        round(min(p[0] - box[0], box[2] - p[0], p[1] - box[1], box[3] - p[1]) / pitch)
        for p in points
    ]
    # A field with nothing inside the outer ring is a QFN's frame, no grid.
    if max(rings) < 1:
        return None
    return {
        "pads": pads,
        "points": points,
        "pitch": pitch,
        "rows": rows,
        "columns": columns,
        "box": box,
        "rings": rings,
    }


def net_pad_counts(board):
    counts = Counter()
    for pad in board.GetPads():
        if pad.GetNetCode() > 0:
            counts[pad.GetNetCode()] += 1
    return counts


def pour_nets(board):
    """Nets with a copper zone of 4 mm2 or more (not a teardrop, not a rule
    area): pour nets, whose balls join by spokes or vias to the plane."""
    nets = set()
    for zone in board.Zones():
        if zone.GetIsRuleArea() or zone.GetNetCode() <= 0:
            continue
        if hasattr(zone, "IsTeardropArea") and zone.IsTeardropArea():
            continue
        if zone.Outline().Area() / MM / MM >= 4.0:
            nets.add(zone.GetNetCode())
    return nets


def find(args):
    tasks = json.load(open(args.tasks))["tasks"]
    out = []
    for task in tasks:
        path = os.path.join(task["directory"], task["board_id"] + ".kicad_pcb")
        try:
            board = load(path)
        except Exception as error:  # noqa: BLE001
            print(f"{task['name']}: {error}", file=sys.stderr)
            continue
        if board is None:
            print(f"{task['name']}: not loaded", file=sys.stderr)
            continue
        counts = net_pad_counts(board)
        zoned = pour_nets(board)
        for fp in board.GetFootprints():
            grid = grid_of(fp)
            if grid is None:
                continue
            balls = [pad for pad in grid["pads"] if counts[pad.GetNetCode()] >= 2]
            out.append(
                {
                    "task": task["name"],
                    "board_id": task["board_id"],
                    "path": path,
                    "reference": fp.GetReference(),
                    "footprint": str(fp.GetFPID().GetUniStringLibItemName()),
                    "pitch_mm": round(grid["pitch"], 4),
                    "rows": grid["rows"],
                    "columns": grid["columns"],
                    "pads": len(grid["pads"]),
                    "balls": len(balls),
                    "signal_balls": sum(1 for pad in balls if pad.GetNetCode() not in zoned),
                    "max_ring": max(grid["rings"]),
                    "layers": board.GetCopperLayerCount(),
                }
            )
            print(json.dumps(out[-1]), flush=True)
    os.makedirs(args.out, exist_ok=True)
    json.dump(out, open(os.path.join(args.out, "bgas.json"), "w"), indent=1)


class Copper:
    """A board's tracks, vias and zones by net."""

    def __init__(self, board):
        import pcbnew

        self.board = board
        self.segments = defaultdict(list)  # (net, layer) -> [(a, b)]
        self.vias = defaultdict(list)  # net -> [(at, radius, drill)]
        for item in board.GetTracks():
            net = item.GetNetCode()
            if item.Type() == pcbnew.PCB_VIA_T:
                via = pcbnew.Cast_to_PCB_VIA(item) if hasattr(pcbnew, "Cast_to_PCB_VIA") else item
                try:
                    width = via.GetWidth(pcbnew.F_Cu)
                except TypeError:
                    width = via.GetWidth()
                self.vias[net].append(
                    ((via.GetPosition().x, via.GetPosition().y), width / 2.0, via.GetDrillValue() / 2.0)
                )
            else:
                start, end = item.GetStart(), item.GetEnd()
                self.segments[(net, item.GetLayer())].append(((start.x, start.y), (end.x, end.y)))
        self.zones = defaultdict(list)
        for zone in board.Zones():
            self.zones[zone.GetNetCode()].append(zone)

    def zone_at(self, net, layer, point):
        import pcbnew

        for zone in self.zones.get(net, []):
            if zone.IsOnLayer(layer) and zone.HitTestFilledArea(layer, pcbnew.VECTOR2I(int(point[0]), int(point[1]))):
                return True
        return False

    def segment_layers_at(self, net, point, radius, exclude):
        layers = set()
        for (n, layer), segments in self.segments.items():
            if n != net or layer == exclude:
                continue
            for a, b in segments:
                if math.dist(a, point) <= radius or math.dist(b, point) <= radius:
                    layers.add(layer)
                    break
        return layers


def copper_layer(pad):
    """The SMD pad's copper layer (`GetPrincipalLayer` answers F.Cu for a
    flipped footprint's pads on KiCad 10)."""
    layers = list(pad.GetLayerSet().CuStack())
    return layers[0] if layers else pad.GetPrincipalLayer()


def classify_ball(board, copper, fp, grid, pad, field):
    import pcbnew

    net = pad.GetNetCode()
    layer = copper_layer(pad)
    center = (pad.GetPosition().x, pad.GetPosition().y)
    pitch = grid["pitch"]

    def inside_pad(point):
        return pad.HitTest(pcbnew.VECTOR2I(int(point[0]), int(point[1])))

    def in_field(point):
        x, y = local(fp, point)
        return field[0] <= x <= field[2] and field[1] <= y <= field[3]

    for at, radius, _ in copper.vias.get(net, []):
        if inside_pad(at):
            return {"kind": "via_in_pad", "via_mm": round(2 * radius / MM, 3)}
    segments = copper.segments.get((net, layer), [])
    key = lambda p: (round(p[0] / 1000), round(p[1] / 1000))  # noqa: E731
    adjacency = defaultdict(list)
    for a, b in segments:
        length = math.dist(a, b) / MM
        adjacency[key(a)].append((key(b), b, length))
        adjacency[key(b)].append((key(a), a, length))
    starts = [p for a, b in segments for p in (a, b) if inside_pad(p)]
    if not starts:
        if copper.zone_at(net, layer, center):
            return {"kind": "zone"}
        return {"kind": "open"}
    others = [
        other
        for other in fp.Pads()
        if other.GetNetCode() == net and other.GetNumber() != pad.GetNumber()
    ]
    vias = [v for v in copper.vias.get(net, []) if not inside_pad(v[0])]
    import heapq

    best = {}
    heap = []
    for p in starts:
        heapq.heappush(heap, (math.dist(p, center) / MM, key(p), p))
    while heap:
        distance, k, point = heapq.heappop(heap)
        if k in best:
            continue
        best[k] = distance
        for at, radius, _ in vias:
            if math.dist(at, point) <= max(radius, 2000):
                x, y = local(fp, at)
                cx, cy = local(fp, center)
                dx, dy = abs(x - cx) / pitch, abs(y - cy) / pitch
                if not in_field(at):
                    return {"kind": "surface", "length_mm": round(distance, 3), "via_outside": True}
                exits = copper.segment_layers_at(net, at, radius, layer)
                planes = [
                    other_layer
                    for other_layer in board.GetEnabledLayers().CuStack()
                    if other_layer != layer and copper.zone_at(net, other_layer, at)
                ]
                return {
                    "kind": "dogbone",
                    "length_mm": round(distance, 3),
                    "diagonal": 0.3 <= dx <= 0.7 and 0.3 <= dy <= 0.7,
                    "offset_pitches": [round(dx, 2), round(dy, 2)],
                    "exit_layers": sorted(board.GetLayerName(other) for other in exits),
                    "plane_layers": sorted(board.GetLayerName(other) for other in planes),
                }
        if not inside_pad(point):
            for other in others:
                if other.HitTest(pcbnew.VECTOR2I(int(point[0]), int(point[1]))):
                    return {"kind": "joined", "to": other.GetNumber(), "length_mm": round(distance, 3)}
        if not in_field(point):
            return {"kind": "surface", "length_mm": round(distance, 3)}
        for next_key, next_point, length in adjacency[k]:
            if next_key not in best:
                heapq.heappush(heap, (distance + length, next_key, next_point))
    return {"kind": "stub"}


def unconnected(drc_path, reference, nets):
    if not os.path.exists(drc_path):
        return None
    drc = json.load(open(drc_path))
    on_pads = 0
    on_nets = 0
    for item in drc.get("unconnected_items", []):
        texts = [entry.get("description", "") for entry in item.get("items", [])]
        if any(f" of {reference} " in text for text in texts):
            on_pads += 1
        if any(any(f"[{net}]" in text for net in nets) for text in texts):
            on_nets += 1
    return {"items_at_pads": on_pads, "items_on_nets": on_nets}


def escapes_of(path, reference, drc_path=None):
    board = load(path)
    fp = board.FindFootprintByReference(reference)
    if fp is None:
        return None
    grid = grid_of(fp)
    if grid is None:
        return None
    copper = Copper(board)
    counts = net_pad_counts(board)
    zoned = pour_nets(board)
    box = grid["box"]
    half = grid["pitch"] / 2.0
    field = (box[0] - half, box[1] - half, box[2] + half, box[3] + half)
    balls = []
    nets = set()
    for pad, ring in zip(grid["pads"], grid["rings"]):
        if counts[pad.GetNetCode()] < 2:
            continue
        nets.add(pad.GetNetname())
        record = classify_ball(board, copper, fp, grid, pad, field)
        record.update(
            {
                "ball": pad.GetNumber(),
                "net": pad.GetNetname(),
                "ring": ring,
                "pour_net": pad.GetNetCode() in zoned,
                "layer": board.GetLayerName(copper_layer(pad)),
            }
        )
        # Our result boards are written with their zones unfilled: a pour
        # net's ball with no copper of its own is the pour's.
        if record["kind"] == "open" and record["pour_net"]:
            record["kind"] = "zone"
        balls.append(record)
    result = {
        "path": path,
        "side": board.GetLayerName(fp.GetLayer()),
        "orientation": fp.GetOrientationDegrees(),
        "balls": balls,
    }
    if drc_path:
        result["unconnected"] = unconnected(drc_path, reference, nets)
    return result


def escape(args):
    bgas = json.load(open(os.path.join(args.out, "bgas.json")))
    path = os.path.join(args.out, "escapes.json")
    done = json.load(open(path)) if os.path.exists(path) else {}
    extra = dict(entry.split("=", 1) for entry in args.board or [])
    for bga in bgas:
        key = f"{bga['task']}/{bga['reference']}"
        if args.only and not any(only in key for only in args.only):
            continue
        sides = {"designer": (bga["path"], None)}
        layout = os.path.join(args.sweep, bga["task"], "layout", "result")
        sides["layout"] = (
            os.path.join(layout, bga["board_id"] + ".kicad_pcb"),
            os.path.join(layout, "drc.json"),
        )
        route = os.path.join(args.designer, bga["task"], "route")
        sides["designer_route"] = (
            os.path.join(route, bga["board_id"] + ".kicad_pcb"),
            os.path.join(route, "drc.json"),
        )
        for name, board_path in extra.items():
            sides[name] = (board_path.replace("{task}", bga["task"]).replace("{board_id}", bga["board_id"]), None)
        entry = done.get(key, {})
        for side, (board_path, drc_path) in sides.items():
            if side in entry and not args.redo:
                continue
            if not os.path.exists(board_path):
                continue
            try:
                entry[side] = escapes_of(board_path, bga["reference"], drc_path)
            except Exception as error:  # noqa: BLE001
                print(f"{key} {side}: {error}", file=sys.stderr)
        done[key] = entry
        json.dump(done, open(path, "w"), indent=1)
        print(key, summary_line(entry), flush=True)


def summarise(side):
    if not side:
        return None
    pour = Counter(ball["kind"] for ball in side["balls"] if ball["pour_net"])
    signal = [ball for ball in side["balls"] if not ball["pour_net"]]
    kinds = Counter(ball["kind"] for ball in signal)
    dog = [ball for ball in signal if ball["kind"] == "dogbone"]
    stubs = sorted(ball["length_mm"] for ball in dog)
    layers = Counter()
    for ball in dog:
        exits = ball["exit_layers"] or [f"plane {name}" for name in ball["plane_layers"]] or ["none"]
        layers[exits[0]] += 1
    per_ring = defaultdict(Counter)
    for ball in signal:
        per_ring[ball["ring"]][ball["kind"]] += 1
    return {
        "kinds": dict(kinds),
        "pour": dict(pour),
        "diagonal": sum(1 for ball in dog if ball["diagonal"]),
        "stub_median": round(statistics.median(stubs), 2) if stubs else None,
        "stub_p90": round(stubs[int(0.9 * (len(stubs) - 1))], 2) if stubs else None,
        "stub_max": round(stubs[-1], 2) if stubs else None,
        "via_layers": dict(layers),
        "per_ring": {ring: dict(counter) for ring, counter in sorted(per_ring.items())},
        "unconnected": side.get("unconnected"),
    }


def summary_line(entry):
    return {side: (summarise(value) or {}).get("kinds") for side, value in entry.items()}


def table(args):
    bgas = json.load(open(os.path.join(args.out, "bgas.json")))
    done = json.load(open(os.path.join(args.out, "escapes.json")))
    lines = [
        "| Board | Ref | Pitch | Rows x cols | Balls (signal) | Rings | Side | signal balls: via in pad / dog-bone (diagonal) / surface / joined / zone / stub / open | stub mm median / p90 / max | dog-bone vias by exit layer | open signal balls by ring | pour balls | unconnected at pads / on its nets |",
        "| --- | --- | ---: | --- | --- | ---: | --- | --- | --- | --- | --- | --- | --- |",
    ]
    for bga in bgas:
        key = f"{bga['task']}/{bga['reference']}"
        entry = done.get(key, {})
        for side, value in entry.items():
            summary = summarise(value)
            if not summary:
                continue
            kinds = summary["kinds"]
            counts = " / ".join(
                str(kinds.get(kind, 0)) if kind != "dogbone" else f"{kinds.get(kind, 0)} ({summary['diagonal']})"
                for kind in ("via_in_pad", "dogbone", "surface", "joined", "zone", "stub", "open")
            )
            stubs = (
                f"{summary['stub_median']} / {summary['stub_p90']} / {summary['stub_max']}"
                if summary["stub_median"] is not None
                else "-"
            )
            layers = ", ".join(f"{name} {count}" for name, count in sorted(summary["via_layers"].items()))
            by_ring = " ".join(
                f"r{ring}:{kinds_of.get('open', 0)}/{sum(kinds_of.values())}"
                for ring, kinds_of in summary["per_ring"].items()
            )
            pour = ", ".join(f"{kind} {count}" for kind, count in sorted(summary["pour"].items())) or "-"
            open_items = summary["unconnected"]
            open_text = f"{open_items['items_at_pads']} / {open_items['items_on_nets']}" if open_items else "-"
            lines.append(
                f"| {bga['task']} | {bga['reference']} | {bga['pitch_mm']} | {bga['rows']} x {bga['columns']} "
                f"| {bga['balls']} ({bga['signal_balls']}) | {bga['max_ring'] + 1} | {side} | {counts} | {stubs} "
                f"| {layers or '-'} | {by_ring} | {pour} | {open_text} |"
            )
    text = "\n".join(lines) + "\n"
    open(os.path.join(args.out, "tables.md"), "w").write(text)
    print(text)


def main():
    parser = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    sub = parser.add_subparsers(dest="command", required=True)
    one = sub.add_parser("find")
    one.add_argument("--tasks", default="benchmarks/agent-tasks/tasks/v7/tasks.json")
    one.add_argument("--out", required=True)
    two = sub.add_parser("escape")
    two.add_argument("--out", required=True)
    two.add_argument("--sweep", default="build/nightly/2026-11-04/sweep")
    two.add_argument("--designer", default="build/nightly/2026-11-04/designer")
    two.add_argument("--board", action="append", help="name=path, {task} and {board_id} substituted")
    two.add_argument("--only", action="append")
    two.add_argument("--redo", action="store_true")
    three = sub.add_parser("table")
    three.add_argument("--out", required=True)
    args = parser.parse_args()
    {"find": find, "escape": escape, "table": table}[args.command](args)


if __name__ == "__main__":
    main()
