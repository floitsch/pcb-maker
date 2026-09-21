#!/usr/bin/env python3
# Copyright (C) 2026 Toit contributors.
"""Production placement proposals; independent, exact-edge toy routing audit.

Run from repository root. No KiCad/native fabrication claim is made by this
deliberately bounded single-layer route probe. Each proposal gets an SVG and
the complete suite gets a standalone animated HTML viewer, including failures.
"""
import collections
import copy
import hashlib
import heapq
import html
import json
import math
from pathlib import Path
import subprocess
import time

ROOT = Path(__file__).resolve().parents[2]
OUT = ROOT / 'build/algorithm-exploration-2026-09-07/placement'
EXE = OUT / 'target/release/placement-exploration'

def point(x, y): return {'x': x, 'y': y}
def component(name, x, y, w=1, h=1, fixed=True, pins=None, wall=False):
    return {'id': name, 'position': point(x,y), 'size': point(w,h),
            'constraints': {'movement': 'fixed' if fixed else 'free', 'rotation': 'fixed'},
            'body_is_routing_keepout': wall,
            'pins': pins if pins is not None else [pin('1',0,0)]}
def pin(name,x,y):
    return {'id':name,'offset':point(x,y),'pads':[{'layer':'top','shape':{'kind':'circle','diameter':0.4}}]}
def net(name,a,ap,b,bp,width):
    return {'id':name,'from':{'component':a,'pin':ap},'to':{'component':b,'pin':bp},'width':width,'layer':'top'}
def fixtures():
    base={'schema_version':1,'board':{'bounds':{'min':point(0,0),'max':point(20,14)},'layers':[{'id':'top'}]},'rules':{'clearance':0.4,'via_diameter':0.8,'via_drill':0.4}}
    wall=copy.deepcopy(base)
    wall['components']=[component('L',2,7),component('R',18,7),component('WALL',10,7,2,11.6,False,[],True)]
    wall['components'][-1]['constraints']['movement']='vertical'
    wall['nets']=[net('SIGNAL','L','1','R','1',0.6)]
    crossed=copy.deepcopy(base); crossed['rules']['clearance']=0.2
    crossed['components']=[component('L1',2,3),component('R1',18,3),component('L2',2,11),component('R2',18,11),component('A',10,11,2,1,False,[pin('1',-1.2,0),pin('2',1.2,0)]),component('B',10,3,2,1,False,[pin('1',-1.2,0),pin('2',1.2,0)])]
    crossed['nets']=[net('A_IN','L1','1','A','1',0.3),net('A_OUT','A','2','R1','1',0.3),net('B_IN','L2','1','B','1',0.3),net('B_OUT','B','2','R2','1',0.3)]
    dual=json.loads((ROOT/'benchmarks/imported/layout-trace/dual-esp32-benchmark.json').read_text())
    return {'movable-wall':wall,'crossed-resistors':crossed,'dual-esp32':dual}

def poses_of(problem):
    return [{'component':c['id'],'position':c['position'],'rotation_degrees':c.get('rotation_degrees',0)} for c in problem['components']]
def xy(p): return (p['x'],p['y'])
def transform(offset,pose):
    a=math.radians(pose['rotation_degrees']);x,y=xy(offset);u,v=xy(pose['position'])
    return (u+x*math.cos(a)-y*math.sin(a),v+x*math.sin(a)+y*math.cos(a))
def polygon(c,pose):
    w,h=xy(c['size'])
    return [transform(point(x*w/2,y*h/2),pose) for x,y in [(-1,-1),(1,-1),(1,1),(-1,1)]]
def sat_overlap(a,b):
    for poly in [a,b]:
        for p,q in zip(poly,poly[1:]+poly[:1]):
            axis=(p[1]-q[1],q[0]-p[0]);aa=[x*axis[0]+y*axis[1] for x,y in a];bb=[x*axis[0]+y*axis[1] for x,y in b]
            if max(aa)<=min(bb)+1e-7 or max(bb)<=min(aa)+1e-7:return False
    return True
def independent_pose_check(problem,poses):
    by={p['component']:p for p in poses}; comps=problem['components'];bounds=problem['board']['bounds'];issues=[]
    polys={c['id']:polygon(c,by[c['id']]) for c in comps}
    for c in comps:
        p=by[c['id']]; movement=c.get('constraints',{}).get('movement','free')
        if movement=='fixed' and math.dist(xy(c['position']),xy(p['position']))>1e-6: issues.append('moved fixed '+c['id'])
        if movement=='vertical' and abs(c['position']['x']-p['position']['x'])>1e-6:issues.append('vertical '+c['id'])
        if movement=='horizontal' and abs(c['position']['y']-p['position']['y'])>1e-6:issues.append('horizontal '+c['id'])
        if c.get('constraints',{}).get('rotation')=='fixed' and abs((p['rotation_degrees']-c.get('rotation_degrees',0)+180)%360-180)>1e-6:issues.append('rotated fixed '+c['id'])
        for x,y in polys[c['id']]:
            if not bounds['min']['x']-1e-6<=x<=bounds['max']['x']+1e-6 or not bounds['min']['y']-1e-6<=y<=bounds['max']['y']+1e-6:issues.append('outside '+c['id']);break
    for i,c in enumerate(comps):
        for d in comps[i+1:]:
            if sat_overlap(polys[c['id']],polys[d['id']]):issues.append('body overlap '+c['id']+'/'+d['id'])
    return {'passed':not issues,'issues':issues,'scope':'Independent oriented full-body SAT, board containment and movement/rotation locks. Production projector additionally checks pad-extended envelopes and represented relational constraints.'}

def point_segment(p,a,b):
    dx=b[0]-a[0];dy=b[1]-a[1];den=dx*dx+dy*dy
    t=max(0,min(1,((p[0]-a[0])*dx+(p[1]-a[1])*dy)/den)) if den else 0
    return math.hypot(p[0]-a[0]-t*dx,p[1]-a[1]-t*dy)
def orient(a,b,c):return (b[0]-a[0])*(c[1]-a[1])-(b[1]-a[1])*(c[0]-a[0])
def seg_distance(a,b,c,d):
    if orient(a,b,c)*orient(a,b,d)<0 and orient(c,d,a)*orient(c,d,b)<0:return 0
    return min(point_segment(a,c,d),point_segment(b,c,d),point_segment(c,a,b),point_segment(d,a,b))
def segment_rect(a,b,rect):
    # Slab intersection against the closed inflated axis-aligned rectangle.
    lo,hi=0.0,1.0
    for axis in range(2):
        delta=b[axis]-a[axis]
        if abs(delta)<1e-12:
            if a[axis]<rect[axis] or a[axis]>rect[axis+2]:return False
        else:
            u=(rect[axis]-a[axis])/delta;v=(rect[axis+2]-a[axis])/delta
            lo=max(lo,min(u,v));hi=min(hi,max(u,v))
            if lo>hi:return False
    return True

def route_probe(problem,poses):
    """Four-neighbor, single-layer, sequential A*: exact continuous edge checks.

    Only called on authored micro fixtures with circular pads, axis-aligned
    walls and no other keepout/rule constructs. Full boards use no toy routing.
    """
    by={p['component']:p for p in poses};pitch=0.2; clearance=problem['rules']['clearance']; allpads={};walls=[]
    for c in problem['components']:
        assert abs(by[c['id']]['rotation_degrees'])<1e-8
        for pin_ in c['pins']:
            assert len(pin_['pads'])==1 and pin_['pads'][0]['shape']['kind']=='circle'
            allpads[(c['id'],pin_['id'])]=(transform(pin_['offset'],by[c['id']]),pin_['pads'][0]['shape']['diameter']/2)
        if c.get('body_is_routing_keepout'):
            x,y=xy(by[c['id']]['position']);w,h=xy(c['size']);walls.append((x-w/2,y-h/2,x+w/2,y+h/2))
    maxx=problem['board']['bounds']['max']['x'];maxy=problem['board']['bounds']['max']['y'];previous=[];routes=[];expansions=0
    def allowed(a,b,width,own,prior):
        radius=clearance+width/2
        if any(x<radius-1e-8 or x>maxx-radius+1e-8 or y<radius-1e-8 or y>maxy-radius+1e-8 for x,y in [a,b]):return False
        for rect in walls:
            expanded=(rect[0]-radius,rect[1]-radius,rect[2]+radius,rect[3]+radius)
            if segment_rect(a,b,expanded):return False
        for key,(p,r) in allpads.items():
            if key not in own and point_segment(p,a,b)<r+radius-1e-8:return False
        for p,q,w in prior:
            if seg_distance(a,b,p,q)<clearance+(width+w)/2-1e-8:return False
        return True
    for net_ in problem['nets']:
        startkey=(net_['from']['component'],net_['from']['pin']);endkey=(net_['to']['component'],net_['to']['pin']);own={startkey,endkey};start=allpads[startkey][0];end=allpads[endkey][0];width=net_['width']
        s=tuple(round(v/pitch) for v in start);t=tuple(round(v/pitch) for v in end);pos=lambda k:(k[0]*pitch,k[1]*pitch)
        queue=[(math.dist(start,end),0,s)];dist={s:0};parents={};found=False
        # Preserve exact physical pin centers through checked short access edges.
        if not allowed(start,pos(s),width,own,previous) or not allowed(pos(t),end,width,own,previous):queue=[]
        while queue:
            _,g,k=heapq.heappop(queue)
            if g>dist[k]+1e-10:continue
            expansions+=1
            if k==t:found=True;break
            for dx,dy in [(1,0),(-1,0),(0,1),(0,-1)]:
                q=(k[0]+dx,k[1]+dy)
                if not allowed(pos(k),pos(q),width,own,previous):continue
                ng=g+pitch
                if ng<dist.get(q,float('inf'))-1e-10:
                    dist[q]=ng;parents[q]=k;heapq.heappush(queue,(ng+math.dist(pos(q),end),ng,q))
        path=[]
        if found:
            k=t;path=[pos(k)]
            while k!=s:k=parents[k];path.append(pos(k))
            path=list(reversed(path));path=[start]+path+[end]
            # Remove collinear grid vertices only. Keep exact pad access edges.
            compact=[]
            for p in path:
                if compact and math.dist(compact[-1],p)<1e-9:continue
                while len(compact)>1 and abs(orient(compact[-2],compact[-1],p))<1e-9 and point_segment(compact[-1],compact[-2],p)<1e-9:compact.pop()
                compact.append(p)
            path=compact
            # Independent replay after search: every output edge is continuous-legal.
            assert all(allowed(a,b,width,own,previous) for a,b in zip(path,path[1:]))
            previous += [(a,b,width) for a,b in zip(path,path[1:])]
        routes.append({'net':net_['id'],'width':width,'found':found,'path':path})
    return {'scope':'Toy single-layer exact-edge route probe; no vias, KiCad rules or native DRC. Fixed net order; failure is algorithmic, not an unroutability proof.', 'completed':sum(r['found'] for r in routes),'total':len(routes),'length_mm':sum(math.dist(a,b) for r in routes for a,b in zip(r['path'],r['path'][1:])),'expansions':expansions,'routes':routes,'continuous_output_validation_passed':True}

def svg(problem,poses,title,routing=None,junction=False):
    b=problem['board']['bounds'];x0,y0=xy(b['min']);x1,y1=xy(b['max']);w=x1-x0;h=y1-y0;by={p['component']:p for p in poses};font=max(w/100,0.18)
    parts=[f'<svg xmlns="http://www.w3.org/2000/svg" viewBox="{x0-1} {y0-2} {w+2} {h+3}" width="1100" height="780"><rect x="{x0-1}" y="{y0-2}" width="{w+2}" height="{h+3}" fill="#101923"/><text x="{x0}" y="{y0-0.7}" fill="white" font-size="{font*1.4}">{html.escape(title)}</text><rect x="{x0}" y="{y0}" width="{w}" height="{h}" fill="#18352d" stroke="#82968c" stroke-width=".07"/>']
    pins={}
    for c in problem['components']:
        pose=by[c['id']];fixed=c.get('constraints',{}).get('movement')=='fixed';p=polygon(c,pose)
        is_junction = c['id'] in junction if isinstance(junction, set) else junction and not fixed
        if is_junction:
            cx,cy=xy(pose['position']);parts.append(f'<circle cx="{cx}" cy="{cy}" r=".13" fill="#ffee66"/>')
        else:
            parts.append(f'<polygon points="{" ".join(f"{x},{y}" for x,y in p)}" fill="{"#728391" if fixed else "#53a8b6"}" fill-opacity=".45" stroke="#b7d4de" stroke-width=".06"/>')
        cx,cy=xy(pose['position']);parts.append(f'<text x="{cx}" y="{cy}" fill="white" font-size="{font}" text-anchor="middle">{html.escape(c["id"])}</text>')
        for pin_ in c['pins']:
            q=xy(pose['position']) if is_junction else transform(pin_['offset'],pose);pins[(c['id'],pin_['id'])]=q
            if is_junction: continue
            for pad in pin_.get('pads',[]):
                center=transform(pad.get('local_center',pin_['offset']),pose);shape=pad['shape']
                if shape['kind']=='circle':parts.append(f'<circle cx="{center[0]}" cy="{center[1]}" r="{shape["diameter"]/2}" fill="#d6b966"/>')
                else:
                    pw,ph=xy(shape['size']);angle=pose['rotation_degrees']+shape.get('rotation_degrees',0)
                    parts.append(f'<rect x="{-pw/2}" y="{-ph/2}" width="{pw}" height="{ph}" transform="translate({center[0]} {center[1]}) rotate({angle})" fill="#d6b966"/>')
    for n in problem.get('nets',[]):
        a=pins[(n['from']['component'],n['from']['pin'])];bb=pins[(n['to']['component'],n['to']['pin'])];parts.append(f'<path d="M {a[0]} {a[1]} L {bb[0]} {bb[1]}" fill="none" stroke="#e5eaff" stroke-opacity=".25" stroke-width=".045"/>')
    if routing:
        for i,r in enumerate(routing['routes']):
            if r['path']:parts.append(f'<polyline points="{" ".join(f"{x},{y}" for x,y in r["path"])}" fill="none" stroke="{["#ffac52","#ff7095","#70cfff","#a7ed76"][i%4]}" stroke-width="{r["width"]}" stroke-linecap="round" stroke-linejoin="round"/>')
    return ''.join(parts)+'</svg>'

def run():
    OUT.mkdir(parents=True,exist_ok=True);problems=fixtures();jobs=[]
    harmonic={'kind':'harmonic_ports','iterations':128,'legalization_sweeps':256,'maximum_pair_checks':1000000,'connected_pair_spacing_floor':False}
    for name,p in problems.items():
        (OUT/f'{name}.problem.json').write_text(json.dumps(p,indent=2)+'\n')
        configs=[('retained',{'kind':'declared'},0,False),('barycentric',{'kind':'connectivity_barycentric','iterations':128,'attraction':0.2},0,False),('harmonic-full-size',harmonic,0,False),('junction-reinsert',harmonic,0,True)]
        configs += [(f'random-{s:02}',{'kind':'random','attempts':1},s,False) for s in range(16)]
        configs += [(f'junction-random-{s:02}',dict(harmonic,underanchored_seed={'kind':'random','attempts':1}),s,True) for s in range(8)]
        for label,policy,seed,junction in configs:
            jobs.append({'fixture':name,'label':label,'problem':p,'config':{'policy':policy,'seed':seed,'projection_sweeps':512},'junction':junction})
        if name=='movable-wall':
            # A separate directed intervention; not smuggled into the generic policies.
            for sign in [-1,1]:
                pp=copy.deepcopy(p);pp['components'][-1]['position']['y']+=sign*0.6
                jobs.append({'fixture':name,'label':f'pressure-shift-{sign:+d}','problem':pp,'config':{'policy':{'kind':'declared'},'seed':0,'projection_sweeps':512},'junction':False})
    (OUT/'requests.jsonl').write_text(''.join(json.dumps(j)+'\n' for j in jobs))
    started=time.time()
    process=subprocess.run([str(EXE)],input=(OUT/'requests.jsonl').read_text(),capture_output=True,text=True,check=True)
    (OUT/'raw-results.jsonl').write_text(process.stdout);results=[json.loads(s) for s in process.stdout.splitlines()];assert len(results)==len(jobs)
    records=[];frames=[]
    for j,r in zip(jobs,results):
        name=j['fixture'];label=j['label'];p=j['problem'];record={'fixture':name,'label':label,'config':j['config'],'junction_experiment':j['junction'],**r};poses=r['result']['poses'] if r['ok'] else poses_of(p)
        if r['ok']:
            record['independent_pose_validation']=independent_pose_check(p,poses);assert record['independent_pose_validation']['passed'],record
            if name!='dual-esp32':record['route_probe']=route_probe(p,poses)
        title=f'{name} / {label}: '+('legal full-size placement' if r['ok'] else 'rejected; displaying input')
        if 'route_probe' in record:title+=f' / routed {record["route_probe"]["completed"]}/{record["route_probe"]["total"]}'
        filename=f'{name}--{label}.svg';drawing=svg(p,poses,title,record.get('route_probe'));(OUT/filename).write_text(drawing);record['render']=filename
        if r['junctions']:
            jfilename=f'{name}--{label}--junctions.svg';jdrawing=svg(p,r['junctions'],f'{name} / {label}: reduced junction seed, NOT full-size legal',junction=True);(OUT/jfilename).write_text(jdrawing);record['junction_render']=jfilename;frames.append({'label':f'{name} / {label} / reduced seed','svg':jdrawing})
        frames.append({'label':title,'svg':drawing});records.append(record)
        print(title,flush=True)
    summary={'scope':'Production pcb-placement policies and production full-size projector; experimental shrink-to-junction adapter. Independent micro-fixture route probe only. Real 43-component fixture scores placement legality and congestion, not routing. Each random policy has one proposal per fixed seed; failed proposals retained. No native KiCad claim.', 'executable_sha256':hashlib.sha256(EXE.read_bytes()).hexdigest(),'wall_seconds':time.time()-started,'records':records}
    (OUT/'results.json').write_text(json.dumps(summary,indent=2)+'\n')
    document='''<!doctype html><meta charset="utf-8"><title>Placement exploration</title><style>body{background:#101923;color:#eee;font:16px system-ui;margin:20px}button,select,input{font:inherit}#board svg{max-width:100%;height:74vh}#board{display:flex;justify-content:center}</style><h1>Placement proposal exploration</h1><p>Actual production placement policies. Animation cycles independent proposals and reduced junction seeds; it is not a spring simulation or a continuous feasible motion. Failed proposals show their labeled input. Orange/colored paths are the bounded single-layer probe, not KiCad verification.</p><button id="play">Play proposals</button> <select id="select"></select> <input id="slider" type="range" min="0" value="0"><p id="caption"></p><div id="board"></div><script>const frames=FRAMES;let index=0,timer=null;const select=document.getElementById('select'),slider=document.getElementById('slider');frames.forEach((f,i)=>select.add(new Option(f.label,i)));slider.max=frames.length-1;function show(i){index=Number(i);select.value=index;slider.value=index;document.getElementById('board').innerHTML=frames[index].svg;document.getElementById('caption').textContent=(index+1)+' / '+frames.length+' — '+frames[index].label;}select.onchange=()=>show(select.value);slider.oninput=()=>show(slider.value);document.getElementById('play').onclick=()=>{if(timer){clearInterval(timer);timer=null;}else timer=setInterval(()=>show((index+1)%frames.length),900);};show(0);</script>'''.replace('FRAMES',json.dumps(frames).replace('</','<\\/'))
    (OUT/'index.html').write_text(document)

if __name__=='__main__':run()
