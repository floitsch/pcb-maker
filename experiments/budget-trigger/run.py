#!/usr/bin/env python3
# Copyright (C) 2026 Toit contributors.
"""Replay retained budget and disconnection failures through conditional routing."""
import argparse
import concurrent.futures
import copy
import hashlib
import html
import json
import os
from pathlib import Path
import shutil
import subprocess
import time
import xml.etree.ElementTree as ET

ROOT = Path(__file__).resolve().parents[2]


def read(path):
    return json.loads(path.read_text())


def write(path, value):
    path.write_text(json.dumps(value, indent=2) + '\n')


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('output', type=Path)
    args = parser.parse_args()
    out = args.output.resolve()
    out.mkdir(parents=True, exist_ok=False)
    exe = out / 'pcb-maker'
    shutil.copy2(ROOT / 'target/release/pcb-maker', exe)
    digest = hashlib.sha256(exe.read_bytes()).hexdigest()
    source = ROOT / 'build/placement-dense-prefix19-2026-09-07'
    manifest = {'executable_sha256': digest, 'cases': [], 'scope': __doc__}
    for name, count in [('harmonic-blocked', 3), ('junction-blocked', 1)]:
        directory = out / name
        directory.mkdir()
        original = source / name
        progress = read(original / 'run/solve/progression.json')
        cold = read(original / 'run/cold-prefix.json')
        insertion = read(original / 'routing.json')
        guided = copy.deepcopy(insertion['local_routing_portfolio'][0])
        guided['heuristic'] = 'obstacle_distances'
        insertion['conditional_local_candidate_cap'] = 1
        insertion['conditional_local_routing_portfolio'] = [{
            'trigger': {'kind': 'search_budget_exhausted_without_candidate'}, 'routing': guided}]
        config = {'maximum_connections': count, 'reuse_committed_parent_verification': True, 'insertion': insertion}
        write(directory / 'config.json', config)
        command = [str(exe), 'progress-kicad-connections', cold['template']['declaration'], progress['final_directory'],
                   str(directory / 'run'), str(directory / 'config.json')]
        manifest['cases'].append({'name': name, 'directory': str(directory), 'source_rung': progress['final_rung'],
                                  'command': command, 'status': 'prepared'})
    write(out / 'manifest.json', manifest)
    def run(case):
        start = time.monotonic()
        with (Path(case['directory']) / 'run.log').open('w') as log:
            result = subprocess.run(case['command'], stdout=log, stderr=subprocess.STDOUT)
        return case, result.returncode, time.monotonic() - start
    with concurrent.futures.ThreadPoolExecutor(max_workers=2) as pool:
        futures = [pool.submit(run, case) for case in manifest['cases']]
        for future in concurrent.futures.as_completed(futures):
            case, code, elapsed = future.result()
            case.update(status='terminal', exit_code=code, wall_seconds=elapsed)
            write(out / 'manifest.json', manifest)
            print(json.dumps({'case': case['name'], 'exit_code': code, 'wall_seconds': elapsed}), flush=True)
    assert hashlib.sha256(exe.read_bytes()).hexdigest() == digest
    rows = []
    for case in manifest['cases']:
        directory = Path(case['directory'])
        progress = read(directory / 'run/progression.json')
        final = Path(progress['final_directory'])
        verification = read(final / 'verification.json')
        assert verification['complete']
        stats = json.loads(subprocess.run([str(exe), 'inspect-kicad-board', str(final / 'dual-esp32.kicad_pcb')], capture_output=True, text=True, check=True).stdout)
        initial = Path(progress['initial_directory'])
        initial_stats = json.loads(subprocess.run([str(exe), 'inspect-kicad-board', str(initial / 'dual-esp32.kicad_pcb')], capture_output=True, text=True, check=True).stdout)
        assert stats['component_placement_sha256'] == initial_stats['component_placement_sha256']
        steps = []
        for step in progress['steps']:
            result = step['result']
            evidence = result['evidence']
            assert evidence['parent_unchanged']
            selected = Path(result['selected_directory'])
            ET.parse(selected / 'preview.svg')
            assert read(selected / 'verification.json')['complete']
            decision, = evidence['conditional_local_routing']
            row = {'connection': evidence['inserted_connection'], 'disposition': result['disposition'],
                   'conditional': decision, 'attempts': [], 'preview': os.path.relpath(selected / 'preview.svg', out)}
            for attempt in evidence['attempts']:
                work = attempt.get('route_failure')
                if work is None:
                    candidate = read(Path(attempt['artifact_directory']) / 'inserted-route.json')
                    work = {'astar_expansions': candidate['expansions'], 'reachability_expansions': candidate['reachability_expansions'],
                            'heuristic_expansions': candidate.get('heuristic_expansions', 0)}
                row['attempts'].append({'status': attempt['status'], **work})
            steps.append(row)
        if case['name'] == 'harmonic-blocked':
            assert case['exit_code'] == 0 and progress['final_rung'] == 19
            assert [s['conditional']['activated'] for s in steps] == [True, False, False]
            assert steps[0]['conditional']['observed_budget_exhausted_attempts'] == [0]
            assert steps[0]['attempts'][0]['kind'] == 'search_budget_exhausted'
            assert steps[0]['attempts'][0]['astar_expansions'] == 2000001
            candidate = read(Path(progress['steps'][0]['result']['selected_candidate']))
            prior = read(source / 'guided-followups/harmonic-blocked-replay/run/progression.json')
            old_candidate = read(Path(prior['steps'][0]['result']['selected_candidate']))
            for key in ['cost', 'supplemental_segments', 'supplemental_vias']:
                assert candidate[key] == old_candidate[key]
        else:
            assert case['exit_code'] == 1 and progress['final_rung'] == 7
            assert not steps[0]['conditional']['activated']
            assert steps[0]['attempts'][0]['kind'] == 'grid_disconnected'
            assert len(steps[0]['attempts']) == 1
            assert progress['steps'][0]['result']['evidence']['rollback_exact']
        shutil.copy2(final / 'preview.svg', directory / 'final-preview.svg')
        subprocess.run(['rsvg-convert', '--background-color', 'white', '--width', '1000', '--output', str(directory / 'final-preview.png'), str(directory / 'final-preview.svg')], check=True)
        write(directory / 'statistics.json', stats)
        rows.append({'case': case['name'], 'final_rung': progress['final_rung'], 'steps': steps,
                     'length_mm': stats['physical_copper']['physical_centerline_length_mm'], 'vias': stats['vias'],
                     'verification': verification, 'native_placement_unchanged': True})
    write(out / 'summary.json', {'executable_sha256': digest, 'cases': rows})
    cards = []
    for row in rows:
        cards.append('<h2>' + html.escape(row['case']) + '</h2><p>Accepted prefix: ' + str(row['final_rung']) + '</p><img src="' + row['case'] + '/final-preview.svg"><pre>' + html.escape(json.dumps(row, indent=2)) + '</pre>')
    (out / 'index.html').write_text('<!doctype html><meta charset="utf-8"><title>Automatic budget recovery</title><style>body{font:16px system-ui;background:#17222d;color:white;margin:24px}pre{white-space:pre-wrap}img{width:100%;max-width:1000px;background:white}a{color:#9df}</style><h1>Automatic budget recovery</h1><p>Budget exhaustion activates guidance only when no route candidate exists. Grid disconnection and successful ordinary routing skip it. Every accepted or rolled-back board passes native verification.</p><a href="summary.json">Machine-readable work and decisions</a>' + ''.join(cards))
    print(out / 'index.html', flush=True)


if __name__ == '__main__':
    main()
