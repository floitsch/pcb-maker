#!/usr/bin/env python3
# Copyright (C) 2026 Toit contributors.
# Use of this source code is governed by an MIT-style license that can be
# found in the LICENSE file.
"""Which boards allow or use blind, buried and micro vias (docs/router.md,
"Blind, buried and micro vias (2026-11-04)").

    census.py [--root benchmarks/real/external/github] [--bgas build/bga1/census/bgas.json]
              [--out DIR]

For every board directory under the root (one `.kicad_pcb`, its
`.kicad_pro` and `.kicad_dru` beside it), with KiCad's pcbnew: the copper
layer count, the file version, the project's `rules.allow_blind_buried_vias`
and `rules.allow_microvias` (KiCad 6 and 7 wrote them; KiCad 8 dropped the
switches, so a later project has none: every via kind is always allowed
there), the custom rules that name a via type, and the designer's vias by
kind and by layer span. With `--bgas` (experiments/bga-fanout/census.py
find), the grid arrays on each board. Writes `DIR/census.json` and
`DIR/census.md`; prints the table.
"""

import argparse
import glob
import json
import os
import re
import sys
from collections import Counter, defaultdict


def kind_name(pcbnew, kind):
    return {
        pcbnew.VIATYPE_THROUGH: "through",
        pcbnew.VIATYPE_BLIND: "blind",
        pcbnew.VIATYPE_BURIED: "buried",
        pcbnew.VIATYPE_MICROVIA: "micro",
    }.get(kind, "other")


def project_flags(pro_path):
    if not pro_path or not os.path.exists(pro_path):
        return None, None
    with open(pro_path, encoding="utf-8", errors="replace") as handle:
        try:
            project = json.load(handle)
        except json.JSONDecodeError:
            return None, None
    rules = project.get("board", {}).get("design_settings", {}).get("rules", {})
    return rules.get("allow_blind_buried_vias"), rules.get("allow_microvias")


def rule_mentions(dru_path):
    """The custom rules' lines that name a via type or a micro via size."""
    if not dru_path or not os.path.exists(dru_path):
        return []
    with open(dru_path, encoding="utf-8", errors="replace") as handle:
        text = handle.read()
    pattern = re.compile(r"Via_Type|[Bb]lind|[Bb]uried|[Mm]icro|uvia", re.UNICODE)
    return [line.strip() for line in text.splitlines() if pattern.search(line)]


def board_census(pcbnew, directory, path=None):
    if path is None:
        boards = sorted(glob.glob(os.path.join(directory, "*.kicad_pcb")))
        if not boards:
            return None
        path = boards[0]
    stem = path[: -len(".kicad_pcb")]
    with open(path, encoding="utf-8", errors="replace") as handle:
        head = handle.read(200)
    version = re.search(r"\(version (\d+)\)", head)
    board = pcbnew.LoadBoard(path)
    if board is None:
        print("unreadable: %s" % path, file=sys.stderr)
        return None
    layers = board.GetCopperLayerCount()
    kinds = Counter()
    spans = Counter()
    for track in board.GetTracks():
        if track.GetClass() != "PCB_VIA":
            continue
        kind = kind_name(pcbnew, track.GetViaType())
        kinds[kind] += 1
        if kind != "through":
            span = "%s-%s" % (
                board.GetLayerName(track.TopLayer()),
                board.GetLayerName(track.BottomLayer()),
            )
            spans[span] += 1
    allow_bb, allow_micro = project_flags(stem + ".kicad_pro")
    return {
        "task": os.path.basename(directory),
        "path": path,
        "version": int(version.group(1)) if version else None,
        "layers": layers,
        "allow_blind_buried_vias": allow_bb,
        "allow_microvias": allow_micro,
        "micro_via_min": [
            board.GetDesignSettings().m_MicroViasMinSize / 1e6,
            board.GetDesignSettings().m_MicroViasMinDrill / 1e6,
        ],
        "rules": rule_mentions(stem + ".kicad_dru"),
        "vias": dict(kinds),
        "spans": dict(spans),
    }


def flag(value):
    return "-" if value is None else ("yes" if value else "no")


def table(rows, arrays):
    lines = [
        "| Board | Layers | KiCad file | allow blind/buried | allow micro | rules naming a via type | vias (through / blind / buried / micro) | spans | grid arrays (pitch, grid) |",
        "| --- | --- | --- | --- | --- | --- | --- | --- | --- |",
    ]
    for row in rows:
        vias = row["vias"]
        spans = ", ".join("%s %d" % item for item in sorted(row["spans"].items())) or "-"
        bga = ", ".join(
            "%s %.2g mm %dx%d" % (a["reference"], a["pitch_mm"], a["rows"], a["columns"])
            for a in arrays.get(row["task"], [])
        ) or "-"
        lines.append(
            "| %s | %d | %s | %s | %s | %d | %d / %d / %d / %d | %s | %s |"
            % (
                row["task"],
                row["layers"],
                row["version"],
                flag(row["allow_blind_buried_vias"]),
                flag(row["allow_microvias"]),
                len(row["rules"]),
                vias.get("through", 0),
                vias.get("blind", 0),
                vias.get("buried", 0),
                vias.get("micro", 0),
                spans,
                bga,
            )
        )
    return "\n".join(lines)


def main():
    parser = argparse.ArgumentParser(description=__doc__.splitlines()[0])
    parser.add_argument("--root", default="benchmarks/real/external/github")
    parser.add_argument("--bgas", default="build/bga1/census/bgas.json")
    parser.add_argument("--out", default=None)
    parser.add_argument(
        "--list",
        default=None,
        help="a file of board paths, one a line, read instead of the root's directories",
    )
    args = parser.parse_args()
    import pcbnew

    arrays = defaultdict(list)
    if args.bgas and os.path.exists(args.bgas):
        with open(args.bgas) as handle:
            for array in json.load(handle):
                arrays[array["task"]].append(array)
    rows = []
    if args.list:
        with open(args.list) as handle:
            entries = [(None, line.strip()) for line in handle if line.strip()]
    else:
        entries = [
            (directory, None)
            for directory in sorted(glob.glob(os.path.join(args.root, "*")))
            if os.path.isdir(directory)
        ]
    for directory, path in entries:
        if path is not None:
            directory = os.path.dirname(path)
        row = board_census(pcbnew, directory, path)
        if row is not None:
            rows.append(row)
            print(row["task"], row["layers"], row["vias"], file=sys.stderr)
    text = table(rows, arrays)
    if args.out:
        os.makedirs(args.out, exist_ok=True)
        with open(os.path.join(args.out, "census.json"), "w") as handle:
            json.dump(rows, handle, indent=1)
        with open(os.path.join(args.out, "census.md"), "w") as handle:
            handle.write(text + "\n")
    print(text)


if __name__ == "__main__":
    main()
