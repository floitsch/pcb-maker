#!/usr/bin/env python3
# Copyright (C) 2026 Toit contributors.
"""Exact native-object lookup with conservative local context, no repair search."""
import argparse
import json
import math
from pathlib import Path
from native_renderer import render


def object_bounds(obj):
    if 'start' in obj:
        a,b=obj['start'],obj['end']
        r=obj['width']/2
        return [min(a[0],b[0])-r,min(a[1],b[1])-r,max(a[0],b[0])+r,max(a[1],b[1])+r]
    x,y=obj['at']
    r=math.hypot(*obj['size'])/2 if 'size' in obj else obj['diameter']/2
    # Pad circumcircle is deliberately conservative for rotation/shape.
    r+=math.hypot(*obj.get('offset',[0,0]))
    return [x-r,y-r,x+r,y+r]


def region(board,bounds):
    x0,y0,x1,y1=bounds
    def overlaps(obj):
        a,b,c,d=object_bounds(obj)
        return a<=x1 and c>=x0 and b<=y1 and d>=y0
    return dict(bounds=bounds,tracks=[t for t in board['tracks'] if overlaps(t)],
                vias=[v for v in board['vias'] if overlaps(v)],pads=[p for p in board['pads'] if overlaps(p)])


def main():
    p=argparse.ArgumentParser(description=__doc__)
    p.add_argument('board')
    p.add_argument('command',choices=['index','net','objects','region','render','neighborhood'])
    p.add_argument('--net')
    p.add_argument('--ids',nargs='+')
    p.add_argument('--bounds',type=float,nargs=4)
    p.add_argument('--output')
    p.add_argument('--radius',type=float,default=3)
    args=p.parse_args()
    b=json.loads(Path(args.board).read_text())
    if args.command=='render':
        if not args.output:
            p.error('render requires --output')
        render(b,args.output,args.net,args.bounds,labels=True)
        print(args.output)
        return
    common=dict(id=b['id'],units=b['units'],layers=b['layers'],
                rules=b['board_rules'],net_settings=b['net_settings'])
    for key in ['effective_netclasses','netclasses_by_net','effective_board_rules']:
        if key in b:
            common[key]=b[key]
    objects=b['tracks']+b['vias']+b['pads']
    if args.command=='index':
        common['nets']=[dict(id=n,name=name,track_count=sum(t['net']==n for t in b['tracks']),
            stored_length_mm=sum(math.dist(t['start'],t['end']) for t in b['tracks'] if t['net']==n),
            vias=[dict(id=v['id'],at=v['at']) for v in b['vias'] if v['net']==n],
            pads=[p['id'] for p in b['pads'] if p['net']==n]) for n,name in b['nets'].items()]
    elif args.command=='objects':
        selected=[o for o in objects if o['id'] in (args.ids or [])]
        common['objects']=selected
        common['missing_ids']=sorted(set(args.ids or [])-{o['id'] for o in selected})
    elif args.command=='neighborhood':
        selected=[o for o in objects if o['id'] in (args.ids or [])]
        if not selected:
            p.error('neighborhood requires existing --ids')
        boxes=[object_bounds(o) for o in selected]
        bounds=[min(x[0] for x in boxes)-args.radius,min(x[1] for x in boxes)-args.radius,
                max(x[2] for x in boxes)+args.radius,max(x[3] for x in boxes)+args.radius]
        common['selected_ids']=args.ids
        common.update(region(b,bounds))
    elif args.command=='region':
        if not args.bounds:
            p.error('region requires --bounds')
        common.update(region(b,args.bounds))
    elif args.command=='net':
        if not args.net:
            p.error('net requires --net')
        selected=[o for o in objects if o['net']==args.net]
        common['selected_net']=args.net
        common['objects']=selected
        if selected:
            boxes=[object_bounds(o) for o in selected]
            bounds=[min(x[0] for x in boxes)-2,min(x[1] for x in boxes)-2,
                    max(x[2] for x in boxes)+2,max(x[3] for x in boxes)+2]
            common['context']=region(b,bounds)
    print(json.dumps(common,indent=2))


if __name__=='__main__':
    main()
