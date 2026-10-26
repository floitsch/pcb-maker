# Copyright (C) 2026 Toit contributors.
"""Tests of placer_ab.py: python3 -m pytest benchmarks/agent-tasks/test_placer_ab.py"""

import json
import random
import statistics
import sys
import tempfile
import unittest
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parent))
import placer_ab  # noqa: E402


def row(name, unconnected, first=None):
    return {"name": name, "native": {"unconnected": unconnected, "errors": {}},
            "first_unconnected_terminals": first if first is not None else unconnected + 10}


def noise_values(boards=6, seeds=9, seed=3):
    rng = random.Random(seed)
    return {f"b{i}": [max(0.0, rng.gauss(50 + 20 * i, 5 + 2 * i)) for _ in range(seeds)] for i in range(boards)}


class Reading(unittest.TestCase):
    def test_the_seed_comes_from_the_name_or_the_argument(self):
        self.assertEqual(placer_ab.seed_of("build/legal1-dense-s201b-off")[1], 201)
        self.assertEqual(placer_ab.seed_of("build/group1-dense-s201-on-b")[1], 201)
        self.assertEqual(placer_ab.seed_of("build/x@7"), (Path("build/x"), 7))
        with self.assertRaises(SystemExit):
            placer_ab.seed_of("build/nothing")

    def test_a_side_merges_its_directories_and_reads_a_stopped_run_from_its_out_file(self):
        with tempfile.TemporaryDirectory() as tmp:
            tmp = Path(tmp)
            first = tmp / "t-s1-off"
            first.mkdir()
            (first / "results.json").write_text(json.dumps([row("a", 5), row("b", 7)]))
            second = tmp / "t-s1b-off"
            second.mkdir()
            (tmp / "t-s1b-off.out").write_text("run.py: waiting\n" + json.dumps(row("b", 9)) + "\n")
            side = placer_ab.side_of([str(first), str(second)])
            self.assertEqual(sorted(side), [1])
            self.assertEqual(placer_ab.compare.unconnected(side[1]["a"][0]), 5)
            self.assertEqual(placer_ab.compare.unconnected(side[1]["b"][0]), 9)

    def test_pairs_need_both_sides(self):
        off = {1: {"a": (row("a", 5), None), "b": (row("b", 7), None)}, 2: {"a": (row("a", 6), None)}}
        on = {1: {"a": (row("a", 4), None)}, 2: {"a": (row("a", 6), None)}}
        pairs, missing = placer_ab.paired(off, on)
        self.assertEqual(pairs, {"a": {1: (5.0, 4.0), 2: (6.0, 6.0)}})
        self.assertEqual(missing, [("b", 1)])


class Statistics(unittest.TestCase):
    pairs = {"a": {1: (10.0, 8.0), 2: (12.0, 11.0), 3: (11.0, 12.0)},
             "b": {1: (100.0, 90.0), 2: (80.0, 85.0), 3: (90.0, 70.0)}}
    reference = {"a": (10.0, 4.0), "b": (100.0, 1.0)}

    def test_the_statistics_of_paired_data(self):
        s = placer_ab.statistics_of(self.pairs, self.reference)
        self.assertEqual(s["sum_medians"], (101.0, 96.0))
        self.assertEqual(s["pair_changes"], [-12.0, 4.0, -19.0])
        self.assertAlmostEqual(s["pair_sums_change"], -9.0)
        self.assertFalse(s["old_rule"])  # seed 2's pair rises
        self.assertEqual(s["sign"][:2], (2, 0))  # a: 11 -> 10.33, b: 90 -> 81.67
        # a's mean change -0.667 / 10, weight 4; b's -8.333 / 100, weight 1.
        self.assertAlmostEqual(s["weighted"], (4 * -0.0667 + 1 * -0.0833) / 5, places=3)

    def test_identical_sides_are_not_resolved(self):
        same = {b: {s: (p[0], p[0]) for s, p in by_seed.items()} for b, by_seed in self.pairs.items()}
        self.assertEqual(placer_ab.statistics_of(same, self.reference)["weighted"], 0.0)
        self.assertEqual(placer_ab.sign_flip(same, self.reference, "weighted", rounds=200), 1.0)

    def test_a_large_effect_on_every_board_is_resolved_and_its_reverse_is_worse(self):
        values = noise_values()
        reference = placer_ab.weights_of(values)
        rng = random.Random(5)
        draw = {b: placer_ab.smoothed(xs) for b, xs in values.items()}
        better = {b: {s: (draw[b](rng), 0.6 * draw[b](rng)) for s in range(5)} for b in values}
        self.assertLess(placer_ab.sign_flip(better, reference, "weighted", rounds=2000), 0.01)
        worse = {b: {s: (on, off) for s, (off, on) in by_seed.items()} for b, by_seed in better.items()}
        self.assertGreater(placer_ab.sign_flip(worse, reference, "weighted", rounds=2000), 0.99)
        lo, hi = placer_ab.bootstrap_interval(better, reference, rounds=500)
        self.assertLess(hi, 0.0)

    def test_the_weight_floors_the_coefficient_of_variation(self):
        reference = placer_ab.weights_of({"flat": [50.0, 50.0, 51.0], "wide": [10.0, 30.0, 20.0]})
        self.assertAlmostEqual(reference["flat"][1], 1 / placer_ab.CV_FLOOR ** 2)
        self.assertAlmostEqual(reference["wide"][1], 4.0)

    def test_the_smoothed_draw_keeps_mean_and_variance(self):
        xs = [10.0, 14.0, 20.0, 22.0, 30.0, 31.0, 40.0, 45.0, 50.0]
        draw = placer_ab.smoothed(xs)
        rng = random.Random(1)
        ys = [draw(rng) for _ in range(40000)]
        self.assertAlmostEqual(statistics.fmean(ys), statistics.fmean(xs), delta=0.3)
        self.assertAlmostEqual(statistics.variance(ys) / statistics.variance(xs), 1.0, delta=0.05)


class Resolution(unittest.TestCase):
    def test_more_seeds_resolve_smaller_effects_at_the_false_positive_rate(self):
        values = noise_values()
        table = placer_ab.resolution(values, [3, 9], alpha=0.1, reps=300, grid=[i / 50 for i in range(1, 26)])
        for k in (3, 9):
            false_positive, smallest = table[k]["weighted"]
            self.assertLessEqual(false_positive, 0.12)
            self.assertIsNotNone(smallest)
        self.assertLess(table[9]["weighted"][1], table[3]["weighted"][1])
        self.assertLessEqual(table[9]["pair_sums"][1], table[3]["pair_sums"][1])

    def test_seeds_needed_falls_with_the_effect(self):
        values = noise_values()
        large = placer_ab.seeds_needed(values, 0.3, alpha=0.1, reps=200)
        small = placer_ab.seeds_needed(values, 0.08, alpha=0.1, reps=200)
        self.assertIsNotNone(large)
        self.assertTrue(small is None or small >= large)
        self.assertIsNone(placer_ab.seeds_needed(values, -0.1))
        self.assertIsNotNone(placer_ab.seeds_needed(values, 0.3, alpha=0.1, reps=200, key="pair_sums_change"))


class Noise(unittest.TestCase):
    def test_the_report_has_a_row_per_board_and_the_pooled_correlation(self):
        noise = {"binaries": ["x"], "seeds": [1, 2, 3],
                 "boards": {"a__b__c": {"seeds": [1, 2, 3], "unconnected": [1, 2, 3], "first_open": [2, 4, 7],
                                        "objective": [None, None, None], "crossings": [5, 5, 5]}}}
        lines = placer_ab.report(noise)
        self.assertTrue(any(line.startswith("| c | 3 |") for line in lines))
        self.assertTrue(any("within boards, pooled | +0.99" in line for line in lines))


if __name__ == "__main__":
    unittest.main()
