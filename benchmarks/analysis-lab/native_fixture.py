#!/usr/bin/env python3
# Copyright (C) 2026 Toit contributors.
"""Build and evaluate one blinded, native KiCad routing-improvement fixture.

The native board/project and KiCad DRC remain authoritative. Geometry export is
an inspection packet, never input to the synthetic-board semantic validator.
Generator source and private outputs contain the mutation answer key.
"""

import argparse
from collections import Counter
import hashlib
import json
import math
from pathlib import Path
import shutil
import subprocess
import uuid

import pcbnew

if not hasattr(pcbnew.SwigPyIterator, "next"):
    pcbnew.SwigPyIterator.next = pcbnew.SwigPyIterator.__next__

ROOT = Path(__file__).resolve().parents[2]
SOURCE = ROOT / "build/sequence-224-m0-ecc83-formal-comparison/pcb-maker/result/ecc83-pp.kicad_pcb"
OUTPUT = ROOT / "build/analysis-lab/native"
CASE_ID = "n08"


def write_json(path, value):
    path.parent.mkdir(parents=True, exist_ok=True)
    path.write_text(json.dumps(value, indent=2) + "\n")


def sha256(path):
    return hashlib.sha256(path.read_bytes()).hexdigest()


def mm(point):
    return [point.x / 1_000_000, point.y / 1_000_000]


def iu(point):
    return pcbnew.VECTOR2I(*(round(value * 1_000_000) for value in point))


def identity(item):
    return item.m_Uuid.AsString()


def stage_project(source, directory, case_id=CASE_ID):
    directory.mkdir(parents=True, exist_ok=True)
    target = directory / f"{case_id}.kicad_pcb"
    for suffix in [".kicad_pcb", ".kicad_pro", ".kicad_sch", ".kicad_dru"]:
        original = source.with_suffix(suffix)
        if original.exists():
            shutil.copy2(original, target.with_suffix(suffix))
    for name in ["fp-lib-table", "sym-lib-table", "footprints.pretty", "3d_shapes"]:
        original = source.parent / name
        if original.is_dir():
            shutil.copytree(original, directory / name, dirs_exist_ok=True)
        elif original.is_file():
            shutil.copy2(original, directory / name)
    for original in source.parent.glob("*.kicad_sym"):
        shutil.copy2(original, directory / original.name)
    for original in source.parent.glob("*.pretty"):
        shutil.copytree(original, directory / original.name, dirs_exist_ok=True)
    return target


def drc(board_path, report_path):
    result = subprocess.run(["kicad-cli", "pcb", "drc", "--format", "json",
        "--all-track-errors", "--schematic-parity", "--refill-zones", "--output",
        str(report_path), str(board_path)], capture_output=True, text=True)
    if result.returncode != 0 or not report_path.exists():
        raise RuntimeError(f"Native DRC failed: {result.stdout}\n{result.stderr}")
    report = json.loads(report_path.read_text())
    return report, {"stdout": result.stdout.strip(), "stderr": result.stderr.strip()}


def save_board(board_path, board):
    """Save copper while preserving the exact supplied project/rules bytes.

    KiCad 10 SaveBoard also serializes default project settings; preventing that
    side effect keeps an implicit-default input project implicit and immutable.
    """
    project_path = board_path.with_suffix(".kicad_pro")
    project_bytes = project_path.read_bytes()
    pcbnew.SaveBoard(str(board_path), board)
    project_path.write_bytes(project_bytes)


def finding_signatures(report):
    return Counter(json.dumps({"section": section, "type": finding.get("type"),
        "severity": finding.get("severity"), "description": finding.get("description"),
        "items": sorted(finding.get("items", []), key=lambda item: item.get("uuid", ""))},
        sort_keys=True) for section in ["violations", "unconnected_items", "schematic_parity"]
        for finding in report.get(section, []))


def drc_summary(report):
    return {"violations": len(report.get("violations", [])),
            "errors": sum(f.get("severity") == "error" for f in report.get("violations", [])),
            "opens": len(report.get("unconnected_items", [])),
            "schematic_parity": len(report.get("schematic_parity", [])),
            "ignored_checks": report.get("ignored_checks"),
            "kicad_version": report.get("kicad_version")}


def metrics(board):
    tracks = list(board.GetTracks())
    vias = sum(t.GetClass() == "PCB_VIA" for t in tracks)
    stored_length = sum(t.GetLength() / 1_000_000 for t in tracks if t.GetClass() != "PCB_VIA")
    # Integer normalized directions and supporting-line offsets avoid float
    # equality tolerances when combining collinear same-net/layer copper.
    groups = {}
    for track in tracks:
        if track.GetClass() == "PCB_VIA":
            continue
        if track.GetClass() != "PCB_TRACK":
            raise ValueError("Centerline union metrics currently support straight tracks only")
        start, end = track.GetStart(), track.GetEnd()
        dx, dy = end.x-start.x, end.y-start.y
        divisor = math.gcd(dx, dy)
        if divisor == 0:
            continue
        ux, uy = dx//divisor, dy//divisor
        if ux < 0 or (ux == 0 and uy < 0):
            ux, uy = -ux, -uy
        key = (track.GetNetCode(), track.GetLayer(), ux, uy, ux*start.y-uy*start.x)
        interval = sorted([ux*start.x+uy*start.y, ux*end.x+uy*end.y])
        groups.setdefault(key, []).append(interval)
    length = 0.0
    for key, intervals in groups.items():
        intervals.sort()
        begin, end = intervals[0]
        union_projection = 0
        for next_begin, next_end in intervals[1:]:
            if next_begin <= end:
                end = max(end, next_end)
            else:
                union_projection += end-begin
                begin, end = next_begin, next_end
        union_projection += end-begin
        length += union_projection/math.hypot(key[2], key[3])/1_000_000
    return {"stored_length_mm": stored_length, "length_mm": length,
            "vias": vias, "cost_mm": length + 2*vias}


def resolved_pad_clearances(board, pad, copper_layers):
    result = {}
    for layer in copper_layers:
        if pad.IsOnLayer(layer):
            value = pad.GetOwnClearance(layer)/1_000_000
            if not math.isfinite(value) or value < 0:
                raise ValueError("Native pad own clearance did not resolve to a nonnegative value")
            result[board.GetLayerName(layer)] = {"value_mm": value,
                "source": "pcbnew.PAD.GetOwnClearance(layer)", "source_layer_id": layer}
    return result


def native_pad_shape(pad):
    shapes = {pcbnew.PAD_SHAPE_CIRCLE: "circle", pcbnew.PAD_SHAPE_RECT: "rect",
              pcbnew.PAD_SHAPE_OVAL: "oval", pcbnew.PAD_SHAPE_ROUNDRECT: "roundrect"}
    if pad.GetShape() == pcbnew.PAD_SHAPE_TRAPEZOID:
        delta = pad.GetDelta()
        if delta.x != 0 or delta.y != 0:
            raise ValueError("Nonzero-delta native trapezoid pads remain unsupported")
        return {"shape": "rect", "source_shape": "trapezoid", "source_shape_delta": [0, 0],
                "shape_equivalence": "Native zero trapezoid delta is exactly rectangular; native board was not modified."}
    if pad.GetShape() not in shapes:
        raise ValueError("Fixture export does not support this native pad shape")
    return {"shape": shapes[pad.GetShape()]}


def nonrouting_rule_areas(board):
    result = []
    for area in board.Zones():
        if not area.GetIsRuleArea():
            raise ValueError("Actual copper zones remain unsupported by the inspection exporter")
        flags = {name: bool(getattr(area, method)()) for name, method in {
            "do_not_allow_tracks": "GetDoNotAllowTracks", "do_not_allow_vias": "GetDoNotAllowVias",
            "do_not_allow_pads": "GetDoNotAllowPads", "do_not_allow_footprints": "GetDoNotAllowFootprints",
            "do_not_allow_zone_fills": "GetDoNotAllowZoneFills"}.items()}
        if flags["do_not_allow_tracks"] or flags["do_not_allow_vias"]:
            raise ValueError("Routing-restrictive native rule areas remain unsupported")
        polygons = area.Outline()
        def contour(chain):
            if chain.ArcCount():
                raise ValueError("Curved rule-area boundary metadata remains unsupported")
            return [mm(chain.CPoint(index)) for index in range(chain.PointCount())]
        outlines = [{"points": contour(polygons.COutline(index)),
                     "holes": [contour(polygons.CHole(index, hole)) for hole in range(polygons.HoleCount(index))]}
                    for index in range(polygons.OutlineCount())]
        result.append({"uuid": identity(area), "is_rule_area": True,
            "layers": [board.GetLayerName(layer) for layer in area.GetLayerSet().Seq()],
            **flags, "outlines": outlines,
            "source": "pcbnew.ZONE rule-area flags and exact linear contours",
            "scope": "Metadata only: area allows tracks/vias, has no copper fill, and is not a copper-clearance blocker."})
    return result


def export_geometry(board_path):
    board = pcbnew.LoadBoard(str(board_path))
    copper_layers = list(board.GetEnabledLayers().CuStack())
    layer_name = lambda layer: board.GetLayerName(layer)
    track_records, via_records, pad_records = [], [], []
    for track in sorted(board.GetTracks(), key=identity):
        common = {"uuid": identity(track), "net": f"N{track.GetNetCode()}"}
        if track.GetClass() == "PCB_VIA":
            via_records.append({"id": f"V{len(via_records)+1}", **common,
                "at": mm(track.GetPosition()), "diameter": track.GetWidth(track.TopLayer())/1_000_000,
                "drill": track.GetDrillValue()/1_000_000, "via_type": int(track.GetViaType()),
                "layers": [layer_name(layer) for layer in copper_layers if track.IsOnLayer(layer)]})
        elif track.GetClass() == "PCB_TRACK":
            track_records.append({"id": f"T{len(track_records)+1}", **common,
                "start": mm(track.GetStart()), "end": mm(track.GetEnd()),
                "width": track.GetWidth()/1_000_000, "layer": layer_name(track.GetLayer())})
        else:
            raise ValueError("Fixture export currently requires straight native tracks")
    for footprint in sorted(board.GetFootprints(), key=identity):
        for pad in sorted(footprint.Pads(), key=identity):
            pad_record = {"id": f"P{len(pad_records)+1}", "uuid": identity(pad),
                "net": f"N{pad.GetNetCode()}", "at": mm(pad.GetPosition()),
                **native_pad_shape(pad), "size": mm(pad.GetSize()),
                "rotation_degrees": pad.GetOrientationDegrees(), "offset": mm(pad.GetOffset()),
                "drill": mm(pad.GetDrillSize()), "drill_shape": int(pad.GetDrillShape()),
                "attribute": int(pad.GetAttribute()),
                "layers": [layer_name(layer) for layer in copper_layers if pad.IsOnLayer(layer)],
                "own_clearance_by_layer": resolved_pad_clearances(board, pad, copper_layers),
                "footprint": footprint.GetReference(), "pad_number": pad.GetNumber()}
            if pad.GetShape() == pcbnew.PAD_SHAPE_ROUNDRECT:
                pad_record["roundrect_radius"] = pad.GetRoundRectCornerRadius()/1_000_000
                pad_record["roundrect_radius_ratio"] = pad.GetRoundRectRadiusRatio()
            pad_records.append(pad_record)
    outline = []
    for drawing in board.GetDrawings():
        if drawing.GetLayer() == pcbnew.Edge_Cuts:
            first, last = mm(drawing.GetStart()), mm(drawing.GetEnd())
            if drawing.GetShapeStr() == "Line":
                outline.append({"id": identity(drawing), "start": first,
                                "end": last, "width": drawing.GetWidth()/1_000_000})
            elif drawing.GetShapeStr() == "Rect":
                corners = [first, [last[0], first[1]], last, [first[0], last[1]], first]
                for i, (start, end) in enumerate(zip(corners, corners[1:])):
                    outline.append({"id": f"{identity(drawing)}:{i}", "source_uuid": identity(drawing),
                        "source_shape": "rect", "start": start, "end": end,
                        "width": drawing.GetWidth()/1_000_000})
            else:
                raise ValueError("Fixture export currently requires line or rectangle board edges")
    coordinates = [p for edge in outline for p in [edge["start"], edge["end"]]]
    rule_areas = nonrouting_rule_areas(board)
    project = json.loads(board_path.with_suffix(".kicad_pro").read_text())
    nets = {f"N{item.GetNetCode()}": item.GetNetname() for item in board.GetTracks()}
    explicit_classes = {item["name"] for item in project.get("net_settings", {}).get("classes", [])}
    effective_netclasses = {name: {
        "clearance": netclass.GetClearance()/1_000_000,
        "preferred_trace_width": netclass.GetTrackWidth()/1_000_000,
        "preferred_via_diameter": netclass.GetViaDiameter()/1_000_000,
        "preferred_via_drill": netclass.GetViaDrill()/1_000_000,
        "source": "project netclass resolved by pcbnew" if name in explicit_classes else "KiCad default resolved by pcbnew"}
        for name, netclass in board.GetAllNetClasses().items()}
    settings = board.GetDesignSettings()
    explicit_rules = project.get("board", {}).get("design_settings", {}).get("rules", {})
    effective_board_rules = {name: {"value": getattr(settings, native_name)/1_000_000,
        "source": "project rule resolved by pcbnew" if name in explicit_rules else "KiCad default resolved by pcbnew"}
        for name, native_name in {"min_clearance": "m_MinClearance", "min_track_width": "m_TrackMinWidth",
            "min_via_diameter": "m_ViasMinSize", "min_via_annular_width": "m_ViasMinAnnularWidth",
            "min_through_hole_diameter": "m_MinThroughDrill", "min_copper_edge_clearance": "m_CopperEdgeClearance",
            "min_hole_clearance": "m_HoleClearance", "min_hole_to_hole": "m_HoleToHoleMin"}.items()}
    return {"schema_version": 1, "format": "analysis-lab-native-geometry", "id": board_path.stem,
        "units": "mm", "coordinate_system": "Native KiCad absolute XY; +Y down; angles are native pad orientation degrees.",
        "bounds": [min(p[0] for p in coordinates), min(p[1] for p in coordinates),
                   max(p[0] for p in coordinates), max(p[1] for p in coordinates)],
        "layers": [{"id": layer, "name": layer_name(layer)} for layer in copper_layers],
        "nets": nets, "tracks": track_records, "vias": via_records, "pads": pad_records,
        "edge_cuts": outline, "zones": [], "rule_areas": rule_areas, "metrics": metrics(board),
        "net_settings": project.get("net_settings"),
        "board_rules": project.get("board", {}).get("design_settings", {}).get("rules"),
        "effective_netclasses": effective_netclasses,
        "netclasses_by_net": {f"N{item.GetNetCode()}": item.GetNetClassName() for item in board.GetTracks()},
        "effective_board_rules": effective_board_rules,
        "rule_notes": "Values in mm. Netclass preferred dimensions are defaults for new routing, not width-preservation permissions or DRC minimums. Native DRC applies full rules and overrides. Actual added track widths must match an existing width on that same net.",
        "authority": "Native .kicad_pcb plus matching .kicad_pro and native KiCad DRC. Inspection geometry is not a synthetic validator model.",
        "limitations": ["Actual copper zones and track/via-restrictive rule areas are unsupported; non-copper areas allowing tracks/vias are retained as metadata.",
            "Pad support is circle/rect/oval/roundrect and exactly zero-delta trapezoid-as-rectangle; nonzero trapezoids and custom pad shapes remain unsupported.",
            "Non-copper graphics, text, courtyards, and 3D models are omitted from inspection geometry but retained in native files.",
            "Pad sizes, rotations, offsets, attributes, drills, and copper layers are preserved; do not replace all pads with circles.",
            "Per-pad/per-layer own clearances come from the native resolved API; arbitrary custom pair rules and general hole/hole-pair constraints are not represented by this inspection model.",
            "Exporting an inspection packet does not establish a clean or accepted board. Native DRC remains the acceptance authority."],
        "native_sha256": sha256(board_path), "project_sha256": sha256(board_path.with_suffix(".kicad_pro"))}


def new_track(board, start, end, width, layer, net_code, stable_name):
    track = pcbnew.PCB_TRACK(board)
    track.SetStart(iu(start)); track.SetEnd(iu(end))
    track.SetWidth(round(width*1_000_000)); track.SetLayer(layer); track.SetNetCode(net_code)
    track.SetUuid(pcbnew.KIID(str(uuid.uuid5(uuid.NAMESPACE_URL, stable_name))))
    board.Add(track)
    return track


def mutate(board, track_id, fractions, via_dimensions=(1.2, .6)):
    track = next(t for t in board.GetTracks() if identity(t) == track_id)
    original = {"uuid": track_id, "start": mm(track.GetStart()), "end": mm(track.GetEnd()),
                "width": track.GetWidth()/1_000_000, "layer": track.GetLayer(), "net_code": track.GetNetCode()}
    a, b = original["start"], original["end"]
    cuts = [[a[j]+fraction*(b[j]-a[j]) for j in range(2)] for fraction in fractions]
    cuts = [mm(iu(p)) for p in cuts]
    points = [a, *cuts, b]
    opposite = pcbnew.B_Cu if original["layer"] == pcbnew.F_Cu else pcbnew.F_Cu
    board.Remove(track)
    track.thisown = False  # See the KiCad/SWIG lifetime workaround in evaluate.
    created_tracks, created_vias = [], []
    for i, (start, end) in enumerate(zip(points, points[1:])):
        item = new_track(board, start, end, original["width"],
            opposite if i == 1 else original["layer"], original["net_code"],
            f"analysis-lab/{track_id}/{fractions}/{i}")
        created_tracks.append(identity(item))
    for i, position in enumerate(cuts):
        via = pcbnew.PCB_VIA(board)
        via.SetPosition(iu(position)); via.SetWidth(round(via_dimensions[0]*1_000_000))
        via.SetDrill(round(via_dimensions[1]*1_000_000))
        via.SetViaType(pcbnew.VIATYPE_THROUGH); via.SetLayerPair(pcbnew.F_Cu, pcbnew.B_Cu)
        via.SetNetCode(original["net_code"])
        via.SetUuid(pcbnew.KIID(str(uuid.uuid5(uuid.NAMESPACE_URL, f"analysis-lab/{track_id}/{fractions}/via/{i}"))))
        board.Add(via); created_vias.append(identity(via))
    return {"original": original, "created_tracks": created_tracks, "created_vias": created_vias,
            "fractions": fractions}


def restore(board, mutation):
    for item in list(board.GetTracks()):
        if identity(item) in mutation["created_tracks"] + mutation["created_vias"]:
            board.Remove(item)
            item.thisown = False
    old = mutation["original"]
    track = new_track(board, old["start"], old["end"], old["width"], old["layer"], old["net_code"], "restored")
    track.SetUuid(pcbnew.KIID(old["uuid"]))


def generate(source=SOURCE, output=OUTPUT, case_id=CASE_ID):
    private = output / "private"
    baseline_path = stage_project(source, private / "original", case_id)
    baseline_report, baseline_log = drc(baseline_path, private / "original/drc.json")
    if baseline_report.get("unconnected_items"):
        raise ValueError("This fixture requires zero baseline opens")
    baseline = pcbnew.LoadBoard(str(baseline_path))
    original_metrics = metrics(baseline)
    via_sizes = Counter((t.GetWidth(t.TopLayer())/1_000_000, t.GetDrillValue()/1_000_000)
                        for t in baseline.GetTracks() if t.GetClass() == "PCB_VIA")
    via_dimensions = via_sizes.most_common(1)[0][0] if via_sizes else (1.2, .6)
    expected = finding_signatures(baseline_report)
    candidates = sorted((t for t in baseline.GetTracks() if t.GetClass() == "PCB_TRACK"
        and t.GetLayer() == pcbnew.F_Cu and t.GetLength() >= 6_000_000), key=lambda t: (-t.GetLength(), identity(t)))
    trial_path = stage_project(source, private / "degraded", case_id)
    attempts = []
    for original in candidates:
        for fractions in [[.3, .7], [.15, .4], [.6, .85], [.4, .6]]:
            board = pcbnew.LoadBoard(str(baseline_path))
            mutation = mutate(board, identity(original), fractions, via_dimensions)
            trial_metrics = metrics(board)
            if not math.isclose(trial_metrics["length_mm"], original_metrics["length_mm"], abs_tol=1e-5):
                attempts.append({"source_track": identity(original), "fractions": fractions,
                    "reason": "Skipped mutation that changes physical union length due to existing overlap",
                    "physical_length_delta": trial_metrics["length_mm"]-original_metrics["length_mm"]})
                continue
            save_board(trial_path, board)
            report, log = drc(trial_path, private / "degraded/drc.json")
            added = finding_signatures(report) - expected
            attempts.append({"source_track": identity(original), "fractions": fractions,
                             "summary": drc_summary(report), "added_findings": list(added.elements())})
            if not added and finding_signatures(report) == expected:
                break
        else:
            continue
        break
    else:
        write_json(private / "attempts.json", attempts)
        raise ValueError("No legal two-via excursion found")
    restored_path = stage_project(source, private / "restored", case_id)
    restored_board = pcbnew.LoadBoard(str(trial_path))
    restore(restored_board, mutation)
    save_board(restored_path, restored_board)
    restored_report, restored_log = drc(restored_path, private / "restored/drc.json")
    if finding_signatures(restored_report) != expected:
        raise ValueError("Restored board did not reproduce baseline DRC findings")
    original_metrics, degraded_metrics, restored_metrics = metrics(baseline), metrics(board), metrics(restored_board)
    if degraded_metrics["vias"] != original_metrics["vias"]+2 or not math.isclose(
            degraded_metrics["length_mm"], original_metrics["length_mm"], abs_tol=1e-5):
        raise ValueError("Mutation changed something beyond exactly two vias and a collinear segment split")
    public_path = stage_project(trial_path, output / "public", case_id)
    geometry = export_geometry(public_path)
    write_json(output / f"public/{case_id}.json", geometry)
    uuid_to_id = {item["uuid"]: item["id"] for section in ["tracks", "vias"] for item in geometry[section]}
    old = mutation["original"]
    repair = {"remove_tracks": [uuid_to_id[u] for u in mutation["created_tracks"]],
        "remove_vias": [uuid_to_id[u] for u in mutation["created_vias"]], "add_tracks": [{
            "start": old["start"], "end": old["end"], "width": old["width"],
            "layer": board.GetLayerName(old["layer"]), "net": f"N{old['net_code']}"}]}
    write_json(private / "answer.json", {"mutation": mutation, "proposal": repair,
        "original_metrics": original_metrics, "degraded_metrics": degraded_metrics,
        "restored_metrics": restored_metrics, "attempts": attempts,
        "original_drc": drc_summary(baseline_report), "degraded_drc": drc_summary(report),
        "restored_drc": drc_summary(restored_report), "native_logs": {
            "original": baseline_log, "degraded": log, "restored": restored_log}})
    write_json(private / "provenance.json", {"source": str(source), "source_sha256": sha256(source),
        "source_project_sha256": sha256(source.with_suffix(".kicad_pro")),
        "original_sha256": sha256(baseline_path), "degraded_sha256": sha256(trial_path),
        "restored_sha256": sha256(restored_path), "generator_sha256": sha256(Path(__file__)),
        "kicad_version": pcbnew.GetBuildVersion(),
        "validation": "Native kicad-cli DRC with all-track-errors, schematic-parity, and refill-zones; identical normalized finding multiset and zero opens across original/degraded/restored.",
        "blinding": "Private by protocol only; source and original board must not be accessible to blind solvers.",
        "limitations": geometry["limitations"]})
    write_json(output / "public/objective.json", {"objective": "Minimize physical copper centerline union length_mm + 2mm per via, preserving width, connectivity, fixed pads, rules, and board geometry. Collinear same-net/layer overlaps count once; stored segment length is reported separately.",
        "authority": "Native board/project and native KiCad DRC. Existing warnings are permitted only if no new findings or opens appear.",
        "proposal_format": {"remove_tracks": ["T1"], "remove_vias": [], "add_tracks": [{
            "start": [0, 0], "end": [1, 1], "width": .8, "layer": "top_cu", "net": "N1"}]},
        "inspection_schema": f"{case_id}.json contains exact native coordinates/shapes; short IDs map to UUIDs. Replacement is a simultaneous transaction. Use only IDs that exist in your board."})
    return {"public": str(public_path), "geometry": str(output / f"public/{case_id}.json"),
            "drc": drc_summary(report), "original": original_metrics, "degraded": degraded_metrics,
            "restored": restored_metrics, "attempts": len(attempts)}


def evaluate(board_path, geometry_path, proposal, output):
    if output.exists() and any(output.iterdir()):
        raise ValueError("Evaluation output directory must be absent or empty")
    output.mkdir(parents=True, exist_ok=True)
    candidate_path = stage_project(board_path, output, board_path.stem)
    geometry = json.loads(geometry_path.read_text())
    if sha256(board_path) != geometry["native_sha256"]:
        raise ValueError("Geometry packet does not match native board hash")
    if sha256(board_path.with_suffix(".kicad_pro")) != geometry["project_sha256"]:
        raise ValueError("Geometry packet does not match native project/rules hash")
    board = pcbnew.LoadBoard(str(board_path))
    initial_metrics = metrics(board)
    baseline_report, _ = drc(board_path, output / "baseline-drc.json")
    records = {item["id"]: item for section in ["tracks", "vias"] for item in geometry[section]}
    remove_ids = proposal.get("remove_tracks", []) + proposal.get("remove_vias", [])
    if len(set(remove_ids)) != len(remove_ids):
        raise ValueError("Duplicate removal ID")
    remove_uuids = {records[item_id]["uuid"] for item_id in remove_ids}
    for item in list(board.GetTracks()):
        if identity(item) in remove_uuids:
            board.Remove(item)
            # KiCad 10/Python 3.14 transfers removed-item ownership to SWIG;
            # destroying a downcast via wrapper corrupts its type registry.
            # Retain native storage until this short-lived worker exits.
            item.thisown = False
    layer_ids = {item["name"]: item["id"] for item in geometry["layers"]}
    allowed_widths = {}
    for item in geometry["tracks"]:
        allowed_widths.setdefault(item["net"], set()).add(item["width"])
    for i, item in enumerate(proposal.get("add_tracks", [])):
        if item["width"] not in allowed_widths.get(item["net"], set()):
            raise ValueError("Added width must match an existing width on that same net")
        if item["net"] not in geometry["nets"]:
            raise ValueError("Unknown net")
        new_track(board, item["start"], item["end"], item["width"], layer_ids[item["layer"]],
                  int(item["net"][1:]), f"analysis-lab/proposal/{i}/{json.dumps(item,sort_keys=True)}")
    save_board(candidate_path, board)
    final_metrics = metrics(board)
    report, log = drc(candidate_path, output / "drc.json")
    added = finding_signatures(report) - finding_signatures(baseline_report)
    valid = not added and len(report.get("unconnected_items", [])) <= len(baseline_report.get("unconnected_items", []))
    result = {"valid": valid, "improved": valid and final_metrics["cost_mm"] < initial_metrics["cost_mm"]-1e-7,
        "original": initial_metrics, "new": final_metrics, "drc": drc_summary(report),
        "added_findings": [json.loads(value) for value in added.elements()], "native_log": log,
        "provenance": {"board_sha256": sha256(board_path),
            "project_sha256": sha256(board_path.with_suffix(".kicad_pro")),
            "candidate_sha256": sha256(candidate_path), "evaluator_sha256": sha256(Path(__file__)),
            "kicad_version": pcbnew.GetBuildVersion()}}
    write_json(output / "assessment.json", result)
    return result


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    sub = parser.add_subparsers(dest="command", required=True)
    make = sub.add_parser("generate")
    make.add_argument("--source", type=Path, default=SOURCE)
    make.add_argument("--output", type=Path, default=OUTPUT)
    make.add_argument("--case-id", default=CASE_ID)
    check = sub.add_parser("evaluate")
    check.add_argument("board", type=Path)
    check.add_argument("geometry", type=Path)
    check.add_argument("proposal", type=Path)
    check.add_argument("--output", type=Path, required=True)
    args = parser.parse_args()
    if args.command == "generate":
        result = generate(args.source, args.output, args.case_id)
    else:
        result = evaluate(args.board, args.geometry, json.loads(args.proposal.read_text()), args.output)
    print(json.dumps(result, indent=2))


if __name__ == "__main__":
    main()
