#!/usr/bin/env python3
# Copyright (C) 2026 Toit contributors.
"""Replay every accepted cold route with a new executable and require exact equality.

This is a deterministic regression replay, not a new search-policy comparison.
The selected per-route configs come from the recorded run. Each matching board
reuses its verified rendering; the final project is verified again natively.
"""
import argparse
import json
from pathlib import Path
import shutil

from run import ROOT, command, digest, read, write


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('sequential_run', type=Path)
    parser.add_argument('output', type=Path)
    parser.add_argument('--executable', type=Path, default=ROOT/'target/release/pcb-maker')
    args = parser.parse_args()
    old = read(args.sequential_run/'sequential-route.json')
    assert old['termination'] == 'complete'
    out = args.output.resolve()
    out.mkdir(parents=True,exist_ok=False)
    exe = out/'pcb-maker'
    shutil.copy2(args.executable,exe)
    native = out/'native'
    shutil.copytree(old['source_directory'],native)
    board = native/(old['board_id']+'.kicad_pcb')
    assert digest(board) == old['source_board_sha256_before']
    report = {'scope':__doc__, 'executable_sha256':digest(exe),
              'source_sha256':digest(board), 'steps':[]}
    write(out/'replay.json',report)
    for step in old['steps']:
        index = step['selected_config_index']
        assert index is not None
        attempt = next(a for a in step['attempts'] if a['config_index']==index)
        expected_candidate = read(Path(attempt['candidate']))
        directory = out/f'step-{step["ordinal"]:03d}'
        directory.mkdir()
        write(directory/'config.json',expected_candidate['config'])
        result = command([exe,'route-kicad-connection',board,step['connection'],
                          directory/'candidate.json',directory/'config.json'],directory/'route.log')
        assert result['exit_code'] == 0
        assert read(directory/'candidate.json') == expected_candidate, step['connection']
        applied = command([exe,'apply-kicad-route-candidate',board,directory/'candidate.json',board],directory/'apply.log')
        assert applied['exit_code'] == 0
        expected_board = Path(attempt['native_admission']['directory'])/board.name
        assert digest(board) == digest(expected_board), step['connection']
        shutil.copy2(expected_board.parent/'preview.svg',directory/'preview.svg')
        report['steps'].append({'connection':step['connection'],'candidate_equal':True,
            'board_byte_equal':True,'board_sha256':digest(board),
            'native_opens':attempt['native_admission']['verification']['selected_net_unconnected_items'],
            'route_process':result,'preview':f'{directory.name}/preview.svg'})
        write(out/'replay.json',report)
        print(step['connection']+': candidate and board equal',flush=True)
    result = command([exe,'verify-kicad-rung',native,old['board_id']],out/'verify.log')
    report['verification_process'] = result
    report['verification'] = read(native/'verification.json')
    assert result['exit_code'] == 0 and report['verification']['complete']
    report['source_unchanged'] = digest(Path(old['source_directory'])/board.name)==report['source_sha256']
    assert report['source_unchanged']
    report['finished'] = True
    write(out/'replay.json',report)
    data = json.dumps(report['steps']).replace('<','\\u003c')
    (out/'index.html').write_text('<!doctype html><meta charset="utf-8"><title>Complete cold PIC routing</title>'
        '<style>body{font:18px system-ui;margin:25px}img{max-width:95vw;max-height:80vh}input{width:75vw}</style>'
        '<h1>Complete cold routing replay</h1><p>Every candidate and resulting board matches the recorded run. Final native verification passes.</p>'
        '<input id="step" type="range" min="0" max="'+str(len(report['steps'])-1)+'" value="0"><p id="caption"></p><img id="board">'
        '<p><a href="replay.json">Evidence</a> · <a href="native/preview.svg">Final board</a></p>'
        '<script>const rows='+data+'; const slider=document.getElementById("step");function show(){const r=rows[+slider.value];document.getElementById("board").src=r.preview;document.getElementById("caption").textContent=(+slider.value+1)+" / "+rows.length+": "+r.connection+"; remaining native opens: "+r.native_opens;}slider.oninput=show;show();</script>')


if __name__=='__main__':
    main()
