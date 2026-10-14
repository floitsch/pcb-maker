# Copyright (C) 2026 Toit contributors.
"""Starved-thermal verdicts of two pour models against KiCad's, by cause.

    python3 -I causes.py <census-directory>...

Each directory holds census.py's per-board JSON for one model (name the
old model's directories with "old" in them). For every thermal pad and
layer where a model's verdict (starved by the DRC's rule on its counts)
differs from KiCad's DRC, the first cause that explains it:

- missing: the model has no such pad (pads sharing a number, custom pads);
- custom: the DRC compares against a custom pad's spoke template;
- tracks: KiCad counts the pad's tracks as spokes and the model's count
  of them differs;
- island: KiCad counts a spoke the model calls an island (or the reverse);
- landing: KiCad has a spoke where the model finds no fill;
- clearance: KiCad has a spoke the model finds blocked;
- extra: the model has a spoke where KiCad has none;
- other.
"""

import collections
import glob
import json
import os
import sys

sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
import spokes  # noqa: E402

ORDER = ["+x", "-x", "+y", "-y"]
ROUND = ["+x+y", "+x-y", "-x+y", "-x-y"]


def cause(row):
    if "our_spokes" not in row:
        return "missing"
    if row.get("drc_starved") and "custom" in row["drc_starved"]:
        return "custom"
    names = ROUND if row.get("round") else ORDER
    kicad = collections.Counter(row.get("directions", []))
    kinds = []
    for name, outcome in zip(names, row["our_spokes"]):
        ours = outcome.split()[0]
        if kicad.get(name, 0) >= 2:
            kinds.append({"good": None, "island": "island", "none": "landing", "blocked": "clearance"}[ours])
        elif kicad.get(name + "*", 0) >= 2:
            kinds.append("island" if ours == "good" else None)
        elif ours == "good":
            kinds.append("extra")
    kinds = [kind for kind in kinds if kind]
    if row.get("kicad_tracks", 0) != row.get("ours_tracks", 0):
        return "tracks"
    return kinds[0] if kinds else "other"


def main():
    for directory in sys.argv[1:]:
        verdicts = collections.Counter()
        causes = {"missed": collections.Counter(), "false alarm": collections.Counter()}
        for path in sorted(glob.glob(os.path.join(directory, "*.json"))):
            for row in json.load(open(path)):
                drc = bool(row.get("drc_starved"))
                mine = spokes.our_starved(row)
                if drc and mine:
                    verdicts["both"] += 1
                elif drc:
                    verdicts["missed"] += 1
                    causes["missed"][cause(row)] += 1
                elif mine:
                    verdicts["false alarm"] += 1
                    causes["false alarm"][cause(row)] += 1
                verdicts["starved"] += drc
        print(directory)
        print("  ", dict(verdicts))
        for kind, counter in causes.items():
            print("  ", kind, dict(counter.most_common()))


if __name__ == "__main__":
    main()
