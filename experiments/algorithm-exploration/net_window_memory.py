#!/usr/bin/env python3
# Copyright (C) 2026 Toit contributors.
"""Memory of per-net fields over each net's bounding box (+2 mm), for the GPU survey.

usage: net_window_memory.py [corpus.json]  (default benchmarks/corpus/corpus.json)
"""
import json, math, os, re, sys
def parse(s):
    toks=re.findall(r'\(|\)|"(?:[^"\\]|\\.)*"|[^\s()]+',s)
    st=[[]]
    for t in toks:
        if t=='(': st.append([])
        elif t==')': x=st.pop(); st[-1].append(x)
        else: st[-1].append(t.strip('"') if t.startswith('"') else t)
    return st[0][0]
def find(n,key): return [c for c in n if isinstance(c,list) and c and c[0]==key]
c=json.load(open(sys.argv[1] if len(sys.argv)>1 else 'benchmarks/corpus/corpus.json'))
print(f"{'board':12} {'layers':>6} {'nets':>5} {'board mm2':>9} {'sum bbox+2mm':>12} {'ratio':>6} {'max net':>8} | MB/field @0.1mm @0.05mm (all nets, f32, per layer-stack)")
for b in c['boards']:
    p=os.path.join(b['directory'],b['board_id']+'.kicad_pcb')
    if not os.path.exists(p): continue
    t=parse(open(p).read())
    layers=[l for l in find(t,'layers')[0][1:] if isinstance(l,list) and len(l)>2 and l[2] in ('signal','power','mixed')]
    nl=len(layers)
    pads={}
    xs=[];ys=[]
    for g in find(t,'gr_line')+find(t,'gr_rect')+find(t,'gr_arc')+find(t,'gr_poly'):
        lay=find(g,'layer')
        if lay and lay[0][1]=='Edge.Cuts':
            for k in ('start','end','mid'):
                for q in find(g,k): xs.append(float(q[1])); ys.append(float(q[2]))
            for pts in find(g,'pts'):
                for q in find(pts,'xy'): xs.append(float(q[1])); ys.append(float(q[2]))
    for fp in find(t,'footprint'):
        at=find(fp,'at')[0]; fx,fy=float(at[1]),float(at[2]); fr=float(at[3]) if len(at)>3 else 0
        for pd in find(fp,'pad'):
            nt=find(pd,'net')
            if not nt: continue
            name=nt[0][-1]
            pa=find(pd,'at')[0]; px,py=float(pa[1]),float(pa[2])
            a=math.radians(-fr); x=fx+px*math.cos(a)-py*math.sin(a); y=fy+px*math.sin(a)+py*math.cos(a)
            pads.setdefault(name,[]).append((x,y))
    bw=(max(xs)-min(xs)) if xs else 0; bh=(max(ys)-min(ys)) if ys else 0
    area=bw*bh
    s=0; mx=0; n=0
    for name,pts in pads.items():
        if len(pts)<2: continue
        n+=1
        w=max(p[0] for p in pts)-min(p[0] for p in pts)+4; h=max(p[1] for p in pts)-min(p[1] for p in pts)+4
        w=min(w,bw) if bw else w; h=min(h,bh) if bh else h
        s+=w*h; mx=max(mx,w*h)
    mb=lambda pitch: s/(pitch*pitch)*nl*4/1e6
    print(f"{b['name']:12} {nl:6} {n:5} {area:9.0f} {s:12.0f} {s/area if area else 0:6.1f} {mx/area if area else 0:8.2f} | {mb(0.1):7.0f} {mb(0.05):7.0f}")
