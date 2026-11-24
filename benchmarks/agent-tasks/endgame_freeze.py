#!/usr/bin/env python3
# Copyright (C) 2026 Florian Loitsch.
"""Freezes the endgame tier's inputs: the routed boards a nightly left with
a few KiCad-unconnected items, copied out of build/nightly (which the
nightly rotates) with the router settings their run used, and the tier file
that names them (benchmarks/agent-tasks/endgame.json; README.md, "The
endgame tier").

    benchmarks/agent-tasks/endgame_freeze.py build/endgame-tier/v1 \\
        --source sweep-1118=build/nightly/2026-11-18/sweep.log \\
        --source designer-1124=build/nightly/2026-11-24/designer.log \\
        --reserve designer-1118=build/nightly/2026-11-18/designer.log \\
        --designer-drc build/scratch/fund-endgame/designer-drc \\
        --write benchmarks/agent-tasks/endgame.json

A source is a run.py log beside its output directory (the log's stem). A
row joins the tier when it fails the strict rule with 1 to --max-open KiCad
unconnected items and is not excluded (EXCLUDED: the input's own opens).
Entries are named `<task>@<tag>`. `--source` rows are the tier's boards
(`--tier endgame` runs them); `--reserve` rows are frozen too but run only
when named with `--only`. With `--designer-drc` (a directory of
`drc-kicad-board` reports of the designers' own boards, `<task>.json`) each
entry counts the open items whose net the designer's own board also leaves
open (`input_like_items`)."""

import argparse
import hashlib
import json
import re
import shutil
import sys
from pathlib import Path

HERE = Path(__file__).resolve().parent
ROOT = HERE.parents[1]
sys.path.insert(0, str(HERE))
from compare import by_name, rows_of  # noqa: E402

# Rows whose remaining opens are the input's own (the designer's board
# leaves the same nets open, or the project's rules leave the pads no
# track): they cannot pass the strict rule whatever the endgame does.
EXCLUDED = {
    "bismarx-v1__Sumec-MiniSumo__SUMEC_MK_IV":
        "the project's own rules leave pads no track of their class can enter (handover, 'Not targets'); "
        "the designer's board leaves GND, SHLD and +12V open too",
    "MbFredys__PCB-Modular-Multi-Protocol-Hub__Hub":
        "the designer's own board leaves the same two GND zone pieces unconnected",
}
# Files of a routed project the endgame does not read (previews, the
# placement page): left out of the frozen copy.
SKIPPED = {"preview.svg", "preview.json", "preview-labels.json", "placement.html", "board-placer.json"}


def absolute(path):
    """`path` under the checkout when relative, links not followed (build/
    is a link into the main checkout in a worktree): the tier names paths
    relative to the checkout."""
    return path if path.is_absolute() else ROOT / path


def relative(path):
    return str(path.relative_to(ROOT)) if path.is_relative_to(ROOT) else str(path)


def sha256(path):
    digest = hashlib.sha256()
    with open(path, "rb") as stream:
        for block in iter(lambda: stream.read(1 << 20), b""):
            digest.update(block)
    return digest.hexdigest()


def item_nets(item):
    """The nets a KiCad unconnected item names (`[net]` in its items'
    descriptions)."""
    nets = set()
    for part in item.get("items", []):
        match = re.search(r"\[([^\]]*)\]", part.get("description", ""))
        if match:
            nets.add(match.group(1))
    return nets


def result_project(output, row):
    """The routed project of a row: a layout's `layout/result`, a route
    row's `route`."""
    work = output / row["name"]
    for candidate in (work / "layout" / "result", work / "route"):
        if (candidate / "drc.json").exists():
            return candidate
    return None


def board_id_of(project):
    boards = sorted(project.glob("*.kicad_pcb"))
    return boards[0].stem if len(boards) == 1 else None


def freeze(destination, tag, log, max_open, designer_drc, reserve, keep_excluded=False):
    """The entries of one source, their projects copied under
    `destination`."""
    output = log.with_suffix("")
    entries, excluded = {}, {}
    for name, row in sorted(by_name(rows_of(log)).items()):
        unconnected = (row.get("native") or {}).get("unconnected")
        if row.get("pass_strict") or not unconnected or unconnected > max_open:
            continue
        key = f"{name}@{tag}"
        if name in EXCLUDED and not (reserve and keep_excluded):
            excluded[key] = EXCLUDED[name]
            continue
        project = result_project(output, row)
        if project is None:
            excluded[key] = f"no routed project under {output / name}"
            continue
        target = destination / key
        if target.exists():
            shutil.rmtree(target)
        shutil.copytree(project, target / "project", ignore=lambda _, names: [n for n in names if n in SKIPPED])
        router = output / name / "router.json"
        settings = json.loads(router.read_text()) if router.exists() else {}
        (target / "router.json").write_text(json.dumps(settings, indent=1))
        (target / "row.json").write_text(json.dumps(row, indent=1))
        board_id = board_id_of(target / "project")
        drc = json.loads((target / "project" / "drc.json").read_text())
        items = drc.get("unconnected_items", [])
        entry = {
            "task": name,
            "source": relative(project),
            "project": relative(target / "project"),
            "board_id": board_id,
            "board_sha256": sha256(target / "project" / f"{board_id}.kicad_pcb"),
            "binary": Path(row.get("binary", "")).name,
            "mode": row.get("mode"),
            "placement": row.get("placement", "ours" if row.get("mode") == "layout" else None),
            "pour_mode": (row.get("repairs") or {}).get("pour_mode"),
            "unconnected_before": unconnected,
            "open_nets_before": sorted(set().union(*[item_nets(item) for item in items])) if items else [],
            "errors_before": (row.get("native") or {}).get("errors", {}),
            "new_findings_strict_before": row.get("new_findings_strict", {}),
            "work_seconds_before": row.get("work_seconds"),
            "seconds_before": row.get("seconds"),
            "cpu_seconds_before": row.get("cpu_seconds"),
            "peak_rss_mb": row.get("peak_rss_mb"),
        }
        if designer_drc is not None:
            report = designer_drc / f"{name}.json"
            if report.exists():
                theirs = json.loads(report.read_text()).get("unconnected_items", [])
                nets = set().union(*[item_nets(item) for item in theirs]) if theirs else set()
                entry["input_like_items"] = sum(1 for item in items if item_nets(item) & nets)
        if reserve:
            entry["reserve"] = True
        if name in EXCLUDED:
            entry["excluded"] = EXCLUDED[name]
        entries[key] = entry
    return entries, excluded


def main():
    parser = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    parser.add_argument("destination", type=Path, help="where the frozen projects go (outside build/nightly)")
    parser.add_argument("--source", action="append", default=[], metavar="TAG=LOG",
                        help="a run.py log whose near misses are the tier's boards")
    parser.add_argument("--reserve", action="append", default=[], metavar="TAG=LOG",
                        help="a run.py log whose near misses are frozen as reserve entries (run with --only)")
    parser.add_argument("--max-open", type=int, default=7)
    parser.add_argument("--keep-excluded-in-reserve", action="store_true",
                        help="freeze the excluded rows of --reserve sources too, marked with their reason "
                             "(a prediction check that needs them)")
    parser.add_argument("--designer-drc", type=Path, default=None)
    parser.add_argument("--budget", type=float, default=30.0,
                        help="the endgame's ladder budget in work seconds (the tier's endgame_budget_seconds)")
    parser.add_argument("--write", type=Path, default=None, help="the tier file to write")
    arguments = parser.parse_args()
    arguments.destination = absolute(arguments.destination)
    if arguments.designer_drc is not None:
        arguments.designer_drc = absolute(arguments.designer_drc)
    arguments.destination.mkdir(parents=True, exist_ok=True)
    boards, excluded, sources = {}, {}, {}
    for reserve, specs in ((False, arguments.source), (True, arguments.reserve)):
        for spec in specs:
            tag, _, log = spec.partition("=")
            log = absolute(Path(log))
            entries, left = freeze(arguments.destination, tag, log, arguments.max_open, arguments.designer_drc,
                                   reserve, arguments.keep_excluded_in_reserve)
            boards.update(entries)
            excluded.update(left)
            sources[tag] = {"log": relative(log), "reserve": reserve, "entries": len(entries)}
            print(f"{tag}: {len(entries)} entries, {len(left)} excluded", file=sys.stderr)
    tier = {
        "description": (
            "Endgame tier: routed boards a nightly left with 1 to "
            f"{arguments.max_open} KiCad-unconnected items, frozen under {relative(arguments.destination)} "
            "(endgame_freeze.py), resumed with their copper kept: route-kicad-board on the routed project "
            "(existing_tracks keep, the run's own router settings and pour mode, endgame_budget_seconds of "
            "ladder work), then repair-kicad-opens --local where KiCad still reports opens. It answers one "
            "question in minutes: does the endgame close more of the last connections on these boards "
            "(README.md, 'The endgame tier'). Entries are <task>@<source>; reserve entries run only when "
            "named with --only."),
        "kind": "endgame",
        "tasks": "benchmarks/agent-tasks/tasks/v7/tasks.json",
        "inputs": relative(arguments.destination),
        "endgame_budget_seconds": arguments.budget,
        "max_open": arguments.max_open,
        "sources": sources,
        "excluded": excluded,
        "boards": boards,
    }
    text = json.dumps(tier, indent=1) + "\n"
    if arguments.write:
        arguments.write.write_text(text)
    else:
        print(text)


if __name__ == "__main__":
    main()
