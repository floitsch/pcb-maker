#!/usr/bin/env python3
# Copyright (C) 2026 Toit contributors.
"""Replay and render production orientation refinement evidence at fixed centers."""
import copy
import itertools
import json
from pathlib import Path
import subprocess
import sys
import xml.etree.ElementTree as ET

sys.path.insert(0,str(Path(__file__).resolve().parents[1]/'placement-exploration'))
import run as geometry


def main():
    baseline, refined = map(Path,sys.argv[1:])
    problem=json.loads((baseline/'problem.json').read_text())
    assert problem==json.loads((refined/'problem.json').read_text())
    before=json.loads((baseline/'placement.json').read_text())
    after=json.loads((refined/'placement.json').read_text())
    evidence=after['evidence']['orientation_refinement']
    poses={p['component']:copy.deepcopy(p) for p in before['poses']}
    frames=[]; rejected=[]; cost=evidence['cost_before']
    def frame(title):
        svg=geometry.svg(problem,list(poses.values()),title);ET.fromstring(svg)
        ordinal=len(frames);name=f'orientation-{ordinal:02}.svg'
        (refined/name).write_text(svg)
        frames.append({'title':title,'svg':svg,'cost':cost,'file':name})
    frame('After legalization, before orientation refinement')
    for key, group in itertools.groupby(evidence['trials'],lambda t:(t['sweep'],t['component'])):
        group=list(group); accepted=[t for t in group if t['status']=='accepted'];assert len(accepted)<=1
        for t in group:
            assert poses[t['component']]['rotation_degrees']==t['from_degrees']
            assert abs(t['cost_before']-cost)<1e-8
            if t['status']=='rejected':rejected.append(t)
        if accepted:
            t=accepted[0];assert t['cost_after']<cost
            poses[t['component']]['rotation_degrees']=t['to_degrees'];cost=t['cost_after']
            assert geometry.independent_pose_check(problem,list(poses.values()))['passed']
            frame(f'Sweep {t["sweep"]+1}: {t["component"]} {t["from_degrees"]:g} → {t["to_degrees"]:g} degrees')
    assert poses=={p['component']:p for p in after['poses']}
    assert abs(cost-evidence['cost_after'])<1e-8
    assert len(frames)-1==evidence['accepted_changes']
    # Show rejected geometry explicitly, without admitting it into the replay.
    replay={p['component']:copy.deepcopy(p) for p in before['poses']}
    rejects=[]
    for key,group in itertools.groupby(evidence['trials'],lambda t:(t['sweep'],t['component'])):
        group=list(group)
        for t in group:
            if t['status']=='rejected' and len(rejects)<12:
                candidate=copy.deepcopy(replay);candidate[t['component']]['rotation_degrees']=t['to_degrees']
                name=f'rejected-{len(rejects):02}.svg'
                (refined/name).write_text(geometry.svg(problem,list(candidate.values()),f'REJECTED: {t["component"]} to {t["to_degrees"]:g} degrees'))
                rejects.append({**t,'preview':name})
        for t in group:
            if t['status']=='accepted':replay[t['component']]['rotation_degrees']=t['to_degrees']
    summary={'accepted_changes':len(frames)-1,'rejected_trials':len(rejected),'rejected_previews':rejects,
             'cost_before':evidence['cost_before'],'cost_after':cost,'termination':evidence['termination'],
             'same_problem':True,'exact_replay':True,'centers_unchanged':True,'frames':[{k:v for k,v in f.items() if k!='svg'} for f in frames]}
    (refined/'refinement-audit.json').write_text(json.dumps(summary,indent=2)+'\n')
    doc='''<!doctype html><meta charset="utf-8"><title>Production orientation refinement</title><style>body{font:16px system-ui;background:#101923;color:white;margin:24px}svg{width:100%;height:76vh}button{font:inherit}</style><h1>Orientation refinement after legalization</h1><p>Only legal improving rotations are committed. All component centers stay fixed. The score is a placement surrogate; these lines are connectivity, not copper.</p><button onclick="playing=!playing">Play / pause</button><button onclick="i=(i+1)%frames.length;show()">Next</button><p id="label"></p><div id="view"></div><script>const frames=DATA;let i=0,playing=false;function show(){document.getElementById('label').textContent=frames[i].title+' · attraction score '+frames[i].cost.toFixed(3);document.getElementById('view').innerHTML=frames[i].svg}setInterval(()=>{if(playing){i=(i+1)%frames.length;show()}},1000);show();</script>'''
    (refined/'refinement.html').write_text(doc.replace('DATA',json.dumps(frames).replace('</','<\\/')))
    print(json.dumps({k:v for k,v in summary.items() if k not in ['frames','rejected_previews']}))


if __name__=='__main__':main()
