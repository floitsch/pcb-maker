#!/usr/bin/env python3
# Copyright (C) 2026 Toit contributors.
"""Sets a tier's per-board estimates (`peak_rss_mb`, `seconds`: what
run.py's memory gate reserves for a board and how long it expects it to
run) from the rows of runs of that tier: per board the largest of its
rows' values. Other fields of a board's entry stay; boards without a row
keep their estimates. Prints the boards whose peak changed most.

    benchmarks/agent-tasks/tier_estimates.py benchmarks/agent-tasks/designer.json \\
        --placement designer build/nightly/2026-10-24/designer.log ...

Until 2026-10-26 the designer tier's estimates were its boards' v17 layout
rows (x1170: placement and routing), an upper bound for a route on the
designer's placement that was 1.8 times the measured peak at the median
and 34 times on the ULK boards (23 215 MB expected, 674-1512 MB
measured): the nightly's last boards waited for 33-38 GB of available
memory they never used.
"""

import argparse
import json
import sys
from pathlib import Path

sys.path.append(str(Path(__file__).resolve().parents[1]))
import harness_common  # noqa: E402


def measured(logs, placement=None):
    """Per board name the largest `peak_rss_mb` and `seconds` of its rows
    in `logs` (JSON lines; other lines are skipped), of `placement` where
    given, and how many rows."""
    found = {}
    for log in logs:
        for line in Path(log).read_text().splitlines():
            if not line.startswith("{"):
                continue
            try:
                row = json.loads(line)
            except ValueError:
                continue
            if not isinstance(row, dict) or "name" not in row or not row.get("peak_rss_mb"):
                continue
            if placement is not None and row.get("placement") != placement:
                continue
            entry = found.setdefault(row["name"], {"peak_rss_mb": 0, "seconds": 0.0, "rows": 0})
            entry["peak_rss_mb"] = max(entry["peak_rss_mb"], int(row["peak_rss_mb"]))
            entry["seconds"] = max(entry["seconds"], float(row.get("seconds") or 0.0))
            entry["rows"] += 1
    return found


def update(tier, found):
    """Sets the estimates of `tier`'s boards from `found` (measured);
    returns (name, old peak, new peak) of the boards that changed."""
    changed = []
    for name, entry in tier["boards"].items():
        if name not in found:
            continue
        old = entry.get("peak_rss_mb")
        entry["peak_rss_mb"] = found[name]["peak_rss_mb"]
        if found[name]["seconds"] > 0:
            entry["seconds"] = round(found[name]["seconds"], 1)
        if old != entry["peak_rss_mb"]:
            changed.append((name, old, entry["peak_rss_mb"]))
    return changed


def main():
    parser = argparse.ArgumentParser(description=__doc__.split("\n\n")[0])
    parser.add_argument("tier", type=Path, help="the tier file (designer.json), rewritten in place")
    parser.add_argument("logs", type=Path, nargs="+", help="run.py logs of the tier")
    parser.add_argument("--placement", help="only rows of this placement (designer)")
    arguments = parser.parse_args()
    tier = json.loads(arguments.tier.read_text())
    found = measured(arguments.logs, arguments.placement)
    changed = update(tier, found)
    harness_common.atomic_json(arguments.tier, tier, indent=1, newline=True)
    missing = sorted(set(tier["boards"]) - set(found))
    print(f"{len(found)} boards with rows, {len(changed)} estimates changed, {len(missing)} without a row")
    for name, old, new in sorted(changed, key=lambda c: -abs((c[1] or 0) - c[2]))[:10]:
        print(f"  {name}: {old} -> {new} MB")


if __name__ == "__main__":
    main()
