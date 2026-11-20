# Copyright (C) 2026 Florian Loitsch.

"""Find possible width chords on saved net copper using KiCad's geometry."""

import bisect
import collections
import json
import math
import sys
from pathlib import Path

import pcbnew


# KiCad reports these widths to four decimal places in mm.
ROUNDING_NM = 50
MAX_POINTS = 500_000
MAX_PAIRS_TESTED = 12_000_000
MAX_CANDIDATES = 2_048
PROBE_WIDTH_NM = 1_000


def outline_chords(polygons, widths):
    radius = widths[-1] + ROUNDING_NM
    maximum_square = radius**2
    minimum_square = max(0, widths[0] - ROUNDING_NM)**2
    tested = 0
    point_count = 0
    for index in range(polygons.OutlineCount()):
        outline = polygons.COutline(index)
        count = outline.PointCount()
        point_count += count
        if point_count > MAX_POINTS:
            raise ValueError("saved copper exceeds the width geometry point limit")
        buckets = collections.defaultdict(list)
        for i in range(count):
            x, y = outline.CPoint(i)
            col, row = x // radius, y // radius
            for u in range(col - 1, col + 2):
                for v in range(row - 1, row + 2):
                    neighbors = buckets.get((u, v), ())
                    tested += len(neighbors)
                    if tested > MAX_PAIRS_TESTED:
                        raise ValueError("width geometry exceeds the local comparison limit")
                    for j, a, b in neighbors:
                        if i - j == 1 or i - j == count - 1:
                            continue
                        square = (x - a)**2 + (y - b)**2
                        if not minimum_square <= square <= maximum_square:
                            continue
                        width = math.sqrt(square)
                        match_index = bisect.bisect_left(widths, width - ROUNDING_NM)
                        if match_index < len(widths) and widths[match_index] <= width + ROUNDING_NM:
                            yield widths[match_index], tuple(sorted(((x, y), (a, b))))
            buckets[col, row].append((i, x, y))


def width_probes(board, targets):
    zones = collections.defaultdict(list)
    for zone in board.Zones():
        zones[zone.m_Uuid.AsString()].append(zone)
    groups = collections.defaultdict(lambda: collections.defaultdict(set))
    for target in targets:
        matches = zones[target["zone_uuid"]]
        if len(matches) != 1:
            continue
        zone = matches[0]
        layer = board.GetLayerID(target["layer"])
        if (layer < 0 or not pcbnew.IsCopperLayer(layer) or not zone.IsOnLayer(layer)
                or zone.GetIsRuleArea() or not zone.GetNetname()
                or zone.GetFilledPolysList(layer).IsEmpty()):
            continue
        groups[(zone.GetNetname(), layer, target["layer"])][target["actual_nm"]].add(
            target["zone_uuid"]
        )
    copper = list(board.GetTracks()) + list(board.GetPads()) + list(board.Zones())
    probes = []
    for (net, layer, layer_name), targets_by_width in sorted(groups.items()):
        polygons = pcbnew.SHAPE_POLY_SET()
        for item in copper:
            if item.GetNetname() == net and item.IsOnLayer(layer):
                item.TransformShapeToPolygon(
                    polygons, layer, 0, pcbnew.FromMM(pcbnew.ARC_HIGH_DEF_MM), pcbnew.ERROR_OUTSIDE
                )
        polygons.Fracture()
        centers = collections.defaultdict(list)
        for width, chord in outline_chords(polygons, sorted(targets_by_width)):
            (x0, y0), (x1, y1) = chord
            x, y = (x0 + x1) // 2, (y0 + y1) // 2
            col, row = x // PROBE_WIDTH_NM, y // PROBE_WIDTH_NM
            close = any(
                (x - a)**2 + (y - b)**2 <= PROBE_WIDTH_NM**2
                for u in range(col - 1, col + 2)
                for v in range(row - 1, row + 2)
                for a, b in centers.get((width, u, v), ())
            )
            # Two near-identical chords can both name the same connection
            # width, displacing the original zone witness in the DRC.
            if not close:
                centers[width, col, row].append((x, y))
                probes.append({
                    "net": net,
                    "layer": layer_name,
                    "actual_nm": width,
                    "zone_uuids": sorted(targets_by_width[width]),
                    "midpoint_nm": (x, y),
                    "chord_nm": chord,
                })
                if len(probes) > MAX_CANDIDATES:
                    raise ValueError("width geometry exceeds the scratch witness limit")
    return probes


def main():
    pcb, request, output, version = sys.argv[1:]
    if pcbnew.GetBuildVersion().strip() != version:
        raise ValueError("KiCad CLI and PCB bindings have different versions")
    board = pcbnew.LoadBoard(pcb)
    if not board:
        raise ValueError("KiCad could not open the stored PCB")
    targets = json.loads(Path(request).read_text(encoding="utf-8"))
    Path(output).write_text(json.dumps(width_probes(board, targets)), encoding="utf-8")


if __name__ == "__main__":
    main()
