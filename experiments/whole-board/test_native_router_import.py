# Copyright (C) 2026 Toit contributors.
"""Native session-import controls: preserve fixed poses and reject changed inputs.

Run against an archived successful compare_native_router.py import. Each
negative control retains and renders the unchanged native source.
"""
import argparse
import json
from pathlib import Path
import re
import shutil
import sys

from audit_partial_ripup import isolated_snapshot
from render_inspection_layers import render
from run import command, digest, read, write


def controls(comparison, output):
    output.mkdir(exist_ok=False)
    report=read(comparison/'report.json')
    assert report['status']=='finished' and report['imported']
    name=report['native']['board_id']
    source=comparison/'source'
    board_name=name+'.kicad_pcb'
    before=isolated_snapshot(source/board_name)
    after=isolated_snapshot(comparison/'result'/board_name)
    assert before['pose']==after['pose'] and len(after['tracks'])>len(before['tracks'])
    receipt=read(comparison/'result/import-pose-restoration.json')
    assert receipt['restored'], 'This control requires off-grid source coordinates'
    session=(comparison/'output.ses').read_text()
    place=re.compile(r'(\(place\s+\S+\s+)(-?\d+)(\s+-?\d+\s+(?:front|back)\s+)(-?\d+)(\))')
    assert place.search(session)
    cases=[
        ('moved',place.sub(lambda m:m[1]+str(int(m[2])+100)+m[3]+m[4]+m[5],session,count=1),
         'beyond exchange rounding'),
        ('rotated',place.sub(lambda m:m[1]+m[2]+m[3]+str((int(m[4])+90)%360)+m[5],session,count=1),
         'orientation/side'),
        ('resolution',session.replace('(resolution um 10)','(resolution um 1)'),
         'Unsupported SES resolution')]
    results=[]
    for label,content,error in cases:
        directory=output/label
        shutil.copytree(source,directory)
        session_path=directory/'mutated.ses'
        session_path.write_text(content)
        board=directory/board_name
        original_hash=digest(board)
        result=command([sys.executable,Path(__file__).with_name('compare_native_router.py'),
                        '--native','import',board,session_path],directory/'import.log')
        assert result['exit_code']!=0 and error in (directory/'import.log').read_text()
        assert digest(board)==original_hash==report['source_sha256']
        assert not (directory/'import-pose-restoration.json').exists()
        render(directory,name)
        results.append(dict(control=label,expected_error=error,source_unchanged=True,**result))
    archived=output/'tampered-archive'
    archived.mkdir()
    for name_part in ['report.json','input.dsn','output.ses']:
        shutil.copy2(comparison/name_part,archived/name_part)
    with (archived/'output.ses').open('a') as file:
        file.write('\n')
    replay=output/'tampered-replay'
    input_args=['--sequence',report['sequence']] if report['sequence'] else ['--source',report['source_original'],'--board-id',name]
    adapter_args=['--translate-edge-rule'] if report.get('edge_translation') else []
    if report.get('layer_adaptation'):adapter_args.append('--all-copper-signal-layers')
    failure=command([sys.executable,Path(__file__).with_name('compare_native_router.py'),
        *input_args,'--binary',comparison/'pcb-maker','--output',replay,
        '--replay-session-from',archived,*adapter_args],output/'tampered-replay.log')
    failed=read(replay/'report.json')
    assert failure['exit_code']!=0 and failed['status']=='failed'
    assert 'Session replay hash differs' in failed['error']
    assert digest(replay/'source'/board_name)==report['source_sha256']
    assert (replay/'source/inspection-combined.svg').exists() and not (replay/'result').exists()
    result=dict(positive_exact_pose_and_copper_import=True,negative_controls=results,
                tampered_session_replay_rejected=True,terminal_failure_recorded=True,
                source_report_sha256=digest(comparison/'report.json'))
    write(output/'report.json',result)
    print(json.dumps(result))


if __name__=='__main__':
    p=argparse.ArgumentParser(description=__doc__)
    p.add_argument('comparison',type=Path)
    p.add_argument('output',type=Path)
    a=p.parse_args()
    controls(a.comparison.resolve(),a.output.resolve())
