#!/usr/bin/env python3
# Copyright (C) 2026 Toit contributors.
"""Export immutable placement features and queue variants for measured routing.

This does not train a network. Features alone carry no success label; triage
must measure the exact saved placement. Splits group whole repositories so
revisions cannot silently straddle training and evaluation.
"""

import argparse
from concurrent.futures import ThreadPoolExecutor, as_completed
import hashlib
import json
import os
from pathlib import Path
import shutil
import signal
import threading
import time

import triage

HERE = Path(__file__).resolve().parent
ROOT = HERE.parents[1]


def family_split(repository):
    value = int(hashlib.sha256(repository.lower().encode()).hexdigest()[:8], 16) % 100
    return 'train' if value < 80 else ('validation' if value < 90 else 'test')


def variants(count):
    result = [{'name': 'designer', 'skip_probe': True}]
    for i in range(1, count):
        result.append({'name': f'perturb-{i}', 'skip_probe': True,
                       'perturb': {'moves': 5 * i, 'swaps': i, 'rotations': i,
                                   'max_mm': min(2 * i, 8), 'seed': i}})
    return result


def eligible(row):
    return (row.get('nn_license_eligible', False)
            and row.get('source_quality') == 'native_clean'
            and not row.get('rules_inferred', True)
            and row.get('category') not in {
                'source_issue', 'import_error', 'model_error', 'resource_limit',
                'unresolved_timeout', 'cancelled'})


def unique_placements(boards):
    """Keep exact saved-geometry/rule duplicates as aliases, not extra routes."""
    result = {}
    for board in boards:
        key = (board['sha256'], json.dumps(board.get('rule_files_sha256', {}), sort_keys=True))
        result.setdefault(key, board)
    return list(result.values())


def retry_export(result):
    process = result.get('process', {})
    transient = process.get('stop_reason') in {
        'cancelled', 'wall_timeout', 'rss_limit', 'allocation_limit', 'spawn_error'}
    return (result.get('status') != 'exported' and result.get('attempts', 1) < 3
            and (transient or process.get('exit_code', 0) != 0))


def export_one(row, args, binary_hash):
    provenance = row['provenance']
    repository = provenance.get('repository', provenance.get('source_id', row['name']))
    config = {'variants': variants(args.variants), 'keep_boards': True}
    identity = {'parent': row['fingerprint'], 'binary_sha256': binary_hash,
                'job': config, 'exporter_sha256': triage.digest(__file__)}
    key = hashlib.sha256(json.dumps(identity, sort_keys=True).encode()).hexdigest()
    work = args.output / 'exports' / key
    saved = triage.read_json(work / 'result.json')
    if saved and not retry_export(saved):
        return saved
    work.mkdir(parents=True, exist_ok=True)
    # Use the original declared rules and designer placement. The exporter
    # strips routing itself, just as the measured variant route will do.
    source = Path(row['work_directory']) / 'reference'
    board_id = provenance['board_id']
    temporary = work / 'tmp'
    temporary.mkdir(exist_ok=True)
    environment = dict(os.environ, TMPDIR=str(temporary), RAYON_NUM_THREADS='1',
                       OMP_NUM_THREADS='1', OPENBLAS_NUM_THREADS='1')
    for name in list(environment):
        if name.startswith('PCB_'):
            del environment[name]
    job = work / 'job.json'
    triage.atomic_json(job, config)
    # The parent schedules bounded batches; wait before allocating on a
    # machine that became busy with another user's work.
    while not triage.STOP.is_set():
        with args.gate_lock:
            if triage.admission_gate(args, args.active) is None:
                args.active += 1
                break
        triage.STOP.wait(1)
    else:
        return {'parent': row['name'], 'boards': [], 'status': 'cancelled'}
    output = work / 'features'
    try:
        process = triage.run_process(
            ['nice', '-n', '15', args.binary, 'export-congestion-sample', source,
             board_id, output, job, 'auto'], work / 'export.log', args.timeout,
            args.max_rss_gib, environment)
    finally:
        with args.gate_lock:
            args.active -= 1
            triage.shared_memory_gate(args, active=args.active)
    boards = []
    for variant in config['variants']:
        name = variant['name']
        header = output / f'{name}.json'
        geometry = output / f'{name}.kicad_pcb'
        blob = output / f'{name}.bin'
        if not all(p.is_file() for p in (header, geometry, blob)):
            continue
        project = work / 'projects' / name
        project.mkdir(parents=True, exist_ok=True)
        shutil.copy2(geometry, project / 'board.kicad_pcb')
        for extension in ('.kicad_pro', '.kicad_dru'):
            sidecar = source / (board_id + extension)
            if sidecar.is_file():
                shutil.copy2(sidecar, project / ('board' + extension))
        boards.append({
            'name': row['name'] + '__' + name + '__' + key[:8],
            'source_id': provenance.get('source_id', row['name']) + '#' + name,
            'board_id': 'board', 'directory': str(project),
            'sha256': triage.digest(project / 'board.kicad_pcb'),
            'rule_files_sha256': {p.suffix: triage.digest(p) for p in project.iterdir()
                                 if p.suffix in ('.kicad_pro', '.kicad_dru')},
            'repository': repository, 'family': repository,
            'split': family_split(repository), 'rules_inferred': False,
            'nn_license_eligible': True,
            'license_spdx': provenance.get('license_spdx'),
            'license_scope': provenance.get('license_scope'),
            'commit': provenance.get('commit'),
            'parent_source_sha256': row['source_sha256'],
            'parent_triage_fingerprint': row['fingerprint'],
            'features_header': str(header), 'features_sha256': triage.digest(blob),
            'feature_binary_sha256': binary_hash, 'variant': variant,
            'label_status': 'unmeasured; join exact placement to subsequent triage result',
        })
    result = {'key': key, 'identity': identity, 'parent': row['name'],
              'process': process, 'boards': boards,
              'attempts': (saved or {}).get('attempts', 0) + 1,
              'status': 'exported' if len(boards) == args.variants else 'partial_or_failed'}
    triage.atomic_json(work / 'result.json', result)
    return result


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--triage-output', required=True, type=Path)
    parser.add_argument('--output', required=True, type=Path)
    parser.add_argument('--binary', required=True, type=Path)
    parser.add_argument('--workers', type=int, default=8)
    parser.add_argument('--variants', type=int, default=4)
    parser.add_argument('--limit', type=int, default=100)
    parser.add_argument('--reserve-gib', type=float, default=10)
    parser.add_argument('--memory-gate', type=Path)
    parser.add_argument('--job-memory-gib', type=float, default=1.5)
    parser.add_argument('--min-disk-gib', type=float, default=25)
    parser.add_argument('--max-load', type=float, default=(os.cpu_count() or 1) * 1.5)
    parser.add_argument('--max-rss-gib', type=float, default=3)
    parser.add_argument('--timeout', type=float, default=180)
    args = parser.parse_args()
    if args.workers < 1 or args.variants < 1 or args.limit < 1:
        parser.error('workers, variants and limit must be positive')
    args.output = args.output.resolve()
    args.binary = args.binary.resolve()
    args.output.mkdir(parents=True, exist_ok=True)
    args.gate_lock = threading.Lock()
    args.active = 0
    signal.signal(signal.SIGTERM, lambda *_: triage.STOP.set())
    signal.signal(signal.SIGINT, lambda *_: triage.STOP.set())
    import fcntl
    with (args.output / 'samples.lock').open('w') as lock:
        fcntl.flock(lock, fcntl.LOCK_EX | fcntl.LOCK_NB)
        rows = [triage.read_json(p) for p in args.triage_output.glob('boards/*/result.json')]
        latest = {}
        for row in sorted((r for r in rows if r), key=lambda r: r.get('finished_at', '')):
            latest[row.get('provenance', {}).get('source_id', row['name'])] = row
        rows = [r for r in latest.values() if eligible(r)]
        # Keep easy examples, but spend the first collection work on real
        # routing pressure. Preserve diversity with round-robin repositories.
        groups = {}
        for row in sorted(rows, key=lambda r: (r['category'] == 'clean_fast', r['name'])):
            repository = row['provenance'].get('repository', row['name'])
            groups.setdefault(repository, []).append(row)
        ordered = []
        while groups:
            for repository in list(groups):
                ordered.append(groups[repository].pop(0))
                if not groups[repository]:
                    del groups[repository]
        binary_hash = triage.digest(args.binary)
        configuration = {'variants': variants(args.variants), 'keep_boards': True}
        previous = [triage.read_json(p) for p in args.output.glob('exports/*/result.json')]
        previous = [r for r in previous if r
                    and r.get('identity', {}).get('binary_sha256') == binary_hash
                    and r['identity'].get('job') == configuration
                    and r['identity'].get('exporter_sha256') == triage.digest(__file__)]
        processed = {r['identity']['parent'] for r in previous if not retry_export(r)}
        rows = [r for r in ordered if r['fingerprint'] not in processed][:args.limit]
        # Limit concurrent growth before features allocate memory.
        available_workers = max(1, int(triage.memory_available() / triage.GIB - args.reserve_gib))
        boards = [b for r in previous for b in r['boards']]
        with ThreadPoolExecutor(max_workers=min(args.workers, available_workers)) as pool:
            futures = [pool.submit(export_one, r, args, binary_hash) for r in rows]
            for future in as_completed(futures):
                result = future.result()
                boards.extend(result['boards'])
                triage.atomic_json(args.output / 'manifest.json', {'boards': unique_placements(boards)})
                print(json.dumps({'parent': result['parent'], 'status': result['status'],
                                  'variants': len(result['boards'])}), flush=True)
        triage.shared_memory_gate(args, remove=True)
        boards = unique_placements(boards)
        triage.atomic_json(args.output / 'manifest.json', {'boards': boards})
        triage.atomic_json(args.output / 'status.json', {
            'parents': len(rows), 'variants': len(boards),
            'splits': {s: sum(b['split'] == s for b in boards)
                       for s in ('train', 'validation', 'test')},
            'label_contract': 'Features only. Join by exact geometry hash to measured routing results; failures and missing DRC are not successes.'})


if __name__ == '__main__':
    main()
