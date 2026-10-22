#!/usr/bin/env python3
# Copyright (C) 2026 Toit contributors.
"""Publication must never retain stale or unvalidated training eligibility."""
import importlib.util
import json
from pathlib import Path
import tempfile
import os
import sys
from types import SimpleNamespace
import unittest

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

    def test_whole_job_timeout_kills_native_child_and_grandchild(self):
        marker = self.data / 'native-pids.json'
        fake = self.data / 'fake-kicad'
        fake.write_text('#!' + sys.executable + '\n' +
            'import json,os,subprocess,sys,time\n' +
            'if sys.argv[1:] == ["version"]: print("10.0.7"); sys.exit(0)\n' +
            'child = subprocess.Popen([sys.executable,"-c","import time; time.sleep(30)"])\n' +
            'open(' + repr(str(marker)) + ',"w").write(json.dumps([os.getpid(),child.pid]))\n' +
            'time.sleep(30)\n')
        fake.chmod(0o755)
        args = SimpleNamespace(stage_timeout=30, job_timeout=1, kicad_cli=str(fake))
        MODULE.STOP = False
        outcome = MODULE.convert_job(self.row, self.data, 'converter', '10.0.7', args)
        self.assertEqual(outcome['status'], 'wall_timeout')
        self.assertLess(outcome['wall_seconds'], 3)
        self.assertTrue(marker.exists())
        for pid in json.loads(marker.read_text()):
            state = Path(f'/proc/{pid}/stat')
            # A killed orphan can briefly remain a zombie until PID 1 reaps it.
            self.assertTrue(not state.exists() or state.read_text().split()[2] == 'Z')

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
