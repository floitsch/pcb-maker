# Copyright (C) 2026 Toit contributors.
import unittest

try:
    import native_fixture as native
except ModuleNotFoundError:
    native = None


@unittest.skipIf(native is None, "pcbnew is unavailable")
class NativeRuleExportTests(unittest.TestCase):
    def make_pad(self):
        pcb = native.pcbnew
        board = pcb.BOARD()
        footprint = pcb.FOOTPRINT(board)
        board.Add(footprint)
        pad = pcb.PAD(footprint)
        footprint.Add(pad)
        layers=pcb.LSET()
        layers.AddLayer(pcb.F_Cu)
        layers.AddLayer(pcb.B_Cu)
        pad.SetLayerSet(layers)
        return board, pad

    def test_native_own_clearance_is_exported_on_each_real_layer(self):
        source = native.ROOT / 'build/sequence-231-m0-olimex-c3-freerouting/result/ESP32-C3-DevKit-Lipo_Rev_C.kicad_pcb'
        if not source.exists():
            self.skipTest('Retained native local-clearance audit board is unavailable')
        # GetOwnClearance resolves through a loaded native board's rule context;
        # an unattached synthetic pad does not exercise that resolution.
        board = native.pcbnew.LoadBoard(str(source))
        pad = next(p for f in board.GetFootprints() if f.GetReference() == 'MH1' for p in f.Pads())
        layers = [native.pcbnew.F_Cu, native.pcbnew.B_Cu]
        result = native.resolved_pad_clearances(board, pad, layers)
        self.assertEqual(len(result), 2)
        for layer in layers:
            record = result[board.GetLayerName(layer)]
            self.assertEqual(record['value_mm'], pad.GetOwnClearance(layer)/1_000_000)
            self.assertEqual(record['value_mm'], 1.85)
            self.assertEqual(record['source_layer_id'], layer)
            self.assertEqual(record['source'], 'pcbnew.PAD.GetOwnClearance(layer)')

    def test_only_zero_delta_trapezoids_have_exact_rectangle_equivalence(self):
        board, pad = self.make_pad()
        pad.SetShape(native.pcbnew.PAD_SHAPE_TRAPEZOID)
        pad.SetDelta(native.pcbnew.VECTOR2I(0, 0))
        result = native.native_pad_shape(pad)
        self.assertEqual(result['shape'], 'rect')
        self.assertEqual(result['source_shape'], 'trapezoid')
        pad.SetDelta(native.pcbnew.VECTOR2I(100_000, 0))
        with self.assertRaisesRegex(ValueError, 'Nonzero-delta'):
            native.native_pad_shape(pad)


if __name__ == '__main__':
    unittest.main()
