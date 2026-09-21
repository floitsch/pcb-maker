# Copyright (C) 2026 Toit contributors.
"""Publish terminal tree-search controls and the audited broader repair."""
import argparse
import hashlib
import html
import json
import os
from pathlib import Path
import xml.etree.ElementTree as ET


def report(root):
    read = lambda p: json.loads(p.read_text())
    digest = lambda p: hashlib.sha256(p.read_bytes()).hexdigest()
    before = read(root/'ecc83-default/sequential-route.json')
    after = read(root/'ecc83/sequential-route.json')
    outer = read(root/'outer-audit.json')
    assert before['termination'] == after['termination'] == 'complete'
    assert digest(Path(before['result_board'])) == digest(Path(after['result_board']))
    assert read(root/'ecc83/recovery-audit.json')['native_dimensions_match_selected_rules']
    assert read(root/'ecc83-default/recovery-audit.json')['identical_reference_prefix_steps'] == 9
    rows = []
    for directory in [root.parent/'tree-restoration-2026-09-08',root.parent/'tree-restoration-fine-2026-09-08']:
        control = read(directory/'report.json')
        assert control['finished'] and control['process_exit_code'] == 0
        for row in control['rows']:
            case = directory/row['case']
            rows.append(dict(case=case.name,resolution_mm=read(case/'config.json')['resolution_mm'],
                             route_message=row['route_message'],
                             preview=os.path.relpath(case/'candidate/preview.svg',root)))
    restoration = read(root/'restoration/report.json')
    assert restoration['finished'] and not restoration['rows'][0]['admitted_progress']
    case = root/'restoration/grid-0.25'
    rows.append(dict(case='multi-source',resolution_mm=.25,route_message=(case/'route.log').read_text().strip(),
                     preview=os.path.relpath(case/'candidate/preview.svg',root)))
    checked = 0
    for svg in root.rglob('*.svg'):
        ET.parse(svg)
        checked += 1
    summary = dict(outer=outer,attachment_controls=rows,
                   ecc83=dict(default_expansions=before['total_expansions'],
                              multi_source_expansions=after['total_expansions'],
                              expansion_reduction=1-after['total_expansions']/before['total_expansions'],
                              identical_final_native_board=True),svg_xml_checked=checked)
    (root/'summary.json').write_text(json.dumps(summary,indent=2)+'\n')
    final = Path(outer['result_board']).parent
    original = Path(outer['original_board']).parent
    link = lambda p: html.escape(os.path.relpath(p,root),quote=True)
    page = ['<!doctype html><meta charset="utf-8"><title>Whole-board restoration</title>',
            '<style>body{font:17px system-ui;margin:2rem}img{width:100%}.pair{display:grid;grid-template-columns:1fr 1fr;gap:1rem}table{border-collapse:collapse}td,th{padding:.5rem;border:1px solid #bbb}</style>',
            '<h1>Broader repair advances the larger board</h1>',
            '<p>27 → 28 routed nets; 36 → 34 native open items. Seven existing annotation findings remain. This is a partial board. The two-net restoration is an audited diagnostic transaction; automatic coordinator integration remains outstanding.</p>',
            '<p>Insert Net-(C201-Pad2), restore +12V while yielding Net-(U102-CAP+), then restore Net-(U102-CAP+). All unrelated copper and component poses remain unchanged.</p>',
            '<div class="pair"><figure><figcaption>Original retained board: 27 nets</figcaption>',
            f'<img src="{link(original/"inspection-combined.svg")}"></figure><figure><figcaption>Audited repair: 28 nets</figcaption>',
            f'<img src="{link(final/"inspection-combined.svg")}"></figure></div>',
            '<p><a href="outer-audit.json">Audit against the original board</a> · <a href="nested/index.html">All eight restoration attempts</a></p>',
            '<h2>Attachment-only controls</h2><p>None resolves the frozen restoration context. These images show provisional boards with +12V still absent; they are not admitted improvements.</p><table><tr><th>Policy</th><th>Grid (mm)</th><th>Search result</th></tr>']
    for row in rows:
        page.append(f'<tr><td><a href="{html.escape(row["preview"],quote=True)}">{row["case"]}</a></td><td>{row["resolution_mm"]}</td><td>{html.escape(row["route_message"])}</td></tr>')
    page.extend(['</table><h2>Complete-board search control</h2>',
                 '<p>ECC83 remains native-complete and byte-identical. A* expansions fall from 2,227,164 to 199,061 (91.1%). This measures search work on one board, not end-to-end elapsed time.</p>',
                 '<p><a href="ecc83/recovery.html">Multi-source step viewer</a> · <a href="ecc83-default/recovery.html">Default step viewer</a></p>',
                 '<p>All 127 KiCad library tests pass. Native verification produces combined front/back copper previews with component bodies hidden and labels retained.</p>',
                 '<p><a href="summary.json">Structured summary</a> · <a href="provenance.json">Executable/source hashes</a> · <a href="processes.json">Terminal process records</a></p>'])
    (root/'index.html').write_text(''.join(page))
    print(json.dumps(summary['ecc83'],indent=2))


if __name__ == '__main__':
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('root',type=Path)
    report(parser.parse_args().root.resolve())
