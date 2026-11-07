#!/usr/bin/env python3
# Copyright (C) 2026 Toit contributors.
# Use of this source code is governed by an MIT-style license that can be
# found in the LICENSE file.
"""Tests of stages.py's measures (no KiCad, no binary)."""

import math
import random
import sys
import tempfile
import unittest
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parent))

import stages  # noqa: E402


def moved(points, angle, shift):
    c, s = math.cos(angle), math.sin(angle)
    return [[c * x - s * y + shift[0], s * x + c * y + shift[1]] for x, y in points]


class Measures(unittest.TestCase):
    def setUp(self):
        generator = random.Random(5)
        self.designer = [[generator.uniform(0, 50), generator.uniform(0, 30)] for _ in range(30)]

    def test_a_rigid_copy_is_the_designers_arrangement(self):
        ours = moved(self.designer, 0.7, [12.0, -4.0])
        result = stages.arrangement(ours, self.designer)
        self.assertAlmostEqual(result["displacement_mm"], 0.0, places=9)
        self.assertGreater(result["raw_mm"], 5.0)
        self.assertAlmostEqual(result["pairwise"], 1.0, places=9)
        self.assertAlmostEqual(result["neighbours"], 1.0)

    def test_the_fit_does_not_mirror(self):
        mirrored = [[-x, y] for x, y in self.designer]
        self.assertGreater(stages.displacement(mirrored, self.designer), 1.0)
        # Its distances are the designer's: only the displacement sees it.
        self.assertAlmostEqual(stages.pairwise(mirrored, self.designer), 1.0, places=9)

    def test_a_shuffle_loses_the_arrangement(self):
        generator = random.Random(9)
        ours = self.designer[:]
        generator.shuffle(ours)
        result = stages.arrangement(ours, self.designer)
        self.assertGreater(result["displacement_share"], 0.8)
        self.assertLess(abs(result["pairwise"]), 0.3)
        self.assertLess(result["neighbours"], 0.4)

    def test_one_part_moved_moves_the_mean(self):
        ours = [p[:] for p in self.designer]
        ours[0][0] += 30.0
        aligned = stages.displacement(ours, self.designer)
        raw = stages.displacement(ours, self.designer, align=False)
        self.assertAlmostEqual(raw, 1.0)
        # The best alignment is no worse than none, and the least squares
        # alone would spread the one part's error over all (1.98).
        self.assertLess(aligned, raw + 1e-9)
        self.assertLess(stages.mean_distance(stages.weighted_fit(ours, self.designer, [1.0] * 30),
                                             self.designer), 2.0)
        self.assertGreater(stages.mean_distance(stages.weighted_fit(ours, self.designer, [1.0] * 30),
                                                self.designer), 1.5)

    def test_spread_and_degenerate_inputs(self):
        self.assertAlmostEqual(stages.spread([[0, 0], [2, 0]]), 1.0)
        self.assertIsNone(stages.displacement([], []))
        self.assertIsNone(stages.neighbours([[0, 0], [1, 1]], [[0, 0], [1, 1]]))


class Traces(unittest.TestCase):
    def objective(self, scale, overlap=0.0):
        return {"wirelength": [scale, 2 * scale, 3 * scale], "half_perimeter": [scale, 2 * scale, 6 * scale],
                "overlap": overlap, "overlapping": 0, "relations": 0.0, "hard": 0.0,
                "crossings_anneal": 10, "crossings_all": 12, "crossing_weight": 3.0}

    def trace_seed(self):
        designer = [[0.0, 0.0], [10.0, 0.0], [0.0, 10.0], [10.0, 10.0], [5.0, 5.0]]
        parts = []
        for index, center in enumerate(designer):
            reference = "U1" if index < 2 else f"Q{index}"
            parts.append({"reference": reference, "designer_center": center,
                          "centers": [[c + 1.0 for c in center], center],
                          "poses": [[c + 1.0 for c in center] + [0.0], center + [0.0]]})
        return {"seed": 3, "kept": True, "stages": ["global", "final"], "parts": parts,
                "objective": {"global": self.objective(2.0), "final": self.objective(1.0, 0.5),
                              "designer": self.objective(1.0)}}

    def test_a_seed_measured_by_stage_with_repeated_references(self):
        seed = self.trace_seed()
        self.assertEqual(stages.keyed(seed["parts"])[:3], ["U1#0", "U1#1", "Q2#0"])
        result = stages.measure_seed(seed, {"U1#0", "U1#1", "Q2#0", "Q3#0", "Q4#0"})
        final = result["stages"]["final"]
        self.assertAlmostEqual(final["displacement_mm"], 0.0)
        self.assertEqual(final["hpwl"], [1.0, 1.0, 1.0])
        self.assertAlmostEqual(result["stages"]["global"]["hpwl_all"], 2.0)
        self.assertAlmostEqual(result["stages"]["global"]["raw_mm"], math.sqrt(2.0))
        # 6 + 30 of crossings, the overlap at the anneal's last factor.
        self.assertAlmostEqual(final["objective"], 6.0 + 400 * 0.5 + 30.0)
        self.assertAlmostEqual(result["designer_objective"], 36.0)
        # The active set chooses the parts.
        self.assertEqual(stages.measure_seed(seed, {"U1#1", "Q3#0"})["stages"]["final"]["parts"], 2)

    def test_the_race_seed_is_found_by_its_poses(self):
        seed = self.trace_seed()
        other = dict(seed, seed=4, parts=[dict(p, poses=[p["poses"][0], [99.0, 99.0, 0.0]]) for p in seed["parts"]])
        footprints = "".join(f'(footprint "X" (layer "F.Cu") (at {x} {y} 90) (pad "1"))\n'
                             for x, y in [[0, 0], [10, 0], [0, 10], [10, 10], [5, 5], [40, 1]])
        with tempfile.TemporaryDirectory() as directory:
            path = Path(directory) / "race.kicad_pcb"
            path.write_text(f"(kicad_pcb\n{footprints})\n")
            self.assertEqual(stages.footprint_ats(path)[0], (0.0, 0.0, 90.0))
            self.assertEqual(stages.race_seed({"seeds": [other, seed]}, path), 3)
            self.assertIsNone(stages.race_seed({"seeds": [other]}, path))

    def skeleton_seed(self):
        """A seed placed by skeleton: U1 (4 x 2 mm at the origin) held,
        C1 inside it until the anneal, C2 outside throughout, C3 on the
        other side under it."""
        stages_ = ["skeleton_start", "skeleton_final", "start", "global", "annealed", "legal", "final"]

        def part(reference, centers, side="Front", body=(1.0, 0.5)):
            return {"reference": reference, "body": list(body), "side": side, "angles": [0.0] * len(stages_),
                    "centers": centers, "poses": [c + [0.0] for c in centers]}
        held = [[0.0, 0.0]] * len(stages_)
        return {"seed": 1, "kept": True, "stages": stages_, "relaxation": {
                    "halo_scale": 0.0, "small_halo_scale": 0.0, "spacing": 0.16, "tight": True,
                    "courtyard_spacing": False},
                "objective": {stage: self.objective(1.0) for stage in stages_},
                "global_history": {"global": [[0, 0.9, 1.0, 1, 1, 1, 1], [135, 0.65, 54.0, 1, 1, 1, 1]]},
                "parts": [part("U1", held, body=(4.0, 2.0)),
                          part("C1", [[9, 9], [9, 9], [0.5, 0.2], [0.5, 0.2], [0.5, 1.6], [0.5, 1.6], [0.5, 1.6]]),
                          part("C2", [[9, 9], [9, 9], [5.0, 0.0], [5.0, 0.0], [5.0, 0.0], [6.0, 0.0], [6.0, 0.0]]),
                          part("C3", [[9, 9], [9, 9], [0.0, 0.5], [0.1, 0.5], [0.1, 0.5], [0.1, 0.5], [0.1, 0.5]],
                               side="Back")]}

    def test_the_passives_inside_a_held_body_by_stage(self):
        seed = self.skeleton_seed()
        self.assertEqual(stages.held_parts(seed), {0})
        result = stages.measure_passives(seed)
        # C3 lies under U1 on the other side: not inside.
        self.assertEqual(result["inside"], {"start": 1, "global": 1, "annealed": 0, "legal": 0, "final": 0})
        self.assertEqual(result["inside_by_part"], {"U1": 1})
        self.assertEqual((result["held"], result["passives"]), (1, 3))
        self.assertAlmostEqual(result["moved_trapped"], 1.4)
        self.assertAlmostEqual(result["moved_free"], (1.0 + 0.0) / 2)
        self.assertEqual((result["iterations"], result["overflow"], result["lambda_growth"]), (136, 0.65, 54.0))
        self.assertEqual(result["level"], "halo 0/0, spacing 0.16, tight")
        # A body turned a quarter: its box swaps its sides.
        turned = dict(seed["parts"][0], angles=[90.0] * 7)
        half = stages.body_box(turned, 0)[1]
        self.assertAlmostEqual(half[0], 1.0)
        self.assertAlmostEqual(half[1], 2.0)

    def test_a_plain_seed_has_no_passives_measure(self):
        seed = self.skeleton_seed()
        seed["stages"] = seed["stages"][2:]
        for part in seed["parts"]:
            part["centers"] = part["centers"][2:]
        self.assertIsNone(stages.held_parts(seed))
        self.assertIsNone(stages.measure_passives(seed))

    def test_only_the_designers_variants_start_from_the_designers_poses(self):
        with tempfile.TemporaryDirectory() as directory:
            inputs = Path(directory)
            (inputs / "b-s1" / "source").mkdir(parents=True)
            (inputs / "b-s1" / "source" / "b.kicad_pcb").write_text("(kicad_pcb)")
            (inputs / "b.designer").write_text("/designer/b.kicad_pcb\n")
            arguments = type("Arguments", (), {"inputs": str(inputs), "out": str(inputs / "out"),
                                               "threads": 2, "binary": "/bin/true"})()
            for variant, designer in (("ours", False), ("debug", False), ("nostall", False),
                                      ("global-all", True), ("anneal", True), ("legalize-noanneal", True)):
                env = stages.unit_command(arguments, f"b-s1-{variant}")[2]
                self.assertEqual("PCB_PLACER_START" in env, designer, variant)

    def test_unit_names(self):
        self.assertEqual(stages.unit_parts("SNSP-CPU-01-s101-global-active"), ("SNSP-CPU-01", 101, "global-active"))
        with self.assertRaises(SystemExit):
            stages.unit_parts("video")


if __name__ == "__main__":
    unittest.main()
