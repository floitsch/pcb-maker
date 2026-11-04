#!/usr/bin/env python3
# Copyright (C) 2026 Toit contributors.
"""Publication must never retain stale or unvalidated training eligibility."""
import importlib.util
import json
import time
from pathlib import Path
import tempfile
import os
import select
import signal
import sys
from types import SimpleNamespace
import unittest
from unittest import mock

# The tests' own memory ledger, not the machine's (harness_common.Ledger;
# their subprocesses inherit it).
os.environ.setdefault("PCB_MAKER_LEDGER", str(Path(tempfile.gettempdir()) / "pcb-maker-test-ledger.json"))

SPEC = importlib.util.spec_from_file_location('import_pending', Path(__file__).with_name('import_pending.py'))
MODULE = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(MODULE)


class PendingConversionTests(unittest.TestCase):
    def setUp(self):
        self.temporary = tempfile.TemporaryDirectory()
        self.addCleanup(self.temporary.cleanup)
        self.data = Path(self.temporary.name)
        self.source = self.data / 'source.PcbDoc'
        self.source.write_bytes(b'original')
        self.source_hash = MODULE.digest(self.source)
        self.board_dir = self.data / 'conversions' / 'one'
        self.board_dir.mkdir(parents=True)
        self.board = self.board_dir / 'board.kicad_pcb'
        self.board.write_text('(kicad_pcb)')
        self.row = {'source_id': 'owner/repo:path.PcbDoc', 'repository': 'owner/repo',
                    'path': 'path.PcbDoc', 'commit': 'abc', 'sha256': self.source_hash,
                    'source_format': 'altium', 'board_path': str(self.source),
                    'nn_license_eligible': True, 'rules_inferred': False}
        self.outcome = {'source_id': self.row['source_id'], 'status': 'imported_unvalidated',
            'source_sha256': self.source_hash, 'converted': str(self.board),
            'converted_sha256': MODULE.digest(self.board),
            'fingerprint': MODULE.signature(self.source_hash, 'converter', '10.0.7')}
        self.report = {'status': 'imported_unvalidated', 'source_sha256': self.source_hash,
            'steps': {stage: {'exit_code': 0, 'timeout': False} for stage in ('import', 'readback', 'drc')}}
        self.write_report()
        for name in ('import.json', 'stats.json', 'drc.json'):
            (self.board_dir / name).write_text('{}')

    def write_report(self):
        (self.board_dir / 'conversion.json').write_text(json.dumps(self.report))

    def publish(self, converter='converter', version='10.0.7'):
        return MODULE.publish(self.data, {self.row['source_id']: self.row},
                              {self.row['source_id']: self.outcome}, converter, version)

    def test_ready_is_experimental_and_preserves_original_metadata(self):
        self.assertEqual(self.publish(), 1)
        row = json.loads((self.data / 'converted-manifest.json').read_text())['boards'][0]
        self.assertFalse(row['nn_license_eligible'])
        self.assertFalse(row['eligible_as_routing_label'])
        self.assertTrue(row['rules_inferred'])
        self.assertEqual(row['semantic_validation'], 'unvalidated')
        self.assertEqual(row['original_source'], self.row)
        self.assertEqual(row['sha256'], MODULE.digest(self.board))
        self.assertIsNone(row['project_path'])

    def test_source_mutation_revokes_existing_manifest(self):
        self.assertEqual(self.publish(), 1)
        self.source.write_bytes(b'changed')
        self.assertEqual(self.publish(), 0)
        self.assertEqual(json.loads((self.data / 'converted-manifest.json').read_text())['boards'], [])

    def test_converter_or_kicad_version_change_revokes_manifest(self):
        self.assertEqual(self.publish(converter='changed'), 0)
        self.assertEqual(self.publish(version='11'), 0)

    def test_modified_or_missing_converted_artifact_revokes_manifest(self):
        self.board.write_text('(changed)')
        self.assertEqual(self.publish(), 0)
        self.board.unlink()
        self.assertEqual(self.publish(), 0)

    def test_failed_baseline_cannot_retain_prior_success(self):
        self.report['steps']['drc']['exit_code'] = 3
        self.write_report()
        self.assertEqual(self.publish(), 0)
        self.report['steps']['drc']['exit_code'] = 0
        self.write_report()
        (self.board_dir / 'drc.json').unlink()
        self.assertEqual(self.publish(), 0)

    def test_harvest_hash_mismatch_blocks_even_matching_old_conversion(self):
        self.row['sha256'] = 'changed-expectation'
        self.assertEqual(self.publish(), 0)

    def test_latest_failure_revokes_prior_success(self):
        self.outcome['status'] = 'wall_timeout'
        self.assertEqual(self.publish(), 0)

    def test_journal_recovers_partial_line_and_keeps_last_status(self):
        path = self.data / 'conversion-outcomes.jsonl'
        path.write_text('{"source_id":"x","status":"old"}\n{"partial":')
        MODULE.append_outcome(path, {'source_id': 'x', 'status': 'new'})
        self.assertEqual(MODULE.journal(path)['x']['status'], 'new')

    @unittest.skipUnless(hasattr(os, 'pidfd_open'), 'needs pidfd_open (Linux 5.3+)')
    def test_whole_job_timeout_kills_native_child_and_grandchild(self):
        """The whole-job timeout kills the native tree, and the job ends wall_timeout.

        The race this test used to lose under load (load 25-50, and
        reproduced every time with 32 busy loops pinned to the test's CPU):
        the job's 1 s of wall started before the worker's interpreter, which
        then runs `kicad-cli version` and `kicad-cli pcb import` (three
        Python start-ups in all) before the fake CLI spawns its grandchild
        and writes the marker. That took 0.2 s idle and 2 s under the pinned
        load, so the timeout fired first, the process group died before the
        native tree existed, and `marker.exists()` failed. The 2026-10-27
        loosening (the wall bound, the /proc poll) waited longer for a kill
        of processes that were never started. The runner was right all
        along: the worker runs in its own session and the timeout SIGKILLs
        the whole process group (os.killpg), grandchild included.

        Now the job's clock (the runner's time.monotonic) stands still until
        the fake CLI has published both PIDs (atomically), then jumps past
        the timeout: the timeout fires with both alive, every time. Both
        PIDs are held as pidfds opened while they are alive (no PID reuse),
        and the test waits for them to exit without a timing bound (300 s is
        a hang guard only). Both sleep an hour and the stage timeout is an
        hour, so only the runner's kill of the process group can end them.
        """
        marker = self.data / 'native-pids.json'
        fake = self.data / 'fake-kicad'
        fake.write_text('#!' + sys.executable + '\n' +
            'import json,os,subprocess,sys,time\n' +
            'if sys.argv[1:] == ["version"]: print("10.0.7"); sys.exit(0)\n' +
            'child = subprocess.Popen([sys.executable,"-c","import time; time.sleep(3600)"])\n' +
            'open(' + repr(str(marker) + '.tmp') + ',"w").write(json.dumps([os.getpid(),child.pid]))\n' +
            'os.replace(' + repr(str(marker) + '.tmp') + ',' + repr(str(marker)) + ')\n' +
            'time.sleep(3600)\n')
        fake.chmod(0o755)
        job_timeout = 1
        pidfds = []
        state = {'started': None}

        def monotonic():
            now = time.monotonic()
            if state['started'] is None:
                state['started'] = now
            if not pidfds and marker.exists():
                pidfds.extend(os.pidfd_open(pid) for pid in json.loads(marker.read_text()))
            # Hang guard only: without the marker the job still ends, and
            # the marker assertion below fails.
            if pidfds or now - state['started'] > 300:
                return state['started'] + job_timeout
            return state['started']

        def release():
            # A failing run must not leave the hour-long sleepers behind.
            for fd in pidfds:
                try:
                    signal.pidfd_send_signal(fd, signal.SIGKILL)
                except ProcessLookupError:
                    pass
                os.close(fd)
        self.addCleanup(release)
        clock = SimpleNamespace(monotonic=monotonic, sleep=time.sleep, time=time.time)
        args = SimpleNamespace(stage_timeout=3600, job_timeout=job_timeout, kicad_cli=str(fake))
        MODULE.STOP = False
        with mock.patch.object(MODULE, 'time', clock):
            outcome = MODULE.convert_job(self.row, self.data, 'converter', '10.0.7', args)
        self.assertEqual(outcome['status'], 'wall_timeout')
        self.assertEqual(outcome['reason'], 'whole conversion process group terminated')
        self.assertTrue(marker.exists(), 'the native tree never started')
        self.assertEqual(len(pidfds), 2)
        # A pidfd turns readable when its process has exited (zombie or
        # reaped); they would sleep an hour on their own.
        waiting, deadline = set(pidfds), time.monotonic() + 300
        while waiting and time.monotonic() < deadline:
            ready, _, _ = select.select(list(waiting), [], [], deadline - time.monotonic())
            waiting.difference_update(ready)
        self.assertFalse(waiting, 'the native child or grandchild outlived the job timeout')

    def test_manual_sources_include_pinned_original_without_native_overwrite(self):
        p = self.data / 'non-kicad' / 'example'
        p.mkdir(parents=True)
        (p / 'provenance.json').write_text(json.dumps({'repository': 'owner/repo',
                'path': 'native.PcbDoc', 'commit': 'pinned'}))
        (p / 'source.PcbDoc').write_bytes(b'original')
        records = MODULE.sources(self.data)
        self.assertEqual(records['owner/repo:native.PcbDoc']['source_format'], 'altium')
        self.assertEqual(records['owner/repo:native.PcbDoc']['commit'], 'pinned')


if __name__ == '__main__':
    unittest.main()
