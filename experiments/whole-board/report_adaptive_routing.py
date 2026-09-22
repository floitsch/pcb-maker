# Copyright (C) 2026 Toit contributors.
"""Audit finished adaptive commands and compare their retained native boards.

Call only after every command is terminal. This does not treat progress files
or observation timeouts as evidence that a routing process has stopped.
"""
import argparse
from collections import Counter
import copy
import hashlib
import html
import json
import math
from pathlib import Path
import re
import subprocess
import xml.etree.ElementTree as ET
from report_routing_demand import pose, pcbnew
from render_inspection_layers import render
from run_routing_demand import write_json
from sequential_evidence import selected_rules


def clean(native):
    return all(native[key] == 0 for key in ['erc_violations', 'drc_design_violations', 'schematic_parity_issues'])


ANNOTATION_TYPES = {'silk_overlap', 'silk_over_copper', 'silk_edge_clearance'}


def design_findings(drc):
    return [f for f in drc['violations']
            if not (f['type'] == 'lib_footprint_mismatch' and f['severity'] == 'warning')]


def finding_counts(findings):
    def fingerprint(finding):
        value = copy.deepcopy(finding)
        value['items'].sort(key=lambda item: json.dumps(item, sort_keys=True))
        return json.dumps(value, sort_keys=True)
    return Counter(map(fingerprint, findings))


# ERC and schematic-parity findings that the source itself has are not the
# router's doing; a benchmark harness sets this from the source verification.
BASELINE_NATIVE = {'erc_violations': 0, 'schematic_parity_issues': 0}


def progress_admissible(native, drc, baseline, allow_annotations):
    findings = design_findings(drc)
    if (native['erc_violations'] > BASELINE_NATIVE['erc_violations']
            or native['schematic_parity_issues'] > BASELINE_NATIVE['schematic_parity_issues']
            or len(findings) != native['drc_design_violations']):
        return False
    return not findings or (allow_annotations
        and all(f['type'] in ANNOTATION_TYPES for f in findings)
        and not (finding_counts(findings) - finding_counts(design_findings(baseline))))


def audit_observed_retry(report):
    retries = [p for p in report['passes']
               if p['reason'] == 'observed failure before independent forecasts']
    assert len(retries) <= 1, 'more than one forecast-free retry'
    for retry in retries:
        assert report['config'].get('retry_observed_failures_before_forecasts', False)
        assert retry['ordinal'] == 1 and retry['parent_pass'] == 0
        parent = report['passes'][0]
        assert parent['error'] is None and parent['fixed_design_preserved']
        assert parent['native_progress_admissible'] and parent['native']['selected_net_unconnected_items'] > 0
        sequential = parent['sequential']
        attempted = {step['connection'] for step in sequential['steps']}
        routed = set()
        for step in sequential['steps']:
            if any(a['selected'] and a.get('quality') is not None for a in step['attempts']):
                routed.add(step['connection'])
            repair = step.get('repair')
            if repair and repair['selected']:
                if repair.get('target_quality') is not None:
                    routed.add(step['connection'])
                if repair.get('yielded_connection') and repair.get('yielded_quality') is not None:
                    routed.add(repair['yielded_connection'])
                routed.update(repair.get('changed_routes', {}))
        order = sequential['connection_order']
        expected_order = sorted(order, key=lambda net: 0 if net in attempted and net not in routed
                                else 1 if net not in attempted else 2)
        assert expected_order != parent['config']['connection_order'], 'no-op retry'
        expected = copy.deepcopy(parent['config'])
        expected['connection_order'] = expected_order
        assert retry['config'] == expected, 'forecast-free retry changed parent policy beyond order'
    if retries and len(report['passes']) == 2 and report['forecast_generation'].startswith('skipped_'):
        assert not report['forecasts'] and report['forecast_elapsed_micros'] == 0
    return {'forecast_free_retries': len(retries), 'exact_parent_policy_preserved': True}


def audit(root):
    rows = []
    for path in sorted(root.rglob('adaptive-routing.json')):
        report = json.loads(path.read_text())
        retry_evidence = audit_observed_retry(report)
        directory = path.parent
        board_id = report['board_id']
        source = Path(report['source_directory'])
        board_name = board_id + '.kicad_pcb'
        assert hashlib.sha256((source/board_name).read_bytes()).hexdigest() == report['source_board_sha256']
        assert report['source_unchanged']
        before = pcbnew.LoadBoard(str(source/board_name))
        source_pose = pose(before)
        del before
        after = pcbnew.LoadBoard(str(directory/'result'/board_name))
        assert source_pose == pose(after)
        for suffix in ['kicad_pro', 'kicad_sch']:
            assert (source/f'{board_id}.{suffix}').read_bytes() == (directory/'result'/f'{board_id}.{suffix}').read_bytes()
        incumbent = json.loads((directory/'source/verification.json').read_text())
        baseline_drc = json.loads((directory/'source/drc.json').read_text())
        provisional_schema = report['schema_version'] >= 2
        allow_annotations = report['config'].get('allow_existing_annotation_findings', False)
        if provisional_schema:
            assert progress_admissible(incumbent, baseline_drc, baseline_drc, allow_annotations)
        incumbent_score = 0.0
        selected = None
        decisions = []
        fingerprints = set()
        for trial in report['passes']:
            fingerprint = json.dumps(trial['config'], sort_keys=True)
            assert fingerprint not in fingerprints
            fingerprints.add(fingerprint)
            native = trial['native']
            if native is None:
                assert trial['error']
                continue
            proposed_score = trial['score_mm']
            admissible = clean(native)
            if provisional_schema:
                drc = json.loads((Path(trial['directory'])/'result/drc.json').read_text())
                admissible = progress_admissible(native, drc, baseline_drc, allow_annotations)
                assert trial['native_progress_admissible'] == admissible
            accepted = trial['fixed_design_preserved'] and admissible and (
                not incumbent['complete'] or native['complete']) and (
                (native['complete'] and not incumbent['complete'])
                or native['selected_net_unconnected_items'] < incumbent['selected_net_unconnected_items']
                or (native['selected_net_unconnected_items'] == incumbent['selected_net_unconnected_items']
                    and proposed_score+1e-6 < incumbent_score))
            decisions.append(dict(pass_index=trial['ordinal'], accepted=accepted, complete=native['complete'],
                                  native_progress_admissible=admissible,
                                  design_findings=native['drc_design_violations'],
                                  unconnected_items=native['selected_net_unconnected_items'], score_mm=proposed_score))
            if accepted:
                selected, incumbent, incumbent_score = trial['ordinal'], native, proposed_score
        assert selected == report['selected_pass']
        assert incumbent == report['result_verification']
        assert report['complete'] == incumbent['complete']
        if report['schema_version'] >= 3 and not report['config']['optimize_after_routing_complete']:
            # Stop after the first accepted fully connected board, even when
            # unchanged annotations still prevent full-layout admission.
            connected = [d['pass_index'] for d in decisions if d['accepted'] and d['unconnected_items'] == 0]
            if connected:
                assert connected == [len(report['passes'])-1]
                assert report['termination'] == 'routing_complete'
        if provisional_schema:
            final_drc = json.loads((directory/'result/drc.json').read_text())
            assert progress_admissible(incumbent, final_drc, baseline_drc, allow_annotations)
            assert report['routing_complete'] == (incumbent['selected_net_unconnected_items'] == 0)
            assert report['outstanding_annotation_findings'] == sum(
                f['type'] in ANNOTATION_TYPES for f in design_findings(final_drc))
        if selected is None:
            assert (directory/'result'/board_name).read_bytes() == (source/board_name).read_bytes()
        expected_geometry = {}
        if selected is not None:
            chosen_pass = report['passes'][selected]
            for step in chosen_pass['sequential']['steps']:
                expected_geometry.update(selected_rules(step,chosen_pass['config']['routing_portfolio']))
        for track in after.GetTracks():
            expected = expected_geometry[track.GetNetname().removeprefix('/')]
            if isinstance(track, pcbnew.PCB_VIA):
                assert track.GetWidth(pcbnew.F_Cu) == round(expected['via_size_mm']*1e6)
                assert track.GetWidth(pcbnew.B_Cu) == round(expected['via_size_mm']*1e6)
                assert track.GetDrillValue() == round(expected['via_drill_mm']*1e6)
            else:
                assert track.GetWidth() == round(expected['trace_width_mm']*1e6)
        for forecast in report['forecasts']:
            for attempt in forecast['attempts']:
                artifact = Path(attempt['directory'])
                preview = json.loads((artifact/'preview.json').read_text())
                assert preview['source_sha256'] == hashlib.sha256((artifact/board_name).read_bytes()).hexdigest()
                assert (artifact/'preview.svg').is_file()
                assert not (artifact/'verification.json').exists(), artifact
                assert not (artifact/'drc.json').exists(), artifact
        tracks = [t for t in after.GetTracks() if not isinstance(t, pcbnew.PCB_VIA)]
        vias = sum(isinstance(t, pcbnew.PCB_VIA) for t in after.GetTracks())
        length = sum(t.GetLength()/1e6 for t in tracks)
        assert vias == report['result_statistics']['vias']
        assert math.isclose(length, report['result_statistics']['stored_segment_length_mm'], abs_tol=1e-4)
        render(directory/'result', board_id)
        if report['passes'][0]['sequential'] is not None:
            render(directory/'pass-000/result', board_id)
        page = (directory/'index.html').read_text()
        # Check the generated player's actual frame URLs and JavaScript syntax.
        # Headless Firefox failed to launch in this environment; structural
        # checks and JavaScript syntax do not claim browser playback.
        script = re.search(r'<script>(.*?)</script>', page, re.S).group(1)
        frames = json.loads(re.search(r'const frames=(.*?);const slider=', script, re.S).group(1))
        for frame in frames:
            ET.parse(directory/frame['src'])
        script_path = directory/'player-syntax-check.js'
        script_path.write_text(script)
        subprocess.run(['node', '--check', str(script_path)], check=True, capture_output=True)
        rows.append(dict(run=directory.relative_to(root).as_posix(), board_id=board_id, complete=report['complete'], termination=report['termination'],
                         observed_retry=retry_evidence,
                         passes=len(report['passes']), selected_pass=selected, vias=vias, track_mm=length,
                         remaining_unconnected_items=incumbent['selected_net_unconnected_items'], decisions=decisions,
                         design_findings=incumbent['drc_design_violations'],
                         fixed_inputs_and_pose_preserved=True, incumbent_selection_reproduced=True,
                         track_and_via_dimensions_match_requested_rules=True,
                         animation_frames_checked=len(frames), browser_playback_tested=False))
    svg_count = 0
    for path in root.rglob('*.svg'):
        ET.parse(path)
        svg_count += 1
    write_json(root/'audit.json', dict(runs=rows, svg_xml_checked=svg_count))
    page = ['<!doctype html><meta charset="utf-8"><title>Adaptive routing integration</title>',
            '<style>body{font:17px system-ui;margin:25px}table{border-collapse:collapse}td,th{padding:8px;border:1px solid #aaa}.pair{display:grid;grid-template-columns:1fr 1fr;gap:20px}img{width:100%}</style>',
            '<h1>Adaptive routing integration</h1><p>One command generates forecasts, routes, learns net priorities and retains its best admissible board. Provisional annotation findings remain visible and prevent complete-layout admission. Click a run to inspect its pass table and routing-step player.</p>',
            '<table><tr><th>Run</th><th>Complete</th><th>Passes</th><th>Selected</th><th>Vias</th><th>Track mm</th><th>Opens</th><th>Design findings</th></tr>']
    for row in rows:
        page.append(f'<tr><td><a href="{row["run"]}/index.html">{html.escape(row["run"])}</a></td><td>{row["complete"]}</td><td>{row["passes"]}</td><td>{row["selected_pass"]}</td><td>{row["vias"]}</td><td>{row["track_mm"]:.3f}</td><td>{row["remaining_unconnected_items"]}</td><td>{row["design_findings"]}</td></tr>')
    page.append('</table><p>Complete requires zero ERC, design DRC, parity and open items. Library metadata warnings are recorded separately in native reports.</p>')
    for row in rows:
        name = row['run']
        if not (root/name/'pass-000/result/inspection-combined.svg').exists():
            continue
        for side in ['combined', 'back', 'front']:
            page.append(f'<h2>{name}: {side} copper</h2><div class="pair"><div><h3>Original configured policy</h3><img src="{name}/pass-000/result/inspection-{side}.svg"></div><div><h3>Automatically selected</h3><img src="{name}/result/inspection-{side}.svg"></div></div>')
    page.append('<p><a href="audit.json">Independent audit</a> · <a href="binaries.json">Executable provenance</a></p>')
    (root/'index.html').write_text(''.join(page))
    print(json.dumps(rows, indent=2))


if __name__ == '__main__':
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('root', type=Path)
    audit(parser.parse_args().root.resolve())
