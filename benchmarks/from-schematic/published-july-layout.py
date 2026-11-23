#!/usr/bin/env python3
# Copyright (C) 2026 Florian Loitsch.
"""Replay the five archived July 14 powder-doser boards and a fresh layout."""

import argparse
import json
import math
import os
import re
import shutil
import subprocess
import sys
from concurrent.futures import ThreadPoolExecutor
from pathlib import Path

import comparison_scoring as common
import july_rail_drop
from exchange_package import board_text
from returned_board import children, pad_distance_checks
from routed_supply_paths import measure_supply_paths

HERE = Path(__file__).resolve().parent

MANIFEST = HERE / "published-july-layout.json"
BOARD_COUNTS = (15, 172, 22)
SOURCE_LAYERS = ["F.Cu", "B.Cu"]
ROUTED_LAYERS = ["F.Cu", "In1.Cu", "In2.Cu", "B.Cu"]
PHYSICAL_PAIRS = [("C1:1", "U1:3"), ("C2:1", "U2:22"), ("C3:1", "U5:16")]
PUBLISHED_PAIRS = [("C1:1", "U1:2"), *PHYSICAL_PAIRS[1:]]
POWER_NETS = ["+12V", "GND"]
POWER_WIDTH_MM = 0.513593
ROUTER = {"add_pours": [{"net": "GND", "layers": ["In1.Cu"], "plane": True}], "automatic_planes": False,
          "net_classes": [{"name": "PublishedPower", "nets": POWER_NETS, "track_width_mm": POWER_WIDTH_MM}]}
STACKUP_TOKEN = re.compile(r'"(?:[^"\\]|\\.)*"|[()]|[^\s()"]+')


def stackup(data):
    setup = dict(children(data.decode()))["setup"]
    return dict(children(setup)).get("stackup")


def adapt_starter(data, stack):
    """Add only the shared stack metadata and two conductors to the archived starter."""
    items = []
    for key, form in children(data.decode()):
        if key == "layers":
            layers = [definition for _, definition in children(form)]
            form = "(layers " + " ".join([layers[0], '(1 "In1.Cu" signal)', '(2 "In2.Cu" signal)', *layers[1:]]) + ")"
        elif key == "setup":
            common.require("stackup" not in dict(children(form)), "archived setup already contains a stackup")
            form = form[:-1] + stack + ")"
        items.append(form)
    return board_text(items).encode()


def import_schematic_project(work, directory, board_id, binary, source, settings, stack, env, timeout):
    board_name = f"{board_id}.kicad_pcb"
    schematic_name = f"{board_id}.kicad_sch"
    donor = common.write_project(work / "schematic-source", board_id, source[schematic_name],
                                 source[board_name], settings)
    process = common.run_command([binary, "import-kicad-netlist", donor.with_suffix(".kicad_sch"),
                                  directory, board_id, "--layers", "4", "--footprints-from", donor],
                                 work / "import.log", env, timeout)
    common.require(process.returncode == 0, "schematic import failed; see import.log")
    report = json.loads(process.stdout)
    common.require(report.get("footprints") == BOARD_COUNTS[0] and not report.get("missing"),
                   "schematic import did not recover every source footprint")
    common.require((directory / schematic_name).read_bytes() == source[schematic_name]
                   and (directory / f"{board_id}.kicad_pro").read_bytes() == settings,
                   "schematic import changed the pinned schematic or matched project rules")
    board = directory / board_name
    items = []
    for head, form in children(board.read_text()):
        common.require(head not in {"segment", "arc", "via", "zone"},
                       "the schematic import already contains board copper")
        if head == "setup":
            common.require("stackup" not in dict(children(form)), "schematic import already contains stackup metadata")
            form = form[:-1] + stack + ")"
        items.append(form)
    outline = [form for head, form in children(source[board_name].decode())
               if head.startswith("gr_") and dict(children(form)).get("layer") == '(layer "Edge.Cuts")']
    common.require(len(outline) == 4, "expected the pinned four-edge rectangle")
    board.write_text(board_text(items + outline))
    common.harness_common.atomic_json(work / "import.json", report, newline=True)
    return board


def routed_pad_names(signature):
    # The archive leaves these pins unnamed; a native schematic import
    # gives each one its own disconnected pseudo-net.
    return sorted((ref, pad, "" if net.startswith("unconnected-(") else net)
                  for ref, pad, net in signature["pad_nets"])


def bounds(points):
    x, y = zip(*points)
    return min(x), min(y), max(x), max(y)


def bounds_gap(left, right):
    x0, y0, x1, y1 = left
    u0, v0, u1, v1 = right
    return math.hypot(max(x0 - u1, u0 - x1, 0), max(y0 - v1, v0 - y1, 0))


def fab_rectangle(pcbnew, footprint):
    """Use the nominal Fab outline only when its four native lines prove the box."""
    layer = pcbnew.B_Fab if footprint.IsFlipped() else pcbnew.F_Fab
    graphics = [item for item in footprint.GraphicalItems() if isinstance(item, pcbnew.PCB_SHAPE)
                and item.GetLayer() == layer]
    if len(graphics) != 4 or any(item.GetShape() != pcbnew.SHAPE_T_SEGMENT for item in graphics):
        return None
    edges = {frozenset((common.point(item.GetStart()), common.point(item.GetEnd()))) for item in graphics}
    box = bounds([vertex for edge in edges for vertex in edge])
    x0, y0, x1, y1 = box
    corners = [(x0, y0), (x1, y0), (x1, y1), (x0, y1)]
    if x0 < x1 and y0 < y1 and edges == {frozenset((a, b)) for a, b in zip(corners, corners[1:] + corners[:1])}:
        return box
    return None


def courtyard_bounds(polygon):
    """Measure the polygon vertices. KiCad's cached BBox has its own margin."""
    points = []
    for i in range(polygon.OutlineCount()):
        outline = polygon.COutline(i)
        points.extend(common.point(outline.CPoint(j)) for j in range(outline.PointCount()))
    return bounds(points) if points else None


def package_spacing(pcbnew, source, target):
    parts = [source, target]
    angles = [part.GetOrientationDegrees() % 360 for part in parts]
    layers = [pcbnew.B_Cu if part.IsFlipped() else pcbnew.F_Cu for part in parts]
    shapes = [part.GetCourtyard(layer) for part, layer in zip(parts, layers)]
    court_boxes = [courtyard_bounds(shape) for shape in shapes]
    fab_boxes = [fab_rectangle(pcbnew, part) for part in parts]
    return {"faces": ["back" if part.IsFlipped() else "front" for part in parts],
            "orientation_degrees": angles, "relative_rotation_degrees": (angles[0] - angles[1]) % 360,
            "projected_fab_outline_gap_mm": round(pcbnew.ToMM(bounds_gap(*fab_boxes)), 6) if all(fab_boxes) else None,
            "projected_courtyard_gap_mm": pcbnew.ToMM(shapes[0].GetClearance(shapes[1])) if all(court_boxes) else None,
            "projected_courtyard_aabb_gap_mm": round(pcbnew.ToMM(bounds_gap(*court_boxes)), 6) if all(court_boxes) else None,
            "projected_courtyards_collide": shapes[0].Collide(shapes[1]) if all(court_boxes) else None}


def cap_distance(pcbnew, board, source, target, *, package=False):
    def pad(anchor):
        ref, pin = anchor.split(":")
        result = [(fp, pad) for fp in board.GetFootprints() if fp.GetReference() == ref
                  for pad in fp.Pads() if pad.GetNumber() == pin]
        common.require(len(result) == 1, f"missing or ambiguous metric pad {anchor}")
        return result[0]

    source_part, a = pad(source)
    target_part, b = pad(target)
    common.require(a.GetNetCode() > 0 and a.GetNetCode() == b.GetNetCode(), f"metric pads differ in net: {source}, {target}")
    delta = [end - start for start, end in zip(common.point(a.GetPosition()), common.point(b.GetPosition()))]
    result = {"from": source, "to": target, "target_pin_function": b.GetPinFunction(),
              "distance_mm": round(pcbnew.ToMM(math.hypot(*delta)), 6)}
    if package:
        result["pad_delta_mm"] = [pcbnew.ToMM(value) for value in delta]
        result["package_spacing"] = package_spacing(pcbnew, source_part, target_part)
        result["pad_copper_contact"] = pad_copper_contact(pcbnew, board, a, b)
    return result


def pad_copper_contact(pcbnew, board, a, b):
    """Measure the pad copper polygons on each shared enabled copper layer."""
    measurements = {}
    for layer in board.GetEnabledLayers().CuStack():
        if not a.IsOnLayer(layer) or not b.IsOnLayer(layer):
            continue
        name = board.GetStandardLayerName(layer)
        left = a.GetEffectivePolygon(layer, pcbnew.ERROR_INSIDE)
        right = b.GetEffectivePolygon(layer, pcbnew.ERROR_INSIDE)
        if not left.OutlineCount() or not right.OutlineCount():
            measurements[name] = {"status": "unavailable"}
            continue
        gap = left.GetClearance(right)
        measurements[name] = {
            "status": "measured", "copper_gap_mm": pcbnew.ToMM(gap), "contact": gap == 0,
        }
    return measurements


def placement_constraints(pad_to_pad, near_objective, pad_near_max_mm):
    if near_objective or pad_near_max_mm is not None:
        common.require(pad_to_pad and not (near_objective and pad_near_max_mm is not None),
                       "choose one mode for the explicit pad-near pairs")
    if pad_near_max_mm is not None:
        common.require(math.isfinite(pad_near_max_mm) and pad_near_max_mm > 0,
                       "the diagnostic pad-near limit must be a positive finite number")
    constraints = {"version": 1, "move_all": True, "front": ["*"], "near": []}
    for cap, pin in PHYSICAL_PAIRS:
        part, pad = cap.split(":")
        entry = {"part": part, "pin_of": pin}
        if pad_to_pad:
            entry["pad"] = pad
        if near_objective:
            entry["minimize"] = True
        else:
            entry["max_mm"] = pad_near_max_mm if pad_near_max_mm is not None else 0
        constraints["near"].append(entry)
    return constraints


def refinement_input(pcbnew, directory, board_name, starter, binary, budget, from_schematic=False):
    control = json.loads((directory / "results.json").read_bytes())
    common.require(control["complete"] and control["manifest_sha256"] == common.harness_common.file_sha256(MANIFEST)
                      and control["binary"]["sha256"] == common.harness_common.file_sha256(binary)
                      and control["adaptation"]["starter_sha256"] == common.digest(starter)
                      and control["layout"]["router"] == ROUTER and control["layout"]["budget_seconds"] == budget
                      and control["layout"].get("from_schematic", False) == from_schematic
                      and not control["layout"]["pad_to_pad_placement"], "refinement requires a matched body-to-pin replay")
    path = directory / "layout" / "result" / board_name
    result = control["boards"]["pcb-maker"]
    common.require(result["board_sha256"] == common.harness_common.file_sha256(path)
                      and result["placement"]["all_front"] and result["placement"]["inside_outline"]
                      and result["outline_matches"] and result["layers_match"] and not result["footprint_or_pin_mismatches"],
                      "the body-to-pin result does not match its verified geometry")
    return path, common.poses(pcbnew.LoadBoard(str(path)))


def run_layout(command, log, env, timeout):
    try:
        process = common.run_command(command, log, env, timeout)
    except subprocess.TimeoutExpired:
        return 3, f"layout exceeded the {timeout} s wall timeout"
    transcript = process.stdout + process.stderr
    error = "layout could not use the shared memory ledger" if "cannot be written" in transcript and "without it" in transcript else None
    return process.returncode, error


def score_archive(directory, board_id, schematic, pcb, settings, kicad, env, timeout):
    path = common.write_project(directory, board_id, schematic, pcb, settings)
    return common.run_native_checks(path, schematic, pcb, settings, kicad, env, timeout)


def run(work, dataset, binary, budget, timeout, pcbnew, pad_to_pad, refine, *,
        near_objective=False, pad_near_max_mm=None, from_schematic=False, serial_archive_scoring=False,
        rail_drop_simulator=None):
    manifest = json.loads(MANIFEST.read_text())
    common.require(not work.is_relative_to(dataset), "place the fresh work directory outside the archival dataset")
    common.require(budget > 0, "layout budget must be positive")
    constraints = placement_constraints(pad_to_pad, near_objective, pad_near_max_mm)
    if rail_drop_simulator is not None:
        simulator, simulator_provenance = july_rail_drop.load_simulator(rail_drop_simulator)
    board_id = manifest["board_id"]
    board_name, project_name, schematic_name = (f"{board_id}.kicad_{suffix}" for suffix in ("pcb", "pro", "sch"))
    source = {name: common.pinned(dataset / "original" / name, pin) for name, pin in manifest["source_files"].items()}
    common.pinned(dataset / "review.md", manifest["review"])
    candidates = {name: {file: common.pinned(dataset / "candidates" / name / file, pin) for file, pin in files.items()}
                  for name, files in manifest["candidates"].items()}
    stacks = {stackup(files[board_name]) for files in candidates.values()}
    settings = {files[project_name] for files in candidates.values()}
    common.require(len(stacks) == 1 and None not in stacks and len(settings) == 1,
                      "published boards do not agree on stackup and returned project settings")
    stack, common_project = stacks.pop(), settings.pop()
    adapted = adapt_starter(source[board_name], stack)
    design = {label: json.loads(data)["board"]["design_settings"]["rules"]
              for label, data in (("source", source[project_name]), ("common", common_project))}
    minimums = {label: rules["min_track_width"] for label, rules in design.items()}
    work.mkdir(parents=True)
    copied_binary = work / "pcb-maker"
    shutil.copy2(binary, copied_binary)
    env = {key: value for key, value in os.environ.items() if not key.startswith("PCB_")}
    ledger = os.environ.get("PCB_MAKER_LEDGER") or str(Path(os.environ.get("XDG_RUNTIME_DIR") or "/tmp") / "pcb-maker-ledger.json")
    common.require(ledger != "off", "the July layout needs the shared memory ledger")
    kicad = os.environ.get("PCB_KICAD_CLI") or "kicad-cli"
    env.update({"LC_ALL": "C", "PCB_KICAD_CLI": kicad, "PCB_WORK_RATE": common.WORK_RATE, "RAYON_NUM_THREADS": "4",
                "PCB_MAKER_LEDGER": ledger, "PCB_MAKER_CACHE_DIR": str(work / "cache"),
                "XDG_CONFIG_HOME": str(work / "xdg-config"), "KICAD_CONFIG_HOME": str(work / "kicad-config")})
    version = common.run_command([kicad, "version"], work / "kicad-version.log", env, timeout)
    common.require(version.returncode == 0 and version.stdout.strip() == pcbnew.Version()
                      and pcbnew.Version().split(".", 1)[0] == "10", "matching KiCad 10 CLI and pcbnew versions required")
    archive_env = dict(env, XDG_CONFIG_HOME=str(work / "archive-xdg-config"),
                       KICAD_CONFIG_HOME=str(work / "archive-kicad-config"))
    for check_env in (env, archive_env):
        config_home = Path(check_env["KICAD_CONFIG_HOME"]) / ".".join(pcbnew.Version().split(".")[:2])
        config_home.mkdir(parents=True, exist_ok=True)
        for table in ("fp", "sym"):
            (config_home / f"{table}-lib-table").write_text(f"({table}_lib_table (version 7))\n")
    source_measurement = common.measure_board(pcbnew, work / "original-source", board_id, source[schematic_name],
                                       source[board_name], source[project_name], minimums, kicad, env, timeout, electrical=True, power_nets=POWER_NETS)
    original_signature, original_details, original_board, source_geometry, source_checks, _ = source_measurement
    input_dir = work / "layout-input"
    archive_dir = work / "adapted-reference" if from_schematic else input_dir
    archive_board = common.write_project(archive_dir, board_id, source[schematic_name], adapted, common_project)
    adapted_board = pcbnew.LoadBoard(str(archive_board))
    signature, geometry = common.inspect_board(pcbnew, archive_board, minimums, board=adapted_board)
    common.require(original_signature["copper_layers"] == SOURCE_LAYERS and signature["copper_layers"] == ROUTED_LAYERS,
                      "the source or adapted copper layers differ from the expected stack")
    common.require((geometry["footprints"], geometry["pads"], geometry["nets"]) == BOARD_COUNTS
                      and geometry["footprints_clear_of_outline_bounds"] == BOARD_COUNTS[0]
                      and not any(geometry[key] for key in ("track_segments_and_arcs", "vias", "zones", "locked_footprints")),
                      "the adapted starter must remain unplaced and unrouted")
    common.require(all(signature[key] == original_signature[key] for key in signature if key != "copper_layers")
                      and not common.footprint_differences(original_details, common.normalized_footprints(pcbnew, adapted_board, electrical=True)),
                      "the four-layer adaptation changed source geometry")
    common.require(common.poses(original_board) == common.poses(adapted_board), "the four-layer adaptation changed unplaced poses")
    layout_starter = adapted
    layout_details = original_details
    if from_schematic:
        input_board = import_schematic_project(work, input_dir, board_id, copied_binary, source,
                                               common_project, stack, env, timeout)
        imported = pcbnew.LoadBoard(str(input_board))
        imported_signature, imported_geometry = common.inspect_board(pcbnew, input_board, minimums, board=imported)
        common.require(all(imported_signature[key] == signature[key] for key in signature if key != "pad_nets")
                       and routed_pad_names(imported_signature) == routed_pad_names(signature)
                       and not imported_geometry["locked_footprints"],
                       "the schematic import changed matched outlines, layers, footprint IDs or routed terminals")
        common.require(not common.footprint_differences(common.normalized_footprints(pcbnew, adapted_board, electrical=False),
                                                       common.normalized_footprints(pcbnew, imported, electrical=False)),
                       "the schematic import changed matched footprint geometry")
        layout_starter = input_board.read_bytes()
        common.require(STACKUP_TOKEN.findall(stackup(layout_starter) or "") == STACKUP_TOKEN.findall(stack),
                       "the schematic import lost the matched stackup")
        layout_details = common.normalized_footprints(pcbnew, imported, electrical=True)
    refinement = None
    if refine is not None:
        control_board, control_poses = refinement_input(pcbnew, refine, board_name, layout_starter, copied_binary,
                                                        budget, from_schematic)
        move = {entry["part"] for entry in constraints["near"]}
        held = sorted(set(control_poses) - move)
        constraints["place"] = [{"part": ref, "x": pcbnew.ToMM(control_poses[ref][0]),
                                 "y": pcbnew.ToMM(control_poses[ref][1]), "angle": control_poses[ref][2],
                                 "origin": "absolute"} for ref in held]
        refinement = {"control_board": str(control_board), "control_board_sha256": common.harness_common.file_sha256(control_board),
                      "held_references": held, "movable_references": sorted(move)}
    layout = {"total_seconds": budget, "placer": {"constraints": "constraints.json"}}
    for filename, value in (("constraints.json", constraints), ("router.json", ROUTER), ("layout.json", layout)):
        common.harness_common.atomic_json(input_dir / filename, value, newline=True)
    laid_out = work / "layout"
    layout_command = [copied_binary, "layout-kicad-board", input_dir, board_id, laid_out,
                      input_dir / "router.json", input_dir / "layout.json"]
    layout_log = work / "layout.log"
    if serial_archive_scoring:
        exit_code, layout_error = run_layout(layout_command, layout_log, env, timeout)
    # One concurrent native process keeps the additional load bounded. The
    # archive projects and KiCad config are separate from the layout work.
    with ThreadPoolExecutor(max_workers=1) as executor:
        archive_scores = {
            name: executor.submit(score_archive, work / "scores" / name, board_id, source[schematic_name],
                                  files[board_name], common_project, kicad, archive_env, timeout)
            for name, files in candidates.items()
        }
        if not serial_archive_scoring:
            exit_code, layout_error = run_layout(layout_command, layout_log, env, timeout)
    own_path = laid_out / "result" / board_name
    run_report = common.harness_common.read_json(laid_out / "board-layout.json") or {}
    result = {"manifest": manifest, "manifest_sha256": common.harness_common.file_sha256(MANIFEST),
              "dataset": str(dataset), "binary": {"path": str(binary), "sha256": common.harness_common.file_sha256(copied_binary)},
              "native": {"kicad_version": pcbnew.Version(), "arguments": common.NATIVE_CHECKS,
                         "report_cap_per_type": common.DRC_CAP,
                         "archive_scoring": "serial" if serial_archive_scoring else "overlapped_with_layout"},
              "rules": {"source": {"sha256": common.digest(source[project_name]), "design_rules": design["source"]},
                        "common_returned": {"sha256": common.digest(common_project), "design_rules": design["common"]}},
              "adaptation": {"source_layers": SOURCE_LAYERS, "routed_layers": ROUTED_LAYERS,
                             "stackup_sha256": common.digest(stack.encode()), "original_poses_preserved": not from_schematic,
                             "starter_sha256": common.digest(layout_starter), "input_copper": 0},
              "layout": {"budget_seconds": budget, "work_rate": common.WORK_RATE, "threads": 4,
                         "ledger": ledger, "exit_code": exit_code, "error": layout_error,
                         "router": ROUTER, "constraints": constraints, "pad_to_pad_placement": pad_to_pad,
                         "from_schematic": from_schematic, "refinement": refinement,
                         "work_seconds": run_report.get("work_seconds"), "routed": run_report.get("routed", {}).get("routed_connections"),
                         "routable": run_report.get("routed", {}).get("routable_connections"),
                         "soft_constraint_report": run_report.get("constraints")},
              "cap_distance_metric": {"method": "2D Euclidean distance between native pad centers",
                                      "acceptance_threshold_mm": pad_near_max_mm,
                                      "published_buck_anchor": "U1:2, SHDN tied to +12V", "physical_buck_anchor": "U1:3, VIN"},
              "original_source": {"geometry": source_geometry, "native": source_checks}, "boards": {}}
    boards = {"adapted_starter": adapted, **{key: value[board_name] for key, value in candidates.items()}}
    if from_schematic:
        boards["schematic_starter"] = layout_starter
    if own_path.is_file():
        boards["pcb-maker"] = own_path.read_bytes()
    else:
        result["boards"]["pcb-maker"] = {"status": "unverified", "error": "layout produced no board"}
    starter_parities = schematic_parities = None
    for name, pcb in boards.items():
        directory = work / "scores" / name
        report = {"status": "unverified", "board_sha256": common.digest(pcb), "path": str(directory.relative_to(work))}
        result["boards"][name] = report
        try:
            if name in archive_scores:
                checks, parities = archive_scores[name].result()
                actual, footprints, board, geo = common.measure_geometry(
                    pcbnew, directory / board_name, minimums, electrical=True, power_nets=POWER_NETS)
            else:
                actual, footprints, board, geo, checks, parities = common.measure_board(
                    pcbnew, directory, board_id, source[schematic_name], pcb, common_project, minimums, kicad, env, timeout,
                    electrical=True, power_nets=POWER_NETS)
            expected = layout_details if name in {"schematic_starter", "pcb-maker"} else original_details
            mismatches = common.footprint_differences(expected, footprints)
            if name == "adapted_starter":
                starter_parities = parities
            if name == "schematic_starter":
                schematic_parities = parities
            common.require(starter_parities is not None, "missing adapted starter parity evidence")
            placement = None if name in {"adapted_starter", "schematic_starter"} else {
                "all_front": not geo["back_footprints"],
                "inside_outline": geo["footprint_bounds_inside_outline"] == BOARD_COUNTS[0]
                and geo["footprint_origins_inside_outline"] == BOARD_COUNTS[0],
                "published_cap_distances": [cap_distance(pcbnew, board, a, b) for a, b in PUBLISHED_PAIRS],
                "physical_cap_distances": [cap_distance(pcbnew, board, a, b, package=True) for a, b in PHYSICAL_PAIRS],
                "routed_supply_paths": measure_supply_paths(pcbnew, board, PHYSICAL_PAIRS),
            }
            if name == "pcb-maker" and refinement is not None:
                candidate_poses = common.poses(board)
                changed = sorted(ref for ref, at in control_poses.items() if candidate_poses.get(ref) != at)
                placement["held_poses_match"] = all(ref in move for ref in changed)
                placement["changed_poses"] = changed
                common.require(placement["held_poses_match"], "refinement changed a held footprint pose")
            if name == "pcb-maker" and (near_objective or pad_near_max_mm is not None):
                near = pad_distance_checks(constraints["near"], directory / board_name)
                placement["near_measurements"] = near
                placement["near_limits_met"] = None if near_objective else all(row["satisfied"] for row in near)
            report.update({"status": "measured", "geometry": geo, "placement": placement, "native": checks,
                           "outline_matches": actual["outline_iu"] == signature["outline_iu"],
                           "layers_match": actual["copper_layers"] == ROUTED_LAYERS,
                           "stackup_matches": STACKUP_TOKEN.findall(stackup(pcb) or "") == STACKUP_TOKEN.findall(stack),
                           "footprint_or_pin_mismatches": mismatches,
                           "parity_matches_starter": {mode: parity == starter_parities[mode] for mode, parity in parities.items()}})
            if from_schematic and name == "pcb-maker":
                common.require(schematic_parities is not None, "missing schematic starter parity evidence")
                report["parity_matches_schematic_starter"] = {
                    mode: parity == schematic_parities[mode] for mode, parity in parities.items()
                }
        except (ValueError, OSError, TypeError, RuntimeError, KeyError, subprocess.TimeoutExpired) as error:
            report["error"] = str(error)
    if rail_drop_simulator is not None:
        result["rail_drop"] = simulator_provenance
        for name in (*candidates, "pcb-maker"):
            pcb_path = own_path if name == "pcb-maker" else work / "scores" / name / board_name
            result["boards"][name]["rail_drop"] = july_rail_drop.score_board(
                pcbnew, pcb_path, boards.get(name), simulator, simulator_provenance.get("reason"),
            )
    result["complete"] = not layout_error and all(report["status"] == "measured" for report in result["boards"].values())
    common.harness_common.atomic_json(work / "results.json", result, newline=True)
    return result["complete"]


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("work", type=Path, help="fresh output directory")
    parser.add_argument("--dataset", type=Path, required=True, help="pinned original/, candidates/ and review.md")
    parser.add_argument("--binary", type=Path, required=True, help="pcb-maker executable")
    parser.add_argument("--budget-seconds", type=int, default=1200, help="modeled layout seconds")
    near = parser.add_mutually_exclusive_group()
    near.add_argument("--pad-to-pad", action="store_true", help="replay the historical numeric zero-limit pad-to-pad settings")
    near.add_argument("--near-objective", action="store_true", help="minimize the three explicit pad distances without a numerical limit")
    near.add_argument("--pad-near-max-mm", type=float, metavar="MM", help="require a positive diagnostic limit on all three pad distances")
    parser.add_argument("--refine-body-to-pin", type=Path, metavar="CONTROL_WORK",
                        help="use pad-to-pad placement while holding non-capacitor poses from a matched body-to-pin replay")
    parser.add_argument("--from-schematic", action="store_true",
                        help="import the pinned schematic, then add the matched outline, stack and rules")
    parser.add_argument("--serial-archive-scoring", action="store_true",
                        help="run the native archived-board checks after the layout command exits")
    parser.add_argument("--rail-drop-simulator", type=Path, metavar="PINNED_LOCAL_PY",
                        help="also score rail drop with the supplied pinned July author script and ngspice")
    parser.add_argument("--timeout", type=int, default=1800, help="wall timeout per command in seconds")
    args = parser.parse_args()
    try:
        import pcbnew
        refine = args.refine_body_to_pin.resolve() if args.refine_body_to_pin else None
        complete = run(args.work.resolve(), args.dataset.resolve(), args.binary.resolve(), args.budget_seconds,
                       args.timeout, pcbnew, args.pad_to_pad or args.near_objective
                       or args.pad_near_max_mm is not None or refine is not None, refine,
                       near_objective=args.near_objective, pad_near_max_mm=args.pad_near_max_mm,
                       from_schematic=args.from_schematic, serial_archive_scoring=args.serial_archive_scoring,
                       rail_drop_simulator=args.rail_drop_simulator.resolve() if args.rail_drop_simulator else None)
    except (ImportError, ValueError, OSError, RuntimeError, KeyError, subprocess.TimeoutExpired) as error:
        parser.exit(2, f"{error}\n")
    print(f"{'complete' if complete else 'incomplete'} July comparison. See {args.work / 'results.json'}.")
    return 0 if complete else 1


if __name__ == "__main__":
    sys.exit(main())
