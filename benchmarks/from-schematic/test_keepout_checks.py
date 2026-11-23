# Copyright (C) 2026 Florian Loitsch.

import tempfile
import unittest
from pathlib import Path

from test_mechanical_runner import run

exchange = run.exchange_package
keepouts = exchange.keepout_checks


class KeepoutTests(unittest.TestCase):
    def test_origin_and_rotated_body(self):
        area, = keepouts.areas_for([{"x": [2, 1], "y": [3, 4], "side": "back"}], (100, 200))
        self.assertEqual(area["bounds_mm"], [101, 203, 102, 204])
        part = {"at": [0, 0, 45], "body_corners": [[0, 2], [2, 0], [0, -2], [-2, 0]]}
        self.assertGreater(keepouts.body_separation([1.5, 1.5, 2.5, 2.5], part), 0)
        self.assertLess(keepouts.body_separation([0.3, 0.3, 0.5, 0.5], part), 0)
        with self.assertRaisesRegex(ValueError, "unsupported keepout side"):
            keepouts.areas_for([{"x": [1, 2], "y": [1, 2], "side": ""}], (0, 0))

    def kicad(self):
        try:
            import pcbnew
        except ImportError:
            self.skipTest("KiCad Python bindings are not installed")
        if not hasattr(pcbnew, "FLIP_DIRECTION_TOP_BOTTOM"):
            self.skipTest("KiCad 10 Python bindings are required")
        return pcbnew

    def test_native_layer_flags_and_routed_copper(self):
        pcbnew = self.kicad()
        board = pcbnew.BOARD()
        board.SetCopperLayerCount(4)
        both, front = keepouts.areas_for([{"x": [1, 2], "y": [3, 4], "copper": True},
                                         {"x": [1, 2], "y": [3, 4], "side": "front"}], (0, 0))
        copper = list(board.GetEnabledLayers().CuStack())
        self.assertEqual(keepouts.copper_layers_for(both, pcbnew, board), copper)
        self.assertEqual(keepouts.copper_layers_for(front, pcbnew, board), [pcbnew.F_Cu])
        both.update({"layers": [board.GetStandardLayerName(layer) for layer in copper],
                     "blocked": ["Tracks", "Vias", "ZoneFills", "Footprints"]})
        front.update({"layers": ["F.Cu"], "blocked": ["Footprints"]})
        exchange.add_rule_areas(pcbnew, board, [front, both])
        with tempfile.TemporaryDirectory() as temporary:
            path = Path(temporary) / 'case.kicad_pcb'
            self.assertTrue(pcbnew.SaveBoard(str(path), board, True))
            exchange.verify_rule_areas(pcbnew, path, [front, both])
        track, via, pour = pcbnew.PCB_TRACK(board), pcbnew.PCB_VIA(board), pcbnew.ZONE(board)
        track.SetStart(pcbnew.VECTOR2I(pcbnew.FromMM(0), pcbnew.FromMM(3.5)))
        track.SetEnd(pcbnew.VECTOR2I(pcbnew.FromMM(3), pcbnew.FromMM(3.5)))
        track.SetLayer(pcbnew.F_Cu)
        track.SetWidth(pcbnew.FromMM(0.2))
        via.SetPosition(pcbnew.VECTOR2I(pcbnew.FromMM(1.5), pcbnew.FromMM(3.5)))
        via.SetWidth(pcbnew.FromMM(0.6))
        via.SetLayerPair(pcbnew.F_Cu, pcbnew.B_Cu)
        pour.SetLayer(pcbnew.B_Cu)
        poly = keepouts.polygon_for(pcbnew, [1.1, 3.1, 1.9, 3.9])
        pour.SetFilledPolysList(pcbnew.B_Cu, poly)
        for item in (track, via, pour):
            board.Add(item)
        bounds = both['bounds_mm']
        self.assertEqual({item['kind'] for item in keepouts.copper_intrusions(pcbnew, board, bounds, [pcbnew.F_Cu])},
                         {'track', 'via'})
        self.assertEqual({item['kind'] for item in keepouts.copper_intrusions(pcbnew, board, bounds, [pcbnew.B_Cu])},
                         {'pour', 'via'})

    def test_candidate_lock_does_not_authorize_held_overlap(self):
        pcbnew = self.kicad()
        board = pcbnew.BOARD()
        part = pcbnew.FOOTPRINT(board)
        part.SetReference('J1')
        part.SetLocked(True)
        shape = pcbnew.PCB_SHAPE(part)
        shape.SetShape(pcbnew.SHAPE_T_RECT)
        shape.SetLayer(pcbnew.F_CrtYd)
        shape.SetStart(pcbnew.VECTOR2I(0, 0))
        shape.SetEnd(pcbnew.VECTOR2I(pcbnew.FromMM(3), pcbnew.FromMM(3)))
        shape.SetWidth(pcbnew.FromMM(0.05))
        part.Add(shape)
        board.Add(part)
        area, = keepouts.areas_for([{"x": [1, 2], "y": [1, 2]}], (0, 0))
        body = {"reference": "J1", "uuid": part.m_Uuid.AsString(), "side": "front", "locked": True, "at": [0, 0, 0],
                "body_corners": [[0, 0], [3, 0], [3, 3], [0, 3]]}
        with tempfile.TemporaryDirectory() as temporary:
            path = Path(temporary) / 'case.kicad_pcb'
            self.assertTrue(pcbnew.SaveBoard(str(path), board, True))
            no_exception, = keepouts.measure(pcbnew, path, [area], {"footprints": [body]}, set())
            self.assertEqual(no_exception['encroaching_parts'], ['J1'])
            authored, = keepouts.measure(pcbnew, path, [area], {"footprints": [body]}, {'J1'})
            self.assertTrue(authored['satisfied'])
            self.assertEqual(authored['permitted_held_parts'], ['J1'])
            other_side = {**area, "side": "back"}
            clear, = keepouts.measure(pcbnew, path, [other_side], {"footprints": [body]}, set())
            self.assertTrue(clear['satisfied'])
