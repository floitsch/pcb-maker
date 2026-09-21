#!/usr/bin/env python3
# Copyright (C) 2026 Toit contributors.
"""Compare passage policies and automatically retain semantic/native renders."""
import argparse
import hashlib
import json
from pathlib import Path
import subprocess
import sys

parser=argparse.ArgumentParser(); parser.add_argument('output',type=Path); args=parser.parse_args()
root=Path(__file__).resolve().parents[2]; exe=root/'target/release/pcb-maker'
assert not args.output.exists(),'Use a fresh output directory'
args.output.mkdir(parents=True)
problem=root/'benchmarks/small/passage-pressure-occupied.json'
routing=json.loads((root/'experiments/configs/dut-grid-adaptive.json').read_text())
routing.update(grid_mm=0.1,retry_grid_mm=[],allow_vias=False)
(args.output/'routing.json').write_text(json.dumps(routing,indent=2))
rows=[]
for name,policy in [('control','passage_capacity'),('occupied','passage_capacity_with_copper')]:
    config=json.loads((root/'experiments/configs/blocker-passage-repair.json').read_text()); config['direction_policy']=policy
    cfg=args.output/(name+'-config.json');cfg.write_text(json.dumps(config,indent=2))
    result=args.output/(name+'.json')
    with (args.output/(name+'.log')).open('w') as log:
        process=subprocess.run([str(exe),'repair-route-pressure',str(problem),'declared',str(result),str(args.output/'routing.json'),str(cfg)],stdout=log,stderr=subprocess.STDOUT)
        assert result.exists(),'No candidate artifact; inspect the retained log'
        for words in [['view-route',str(problem),str(result),str(args.output/(name+'.html'))],
                      ['render-route-layers',str(problem),str(result),str(args.output/name)]]:
            subprocess.run([str(exe),*words],stdout=log,stderr=subprocess.STDOUT,check=True)
    data=json.loads(result.read_text())
    assert data['validation']['complete']==(name=='occupied')
    rows.append(dict(policy=policy,exit_code=process.returncode,complete=data['validation']['complete'],
                     routed=data['evidence']['routing']['routed_branches'],attempts=data['evidence']['attempted_repairs'],
                     expansions=data['evidence']['total_expansions']))
(args.output/'summary.json').write_text(json.dumps({'problem_sha256':hashlib.sha256(problem.read_bytes()).hexdigest(),
    'executable_sha256':hashlib.sha256(exe.read_bytes()).hexdigest(),'runs':rows},indent=2))
subprocess.run([sys.executable,str(Path(__file__).with_name('promote.py')),str(args.output/'occupied.json'),str(args.output/'native'),
                '--problem',str(problem)],check=True)
print(json.dumps(rows,indent=2))
