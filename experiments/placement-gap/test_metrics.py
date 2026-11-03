#!/usr/bin/env python3
# Copyright (C) 2026 Toit contributors.
# Use of this source code is governed by an MIT-style license that can be
# found in the LICENSE file.
"""Tests of metrics.py on boards built by hand (no KiCad needed).

    python3 -I -m unittest experiments/placement-gap/test_metrics.py
"""

import math
import sys
import unittest
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parent))

import metrics  # noqa: E402


def pad(number, net, x, y, smd=True, size=0.5):
    return {"number": number, "net": net, "x": x, "y": y, "smd": smd,
            "box": [x - size / 2, y - size / 2, x + size / 2, y + size / 2]}


def part(reference, x, y, pads, angle=0.0, footprint="R_0402", back=False, through=False, body=None):
    boxes = [p["box"] for p in pads]
    body = body or [min(b[0] for b in boxes) - 0.2, min(b[1] for b in boxes) - 0.2,
                    max(b[2] for b in boxes) + 0.2, max(b[3] for b in boxes) + 0.2]
    return {"reference": reference, "footprint": footprint, "value": "", "x": x, "y": y, "angle": angle,
            "back": back, "through_hole": through, "courtyard": True, "body": body, "pads": pads}


def resistor(reference, x, y, a, b, angle=0.0):
    return part(reference, x, y, [pad("1", a, x - 0.5, y), pad("2", b, x + 0.5, y)], angle=angle)


def board(footprints, pours=(), outline=None):
    outline = outline or [[[0.0, 0.0], [20.0, 0.0], [20.0, 20.0], [0.0, 20.0]]]
    return {"copper_layers": 2, "outline": outline, "pour_nets": list(pours), "footprints": footprints}


class Geometry(unittest.TestCase):
    def test_proper_crossings_only(self):
        self.assertTrue(metrics.lines_cross((0, 0), (2, 2), (0, 2), (2, 0)))
        # Touching at an end and collinear overlaps do not cross.
        self.assertFalse(metrics.lines_cross((0, 0), (2, 2), (2, 2), (3, 0)))
        self.assertFalse(metrics.lines_cross((0, 0), (2, 0), (1, 0), (3, 0)))

    def test_spanning_tree_from_the_first_pin(self):
        points = [(0, 0), (10, 0), (1, 0), (2, 0)]
        lines = metrics.spanning_tree(points)
        length = sum(math.dist(points[a], points[b]) for a, b in lines)
        self.assertEqual(len(lines), 3)
        self.assertAlmostEqual(length, 10.0)

    def test_octilinear_metric(self):
        self.assertAlmostEqual(metrics.octilinear((0, 0), (3, 4)), 4 + (math.sqrt(2) - 1) * 3)

    def test_box_outline_distance(self):
        outline = [[[0.0, 0.0], [20.0, 0.0], [20.0, 20.0], [0.0, 20.0]]]
        self.assertAlmostEqual(metrics.box_outline_distance([2, 3, 5, 6], outline), 2.0)
        self.assertEqual(metrics.box_outline_distance([-1, 3, 5, 6], outline), 0.0)


class Metrics(unittest.TestCase):
    def test_crossings_and_wirelength(self):
        # Two two-pin nets that cross (an X), and a pour net left out.
        b = board([
            part("A", 1, 1, [pad("1", "N1", 1, 1), pad("2", "N2", 1, 9)]),
            part("B", 9, 9, [pad("1", "N1", 9, 9), pad("2", "N2", 9, 1)]),
            part("C", 5, 15, [pad("1", "GND", 3, 15), pad("2", "GND", 7, 15)]),
        ], pours=["GND"])
        lines = metrics.ratsnest(b)
        self.assertEqual(len(lines), 2)
        self.assertEqual(metrics.crossing_count(lines), 1)
        wire = metrics.wirelength(b)
        self.assertAlmostEqual(wire["hpwl_2_mm"], 32.0)
        self.assertAlmostEqual(wire["hpwl_pour_mm"], 4.0)
        self.assertAlmostEqual(wire["hpwl_mm"], 36.0)

    def test_pin_row_crossings(self):
        # A row of three SMD pads; another net's line crosses between pads
        # 1 and 2, once per pad pair and net.
        row = part("U1", 5, 5, [pad("1", "A", 4, 5), pad("2", "B", 5, 5), pad("3", "C", 6, 5)])
        other = part("R1", 4.5, 2, [pad("1", "X", 4.5, 2), pad("2", "Y", 8, 2)])
        far = part("R2", 4.5, 8, [pad("1", "X", 4.5, 8), pad("2", "Y", 8, 12)])
        b = board([row, other, far])
        lines = metrics.ratsnest(b)
        self.assertEqual(metrics.pin_row_crossings(b, lines), 1)

    def test_orientation_and_rows_of_moved_parts(self):
        parts = [resistor("R1", 2, 5, "a", "b"), resistor("R2", 4, 5, "c", "d", angle=180.0),
                 resistor("R3", 6, 5.3, "e", "f", angle=90.0)]
        # Two-pin parts half a turn apart are alike: two of three agree.
        self.assertAlmostEqual(metrics.orientation_agreement(parts), 2 / 3)
        # R1 and R2 share a y; R3 shares nothing.
        self.assertAlmostEqual(metrics.row_alignment(parts), 2 / 3)

    def test_moved_references(self):
        designer = board([resistor("R1", 2, 5, "a", "b"), resistor("R2", 4, 5, "c", "d")])
        ours = board([resistor("R1", 2, 5, "a", "b"), resistor("R2", 4, 6, "c", "d")])
        self.assertEqual(metrics.moved_references(designer, ours), {"R2"})

    def test_decoupling_distance(self):
        ic = part("U1", 10, 10, [pad(str(i), "+3V3" if i == 1 else f"S{i}", 9 + 0.5 * i, 10) for i in range(1, 9)],
                  footprint="SOIC-8")
        near = part("C1", 10, 12, [pad("1", "+3V3", 9.5, 12), pad("2", "GND", 10.5, 12)], footprint="C_0402")
        far = part("C2", 10, 18, [pad("1", "+3V3", 9.5, 18), pad("2", "GND", 10.5, 18)], footprint="C_0402")
        result = metrics.decoupling(board([ic, near, far]))
        self.assertAlmostEqual(result["decoupling_within_3mm"], 0.5)
        self.assertAlmostEqual(result["decoupling_mm"], (2.0 + 8.0) / 2)

    def test_gaps_see_the_far_side_only_through_holes(self):
        front = resistor("R1", 5, 5, "a", "b")
        back = part("R2", 5, 5, [pad("1", "c", 4.5, 5), pad("2", "d", 5.5, 5)], back=True)
        tht = part("J1", 5, 7, [pad("1", "e", 5, 7, smd=False)], back=True, through=True, body=[3, 6.5, 7, 9])
        b = board([front, back, tht])
        result = metrics.gaps(b, {"R1"})
        # R2 lies on the back under R1: no gap to it. J1's body is on the
        # back, its pad (6.75 to 7.25) blocks the front.
        self.assertAlmostEqual(result["gap_median_mm"], 6.75 - 5.45)

    def test_a_frame_part_blocks_with_its_pads(self):
        frame = part("U9", 10, 10, [pad("1", "x", 0.5, 0.5, smd=False), pad("2", "y", 19.5, 19.5, smd=False)],
                     through=True)
        r = resistor("R1", 10, 10, "a", "b")
        b = board([frame, r])
        self.assertTrue(metrics.is_frame(frame, b))
        self.assertGreater(metrics.gaps(b, {"R1"})["gap_median_mm"], 5.0)

    def test_density_takes_the_fuller_side(self):
        # A 4 x 4 body on the front over a 4 x 4 body on the back: the
        # fuller side counts, not their sum.
        front = part("U1", 2, 2, [pad("1", "a", 1, 1), pad("2", "b", 3, 3)], body=[0, 0, 4, 4])
        back = part("U2", 2, 2, [pad("1", "c", 1, 1), pad("2", "d", 3, 3)], body=[0, 0, 4, 4], back=True)
        b = board([front, back], outline=[[[0.0, 0.0], [10.0, 0.0], [10.0, 10.0], [0.0, 10.0]]])
        result = metrics.density(b, cells=2)
        self.assertAlmostEqual(result["density_max"], 16 / 25)
        self.assertAlmostEqual(result["density_p90"], 0.7 * 16 / 25)


class Summary(unittest.TestCase):
    def test_spearman(self):
        self.assertAlmostEqual(metrics.spearman([1, 2, 3, 4], [10, 20, 30, 40]), 1.0)
        self.assertAlmostEqual(metrics.spearman([1, 2, 3, 4], [4, 3, 2, 1]), -1.0)
        self.assertIsNone(metrics.spearman([1, 1, 1], [1, 2, 3]))

    def test_summary_counts_the_sign_and_the_paired_differences(self):
        rows = []
        for i, (d, o, fd, fo) in enumerate([(1.0, 2.0, 10, 30), (2.0, 3.0, 5, 9), (3.0, 2.0, 20, 12)]):
            rows.append({"board": f"b{i}", "connections": 100,
                         "designer": {"crossings": d, "first_open": fd},
                         "ours": {"crossings": o, "first_open": fo}})
        summary = metrics.summarise(rows)["crossings"]
        self.assertEqual((summary["ours_above"], summary["ours_below"]), (2, 1))
        self.assertEqual(summary["placements"], 6)
        self.assertAlmostEqual(summary["spearman_differences"], 1.0)


if __name__ == "__main__":
    unittest.main()
