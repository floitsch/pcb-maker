# Copyright (C) 2026 Florian Loitsch.

"""Screen a small bridge or fill patch around a native hatch width chord."""

import json
import math
import sys
from pathlib import Path

import pcbnew


def vector(x, y):
    return pcbnew.VECTOR2I(round(x), round(y))


def candidate(board, target, zone, layer, obstacles, clearance):
    min_width = pcbnew.FromMM(target["minimum_mm"])
    ((ax, ay), (bx, by)) = [
        [pcbnew.FromMM(value) for value in point] for point in target["chord_mm"]
    ]
    dx, dy = bx - ax, by - ay
    length = math.hypot(dx, dy)
    if not 0 < length < min_width:
        return None
    x, y = (ax + bx) / 2, (ay + by) / 2
    midpoint = vector(x, y)
    filled = zone.GetFilledPolysList(layer)
    if not 0 < filled.OutlineCount() <= 10_000 or not zone.Outline().Contains(midpoint):
        return None
    distance, nearest = min(
        (filled.SquaredDistanceToPolygon(midpoint, index, vector(0, 0)), index)
        for index in range(filled.OutlineCount())
    )
    if distance > (length / 2 + pcbnew.FromMM(0.01))**2:
        return None

    # A trace on an isolated hatch polygon can turn floating copper into
    # a required open. Let the filler shape a local solid patch there.
    if zone.IsIsland(layer, nearest):
        radius = math.ceil(min_width * 0.67)
        nx, ny = -dy / length, dx / length
        corners = [
            vector(x + side * dx + (side * dx / length + across * nx) * radius,
                   y + side * dy + (side * dy / length + across * ny) * radius)
            for side, across in [(-1, -1), (1, -1), (1, 1), (-1, 1)]
        ]
        if all(zone.Outline().Contains(point) for point in corners):
            return {"patch_nm": [tuple(point) for point in corners], "clearance_nm": clearance}
        return None

    track = pcbnew.PCB_TRACK(board)
    track.SetLayer(layer)
    track.SetNetCode(zone.GetNetCode())
    track.SetWidth(min_width + pcbnew.FromMM(0.010))
    for offset in [0, 0.65, -0.65, 0.70, -0.70]:
        cx = x + min_width * offset * dx / length
        cy = y + min_width * offset * dy / length
        start, end = vector(cx + dy, cy - dx), vector(cx - dy, cy + dx)
        if not zone.Outline().Contains(start) or not zone.Outline().Contains(end):
            continue
        track.SetStart(start)
        track.SetEnd(end)
        shape = track.GetEffectiveShape(layer)
        if not any(shape.Collide(other, max(0, clearance - 1)) for other in obstacles):
            return {"start_nm": tuple(start), "end_nm": tuple(end), "width_nm": track.GetWidth()}
    return None


def main():
    pcb, request, output, version = sys.argv[1:]
    if pcbnew.GetBuildVersion().strip() != version:
        raise ValueError("KiCad CLI and PCB bindings have different versions")
    board = pcbnew.LoadBoard(pcb)
    if not board:
        raise ValueError("KiCad could not open the stored PCB")
    targets = json.loads(Path(request).read_text(encoding="utf-8"))
    zones = list(board.Zones())
    copper = list(board.GetTracks()) + list(board.GetPads())
    clearance = board.GetDesignSettings().GetSmallestClearanceValue()
    shapes = {}
    candidates = []
    for target in targets:
        layer = board.GetLayerID(target["layer"])
        matching = [zone for zone in zones if zone.m_Uuid.AsString() == target["zone_uuid"]]
        if (len(matching) != 1 or layer < 0 or not pcbnew.IsCopperLayer(layer)
                or not matching[0].IsOnLayer(layer) or matching[0].GetNetname() != target["net"]):
            candidates.append(None)
            continue
        zone = matching[0]
        key = (zone.GetNetCode(), layer)
        if key not in shapes:
            shapes[key] = [
                item.GetEffectiveShape(layer) for item in copper
                if item.IsOnLayer(layer) and item.GetNetCode() != zone.GetNetCode()
            ]
        candidates.append(candidate(board, target, zone, layer, shapes[key], clearance))
    Path(output).write_text(json.dumps(candidates), encoding="utf-8")


if __name__ == "__main__":
    main()
