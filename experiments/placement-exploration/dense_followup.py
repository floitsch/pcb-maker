#!/usr/bin/env python3
# Copyright (C) 2026 Toit contributors.
"""Continue one stalled dense probe with guidance or one-net rip-up.

The original comparison is immutable. This counterfactual changes only the
routing heuristic or enables the existing bounded one-net rip-up policy.
Native verification previews are retained through the progression command.
"""
import argparse
import hashlib
import json
from pathlib import Path
import subprocess
import time


def read(path):
    return json.loads(path.read_text())


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('directory', type=Path)
    parser.add_argument('case')
    parser.add_argument('--label', help='Fresh output name for an explicitly separate replay')
    parser.add_argument('--policy', choices=['guided', 'ripup'], default='guided')
    parser.add_argument('--maximum-connections', type=int)
    args = parser.parse_args()
    root = args.directory.resolve()
    case = root / args.case
    manifest = read(root / 'manifest.json')
    record = next(c for c in manifest['cases'] if c['id'] == args.case)
    assert record['status'] == 'terminal'
    progress = read(case / 'run/solve/progression.json')
    assert progress['termination'] == 'rolled_back'
    attempts = progress['steps'][-1]['result']['evidence']['attempts']
    trigger = 'exhausted its search budget' if args.policy == 'guided' else 'no path on the routing grid'
    assert len(attempts) == 1 and trigger in attempts[0]['error']
    out = root / f'{args.policy}-followups' / (args.label or args.case)
    out.mkdir(parents=True, exist_ok=False)
    executable = root / 'pcb-maker'
    assert hashlib.sha256(executable.read_bytes()).hexdigest() == manifest['executable_sha256']
    insertion = read(case / 'routing.json')
    assert len(insertion['local_routing_portfolio']) == 1
    if args.policy == 'guided':
        insertion['local_routing_portfolio'][0]['heuristic'] = 'obstacle_distances'
    else:
        insertion['single_connection_ripup'] = {
            'diagnosis': {'maximum_yielding_connections': 1, 'maximum_trials': progress['final_rung'],
                          'routing': insertion['local_routing_portfolio'][0]}, 'via_penalty_mm': 2.0}
    maximum = args.maximum_connections or manifest['target_connections'] - progress['final_rung']
    assert maximum > 0
    config = {'maximum_connections': maximum,
              'reuse_committed_parent_verification': True, 'insertion': insertion}
    (out / 'config.json').write_text(json.dumps(config, indent=2) + '\n')
    cold = read(case / 'run/cold-prefix.json')
    command = [str(executable), 'progress-kicad-connections', cold['template']['declaration'],
               progress['final_directory'], str(out / 'run'), str(out / 'config.json')]
    result = {'case': args.case, 'policy': args.policy, 'source_rung': progress['final_rung'],
              'failed_connection': progress['steps'][-1]['result']['evidence']['inserted_connection'],
              'executable_sha256': manifest['executable_sha256'], 'command': command, 'status': 'running',
              'scope': __doc__}
    def save():
        (out / 'manifest.json').write_text(json.dumps(result, indent=2) + '\n')
    save()
    start = time.monotonic()
    with (out / 'run.log').open('w') as log:
        process = subprocess.run(command, stdout=log, stderr=subprocess.STDOUT)
    result.update(status='terminal', exit_code=process.returncode, wall_seconds=time.monotonic() - start)
    save()
    print(json.dumps(result, indent=2))


if __name__ == '__main__':
    main()
