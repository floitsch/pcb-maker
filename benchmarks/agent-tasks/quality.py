#!/usr/bin/env python3
# Copyright (C) 2026 Toit contributors.
"""Scores the designer's board and pcb-maker's layout of every passing task.

    benchmarks/agent-tasks/quality.py <run-dir> [--binary PATH] [--tasks tasks.json]

Runs `score-kicad-board` on each task's source board (the designer's
placement and routing) and on its layout result, and prints per-measure
summaries side by side: the question is whether ours is at least as good as
the human's, measure by measure."""

import argparse
import json
import statistics
import subprocess
from pathlib import Path

HERE = Path(__file__).resolve().parent
ROOT = HERE.parents[1]


def score(binary, directory, board_id):
    process = subprocess.run([str(binary), "score-kicad-board", str(directory), board_id],
                             capture_output=True, text=True)
    if process.returncode != 0:
        return None
    return json.loads(process.stdout)


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("run", type=Path)
    parser.add_argument("--binary", type=Path, default=ROOT / "target/release/pcb-maker")
    parser.add_argument("--tasks", type=Path, default=ROOT / "build/agent-pcbench/tasks.json")
    arguments = parser.parse_args()
    tasks = {task["name"]: task for task in json.loads(arguments.tasks.read_text())["tasks"]}
    results = json.loads((arguments.run / "results.json").read_text())
    pairs = []
    for row in results:
        if not row["pass"] or row["name"] not in tasks:
            continue
        task = tasks[row["name"]]
        human = score(arguments.binary, Path(task["directory"]), task["board_id"])
        ours = score(arguments.binary, arguments.run / row["name"] / "layout/result", task["board_id"])
        if human and ours:
            pairs.append((row["name"], human, ours))
    print(f"{len(pairs)} boards scored\n")

    def collect(path, pick):
        values = []
        for _, human, ours in pairs:
            h, o = pick(human, path), pick(ours, path)
            if h is not None and o is not None:
                values.append((h, o))
        return values

    def get(report, path):
        value = report
        for key in path:
            value = value.get(key) if isinstance(value, dict) else None
            if value is None:
                return None
        return value

    rows = [
        ("decoupling: median cap-to-pin distance (mm)", ("decoupling", "capacitor_median_mm"), get, "lower"),
        ("decoupling: caps within 3 mm of a supply pin", ("decoupling", "capacitors_within_3mm"), get, "higher"),
        ("decoupling: median pin-to-cap distance (mm)", ("decoupling", "median_mm"), get, "lower"),
        ("decoupling: supply pins with a cap within 3 mm", ("decoupling", "within_3mm"), get, "higher"),
        ("decoupling: within 5 mm", ("decoupling", "within_5mm"), get, "higher"),
        ("crystal: longest path (mm)", ("crystals",),
         lambda r, p: max((c["longest_mm"] for c in r["crystals"] if c["longest_mm"] is not None), default=None), "lower"),
        ("inductor to crystal/antenna: closest (mm)", ("separation", "closest_mm"), get, "higher"),
        ("orientation consistency", ("assembly", "orientation_consistency"), get, "higher"),
        ("parts < 2.5 mm from the edge", ("assembly", "near_edge"), lambda r, p: len(get(r, p)), "lower"),
        ("connectors not facing out", ("connectors", "not_facing_out"), lambda r, p: len(get(r, p)), "lower"),
        ("vias in pads", ("manufacturing", "vias_in_pads"), get, "lower"),
        ("tombstone-risk parts", ("manufacturing", "tombstone_risk"), lambda r, p: len(get(r, p)), "lower"),
        ("ground pour cut by other tracks (mm)", ("planes",),
         lambda r, p: sum(plane["cut_by_mm"] for plane in r["planes"]) if r["planes"] else None, "lower"),
        ("vias", ("economy", "vias"), get, "lower"),
        ("track length (mm)", ("economy", "track_length_mm"), get, "lower"),
        ("smallest via drill (mm)", ("economy", "smallest_via_drill_mm"), get, "higher"),
        ("narrowest track (mm)", ("economy", "narrowest_track_mm"), get, "higher"),
    ]
    print(f"| Measure | boards | designer (median) | ours (median) | ours no worse |")
    print(f"| --- | --- | --- | --- | --- |")
    for label, path, pick, better in rows:
        values = collect(path, pick)
        if not values:
            continue
        human = statistics.median(h for h, _ in values)
        ours = statistics.median(o for _, o in values)
        no_worse = sum(1 for h, o in values if (o <= h + 1e-9 if better == "lower" else o >= h - 1e-9))
        print(f"| {label} | {len(values)} | {human:g} | {ours:g} | {no_worse}/{len(values)} |")


if __name__ == "__main__":
    main()
