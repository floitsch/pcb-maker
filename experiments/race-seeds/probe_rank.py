#!/usr/bin/env python3
# Copyright (C) 2026 Florian Loitsch.
"""How well a layout's early counts predict its final result, from stored
layouts (docs/experiments/race-seeds.md).

    probe_rank.py STORED.json

Within a board, over its stored layouts (placer seeds, binaries): Spearman's
rank correlation between KiCad's unconnected items and the kept probe's
ripped count, its open terminals, and the first route's open terminals.
Only the kept placement of a layout has a final result, so this is the
kept probes' predictiveness, not the race's ranking of its candidates.
"""

import json
import sys
from collections import defaultdict


def ranks(values):
    order = sorted(range(len(values)), key=lambda index: values[index])
    result = [0.0] * len(values)
    index = 0
    while index < len(order):
        end = index
        while end + 1 < len(order) and values[order[end + 1]] == values[order[index]]:
            end += 1
        for position in range(index, end + 1):
            result[order[position]] = (index + end) / 2
        index = end + 1
    return result


def spearman(left, right):
    a, b = ranks(left), ranks(right)
    mean_a, mean_b = sum(a) / len(a), sum(b) / len(b)
    cov = sum((x - mean_a) * (y - mean_b) for x, y in zip(a, b))
    var = (sum((x - mean_a) ** 2 for x in a) * sum((y - mean_b) ** 2 for y in b)) ** 0.5
    return cov / var if var else None


def kept_probe(row):
    for probe in row["probes"]:
        if probe["placement"] == row["kept"]:
            return probe
    return None


def main():
    table = json.load(open(sys.argv[1]))
    for title, keep in (("main cb7743eb, placer seeds 1, 101, 201", lambda row: row["binary"] == "pcb-maker-main-cb7743eb"),
                        ("every stored layout (binaries mixed)", lambda row: True)):
        boards = defaultdict(list)
        for row in table:
            if keep(row) and row["unconnected"] is not None and kept_probe(row):
                boards[row["board"]].append(row)
        print(f"## {title}")
        print("| predictor | boards | layouts | mean Spearman | median | boards > 0 | boards < 0 | pooled (ranks within board) |")
        print("| --- | ---: | ---: | ---: | ---: | ---: | ---: | ---: |")
        for label, key in (("kept probe, ripped count", lambda row: kept_probe(row)["ripped_open_terminals"]),
                           ("kept probe, open terminals", lambda row: kept_probe(row)["open_terminals"]),
                           ("first route, open terminals", lambda row: row["first_open"])):
            values = []
            pooled_x, pooled_y = [], []
            layouts = 0
            for rows in boards.values():
                rows = [row for row in rows if key(row) is not None]
                final = [row["unconnected"] for row in rows]
                if len(rows) < 3 or max(final) - min(final) < 3:
                    continue
                value = spearman([key(row) for row in rows], final)
                if value is None:
                    continue
                values.append(value)
                layouts += len(rows)
                # Ranks centred within the board, so boards pool.
                x, y = ranks([key(row) for row in rows]), ranks(final)
                pooled_x += [(v - sum(x) / len(x)) / len(x) for v in x]
                pooled_y += [(v - sum(y) / len(y)) / len(y) for v in y]
            values.sort()
            pooled = spearman(pooled_x, pooled_y) if pooled_x else None
            print(f"| {label} | {len(values)} | {layouts} | {sum(values) / len(values):+.2f} | {values[len(values) // 2]:+.2f} | "
                  f"{sum(v > 0 for v in values)} | {sum(v < 0 for v in values)} | {pooled:+.2f} |")
        print()


if __name__ == "__main__":
    sys.exit(main())
