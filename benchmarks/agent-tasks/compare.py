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
passes. A row the wall clock cut short (its `cut` field: the layout's
deadline under load, the harness's timeout) depends on the machine's load,
not only on the binary: it is marked CUT and left out of better / same /
worse. A row copied from an earlier log (run.py --splice-from) is marked
SPL and counted in the totals. Below the totals, each run's continuous
objectives over the boards in both (README.md, "How to measure")."""

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


def cut(row):
    """Whether the wall clock, not the work, ended part of the run."""
    return row is not None and bool(row.get("cut"))


def spliced(row):
    return row is not None and bool(row.get("spliced"))


def objectives(rows):
    """Continuous measures over `rows`, usable from one run: KiCad's
    unconnected items and new copper errors, vias, length, CPU and work
    seconds, rows cut by the wall clock, and the router's unconnected
    terminals after the first route (layouts only). Rows without a result
    (an error, a timeout) add only to `failed`."""
    results = [row for row in rows if unconnected(row) is not None]
    with_work = [row for row in rows if row.get("work_seconds") is not None]
    with_first = [row for row in rows if row.get("first_unconnected_terminals") is not None]
    return {
        "rows": len(rows),
        "passes": sum(1 for row in rows if passes(row)),
        "failed": len(rows) - len(results),
        "unconnected": sum(unconnected(row) for row in results),
        "new_errors": sum(errors(row) for row in results),
        "vias": sum(row.get("vias") or 0 for row in results),
        "length_mm": round(sum(row.get("length_mm") or 0 for row in results), 1),
        "cpu_seconds": round(sum(row.get("cpu_seconds") or 0 for row in rows)),
        "work_seconds": round(sum(row["work_seconds"] for row in with_work)),
        "work_rows": len(with_work),
        "cut": sum(1 for row in rows if cut(row)),
        "first_unconnected_terminals": sum(row["first_unconnected_terminals"] for row in with_first),
        "first_rows": len(with_first),
    }


def objectives_line(o, label="objectives"):
    text = (f"{label}: unconnected {o['unconnected']}, new errors {o['new_errors']}, vias {o['vias']}, "
            f"length {o['length_mm']:.0f} mm, CPU {o['cpu_seconds']} s")
    if o["work_rows"]:
        text += f", work {o['work_seconds']} s"
        if o["work_rows"] != o["rows"]:
            text += f" ({o['work_rows']} rows)"
    text += f", cut {o['cut']}"
    if o["first_rows"]:
        text += f", first-route open terminals {o['first_unconnected_terminals']}"
        if o["first_rows"] != o["rows"]:
            text += f" ({o['first_rows']} rows)"
    text += f"; {o['rows']} rows"
    if o["failed"]:
        text += f", {o['failed']} without a result"
    return text


def cell(row):
    if row is None:
        return "-"
    if row.get("error"):
        text = f"ERROR ({str(row['error'])[:30]})"
        text += " CUT" if cut(row) else ""
        return text + " SPL" if spliced(row) else text
    text = f"{row.get('routed', '?')} {unconnected(row)}"
    if errors(row):
        text += f" +{errors(row)}e"
    if passes(row):
        text += " pass"
    if cut(row):
        text += " CUT"
    if spliced(row):
        text += " SPL"
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
    better = same = worse = cut_rows = 0
    total_before = total_after = 0
    for name in common:
        b, a = before[name], after[name]
        ub, ua = unconnected(b), unconnected(a)
        if ub is not None and ua is not None:
            total_before += ub
            total_after += ua
            if cut(a) or cut(b):
                cut_rows += 1
            elif ua < ub:
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
          f"unconnected {total_before} -> {total_after}; passes {passes_before} -> {passes_after}"
          + (f"; {cut_rows} cut by the wall clock in either run (not in better/same/worse)" if cut_rows else ""))
    for label, table in (("before", before), ("after", after)):
        count = sum(1 for n in common if spliced(table[n]))
        if count:
            print(f"{label}: {count} of the {len(common)} rows spliced from an earlier run (SPL)")
    print(objectives_line(objectives([before[n] for n in common]), "before"))
    print(objectives_line(objectives([after[n] for n in common]), "after "))
    if only_after:
        print(f"{len(only_after)} boards only after: {sum(1 for n in only_after if passes(after[n]))} pass")
    if only_before:
        print(f"{len(only_before)} boards only before")


if __name__ == "__main__":
    main()
