# Copyright (C) 2026 Toit contributors.
"""Compare audited history/boundary repair on an identical retained board.

Invoke after both repair actions are terminal and independently audited.
Trial coverage measures a counterfactual route, not a restored valid board.
"""
import argparse
import hashlib
import html
import json
import os
from pathlib import Path


def read(path):
    return json.loads(Path(path).read_text())


def compare(history, boundary, history_config, boundary_config, output):
    roots = [history.resolve(),boundary.resolve()]
    configs = [read(history_config),read(boundary_config)]
    before,after = [dict(c['diagnosis']) for c in configs]
    assert not before.pop('prioritize_boundary_blockers',False)
    assert after.pop('prioritize_boundary_blockers',False)
    assert before == after, 'diagnosis comparison changes more than priority'
    assert {k:v for k,v in configs[0].items() if k!='diagnosis'} == {
        k:v for k,v in configs[1].items() if k!='diagnosis'}
    reports = [read(root/'ripup.json') for root in roots]
    audits = [read(root/'audit.json') for root in roots]
    assert reports[0]['board_id'] == reports[1]['board_id']
    assert reports[0]['target_connection'] == reports[1]['target_connection']
    board = reports[0]['board_id']
    hashes = {}
    for suffix in ['kicad_pcb','kicad_pro','kicad_sch']:
        blobs = [(root/'source-unrouted-target'/f'{board}.{suffix}').read_bytes() for root in roots]
        assert blobs[0] == blobs[1], 'priority comparison has different native inputs'
        hashes[suffix] = hashlib.sha256(blobs[0]).hexdigest()
    assert reports[0]['diagnosis']['foreign_routed_connections'] == reports[1]['diagnosis']['foreign_routed_connections']
    rows = []
    for label,root,report,audit in zip(['history','boundary'],roots,reports,audits):
        assert audit['native_progress_gate_and_selection_reproduced']
        assert audit['yielding_priority_checked']
        assert report['selected_attempt'] == audit['selected_attempt']
        trials = report['diagnosis']['trials']
        outcomes = {a['yielding_connection'].removeprefix('/'):a['status'] for a in report['attempts']}
        selected = report['selected_attempt']
        chosen = None if selected is None else report['attempts'][selected]
        directory = root/'baseline-verification' if chosen is None else Path(chosen['artifact_directory'])
        rows.append(dict(mode=label,root=str(root),tested=len(trials),
            counterfactual_routes=sum(t['route_found'] for t in trials),
            restored_candidates=sum(a['status'] in ['complete','progress'] for a in report['attempts']),
            selected_yield=None if chosen is None else chosen['yielding_connection'],
            selected_changed_nets=[] if chosen is None else sorted(chosen['changed_routes']),
            selected_native=None if chosen is None else chosen['final_verification'],
            total_astar_expansions=report['total_route_expansions'],
            diagnosis_seconds=report['diagnosis']['elapsed_micros']/1e6,
            image=str(directory/'inspection-combined.svg'),
            trials=[dict(rank=i+1,nets=t['yielding_connections'],route_found=t['route_found'],
                         restoration_status=outcomes.get(t['yielding_connections'][0].removeprefix('/')))
                    for i,t in enumerate(trials)]))
    assert rows[0]['tested'] == rows[1]['tested'] == before['maximum_trials']
    result = dict(board_id=board,target=reports[0]['target_connection'],native_input_sha256=hashes,
                  changed_configuration='diagnosis.prioritize_boundary_blockers only',rows=rows,
                  scope='Equal diagnosis-trial caps and native inputs. Restoring more candidates may cost more work. One timing sample per mode, with concurrent work; no runtime-speedup claim.')
    output.mkdir(exist_ok=False)
    (output/'comparison.json').write_text(json.dumps(result,indent=2)+'\n')
    esc = lambda s:html.escape(str(s))
    page=['<!doctype html><meta charset="utf-8"><title>History versus boundary diagnosis</title>',
          '<style>body{font:17px system-ui;margin:2rem}td,th{padding:.5rem 1rem;border-bottom:1px solid #ccc;text-align:left}img{max-width:100%;max-height:75vh}</style>',
          '<h1>'+esc(result['target'])+': compare diagnosis priority</h1>',
          '<p>'+esc(result['scope'])+'</p><table><tr><th>Mode</th><th>Trials</th><th>Target routes found</th><th>Fully restored candidates</th><th>Selected yield</th></tr>']
    for row in rows:
        page.append('<tr>'+''.join('<td>'+esc(row[k])+'</td>' for k in ['mode','tested','counterfactual_routes','restored_candidates','selected_yield'])+'</tr>')
    page.append('</table>')
    for row in rows:
        page.append('<h2>'+esc(row['mode'])+'</h2><p><a href="'+esc(os.path.relpath(Path(row['root'])/'index.html',output))+'">Full repair and native audit</a></p>')
        page.append('<img src="'+esc(os.path.relpath(row['image'],output))+'">')
    page.append('<p><a href="comparison.json">Ranked coverage and provenance</a></p>')
    (output/'index.html').write_text(''.join(page))
    return result


if __name__ == '__main__':
    parser=argparse.ArgumentParser(description=__doc__)
    for name in ['history','boundary','history_config','boundary_config','output']:
        parser.add_argument(name,type=Path)
    args=parser.parse_args()
    result=compare(args.history,args.boundary,args.history_config,args.boundary_config,args.output)
    print(json.dumps({r['mode']:{k:r[k] for k in ['tested','counterfactual_routes','restored_candidates','selected_yield']} for r in result['rows']},indent=2))
