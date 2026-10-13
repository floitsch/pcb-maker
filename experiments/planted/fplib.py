#!/usr/bin/env python3
# Copyright (C) 2026 Toit contributors.
"""Collects real footprints from the harvested boards into one library
board (build/planted/fplib.kicad_pcb), each at the origin, unrotated, on
the front, without nets, plus fplib.json (name -> category, pads, source).

    experiments/planted/fplib.py [--scan] [--survey build/planted/fp_survey.json]

The survey (footprint name -> one board and reference that has it) comes
from scanning benchmarks/real/external/github once (`--scan`, a few
minutes)."""

import argparse
import json
import sys
from pathlib import Path

ROOT = Path(__file__).resolve().parents[2]

# name -> category. Categories drive the generators' part mix.
WANTED = {
    "C_0402_1005Metric": "passive", "R_0402_1005Metric": "passive",
    "C_0603_1608Metric": "passive", "R_0603_1608Metric": "passive",
    "C_0805_2012Metric": "passive", "R_0805_2012Metric": "passive",
    "LED_0603_1608Metric": "passive", "D_SOD-123": "passive", "C_1206_3216Metric": "passive",
    "SOT-23": "small", "SOT-23-5": "small", "SOT-23-6": "small", "SOT-223-3_TabPin2": "small",
    "SOIC-8_3.9x4.9mm_P1.27mm": "small", "MSOP-8_3x3mm_P0.65mm": "small",
    "Crystal_SMD_3225-4Pin_3.2x2.5mm": "small",
    "TSSOP-14_4.4x5mm_P0.65mm": "ic", "TSSOP-16_4.4x5mm_P0.65mm": "ic",
    "SSOP-16_3.9x4.9mm_P0.635mm": "ic", "SOIC-16_3.9x9.9mm_P1.27mm": "ic",
    "QFN-24-1EP_4x4mm_P0.5mm_EP2.6x2.6mm": "ic", "QFN-32-1EP_5x5mm_P0.5mm_EP3.7x3.7mm": "ic",
    "LQFP-48_7x7mm_P0.5mm": "ic", "TQFP-64_10x10mm_P0.5mm": "ic", "TQFP-44_10x10mm_P0.8mm": "ic",
    "LQFP-32_7x7mm_P0.8mm": "ic",
    "PinHeader_1x02_P2.54mm_Vertical": "connector", "PinHeader_1x03_P2.54mm_Vertical": "connector",
    "PinHeader_1x04_P2.54mm_Vertical": "connector", "PinHeader_1x06_P2.54mm_Vertical": "connector",
    "PinHeader_1x08_P2.54mm_Vertical": "connector", "PinHeader_2x05_P2.54mm_Vertical": "connector",
    "PinHeader_2x03_P2.54mm_Vertical": "connector",
    "DIP-8_W7.62mm": "tht", "DIP-16_W7.62mm": "tht", "C_Disc_D4.3mm_W1.9mm_P5.00mm": "tht",
    "R_Axial_DIN0207_L6.3mm_D2.5mm_P7.62mm_Horizontal": "tht", "TO-92L_Inline": "tht",
    "MountingHole_2.2mm_M2_DIN965": "hole", "TestPoint_Pad_D1.0mm": "testpoint",
    "TestPoint_THTPad_D1.0mm_Drill0.5mm": "testpoint",
}


def scan(out):
    import collections
    import pcbnew
    count = collections.Counter()
    example = {}
    for path in sorted((ROOT / "benchmarks/real/external/github").glob("*/*.kicad_pcb")):
        try:
            board = pcbnew.LoadBoard(str(path))
        except Exception:  # noqa: BLE001 - an unreadable board is skipped
            board = None
        if board is None:
            continue
        for fp in board.GetFootprints():
            name = str(fp.GetFPID().GetLibItemName())
            count[name] += 1
            example.setdefault(name, (str(path.relative_to(ROOT)), fp.GetReference(), len(list(fp.Pads()))))
    Path(out).write_text(json.dumps({"count": count, "ex": example}))


def main():
    parser = argparse.ArgumentParser()
    parser.add_argument("--scan", action="store_true", help="rebuild the survey first")
    parser.add_argument("--survey", default=str(ROOT / "build/planted/fp_survey.json"))
    parser.add_argument("--out", default=str(ROOT / "build/planted/fplib.kicad_pcb"))
    arguments = parser.parse_args()
    import pcbnew
    if arguments.scan or not Path(arguments.survey).exists():
        scan(arguments.survey)
    survey = json.loads(Path(arguments.survey).read_text())["ex"]
    by_board = {}
    for name in WANTED:
        if name not in survey:
            print(f"missing {name}", file=sys.stderr)
            continue
        board, reference, _ = survey[name]
        by_board.setdefault(board, []).append((name, reference))
    library = pcbnew.BOARD()
    library.SetCopperLayerCount(4)
    index = {}
    for board_path, items in sorted(by_board.items()):
        board = pcbnew.LoadBoard(str(ROOT / board_path) if not Path(board_path).is_absolute() else board_path)
        for name, reference in items:
            source = None
            for fp in board.GetFootprints():
                if fp.GetReference() == reference and str(fp.GetFPID().GetLibItemName()) == name:
                    source = fp
                    break
            if source is None:
                print(f"not found {name} {reference}", file=sys.stderr)
                continue
            fp = pcbnew.FOOTPRINT(source)
            library.Add(fp)
            if fp.IsFlipped():
                fp.Flip(fp.GetPosition(), pcbnew.FLIP_DIRECTION_TOP_BOTTOM)
            fp.SetOrientationDegrees(0)
            fp.SetPosition(pcbnew.VECTOR2I(0, 0))
            for pad in fp.Pads():
                pad.SetNetCode(0)
            fp.SetReference(f"LIB{len(index) + 1}")
            fp.SetValue(name)
            index[name] = {"category": WANTED[name], "pads": len(list(fp.Pads())),
                           "source": board_path, "reference": reference,
                           "library_reference": fp.GetReference()}
    pcbnew.SaveBoard(arguments.out, library)
    Path(arguments.out).with_suffix(".json").write_text(json.dumps(index, indent=1))
    print(f"{len(index)} footprints -> {arguments.out}")


if __name__ == "__main__":
    main()
