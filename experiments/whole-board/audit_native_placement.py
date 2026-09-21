#!/usr/bin/env python3
# Copyright (C) 2026 Toit contributors.
"""Audit fixed-orientation translation into a native board before cold routing.

Checks the native pad identity, geometry, layers, raw net and relative position
inventory, footprint identity/value/orientation, requested translations, empty
copper, rectangular outline and unchanged project. It does not establish
datasheet correctness or unrepresented product constraints.
"""
from area_probe import load_pcbnew, rectangular_outline_bounds
from run import digest
from pathlib import Path
from decimal import Decimal, InvalidOperation


def custom_primitives(path):
    from audit_dsn_classes import parse_sexpr, child, children
    def canonical(value):
        if isinstance(value,list):return [canonical(v) for v in value]
        try:
            number=Decimal(value)
            assert number.is_finite()
            return format(number.normalize(),'f')
        except InvalidOperation:return value
    board=parse_sexpr(path.read_text());assert board[0]=='kicad_pcb'
    result={}
    for fp in children(board,'footprint'):
        for pad in children(fp,'pad'):
            if len(pad)<4 or pad[3]!='custom':continue
            uuid=child(pad,'uuid')[1]
            assert uuid not in result
            result[uuid]=canonical(child(pad,'primitives'))
    return result


def xy(p):
    return [p.x,p.y]


def inventory(board):
    pcbnew=load_pcbnew()
    custom=custom_primitives(Path(board.GetFileName()))
    result={}
    for fp in board.GetFootprints():
        anchor=fp.GetPosition()
        pads=[]
        for pad in fp.Pads():
            pos=pad.GetPosition()
            pads.append({'uuid':pad.m_Uuid.AsString(),'number':str(pad.GetNumber()),
                'net':str(pad.GetNetname()),'attribute':int(pad.GetAttribute()),
                'layers':[layer for layer in range(pcbnew.PCB_LAYER_ID_COUNT) if pad.IsOnLayer(layer)],
                'offset_from_anchor':[pos.x-anchor.x,pos.y-anchor.y],
                'orientation':pad.GetOrientationDegrees(),'shape':int(pad.GetShape()),
                'size':xy(pad.GetSize()),'offset':xy(pad.GetOffset()),
                'drill_shape':int(pad.GetDrillShape()),'drill_size':xy(pad.GetDrillSize()),
                'round_ratio':pad.GetRoundRectRadiusRatio(),'chamfer_ratio':pad.GetChamferRectRatio()})
            if pad.GetShape()==pcbnew.PAD_SHAPE_CUSTOM:
                pads[-1]['custom_primitives']=custom.pop(pad.m_Uuid.AsString())
                pads[-1]['custom_anchors']={str(layer):int(pad.GetAnchorPadShape(layer))
                    for layer in [pcbnew.F_Cu,pcbnew.B_Cu] if pad.IsOnLayer(layer)}
                pads[-1]['custom_zone_shape']=int(pad.GetCustomShapeInZoneOpt())
        ref=str(fp.GetReference());assert ref not in result
        result[ref]={'uuid':fp.m_Uuid.AsString(),'value':str(fp.GetValue()),
                     'orientation':fp.GetOrientationDegrees(),'layer':int(fp.GetLayer()),
                     'attributes':int(fp.GetAttributes()),
                     'pads':sorted(pads,key=lambda p:p['uuid'])}
    assert not custom, 'Custom pad inventory did not match native UUIDs'
    return result


def audit(source, placed, poses, mapping, source_bounds, ratio):
    pcbnew=load_pcbnew()
    before=pcbnew.LoadBoard(str(source));after=pcbnew.LoadBoard(str(placed))
    expected,actual=inventory(before),inventory(after)
    assert expected==actual, 'Native identity or pad geometry changed'
    assert before.GetCopperLayerCount()==after.GetCopperLayerCount()==2
    assert len(list(after.GetTracks()))==0 and len(list(after.Zones()))==0
    assert not any(s.GetLayer() in (pcbnew.F_Cu,pcbnew.B_Cu) for s in after.GetDrawings())
    assert not any(s.GetLayer() in (pcbnew.F_Cu,pcbnew.B_Cu) for fp in after.GetFootprints() for s in fp.GraphicalItems())
    assert digest(source.with_suffix('.kicad_pro'))==digest(placed.with_suffix('.kicad_pro'))
    from preserved_board_graphics import partition
    graphics = partition(before, mapping, poses)
    assert graphics == partition(after, mapping, poses), 'Preserved native graphic changed'
    by={p['component']:p for p in poses}
    for fp in after.GetFootprints():
        ref=str(fp.GetReference())
        if ref in graphics:
            continue
        p=by[ref];assert p['rotation_degrees']==0
        offset=mapping[ref]['anchor_offset']; pos=fp.GetPosition()
        wanted=[round((source_bounds[0]+p['position']['x']+offset['x'])*1e6),
                round((source_bounds[1]+p['position']['y']+offset['y'])*1e6)]
        assert xy(pos)==wanted, ref
    edges=[s for s in after.GetDrawings() if s.GetLayer()==pcbnew.Edge_Cuts]
    assert all(s.GetShape()==pcbnew.SHAPE_T_SEGMENT for s in edges)
    x0,y0,x1,y1=source_bounds
    left,top=round(x0*1e6),round(y0*1e6)
    right,bottom=round((x0+(x1-x0)*ratio**.5)*1e6),round((y0+(y1-y0)*ratio**.5)*1e6)
    assert rectangular_outline_bounds(after)==(left,top,right,bottom)
    return {'passed':True,'scope':__doc__,'footprints':len(actual),
             'pads':sum(len(f['pads']) for f in actual.values()),
             'source_sha256':digest(source),'placed_sha256':digest(placed),
             'area_mm2':(right-left)*(bottom-top)/1e12,
             'native_pad_inventory':actual}


if __name__ == '__main__':
    import argparse
    import json
    from pathlib import Path
    parser=argparse.ArgumentParser(description=__doc__)
    parser.add_argument('request',type=Path)
    parser.add_argument('output',type=Path)
    args=parser.parse_args()
    request=json.loads(args.request.read_text())
    result=audit(Path(request['source']),Path(request['placed']),request['poses'],
                 request['mapping'],request['source_bounds'],request['ratio'])
    args.output.write_text(json.dumps(result,indent=2)+'\n')
