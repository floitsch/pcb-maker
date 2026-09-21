#!/usr/bin/env python3
# Copyright (C) 2026 Toit contributors.
"""Inspect a prepared routing branch and automatically render its reachable region.

Requires a retained combined copper view. This is discrete routing evidence,
not native admission or an assertion that sampled blockers form a minimum cut.
"""
import argparse
import copy
import hashlib
import html
import json
from pathlib import Path
import shutil
import subprocess
import time
import xml.etree.ElementTree as ET


def render(report, background, output, highlight_net=None):
    namespace = 'http://www.w3.org/2000/svg'
    ET.register_namespace('', namespace)
    tag = lambda name: '{'+namespace+'}'+name
    source = ET.parse(background).getroot()
    svg = ET.Element(tag('svg'), {'viewBox':source.attrib['viewBox'], 'width':'1200', 'height':'1100'})
    ET.SubElement(svg,tag('title')).text = 'Prepared-grid reachable region on retained combined copper'
    underlay = ET.SubElement(svg,tag('g'),opacity='.55')
    for child in source: underlay.append(copy.deepcopy(child))
    step = report['resolution_mm'];ox,oy = report['origin_mm']
    colors = ['#16a66c','#8d45c5']
    region = ET.SubElement(svg,tag('g'),opacity='.45')
    bounds = []
    for layer,row,first,last in report['reachable_runs']:
        x,y = ox+(first-.5)*step, oy+(row-.5)*step
        width = (last-first+1)*step
        ET.SubElement(region,tag('rect'),x=str(x),y=str(y),width=str(width),height=str(step),fill=colors[layer])
        bounds.extend([(x,y),(x+width,y+step)])
    def shape(parent, geometry):
        kind=geometry['kind']
        if kind=='segment':
            a,b=geometry['start'],geometry['end']
            ET.SubElement(parent,tag('line'),x1=str(a[0]),y1=str(a[1]),x2=str(b[0]),y2=str(b[1]),**{'stroke-width':str(2*geometry['radius']), 'stroke-linecap':'round'})
        elif kind=='circle':
            x,y=geometry['center'];ET.SubElement(parent,tag('circle'),cx=str(x),cy=str(y),r=str(geometry['radius']),**{'stroke-width':'.2'})
        elif kind=='rectangle':
            x,y=geometry['center'];w,h=geometry['half_size']
            ET.SubElement(parent,tag('rect'),x=str(x-w),y=str(y-h),width=str(2*w),height=str(2*h),transform=f'rotate({geometry["angle_degrees"]} {x} {y})',**{'stroke-width':'.2'})
        elif kind=='polygon':
            ET.SubElement(parent,tag('polygon'),points=' '.join(f'{x},{y}' for x,y in geometry['points']),**{'stroke-width':'.2'})
        elif kind=='union':
            for part in geometry['parts']:shape(parent,part)
    if highlight_net:
        highlight=ET.SubElement(svg,tag('g'),stroke='#d32917',fill='none')
        for blocker in report['blockers']:
            if blocker['net']==highlight_net and blocker.get('geometry'):
                group=ET.SubElement(highlight,tag('g'))
                ET.SubElement(group,tag('title')).text=f'Model obstacle {blocker["obstacle_index"]}: {highlight_net}'
                shape(group,blocker['geometry'])
    for name,point,color in [('start',report['start_mm'],'#154fbd'),('finish',report['finish_mm'],'#bd1552')]:
        x,y=point
        ET.SubElement(svg,tag('circle'),cx=str(x),cy=str(y),r='1',fill='none',stroke=color,**{'stroke-width':'.3'})
        text=ET.SubElement(svg,tag('text'),x=str(x+1.3),y=str(y-1.3),fill=color,**{'font-size':'1.4','font-family':'sans-serif','paint-order':'stroke','stroke':'white','stroke-width':'.4'})
        text.text=name
    ET.ElementTree(svg).write(output/'overview.svg',encoding='unicode')
    if bounds:
        x0=min(p[0] for p in bounds)-3;y0=min(p[1] for p in bounds)-3
        x1=max(p[0] for p in bounds)+3;y1=max(p[1] for p in bounds)+3
        svg.set('viewBox',f'{x0} {y0} {x1-x0} {y1-y0}')
        svg.set('height',str(1200*(y1-y0)/(x1-x0)))
    ET.ElementTree(svg).write(output/'region.svg',encoding='unicode')
    subprocess.run(['rsvg-convert','--background-color','white','--width','1200','--output',str(output/'region.png'),str(output/'region.svg')],check=True)
    esc=lambda s:html.escape(str(s))
    page=['<!doctype html><meta charset="utf-8"><title>Routing cut inspection</title><style>body{font:17px system-ui;margin:2rem;max-width:1300px}img{max-width:100%}td,th{text-align:left;padding:.4rem;border-bottom:1px solid #bbb}code{font-size:12px}</style>',
          '<h1>'+esc(report['connection'])+': branch '+str(report['branch'])+'</h1>',
          '<p>'+esc(report['interpretation'])+'</p>',
          f'<p>Connected: {report["connected"]}. Start reaches {report["start_reachable_states"]} states; finish reaches {report["finish_reachable_states"]}. Showing the smaller region at the {report["inspected_endpoint"]}.</p>',
          '<p>Green: reachable front-layer cells. Purple: reachable back-layer cells. Copper from both layers is shown with component bodies hidden.</p>',
          '<p><a href="report.json">Structured evidence</a> · <a href="overview.svg">Whole-board view</a> · <a href="run.json">Run provenance</a></p>',
          '<img src="region.svg" alt="Reachable terminal region on combined copper">',
          f'<p>Planar boundary: {report["sampled_planar_transitions"]}/{report["planar_boundary_transitions"]} transitions sampled. Forbidden vias: {report["sampled_via_entries"]}/{report["rejected_via_entries"]} entries sampled. {report["samples_without_copper_attribution"]} samples have no copper attribution.</p>',
          ('<p>Red outlines: sampled boundary geometry belonging to '+esc(highlight_net)+'.</p>' if highlight_net else ''),
          '<table><tr><th>Net</th><th>Kind / footprint</th><th>Model obstacle</th><th>Planar hits</th><th>Via hits</th></tr>']
    for b in report['blockers']:
        page.append('<tr><td>'+esc(b['net'])+'</td><td>'+esc(b['kind'])+' / '+esc(b['footprint'])+'</td><td><code>'+esc(b.get('obstacle_index',b['object']))+'</code></td><td>'+str(b['planar_hits'])+'</td><td>'+str(b['via_hits'])+'</td></tr>')
    page.append('</table>')
    (output/'index.html').write_text(''.join(page))


def inspect(args):
    out=args.output.resolve();out.mkdir(parents=True,exist_ok=False)
    source=args.source.resolve();board=source/(args.board_id+'.kicad_pcb')
    digest=lambda p:hashlib.sha256(p.read_bytes()).hexdigest()
    before=digest(board)
    shutil.copy2(__file__,out/Path(__file__).name)
    shutil.copy2(source/'inspection-combined.svg',out/'retained-combined.svg')
    config=dict(branch=args.branch,maximum_samples_per_kind=args.maximum_samples,routing=json.loads(args.config.read_text()))
    (out/'config.json').write_text(json.dumps(config,indent=2)+'\n')
    started=time.monotonic()
    command=[str(args.binary.resolve()),'inspect-kicad-routing-cut',str(board),args.connection,str(out/'report.json'),str(out/'config.json')]
    with (out/'run.log').open('w') as log:
        code=subprocess.run(command,stdout=log,stderr=subprocess.STDOUT).returncode
    record=dict(command=command,exit_code=code,seconds=time.monotonic()-started,source_sha256=before,
                source_unchanged=digest(board)==before,binary_sha256=digest(args.binary))
    (out/'run.json').write_text(json.dumps(record,indent=2)+'\n')
    assert record['source_unchanged']
    if code:
        shutil.copy2(out/'retained-combined.svg',out/'failed-source.svg')
        raise RuntimeError('Inspection failed; source rendering and command log retained')
    report=json.loads((out/'report.json').read_text())
    render(report,out/'retained-combined.svg',out,args.highlight_net)
    print(json.dumps({k:report[k] for k in ['connected','start_reachable_states','finish_reachable_states','inspected_endpoint','boundary_sampling_truncated']}))


if __name__=='__main__':
    parser=argparse.ArgumentParser(description=__doc__)
    for name in ['source','config','output','binary']:parser.add_argument('--'+name,type=Path,required=True)
    parser.add_argument('--board-id',required=True)
    parser.add_argument('--connection',required=True)
    parser.add_argument('--branch',type=int,default=0)
    parser.add_argument('--maximum-samples',type=int,default=2048)
    parser.add_argument('--highlight-net')
    inspect(parser.parse_args())
