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
import subprocess
import sys
from pathlib import Path

HERE = Path(__file__).resolve().parent
ROOT = HERE.parents[1]
sys.path.insert(0, str(ROOT / "benchmarks/pcbench"))
import run as pcbench  # noqa: E402

WORK = ROOT / "benchmarks/real/external/pcbench-work/newdrc"
STEM = "processed_v9_guide_v3"
CONNECTOR_PREFIXES = ("J", "P", "CN", "CON", "USB", "X")


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
            continue
        low, high = outline["minimum"], outline["maximum"]
        edges, rotations, fixed, hollow = [], [], [], []
        footprints = [f for f in description["footprints"] if f["reference"]]
        for footprint in footprints:
            body = footprint["body"]
            inside = [other for other in footprints if other is not footprint
                      and (other["side"] == footprint["side"] or footprint["through_hole"] or other["through_hole"])
                      and body[0] <= other["body"][0] and body[1] <= other["body"][1]
                      and other["body"][2] <= body[2] and other["body"][3] <= body[3]]
            if inside and any(net for _, net in footprint["pads"]):
                # It holds other parts: a shield's outline, a module over
                # parts. It stays, and parts may sit inside it.
                hollow.append(footprint["reference"])
        for footprint in description["footprints"]:
            if not footprint["reference"] or not any(net for _, net in footprint["pads"]):
                continue
            if footprint["reference"] in hollow:
                fixed.append(footprint["reference"])
                continue
            body = footprint["body"]
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
                      "unplace": True, "remove_outline": False,
                      "stack_at": [(low[0] + high[0]) / 2, (low[1] + high[1]) / 2],
                      "constraints": f"{name}.json", "router": {}})
    (arguments.output / "tasks.json").write_text(json.dumps({"comment": __doc__.splitlines()[0], "tasks": tasks}, indent=1))
    print(f"{len(tasks)} tasks", file=sys.stderr)


if __name__ == "__main__":
    main()
