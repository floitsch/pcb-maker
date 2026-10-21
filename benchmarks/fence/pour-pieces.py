# Copyright (C) 2026 Toit contributors.
# Counts the pieces of every filled zone: pour-pieces.py <board.kicad_pcb> <board height mm>
import sys, pcbnew
path, h = sys.argv[1], float(sys.argv[2])
b = pcbnew.LoadBoard(path)
pcbnew.ZONE_FILLER(b).Fill([z for z in b.Zones() if not z.GetIsRuleArea()])
for z in b.Zones():
    if z.GetIsRuleArea(): continue
    for layer in z.GetLayerSet().Seq():
        fill = z.GetFilledPolysList(layer)
        areas = sorted((fill.Outline(i).Area()/1e12 for i in range(fill.OutlineCount())), reverse=True)
        print(z.GetNetname(), b.GetLayerName(layer), 'pieces', len(areas), 'largest', [round(a) for a in areas[:4]], 'total', round(sum(areas)), 'of', round(76.2*h))
