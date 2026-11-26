#!/usr/bin/env python3
# Copyright (C) 2026 Florian Loitsch.
"""How far each stage of our own placements moves the active parts, and
what the objective does over those moves (docs/experiments/keep-arrangement.md).

    stages.py place --out DIR --binary BIN [--slots 6] [--seed 1] TIER:BOARD...
    stages.py report --out DIR [--json FILE]

`place` runs the layout's place check (`place-kicad-board` with a nightly
tier's layout.json and router.json, two threads, the pinned work rates)
with the placer's trace into `<out>/<tier>/<board>/s<seed>/`. TIER is
`short` or `sweep` (the night's directory of that name: the short tier's
budget places plain, the sweep's by skeleton). `report` reads every trace
there and prints, per board and stage, the active parts' displacement (the
parts `skeleton_passive` does not call passive) and the objective's terms.
The runner waits while the 1-minute load is above 20 or less than 16 GB are
available, starts nothing between 01:45 and 07:00, and skips units already
done.
"""

import argparse
import json
import math
import os
import statistics
import subprocess
import sys
import time
from pathlib import Path

ROOT = Path(__file__).resolve().parents[2]
NIGHT = Path("/home/flo/programming/pcb-maker/build/nightly/2026-11-25")


def machine_free():
    load = float(Path("/proc/loadavg").read_text().split()[0])
    available = 0.0
    for line in Path("/proc/meminfo").read_text().splitlines():
        if line.startswith("MemAvailable:"):
            available = int(line.split()[1]) / 1e6
    now = time.localtime()
    minutes = now.tm_hour * 60 + now.tm_min
    return load <= 20 and available >= 16 and not (105 <= minutes < 420)


def place_unit(out, binary, tier, board, seed, tuning=None):
    run = NIGHT / tier / board
    source = run / "source"
    board_id = next(source.glob("*.kicad_pcb")).stem
    layout = json.loads((run / "layout.json").read_text())
    router = run / "router.json"
    placer = dict(layout.get("placer", {}))
    placer["tuning"] = {**placer.get("tuning", {}), **(tuning or {}), "seed": seed}
    work = (Path(out) / tier / board / f"s{seed}").resolve()
    command = [str(Path(binary).resolve()), "place-kicad-board", str(source), board_id, str(work / "placed"),
               str(work / "layout.json"), "--router", str(router) if router.exists() else "auto"]
    env = {"PCB_PLACER_TRACE": str(work / "trace.json")}

    def prepare():
        work.mkdir(parents=True, exist_ok=True)
        (work / "layout.json").write_text(json.dumps({**layout, "placer": placer}))

    return work / "done", command, env, prepare, work / "place.log"


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
            time.sleep(2)
        time.sleep(10)
    print(time.strftime("%H:%M:%S"), "pool empty", flush=True)


def distance(a, b):
    return math.hypot(a[0] - b[0], a[1] - b[1])


def total(objective):
    """The objective without its overlap term (an annealed placement
    overlaps, a legal one does not): wire, relations, hard, crossings."""
    return (sum(objective["wirelength"]) + objective["relations"] + objective["hard"]
            + objective["crossing_weight"] * (objective["crossings_all"] or 0))


# The steps between stages, in pipeline order: (name, from, to).
STEPS = [
    ("skeleton legaliser", "skeleton_annealed", "skeleton_legal"),
    ("skeleton refinement", "skeleton_legal", "skeleton_refined"),
    ("skeleton repair", "skeleton_refined", "skeleton_repaired"),
    ("skeleton align", "skeleton_repaired", "skeleton_final"),
    ("passives' phase", "skeleton_final", "final"),
    ("legaliser", "annealed", "legal"),
    ("refinement", "legal", "refined"),
    ("relation repair", "refined", "repaired"),
    ("align and exits", "repaired", "final"),
]


def seed_rows(seed):
    """Per step of one traced seed: the active and passive parts' moves and
    the objective before and after (the skeleton's own for its steps)."""
    stages = seed["stages"]
    index = {name: position for position, name in enumerate(stages)}
    by_skeleton = "skeleton_final" in index and not (seed.get("skeleton") or {}).get("fallback_kept")
    rows = []
    for name, start, end in STEPS:
        if start not in index or end not in index:
            continue
        skeleton_step = start.startswith("skeleton_") and end.startswith("skeleton_")
        moves = {True: [], False: []}
        turned = {True: 0, False: 0}
        for part in seed["parts"]:
            a, b = part["centers"][index[start]], part["centers"][index[end]]
            moves[part["passive"]].append(distance(a, b))
            if part["angles"][index[start]] != part["angles"][index[end]]:
                turned[part["passive"]] += 1
        # The legaliser's moves of the parts that met nothing where the
        # anneal left them (`met_annealed`: the last anneal's, so not the
        # skeleton's own).
        clean = [distance(part["centers"][index[start]], part["centers"][index[end]])
                 for part in seed["parts"] if name == "legaliser" and not part.get("met_annealed", [None])]
        objective = seed["skeleton_objective"] if skeleton_step and seed.get("skeleton_objective") else seed["objective_bare"]
        before, after = objective[start], objective[end]
        active = moves[False]
        rows.append({
            "step": name,
            "active": len(active),
            "active_mean": statistics.fmean(active) if active else 0.0,
            "active_moved": sum(1 for d in active if d > 0.5),
            "active_far": sum(1 for d in active if d > 2.0),
            "active_turned": turned[False],
            "passive_mean": statistics.fmean(moves[True]) if moves[True] else 0.0,
            "clean": len(clean),
            "clean_moved": sum(1 for d in clean if d > 1.0),
            "clean_moved_mean": statistics.fmean([d for d in clean if d > 1.0] or [0.0]),
            "objective": [total(before), total(after)],
            "wire": [sum(before["wirelength"]), sum(after["wirelength"])],
            "crossings": [before["crossings_all"] or 0, after["crossings_all"] or 0],
            "relations": [before["relations"], after["relations"]],
            "overlap": [before["overlap"], after["overlap"]],
        })
    return by_skeleton, rows


def report(out, json_file):
    out = Path(out)
    result = {}
    for trace in sorted(out.glob("*/*/s*/trace.json")):
        tier, board = trace.parts[-4], trace.parts[-3]
        done = trace.parent / "done"
        if not done.exists():
            continue
        data = json.loads(trace.read_text())
        entry = result.setdefault(f"{tier}:{board}", {"exit": [], "seeds": [], "parts": []})
        entry["exit"].append(done.read_text().strip())
        for seed in data["seeds"]:
            by_skeleton, rows = seed_rows(seed)
            entry["parts"].append([part["reference"] for part in seed["parts"]])
            entry["seeds"].append({"seed": seed["seed"], "kept": seed["kept"], "skeleton": by_skeleton,
                                   "unplaced": seed["unplaced"], "illegal": seed["illegal"], "rows": rows})
    if json_file:
        Path(json_file).write_text(json.dumps(result, indent=1))
    print("| board (tier) | seeds (by skeleton) | step | active parts | moved, mean mm | > 0.5 mm | > 2 mm | turned | passives, mean mm | objective before -> after | wire | crossings | relations |")
    print("| --- | --- | --- | ---: | ---: | ---: | ---: | ---: | ---: | --- | --- | --- | --- |")
    pooled = {}
    for name, entry in result.items():
        tier, board = name.split(":")
        seeds = entry["seeds"]
        steps = []
        for seed in seeds:
            for row in seed["rows"]:
                if row["step"] not in steps:
                    steps.append(row["step"])
        for step in steps:
            rows = [row for seed in seeds for row in seed["rows"] if row["step"] == step]
            mean = lambda key: statistics.fmean(row[key] for row in rows)  # noqa: E731
            pair = lambda key: f"{statistics.fmean(row[key][0] for row in rows):.0f} -> {statistics.fmean(row[key][1] for row in rows):.0f}"  # noqa: E731
            print(f"| {board.split('__')[-1]} ({tier}) | {len(rows)} ({sum(1 for s in seeds if s['skeleton'])}) | {step} | "
                  f"{mean('active'):.0f} | {mean('active_mean'):.2f} | {mean('active_moved'):.1f} | {mean('active_far'):.1f} | "
                  f"{mean('active_turned'):.1f} | {mean('passive_mean'):.2f} | {pair('objective')} | {pair('wire')} | "
                  f"{pair('crossings')} | {pair('relations')} |")
            pooled.setdefault((tier, step), []).extend(rows)
    print()
    print("| board (tier) | parts | legal where the anneal left them | of those the legaliser moved > 1 mm (their mean mm) | objective annealed -> legal -> final |")
    print("| --- | ---: | ---: | ---: | --- |")
    for name, entry in result.items():
        tier, board = name.split(":")
        rows = [row for seed in entry["seeds"] for row in seed["rows"] if row["step"] == "legaliser"]
        finals = [row for seed in entry["seeds"] for row in seed["rows"] if row["step"] == "align and exits"]
        if not rows or len(rows) != len(finals):
            continue
        mean = lambda key: statistics.fmean(row[key] for row in rows)  # noqa: E731
        moved = [row["clean_moved_mean"] for row in rows if row["clean_moved"]]
        print(f"| {board.split('__')[-1]} ({tier}) | {statistics.fmean(len(seed_parts) for seed_parts in entry['parts']):.0f} | "
              f"{mean('clean'):.0f} | {mean('clean_moved'):.1f} ({statistics.fmean(moved) if moved else 0.0:.1f}) | "
              f"{statistics.fmean(row['objective'][0] for row in rows):.0f} -> {statistics.fmean(row['objective'][1] for row in rows):.0f} -> "
              f"{statistics.fmean(row['objective'][1] for row in finals):.0f} |")
    print()
    print("| tier | step | seeds | active moved, mean mm (mean over seeds) | share of active parts > 0.5 mm | > 2 mm | objective after / before (median) | seeds whose objective rose |")
    print("| --- | --- | ---: | ---: | ---: | ---: | ---: | ---: |")
    for (tier, step), rows in pooled.items():
        parts = sum(row["active"] for row in rows) or 1
        ratios = [row["objective"][1] / row["objective"][0] for row in rows if row["objective"][0] > 0]
        print(f"| {tier} | {step} | {len(rows)} | {statistics.fmean(row['active_mean'] for row in rows):.2f} | "
              f"{sum(row['active_moved'] for row in rows) / parts:.2f} | {sum(row['active_far'] for row in rows) / parts:.2f} | "
              f"{statistics.median(ratios):.3f} | {sum(1 for ratio in ratios if ratio > 1.0005)} |")


def main():
    parser = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    sub = parser.add_subparsers(dest="command", required=True)
    p = sub.add_parser("place")
    p.add_argument("--out", required=True)
    p.add_argument("--binary", required=True)
    p.add_argument("--slots", type=int, default=6)
    p.add_argument("--seed", type=int, default=1)
    p.add_argument("--tuning", default="{}", help="placer tuning keys over the tier's, as JSON (a diagnosis)")
    p.add_argument("items", nargs="+")
    r = sub.add_parser("report")
    r.add_argument("--out", required=True)
    r.add_argument("--json")
    arguments = parser.parse_args()
    if arguments.command == "report":
        report(arguments.out, arguments.json)
        return
    units = []
    for item in arguments.items:
        tier, _, board = item.partition(":")
        units.append((item, (lambda t=tier, b=board: place_unit(arguments.out, arguments.binary, t, b, arguments.seed,
                                                                json.loads(arguments.tuning)))))
    env = dict(os.environ, RAYON_NUM_THREADS="2", PCB_WORK_RATE="3.15e6,150e6")
    pool(units, arguments.slots, env)


if __name__ == "__main__":
    sys.exit(main())
