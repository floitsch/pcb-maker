#!/usr/bin/env python3
# Copyright (C) 2026 Toit contributors.
"""Independently audit legacy-branch attraction costs and production trial replay."""
import copy
import itertools
import json
import math
from pathlib import Path
import sys

sys.path.insert(0,str(Path(__file__).resolve().parents[1]/'placement-exploration'))
import run as geometry


def main():
    baseline, refined=map(Path,sys.argv[1:])
    problem=json.loads((baseline/'problem.json').read_text());assert not problem.get('electrical_nets')
    before=json.loads((baseline/'placement.json').read_text())
    after=json.loads((refined/'placement.json').read_text())
    poses={p['component']:copy.deepcopy(p) for p in before['poses']}
    components={c['id']:c for c in problem['components']}
    config=json.loads((refined/'config.json').read_text())['orientation_refinement']
    e=after['evidence']['orientation_refinement']
    def costs(poses):
        values={}
        for net in problem['nets']:
            endpoints=[]
            for key in ['from','to']:
                t=net[key];pin=next(p for p in components[t['component']]['pins'] if p['id']==t['pin'])
                allowed=set(net.get('allowed_layers',[]))|{net.get('layer','top')}
                offsets=[p.get('local_center',pin['offset']) for p in pin.get('pads',[]) if p['layer'] in allowed] or [pin['offset']]
                endpoints.append([geometry.transform(p,poses[t['component']]) for p in offsets])
            separation=min(math.dist(a,b)**2 for a in endpoints[0] for b in endpoints[1])
            values[net['id']]=separation*net['width']*net.get('tension_weight',1)
        return values
    def close(a,b): assert abs(a-b)<1e-8,(a,b)
    close(sum(costs(poses).values()),e['cost_before'])
    for key,group in itertools.groupby(e['trials'],lambda t:(t['sweep'],t['component'])):
        group=list(group);current=sum(costs(poses).values());accepted=[]
        for t in group:
            assert t['from_degrees']==poses[t['component']]['rotation_degrees']
            close(t['cost_before'],current)
            candidate=copy.deepcopy(poses);candidate[t['component']]['rotation_degrees']=t['to_degrees']
            cost=sum(costs(candidate).values());close(cost,t['cost_after'])
            epsilon=max(config.get('minimum_improvement',1e-9),abs(current)*1e-12)
            if t['status']=='no_improvement':assert current-cost<=epsilon
            else:assert current-cost>epsilon
            if t['status']=='accepted':accepted.append(t)
            if t['status']=='rejected':assert t['error']
        assert len(accepted)<=1
        if accepted:
            chosen=accepted[0]
            assert all(chosen['cost_after']<=t['cost_after'] for t in group if t['status']=='feasible_not_selected')
            poses[chosen['component']]['rotation_degrees']=chosen['to_degrees']
    assert poses=={p['component']:p for p in after['poses']}
    final=costs(poses);close(sum(final.values()),e['cost_after'])
    for row in e['net_costs_after']:close(final[row['connection']],row['weighted_cost'])
    assert len(e['trials'])<=config.get('maximum_trials',512)
    assert e['sweeps']<=config.get('maximum_sweeps',4)
    checks=sum(t['status']!='no_improvement' for t in e['trials']);assert checks==e['legality_checks']
    result={'all_trial_costs_independently_checked':len(e['trials']),'positive_trials':checks,
            'replayed_exactly':True,'best_legal_choice_per_component':True,'budgets_respected':True,
            'scope':'Independent Python scoring and sequential replay for the complete legacy-branch board. Production validators and native checks cover legality separately.'}
    (refined/'independent-score-audit.json').write_text(json.dumps(result,indent=2)+'\n');print(json.dumps(result))


if __name__=='__main__':main()
