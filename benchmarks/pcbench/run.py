#!/usr/bin/env python3
# Copyright (C) 2026 Toit contributors.
"""Routes (and optionally places and routes) PCBench/D3 boards with pcb-maker.

    benchmarks/pcbench/run.py <output> [--subset d3-test] [--jobs 4] [--mode route]

Each board is the D3 guide board with its tracks and vias removed. A board
passes clean (PCBWorld's "CP") when every connection is routed and native
KiCad DRC, with the board's own (D3-patched) rules, reports no error and no
unconnected item. Results go to <output>/results.jsonl (one row per board,
appended as boards finish, so an interrupted run can be resumed) and
<output>/summary.md."""

import argparse
import json
import os
import shutil
import subprocess
import sys
import time
from concurrent.futures import ThreadPoolExecutor
from pathlib import Path

ROOT = Path(__file__).resolve().parents[2]
WORK = ROOT / "benchmarks/real/external/pcbench-work/newdrc"
MANIFEST = Path(__file__).with_name("manifest.json")
BOARD = "board"


def subset(manifest, name):
    boards = [b for b in manifest["boards"] if "error" not in b]
    d3 = lambda b, tier=None, test=None: b.get("d3") and (tier is None or b["d3"]["tier"] == tier) \
        and (test is None or b["d3"]["test"] == test)
    if name == "all":
        return boards
    if name == "d3":
        return [b for b in boards if d3(b)]
    if name == "d3-test":
        return [b for b in boards if d3(b, test=True)]
    if name in ("d3a", "d3b", "d3c"):
        return [b for b in boards if d3(b, tier=name)]
    if name in ("d3a-test", "d3b-test", "d3c-test"):
        return [b for b in boards if d3(b, tier=name[:3], test=True)]
    if name == "smoke":
        # Four boards per tier, spread over the size order.
        picked = []
        for tier in ("d3a", "d3b", "d3c"):
            members = sorted((b for b in boards if d3(b, tier=tier)), key=lambda b: b["d3"]["d3_id"])
            picked += [members[i * (len(members) - 1) // 3] for i in range(4)] if members else []
        return picked
    raise SystemExit(f"unknown subset {name}")


def drc_counts(report_path):
    if not report_path.exists():
        return None
    report = json.loads(report_path.read_text())
    errors = {}
    for violation in report.get("violations", []):
        if violation.get("severity") == "error":
            errors[violation["type"]] = errors.get(violation["type"], 0) + 1
    return {"errors": errors, "unconnected": len(report.get("unconnected_items", []))}


def run_board(board, arguments):
    name = board["name"]
    work = arguments.output / "boards" / name
    if work.exists():
        shutil.rmtree(work)
    source = work / "source"
    source.mkdir(parents=True)
    stem = "processed_v9_guide_v3"
    shutil.copy(WORK / name / f"{stem}_unrouted.kicad_pcb", source / f"{BOARD}.kicad_pcb")
    shutil.copy(WORK / name / f"{stem}.kicad_pro", source / f"{BOARD}.kicad_pro")
    environment = dict(os.environ, RAYON_NUM_THREADS=str(arguments.threads))
    row = {"name": name, "tier": (board.get("d3") or {}).get("tier"),
           "connections_expected": board.get("connections")}
    modes = ["route", "layout"] if arguments.mode == "both" else [arguments.mode]
    for mode in modes:
        command = ("route-kicad-board" if mode == "route" else "layout-kicad-board")
        output = work / mode
        started = time.monotonic()
        try:
            process = subprocess.run([str(arguments.binary), command, str(source), BOARD, str(output), "auto"],
                                     stdout=subprocess.PIPE, stderr=subprocess.STDOUT, text=True,
                                     timeout=arguments.timeout, env=environment)
            log, code = process.stdout, process.returncode
        except subprocess.TimeoutExpired as error:
            log = error.stdout.decode(errors="replace") if isinstance(error.stdout, bytes) else (error.stdout or "")
            code = "timeout"
        seconds = round(time.monotonic() - started, 1)
        (work / f"{mode}.log").write_text("\n".join(l for l in log.splitlines() if "PROPERTY_ENUM" not in l))
        report = output / ("board-router.json" if mode == "route" else "board-layout.json")
        entry = {"seconds": seconds, "exit": code}
        if report.exists():
            result = json.loads(report.read_text())
            if mode == "layout":
                result = result["routed"]
            native = drc_counts(output / ("drc.json" if mode == "route" else "result/drc.json"))
            entry.update(routed=result["routed_connections"], connections=result["routable_connections"],
                         vias=result["vias"], length_mm=round(result["length_mm"], 1),
                         internal=len(result["internal_violations"]), native=native)
            entry["clean"] = bool(native is not None and result["routed_connections"] == result["routable_connections"]
                                  and not native["errors"] and native["unconnected"] == 0)
        else:
            entry["clean"] = False
            entry["error"] = log.strip().splitlines()[-1][:300] if log.strip() else "no output"
        row[mode] = entry
    return row


def summarize(rows, manifest, arguments):
    by_name = {b["name"]: b for b in manifest["boards"]}
    lines = [f"# PCBench / D3 results ({arguments.mode}, {len(rows)} boards)", ""]
    mode = "route" if arguments.mode in ("route", "both") else "layout"
    tiers = sorted({r["tier"] or "-" for r in rows})
    lines += ["| Tier | Boards | pcb-maker clean | PCBench Freerouting success | pcb-maker vias / designer | copper / designer | median s |",
              "| --- | ---: | ---: | ---: | ---: | ---: | ---: |"]
    for tier in tiers + ["all"]:
        members = [r for r in rows if tier == "all" or (r["tier"] or "-") == tier]
        if not members:
            continue
        clean = sum(1 for r in members if r.get(mode, {}).get("clean"))
        fr = [by_name[r["name"]].get("pcbench") for r in members]
        fr_success = sum(1 for f in fr if f and f["freerouting"]["success"])
        both = [r for r in members if r.get(mode, {}).get("clean")]
        via_ratio = sum(r[mode]["vias"] for r in both) / max(1, sum(by_name[r["name"]]["designer"]["vias"] for r in both))
        length_ratio = sum(r[mode]["length_mm"] for r in both) / max(1.0, sum(by_name[r["name"]]["designer"]["length_mm"] for r in both))
        times = sorted(r.get(mode, {}).get("seconds", 0) for r in members)
        lines.append(f"| {tier} | {len(members)} | {clean} ({100 * clean / len(members):.0f} %) | "
                     f"{fr_success} ({100 * fr_success / len(members):.0f} %) | {via_ratio:.2f} | {length_ratio:.2f} | "
                     f"{times[len(times) // 2]} |")
    lines += ["", "Not clean:", ""]
    for r in sorted(rows, key=lambda r: r["name"]):
        entry = r.get(mode, {})
        if not entry.get("clean"):
            native = entry.get("native") or {}
            lines.append(f"- {r['name']} ({r['tier']}): {entry.get('routed')}/{entry.get('connections')}, "
                         f"unconnected {native.get('unconnected')}, errors {native.get('errors')}, "
                         f"{entry.get('error', '')} ({entry.get('seconds')} s)")
    (arguments.output / "summary.md").write_text("\n".join(lines) + "\n")
    print("\n".join(lines[:4 + len(tiers) + 1]))


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("output", type=Path)
    parser.add_argument("--subset", default="d3-test")
    parser.add_argument("--only", nargs="*")
    parser.add_argument("--mode", choices=["route", "layout", "both"], default="route")
    parser.add_argument("--jobs", type=int, default=4)
    parser.add_argument("--threads", type=int, default=2, help="Rayon threads per board")
    parser.add_argument("--timeout", type=int, default=900)
    parser.add_argument("--binary", type=Path, default=ROOT / "target/release/pcb-maker")
    arguments = parser.parse_args()
    arguments.output.mkdir(parents=True, exist_ok=True)
    manifest = json.loads(MANIFEST.read_text())
    boards = subset(manifest, arguments.subset)
    if arguments.only:
        boards = [b for b in manifest["boards"] if b["name"] in arguments.only]
    results_path = arguments.output / "results.jsonl"
    done = {}
    if results_path.exists():
        for line in results_path.read_text().splitlines():
            row = json.loads(line)
            done[row["name"]] = row
    pending = [b for b in boards if b["name"] not in done]
    print(f"{len(boards)} boards, {len(pending)} to run", file=sys.stderr)
    with ThreadPoolExecutor(arguments.jobs) as pool, open(results_path, "a") as results:
        for row in pool.map(lambda board: run_board(board, arguments), pending):
            results.write(json.dumps(row) + "\n")
            results.flush()
            done[row["name"]] = row
            entry = row.get(arguments.mode if arguments.mode != "both" else "route", {})
            print(f"{row['name']}: clean={entry.get('clean')} {entry.get('routed')}/{entry.get('connections')} "
                  f"vias {entry.get('vias')} {entry.get('seconds')} s", file=sys.stderr, flush=True)
    names = {b["name"] for b in boards}
    summarize([row for name, row in done.items() if name in names], manifest, arguments)


if __name__ == "__main__":
    main()
