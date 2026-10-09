#!/usr/bin/env python3
# Copyright (C) 2026 Toit contributors.
"""Compares probes of the same placements at different work budgets: how
well the unfinished count (and tile labels summed) after a short probe
ranks placements the way a longer probe does.

    experiments/congestion/budgets.py build/congestion-pilot/samples-10 build/congestion-pilot/samples-25 ..."""

import json
import sys
from pathlib import Path

import numpy as np

sys.path.insert(0, str(Path(__file__).resolve().parent))
from dataset import read_sample  # noqa: E402
from evaluate import spearman  # noqa: E402


def load(directory):
    out = {}
    for path in Path(directory).glob("*/*.json"):
        header, arrays = read_sample(path)
        if "probe" not in header:
            continue
        row = {"unfinished": header["probe"]["unfinished"], "conflicted": header["probe"]["conflicted_nets"],
               "incomplete": header["probe"]["incomplete_nets"], "expansions": header["probe"]["expansions"],
               "seconds": header["probe"]["seconds"], "wirelength": header["wirelength_mm"]}
        for name in ("conflict", "history", "tile_history", "usage"):
            row[name] = float(arrays[name].sum())
        out[(path.parent.name, path.stem)] = row
    return out


def main():
    budgets = [load(directory) for directory in sys.argv[1:]]
    names = [Path(directory).name for directory in sys.argv[1:]]
    longest = budgets[-1]
    for name, budget in zip(names, budgets):
        keys = sorted(set(budget) & set(longest))
        print(f"{name}: {len(budget)} samples, mean probe {np.mean([r['seconds'] for r in budget.values()]):.1f} s wall, "
              f"{np.mean([r['expansions'] for r in budget.values()]) / 1e6:.1f}M expansions")
        truth = [longest[key]["unfinished"] for key in keys]
        for field in ("unfinished", "conflicted", "incomplete", "conflict", "history", "tile_history", "wirelength"):
            values = [budget[key][field] for key in keys]
            pooled = spearman(values, truth)
            boards = {}
            for key, value, target in zip(keys, values, truth):
                boards.setdefault(key[0], []).append((value, target))
            within = [spearman([v for v, _ in rows], [t for _, t in rows]) for rows in boards.values() if len(rows) >= 3]
            within = [rho for rho in within if not np.isnan(rho)]
            print(f"  {field:>13} vs {names[-1]} unfinished: pooled {pooled:+.3f}, within-board {np.mean(within) if within else float('nan'):+.3f} ({len(within)} boards)")


if __name__ == "__main__":
    main()
