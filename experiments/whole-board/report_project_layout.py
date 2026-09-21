# Copyright (C) 2026 Toit contributors.
"""Summarize terminal general-layout runs and their native failure controls.

Invoke only after awaiting the actual runner processes, not from progress JSON.
"""
import argparse
import hashlib
import html
import json
from pathlib import Path
import xml.etree.ElementTree as ET
from render_inspection_layers import render
from sequential_evidence import selected_admission


def read(path):
    return json.loads(path.read_text())


def digest(path):
    return hashlib.sha256(path.read_bytes()).hexdigest()


def report(root):
    processes={r['case']:r for r in read(root/'processes.json')}
    rows = []
    for name, mode in [('ecc83-final', 'adaptive'), ('complex-offset', 'diagnostic-routing')]:
        assert processes[name]['terminal_observed'] and processes[name]['exit_code']==0
        directory = root/name
        result = read(directory/'report.json')
        assert result['finished']
        assert digest(Path(result['source'])) == result['source_sha256']
        attempt = result['attempts'][0]
        native_dir = directory/'area-00'/mode/'result'
        native = read(native_dir/'verification.json')
        assert native == attempt['routing_result']['native']
        assert bool(attempt['admitted']) == bool(native['complete'])
        for audit_name, board_dir in [('native-inventory-audit', 'native'),
                                      ('silkscreen-native-inventory-audit', 'native-silk')]:
            audit = read(directory/'area-00'/f'{audit_name}.json')
            assert audit['passed'] and audit['source_sha256'] == result['source_sha256']
            assert audit['placed_sha256'] == digest(directory/'area-00'/board_dir/f'{result["board_id"]}.kicad_pcb')
        render(native_dir, result['board_id'])
        sequence_dir = directory/'area-00'/('adaptive/pass-000' if mode == 'adaptive' else mode)
        sequential = read(sequence_dir/'sequential-route.json')
        accepted = [s for s in sequential['steps'] if selected_admission(s) is not None]
        assert len(accepted) == sequential['completed_connections']
        for step in accepted:
            admission = selected_admission(step)
            assert admission['complete']
            assert admission['introduced_drc_design_violations'] == 0
        rows.append(dict(case=name, components=result['components'], nets=result['electrical_nets'],
                         admitted_nets=len(accepted), area_mm2=attempt['area_mm2'], native=native,
                         complete_layout=attempt['admitted'],
                         image=str((native_dir/'inspection-combined.svg').relative_to(root)),
                         details=f'{name}/index.html'))
    probe = read(root/'next-failure-probe/results.json')
    assert [r['exit_code'] for r in probe] == [0, 1]
    assert all(r['native']['drc_design_violations'] == 7 for r in probe)
    offset = read(root/'pad-offset-native/report.json')
    assert offset['new_candidate_added_clearance_violations'] == 0
    replay = read(root/'ecc83-offset-replay/report.json')
    assert len(replay['results']) == 9 and all(r['candidate_identical'] for r in replay['results'])
    controls = read(root/'extraction-controls-offset/results.json')
    assert controls['passed'] and controls['offset_copper_matches_native_shape_positions']
    for path in root.rglob('*.svg'):
        ET.parse(path)
    evidence = dict(cases=rows, offset_regression=offset, next_failure_probe=probe,
                    extraction_controls=controls, final_binary_replay=replay,
                    svg_files_checked=sum(1 for _ in root.rglob('*.svg')))
    (root/'audit.json').write_text(json.dumps(evidence, indent=2)+'\n')
    page = ['<!doctype html><meta charset="utf-8"><title>General project layout</title>',
            '<style>body{font:17px system-ui;margin:25px}img{max-width:100%}.pair{display:grid;grid-template-columns:1fr 1fr;gap:15px}td,th{padding:8px;border:1px solid #bbb}table{border-collapse:collapse}</style>',
            '<h1>General project placement and routing</h1>',
            '<p>Complete-board capability comes before local via optimization. ',
            'All source components and nets are retained; original orientations are fixed.</p>',
            '<table><tr><th>Case</th><th>Components</th><th>Admitted nets</th><th>Native opens</th><th>Design findings</th><th>Complete layout</th></tr>']
    for row in rows:
        cells = [row['case'], row['components'], f'{row["admitted_nets"]}/{row["nets"]}',
                 row['native']['selected_net_unconnected_items'], row['native']['drc_design_violations'], row['complete_layout']]
        page.append('<tr>'+''.join(f'<td>{html.escape(str(v))}</td>' for v in cells)+'</tr>')
    page.append('</table><p>Complex-board findings are seven retained annotation errors; each admitted net adds no findings. It earns no complete-layout or area credit.</p>')
    for row in rows:
        page.append(f'<h2>{html.escape(row["case"])}</h2><a href="{row["details"]}">Placement and routing stages</a><br><img src="{row["image"]}">')
    page.extend(['<h2>Pad copper offset regression</h2><p>The same cold placement and routing rules: correcting the ignored 0.4 mm copper offset removes the extra native clearance violation.</p>',
                 '<div class="pair"><div>Before: 0.2685 mm clearance; 0.3000 required<img src="offset-before-detail.svg"></div><div>After: no new native clearance findings<img src="offset-after-detail.svg"></div></div>',
                 '<h2>Next failure: routing interference</h2><p>Net-(D303-A) routes with no earlier copper and fails behind thirteen admitted nets. Both contexts have combined native previews.</p>',
                 '<div class="pair"><img src="next-failure-probe/empty-placement/preview.svg"><img src="next-failure-probe/after-13-nets/preview.svg"></div>',
                 '<p><a href="input-coverage.json">Five-family adapter coverage</a> · <a href="audit.json">Checked evidence</a> · <a href="processes.json">Process outcomes, including failed driver trials</a></p>'])
    (root/'index.html').write_text(''.join(page))


if __name__ == '__main__':
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('root', type=Path)
    args = parser.parse_args()
    report(args.root.resolve())
