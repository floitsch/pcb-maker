# Copyright (C) 2026 Toit contributors.
"""Regression checks for advancing/resuming the unattended collection."""

import importlib.util
import json
from pathlib import Path
import tempfile
import unittest

import os

# The tests' own memory ledger, not the machine's (harness_common.Ledger;
# their subprocesses inherit it).
os.environ.setdefault("PCB_MAKER_LEDGER", str(Path(tempfile.gettempdir()) / "pcb-maker-test-ledger.json"))

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
