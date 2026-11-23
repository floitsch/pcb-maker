#!/usr/bin/env python3
# Copyright (C) 2026 Florian Loitsch.
"""Replay the published standalone QPlayer board and score it in native KiCad."""

import argparse
import collections
import io
import json
import os
from pathlib import Path
import shutil
import subprocess
import sys
import zipfile

import comparison_scoring as common
from returned_board import board_outline, children, footprint_cutouts, outline_digest, polygon

SOURCE = "https://www.quilter.ai/blog/qplayer"
STARTER = {"sha256": "4a5a79c05f81021f7ce4a34632fd3f2361b8685ff5455ee6331387827bc90e41"}
ARCHIVE = {"sha256": "133e504ba143a9f8ec5654a6139bae4bdfed7a09c432bfcfce9ab5a5aed3ceba"}
BOARD_ID = "quilter"
BOARD = f"{BOARD_ID}.kicad_pcb"
PROJECT = f"{BOARD_ID}.kicad_pro"
ANCHORS = ["CN1", "JP1", "JP2", "JP4", "JP6", "JP8", "LEFTSPK0", "RIGHTSPK0", "X1"]
LAYERS = ["F.Cu", "In1.Cu", "In2.Cu", "B.Cu"]
PLANES = {"GND": "In1.Cu", "3.3V": "In2.Cu"}
ROUTER = {"automatic_planes": False, "add_pours": [{"net": net, "layers": [layer], "plane": True}
                                                  for net, layer in PLANES.items()]}
LAYOUT = {"total_seconds": 1500, "placer": {"constraints": "constraints.json"}}
DUMP = common.harness_common.atomic_json


def stage_board(directory, data, project):
    directory.mkdir(parents=True)
    (directory / PROJECT).write_bytes(project)
    path = directory / BOARD
    path.write_bytes(data)
    return path


def inspect(pcbnew, path):
    board = pcbnew.LoadBoard(str(path))
    common.require(board is not None, f"KiCad could not load {path}")
    items = list(children(path.read_text()))
    fields = dict(items)
    setup = {key: form for key, form in children(fields["setup"]) if key != "pcbplotparams"}
    outline = board_outline(items)
    common.require(not footprint_cutouts(items), f"unexpected footprint cutout in {path}")
    models = common.normalized_footprints(pcbnew, board, electrical=True)
    poses = common.poses(board)
    pads = [pad for model in models.values() for group in model["pads"].values() for pad in group]
    box = board.GetBoardEdgesBoundingBox()
    vias, tracks = [], []
    for item in board.GetTracks():
        (vias if isinstance(item, pcbnew.PCB_VIA) else tracks).append(item)
    planes, keepouts = [], []
    shape_matches = True
    outline_iu = outline_internal_units(outline)
    for zone in board.Zones():
        layers = [board.GetLayerName(layer) for layer in zone.GetLayerSet().CuStack()]
        shape = zone.Outline()
        if shape.OutlineCount() == 1 and shape.HoleCount(0) == 0:
            chain = shape.COutline(0)
            points = [common.point(chain.CPoint(i)) for i in range(chain.PointCount())]
            shape_matches &= len(set(points)) >= 3 and collections.Counter(polygon(points)) == outline_iu
        else:
            shape_matches = False
        if zone.GetIsRuleArea():
            keepouts.append({"name": zone.GetZoneName(), "layers": layers, "tracks_only": zone.GetDoNotAllowTracks()
                             and not any((zone.GetDoNotAllowVias(), zone.GetDoNotAllowPads(),
                                          zone.GetDoNotAllowZoneFills(), zone.GetDoNotAllowFootprints()))})
        else:
            planes.append({"net": zone.GetNetname(), "layers": layers})
    geometry = {"outline_sha256": outline_digest(outline), "footprints": len(models), "pads": len(pads),
                "pad_nets": len({pad["net"] for pad in pads if pad["net"]}),
                "layers": [board.GetLayerName(layer) for layer in board.GetEnabledLayers().CuStack()],
                "outside_origins": sorted(ref for ref, (x, y, _, _) in poses.items()
                                          if not box.Contains(pcbnew.VECTOR2I(x, y))),
                "outside_bounds": sorted(fp.GetReference() for fp in board.GetFootprints()
                                         if not box.Contains(fp.GetBoundingBox(False, False))),
                "wire_length_mm": round(pcbnew.ToMM(sum(track.GetLength() for track in tracks)), 3),
                "unassigned_wire_length_mm": round(pcbnew.ToMM(sum(track.GetLength() for track in tracks
                                                                  if track.GetNetCode() <= 0)), 3),
                "segments_and_arcs": len(tracks), "vias": len(vias), "planes": sorted(planes, key=lambda z: z["net"]),
                "plane_track_keepouts": sorted(keepouts, key=lambda z: z["name"]),
                "zone_outlines_match_board": shape_matches}
    return board, geometry, models, poses, (dict(children(fields["general"])), setup)


def outline_internal_units(outline):
    common.require(all(edge[0] == "line" for edge in outline), "expected straight QPlayer outline edges")
    return collections.Counter({(kind, tuple(int(v * 1_000_000) for v in a), tuple(int(v * 1_000_000) for v in b)): count
                                for (kind, a, b), count in outline.items()})


def score(path, board_data, project, kicad, env, timeout, version):
    reports = {}
    for mode, flags in common.NATIVE_CHECKS.items():
        args = [flag for flag in flags if flag != "--schematic-parity"]
        path_json = path.parent / f"drc-{mode}.json"
        process = common.run_command([kicad, *args, "--output", path_json, path], path_json.with_suffix(".log"), env, timeout)
        common.require(process.returncode == 0, f"KiCad DRC exited {process.returncode}: {path_json.with_suffix('.log')}")
        common.require(path.read_bytes() == board_data and path.with_suffix(".kicad_pro").read_bytes() == project,
                       f"KiCad changed the scored project: {path}")
        report = json.loads(path_json.read_text())
        counts = common.native_counts(report, BOARD, process.stdout, schematic_parity=False)
        common.require(counts["kicad_version"] == version, f"KiCad DRC version changed: {path_json}")
        counts["raw_drc_errors_by_type"] = dict(collections.Counter(issue["type"] for issue in report["violations"]
                                                                  if issue["severity"] == "error"))
        reports[mode] = counts
    return reports


def replay(pcbnew, work, starter_path, archive_path, binary, existing, timeout):
    common.require(not existing or not work.is_relative_to(existing), "put the fresh output outside the existing layout")
    original = common.pinned(starter_path, STARTER)
    with zipfile.ZipFile(io.BytesIO(common.pinned(archive_path, ARCHIVE))) as archive:
        candidate = archive.read(f"{BOARD_ID}/{BOARD}")
        project = archive.read(f"{BOARD_ID}/{PROJECT}")
    work.mkdir(parents=True)
    starter = stage_board(work / "authored", original, project)
    board, baseline, authored_models, authored_poses, authored_setup = inspect(pcbnew, starter)
    common.require((baseline["footprints"], baseline["pads"], baseline["pad_nets"]) == (84, 319, 93)
                   and baseline["layers"] == ["F.Cu", "B.Cu"] and not board.Zones()
                   and not board.GetTracks() and sorted(authored_poses.keys() - set(baseline["outside_origins"])) == ANCHORS,
                   "unexpected archived starter geometry")
    front = sorted(ref for ref, pose in authored_poses.items() if not pose[-1])
    back = sorted(authored_poses.keys() - set(front))
    footprints = {footprint.GetReference(): footprint for footprint in board.GetFootprints()}
    brief = {"version": 1, "move_all": True, "front": front, "back": back, "place": [
        {"part": ref, "origin": "absolute", "x": pcbnew.ToMM(authored_poses[ref][0]),
         "y": pcbnew.ToMM(authored_poses[ref][1]), "angle": footprints[ref].GetOrientationDegrees()} for ref in ANCHORS]}
    marker = '(0 "F.Cu" signal)'
    text = starter.read_text()
    common.require(text.count(marker) == 1, "unexpected starter's copper layer declarations")
    adapted = text.replace(marker, marker + '\n    (1 "In1.Cu" signal)\n    (2 "In2.Cu" signal)').encode()
    layout_input = stage_board(work / "input", adapted, project).parent
    configs = {"constraints.json": brief, "layout.json": LAYOUT, "router.json": ROUTER}
    for filename, data in configs.items():
        DUMP(layout_input / filename, data, newline=True)
    kicad = os.environ.get("PCB_KICAD_CLI") or "kicad-cli"
    env = {key: value for key, value in os.environ.items() if not key.startswith("PCB_")}
    env.update({"LC_ALL": "C", "PCB_KICAD_CLI": kicad, "PCB_WORK_RATE": common.WORK_RATE, "RAYON_NUM_THREADS": "4",
                "PCB_MAKER_CACHE_DIR": str(work / "cache"), "PCB_MAKER_LEDGER": str(work / "ledger.json"),
                "XDG_CONFIG_HOME": str(work / "xdg-config"), "KICAD_CONFIG_HOME": str(work / "kicad-config")})
    version = common.run_command([kicad, "version"], work / "kicad-version.log", env, timeout).stdout.strip()
    common.require(version.startswith("10.") and pcbnew.Version() == version, "need matching KiCad 10 CLI and pcbnew")
    layout = existing or work / "layout"
    result = {"complete": False, "source": SOURCE, "archive_starter": STARTER, "archive_return": ARCHIVE,
              "authored": baseline, "kicad_version": version, "schematic_parity": "unavailable: board-only archives",
              "common_project_sha256": common.digest(project), "configs": configs, "layout_path": str(layout),
              "work_rate": common.WORK_RATE, "reused_layout": existing is not None,
              "native_drc_includes_all_track_errors": True, "boards": {}}
    report_path = work / "results.json"
    DUMP(report_path, result, newline=True)
    if existing is None:
        binary = Path(shutil.copy2(binary, work / "pcb-maker"))
        result["binary_sha256"] = common.harness_common.file_sha256(binary)
        result["threads"] = 4
        command = [binary, "layout-kicad-board", layout_input, BOARD_ID, layout,
                   layout_input / "router.json", layout_input / "layout.json"]
        process = common.run_command(command, work / "layout.log", env, timeout)
        common.require(process.returncode in (0, 1, 3), f"layout exited {process.returncode}; see layout.log")
    verdict = json.loads((layout / "verdict.json").read_text())
    common.require(verdict["command"] == "layout-kicad-board" and verdict["work_rate"] == 3_150_000
                   and verdict["placer_work_rate"] == 150_000_000, "unexpected layout work rates")
    for directory in (layout / "placed", layout / "result"):
        common.require((directory / PROJECT).read_bytes() == project
                       and all(json.loads((directory / name).read_text()) == data for name, data in configs.items())
                       and not list(directory.glob("*.kicad_dru")) and not list(directory.glob("*.kicad_sch")),
                       f"unexpected project settings, schematic or layout config in {directory}")
    result["pcb_maker_verdict"] = verdict
    planes = [{"net": net, "layers": [PLANES[net]]} for net in sorted(PLANES)]
    keepouts = [{"name": f"plane {net} {PLANES[net]}", "layers": [PLANES[net]], "tracks_only": True} for net in sorted(PLANES)]
    for label, data in {"starter": adapted, "quilter": candidate, "pcb-maker": (layout / "result" / BOARD).read_bytes()}.items():
        path = stage_board(work / "scores" / label, data, project)
        _, geometry, models, poses, setup = inspect(pcbnew, path)
        changed_anchors = [ref for ref in ANCHORS if poses.get(ref) != authored_poses[ref]]
        changed_faces = sorted(ref for ref, pose in authored_poses.items() if pose[-1] != poses.get(ref, (None,))[-1])
        mismatch = common.footprint_differences(authored_models, models)
        checks = {"footprint_and_pad_identity": not mismatch, "anchored_poses": not changed_anchors,
                  "footprint_faces": not changed_faces, "outline": geometry["outline_sha256"] == baseline["outline_sha256"],
                  "layers": geometry["layers"] == LAYERS, "board_settings": setup == authored_setup,
                  "planes": geometry["zone_outlines_match_board"] and geometry["planes"] == (planes if label != "starter" else []),
                  "plane_rule_areas": geometry["plane_track_keepouts"] == (keepouts if label == "pcb-maker" else []),
                  "placement": (geometry["outside_origins"] == baseline["outside_origins"] if label == "starter"
                                else geometry["outside_origins"] == [] and geometry["outside_bounds"] == ["CN1", "X1"])}
        row = {"board_sha256": common.digest(data), "matched": all(checks.values()), "checks": checks, "geometry": geometry,
               "score_path": str(path.parent.relative_to(work)),
               "changed_anchors": changed_anchors, "changed_faces": changed_faces, "footprint_or_pad_mismatches": mismatch}
        result["boards"][label] = row
        DUMP(report_path, result, newline=True)
        if row["matched"]:
            row["native_drc"] = score(path, data, project, kicad, env, timeout, version)
        DUMP(report_path, result, newline=True)
    result["complete"] = all(row["matched"] and "native_drc" in row for row in result["boards"].values())
    DUMP(report_path, result, newline=True)
    return result["complete"]


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("work", type=Path, help="fresh output directory")
    parser.add_argument("--starter", type=Path, required=True, help="pinned input KiCad board from Quilter")
    parser.add_argument("--archive", type=Path, required=True, help="pinned output ZIP from Quilter")
    mode = parser.add_mutually_exclusive_group(required=True)
    mode.add_argument("--binary", type=Path, help="pcb-maker executable for a fresh layout")
    mode.add_argument("--existing-layout", type=Path, help="rescore a layout made with this benchmark's inputs")
    parser.add_argument("--timeout", type=int, default=1800, help="wall seconds per command")
    args = parser.parse_args()
    try:
        import pcbnew
        complete = replay(pcbnew, args.work.resolve(), args.starter.resolve(), args.archive.resolve(),
                          args.binary.resolve() if args.binary else None,
                          args.existing_layout.resolve() if args.existing_layout else None, args.timeout)
    except (ImportError, KeyError, OSError, RuntimeError, ValueError, subprocess.TimeoutExpired, zipfile.BadZipFile) as error:
        parser.exit(2, f"{error}\n")
    print(f"{'complete' if complete else 'incomplete'} comparison. See {args.work / 'results.json'}.")
    return 0 if complete else 1


if __name__ == "__main__":
    sys.exit(main())
