# Copyright (C) 2026 Toit contributors.
"""Metric tests; requires the same pcbnew installation as native experiments."""
import math
from types import SimpleNamespace
import unittest
from native_fixture import metrics


class Track:
    def __init__(self,a,b,net=1,layer=0):
        self.a=SimpleNamespace(x=a[0],y=a[1])
        self.b=SimpleNamespace(x=b[0],y=b[1])
        self.net=net
        self.layer=layer

    def GetClass(self): return 'PCB_TRACK'
    def GetStart(self): return self.a
    def GetEnd(self): return self.b
    def GetNetCode(self): return self.net
    def GetLayer(self): return self.layer
    def GetLength(self): return math.hypot(self.b.x-self.a.x,self.b.y-self.a.y)


class CopperUnion(unittest.TestCase):
    def measured(self,tracks):
        return metrics(SimpleNamespace(GetTracks=lambda:tracks))

    def test_duplicate_and_reversed_diagonal_do_not_improve_physical_score(self):
        track=Track((0,0),(10_000_000,10_000_000))
        duplicate=Track((8_000_000,8_000_000),(2_000_000,2_000_000))
        full=self.measured([track,duplicate])
        cleaned=self.measured([track])
        self.assertAlmostEqual(full['cost_mm'],cleaned['cost_mm'])
        self.assertGreater(full['stored_length_mm'],cleaned['stored_length_mm'])

    def test_collinear_material_on_other_layers_and_nets_is_distinct(self):
        tracks=[Track((0,0),(10_000_000,0)),Track((0,0),(10_000_000,0),layer=2),
                Track((0,0),(10_000_000,0),net=2)]
        self.assertEqual(self.measured(tracks)['length_mm'],30)

    def test_disjoint_collinear_intervals_preserve_the_gap(self):
        tracks=[Track((0,0),(2_000_000,0)),Track((4_000_000,0),(6_000_000,0))]
        self.assertEqual(self.measured(tracks)['length_mm'],4)


if __name__=='__main__':
    unittest.main()
