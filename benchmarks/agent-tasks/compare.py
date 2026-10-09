#!/usr/bin/env python3
# Copyright (C) 2026 Toit contributors.
"""Compares two layout sweeps board by board, on KiCad's connectivity.

    benchmarks/agent-tasks/compare.py <before.log> <after.log> [--splice <rerun.log> ...]

Each argument is a runner log (its JSON lines) or an output directory (its
results.json). A spliced log replaces the after-run's rows of the same
boards, so a board rerun on a newer binary counts with the sweep. The
table lists the boards of the after-run that the before-run has too,
sorted by the change of KiCad's unconnected count, then the boards only
one run has, then the totals: better / same / worse, unconnected in all,
passes."""

import argparse
import json
from pathlib import Path


def rows_of(path):
    path = Path(path)
    if path.is_dir():
        results = path / "results.json"
        if results.exists():
            return json.loads(results.read_text())
        path = path.with_suffix(".log")
    rows = []
    for line in path.read_text().splitlines():
        if line.startswith("{"):
            try:
                rows.append(json.loads(line))
            except json.JSONDecodeError:
                pass
    return rows


def by_name(rows):
    table = {}
    for row in rows:
        table[row["name"]] = row  # the last row of a board wins
    return table


def unconnected(row):
    native = row.get("native")
    if row.get("error") or native is None:
        return None
    return native.get("unconnected")


def errors(row):
    native = row.get("native") or {}
    return sum((native.get("errors") or {}).values())


def passes(row):
    """The pass rule of the runner (KiCad's connectivity, no copper error,
    no unexpected constraint miss), applied again: rows written before the
    rule changed carry the old flag."""
    if row is None or row.get("error") or row.get("native") is None:
        return False
    return unconnected(row) == 0 and not errors(row) and not row.get("unexpected_misses")


def cell(row):
    if row is None:
        return "-"
    if row.get("error"):
        return f"ERROR ({str(row['error'])[:30]})"
    text = f"{row.get('routed', '?')} {unconnected(row)}"
    if errors(row):
        text += f" +{errors(row)}e"
    if passes(row):
        text += " pass"
    return text


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("before")
    parser.add_argument("after")
    parser.add_argument("--splice", nargs="*", default=[])
    parser.add_argument("--markdown", action="store_true")
    arguments = parser.parse_args()
    before = by_name(rows_of(arguments.before))
    after = by_name(rows_of(arguments.after))
    for path in arguments.splice:
        after.update(by_name(rows_of(path)))
    common = sorted(set(before) & set(after))

    def delta(name):
        a, b = unconnected(after[name]), unconnected(before[name])
        if a is None or b is None:
            return float("inf")
        return a - b

    common.sort(key=lambda n: (delta(n), n))
    width = max((len(n) for n in list(before) + list(after)), default=10)
    sep = " | " if arguments.markdown else "  "
    print(f"{'board':{width}}{sep}{'before':>24}{sep}{'after':>24}{sep}{'s':>6}")
    if arguments.markdown:
        print(f"{'-' * width} | {'-' * 24} | {'-' * 24} | {'-' * 6}")
    better = same = worse = 0
    total_before = total_after = 0
    for name in common:
        b, a = before[name], after[name]
        ub, ua = unconnected(b), unconnected(a)
        if ub is not None and ua is not None:
            total_before += ub
            total_after += ua
            if ua < ub:
                better += 1
            elif ua > ub:
                worse += 1
            else:
                same += 1
        print(f"{name:{width}}{sep}{cell(b):>24}{sep}{cell(a):>24}{sep}{round(a.get('seconds', 0)):>6}")
    only_after = sorted(set(after) - set(before))
    only_before = sorted(set(before) - set(after))
    for name in only_after:
        a = after[name]
        print(f"{name:{width}}{sep}{'-':>24}{sep}{cell(a):>24}{sep}{round(a.get('seconds', 0)):>6}")
    for name in only_before:
        print(f"{name:{width}}{sep}{cell(before[name]):>24}{sep}{'-':>24}{sep}{'':>6}")
    passes_before = sum(1 for n in common if passes(before[n]))
    passes_after = sum(1 for n in common if passes(after[n]))
    print()
    print(f"{len(common)} boards in both: better {better}, same {same}, worse {worse}; "
          f"unconnected {total_before} -> {total_after}; passes {passes_before} -> {passes_after}")
    if only_after:
        print(f"{len(only_after)} boards only after: {sum(1 for n in only_after if passes(after[n]))} pass")
    if only_before:
        print(f"{len(only_before)} boards only before")


if __name__ == "__main__":
    main()
