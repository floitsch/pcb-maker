# Copyright (C) 2026 Toit contributors.
"""Native-derived courtyard, side and disconnected-seed controls with renderings."""
import argparse
import copy
import json
from pathlib import Path
import shutil

from area_probe import load_pcbnew, render
from courtyard_geometry import extract_courtyards
from placement_polygons import ordered_boundary, segment_distance, edges
from run import command, digest, read, write
from spectral_placement_seed import seed


def inside(point, polygon):
    x,y=point;value=False
    for a,b in edges(polygon):
        if (a[1]>y)!=(b[1]>y) and x<(b[0]-a[0])*(y-a[1])/(b[1]-a[1])+a[0]:
            value=not value
    return value


def controls(source, problem_path, connected_reference, binary, output):
    output.mkdir(exist_ok=False)
    shutil.copy2(__file__,output/Path(__file__).name)
    k=load_pcbnew();board=k.LoadBoard(str(source));bodies=extract_courtyards(board)
    sampling=[]
    for fp in board.GetFootprints():
        ref=str(fp.GetReference());body=bodies[ref]
        if not body.get('body_parts'):continue
        layer=k.F_CrtYd if fp.GetLayer()==k.F_Cu else k.B_CrtYd
        shapes=[s for s in fp.GraphicalItems() if isinstance(s,k.PCB_SHAPE) and s.GetLayer()==layer]
        polygon=ordered_boundary([(tuple(s.GetStart()),tuple(s.GetEnd())) for s in shapes])
        cx,cy=body['center'];w,h=body['size']
        parts=[[(cx+v['x'],cy+v['y']) for v in p['vertices']] for p in body['body_parts']]
        checked=outside=0
        for ix in range(31):
            for iy in range(31):
                point=(cx-w/2+(ix+.37)*w/31,cy-h/2+(iy+.61)*h/31)
                if min(segment_distance(point,a,b) for a,b in edges(polygon))<1e-6:continue
                expected=inside(point,polygon)
                assert any(inside(point,p) for p in parts)==expected, (ref,point)
                checked+=1;outside+=not expected
        assert outside>0, 'This real control must exercise excluded hull space'
        sampling.append(dict(component=ref,checked_points=checked,points_in_excluded_hull_space=outside))
    assert {s['component'] for s in sampling}=={'P3','RV1','U3'}
    assert bodies['JP1']['layers']==['bottom']
    original=read(problem_path)
    parent=next(c for c in original['components'] if c['id']=='P3')
    rows=[]
    for label,position,layer,envelope,accepted in [
        ('notch',[168,60],'top',False,True),
        ('filled-envelope',[168,60],'top',True,False),
        ('actual-material',[180,60],'top',False,False),
        ('opposite-side',[180,60],'bottom',False,True)]:
        case=output/label;case.mkdir()
        problem=copy.deepcopy(original)
        problem['board']['bounds']={'min':{'x':0,'y':0},'max':{'x':220,'y':130}}
        a=copy.deepcopy(parent);a['position']=dict(zip(['x','y'],bodies['P3']['center']))
        a['constraints']['movement']='fixed'
        if envelope:a['placement_geometry'].pop('body_parts')
        probe=dict(id='probe',position=dict(zip(['x','y'],position)),size={'x':.8,'y':.8},pins=[],
            constraints={'movement':'fixed','rotation':'fixed'},body_is_routing_keepout=False,
            placement_geometry={'body':{'kind':'circle','diameter':.8},'clearance':0,
                                'body_layers':[layer]})
        problem['components']=[a,probe];problem['electrical_nets']=[]
        poses=[dict(component=c['id'],position=c['position'],rotation_degrees=0) for c in problem['components']]
        independent=render(problem,poses,case/'geometry.svg',label+'; native-derived P3 courtyard')
        assert independent['passed']==accepted
        write(case/'problem.json',problem);write(case/'config.json',{'policy':{'kind':'declared'}})
        process=command([binary,'place',case/'problem.json',case/'config.json',case/'placement.json'],case/'place.log')
        assert (process['exit_code']==0)==accepted, label
        rows.append(dict(case=label,accepted=accepted,independent=independent,process=process))
    candidate,evidence=seed(original)
    assert sorted(len(b['components']) for b in evidence['attraction_blocks'])==[1,1,1,1,1,1,57]
    shuffled=copy.deepcopy(original)
    shuffled['components'].reverse();shuffled['electrical_nets'].reverse()
    for c in shuffled['components']:c['position']={'x':123456,'y':-99999}
    for n in shuffled['electrical_nets']:n['terminals'].reverse()
    alternate,_=seed(shuffled)
    positions=lambda p:{c['id']:c['position'] for c in p['components']}
    assert positions(candidate)==positions(alternate)
    connected=read(connected_reference);assert seed(connected)[0]==connected
    poses=[dict(component=c['id'],position=c['position'],rotation_degrees=0) for c in candidate['components']]
    render(candidate,poses,output/'disconnected-seed.svg','PIC disconnected seed; not yet legalized')
    write(output/'report.json',dict(status='passed',cases=rows,real_polygon_membership_controls=sampling,
        disconnected_blocks=7,unused_source_centers=True,input_order_invariant=True,
        connected_seed_exact_replay=True,source_sha256=digest(source),binary_sha256=digest(binary)))
    page=['<!doctype html><meta charset="utf-8"><title>Courtyard geometry controls</title>',
          '<style>body{font:17px system-ui;margin:2rem}img{max-width:100%}</style>',
          '<h1>Concave courtyards retain usable space</h1>',
          '<p>Native-derived P3 geometry: the notch is usable, actual material blocks same-side placement, '
          'and opposite-side assembly remains possible. These are placement-geometry controls, not routed boards.</p>']
    for row in rows:
        page.append(f'<h2>{row["case"]}: {"accepted" if row["accepted"] else "rejected"}</h2>'
                    f'<img src="{row["case"]}/geometry.svg">')
    page.append('<h2>All 63 components receive seed positions</h2><img src="disconnected-seed.svg">'
                '<p><a href="report.json">Assertions and native-source membership checks</a></p>')
    (output/'index.html').write_text(''.join(page))


if __name__=='__main__':
    parser=argparse.ArgumentParser(description=__doc__)
    for name in ['source','problem','connected_reference','binary','output']:parser.add_argument(name,type=Path)
    args=parser.parse_args()
    controls(args.source.resolve(),args.problem.resolve(),args.connected_reference.resolve(),args.binary.resolve(),args.output.resolve())
