#!/usr/bin/env python3
# Copyright (C) 2026 Toit contributors.
"""Check KiCad-exported DSN class geometry and net/pin inventory against native input.

This checks the basic per-net geometry contract, not arbitrary DSN constraints,
custom KiCad rules, or equivalence of every footprint shape.
"""
import argparse
import collections
import json
from pathlib import Path
import re
import pcbnew

from compile_net_classes import compile_rules
from run import digest, write


def parse_dsn(source):
    # Specctra's quote declaration is intentionally not a quoted S-expression.
    assert source.count('(string_quote ")') == 1
    source = source.replace('(string_quote ")', '(string_quote double_quote)', 1)
    return parse_sexpr(source)


def parse_sexpr(source):
    """Read an ordinary KiCad/DSN S-expression without changing atom spelling."""
    token = re.compile(r'"(?:[^"\\]|\\.)*"|[^\s()"]+|[()]')
    stack, root, last = [], [], 0
    for match in token.finditer(source):
        assert not source[last:match.start()].strip(), 'Unsupported S-expression token'
        last = match.end()
        word = match.group()
        if word == '(':
            child = []
            (stack[-1] if stack else root).append(child)
            stack.append(child)
        elif word == ')':
            assert stack
            stack.pop()
        else:
            assert stack
            stack[-1].append(json.loads(word) if word.startswith('"') else word)
    assert not source[last:].strip() and not stack and len(root) == 1
    return root[0]


def children(node, head):
    return [x for x in node if isinstance(x,list) and x and x[0] == head]


def child(node, head):
    values = children(node,head)
    assert len(values) == 1, f'Expected exactly one {head}'
    return values[0]


def audit(board_path, dsn_path):
    rules, native = compile_rules(board_path)
    tree = parse_dsn(dsn_path.read_text())
    assert tree[0] == 'pcb' and child(tree,'unit')[1:] == ['um']
    assert child(tree,'resolution')[1:] == ['um','10']
    assert child(tree,'wiring') == ['wiring'], 'Inherited routing in DSN'
    network, library = child(tree,'network'), child(tree,'library')
    structure = child(tree,'structure')
    layers = {x[1] for x in children(structure,'layer')}
    assert len(layers) == 2 and not children(structure,'plane')
    padstacks = {p[1]:p for p in children(library,'padstack')}
    actual, classes = {}, []
    for cls in children(network,'class'):
        nets = [x for x in cls[2:] if isinstance(x,str)]
        rule = child(cls,'rule')
        width, clearance = float(child(rule,'width')[1])/1000, float(child(rule,'clearance')[1])/1000
        via = child(child(cls,'circuit'),'use_via')[1:]
        assert len(via) == 1
        # KiCad encodes the imported drill in its via-padstack identifier.
        match = re.fullmatch(r'Via\[0-1\]_(\d+(?:\.\d+)?):(\d+(?:\.\d+)?)_um',via[0])
        assert match, 'Unsupported via exchange identifier'
        diameter, drill = (float(v)/1000 for v in match.groups())
        shapes = [child(s,'circle') for s in children(padstacks[via[0]],'shape')]
        assert len(shapes) == 2 and {s[1] for s in shapes} == layers
        assert all(float(s[2])/1000 == diameter for s in shapes)
        geometry = dict(trace_width_mm=width,clearance_mm=clearance,
                        via_size_mm=diameter,via_drill_mm=drill)
        for name in nets:
            key = name.removeprefix('/')
            assert key not in actual, 'Duplicate/ambiguous class membership'
            assert native['assignments'][key]['native_name'] == name
            actual[key] = geometry
        classes.append({'dsn_class':cls[1],'nets':nets,'geometry':geometry})
    assert actual == rules, 'DSN class geometry differs from native effective classes'
    board = pcbnew.LoadBoard(str(board_path))
    assert board.GetCopperLayerCount() == 2 and not list(board.GetTracks())
    assert not any(not zone.GetIsRuleArea() for zone in board.Zones())
    pins = collections.defaultdict(list)
    for fp in board.GetFootprints():
        for pad in fp.Pads():
            name = str(pad.GetNetname())
            if name:pins[name].append(str(fp.GetReference())+'-'+str(pad.GetNumber()))
    exported = {}
    for net in children(network,'net'):
        assert net[1] not in exported
        # KiCad's exporter suffixes repeated pad numbers (U1-39@1, ...) and
        # quotes references with special characters ("USB-C1"-A7), which the
        # tokenizer above splits into '"USB-C1"' and '-A7'.
        tokens = child(net,'pins')[1:]
        joined = []
        for token in tokens:
            # A pin id never starts with '-': such a token continues the
            # (quoted) reference before it.
            if joined and token.startswith('-'):
                joined[-1] = joined[-1].strip('"') + token
            else:
                joined.append(token)
        exported[net[1]] = sorted({pin.split('@')[0].replace('"','') for pin in joined})
    # Compare distinct pad ids of nets with something to route.
    native = {k:sorted(set(v)) for k,v in pins.items()}
    native = {k:v for k,v in native.items() if len(v) > 1}
    exported = {k:v for k,v in exported.items() if len(v) > 1}
    assert exported == native, 'DSN net/pin inventory changed: ' + str(
        [(k, native.get(k), exported.get(k)) for k in set(native) | set(exported) if native.get(k) != exported.get(k)][:3])
    return {'scope':__doc__, 'source_board_sha256':digest(board_path),
            'source_project_sha256':digest(board_path.with_suffix('.kicad_pro')),
            'dsn_sha256':digest(dsn_path),'class_geometry_matches':True,
            'net_pin_inventory_matches':True,'zero_inherited_copper':True,
            'classes':classes,'nets':len(actual)}


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('board',type=Path);parser.add_argument('dsn',type=Path);parser.add_argument('output',type=Path)
    args = parser.parse_args()
    result = audit(args.board,args.dsn)
    write(args.output,result)
    print(json.dumps({'nets':result['nets'],'class_geometry_matches':True,'net_pin_inventory_matches':True}))


if __name__=='__main__':main()
