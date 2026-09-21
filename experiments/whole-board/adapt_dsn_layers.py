# Copyright (C) 2026 Toit contributors.
"""Make each empty-board DSN copper layer available for track routing.

KiCad signal/power/mixed/jumper designations are descriptive; they do not prohibit
tracks. The native-project backend uses this mapping to preserve those routing
semantics. The standalone comparison enables it explicitly. Native board/project
metadata is untouched. Planes and existing wiring are rejected; geometry, stack
order, net rules and every other DSN node are kept.
"""
import copy
from pathlib import Path
import re

from audit_dsn_classes import parse_dsn,child,children
from run import digest


def all_signal(source,output):
    assert not output.exists()
    text=source.read_text();tree=parse_dsn(text)
    structure=child(tree,'structure')
    layers=children(structure,'layer')
    assert len(layers)==2 and not children(structure,'plane'), 'Requires an empty two-layer routing input'
    assert child(tree,'wiring')==['wiring']
    before=[dict(name=l[1],role=child(l,'type')[1]) for l in layers]
    assert all(row['role'] in ['signal','power','mixed','jumper'] for row in before)
    expected=copy.deepcopy(tree)
    for layer in children(child(expected,'structure'),'layer'):
        child(layer,'type')[1]='signal'
    pattern=r'(\(layer\s+(?:"(?:[^"\\]|\\.)*"|[^\s()"]+)\s+\(type\s+)(signal|power|mixed|jumper)(\))'
    rewritten,count=re.subn(pattern,lambda m:m[1]+'signal'+m[3],text)
    assert count==len(layers) and parse_dsn(rewritten)==expected, 'Layer adaptation changed unrelated content'
    output.write_text(rewritten)
    return dict(scope=__doc__,policy='all_copper_signal',source_dsn_sha256=digest(source),
        adapted_dsn_sha256=digest(output),source_layers=before,
        adapted_layers=[dict(name=row['name'],role='signal') for row in before],
        source_layer_roles_changed=any(row['role']!='signal' for row in before),
        geometry_stack_and_net_rules_unchanged=True)
