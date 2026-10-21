#!/usr/bin/env python3
# Copyright (C) 2026 Toit contributors.
"""Which boards' router rules the 2026-10-21 project-rules change can
move, without routing: per board, the board setup minimums the project
leaves to KiCad's defaults (those above the old reading's 0), the nets
whose class values differ between the old reading (first wildcard pattern,
a class's missing value at KiCad's built-in default) and the new one
(wildcard or regular expression, every match, values by priority, then
Default's), and whether a .kicad_dru is there.

    python3 -I experiments/project-rules/changes.py < boards.txt   (one .kicad_pcb path a line)
"""
import json
import re
import sys
from pathlib import Path

DEFAULTS = {"track_width": 0.2, "clearance": 0.2, "via_diameter": 0.6, "via_drill": 0.3}
# The minimums whose default moved from 0 (the old reading) to KiCad's.
NEW_DEFAULTS = {"min_track_width": 0.2, "min_via_annular_width": 0.1, "min_via_diameter": 0.5,
                "min_through_hole_diameter": 0.3}


def wildcard(pattern, text):
    regex = "".join(".*" if c == "*" else "." if c == "?" else re.escape(c) for c in pattern)
    return re.fullmatch(regex, text, re.S) is not None


def regex(pattern, text):
    try:
        return re.fullmatch(f"(?:{pattern})", text) is not None
    except re.error:
        return False


def pad_nets(text):
    """The board's net names (its net list, or the nets its items name)."""
    return sorted(set(re.findall(r'\(net (?:\d+ )?"((?:[^"\\]|\\.)*)"\)', text)) - {""})


def main():
    for line in sys.stdin:
        if not line.strip():
            continue
        board = Path(line.strip())
        project_path = board.with_suffix(".kicad_pro")
        project = json.loads(project_path.read_text()) if project_path.exists() else {}
        settings = project.get("net_settings") or {}
        classes = {c.get("name"): (i, c) for i, c in enumerate(settings.get("classes") or [])}
        default = classes.get("Default", (0, {}))[1]
        patterns = [(p.get("pattern") or "", p.get("netclass")) for p in settings.get("netclass_patterns") or []]
        assignments = settings.get("netclass_assignments") or {}
        listed = {net: c.get("name") for c in settings.get("classes") or [] for net in c.get("nets") or []}
        changed = []
        nets = pad_nets(board.read_text(errors="replace"))
        for net in nets:
            assigned = assignments.get(net)
            old = (assigned if isinstance(assigned, str) else (assigned or [None])[0]) \
                or next((c for p, c in patterns if wildcard(p, net)), None) or listed.get(net)
            old = old if old in classes else "Default"
            old_values = {k: (classes[old][1].get(k) or DEFAULTS[k]) if old != "Default" or "Default" in classes
                          else None for k in DEFAULTS}
            names = []
            for name in ([assigned] if isinstance(assigned, str) else (assigned or [])) + \
                    ([listed[net]] if net in listed else []) + \
                    [c for p, c in patterns if wildcard(p, net) or regex(p, net)]:
                if name in classes and name not in names:
                    names.append(name)
            ordered = sorted((classes[n] for n in names), key=lambda e: e[1].get("priority", e[0]))
            new_values = {k: next((c.get(k) for _, c in ordered if c.get(k)), None) or default.get(k) or DEFAULTS[k]
                          for k in DEFAULTS} if names else {k: default.get(k) or DEFAULTS[k] for k in DEFAULTS}
            if "Default" not in classes:
                continue
            if any(abs((old_values[k] or 0) - (new_values[k] or 0)) > 1e-9 for k in DEFAULTS):
                changed.append(f"{net}: {old} {old_values} -> {names} {new_values}")
        rules = (project.get("board") or {}).get("design_settings", {}).get("rules") or {}
        missing = [k for k in NEW_DEFAULTS if k not in rules]
        dru = board.with_suffix(".kicad_dru")
        dru_rules = len(re.findall(r"^\s*\(rule\b", dru.read_text(errors="replace"), re.M)) if dru.exists() else 0
        if changed or missing or dru_rules:
            print(f"{board}: nets changed {len(changed)}, minimums at KiCad's default {missing}, "
                  f".kicad_dru rules {dru_rules}")
            for line in changed[:6]:
                print("   ", line)


if __name__ == "__main__":
    main()
