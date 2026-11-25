#!/usr/bin/env python3
# Copyright (C) 2026 Florian Loitsch.
"""Runs the noise-source experiments' first routes (docs/experiments/noise-sources.md).

    experiments/noise-sources/grid.py run PLAN [--out DIR] [--binary B] [--parallel 3] [--only CELL...]
    experiments/noise-sources/grid.py list PLAN

PLAN is `e3` (boards x placer seeds x router seeds, `PCB_ROUTER_SEED`),
`e3-repeat` (one E3 cell run again, for byte determinism), `e2` (placebo
pairs: base, `PCB_PLACER_PLACEBO=draw|nudge`, each with the sequential
and the counter-based anneal random numbers, `PCB_PLACER_COUNTER_RANDOM`)
or `identity` (main's binary against this branch's with the switches off).

Every cell is one board's layout stopped after the first route, as the
placer ladder's stage 1 runs it (tasks v7, pinned rates, `--threads 2`,
through run.py and its memory ledger), into `<out>/<cell>`. A cell whose
results.json holds its row is skipped. At most `--parallel` cells run at
once; none starts while MemAvailable is under 16 GB or the one-minute load
above 20 (`--min-memory`, `--max-load`; checked once a minute), nor between 01:40 and 07:00 (the
nightly's 02:00 to 07:00, less a cell's length)."""

import argparse
import datetime
import json
import os
import subprocess
import sys
import time
from pathlib import Path

HERE = Path(__file__).resolve().parent
ROOT = HERE.parents[1]
RUN = ROOT / "benchmarks" / "agent-tasks" / "run.py"
TASKS = ROOT / "benchmarks" / "agent-tasks" / "tasks" / "v7" / "tasks.json"
MAIN = Path("/home/flo/programming/pcb-maker")
OUT = MAIN / "build" / "scratch" / "fund-noise" / "runs"
BINARY = MAIN / "build" / "scratch" / "fund-noise" / "bin" / "pcb-maker-noise"
MAIN_BINARY = MAIN / "build" / "bin" / "pcb-maker-nightly-2026-11-24-96e891b27162"

BOARDS = {
    "protoesp": "CRImier__MyKiCad__protoesp",
    "mini": "OpenDrone-hw__OpenESC-20x20__4in1-mini",
    "glasgow": "GlasgowEmbedded__glasgow__glasgow__revC3",
}
PLACER_SEEDS = (1, 101, 201, 301)
ROUTER_SEEDS = (1001, 1002, 1003, 1004)


def cells(plan):
    """(name, board, placer seed, environment, binary or None) of a plan."""
    out = []
    if plan == "e3":
        for short, board in BOARDS.items():
            for placer in PLACER_SEEDS:
                for router in ROUTER_SEEDS:
                    out.append((f"e3-{short}-p{placer}-r{router}", board, placer,
                                {"PCB_ROUTER_SEED": str(router)}, None))
    elif plan == "e3-repeat":
        out.append(("e3rep-protoesp-p101-r1002", BOARDS["protoesp"], 101,
                    {"PCB_ROUTER_SEED": "1002"}, None))
    elif plan == "e2":
        for short, board in BOARDS.items():
            for placer in PLACER_SEEDS:
                for rng in ("seq", "ctr"):
                    for placebo in ("base", "draw", "nudge"):
                        environment = {}
                        if rng == "ctr":
                            environment["PCB_PLACER_COUNTER_RANDOM"] = "1"
                        if placebo != "base":
                            environment["PCB_PLACER_PLACEBO"] = placebo
                        out.append((f"e2-{short}-p{placer}-{rng}-{placebo}", board, placer,
                                    environment, None))
    elif plan == "identity":
        out.append(("id-main-protoesp-p1", BOARDS["protoesp"], 1, {}, MAIN_BINARY))
        out.append(("id-branch-protoesp-p1", BOARDS["protoesp"], 1, {}, None))
    else:
        raise SystemExit(f"unknown plan {plan}")
    return out


def done(directory):
    results = directory / "results.json"
    if not results.exists():
        return False
    try:
        rows = json.loads(results.read_text())
    except json.JSONDecodeError:
        return False
    rows = rows if isinstance(rows, list) else rows.get("results", [])
    return any(row.get("first_unconnected_terminals") is not None for row in rows)


def machine_ready(max_load=20.0, min_memory=16.0):
    """Whether a run may start: memory, load and the nightly's hours."""
    now = datetime.datetime.now()
    # A cell takes up to a quarter of an hour: none starts from 01:40 on,
    # so none runs into the nightly's 02:00.
    if (now.hour == 1 and now.minute >= 40) or 2 <= now.hour < 7:
        return False, "the nightly's hours"
    available = 0
    for line in Path("/proc/meminfo").read_text().splitlines():
        if line.startswith("MemAvailable:"):
            available = int(line.split()[1]) / 1024 / 1024
    load = os.getloadavg()[0]
    if available < min_memory:
        return False, f"MemAvailable {available:.1f} GB"
    if load > max_load:
        return False, f"load {load:.1f}"
    return True, ""


def command(cell, out, binary):
    name, board, placer, _, own = cell
    return [sys.executable, str(RUN), str(out / name), "--tasks", str(TASKS), "--only", board,
            "--jobs", "1", "--threads", "2", "--binary", str(own or binary),
            "--placer", json.dumps({"tuning": {"seed": placer}}),
            "--layout", json.dumps({"stop_after_first_route": True})]


def run(arguments):
    plan = [cell for cell in cells(arguments.plan)
            if not arguments.only or cell[0] in arguments.only]
    out = arguments.out
    out.mkdir(parents=True, exist_ok=True)
    pending = [cell for cell in plan if not done(out / cell[0])]
    print(f"{len(plan)} cells, {len(pending)} to run", flush=True)
    running = {}
    while pending or running:
        for name, process in list(running.items()):
            if process.poll() is not None:
                print(f"{time.strftime('%H:%M:%S')} {name} exit {process.returncode}", flush=True)
                del running[name]
        if pending and len(running) < arguments.parallel:
            ready, why = machine_ready(arguments.max_load, arguments.min_memory)
            if ready:
                cell = pending.pop(0)
                environment = dict(os.environ, **cell[3])
                for key in ("PCB_ROUTER_SEED", "PCB_PLACER_PLACEBO", "PCB_PLACER_COUNTER_RANDOM"):
                    if key not in cell[3]:
                        environment.pop(key, None)
                log = open(out / f"{cell[0]}.log", "w")
                running[cell[0]] = subprocess.Popen(command(cell, out, arguments.binary), env=environment,
                                                    stdout=log, stderr=subprocess.STDOUT)
                print(f"{time.strftime('%H:%M:%S')} start {cell[0]} {cell[3]}", flush=True)
                # The load average needs a while to show a new run.
                time.sleep(20)
                continue
            print(f"{time.strftime('%H:%M:%S')} waiting: {why}", flush=True)
            time.sleep(60)
            continue
        time.sleep(10)


def main():
    parser = argparse.ArgumentParser()
    parser.add_argument("action", choices=("run", "list"))
    parser.add_argument("plan")
    parser.add_argument("--out", type=Path, default=OUT)
    parser.add_argument("--binary", type=Path, default=BINARY)
    parser.add_argument("--parallel", type=int, default=3)
    parser.add_argument("--only", nargs="*")
    parser.add_argument("--max-load", type=float, default=20.0,
                        help="no run starts above this one-minute load (the rules' 20)")
    parser.add_argument("--min-memory", type=float, default=16.0,
                        help="no run starts under this MemAvailable in GB (the rules' 16)")
    arguments = parser.parse_args()
    if arguments.action == "list":
        for cell in cells(arguments.plan):
            print(cell[0], cell[3], "done" if done(arguments.out / cell[0]) else "")
        return
    run(arguments)


if __name__ == "__main__":
    main()
