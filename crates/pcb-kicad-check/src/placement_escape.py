# Copyright (C) 2026 Florian Loitsch.

"""Screen a short opposite-face escape on the exact copper-free placement."""

from collections import Counter
from itertools import product
import json
from math import hypot
from pathlib import Path
import sys

import pcbnew as k


MM = 1_000_000
OUTER = (k.F_Cu, k.B_Cu)


def point(p):
    return p.x, p.y


def vector(p):
    return k.VECTOR2I(*p)


def mm(p):
    return [v / MM for v in p]


def uuid(item):
    return item.m_Uuid.AsString().lower()


def footprint_geometry(board):
    return sorted((uuid(part), part.GetReference(), part.GetFPIDAsString(),
                   point(part.GetPosition()), part.GetOrientationDegrees(), part.GetLayer(),
                   sorted((uuid(p), p.GetNumber(), p.GetNetname(), point(p.GetPosition()),
                           point(p.GetSize()), point(p.GetDrillSize()), p.GetShape(),
                           p.GetOrientationDegrees(), p.GetLayerSet().FmtBin()) for p in part.Pads()))
                  for part in board.GetFootprints())


def close_to(p, box, radius):
    x, y = p
    return (box.GetX() - radius <= x <= box.GetRight() + radius
            and box.GetY() - radius <= y <= box.GetBottom() + radius)


def add(p, d):
    return p[0] + d[0], p[1] + d[1]


def local_shapes(board, center, endpoints):
    rows = []
    for part in board.GetFootprints():
        for pad in part.Pads():
            if uuid(pad) not in endpoints and close_to(center, pad.GetBoundingBox(), 5 * MM):
                for layer in OUTER:
                    if pad.IsOnLayer(layer):
                        rows.append((part.GetReference(), pad.GetNumber(), layer,
                                     pad.GetEffectiveShape(layer)))
    return rows


def sites(center, outward):
    axis, sign = outward
    for longitudinal, lateral in product(range(6, 26), range(-8, 9)):
        offset = [0, 0]
        offset[axis] = sign * longitudinal * 100000
        offset[1-axis] = lateral * 100000
        yield add(center, offset)


def screen(shapes, via, front_track, rear_track, clearance):
    return {(ref, pin) for ref, pin, layer, shape in shapes
            if k.SHAPE.Collide(via, shape, clearance)
            or k.SHAPE.Collide(front_track if layer == k.F_Cu else rear_track, shape, clearance)}


def unsupported_artwork(board, origin):
    zones = list(board.Zones())
    drawings = list(board.GetDrawings())
    for part in board.GetFootprints():
        zones.extend(part.Zones())
        drawings.extend(part.GraphicalItems())
    return any(drawing.IsOnCopperLayer() and close_to(origin, drawing.GetBoundingBox(), 4 * MM)
               for drawing in drawings) or any(
                   zone.GetIsRuleArea() and (zone.GetDoNotAllowTracks() or zone.GetDoNotAllowVias()
                                             or zone.GetDoNotAllowFootprints())
                   and close_to(origin, zone.GetBoundingBox(), 4 * MM) for zone in zones)


def solve(board, routed, request):
    if list(board.GetTracks()) or footprint_geometry(board) != footprint_geometry(routed):
        return {'status': 'placed_board_geometry_does_not_match'}
    placed_outline = board.GetBoardEdgesBoundingBox()
    routed_outline = routed.GetBoardEdgesBoundingBox()
    if (point(placed_outline.GetOrigin()) != point(routed_outline.GetOrigin())
            or point(placed_outline.GetSize()) != point(routed_outline.GetSize())):
        return {'status': 'placed_board_outline_does_not_match'}
    front_id, rear_id = request['pads']
    pads = [p for p in board.GetPads() if p.GetNetname() == request['net']]
    if len(pads) != 2 or {uuid(p) for p in pads} != {front_id, rear_id}:
        return {'status': 'requires_exactly_two_physical_pads'}
    if any(p.GetAttribute() != k.PAD_ATTRIB_SMD
           or sum(p.IsOnLayer(layer) for layer in OUTER) != 1 for p in pads):
        return {'status': 'requires_two_surface_pads_on_opposite_faces'}
    by_layer = {layer: p for p in pads for layer in OUTER if p.IsOnLayer(layer)}
    if set(by_layer) != set(OUTER):
        return {'status': 'requires_two_surface_pads_on_opposite_faces'}
    front, rear = [by_layer[layer] for layer in OUTER]
    origin, goal = (point(p.GetPosition()) for p in (front, rear))
    center = point(front.GetParentFootprint().GetPosition())
    delta = [a - b for a, b in zip(origin, center)]
    axis = max(range(2), key=lambda i: abs(delta[i]))
    if abs(delta[axis]) < 100000 or hypot(*[a-b for a, b in zip(origin, goal)]) > 1.5 * MM:
        return {'status': 'requires_overlapping_local_endpoint_neighbourhoods'}

    width = round(request['width_mm'] * MM)
    radius = round(request['via_size_mm'] * MM / 2)
    clearance = max(0, round(request['clearance_mm'] * MM) - 1)
    courtyard_gap = max(0, round(request['courtyard_spacing_mm'] * MM) - 1)
    if not close_to(origin, placed_outline, -4 * MM):
        return {'status': 'outside_local_interior_screen'}
    if unsupported_artwork(board, origin):
        return {'status': 'local_copper_artwork_or_keepout_requires_native_review'}
    endpoint_shapes = [p.GetEffectiveShape(layer) for layer, p in by_layer.items()]
    shapes = local_shapes(board, origin, {front_id, rear_id})
    motions = []
    collisions = Counter()
    already_open = 0
    for candidate in sites(origin, (axis, 1 if delta[axis] > 0 else -1)):
        via = k.SHAPE_CIRCLE(vector(candidate), radius)
        if any(k.SHAPE.Collide(via, shape, 0) for shape in endpoint_shapes):
            continue
        tracks = [k.SHAPE_SEGMENT(vector(end), vector(candidate), width) for end in (origin, goal)]
        blocked = screen(shapes, via, *tracks, clearance)
        if not blocked:
            already_open += 1
        elif len({ref for ref, _ in blocked}) == 1:
            owner = next(iter(blocked))[0]
            collisions[owner] += 1
            motions.append((owner, candidate, via, tracks, sorted(pin for _, pin in blocked)))
    if already_open:
        return {'status': 'original_placement_has_short_paths', 'original_candidate_count': already_open}

    # A single eligible body must clear every pad collision for the same
    # path. Search the nearest such motions in fixed 0.25 mm steps.
    allowed = set(request['movable']) - {p.GetParentFootprint().GetReference() for p in pads}
    parts = {p.GetReference(): p for p in board.GetFootprints()}
    if len(parts) != len(list(board.GetFootprints())):
        return {'status': 'ambiguous_footprint_references'}
    names = [name for name, _ in collisions.most_common(3) if name in allowed]
    moves = sorted(((x, y) for x, y in product(range(-4, 5), repeat=2) if 0 < hypot(x, y) <= 4),
                   key=lambda offset: (hypot(*offset), offset))
    answer = []
    for name in names:
        part = parts.get(name)
        if (part is None or part.GetLayer() != k.F_Cu or part.IsLocked()
                or part.GetParentGroup()
                or not all(p.GetAttribute() == k.PAD_ATTRIB_SMD and p.IsOnLayer(k.F_Cu)
                           and not p.IsOnLayer(k.B_Cu) for p in part.Pads())):
            continue
        begin = point(part.GetPosition())
        nearby = [(p.GetReference(), p.GetCourtyard(k.F_Cu)) for p in parts.values()
                  if p.GetReference() != name and p.GetLayer() == k.F_Cu
                  and close_to(begin, p.GetBoundingBox(False, False), 6 * MM)]
        courtyards = [outline for _, outline in nearby if outline.OutlineCount()]
        missing_courtyards = sorted(ref for ref, outline in nearby if not outline.OutlineCount())
        other_copper = [shape for ref, _, layer, shape in local_shapes(board, begin, set())
                        if ref != name and layer == k.F_Cu]
        for dx, dy in moves:
            where = add(begin, (dx*250000, dy*250000))
            part.SetPosition(vector(where))
            courtyard = part.GetCourtyard(k.F_Cu)
            if not courtyard.OutlineCount() or any(k.SHAPE.Collide(courtyard, p, courtyard_gap) for p in courtyards):
                continue
            moved = [(name, p.GetNumber(), k.F_Cu, p.GetEffectiveShape(k.F_Cu)) for p in part.Pads()]
            if any(k.SHAPE.Collide(pad, other, clearance) for _, _, _, pad in moved for other in other_copper):
                continue
            clear = [(candidate, pins) for owner, candidate, via, tracks, pins in motions
                     if owner == name and not screen(moved, via, *tracks, clearance)]
            if clear:
                via, pins = min(clear, key=lambda option: hypot(*[a-b for a, b in zip(origin, option[0])]))
                distance = hypot(dx, dy) * 0.25
                answer.append((len(clear), distance, name, {
                    'status': 'same_face_move_screened', 'original_candidate_count': 0,
                    'moved_candidate_count': len(clear), 'footprint': name, 'footprint_uuid': uuid(part),
                    'original_pose_mm': mm(begin), 'face': 'front', 'blocking_pads': pins,
                    'nearby_missing_courtyards': missing_courtyards,
                    'constraints_patch': {'place': [{'part': name, 'x': where[0] / MM, 'y': where[1] / MM,
                                                      'angle': part.GetOrientationDegrees(), 'origin': 'absolute'}]},
                    'witness': {'front_pad_mm': mm(origin), 'rear_pad_mm': mm(goal), 'via_mm': mm(via),
                                'track_width_mm': request['width_mm'], 'via_size_mm': request['via_size_mm'],
                                'via_drill_mm': request['via_drill_mm']},
                }))
        part.SetPosition(vector(begin))
    if not answer:
        return {'status': 'no_short_path_with_a_single_eligible_move', 'original_candidate_count': 0}
    return sorted(answer, key=lambda row: (-row[0], row[1], row[2]))[0][3]


if __name__ == '__main__':
    path, request_path, output, version = sys.argv[1:]
    if k.GetBuildVersion().strip() != version:
        raise ValueError('KiCad CLI and PCB bindings have different versions')
    request = json.loads(Path(request_path).read_text())
    result = solve(k.LoadBoard(path), k.LoadBoard(request['routed']), request)
    Path(output).write_text(json.dumps(result), encoding='utf-8')
