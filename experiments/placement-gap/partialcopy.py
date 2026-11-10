#!/usr/bin/env python3
# Copyright (C) 2026 Toit contributors.
# Use of this source code is governed by an MIT-style license that can be
# found in the LICENSE file.
"""Which placement decisions carry the designer's advantage: a part of the
designer's placement copied, the rest placed by the placer, the first route
of each (docs/placer.md, "Which placement decisions matter").

    partialcopy.py prepare --out DIR BOARD...
    partialcopy.py run --out DIR --binary PATH [--slots 4]
    partialcopy.py table --out DIR

Per board a subset of its movable parts (those run.py's unplace stacks: a
pad on a net, not locked, not named by the task's `fixed`) is pinned at the
designer's pose with `place` entries (`x`, `y` of the footprint's origin,
`"origin": "absolute"`, `angle`; the side needs no entry: the placer never
changes a part's side and unplace keeps it) added to the task's own
constraints, and the layout runs as the sweep's (tasks v7, budget scale 1)
with `stop_after_first_route`. The rules (`RULES`), each over the movable
parts:

- `ours`: nothing pinned (the sweep's layout), seeds 1 and 101;
- `connectors`: connectors and mechanical parts;
- `anchors`: `connectors` and the three largest ICs by pin count;
- `bus_ics`: `anchors` and every IC with a pin on a net of nine pins or
  more that no pour carries (metrics.py's `9plus` bucket);
- `displaced`: the control, the k parts farthest (origin to origin) from
  the designer's in our seed-1 placement (`ours-s1`'s placed board), k a
  tenth of the movable parts;
- `nonpassive`: every movable part but the passives (resistors,
  capacitors, inductors, ferrites, fuses, arrays of them);
- `designer`: every movable part, seeds 1 and 101.

A rule whose pinned set equals an earlier rule's is not run again (the
same input and seed: the same layout); its row is that rule's.

One run.py per unit at --jobs 1 --threads 2 and --expected-gb from the
board's measured layout peak (estimates.json), at most `--slots` in
flight; a unit with a `.done` file is skipped (restartable). `table`
writes `DIR/tables.md` and `DIR/results.json`: per unit the first route's
open terminals (`first_unconnected_terminals`), KiCad's unconnected count
of the board written (`native.unconnected`), the placer's final
wirelength (`wirelength_final_mm`), the placement digest, and whether
every pinned part kept the designer's pose; per subset the share of the
designer's advantage it recovers, (ours - subset) / (ours - designer)
over the means of the two seeds of each end. The summary over the
boards counts those whose designer's first route is below both of our
seeds (`counted`); `seed_spread` is our two seeds' difference as a share
of the advantage. A layout without a legal placement gives its error.
"""

import argparse
import json
import math
import re
import statistics
import subprocess
import sys
import time
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parent))

import metrics  # noqa: E402

ROOT = Path(__file__).resolve().parents[2]
TASKS = ROOT / "benchmarks/agent-tasks/tasks/v7/tasks.json"
ESTIMATES = ROOT / "benchmarks/agent-tasks/estimates.json"
RULES = ("connectors", "anchors", "bus_ics", "displaced", "nonpassive")
END_SEEDS = (1, 101)
SUBSET_SEED = 1
MECHANICAL_PREFIXES = ("H", "MH", "MK", "FID", "HOLE", "MP", "STANDOFF")
MECHANICAL_WORDS = ("mountinghole", "mounting_hole", "fiducial", "standoff", "spacer")
PASSIVE_PREFIXES = ("R", "C", "L", "FB", "F", "RN", "RA", "RR", "CN_ARRAY", "FL", "FER")
IC_PREFIXES = ("U", "IC", "MOD")
LARGE_ICS = 3


# --- the board ---------------------------------------------------------------------


def extract(path):
    """The board's footprints (reference, uuid, footprint, x, y, angle,
    side, locked, pads with their nets) and the pour nets, read by KiCad's
    pcbnew; a pad's `unconnected-...` net counts as none."""
    import pcbnew  # noqa: PLC0415  (only the extraction needs KiCad)
    board = pcbnew.LoadBoard(str(path))
    pours = sorted({zone.GetNetname() for zone in board.Zones()
                    if not zone.GetIsRuleArea() and zone.GetNetname() and zone.IsOnCopperLayer()})
    footprints = []
    for footprint in board.GetFootprints():
        position = footprint.GetPosition()
        pads = []
        for pad in footprint.Pads():
            net = pad.GetNetname() or None
            if net and net.startswith("unconnected-"):
                net = None
            pads.append({"number": pad.GetNumber(), "net": net})
        footprints.append({
            "reference": footprint.GetReference(), "uuid": footprint.m_Uuid.AsString(),
            "footprint": footprint.GetFPIDAsString(), "value": footprint.GetValue(),
            "x": position.x * metrics.NM, "y": position.y * metrics.NM,
            "angle": footprint.GetOrientationDegrees() % 360.0, "back": footprint.IsFlipped(),
            "locked": footprint.IsLocked(), "pads": pads})
    return {"pour_nets": pours, "footprints": footprints}


def extracted(path, cache):
    cache = Path(cache)
    if cache.exists() and cache.stat().st_mtime >= Path(path).stat().st_mtime:
        return json.loads(cache.read_text())
    data = extract(path)
    cache.parent.mkdir(parents=True, exist_ok=True)
    cache.write_text(json.dumps(data))
    return data


def glob_matches(pattern, text):
    """The constraints' glob (`*`, `?`, a backslash escapes the next
    character) against a reference."""
    expression, index = "", 0
    while index < len(pattern):
        character = pattern[index]
        if character == "\\" and index + 1 < len(pattern):
            expression += re.escape(pattern[index + 1])
            index += 2
            continue
        expression += ".*" if character == "*" else "." if character == "?" else re.escape(character)
        index += 1
    return re.fullmatch(expression, text) is not None


def has_net(footprint):
    return any(pad["net"] for pad in footprint["pads"])


def movable(footprints, fixed_patterns):
    """The footprints unplace stacks, in board order: a pad on a net, not
    locked, not named by `fixed`, and not sharing its reference with a
    netless footprint (which stays, and holds its namesakes)."""
    netless = {f["reference"] for f in footprints if f["reference"] and not has_net(f)}
    return [f for f in footprints
            if has_net(f) and not f["locked"] and f["reference"] not in netless
            and not any(glob_matches(pattern, f["reference"]) for pattern in fixed_patterns)]


# --- the classes --------------------------------------------------------------------------


def prefix(footprint):
    return re.match(r"[A-Za-z_]*", footprint["reference"]).group(0).upper()


def pins(footprint):
    return len({pad["number"] for pad in footprint["pads"] if pad["number"]})


def is_mechanical(footprint):
    name = footprint["footprint"].lower()
    return prefix(footprint) in MECHANICAL_PREFIXES or any(word in name for word in MECHANICAL_WORDS)


def is_passive(footprint):
    return prefix(footprint) in PASSIVE_PREFIXES and not metrics.is_connector(footprint)


def is_ic(footprint):
    """An IC: a U, IC or MOD reference, or eight pins or more that are not
    a connector's, a passive array's or a mechanical part's."""
    if metrics.is_connector(footprint) or is_mechanical(footprint) or is_passive(footprint):
        return False
    return prefix(footprint) in IC_PREFIXES or pins(footprint) >= 8


def large_nets(board):
    """The nets of nine pins or more that no pour carries."""
    counts = {}
    for footprint in board["footprints"]:
        for pad in footprint["pads"]:
            if pad["net"]:
                counts[pad["net"]] = counts.get(pad["net"], 0) + 1
    pours = set(board["pour_nets"])
    return {net for net, count in counts.items() if metrics.bucket(net, count, pours) == "9plus"}


def key(footprint):
    """A footprint's identity across two boards: its reference and its
    place among the footprints of that reference."""
    return footprint["reference"], footprint.get("nth", 0)


def numbered(footprints):
    """The footprints, each with `nth`: how many of its reference came
    before it in board order."""
    seen = {}
    result = []
    for footprint in footprints:
        n = seen.get(footprint["reference"], 0)
        seen[footprint["reference"]] = n + 1
        result.append({**footprint, "nth": n})
    return result


def displacement(designer, ours):
    """{key: distance between the two origins} over the footprints both
    boards hold."""
    other = {key(f): f for f in numbered(ours["footprints"])}
    result = {}
    for footprint in numbered(designer["footprints"]):
        twin = other.get(key(footprint))
        if twin is not None:
            result[key(footprint)] = math.hypot(twin["x"] - footprint["x"], twin["y"] - footprint["y"])
    return result


def subsets(board, fixed_patterns, ours=None):
    """{rule: [footprint]} over the movable footprints (each with `nth`),
    `displaced` only with our placement; `designer` every movable one."""
    parts = movable(numbered(board["footprints"]), fixed_patterns)
    connectors = [f for f in parts if metrics.is_connector(f) or is_mechanical(f)]
    chosen = {key(f) for f in connectors}
    ics = [f for f in parts if is_ic(f) and key(f) not in chosen]
    largest = sorted(ics, key=lambda f: (-pins(f), f["reference"], f["nth"]))[:LARGE_ICS]
    anchors = chosen | {key(f) for f in largest}
    nets = large_nets(board)
    bus = anchors | {key(f) for f in ics if any(pad["net"] in nets for pad in f["pads"])}
    result = {
        "connectors": [f for f in parts if key(f) in chosen],
        "anchors": [f for f in parts if key(f) in anchors],
        "bus_ics": [f for f in parts if key(f) in bus],
        "nonpassive": [f for f in parts if not is_passive(f)],
        "designer": parts,
    }
    if ours is not None:
        distance = displacement(board, ours)
        k = max(1, round(len(parts) / 10))
        farthest = sorted(parts, key=lambda f: (-distance.get(key(f), 0.0), f["reference"], f["nth"]))[:k]
        result["displaced"] = [f for f in parts if key(f) in {key(g) for g in farthest}]
    return result


def place_entries(footprints, board):
    """The `place` entries pinning these footprints at their poses; a
    reference several footprints share gets the footprint's uuid."""
    counts = {}
    for footprint in board["footprints"]:
        counts[footprint["reference"]] = counts.get(footprint["reference"], 0) + 1
    entries = []
    for footprint in footprints:
        entry = {"part": footprint["reference"], "x": round(footprint["x"], 6), "y": round(footprint["y"], 6),
                 "angle": round(footprint["angle"], 6), "origin": "absolute"}
        if counts.get(footprint["reference"], 0) > 1:
            entry["uuid"] = footprint["uuid"]
        entries.append(entry)
    return entries


def with_places(constraints, entries):
    """The task's constraints with the entries added to its `place` list."""
    result = json.loads(json.dumps(constraints))
    result["place"] = list(result.get("place", [])) + entries
    return result


# --- the units --------------------------------------------------------------------------------


def v7_tasks():
    return {task["name"]: task for task in json.loads(TASKS.read_text())["tasks"]}


def expected_gb(board):
    entry = json.loads(ESTIMATES.read_text())["boards"].get(board)
    if not entry or "layout" not in entry:
        raise SystemExit(f"{board}: no measured layout peak in {ESTIMATES}")
    return round(entry["layout"]["peak_rss_mb"] / 1024 + 0.05, 1)


def short(board):
    return board.split("__")[-1]


def unit_names(board):
    """The units of a board in their order: the ends first (`displaced`
    needs ours-s1's placed board), then the subsets."""
    names = [f"ours-s{seed}" for seed in END_SEEDS] + [f"designer-s{seed}" for seed in END_SEEDS]
    return names + [f"{rule}-s{SUBSET_SEED}" for rule in RULES]


def designer_board(task):
    directory = Path(task["directory"])
    if not directory.exists():
        directory = ROOT / "benchmarks/real/external/github" / task["name"]
    return directory / f"{task['board_id']}.kicad_pcb"


def ours_placed(out, board, board_id):
    return out / "runs" / f"{short(board)}-ours-s1" / board / "layout/placed" / f"{board_id}.kicad_pcb"


def write_unit(out, task, rule, footprints, board):
    """Writes the unit's constraints, its pinned list and its task list."""
    board_name = task["name"]
    directory = out / "units" / short(board_name)
    directory.mkdir(parents=True, exist_ok=True)
    constraints = json.loads((TASKS.parent / task["constraints"]).read_text()) if task.get("constraints") else {}
    constraints = with_places(constraints, place_entries(footprints, board))
    (directory / f"{rule}.constraints.json").write_text(json.dumps(constraints, indent=1))
    (directory / f"{rule}.pinned.json").write_text(json.dumps(sorted(f"{f['reference']}#{f['nth']}"
                                                                     for f in footprints)))
    entry = {key: value for key, value in task.items() if key != "constraints"}
    entry["directory"] = str(designer_board(task).parent)
    entry["constraints"] = f"{rule}.constraints.json"
    (directory / f"{rule}.tasks.json").write_text(json.dumps({"comment": "partialcopy.py", "tasks": [entry]},
                                                             indent=1))


def prepare_board(out, task, ours=None):
    """Writes every rule's unit of a board that its inputs allow; returns
    the rules written."""
    board = extracted(designer_board(task), out / "extract" / "designer" / f"{task['name']}.json")
    fixed = (json.loads((TASKS.parent / task["constraints"]).read_text()) if task.get("constraints")
             else {}).get("fixed", [])
    chosen = subsets(board, fixed, ours)
    chosen["ours"] = []
    seen = {}
    aliases = {}
    for rule in ("ours", "designer") + RULES:
        if rule not in chosen:
            continue
        pinned = tuple(sorted(key(f) for f in chosen[rule]))
        if pinned in seen and rule in RULES:
            aliases[rule] = seen[pinned]
        else:
            seen.setdefault(pinned, rule)
        write_unit(out, task, rule, chosen[rule], board)
    directory = out / "units" / short(task["name"])
    counts = {rule: len(footprints) for rule, footprints in chosen.items()}
    (directory / "rules.json").write_text(json.dumps({"movable": len(chosen["designer"]), "pinned": counts,
                                                      "aliases": aliases}, indent=1))
    return set(chosen)


def prepare(arguments):
    tasks = v7_tasks()
    out = Path(arguments.out)
    out.mkdir(parents=True, exist_ok=True)
    (out / "boards.json").write_text(json.dumps(arguments.boards, indent=1))
    for board in arguments.boards:
        written = prepare_board(out, tasks[board])
        rules = json.loads((out / "units" / short(board) / "rules.json").read_text())
        print(f"{board}: {rules['movable']} movable, pinned {rules['pinned']}, same as {rules['aliases']}",
              flush=True)
        assert "displaced" not in written


def unit_command(out, board, name, binary):
    rule, seed = name.rsplit("-s", 1)
    directory = out / "units" / short(board)
    placer = {"seed": int(seed)}
    return [sys.executable, str(ROOT / "benchmarks/agent-tasks/run.py"), str(out / "runs" / f"{short(board)}-{name}"),
            "--tasks", str(directory / f"{rule}.tasks.json"), "--only", board, "--binary", binary,
            "--jobs", "1", "--threads", "2", "--expected-gb", str(expected_gb(board)),
            "--placer", json.dumps(placer), "--layout", json.dumps({"stop_after_first_route": True})]


def ready(out, board, name):
    """Whether a unit can start: `displaced` waits for ours-s1's placed
    board and is written then; a rule aliased to an earlier one never
    runs."""
    rule = name.rsplit("-s", 1)[0]
    directory = out / "units" / short(board)
    if rule == "displaced" and not (directory / "displaced.tasks.json").exists():
        task = v7_tasks()[board]
        placed = ours_placed(out, board, task["board_id"])
        if not placed.exists() or not (out / "runs" / f"{short(board)}-ours-s1.done").exists():
            return False
        ours = extracted(placed, out / "extract" / "ours" / f"{board}.json")
        prepare_board(out, task, ours)
    return True


def aliased(out, board, name):
    rule = name.rsplit("-s", 1)[0]
    path = out / "units" / short(board) / "rules.json"
    return path.exists() and rule in json.loads(path.read_text())["aliases"]


def run(arguments):
    out = Path(arguments.out)
    (out / "runs").mkdir(parents=True, exist_ok=True)
    boards = json.loads((out / "boards.json").read_text())
    # The ends of every board first, then the subsets, the largest boards
    # first within each.
    order = sorted(boards, key=lambda b: -json.loads(ESTIMATES.read_text())["boards"][b]["layout"]["seconds"])
    queue = [(board, name) for board in order for name in unit_names(board)[:4]] + \
            [(board, name) for board in order for name in unit_names(board)[4:]]
    queue = [(b, n) for b, n in queue if not (out / "runs" / f"{short(b)}-{n}.done").exists()]
    running = {}
    while queue or running:
        for label, (process, log) in list(running.items()):
            if process.poll() is not None:
                log.close()
                (out / "runs" / f"{label}.done").write_text(f"{process.returncode}\n")
                print(time.strftime("%H:%M:%S"), "done", label, process.returncode, flush=True)
                del running[label]
        slots = arguments.slots
        if arguments.slots_file and Path(arguments.slots_file).exists():
            try:
                slots = min(int(Path(arguments.slots_file).read_text().strip()), arguments.slots)
            except ValueError:
                pass
        for item in list(queue):
            if len(running) >= slots:
                break
            board, name = item
            label = f"{short(board)}-{name}"
            if aliased(out, board, name):
                queue.remove(item)
                (out / "runs" / f"{label}.done").write_text("alias\n")
                print(time.strftime("%H:%M:%S"), "alias", label, flush=True)
                continue
            if not ready(out, board, name):
                continue
            if aliased(out, board, name):
                continue
            queue.remove(item)
            log = open(out / "runs" / f"{label}.out", "w")
            command = unit_command(out, board, name, arguments.binary)
            running[label] = (subprocess.Popen(command, cwd=ROOT, stdout=log, stderr=subprocess.STDOUT), log)
            print(time.strftime("%H:%M:%S"), "start", label, flush=True)
        time.sleep(20)
    print(time.strftime("%H:%M:%S"), "pool empty", flush=True)


# --- the tables ------------------------------------------------------------------------------------


def unit_result(out, board, name):
    """The unit's numbers, or None before it finished; a layout that
    stopped before its route (no legal placement) gives its error."""
    task = v7_tasks()[board]
    base = out / "runs" / f"{short(board)}-{name}"
    report = base / board / "layout/board-layout.json"
    if not report.exists():
        verdict = base / board / "layout/verdict.json"
        if verdict.exists():
            return {"error": json.loads(verdict.read_text()).get("error", "no report"), "board_id": task["board_id"]}
        return None
    layout = json.loads(report.read_text())
    rows = json.loads((base / "results.json").read_text()) if (base / "results.json").exists() else []
    row = rows[0] if rows else {}
    placer_path = base / board / "layout/placed/board-placer.json"
    placer = json.loads(placer_path.read_text()) if placer_path.exists() else {}
    place = [c for c in layout.get("constraints", []) if c.get("kind") == "place"]
    return {"first_open": layout.get("first_unconnected_terminals"),
            "unconnected": (row.get("native") or {}).get("unconnected"),
            "wirelength_mm": placer.get("wirelength_final_mm"),
            "digest": layout.get("placement_digest"),
            "pinned_kept": sum(1 for c in place if c.get("satisfied")), "pinned": len(place),
            "unplaced": len(placer.get("unplaced_parts") or []),
            "board_id": task["board_id"]}


def recovery(ours, designer, subset):
    """The share of the designer's advantage a subset recovers: 0 at ours,
    1 at the designer's; None where the ends do not differ."""
    if ours is None or designer is None or subset is None or ours == designer:
        return None
    return (ours - subset) / (ours - designer)


def mean(values):
    values = [v for v in values if v is not None]
    return statistics.mean(values) if values else None


def board_summary(out, board):
    rules = json.loads((out / "units" / short(board) / "rules.json").read_text())
    results = {name: unit_result(out, board, name) for name in unit_names(board)}
    for rule, target in rules["aliases"].items():
        # An alias of an end is that end's seed-1 row.
        results[f"{rule}-s{SUBSET_SEED}"] = results.get(f"{target}-s{SUBSET_SEED}")

    def end(rule):
        return mean([(results.get(f"{rule}-s{seed}") or {}).get("first_open") for seed in END_SEEDS])

    ours, designer = end("ours"), end("designer")
    shares = {rule: recovery(ours, designer, (results.get(f"{rule}-s{SUBSET_SEED}") or {}).get("first_open"))
              for rule in RULES}
    seeds = [(results.get(f"ours-s{seed}") or {}).get("first_open") for seed in END_SEEDS]
    return {"board": board, "movable": rules["movable"], "pinned": rules["pinned"], "aliases": rules["aliases"],
            "results": results, "ours": ours, "designer": designer, "recovery": shares,
            "counted": counted(seeds, designer), "seed_spread": spread(seeds, ours, designer)}


def counted(ours_seeds, designer):
    """Whether a board has a designer's advantage to recover: the
    designer's first route below both of our seeds."""
    values = [v for v in ours_seeds if v is not None]
    return bool(values) and designer is not None and designer < min(values)


def spread(ours_seeds, ours, designer):
    """Our two seeds' difference as a share of the designer's advantage:
    the recovery's noise at one seed is about half of it."""
    values = [v for v in ours_seeds if v is not None]
    if len(values) < 2 or ours is None or designer is None or ours == designer:
        return None
    return abs(values[0] - values[1]) / abs(ours - designer)


def fmt(value, digits=0):
    if value is None:
        return "-"
    return f"{value:.{digits}f}" if isinstance(value, float) else str(value)


def tables(summaries):
    out = []
    for s in summaries:
        out.append(f"### {short(s['board'])} ({s['movable']} movable parts)\n")
        out.append("| unit | pinned | first-route open | recovered | KiCad unconnected | placer wire mm "
                   "| pinned kept | digest |")
        out.append("| --- | ---: | ---: | ---: | ---: | ---: | ---: | --- |")
        for name in unit_names(s["board"]):
            rule = name.rsplit("-s", 1)[0]
            r = s["results"].get(name) or {}
            same = f" (= {s['aliases'][rule]})" if rule in s["aliases"] else ""
            share = s["recovery"].get(rule) if rule in RULES else None
            first = "no legal placement" if r.get("error") else fmt(r.get("first_open"))
            out.append(f"| {name}{same} | {s['pinned'].get(rule, '-')} | {first} "
                       f"| {fmt(share, 2)} | {fmt(r.get('unconnected'))} | {fmt(r.get('wirelength_mm'))} "
                       f"| {r.get('pinned_kept', '-')}/{r.get('pinned', '-')} | {(r.get('digest') or '-')[:12]} |")
        out.append("")
    out.append("### Recovery, board by subset\n")
    out.append("| board | ours (s1, s101) | designer (s1, s101) | seed spread | " + " | ".join(RULES) + " |")
    out.append("| --- | --- | --- | ---: | " + " | ".join("---:" for _ in RULES) + " |")
    for s in summaries:
        def pair(rule):
            return ", ".join(fmt((s["results"].get(f"{rule}-s{seed}") or {}).get("first_open"))
                             for seed in END_SEEDS)
        def cell(rule):
            if (s["results"].get(f"{rule}-s{SUBSET_SEED}") or {}).get("error"):
                return f"unplaced ({s['pinned'].get(rule, '-')})"
            return f"{fmt(s['recovery'][rule], 2)} ({s['pinned'].get(rule, '-')})"
        name = short(s["board"]) + ("" if s["counted"] else " (no advantage)" if s["ours"] is not None
                                    else " (not run)")
        out.append(f"| {name} | {pair('ours')} | {pair('designer')} | {fmt(s['seed_spread'], 2)} | "
                   + " | ".join(cell(rule) for rule in RULES) + " |")
    out.append("")
    summaries = [s for s in summaries if s["counted"]]
    out.append(f"### Over the {len(summaries)} boards with an advantage to recover\n")
    out.append("| subset | boards | median recovered | mean recovered | median share of movable pinned |")
    out.append("| --- | ---: | ---: | ---: | ---: |")
    for rule in RULES:
        values = [s["recovery"][rule] for s in summaries if s["recovery"][rule] is not None]
        pinned = [s["pinned"][rule] / s["movable"] for s in summaries if rule in s["pinned"] and s["movable"]]
        out.append(f"| {rule} | {len(values)} | {fmt(statistics.median(values) if values else None, 2)} "
                   f"| {fmt(statistics.mean(values) if values else None, 2)} "
                   f"| {fmt(statistics.median(pinned) if pinned else None, 2)} |")
    return "\n".join(out) + "\n"


def table(arguments):
    out = Path(arguments.out)
    boards = json.loads((out / "boards.json").read_text())
    summaries = [board_summary(out, board) for board in boards]
    (out / "results.json").write_text(json.dumps(summaries, indent=1))
    text = tables(summaries)
    (out / "tables.md").write_text(text)
    print(text)


def main():
    parser = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    sub = parser.add_subparsers(dest="command", required=True)
    p = sub.add_parser("prepare")
    p.add_argument("--out", required=True)
    p.add_argument("boards", nargs="+")
    r = sub.add_parser("run")
    r.add_argument("--out", required=True)
    r.add_argument("--binary", required=True)
    r.add_argument("--slots", type=int, default=4)
    r.add_argument("--slots-file", help="a file holding the slots (at most --slots), read again every 20 s")
    t = sub.add_parser("table")
    t.add_argument("--out", required=True)
    arguments = parser.parse_args()
    {"prepare": prepare, "run": run, "table": table}[arguments.command](arguments)


if __name__ == "__main__":
    main()
