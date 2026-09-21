#!/usr/bin/env python3
# Copyright (C) 2026 Toit contributors.
"""Small, deterministic views for blinded board-analysis experiments.

No answer keys, opportunity ranking, or repair search are used here.
"""
import argparse
import json
import math
from pathlib import Path


def resolve_proposal(board, proposal):
    """Resolve explicit object references; never infer a path or select an edit."""
    import copy
    result=copy.deepcopy(proposal)
    routes={r['id']:r for r in board['routes']}
    pads={p['id']:p for p in board['pads']}
    for replacement in result.get('replacements',[]):
        route=routes[replacement['route_id']]
        points=[]
        for point in replacement['points']:
            if point=='start':
                point=route['points'][0]
            elif point=='end':
                point=route['points'][-1]
            elif isinstance(point,dict):
                if set(point)=={'pad'}:
                    point=pads[point['pad']]['at']
                elif set(point)=={'point_index'}:
                    point=route['points'][point['point_index']]
                else:
                    raise ValueError('Unknown point reference')
            points.append(point)
        replacement['points']=points
    return result


def point_segment_distance(p, a, b):
    v = (b[0]-a[0], b[1]-a[1])
    length2 = v[0]*v[0]+v[1]*v[1]
    t = max(0, min(1, ((p[0]-a[0])*v[0]+(p[1]-a[1])*v[1])/length2)) if length2 else 0
    return math.dist(p, (a[0]+t*v[0], a[1]+t*v[1]))


def segment_distance(a, b, c, d):
    def cross(p,q,r):
        return (q[0]-p[0])*(r[1]-p[1])-(q[1]-p[1])*(r[0]-p[0])
    ab_c,ab_d,cd_a,cd_b=cross(a,b,c),cross(a,b,d),cross(c,d,a),cross(c,d,b)
    if ab_c*ab_d < 0 and cd_a*cd_b < 0:
        return 0.0
    return min(point_segment_distance(a,c,d),point_segment_distance(b,c,d),
               point_segment_distance(c,a,b),point_segment_distance(d,a,b))


def rectangle_distance(a,b,rect):
    x0,y0,x1,y1=rect
    if any(x0<=p[0]<=x1 and y0<=p[1]<=y1 for p in (a,b)):
        return 0.0
    corners=[(x0,y0),(x1,y0),(x1,y1),(x0,y1)]
    return min(segment_distance(a,b,corners[i],corners[(i+1)%4]) for i in range(4))


def probe(board, a, b, layer, net):
    """Diagnostic only: fixed-geometry clearance of one proposed planar segment."""
    clearance=board['rules']['clearance']
    radius=board['rules']['trace_width']/2
    findings=[]
    def record(object_id,kind,distance,required):
        findings.append(dict(object_id=object_id,kind=kind,distance_mm=distance,
                             required_mm=required,margin_mm=distance-required))
    for obs in board.get('obstacles',[]):
        if layer in obs['layers']:
            record(obs['id'],'obstacle',rectangle_distance(a,b,obs['rect']),radius+clearance)
    for pad in board['pads']:
        if pad['net'] != net and layer in pad['layers']:
            record(pad['id'],'pad',point_segment_distance(pad['at'],a,b),radius+clearance+pad['diameter']/2)
    for route in board['routes']:
        if route['net']==net:
            continue
        for i,(c,d) in enumerate(zip(route['points'],route['points'][1:])):
            if route['layers'][i]==layer:
                record(f"{route['id']}:s{i}",'trace',segment_distance(a,b,c,d),2*radius+clearance)
        for via in route_summary(route)['transitions']:
            record(f"{route['id']}:v{via['point_index']}",'via',point_segment_distance(via['at'],a,b),radius+clearance+board['rules']['via_diameter']/2)
    findings.sort(key=lambda f:f['margin_mm'])
    return dict(scope='Diagnostic planar-segment query; not full route/connectivity/via/board-boundary validation.',
                start=a,end=b,layer=layer,net=net,
                blockers=[f for f in findings if f['margin_mm'] < -1e-9],nearest=findings[:5])


def probe_via(board, at, net):
    """Check a proposed through-via's copper against both layers, not a route."""
    clearance=board['rules']['clearance']
    radius=board['rules']['via_diameter']/2
    findings=[]
    def record(object_id,kind,distance,required,layers):
        findings.append(dict(object_id=object_id,kind=kind,layers=layers,
                             distance_mm=distance,required_mm=required,
                             margin_mm=distance-required))
    for obs in board.get('obstacles',[]):
        if set(obs['layers']) & {'top','bottom'}:
            record(obs['id'],'obstacle',rectangle_distance(at,at,obs['rect']),
                   radius+clearance,obs['layers'])
    for pad in board['pads']:
        if pad['net']!=net and set(pad['layers']) & {'top','bottom'}:
            record(pad['id'],'pad',math.dist(at,pad['at']),
                   radius+pad['diameter']/2+clearance,pad['layers'])
    for route in board['routes']:
        if route['net']==net:
            continue
        for i,(a,b) in enumerate(zip(route['points'],route['points'][1:])):
            record(f"{route['id']}:s{i}",'trace',point_segment_distance(at,a,b),
                   radius+board['rules']['trace_width']/2+clearance,[route['layers'][i]])
        for via in route_summary(route)['transitions']:
            record(f"{route['id']}:v{via['point_index']}",'via',math.dist(at,via['at']),
                   2*radius+clearance,['top','bottom'])
    findings.sort(key=lambda f:f['margin_mm'])
    return dict(scope='Diagnostic through-via copper query on both layers; not connectivity, drill, board-boundary or full-route validation.',
                at=at,net=net,diameter_mm=2*radius,
                blockers=[f for f in findings if f['margin_mm'] < -1e-9],nearest=findings[:5])


def route_summary(route):
    points = route['points']
    layers = route['layers']
    transitions = [dict(point_index=i, at=points[i], before=layers[i-1], after=layers[i])
                   for i in range(1, len(layers)) if layers[i-1] != layers[i]]
    length = sum(math.dist(a, b) for a, b in zip(points, points[1:]))
    direct = math.dist(points[0], points[-1])
    return dict(id=route['id'], net=route['net'], length_mm=length,
                endpoint_distance_mm=direct, detour_ratio=length/direct if direct else None,
                transitions=transitions, points=points, layers=layers)


def route_index(board):
    rows=[]
    for r in board['routes']:
        s=route_summary(r)
        rows.append(dict(id=r['id'],net=r['net'],length_mm=s['length_mm'],
                         endpoint_distance_mm=s['endpoint_distance_mm'],via_count=len(s['transitions']),
                         bounds=[min(p[0] for p in r['points']),min(p[1] for p in r['points']),
                                 max(p[0] for p in r['points']),max(p[1] for p in r['points'])]))
    return dict(id=board['id'],bounds=board['bounds'],rules=board['rules'],routes=rows)


def region(board, bounds):
    """Conservative local extraction. Keep whole objects and boundary connectivity."""
    import copy
    result=copy.deepcopy(board)
    margin=board['rules']['clearance']+max(board['rules']['trace_width'],board['rules']['via_diameter'])/2
    x0,y0,x1,y1=bounds
    def intersects(box):
        return box[2]>=x0-margin and box[0]<=x1+margin and box[3]>=y0-margin and box[1]<=y1+margin
    result['obstacles']=[o for o in board['obstacles'] if intersects(o['rect'])]
    result['pads']=[p for p in board['pads'] if intersects([p['at'][0]-p['diameter']/2,p['at'][1]-p['diameter']/2,
                                                        p['at'][0]+p['diameter']/2,p['at'][1]+p['diameter']/2])]
    result['routes']=[r for r in board['routes'] if intersects([min(p[0] for p in r['points']),min(p[1] for p in r['points']),
                                                              max(p[0] for p in r['points']),max(p[1] for p in r['points'])])]
    result['view_bounds']=bounds
    result['scope']='Local broad-phase selection; whole routes retained. Not a standalone board or proof of global validity.'
    return result


def render(board, output, bounds=None, selected_net=None, overlay=False, labels=True):
    from PIL import Image, ImageDraw, ImageFont
    bounds = bounds or board['bounds']
    x0, y0, x1, y1 = bounds
    panels = ['overlay'] if overlay else ['top', 'bottom']
    panel_w, panel_h = 760, 620
    im = Image.new('RGB', (panel_w * len(panels), panel_h), 'white')
    d = ImageDraw.Draw(im)
    import subprocess
    font_path = subprocess.check_output(['fc-match', '-f', '%{file}', 'sans'], text=True)
    font = ImageFont.truetype(font_path, 13)
    title_font = ImageFont.truetype(font_path, 18)
    scale = min((panel_w-100)/(x1-x0), (panel_h-130)/(y1-y0))
    colors = {'top': '#bd342d', 'bottom': '#285fbd'}
    for k, layer in enumerate(panels):
        ox, oy = k * panel_w + 55, 70
        def xy(p):
            return (ox+(p[0]-x0)*scale, oy+(p[1]-y0)*scale)
        d.text((k*panel_w+25, 12), f"{board['id']} | {layer} | mm | y increases downward", fill='black', font=title_font)
        if list(bounds)!=list(board['bounds']):
            d.text((k*panel_w+25,38),'Cropped view: frame is viewport, not board boundary',fill='#555555',font=font)
        for x in range(math.ceil(x0/5)*5, math.floor(x1/5)*5+1, 5):
            a, b = xy((x,y0)), xy((x,y1))
            d.line([a,b], fill='#ededed')
            d.text((a[0]-7, a[1]-20), str(x), fill='#555555', font=font)
        for y in range(math.ceil(y0/5)*5, math.floor(y1/5)*5+1, 5):
            a, b = xy((x0,y)), xy((x1,y))
            d.line([a,b], fill='#ededed')
            d.text((a[0]-30, a[1]-7), str(y), fill='#555555', font=font)
        d.rectangle([xy((x0,y0)),xy((x1,y1))], outline='black', width=2)
        for obs in board.get('obstacles', []):
            if layer != 'overlay' and layer not in obs['layers']:
                continue
            a,b,c,e=obs['rect']
            a,c=max(a,x0),min(c,x1)
            b,e=max(b,y0),min(e,y1)
            if a>c or b>e:
                continue
            d.rectangle([xy((a,b)),xy((c,e))],fill='#d5d5d5',outline='#686868',width=2)
            if labels:
                d.text(xy((a,b)),obs['id']+' '+ '/'.join(obs['layers']),fill='black',font=font)
        base_draw=d
        ink=Image.new('RGBA',im.size,(0,0,0,0))
        d=ImageDraw.Draw(ink)
        for route in board['routes']:
            active = selected_net is None or route['net'] == selected_net
            for i, (a,b) in enumerate(zip(route['points'],route['points'][1:])):
                seg_layer=route['layers'][i]
                if layer != 'overlay' and layer != seg_layer:
                    continue
                color=colors[seg_layer] if active else '#c5c5c5'
                d.line([xy(a),xy(b)], fill=color, width=max(2, round(board['rules']['trace_width']*scale)))
                if labels and active and i == 0:
                    mid=((a[0]+b[0])/2,(a[1]+b[1])/2)
                    px,py=xy(mid)
                    d.text((px+4,py+4),route['id']+' / '+route['net'],fill=color,font=font)
            for via in route_summary(route)['transitions']:
                px,py=xy(via['at'])
                r=board['rules']['via_diameter']*scale/2
                d.ellipse((px-r,py-r,px+r,py+r), fill='#f3d374' if active else '#dddddd',outline='black',width=2)
                r=board['rules']['via_drill']*scale/2
                d.ellipse((px-r,py-r,px+r,py+r),fill='white')
                if labels and active:
                    d.text((px+7,py-18),f"{route['id']}:v{via['point_index']}",fill='black',font=font)
        for pad in board['pads']:
            if layer != 'overlay' and layer not in pad['layers']:
                continue
            px,py=xy(pad['at'])
            r=pad['diameter']*scale/2
            d.ellipse((px-r,py-r,px+r,py+r),fill='#57a078',outline='black')
            if labels:
                endpoints=[]
                for route in board['routes']:
                    if route['net']==pad['net']:
                        if pad['at']==route['points'][0]: endpoints.append('S')
                        if pad['at']==route['points'][-1]: endpoints.append('E')
                suffix=' '+ '/'.join(sorted(set(endpoints))) if endpoints else ''
                d.text((px+5,py+5),pad['id']+suffix,fill='#16482b',font=font)
        box=(round(ox),round(oy),round(ox+(x1-x0)*scale)+1,round(oy+(y1-y0)*scale)+1)
        clipped=ink.crop(box)
        im.paste(clipped,box,clipped)
        d=base_draw
        d.text((k*panel_w+25,panel_h-45), 'Red: top copper   Blue: bottom copper   Gold: via   Gray: hard routing obstacle', fill='black',font=font)
        d.text((k*panel_w+25,panel_h-25),f"Trace {board['rules']['trace_width']}; gap {board['rules']['clearance']}; via {board['rules']['via_diameter']}/{board['rules']['via_drill']} mm. S/E: route start/end.",fill='black',font=font)
    Path(output).parent.mkdir(parents=True, exist_ok=True)
    im.save(output)


def main():
    p=argparse.ArgumentParser(description=__doc__)
    p.add_argument('board')
    p.add_argument('command',choices=['summary','index','net','render','probe','probe-via','anchors','compile','region','check'])
    p.add_argument('--net')
    p.add_argument('--bounds',type=float,nargs=4)
    p.add_argument('--output')
    p.add_argument('--overlay',action='store_true')
    p.add_argument('--from',dest='start',type=float,nargs=2)
    p.add_argument('--to',dest='end',type=float,nargs=2)
    p.add_argument('--at',type=float,nargs=2)
    p.add_argument('--layer',choices=['top','bottom'])
    p.add_argument('--proposal')
    args=p.parse_args()
    board=json.loads(Path(args.board).read_text())
    if args.proposal and args.command in ['probe','probe-via']:
        from generate import apply_proposal
        board=apply_proposal(board,json.loads(Path(args.proposal).read_text()))
    if args.command == 'render':
        if not args.output:
            p.error('render requires --output')
        render(board,args.output,args.bounds,args.net,args.overlay)
        print(args.output)
    elif args.command=='index':
        print(json.dumps(route_index(board),indent=2))
    elif args.command=='region':
        if not args.bounds:
            p.error('region requires --bounds X0 Y0 X1 Y1')
        print(json.dumps(region(board,args.bounds),indent=2))
    elif args.command=='check':
        if not args.proposal:
            p.error('check requires --proposal FILE')
        from generate import evaluate
        print(json.dumps(evaluate(board,json.loads(Path(args.proposal).read_text())),indent=2))
    elif args.command=='compile':
        if not args.proposal:
            p.error('compile requires --proposal FILE')
        print(json.dumps(resolve_proposal(board,json.loads(Path(args.proposal).read_text())),indent=2))
    elif args.command=='anchors':
        print(json.dumps(dict(rules=board['rules'],pads=board['pads'],
            routes=[dict(id=r['id'],net=r['net'],start=r['points'][0],end=r['points'][-1]) for r in board['routes']]),indent=2))
    elif args.command=='probe':
        if args.start is None or args.end is None or not args.layer or not args.net:
            p.error('probe requires --from X Y --to X Y --layer L --net N')
        print(json.dumps(probe(board,args.start,args.end,args.layer,args.net),indent=2))
    elif args.command=='probe-via':
        if args.at is None or not args.net:
            p.error('probe-via requires --at X Y --net N')
        print(json.dumps(probe_via(board,args.at,args.net),indent=2))
    else:
        routes=[r for r in board['routes'] if args.net is None or r['net']==args.net]
        result=dict(id=board['id'],bounds=board['bounds'],rules=board['rules'],routes=[route_summary(r) for r in routes])
        if args.command=='net':
            result['pads']=[pad for pad in board['pads'] if pad['net']==args.net]
            result['obstacles']=board.get('obstacles',[])
            result['context_routes']=[route_summary(r) for r in board['routes'] if r['net']!=args.net]
        print(json.dumps(result,indent=2))


if __name__=='__main__':
    main()
