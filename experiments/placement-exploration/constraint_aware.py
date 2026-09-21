#!/usr/bin/env python3
# Copyright (C) 2026 Toit contributors.
"""Retained follow-up: do not shrink components whose constraints use their shape."""
import hashlib
import html
import json
import subprocess
import sys
import run

def main():
    floor='--connected-floor' in sys.argv
    p=run.fixtures()['dual-esp32'];out=run.OUT/('constraint-aware-with-floor' if floor else 'constraint-aware');out.mkdir(exist_ok=True)
    policy={'kind':'harmonic_ports','iterations':128,'legalization_sweeps':256,'maximum_pair_checks':1000000,'connected_pair_spacing_floor':floor}
    jobs=[{'label':'junction-constraint-aware','problem':p,'config':{'policy':policy,'seed':0,'projection_sweeps':512},'junction':True,'constraint_aware':True}]
    for seed in range(0 if floor else 8):
        jobs.append({'label':f'junction-constraint-aware-random-{seed:02}','problem':p,'config':{'policy':dict(policy,underanchored_seed={'kind':'random','attempts':1}),'seed':seed,'projection_sweeps':512},'junction':True,'constraint_aware':True})
    if not floor:
        jobs.append({'label':'harmonic-production-default-floor','problem':p,'config':{'policy':dict(policy,connected_pair_spacing_floor=True),'seed':0,'projection_sweeps':512},'junction':False})
    requests=''.join(json.dumps(j)+'\n' for j in jobs);(out/'requests.jsonl').write_text(requests)
    output=subprocess.run([str(run.EXE)],input=requests,capture_output=True,text=True,check=True).stdout
    (out/'raw-results.jsonl').write_text(output);results=[json.loads(line) for line in output.splitlines()];assert len(results)==len(jobs)
    constraint_text=json.dumps(p.get('placement_constraints',[]));shrunk={c['id'] for c in p['components'] if c.get('constraints',{}).get('movement','free')=='free' and not c.get('constraints',{}).get('region') and json.dumps(c['id']) not in constraint_text}
    records=[];frames=[]
    for j,r in zip(jobs,results):
        poses=r['result']['poses'] if r['ok'] else run.poses_of(p)
        record={'label':j['label'],**r,'shrunk_components':sorted(shrunk) if j['junction'] else []}
        if r['ok']:
            record['independent_pose_validation']=run.independent_pose_check(p,poses);assert record['independent_pose_validation']['passed']
        drawing=run.svg(p,poses,j['label']+(': legal full-size' if r['ok'] else ': rejected; input shown'))
        (out/f'{j["label"]}.svg').write_text(drawing)
        if r['junctions']:
            seed=run.svg(p,r['junctions'],j['label']+': junction seed (not admitted at full size)',junction=shrunk)
            (out/f'{j["label"]}--junctions.svg').write_text(seed);frames.append({'label':j['label']+' seed','svg':seed})
        frames.append({'label':j['label'],'svg':drawing});records.append(record)
        print(j['label'],r['ok'],r.get('error'),flush=True)
    (out/'results.json').write_text(json.dumps({'scope':__doc__,'executable_sha256':hashlib.sha256(run.EXE.read_bytes()).hexdigest(),'records':records},indent=2)+'\n')
    doc='''<!doctype html><meta charset="utf-8"><title>Constraint-aware reinsertion</title><style>body{background:#101923;color:white;font:16px system-ui}svg{height:80vh;width:100%}</style><h1>Constraint-aware junction reinsertion</h1><p>Only unconstrained free bodies are shrunk. Each seed is reinserted with full bodies and pads by the production projector. This cycles proposals, not physical motion.</p><button onclick="i=(i+1)%f.length;show()">Next</button> <button onclick="playing=!playing">Play / pause</button><span id="label"></span><div id="view"></div><script>const f=FRAMES;let i=0,playing=false;function show(){document.getElementById('label').textContent=f[i].label;document.getElementById('view').innerHTML=f[i].svg;}setInterval(()=>{if(playing){i=(i+1)%f.length;show()}},1100);show();</script>'''.replace('FRAMES',json.dumps(frames).replace('</','<\\/'))
    (out/'index.html').write_text(doc)

if __name__=='__main__':main()
