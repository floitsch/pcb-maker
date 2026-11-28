#!/usr/bin/env python3
# Copyright (C) 2026 Florian Loitsch.
"""The ceiling of a selection change, from stored layouts
(docs/experiments/race-seeds.md).

    ceiling.py STORED.json [--binary NAME] [--boards]

Over one binary's layouts at several placer seeds: per board KiCad's
unconnected items per seed, their mean (what a layout gives today) and their
least (what an oracle that picked among the seeds' finished boards would
give), and the strict passes likewise; then the sets pooled.
"""

import argparse
import json
import sys
from collections import defaultdict


def main():
    parser = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    parser.add_argument("stored")
    parser.add_argument("--binary", default="pcb-maker-main-cb7743eb")
    parser.add_argument("--boards", action="store_true")
    arguments = parser.parse_args()
    table = [row for row in json.load(open(arguments.stored))
             if row["binary"] == arguments.binary and row["seed"] is not None and "/rerun/" not in row["run"]]
    sets = defaultdict(lambda: defaultdict(dict))
    for row in table:
        name = row["run"].rsplit("/", 1)[-1].split("-s")[0]
        sets[name][row["board"]][row["seed"]] = row
    for name, boards in sorted(sets.items()):
        seeds = sorted({seed for by_seed in boards.values() for seed in by_seed})
        print(f"## {name} ({len(boards)} boards, seeds {seeds})")
        if arguments.boards:
            print("| board | " + " | ".join(f"s{seed}" for seed in seeds) + " | mean | least | to win | strict seeds |")
            print("| --- | " + " | ".join("---:" for _ in seeds) + " | ---: | ---: | ---: | ---: |")
        mean_total = least_total = first_total = 0.0
        strict_seeds = strict_any = strict_all = failing = 0
        per_seed = defaultdict(int)
        per_seed_strict = defaultdict(int)
        for board, by_seed in sorted(boards.items(), key=lambda item: -max(row["unconnected"] or 0 for row in item[1].values())):
            counts = [by_seed[seed]["unconnected"] for seed in seeds if seed in by_seed]
            if len(counts) != len(seeds) or None in counts:
                print(f"(skipped {board}: {counts})")
                continue
            passes = [by_seed[seed]["strict"] for seed in seeds]
            for seed in seeds:
                per_seed[seed] += by_seed[seed]["unconnected"]
                per_seed_strict[seed] += by_seed[seed]["strict"]
            mean = sum(counts) / len(counts)
            mean_total += mean
            least_total += min(counts)
            first_total += counts[0]
            strict_seeds += sum(passes)
            strict_any += any(passes)
            strict_all += all(passes)
            failing += not all(passes)
            if arguments.boards and (max(counts) or not all(passes)):
                print(f"| {board.split('__', 1)[-1]} | "
                      + " | ".join(f"{count}{'' if ok else '!'}" for count, ok in zip(counts, passes))
                      + f" | {mean:.1f} | {min(counts)} | {mean - min(counts):.1f} | {sum(passes)} |")
        count = len(seeds)
        print()
        print(f"Unconnected per seed: {dict(per_seed)}; strict per seed: {dict(per_seed_strict)}.")
        print(f"Mean over seeds {mean_total:.1f}, the oracle's least {least_total:.0f}: "
              f"{mean_total - least_total:.1f} to win ({100 * (mean_total - least_total) / max(mean_total, 1e-9):.0f} %).")
        print(f"Strict passes: mean per seed {strict_seeds / count:.1f}, boards passing on some seed {strict_any}, "
              f"on every seed {strict_all}: {strict_any - strict_seeds / count:.1f} to win.")
        print()


if __name__ == "__main__":
    sys.exit(main())
