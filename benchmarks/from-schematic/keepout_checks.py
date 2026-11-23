# Copyright (C) 2026 Florian Loitsch.
"""Read back authored component and optional copper keepouts independently."""

import math


FLAGS = ("Tracks", "Vias", "Pads", "ZoneFills", "Footprints")


def areas_for(entries, corner):
    from returned_board import numbers, require

    require(isinstance(entries, list), "keepout must be a list")
    areas = []
    for ordinal, entry in enumerate(entries, 1):
        require(isinstance(entry, dict) and entry.keys() <= {"x", "y", "origin", "side", "copper"},
                f"keepout {ordinal} must be a rectangular x/y box")
        origin = entry.get("origin")
        side, copper = entry.get("side"), entry.get("copper", False)
        side = "both" if side is None else side
        require(origin in (None, "absolute", "board"), f"unsupported keepout origin {origin!r}")
        require(side in ("front", "back", "both"), f"unsupported keepout side {side!r}")
        require(type(copper) is bool, "keepout copper must be true or false")
        x, y = (0, 0) if origin == "absolute" else corner
        x0, x1 = sorted(numbers(entry.get("x"), 2, "keepout x"))
        y0, y1 = sorted(numbers(entry.get("y"), 2, "keepout y"))
        require(x0 < x1 and y0 < y1, f"keepout {ordinal} is empty")
        areas.append({"name": f"constraint keepout {ordinal}", "bounds_mm": [x + x0, y + y0, x + x1, y + y1],
                      "side": side, "copper": copper})
    return areas


def copper_layers_for(area, pcbnew, board):
    if area["side"] == "front":
        return [pcbnew.F_Cu]
    if area["side"] == "back":
        return [pcbnew.B_Cu]
    if area["copper"]:
        return list(board.GetEnabledLayers().CuStack())
    return [pcbnew.F_Cu, pcbnew.B_Cu]


def rectangle_corners(bounds):
    x0, y0, x1, y1 = bounds
    return [(x0, y0), (x1, y0), (x1, y1), (x0, y1)]


def native_corners(pcbnew, bounds):
    from returned_board import require

    x0, y0, x1, y1 = map(pcbnew.FromMM, bounds)
    require(x0 < x1 and y0 < y1, "rectangle needs a positive area at KiCad precision")
    return rectangle_corners([x0, y0, x1, y1])


def polygon_for(pcbnew, bounds):
    polygon = pcbnew.SHAPE_POLY_SET()
    polygon.NewOutline()
    for x, y in native_corners(pcbnew, bounds):
        polygon.Append(x, y)
    return polygon


def held_overlaps(pcbnew, board, area, exact_refs):
    """Core allows authored held footprints whose courtyard box meets the area."""
    area_box = polygon_for(pcbnew, area["bounds_mm"]).BBox()
    layers = [layer for layer in copper_layers_for(area, pcbnew, board) if layer in (pcbnew.F_Cu, pcbnew.B_Cu)]
    overlaps = []
    for part in board.GetFootprints():
        if part.GetReference() not in exact_refs:
            continue
        for layer in layers:
            court = part.GetCourtyard(layer)
            if not court.OutlineCount():
                continue
            box = court.BBox()
            if (max(area_box.GetX(), box.GetX()) < min(area_box.GetRight(), box.GetRight())
                    and max(area_box.GetY(), box.GetY()) < min(area_box.GetBottom(), box.GetBottom())):
                overlaps.append(part.GetReference())
                break
    return sorted(overlaps)


def body_separation(bounds, part):
    """Signed separation using describe's rotated box or circular body in mm."""
    from returned_board import numbers, require

    x0, y0, x1, y1 = bounds
    circle = part.get("body_circle")
    if circle is not None:
        require(isinstance(circle, list) and len(circle) == 2, "invalid footprint circle")
        (x, y), (radius,) = numbers(circle[0], 2, "body circle center"), numbers([circle[1]], 1, "body circle radius")
        require(radius > 0, "body circle must have positive radius")
        return math.hypot(x - min(max(x, x0), x1), y - min(max(y, y0), y1)) - radius
    corners = part.get("body_corners")
    require(isinstance(corners, list) and len(corners) == 4, "no rotated footprint body evidence")
    corners = [numbers(point, 2, "body corner") for point in corners]
    angle = math.radians(numbers(part.get("at"), 3, "footprint pose")[2])
    sine, cosine = math.sin(angle), math.cos(angle)
    rectangle = rectangle_corners(bounds)
    gaps = []
    for a, b in ((1, 0), (0, 1), (cosine, -sine), (sine, cosine)):
        body = [a * x + b * y for x, y in corners]
        area = [a * x + b * y for x, y in rectangle]
        gaps.append(max(min(body) - max(area), min(area) - max(body)))
    return max(gaps)


def far_side_meets(pcbnew, part, area, layer):
    """Flag backside evidence for review. Solver lead margins are not reproduced."""
    courtyard = part.GetCourtyard(layer)
    if courtyard.OutlineCount() and area.Collide(courtyard):
        return True
    if any(pad.IsOnLayer(layer) and area.Collide(pad.GetEffectiveShape(layer)) for pad in part.Pads()):
        return True
    return any(graphic.GetLayer() == layer and area.BBox().Intersects(graphic.GetBoundingBox())
               for graphic in part.GraphicalItems())


def copper_intrusions(pcbnew, board, bounds, layers):
    area = polygon_for(pcbnew, bounds)
    collisions = []
    for track in board.GetTracks():
        via = isinstance(track, pcbnew.PCB_VIA)
        for layer in layers:
            if track.IsOnLayer(layer) and area.Collide(track.GetEffectiveShape(layer)):
                collisions.append({"kind": "via" if via else "track", "uuid": track.m_Uuid.AsString(),
                                   "layer": board.GetStandardLayerName(layer)})
    zones = list(board.Zones()) + [zone for part in board.GetFootprints() for zone in part.Zones()]
    for zone in zones:
        if zone.GetIsRuleArea():
            continue
        for layer in layers:
            if not zone.IsOnLayer(layer) or not zone.HasFilledPolysForLayer(layer):
                continue
            intersection = pcbnew.SHAPE_POLY_SET()
            intersection.BooleanIntersection(area, zone.GetFilledPolysList(layer))
            if intersection.Area() > 0:
                collisions.append({"kind": "pour", "uuid": zone.m_Uuid.AsString(),
                                   "layer": board.GetStandardLayerName(layer)})
    return collisions


def measure(pcbnew, candidate, areas, description, exact_refs, refill=None):
    from returned_board import require

    board = pcbnew.LoadBoard(str(candidate))
    require(board is not None, "KiCad could not load the candidate for keepout checks")
    native_parts = {part.m_Uuid.AsString(): part for part in board.GetFootprints()}
    filled = pcbnew.LoadBoard(str(refill)) if refill is not None else None
    output = []
    for area in areas:
        bounds = area["bounds_mm"]
        polygon = polygon_for(pcbnew, bounds)
        target = pcbnew.B_Cu if area["side"] == "back" else pcbnew.F_Cu
        held = held_overlaps(pcbnew, board, area, exact_refs)
        conflicts, uncertain = [], []
        for part in description["footprints"]:
            ref = part["reference"]
            if ref in held:
                continue
            side = part.get("side")
            require(side in ("front", "back"), f"missing side evidence for {ref}")
            if area["side"] in ("both", side):
                gap = body_separation(bounds, part)
                # Each coordinate of describe's body is rounded to a micron.
                if gap < -0.001:
                    conflicts.append(ref)
                elif gap <= 0.001:
                    uncertain.append({"part": ref, "reason": "body within 0.001 mm of the keepout boundary"})
            else:
                native = native_parts.get(part.get("uuid"))
                require(native is not None, f"no native opposite-side geometry for {ref}")
                if far_side_meets(pcbnew, native, polygon, target):
                    uncertain.append({"part": ref, "reason": "opposite-side pad or courtyard geometry reaches the keepout"})
        copper = []
        if area["copper"]:
            require(filled is not None, "no native refilled copper evidence for the keepout")
            for label, measured in (("submitted", board), ("refilled", filled)):
                layers = copper_layers_for(area, pcbnew, measured)
                copper.extend({**item, "evidence": label} for item in copper_intrusions(pcbnew, measured, bounds, layers))
        output.append({"kind": "keepout", **area, "satisfied": not (conflicts or uncertain or copper),
                       "encroaching_parts": sorted(conflicts), "indeterminate_parts": uncertain,
                       "permitted_held_parts": held, "copper_intrusions": copper})
    return output
