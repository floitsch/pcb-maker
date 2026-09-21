# Copyright (C) 2026 Toit contributors.
"""Combined and per-layer copper SVGs, using native preview labels and crop.

Usage: render_inspection_layers.py VERIFIED_DIRECTORY BOARD_ID
Combined copper is viewed from the front with translucent layer groups. The
separate back view is mirrored to match the bottom camera; text stays upright.
"""
import copy
import json
from pathlib import Path
import subprocess
import sys
import xml.etree.ElementTree as ET

NS='http://www.w3.org/2000/svg'
ET.register_namespace('',NS)
def tag(name):return '{'+NS+'}'+name


def render(directory,board_id):
    preview=ET.parse(directory/'preview.svg').getroot()
    x,y,w,h=map(float,preview.attrib['viewBox'].split())
    labels=json.loads((directory/'preview-labels.json').read_text())['labels']
    native_layers={}
    for side,layer in [('front','F.Cu'),('back','B.Cu')]:
        target=directory/f'inspection-{side}.svg'
        subprocess.run(['kicad-cli','pcb','export','svg','--mode-single','--layers',layer+',Edge.Cuts',
                        '--page-size-mode','0','--exclude-drawing-sheet','--output',str(target),
                        str(directory/f'{board_id}.kicad_pcb')],check=True,stdout=subprocess.DEVNULL)
        svg=ET.parse(target).getroot()
        for key in ['width','height','viewBox']:svg.set(key,preview.attrib[key])
        native_layers[side]=[copy.deepcopy(child) for child in svg]
        if side=='back':
            group=ET.Element(tag('g'),{'transform':f'translate({2*x+w} 0) scale(-1 1)'})
            for child in list(svg):svg.remove(child);group.append(child)
            svg.append(group)
        overlay=ET.SubElement(svg,tag('g'),{'font-family':'sans-serif','font-size':'.65','fill':'#172b3a',
            'stroke':'white','stroke-width':'.12','paint-order':'stroke fill','stroke-linejoin':'round'})
        for label in labels:
            px,py=label['at_mm'];px=2*x+w-px if side=='back' else px
            tx=max(x+.2,min(px+.7,x+w-len(label['label'])*.42));ty=max(y+.8,min(py-.7,y+h-.2))
            text=ET.SubElement(overlay,tag('text'),{'x':str(tx),'y':str(ty)});text.text=label['label']
            ET.SubElement(text,tag('title')).text=f'{label["uuid"]}: {label["net"]}'
        ET.ElementTree(svg).write(target,encoding='unicode')
        subprocess.run(['rsvg-convert','--background-color','white','--width','1200','--output',str(target.with_suffix('.png')),str(target)],check=True)
    combined=copy.deepcopy(preview)
    for child in list(combined):combined.remove(child)
    ET.SubElement(combined,tag('title')).text='Combined front and back copper, viewed from the front; component bodies hidden.'
    for side in ['front','back']:
        group=ET.SubElement(combined,tag('g'),{'id':f'combined-{side}-copper','opacity':'.8'})
        for child in native_layers[side]:group.append(child)
    # Labels use front-view coordinates, stay opaque and appear only once.
    overlay=ET.SubElement(combined,tag('g'),{'id':'combined-inspection-labels','font-family':'sans-serif','font-size':'.65',
        'fill':'#172b3a','stroke':'white','stroke-width':'.12','paint-order':'stroke fill','stroke-linejoin':'round'})
    for label in labels:
        px,py=label['at_mm']
        tx=max(x+.2,min(px+.7,x+w-len(label['label'])*.42));ty=max(y+.8,min(py-.7,y+h-.2))
        text=ET.SubElement(overlay,tag('text'),{'x':str(tx),'y':str(ty)});text.text=label['label']
        ET.SubElement(text,tag('title')).text=f'{label["uuid"]}: {label["net"]}'
    target=directory/'inspection-combined.svg'
    ET.ElementTree(combined).write(target,encoding='unicode')
    subprocess.run(['rsvg-convert','--background-color','white','--width','1200','--output',str(target.with_suffix('.png')),str(target)],check=True)


if __name__=='__main__':render(Path(sys.argv[1]),sys.argv[2])
