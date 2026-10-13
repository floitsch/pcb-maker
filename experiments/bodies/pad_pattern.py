#!/usr/bin/env python3
# Copyright (C) 2026 Toit contributors.
"""The pad-pattern rule for courtyard-less footprints, evaluated on the
census (`courtless_census.py --json`): which footprints it calls solid
(a body fills the pad box), and which of those hold a part the designer put
inside the pad box (the rule forbids the designer's own placement there).

    experiments/bodies/pad_pattern.py census.json [--pitch 2.6] [--share 0.75]
"""
import argparse
import collections
import json
from pathlib import Path


def pin_rows(pads, pitch, tolerance=0.05):
    """The pads (indices) that lie in a row or column of three or more
    pads whose neighbours are at most `pitch` apart."""
    members = set()
    for axis in (0, 1):
        lines = collections.defaultdict(list)
        for index, pad in enumerate(pads):
            key = round(pad[1 - axis] / tolerance)
            # Pads within the tolerance of a line join it.
            for candidate in (key - 1, key, key + 1):
                if candidate in lines:
                    key = candidate
                    break
            lines[key].append(index)
        for line in lines.values():
            line.sort(key=lambda index: pads[index][axis])
            run = [line[0]]
            for previous, index in zip(line, line[1:]):
                gap = pads[index][axis] - pads[previous][axis]
                if 1.0e-6 < gap <= pitch:
                    run.append(index)
                else:
                    if len(run) >= 3:
                        members.update(run)
                    run = [index]
            if len(run) >= 3:
                members.update(run)
    return members


def solid(pad_list, pitch=2.6, share=0.75, minimum=4):
    # One pad per position (stacked pads of one pin count once).
    seen = {}
    for pad in pad_list:
        seen.setdefault((round(pad[0], 3), round(pad[1], 3)), pad)
    pads = list(seen.values())
    if len(pads) < minimum:
        return False
    members = pin_rows(pads, pitch)
    return len(members) >= share * len(pads)


def main():
    parser = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    parser.add_argument("census", type=Path)
    parser.add_argument("--pitch", type=float, default=2.6)
    parser.add_argument("--share", type=float, default=0.75)
    parser.add_argument("--show", action="store_true")
    arguments = parser.parse_args()
    data = json.loads(arguments.census.read_text())
    rows = [(board, row) for board, value in data.items() for row in value["courtless"]]
    called = [(board, row) for board, row in rows if solid(row["pad_list"], arguments.pitch, arguments.share)]
    wrong = [(board, row) for board, row in called if row["inside_pad_box"]]
    print(f"courtyard-less {len(rows)}; solid {len(called)} ({sum(1 for _, r in called if not r['fixed'])} movable); "
          f"solid holding a designer's part {len(wrong)} ({sum(1 for _, r in wrong if not r['fixed'])} movable)")
    hollow_holding = [(b, r) for b, r in rows if r["inside_pad_box"] and (b, r) not in called]
    print(f"hollow (not solid) and holding a designer's part: {len(hollow_holding)}")
    for board, row in wrong:
        print("  wrong:", board.split("__")[-1][:28], row["reference"], row["library"], row["pads"], row["pad_box_size"],
              "fixed" if row["fixed"] else "movable", row["inside_pad_box"][:5])
    if arguments.show:
        libraries = collections.Counter((row["library"], row["pads"]) for _, row in called)
        for (library, pads), count in libraries.most_common():
            print(f"  solid: {count:4} {library} ({pads} pads)")


if __name__ == "__main__":
    main()
