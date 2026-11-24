#!/usr/bin/env python3
# Copyright (C) 2026 Florian Loitsch.
"""Proposal C's gate (docs/experiments/cut-census.md): does the skeleton's
optimiser leave objective on the table? Per board, our seeds' skeletons
against the designer's skeleton legalised by our legaliser, all judged by
the skeleton's own objective (the trace's `skeleton_objective`: the
problem `place_skeleton` builds, without halos).

    skeleton_gap.py run --sweep build/nightly/2026-11-18/sweep \\
        --designer build/nightly/2026-11-18/designer --binary BIN --out DIR \\
        [--bases 1,101,201,301,401,501] [--slots 3] BOARD...
    skeleton_gap.py table --out DIR

`run` places each board as the layout's place check does
(`place-kicad-board` with the sweep's layout.json and router.json, two
threads, the pinned work rates) with the trace (`PCB_PLACER_TRACE`) and
the designer's poses beside (`PCB_PLACER_DESIGNER`):

- `ours-s<base>`: the placer's own start at `tuning.seed` = base (the
  kept seeds of each run are base, base + 1, ...; the bases lie 100 apart);
- `designer`: every level's legaliser seats the designer's poses and the
  refinement follows (`PCB_PLACER_START_STAGE=legalize`), in the skeleton's
  phase as in the passives'.

Each unit writes `<out>/runs/<board>/<variant>/` (the placed project,
`trace.json`, `place.log`, `done`). A run waits while the machine is loaded
(1-minute load above 20 or less than 16 GB available) and never starts
between 02:00 and 07:00.

`table` reads every trace: per kept seed the skeleton objective after the
refinement (`skeleton_final`: wire + relations + 1000 hard + crossings,
the anneal's prices without the overlap term, and the overlap apart), the
designer's legalised and refined skeleton the same way (and its raw poses
on the skeleton's problem), and writes `<out>/skeleton-gap.md` and
`<out>/skeleton-gap.json`. The verdict per board: the designer's below our
best seed by more than the seeds' spread (max - min), inside it, or above.
"""

import argparse
import json
import os
import statistics
import subprocess
import sys
import time
from pathlib import Path

ROOT = Path(__file__).resolve().parents[2]


def machine_free():
    load = float(Path("/proc/loadavg").read_text().split()[0])
    available = 0
    for line in Path("/proc/meminfo").read_text().splitlines():
        if line.startswith("MemAvailable:"):
            available = int(line.split()[1]) / 1e6
    hour = time.localtime().tm_hour
    return load <= 20 and available >= 16 and not (2 <= hour < 7)


def units(arguments):
    sweep = Path(arguments.sweep).resolve()
    designer_root = Path(arguments.designer).resolve()
    result = []
    for board in arguments.boards:
        source = sweep / board / "source"
        board_id = next(source.glob("*.kicad_pcb")).stem
        designer = next((designer_root / board / "placed").glob("*.kicad_pcb"))
        layout = json.loads((sweep / board / "layout.json").read_text())
        router = sweep / board / "router.json"
        variants = [(f"ours-s{base}", base, {}) for base in arguments.bases]
        variants.append(("designer", arguments.bases[0],
                         {"PCB_PLACER_START": str(designer), "PCB_PLACER_START_STAGE": "legalize"}))
        for name, base, env in variants:
            placer = dict(layout.get("placer", {}))
            placer["tuning"] = {**placer.get("tuning", {}), "seed": base}
            work = (Path(arguments.out) / "runs" / board / name).resolve()
            command = [str(Path(arguments.binary).resolve()), "place-kicad-board", str(source), board_id,
                       str(work / "placed"), str(work / "layout.json"),
                       "--router", str(router) if router.exists() else "auto"]
            env = dict(env, PCB_PLACER_TRACE=str(work / "trace.json"), PCB_PLACER_DESIGNER=str(designer))
            result.append((f"{board}/{name}", work, {**layout, "placer": placer}, command, env))
    return result


def run(arguments):
    queue = [u for u in units(arguments) if not (u[1] / "done").exists()]
    base_env = dict(os.environ, RAYON_NUM_THREADS="2", PCB_WORK_RATE="3.15e6,150e6")
    running = {}
    while queue or running:
        for name, (process, log, work) in list(running.items()):
            if process.poll() is not None:
                log.close()
                (work / "done").write_text(f"{process.returncode}\n")
                print(time.strftime("%H:%M:%S"), "done", name, process.returncode, flush=True)
                del running[name]
        while queue and len(running) < arguments.slots and machine_free():
            name, work, layout, command, env = queue.pop(0)
            work.mkdir(parents=True, exist_ok=True)
            (work / "layout.json").write_text(json.dumps(layout))
            log = open(work / "place.log", "w")
            running[name] = (subprocess.Popen(command, cwd=ROOT, stdout=log, stderr=subprocess.STDOUT,
                                              env=dict(base_env, **env)), log, work)
            print(time.strftime("%H:%M:%S"), "start", name, flush=True)
            time.sleep(5)
        time.sleep(30)
    print(time.strftime("%H:%M:%S"), "pool empty", flush=True)


def total(objective, overlap=False):
    crossings = objective.get("crossings_anneal") or 0
    value = (sum(objective["wirelength"]) + objective["relations"] + 1000.0 * objective["hard"]
             + objective["crossing_weight"] * crossings)
    return value + (400.0 * objective["overlap"] if overlap else 0.0)


def terms(objective):
    return {"wire": sum(objective["wirelength"]), "relations": objective["relations"] + 1000.0 * objective["hard"],
            "crossings": objective.get("crossings_anneal") or 0, "overlap_mm2": objective["overlap"],
            "total": total(objective)}


def read_board(directory):
    """Per variant the kept seeds' skeleton objectives (and the trace's
    full-placement objective at `final`)."""
    rows = {}
    for variant in sorted(p for p in directory.iterdir() if p.is_dir()):
        trace = variant / "trace.json"
        if not trace.exists():
            continue
        data = json.loads(trace.read_text())
        seeds = []
        for seed in data["seeds"]:
            skeleton = seed.get("skeleton_objective") or {}
            entry = {"seed": seed["seed"], "kept": seed["kept"], "unplaced": seed["unplaced"],
                     "illegal": seed["illegal"],
                     "fallback": bool((seed.get("skeleton") or {}).get("fallback_kept")),
                     "level": (seed.get("skeleton") or {}).get("skeleton_level")}
            for stage in ("skeleton_legal", "skeleton_final", "designer"):
                if stage in skeleton:
                    entry[stage] = terms(skeleton[stage])
            if "final" in seed["objective"]:
                entry["whole_final"] = terms(seed["objective"]["final"])
            if "designer" in seed["objective"]:
                entry["whole_designer_raw"] = terms(seed["objective"]["designer"])
            seeds.append(entry)
        rows[variant.name] = seeds
    return rows


def table(arguments):
    out = Path(arguments.out)
    result = {}
    lines = ["| board | our seeds | ours best / median / worst | spread | designer legalised | raw designer | "
             "designer - best, in spreads | verdict |", "| --- " * 8 + "|"]
    for board_dir in sorted((out / "runs").iterdir()):
        rows = read_board(board_dir)
        ours = [s for name, seeds in rows.items() if name.startswith("ours") for s in seeds
                if "skeleton_final" in s and not s["fallback"]]
        designer = [s for s in rows.get("designer", []) if "skeleton_final" in s and not s["fallback"]]
        raw = [s["designer"]["total"] for name, seeds in rows.items() for s in seeds if "designer" in s]
        entry = {"rows": rows}
        if ours:
            values = sorted(s["skeleton_final"]["total"] for s in ours)
            best, worst = values[0], values[-1]
            spread = worst - best
            entry.update({"ours": values, "best": best, "worst": worst, "spread": spread,
                          "median": statistics.median(values)})
            if designer:
                d = min(s["skeleton_final"]["total"] for s in designer)
                entry["designer"] = d
                margin = (best - d) / spread if spread > 0 else float("inf")
                entry["margin_spreads"] = margin
                if d < best - spread:
                    verdict = "below by more than the spread"
                elif d < best:
                    verdict = "below best, within the spread"
                elif d <= worst:
                    verdict = "inside the spread"
                else:
                    verdict = "above our worst"
                entry["verdict"] = verdict
            else:
                d, margin, verdict = None, None, "no designer skeleton"
            entry["raw_designer"] = statistics.median(raw) if raw else None
            lines.append(f"| {board_dir.name} | {len(values)} | {best:.0f} / {statistics.median(values):.0f} / "
                         f"{worst:.0f} | {spread:.0f} | {d if d is None else round(d)} | "
                         f"{entry['raw_designer'] if entry['raw_designer'] is None else round(entry['raw_designer'])} | "
                         f"{'' if margin is None else f'{(d - best) / spread if spread > 0 else 0:+.2f}'} | {verdict} |")
        else:
            lines.append(f"| {board_dir.name} | 0 | | | | | | no skeleton seeds |")
        result[board_dir.name] = entry
    (out / "skeleton-gap.json").write_text(json.dumps(result, indent=1))
    (out / "skeleton-gap.md").write_text("\n".join(lines) + "\n")
    print("\n".join(lines))


def main():
    parser = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    sub = parser.add_subparsers(dest="command", required=True)
    r = sub.add_parser("run")
    r.add_argument("--sweep", required=True)
    r.add_argument("--designer", required=True)
    r.add_argument("--binary", required=True)
    r.add_argument("--out", required=True)
    r.add_argument("--bases", default="1,101,201,301,401,501")
    r.add_argument("--slots", type=int, default=3)
    r.add_argument("boards", nargs="+")
    t = sub.add_parser("table")
    t.add_argument("--out", required=True)
    arguments = parser.parse_args()
    if arguments.command == "run":
        arguments.bases = [int(b) for b in arguments.bases.split(",")]
        run(arguments)
    else:
        table(arguments)


if __name__ == "__main__":
    sys.exit(main())
