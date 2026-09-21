# Copyright (C) 2026 Toit contributors.
"""Forecast uncommitted copper, then compare native sequential routing policies.

Every independent forecast gets a simple SVG. Every sequential admission uses
the production native verifier and its automatic labelled copper rendering.
"""
import argparse
import copy
import hashlib
import html
import json
from pathlib import Path
import shutil
import subprocess
import time
import xml.etree.ElementTree as ET


def write_json(path, value):
    path.write_text(json.dumps(value, indent=2) + '\n')


def forecast_svg(candidate, destination, background=None):
    """Board-coordinate vector evidence; deliberately no component bodies."""
    points = [p['at'] for b in candidate['branches'] for p in b['path']]
    if background:
        svg = ET.parse(background).getroot()
        view = svg.attrib['viewBox']
        underlay = ''.join(ET.tostring(e, encoding='unicode') for e in svg)
    else:
        xs, ys = zip(*points)
        view = f'{min(xs)-2} {min(ys)-2} {max(xs)-min(xs)+4} {max(ys)-min(ys)+4}'
        underlay = ''
    elements = [f'<svg xmlns="http://www.w3.org/2000/svg" viewBox="{view}">', underlay]
    for branch in candidate['branches']:
        for a, b in zip(branch['path'], branch['path'][1:]):
            if a['layer'] != b['layer']:
                elements.append(f'<circle cx="{a["at"][0]}" cy="{a["at"][1]}" r="0.45" fill="#8560bb"/>')
            else:
                color = '#d74731' if a['layer'] == 'F.Cu' else '#237bbb'
                elements.append(f'<path d="M {a["at"][0]} {a["at"][1]} L {b["at"][0]} {b["at"][1]}" stroke="{color}" stroke-width="{candidate["config"]["trace_width_mm"]}" fill="none"/>')
    elements.append('</svg>')
    destination.write_text(''.join(elements))
    ET.parse(destination)


def run(args):
    source = args.source.resolve()
    out = args.output.resolve()
    if out.exists() or out.is_relative_to(source):
        raise ValueError('Output must be fresh and outside source')
    out.mkdir(parents=True)
    exe = out / 'pcb-maker'
    shutil.copy2(args.binary.resolve(), exe)
    original_board = source / f'{args.board_id}.kicad_pcb'
    original_hash = hashlib.sha256(original_board.read_bytes()).hexdigest()
    steps = []

    def command(name, *arguments, required=True):
        started = time.monotonic()
        with (out / f'{name}.log').open('w') as log:
            process = subprocess.run([str(exe), *map(str, arguments)], stdout=log, stderr=subprocess.STDOUT)
        steps.append(dict(name=name, exit_code=process.returncode, elapsed_seconds=time.monotonic()-started))
        write_json(out / 'steps.json', steps)
        if required and process.returncode:
            raise RuntimeError(f'{name} failed; see its log')
        return process.returncode

    shutil.copytree(source, out / 'source')
    board = out / 'source' / original_board.name
    command('strip', 'strip-kicad-copper', original_board, board)
    command('verify-source', 'verify-kicad-rung', board.parent, args.board_id, required=False)
    # A cold source has expected opens. Other source findings remain visible
    # in native baseline admission; a failed verification is never hidden.
    if not (board.parent / 'verification.json').exists():
        raise RuntimeError('Missing native source verification')
    sequential = json.loads(args.config.read_text())
    write_json(out / 'input-config.json', sequential)
    command('connections', 'inspect-kicad-connections', board)
    discovered = json.loads((out / 'connections.log').read_text())
    order = sequential['connection_order'] or [row['connection'] for row in discovered]
    alternatives = []
    coverage = []
    forecast_directory = out / 'forecasts'
    forecast_directory.mkdir()
    for index, connection in enumerate(order):
        found = {}
        row = dict(connection=connection, attempts=[])
        # Two budgets explore route preferences; identical resulting copper
        # is deduplicated before assigning alternative probabilities.
        for variant, via_mm in enumerate([None, 20.0]):
            config = copy.deepcopy(sequential['routing_portfolio'][0])
            config.pop('routing_demand', None)
            config['via_cost_mm'] = via_mm
            if config.get('multi_terminal_routing') == 'shared_copper_tree':
                config['tree_attachment_objective'] = 'router_cost'
            config_path = forecast_directory / f'{index:02}-{variant}-config.json'
            candidate_path = forecast_directory / f'{index:02}-{variant}.json'
            write_json(config_path, config)
            code = command(f'forecast-{index:02}-{variant}', 'route-kicad-connection', board, connection, candidate_path, config_path, required=False)
            row['attempts'].append(dict(variant=variant, exit_code=code))
            if code:
                svg = ET.parse(board.parent / 'preview.svg')
                title = ET.SubElement(svg.getroot(), '{http://www.w3.org/2000/svg}title')
                title.text = f'Forecast failed for {connection}; this shows the unrouted source board.'
                svg.write(candidate_path.with_suffix('.svg'), encoding='unicode')
                continue
            candidate = json.loads(candidate_path.read_text())
            forecast_svg(candidate, candidate_path.with_suffix('.svg'), board.parent / 'preview.svg')
            paths = [b['path'] for b in candidate['branches']]
            fingerprint = json.dumps(paths, sort_keys=True)
            resolved = candidate['config']
            found[fingerprint] = dict(connection=connection, paths=paths, weight=0.0,
                                      trace_width_mm=resolved['trace_width_mm'], clearance_mm=resolved['clearance_mm'], via_size_mm=resolved['via_size_mm'])
        for alternative in found.values():
            alternative['weight'] = 1.0 / len(found)
            alternatives.append(alternative)
        row['distinct_alternatives'] = len(found)
        coverage.append(row)
        write_json(out / 'forecast-coverage.json', coverage)
    write_json(out / 'forecast-alternatives.json', alternatives)
    policies = [('original', None, False), ('cost-control', None, True)]
    policies.extend((f'demand-{strength:g}', strength, True) for strength in args.strengths)
    results = []
    for name, strength, cost_objective in policies:
        config = copy.deepcopy(sequential)
        for routing in config['routing_portfolio']:
            routing.pop('routing_demand', None)
            if cost_objective and routing.get('multi_terminal_routing') == 'shared_copper_tree':
                routing['tree_attachment_objective'] = 'router_cost'
            if strength is not None:
                routing['routing_demand'] = dict(strength=strength, shoulder_mm=args.shoulder_mm, alternatives=alternatives)
        config_path = out / f'{name}-config.json'
        write_json(config_path, config)
        code = command(name, 'route-kicad-board-sequential', board.parent, args.board_id, out / name, config_path, required=False)
        result_path = out / name / 'sequential-route.json'
        row = dict(policy=name, exit_code=code)
        if result_path.exists():
            result = json.loads(result_path.read_text())
            row.update(termination=result['termination'], completed_connections=result['completed_connections'],
                       statistics=result['final_statistics'], total_expansions=result['total_expansions'])
            native = out / name / 'result' / 'verification.json'
            row['native'] = json.loads(native.read_text()) if native.exists() else None
            from render_routing_demand import render
            for step in result['steps']:
                selected = next((a for a in step['attempts'] if a['selected']), None)
                if selected:
                    render(Path(selected['candidate']), board.parent / 'preview.svg', Path(step['artifact_directory']) / 'forecast-cost')
        results.append(row)
        write_json(out / 'results.json', results)
        report_page(out, results, coverage)
    assert hashlib.sha256(original_board.read_bytes()).hexdigest() == original_hash
    write_json(out / 'report.json', dict(source=str(source), source_sha256=original_hash, source_unchanged=True,
               executable_sha256=hashlib.sha256(exe.read_bytes()).hexdigest(), coverage=coverage, results=results, steps=steps,
               limitations=['Forecasts are independently routed copper, not a capacity certificate.',
                            'Alternative diversity is limited to two via preferences.',
                            'Cost-control separates tree attachment scoring from the demand field.',
                            'No forecast criticality learning or passage-capacity check yet.']))


def report_page(out, results, coverage):
    page = ['<!doctype html><meta charset="utf-8"><title>Future routing demand</title>',
            '<style>body{font:16px system-ui;margin:24px}.boards{display:grid;grid-template-columns:repeat(2,1fr);gap:20px}img{width:100%}td,th{padding:8px;border:1px solid #aaa}table{border-collapse:collapse}</style>',
            '<h1>Future routing demand</h1><p>Identical placement, rules and routing order. Forecasts use the empty board, without the human routes. Component bodies are hidden.</p>',
            '<table><tr><th>Policy</th><th>Routed nets</th><th>Termination</th><th>Vias</th><th>Track mm</th></tr>']
    for row in results:
        stats = row.get('statistics', {})
        page.append(f'<tr><td>{html.escape(row["policy"])}</td><td>{row.get("completed_connections", "?")}</td><td>{row.get("termination", "error")}</td><td>{stats.get("vias", "?")}</td><td>{stats.get("stored_segment_length_mm", 0):.3f}</td></tr>')
    page.append('</table><div class="boards">')
    for row in results:
        name = row['policy']
        page.append(f'<div><h2>{html.escape(name)}</h2><img src="{name}/result/preview.svg"></div>')
    page.append('</div><h2>Independent forecasts</h2><div class="boards">')
    for index, row in enumerate(coverage):
        page.append(f'<div><h3>{html.escape(row["connection"])}: {row["distinct_alternatives"]} distinct alternatives</h3><img src="forecasts/{index:02}-0.svg"></div>')
    page.append('</div><p><a href="report.json">Evidence and limitations</a></p>')
    (out / 'index.html').write_text(''.join(page))


if __name__ == '__main__':
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('source', type=Path)
    parser.add_argument('board_id')
    parser.add_argument('config', type=Path)
    parser.add_argument('output', type=Path)
    parser.add_argument('--binary', type=Path, default=Path('target/release/pcb-maker'))
    parser.add_argument('--strengths', type=float, nargs='+', default=[1.0, 3.0])
    parser.add_argument('--shoulder-mm', type=float, default=1.0)
    run(parser.parse_args())
