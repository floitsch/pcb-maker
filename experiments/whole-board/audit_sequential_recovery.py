# Copyright (C) 2026 Toit contributors.
"""Audit a terminal sequential routing run, including retained-prefix repairs."""
import argparse
import hashlib
import html
import json
import shutil
from pathlib import Path
import xml.etree.ElementTree as ET

from audit_partial_ripup import isolated_snapshot
from report_adaptive_routing import finding_counts, progress_admissible
from render_inspection_layers import render
from sequential_evidence import selected_admission, selected_rules, recorded_rules, audit_search_attempts, audit_yielding_priority


def read(path):
    return json.loads(path.read_text())


def digest(path):
    return hashlib.sha256(path.read_bytes()).hexdigest()


def classified_sparse_failure(step):
    attempted = [a for a in step['attempts'] if not a.get('skip_reason')]
    return bool(attempted) and step.get('repair') is None and all(
        a.get('candidate') is None and a.get('native_admission') is None
        and (a.get('route_failure') or {}).get('kind') in
        ['grid_disconnected', 'search_budget_exhausted'] for a in attempted)


def audit_sweep_inventory(report, config):
    sweep = report.get('sweep')
    assert bool(sweep) == bool(config.get('continue_after_routing_failure', False))
    if sweep is None:
        assert report['schema_version'] <= 8
        return
    assert report['schema_version'] == 9 and not report.get('resume')
    order, steps = report['connection_order'], report['steps']
    assert len(order) == len(set(order))
    assert len(steps) <= min(report['maximum_connections'], len(order))
    assert [(s['ordinal'], s['connection']) for s in steps] == list(enumerate(order[:len(steps)]))
    completed = [s['connection'] for s in steps if selected_admission(s)]
    failed = [s['connection'] for s in steps if not selected_admission(s)]
    assert sweep['attempted_connections'] == len(steps)
    assert sweep['completed'] == completed and sweep['failed'] == failed
    assert sweep['unattempted'] == order[len(steps):]
    assert report['completed_connections'] == len(completed)
    assert sweep['repair_invocations_used'] == sum(bool(s.get('repair')) for s in steps)
    assert sweep['repair_invocations_used'] <= (config.get('ripup') or {}).get('maximum_invocations', 0)
    counts = {s['connection']:s.get('electrical_terminal_count', s['distinct_pad_centers'])-1
              for s in report['discovered_connections']}
    assert sweep['expected_unconnected_items'] == sum(counts[n] for n in order if n not in completed)
    halted = bool(steps and not selected_admission(steps[-1]) and not classified_sparse_failure(steps[-1]))
    assert sweep['halted_on_unclassified_failure'] == halted
    for step in steps[:-1]:
        assert selected_admission(step) or classified_sparse_failure(step)
    expected = ('complete' if len(completed) == len(order) else
                'routing_failed' if len(steps) == len(order) or halted else 'bound_reached')
    assert report['termination'] == expected


def audit(root, config, reference, allow_annotations=False):
    report = read(root/'sequential-route.json')
    audit_sweep_inventory(report, config)
    search_evidence = audit_search_attempts(report, config)
    resume = report.get('resume')
    initial_count = resume['initial_completed_connections'] if resume else 0
    board_id = report['board_id']
    name = board_id+'.kicad_pcb'
    source = Path(report['source_directory']).resolve()
    assert digest(source/name) == report['source_board_sha256_before'] == report['source_board_sha256_after']
    assert report['source_board_unchanged']
    baseline = read(root/'native-source-baseline/drc.json')
    previous = baseline
    previous_board = source/name
    previous_snapshot = isolated_snapshot(previous_board)
    source_pose = previous_snapshot['pose']
    counts = {s['connection']:s.get('electrical_terminal_count', s['distinct_pad_centers'])-1 for s in report['discovered_connections']}
    opens = report['expected_source_unconnected_items']
    frames = []
    invocations = []
    completed_repairs = 0
    priority_diagnoses_checked = 0
    prefix_matches = 0
    reference_steps = read(reference/'sequential-route.json')['steps'] if reference else []
    # Adaptive runs may allow unchanged annotations even without a rip-up
    # policy. The sequential config does not serialize that outer setting;
    # require an explicit audit option instead of inferring it from findings.
    allowed = allow_annotations or bool(config.get('ripup') and config['ripup']['allow_existing_annotation_findings'])
    render(root/'native-source-baseline',board_id)
    for step in report['steps']:
        repair = step.get('repair')
        if repair:
            compound = repair.get('compound')
            compound_record = None
            assert step['selected_config_index'] is None and not any(a['selected'] for a in step['attempts'])
            if step['ordinal'] >= initial_count:
                assert config['ripup']['routing_config_index'] == repair['routing_config_index']
            record = read(Path(repair['directory'])/'ripup.json') if (Path(repair['directory'])/'ripup.json').exists() else None
            if record:
                if step['ordinal'] >= initial_count:
                    assert len(record['diagnosis']['trials']) <= config['ripup']['maximum_diagnosis_trials']
                assert record['diagnosis']['maximum_yielding_connections'] == 1
                repair_config = read(Path(step['artifact_directory'])/'ripup-config.json')
                from sequential_evidence import audit_ripup_selection
                audit_ripup_selection(record, repair_config)
                if step['ordinal'] >= initial_count:
                    assert repair_config.get('selection_policy', 'best_score') == config['ripup'].get('selection_policy', 'best_score')
                diagnosis_config = repair_config['diagnosis']
                priority_diagnoses_checked += audit_yielding_priority(record['diagnosis'], diagnosis_config)
                assert repair.get('restoration_invocations',0) == record.get('restoration_invocations',0) + (compound or {}).get('nested_restoration_invocations',0)
                if step['ordinal'] >= initial_count:
                    assert record.get('restoration_invocations',0) <= (config['ripup'].get('restoration') or {}).get('maximum_invocations',0)
                # The action must start at the retained predecessor, not at a
                # cold board or a board with unrelated routes removed.
                assert digest(Path(repair['directory'])/'source-unrouted-target'/name) == digest(previous_board)
            if compound:
                compound_root = Path(compound['directory'])
                if (compound_root/'compound.json').exists():
                    from audit_compound_recovery import audit as audit_compound
                    artifact = Path(step['artifact_directory'])
                    audit_compound(compound_root,artifact/'ripup-config.json',artifact/'compound-config.json')
                    compound_record = read(compound_root/'compound.json')
                    assert digest(Path(compound_record['source_snapshot'])/name) == digest(previous_board)
                    for field in ['selected_attempt','restoration_repairs','nested_restoration_invocations']:
                        assert compound[field] == compound_record[field]
                    assert compound['route_expansions'] == compound_record['total_route_expansions']
                    assert repair['route_expansions'] == (record or {}).get('total_route_expansions',0) + compound['route_expansions']
                else:
                    assert compound['error'] and compound['selected_attempt'] is None
            invocations.append(dict(ordinal=step['ordinal'],connection=step['connection'],selected=repair['selected'],error=repair['error'],inherited=step['ordinal']<initial_count))
        admission = selected_admission(step)
        if admission is None:
            assert report.get('sweep') or step is report['steps'][-1]
            assert step['board_sha256_after'] is None
            continue
        opens -= counts[step['connection']]
        directory = Path(admission['directory']).resolve()
        native = read(directory/'verification.json')
        drc = read(directory/'drc.json')
        assert admission['complete'] and native == admission['verification']
        assert native['selected_net_unconnected_items'] == admission['expected_unconnected_items'] == opens
        assert progress_admissible(native,drc,baseline,allowed)
        assert not (finding_counts(drc['unconnected_items']) - finding_counts(previous['unconnected_items']))
        assert len(drc['unconnected_items']) == opens
        if repair and repair['selected']:
            if compound_record and compound_record['selected_attempt'] is not None:
                chosen = compound_record['attempts'][compound_record['selected_attempt']]
                assert repair['selected_attempt'] is None and repair['yielded_connection'] is None
                assert Path(chosen['final_directory']).resolve() == directory
                assert chosen['selected']
            else:
                record = read(Path(repair['directory'])/'ripup.json')
                assert record['selected_attempt'] == repair['selected_attempt']
                chosen = record['attempts'][record['selected_attempt']]
                assert Path(chosen['artifact_directory']).resolve() == directory
                assert chosen['yielding_connection'] == repair['yielded_connection']
                assert chosen['status'] in ['complete','progress']
            if repair.get('changed_routes'):
                assert repair['changed_routes'] == chosen['changed_routes']
            completed_repairs += 1
        snapshot = isolated_snapshot(directory/name)
        assert snapshot['pose'] == source_pose
        changed = recorded_rules(step) if step['ordinal']<initial_count else selected_rules(step,config['routing_portfolio'])
        other = lambda s:{u:t for u,t in s['tracks'].items() if t['net'] not in changed}
        assert other(previous_snapshot) == other(snapshot)
        for track in snapshot['tracks'].values():
            if track['net'] not in changed:
                continue
            rules = changed[track['net']]
            assert track['width'] == round(rules['via_size_mm' if 'drill' in track else 'trace_width_mm']*1e6)
            if 'drill' in track:
                assert track['drill'] == round(rules['via_drill_mm']*1e6)
                assert track['width'] == track['back_width']
        for suffix in ['kicad_pro','kicad_sch']:
            assert (directory/f'{board_id}.{suffix}').read_bytes() == (source/f'{board_id}.{suffix}').read_bytes()
        assert digest(directory/name) == step['board_sha256_after']
        if step['ordinal'] < len(reference_steps):
            old = reference_steps[step['ordinal']]
            old_admission = selected_admission(old)
            if old_admission:
                assert step['connection'] == old['connection']
                assert bool(repair and repair['selected']) == bool(old.get('repair') and old['repair']['selected'])
                assert digest(Path(old_admission['directory'])/name) == digest(directory/name)
                prefix_matches += 1
        frame_path = directory/'preview.svg'
        if step['ordinal'] < initial_count:
            inherited = root/'inherited-frames'
            inherited.mkdir(exist_ok=True)
            frame_path = inherited/f'step-{step["ordinal"]:03}.svg'
            shutil.copy2(directory/'preview.svg',frame_path)
        frames.append(dict(ordinal=step['ordinal'],connection=step['connection'],opens=opens,
                           repaired=bool(repair and repair['selected']),
                           image=str(frame_path.relative_to(root))))
        previous, previous_board, previous_snapshot = drc, directory/name, snapshot
    assert len(frames) == report['completed_connections']
    new_invocations = [r for r in invocations if not r['inherited']]
    assert len(new_invocations) <= (config.get('ripup') or {}).get('maximum_invocations',0)
    if resume:
        checkpoint = Path(resume['checkpoint_directory'])
        assert digest(checkpoint/'sequential-route.json') == resume['checkpoint_report_sha256']
        assert digest(Path(resume['snapshot_directory'])/name) == resume['retained_board_sha256']
        assert read(Path(resume['snapshot_directory'])/'verification.json') == resume['verification']
        assert all(int(path.name.split('-')[1]) >= initial_count
                   for path in root.glob('step-*') if path.is_dir())
    result = Path(report['result_directory']).resolve()
    assert digest(result/name) == digest(previous_board)
    final_native = read(result/'verification.json')
    assert final_native['selected_net_unconnected_items'] == opens
    assert (report['termination']=='complete') == (len(frames)==len(report['connection_order']))
    if report['steps'][-1].get('repair_skip_reason') == 'maximum rip-up invocations reached':
        assert len(new_invocations) == config['ripup']['maximum_invocations']
    render(result,board_id)
    svg_count=0
    for path in root.rglob('*.svg'):
        ET.parse(path)
        svg_count+=1
    evidence=dict(priority_diagnoses_checked=priority_diagnoses_checked,search_attempts=search_evidence,complete=final_native['complete'],native=final_native,completed_nets=len(frames),
                  unchanged_annotation_contract=allowed,
                  repair_invocations=invocations,committed_repairs=completed_repairs,
                  initial_completed_connections=initial_count,new_committed_connections=len(frames)-initial_count,
                  identical_reference_prefix_steps=prefix_matches,frames=frames,svg_xml_checked=svg_count,
                  native_connectivity_ledger_reproduced=True,unrelated_copper_and_pose_preserved=True,
                  native_dimensions_match_selected_rules=True,source_unchanged=True)
    (root/'recovery-audit.json').write_text(json.dumps(evidence,indent=2)+'\n')
    page=['<!doctype html><meta charset="utf-8"><title>Routing recovery</title>',
          '<style>body{font:17px system-ui;margin:25px}img{max-width:95vw;max-height:78vh}input{width:80%}</style>',
          f'<h1>Routing recovery</h1><p>{len(frames)} nets routed, {opens} native open items, '
          f'{completed_repairs} committed repairs. Full native completion: {final_native["complete"]}.</p>',
          '<h2>Retained result</h2><img src="result/inspection-combined.svg">',
          '<h2>Committed routing steps</h2><input id="step" type="range" min="0" max="'+str(max(0,len(frames)-1))+'" value="0"><p id="caption"></p><img id="frame">',
          '<p><a href="recovery-audit.json">Independent audit</a> · <a href="sequential-route.json">Full step evidence</a></p>']
    data=json.dumps(frames).replace('<','\\u003c')
    page.append('<script>const rows='+data+';const slider=document.getElementById("step");function show(){const r=rows[+slider.value];if(!r)return;document.getElementById("frame").src=r.image;document.getElementById("caption").textContent=(r.ordinal+1)+": "+r.connection+(r.repaired?" (repaired)":"")+"; opens: "+r.opens;}slider.oninput=show;show();</script>')
    (root/'recovery.html').write_text(''.join(page))
    from summarize_routing_recovery import write_summary
    write_summary(root)
    print(json.dumps({k:v for k,v in evidence.items() if k!='frames'},indent=2))


if __name__=='__main__':
    parser=argparse.ArgumentParser(description=__doc__)
    parser.add_argument('root',type=Path)
    parser.add_argument('config',type=Path)
    parser.add_argument('--reference-prefix',type=Path)
    parser.add_argument('--allow-annotations',action='store_true',help='Audit the explicit provisional-annotation contract; only unchanged baseline annotation findings may remain')
    args=parser.parse_args()
    audit(args.root.resolve(),read(args.config),args.reference_prefix,args.allow_annotations)
