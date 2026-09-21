# Copyright (C) 2026 Toit contributors.
"""Validation guards before KiCad resolves omitted global rules."""
import unittest
from pathlib import Path
import tempfile
from unittest import mock

from compile_net_classes import compile_rules, validate_explicit_global_rules


class ExplicitGlobalRulesTest(unittest.TestCase):
    def test_omitted_zero_and_nondefault_rules_are_valid(self):
        for rules in ({}, {'min_copper_edge_clearance': 0},
                      {'min_hole_to_hole': 0.375},
                      {'min_track_width': 0}, {'min_track_width': 0.5},
                      {'min_clearance': 0}, {'min_clearance': 0.35},
                      {'min_copper_edge_clearance': 0.125, 'min_hole_to_hole': 0}):
            validate_explicit_global_rules({'board': {'design_settings': {'rules': rules}}})
        validate_explicit_global_rules({})

    def test_malformed_explicit_values_cannot_become_native_defaults(self):
        for key in ('min_copper_edge_clearance', 'min_hole_to_hole', 'min_track_width', 'min_clearance'):
            for value in (True, False, None, '0.5', [], {}, -0.1, 10**400,
                          float('nan'), float('inf'), -float('inf')):
                with self.subTest(key=key, value=value), self.assertRaises(ValueError):
                    validate_explicit_global_rules({'board': {'design_settings': {'rules': {key: value}}}})

    def test_explicit_nonobject_containers_are_rejected(self):
        for project in (None, [], {'board': None}, {'board': {'design_settings': []}},
                        {'board': {'design_settings': {'rules': None}}}):
            with self.subTest(project=project), self.assertRaises(ValueError):
                validate_explicit_global_rules(project)

    def test_custom_and_malformed_rules_reject_before_native_loading(self):
        with tempfile.TemporaryDirectory() as directory:
            board = Path(directory) / 'test.kicad_pcb'
            project = board.with_suffix('.kicad_pro')
            custom = board.with_suffix('.kicad_dru')
            project.write_text('{}')
            custom.write_text('(version 1)')
            with mock.patch('compile_net_classes.pcbnew.LoadBoard') as load:
                with self.assertRaisesRegex(ValueError, 'Custom design rules'):
                    compile_rules(board)
                load.assert_not_called()
                custom.unlink()
                project.write_text('{"board":{"design_settings":{"rules":{"min_hole_to_hole":true}}}}')
                with self.assertRaisesRegex(ValueError, 'min_hole_to_hole'):
                    compile_rules(board)
                load.assert_not_called()


if __name__ == '__main__':
    unittest.main()
