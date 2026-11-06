# Copyright (C) 2026 Toit contributors.
"""Tests of census.py's reading and classification:
python3 -m pytest experiments/router-ceiling/test_census.py"""

import collections
import sys
import unittest
from pathlib import Path

HERE = Path(__file__).resolve().parent
sys.path.insert(0, str(HERE))
import census  # noqa: E402


def pad(description, x=0.0, y=0.0):
    return {'description': description, 'pos': {'x': x, 'y': y}}


def pair(*items):
    return census.pairs_of({'unconnected_items': [{'items': list(items)}]})[0]


class Reading(unittest.TestCase):
    def test_items_name_pads_by_reference_and_number_and_drop_the_hierarchys_slash(self):
        kind, label, net, at = census.item_of(pad('Pad A6 [/Left Board/USB/DATA+] of J7 on B.Cu', 1.0, 2.0))
        self.assertEqual((kind, label, net, at), ('pad', 'J7.A6', 'Left Board/USB/DATA+', (1.0, 2.0)))
        self.assertEqual(census.item_of(pad('PTH pad S1 [GND] of J3'))[:3], ('pad', 'J3.S1', 'GND'))
        self.assertEqual(census.item_of(pad("Zone 'GND' [GND] on F.Cu, B.Cu and 2 more, priority 0"))[:3],
                         ('zone', None, 'GND'))
        self.assertEqual(census.item_of(pad('Track [+12V] on B.Cu, length 0.7698 mm'))[:3], ('track', None, '+12V'))
        self.assertEqual(census.item_of(pad('Track (arc) [/SHLD] on F.Cu, length 0.5 mm'))[:3], ('track', None, 'SHLD'))

    def test_the_router_writes_a_slash_in_a_net_name_as_a_tag(self):
        self.assertEqual(census.plain_net('Net-(U704-OVLO{slash}OVCSEL)'), 'Net-(U704-OVLO/OVCSEL)')

    def test_the_crossing_census_says_what_closes_an_open_pad(self):
        self.assertEqual(census.crossing_status('no straight way out'), 'no straight way out')
        self.assertEqual(census.crossing_status('2 of 3 ways closed by [A track, row U1.4, crossing]; a crossing pair'),
                         'a crossing pair')
        self.assertEqual(census.crossing_status('3 of 3 ways closed by [A via]'), 'every straight way closed')
        self.assertEqual(census.crossing_status('1 of 3 ways closed by [A track]'), 'a straight way open')


class WhyLog(unittest.TestCase):
    def test_the_census_and_the_refusals_are_read_from_the_kept_attempts_block(self):
        import tempfile, os
        text = '\n'.join([
            'probe pours=tracks: 6 nets unfinished after 66 s',
            'trouble exchange VDD: a pour net in its way (GND)',
            'crossing census after the repair: U10.10 (VDD) layer 3: 1 of 2 ways closed by [GPIO7 track, row U10.9]',
            'attempt pours=connect pitch=regular (probed, continued): 2 open, 0 starved, 746 vias, 599.5 s',
            'trouble exchange /A: refused: 1 open to 1, 1 nets open to 1 (B: ...)',
            'crossing census after the repair: U10.10 (VDD) layer 3: no straight way out',
            'attempt pours=tracks pitch=regular: 560 open, 0 starved, 470 vias, 102.0 s',
        ])
        with tempfile.TemporaryDirectory() as directory:
            path = os.path.join(directory, 'route.log')
            open(path, 'w').write(text + '\n')
            self.assertEqual(len(census.attempt_blocks(path)), 2)
            verdicts, refusals = census.read_why(path, 0)
            self.assertEqual(verdicts['U10.10'], collections.Counter({'a straight way open': 1}))
            self.assertEqual(refusals['VDD'], collections.Counter({'a pour net in its way': 1}))
            verdicts, refusals = census.read_why(path, 1)
            self.assertEqual(verdicts['U10.10'], collections.Counter({'no straight way out': 1}))
            self.assertEqual(list(refusals['A']), ['refused (the open terminals would not drop)'])
            self.assertEqual(sum(census.read_why(path)[0]['U10.10'].values()), 2)


    def test_a_pocket_closed_without_a_corridor_stub_walls_its_pad_on_that_lattice(self):
        import tempfile, os
        text = '\n'.join([
            'corridor census: pitch 0.1 origin [76.565, 60.89] pad U15.31 net ENC1_RX layer 0 closed 51 none',
            'corridor census: pitch 0.1 origin [76.565, 60.89] pad U15.32 net X layer 0 open',
            'corridor census: pitch 0.1 origin [1.0, 2.0] pad U10.5 net +3.3V layer 1 closed 4 stub 3 points width 0.09 margin 0.01',
            'corridor census: pitch 0.075 origin [0, 0] pad U15.33 net Y layer 0 closed 2 none',
        ])
        with tempfile.TemporaryDirectory() as directory:
            path = os.path.join(directory, 'route.log')
            open(path, 'w').write(text + '\n')
            self.assertEqual(census.read_corridors(path, 0.1), {'U15.31': 51})
            self.assertEqual(census.read_corridors(path), {'U15.31': 51, 'U15.33': 2})
        signal = pair(pad('Pad 31 [ENC1_RX] of U15 on F.Cu'), pad('Pad 1 [ENC1_RX] of R1 on F.Cu', 2))
        self.assertEqual(census.classify_pair(signal, {'pockets': {'U15.31': 51}})[0], census.WALLED)


class Matching(unittest.TestCase):
    def test_the_same_pads_in_either_order_are_the_same_pair(self):
        ours = pair(pad('Pad 2 [GND] of MIC1 on F.Cu', 1, 1), pad('Pad 5 [GND] of MIC1 on F.Cu', 2, 1))
        theirs = pair(pad('Pad 5 [GND] of MIC1 on F.Cu', 2, 1), pad('Pad 2 [GND] of MIC1 on F.Cu', 1, 1))
        self.assertTrue(census.same_pair(ours, theirs))

    def test_a_track_at_a_pads_place_is_that_pad(self):
        ours = pair(pad('Track [/MCU/SHLD] on F.Cu, length 0.8 mm', 103.5, 117.83), pad('Pad 2 [/MCU/SHLD] of C19 on F.Cu', 95.79, 118.25))
        theirs = pair(pad('Pad 2 [/MCU/SHLD] of C19 on F.Cu', 95.79, 118.25), pad('PTH pad S4 [/MCU/SHLD] of J2', 103.5, 117.83))
        self.assertTrue(census.same_pair(ours, theirs))

    def test_a_pour_piece_matches_by_its_zone_alone_but_a_pad_must_match(self):
        ours = pair(pad('Zone [GND] on F.Cu, priority 0', 280.35, 132.12), pad('Zone [GND] on F.Cu, priority 0', 280.35, 132.12))
        theirs = pair(pad('Zone [GND] on B.Cu, priority 1', 280.25, 15.0), pad('Zone [GND] on F.Cu, priority 0', 280.35, 132.12))
        self.assertTrue(census.same_pair(ours, theirs))
        pad_to_zone = pair(pad('PTH pad S1 [GND] of J3', 141.7, 83.59), pad('Zone [GND] on F.Cu, priority 0', 280.35, 132.12))
        self.assertFalse(census.same_pair(pad_to_zone, theirs))

    def test_another_net_is_another_pair(self):
        ours = pair(pad('Pad 1 [A] of R1 on F.Cu'), pad('Pad 2 [A] of R2 on F.Cu', 5))
        theirs = pair(pad('Pad 1 [B] of R1 on F.Cu'), pad('Pad 2 [B] of R2 on F.Cu', 5))
        self.assertFalse(census.same_pair(ours, theirs))

    def test_each_of_the_designers_pairs_answers_for_one_of_ours(self):
        # A panel's copies share references: two of ours, one of the designer's.
        one = pair(pad('Pad 1 [GND] of C1 on F.Cu'), pad('Pad 2 [GND] of C2 on F.Cu', 3))
        context = {'designer_pairs': [pair(pad('Pad 1 [GND] of C1 on F.Cu'), pad('Pad 2 [GND] of C2 on F.Cu', 3))]}
        self.assertEqual(census.classify_pair(one, context)[0], census.INPUT)
        self.assertEqual(census.classify_pair(one, context)[0], census.CONGESTION)


class Causes(unittest.TestCase):
    def setUp(self):
        self.signal = pair(pad('Pad 5 [S] of U1 on F.Cu'), pad('Pad 1 [S] of R1 on F.Cu', 2))

    def test_a_dead_pad_is_walled_and_says_what_lies_near(self):
        context = {'dead_pads': {'U1.5': ['AE1 copper fp_poly', 'board cutout']}}
        cause, detail = census.classify_pair(self.signal, context)
        self.assertEqual(cause, census.WALLED)
        self.assertIn('copper graphic', detail)
        self.assertEqual(census.walled_detail(['U7.B1', 'U7.B2', 'U7.C1']), "its own part's pads")
        self.assertEqual(census.walled_detail(['U3.2', 'solder mask opening (pad U3.)']), "its own part's pads and mask openings")
        self.assertEqual(census.walled_detail(['U3.2', 'solder mask opening (gr_poly)', 'R1.1']), 'mask openings')
        self.assertEqual(census.walled_detail([]), 'nothing near (no lattice node in the pad)')

    def test_a_zone_in_the_pair_is_a_pour_island(self):
        island = pair(pad('Zone [GND] on F.Cu, priority 0'), pad('PTH pad S1 [GND] of J3', 5))
        self.assertEqual(census.classify_pair(island, {})[0], census.POUR)

    def test_a_pad_of_a_fine_row_is_the_rows_and_the_census_says_why(self):
        features = {'U1.5': {'pitch_mm': 0.4, 'part_pads': 57, 'footprint': 'lib:QFN-56'}}
        crossings = {'U1.5': collections.Counter({'a crossing pair': 3, 'a straight way open': 1})}
        cause, detail = census.classify_pair(self.signal, {'pads': features, 'crossings': crossings})
        self.assertEqual(cause, census.FINE)
        self.assertIn('QFN-56, 0.40 mm', detail)
        self.assertTrue(detail.endswith('a crossing pair'))

    def test_a_fine_pitch_pad_whose_way_out_is_open_is_congestion_and_says_the_repairs_refusal(self):
        features = {'U1.5': {'pitch_mm': 0.4, 'part_pads': 57, 'footprint': 'lib:QFN-56'}}
        crossings = {'U1.5': collections.Counter({'a straight way open': 1})}
        refusals = {'S': collections.Counter({'a pour net in its way': 1})}
        cause, detail = census.classify_pair(self.signal, {'pads': features, 'crossings': crossings, 'refusals': refusals})
        self.assertEqual(cause, census.CONGESTION)
        self.assertIn('a straight way open', detail)
        self.assertTrue(detail.endswith('the repair: a pour net in its way'))
        # Without a census at all the row is the guess.
        self.assertEqual(census.classify_pair(self.signal, {'pads': features})[0], census.FINE)

    def test_two_pad_parts_have_no_row(self):
        features = {'U1.5': {'pitch_mm': 0.64, 'part_pads': 2, 'footprint': 'lib:R_0201'}}
        self.assertEqual(census.classify_pair(self.signal, {'pads': features})[0], census.CONGESTION)

    def test_the_input_comes_first(self):
        context = {'dead_pads': {'U1.5': []}, 'designer_pairs': [pair(pad('Pad 1 [S] of R1 on F.Cu', 2), pad('Pad 5 [S] of U1 on F.Cu'))]}
        self.assertEqual(census.classify_pair(self.signal, context)[0], census.INPUT)


class Boards(unittest.TestCase):
    def test_the_input_decides_a_board_whatever_else_is_open(self):
        ranked = census.rank_classes(collections.Counter({census.WALLED: 10, census.INPUT: 1}), collections.Counter())
        self.assertEqual(ranked, [census.INPUT, census.WALLED])

    def test_open_pairs_rank_before_findings(self):
        ranked = census.rank_classes(collections.Counter({census.FINE: 1}), collections.Counter({'length rule': 5}))
        self.assertEqual(ranked, [census.FINE, 'length rule'])
        self.assertEqual(census.rank_classes(collections.Counter(), collections.Counter({'starved thermal': 1})),
                         ['starved thermal'])

    def test_the_ledger_says_when_the_budget_stopped_the_board(self):
        report = {'attempts': [{'name': 'the rung pours=tracks pitch=regular', 'negotiation_stop': 'negotiation_work_limit',
                                'work_seconds': 228.0, 'selected': True}],
                  'skipped': ['the finer lattice (0.075 mm) was not tried: projected 757 s of routing exceeds refine_budget_seconds (240 s)']}
        self.assertEqual(census.ladder_of(report), ('pours=tracks: negotiation_work_limit, 228 s', ['finer lattice']))
        self.assertTrue(census.budget_bound(report))
        report['attempts'][0]['negotiation_stop'] = 'stalled'
        self.assertFalse(census.budget_bound(report))
        report['skipped'].append("the narrow-signal step (signals at 0.102 mm) did not run: too little work was left")
        self.assertTrue(census.budget_bound(report))

    def test_a_board_that_passes_with_more_work_is_the_budgets(self):
        board = {'class': census.CONGESTION, 'second': None, 'owner': 'ours'}
        census.apply_budget(board, [None, {'pass_strict': False}, {'pass_strict': True}])
        self.assertEqual((board['class'], board['second'], board['owner']), (census.BUDGET, census.CONGESTION, 'the budget'))
        theirs = {'class': census.INPUT, 'second': None, 'owner': "input's"}
        census.apply_budget(theirs, [{'pass_strict': True}])
        self.assertEqual(theirs['class'], census.INPUT)

    def test_the_ranking_counts_first_only_and_second_classes(self):
        boards = [{'board': 'a', 'class': census.FINE, 'second': None},
                  {'board': 'b', 'class': census.FINE, 'second': census.WALLED},
                  {'board': 'c', 'class': census.WALLED, 'second': None}]
        table = dict(census.ranking(boards))
        self.assertEqual(table[census.FINE], {'first': ['a', 'b'], 'only': ['a'], 'second': []})
        self.assertEqual(table[census.WALLED], {'first': ['c'], 'only': ['c'], 'second': ['b']})


if __name__ == '__main__':
    unittest.main()
