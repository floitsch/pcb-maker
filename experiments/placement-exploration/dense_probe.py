#!/usr/bin/env python3
# Copyright (C) 2026 Toit contributors.
"""Paired dense routing probes of retained full-connectivity placement seeds.

Positions were computed using all 52 connections. Each routing probe starts
from zero copper and selects the first 19 connections. This tests fixed seeds,
not cold placement computed from a pruned nineteen-connection netlist.
"""
import argparse
import concurrent.futures
import hashlib
import json
from pathlib import Path
import shutil
import subprocess
import time

ROOT=Path(__file__).resolve().parents[2]


def sha(path):
    return hashlib.sha256(path.read_bytes()).hexdigest()


def main():
    parser=argparse.ArgumentParser()
    parser.add_argument('output',type=Path)
    parser.add_argument('--seed', action='append', default=[], metavar='NAME=DIRECTORY',
                        help='Probe a prepared problem.json / declared placement.json pair; repeat for multiple seeds')
    args=parser.parse_args(); out=args.output.resolve()
    assert not out.exists(),'Use a fresh output directory to retain previous results'
    out.mkdir(parents=True)
    old=ROOT/'build/algorithm-exploration-2026-09-07/placement'
    seeds={
        'harmonic':old/'native-prefix03-followups/harmonic-production-default-floor',
        'junction':old/'native-prefix03-followups/junction-constraint-aware',
        'retained':old/'native-prefix03/retained',
    }
    if args.seed:
        seeds = {}
        for value in args.seed:
            name, separator, directory = value.partition('=')
            assert separator and name and Path(name).name == name and name not in seeds, value
            seeds[name] = Path(directory).resolve()
    executable=out/'pcb-maker'
    shutil.copy2(ROOT/'target/release/pcb-maker',executable)
    manifest={'scope':__doc__,'target_connections':19,'executable_sha256':sha(executable),
              'parallel_workers':2,'cases':[]}
    jobs=[]
    for name,source in seeds.items():
        problem=json.loads((source/'problem.json').read_text())
        poses=[{k:c.get(k,0) for k in ['id','position','rotation_degrees']} for c in problem['components']]
        pose_hash=hashlib.sha256(json.dumps(poses,sort_keys=True).encode()).hexdigest()
        for profile in ['blocked','open']:
            directory=out/f'{name}-{profile}'; directory.mkdir()
            for filename in ['problem.json','placement.json']:
                shutil.copy2(source/filename,directory/filename)
            settings=ROOT/f'benchmarks/esp32-pad-gaps/dual-prefix19-{profile}'
            shutil.copy2(settings/'routing-physical.json',directory/'routing.json')
            shutil.copy2(settings/'template.json',directory/'template.json')
            command=[str(executable),'solve-semantic-kicad-prefix',str(directory/'problem.json'),
                     str(directory/'placement.json'),str(directory/'template.json'),'19',str(directory/'run'),str(directory/'routing.json')]
            record={'id':directory.name,'placement':name,'profile':profile,'source':str(source),
                    'input_pose_sha256':pose_hash,'problem_sha256':sha(directory/'problem.json'),
                    'routing_sha256':sha(directory/'routing.json'),'template_sha256':sha(directory/'template.json'),
                    'command':command,'status':'prepared'}
            manifest['cases'].append(record); jobs.append((record,directory))
    def save():
        temporary=out/'manifest.tmp'
        temporary.write_text(json.dumps(manifest,indent=2));temporary.replace(out/'manifest.json')
    save()
    def run(job):
        record,directory=job;start=time.monotonic()
        with (directory/'run.log').open('w') as log:
            result=subprocess.run(record['command'],stdout=log,stderr=subprocess.STDOUT)
        return record,{'status':'terminal','exit_code':result.returncode,'wall_seconds':time.monotonic()-start}
    with concurrent.futures.ThreadPoolExecutor(max_workers=2) as pool:
        futures=[pool.submit(run,job) for job in jobs]
        for future in concurrent.futures.as_completed(futures):
            record,done=future.result();record.update(done);save()
            print(json.dumps({'case':record['id'],**done}),flush=True)
    manifest['executable_unchanged']=sha(executable)==manifest['executable_sha256'];save()
    assert manifest['executable_unchanged']
    print(f'All {len(jobs)} probes terminal. Native verification captures per-step previews automatically.',flush=True)
    with (out/'summary.log').open('w') as log:
        subprocess.run(['python3',str(Path(__file__).with_name('dense_summary.py')),str(out)],
                       stdout=log,stderr=subprocess.STDOUT,check=True)
    print(f'Checked comparison and playback: {out / "index.html"}',flush=True)


if __name__=='__main__': main()
