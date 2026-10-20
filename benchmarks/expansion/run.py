#!/usr/bin/env python3
# Copyright (C) 2026 Toit contributors.
"""Unattended acquisition, quick triage, deeper routing and placement data.

A small quick-triage pool keeps up with downloads while the remaining CPU
slots perform deeper routing and generate placement data. Every stage resumes.
Send SIGTERM to this supervisor, or create data/STOP, to stop its children.
"""

import argparse
from collections import Counter
import fcntl
import json
import os
from pathlib import Path
import signal
import subprocess
import sys
import threading
import time

import triage

HERE = Path(__file__).resolve().parent
ROOT = HERE.parents[1]
STOP = threading.Event()


def read_results(output):
    rows = [triage.read_json(p) for p in output.glob('boards/*/result.json')]
    latest = {}
    for row in sorted((r for r in rows if r), key=lambda r: r.get('finished_at', '')):
        latest[row.get('provenance', {}).get('source_id', row['name'])] = row
    return list(latest.values())


def select_manifest(rows, destination, categories, limit, completed_output=None):
    selected = []
    seen = {(r.get('provenance', {}).get('source_id', r['name']),
             r.get('provenance', {}).get('intake_parent_fingerprint', 'legacy'))
            for r in read_results(completed_output)
            if r.get('category') != 'cancelled'} if completed_output else set()
    for row in rows:
        source = row.get('provenance', {})
        parent = row.get('fingerprint', 'legacy')
        key = (source.get('source_id', row['name']), parent)
        if row.get('category') not in categories or key in seen:
            continue
        seen.add(key)
        selected.append(dict(source, intake_parent_fingerprint=parent))
    # Stable order is important: resumption must not repeatedly choose a
    # different subset. Limits bound the time until fresh quick triage runs.
    selected.sort(key=lambda b: b['name'])
    triage.atomic_json(destination, {'boards': selected[:limit]})
    return len(selected[:limit])


def stop_child(process):
    if process.poll() is not None:
        return
    try:
        os.killpg(process.pid, signal.SIGTERM)
        process.wait(timeout=20)
    except ProcessLookupError:
        pass
    except subprocess.TimeoutExpired:
        os.killpg(process.pid, signal.SIGKILL)
        process.wait()


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--data', type=Path, default=HERE / 'data')
    parser.add_argument('--binary', type=Path, default=ROOT / 'target/release/pcb-maker')
    parser.add_argument('--workers', type=int, default=28)
    parser.add_argument('--quick-workers', type=int, default=8)
    parser.add_argument('--download-workers', type=int, default=6)
    parser.add_argument('--hours', type=float, default=24)
    parser.add_argument('--batch-size', type=int, default=128)
    parser.add_argument('--reserve-gib', type=float, default=10)
    parser.add_argument('--deep-seconds', type=float, default=300)
    parser.add_argument('--variants', type=int, default=4)
    args = parser.parse_args()
    if min(args.workers, args.quick_workers, args.download_workers, args.hours, args.batch_size,
           args.deep_seconds, args.variants) <= 0:
        parser.error('worker counts, hours, batch size, budgets and variants must be positive')
    if args.quick_workers >= args.workers:
        parser.error('quick workers must leave at least one worker for follow-up jobs')
    data = args.data.resolve()
    data.mkdir(parents=True, exist_ok=True)
    lock = (data / 'supervisor.lock').open('w')
    try:
        fcntl.flock(lock, fcntl.LOCK_EX | fcntl.LOCK_NB)
    except BlockingIOError:
        parser.error('a supervisor already owns this dataset')
    if (data / 'STOP').exists():
        parser.error('data/STOP exists; remove it deliberately before restarting')
    signal.signal(signal.SIGTERM, lambda *_: STOP.set())
    signal.signal(signal.SIGINT, lambda *_: STOP.set())
    binary_hash = triage.digest(args.binary)
    binary = data / ('pcb-maker-' + binary_hash[:16])
    if not binary.exists():
        import shutil
        shutil.copy2(args.binary, binary)
    if triage.digest(binary) != binary_hash:
        parser.error('binary changed during snapshot')
    binary.chmod(0o755)
    started = time.time()
    deadline = started + args.hours * 3600
    metadata = {'pid': os.getpid(), 'started_at': started,
                'deadline_at': deadline, 'binary_sha256': binary_hash,
                'git_head': subprocess.check_output(['git', 'rev-parse', 'HEAD'], cwd=ROOT, text=True).strip(),
                'workers': args.workers, 'reserve_gib': args.reserve_gib,
                'data': str(data), 'supervisor_sha256': triage.digest(__file__)}
    native_cores = ','.join(map(str, sorted(os.sched_getaffinity(0))[-4:]))
    metadata['native_cores'] = native_cores
    triage.atomic_json(data / 'supervisor.json', metadata)
    children = []

    def launch(name, command):
        children[:] = [p for p in children if p.poll() is None]
        with (data / (name + '.log')).open('a') as log:
            process = subprocess.Popen([str(x) for x in command], cwd=ROOT,
                                       stdout=log, stderr=subprocess.STDOUT,
                                       start_new_session=True)
        children.append(process)
        metadata.update(current_stage=name, stage_pid=process.pid, updated_at=time.time())
        triage.atomic_json(data / 'supervisor.json', metadata)
        print(json.dumps({'event': 'start', 'stage': name, 'pid': process.pid}), flush=True)
        return process

    def stopping():
        return STOP.is_set() or (data / 'STOP').exists() or time.time() >= deadline

    def execute(name, command):
        process = launch(name, command)
        while process.poll() is None and not stopping():
            STOP.wait(2)
        if stopping():
            stop_child(process)
        print(json.dumps({'event': 'finished', 'stage': name,
                          'exit_code': process.poll()}), flush=True)
        return process.returncode

    def route_command(manifest, output, seconds, workers=None, memory=1.5, rss=3):
        return [sys.executable, HERE / 'triage.py', '--manifest', manifest,
                '--output', output, '--binary', binary,
                '--workers', workers or (args.workers - args.quick_workers), '--native-workers', 2,
                '--route-seconds', seconds, '--deadline-seconds', max(90, seconds * 3),
                '--route-timeout', max(180, seconds * 3 + 60),
                '--reserve-gib', args.reserve_gib, '--job-memory-gib', memory,
                '--max-rss-gib', rss, '--min-disk-gib', 25,
                '--memory-gate', data / 'memory-gate.json', '--native-cores', native_cores]

    try:
        harvest = launch('harvest', [sys.executable, HERE / 'harvest.py', 'run',
                                    '--data', data, '--workers', args.download_workers,
                                    '--watch', '--non-kicad'])
        quick = data / 'runs/quick'
        quick_process = launch('quick', route_command(data / 'manifest.json', quick, 30,
                                                     workers=args.quick_workers) + ['--watch'])
        metadata.update(quick_pid=quick_process.pid, harvester_pid=harvest.pid)
        cycle = 0
        while not stopping():
            if harvest.poll() is not None:
                raise RuntimeError('harvester exited; inspect data/harvest.log before resuming')
            if quick_process.poll() is not None:
                raise RuntimeError('quick triage exited; inspect data/quick.log before resuming')
            cycle += 1
            rows = read_results(quick)
            high_manifest = data / 'high-memory-manifest.json'
            if select_manifest(rows, high_manifest, {'resource_limit'}, args.batch_size,
                               data / 'runs/high-memory'):
                execute('high-memory', route_command(high_manifest, data / 'runs/high-memory',
                                                    30, workers=4, memory=7, rss=10))
            rows += read_results(data / 'runs/high-memory')
            if stopping():
                break
            execute('conversions', [sys.executable, HERE / 'import_pending.py',
                                    '--data', data, '--limit', 16, '--workers', 2])
            if stopping():
                break
            deep_manifest = data / 'deep-manifest.json'
            if select_manifest(rows, deep_manifest, {
                    'incomplete_at_budget', 'incomplete_early', 'cut_wall_clock',
                    'clean_slow', 'clean_medium'}, args.batch_size, data / 'runs/deep'):
                execute('deep', route_command(deep_manifest, data / 'runs/deep', args.deep_seconds))
            if stopping():
                break
            # Export features for diverse eligible boards, preserving exact
            # saved placements; full routing supplies their eventual labels.
            samples = data / 'samples'
            execute('features', [sys.executable, HERE / 'samples.py', '--triage-output', quick,
                                 '--output', samples, '--binary', binary,
                                 '--workers', min(args.workers, 16), '--variants', args.variants,
                                 '--limit', args.batch_size, '--reserve-gib', args.reserve_gib,
                                 '--memory-gate', data / 'memory-gate.json'])
            if stopping():
                break
            manifest = triage.read_json(samples / 'manifest.json')
            if manifest and manifest['boards']:
                execute('placement-labels', route_command(samples / 'manifest.json',
                                                         data / 'runs/placement-labels', args.deep_seconds))
            if stopping():
                break
            converted = triage.read_json(data / 'converted-manifest.json')
            if converted and converted['boards']:
                execute('experimental', route_command(data / 'converted-manifest.json',
                                                       data / 'runs/experimental', 30, workers=2,
                                                       memory=4, rss=6))
            summaries = {name: triage.read_json(data / 'runs' / name / 'status.json')
                         for name in ('quick', 'high-memory', 'experimental', 'deep', 'placement-labels')}
            triage.atomic_json(data / 'status.json', {
                'cycle': cycle, 'updated_at': time.time(), 'pid': os.getpid(),
                'stages': summaries, 'samples': triage.read_json(samples / 'status.json'),
                'harvester_pid': harvest.pid, 'deadline_at': deadline})
            STOP.wait(15)
    finally:
        STOP.set()
        for process in reversed(children):
            stop_child(process)
        metadata['stopped_at'] = time.time()
        triage.atomic_json(data / 'supervisor.json', metadata)
        lock.close()


if __name__ == '__main__':
    main()
