#!/usr/bin/env python3
# Copyright (C) 2026 Toit contributors.
# Use of this source code is governed by an MIT-style license that can be
# found in the LICENSE file.
"""Route-mode tasks of the boards that use blind, buried or micro vias
(docs/router.md, "Blind, buried and micro vias"), for
benchmarks/agent-tasks/run.py: the designer's placement, tracks stripped,
with the designer's findings as the baseline.

    tasks.py --boards DIR [--budget-seconds S] --out DIR

Each board directory under `--boards` (a copy of the harvest's: the
`.kicad_pcb` with its `.kicad_pro` and `.kicad_dru`) gives two task lists:
`off.json` (the router's settings unchanged) and `on.json` (`blind_vias`
on and `via_spans` the spans of the designer's own blind, buried and micro
vias, which the stripped board no longer shows the router).
"""

import argparse
import glob
import json
import os
import re
from collections import Counter

VIA = re.compile(r"\(via\s+(blind|buried|micro)\b(.*?)\(layers\s+\"([^\"]+)\"\s+\"([^\"]+)\"\)", re.S)


def designer_spans(path):
    """The spans of the board's vias of a kind short of through, as
    `via_spans` entries, the most used first."""
    text = open(path, encoding="utf-8", errors="replace").read()
    copper = [name for name in re.findall(r"\(\d+\s+\"([^\"]+\.Cu)\"\s+(?:signal|power|mixed|jumper)", text)]
    order = {name: index for index, name in enumerate(
        ["F.Cu"] + sorted((n for n in copper if n.startswith("In")), key=lambda n: int(n[2:-3])) + ["B.Cu"])}
    found = Counter()
    for kind, _, first, last in VIA.findall(text):
        if first not in order or last not in order:
            continue
        a, b = sorted((order[first], order[last]))
        if a == 0 and b == len(order) - 1:
            continue
        names = sorted((first, last), key=order.get)
        micro = kind == "micro" and b - a == 1
        found[f"{names[0]}-{names[1]}" + (":micro" if micro else "")] += 1
    return [span for span, _ in found.most_common()], len(order)


def main():
    parser = argparse.ArgumentParser(description=__doc__.splitlines()[0])
    parser.add_argument("--boards", required=True)
    parser.add_argument("--budget-seconds", type=float, default=None)
    parser.add_argument("--out", required=True)
    args = parser.parse_args()
    lists = {"off": [], "on": []}
    for directory in sorted(glob.glob(os.path.join(args.boards, "*"))):
        boards = glob.glob(os.path.join(directory, "*.kicad_pcb"))
        if len(boards) != 1:
            continue
        board_id = os.path.basename(boards[0])[: -len(".kicad_pcb")]
        spans, layers = designer_spans(boards[0])
        if not spans or layers < 3:
            continue
        base = {"name": os.path.basename(directory), "mode": "route", "directory": os.path.abspath(directory),
                "board_id": board_id, "designer_budget": True, "router": {}}
        if args.budget_seconds is not None:
            base["budget_seconds"] = args.budget_seconds
        lists["off"].append(base)
        lists["on"].append(dict(base, router={"blind_vias": True, "via_spans": spans}))
        print(base["name"], layers, spans)
    os.makedirs(args.out, exist_ok=True)
    for name, tasks in lists.items():
        with open(os.path.join(args.out, f"{name}.json"), "w") as handle:
            json.dump({"comment": __doc__.splitlines()[0], "tasks": tasks}, handle, indent=1)


if __name__ == "__main__":
    main()
