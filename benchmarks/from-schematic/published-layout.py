#!/usr/bin/env python3
# Copyright (C) 2026 Florian Loitsch.
"""Reproduce the published June 10 powder-doser layout comparison locally."""

import argparse
import io
import json
import os
import subprocess
import sys
import zipfile
from pathlib import Path

HERE = Path(__file__).resolve().parent
from comparison_scoring import (DRC_ARGS, DRC_CAP, WORK_RATE, digest, harness_common, inspect_board,
                                require, run_command, score, write_project)

MANIFEST = HERE / "published-layout.json"
LAYERS = ["F.Cu", "B.Cu"]
PARTS, PADS, NETS = 14, 66, 20
THREADS = "2"
CONSTRAINTS = {"version": 1, "move_all": True}
LAYOUT = {"total_seconds": 240, "placer": {"constraints": "constraints.json"}}


def verified_inputs(manifest, source_dir, archives):
    """Read and pin bytes before staging. ZIP member names never become paths."""
    source = {}
    for name, evidence in manifest["source_files"].items():
        path = source_dir / name
        data = path.read_bytes()
        require(digest(data) == evidence["sha256"], f"source checksum mismatch: {path}")
        source[name] = data

    pins = {entry["sha256"]: entry["id"] for entry in manifest["candidates"]}
    files = (f'{manifest["board_id"]}.kicad_pcb', f'{manifest["board_id"]}.kicad_pro')
    candidates, archive_paths = {}, {}
    require(len(archives) == len(pins), f"supply all {len(pins)} distinct published archives")
    for path in archives:
        data = path.read_bytes()
        candidate = pins.get(digest(data))
        require(candidate is not None, f"archive is outside the pinned June 10 set: {path}")
        require(candidate not in candidates, f"duplicate archive for {candidate}: {path}")
        with zipfile.ZipFile(io.BytesIO(data)) as archive:
            require(sorted(archive.namelist()) == sorted(files), f"unexpected archive contents: {path}")
            board, settings = (archive.read(name) for name in files)
        require(digest(settings) == manifest["returned_project_sha256"], f"returned settings differ: {path}")
        candidates[candidate] = (board, settings)
        archive_paths[candidate] = str(path)
    return source, dict(sorted(candidates.items())), archive_paths


def run_comparison(work, binary, source_dir, archives, pcbnew, timeout):
    require(not work.is_relative_to(source_dir), "place the fresh work directory outside the source directory")
    manifest = json.loads(MANIFEST.read_text())
    source, candidates, archive_paths = verified_inputs(manifest, source_dir, archives)
    binary_hash = harness_common.file_sha256(binary)
    board_id = manifest["board_id"]
    base_board = source[f"{board_id}.kicad_pcb"]
    schematic = source[f"{board_id}.kicad_sch"]
    projects = {"original": source[f"{board_id}.kicad_pro"], "returned": next(iter(candidates.values()))[1]}
    design_rules = {name: json.loads(settings)["board"]["design_settings"]["rules"] for name, settings in projects.items()}
    min_tracks = {name: rules["min_track_width"] for name, rules in design_rules.items()}
    work.mkdir(parents=True)
    # Keep the normal defaults and pinned work rate independent of local
    # tuning overrides. Build and score every board in the fresh work tree.
    env = {key: value for key, value in os.environ.items() if not key.startswith("PCB_")}
    kicad = os.environ.get("PCB_KICAD_CLI") or "kicad-cli"
    env.update({"LC_ALL": "C", "PCB_KICAD_CLI": kicad, "PCB_WORK_RATE": WORK_RATE, "RAYON_NUM_THREADS": THREADS,
                "PCB_MAKER_LEDGER": os.environ.get("PCB_MAKER_LEDGER") or str(work / "ledger.json"),
                "PCB_MAKER_CACHE_DIR": str(work / "cache"), "XDG_CONFIG_HOME": str(work / "xdg-config"),
                "KICAD_CONFIG_HOME": str(work / "kicad-config")})
    version_result = run_command([kicad, "version"], work / "kicad-version.log", env, timeout)
    version = version_result.stdout.strip()
    require(version_result.returncode == 0 and version.split(".", 1)[0] == "10" and version == pcbnew.Version(),
            "the published comparison needs matching KiCad 10 CLI and pcbnew versions")
    series = work / "kicad-config" / ".".join(version.split(".")[:2])
    series.mkdir(parents=True, exist_ok=True)
    for table in ("fp", "sym"):
        (series / f"{table}-lib-table").write_text(f"({table}_lib_table (version 7))\n")
    result = {
        "manifest": manifest, "manifest_sha256": harness_common.file_sha256(MANIFEST),
        "source_dir": str(source_dir), "archives": archive_paths, "binary": {"path": str(binary), "sha256": binary_hash},
        "kicad_version": version, "drc_arguments": DRC_ARGS, "drc_report_cap_per_type": DRC_CAP,
        "constraints": CONSTRAINTS, "layout": LAYOUT, "work_rate": WORK_RATE, "threads": int(THREADS), "settings": {},
    }
    for rules, settings in projects.items():
        scenario = work / rules
        row = {"project_sha256": digest(settings), "project_minimum_track_width_mm": min_tracks[rules],
               "project_minimum_clearance_mm": design_rules[rules]["min_clearance"], "boards": {}}
        result["settings"][rules] = row
        input_project = scenario / "layout-input"
        starter = write_project(input_project, board_id, schematic, base_board, settings)
        source_signature, starter_stats = inspect_board(pcbnew, starter, min_tracks)
        require(starter_stats["copper_layers"] == LAYERS and (starter_stats["footprints"], starter_stats["pads"],
                starter_stats["nets"]) == (PARTS, PADS, NETS), "starter has changed proxies, nets or signal layers")
        require(all(starter_stats[key] == 0 for key in ("track_segments_and_arcs", "vias", "zones", "locked_footprints"))
                and starter_stats["footprints_clear_of_outline_bounds"] == PARTS, "starter must be unrouted and unplaced")
        harness_common.atomic_json(input_project / "constraints.json", CONSTRAINTS, newline=True)
        config = input_project / "layout.json"
        harness_common.atomic_json(config, LAYOUT, newline=True)
        layout_dir = scenario / "pcb-maker-layout"
        try:
            placement = run_command([binary, "layout-kicad-board", input_project, board_id, layout_dir, "auto", config],
                                    scenario / "layout.log", env, timeout)
            row["layout_exit_code"] = placement.returncode
        except subprocess.TimeoutExpired:
            row["layout_error"] = f"layout timed out after {timeout} s"
        placed = harness_common.read_json(layout_dir / "placed/board-placer.json")
        row["layout_movable"] = placed.get("movable") if isinstance(placed, dict) else None
        own_board = layout_dir / "result" / starter.name
        boards = {"starter": base_board, **{key: value[0] for key, value in candidates.items()}}
        if own_board.is_file():
            boards["pcb-maker"] = own_board.read_bytes()
        else:
            row["boards"]["pcb-maker"] = {"status": "unverified", "error": "layout did not produce a result PCB"}
        starter_parity = None
        for label, pcb in boards.items():
            directory = scenario / "scores" / label
            report = {"status": "unverified", "board_sha256": digest(pcb), "score_path": str(directory.relative_to(work))}
            row["boards"][label] = report
            try:
                path = write_project(directory, board_id, schematic, pcb, settings)
                signature, stats = inspect_board(pcbnew, path, min_tracks)
                checks = {key: value == source_signature[key] for key, value in signature.items()}
                if label != "starter":
                    checks["placed_inside_outline"] = (stats["footprint_origins_inside_outline"] == PARTS
                                                        and stats["footprint_bounds_inside_outline"] == PARTS)
                counts, parity = score(path, pcb, board_id, settings, kicad, env, directory, timeout)
                if label == "starter":
                    starter_parity = parity
                require(starter_parity is not None, "no verified starter parity report")
                report.update({"status": "measured" if all(checks.values()) else "unmatched", "geometry": stats,
                               "matches_starter": checks, "parity_matches_starter": parity == starter_parity, **counts})
            except (ValueError, OSError, TypeError, RuntimeError, KeyError, subprocess.TimeoutExpired) as error:
                report["error"] = str(error)
        harness_common.atomic_json(work / "results.json", result, newline=True)
    result["complete"] = all(row.get("layout_exit_code") in (0, 1, 3) and row["layout_movable"] == PARTS
                             and all(board["status"] == "measured" for board in row["boards"].values())
                             for row in result["settings"].values())
    harness_common.atomic_json(work / "results.json", result, newline=True)
    return result["complete"]


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("work", type=Path, help="fresh output directory")
    parser.add_argument("--source-dir", required=True, type=Path, help="directory with the three SHA-pinned source files")
    parser.add_argument("--candidates", required=True, nargs=6, type=Path, help="all six June 10 published ZIP files")
    parser.add_argument("--binary", required=True, type=Path, help="pcb-maker CLI executable")
    parser.add_argument("--timeout", type=int, default=900, help="wall timeout per command in seconds")
    args = parser.parse_args()
    try:
        import pcbnew
        complete = run_comparison(args.work.resolve(), args.binary.resolve(), args.source_dir.resolve(),
                                  [path.resolve() for path in args.candidates], pcbnew, args.timeout)
    except (ImportError, ValueError, OSError, RuntimeError, subprocess.TimeoutExpired, zipfile.BadZipFile) as error:
        parser.exit(2, f"{error}\n")
    print(f"{'complete' if complete else 'incomplete'} comparison. See {args.work / 'results.json'}.")
    return 0 if complete else 1


if __name__ == "__main__":
    sys.exit(main())
