#!/usr/bin/env python3
# Copyright (C) 2026 Toit contributors.
"""Try both restoration orders for successful two-net counterfactuals.

This bounded experiment stops at the first independently checked improvement.
It does not commit to a sequential routing journal or claim board completion.
Every attempted native stage is verified and rendered, including failed routes.
"""
import argparse
import hashlib
import itertools
from pathlib import Path
import shutil

from audit_partial_ripup import isolated_snapshot
from probe_partial_ripup import read, write, run
from render_inspection_layers import render
from report_adaptive_routing import finding_counts, progress_admissible


def probe(args):
    source, out = args.source.resolve(), args.output.resolve()
    out.mkdir(exist_ok=False)
    binary = out/'pcb-maker'
    shutil.copy2(args.binary, binary)
    shutil.copy2(__file__, out/Path(__file__).name)
    diagnosis = read(args.diagnosis)
    routing = read(args.config)['routing']
    write(out/'routing.json', routing)
    board = args.board_id+'.kicad_pcb'
    assert Path(diagnosis['board']).resolve() == source/board
    digest = lambda p: hashlib.sha256(p.read_bytes()).hexdigest()
    original_hash = digest(source/board)
    before = isolated_snapshot(source/board)
    baseline, baseline_drc = read(source/'verification.json'), read(source/'drc.json')
    assert progress_admissible(baseline, baseline_drc, baseline_drc, False)
    report = dict(finished=False, source=str(source), source_sha256=original_hash,
                  target=diagnosis['target_connection'], baseline=baseline,
                  scope=__doc__, trials=[], selected_directory=None)

    def checkpoint():
        write(out/'report.json', report)

    def verify(directory, log):
        run(binary, ['verify-kicad-rung', directory, args.board_id], log)
        # Native serialization must preserve project rules exactly.
        assert (directory/(args.board_id+'.kicad_pro')).read_bytes() == (source/(args.board_id+'.kicad_pro')).read_bytes()
        render(directory, args.board_id)
        return read(directory/'verification.json'), read(directory/'drc.json')

    checkpoint()
    for trial in diagnosis['trials']:
        if not trial['route_found']:
            continue
        yielding = trial['yielding_connections']
        assert len(yielding) == 2
        case = out/f'pair-{trial["ordinal"]:03d}'
        case.mkdir()
        write(case/'target.json', trial['candidate'])
        write(case/'yielding.json', yielding)
        provisional = case/'target-only'
        shutil.copytree(source, provisional)
        applied = run(binary, ['apply-kicad-route-candidate', source/board, case/'target.json',
                              provisional/board, case/'yielding.json'], case/'apply-target.log')
        assert applied['exit_code'] == 0
        native, drc = verify(provisional, case/'verify-target.log')
        row = dict(yielding=yielding, provisional=str(provisional), provisional_native=native, orders=[])
        report['trials'].append(row)
        checkpoint()
        if not progress_admissible(native, drc, baseline_drc, False):
            row['rejected'] = 'Target counterfactual failed native design checks'
            continue
        for ordinal, order in enumerate(itertools.permutations(yielding)):
            order_row = dict(order=list(order), stages=[], admitted=False)
            row['orders'].append(order_row)
            parent = provisional
            for stage, net in enumerate(order):
                directory = case/f'order-{ordinal}-stage-{stage}'
                shutil.copytree(parent, directory)
                candidate = case/f'order-{ordinal}-stage-{stage}.json'
                routed = run(binary, ['route-kicad-connection', parent/board, net, candidate,
                                      out/'routing.json'], case/f'order-{ordinal}-stage-{stage}-route.log')
                if routed['exit_code'] == 0:
                    applied = run(binary, ['apply-kicad-route-candidate', parent/board, candidate,
                                          directory/board], case/f'order-{ordinal}-stage-{stage}-apply.log')
                    assert applied['exit_code'] == 0
                native, drc = verify(directory, case/f'order-{ordinal}-stage-{stage}-verify.log')
                good = routed['exit_code'] == 0 and progress_admissible(native, drc, baseline_drc, False)
                order_row['stages'].append(dict(net=net, route=routed, native=native, directory=str(directory), valid=good))
                checkpoint()
                print(yielding, order, net, 'valid', good, 'opens', native['selected_net_unconnected_items'], flush=True)
                if not good:
                    break
                parent = directory
            else:
                # No added opens anywhere, all target opens removed; retained
                # original connections were complete before the counterfactual.
                removed = finding_counts(baseline_drc['unconnected_items'])-finding_counts(drc['unconnected_items'])
                added = finding_counts(drc['unconnected_items'])-finding_counts(baseline_drc['unconnected_items'])
                good = not added and sum(removed.values()) == args.expected_open_reduction
                good = good and native['selected_net_unconnected_items'] == baseline['selected_net_unconnected_items']-args.expected_open_reduction
                if good:
                    after = isolated_snapshot(parent/board)
                    changed = {n.removeprefix('/') for n in [*yielding, diagnosis['target_connection']]}
                    unrelated = lambda s: {u:t for u,t in s['tracks'].items() if t['net'] not in changed}
                    assert unrelated(before) == unrelated(after)
                    assert before['pose'] == after['pose']
                    for suffix in ['kicad_pro', 'kicad_sch']:
                        assert (source/(args.board_id+'.'+suffix)).read_bytes() == (parent/(args.board_id+'.'+suffix)).read_bytes()
                    for track in after['tracks'].values():
                        if track['net'] not in changed:
                            continue
                        rules = routing['connection_rules'][track['net']]
                        assert track['width'] == round(rules['via_size_mm' if 'drill' in track else 'trace_width_mm']*1e6)
                        if 'drill' in track:
                            assert track['drill'] == round(rules['via_drill_mm']*1e6)
                            assert track['back_width'] == track['width']
                    order_row.update(admitted=True, unrelated_copper_and_poses_preserved=True, native_dimensions_match=True)
                    report['selected_directory'] = str(parent)
                    break
            checkpoint()
        if report['selected_directory']:
            break
    assert digest(source/board) == original_hash
    report.update(finished=True, source_unchanged=True)
    checkpoint()
    links = ['<!doctype html><meta charset="utf-8"><title>Pair restoration</title><h1>Pair restoration experiment</h1><p>Both removed nets must be restored before progress is accepted.</p><a href="report.json">Evidence</a>']
    for row in report['trials']:
        for directory in [row['provisional'], *[s['directory'] for o in row['orders'] for s in o['stages']]]:
            relative = Path(directory).relative_to(out)
            links.append(f'<p><a href="{relative}/inspection-combined.svg">{relative}: combined copper</a></p>')
    (out/'index.html').write_text(''.join(links))
    print('Selected:', report['selected_directory'], flush=True)


if __name__ == '__main__':
    parser = argparse.ArgumentParser(description=__doc__)
    for name in ['source', 'output', 'binary', 'diagnosis', 'config']:
        parser.add_argument('--'+name, required=True, type=Path)
    parser.add_argument('--board-id', required=True)
    parser.add_argument('--expected-open-reduction', required=True, type=int)
    probe(parser.parse_args())
