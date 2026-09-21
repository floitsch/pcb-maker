#!/usr/bin/env python3
# Copyright (C) 2026 Toit contributors.
"""Native three-connection probe of retained full-connectivity placement seeds.

These placements were generated using all 52 electrical connections. Routing
then prunes to the same first three connections and preserves the proposed
poses via the declared policy. This is not a cold prefix placement comparison.
"""
import concurrent.futures
import copy
import hashlib
import json
from pathlib import Path
import subprocess
import sys
import run

def main():
    followups='--followups' in sys.argv
    out=run.OUT/('native-prefix03-followups' if followups else 'native-prefix03');out.mkdir(exist_ok=True)
    data=json.loads((run.OUT/'results.json').read_text())
    records=[r for r in data['records'] if r['fixture']=='dual-esp32' and r['ok']]
    if followups:
        records=[]
        for folder in ['constraint-aware','constraint-aware-with-floor']:
            records.extend(r for r in json.loads((run.OUT/folder/'results.json').read_text())['records'] if r['ok'])
    base=json.loads((run.OUT/'dual-esp32.problem.json').read_text())
    executable=run.ROOT/'target/release/pcb-maker'
    manifest={'scope':__doc__,'executable_sha256':hashlib.sha256(executable.read_bytes()).hexdigest(),'cases':[]}
    def case(record):
        label=record['label'];d=out/label;d.mkdir(exist_ok=True);p=copy.deepcopy(base);poses={x['component']:x for x in record['result']['poses']}
        for c in p['components']:
            c['position']=poses[c['id']]['position'];c['rotation_degrees']=poses[c['id']]['rotation_degrees']
        (d/'problem.json').write_text(json.dumps(p,indent=2)+'\n')
        (d/'placement.json').write_text(json.dumps({'policy':{'kind':'declared'},'seed':0,'projection_sweeps':1024})+'\n')
        command=[str(executable),'solve-semantic-kicad-prefix',str(d/'problem.json'),str(d/'placement.json'),str(run.ROOT/'benchmarks/esp32-pad-gaps/dual-prefix03-open/template.json'),'3',str(d/'run'),str(run.ROOT/'benchmarks/esp32-pad-gaps/dual-prefix03-open/routing-physical.json')]
        with (d/'run.log').open('w') as log:
            result=subprocess.run(command,stdout=log,stderr=subprocess.STDOUT)
        item={'label':label,'returncode':result.returncode,'directory':str(d),'command':command}
        print(json.dumps(item),flush=True);return item
    with concurrent.futures.ThreadPoolExecutor(max_workers=2) as pool:
        manifest['cases']=list(pool.map(case,records))
    manifest['executable_unchanged']=manifest['executable_sha256']==hashlib.sha256(executable.read_bytes()).hexdigest()
    (out/'manifest.json').write_text(json.dumps(manifest,indent=2)+'\n')
if __name__=='__main__':main()
