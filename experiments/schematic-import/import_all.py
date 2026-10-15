#!/usr/bin/env python3
# Copyright (C) 2026 Toit contributors.
"""Imports every harvested schematic (benchmarks/github/schematics.json,
status ok) the way the schematic tier does, without the layout, and counts
what the import got: imported (a report), complete (no footprint missing),
missing footprints, footprints taken from the board, drifted footprints,
and the designer's parts with nets that the schematic lacks (added with
`add` when --footprints-from is on, else lost).

    experiments/schematic-import/import_all.py <out-dir> --binary build/bin/pcb-maker-xNNN
        [--footprints-from] [--jobs 6] [--only NAME ...]

Per board: copy the fetched project, import-kicad-netlist (with the
designer's board as --footprints-from when asked), add the libraries
harvest_schematics.py found elsewhere and import again when footprints are
missing (benchmarks/agent-tasks/schematic.py), then, with --footprints-from,
write the `add` entries for the board-only parts with nets and import again.
Writes <out-dir>/rows.json and prints a line per board and the counts."""

import argparse
import json
import re
import shutil
import sys
from concurrent.futures import ThreadPoolExecutor
from pathlib import Path

ROOT = Path(__file__).resolve().parents[2]
sys.path.insert(0, str(ROOT / "benchmarks/agent-tasks"))
import schematic  # noqa: E402

BOARDS = ROOT / "benchmarks/real/external/github"


def copper_layers(text):
    layers = text[text.find("(layers"):]
    layers = layers[:schematic.block_end(layers, 0)]
    return len(re.findall(r'\(\d+\s+"?(?:F|B|In\d+)\.Cu"?', layers))


def one(record, arguments):
    name, board_id = record["name"], record["board_id"]
    work = arguments.output / name
    if work.exists():
        shutil.rmtree(work)
    work.mkdir(parents=True)
    designer_file = BOARDS / name / f"{board_id}.kicad_pcb"
    designer_text = designer_file.read_text()
    row = {"name": name}
    report, steps, error = schematic.import_schematic(
        arguments.binary, record, work, board_id, copper_layers(designer_text),
        designer_file if arguments.footprints_from else None)
    row["exit"] = steps.get("exit")
    row["libraries_added"] = len(steps.get("libraries_added", []))
    if error:
        row["error"] = error
        return row
    code, stdout, stderr = schematic.run([arguments.binary, "describe-kicad-board", BOARDS / name, board_id])
    description = json.loads(stdout) if code == 0 else None
    board = work / "project" / f"{board_id}.kicad_pcb"
    if description is not None:
        _, facts, _ = schematic.constraints_for_import({}, description, designer_text, board.read_text())
        entries, dropped = schematic.additions(designer_text, board.read_text(), facts["board_only"])
        row["netted_board_only"] = len(entries) + len(dropped)
        if arguments.footprints_from and entries:
            (work / "project" / "constraints.json").write_text(json.dumps({"version": 1, "add": entries}, indent=1))
            again, error = schematic.import_again(work, steps)
            row["exit"] = steps["exit"]
            if error:
                row["error"] = "with add: " + error
            else:
                report = again
    row.update(missing=len(report["missing"]), missing_parts=report["missing"][:5],
               substituted=len(report["substituted"]), unmatched_pins=len(report["unmatched_pins"]),
               footprints=report["footprints"],
               from_board=len(report.get("footprints_from_board") or []),
               board_versions=report.get("board_versions") or 0,
               drifted=len(report.get("drifted_footprints") or []),
               drifted_parts=sum(int(re.search(r"\((\d+) parts?:", line).group(1))
                                 for line in report.get("drifted_footprints") or []),
               drifted_footprints=(report.get("drifted_footprints") or [])[:6],
               added=len(report.get("added") or []))
    return row


def main():
    parser = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    parser.add_argument("output", type=Path)
    parser.add_argument("--binary", type=Path, required=True)
    parser.add_argument("--footprints-from", action="store_true")
    parser.add_argument("--jobs", type=int, default=6)
    parser.add_argument("--only", nargs="*")
    arguments = parser.parse_args()
    arguments.binary = arguments.binary.resolve()
    arguments.output = arguments.output.resolve()
    records = [record for record in json.loads((ROOT / "benchmarks/github/schematics.json").read_text())["boards"].values()
               if record.get("status") == "ok" and (not arguments.only or record["name"] in arguments.only)]
    with ThreadPoolExecutor(arguments.jobs) as pool:
        rows = list(pool.map(lambda record: one(record, arguments), records))
    for row in rows:
        print(f"{row['name'][:60]:60} exit {row.get('exit')} missing {row.get('missing', '-'):>4} "
              f"from_board {row.get('from_board', 0):>4} drifted {row.get('drifted', 0):>3} ({row.get('drifted_parts', 0)} parts) "
              f"netted board-only {row.get('netted_board_only', '-')} added {row.get('added', 0)}"
              + (f" error {row['error']}" if row.get("error") else ""))
    imported = [row for row in rows if "missing" in row]
    summary = {
        "boards": len(rows),
        "imported": len(imported),
        "complete": sum(1 for row in imported if row["missing"] == 0),
        "missing_footprints": sum(row["missing"] for row in imported),
        "boards_missing": sum(1 for row in imported if row["missing"]),
        "from_board": sum(row["from_board"] for row in imported),
        "board_versions": sum(row["board_versions"] for row in imported),
        "boards_drifted": sum(1 for row in imported if row["drifted"]),
        "drifted_footprints": sum(row["drifted"] for row in imported),
        "drifted_parts": sum(row["drifted_parts"] for row in imported),
        "netted_board_only": sum(row.get("netted_board_only", 0) for row in rows),
        "boards_with_netted_board_only": sum(1 for row in rows if row.get("netted_board_only")),
        "added": sum(row.get("added", 0) for row in imported),
    }
    print(json.dumps(summary))
    (arguments.output / "rows.json").write_text(json.dumps({"summary": summary, "rows": rows}, indent=1))


if __name__ == "__main__":
    main()
