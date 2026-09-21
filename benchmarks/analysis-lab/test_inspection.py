# Copyright (C) 2026 Toit contributors.
import math
import unittest
from inspect_board import segment_distance, rectangle_distance, probe, probe_via, resolve_proposal


class GeometryQueries(unittest.TestCase):
    def test_anchor_actions_preserve_fractional_endpoints(self):
        board=dict(pads=[],routes=[dict(id='R',points=[[1.123,2.789],[9.413,8.617]])])
        proposal=dict(replacements=[dict(route_id='R',points=['start',[3,4],'end'],layers=['top','top'])])
        resolved=resolve_proposal(board,proposal)
        self.assertEqual(resolved['replacements'][0]['points'],[[1.123,2.789],[3,4],[9.413,8.617]])
        self.assertEqual(proposal['replacements'][0]['points'][0],'start')

    def test_crossing_touching_parallel_and_degenerate(self):
        self.assertEqual(segment_distance((0,0),(2,2),(0,2),(2,0)),0)
        self.assertEqual(segment_distance((0,0),(2,0),(1,0),(3,0)),0)
        self.assertEqual(segment_distance((0,0),(2,0),(0,1),(2,1)),1)
        self.assertEqual(segment_distance((0,0),(0,0),(1,0),(2,0)),1)

    def test_rectangle_corner_is_euclidean_not_bounding_box(self):
        self.assertAlmostEqual(rectangle_distance((0,0),(1,1),(2,2,3,3)),math.sqrt(2))
        self.assertEqual(rectangle_distance((0,2.5),(5,2.5),(2,2,3,3)),0)
        self.assertEqual(rectangle_distance((2.2,2.2),(2.3,2.3),(2,2,3,3)),0)

    def test_foreign_via_blocks_both_layers(self):
        board=dict(rules=dict(clearance=.3,trace_width=.4,via_diameter=1),pads=[],obstacles=[],
                   routes=[dict(id='R',net='foreign',points=[[0,0],[2,0],[4,0]],layers=['top','bottom'])])
        result=probe(board,(2,-2),(2,2),'top','selected')
        self.assertTrue(any(x['kind']=='via' for x in result['blockers']))
        result=probe(board,(2,-2),(2,2),'bottom','selected')
        self.assertTrue(any(x['kind']=='via' for x in result['blockers']))
        self.assertEqual(probe(board,(2,-2),(2,2),'top','foreign')['blockers'],[])

    def test_trace_fits_but_via_diameter_does_not(self):
        board=dict(rules=dict(clearance=.25,trace_width=.4,via_diameter=.8),pads=[],routes=[],
                   obstacles=[dict(id='O',rect=[0,0,5,1],layers=['top'])])
        self.assertEqual(probe(board,[1,1.5],[4,1.5],'top','N')['blockers'],[])
        blocked=probe_via(board,[2,1.5],'N')['blockers']
        self.assertEqual([b['object_id'] for b in blocked],['O'])
        self.assertAlmostEqual(blocked[0]['margin_mm'],-.15)

    def test_via_checks_opposite_layer_and_foreign_via(self):
        board=dict(rules=dict(clearance=.25,trace_width=.4,via_diameter=.8),
                   pads=[dict(id='P',at=[2,2],net='foreign',diameter=1,layers=['bottom'])],
                   obstacles=[],routes=[dict(id='R',net='foreign',points=[[0,0],[1,0],[3,0]],layers=['top','bottom'])])
        self.assertEqual(probe(board,[1.9,2],[2.1,2],'top','N')['blockers'],[])
        self.assertIn('P',[b['object_id'] for b in probe_via(board,[2,2],'N')['blockers']])
        self.assertIn('R:v1',[b['object_id'] for b in probe_via(board,[1,.9],'N')['blockers']])

    def test_transactional_context_replaces_old_foreign_routes(self):
        from generate import apply_proposal
        board=dict(rules=dict(clearance=.25,trace_width=.4,via_diameter=.8),pads=[],obstacles=[],
                   routes=[dict(id='R',net='foreign',points=[[0,0],[2,2],[4,0]],layers=['top','bottom'])])
        self.assertTrue(probe_via(board,[2,2],'N')['blockers'])
        proposal=dict(replacements=[dict(route_id='R',points=['start',[2,0],'end'],layers=['top','bottom'])])
        updated=apply_proposal(board,proposal)
        self.assertEqual(probe_via(updated,[2,2],'N')['blockers'],[])
        self.assertTrue(probe_via(updated,[2,0],'N')['blockers'])
        self.assertEqual(board['routes'][0]['points'][1],[2,2])


if __name__=='__main__':
    unittest.main()
