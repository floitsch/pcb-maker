#!/usr/bin/env python3
# Copyright (C) 2026 Toit contributors.
"""How reproducible the race's judge is: the same task placements probed
under several router seeds (PCB_ROUTER_SEED). Within each board, the
Spearman agreement of one probe with another, and of the mean of two
probes with the mean of two others.

    experiments/congestion/noise.py build/cdata/samples build/cnoise2/samples-r1 build/cnoise2/samples-r2 ..."""

import json
import re
import sys
from pathlib import Path

import numpy as np

sys.path.insert(0, str(Path(__file__).resolve().parent))
from evaluate import spearman  # noqa: E402


def load(directory, boards):
    out = {}
    for board in boards:
        for path in (Path(directory) / board).glob("task-seed*.json"):
            if re.fullmatch(r"task-seed\d+", path.stem):
                out[(board, path.stem)] = json.loads(path.read_text())["probe"]["unfinished"]
    return out


def main():
    directories = sys.argv[1:]
    boards = sorted({p.name for p in Path(directories[1]).iterdir() if p.is_dir()})
    runs = [load(directory, boards) for directory in directories]
    singles, pairs = [], []
    for board in boards:
        keys = sorted(set.intersection(*[{k for k in run if k[0] == board} for run in runs]))
        if len(keys) < 4:
            continue
        values = np.array([[run[k] for k in keys] for run in runs], dtype=float)
        if values.std(axis=1).min() == 0:
            continue
        rhos = [spearman(values[i], values[j]) for i in range(len(runs)) for j in range(i + 1, len(runs))]
        singles.append(np.nanmean(rhos))
        if len(runs) >= 4:
            pairs.append(spearman(values[:2].mean(0), values[2:4].mean(0)))
        print(f"{board[:45]:>45}: {len(keys)} placements, one probe against another {np.nanmean(rhos):+.2f}"
              + (f", 2-probe mean against 2-probe mean {pairs[-1]:+.2f}" if len(runs) >= 4 else ""))
    print(f"mean over {len(singles)} boards: single {np.nanmean(singles):+.3f}"
          + (f", 2-probe means {np.nanmean(pairs):+.3f}" if pairs else ""))


if __name__ == "__main__":
    main()
