#!/usr/bin/env python3
# Copyright (C) 2026 Toit contributors.
"""Continuously classify harvested boards with bounded, single-thread pcb-maker runs.

Input: atomic JSON {boards:[{name,board_id,directory,sha256,...}]} where directory
is repository-relative. JSONL downloaded records with board_path relative to the
manifest directory are also accepted. Original sources are never modified.
"""

import argparse
from collections import Counter
from concurrent.futures import ThreadPoolExecutor, wait, FIRST_COMPLETED
import fcntl
import hashlib
import json
import os
from pathlib import Path
import re
import shutil
import signal
import subprocess
import threading
import time

ROOT = Path(__file__).resolve().parents[2]
GIB = 1024 ** 3
COSMETIC = re.compile(r"^(silk_|text_|lib_footprint|footprint_type_mismatch|missing_courtyard|isolated_copper|nonmirrored_text)")
STOP = threading.Event()


def digest(path):
    h = hashlib.sha256()
    with Path(path).open('rb') as stream:
        for chunk in iter(lambda: stream.read(1024 * 1024), b''):
            h.update(chunk)
    return h.hexdigest()


def atomic_json(path, value):
    path = Path(path)
    temporary = path.with_name(path.name + '.tmp')
    with temporary.open('w') as stream:
        json.dump(value, stream, sort_keys=True, indent=2)
        stream.write('\n')
        stream.flush()
        os.fsync(stream.fileno())
    os.replace(temporary, path)


def read_json(path):
    try:
        return json.loads(Path(path).read_text())
    except (OSError, ValueError):
        return None


def read_boards(manifest):
    if manifest.suffix == '.jsonl':
        latest = {}
        for line in manifest.read_text().splitlines():
            try:
                row = json.loads(line)
            except ValueError:  # A harvester may still be appending its final line.
                continue
            latest[row['source_id']] = row
        boards = [row for row in latest.values() if row.get('status') == 'downloaded']
    else:
        boards = json.loads(manifest.read_text())['boards']
    answer = []
    for original in boards:
        row = dict(original)
        if row.get('directory'):
            directory = Path(row['directory'])
            if not directory.is_absolute():
                directory = ROOT / directory
            board = directory / (row['board_id'] + '.kicad_pcb')
        else:
            board = Path(row['board_path'])
            if not board.is_absolute():
                board = manifest.parent / board
            row['board_id'] = board.stem
        row['_board'] = str(board.resolve())
        if not row.get('name'):
            row['name'] = hashlib.sha256(row['source_id'].encode()).hexdigest()[:20]
        answer.append(row)
    return answer


def process_rss(pid):
    try:
        for line in Path(f'/proc/{pid}/status').read_text().splitlines():
            if line.startswith('VmRSS:'):
                return int(line.split()[1]) * 1024
    except OSError:
        pass
    return 0


def terminate_group(process):
    try:
        os.killpg(process.pid, signal.SIGKILL)
    except ProcessLookupError:
        pass


def run_process(command, log, timeout, max_rss_gib=3, env=None, cwd=None):
    """Kill the whole session on timeout/cancel; wait4 gives per-job child CPU/RSS.

    Output goes to disk, so a noisy child cannot grow the orchestrator's memory.
    Each session is owned by exactly one thread; no subprocess wait() races wait4.
    """
    started = time.monotonic()
    reason = None
    with Path(log).open('wb') as stream:
        try:
            lowered = list(map(str, command))
            if shutil.which('nice'):
                lowered = ['nice', '-n', '19', *lowered]
            if max_rss_gib and shutil.which('prlimit'):
                lowered = ['prlimit', '--as=' + str(int(max_rss_gib * 4 * GIB)), '--', *lowered]
            process = subprocess.Popen(lowered, stdout=stream,
                                       stderr=subprocess.STDOUT, start_new_session=True,
                                       cwd=cwd, env=env)
        except OSError as error:
            return {'exit_code': None, 'stop_reason': 'spawn_error', 'error': str(error),
                    'wall_seconds': time.monotonic() - started}
        peak = 0
        while True:
            peak = max(peak, process_rss(process.pid))
            pid, status, usage = os.wait4(process.pid, os.WNOHANG)
            if pid:
                break
            if STOP.is_set():
                reason = 'cancelled'
            elif max_rss_gib and peak > max_rss_gib * GIB:
                reason = 'rss_limit'
            elif time.monotonic() - started >= timeout:
                reason = 'wall_timeout'
            if reason:
                terminate_group(process)
                _, status, usage = os.wait4(process.pid, 0)
                break
            time.sleep(0.05)
        process.returncode = os.waitstatus_to_exitcode(status)
        # A CLI can exit while leaving a native child: clean its session too.
        terminate_group(process)
    if reason is None and process.returncode:
        with Path(log).open('rb') as stream:
            stream.seek(max(0, stream.seek(0, 2) - 4096))
            tail = stream.read().decode(errors='replace').lower()
        if any(marker in tail for marker in ('memory allocation', 'cannot allocate memory', 'bad_alloc', 'out of memory')):
            reason = 'allocation_limit'
    return {'exit_code': process.returncode, 'stop_reason': reason,
            'wall_seconds': round(time.monotonic() - started, 4),
            'cpu_seconds': round(usage.ru_utime + usage.ru_stime, 4),
            'peak_rss_mb': round(max(peak / 1024 ** 2, usage.ru_maxrss / 1024), 2)}


def drc_summary(report):
    if not isinstance(report, dict) or 'violations' not in report or 'unconnected_items' not in report:
        return None
    errors = Counter(v.get('type', 'unknown') for v in report['violations']
                     if v.get('severity', 'error') == 'error' and not v.get('excluded')
                     and not COSMETIC.match(v.get('type', '')))
    return {'unconnected': len(report['unconnected_items']), 'errors': dict(errors),
            'clean': not errors and not report['unconnected_items']}


def infer_rules(text, project):
    """Freeze missing-class fallback BEFORE removing the designer's copper.

    pcb-maker otherwise infers width/via sizes from its stripped input, losing the
    reference's geometry. These are explicitly inferred rules, never original rules.
    """
    def common(pattern, fallback):
        counts = Counter(float(n) for n in re.findall(pattern, text, re.S))
        return counts.most_common(1)[0][0] if counts else fallback
    minimums = project.get('board', {}).get('design_settings', {}).get('rules', {})
    drill = max(common(r'\(via\b.*?\(drill\s+([\d.]+)', .3), minimums.get('min_through_hole_diameter', 0))
    return {'trace_width_mm': max(common(r'\(segment\b.*?\(width\s+([\d.]+)', .25), minimums.get('min_track_width', 0)),
            'clearance_mm': max(.2, minimums.get('min_clearance', 0)),
            'via_size_mm': max(common(r'\(via\b.*?\(size\s+([\d.]+)', .6), drill + .2, minimums.get('min_via_diameter', 0)),
            'via_drill_mm': drill}


def classify(row, simple_seconds, budget):
    stage = row.get('route_process', {})
    result = row.get('route')
    if stage.get('stop_reason') in ('rss_limit', 'allocation_limit'):
        return 'resource_limit'
    if stage.get('stop_reason') in ('wall_timeout', 'cancelled'):
        return 'unresolved_timeout' if stage['stop_reason'] == 'wall_timeout' else 'cancelled'
    if not result:
        if stage.get('exit_code') is not None and stage['exit_code'] < 0:
            return 'process_signal'
        return 'import_error' if stage.get('exit_code') == 2 else 'model_error'
    native = row.get('native_result', {}).get('summary')
    complete = result.get('unconnected_terminals', 1) == 0 and result.get('routed_connections') == result.get('routable_connections')
    internal = result.get('internal_violations', [])
    seconds = result.get('routing_seconds', stage.get('wall_seconds', 0))
    if complete and not internal:
        if native is None:
            return 'verification_unknown'
        if not native['clean']:
            return 'native_drc_failure'
        if seconds <= simple_seconds:
            return 'clean_fast'
        return 'clean_medium' if seconds <= budget else 'clean_slow'
    if result.get('cut'):
        return 'cut_wall_clock'
    if result.get('work_seconds', 0) >= .95 * budget or seconds >= .95 * budget:
        return 'incomplete_at_budget'
    return 'internal_drc_failure' if internal else 'incomplete_early'


def native_check(directory, board_id, work, label, args, semaphore, env):
    if not args.kicad_cli:
        return {'summary': None, 'reason': 'kicad-cli unavailable'}
    with semaphore:
        # Each DRC may use multiple GiB. Wait without holding a CPU process.
        while not STOP.is_set() and memory_available() < args.reserve_gib * GIB + 2 * GIB:
            STOP.wait(1)
        if STOP.is_set():
            return {'summary': None, 'reason': 'cancelled'}
        report = work / (label + '-drc.json')
        native_command = [args.kicad_cli, 'pcb', 'drc', '--refill-zones',
                               '--severity-error', '--format', 'json', '-o', report,
                               directory / (board_id + '.kicad_pcb')]
        if getattr(args, 'native_cores', None):
            native_command = ['taskset', '-c', args.native_cores, *native_command]
        process = run_process(native_command, work / (label + '-drc.log'), args.native_timeout,
                              args.max_rss_gib, env, cwd=directory)
        return {'process': process, 'summary': drc_summary(read_json(report)) if process.get('exit_code') == 0 and not process.get('stop_reason') else None, 'report': str(report)}


def evaluate(board, fingerprint, args, binary, config, native_semaphore):
    safe = re.sub(r'[^A-Za-z0-9_.-]', '_', board['name'])[:120]
    work = args.output / 'boards' / (safe + '-' + fingerprint[:16])
    if work.exists():
        shutil.rmtree(work)  # Incomplete runs only; complete result.json is skipped upstream.
    work.mkdir(parents=True)
    original = Path(board['_board'])
    board_id = board['board_id']
    row = {'name': board['name'], 'fingerprint': fingerprint, 'provenance': {k: v for k, v in board.items() if not k.startswith('_')},
           'started_at': time.strftime('%Y-%m-%dT%H:%M:%SZ', time.gmtime()), 'work_directory': str(work)}
    start = time.monotonic()
    temporary_directory = work / 'tmp'
    temporary_directory.mkdir()
    env = dict(os.environ, TMPDIR=str(temporary_directory), RAYON_NUM_THREADS='1', OMP_NUM_THREADS='1', OPENBLAS_NUM_THREADS='1')
    for key in list(env):
        if key.startswith('PCB_'):
            del env[key]  # Experiment hooks cannot silently contaminate a run.
    try:
        if not original.is_file():
            raise ValueError('source board is missing')
        actual_sha = digest(original)
        row['source_sha256'] = actual_sha
        if board.get('sha256') and actual_sha != board['sha256']:
            raise ValueError('source SHA-256 differs from manifest')
        reference, source = work / 'reference', work / 'source'
        reference.mkdir()
        source.mkdir()
        shutil.copy2(original, reference / (board_id + '.kicad_pcb'))
        # Copy only project rule files, never arbitrary external trees or symlinks.
        project = {}
        for extension in ('.kicad_pro', '.kicad_dru'):
            path = original.with_suffix(extension)
            if path.is_file():
                shutil.copy2(path, source / (board_id + extension))
                shutil.copy2(path, reference / (board_id + extension))
                if extension == '.kicad_pro':
                    project = json.loads(path.read_text())
        local_config = dict(config)
        row['rules_inferred'] = bool(board.get('rules_inferred')) or not project.get('net_settings', {}).get('classes')
        if not project.get('net_settings', {}).get('classes'):
            rules = infer_rules(original.read_text(), project)
            row['inferred_default_rules'] = rules
            # Keep project-wide minimums, edge/hole rules and severities in the
            # normal resolver. A default_rules config would bypass those.
            project.setdefault('net_settings', {})['classes'] = [{
                'name': 'Default', 'track_width': rules['trace_width_mm'],
                'clearance': rules['clearance_mm'], 'via_diameter': rules['via_size_mm'],
                'via_drill': rules['via_drill_mm']}]
            atomic_json(source / (board_id + '.kicad_pro'), project)
        atomic_json(work / 'router-config.json', local_config)
        row['strip_process'] = run_process([binary, 'strip-kicad-tracks', original, source / (board_id + '.kicad_pcb')],
                                          work / 'strip.log', args.setup_timeout, args.max_rss_gib, env)
        if row['strip_process']['exit_code'] != 0:
            reason = row['strip_process'].get('stop_reason')
            row['category'] = 'resource_limit' if reason in ('rss_limit', 'allocation_limit') else ('setup_timeout' if reason == 'wall_timeout' else 'source_issue')
        else:
            row['stripped_sha256'] = digest(source / (board_id + '.kicad_pcb'))
            routed = work / 'routed'
            row['route_process'] = run_process([binary, 'route-kicad-board', source, board_id, routed,
                                                work / 'router-config.json', '--reference', 'none'],
                                               work / 'route.log', args.route_timeout, args.max_rss_gib, env)
            report = read_json(routed / 'board-router.json')
            row['route'] = report
            row['verdict'] = read_json(routed / 'verdict.json')
            if report:
                # Preserve dense per-net routing data in its original report, keep JSONL modest.
                row['route_report'] = str(routed / 'board-router.json')
                row['route'] = {k: v for k, v in report.items() if k not in ('nets', 'diagnostics')}
                if report.get('unconnected_terminals') == 0 and not report.get('internal_violations'):
                    row['native_result'] = native_check(routed, board_id, work, 'result', args, native_semaphore, env)
                row['native_source'] = native_check(reference, board_id, work, 'source', args, native_semaphore, env)
            row['category'] = classify(row, args.simple_seconds, args.route_seconds)
        source_native = row.get('native_source', {}).get('summary')
        row['source_quality'] = 'native_clean' if source_native and source_native['clean'] else ('native_findings' if source_native else 'unknown')
        row['benchmark_candidate'] = row['category'] in ('incomplete_at_budget', 'clean_slow') and row['source_quality'] == 'native_clean' and not row['rules_inferred']
        row['benchmark_review_candidate'] = row['category'] in ('incomplete_at_budget', 'cut_wall_clock', 'clean_slow', 'incomplete_early')
        row['training_candidate'] = row['category'].startswith('clean_') and not row['rules_inferred']
        row['nn_license_eligible'] = board.get('nn_license_eligible', False)
    except Exception as error:
        row.update(category='source_issue', error=f'{type(error).__name__}: {error}')
    row['elapsed_seconds'] = round(time.monotonic() - start, 3)
    row['finished_at'] = time.strftime('%Y-%m-%dT%H:%M:%SZ', time.gmtime())
    atomic_json(work / 'result.json', row)
    return row


def memory_available():
    for line in Path('/proc/meminfo').read_text().splitlines():
        if line.startswith('MemAvailable:'):
            return int(line.split()[1]) * 1024
    return 0


def pid_alive(pid):
    try:
        os.kill(pid, 0)
        return True
    except ProcessLookupError:
        return False
    except PermissionError:
        return True


def shared_memory_gate(args, active=None, admit=False, remove=False):
    """Update/reserve estimated memory across runners under one persistent flock.

    This file must not be replaced: readers lock the same inode. An admission
    reserves its additional slot before the worker starts, closing the race
    between independent orchestrators observing the same available memory.
    """
    path = getattr(args, 'memory_gate', None)
    if not path:
        return None
    path = Path(path)
    path.parent.mkdir(parents=True, exist_ok=True)
    with path.open('a+') as stream:
        fcntl.flock(stream, fcntl.LOCK_EX)
        stream.seek(0)
        try:
            entries = json.load(stream)
        except ValueError:
            entries = {}
        entries = {pid: row for pid, row in entries.items() if pid.isdigit() and pid_alive(int(pid))}
        own = str(os.getpid())
        if remove:
            entries.pop(own, None)
        elif active is not None:
            entries[own] = {'active': active, 'estimate_gib': active * args.job_memory_gib,
                            'updated_at': time.time()}
        reason = None
        if admit:
            estimates = sum(row.get('estimate_gib', 0) for row in entries.values())
            if memory_available() < (args.reserve_gib + estimates + args.job_memory_gib) * GIB:
                reason = 'shared_memory_admission_estimate'
            else:
                entries[own]['active'] += 1
                entries[own]['estimate_gib'] += args.job_memory_gib
        stream.seek(0)
        stream.truncate()
        json.dump(entries, stream, sort_keys=True)
        stream.write('\n')
        stream.flush()
        os.fsync(stream.fileno())
        return reason


def admission_gate(args, active):
    if shutil.disk_usage(args.output).free < args.min_disk_gib * GIB:
        return 'disk_reserve'
    if memory_available() < (args.reserve_gib + args.job_memory_gib) * GIB:
        return 'memory_reserve'
    # Slow starting jobs can all be admitted before their RSS grows. Reserve an
    # estimate for each active job on top of currently available memory.
    if not getattr(args, 'memory_gate', None) and memory_available() < (args.reserve_gib + (active + 1) * args.job_memory_gib) * GIB:
        return 'memory_admission_estimate'
    if os.getloadavg()[0] > args.max_load:
        return 'load_limit'
    return shared_memory_gate(args, active=active, admit=True)


def source_fingerprint(board, run_key):
    original = Path(board['_board'])
    # Include actual content and rules, so stale manifests cannot reuse results.
    sources = {extension: digest(original.with_suffix(extension))
               for extension in ('.kicad_pcb', '.kicad_pro', '.kicad_dru')
               if original.with_suffix(extension).is_file()}
    return hashlib.sha256(json.dumps({'source': board, 'files': sources, 'run': run_key}, sort_keys=True).encode()).hexdigest()


def repair_journal(path):
    if not path.exists():
        return
    with path.open('rb+') as stream:
        valid_end = 0
        for line in stream:
            try:
                json.loads(line)
            except ValueError:
                break
            if not line.endswith(b'\n'):
                break
            valid_end = stream.tell()
        stream.truncate(valid_end)


def main(argv=None):
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--manifest', type=Path, required=True)
    parser.add_argument('--output', type=Path, required=True)
    parser.add_argument('--binary', type=Path, default=ROOT / 'target/release/pcb-maker')
    parser.add_argument('--workers', type=int, default=max(1, min(28, (os.cpu_count() or 1) - 3)))
    parser.add_argument('--native-workers', type=int, default=2)
    parser.add_argument('--native-cores', help='optional taskset CPU list for native KiCad only, e.g. 28-31')
    parser.add_argument('--memory-gate', type=Path, help='optional shared admission JSON/lock across runners')
    parser.add_argument('--route-seconds', type=float, default=30)
    parser.add_argument('--simple-seconds', type=float, default=3)
    parser.add_argument('--deadline-seconds', type=float, default=90,
                        help='wall deadline inside routing; a cut is distinct from exhausted work')
    parser.add_argument('--route-timeout', type=float, default=120,
                        help='hard whole-process wall timeout, including import/export')
    parser.add_argument('--setup-timeout', type=float, default=60)
    parser.add_argument('--native-timeout', type=float, default=120)
    parser.add_argument('--reserve-gib', type=float, default=10)
    parser.add_argument('--job-memory-gib', type=float, default=1)
    parser.add_argument('--max-rss-gib', type=float, default=3)
    parser.add_argument('--min-disk-gib', type=float, default=10)
    parser.add_argument('--max-load', type=float, default=(os.cpu_count() or 1) * 1.5)
    parser.add_argument('--kicad-cli', default=shutil.which('kicad-cli'))
    parser.add_argument('--watch', action='store_true')
    parser.add_argument('--idle-exit-seconds', type=float, default=0)
    parser.add_argument('--only', nargs='*', help='manifest board names to include')
    args = parser.parse_args(argv)
    for field in ('workers', 'native_workers', 'route_seconds', 'deadline_seconds', 'route_timeout', 'setup_timeout', 'native_timeout', 'job_memory_gib', 'max_rss_gib'):
        if getattr(args, field) <= 0:
            parser.error('--' + field.replace('_', '-') + ' must be positive')
    args.output = args.output.resolve()
    args.manifest = args.manifest.resolve()
    if args.memory_gate:
        args.memory_gate = args.memory_gate.resolve()
    if args.native_cores:
        if not shutil.which('taskset'):
            parser.error('--native-cores requires taskset')
        try:
            selected = set()
            for part in args.native_cores.split(','):
                ends = list(map(int, part.split('-')))
                if len(ends) not in (1, 2) or min(ends) < 0 or ends[-1] < ends[0]:
                    raise ValueError()
                selected.update(range(ends[0], ends[-1] + 1))
            if not selected or not selected.issubset(os.sched_getaffinity(0)):
                raise ValueError()
        except (ValueError, IndexError):
            parser.error('--native-cores must select available logical CPUs')
    args.output.mkdir(parents=True, exist_ok=True)
    lock = (args.output / 'runner.lock').open('w')
    try:
        fcntl.flock(lock, fcntl.LOCK_EX | fcntl.LOCK_NB)
    except BlockingIOError:
        parser.error('another triage runner owns this output directory')
    config = {'ladder_budget_seconds': args.route_seconds, 'deadline_seconds': args.deadline_seconds,
              'skip_native_verification': True, 'seeds': 1}
    binary_sha = digest(args.binary)
    pinned = args.output / ('pcb-maker-' + binary_sha[:16])
    if not pinned.exists():
        temporary = pinned.with_suffix('.tmp')
        shutil.copy2(args.binary, temporary)
        if digest(temporary) != binary_sha:
            temporary.unlink()
            parser.error('binary changed while snapshotting; retry')
        temporary.chmod(0o755)
        temporary.replace(pinned)
    if digest(pinned) != binary_sha:
        parser.error('pinned binary has changed')
    metadata = {'binary_sha256': binary_sha, 'binary_original': str(args.binary.resolve()),
                'runner_sha256': digest(__file__), 'config': config, 'simple_seconds': args.simple_seconds,
                'route_timeout': args.route_timeout, 'setup_timeout': args.setup_timeout,
                'native_timeout': args.native_timeout, 'max_rss_gib': args.max_rss_gib,
                'kicad_cli': args.kicad_cli, 'kicad_cli_sha256': digest(args.kicad_cli) if args.kicad_cli and Path(args.kicad_cli).is_file() else None, 'native_workers': args.native_workers,
                'process_niceness': 19, 'address_space_limit_gib': args.max_rss_gib * 4,
                'strip_contract': 'pcb-maker strip-kicad-tracks: remove segments/arcs/vias, cached pour fills, and copper zones smaller than 10% board area; retain large pour declarations and rule areas',
                'qualification': 'Native DRC source and completed output use original project rules, no schematic ERC; inferred rules are flagged. Cuts/timeouts are not proof of difficult routing.',
                'environment': {'RAYON_NUM_THREADS': '1', 'OMP_NUM_THREADS': '1', 'OPENBLAS_NUM_THREADS': '1', 'PCB_*': 'cleared'}}
    run_key = hashlib.sha256(json.dumps(metadata, sort_keys=True).encode()).hexdigest()
    metadata.update(run_key=run_key, manifest=str(args.manifest), workers=args.workers,
                    reserve_gib=args.reserve_gib, max_load=args.max_load,
                    memory_gate=str(args.memory_gate) if args.memory_gate else None, native_cores=args.native_cores)
    atomic_json(args.output / 'run.json', metadata)
    completed = {}
    for path in (args.output / 'boards').glob('*/result.json'):
        row = read_json(path)
        if row and row.get('category') != 'cancelled':
            completed[row['fingerprint']] = row
    native_semaphore = threading.Semaphore(args.native_workers)
    started = time.monotonic()
    idle_since = None
    last_status = 0
    active = {}
    active_keys = set()
    fingerprint_cache = {}
    signal.signal(signal.SIGINT, lambda *_: STOP.set())
    signal.signal(signal.SIGTERM, lambda *_: STOP.set())
    repair_journal(args.output / 'results.jsonl')
    journal = (args.output / 'results.jsonl').open('a')
    try:
        with ThreadPoolExecutor(max_workers=args.workers) as pool:
            while not STOP.is_set() or active:
                shared_memory_gate(args, active=len(active))
                boards = read_boards(args.manifest) if args.manifest.exists() else []
                if args.only:
                    boards = [b for b in boards if b['name'] in args.only]
                # Hash board content once per observed manifest/source mtime tuple.
                candidates = []
                for board in boards:
                    original = Path(board['_board'])
                    stats = [(str(p), p.stat().st_mtime_ns, p.stat().st_size)
                             for p in (original.with_suffix(e) for e in ('.kicad_pcb', '.kicad_pro', '.kicad_dru')) if p.is_file()]
                    cache_key = json.dumps([board, stats], sort_keys=True)
                    if cache_key not in fingerprint_cache:
                        fingerprint_cache[cache_key] = source_fingerprint(board, run_key)
                    fingerprint = fingerprint_cache[cache_key]
                    candidates.append((board, fingerprint))
                pending = [(b, f) for b, f in candidates if f not in completed and f not in active_keys]
                gate = None
                while pending and len(active) < args.workers and not STOP.is_set():
                    gate = admission_gate(args, len(active))
                    if gate:
                        break
                    board, fingerprint = pending.pop(0)
                    future = pool.submit(evaluate, board, fingerprint, args, pinned, config, native_semaphore)
                    active[future] = fingerprint
                    active_keys.add(fingerprint)
                done = wait(active, timeout=.2, return_when=FIRST_COMPLETED).done if active else set()
                for future in done:
                    fingerprint = active.pop(future)
                    active_keys.remove(fingerprint)
                    row = future.result()
                    completed[fingerprint] = row
                    shared_memory_gate(args, active=len(active))
                    journal.write(json.dumps(row, sort_keys=True) + '\n')
                    journal.flush()
                    os.fsync(journal.fileno())
                    print(json.dumps({'event': 'result', 'name': row['name'], 'category': row['category'],
                                      'seconds': row['elapsed_seconds'], 'benchmark_candidate': row.get('benchmark_candidate', False)}), flush=True)
                current = [completed[f] for _, f in candidates if f in completed]
                now = time.monotonic()
                if now - last_status > 10 or (done and not active and not pending):
                    status = {'completed': len(current), 'total': len(candidates), 'active': len(active),
                              'pending': len(pending), 'categories': dict(Counter(r['category'] for r in current)),
                              'benchmark_candidates': sum(r.get('benchmark_candidate', False) for r in current),
                              'training_candidates': sum(r.get('training_candidate', False) for r in current),
                              'gate': gate, 'memory_available_gib': round(memory_available() / GIB, 2),
                              'elapsed_seconds': round(now - started, 1), 'stopping': STOP.is_set(),
                              'updated_at': time.strftime('%Y-%m-%dT%H:%M:%SZ', time.gmtime())}
                    atomic_json(args.output / 'status.json', status)
                    print(json.dumps({'event': 'status', **status}), flush=True)
                    last_status = now
                if not active and not pending:
                    if not args.watch or STOP.is_set():
                        break
                    if idle_since is None:
                        idle_since = now
                    if args.idle_exit_seconds and now - idle_since >= args.idle_exit_seconds:
                        break
                else:
                    idle_since = None
                if not active:
                    STOP.wait(2)
    finally:
        shared_memory_gate(args, remove=True)
        journal.close()
        lock.close()
    return 0


if __name__ == '__main__':
    raise SystemExit(main())
