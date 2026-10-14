# Copyright (C) 2026 Toit contributors.
"""The fill islands behind KiCad's starved thermals, and what could join them.

    python3 -I islands.py <board.kicad_pcb> <drc.json> [--designer <board.kicad_pcb>]

For every pad the DRC reports starved, on the refilled board: the fill
outlines the pad's ring crosses (as the DRC finds them), each with its
area, whether it is a single-connection outline (KiCad's "isolated
island"), and what cuts it off: the other nets' items within the zone's
clearance and minimum width of it, by kind (track on the layer, via, pad)
and whether they are routed (tracks, vias) or fixed (pads). Then what
could make the island count:

- a stitching via: a spot in the island where a via of the net's class
  keeps its clearance on every layer and lands in kept fill of the net on
  another layer (the island then holds the pad and the via);
- a via anywhere in the island (a lone via, joined to the pad by a track on
  another layer);
- the pad's own track on the layer: the DRC joins the pad and its track,
  two items, so the island is no longer single.

With `--designer`, the same pad's spokes on the designer's board (the
original placement and routing), to tell routing-made islands from
footprint-made ones.
"""

import argparse
import json
import math
import os
import sys

import pcbnew

sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
import spokes  # noqa: E402

NM = 1.0e6


def mm(value):
    return value / NM


def point(x, y):
    return pcbnew.VECTOR2I(int(round(x * NM)), int(round(y * NM)))


def pads_by_label(board):
    result = {}
    for footprint in board.GetFootprints():
        for pad in footprint.Pads():
            result.setdefault(spokes.pad_label(pad), []).append(pad)
    return result


def net_rules(board, net_code):
    """(track width, clearance, via diameter, via drill) of the net's class,
    in nm, from the project file next to the board."""
    project = os.path.splitext(board.GetFileName())[0] + ".kicad_pro"
    settings = json.load(open(project)).get("net_settings", {})
    name = str(board.FindNet(net_code).GetNetname())
    classes = {entry["name"]: entry for entry in settings.get("classes", [])}
    chosen = classes.get("Default", {})
    for pattern in settings.get("netclass_patterns", []) or []:
        import fnmatch
        if fnmatch.fnmatchcase(name, pattern["pattern"]) and pattern["netclass"] in classes:
            chosen = classes[pattern["netclass"]]
            break
    assigned = (settings.get("netclass_assignments") or {}).get(name)
    if isinstance(assigned, list) and assigned:
        assigned = assigned[0]
    if assigned in classes:
        chosen = classes[assigned]
    get = lambda key, default: int(round(chosen.get(key, default) * NM))
    return get("track_width", 0.25), get("clearance", 0.2), get("via_diameter", 0.6), get("via_drill", 0.3)


def outlines_of_pad(board, zones, zone, layer, pad):
    """The fill outlines the pad's ring (half the gap) crosses, with their
    single-connection verdict, as spokes.py counts them."""
    polys = zone.GetFilledPolysList(layer)
    gap_override = pad.GetLocalThermalGapOverride()
    gap = gap_override if isinstance(gap_override, int) and gap_override > 0 else zone.GetThermalReliefGap()
    ring = pcbnew.SHAPE_POLY_SET()
    pad.TransformShapeToPolygon(ring, layer, gap // 2, 5000, pcbnew.ERROR_OUTSIDE)
    chain = ring.Outline(0)
    items = [item for item in list(board.GetTracks()) + [p for f in board.GetFootprints() for p in f.Pads()] if item.GetNetCode() == zone.GetNetCode()]
    result = []
    for index in range(polys.OutlineCount()):
        outline = polys.Outline(index)
        if not outline.BBox().Intersects(chain.BBox()):
            continue
        crossings = spokes.segment_crossings(outline, chain)
        if len(crossings) < 2:
            continue
        single = spokes.outline_items(board, zones, zone, layer, polys, index, items) == 1
        result.append((index, len(set(crossings)) // 2, single))
    return polys, result


def cutters(board, zone, layer, unit, reach):
    """Other nets' items within `reach` of the island, by kind."""
    box = unit.BBox()
    box.Inflate(int(reach))
    found = {}
    for item in list(board.GetTracks()) + [pad for footprint in board.GetFootprints() for pad in footprint.Pads()]:
        if item.GetNetCode() == zone.GetNetCode() or not item.IsOnLayer(layer):
            continue
        if not box.Intersects(item.GetBoundingBox()):
            continue
        if isinstance(item, pcbnew.PAD) and not item.FlashLayer(layer):
            continue
        shape = item.GetEffectiveShape(layer)
        if not shape.Collide(unit, int(reach)):
            continue
        if isinstance(item, pcbnew.PCB_VIA):
            kind = "via"
        elif isinstance(item, pcbnew.PCB_TRACK):
            kind = "track"
        else:
            kind = "pad"
        found.setdefault(kind, set()).add(str(item.GetNetname()) if kind != "pad" else spokes.pad_label(item))
    return {kind: sorted(names) for kind, names in found.items()}


def via_spots(board, zones, zone, layer, unit, net_code, rules, pad):
    """Where in the island a via of the net's class fits: the island less
    the other nets' copper on every layer grown by the clearance and the
    via's radius, and less every hole grown by the hole-to-hole distance.
    Returns (area where a via fits, area of that where it also lands in a
    fill outline of the net with another item on another layer: a stitch,
    the nearest such spot to the pad)."""
    width, clearance, diameter, drill = rules
    radius = diameter // 2
    box = unit.BBox()
    grow = radius + clearance + int(2 * NM)
    area = pcbnew.BOX2I(box.GetPosition(), box.GetSize())
    area.Inflate(grow)
    copper = [layer_id for layer_id in board.GetEnabledLayers().Seq() if pcbnew.IsCopperLayer(layer_id)]
    settings = board.GetDesignSettings()
    forbidden = pcbnew.SHAPE_POLY_SET()
    near = [item for item in list(board.GetTracks()) + [p for f in board.GetFootprints() for p in f.Pads()] if area.Intersects(item.GetBoundingBox())]
    for item in near:
        if item.GetNetCode() != net_code:
            for layer_id in copper:
                if item.IsOnLayer(layer_id) and (not isinstance(item, pcbnew.PAD) or item.FlashLayer(layer_id)):
                    item.TransformShapeToPolygon(forbidden, layer_id, clearance + radius, 5000, pcbnew.ERROR_OUTSIDE)
        if isinstance(item, pcbnew.PCB_VIA) or (isinstance(item, pcbnew.PAD) and item.HasHole()):
            hole = item.GetEffectiveHoleShape()
            centre = hole.Centre() if hasattr(hole, "Centre") else item.GetPosition()
            reach = settings.m_HoleToHoleMin + drill // 2 + max(item.GetDrillSize().x if isinstance(item, pcbnew.PAD) else item.GetDrillValue(), 0) // 2
            if item.GetNetCode() != net_code:
                reach = max(reach, settings.m_HoleClearance + radius + (item.GetDrillSize().x if isinstance(item, pcbnew.PAD) else item.GetDrillValue()) // 2)
            circle = pcbnew.SHAPE_POLY_SET()
            circle.NewOutline()
            for step in range(24):
                angle = 2 * math.pi * step / 24
                circle.Append(int(item.GetPosition().x + reach * math.cos(angle)), int(item.GetPosition().y + reach * math.sin(angle)))
            forbidden.BooleanAdd(circle)
    fits = pcbnew.SHAPE_POLY_SET(unit)
    fits.BooleanSubtract(forbidden)
    stitch = pcbnew.SHAPE_POLY_SET()
    for other in copper:
        if other == layer:
            continue
        for other_zone in zones:
            if other_zone.GetNetCode() != net_code or not other_zone.IsOnLayer(other):
                continue
            polys = other_zone.GetFilledPolysList(other)
            for index in range(polys.OutlineCount()):
                if not polys.Outline(index).BBox().Intersects(box):
                    continue
                if spokes.outline_items(board, zones, other_zone, other, polys, index, [i for i in list(board.GetTracks()) + [p for f in board.GetFootprints() for p in f.Pads()] if i.GetNetCode() == net_code]) >= 2:
                    piece = pcbnew.SHAPE_POLY_SET(fits)
                    piece.BooleanIntersection(polys.UnitSet(index))
                    stitch.BooleanAdd(piece)
    nearest = None
    for index in range(stitch.OutlineCount()):
        chain = stitch.Outline(index)
        for vertex in range(chain.PointCount()):
            at = chain.CPoint(vertex)
            distance = math.hypot(at.x - pad.GetPosition().x, at.y - pad.GetPosition().y) / NM
            if nearest is None or distance < nearest[0]:
                nearest = (round(distance, 2), round(at.x / NM, 3), round(at.y / NM, 3))
    return round(fits.Area() / NM / NM, 4), round(stitch.Area() / NM / NM, 4), nearest


def main():
    parser = argparse.ArgumentParser()
    parser.add_argument("board")
    parser.add_argument("drc")
    parser.add_argument("--designer")
    parser.add_argument("--json")
    args = parser.parse_args()
    board = pcbnew.LoadBoard(args.board)
    zones = spokes.fill(board)
    designer = None
    if args.designer:
        designer = pcbnew.LoadBoard(args.designer)
        designer_zones = spokes.fill(designer)
        designer_rows = spokes.kicad_spokes(designer, designer_zones)
    pads = pads_by_label(board)
    rows = []
    for (label, layer_name), reports in sorted(spokes.starved_from_drc(args.drc).items()):
        layer = board.GetLayerID(layer_name)
        for position, description in reports:
            candidates = pads.get(label, [])
            if not candidates:
                continue
            pad = min(candidates, key=lambda pad: (mm(pad.GetPosition().x) - position[0]) ** 2 + (mm(pad.GetPosition().y) - position[1]) ** 2)
            net = pad.GetNetCode()
            zone = next((zone for zone in zones if zone.GetNetCode() == net and zone.IsOnLayer(layer) and zone.GetFilledPolysList(layer).Collide(pad.GetPosition(), int(5 * NM))), None)
            if zone is None:
                continue
            rules = net_rules(board, net)
            polys, outlines = outlines_of_pad(board, zones, zone, layer, pad)
            own_tracks = [track for track in board.GetConnectivity().GetConnectedTracks(pad) if track.IsOnLayer(layer) and not isinstance(track, pcbnew.PCB_VIA)]
            row = {"pad": label, "layer": layer_name, "net": str(pad.GetNetname()), "at": [mm(pad.GetPosition().x), mm(pad.GetPosition().y)],
                   "drill": mm(pad.GetDrillSize().x) if pad.HasHole() else None, "description": description.split("(")[-1].rstrip(")"),
                   "own_tracks_on_layer": len(own_tracks), "outlines": []}
            reach = zone.GetLocalClearance() or 0
            reach = max(reach, rules[1]) + zone.GetMinThickness() + int(0.02 * NM)
            for index, count, single in outlines:
                unit = polys.UnitSet(index)
                entry = {"spokes": count, "single": single, "area": round(unit.Area() / NM / NM, 3)}
                if single:
                    entry["cut_by"] = cutters(board, zone, layer, unit, reach)
                    fits, stitch, nearest = via_spots(board, zones, zone, layer, unit, net, rules, pad)
                    entry["via_area"] = fits
                    entry["stitch_area"] = stitch
                    entry["nearest_stitch"] = nearest
                row["outlines"].append(entry)
            if designer is not None:
                key = [key for key in designer_rows if key[0] == label and key[1] == layer_name]
                if key:
                    other = designer_rows[key[0]]
                    row["designer"] = {"kicad": other["kicad"], "single": other["kicad_single"], "tracks": other["kicad_tracks"]}
            rows.append(row)
            print(json.dumps(row))
    if args.json:
        json.dump(rows, open(args.json, "w"), indent=1)


if __name__ == "__main__":
    main()
