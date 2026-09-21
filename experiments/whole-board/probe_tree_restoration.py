# Copyright (C) 2026 Toit contributors.
"""Compare tree attachment policies on a frozen failed restoration context."""
import argparse
import copy
import hashlib
import json
from pathlib import Path
import shutil

from audit_partial_ripup import isolated_snapshot
from probe_partial_ripup import read, write, run
from report_adaptive_routing import finding_counts, progress_admissible


def probe(args):
    out = args.output.resolve()
    out.mkdir(exist_ok=False)
    source, reference = args.source.resolve(), args.reference.resolve()
    binary = out/'pcb-maker'
    shutil.copy2(args.binary, binary)
    shutil.copy2(__file__, out/Path(__file__).name)
    routing = read(args.config)['diagnosis']['routing']
    if args.resolution is not None:
        factor = routing['resolution_mm']/args.resolution
        routing['resolution_mm'] = args.resolution
        routing['bend_cost'] = round(routing['bend_cost']*factor)
        routing['via_cost'] = round(routing['via_cost']*factor)
    if routing.get('routing_demand'):
        # The inserted target is now retained copper, not future demand.
        routing['routing_demand']['alternatives'] = [a for a in routing['routing_demand']['alternatives']
            if a['connection'].removeprefix('/') != args.inserted.removeprefix('/')]
    board_name = args.board_id+'.kicad_pcb'
    digest = lambda p: hashlib.sha256(p.read_bytes()).hexdigest()
    source_hash = digest(source/board_name)
    before = isolated_snapshot(source/board_name)
    baseline_drc = read(reference/'drc.json')
    before_opens = read(reference/'verification.json')['selected_net_unconnected_items']
    cases = [
        ('baseline', {}),
        ('rooted', dict(multi_terminal_routing='rooted_star', maximum_tree_attachment_searches=1,
                        tree_attachment_minimum_spacing_mm=0.0,tree_attachment_objective='length_then_vias')),
        ('diverse', dict(maximum_tree_attachment_searches=32,tree_attachment_minimum_spacing_mm=4.0)),
        ('expanded', dict(maximum_tree_attachment_searches=64)),
    ]
    report = dict(finished=False, source=str(source), reference=str(reference), connection=args.connection,
                  source_sha256=source_hash, executable_sha256=digest(binary), rows=[])
    for name, changes in cases:
        directory = out/name
        directory.mkdir()
        candidate = directory/'candidate'
        shutil.copytree(source,candidate)
        config = copy.deepcopy(routing)
        config.update(changes)
        write(directory/'config.json',config)
        row = dict(case=name)
        row['route'] = run(binary,['route-kicad-connection',source/board_name,args.connection,
                                  directory/'route.json',directory/'config.json'],directory/'route.log')
        if row['route']['exit_code'] == 0:
            row['apply'] = run(binary,['apply-kicad-route-candidate',source/board_name,directory/'route.json',
                                      candidate/board_name],directory/'apply.log')
            assert row['apply']['exit_code'] == 0
        row['verification'] = run(binary,['verify-kicad-rung',candidate,args.board_id],directory/'verify.log')
        native, drc = read(candidate/'verification.json'), read(candidate/'drc.json')
        row['native'] = native
        row['restored_progress'] = (row['route']['exit_code'] == 0
            and progress_admissible(native,drc,baseline_drc,True)
            and native['selected_net_unconnected_items'] < before_opens
            and not (finding_counts(drc['unconnected_items'])-finding_counts(baseline_drc['unconnected_items'])))
        after = isolated_snapshot(candidate/board_name)
        assert before['pose'] == after['pose']
        other = lambda s:{k:v for k,v in s['tracks'].items() if v['net'] != args.connection.removeprefix('/')}
        assert other(before) == other(after)
        assert digest(source/board_name) == source_hash
        row['unchanged_pose_and_unrelated_copper'] = True
        report['rows'].append(row)
        write(out/'report.json',report)
        print(name, 'route',row['route']['exit_code'],'progress',row['restored_progress'],flush=True)
    report['finished'] = True
    write(out/'report.json',report)


if __name__ == '__main__':
    parser = argparse.ArgumentParser(description=__doc__)
    for name in ['source','reference','config','binary','output']:
        parser.add_argument('--'+name,type=Path,required=True)
    for name in ['board-id','connection','inserted']:
        parser.add_argument('--'+name,required=True)
    parser.add_argument('--resolution',type=float)
    probe(parser.parse_args())
