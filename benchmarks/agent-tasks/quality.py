#!/usr/bin/env python3
# Copyright (C) 2026 Toit contributors.
"""Scores the designer's board and pcb-maker's layout of every passing task.

    benchmarks/agent-tasks/quality.py <run-dir> [--binary PATH] [--tasks tasks.json] [--all]
    benchmarks/agent-tasks/quality.py --designer-numbers [--binary PATH] [--tasks tasks.json]

Runs `score-kicad-board` on each task's source board (the designer's
placement and routing) and on its layout result, and prints per-measure
summaries side by side: the question is whether ours is at least as good as
the human's, measure by measure. Rows that carry the scores already
(run.py --quality writes them into each row's `quality`) are not scored
again. `--all` takes every row with a result, not only the passing ones.

`--designer-numbers` writes designer-numbers.json next to this script: per task of
the list, the designer's vias, track length and ground pour cut (the
measures compare.py normalises by), with the board's SHA-256 so that a
number is only used for the same board."""

import argparse
import hashlib
import json
import statistics
import subprocess
from pathlib import Path

HERE = Path(__file__).resolve().parent
ROOT = HERE.parents[1]
DESIGNER_NUMBERS = HERE / "designer-numbers.json"
DEFAULT_TASKS = HERE / "tasks/v7/tasks.json"


def score(binary, directory, board_id):
    process = subprocess.run([str(binary), "score-kicad-board", str(directory), board_id],
                             capture_output=True, text=True)
    if process.returncode != 0:
        return None
    try:
        return json.loads(process.stdout)
    except ValueError:
        return None


def get(report, path):
    value = report
    for key in path:
        value = value.get(key) if isinstance(value, dict) else None
        if value is None:
            return None
    return value


def ground_cut_mm(report, path=None):
    """The ground pours' integrity: the length of other nets' tracks that cut
    them, summed over the pours (score-kicad-board's planes); None when the
    board has no pour."""
    planes = report.get("planes") or []
    return round(sum(plane["cut_by_mm"] for plane in planes), 2) if planes else None


# (key, label, how to read it from a score, which way is better).
MEASURES = [
    ("decoupling_cap_median_mm", "decoupling: median cap-to-pin distance (mm)",
     lambda r: get(r, ("decoupling", "capacitor_median_mm")), "lower"),
    ("decoupling_caps_within_3mm", "decoupling: caps within 3 mm of a supply pin",
     lambda r: get(r, ("decoupling", "capacitors_within_3mm")), "higher"),
    ("decoupling_pin_median_mm", "decoupling: median pin-to-cap distance (mm)",
     lambda r: get(r, ("decoupling", "median_mm")), "lower"),
    ("decoupling_pins_within_3mm", "decoupling: supply pins with a cap within 3 mm",
     lambda r: get(r, ("decoupling", "within_3mm")), "higher"),
    ("decoupling_within_5mm", "decoupling: within 5 mm", lambda r: get(r, ("decoupling", "within_5mm")), "higher"),
    ("crystal_longest_mm", "crystal: longest path (mm)",
     lambda r: max((c["longest_mm"] for c in r.get("crystals") or [] if c["longest_mm"] is not None), default=None),
     "lower"),
    ("inductor_separation_mm", "inductor to crystal/antenna: closest (mm)",
     lambda r: get(r, ("separation", "closest_mm")), "higher"),
    ("switch_inductor_mm", "switch inductor to its IC pin (mm)",
     lambda r: max((d for _, d in r.get("switch_inductors") or [] if d is not None), default=None), "lower"),
    ("esd_mm", "ESD protector to its connector pin (mm)",
     lambda r: max((d for _, d in r.get("esd") or [] if d is not None), default=None), "lower"),
    ("orientation_consistency", "orientation consistency",
     lambda r: get(r, ("assembly", "orientation_consistency")), "higher"),
    ("near_edge", "parts < 2.5 mm from the edge",
     lambda r: len(get(r, ("assembly", "near_edge"))) if get(r, ("assembly", "near_edge")) is not None else None,
     "lower"),
    ("connectors_not_facing_out", "connectors not facing out",
     lambda r: len(get(r, ("connectors", "not_facing_out")))
     if get(r, ("connectors", "not_facing_out")) is not None else None, "lower"),
    ("vias_in_pads", "vias in pads", lambda r: get(r, ("manufacturing", "vias_in_pads")), "lower"),
    ("tombstone_risk", "tombstone-risk parts",
     lambda r: len(get(r, ("manufacturing", "tombstone_risk")))
     if get(r, ("manufacturing", "tombstone_risk")) is not None else None, "lower"),
    ("ground_cut_mm", "ground pour cut by other tracks (mm)", ground_cut_mm, "lower"),
    ("vias", "vias", lambda r: get(r, ("economy", "vias")), "lower"),
    ("track_length_mm", "track length (mm)", lambda r: get(r, ("economy", "track_length_mm")), "lower"),
    ("smallest_via_drill_mm", "smallest via drill (mm)",
     lambda r: get(r, ("economy", "smallest_via_drill_mm")), "higher"),
    ("narrowest_track_mm", "narrowest track (mm)", lambda r: get(r, ("economy", "narrowest_track_mm")), "higher"),
]


def measures(report):
    """The MEASURES of a score-kicad-board report, by key (None where the
    board has nothing to measure)."""
    values = {}
    for key, _, pick, _ in MEASURES:
        try:
            values[key] = pick(report)
        except (KeyError, TypeError, ValueError):
            values[key] = None
    return values


def file_sha256(path):
    digest = hashlib.sha256()
    with open(path, "rb") as stream:
        for chunk in iter(lambda: stream.read(1 << 20), b""):
            digest.update(chunk)
    return digest.hexdigest()


def task_directory(task):
    import run  # the runner's resolution of a task's board directory
    return run.task_directory(task)


def designer_numbers(binary, tasks_file):
    """designer-numbers.json entries: per task, the designer board's SHA-256 and
    the measures compare.py normalises by."""
    numbers = {}
    for task in json.loads(Path(tasks_file).read_text())["tasks"]:
        directory = task_directory(task)
        board = directory / f"{task['board_id']}.kicad_pcb"
        if not board.is_file():
            continue
        report = score(binary, directory, task["board_id"])
        if report is None:
            continue
        values = measures(report)
        numbers[task["name"]] = {"board_sha256": file_sha256(board), "vias": values["vias"],
                                 "track_length_mm": values["track_length_mm"],
                                 "ground_cut_mm": values["ground_cut_mm"]}
    return numbers


def main():
    import sys
    sys.path.insert(0, str(HERE))
    parser = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    parser.add_argument("run", type=Path, nargs="?")
    parser.add_argument("--binary", type=Path, default=ROOT / "target/release/pcb-maker")
    parser.add_argument("--tasks", type=Path, default=DEFAULT_TASKS)
    parser.add_argument("--all", action="store_true", help="every row with a result, not only the passing ones")
    parser.add_argument("--designer-numbers", action="store_true",
                        help=f"write {DESIGNER_NUMBERS.name}: the designer's numbers per task of --tasks")
    arguments = parser.parse_args()
    if arguments.designer_numbers:
        numbers = designer_numbers(arguments.binary, arguments.tasks)
        try:
            label = str(arguments.tasks.resolve().relative_to(ROOT))
        except ValueError:
            label = str(arguments.tasks)
        DESIGNER_NUMBERS.write_text(json.dumps({
            "comment": "The designer's boards scored with score-kicad-board (quality.py --designer-numbers): "
                       "vias, track length and the ground pours' cut by other tracks (null: no pour), with "
                       "the board's SHA-256. compare.py normalises each row by these.",
            "tasks": label, "boards": numbers}, indent=1, sort_keys=True) + "\n")
        print(f"{len(numbers)} boards in {DESIGNER_NUMBERS}")
        return
    if arguments.run is None:
        parser.error("a run directory, or --designer-numbers")
    tasks = {task["name"]: task for task in json.loads(arguments.tasks.read_text())["tasks"]}
    results = json.loads((arguments.run / "results.json").read_text())
    pairs = []
    for row in results:
        if row["name"] not in tasks or not (row["pass"] or (arguments.all and row.get("native") is not None)):
            continue
        if row.get("quality"):
            human, ours = row["quality"].get("designer"), row["quality"].get("ours")
        else:
            task = tasks[row["name"]]
            human = score(arguments.binary, task_directory(task), task["board_id"])
            result = arguments.run / row["name"] / ("route" if row.get("mode") == "route" else "layout/result")
            ours = score(arguments.binary, result, task["board_id"])
            human, ours = (measures(human) if human else None), (measures(ours) if ours else None)
        if human and ours:
            pairs.append((row["name"], human, ours))
    print(f"{len(pairs)} boards scored\n")
    print("| Measure | boards | designer (median) | ours (median) | ours no worse |")
    print("| --- | --- | --- | --- | --- |")
    for key, label, _, better in MEASURES:
        values = [(h[key], o[key]) for _, h, o in pairs if h.get(key) is not None and o.get(key) is not None]
        if not values:
            continue
        human = statistics.median(h for h, _ in values)
        ours = statistics.median(o for _, o in values)
        no_worse = sum(1 for h, o in values if (o <= h + 1e-9 if better == "lower" else o >= h - 1e-9))
        print(f"| {label} | {len(values)} | {human:g} | {ours:g} | {no_worse}/{len(values)} |")


if __name__ == "__main__":
    main()
