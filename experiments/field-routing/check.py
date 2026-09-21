#!/usr/bin/env python3
# Copyright (C) 2026 Toit contributors.
"""Recheck saved routes independently; reject deliberately corrupted examples."""
import hashlib
import json
from pathlib import Path
import sys

from run import validate

root=Path(sys.argv[1] if len(sys.argv)>1 else 'build/algorithm-exploration-2026-09-07/field/final')
data=json.loads((root/'results.json').read_text())
cases={c['name']:c for c in data['cases']}
for result in data['runs']:
    assert validate(cases[result['case']],result['routes'],result['neighbors'])==result['validation']
    assert not result['work'].get('iteration_budget',0)
    assert not result['work'].get('flat_field',0)
for case in cases.values():
    if case['witness'] is not None: assert validate(case,case['witness'],4)['complete']

bad=[]
case=cases['single_layer_cross_impossible']
routes=[[(x,7,0) for x in range(21)],[(10,y,0) for y in range(15)]]
assert validate(case,routes,4)['errors'].get('shared_cell');bad.append('shared_cell')
case=dict(free=[(0,0,0),(1,1,0),(1,0,0),(0,1,0)],
          nets=[((0,0,0),(1,1,0)),((1,0,0),(0,1,0))])
assert validate(case,case['nets'],8)['errors'].get('diagonal_crossing');bad.append('diagonal_crossing')
case=dict(free=[(0,0,0),(1,1,0)],nets=[((0,0,0),(1,1,0))])
assert validate(case,case['nets'],8)['errors'].get('corner_cut');bad.append('corner_cut')
case=dict(free=[(0,0,0),(1,0,1)],nets=[((0,0,0),(1,0,1))])
assert validate(case,case['nets'],4)['errors'].get('invalid_via');bad.append('invalid_via')
case=dict(free=[(0,0,0),(2,0,0)],nets=[((0,0,0),(2,0,0))])
assert validate(case,[[(0,0,0),(1,0,0),(2,0,0)]],4)['errors'].get('obstacle');bad.append('obstacle')
assert validate(case,[[(0,0,0)]],4)['errors'].get('wrong_terminal');bad.append('wrong_terminal')
for layer in [0,1]:
    via=[(1,0,0),(1,1,0),(1,1,1),(1,2,1)]
    crossing=[(0,1,layer),(1,1,layer),(2,1,layer)]
    case=dict(free=list(set(via+crossing)),nets=[(via[0],via[-1]),(crossing[0],crossing[-1])])
    assert validate(case,[via,crossing],4)['errors'].get('shared_cell')
    bad.append('via_occupied_layer_'+str(layer))
case=dict(free=[(0,0,0),(1,0,0),(0,0,1),(1,0,1)],nets=[((0,0,0),(1,0,0))])
assert validate(case,[[(0,0,1),(1,0,1)]],4)['errors'].get('wrong_terminal')
bad.append('wrong_terminal_layer')

# The feedback-only variants update all nets against identical prices. Shuffling
# evaluation order does not change paths; these are deterministic repeats.
for method in ['field_feedback','shortest_feedback']:
    for case in cases:
        for neighbors in [4,8]:
            rows=[r for r in data['runs'] if r['method']==method and r['case']==case and r['neighbors']==neighbors]
            assert all(r['routes']==rows[0]['routes'] for r in rows)

report=dict(rechecked_runs=len(data['runs']),witnesses=3,corruptions_rejected=bad,
            all_reachable_field_solves_converged=True,
            scripts={p.name:hashlib.sha256(p.read_bytes()).hexdigest() for p in Path(__file__).parent.glob('*.py')},
            scope='Independent discrete geometry; no physical trace width, clearance, or native KiCad admission.')
(root/'validation.json').write_text(json.dumps(report,indent=2))
print(json.dumps(report,indent=2))
