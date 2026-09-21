#!/usr/bin/env python3
# Copyright (C) 2026 Toit contributors.
"""Native courtyard evidence for placement (circles and simple straight polygons).

Courtyards are mechanical exclusion regions, not copper clearance settings.
Unsupported or missing geometry is rejected instead of replaced by a display
bounding box. This is not a component datasheet or product constraint audit.
"""
import argparse
import html
import itertools
import math
from pathlib import Path

from area_probe import load_pcbnew
from run import digest, write
from placement_polygons import body_polygons, circle_gap, convex_gap, ordered_boundary, triangulate, validate


def extract_courtyards(board):
    pcbnew = load_pcbnew()
    result = {}
    from preserved_board_graphics import preserved_graphics
    graphics = preserved_graphics(board.GetFileName())
    for fp in board.GetFootprints():
        if str(fp.GetReference()) in graphics:
            continue
        assert fp.GetLayer() in (pcbnew.F_Cu,pcbnew.B_Cu), 'Footprint must be on an outer copper layer'
        front=fp.GetLayer()==pcbnew.F_Cu
        layer=pcbnew.F_CrtYd if front else pcbnew.B_CrtYd
        shapes = [s for s in fp.GraphicalItems()
                  if isinstance(s, pcbnew.PCB_SHAPE) and s.GetLayer() == layer]
        ref = str(fp.GetReference())
        parts = []
        evidence = f'native {"F" if front else "B"}.CrtYd centerline; stroke width excluded'
        assert shapes, f'{ref}: missing native courtyard on its component side'
        if len(shapes) == 1 and shapes[0].GetShape() == pcbnew.SHAPE_T_CIRCLE:
            s = shapes[0]
            center = [s.GetStart().x/1e6, s.GetStart().y/1e6]
            radius = math.hypot(s.GetEnd().x/1e6-center[0], s.GetEnd().y/1e6-center[1])
            shape = {'kind': 'circle', 'diameter': 2*radius}
            size = [2*radius, 2*radius]
        else:
            if len(shapes)==1 and shapes[0].GetShape()==pcbnew.SHAPE_T_RECT:
                polygon=[(p.x/1e6,p.y/1e6) for p in shapes[0].GetRectCorners()]
                validate(polygon)
            else:
                assert all(s.GetShape()==pcbnew.SHAPE_T_SEGMENT for s in shapes), f'{ref}: unsupported curved or compound courtyard'
                polygon=ordered_boundary([(tuple(s.GetStart()),tuple(s.GetEnd())) for s in shapes])
            points=set(polygon)
            x0, y0 = min(p[0] for p in points), min(p[1] for p in points)
            x1, y1 = max(p[0] for p in points), max(p[1] for p in points)
            corners = [(x0,y0),(x1,y0),(x1,y1),(x0,y1)]
            center, size = [(x0+x1)/2, (y0+y1)/2], [x1-x0,y1-y0]
            shape = {'kind': 'rect', 'size': {'x': size[0], 'y': size[1]}}
            if len(polygon)!=4 or points!=set(corners):
                parts=[{'vertices':[{'x':p[0]-center[0],'y':p[1]-center[1]} for p in triangle]}
                       for triangle in triangulate(polygon)]
                evidence += '; exact union of convex polygon parts'
        assert min(size) > 0
        result[ref] = {'center': center, 'size': size, 'shape': shape,
                       'layers':['top' if front else 'bottom'],
                       'evidence': evidence,
                       'footprint': str(fp.GetFPID().GetLibItemName())}
        if parts:result[ref]['body_parts']=parts
    return dict(sorted(result.items()))


def shares_layer(a,b):
    return not a.get('layers') or not b.get('layers') or bool(set(a['layers'])&set(b['layers']))


def signed_gap(a, b):
    """Exact Euclidean separation outside; negative penetration inside."""
    if not shares_layer(a,b):return math.inf
    if a.get('body_parts') or b.get('body_parts'):
        if a['shape']['kind']=='circle':
            return min(circle_gap(a['center'],a['size'][0]/2,p) for p in body_polygons(b))
        if b['shape']['kind']=='circle':
            return min(circle_gap(b['center'],b['size'][0]/2,p) for p in body_polygons(a))
        return min(convex_gap(x,y) for x in body_polygons(a) for y in body_polygons(b))
    dx, dy = [abs(x-y) for x,y in zip(a['center'], b['center'])]
    ac, bc = a['shape']['kind'] == 'circle', b['shape']['kind'] == 'circle'
    if ac and bc:
        return math.hypot(dx,dy)-(a['size'][0]+b['size'][0])/2
    if ac or bc:
        circle, rect = (a,b) if ac else (b,a)
        qx, qy = dx-rect['size'][0]/2, dy-rect['size'][1]/2
        return math.hypot(max(qx,0),max(qy,0))+min(max(qx,qy),0)-circle['size'][0]/2
    qx, qy = dx-(a['size'][0]+b['size'][0])/2, dy-(a['size'][1]+b['size'][1])/2
    return math.hypot(max(qx,0),max(qy,0))+min(max(qx,qy),0)


def audit(bodies, bounds, overhang=None, clearance=None):
    overhang = overhang or {}
    clearance = clearance or {}
    assert not (set(overhang)-set(bodies))
    assert all(math.isfinite(v) and v >= 0 for v in overhang.values())
    assert all(math.isfinite(v) and v >= 0 for v in clearance.values())
    pairs = [{'first':a,'second':b,'gap_mm':signed_gap(bodies[a],bodies[b]),
              'clearance_mm':max(clearance.get(a,0),clearance.get(b,0))}
             for a,b in itertools.combinations(bodies,2) if shares_layer(bodies[a],bodies[b])]
    outside = []
    for ref, b in bodies.items():
        x,y = b['center']; w,h = b['size']; x0,y0,x1,y1 = bounds
        excess = max(0,x0-(x-w/2),y0-(y-h/2),x+w/2-x1,y+h/2-y1)
        if excess > 1e-6:
            outside.append({'component':ref,'overhang_mm':excess,
                            'allowed_mm':overhang.get(ref,0),
                            'passed':excess <= overhang.get(ref,0)+1e-6})
    return {'passed':all(p['gap_mm'] >= p['clearance_mm']-1e-6 for p in pairs) and all(p['passed'] for p in outside),
            'overlaps':[p for p in pairs if p['gap_mm'] < -1e-6],
            'clearance_violations':[p for p in pairs if p['gap_mm'] < p['clearance_mm']-1e-6],
            'closest_pairs':sorted(pairs,key=lambda p:p['gap_mm'])[:10],
            'outside':outside}


def render(bodies, bounds, path, title, pads=None):
    x0,y0,x1,y1 = bounds; width,height=x1-x0,y1-y0
    s=[f'<svg xmlns="http://www.w3.org/2000/svg" width="1000" height="950" viewBox="{x0-3} {y0-5} {width+6} {height+8}">',
       f'<rect x="{x0-3}" y="{y0-5}" width="{width+6}" height="{height+8}" fill="white"/>',
       f'<text x="{x0}" y="{y0-2}" font-size="1.1">{html.escape(title)}</text>',
       f'<rect x="{x0}" y="{y0}" width="{width}" height="{height}" fill="#f3f8f4" stroke="#222" stroke-width="0.15"/>']
    for ref,b in bodies.items():
        x,y=b['center'];w,h=b['size']
        common='fill="#68aeda" fill-opacity="0.28" stroke="#145580" stroke-width="0.1"'
        if b.get('body_parts'):
            for polygon in body_polygons(b):
                points=' '.join(f'{px},{py}' for px,py in polygon)
                s.append(f'<polygon points="{points}" {common}/>')
        elif b['shape']['kind']=='circle':s.append(f'<circle cx="{x}" cy="{y}" r="{w/2}" {common}/>')
        else:s.append(f'<rect x="{x-w/2}" y="{y-h/2}" width="{w}" height="{h}" {common}/>')
        s.append(f'<text x="{x}" y="{y}" text-anchor="middle" font-size="1.1">{html.escape(ref)}</text>')
    for items in (pads or {}).values():
        for pad in items:
            x,y=pad['center'];w,h=pad['size']
            s.append(f'<rect x="{x-w/2}" y="{y-h/2}" width="{w}" height="{h}" fill="#b34927" fill-opacity="0.4" stroke="#84351d" stroke-width="0.06"/>')
    path.write_text(''.join(s)+'</svg>')


def main():
    parser=argparse.ArgumentParser(description=__doc__)
    parser.add_argument('board',type=Path)
    parser.add_argument('output',type=Path)
    args=parser.parse_args(); args.output.mkdir(parents=True,exist_ok=False)
    pcbnew=load_pcbnew(); board=pcbnew.LoadBoard(str(args.board))
    edges=[s for s in board.GetDrawings() if s.GetLayer()==pcbnew.Edge_Cuts]
    assert len(edges)==4 and all(s.GetShape()==pcbnew.SHAPE_T_SEGMENT for s in edges)
    points=[(p.x/1e6,p.y/1e6) for s in edges for p in (s.GetStart(),s.GetEnd())]
    bounds=[min(p[0] for p in points),min(p[1] for p in points),max(p[0] for p in points),max(p[1] for p in points)]
    bodies=extract_courtyards(board)
    report={'source_sha256':digest(args.board),'scope':__doc__,'bounds':bounds,
            'bodies':bodies,'audit':audit(bodies,bounds)}
    write(args.output/'report.json',report)
    render(bodies,bounds,args.output/'preview.svg','Native courtyard geometry; human placement')
    print(report['audit'])


if __name__=='__main__':main()
