# Copyright (C) 2026 Toit contributors.
"""Overlapping zones of one net with different priorities, per board.

    python3 -I seams.py <board.kicad_pcb>... [--json out.json]

KiCad 10 fills each zone on its own (zone_filler.cpp): a zone's fill is cut
to its own outline, less the outlines of the net's zones of a higher
priority on the layer (subtractHigherPriorityZones), so where they overlap
the higher one fills and the lower stops at its outline, the seam. The DRC
(DRC_TEST_PROVIDER_ZONE_CONNECTIONS) counts a pad's spokes per zone: a pad
whose spokes cross the seam is checked against each zone with the spokes
in that zone's part alone. Per board: each pair of such zones (layer,
priorities, overlap in mm²) and the thermal pads of the net within a
millimetre of the seam (the higher zone's outline inside the lower one).
"""

import json
import os
import sys

import pcbnew

NM = 1.0e6


def pad_label(pad):
    return f"{pad.GetParentFootprint().GetReference()}.{pad.GetNumber()}"


def outline(zone):
    polys = zone.Outline().CloneDropTriangulation()
    polys.ClearArcs()
    return polys


def seams(board):
    zones = [zone for zone in board.Zones() if not zone.GetIsRuleArea() and zone.GetNetCode() > 0 and not zone.IsTeardropArea()]
    pads_by_net = {}
    for footprint in board.GetFootprints():
        for pad in footprint.Pads():
            if pad.GetNetCode() > 0:
                pads_by_net.setdefault(pad.GetNetCode(), []).append(pad)
    rows = []
    for low in zones:
        for high in zones:
            if high is low or high.GetNetCode() != low.GetNetCode() or high.GetAssignedPriority() <= low.GetAssignedPriority():
                continue
            for layer in low.GetLayerSet().Seq():
                if not pcbnew.IsCopperLayer(layer) or not high.IsOnLayer(layer):
                    continue
                overlap = outline(low)
                overlap.BooleanIntersection(outline(high))
                if overlap.OutlineCount() == 0 or overlap.Area() < 1.0e-6 * NM * NM:
                    continue
                # The seam: the higher zone's outline where it runs inside
                # the lower one, as a thin band.
                edge = outline(high)
                band = pcbnew.SHAPE_POLY_SET(edge)
                band.Inflate(int(1.0 * NM), pcbnew.CORNER_STRATEGY_ROUND_ALL_CORNERS, 5000) if hasattr(pcbnew, "CORNER_STRATEGY_ROUND_ALL_CORNERS") else band.Inflate(int(1.0 * NM), 32)
                inner = pcbnew.SHAPE_POLY_SET(edge)
                inner.Deflate(int(1.0 * NM), pcbnew.CORNER_STRATEGY_ROUND_ALL_CORNERS, 5000) if hasattr(pcbnew, "CORNER_STRATEGY_ROUND_ALL_CORNERS") else inner.Deflate(int(1.0 * NM), 32)
                band.BooleanSubtract(inner)
                band.BooleanIntersection(outline(low))
                near = []
                for pad in pads_by_net.get(low.GetNetCode(), []):
                    if not pad.FlashLayer(layer):
                        continue
                    if band.Collide(pcbnew.VECTOR2I(pad.GetPosition())):
                        near.append(pad_label(pad))
                rows.append({
                    "net": str(low.GetNetname()),
                    "layer": str(board.GetStandardLayerName(layer)),
                    "low": low.GetAssignedPriority(),
                    "high": high.GetAssignedPriority(),
                    "overlap_mm2": round(overlap.Area() / NM / NM, 2),
                    "low_hatched": low.GetFillMode() == pcbnew.ZONE_FILL_MODE_HATCH_PATTERN,
                    "pads_near_seam": sorted(near),
                })
    return rows


def main():
    arguments = sys.argv[1:]
    out = None
    if "--json" in arguments:
        index = arguments.index("--json")
        out = arguments[index + 1]
        del arguments[index:index + 2]
    result = {}
    for path in arguments:
        board = pcbnew.LoadBoard(path)
        if board is None:
            print(f"{path}: does not load", file=sys.stderr)
            continue
        rows = seams(board)
        result[path] = rows
        if rows:
            print(f"{path}: {len(rows)} same-net zone overlaps of different priority")
            for row in rows:
                print(f"  {row['net']} {row['layer']} priority {row['low']} under {row['high']}: {row['overlap_mm2']} mm², {len(row['pads_near_seam'])} pads near the seam {row['pads_near_seam'][:12]}")
    if out:
        json.dump(result, open(out, "w"), indent=1)


if __name__ == "__main__":
    main()
