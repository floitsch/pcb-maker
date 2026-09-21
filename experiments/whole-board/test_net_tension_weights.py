# Copyright (C) 2026 Toit contributors.
"""Native placement attraction intent stays separate from physical net rules."""
import copy
import json
import unittest

from area_probe import apply_net_tension_weights


class NetTensionWeightsTest(unittest.TestCase):
    def setUp(self):
        self.problem = {
            'rules': {'clearance': .28, 'via_diameter': 1.6, 'via_drill': .6},
            'components': [{'id': 'J1', 'position': {'x': 3, 'y': 4}}],
            'electrical_nets': [
                {'id': name, 'width': .8, 'allowed_layers': ['top', 'bottom'],
                 'terminals': [{'component': 'J1', 'pin': '1'},
                               {'component': 'U1', 'pin': '2'}]}
                for name in ['GND', '/channel/RETURN', 'LOCAL']],
        }

    def test_omitted_and_empty_policies_preserve_exact_serialization(self):
        before = json.dumps(self.problem)
        for policy in [None, {}, {'net_tension_weights': {}}]:
            apply_net_tension_weights(self.problem, policy)
            self.assertEqual(json.dumps(self.problem), before)

    def test_exact_names_zero_and_fractional_weights_change_only_attraction(self):
        before = copy.deepcopy(self.problem)
        apply_net_tension_weights(self.problem, {
            'net_tension_weights': {'/channel/RETURN': 0, 'GND': .05}})
        self.assertEqual(self.problem['electrical_nets'][0].pop('tension_weight'), .05)
        self.assertEqual(self.problem['electrical_nets'][1].pop('tension_weight'), 0)
        self.assertEqual(self.problem, before)

    def test_invalid_maps_are_rejected_atomically(self):
        invalid = [None, [], 'GND', {'channel/RETURN': .05}, {'gnd': .05},
                   {'MISSING': .05}, {1: .05}]
        invalid += [{'LOCAL': value} for value in
                    [-1, float('nan'), float('inf'), -float('inf'), True,
                     False, '.05', None, [], {}, 10**1000]]
        before = copy.deepcopy(self.problem)
        for weights in invalid:
            with self.subTest(weights=weights):
                if isinstance(weights, dict):
                    weights = {'GND': .05, **weights}
                with self.assertRaises(ValueError):
                    apply_net_tension_weights(self.problem, {'net_tension_weights': weights})
                self.assertEqual(self.problem, before)


if __name__ == '__main__':
    unittest.main()
