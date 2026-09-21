# Copyright (C) 2026 Toit contributors.
import copy
import argparse
import json
from pathlib import Path
import tempfile
import unittest
from unittest.mock import patch

from route_one_sweep import suffix_policy, classified_failure, native_source_admissible, run


class SweepPolicyTests(unittest.TestCase):
    def test_invalid_source_is_rejected_before_any_routing_invocation(self):
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary)
            source = root / 'source'
            source.mkdir()
            (source / 'board.kicad_pcb').write_text('immutable input')
            binary = root / 'binary'
            binary.write_text('instrumented fake executable')
            config = root / 'config.json'
            config.write_text('{}')
            args = argparse.Namespace(source=source, board_id='board', binary=binary,
                config=config, output=root/'result', continue_after_failure=True, allow_annotations=False)
            calls = []
            def invalid_preflight(command, **kwargs):
                calls.append(command)
                self.assertEqual(command[1], 'verify-kicad-rung')
                (Path(command[2])/'verification.json').write_text(json.dumps(dict(
                    erc_violations=0, schematic_parity_issues=0, drc_design_violations=1)))
                return argparse.Namespace(returncode=1)
            with patch('route_one_sweep.subprocess.run', invalid_preflight):
                self.assertEqual(run(args), 2)
            self.assertEqual(len(calls), 1)
            state = json.loads((args.output/'sweep.json').read_text())
            self.assertEqual(state['termination'], 'source_preflight_rejected')
            self.assertEqual(state['invocations'], [])

    def test_native_source_errors_cannot_be_inherited_as_valid_progress(self):
        native = dict(erc_violations=0, schematic_parity_issues=0, drc_design_violations=0)
        self.assertTrue(native_source_admissible(native, None, False))
        for field in native:
            invalid = dict(native, **{field: 1})
            self.assertFalse(native_source_admissible(invalid, None, False))

    def test_rotate_failed_tail_without_retrying_or_changing_route_rules(self):
        base = dict(connection_order=['H', 'V', 'L'], routing_portfolio=[dict(clearance_mm=0.2)],
                    ripup=dict(maximum_invocations=4, maximum_diagnosis_trials=8))
        original = copy.deepcopy(base)
        result = suffix_policy(base, ['H', 'V', 'L'], ['H'], {'H', 'V'}, 3)
        self.assertEqual(result['connection_order'], ['H', 'L', 'V'])
        self.assertEqual(result['maximum_connections'], 2)
        self.assertEqual(result['ripup'], dict(maximum_invocations=1, maximum_diagnosis_trials=8))
        self.assertEqual(result['routing_portfolio'], base['routing_portfolio'])
        self.assertEqual(base, original)

    def test_no_commit_and_exhausted_budget_still_visits_unseen_once(self):
        base = dict(ripup=dict(maximum_invocations=1))
        result = suffix_policy(base, ['A', 'B', 'C'], [], {'A'}, 1)
        self.assertEqual(result['connection_order'], ['B', 'C', 'A'])
        self.assertEqual(result['maximum_connections'], 2)
        self.assertIsNone(result['ripup'])
        self.assertIsNone(suffix_policy(base, ['A', 'B', 'C'], [], {'A', 'B', 'C'}, 1))

    def test_cumulative_allowance_cannot_reset_or_go_negative(self):
        base = dict(ripup=dict(maximum_invocations=1))
        for used in [-1, 2]:
            with self.assertRaises(AssertionError):
                suffix_policy(base, ['A', 'B'], [], {'A'}, used)

    def test_only_classified_routing_failures_can_trigger_continuation(self):
        tail = dict(attempts=[dict(route_failure=dict(kind='grid_disconnected'))])
        self.assertTrue(classified_failure(tail))
        tail['attempts'][0] = dict(error='source read failed')
        self.assertFalse(classified_failure(tail))
        tail['attempts'][0] = dict(native_admission=dict(complete=False))
        self.assertFalse(classified_failure(tail))
        tail['attempts'][0] = dict(skip_reason='not activated')
        self.assertFalse(classified_failure(tail))
        tail['attempts'][0] = dict(route_failure=dict(kind='grid_disconnected'))
        tail['repair'] = dict(source_unchanged=True, error='unexpected native error')
        self.assertFalse(classified_failure(tail))


if __name__ == '__main__':
    unittest.main()
