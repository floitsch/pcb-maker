#!/usr/bin/env python3
# Copyright (C) 2026 Toit contributors.
"""Inspect and automatically render pad/grid access across chosen resolutions."""
import argparse
import copy
import hashlib
import html
import json
from pathlib import Path
import shutil
import subprocess


def shape(geometry, color):
    kind = geometry['kind']
    style = f'fill="{color}" stroke="{color}" stroke-width=".008"'
    if kind == 'circle':
        x,y=geometry['center'];return f'<circle cx="{x}" cy="{y}" r="{geometry["radius"]}" {style}/>'
    if kind == 'rectangle':
        x,y=geometry['center'];w,h=geometry['half_size']
        return f'<rect x="{x-w}" y="{y-h}" width="{2*w}" height="{2*h}" transform="rotate({geometry["angle_degrees"]} {x} {y})" {style}/>'
    if kind == 'segment':
        a,b=geometry['start'],geometry['end']
        return f'<line x1="{a[0]}" y1="{a[1]}" x2="{b[0]}" y2="{b[1]}" stroke="{color}" stroke-width="{2*geometry["radius"]}" stroke-linecap="round"/>'
    if kind == 'polygon':
        points=' '.join(f'{x},{y}' for x,y in geometry['points'])
        return f'<polygon points="{points}" {style}/>'
    if kind == 'union':return ''.join(shape(part,color) for part in geometry['parts'])
    raise ValueError(kind)


def render(terminal, resolution, path):
    low,high=terminal['viewport']['min'],terminal['viewport']['max']
    width,height=high[0]-low[0],high[1]-low[1]
    title=f'{" / ".join(terminal["footprints"])} {terminal["layer"]}; grid {resolution:g} mm'
    scale=min(720/width,720/height)
    parts=['<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 750 850" width="750" height="850">',
           '<rect width="750" height="850" fill="white"/>',
           f'<text x="15" y="30" font-family="sans-serif" font-size="20">{html.escape(title)}</text>',
           f'<g transform="translate(15 60) scale({scale}) translate({-low[0]} {-low[1]})">',
           f'<defs><clipPath id="crop"><rect x="{low[0]}" y="{low[1]}" width="{width}" height="{height}"/></clipPath></defs>',
           '<g clip-path="url(#crop)">']
    for i,obstacle in enumerate(terminal['obstacles']):
        parts.append('<g><title>'+html.escape(f'O{i+1}: {obstacle["object"]}; net={obstacle["net"]}; clearance={obstacle["clearance_mm"]}')+'</title>'+shape(obstacle['geometry'],'#edc5b8')+'</g>')
    parts.append(shape(terminal['geometry'],'#b8dfeb'))
    dot=max(.012,min(.025,resolution/9))
    for cell in terminal['cells']:
        color='#bf2636' if cell['blocked'] else ('#008b43' if cell['inside_pad'] else '#aab2ba')
        x,y=cell['at'];reason='; '.join(cell['blockers']) or ('outline' if cell['outside_outline'] else 'clear')
        parts.append(f'<circle cx="{x}" cy="{y}" r="{dot}" fill="{color}"><title>{html.escape(reason)}</title></circle>')
    x,y=terminal['anchor'];parts.append(f'<circle cx="{x}" cy="{y}" r=".045" fill="#ffd43d" stroke="#333" stroke-width=".008"/>')
    if terminal['nearest']:
        x,y=terminal['nearest']['at'];d=.07
        parts.append(f'<path d="M {x-d} {y-d} L {x+d} {y+d} M {x-d} {y+d} L {x+d} {y-d}" fill="none" stroke="#111" stroke-width=".018"/>')
    parts.append('</g></g>')
    parts.append('<text x="15" y="810" font-family="sans-serif" font-size="14">Gold: anchor; X: nearest grid point; green: unblocked inside pad; red: blocked.</text></svg>')
    path.write_text(''.join(parts))


def main():
    p=argparse.ArgumentParser(description=__doc__)
    p.add_argument('board',type=Path);p.add_argument('connection');p.add_argument('config',type=Path);p.add_argument('output',type=Path)
    p.add_argument('--binary',type=Path,default=Path('target/release/pcb-maker'))
    p.add_argument('--resolutions',type=float,nargs='+',default=[.5,.25,.125])
    p.add_argument('--footprint',help='Render this footprint only; retain every terminal in JSON')
    args=p.parse_args();args.output.mkdir(parents=True,exist_ok=False)
    config=json.loads(args.config.read_text());rows=[]
    shutil.copy2(__file__,args.output/'inspect_terminal_access.py')
    for resolution in args.resolutions:
        root=args.output/f'grid-{resolution:g}';root.mkdir()
        candidate=copy.deepcopy(config);candidate['resolution_mm']=resolution
        (root/'config.json').write_text(json.dumps(candidate,indent=2)+'\n')
        command=[str(args.binary.resolve()),'inspect-kicad-terminal-access',str(args.board.resolve()),args.connection,str((root/'report.json').resolve()),str((root/'config.json').resolve())]
        with (root/'run.log').open('w') as log:
            subprocess.run(command,stdout=log,stderr=subprocess.STDOUT,check=True)
        report=json.loads((root/'report.json').read_text())
        for i,terminal in enumerate(report['terminals']):
            if args.footprint and args.footprint not in terminal['footprints']:continue
            nearest=next((c for c in terminal['cells'] if terminal['nearest'] and c['grid']==terminal['nearest']['grid']),None)
            path=root/f'terminal-{i:03d}.svg';render(terminal,resolution,path)
            rows.append({'resolution_mm':resolution,'footprints':terminal['footprints'],'layer':terminal['layer'],'anchor':terminal['anchor'],
                'nearest_blocked':nearest['blocked'] if nearest else None,
                'nearest_blockers':nearest['blockers'] if nearest else [],
                'clear_centers_inside_pad':sum(c['inside_pad'] and not c['blocked'] for c in terminal['cells']),
                'clear_connected_centers_inside_pad':sum(c.get('anchor_segment_inside_pad',False) and not c['blocked'] for c in terminal['cells']),
                'image':str(path.relative_to(args.output))})
    summary={'source_sha256':hashlib.sha256(args.board.read_bytes()).hexdigest(),
        'binary_sha256':hashlib.sha256(args.binary.read_bytes()).hexdigest(),
        'connection':args.connection,'rows':rows,
        'scope':'Center-point clearance and optional straight pad-internal continuity checks; green cells do not prove a routed path or native admission.'}
    (args.output/'summary.json').write_text(json.dumps(summary,indent=2)+'\n')
    images=''.join(f'<section><p>{r["resolution_mm"]} mm: nearest blocked={r["nearest_blocked"]}; {r["clear_centers_inside_pad"]} clear centers inside pad</p><img src="{r["image"]}"></section>' for r in rows)
    (args.output/'index.html').write_text('<!doctype html><meta charset="utf-8"><title>Terminal access diagnostic</title><style>body{font:17px system-ui;margin:2rem}main{display:flex;flex-wrap:wrap}section{width:32%;min-width:300px}img{width:100%}</style><h1>'+html.escape(args.connection)+'</h1><p>Local parsed copper and routing-rule geometry. Bodies are hidden. Each crop identifies its copper layer; no route or native-validity claim is implied.</p><main>'+images+'</main><a href="summary.json">Diagnostic summary</a>')
    print(json.dumps(summary,indent=2))


if __name__=='__main__':main()
