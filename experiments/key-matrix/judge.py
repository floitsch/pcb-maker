#!/usr/bin/env python3
# Copyright (C) 2026 Toit contributors.
"""Judges the key matrices import-kicad-netlist infers (its report's
`starter`, `constraints.starter.json`) against the designer's boards.

    experiments/key-matrix/judge.py <out-dir> --binary build/bin/pcb-maker-xNNN
        [--jobs 4] [--only NAME ...]

Per harvested keyboard schematic: import it as the schematic tier does
(with the designer's board as --footprints-from), then, on the designer's
board (describe-kicad-board):
- rows: each line of the starter's `row`s (a grid's lines, a single row)
  lies on a line: the switch centres' largest distance from their best-fit
  line is within the row's key pitch (`line`); along that line the keys
  come in the row's order (`ordered`, either direction); the median
  neighbour spacing against the pitch;
- below: each `relative` (a row below the one before) holds on the
  designer's board: the lower row's centre lies below the upper's (larger y
  across the upper row's line);
- diodes: each `near` diode within its 3 mm of its key's body on the
  designer's board (box gap), and its centre distance;
- halves: the two halves' keys lie apart (their boxes do not overlap in
  x), and whether the region's left half is the designer's left.
Writes <out-dir>/rows.json and prints a table."""

import argparse
import json
import math
import statistics
import sys
from concurrent.futures import ThreadPoolExecutor
from pathlib import Path

ROOT = Path(__file__).resolve().parents[2]
sys.path.insert(0, str(ROOT / "benchmarks/agent-tasks"))
import schematic  # noqa: E402

BOARDS = ROOT / "benchmarks/real/external/github"
KEYBOARDS = [
    "duckyb__urchin__main",
    "sporkus__le_chiffre_keyboard_stm32__stm32_chiffre_36keys",
    "sporkus__le_chiffre_keyboard_stm32__stm32_hotswap_chiffre",
    "tubbytwins__bumwings-kbd__bumwings_v001",
    "headblockhead__slab-pcb__slab-pcb-left",
    "headblockhead__slab-pcb__interchange-pcb-right",
    "byrantech__laptop__keyboard",
    "siderakb__ergo-snm-keyboard__ErgoSNM_keyboard",
    "zli117__CyberKeeb2040__MainBoard",
    "duckyb__eternal-keypad__eternal-keypad",
    "Goga64__ULK__ULK",
    "Ladniy__jiran-ble-lite__jiran-ble-lite",
    "sporkus__capybully_keyboard__capybully",
    "obsilab__Quanta75__Quanta75_BareRP2040_JLCPCBAoptimized",
    "thpoll83__PolyKybd__poly_corne_split42_left",
    "ebastler__osprey__osprey_rev_a",
    "tzarc__keyboards__ghoul",
    "CDFER__Business-Cards__USB_Keypad",
]


def line_fit(points):
    """Direction of the best-fit line through the points (principal axis)."""
    mx = sum(p[0] for p in points) / len(points)
    my = sum(p[1] for p in points) / len(points)
    sxx = sum((p[0] - mx) ** 2 for p in points)
    syy = sum((p[1] - my) ** 2 for p in points)
    sxy = sum((p[0] - mx) * (p[1] - my) for p in points)
    angle = 0.5 * math.atan2(2 * sxy, sxx - syy)
    return (mx, my), (math.cos(angle), math.sin(angle))


def box_gap(a, b):
    dx = max(a[0] - b[2], b[0] - a[2], 0.0)
    dy = max(a[1] - b[3], b[1] - a[3], 0.0)
    return math.hypot(dx, dy)


def lines_of(entry):
    parts, columns = entry["parts"], entry.get("columns") or len(entry["parts"])
    return [parts[at:at + columns] for at in range(0, len(parts), columns)]


def one(name, arguments, records):
    record = records[name]
    board_id = record["board_id"]
    work = arguments.output / name
    if work.exists():
        import shutil
        shutil.rmtree(work)
    work.mkdir(parents=True)
    designer_file = BOARDS / name / f"{board_id}.kicad_pcb"
    report, steps, error = schematic.import_schematic(
        arguments.binary, record, work, board_id, 2, designer_file)
    row = {"name": name}
    if error:
        row["error"] = error
        return row
    starter = report.get("starter")
    row["starter"] = starter
    if not starter or not starter.get("file"):
        return row
    constraints = json.loads(Path(starter["file"]).read_text())
    code, stdout, _ = schematic.run([arguments.binary, "describe-kicad-board", BOARDS / name, board_id])
    footprints = {f["reference"]: f for f in json.loads(stdout)["footprints"]}
    centre = lambda reference: footprints[reference]["at"][:2]
    lines = []
    for entry in constraints.get("row", []):
        for members in lines_of(entry):
            members = [member for member in members if member in footprints]
            if len(members) < 2:
                continue
            points = [centre(member) for member in members]
            origin, direction = line_fit(points)
            normal = (-direction[1], direction[0])
            off = max(abs((p[0] - origin[0]) * normal[0] + (p[1] - origin[1]) * normal[1]) for p in points)
            along = [(p[0] - origin[0]) * direction[0] + (p[1] - origin[1]) * direction[1] for p in points]
            steps_along = [b - a for a, b in zip(along, along[1:])]
            ordered = all(step > 0 for step in steps_along) or all(step < 0 for step in steps_along)
            spacing = statistics.median(math.dist(a, b) for a, b in zip(points, points[1:]))
            lines.append({"parts": members, "off_line_mm": round(off, 2), "line": off <= entry["pitch_mm"],
                          "ordered": ordered, "spacing_mm": round(spacing, 2), "pitch_mm": entry["pitch_mm"]})
    below = []
    for entry in constraints.get("relative", []):
        if "below" not in entry:
            continue
        upper, lower = entry["below"], entry["part"]
        if upper in footprints and lower in footprints:
            below.append({"part": lower, "below": upper, "kept": centre(lower)[1] > centre(upper)[1]})
    diodes = []
    for entry in constraints.get("near", []):
        part, key = entry["part"], entry["part_of"]
        if not part.startswith("D") or part not in footprints or key not in footprints:
            continue
        gap = box_gap(footprints[part]["body"], footprints[key]["body"])
        diodes.append({"diode": part, "key": key, "gap_mm": round(gap, 2), "kept": gap <= entry["max_mm"] + 1e-6,
                       "centres_mm": round(math.dist(centre(part), centre(key)), 2)})
    halves = None
    regions = constraints.get("region", [])
    if len(regions) == 2:
        # Keys of each half: its rows' parts (a row is in the half its
        # leader's region names).
        by_half = []
        for region in regions:
            leaders = set(region["parts"])
            keys = [k for entry in constraints["row"] if entry["parts"][0] in leaders for k in entry["parts"]]
            keys += [k for k in leaders if k in footprints and k.upper().startswith(("SW", "K", "MX"))]
            xs = [centre(k)[0] for k in keys if k in footprints]
            by_half.append((min(xs), max(xs), sum(xs) / len(xs)))
        apart = by_half[0][1] < by_half[1][0] or by_half[1][1] < by_half[0][0]
        halves = {"apart": apart, "left_is_left": by_half[0][2] < by_half[1][2]}
    row["lines"] = lines
    row["below"] = below
    row["diodes"] = diodes
    row["halves"] = halves
    return row


def main():
    parser = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    parser.add_argument("output", type=Path)
    parser.add_argument("--binary", type=Path, required=True)
    parser.add_argument("--jobs", type=int, default=4)
    parser.add_argument("--only", nargs="*")
    arguments = parser.parse_args()
    arguments.binary = arguments.binary.resolve()
    arguments.output = arguments.output.resolve()
    records = json.loads((ROOT / "benchmarks/github/schematics.json").read_text())["boards"]
    names = [name for name in KEYBOARDS if name in records and (not arguments.only or name in arguments.only)]
    with ThreadPoolExecutor(arguments.jobs) as pool:
        rows = list(pool.map(lambda name: one(name, arguments, records), names))
    print(f"{'board':42} {'matrix':>7} {'sw':>3} {'dio':>3} {'halves':>6} {'unas':>4} {'lines ok':>9} {'ordered':>8} {'below':>6} {'diodes ok':>10}")
    for row in rows:
        starter = row.get("starter") or {}
        if row.get("error"):
            print(f"{row['name'][:42]:42} error {row['error']}")
            continue
        lines, below, diodes = row.get("lines", []), row.get("below", []), row.get("diodes", [])
        matrix = "x".join(map(str, starter.get("matrix", []))) or "-"
        print(f"{row['name'][:42]:42} {matrix:>7} {starter.get('switches', 0):>3} {starter.get('diodes', 0):>3} "
              f"{len(starter.get('halves', [])) if starter.get('halves_by') else 1 if starter.get('file') else 0:>6} "
              f"{len(starter.get('unassigned', [])):>4} "
              f"{sum(l['line'] for l in lines):>4}/{len(lines):<4} {sum(l['ordered'] for l in lines):>3}/{len(lines):<4} "
              f"{sum(b['kept'] for b in below):>2}/{len(below):<3} {sum(d['kept'] for d in diodes):>4}/{len(diodes):<5}"
              + (f" halves {row['halves']}" if row.get("halves") else "")
              + (f" ({starter['why'][:70]})" if starter.get("why") else ""))
    (arguments.output / "rows.json").write_text(json.dumps(rows, indent=1))


if __name__ == "__main__":
    main()
