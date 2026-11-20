#!/usr/bin/env python3
# Copyright (C) 2026 Florian Loitsch.
"""Replay the archived GigaShield starter and Quilter return under KiCad DRC."""

import argparse
import json
import os
from pathlib import Path
import subprocess
import sys

import comparison_scoring as common

HERE = Path(__file__).resolve().parent
MANIFEST = HERE / "gigashield-layout.json"
REPORTED_POWER_NETS = ("+12V", "GND")


def source_brief(pcbnew, board):
    outline = board.GetBoardEdgesBoundingBox()
    anchors = [part for part in board.GetFootprints() if outline.Contains(part.GetBoundingBox(False, False))]
    return {"version": 1, "move_all": True, "front": ["*"], "place": [
        {"part": part.GetReference(), "origin": "absolute", "x": pcbnew.ToMM(part.GetPosition().x),
         "y": pcbnew.ToMM(part.GetPosition().y),
         "angle": (part.GetOrientationDegrees() + 180) % 360 - 180}
        for part in sorted(anchors, key=lambda item: item.GetReference())
    ]}


def zone_override(change):
    """Keep changed pad-to-pour policy distinct from physical and pin identity."""
    if "pad" not in change or not change["source"] or not change["candidate"]:
        return False

    def without_zone(items):
        return sorted(json.dumps({**item, "geometry": {key: value for key, value in item["geometry"].items()
                                                      if key != "zone_connect"}}, sort_keys=True)
                      for item in items)

    return without_zone(change["source"]) == without_zone(change["candidate"])


def measure(work, dataset, binary, existing, timeout, pcbnew):
    common.require(not work.is_relative_to(dataset), "use a fresh work directory outside the dataset")
    manifest = json.loads(MANIFEST.read_text())
    expected = manifest["expected"]
    board_id = manifest["board_id"]
    pcb, pro, sch = [f"{board_id}.kicad_{suffix}" for suffix in ("pcb", "pro", "sch")]
    inputs = {name: {filename: common.pinned(dataset / pin["path"], pin) for filename, pin in manifest[name].items()}
              for name in ("starter", "candidate")}
    source, candidate = inputs["starter"], inputs["candidate"]
    settings = {"original": source[pro], "returned": candidate[pro]}
    rules = {name: json.loads(data)["board"]["design_settings"]["rules"] for name, data in settings.items()}
    common.require(all(rule["min_clearance"] == expected["minimum_clearance_mm"] for rule in rules.values()),
                   "the archived minimum clearances differ")
    work.mkdir(parents=True)
    starter = common.write_project(work / "input", board_id, source[sch], source[pcb], source[pro])
    board = pcbnew.LoadBoard(str(starter))
    minimums = {name: rule["min_track_width"] for name, rule in rules.items()}
    signature, stats = common.inspect_board(pcbnew, starter, minimums, board=board)
    common.require(stats["copper_layers"] == manifest["copper_layers"]
                   and all(stats[key] == expected[key] for key in ("footprints", "pads", "nets"))
                   and stats["track_segments_and_arcs"] == stats["vias"] == 0
                   and stats["footprint_bounds_inside_outline"] == expected["preplaced"]
                   and stats["footprints_clear_of_outline_bounds"] == expected["footprints"] - expected["preplaced"],
                   "unexpected parked-board geometry")
    brief = source_brief(pcbnew, board)
    canonical = {"total_seconds": 1500, "placer": {"constraints": "constraints.json"}}
    dump = common.harness_common.atomic_json
    dump(starter.parent / "constraints.json", brief, newline=True)
    dump(starter.parent / "layout.json", canonical, newline=True)
    kicad = os.environ.get("PCB_KICAD_CLI") or "kicad-cli"
    env = {key: value for key, value in os.environ.items() if not key.startswith("PCB_")}
    env.update({"LC_ALL": "C", "PCB_KICAD_CLI": kicad, "PCB_WORK_RATE": common.WORK_RATE,
                "RAYON_NUM_THREADS": "4", "PCB_MAKER_CACHE_DIR": str(work / "cache"),
                "PCB_MAKER_LEDGER": os.environ.get("PCB_MAKER_LEDGER") or str(work / "ledger.json"),
                "XDG_CONFIG_HOME": str(work / "xdg-config"), "KICAD_CONFIG_HOME": str(work / "kicad-config")})
    version = common.run_command([kicad, "version"], work / "kicad-version.log", env, timeout).stdout.strip()
    common.require(version.startswith("10.") and pcbnew.Version() == version, "need matching KiCad 10 CLI and pcbnew")
    series = work / "kicad-config" / ".".join(version.split(".")[:2])
    series.mkdir(parents=True, exist_ok=True)
    for table in ("fp", "sym"):
        (series / f"{table}-lib-table").write_text(f"({table}_lib_table (version 7))\n")
    result = {"complete": False, "manifest": manifest, "manifest_sha256": common.digest(MANIFEST.read_bytes()),
              "kicad_version": version, "constraints": brief, "layout_config": canonical,
              "work_rate": common.WORK_RATE, "threads": 4, "design_rules": rules, "boards": {}}
    report_path = work / "results.json"
    result["layout"] = {"path": str(existing or work / "layout"), "reused": existing is not None,
                        "binary": str(binary), "binary_sha256": common.harness_common.file_sha256(binary),
                        "binary_invoked_by_this_runner": existing is None}
    dump(report_path, result, newline=True)
    layout = existing or work / "layout"
    try:
        if existing is None:
            process = common.run_command([binary, "layout-kicad-board", starter.parent, board_id, layout,
                                          "auto", starter.parent / "layout.json"], work / "layout.log", env, timeout)
            result["layout"]["exit_code"] = process.returncode
        verdict = json.loads((layout / "verdict.json").read_text())
        placed = json.loads((layout / "placed/board-placer.json").read_text())
        common.require(json.loads((layout / "placed/constraints.json").read_text()) == brief
                       and json.loads((layout / "placed/layout.json").read_text()) == canonical
                       and (layout / "result" / pro).read_bytes() == source[pro]
                       and (layout / "result" / sch).read_bytes() == source[sch]
                       and (verdict["work_rate"], verdict["placer_work_rate"]) == (3_150_000, 150_000_000),
                       "existing layout used another brief, work rate or project")
        result["layout"].update({"verdict": verdict, "constraints_held": sum(row["satisfied"] for row in placed["constraints"]),
                                 "constraints_total": len(placed["constraints"]), "unplaced": placed["unplaced"],
                                 "illegal": placed["illegal"], "movable": placed["movable"]})
        result_board = (layout / "result" / pcb).read_bytes()
    except (OSError, ValueError, RuntimeError, KeyError, subprocess.TimeoutExpired) as error:
        result["layout"]["error"] = str(error)
        result_board = None

    models = common.normalized_footprints(pcbnew, board, electrical=True)
    baseline_poses = common.poses(board)
    anchored = [row["part"] for row in brief["place"]]
    boards = {"starter": source[pcb], "quilter": candidate[pcb]}
    if result_board is not None:
        boards["pcb-maker"] = result_board
    for policy, project in settings.items():
        reports = result["boards"][policy] = {}
        baseline_parity = None
        for label, data in boards.items():
            directory = work / "scores" / policy / label
            row = reports[label] = {"board_sha256": common.digest(data), "project_sha256": common.digest(project),
                                    "path": str(directory.relative_to(work)), "matched": False}
            try:
                actual, parts, scored, geometry, native, parity = common.measure_board(
                    pcbnew, directory, board_id, source[sch], data, project, minimums, kicad, env, timeout, electrical=True, power_nets=REPORTED_POWER_NETS)
                poses = common.poses(scored)
                moved = [ref for ref in anchored if poses.get(ref) != baseline_poses[ref]]
                differences = common.footprint_differences(models, parts)
                overrides = [change for change in differences if zone_override(change)] if label == "pcb-maker" else []
                mismatches = [change for change in differences if change not in overrides]
                if label == "starter":
                    baseline_parity = parity
                row.update({"matched": actual == signature and not mismatches and not moved
                            and (label == "starter" or (not geometry["back_footprints"]
                                 and geometry["footprint_bounds_inside_outline"] == expected["footprints"])),
                            "geometry": geometry, "native": native, "archived_anchors_changed": moved,
                            "footprint_or_pin_mismatches": mismatches, "pcb_maker_pad_zone_overrides": overrides,
                            "observed_parity_matches_starter": parity == baseline_parity,
                            "capped_parity_types": {mode: sorted(kind for kind, count in score["parity_by_type"].items()
                                                                 if count >= common.DRC_CAP) for mode, score in native.items()}})
            except (OSError, ValueError, RuntimeError, KeyError, subprocess.TimeoutExpired) as error:
                row["error"] = str(error)
            dump(report_path, result, newline=True)
    result["complete"] = result_board is not None and all(row["matched"] for group in result["boards"].values()
                                                          for row in group.values())
    dump(report_path, result, newline=True)
    return result["complete"]


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("work", type=Path, help="fresh output directory")
    parser.add_argument("--dataset", required=True, type=Path, help="GigaShield checkout at the pinned commit")
    parser.add_argument("--binary", required=True, type=Path, help="pcb-maker executable used for the layout")
    parser.add_argument("--existing-layout", type=Path, help="rescore a run from the same brief and work rates")
    parser.add_argument("--timeout", type=int, default=1800, help="wall timeout per command in seconds")
    args = parser.parse_args()
    try:
        import pcbnew
        existing = args.existing_layout.resolve() if args.existing_layout else None
        complete = measure(args.work.resolve(), args.dataset.resolve(), args.binary.resolve(), existing, args.timeout, pcbnew)
    except (ImportError, OSError, ValueError, RuntimeError) as error:
        parser.exit(2, f"{error}\n")
    print(f"{'complete' if complete else 'incomplete'} comparison. See {args.work / 'results.json'}.")
    return 0 if complete else 1


if __name__ == "__main__":
    sys.exit(main())
