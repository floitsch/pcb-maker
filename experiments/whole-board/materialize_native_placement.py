# Copyright (C) 2026 Toit contributors.
"""Apply validated translations in a fresh native process, preserving footprints."""
import argparse
import json
import math
from pathlib import Path
from area_probe import load_pcbnew


def apply(request):
    pcbnew=load_pcbnew()
    board_path=Path(request['board'])
    mapping=request['mapping'];bounds=request['bounds'];ratio=request['ratio']
    w=(bounds[2]-bounds[0])*math.sqrt(ratio)
    h=(bounds[3]-bounds[1])*math.sqrt(ratio)
    moved = pcbnew.LoadBoard(str(board_path))
    from preserved_board_graphics import partition
    graphics = partition(moved, mapping, request['poses'])
    by_ref = {p['component']:p for p in request['poses']}
    for fp in moved.GetFootprints():
        if str(fp.GetReference()) in graphics:
            continue
        pose = by_ref[fp.GetReference()]['position']
        offset = mapping[fp.GetReference()]['anchor_offset']
        fp.SetPosition(pcbnew.VECTOR2I(round((bounds[0]+pose['x']+offset['x'])*1e6),
                                      round((bounds[1]+pose['y']+offset['y'])*1e6)))
    if request['outline'] == 'free_rectangle':
        for edge in list(moved.GetDrawings()):
            if edge.GetLayer() == pcbnew.Edge_Cuts:
                moved.Remove(edge)
        corners=[(bounds[0],bounds[1]),(bounds[0]+w,bounds[1]),
                 (bounds[0]+w,bounds[1]+h),(bounds[0],bounds[1]+h)]
        for start,finish in zip(corners,corners[1:]+corners[:1]):
            edge=pcbnew.PCB_SHAPE(moved)
            edge.SetShape(pcbnew.SHAPE_T_SEGMENT)
            edge.SetLayer(pcbnew.Edge_Cuts)
            edge.SetWidth(50000)
            edge.SetStart(pcbnew.VECTOR2I(round(start[0]*1e6),round(start[1]*1e6)))
            edge.SetEnd(pcbnew.VECTOR2I(round(finish[0]*1e6),round(finish[1]*1e6)))
            moved.Add(edge)
    else:
        for edge in moved.GetDrawings():
            if edge.GetLayer() != pcbnew.Edge_Cuts:
                continue
            for get,set_ in ((edge.GetStart,edge.SetStart),(edge.GetEnd,edge.SetEnd)):
                p = get()
                set_(pcbnew.VECTOR2I(round((bounds[0]+(p.x/1e6-bounds[0])*math.sqrt(ratio))*1e6),
                                     round((bounds[1]+(p.y/1e6-bounds[1])*math.sqrt(ratio))*1e6)))
    pcbnew.SaveBoard(str(board_path),moved)


if __name__=='__main__':
    parser=argparse.ArgumentParser(description=__doc__)
    parser.add_argument('request',type=Path)
    args=parser.parse_args()
    apply(json.loads(args.request.read_text()))
