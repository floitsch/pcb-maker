#!/usr/bin/env python3
# Copyright (C) 2026 Toit contributors.
"""Read-only snapshot of the local board expansion pipeline."""

import argparse
from collections import Counter
from datetime import datetime, timezone
import json
from pathlib import Path


def read(path, default=None):
    try:
        return json.loads(path.read_text())
    except (OSError, ValueError):
        return default


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--data', type=Path, default=Path(__file__).resolve().parent / 'data')
    parser.add_argument('--json', action='store_true')
    args = parser.parse_args()
    data = args.data
    supervisor = read(data / 'supervisor.json', {})
    manifest = read(data / 'manifest.json', {'boards': []})
    outcomes = {}
    journal = data / 'harvest.jsonl'
    if journal.exists():
        for line in journal.read_text().splitlines():
            try:
                row = json.loads(line)
                outcomes[row['source_id']] = row
            except (ValueError, KeyError):
                pass
    stages = {}
    for path in (data / 'runs').glob('*/status.json'):
        stages[path.parent.name] = read(path)
    answer = {
        'supervisor': supervisor, 'downloaded_native_boards': len(manifest['boards']),
        'harvest_outcomes': dict(Counter(r['status'] for r in outcomes.values())),
        'repositories': len({b['repository'] for b in manifest['boards']}),
        'stages': stages, 'samples': read(data / 'samples/status.json'),
        'converted_boards': len(read(data / 'converted-manifest.json', {'boards': []})['boards']),
        'stop_requested': (data / 'STOP').exists(),
    }
    if args.json:
        print(json.dumps(answer, indent=2))
        return
    deadline = supervisor.get('deadline_at')
    deadline = datetime.fromtimestamp(deadline, timezone.utc).isoformat(timespec='seconds') if deadline else 'unknown'
    print(f"Supervisor PID {supervisor.get('pid', '?')}; deadline {deadline}; stop requested {answer['stop_requested']}")
    print(f"Native boards: {answer['downloaded_native_boards']} from {answer['repositories']} repositories; experimental imports: {answer['converted_boards']}")
    print('Harvest outcomes:', json.dumps(answer['harvest_outcomes'], sort_keys=True))
    for name, stage in sorted(stages.items()):
        if not stage:
            continue
        print(f"{name}: {stage['completed']}/{stage['total']} completed, {stage['active']} active, {stage['pending']} pending; gate {stage.get('gate')}; updated {stage.get('updated_at')}")
        print('  ', json.dumps(stage.get('categories', {}), sort_keys=True))
    if answer['samples']:
        print('Placement variants:', json.dumps(answer['samples'], sort_keys=True))


if __name__ == '__main__':
    main()
