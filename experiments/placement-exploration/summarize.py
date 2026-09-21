#!/usr/bin/env python3
# Copyright (C) 2026 Toit contributors.
"""Audit retained experiment artifacts and collect native copper measurements."""
import copy
import hashlib
import json
import math
from pathlib import Path
import subprocess
import run

def main():
    import pcbnew
    if not hasattr(pcbnew.SwigPyIterator,'next'):
        pcbnew.SwigPyIterator.next=pcbnew.SwigPyIterator.__next__
    root=run.OUT;data=json.loads((root/'results.json').read_text());records=data['records'];by={(r['fixture'],r['label']):r for r in records}
    validation={'proposal_count':len(records),'all_proposal_renders_present':all((root/r['render']).exists() for r in records),'all_emitted_poses_pass_independent_body_audit':all(r['independent_pose_validation']['passed'] for r in records if r['ok']),'all_toy_routes_pass_continuous_replay':all(r['route_probe']['continuous_output_validation_passed'] for r in records if 'route_probe' in r),'scope':'Independent body audit is separate SAT implementation; route output replay uses the same exact geometric predicates as its search. Native KiCad is an additional independent authority for five real-board partial routing probes.'}
    for label in ['retained','barycentric','harmonic-full-size','junction-reinsert']:
        assert by['movable-wall',label]['route_probe']['completed']==0
    for label in ['pressure-shift--1','pressure-shift-+1']:
        assert by['movable-wall',label]['route_probe']['completed']==1
        assert by['movable-wall',label]['result']['evidence']['connectivity_distance_after_mm']==by['movable-wall','retained']['result']['evidence']['connectivity_distance_after_mm']
    validation['wall_analytic_control']={'board_height_mm':14,'wall_height_mm':11.6,'centered_gap_mm':1.2,'required_gap_mm':1.4,'shift_mm':0.6,'opened_gap_mm':1.8,'unchanged_connection_surrogate':True,'directed_shift_is_hand_specified_oracle':True}
    assert by['crossed-resistors','retained']['route_probe']['completed']==4
    assert abs(by['crossed-resistors','retained']['route_probe']['length_mm']-64)<1e-8
    assert abs(by['crossed-resistors','junction-reinsert']['route_probe']['length_mm']-27.2)<1e-8
    # Correct the original reduced-state drawings to collapse their pad anchors
    # as well as bodies. This only regenerates views of stored experiment data.
    frames=[]
    for r in records:
        problem=json.loads((root/f'{r["fixture"]}.problem.json').read_text())
        if r['label'].startswith('pressure-shift'):
            problem=copy.deepcopy(problem);problem['components'][-1]['position']['y']+=(-0.6 if '--1' in r['label'] else 0.6)
        poses=r['result']['poses'] if r['ok'] else run.poses_of(problem)
        title=f'{r["fixture"]} / {r["label"]}: '+('legal full-size placement' if r['ok'] else 'rejected; displaying input')
        if 'route_probe' in r:title+=f' / routed {r["route_probe"]["completed"]}/{r["route_probe"]["total"]}'
        drawing=run.svg(problem,poses,title,r.get('route_probe'));(root/r['render']).write_text(drawing)
        if r['junctions']:
            jdrawing=run.svg(problem,r['junctions'],f'{r["fixture"]} / {r["label"]}: reduced junction seed, NOT full-size legal',junction=True)
            (root/r['junction_render']).write_text(jdrawing);frames.append({'label':f'{r["fixture"]} / {r["label"]} / reduced seed','svg':jdrawing})
        frames.append({'label':title,'svg':drawing})
    page=(root/'index.html').read_text();start=page.index('const frames=')+len('const frames=');end=page.index(';let index=',start)
    (root/'index.html').write_text(page[:start]+json.dumps(frames).replace('</','<\\/')+page[end:])
    native=[]
    for folder in ['native-prefix03','native-prefix03-followups']:
        manifest=json.loads((root/folder/'manifest.json').read_text());assert manifest['executable_unchanged']
        for c in manifest['cases']:
            d=Path(c['directory']);summary=json.loads((d/'run/cold-prefix.json').read_text());progress=json.loads((d/'run/solve/progression.json').read_text());final=Path(progress['final_directory']);verification=json.loads((final/'verification.json').read_text());initial=Path(summary['initial']['output_directory'])
            source=json.loads((d/'problem.json').read_text());sourceposes={p['id']:(p['position'],p.get('rotation_degrees',0)) for p in source['components']}
            for pose in summary['placement']['poses']:
                original,angle=sourceposes[pose['component']]
                assert math.dist(run.xy(original),run.xy(pose['position']))<1e-8
                assert abs((angle-pose['rotation_degrees']+180)%360-180)<1e-8
            def native_poses(board):
                return sorted((str(f.GetReference()),f.GetPosition().x,f.GetPosition().y,round(f.GetOrientationDegrees(),9)) for f in board.GetFootprints())
            b=pcbnew.LoadBoard(str(final/'dual-esp32.kicad_pcb'));b0=pcbnew.LoadBoard(str(initial/'dual-esp32.kicad_pcb'));assert len(list(b0.GetTracks()))==0;assert native_poses(b)==native_poses(b0)
            length=0;vias=0;segments=0
            for track in b.GetTracks():
                if isinstance(track,pcbnew.PCB_VIA):vias+=1
                else:
                    assert not isinstance(track,pcbnew.PCB_ARC)
                    length+=pcbnew.ToMM(track.GetLength());segments+=1
            assert verification['complete'] and (final/'preview.svg').exists()
            target=d/'final-preview.svg';target.write_bytes((final/'preview.svg').read_bytes())
            subprocess.run(['rsvg-convert','--background-color','white','--width','1000','--output',str(d/'final-preview.png'),str(target)],check=True)
            native.append({'label':c['label'],'native_completed':summary['completed'],'final_rung':progress['final_rung'],'trace_length_mm':length,'vias':vias,'segments':segments,'score_length_plus_2mm_per_via':length+2*vias,'astar_expansions':progress['total_route_expansions'],'verification':verification,'placement_preserved_from_proposed_pose':True,'native_placement_unchanged_during_routing':True,'preview':str(d/'final-preview.svg')})
    validation['native_cases']=native
    validation['followup_proposal_count']=sum(len(json.loads((root/f/'results.json').read_text())['records']) for f in ['constraint-aware','constraint-aware-with-floor'])
    (root/'validation.json').write_text(json.dumps(validation,indent=2)+'\n');print(json.dumps(validation,indent=2))
    native_frames=[dict(r,svg=Path(r['preview']).read_text()) for r in native]
    document='''<!doctype html><meta charset="utf-8"><title>Native placement comparison</title><style>body{font:16px system-ui;background:#16212c;color:white;margin:20px}button,select{font:inherit}#board{background:white}#board svg{width:100%;height:75vh}</style><h1>Native routing of five placement seeds</h1><p>All 43 components were placed using full connectivity; the same first three connections were then routed from zero copper. All five native checks pass. This cycles independently computed layouts, not physical motion. The remaining connections have not been routed in this experiment.</p><button onclick="playing=!playing">Play / pause</button> <select id="pick"></select><p id="info"></p><div id="board"></div><script>const f=FRAMES;let i=0,playing=false;const pick=document.getElementById('pick');f.forEach((r,k)=>pick.add(new Option(r.label,k)));function show(){pick.value=i;let r=f[i];document.getElementById('info').textContent=r.label+' — '+r.trace_length_mm.toFixed(3)+' mm / '+r.vias+' vias / '+r.astar_expansions.toLocaleString()+' A* expansions';document.getElementById('board').innerHTML=r.svg;}pick.onchange=()=>{i=Number(pick.value);show()};setInterval(()=>{if(playing){i=(i+1)%f.length;show()}},1600);show();</script>'''.replace('FRAMES',json.dumps(native_frames).replace('</','<\\/'))
    (root/'native-comparison.html').write_text(document)
    overview='''<!doctype html><meta charset="utf-8"><title>Placement exploration</title><style>body{max-width:950px;margin:40px auto;font:18px system-ui;line-height:1.55;background:#14202c;color:#edf5fc}a{color:#91def7}</style><h1>Placement exploration: 97 proposals, five native probes</h1><p>The existing dimension-aware harmonic placement wins this partial real-board probe: 50.845 mm without vias, versus the retained pose's 92.432 mm without vias. Explicit junction reduction improves the retained layout but loses to full-size harmonic here.</p><ul><li><a href="native-comparison.html">Compare five native layouts and copper metrics</a></li><li><a href="index.html">86 initial proposals, reduced seeds and failures</a></li><li><a href="constraint-aware/index.html">Ten constraint-aware follow-up outcomes</a></li><li><a href="constraint-aware-with-floor/index.html">Successful junction reinsertion using the existing spacing floor</a></li><li><a href="validation.json">Validation and native measurements</a></li><li><a href="results.json">Initial proposal results</a></li></ul><p>Native probes use full-connectivity placement seeds, then route the same first three connections from zero copper. They do not establish full-board completion. Rejected placements were not sent to native routing. Every native probe has zero design findings; retained metadata warning counts vary with pose.</p><p>These viewers cycle independently computed proposals. They do not depict continuous spring motion.</p><h2>Why net length is insufficient</h2><p>A pinless wall blocks a trace without changing net-distance or ratsnest-demand scores. Retained, barycentric, harmonic and naive junction proposals keep it blocked. Random starts escape. Two hand-selected ±0.6 mm shifts open a passage; these are diagnostic oracles, not an automatic pressure algorithm.</p><p><a href="movable-wall--retained.svg">Blocked wall</a> · <a href="movable-wall--pressure-shift-+1.svg">Hand-shifted wall</a> · <a href="crossed-resistors--retained.svg">Crossed resistors</a> · <a href="crossed-resistors--junction-reinsert.svg">Junction-derived resistor placement</a></p>'''
    (root/'overview.html').write_text(overview)

if __name__=='__main__':main()
