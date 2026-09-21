# Copyright (C) 2026 Toit contributors.
import unittest
from native_probe import pad_distance, check_batch, check_tracks


class NativePadDistances(unittest.TestCase):
    def pad(self,shape,size,angle=0,**kw):
        return dict(shape=shape,size=size,at=[0,0],rotation_degrees=angle,**kw)

    def test_rotated_rectangle(self):
        p=self.pad('rect',[4,2],90)
        self.assertEqual(pad_distance([2,-4],[2,4],p),1)

    def test_oval_and_roundrect_corner(self):
        oval=self.pad('oval',[4,2])
        self.assertEqual(pad_distance([3,0],[4,0],oval),1)
        rr=self.pad('roundrect',[4,2],roundrect_radius=.5)
        self.assertAlmostEqual(pad_distance([2,1],[3,2],rr),2**.5/2-.5)

    def test_offset_and_circle_inside(self):
        circle=self.pad('circle',[2,2],offset=[1,0])
        self.assertEqual(pad_distance([1,0],[1,.5],circle),0)
        self.assertEqual(pad_distance([3,0],[4,0],circle),1)

    def test_batch_candidates_have_independent_removal_context(self):
        board=dict(net_settings=dict(classes=[dict(clearance=.2)]),pads=[],vias=[],
                   tracks=[dict(id='T1',net='foreign',layer='F.Cu',width=.4,start=[0,0],end=[3,0])])
        track=dict(net='selected',layer='F.Cu',width=.4,start=[1,-1],end=[1,1])
        candidates=[dict(id='blocked',proposal=dict(add_tracks=[track])),
                    dict(id='removed',proposal=dict(add_tracks=[track],remove_tracks=['T1']))]
        result=check_batch(board,candidates)
        self.assertEqual([r['assessment']['blocked_tracks'] for r in result['candidates']],[1,0])
        self.assertEqual(result['candidates'][0]['assessment'],check_tracks(board,candidates[0]['proposal']))
        compact=check_batch(board,candidates,compact=True)['candidates']
        self.assertEqual(compact[0]['assessment']['blockers'][0]['object_id'],'T1')
        self.assertIsNone(compact[1]['assessment']['minimum_margin_mm'])
        self.assertEqual(len(board['tracks']),1)

    def test_batch_rejects_ambiguous_ids(self):
        with self.assertRaises(ValueError):
            check_batch(dict(tracks=[],vias=[],pads=[]),[dict(id='x',proposal={}),dict(id='x',proposal={})])

    def local_rule_board(self):
        pad=dict(id='P1',net='B',layers=['front','back'],shape='circle',size=[2,2],at=[0,0],rotation_degrees=0,
                 own_clearance_by_layer={'front':dict(value_mm=1.0,source='native own'),
                                         'back':dict(value_mm=.25,source='native own')})
        return dict(tracks=[],vias=[],pads=[pad],effective_netclasses={'A':dict(clearance=.1),'B':dict(clearance=1.4)},
                    netclasses_by_net={'A':'A','B':'B'},effective_board_rules={'min_clearance':dict(value=0)})

    def test_resolved_pad_own_rule_overrides_its_netclass_per_layer(self):
        board=self.local_rule_board()
        def result(layer):
            proposal=dict(add_tracks=[dict(net='A',layer=layer,width=.2,start=[2,-1],end=[2,1])])
            return check_tracks(board,proposal)['tracks'][0]
        front,back=result('front'),result('back')
        self.assertEqual(front['blockers'][0]['object_id'],'P1')
        self.assertAlmostEqual(front['nearest'][0]['copper_gap_mm'],.9)
        self.assertEqual(front['nearest'][0]['required_mm'],1.0)
        self.assertEqual(back['blockers'],[])
        self.assertEqual(back['nearest'][0]['required_mm'],.25)
        self.assertEqual(back['nearest'][0]['clearance_basis']['pad_source'],'native own')

    def test_pad_local_rule_cannot_reduce_board_floor_or_other_net_requirement(self):
        board=self.local_rule_board()
        proposal=dict(add_tracks=[dict(net='A',layer='back',width=.2,start=[2,-1],end=[2,1])])
        board['effective_board_rules']['min_clearance']['value']=.35
        self.assertEqual(check_tracks(board,proposal)['tracks'][0]['nearest'][0]['required_mm'],.35)
        board['effective_netclasses']['A']['clearance']=.6
        self.assertEqual(check_tracks(board,proposal)['tracks'][0]['nearest'][0]['required_mm'],.6)

    def test_old_packet_fallback_and_invalid_resolved_rule(self):
        board=self.local_rule_board()
        proposal=dict(add_tracks=[dict(net='A',layer='front',width=.2,start=[2,-1],end=[2,1])])
        del board['pads'][0]['own_clearance_by_layer']
        witness=check_tracks(board,proposal)['tracks'][0]['nearest'][0]
        self.assertEqual(witness['required_mm'],1.4)
        self.assertIn('fallback',witness['clearance_basis']['pad_source'])
        board['pads'][0]['own_clearance_by_layer']={'front':dict(value_mm=-1,source='invalid')}
        with self.assertRaises(ValueError):
            check_tracks(board,proposal)

    def test_rule_area_metadata_is_only_accepted_when_nonrouting(self):
        board=self.local_rule_board()
        board['rule_areas']=[dict(is_rule_area=True,do_not_allow_tracks=False,do_not_allow_vias=False,do_not_allow_zone_fills=True)]
        self.assertEqual(check_tracks(board,{})['blocked_tracks'],0)
        board['rule_areas'][0]['do_not_allow_tracks']=True
        with self.assertRaises(ValueError):
            check_tracks(board,{})


if __name__=='__main__':
    unittest.main()
