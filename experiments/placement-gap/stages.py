#!/usr/bin/env python3
# Copyright (C) 2026 Toit contributors.
# Use of this source code is governed by an MIT-style license that can be
# found in the LICENSE file.
"""Where in the placer the arrangement of the active parts leaves the
designer's, and whether the designer's arrangement is a fixed point of each
stage (docs/placer.md, "Where the arrangement is lost (2026-11-04)").

    stages.py run --inputs DIR --out DIR --binary PATH [--slots 3] [--threads 2] UNIT...
    stages.py route --inputs DIR --out DIR --binary PATH [--slots 2] UNIT...
    stages.py table --inputs DIR --out DIR

A unit is `<board>-s<seed>-<variant>`; `--inputs` holds per board and seed
`<board>-s<seed>/` (the layout's `source/`, `layout.json`, `router.json`,
copied from a layout run, and the layout's placed board as
`race-*.kicad_pcb`), per board `<board>.active.json` (the active parts,
partialcopy.py's `nonpassive.pinned.json`: `REF#n`) and
`<board>.designer` (a line: the designer's .kicad_pcb). Each unit is the
layout's place check (`place-kicad-board` with the layout's config: its
seeds, its budget, the reference's rates) with the placer's trace
(`PCB_PLACER_TRACE`, the designer's poses beside: `PCB_PLACER_DESIGNER`),
and with a variant's start (`PCB_PLACER_START`):

- `ours`: the placer's own start;
- `global-all`: the global placement from the designer's poses;
- `global-active`: the same for the active parts, the passives from our start;
- `anneal`: no global placement, each level's anneal from the designer's poses;
- `legalize`: no global placement and no anneal, each level's legaliser
  seats the designer's poses, then the refinement (its `legal` stage is
  the minimal legalisation, its `final` the refinement from there);
- `off`: `ours` without the trace (the identity check of the dump).

`run` writes `<out>/runs/<unit>/` (the placed project, `trace.json`,
`place.log`, `unit.json`: exit code, wall and CPU seconds). `route` lays
out first routes with partialcopy.py's machinery (`<out>/firstroute`, the
layout at seed 1 with `stop_after_first_route` through run.py and the
ledger): `<board>-designer` (every movable part pinned at the designer's
pose), `<board>-ours` (nothing pinned) and `<board>-s<seed>-<variant>`
(every movable part pinned at that place check's kept pose). `table`
measures every traced unit and writes `<out>/tables.md` and
`<out>/stages.json`.

The measures of an arrangement of the active parts against the designer's,
per seed and stage (the trace's stages: `global`, `annealed`, `legal`,
`final`):

- `hpwl`: the placer's half perimeters by net size (2 pins, 3-8, 9 or
  more; the trace's `objective`), ours over the designer's;
- `displacement`: the mean distance of each active part's body centre from
  the designer's after the best rigid alignment (the rotation and
  translation of ours with the least mean distance, `rigid_fit`), in mm and over the designer's
  spread (the root mean square distance of its active parts from their
  centroid: a random arrangement of the same parts scores about 1.4);
  `raw`: without the alignment;
- `pairwise`: the Spearman rank correlation of the active parts' pairwise
  centre distances in ours against the designer's (1: the same parts near
  the same parts);
- `neighbours`: the share of each active part's three nearest active
  parts in the designer's that are among its three nearest in ours.
"""

import argparse
import json
import math
import os
import re
import statistics
import subprocess
import sys
import time
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parent))

import metrics  # noqa: E402

ROOT = Path(__file__).resolve().parents[2]
VARIANTS = {
    "ours": {},
    "off": {},
    "global-all": {"PCB_PLACER_START_STAGE": "global"},
    "global-active": {"PCB_PLACER_START_STAGE": "global"},
    "anneal": {"PCB_PLACER_START_STAGE": "anneal"},
    "legalize": {"PCB_PLACER_START_STAGE": "legalize"},
}
STAGES = ("global", "annealed", "legal", "final")
NEIGHBOURS = 3


# --- the measures -----------------------------------------------------------------------------


def weighted_fit(ours, designer, weights):
    """The rotation and translation that bring the points `ours` closest
    to `designer` in the weighted least squares (no reflection, no scale:
    Kabsch in the plane); returns `ours` moved."""
    total = sum(weights)
    if not ours or total <= 0:
        return [p[:] for p in ours]
    mean_o = [sum(w * p[axis] for w, p in zip(weights, ours)) / total for axis in (0, 1)]
    mean_d = [sum(w * p[axis] for w, p in zip(weights, designer)) / total for axis in (0, 1)]
    cross = dot = 0.0
    for w, (ox, oy), (dx, dy) in zip(weights, ours, designer):
        ox, oy, dx, dy = ox - mean_o[0], oy - mean_o[1], dx - mean_d[0], dy - mean_d[1]
        dot += w * (ox * dx + oy * dy)
        cross += w * (ox * dy - oy * dx)
    angle = math.atan2(cross, dot)
    c, s = math.cos(angle), math.sin(angle)
    return [[c * (x - mean_o[0]) - s * (y - mean_o[1]) + mean_d[0],
             s * (x - mean_o[0]) + c * (y - mean_o[1]) + mean_d[1]] for x, y in ours]


def mean_distance(ours, designer):
    return statistics.mean(math.dist(a, b) for a, b in zip(ours, designer))


def rigid_fit(ours, designer, rounds=30):
    """The rigid motion (rotation and translation, no reflection, no
    scale) of `ours` with the least mean distance to `designer`: the least
    squares fit reweighted by the inverse distances (Weiszfeld's iteration
    for the sum of distances), kept only where it beats the points as
    placed; returns `ours` moved."""
    if not ours:
        return []
    best = [p[:] for p in ours]
    best_mean = mean_distance(best, designer)
    weights = [1.0] * len(ours)
    for _ in range(rounds):
        fitted = weighted_fit(ours, designer, weights)
        mean = mean_distance(fitted, designer)
        if mean < best_mean - 1e-12:
            best, best_mean = fitted, mean
        weights = [1.0 / max(math.dist(a, b), 1e-6) for a, b in zip(fitted, designer)]
    return best


def spread(points):
    """The root mean square distance of the points from their centroid."""
    if not points:
        return 0.0
    mean = [sum(p[axis] for p in points) / len(points) for axis in (0, 1)]
    return math.sqrt(sum((p[0] - mean[0]) ** 2 + (p[1] - mean[1]) ** 2 for p in points) / len(points))


def displacement(ours, designer, align=True):
    """The mean distance of each point from its designer's twin, after the
    rigid fit (`align`) or as placed."""
    if not ours:
        return None
    moved = rigid_fit(ours, designer) if align else ours
    return statistics.mean(math.dist(a, b) for a, b in zip(moved, designer))


def pairwise(ours, designer):
    """Spearman's rank correlation of the pairwise distances."""
    xs, ys = [], []
    for i in range(len(ours)):
        for j in range(i + 1, len(ours)):
            xs.append(math.dist(ours[i], ours[j]))
            ys.append(math.dist(designer[i], designer[j]))
    return metrics.spearman(xs, ys)


def nearest(points, index, k):
    others = sorted((math.dist(points[index], p), j) for j, p in enumerate(points) if j != index)
    return {j for _, j in others[:k]}


def neighbours(ours, designer, k=NEIGHBOURS):
    """The share of each point's k nearest in the designer's that are among
    its k nearest in ours."""
    if len(ours) <= k:
        return None
    kept = [len(nearest(ours, i, k) & nearest(designer, i, k)) / k for i in range(len(ours))]
    return statistics.mean(kept)


def arrangement(ours, designer):
    """Every measure of one arrangement against the designer's."""
    scale = spread(designer)
    aligned = displacement(ours, designer)
    return {
        "displacement_mm": aligned,
        "displacement_share": aligned / scale if aligned is not None and scale > 0 else None,
        "raw_mm": displacement(ours, designer, align=False),
        "pairwise": pairwise(ours, designer),
        "neighbours": neighbours(ours, designer),
        "parts": len(ours),
    }


def total(objective):
    """The anneal's objective at the end of its schedule (its overlap
    factor 400) with the crossings it counts."""
    crossings = objective.get("crossings_anneal") or 0
    return (sum(objective["wirelength"]) + objective["relations"] + 1000.0 * objective["hard"]
            + 400.0 * objective["overlap"] + objective["crossing_weight"] * crossings)


def keyed(parts):
    """Each traced part's key `REF#n` (the n-th of its reference in the
    trace's order)."""
    seen = {}
    result = []
    for part in parts:
        n = seen.get(part["reference"], 0)
        seen[part["reference"]] = n + 1
        result.append(f"{part['reference']}#{n}")
    return result


def measure_seed(seed, active):
    """Per stage the measures of a traced seed's active parts against the
    designer's (`designer_center` in the trace)."""
    keys = keyed(seed["parts"])
    chosen = [part for key, part in zip(keys, seed["parts"]) if key in active and "designer_center" in part]
    designer = [part["designer_center"] for part in chosen]
    designer_objective = seed["objective"].get("designer")
    result = {"seed": seed.get("seed"), "kept": seed.get("kept"), "stages": {}, "centers": {}}
    for index, stage in enumerate(seed["stages"]):
        ours = [part["centers"][index] for part in chosen]
        result["centers"][stage] = ours
        entry = arrangement(ours, designer)
        objective = seed["objective"][stage]
        entry["objective"] = total(objective)
        if designer_objective:
            entry["hpwl"] = [o / d if d > 0 else None
                             for o, d in zip(objective["half_perimeter"], designer_objective["half_perimeter"])]
            entry["hpwl_all"] = sum(objective["half_perimeter"]) / max(sum(designer_objective["half_perimeter"]),
                                                                       1e-9)
        result["stages"][stage] = entry
    if designer_objective:
        result["designer_objective"] = total(designer_objective)
    result["terms"] = {stage: split(seed["objective"][stage]) for stage in seed["stages"]}
    if designer_objective:
        result["terms"]["designer"] = split(designer_objective)
    return result


def split(objective):
    """The objective's terms as the anneal prices them at the end of its
    schedule (mm of wire): the weighted wires by net size, the overlap
    (times 400), the crossings it counts (times their weight), the
    relations (and the hard constraints times 1000)."""
    return {
        "wire_2": objective["wirelength"][0], "wire_3_8": objective["wirelength"][1],
        "wire_9plus": objective["wirelength"][2],
        "overlap": 400.0 * objective["overlap"], "overlapping": objective["overlapping"],
        "crossings": objective["crossing_weight"] * (objective.get("crossings_anneal") or 0),
        "relations": objective["relations"] + 1000.0 * objective["hard"],
        "total": total(objective),
    }


# --- the race's placement ------------------------------------------------------------------


def footprint_ats(path):
    """The (x, y, angle) of every footprint of a .kicad_pcb in board order
    (the footprint's first `(at ...)`), read without KiCad."""
    text = Path(path).read_text()
    result = []
    position = 0
    while True:
        start = text.find("(footprint ", position)
        if start < 0:
            break
        at = text.find("(at ", start)
        end = text.find(")", at)
        values = [float(v) for v in text[at + 4:end].split()]
        result.append((values[0], values[1], values[2] if len(values) > 2 else 0.0))
        position = end
    return result


def race_seed(trace, board_path):
    """The traced seed whose final poses are the board's (the layout's race
    winner), or None."""
    ats = {(round(x, 2), round(y, 2)) for x, y, _ in footprint_ats(board_path)}
    for seed in trace["seeds"]:
        final = seed["stages"].index("final")
        # Every movable part's origin is a footprint's of the board.
        if all((round(part["poses"][final][0], 2), round(part["poses"][final][1], 2)) in ats
               for part in seed["parts"]):
            return seed.get("seed")
    return None


# --- the runs ---------------------------------------------------------------------------------


def unit_parts(name):
    """`<board>-s<seed>-<variant>` as (board, seed, variant)."""
    match = re.fullmatch(r"(.+)-s(\d+)-([a-z-]+)", name)
    if match is None:
        raise SystemExit(f"{name}: not <board>-s<seed>-<variant>")
    return match.group(1), int(match.group(2)), match.group(3)


def unit_command(arguments, name):
    board, seed, variant = unit_parts(name)
    inputs = Path(arguments.inputs).resolve()
    source = inputs / f"{board}-s{seed}"
    board_id = next((source / "source").glob("*.kicad_pcb")).stem
    out = Path(arguments.out).resolve() / "runs" / name
    designer = (inputs / f"{board}.designer").read_text().strip()
    env = dict(os.environ, RAYON_NUM_THREADS=str(arguments.threads), PCB_WORK_RATE="3.15e6,150e6")
    if variant != "off":
        env["PCB_PLACER_TRACE"] = str(out / "trace.json")
        env["PCB_PLACER_DESIGNER"] = designer
    env.update(VARIANTS[variant])
    if variant not in ("ours", "off"):
        env["PCB_PLACER_START"] = designer
    if variant == "global-active":
        env["PCB_PLACER_START_PARTS"] = str(inputs / f"{board}.active.json")
    router = source / "router.json"
    command = [str(Path(arguments.binary).resolve()), "place-kicad-board", str(source / "source"), board_id,
               str(out / "placed"), str(source / "layout.json"), "--router",
               str(router) if router.exists() else "auto"]
    return out, command, env


def run(arguments):
    queue = [name for name in arguments.units if not (Path(arguments.out) / "runs" / name / "unit.json").exists()]
    running = {}
    while queue or running:
        for name, (process, log, out, started) in list(running.items()):
            waited = os.wait4(process.pid, os.WNOHANG)
            if waited[0] == 0:
                continue
            log.close()
            usage = waited[2]
            code = os.waitstatus_to_exitcode(waited[1])
            (out / "unit.json").write_text(json.dumps({
                "exit": code, "wall_seconds": round(time.time() - started, 1),
                "cpu_seconds": round(usage.ru_utime + usage.ru_stime, 1),
                "peak_rss_mb": round(usage.ru_maxrss / 1024)}))
            print(time.strftime("%H:%M:%S"), "done", name, code, flush=True)
            del running[name]
        while queue and len(running) < arguments.slots:
            name = queue.pop(0)
            out, command, env = unit_command(arguments, name)
            out.mkdir(parents=True, exist_ok=True)
            log = open(out / "place.log", "w")
            process = subprocess.Popen(command, cwd=ROOT, stdout=log, stderr=subprocess.STDOUT, env=env)
            running[name] = (process, log, out, time.time())
            print(time.strftime("%H:%M:%S"), "start", name, flush=True)
        time.sleep(5)


# --- the first routes ------------------------------------------------------------------------


BOARDS = {
    "video": "Huaqiu-Electronics__ecad-viewer__video",
    "SNSP-CPU-01": "stonedDiscord__nonSNES__SNSP-CPU-01",
    "4in1": "OpenDrone-hw__OpenESC-30x30__4in1",
    "revC3": "GlasgowEmbedded__glasgow__glasgow__revC3",
}


def route_unit(arguments, name):
    """Writes the partialcopy.py unit of a first route and returns its
    command: `<board>-designer` (every movable part pinned at the
    designer's pose), `<board>-ours` (nothing pinned: the layout's own
    placement), `<board>-s<seed>-<variant>` (every movable part pinned at
    that place check's pose); each at layout seed 1."""
    import partialcopy  # noqa: PLC0415  (its extraction needs KiCad)
    out = Path(arguments.out).resolve() / "firstroute"
    if name.endswith(("-designer", "-ours")):
        short, rule = name.rsplit("-", 1)
    else:
        short, rule = unit_parts(name)[0], name
    board = BOARDS[short]
    task = partialcopy.v7_tasks()[board]
    designer = partialcopy.extracted(partialcopy.designer_board(task), out / "extract" / "designer" / f"{board}.json")
    fixed = (json.loads((partialcopy.TASKS.parent / task["constraints"]).read_text()) if task.get("constraints")
             else {}).get("fixed", [])
    movable = {partialcopy.key(f) for f in partialcopy.movable(partialcopy.numbered(designer["footprints"]), fixed)}
    if rule == "designer":
        source = designer
    elif rule == "ours":
        source = None
    else:
        placed = next((Path(arguments.out).resolve() / "runs" / name / "placed").glob("*.kicad_pcb"))
        source = partialcopy.extracted(placed, out / "extract" / f"{name}.json")
    footprints = ([] if source is None else
                  [f for f in partialcopy.numbered(source["footprints"]) if partialcopy.key(f) in movable])
    if source is not None and len(footprints) != len(movable):
        raise SystemExit(f"{name}: {len(footprints)} of {len(movable)} movable parts found")
    partialcopy.write_unit(out, task, rule, footprints, source or designer)
    return out, partialcopy.unit_command(out, board, f"{rule}-s1", arguments.binary)


def route(arguments):
    """The first routes of `arguments.units`, at most `--slots` at once
    (run.py and the ledger), restartable (`.done`)."""
    queue = list(arguments.units)
    running = {}

    def alive(pid):
        try:
            os.kill(pid, 0)
            return True
        except OSError:
            return False

    while queue or running:
        for name, (process, log, done) in list(running.items()):
            if process.poll() is not None:
                log.close()
                done.write_text(f"{process.returncode}\n")
                print(time.strftime("%H:%M:%S"), "done", name, process.returncode, flush=True)
                del running[name]
        busy = sum(1 for pid in arguments.busy if alive(pid))
        while queue and len(running) + busy < arguments.slots:
            name = queue.pop(0)
            out, command = route_unit(arguments, name)
            label = Path(command[command.index("--tasks") - 1]).name
            done = out / "runs" / f"{label}.done"
            if done.exists():
                continue
            (out / "runs").mkdir(parents=True, exist_ok=True)
            log = open(out / "runs" / f"{label}.out", "w")
            running[name] = (subprocess.Popen(command, cwd=ROOT, stdout=log, stderr=subprocess.STDOUT), log, done)
            print(time.strftime("%H:%M:%S"), "start", name, flush=True)
        time.sleep(15)


def route_results(out):
    """{run label: (first-route open terminals, KiCad unconnected, CPU seconds)}."""
    result = {}
    runs = out / "firstroute" / "runs"
    if not runs.exists():
        return result
    for done in sorted(runs.glob("*.done")):
        label = done.stem
        reports = list((runs / label).glob("*/layout/board-layout.json"))
        rows = list((runs / label).glob("results.json"))
        first = json.loads(reports[0].read_text()).get("first_unconnected_terminals") if reports else None
        row = (json.loads(rows[0].read_text()) or [{}])[0] if rows else {}
        result[label] = {"first_open": first, "unconnected": (row.get("native") or {}).get("unconnected"),
                         "cpu_seconds": row.get("cpu_seconds")}
    return result


# --- the tables -------------------------------------------------------------------------------


def fmt(value, digits=2):
    if value is None:
        return "-"
    return f"{value:.{digits}f}"


def table(arguments):
    out = Path(arguments.out)
    inputs = Path(arguments.inputs)
    results = {}
    for directory in sorted((out / "runs").iterdir()):
        trace_path = directory / "trace.json"
        if not trace_path.exists():
            continue
        board, seed, variant = unit_parts(directory.name)
        active = set(json.loads((inputs / f"{board}.active.json").read_text()))
        trace = json.loads(trace_path.read_text())
        seeds = [measure_seed(entry, active) for entry in trace["seeds"]]
        race_boards = sorted((inputs / f"{board}-s{seed}").glob("race-*.kicad_pcb"))
        unit = json.loads((directory / "unit.json").read_text()) if (directory / "unit.json").exists() else {}
        results[directory.name] = {
            "board": board, "seed": seed, "variant": variant, "seeds": seeds,
            "race_seed": race_seed(trace, race_boards[0]) if race_boards and variant == "ours" else None,
            "unit": unit,
        }
    routes = route_results(out)
    (out / "stages.json").write_text(json.dumps({"units": results, "first_routes": routes}, indent=1))
    lines = ["| unit | seed | stage | displacement mm (share) | raw mm | pairwise | neighbours | "
             "hpwl 2 / 3-8 / 9+ / all, ours over designer | objective / designer's |", "| --- " * 9 + "|"]
    for name, result in results.items():
        for seed in result["seeds"]:
            tags = []
            if seed["kept"]:
                tags.append("kept")
            if result["race_seed"] is not None and seed["seed"] == result["race_seed"]:
                tags.append("race")
            for stage in STAGES:
                entry = seed["stages"].get(stage)
                if entry is None:
                    continue
                hpwl = " / ".join(fmt(v) for v in entry.get("hpwl", [])) + f" / {fmt(entry.get('hpwl_all'))}"
                ratio = (entry["objective"] / seed["designer_objective"]
                         if seed.get("designer_objective") else None)
                lines.append(f"| {name} | {seed['seed']}{' (' + ', '.join(tags) + ')' if tags else ''} | {stage} | "
                             f"{fmt(entry['displacement_mm'], 1)} ({fmt(entry['displacement_share'])}) | "
                             f"{fmt(entry['raw_mm'], 1)} | {fmt(entry['pairwise'])} | {fmt(entry['neighbours'])} | "
                             f"{hpwl} | {fmt(ratio)} |")
    (out / "tables.md").write_text("\n".join(summary(results, routes) + ["", "## Every seed and stage", ""] + lines)
                                   + "\n")
    print(f"{len(results)} units -> {out / 'tables.md'}")


def agreement(seeds, stage):
    """The mean pairwise rank correlation of the active parts' arrangement
    between every two seeds at a stage: how much of the arrangement the
    pipeline decides rather than its noise."""
    values = [pairwise(a["centers"][stage], b["centers"][stage])
              for i, a in enumerate(seeds) for b in seeds[i + 1:]]
    values = [v for v in values if v is not None]
    return statistics.mean(values) if values else None


def kept(result):
    return next(seed for seed in result["seeds"] if seed["kept"])


def cell(entry):
    """`share / pairwise` of a stage's measures."""
    if entry is None:
        return "-"
    return f"{fmt(entry['displacement_share'])} / {fmt(entry['pairwise'])}"


def summary(results, routes):
    """The decision tables: per board and seed our stages (the kept seed,
    the race's where known, the mean over the seeds), the warm starts'
    fixed points, the objective's split, the first routes."""
    lines = ["## Our pipeline: the active parts against the designer's, per stage", "",
             "displacement share / pairwise rank correlation (0 / 1: the designer's arrangement; "
             "a shuffle about 1.4 / 0)", "",
             "| board | seed | placement | global | annealed | legal | final | hpwl all, final |",
             "| --- | --- | --- | --- | --- | --- | --- | ---: |"]
    for name, result in results.items():
        if result["variant"] != "ours":
            continue
        rows = [("kept", kept(result))]
        race = next((seed for seed in result["seeds"] if seed["seed"] == result["race_seed"]), None)
        if race is not None:
            rows.append(("race", race))
        for label, seed in rows:
            lines.append(f"| {result['board']} | {result['seed']} | {label} (seed {seed['seed']}) | "
                         + " | ".join(cell(seed["stages"].get(stage)) for stage in STAGES)
                         + f" | {fmt(seed['stages']['final'].get('hpwl_all'))} |")
        means = []
        for stage in STAGES:
            shares = [seed["stages"][stage]["displacement_share"] for seed in result["seeds"]]
            pairs = [seed["stages"][stage]["pairwise"] for seed in result["seeds"]]
            means.append(f"{fmt(statistics.mean(shares))} / {fmt(statistics.mean(pairs))}")
        lines.append(f"| {result['board']} | {result['seed']} | mean of {len(result['seeds'])} | "
                     + " | ".join(means) + " | |")
        lines.append(f"| {result['board']} | {result['seed']} | seeds against each other (pairwise) | "
                     + " | ".join(fmt(agreement(result["seeds"], stage)) for stage in STAGES) + " | |")
    lines += ["", "## Warm starts from the designer's poses (kept seed)", "",
              "the stage's own result, then the final placement: share / pairwise; the final objective "
              "against ours (the kept seed's final, same board and seed)", "",
              "| board | seed | start | the stage's result | final | final objective / ours |",
              "| --- | --- | --- | --- | --- | ---: |"]
    own = {"global-all": "global", "global-active": "global", "anneal": "annealed", "legalize": "legal"}
    for name, result in results.items():
        if result["variant"] not in own:
            continue
        ours = results.get(f"{result['board']}-s{result['seed']}-ours")
        seed = kept(result)
        ratio = (seed["stages"]["final"]["objective"] / kept(ours)["stages"]["final"]["objective"]
                 if ours else None)
        lines.append(f"| {result['board']} | {result['seed']} | {result['variant']} | "
                     f"{cell(seed['stages'].get(own[result['variant']]))} | {cell(seed['stages'].get('final'))} | "
                     f"{fmt(ratio)} |")
    lines += ["", "## The objective split (kept seed, the placement's level, mm of wire)", "",
              "| board | seed | arrangement | wire 2 | wire 3-8 | wire 9+ | overlap x 400 (parts) | "
              "crossings x weight | relations | total |", "| --- " * 10 + "|"]
    for name, result in results.items():
        rows = []
        if result["variant"] == "ours":
            seed = kept(result)
            rows = [("ours, final", seed["terms"]["final"]), ("designer's poses", seed["terms"].get("designer"))]
        elif result["variant"] == "legalize":
            seed = kept(result)
            rows = [("designer's, legalised", seed["terms"]["legal"]),
                    ("designer's, legalised and refined", seed["terms"]["final"])]
        elif result["variant"] == "anneal":
            rows = [("designer's, annealed, final", kept(result)["terms"]["final"])]
        for label, terms in rows:
            if terms is None:
                continue
            lines.append(f"| {result['board']} | {result['seed']} | {label} | {terms['wire_2']:.0f} | "
                         f"{terms['wire_3_8']:.0f} | {terms['wire_9plus']:.0f} | {terms['overlap']:.0f} "
                         f"({terms['overlapping']}) | {terms['crossings']:.0f} | {terms['relations']:.0f} | "
                         f"{terms['total']:.0f} |")
    if routes:
        lines += ["", "## First routes (layout seed 1, `stop_after_first_route`)", "",
                  "| run | open terminals | KiCad unconnected | CPU s |", "| --- | ---: | ---: | ---: |"]
        lines += [f"| {label} | {r['first_open']} | {r['unconnected']} | {r['cpu_seconds']} |"
                  for label, r in routes.items()]
    cpu = [result["unit"].get("cpu_seconds") or 0 for result in results.values()]
    lines += ["", f"Place checks: {len(cpu)} units, {sum(cpu):.0f} CPU seconds."]
    return lines


def main():
    parser = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    commands = parser.add_subparsers(dest="command", required=True)
    runner = commands.add_parser("run")
    runner.add_argument("--inputs", required=True)
    runner.add_argument("--out", required=True)
    runner.add_argument("--binary", required=True)
    runner.add_argument("--slots", type=int, default=3)
    runner.add_argument("--threads", type=int, default=2)
    runner.add_argument("units", nargs="+")
    router = commands.add_parser("route")
    router.add_argument("--inputs", required=True)
    router.add_argument("--out", required=True)
    router.add_argument("--binary", required=True)
    router.add_argument("--slots", type=int, default=2)
    router.add_argument("--busy", type=int, action="append", default=[],
                        help="a process (pid) of runs started elsewhere: it takes a slot while it lives")
    router.add_argument("units", nargs="+")
    tabler = commands.add_parser("table")
    tabler.add_argument("--inputs", required=True)
    tabler.add_argument("--out", required=True)
    arguments = parser.parse_args()
    {"run": run, "route": route, "table": table}[arguments.command](arguments)


if __name__ == "__main__":
    main()
