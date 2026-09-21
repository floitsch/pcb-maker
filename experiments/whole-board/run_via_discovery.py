# Copyright (C) 2026 Toit contributors.
"""Discover and test a bounded ranked portfolio on a complete native board.

No via coordinates, net choices or source route candidates are supplied.
Every native verification/refinement stage automatically renders its board.
"""
import argparse
import hashlib
import json
from pathlib import Path
import shutil
import subprocess
import time


def run(args):
    source=args.source.resolve();out=args.output.resolve()
    if out.exists() or out.is_relative_to(source):raise ValueError('Output must be fresh and outside source')
    out.mkdir(parents=True)
    exe=out/'pcb-maker';shutil.copy2(args.binary.resolve(),exe)
    shutil.copytree(source,out/'source')
    board=out/'source'/f'{args.board_id}.kicad_pcb'
    original_hash=hashlib.sha256(board.read_bytes()).hexdigest()
    config=json.loads(args.config.read_text());(out/'discovery-config.json').write_text(json.dumps(config,indent=2)+'\n')
    steps=[];trials=[]
    def command(name,*arguments,required=True):
        start=time.monotonic()
        with (out/f'{name}.log').open('w') as log:
            process=subprocess.run([str(exe),*map(str,arguments)],stdout=log,stderr=subprocess.STDOUT)
        steps.append({'name':name,'exit_code':process.returncode,'elapsed_seconds':time.monotonic()-start})
        (out/'steps.json').write_text(json.dumps(steps,indent=2)+'\n')
        if required and process.returncode:raise RuntimeError(f'{name} failed; see {name}.log')
        return process.returncode
    command('verify-source','verify-kicad-rung',board.parent,args.board_id)
    command('source-statistics','inspect-kicad-board',board)
    baseline=json.loads((out/'source-statistics.log').read_text())
    command('discovery','discover-kicad-vias',board,out/'scan',out/'discovery-config.json')
    discovery=json.loads((out/'scan/discovery.json').read_text())
    penalty=config['via_penalty_mm']
    def score(stats):return stats['stored_segment_length_mm']+penalty*stats['vias']
    selected=board.parent;selected_stats=baseline
    for action in discovery['actions'][:args.maximum_repairs]:
        rank=action['rank'];artifact=Path(action['artifacts']);trial=out/f'rank-{rank:03}'
        code=command(f'refine-{rank:03}','refine-kicad-vias',board.parent,args.board_id,artifact/'candidate.json',trial,artifact/'config.json',required=False)
        row={'rank':rank,'connection':action['connection'],'target_label':action['target_label'],'exit_code':code,'selected':False}
        if code==0:
            refinement=json.loads((trial/'refinement.json').read_text())
            stats=refinement['result_statistics'];native=json.loads((trial/'result/verification.json').read_text())
            # Rank trials independently from the original board. No score
            # credit for merely normalizing an imported candidate inside pads.
            accepted=(native['complete'] and stats['vias']<baseline['vias'] and score(stats)<score(selected_stats))
            row.update(statistics=stats,native=native,improved_incumbent=accepted)
            if accepted:selected=trial/'result';selected_stats=stats
        trials.append(row)
        (out/'trials.json').write_text(json.dumps(trials,indent=2)+'\n')
    assert hashlib.sha256(board.read_bytes()).hexdigest()==original_hash
    assert hashlib.sha256((source/board.name).read_bytes()).hexdigest()==original_hash
    shutil.copytree(selected,out/'result')
    for row in trials:row['selected']=selected==out/f'rank-{row["rank"]:03}'/'result'
    report={'source':str(source),'source_sha256':original_hash,'source_statistics':baseline,'result_statistics':selected_stats,
            'selected':str(selected),'trials':trials,'steps':steps,'discovery_coverage':discovery['coverage'],
            'generated_actions':discovery['generated_actions'],'source_unchanged':True,'via_penalty_mm':penalty,
            'selection':'Original-board baseline; fewer vias and lower whole-board weighted cost; complete native verification required.',
            'executable_sha256':hashlib.sha256(exe.read_bytes()).hexdigest()}
    (out/'report.json').write_text(json.dumps(report,indent=2)+'\n')
    html=['<!doctype html><meta charset="utf-8"><title>Automatic via discovery and refinement</title>',
          '<style>body{font:17px system-ui;margin:25px}.pair{display:grid;grid-template-columns:1fr 1fr;gap:16px}img{width:100%}td,th{padding:8px;border:1px solid #aaa}table{border-collapse:collapse}</style>',
          '<h1>Automatic via discovery and refinement</h1>',
          f'<p>Vias: {baseline["vias"]} → {selected_stats["vias"]}. Stored track: {baseline["stored_segment_length_mm"]:.3f} → {selected_stats["stored_segment_length_mm"]:.3f} mm. {len(trials)} ranked candidates tested. Both endpoint boards are native-complete.</p>',
          '<div class="pair"><div><h2>Original</h2><img src="source/preview.svg"></div><div><h2>Selected</h2><img src="result/preview.svg"></div></div>',
          '<p><a href="scan/discovery.json">Ranked opportunities and coverage</a> · <a href="report.json">Full report</a> · <a href="steps.json">Timing and process status</a></p>']
    (out/'index.html').write_text(''.join(html))
    print(f'Vias {baseline["vias"]} -> {selected_stats["vias"]}; {len(trials)} ranked candidates tested.')


if __name__=='__main__':
    parser=argparse.ArgumentParser(description=__doc__)
    parser.add_argument('source',type=Path);parser.add_argument('board_id');parser.add_argument('config',type=Path);parser.add_argument('output',type=Path)
    parser.add_argument('--binary',type=Path,default=Path('target/release/pcb-maker'))
    parser.add_argument('--maximum-repairs',type=int,default=2)
    args=parser.parse_args()
    if args.maximum_repairs<0:parser.error('--maximum-repairs must be nonnegative')
    run(args)
