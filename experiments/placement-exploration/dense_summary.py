#!/usr/bin/env python3
# Copyright (C) 2026 Toit contributors.
"""Audit dense_probe outputs and render matched-prefix routing playback.

With --allow-running this writes an explicitly partial snapshot. It never
infers subprocess termination from a progression file or an absent process.
"""
import argparse
import hashlib
import importlib.util
import json
import math
import os
from pathlib import Path
import re
import shutil
import subprocess
import xml.etree.ElementTree as ET

ROOT = Path(__file__).resolve().parents[2]


def read(path):
    return json.loads(path.read_text())


def sha(path):
    return hashlib.sha256(path.read_bytes()).hexdigest()


def write(path, data):
    path.write_text(json.dumps(data, indent=2) + '\n')


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('directory', type=Path)
    parser.add_argument('--allow-running', action='store_true')
    args = parser.parse_args()
    root = args.directory.resolve()
    manifest = read(root / 'manifest.json')
    terminal = all(c['status'] == 'terminal' for c in manifest['cases'])
    assert terminal or args.allow_running, 'Wait for authoritative terminal statuses'
    executable = root / 'pcb-maker'
    assert sha(executable) == manifest['executable_sha256']
    spec = importlib.util.spec_from_file_location('pad_gap_summary', ROOT / 'benchmarks/esp32-pad-gaps/summarize.py')
    gaps = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(gaps)
    rows = []

    for case in manifest['cases']:
        directory = root / case['id']
        for filename, key in [('problem.json', 'problem_sha256'), ('routing.json', 'routing_sha256'), ('template.json', 'template_sha256')]:
            assert sha(directory / filename) == case[key], (case['id'], filename)
        assert read(directory / 'placement.json')['policy']['kind'] == 'declared'
        progress_path = directory / 'run/solve/progression.json'
        row = {k: case[k] for k in ['id', 'placement', 'profile', 'status']}
        row['frames'] = []
        rows.append(row)
        if not progress_path.exists():
            assert case['status'] != 'terminal', (case['id'], 'terminal before progression; inspect run.log')
            continue
        progress = read(progress_path)
        assert progress['initial_rung'] == 0
        initial = Path(progress['initial_directory'])
        cache = directory / 'measurements'
        cache.mkdir(exist_ok=True)

        def measure(folder, rung):
            board = folder / 'dual-esp32.kicad_pcb'
            digest = sha(board)
            record_path = cache / f'rung-{rung:02}.json'
            record = read(record_path) if record_path.exists() else None
            if record is None or record['board_sha256'] != digest:
                result = subprocess.run([str(executable), 'inspect-kicad-board', str(board)], capture_output=True, text=True, check=True)
                stats = json.loads(result.stdout)
                record = {'board_sha256': digest, 'statistics': stats, 'pad_gaps': gaps.crossings(board)}
                write(record_path, record)
            stats = record['statistics']
            assert stats['physical_copper']['centerline_union_exact']
            assert record['pad_gaps']['unsupported_front_arcs'] == 0
            if case['profile'] == 'blocked':
                assert record['pad_gaps']['crossed_pad_gaps'] == 0
            return record

        initial_measure = measure(initial, 0)
        pose_hash = initial_measure['statistics']['component_placement_sha256']
        assert initial_measure['statistics']['segments'] == 0
        assert initial_measure['statistics']['vias'] == 0
        cumulative_expansions = 0
        cumulative_preflight = 0
        failures = []
        for step in progress['steps']:
            result = step['result']
            evidence = result['evidence']
            assert evidence['parent_unchanged']
            assert evidence['external_parent_sha256_before'] == evidence['external_parent_sha256_after']
            cumulative_expansions += sum(a.get('route_expansions') or 0 for a in evidence['attempts'])
            for attempt in evidence['attempts']:
                candidate_path = Path(attempt['artifact_directory']) / 'inserted-route.json'
                if candidate_path.exists():
                    candidate = read(candidate_path)
                    assert candidate.get('heuristic_expansions', 0) == 0
                    cumulative_preflight += candidate.get('reachability_expansions', 0)
                elif attempt.get('error'):
                    count = re.search(r'reachability preflight used (\d+) expansions', attempt['error'])
                    assert count, ('Unaccounted failed-route preflight work', attempt['error'])
                    cumulative_preflight += int(count[1])
            if result['disposition'] != 'committed':
                assert evidence['rollback_exact']
                failures.append({'connection': evidence['inserted_connection'], 'target_rung': step['target_rung'],
                                 'attempts': evidence['attempts'], 'rollback_exact': True,
                                 'directory': step['transaction_directory']})
                continue
            folder = Path(result['selected_directory'])
            verification = read(folder / 'verification.json')
            assert verification['complete']
            for key in ['erc_violations', 'drc_design_violations', 'schematic_parity_issues', 'selected_net_unconnected_items']:
                assert verification[key] == 0, (case['id'], step['target_rung'], key)
            preview = folder / 'preview.svg'
            ET.parse(preview)
            assert (folder / 'preview.json').exists()
            measured = measure(folder, step['target_rung'])
            stats = measured['statistics']
            assert stats['component_placement_sha256'] == pose_hash
            frame = {'rung': step['target_rung'], 'connection': evidence['inserted_connection'],
                     'length_mm': stats['physical_copper']['physical_centerline_length_mm'], 'vias': stats['vias'],
                     'astar_expansions': cumulative_expansions,
                     'preflight_expansions': cumulative_preflight,
                     'search_state_expansions': cumulative_expansions + cumulative_preflight,
                     'pad_gap_crossings': measured['pad_gaps']['crossed_pad_gaps'],
                     'preview': os.path.relpath(preview, root), 'verification': verification}
            row['frames'].append(frame)
        assert len(row['frames']) == progress['final_rung']
        assert cumulative_expansions == progress['total_route_expansions']
        row.update(final_rung=progress['final_rung'], termination=progress['termination'],
                   astar_expansions=progress['total_route_expansions'], failures=failures,
                   preflight_expansions=cumulative_preflight,
                   search_state_expansions=cumulative_expansions + cumulative_preflight,
                   native_placement_sha256=pose_hash, native_placement_unchanged=True)
        if row['frames']:
            last = row['frames'][-1]
            row.update({k: last[k] for k in ['length_mm', 'vias', 'pad_gap_crossings']})
            shutil.copy2(root / last['preview'], directory / 'final-preview.svg')
            subprocess.run(['rsvg-convert', '--background-color', 'white', '--width', '1000',
                            '--output', str(directory / 'final-preview.png'), str(directory / 'final-preview.svg')], check=True)
        if case['status'] == 'terminal':
            cold = read(directory / 'run/cold-prefix.json')
            assert progress['termination'] != 'running'
            assert cold['final_rung'] == progress['final_rung']
            assert cold['completed'] == (progress['final_rung'] == manifest['target_connections'])
            source = read(directory / 'problem.json')
            source_poses = {c['id']: c for c in source['components']}
            assert len(cold['placement']['poses']) == len(source_poses)
            for pose in cold['placement']['poses']:
                original = source_poses[pose['component']]
                assert math.dist([original['position'][a] for a in ['x', 'y']], [pose['position'][a] for a in ['x', 'y']]) < 1e-8
                assert abs((original.get('rotation_degrees', 0) - pose['rotation_degrees'] + 180) % 360 - 180) < 1e-8
            selected = Path(progress['final_directory'])
            final_verification = read(selected / 'verification.json')
            assert final_verification['complete']
            row.update(exit_code=case['exit_code'], completed=cold['completed'],
                       wall_seconds=case['wall_seconds'], source_placement_preserved=True,
                       final_verification=final_verification)

    for name in sorted({r['placement'] for r in rows}):
        pair = [r for r in rows if r['placement'] == name and 'native_placement_sha256' in r]
        if len(pair) == 2:
            assert pair[0]['native_placement_sha256'] == pair[1]['native_placement_sha256']
    summary = {'terminal': terminal, 'target_connections': manifest['target_connections'],
               'executable_sha256': manifest['executable_sha256'], 'cases': rows,
               'scope': 'Fixed placements computed from all 52 connections, then routing the same first 19 from zero copper. Compare quality at matching completed prefixes. Search state expansions count A* plus separately recorded reachability preflight; they exclude mask construction, placement and native verification. Concurrent wall times are advisory. Native metadata warnings are retained separately from design findings.'}
    write(root / 'summary.json', summary)
    template = (Path(__file__).with_name('dense_viewer.html')).read_text()
    diagnostic_link = ' · <a href="diagnostics.html">Separate failure diagnostics</a>' if (root / 'diagnostics.html').exists() else ''
    (root / 'index.html').write_text(template.replace('DATA_PLACEHOLDER', json.dumps(summary).replace('</', '<\\/')).replace('DIAGNOSTICS_LINK', diagnostic_link))
    print(json.dumps({**{k: v for k, v in summary.items() if k != 'cases'},
                      'cases': [{k: v for k, v in r.items() if k not in ['frames', 'failures', 'final_verification']} for r in rows]}, indent=2))


if __name__ == '__main__':
    main()
