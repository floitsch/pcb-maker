# Copyright (C) 2026 Toit contributors.
"""A first trace must leave one usable lane in a shared narrow passage.

This fixture has native PCB geometry/connectivity checks with explicit rules,
but no schematic/ERC. The real-board companion supplies the full project gate.
"""
import argparse
import copy
import json
from pathlib import Path
import shutil
import subprocess
from run_routing_demand import forecast_svg, write_json


def native_geometry_check(board):
    board.with_suffix('.kicad_dru').write_text('''(version 1)
(rule "Fixture clearance" (constraint clearance (min 0.4)))
(rule "Fixture width" (constraint track_width (min 0.8)))
(rule "Fixture edge" (constraint edge_clearance (min 0.4)))
''')
    report = board.with_name(board.stem + '-drc.json')
    with board.with_name(board.stem + '-drc.log').open('w') as log:
        subprocess.run(['kicad-cli', 'pcb', 'drc', '--format', 'json', '--exit-code-violations', '--output', str(report), str(board)], stdout=log, stderr=subprocess.STDOUT, check=True)
    native = json.loads(report.read_text())
    assert not native['violations'] and not native['unconnected_items']
    return dict(drc_violations=0, unconnected_items=0, scope='PCB geometry and connectivity; no schematic/ERC.')


def run(args):
    out = args.output.resolve()
    out.mkdir(parents=True, exist_ok=False)
    exe = out / 'pcb-maker'
    shutil.copy2(args.binary.resolve(), exe)
    pcb = ['(kicad_pcb (version 20241229) (generator "pcbnew")',
           '(general (thickness 1.6)) (paper "A4")',
           '(layers (0 "F.Cu" signal) (31 "B.Cu" signal) (44 "Edge.Cuts" user))',
           '(gr_rect (start 0 0) (end 30 30) (stroke (width 0.05) (type default)) (layer "Edge.Cuts"))']
    background = ['<svg xmlns="http://www.w3.org/2000/svg" viewBox="-1 -1 32 32">',
                  '<rect width="30" height="30" fill="white" stroke="black" stroke-width="0.1"/>']
    for reference, net, x, y in [('A1', 'A', 3, 15.5), ('A2', 'A', 27, 15.5), ('B1', 'B', 3, 19), ('B2', 'B', 27, 19)]:
        pcb.append(f'(footprint "test" (layer "F.Cu") (at {x} {y}) (property "Reference" "{reference}") (pad "1" smd circle (at 0 0) (size 1 1) (layers "F.Cu") (net "/{net}")))')
        background.append(f'<circle cx="{x}" cy="{y}" r="0.5" fill="#d59c37"/><text x="{x+0.7}" y="{y-0.6}" font-size="0.8">{reference}</text>')
    for name, y0, y1 in [('top', 0, args.passage_top), ('bottom', 17, 30)]:
        pcb.append(f'(zone (layers "F.Cu" "B.Cu") (name "{name}") (keepout (tracks not_allowed) (vias not_allowed)) (polygon (pts (xy 10 {y0}) (xy 20 {y0}) (xy 20 {y1}) (xy 10 {y1}))))')
        background.append(f'<rect x="10" y="{y0}" width="10" height="{y1-y0}" fill="#cbd3db"/>')
    pcb.append(')')
    background.append('</svg>')
    board = out / 'source.kicad_pcb'
    board.write_text('\n'.join(pcb))
    (out / 'source.svg').write_text(''.join(background))
    config = dict(resolution_mm=0.1, trace_width_mm=0.8, clearance_mm=0.4, edge_clearance_mm=0.4,
                  via_size_mm=1.2, via_drill_mm=0.6, via_cost_mm=20.0, max_expansions=2000000)
    write_json(out / 'base-config.json', config)

    def command(name, *arguments):
        with (out / f'{name}.log').open('w') as log:
            subprocess.run([str(exe), *map(str, arguments)], stdout=log, stderr=subprocess.STDOUT, check=True)

    command('forecast', 'route-kicad-connection', board, 'B', out / 'forecast.json', out / 'base-config.json')
    forecast = json.loads((out / 'forecast.json').read_text())
    forecast_svg(forecast, out / 'forecast.svg', out / 'source.svg')
    results = []
    for name, strength in [('baseline', 0.0), ('demand', 3.0)]:
        active = copy.deepcopy(config)
        active['routing_demand'] = dict(strength=strength, shoulder_mm=0.5, alternatives=[dict(
            connection='B', weight=1.0, trace_width_mm=0.8, clearance_mm=0.4, via_size_mm=1.2,
            paths=[b['path'] for b in forecast['branches']])])
        config_path = out / f'{name}-config.json'
        write_json(config_path, active)
        current = board
        background_path = out / 'source.svg'
        for net in ['A', 'B']:
            candidate_path = out / f'{name}-{net}.json'
            command(f'{name}-{net}', 'route-kicad-connection', current, net, candidate_path, config_path)
            candidate = json.loads(candidate_path.read_text())
            forecast_svg(candidate, candidate_path.with_suffix('.svg'), background_path)
            next_board = out / f'{name}-{net}.kicad_pcb'
            command(f'{name}-{net}-apply', 'apply-kicad-route-candidate', current, candidate_path, next_board)
            current, background_path = next_board, candidate_path.with_suffix('.svg')
        command(f'{name}-statistics', 'inspect-kicad-board', current)
        results.append(dict(policy=name, statistics=json.loads((out / f'{name}-statistics.log').read_text()),
                            native=native_geometry_check(current)))
    assert results[0]['statistics']['vias'] == 2
    if args.passage_top == 14.0:
        assert results[1]['statistics']['vias'] == 0
    elif args.passage_top == 14.5:
        assert results[1]['statistics']['vias'] >= 2
    assert results[0]['statistics']['component_placement_sha256'] == results[1]['statistics']['component_placement_sha256']
    write_json(out / 'report.json', dict(results=results, passage_width_mm=17-args.passage_top,
               two_trace_required_width_mm=2*0.8+3*0.4, scope='Geometric route fixture; no schematic/native whole-project gate.'))
    (out / 'index.html').write_text('''<!doctype html><meta charset="utf-8"><title>Shared passage control</title>
<style>body{font:17px system-ui;margin:25px}.pair{display:grid;grid-template-columns:1fr 1fr;gap:20px}img{width:100%}</style>
<h1>Leave a usable lane</h1><p>Grey regions forbid copper on both layers. A is routed first. B forecasts its own route independently. Red is front copper; blue is back copper.</p>
<div class="pair"><div><h2>Baseline</h2><img src="baseline-B.svg"></div><div><h2>Future demand</h2><img src="demand-B.svg"></div></div>
<p><a href="report.json">Measurements</a> · <a href="forecast.svg">B forecast</a></p>''')
    print(json.dumps(results))


if __name__ == '__main__':
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('output', type=Path)
    parser.add_argument('--binary', type=Path, default=Path('target/release/pcb-maker'))
    parser.add_argument('--passage-top', type=float, default=14.0)
    run(parser.parse_args())
