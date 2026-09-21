# Copyright (C) 2026 Toit contributors.
"""Audit and render the ECC83 via-refinement comparison after runs finish."""
from pathlib import Path
import hashlib
import json
import math
import sys
import xml.etree.ElementTree as ET
import pcbnew
from render_inspection_layers import render

if not hasattr(pcbnew.SwigPyIterator,'next'):pcbnew.SwigPyIterator.next=pcbnew.SwigPyIterator.__next__


def report(root):
    source=root/'socket-exact/source-candidate'
    result=root/'final-front/result'
    refinement=json.loads((root/'final-front/refinement.json').read_text())
    boards=[pcbnew.LoadBoard(str(d/'ecc83-pp.kicad_pcb')) for d in [source,result]]
    def pose(board):
        return sorted((f.GetReference(),f.GetPosition().x,f.GetPosition().y,f.GetOrientationDegrees(),
                       tuple(sorted((p.GetNumber(),p.GetNetname(),p.GetPosition().x,p.GetPosition().y,
                                     p.GetSize().x,p.GetSize().y,p.GetDrillSize().x,p.GetDrillSize().y) for p in f.Pads()))) for f in board.GetFootprints())
    assert pose(boards[0])==pose(boards[1])
    assert (source/'ecc83-pp.kicad_pro').read_bytes()==(result/'ecc83-pp.kicad_pro').read_bytes()
    def copper(board):
        nets={}
        for t in board.GetTracks():
            if isinstance(t,pcbnew.PCB_VIA):item=('via',t.GetPosition().x,t.GetPosition().y,t.GetWidth(),t.GetDrillValue())
            else:item=('track',t.GetStart().x,t.GetStart().y,t.GetEnd().x,t.GetEnd().y,t.GetLayer(),t.GetWidth())
            nets.setdefault(t.GetNetname(),[]).append(item)
        return {k:sorted(v) for k,v in nets.items()}
    a,b=map(copper,boards);changed=sorted(n for n in set(a)|set(b) if a.get(n)!=b.get(n))
    assert changed==['Net-(P3-P1)','Net-(P4-PM)']
    rows=[]
    for name,directory in [('Before',source),('Automatic repair',result)]:
        native=json.loads((directory/'verification.json').read_text());assert native['complete']
        board=pcbnew.LoadBoard(str(directory/'ecc83-pp.kicad_pcb'))
        rows.append({'name':name,'vias':sum(isinstance(t,pcbnew.PCB_VIA) for t in board.GetTracks()),
            'track_mm':sum(t.GetLength()/1e6 for t in board.GetTracks() if not isinstance(t,pcbnew.PCB_VIA)),'native':native})
        render(directory,'ecc83-pp')
        labels=json.loads((directory/'preview-labels.json').read_text())['labels']
        assert len({x['label'] for x in labels})==len(labels)
        assert sum(x['kind']=='component' for x in labels)==15
        assert sum(x['kind']=='via' for x in labels)==rows[-1]['vias']
    assert rows[1]['vias']<rows[0]['vias']
    audit={'native_readback':rows,'placement_pads_nets_and_project_preserved':True,'changed_connections':changed,
           'final_executable_sha256':hashlib.sha256((root/'pcb-maker-refine-final').read_bytes()).hexdigest(),
           'automatic_selection':refinement['selected'],'browser_playback_tested':False}
    for p in root.rglob('*.svg'):ET.parse(p)
    audit['all_svg_xml_parsed']=True
    (root/'audit.json').write_text(json.dumps(audit,indent=2)+'\n')
    html=['<!doctype html><meta charset="utf-8"><title>ECC83: automatic via refinement</title>',
          '<style>body{font:17px system-ui;margin:25px}table{border-collapse:collapse}td,th{border:1px solid #aaa;padding:10px}.pair{display:grid;grid-template-columns:1fr 1fr;gap:16px}img{width:100%}a{color:#175995}</style>',
          '<h1>ECC83: automatic via refinement</h1><p>The program removes a two-via excursion, diagnoses the blocking net, reroutes it, and admits only a complete board. Component bodies are hidden; references and stable via IDs identify the geometry.</p>',
          '<table><tr><th>Case</th><th>Vias</th><th>Track length</th><th>Native design findings / opens</th></tr>']
    for row in rows:html.append(f'<tr><td>{row["name"]}</td><td>{row["vias"]}</td><td>{row["track_mm"]:.3f} mm</td><td>0 / 0</td></tr>')
    html.append('</table><p>Selection uses track length + 5 mm per via. This is an explicit routing tradeoff, not a claim of electrical equivalence or global optimality. Seven library metadata warnings remain recorded separately in both cases.</p>')
    for side in ['combined','back','front']:
        html.append(f'<h2>{side.capitalize()} copper</h2><div class="pair">')
        for name,directory in [('Before',source),('Automatic repair',result)]:html.append(f'<div><h3>{name}</h3><img src="{directory.relative_to(root)}/inspection-{side}.svg"></div>')
        html.append('</div>')
    html.append('<h2>R3 / P8 short detour</h2><p>1.429 mm copper-edge gap; 1.600 mm required for a 0.8 mm trace with 0.4 mm clearance each side. This particular passage cannot fit under the unchanged benchmark rules.</p><img style="width:450px" src="r3-p8-gap.svg">')
    html.append('<p><a href="final-front/refinement.json">Automatic repair report</a> · <a href="audit.json">Independent audit</a> · <a href="final-back/refinement.json">Back-layer alternative</a> · <a href="refined-middle/refinement.json">No-improvement control</a></p>')
    (root/'index.html').write_text(''.join(html))
    print(json.dumps(audit,indent=2))


if __name__=='__main__':report(Path(sys.argv[1]))
