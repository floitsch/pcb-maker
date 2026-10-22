#!/usr/bin/env python3
# Copyright (C) 2026 Toit contributors.
"""Tests for durable classification, bounded subprocesses, and content-based resume."""
import importlib.util
import json
import os
from pathlib import Path
import subprocess
import sys
import tempfile
import time
import unittest
import unittest.mock

# The tests' own memory ledger, not the machine's (harness_common.Ledger;
# their subprocesses inherit it).
os.environ.setdefault("PCB_MAKER_LEDGER", str(Path(tempfile.gettempdir()) / "pcb-maker-test-ledger.json"))

spec = importlib.util.spec_from_file_location('triage', Path(__file__).with_name('triage.py'))
triage = importlib.util.module_from_spec(spec)
spec.loader.exec_module(triage)


class TriageTests(unittest.TestCase):
    def setUp(self):
        self.temp = tempfile.TemporaryDirectory()
        self.root = Path(self.temp.name)
        triage.STOP.clear()

    def tearDown(self):
        self.temp.cleanup()

    def executable(self, name, code):
        path = self.root / name
        path.write_text('#!' + sys.executable + '\n' + code)
        path.chmod(0o755)
        return path

    def test_timeout_kills_descendants_and_keeps_cpu_measurement(self):
        # Deterministic under load: the timeout fires on a clock of the
        # test's own, which passes the limit only once the descendant has
        # been started and named (the pid file is renamed into place whole;
        # a real 0.4 s could pass before the interpreters under nice had
        # started it), and a descendant reaped between the look into /proc
        # and the read (ESRCH, the failure seen one run in three) counts
        # as gone.
        pidfile = self.root / 'child.pid'
        code = ("import os,subprocess,time,sys\n"
                "p=subprocess.Popen([sys.executable,'-c','import time;time.sleep(600)'])\n"
                "open(sys.argv[1]+'.tmp','w').write(str(p.pid))\n"
                "os.rename(sys.argv[1]+'.tmp',sys.argv[1])\n"
                "time.sleep(600)\n")
        command = self.executable('parent', code)

        class Clock:
            """0 until the descendant is named, then far past the timeout."""
            sleep = staticmethod(time.sleep)

            @staticmethod
            def monotonic():
                return 1000.0 if pidfile.exists() else 0.0

        started = time.monotonic()
        with unittest.mock.patch.object(triage, 'time', Clock):
            result = triage.run_process([command, pidfile], self.root / 'timeout.log', 1)
        self.assertEqual(result['stop_reason'], 'wall_timeout')
        self.assertIn('cpu_seconds', result)
        # Both processes sleep 600 s: well under that, the kill ended them.
        self.assertLess(time.monotonic() - started, 300)
        child = int(pidfile.read_text())
        deadline = time.monotonic() + 300
        while time.monotonic() < deadline:
            try:
                state = Path(f'/proc/{child}/stat').read_text().rsplit(')', 1)[1].split()[0]
            except (FileNotFoundError, ProcessLookupError):
                # Gone, or reaped between the lookup and the read (ESRCH).
                break
            if state == 'Z':
                break
            time.sleep(.025)
        else:
            self.fail('subprocess descendant survived the timeout')

    def test_classification_requires_native_and_distinguishes_cut_from_work(self):
        complete = {'unconnected_terminals': 0, 'routable_connections': 4,
                    'routed_connections': 4, 'routing_seconds': 1, 'internal_violations': []}
        row = {'route': complete, 'route_process': {'exit_code': 0}}
        self.assertEqual(triage.classify(row, 3, 30), 'verification_unknown')
        row['native_result'] = {'summary': {'clean': True}}
        self.assertEqual(triage.classify(row, 3, 30), 'clean_fast')
        row['route'] = dict(complete, unconnected_terminals=3, work_seconds=30, routing_seconds=31)
        self.assertEqual(triage.classify(row, 3, 30), 'incomplete_at_budget')
        row['route']['cut'] = ['deadline']
        self.assertEqual(triage.classify(row, 3, 30), 'cut_wall_clock')
        row['route_process']['stop_reason'] = 'wall_timeout'
        self.assertEqual(triage.classify(row, 3, 30), 'unresolved_timeout')

    def test_native_errors_ignore_only_cosmetics_warnings_and_exclusions(self):
        self.assertIsNone(triage.drc_summary({}))
        summary = triage.drc_summary({'unconnected_items': [], 'violations': [
            {'type': 'clearance', 'severity': 'error'},
            {'type': 'clearance', 'severity': 'warning'},
            {'type': 'shorting_items', 'excluded': True},
            {'type': 'silk_overlap', 'severity': 'error'}]})
        self.assertEqual(summary['errors'], {'clearance': 1})
        self.assertFalse(summary['clean'])

    def test_failed_native_process_cannot_supply_a_clean_label(self):
        from types import SimpleNamespace
        import threading
        native = self.executable('failed-native', "import pathlib,json,sys\npathlib.Path(sys.argv[sys.argv.index('-o')+1]).write_text(json.dumps(dict(unconnected_items=[],violations=[])))\nraise SystemExit(7)\n")
        args = SimpleNamespace(kicad_cli=str(native), reserve_gib=0, native_timeout=30, max_rss_gib=3)
        result = triage.native_check(self.root, 'board', self.root, 'failed', args, threading.Semaphore(1), dict(os.environ))
        self.assertEqual(result['process']['exit_code'], 7)
        self.assertIsNone(result['summary'])

    @unittest.skipUnless(__import__('shutil').which('taskset'), 'taskset unavailable')
    def test_native_cpu_affinity_is_applied(self):
        from types import SimpleNamespace
        import threading
        native = self.executable('affinity-native', "import pathlib,json,sys,os\npathlib.Path(sys.argv[sys.argv.index('-o')+1]).write_text(json.dumps(dict(unconnected_items=[],violations=[],affinity=sorted(os.sched_getaffinity(0)))))\n")
        cpu = min(os.sched_getaffinity(0))
        args = SimpleNamespace(kicad_cli=str(native), reserve_gib=0, native_timeout=30,
                               max_rss_gib=3, native_cores=str(cpu))
        result = triage.native_check(self.root, 'board', self.root, 'affinity', args,
                                     threading.Semaphore(1), dict(os.environ))
        self.assertTrue(result['summary']['clean'])
        self.assertEqual(json.loads(Path(result['report']).read_text())['affinity'], [cpu])

    def test_inferred_geometry_is_frozen_before_stripping(self):
        rules = triage.infer_rules('(segment (width 0.127)) (segment (width 0.127)) (via (size .5) (drill .2))', {})
        self.assertEqual(rules['trace_width_mm'], .127)
        self.assertEqual(rules['via_size_mm'], .5)
        self.assertEqual(rules['via_drill_mm'], .2)

    def test_fingerprints_cover_actual_board_and_project_content(self):
        path = self.root / 'board.kicad_pcb'
        path.write_text('one')
        board = {'_board': str(path), 'name': 'a'}
        first = triage.source_fingerprint(board, 'binary_config')
        path.with_suffix('.kicad_pro').write_text('{}')
        second = triage.source_fingerprint(board, 'binary_config')
        self.assertNotEqual(first, second)
        path.write_text('two')
        self.assertNotEqual(second, triage.source_fingerprint(board, 'binary_config'))

    def test_shared_gate_reserves_between_processes_and_removes_dead_owners(self):
        from types import SimpleNamespace
        import multiprocessing
        from unittest.mock import patch
        context = multiprocessing.get_context('fork')
        ready, release, results = context.Event(), context.Event(), context.Queue()
        gate = self.root / 'shared-memory.json'
        args = SimpleNamespace(memory_gate=gate, job_memory_gib=3, reserve_gib=1)

        def reserve_elsewhere():
            triage.memory_available = lambda: 5 * triage.GIB
            results.put(triage.shared_memory_gate(args, active=0, admit=True))
            ready.set()
            release.wait(10)

        process = context.Process(target=reserve_elsewhere)
        process.start()
        try:
            self.assertTrue(ready.wait(3))
            self.assertIsNone(results.get(timeout=1))
            with patch.object(triage, 'memory_available', return_value=5 * triage.GIB):
                self.assertEqual(triage.shared_memory_gate(args, active=0, admit=True),
                                 'shared_memory_admission_estimate')
            release.set()
            process.join(3)
            self.assertFalse(process.is_alive())
            with patch.object(triage, 'memory_available', return_value=5 * triage.GIB):
                self.assertIsNone(triage.shared_memory_gate(args, active=0, admit=True))
            # The machine ledger's format (harness_common.Ledger): the dead
            # runner's entry is gone, this one's holds one job.
            state = json.loads(gate.read_text())['reservations']
            self.assertNotIn(process.pid, [entry['pid'] for entry in state])
            mine = [entry for entry in state if entry['pid'] == os.getpid()]
            self.assertEqual([(entry['megabytes'], entry['label']) for entry in mine], [(3 * 1024, 'triage: 1 jobs')])
            triage.shared_memory_gate(args, remove=True)
            self.assertEqual(json.loads(gate.read_text())['reservations'], [])
        finally:
            release.set()
            if process.is_alive():
                process.terminate()
            process.join(3)

    def test_journal_recovers_partial_tail(self):
        path = self.root / 'results.jsonl'
        path.write_bytes(b'{"one":1}\n{"two":')
        triage.repair_journal(path)
        self.assertEqual(path.read_bytes(), b'{"one":1}\n')

    def test_end_to_end_native_classification_and_resume(self):
        binary = self.executable('pcb-maker', '''import sys,json,pathlib,shutil
args=sys.argv[1:]
if args[0]=='strip-kicad-tracks':
 shutil.copyfile(args[1],args[2])
elif args[0]=='route-kicad-board':
 src,board,out=pathlib.Path(args[1]),args[2],pathlib.Path(args[3])
 out.mkdir()
 shutil.copyfile(src/(board+'.kicad_pcb'),out/(board+'.kicad_pcb'))
 (out/'board-router.json').write_text(json.dumps(dict(unconnected_terminals=0,routable_connections=4,routed_connections=4,routing_seconds=.1,work_seconds=.1,internal_violations=[])))
else:
 raise SystemExit(2)
''')
        native = self.executable('kicad-cli', '''import pathlib,json,sys
pathlib.Path(sys.argv[sys.argv.index('-o')+1]).write_text(json.dumps(dict(unconnected_items=[],violations=[])))
''')
        source = self.root / 'source'
        source.mkdir()
        (source / 'board.kicad_pcb').write_text('(kicad_pcb)')
        (source / 'board.kicad_pro').write_text(json.dumps({'net_settings': {'classes': [{'name': 'Default'}]}}))
        manifest = self.root / 'manifest.json'
        manifest.write_text(json.dumps({'boards': [{'name': 'fresh', 'directory': str(source), 'board_id': 'board'}]}))
        output = self.root / 'out'
        command = [sys.executable, triage.__file__, '--manifest', str(manifest), '--output', str(output),
                   '--binary', str(binary), '--kicad-cli', str(native), '--workers', '2',
                   '--reserve-gib', '0', '--min-disk-gib', '0', '--max-load', '999']
        subprocess.run(command, check=True, stdout=subprocess.PIPE, stderr=subprocess.PIPE, timeout=20)
        rows = [json.loads(line) for line in (output / 'results.jsonl').read_text().splitlines()]
        self.assertEqual(len(rows), 1)
        self.assertEqual(rows[0]['category'], 'clean_fast')
        self.assertEqual(rows[0]['source_quality'], 'native_clean')
        self.assertTrue(rows[0]['training_candidate'])
        self.assertFalse(rows[0]['rules_inferred'])
        subprocess.run(command, check=True, stdout=subprocess.PIPE, stderr=subprocess.PIPE, timeout=20)
        self.assertEqual(len((output / 'results.jsonl').read_text().splitlines()), 1)
        (source / 'board.kicad_pcb').write_text('(kicad_pcb (changed))')
        subprocess.run(command, check=True, stdout=subprocess.PIPE, stderr=subprocess.PIPE, timeout=20)
        self.assertEqual(len((output / 'results.jsonl').read_text().splitlines()), 2)


if __name__ == '__main__':
    unittest.main()
