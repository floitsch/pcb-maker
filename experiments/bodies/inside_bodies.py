#!/usr/bin/env python3
# Copyright (C) 2026 Toit contributors.
"""Parts placed inside the pad box of a courtyard-less footprint whose pads
stand in rows of pins (the boxes `solid_pin_rows` blocks).

    experiments/bodies/inside_bodies.py board.kicad_pcb [...]
"""
import sys
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parent))
from courtless_census import footprint_info, head, holds, parse  # noqa: E402
from pad_pattern import solid  # noqa: E402


def main():
    for path in sys.argv[1:]:
        tree = parse(Path(path).read_text(errors="replace"))
        footprints = [footprint_info(item) for item in tree if head(item) == "footprint"]
        found = 0
        for info in footprints:
            if info["courtyard"] or not info["pads"] or not solid(info["pad_list"]):
                continue
            inside = [
                other["reference"]
                for other in footprints
                if other is not info
                and (other["side"] == info["side"] or (other["through"] and info["through"]))
                and holds(info, info["pad_box"], other)
            ]
            if inside:
                found += 1
                print(f"{path}: {info['reference']} ({info['library']}) holds {inside}")
        print(f"{path}: {found} solid courtyard-less footprints hold parts")


if __name__ == "__main__":
    main()
