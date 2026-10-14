# Copyright (C) 2026 Toit contributors.
"""Draws KiCad's fill and the router's pour map around a pad, as SVG.

    python3 -I render.py <board.kicad_pcb> <dump-directory> <pad> <layer> <out.svg>
        [--radius MM] [--filled FILLED.kicad_pcb] [--net NET]

`<pad>` is `REF.NUMBER` (the first such pad; `REF.NUMBER@X,Y` picks the one
nearest X,Y). The board's zones are filled as kicad-cli fills them unless
`--filled` names a board whose zones are filled already (spokes.py
--save-filled). Drawn: KiCad's fill of the pad's net on the layer (green;
outlines whose only item is the pad in orange), the pad's DRC ring (half a
gap around it, dashed), other nets' copper on the layer (grey), the net's
own (blue), and the router's free nodes (dots, one colour per piece; red
crosses for nodes of the net's routed copper). Convert with rsvg-convert.
"""

import argparse
import colorsys
import json
import math
import os
import sys

import numpy as np
import pcbnew

sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
import spokes  # noqa: E402

NM = 1.0e6


def load_dump(path):
    with open(path, "rb") as handle:
        header = json.loads(handle.readline())
        nx, ny = header["nx"], header["ny"]
        flags = np.frombuffer(handle.read(nx * ny), dtype=np.uint8).reshape(ny, nx)
        labels = np.frombuffer(handle.read(4 * nx * ny), dtype="<u4").reshape(ny, nx)
    return header, flags, labels


def chain_points(chain):
    return [(chain.CPoint(i).x / NM, chain.CPoint(i).y / NM) for i in range(chain.PointCount())]


def path_of(rings):
    return " ".join("M " + " L ".join(f"{x:.4f},{y:.4f}" for x, y in ring) + " Z" for ring in rings if ring)


def colour(label):
    hue = (label * 0.6180339887) % 1.0
    r, g, b = colorsys.hsv_to_rgb(hue, 0.85, 0.85)
    return f"#{int(r * 255):02x}{int(g * 255):02x}{int(b * 255):02x}"


def shape_rings(item, layer):
    polys = pcbnew.SHAPE_POLY_SET()
    item.TransformShapeToPolygon(polys, layer, 0, 5000, pcbnew.ERROR_INSIDE)
    rings = []
    for index in range(polys.OutlineCount()):
        rings.append(chain_points(polys.Outline(index)))
        for hole in range(polys.HoleCount(index)):
            rings.append(chain_points(polys.Hole(index, hole)))
    return rings


def main():
    parser = argparse.ArgumentParser()
    parser.add_argument("board")
    parser.add_argument("dump")
    parser.add_argument("pad")
    parser.add_argument("layer")
    parser.add_argument("out")
    parser.add_argument("--radius", type=float, default=2.0)
    parser.add_argument("--filled")
    parser.add_argument("--net")
    args = parser.parse_args()
    if args.filled:
        board = pcbnew.LoadBoard(args.filled)
        zones = [zone for zone in board.Zones() if not zone.GetIsRuleArea()]
    else:
        board = pcbnew.LoadBoard(args.board)
        zones = spokes.fill(board)
    layer = board.GetLayerID(args.layer)
    label, _, near = args.pad.partition("@")
    reference, number = label.rsplit(".", 1)
    candidates = [pad for footprint in board.GetFootprints() if footprint.GetReference() == reference for pad in footprint.Pads() if pad.GetNumber() == number]
    if not candidates:
        sys.exit(f"no pad {label}")
    if near:
        x, y = (float(value) for value in near.split(","))
        candidates.sort(key=lambda pad: (pad.GetPosition().x / NM - x) ** 2 + (pad.GetPosition().y / NM - y) ** 2)
    pad = candidates[0]
    net = args.net or str(pad.GetNetname())
    cx, cy = pad.GetPosition().x / NM, pad.GetPosition().y / NM
    radius = args.radius
    box = pcbnew.BOX2I(pcbnew.VECTOR2I(int((cx - radius) * NM), int((cy - radius) * NM)), pcbnew.VECTOR2L(int(2 * radius * NM), int(2 * radius * NM)))
    parts = []
    scale = 400.0 / radius
    parts.append(f'<svg xmlns="http://www.w3.org/2000/svg" width="800" height="800" viewBox="{cx - radius} {cy - radius} {2 * radius} {2 * radius}">')
    parts.append(f'<rect x="{cx - radius}" y="{cy - radius}" width="{2 * radius}" height="{2 * radius}" fill="white"/>')
    # KiCad's fill of the net on the layer.
    for zone in zones:
        if str(zone.GetNetname()) != net or not zone.IsOnLayer(layer):
            continue
        polys = zone.GetFilledPolysList(layer)
        for index in range(polys.OutlineCount()):
            chain = polys.Outline(index)
            if not chain.BBox().Intersects(box):
                continue
            rings = [chain_points(chain)] + [chain_points(polys.Hole(index, hole)) for hole in range(polys.HoleCount(index))]
            parts.append(f'<path d="{path_of(rings)}" fill="#9be59b" fill-opacity="0.8" fill-rule="evenodd" stroke="#2a8a2a" stroke-width="{1.0 / scale}"/>')
    # Copper on the layer.
    items = []
    for footprint in board.GetFootprints():
        for other in footprint.Pads():
            if other.IsOnLayer(layer) and other.GetBoundingBox().Intersects(box):
                items.append(other)
    for track in board.GetTracks():
        if track.IsOnLayer(layer) and track.GetBoundingBox().Intersects(box):
            items.append(track)
    for item in items:
        own = str(item.GetNetname()) == net
        fill = "#3060d0" if own else "#909090"
        parts.append(f'<path d="{path_of(shape_rings(item, layer))}" fill="{fill}" fill-opacity="0.45" stroke="{fill}" stroke-width="{1.0 / scale}"/>')
        if isinstance(item, pcbnew.PAD):
            p = item.GetPosition()
            parts.append(f'<text x="{p.x / NM}" y="{p.y / NM}" font-size="{10 / scale}" text-anchor="middle" fill="black">{item.GetParentFootprint().GetReference()}.{item.GetNumber()}</text>')
    # The pad's DRC ring.
    zone = next((zone for zone in zones if str(zone.GetNetname()) == net and zone.IsOnLayer(layer)), None)
    if zone is not None:
        gap = pad.GetLocalThermalGapOverride()
        gap = gap if isinstance(gap, int) and gap > 0 else zone.GetThermalReliefGap()
        ring = pcbnew.SHAPE_POLY_SET()
        pad.TransformShapeToPolygon(ring, layer, gap // 2, 5000, pcbnew.ERROR_OUTSIDE)
        parts.append(f'<path d="{path_of([chain_points(ring.Outline(0))])}" fill="none" stroke="black" stroke-dasharray="{3 / scale},{3 / scale}" stroke-width="{1.0 / scale}"/>')
    # The router's map.
    names = json.load(open(os.path.join(args.dump, "layers.json")))
    layer_index = names.index(args.layer)
    file_name = "".join(c if c.isascii() and (c.isalnum() or c in "+-_") else "_" for c in net)
    path = os.path.join(args.dump, f"{file_name}-L{layer_index}.pour")
    if os.path.exists(path):
        header, flags, labels = load_dump(path)
        ox, oy, pitch = header["origin"][0], header["origin"][1], header["pitch"]
        x0, x1 = max(0, int((cx - radius - ox) / pitch)), min(header["nx"] - 1, int((cx + radius - ox) / pitch) + 1)
        y0, y1 = max(0, int((cy - radius - oy) / pitch)), min(header["ny"] - 1, int((cy + radius - oy) / pitch) + 1)
        dot = pitch * 0.18
        for y in range(y0, y1 + 1):
            for x in range(x0, x1 + 1):
                px, py = ox + x * pitch, oy + y * pitch
                flag = flags[y, x]
                if flag & 16:
                    parts.append(f'<circle cx="{px:.4f}" cy="{py:.4f}" r="{dot:.4f}" fill="{colour(int(labels[y, x]))}"/>')
                elif flag & 1:
                    parts.append(f'<circle cx="{px:.4f}" cy="{py:.4f}" r="{dot * 0.4:.4f}" fill="#bbbbbb"/>')
                if flag & 32:
                    d = dot
                    parts.append(f'<path d="M {px - d},{py - d} L {px + d},{py + d} M {px - d},{py + d} L {px + d},{py - d}" stroke="red" stroke-width="{1.0 / scale}"/>')
        # Piece labels near the pad.
        seen = {}
        for y in range(y0, y1 + 1):
            for x in range(x0, x1 + 1):
                if flags[y, x] & 16 and int(labels[y, x]) not in seen:
                    seen[int(labels[y, x])] = (ox + x * pitch, oy + y * pitch)
        for piece, (px, py) in seen.items():
            parts.append(f'<text x="{px:.4f}" y="{py:.4f}" font-size="{9 / scale}" fill="{colour(piece)}">{piece}</text>')
    parts.append("</svg>")
    open(args.out, "w").write("\n".join(parts))


if __name__ == "__main__":
    main()
