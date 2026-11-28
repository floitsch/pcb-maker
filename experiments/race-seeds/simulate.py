#!/usr/bin/env python3
# Copyright (C) 2026 Florian Loitsch.
"""The race's allocations simulated on the ground-truth matrix
(docs/experiments/race-seeds.md).

    simulate.py [MATRIX-DIR] [--boards] [--cells]

Reads matrix.py's cells: per board, placement (the race's rank) and router
seed the finished layout's KiCad unconnected items and strict pass, and
every placement's probe under every router seed. Checks the cells (exit
codes, a result, the wall-clock stretch, that a probe is the same in every
cell that ran it), then prints what each allocation of the race would have
kept and what it ends with, per board and summed.

An allocation that runs more probes than today's three is not charged for
them here: its row is an upper bound on what it gains.
"""

import argparse
import itertools
import json
import sys
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parent))
import matrix  # noqa: E402
from probe_rank import spearman  # noqa: E402

RACE_RIPPED_MARGIN = 0.02


def race_pick(counts):
    """`race_pick` of board_layout.rs over the ripped counts in the
    ranking's order (ties to the better-ranked; the standing's tie-break
    left out)."""
    best = min(range(len(counts)), key=lambda index: (counts[index], index))
    if best == 0 or counts[0] <= 0:
        return 0
    return best if (counts[0] - counts[best]) / counts[0] >= RACE_RIPPED_MARGIN else 0


def components(table):
    """The variance components of a rows x columns table without
    replication (random effects, floored at 0): rows, columns, the
    interaction (the residual), and the mean."""
    rows, columns = len(table), len(table[0])
    mean = sum(map(sum, table)) / (rows * columns)
    row_means = [sum(row) / columns for row in table]
    column_means = [sum(table[r][c] for r in range(rows)) / rows for c in range(columns)]
    ms_rows = columns * sum((m - mean) ** 2 for m in row_means) / max(rows - 1, 1)
    ms_columns = rows * sum((m - mean) ** 2 for m in column_means) / max(columns - 1, 1)
    ms_rest = sum((table[r][c] - row_means[r] - column_means[c] + mean) ** 2
                  for r in range(rows) for c in range(columns)) / max((rows - 1) * (columns - 1), 1)
    return max((ms_rows - ms_rest) / columns, 0.0), max((ms_columns - ms_rest) / rows, 0.0), ms_rest, mean


def load(out):
    """board -> {"final": {(rank, seed): row}, "probe": {(rank, seed): probe}}, and the problems found."""
    boards, problems = {}, []
    for short, name in matrix.BOARDS.items():
        final, probe = {}, {}
        for rank in matrix.RANKS:
            for seed in matrix.ROUTER_SEEDS:
                cell = out / f"{short}-k{rank}-r{seed}"
                row = matrix.row(cell)
                if row is None:
                    continue
                code = (out / f"{cell.name}.exit")
                code = code.read_text().strip() if code.exists() else "?"
                try:
                    report = json.loads((cell / name / "layout" / "board-layout.json").read_text())
                except (OSError, ValueError):
                    problems.append(f"{cell.name}: no report (exit {code})")
                    continue
                race = report.get("race") or {}
                unconnected = (row.get("native") or {}).get("unconnected")
                if unconnected is None:
                    problems.append(f"{cell.name}: no KiCad result (exit {code}, {row.get('verdict')})")
                    continue
                kept_rank = next((index for index, item in enumerate(race.get("probe") or [])
                                  if item["placement"] == race.get("kept")), None)
                if kept_rank != rank:
                    problems.append(f"{cell.name}: the race kept rank {kept_rank}, not {rank}")
                    continue
                if not str(race.get("reason", "")).startswith("PCB_RACE_KEEP"):
                    problems.append(f"{cell.name}: the hook did not fire")
                stretch = row.get("wall_stretch") or 0
                log = (cell / name / "layout.log")
                text = log.read_text(errors="replace") if log.exists() else ""
                # A row cut by the wall clock, a fired guard, a check's
                # timeout, a kill.
                lowered = text.lower()
                cut = [word for word in ("wall_guard_expired", "wall clock passed the layout's guard",
                                         "wall guard elapsed", "wall guard ended", "wall guard expired",
                                         "timed out", "timeout expired")
                       if word in lowered]
                try:
                    verdict = json.loads((cell / name / "layout" / "verdict.json").read_text())
                except (OSError, ValueError):
                    verdict = {}
                cut += ["verdict.cut"] if verdict.get("cut") else []
                cut += [key for key in ("cut", "killed", "error") if row.get(key)]
                cut += [f"exit {row.get('exit')}"] if row.get("exit") not in (0, 1) else []
                cut += ["stretch at the cap"] if stretch >= 3.999 else []
                final[(rank, seed)] = {
                    "unconnected": unconnected, "strict": bool(row.get("pass_strict")),
                    "first_open": row.get("first_unconnected_terminals"), "work": row.get("work_seconds"),
                    "stretch": stretch, "cut": cut, "digest": report.get("placement_digest"),
                    "total": report.get("total_seconds"),
                }
                for index, item in enumerate(race.get("probe") or []):
                    value = (item["ripped_open_terminals"], item["open_terminals"], item["unfinished"])
                    if probe.setdefault((index, seed), value) != value:
                        problems.append(f"{cell.name}: probe of rank {index} differs from another cell's "
                                        f"({value} against {probe[(index, seed)]})")
        if final:
            boards[short] = {"final": final, "probe": probe}
    return boards, problems


def allocations(final, probe, ranks, seeds):
    """name -> the (rank, seed) cells each allocation ends with, one per
    way of drawing its router seeds (their results are averaged)."""
    ripped = lambda rank, seed: probe[(rank, seed)][0]  # noqa: E731
    first = lambda rank, seed: final[(rank, seed)]["first_open"]  # noqa: E731
    out = {}
    # (a) today: the placements at one router seed, the race's pick.
    out["a: n placements x 1 seed (today)"] = [
        (race_pick([ripped(rank, seed) for rank in ranks]), seed) for seed in seeds]
    out["no race: the first-ranked placement"] = [(0, seed) for seed in seeds]
    out["a chance pick among the placements"] = [(rank, seed) for rank in ranks for seed in seeds]
    # (b) the first-ranked placement under every router seed, the best probe routes on.
    out["b: 1 placement x 3 seeds, best probe"] = [
        (0, min(seeds, key=lambda seed: (ripped(0, seed), seed)))]
    # (b) two placements x two seeds (four probes), the best probe routes on.
    if len(ranks) >= 2:
        out["b: 2 placements x 2 seeds, best probe (4 probes)"] = [
            min(((rank, seed) for rank in (0, 1) for seed in pair), key=lambda cell: (ripped(*cell), cell))
            for pair in itertools.combinations(seeds, 2)]
        # ... the placement with the lower mean, then its better probe.
        cells = []
        for pair in itertools.combinations(seeds, 2):
            rank = min((0, 1), key=lambda rank: (sum(ripped(rank, seed) for seed in pair), rank))
            cells.append((rank, min(pair, key=lambda seed: (ripped(rank, seed), seed))))
        out["b: 2 placements x 2 seeds, lower mean (4 probes)"] = cells
    # Every placement under every seed: by the mean, then its best probe.
    rank = min(ranks, key=lambda rank: (sum(ripped(rank, seed) for seed in seeds), rank))
    out["b: n placements x 3 seeds, lower mean (9 probes)"] = [
        (rank, min(seeds, key=lambda seed: (ripped(rank, seed), seed)))]
    out["b: n placements x 3 seeds, best probe (9 probes)"] = [
        min(((rank, seed) for rank in ranks for seed in seeds), key=lambda cell: (ripped(*cell), cell))]
    # (c) today's pick, then the kept placement's best probe of the three seeds.
    cells = []
    for seed in seeds:
        rank = race_pick([ripped(rank, seed) for rank in ranks])
        cells.append((rank, min(seeds, key=lambda other: (ripped(rank, other), other != seed, other))))
    out["c: today's pick, its best probe of 3 seeds (5 probes)"] = cells
    # (d) halving: the two best probes route their first route, the better first route goes on.
    cells = []
    for seed in seeds:
        two = sorted(ranks, key=lambda rank: (ripped(rank, seed), rank))[:2]
        cells.append((min(two, key=lambda rank: (first(rank, seed), rank)), seed))
    out["d: halving, 2 first routes (an extra first route)"] = cells
    # Oracles.
    out["oracle: the best placement at the seed"] = [
        (min(ranks, key=lambda rank: final[(rank, seed)]["unconnected"]), seed) for seed in seeds]
    out["oracle: the best seed of the first-ranked placement"] = [
        (0, min(seeds, key=lambda seed: final[(0, seed)]["unconnected"]))]
    out["oracle: the best cell"] = [
        min(final, key=lambda cell: final[cell]["unconnected"])]
    out["oracle: the first route's open terminals at the seed"] = [
        (min(ranks, key=lambda rank: (first(rank, seed), rank)), seed) for seed in seeds]
    return out


def main():
    parser = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    parser.add_argument("out", nargs="?", default=str(matrix.OUT))
    parser.add_argument("--boards", action="store_true")
    parser.add_argument("--cells", action="store_true")
    arguments = parser.parse_args()
    boards, problems = load(Path(arguments.out))
    for problem in problems:
        print(f"PROBLEM {problem}")
    seeds = matrix.ROUTER_SEEDS
    totals, strict_totals, complete = {}, {}, []
    correlations = {"within a router seed, over placements": [], "within a placement, over router seeds": [],
                    "over a board's cells": [], "first route over a board's cells": []}
    for short, data in boards.items():
        final, probe = data["final"], data["probe"]
        ranks = sorted({rank for rank, _ in probe})
        missing = [(rank, seed) for rank in ranks for seed in seeds if (rank, seed) not in final or (rank, seed) not in probe]
        if arguments.cells:
            print(f"### {short}: KiCad unconnected (`!` no strict pass) / ripped count of the probe / first route open")
            print("| placement | " + " | ".join(f"r{seed}" for seed in seeds) + " |")
            print("| --- | " + " | ".join("---" for _ in seeds) + " |")
            for rank in ranks:
                line = []
                for seed in seeds:
                    cell = final.get((rank, seed))
                    ripped = probe.get((rank, seed), ("-",))[0]
                    line.append("-" if cell is None else
                                f"{cell['unconnected']}{'' if cell['strict'] else '!'} / {ripped} / {cell['first_open']}")
                print(f"| {rank} | " + " | ".join(line) + " |")
            stretched = [f"{cell} {value['stretch']:.1f}" for cell, value in final.items() if value["stretch"] > 3]
            flagged = [f"{cell} {value['cut']}" for cell, value in final.items() if value["cut"]]
            work = [value["work"] for value in final.values()]
            print(f"work {min(work):.0f} to {max(work):.0f} of {next(iter(final.values()))['total']:.0f}; "
                  f"stretch > 3: {stretched or 'none'}; log words: {flagged or 'none'}")
            print()
        if missing:
            print(f"({short}: cells missing {missing}; left out of the simulation)")
            continue
        complete.append(short)
        value = lambda cell: final[cell]["unconnected"]  # noqa: E731
        for seed in seeds:
            if len(ranks) > 2:
                correlations["within a router seed, over placements"].append(
                    spearman([probe[(rank, seed)][0] for rank in ranks], [value((rank, seed)) for rank in ranks]))
        for rank in ranks:
            correlations["within a placement, over router seeds"].append(
                spearman([probe[(rank, seed)][0] for seed in seeds], [value((rank, seed)) for seed in seeds]))
        cells = sorted(final)
        correlations["over a board's cells"].append(
            spearman([probe[cell][0] for cell in cells], [value(cell) for cell in cells]))
        correlations["first route over a board's cells"].append(
            spearman([final[cell]["first_open"] for cell in cells], [value(cell) for cell in cells]))
        results = allocations(final, probe, ranks, seeds)
        if arguments.boards:
            print(f"### {short}")
        for name, picked in results.items():
            mean = sum(value(cell) for cell in picked) / len(picked)
            strict = sum(final[cell]["strict"] for cell in picked) / len(picked)
            totals[name] = totals.get(name, 0.0) + mean
            strict_totals[name] = strict_totals.get(name, 0.0) + strict
            if arguments.boards:
                print(f"  {name}: {mean:.1f} ({picked})")
    print("## Where the final result's variance comes from (two-way, a cell is deterministic: the interaction is the residual)")
    print("| board | placement | router seed | interaction | sd of a cell | mean | the first route's: placement | router seed | interaction |")
    print("| --- | ---: | ---: | ---: | ---: | ---: | ---: | ---: | ---: |")
    for short in complete:
        final = boards[short]["final"]
        ranks = sorted({rank for rank, _ in final})
        line = []
        for key in ("unconnected", "first_open"):
            placement, router, interaction, mean = components(
                [[final[(rank, seed)][key] for seed in seeds] for rank in ranks])
            total = max(placement + router + interaction, 1e-9)
            shares = [f"{part:.1f} ({100 * part / total:.0f} %)" for part in (placement, router, interaction)]
            line += shares + ([f"{total ** 0.5:.1f}", f"{mean:.1f}"] if key == "unconnected" else [])
        print(f"| {short} | " + " | ".join(line) + " |")
    print()
    print(f"## The allocations over {len(complete)} boards ({', '.join(complete)})")
    print("| allocation | unconnected | against today | strict passes |")
    print("| --- | ---: | ---: | ---: |")
    today = totals.get("a: n placements x 1 seed (today)", 0.0)
    for name, total in totals.items():
        print(f"| {name} | {total:.1f} | {100 * (total - today) / max(today, 1e-9):+.0f} % | {strict_totals[name]:.2f} |")
    print()
    print("## The probe's ripped count against the final result (Spearman)")
    print("| over | cases | mean | > 0 | < 0 |")
    print("| --- | ---: | ---: | ---: | ---: |")
    for name, values in correlations.items():
        values = [v for v in values if v is not None]
        if values:
            print(f"| {name} | {len(values)} | {sum(values) / len(values):+.2f} | "
                  f"{sum(v > 0 for v in values)} | {sum(v < 0 for v in values)} |")


if __name__ == "__main__":
    sys.exit(main())
