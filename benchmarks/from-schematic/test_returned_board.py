# Copyright (C) 2026 Florian Loitsch.

"""Focused checks for the vendor-neutral standalone verification contract."""

import json
import subprocess
import sys
import tempfile
import unittest
from pathlib import Path
from types import SimpleNamespace
from unittest.mock import patch

from test_mechanical_runner import run

import mechanical_checks

checker = run.returned_board


class ExplicitMechanicalChecksTests(unittest.TestCase):
    BOARD = '''(kicad_pcb
      (version 20260206) (generator pcbnew)
      (general (thickness 1.6))
      (layers (0 "F.Cu" signal) (2 "B.Cu" signal) (25 "Edge.Cuts" user))
      (gr_rect (start 0 0) (end 10 10) (stroke (width 0.05) (type default))
        (fill none) (layer "Edge.Cuts"))
      (footprint "Test:H1" (layer "F.Cu") (at 2 2)
        (uuid "00000000-0000-4000-8000-000000000001")
        (property "Reference" "H1" (at 0 0 0) (layer "F.Cu")
          (hide yes) (effects (font (size 1 1) (thickness 0.1))))
        (attr board_only)
        (pad "" np_thru_hole circle (at 0 0) (size 1 1) (drill 1) (layers "*.Cu" "*.Mask")
          (uuid "00000000-0000-4000-8000-000000000002")))
      (footprint "Test:antenna" (layer "F.Cu") (at 8 5)
        (property "Reference" "U2" (at 0 0 0) (layer "F.Cu")
          (hide yes) (effects (font (size 1 1) (thickness 0.1))))
        (zone (layers "F.Cu" "B.Cu") (hatch edge 0.5)
          (keepout (tracks not_allowed) (vias not_allowed) (pads not_allowed)
            (copperpour not_allowed) (footprints not_allowed))
          (polygon (pts (xy 10.5 3) (xy 12 3) (xy 12 7) (xy 10.5 7))))
        (pad "1" smd rect (at 0 0) (size 1 1) (layers "F.Cu"))))'''

    def test_requires_explicit_donor_and_supported_brief(self):
        add = {"part": "H1", "from_board": True}
        with self.assertRaisesRegex(ValueError, "explicit --mechanical-donor"):
            mechanical_checks.additions_for({"add": [add]}, None)
        self.assertEqual(mechanical_checks.additions_for({"add": [add]}, Path("donor.kicad_pcb")), {"H1"})
        with self.assertRaisesRegex(ValueError, "exact part and from_board"):
            mechanical_checks.additions_for({"add": [dict(add, x=2)]}, Path("donor.kicad_pcb"))
        edge = {"part": "U2", "edge": "any", "flush": True, "overhang": {}}
        self.assertEqual(mechanical_checks.edges_for({"edge": [edge]}), {"U2"})
        with self.assertRaisesRegex(ValueError, "edge supports"):
            mechanical_checks.edges_for({"edge": [dict(edge, overhang={"edge": "right"})]})

    def test_native_addition_and_antenna_geometry(self):
        try:
            import pcbnew as p
        except ImportError:
            self.skipTest("native mechanical matching requires pcbnew")
        with tempfile.TemporaryDirectory() as directory:
            path = Path(directory) / "mechanical.kicad_pcb"
            path.write_text(self.BOARD)
            donor, returned = (p.LoadBoard(str(path)) for _ in range(2))
            parts = mechanical_checks.by_reference(returned)
            hole, antenna = parts["H1"][0], parts["U2"][0]
            unlocked, = mechanical_checks.addition_checks(p, donor, returned, {"H1"})
            self.assertEqual(unlocked["failures"], ["unlocked"])
            self.assertFalse(unlocked["donor_was_locked"])
            self.assertTrue(unlocked["expected"]["locked"])
            hole.SetLocked(True)
            added, = mechanical_checks.addition_checks(p, donor, returned, {"H1"})
            self.assertTrue(added["satisfied"])
            edge, = mechanical_checks.edge_checks(p, donor, returned, {"U2"})
            self.assertTrue(edge["satisfied"])
            self.assertEqual(edge["edge_gap_mm"], 0.5)
            self.assertEqual(edge["antenna_outline_overlap_mm2"], 0)

            hole.SetPosition(p.VECTOR2I(p.FromMM(3), p.FromMM(2)))
            antenna.SetPosition(p.VECTOR2I(p.FromMM(7), p.FromMM(5)))
            moved, = mechanical_checks.addition_checks(p, donor, returned, {"H1"})
            self.assertEqual(moved["failures"], ["pose"])
            encroaching, = mechanical_checks.edge_checks(p, donor, returned, {"U2"})
            self.assertFalse(encroaching["satisfied"])
            self.assertEqual(encroaching["antenna_outline_overlap_mm2"], 2)

            hole_form = next(form for kind, form in checker.children(self.BOARD)
                             if kind == "footprint" and '"Test:H1"' in form)
            path.write_text(self.BOARD.replace(hole_form, ""))
            missing, = mechanical_checks.addition_checks(p, donor, p.LoadBoard(str(path)), {"H1"})
            self.assertEqual(missing["failures"], ["missing"])


class ModuleNonoverlapTests(unittest.TestCase):
    def test_native_module_gap_at_chamfers_or_corners(self):
        try:
            import pcbnew as p
        except ImportError:
            self.skipTest("native module outlines require pcbnew")
        board = p.BOARD()
        for ref, x, y in (("U1", 0, 0), ("U2", 5, 6)):
            part = p.FOOTPRINT(board)
            part.SetReference(ref)
            shape = p.PCB_SHAPE(part)
            shape.SetShape(p.SHAPE_T_RECT)
            shape.SetStart(p.VECTOR2I(0, 0))
            shape.SetEnd(p.VECTOR2I(p.FromMM(2), p.FromMM(2)))
            shape.SetLayer(p.Eco2_User)
            part.Add(shape)
            part.SetPosition(p.VECTOR2I(p.FromMM(x), p.FromMM(y)))
            board.Add(part)
        with tempfile.TemporaryDirectory() as directory:
            path = Path(directory) / "modules.kicad_pcb"
            p.SaveBoard(str(path), board)
            group = {"parts": ["U1", "U2"], "layer": "User.Eco2", "min_gap_mm": 5}
            accepted, = checker.module_nonoverlap_checks([group], path)
            self.assertEqual(accepted["distance_mm"], 5.0)
            self.assertTrue(accepted["satisfied"])
            denied, = checker.module_nonoverlap_checks([dict(group, min_gap_mm=5.1)], path)
            self.assertFalse(denied["satisfied"])
            self.assertEqual(denied["intersection_mm2"], 0.0)
            with self.assertRaisesRegex(ValueError, "finite nonnegative length"):
                checker.module_nonoverlap_checks([dict(group, min_gap_mm=-0.5)], path)

    def test_rotated_contours_opposite_faces_and_missing_shapes(self):
        try:
            import pcbnew as p
        except ImportError:
            self.skipTest("native module outlines require pcbnew")
        board = p.BOARD()
        parts = []
        for ref in ("U1", "U2", "U3"):
            part = p.FOOTPRINT(board)
            part.SetReference(ref)
            shape = p.PCB_SHAPE(part)
            shape.SetShape(p.SHAPE_T_RECT)
            shape.SetStart(p.VECTOR2I(0, 0))
            shape.SetEnd(p.VECTOR2I(p.FromMM(8), p.FromMM(2)))
            shape.SetLayer(p.Eco2_User)
            part.Add(shape)
            board.Add(part)
            parts.append(part)
        parts[1].SetOrientationDegrees(90)
        parts[1].SetPosition(p.VECTOR2I(p.FromMM(3), p.FromMM(4)))
        parts[2].Flip(parts[2].GetPosition(), p.FLIP_DIRECTION_TOP_BOTTOM)
        with tempfile.TemporaryDirectory() as directory:
            path = Path(directory) / "modules.kicad_pcb"
            p.SaveBoard(str(path), board)
            entries = [{"parts": ["U1", "U2", "U3"], "layer": "Eco2.User"}]
            first, second, third = checker.module_nonoverlap_checks(entries, path)
            self.assertEqual(first["intersection_mm2"], 4.0)
            self.assertFalse(first["satisfied"])
            self.assertEqual(first["distance_mm"], 0.0)
            self.assertEqual(first["min_gap_mm"], 0.0)
            self.assertTrue(second["satisfied"] and third["satisfied"])
            self.assertFalse(second["same_side"] or third["same_side"])
            self.assertIsNone(second["distance_mm"])
            with self.assertRaisesRegex(ValueError, "no usable closed module contour"):
                checker.module_nonoverlap_checks([dict(entries[0], layer="User.Eco1")], path)
            options = {"electrical": False, "module_groups": entries, "modules_only": True}
            expected = checker.normalized_footprints(p, board, **options)
            outline = next(g for g in parts[0].GraphicalItems() if g.GetLayer() == p.Eco2_User)
            outline.SetEnd(p.VECTOR2I(p.FromMM(1), p.FromMM(1)))
            changed = checker.normalized_footprints(p, board, **options)
            differences = checker.footprint_differences(expected, changed)
            self.assertEqual([(d["reference"], d["difference"]) for d in differences], [("U1", "body")])


class FootprintClearancePolicyTests(unittest.TestCase):
    def test_a_generated_override_does_not_pass_as_the_supplied_policy(self):
        board = '''(kicad_pcb
            (footprint "IC" (property "Reference" "U9") (pad "1" (clearance 0.1)))
            (footprint "Authored" (property "Reference" "J1") (clearance 0.12)))'''
        expected = list(checker.children(board))
        unchanged = board.replace("0.12", "0.120000")
        self.assertTrue(checker.check_footprint_clearances(expected, checker.children(unchanged))["satisfied"])
        changed = board.replace('(pad "1"', '(clearance 0.15) (pad "1"')
        comparison = checker.check_footprint_clearances(expected, checker.children(changed))
        self.assertFalse(comparison["satisfied"])
        self.assertEqual(comparison["differences"], [{"reference": "U9", "expected_mm": None, "actual_mm": 0.15}])
        evidence = {"footprint_clearance_policy": "preserve_supplied"}
        checker.check_imported_clearance_policy({**evidence, "clearances": []}, evidence)
        with self.assertRaisesRegex(ValueError, "did not record preservation"):
            checker.check_imported_clearance_policy({"clearances": []}, evidence)


class StackupTests(unittest.TestCase):
    BOARD = '''(kicad_pcb
      (general (thickness 1.5296))
      (layers (0 "F.Cu" signal) (2 "B.Cu" signal))
      (setup (stackup
        (layer "F.Cu" (type "copper") (thickness 0.035))
        (layer "dielectric 1" (type "prepreg") (thickness 1.4596) (material "FR4") (epsilon_r 4.45))
        (layer "B.Cu" (type "copper") (thickness 0.035))
        (copper_finish "ENIG") (dielectric_constraints yes))))'''

    def test_complete_layer_stack_is_compared_at_the_same_thickness(self):
        expected = checker.stackup_signature(self.BOARD)
        self.assertEqual(expected["thickness_mm"], 1.5296)
        same_stack = self.BOARD.replace("4.45", "4.4500").replace("1.5296", "1.5296000")
        self.assertTrue(checker.check_stackup(expected, same_stack)["satisfied"])
        for original, changed in (("4.45", "4.15"), ("FR4", "PTFE"), ("ENIG", "HASL"), ("1.5296", "1.6")):
            with self.subTest(changed=changed):
                complaint = checker.check_stackup(expected, self.BOARD.replace(original, changed))
                self.assertFalse(complaint["satisfied"])
                self.assertEqual(complaint["actual"]["copper_layers"], expected["copper_layers"])

    def test_stackup_needs_the_declared_copper_in_order(self):
        expected = checker.stackup_signature(self.BOARD)
        for malformed in (self.BOARD.replace('(layer "B.Cu"', '(layer "In1.Cu"'),
                          self.BOARD.replace("stackup", "other")):
            complaint = checker.check_stackup(expected, malformed)
            self.assertFalse(complaint["satisfied"])
            self.assertIn("error", complaint)

    def test_native_resave_normalizes_thickness_and_numbered_dielectric_labels(self):
        original = (self.BOARD.replace("1.5296", "1.52960049")
                    .replace("0.035", "0.035001199999999996")
                    .replace('"dielectric 1"', '"Dielectric 2"'))
        resaved = self.BOARD.replace("0.035", "0.035001")
        expected = checker.stackup_signature(original)
        self.assertTrue(checker.check_stackup(expected, resaved)["satisfied"])
        for changed in (resaved.replace("1.5296", "1.52960050"),
                        resaved.replace("0.035001", "0.035002", 1),
                        resaved.replace('"dielectric 1"', '"custom dielectric"')):
            self.assertFalse(checker.check_stackup(expected, changed)["satisfied"])

    def test_stackup_difference_names_a_changed_dielectric_property(self):
        with tempfile.TemporaryDirectory() as temporary:
            reference = Path(temporary) / "accepted.kicad_pcb"
            original = self.BOARD.replace('"dielectric 1"', '"Dielectric 2"').replace("0.035", "0.0349999")
            reference.write_text(original)
            expected = checker.stackup_signature(original)
            changed = self.BOARD.replace("4.45", "4.4500001")
            check = checker.check_stackup(expected, changed, reference)
            self.assertFalse(check["satisfied"])
            self.assertEqual(check["difference"], {
                "layer": "Dielectric 2", "property": "epsilon_r", "expected": "4.45", "actual": "4.4500001"})
            reference.write_text(changed)
            with self.assertRaisesRegex(ValueError, "comparison snapshot differs"):
                checker.check_stackup(expected, changed, reference)

    def test_donor_staging_rejects_changed_input(self):
        with tempfile.TemporaryDirectory() as temporary:
            work = Path(temporary)
            donor = work / "source.kicad_pcb"
            donor.write_text(self.BOARD)
            original, expected = checker.read_stackup_donor(donor)
            arguments = SimpleNamespace(footprints_from=None, stackup_from=donor)
            evidence = {"inputs": {"stackup_from": original}, "expected_stackup": expected}
            with checker.board_donor_arguments(arguments, evidence, work) as args:
                self.assertEqual(args[0], "--stackup-from")
                self.assertNotEqual(Path(args[1]), donor)
                self.assertEqual(Path(args[1]).read_text(), self.BOARD)
            self.assertFalse(Path(args[1]).exists())
            donor.write_text(self.BOARD.replace("4.45", "4.15"))
            with self.assertRaisesRegex(ValueError, "stackup donor changed"):
                with checker.board_donor_arguments(arguments, evidence, work):
                    self.fail("changed stackup reached the import")


class PadDistanceTests(unittest.TestCase):
    def test_body_constraint_does_not_silently_become_a_pad_constraint(self):
        with self.assertRaisesRegex(ValueError, "pad, pin_of"):
            checker.pad_distance_checks([{"part": "C1", "pin_of": "J1:2", "max_mm": 1}], None)

    def test_body_to_pin_uses_offset_courtyard_and_pad_extent_at_final_poses(self):
        try:
            import pcbnew  # noqa: F401
        except ImportError:
            self.skipTest("native body measurements require pcbnew")
        board = '''(kicad_pcb
          (version 20260206) (generator pcbnew)
          (general (thickness 1.6))
          (layers (0 "F.Cu" signal) (2 "B.Cu" signal) (25 "Edge.Cuts" user)
            (31 "F.CrtYd" user) (29 "B.CrtYd" user))
          (footprint "Test:C1" (layer "B.Cu") (at 5 5 90)
            (property "Reference" "C1" (at 0 0 90) (layer "B.Cu")
              (hide yes) (effects (font (size 1 1) (thickness 0.1))))
            (fp_rect (start 1 1) (end 3 4) (stroke (width 0.1) (type solid)) (fill none) (layer "B.CrtYd"))
            (fp_rect (start -8 -9) (end 5 12) (stroke (width 0.1) (type solid)) (fill none) (layer "F.CrtYd"))
            (pad "1" smd rect (at 2 2 90) (size 1 1) (layers "B.Cu")))
          (footprint "Test:C2" (layer "F.Cu") (at 4 1 90)
            (property "Reference" "C2" (at 0 0 90) (layer "F.Cu")
              (hide yes) (effects (font (size 1 1) (thickness 0.1))))
            (pad "1" smd rect (at 1 2 90) (size 2 1) (layers "F.Cu")))
          (footprint "Test:C3" (layer "F.Cu") (at 4 6 45)
            (property "Reference" "C3" (at 0 0 45) (layer "F.Cu")
              (hide yes) (effects (font (size 1 1) (thickness 0.1))))
            (fp_rect (start -1 -2) (end 1 2) (stroke (width 0.1) (type solid)) (fill none) (layer "F.CrtYd")))
          (footprint "Test:J1" (layer "F.Cu") (at 10 6)
            (property "Reference" "J1" (at 0 0 0) (layer "F.Cu")
              (hide yes) (effects (font (size 1 1) (thickness 0.1))))
            (pad "1" smd rect (at 0 0) (size 1 1) (layers "F.Cu"))))'''
        with tempfile.TemporaryDirectory() as directory:
            path = Path(directory) / "near.kicad_pcb"
            path.write_text(board)
            entries = [{"part": ref, "pin_of": "J1:1", "max_mm": 2.5} for ref in ("C1", "C2", "C3")]
            offset, fallback, angled = checker.near_checks(entries, path)
            self.assertEqual(offset["measurement"], "body_to_pin")
            self.assertNotIn("pad", offset)
            self.assertEqual(offset["body_bounds_mm"], [6, 2, 9, 4])
            self.assertAlmostEqual(offset["distance_mm"], 5**0.5, places=6)
            self.assertTrue(offset["satisfied"])
            self.assertEqual(fallback["body_bounds_mm"], [5.5, -1, 6.5, 1])
            self.assertAlmostEqual(fallback["distance_mm"], (3.5**2 + 5**2)**0.5, places=6)
            self.assertFalse(fallback["satisfied"])
            self.assertAlmostEqual(angled["distance_mm"], 6 - 3 / 2**0.5, places=5)
            with self.assertRaisesRegex(ValueError, "one footprint for 'C9'"):
                checker.near_checks([dict(entries[0], part="C9")], path)
            with self.assertRaisesRegex(ValueError, "one pad centre for J1:2"):
                checker.near_checks([dict(entries[0], pin_of="J1:2")], path)
            path.write_text(board.replace('(property "Reference" "C2"', '(property "Reference" "C1"'))
            with self.assertRaisesRegex(ValueError, "one footprint for 'C1'"):
                checker.near_checks(entries, path)
            path.write_text(board.replace('(pad "1" smd rect (at 0 0) (size 1 1) (layers "F.Cu"))',
                                          '(pad "1" smd rect (at 0 0) (size 1 1) (layers "F.Cu")) '
                                          '(pad "1" smd rect (at 2 0) (size 1 1) (layers "F.Cu"))'))
            with self.assertRaisesRegex(ValueError, "one pad centre for J1:1"):
                checker.near_checks(entries, path)

    def test_objective_measures_the_same_pads_without_a_limit(self):
        try:
            import pcbnew  # noqa: F401
        except ImportError:
            self.skipTest("native pad measurements require pcbnew")
        board = run.ROOT / "tests/fixtures/f1/f1.kicad_pcb"
        pair = {"part": "J2", "pad": "1", "pin_of": "J1:1"}
        objective = {**pair, "minimize": True}
        numeric, measured, explicit_null = checker.pad_distance_checks([
            {**pair, "minimize": False, "max_mm": 6}, objective,
            {**objective, "max_mm": None}], board)
        self.assertEqual(measured, explicit_null)
        self.assertEqual(measured["kind"], "pad_near_objective")
        self.assertEqual(measured["distance_mm"], numeric["distance_mm"])
        self.assertTrue(measured["measured"])
        self.assertFalse(numeric["satisfied"])
        self.assertFalse({"satisfied", "max_mm", "excess_mm", "within_placer_tolerance"} & measured.keys())
        self.assertEqual(checker.missed_constraints([numeric, measured]), [numeric])

    def test_near_requires_an_explicit_objective_or_legacy_limit(self):
        pair = {"part": "J2", "pad": "1", "pin_of": "J1:1"}
        for extra, message in (
            ({}, "near max_mm"),
            ({"minimize": False}, "near max_mm"),
            ({"max_mm": None}, "near max_mm"),
            ({"minimize": 1}, "must be a Boolean"),
            ({"minimize": True, "max_mm": 0}, "cannot be combined"),
            ({"minimize": True, "max_mm": 6}, "cannot be combined"),
        ):
            with self.subTest(extra=extra), self.assertRaisesRegex(ValueError, message):
                checker.pad_distance_checks([{**pair, **extra}], None)
        with self.assertRaisesRegex(ValueError, "pad, pin_of"):
            checker.pad_distance_checks([{"part": "C1", "minimize": True, "pin_of": "J1:2"}], None)

    def test_distinct_pads_and_coincident_pad_shapes(self):
        try:
            import pcbnew  # noqa: F401
        except ImportError:
            self.skipTest("native pad measurements require pcbnew")
        board = run.ROOT / "tests/fixtures/f1/f1.kicad_pcb"
        # J2:1 has two coincident copper shapes at (27.5, 19). J1:1 and
        # J1:2 are at (2.5, 5) and (2.5, 7.54), respectively.
        entries = [{"part": "J2", "pad": "1", "pin_of": pin, "max_mm": 28} for pin in ("J1:1", "J1:2")]
        a, b = checker.pad_distance_checks(entries, board)
        self.assertAlmostEqual(a["distance_mm"], (25**2 + 14**2)**0.5, places=6)
        self.assertAlmostEqual(b["distance_mm"], (25**2 + 11.46**2)**0.5, places=6)
        self.assertFalse(a["satisfied"])
        self.assertTrue(b["satisfied"])
        close = {**entries[0], "max_mm": a["distance_mm"] - 0.006}
        measured, = checker.pad_distance_checks([close], board)
        self.assertFalse(measured["satisfied"])
        self.assertTrue(measured["within_placer_tolerance"])
        self.assertAlmostEqual(measured["excess_mm"], 0.006, places=6)
        anchor_pose = {"kind": "place", "part": "J1", "satisfied": True}
        info, action = checker.pad_near_retry(
            [measured, b, anchor_pose], [measured],
            [{"reference": "J1", "locked": True}, {"reference": "J2", "locked": False}],
        )
        self.assertEqual(info, {
            "kind": "refine_pad_near",
            "constraints_missed_indices": [0],
            "placer_config": {"tuning": {"refine_pad_near": True}},
        })
        self.assertIn(f'J2:1 to J1:1 measured {measured["distance_mm"]} mm', action)
        self.assertIn(f'0.006 mm over the {measured["max_mm"]} mm limit', action)
        self.assertIn("save placement_retry.placer_config as a JSON file and pass it to --placer-config", action)
        self.assertIn("merge the same settings into placer in layout.json", action)
        with self.assertRaisesRegex(ValueError, "one pad centre for J1:99"):
            checker.pad_distance_checks([{**entries[0], "pin_of": "J1:99"}], board)

    def test_retry_excludes_fixed_sources_and_non_pad_targets(self):
        missed = {"kind": "near", "part": "J2", "pad": "1", "pin_of": "J1:1",
                  "satisfied": False, "distance_mm": 2.01, "max_mm": 2, "excess_mm": 0.01}
        for footprints, checks, failed in (
            ([{"reference": "J2", "locked": True}], [missed], [missed]),
            ([{"reference": "J2"}], [missed], [missed]),
            ([{"reference": "J2", "locked": False}],
             [missed, {"kind": "place", "part": "J2", "satisfied": False}], [missed]),
            ([{"reference": "J2", "locked": False}],
             [missed], [{"kind": "near", "part": "J2", "pin_of": "J1:1", "satisfied": False}]),
            ([{"reference": "J2", "locked": False}],
             [missed], [{**missed, "pin_of": "J2:2"}]),
            ([{"reference": "J2", "locked": False}], [{**missed, "satisfied": True}], []),
        ):
            with self.subTest(footprints=footprints, checks=checks, failed=failed):
                self.assertIsNone(checker.pad_near_retry(checks, failed, footprints))


RECT = '(gr_rect (start 10 20) (end 40 50) (layer "Edge.Cuts"))'
OUTLINE = {"x": 10, "y": 20, "width": 30, "height": 30}
FRONT_SLOT = '''(footprint "custom:Switch" (layer "F.Cu")
    (property "Reference" "SW2")
    (fp_rect (start -1 2) (end 3 4) (layer "Edge.Cuts")))'''
BACK_SLOT = FRONT_SLOT.replace('F.Cu', 'B.Cu').replace('-1 2', '-1 -2').replace('3 4', '3 -4')
DUMMY_DRC = {"source": "case.kicad_pcb", "coordinate_units": "mm", "kicad_version": "10.0.8",
             "included_severities": ["error", "warning", "exclusion"], "violations": [],
             "unconnected_items": [], "schematic_parity": []}
DESCRIPTION = {"board_id": "case", "outline": {"minimum": [10, 20], "maximum": [40, 50], "from_footprints": False},
               "copper_layers": ["F.Cu", "B.Cu"], "footprints": [
                   {"reference": "SW2", "layer": "B.Cu", "at": [12, 23, 270], "body": [11, 22, 13, 24]}]}


def board(*items):
    return '(kicad_pcb ' + ' '.join(items) + ')'


class GeometryTests(unittest.TestCase):
    def test_duplicate_board_zone_uuids_ignore_names_and_nested_objects(self):
        raw = board('(zone (uuid "a-b") (name "(uuid b-c)"))', '(zone (uuid A-B))',
                    '(footprint "part" (zone (uuid "b-c")))', '(zone (uuid b-c))')
        row = {}
        checker.check_zone_ids(row, checker.children(raw))
        self.assertEqual(row["duplicate_zone_uuids"], {"a-b": 2})
        self.assertTrue(row["next_actions"])
        row = {}
        checker.check_zone_ids(row, checker.children(raw.replace("(uuid A-B)", "(uuid c-d)")))
        self.assertEqual(row, {})

    def test_edge_error_attribution_requires_a_pad_and_its_own_cutout(self):
        def item(uuid):
            return SimpleNamespace(m_Uuid=SimpleNamespace(AsString=lambda: uuid),
                                   GetLayerName=lambda: "Edge.Cuts", GetClass=lambda: "PCB_SHAPE")

        def footprint(ref, pad, edge):
            return SimpleNamespace(GetReference=lambda: ref, Pads=lambda: [item(pad)],
                                   GraphicalItems=lambda: [item(edge)])

        board = SimpleNamespace(GetFootprints=lambda: [footprint("J1", "p1", "e1"), footprint("J2", "p2", "e2")])
        def error(first, second):
            return {"type": "copper_edge_clearance", "severity": "error", "items": [{"uuid": first}, {"uuid": second}]}

        findings = [error("p1", "e1"), error("e2", "p2"), error("p1", "e2"), error("p1", "board-edge"),
                    error("p1", "p1"), dict(error("p1", "e1"), severity="exclusion")]
        row = {}
        checker.explain_footprint_edges(row, findings, board)
        self.assertEqual(row["pad_to_footprint_cutout_errors"], {"J1": 1, "J2": 1})
        self.assertIn("drc.json", row["next_actions"][0])

    def test_zone_fill_can_change_through_pad_connections(self):
        try:
            import pcbnew
        except ImportError:
            self.skipTest("native footprint geometry requires pcbnew")
        board = pcbnew.LoadBoard(str(run.ROOT / "tests/fixtures/f1/f1.kicad_pcb"))
        part = next(item for item in board.GetFootprints() if item.GetReference() == "J1")
        pad = next(item for item in part.Pads() if item.GetNumber() == "1")
        pad.SetRemoveUnconnected(True)
        pad.SetKeepTopBottom(True)
        def signature():
            return checker.normalized_footprints(pcbnew, board, electrical=False)
        original = signature()
        pad.SetZoneLayerOverride(pcbnew.F_Cu, pcbnew.ZLO_FORCE_FLASHED)
        self.assertEqual(original, signature())
        pad.SetKeepTopBottom(False)
        self.assertNotEqual(original, signature())

    def test_polygon_arcs_preserve_curves_and_their_connecting_lines(self):
        forward = "(pts (arc (start 0 0) (mid 1 -1) (end 2 0)) (arc (start 2 3) (mid 1 4) (end 0 3)))"
        reverse = "(pts (arc (start 0 3) (mid 1 4) (end 2 3)) (arc (start 2 0) (mid 1 -1) (end 0 0)))"
        contour = lambda pts: sorted(checker.polygon_contour(pts, float))
        shape = contour(forward)
        self.assertEqual([item[0] for item in shape], ["arc", "arc", "line", "line"])
        self.assertEqual(shape, contour(reverse))
        self.assertNotEqual(shape, contour(forward.replace("(mid 1 4)", "(mid 1 5)")))
        mixed = "(pts (xy 0 0) (xy 2 0) (arc (start 2 1) (mid 1 2) (end 0 1)))"
        self.assertEqual(len(contour(mixed)), 4)
        with self.assertRaises(ValueError):
            contour(mixed.replace("(mid 1 2)", "(mid 0 1)"))

    def test_pad_centres_and_layers_after_kicad_saves_a_two_layer_board(self):
        try:
            import pcbnew
        except ImportError:
            self.skipTest("native footprint geometry requires pcbnew")
        fixture = (run.ROOT / "tests/fixtures/f1/f1.kicad_pcb").read_text()
        with tempfile.TemporaryDirectory() as tmp:
            path = Path(tmp) / "board.kicad_pcb"

            def signature(x, layers, smd_copper="F.Cu"):
                text = fixture.replace('(pad "1" thru_hole rect (at 0 0)',
                                       f'(pad "1" thru_hole rect (at {x} -4.2)', 1)
                text = text.replace('(layers "*.Cu" "*.Mask")', f'(layers {layers} "B.Mask")', 1)
                path.write_text(text.replace('(layers "F.Cu" "F.Mask" "F.Paste")',
                                             f'(layers "{smd_copper}" "F.Mask" "F.Paste")', 1))
                board = pcbnew.LoadBoard(str(path))
                return checker.normalized_footprints(pcbnew, board, electrical=False)

            original = signature("5.22", '"F.Cu" "B.Cu" "In1.Cu"')
            self.assertEqual(checker.footprint_differences(original, signature("5.220001", '"*.Cu"')), [])
            for x, smd_copper, changed_ref in [("5.24", "F.Cu", "J1"), ("5.22", "B.Cu", "U2")]:
                with self.subTest(x=x, smd_copper=smd_copper):
                    differences = checker.footprint_differences(original, signature(x, '"*.Cu"', smd_copper))
                    self.assertEqual([(d["reference"], d["pad"]) for d in differences], [(changed_ref, "1")])

    def test_native_donor_geometry_follows_pose_and_detects_pad_and_body_edits(self):
        try:
            import pcbnew
        except ImportError:
            self.skipTest("native footprint geometry requires pcbnew")
        path = str(run.ROOT / "tests/fixtures/f1/f1.kicad_pcb")
        source, candidate = pcbnew.LoadBoard(path), pcbnew.LoadBoard(path)
        expected = checker.normalized_footprints(pcbnew, source, electrical=False)
        part = next(item for item in candidate.GetFootprints() if item.GetReference() == "J1")
        part.SetPosition(pcbnew.VECTOR2I(pcbnew.FromMM(23), pcbnew.FromMM(12)))
        part.SetOrientationDegrees(29)
        part.Flip(part.GetPosition(), pcbnew.FLIP_DIRECTION_TOP_BOTTOM)
        pad = next(item for item in part.Pads() if item.GetNumber() == "1")
        pad.SetPinFunction("Changed by the schematic")
        pad.SetPinType("no_connect")

        def changes():
            return checker.footprint_differences(expected, checker.normalized_footprints(pcbnew, candidate, electrical=False))

        self.assertEqual(changes(), [])
        size = pad.GetSize()
        pad.SetSize(pcbnew.VECTOR2I(size.x + pcbnew.FromMM(0.02), size.y))
        body = next(item for item in part.GraphicalItems() if item.GetLayer() == pcbnew.B_CrtYd)
        body.SetWidth(body.GetWidth() + pcbnew.FromMM(0.01))
        self.assertEqual([(item["reference"], item.get("pad") or item["difference"]) for item in changes()],
                         [("J1", "body"), ("J1", "1")])

    def test_repeated_references_compare_physical_occurrences(self):
        try:
            import pcbnew
        except ImportError:
            self.skipTest("native footprint geometry requires pcbnew")
        path = str(run.ROOT / "tests/fixtures/f1/f1.kicad_pcb")

        def board_with_repeated_reference():
            board = pcbnew.LoadBoard(path)
            parts = {part.GetReference(): part for part in board.GetFootprints()}
            for ref in ("J1", "U2"):
                parts[ref].SetReference("G***")
            return board, parts

        source, _ = board_with_repeated_reference()
        candidate, parts = board_with_repeated_reference()
        signatures = lambda board: checker.normalized_footprints(pcbnew, board, electrical=False)
        expected = signatures(source)
        first, second = parts["J1"], parts["U2"]
        first_at, second_at = first.GetPosition(), second.GetPosition()
        first.SetPosition(second_at)
        second.SetPosition(first_at)
        self.assertEqual(checker.footprint_differences(expected, signatures(candidate)), [])

        pad = next(item for item in first.Pads() if item.GetNumber() == "1")
        size = pad.GetSize()
        pad.SetSize(pcbnew.VECTOR2I(size.x + pcbnew.FromMM(0.02), size.y))
        changed, = checker.footprint_differences(expected, signatures(candidate))
        self.assertEqual((changed["reference"], changed["pad"]), ("G***", "1"))
        self.assertEqual(changed["source_occurrence"]["uuid"], first.m_Uuid.AsString())
        self.assertEqual(changed["candidate_occurrence"]["uuid"], first.m_Uuid.AsString())
        pad.SetSize(size)

        candidate.Remove(second)
        count, missing = checker.footprint_differences(expected, signatures(candidate))
        self.assertEqual(count, {"reference": "G***", "difference": "count", "source_count": 2, "candidate_count": 1})
        self.assertEqual(missing["difference"], "missing")
        self.assertIn("uuid", missing["source_occurrence"])

        candidate, parts = board_with_repeated_reference()
        copied = pcbnew.FOOTPRINT(parts["J1"])
        candidate.Add(copied)
        count, extra = checker.footprint_differences(expected, signatures(candidate))
        self.assertEqual(count["candidate_count"], 3)
        self.assertEqual(extra["difference"], "extra")
        uuid = parts["J1"].m_Uuid.AsString()
        original = next(item for item in expected["G***"]["occurrences"] if item["uuid"] == uuid)
        self.assertEqual(extra["candidate_occurrence"]["signature_sha256"],
                         checker.occurrence_evidence(original)["signature_sha256"])

        same_model = dict(original, uuid="other-copy")
        expected = {"G***": {"occurrences": [original, same_model]}}
        result = {"G***": {"occurrences": [same_model]}}
        _, missing = checker.footprint_differences(expected, result)
        self.assertEqual(missing["source_occurrence"]["uuid"], original["uuid"])

    def test_outline_primitives_ignore_direction_order_metadata_and_rect_encoding(self):
        polygon = '(gr_poly (pts (xy 40 50) (xy 40 20) (xy 10 20) (xy 10 50)) (layer Edge.Cuts))'
        reversed_lines = ' '.join([
            '(gr_line (start 10.0 50) (end 40 50) (layer Edge.Cuts) (uuid "ignored"))',
            '(gr_line (start 10 20) (end 10 50) (layer Edge.Cuts))',
            '(gr_line (start 40 20) (end 10 20) (layer Edge.Cuts))',
            '(gr_line (start 40 50) (end 40 20) (layer Edge.Cuts))',
        ])
        rect = checker.board_outline(checker.children(board(RECT)))
        self.assertEqual(checker.board_outline(checker.children(board(polygon))), rect)
        self.assertEqual(checker.board_outline(checker.children(board(reversed_lines))), rect)
        hole = '(gr_circle (center 15 25) (end 16 25) (layer Edge.Cuts))'
        self.assertNotEqual(checker.board_outline(checker.children(board(RECT, hole))), rect)
        unexpected = '(dimension (pts (xy 1 1) (xy 2 2)) (layer "Edge.Cuts"))'
        with self.assertRaisesRegex(ValueError, 'unsupported Edge.Cuts item dimension'):
            checker.board_outline(checker.children(board(RECT, unexpected)))
        with self.assertRaisesRegex(ValueError, 'unsupported Edge.Cuts item zone'):
            checker.board_outline(checker.children(board(RECT, '(zone (layers F.Cu Edge.Cuts))')))

    def test_donor_and_explicit_outline_must_agree(self):
        with tempfile.TemporaryDirectory() as folder:
            donor = Path(folder) / 'donor.kicad_pcb'
            # A donor footprint may carry unrelated geometry. The fixture
            # only provides board-level Edge.Cuts to the brief.
            donor.write_text(board(RECT, FRONT_SLOT.replace('SW2', 'X100')))
            self.assertEqual(checker.expected_outline({"outline": OUTLINE}, donor),
                             checker.expected_outline({"outline": OUTLINE}, None))
            with self.assertRaisesRegex(ValueError, 'disagrees'):
                checker.expected_outline({"outline": dict(OUTLINE, width=31)}, donor)

    def test_footprint_slot_is_compared_in_its_library_frame(self):
        front = checker.footprint_cutouts(checker.children(board(FRONT_SLOT)))
        self.assertIn('SW2', front)
        self.assertEqual(checker.footprint_cutouts(checker.children(board(BACK_SLOT))), front)
        self.assertNotEqual(checker.footprint_cutouts(checker.children(board(FRONT_SLOT.replace('3 4', '4 4')))), front)

    def test_pose_side_and_body_region_have_separate_evidence(self):
        brief = {"version": 1, "move_all": True, "place": [{"part": "SW2", "x": 2, "y": 3, "angle": -90}],
                 "rotation": [{"part": "SW2", "angles": [0, 270]}], "back": ["SW?"],
                 "region": [{"parts": ["SW*"], "x": [1, 3], "y": [2, 4]}]}
        self.assertTrue(all(check['satisfied'] for check in checker.checks_for(brief, DESCRIPTION)))
        brief['region'][0]['x'] = [2, 3]
        brief['front'] = ['SW2']
        self.assertEqual([row['kind'] for row in checker.checks_for(brief, DESCRIPTION) if not row['satisfied']],
                         ['front', 'region'])
        with self.assertRaisesRegex(ValueError, 'unsupported placement constraint kinds: edge'):
            checker.checks_for(dict(brief, edge=[]), DESCRIPTION)
        with self.assertRaisesRegex(ValueError, 'matches no candidate footprint'):
            checker.checks_for(dict(brief, front=['J?']), DESCRIPTION)


class VerificationTests(unittest.TestCase):
    def test_parity_diagnosis_matches_both_reference_and_pin(self):
        report = {"schematic_parity": [
            {"type": "net_conflict", "description": "No pad found for pin 17 in schematic",
             "items": [{"description": "Footprint U1"}]},
            {"type": "missing_footprint", "description": "Footprint J6 is missing"}]}
        pins = [{"reference": ref, "pin": "17", "connection": "no_connect", "next_action": "verify PCB pads"}
                for ref in ("U1", "U2")]
        row = {}
        checker.explain_parity(row, report, {"unmatched_pin_details": pins}, "drc.json")
        self.assertEqual(row["unmatched_pin_details"], pins[:1])
        self.assertIn("no-connect", row["next_actions"][0])
        self.assertIn("PCB", row["next_actions"][0])
        self.assertIn("drc.json", row["next_actions"][1])

    def test_uses_original_context_stages_no_candidate_settings_and_rejects_deleted_slot(self):
        with tempfile.TemporaryDirectory() as folder:
            root = Path(folder)
            schematic = root / 'case.kicad_sch'
            schematic.write_text('(kicad_sch (sheet (property "Sheetfile" "sheets/child.kicad_sch")))')
            child = root / 'sheets/child.kicad_sch'
            child.parent.mkdir()
            child.write_text('(kicad_sch)')
            settings = '{"source":"original"}'
            rules = '(version 1)\n(rule "original" (constraint clearance (min 0.2mm)))'
            schematic.with_suffix('.kicad_pro').write_text(settings)
            schematic.with_suffix('.kicad_dru').write_text(rules)
            constraints = root / 'constraints.json'
            constraints.write_text(json.dumps({"version": 1, "outline": OUTLINE, "back": ["SW2"]}))
            returned = root / 'submission/returned.kicad_pcb'
            returned.parent.mkdir()
            returned.with_suffix('.kicad_pro').write_text('{}')
            returned.with_suffix('.kicad_dru').write_text('candidate custom rules')
            binary = Path(__file__).resolve()
            arguments = SimpleNamespace(schematic=schematic, constraints=constraints, outline_board=None,
                                        binary=binary, candidate_board=returned, timeout=30)
            checks = []

            def invoke(command, **options):
                checks.append(command[1])
                if command[1] == 'describe-kicad-board':
                    staged = Path(command[2])
                    self.assertEqual((staged / 'case.kicad_pro').read_text(), settings)
                    self.assertEqual((staged / 'case.kicad_dru').read_text(), rules)
                    self.assertEqual((staged / 'sheets/child.kicad_sch').read_bytes(), child.read_bytes())
                    self.assertEqual((staged / 'case.kicad_pcb').read_bytes(), returned.read_bytes())
                    return subprocess.CompletedProcess(command, 0, json.dumps(DESCRIPTION), '')
                if command[1] == 'import-kicad-netlist':
                    donor = getattr(arguments, 'footprints_from', None)
                    if donor is None:
                        self.assertNotIn('--footprints-from', command)
                    else:
                        staged_donor = Path(command[command.index('--footprints-from') + 1])
                        self.assertNotEqual(staged_donor, returned)
                        self.assertEqual(staged_donor.read_bytes(), donor.read_bytes())
                    reference = Path(command[3])
                    reference.mkdir()
                    (reference / 'case.kicad_pcb').write_text(board(FRONT_SLOT))
                    (reference / 'case.net').write_text('(export)')
                    return subprocess.CompletedProcess(command, 0, '{"footprints":1,"missing":[]}', '')
                self.assertEqual(command[1:3], ['pcb', 'drc'])
                self.assertIn('--schematic-parity', command)
                self.assertIn('--all-track-errors', command)
                self.assertNotIn('--save-board', command)
                report = Path(command[command.index('--output') + 1])
                native = dict(DUMMY_DRC)
                if report.name == 'drc-as-saved.json':
                    self.assertNotIn('--refill-zones', command)
                    native['unconnected_items'] = [{'type': 'unconnected_items', 'severity': 'error'}]
                if report.parent.name == 'unfilled' and report.name == 'drc.json':
                    self.assertIn('--refill-zones', command)
                report.write_text(json.dumps(native))
                return subprocess.CompletedProcess(command, 0, 'Found 0 schematic parity issues\n', '')

            with patch.object(checker.harness_common, 'run_group', side_effect=invoke):
                returned.write_text(board(RECT, BACK_SLOT))
                first = root / 'build/first'
                first.mkdir(parents=True)
                accepted = checker.verify(arguments, schematic, first, 2)
                returned.write_text(board(RECT, '(footprint "custom:Switch" (layer B.Cu) (property "Reference" "SW2"))'))
                second = root / 'build/second'
                second.mkdir()
                rejected = checker.verify(arguments, schematic, second, 2)
                donor = root / 'source-donor.kicad_pcb'
                donor.write_text(board(FRONT_SLOT, '(segment (start 10 10) (end 20 20))'))
                arguments.footprints_from = donor
                returned.write_text(board(RECT, BACK_SLOT))
                third = root / 'build/donated-geometry'
                third.mkdir()
                # The command fixture uses partial boards. The native geometry
                # comparison is exercised separately on a real board fixture.
                bindings = SimpleNamespace(LoadBoard=lambda path: Path(path))
                geometry = {"SW2": {"body": [], "pads": {}}}
                with patch.dict(sys.modules, {"pcbnew": bindings}), patch.object(checker, 'normalized_footprints', return_value=geometry):
                    donor_result = checker.verify(arguments, schematic, third, 2)
                arguments.footprints_from = None
                returned.write_text(board(RECT, BACK_SLOT, '(zone (layer F.Cu))'))
                fourth = root / 'build/unfilled'
                fourth.mkdir()
                unfilled = checker.verify(arguments, schematic, fourth, 2)
            self.assertEqual(accepted['status'], 'accepted', accepted)
            self.assertEqual(rejected['status'], 'rejected', rejected)
            self.assertEqual(rejected['footprint_cutouts']['different_parts'], ['SW2'])
            self.assertEqual(accepted['brief_sha256'], rejected['brief_sha256'])
            self.assertNotEqual(accepted['candidate_board']['sha256'], rejected['candidate_board']['sha256'])
            self.assertEqual(donor_result['status'], 'accepted', donor_result)
            self.assertTrue(donor_result['footprint_geometry']['satisfied'])
            self.assertEqual(donor_result['inputs']['footprints_from'], checker.record(donor))
            self.assertNotEqual(accepted['brief_sha256'], donor_result['brief_sha256'])
            self.assertEqual(unfilled['status'], 'rejected', unfilled)
            self.assertEqual(unfilled['as_saved']['unconnected'], 1)
            self.assertEqual(unfilled['unconnected'], 0)
            self.assertIn('save the PCB', unfilled['next_actions'][0])
            self.assertEqual(checks, ['describe-kicad-board', 'import-kicad-netlist', 'pcb'] * 4 + ['pcb'])

    def test_missing_malformed_and_unconfirmed_native_evidence(self):
        for field in ('violations', 'unconnected_items', 'schematic_parity'):
            for value in (None, [{'type': 'unknown', 'severity': '???'}]):
                with self.subTest(field=field, value=value), self.assertRaisesRegex(ValueError, field):
                    checker.native_counts(dict(DUMMY_DRC, **{field: value}), 'case.kicad_pcb',
                                          'Found 0 schematic parity issues\n')
        with self.assertRaisesRegex(ValueError, 'did not confirm'):
            checker.native_counts(DUMMY_DRC, 'case.kicad_pcb', '')
        with self.assertRaisesRegex(ValueError, 'skipped the unconnected-items check'):
            checker.native_counts(dict(DUMMY_DRC, ignored_checks=[{"key": "unconnected_items"}]),
                                  'case.kicad_pcb', 'Found 0 schematic parity issues')
        with self.assertRaisesRegex(ValueError, 'unbalanced'):
            checker.check_rule_input('(version 1)\n(rule "missing close"')
        with self.assertRaisesRegex(ValueError, 'custom rule input problem'):
            checker.check_rule_transcript('Error loading original custom rules')

    def test_native_exclusions_preserve_active_and_excluded_counts(self):
        error = {"type": "example", "severity": "error"}
        findings = [error, dict(error, severity="warning", excluded=False),
                    dict(error, excluded=True), dict(error, severity="exclusion")]
        report = dict(DUMMY_DRC, violations=findings, unconnected_items=findings, schematic_parity=findings)
        counts = checker.native_counts(report, 'case.kicad_pcb', 'Found 4 schematic parity issues')
        self.assertEqual(
            [counts[key] for key in ("drc_errors", "drc_warnings", "drc_exclusions", "unconnected",
                                     "unconnected_exclusions", "parity", "parity_exclusions")],
            [1, 1, 2, 2, 2, 2, 2])
        report["violations"] = [dict(error, excluded="true")]
        with self.assertRaisesRegex(ValueError, "malformed KiCad violations"):
            checker.native_counts(report, 'case.kicad_pcb', 'Found 4 schematic parity issues')


if __name__ == '__main__':
    unittest.main()
