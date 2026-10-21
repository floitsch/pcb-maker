#!/usr/bin/env python3
# Copyright (C) 2026 Toit contributors.
"""Writes the route-mode task list of the KiCad connectivity measurement
(docs/changelog.md, "KiCad connectivity agreement"): the intake's boards
where the router counted every connection routed and KiCad's DRC (zones
refilled) reported unconnected items, with a licence run.py fetches
(boards without one are listed by census.py only).

    python3 -I experiments/kicad-connectivity/tasks.py [--data DIR] > benchmarks/agent-tasks/connectivity-tasks.json
    benchmarks/agent-tasks/run.py build/conn-x1322 --no-lock \\
      --tasks benchmarks/agent-tasks/connectivity-tasks.json --jobs 8 --threads 1 \\
      --binary build/bin/pcb-maker-x1322 > build/conn-x1322.log
"""
import argparse
import json
import sys
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parent))
from census import DATA, disagreeing  # noqa: E402

NO_LICENCE = (None, "", "NOASSERTION", "NONE")


def task(record):
    board_id = record["board_id"]
    files = {f"{board_id}.kicad_pcb": record["sha256"]}
    if record.get("project_sha256"):
        files[f"{board_id}.kicad_pro"] = record["project_sha256"]
    if record.get("custom_rules_sha256"):
        files[f"{board_id}.kicad_dru"] = record["custom_rules_sha256"]
    name = record["name"].split(".kicad_pcb")[0][:80]
    return {
        "name": name,
        "mode": "route",
        "directory": f"build/connectivity-tasks/{name}/source",
        "board_id": board_id,
        "designer_budget": True,
        "budget_seconds": 30,
        "router": {"seeds": 1, "skip_native_verification": True},
        "source": {
            "repository": record["repository"],
            "commit": record["commit"],
            "path": record["path"],
            "url": record["source_url"],
            "license": record["license_spdx"],
            "harvest_name": record["name"],
            "files": files,
        },
    }


def main():
    parser = argparse.ArgumentParser()
    parser.add_argument("--data", type=Path, default=DATA, help="the harvest's data directory")
    data = parser.parse_args().data
    records = {record["name"]: record for record in json.loads((data / "manifest.json").read_text())["boards"]}
    names = sorted(row["name"] for row in disagreeing(data))
    tasks = [task(records[name]) for name in names if records[name].get("license_spdx") not in NO_LICENCE]
    print(json.dumps({
        "comment": "Route-mode tasks of the KiCad connectivity measurement (experiments/kicad-connectivity/tasks.py): "
                   "the intake's boards complete by the router's count with KiCad unconnected items, "
                   "the intake's settings, the designer's placement, 30 s of work.",
        "tasks": tasks,
    }, indent=1))


if __name__ == "__main__":
    main()
