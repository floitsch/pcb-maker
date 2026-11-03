#!/usr/bin/env python3
# Copyright (C) 2026 Toit contributors.
# Use of this source code is governed by an MIT-style license that can be
# found in the LICENSE file.
"""The first route of two placements of a board, alike.

    firstroute.py prepare --sweep build/nightly/2026-11-03/sweep --out DIR BOARD...
    firstroute.py run --out DIR --binary PATH [--slots 5]
    firstroute.py table --out DIR

Each placement (the designer's own board, and ours: the nightly's
`layout/placed` board in the designer's project directory) is laid out by
`layout-kicad-board` with every part fixed (`fixed_patterns: ["*"]`) and
`stop_after_first_route`, at the sweep's budget (scale 1): the race has one
placement, the first route gets the same budget on both sides, and its open
terminals (`first_unconnected_terminals`) are a function of the placement
alone. One run.py per (side, board) at --jobs 1 --threads 2, at most
`--slots` in flight, each with --expected-gb from the board's measured
layout peak (benchmarks/agent-tasks/estimates.json).
"""

import argparse
import json
import shutil
import subprocess
import sys
import time
from pathlib import Path

ROOT = Path(__file__).resolve().parents[2]
TASKS = ROOT / "benchmarks/agent-tasks/tasks/v7/tasks.json"
ESTIMATES = ROOT / "benchmarks/agent-tasks/estimates.json"
SIDES = ("designer", "ours")


def v7_tasks():
    return {task["name"]: task for task in json.loads(TASKS.read_text())["tasks"]}


def expected_gb(board):
    """The board's measured layout peak (estimates.json), in GB."""
    entry = json.loads(ESTIMATES.read_text())["boards"].get(board)
    if not entry or "layout" not in entry:
        raise SystemExit(f"{board}: no measured layout peak in {ESTIMATES}")
    return round(entry["layout"]["peak_rss_mb"] / 1024 + 0.05, 1)


def prepare(arguments):
    tasks = v7_tasks()
    out = Path(arguments.out)
    (out / "src").mkdir(parents=True, exist_ok=True)
    (out / "fixed-all.json").write_text(json.dumps({"version": 1}))
    for side in SIDES:
        entries = []
        for board in arguments.boards:
            task = tasks[board]
            directory = Path(task["directory"])
            if not directory.exists():
                directory = ROOT / "benchmarks/real/external/github" / board
            target = out / "src" / side / board
            if target.exists():
                shutil.rmtree(target)
            shutil.copytree(directory, target, ignore=shutil.ignore_patterns(".history", "*-backups"))
            if side == "ours":
                placed = Path(arguments.sweep) / board / "layout/placed" / f"{task['board_id']}.kicad_pcb"
                shutil.copyfile(placed, target / f"{task['board_id']}.kicad_pcb")
            entries.append({"name": board, "directory": str(target.resolve()), "board_id": task["board_id"],
                            "unplace": False, "constraints": "fixed-all.json", "router": task.get("router", {})})
        (out / f"tasks-{side}.json").write_text(json.dumps({"comment": "firstroute.py", "tasks": entries}, indent=1))
    print(f"prepared {len(arguments.boards)} boards x {len(SIDES)} sides in {out}")


def units(out, binary):
    result = []
    for side in SIDES:
        for task in json.loads((out / f"tasks-{side}.json").read_text())["tasks"]:
            board = task["name"]
            name = f"{side}-{board.split('__')[-1]}"
            command = [sys.executable, str(ROOT / "benchmarks/agent-tasks/run.py"), str(out / "runs" / name),
                       "--tasks", str(out / f"tasks-{side}.json"), "--only", board, "--binary", binary,
                       "--jobs", "1", "--threads", "2", "--expected-gb", str(expected_gb(board)),
                       "--placer", json.dumps({"fixed_patterns": ["*"]}),
                       "--layout", json.dumps({"stop_after_first_route": True})]
            result.append((name, command))
    return result


def run(arguments):
    out = Path(arguments.out)
    (out / "runs").mkdir(parents=True, exist_ok=True)
    queue = [u for u in units(out, arguments.binary) if not (out / "runs" / f"{u[0]}.done").exists()]
    running = {}
    while queue or running:
        for name, (process, log) in list(running.items()):
            if process.poll() is not None:
                log.close()
                (out / "runs" / f"{name}.done").write_text(f"{process.returncode}\n")
                print(time.strftime("%H:%M:%S"), "done", name, process.returncode, flush=True)
                del running[name]
        while queue and len(running) < arguments.slots:
            name, command = queue.pop(0)
            log = open(out / "runs" / f"{name}.out", "w")
            running[name] = (subprocess.Popen(command, cwd=ROOT, stdout=log, stderr=subprocess.STDOUT), log)
            print(time.strftime("%H:%M:%S"), "start", name, flush=True)
        time.sleep(15)
    print(time.strftime("%H:%M:%S"), "pool empty", flush=True)


def first_open(out):
    """{(side, board): first-route open terminals} of the finished runs."""
    result = {}
    for side in SIDES:
        for task in json.loads((out / f"tasks-{side}.json").read_text())["tasks"]:
            board = task["name"]
            report = out / "runs" / f"{side}-{board.split('__')[-1]}" / board / "layout/board-layout.json"
            if report.exists():
                data = json.loads(report.read_text())
                result[(side, board)] = data.get("first_unconnected_terminals")
    return result


def table(arguments):
    out = Path(arguments.out)
    values = first_open(out)
    boards = sorted({board for _, board in values})
    print("| board | designer | ours |\n| --- | ---: | ---: |")
    for board in boards:
        print(f"| {board} | {values.get(('designer', board))} | {values.get(('ours', board))} |")
    json.dump({f"{side}|{board}": value for (side, board), value in values.items()},
              open(out / "first-open.json", "w"), indent=1)


def main():
    parser = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    sub = parser.add_subparsers(dest="command", required=True)
    p = sub.add_parser("prepare")
    p.add_argument("--sweep", required=True)
    p.add_argument("--out", required=True)
    p.add_argument("boards", nargs="+")
    r = sub.add_parser("run")
    r.add_argument("--out", required=True)
    r.add_argument("--binary", required=True)
    r.add_argument("--slots", type=int, default=5)
    t = sub.add_parser("table")
    t.add_argument("--out", required=True)
    arguments = parser.parse_args()
    {"prepare": prepare, "run": run, "table": table}[arguments.command](arguments)


if __name__ == "__main__":
    main()
