# Copyright (C) 2026 Toit contributors.
"""Independently audit a terminal native partial-ripup command and render it."""
import argparse
import hashlib
import html
import json
import math
from pathlib import Path
import subprocess
import sys
import xml.etree.ElementTree as ET


def read(path):
    return json.loads(path.read_text())


def snapshot(path):
    import pcbnew
    if not hasattr(pcbnew.SwigPyIterator, 'next'):
        pcbnew.SwigPyIterator.next = pcbnew.SwigPyIterator.__next__
    from report_routing_demand import pose
    board = pcbnew.LoadBoard(str(path))
    tracks = {}
    for item in board.GetTracks():
        via = isinstance(item, pcbnew.PCB_VIA)
        row = dict(net=item.GetNetname().removeprefix('/'), kind=item.GetClass(),
                   width=item.GetWidth(pcbnew.F_Cu) if via else item.GetWidth(),
                   start=[item.GetStart().x,item.GetStart().y], end=[item.GetEnd().x,item.GetEnd().y],
                   layers=item.GetLayerSet().FmtHex())
        if via:
            row['drill'] = item.GetDrillValue()
            row['back_width'] = item.GetWidth(pcbnew.B_Cu)
        if isinstance(item, pcbnew.PCB_ARC):
            row['mid'] = [item.GetMid().x,item.GetMid().y]
        tracks[item.m_Uuid.AsString()] = row
    return dict(pose=pose(board), tracks=tracks,
                copper_layer_masks={name:1<<layer for name,layer in [('F.Cu',pcbnew.F_Cu),('B.Cu',pcbnew.B_Cu)]},
                stored_length_mm=sum(t.GetLength()/1e6 for t in board.GetTracks() if not isinstance(t,pcbnew.PCB_VIA)),
                vias=sum(isinstance(t,pcbnew.PCB_VIA) for t in board.GetTracks()))


def isolated_snapshot(path):
    return json.loads(subprocess.check_output([sys.executable, __file__, '--snapshot', str(path)]))


def audit(root, config_path):
    from report_adaptive_routing import finding_counts, progress_admissible
    from render_inspection_layers import render
    from sequential_evidence import receipt_rules
    report = read(root/'ripup.json')
    config = read(config_path)
    from sequential_evidence import audit_ripup_selection
    selection_evidence = audit_ripup_selection(report, config)
    assert {Path(a['artifact_directory']).name for a in report['attempts']} == {
        p.name for p in root.glob('attempt-*-yield-*') if p.is_dir()}
    assert config['allow_partial_progress']
    # The coordinator may continue mutating its result directory after this
    # repair. Audit the immutable snapshot taken at action entry.
    source = root/'source-unrouted-target'
    board_id = report['board_id']
    board_name = board_id+'.kicad_pcb'
    before = isolated_snapshot(source/board_name)
    baseline_drc = read(root/'baseline-verification/drc.json')
    expected = report['expected_unconnected_items']
    target = report['target_connection'].removeprefix('/')
    source_hash = hashlib.sha256((source/board_name).read_bytes()).hexdigest()
    rows = []
    candidates = []
    restoration_ordinals = []
    render(root/'baseline-verification', board_id)
    for trial in report['attempts']:
        directory = Path(trial['artifact_directory']).resolve()
        yielding = trial['yielding_connection'].removeprefix('/')
        assert yielding not in {n.removeprefix('/') for n in report.get('protected_connections',[])}
        recovery = trial.get('restoration')
        if recovery:
            assert config.get('restoration')
            restoration_ordinals.append(recovery['invocation_ordinal'])
            assert recovery['depth'] <= config['restoration']['maximum_depth']
            child_root = Path(recovery['directory']).resolve()
            if (child_root/'ripup.json').exists():
                audit(child_root,child_root.with_suffix('.config.json'))
                child = read(child_root/'ripup.json')
                child_audit = read(child_root/'audit.json')
                restoration_ordinals.extend(child_audit['restoration_invocation_ordinals'])
                assert recovery['selected_attempt'] == child['selected_attempt']
                assert hashlib.sha256((Path(child['source_directory'])/board_name).read_bytes()).hexdigest() == hashlib.sha256((Path(trial['provisional_directory'])/board_name).read_bytes()).hexdigest()
                if child['selected_attempt'] is not None and trial['status'] in ['complete','progress']:
                    chosen = child['attempts'][child['selected_attempt']]
                    assert (directory/board_name).read_bytes() == (Path(chosen['artifact_directory'])/board_name).read_bytes()
                    assert set(trial['changed_routes']) == set(chosen['changed_routes']) | {target}
        changes = receipt_rules(trial['changed_routes']) if trial.get('changed_routes') else {
            n:config['diagnosis']['routing'].get('connection_rules',{}).get(n,config['diagnosis']['routing'])
            for n in [target,yielding]}
        after = isolated_snapshot(directory/board_name)
        assert before['pose'] == after['pose']
        other = lambda snapshot: {u:t for u,t in snapshot['tracks'].items() if t['net'] not in changes}
        assert other(before) == other(after)
        for suffix in ['kicad_pro','kicad_sch']:
            assert (source/f'{board_id}.{suffix}').read_bytes() == (directory/f'{board_id}.{suffix}').read_bytes()
        for track in after['tracks'].values():
            if track['net'] not in changes:
                continue
            rules = changes[track['net']]
            assert track['width'] == round(rules['via_size_mm' if 'drill' in track else 'trace_width_mm']*1e6)
            if 'drill' in track:
                assert track['drill'] == round(rules['via_drill_mm']*1e6)
                assert track['back_width'] == track['width']
        admitted = False
        if trial['final_verification'] is not None:
            native = read(directory/'verification.json')
            drc = read(directory/'drc.json')
            assert native == trial['final_verification']
            admitted = (progress_admissible(native,drc,baseline_drc,config['allow_existing_annotation_findings'])
                and len(drc['unconnected_items']) == native['selected_net_unconnected_items'] == expected
                and not (finding_counts(drc['unconnected_items']) - finding_counts(baseline_drc['unconnected_items'])))
            assert admitted == (trial['status'] in ['complete','progress'])
            if admitted:
                assert (trial['status'] == 'complete') == native['complete']
                score = trial.get('selection_score_mm',trial['combined_score_mm'])
                if 'selection_score_mm' in trial:
                    # These emitted trees contain no overlapping segments;
                    # native stored length independently checks the total.
                    expected_score = after['stored_length_mm'] + config['via_penalty_mm']*after['vias']
                    assert math.isclose(score,expected_score,abs_tol=1e-3), (score,expected_score)
                candidates.append((not native['complete'],score,trial['ordinal']))
        else:
            assert trial['status'] not in ['complete','progress']
        provisional = Path(trial['provisional_directory'])
        assert read(provisional/'verification.json') == trial['provisional_verification']
        assert hashlib.sha256((provisional/board_name).read_bytes()).hexdigest() == read(provisional/'preview.json')['source_sha256']
        render(directory,board_id)
        rows.append(dict(ordinal=trial['ordinal'],yielding=yielding,status=trial['status'],admitted=admitted,
                         directory=str(directory.relative_to(root)),selected=trial['selected']))
    selected = min(candidates)[2] if candidates else None
    assert len(restoration_ordinals) == report.get('restoration_invocations',0)
    assert len(set(restoration_ordinals)) == len(restoration_ordinals)
    if config.get('restoration'):
        assert len(restoration_ordinals) <= config['restoration']['maximum_invocations']
    assert selected == report['selected_attempt']
    assert [r['ordinal'] for r in rows if r['selected']] == ([] if selected is None else [selected])
    assert report['source_unchanged'] and hashlib.sha256((source/board_name).read_bytes()).hexdigest() == source_hash
    from sequential_evidence import audit_yielding_priority
    priority_checked = audit_yielding_priority(report['diagnosis'], config['diagnosis'])
    cut = report['diagnosis'].get('routing_cut')
    if cut:
        from inspect_routing_cut import render as render_cut
        directory = root/'boundary-inspection'
        directory.mkdir(exist_ok=True)
        (directory/'report.json').write_text(json.dumps(cut,indent=2)+'\n')
        (directory/'run.json').write_text(json.dumps({'scope':'Render of the retained diagnosis; no new routing or native admission.',
            'journal':str(root/'ripup.json')},indent=2)+'\n')
        render_cut(cut,root/'baseline-verification/inspection-combined.svg',directory)
    svg_count = 0
    for path in root.rglob('*.svg'):
        ET.parse(path)
        svg_count += 1
    audit_result = dict(source_snapshot=str(source),expected_open_items=expected,selected_attempt=selected,trials=rows,svg_xml_checked=svg_count,selection_evidence=selection_evidence,
                        restoration_invocation_ordinals=restoration_ordinals,
                        yielding_priority_checked=priority_checked,
                        native_pose_and_other_copper_preserved=True,changed_net_dimensions_match_rules=True,
                        native_progress_gate_and_selection_reproduced=True,source_board_unchanged=True)
    (root/'audit.json').write_text(json.dumps(audit_result,indent=2)+'\n')
    page = ['<!doctype html><meta charset="utf-8"><title>Partial-board rip-up</title>',
        '<style>body{font:17px system-ui;margin:25px}.pair{display:grid;grid-template-columns:1fr 1fr;gap:20px}img{width:100%}</style>',
        '<h1>Partial-board rip-up</h1><p>Connect the blocked target and restore a yielding net. Progress is distinct from complete-layout admission.</p>']
    if cut:
        page.append('<p><a href="boundary-inspection/index.html">Reachable-region map and boundary evidence used for removal priority</a></p>')
    for row in rows:
        page.append(f'<h2>{html.escape(row["yielding"])}: {row["status"]}{" (selected)" if row["selected"] else ""}</h2>'
                    f'<div class="pair"><img src="baseline-verification/inspection-combined.svg">'
                    f'<img src="{row["directory"]}/inspection-combined.svg"></div>')
    page.append('<p><a href="audit.json">Independent audit</a> · <a href="ripup.json">Full evidence</a></p>')
    (root/'index.html').write_text(''.join(page))
    print(json.dumps(audit_result,indent=2))


if __name__ == '__main__':
    parser=argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--snapshot',type=Path)
    parser.add_argument('root',type=Path,nargs='?')
    parser.add_argument('config',type=Path,nargs='?')
    args=parser.parse_args()
    if args.snapshot:
        print(json.dumps(snapshot(args.snapshot)))
    else:
        audit(args.root.resolve(),args.config.resolve())
