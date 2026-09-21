#!/usr/bin/env python3
# Copyright (C) 2026 Toit contributors.
"""Private evaluator and simple deterministic baseline. Not an agent tool."""
import argparse
import json
from pathlib import Path
from generate import evaluate, measurements, write_json


def baseline(board):
    """Try endpoint chords and single-layer rewrites, independently per route."""
    trials=[]
    for route in board['routes']:
        for layer in ['top','bottom']:
            candidates=[([route['points'][0],route['points'][-1]],[layer]),
                        (route['points'],[layer]*len(route['layers']))]
            for points,layers in candidates:
                if points==route['points'] and layers==route['layers']:
                    continue
                proposal=dict(replacements=[dict(route_id=route['id'],points=points,layers=layers)])
                if any(x['proposal']==proposal for x in trials):
                    continue
                assessment=evaluate(board,proposal)
                trials.append(dict(proposal=proposal,assessment=assessment))
    good=[t for t in trials if t['assessment']['improved']]
    selected=min(good,key=lambda t:t['assessment']['new']['cost_mm']) if good else None
    return dict(case_id=board['id'],trial_count=len(trials),selected=selected,trials=trials)


def score(public, answers):
    ids=[answer['case_id'] for answer in answers]
    if len(set(ids))!=len(ids):
        raise ValueError('Duplicate case IDs in submission')
    results=[]
    for answer in answers:
        board=json.loads((public/(answer['case_id']+'.json')).read_text())
        results.append(dict(case_id=board['id'],proposal=answer,assessment=evaluate(board,answer)))
    return dict(cases=results,improved=sum(r['assessment']['improved'] for r in results),
                invalid=sum(not r['assessment']['valid'] for r in results),
                abstained=sum(not r['proposal'].get('replacements') for r in results))


def main():
    p=argparse.ArgumentParser(description=__doc__)
    p.add_argument('command',choices=['score','baseline'])
    p.add_argument('--public',type=Path,default=Path('build/analysis-lab/public'))
    p.add_argument('--answers',type=Path)
    p.add_argument('--output',type=Path,required=True)
    args=p.parse_args()
    if args.command=='baseline':
        boards=[json.loads(path.read_text()) for path in sorted(args.public.glob('*.json'))]
        result=[baseline(board) for board in boards if 'routes' in board]
    else:
        result=score(args.public,json.loads(args.answers.read_text()))
    write_json(args.output,result)
    print(args.output)


if __name__=='__main__':
    main()
