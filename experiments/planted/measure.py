#!/usr/bin/env python3
# Copyright (C) 2026 Toit contributors.
"""Routes generated boards with our router and compares the result with
the planted solution.

    experiments/planted/measure.py <work-dir> <board-dir>... [--binary build/bin/pcb-maker-x510]
        [--jobs 4] [--timeout 1800] [--config auto] [--cores 24-31]

Each board directory holds <id>.kicad_pcb (with the planted copper),
<id>.kicad_pro and planted.json. The board is copied, its tracks and vias
stripped (`strip-kicad-tracks`), and routed with `route-kicad-board ...
auto` (the ladder) pinned with taskset and nice. One JSON row per board
goes to <work-dir>/results.jsonl: the router's nets routed, vias and
length, KiCad's unconnected count and copper errors, CPU seconds from
rusage, and per net (complete in both) the length and via ratios ours /
planted."""

import argparse
import json
import os
import re
import shutil
import subprocess
import sys
import threading
import time
from concurrent.futures import ThreadPoolExecutor
from pathlib import Path

HERE = Path(__file__).resolve().parent
ROOT = HERE.parents[1]
sys.path.insert(0, str(HERE))
COSMETIC = re.compile(r"^(silk_|text_|lib_footprint|footprint_type_mismatch|missing_courtyard|isolated_copper|nonmirrored_text)")


def run_measured(command, timeout, env):
    process = subprocess.Popen(command, stdout=subprocess.PIPE, stderr=subprocess.STDOUT, env=env)
    chunks = []
    reader = threading.Thread(target=lambda: chunks.append(process.stdout.read()), daemon=True)
    reader.start()
    deadline = time.monotonic() + timeout
    while True:
        pid, status, usage = os.wait4(process.pid, os.WNOHANG)
        if pid:
            code = os.waitstatus_to_exitcode(status)
            break
        if time.monotonic() > deadline:
            process.kill()
            _, status, usage = os.wait4(process.pid, 0)
            code = "timeout"
            break
        time.sleep(0.5)
    process.returncode = code if isinstance(code, int) else -9
    reader.join(60)
    output = (chunks[0] if chunks else b"").decode(errors="replace")
    return code, output, round(usage.ru_utime + usage.ru_stime, 1), round(usage.ru_maxrss / 1024)


def drc_summary(path):
    if not path.exists():
        return None
    drc = json.loads(path.read_text())
    errors = {}
    for v in drc.get("violations", []):
        if v.get("severity") == "error" and not COSMETIC.match(v["type"]):
            errors[v["type"]] = errors.get(v["type"], 0) + 1
    return {"errors": errors, "unconnected": len(drc.get("unconnected_items", []))}


def planted_nets(board):
    """Per net of the planted board: pads, MST, length, vias."""
    from stats import board_stats
    stats = board_stats(board)
    return {row["net"]: row for row in stats["per_net"]}, stats


def measure(board_dir, arguments):
    board_dir = Path(board_dir)
    record = json.loads((board_dir / "planted.json").read_text())
    board_id = record["knobs"].get("board_id", "planted")
    name = board_dir.name
    work = arguments.work / name
    if work.exists():
        shutil.rmtree(work)
    source = work / "source"
    source.mkdir(parents=True)
    for suffix in (".kicad_pcb", ".kicad_pro"):
        shutil.copy(board_dir / f"{board_id}{suffix}", source / f"{board_id}{suffix}")
    subprocess.run([str(arguments.binary), "strip-kicad-tracks", str(source / f"{board_id}.kicad_pcb"),
                    str(source / f"{board_id}.kicad_pcb")], capture_output=True, check=True)
    env = dict(os.environ, RAYON_NUM_THREADS=str(arguments.threads))
    command = ["taskset", "-c", arguments.cores, "nice", "-n", "10", str(arguments.binary), "route-kicad-board",
               str(source), board_id, str(work / "routed"), arguments.config]
    code, output, cpu, peak = run_measured(command, arguments.timeout, env)
    (work / "route.log").write_text(output)
    row = {"name": name, "knobs": {k: record["knobs"].get(k) for k in arguments.knob_keys},
           "planted": {"length_mm": record["planted"]["length_mm"], "vias": record["planted"]["vias"],
                       "nets": record["netlist"]["nets"], "connections": record["netlist"]["connections"]},
           "cpu_seconds": cpu, "peak_mb": peak, "exit": code}
    report = work / "routed" / "board-router.json"
    if not report.exists():
        row["error"] = output.strip().splitlines()[-1][:300] if output.strip() else "no output"
        return row
    result = json.loads(report.read_text())
    native = drc_summary(work / "routed" / "drc.json")
    # Findings the planted board has itself (a derived board's source) are
    # not the router's.
    allowed = (record.get("drc") or {}).get("errors") or {}
    if native and allowed:
        errors = dict(native["errors"])
        for kind, count in allowed.items():
            if kind in errors:
                errors[kind] -= count
                if errors[kind] <= 0:
                    del errors[kind]
        native = dict(native, errors=errors, allowed=allowed)
    row["ours"] = {"nets_routed": result["routed_connections"], "nets": result["routable_connections"],
                   "vias": result["vias"], "length_mm": round(result["length_mm"], 1),
                   "pitch": result.get("grid_pitch_mm"), "routing_seconds": round(result["routing_seconds"], 1),
                   "native": native}
    row["complete"] = bool(native and native["unconnected"] == 0 and not native["errors"]
                           and result["routed_connections"] == result["routable_connections"])
    # Per net: ours against planted, nets complete in both.
    planted, _ = planted_nets(board_dir / f"{board_id}.kicad_pcb")
    ratios_length = []
    ratios_detour = []
    vias_ours = vias_planted = 0
    better = worse = 0
    for net in result.get("nets", []):
        p = planted.get(net["connection"])
        if p is None or p.get("pour") or net["status"] != "routed" or p["length_mm"] <= 0:
            continue
        ratios_length.append(net["length_mm"] / p["length_mm"])
        if p["mst_mm"] > 0.3:
            ratios_detour.append((net["length_mm"] / p["mst_mm"], p["length_mm"] / p["mst_mm"]))
        vias_ours += net["vias"]
        vias_planted += p["vias"]
        if net["length_mm"] < p["length_mm"] * 0.98:
            better += 1
        elif net["length_mm"] > p["length_mm"] * 1.02:
            worse += 1
    if ratios_length:
        ratios_length.sort()
        row["per_net"] = {
            "compared": len(ratios_length),
            "length_ratio_median": round(ratios_length[len(ratios_length) // 2], 3),
            "length_ratio_mean": round(sum(ratios_length) / len(ratios_length), 3),
            "shorter": better, "longer": worse,
            "vias_ours": vias_ours, "vias_planted": vias_planted,
            "detour_ours_mean": round(sum(a for a, _ in ratios_detour) / max(1, len(ratios_detour)), 3),
            "detour_planted_mean": round(sum(b for _, b in ratios_detour) / max(1, len(ratios_detour)), 3),
        }
    return row


def main():
    parser = argparse.ArgumentParser()
    parser.add_argument("work", type=Path)
    parser.add_argument("boards", nargs="*")
    parser.add_argument("--list", help="a file with one board directory per line")
    parser.add_argument("--binary", type=Path, default=ROOT / "build/bin/pcb-maker-x510")
    parser.add_argument("--jobs", type=int, default=4)
    parser.add_argument("--threads", type=int, default=2)
    parser.add_argument("--timeout", type=int, default=1800)
    parser.add_argument("--config", default="auto")
    parser.add_argument("--cores", default="24-31")
    parser.add_argument("--knobs", default="mode,level,layers,parts,area_per_part,wind,detour_max,corridor,blocker_density,one_layer,seed,density,source")
    arguments = parser.parse_args()
    arguments.jobs = min(arguments.jobs, 4)
    if arguments.list:
        arguments.boards += [line.strip() for line in open(arguments.list) if line.strip()]
    arguments.knob_keys = arguments.knobs.split(",")
    arguments.work.mkdir(parents=True, exist_ok=True)
    results = arguments.work / "results.jsonl"
    lock = threading.Lock()

    def one(board):
        try:
            row = measure(board, arguments)
        except Exception as error:  # noqa: BLE001 - a failed board is a row
            row = {"name": Path(board).name, "error": repr(error)}
        with lock:
            with results.open("a") as out:
                out.write(json.dumps(row) + "\n")
            print(json.dumps(row), flush=True)

    with ThreadPoolExecutor(arguments.jobs) as pool:
        list(pool.map(one, arguments.boards))


if __name__ == "__main__":
    main()
