# Copyright (C) 2026 Toit contributors.
"""Restore each single-net yielding counterfactual on an incomplete native board.

This is diagnostic evidence, not an alternate complete-layout admission gate.
Every materialized stage receives native verification and its automatic render.
"""
import argparse
import copy
import json
from pathlib import Path
import shutil
import subprocess
import time

from report_adaptive_routing import finding_counts, progress_admissible


def read(path):
    return json.loads(path.read_text())


def write(path, value):
    path.write_text(json.dumps(value, indent=2) + '\n')


def run(binary, args, log):
    started = time.monotonic()
    with log.open('w') as output:
        result = subprocess.run([str(binary), *map(str, args)], stdout=output, stderr=subprocess.STDOUT)
    return dict(exit_code=result.returncode, seconds=time.monotonic()-started, log=str(log))


def probe(args):
    out = args.output.resolve()
    out.mkdir(exist_ok=False)
    source = args.source.resolve()
    diagnosis = read(args.diagnosis)
    config = read(args.config)['routing']
    target = diagnosis['target_connection']
    board_name = args.board_id + '.kicad_pcb'
    assert Path(diagnosis['board']).resolve() == source/board_name
    binary = out/'pcb-maker'
    shutil.copy2(args.binary, binary)
    shutil.copy2(__file__, out/Path(__file__).name)
    shutil.copytree(source, out/'source')
    baseline_process = run(binary, ['verify-kicad-rung', out/'source', args.board_id], out/'baseline.log')
    baseline = read(out/'source/verification.json')
    baseline_drc = read(out/'source/drc.json')
    assert progress_admissible(baseline, baseline_drc, baseline_drc, args.allow_annotations)
    rows = []
    report = dict(finished=False, source=str(source), target=target, baseline=baseline,
                  baseline_process=baseline_process, trials=rows)
    for trial in diagnosis['trials']:
        if not trial['route_found']:
            continue
        assert len(trial['yielding_connections']) == 1
        directory = out/f'trial-{trial["ordinal"]:03}'
        directory.mkdir()
        write(directory/'target.json', trial['candidate'])
        write(directory/'yielding.json', trial['yielding_connections'])
        row = dict(yielding=trial['yielding_connections'], directory=str(directory), processes=[], restored=False)
        rows.append(row)
        provisional = directory/'target-only'
        shutil.copytree(source, provisional)
        applied = run(binary, ['apply-kicad-route-candidate', source/board_name, directory/'target.json',
                              provisional/board_name, directory/'yielding.json'], directory/'apply-target.log')
        row['processes'].append(applied)
        assert applied['exit_code'] == 0
        row['processes'].append(run(binary, ['verify-kicad-rung', provisional, args.board_id], directory/'verify-target.log'))
        native = read(provisional/'verification.json')
        row['provisional_native'] = native
        assert progress_admissible(native, read(provisional/'drc.json'), baseline_drc, args.allow_annotations)
        routing = copy.deepcopy(config)
        if routing.get('routing_demand'):
            routing['routing_demand']['alternatives'] = [a for a in routing['routing_demand']['alternatives']
                if a['connection'].removeprefix('/') != target.removeprefix('/')]
        write(directory/'reroute-config.json', routing)
        reroute = run(binary, ['route-kicad-connection', provisional/board_name, trial['yielding_connections'][0],
                     directory/'rerouted.json', directory/'reroute-config.json'], directory/'reroute.log')
        row['processes'].append(reroute)
        if reroute['exit_code'] == 0:
            restored = directory/'restored'
            shutil.copytree(provisional, restored)
            applied = run(binary, ['apply-kicad-route-candidate', provisional/board_name, directory/'rerouted.json',
                                  restored/board_name], directory/'apply-rerouted.log')
            row['processes'].append(applied)
            assert applied['exit_code'] == 0
            row['processes'].append(run(binary, ['verify-kicad-rung', restored, args.board_id], directory/'verify-restored.log'))
            native = read(restored/'verification.json')
            drc = read(restored/'drc.json')
            row['final_native'] = native
            row['restored'] = (progress_admissible(native, drc, baseline_drc, args.allow_annotations)
                and native['selected_net_unconnected_items'] < baseline['selected_net_unconnected_items']
                and not (finding_counts(drc['unconnected_items']) - finding_counts(baseline_drc['unconnected_items'])))
        write(out/'report.json', report)
        print(row['yielding'], 'restored:', row['restored'], flush=True)
    report['finished'] = True
    write(out/'report.json', report)


if __name__ == '__main__':
    parser = argparse.ArgumentParser(description=__doc__)
    for name in ['source', 'diagnosis', 'config', 'binary', 'output']:
        parser.add_argument('--'+name, type=Path, required=True)
    parser.add_argument('--board-id', required=True)
    parser.add_argument('--allow-annotations', action='store_true')
    probe(parser.parse_args())
