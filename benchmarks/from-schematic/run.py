#!/usr/bin/env python3
# Copyright (C) 2026 Toit contributors.
"""Schematic to board: every KiCad demo whose footprints are all found.

    benchmarks/from-schematic/run.py <output> [--only NAME...] [--jobs N] [--binary PATH]

Each board goes the way an agent's would: `import-kicad-netlist` from the
demo's schematic, `layout-kicad-board` with the starter `layout.json` it
writes (every part placed, outline sized from the parts, plug-in connectors
on an edge), unchanged. A board passes when every connection is routed and
KiCad reports it complete (no unconnected item, no DRC error, schematic
parity). The designer's own board is scored alongside for size, vias and
copper."""

import argparse
import json
import re
import shutil
import subprocess
import time
from concurrent.futures import ThreadPoolExecutor
from pathlib import Path

HERE = Path(__file__).resolve().parent
ROOT = HERE.parents[1]
DEMOS = ROOT / "benchmarks/real/external/_downloads/kicad-src/demos"
# Demos whose footprints the libraries have (the simulation demos have
# none; jetson, One-Air-Max, RoyalBlue54L and Q17ng miss some; vme-wren's
# 1507 parts are out of reach).
BOARDS = ["ecc83/ecc83-pp", "ecc83/ecc83-pp_v2", "sonde xilinx/sonde xilinx", "interf_u/interf_u",
          "pic_programmer/pic_programmer", "complex_hierarchy/complex_hierarchy", "cm5_minima/CM5_MINIMA_3",
          "stickhub/StickHub", "multichannel/multichannel_mixer", "kit-dev-coldfire-xilinx_5213/kit-dev-coldfire-xilinx_5213",
          "video/video"]


def board(arguments, entry):
    folder, name = entry.split("/")
    matches = [path for path in DEMOS.rglob(f"{name}.kicad_sch") if path.parent.name == folder or folder in str(path.parent)]
    row = {"name": name}
    if not matches:
        row["error"] = "schematic not found"
        return row
    schematic = matches[0]
    work = arguments.output / name
    shutil.rmtree(work, ignore_errors=True)
    project = work / "project"
    started = time.monotonic()
    imported = subprocess.run([str(arguments.binary), "import-kicad-netlist", str(schematic), str(project), name],
                              capture_output=True, text=True)
    if imported.returncode:
        row["error"] = imported.stderr.strip()[-300:]
        return row
    report = json.loads(imported.stdout)
    row["parts"] = report["footprints"]
    row["missing"] = len(report["missing"])
    try:
        process = subprocess.run([str(arguments.binary), "layout-kicad-board", str(project), name, str(work / "layout"), "auto",
                                  str(project / "layout.json")],
                                 stdout=subprocess.PIPE, stderr=subprocess.STDOUT, text=True, timeout=arguments.timeout)
        (work / "layout.log").write_text(process.stdout)
    except subprocess.TimeoutExpired:
        row["error"] = f"timed out after {arguments.timeout} s"
        return row
    row["seconds"] = round(time.monotonic() - started)
    result_path = work / "layout/board-layout.json"
    if not result_path.exists():
        lines = process.stdout.strip().splitlines()
        row["error"] = lines[-1][:300] if lines else "no output"
        return row
    result = json.loads(result_path.read_text())
    routed = result["routed"]
    native = routed["native"]
    row.update({
        "routed": f"{routed['routed_connections']}/{routed['routable_connections']}",
        "complete": native["complete"],
        "drc": native["drc_design_violations"],
        "parity": native["schematic_parity_issues"],
    })
    quality = result.get("quality") or {}
    economy = quality.get("economy") or {}
    row["ours"] = {"size_mm": [economy.get("width_mm"), economy.get("height_mm")], "vias": economy.get("vias"),
                   "copper_mm": economy.get("track_length_mm")}
    designer = subprocess.run([str(arguments.binary), "score-kicad-board", str(schematic.parent), name], capture_output=True, text=True)
    if designer.returncode == 0:
        theirs = json.loads(designer.stdout)["economy"]
        row["designer"] = {"size_mm": [theirs.get("width_mm"), theirs.get("height_mm")], "vias": theirs.get("vias"),
                           "copper_mm": theirs.get("track_length_mm"), "layers": theirs.get("copper_layers")}
    row["pass"] = routed["routed_connections"] == routed["routable_connections"] and native["complete"]
    return row


def main():
    parser = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    parser.add_argument("output", type=Path)
    parser.add_argument("--only", nargs="*")
    parser.add_argument("--jobs", type=int, default=1)
    parser.add_argument("--timeout", type=int, default=3600)
    parser.add_argument("--binary", type=Path, default=ROOT / "target/release/pcb-maker")
    arguments = parser.parse_args()
    arguments.output = arguments.output.resolve()
    arguments.output.mkdir(parents=True, exist_ok=True)
    boards = [entry for entry in BOARDS if not arguments.only or entry.split("/")[1] in arguments.only]
    with ThreadPoolExecutor(arguments.jobs) as pool:
        rows = []
        for row in pool.map(lambda entry: board(arguments, entry), boards):
            print(json.dumps(row), flush=True)
            rows.append(row)
    (arguments.output / "results.json").write_text(json.dumps(rows, indent=1))
    passed = sum(1 for row in rows if row.get("pass"))
    print(f"{passed}/{len(rows)} complete")


if __name__ == "__main__":
    main()
