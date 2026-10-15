#!/usr/bin/env python3
# Copyright (C) 2026 Toit contributors.
"""Which corpus boards have a minimum connection width (board setup
min_connection) above a track width our router may choose.

Mirrors crates/pcb-kicad/src/project_rules.rs as of x952: class widths
(floored at min_track_width), the narrow-signal width (the smallest width
of the project's menu at or above min_track_width, used when narrower than
a class), and the neck width (min_track_width, at least 0.1; without one
the narrowest class, at most 0.127, at least 0.1).

    census.py build/github-tasks-v6/tasks.json [--markdown]
"""
import json
import sys
from pathlib import Path

DEFAULT_TRACK = 0.2


def widths(project):
    design = project.get("board", {}).get("design_settings", {})
    rules = design.get("rules", {})
    minimum = rules.get("min_track_width", 0.0) or 0.0
    classes = {}
    for cls in project.get("net_settings", {}).get("classes", []) or []:
        width = cls.get("track_width") or 0.0
        classes[cls.get("name", "?")] = width if width > 0 else DEFAULT_TRACK
    if "Default" not in classes:
        classes["Default"] = DEFAULT_TRACK  # the router takes the board's most common width here
    classes = {name: max(width, minimum) for name, width in classes.items()}
    narrowest = min(classes.values())
    menu = [w for w in design.get("track_widths", []) or [] if w and w > 0 and w >= minimum]
    narrow = min(menu) if menu and min(menu) < max(classes.values()) else None
    neck = max(minimum, 0.1) if minimum > 0 else max(min(narrowest, 0.127), 0.1)
    return classes, narrow, neck


def main():
    tasks = json.load(open(sys.argv[1]))["tasks"]
    markdown = "--markdown" in sys.argv
    rows = []
    for task in tasks:
        project = Path(task["directory"]) / f"{task['board_id']}.kicad_pro"
        if not project.exists():
            continue
        data = json.load(open(project))
        design = data.get("board", {}).get("design_settings", {})
        minimum = design.get("rules", {}).get("min_connection", 0.0) or 0.0
        severity = design.get("rule_severities", {}).get("connection_width", "(default warning)")
        if minimum <= 0:
            continue
        classes, narrow, neck = widths(data)
        below = sorted({f"{n} {w:g}" for n, w in classes.items() if w < minimum - 1e-9})
        flags = []
        if below:
            flags.append("classes " + ", ".join(below))
        if narrow is not None and narrow < minimum - 1e-9:
            flags.append(f"narrow {narrow:g}")
        if neck < minimum - 1e-9:
            flags.append(f"neck {neck:g}")
        rows.append((task["name"], minimum, severity, min(classes.values()), narrow, neck, "; ".join(flags) or "-"))
    if markdown:
        print("| board | min_connection | severity | narrowest class | narrow step | neck | below the minimum |")
        print("| --- | ---: | --- | ---: | ---: | ---: | --- |")
        for name, minimum, severity, cls, narrow, neck, flags in rows:
            print(f"| {name} | {minimum:g} | {severity} | {cls:g} | {narrow if narrow is not None else '-'} | {neck:g} | {flags} |")
    else:
        for row in rows:
            print(*row, sep="\t")
    print(f"{len(rows)} of {len(tasks)} tasks have a min_connection; {sum(r[-1] != '-' for r in rows)} have a router width below it", file=sys.stderr)


if __name__ == "__main__":
    main()
