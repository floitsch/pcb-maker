# Copyright (C) 2026 Toit contributors.
"""Audit a nested restoration against the original retained whole-board prefix."""
import argparse
import hashlib
import json
from pathlib import Path

from audit_partial_ripup import isolated_snapshot
from report_adaptive_routing import finding_counts, progress_admissible
from sequential_evidence import selected_admission


def audit(original_sequence, repair_root, inserted, output):
    read = lambda p: json.loads(p.read_text())
    digest = lambda p: hashlib.sha256(p.read_bytes()).hexdigest()
    original = read(original_sequence/'sequential-route.json')
    repair = read(repair_root/'ripup.json')
    assert repair['selected_attempt'] is not None
    selected = repair['attempts'][repair['selected_attempt']]
    assert selected['status'] == 'progress'
    baseline = Path(original['result_directory']).resolve()
    final = Path(selected['artifact_directory']).resolve()
    name = original['board_id']+'.kicad_pcb'
    before, after = isolated_snapshot(baseline/name), isolated_snapshot(final/name)
    changed = {inserted.removeprefix('/'), repair['target_connection'].removeprefix('/'),
               selected['yielding_connection'].removeprefix('/')}
    unrelated = lambda s:{key:value for key,value in s['tracks'].items() if value['net'] not in changed}
    assert before['pose'] == after['pose'] and unrelated(before) == unrelated(after)
    committed = {s['connection'].removeprefix('/') for s in original['steps'] if selected_admission(s)}
    assert len(committed) == original['completed_connections']
    assert inserted.removeprefix('/') not in committed
    assert {t['net'] for t in after['tracks'].values()} == committed | {inserted.removeprefix('/')}
    for path in baseline.rglob('*'):
        if path.is_file() and (path.suffix in ['.kicad_pro','.kicad_sch','.kicad_dru']
                               or path.name in ['sym-lib-table','fp-lib-table']):
            assert path.read_bytes() == (final/path.relative_to(baseline)).read_bytes()
    previous_native = read(baseline/'verification.json')
    native, drc = read(final/'verification.json'), read(final/'drc.json')
    baseline_drc = read(baseline/'drc.json')
    reduction = next(c.get('electrical_terminal_count', c['distinct_pad_centers'])-1 for c in original['discovered_connections']
                     if c['connection'].removeprefix('/') == inserted.removeprefix('/'))
    assert native['selected_net_unconnected_items'] == previous_native['selected_net_unconnected_items']-reduction
    assert progress_admissible(native,drc,baseline_drc,True)
    assert not (finding_counts(drc['unconnected_items'])-finding_counts(baseline_drc['unconnected_items']))
    # The inner repair's independent audit checks changed dimensions and every
    # restoration alternative. Its original target insertion was already native
    # admitted by the outer failed single-net repair.
    inner = read(repair_root/'audit.json')
    assert inner['native_progress_gate_and_selection_reproduced']
    assert inner['changed_net_dimensions_match_rules']
    evidence = dict(original_board=str(baseline/name),result_board=str(final/name),
                    original_sha256=digest(baseline/name),result_sha256=digest(final/name),
                    original_completed_nets=len(committed),completed_nets=len(committed)+1,
                    original_open_items=previous_native['selected_net_unconnected_items'],
                    open_items=native['selected_net_unconnected_items'],changed_nets=sorted(changed),
                    native=native,unchanged_pose_and_unrelated_copper=True,
                    unchanged_project_schematic_and_rules=True,no_new_native_findings=True,
                    no_new_disconnections=True,coordinator_integration=False)
    output.write_text(json.dumps(evidence,indent=2)+'\n')
    print(json.dumps(evidence,indent=2))


if __name__ == '__main__':
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('original_sequence',type=Path)
    parser.add_argument('repair_root',type=Path)
    parser.add_argument('inserted')
    parser.add_argument('output',type=Path)
    args = parser.parse_args()
    audit(args.original_sequence,args.repair_root,args.inserted,args.output)
