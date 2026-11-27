#!/usr/bin/env python3
# Copyright (C) 2026 Florian Loitsch.
"""Runs the race-seeds ground-truth matrix (docs/experiments/race-seeds.md).

    experiments/race-seeds/matrix.py run [--out DIR] [--binary B] [--parallel 6] [--only CELL...]
    experiments/race-seeds/matrix.py list

Per board (placer seed 1, so one pool of candidate placements): every
placement the race probes (`PCB_RACE_KEEP=<rank>`, the hook of the branch's
experiment binary) routed to the end as a full layout under each router
seed (`PCB_ROUTER_SEED`). A cell is one layout through run.py (tasks v7,
the sweep's settings, `--threads 2`) into `<out>/<board>-k<rank>-r<seed>`.
Each cell's report also holds every placement's probe under that router
seed. A cell whose results.json holds its row is skipped; a rank the
board's race does not have is skipped once a finished cell shows it.

No cell starts while MemAvailable is under 16 GB or the one-minute load
above 30, nor from 01:20 until the nightly's summary.md of that day exists
(at most until 10:00)."""

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
MAIN = Path("/home/flo/programming/pcb-maker")
RUN = ROOT / "benchmarks" / "agent-tasks" / "run.py"
TASKS = ROOT / "benchmarks" / "agent-tasks" / "tasks" / "v7" / "tasks.json"
OUT = MAIN / "build" / "scratch" / "race-seeds" / "matrix"
BINARY = MAIN / "build" / "bin" / "pcb-maker-race-seeds"

# Fixed before any result: the noise study's three boards, and five failing
# boards of the 2026-11-26 sweep with 1 to 20 unconnected items and three
# probes, one per repository, the most open first (ties by name).
BOARDS = {
    "protoesp": "CRImier__MyKiCad__protoesp",
    "mini": "OpenDrone-hw__OpenESC-20x20__4in1-mini",
    "glasgow": "GlasgowEmbedded__glasgow__glasgow__revC3",
    "eurorack": "apfaudio__eurorack-pmod__eurorack-pmod-pcb",
    "rp2040": "Twisted-Fields__rp2040-motor-controller__RP2040_base",
    "snsp": "stonedDiscord__nonSNES__SNSP-CPU-01",
    "quanta": "obsilab__Quanta75__Quanta75_BareRP2040_JLCPCBAoptimized",
    "link": "oro-os__link__link",
}
RANKS = (0, 1, 2)
ROUTER_SEEDS = (1001, 1002, 1003)
PLACER_SEED = 1


def cells():
    """(name, short board, rank, router seed), the first-ranked placements first."""
    return [(f"{short}-k{rank}-r{router}", short, rank, router)
            for rank in RANKS for router in ROUTER_SEEDS for short in BOARDS]


def row(directory):
    try:
        rows = json.loads((directory / "results.json").read_text())
    except (OSError, ValueError):
        return None
    return rows[0] if rows else None


def probes(out, short):
    """How many placements the board's race probed, from a finished cell."""
    # (A rerun into another directory reads the matrix's own cells too.)
    for directory in [*out.glob(f"{short}-k*-r*"), *OUT.glob(f"{short}-k*-r*")]:
        report = directory / BOARDS[short] / "layout" / "board-layout.json"
        try:
            return len((json.loads(report.read_text()).get("race") or {}).get("probe") or [])
        except (OSError, ValueError):
            continue
    return None


def machine_ready():
    now = datetime.datetime.now()
    minutes = now.hour * 60 + now.minute
    summary = MAIN / "build" / "nightly" / now.strftime("%Y-%m-%d") / "summary.md"
    if 80 <= minutes < 600 and not summary.exists():
        return False, "the nightly's hours"
    available = 0.0
    for line in Path("/proc/meminfo").read_text().splitlines():
        if line.startswith("MemAvailable:"):
            available = int(line.split()[1]) / 1024 / 1024
    load = os.getloadavg()[0]
    if available < 16.0:
        return False, f"MemAvailable {available:.1f} GB"
    if load > 30.0:
        return False, f"load {load:.1f}"
    return True, ""


def command(cell, out, binary):
    name, short, _, _ = cell
    return [sys.executable, str(RUN), str(out / name), "--tasks", str(TASKS), "--quality",
            "--only", BOARDS[short], "--jobs", "1", "--threads", "2", "--binary", str(binary),
            "--placer", json.dumps({"tuning": {"seed": PLACER_SEED}})]


def run(arguments):
    out = arguments.out
    out.mkdir(parents=True, exist_ok=True)
    pending = [cell for cell in cells()
               if (not arguments.only or cell[0] in arguments.only) and row(out / cell[0]) is None]
    print(f"{len(pending)} cells to run", flush=True)
    running = {}
    while pending or running:
        for name, process in list(running.items()):
            if process.poll() is not None:
                print(f"{time.strftime('%H:%M:%S')} {name} exit {process.returncode}", flush=True)
                (out / f"{name}.exit").write_text(f"{process.returncode}\n")
                del running[name]
        if pending and len(running) < arguments.parallel:
            # A board's cells do not run side by side until its first is
            # done: the first tells how many placements its race probes.
            index = next((i for i, cell in enumerate(pending)
                          if cell[2] == 0 or probes(out, cell[1]) is not None), None)
            if index is None:
                time.sleep(10)
                continue
            cell = pending[index]
            count = probes(out, cell[1])
            if count is not None and cell[2] >= count:
                print(f"{time.strftime('%H:%M:%S')} skip {cell[0]}: the race probes {count} placements", flush=True)
                pending.pop(index)
                continue
            ready, why = machine_ready()
            if not ready:
                print(f"{time.strftime('%H:%M:%S')} waiting: {why}", flush=True)
                time.sleep(60)
                continue
            pending.pop(index)
            environment = dict(os.environ, PCB_RACE_KEEP=str(cell[2]), PCB_ROUTER_SEED=str(cell[3]))
            log = open(out / f"{cell[0]}.log", "w")
            running[cell[0]] = subprocess.Popen(command(cell, out, arguments.binary), env=environment,
                                                stdout=log, stderr=subprocess.STDOUT)
            print(f"{time.strftime('%H:%M:%S')} start {cell[0]}", flush=True)
            # The load average needs a while to show a new run.
            time.sleep(20)
            continue
        time.sleep(10)
    print("done", flush=True)


def main():
    parser = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    parser.add_argument("action", choices=("run", "list"))
    parser.add_argument("--out", type=Path, default=OUT)
    parser.add_argument("--binary", type=Path, default=BINARY)
    parser.add_argument("--parallel", type=int, default=6)
    parser.add_argument("--only", nargs="*")
    arguments = parser.parse_args()
    if arguments.action == "list":
        for cell in cells():
            print(cell[0], "done" if row(arguments.out / cell[0]) else "")
        return
    run(arguments)


if __name__ == "__main__":
    main()
