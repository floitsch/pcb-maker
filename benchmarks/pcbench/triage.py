#!/usr/bin/env python3
# Copyright (C) 2026 Toit contributors.
"""Sorts the boards of a run.py output that are not clean by cause.

    benchmarks/pcbench/triage.py <run-output> [--mode route]

- **error.** The board was not routed at all: adapter error or timeout.
- **mismatch.** KiCad finds unconnected items although the router thinks
  every terminal is connected: the router's model of the board differs from
  KiCad's. These are bugs.
- **drc.** KiCad finds copper errors: clearance model differences, also bugs.
- **open.** The router left connections open: routing difficulty."""

import argparse
import json
from collections import defaultdict
from pathlib import Path


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("output", type=Path)
    parser.add_argument("--mode", default="route")
    arguments = parser.parse_args()
    classes = defaultdict(list)
    for line in (arguments.output / "results.jsonl").read_text().splitlines():
        row = json.loads(line)
        entry = row.get(arguments.mode, {})
        if entry.get("clean"):
            continue
        native = entry.get("native") or {}
        if "routed" not in entry:
            detail = entry.get("error", "")
            classes["error"].append((row["name"], row["tier"], str(entry.get("exit")) + " " + detail[:120]))
            continue
        report = arguments.output / "boards" / row["name"] / arguments.mode / "board-router.json"
        open_terminals = None
        if report.exists():
            open_terminals = json.loads(report.read_text()).get("unconnected_terminals")
        if native.get("errors"):
            classes["drc"].append((row["name"], row["tier"], json.dumps(native["errors"])))
        elif open_terminals == 0 and native.get("unconnected"):
            classes["mismatch"].append((row["name"], row["tier"], f"{native['unconnected']} unconnected in KiCad"))
        else:
            classes["open"].append((row["name"], row["tier"],
                                    f"{entry['routed']}/{entry['connections']} nets, {open_terminals} open terminals"))
    for kind in ("error", "mismatch", "drc", "open"):
        print(f"## {kind} ({len(classes[kind])})")
        for name, tier, detail in sorted(classes[kind]):
            print(f"- {name} ({tier}): {detail}")
        print()


if __name__ == "__main__":
    main()
