#!/usr/bin/env python3
# Copyright (C) 2026 Toit contributors.
"""Recheck saved geometry, known lower bounds, ablations and corrupted outputs."""
import copy
import json
from pathlib import Path
import sys

from run import assess, body_distance, segment_distance

root=Path(sys.argv[1] if len(sys.argv)>1 else 'build/algorithm-exploration-2026-09-07')
cases={}
frames=0
for directory in ['engine-checked','engine-convergence']:
    for path in (root/directory).glob('*.json'):
        doc=json.loads(path.read_text())
        if not isinstance(doc,dict) or 'frames' not in doc: continue
        cases[doc['id']]=doc
        for state in doc['frames']:
            assert assess(doc,state)==state['independent_check']
            assert state['independent_check']['fixed_drift_mm']==0
            frames+=1
        assert doc['frames'][-1]['independent_check']['valid']
assert len(cases)==10
assert segment_distance((0,0),(2,2),(0,2),(2,0))==0
assert segment_distance((0,0),(2,0),(1,1),(3,1))==1
assert segment_distance((0,0),(1,0),(2,0),(3,0))==1
assert cases['slack-no-tension']['frames'][0]['routes']==cases['slack-no-tension']['frames'][-1]['routes']
assert all(a['routes']==b['routes'] for a,b in zip(cases['trace-contact']['frames'],cases['trace-contact-density']['frames']))
assert max(s['frame']['metrics']['max_field_pressure'] for s in cases['trace-contact-density-weighted']['frames'])>0
assert abs(cases['slack-settled']['frames'][-1]['length_mm']-26)<1e-4
# Triangle inequality: external endpoints 26 mm apart, rigid terminal span 4 mm,
# plus 1 mm total fixed stubs. The 23 mm bound is attained in this empty fixture.
assert abs(cases['coupled-settled']['frames'][-1]['length_mm']-23)<1e-4
corruptions=[]
doc=cases['obstacle-upper']; state=copy.deepcopy(doc['frames'][-1])
state['routes'][0]['points'][2]={'x':15.,'y':9.}
assert 'clearance' in assess(doc,state)['errors']; corruptions.append('obstacle penetration')
doc=cases['slack-settled']; state=copy.deepcopy(doc['frames'][-1]); state['routes'][0]['points'][0]['x']+=1
assert 'fixed geometry moved' in assess(doc,state)['errors']; corruptions.append('fixed endpoint moved')
doc=cases['coupled-settled']; state=copy.deepcopy(doc['frames'][-1]); state['routes'][0]['points'][-1]['y']+=1
assert 'attachment' in assess(doc,state)['errors']; corruptions.append('rigid attachment detached')
state=copy.deepcopy(doc['frames'][-1]); state['routes'][0]['points'][0]['y']+=1
assert 'shared endpoint detached' in assess(doc,state)['errors']; corruptions.append('shared external anchor detached')
report={'cases':len(cases),'rechecked_frames':frames,'corruptions_rejected':corruptions,
        'known_empty_fixture_lower_bounds_attained_mm':{'slack':26,'coupled_with_fixed_stubs':23},
        'scope':'Specified single-layer copper/body geometry, rigid and shared attachments; no swept-motion certificate, native KiCad or industrial DRC coverage.'}
(root/'engine-validation.json').write_text(json.dumps(report,indent=2))
print(json.dumps(report,indent=2))
