#!/usr/bin/env python3
# Copyright (C) 2026 Toit contributors.
"""Convert a bounded batch of harvested non-KiCad boards for experimental triage.

Writes converted-manifest.json atomically and appends conversion-outcomes.jsonl.
No converted board is a verified neural-network routing label. Sources stay intact.
"""
import argparse
import concurrent.futures
import fcntl
import hashlib
import importlib.util
import json
import os
from pathlib import Path
import shutil
import signal
import subprocess
import sys
import time

HERE = Path(__file__).resolve().parent
ROOT = HERE.parents[1]
GIB = 1024 ** 3
STOP = False
sys.path.append(str(HERE.parent))
import harness_common  # noqa: E402


digest = harness_common.file_sha256
read_json = harness_common.read_json


def atomic_json(path, value):
    harness_common.atomic_json(path, value, indent=2, newline=True)


def journal(path):
    latest = {}
    if Path(path).exists():
        with Path(path).open() as stream:
            for line in stream:
                try:
                    row = json.loads(line)
                    latest[row['source_id']] = row
                except (ValueError, KeyError, TypeError):
                    continue
    return latest


def relative_source(row, data):
    value = Path(row['board_path'])
    if value.is_absolute():
        return value.resolve()
    # Harvest paths are repository-relative; hand-written local records may use
    # paths relative to the data directory. Never interpret source path as code.
    for parent in (ROOT, data, data.parent):
        if (parent / value).is_file():
            return (parent / value).resolve()
    return (ROOT / value).resolve()


def sources(data):
    found = {k: v for k, v in journal(data / 'harvest.jsonl').items()
             if v.get('status') == 'conversion_pending'}
    # Include the explicitly retained real-source evaluation examples.
    for path in sorted((data / 'non-kicad').glob('*/provenance.json')):
        metadata = read_json(path)
        if not metadata or not metadata.get('repository') or not metadata.get('path'):
            continue
        source_id = metadata['repository'] + ':' + metadata['path']
        candidates = sorted(path.parent.glob('source.*'))
        candidates = [p for p in candidates if p.suffix.lower() in ('.pcbdoc', '.brd')]
        if candidates:
            source = candidates[0]
            found.setdefault(source_id, dict(metadata, source_id=source_id,
                board_path=str(source.resolve()), directory=str(path.parent.resolve()),
                source_format='altium' if source.suffix.lower() == '.pcbdoc' else 'eagle',
                status='conversion_pending'))
    return found


def safe_name(source_id):
    return harness_common.safe_name(source_id, limit=90)


def signature(source_hash, converter_hash, kicad_version):
    return hashlib.sha256(json.dumps([source_hash, converter_hash, kicad_version]).encode()).hexdigest()


def eligible(row, outcome, source_hash, converter_hash, version):
    """Publish only a currently matching successful conversion and artifact."""
    if row.get('sha256') and source_hash != row['sha256']:
        return False
    if not outcome or outcome.get('status') != 'imported_unvalidated':
        return False
    if outcome.get('fingerprint') != signature(source_hash, converter_hash, version):
        return False
    board = Path(outcome.get('converted', ''))
    try:
        if not board.is_file() or digest(board) != outcome.get('converted_sha256'):
            return False
        report = read_json(board.parent / 'conversion.json')
        if not report or report.get('status') != 'imported_unvalidated' or report.get('source_sha256') != source_hash:
            return False
        steps = report.get('steps', {})
        if any(steps.get(name, {}).get('exit_code') != 0 or steps.get(name, {}).get('timeout')
               for name in ('import', 'readback', 'drc')):
            return False
        if any(read_json(board.parent / name) is None for name in ('import.json', 'stats.json', 'drc.json')):
            return False
    except OSError:
        return False
    return True


def publish(data, source_rows, outcomes, converter_hash, version):
    ready = []
    for source_id, original in source_rows.items():
        outcome = outcomes.get(source_id)
        try:
            source_hash = digest(relative_source(original, data))
        except OSError:
            continue
        if not eligible(original, outcome, source_hash, converter_hash, version):
            continue
        board = Path(outcome['converted'])
        ready.append(dict(original, name=safe_name(source_id), board_id='board',
            directory=str(board.parent), board_path=str(board), sha256=outcome['converted_sha256'],
            status='downloaded', conversion_status='imported_unvalidated', source_format='kicad',
            original_source_format=original.get('source_format'), original_source=dict(original),
            original_source_sha256=source_hash, conversion_fingerprint=outcome['fingerprint'],
            conversion_report=str(board.parent / 'conversion.json'), rules_inferred=True,
            project_path=None, nn_license_eligible=False, semantic_validation='unvalidated',
            eligible_as_routing_label=False, experimental_conversion=True))
    atomic_json(data / 'converted-manifest.json', {'schema_version': 1, 'updated_at': time.time(),
        'kicad_version': version, 'converter_sha256': converter_hash,
        'purpose': 'experimental routing triage; source semantics unvalidated', 'boards': ready})
    return len(ready)


def worker_command(arguments, timeout):
    """Keep every CLI in the worker session so the supervisor owns its lifetime."""
    limiter = shutil.which('prlimit')
    bounded = ([limiter, '--as=8589934592', '--core=0', '--'] if limiter else []) + list(map(str, arguments))
    started = time.monotonic()
    process = subprocess.Popen(bounded, stdout=subprocess.PIPE, stderr=subprocess.PIPE,
                               text=True, errors='replace')
    try:
        stdout, stderr = process.communicate(timeout=timeout)
        timed_out = False
    except subprocess.TimeoutExpired:
        process.kill()
        stdout, stderr = process.communicate()
        timed_out = True
    return {'command': list(map(str, arguments)), 'exit_code': process.returncode,
            'timeout': timed_out, 'seconds': round(time.monotonic() - started, 3),
            'stdout': stdout[-16000:], 'stderr': stderr[-16000:]}


def worker(job_path):
    job = read_json(job_path)
    spec = importlib.util.spec_from_file_location('trusted_board_converter', HERE / 'convert.py')
    module = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(module)
    module.command = worker_command
    module.convert_board(job['source'], job['output'], job['format'], job['timeout'],
                         job['metadata'], job['kicad_cli'])


def convert_job(row, data, converter_hash, version, args):
    source_id = row['source_id']
    output = data / 'conversions' / safe_name(source_id)
    output.mkdir(parents=True, exist_ok=True)
    source = relative_source(row, data)
    outcome = {'source_id': source_id, 'recorded_at': time.time(), 'source_metadata': row,
               'converter_sha256': converter_hash, 'kicad_version': version}
    try:
        source_hash = digest(source)
        outcome.update(source_sha256=source_hash,
                       fingerprint=signature(source_hash, converter_hash, version))
        if row.get('sha256') and source_hash != row['sha256']:
            return dict(outcome, status='source_changed', reason='source hash differs from harvest')
        source_format = row.get('source_format', 'auto')
        if source_format == 'eagle-or-legacy':
            with source.open('rb') as stream:
                prefix = stream.read(8192)
            source_format = 'eagle' if b'<eagle' in prefix else 'auto'
        if source_format not in ('altium', 'eagle', 'auto', 'pads', 'cadstar', 'fabmaster', 'pcad', 'solidworks'):
            return dict(outcome, status='unsupported_source', reason='unsupported harvested source format')
        temporary = output / 'tmp'
        temporary.mkdir(exist_ok=True)
        job = {'source': str(source), 'output': str(output), 'format': source_format,
               'timeout': args.stage_timeout, 'metadata': row, 'kicad_cli': args.kicad_cli}
        atomic_json(output / 'job.json', job)
        (output / 'conversion.json').unlink(missing_ok=True)
        environment = dict(os.environ, TMPDIR=str(temporary), OMP_NUM_THREADS='1',
                           OPENBLAS_NUM_THREADS='1', RAYON_NUM_THREADS='1')
        started = time.monotonic()
        stop_reason = None
        with (output / 'worker.log').open('wb') as log:
            process = subprocess.Popen([sys.executable, str(Path(__file__).resolve()),
                '--_worker', str(output / 'job.json')], stdout=log, stderr=subprocess.STDOUT,
                env=environment, start_new_session=True)
            try:
                while process.poll() is None:
                    if STOP:
                        stop_reason = 'cancelled'
                    elif time.monotonic() - started >= args.job_timeout:
                        stop_reason = 'wall_timeout'
                    if stop_reason:
                        break
                    time.sleep(0.1)
            finally:
                try:
                    os.killpg(process.pid, signal.SIGKILL)
                except ProcessLookupError:
                    pass
                process.wait()
        outcome.update(wall_seconds=round(time.monotonic() - started, 3), exit_code=process.returncode)
        if stop_reason:
            return dict(outcome, status=stop_reason, reason='whole conversion process group terminated')
        result = read_json(output / 'conversion.json')
        if process.returncode or not result or result.get('source_sha256') != source_hash:
            return dict(outcome, status='worker_failed', reason='no matching successful worker result; inspect worker.log')
        outcome.update(status=result.get('status', 'worker_failed'), reason=result.get('reason'),
                       converted=str(output / 'board.kicad_pcb'), semantic_validation='unvalidated')
        if outcome['status'] == 'imported_unvalidated':
            outcome['converted_sha256'] = digest(output / 'board.kicad_pcb')
        return outcome
    except Exception as error:
        return dict(outcome, status='worker_failed', reason=str(error))


def append_outcome(path, row):
    with path.open('a+b') as stream:
        stream.seek(0, os.SEEK_END)
        if stream.tell():
            stream.seek(-1, os.SEEK_END)
            if stream.read(1) != b'\n':
                stream.seek(0, os.SEEK_END)
                stream.write(b'\n')
        stream.seek(0, os.SEEK_END)
        stream.write((json.dumps(row) + '\n').encode())
        stream.flush()
        os.fsync(stream.fileno())


def main():
    global STOP
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--data', type=Path, default=HERE / 'data')
    parser.add_argument('--limit', type=int, default=16)
    parser.add_argument('--workers', type=int, default=2)
    parser.add_argument('--stage-timeout', type=float, default=60)
    parser.add_argument('--job-timeout', type=float, default=200)
    parser.add_argument('--reserve-memory-gib', type=float, default=10)
    parser.add_argument('--memory-per-worker-gib', type=float, default=4)
    parser.add_argument('--kicad-cli', default='kicad-cli')
    parser.add_argument('--_worker', type=Path, help=argparse.SUPPRESS)
    args = parser.parse_args()
    if args._worker:
        worker(args._worker)
        return 0
    if args.limit < 1 or args.workers < 1 or min(args.stage_timeout, args.job_timeout, args.memory_per_worker_gib) <= 0 or args.reserve_memory_gib < 0:
        parser.error('positive limit/workers/timeouts/memory required; reserve must be nonnegative')
    args.data = args.data.resolve()
    args.data.mkdir(parents=True, exist_ok=True)
    # One journal/manifest writer; harmlessly skip a concurrent scheduler tick.
    lock = (args.data / 'conversion.lock').open('a')
    try:
        fcntl.flock(lock, fcntl.LOCK_EX | fcntl.LOCK_NB)
    except BlockingIOError:
        print(json.dumps({'status': 'already_running'}))
        return 0
    def stop(signum, frame):
        global STOP
        STOP = True
    signal.signal(signal.SIGTERM, stop)
    signal.signal(signal.SIGINT, stop)
    version_result = worker_command([args.kicad_cli, 'version'], 5)
    version = version_result['stdout'].strip()
    if version_result['exit_code'] or not version:
        raise RuntimeError('cannot determine KiCad version')
    converter_hash = hashlib.sha256((HERE / 'convert.py').read_bytes() + Path(__file__).read_bytes()).hexdigest()
    source_rows = sources(args.data)
    outcomes_path = args.data / 'conversion-outcomes.jsonl'
    outcomes = journal(outcomes_path)
    ready_count = publish(args.data, source_rows, outcomes, converter_hash, version)
    pending = []
    for source_id, row in source_rows.items():
        try:
            fingerprint = signature(digest(relative_source(row, args.data)), converter_hash, version)
        except OSError:
            fingerprint = None
        previous = outcomes.get(source_id)
        if previous and previous.get('fingerprint') == fingerprint:
            # Failed conversions stay recorded; successful missing/tampered output
            # must be rebuilt instead of preserving stale publication eligibility.
            if previous.get('status') != 'imported_unvalidated' or eligible(row, previous, previous.get('source_sha256'), converter_hash, version):
                continue
        pending.append(row)
    pending.sort(key=lambda row: (row.get('source_format') != 'altium', row['source_id']))
    pending = pending[:args.limit]
    completed = 0
    with concurrent.futures.ThreadPoolExecutor(max_workers=args.workers) as executor:
        active = set()
        # Each conversion reserves its memory in the machine's ledger
        # (harness_common.try_admit: the rule every harness admits by).
        ledger = harness_common.Ledger.from_env()
        reservations = {}
        while (pending or active) and not STOP:
            while pending and len(active) < args.workers:
                key, _ = harness_common.try_admit(ledger, args.memory_per_worker_gib * 1024,
                                                  'import_pending: ' + pending[0]['source_id'][:80],
                                                  args.reserve_memory_gib * 1024)
                if key is None:
                    break
                future = executor.submit(convert_job, pending.pop(0), args.data, converter_hash, version, args)
                reservations[future] = key
                active.add(future)
            if not active:
                print(json.dumps({'status': 'memory_deferred', 'pending': len(pending), 'ready': ready_count}), flush=True)
                break
            done, active = concurrent.futures.wait(active, timeout=0.5, return_when=concurrent.futures.FIRST_COMPLETED)
            for future in done:
                ledger.release(reservations.pop(future, None))
                result = future.result()
                append_outcome(outcomes_path, result)
                outcomes[result['source_id']] = result
                completed += 1
                ready_count = publish(args.data, source_rows, outcomes, converter_hash, version)
                print(json.dumps({'source_id': result['source_id'], 'status': result['status'], 'ready': ready_count}), flush=True)
        # Drain interrupted jobs so their cancellation is journaled too.
        for future in concurrent.futures.as_completed(active):
            ledger.release(reservations.pop(future, None))
            result = future.result()
            append_outcome(outcomes_path, result)
            outcomes[result['source_id']] = result
    ready_count = publish(args.data, source_rows, outcomes, converter_hash, version)
    print(json.dumps({'status': 'complete', 'converted_this_batch': completed, 'ready': ready_count}), flush=True)
    return 0


if __name__ == '__main__':
    raise SystemExit(main())
