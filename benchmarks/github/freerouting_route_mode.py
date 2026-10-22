#!/usr/bin/env python3
# Copyright (C) 2026 Toit contributors.
"""Freerouting on the placements of a route-mode run, judged as run.py
judges ours.

    benchmarks/github/freerouting_route_mode.py <output> --ours build/fr2-route-x1367 \\
        --route-from build/github-layout-v17 --binary build/bin/pcb-maker-x1367 --jobs 8 --threads 2

Each board is the placement `run.py --route-from` routed
(`<route-from>/<task>/layout/placed`: the layout's parts, keepouts and
designer pours, no tracks), as it is: KiCad's DSN export makes the pours
planes, which Freerouting reaches with vias, and KiCad's DRC refills them.
Freerouting runs headless (freerouting.py's pipeline: DSN export, the class
rules raised to the board minimums, its passes, the session import) with
its own job timeout at the wall seconds our row of the board took
(`--ours`; `--wall` gives one for every board instead): a best effort
within the same wall, its passes until it stops or the wall. The result is
judged with run.py's own functions: KiCad's unconnected items and copper
errors less the designer's per type (`pass`), and the strict rule against
the designer's stripped board at the same place (`pass_strict`). Rows go to
stdout as JSON lines and to <output>/results.json."""

import argparse
import fcntl
import importlib.util
import json
import os
import sys
import threading
import time
from pathlib import Path

ROOT = Path(__file__).resolve().parents[2]
sys.path.insert(0, str(ROOT / "benchmarks/github"))
import freerouting  # noqa: E402

sys.path.insert(1, str(ROOT / "benchmarks/agent-tasks"))
_spec = importlib.util.spec_from_file_location("agent_run", ROOT / "benchmarks/agent-tasks/run.py")
agent_run = importlib.util.module_from_spec(_spec)
_spec.loader.exec_module(agent_run)


def our_rows(path):
    """The rows of a run.py output directory (results.json) or log, by name."""
    if path is None:
        return {}
    path = Path(path)
    if path.is_dir():
        rows = json.loads((path / "results.json").read_text())
    else:
        rows = [json.loads(line) for line in path.read_text().splitlines() if line.startswith("{")]
    return {row["name"]: row for row in rows}


def route_board(task, ours, arguments, config):
    name, board_id = task["name"], task["board_id"]
    work = arguments.output / name
    if work.exists():
        import shutil
        shutil.rmtree(work)
    work.mkdir(parents=True)
    placed = arguments.route_from / name / "layout" / "placed"
    row = {"name": name, "mode": "freerouting-route", "route_from": str(placed), "threads": arguments.threads}
    if not (placed / f"{board_id}.kicad_pcb").exists():
        row.update(error=f"no placement in {placed}", **{"pass": False, "pass_strict": False})
        return row
    if arguments.wall is not None:
        wall = arguments.wall
    elif ours is not None and ours.get("seconds"):
        wall = ours["seconds"]
    else:
        wall = arguments.default_wall
    row["ours_seconds"] = ours.get("seconds") if ours else None
    job_timeout = max(10, round(wall))
    directory = agent_run.task_directory(task)
    if ours is not None and "designer_findings" in ours:
        allowed = ours["designer_findings"]
    elif task.get("designer_budget"):
        allowed = agent_run.designer_findings(directory, board_id, work, arguments.binary)
    else:
        allowed = {}
    baseline = agent_run.StrictBaseline(directory, board_id, work, arguments.binary, bool(task.get("designer_budget")))
    started = time.monotonic()
    load_before = os.getloadavg()[0]
    entry = freerouting.freeroute(
        placed, board_id, work / "freerouting", config, job_timeout + arguments.grace, arguments.java_heap,
        arguments.slot, lambda: freerouting.wait_for_memory(arguments.min_free_gb, name + " (Freerouting)"),
        arguments.binary, threads=arguments.threads, job_timeout=job_timeout, refill=True)
    row["seconds"] = round(time.monotonic() - started, 1)
    row["load"] = [round(load_before, 1), round(os.getloadavg()[0], 1)]
    row["freerouting"] = entry
    # The router's CPU and the pipeline's (DSN export, rules, session
    # import, KiCad's DRC), as ours counts its own KiCad checks.
    row["cpu_seconds"] = round((entry.get("cpu_seconds") or 0) + (entry.get("pipeline_cpu_seconds") or 0), 1)
    for key in ("vias", "length_mm", "status", "job_timed_out", "passes", "log_unrouted"):
        if key in entry:
            row[key] = entry[key]
    result = work / "freerouting" / "result"
    copper = agent_run.native(result, allowed)
    if allowed:
        row["designer_findings"] = allowed
    row["native"] = copper
    row["pass"] = bool(copper is not None and copper["unconnected"] == 0 and not copper["errors"])
    agent_run.strict_row(row, agent_run.strict_native(result, baseline))
    if copper is None:
        row["error"] = entry.get("status", "no result")
    return row


def main():
    parser = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    parser.add_argument("output", type=Path)
    parser.add_argument("--route-from", type=Path, required=True)
    parser.add_argument("--ours", type=Path, help="run.py's route-mode output (or log): each board's wall")
    parser.add_argument("--wall", type=float, help="Freerouting's job timeout for every board (seconds)")
    parser.add_argument("--default-wall", type=float, default=1500.0, help="for boards without a row of ours")
    parser.add_argument("--grace", type=float, default=300.0,
                        help="beyond the job timeout before the process is killed (DSN read, session write)")
    parser.add_argument("--tasks", type=Path, default=ROOT / "benchmarks/agent-tasks/tasks/v7/tasks.json")
    parser.add_argument("--only", nargs="*")
    parser.add_argument("--binary", type=Path, required=True, help="for KiCad's DRC (drc-kicad-board) and the baseline")
    parser.add_argument("--freerouting", type=Path, default=ROOT / "benchmarks/corpus/freerouting.json")
    parser.add_argument("--threads", type=int, default=1, help="Freerouting's -mt")
    parser.add_argument("--jobs", type=int, default=1)
    parser.add_argument("--java-heap", default="4g")
    parser.add_argument("--min-free-gb", type=float, default=12.0)
    parser.add_argument("--resume", action="store_true")
    parser.add_argument("--lock", type=Path,
                        help="take this flock (build/quick-tier.lock) per batch of --jobs boards and release it "
                             "between batches, so that other runs waiting for it get their turn")
    arguments = parser.parse_args()
    arguments.output.mkdir(parents=True, exist_ok=True)
    config = json.loads(arguments.freerouting.read_text())
    tasks = json.loads(arguments.tasks.read_text())
    tasks = tasks["tasks"] if isinstance(tasks, dict) else tasks
    if arguments.only:
        tasks = [task for task in tasks if task["name"] in arguments.only]
    ours = our_rows(arguments.ours)
    order = {task["name"]: index for index, task in enumerate(tasks)}
    rows = []
    results = arguments.output / "results.json"
    if arguments.resume and results.exists():
        rows = [row for row in json.loads(results.read_text()) if row["name"] in order]
        done = {row["name"] for row in rows}
        tasks = [task for task in tasks if task["name"] not in done]
    arguments.slot = None
    lock = threading.Lock()

    def finish(row):
        with lock:
            rows[:] = [r for r in rows if r["name"] != row["name"]] + [row]
            rows.sort(key=lambda r: order[r["name"]])
            print(json.dumps(row), flush=True)
            results.write_text(json.dumps(rows, indent=1))

    def jobs():
        try:
            return max(0, int((arguments.output / "jobs").read_text().strip()))
        except (OSError, ValueError):
            return max(1, arguments.jobs)

    # The longest walls first: the sweep is as long as its longest board.
    tasks.sort(key=lambda task: -((ours.get(task["name"]) or {}).get("seconds") or arguments.default_wall))
    if arguments.lock is not None:
        # One batch of boards per hold of the lock.
        while tasks:
            batch, tasks = tasks[:max(1, jobs())], tasks[max(1, jobs()):]
            with open(arguments.lock, "a") as lock_file:
                fcntl.flock(lock_file, fcntl.LOCK_EX)
                print(f"{time.strftime('%H:%M:%S')} lock held for {len(batch)} boards", file=sys.stderr, flush=True)
                threads = []
                for task in batch:
                    freerouting.wait_for_memory(arguments.min_free_gb, task["name"])
                    thread = threading.Thread(
                        target=lambda t=task: finish(route_board(t, ours.get(t["name"]), arguments, config)))
                    thread.start()
                    threads.append(thread)
                    print(f"{time.strftime('%H:%M:%S')} started {task['name']} (load {os.getloadavg()[0]:.1f})",
                          file=sys.stderr, flush=True)
                    time.sleep(20)
                for thread in threads:
                    thread.join()
                fcntl.flock(lock_file, fcntl.LOCK_UN)
    running = []
    for task in tasks:
        while True:
            running = [thread for thread in running if thread.is_alive()]
            if len(running) < jobs():
                break
            time.sleep(5)
        freerouting.wait_for_memory(arguments.min_free_gb, task["name"])
        thread = threading.Thread(target=lambda t=task: finish(route_board(t, ours.get(t["name"]), arguments, config)))
        thread.start()
        running.append(thread)
        print(f"{time.strftime('%H:%M:%S')} started {task['name']} ({len(running)} running, "
              f"load {os.getloadavg()[0]:.1f})", file=sys.stderr, flush=True)
        if jobs() > 1:
            time.sleep(20)
    for thread in running:
        thread.join()
    passing = sum(1 for row in rows if row.get("pass"))
    strict = sum(1 for row in rows if row.get("pass_strict"))
    unconnected = sum((row.get("native") or {}).get("unconnected", 0) for row in rows)
    print(f"{passing}/{len(rows)} boards pass; strict rule: {strict} pass; KiCad unconnected {unconnected}",
          file=sys.stderr, flush=True)


if __name__ == "__main__":
    main()
