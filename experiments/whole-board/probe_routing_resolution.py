# Copyright (C) 2026 Toit contributors.
"""Probe finer-grid insertion on a frozen native parent; always verify/render."""
import argparse
import copy
import json
from pathlib import Path
import shutil

from probe_partial_ripup import read, write, run
from report_adaptive_routing import finding_counts, progress_admissible


def probe(args):
    out=args.output.resolve()
    out.mkdir(exist_ok=False)
    source=args.source.resolve()
    config=read(args.config)
    baseline=read(source/'verification.json')
    baseline_drc=read(source/'drc.json')
    binary=out/'pcb-maker'
    shutil.copy2(args.binary,binary)
    shutil.copy2(__file__,out/Path(__file__).name)
    rows=[]
    for resolution in args.resolutions:
        directory=out/f'grid-{resolution:g}'
        directory.mkdir()
        candidate=directory/'candidate'
        shutil.copytree(source,candidate)
        routing=copy.deepcopy(config)
        factor=routing['resolution_mm']/resolution
        routing['resolution_mm']=resolution
        # Preserve the old physical bend/via tradeoff when scaling grid cells.
        routing['bend_cost']=round(routing['bend_cost']*factor)
        routing['via_cost']=round(routing['via_cost']*factor)
        write(directory/'config.json',routing)
        board_name=args.board_id+'.kicad_pcb'
        routed=run(binary,['route-kicad-connection',source/board_name,args.connection,
            directory/'route.json',directory/'config.json'],directory/'route.log')
        applied=None
        if routed['exit_code']==0:
            applied=run(binary,['apply-kicad-route-candidate',source/board_name,directory/'route.json',
                candidate/board_name],directory/'apply.log')
            assert applied['exit_code']==0
        verification=run(binary,['verify-kicad-rung',candidate,args.board_id],directory/'verify.log')
        native=read(candidate/'verification.json')
        drc=read(candidate/'drc.json')
        admitted=(routed['exit_code']==0 and progress_admissible(native,drc,baseline_drc,args.allow_annotations)
            and native['selected_net_unconnected_items']<baseline['selected_net_unconnected_items']
            and not(finding_counts(drc['unconnected_items'])-finding_counts(baseline_drc['unconnected_items'])))
        rows.append(dict(resolution_mm=resolution,route=routed,apply=applied,verification=verification,
                         native=native,admitted_progress=admitted))
        write(out/'report.json',dict(finished=False,baseline=baseline,connection=args.connection,rows=rows))
        print(resolution,'admitted',admitted,'opens',native['selected_net_unconnected_items'],flush=True)
    write(out/'report.json',dict(finished=True,baseline=baseline,connection=args.connection,rows=rows))


if __name__=='__main__':
    parser=argparse.ArgumentParser(description=__doc__)
    for name in ['source','config','binary','output']:
        parser.add_argument('--'+name,type=Path,required=True)
    parser.add_argument('--board-id',required=True)
    parser.add_argument('--connection',required=True)
    parser.add_argument('--resolutions',type=float,nargs='+',required=True)
    parser.add_argument('--allow-annotations',action='store_true')
    probe(parser.parse_args())
