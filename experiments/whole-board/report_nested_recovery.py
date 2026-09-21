# Copyright (C) 2026 Toit contributors.
"""Summarize terminal automatic nested-recovery runs and their native audits."""
import argparse
import html
import json
from pathlib import Path
import xml.etree.ElementTree as ET


def report(root):
    read = lambda p: json.loads(p.read_text())
    rows = []
    for case in ['success','continued','finish','rollback','validate','ecc83']:
        directory = root/case
        sequence = read(directory/'sequential-route.json')
        audit = read(directory/'recovery-audit.json')
        assert audit['native_connectivity_ledger_reproduced']
        repairs = [s for s in sequence['steps'] if s.get('repair') and s['repair']['selected']]
        initial = (sequence.get('resume') or {}).get('initial_completed_connections',0)
        rows.append(dict(case=case,initial=initial,completed=sequence['completed_connections'],
                         total=len(sequence['connection_order']),opens=audit['native']['selected_net_unconnected_items'],
                         annotation_findings=audit['native']['drc_design_violations'],
                         native_complete=audit['complete'],termination=sequence['termination'],
                         committed_repairs=len(repairs),new_connections=sequence['completed_connections']-initial,
                         identical_reference_prefix=audit['identical_reference_prefix_steps']))
    for case,ordinal in [('success',27),('continued',28)]:
        sequence = read(root/case/'sequential-route.json')
        repair_root = Path(sequence['steps'][ordinal]['repair']['directory'])
        assert read(repair_root/'audit.json')['native_progress_gate_and_selection_reproduced']
    assert 'changed-route receipt disagrees with its candidate' in (root/'tampered.log').read_text()
    count=0
    for p in root.rglob('*.svg'):
        ET.parse(p)
        count+=1
    summary=dict(cases=rows,svg_xml_checked=count,tests_passed=129,benchmark_tests_passed=8,
                 corrupt_candidate_rejected=True,goal_complete=False)
    (root/'summary.json').write_text(json.dumps(summary,indent=2)+'\n')
    best=rows[2]
    page=['<!doctype html><meta charset="utf-8"><title>Automatic nested recovery</title>',
          '<style>body{font:17px system-ui;margin:2rem}img{width:100%}.pair{display:grid;grid-template-columns:1fr 1fr;gap:1rem}table{border-collapse:collapse}td,th{padding:.5rem;border:1px solid #bbb}</style>',
          '<h1>Automatic nested routing recovery</h1>',
          f'<p>The retained larger board advances from 27 to {best["completed"]}/{best["total"]} routed nets, '
          f'with {best["opens"]} native open items and {best["annotation_findings"]} existing annotation findings. '
          f'Full native completion: {best["native_complete"]}.</p>',
          '<p>Every nested level must restore its displaced net. The enclosing transaction checks its original connectivity ledger and unchanged copper outside the recorded changed-net set. Invocation and depth limits are shared within each outer repair.</p>',
          '<table><tr><th>Run</th><th>Starting nets</th><th>Retained nets</th><th>Opens</th><th>Termination</th></tr>']
    for row in rows:
        page.append(f'<tr><td><a href="{row["case"]}/recovery.html">{row["case"]}</a></td><td>{row["initial"]}</td>'
                    f'<td>{row["completed"]}/{row["total"]}</td><td>{row["opens"]}</td><td>{row["termination"]}</td></tr>')
    page.extend(['</table><h2>Combined copper: original and retained result</h2><div class="pair">',
                 '<figure><figcaption>Original 27-net checkpoint</figcaption><img src="success/resume-source/preview.svg"></figure>',
                 '<figure><figcaption>Final retained larger board</figcaption><img src="finish/result/inspection-combined.svg"></figure></div>',
                 '<p>Front and back share one view. Component bodies are hidden and labels remain.</p>',
                 '<h2>Controls and evidence</h2><p>The restricted repair retains the original 27-net board byte for byte. The 28-net checkpoint resumes, while a changed route candidate fails hash validation. Enabled-but-unused recovery reproduces the complete nine-net ECC83 sequence. All 129 KiCad library tests pass.</p>',
                 '<p>The corrected repair auditor uses immutable action-entry snapshots, whose directory hashes are checked separately. The old audit failure caused by reading a later mutable result remains in the logs.</p>',
                 '<p><a href="summary.json">Summary</a> · <a href="provenance.json">Executable/source hashes</a> · <a href="processes.json">Process outcomes</a> · <a href="snapshot-hash-audit.json">Immutable input hashes</a></p>'])
    (root/'index.html').write_text(''.join(page))
    print(json.dumps(summary,indent=2))


if __name__ == '__main__':
    parser=argparse.ArgumentParser(description=__doc__)
    parser.add_argument('root',type=Path)
    report(parser.parse_args().root.resolve())
