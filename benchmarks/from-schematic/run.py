#!/usr/bin/env python3
# Copyright (C) 2026 Toit contributors.
"""Schematic to board: every KiCad demo whose footprints are all found.

    benchmarks/from-schematic/run.py <output> [--only NAME...] [--jobs N] [--binary PATH]
    benchmarks/from-schematic/run.py <output> --schematic FILE --constraints FILE --layers N
    benchmarks/from-schematic/run.py <output> --schematic FILE --constraints FILE --layers N --candidate-board FILE
    benchmarks/from-schematic/run.py <output> --schematic FILE --constraints FILE --layers N --prepare-only

Each demo runs `import-kicad-netlist` from the
demo's schematic, `layout-kicad-board` with the starter `layout.json` it
writes (every part placed, outline sized from the parts, plug-in connectors
on an edge), unchanged, on as many copper layers as the designer used. A
board passes when every connection is routed and KiCad finds no unconnected
item, no DRC error (warnings, such as a library footprint's silkscreen on
its own pads, are reported apart) and no schematic parity issue. The
designer's own board is scored alongside for size, vias and copper.

With --schematic, run one explicit mechanical brief instead. The
supplied constraints replace the starter, and --outline-board supplies
only the board-level Edge.Cuts shapes. Report every missed constraint and
exit 1 on a failed layout or verification. With --candidate-board, check
the supplied PCB against the same brief without placing or routing it.
With --prepare-only, write the portable schematic and mechanical inputs.
With --defer-zone-fill, omit saving verified fills during a single layout.
With --plane-escape-budget, reserve part of the single layout's total work for
a retry with protected plane-pad escapes.
Use --footprints-from to take footprint geometry from a supplied board.
Use --keep-footprint-clearances to preserve supplied footprint clearance policy.
Use --rule-areas-from-outline to transfer the outline board's keepout polygons."""

import argparse
import collections
import hashlib
import json
import shutil
import subprocess
import sys
import time
from concurrent.futures import ThreadPoolExecutor
from pathlib import Path

HERE = Path(__file__).resolve().parent
ROOT = HERE.parents[1]
sys.path.insert(0, str(HERE.parent / "agent-tasks"))
sys.path.insert(0, str(HERE.parent))
import harness_common  # noqa: E402
import exchange_package  # noqa: E402
import outline_rule_areas  # noqa: E402
import quality  # noqa: E402
import returned_board  # noqa: E402
import stored_copper  # noqa: E402
from schematic import append_items, edge_cuts  # noqa: E402

DEMOS = ROOT / "benchmarks/real/external/_downloads/kicad-src/demos"
# Demos whose footprints the libraries have (the simulation demos have
# none; jetson, One-Air-Max, RoyalBlue54L and Q17ng miss some; vme-wren's
# 1507 parts are out of reach).
BOARDS = ["ecc83/ecc83-pp", "ecc83/ecc83-pp_v2", "sonde xilinx/sonde xilinx", "interf_u/interf_u",
          "pic_programmer/pic_programmer", "complex_hierarchy/complex_hierarchy", "cm5_minima/CM5_MINIMA_3",
          "stickhub/StickHub", "multichannel/multichannel_mixer", "kit-dev-coldfire-xilinx_5213/kit-dev-coldfire-xilinx_5213",
          "video/video"]


class WorkflowWallTime:
    """Per-board monotonic time, with one active stage at a time."""

    def __init__(self, stage="input_preparation"):
        self.started = self.since = time.monotonic_ns()
        self.stage = stage
        self.stages = {}

    def enter(self, stage):
        now = time.monotonic_ns()
        self.stages[self.stage] = (now - self.since) / 1e9
        self.stage, self.since = stage, now

    def finish(self, row):
        now = time.monotonic_ns()
        self.stages[self.stage] = (now - self.since) / 1e9
        row["wall_time_seconds"] = {
            "total": (now - self.started) / 1e9,
            "stages": self.stages,
        }
        return row


def board(arguments, entry):
    timing = WorkflowWallTime("demo_discovery")
    folder, name = entry.split("/")
    matches = [path for path in DEMOS.rglob(f"{name}.kicad_sch") if path.parent.name == folder or folder in str(path.parent)]
    row = {"name": name}
    if not matches:
        row["error"] = "schematic not found"
        return timing.finish(row)
    schematic = matches[0]
    work = arguments.output / name
    shutil.rmtree(work, ignore_errors=True)
    timing.enter("designer_scoring")
    designer = quality.score(arguments.binary, schematic.parent, name, stored=True) or {}
    theirs = designer.get("economy")
    layers = theirs["copper_layers"] if theirs else 2
    timing.enter("input_preparation")
    return run_board(arguments, schematic, name, work, layers, theirs, timing=timing)


def read_placer_config(path):
    raw = path.read_bytes()
    settings = json.loads(raw)
    if not isinstance(settings, dict):
        raise ValueError("--placer-config must contain a JSON object")
    if settings.get("constraints", "constraints.json") != "constraints.json":
        raise ValueError("--placer-config constraints must use the staged constraints.json supplied by --constraints")
    json.dumps(settings, allow_nan=False)
    settings["constraints"] = "constraints.json"
    return settings, {"path": str(path), "sha256": hashlib.sha256(raw).hexdigest()}


def accepted_reuse_coverage(layout, verdict, unconnected):
    routed = layout["routed"]
    reuse = routed.get("route_reuse")
    if not isinstance(reuse, dict) or reuse.get("status") != "accepted":
        return None
    outcome = verdict.get("route_reuse")
    if isinstance(outcome, dict) and outcome.get("status") != "accepted":
        return None
    coverage = {
        "pending_routed_connections": routed["routed_connections"],
        "pending_routable_connections": routed["routable_connections"],
        "kicad_unconnected_items": unconnected,
    }
    if isinstance(reuse.get("matched_nets"), list):
        coverage["retained_nets"] = len(reuse["matched_nets"])
    if unconnected == 0 and routed["native"].get("schematic_parity_issues") == 0:
        # Older layout reports put the saved-board inventory under placement.
        saved = outcome.get("saved_routable_nets") if isinstance(outcome, dict) else None
        placement = layout.get("route_reuse_placement")
        if saved is None and isinstance(placement, dict):
            saved = placement.get("saved_routable_nets")
        if type(saved) is int and saved >= 0:
            coverage["saved_routable_nets"] = saved
    return coverage


def run_board(arguments, schematic, name, work, layers, theirs=None, *, placer=None, timing=None):
    timing = timing or WorkflowWallTime()
    row = _run_layout(arguments, schematic, name, work, layers, theirs, placer, timing)
    return timing.finish(row)


def has_fillable_zones(board):
    """Recognize copper zones on the board or a footprint, excluding rule areas."""
    items = list(returned_board.children(board))
    zones = [zone for kind, zone in items if kind == "zone"]
    for kind, part in items:
        if kind in {"footprint", "module"}:
            zones.extend(zone for kind, zone in returned_board.children(part) if kind == "zone")
    for zone in zones:
        fields = dict(returned_board.children(zone))
        layer = fields.get("layers", fields.get("layer", ""))
        fill = fields.get("fill", "").rstrip(")").split()[1:2]
        if ("keepout" not in fields and fill != ["no"]
            and any(name.strip('"').endswith(".Cu") for name in layer.rstrip(")").split()[1:])):
            return True
    return False


def saved_layout_copper(board, drc, publication, deferred, timeout, timing):
    version = drc.get("kicad_version")
    verified = stored_copper.verified_fill(board, publication, version)
    if verified is not None:
        return verified
    try:
        if not has_fillable_zones(board.read_text()):
            actual = stored_copper.counts(drc, board, version)
            if actual is not None:
                return {**actual, "source": "no_fillable_zones", "board_sha256": harness_common.file_sha256(board)}
        if deferred:
            return {"source": "deferred"}
    except (OSError, ValueError):
        return None
    timing.enter("stored_verification")
    actual, _ = stored_copper.check(board, version, timeout=min(timeout, 300))
    return actual


def _run_layout(arguments, schematic, name, work, layers, theirs, placer, timing):
    row = {"name": name}
    project = work / "project"
    mechanical = arguments.schematic is not None
    if mechanical:
        row.update({"pass": False, "stage": "import"})
        project.mkdir()
        shutil.copyfile(arguments.constraints, project / "constraints.json")
        try:
            evidence, _ = returned_board.brief_evidence(arguments, schematic, layers)
            row.update(evidence)
        except (ArithmeticError, ValueError, OSError) as error:
            row["brief_evidence_error"] = str(error)
            if getattr(arguments, "rule_areas_from_outline", False):
                row["error"] = f"cannot record the outline rule areas: {error}"
                return row
            if getattr(arguments, "footprints_from", None) is not None:
                row["error"] = f"cannot record the footprint donor brief: {error}"
                return row
            if getattr(arguments, "stackup_from", None) is not None:
                row["error"] = f"cannot record the stackup donor brief: {error}"
                return row
        row["layers"] = layers
    started = time.monotonic()
    timing.enter("schematic_import")
    try:
        with returned_board.board_donor_arguments(arguments, row, work) as donor_args:
            imported = harness_common.run_group(
                [str(arguments.binary), "import-kicad-netlist", str(schematic), str(project), name,
                 "--layers", str(layers), *donor_args, *returned_board.footprint_clearance_arguments(arguments)],
                capture_output=True, text=True, timeout=arguments.timeout)
    except subprocess.TimeoutExpired:
        row["error"] = f"import timed out after {arguments.timeout} s"
        return row
    except (ValueError, OSError) as error:
        row["error"] = str(error)
        return row
    if mechanical:
        (work / "import.log").write_text(imported.stdout + imported.stderr)
        row["import_exit_code"] = imported.returncode
    try:
        report = json.loads(imported.stdout)
    except ValueError:
        report = None
    if mechanical and isinstance(report, dict):
        (work / "import.json").write_text(imported.stdout)
        row["import_report"] = "import.json"
        row["parts"] = report.get("footprints")
        row["missing"] = report.get("missing_parts", len(report.get("missing", [])))
    if imported.returncode:
        error = imported.stderr.strip()
        row["error"] = (error[:300] if mechanical else error[-300:]) or f"import exited {imported.returncode}"
        return row
    if not isinstance(report, dict) or "footprints" not in report or "missing" not in report:
        row["error"] = "import has no readable footprint report"
        return row
    try:
        returned_board.check_imported_clearance_policy(report, row)
        returned_board.check_imported_stackup(report, row, project / f"{name}.kicad_pcb")
    except (ArithmeticError, ValueError, OSError) as error:
        row["error"] = str(error)
        return row
    if report.get("footprint_clearance_policy"):
        row["footprint_clearance_policy"] = report["footprint_clearance_policy"]
    row["parts"] = report["footprints"]
    if not mechanical:
        row["missing"] = len(report["missing"])
    timing.enter("layout_preparation")
    layout_config = project / "layout.json"
    donor_rules = []
    if mechanical:
        row["stage"] = "constraints"
        if arguments.outline_board:
            try:
                shapes = edge_cuts(arguments.outline_board.read_text())
                donor_rules = outline_rule_areas.donor_forms(arguments, row, layers)
                if donor_rules:
                    existing = (project / f"{name}.kicad_pcb").read_text()
                    outline_rule_areas.ensure_unique_ids(existing + "".join(shapes), donor_rules)
            except (ArithmeticError, ValueError, OSError) as error:
                row["error"] = f"cannot read the outline board: {error}"
                return row
            if not shapes:
                row["error"] = "outline board has no board-level Edge.Cuts shapes"
                return row
            append_items(project / f"{name}.kicad_pcb", shapes + donor_rules)
            row["outline_shapes"] = len(shapes)
        defer_zone_fill = getattr(arguments, "defer_zone_fill", False)
        if arguments.total_seconds is not None or placer is not None or defer_zone_fill:
            try:
                settings = json.loads(layout_config.read_text())
                if arguments.total_seconds is not None:
                    settings["total_seconds"] = arguments.total_seconds
                if placer is not None:
                    settings["placer"] = {**settings.get("placer", {}), **placer}
                if defer_zone_fill:
                    settings["save_zone_fills"] = False
                    row["save_zone_fills"] = False
                layout_config.write_text(json.dumps(settings, indent=2) + "\n")
                if placer is not None or defer_zone_fill:
                    row["layout_config"] = {"path": "project/layout.json", "sha256": harness_common.file_sha256(layout_config)}
            except (ValueError, OSError, TypeError, AttributeError) as error:
                row["error"] = f"cannot configure layout: {error}"
                return row
        row["stage"] = "layout"
    router_config = "auto"
    if mechanical and arguments.router_config:
        router_config = str(arguments.router_config)
        row["router_config"] = returned_board.record(arguments.router_config)
    reuse = getattr(arguments, "reuse_routes", None) if mechanical else None
    reuse_option = []
    if reuse is not None:
        try:
            row["reuse_routes_input"] = returned_board.record(reuse)
        except OSError as error:
            row["error"] = f"cannot read the previous route board: {error}"
            return row
        reuse_option = ["--reuse-routes", str(reuse)]
    if mechanical:
        for retry in ("plane_escape", "terminal_launch"):
            reserve = getattr(arguments, f"{retry}_budget", None)
            if reserve is not None:
                reuse_option += [f"--{retry.replace('_', '-')}-budget", str(reserve)]
                row[f"{retry}_reserved_seconds"] = reserve
    timing.enter("layout")
    layout_log = work / "layout.log"
    try:
        with layout_log.open("w") as log:
            process = harness_common.run_group(
                [str(arguments.binary), "layout-kicad-board", str(project), name, str(work / "layout"),
                 router_config, str(layout_config), *reuse_option],
                stdout=log, stderr=subprocess.STDOUT, timeout=arguments.timeout)
    except subprocess.TimeoutExpired:
        row["error"] = f"timed out after {arguments.timeout} s"
        return row
    row["seconds"] = round(time.monotonic() - started)
    timing.enter("result_readback")
    result_path = work / "layout/board-layout.json"
    verdict = harness_common.read_json(work / "layout/verdict.json")
    if not isinstance(verdict, dict):
        verdict = {}
    if mechanical:
        row["layout_exit_code"] = process.returncode
        if verdict:
            row["verdict"] = verdict.get("status")
            row["verdict_path"] = "layout/verdict.json"
            row["kicad_checked"] = verdict.get("kicad_checked") is True
            if process.returncode != 0:
                if verdict.get("reason"):
                    row["layout_reason"] = verdict["reason"]
                if verdict.get("next_actions"):
                    row["next_actions"] = verdict["next_actions"]
    result = harness_common.read_json(result_path)
    if not isinstance(result, dict) or not isinstance(result.get("routed"), dict):
        row["error"] = verdict.get("reason")
        if not row["error"]:
            with layout_log.open(errors="replace") as log:
                last = ""
                for line in log:
                    if line.strip():
                        last = line.rstrip()
            row["error"] = last.lstrip()[:300] or "no layout report"
        if mechanical:
            for key in ("unplaced", "room"):
                if verdict.get(key):
                    row[key] = verdict[key]
            placement_path = "layout/placed/board-placer.json"
            placement = harness_common.read_json(work / placement_path)
            if isinstance(placement, dict):
                row["placement_report"] = placement_path
                if isinstance(placement.get("constraints"), list):
                    row["placement_constraints"] = placement["constraints"]
        return row
    routed = result["routed"]
    if mechanical and isinstance(result.get("fill_publication"), dict):
        row["fill_publication"] = result["fill_publication"]
    if mechanical:
        for key in ("plane_escape", "terminal_launch_retry"):
            if isinstance(result.get(key), dict):
                row[key] = result[key]
    if reuse is not None:
        reused = routed.get("route_reuse")
        if not isinstance(reused, dict) or reused.get("sha256") != row["reuse_routes_input"]["sha256"]:
            row["error"] = "layout has no matching route reuse provenance"
            return row
        row["route_reuse"] = reused
        row["route_reuse_placement"] = result.get("route_reuse_placement")
        if routed.get("kept_copper"):
            row["kept_copper"] = routed["kept_copper"]
    done = routed.get("routed_connections")
    needed = routed.get("routable_connections")
    native = routed.get("native")
    drc = harness_common.read_json(work / "layout/result/drc.json")
    if (not isinstance(native, dict) or not isinstance(drc, dict)
        or not isinstance(drc.get("violations"), list)
        or (mechanical and not isinstance(drc.get("unconnected_items"), list))):
        row["error"] = "layout has no readable KiCad verification report"
        return row
    if not isinstance(done, int) or not isinstance(needed, int):
        row["error"] = "layout has no readable connection counts"
        return row
    if mechanical:
        row["stage"] = "verification"
    keep_footprint_clearances = row.get("footprint_clearance_policy") == "preserve_supplied"
    if donor_rules or "expected_stackup" in row or keep_footprint_clearances:
        try:
            saved = (work / "layout/result" / f"{name}.kicad_pcb").read_text()
            if donor_rules:
                row["rule_area_check"] = outline_rule_areas.check(donor_rules, saved, layers)
            if "expected_stackup" in row:
                row["stackup_check"] = returned_board.check_stackup(row["expected_stackup"], saved, project / f"{name}.kicad_pcb")
            if keep_footprint_clearances:
                imported_items = returned_board.children((project / f"{name}.kicad_pcb").read_text())
                row["footprint_clearance_check"] = returned_board.check_footprint_clearances(
                    imported_items, returned_board.children(saved))
        except (ArithmeticError, ValueError, OSError) as error:
            row["error"] = f"cannot check the saved board against the authored metadata: {error}"
            return row
    severities = collections.Counter(violation["severity"] for violation in drc["violations"])
    unconnected = len(drc.get("unconnected_items", []))
    coverage = accepted_reuse_coverage(result, verdict, unconnected)
    if coverage is not None:
        row["routing_coverage"] = coverage
    row.update({
        "routed": f"{done}/{needed}",
        "unconnected": unconnected,
        "drc_errors": severities.get("error", 0),
        "drc_warnings": severities.get("warning", 0),
        "parity": native.get("schematic_parity_issues"),
        "erc": native.get("erc_violations"),
    })
    quality = result.get("quality") or {}
    economy = quality.get("economy") or {}
    row["ours"] = {"size_mm": [economy.get("width_mm"), economy.get("height_mm")], "vias": economy.get("vias"),
                   "copper_mm": economy.get("track_length_mm")}
    if theirs:
        row["designer"] = {"size_mm": [theirs.get("width_mm"), theirs.get("height_mm")], "vias": theirs.get("vias"),
                           "copper_mm": theirs.get("track_length_mm"), "layers": layers}
    row["pass"] = ((mechanical or done == needed) and row["unconnected"] == 0
                   and row["drc_errors"] == 0 and row["parity"] == 0)
    if mechanical:
        row["total_seconds"] = result.get("total_seconds")
        row["work_seconds"] = verdict.get("work_seconds", result.get("work_seconds"))
        if verdict.get("status") == "complete" and row["drc_errors"]:
            row["native_errors_by_type"] = dict(collections.Counter(
                finding.get("type", "unknown") for finding in drc["violations"] if finding["severity"] == "error"))
            for field in ("footprint_findings", "preexisting_findings"):
                if verdict.get(field):
                    row[field] = verdict[field]
            row.setdefault("next_actions", []).append(
                f"The layout verdict is complete. The benchmark counts all {row['drc_errors']} native DRC errors. "
                "Inspect their items and measured limits in layout/result/drc.json and compare the layout's "
                "classification in layout/verdict.json before revising the design."
            )
        row["constraints"] = result.get("constraints")
        if not verdict or not isinstance(row["constraints"], list):
            row.update({"pass": False, "error": "layout has no readable constraint verdict"})
            return row
        row["work_rate"] = verdict.get("work_rate")
        row["placer_work_rate"] = verdict.get("placer_work_rate")
        row["constraints_missed"] = verdict.get("constraints_missed", [])
        if row["constraints_missed"] and verdict.get("next_actions"):
            row["next_actions"] = verdict["next_actions"]
        if row["parity"]:
            returned_board.explain_parity(row, drc, report, "layout/result/drc.json")
        row["pass"] &= (process.returncode == 0 and verdict.get("status") == "complete"
                        and verdict.get("kicad_checked") is True
                        and row.get("rule_area_check", {}).get("satisfied", True)
                        and row.get("stackup_check", {}).get("satisfied", True)
                        and row.get("footprint_clearance_check", {}).get("satisfied", True)
                        and not row["constraints_missed"]
                        and all(harness_common.constraint_satisfied(constraint) for constraint in row["constraints"]))
        if not row.get("stackup_check", {}).get("satisfied", True):
            row["error"] = "the saved PCB does not preserve the authored stackup"
        if not row.get("footprint_clearance_check", {}).get("satisfied", True):
            row["error"] = "the saved PCB changes supplied footprint clearances"
        if row["pass"]:
            board = work / "layout/result" / f"{name}.kicad_pcb"
            if board.is_file():
                try:
                    returned_board.check_zone_ids(row, returned_board.children(board.read_text()))
                except (ValueError, OSError) as error:
                    row.update({"pass": False, "error": f"cannot read saved board zone identities: {error}"})
                    return row
                if row.get("duplicate_zone_uuids"):
                    row.update({"pass": False, "error": "the saved board repeats a zone UUID"})
                    return row
            publication = verdict.get("fill_publication", row.get("fill_publication"))
            stored = saved_layout_copper(board, drc, publication, defer_zone_fill, arguments.timeout, timing)
            row["stored_copper"] = stored
            if stored is None:
                row.update({"pass": False, "error": "no native verification of the PCB as stored"})
            elif stored["source"] == "deferred":
                row["stage"] = "routing_iteration"
            elif any(stored[key] for key in ("unconnected", "parity", "errors")):
                row.update({"pass": False, "error": f"saved PCB has {stored['unconnected']} opens, "
                            f"{stored['errors']} DRC errors and {stored['parity']} schematic parity findings"})
                if stored["unconnected"] and stored["source"] == "fresh_as_stored_drc":
                    out = work / "verified-filled-project"
                    stored["next_action"] = {
                        "kind": "save_and_check_fills",
                        "argv": [str(arguments.binary), "drc-kicad-board", str(board.parent), name,
                                 str(work / "drc-before-fill.json"), "--save-filled", str(out)],
                        "board": str(out / board.name), "report": str(out / "fill-handoff.json"),
                    }
            else:
                row["stage"] = "complete"
    return row


def main():
    parser = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    parser.add_argument("output", type=Path)
    parser.add_argument("--only", nargs="*")
    parser.add_argument("--jobs", type=int, default=1)
    parser.add_argument("--timeout", type=int, default=3600)
    parser.add_argument("--binary", type=Path, default=ROOT / "target/release/pcb-maker")
    parser.add_argument("--schematic", type=Path, help="run a single supplied KiCad schematic")
    parser.add_argument("--constraints", type=Path, help="placement constraints for the supplied schematic")
    parser.add_argument("--layers", type=int, help="copper layer count for the supplied schematic")
    parser.add_argument("--outline-board", type=Path, help="take only board-level Edge.Cuts from this board")
    parser.add_argument("--rule-areas-from-outline", action="store_true",
                        help="also take the outline board's board-level keepout polygons")
    parser.add_argument("--footprints-from", type=Path, help="take footprint geometry from this board")
    parser.add_argument("--mechanical-donor", type=Path,
                        help="verify explicitly added board-only parts against this PCB (candidate scoring only)")
    parser.add_argument("--keep-footprint-clearances", action="store_true",
                        help="preserve supplied footprint clearance settings in import and candidate scoring")
    parser.add_argument("--stackup-from", type=Path, help="take the authored fabrication stackup and thickness from this board")
    parser.add_argument("--total-seconds", type=float, help="layout budget in seconds of work for the single schematic")
    parser.add_argument("--router-config", type=Path, help="router options for a single-schematic layout (default: auto)")
    parser.add_argument("--placer-config", type=Path, help="placer options for a single-schematic layout or prepared project")
    parser.add_argument("--reuse-routes", type=Path, help="try matching copper from this previous board under the current rules")
    recovery = parser.add_mutually_exclusive_group()
    recovery.add_argument("--plane-escape-budget", type=float,
                          help="part of --total-seconds reserved for a guarded paired plane escape retry")
    recovery.add_argument("--terminal-launch-budget", type=float,
                          help="part of --total-seconds reserved for a guarded retry of blocked footprint pads")
    parser.add_argument("--defer-zone-fill", action="store_true",
                        help="defer saving verified zone fills in a single-schematic layout")
    modes = parser.add_mutually_exclusive_group()
    modes.add_argument("--candidate-board", type=Path, help="verify a returned .kicad_pcb against the brief without laying it out")
    modes.add_argument("--prepare-only", action="store_true", help="prepare the project and mechanical inputs without layout")
    arguments = parser.parse_args()
    arguments.output = arguments.output.resolve()
    if arguments.schematic:
        timing = WorkflowWallTime()
        if arguments.only is not None or arguments.constraints is None or (arguments.layers is None and arguments.stackup_from is None):
            parser.error("--schematic takes --constraints and either --layers or --stackup-from and runs a single schematic")
        for key in ("schematic", "constraints", "outline_board", "footprints_from", "mechanical_donor",
                    "stackup_from", "binary", "candidate_board", "router_config", "placer_config", "reuse_routes"):
            path = getattr(arguments, key)
            if path is not None:
                if not path.is_file():
                    parser.error(f"--{key.replace('_', '-')} {path} is not a file")
                setattr(arguments, key, path.resolve())
        if arguments.rule_areas_from_outline and arguments.outline_board is None:
            parser.error("--rule-areas-from-outline needs --outline-board")
        if arguments.schematic.suffix != ".kicad_sch":
            parser.error("--schematic needs a .kicad_sch file for the parity check")
        if arguments.footprints_from and arguments.footprints_from.suffix != ".kicad_pcb":
            parser.error("--footprints-from needs a .kicad_pcb file")
        if arguments.mechanical_donor:
            if not arguments.candidate_board:
                parser.error("--mechanical-donor only applies when scoring a --candidate-board")
            if arguments.mechanical_donor.suffix != ".kicad_pcb":
                parser.error("--mechanical-donor needs a .kicad_pcb file")
        if arguments.stackup_from is not None:
            if arguments.stackup_from.suffix != ".kicad_pcb":
                parser.error("--stackup-from needs a .kicad_pcb file")
            try:
                _, stackup = returned_board.read_stackup_donor(arguments.stackup_from)
            except (ArithmeticError, ValueError, OSError) as error:
                parser.error(f"cannot read --stackup-from: {error}")
            donor_layers = len(stackup["copper_layers"])
            if arguments.layers is not None and arguments.layers != donor_layers:
                parser.error(f"--stackup-from has {donor_layers} copper layers but --layers asks for {arguments.layers}")
            arguments.layers = donor_layers
        if arguments.reuse_routes and arguments.reuse_routes.suffix != ".kicad_pcb":
            parser.error("--reuse-routes needs a .kicad_pcb file")
        if arguments.reuse_routes and arguments.terminal_launch_budget is not None:
            parser.error("--terminal-launch-budget needs a cold layout without --reuse-routes")
        if arguments.candidate_board:
            if arguments.candidate_board.suffix != ".kicad_pcb":
                parser.error("--candidate-board needs a .kicad_pcb file")
            if arguments.placer_config is not None:
                parser.error("--placer-config only applies when running or preparing a layout")
        for key in ("plane_escape_budget", "terminal_launch_budget"):
            reserve = getattr(arguments, key)
            if reserve is None:
                continue
            flag = "--" + key.replace("_", "-")
            if arguments.candidate_board or arguments.prepare_only:
                parser.error(f"{flag} only applies when running a layout")
            if arguments.total_seconds is None:
                parser.error(f"{flag} needs an explicit --total-seconds")
            if not 0 < reserve < arguments.total_seconds:
                parser.error(f"{flag} needs positive seconds smaller than --total-seconds")
        if arguments.candidate_board or arguments.prepare_only:
            if arguments.reuse_routes is not None:
                parser.error("--reuse-routes only applies when running a layout")
            if arguments.defer_zone_fill:
                parser.error("--defer-zone-fill only applies when running a layout")
            if arguments.router_config is not None:
                parser.error("--router-config only applies when running a layout")
            if arguments.total_seconds is not None:
                parser.error("--candidate-board and --prepare-only do not take a layout budget")
            if not 2 <= arguments.layers <= 32 or arguments.layers % 2:
                parser.error("--layers needs an even copper-layer count from 2 to 32")
        placer = placer_source = None
        if arguments.placer_config is not None:
            try:
                placer, placer_source = read_placer_config(arguments.placer_config)
            except (ValueError, OSError) as error:
                parser.error(str(error))
        try:
            arguments.output.mkdir(parents=True)
        except FileExistsError:
            parser.error("give a new output directory for the single schematic")
        if arguments.candidate_board:
            timing.enter("candidate_verification")
            row = returned_board.verify(arguments, arguments.schematic, arguments.output, arguments.layers)
        elif arguments.prepare_only:
            timing.enter("package_preparation")
            row = exchange_package.prepare(arguments, arguments.schematic, arguments.output, arguments.layers, placer=placer)
        else:
            row = _run_layout(arguments, arguments.schematic, arguments.schematic.stem, arguments.output,
                             arguments.layers, None, placer, timing)
        if placer_source is not None:
            row["placer_config"] = placer_source
        timing.finish(row)
        harness_common.atomic_json(arguments.output / "results.json", [row], indent=1, newline=True)
        print(json.dumps(row), flush=True)
        return 0 if row["pass"] else 1
    if arguments.prepare_only or arguments.defer_zone_fill or arguments.rule_areas_from_outline or any(value is not None for value in (arguments.constraints, arguments.layers,
                                           arguments.outline_board, arguments.footprints_from, arguments.mechanical_donor,
                                           arguments.stackup_from, arguments.total_seconds, arguments.candidate_board,
                                           arguments.router_config, arguments.placer_config, arguments.reuse_routes,
                                           arguments.plane_escape_budget, arguments.terminal_launch_budget)):
        parser.error("mechanical inputs, layout options, recovery budgets, --candidate-board and --prepare-only require --schematic")
    arguments.output.mkdir(parents=True, exist_ok=True)
    boards = [entry for entry in BOARDS if not arguments.only or entry.split("/")[1] in arguments.only]
    with ThreadPoolExecutor(arguments.jobs) as pool:
        rows = []
        for row in pool.map(lambda entry: board(arguments, entry), boards):
            print(json.dumps(row), flush=True)
            rows.append(row)
    (arguments.output / "results.json").write_text(json.dumps(rows, indent=1))
    passed = sum(1 for row in rows if row.get("pass"))
    print(f"{passed}/{len(rows)} complete")


if __name__ == "__main__":
    sys.exit(main())
