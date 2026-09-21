#!/usr/bin/env python3
# Copyright (C) 2026 Toit contributors.
"""Continue a retained dense placement with budget guidance and one-net rip-up."""
import argparse
import copy
import hashlib
import json
from pathlib import Path
import shutil
import subprocess
import time

ROOT = Path(__file__).resolve().parents[2]


def read(path):
    return json.loads(path.read_text())


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('output', type=Path)
    parser.add_argument('--case', default='junction-blocked', choices=['junction-blocked', 'harmonic-blocked'])
    args = parser.parse_args()
    out = args.output.resolve()
    out.mkdir(parents=True, exist_ok=False)
    source = ROOT / 'build/placement-dense-prefix19-2026-09-07' / args.case
    progress = read(source / 'run/solve/progression.json')
    cold = read(source / 'run/cold-prefix.json')
    assert progress['termination'] == 'rolled_back'
    insertion = read(ROOT / 'experiments/configs/dual-esp32-blocked-adaptive-search.json')
    insertion['single_connection_ripup'] = {
        'diagnosis': {'maximum_yielding_connections': 1, 'maximum_trials': 19,
                      'routing': copy.deepcopy(insertion['local_routing_portfolio'][0])}, 'via_penalty_mm': 2.0}
    config = {'maximum_connections': 19 - progress['final_rung'], 'reuse_committed_parent_verification': True,
              'insertion': insertion}
    (out / 'config.json').write_text(json.dumps(config, indent=2) + '\n')
    executable = out / 'pcb-maker'
    shutil.copy2(ROOT / 'target/release/pcb-maker', executable)
    digest = hashlib.sha256(executable.read_bytes()).hexdigest()
    command = [str(executable), 'progress-kicad-connections', cold['template']['declaration'],
               progress['final_directory'], str(out / 'run'), str(out / 'config.json')]
    manifest = {'source_case': args.case, 'source_rung': progress['final_rung'], 'target_rung': 19,
                'executable_sha256': digest, 'command': command, 'status': 'running', 'scope': __doc__}
    def save():
        (out / 'manifest.json').write_text(json.dumps(manifest, indent=2) + '\n')
    save()
    start = time.monotonic()
    with (out / 'run.log').open('w') as log:
        result = subprocess.run(command, stdout=log, stderr=subprocess.STDOUT)
    manifest.update(status='terminal', exit_code=result.returncode, wall_seconds=time.monotonic() - start)
    assert hashlib.sha256(executable.read_bytes()).hexdigest() == digest
    save()
    print(json.dumps(manifest, indent=2), flush=True)
    with (out / 'summary.log').open('w') as log:
        subprocess.run(['python3', str(Path(__file__).with_name('summarize.py')), str(out)],
                       stdout=log, stderr=subprocess.STDOUT, check=True)
    print(out / 'index.html', flush=True)


if __name__ == '__main__':
    main()
