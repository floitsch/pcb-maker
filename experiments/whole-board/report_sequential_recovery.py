# Copyright (C) 2026 Toit contributors.
"""Summarize audited recovery runs, distinguishing interrupted checkpoints."""
import argparse
import html
import json
from pathlib import Path


def read(path):
    return json.loads(path.read_text())


def report(root):
    processes=read(root/'processes.json')
    assert processes['demand']['exit_code']==1
    assert processes['ecc83']['exit_code']==0
    assert processes['adaptive']['status']=='interrupted'
    assert not (root/'adaptive/adaptive-routing.json').exists()
    rows=[]
    for name,directory,status in [
        ('Demand-guided recovery','demand','finished; routing incomplete'),
        ('ECC83 control','ecc83/pass-000','finished; native complete'),
        ('No-demand checkpoint','adaptive/pass-000','interrupted; excluded from finished comparisons')]:
        audit=read(root/directory/'recovery-audit.json')
        rows.append(dict(name=name,directory=directory,status=status,completed_nets=audit['completed_nets'],
                         native=audit['native'],committed_repairs=audit['committed_repairs'],
                         unchanged_reference_prefix=audit['identical_reference_prefix_steps']))
    assert rows[0]['completed_nets']==27 and rows[0]['native']['selected_net_unconnected_items']==36
    assert rows[0]['unchanged_reference_prefix']==23 and rows[1]['unchanged_reference_prefix']==9
    probes={name:read(root/name/'report.json') for name in ['finer-target','finer-restoration']}
    assert all(p['finished'] and not any(row['admitted_progress'] for row in p['rows']) for p in probes.values())
    (root/'integration-summary.json').write_text(json.dumps(dict(runs=rows,grid_probes=probes),indent=2)+'\n')
    page=['<!doctype html><meta charset="utf-8"><title>Automatic routing recovery</title>',
          '<style>body{font:17px system-ui;margin:25px}table{border-collapse:collapse}td,th{padding:8px;border:1px solid #aaa}img{max-width:95vw;max-height:85vh}</style>',
          '<h1>Automatic routing recovery</h1><p>Recovery continues the retained routing prefix. The larger completed run improves from 23 nets / 42 opens to 27 nets / 36 opens. Seven source annotation findings remain.</p>',
          '<table><tr><th>Case</th><th>Process outcome</th><th>Routed nets</th><th>Native opens</th><th>Repairs committed</th><th>Native complete</th></tr>']
    for row in rows:
        page.append(f'<tr><td><a href="{row["directory"]}/recovery.html">{html.escape(row["name"])}</a></td>'
                    f'<td>{html.escape(row["status"])}</td><td>{row["completed_nets"]}</td>'
                    f'<td>{row["native"]["selected_net_unconnected_items"]}</td><td>{row["committed_repairs"]}</td><td>{row["native"]["complete"]}</td></tr>')
    page.append('</table><p>The interrupted run retains a verified checkpoint; its in-flight repair was not committed. Its unfinished search is not a negative result for that routing policy.</p>')
    for row in rows:
        page.append(f'<h2>{html.escape(row["name"])}</h2><img src="{row["directory"]}/result/inspection-combined.svg">')
    page.append('<p><a href="integration-summary.json">Summary evidence</a> · <a href="processes.json">Process outcomes</a> · <a href="binaries.json">Executable and sources</a></p>')
    (root/'index.html').write_text(''.join(page))


if __name__=='__main__':
    parser=argparse.ArgumentParser(description=__doc__)
    parser.add_argument('root',type=Path)
    report(parser.parse_args().root.resolve())
