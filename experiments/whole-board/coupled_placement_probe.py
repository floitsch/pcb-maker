#!/usr/bin/env python3
# Copyright (C) 2026 Toit contributors.
"""Test coupled contact corrections using the production geometry as an oracle.

Restricted to all-free, fixed zero-orientation native placement problems. This
is an experimental proposal generator; every candidate is rendered and checked
by the ordinary placer. No native routing or placement credit is implied.
"""
import argparse
import copy
import json
from pathlib import Path
import shutil
import subprocess
import sys
import time

import numpy as np


def project(rows, rhs, maximum_iterations=4096):
    """Minimum-norm displacement satisfying A*x >= b, via an active dual set."""
    a, b = np.asarray(rows), np.asarray(rhs)
    gram = a @ a.T
    multipliers = np.zeros(len(b))
    active = []
    solves = 0
    for iteration in range(maximum_iterations):
        violation = b - gram @ multipliers
        if np.max(violation) <= 1e-8:
            result = a.T @ multipliers
            assert np.max(b-a@result) <= 1e-8
            return result, {'iterations': iteration, 'linear_solves': solves,
                            'active_rows': len(active), 'maximum_residual_mm': float(np.max(b-a@result))}
        available = [(float(violation[i]), i) for i in range(len(b)) if i not in active]
        if not available or max(available)[0] <= 1e-8:
            raise ValueError('Active constraints remain inconsistent')
        active.append(max(available)[1])
        for _ in range(maximum_iterations):
            block = gram[np.ix_(active, active)]
            z, _, _, _ = np.linalg.lstsq(block, b[active], rcond=1e-12)
            solves += 1
            if np.max(np.abs(block@z-b[active])) > 1e-7:
                raise ValueError('Inconsistent contact branch or singular active system')
            if np.all(z > 0):
                multipliers[:] = 0
                multipliers[active] = z
                break
            steps = [(multipliers[i]/(multipliers[i]-z[j]), j) for j, i in enumerate(active)
                     if z[j] <= 0 and multipliers[i]-z[j] > 0]
            if not steps:
                # A newly added redundant zero constraint contributes no force.
                active.pop(next(j for j, value in enumerate(z) if value <= 0))
                continue
            alpha, leaving = min(steps)
            multipliers[active] += alpha*(z-multipliers[active])
            multipliers[active[leaving]] = 0
            active.pop(leaving)
        else:
            raise ValueError('Dual inner iteration budget exhausted')
    raise ValueError('Dual iteration budget exhausted')


def bounds(problem, component):
    geometry = component['placement_geometry']
    board = problem['board']['bounds']
    low = np.array([board['min'][k] for k in ['x', 'y']])
    high = np.array([board['max'][k] for k in ['x', 'y']])
    extent = np.array([component['size'][k]/2 for k in ['x', 'y']])
    lower = low - geometry.get('board_overhang', 0) + extent
    upper = high + geometry.get('board_overhang', 0) - extent
    assert 'region' not in component['constraints']
    for pin in component['pins']:
        for pad in pin.get('pads', []):
            shape = pad['shape']
            assert shape['kind'] == 'rect' and shape.get('rotation_degrees', 0) == 0
            offset = np.array([pad.get('local_center', pin['offset'])[k] for k in ['x', 'y']])
            half = np.array([shape['size'][k]/2 for k in ['x', 'y']])
            edge = geometry.get('pad_edge_clearance', 0)
            lower = np.maximum(lower, low + edge + half - offset)
            upper = np.minimum(upper, high - edge - half - offset)
    assert np.all(lower <= upper)
    return lower, upper


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('problem', type=Path)
    parser.add_argument('trace', type=Path)
    parser.add_argument('output', type=Path)
    parser.add_argument('--checkpoint', type=int, default=128)
    parser.add_argument('--rounds', type=int, default=16)
    parser.add_argument('--binary', type=Path, required=True)
    args = parser.parse_args()
    args.output.mkdir(parents=True, exist_ok=False)
    shutil.copy2(__file__, args.output/'coupled_placement_probe.py')
    problem = json.loads(args.problem.read_text())
    trace = json.loads(args.trace.read_text())
    frame = trace['frames'][args.checkpoint]
    assert frame['sweep'] == args.checkpoint
    ids = sorted(c['id'] for c in problem['components'])
    by = {c['id']: c for c in problem['components']}
    for c in by.values():
        assert c['constraints']['movement'] == 'free' and c['constraints']['rotation'] == 'fixed'
    assert all(p['rotation_degrees'] == 0 for p in frame['poses'])
    positions = {p['component']: p['position'] for p in frame['poses']}
    x0 = np.array([positions[ref][axis] for ref in ids for axis in ['x', 'y']])
    current = x0.copy()
    dimension = len(x0)
    rows, rhs, contacts = [], [], {}
    for i, ref in enumerate(ids):
        low, high = bounds(problem, by[ref])
        for axis in range(2):
            row = np.zeros(dimension);row[2*i+axis] = 1
            rows.extend([row, -row]);rhs.extend([low[axis]-x0[2*i+axis], x0[2*i+axis]-high[axis]])
    fixed_count = len(rows)
    report = {'checkpoint_sweep': args.checkpoint, 'geometry_accepted': False, 'rounds': [],
              'scope': __doc__, 'uses_successful_reference_poses': False}
    config = copy.deepcopy(trace['config']);config['policy']['legalization_sweeps'] = 1
    config['policy']['maximum_pair_checks'] = max(1000000, len(ids)**2)
    (args.output/'config.json').write_text(json.dumps(config, indent=2)+'\n')
    start = time.monotonic()
    for iteration in range(args.rounds):
        for contact in frame['contacts']:
            normal = np.array([contact['normal'][k] for k in ['x', 'y']])
            a, b = ids.index(contact['first']), ids.index(contact['second'])
            row = np.zeros(dimension);row[2*a:2*a+2] = normal;row[2*b:2*b+2] = -normal
            key = (contact['first'], contact['second'], *np.round(normal, 8))
            # A small explicit numerical margin, not a relaxed geometry gate.
            contacts[key] = (row, float(contact['residual_mm'] + row@(current-x0) + 2e-6))
        rows = rows[:fixed_count] + [value[0] for value in contacts.values()]
        rhs = rhs[:fixed_count] + [value[1] for value in contacts.values()]
        try:
            correction, evidence = project(rows, rhs)
        except ValueError as error:
            report['error'] = str(error)
            break
        current = x0 + correction
        candidate = copy.deepcopy(problem)
        for c in candidate['components']:
            i = ids.index(c['id']);c['position'] = dict(zip(['x', 'y'], map(float, current[2*i:2*i+2])))
        path = args.output/f'round-{iteration:02d}-problem.json'
        path.write_text(json.dumps(candidate, indent=2)+'\n')
        destination = args.output/f'round-{iteration:02d}'
        with (args.output/f'round-{iteration:02d}.log').open('w') as log:
            result = subprocess.run([sys.executable, str(Path(__file__).with_name('trace_placement.py')),
                str(path), str(args.output/'config.json'), str(destination), '--binary', str(args.binary)],
                stdout=log, stderr=subprocess.STDOUT)
        assert result.returncode == 0, destination
        observed = json.loads((destination/'trace.json').read_text())
        frame = observed['frames'][0]
        actual = {p['component']:p['position'] for p in frame['poses']}
        assert max(abs(actual[ref][axis]-current[2*i+j]) for i, ref in enumerate(ids) for j, axis in enumerate(['x', 'y'])) < 1e-8
        evidence.update(round=iteration, rows=len(rows), remaining_contacts=len(frame['contacts']),
                        maximum_residual_mm=max((c['residual_mm'] for c in frame['contacts']), default=0),
                        placement_accepted=observed['error'] is None)
        report['rounds'].append(evidence)
        if not frame['contacts'] and observed['error'] is None:
            report['geometry_accepted'] = True
            report['selected_directory'] = str(destination)
            break
    report['seconds'] = time.monotonic()-start
    (args.output/'report.json').write_text(json.dumps(report, indent=2)+'\n')
    links = ''.join(f'<li><a href="round-{i:02d}/index.html">Coupled round {i}</a></li>' for i in range(len(report['rounds'])))
    (args.output/'index.html').write_text('<!doctype html><meta charset="utf-8"><h1>Coupled placement probe</h1><p>Experimental geometry corrections; native verification and routing remain separate.</p><ul>'+links+'</ul><a href="report.json">Evidence</a>')
    print(json.dumps(report, indent=2))


if __name__ == '__main__':
    main()
