# Copyright (C) 2026 Florian Loitsch.

"""Prove native shape contact along unassigned footprint copper artwork."""

import json
import sys
from collections import Counter, defaultdict, deque
from itertools import combinations
from pathlib import Path

import pcbnew as kicad


MAX_GRAPHICS = 64
MAX_PADS = 128
MAX_POLYGON_VERTICES = 2048
MAX_PATHS = 16
SHAPES = {
    kicad.SHAPE_T_SEGMENT,
    kicad.SHAPE_T_RECT,
    kicad.SHAPE_T_CIRCLE,
    kicad.SHAPE_T_ARC,
    kicad.SHAPE_T_POLY,
}


def identity(item):
    return item.m_Uuid.AsString().lower()


def position(item):
    pos = item.GetPosition()
    return [kicad.ToMM(pos.x), kicad.ToMM(pos.y)]


def pad_contact(pad):
    return {
        "uuid": identity(pad),
        "pin": str(pad.GetNumber()),
        "net": str(pad.GetNetname()),
        "at_mm": position(pad),
    }


def artwork_item(graphic):
    return {
        "uuid": identity(graphic),
        "shape": str(graphic.GetShapeStr()),
        "at_mm": position(graphic),
    }


def plated_annulus_contact(pad, graphic, layer):
    """Prove artwork intersects copper after subtracting the plated hole."""
    if (pad.IsBackdrilledOrPostMachined(layer) or pad.GetSecondaryDrillSize().x > 0
            or pad.GetTertiaryDrillSize().x > 0):
        return False
    error = int(kicad.FromMM(0.001))
    annulus = kicad.SHAPE_POLY_SET()
    hole = kicad.SHAPE_POLY_SET()
    art = kicad.SHAPE_POLY_SET()
    pad.TransformShapeToPolySet(annulus, layer, 0, error, kicad.ERROR_INSIDE)
    if not pad.TransformHoleToPolygon(hole, 0, error, kicad.ERROR_OUTSIDE):
        return False
    annulus.BooleanSubtract(hole)
    graphic.TransformShapeToPolySet(art, layer, 0, error, kicad.ERROR_INSIDE)
    intersection = kicad.SHAPE_POLY_SET()
    intersection.BooleanIntersection(annulus, art)
    return intersection.Area() > 0


def shortest_paths(starts, adjacent):
    paths = {start: [start] for start in sorted(starts)}
    pending = deque(paths)
    while pending:
        current = pending.popleft()
        for neighbor in sorted(adjacent[current]):
            if neighbor not in paths:
                paths[neighbor] = paths[current] + [neighbor]
                pending.append(neighbor)
    return paths


def prove_footprint_layer(board, footprint, layer, witnesses):
    # A KiCad net-tie declaration authorizes copper joins in this land pattern.
    # Leave those footprints to the native net-tie rule checker.
    if footprint.IsNetTie():
        return [], None
    graphics = [g for g in footprint.GraphicalItems()
                if isinstance(g, kicad.PCB_SHAPE) and g.GetLayer() == layer
                and g.GetNetCode() == 0 and not g.GetNetname()]
    # Scope drilled-pad work to the native short and check the copper ring.
    plated_contacts = {(w["finding_pad_uuid"], w["graphic_uuid"]) for w in witnesses
                       if w["kind"] == "shorting_items"}
    witnessed_pads = {pad for pad, _ in plated_contacts}
    pads = [p for p in footprint.Pads() if p.IsOnLayer(layer)
            and (p.GetAttribute() in (kicad.PAD_ATTRIB_SMD, kicad.PAD_ATTRIB_CONN)
                 or p.GetAttribute() == kicad.PAD_ATTRIB_PTH and identity(p) in witnessed_pads)
            and p.FlashLayer(layer) and p.GetNetCode() > 0
            and p.GetNetname() and p.GetNumber()]
    if len(graphics) > MAX_GRAPHICS or len(pads) > MAX_PADS:
        return [], "footprint exceeds the native artwork probe's item limit"
    if len({str(pad.GetNetname()) for pad in pads}) < 2:
        return [], None
    # A graph vertex represents one continuous shape. Multiple disconnected
    # polygon outlines in one native item would invalidate a path through it.
    connected_graphics = []
    vertices = 0
    for graphic in graphics:
        kind = graphic.GetShape()
        if kind not in SHAPES:
            continue
        if kind == kicad.SHAPE_T_POLY:
            polygon = graphic.GetPolyShape()
            if polygon.OutlineCount() != 1 or not graphic.IsPolyShapeValid():
                continue
            vertices += polygon.TotalVertices()
        connected_graphics.append(graphic)
    if vertices > MAX_POLYGON_VERTICES:
        return [], "footprint exceeds the native artwork probe's polygon limit"
    all_ids = Counter(identity(item) for item in graphics + pads)
    if any(count != 1 for count in all_ids.values()):
        return [], "footprint contains ambiguous native copper identities"
    graphic_by_id = {identity(g): g for g in connected_graphics}
    pad_by_id = {identity(p): p for p in pads}
    graphic_shapes = {uid: item.GetEffectiveShape(layer) for uid, item in graphic_by_id.items()}
    pad_shapes = {uid: item.GetEffectiveShape(layer) for uid, item in pad_by_id.items()}
    contacts = {
        pad: {uid for uid, shape in graphic_shapes.items() if copper.Collide(shape, 0)
              and (pad_by_id[pad].GetAttribute() != kicad.PAD_ATTRIB_PTH
                   or (pad, uid) in plated_contacts
                   and plated_annulus_contact(pad_by_id[pad], graphic_by_id[uid], layer))}
        for pad, copper in pad_shapes.items()
    }
    direct = defaultdict(dict)
    siblings = []
    for witness in witnesses:
        pad = witness["open_pad_uuid"]
        found_pad, graphic = witness["finding_pad_uuid"], witness["graphic_uuid"]
        if not contacts.get(pad) or graphic not in contacts.get(found_pad, ()):
            continue
        if pad == found_pad:
            direct[pad][graphic] = witness
        elif witness["kind"] == "shorting_items":
            siblings.append(witness)
    if not direct and not siblings:
        return [], None
    adjacent = defaultdict(set)
    for a, b in combinations(graphic_shapes, 2):
        if graphic_shapes[a].Collide(graphic_shapes[b], 0):
            adjacent[a].add(b)
            adjacent[b].add(a)
    candidate_paths = []
    for start, hits in sorted(direct.items()):
        paths = shortest_paths(hits, adjacent)
        for end, end_contacts in sorted(contacts.items()):
            if pad_by_id[start].GetNetname() == pad_by_id[end].GetNetname():
                continue
            path = min((paths[uid] for uid in end_contacts if uid in paths),
                       key=lambda path: (len(path), path), default=None)
            if path is not None:
                candidate_paths.append((start, end, path, hits[path[0]]))
    for witness in siblings:
        start, end = witness["open_pad_uuid"], witness["finding_pad_uuid"]
        if pad_by_id[start].GetNetname() == pad_by_id[end].GetNetname():
            continue
        path = shortest_paths(contacts[start], adjacent).get(witness["graphic_uuid"])
        if path is not None:
            candidate_paths.append((start, end, path, witness))

    seen = set()
    result = []
    ref = str(footprint.GetReference())
    fpid = footprint.GetFPID()
    nickname = str(fpid.GetLibNickname())
    model = str(fpid.GetLibItemName())
    footprint_id = f"{nickname}:{model}" if nickname else model
    for start, end, path, witness in candidate_paths:
        pair = tuple(sorted((start, end)))
        if pair in seen:
            continue
        if len(result) >= MAX_PATHS:
            return result, "additional native artwork paths exceed the reporting limit"
        seen.add(pair)
        result.append({
            "footprint": ref,
            "footprint_id": footprint_id,
            "layer": board.GetLayerName(layer),
            "from": pad_contact(pad_by_id[start]),
            "to": pad_contact(pad_by_id[end]),
            "native_finding": {
                "kind": witness["kind"],
                "pad_uuid": witness["finding_pad_uuid"],
                "graphic_uuid": witness["graphic_uuid"],
            },
            "artwork_path": [artwork_item(graphic_by_id[uid]) for uid in path],
        })
    return result, None


def main():
    path, request, output, expected_version = sys.argv[1:]
    version = kicad.GetBuildVersion().strip()
    if version != expected_version:
        raise ValueError("KiCad CLI report and PCB bindings have different versions")
    board = kicad.LoadBoard(path)
    if not board:
        raise ValueError("KiCad could not load the board for the artwork contact check")
    requests = json.loads(Path(request).read_text(encoding="utf-8"))
    targets = defaultdict(list)
    for witness in requests:
        targets[(witness["footprint"], witness["layer"])].append(witness)
    refs = defaultdict(list)
    for footprint in board.GetFootprints():
        refs[str(footprint.GetReference())].append(footprint)
    proofs, skipped = [], []
    for (ref, layer_name), witnesses in sorted(targets.items()):
        layer = board.GetLayerID(layer_name)
        if len(refs[ref]) != 1 or not board.IsLayerEnabled(layer):
            skipped.append(f"{ref} on {layer_name} has no unique native footprint or active layer")
            continue
        result, stopping_point = prove_footprint_layer(board, refs[ref][0], layer, witnesses)
        proofs.extend(result)
        if stopping_point:
            skipped.append(f"{ref} on {layer_name}: {stopping_point}")
    Path(output).write_text(json.dumps({"paths": proofs, "skipped": skipped}), encoding="utf-8")


if __name__ == "__main__":
    main()
