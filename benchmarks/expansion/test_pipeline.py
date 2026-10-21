# Copyright (C) 2026 Toit contributors.
"""Regression checks for advancing/resuming the unattended collection."""

import importlib.util
import json
from pathlib import Path
import sys
import tempfile
import unittest
from unittest.mock import patch

import samples

# This directory's run.py by its path: benchmarks/agent-tasks has a run.py
# too, and a pytest run over both directories imports whichever came first
# under the name `run`.
_SPEC = importlib.util.spec_from_file_location("expansion_run", Path(__file__).resolve().parent / "run.py")
pipeline = importlib.util.module_from_spec(_SPEC)
_SPEC.loader.exec_module(pipeline)


class PipelineTests(unittest.TestCase):
    def test_followup_batch_advances_past_completed_sources(self):
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary)
            completed = root / 'results/boards/a'
            completed.mkdir(parents=True)
            rows = [{'name': str(i), 'category': 'incomplete_at_budget',
                     'provenance': {'name': str(i), 'source_id': f'repo:{i}'}}
                    for i in range(3)]
            (completed / 'result.json').write_text(json.dumps(rows[0]))
            destination = root / 'manifest.json'
            pipeline.select_manifest(rows, destination, {'incomplete_at_budget'}, 1,
                                     root / 'results')
            self.assertEqual(json.loads(destination.read_text())['boards'][0]['name'], '1')

    def test_feature_batches_accumulate_without_reexporting_first_batch(self):
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary)
            binary = root / 'fake-exporter'
            binary.write_text('#!' + sys.executable + '\n'
                              'import json,pathlib,sys\n'
                              'out=pathlib.Path(sys.argv[4]);out.mkdir(parents=True,exist_ok=True)\n'
                              'job=json.loads(pathlib.Path(sys.argv[5]).read_text())\n'
                              'for v in job["variants"]:\n'
                              ' p=out/v["name"]\n'
                              ' p.with_suffix(".json").write_text("{}")\n'
                              ' p.with_suffix(".bin").write_bytes(b"features")\n'
                              ' p.with_suffix(".kicad_pcb").write_text("(kicad_pcb " + str(out) + v["name"] + ")")\n')
            binary.chmod(0o755)
            quick = root / 'quick'
            for i in range(2):
                work = quick / f'boards/{i}'
                reference = work / 'reference'
                reference.mkdir(parents=True)
                (reference / 'board.kicad_pro').write_text('{}')
                row = {'name': str(i), 'fingerprint': str(i), 'source_sha256': str(i),
                       'work_directory': str(work), 'nn_license_eligible': True,
                       'source_quality': 'native_clean', 'rules_inferred': False,
                       'category': 'clean_medium', 'provenance': {
                           'repository': f'owner/repo{i}', 'board_id': 'board'}}
                (work / 'result.json').write_text(json.dumps(row))
            output = root / 'samples'
            arguments = ['samples.py', '--triage-output', str(quick), '--output', str(output),
                         '--binary', str(binary), '--workers', '1', '--variants', '2',
                         '--limit', '1', '--reserve-gib', '0', '--min-disk-gib', '0',
                         '--max-load', '1000000']
            with patch.object(sys, 'argv', arguments):
                samples.main()
                samples.main()
            manifest = json.loads((output / 'manifest.json').read_text())
            self.assertEqual(len(manifest['boards']), 4)
            self.assertEqual(len(list(output.glob('exports/*/result.json'))), 2)
            self.assertTrue(all(b['label_status'].startswith('unmeasured')
                                for b in manifest['boards']))

    def test_unverified_rules_and_timeouts_do_not_become_training_inputs(self):
        row = {'nn_license_eligible': True, 'source_quality': 'native_clean',
               'rules_inferred': False, 'category': 'clean_fast'}
        self.assertTrue(samples.eligible(row))
        self.assertFalse(samples.eligible(dict(row, rules_inferred=True)))
        self.assertFalse(samples.eligible(dict(row, category='unresolved_timeout')))
        self.assertFalse(samples.eligible(dict(row, nn_license_eligible=False)))

    def test_identical_variants_are_not_routed_twice_but_rules_distinguish_inputs(self):
        rows = [{'sha256': 'geometry', 'rule_files_sha256': {'.kicad_pro': 'a'}},
                {'sha256': 'geometry', 'rule_files_sha256': {'.kicad_pro': 'a'}},
                {'sha256': 'geometry', 'rule_files_sha256': {'.kicad_pro': 'b'}}]
        self.assertEqual(len(samples.unique_placements(rows)), 2)

    def test_interrupted_exports_retry_but_repeated_resource_failures_are_bounded(self):
        row = {'status': 'partial_or_failed', 'process': {'stop_reason': 'cancelled'}, 'attempts': 1}
        self.assertTrue(samples.retry_export(row))
        self.assertFalse(samples.retry_export(dict(row, attempts=3)))
        self.assertFalse(samples.retry_export(dict(row, status='exported')))

    def test_old_followup_does_not_hide_a_new_source_or_binary_fingerprint(self):
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary)
            old = root / 'results/boards/old'
            old.mkdir(parents=True)
            (old / 'result.json').write_text(json.dumps({
                'name': 'board', 'category': 'clean_medium', 'provenance': {
                    'name': 'board', 'source_id': 'repo:board', 'intake_parent_fingerprint': 'old'}}))
            rows = [{'name': 'board', 'fingerprint': 'new', 'category': 'incomplete_at_budget',
                     'provenance': {'name': 'board', 'source_id': 'repo:board'}}]
            path = root / 'next.json'
            self.assertEqual(pipeline.select_manifest(rows, path, {'incomplete_at_budget'}, 1,
                                                     root / 'results'), 1)
            self.assertEqual(json.loads(path.read_text())['boards'][0]['intake_parent_fingerprint'], 'new')


if __name__ == '__main__':
    unittest.main()
