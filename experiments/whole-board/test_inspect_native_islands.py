# Copyright (C) 2026 Toit contributors.
"""Native connectivity controls for the read-only island helper."""
import json
import os
from pathlib import Path
import tempfile
import unittest

import pcbnew
import inspect_native_islands as helper

if not hasattr(pcbnew.SwigPyIterator, 'next'):
    pcbnew.SwigPyIterator.next = pcbnew.SwigPyIterator.__next__


class NativeIslandTests(unittest.TestCase):
    def setUp(self):
        root = os.environ.get('PCB_MAKER_ISLAND_TEST_OUTPUT')
        self.temporary = None if root else tempfile.TemporaryDirectory()
        self.root = Path(root or self.temporary.name) / self._testMethodName
        self.root.mkdir(parents=True, exist_ok=True)

    def tearDown(self):
        if self.temporary:
            self.temporary.cleanup()

    def board(self):
        board = pcbnew.BOARD()
        net = pcbnew.NETINFO_ITEM(board, 'N', 1)
        board.Add(net)
        return board, net

    def pad(self, board, net, ref, xy, layer=pcbnew.F_Cu, number='1'):
        footprint = pcbnew.FOOTPRINT(board)
        footprint.SetReference(ref)
        footprint.SetPosition(pcbnew.VECTOR2I(*(round(v * 1e6) for v in xy)))
        pad = pcbnew.PAD(footprint)
        pad.SetNumber(number)
        pad.SetAttribute(pcbnew.PAD_ATTRIB_SMD)
        pad.SetShape(pcbnew.PAD_SHAPE_CIRCLE)
        pad.SetSize(pcbnew.VECTOR2I(1_000_000, 1_000_000))
        pad.SetPosition(footprint.GetPosition())
        layers = pcbnew.LSET()
        layers.AddLayer(layer)
        pad.SetLayerSet(layers)
        pad.SetNetCode(net.GetNetCode())
        footprint.Add(pad)
        board.Add(footprint)
        return pad

    def track(self, board, net, a, b):
        track = pcbnew.PCB_TRACK(board)
        track.SetStart(pcbnew.VECTOR2I(*(round(v * 1e6) for v in a)))
        track.SetEnd(pcbnew.VECTOR2I(*(round(v * 1e6) for v in b)))
        track.SetLayer(pcbnew.F_Cu)
        track.SetWidth(200_000)
        track.SetNetCode(net.GetNetCode())
        board.Add(track)
        return track

    def inspect(self, board, name='board', maximum=8):
        path = self.root / (name + '.kicad_pcb')
        pcbnew.SaveBoard(str(path), board)
        before = helper.digest(path)
        report, loaded = helper.inspect(path, maximum)
        helper.render(loaded, report, self.root / (name + '.svg'))
        (self.root / (name + '.json')).write_text(json.dumps(report, indent=2) + '\n')
        self.assertEqual(before, helper.digest(path))
        return report

    def test_coincident_opposite_layers_require_real_via(self):
        board, net = self.board()
        self.pad(board, net, 'X1', [10, 10])
        self.pad(board, net, 'X2', [10, 10], pcbnew.B_Cu)
        opened = self.inspect(board, 'open')
        self.assertEqual([i['pad_count'] for i in opened['nets'][0]['islands']], [1, 1])
        self.assertEqual(opened['proposals'][0]['distance_mm'], 0.0)
        self.assertFalse(opened['complete'])
        via = pcbnew.PCB_VIA(board)
        via.SetPosition(pcbnew.VECTOR2I(10_000_000, 10_000_000))
        via.SetWidth(800_000)
        via.SetDrill(400_000)
        via.SetViaType(pcbnew.VIATYPE_THROUGH)
        via.SetLayerPair(pcbnew.F_Cu, pcbnew.B_Cu)
        via.SetNetCode(net.GetNetCode())
        board.Add(via)
        # Deliberately stale adjacent report must not influence native discovery.
        (self.root / 'drc.json').write_text('{"unconnected_items":[{"description":"fake open N"}]}')
        closed = self.inspect(board, 'closed')
        self.assertTrue(closed['complete'])
        self.assertEqual(closed['nets'][0]['islands'][0]['pad_count'], 2)
        self.assertEqual(closed['nets'][0]['islands'][0]['kind_counts']['PCB_VIA'], 1)
        self.assertEqual(closed['proposals'], [])

    def test_same_net_name_does_not_bridge_physical_gap(self):
        board, net = self.board()
        self.pad(board, net, 'X1', [10, 10])
        self.pad(board, net, 'X2', [12, 10])
        track = self.track(board, net, [10, 10], [11.2, 10])
        self.assertTrue(self.inspect(board, 'gap')['has_open_islands'])
        track.SetEnd(pcbnew.VECTOR2I(12_000_000, 10_000_000))
        self.assertTrue(self.inspect(board, 'connected')['complete'])

    def test_padless_island_is_retained_and_explicitly_unsupported(self):
        board, net = self.board()
        self.pad(board, net, 'X1', [10, 10])
        orphan = self.track(board, net, [20, 10], [22, 10])
        report = self.inspect(board)
        self.assertTrue(report['discovery_complete'])
        self.assertTrue(report['has_open_islands'])
        self.assertEqual(report['proposals'], [])
        self.assertEqual(report['unsupported'][0]['kind'], 'no_eligible_pad_in_island')
        self.assertIn(orphan.m_Uuid.AsString(), report['unsupported'][0]['item_uuids'])
        self.assertEqual(sorted(i['pad_count'] for i in report['nets'][0]['islands']), [0, 1])

    def test_ambiguous_existing_pad_locator_is_not_silently_selected(self):
        board, net = self.board()
        self.pad(board, net, 'X1', [10, 10])
        self.pad(board, net, 'X1', [20, 10])
        self.pad(board, net, 'X2', [30, 10])
        report = self.inspect(board)
        self.assertTrue(report['discovery_complete'])
        self.assertEqual(report['nets'][0]['pad_count'], 3)
        self.assertEqual(report['proposals'], [])
        self.assertEqual(len(report['unsupported']), 2)
        reasons = [p['endpoint_reason'] for i in report['nets'][0]['islands'] for p in i['pads']]
        self.assertEqual(reasons.count('ambiguous footprint/pad locator'), 2)

    def test_zone_context_is_unknown_not_complete(self):
        board, net = self.board()
        self.pad(board, net, 'X1', [10, 10])
        zone = pcbnew.ZONE(board)
        zone.SetLayer(pcbnew.F_Cu)
        zone.SetNetCode(net.GetNetCode())
        outline = zone.Outline()
        outline.NewOutline()
        for x, y in [(5, 5), (15, 5), (15, 15), (5, 15)]:
            outline.Append(x * 1_000_000, y * 1_000_000)
        board.Add(zone)
        report = self.inspect(board)
        self.assertFalse(report['discovery_complete'])
        self.assertFalse(report['complete'])
        self.assertIsNone(report['has_open_islands'])
        self.assertEqual(report['proposals'], [])
        self.assertEqual(report['unsupported'][0]['kind'], 'copper_zone_islands')

    def test_canonical_net_collision_is_explicit(self):
        board, net = self.board()
        other = pcbnew.NETINFO_ITEM(board, '/N', 2)
        board.Add(other)
        self.pad(board, net, 'X1', [10, 10])
        self.pad(board, other, 'X2', [20, 10])
        report = self.inspect(board)
        self.assertFalse(report['discovery_complete'])
        self.assertIn('ambiguous_canonical_net_name', [u['kind'] for u in report['unsupported']])

    def test_truncated_stable_shortlist_does_not_change_discovery_completeness(self):
        board, net = self.board()
        for ref, point in [('X4', [20, 20]), ('X2', [20, 10]), ('X3', [10, 20]), ('X1', [10, 10])]:
            self.pad(board, net, ref, point)
        report = self.inspect(board, maximum=2)
        self.assertTrue(report['discovery_complete'])
        self.assertTrue(report['has_open_islands'])
        self.assertTrue(report['proposals_truncated'])
        self.assertEqual(report['candidate_pad_pairs_considered'], 6)
        self.assertEqual([(p['start']['footprint'], p['finish']['footprint']) for p in report['proposals']], [('X1', 'X2'), ('X1', 'X3')])
        nets = json.loads(json.dumps(report['nets']))
        nets.reverse()
        for n in nets:
            n['islands'].reverse()
            for island in n['islands']:
                island['pads'].reverse()
        self.assertEqual(helper.pair_suggestions(nets, 2)[0], report['proposals'])
        repeated, _ = helper.inspect(self.root / 'board.kicad_pcb', 2)
        self.assertEqual(repeated, report)


if __name__ == '__main__':
    unittest.main()
