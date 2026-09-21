#!/usr/bin/env python3
# Copyright (C) 2026 Toit contributors.
"""Native check of a saved isolated route against retained copper, with/without yielding.

This probes target feasibility only. A yielded connection remains unrouted and
must be repaired before the board could be admitted as a routing improvement.
"""
import argparse
import hashlib
import json
from pathlib import Path
import shutil
import subprocess
import time


def main():
    parser=argparse.ArgumentParser(description=__doc__)
    parser.add_argument('board',type=Path);parser.add_argument('candidate',type=Path)
    parser.add_argument('yielding');parser.add_argument('output',type=Path)
    args=parser.parse_args();out=args.output.resolve();out.mkdir(parents=True,exist_ok=False)
    repo=Path(__file__).resolve().parents[2];exe=out/'pcb-maker';shutil.copy2(repo/'target/release/pcb-maker',exe)
    source=args.board.resolve();candidate=args.candidate.resolve();board_id=source.stem
    data=json.loads(candidate.read_text());assert not data.get('footprint_placements')
    manifest={'scope':__doc__,'source':str(source),'source_sha256':hashlib.sha256(source.read_bytes()).hexdigest(),
              'candidate':str(candidate),'candidate_sha256':hashlib.sha256(candidate.read_bytes()).hexdigest(),
              'executable_sha256':hashlib.sha256(exe.read_bytes()).hexdigest(),'cases':[]}
    def save():(out/'manifest.json').write_text(json.dumps(manifest,indent=2)+'\n')
    save()
    for label,yielding in [('retained',False),('yielded',True)]:
        case=out/label;case.mkdir()
        for entry in source.parent.iterdir():
            if entry.name=='libs' and entry.is_dir():shutil.copytree(entry,case/entry.name)
            elif entry.is_file() and (entry.suffix in ['.kicad_pcb','.kicad_sch','.kicad_pro','.kicad_sym'] or entry.name in ['fp-lib-table','sym-lib-table']):shutil.copy2(entry,case/entry.name)
        copied=case/source.name
        yielding_path=case/'yielding-connections.json'
        yielding_path.write_text(json.dumps([args.yielding] if yielding else [])+'\n')
        preapply=out/f'{label}-before-application.kicad_pcb';shutil.copy2(copied,preapply)
        apply=[str(exe),'apply-kicad-route-candidate',str(preapply),str(candidate),str(copied),str(yielding_path)]
        subprocess.run(apply,capture_output=True,text=True,check=True)
        command=[str(exe),'verify-kicad-rung',str(case),board_id]
        record={'case':label,'yielded_connection':args.yielding if yielding else None,'source_preapply_sha256':hashlib.sha256(preapply.read_bytes()).hexdigest(),
                'apply_command':apply,'verify_command':command,'status':'running'};manifest['cases'].append(record);save()
        start=time.monotonic()
        with (case/'verify.log').open('w') as log:
            completed=subprocess.run(command,stdout=log,stderr=subprocess.STDOUT)
        record.update(status='terminal',exit_code=completed.returncode,seconds=time.monotonic()-start)
        record['verification']=json.loads((case/'verification.json').read_text());assert (case/'preview.svg').exists()
        subprocess.run(['rsvg-convert','--width','1100','--background-color','white','--output',str(case/'preview.png'),str(case/'preview.svg')],check=True)
        save();print(json.dumps(record),flush=True)
    assert hashlib.sha256(source.read_bytes()).hexdigest()==manifest['source_sha256']
    manifest['source_unchanged']=True;save()


if __name__=='__main__':main()
