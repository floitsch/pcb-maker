#!/usr/bin/env python3
# Copyright (C) 2026 Toit contributors.
"""Freerouting against our cold route, board by board.

    benchmarks/github/freerouting_compare.py <ours-cold.log> <freerouting.log>... [--markdown]

The logs are freerouting.py's JSON lines: the first run with `--ours`
(its `cold_route` rows), the others Freerouting's (`freerouting` rows;
a later log's row of a board replaces an earlier one's). Both route the
same cold board (the designer's placement, every track and via removed)
under the same wall-clock limit; the counts are KiCad's after a zone
refill, errors beyond the designer's own board's. The table lists the
boards both have, then the totals."""

import argparse
import json
from pathlib import Path


def rows_of(path):
    rows = {}
    for line in Path(path).read_text().splitlines():
        if line.startswith("{"):
            try:
                row = json.loads(line)
            except json.JSONDecodeError:
                continue
            rows[row["name"]] = row
    return rows


def errors(native):
    return sum((native or {}).get("errors", {}).values())


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("ours")
    parser.add_argument("freerouting", nargs="+")
    parser.add_argument("--markdown", action="store_true")
    arguments = parser.parse_args()
    ours = rows_of(arguments.ours)
    theirs = {}
    for path in arguments.freerouting:
        theirs.update(rows_of(path))
    common = sorted(name for name in ours if name in theirs and "cold_route" in ours[name] and "freerouting" in theirs[name])
    sep = " | " if arguments.markdown else "  "
    width = max((len(n) for n in common), default=10)
    print(f"{'board':{width}}{sep}{'Freerouting':>30}{sep}{'pcb-maker':>34}")
    if arguments.markdown:
        print(f"{'-' * width} | {'-' * 30} | {'-' * 34}")
    finished = timed_out = 0
    ours_fewer = theirs_fewer = 0
    total_theirs = total_ours = 0
    clean_theirs = clean_ours = 0
    for name in common:
        f = theirs[name]["freerouting"]
        o = ours[name]["cold_route"]
        status = f.get("status")
        f_native = f.get("native") or {}
        f_open = f_native.get("unconnected")
        if status == "finished" and f_open is not None:
            finished += 1
            f_cell = f"{f_open} open +{errors(f_native)}e {f.get('router_seconds', 0):.0f} s"
        else:
            timed_out += status == "timeout"
            f_cell = f"{status} {f.get('router_seconds', 0):.0f} s"
        o_native = o.get("native") or {}
        o_open = o_native.get("unconnected")
        o_cell = f"{o['routed']}/{o['connections']} {o_open} open +{errors(o_native)}e {o.get('wall_seconds', 0):.0f} s"
        print(f"{name:{width}}{sep}{f_cell:>30}{sep}{o_cell:>34}")
        if status == "finished" and f_open is not None and o_open is not None:
            ours_fewer += o_open < f_open
            theirs_fewer += o_open > f_open
            total_theirs += f_open
            total_ours += o_open
            clean_theirs += f_open == 0 and errors(f_native) == 0
            clean_ours += o_open == 0 and errors(o_native) == 0
    print()
    print(f"{len(common)} boards; Freerouting finished {finished}, timed out {timed_out}, failed {len(common) - finished - timed_out}")
    print(f"where Freerouting finished: pcb-maker fewer unconnected on {ours_fewer}, Freerouting fewer on {theirs_fewer}; "
          f"unconnected in all: Freerouting {total_theirs}, pcb-maker {total_ours}; clean boards: Freerouting {clean_theirs}, pcb-maker {clean_ours}")


if __name__ == "__main__":
    main()
