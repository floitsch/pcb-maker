# Copyright (C) 2026 Toit contributors.
import copy
import unittest
from pathlib import Path
from preserved_board_graphics import is_board_graphic, preserved_graphics, children, canonical

SOURCE = Path('benchmarks/real/external/kicad-interf-u/interf_u.kicad_pcb')

class BoardGraphicsTests(unittest.TestCase):
    def setUp(self):
        self.fp = preserved_graphics(SOURCE)['G1']

    def test_explicit_silk_logo(self):
        self.assertTrue(is_board_graphic(self.fp))
        self.assertEqual(set(preserved_graphics(SOURCE)), {'G1'})

    def test_physical_or_unknown_content_is_never_skipped(self):
        for content in (['pad','1','thru_hole','circle',['drill','1']], ['model','body.step'],
                        ['zone'], ['fp_rect',['layer','F.CrtYd']],
                        ['fp_line',['layer','F.Fab']], ['unknown_geometry']):
            with self.subTest(content=content):
                self.assertFalse(is_board_graphic(self.fp+[content]))
        for layer in ('F.Cu','B.Cu','In1.Cu','Edge.Cuts'):
            self.assertFalse(is_board_graphic(self.fp+[['fp_text','user','x',['layer',layer]]]))
            self.assertFalse(is_board_graphic(self.fp+[['property','Copper','x',['layer',layer],['hide','yes']]]))

    def test_numeric_text_remains_literal(self):
        self.assertNotEqual(canonical(['fp_text','user','001',['at','1.0','2']]),
                            canonical(['fp_text','user','1',['at','1','2.0']]))
        self.assertEqual(canonical(['at','1.0','2']), canonical(['at','1','2.0']))

    def test_board_only_must_be_explicit(self):
        fp=copy.deepcopy(self.fp)
        children(fp,'attr')[0].remove('board_only')
        self.assertFalse(is_board_graphic(fp))

if __name__ == '__main__':
    unittest.main()
