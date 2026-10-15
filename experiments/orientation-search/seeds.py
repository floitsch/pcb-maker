#!/usr/bin/env python3
# Copyright (C) 2026 Toit contributors.
# Use of this source code is governed by an MIT-style license that can be
# found in the LICENSE file.
"""Placement only, one seed per run, over seeds, starts and binaries or
placer options: the final weighted HPWL of each, side by side.

    seeds.py <out dir> --board name:dir:board_id ... --variant name:binary[:json options] ...
             [--seeds 1-6] [--jobs 6]

Each run is `place-kicad-board <dir> <board_id> <out>/<board>-<variant>-<seed>`
(a variant's `_env` option sets environment variables)
with `{"constraints": "constraints.json", "placement_seeds": 1, "seed": s}`
plus the variant's options, one search thread each. Prints a table of
HPWL per board and variant (median and the seeds).
"""
import argparse
import concurrent.futures
import json
import os
import re
import statistics
import subprocess
from pathlib import Path


def run(job):
    out, binary, directory, board_id, options = job
    log = Path(str(out) + ".log")
    if not (out / "board-placer.json").exists():
        out.parent.mkdir(parents=True, exist_ok=True)
        config = Path(str(out) + ".json")
        env = dict(os.environ, RAYON_NUM_THREADS="1", **options.pop("_env", {}))
        config.write_text(json.dumps(options))
        subprocess.run(["rm", "-rf", str(out)])
        with open(log, "w") as handle:
            subprocess.run([binary, "place-kicad-board", directory, board_id, str(out), str(config)],
                           stdout=handle, stderr=subprocess.STDOUT, env=env)
    try:
        report = json.loads((out / "board-placer.json").read_text())
        return report["wirelength_final_mm"], report.get("work_seconds")
    except (OSError, ValueError, KeyError):
        return None, None


def main():
    parser = argparse.ArgumentParser()
    parser.add_argument("out", type=Path)
    parser.add_argument("--board", action="append", default=[])
    parser.add_argument("--variant", action="append", default=[])
    parser.add_argument("--seeds", default="1-6")
    parser.add_argument("--jobs", type=int, default=6)
    arguments = parser.parse_args()
    low, high = (int(part) for part in arguments.seeds.split("-"))
    seeds = list(range(low, high + 1))
    jobs, keys = [], []
    for board in arguments.board:
        name, directory, board_id = board.split(":")
        for variant in arguments.variant:
            vname, binary, *rest = variant.split(":", 2)
            options = {"constraints": "constraints.json", "placement_seeds": 1}
            if rest:
                options.update(json.loads(rest[0]))
            for seed in seeds:
                jobs.append((arguments.out / f"{name}-{vname}-{seed}", binary, directory, board_id,
                             json.loads(json.dumps(dict(options, seed=seed)))))
                keys.append((name, vname, seed))
    with concurrent.futures.ThreadPoolExecutor(arguments.jobs) as pool:
        results = list(pool.map(run, jobs))
    table = {}
    for (name, vname, seed), (hpwl, work) in zip(keys, results):
        table.setdefault((name, vname), []).append((seed, hpwl, work))
    for (name, vname), rows in table.items():
        values = [hpwl for _, hpwl, _ in rows if hpwl is not None]
        works = [work for _, _, work in rows if work is not None]
        median = statistics.median(values) if values else None
        print(f"{name:14} {vname:22} median {median and round(median):>7}  mean {values and round(statistics.mean(values)):>7}  "
              f"work {works and round(statistics.mean(works), 1)}  seeds " + " ".join(str(round(h)) if h else "-" for _, h, _ in rows))


if __name__ == "__main__":
    main()
