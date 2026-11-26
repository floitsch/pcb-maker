#!/usr/bin/env python3
# Copyright (C) 2026 Florian Loitsch.
"""The totals of judge.sh's runs (docs/experiments/keep-arrangement.md).

    judge.py OUT-DIR [--set short|sweep] [--boards]

Per set and seed: KiCad's unconnected items, strict passes, passes, the
first routes' open terminals and the work, off against on, over the boards
with a result on both sides; then the seeds pooled, and with `--boards` each
board's unconnected and strict pass per seed.
"""

import argparse
import json
import re
import sys
from pathlib import Path


def rows(directory):
    results = directory / "results.json"
    if not results.exists():
        return {}
    return {row["name"]: row for row in json.loads(results.read_text())}


def unconnected(row):
    return (row.get("native") or {}).get("unconnected")


def main():
    parser = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    parser.add_argument("out")
    parser.add_argument("--set", dest="sets", action="append")
    parser.add_argument("--boards", action="store_true")
    arguments = parser.parse_args()
    out = Path(arguments.out)
    runs = {}
    for directory in sorted(out.glob("*-s*-off")):
        match = re.fullmatch(r"(.+)-s(\d+)-off", directory.name)
        if match and (not arguments.sets or match.group(1) in arguments.sets):
            runs.setdefault(match.group(1), []).append(int(match.group(2)))
    for name, seeds in runs.items():
        print(f"## {name}")
        print("| seed | boards with both | unconnected off -> on | strict off -> on | pass off -> on | first open off -> on | work s off -> on | without a result off, on | exit off, on |")
        print("| --- | ---: | --- | --- | --- | --- | --- | --- | --- |")
        pooled = [0] * 10
        boards = {}
        for seed in sorted(seeds):
            sides = [rows(out / f"{name}-s{seed}-{side}") for side in ("off", "on")]
            exits = [(out / f"{name}-s{seed}-{side}.exit").read_text().strip()
                     if (out / f"{name}-s{seed}-{side}.exit").exists() else "running" for side in ("off", "on")]
            both = [board for board in sides[0] if board in sides[1]
                    and unconnected(sides[0][board]) is not None and unconnected(sides[1][board]) is not None]
            missing = [sum(1 for row in side.values() if unconnected(row) is None) for side in sides]
            total = lambda side, key: sum(key(sides[side][board]) for board in both)  # noqa: E731
            numbers = []
            for key in (unconnected, lambda row: bool(row.get("pass_strict")), lambda row: bool(row.get("pass")),
                        lambda row: row.get("first_unconnected_terminals") or 0, lambda row: row.get("work_seconds") or 0):
                numbers += [total(0, key), total(1, key)]
            pooled = [a + b for a, b in zip(pooled, numbers)]
            print(f"| {seed} | {len(both)} | " + " | ".join(f"{numbers[i]:.0f} -> {numbers[i + 1]:.0f}" for i in range(0, 10, 2))
                  + f" | {missing[0]}, {missing[1]} | {exits[0]}, {exits[1]} |")
            for board in sorted(set(sides[0]) | set(sides[1])):
                boards.setdefault(board, {})[seed] = [
                    (unconnected(side[board]), bool(side[board].get("pass_strict"))) if board in side else None
                    for side in sides]
        print("| pooled | | " + " | ".join(f"{pooled[i]:.0f} -> {pooled[i + 1]:.0f}" for i in range(0, 10, 2)) + " | | |")
        print()
        if arguments.boards:
            print("| board | " + " | ".join(f"s{seed} off -> on" for seed in sorted(seeds)) + " | strict passes off -> on |")
            print("| --- | " + " | ".join("---" for _ in seeds) + " | --- |")
            for board, by_seed in sorted(boards.items()):
                cells = []
                strict = [0, 0]
                for seed in sorted(seeds):
                    pair = by_seed.get(seed) or [None, None]
                    cells.append(" -> ".join("-" if side is None or side[0] is None else f"{side[0]}{'' if side[1] else '!'}"
                                             for side in pair))
                    for index, side in enumerate(pair):
                        strict[index] += bool(side and side[1])
                print(f"| {board.split('__', 1)[-1]} | " + " | ".join(cells) + f" | {strict[0]} -> {strict[1]} |")
            print("(`!`: not a strict pass.)")
            print()


if __name__ == "__main__":
    sys.exit(main())
