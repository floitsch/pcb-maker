#!/usr/bin/env python3
# Copyright (C) 2026 Toit contributors.
"""Planted placement faults on the retained native-courtyard human control.

These are placement-model experiments; original native source files are never
changed. Every case gets an independent audit, rendering and production check.
"""
import argparse
import copy
from pathlib import Path

from area_probe import render
from run import command, digest, read, write


def main():
    parser=argparse.ArgumentParser(description=__doc__)
    parser.add_argument('study',type=Path);parser.add_argument('output',type=Path)
    args=parser.parse_args();args.output.mkdir(parents=True,exist_ok=False)
    base=read(args.study/'human-control-problem.json');exe=args.study/'pcb-maker'
    def modified(ref):
        problem=copy.deepcopy(base)
        return problem,next(c for c in problem['components'] if c['id']==ref)
    cases=[('human-positive',base,True)]
    p,c=modified('U1');c['position']['x']-=8
    cases.append(('socket-body-collision',p,False))
    p,c=modified('P4');c['placement_geometry']['board_overhang']=0
    cases.append(('forbidden-connector-overhang',p,False))
    p,c=modified('P2');c['placement_geometry']['clearance']=0.4
    cases.append(('double-counted-body-clearance',p,False))
    p,c=modified('P4');c['pins'][0]['offset']['y']+=10
    cases.append(('pad-outside-despite-body-allowance',p,False))
    p,c=modified('P2')
    other=next(x for x in p['components'] if x['id']=='U1')
    c['pins'][0]['offset']={a:other['position'][a]-c['position'][a] for a in ['x','y']}
    cases.append(('external-pad-hits-socket',p,False))
    report={'scope':__doc__,'executable_sha256':digest(exe),'cases':[]}
    for name,problem,expected in cases:
        directory=args.output/name;directory.mkdir()
        poses=[{'component':c['id'],'position':c['position'],'rotation_degrees':0} for c in problem['components']]
        independent=render(problem,poses,directory/'preview.svg',name)
        write(directory/'problem.json',problem)
        write(directory/'config.json',{'policy':{'kind':'declared'},'projection_sweeps':16})
        process=command([exe,'place',directory/'problem.json',directory/'config.json',directory/'placement.json'],directory/'place.log')
        row={'name':name,'expected_legal':expected,'production_legal':process['exit_code']==0,'independent':independent,'process':process}
        report['cases'].append(row)
        write(args.output/'report.json',report)
        assert row['production_legal']==expected,name
        assert independent['passed']==expected,name
    report['passed']=True;write(args.output/'report.json',report)
    (args.output/'index.html').write_text('<!doctype html><meta charset="utf-8"><h1>Planted courtyard controls</h1>'+
        ''.join(f'<h2>{r["name"]}: production legal={r["production_legal"]}</h2><img style="width:750px" src="{r["name"]}/preview.svg">' for r in report['cases'])+
        '<p><a href="report.json">Results and scope</a></p>')
    print('All six expected production outcomes matched.')


if __name__=='__main__':main()
