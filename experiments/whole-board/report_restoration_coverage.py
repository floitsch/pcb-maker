# Copyright (C) 2026 Toit contributors.
"""Report audited repair-search coverage and external input discrepancies."""
import argparse
import html
import json
from pathlib import Path
import xml.etree.ElementTree as ET

from run import digest, read, write


def report(root):
    sequence=read(root/'run/sequential-route.json')
    audit=read(root/'run/recovery-audit.json')
    assert sequence['termination']=='routing_failed'
    assert audit['native_connectivity_ledger_reproduced'] and audit['new_committed_connections']==0
    step=sequence['steps'][-1]
    repair=Path(step['repair']['directory'])
    journal=read(repair/'ripup.json')
    assert read(repair/'audit.json')['native_progress_gate_and_selection_reproduced']
    rows=[]
    for trial in journal['attempts']:
        child=Path(trial['restoration']['directory'])
        diagnosis=read(child/'diagnosis.json')
        child_journal=read(child/'ripup.json')
        assert not diagnosis['truncated'] and child_journal['selected_attempt'] is None
        rows.append(dict(yielding=trial['yielding_connection'],
            eligible_nets=diagnosis['foreign_routed_connections'],
            diagnosis_trials=len(diagnosis['trials']),
            successful_counterfactuals=diagnosis['successful_counterfactuals'],
            attempted_restorations=len(child_journal['attempts']),status=trial['status']))
    comparison=read(root/'freerouting-final/report.json')
    assert comparison['status']=='finished' and comparison['router_execution_reused']
    assert comparison['fixed_placement_matches'] and not comparison['dimensional_mismatches']
    assert comparison['equal_rule_comparison'] is False
    controls=read(root/'import-controls-final/report.json')
    assert controls['positive_exact_pose_and_copper_import']
    assert controls['tampered_session_replay_rejected'] and controls['terminal_failure_recorded']
    assert all(x['exit_code']!=0 and x['source_unchanged'] for x in controls['negative_controls'])
    geometry=comparison['exchange_geometry']
    ecc83=read(root/'exchange-ecc83-final/report.json')
    assert ecc83['source_unchanged']
    previous=Path(sequence['resume']['checkpoint_directory'])
    board=sequence['board_id']+'.kicad_pcb'
    assert digest(root/'run/result'/board)==digest(previous/'result'/board)
    count=0
    for svg in root.rglob('*.svg'):
        ET.parse(svg)
        count+=1
    summary=dict(completed_nets=sequence['completed_connections'],total_nets=len(sequence['connection_order']),
        native_opens=audit['native']['selected_net_unconnected_items'],
        retained_board_identical=True,sibling_coverage=rows,
        child_diagnosis_trials=sum(x['diagnosis_trials'] for x in rows),
        child_restoration_attempts=sum(x['attempted_restorations'] for x in rows),
        external_native=comparison['native'],equal_rule_comparison=False,
        external_unique_input_violations=geometry['unique_violation_count'],
        external_effective_edge_rules=geometry['effective_edge_rules'],
        native_edge_clearance_mm=geometry['native_edge_clearance_mm'],
        ecc83_input_violations=ecc83['unique_violation_count'],
        ecc83_edge_requirement_mismatch=ecc83['edge_requirement_mismatch_observed'],
        import_controls=controls,svg_xml_checked=count)
    write(root/'summary.json',summary)
    page=['<!doctype html><meta charset="utf-8"><title>Whole-board recovery coverage</title>',
        '<style>body{font:17px system-ui;margin:2rem}img{max-width:95vw}td,th{padding:.5rem;border:1px solid #aaa}table{border-collapse:collapse}</style>',
        '<h1>Whole-board recovery coverage and exchange inspection</h1>',
        f'<p>Broader nested recovery retains {summary["completed_nets"]}/{summary["total_nets"]} nets and {summary["native_opens"]} native opens. '
        'No new route is committed; the original board is retained byte for byte.</p>',
        '<table><tr><th>Yielded net</th><th>Child diagnosis trials</th><th>Child restoration attempts</th><th>Result</th></tr>']
    for row in rows:
        page.append(f'<tr><td>{html.escape(row["yielding"])}</td><td>{row["diagnosis_trials"]}</td>'
            f'<td>{row["attempted_restorations"]}</td><td>{row["status"]}</td></tr>')
    page.extend(['</table><p>All six siblings receive untruncated single-net child diagnosis. This does not exhaust multi-net topology or placement alternatives.</p>',
        '<img src="run/result/inspection-combined.svg">',
        '<h2>External comparison</h2><p>The saved Freerouting session imports after restoring only bounded coordinate rounding. '
        'Exact native poses, source class dimensions and source project are preserved; KiCad still reports 65 opens and seven unchanged annotation findings. '
        'No new external routing execution is claimed.</p>',
        '<p>The exchange input has 14 layer-specific pad/edge conflicts at seven pads. Its effective edge requirement is 0.31 mm; the native project requires 0.01 mm. '
        'This is not an equal-rule comparison and does not establish placement infeasibility.</p>',
        '<img src="freerouting-final/exchange-geometry/inspection.svg">',
        '<p><a href="run/recovery.html">Sequence audit</a> · '
        '<a href="run/step-034-Net--R212-Pad2-/ripup/index.html">Repair audit</a> · '
        '<a href="freerouting-final/index.html">External result</a> · '
        '<a href="freerouting-final/exchange-geometry/index.html">Input geometry findings</a> · '
        '<a href="exchange-ecc83-final/index.html">ECC83 diagnostic control</a> · '
        '<a href="import-controls-final/report.json">Import controls</a> · <a href="summary.json">Summary</a></p>'])
    (root/'index.html').write_text(''.join(page))
    print(json.dumps({k:v for k,v in summary.items() if k not in ['sibling_coverage','import_controls']}))


if __name__=='__main__':
    parser=argparse.ArgumentParser(description=__doc__)
    parser.add_argument('root',type=Path)
    report(parser.parse_args().root.resolve())
