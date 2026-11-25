#!/usr/bin/env python3
# Copyright (C) 2026 Florian Loitsch.
"""The endgame tier's table (README.md, "The endgame tier"): per board the
KiCad-unconnected items before (the frozen board's), after the resumed
route and after the local open repair, the strict verdict, and the work,
wall and CPU seconds; then the totals. With two or more logs, one column
group per log and a line per board where their final counts differ.

    benchmarks/agent-tasks/endgame_report.py build/endgame-x<n>.log [build/endgame-x<m>.log ...]"""

import argparse
import sys
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parent))
from compare import by_name, rows_of  # noqa: E402


def final_open(row):
    native = row.get("native")
    return None if native is None else native.get("unconnected")


def stage_open(row, stage):
    return ((row.get("endgame") or {}).get(stage) or {}).get(
        "unconnected" if stage == "route" else "opens_after")


def summary(rows):
    known = [row for row in rows if final_open(row) is not None]
    before = sum(row.get("unconnected_before") or 0 for row in known)
    after = sum(final_open(row) for row in known)
    return {
        "boards": len(rows),
        "no_result": len(rows) - len(known),
        "before": before,
        "after": after,
        "boards_closed_some": sum(1 for row in known if final_open(row) < (row.get("unconnected_before") or 0)),
        "boards_opened_some": sum(1 for row in known if final_open(row) > (row.get("unconnected_before") or 0)),
        "local_closed": sum(max(0, (stage_open(row, "route") or 0) - (stage_open(row, "local") or 0))
                            for row in known if stage_open(row, "local") is not None),
        "strict": sum(1 for row in rows if row.get("pass_strict")),
        "work_seconds": round(sum(row.get("work_seconds") or 0 for row in rows)),
        "seconds": round(sum(row.get("seconds") or 0 for row in rows)),
        "cpu_seconds": round(sum(row.get("cpu_seconds") or 0 for row in rows)),
    }


def text(value):
    return "-" if value is None else str(value)


def main():
    parser = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    parser.add_argument("logs", type=Path, nargs="+")
    arguments = parser.parse_args()
    runs = [by_name(rows_of(log)) for log in arguments.logs]
    names = sorted(set().union(*runs), key=lambda name: (name.split("@")[-1], name))
    width = max((len(name) for name in names), default=10)
    if len(runs) == 1:
        print(f"{'board':{width}}  before route local strict   work   wall    cpu")
        for name in names:
            row = runs[0][name]
            print(f"{name:{width}}  {text(row.get('unconnected_before')):>6} {text(stage_open(row, 'route')):>5} "
                  f"{text(stage_open(row, 'local')):>5} {'pass' if row.get('pass_strict') else 'fail':>6} "
                  f"{text(row.get('work_seconds')):>6} {text(row.get('seconds')):>6} {text(row.get('cpu_seconds')):>6}"
                  + (f"  {row['error'][:60]}" if row.get("error") else ""))
    else:
        print(f"{'board':{width}}  before  " + "  ".join(f"run{i + 1:<3}" for i in range(len(runs))))
        for name in names:
            rows = [run.get(name) or {} for run in runs]
            finals = [final_open(row) for row in rows]
            mark = "" if len(set(finals)) == 1 else "  differ"
            before = next((row.get("unconnected_before") for row in rows if row), None)
            print(f"{name:{width}}  {text(before):>6}  "
                  + "  ".join(f"{text(final) + ('*' if row.get('pass_strict') else ''):>6}"
                              for final, row in zip(finals, rows)) + mark)
        print("(* passes the strict rule)")
    for log, run in zip(arguments.logs, runs):
        totals = summary(list(run.values()))
        print(f"{log}: {totals['boards']} boards ({totals['no_result']} without a result): unconnected "
              f"{totals['before']} -> {totals['after']}, {totals['boards_closed_some']} boards closed some, "
              f"{totals['boards_opened_some']} opened some, the local repair closed {totals['local_closed']}; "
              f"strict passes {totals['strict']}; work {totals['work_seconds']} s, wall {totals['seconds']} s "
              f"summed, CPU {totals['cpu_seconds']} s")


if __name__ == "__main__":
    main()
