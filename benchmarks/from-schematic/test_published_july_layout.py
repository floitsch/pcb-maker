# Copyright (C) 2026 Florian Loitsch.

import importlib.util
import json
import subprocess
import tempfile
import unittest
from pathlib import Path
from types import SimpleNamespace
from unittest.mock import patch

spec = importlib.util.spec_from_file_location("published_july_layout_runner", Path(__file__).with_name("published-july-layout.py"))
july = importlib.util.module_from_spec(spec)
spec.loader.exec_module(july)


class JulyLayoutTests(unittest.TestCase):
    def test_rail_drop_refuses_an_unpinned_simulator_and_keeps_scoring_independent(self):
        with tempfile.TemporaryDirectory() as work:
            path = Path(work) / "simulator.py"
            path.write_text("raise AssertionError('must never be imported')\n")
            with self.assertRaisesRegex(ValueError, july.july_rail_drop.SIMULATOR_BLOB):
                july.july_rail_drop.load_simulator(path)
            self.assertEqual(july.july_rail_drop.score_board(None, path, None, None, "ngspice missing"),
                             {"status": "unavailable", "reason": "ngspice missing"})
            parser = SimpleNamespace(LoadBoard=lambda _: object())
            with patch.object(july.july_rail_drop, "simulate", side_effect=ValueError("unsupported copper")):
                self.assertEqual(july.july_rail_drop.score_board(parser, path, path.read_bytes(), object(), None),
                                 {"status": "unavailable", "reason": "unsupported copper"})
                mismatch = july.july_rail_drop.score_board(parser, path, b"scored PCB", object(), None)
                self.assertIn("does not match the scored bytes", mismatch["reason"])

    def test_rail_drop_checks_native_trace_nets_in_the_author_component(self):
        # The author graph names pads and builds resistors from the track
        # geometry. A copper item can have a foreign net and no foreign pad.
        points = [(0.0, 0.0, "F.Cu"), (1.0, 0.0, "F.Cu")]
        nodes = SimpleNamespace(pts=points, find=lambda index: index)
        graph = nodes, [(0, 1, 0.01)], [("J1", "+12V", 0), ("U4", "+12V", 1)], {}
        author = SimpleNamespace(SOURCES={"+12V": "J1"}, _build_graph=lambda *args: graph,
                                 _component=lambda *args: {0, 1})
        with patch.object(july.july_rail_drop, "_native_input",
                          return_value=(None, [("different-track", "GND", 0, points)], [])):
            with self.assertRaisesRegex(ValueError, "unexpected track or via nets.*different-track"):
                july.july_rail_drop.simulate(None, None, author)

    def test_rail_drop_requires_every_load_and_a_transient_on_12v(self):
        results = {"loads_a": {"U4": 0.65}, "unreached_loads": [], "foreign_nets_on_copper": [],
                   "ir_drop_mv": {"U4": 10.2}, "v_supply_pad": 12.0,
                   "worst_load": "U4", "worst_drop_mv": 10.2,
                   "transient": {"rail_min_v": 11.9898, "rail_max_v": 12.0, "droop_mv": 10.2}}
        check = july.july_rail_drop._require_complete
        check("+12V", results, {"U4": 0.65})
        for patch_values, reason in (
            ({"unreached_loads": ["U4"]}, "did not reach"),
            ({"ir_drop_mv": {}}, "operating-point voltages"),
            ({"ir_drop_mv": {"U4": float("nan")}}, "operating-point voltages"),
            ({"transient": {}}, "transient droop"),
        ):
            with self.subTest(reason=reason), self.assertRaisesRegex(ValueError, reason):
                check("+12V", dict(results, **patch_values), {"U4": 0.65})

    def test_archive_scoring_stages_exact_project_and_runs_both_native_modes(self):
        with tempfile.TemporaryDirectory() as work:
            directory = Path(work) / "candidate"
            destination = directory / "test.kicad_pcb"
            native_env = {"KICAD_CONFIG_HOME": str(Path(work) / "archive-config")}
            calls = []

            def run(command, log, env, timeout):
                self.assertEqual(destination.read_bytes(), b"pinned PCB")
                self.assertEqual(destination.with_suffix(".kicad_sch").read_bytes(), b"pinned schematic")
                self.assertEqual(destination.with_suffix(".kicad_pro").read_bytes(), b"matched project rules")
                self.assertEqual(command[-1], destination)
                self.assertEqual(env, native_env)
                calls.append(command[1:-3])
                command[-2].write_text(json.dumps({
                    "source": str(destination), "coordinate_units": "mm", "kicad_version": "10.0.8",
                    "included_severities": ["error", "warning", "exclusion"], "violations": [],
                    "unconnected_items": [], "schematic_parity": [],
                }))
                return subprocess.CompletedProcess(command, 0, "Found 0 schematic parity issues\n", "")

            with patch.object(july.common, "run_command", side_effect=run):
                native, parity = july.score_archive(directory, "test", b"pinned schematic", b"pinned PCB",
                                                     b"matched project rules", "kicad-cli", native_env, 10)
            self.assertEqual(calls, list(july.common.NATIVE_CHECKS.values()))
            self.assertEqual(parity, {"as_saved": {}, "refilled": {}})
            self.assertTrue(all(result["kicad_checked"] for result in native.values()))

    def test_near_replay_modes_keep_all_footprints_movable(self):
        body = july.placement_constraints(False, False, None)
        historical = july.placement_constraints(True, False, None)
        objective = july.placement_constraints(True, True, None)
        diagnostic = july.placement_constraints(True, False, 6)
        for brief in (body, historical, objective, diagnostic):
            self.assertTrue(brief["move_all"])
            self.assertEqual(brief["front"], ["*"])
            self.assertNotIn("place", brief)
            self.assertNotIn("fixed", brief)
        for legacy, goal, limit in zip(historical["near"], objective["near"], diagnostic["near"]):
            self.assertEqual(legacy, {**limit, "max_mm": 0})
            self.assertEqual(goal, {k: v for k, v in legacy.items() if k != "max_mm"} | {"minimize": True})
        for value in (0, -6, float("inf"), float("nan")):
            with self.assertRaisesRegex(ValueError, "positive finite"):
                july.placement_constraints(True, False, value)
        for mode in ((True, True, 6), (False, True, None), (False, False, 6)):
            with self.assertRaisesRegex(ValueError, "choose one mode"):
                july.placement_constraints(*mode)

    def test_pad_copper_gap_is_measured_only_on_shared_copper(self):
        try:
            import pcbnew
        except ImportError:
            self.skipTest("native pad geometry requires pcbnew")
        board = pcbnew.BOARD()
        footprint = pcbnew.FOOTPRINT(board)
        board.Add(footprint)

        def pad(x, layer):
            obj = pcbnew.PAD(footprint)
            obj.SetAttribute(pcbnew.PAD_ATTRIB_SMD)
            obj.SetShape(pcbnew.PAD_SHAPE_RECT)
            obj.SetSize(pcbnew.VECTOR2I(pcbnew.FromMM(1), pcbnew.FromMM(1)))
            obj.SetPosition(pcbnew.VECTOR2I(pcbnew.FromMM(x), 0))
            layers = pcbnew.LSET()
            layers.AddLayer(layer)
            obj.SetLayerSet(layers)
            footprint.Add(obj)
            return obj

        a = pad(0, pcbnew.F_Cu)
        b = pad(1.5, pcbnew.F_Cu)
        self.assertEqual(july.pad_copper_contact(pcbnew, board, a, b), {
            "F.Cu": {"status": "measured", "copper_gap_mm": 0.5, "contact": False}})
        c = pad(0.9, pcbnew.F_Cu)
        self.assertTrue(july.pad_copper_contact(pcbnew, board, a, c)["F.Cu"]["contact"])
        self.assertEqual(july.pad_copper_contact(pcbnew, board, a, pad(0, pcbnew.B_Cu)), {})

    def test_adaptation_changes_only_layers_and_stack_metadata(self):
        initial = b'''(kicad_pcb (version 20221018)
            (layers (0 "F.Cu" signal) (31 "B.Cu" signal) (44 "Edge.Cuts" user))
            (setup (pad_to_mask_clearance 0.0))
            (footprint "R_0603" (at 21 30 90) (pad "1" thru_hole circle (net 1 "GND")))
            (gr_line (start 0 0) (end 10 0) (layer "Edge.Cuts")))'''
        stack = '(stackup (layer "In1.Cu" (type "copper")))'
        before = dict(july.children(initial.decode()))
        after = dict(july.children(july.adapt_starter(initial, stack).decode()))
        self.assertEqual(set(before), set(after))
        self.assertEqual({name for name in before if before[name] != after[name]}, {"layers", "setup"})
        self.assertIn('(1 "In1.Cu" signal)', after["layers"])
        self.assertIn('(2 "In2.Cu" signal)', after["layers"])
        self.assertIn(stack, after["setup"])

    def test_schematic_net_comparison_preserves_all_routed_terminal_names(self):
        archived = {"pad_nets": [("J1", "1", ""), ("C1", "1", "+12V"), ("C1", "2", "GND")]}
        imported = {"pad_nets": [("C1", "1", "+12V"), ("J1", "1", "unconnected-(J1-Pad1)"),
                                 ("C1", "2", "GND")]}
        self.assertEqual(july.routed_pad_names(archived), july.routed_pad_names(imported))
        self.assertNotEqual(july.routed_pad_names(archived),
                            july.routed_pad_names({"pad_nets": imported["pad_nets"][:-1]}))
        imported["pad_nets"][-1] = ("C1", "2", "unconnected-(C1-Pad2)")
        self.assertNotEqual(july.routed_pad_names(archived), july.routed_pad_names(imported))

    def test_exact_pin_mapping_differences_are_reported_per_reference_and_pin(self):
        s = {"J1": {"library": "lib:shape", "body": [], "pads": {
            "2": [{"geometry": [1, 2], "net": "GND", "function": "gnd"}],
        }}}
        t = {"J1": {"library": "lib:shape", "body": [], "pads": {
            "2": [{"geometry": [1, 2], "net": "CLK", "function": "clk"}],
        }}}
        self.assertEqual(july.common.footprint_differences(s, s), [])
        self.assertEqual(july.common.footprint_differences(s, t), [{"reference": "J1", "pad": "2",
                                                  "source": s["J1"]["pads"]["2"], "candidate": t["J1"]["pads"]["2"]}])
        self.assertNotIn("--refill-zones", july.common.NATIVE_CHECKS["as_saved"])
        self.assertIn("--refill-zones", july.common.NATIVE_CHECKS["refilled"])
        self.assertIn("--schematic-parity", july.common.NATIVE_CHECKS["as_saved"])

    def test_native_courtyard_clearance_uses_vertices_and_the_fab_box_requires_four_edges(self):
        try:
            import pcbnew
        except ImportError:
            self.skipTest("native footprint geometry requires pcbnew")

        board = pcbnew.BOARD()

        def part(fab, courtyard):
            footprint = pcbnew.FOOTPRINT(board)
            footprint.SetLayer(pcbnew.F_Cu)
            board.Add(footprint)
            for layer, polygon in ((pcbnew.F_Fab, fab), (pcbnew.F_CrtYd, courtyard)):
                for start, end in zip(polygon, polygon[1:] + polygon[:1]):
                    line = pcbnew.PCB_SHAPE(footprint)
                    line.SetShape(pcbnew.SHAPE_T_SEGMENT)
                    line.SetLayer(layer)
                    line.SetWidth(pcbnew.FromMM(0.05))
                    line.SetStart(pcbnew.VECTOR2I(*(pcbnew.FromMM(v) for v in start)))
                    line.SetEnd(pcbnew.VECTOR2I(*(pcbnew.FromMM(v) for v in end)))
                    footprint.Add(line)
            return footprint

        a = part([(0, 0), (1, 0), (1, 1), (0, 1)], [(0, 0), (4, 0), (0, 4)])
        b = part([(5, 3), (6, 3), (5, 4)], [(2, 4), (4, 2), (4, 4)])
        self.assertEqual(july.fab_rectangle(pcbnew, a), (0, 0, pcbnew.FromMM(1), pcbnew.FromMM(1)))
        self.assertIsNone(july.fab_rectangle(pcbnew, b))
        spacing = july.package_spacing(pcbnew, a, b)
        self.assertIsNone(spacing["projected_fab_outline_gap_mm"])
        self.assertEqual(spacing["projected_courtyard_aabb_gap_mm"], 0)
        self.assertGreater(spacing["projected_courtyard_gap_mm"], 1)
        self.assertFalse(spacing["projected_courtyards_collide"])
