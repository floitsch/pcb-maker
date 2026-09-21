#!/usr/bin/env python3
# Copyright (C) 2026 Toit contributors.
"""Audit committed/rollback boards and retain recovery decisions and work."""
import argparse
import hashlib
import json
import os
from pathlib import Path
import re
import shutil
import subprocess
import xml.etree.ElementTree as ET


def read(path):
    return json.loads(path.read_text())


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('directory', type=Path)
    args = parser.parse_args()
    root = args.directory.resolve()
    manifest = read(root / 'manifest.json')
    assert manifest['status'] == 'terminal'
    exe = root / 'pcb-maker'
    assert hashlib.sha256(exe.read_bytes()).hexdigest() == manifest['executable_sha256']
    progress = read(root / 'run/progression.json')
    assert progress['termination'] != 'running'
    def measure(folder):
        return json.loads(subprocess.run([str(exe), 'inspect-kicad-board', str(folder / 'dual-esp32.kicad_pcb')],
                                        capture_output=True, text=True, check=True).stdout)
    initial = measure(Path(progress['initial_directory']))
    rows = []
    total_preflight = 0
    total_guidance = 0
    complete_work = True
    for step in progress['steps']:
        result = step['result']
        assert result is not None, step.get('error')
        evidence = result['evidence']
        assert evidence['parent_unchanged']
        if result['disposition'] != 'committed':
            assert evidence['rollback_exact']
        folder = Path(result['selected_directory'])
        verification = read(folder / 'verification.json')
        assert verification['complete']
        stats = measure(folder)
        assert stats['component_placement_sha256'] == initial['component_placement_sha256']
        ET.parse(folder / 'preview.svg')
        preflight = 0
        guidance = 0
        attempts = []
        for attempt in evidence['attempts']:
            row = {k: attempt[k] for k in ['ordinal', 'scope', 'status', 'route_expansions', 'error', 'selected']}
            repair = attempt.get('single_connection_ripup')
            if repair:
                assert repair['source_unchanged']
                diagnosis = repair['diagnosis']
                preflight += diagnosis['total_reachability_expansions']
                row['yielding_trials'] = [{'nets': t['yielding_connections'], 'route_found': t['route_found'], 'error': t['error']}
                                         for t in diagnosis['trials']]
                row['repair_attempts'] = [{'net': t['yielding_connection'], 'status': t['status'], 'error': t['error'], 'selected': t['selected']}
                                          for t in repair['attempts']]
                for trial in repair['attempts']:
                    if trial['rerouted_candidate']:
                        candidate = read(Path(trial['rerouted_candidate']))
                        preflight += candidate['reachability_expansions']
                        guidance += candidate.get('heuristic_expansions', 0)
                    elif trial['error']:
                        count = re.search(r'reachability preflight used (\d+) expansions', trial['error'])
                        if count:
                            preflight += int(count[1])
                        else:
                            complete_work = False
            elif attempt.get('route_failure'):
                failure = attempt['route_failure']
                row['failure'] = failure
                preflight += failure['reachability_expansions']
                guidance += failure['heuristic_expansions'] or 0
            else:
                candidate_path = Path(attempt['artifact_directory']) / 'inserted-route.json'
                if candidate_path.exists():
                    candidate = read(candidate_path)
                    preflight += candidate['reachability_expansions']
                    guidance += candidate.get('heuristic_expansions', 0)
                else:
                    complete_work = False
            attempts.append(row)
        total_preflight += preflight
        total_guidance += guidance
        rows.append({'source_rung': step['source_rung'], 'target_rung': step['target_rung'],
                     'connection': evidence['inserted_connection'], 'disposition': result['disposition'],
                     'length_mm': stats['physical_copper']['physical_centerline_length_mm'], 'vias': stats['vias'],
                     'conditional': evidence['conditional_local_routing'], 'attempts': attempts,
                     'preflight_expansions': preflight, 'guidance_expansions': guidance,
                     'verification': verification, 'preview': os.path.relpath(folder / 'preview.svg', root)})
    final = Path(progress['final_directory'])
    stats = measure(final)
    shutil.copy2(final / 'preview.svg', root / 'final-preview.svg')
    subprocess.run(['rsvg-convert', '--background-color', 'white', '--width', '1000', '--output', str(root / 'final-preview.png'), str(root / 'final-preview.svg')], check=True)
    summary = {'source_rung': progress['initial_rung'], 'final_rung': progress['final_rung'], 'target_rung': manifest['target_rung'],
               'termination': progress['termination'], 'steps': rows, 'native_placement_unchanged': True,
               'final_statistics': stats, 'astar_expansions': progress['total_route_expansions'],
               'preflight_expansions': total_preflight if complete_work else None,
               'guidance_expansions': total_guidance if complete_work else None, 'work_accounting_complete': complete_work}
    (root / 'summary.json').write_text(json.dumps(summary, indent=2) + '\n')
    page='''<!doctype html><meta charset="utf-8"><title>Composed routing recovery</title><style>body{font:16px system-ui;margin:24px;background:#17222d;color:white}a{color:#9df}img{width:100%;max-width:1000px;background:white}pre{white-space:pre-wrap}select,button{font:inherit}</style><h1>Composed routing recovery</h1><p id="status"></p><p>Frozen placement and physical rules. Ordinary routing, conditional budget guidance, then bounded one-net rip-up with restoration and native admission. Each image shows the accepted or exactly rolled-back board.</p><button id="play">Play / pause</button> <select id="pick"></select><p id="caption"></p><img id="board"><pre id="detail"></pre><p><a href="summary.json">Work and decisions</a> · <a href="manifest.json">Execution record</a></p><script>const d=DATA;document.getElementById('status').textContent='Prefix '+d.source_rung+' → '+d.final_rung+' / target '+d.target_rung+'; '+d.termination;const pick=document.getElementById('pick');d.steps.forEach((s,i)=>pick.add(new Option(s.connection,i)));let playing=false;function show(){const s=d.steps[Number(pick.value)];document.getElementById('caption').textContent=s.source_rung+' → '+s.target_rung+' '+s.connection+' '+s.disposition+'; '+s.length_mm.toFixed(3)+' mm / '+s.vias+' vias';document.getElementById('board').src=s.preview;document.getElementById('detail').textContent=JSON.stringify({conditional:s.conditional,attempts:s.attempts},null,2)}pick.onchange=show;document.getElementById('play').onclick=()=>playing=!playing;setInterval(()=>{if(playing){pick.value=(Number(pick.value)+1)%d.steps.length;show()}},1500);show();</script>'''
    analysis = ''
    if (root / 'pad-access-diagnostic/analysis.json').exists():
        analysis = '<details><summary>Separate post-hoc blocker-priority analysis</summary><p>The isolated route conflicts with three retained nets. A via centered in target pad R3.2 conflicts with SIG03_W_R, the known successful yielding net. This hint was not used in the composed run and has not been tested on a held-out board.</p><a href="pad-access-diagnostic/analysis.json">Local geometry and advisory trial order</a><p><img src="pad-access-diagnostic/conflicts.svg"></p></details>'
    (root / 'index.html').write_text(page.replace('DATA', json.dumps(summary).replace('</', '<\\/')).replace('<script>', analysis + '<script>', 1))
    print(json.dumps({k: v for k, v in summary.items() if k not in ['steps', 'final_statistics']}, indent=2))


if __name__ == '__main__':
    main()
