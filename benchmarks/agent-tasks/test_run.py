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

# The tests' own memory ledger, not the machine's (harness_common.Ledger;
# their subprocesses inherit it).
os.environ.setdefault("PCB_MAKER_LEDGER", str(Path(tempfile.gettempdir()) / "pcb-maker-test-ledger.json"))

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

    def test_a_type_at_kicads_cap_is_subtracted_only_at_the_same_place(self):
        ours = finding("hole_clearance", ("a", (1.0, 1.0)))
        elsewhere = finding("hole_clearance", ("b", (70.0, 70.0)))
        reference = [finding("hole_clearance", ("z", (float(i), 40.0))) for i in range(run.KICAD_FINDING_CAP)]
        split = run.strict_findings([ours, elsewhere], reference + [ours])
        self.assertEqual(split, {"new": {"hole_clearance": 1}, "preexisting": {"hole_clearance": 1},
                                 "unknown": ["hole_clearance"], "mask_openings": 0})

    def test_a_footprints_own_finding_is_the_references(self):
        def between(first, second):
            return ("clearance", [("u1", (1.0, 2.0), first, "c"), ("u2", (1.0, 4.0), second, "c")])
        own = [between("PTH pad [<no net>] of M15", "PTH pad [GND] of M15"),
               between("Pad 1 [Net-(AE1-A)] of AE1 on F.Cu", "Segment [<no net>] of AE1 on F.Cu")]
        other = [between("PTH pad [<no net>] of M15", "PTH pad [GND] of M16"),
                 between("Track [GND] on F.Cu, length 1.0000 mm", "Pad 2 [VCC] of R62 on F.Cu")]
        self.assertTrue(all(run.footprint_own(f) for f in own))
        self.assertFalse(any(run.footprint_own(f) for f in other))
        reference = [finding("clearance", ("z", (float(i), 40.0))) for i in range(run.KICAD_FINDING_CAP)]
        split = run.strict_findings(own + other, reference)
        self.assertEqual((split["new"], split["preexisting"]), ({"clearance": 2}, {"clearance": 2}))

    def test_a_copper_edge_finding_is_at_the_same_place_by_its_copper(self):
        def edge(outline, uuid, x, ring=("ring", (17.55, 125.0))):
            return ("copper_edge_clearance", [(uuid, (x, 125.0), outline, "edge"),
                                              (ring[0], ring[1], "Segment [GND] on F.Cu", "edge")])
        theirs = edge("Segment on Edge.Cuts", "e2", 17.0)
        self.assertTrue(run.same_place(edge("Arc on Edge.Cuts", "e1", 99.0), theirs))
        self.assertFalse(run.same_place(edge("Segment on Edge.Cuts", "e2", 17.0, ("t", (30.0, 125.0))), theirs))

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


def zone_neck(actual, uuid="8951654c-dd26-460d-9f97-f89e9425e93d", at=(0.0, 0.0), **violation):
    """A KiCad connection_width violation on a pour (KiCad places the
    zone item at (0, 0))."""
    return {"type": "connection_width", "severity": "error",
            "description": f"Copper connection too narrow (board setup constraints minimum connection width "
                           f"0.1270 mm; actual {actual:.4f} mm) (F.Cu)",
            "items": [{"description": "Zone [GND] on F.Cu and B.Cu, priority 0", "uuid": uuid,
                       "pos": {"x": at[0], "y": at[1]}}], **violation}


class OneItemFindings(unittest.TestCase):
    """Rule 3 (review 2026-10-21, H2): a finding of one item is at the
    designer's place only where its item is."""

    def test_usb_keypads_neck_on_the_pour_is_ours(self):
        # designer-x1315b: our one connection_width on zone 8951654c, the
        # stripped baseline 45 on the same zone, every one at (0, 0).
        ours = run.counting_findings({"violations": [zone_neck(0.1098)]})
        baseline = run.counting_findings({"violations": [zone_neck(0.05 + index / 1000) for index in range(45)]})
        split = run.strict_findings(ours, baseline)
        self.assertEqual((split["new"], split["preexisting"]), ({"connection_width": 1}, {}))

    def test_a_zone_finding_with_a_place_of_its_own_matches_there(self):
        # A report that gives the finding a position stands in for the
        # zone's (0, 0).
        ours = run.counting_findings({"violations": [zone_neck(0.1, pos={"x": 20.0, "y": 30.0})]})
        here = run.counting_findings({"violations": [zone_neck(0.1, pos={"x": 20.03, "y": 30.0})]})
        there = run.counting_findings({"violations": [zone_neck(0.1, pos={"x": 25.0, "y": 30.0})]})
        self.assertEqual(run.strict_findings(ours, here)["preexisting"], {"connection_width": 1})
        self.assertEqual(run.strict_findings(ours, there)["new"], {"connection_width": 1})

    def test_one_item_matches_by_place_not_by_uuid(self):
        pad = finding("starved_thermal", ("p1", (10.0, 10.0)))
        self.assertTrue(run.same_place(finding("starved_thermal", ("p1", (10.02, 10.0))), pad))
        # The same pad elsewhere (the part moved): not the designer's.
        self.assertFalse(run.same_place(finding("starved_thermal", ("p1", (30.0, 10.0))), pad))
        # Another item at the pad's place: there.
        self.assertTrue(run.same_place(finding("starved_thermal", ("q7", (10.0, 10.04))), pad))
        self.assertFalse(run.same_place(finding("starved_thermal", ("p1", None)), pad))

    def test_a_moved_items_own_finding_word_for_word_goes_with_it(self):
        # mackerel-10-v1's annular rings (v18): the designer's pads, moved
        # by the layout, with KiCad's measurement unchanged.
        def ring(at, actual="0.1000"):
            return {"type": "annular_width", "severity": "error",
                    "description": f"Annular width (board setup constraints min 0.1270 mm; actual {actual} mm)",
                    "items": [{"description": "PTH pad 1 [GND] of J1", "uuid": "p1", "pos": {"x": at, "y": 5.0}}]}
        designer = run.counting_findings({"violations": [ring(10.0)]})[0]
        self.assertTrue(run.same_place(run.counting_findings({"violations": [ring(40.0)]})[0], designer))
        self.assertFalse(run.same_place(run.counting_findings({"violations": [ring(40.0, "0.0500")]})[0], designer))
        # Never for a zone: its findings are the pour's, wherever.
        ours = run.counting_findings({"violations": [zone_neck(0.1)]})[0]
        self.assertFalse(run.same_place(ours, run.counting_findings({"violations": [zone_neck(0.1)]})[0]))

    def test_two_items_keep_the_rule_of_before(self):
        designer = finding("hole_to_hole", ("h1", (85.0, 13.8)), ("h2", (85.0, 13.2)))
        self.assertTrue(run.same_place(finding("hole_to_hole", ("h2", (20.0, 5.0)), ("h1", (20.6, 5.0))), designer))


class KilledRuns(unittest.TestCase):
    """A run the machine killed is no result (review 2026-10-21, H1)."""

    def test_codes_outside_the_verdicts_are_kills(self):
        for code in (-9, -11, 137, 101, 5):
            self.assertTrue(run.killed(code), code)
        for code in (0, 1, 2, 3, 4, "timeout", None):
            self.assertFalse(run.killed(code), code)

    def test_a_killed_row_has_an_error_and_no_judgement(self):
        row = run.killed_row({"name": "katia"}, -9, "ladder: 3 of 3 rungs\n")
        self.assertEqual((row["error"], row["cut"], row["pass"], row["killed"]),
                         ("killed by signal 9 (SIGKILL)", ["killed"], False, True))
        self.assertNotIn("native", row)
        self.assertIn("exit 137", run.killed_text(137, "x\n"))

    def test_compare_counts_an_old_killed_row_as_a_failure(self):
        # fr2-route-x1371-r2's katia: exit -9, the best attempt so far
        # judged as a result, no error.
        old = {"name": "katia", "exit": -9, "routed": "149/162", "native": {"unconnected": 93, "errors": {}},
               "new_findings_strict": {}, "pass": False}
        self.assertTrue(compare.killed(old))
        self.assertIsNone(compare.unconnected(old))
        self.assertFalse(compare.passes({**old, "native": {"unconnected": 0, "errors": {}}}))
        self.assertIs(compare.passes_strict(old), False)
        self.assertIn("KILLED (exit -9)", compare.cell(old))
        self.assertEqual(compare.objectives([old])["failed"], 1)


@unittest.skipUnless(Path("/proc").is_dir(), "needs /proc")
class ProcessGroups(unittest.TestCase):
    """A timeout kills the board's whole tree, and the gate admits the next
    board only once it is gone (review 2026-10-21, M4)."""

    def test_a_timeout_kills_the_grandchildren(self):
        started = []
        code, _, _, _, _ = run.run_measured(["sh", "-c", "sleep 60 & sleep 60"], timeout=1,
                                            on_start=started.append)
        self.assertEqual(code, "timeout")
        self.assertEqual(run.harness_common.group_members(started[0]), [])

    def test_a_child_left_running_is_ended(self):
        started = []
        code, _, _, _, _ = run.run_measured(["sh", "-c", "sleep 60 > /dev/null 2>&1 & exit 0"], timeout=30,
                                            on_start=started.append)
        self.assertEqual(code, 0)
        self.assertEqual(run.harness_common.group_members(started[0]), [])

    def test_run_group_kills_the_group_on_a_timeout(self):
        before = time.monotonic()
        with self.assertRaises(subprocess.TimeoutExpired):
            run.harness_common.run_group(["sh", "-c", "sleep 60 & sleep 60"], timeout=1, capture_output=True)
        self.assertLess(time.monotonic() - before, 30)


class Provenance(unittest.TestCase):
    """Rows name their binary, spliced rows their log (review 2026-10-21,
    M2 and M3)."""

    def test_spliced_rows_name_their_source(self):
        with tempfile.TemporaryDirectory() as scratch:
            log = Path(scratch) / "old.log"
            log.write_text(json.dumps({"name": "a", "pass": True}) + "\n"
                           + json.dumps({"name": "b", "pass": False, "spliced": True,
                                         "spliced_from": "build/older.log"}) + "\n")
            rows = run.spliced_rows(log, ["a", "b", "c"])
            self.assertEqual(set(rows), {"a", "b"})
            self.assertEqual((rows["a"]["spliced_from"], rows["b"]["spliced_from"]), (str(log), "build/older.log"))

    def test_the_rules_of_a_spliced_run(self):
        rows = [{"new_findings_strict": {}}, {"new_findings_strict": {}, "strict_rule": 3}, {"error": "x"}]
        self.assertEqual(run.rules_of(rows), {1: 1, 3: 1})
        self.assertEqual(compare.rule_mix_text(rows), "rules 1 (1 rows), 3 (1 rows)")

    def test_the_stretch_cap_is_the_binarys(self):
        self.assertEqual(run.wall_stretch_max({}), run.WALL_STRETCH_MAX)
        self.assertEqual(run.wall_stretch_max({"PCB_WALL_STRETCH_MAX": "6"}), 6.0)
        for text in ("0.5", "x", "inf"):
            self.assertEqual(run.wall_stretch_max({"PCB_WALL_STRETCH_MAX": text}), run.WALL_STRETCH_MAX, text)


class FinalLadder(unittest.TestCase):
    """The layout's final ladder's work (review 2026-10-21, H3)."""

    def test_from_the_report(self):
        self.assertEqual(run.final_ladder_budget({"final_ladder_budget_seconds": 917.8}, "", 1500),
                         (917.8, "report"))
        self.assertEqual(run.final_ladder_budget({"final_ladder_budget_seconds": None}, "", 1500), (None, "report"))

    def test_from_an_older_layouts_log(self):
        log = ("layout: placed (199 s, work 56 s, 14 seeds kept of 14 run, seed 12 kept; 80 s more of it in the "
               "pool of seeds, 623 s of seeds in all)\nlayout: routed (489 s, work 502 s)\n"
               "layout: 918 s of work left: the narrow-signal attempt instead of the ladder (pours connected)\n")
        budget, source = run.final_ladder_budget({"work_seconds": 502.2}, log, 1500)
        self.assertEqual((round(budget, 1), source), (917.8, "log"))
        self.assertEqual(run.final_ladder_budget({"work_seconds": 40.0}, "layout: routed (3 s, work 40 s)\n"
                                                 "layout: done\n", 1500), (None, "log"))


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


class Order(unittest.TestCase):
    def test_a_tier_starts_the_longest_or_the_largest_first(self):
        tasks = [{"name": "a", "peak_rss_mb": 500, "expected_seconds": 900.0},
                 {"name": "b", "peak_rss_mb": 6500, "expected_seconds": 100.0},
                 {"name": "c", "expected_seconds": 300.0},
                 {"name": "d", "peak_rss_mb": 6500, "expected_seconds": 800.0}]
        names = lambda tasks: [task["name"] for task in tasks]  # noqa: E731
        self.assertEqual(names(run.order_tasks(tasks, "seconds-desc", True, 3072)), ["a", "d", "c", "b"])
        self.assertEqual(names(run.order_tasks(tasks, "seconds-desc", False, 3072)), ["a", "b", "c", "d"])
        # The largest first (ties: the longest); one without a peak counts --expected-gb.
        self.assertEqual(names(run.order_tasks(tasks, "peak-desc", True, 3072)), ["d", "b", "c", "a"])


class Estimates(unittest.TestCase):
    """run.py reads each board's expected peak from the sidecar
    (--estimates), falls back to the size model on the board's file and
    to --expected-gb's default, and --expected-gb overrides every one."""

    def test_the_declarations_of_a_dry_run(self):
        directory = Path(tempfile.mkdtemp(prefix="est1-run-"))
        self.addCleanup(shutil.rmtree, directory)
        tasks = []
        for name in ("small", "large", "unsized", "absent"):
            board = directory / name
            board.mkdir()
            if name != "absent":
                (board / "b.kicad_pcb").write_text(
                    '(kicad_pcb (layers (0 "F.Cu" signal) (31 "B.Cu" signal)) (footprint "R" (pad "1" smd) '
                    '(pad "2" smd)))')
            tasks.append({"name": name, "directory": str(board), "board_id": "b"})
        (directory / "tasks.json").write_text(json.dumps({"tasks": tasks}))
        sidecar = {"margin": 1.25, "model": {"layout": {"base_mb": 1000, "mb_per_pad_layer": 100.0}},
                   "boards": {"small": {"pads": 2, "layers": 2, "layout": {"peak_rss_mb": 500, "seconds": 30.0}},
                              "large": {"layout": {"peak_rss_mb": 7000, "seconds": 20.0}}}}
        (directory / "estimates.json").write_text(json.dumps(sidecar))

        def dry_run(*extra):
            result = subprocess.run([sys.executable, str(HERE / "run.py"), str(directory / "out"), "--tasks",
                                     str(directory / "tasks.json"), "--estimates", str(directory / "estimates.json"),
                                     "--binary", str(directory / "none"), "--min-free-gb", "10", "--dry-run",
                                     *extra], capture_output=True, text=True, timeout=60)
            self.assertEqual(result.returncode, 0, result.stderr)
            return result.stdout.splitlines(), result.stderr

        lines, err = dry_run("--order", "peak-desc")
        # The largest first; a board without a row by the size model on its
        # file (2 pads x 2 layers), one without a file by the default.
        self.assertEqual(lines, ["large: 7000 MB, 20 s (layout rows)",
                                 "absent: 3072 MB (the default)",
                                 "unsized: 1750 MB (size model)",
                                 "small: 500 MB, 30 s (layout rows)"])
        for part in ("2 from layout rows", "1 from size model", "1 from the default 3 GB", "the largest 6.8 GB"):
            self.assertIn(part, err)
        lines, err = dry_run("--expected-gb", "2")
        self.assertEqual(lines, ["small: 2048 MB, 30 s (--expected-gb)", "large: 2048 MB, 20 s (--expected-gb)",
                                 "unsized: 2048 MB (--expected-gb)", "absent: 2048 MB (--expected-gb)"])
        (directory / "estimates.json").unlink()
        lines, err = dry_run()
        self.assertEqual(lines, ["small: 3072 MB (the default)", "large: 3072 MB (the default)",
                                 "unsized: 3072 MB (the default)", "absent: 3072 MB (the default)"])
        self.assertIn("expected peaks (no estimates file): 4 from the default 3 GB", err)

    def test_a_board_takes_the_estimate_of_its_kind(self):
        def kind(task=None, **given):
            arguments = type("A", (), {"designer": False, "schematic": False, "tier": None, "route_from": None,
                                       **given})()
            return run.estimate_kind(task or {}, arguments)

        self.assertEqual(kind(), "layout")
        self.assertEqual(kind(tier="quick"), "layout")
        self.assertEqual(kind(tier="short"), "short")
        self.assertEqual(kind(designer=True, tier="designer"), "designer")
        self.assertEqual(kind(schematic=True, tier="schematic"), "schematic")
        self.assertEqual(kind({"mode": "route"}, tier="expansion"), "expansion")
        self.assertEqual(kind({"mode": "route"}), "expansion")
        self.assertEqual(kind(route_from=Path("build/x")), "layout")
        # The estimate's fields are not the task's identity.
        task = {"name": "a", "directory": "/nonexistent", "board_id": "b", "router": {}}
        estimated, declared = run.with_estimate(task, "layout", {"boards": {"a": {"layout": {
            "peak_rss_mb": 900, "seconds": 12.0}}}})
        self.assertEqual((estimated["peak_rss_mb"], estimated["expected_seconds"], declared),
                         (900, 12.0, (900, "layout rows")))
        self.assertEqual(compare.task_digest(estimated, None), compare.task_digest(task, None))


class MemoryLedger(unittest.TestCase):
    """The admission against the machine's memory ledger
    (harness_common.Ledger, admit, memory_room; run.MemoryGate)."""

    def setUp(self):
        self.root = Path(tempfile.mkdtemp(prefix="pcb-maker-ledger-test-"))
        self.ledger = run.harness_common.Ledger(self.root / "ledger.json")
        self.saved = {name: os.environ.get(name) for name in ("PCB_MAKER_LEDGER_TOTAL_MB",)}
        os.environ["PCB_MAKER_LEDGER_TOTAL_MB"] = "4000"
        self.children = []

    def tearDown(self):
        for child in self.children:
            child.kill()
            child.wait()
        for name, value in self.saved.items():
            if value is None:
                os.environ.pop(name, None)
            else:
                os.environ[name] = value
        shutil.rmtree(self.root)

    def sleeper(self):
        child = subprocess.Popen(["sleep", "60"])
        self.children.append(child)
        return child

    def test_waits_while_the_reservations_fill_the_machine_and_goes_when_one_ends(self):
        other = self.sleeper()
        self.ledger.reserve(3000, "another run", pid=other.pid)
        gate = run.MemoryGate(0, ledger=self.ledger, poll_seconds=0.05)
        small = gate.admit("small", 500)
        self.assertEqual(sum(r["megabytes"] for r in self.ledger.live()), 3500)
        admitted = []
        waiter = run.threading.Thread(target=lambda: admitted.append(gate.admit("large", 1500)))
        waiter.start()
        time.sleep(0.5)
        self.assertEqual(admitted, [], "3000 + 500 + 1500 MB do not fit 4000")
        other.kill()
        other.wait()
        waiter.join(10)
        self.assertEqual(len(admitted), 1)
        labels = sorted(r["label"] for r in self.ledger.live())
        self.assertEqual(labels, ["run.py: large (expected peak)", "run.py: small (expected peak)"])
        gate.done(small)
        gate.done(admitted[0])
        self.assertEqual(self.ledger.live(), [])

    def test_alone_a_board_larger_than_the_machine_still_runs(self):
        gate = run.MemoryGate(0, ledger=self.ledger, poll_seconds=0.05)
        token = gate.admit("huge", 9000)
        self.assertEqual([r["megabytes"] for r in self.ledger.live()], [9000])
        gate.done(token)

    def test_the_reservation_goes_with_the_binary_it_was_made_for(self):
        gate = run.MemoryGate(0, ledger=self.ledger, poll_seconds=0.05)
        token = gate.admit("board", 1000)
        binary = self.sleeper()
        gate.started(token, binary.pid)
        live = self.ledger.live()
        self.assertEqual([(r["pid"], r["megabytes"]) for r in live], [(binary.pid, 1000)])
        self.assertEqual(live[0]["start_ticks"], run.harness_common.start_ticks(binary.pid))
        # The binary's death releases it (the pid rule), done() then finds nothing.
        binary.kill()
        binary.wait()
        self.assertEqual(self.ledger.live(), [])
        gate.done(token)
        # A binary that reserved for itself first: the harness's goes.
        token = gate.admit("board", 1000)
        binary = self.sleeper()
        self.ledger.reserve(700, "route-kicad-board board", pid=binary.pid)
        gate.started(token, binary.pid)
        self.assertEqual([r["megabytes"] for r in self.ledger.live()], [700])
        gate.done(token)

    def test_the_floor_counts_what_the_reservations_still_grow_by(self):
        room = run.harness_common.memory_room
        reservations = [{"pid": 11, "megabytes": 3000.0}, {"pid": 12, "megabytes": 2000.0}]
        resident = {11: 1000.0, 12: 2500.0}.get
        # Growth: 2000 (11) and 0 (12, past its reservation); 10000 available.
        fits, text = room(reservations, 1000, 7000, None, 10000, resident=resident, own_pid=1)
        self.assertTrue(fits, text)
        fits, _ = room(reservations, 1001, 7000, None, 10000, resident=resident, own_pid=1)
        self.assertFalse(fits)
        # The ledger: 5000 reserved, 1000 more fit 6000, 1001 do not.
        self.assertTrue(room(reservations, 1000, 0, 6000, None, resident=resident)[0])
        self.assertFalse(room(reservations, 1001, 0, 6000, None, resident=resident)[0])
        # Nothing reserved: anything runs alone.
        self.assertTrue(room([], 1e9, 7000, 6000, 0)[0])
        self.assertIn("4.9 GB reserved by 2 run(s) of 5.9 GB", room(reservations, 1, 0, 6000, None,
                                                                     resident=resident)[1])

    def test_ordered_admission_does_not_pass_the_request_that_waited_longest(self):
        allows = run.harness_common.order_allows

        def holder(pid, megabytes, since):
            return {"pid": pid, "started": since, "megabytes": megabytes, "held_since": since, "label": ""}

        def waiter(pid, wants, since):
            return {"pid": pid, "started": since, "megabytes": 0.0, "waiting_mb": wants, "waiting_since": since,
                    "label": ""}

        large = waiter(13, 8000.0, 10.0)
        # No waiter or no capacity: as before.
        self.assertTrue(allows([holder(11, 6000.0, 1.0)], 4000.0, 30.0, 10000.0, own_pid=99))
        self.assertTrue(allows([holder(11, 6000.0, 1.0), large], 4000.0, 30.0, None, own_pid=99))
        # The large one waits for 11's 6000: 2000 more leave it its room once 11 ends, 2001 do not.
        self.assertTrue(allows([holder(11, 6000.0, 1.0), large], 2000.0, 30.0, 10000.0, own_pid=99))
        self.assertFalse(allows([holder(11, 6000.0, 1.0), large], 2001.0, 30.0, 10000.0, own_pid=99))
        # Runs admitted while it waits count.
        crowded = [holder(11, 6000.0, 1.0), holder(12, 1500.0, 20.0), large]
        self.assertTrue(allows(crowded, 500.0, 30.0, 10000.0, own_pid=99))
        self.assertFalse(allows(crowded, 501.0, 30.0, 10000.0, own_pid=99))
        # An older request goes by the capacity alone.
        self.assertTrue(allows(crowded, 9000.0, 5.0, 10000.0, own_pid=99))
        # The oldest fits now: the next slot is its own.
        fits = [holder(11, 1000.0, 1.0), holder(12, 500.0, 20.0), large]
        self.assertFalse(allows(fits, 1000.0, 30.0, 10000.0, own_pid=99))
        floored = dict(large, waits_on_floor=True)
        self.assertTrue(allows([holder(11, 1000.0, 1.0), holder(12, 500.0, 20.0), floored], 1500.0, 30.0, 10000.0,
                               own_pid=99))
        # Larger than the capacity: it runs alone, nothing newer passes it.
        self.assertFalse(allows([waiter(13, 12000.0, 10.0)], 1.0, 30.0, 10000.0, own_pid=99))
        # An older binary's entries: `started` stands in.
        old = [{"pid": 11, "started": 1.0, "megabytes": 6000.0}, {"pid": 13, "started": 10.0, "megabytes": 0.0,
                                                                   "waiting_mb": 8000.0}]
        self.assertTrue(allows(old, 2000.0, 30.0, 10000.0, own_pid=99))
        self.assertFalse(allows(old, 2001.0, 30.0, 10000.0, own_pid=99))

    def test_a_waiting_request_is_in_the_ledger_and_newer_ones_let_it_go_first(self):
        other = self.sleeper()
        self.ledger.reserve(3000, "another run", pid=other.pid)
        gate = run.MemoryGate(0, ledger=self.ledger, poll_seconds=0.05)
        admitted = []
        large = run.threading.Thread(target=lambda: admitted.append(("large", gate.admit("large", 3500))))
        large.start()
        deadline = time.time() + 10
        while not any(r.get("waiting_mb") == 3500 for r in self.ledger.live()):
            self.assertLess(time.time(), deadline)
            time.sleep(0.01)
        live = self.ledger.live()
        self.assertIn(("run.py: large (expected peak)", 0.0, 3500.0),
                      [(r["label"], r["megabytes"], r.get("waiting_mb")) for r in live])
        # 600 fit the capacity beside the 3000 (3600 of 4000) but would
        # leave the large request short once the 3000 end (600 + 3500): the
        # small one waits, behind it; 500 leave it its room and go.
        key, text = run.harness_common.try_admit(self.ledger, 600, "small")
        self.assertIsNone(key)
        self.assertIn("behind an older request: run.py: large (expected peak)", text)
        key, _ = run.harness_common.try_admit(self.ledger, 500, "small")
        self.assertIsNotNone(key)
        self.ledger.release(key)
        small = run.threading.Thread(target=lambda: admitted.append(("small", gate.admit("small", 600))))
        small.start()
        time.sleep(0.3)
        other.kill()
        other.wait()
        large.join(10)
        time.sleep(0.3)
        # The large one went first; the small one waits for it (3500 + 600 > 4000).
        self.assertEqual([name for name, _ in admitted], ["large"])
        gate.done(admitted[0][1])
        small.join(10)
        self.assertEqual([name for name, _ in admitted], ["large", "small"])
        gate.done(admitted[1][1])
        self.assertEqual(self.ledger.live(), [])

    def test_two_processes_are_admitted_in_the_order_they_began_waiting(self):
        # Two harness processes: the older one's 3500 MB do not fit beside a
        # holder's 3000 of 4000; the newer one's 900 do (3900) but would
        # leave the older short once the holder ends (900 + 3500): the newer
        # one waits for the older one to end (it went first before
        # 2026-10-24).
        other = self.sleeper()
        self.ledger.reserve(3000, "another run", pid=other.pid)
        code = ("import sys, time; sys.path.insert(0, {benchmarks!r}); import harness_common as h; "
                "ledger = h.Ledger({path!r}); key = h.admit(ledger, {mb}, {label!r}, poll_seconds=0.05); "
                "print(time.time(), flush=True); time.sleep(0.5); ledger.release(key)")
        benchmarks = str(HERE.parent)
        environment = dict(os.environ, PCB_MAKER_LEDGER_TOTAL_MB="4000")

        def start(mb, label):
            child = subprocess.Popen([sys.executable, "-c", code.format(benchmarks=benchmarks, path=str(
                self.ledger.path), mb=mb, label=label)], stdout=subprocess.PIPE, text=True, env=environment)
            self.children.append(child)
            return child

        def waiting(label):
            return any(r["label"] == label and r.get("waiting_mb") for r in self.ledger.live())

        older = start(3500, "older")
        deadline = time.time() + 20
        while not waiting("older"):
            self.assertLess(time.time(), deadline)
            time.sleep(0.01)
        newer = start(900, "newer")
        while not waiting("newer"):
            self.assertLess(time.time(), deadline)
            time.sleep(0.01)
        other.kill()
        other.wait()
        older_at = float(older.communicate(timeout=20)[0])
        newer_at = float(newer.communicate(timeout=20)[0])
        # The newer one began only after the older one released (0.5 s on).
        self.assertGreaterEqual(newer_at, older_at + 0.5)
        self.assertEqual(self.ledger.live(), [])

    @staticmethod
    def starve(drain_after, horizon):
        """The machine of 2026-10-25, second by second (as ledger.rs's
        `starve`): a ledger of 56 000 MB, 30 000 MB available beside the
        reservations (each run resident at its reservation), requests of
        2000 MB, one every 5 s, each running 60 s and looking every 2 s
        while it waits (the ledger, the order and the drain, no floor of
        their own), and from 300 s a harness's request of 19 000 MB with
        a floor of 10 000 MB looking every 15 s (memory_room). Returns how
        long it waited (None: not within `horizon` s) and how many small
        ones ran before it."""
        h = run.harness_common
        capacity, free, floor, large, small, arrives = 56000.0, 30000.0, 10000.0, 19000.0, 2000.0, 300
        book = []  # [reservation, end]
        ran = 0
        resident = lambda pid: next(r["megabytes"] for r, _ in book if r["pid"] == pid)  # noqa: E731
        for t in range(arrives + horizon):
            book = [entry for entry in book if entry[1] is None or entry[1] > t]
            if t == arrives:
                book.append([{"pid": 1, "started": t, "megabytes": 0.0, "label": "katia", "waiting_mb": large,
                              "waiting_since": t, "waits_on_floor": True, "floor_mb": floor,
                              "drain_after": drain_after}, None])
            if t % 5 == 0 and t > 0:
                book.append([{"pid": 100 + t, "started": t, "megabytes": 0.0, "label": "route",
                              "waiting_mb": small, "waiting_since": t}, None])
            for entry in [e for e in book if e[0].get("waiting_mb") is not None]:
                me = entry[0]
                waited = t - me["waiting_since"]
                if waited % (15 if me["pid"] == 1 else 2):
                    continue
                others = [r for r, _ in book if r is not me]
                available = free - sum(r["megabytes"] for r in others)
                if me["pid"] == 1:
                    admitted = h.memory_room(others, large, floor, capacity, available, resident=resident,
                                             own_pid=1)[0]
                else:
                    since = me["waiting_since"]
                    drained = h.drained_for(others, since, t, own_pid=me["pid"], capacity_mb=capacity,
                                            available_mb=available)
                    admitted = (h.memory_room(others, small, 0.0, capacity, None)[0]
                                and h.order_allows(others, small, since, capacity, own_pid=me["pid"])
                                and (drained is None
                                     or h.drain_allows(drained, others, small, capacity, available)))
                if admitted:
                    if me["pid"] == 1:
                        return t - arrives, ran
                    me.update(megabytes=me.pop("waiting_mb"), held_since=t)
                    entry[1] = t + 60
                    ran += 1
        return None, ran

    def test_a_stream_of_small_requests_starves_a_waiter_until_the_ledger_drains(self):
        # Without the drain (the rule of 2026-10-24) the large request never
        # starts in four hours; with it, once it has waited 600 s, the
        # running set ends within 60 s and it goes at its next look.
        started, ran = self.starve(float("inf"), 4 * 3600)
        self.assertIsNone(started)
        self.assertGreater(ran, 2800)
        started, ran = self.starve(600.0, 4 * 3600)
        self.assertIsNotNone(started)
        self.assertTrue(600 <= started <= 600 + 60 + 15, started)
        self.assertEqual(ran, 179)

    @staticmethod
    def tonight(small_mb, horizon=4 * 3600):
        """The machine of 2026-10-25 22:30, second by second (as ledger.rs's
        `tonight`): a ledger of 58 066 MB, 42 000 MB available beside the
        reservations (each run resident at its reservation), a run of
        8800 MB holding until 900 s, three nightly boards waiting from 0 s
        (27 876, 24 724 and 23 215 MB expected, a floor of 10 240 MB,
        looking every 15 s, each running 1500 s) and three agents' chains of
        binaries (2000 MB, 120 s each, the next one as one ends, looking
        every 2 s). `small_mb`: the drain's small-request bound (0: the
        rule of 2026-10-25). Returns the nightly boards' starts, the
        binaries' longest wait and how many ran."""
        h = run.harness_common
        capacity, free, floor = 58066.0, 42000.0, 10240.0
        book = [[{"pid": 2, "started": 0, "megabytes": 8800.0, "label": "layout", "held_since": 0,
                  "expected_end": 900.0}, 900, None]]
        for pid, megabytes in ((11, 27876.0), (12, 24724.0), (13, 23215.0)):
            book.append([{"pid": pid, "started": 0, "megabytes": 0.0, "label": "nightly", "waiting_mb": megabytes,
                          "waiting_since": 0, "floor_mb": floor, "waits_on_floor": True, "priority": "nightly",
                          "drain_after": 0.0}, None, None])
        starts, longest, ran, next_pid = [], 0, 0, 100
        for t in range(horizon):
            book = [entry for entry in book if entry[1] is None or entry[1] > t]
            for chain in range(3):
                if not any(entry[2] == chain for entry in book):
                    next_pid += 1
                    book.append([{"pid": next_pid, "started": t, "megabytes": 0.0, "label": "route",
                                  "waiting_mb": 2000.0, "waiting_since": t}, None, chain])
            for entry in [e for e in book if e[0].get("waiting_mb") is not None]:
                me = entry[0]
                nightly = me.get("priority") == "nightly"
                if (t - me["waiting_since"]) % (15 if nightly else 2):
                    continue
                wanted, since = me["waiting_mb"], me["waiting_since"]
                others = [r for r, _, _ in book if r is not me]
                available = free - sum(r["megabytes"] for r in others)
                held = {r["pid"]: r["megabytes"] for r in others}
                fits = (h.memory_room(others, wanted, floor, capacity, available, resident=held.get,
                                      own_pid=me["pid"])[0] if nightly
                        else h.memory_room(others, wanted, 0.0, capacity, None)[0])
                priority = me.get("priority")
                drained = h.drained_for(others, since, t, own_pid=me["pid"], priority=priority, capacity_mb=capacity,
                                        available_mb=available)
                end = t + 1500.0 if nightly else None
                if (fits and h.order_allows(others, wanted, since, capacity, own_pid=me["pid"], priority=priority)
                        and (drained is None or h.drain_allows(drained, others, wanted, capacity, available, end,
                                                               small_mb=small_mb))):
                    me.update(megabytes=me.pop("waiting_mb"), held_since=t)
                    if nightly:
                        me["expected_end"] = end
                        starts.append(t)
                    else:
                        longest = max(longest, t - since)
                        ran += 1
                    entry[1] = t + (1500 if nightly else 120)
        return starts, longest, ran

    def test_small_requests_go_while_the_waiter_waits_for_a_large_run_to_end(self):
        # The rule of 2026-10-25: a binary waits up to 3900 s (from the
        # first board's end to the third board's start every request is
        # held: each board needs 33-38 GB available, the machine has 42).
        self.assertEqual(self.tonight(0.0), ([900, 2400, 3900], 3900, 264))
        # The feasibility cap: the binaries go beside the large runs (the
        # next board cannot fit before the running one ends) and wait at
        # most one of their own runs (the full drain before each board);
        # the boards start 60 s later.
        self.assertEqual(self.tonight(run.harness_common.SMALL_REQUEST_MB), ([960, 2460, 3960], 120, 348))

    def test_a_waiter_blocked_by_a_large_run_lets_small_requests_go(self):
        h = run.harness_common
        holder = lambda pid, megabytes: {"pid": pid, "megabytes": megabytes, "held_since": 1.0,  # noqa: E731
                                         "label": ""}
        waiter = {"pid": 13, "started": 10.0, "megabytes": 0.0, "waiting_mb": 8000.0, "waiting_since": 10.0,
                  "floor_mb": 2000.0, "waits_on_floor": True, "priority": "nightly", "label": ""}
        others = [holder(11, 5000.0), holder(12, 1000.0), waiter]
        small = h.SMALL_REQUEST_MB
        # 6000 available: with the small run gone 7000 less the floor is
        # 5000, short of 8000: the large run must end first.
        self.assertTrue(h.blocked_by_large(waiter, others, 20000.0, 6000.0))
        self.assertTrue(h.drain_allows(waiter, others, small, 20000.0, 6000.0))
        self.assertFalse(h.drain_allows(waiter, others, small + 1, 20000.0, 6000.0))
        # 9000 available: with the small run gone it fits: the drain holds
        # the small requests back too.
        self.assertFalse(h.blocked_by_large(waiter, others, 20000.0, 9000.0))
        self.assertFalse(h.drain_allows(waiter, others, 1000.0, 20000.0, 9000.0))
        # Only small runs hold memory: the drain empties the machine.
        self.assertFalse(h.blocked_by_large(waiter, [holder(12, 1000.0), waiter], 20000.0, 1000.0))

    def test_a_drain_admits_what_leaves_the_waiter_its_room_or_ends_first(self):
        h = run.harness_common
        holder = {"pid": 11, "started": 1.0, "megabytes": 4000.0, "held_since": 1.0, "expected_end": 900.0,
                  "label": ""}
        waiter = {"pid": 13, "started": 10.0, "megabytes": 0.0, "waiting_mb": 8000.0, "waiting_since": 10.0,
                  "floor_mb": 2000.0, "waits_on_floor": True, "label": ""}
        others = [holder, waiter]
        self.assertIsNone(h.drained_for(others, 30.0, 609.0, own_pid=99))
        self.assertIsNone(h.drained_for(others, 5.0, 700.0, own_pid=99))
        self.assertIs(h.drained_for(others, 30.0, 610.0, own_pid=99), waiter)
        self.assertIsNone(h.drained_for([holder, dict(waiter, drain_after=1000.0)], 30.0, 610.0, own_pid=99))
        # 12 000 available: 2000 leave 12 000 - 2000 - 2000 = 8000, its room.
        self.assertTrue(h.drain_allows(waiter, others, 2000.0, 20000.0, 12000.0))
        self.assertFalse(h.drain_allows(waiter, others, 2001.0, 20000.0, 12000.0))
        self.assertFalse(h.drain_allows(waiter, others, 8001.0, 20000.0, None))
        # Ends before every holder: goes; after, or beside a holder of unknown end: waits.
        self.assertTrue(h.drain_allows(waiter, others, 5000.0, 20000.0, 12000.0, expected_end=900.0))
        self.assertFalse(h.drain_allows(waiter, others, 5000.0, 20000.0, 12000.0, expected_end=901.0))
        unknown = [holder, {"pid": 12, "megabytes": 100.0, "label": ""}, waiter]
        self.assertFalse(h.drain_allows(waiter, unknown, 5000.0, 20000.0, 12000.0, expected_end=800.0))
        # The nightly's: first among the waiters, drained for at once, no end exception.
        nightly = dict(waiter, pid=14, waiting_since=20.0, priority="nightly")
        both = [holder, waiter, nightly]
        self.assertIs(h.oldest_waiter(both), nightly)
        self.assertIs(h.drained_for(both, 21.0, 21.0, own_pid=99), nightly)
        self.assertFalse(h.drain_allows(nightly, both, 5000.0, 20000.0, 12000.0, expected_end=100.0))
        # A nightly request is not behind an older one that is not.
        self.assertTrue(h.order_allows([holder, waiter], 9000.0, 30.0, 10000.0, own_pid=99, priority="nightly"))
        self.assertIsNone(h.drained_for([holder, waiter], 30.0, 9999.0, own_pid=99, priority="nightly"))

    def test_the_ledger_drains_only_for_a_waiter_it_could_start(self):
        # The ledger of 2026-10-26 (58 066 MB): the old nightly harness's
        # waiter (356c8db: no floor_mb, no refreshed, waits_on_floor) of
        # 24 724 MB waiting since 0 s, 26 GB reserved by two runs (the
        # smaller 3 GB, holding for another hour), 31 000 MB available less
        # growth, a request of 7 GB (katia). ledger.rs: the same test.
        h = run.harness_common
        holder = lambda pid, megabytes, since, end: {"pid": pid, "started": since, "megabytes": megabytes,  # noqa: E731
                                                     "held_since": since, "expected_end": end, "label": ""}
        waiter = {"pid": 4134524, "started": 0.0, "megabytes": 0.0, "waiting_mb": 24724.0, "waiting_since": 0.0,
                  "waits_on_floor": True, "label": "run.py: Goga64__ULK__ULK_sl_PG1316s (expected peak)"}
        capacity, available, katia, now = 58066.0, 31000.0, 7168.0, 1200.0
        self.assertEqual(h.assumed_floor_mb(waiter), h.DEFAULT_FLOOR_MB)
        # The runs began holding before its wait began.
        others = [holder(11, 3072.0, -600.0, now + 3600), holder(12, 23552.0, -600.0, now + 7200), waiter]
        # The drain of 2026-10-25 held the 7 GB request back.
        self.assertFalse(h.drain_allows(waiter, others, katia, capacity, available, expected_end=now + 4000))
        # Now: with no newer run to go its room is 31 000 - 10 240 = 20 760:
        # the drain ends, and ordered admission lets the request go.
        self.assertEqual(h.drain_state(waiter, others, now, capacity, available), ("cannot start", 20760.0))
        self.assertIsNone(h.drained_for(others, now, now, own_pid=99, capacity_mb=capacity, available_mb=available))
        self.assertTrue(h.order_allows(others, katia, now, capacity, own_pid=99))
        # The live ledger's case: every run began after its wait (24 GB): it
        # could start once they end, the ledger drains for it ...
        others = [holder(11, 3072.0, 600.0, now + 3600), holder(12, 21504.0, 600.0, now + 7200), waiter]
        self.assertEqual(h.drain_state(waiter, others, now, capacity, available), ("drains", None))
        drained = h.drained_for(others, now, now, own_pid=99, capacity_mb=capacity, available_mb=available)
        self.assertIs(drained, waiter)
        self.assertFalse(h.drain_allows(drained, others, katia, capacity, available))
        # ... for at most 2 hours of its wait; then ordered admission alone,
        # and the waiter keeps its place (16 GB still waits).
        later = h.DEFAULT_DRAIN_MAX_SECONDS + 1
        self.assertEqual(h.drain_state(waiter, others, later, capacity, available),
                         ("past bound", h.DEFAULT_DRAIN_MAX_SECONDS))
        self.assertIsNone(h.drained_for(others, later, later, own_pid=99, capacity_mb=capacity,
                                        available_mb=available))
        self.assertTrue(h.order_allows(others, katia, later, capacity, own_pid=99))
        self.assertFalse(h.order_allows(others, 16384.0, later, capacity, own_pid=99))
        self.assertEqual(h.drain_state(dict(waiter, drain_max=10000.0), others, later, capacity, available)[0],
                         "drains")
        # A new writer's entry: its own floor or none; stale after 120 s.
        fresh = dict(waiter, refreshed=now - h.STALE_AFTER_SECONDS)
        self.assertEqual(h.assumed_floor_mb(fresh), 0.0)
        self.assertEqual(h.drain_state(fresh, others, now, capacity, available)[0], "drains")
        stale = dict(waiter, refreshed=now - h.STALE_AFTER_SECONDS - 1)
        self.assertEqual(h.drain_state(stale, others, now, capacity, available), ("stale", h.STALE_AFTER_SECONDS + 1))
        self.assertEqual(h.drain_state(waiter, others, 599.0, capacity, available)[0], "not yet")

    def test_waiting_entries_carry_what_the_drain_reads_and_newer_requests_wait(self):
        other = self.sleeper()
        self.ledger.reserve(3000, "another run", pid=other.pid)
        h = run.harness_common
        # A request that cannot fit (3000 + 3500 of 4000), waiting since 700 s ago.
        since = time.time() - 700
        key, _ = h.try_admit(self.ledger, 3500, "large", 1024, since=since, drain_after=600.0,
                             priority="nightly", drain_max=7200.0)
        self.assertIsNone(key)
        entry = next(r for r in self.ledger.live() if r["label"] == "large")
        self.assertEqual((entry["floor_mb"], entry["drain_after"], entry["priority"], entry["drain_max"]),
                         (1024.0, 600.0, "nightly", 7200.0))
        self.assertAlmostEqual(entry["refreshed"], time.time(), delta=5)

        def large(**fields):
            self.ledger.update(lambda reservations: [r.update(**fields) for r in reservations
                                                      if r["label"] == "large"])

        def holder_since(at):
            self.ledger.update(lambda reservations: [r.update(held_since=at) for r in reservations
                                                      if r["label"] == "another run"])
        # The holder began after the large one began waiting: the ledger
        # could start it once the holder ends and drains for it; 500 more
        # wait (by the order as well: 3000 + 500 + 3500), and say so.
        holder_since(since + 10)
        key, text = h.try_admit(self.ledger, 500, "small")
        self.assertIsNone(key)
        self.assertIn("the ledger drains for it", text)
        # Stale (not rewritten for 200 s), or past its bound: not drained for.
        large(refreshed=time.time() - 200)
        self.assertIn("(stale)", h.try_admit(self.ledger, 500, "small")[1])
        large(refreshed=time.time(), drain_max=100.0)
        self.assertIn("past the drain's bound of 100 s", h.try_admit(self.ledger, 500, "small")[1])
        large(drain_max=7200.0)
        # The holder held before the large one began waiting: the ledger
        # cannot start it before the holder ends (500 of 4000 beside the
        # holder's 3000): no drain, and the order lets 500 more go (the
        # large one fits once the holder ends: 500 + 3500).
        holder_since(since - 10)
        key, text = h.try_admit(self.ledger, 500, "small")
        self.assertIsNotNone(key, text)
        self.ledger.release(key)
        # Before its drain_after (and not the nightly's) it is not drained
        # for either.
        holder_since(since + 10)
        large(drain_after=800.0, priority=None)
        self.assertNotIn("drain", h.try_admit(self.ledger, 500, "small")[1])
        # Admitted, a reservation carries its expected end and priority.
        h.withdraw(self.ledger, "large", since)
        key, _ = h.try_admit(self.ledger, 500, "small", expected_seconds=60, priority="nightly")
        self.assertIsNotNone(key)
        entry = next(r for r in self.ledger.live() if r["label"] == "small")
        self.assertAlmostEqual(entry["expected_end"], time.time() + 60, delta=5)
        self.assertEqual(entry["priority"], "nightly")
        self.ledger.release(key)

    def test_dead_and_reused_pids_are_dropped_and_other_fields_kept(self):
        child = self.sleeper()
        self.ledger.reserve(100, "live", pid=child.pid)
        stored = json.loads((self.root / "ledger.json").read_text())
        stored["reservations"][0]["waiting_mb"] = 300.0
        stored["reservations"].append({"pid": child.pid, "started": 1.0, "megabytes": 5.0, "label": "reused",
                                       "start_ticks": run.harness_common.start_ticks(child.pid) + 1})
        stored["reservations"].append({"pid": 4194311, "started": 1.0, "megabytes": 5.0, "label": "dead"})
        (self.root / "ledger.json").write_text(json.dumps(stored))
        live = self.ledger.live()
        self.assertEqual([(r["label"], r.get("waiting_mb")) for r in live], [("live", 300.0)])

    def test_the_tier_lock_is_opt_in(self):
        result = subprocess.run([sys.executable, str(HERE / "run.py"), "--help"], capture_output=True, text=True,
                                timeout=60)
        self.assertIn("--lock", result.stdout)
        self.assertIn("memory ledger", result.stdout)


def footprint(reference, x, y, layer="F.Cu", net=True, locked=False, pad="smd", angle=None):
    at = f"{x} {y}" + (f" {angle}" if angle is not None else "")
    layers = '"*.Cu" "*.Mask"' if pad == "thru_hole" else '"F.Cu" "F.Mask"'
    drill = " (drill 1)" if pad == "thru_hole" else ""
    return (f'  (footprint "R"{" (locked yes)" if locked else ""} (layer "{layer}") (at {at})\n'
            f'    (property "Reference" "{reference}" (at 0 0 0) (layer "F.SilkS"))\n'
            f'    (fp_rect (start -2 -1) (end 2 1) (layer "F.CrtYd"))\n'
            f'    (pad "1" {pad} rect (at -1 0) (size 1 1){drill} (layers {layers})'
            f'{chr(10) + "      (net 1 " + chr(34) + "N" + chr(34) + ")" if net else ""})\n  )\n')


def text(content, x, y, layer="F.SilkS", extra=""):
    return f'  (gr_text "{content}" (at {x} {y}) (layer "{layer}") (effects (font (size 1 1) (thickness 0.15)){extra}))\n'


def line(x0, y0, x1, y1, layer="F.SilkS"):
    return f'  (gr_line (start {x0} {y0}) (end {x1} {y1}) (stroke (width 0.12) (type solid)) (layer "{layer}"))\n'


class Labels(unittest.TestCase):
    """unplace drops the board's texts and silkscreen graphics that label a
    movable part at the designer's position (docs/placer.md, "The board's
    own texts and silkscreen graphics"): stacked, the part claims nothing
    and the label would mark the designer's spot."""

    def unplaced(self, items, remove_outline=False):
        with tempfile.TemporaryDirectory() as directory:
            board = Path(directory) / "b.kicad_pcb"
            board.write_text('(kicad_pcb (version 20240108)\n' + "".join(items) + ')\n')
            netless, dropped = run.unplace(board, (50, 50), remove_outline)
            return board.read_text(), dropped

    def test_a_text_over_a_movable_part_goes_one_over_free_space_or_a_fixed_part_stays(self):
        result, dropped = self.unplaced([
            footprint("J1", 10, 10), footprint("J2", 30, 10, locked=True),
            text("GND", 10.5, 10), text("VCC", 30, 10), text("logo", 20, 30)])
        self.assertEqual(dropped, 1)
        self.assertNotIn('"GND"', result)
        self.assertIn('"VCC"', result)
        self.assertIn('"logo"', result)
        self.assertIn("(at 50 50)", result)

    def test_parts_sharing_an_origin_claim_nothing(self):
        result, dropped = self.unplaced([footprint("J1", 10, 10), footprint("J2", 10, 10), text("GND", 10, 10)])
        self.assertEqual(dropped, 0)
        self.assertIn('"GND"', result)

    def test_a_cluster_of_graphics_goes_whole_and_counts_once(self):
        result, dropped = self.unplaced([
            footprint("U1", 10, 10), line(9, 9, 11, 9), line(11.3, 9, 16, 9), line(30, 30, 31, 30)])
        self.assertEqual(dropped, 1)
        self.assertNotIn("(start 11.3 9)", result)
        self.assertIn("(start 30 30)", result)

    def test_the_other_side_counts_holes_and_through_pads_only(self):
        result, dropped = self.unplaced([
            footprint("U1", 10, 10), footprint("J1", 30, 10, pad="thru_hole"),
            text("A", 10, 10, "B.SilkS"), text("B", 29, 10, "B.SilkS")])
        self.assertEqual(dropped, 1)
        self.assertIn('"A"', result)
        self.assertNotIn('"B"', result)

    def test_hidden_texts_and_texts_inside_dimensions_stay(self):
        dimension = '  (dimension (type aligned) (layer "F.SilkS")\n' + text("5 mm", 10, 10) + "  )\n"
        result, dropped = self.unplaced([
            footprint("U1", 10, 10), text("H", 10, 10, extra=" (hide yes)"), dimension])
        self.assertEqual(dropped, 0)
        self.assertIn('"H"', result)
        self.assertIn('"5 mm"', result)

    def test_a_turned_part_s_box_turns_and_the_outline_still_goes(self):
        # J1 turned by 90 degrees: its 4 x 2 mm courtyard is 2 x 4 mm.
        result, dropped = self.unplaced([
            footprint("J1", 10, 10, angle=90), text("X", 10, 12.5), text("Y", 11.8, 10),
            line(0, 0, 100, 0, "Edge.Cuts")], remove_outline=True)
        self.assertEqual(dropped, 1)
        self.assertNotIn('"X"', result)
        self.assertIn('"Y"', result)
        self.assertNotIn("Edge.Cuts", result)


if __name__ == "__main__":
    unittest.main()
