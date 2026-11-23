# Copyright (C) 2026 Florian Loitsch.
"""Build the portable KiCad inputs for a schematic/mechanical comparison."""

import json
import os
import re
import shutil
import subprocess
import tempfile
from pathlib import Path

import harness_common
import keepout_checks
import outline_rule_areas
from returned_board import (
    EDGE_LAYER, POINT, SHAPES, board_donor_arguments, board_outline, brief_evidence, check_imported_clearance_policy, check_imported_stackup,
    check_stackup, checks_for, children, expected_outline, footprint_cutouts, glob_pattern, numbers, outline_digest,
    footprint_clearance_arguments, pad_distance_checks, require, stage_project,
)

KEEPOUT_FLAGS = keepout_checks.FLAGS


def shape_forms(donor, geometry):
    """Keep geometrical fields only. The donor contributes no item IDs."""
    if donor is None:
        # Brief-defined rectangles and polygons already lower to lines.
        return [f'(gr_line (start {a[0]} {a[1]}) (end {b[0]} {b[1]}) '
                '(stroke (width 0.05) (type solid)) (layer "Edge.Cuts"))'
                for (_, a, b), count in sorted(geometry.items()) for _ in range(count)]
    shapes = []
    for name, block in children(donor.read_text()):
        if name not in SHAPES or not EDGE_LAYER.search(block):
            continue
        fields = []
        for key, form in children(block):
            if key in {"start", "mid", "end", "center"}:
                point = re.fullmatch(rf"\({key}\s+{POINT}\s*\)", form)
                require(point is not None, f"unsupported Edge.Cuts point in {name}")
                fields.append(f"({key} {point[1]} {point[2]})")
            elif key == "pts":
                points = [re.fullmatch(rf"\(xy\s+{POINT}\s*\)", point) for _, point in children(form)]
                require(points and all(points), f"unsupported Edge.Cuts point list in {name}")
                fields.append("(pts " + " ".join(f"(xy {point[1]} {point[2]})" for point in points) + ")")
        fields.append("(stroke (width 0.05) (type solid))")
        if name in {"gr_rect", "gr_circle", "gr_poly"}:
            fields.append("(fill no)")
        shapes.append(f'({name} {" ".join(fields)} (layer "Edge.Cuts"))')
    return shapes


def board_text(items):
    return "(kicad_pcb\n  " + "\n  ".join(items) + "\n)\n"


def materialized_constraints(brief, references, board_min):
    """Decode the portable pose and side subset; copy other entries for handoff."""
    require(isinstance(brief, dict) and type(brief.get("version")) is int and brief["version"] == 1,
            "expected version 1 placement constraints")
    require(type(brief.get("move_all", False)) is bool, "invalid move_all")
    poses, sides, manual = {}, {}, []
    for kind, entries in brief.items():
        if kind in {"version", "comment", "move_all", "outline", "region", "keepout"}:
            continue
        if kind not in {"place", "front", "back"}:
            if kind == "near" and isinstance(entries, list):
                objectives = [e for e in entries if isinstance(e, dict) and e.get("minimize") is True]
                if objectives:
                    manual.append({"kind": kind, "entries": objectives,
                                   "action": "Place each named pad as close as feasible to its target pin "
                                             "while meeting the board constraints. Report each achieved "
                                             "pad-centre distance in millimetres."})
                    entries = [e for e in entries if e not in objectives]
            if entries not in ([], None):
                manual.append({"kind": kind, "entries": entries,
                               "action": "Enter this constraint in the layout tool before placing parts."})
            continue
        require(isinstance(entries, list), f"{kind} must be a list")
        for entry in entries:
            if kind in {"front", "back"}:
                selector = glob_pattern(entry)
                matched = [reference for reference in sorted(references) if selector.fullmatch(reference)]
                require(matched, f"{kind} selector {entry!r} matches no source footprint")
                for reference in matched:
                    require(reference not in sides or sides[reference] == kind,
                            f"{reference} is required on both sides")
                    sides[reference] = kind
                continue
            require(isinstance(entry, dict), "invalid place entry")
            expected_fields = {"part", "origin", "x", "y", "angle"}
            if not {"x", "y", "angle"} <= entry.keys() or not entry.keys() <= expected_fields:
                manual.append({"kind": kind, "entries": [entry],
                               "action": "Set this position in the layout tool. Automatic preparation requires x, y and angle."})
                continue
            reference = entry.get("part")
            require(isinstance(reference, str) and reference in references, f"no source footprint for {reference!r}")
            origin = entry.get("origin")
            require(origin in (None, "absolute", "board"), f"unsupported place origin {origin!r}")
            start = (0, 0) if origin == "absolute" else board_min
            x, y, angle = numbers([entry["x"], entry["y"], entry["angle"]], 3, f"place pose for {reference}")
            pose = [start[0] + x, start[1] + y, angle % 360]
            require(reference not in poses or poses[reference] == pose, f"conflicting exact poses for {reference}")
            poses[reference] = pose
    areas = placement_regions(brief.get("region", []), references, sides, board_min, manual)
    return poses, sides, manual, areas


def placement_regions(entries, references, sides, board_min, manual):
    """Represent each supported rectangle once per eligible placement side."""
    require(isinstance(entries, list), "region must be a list")
    areas = []
    for ordinal, entry in enumerate(entries, 1):
        handoff = {"kind": "region", "entries": [entry],
                   "action": "Enter this constraint in the layout tool before placing parts."}
        manual.append(handoff)
        if (not isinstance(entry, dict) or not {"parts", "x", "y"} <= entry.keys()
                or not entry.keys() <= {"parts", "x", "y", "origin"}):
            continue
        origin = entry.get("origin")
        require(origin in (None, "board", "absolute"), f"unsupported region origin {origin!r}")
        selectors = entry["parts"]
        require(isinstance(selectors, list) and selectors, "region needs part selectors")
        patterns = [glob_pattern(selector) for selector in selectors]
        matches = [{ref for ref in references if pattern.fullmatch(ref)} for pattern in patterns]
        if not all(matches):
            continue
        x, y = (0, 0) if origin == "absolute" else board_min
        x0, x1 = sorted(numbers(entry["x"], 2, "region x"))
        y0, y1 = sorted(numbers(entry["y"], 2, "region y"))
        require(x0 < x1 and y0 < y1, "region bounds must enclose a positive area")
        bounds = [x + x0, y + y0, x + x1, y + y1]
        targets = {ref: [] for ref in sorted(set.union(*matches))}
        for side, layer in (("front", "F.Cu"), ("back", "B.Cu")):
            eligible = [ref for ref in targets if sides.get(ref) in (None, side)]
            if eligible:
                name = f"Region {ordinal} {layer}"
                areas.append({"name": name, "layer": layer, "bounds_mm": bounds})
                for ref in eligible:
                    targets[ref].append(name)
        handoff.update({"reference_assignments": targets,
                        "action": "Manually associate each reference with an eligible area in the layout tool. "
                                  "Multiple areas for a reference represent its allowed side choices."})
    return areas


def rule_area_layer_ids(board, area):
    names = area["layers"] if "layers" in area else [area["layer"]]
    return [layer for layer in board.GetEnabledLayers().CuStack() if board.GetStandardLayerName(layer) in names]


def add_rule_areas(pcbnew, board, areas):
    for area in areas:
        zone = pcbnew.ZONE(board)
        layers = pcbnew.LSET()
        for layer in rule_area_layer_ids(board, area):
            layers.AddLayer(layer)
        zone.SetLayerSet(layers)
        zone.SetZoneName(area["name"])
        zone.SetIsRuleArea(True)
        for flag in KEEPOUT_FLAGS:
            getattr(zone, f"SetDoNotAllow{flag}")(flag in area.get("blocked", []))
        polygon = zone.Outline()
        polygon.NewOutline()
        for x, y in keepout_checks.native_corners(pcbnew, area["bounds_mm"]):
            polygon.Append(x, y)
        board.Add(zone)


def verify_rule_areas(pcbnew, path, areas, donor_count=0):
    board = pcbnew.LoadBoard(str(path))
    require(board is not None and not len(board.GetTracks()), "prepared board has tracks or vias")
    names = {area["name"] for area in areas}
    zones = {zone.GetZoneName(): zone for zone in board.Zones() if zone.GetZoneName() in names}
    require(len(board.Zones()) == len(areas) + donor_count and zones.keys() == names
            and sum(zone.GetZoneName() in names for zone in board.Zones()) == len(names),
            "prepared board contains unexpected zones")
    for area in areas:
        zone = zones[area["name"]]
        layers = rule_area_layer_ids(board, area)
        require(zone.GetIsRuleArea() and zone.GetNetCode() == 0 and not zone.IsFilled()
                and layers and list(zone.GetLayerSet().CuStack()) == layers
                and all(getattr(zone, f"GetDoNotAllow{flag}")() == (flag in area.get("blocked", []))
                        for flag in KEEPOUT_FLAGS),
                f"{area['name']} must be an unfilled rule area with the authored keepout flags")
        outline = zone.Outline()
        require(outline.OutlineCount() == 1 and outline.HoleCount(0) == 0, f"invalid area for {area['name']}")
        points = [(point.x, point.y) for point in outline.COutline(0).CPoints()]
        require(len(points) == 4 and set(points) == set(keepout_checks.native_corners(pcbnew, area["bounds_mm"])),
                f"{area['name']} bounds changed during KiCad readback")


def apply_board_placements(pcbnew, path, bounds, poses, sides, areas, keepouts):
    """KiCad carries each pad, text, and footprint keepout with its part."""
    board = pcbnew.LoadBoard(str(path))
    require(board is not None, "KiCad could not load the imported board")
    footprints = sorted(board.GetFootprints(), key=lambda part: part.GetReference())
    millimetres = pcbnew.FromMM
    parking_left = bounds[2] + 10
    parking_right = parking_left + max(80, bounds[2] - bounds[0])
    x, y, row_height = parking_left, bounds[1], 0
    parked = []
    for part in footprints:
        reference = part.GetReference()
        if reference in sides and part.IsFlipped() != (sides[reference] == "back"):
            part.Flip(part.GetPosition(), pcbnew.FLIP_DIRECTION_TOP_BOTTOM)
        if reference in poses:
            px, py, degrees = poses[reference]
            part.SetOrientationDegrees(degrees)
            part.SetPosition(pcbnew.VECTOR2I(millimetres(px), millimetres(py)))
        else:
            box = part.GetBoundingBox(False, False)
            width, height = pcbnew.ToMM(box.GetWidth()), pcbnew.ToMM(box.GetHeight())
            if x > parking_left and x + width > parking_right:
                x, y, row_height = parking_left, y + row_height + 3, 0
            position = part.GetPosition()
            px = x + pcbnew.ToMM(position.x - box.GetX())
            py = y + pcbnew.ToMM(position.y - box.GetY())
            part.SetPosition(pcbnew.VECTOR2I(millimetres(px), millimetres(py)))
            x += width + 3
            row_height = max(row_height, height)
            parked.append(reference)
        part.SetLocked(reference in poses)
    for area in keepouts:
        held = keepout_checks.held_overlaps(pcbnew, board, area, poses.keys())
        layers = keepout_checks.copper_layers_for(area, pcbnew, board)
        blocked = ["Tracks", "Vias", "ZoneFills"] if area["copper"] else []
        if not held:
            blocked.append("Footprints")
        area.update({"permitted_held_parts": held, "blocked": blocked,
                     "layers": [board.GetStandardLayerName(layer) for layer in layers]})
        if held:
            area["action"] = "Preserve the authored held poses. Keep other footprint bodies outside this area."
    add_rule_areas(pcbnew, board, areas + keepouts)
    require(pcbnew.SaveBoard(str(path), board, True), "KiCad could not save the prepared board")
    return parked


def source_parts(description):
    # Compare net incidence and IDs before and after KiCad's transforms.
    return {part["reference"]: (part["uuid"], part["footprint"], sorted(pad for pad in part["pads"] if pad[1] is not None))
            for part in description["footprints"]}


def prepare(arguments, schematic, work, layers, *, placer=None):
    """Publish the bundle only after the sanitized artifacts pass readback."""
    name = schematic.stem
    row = {"name": name, "mode": "prepare-only", "status": "unverified", "pass": False, "stage": "inputs"}
    env = os.environ.copy()
    env.update({"LC_ALL": "C", "XDG_CONFIG_HOME": str(work / "kicad-config")})

    def command(label, args):
        process = harness_common.run_group([str(arguments.binary), *args], capture_output=True, text=True,
                                           timeout=arguments.timeout, env=env)
        (work / f"{label}.log").write_text(process.stdout + process.stderr)
        require(process.returncode == 0, f"{label} exited {process.returncode}; see {label}.log")
        return json.loads(process.stdout)

    try:
        try:
            import pcbnew
        except ImportError as error:
            raise ValueError("--prepare-only needs KiCad's pcbnew Python bindings in the runner's interpreter") from error
        require(hasattr(pcbnew, "FLIP_DIRECTION_TOP_BOTTOM"),
                "--prepare-only needs KiCad 10's pcbnew Python bindings")
        evidence, files = brief_evidence(arguments, schematic, layers)
        row.update(evidence)
        brief = json.loads(arguments.constraints.read_text())
        require(isinstance(brief, dict), "expected placement constraints object")
        geometry = expected_outline(brief, arguments.outline_board)
        require(outline_digest(geometry) == evidence.get("outline_sha256"), "the outline changed while being staged")
        shapes = shape_forms(arguments.outline_board, geometry)
        donor_rules = outline_rule_areas.donor_forms(arguments, evidence, layers)
        require(board_outline(children(board_text(shapes))) == geometry, "could not reproduce the expected outline")
        with tempfile.TemporaryDirectory(dir=work) as temporary:
            scratch = Path(temporary)
            package = scratch / "package"
            project = package / "project"
            stage_project(files, evidence, project)
            board_path = project / f"{name}.kicad_pcb"
            imported = scratch / "imported"
            row["stage"] = "import"
            with board_donor_arguments(arguments, evidence, scratch) as donor_args:
                report = command("import", ["import-kicad-netlist", str(project / schematic.name),
                                           str(imported), name, "--layers", str(layers), *donor_args,
                                           *footprint_clearance_arguments(arguments)])
            harness_common.atomic_json(work / "import.json", report, newline=True)
            require(isinstance(report, dict) and report.get("missing") == [] and report.get("footprints", 0) > 0,
                    "the source import has incomplete footprint evidence")
            require(not report.get("substituted"),
                    "the import renamed footprint IDs; resolve the source library links before packaging")
            source_pcb = imported / board_path.name
            check_imported_clearance_policy(report, evidence)
            check_imported_stackup(report, evidence, source_pcb)
            imported_board = list(children(source_pcb.read_text()))
            header = {"version", "generator", "generator_version", "general", "layers", "paper", "setup"}
            require(all(key in header | {"footprint", "net"} for key, _ in imported_board),
                    "the source import contains unexpected board-level items")
            if donor_rules:
                outline_rule_areas.ensure_unique_ids(board_text([form for _, form in imported_board] + shapes), donor_rules)
            outline_file = package / "outline.kicad_pcb"
            outline_file.write_text(board_text([form for key, form in imported_board if key in header] + shapes + donor_rules))
            row["stage"] = "outline"
            outline_description = command("outline", ["describe-kicad-board", str(package), "outline"])
            outline = outline_description.get("outline")
            outline_board = pcbnew.LoadBoard(str(outline_file))
            closed = outline_board is not None and outline_board.GetBoardPolygonOutlines(pcbnew.SHAPE_POLY_SET(), False)
            require(closed and isinstance(outline, dict) and outline.get("from_footprints") is False,
                    "supply a complete closed board-level Edge.Cuts outline; any footprint cuts must be separate")
            board_min = numbers(outline.get("minimum"), 2, "outline minimum")
            board_max = numbers(outline.get("maximum"), 2, "outline maximum")
            board_path.write_text(board_text([form for _, form in imported_board] + shapes + donor_rules))
            row["stage"] = "placement"
            description = command("source", ["describe-kicad-board", str(project), name])
            source = source_parts(description)
            require(len(source) == report["footprints"], "imported references are missing or ambiguous")
            poses, sides, manual, areas = materialized_constraints(brief, source, board_min)
            # Validate objective selectors against the donor footprints before handing
            # them to another tool. Distances in this unplaced board have no meaning.
            near = brief.get("near", [])
            if isinstance(near, list):
                marked = [e for e in near if isinstance(e, dict) and "minimize" in e]
                require(all(type(e["minimize"]) is bool for e in marked), "near minimize must be a Boolean")
                pad_distance_checks([e for e in marked if e["minimize"]], board_path)
            keepouts = keepout_checks.areas_for(brief.get("keepout", []), board_min)
            parked = apply_board_placements(pcbnew, board_path, [*board_min, *board_max], poses, sides, areas, keepouts)
            actual = command("prepared", ["describe-kicad-board", str(project), name])
            require(source_parts(actual) == source, "KiCad's transforms changed source footprint identities or nets")
            require(actual.get("copper_layers") == description.get("copper_layers")
                    and len(actual["copper_layers"]) == layers, "prepared copper layers differ from the brief")
            items = list(children(board_path.read_text()))
            require(board_outline(items) == geometry, "prepared board-level outline changed")
            require(footprint_cutouts(items) == footprint_cutouts(imported_board),
                    "prepared board changed the source footprint cutouts")
            # SaveBoard mirrors the project's text variables into the board
            # and creates user preferences. The original project owns both
            # imported rule context and text variables; leave preferences out.
            if any(key == "property" for key, _ in items):
                items = [(key, form) for key, form in items if key != "property"]
                board_path.write_text(board_text([form for _, form in items]))
            board_path.with_suffix(".kicad_prl").unlink(missing_ok=True)
            if areas or keepouts or donor_rules:
                verify_rule_areas(pcbnew, board_path, areas + keepouts, len(donor_rules))
            if donor_rules:
                row["rule_area_check"] = outline_rule_areas.check(donor_rules, board_path.read_text(), layers)
                require(row["rule_area_check"]["satisfied"], "prepared board changed an outline rule area")
            if "expected_stackup" in evidence:
                row["stackup_check"] = check_stackup(evidence["expected_stackup"], board_path.read_text(), source_pcb)
                require(row["stackup_check"]["satisfied"], "prepared board changed the authored stackup")
            allowed = header | {"footprint", "net"} | ({"zone"} if areas or keepouts or donor_rules else set())
            unexpected = {key for key, form in items if not (key in allowed
                          or key in SHAPES and EDGE_LAYER.search(form) or form == "(embedded_fonts no)")}
            require(not unexpected, f"prepared board has unexpected board items: {', '.join(sorted(unexpected))}")
            expected = {"version": 1, "place": [dict(part=ref, origin="absolute", x=x, y=y, angle=angle)
                        for ref, (x, y, angle) in poses.items()]}
            for side in ("front", "back"):
                expected[side] = [ref for ref, wanted in sides.items() if wanted == side]
            checks = checks_for(expected, actual)
            require(all(check["satisfied"] for check in checks), "prepared pose or side did not survive KiCad readback")
            require({part["reference"] for part in actual["footprints"] if part["locked"]} == poses.keys(),
                    "prepared locked footprints do not match the exact requirements")
            require(all(part["body"][0] > board_max[0] for part in actual["footprints"] if part["reference"] in parked),
                    "an unplaced footprint was not parked outside the outline")
            for relative, digest in evidence["project_files_sha256"].items():
                require(harness_common.file_sha256(project / relative) == digest,
                        f"preparation changed the original project file: {relative}")
            brief_path = project / "constraints.json"
            shutil.copyfile(arguments.constraints, brief_path)
            require(harness_common.file_sha256(brief_path) == evidence["inputs"]["constraints"]["sha256"],
                    "mechanical constraints changed while being staged")
            generated = {board_path.name, brief_path.name}
            layout_evidence = None
            if placer is not None:
                layout_config = project / "layout.json"
                harness_common.atomic_json(layout_config, {"placer": placer}, indent=2, newline=True)
                generated.add(layout_config.name)
                layout_evidence = {"path": "project/layout.json", "sha256": harness_common.file_sha256(layout_config)}
            delivered_files = {str(path.relative_to(project)) for path in project.rglob("*") if path.is_file()}
            require(delivered_files == files.keys() | generated,
                    "the staged project contains unexpected generated files")
            manifest = {
                "brief_sha256": evidence["brief_sha256"], "outline_sha256": evidence["outline_sha256"],
                "board": f"project/{board_path.name}", "schematic": f"project/{schematic.name}",
                "outline": "outline.kicad_pcb", "constraints": "project/constraints.json", "layers": layers,
                "parts": len(source), "locked_poses": poses, "required_sides": sides,
                "unplaced": parked, "placement_regions": areas, "keepouts": keepouts, "manual_constraints": manual,
            }
            if "unresolved_project_inputs" in evidence:
                manifest["unresolved_project_inputs"] = evidence["unresolved_project_inputs"]
            if "footprint_clearance_policy" in evidence:
                manifest["footprint_clearance_policy"] = evidence["footprint_clearance_policy"]
            if "footprints_from" in evidence["inputs"]:
                donor = evidence["inputs"]["footprints_from"]
                manifest["footprints_from"] = {"filename": Path(donor["path"]).name, "sha256": donor["sha256"]}
            if donor_rules:
                manifest["outline_rule_areas"] = evidence["outline_rule_areas"]
            if "expected_stackup" in evidence:
                donor = evidence["inputs"]["stackup_from"]
                manifest["stackup_from"] = {"filename": Path(donor["path"]).name, "source_sha256": donor["sha256"],
                                            **evidence["expected_stackup"]}
            if layout_evidence is not None:
                manifest["layout_config"] = layout_evidence
            harness_common.atomic_json(package / "exchange.json", manifest, newline=True)
            package.rename(work / "package")
        row.update({"status": "prepared", "pass": True, "stage": "complete",
                    "manifest": "package/exchange.json", "parts": len(source), "placed": len(poses),
                    "unplaced": len(parked), "placement_regions": areas, "keepouts": keepouts,
                    "manual_constraints": manual})
        if layout_evidence is not None:
            row["layout_config"] = {**layout_evidence, "path": "package/" + layout_evidence["path"]}
    except subprocess.TimeoutExpired:
        row["error"] = f"{row['stage']} timed out after {arguments.timeout} s"
    except (ValueError, ArithmeticError, TypeError, OSError, RuntimeError) as error:
        row["error"] = str(error)
    return row
