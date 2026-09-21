#!/usr/bin/env python3
# Copyright (C) 2026 Toit contributors.
"""Run the actual Rust engine; independently check and render every trajectory."""
import argparse
import hashlib
import html
import json
import math
from pathlib import Path
import subprocess

HERE = Path(__file__).resolve().parent
ROOT = HERE.parent.parent


def xy(p):
    return p['x'], p['y']


def point_distance(p, a, b):
    d = (b[0]-a[0], b[1]-a[1])
    denom = d[0]*d[0]+d[1]*d[1]
    t = max(0, min(1, ((p[0]-a[0])*d[0]+(p[1]-a[1])*d[1])/denom)) if denom else 0
    return math.dist(p, (a[0]+t*d[0], a[1]+t*d[1]))


def cross(a, b, c):
    return (b[0]-a[0])*(c[1]-a[1])-(b[1]-a[1])*(c[0]-a[0])


def segment_distance(a, b, c, d):
    # Strict crossing, with collinear/touching cases handled by endpoint distances.
    if cross(a,b,c)*cross(a,b,d)<0 and cross(c,d,a)*cross(c,d,b)<0:
        return 0.0
    return min(point_distance(a,c,d), point_distance(b,c,d), point_distance(c,a,b), point_distance(d,a,b))


def corners(body):
    x,y = xy(body['position']); w,h = xy(body['half_size']); a=body['angle_radians']
    return [(x+dx*math.cos(a)-dy*math.sin(a), y+dx*math.sin(a)+dy*math.cos(a)) for dx,dy in [(-w,-h),(w,-h),(w,h),(-w,h)]]


def body_distance(a,b,body):
    center = xy(body['position']); angle=body['angle_radians']; w,h=xy(body['half_size'])
    for p in [a,b]:
        dx,dy=p[0]-center[0],p[1]-center[1]
        if abs(dx*math.cos(angle)+dy*math.sin(angle))<=w and abs(-dx*math.sin(angle)+dy*math.cos(angle))<=h:
            return 0.0
    pts=corners(body)
    return min(segment_distance(a,b,c,d) for c,d in zip(pts,pts[1:]+pts[:1]))


def assess(doc, state):
    frame=state['frame']; routes={r['id']:[xy(p) for p in r['points']] for r in state['routes']}
    specs={r['id']:r for r in doc['routes']}; bodies={b['label']:b for b in frame['bodies']}
    errors=[]; margin=math.inf
    for pair in doc['body_clearances']:
        points=routes[pair['route']]
        required=specs[pair['route']]['width']/2+pair['clearance']
        distance=min(body_distance(a,b,bodies[pair['body']]) for a,b in zip(points,points[1:]))
        margin=min(margin,distance-required)
    for pair in doc['trace_clearances']:
        aa,bb=routes[pair['first']],routes[pair['second']]
        required=(specs[pair['first']]['width']+specs[pair['second']]['width'])/2+pair['clearance']
        distance=min(segment_distance(a,b,c,d) for a,b in zip(aa,aa[1:]) for c,d in zip(bb,bb[1:]))
        margin=min(margin,distance-required)
    if margin < -1e-4: errors.append('clearance')
    fixed_drift=0.0
    for name,spec in specs.items():
        initial=[xy(p) for p in spec['initial_points']]
        indices=range(len(initial)) if spec['mobility']=='Fixed' else ([0,len(initial)-1] if spec['mobility']=='Interior' else [])
        for i in indices: fixed_drift=max(fixed_drift,math.dist(routes[name][i],initial[i]))
        for x,y in routes[name]:
            radius=spec['width']/2
            if min(x-radius,y-radius,30-x-radius,20-y-radius)<-1e-4: errors.append('board edge')
    for spec in doc['bodies']:
        b=bodies[spec['id']]
        if not spec['mobility']['translate_x'] and not spec['mobility']['translate_y']:
            fixed_drift=max(fixed_drift,math.dist(xy(b['position']),xy(spec['initial_position'])))
        if not spec['mobility']['rotate']:
            fixed_drift=max(fixed_drift,abs(b['angle_radians']-spec['initial_angle']))
        if any(min(x,y,30-x,20-y)<-1e-4 for x,y in corners(b)): errors.append('body outside')
    attachment_error=0.0
    for a in doc['attachments']:
        b=bodies[a['body']]; x,y=xy(b['position']); dx,dy=xy(a['local_position']); angle=b['angle_radians']
        target=(x+dx*math.cos(angle)-dy*math.sin(angle),y+dx*math.sin(angle)+dy*math.cos(angle))
        attachment_error=max(attachment_error,math.dist(target,routes[a['route']][a['point']]))
    if attachment_error>1e-4: errors.append('attachment')
    for group in doc['shared_points']:
        members=[routes[m['route']][m['point']] for m in group['members']]
        if any(math.dist(members[0],p)>1e-5 for p in members[1:]): errors.append('shared endpoint detached')
    if fixed_drift>1e-5: errors.append('fixed geometry moved')
    return dict(valid=not errors,errors=sorted(set(errors)),clearance_margin_mm=None if math.isinf(margin) else margin,
                fixed_drift_mm=fixed_drift,attachment_error_mm=attachment_error)


COLORS=['#22aadd','#f09b45','#4ac98d','#ba95ee']


def svg(doc,state):
    scale=28; ox=35; oy=110
    def p(v): return f'{ox+v[0]*scale:.3f},{oy+v[1]*scale:.3f}'
    f=state['frame']; check=state['independent_check']
    out=[f'<svg xmlns="http://www.w3.org/2000/svg" width="920" height="740" viewBox="0 0 920 740"><rect width="920" height="740" fill="#111c2a"/>',
         f'<g font-family="sans-serif" fill="#e3edf6"><text x="35" y="35" font-size="23">{html.escape(doc["id"])}</text>',
         f'<text x="35" y="66" font-size="16">Step {f["step"]} · length {state["length_mm"]:.3f} mm · {"geometry passes" if check["valid"] else "geometry fails"}</text>',
         '<text x="35" y="91" font-size="13">Actual CPU engine · gold: tension · red: correction · arrows × 12</text></g>',
         '<rect x="35" y="110" width="840" height="560" fill="#1b2a38" stroke="#647b8c"/>']
    for b in f['bodies']:
        out.append(f'<polygon points="{" ".join(p(v) for v in corners(b))}" fill="#596679" stroke="#c6a8ee" stroke-width="2"/>')
    for i,r in enumerate(state['routes']):
        width=doc['routes'][i]['width']*scale
        out.append(f'<polyline points="{" ".join(p(xy(v)) for v in r["points"])}" fill="none" stroke="{COLORS[i%4]}" stroke-width="{width}" stroke-linecap="round" stroke-linejoin="round"/>')
    for v in f['vectors']:
        if math.hypot(*xy(v['vector']))<1e-6: continue
        origin=xy(v['origin']); dx,dy=xy(v['vector']); end=(origin[0]+dx*12,origin[1]+dy*12)
        color={'trace_tension':'#ffd15d','constraint_correction':'#ff6575','density_field':'#b291ff'}[v['source']]
        out.append(f'<polyline points="{p(origin)} {p(end)}" fill="none" stroke="{color}" stroke-width="2"/>')
    for particle in f['particles']:
        x,y=xy(particle['position']); color='#ffffff' if particle['inverse_mass']==0 else '#94d7ec'
        out.append(f'<circle cx="{ox+x*scale}" cy="{oy+y*scale}" r="3" fill="{color}"/>')
    out.append('<text x="35" y="707" font-family="sans-serif" font-size="13" fill="#c2d0dd">Reduced single-layer fixture; geometric checks, no KiCad admission.</text></svg>')
    return ''.join(out)


def render(directory):
    summaries=[]
    template=(HERE/'viewer.html').read_text()
    for path in sorted(directory.glob('*.json')):
        if path.name in ['summary.json','provenance.json']: continue
        doc=json.loads(path.read_text())
        for state in doc['frames']: state['independent_check']=assess(doc,state)
        doc['summary']={
            'id':doc['id'],'initial_length_mm':doc['frames'][0]['length_mm'],
            'final_length_mm':doc['frames'][-1]['length_mm'],
            'initial_check':doc['frames'][0]['independent_check'],'final_check':doc['frames'][-1]['independent_check'],
            'invalid_frames':sum(not s['independent_check']['valid'] for s in doc['frames']),
            'max_fixed_drift_mm':max(s['independent_check']['fixed_drift_mm'] for s in doc['frames']),
            'last_step_motion_mm':doc['frames'][-1]['frame']['metrics']['max_displacement'],
            'final_body_poses':doc['frames'][-1]['frame']['bodies'],
            'max_pressure':max(s['frame']['metrics']['max_field_pressure'] for s in doc['frames'])}
        summaries.append(doc['summary'])
        path.write_text(json.dumps(doc,separators=(',',':')))
        (directory/(doc['id']+'.html')).write_text(template.replace('__DATA__',json.dumps(doc,separators=(',',':')).replace('</','<\\/')))
        for index in [0,1,len(doc['frames'])//2,len(doc['frames'])-1]:
            (directory/f'{doc["id"]}-step-{index:04}.svg').write_text(svg(doc,doc['frames'][index]))
        # A compact animated image is also usable without JavaScript or a server.
        from PIL import Image
        import io
        frames=[]
        indices=sorted(set([0,1,2,3,5,10]+list(range(0,len(doc['frames']),max(1,len(doc['frames'])//45)))+[len(doc['frames'])-1]))
        for index in indices:
            png=subprocess.run(['rsvg-convert','--width','740'],input=svg(doc,doc['frames'][index]).encode(),stdout=subprocess.PIPE,check=True).stdout
            frames.append(Image.open(io.BytesIO(png)).convert('RGB'))
        frames[0].save(directory/(doc['id']+'.gif'),save_all=True,append_images=frames[1:],duration=[700]+[90]*(len(frames)-2)+[1500],loop=0)
    (directory/'summary.json').write_text(json.dumps(summaries,indent=2))
    links=''.join(f'<li><a href="{s["id"]}.html">{s["id"]}</a> · <a href="{s["id"]}.gif">animation</a> · {s["initial_length_mm"]:.3f} → {s["final_length_mm"]:.3f} mm</li>' for s in summaries)
    (directory/'index.html').write_text('<!doctype html><meta charset="utf-8"><title>Actual engine experiments</title><style>body{max-width:950px;margin:50px auto;font:18px system-ui;background:#111c2a;color:#dbe9f4}a{color:#72cdff}li{margin:20px}</style><h1>Actual engine playback</h1><p>Retained CPU solver iterations. Play or scrub each experiment; arrows distinguish tension, field proposals and constraint corrections. No interpolated component motion.</p><ul>'+links+'</ul>')
    print(json.dumps([{k:v for k,v in s.items() if k!='final_body_poses'} for s in summaries],indent=2))


def main():
    parser=argparse.ArgumentParser(); parser.add_argument('--output',type=Path,default=ROOT/'build/algorithm-exploration-2026-09-07/engine'); parser.add_argument('--render-only',action='store_true'); parser.add_argument('--settled-only',action='store_true'); args=parser.parse_args()
    args.output.mkdir(parents=True,exist_ok=True)
    if not args.render_only:
        subprocess.run(['cargo','run','--release','--offline','--manifest-path',str(HERE/'Cargo.toml'),'--target-dir',str(ROOT/'target/algorithm-exploration'),'--',str(args.output)]+(['--settled-only'] if args.settled_only else []),cwd=ROOT,check=True)
    render(args.output)
    files=list((ROOT/'crates/pcb-engine/src').glob('*.rs'))+list(HERE.glob('*.py'))+list((HERE/'src').glob('*.rs'))+[HERE/'viewer.html',HERE/'Cargo.toml']
    (args.output/'provenance.json').write_text(json.dumps({str(p.relative_to(ROOT)):hashlib.sha256(p.read_bytes()).hexdigest() for p in files},indent=2))


if __name__=='__main__': main()
