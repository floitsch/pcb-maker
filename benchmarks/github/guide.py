#!/usr/bin/env python3
# Copyright (C) 2026 Toit contributors.
"""Guided copies of the harvested boards: each net routes at its designer's
width.

    benchmarks/github/guide.py [--only NAME...] [--workers 4]

Designers often leave the default class at one width and draw most tracks
narrower (ohdsp's DSP board: class 0.5 mm, 840 of 1992 segments at
0.135 mm). Routing every net at the class width is then much harder than
what the designer did. As PCBench's guide step does (benchmarks/pcbench/
guide.py, after PCBWorld's make_guide), every net gets the widest track its
designer used, narrowed to the next observed width while KiCad's DRC finds
that width breaking rules on the designer's own copper, written into a
copy of the project as one net class per (class, width).

Output: benchmarks/real/external/github-guided/<name>/ (the routed board
with the chosen widths and the guided project), and
benchmarks/github/boards-guided.json for the corpus runner."""

import argparse
import importlib.util
import re
import json
import shutil
import tempfile
from concurrent.futures import ProcessPoolExecutor
from pathlib import Path

HERE = Path(__file__).resolve().parent
ROOT = HERE.parents[1]
SOURCE = ROOT / "benchmarks/real/external/github"
TARGET = ROOT / "benchmarks/real/external/github-guided"
TRIALS = 5

_spec = importlib.util.spec_from_file_location("pcbench_guide", ROOT / "benchmarks/pcbench/guide.py")
guide = importlib.util.module_from_spec(_spec)
_spec.loader.exec_module(guide)


NET_NUMBER = re.compile(r'\(net\s+(\d+)\)')
NET_TABLE = re.compile(r'\n\s*\(net\s+(\d+)\s+"((?:[^"\\]|\\.)*)"\)')
STAMP = re.compile(r'\((?:uuid|tstamp)\s+"?([0-9a-fA-F-]+)"?\)')


def track_net(block, table):
    """The net of a track block by name: KiCad 8+ names it, KiCad 6 and 7
    give its number in the board's net table."""
    named = guide.NET.search(block)
    if named:
        return named.group(1)
    number = NET_NUMBER.search(block)
    return table.get(number.group(1)) if number else None


def track_widths(text, table):
    widths = {}
    for head in ("segment", "arc"):
        for start, end in guide.blocks(text, head):
            block = text[start:end]
            net = track_net(block, table)
            width = guide.WIDTH.search(block)
            if net and width:
                widths.setdefault(net, set()).add(round(float(width.group(1)), 4))
    return {net: sorted(values) for net, values in widths.items()}


def rewrite_widths(text, chosen, table):
    pieces, last = [], 0
    for start, end in sorted(guide.blocks(text, "segment") + guide.blocks(text, "arc")):
        block = text[start:end]
        net = track_net(block, table)
        if net in chosen:
            block = guide.WIDTH.sub(f"(width {chosen[net]:g})", block, count=1)
        pieces.append(text[last:start])
        pieces.append(block)
        last = end
    pieces.append(text[last:])
    return "".join(pieces)


def track_uuids(text, table):
    uuids = {}
    for head in ("segment", "arc"):
        for start, end in guide.blocks(text, head):
            block = text[start:end]
            net = track_net(block, table)
            stamp = STAMP.search(block)
            if net and stamp:
                uuids[stamp.group(1)] = net
    return uuids


def process(board):
    name, board_id = board["name"], board["board_id"]
    result = {"name": name, "status": "error"}
    try:
        source = SOURCE / name
        text = (source / f"{board_id}.kicad_pcb").read_text()
        project = json.loads((source / f"{board_id}.kicad_pro").read_text())
        table = dict(NET_TABLE.findall(text[:text.find("(footprint")] if "(footprint" in text else text))
        widths = track_widths(text, table)
        if not widths:
            result.update(status="skip", message="no routed tracks")
            return result
        chosen = {net: values[-1] for net, values in widths.items()}
        uuids = track_uuids(text, table)
        with tempfile.TemporaryDirectory() as scratch:
            trial_board = Path(scratch) / f"{board_id}.kicad_pcb"
            for trial in range(TRIALS):
                trial_board.write_text(rewrite_widths(text, chosen, table))
                trial_board.with_suffix(".kicad_pro").write_text(json.dumps(guide.guide_project(project, chosen), indent=2))
                report = guide.drc(trial_board)
                if report is None:
                    break
                result.update(trial=trial, errors=len(report.get("violations", [])))
                narrowed = False
                for net in guide.offenders(report, uuids):
                    values = widths[net]
                    position = values.index(chosen[net])
                    if position > 0:
                        chosen[net] = values[position - 1]
                        narrowed = True
                if not narrowed:
                    break
        target = TARGET / name
        if target.exists():
            shutil.rmtree(target)
        target.mkdir(parents=True)
        (target / f"{board_id}.kicad_pcb").write_text(rewrite_widths(text, chosen, table))
        (target / f"{board_id}.kicad_pro").write_text(json.dumps(guide.guide_project(project, chosen), indent=2))
        if (source / "origin.json").exists():
            shutil.copy(source / "origin.json", target / "origin.json")
        result.update(status="ok", nets=len(chosen),
                      narrowed=sum(1 for net, values in widths.items() if chosen[net] != values[-1]))
    except Exception as error:  # noqa: BLE001 - reported per board
        result.update(status="error", message=repr(error)[:300])
    return result


def main():
    parser = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    parser.add_argument("--only", nargs="*")
    parser.add_argument("--workers", type=int, default=4)
    arguments = parser.parse_args()
    boards = json.loads((HERE / "boards.json").read_text())["boards"]
    if arguments.only:
        boards = [board for board in boards if board["name"] in arguments.only]
    with ProcessPoolExecutor(arguments.workers) as pool:
        results = list(pool.map(process, boards))
    for result in results:
        print(json.dumps(result))
    done = {result["name"] for result in results if result["status"] == "ok"}
    all_boards = json.loads((HERE / "boards.json").read_text())["boards"]
    guided = [dict(board, directory=f"benchmarks/real/external/github-guided/{board['name']}")
              for board in all_boards if board["name"] in done or (TARGET / board["name"]).exists()]
    (HERE / "boards-guided.json").write_text(json.dumps({
        "comment": "The harvested boards with every net at its designer's width (guide.py).",
        "boards": guided}, indent=1))
    print(f"{len(done)} boards guided", flush=True)


if __name__ == "__main__":
    main()
