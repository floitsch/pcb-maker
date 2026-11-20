# Copyright (C) 2026 Florian Loitsch.

"""Run with python3 -m unittest discover -s benchmarks/from-schematic."""

import importlib.util
import io
import json
import subprocess
import sys
import tempfile
import unittest
from contextlib import redirect_stderr, redirect_stdout
from pathlib import Path
from types import SimpleNamespace
from unittest.mock import patch

spec = importlib.util.spec_from_file_location("from_schematic_runner", Path(__file__).with_name("run.py"))
run = importlib.util.module_from_spec(spec)
spec.loader.exec_module(run)


def write_layout_drc(directory, board="(kicad_pcb)"):
    native = {
        "violations": [], "unconnected_items": [], "schematic_parity": [], "ignored_checks": [],
        "source": "case.kicad_pcb", "kicad_version": "10.0.8",
        "included_severities": ["error", "warning", "exclusion"],
    }
    (directory / "case.kicad_pcb").write_text(board)
    (directory / "drc.json").write_text(json.dumps(native))
    return native


class MechanicalResults(unittest.TestCase):
    def test_only_explicit_pad_near_uses_the_strict_flag(self):
        satisfied = run.harness_common.constraint_satisfied
        status = {"kind": "near", "satisfied": True, "violation_mm": 0}
        self.assertTrue(satisfied(status))
        self.assertFalse(satisfied(dict(status, satisfied=False)))
        self.assertFalse(satisfied(dict(status, pad_near={"satisfied": False})))
        self.assertTrue(satisfied(dict(status, pad_near={"satisfied": True})))
        self.assertTrue(satisfied(dict(status, kind="back", pad_near={"satisfied": False})))
        self.assertFalse(satisfied(dict(status, pad_near={})))

    def test_incomplete_placement_preserves_actions_and_attempted_constraint_evidence(self):
        with tempfile.TemporaryDirectory() as temporary:
            work = Path(temporary)
            schematic = work / "case.kicad_sch"
            schematic.touch()
            constraints = work / "constraints.json"
            constraints.write_text("{}")
            router_config = work / "outer-layer pours.json"
            router_config.write_text('{"add_pours":[{"net":"GND","layers":["F.Cu","B.Cu"]}]}')
            donor = work / "donor.kicad_pcb"
            donor.write_text("(kicad_pcb (footprint fixture (at 50 60)))")
            arguments = SimpleNamespace(
                schematic=schematic,
                constraints=constraints,
                binary=Path(__file__),
                outline_board=None,
                total_seconds=None,
                router_config=router_config,
                footprints_from=donor,
                timeout=10,
            )
            placed = work / "layout/placed"
            placed.mkdir(parents=True)
            checks = [{"kind": "place", "part": "J1", "satisfied": True, "violation_mm": 0.0}]
            (placed / "board-placer.json").write_text(json.dumps({"constraints": checks}))
            verdict = {
                "status": "incomplete",
                "kicad_checked": False,
                "reason": "U1 has no legal place",
                "unplaced": ["U1"],
                "constraints_missed": [],
                "room": {"utilization": [1.053, 0.0], "suggested_outline_mm": [42.5, 42.5]},
                "next_actions": ["Move components to the back."],
            }
            (placed.parent / "verdict.json").write_text(json.dumps(verdict))
            commands = [
                subprocess.CompletedProcess([], 0, '{"footprints": 2, "missing": []}', ""),
                subprocess.CompletedProcess([], 1, "layout stopped"),
            ]
            staged_donors = []

            def execute(command, **kwargs):
                if command[1] == "import-kicad-netlist":
                    staged = Path(command[command.index("--footprints-from") + 1])
                    self.assertNotEqual(staged, donor)
                    self.assertEqual(staged.read_bytes(), donor.read_bytes())
                    staged_donors.append(staged)
                return commands.pop(0)

            with patch.object(run.harness_common, "run_group", side_effect=execute) as invoke:
                summary = run.run_board(arguments, schematic, "case", work, 6)
            self.assertEqual(invoke.call_args_list[1].args[0][-2], str(router_config))
            self.assertEqual(summary["router_config"], run.returned_board.record(router_config))
            self.assertEqual(summary["inputs"]["footprints_from"], run.returned_board.record(donor))
            self.assertEqual(len(staged_donors), 1)
            self.assertFalse(staged_donors[0].exists())
        self.assertFalse(summary["pass"])
        self.assertFalse(summary["kicad_checked"])
        self.assertEqual(summary["room"], verdict["room"])
        self.assertEqual(summary["next_actions"], verdict["next_actions"])
        self.assertEqual(summary["unplaced"], ["U1"])
        self.assertEqual(summary["placement_report"], "layout/placed/board-placer.json")
        self.assertEqual(summary["placement_constraints"], checks)
        self.assertNotIn("constraints_missed", summary)
        self.assertNotIn("layout_config", summary)

    def test_layout_applies_and_reports_placer_options(self):
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary)
            output = root / "run"
            schematic = root / "case.kicad_sch"
            schematic.touch()
            brief = root / "brief.json"
            brief.write_text("{}\n")
            placer = root / "placer.json"
            placer.write_text('{ "decoupling_near": true }\n')
            layout_config = output / "project/layout.json"

            def execute(command, **kwargs):
                if command[1] == "import-kicad-netlist":
                    layout_config.write_text('{"placer":{"constraints":"constraints.json","constraint_weight":3},"polish_seconds":4}')
                    return subprocess.CompletedProcess([], 0, '{"footprints": 2, "missing": []}', "")
                self.assertEqual(command[-1], str(layout_config))
                self.assertEqual(json.loads(layout_config.read_text()), {
                    "placer": {"constraints": "constraints.json", "constraint_weight": 3, "decoupling_near": True},
                    "polish_seconds": 4, "total_seconds": 25,
                })
                self.assertEqual((output / "project/constraints.json").read_bytes(), brief.read_bytes())
                layout = output / "layout"
                (layout / "result").mkdir(parents=True)
                write_layout_drc(layout / "result")
                (layout / "verdict.json").write_text('{"status":"complete","kicad_checked":true}')
                (layout / "board-layout.json").write_text(json.dumps({"constraints": [], "routed": {
                    "routed_connections": 1, "routable_connections": 1, "native": {"schematic_parity_issues": 0},
                }}))
                return subprocess.CompletedProcess([], 0, "layout complete")

            argv = ["run.py", str(output), "--schematic", str(schematic), "--constraints", str(brief), "--layers", "2",
                    "--binary", __file__, "--total-seconds", "25", "--placer-config", str(placer)]
            seconds = (1, 2, 6, 8, 18, 19)
            times = [second * 10 ** 9 for second in seconds]
            with (patch.object(sys, "argv", argv),
                  patch.object(run.harness_common, "run_group", side_effect=execute) as invoke,
                  patch.object(run.time, "monotonic_ns", side_effect=times),
                  redirect_stdout(io.StringIO())):
                self.assertEqual(run.main(), 0)
            self.assertEqual(invoke.call_count, 2)
            row, = json.loads((output / "results.json").read_text())
            self.assertEqual(row["wall_time_seconds"], {
                "total": 18.0,
                "stages": {
                    "input_preparation": 1.0, "schematic_import": 4.0,
                    "layout_preparation": 2.0, "layout": 10.0, "result_readback": 1.0,
                },
            })
            self.assertEqual(row["placer_config"], run.returned_board.record(placer))
            self.assertNotIn("placer_config", row["inputs"])
            self.assertEqual(row["layout_config"], {"path": "project/layout.json",
                                                    "sha256": run.harness_common.file_sha256(layout_config)})

    def test_import_timeout_records_only_the_work_reached(self):
        with tempfile.TemporaryDirectory() as temporary:
            work = Path(temporary)
            schematic = work / "case.kicad_sch"
            schematic.touch()
            brief = work / "brief.json"
            brief.write_text("{}")
            arguments = SimpleNamespace(schematic=schematic, constraints=brief, outline_board=None,
                                        binary=Path(__file__), timeout=7)
            with (patch.object(run.harness_common, "run_group", side_effect=subprocess.TimeoutExpired("import", 7)),
                  patch.object(run.time, "monotonic_ns", side_effect=[0, 1_000_000_000, 8_000_000_000])):
                row = run.run_board(arguments, schematic, "case", work, 4)
            self.assertEqual(row["error"], "import timed out after 7 s")
            self.assertEqual(row["wall_time_seconds"], {
                "total": 8.0, "stages": {"input_preparation": 1.0, "schematic_import": 7.0},
            })
            self.assertNotIn("seconds", row)

    def test_layout_streams_before_exit_and_preserves_timeout_output(self):
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary)
            schematic, brief = (root / name for name in ("case.kicad_sch", "brief.json"))
            schematic.touch()
            brief.write_text("{}")
            arguments = SimpleNamespace(schematic=schematic, constraints=brief, outline_board=None,
                                        binary=Path(__file__), total_seconds=None, router_config=None,
                                        timeout=1)
            run_child = run.harness_common.run_group
            for wait in (False, True):
                with self.subTest(timeout=wait):
                    output = root / f"run-{wait}"
                    output.mkdir()
                    log_path = output / "layout.log"

                    def execute(command, **options):
                        if command[1] == "import-kicad-netlist":
                            return subprocess.CompletedProcess([], 0, '{"footprints":2,"missing":[]}', "")
                        script = "\n".join([
                            "from pathlib import Path",
                            "import sys, time",
                            "print('layout started', flush=True)",
                            "assert Path(sys.argv[1]).read_text() == 'layout started\\n'",
                            "print('route stopped', file=sys.stderr, flush=True)",
                            "time.sleep(30)" if wait else "sys.exit(2)",
                        ])
                        return run_child([sys.executable, "-c", script, log_path], **options)

                    with patch.object(run.harness_common, "run_group", side_effect=execute):
                        row = run.run_board(arguments, schematic, "case", output, 2)
                    self.assertEqual(log_path.read_text(), "layout started\nroute stopped\n")
                    self.assertEqual(row["error"], "timed out after 1 s" if wait else "route stopped")
                    if not wait:
                        self.assertEqual(row["layout_exit_code"], 2)

    def test_single_workflow_requires_native_connectivity_on_the_delivered_board(self):
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary)
            schematic, brief = (root / name for name in ("case.kicad_sch", "brief.json"))
            schematic.touch()
            brief.write_text("{}")
            arguments = SimpleNamespace(
                schematic=schematic, constraints=brief, binary=Path(__file__), outline_board=None,
                total_seconds=None, router_config=None, timeout=10,
            )
            for status, extra_check, passed in (
                ("skipped", True, False), ("saved", False, True), ("rule_area", False, True),
            ):
                with self.subTest(status=status):
                    work = root / status
                    work.mkdir()
                    result = work / "layout/result"

                    def execute(command, **kwargs):
                        if command[1] == "import-kicad-netlist":
                            return subprocess.CompletedProcess([], 0, '{"footprints":2,"missing":[]}', "")
                        if command[1] == "pcb":
                            self.assertIn("--all-track-errors", command)
                            self.assertNotIn("--refill-zones", command)
                            report = json.loads((result / "drc.json").read_text())
                            report["unconnected_items"] = [{"type": "unconnected_items"}]
                            Path(command[command.index("--output") + 1]).write_text(json.dumps(report))
                            return subprocess.CompletedProcess([], 0, "")

                        result.mkdir(parents=True)
                        zone = '(zone (layer "B.Cu") (fill yes))'
                        if status == "rule_area":
                            zone = '(zone (layer "B.Cu") (keepout (tracks not_allowed)))'
                        native = write_layout_drc(result, f"(kicad_pcb {zone})")
                        verdict = {"status": "complete", "kicad_checked": True, "constraints_missed": []}
                        if status == "saved":
                            verdict["fill_publication"] = {
                                "status": status,
                                "saved_board_sha256": run.harness_common.file_sha256(result / "case.kicad_pcb"),
                                "saved_as_stored": {"opens": 0, "errors": 0, "schematic_parity": 0},
                            }
                            (result / "drc-saved-fill.json").write_text(json.dumps(native))
                        elif status == "skipped":
                            verdict["fill_publication"] = {"status": status, "reason": "budget"}
                        (result.parent / "verdict.json").write_text(json.dumps(verdict))
                        (result.parent / "board-layout.json").write_text(json.dumps({"constraints": [], "routed": {
                            "routed_connections": 261, "routable_connections": 262,
                            "native": {"schematic_parity_issues": 0},
                        }}))
                        return subprocess.CompletedProcess([], 0, "")

                    with patch.object(run.harness_common, "run_group", side_effect=execute) as invoke:
                        row = run.run_board(arguments, schematic, "case", work, 2)
                    self.assertEqual(invoke.call_count, 3 if extra_check else 2)
                    self.assertEqual(row["routed"], "261/262")
                    self.assertEqual(row["unconnected"], 0)
                    self.assertIs(row["pass"], passed)
                    self.assertEqual(row["stored_copper"]["unconnected"], 0 if passed else 1)
                    if extra_check:
                        action = row["stored_copper"]["next_action"]
                        self.assertEqual(action["kind"], "save_and_check_fills")
                        self.assertEqual(action["argv"][1:4], ["drc-kicad-board", str(result), "case"])
                        self.assertEqual(action["argv"][-2:], ["--save-filled", str(work / "verified-filled-project")])

    def test_candidate_mode_records_separate_verification(self):
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary)
            schematic, brief, board = (root / name for name in ("case.kicad_sch", "brief.json", "case.kicad_pcb"))
            for path in (schematic, brief, board):
                path.touch()
            output = root / "run"
            argv = ["run.py", str(output), "--binary", __file__, "--schematic", str(schematic),
                    "--constraints", str(brief), "--layers", "2", "--candidate-board", str(board)]
            with (patch.object(sys, "argv", argv),
                  patch.object(run.returned_board, "verify", return_value={"pass": True}) as verify,
                  patch.object(run.time, "monotonic_ns", side_effect=[0, 2_000_000_000, 5_000_000_000]),
                  redirect_stdout(io.StringIO())):
                self.assertEqual(run.main(), 0)
            verify.assert_called_once()
            row, = json.loads((output / "results.json").read_text())
            self.assertEqual(row["wall_time_seconds"], {
                "total": 5.0,
                "stages": {"input_preparation": 2.0, "candidate_verification": 3.0},
            })

    def test_defer_zone_fill_records_verified_layout_configuration(self):
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary)
            output = root / "run"
            schematic, brief, board = (root / name for name in ("case.kicad_sch", "brief.json", "case.kicad_pcb"))
            for path in (schematic, brief, board):
                path.touch()
            argv = ["run.py", str(output), "--binary", __file__, "--defer-zone-fill"]
            standalone = ["--schematic", str(schematic), "--constraints", str(brief), "--layers", "2"]
            for mode, error in [
                ([], "require --schematic"),
                (standalone + ["--candidate-board", str(board)], "only applies when running a layout"),
                (standalone + ["--prepare-only"], "only applies when running a layout"),
            ]:
                with self.subTest(mode=mode), patch.object(sys, "argv", argv + mode):
                    errors = io.StringIO()
                    with redirect_stderr(errors), self.assertRaises(SystemExit) as exit_:
                        run.main()
                    self.assertEqual(exit_.exception.code, 2)
                    self.assertIn(error, errors.getvalue())
                    self.assertFalse(output.exists())

            layout_config = output / "project/layout.json"
            fill = {"status": "skipped", "reason": "save_zone_fills is disabled for this layout"}

            def execute(command, **kwargs):
                if command[1] == "import-kicad-netlist":
                    layout_config.write_text('{"moves":0}')
                    return subprocess.CompletedProcess([], 0, '{"footprints":2,"missing":[]}', "")
                self.assertEqual(command[-1], str(layout_config))
                self.assertEqual(json.loads(layout_config.read_text()), {"moves": 0, "save_zone_fills": False})
                layout = output / "layout"
                (layout / "result").mkdir(parents=True)
                write_layout_drc(layout / "result")
                (layout / "verdict.json").write_text('{"status":"complete","kicad_checked":true}')
                (layout / "board-layout.json").write_text(json.dumps({"constraints": [],
                    "fill_publication": fill, "routed": {"routed_connections": 1,
                    "routable_connections": 1, "native": {"schematic_parity_issues": 0}}}))
                return subprocess.CompletedProcess([], 0, "layout complete")

            with patch.object(sys, "argv", argv + standalone), redirect_stdout(io.StringIO()):
                with patch.object(run.harness_common, "run_group", side_effect=execute):
                    self.assertEqual(run.main(), 0)
            row, = json.loads((output / "results.json").read_text())
            self.assertIs(row["save_zone_fills"], False)
            self.assertEqual(row["fill_publication"], fill)
            self.assertEqual(row["layout_config"], {
                "path": "project/layout.json", "sha256": run.harness_common.file_sha256(layout_config)})

    def test_placer_config_is_available_in_preparation_and_rejects_conflicting_modes_or_constraints(self):
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary)
            output = root / "run"
            schematic = root / "case.kicad_sch"
            schematic.touch()
            brief = root / "brief.json"
            brief.write_text("{}")
            pcb = root / "case.kicad_pcb"
            pcb.touch()
            placer = root / "placer.json"
            argv = ["run.py", str(output), "--binary", __file__, "--placer-config", str(placer)]
            standalone = ["--schematic", str(schematic), "--constraints", str(brief), "--layers", "2"]
            for mode, raw, message in [
                ([], '{"decoupling_near": true}', "require --schematic"),
                (standalone + ["--candidate-board", str(pcb)], '{}', "only applies when running or preparing a layout"),
                (standalone, '[]', "must contain a JSON object"),
                (standalone + ["--prepare-only"], '{"constraints":"other.json"}', "must use the staged constraints.json"),
                (standalone, '{"constraints":{"version":1}}', "must use the staged constraints.json"),
            ]:
                with self.subTest(mode=mode, raw=raw):
                    placer.write_text(raw)
                    errors = io.StringIO()
                    with patch.object(sys, "argv", argv + mode), redirect_stderr(errors), self.assertRaises(SystemExit) as exit_:
                        run.main()
                    self.assertEqual(exit_.exception.code, 2)
                    self.assertIn(message, errors.getvalue())
                    self.assertFalse(output.exists())

            placer.write_text('{"constraints":"constraints.json","decoupling_near":true}')
            with patch.object(sys, "argv", argv + standalone + ["--prepare-only"]), redirect_stdout(io.StringIO()):
                with patch.object(run.exchange_package, "prepare", return_value={"pass": True}) as prepare:
                    self.assertEqual(run.main(), 0)
            self.assertEqual(prepare.call_args.kwargs["placer"], {"constraints": "constraints.json", "decoupling_near": True})
            row, = json.loads((output / "results.json").read_text())
            self.assertEqual(row["placer_config"], run.returned_board.record(placer))

    def test_layout_handoff_checks_previous_board_provenance_and_total_work(self):
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary)
            schematic = root / "case.kicad_sch"
            schematic.touch()
            brief = root / "brief.json"
            brief.write_text("{}")
            previous = root / "previous board.kicad_pcb"
            previous.write_text("(kicad_pcb)")
            previous_record = run.returned_board.record(previous)

            for matching in (True, False):
                output = root / str(matching)

                def execute(command, **kwargs):
                    if command[1] == "import-kicad-netlist":
                        return subprocess.CompletedProcess([], 0, '{"footprints": 2, "missing": []}', "")
                    self.assertEqual(command[-2:], ["--reuse-routes", str(previous)])
                    layout = output / "layout"
                    (layout / "result").mkdir(parents=True)
                    write_layout_drc(layout / "result")
                    (layout / "verdict.json").write_text(
                        '{"status":"complete","kicad_checked":true,"work_seconds":28,'
                        '"route_reuse":{"status":"accepted","saved_routable_nets":18}}')
                    digest = previous_record["sha256"] if matching else "wrong"
                    (layout / "board-layout.json").write_text(json.dumps({
                        "constraints": [], "work_seconds": 12,
                        "routed": {"routed_connections": 0, "routable_connections": 0,
                                   "native": {"schematic_parity_issues": 0},
                                   "route_reuse": {"sha256": digest, "status": "accepted",
                                                   "matched_nets": ["retained"]}},
                    }))
                    return subprocess.CompletedProcess([], 0, "layout complete")

                argv = ["run.py", str(output), "--schematic", str(schematic), "--constraints", str(brief),
                        "--layers", "2", "--binary", __file__, "--reuse-routes", str(previous)]
                with patch.object(sys, "argv", argv), patch.object(run.harness_common, "run_group", side_effect=execute):
                    with redirect_stdout(io.StringIO()):
                        self.assertEqual(run.main(), 0 if matching else 1)
                row, = json.loads((output / "results.json").read_text())
                self.assertEqual(row["reuse_routes_input"], previous_record)
                if matching:
                    self.assertEqual(row["route_reuse"]["status"], "accepted")
                    self.assertEqual(row["work_seconds"], 28)
                    self.assertEqual(row["routed"], "0/0")
                    self.assertEqual(row["routing_coverage"], {
                        "pending_routed_connections": 0,
                        "pending_routable_connections": 0,
                        "kicad_unconnected_items": 0,
                        "retained_nets": 1,
                        "saved_routable_nets": 18,
                    })
                else:
                    self.assertIn("provenance", row["error"])

    def test_layout_recovery_budget_is_a_slice_of_the_declared_total(self):
        for option, report in [("plane_escape", "plane_escape"), ("terminal_launch", "terminal_launch_retry")]:
            with self.subTest(option=option):
                self.check_layout_recovery(option, report)

    def test_complete_layout_explains_native_errors_that_reject_the_board(self):
        self.check_layout_recovery("terminal_launch", "terminal_launch_retry", footprint_errors=True)

    def check_layout_recovery(self, option, report, footprint_errors=False):
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary)
            schematic = root / "case.kicad_sch"
            schematic.touch()
            brief = root / "brief.json"
            brief.write_text("{}")
            output = root / "run"
            output.mkdir()
            arguments = SimpleNamespace(
                schematic=schematic, constraints=brief, binary=Path(__file__), outline_board=None,
                total_seconds=390, router_config=None, timeout=10,
            )
            setattr(arguments, option + "_budget", 150)
            retry = {"status": "skipped", "reason": "requires_one_native_open_and_clean_parity"}

            def execute(command, **kwargs):
                if command[1] == "import-kicad-netlist":
                    (output / "project/layout.json").write_text("{}")
                    return subprocess.CompletedProcess([], 0, '{"footprints": 2, "missing": []}', "")
                self.assertEqual(command[-2:], ["--" + option.replace("_", "-") + "-budget", "150"])
                self.assertEqual(json.loads(Path(command[-3]).read_text())["total_seconds"], 390)
                layout = output / "layout"
                (layout / "result").mkdir(parents=True)
                native = write_layout_drc(layout / "result")
                verdict = {"status": "complete", "kicad_checked": True, "constraints_missed": []}
                if footprint_errors:
                    native["violations"] = [{"severity": "error", "type": "copper_edge_clearance"}]
                    (layout / "result/drc.json").write_text(json.dumps(native))
                    verdict["footprint_findings"] = {"copper_edge_clearance": 1}
                (layout / "verdict.json").write_text(json.dumps(verdict))
                (layout / "board-layout.json").write_text(json.dumps({
                    report: retry, "constraints": [],
                    "routed": {"routed_connections": 1, "routable_connections": 1,
                               "native": {"schematic_parity_issues": 0}},
                }))
                return subprocess.CompletedProcess([], 0, "")

            with patch.object(run.harness_common, "run_group", side_effect=execute):
                row = run.run_board(arguments, schematic, "case", output, 2)
            self.assertEqual(row["pass"], not footprint_errors)
            if footprint_errors:
                self.assertEqual(row["native_errors_by_type"], {"copper_edge_clearance": 1})
                self.assertEqual(row["footprint_findings"], {"copper_edge_clearance": 1})
                self.assertIn("layout/result/drc.json", row["next_actions"][0])
            self.assertEqual(row[option + "_reserved_seconds"], 150)
            self.assertEqual(row[report], retry)

    def test_older_partial_reuse_coverage_needs_native_connectivity(self):
        layout = {
            "routed": {"routed_connections": 2, "routable_connections": 2,
                       "native": {"schematic_parity_issues": 0},
                       "route_reuse": {"status": "accepted", "matched_nets": ["A", "B"]}},
            "route_reuse_placement": {"saved_routable_nets": 3},
        }
        self.assertEqual(run.accepted_reuse_coverage(layout, {}, 0), {
            "pending_routed_connections": 2, "pending_routable_connections": 2,
            "kicad_unconnected_items": 0, "retained_nets": 2, "saved_routable_nets": 3,
        })
        old_outcome = {"route_reuse": {"status": "accepted", "retained_nets": 2}}
        self.assertEqual(run.accepted_reuse_coverage(layout, old_outcome, 0)["saved_routable_nets"], 3)
        self.assertNotIn("saved_routable_nets", run.accepted_reuse_coverage(layout, {}, 1))
        layout["routed"]["native"]["schematic_parity_issues"] = 1
        self.assertNotIn("saved_routable_nets", run.accepted_reuse_coverage(layout, {}, 0))
        layout["routed"]["route_reuse"]["status"] = "fallback"
        self.assertIsNone(run.accepted_reuse_coverage(layout, {}, 0))

    def test_incomplete_cold_fallback_preserves_routing_recovery_actions(self):
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary)
            schematic = root / "case.kicad_sch"
            schematic.touch()
            brief = root / "brief.json"
            brief.write_text("{}")
            previous = root / "previous.kicad_pcb"
            previous.write_text("(kicad_pcb)")
            output = root / "run"
            output.mkdir()
            arguments = SimpleNamespace(
                schematic=schematic, constraints=brief, binary=Path(__file__), outline_board=None,
                total_seconds=None, router_config=None, reuse_routes=previous, timeout=10,
            )
            action = "Inspect the open net and its pads with pcb-maker inspect-routing-run."
            reason = "incomplete: 1 unconnected item in KiCad"

            def execute(command, **kwargs):
                if command[1] == "import-kicad-netlist":
                    return subprocess.CompletedProcess([], 0, '{"footprints": 2, "missing": []}', "")
                self.assertEqual(command[-2:], ["--reuse-routes", str(previous)])
                layout = output / "layout"
                (layout / "result").mkdir(parents=True)
                (layout / "result/drc.json").write_text('{"violations":[],"unconnected_items":[{}]}')
                (layout / "verdict.json").write_text(json.dumps({
                    "status": "incomplete", "kicad_checked": True,
                    "reason": reason, "next_actions": [action], "constraints_missed": [],
                }))
                (layout / "board-layout.json").write_text(json.dumps({
                    "constraints": [],
                    "routed": {
                        "routed_connections": 0, "routable_connections": 1,
                        "native": {"schematic_parity_issues": 0},
                        "route_reuse": {
                            "sha256": run.returned_board.record(previous)["sha256"],
                            "status": "fallback", "reason": "too little layout budget for the trial",
                        },
                    },
                }))
                return subprocess.CompletedProcess([], 1, reason)

            with patch.object(run.harness_common, "run_group", side_effect=execute):
                row = run.run_board(arguments, schematic, "case", output, 2)
            self.assertFalse(row["pass"])
            self.assertEqual(row["stage"], "verification")
            self.assertEqual(row["route_reuse"]["status"], "fallback")
            self.assertEqual(row["unconnected"], 1)
            self.assertEqual(row["constraints_missed"], [])
            self.assertEqual(row["layout_reason"], reason)
            self.assertEqual(row["next_actions"], [action])


if __name__ == "__main__":
    unittest.main()
