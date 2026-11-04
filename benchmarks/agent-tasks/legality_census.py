#!/usr/bin/env python3
# Copyright (C) 2026 Toit contributors.
"""The legality census (docs/placer.md, "The legality model against KiCad"):
every task's designer placement judged by the placer's model at each
relaxation level (`pcb-maker legality-kicad-board`), beside KiCad's DRC of
the designer's board (`drc-kicad-board`, through the DRC cache).

  legality_census.py <out-dir> --binary build/bin/pcb-maker-xNNNN \
      [--sources build/github-layout-v20] [--tasks .../tasks.json] [--only NAME ...]

`--sources` is a run.py output tree whose `<task>/source` holds the tasks'
prepared boards and constraints (what the placer lowers); the designer's
poses come from the task's own directory. Writes `<out-dir>/<task>.json`
(the tool's report), `<out-dir>/drc/<task>.json` and `summary.json`, and
prints the per-board table and the causes summed over the boards.
"""

import argparse
import collections
import json
import pathlib
import subprocess
import sys

ROOT = pathlib.Path(__file__).resolve().parents[2]


def run_board(binary, task, sources, out):
    name = task["name"]
    report = out / f"{name}.json"
    drc = out / "drc" / f"{name}.json"
    designer = pathlib.Path(task["directory"]) / f"{task['board_id']}.kicad_pcb"
    if not drc.exists():
        drc.parent.mkdir(parents=True, exist_ok=True)
        subprocess.run([str(binary), "drc-kicad-board", task["directory"], task["board_id"], str(drc)],
                       stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL, timeout=3600)
    source = sources / name / "source"
    if not source.exists():
        source = pathlib.Path(task["directory"])
    command = [str(binary), "legality-kicad-board", str(source), task["board_id"], str(report),
               "--poses", str(designer)]
    if drc.exists():
        command += ["--drc", str(drc)]
    done = subprocess.run(command, capture_output=True, text=True, timeout=3600)
    if done.returncode != 0:
        print(f"{name}: {done.stderr.strip()[:300]}", file=sys.stderr)
        return None
    return json.loads(report.read_text())


def tightest(report):
    return report["levels"][-1]


def zero_halo(report):
    """The first level without halos at the least spacing (the ablation's
    'tightest model')."""
    for level in report["levels"]:
        if level["halo_scale"] == 0 and level["small_halo_scale"] == 0 and \
                abs(level["spacing_mm"] - report["rules"]["clearance_mm"]) < 1e-9:
            return level
    return report["levels"][-1]


def main():
    parser = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    parser.add_argument("out")
    parser.add_argument("--binary", required=True)
    parser.add_argument("--sources", default=str(ROOT / "build/github-layout-v20"))
    parser.add_argument("--tasks", default=str(ROOT / "benchmarks/agent-tasks/tasks/v7/tasks.json"))
    parser.add_argument("--only", nargs="*")
    arguments = parser.parse_args()
    out = pathlib.Path(arguments.out)
    out.mkdir(parents=True, exist_ok=True)
    tasks = json.loads(pathlib.Path(arguments.tasks).read_text())["tasks"]
    if arguments.only:
        tasks = [task for task in tasks if task["name"] in arguments.only]
    rows = []
    sums = {"loose": collections.Counter(), "zero": collections.Counter(), "tight": collections.Counter()}
    stricter = {"zero": collections.Counter(), "tight": collections.Counter()}
    for task in tasks:
        report = run_board(arguments.binary, task, pathlib.Path(arguments.sources), out)
        if report is None:
            continue
        loose, zero, tight = report["levels"][0], zero_halo(report), tightest(report)
        drc = report.get("kicad") or {}
        row = {
            "task": task["name"], "board_id": task["board_id"], "movable": report["movable"],
            "courtyards_overlap": report["rules"]["courtyards_overlap"],
            "tight_bodies": report["rules"]["tight_bodies"],
            "clearance_mm": report["rules"]["clearance_mm"],
            "illegal": [level["illegal"] for level in report["levels"]],
            "loose": loose["illegal"], "zero": zero["illegal"], "tight": tight["illegal"],
            "zero_agrees": zero.get("kicad_agrees"), "tight_agrees": tight.get("kicad_agrees"),
            "designer_placement_errors": sum(drc.get("placement_errors", {}).values()),
            "designer_errors": drc.get("placement_errors", {}),
        }
        rows.append(row)
        sums["loose"].update(loose["causes"])
        sums["zero"].update(zero["causes"])
        sums["tight"].update(tight["causes"])
        stricter["zero"].update(zero.get("stricter_causes", {}))
        stricter["tight"].update(tight.get("stricter_causes", {}))
    summary = {
        "boards": len(rows), "rows": rows,
        "movable": sum(row["movable"] for row in rows),
        "illegal": {key: sum(row[key] for row in rows) for key in ("loose", "zero", "tight")},
        "kicad_agrees": {key: sum(row[f"{key}_agrees"] or 0 for row in rows) for key in ("zero", "tight")},
        "causes": {key: dict(counter.most_common()) for key, counter in sums.items()},
        "stricter_causes": {key: dict(counter.most_common()) for key, counter in stricter.items()},
    }
    (out / "summary.json").write_text(json.dumps(summary, indent=1))
    print("| board | movable | illegal loosest / zero halo / tightest | with a KiCad error zero / tightest | designer's placement errors |")
    print("| --- | ---: | ---: | ---: | ---: |")
    for row in sorted(rows, key=lambda row: -row["zero"]):
        print(f"| {row['task']} | {row['movable']} | {row['loose']} / {row['zero']} / {row['tight']} | "
              f"{row['zero_agrees']} / {row['tight_agrees']} | {row['designer_placement_errors']} |")
    print(json.dumps({key: summary[key] for key in ("boards", "movable", "illegal", "kicad_agrees")}))
    for key in ("zero", "tight"):
        print(f"causes at {key}:", json.dumps(summary["causes"][key], indent=1))
        print(f"without a KiCad error at {key}:", json.dumps(summary["stricter_causes"][key], indent=1))


if __name__ == "__main__":
    main()
