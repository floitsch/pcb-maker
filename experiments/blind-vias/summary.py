#!/usr/bin/env python3
# Copyright (C) 2026 Toit contributors.
# Use of this source code is governed by an MIT-style license that can be
# found in the LICENSE file.
"""The rows of run.py logs side by side (docs/router.md, "Blind, buried and micro vias"): KiCad's unconnected items, the strict pass and
its new findings, the vias of the routed board by kind, the length and the
CPU seconds.

    summary.py <label>=<run.py output dir> ...

Reads `<dir>.log` (the rows) and each row's routed board
(`<dir>/<name>/route/<id>.kicad_pcb`, or the layout's `layout/`).
"""

import glob
import json
import os
import re
import sys
from collections import Counter


def vias_by_kind(board):
    if not board or not os.path.exists(board):
        return {}
    text = open(board, encoding="utf-8", errors="replace").read()
    kinds = Counter()
    for match in re.finditer(r"\(via(\s+(blind|buried|micro))?\s", text):
        kinds[match.group(2) or "through"] += 1
    return dict(kinds)


def rows_of(directory):
    rows = {}
    with open(directory.rstrip("/") + ".log") as handle:
        for line in handle:
            line = line.strip()
            if line.startswith("{"):
                row = json.loads(line)
                if "name" in row:
                    rows[row["name"]] = row
    return rows


def board_of(directory, name):
    for sub in ("route", "layout/result"):
        found = glob.glob(os.path.join(directory, name, sub, "*.kicad_pcb"))
        if found:
            return found[0]
    return None


def main():
    runs = [argument.split("=", 1) for argument in sys.argv[1:]]
    tables = {label: rows_of(directory) for label, directory in runs}
    names = sorted({name for rows in tables.values() for name in rows})
    print("| Board | " + " | ".join(label for label, _ in runs) + " |")
    print("| --- |" + " --- |" * len(runs))
    for name in names:
        cells = []
        for label, directory in runs:
            row = tables[label].get(name)
            if row is None:
                cells.append("-")
                continue
            native = row.get("native") or {}
            kinds = vias_by_kind(board_of(directory, name))
            spelled = ", ".join(f"{kind} {count}" for kind, count in sorted(kinds.items()))
            new = row.get("new_findings_strict") or {}
            cells.append(
                "%s / %s / %s / %s / %.0f mm / %.0f s"
                % (
                    native.get("unconnected", "?"),
                    "yes" if row.get("pass_strict") else "no",
                    sum(new.values()) if new else 0,
                    spelled or "0",
                    row.get("length_mm") or 0,
                    row.get("cpu_seconds") or 0,
                )
            )
        print(f"| {name} | " + " | ".join(cells) + " |")


if __name__ == "__main__":
    main()
