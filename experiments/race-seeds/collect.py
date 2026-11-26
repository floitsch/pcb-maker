#!/usr/bin/env python3
# Copyright (C) 2026 Florian Loitsch.
"""Collects stored layouts into one table (docs/experiments/race-seeds.md).

    collect.py OUT.json RUN-ROOT...

Walks each RUN-ROOT for run.py output directories (a `results.json` beside
one directory per board) and writes one row per layout: the board, the
placer seed, the binary, the race's probes and pick, the first route's open
terminals, KiCad's unconnected items, the strict pass and the work.
"""

import json
import os
import sys
from pathlib import Path


def load(path):
    try:
        return json.loads(path.read_text())
    except (OSError, ValueError):
        return None


def main():
    out = Path(sys.argv[1])
    table = []
    seen = set()
    for root in sys.argv[2:]:
        for directory, _, files in os.walk(root, followlinks=False):
            if "results.json" not in files:
                continue
            run = Path(directory)
            real = run.resolve()
            if real in seen:
                continue
            seen.add(real)
            for row in load(run / "results.json") or []:
                if row.get("mode") != "layout":
                    continue
                board = run / row["name"]
                report = load(board / "layout" / "board-layout.json") or {}
                task = load(board / "layout.json") or {}
                seed = ((task.get("placer") or {}).get("tuning") or {}).get("seed")
                race = report.get("race") or {}
                table.append({
                    "run": str(run),
                    "board": row["name"],
                    "seed": seed,
                    "placer": task.get("placer"),
                    "binary": Path(row.get("binary") or "").name,
                    "version": (row.get("environment") or {}).get("pcb_maker_version"),
                    "tasks": row.get("tasks"),
                    "unconnected": (row.get("native") or {}).get("unconnected"),
                    "strict": bool(row.get("pass_strict")),
                    "pass": bool(row.get("pass")),
                    "exit": row.get("exit"),
                    "verdict": row.get("verdict"),
                    "first_open": row.get("first_unconnected_terminals"),
                    "work": row.get("work_seconds"),
                    "total": report.get("total_seconds"),
                    "wall_stretch": row.get("wall_stretch"),
                    "digest": report.get("placement_digest"),
                    "pre_rank": report.get("placement_race"),
                    "kept": race.get("kept"),
                    "best": race.get("best"),
                    "confident": race.get("confident"),
                    "probe_seconds": race.get("probe_seconds"),
                    "dropped": race.get("dropped"),
                    "probes": [
                        {key: probe.get(key) for key in
                         ("placement", "open_terminals", "ripped_open_terminals", "unfinished", "conflicted", "work_seconds", "gated")}
                        for probe in race.get("probe") or []],
                    "open_terminals": (report.get("routed") or {}).get("unconnected_terminals"),
                })
    out.write_text(json.dumps(table, indent=1))
    print(f"{len(table)} layouts from {len(seen)} runs -> {out}")


if __name__ == "__main__":
    sys.exit(main())
