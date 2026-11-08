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
    stages.py passives --out DIR [UNIT...]
    stages.py stacks --out DIR [UNIT...]

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
- `debug`: `ours` with the placer's debug lines (`PCB_PLACER_DEBUG`);
- `global-all`: the global placement from the designer's poses;
- `global-active`: the same for the active parts, the passives from our start;
- `anneal`: no global placement, each level's anneal from the designer's poses;
- `legalize`: no global placement and no anneal, each level's legaliser
  seats the designer's poses, then the refinement (its `legal` stage is
  the minimal legalisation, its `final` the refinement from there);
- `off`: `ours` without the trace (the identity check of the dump);
- `glue-multi`: `ours` with the stacks of parts with three nets or more
  glued into banks in the descent (`PCB_PLACER_STACKS`; the variants
  `bank`, `glue` of 2026-11-08 are gone with their switch values);
- `<variant>-noanneal`: the variant without the anneal
  (`PCB_PLACER_SKIP_ANNEAL`: each level's legaliser seats the global
  placement's poses).

`run` writes `<out>/runs/<unit>/` (the placed project, `trace.json`,
`place.log`, `unit.json`: exit code, wall and CPU seconds). `route` lays
out first routes with partialcopy.py's machinery (`<out>/firstroute`, the
layout at seed 1 with `stop_after_first_route` through run.py and the
ledger): `<board>-designer` (every movable part pinned at the designer's
pose), `<board>-ours` (nothing pinned) and `<board>-s<seed>-<variant>`
(every movable part pinned at that place check's kept pose; with
`@<stage>`, at the kept seed's poses after that stage). `table`
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

`passives` measures the passives around a held skeleton (the race over
skeletons' phase 2; docs/placer.md, "The passives' descent around a held
skeleton (2026-11-08)"), per traced unit and seed placed by skeleton: the
held parts are the movable parts that do not move from the passives'
`start` to `final`, the passives the others. Per stage (`start`, `global`,
`annealed`, `legal`, `final`): how many passives have their centre inside
a held body on their side (`inside`); the passives' displacement between
the stages (mean mm, and of those inside after the descent); the placer's
objective by term at each stage; the passives' descent (iterations, the
overflow at the stop); and the relaxation level's halos, tight bodies and
courtyard spacing. Writes `<out>/passives.md` and `<out>/passives.json`.

`stacks` measures the netlist's stacks (docs/placer.md, "The netlist's
stacks are lost in the descent, not at the start (2026-11-08)"), per
traced unit, seed and phase (the skeleton's solve, the whole or the
passives'): the coincident stacks at the solve's start (centres within half
the smaller half extent, any nets: how many, how large, how many on the
same nets) and the groups of parts on the same nets (the trace's `nets`),
with their arrangement at the phase's later stages (`shape`: the rms
distance from the centroid over the radius of their area's disc, the
anisotropy, row, cluster or scattered; the centroid's move and the
members' distance from the start's centroid), their room at the level and
the levels whose walk left a member without room. Writes `<out>/stacks.md`
and `<out>/stacks.json`.
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

sys.path.insert(0, str(Path(__file__).resolve().parents[2] / "benchmarks"))
import harness_common  # noqa: E402

ROOT = Path(__file__).resolve().parents[2]
VARIANTS = {
    "ours": {},
    "off": {},
    # `ours` with the placer's debug lines in place.log (the skeleton's
    # parts without a place, the levels' walk).
    "debug": {"PCB_PLACER_DEBUG": "1"},
    # The multi-net stacks glued into banks in the descent (the
    # measurement's switch; docs/placer.md, "The netlist's stacks are lost
    # in the descent, not at the start (2026-11-08)").
    "glue-multi": {"PCB_PLACER_STACKS": "glue-multi"},
    "global-all": {"PCB_PLACER_START_STAGE": "global"},
    "global-active": {"PCB_PLACER_START_STAGE": "global"},
    "anneal": {"PCB_PLACER_START_STAGE": "anneal"},
    "legalize": {"PCB_PLACER_START_STAGE": "legalize"},
}
# The variants that start from the designer's poses (`PCB_PLACER_START`); the
# others start as the placer does. Until 2026-11-08 every variant but `ours`
# and `off` did: `netnoise` and `netpush` (2026-11-07, their switches gone
# since) ran from the designer's poses.
DESIGNER_STARTS = ("global-all", "global-active", "anneal", "legalize")
# `<variant>-noanneal`: the variant without the anneal (`PCB_PLACER_SKIP_ANNEAL`).
NO_ANNEAL = "-noanneal"
STAGES = ("global", "annealed", "legal", "final")
# Every stage a trace may hold, in the tables' order: the kept skeleton's
# (the race over skeletons) and the global placement's start before ours.
ALL_STAGES = ("skeleton_start", "skeleton_global", "skeleton_final", "start") + STAGES
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
    result = {"seed": seed.get("seed"), "kept": seed.get("kept"), "stages": {}, "centers": {},
              "global_history": seed.get("global_history")}
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


# --- the passives around a held skeleton ------------------------------------------------------


PASSIVE_STAGES = ("start", "global", "annealed", "legal", "final")


def body_box(part, stage):
    """The part's body at a stage as (centre, half extents), its rotation's
    bounding box."""
    width, height = part["body"]
    angle = math.radians(part["angles"][stage])
    c, s = abs(math.cos(angle)), abs(math.sin(angle))
    return part["centers"][stage], (c * width / 2 + s * height / 2, s * width / 2 + c * height / 2)


def sides_meet(a, b):
    return a == b or "Both" in (a, b)


def held_parts(seed):
    """The indices of the seed's parts held in the passives' placement (the
    skeleton's: they do not move from `start` to `final`), or None where
    the seed was not placed by skeleton."""
    stages = seed["stages"]
    if "skeleton_final" not in stages or not all(stage in stages for stage in PASSIVE_STAGES):
        return None
    indices = [stages.index(stage) for stage in PASSIVE_STAGES]
    return {number for number, part in enumerate(seed["parts"])
            if all(math.dist(part["centers"][indices[0]], part["centers"][k]) < 1e-6 for k in indices[1:])}


def inside_held(seed, held, stage):
    """The passives (indices) whose centre lies inside a held body on their
    side at `stage` (a stage name)."""
    k = seed["stages"].index(stage)
    boxes = [(body_box(seed["parts"][h], k), seed["parts"][h]["side"]) for h in held]
    result = []
    for number, part in enumerate(seed["parts"]):
        if number in held:
            continue
        x, y = part["centers"][k]
        for ((cx, cy), (hx, hy)), side in boxes:
            if sides_meet(side, part["side"]) and abs(x - cx) < hx and abs(y - cy) < hy:
                result.append(number)
                break
    return result


def level_name(relaxation):
    """The relaxation level in words: halos, spacing, tight bodies, courtyards."""
    words = [f"halo {relaxation['halo_scale']:g}/{relaxation['small_halo_scale']:g}",
             f"spacing {relaxation['spacing']:g}"]
    if relaxation.get("tight"):
        words.append("tight")
    if relaxation.get("courtyard_spacing"):
        words.append("courtyards")
    return ", ".join(words)


def measure_passives(seed):
    """The passives around the seed's held skeleton (`passives`'s measures),
    or None where it was not placed by skeleton."""
    held = held_parts(seed)
    if held is None:
        return None
    stages = seed["stages"]
    passive = [n for n in range(len(seed["parts"])) if n not in held]
    inside = {stage: inside_held(seed, held, stage) for stage in PASSIVE_STAGES}

    def moved(numbers, first, second):
        a, b = stages.index(first), stages.index(second)
        distances = [math.dist(seed["parts"][n]["centers"][a], seed["parts"][n]["centers"][b]) for n in numbers]
        return statistics.mean(distances) if distances else None
    trapped = inside["global"]
    free = [n for n in passive if n not in set(trapped)]
    steps = (seed.get("global_history") or {}).get("global") or []
    terms = {stage: split(seed["objective"][stage]) for stage in PASSIVE_STAGES if stage in seed["objective"]}
    return {
        "seed": seed.get("seed"), "kept": seed.get("kept"), "held": len(held), "passives": len(passive),
        "inside": {stage: len(numbers) for stage, numbers in inside.items()},
        "inside_by_part": _by_held(seed, held, inside["global"], stages.index("global")),
        "moved": {f"{a}-{b}": moved(passive, a, b) for a, b in
                  (("global", "annealed"), ("annealed", "legal"), ("legal", "final"), ("global", "final"))},
        "moved_trapped": moved(trapped, "global", "final"), "moved_free": moved(free, "global", "final"),
        "terms": terms,
        "iterations": int(steps[-1][0]) + 1 if steps else None,
        "overflow": steps[-1][1] if steps else None,
        "lambda_growth": steps[-1][2] / steps[0][2] if steps and steps[0][2] > 0 else None,
        "level": level_name(seed["relaxation"]), "unplaced": seed.get("unplaced"), "illegal": seed.get("illegal"),
    }


def _by_held(seed, held, trapped, stage):
    """How many of the trapped passives lie in each held part, the most first."""
    counts = {}
    for n in trapped:
        x, y = seed["parts"][n]["centers"][stage]
        for h in held:
            (cx, cy), (hx, hy) = body_box(seed["parts"][h], stage)
            if (sides_meet(seed["parts"][h]["side"], seed["parts"][n]["side"])
                    and abs(x - cx) < hx and abs(y - cy) < hy):
                reference = seed["parts"][h]["reference"]
                counts[reference] = counts.get(reference, 0) + 1
                break
    return dict(sorted(counts.items(), key=lambda item: -item[1]))


def passives_table(arguments):
    out = Path(arguments.out)
    names = arguments.units or sorted(p.parent.name for p in (out / "runs").glob("*/trace.json"))
    results = {}
    for name in names:
        trace = json.loads((out / "runs" / name / "trace.json").read_text())
        unit = json.loads((out / "runs" / name / "unit.json").read_text()) if (
            out / "runs" / name / "unit.json").exists() else {}
        seeds = [measure_passives(seed) for seed in trace["seeds"]]
        results[name] = {"unit": unit, "seeds": [seed for seed in seeds if seed is not None],
                         "plain": sum(seed is None for seed in seeds),
                         # Every seed in the race's order: by skeleton or plain, its level, its wire.
                         "race": [{"seed": seed.get("seed"), "skeleton": measured is not None,
                                   "level": level_name(seed["relaxation"]),
                                   "wirelength": seed["wirelength"][seed["stages"].index("final")],
                                   "illegal": seed.get("illegal"), "unplaced": seed.get("unplaced")}
                                  for seed, measured in zip(trace["seeds"], seeds)]}
    (out / "passives.json").write_text(json.dumps(results, indent=1))
    lines = ["# The passives around the held skeleton", "",
             "Kept seed; inside: passives whose centre lies in a held body on their side, at the passives' "
             "start / after the descent / the anneal / the legaliser / the refinement; moved: the passives' mean "
             "displacement in mm from the descent's spots to the final ones (all / those inside after the "
             "descent / the others); the objective (the anneal's, at the level) after the descent and at the end.",
             "", "| unit | seeds by skeleton | held / passives | inside: start / global / annealed / legal / final "
             "| descent: iterations, overflow at the stop | moved all / inside / outside | level | "
             "objective global -> final (relations, crossings) | CPU s |", "| --- " * 10 + "|"]
    for name, result in results.items():
        seeds = result["seeds"]
        skeleton_seeds = f"{len(seeds)} of {len(seeds) + result['plain']}"
        kept_seed = next((seed for seed in seeds if seed["kept"]), None)
        cpu = result["unit"].get("cpu_seconds", "-")
        if kept_seed is None:
            lines.append(f"| {name} | {skeleton_seeds} (kept: plain) | - | - | - | - | - | - | {cpu} |")
            continue
        k = kept_seed
        inside = " / ".join(str(k["inside"][stage]) for stage in PASSIVE_STAGES)
        global_terms, final_terms = k["terms"].get("global"), k["terms"].get("final")
        objective = (f"{global_terms['total']:.0f} -> {final_terms['total']:.0f} "
                     f"({global_terms['relations']:.0f} -> {final_terms['relations']:.0f}, "
                     f"{global_terms['crossings']:.0f} -> {final_terms['crossings']:.0f})"
                     if global_terms and final_terms else "-")
        lines.append(f"| {name} | {skeleton_seeds} | {k['held']} / {k['passives']} | {inside} | "
                     f"{k['iterations']}, {fmt(k['overflow'], 3)} | {fmt(k['moved']['global-final'])} / "
                     f"{fmt(k['moved_trapped'])} / {fmt(k['moved_free'])} | {k['level']} | {objective} | {cpu} |")
    lines += ["", "Every seed placed by skeleton (inside after the descent, at the end; the descent's iterations "
              "and overflow at the stop; the level):", ""]
    for name, result in results.items():
        cells = [f"s{seed['seed']}{'*' if seed['kept'] else ''}: {seed['inside']['global']} -> "
                 f"{seed['inside']['final']}, {seed['iterations']} it {fmt(seed['overflow'], 2)}, "
                 f"{seed['level']}" for seed in result["seeds"]]
        lines.append(f"- {name}: " + ("; ".join(cells) if cells else "no seed by skeleton"))
    lines += ["", "The seeds in the placer's order (the first kept): skeleton or plain, level, final wire mm:", ""]
    for name, result in results.items():
        lines.append(f"- {name}: " + "; ".join(
            f"s{seed['seed']} {'skeleton' if seed['skeleton'] else 'plain'} ({seed['level']}) "
            f"{seed['wirelength']:.0f}" for seed in result["race"]))
    lines += ["", "The held parts with the most passives inside after the descent (kept seed):", ""]
    for name, result in results.items():
        kept_seed = next((seed for seed in result["seeds"] if seed["kept"]), None)
        if kept_seed and kept_seed["inside_by_part"]:
            top = list(kept_seed["inside_by_part"].items())[:6]
            lines.append(f"- {name}: " + ", ".join(f"{ref} {count}" for ref, count in top))
    (out / "passives.md").write_text("\n".join(lines) + "\n")
    print("\n".join(lines))


# --- the netlist's stacks -----------------------------------------------------------------------


# The stages a netlist solve starts and the stages after it, per phase.
STACK_PHASES = (("skeleton_start", ("skeleton_global", "skeleton_annealed", "skeleton_legal", "skeleton_final")),
                ("start", ("global", "annealed", "legal", "final")))
# Two parts' centres coincide where they lie within this share of the
# smaller half extent of their bodies on both axes.
COINCIDENT = 0.5


def find_stacks(seed, stage, movable):
    """The stacks at a stage: groups (lists of part indices, by chains) of
    the `movable` parts on one side whose centres coincide (`COINCIDENT`)."""
    k = seed["stages"].index(stage)
    parts = seed["parts"]
    parent = {n: n for n in movable}

    def find(n):
        while parent[n] != n:
            parent[n] = parent[parent[n]]
            n = parent[n]
        return n
    boxes = {n: body_box(parts[n], k) for n in movable}
    order = sorted(movable, key=lambda n: boxes[n][0][0])
    reach = max((boxes[n][1][0] for n in movable), default=0.0)
    for position, a in enumerate(order):
        (ax, ay), (ahx, ahy) = boxes[a]
        for b in order[position + 1:]:
            (bx, by), (bhx, bhy) = boxes[b]
            if bx - ax > reach:
                break
            if not sides_meet(parts[a]["side"], parts[b]["side"]):
                continue
            if abs(ax - bx) < COINCIDENT * min(ahx, bhx) and abs(ay - by) < COINCIDENT * min(ahy, bhy):
                parent[find(a)] = find(b)
    groups = {}
    for n in movable:
        groups.setdefault(find(n), []).append(n)
    return sorted((sorted(g) for g in groups.values() if len(g) > 1), key=lambda g: (-len(g), g))


def stack_nets(parts, members, nets):
    """How the members' nets relate: `same` (every member on the same set
    of nets) or the nets not every member has, with their pin counts."""
    sets = [set(parts[n].get("nets") or []) for n in members]
    if not any(sets):
        return {"same": None, "differing": []}
    common = set.intersection(*sets)
    differing = sorted(set.union(*sets) - common)
    return {"same": not differing, "common": len(common),
            "differing": [(nets[d]["name"] if nets else d, nets[d]["pins"] if nets else None) for d in differing]}


def shape(parts, members, k, start_centroid):
    """The members' arrangement at stage index k: the rms distance from
    their centroid over the radius of a disc of their bodies' area (a
    packed cluster about 0.7), the anisotropy (the principal axes' ratio
    of the spread: a row is large), the centroid's distance from the
    start's, and the mean member displacement from it."""
    points = [parts[n]["centers"][k] for n in members]
    cx = statistics.mean(p[0] for p in points)
    cy = statistics.mean(p[1] for p in points)
    rms = math.sqrt(statistics.mean((p[0] - cx) ** 2 + (p[1] - cy) ** 2 for p in points))
    area = sum(parts[n]["body"][0] * parts[n]["body"][1] for n in members)
    packed = math.sqrt(area / math.pi)
    sxx = statistics.mean((p[0] - cx) ** 2 for p in points)
    syy = statistics.mean((p[1] - cy) ** 2 for p in points)
    sxy = statistics.mean((p[0] - cx) * (p[1] - cy) for p in points)
    half = (sxx + syy) / 2
    root = math.sqrt(max(half * half - (sxx * syy - sxy * sxy), 0.0))
    l1, l2 = half + root, max(half - root, 1e-12)
    from_start = statistics.mean(math.dist(p, start_centroid) for p in points)
    return {"spread": rms / packed if packed > 0 else None, "anisotropy": math.sqrt(l1 / l2),
            "centroid_moved": math.dist((cx, cy), start_centroid), "member_from_start": from_start}


def arrangement_word(entry):
    if entry["spread"] is None:
        return "-"
    if entry["spread"] > 2.0:
        return "scattered"
    if entry["anisotropy"] > 3.0:
        return "row"
    return "cluster"


def measure_stacks(seed, nets):
    """Per phase of the seed (the skeleton's, the whole or the passives'),
    the stacks at the solve's start and what the stages after it make of
    them, and the levels whose walk left a member without room."""
    parts = seed["parts"]
    held = held_parts(seed) or set()
    walk = seed.get("walk") or []
    skeleton = seed.get("skeleton") or {}
    result = []
    for start, later in STACK_PHASES:
        if start not in seed["stages"]:
            continue
        k0 = seed["stages"].index(start)
        if start == "start":
            movable = [n for n in range(len(parts)) if n not in held]
            phase_walk = walk
        else:
            # The skeleton's parts: those that move in its phase.
            last = seed["stages"].index(later[-1])
            movable = [n for n in range(len(parts))
                       if math.dist(parts[n]["centers"][k0], parts[n]["centers"][last]) > 1e-9 or n in held]
            phase_walk = skeleton.get("skeleton_walk") or []
        # The parts on the same nets (two or more) and side: the netlist's
        # stacks by their nets, whatever the start made of them.
        groups = {}
        for n in movable:
            key = tuple(sorted(set(parts[n].get("nets") or [])))
            if len(key) >= 2:
                groups.setdefault((key, parts[n]["side"]), []).append(n)
        coincident = find_stacks(seed, start, movable)
        census = {"stacks": len(coincident), "sizes": [len(g) for g in coincident],
                  "same_nets": sum(bool(stack_nets(parts, g, nets).get("same")) for g in coincident)}
        result.append({"phase": start, "census": census})
        for members in sorted((g for g in groups.values() if len(g) > 1), key=lambda g: (-len(g), g)):
            refs = [parts[n]["reference"] for n in members]
            points = [parts[n]["centers"][k0] for n in members]
            centroid = (statistics.mean(p[0] for p in points), statistics.mean(p[1] for p in points))
            stages = {stage: shape(parts, members, seed["stages"].index(stage), centroid)
                      for stage in later if stage in seed["stages"]}
            left = [(entry[0], sorted(set(refs) & set(entry[1]))) for entry in phase_walk
                    if set(refs) & set(entry[1])]
            halo = max(parts[n]["halo"] for n in members)
            spacing = seed["relaxation"]["spacing"]
            room = sum((parts[n]["body"][0] + 2 * parts[n]["halo"] + spacing)
                       * (parts[n]["body"][1] + 2 * parts[n]["halo"] + spacing) for n in members)
            at_start = shape(parts, members, k0, centroid)
            result.append({"phase": start, "members": refs, "body": parts[members[0]]["body"],
                           "nets": stack_nets(parts, members, nets), "stages": stages, "left_out": left,
                           "room_mm2": room, "halo": halo, "spread_at_start": at_start})
    return result


def stacks_table(arguments):
    out = Path(arguments.out)
    names = arguments.units or sorted(p.parent.name for p in (out / "runs").glob("*/trace.json"))
    results = {}
    for name in names:
        trace = json.loads((out / "runs" / name / "trace.json").read_text())
        nets = trace.get("nets")
        results[name] = [{"seed": seed.get("seed"), "kept": seed.get("kept"),
                          "placed": ("skeleton" if held_parts(seed) is not None else "plain"),
                          "level": level_name(seed["relaxation"]),
                          "walk": [(entry[0], len(entry[1])) for entry in seed.get("walk") or []],
                          "stacks": measure_stacks(seed, nets)} for seed in trace["seeds"]]
    (out / "stacks.json").write_text(json.dumps(results, indent=1))
    lines = ["# The netlist's stacks", "",
             "Per unit and seed (* kept): how it was placed and its level; per phase (the skeleton's solve, the "
             "whole or the passives'), the stacks at the solve's start (centres within half the smaller half "
             "extent): members, whether they share every net (else the nets not all have, with pins), and at "
             "the final stage of the phase the arrangement (spread: rms distance over the radius of their "
             "area's disc, a packed cluster about 0.7; anisotropy; the word), the centroid's move and the mean "
             "member's distance from the start's centroid, mm; the levels whose walk left members out.", ""]
    for name, seeds in results.items():
        lines.append(f"## {name}")
        for seed in seeds:
            lines.append(f"- s{seed['seed']}{'*' if seed['kept'] else ''} {seed['placed']} ({seed['level']}), "
                         f"walk {seed['walk']}")
            for stack in seed["stacks"]:
                if "census" in stack:
                    census = stack["census"]
                    lines.append(f"  - {stack['phase']}: {census['stacks']} coincident stacks (sizes "
                                 f"{census['sizes']}), {census['same_nets']} on the same nets")
                    continue
                final = stack["stages"].get("final") or stack["stages"].get("skeleton_final") or {}
                nets = stack["nets"]
                relation = ("same nets" if nets.get("same") else
                            "differ: " + ", ".join(f"{n}({p})" for n, p in nets.get("differing", [])[:6]))
                members = " ".join(stack["members"][:12]) + (" ..." if len(stack["members"]) > 12 else "")
                word = arrangement_word(final) if final else "-"
                begin = stack["spread_at_start"]
                lines.append(f"  - {stack['phase']}: {len(stack['members'])} x {stack['body'][0]:g}x{stack['body'][1]:g} "
                             f"[{members}] {relation}; start spread {fmt(begin.get('spread'))}; final {word} (spread {fmt(final.get('spread'))}, "
                             f"aniso {fmt(final.get('anisotropy'), 1)}, centroid {fmt(final.get('centroid_moved'), 1)}, "
                             f"members {fmt(final.get('member_from_start'), 1)} mm); room {stack['room_mm2']:.0f} mm2; "
                             f"left out {stack['left_out'] or '-'}")
    (out / "stacks.md").write_text("\n".join(lines) + "\n")
    print("\n".join(lines))


# --- the runs ---------------------------------------------------------------------------------


def unit_parts(name):
    """`<board>-s<seed>-<variant>` as (board, seed, variant)."""
    match = re.fullmatch(r"(.+)-s(\d+)-([a-z-]+)", name)
    if match is None:
        raise SystemExit(f"{name}: not <board>-s<seed>-<variant>")
    return match.group(1), int(match.group(2)), match.group(3)


def unit_command(arguments, name):
    board, seed, variant = unit_parts(name)
    skip = variant.endswith(NO_ANNEAL)
    variant = variant.removesuffix(NO_ANNEAL)
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
    if skip:
        env["PCB_PLACER_SKIP_ANNEAL"] = "1"
    if variant in DESIGNER_STARTS:
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
    # Every place check reserves its expected peak in the machine's memory
    # ledger (`--expected-gb`), handed to the binary's process.
    ledger = harness_common.Ledger.from_env()
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
            key = harness_common.admit(ledger, arguments.expected_gb * 1024, f"stages.py: {name} (expected peak)",
                                       log=lambda line: print(time.strftime("%H:%M:%S"), line, flush=True))
            process = subprocess.Popen(command, cwd=ROOT, stdout=log, stderr=subprocess.STDOUT, env=env)
            ledger.move(key, process.pid)
            running[name] = (process, log, out, time.time())
            print(time.strftime("%H:%M:%S"), "start", name, flush=True)
        time.sleep(5)


# --- the first routes ------------------------------------------------------------------------


BOARDS = {
    "video": "Huaqiu-Electronics__ecad-viewer__video",
    "SNSP-CPU-01": "stonedDiscord__nonSNES__SNSP-CPU-01",
    "4in1": "OpenDrone-hw__OpenESC-30x30__4in1",
    "revC3": "GlasgowEmbedded__glasgow__glasgow__revC3",
    "4in1-mini": "OpenDrone-hw__OpenESC-20x20__4in1-mini",
    "DSP-ADAU1452": "ohdsp__DSP-ADAU1452__DSP-ADAU1452",
    "MegaDrive": "stonedDiscord__MegaDrive__MegaDrive",
    "poly_kybd_split72_left": "thpoll83__PolyKybd__poly_kybd_split72_left",
    "protoesp": "CRImier__MyKiCad__protoesp",
}


def route_unit(arguments, name):
    """Writes the partialcopy.py unit of a first route and returns its
    command: `<board>-designer` (every movable part pinned at the
    designer's pose), `<board>-ours` (nothing pinned: the layout's own
    placement), `<board>-s<seed>-<variant>` (every movable part pinned at
    that place check's pose; with `@<stage>`, at the kept seed's poses
    after that stage; with `@<stage>.<n>`, at traced seed n's: the race's
    other placements); each at layout seed 1."""
    import partialcopy  # noqa: PLC0415  (its extraction needs KiCad)
    out = Path(arguments.out).resolve() / "firstroute"
    if name.endswith(("-designer", "-ours")):
        short, rule = name.rsplit("-", 1)
    else:
        short, rule = unit_parts(name.partition("@")[0])[0], name.replace("@", "-at-")
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
        unit, _, stage = name.partition("@")
        placed = next((Path(arguments.out).resolve() / "runs" / unit / "placed").glob("*.kicad_pcb"))
        source = partialcopy.extracted(placed, out / "extract" / f"{unit}.json")
        if stage:
            stage, _, seed = stage.partition(".")
            source = at_stage(source, Path(arguments.out).resolve() / "runs" / unit / "trace.json", stage,
                              int(seed) if seed else None)
    footprints = ([] if source is None else
                  [f for f in partialcopy.numbered(source["footprints"]) if partialcopy.key(f) in movable])
    if source is not None and len(footprints) != len(movable):
        raise SystemExit(f"{name}: {len(footprints)} of {len(movable)} movable parts found")
    partialcopy.write_unit(out, task, rule, footprints, source or designer)
    return out, partialcopy.unit_command(out, board, f"{rule}-s1", arguments.binary)


def others(runs):
    """The run.py processes laying out into `runs` (any runner's)."""
    count = 0
    for cmdline in Path("/proc").glob("[0-9]*/cmdline"):
        try:
            args = cmdline.read_bytes().split(b"\0")
        except OSError:
            continue
        if any(a.endswith(b"run.py") for a in args) and any(a.startswith(str(runs).encode()) for a in args):
            count += 1
    return count


def at_stage(board, trace_path, stage, seed_number=None):
    """The extracted board with its movable footprints at a stage's poses
    of the trace's kept seed, or of the seed numbered `seed_number` (the
    trace's parts are the movable ones, in board order: matched by
    `REF#n`)."""
    trace = json.loads(Path(trace_path).read_text())
    seed = next(entry for entry in trace["seeds"]
                if (entry["kept"] if seed_number is None else entry.get("seed") == seed_number))
    index = seed["stages"].index(stage)
    poses = dict(zip(keyed(seed["parts"]), (part["poses"][index] for part in seed["parts"])))
    result = json.loads(json.dumps(board))
    seen = {}
    for footprint in result["footprints"]:
        n = seen.get(footprint["reference"], 0)
        seen[footprint["reference"]] = n + 1
        pose = poses.get(f"{footprint['reference']}#{n}")
        if pose is not None:
            footprint["x"], footprint["y"], footprint["angle"] = pose[0], pose[1], pose[2] % 360.0
    return result


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
        if arguments.share:
            busy = max(busy, others(Path(arguments.out).resolve() / "firstroute" / "runs") - len(running))
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
            seed_stages = [stage for stage in ALL_STAGES if stage in seed["stages"]]
            if seed["kept"]:
                tags.append("kept")
            if result["race_seed"] is not None and seed["seed"] == result["race_seed"]:
                tags.append("race")
            for stage in seed_stages:
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
              for i, a in enumerate(seeds) for b in seeds[i + 1:]
              if stage in a["centers"] and stage in b["centers"]]
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
    present = [stage for stage in ALL_STAGES
               if any(stage in seed["stages"] for result in results.values() for seed in result["seeds"])]
    lines = ["## Our pipeline: the active parts against the designer's, per stage", "",
             "displacement share / pairwise rank correlation (0 / 1: the designer's arrangement; "
             "a shuffle about 1.4 / 0)", "",
             "| board | seed | placement | " + " | ".join(present) + " | hpwl all, final | objective, final |",
             "| --- | --- | --- | " + "--- | " * len(present) + "---: | ---: |"]
    for name, result in results.items():
        if result["variant"] not in ("ours", "netlist"):
            continue
        rows = [("kept", kept(result))]
        race = next((seed for seed in result["seeds"] if seed["seed"] == result["race_seed"]), None)
        if race is not None:
            rows.append(("race", race))
        tag = "" if result["variant"] == "ours" else f" {result['variant']}"
        for label, seed in rows:
            lines.append(f"| {result['board']} | {result['seed']}{tag} | {label} (seed {seed['seed']}) | "
                         + " | ".join(cell(seed["stages"].get(stage)) for stage in present)
                         + f" | {fmt(seed['stages']['final'].get('hpwl_all'))}"
                         + f" | {fmt(seed['stages']['final'].get('objective'), 0)} |")
        means = []
        for stage in present:
            shares = [seed["stages"][stage]["displacement_share"] for seed in result["seeds"]
                      if stage in seed["stages"]]
            pairs = [seed["stages"][stage]["pairwise"] for seed in result["seeds"] if stage in seed["stages"]]
            means.append(f"{fmt(statistics.mean(shares))} / {fmt(statistics.mean(pairs))}" if shares else "-")
        lines.append(f"| {result['board']} | {result['seed']}{tag} | mean of {len(result['seeds'])} | "
                     + " | ".join(means) + " | | |")
        lines.append(f"| {result['board']} | {result['seed']}{tag} | seeds against each other (pairwise) | "
                     + " | ".join(fmt(agreement(result["seeds"], stage)) for stage in present) + " | | |")
    lines += ["", "## Warm starts from the designer's poses (kept seed)", "",
              "the stage's own result, then the final placement: share / pairwise; the final objective "
              "against ours (the kept seed's final, same board and seed)", "",
              "| board | seed | start | the stage's result | final | final objective / ours |",
              "| --- | --- | --- | --- | --- | ---: |"]
    own = {"global-all": "global", "global-active": "global", "anneal": "annealed", "legalize": "legal",
           "ours-noanneal": "legal", "global-all-noanneal": "legal", "global-active-noanneal": "legal"}
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
        elif result["variant"] == "netlist":
            rows = [(f"{result['variant']} start, final", kept(result)["terms"]["final"])]
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
    lines += descent(results)
    if routes:
        lines += ["", "## First routes (layout seed 1, `stop_after_first_route`)", "",
                  "| run | open terminals | KiCad unconnected | CPU s |", "| --- | ---: | ---: | ---: |"]
        lines += [f"| {label} | {r['first_open']} | {r['unconnected']} | {r['cpu_seconds']} |"
                  for label, r in routes.items()]
    cpu = [result["unit"].get("cpu_seconds") or 0 for result in results.values()]
    lines += ["", f"Place checks: {len(cpu)} units, {sum(cpu):.0f} CPU seconds."]
    return lines


def wire_share(step):
    """The wirelength gradient's share of the descent's gradient at a
    history step `[iteration, overflow, lambda, gamma, wire, density,
    wirelength]`."""
    total = step[4] + step[5]
    return step[4] / total if total > 0 else None


def descent(results):
    """Per unit the kept seed's global placements (the trace's
    `global_history`): the iterations, the overflow at the stop, the
    density weight's growth, and where the wire gradient's share of the
    gradient falls below a half, a tenth and a hundredth (with the overflow
    there): whether the wires arrange the parts before the density
    spreads them."""
    lines = ["", "## The descents (kept seed)", "",
             "| unit | descent | iterations | overflow at the stop | lambda last / first | "
             "wire share at 0 | share < 0.5 at (overflow) | < 0.1 at | < 0.01 at |",
             "| --- " * 9 + "|"]
    for name, result in results.items():
        seed = kept(result)
        for label, steps in (seed.get("global_history") or {}).items():
            if not steps:
                continue
            first, last = steps[0], steps[-1]

            def below(limit):
                for step in steps:
                    share = wire_share(step)
                    if share is not None and share < limit:
                        return f"{int(step[0])} ({step[1]:.2f})"
                return "-"
            growth = last[2] / first[2] if first[2] > 0 else None
            lines.append(f"| {name} | {label} | {int(last[0]) + 1} | {last[1]:.3f} | "
                         f"{growth:.3g} | {fmt(wire_share(first))} | {below(0.5)} | {below(0.1)} | "
                         f"{below(0.01)} |" if growth is not None else f"| {name} | {label} | - |")
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
    runner.add_argument("--expected-gb", type=float, default=0.5,
                        help="each place check's expected peak, reserved in the memory ledger (default 0.5: the "
                             "dense boards' checks peak at 0.11 to 0.18 GB)")
    runner.add_argument("units", nargs="+")
    router = commands.add_parser("route")
    router.add_argument("--inputs", required=True)
    router.add_argument("--out", required=True)
    router.add_argument("--binary", required=True)
    router.add_argument("--slots", type=int, default=2)
    router.add_argument("--busy", type=int, action="append", default=[],
                        help="a process (pid) of runs started elsewhere: it takes a slot while it lives")
    router.add_argument("--share", action="store_true",
                        help="count every run.py laying out into this output (another runner's) against the slots")
    router.add_argument("units", nargs="+")
    tabler = commands.add_parser("table")
    tabler.add_argument("--inputs", required=True)
    tabler.add_argument("--out", required=True)
    passiver = commands.add_parser("passives")
    passiver.add_argument("--out", required=True)
    passiver.add_argument("units", nargs="*", help="the units to measure (default: every traced unit)")
    stacker = commands.add_parser("stacks")
    stacker.add_argument("--out", required=True)
    stacker.add_argument("units", nargs="*", help="the units to measure (default: every traced unit)")
    arguments = parser.parse_args()
    {"run": run, "route": route, "table": table, "passives": passives_table, "stacks": stacks_table}[arguments.command](arguments)


if __name__ == "__main__":
    main()
