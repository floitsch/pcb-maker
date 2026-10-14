#!/usr/bin/env python3
# Copyright (C) 2026 Toit contributors.
# Use of this source code is governed by an MIT-style license that can be
# found in the LICENSE file.
"""Where the placement's wirelength goes: prepares tasks as run.py does
(tracks stripped, movable parts stacked, the task's constraints), places
them with `place-kicad-board` (16 seeds, the layout's 600 s of work) and
`PCB_PLACER_TRACE`, with the designer's board as `PCB_PLACER_DESIGNER`.

    measure.py <output-dir> --binary <pcb-maker> [--tier quick] [--only NAME ...] [--jobs 4]

Writes <output-dir>/<task>/trace.json and place.log; `analyse.py` reads them.
"""

import argparse
import concurrent.futures
import importlib.util
import json
import os
import re
import shutil
import subprocess
import sys
from pathlib import Path

ROOT = Path(__file__).resolve().parents[2]
MAIN = Path("/home/flo/programming/pcb-maker")
spec = importlib.util.spec_from_file_location("agent_run", ROOT / "benchmarks/agent-tasks/run.py")
agent_run = importlib.util.module_from_spec(spec)
spec.loader.exec_module(agent_run)


def prepare(task, tasks_path, work, binary):
    directory = Path(task["directory"])
    if work.exists():
        shutil.rmtree(work)
    source = work / "source"
    shutil.copytree(directory, source, ignore=shutil.ignore_patterns(
        ".history", "*-backups", "Gerbers", "packages3D", "*.pdf", "*.zip", "fp-info-cache"))
    board = source / f"{task['board_id']}.kicad_pcb"
    subprocess.run([str(binary), "strip-kicad-tracks", str(board), str(board)], capture_output=True, check=True)
    constraints = json.loads((tasks_path.parent / task["constraints"]).read_text())
    if task.get("unplace"):
        outline = constraints.get("outline", {})
        point = task.get("stack_at") or [outline.get("x", 0) + outline.get("width", 40) / 2,
                                          outline.get("y", 0) + outline.get("height", 40) / 2]
        netless = agent_run.unplace(board, point, task.get("remove_outline", False), set(constraints.get("fixed", [])))
        escaped = [re.sub(r"([*?\\])", r"\\\1", r) for r in netless]
        constraints["fixed"] = constraints.get("fixed", []) + [r for r in escaped if r not in constraints.get("fixed", [])]
    (source / "constraints.json").write_text(json.dumps(constraints))
    return source


def run(task, tasks_path, output, binary, placer):
    work = output / task["name"]
    source = prepare(task, tasks_path, work, binary)
    config = {"constraints": "constraints.json", "placement_seeds": 16,
              **task.get("placer", {}), **task.get("layout", {}).get("placer", {}), **placer}
    (work / "placer.json").write_text(json.dumps(config))
    env = dict(os.environ, PCB_PLACER_WORK_SECONDS="600", PCB_PLACER_TRACE=str(work / "trace.json"),
               PCB_PLACER_DESIGNER=str(Path(task["directory"]) / f"{task['board_id']}.kicad_pcb"))
    result = subprocess.run([str(binary), "place-kicad-board", str(source), task["board_id"], str(work / "placed"),
                             str(work / "placer.json")], capture_output=True, text=True, env=env)
    (work / "place.log").write_text(result.stdout + result.stderr)
    shutil.rmtree(source, ignore_errors=True)
    return task["name"], result.returncode, (result.stdout.strip().splitlines() or [""])[-1][:200]


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("output", type=Path)
    parser.add_argument("--binary", type=Path, required=True)
    parser.add_argument("--tasks", type=Path, default=MAIN / "build/github-tasks-v6/tasks.json")
    parser.add_argument("--tier", action="store_true", help="the quick tier's boards")
    parser.add_argument("--only", nargs="*", default=[])
    parser.add_argument("--jobs", type=int, default=4)
    parser.add_argument("--placer", type=json.loads, default={})
    arguments = parser.parse_args()
    arguments.output.mkdir(parents=True, exist_ok=True)
    names = set(arguments.only)
    if arguments.tier:
        names |= set(json.loads((ROOT / "benchmarks/agent-tasks/quick.json").read_text())["boards"])
    tasks = [task for task in json.loads(arguments.tasks.read_text())["tasks"] if task["name"] in names]
    with concurrent.futures.ThreadPoolExecutor(arguments.jobs) as pool:
        futures = [pool.submit(run, task, arguments.tasks, arguments.output, arguments.binary, arguments.placer)
                   for task in tasks]
        for future in concurrent.futures.as_completed(futures):
            print(*future.result(), flush=True)


if __name__ == "__main__":
    sys.exit(main())
