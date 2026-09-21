# Copyright (C) 2026 Toit contributors.
"""Audit bounded pair displacement and restoration against its original parent."""
import argparse
from collections import Counter
import hashlib
import html
import json
import os
from pathlib import Path
import shutil

from audit_partial_ripup import audit as audit_single, isolated_snapshot
from report_adaptive_routing import finding_counts, progress_admissible
from render_inspection_layers import render
from sequential_evidence import audit_yielding_priority, receipt_rules


def read(path):
    return json.loads(Path(path).read_text())


def digest(path):
    return hashlib.sha256(Path(path).read_bytes()).hexdigest()


def directory_digest(root):
    result = hashlib.sha256(b'pcb-maker-kicad-directory-v1')
    files = sorted((p for p in root.rglob('*') if p.is_file()),
                   key=lambda p: os.fsencode(p.relative_to(root)))
    for path in files:
        assert not path.is_symlink()
        name, blob = os.fsencode(path.relative_to(root)), path.read_bytes()
        for part in [name,blob]:
            result.update(len(part).to_bytes(8,'little'))
            result.update(part)
    return result.hexdigest()


def candidate_geometry(candidate, native, net):
    """Compare the full emitted copper multiset, including coordinates/layers."""
    masks = native['copper_layer_masks']
    nm = lambda value: round(value*1e6)
    point = lambda values: tuple(map(nm,values))
    expected = Counter()
    for s in candidate['supplemental_segments']:
        assert s['connection'].removeprefix('/') == net
        expected[('segment',tuple(sorted([point(s['start']),point(s['end'])])),
                  nm(s['width']),masks[s['layer']])] += 1
    for v in candidate['supplemental_vias']:
        assert v['connection'].removeprefix('/') == net
        expected[('via',point(v['at']),nm(v['size']),nm(v['drill']),
                  sum(masks[layer] for layer in v['layers']))] += 1
    actual = Counter()
    for t in native['tracks'].values():
        if t['net'] != net:
            continue
        layer = int(t['layers'].replace('_',''),16)
        if 'drill' in t:
            assert t['start'] == t['end'] and t['width'] == t['back_width']
            actual[('via',tuple(t['start']),t['width'],t['drill'],layer)] += 1
        else:
            assert t['kind'] == 'PCB_TRACK'
            actual[('segment',tuple(sorted([tuple(t['start']),tuple(t['end'])])),
                    t['width'],layer)] += 1
    assert expected == actual, (net,list((expected-actual).items())[:2],
                               list((actual-expected).items())[:2])
    return sum(actual.values())


def audit(root, base_path, policy_path):
    root = root.resolve()
    report, base, policy = read(root/'compound.json'), read(base_path), read(policy_path)
    board_id, target = report['board_id'], report['target_connection'].removeprefix('/')
    name = board_id+'.kicad_pcb'
    source = Path(report['source_snapshot'])
    assert report['source_unchanged']
    assert directory_digest(source) == report['source_sha256']
    baseline = read(root/'baseline-verification/drc.json')
    allowed = base['allow_existing_annotation_findings']
    cache = {}
    def snapshot(directory):
        path = Path(directory)/name
        key = digest(path)
        if key not in cache:
            cache[key] = isolated_snapshot(path)
        return cache[key]
    before = snapshot(source)
    def preserved(parent, current, changed):
        a,b = snapshot(parent),snapshot(current)
        assert a['pose'] == b['pose']
        other = lambda s: {u:t for u,t in s['tracks'].items() if t['net'] not in changed}
        assert other(a) == other(b)
        for suffix in ['kicad_pro','kicad_sch']:
            assert (Path(parent)/f'{board_id}.{suffix}').read_bytes() == (Path(current)/f'{board_id}.{suffix}').read_bytes()
    def native(directory, recorded=None):
        directory = Path(directory)
        v,d = read(directory/'verification.json'),read(directory/'drc.json')
        if recorded is not None:
            assert v == recorded
        assert read(directory/'preview.json')['source_sha256'] == digest(directory/name)
        return v,d
    # Always render outside source/stage directories: these can be immutable
    # snapshots in descendants' evidence contracts.
    views = []
    def view(directory,label):
        directory = Path(directory)
        dest = root/'audit-views'/digest(directory/name)
        if not (dest/'inspection-combined.svg').exists():
            dest.mkdir(parents=True,exist_ok=True)
            for file in [name,board_id+'.kicad_pro','preview.svg','preview-labels.json']:
                shutil.copy2(directory/file,dest/file)
            render(dest,board_id)
        views.append(dict(label=label,image=str((dest/'inspection-combined.svg').relative_to(root))))
    v,d = native(root/'baseline-verification')
    assert progress_admissible(v,d,baseline,allowed)
    assert snapshot(root/'baseline-verification') == before
    view(root/'baseline-verification','Original committed parent')
    diagnosis = report['diagnosis']
    dc = read(root/'diagnosis-config.json')
    expected_dc = dict(base['diagnosis'],minimum_yielding_connections=2,maximum_yielding_connections=2,
                       maximum_trials=policy['maximum_pair_trials'],prioritize_boundary_blockers=False)
    assert dc == expected_dc
    assert audit_yielding_priority(diagnosis,dc)
    successful = sorted((t for t in diagnosis['trials'] if t['route_found']),
        key=lambda t:(t['quality']['length_mm']+base['via_penalty_mm']*t['quality']['vias'],t['ordinal']))[:policy['maximum_pair_candidates']]
    planned = [(t['ordinal'],order) for t in successful
               for order in [t['yielding_connections'],list(reversed(t['yielding_connections']))]]
    actual = [(a['pair_trial'],a['order']) for a in report['attempts']]
    # A provisional native rejection skips both restoration orders for a pair.
    for a in report['attempts']:
        if not a['order']:
            trial = a['pair_trial']
            start = next(i for i,p in enumerate(planned) if p[0] == trial)
            planned[start:start+2] = [(trial,[])]
    assert actual == planned[:len(actual)]
    work = diagnosis['total_route_expansions']
    calls = nested = copper_items = 0
    selected = []
    for ordinal,a in enumerate(report['attempts']):
        assert a['ordinal'] == ordinal
        trial = diagnosis['trials'][a['pair_trial']]
        provisional = Path(a['provisional_directory'])
        candidate = read(a['target_candidate'])
        assert candidate == trial['candidate']
        pair_nets = {n.removeprefix('/') for n in trial['yielding_connections']}
        preserved(source,provisional,pair_nets|{target})
        assert not any(t['net'] in pair_nets for t in snapshot(provisional)['tracks'].values())
        copper_items += candidate_geometry(candidate,snapshot(provisional),target)
        v,d = native(provisional)
        valid = progress_admissible(v,d,baseline,allowed)
        view(provisional,f'Attempt {ordinal}: target routed; pair displaced')
        if not a['order']:
            assert not valid and not a['selected'] and not a['stages']
            continue
        assert valid
        parent = provisional
        latest = {target:str(Path(a['target_candidate']).resolve())}
        assert [s['connection'] for s in a['stages']] == a['order'][:len(a['stages'])]
        for index,stage in enumerate(a['stages']):
            current = Path(stage['directory'])
            net = stage['connection'].removeprefix('/')
            preserved(parent,current,{net})
            v,d = native(current,stage['verification'])
            assert progress_admissible(v,d,baseline,allowed)
            if stage['candidate']:
                assert not stage['error'] and not stage['route_failure']
                c = read(stage['candidate'])
                work += c['expansions']
                copper_items += candidate_geometry(c,snapshot(current),net)
                latest[net] = str(Path(stage['candidate']).resolve())
            else:
                assert stage['error']
                assert digest(parent/name) == digest(current/name)
                work += (stage.get('route_failure') or {}).get('astar_expansions',0)
                assert index == len(a['stages'])-1
            view(current,f'Attempt {ordinal}: restore {net}'+(' (direct route failed)' if not stage['candidate'] else ''))
            parent = current
        child_directory = a['restoration_directory']
        if child_directory:
            calls += 1
            assert len(a['stages']) == 2 and not a['stages'][-1]['candidate']
            child_root = Path(child_directory)
            child_config = child_root.parent/'restoration-config.json'
            cc = read(child_config)
            assert cc['diagnosis']['maximum_trials'] == policy['maximum_restoration_diagnosis_trials']
            assert cc['diagnosis']['prioritize_boundary_blockers']
            assert cc['restoration'] == policy['restoration']
            assert digest(child_root/'source-unrouted-target'/name) == digest(parent/name)
            audit_single(child_root,child_config)
            child = read(child_root/'ripup.json')
            work += child['total_route_expansions']
            nested += child['restoration_invocations']
            assert child['selected_attempt'] == a['restoration_selected_attempt']
            if child['selected_attempt'] is not None:
                chosen = child['attempts'][child['selected_attempt']]
                latest.update({n:str(Path(r['candidate']).resolve()) for n,r in chosen['changed_routes'].items()})
                parent = Path(chosen['artifact_directory'])
        if a['selected']:
            selected.append(ordinal)
            assert ordinal == len(report['attempts'])-1 and not a['error']
            assert Path(a['final_directory']).resolve() == parent.resolve()
            assert {n:str(Path(r['candidate']).resolve()) for n,r in a['changed_routes'].items()} == latest
            rules = receipt_rules(a['changed_routes'])
            preserved(source,parent,set(rules))
            v,d = native(parent,a['final_verification'])
            assert progress_admissible(v,d,baseline,allowed)
            assert v['selected_net_unconnected_items'] == len(d['unconnected_items']) == report['expected_unconnected_items']
            assert not (finding_counts(d['unconnected_items'])-finding_counts(baseline['unconnected_items']))
            for net,receipt in a['changed_routes'].items():
                copper_items += candidate_geometry(read(receipt['candidate']),snapshot(parent),net)
            view(parent,'Admitted result: every displaced net restored')
        else:
            assert a['error'] and not a['final_directory']
    assert selected == ([] if report['selected_attempt'] is None else [report['selected_attempt']])
    assert calls == report['restoration_repairs'] <= policy['maximum_restoration_repairs']
    assert nested == report['nested_restoration_invocations'] <= calls*policy['restoration']['maximum_invocations']
    assert work == report['total_route_expansions']
    assert directory_digest(source) == report['source_sha256']
    evidence = dict(selected_attempt=report['selected_attempt'],attempts=len(actual),restoration_repairs=calls,
                    nested_restoration_invocations=nested,astar_expansions=work,
                    copper_items_compared=copper_items,source_snapshot_unchanged=True,
                    original_parent_admission_reproduced=True,final_receipts_match_native_geometry=True,
                    pair_priority_and_restoration_orders_reproduced=True,views=views)
    (root/'audit.json').write_text(json.dumps(evidence,indent=2)+'\n')
    page = ['<!doctype html><meta charset="utf-8"><title>Compound routing recovery</title>',
            '<style>body{font:17px system-ui;margin:2rem}img{max-width:100%;max-height:80vh}</style>',
            '<h1>Compound routing recovery</h1><p>Both copper layers; component bodies hidden. Intermediate boards retain missing nets until final admission.</p>']
    for row in views:
        page.append('<h2>'+html.escape(row['label'])+'</h2><img src="'+html.escape(row['image'])+'">')
    page.append('<p><a href="audit.json">Independent audit</a> · <a href="compound.json">Full journal</a></p>')
    (root/'index.html').write_text(''.join(page))
    return evidence


if __name__ == '__main__':
    parser = argparse.ArgumentParser(description=__doc__)
    for name in ['root','base','policy']:
        parser.add_argument(name,type=Path)
    args = parser.parse_args()
    print(json.dumps(audit(args.root,args.base,args.policy),indent=2))
