# Copyright (C) 2026 Toit contributors.
"""Pads whose copper lies inside another pad of the same net on a layer.

    python3 -I pads_in_pads.py <board.kicad_pcb>... [--json out.json]

Per board, per copper layer that holds a zone of the net: each pad whose
copper on the layer lies entirely inside a larger pad of its net there (an
exposed pad's thermal vias), with the host, whether they share a number,
whether the zone is hatched, and the zone's connection for the pad. These
are the pads whose spokes KiCad builds and counts inside the host's copper
(zone_filler.cpp; DRC_TEST_PROVIDER_ZONE_CONNECTIONS).
"""

import json
import os
import sys

import pcbnew

NM = 1.0e6


def pad_label(pad):
    return f"{pad.GetParentFootprint().GetReference()}.{pad.GetNumber()}"


def inside_pairs(board):
    zones = [zone for zone in board.Zones() if not zone.GetIsRuleArea() and zone.GetNetCode() > 0]
    by_net = {}
    for footprint in board.GetFootprints():
        for pad in footprint.Pads():
            if pad.GetNetCode() > 0:
                by_net.setdefault(pad.GetNetCode(), []).append(pad)
    rows = []
    for zone in zones:
        for layer in zone.GetLayerSet().Seq():
            if not pcbnew.IsCopperLayer(layer):
                continue
            pads = [pad for pad in by_net.get(zone.GetNetCode(), []) if pad.FlashLayer(layer)]
            shapes = {}
            for pad in pads:
                polys = pcbnew.SHAPE_POLY_SET()
                pad.TransformShapeToPolygon(polys, layer, 0, 5000, pcbnew.ERROR_INSIDE)
                shapes[id(pad)] = polys
            for small in pads:
                box = small.GetBoundingBox()
                for large in pads:
                    if large is small or not large.GetBoundingBox().Contains(box):
                        continue
                    area = lambda pad: pad.GetBoundingBox().GetWidth() * pad.GetBoundingBox().GetHeight()
                    if area(large) <= area(small):
                        continue
                    rest = pcbnew.SHAPE_POLY_SET(shapes[id(small)])
                    rest.BooleanSubtract(shapes[id(large)])
                    if rest.OutlineCount() and rest.Area() > 1.0e-6 * NM * NM:
                        continue
                    rows.append({
                        "pad": pad_label(small),
                        "at": [small.GetPosition().x / NM, small.GetPosition().y / NM],
                        "host": pad_label(large),
                        "host_at": [large.GetPosition().x / NM, large.GetPosition().y / NM],
                        "same_number": small.GetNumber() == large.GetNumber(),
                        "layer": str(board.GetStandardLayerName(layer)),
                        "net": str(small.GetNetname()),
                        "hatched": zone.GetFillMode() == pcbnew.ZONE_FILL_MODE_HATCH_PATTERN,
                    })
                    break
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
        rows = inside_pairs(pcbnew.LoadBoard(path))
        result[path] = rows
        hosts = sorted({(row["host"], row["layer"], row["hatched"]) for row in rows})
        print(f"{os.path.basename(path)}: {len(rows)} pads inside pads; hosts {hosts}")
    if out:
        json.dump(result, open(out, "w"), indent=1)


if __name__ == "__main__":
    main()
