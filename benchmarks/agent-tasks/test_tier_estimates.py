# Copyright (C) 2026 Toit contributors.
import json
import math
import os
import shutil
import tempfile
import unittest
from pathlib import Path

import tier_estimates


class TierEstimates(unittest.TestCase):
    def test_the_largest_measured_peak_and_seconds_replace_the_estimate(self):
        with tempfile.TemporaryDirectory() as directory:
            log = Path(directory) / "designer.log"
            rows = [{"name": "a", "placement": "designer", "peak_rss_mb": 700, "seconds": 99.1},
                    {"name": "a", "placement": "designer", "peak_rss_mb": 1500, "seconds": 80.0},
                    {"name": "a", "placement": "auto", "peak_rss_mb": 9000, "seconds": 900.0},
                    {"name": "b", "placement": "designer", "peak_rss_mb": None}]
            log.write_text("run.py: a line that is not a row\n" + "".join(json.dumps(r) + "\n" for r in rows))
            found = tier_estimates.measured([log], "designer")
            self.assertEqual(found, {"a": {"peak_rss_mb": 1500, "seconds": 99.1, "rows": 2}})
            tier = {"boards": {"a": {"peak_rss_mb": 23215, "seconds": 1523.8, "v17": {"routed": "68/70"}},
                               "b": {"peak_rss_mb": 300, "seconds": 10.0}}}
            self.assertEqual(tier_estimates.update(tier, found), [("a", 23215, 1500)])
            self.assertEqual(tier["boards"]["a"], {"peak_rss_mb": 1500, "seconds": 99.1, "v17": {"routed": "68/70"}})
            self.assertEqual(tier["boards"]["b"], {"peak_rss_mb": 300, "seconds": 10.0})


def write_log(path, rows, mtime):
    path.write_text("run.py: a line that is not a row\n" + "".join(json.dumps(r) + "\n" for r in rows))
    os.utime(path, (mtime, mtime))
    return path


BOARD = """(kicad_pcb (version 20240108)
  (layers
    (0 "F.Cu" signal)
    (1 "In1.Cu" power)
    (31 "B.Cu" signal)
    (44 "Edge.Cuts" user)
  )
  (footprint "R" (pad "1" smd rect (layers "F.Cu")) (pad "2" smd rect (layers "F.Cu")))
  (footprint "C" (pad "1" smd rect (layers "B.Cu")))
)
"""


class Sidecar(unittest.TestCase):
    """estimates.json: refresh (the newest sources' rows, the margin, the
    size model), loading, the fallbacks and the override."""

    def setUp(self):
        self.directory = Path(tempfile.mkdtemp(prefix="est1-sidecar-"))
        self.addCleanup(shutil.rmtree, self.directory)

    def test_refresh_keeps_the_newest_sources_and_takes_their_largest_peak_times_the_margin(self):
        d = self.directory
        old = write_log(d / "old.log", [{"name": "a", "mode": "layout", "peak_rss_mb": 9000, "seconds": 900.0}], 1000)
        logs = [write_log(d / f"sweep{i}.log", [{"name": "a", "mode": "layout", "peak_rss_mb": 1000 + i,
                                                 "seconds": 100.0 + i}], 2000 + i) for i in range(3)]
        mixed = write_log(d / "mixed.log", [
            {"name": "a", "mode": "route", "placement": "designer", "peak_rss_mb": 800, "seconds": 50.0},
            {"name": "b", "mode": "layout", "peak_rss_mb": 4000, "seconds": 10.0, "spliced": True},
            {"name": "c", "mode": "route", "peak_rss_mb": None}], 3000)
        sources = [(old, "layout")] + [(log, "layout") for log in logs] + [(mixed, "designer")]
        sidecar = tier_estimates.refresh([None], sources, {"a": (3, 3), "z": (100, 4)})
        a = sidecar["boards"]["a"]
        # The newest three sources: the old 9000 MB row is out of the window.
        self.assertEqual(a["layout"]["measured_mb"], 1002)
        self.assertEqual(a["layout"]["peak_rss_mb"], math.ceil(1002 * 1.25))
        self.assertEqual(a["layout"]["seconds"], 102.0)
        self.assertEqual([row[0] for row in a["layout"]["rows"]], [str(log) for log in reversed(logs)])
        self.assertEqual(a["designer"]["peak_rss_mb"], 1000)
        # Spliced rows and rows without a peak do not count; a sized board
        # without a row stays (the size model's).
        self.assertNotIn("b", sidecar["boards"])
        self.assertNotIn("c", sidecar["boards"])
        self.assertEqual(sidecar["boards"]["z"], {"pads": 100, "layers": 4})
        self.assertNotIn(str(old), sidecar["sources"])
        # Written and read back; a refresh from it with one newer log keeps
        # the rows it had, and a log read again replaces its own rows.
        path = d / "estimates.json"
        path.write_text(tier_estimates.dumps(sidecar))
        loaded = tier_estimates.load(path)
        self.assertEqual(loaded, json.loads(json.dumps(sidecar)))
        self.assertIsNone(tier_estimates.load(d / "absent.json"))
        newer = write_log(d / "sweep3.log", [{"name": "a", "mode": "layout", "peak_rss_mb": 500, "seconds": 1.0}], 4000)
        write_log(logs[2], [{"name": "a", "mode": "layout", "peak_rss_mb": 600, "seconds": 2.0}], 2002)
        again = tier_estimates.refresh([loaded], [(newer, "layout"), (logs[2], "layout")])
        self.assertEqual([row[1:] for row in again["boards"]["a"]["layout"]["rows"]],
                         [[500, 1.0], [600, 2.0], [1001, 101.0]])
        self.assertEqual(again["boards"]["a"]["designer"], a["designer"])
        self.assertEqual(again["boards"]["z"], {"pads": 100, "layers": 4})

    def test_the_overshoot_is_each_new_sources_peak_less_what_was_declared_before_it(self):
        # (since 2026-10-27, the floor's second part) Board a declared
        # 1250 MB (1000 times 1.25) before the second night, which measures
        # 1400 (150 over) and b 600 against 1000 (-400); a board nothing
        # declares (c, no size) does not count. A refresh that reads the
        # same log again compares it with the rows before it, not itself.
        d = self.directory
        first = write_log(d / "night1.log", [{"name": "a", "placement": "designer", "peak_rss_mb": 1000},
                                              {"name": "b", "placement": "designer", "peak_rss_mb": 800}], 1000)
        second = write_log(d / "night2.log", [{"name": "a", "placement": "designer", "peak_rss_mb": 1400},
                                               {"name": "b", "placement": "designer", "peak_rss_mb": 600},
                                               {"name": "c", "placement": "designer", "peak_rss_mb": 900}], 2000)
        base = tier_estimates.refresh([None], [(first, "designer")])
        self.assertEqual(base["overshoot"], {})
        sidecar = tier_estimates.refresh([base], [(second, "designer")])
        entry = sidecar["overshoot"]["designer"]
        self.assertEqual(entry["sources"][str(second)]["over"], [-400, 150])
        self.assertEqual((entry["p95_mb"], entry["rows"], entry["exceeded"]), (150, 2, 1))
        self.assertEqual(tier_estimates.refresh([sidecar], [(second, "designer")])["overshoot"], sidecar["overshoot"])
        # The newest WINDOW sources count; the quantile is at least 0; a
        # kind without one falls back (short: layout), else 0.
        self.assertEqual(tier_estimates.overshoot({"layout": {f"s{i}": {"date": f"2026-10-2{i}", "over": [-5 - i]}
                                                              for i in range(5)}})["layout"]["p95_mb"], 0)
        kept = tier_estimates.overshoot({"layout": {f"s{i}": {"date": f"2026-10-2{i}", "over": [100 * i]}
                                                    for i in range(5)}})["layout"]
        self.assertEqual((kept["rows"], kept["p95_mb"], sorted(kept["sources"])), (3, 400, ["s2", "s3", "s4"]))
        self.assertEqual(tier_estimates.overshoot_mb({"overshoot": {"layout": kept}}, "short"), 400)
        self.assertEqual(tier_estimates.overshoot_mb({"overshoot": {"layout": kept}}, "expansion"), 0)
        self.assertEqual(tier_estimates.overshoot_mb(None, "designer"), 0)
        # Written and read back.
        path = d / "estimates.json"
        path.write_text(tier_estimates.dumps(sidecar))
        self.assertEqual(tier_estimates.load(path)["overshoot"], json.loads(json.dumps(sidecar["overshoot"])))

    def test_the_size_model_lies_above_the_quantile_of_the_measured_peaks(self):
        points = [(x, 1000 + 2 * x + noise) for x, noise in zip(range(100, 1100, 100), (0, 50, -50, 0, 400, 0, 0,
                                                                                         -100, 0, 20))]
        model = tier_estimates.fit(points)
        self.assertAlmostEqual(model["mb_per_pad_layer"], 2.0, delta=0.5)
        below = sum(1 for x, y in points if y <= model["base_mb"] + model["mb_per_pad_layer"] * x + 0.5)
        self.assertEqual(below, 9)
        self.assertIsNone(tier_estimates.fit(points[:2]))
        self.assertEqual(tier_estimates.model_mb({"base_mb": 1000, "mb_per_pad_layer": 0.5}, 100, 4), 1500)

    def test_declared_falls_back_from_the_kind_to_the_layout_the_tier_and_the_size_model(self):
        sidecar = {"margin": 1.25, "model": {"layout": {"base_mb": 2000, "mb_per_pad_layer": 1.0},
                                             "route": {"base_mb": 800, "mb_per_pad_layer": 0.5}},
                   "boards": {"a": {"pads": 100, "layers": 2,
                                    "layout": {"peak_rss_mb": 5000, "seconds": 900.0},
                                    "designer": {"peak_rss_mb": 1200, "seconds": 300.0}},
                              "z": {"pads": 400, "layers": 4}}}
        declared = tier_estimates.declared
        self.assertEqual(declared(sidecar, "a", "designer"), (1200, 300.0, "designer rows"))
        self.assertEqual(declared(sidecar, "a", "short", {"peak_rss_mb": 900, "seconds": 5.0}),
                         (5000, 900.0, "layout rows"))
        self.assertEqual(declared(sidecar, "a", "expansion", {"peak_rss_mb": 900, "seconds": 5.0}),
                         (900, 5.0, "the tier's entry"))
        self.assertEqual(declared(sidecar, "z", "layout"), (math.ceil(1.25 * (2000 + 1600)), None, "size model"))
        self.assertEqual(declared(sidecar, "z", "designer", {"seconds": 7.0}),
                         (math.ceil(1.25 * (800 + 800)), 7.0, "size model"))
        self.assertEqual(declared(sidecar, "new", "layout", size=(10, 2)), (math.ceil(1.25 * 2020), None, "size model"))
        self.assertEqual(declared(sidecar, "new", "layout"), (None, None, None))
        self.assertEqual(declared(None, "a", "layout", {"peak_rss_mb": 700}), (700, None, "the tier's entry"))

    def test_board_size_counts_pads_and_copper_layers(self):
        path = self.directory / "x.kicad_pcb"
        path.write_text(BOARD)
        self.assertEqual(tier_estimates.board_size(path), (3, 3))
        self.assertIsNone(tier_estimates.board_size(self.directory / "absent.kicad_pcb"))

    def test_the_committed_sidecar_estimates_every_board_of_the_tiers(self):
        sidecar = tier_estimates.load(tier_estimates.DEFAULT_ESTIMATES)
        here = Path(tier_estimates.__file__).resolve().parent
        for tier, kind in (("designer", "designer"), ("expansion", "expansion"), ("short", "short"),
                           ("schematic", "schematic"), ("quick", "layout")):
            for name in json.loads((here / f"{tier}.json").read_text())["boards"]:
                megabytes, _, where = tier_estimates.declared(sidecar, name, kind)
                self.assertEqual(where, f"{kind} rows", (tier, name))
                self.assertGreater(megabytes, 0)
        for task in json.loads((here / "tasks/v7/tasks.json").read_text())["tasks"]:
            self.assertEqual(tier_estimates.declared(sidecar, task["name"], "layout")[2], "layout rows")


if __name__ == "__main__":
    unittest.main()
