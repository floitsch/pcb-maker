# Copyright (C) 2026 Florian Loitsch.

import tempfile
import unittest
from pathlib import Path
from unittest.mock import patch

from freerouting_session import board_forms, import_session

try:
    import pcbnew
except ImportError:
    pcbnew = None


NET = '/A (B) "quote"\\slash'
BOARD = r'''(kicad_pcb
  (version 20211014)
  (generator pcbnew)
  (general (thickness 1.6))
  (paper "A4")
  (layers (0 "F.Cu" signal) (31 "B.Cu" signal) (44 "Edge.Cuts" user))
  (setup (pad_to_mask_clearance 0))
  (net 0 "")
  (net 7 "/OTHER")
  (net 21 "/A (B) \"quote\"\\slash")
  (gr_line (start 1 1) (end 11 1) (layer "Edge.Cuts") (width 0.05)
    (tstamp "01234567-0000-0000-0000-000000000001"))
  (footprint "Example" (layer "F.Cu") (at 2 2)
    (tstamp "01234567-0000-0000-0000-000000000002")
    (fp_text reference "J1" (at 0 -1) (layer "F.SilkS")
      (effects (font (size 1 1) (thickness 0.1))))
    (pad "" np_thru_hole circle (at 0 0) (size 1 1) (drill 1)
      (layers "F.Cu" "B.Cu" "*.Mask"))
    (pad "1" smd rect (at 2 0) (size 0.5 0.6) (layers "F.Cu" "F.Mask")
      (net 21 "/A (B) \"quote\"\\slash")))
  (segment (start 7 2) (end 8 2) (width 0.2) (layer "F.Cu") (net 7)
    (tstamp "01234567-0000-0000-0000-000000000003"))
)
'''


@unittest.skipIf(pcbnew is None, "requires native pcbnew")
class NativeSessionImport(unittest.TestCase):
    def setUp(self):
        self.tmp = tempfile.TemporaryDirectory()
        self.addCleanup(self.tmp.cleanup)
        self.board = Path(self.tmp.name) / "original.kicad_pcb"
        self.original = BOARD.replace("\n", "\r\n").encode()
        self.board.write_bytes(self.original)
        self.rules = self.board.with_suffix(".kicad_dru")
        self.rules.write_bytes(b'(version 1)\n')
        self.session = self.board.with_suffix(".ses")

    def add_copper(self, board, _session):
        code = board.FindNet(NET).GetNetCode()
        self.assertNotEqual(code, 21)
        point = lambda x, y: pcbnew.VECTOR2I(pcbnew.FromMM(x), pcbnew.FromMM(y))
        segment = pcbnew.PCB_TRACK(board)
        segment.SetStart(point(4, 2))
        segment.SetEnd(point(5, 2))
        arc = pcbnew.PCB_ARC(board)
        arc.SetStart(point(5, 2))
        arc.SetMid(point(5.5, 2.25))
        arc.SetEnd(point(6, 2))
        for track in (segment, arc):
            track.SetWidth(pcbnew.FromMM(0.2))
            track.SetLayer(pcbnew.F_Cu)
            track.SetNetCode(code)
            board.Add(track)
        via = pcbnew.PCB_VIA(board)
        via.SetPosition(point(6, 2))
        via.SetViaType(pcbnew.VIATYPE_THROUGH)
        via.SetLayerPair(pcbnew.F_Cu, pcbnew.B_Cu)
        via.SetWidth(pcbnew.FromMM(0.7))
        via.SetDrill(pcbnew.FromMM(0.3))
        via.SetNetCode(code)
        board.Add(via)
        return True

    def test_preserves_original_board_and_uses_legacy_net_codes_for_three_copper_types(self):
        with patch.object(pcbnew, "ImportSpecctraSES", self.add_copper):
            vias, length = import_session(self.board, self.session)
        self.assertEqual(vias, 1)
        self.assertGreater(length, 3.0)
        text = self.board.read_bytes().decode()
        original = self.original.decode()
        close = original.rindex(")")
        self.assertTrue(text.startswith(original[:close]))
        self.assertTrue(text.endswith(original[close:]))
        self.assertEqual(self.rules.read_bytes(), b'(version 1)\n')
        forms, _ = board_forms(text)
        additions = [body for head, body in forms if head in ("segment", "arc", "via") and "(net 21)" in body]
        self.assertEqual(len(additions), 3)
        self.assertTrue(all("(tstamp " in body and "(uuid " not in body for body in additions))
        board = pcbnew.LoadBoard(str(self.board))
        tracks = board.Tracks()
        self.assertEqual(sorted(tracks[i].GetClass() for i in range(tracks.size())),
                         ["PCB_ARC", "PCB_TRACK", "PCB_TRACK", "PCB_VIA"])
        self.assertEqual(sum(tracks[i].GetNetname() == NET for i in range(tracks.size())), 3)

    def test_rejects_a_moved_part_or_removed_initial_copper_without_replacing_the_pcb(self):
        def move_part(board, _session):
            footprint = board.GetFootprints()[0]
            footprint.SetPosition(pcbnew.VECTOR2I(9000000, 9000000))
            return True

        def remove_copper(board, _session):
            board.Remove(board.Tracks()[0])
            return True

        for change, finding in [(move_part, "non-routing"), (remove_copper, "existing copper")]:
            with self.subTest(finding=finding):
                with patch.object(pcbnew, "ImportSpecctraSES", change):
                    with self.assertRaisesRegex(ValueError, finding):
                        import_session(self.board, self.session)
                self.assertEqual(self.board.read_bytes(), self.original)

    def test_accepts_only_export_grid_rounding_and_keeps_original_pose_bytes(self):
        self.original = self.original.replace(b"(at 2 2)", b"(at 2.000039 2)")
        self.board.write_bytes(self.original)
        dsn = self.board.with_suffix(".dsn")
        dsn.write_text("(pcb sample\n  (resolution um 10)\n)\n")

        def move_to(x):
            def move(board, _session):
                footprint = board.GetFootprints()[0]
                footprint.SetPosition(pcbnew.VECTOR2I(x, 2000000))
                return True
            return move

        with patch.object(pcbnew, "ImportSpecctraSES", move_to(2000001)):
            with self.assertRaisesRegex(ValueError, "non-routing"):
                import_session(self.board, self.session, dsn)
        with patch.object(pcbnew, "ImportSpecctraSES", move_to(2000000)):
            import_session(self.board, self.session, dsn)
        self.assertEqual(self.board.read_bytes(), self.original)
