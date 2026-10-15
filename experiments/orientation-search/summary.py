#!/usr/bin/env python3
# Copyright (C) 2026 Toit contributors.
# Use of this source code is governed by an MIT-style license that can be
# found in the LICENSE file.
"""One line per layout output directory: routed, vias, length, KiCad's
unconnected items and new errors, the placement's weighted HPWL (final),
the kept seed, the work; and, given pairs (`--pair a b`), whether the two
placements put every footprint at the same pose (and how many differ).

    summary.py <out dir>... [--pair <out a> <out b>]...
"""
import argparse
import json
import re
from pathlib import Path


def poses(directory):
    """Reference -> (x, y, angle, layer) of the placed board."""
    boards = list((Path(directory) / "placed").glob("*.kicad_pcb")) or list(Path(directory).glob("*.kicad_pcb"))
    if not boards:
        return {}
    text = boards[0].read_text()
    result = {}
    for block in re.split(r'\n\s*\(footprint "', text)[1:]:
        reference = re.search(r'\((?:property "Reference"|fp_text reference) "([^"]*)"', block)
        layer = re.search(r'\(layer "([^"]+)"\)', block)
        at = re.search(r'\(at ([-\d.]+) ([-\d.]+)(?: ([-\d.]+))?\)', block)
        if reference and at:
            angle = float(at.group(3) or 0) % 360
            key = reference.group(1)
            while key in result:
                key += "'"
            result[key] = (round(float(at.group(1)), 3), round(float(at.group(2)), 3), round(angle, 1),
                           layer.group(1) if layer else "")
    return result


def line(directory):
    directory = Path(directory)
    layout = json.loads((directory / "board-layout.json").read_text()) if (directory / "board-layout.json").exists() else {}
    verdict = json.loads((directory / "verdict.json").read_text()) if (directory / "verdict.json").exists() else {}
    placer = json.loads((directory / "placed/board-placer.json").read_text()) if (directory / "placed/board-placer.json").exists() else {}
    routed = layout.get("routed", {})
    new = verdict.get("new_findings") or {}
    return (f"{directory.name}: routed {routed.get('routed_connections')}/{routed.get('routable_connections')}, "
            f"vias {routed.get('vias')}, length {round(routed.get('length_mm', 0))}, "
            f"KiCad unconnected {verdict.get('kicad_unconnected_items')}, new {dict(new)}, "
            f"HPWL {round(placer.get('wirelength_final_mm', 0))} (global {round(placer.get('wirelength_global_mm', 0))}), "
            f"seed {placer.get('seed_kept')}/{placer.get('seeds')}, work {round(verdict.get('work_seconds') or 0)}, "
            f"consistency {layout.get('quality', {}).get('assembly', {}).get('orientation_consistency')}, "
            f"exit {verdict.get('exit_code')}{' CUT ' + str(verdict.get('cut')) if verdict.get('cut') else ''}")


def main():
    parser = argparse.ArgumentParser()
    parser.add_argument("directories", nargs="*")
    parser.add_argument("--pair", nargs=2, action="append", default=[])
    arguments = parser.parse_args()
    for directory in arguments.directories:
        print(line(directory))
    for a, b in arguments.pair:
        first, second = poses(a), poses(b)
        differ = [reference for reference in first if first[reference] != second.get(reference)]
        turned = [reference for reference in first
                  if reference in second and first[reference][2] != second[reference][2]]
        print(f"{Path(a).name} vs {Path(b).name}: {len(first)} footprints, {len(differ)} at another pose, "
              f"{len(turned)} at another angle" + (" (identical)" if not differ else ""))


if __name__ == "__main__":
    main()
