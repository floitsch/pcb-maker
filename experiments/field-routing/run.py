#!/usr/bin/env python3
# Copyright (C) 2026 Toit contributors.
"""CPU resistor-field routing experiment. No production or KiCad dependency."""
import argparse
from collections import Counter
import heapq
import html
import json
import math
from pathlib import Path
import random
import time

import numpy as np


def fixtures():
    w, h = 21, 15
    def case(name, free, nets, witness=None, layers=1):
        return dict(name=name, width=w, height=h, layers=layers,
                    free=sorted(free), nets=nets, witness=witness)
    plane = {(x, y, 0) for x in range(w) for y in range(h)}
    wall = {(10, y, 0) for y in range(2, h)}
    yield case('single_wall_detour', plane-wall, [((2, 7, 0), (18, 7, 0))],
               [[(x,7,0) for x in range(2,10)]+[(9,y,0) for y in range(6,0,-1)]+
                [(x,1,0) for x in range(10,19)]+[(18,y,0) for y in range(2,8)]])
    corridor = {(x,7,0) for x in range(1,20)} | {(x,2,0) for x in range(1,20)}
    corridor |= {(x,y,0) for x in [1,19] for y in range(2,8)}
    corridor |= {(10,y,0) for y in range(5,10)}
    witness = [[(1,y,0) for y in range(7,1,-1)]+[(x,2,0) for x in range(2,20)]+
               [(19,y,0) for y in range(3,8)], [(10,y,0) for y in range(5,10)]]
    yield case('corridor_order_trap', corridor,
               [((1,7,0),(19,7,0)), ((10,5,0),(10,9,0))], witness)
    cross = {(x,7,0) for x in range(w)} | {(10,y,0) for y in range(h)}
    nets = [((0,7,0),(20,7,0)), ((10,0,0),(10,14,0))]
    yield case('single_layer_cross_impossible', cross, nets)
    layered = cross | {(x,y,1) for x,y,z in cross}
    witness = [[(x,7,0) for x in range(w)], [(10,0,0),(10,0,1)]+
               [(10,y,1) for y in range(1,h)]+[(10,14,0)]]
    yield case('two_layer_cross', layered, nets, witness, layers=2)
    yield case('disconnected_wall', plane-{(10,y,0) for y in range(h)},
               [((2,7,0),(18,7,0))])


def graph(case, directions, forbidden=()):
    points = [tuple(p) for p in case['free'] if tuple(p) not in forbidden]
    ids = {p:i for i,p in enumerate(points)}
    edges = [[] for _ in points]
    offsets = [(1,0),(-1,0),(0,1),(0,-1)]
    if directions == 8:
        offsets += [(1,1),(1,-1),(-1,1),(-1,-1)]
    for i,(x,y,z) in enumerate(points):
        for dx,dy in offsets:
            q = (x+dx,y+dy,z)
            if q not in ids:
                continue
            if dx and dy and ((x+dx,y,z) not in ids or (x,y+dy,z) not in ids):
                continue
            edges[i].append((ids[q], math.hypot(dx,dy)))
        q = (x,y,1-z)
        if q in ids:
            edges[i].append((ids[q], 6.0))
    return points, ids, edges


def shortest(points, ids, edges, source, target, price):
    if source not in ids or target not in ids:
        return [], dict(expansions=0)
    s,t = ids[source],ids[target]
    queue, dist, parent, count = [(0,s)], {s:0}, {}, 0
    while queue:
        cost,u = heapq.heappop(queue)
        if cost != dist[u]:
            continue
        count += 1
        if u == t:
            path=[u]
            while path[-1] != s:
                path.append(parent[path[-1]])
            return [points[i] for i in path[::-1]], dict(expansions=count)
        for v,length in edges[u]:
            new = cost + length * (1 + (price.get(points[u],0)+price.get(points[v],0))/2)
            if new < dist.get(v, float('inf')):
                dist[v],parent[v]=new,u
                heapq.heappush(queue,(new,v))
    return [],dict(expansions=count)


def field(points, ids, edges, source, target, price):
    work=dict(sweeps=0, cell_updates=0, active_cell_updates=0, reachability_visits=0,
              converged=False, residual=None, potential=[])
    if source not in ids or target not in ids:
        return [],work
    s,t=ids[source],ids[target]
    seen,stack={s},[s]
    while stack:
        u=stack.pop()
        for v,_ in edges[u]:
            if v not in seen:
                seen.add(v); stack.append(v)
    work['reachability_visits']=len(seen)
    if t not in seen:
        work['reason']='disconnected'
        return [],work
    # Steady resistor field: source 1, target 0, insulated obstacle boundaries.
    # Jacobi sweeps are local, synchronous, and readily expressible as a stencil.
    a,b,c=[],[],[]
    for u in sorted(seen):
        for v,length in edges[u]:
            a.append(u); b.append(v)
            c.append(1/(length*(1+(price.get(points[u],0)+price.get(points[v],0))/2)))
    a,b,c=np.array(a),np.array(b),np.array(c)
    degree=np.bincount(a,weights=c,minlength=len(points))
    degree[degree==0]=1
    voltage=np.full(len(points),0.5); voltage[s]=1; voltage[t]=0
    snapshots=[]
    for sweep in range(16000):
        new=np.bincount(a,weights=c*voltage[b],minlength=len(points))/degree
        new[s]=1; new[t]=0
        # Damping prevents period-two oscillation on bipartite grids.
        new=.5*voltage+.5*new
        residual=float(np.max(np.abs(new-voltage)))
        voltage=new
        if sweep in [0,4,19,99,499,1999]:
            snapshots.append([[*points[i],round(float(voltage[i]),5)] for i in sorted(seen)])
        if residual < 1e-7:
            work['converged']=True
            break
    work.update(sweeps=sweep+1,cell_updates=(sweep+1)*len(points),
                active_cell_updates=(sweep+1)*len(seen),residual=residual,
                potential=[[*points[i],round(float(voltage[i]),5)] for i in sorted(seen)],
                snapshots=snapshots)
    if not work['converged']:
        work['reason']='iteration_budget'
        return [],work
    path=[s]
    while path[-1] != t:
        u=path[-1]
        candidates=[]
        for v,length in edges[u]:
            if voltage[v] < voltage[u]-1e-10:
                conductance=1/(length*(1+(price.get(points[u],0)+price.get(points[v],0))/2))
                candidates.append((conductance*(voltage[u]-voltage[v]),-v,v))
        if not candidates:
            work['reason']='flat_field'
            return [],work
        path.append(max(candidates)[2])
        if len(path)>len(points):
            raise AssertionError('strict potential descent must be acyclic')
    return [points[i] for i in path],work


def validate(case, routes, directions):
    """Independent geometry checker: does not use graph adjacency or router helpers."""
    free=set(map(tuple,case['free'])); errors=[]; occupied={}; diagonals={}
    length=0.; vias=0; completed=0
    if len(routes)!=len(case['nets']): errors.append('route_count')
    for k,(path,ends) in enumerate(zip(routes,case['nets'])):
        if not path:
            continue
        completed+=1
        if tuple(path[0]) != tuple(ends[0]) or tuple(path[-1]) != tuple(ends[1]):
            errors.append('wrong_terminal')
        for point in map(tuple,path):
            if point not in free: errors.append('obstacle')
            if any(point==tuple(p) for j,terminals in enumerate(case['nets']) if j!=k for p in terminals):
                errors.append('foreign_terminal')
            if point in occupied and occupied[point]!=k: errors.append('shared_cell')
            occupied[point]=k
        for p,q in zip(path,path[1:]):
            dx,dy,dz=[abs(p[i]-q[i]) for i in range(3)]
            if dz:
                if dx or dy or dz!=1: errors.append('invalid_via')
                vias+=1
            else:
                if max(dx,dy)!=1 or (directions==4 and dx+dy!=1): errors.append('invalid_step')
                length+=math.hypot(dx,dy)
                if dx and dy:
                    if (p[0],q[1],p[2]) not in free or (q[0],p[1],p[2]) not in free:
                        errors.append('corner_cut')
                    key=(min(p[0],q[0]),min(p[1],q[1]),p[2])
                    slope=(q[0]-p[0])*(q[1]-p[1])
                    if key in diagonals and diagonals[key][0]!=k and diagonals[key][1]!=slope:
                        errors.append('diagonal_crossing')
                    diagonals[key]=(k,slope)
    return dict(complete=completed==len(case['nets']) and not errors,
                connected_nets=completed, errors=dict(Counter(errors)), length_cells=length,
                vias=vias, objective=length+6*vias)


def run(case, directions, method, seed):
    started=time.perf_counter(); rng=random.Random(seed)
    history=Counter(); frames=[]; total=Counter(); routes=[[] for _ in case['nets']]
    negotiated=method.endswith('negotiated')
    feedback=method.endswith('feedback') or negotiated
    sequential=method.endswith('sequential')
    solver=field if method.startswith('field') else shortest
    for iteration in range(24 if feedback else 1):
        routes=[[] for _ in case['nets']]; occupied=set()
        order=list(range(len(routes)))
        if feedback: rng.shuffle(order)
        work=[]
        for k in order:
            source,target=map(tuple,case['nets'][k])
            # Foreign terminals are hard obstacles under every policy.
            forbidden={tuple(p) for j,ends in enumerate(case['nets']) if j!=k for p in ends}
            if sequential: forbidden |= occupied
            points,ids,edges=graph(case,directions,forbidden)
            price=history.copy()
            if negotiated:
                for point in occupied: price[point]+=8
            routes[k],detail=solver(points,ids,edges,source,target,price)
            occupied.update(routes[k]); work.append(detail)
            for key in ['expansions','sweeps','cell_updates','active_cell_updates','reachability_visits']:
                total[key]+=detail.get(key,0)
            if solver is field:
                total['field_solves']+=1
                total['converged_fields']+=int(detail['converged'])
                if detail.get('reason'): total[detail['reason']]+=1
        check=validate(case,routes,directions)
        potential=next((d.get('potential') for d in work if d.get('potential')),[])
        if iteration==0:
            snapshots=next((d.get('snapshots') for d in work if d.get('snapshots')),[])
            for i,snapshot in enumerate(snapshots):
                frames.append(dict(label=f'field diffusion snapshot {i+1}', routes=[],potential=snapshot,history=[]))
        frames.append(dict(label=f'pass {iteration+1}: {check["connected_nets"]} connected; '
                           f'{sum(check["errors"].values())} conflicts',routes=routes,
                           potential=potential,history=[[*p,v] for p,v in sorted(history.items())]))
        if check['complete']: break
        usage=Counter(p for path in routes for p in set(path))
        for p,n in usage.items():
            if n>1: history[p]+=8*(n-1)
    return dict(case=case['name'],neighbors=directions,method=method,seed=seed,
                passes=iteration+1,seconds=time.perf_counter()-started,work=dict(total),
                validation=check,routes=routes,frames=frames)


def static_svg(case, result):
    size=20; gap=460
    def xy(point): return (20+point[2]*gap+(point[0]+.5)*size,40+(point[1]+.5)*size)
    out=['<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 930 400">',
         '<rect width="930" height="400" fill="#e2e8f0"/>',
         f'<text x="20" y="20" font-family="sans-serif" font-size="14">{html.escape(result["case"]+" / "+result["method"]+" / complete="+str(result["validation"]["complete"]))}</text>']
    for p in case['free']:
        x,y=xy(p);out.append(f'<rect x="{x-9}" y="{y-9}" width="18" height="18" fill="white"/>')
    for k,path in enumerate(result['routes']):
        color=['#047857','#9a3412'][k%2]
        for p,q in zip(path,path[1:]):
            x,y=xy(p);a,b=xy(q)
            if p[2]==q[2]: out.append(f'<path d="M{x},{y} L{a},{b}" stroke="{color}" stroke-width="4"/>')
            else:
                for cx,cy in [(x,y),(a,b)]: out.append(f'<circle cx="{cx}" cy="{cy}" r="7" stroke="{color}" fill="none" stroke-width="3"/>')
        for p in case['nets'][k]:
            x,y=xy(p);out.append(f'<circle cx="{x}" cy="{y}" r="6" fill="{color}"/>')
    return ''.join(out+['</svg>'])


HTML='''<!doctype html><meta charset="utf-8"><title>Field routing experiments</title>
<style>body{font:16px system-ui;background:#111827;color:#eee;margin:24px}canvas{background:#f8fafc;max-width:100%}select,button,input{font:inherit;margin:8px}pre{white-space:pre-wrap}small{color:#cbd5e1}</style>
<h1>Diffusion and congestion feedback</h1><p>CPU toy grids, unit cell capacity. These are experimental centerline layouts, not verified PCB copper.</p>
<select id="run"></select><button id="play">Play / pause</button><input id="frame" type="range" min="0" value="0"><span id="label"></span>
<p><small>Warm colors: source potential; cool colors: sink potential. Magenta cells: accumulated congestion price. Solid colored lines: separate nets. Circles: vias. Layers are side by side.</small></p>
<canvas id="view" width="1000" height="470"></canvas><pre id="metrics"></pre>
<script>const DATA=__DATA__; const s=document.querySelector('#run'), slider=document.querySelector('#frame'), ctx=document.querySelector('canvas').getContext('2d');
DATA.runs.forEach((r,i)=>{let o=document.createElement('option');o.value=i;o.textContent=`${r.case} / ${r.neighbors} directions / ${r.method} / seed ${r.seed} / ${r.validation.complete?'PASS':'INCOMPLETE'}`;s.append(o)});
let playing=false;document.querySelector('#play').onclick=()=>playing=!playing;
function draw(){let r=DATA.runs[+s.value], c=DATA.cases.find(c=>c.name===r.case), f=r.frames[+slider.value];slider.max=r.frames.length-1;ctx.clearRect(0,0,1000,470);const z=20, ox=24, oy=48, gap=460;function xy(p){return [ox+p[2]*gap+p[0]*z+z/2,oy+p[1]*z+z/2]};
for(let layer=0;layer<c.layers;layer++){ctx.fillStyle='#334155';ctx.fillText('Layer '+(layer+1),ox+layer*gap,25);ctx.fillRect(ox+layer*gap,oy,c.width*z,c.height*z)}
for(let p of c.free){let [x,y]=xy(p);ctx.fillStyle='#f1f5f9';ctx.fillRect(x-z/2+1,y-z/2+1,z-2,z-2)}
for(let p of f.potential){let [x,y]=xy(p);ctx.fillStyle=`hsl(${220*(1-p[3])} 65% 75%)`;ctx.fillRect(x-z/2+1,y-z/2+1,z-2,z-2)}
for(let p of f.history){let [x,y]=xy(p);ctx.fillStyle='#db2777';ctx.globalAlpha=Math.min(.85,p[3]/60);ctx.fillRect(x-z/2,y-z/2,z,z);ctx.globalAlpha=1}
const colors=['#047857','#9a3412','#7c3aed'];(f.routes||[]).forEach((path,n)=>{ctx.strokeStyle=colors[n%3];ctx.lineWidth=4;for(let i=1;i<path.length;i++){let a=path[i-1],b=path[i];if(a[2]!==b[2]){for(let p of [a,b]){ctx.beginPath();ctx.arc(...xy(p),7,0,7);ctx.stroke()}}else{ctx.beginPath();ctx.moveTo(...xy(a));ctx.lineTo(...xy(b));ctx.stroke()}}});
c.nets.forEach((ends,n)=>ends.forEach(p=>{ctx.fillStyle=colors[n%3];ctx.beginPath();ctx.arc(...xy(p),6,0,7);ctx.fill()}));document.querySelector('#label').textContent=f.label;document.querySelector('#metrics').textContent=JSON.stringify({...r,frames:undefined,routes:undefined},null,2)}
s.onchange=()=>{slider.value=0;draw()};slider.oninput=draw;setInterval(()=>{if(playing){slider.value=(+slider.value+1)%(+slider.max+1);draw()}},450);draw();</script>'''


def main():
    parser=argparse.ArgumentParser(); parser.add_argument('--output',type=Path,
        default=Path('build/algorithm-exploration-2026-09-07/field')); args=parser.parse_args()
    args.output.mkdir(parents=True,exist_ok=True)
    cases=list(fixtures()); runs=[]
    for c in cases:
        if c['witness'] is not None:
            assert validate(c,c['witness'],4)['complete'], c['name']
        for directions in [4,8]:
            for method in ['shortest_sequential','field_sequential','field_simultaneous','field_feedback','shortest_feedback','field_negotiated','shortest_negotiated']:
                for seed in ([1,7,19] if method.endswith(('feedback','negotiated')) else [7]):
                    result=run(c,directions,method,seed); runs.append(result)
                    print(c['name'],directions,method,seed,result['validation']['complete'],round(result['seconds'],3),flush=True)
                    # Every run is immediately rendered, including failures.
                    name=f'{c["name"]}-{directions}-{method}-{seed}'
                    data=dict(cases=[c],runs=[result])
                    (args.output/(name+'.html')).write_text(HTML.replace('__DATA__',json.dumps(data,separators=(',',':'))))
                    (args.output/(name+'.svg')).write_text(static_svg(c,result))
    data=dict(cases=cases,runs=runs,scope='Synthetic unit-cell routing; CPU weighted Jacobi, no GPU timing, no native PCB validity claim.')
    (args.output/'results.json').write_text(json.dumps(data,indent=2))
    (args.output/'index.html').write_text(HTML.replace('__DATA__',json.dumps(data,separators=(',',':'))))
    metrics=[{k:v for k,v in r.items() if k not in ['frames','routes']} for r in runs]
    (args.output/'metrics.json').write_text(json.dumps(metrics,indent=2))


if __name__=='__main__': main()
