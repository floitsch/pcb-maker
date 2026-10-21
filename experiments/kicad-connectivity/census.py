#!/usr/bin/env python3
# Copyright (C) 2026 Toit contributors.
"""Where our connectivity and KiCad's disagree: the intake's boards (or a
route-mode run's) that the router counts complete while KiCad's DRC
(zones refilled) reports unconnected items, each classified by the cause
of its unconnected items (docs/changelog.md, "KiCad connectivity
agreement").

    python3 -I experiments/kicad-connectivity/census.py              # the intake (benchmarks/expansion/data)
    python3 -I experiments/kicad-connectivity/census.py --run build/conn-x1322 [--json out.json]

The causes, from the routed board and KiCad's report (items by uuid):

    numeric_net_name      a net whose name is a number ("14"): the lowering
                          took it for a net code and left it unrouted
    same_number_stacked   pads of one footprint and number on different
                          layers at one place (an edge connector's top and
                          bottom pads, a USB-C shell's): two items to KiCad
    pour_zone             a zone is one of the items: the refilled pour does
                          not reach a pad or track our pour model connected
    pour_net              two items of a net with a pour, neither a zone:
                          the refilled pour does not join them
    far_side_end          a track ends on a pad that has no copper on the
                          track's layer
    copper_graphic        a copper graphic with a net (a line, arc, circle)
    other                 anything else (inspect)

Nothing in the boards is executed; they are parsed as data.
"""
import argparse
import collections
import json
import math
import re
from pathlib import Path

ROOT = Path(__file__).resolve().parents[2]
DATA = ROOT / "benchmarks/expansion/data"

class Quoted(str):
    """A string the file quoted: `(net "5")` names a net, `(net 5)` is a code."""


TOKEN = re.compile(r'\s*(?:(\()|(\))|"((?:[^"\\]|\\.)*)"|([^\s()"]+))', re.S)


def parse(text):
    stack = [[]]
    position = 0
    while position < len(text):
        match = TOKEN.match(text, position)
        if not match:
            if not text[position:].strip():
                break
            raise ValueError(f"bad token at {position}")
        position = match.end()
        if match.group(1):
            stack.append([])
        elif match.group(2):
            item = stack.pop()
            stack[-1].append(item)
        elif match.group(3) is not None:
            stack[-1].append(Quoted(match.group(3).replace('\\"', '"').replace('\\\\', '\\')))
        else:
            stack[-1].append(match.group(4))
    return stack[0][0]


def child(expr, name):
    return next((item for item in expr[1:] if isinstance(item, list) and item and item[0] == name), None)


def children(expr, name):
    return [item for item in expr[1:] if isinstance(item, list) and item and item[0] == name]


def atom(expr, name, index=1):
    found = child(expr, name)
    if found is None or len(found) <= index or isinstance(found[index], list):
        return None
    return found[index]


def at(expr, name="at"):
    found = child(expr, name)
    if found is None:
        return None
    angle = float(found[3]) if len(found) > 3 and not isinstance(found[3], list) and found[3] not in ("locked", "unlocked") else 0.0
    return [float(found[1]), float(found[2]), angle]


def net_of(expr):
    found = child(expr, "net")
    if found is None or len(found) < 2:
        return None
    return found[-1]


def layer_names(expr):
    found = child(expr, "layers")
    if found is not None:
        return [name for name in found[1:] if isinstance(name, str)]
    name = atom(expr, "layer")
    return [name] if name else []


def copper(names, stack):
    out = set()
    for name in names:
        if name == "*.Cu":
            out |= set(stack)
        elif name == "F&B.Cu":
            out |= {stack[0], stack[-1]}
        elif name.endswith(".Cu"):
            out.add(name)
    return out


def rotate(x, y, degrees):
    angle = math.radians(degrees)
    return x * math.cos(angle) + y * math.sin(angle), -x * math.sin(angle) + y * math.cos(angle)


def board_index(path):
    pcb = parse(Path(path).read_text(encoding="utf-8", errors="replace"))
    stack = [entry[1] for entry in (child(pcb, "layers") or [])[1:]
             if isinstance(entry, list) and len(entry) > 1 and str(entry[1]).endswith(".Cu")]
    # KiCad 9 and before: items name their net by its code, `(net 2)`.
    codes = {entry[1]: entry[2] for entry in children(pcb, "net") if len(entry) > 2}

    def named(net):
        if net is None or isinstance(net, Quoted) or not net.isdigit():
            return net
        return codes.get(net, net)

    items = {}
    zone_nets = set()
    for item in pcb[1:]:
        if not isinstance(item, list) or not item:
            continue
        head = item[0]
        if head == "footprint":
            origin = at(item) or [0, 0, 0]
            reference = None
            for field in children(item, "property"):
                if len(field) > 2 and field[1] == "Reference":
                    reference = field[2]
            footprint = {"reference": reference, "pads": []}
            for pad in children(item, "pad"):
                place = at(pad) or [0, 0, 0]
                dx, dy = rotate(place[0], place[1], origin[2])
                size = child(pad, "size")
                record = {"kind": "pad", "footprint": footprint, "number": pad[1], "type": pad[2],
                          "position": (origin[0] + dx, origin[1] + dy),
                          "size": (float(size[1]), float(size[2])) if size else (0.0, 0.0),
                          "layers": copper(layer_names(pad), stack), "net": net_of(pad)}
                footprint["pads"].append(record)
                uuid = atom(pad, "uuid") or atom(pad, "tstamp")
                if uuid:
                    items[uuid] = record
        elif head in ("segment", "arc", "via", "zone") or head.startswith("gr_"):
            net = atom(item, "net_name") if head == "zone" else named(net_of(item))
            record = {"kind": head, "net": net,
                      "layers": set(stack) if head == "via" else copper(layer_names(item), stack)}
            if head == "zone" and net and record["layers"] and child(item, "keepout") is None:
                zone_nets.add(net)
            uuid = atom(item, "uuid") or atom(item, "tstamp")
            if uuid:
                items[uuid] = record
    return {"items": items, "zone_nets": zone_nets}


def distance(a, b):
    return math.hypot(a[0] - b[0], a[1] - b[1])


def stacked_elsewhere(pad):
    """A same-number pad of the footprint at the same place with no copper
    layer in common."""
    reach = max(pad["size"]) / 2
    return any(other is not pad and other["number"] == pad["number"] and other["number"]
               and distance(other["position"], pad["position"]) < max(reach, max(other["size"]) / 2)
               and not other["layers"] & pad["layers"]
               for other in pad["footprint"]["pads"])


def classify(index, report_items):
    found = [index["items"].get(item.get("uuid")) for item in report_items]
    descriptions = [item.get("description", "") for item in report_items]
    if any(record and record["net"] and record["net"].isdigit() for record in found):
        return "numeric_net_name"
    pads = [record for record in found if record and record["kind"] == "pad"]
    if any(pad for pad in pads if stacked_elsewhere(pad)) or (
            len(pads) == 2 and pads[0]["footprint"] is pads[1]["footprint"]
            and pads[0]["number"] == pads[1]["number"] and not pads[0]["layers"] & pads[1]["layers"]):
        return "same_number_stacked"
    if any((record and record["kind"] == "zone") or text.startswith("Zone") for record, text in zip(found, descriptions)):
        return "pour_zone"
    if any(record and record["kind"].startswith("gr_") for record in found) or any(
            text.split(" ")[0] in ("Arc", "Circle", "Rectangle", "Polygon", "Segment", "Bezier") for text in descriptions):
        return "copper_graphic"
    if {record["net"] for record in found if record} & index["zone_nets"]:
        return "pour_net"
    if len(pads) == 1:
        others = [record for record in found if record is not None and record is not pads[0]]
        if others and others[0]["kind"] in ("segment", "arc") and not others[0]["layers"] & pads[0]["layers"]:
            return "far_side_end"
    return "other"


def board_causes(board, report):
    drc = json.loads(Path(report).read_text())
    index = board_index(board)
    return [classify(index, entry["items"]) for entry in drc.get("unconnected_items", [])]


def complete(result):
    return (result.get("routable_connections") is not None
            and result.get("routed_connections") == result.get("routable_connections")
            and result.get("unconnected_terminals", 0) == 0)


def disagreeing(data=DATA):
    """The intake's rows (the last per board) where the router counted every
    connection and KiCad's check, which loaded the board, found unconnected
    items."""
    rows = {}
    with open(data / "runs/quick/results.jsonl") as stream:
        for line in stream:
            row = json.loads(line)
            rows[row["name"]] = row
    out = []
    for row in rows.values():
        native = row.get("native_result") or {}
        summary = native.get("summary") or {}
        if (complete(row.get("route") or {}) and (summary.get("unconnected") or 0) > 0
                and (native.get("process") or {}).get("exit_code") == 0):
            out.append(row)
    return out


def intake(data):
    records = []
    for row in disagreeing(data):
        directory = Path(row["work_directory"])
        board = next((directory / "routed").glob("*.kicad_pcb"))
        causes = board_causes(board, directory / "result-drc.json")
        records.append({"name": row["name"], "router_complete": True, "kicad_unconnected": len(causes),
                        "first": causes[0] if causes else None, "causes": dict(collections.Counter(causes)),
                        "license": (row.get("provenance") or {}).get("license_spdx")})
    return records


def run(directory):
    records = []
    for task in sorted(Path(directory).iterdir()):
        report = task / "route/board-router.json"
        drc = task / "route/drc.json"
        if not report.is_file():
            continue
        result = json.loads(report.read_text())
        board = next((task / "route").glob("*.kicad_pcb"), None)
        causes = board_causes(board, drc) if drc.is_file() and board else []
        records.append({"name": task.name, "router_complete": complete(result),
                        "open": result.get("routable_connections", 0) - result.get("routed_connections", 0),
                        "kicad_unconnected": len(causes) if drc.is_file() else None,
                        "first": causes[0] if causes else None, "causes": dict(collections.Counter(causes))})
    return records


def main():
    parser = argparse.ArgumentParser()
    parser.add_argument("--data", type=Path, default=DATA, help="the harvest's data directory")
    parser.add_argument("--run", type=Path, help="a run.py output directory of route-mode tasks")
    parser.add_argument("--json", type=Path, help="write the per-board records here")
    arguments = parser.parse_args()
    records = run(arguments.run) if arguments.run else intake(arguments.data)
    if arguments.json:
        arguments.json.write_text(json.dumps(records, indent=1))
    checked = [record for record in records if record["kicad_unconnected"] is not None]
    agree = sum(1 for record in checked if record["router_complete"] == (record["kicad_unconnected"] == 0))
    print(f"boards {len(records)}, KiCad checked {len(checked)}, verdict agrees with KiCad {agree}, "
          f"KiCad unconnected {sum(record['kicad_unconnected'] for record in checked)}")
    disagree = [record for record in checked if record["router_complete"] and record["kicad_unconnected"]]
    print(f"router complete, KiCad unconnected: {len(disagree)} boards")
    first = collections.Counter(record["first"] for record in disagree)
    items = collections.Counter()
    for record in disagree:
        items.update(record["causes"])
    print("| cause | boards (first item) | items |")
    print("| --- | ---: | ---: |")
    for cause, count in first.most_common():
        print(f"| {cause} | {count} | {items[cause]} |")
    for cause in sorted(set(items) - set(first)):
        print(f"| {cause} | 0 | {items[cause]} |")


if __name__ == "__main__":
    main()
