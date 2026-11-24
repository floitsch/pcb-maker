#!/usr/bin/env python3
# Copyright (C) 2026 Florian Loitsch.
"""Placements with a routed label, for rescoring objectives offline
(docs/experiments/objective-terms.md).

    labelled.py place --out DIR --binary BIN [--slots 3] BOARD:VARIANT...
    labelled.py route --out DIR --binary BIN [--slots 3] BOARD:UNIT...
    labelled.py results --out DIR

`place` runs the layout's place check (`place-kicad-board` with the
2026-11-18 sweep's layout.json and router.json, two threads, the pinned
work rates) with the placer's trace and the designer's poses beside, as
experiments/cut-census/skeleton_gap.py does, into
`<out>/runs/<board>/<variant>/`. Variants:

- `ours-s<base>`: the placer as it is, `tuning.seed` = base;
- `ours-plain-s<base>`: the same without the race over skeletons
  (`"skeleton_race": null`);
- `designer`: every level's legaliser seats the designer's poses and the
  refinement follows (`PCB_PLACER_START_STAGE=legalize`), through the
  skeleton's phase and the passives';
- `designer-plain`: the same without the race over skeletons.

`route` lays out first routes (layout seed 1, `stop_after_first_route`,
experiments/placement-gap/partialcopy.py's machinery: every movable part
pinned with `place` entries) of:

- `designer`: the designer's own placement;
- `nonpassive@<seed>`: the designer's active parts pinned (partialcopy.py's
  `nonpassive` rule), the passives placed by the layout at layout seed
  `<seed>`;
- `<variant>@<seed>`: traced seed `<seed>`'s final poses of the place
  check `<variant>` (any seed of the run, kept or not; `kept`: the
  seed the placer kept).

`results` lists every first route's open terminals as JSON.
Both runners wait while the 1-minute load is above 20 or less than 16 GB
are available, never start between 02:00 and 07:00, and skip units
already done (restartable).
"""

import argparse
import json
import os
import subprocess
import sys
import time
from pathlib import Path

ROOT = Path(__file__).resolve().parents[2]
sys.path.insert(0, str(ROOT / "experiments/placement-gap"))
NIGHT = Path("/home/flo/programming/pcb-maker/build/nightly/2026-11-18")


def machine_free():
    load = float(Path("/proc/loadavg").read_text().split()[0])
    available = 0.0
    for line in Path("/proc/meminfo").read_text().splitlines():
        if line.startswith("MemAvailable:"):
            available = int(line.split()[1]) / 1e6
    hour = time.localtime().tm_hour
    return load <= 20 and available >= 16 and not (2 <= hour < 7)


def designer_board(board):
    return next((NIGHT / "designer" / board / "placed").glob("*.kicad_pcb"))


def place_unit(out, binary, board, variant):
    sweep = NIGHT / "sweep" / board
    source = sweep / "source"
    board_id = next(source.glob("*.kicad_pcb")).stem
    layout = json.loads((sweep / "layout.json").read_text())
    router = sweep / "router.json"
    plain = "plain" in variant
    base = int(variant.rsplit("-s", 1)[1]) if variant.startswith("ours") else 1
    placer = dict(layout.get("placer", {}))
    placer["tuning"] = {**placer.get("tuning", {}), "seed": base}
    if plain:
        placer["skeleton_race"] = None
    work = (Path(out) / "runs" / board / variant).resolve()
    command = [str(Path(binary).resolve()), "place-kicad-board", str(source), board_id, str(work / "placed"),
               str(work / "layout.json"), "--router", str(router) if router.exists() else "auto"]
    env = {"PCB_PLACER_TRACE": str(work / "trace.json"), "PCB_PLACER_DESIGNER": str(designer_board(board))}
    if variant.startswith("designer"):
        env.update({"PCB_PLACER_START": str(designer_board(board)), "PCB_PLACER_START_STAGE": "legalize"})

    def prepare():
        work.mkdir(parents=True, exist_ok=True)
        (work / "layout.json").write_text(json.dumps({**layout, "placer": placer}))

    return work / "done", command, env, prepare, work / "place.log"


def route_unit(out, binary, board, unit):
    import partialcopy  # noqa: PLC0415  (its extraction needs KiCad)
    import stages  # noqa: PLC0415

    out = Path(out).resolve()
    first = out / "firstroute"
    task = partialcopy.v7_tasks()[board]
    designer = partialcopy.extracted(partialcopy.designer_board(task), first / "extract" / "designer" / f"{board}.json")
    fixed = (json.loads((partialcopy.TASKS.parent / task["constraints"]).read_text()) if task.get("constraints")
             else {}).get("fixed", [])
    movable = {partialcopy.key(f) for f in partialcopy.movable(partialcopy.numbered(designer["footprints"]), fixed)}
    layout_seed = 1
    if unit == "designer":
        source, rule = designer, "designer"
    elif unit.startswith("nonpassive@"):
        layout_seed = int(unit.partition("@")[2])
        footprints = [f for f in partialcopy.numbered(designer["footprints"])
                      if partialcopy.key(f) in movable and not partialcopy.is_passive(f)]
        partialcopy.write_unit(first, task, "nonpassive", footprints, designer)
        command = partialcopy.unit_command(first, board, f"nonpassive-s{layout_seed}", binary)
        label = Path(command[command.index("--tasks") - 1]).name
        runs = first / "runs"
        return (runs / f"{label}.done", command, {}, lambda: runs.mkdir(parents=True, exist_ok=True),
                runs / f"{label}.out")
    else:
        variant, _, seed = unit.partition("@")
        run = out / "runs" / board / variant
        placed = next((run / "placed").glob("*.kicad_pcb"))
        source = partialcopy.extracted(placed, first / "extract" / board / f"{variant}.json")
        source = stages.at_stage(source, run / "trace.json", "final", None if seed == "kept" else int(seed))
        rule = f"{variant}-at-{seed}"
    footprints = [f for f in partialcopy.numbered(source["footprints"]) if partialcopy.key(f) in movable]
    if len(footprints) != len(movable):
        raise SystemExit(f"{board}:{unit}: {len(footprints)} of {len(movable)} movable parts found")
    partialcopy.write_unit(first, task, rule, footprints, source)
    command = partialcopy.unit_command(first, board, f"{rule}-s{layout_seed}", binary)
    label = Path(command[command.index("--tasks") - 1]).name
    runs = first / "runs"
    return runs / f"{label}.done", command, {}, lambda: runs.mkdir(parents=True, exist_ok=True), runs / f"{label}.out"


def pool(units, slots, base_env):
    queue = list(units)
    running = {}
    while queue or running:
        for name, (process, log, done) in list(running.items()):
            if process.poll() is not None:
                log.close()
                done.write_text(f"{process.returncode}\n")
                print(time.strftime("%H:%M:%S"), "done", name, process.returncode, flush=True)
                del running[name]
        while queue and len(running) < slots and machine_free():
            name, make = queue.pop(0)
            done, command, env, prepare, log_path = make()
            if done.exists():
                continue
            prepare()
            log = open(log_path, "w")
            running[name] = (subprocess.Popen(command, cwd=ROOT, stdout=log, stderr=subprocess.STDOUT,
                                              env=dict(base_env, **env)), log, done)
            print(time.strftime("%H:%M:%S"), "start", name, flush=True)
            time.sleep(5)
        time.sleep(30)
    print(time.strftime("%H:%M:%S"), "pool empty", flush=True)


def split(item):
    board, _, unit = item.partition(":")
    return board, unit


def results(out):
    first = Path(out) / "firstroute" / "runs"
    result = {}
    for done in sorted(first.glob("*.done")) if first.exists() else []:
        label = done.stem
        reports = list((first / label).glob("*/layout/board-layout.json"))
        report = json.loads(reports[0].read_text()) if reports else {}
        result[label] = {"exit": done.read_text().strip(), "first_open": report.get("first_unconnected_terminals")}
    return result


def main():
    parser = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    sub = parser.add_subparsers(dest="command", required=True)
    for name in ("place", "route"):
        p = sub.add_parser(name)
        p.add_argument("--out", required=True)
        p.add_argument("--binary", required=True)
        p.add_argument("--slots", type=int, default=3)
        p.add_argument("items", nargs="+")
    r = sub.add_parser("results")
    r.add_argument("--out", required=True)
    arguments = parser.parse_args()
    if arguments.command == "results":
        print(json.dumps(results(arguments.out), indent=1))
        return
    maker = place_unit if arguments.command == "place" else route_unit
    units = [(item, (lambda b=split(item)[0], u=split(item)[1]: maker(arguments.out, arguments.binary, b, u)))
             for item in arguments.items]
    env = dict(os.environ)
    if arguments.command == "place":
        env.update(RAYON_NUM_THREADS="2", PCB_WORK_RATE="3.15e6,150e6")
    pool(units, arguments.slots, env)


if __name__ == "__main__":
    sys.exit(main())
