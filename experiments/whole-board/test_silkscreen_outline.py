# Copyright (C) 2026 Toit contributors.
"""Silkscreen cleanup accepts only exact rectangular straight-edge coverage."""
import importlib.util
from pathlib import Path
from types import SimpleNamespace
import unittest
from unittest.mock import patch


spec = importlib.util.spec_from_file_location(
    'silkscreen', Path(__file__).resolve().parents[2] / 'crates/pcb-kicad/src/silkscreen.py')
silkscreen = importlib.util.module_from_spec(spec)
with patch.dict('sys.modules', pcbnew=SimpleNamespace(
        Edge_Cuts=2, SHAPE_T_SEGMENT=1, SwigPyIterator=SimpleNamespace(next=None))):
    spec.loader.exec_module(silkscreen)


class Segment:
    def __init__(self, a, b, shape=1):
        self.a = SimpleNamespace(x=a[0], y=a[1])
        self.b = SimpleNamespace(x=b[0], y=b[1])
        self.shape = shape
    def GetStart(self): return self.a
    def GetEnd(self): return self.b
    def GetShape(self): return self.shape
    def GetLayer(self): return 2


def board(segments):
    return SimpleNamespace(GetDrawings=lambda: segments)


class SilkscreenOutlineTest(unittest.TestCase):
    def setUp(self):
        self.edges = [Segment((0,0),(500,0)), Segment((500,0),(1000,0)),
                      Segment((1000,0),(1000,700)), Segment((1000,700),(0,700)),
                      Segment((0,700),(0,0))]

    def test_split_rectangle_retains_native_edges_and_handles_direction_order(self):
        source = board(self.edges)
        before = [(s.a.x,s.a.y,s.b.x,s.b.y) for s in self.edges]
        self.assertEqual(silkscreen.rectangular_outline_bounds(source), (0,0,.001,.0007))
        self.assertEqual([(s.a.x,s.a.y,s.b.x,s.b.y) for s in self.edges], before)
        reversed_edges = [Segment((s.b.x,s.b.y),(s.a.x,s.a.y)) for s in reversed(self.edges)]
        self.assertEqual(silkscreen.rectangular_outline_bounds(board(reversed_edges)), (0,0,.001,.0007))

    def test_unsplit_translated_rectangle_is_unchanged(self):
        points = [(-4000000,2000000),(6000000,2000000),(6000000,9000000),(-4000000,9000000)]
        edges = [Segment(a,b) for a,b in zip(points,points[1:]+points[:1])]
        self.assertEqual(silkscreen.rectangular_outline_bounds(board(edges)), (-4,2,6,9))

    def test_one_native_unit_gap_or_overlap_rejected(self):
        for boundary in [499,501]:
            with self.subTest(boundary=boundary), self.assertRaises(AssertionError):
                silkscreen.rectangular_outline_bounds(board(
                    [self.edges[0], Segment((boundary,0),(1000,0)), *self.edges[2:]]))

    def test_missing_duplicate_zero_length_sloped_curved_and_interior_edges_rejected(self):
        cases = [self.edges[:-1], self.edges+[self.edges[0]],
                 self.edges+[Segment((500,0),(500,0))],
                 [Segment((0,1),(500,0)),*self.edges[1:]],
                 [Segment((0,0),(500,0),shape=3),*self.edges[1:]],
                 self.edges+[Segment((300,200),(600,200))],
                 [Segment((0,0),(1000,700)),Segment((1000,700),(1000,0)),
                  Segment((1000,0),(0,700)),Segment((0,700),(0,0))]]
        for i,edges in enumerate(cases):
            with self.subTest(case=i), self.assertRaises(AssertionError):
                silkscreen.rectangular_outline_bounds(board(edges))


if __name__ == '__main__':
    unittest.main()
