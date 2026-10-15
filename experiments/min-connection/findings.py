#!/usr/bin/env python3
# Copyright (C) 2026 Toit contributors.
"""KiCad's findings of routed boards by type and severity, with the
unconnected count and the vias, and the routed tracks' width counts.

    findings.py <run directory> [<run directory> ...]
"""
import json
import re
import sys
from collections import Counter
from pathlib import Path


def main():
    for run in sys.argv[1:]:
        for board in sorted(p for p in Path(run).iterdir() if p.is_dir()):
            reports = sorted(board.glob("route/drc.json")) or sorted(board.glob("layout/result/drc.json"))
            if not reports:
                continue
            report = json.load(open(reports[0]))
            kinds = Counter((v["type"], v.get("severity", "?")) for v in report.get("violations", []))
            unconnected = len(report.get("unconnected_items", []))
            pcb = next(reports[0].parent.glob("*.kicad_pcb"))
            text = pcb.read_text()
            vias = len(re.findall(r"^\s*\(via\b", text, re.M))
            widths = Counter(float(w) for w in re.findall(r"\((?:segment|arc)\b.*?\(width ([0-9.]+)\)", text, re.S))
            width = kinds.pop(("connection_width", "error"), 0), kinds.pop(("connection_width", "warning"), 0)
            errors = {f"{t}": n for (t, s), n in sorted(kinds.items()) if s == "error"}
            print(f"{run}\t{board.name}\tunconnected {unconnected}\tvias {vias}\tconnection_width error/warning {width[0]}/{width[1]}\terrors {errors}\tnarrowest {min(widths) if widths else '-'} ({widths[min(widths)] if widths else 0} tracks)")


if __name__ == "__main__":
    main()
