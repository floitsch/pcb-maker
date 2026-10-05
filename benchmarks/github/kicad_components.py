#!/usr/bin/env python3
# Copyright (C) 2026 Toit contributors.
"""Refills a board with KiCad and splits one net's copper (fill islands,
pads, vias, tracks) into connected components, as KiCad's connectivity
sees them; prints the components after the largest.

    components.py <board.kicad_pcb> <net>
"""
import sys
import pcbnew

board = pcbnew.LoadBoard(sys.argv[1])
net = sys.argv[2]
zones = list(board.Zones())
pcbnew.ZONE_FILLER(board).Fill(zones)
tracks = board.Tracks()
items = [tracks[index] for index in range(len(tracks)) if tracks[index].GetNetname() == net]
pads = [pad for footprint in board.GetFootprints() for pad in footprint.Pads() if pad.GetNetname() == net]

parent = {}
def find(x):
    parent.setdefault(x, x)
    while parent[x] != x:
        parent[x] = parent[parent[x]]
        x = parent[x]
    return x
def join(a, b):
    parent[find(a)] = find(b)

islands = []  # (key, layer, fill, index)
for zone in zones:
    if zone.GetNetname() != net or zone.GetIsRuleArea():
        continue
    for layer in zone.GetLayerSet().Seq():
        fill = zone.GetFilledPolysList(layer)
        for index in range(fill.OutlineCount()):
            key = ('island', board.GetLayerName(layer), index, id(zone))
            find(key)
            islands.append((key, layer, fill, index))

def touching_islands(point, layer_ok, tolerance):
    for key, layer, fill, index in islands:
        if layer_ok(layer) and fill.Contains(point, index, tolerance):
            yield key

for number, pad in enumerate(pads):
    key = ('pad', pad.GetParentFootprint().GetReference(), pad.GetNumber(), number)
    find(key)
    for island in touching_islands(pad.GetPosition(), pad.IsOnLayer, 0):
        join(key, island)
    pad_key = key
    pads[number] = (pad, pad_key)

for number, item in enumerate(items):
    key = ('track', number)
    find(key)
    if item.Type() == pcbnew.PCB_VIA_T:
        for island in touching_islands(item.GetPosition(), lambda layer: True, 0):
            join(key, island)
        points = [item.GetPosition()]
    else:
        for point in (item.GetStart(), item.GetEnd()):
            for island in touching_islands(point, lambda layer, item=item: layer == item.GetLayer(), 0):
                join(key, island)
        points = [item.GetStart(), item.GetEnd()]
    for pad, pad_key in pads:
        for point in points:
            if pad.HitTest(point):
                join(key, pad_key)
    for other_number, other in enumerate(items[:number]):
        other_points = [other.GetPosition()] if other.Type() == pcbnew.PCB_VIA_T else [other.GetStart(), other.GetEnd()]
        if any((p - q).EuclideanNorm() < 1000 for p in points for q in other_points):
            join(key, ('track', other_number))

components = {}
for key in list(parent):
    components.setdefault(find(key), []).append(key)
ordered = sorted(components.values(), key=len, reverse=True)
with_pads = [c for c in ordered if any(k[0] == 'pad' for k in c)]
print(f"{net}: {len(ordered)} components, {len(with_pads)} with pads")
for component in with_pads[1:12]:
    pads_in = [f"{k[1]}.{k[2]}" for k in component if k[0] == 'pad']
    isl = [f"{k[1]}#{k[2]}" for k in component if k[0] == 'island']
    print(f"  pads {pads_in[:6]} islands {isl[:6]} tracks {sum(1 for k in component if k[0] == 'track')}")

