# Copyright (C) 2026 Toit contributors.
"""Tests of run.py's defaults: python3 -m unittest benchmarks/agent-tasks/test_run.py"""

import json
import os
import shutil
import subprocess
import sys
import tempfile
import time
import unittest
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parent))
import compare  # noqa: E402
import run  # noqa: E402

HERE = Path(__file__).resolve().parent


class Parallelism(unittest.TestCase):
    def test_a_tier_without_jobs_or_threads_runs_its_default(self):
        jobs, threads, source = run.parallelism(True, None, None, 16)
        self.assertEqual((jobs * threads, threads, source), (16, run.TIER_THREADS, "tier default"))

    def test_either_given_keeps_the_older_defaults(self):
        self.assertEqual(run.parallelism(True, 6, None, 16), (6, None, "command line"))
        self.assertEqual(run.parallelism(True, None, 4, 16), (1, 4, "command line"))
        self.assertEqual(run.parallelism(True, 6, 4, 16), (6, 4, "command line"))
        self.assertEqual(run.default_threads(6, 16), 2)

    def test_without_a_tier_one_job(self):
        self.assertEqual(run.parallelism(False, None, None, 16), (1, None, "command line"))

    def test_at_least_one_job(self):
        jobs, _, _ = run.parallelism(True, None, None, 1)
        self.assertGreaterEqual(jobs, 1)


class WorkRateAndTimeout(unittest.TestCase):
    def test_the_reference_rate_is_pinned_unless_asked_otherwise(self):
        environ = {}
        self.assertEqual(run.pin_work_rate(None, environ), run.REFERENCE_WORK_RATE)
        self.assertEqual(environ["PCB_WORK_RATE"], "3.15e6,150e6")
        environ = {"PCB_WORK_RATE": "2e6"}
        self.assertEqual(run.pin_work_rate(None, environ), "2e6")
        self.assertEqual(environ["PCB_WORK_RATE"], "2e6")
        self.assertEqual(run.pin_work_rate("3e6,140e6", environ), "3e6,140e6")
        self.assertEqual(environ["PCB_WORK_RATE"], "3e6,140e6")
        self.assertEqual(run.pin_work_rate("measured", environ), "measured")
        self.assertNotIn("PCB_WORK_RATE", environ)

    def test_the_timeout_follows_the_wall_guard(self):
        class Arguments:
            timeout_given = False
            timeout = run.DEFAULT_TIMEOUT_SECONDS
            budget_scale = 1.0
        arguments = Arguments()
        # The layout's 1500 s at a guard of 2, plus 300 s.
        self.assertEqual(run.scaled_timeout(arguments), 3300)
        self.assertEqual(run.scaled_timeout(arguments, 1500, 3.0), 4800)
        # At least 1800 s.
        self.assertEqual(run.scaled_timeout(arguments, 40), 1800)
        arguments.budget_scale = 2.0
        self.assertEqual(run.scaled_timeout(arguments), 6300)
        # --timeout as given, beyond its fixed 300 s scaled.
        arguments.timeout_given, arguments.timeout = True, 1000
        self.assertEqual(run.scaled_timeout(arguments, 40), 1700)
        arguments.budget_scale = 1.0
        self.assertEqual(run.scaled_timeout(arguments, 40), 1000)

    def test_the_timeout_allows_for_the_stretched_guard(self):
        class Arguments:
            timeout_given = False
            timeout = run.DEFAULT_TIMEOUT_SECONDS
            budget_scale = 1.0
            work_rate = run.REFERENCE_WORK_RATE
        arguments = Arguments()
        # Pinned rates: the binary may stretch its guard four times.
        self.assertEqual(run.wall_stretch_allowance(arguments), 4.0)
        self.assertEqual(run.scaled_timeout(arguments), 1500 * 2 * 4 + 300)
        self.assertEqual(run.intake_timeout(30, 4.0), 420.0)
        arguments.work_rate = "measured"
        self.assertEqual(run.scaled_timeout(arguments), 3300)

    def test_rows_carry_the_verdicts_rates(self):
        row = {}
        run.verdict_extras(row, {"work_rate": 3.15e6, "placer_work_rate": 150e6, "work_rate_source": "PCB_WORK_RATE",
                                 "work_rate_measured": 3.1e6, "placer_work_rate_measured": 1.4e8,
                                 "work_wall_ratio": 0.98765, "environment": {}})
        self.assertEqual(row, {"work_rate": 3.15e6, "placer_work_rate": 150e6, "work_rate_source": "PCB_WORK_RATE",
                               "work_rate_measured": 3.1e6, "placer_work_rate_measured": 1.4e8,
                               "work_wall_ratio": 0.988})
        rows = [{"work_rate_measured": 3.0e6, "work_wall_ratio": 0.9}, {"work_rate_measured": 3.2e6},
                {"work_rate_measured": 2.0e6}, {}]
        self.assertEqual(run.measured_rates(rows), {"work_rate_measured_median": 3.0e6,
                                                    "placer_work_rate_measured_median": None,
                                                    "work_wall_ratio_median": 0.9})


def finding(kind, *items):
    return (kind, [(uuid, at) for uuid, at in items])


class StrictRule(unittest.TestCase):
    """The strict rule (verdict.rs's split_findings_strict, as run.py has it)."""

    def test_a_new_finding_of_a_type_the_designer_has_elsewhere_counts(self):
        theirs = finding("clearance", ("p1", (10.0, 10.0)), ("p2", (10.4, 10.0)))
        elsewhere = finding("clearance", ("t1", (30.0, 12.0)), ("p9", (30.3, 12.0)))
        split = run.strict_findings([theirs, elsewhere],
                                    [theirs, finding("clearance", ("p5", (50.0, 50.0)), ("p6", (50.2, 50.0)))])
        self.assertEqual(split["new"], {"clearance": 1})
        self.assertEqual(split["preexisting"], {"clearance": 1})

    def test_a_parts_own_finding_goes_with_the_part(self):
        designer = finding("hole_to_hole", ("h1", (85.0, 13.8)), ("h2", (85.0, 13.2)))
        moved = finding("hole_to_hole", ("h2", (20.0, 5.0)), ("h1", (20.6, 5.0)))
        apart = finding("hole_to_hole", ("h1", (20.0, 5.0)), ("h2", (22.0, 5.0)))
        self.assertTrue(run.same_place(moved, designer))
        self.assertFalse(run.same_place(apart, designer))
        self.assertTrue(run.same_place(finding("hole_to_hole", ("x", (85.02, 13.8)), (None, (85.0, 13.23))), designer))
        self.assertFalse(run.same_place(finding("clearance", ("h1", (85.0, 13.8)), ("h2", (85.0, 13.2))), designer))

    def test_a_type_at_kicads_cap_is_unknown(self):
        ours = finding("hole_clearance", ("a", (1.0, 1.0)))
        reference = [finding("hole_clearance", ("z", (float(i), 40.0))) for i in range(run.KICAD_FINDING_CAP)]
        split = run.strict_findings([ours], reference + [ours])
        self.assertEqual(split, {"new": {"hole_clearance": 1}, "preexisting": {}, "unknown": ["hole_clearance"],
                                 "mask_openings": 0})

    def test_without_a_baseline_every_finding_counts(self):
        split = run.strict_findings([finding("clearance", ("a", (1.0, 1.0)))], None)
        self.assertEqual(split["new"], {"clearance": 1})


def bridge(*items):
    """A KiCad solder_mask_bridge violation: items as (description, uuid, (x, y))."""
    return {"type": "solder_mask_bridge", "severity": "error",
            "description": "Front solder mask aperture bridges items with different nets",
            "items": [{"description": description, "uuid": uuid, "pos": {"x": at[0], "y": at[1]}}
                      for description, uuid, at in items]}


OPENING = ("Rectangle of J_LCD1 on F.Mask", "r1", (38.9, 56.0))
J2_PAD = ("Pad 3 [GND] of J2 on F.Cu", "p3", (10.0, 10.0))
J2_PAD4 = ("Pad 4 [VCC] of J2 on F.Cu", "p4", (10.5, 10.0))
POUR = ("Zone [GND] on F.Cu, priority 0", "z1", (13.0, 30.1))


def track(uuid, net="LCD_SCLK", at=(46.8, 57.5)):
    return (f"Track [{net}] on F.Cu, length 1.6626 mm", uuid, at)


class StrictMaskOpenings(unittest.TestCase):
    """The strict rule's pre-bridged mask openings (rule 2): a solder mask
    bridge of a designer's opening that bridges on the designer's board
    already, with one of our tracks, is not ours."""

    def split(self, ours, stripped, designer=None):
        report = lambda violations: run.counting_findings({"violations": violations})  # noqa: E731
        return run.strict_findings(report(ours), report(stripped),
                                   None if designer is None else report(designer))

    def test_an_opening_bridged_on_the_stripped_board_bridges_our_track(self):
        # Stripped: the opening shows the pour (opening and zone). Ours:
        # the same pair (the same place) and the opening with our track.
        split = self.split([bridge(OPENING, POUR), bridge(OPENING, track("t9"))], [bridge(OPENING, POUR)])
        self.assertEqual((split["new"], split["preexisting"], split["mask_openings"]),
                         ({}, {"solder_mask_bridge": 1}, 1))

    def test_an_opening_the_designer_shipped_bridged(self):
        # Not on the stripped board (the designer's tracks went with the
        # strip), but on the designer's own: their track under it.
        ours = [bridge(OPENING, track("t9"))]
        self.assertEqual(self.split(ours, [])["new"], {"solder_mask_bridge": 1})
        split = self.split(ours, [], [bridge(OPENING, track("designer-t1", "LCD_DC", (47.0, 58.0)))])
        self.assertEqual((split["new"], split["mask_openings"]), ({}, 1))

    def test_matched_by_position_when_the_uuid_differs(self):
        moved = ("Rectangle of J_LCD1 on F.Mask", "new-uuid", (38.92, 56.0))
        split = self.split([bridge(moved, track("t9"))], [bridge(OPENING, POUR)])
        self.assertEqual(split["mask_openings"], 1)
        far = ("Rectangle of J_LCD1 on F.Mask", "new-uuid", (60.0, 56.0))
        self.assertEqual(self.split([bridge(far, track("t9"))], [bridge(OPENING, POUR)])["new"],
                         {"solder_mask_bridge": 1})

    def test_a_pad_aperture_over_pads_of_several_nets_bridges_a_track_leaving_a_pad(self):
        split = self.split([bridge(J2_PAD, track("t9", "VCC", (10.6, 10.2)))], [bridge(J2_PAD, J2_PAD4)])
        self.assertEqual((split["new"], split["mask_openings"]), ({}, 1))

    def test_our_via_under_a_bridged_aperture_counts(self):
        via = ("Via [SDA] on F.Cu - B.Cu", "v1", (10.2, 10.1))
        split = self.split([bridge(J2_PAD, via), bridge(via, J2_PAD)], [bridge(J2_PAD, J2_PAD4)])
        self.assertEqual((split["new"], split["mask_openings"]), ({"solder_mask_bridge": 2}, 0))

    def test_a_pad_of_another_part_under_a_bridged_aperture_counts(self):
        other = ("Pad 1 [SDA] of R5 on F.Cu", "q1", (10.3, 10.4))
        split = self.split([bridge(J2_PAD, other)], [bridge(J2_PAD, J2_PAD4)])
        self.assertEqual(split["new"], {"solder_mask_bridge": 1})

    def test_two_of_our_own_items_count(self):
        via = ("Via [SDA] on F.Cu - B.Cu", "v1", (20.0, 20.0))
        split = self.split([bridge(via, track("t9", at=(20.1, 20.0)))], [bridge(OPENING, POUR)])
        self.assertEqual(split["new"], {"solder_mask_bridge": 1})

    def test_an_opening_bridged_nowhere_counts(self):
        text = ("PCB text '0bsilab' on F.Mask", "x1", (89.7, 84.1))
        split = self.split([bridge(text, track("t9"))], [bridge(OPENING, POUR)], [bridge(OPENING, POUR)])
        self.assertEqual(split["new"], {"solder_mask_bridge": 1})

    def test_without_a_baseline_every_bridge_counts(self):
        ours = run.counting_findings({"violations": [bridge(OPENING, track("t9"))]})
        designer = run.counting_findings({"violations": [bridge(OPENING, POUR)]})
        self.assertEqual(run.strict_findings(ours, None, designer)["new"], {"solder_mask_bridge": 1})

    def test_openings_are_mask_graphics_and_pads(self):
        for description in ("Rectangle of J_LCD1 on F.Mask", "PCB text 'RX' on B.Mask", "Pad 1 [GND] of U3 on B.Mask",
                            "Pad 3 [GND] of J2 on F.Cu", "PTH pad 1 [VCC] of SW4", "NPTH pad of SW4"):
            self.assertTrue(run.mask_opening((None, None, description)), description)
        for description in ("Track [GND] on F.Cu, length 1.0 mm", "Via [GND] on F.Cu - B.Cu",
                            "Zone [GND] on F.Cu, priority 0", "Segment [GND] of U1 on F.Cu", ""):
            self.assertFalse(run.mask_opening((None, None, description)), description)

    def test_the_row_carries_the_rule_and_the_subtracted_count(self):
        with tempfile.TemporaryDirectory() as scratch:
            work = Path(scratch)
            (work / "result").mkdir()
            (work / "result/drc.json").write_text(json.dumps(
                {"violations": [bridge(OPENING, track("t9")), bridge(J2_PAD, ("Via [SDA] on F.Cu - B.Cu", "v1",
                                                                                (10.2, 10.1)))],
                 "unconnected_items": []}))
            (work / run.DESIGNER_BRIDGES).write_text(json.dumps({"violations": [bridge(OPENING, track("d1"))]}))
            baseline = run.StrictBaseline(work, "board", work, None)
            baseline.enabled, baseline.taken, baseline.result = True, True, []
            row = {"native": {"errors": {}, "unconnected": 0}}
            run.strict_row(row, run.strict_native(work / "result", baseline))
            self.assertEqual((row["strict_rule"], row["strict_mask_openings"], row["new_findings_strict"],
                              row["pass_strict"]), (run.STRICT_RULE, 1, {"solder_mask_bridge": 1}, False))

    def test_the_designers_bridges_are_kept_for_the_rule(self):
        # A stand-in for the binary: drc-kicad-board writes a report with a
        # bridge and a clearance finding.
        with tempfile.TemporaryDirectory() as scratch:
            scratch = Path(scratch)
            report = {"violations": [bridge(OPENING, POUR),
                                     {"type": "clearance", "severity": "error", "items": []}]}
            binary = scratch / "pcb-maker"
            binary.write_text("#!/bin/sh\nif [ \"$1\" = --help ]; then echo drc-kicad-board; exit 0; fi\n"
                              f"printf '%s' '{json.dumps(report)}' > \"$4\"\n")
            binary.chmod(0o755)
            work = scratch / "work"
            work.mkdir()
            counts = run.designer_findings(scratch, "board", work, binary)
            self.assertEqual(counts, {"solder_mask_bridge": 1, "clearance": 1})
            kept = json.loads((work / run.DESIGNER_BRIDGES).read_text())["violations"]
            self.assertEqual([violation["type"] for violation in kept], ["solder_mask_bridge"])


class CommittedTaskLists(unittest.TestCase):
    """benchmarks/agent-tasks/tasks/v6 and v7: the lists the tiers run."""

    def load(self, version):
        directory = HERE / "tasks" / version
        tasks = json.loads((directory / "tasks.json").read_text())["tasks"]
        return directory, {task["name"]: task for task in tasks}

    def test_every_task_has_its_constraint_file(self):
        for version in ("v6", "v7"):
            directory, tasks = self.load(version)
            for task in tasks.values():
                if task.get("constraints"):
                    self.assertTrue((directory / task["constraints"]).is_file(), (version, task["name"]))

    def test_tiers_name_the_committed_list_and_its_boards(self):
        for tier in ("quick", "short", "schematic", "designer"):
            entry = json.loads(run.TIERS[tier].read_text())
            self.assertEqual(entry["tasks"], "benchmarks/agent-tasks/tasks/v7/tasks.json", tier)
            _, tasks = self.load("v7")
            self.assertLessEqual(set(entry["boards"]), set(tasks), tier)

    def test_v6_and_v7_differ_only_on_six_panels(self):
        # The quick and short tiers moved from v6 to v7 (2026-10-21): none of
        # their boards is one of the six, so their older v6 logs stay bases.
        directory6, tasks6 = self.load("v6")
        directory7, tasks7 = self.load("v7")
        self.assertEqual(set(tasks6), set(tasks7))
        differ = {name for name in tasks7 if compare.task_digest(
            tasks6[name], json.loads((directory6 / tasks6[name]["constraints"]).read_text()))
            != compare.task_digest(tasks7[name], json.loads((directory7 / tasks7[name]["constraints"]).read_text()))}
        self.assertEqual(len(differ), 6)
        for tier in ("quick", "short"):
            self.assertFalse(differ & set(json.loads(run.TIERS[tier].read_text())["boards"]), tier)

    def test_an_absolute_directory_elsewhere_falls_back_to_this_checkout(self):
        task = {"directory": "/nowhere/pcb-maker/benchmarks/real/external/github/x"}
        with tempfile.TemporaryDirectory() as root:
            board = Path(root) / "benchmarks/real/external/github/x"
            board.mkdir(parents=True)
            old = run.ROOT
            run.ROOT = Path(root)
            try:
                self.assertEqual(run.task_directory(task), board)
            finally:
                run.ROOT = old
        self.assertEqual(run.task_directory({"directory": "benchmarks/x"}), run.ROOT / "benchmarks/x")


@unittest.skipUnless(Path("/proc/locks").exists() and shutil.which("flock"), "needs /proc/locks and flock(1)")
class TierLock(unittest.TestCase):
    def setUp(self):
        self.directory = Path(tempfile.mkdtemp())
        self.lock = self.directory / "tier.lock"

    def tearDown(self):
        shutil.rmtree(self.directory, ignore_errors=True)

    def hold(self):
        """A process that holds the lock until killed."""
        # -o: the lock stays with flock(1), not with its sleep.
        holder = subprocess.Popen(["flock", "-o", str(self.lock), "sleep", "60"], start_new_session=True)
        for _ in range(100):
            if run.lock_holders(self.lock):
                return holder
            time.sleep(0.05)
        holder.kill()
        self.fail("the holder never took the lock")

    def test_takes_a_free_lock_and_says_who_holds_it(self):
        messages = []
        descriptor = run.take_lock(self.lock, log=messages.append)
        try:
            self.assertIsNotNone(descriptor)
            self.assertIn(os.getpid(), run.lock_holders(self.lock))
            self.assertIn(f"pid {os.getpid()}", self.lock.read_text())
        finally:
            os.close(descriptor)

    def test_waits_for_a_holder_and_names_it(self):
        holder = self.hold()
        messages = []
        try:
            killer = subprocess.Popen(["sh", "-c", f"sleep 0.5; kill -- -{holder.pid}"])
            descriptor = run.take_lock(self.lock, poll_seconds=0.1, log=messages.append)
            killer.wait()
            os.close(descriptor)
        finally:
            try:
                os.killpg(holder.pid, 9)
            except ProcessLookupError:
                pass
            holder.wait()
        self.assertIn("waiting for pid", messages[0])
        self.assertIn("sleep 60", messages[0])
        self.assertIn("taken", messages[-1])

    def test_a_caller_holding_it_lets_the_run_through(self):
        # The older habit: flock build/quick-tier.lock run.py ...
        here = Path(__file__).resolve().parent
        code = (f"import sys; sys.path.insert(0, {str(here)!r}); import run; "
                f"print(run.take_lock({str(self.lock)!r}, poll_seconds=0.1))")
        result = subprocess.run(["flock", str(self.lock), sys.executable, "-c", code],
                                capture_output=True, text=True, timeout=30)
        self.assertEqual(result.stdout.strip(), "None", result.stderr)
        self.assertIn("held by this run's caller", result.stderr)


if __name__ == "__main__":
    unittest.main()
