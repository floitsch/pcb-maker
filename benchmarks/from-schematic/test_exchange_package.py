# Copyright (C) 2026 Florian Loitsch.

"""Exchange inputs carry mechanical requirements without the donor layout."""

import json
import tempfile
import unittest
from pathlib import Path
from types import SimpleNamespace

from test_mechanical_runner import run

exchange = run.exchange_package
checker = run.returned_board


class ExchangePackageTests(unittest.TestCase):
    def test_donor_shape_sanitizer_keeps_geometry_without_tracks_poses_or_ids(self):
        with tempfile.TemporaryDirectory() as folder:
            donor = Path(folder) / "donor.kicad_pcb"
            donor.write_text('''(kicad_pcb
                (gr_rect (start 10 20) (end 40 50) (layer "Edge.Cuts") (uuid "donor-edge-id"))
                (gr_circle (center 15 25) (end 16 25) (layer "Edge.Cuts") (uuid "donor-hole-id"))
                (footprint "some:footprint" (at 99 98) (path "/donor/schematic/uuid"))
                (via (at 13 24)) (segment (start 10 20) (end 40 50)))''')
            geometry = checker.expected_outline({}, donor)
            text = exchange.board_text(exchange.shape_forms(donor, geometry))
            self.assertEqual(checker.board_outline(checker.children(text)), geometry)
            self.assertEqual([key for key, _ in checker.children(text)], ["gr_rect", "gr_circle"])
            for disclosure in ("donor", "99 98", "path", "uuid", "via", "segment"):
                self.assertNotIn(disclosure, text)

    def test_brief_digest_uses_geometry_with_separate_original_file_provenance(self):
        with tempfile.TemporaryDirectory() as folder:
            root = Path(folder)
            schematic = root / "case.kicad_sch"
            schematic.write_text("(kicad_sch)")
            constraints = root / "constraints.json"
            constraints.write_text(json.dumps({"version": 1, "outline": {"x": 10, "y": 20, "width": 30, "height": 30}}))
            donor = root / "raw.kicad_pcb"
            donor.write_text('''(kicad_pcb
                (gr_rect (start 10 20) (end 40 50) (layer Edge.Cuts) (tstamp "identity"))
                (segment (start 11 21) (end 12 22)) )''')
            args = SimpleNamespace(constraints=constraints, outline_board=donor, binary=Path(__file__))
            raw, _ = checker.brief_evidence(args, schematic, 2)
            geometry = checker.expected_outline({"version": 1}, donor)
            sanitized = root / "outline.kicad_pcb"
            sanitized.write_text(exchange.board_text(exchange.shape_forms(None, geometry)))
            args.outline_board = sanitized
            portable, _ = checker.brief_evidence(args, schematic, 2)
            args.outline_board = None
            without_donor, _ = checker.brief_evidence(args, schematic, 2)
            self.assertEqual(raw["brief_sha256"], portable["brief_sha256"])
            self.assertEqual(raw["brief_sha256"], without_donor["brief_sha256"])
            self.assertNotEqual(raw["inputs"]["outline_board"]["sha256"], portable["inputs"]["outline_board"]["sha256"])
            sanitized.write_text(sanitized.read_text().replace("40 50", "41 50"))
            args.outline_board = sanitized
            changed, _ = checker.brief_evidence(args, schematic, 2)
            self.assertNotEqual(changed["brief_sha256"], raw["brief_sha256"])

    def test_round_trip_keeps_the_explicit_geometry_donor_identity(self):
        with tempfile.TemporaryDirectory() as folder:
            root = Path(folder)
            schematic = root / "case.kicad_sch"
            schematic.write_text("(kicad_sch)")
            constraints = root / "constraints.json"
            constraints.write_text('{"version":1}')
            donor = root / "raw.kicad_pcb"
            donor.write_text('''(kicad_pcb
                (gr_rect (start 10 20) (end 40 50) (layer Edge.Cuts))
                (footprint "local:R" (at 500 600))
                (segment (start 11 21) (end 12 22)))''')
            args = SimpleNamespace(constraints=constraints, outline_board=donor,
                                   footprints_from=donor, binary=Path(__file__))
            source, files = checker.brief_evidence(args, schematic, 2)
            self.assertNotIn(donor, files.values())
            sanitized = root / "outline.kicad_pcb"
            geometry = checker.expected_outline({}, donor)
            sanitized.write_text(exchange.board_text(exchange.shape_forms(donor, geometry)))
            args.outline_board = sanitized
            resumed, _ = checker.brief_evidence(args, schematic, 2)
            self.assertEqual(resumed["brief_sha256"], source["brief_sha256"])
            with checker.footprint_donor_arguments(args, resumed, root) as options:
                self.assertEqual(options[0], "--footprints-from")
                staged = Path(options[1])
                self.assertNotEqual(staged, donor)
                self.assertEqual(staged.read_bytes(), donor.read_bytes())
            self.assertFalse(staged.exists())
            donor.write_text(donor.read_text().replace("500 600", "450 600"))
            with self.assertRaisesRegex(ValueError, "changed while being staged"):
                with checker.footprint_donor_arguments(args, resumed, root):
                    self.fail("used an unrecorded donor")
            fresh, _ = checker.brief_evidence(args, schematic, 2)
            self.assertNotEqual(fresh["brief_sha256"], source["brief_sha256"])
            constraints.write_text('{"version":1,"add":[{"part":"H1","from_board":true}]}')
            with self.assertRaisesRegex(ValueError, "imports geometry only"):
                with checker.footprint_donor_arguments(args, fresh, root):
                    self.fail("allowed a donated board item")

    def test_pose_side_and_manual_entries_remain_explicit(self):
        brief = {"version": 1, "move_all": True,
                 "place": [{"part": "SW2", "x": 5, "y": 7, "angle": -90}],
                 "back": ["SW*", "J9"],
                 "region": [{"parts": ["U2"], "origin": "absolute", "x": [21, 31], "y": [25, 42]}]}
        refs = ["SW2", "J9", "U2"]
        poses, sides, manual, areas = exchange.materialized_constraints(brief, refs, (100, 200))
        self.assertEqual(poses, {"SW2": [105, 207, 270]})
        self.assertEqual(sides, {"SW2": "back", "J9": "back"})
        self.assertEqual(manual[0]["entries"], brief["region"])
        self.assertEqual(manual[0]["reference_assignments"], {"U2": ["Region 1 F.Cu", "Region 1 B.Cu"]})
        self.assertEqual([(area["layer"], area["bounds_mm"]) for area in areas],
                         [("F.Cu", [21, 25, 31, 42]), ("B.Cu", [21, 25, 31, 42])])
        with self.assertRaisesRegex(ValueError, "required on both sides"):
            exchange.materialized_constraints(dict(brief, front=["SW2"]), refs, (100, 200))

    def test_soft_proximity_and_hard_limits_have_distinct_handoff_instructions(self):
        objective = {"part": "C1", "pad": "1", "pin_of": "U1:3", "minimize": True}
        limit = {"part": "C2", "pad": "1", "pin_of": "U2:22", "max_mm": 6}
        _, _, manual, _ = exchange.materialized_constraints(
            {"version": 1, "near": [limit, objective]}, ["C1", "U1", "C2", "U2"], (0, 0))
        self.assertEqual([row["entries"] for row in manual], [[objective], [limit]])
        self.assertIn("achieved pad-centre distance", manual[0]["action"])

    def test_board_relative_regions_offer_only_each_parts_eligible_sides(self):
        brief = {"version": 1, "front": ["J*"], "back": ["SW2"], "region": [
            {"parts": ["SW2", "J*", "U1"], "x": [6, 2], "y": [3, 9]},
            {"parts": ["U1"], "at": "left-half"},
        ]}
        _, _, manual, areas = exchange.materialized_constraints(brief, ["SW2", "J1", "U1"], (100, 200))
        self.assertEqual([area["bounds_mm"] for area in areas], [[102, 203, 106, 209]] * 2)
        self.assertEqual(manual[0]["reference_assignments"], {
            "J1": ["Region 1 F.Cu"], "SW2": ["Region 1 B.Cu"], "U1": ["Region 1 F.Cu", "Region 1 B.Cu"],
        })
        self.assertEqual(manual[1]["entries"], [brief["region"][1]])
        self.assertNotIn("reference_assignments", manual[1])

    def test_native_rule_area_round_trip_and_keepout_guard(self):
        try:
            import pcbnew
        except ImportError:
            self.skipTest("KiCad Python bindings are not installed")
        if not hasattr(pcbnew, "FLIP_DIRECTION_TOP_BOTTOM"):
            self.skipTest("KiCad 10 Python bindings are required")
        areas = [{"name": "Region 1 B.Cu", "layer": "B.Cu", "bounds_mm": [102, 203, 106, 209]}]
        board = pcbnew.BOARD()
        exchange.add_rule_areas(pcbnew, board, areas)
        with tempfile.TemporaryDirectory() as folder:
            path = Path(folder) / "board.kicad_pcb"
            self.assertTrue(pcbnew.SaveBoard(str(path), board, True))
            exchange.verify_rule_areas(pcbnew, path, areas)
            next(iter(board.Zones())).SetDoNotAllowTracks(True)
            self.assertTrue(pcbnew.SaveBoard(str(path), board, True))
            with self.assertRaisesRegex(ValueError, "the authored keepout flags"):
                exchange.verify_rule_areas(pcbnew, path, areas)


if __name__ == "__main__":
    unittest.main()
