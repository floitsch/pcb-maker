# Copyright (C) 2026 Toit contributors.
"""Per spoke direction: KiCad's spoke against the model's outcome.

    python3 -I classify.py <census-directory> [--examples N]

Reads the per-board JSON of census.py and counts, over every thermal pad
layer, the directions where KiCad has a spoke (a crossing pair, into an
outline with more than one item) and what the model made of that
direction (good, island, blocked, none), and the reverse (the model good,
KiCad no spoke there). Directions are matched by name (+x, -x, +y, -y;
diagonals for round pads)."""

import collections
import glob
import json
import os
import sys

ORDER = ["+x", "-x", "+y", "-y"]
ROUND = ["+x+y", "+x-y", "-x+y", "-x-y"]


def main():
    directory = sys.argv[1]
    examples = int(sys.argv[3]) if len(sys.argv) > 3 and sys.argv[2] == "--examples" else 0
    table = collections.Counter()
    shown = collections.Counter()
    for path in sorted(glob.glob(os.path.join(directory, "*.json"))):
        board = os.path.basename(path)[:-5]
        for row in json.load(open(path)):
            if "our_spokes" not in row:
                table[("pad missing in model",)] += 1
                continue
            names = ROUND if row.get("round") else ORDER
            kicad = collections.Counter(name for name in row.get("directions", []))
            kicad_dirs = {name for name in names if kicad.get(name, 0) >= 2}
            single_dirs = {name for name in names if kicad.get(name + "*", 0) >= 2}
            other = sorted(name for name in kicad if name.rstrip("*") not in names)
            if other:
                table[("kicad spoke off the model's axes", row.get("spoke_angle"))] += 1
            for name, outcome in zip(names, row["our_spokes"]):
                kind = outcome.split()[0]
                truth = "spoke" if name in kicad_dirs else "single" if name in single_dirs else "no spoke"
                table[(truth, kind)] += 1
                if truth == "spoke" and kind != "good" and shown[kind] < examples:
                    shown[kind] += 1
                    print(board, row["pad"], row["layer"], name, kind, row.get("size"), row.get("shape"), row.get("gap"), row.get("spoke_width"), file=sys.stderr)
    for key, count in sorted(table.items(), key=lambda item: -item[1]):
        print(f"{count:6} {key}")


if __name__ == "__main__":
    main()
