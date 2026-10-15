#!/usr/bin/env python3
# Copyright (C) 2026 Toit contributors.
"""Builds benchmarks/pcbench/manifest.json from the prepared PCBench boards.

For every board that passed PCBWorld's DRC filter (benchmarks/real/external/
pcbench-work/newdrc/<name>/), it records what the board is (layers, parts,
nets, size), what the designer did (vias, copper), what native KiCad says
about the designer's routed guide board, the PCBWorld D3 tier and test-split
membership, and the published PCBench baselines (Freerouting, PcbRouter).
Board content is never copied into the repository; only these numbers are."""

import argparse
import csv
import json
import os
import re
import subprocess
import sys
import tempfile
from concurrent.futures import ProcessPoolExecutor
from pathlib import Path

ROOT = Path(__file__).resolve().parents[2]
EXTERNAL = ROOT / "benchmarks/real/external"
STEM = "processed_v9_guide_v3"

def blocks(text, head):
    """Text of every `(head ...)` block (balanced parentheses, quotes respected)."""
    token = "(" + head
    found = []
    index = 0
    while True:
        start = text.find(token, index)
        if start < 0:
            return found
        if text[start + len(token)] not in ' \n\t"':
            index = start + 1
            continue
        depth, quoted = 0, False
        for position in range(start, len(text)):
            character = text[position]
            if character == '"' and text[position - 1] != "\\":
                quoted = not quoted
            elif not quoted:
                if character == "(":
                    depth += 1
                elif character == ")":
                    depth -= 1
                    if depth == 0:
                        break
        found.append(text[start:position + 1])
        index = position + 1


def statistics(board, binary):
    """What the board is, from its file (pcbnew's Python module is avoided:
    its iterators break on Python 3.14)."""
    text = board.read_text()
    layers = blocks(text, "layers")[0]
    copper = len(re.findall(r'\(\d+ "[^"]*\.Cu" (?:signal|power|mixed|jumper)', layers))
    footprints = blocks(text, "footprint")
    pads_per_net = {}
    smd = tht = back = tht_parts = 0
    for footprint in footprints:
        if re.search(r'^\(footprint "[^"]*"\s+\(layer "B\.Cu"\)', footprint):
            back += 1
        has_tht = False
        for pad in blocks(footprint, "pad"):
            kind = re.match(r'\(pad "[^"]*" (\w+)', pad)
            kind = kind.group(1) if kind else ""
            smd += kind == "smd"
            if kind == "thru_hole":
                tht += 1
                has_tht = True
            net = re.search(r'\(net (?:\d+ )?"((?:[^"\\]|\\.)*)"\)', pad)
            if net and net.group(1) and not net.group(1).startswith("unconnected-"):
                pads_per_net[net.group(1)] = pads_per_net.get(net.group(1), 0) + 1
        tht_parts += has_tht
    points = []
    for head in ("gr_line", "gr_rect", "gr_arc", "gr_circle", "gr_poly"):
        for item in blocks(text, head):
            if '"Edge.Cuts"' in item:
                points += [(float(x), float(y)) for x, y in re.findall(r"\((?:start|end|mid|center|xy) ([-\d.]+) ([-\d.]+)\)", item)]
    nets = {name: count for name, count in pads_per_net.items() if count >= 2}
    inspection = json.loads(subprocess.run([str(binary), "inspect-kicad-board", str(board)], capture_output=True,
                                           text=True, timeout=300).stdout)
    widths = [float(width) for segment in blocks(text, "segment") for width in re.findall(r"\(width ([\d.]+)\)", segment)]
    return {
        "copper_layers": copper,
        "footprints": len(footprints),
        "back_footprints": back,
        "tht_footprints": tht_parts,
        "pads": smd + tht,
        "smd_pads": smd,
        "tht_pads": tht,
        "nets": len(nets),
        "connections": sum(count - 1 for count in nets.values()),
        "largest_net_pads": max(nets.values(), default=0),
        "zones": inspection.get("zones"),
        "width_mm": round(max(p[0] for p in points) - min(p[0] for p in points), 2) if points else None,
        "height_mm": round(max(p[1] for p in points) - min(p[1] for p in points), 2) if points else None,
        "designer": {"tracks": inspection["segments"] + inspection["arcs"], "vias": inspection["vias"],
                     "length_mm": round(inspection["physical_copper"]["physical_centerline_length_mm"], 1),
                     "min_track_width_mm": min(widths) if widths else None},
    }


def characterize(arguments):
    name, directory, binary = arguments
    board = directory / f"{STEM}.kicad_pcb"
    result = {"name": name}
    try:
        result.update(statistics(board, binary))
    except Exception as error:  # noqa: BLE001 - recorded, not fatal
        result["error"] = f"statistics: {error!r}"
        return result
    project = json.loads((directory / f"{STEM}.kicad_pro").read_text())
    classes = project.get("net_settings", {}).get("classes", [])
    rules = project.get("board", {}).get("design_settings", {}).get("rules", {})
    result["rules"] = {
        "classes": len(classes),
        "min_clearance_mm": min((c.get("clearance", 0) for c in classes), default=None),
        "min_class_track_mm": min((c.get("track_width", 0) for c in classes), default=None),
        "min_via_mm": min((c.get("via_diameter", 0) for c in classes), default=None),
        "board_min_track_mm": rules.get("min_track_width"),
    }
    # The designer's routed guide board, judged as the benchmark judges ours.
    with tempfile.TemporaryDirectory() as scratch:
        report = Path(scratch) / "drc.json"
        subprocess.run(["kicad-cli", "pcb", "drc", "--format", "json", "--severity-error", "-o", str(report),
                        str(board)], capture_output=True, timeout=600)
        if report.exists():
            drc = json.loads(report.read_text())
            kinds = {}
            for violation in drc.get("violations", []):
                kinds[violation["type"]] = kinds.get(violation["type"], 0) + 1
            result["designer"]["drc_errors"] = kinds
            result["designer"]["unconnected"] = len(drc.get("unconnected_items", []))
    metadata = directory / "metadata.json"
    if metadata.exists():
        meta = json.loads(metadata.read_text())
        result["source"] = meta.get("source")
        result["license"] = meta.get("licenses") or None
    return result


def baselines(pcbench):
    """PCBench's published results (Baselines/Results_all.csv)."""
    rows = list(csv.reader(open(pcbench / "Baselines/Results_all.csv")))[2:]
    table = {}
    for row in rows:
        def entry(offset):
            if row[offset] != "Yes":
                return {"success": False}
            return {"success": True, "length_mm": round(float(row[offset + 1]), 1),
                    "vias": int(float(row[offset + 2])),
                    "seconds": float(row[offset + 3]) if row[offset + 3] not in ("", "N/A") else None}
        table[row[0]] = {"freerouting": entry(1), "pcbrouter": entry(5)}
    return table


def d3_splits(pcbworld):
    """PCBWorld's D3 tiers (a/b/c = easy/medium/hard) and test splits."""
    d3 = json.loads((pcbworld / "configs/datasets/d3.json").read_text())
    splits = {}
    for difficulty, tier in (("easy", "d3a"), ("medium", "d3b"), ("hard", "d3c")):
        test = set(d3[difficulty]["test"])
        for entry in d3[difficulty]["train"]:
            number, name = entry.split("_", 1)
            splits[name] = {"d3_id": entry, "tier": tier, "test": entry in test}
    return splits


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--work", type=Path, default=EXTERNAL / "pcbench-work/newdrc")
    parser.add_argument("--pcbench", type=Path, default=EXTERNAL / "_downloads/PCBench")
    parser.add_argument("--pcbworld", type=Path, default=EXTERNAL / "_downloads/PCBWorld")
    parser.add_argument("--binary", type=Path, default=ROOT / "target/release/pcb-maker")
    parser.add_argument("--workers", type=int, default=8)
    parser.add_argument("--output", type=Path, default=Path(__file__).with_name("manifest.json"))
    arguments = parser.parse_args()

    # Only boards whose guide board passed DRC (guide.py) are benchmarks.
    guide = json.loads((arguments.work.parent / "guide.json").read_text())
    passed = {entry["name"] for entry in guide if entry["status"] == "pass"}
    names = sorted(entry.name for entry in arguments.work.iterdir()
                   if (entry / f"{STEM}.kicad_pcb").exists() and entry.name in passed)
    published = baselines(arguments.pcbench)
    splits = d3_splits(arguments.pcbworld)
    with ProcessPoolExecutor(arguments.workers) as pool:
        boards = list(pool.map(characterize, [(name, arguments.work / name, arguments.binary) for name in names]))
    for board in boards:
        board["d3"] = splits.get(board["name"])
        board["pcbench"] = published.get(board["name"])
    missing = sorted(set(splits) - set(names))
    manifest = {
        "guide_failed": sorted(entry["name"] for entry in guide if entry["status"] != "pass"),
        "comment": "Generated by benchmarks/pcbench/manifest.py from the boards benchmarks/pcbench/fetch.sh "
                   "prepares. Boards are PCBench (github.com/PCBench/PCBench @ dec3be75) through PCBWorld's D3 "
                   "chain (github.com/LGAI-Research/PCBWorld @ b3d62f5) with the local KiCad.",
        "kicad": subprocess.run(["kicad-cli", "version"], capture_output=True, text=True).stdout.strip(),
        "board_file": STEM,
        "d3_boards_missing_here": missing,
        "boards": boards,
    }
    arguments.output.write_text(json.dumps(manifest, indent=1) + "\n")
    tiers = {}
    for board in boards:
        tier = (board["d3"] or {}).get("tier", "not in d3")
        tiers[tier] = tiers.get(tier, 0) + 1
    print(f"{len(boards)} boards ({tiers}); {len(missing)} D3 boards not reproduced here", file=sys.stderr)


if __name__ == "__main__":
    main()
