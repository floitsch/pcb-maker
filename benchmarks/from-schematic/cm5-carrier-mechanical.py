# Copyright (C) 2026 Florian Loitsch.
"""Derive the CM5Carrier cold benchmark's held interfaces and assembly sides."""

import argparse
import hashlib
import json
from pathlib import Path

import pcbnew


SOURCE_SHA256 = "9f816c7ce8fb9eac7fd12980e71809d3a4c2551bc8f9e6724bf63378ae843d95"
HELD = {
    "Conn1", "Conn2", "Module1", "J1", "J2", "J3", "J4", "J5", "J6", "J7", "J8",
    "U4", "D2", "D3",
}


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("source", type=Path)
    parser.add_argument("mechanical", type=Path)
    parser.add_argument("brief", type=Path)
    args = parser.parse_args()
    if hashlib.sha256(args.source.read_bytes()).hexdigest() != SOURCE_SHA256:
        parser.error("the source must be the pinned CM5Carrier board")
    if len({item.resolve() for item in (args.source, args.mechanical, args.brief)}) != 3:
        parser.error("source board, mechanical donor and brief must be separate files")

    board = pcbnew.LoadBoard(str(args.source.resolve()))
    footprints = sorted(board.GetFootprints(), key=lambda part: part.GetReference())
    references = {part.GetReference() for part in footprints}
    if len(footprints) != 54 or len(references) != 54 or not HELD <= references:
        raise ValueError("expected 54 distinct schematic-backed references and every interface")
    brief = {"version": 1, "move_all": True, "place": [], "front": [], "back": []}
    for part in footprints:
        ref = part.GetReference()
        if not part.GetPath().AsString().strip("/"):
            raise ValueError(f"expected a schematic path for {ref}")
        part.SetLocked(ref in HELD)
        brief["back" if part.IsFlipped() else "front"].append(ref)
        if ref in HELD:
            point = part.GetPosition()
            brief["place"].append({
                "part": ref, "origin": "absolute", "x": pcbnew.ToMM(point.x),
                "y": pcbnew.ToMM(point.y), "angle": part.GetOrientationDegrees(),
            })
    if not pcbnew.SaveBoard(str(args.mechanical.resolve()), board):
        raise ValueError(f"could not save {args.mechanical}")
    args.brief.write_text(json.dumps(brief, indent=2) + "\n")


if __name__ == "__main__":
    main()
