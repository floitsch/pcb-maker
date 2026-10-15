#!/usr/bin/env python3
# Copyright (C) 2026 Toit contributors.
# Use of this source code is governed by an MIT-style license that can be
# found in the LICENSE file.
"""Sets the footprints' angles of a board (pads turned with them, positions
kept): to the designer's (`--from <designer board>`, footprints matched by
their symbol path, else by reference) or to 0 degrees (`--zero`). Parts the
constraints place (`place` entries) keep theirs.

    set_angles.py <board.kicad_pcb> <out.kicad_pcb> (--from <designer> | --zero) [--constraints c.json]

Prints how many footprints it turned. Uses KiCad's pcbnew module.
"""
import argparse
import json
import sys

import pcbnew


def main():
    parser = argparse.ArgumentParser()
    parser.add_argument("board")
    parser.add_argument("out")
    parser.add_argument("--from", dest="designer")
    parser.add_argument("--zero", action="store_true")
    parser.add_argument("--constraints")
    arguments = parser.parse_args()
    if bool(arguments.designer) == arguments.zero:
        sys.exit("give --from or --zero")
    keep = set()
    if arguments.constraints:
        constraints = json.load(open(arguments.constraints))
        keep = {entry["part"] for entry in constraints.get("place", []) if "x" in entry}
    board = pcbnew.LoadBoard(arguments.board)
    angles_by_path, angles_by_reference = {}, {}
    if arguments.designer:
        designer = pcbnew.LoadBoard(arguments.designer)
        for footprint in designer.GetFootprints():
            angle = footprint.GetOrientationDegrees()
            path = footprint.GetPath().AsString()
            if path:
                angles_by_path[path] = angle
            angles_by_reference.setdefault(footprint.GetReference(), []).append(angle)
    turned = unmatched = 0
    for footprint in board.GetFootprints():
        reference = footprint.GetReference()
        if reference in keep:
            continue
        if arguments.zero:
            angle = 0.0
        else:
            path = footprint.GetPath().AsString()
            if path in angles_by_path:
                angle = angles_by_path[path]
            elif len(angles_by_reference.get(reference, [])) == 1:
                angle = angles_by_reference[reference][0]
            else:
                unmatched += 1
                continue
        if abs(((footprint.GetOrientationDegrees() - angle) + 180) % 360 - 180) > 1e-6:
            footprint.SetOrientationDegrees(angle)
            turned += 1
    board.Save(arguments.out)
    print(f"turned {turned}, unmatched {unmatched}")


if __name__ == "__main__":
    main()
