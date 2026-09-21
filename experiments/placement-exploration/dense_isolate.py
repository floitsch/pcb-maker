#!/usr/bin/env python3
# Copyright (C) 2026 Toit contributors.
"""Native-check a stalled connection first, on its unchanged placement seed."""
import argparse
import hashlib
import json
from pathlib import Path
import shutil
import subprocess
import time


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('directory', type=Path)
    parser.add_argument('case')
    args = parser.parse_args()
    root = args.directory.resolve()
    source = root / args.case
    manifest = json.loads((root / 'manifest.json').read_text())
    assert next(c for c in manifest['cases'] if c['id'] == args.case)['status'] == 'terminal'
    progress = json.loads((source / 'run/solve/progression.json').read_text())
    assert progress['termination'] == 'rolled_back'
    target = progress['steps'][-1]['result']['evidence']['inserted_connection']
    out = root / 'isolated-followups' / args.case
    out.mkdir(parents=True, exist_ok=False)
    for name in ['problem.json', 'placement.json', 'routing.json']:
        shutil.copy2(source / name, out / name)
    template = json.loads((source / 'template.json').read_text())
    template['connection_order'] = [target] + [n for n in template['connection_order'] if n != target]
    (out / 'template.json').write_text(json.dumps(template, indent=2) + '\n')
    executable = root / 'pcb-maker'
    assert hashlib.sha256(executable.read_bytes()).hexdigest() == manifest['executable_sha256']
    command = [str(executable), 'solve-semantic-kicad-prefix', str(out / 'problem.json'),
               str(out / 'placement.json'), str(out / 'template.json'), '1', str(out / 'run'), str(out / 'routing.json')]
    record = {'case': args.case, 'target': target, 'command': command, 'status': 'running',
              'executable_sha256': manifest['executable_sha256'],
              'scope': 'Same declared placement and rules; move the failed connection first and prune to one connection before routing from zero copper. Requires post-run native geometry comparison to establish matching terminal access.'}
    def save():
        (out / 'manifest.json').write_text(json.dumps(record, indent=2) + '\n')
    save()
    start = time.monotonic()
    with (out / 'run.log').open('w') as log:
        result = subprocess.run(command, stdout=log, stderr=subprocess.STDOUT)
    record.update(status='terminal', exit_code=result.returncode, wall_seconds=time.monotonic() - start)
    save()
    print(json.dumps(record, indent=2))


if __name__ == '__main__':
    main()
