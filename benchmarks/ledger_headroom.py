#!/usr/bin/env python3
# Copyright (C) 2026 Toit contributors.
"""The memory ledger's headroom samples and the floor they give
(harness_common.measured_floor_mb; docs/benchmarks.md, "Memory ledger",
the floor from measured headroom).

    # what the floor is now, per kind of run (estimates.json's overshoot):
    python3 benchmarks/ledger_headroom.py
    # sample every 10 s (for an hour; default: until stopped), as run.py's
    # MemoryGate does while it runs; light (one read of /proc per sample):
    python3 benchmarks/ledger_headroom.py --sample [--every 10] [--for 3600]

The samples live beside the ledger (`<ledger>.headroom.json`, the last six
hours); every writer adds one at most every 10 s.
"""

import argparse
import json
import sys
import time
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parent))
import harness_common  # noqa: E402

ESTIMATES = Path(__file__).resolve().parent / "agent-tasks" / "estimates.json"


def main():
    parser = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    parser.add_argument("--sample", action="store_true", help="record samples until stopped (or --for)")
    parser.add_argument("--every", type=float, default=harness_common.SAMPLE_EVERY_SECONDS)
    parser.add_argument("--for", dest="seconds", type=float, default=None, help="seconds to sample")
    parser.add_argument("--estimates", type=Path, default=ESTIMATES)
    parser.add_argument("--json", action="store_true")
    arguments = parser.parse_args()
    ledger = harness_common.Ledger.from_env()
    path = harness_common.headroom_path(ledger.path)
    if path is None:
        print("memory ledger: off (PCB_MAKER_LEDGER=off)")
        return 0
    if arguments.sample:
        until = None if arguments.seconds is None else time.monotonic() + arguments.seconds
        while until is None or time.monotonic() < until:
            harness_common.record_headroom(ledger, every=arguments.every)
            time.sleep(arguments.every)
        return 0
    summary = (harness_common.read_headroom(path) or {}).get("summary")
    try:
        overshoot = json.loads(arguments.estimates.read_text()).get("overshoot") or {}
    except (OSError, ValueError):
        overshoot = {}
    floors = {kind: harness_common.measured_floor_mb(summary, entry.get("p95_mb", 0.0))
              for kind, entry in sorted(overshoot.items())}
    floors["(no overshoot)"] = harness_common.measured_floor_mb(summary, 0.0)
    if arguments.json:
        print(json.dumps({"path": str(path), "summary": summary,
                          "floors_mb": {kind: floor for kind, (floor, _) in floors.items()}}, indent=2))
        return 0
    print(f"headroom samples {path}: " + (json.dumps(summary) if summary else "none"))
    for kind, (floor, text) in floors.items():
        print(f"  floor for {kind}: {floor / 1024:.1f} GB ({text})")
    return 0


if __name__ == "__main__":
    sys.exit(main())
