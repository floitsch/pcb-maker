# Copyright (C) 2026 Toit contributors.
import copy
import json
from pathlib import Path
import tempfile
import unittest
from area_probe import read_external_routing, write_index


class ExternalSelectionTests(unittest.TestCase):
    def setUp(self):
        self.temp = tempfile.TemporaryDirectory()
        self.addCleanup(self.temp.cleanup)
        self.root = Path(self.temp.name)
        self.output = self.root/'area-00/freerouting'
        (self.output/'routing/result').mkdir(parents=True)
        self.native = {'complete':True,'selected_net_unconnected_items':0,
            'erc_violations':0,'drc_design_violations':0,'schematic_parity_issues':0}
        self.raw = {'native':copy.deepcopy(self.native),'statistics':{'vias':3},
                    'complete':True,'routing_complete':True}
        self.save('routing/report.json',self.raw)
        self.selected = self.output/'island-bridges/candidate-000'
        self.selected.mkdir(parents=True)
        (self.selected/'preview.svg').write_text('<svg/>')
        self.receipt = {'status':'complete','complete':True,'selected_stage':'island_bridges',
            'selected_directory':str(self.selected),'selected_native':copy.deepcopy(self.native),
            'selected_render':str(self.selected/'preview.svg')}

    def save(self,name,value):
        (self.output/name).write_text(json.dumps(value))

    def test_legacy_receipt_absent_preserves_external_result(self):
        result=read_external_routing(self.output)
        self.assertTrue(result['complete'])
        self.assertEqual(result['external_report'],self.raw)
        self.assertEqual(result['routing_result']['statistics'],{'vias':3})
        self.assertIsNone(result['final_selection'])

    def test_bridge_selection_replaces_stale_result_and_render(self):
        self.raw.update(complete=False,routing_complete=False)
        self.raw['native'].update(complete=False,selected_net_unconnected_items=1)
        self.save('routing/report.json',self.raw)
        self.save('final-selection.json',self.receipt)
        result=read_external_routing(self.output)
        self.assertTrue(result['complete'])
        self.assertTrue(result['routing_complete'])
        self.assertEqual(result['external_report'],self.raw)
        self.assertEqual(result['routing_result']['directory'],str(self.selected))
        self.assertIsNone(result['routing_result']['statistics'])
        write_index(self.root,{'benchmark_status':'test','attempts':[{
            'directory':str(self.output.parent),'area_ratio':1,'status':'complete',
            'routing_result':result['routing_result']}]})
        page=(self.root/'index.html').read_text()
        self.assertIn('Final selected routing result',page)
        self.assertIn('area-00/freerouting/island-bridges/candidate-000/preview.svg',page)
        self.assertIn('Raw external native routing report',page)

    def test_partial_or_annotation_selection_cannot_borrow_raw_completion(self):
        for opens in [0,1]:
            self.receipt['selected_native'].update(complete=False,selected_net_unconnected_items=opens)
            self.receipt.update(complete=False,status='incomplete')
            self.save('final-selection.json',self.receipt)
            result=read_external_routing(self.output)
            self.assertFalse(result['complete'])
            self.assertEqual(result['routing_complete'],opens==0)
            self.assertIsNone(result['routing_result']['statistics'])

    def test_null_receipt_selection_cannot_fall_back_to_raw_success(self):
        self.save('final-selection.json',{'complete':False,'status':'rejected',
            'selected_native':None,'selected_directory':None})
        result=read_external_routing(self.output)
        self.assertFalse(result['complete'])
        self.assertFalse(result['routing_complete'])
        self.assertIsNone(result['routing_result'])

    def test_completion_requires_consistent_receipt_and_native(self):
        for patch in [{'complete':False},{'status':'timed_out'},
                      {'selected_native':dict(self.native,complete=False)},
                      {'selected_native':dict(self.native,selected_net_unconnected_items=1)},
                      {'selected_native':dict(self.native,selected_net_unconnected_items=False)}]:
            with self.subTest(patch=patch):
                self.save('final-selection.json',dict(self.receipt,**patch))
                self.assertFalse(read_external_routing(self.output)['complete'])

    def test_external_receipt_can_retain_its_own_statistics(self):
        self.receipt.update(selected_stage='external',selected_directory=str(self.output/'routing/result'))
        self.save('final-selection.json',self.receipt)
        self.assertEqual(read_external_routing(self.output)['routing_result']['statistics'],{'vias':3})
        self.receipt['selected_directory']=str(self.selected)
        self.save('final-selection.json',self.receipt)
        self.assertIsNone(read_external_routing(self.output)['routing_result']['statistics'])


if __name__ == '__main__':
    unittest.main()
