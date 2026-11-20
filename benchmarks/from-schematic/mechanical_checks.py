# Copyright (C) 2026 Florian Loitsch.
"""Independent native checks for explicit donor additions and antenna edges."""

import collections
import math

from returned_board import (ANGLE_EPSILON, POSITION_EPSILON_MM, footprint_differences,
                            normalized_footprints, require)

# Same gap as pcb_placer's OVERHANG_SLACK.
OVERHANG_SLACK_MM = 0.5


def additions_for(brief, donor):
    entries = brief.get("add", [])
    require(isinstance(entries, list), "add must be a list")
    if entries:
        require(donor is not None, "add.from_board needs an explicit --mechanical-donor for returned-board scoring")
    references = set()
    for entry in entries:
        require(isinstance(entry, dict) and set(entry) == {"part", "from_board"}
                and entry["from_board"] is True and isinstance(entry["part"], str)
                and entry["part"] and not any(c in entry["part"] for c in "*?"),
                "returned-board add supports only an exact part and from_board: true at the donor pose")
        require(entry["part"] not in references, f"repeated add reference {entry['part']}")
        references.add(entry["part"])
    return references


def edges_for(brief):
    entries = brief.get("edge", [])
    require(isinstance(entries, list), "edge must be a list")
    seen = set()
    for entry in entries:
        require(isinstance(entry, dict) and set(entry) == {"part", "edge", "flush", "overhang"}
                and entry["edge"] == "any" and entry["flush"] is True and entry["overhang"] == {}
                and isinstance(entry["part"], str) and entry["part"]
                and not any(c in entry["part"] for c in "*?"),
                "returned-board edge supports an exact part with edge: any, flush: true, and overhang: {}")
        require(entry["part"] not in seen, f"repeated edge reference {entry['part']}")
        seen.add(entry["part"])
    return seen


def by_reference(board):
    indexed = collections.defaultdict(list)
    for part in board.GetFootprints():
        indexed[part.GetReference()].append(part)
    return indexed


def pose(pcbnew, part):
    p = part.GetPosition()
    return {"uuid": part.m_Uuid.AsString(), "position_mm": [pcbnew.ToMM(p.x), pcbnew.ToMM(p.y)],
            "angle_degrees": part.GetOrientationDegrees(), "side": "back" if part.IsFlipped() else "front",
            "locked": part.IsLocked()}


def addition_checks(pcbnew, donor, candidate, references):
    sources, actual = (by_reference(board) for board in (donor, candidate))
    source_models = normalized_footprints(pcbnew, donor, electrical=False, selected_parts=references,
                                          all_graphics=True)
    actual_models = normalized_footprints(pcbnew, candidate, electrical=False, selected_parts=references,
                                          all_graphics=True)

    def pads(part):
        # Key by native pad identity so swapped duplicate pad numbers cannot
        # pass merely because the multiset of named nets is the same.
        return sorted((p.m_Uuid.AsString(), p.GetNumber(), p.GetNetname()) for p in part.Pads())

    checks = []
    for ref in sorted(references):
        require(len(sources[ref]) == 1, f"mechanical donor needs exactly one footprint named {ref}")
        source = sources[ref][0]
        require(not source.Zones(), f"add {ref}: footprint zones on added parts are outside the scoped scorer")
        expected = pose(pcbnew, source)
        donor_locked = expected["locked"]
        expected["locked"] = True
        expected_pads = pads(source)
        require(len({r[0] for r in expected_pads}) == len(expected_pads),
                f"add {ref}: repeated pad UUID in the mechanical donor")
        check = {"kind": "add", "part": ref, "expected": expected, "satisfied": False,
                 "donor_was_locked": donor_locked, "expected_pads": expected_pads}
        candidates = actual[ref]
        if len(candidates) != 1:
            check.update(candidate_count=len(candidates), failures=["missing" if not candidates else "ambiguous"])
        else:
            part = candidates[0]
            found = pose(pcbnew, part)
            failures = []
            if found["uuid"] != expected["uuid"]:
                failures.append("footprint_uuid")
            if (math.dist(found["position_mm"], expected["position_mm"]) > POSITION_EPSILON_MM
                    or abs((found["angle_degrees"] - expected["angle_degrees"] + 180) % 360 - 180) > ANGLE_EPSILON
                    or found["side"] != expected["side"]):
                failures.append("pose")
            if not found["locked"]:
                failures.append("unlocked")
            if (part.GetFPIDAsString() != source.GetFPIDAsString()
                    or part.GetAttributes() != (source.GetAttributes() | pcbnew.FP_BOARD_ONLY)):
                failures.append("footprint_id_or_attributes")
            differences = footprint_differences({ref: source_models[ref]}, {ref: actual_models[ref]})
            if differences or part.Zones():
                failures.append("physical_geometry")
            actual_pads = pads(part)
            if expected_pads != actual_pads:
                failures.append("pad_identity_or_net")
            check.update(actual=found, failures=failures, satisfied=not failures,
                         geometry_differences=differences, actual_pads=actual_pads)
        checks.append(check)
    return checks


def antenna_zone(part):
    ref = part.GetReference()
    zones = list(part.Zones())
    require(len(zones) == 1 and zones[0].GetIsRuleArea(),
            f"edge {ref}: expected one footprint-owned antenna rule area")
    zone = zones[0]
    poly = zone.Outline()
    require(poly.OutlineCount() == 1 and poly.HoleCount(0) == 0,
            f"edge {ref}: the antenna keepout must be a single rectangle")
    contour = poly.Outline(0)
    require(contour.PointCount() == 4 and contour.ArcCount() == 0,
            f"edge {ref}: the antenna keepout must be a single orthogonal rectangle")
    box = poly.BBox()
    bounds = (box.GetLeft(), box.GetTop(), box.GetRight(), box.GetBottom())
    require(poly.Area() > 0 and poly.Area() == box.GetWidth() * box.GetHeight(),
            f"edge {ref}: the antenna keepout must be a single orthogonal rectangle")
    flags = tuple(getattr(zone, f"GetDoNotAllow{item}")()
                  for item in ("Tracks", "Vias", "Pads", "ZoneFills", "Footprints"))
    require(all(flags) and not zone.GetPlacementAreaEnabled(),
            f"edge {ref}: an antenna keepout must exclude copper and footprints")
    return zone, bounds, flags


def zone_signature(pcbnew, part):
    local = pcbnew.FOOTPRINT(part)
    if local.IsFlipped():
        local.Flip(local.GetPosition(), pcbnew.FLIP_DIRECTION_TOP_BOTTOM)
    local.SetOrientationDegrees(0)
    local.SetPosition(pcbnew.VECTOR2I(0, 0))
    zone, bounds, flags = antenna_zone(local)
    return bounds, flags, zone.GetLayerSet().FmtHex(), zone.GetZoneName(), zone.GetAssignedPriority()


def edge_checks(pcbnew, reference, candidate, references):
    sources, actual = (by_reference(board) for board in (reference, candidate))
    board_outline = pcbnew.SHAPE_POLY_SET()
    require(candidate.GetBoardPolygonOutlines(board_outline, False) and board_outline.OutlineCount() == 1,
            "antenna edge scoring needs one closed native board outline")
    outer = board_outline.Outline(0)
    points = [tuple(outer.CPoint(i)) for i in range(outer.PointCount())]
    clockwise = sum(a[0] * b[1] - a[1] * b[0] for a, b in zip(points, points[1:] + points[:1])) > 0
    checks = []
    for ref in sorted(references):
        require(len(sources[ref]) == 1, f"edge {ref}: the reference import needs exactly one footprint")
        want = zone_signature(pcbnew, sources[ref][0])
        check = {"kind": "edge", "part": ref, "satisfied": False}
        if len(actual[ref]) != 1:
            check["reason"] = "missing_or_ambiguous_footprint"
            checks.append(check)
            continue
        part = actual[ref][0]
        try:
            actual_signature = zone_signature(pcbnew, part)
            zone, bounds, _ = antenna_zone(part)
        except ValueError as error:
            check["reason"] = str(error)
            checks.append(check)
            continue
        antenna = zone.Outline()
        material = pcbnew.SHAPE_POLY_SET()
        material.BooleanIntersection(antenna, board_outline)
        copper = pcbnew.SHAPE_POLY_SET()
        layer = part.GetLayer()
        for pad in part.Pads():
            if pad.IsOnLayer(layer):
                pad.TransformShapeToPolygon(copper, layer, 0, pcbnew.FromMM(pcbnew.ARC_HIGH_DEF_MM),
                                            pcbnew.ERROR_OUTSIDE)
        require(copper.Area() > 0, f"edge {ref}: no mounted-side pad copper to measure")
        box = copper.BBox()
        pads = (box.GetLeft(), box.GetTop(), box.GetRight(), box.GetBottom())
        out = pcbnew.SHAPE_POLY_SET()
        out.BooleanSubtract(copper, board_outline)
        delta = [(bounds[i] + bounds[i + 2]) - (pads[i] + pads[i + 2]) for i in (0, 1)]
        axis = max((0, 1), key=lambda i: abs(delta[i]))
        across = 1 - axis
        sign = 1 if delta[axis] > 0 else -1
        edge_name = {(0, 1): "right", (0, -1): "left", (1, 1): "bottom", (1, -1): "top"}[axis, sign]
        inner = bounds[axis if sign > 0 else axis + 2]
        exposed = []
        for i, (a, b) in enumerate(zip(points, points[1:] + points[:1])):
            if a[axis] != b[axis] or outer.IsArcSegment(i):
                continue
            limit = sorted((a[across], b[across]))
            if limit[0] > pads[across] or limit[1] < pads[across + 2]:
                continue
            normal = (b[1] - a[1], a[0] - b[0])[axis] * (1 if clockwise else -1)
            if sign * normal > 0:
                exposed.append((pcbnew.ToMM(sign * (inner - a[axis])), [list(a), list(b)]))
        gap, edge = min(exposed, key=lambda x: abs(x[0]), default=(None, None))
        shape_matches = want == actual_signature
        check.update(antenna_geometry_matches=shape_matches, facing_edge=edge_name, edge_gap_mm=gap,
                     edge_segment_mm=[[pcbnew.ToMM(v) for v in p] for p in edge] if edge else None,
                     antenna_outline_overlap_mm2=material.Area() / 1e12,
                     pad_copper_outside_board_mm2=out.Area() / 1e12, allowable_edge_gap_mm=OVERHANG_SLACK_MM)
        check["satisfied"] = (shape_matches and abs(delta[axis]) > abs(delta[across]) and gap is not None
                              and -1e-6 <= gap <= OVERHANG_SLACK_MM + 1e-6
                              and material.Area() <= 1 and out.Area() <= 1)
        checks.append(check)
    return checks
