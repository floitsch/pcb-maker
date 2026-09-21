# Copyright (C) 2026 Toit contributors.
"""Translate a native global copper-edge rule into pinned Freerouting DSN.

Only an explicit global edge rule representable by the selected outline mode
and 100 nm grid is supported. Outside-area mode uses the exact exterior area.
Source geometry, net classes and all non-edge clearance matrix entries must
remain identical. Full rule equivalence is not
claimed by this translation.
"""
import argparse
import copy
from decimal import Decimal
from pathlib import Path
import re

from audit_dsn_classes import parse_dsn, child, children
from inspect_dsn_geometry import inspect
from run import digest, read, write

EDGE_CLASS='pcbmakeredge'


def scope_end(source, wanted):
    # Mask KiCad's bare quote declaration without changing character offsets.
    source=source.replace('(string_quote ")','(string_quote Q)')
    tokens=re.finditer(r'"(?:[^"\\]|\\.)*"|[^\s()"]+|[()]',source)
    stack=[]
    ends=[]
    for token in tokens:
        word=token.group()
        if word=='(':
            stack.append(None)
        elif word==')':
            if stack==wanted: ends.append(token.start())
            stack.pop()
        elif stack and stack[-1] is None:
            stack[-1]=word
    assert len(ends)==1, f'Unsupported scope multiplicity: {wanted}'
    return ends[0]


def translate(dsn, project, raw, output):
    assert not output.exists()
    custom=project.with_suffix('.kicad_dru')
    assert not custom.exists() or not custom.read_text().strip(), 'Custom edge rules require separate lowering'
    native=Decimal(str(read(project)['board']['design_settings']['rules']['min_copper_edge_clearance']))
    assert native.is_finite() and native>=0
    assert digest(dsn)==raw['dsn_sha256'], 'Stale external inspection'
    assert digest(project)==raw['source_project_sha256'], 'Stale native project inspection'
    loaded=read(Path(raw['raw_path']))
    assert loaded['unit']=='um' and loaded['resolution']==10 and len(loaded['board_edges'])==1
    border=loaded['board_edges'][0]
    half=Decimal(0) if border.get('outside_area',False) else Decimal(str(border['half_width_dsn']))/1000
    clearance=native-half
    assert clearance>=0, 'Native edge clearance is smaller than the external outline half-width'
    assert clearance*10000==(clearance*10000).to_integral_value(), 'Native edge clearance is off the external 100 nm grid'
    names=sorted({v['first'] for v in loaded['clearance_matrix']} - {'none'})
    assert EDGE_CLASS not in names, 'Edge translation already present'
    assert all(re.fullmatch(r'[A-Za-z][A-Za-z0-9_]*',n) for n in names), 'Unsupported external clearance class name'
    source=dsn.read_text()
    tree=parse_dsn(source)
    assert child(tree,'unit')[1:]==['um'] and child(tree,'resolution')[1:]==['um','10']
    structure=child(tree,'structure')
    boundary=child(structure,'boundary')
    assert not children(boundary,'clearance_class'), 'Existing boundary rule requires separate lowering'
    assert len(boundary)==2 and boundary[1][0] in ['path','rect','polygon'], 'Unsupported boundary geometry'
    expected=copy.deepcopy(tree)
    es=child(expected,'structure')
    child(es,'boundary').append(['clearance_class',EDGE_CLASS])
    number=format(clearance*1000,'f')
    # Outside mode applies this dedicated diagonal to its boundary row only
    # after native net classes have initialized in the shared Java loader.
    pairs=[EDGE_CLASS+'_'+EDGE_CLASS] if border.get('outside_area',False) else [EDGE_CLASS+'_'+n for n in names+[EDGE_CLASS]]
    child(es,'rule').append(['clearance',number,['type',*pairs]])
    insertions=[(scope_end(source,['pcb','structure','boundary']),f'  (clearance_class {EDGE_CLASS})\n    '),
                (scope_end(source,['pcb','structure','rule']),f'  (clearance {number} (type {" ".join(pairs)}))\n    ')]
    for offset,text in sorted(insertions,reverse=True):
        source=source[:offset]+text+source[offset:]
    assert parse_dsn(source)==expected, 'Translation changed unrelated DSN content'
    output.write_text(source)
    return dict(source_dsn_sha256=digest(dsn),translated_dsn_sha256=digest(output),
        source_project_sha256=digest(project),native_edge_mm=float(native),
        outline_half_width_mm=float(half),loaded_outline_half_width_mm=float(Decimal(str(border['half_width_dsn']))/1000),
        matrix_edge_clearance_mm=float(clearance),
        new_clearance_class=EDGE_CLASS,unchanged_geometry_and_net_rules=True)


def translate_and_audit(dsn,native_directory,board_id,output,jar,java_bin,outside_outline=False):
    output.mkdir(exist_ok=False)
    before=inspect(dsn,native_directory,board_id,output/'before',jar,java_bin,outside_outline)
    before['raw_path']=str(output/'before/raw.json')
    translated=output/'translated.dsn'
    receipt=translate(dsn,native_directory/(board_id+'.kicad_pro'),before,translated)
    after=inspect(translated,native_directory,board_id,output/'after',jar,java_bin,outside_outline)
    a=read(output/'before/raw.json'); b=read(output/'after/raw.json')
    matrix=lambda raw:{(r['first'],r['second']):r['clearance_per_layer_dsn'] for r in raw['clearance_matrix']
                        if EDGE_CLASS not in [r['first'],r['second']]}
    assert matrix(a)==matrix(b), 'Translation changed non-edge clearances'
    assert a['item_count']==b['item_count'], 'Translation changed board inventory'
    assert a['net_classes']==b['net_classes'], 'Translation changed native class assignments'
    assert a['copper_item_clearance_classes']==b['copper_item_clearance_classes'], 'Translation changed copper item clearance classes'
    assert not after['edge_requirement_mismatch_observed'], 'Loaded external edge rule does not match native rule'
    receipt.update(outside_outline=outside_outline,non_edge_clearances_unchanged=True,loaded_edge_rule_matches=True,
        before_violations=before['unique_violation_count'],after_violations=after['unique_violation_count'],
        before_inspection_sha256=digest(output/'before/report.json'),after_inspection_sha256=digest(output/'after/report.json'))
    write(output/'report.json',receipt)
    return receipt


if __name__=='__main__':
    parser=argparse.ArgumentParser(description=__doc__)
    parser.add_argument('dsn',type=Path)
    parser.add_argument('native_directory',type=Path)
    parser.add_argument('board_id')
    parser.add_argument('output',type=Path)
    parser.add_argument('--jar',type=Path,default=Path('/tmp/freerouting-2.2.4.jar'))
    parser.add_argument('--java-bin',type=Path,default=Path('/usr/lib/jvm/java-26-openjdk/bin'))
    parser.add_argument('--outside-outline',action='store_true')
    args=parser.parse_args()
    print(translate_and_audit(args.dsn.resolve(),args.native_directory.resolve(),args.board_id,
          args.output.resolve(),args.jar.resolve(),args.java_bin.resolve(),args.outside_outline))
