#!/usr/bin/env python3
# Copyright (C) 2026 Toit contributors.
"""Finds the boards whose result flips between tier runs (the short tier's
selection, README.md "The short tier").

    benchmarks/agent-tasks/flippers.py build/quick30-x644.log build/quick30-x651.log ... [--markdown]

The logs are taken in the order given (one round each; a round is a run.py
log or output directory, compare.py reads both). Per board: how often its
pass verdict changed from one round to the next, how often KiCad's
unconnected count changed, how often its copper error count changed, the
range of its unconnected count, its median seconds and CPU seconds, and
how many rounds the wall clock cut. Sorted by flips (pass changes plus
unconnected changes), then by error changes, then by median seconds: the
sensitive boards first, and among equals the fast ones."""

import argparse
import statistics
import sys
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parent))
from compare import by_name, cut, errors, passes, rows_of, unconnected  # noqa: E402


def changes(values):
    """How often a value differs from the one before (None, an error row,
    counts as a value of its own)."""
    return sum(1 for a, b in zip(values, values[1:]) if a != b)


def table(paths):
    rounds = [by_name(rows_of(path)) for path in paths]
    names = sorted(set().union(*rounds))
    result = []
    for name in names:
        rows = [r.get(name) for r in rounds]
        present = [row for row in rows if row is not None]
        verdicts = [passes(row) for row in present]
        opens = [unconnected(row) for row in present]
        copper = [errors(row) if unconnected(row) is not None else None for row in present]
        seconds = [row["seconds"] for row in present if row.get("seconds") is not None]
        cpu = [row["cpu_seconds"] for row in present if row.get("cpu_seconds") is not None]
        known = [o for o in opens if o is not None]
        result.append({
            "name": name,
            "rounds": len(present),
            "pass_flips": changes(verdicts),
            "unconnected_flips": changes(opens),
            "error_flips": changes(copper),
            "flips": changes(verdicts) + changes(opens),
            "passes": sum(verdicts),
            "unconnected": f"{min(known)}-{max(known)}" if known else "-",
            "seconds": round(statistics.median(seconds)) if seconds else None,
            "cpu_seconds": round(statistics.median(cpu)) if cpu else None,
            "cut": sum(1 for row in present if cut(row)),
        })
    result.sort(key=lambda r: (-r["flips"], -r["error_flips"],
                               r["seconds"] if r["seconds"] is not None else float("inf"), r["name"]))
    return result


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("logs", nargs="+")
    parser.add_argument("--markdown", action="store_true")
    arguments = parser.parse_args()
    rows = table(arguments.logs)
    columns = ["board", "rounds", "flips", "pass", "unconn", "errors", "passes", "unconnected", "s", "cpu s", "cut"]
    width = max(len(r["name"]) for r in rows)
    if arguments.markdown:
        print("| " + " | ".join(columns) + " |")
        print("| --- |" + " ---: |" * (len(columns) - 1))
    else:
        print(f"{'board':{width}}  " + "  ".join(f"{c:>11}" for c in columns[1:]))
    for r in rows:
        cells = [r["rounds"], r["flips"], r["pass_flips"], r["unconnected_flips"], r["error_flips"],
                 r["passes"], r["unconnected"], r["seconds"], r["cpu_seconds"], r["cut"]]
        cells = ["-" if c is None else c for c in cells]
        if arguments.markdown:
            print(f"| {r['name']} | " + " | ".join(str(c) for c in cells) + " |")
        else:
            print(f"{r['name']:{width}}  " + "  ".join(f"{str(c):>11}" for c in cells))
    print(f"\n{len(arguments.logs)} rounds; flips = pass changes + unconnected changes between consecutive rounds; "
          "errors = changes of the copper error count; s and cpu s are medians")


if __name__ == "__main__":
    main()
