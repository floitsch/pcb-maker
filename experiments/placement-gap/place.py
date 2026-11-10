#!/usr/bin/env python3
# Copyright (C) 2026 Toit contributors.
# Use of this source code is governed by an MIT-style license that can be
# found in the LICENSE file.
"""Stage 0 of the placer protocol for a placer switch: the placement alone,
as the layout of the sweep's config places (`place-kicad-board` with the
board's layout.json and router.json: the layout's share of its budget, its
seeds), per variant and board (docs/placer.md, "Where the gap to the designer comes from", "Net weights").

    place.py --sweep build/nightly/2026-11-03/sweep --out build/teach2-place \\
        --variant 'steep=build/bin/pcb-maker-x1890={}' \\
        --variant 'flat=build/bin/pcb-maker-teach2-nw1={"net_weight": "flat"}' BOARD...
    metrics.py run --sweep build/teach2-place/flat --designer build/nightly/2026-11-03/designer \\
        --out build/teach2-gap/flat BOARD...

Writes `<out>/<variant>/<board>/layout/placed/<id>.kicad_pcb` (the layout
of a sweep, so that metrics.py reads it as one) and the run's log beside
it. Each run at 2 threads and the pinned work rates (3.15e6, 150e6, as
run.py's tiers); at most `--slots` in flight (or what `--slots-file`
holds, read every 20 s), the boards with the longest layouts first. The
binary reserves its memory in the ledger itself. Units with a `.done`
file are skipped (restartable).
"""

import argparse
import json
import os
import subprocess
import sys
import time
from pathlib import Path

ROOT = Path(__file__).resolve().parents[2]
ESTIMATES = ROOT / "benchmarks/agent-tasks/estimates.json"


def seconds(board):
    try:
        return json.loads(ESTIMATES.read_text())["boards"][board]["layout"]["seconds"]
    except KeyError:
        return 0.0


def units(arguments):
    sweep = Path(arguments.sweep).resolve()
    result = []
    for board in sorted(arguments.boards, key=lambda b: -seconds(b)):
        for variant in arguments.variant:
            name, binary, settings = variant.split("=", 2)
            source = sweep / board / "source"
            board_id = next(source.glob("*.kicad_pcb")).stem
            layout = json.loads((sweep / board / "layout.json").read_text())
            layout["placer"] = {**layout.get("placer", {}), **json.loads(settings)}
            work = (Path(arguments.out) / name / board).resolve()
            router = sweep / board / "router.json"
            command = [str(ROOT / binary), "place-kicad-board", str(source), board_id,
                       str(work / "layout/placed"), str(work / "layout.json"),
                       "--router", str(router) if router.exists() else "auto"]
            result.append((f"{name}/{board}", work, layout, command))
    return result


def main():
    parser = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    parser.add_argument("--sweep", required=True)
    parser.add_argument("--out", required=True)
    parser.add_argument("--variant", action="append", required=True,
                        help="NAME=BINARY=PLACER_JSON (the placer settings merged over the sweep's)")
    parser.add_argument("--slots", type=int, default=4)
    parser.add_argument("--slots-file")
    parser.add_argument("--dry-run", action="store_true")
    parser.add_argument("boards", nargs="+")
    arguments = parser.parse_args()
    queue = [u for u in units(arguments) if not (u[1] / "done").exists()]
    if arguments.dry_run:
        for name, _, layout, command in queue:
            print(name, json.dumps(layout), " ".join(command))
        return
    env = dict(os.environ, RAYON_NUM_THREADS="2", PCB_WORK_RATE="3.15e6,150e6")
    running = {}
    while queue or running:
        for name, (process, log, work) in list(running.items()):
            if process.poll() is not None:
                log.close()
                (work / "done").write_text(f"{process.returncode}\n")
                print(time.strftime("%H:%M:%S"), "done", name, process.returncode, flush=True)
                del running[name]
        slots = arguments.slots
        if arguments.slots_file and Path(arguments.slots_file).exists():
            try:
                slots = int(Path(arguments.slots_file).read_text().strip())
            except ValueError:
                pass
        while queue and len(running) < slots:
            name, work, layout, command = queue.pop(0)
            (work / "layout").mkdir(parents=True, exist_ok=True)
            (work / "layout.json").write_text(json.dumps(layout))
            log = open(work / "place.log", "w")
            running[name] = (subprocess.Popen(command, cwd=ROOT, stdout=log, stderr=subprocess.STDOUT, env=env),
                             log, work)
            print(time.strftime("%H:%M:%S"), "start", name, flush=True)
        time.sleep(20)
    print(time.strftime("%H:%M:%S"), "pool empty", flush=True)


if __name__ == "__main__":
    main()
