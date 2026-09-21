# Copyright (C) 2026 Toit contributors.
"""Geometry controls for protected marks; always writes a marking-only SVG."""
import importlib.util
import json
import math
from pathlib import Path
import sys

ROOT=Path(__file__).resolve().parents[2]
spec=importlib.util.spec_from_file_location('silk',ROOT/'crates/pcb-kicad/src/silkscreen.py')
silk=importlib.util.module_from_spec(spec);spec.loader.exec_module(silk)
p= silk.pcbnew


def run(output):
    output.mkdir(parents=True,exist_ok=False)
    results=[];svg=['<svg xmlns="http://www.w3.org/2000/svg" width="800" height="1680" viewBox="0 0 800 1680"><rect width="800" height="1680" fill="white"/>']
    for index,(name,angle,kind,expected) in enumerate([
        ('edge cross',0,'cross',True),('rotated edge cross',45,'cross',True),
        ('already legal rotated cross',45,'legal',True),
        ('unequal arms',0,'unequal',False),('T junction',0,'tee',False),
        ('attached third line',0,'attached',False),('no room',0,'blocked',True),
    ]):
        board=p.BOARD();fp=p.FOOTPRINT(board);fp.SetReference('MARK');board.Add(fp);fp.thisown=False
        center=(.45 if angle==45 and kind=='cross' else .6,2);theta=math.radians(angle)
        def transform(x,y):return (center[0]+x*math.cos(theta)-y*math.sin(theta),center[1]+x*math.sin(theta)+y*math.cos(theta))
        def line(a,b):
            s=p.PCB_SHAPE(fp);s.SetShape(p.SHAPE_T_SEGMENT);s.SetLayer(p.F_SilkS);s.SetWidth(120000)
            s.SetStart(silk.vector(transform(*a)));s.SetEnd(silk.vector(transform(*b)));fp.Add(s);s.thisown=False
            return s
        a=line((-.5,0),(.5,0));b=line((0,0 if kind=='tee' else -.5),(0,.8 if kind=='unequal' else .5))
        shapes=[(fp,a),(fp,b)]
        if kind=='attached':shapes.append((fp,line((.5,0),(1,0))))
        before=[silk.shape_record(s,'MARK') for _,s in shapes]
        report={'protected_marks':[],'unresolved_marks':[]};patches={}
        protected=silk.move_cross_groups(shapes,(0,0,6,6),[(0,0,6,6)] if kind=='blocked' else [],[],[],[],patches,report)
        after=[silk.shape_record(s,'MARK') for _,s in shapes]
        passed=bool(protected)==expected
        if expected:
            passed &= (bool(report['unresolved_marks'])==(kind=='blocked'))
            for old,new in zip(before,after):
                passed &= abs(math.dist(old['a'],old['b'])-math.dist(new['a'],new['b']))<2e-6
            if kind in ('blocked','legal'):passed &= before==after and patches=={}
            else:passed &= before!=after
        else:passed &= before==after and patches=={}
        results.append({'case':name,'passed':passed,'proposal':report})
        y=index*240
        svg.append(f'<text x="15" y="{y+24}" font-size="18">{name}: {"PASS" if passed else "FAIL"} (before / after)</text>')
        for column,records in enumerate([before,after]):
            x=35+column*390
            svg.append(f'<g transform="translate({x},{y+38}) scale(30)"><rect width="6" height="6" fill="#184c32" stroke="black" stroke-width=".03"/>')
            if kind=='blocked':svg.append('<rect width="6" height="6" fill="#926347"/>')
            for s in records:
                svg.append(f'<line x1="{s["a"][0]}" y1="{s["a"][1]}" x2="{s["b"][0]}" y2="{s["b"][1]}" stroke="white" stroke-width="{2*s["radius"]}" stroke-linecap="round"/>')
            svg.append('</g>')
    svg.append('</svg>');(output/'controls.svg').write_text(''.join(svg))
    (output/'controls.json').write_text(json.dumps(results,indent=2)+'\n')
    assert all(r['passed'] for r in results),results
    print(f'{len(results)} mark recognition/relocation controls passed; rendered controls.svg')


if __name__=='__main__':run(Path(sys.argv[1]))
