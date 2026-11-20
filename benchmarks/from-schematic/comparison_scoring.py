# Copyright (C) 2026 Florian Loitsch.
"""Native evidence shared by the three pinned board comparisons."""

import collections
import hashlib
import json
import sys
from pathlib import Path

HERE = Path(__file__).resolve().parent
sys.path.insert(0, str(HERE.parent / "agent-tasks"))
sys.path.insert(0, str(HERE.parent))
import harness_common  # noqa: E402
from returned_board import (footprint_differences, native_counts, native_is_excluded,  # noqa: E402
                            normalized_footprints, require)

DRC_CAP = 199
WORK_RATE = "3.15e6,150e6"
DRC_ARGS = ["pcb", "drc", "--format", "json", "--severity-all", "--schematic-parity", "--refill-zones", "--all-track-errors"]
NATIVE_CHECKS = {"as_saved": [arg for arg in DRC_ARGS if arg != "--refill-zones"], "refilled": DRC_ARGS}


def digest(data):
    return hashlib.sha256(data).hexdigest()


def pinned(path, evidence):
    data = path.read_bytes()
    require(digest(data) == evidence["sha256"], f"checksum mismatch: {path}")
    return data


def poses(board):
    return {fp.GetReference(): (*point(fp.GetPosition()), fp.GetOrientationDegrees() % 360, fp.IsFlipped())
            for fp in board.GetFootprints()}


def point(vector):
    return vector.x, vector.y


def inspect_board(pcbnew, path, min_tracks, board=None):
    """Read the exact segment endpoints in KiCad internal units on this rectangular fixture."""
    if board is None:
        board = pcbnew.LoadBoard(str(path))
    require(board is not None, f"KiCad could not load {path}")
    layers = [board.GetStandardLayerName(layer) for layer in board.GetEnabledLayers().CuStack()]
    footprints = list(board.GetFootprints())
    edge_cuts = pcbnew.Edge_Cuts
    require(not any(graphic.GetLayer() == edge_cuts for fp in footprints for graphic in fp.GraphicalItems()),
            f"unexpected footprint Edge.Cuts in {path}")
    edges = [graphic for graphic in board.GetDrawings() if graphic.GetLayer() == edge_cuts]
    outline = pcbnew.SHAPE_POLY_SET()
    require(len(edges) == 4 and all(isinstance(edge, pcbnew.PCB_SHAPE) and edge.GetShape() == pcbnew.SHAPE_T_SEGMENT
                                   for edge in edges)
            and board.GetBoardPolygonOutlines(outline, False) and outline.OutlineCount() == 1
            and outline.HoleCount(0) == 0, f"expected the starter's four-edge rectangle in {path}")
    perimeter = sorted(tuple(sorted((point(edge.GetStart()), point(edge.GetEnd())))) for edge in edges)
    outline_box = board.GetBoardEdgesBoundingBox()
    xs, ys = zip(*(vertex for edge in perimeter for vertex in edge))
    x0, y0, x1, y1 = min(xs), min(ys), max(xs), max(ys)
    boxes = [fp.GetBoundingBox(False, False) for fp in footprints]
    origins = [point(fp.GetPosition()) for fp in footprints]
    proxy_ids = sorted((fp.GetReference(), fp.GetFPIDAsString()) for fp in footprints)
    pad_nets = sorted((fp.GetReference(), pad.GetNumber(), pad.GetNetname()) for fp in footprints for pad in fp.Pads())
    tracks, vias = [], []
    for item in board.GetTracks():
        if isinstance(item, pcbnew.PCB_VIA):
            vias.append(item)
        else:
            tracks.append(item)
    length = sum(item.GetLength() for item in tracks)
    unassigned = sum(item.GetLength() for item in tracks if item.GetNetCode() <= 0)
    signature = {"outline_iu": perimeter, "copper_layers": layers, "proxies": proxy_ids, "pad_nets": pad_nets}
    geometry = {
        "footprints": len(footprints), "pads": len(pad_nets), "nets": len({net for _, _, net in pad_nets if net}),
        "copper_layers": layers, "outline_iu": perimeter,
        "pad_nets_sha256": digest(json.dumps(pad_nets).encode()),
        "locked_footprints": sum(fp.IsLocked() for fp in footprints),
        "footprints_clear_of_outline_bounds": sum(not box.Intersects(outline_box) for box in boxes),
        "footprint_origins_inside_outline": sum(x0 <= x <= x1 and y0 <= y <= y1 for x, y in origins),
        "footprint_bounds_inside_outline": sum(x0 <= box.GetX() <= box.GetRight() <= x1
                                               and y0 <= box.GetY() <= box.GetBottom() <= y1 for box in boxes),
        "track_segments_and_arcs": len(tracks), "wire_length_mm": round(pcbnew.ToMM(length), 3),
        "net_assigned_wire_length_mm": round(pcbnew.ToMM(length - unassigned), 3),
        "unassigned_wire_length_mm": round(pcbnew.ToMM(unassigned), 3),
        "vias": len(vias), "unassigned_vias": sum(item.GetNetCode() <= 0 for item in vias), "zones": len(board.Zones()),
        "tracks_below_minimum": {name: sum(item.GetWidth() < pcbnew.FromMM(minimum) for item in tracks)
                                 for name, minimum in min_tracks.items()},
    }
    return signature, geometry


def run_command(command, log, env, timeout):
    process = harness_common.run_group([str(arg) for arg in command], capture_output=True, text=True,
                                       env=env, timeout=timeout)
    log.write_text(process.stdout + process.stderr)
    return process


def write_project(directory, board_id, schematic, pcb, settings):
    directory.mkdir(parents=True)
    (directory / f"{board_id}.kicad_sch").write_bytes(schematic)
    (directory / f"{board_id}.kicad_pro").write_bytes(settings)
    path = directory / f"{board_id}.kicad_pcb"
    path.write_bytes(pcb)
    return path


def score(board, pcb, board_id, settings, kicad, environment, work, timeout, drc_args=DRC_ARGS, report_stem="drc"):
    report_path, log_path = work / f"{report_stem}.json", work / f"{report_stem}.log"
    process = run_command([kicad, *drc_args, "--output", report_path, board], log_path, environment, timeout)
    require(process.returncode == 0, f"KiCad DRC exited {process.returncode}; see {log_path}")
    require(board.read_bytes() == pcb, f"KiCad changed the scored board: {board}")
    require(board.with_suffix(".kicad_pro").read_bytes() == settings, f"KiCad changed the scored settings: {board}")
    report = json.loads(report_path.read_text())
    counts = native_counts(report, f"{board_id}.kicad_pcb", process.stdout)
    errors = [item for item in report["violations"] if item["severity"] == "error"]
    raw_types = collections.Counter(item["type"] for item in errors)
    active_parity = [item for item in report["schematic_parity"] if not native_is_excluded(item)]
    parity = collections.Counter((item["type"], item["description"]) for item in active_parity)
    counts.update({"raw_drc_errors_by_type": dict(sorted(raw_types.items())),
                   "capped_error_types": sorted(kind for kind, count in raw_types.items() if count >= DRC_CAP),
                   "mask_bridge_errors_with_unassigned_items": sum(item["type"] == "solder_mask_bridge" and any(
                       "[<no net>]" in affected.get("description", "") for affected in item.get("items", []))
                       for item in errors if not native_is_excluded(item)),
                   "parity_by_type": dict(collections.Counter(item["type"] for item in active_parity))})
    return counts, parity


def measure_geometry(pcbnew, path, minimums, *, electrical, power_nets):
    board = pcbnew.LoadBoard(str(path))
    signature, geometry = inspect_board(pcbnew, path, minimums, board=board)
    geometry["back_footprints"] = sorted(fp.GetReference() for fp in board.GetFootprints() if fp.IsFlipped())
    geometry["zones"] = [{"net": zone.GetNetname(), "name": zone.GetZoneName(),
                          "layers": [board.GetStandardLayerName(layer) for layer in zone.GetLayerSet().CuStack()],
                          "filled_layers": [board.GetStandardLayerName(layer) for layer in zone.GetLayerSet().CuStack()
                                            if zone.HasFilledPolysForLayer(layer)]}
                         for zone in board.Zones() if not zone.GetIsRuleArea()]
    geometry["widths_mm_by_net"] = {}
    for item in board.GetTracks():
        if not isinstance(item, pcbnew.PCB_VIA):
            geometry["widths_mm_by_net"].setdefault(item.GetNetname(), set()).add(round(pcbnew.ToMM(item.GetWidth()), 6))
    geometry["widths_mm_by_net"] = {net: sorted(widths) for net, widths in geometry["widths_mm_by_net"].items()}
    geometry["power_track_widths_mm"] = {net: geometry["widths_mm_by_net"].get(net, []) for net in power_nets}
    return signature, normalized_footprints(pcbnew, board, electrical=electrical), board, geometry


def run_native_checks(path, schematic, pcb, settings, kicad, env, timeout):
    checks, parities = {}, {}
    for label, args in NATIVE_CHECKS.items():
        stem = f"drc-{label}"
        checks[label], parities[label] = score(path, pcb, path.stem, settings, kicad, env, path.parent, timeout, args, stem)
        require(path.with_suffix(".kicad_sch").read_bytes() == schematic, f"KiCad changed the scored schematic: {path}")
        report = json.loads((path.parent / f"{stem}.json").read_text())
        checks[label]["errors_involving_unassigned_zone"] = sum(
            issue["severity"] == "error" and any("Zone [<no net>]" in item.get("description", "") for item in issue.get("items", []))
            for issue in report["violations"])
    return checks, parities


def measure_board(pcbnew, directory, board_id, schematic, pcb, settings, minimums, kicad, env, timeout, *, electrical, power_nets):
    path = write_project(directory, board_id, schematic, pcb, settings)
    geometry = measure_geometry(pcbnew, path, minimums, electrical=electrical, power_nets=power_nets)
    return (*geometry, *run_native_checks(path, schematic, pcb, settings, kicad, env, timeout))
