#!/usr/bin/env python3
# Copyright (C) 2026 Toit contributors.
# Use of this source code is governed by an MIT-style license that can be
# found in the LICENSE file.
"""Compares placement or layout outputs of one board from inputs in
different orders: per output directory against the first, the footprints
at another pose (by reference and uuid), whether the placed board, the
routed board and their copper (tracks, arcs, vias as a multiset) are the
same, and the layout's line.

    compare.py <out dir> <out dir>...
"""
import json
import re
import sys
from pathlib import Path


def blocks(text, word):
    """The top-level (word ...) lists of a board, as text."""
    result = []
    for match in re.finditer(r"\n  \(" + word + r"[ \n]", text):
        start = match.start() + 3
        depth = 0
        in_string = False
        index = start
        while True:
            char = text[index]
            if in_string:
                if char == "\\":
                    index += 1
                elif char == '"':
                    in_string = False
            elif char == '"':
                in_string = True
            elif char == "(":
                depth += 1
            elif char == ")":
                depth -= 1
                if depth == 0:
                    break
            index += 1
        result.append(text[start:index + 1])
    return result


def poses(text):
    result = {}
    for block in blocks(text, "footprint"):
        reference = re.search(r'\((?:property "Reference"|fp_text reference) "([^"]*)"', block)
        uuid = re.search(r'\n    \((?:uuid|tstamp) "?([^")]*)"?\)', block)
        layer = re.search(r'\(layer "([^"]+)"\)', block)
        at = re.search(r'\(at ([-\d.]+) ([-\d.]+)(?: ([-\d.]+))?\)', block)
        key = (reference.group(1) if reference else "", uuid.group(1) if uuid else "")
        while key in result:
            key = (key[0], key[1] + "'")
        result[key] = (round(float(at.group(1)), 4), round(float(at.group(2)), 4),
                       round(float(at.group(3) or 0) % 360, 2), layer.group(1) if layer else "")
    return result


def copper(text):
    return sorted(block for word in ("segment", "arc", "via") for block in blocks(text, word))


def board(directory, placed):
    directory = Path(directory)
    candidates = list((directory / "placed").glob("*.kicad_pcb")) if placed else []
    if not placed:
        candidates = list((directory / "result").glob("*.kicad_pcb")) or list(directory.glob("*.kicad_pcb"))
    if placed and not candidates:
        candidates = list(directory.glob("*.kicad_pcb"))
    return candidates[0].read_text() if candidates else None


def line(directory):
    directory = Path(directory)
    layout = json.loads((directory / "board-layout.json").read_text()) if (directory / "board-layout.json").exists() else None
    placer_path = directory / "placed/board-placer.json"
    if not placer_path.exists():
        placer_path = directory / "board-placer.json"
    placer = json.loads(placer_path.read_text()) if placer_path.exists() else {}
    verdict = json.loads((directory / "verdict.json").read_text()) if (directory / "verdict.json").exists() else {}
    text = f"HPWL {placer.get('wirelength_final_mm', 0):.1f}, seed {placer.get('seed_kept')}/{placer.get('seeds')}"
    if layout:
        routed = layout.get("routed", {})
        text = (f"vias {routed.get('vias')}, length {routed.get('length_mm', 0):.0f}, "
                f"KiCad unconnected {verdict.get('kicad_unconnected_items')}, " + text +
                (f", CUT {verdict.get('cut')}" if verdict.get("cut") else ""))
    return text


def main():
    directories = sys.argv[1:]
    first_placed = board(directories[0], True)
    first_routed = board(directories[0], False) if (Path(directories[0]) / "board-layout.json").exists() else None
    reference = poses(first_placed)
    print(f"{Path(directories[0]).name}: {len(reference)} footprints; {line(directories[0])}")
    for directory in directories[1:]:
        placed = board(directory, True)
        other = poses(placed)
        moved = [key for key in reference if reference[key] != other.get(key)]
        result = f"{Path(directory).name}: {len(moved)} of {len(reference)} at another pose"
        result += ", placed board " + ("identical" if placed == first_placed else "differs")
        if first_routed is not None:
            routed = board(directory, False)
            result += ", routed board " + ("identical" if routed == first_routed else "differs")
            result += ", copper " + ("identical" if copper(routed) == copper(first_routed) else "differs")
        print(result + "; " + line(directory))


if __name__ == "__main__":
    main()
