#!/usr/bin/env python3
# Copyright (C) 2026 Toit contributors.
"""Run placement with sweep diagnostics and always retain a playback and final SVG.

This visualizes the model's bodies/pads and pair corrections, not native KiCad
DRC. The final frame may be rejected. No routing or area credit is implied.
"""

import argparse
from collections import Counter
import hashlib
import html
import json
import math
from pathlib import Path
import shutil
import subprocess
import time


def sha(path):
    return hashlib.sha256(path.read_bytes()).hexdigest()


def shape_svg(shape):
    kind = shape['kind']
    if kind == 'rect':
        w, h = shape['size']['x'], shape['size']['y']
        return f'<rect x="{-w/2}" y="{-h/2}" width="{w}" height="{h}"/>'
    if kind == 'circle':
        radius = shape['diameter']/2 if 'diameter' in shape else shape['radius']
        return f'<circle r="{radius}"/>'
    raise ValueError(f'Unsupported diagnostic shape: {kind}')


def component_svg(component):
    geometry = component.get('placement_geometry', {})
    parts = geometry.get('body_parts', [])
    if parts:
        body = ''.join('<polygon stroke="none" points="' + ' '.join(f'{v["x"]},{v["y"]}' for v in part['vertices']) + '"/>' for part in parts)
        edges = Counter()
        for part in parts:
            vertices = [(v['x'], v['y']) for v in part['vertices']]
            for a, b in zip(vertices, vertices[1:] + vertices[:1]):
                edges[tuple(sorted((a, b)))] += 1
        body += ''.join(f'<line x1="{a[0]}" y1="{a[1]}" x2="{b[0]}" y2="{b[1]}"/>' for (a, b), count in edges.items() if count == 1)
    else:
        body = shape_svg(geometry.get('body', {'kind': 'rect', 'size': component['size']}))
    color = '#156fa3' if geometry.get('body_layers') != ['bottom'] else '#be6626'
    out = [f'<g fill="{color}" fill-opacity=".16" stroke="{color}" stroke-width=".12">{body}</g>']
    for pin in component['pins']:
        for pad in pin.get('pads', []):
            center = pad.get('local_center', pin['offset'])
            color = '#b72d3b' if pad['layer'] == 'top' else '#3064bf'
            out.append(f'<g transform="translate({center["x"]} {center["y"]})" fill="{color}" fill-opacity=".35">{shape_svg(pad["shape"])}</g>')
    out.append(f'<text text-anchor="middle" font-size="1.2" fill="#111">{html.escape(component["id"])}</text>')
    return ''.join(out)


def render(problem, trace, output):
    frames = trace['frames']
    assert frames, 'No harmonic frames captured; inspect trace.json and run.log'
    components = {c['id']: c for c in problem['components']}
    glyphs = {ref: component_svg(c) for ref, c in components.items()}
    bounds = problem['board']['bounds']
    xs = [bounds['min']['x'], bounds['max']['x']]
    ys = [bounds['min']['y'], bounds['max']['y']]
    for frame in frames:
        for pose in frame['poses']:
            c = components[pose['component']]
            radius = math.hypot(c['size']['x'], c['size']['y']) / 2
            xs.extend([pose['position']['x'] - radius, pose['position']['x'] + radius])
            ys.extend([pose['position']['y'] - radius, pose['position']['y'] + radius])
    viewbox = f'{min(xs)-2} {min(ys)-2} {max(xs)-min(xs)+4} {max(ys)-min(ys)+4}'
    board = f'<rect x="{bounds["min"]["x"]}" y="{bounds["min"]["y"]}" width="{bounds["max"]["x"]-bounds["min"]["x"]}" height="{bounds["max"]["y"]-bounds["min"]["y"]}" fill="#f1f6f1" stroke="#333" stroke-width=".2"/>'
    def scene(frame):
        items = [board]
        by = {p['component']: p for p in frame['poses']}
        for p in frame['poses']:
            items.append(f'<g transform="translate({p["position"]["x"]} {p["position"]["y"]}) rotate({p["rotation_degrees"]})">{glyphs[p["component"]]}</g>')
        for contact in frame['contacts']:
            a, b = by[contact['first']]['position'], by[contact['second']]['position']
            items.append(f'<line x1="{a["x"]}" y1="{a["y"]}" x2="{b["x"]}" y2="{b["y"]}" stroke="#dc162e" stroke-width=".3"/>')
        return ''.join(items)
    for name, frame in [('initial', frames[0]), ('final', frames[-1])]:
        (output/f'{name}.svg').write_text(f'<svg xmlns="http://www.w3.org/2000/svg" viewBox="{viewbox}" width="1100" height="850" style="background:white">{scene(frame)}</svg>')
    data = json.dumps({'frames': frames, 'glyphs': glyphs, 'board': board, 'error': trace['error']}).replace('<', '\\u003c')
    document = '''<!doctype html><meta charset="utf-8"><title>Placement legalization playback</title>
<style>body{font:16px system-ui;margin:1.5rem;background:#fafafa}button,input{font:inherit}#layout{width:100%;height:68vh}#plot{width:100%;height:100px;border:1px solid #aaa}pre{white-space:pre-wrap}#slider{width:65%}td,th{text-align:left;padding:.15rem 1rem}#details{max-height:22rem;overflow:auto}</style>
<h1>Placement legalization</h1><p>Blue: front bodies; orange: back bodies. Pads from both layers are shown together. Red lines identify remaining body/pad conflicts; the optional connected-pair spacing floor and relational constraints are excluded. These are model diagnostics, not native DRC. Only harmonic seed/legalization phases are traced.</p>
<pre id="outcome"></pre><button id="play">Play</button> <input id="slider" type="range" min="0" step="1" value="0"> <span id="label"></span>
<svg id="plot" viewBox="0 0 1000 100" preserveAspectRatio="none"></svg><small>Maximum remaining pair correction per saved frame; click to seek.</small>
<svg id="layout" viewBox="VIEWBOX"></svg><p id="stats"></p><div id="details"></div>
<p><a href="trace.json">Full trace</a> · <a href="summary.json">Diagnostic summary</a> · <a href="final.svg">Final frame SVG</a></p>
<script id="data" type="application/json">DATA</script><script>
const D=JSON.parse(document.getElementById('data').textContent), F=D.frames;
const slider=document.getElementById('slider'), layout=document.getElementById('layout');
const esc=s=>String(s).replaceAll('&','&amp;').replaceAll('<','&lt;').replaceAll('"','&quot;');
slider.max=F.length-1;
document.getElementById('outcome').textContent=D.error||'Placement accepted by model checks; native verification and routing remain separate.';
const peaks=F.map(f=>Math.max(0,...f.contacts.map(c=>c.residual_mm))), peak=Math.max(1e-9,...peaks);
const poly=peaks.map((v,i)=>`${i*1000/Math.max(1,F.length-1)},${98-v/peak*94}`).join(' ');
function draw(){const i=+slider.value,f=F[i],by=Object.fromEntries(f.poses.map(p=>[p.component,p]));
layout.innerHTML=D.board+f.poses.map(p=>`<g transform="translate(${p.position.x} ${p.position.y}) rotate(${p.rotation_degrees})">${D.glyphs[p.component]}</g>`).join('')+f.contacts.map(c=>{let a=by[c.first].position,b=by[c.second].position;return `<line x1="${a.x}" y1="${a.y}" x2="${b.x}" y2="${b.y}" stroke="#dc162e" stroke-width=".3"><title>${esc(c.first)} / ${esc(c.second)}: ${c.residual_mm.toFixed(6)} mm</title></line>`}).join('');
document.getElementById('label').textContent=`Attempt ${f.attempt}, sweep ${f.sweep}`+(f.seed_projection?` · ${f.seed_projection} (${(f.seed_projection_seconds||0).toFixed(3)} s, ${f.pair_checks} shared pair-work checks)`+(f.seed_projection_broad_phase?`; ${f.seed_projection_broad_phase.comparisons} broadphase comparisons, ${f.seed_projection_broad_phase.rejected_disjoint} disjoint rejections; ${f.seed_projection_broad_phase.interval_preparations||0} interval preparations, ${f.seed_projection_broad_phase.row_certificates||0} row certificates, ${f.seed_projection_broad_phase.skipped_samples||0} skipped samples`:""):"")+(f.coupled?` · contact branch ${f.coupled.branch_trial||0}, coupled round ${f.coupled.round}: ${f.coupled.status}`:"");
document.getElementById('stats').textContent=`${f.contacts.length} remaining contacts; maximum correction ${peaks[i].toFixed(6)} mm; maximum encountered during sweep ${f.maximum_encountered_residual_mm.toFixed(6)} mm`+(f.coupled?`; ${f.coupled.linear_solves} linear solves, ${f.coupled.active_rows} active constraints`:"");
document.getElementById('details').innerHTML='<table><tr><th>Pair</th><th>Correction (mm)</th></tr>'+[...f.contacts].sort((a,b)=>b.residual_mm-a.residual_mm).map(c=>`<tr><td>${esc(c.first)} / ${esc(c.second)}</td><td>${c.residual_mm.toFixed(6)}</td></tr>`).join('')+'</table>';
document.getElementById('plot').innerHTML=`<polyline points="${poly}" fill="none" stroke="#c32" stroke-width="2"/><line x1="${i*1000/Math.max(1,F.length-1)}" y1="0" x2="${i*1000/Math.max(1,F.length-1)}" y2="100" stroke="#168"/>`;
}
let timer=null;document.getElementById('play').onclick=()=>{if(timer){clearInterval(timer);timer=null;document.getElementById('play').textContent='Play';}else{document.getElementById('play').textContent='Pause';timer=setInterval(()=>{slider.value=(+slider.value+1)%F.length;draw()},70)}};
slider.oninput=draw;document.getElementById('plot').onclick=e=>{let b=e.currentTarget.getBoundingClientRect();slider.value=Math.round((e.clientX-b.left)/b.width*(F.length-1));draw()};draw();
</script>'''.replace('VIEWBOX', viewbox).replace('DATA', data)
    (output/'index.html').write_text(document)


def summarize(trace):
    frames = trace['frames']
    if not frames:
        return {'error': trace['error'], 'frames': 0}
    last = frames[-1]
    tail = [f for f in frames if f['attempt'] == last['attempt']][-64:]
    pairs = {}
    for f in tail:
        for c in f['contacts']:
            key = c['first'] + '/' + c['second']
            entry = pairs.setdefault(key, {'frames': 0, 'min_mm': c['residual_mm'], 'max_mm': 0})
            entry['frames'] += 1
            entry['min_mm'] = min(entry['min_mm'], c['residual_mm'])
            entry['max_mm'] = max(entry['max_mm'], c['residual_mm'])
    motion = []
    for a, b in zip(tail, tail[1:]):
        old = {p['component']: p['position'] for p in a['poses']}
        motion.append(max(math.hypot(p['position']['x']-old[p['component']]['x'], p['position']['y']-old[p['component']]['y']) for p in b['poses']))
    return {'error': trace['error'], 'frames': len(frames), 'last_sweep': last['sweep'],
            'initial_contacts': len(frames[0]['contacts']), 'final_contacts': last['contacts'],
            'coupled_events': [f['coupled'] for f in frames if f.get('coupled')],
            'tail_frames': len(tail), 'persistent_pairs': pairs,
            'tail_maximum_pose_step_mm': max(motion, default=0),
            'tail_minimum_pose_step_mm': min(motion, default=0)}


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('problem', type=Path)
    parser.add_argument('config', type=Path)
    parser.add_argument('output', type=Path)
    parser.add_argument('--binary', type=Path, default=Path('target/release/pcb-maker'))
    args = parser.parse_args()
    args.output.mkdir(parents=True, exist_ok=False)
    for path, name in [(args.problem, 'problem.json'), (args.config, 'config.json'), (Path(__file__), 'trace_placement.py')]:
        shutil.copy2(path, args.output/name)
    command = [str(args.binary.resolve()), 'trace-placement', str(args.problem.resolve()), str(args.config.resolve()), str((args.output/'trace.json').resolve())]
    start = time.monotonic()
    with (args.output/'run.log').open('w') as log:
        process = subprocess.run(command, stdout=log, stderr=subprocess.STDOUT)
    provenance = {'command': command, 'exit_code': process.returncode, 'seconds': time.monotonic()-start,
                  'binary_sha256': sha(args.binary), 'problem_sha256': sha(args.problem), 'config_sha256': sha(args.config)}
    (args.output/'run.json').write_text(json.dumps(provenance, indent=2)+'\n')
    trace = json.loads((args.output/'trace.json').read_text())
    (args.output/'summary.json').write_text(json.dumps(summarize(trace), indent=2)+'\n')
    render(json.loads(args.problem.read_text()), trace, args.output)
    print(args.output/'index.html')


if __name__ == '__main__':
    main()
