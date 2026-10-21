# Copyright (C) 2026 Toit contributors.
"""Tests of compare.py's measures: python3 -m pytest benchmarks/agent-tasks/test_compare.py"""

import sys
import unittest
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parent))
import compare  # noqa: E402
import quality  # noqa: E402


def row(name, routed, unconnected, vias, length, **extra):
    return {"name": name, "routed": routed, "native": {"unconnected": unconnected, "errors": {}},
            "vias": vias, "length_mm": length, **extra}


class Normalised(unittest.TestCase):
    numbers = {"a": {"vias": 100, "track_length_mm": 1000.0, "ground_cut_mm": 50.0, "board_sha256": "aa"},
               "b": {"vias": 10, "track_length_mm": 200.0, "ground_cut_mm": None, "board_sha256": "bb"},
               "c": {"vias": 0, "track_length_mm": 100.0, "ground_cut_mm": 0.0}}

    def test_medians_of_the_per_board_ratios(self):
        rows = [row("a", "90/100", 10, 50, 900.0), row("b", "20/20", 0, 30, 100.0), row("c", "5/5", 0, 4, 300.0)]
        n = compare.normalised(rows, self.numbers)
        # vias: a 0.5, b 3.0 (c's designer has none); length: 0.9, 0.5, 3.0.
        self.assertEqual((n["vias_ratio"], n["vias_boards"]), (1.75, 2))
        self.assertEqual((n["length_ratio"], n["length_boards"]), (0.9, 3))
        self.assertEqual((n["open_share"], n["open_boards"]), (0.0, 3))
        self.assertAlmostEqual(n["open_share_mean"], 0.1 / 3, places=4)
        self.assertEqual(n["ground_boards"], 0)
        self.assertIn("vias x1.75", compare.normalised_line(n))

    def test_rows_without_a_result_or_of_another_board_are_left_out(self):
        rows = [row("a", "90/100", 10, 50, 900.0, board_sha256="other"),
                {"name": "b", "error": "timed out", "native": None}]
        n = compare.normalised(rows, self.numbers)
        self.assertEqual((n["boards"], n["vias_boards"], n["open_boards"]), (1, 0, 1))

    def test_ground_pour_cut_from_the_rows_scores(self):
        rows = [row("a", "100/100", 0, 50, 900.0, quality={"ours": {"ground_cut_mm": 20.0}}),
                row("c", "5/5", 0, 4, 300.0, quality={"ours": {"ground_cut_mm": None}})]
        n = compare.normalised(rows, self.numbers)
        self.assertEqual((n["ground_cut_delta_mm"], n["ground_boards"], n["ground_pour_lost"]), (-30.0, 1, 1))


class Gap(unittest.TestCase):
    def test_placement_and_routing_gap_per_board_and_in_all(self):
        layout = {"a": row("a", "90/100", 10, 0, 0), "b": row("b", "20/20", 1, 0, 0),
                  "c": {"name": "c", "error": "incomplete: 3 parts without a legal place", "native": None}}
        designer = {name: row(name, f"{n}/{n}", u, 0, 0, placement="designer")
                    for name, n, u in (("a", 100, 4), ("b", 20, 2), ("c", 30, 0))}
        table = {entry["name"]: entry for entry in compare.gap_rows(layout, designer)}
        self.assertEqual((table["a"]["placement_gap"], table["a"]["routing_gap"]), (6, 4))
        self.assertEqual((table["b"]["placement_gap"], table["b"]["routing_gap"]), (-1, 2))
        self.assertIsNone(table["c"]["placement_gap"])
        totals = compare.gap_totals(list(table.values()))
        self.assertEqual((totals["both"], totals["layout_open"], totals["placement_gap"], totals["routing_gap"]),
                         (2, 11, 5, 6))
        self.assertEqual((totals["placement_worse"], totals["placement_better"]), (1, 1))
        self.assertEqual(totals["layout_without_result"], ["c"])


class StrictRules(unittest.TestCase):
    def test_runs_judged_under_different_rules_are_named(self):
        old = [row("a", "1/1", 0, 0, 0, new_findings_strict={}), row("b", "1/1", 0, 0, 0)]
        new = [row("a", "1/1", 0, 0, 0, new_findings_strict={}, strict_rule=2, strict_mask_openings=3)]
        self.assertEqual(compare.strict_rules(old), [1])
        self.assertIsNone(compare.rules_warning(new, new))
        self.assertIn("1 before, 2 after", compare.rules_warning(old, new))
        self.assertIsNone(compare.rules_warning(old, [row("c", "1/1", 0, 0, 0)]))
        o = compare.objectives(new)
        self.assertEqual(o["strict_mask_openings"], 3)
        self.assertIn("3 mask-opening bridges not counted", compare.objectives_line(o))


class Measures(unittest.TestCase):
    def test_a_score_report_gives_every_measure(self):
        report = {"economy": {"vias": 3, "track_length_mm": 12.5},
                  "planes": [{"cut_by_mm": 1.5}, {"cut_by_mm": 2.0}], "assembly": {"near_edge": ["R1"]}}
        values = quality.measures(report)
        self.assertEqual(set(values), {key for key, *_ in quality.MEASURES})
        self.assertEqual((values["vias"], values["track_length_mm"], values["ground_cut_mm"], values["near_edge"]),
                         (3, 12.5, 3.5, 1))
        self.assertIsNone(quality.measures({"planes": []})["ground_cut_mm"])


if __name__ == "__main__":
    unittest.main()
