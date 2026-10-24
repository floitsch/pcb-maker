#!/usr/bin/env python3
# Copyright (C) 2026 Toit contributors.
"""Placement ablation (docs/placer.md "Placement ablation (2026-10-24)"):
which of our placement's decisions cost open connections.

    ablation.py prepare <out> --task NAME --designer <designer project dir> --placed <our placed dir> \\
        --tool <pcb-maker with ablate-kicad-placement> [--groups ...] [--config ablation.json]
    ablation.py route <out> --binary <snapshot> [--jobs 3 --threads 2 --min-free-gb 10] [--only VARIANT ...]
    ablation.py report <out> [<out> ...] [--markdown]

`prepare` runs `pcb-maker ablate-kicad-placement` (the hybrids: the
designer's placement with one group of parts where we put them, `d-<group>`,
and ours with the group where the designer put it, `o-<group>`; the two
placements themselves, `designer` and `ours`; null variants with one passive
nudged, `d-null-N` and `o-null-N`), then lays the variants out as a
`--route-from` tree (`<out>/tree/<task>__<variant>/layout/placed`) with a task
list that names each variant as a task of its own (`<out>/tasks.json`, the
task's entry of tasks/v7 with its name and directory changed: the directory
is the designer's project, for the pass rule's designer findings).

`route` runs run.py in route mode over that list (the task budget, the same
binary for every variant, the memory ledger), the baselines and nulls first,
then the groups in the report's order.

`report` joins `<out>/ablation.json` with the routes' rows (KiCad's
unconnected count): per group the open connections our placement of it
explains, from both sides,

    cost_d = open(d-<group>) - open(designer)    (our group in the designer's placement)
    cost_o = open(ours) - open(o-<group>)        (the designer's group in ours)

their mean the group's attributed cost, ranked; the nulls give the noise
(a nudge that should cost nothing). Then the signal check: for each signal
(wirelength, escape demand's channel overflow, blocked exits, the global
route's overflow and soft overflow at 0.5), its change between the same
pairs, whether it orders the two placements as the routes do, and how
often it orders a group's two variants (cost_d and cost_o separately) the
same way as the open connections (pairwise agreement over the variants
whose open count moved by more than the noise).
"""

import argparse
import json
import math
import shutil
import subprocess
import sys
from pathlib import Path

HERE = Path(__file__).resolve().parent
ROOT = HERE.parent.parent
TASKS = HERE / "tasks/v7/tasks.json"

SIGNALS = [
    ("wirelength_mm", "wirelength (mm)", lambda s: s["wirelength_mm"]),
    ("channel_overflow", "escape channel overflow (tracks)", lambda s: s["channel_overflow"]),
    ("blocked_exits", "exits facing a body within 5 mm", lambda s: s["blocked_exits"]),
    ("ratsnest_crossings", "ratsnest crossings", lambda s: s.get("ratsnest_crossings")),
    ("large_crossings", "ratsnest crossings between large parts", lambda s: s.get("large_crossings")),
    ("global_overflow", "global route overflow", lambda s: (s.get("global") or {}).get("total_overflow")),
    ("global_soft", "global route soft overflow at 0.5", lambda s: (s.get("global") or {}).get("soft_overflow_05")),
    ("global_nets", "global route overflowed nets", lambda s: (s.get("global") or {}).get("overflowed_nets")),
]


def task_entry(name):
    for task in json.loads(TASKS.read_text())["tasks"]:
        if task["name"] == name:
            return task
    raise SystemExit(f"{name}: not in {TASKS}")


def order(report):
    """Variants in routing order: the baselines, the nulls, then the groups
    (large parts, connectors, quadrants, clusters), both sides of each."""
    kinds = {"large": 0, "connectors": 1, "quadrant": 2, "cluster": 3}
    rank = {group["name"]: (kinds.get(group["kind"], 9), index) for index, group in enumerate(report["groups"])}
    def key(variant):
        if variant.get("group") is None:
            return (0, 0, variant["name"])
        if variant.get("group").startswith("null:"):
            return (1, 0, variant["name"])
        return (2,) + rank.get(variant.get("group"), (9, 0)) + (variant["name"],)
    return sorted(report["variants"], key=key)


def prepare(arguments):
    out = arguments.out.resolve()
    task = task_entry(arguments.task)
    board_id = task["board_id"]
    hybrids = out / "hybrids"
    if not (hybrids / "ablation.json").exists():
        if hybrids.exists():
            shutil.rmtree(hybrids)
        command = [str(arguments.tool), "ablate-kicad-placement",
                   str(arguments.designer / f"{board_id}.kicad_pcb"), str(arguments.placed), board_id, str(hybrids)]
        config = {}
        if arguments.config:
            config = json.loads(arguments.config.read_text())
        if arguments.groups:
            config["groups"] = arguments.groups
        if config:
            out.mkdir(parents=True, exist_ok=True)
            (out / "ablation-config.json").write_text(json.dumps(config))
            command.append(str(out / "ablation-config.json"))
        print(" ".join(command), file=sys.stderr)
        subprocess.run(command, check=True)
    report = json.loads((hybrids / "ablation.json").read_text())
    tree = out / "tree"
    if tree.exists():
        shutil.rmtree(tree)
    tasks = []
    for variant in order(report):
        name = f"{arguments.task}__{variant['name']}"
        placed = tree / name / "layout" / "placed"
        placed.parent.mkdir(parents=True)
        placed.symlink_to(Path(variant["directory"]).resolve())
        entry = dict(task, name=name, directory=str(arguments.designer.resolve()))
        entry.pop("constraints", None)
        tasks.append(entry)
    (out / "tasks.json").write_text(json.dumps({"tasks": tasks}, indent=1))
    (out / "task.json").write_text(json.dumps({"task": arguments.task, "board_id": board_id,
                                               "designer": str(arguments.designer.resolve()),
                                               "placed": str(arguments.placed.resolve()),
                                               "base_from": str(arguments.base_from.resolve())
                                               if arguments.base_from else None}))
    print(f"{len(tasks)} variants in {out / 'tasks.json'}", file=sys.stderr)


def route(arguments):
    out = arguments.out.resolve()
    command = [sys.executable, str(HERE / "run.py"), str(out / "routes"), "--tasks", str(out / "tasks.json"),
               "--route-from", str(out / "tree"), "--binary", str(arguments.binary),
               "--jobs", str(arguments.jobs), "--threads", str(arguments.threads),
               "--min-free-gb", str(arguments.min_free_gb), "--expected-gb", str(arguments.expected_gb)]
    if arguments.only:
        task = json.loads((out / "task.json").read_text())["task"]
        command += ["--only"] + [f"{task}__{variant}" for variant in arguments.only]
    print(" ".join(command), file=sys.stderr)
    log = out / f"routes-{arguments.log_suffix}.log" if arguments.log_suffix else out / "routes.log"
    with open(log, "a") as handle:
        sys.exit(subprocess.run(command, stdout=handle).returncode)


def rows_of(out):
    """The route rows by variant (the last row of a variant wins), from
    every routes*.log in `out`; with `--base-from` the baselines' and the
    nulls' rows of that run where this one has none (the same boards and
    binary give the same rows)."""
    meta = json.loads((out / "task.json").read_text())
    task = meta["task"]
    rows = {}
    if meta.get("base_from"):
        for name, row in rows_of(Path(meta["base_from"])).items():
            if name in ("designer", "ours") or "-null-" in name:
                rows[name] = dict(row, base_from=meta["base_from"])
    for log in sorted(out.glob("routes*.log")):
        for line in log.read_text().splitlines():
            if not line.startswith("{"):
                continue
            row = json.loads(line)
            if row.get("name", "").startswith(task + "__"):
                rows[row["name"][len(task) + 2:]] = row
    return rows


def open_count(row):
    if row is None or row.get("cut") or row.get("error"):
        return None
    return (row.get("native") or {}).get("unconnected")


def spearman(xs, ys):
    def ranks(values):
        indexed = sorted(range(len(values)), key=lambda i: values[i])
        result = [0.0] * len(values)
        i = 0
        while i < len(indexed):
            j = i
            while j + 1 < len(indexed) and values[indexed[j + 1]] == values[indexed[i]]:
                j += 1
            for k in range(i, j + 1):
                result[indexed[k]] = (i + j) / 2
            i = j + 1
        return result
    if len(xs) < 3:
        return None
    rx, ry = ranks(xs), ranks(ys)
    mx, my = sum(rx) / len(rx), sum(ry) / len(ry)
    sx = math.sqrt(sum((r - mx) ** 2 for r in rx))
    sy = math.sqrt(sum((r - my) ** 2 for r in ry))
    if sx == 0 or sy == 0:
        return None
    return sum((a - mx) * (b - my) for a, b in zip(rx, ry)) / (sx * sy)


def analyse(out):
    """Per group the open connections our placement of it explains, from
    the routes' means: the baselines with their nulls and replicates
    (`designer`, `d-null-N`, `designer-rN`), a variant with its replicates
    (`d-<group>`, `d-<group>-rN`)."""
    meta = json.loads((out / "task.json").read_text())
    report = json.loads((out / "hybrids" / "ablation.json").read_text())
    rows = rows_of(out)
    if meta.get("base_from"):
        # The baselines are the earlier run's: the nulls with them.
        base_report = json.loads((Path(meta["base_from"]) / "hybrids" / "ablation.json").read_text())
        names = {variant["name"] for variant in report["variants"]}
        report["variants"] += [variant for variant in base_report["variants"]
                               if ("-null-" in variant["name"] or variant["name"] in ("designer", "ours"))
                               and variant["name"] not in names]
    variants = {variant["name"]: variant for variant in report["variants"]}
    # Signals measured again by a later tool on the same boards
    # (`<out>-sig`, written with the same configuration: the boards are
    # byte for byte the same) replace the first measurement's.
    agains = [out.parent / (out.name + "-sig")]
    if meta.get("base_from"):
        agains.append(Path(meta["base_from"]).parent / (Path(meta["base_from"]).name + "-sig"))
    for again in agains:
        if (again / "ablation.json").exists():
            for variant in json.loads((again / "ablation.json").read_text())["variants"]:
                if variant["name"] in variants:
                    variants[variant["name"]] = dict(variants[variant["name"]], signals=variant["signals"])
    opened = {name: open_count(row) for name, row in rows.items()}

    def runs(name):
        """The open counts of a variant and its replicates."""
        found = [opened[name]] if opened.get(name) is not None else []
        found += [count for other, count in opened.items()
                  if count is not None and other.startswith(name + "-r") and other[len(name) + 2:].isdigit()]
        return found

    base_runs = {}
    for side in ("designer", "ours"):
        base_runs[side] = runs(side) + [count for other, count in opened.items()
                                        if count is not None and other.startswith(side[0] + "-null-")]
    base = {side: (sum(values) / len(values) if values else None) for side, values in base_runs.items()}
    deviations = [count - base[side] for side, values in base_runs.items() for count in values]
    degrees = sum(len(values) - 1 for values in base_runs.values() if values)
    sd = math.sqrt(sum(d * d for d in deviations) / degrees) if degrees > 0 else None
    groups = []
    for group in report["groups"]:
        entry = dict(group)
        for side, prefix, sign in (("designer", "d", 1), ("ours", "o", -1)):
            values = runs(f"{prefix}-{group['name']}")
            mean = sum(values) / len(values) if values else None
            cost = None if mean is None or base[side] is None else sign * (mean - base[side])
            se = None if sd is None or not values or not base_runs[side] else \
                sd * math.sqrt(1 / len(values) + 1 / len(base_runs[side]))
            entry.update({f"open_{prefix}": mean, f"runs_{prefix}": values, f"cost_{prefix}": cost,
                          f"se_{prefix}": se,
                          f"displaced_{prefix}": len(variants.get(f"{prefix}-{group['name']}", {})
                                                     .get("displaced", []))})
        known = [entry[key] for key in ("cost_d", "cost_o") if entry[key] is not None]
        entry["cost"] = sum(known) / len(known) if known else None
        groups.append(entry)
    groups.sort(key=lambda group: (group["cost"] is None, -(group["cost"] or 0)))
    # The signal check: per signal, its change on the same pairs as the
    # attributed cost (the primary variant's signals: a replicate's nudge
    # hardly moves them).
    checks = []
    for key, label, read in SIGNALS:
        sd_, so = (read(variants[name]["signals"]) if name in variants else None for name in ("designer", "ours"))
        whole = None
        if None not in (sd_, so, base["designer"], base["ours"]):
            whole = (so - sd_) * (base["ours"] - base["designer"]) > 0
        pairs = []
        for group in groups:
            for prefix, reference, sign in (("d", sd_, 1), ("o", so, -1)):
                name = f"{prefix}-{group['name']}"
                cost = group[f"cost_{prefix}"]
                if name not in variants or cost is None or reference is None:
                    continue
                signal = read(variants[name]["signals"])
                if signal is None:
                    continue
                pairs.append((sign * (signal - reference), cost, group[f"se_{prefix}"] or 0))
        moved = [(signal, cost) for signal, cost, se in pairs if abs(cost) > 2 * se]
        agree = sum(1 for signal, cost in moved if signal * cost > 0)
        checks.append(dict(signal=key, label=label, designer=sd_, ours=so, whole_right=whole,
                           spearman=spearman([p[0] for p in pairs], [p[1] for p in pairs]),
                           pairs=len(pairs), moved=len(moved), agree=agree))
    return dict(task=meta["task"], report=report, rows=rows, base=base, base_runs=base_runs, sd=sd,
                groups=groups, checks=checks)


def part_summary(parts, members):
    """A group's escape in one placement, from the tool's per-part
    descriptions: channel overflow, the demand on sides that face a body
    within 5 mm, the sides where two dense pin rows face each other, the
    connections' mean distance, the nearest board edge, the global route's
    attribution."""
    chosen = [parts[name] for name in members if name in parts]
    if not chosen:
        return None
    facing = sum(side["demand"] for part in chosen for side in part["sides"] if side.get("faces"))
    dense = sum(1 for part in chosen for side in part["sides"]
                if side["demand"] >= 4 and (side.get("faced_demand") or 0) >= 4)
    tight = sum(side["demand"] for part in chosen for side in part["sides"]
                if side.get("faces") and side["gap_mm"] < 1.0)
    demand = sum(side["demand"] for part in chosen for side in part["sides"])
    return dict(parts=len(chosen), demand=demand, facing=facing, tight=tight, dense=dense,
                channel=sum(part["channel_overflow"] for part in chosen),
                connection=sum(part["connection_mm"] for part in chosen) / len(chosen),
                edge=sum(min(side["edge_mm"] for side in part["sides"]) for part in chosen) / len(chosen),
                attribution=sum(part.get("global_attribution") or 0 for part in chosen))


def describe(result, describe_path, top=3):
    """The top groups' escape, the designer's against ours."""
    parts = json.loads(Path(describe_path).read_text())
    designer = {part["reference"]: part for part in parts["parts_designer"]}
    ours = {part["reference"]: part for part in parts["parts_ours"]}
    lines = ["", "| group | placement | demand | on sides facing a body within 5 mm | within 1 mm | dense rows facing | channel overflow | connection mm | edge mm | global attribution |",
             "| --- | --- | ---: | ---: | ---: | ---: | ---: | ---: | ---: | ---: |"]
    for group in [group for group in result["groups"] if group["cost"] is not None][:top]:
        for label, table in (("designer's", designer), ("ours", ours)):
            summary = part_summary(table, group["members"])
            if summary is None:
                continue
            lines.append(f"| {group['name']} | {label} | {summary['demand']} | {summary['facing']} | {summary['tight']} "
                         f"| {summary['dense']} | {summary['channel']:.1f} | {summary['connection']:.1f} "
                         f"| {summary['edge']:.1f} | {summary['attribution']:.0f} |")
    return "\n".join(lines)


def fmt(value, digits=0):
    if value is None:
        return "-"
    if isinstance(value, float):
        return f"{value:.{digits}f}"
    return str(value)


def markdown(result):
    lines = [f"### {result['task']}", ""]
    base, runs = result["base"], result["base_runs"]
    lines.append(f"Routed open (KiCad unconnected), the baselines with their nulls: designer's placement "
                 f"{runs['designer']} (mean {fmt(base['designer'], 1)}), ours {runs['ours']} "
                 f"(mean {fmt(base['ours'], 1)}); a route's noise (pooled standard deviation) {fmt(result['sd'], 1)}.")
    lines += ["", "| group | parts | open d+ours / o+designer's (routes) | cost d / o (± se) | cost (mean) | displaced d / o |",
              "| --- | ---: | ---: | ---: | ---: | ---: |"]
    for group in result["groups"]:
        lines.append(f"| {group['name']} | {len(group['members'])} "
                     f"| {fmt(group['open_d'], 1)} ({len(group['runs_d'])}) / {fmt(group['open_o'], 1)} ({len(group['runs_o'])}) "
                     f"| {fmt(group['cost_d'], 1)} ± {fmt(group['se_d'], 0)} / {fmt(group['cost_o'], 1)} ± {fmt(group['se_o'], 0)} "
                     f"| {fmt(group['cost'], 1)} | {group['displaced_d']} / {group['displaced_o']} |")
    lines += ["", "| signal | designer's | ours | orders the placements right | Spearman with cost | agrees, pairs beyond 2 se |",
              "| --- | ---: | ---: | --- | ---: | ---: |"]
    for check in result["checks"]:
        lines.append(f"| {check['label']} | {fmt(check['designer'], 1)} | {fmt(check['ours'], 1)} "
                     f"| {fmt(check['whole_right'])} | {fmt(check['spearman'], 2)} "
                     f"| {check['agree']} of {check['moved']} |")
    return "\n".join(lines)


def report_command(arguments):
    for out in arguments.outs:
        result = analyse(out.resolve())
        if arguments.markdown:
            print(markdown(result))
            describe_path = out.resolve().parent / (out.name + "-describe") / "ablation.json"
            if describe_path.exists():
                print(describe(result, describe_path))
            print()
        else:
            print(json.dumps({key: result[key] for key in ("task", "base", "base_runs", "sd", "groups", "checks")},
                             indent=1))


def main():
    parser = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    commands = parser.add_subparsers(dest="command", required=True)
    p = commands.add_parser("prepare")
    p.add_argument("out", type=Path)
    p.add_argument("--task", required=True)
    p.add_argument("--designer", type=Path, required=True, help="the designer's project directory")
    p.add_argument("--placed", type=Path, required=True, help="our placed project directory (layout/placed)")
    p.add_argument("--tool", type=Path, required=True, help="a pcb-maker with ablate-kicad-placement")
    p.add_argument("--groups", nargs="*")
    p.add_argument("--base-from", type=Path, help="an earlier ablation of the same boards whose baseline and "
                   "null rows this one reuses (route only the groups: route --only)")
    p.add_argument("--config", type=Path)
    r = commands.add_parser("route")
    r.add_argument("out", type=Path)
    r.add_argument("--binary", type=Path, required=True)
    r.add_argument("--jobs", type=int, default=3)
    r.add_argument("--threads", type=int, default=2)
    r.add_argument("--min-free-gb", type=float, default=10)
    r.add_argument("--expected-gb", type=float, default=3)
    r.add_argument("--only", nargs="*")
    r.add_argument("--log-suffix")
    q = commands.add_parser("report")
    q.add_argument("outs", type=Path, nargs="+")
    q.add_argument("--markdown", action="store_true")
    arguments = parser.parse_args()
    {"prepare": prepare, "route": route, "report": report_command}[arguments.command](arguments)


if __name__ == "__main__":
    main()
