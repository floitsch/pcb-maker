#!/usr/bin/env python3
# Copyright (C) 2026 Toit contributors.
"""Per-board comparison of layout runs (benchmarks/agent-tasks/run.py
outputs) with different placement-race settings: placement wall time,
the race's probe results, open terminals after the first route, the final
row (KiCad unconnected, pass).

    experiments/congestion/race_compare.py build/quick-pre-base build/quick-pre-rudy build/quick-pre-model"""

import json
import sys
from pathlib import Path


def load(directory):
    directory = Path(directory)
    rows = {}
    for line in Path(str(directory) + ".log").read_text().splitlines():
        if not line.startswith("{"):
            continue
        row = json.loads(line)
        report = directory / row["name"] / "layout/board-layout.json"
        log = directory / row["name"] / "layout.log"
        layout = json.loads(report.read_text()) if report.exists() else {}
        placed = None
        if log.exists():
            for text in log.read_text().splitlines():
                if text.startswith("layout: placed ("):
                    placed = float(text.split("(")[1].split(" s")[0])
        native = row.get("native") or {}
        rows[row["name"]] = {
            "pass": row.get("pass"), "unconnected": native.get("unconnected"),
            "routed": row.get("routed"), "first_open": layout.get("first_unconnected_terminals"),
            "race": layout.get("placement_race"), "probed": (layout.get("congestion") or {}).get("probed"),
            "placed_s": placed, "seconds": row.get("seconds"), "error": row.get("error"),
        }
    return rows


def main():
    runs = [load(directory) for directory in sys.argv[1:]]
    names = [Path(directory).name for directory in sys.argv[1:]]
    boards = sorted(set().union(*runs))
    print("| board | " + " | ".join(f"{n}: race / first open / KiCad unconnected / pass / placed s" for n in names) + " |")
    print("| --- |" + " --- |" * len(names))
    totals = [[0, 0, 0, 0, 0] for _ in runs]
    common = [b for b in boards if all(b in run for run in runs)]
    for board in boards:
        cells = []
        for index, run in enumerate(runs):
            row = run.get(board)
            if row is None:
                cells.append("-")
                continue
            race = "" if not row["race"] else min(row["race"])
            cells.append(f"{race} / {row['first_open']} / {row['unconnected']} / {'P' if row['pass'] else '-'} / "
                         f"{row['placed_s'] if row['placed_s'] is not None else '?'}")
            if board in common:
                totals[index][0] += row["first_open"] or 0
                totals[index][1] += row["unconnected"] or 0
                totals[index][2] += 1 if row["pass"] else 0
                totals[index][3] += min(row["race"]) if row["race"] else 0
                totals[index][4] += row["placed_s"] or 0
        print(f"| {board[:45]} | " + " | ".join(cells) + " |")
    print(f"\nOn the {len(common)} boards all runs have:")
    for name, total in zip(names, totals):
        print(f"  {name}: race best (sum) {total[3]}, first-route open {total[0]}, KiCad unconnected {total[1]}, "
              f"passes {total[2]}, placement wall {total[4]:.0f} s")
    for index in range(1, len(runs)):
        better = worse = same = 0
        for board in common:
            a, b = runs[0][board]["unconnected"], runs[index][board]["unconnected"]
            if a is None or b is None:
                continue
            better += b < a
            worse += b > a
            same += b == a
        print(f"  {names[index]} against {names[0]} (KiCad unconnected): better {better}, same {same}, worse {worse}")


if __name__ == "__main__":
    main()
