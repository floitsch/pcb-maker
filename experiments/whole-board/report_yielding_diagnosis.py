#!/usr/bin/env python3
# Copyright (C) 2026 Toit contributors.
"""Render terminal-specific failures and coverage from a completed diagnosis.

Input copper is the retained board, including the counterfactually yielded
nets. Rings show search endpoints, not admitted traces or obstacle locations.
"""
import argparse
from collections import Counter
import copy
import html
import json
import math
from pathlib import Path
import shutil
import xml.etree.ElementTree as ET


def write_report(evidence, copper, output):
    evidence, copper, output = map(Path, (evidence, copper, output))
    diagnosis = json.loads(evidence.read_text())
    output.mkdir(parents=True, exist_ok=True)
    # Keep render references portable and never modify the source board/views.
    shutil.copy2(copper, output/'retained-combined.svg')
    shutil.copy2(evidence, output/'diagnosis.json')
    namespace = 'http://www.w3.org/2000/svg'
    ET.register_namespace('', namespace)
    background = ET.parse(copper).getroot()
    x, y, width, height = map(float, background.attrib['viewBox'].split())
    contexts = {}

    def context_view(failure):
        context = (failure or {}).get('terminal_context')
        if not context:
            return None
        key = json.dumps(context, sort_keys=True)
        if key in contexts:
            return contexts[key]['view']
        name = f'context-{len(contexts):02d}.svg'
        svg = ET.Element(f'{{{namespace}}}svg', {
            'viewBox': background.attrib['viewBox'], 'width': '1200',
            'height': str(1200*height/width)})
        ET.SubElement(svg, 'title').text = 'Retained copper with search-only terminal context'
        ET.SubElement(svg, 'rect', x=str(x), y=str(y), width=str(width), height=str(height), fill='white')
        # Inline copper: browsers suppress external resources in an SVG used
        # as an HTML image, so a nested file reference would hide the board.
        group = ET.SubElement(svg, 'g', opacity='.45')
        for child in background:
            group.append(copy.deepcopy(child))
        reached = [p for p in context['reached_mm'] if p != context['root_mm']]
        points = [('root', context['root_mm'], '#1363bd')]
        points += [('reached', p, '#187647') for p in reached]
        points += [('pending', context['pending_mm'], '#bb1f52')]
        for label, (px, py), color in points:
            ET.SubElement(svg, 'circle', cx=str(px), cy=str(py), r='1.5', fill='none', stroke=color, **{'stroke-width': '.5'})
            text = ET.SubElement(svg, 'text', x=str(px+2), y=str(py-2), fill=color,
                                 **{'font-size': '1.5', 'font-family': 'sans-serif', 'paint-order': 'stroke', 'stroke': 'white', 'stroke-width': '.6'})
            text.text = f'{label} ({px:.3f}, {py:.3f})'
        ET.ElementTree(svg).write(output/name, encoding='unicode', xml_declaration=True)
        contexts[key] = {'view': name, 'terminal_context': context}
        return name

    base = {'yielding_connections': [], 'route_found': diagnosis['base_route_found'],
            'route_failure': diagnosis.get('base_route_failure'), 'error': diagnosis.get('base_error')}
    trials = diagnosis['trials']
    rows = []
    for trial in [base, *trials]:
        row = {k: trial.get(k) for k in ['yielding_connections', 'route_found', 'route_failure', 'error', 'route_expansions']}
        row['context_view'] = context_view(row['route_failure'])
        rows.append(row)
    n = len(diagnosis['foreign_routed_connections'])
    minimum = diagnosis.get('minimum_yielding_connections', 1)
    maximum = diagnosis['maximum_yielding_connections']
    total = sum(math.comb(n, k) for k in range(minimum, maximum+1))
    tested = [tuple(t['yielding_connections']) for t in trials]
    assert len(set(map(frozenset, tested))) == len(tested), 'Repeated counterfactual set'
    assert all(minimum <= len(t) <= maximum for t in tested), 'Trial outside declared cardinality'
    assert len(tested) <= diagnosis['maximum_trials']
    assert diagnosis['truncated'] == (len(tested) < total)
    report = {
        'schema_version': 1, 'target': diagnosis['target_connection'],
        'scope': __doc__, 'tested': len(trials), 'possible_sets': total,
        'truncated': diagnosis['truncated'],
        'successful_sets': [t['yielding_connections'] for t in trials if t['route_found']],
        'failure_kinds': dict(Counter((t.get('route_failure') or {}).get('kind', 'untyped') for t in trials if not t['route_found'])),
        'contexts': list(contexts.values()), 'rows': rows,
    }
    (output/'summary.json').write_text(json.dumps(report, indent=2)+'\n')
    esc = lambda s: html.escape(str(s))
    parts = ['<!doctype html><meta charset="utf-8"><title>Yielding diagnosis</title>',
             '<style>body{font:17px system-ui;margin:2rem;max-width:1300px}img{width:100%}td,th{text-align:left;padding:.4rem;border-bottom:1px solid #bbb}pre{white-space:pre-wrap}</style>',
             '<h1>'+esc(report['target'])+' obstruction diagnosis</h1>',
             f'<p>{len(trials)} of {total} possible removal sets tested; {len(report["successful_sets"])} counterfactual routes.</p>',
             '<p>These are provisional searches. Every displaced net must be restored before a routing improvement can be admitted. Failed bounded searches do not prove physical infeasibility.</p>',
             '<p>Copper below is the retained board, including yielded nets. Rings mark search endpoints; they do not identify the blocking copper.</p>',
             '<p><a href="summary.json">Compact JSON</a> · <a href="diagnosis.json">Full diagnosis</a></p>',
             '<img src="retained-combined.svg" alt="Retained combined copper; bodies hidden">']
    for context in contexts.values():
        c = context['terminal_context']
        point = lambda p: f'({p[0]:.3f}, {p[1]:.3f})'
        label = f'Root {point(c["root_mm"])}; {len(c["reached_mm"])} terminals reached; pending {point(c["pending_mm"])}'
        parts += ['<details><summary>'+esc(label)+'</summary><img src="'+context['view']+'" alt="Root, reached and pending terminals on retained copper"></details>']
    parts.append('<table><tr><th>Removed nets</th><th>Search result</th><th>Terminal context</th></tr>')
    for row in rows:
        status = 'Route found; restoration required' if row['route_found'] else row['error']
        link = '<a href="'+row['context_view']+'">View endpoints</a>' if row['context_view'] else 'Unavailable'
        parts.append('<tr><td>'+esc(', '.join(row['yielding_connections']) or 'None (baseline)')+'</td><td>'+esc(status)+'</td><td>'+link+'</td></tr>')
    parts.append('</table>')
    (output/'index.html').write_text(''.join(parts))
    return report


if __name__ == '__main__':
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('evidence', type=Path)
    parser.add_argument('combined_copper', type=Path)
    parser.add_argument('output', type=Path)
    args = parser.parse_args()
    report = write_report(args.evidence, args.combined_copper, args.output)
    print(json.dumps({k: report[k] for k in ['tested', 'possible_sets', 'successful_sets', 'failure_kinds']}))
