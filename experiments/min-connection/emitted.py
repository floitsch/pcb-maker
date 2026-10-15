#!/usr/bin/env python3
# Copyright (C) 2026 Toit contributors.
"""Tracks narrower than the project's min_connection in routed boards.

    emitted.py <run directory> [<run directory> ...]

Each run directory holds one directory per board with source/<id>.kicad_pro
and a routed result/<id>.kicad_pcb (layout/result or result).
"""
import json
import re
import sys
from collections import Counter
from pathlib import Path

SEGMENT = re.compile(r"\((?:segment|arc)\b.*?\(width ([0-9.]+)\)", re.S)


def main():
    for run in sys.argv[1:]:
        for board in sorted(Path(run).iterdir()):
            projects = list((board / "source").glob("*.kicad_pro"))
            if not projects:
                continue
            project = json.load(open(projects[0]))
            design = project.get("board", {}).get("design_settings", {})
            minimum = design.get("rules", {}).get("min_connection", 0.0) or 0.0
            if minimum <= 0:
                continue
            severity = design.get("rule_severities", {}).get("connection_width", "warning")
            pcbs = [p for p in board.rglob("result/*.kicad_pcb")]
            if not pcbs:
                print(f"{board.name}\t{minimum}\t{severity}\tno result")
                continue
            text = pcbs[0].read_text()
            widths = Counter(float(w) for w in SEGMENT.findall(text))
            below = {w: n for w, n in widths.items() if w < minimum - 1e-6}
            print(f"{board.name}\t{minimum}\t{severity}\t{sum(widths.values())} tracks\tbelow: {sum(below.values())} {dict(sorted(below.items()))}")


if __name__ == "__main__":
    main()
