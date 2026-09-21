# Copyright (C) 2026 Toit contributors.
"""Measure directed pairwise routing interference and test vulnerable nets first.

Consumes a run_routing_demand.py experiment, reusing its independent forecasts
and frozen executable. Pairwise probes are provisional; full order trials use
the production native gate. No human route or selected trouble spot is used.
"""
import argparse
import copy
import hashlib
import html
import json
import math
from pathlib import Path
import shutil
import subprocess
import time
from run_routing_demand import forecast_svg, report_page, write_json


def quality(candidate):
    # The native router deduplicates supplemental physical copper. Shared
    # root-to-terminal paths must not be summed as independent conductors.
    segments = candidate['supplemental_segments']
    return dict(length_mm=sum(math.dist(s['start'], s['end']) for s in segments),
                vias=len(candidate['supplemental_vias']))


def observed_cost_ranking(previous, order, baseline, penalty):
    """Difficulty seen in a real pass, including interactions among many nets."""
    routed = json.loads((previous / 'cost-control/sequential-route.json').read_text())
    steps = {step['connection']: step for step in routed['steps']}
    rows = []
    for index, net in enumerate(order):
        step = steps.get(net)
        selected = next((a for a in step['attempts'] if a['selected']), None) if step else None
        if selected:
            measured = selected['quality']
            delta = measured['length_mm'] - baseline[index]['length_mm'] + penalty * (measured['vias'] - baseline[index]['vias'])
            if abs(delta) < 1e-6:
                delta = 0.0
            rows.append(dict(connection=net, original_index=index, status='routed', added_cost_mm=delta))
        else:
            rows.append(dict(connection=net, original_index=index, status='failed' if step else 'not_attempted', added_cost_mm=None))
    rows.sort(key=lambda r: (0 if r['status'] == 'failed' else 1 if r['status'] == 'not_attempted' else 2,
                             -(r['added_cost_mm'] or 0.0), r['original_index']))
    return rows


def run(args):
    previous = args.experiment.resolve()
    out = args.output.resolve()
    if out.exists() or out.is_relative_to(previous):
        raise ValueError('Output must be fresh and outside the input experiment')
    out.mkdir(parents=True)
    exe = out / 'pcb-maker'
    shutil.copy2(previous / 'pcb-maker', exe)
    shutil.copytree(previous / 'source', out / 'source')
    # Source projects can contain unrelated alternate board files. The
    # primary name is recorded by the native source verification.
    board_id = json.loads((out / 'source/verification.json').read_text())['board_id']
    board = out / 'source' / f'{board_id}.kicad_pcb'
    board_hash = hashlib.sha256(board.read_bytes()).hexdigest()
    sequential = json.loads((previous / 'cost-control-config.json').read_text())
    original_order = sequential['connection_order']
    if not original_order:
        original_order = [r['connection'] for r in json.loads((previous / 'forecast-coverage.json').read_text())]
    forecasts = [json.loads((previous / 'forecasts' / f'{index:02}-0.json').read_text()) for index in range(len(original_order))]
    baseline = [quality(c) for c in forecasts]
    route_config = copy.deepcopy(sequential['routing_portfolio'][0])
    route_config.pop('routing_demand', None)
    write_json(out / 'probe-config.json', route_config)
    steps = []

    def command(name, *arguments, required=True):
        started = time.monotonic()
        with (out / f'{name}.log').open('w') as log:
            process = subprocess.run([str(exe), *map(str, arguments)], stdout=log, stderr=subprocess.STDOUT)
        steps.append(dict(name=name, exit_code=process.returncode, elapsed_seconds=time.monotonic()-started))
        write_json(out / 'steps.json', steps)
        if required and process.returncode:
            raise RuntimeError(f'{name} failed; see log')
        return process.returncode

    rows = []
    penalty = args.via_penalty_mm
    for earlier, connection in enumerate(original_order):
        directory = out / f'probe-{earlier:02}'
        directory.mkdir()
        first = directory / 'first.json'
        write_json(first, forecasts[earlier])
        committed = directory / board.name
        command(f'commit-{earlier:02}', 'apply-kicad-route-candidate', board, first, committed)
        forecast_svg(forecasts[earlier], directory / 'first.svg', out / 'source/preview.svg')
        for later, target in enumerate(original_order):
            if earlier == later:
                continue
            candidate_path = directory / f'target-{later:02}.json'
            code = command(f'probe-{earlier:02}-{later:02}', 'route-kicad-connection', committed, target, candidate_path, out / 'probe-config.json', required=False)
            row = dict(earlier=connection, target=target, earlier_index=earlier, target_index=later, exit_code=code,
                       baseline=baseline[later], rendering=f'probe-{earlier:02}/target-{later:02}.svg')
            if code == 0:
                candidate = json.loads(candidate_path.read_text())
                forecast_svg(candidate, candidate_path.with_suffix('.svg'), directory / 'first.svg')
                measured = quality(candidate)
                delta = measured['length_mm'] - baseline[later]['length_mm'] + penalty*(measured['vias']-baseline[later]['vias'])
                row.update(measured=measured, additional_cost_mm=delta, positive_harm_mm=max(0.0, delta))
            else:
                # Keep a rendering for failed probes too, clearly identified
                # as the earlier route rather than a successful target route.
                shutil.copy2(directory / 'first.svg', candidate_path.with_suffix('.svg'))
                row['failure'] = (out / f'probe-{earlier:02}-{later:02}.log').read_text()
            rows.append(row)
            write_json(out / 'probes.json', rows)
    ranking = []
    for index, connection in enumerate(original_order):
        affected = [row for row in rows if row['target_index'] == index]
        successful = [row for row in affected if row['exit_code'] == 0]
        ranking.append(dict(connection=connection, original_index=index,
                            failed_probes=sum(row['exit_code'] != 0 for row in affected),
                            total_positive_harm_mm=sum(row['positive_harm_mm'] for row in successful),
                            maximum_positive_harm_mm=max((row['positive_harm_mm'] for row in successful), default=0.0),
                            baseline=baseline[index]))
    ranking.sort(key=lambda r: (-r['failed_probes'], -r['total_positive_harm_mm'], -r['maximum_positive_harm_mm'], r['original_index']))
    write_json(out / 'ranking.json', dict(ranking=ranking, ranking_rule='failed pairwise probes, summed positive added length + via penalty, worst added cost, original index', via_penalty_mm=penalty))
    hard_order = [r['connection'] for r in ranking]
    observed = observed_cost_ranking(previous, original_order, baseline, penalty)
    write_json(out / 'observed-cost-ranking.json', observed)
    order_policies = [('vulnerable-first', hard_order), ('vulnerable-last', list(reversed(hard_order))),
                      ('observed-cost-first', [r['connection'] for r in observed])]
    results = []
    for order_name, order in order_policies:
        for demand in [False, True]:
            name = order_name + ('-demand' if demand else '-control')
            template = previous / (f'demand-{args.demand_strength:g}-config.json' if demand else 'cost-control-config.json')
            config = json.loads(template.read_text())
            config['connection_order'] = order
            write_json(out / f'{name}-config.json', config)
            code = command(name, 'route-kicad-board-sequential', board.parent, board_id, out / name, out / f'{name}-config.json', required=False)
            row = dict(policy=name, connection_order=order, exit_code=code)
            result_file = out / name / 'sequential-route.json'
            if result_file.exists():
                result = json.loads(result_file.read_text())
                row.update(termination=result['termination'], completed_connections=result['completed_connections'], statistics=result['final_statistics'], total_expansions=result['total_expansions'])
                native = out / name / 'result/verification.json'
                row['native'] = json.loads(native.read_text()) if native.exists() else None
            results.append(row)
            write_json(out / 'results.json', results)
            report_page(out, results, [])
    assert hashlib.sha256(board.read_bytes()).hexdigest() == board_hash
    write_json(out / 'report.json', dict(source=str(previous), source_board_sha256=board_hash, source_unchanged=True,
               executable_sha256=hashlib.sha256(exe.read_bytes()).hexdigest(), ranking=ranking, observed_cost_ranking=observed, probes=rows, results=results, steps=steps,
               limitations=['Pairwise interference does not prove routability with three or more competing nets.',
                            'Search failures include bounded-search failure; they are not impossibility proofs.',
                            'Independent forecasts provide one chosen embedding per earlier net.',
                            'Order and demand settings are experimental, not selected production defaults.']))
    matrix_page(out, original_order, rows, ranking, penalty)


def matrix_page(out, order, probes, ranking, penalty=5.0):
    page = ['<!doctype html><meta charset="utf-8"><title>Routing interference</title>',
            '<style>body{font:15px system-ui;margin:24px}td,th{padding:8px;border:1px solid #aaa}table{border-collapse:collapse}a{color:inherit}</style>',
            f'<h1>Who makes whom harder to route?</h1><p>Rows: net committed first. Columns: net routed afterwards. Cells show added track-equivalent cost ({penalty:g} mm per via); negative values are improvements. Click for copper-only probe renderings. Failed probes display only the first route.</p><table><tr><th>Earlier ↓ / Target →</th>']
    page.extend(f'<th>{html.escape(net)}</th>' for net in order)
    page.append('</tr>')
    for earlier, net in enumerate(order):
        page.append(f'<tr><th>{html.escape(net)}</th>')
        for later in range(len(order)):
            row = next((r for r in probes if r['earlier_index'] == earlier and r['target_index'] == later), None)
            if row is None:
                page.append('<td>—</td>')
                continue
            value = row.get('additional_cost_mm')
            label = 'failed' if value is None else f'{value:+.2f}'
            opacity = 0.8 if value is None else min(0.8, max(0.0, value)/20)
            page.append(f'<td style="background:rgba(230,100,50,{opacity})"><a href="{row["rendering"]}">{label}</a></td>')
        page.append('</tr>')
    page.append('</table><h2>Most vulnerable first</h2><ol>')
    page.extend(f'<li>{html.escape(r["connection"])}: {r["failed_probes"]} failed probes; {r["total_positive_harm_mm"]:.2f} mm summed positive harm</li>' for r in ranking)
    page.append('</ol><p><a href="index.html">Whole-board order trials</a> · <a href="report.json">Full evidence</a></p>')
    (out / 'interference.html').write_text(''.join(page))


if __name__ == '__main__':
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('experiment', type=Path)
    parser.add_argument('output', type=Path)
    parser.add_argument('--via-penalty-mm', type=float, default=5.0)
    parser.add_argument('--demand-strength', type=float, default=1.0)
    run(parser.parse_args())
