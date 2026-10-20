#!/usr/bin/env python3
# Copyright (C) 2026 Toit contributors.
"""Writes the expansion tier (expansion.json) and its task list
(expansion-tasks.json) from the intake's tier proposal.

    python3 -I benchmarks/agent-tasks/from_expansion.py [--manifest PATH]

Reads benchmarks/expansion/analysis/expansion-tasks.json (the 46 boards
and their sources), expansion_tier.csv (bucket and the intake's quick and
deep outcomes per board) and the harvest's manifest
(benchmarks/expansion/data/manifest.json: the repository path and the
SHA-256 of the board and of its .kicad_pro and .kicad_dru as harvested)
and the quick run's results (data/runs/quick/results.jsonl: the stripped
board's SHA-256, vias and length, so that a run can check that it routed
the intake's input). With `--run-log` and `--run-label`, a run of the tier
(run.py's log) gives each board its `seconds` and `peak_rss_mb` and its
result under the label:

    python3 -I benchmarks/agent-tasks/from_expansion.py \
      --run-log build/expansion-x1211.log --run-label x1211
Each task is a route-mode task (`"mode": "route"`, README.md, "The
expansion tier"): run.py fetches the files at the pinned commit into
`build/github-expansion/<task>/source`, verifies them against these
digests, strips the board's tracks and routes the designer's placement
with `budget_seconds` of work and the intake's router settings."""

import argparse
import csv
import json
from pathlib import Path

HERE = Path(__file__).resolve().parent
ROOT = HERE.parents[1]
ANALYSIS = ROOT / "benchmarks/expansion/analysis"
# The intake's quick-route settings (benchmarks/expansion/triage.py, the
# quick run's run.json): one seed and no KiCad check inside the router (the
# runner checks the result with drc-kicad-board). run.py adds the ladder's
# budget (budget_seconds times the budget scale) and the intake's wall guard.
INTAKE_ROUTER = {"seeds": 1, "skip_native_verification": True}
BUDGET_SECONDS = 30


def number(text):
    if text in ("", None):
        return None
    value = float(text)
    return int(value) if value.is_integer() else value


def task_entry(proposal, record):
    """The tier's task for a proposal entry and its harvest record."""
    board_id = proposal["board_id"]
    files = {f"{board_id}.kicad_pcb": record["sha256"]}
    if record.get("project_sha256"):
        files[f"{board_id}.kicad_pro"] = record["project_sha256"]
    if record.get("custom_rules_sha256"):
        files[f"{board_id}.kicad_dru"] = record["custom_rules_sha256"]
    source = proposal["source"]
    return {
        "name": proposal["name"],
        "mode": "route",
        "directory": f"build/github-expansion/{proposal['name']}/source",
        "board_id": board_id,
        "designer_budget": proposal.get("designer_budget", True),
        "budget_seconds": BUDGET_SECONDS,
        "router": dict(INTAKE_ROUTER, **(proposal.get("router") or {})),
        "source": {
            "repository": source["repository"],
            "commit": source["commit"],
            "path": record["path"],
            "url": source["url"],
            "license": source["license"],
            "harvest_name": Path(proposal["directory"]).name,
            "files": files,
        },
    }


def quick_rows(path, names):
    """The intake's quick-run row of each harvest name in `names` whose
    class is the CSV's (`names`: harvest name -> class), else its last."""
    found = {}
    if not path.exists():
        return found
    for line in path.read_text().splitlines():
        try:
            row = json.loads(line)
        except json.JSONDecodeError:
            continue
        name = row.get("name")
        if name in names and (name not in found or row.get("category") == names[name]
                              or found[name].get("category") != names[name]):
            found[name] = row
    return found


def tier_entry(row, quick=None):
    """The tier's entry for a board: what it is and the intake's outcomes.
    `peak_rss_mb` (the memory gate's expectation) and `seconds` (the order)
    start as the intake's quick run's and are replaced by the tier's own
    run's (README.md)."""
    entry = {
        "bucket": row["bucket"],
        "layers": number(row["layers"]),
        "pads": number(row["pads"]),
        "nets": number(row["nets"]),
        "connections": number(row["connections"]),
        "pitch_mm": number(row["pitch_mm"]),
        "license": row["license"],
        "repository": row["repository"],
        "intake": {
            "quick_outcome": row["quick_outcome"],
            "open_at_30s": number(row["open_at_30s"]),
            "quick_routing_seconds": number(row["quick_routing_seconds"]),
            "quick_work_seconds": number(row["quick_work_seconds"]),
            "deep_outcome": row["deep_outcome"],
            "open_at_300s": number(row["open_at_300s"]),
            "deep_work_seconds": number(row["deep_work_seconds"]),
        },
        "peak_rss_mb": round(number(row["quick_peak_rss_mb"]) or 0) or None,
        "seconds": number(row["quick_routing_seconds"]),
    }
    route = (quick or {}).get("route") or {}
    if quick and quick.get("stripped_sha256"):
        entry["intake"]["stripped_sha256"] = quick["stripped_sha256"]
    if route.get("routable_connections") is not None:
        entry["intake"].update(quick_routed=f"{route['routed_connections']}/{route['routable_connections']}",
                               quick_vias=route.get("vias"), quick_length_mm=round(route.get("length_mm", 0), 1))
    return entry


def add_run(entry, row, label):
    """A tier run's row: the board's seconds and peak (the runner's order
    and memory gate) and its result under `label`."""
    if row.get("seconds") is not None:
        entry["seconds"] = row["seconds"]
    if row.get("peak_rss_mb"):
        entry["peak_rss_mb"] = row["peak_rss_mb"]
    native = row.get("native") or {}
    result = {"outcome": row.get("outcome"), "routed": row.get("routed"), "open": row.get("open"),
              "unconnected": native.get("unconnected"), "errors": native.get("errors"),
              "work_seconds": row.get("work_seconds"), "pass": row.get("pass")}
    if row.get("cut"):
        result["cut"] = row["cut"]
    if row.get("error"):
        result["error"] = row["error"]
    entry[label] = result


def main():
    parser = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    parser.add_argument("--manifest", type=Path, default=ROOT / "benchmarks/expansion/data/manifest.json")
    parser.add_argument("--quick-results", type=Path,
                        default=ROOT / "benchmarks/expansion/data/runs/quick/results.jsonl")
    parser.add_argument("--run-log", type=Path, help="a run.py log of the tier: its seconds, peaks and results")
    parser.add_argument("--run-label", help="the key of the run's results in each board's entry (e.g. x1211)")
    parser.add_argument("--tasks-out", type=Path, default=HERE / "expansion-tasks.json")
    parser.add_argument("--tier-out", type=Path, default=HERE / "expansion.json")
    arguments = parser.parse_args()
    if bool(arguments.run_log) != bool(arguments.run_label):
        parser.error("--run-log and --run-label go together")
    run_rows = {}
    if arguments.run_log:
        for line in arguments.run_log.read_text().splitlines():
            if line.startswith("{"):
                row = json.loads(line)
                run_rows[row["name"]] = row
    proposals = json.loads((ANALYSIS / "expansion-tasks.json").read_text())["tasks"]
    rows = {row["task"]: row for row in csv.DictReader((ANALYSIS / "expansion_tier.csv").open())}
    records = {record["name"]: record for record in json.loads(arguments.manifest.read_text())["boards"]}
    quick = quick_rows(arguments.quick_results, {Path(p["directory"]).name: rows[p["name"]]["quick_outcome"]
                                                 for p in proposals})
    tasks, boards = [], {}
    for proposal in proposals:
        record = records[Path(proposal["directory"]).name]
        if not proposal["source"].get("license"):
            raise SystemExit(f"{proposal['name']}: no licence in the proposal")
        tasks.append(task_entry(proposal, record))
        boards[proposal["name"]] = tier_entry(rows[proposal["name"]], quick.get(Path(proposal["directory"]).name))
        if proposal["name"] in run_rows:
            add_run(boards[proposal["name"]], run_rows[proposal["name"]], arguments.run_label)
    arguments.tasks_out.write_text(json.dumps({
        "comment": "Route-mode tasks of the expansion tier (README.md, 'The expansion tier'; from_expansion.py): "
                   "the designer's placement, tracks stripped, routed with budget_seconds of work and the "
                   "intake's router settings. run.py fetches source.files at source.commit into directory "
                   "(from the harvest's copy or GitHub) and verifies their SHA-256; a task without a "
                   "licence is not fetched.",
        "tasks": tasks}, indent=1) + "\n")
    arguments.tier_out.write_text(json.dumps({
        "description": "Expansion tier: 46 boards of the 2026-10-18 intake (benchmarks/expansion), routed on the "
                       "designer's placement with tracks stripped (route mode, 30 s of work: run.py --tier "
                       "expansion; --budget-scale 10 for the deep run's 300 s). Selection and buckets in "
                       "README.md, 'The expansion tier', and benchmarks/expansion/analysis/expansion-tier.json. "
                       "Per board: bucket, size, licence, the intake's quick (30 s) and deep (300 s) outcomes "
                       "(binary e91e8441, main be4b012, one search thread), and peak_rss_mb (the memory gate's "
                       "expectation; route process or drc-kicad-board, whichever is larger) and seconds (the "
                       "runner starts the longest first) from the tier's first run, x1211 (2026-10-21, 8 jobs x "
                       "1 thread, load 13-23), whose result is under x1211.",
        "tasks": str(arguments.tasks_out.resolve().relative_to(ROOT)),
        "budget_scale": 1,
        "boards": boards}, indent=1) + "\n")
    print(f"{len(tasks)} tasks: {arguments.tasks_out}, {arguments.tier_out}")


if __name__ == "__main__":
    main()
