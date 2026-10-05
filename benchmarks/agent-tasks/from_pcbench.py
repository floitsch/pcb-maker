#!/usr/bin/env python3
# Copyright (C) 2026 Toit contributors.
"""Generates agent layout tasks from PCBench boards, or from any board list
in the corpus runner's format (`--corpus benchmarks/github/boards.json`).

    benchmarks/agent-tasks/from_pcbench.py <output-dir> [--subset d3-test] [--limit N]
    benchmarks/agent-tasks/from_pcbench.py <output-dir> --corpus <boards.json>

For each board the task is: keep the outline, put every connector the
designer placed at an edge on that edge (in the designer's orientation),
keep parts that hang over the outline where they are, keep parts whose
courtyard holds other parts (a shield's outline) where they are and let
parts sit inside them (`hollow`), and place everything else from a single
stack. The output directory receives tasks.json and one
constraints file per board; run them with
`benchmarks/agent-tasks/run.py <out> --tasks <output-dir>/tasks.json`."""

import argparse
import json
import re
import subprocess
import sys
from pathlib import Path

HERE = Path(__file__).resolve().parent
ROOT = HERE.parents[1]
sys.path.insert(0, str(ROOT / "benchmarks/pcbench"))
import run as pcbench  # noqa: E402
import importlib.util  # noqa: E402

_spec = importlib.util.spec_from_file_location("agent_run", HERE / "run.py")
agent_run = importlib.util.module_from_spec(_spec)
_spec.loader.exec_module(agent_run)

WORK = ROOT / "benchmarks/real/external/pcbench-work/newdrc"
STEM = "processed_v9_guide_v3"
CONNECTOR_PREFIXES = ("J", "P", "CN", "CON", "USB", "X")


def outline_carriers(board_file):
    """References of footprints that draw on Edge.Cuts (cutouts in key
    switch footprints, connector slots): they are part of the outline, so
    they stay."""
    text = board_file.read_text()
    carriers = set()
    last = 0
    for match in re.finditer(r'\(footprint "', text):
        if match.start() < last:
            continue
        last = agent_run.block_end(text, match.start())
        block = text[match.start():last]
        if '"Edge.Cuts"' in block:
            reference = re.search(r'(?:property "Reference"|fp_text reference) "([^"]*)"', block)
            if reference and reference.group(1):
                carriers.add(reference.group(1))
    return carriers


def board_cutouts(board_file):
    """Bounding boxes of the holes the board's own Edge.Cuts draws as
    circles, rectangles or polygons (a phone's bottom connector sits over
    three round cutouts: Sisu's J8)."""
    text = board_file.read_text()
    boxes = []
    for match in re.finditer(r'\((gr_circle|gr_rect|gr_poly)\b', text):
        block = text[match.start():agent_run.block_end(text, match.start())]
        if '"Edge.Cuts"' not in block:
            continue
        points = [(float(x), float(y)) for x, y in re.findall(r'\((?:start|end|center|xy) ([-\d.]+) ([-\d.]+)\)', block)]
        if not points:
            continue
        if match.group(1) == "gr_circle":
            (cx, cy), (ex, ey) = points[0], points[1]
            radius = ((ex - cx) ** 2 + (ey - cy) ** 2) ** 0.5
            boxes.append((cx - radius, cy - radius, cx + radius, cy + radius))
        else:
            xs, ys = [x for x, _ in points], [y for _, y in points]
            boxes.append((min(xs), min(ys), max(xs), max(ys)))
    return boxes


def mask_openings(board_file):
    """Boxes of the board's own graphics on F.Mask / B.Mask (not text), with
    their side: hand-drawn pad openings (OpenFC's over its QFN) belong to
    the part under them."""
    text = board_file.read_text()
    boxes = []
    for match in re.finditer(r'\((gr_line|gr_rect|gr_arc|gr_circle|gr_poly)\b', text):
        block = text[match.start():agent_run.block_end(text, match.start())]
        side = re.search(r'\(layer "?([FB])\.Mask"?\)', block)
        if not side:
            continue
        points = [(float(x), float(y)) for x, y in re.findall(r'\((?:start|end|mid|center|xy) ([-\d.]+) ([-\d.]+)\)', block)]
        if not points:
            continue
        if match.group(1) == "gr_circle":
            (cx, cy), (ex, ey) = points[0], points[1]
            radius = ((ex - cx) ** 2 + (ey - cy) ** 2) ** 0.5
            boxes.append((side.group(1), (cx - radius, cy - radius, cx + radius, cy + radius)))
        else:
            xs, ys = [x for x, _ in points], [y for _, y in points]
            boxes.append((side.group(1), (min(xs), min(ys), max(xs), max(ys))))
    return boxes


def is_connector(footprint):
    reference = footprint["reference"]
    prefix = reference.rstrip("0123456789")
    return prefix in CONNECTOR_PREFIXES and sum(1 for _, net in footprint["pads"] if net) >= 1


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("output", type=Path)
    parser.add_argument("--subset", default="d3-test")
    parser.add_argument("--corpus", type=Path, help="a boards.json in the corpus runner's format instead of PCBench")
    parser.add_argument("--limit", type=int, default=0)
    parser.add_argument("--binary", type=Path, default=ROOT / "target/release/pcb-maker")
    parser.add_argument("--reach", type=float, default=2.0, help="distance to an edge that counts as at the edge")
    arguments = parser.parse_args()
    arguments.output.mkdir(parents=True, exist_ok=True)
    if arguments.corpus:
        boards = []
        for board in json.loads(arguments.corpus.read_text())["boards"]:
            directory = Path(board["directory"])
            boards.append({"name": board["name"], "directory": directory if directory.is_absolute() else ROOT / directory,
                           "board_id": board["board_id"]})
    else:
        manifest = json.loads((ROOT / "benchmarks/pcbench/manifest.json").read_text())
        boards = [{"name": b["name"], "directory": WORK / b["name"], "board_id": STEM}
                  for b in pcbench.subset(manifest, arguments.subset)]
    if arguments.limit:
        boards = boards[:arguments.limit]
    tasks = []
    for board in boards:
        name = board["name"]
        if not (board["directory"] / f'{board["board_id"]}.kicad_pcb').exists():
            print(f"skipping {name}: {board['directory']} not found", file=sys.stderr)
            continue
        described = subprocess.run([str(arguments.binary), "describe-kicad-board", str(board["directory"]), board["board_id"]],
                                   capture_output=True, text=True)
        if described.returncode != 0:
            print(f"skipping {name}: {described.stderr.strip()[-200:]}", file=sys.stderr)
            continue
        description = json.loads(described.stdout)
        outline = description["outline"]
        if outline is None:
            print(f"skipping {name}: no outline", file=sys.stderr)
            continue
        carriers = outline_carriers(board["directory"] / f'{board["board_id"]}.kicad_pcb')
        cutouts = board_cutouts(board["directory"] / f'{board["board_id"]}.kicad_pcb')
        openings = mask_openings(board["directory"] / f'{board["board_id"]}.kicad_pcb')
        low, high = outline["minimum"], outline["maximum"]
        edges, rotations, fixed, hollow = [], [], [], []
        held = set()
        project = board["directory"] / f'{board["board_id"]}.kicad_pro'
        try:
            severity = json.loads(project.read_text())["board"]["design_settings"]["rule_severities"].get("courtyards_overlap", "error")
        except (OSError, ValueError, KeyError, TypeError):
            severity = "error"
        overlap_allowed = severity != "error"
        footprints = [f for f in description["footprints"] if f["reference"]]
        for footprint in footprints:
            body = footprint["body"]
            inside = [other for other in footprints if other is not footprint
                      and (other["side"] == footprint["side"] or footprint["through_hole"] or other["through_hole"])
                      and body[0] <= other["body"][0] and body[1] <= other["body"][1]
                      and other["body"][2] <= body[2] and other["body"][3] <= body[3]]
            if inside and any(net for _, net in footprint["pads"]):
                # It holds other parts: a shield's outline, a module over
                # parts. It stays, and parts may sit inside it, where the
                # project lets courtyards overlap; where that is an error
                # (link's U1), the parts the designer put inside stay too
                # and no other may enter.
                if overlap_allowed:
                    hollow.append(footprint["reference"])
                else:
                    held.update(other["reference"] for other in inside)
                    held.add(footprint["reference"])
        for footprint in description["footprints"]:
            if not footprint["reference"] or not any(net for _, net in footprint["pads"]):
                continue
            if footprint["reference"] in hollow or footprint["reference"] in carriers or footprint["reference"] in held:
                fixed.append(footprint["reference"])
                continue
            body = footprint["body"]
            # Over a cutout (and not just the outline's bounding box): the
            # part is fitted to the hole, it stays.
            if any(box[0] < body[2] and body[0] < box[2] and box[1] < body[3] and body[1] < box[3]
                   and not (box[0] <= low[0] + 0.01 and box[1] <= low[1] + 0.01
                            and box[2] >= high[0] - 0.01 and box[3] >= high[1] - 0.01)
                   for box in cutouts):
                fixed.append(footprint["reference"])
                continue
            # Under one of the board's own mask openings on its side (or
            # through-hole): the opening was drawn for it.
            if any((footprint["through_hole"] or footprint["side"][:1].upper() == side)
                   and box[0] < body[2] and body[0] < box[2] and box[1] < body[3] and body[1] < box[3]
                   for side, box in openings):
                fixed.append(footprint["reference"])
                continue
            distances = {"left": body[0] - low[0], "top": body[1] - low[1],
                         "right": high[0] - body[2], "bottom": high[1] - body[3]}
            if min(distances.values()) < -0.01:
                # The designer lets it hang over the outline: it stays.
                fixed.append(footprint["reference"])
                continue
            if not is_connector(footprint):
                continue
            # Ties (a header along a whole side touches two edges): the edge
            # along the part's long side.
            long_horizontal = (body[2] - body[0]) >= (body[3] - body[1])
            along = {"top": long_horizontal, "bottom": long_horizontal,
                     "left": not long_horizontal, "right": not long_horizontal}
            side, distance = min(distances.items(), key=lambda item: (round(item[1], 2), not along[item[0]]))
            if distance <= arguments.reach:
                edges.append({"part": footprint["reference"], "edge": side, "max_mm": round(distance + 0.5, 2)})
                rotations.append({"part": footprint["reference"], "angle": footprint["at"][2] % 360})
        constraints = {"version": 1, "move_all": True, "fixed": fixed, "edge": edges, "rotation": rotations}
        if hollow:
            constraints["hollow"] = hollow
        (arguments.output / f"{name}.json").write_text(json.dumps(constraints, indent=1))
        tasks.append({"name": name, "directory": str(board["directory"]), "board_id": board["board_id"],
                      # Harvested boards keep their makers' DRC findings;
                      # PCBench's were cleaned by the D3 chain.
                      **({"designer_budget": True} if arguments.corpus else {}),
                      "unplace": True, "remove_outline": False,
                      "stack_at": [(low[0] + high[0]) / 2, (low[1] + high[1]) / 2],
                      "constraints": f"{name}.json", "router": {}})
    (arguments.output / "tasks.json").write_text(json.dumps({"comment": __doc__.splitlines()[0], "tasks": tasks}, indent=1))
    print(f"{len(tasks)} tasks", file=sys.stderr)


if __name__ == "__main__":
    main()
