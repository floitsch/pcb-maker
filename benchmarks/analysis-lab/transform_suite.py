#!/usr/bin/env python3
# Copyright (C) 2026 Toit contributors.
"""Private fixture transformation for fresh-agent transfer checks.

These preserve mechanisms; they are not independent held-out board families.
"""
import copy
import json
import random
from pathlib import Path
from generate import evaluate, write_json
from inspect_board import render


def transform(board, repair, rng, new_id):
    result=copy.deepcopy(board)
    repair=copy.deepcopy(repair)
    rotate=rng.choice([True,False])
    reflect=rng.choice([True,False])
    dx,dy=round(rng.uniform(.1,.9),3),round(rng.uniform(.1,.9),3)
    x0,y0,x1,y1=board['bounds']
    def point(p):
        x,y=p
        if reflect:
            x=x0+x1-x
        if rotate:
            x,y=y,x
        return [round(x+dx,6),round(y+dy,6)]
    def rect(r):
        points=[point([r[0],r[1]]),point([r[2],r[3]])]
        return [min(p[0] for p in points),min(p[1] for p in points),max(p[0] for p in points),max(p[1] for p in points)]
    mapping={}
    for objects,prefix in [(result['routes'],'R'),(result['pads'],'P'),(result['obstacles'],'O')]:
        numbers=rng.sample(range(100,999),len(objects))
        for obj,n in zip(objects,numbers):
            mapping[obj['id']]=prefix+str(n)
    result['id']=new_id
    result['bounds']=rect(board['bounds'])
    for pad in result['pads']:
        pad['id']=mapping[pad['id']]
        pad['at']=point(pad['at'])
    for obstacle in result['obstacles']:
        obstacle['id']=mapping[obstacle['id']]
        obstacle['rect']=rect(obstacle['rect'])
    for route in result['routes']:
        route['id']=mapping[route['id']]
        route['points']=[point(p) for p in route['points']]
    for replacement in repair['replacements']:
        replacement['route_id']=mapping[replacement['route_id']]
        replacement['points']=[point(p) for p in replacement['points']]
    for key in ['routes','pads','obstacles']:
        rng.shuffle(result[key])
    return result,repair


def main():
    source=Path('build/analysis-lab/harder')
    target=Path('build/analysis-lab/transfer')
    rng=random.Random(731904)
    paths=sorted((source/'public').glob('h*.json'))
    rng.shuffle(paths)
    manifest=[]
    for path,new_id in zip(paths,['t08','t21','t35','t47','t59','t72']):
        answer=json.loads((source/'private'/path.stem/'answer.json').read_text())
        board,repair=transform(json.loads(path.read_text()),answer['proposal'],rng,new_id)
        assessment=evaluate(board,repair)
        assert assessment['valid'],(new_id,assessment)
        assert not repair['replacements'] or assessment['improved'],(new_id,assessment)
        write_json(target/'public'/f'{new_id}.json',board)
        render(board,target/'public'/f'{new_id}.png')
        write_json(target/'private'/new_id/'answer.json',dict(source=path.stem,proposal=repair,assessment=assessment))
        manifest.append(dict(id=new_id,source=path.stem,assessment=assessment))
    write_json(target/'private/manifest.json',dict(seed=731904,cases=manifest,scope='transformed mechanism transfer, not independent holdout families'))
    write_json(target/'public/objective.json',json.loads((source/'public/objective.json').read_text()))
    print(target)


if __name__=='__main__':
    main()
