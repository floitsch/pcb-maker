# Copyright (C) 2026 Toit contributors.
"""Tests of census.py's measures: python3 -m pytest experiments/aesthetics/test_census.py"""

import sys
import unittest
from pathlib import Path

HERE = Path(__file__).resolve().parent
sys.path.insert(0, str(HERE))
import census  # noqa: E402


def track(start, end, net, layer='F.Cu', width=0.2, mid=None):
    return {'start': start, 'end': end, 'mid': mid, 'layer': layer, 'net': net, 'width': width,
            'length': census.dist(start, end)}


def pad(center, net, part, layer='F.Cu', half=(0.15, 0.5)):
    return {'center': center, 'half': half, 'net': net, 'layer': layer, 'smd': layer is not None,
            'part': part, 'name': f'{part}'}


class Bends(unittest.TestCase):
    def test_a_corner_cut_at_45_degrees_is_one_bend_and_a_sidestep_a_jog(self):
        corner = [(0, 0), (5, 0), (5.35, 0.35), (5.35, 5)]
        self.assertEqual(len(census.bends_of(corner)), 1)
        jog = [(0, 0), (5, 0), (5.5, 0.5), (10, 0.5)]
        bends = census.bends_of(jog)
        self.assertEqual(len(bends), 2)
        self.assertEqual(census.returns_of(bends), 1)
        self.assertEqual(census.chains_of(bends), (1, 0, 0))
        # The same sidestep over 3 mm of diagonal is two bends, no jog.
        wide = [(0, 0), (5, 0), (7.2, 2.2), (12, 2.2)]
        self.assertEqual(census.chains_of(census.bends_of(wide)), (0, 0, 0))

    def test_a_diagonal_drawn_in_steps_is_one_staircase(self):
        stairs = [(0, 0)]
        for step in range(4):
            stairs += [(step + 0.5, step * 0.5), (step + 0.5, step * 0.5 + 0.5)]
        stairs.append((10, 2))
        bends = census.bends_of(stairs)
        self.assertEqual(len(bends), 8)
        jogs, staircases, stair_bends = census.chains_of(bends)
        self.assertEqual((jogs, staircases, stair_bends), (0, 1, 8))
        # score-kicad-board's stair-steps count it in pairs.
        self.assertEqual(census.returns_of(bends), 4)

    def test_an_acute_corner_is_a_sharp_bend(self):
        bends = census.bends_of([(0, 0), (5, 0), (2, 3)])
        self.assertEqual(len(bends), 1)
        self.assertGreater(abs(bends[0][0]), census.SHARP_DEGREES)
        self.assertLess(abs(census.bends_of([(0, 0), (5, 0), (5, 3)])[0][0]), census.SHARP_DEGREES)


class Measures(unittest.TestCase):
    def board(self):
        pads = [pad((0, 0), 'A', 0), pad((0.5, 0), 'B', 0), pad((1.0, 0), 'C', 0),
                pad((10, 0), 'A', 1, layer=None, half=(0.8, 0.8))]
        tracks = [
            # A leaves its pin of the row with a hook inside the pad (a
            # point inside the pad ends a path: no bend), then two bends
            # to its other pad.
            track((0, 0), (0.05, 0.1), 'A'),
            track((0.05, 0.1), (0, 2), 'A'),
            track((0, 2), (10, 2), 'A'),
            track((10, 2), (10, 0), 'A'),
            # D crosses between B and C, runs beside A on the way.
            track((0.75, -2), (0.75, 2.6), 'D'),
            track((0.75, 2.6), (9, 2.6), 'D'),
        ]
        return {'tracks': tracks, 'vias': [], 'pads': pads, 'zone_nets': set()}

    def test_the_measures_of_a_small_board(self):
        result = census.measure(self.board())
        self.assertEqual(result['connections'], 1)
        self.assertEqual(result['crossings'], 1)
        self.assertEqual(result['crossings_charged'], 1)
        # A: two bends outside its pads; D: one.
        self.assertEqual(result['bends'], 3)
        self.assertEqual(result['paths'], 3)
        self.assertEqual(result['detour_nets'], 1)
        self.assertAlmostEqual(result['detour'], (census.dist((0, 0), (0.05, 0.1)) + census.dist((0.05, 0.1), (0, 2)) + 12) / 10, 3)
        # D runs 8.25 mm beside A at 0.4 mm (0.6 between centres).
        self.assertAlmostEqual(result['parallel_mm'], 8.2, 1)
        self.assertAlmostEqual(result['parallel_gap_mm'], 0.4, 3)
        self.assertAlmostEqual(result['parallel_spread_mm'], 0.0, 4)

    def test_a_wobbling_neighbour_spreads_the_spacing(self):
        board = self.board()
        board['tracks'][-1] = track((0.75, 2.6), (4, 2.6), 'D')
        board['tracks'].append(track((4, 2.6), (4.2, 2.4), 'D'))
        board['tracks'].append(track((4.2, 2.4), (9, 2.4), 'D'))
        result = census.measure(board)
        self.assertGreater(result['parallel_spread_mm'], 0.05)

    def test_detour_counts_only_nets_eligible_on_both_boards(self):
        designer, ours = self.board(), self.board()
        ours['zone_nets'] = {'A'}
        d, o = census.measure_pair(designer, ours)
        self.assertEqual((d['detour_nets'], o['detour_nets']), (0, 0))

    def test_a_via_is_a_layer_change_only_where_tracks_meet_on_two_layers(self):
        board = self.board()
        board['vias'] = [{'at': (10, 2), 'net': 'A', 'radius': 0.3}, {'at': (5, 5), 'net': 'A', 'radius': 0.3}]
        board['tracks'][3] = track((10, 2), (10, 0), 'A', layer='B.Cu')
        result = census.measure(board)
        self.assertEqual((result['vias'], result['layer_changes']), (2, 1))


class Fixture(unittest.TestCase):
    """A small board read with pcbnew (fixtures/small.kicad_pcb)."""

    def test_the_fixture_reads_and_measures(self):
        try:
            import pcbnew  # noqa: F401
        except ImportError:
            self.skipTest('no pcbnew')
        board = census.read_board(str(HERE / 'fixtures' / 'small.kicad_pcb'))
        # U1's pad 4 lies on B.Cu though the part is on the front (a
        # reversible footprint's): the B.Cu path starts there.
        layers = {p['name']: p['layer'] for p in board['pads']}
        self.assertEqual(layers['U1.4'], 'B.Cu')
        self.assertIsNone(layers['J1.1'])
        # J1 is turned 90 degrees: its pad 2 lies above pad 1.
        centers = {p['name']: p['center'] for p in board['pads']}
        self.assertAlmostEqual(centers['J1.2'][1], 8.0, 6)
        result = census.measure(board)
        self.assertEqual(result['connections'], 2)
        self.assertEqual(result['paths'], 5)
        self.assertEqual(result['bends'], 9)
        self.assertEqual(result['jogs'], 1)
        self.assertEqual(result['crossings_charged'], 1)
        self.assertEqual((result['vias'], result['layer_changes']), (1, 1))
        self.assertAlmostEqual(result['detour'], 2.537, 3)


if __name__ == '__main__':
    unittest.main()
