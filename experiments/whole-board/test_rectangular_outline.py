# Copyright (C) 2026 Toit contributors.
"""Exact rectangular contour recognition; native split segments stay intact."""
import unittest
from unittest.mock import patch
from types import SimpleNamespace
import area_probe


class Segment:
    def __init__(self, a, b):
        self.a = SimpleNamespace(x=a[0], y=a[1])
        self.b = SimpleNamespace(x=b[0], y=b[1])
    def GetStart(self): return self.a
    def GetEnd(self): return self.b
    def GetShape(self): return 1
    def GetLayer(self): return 2


def board(points, extra=()):
    edges = [Segment(a,b) for a,b in zip(points,points[1:]+points[:1])]
    edges += [Segment(a,b) for a,b in extra]
    return SimpleNamespace(GetDrawings=lambda: edges)


class RectangleOutlineTest(unittest.TestCase):
    def setUp(self):
        self.patcher = patch.object(area_probe, 'load_pcbnew',
                                    return_value=SimpleNamespace(Edge_Cuts=2, SHAPE_T_SEGMENT=1))
        self.patcher.start()
        self.addCleanup(self.patcher.stop)

    def test_split_rectangle_retains_every_edge(self):
        native = board([(0,0),(500,0),(1000,0),(1000,700),(0,700)])
        edges = native.GetDrawings()[:]
        self.assertEqual(area_probe.rectangular_outline_bounds(native), (0,0,1000,700))
        self.assertEqual(native.GetDrawings(), edges)
        self.assertEqual(len(native.GetDrawings()), 5)

    def test_slopes_notches_crossings_and_multiple_loops_rejected(self):
        cases = [
            board([(0,0),(1000,1),(1000,700),(0,700)]),
            board([(0,0),(1000,0),(1000,700),(500,700),(500,650),(0,650)]),
            board([(0,0),(1000,700),(1000,0),(0,700)]),
            board([(0,0),(1000,0),(1000,700),(0,700)],
                  [((200,200),(300,200)),((300,200),(300,300)),((300,300),(200,200))]),
        ]
        for native in cases:
            with self.subTest(native=native), self.assertRaises(AssertionError):
                area_probe.rectangular_outline_bounds(native)


if __name__ == '__main__':
    unittest.main()
