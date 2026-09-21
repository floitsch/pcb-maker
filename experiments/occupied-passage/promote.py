#!/usr/bin/env python3
# Copyright (C) 2026 Toit contributors.
"""Native-check the exact semantic copper under explicitly matching rules."""
import argparse
import hashlib
import json
from pathlib import Path
import shutil
import subprocess

parser=argparse.ArgumentParser()
parser.add_argument('result',type=Path)
parser.add_argument('output',type=Path)
parser.add_argument('--problem',type=Path,default=Path('benchmarks/small/passage-pressure-occupied.json'))
args=parser.parse_args()
root=Path(__file__).resolve().parents[2]
exe=root/'target/release/pcb-maker'
result=json.loads(args.result.read_text()); problem=json.loads(args.problem.read_text())
assert result['validation']['complete'], 'Only promote an independently admitted semantic result'
assert not args.output.exists(), 'Use a fresh output directory to preserve previous evidence'
args.output.mkdir(parents=True)
poses={c['id']:c for c in result['candidate']['components']}
for c in problem['components']:
    c['position']=poses[c['id']]['position']; c['rotation_degrees']=poses[c['id']]['rotation_degrees']
(args.output/'problem.json').write_text(json.dumps(problem,indent=2))
config=dict(board_id='occupied-passage',connection_order=[n['id'] for n in problem['nets']],
            board_origin_mm=[10,10],board_edge_margin_mm=0,courtyard_clearance_mm=0,
            copper_clearance_mm=problem['rules']['clearance'],
            copper_edge_clearance_mm=problem['rules']['clearance'])
(args.output/'template.json').write_text(json.dumps(config,indent=2))
with (args.output/'run.log').open('w') as log:
    def command(*words,check=True):
        return subprocess.run([str(exe),*map(str,words)],stdout=log,stderr=subprocess.STDOUT,check=check)
    command('generate-semantic-kicad-ladder',args.output/'problem.json','declared',args.output/'template.json',args.output/'generated')
    command('materialize-kicad-rung',args.output/'generated/declaration.json',len(problem['nets']),args.output/'unrouted')
    source=next((args.output/'unrouted').glob('*/*.kicad_pcb'))
    native=args.output/'verified'
    shutil.copytree(source.parent,native)
    import pcbnew
    if not hasattr(pcbnew.SwigPyIterator,'next'):
        pcbnew.SwigPyIterator.next=pcbnew.SwigPyIterator.__next__
    path=native/source.name; board=pcbnew.LoadBoard(str(path.resolve()))
    assert not list(board.GetTracks())
    original_pads=sorted((p.GetNetname(),p.GetPosition().x,p.GetPosition().y) for f in board.GetFootprints() for p in f.Pads())
    for trace in result['candidate']['traces']:
        assert not trace['vias'],'This promotion experiment does not implement via export'
        net=board.FindNet('/'+trace['electrical_net']); assert net
        for i,(a,b) in enumerate(zip(trace['points'],trace['points'][1:])):
            layer=(trace['segment_layers'] or [trace['layer']]*(len(trace['points'])-1))[i]
            assert layer in ['top','bottom']
            track=pcbnew.PCB_TRACK(board)
            track.SetStart(pcbnew.VECTOR2I(round((a['x']+10)*1e6),round((a['y']+10)*1e6)))
            track.SetEnd(pcbnew.VECTOR2I(round((b['x']+10)*1e6),round((b['y']+10)*1e6)))
            track.SetWidth(round(trace['width']*1e6));track.SetLayer(pcbnew.F_Cu if layer=='top' else pcbnew.B_Cu)
            track.SetNetCode(net.GetNetCode());board.Add(track)
    pcbnew.SaveBoard(str(path.resolve()),board)
    assert original_pads==sorted((p.GetNetname(),p.GetPosition().x,p.GetPosition().y) for f in board.GetFootprints() for p in f.Pads())
    (args.output/'promotion.json').write_text(json.dumps({
        'source_problem_sha256':hashlib.sha256(args.problem.read_bytes()).hexdigest(),
        'candidate_sha256':hashlib.sha256(args.result.read_bytes()).hexdigest(),
        'executable_sha256':hashlib.sha256(exe.read_bytes()).hexdigest(),
        'rules':config,'segments':len(list(board.GetTracks())),
        'scope':'Exact semantic polylines and widths, translated by (10,10) mm and rounded to native nanometers. No native rerouting; explicit edge clearance matches the declared semantic boundary rule.'},indent=2))
    outcome=command('verify-kicad-rung',native,config['board_id'],check=False)
    print('Native verification exit:',outcome.returncode,'preview:',native/'preview.svg')
    if outcome.returncode: raise SystemExit(outcome.returncode)
