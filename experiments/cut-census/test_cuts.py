#!/usr/bin/env python3
# Copyright (C) 2026 Florian Loitsch.
"""Tests of cuts.py on boards made up in plain data (no pcbnew)."""

import sys
import unittest
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parent))

import cuts  # noqa: E402


def rect(x, y, w, h):
    return [[x - w / 2, y - h / 2], [x + w / 2, y - h / 2], [x + w / 2, y + h / 2], [x - w / 2, y + h / 2]]


def pad(number, net, x, y, w, h):
    return {"number": str(number), "net": net, "class": "Default", "npth": False,
            "layers": {"F.Cu": [rect(x, y, w, h)]}}


def board(footprints, outline=40.0):
    return {"rules": {"track": 0.2, "clearance": 0.2, "via": 0.6, "edge": 0.3,
                      "classes": {"Default": [0.2, 0.2]}},
            "outline": [rect(0.0, 0.0, outline, outline)], "zone_nets": [], "copper_layers": 2,
            "footprints": footprints}


def part(pins):
    """A row of `pins` pads 0.3 x 1.0 mm at a 1.0 mm pitch along x at y = 0,
    each on its own net, each net also on a far-away part."""
    pads = [pad(i + 1, f"N{i}", (i - (pins - 1) / 2) * 1.0, 0.0, 0.3, 1.0) for i in range(pins)]
    far = [pad(i + 1, f"N{i}", -15.0 + i * 1.0, 15.0, 0.3, 0.3) for i in range(pins)]
    return [{"reference": "U1", "back": False, "at": [0, 0, 0], "pads": pads},
            {"reference": "J1", "back": False, "at": [0, 15, 0], "pads": far}]


def wall(gap, opening=None, offset=1.2):
    """A closed ring of 0.5 mm pads around U1's row, `gap` between pads
    (copper to copper), `offset` from the row's box (inside the sink's
    ring); with `opening`, one gap of that width in the ring's top side."""
    pads, number = [], 0
    half_x, half_y = 5.0 + offset + 0.25, 0.5 + offset + 0.25
    step = 0.5 + gap
    xs = []
    x = -half_x
    while x <= half_x + 1e-9:
        xs.append(x)
        x += step
    ys = []
    y = -half_y
    while y <= half_y + 1e-9:
        ys.append(y)
        y += step
    spots = {(round(x, 6), round(-half_y, 6)) for x in xs} | {(round(x, 6), round(half_y, 6)) for x in xs}
    spots |= {(round(-half_x, 6), round(y, 6)) for y in ys} | {(round(half_x, 6), round(y, 6)) for y in ys}
    for x, y in sorted(spots):
        if opening is not None and abs(y - half_y) < 1e-6 and abs(x) < step / 2:
            continue
        number += 1
        pads.append(pad(number, None, x, y, 0.5, 0.5))
    if opening is not None:
        # Two pads beside the opening, `opening` apart.
        for side in (-1, 1):
            number += 1
            pads.append(pad(number, None, side * (opening / 2 + 0.25), half_y, 0.5, 0.5))
    return {"reference": "W1", "back": False, "at": [0, 0, 0], "pads": pads}


class EscapeTest(unittest.TestCase):
    def flow(self, footprints, ring=2.0):
        b = cuts.Board(board(footprints))
        return cuts.escape(b, 0, ring)

    def test_an_open_row_escapes_whole(self):
        result = self.flow(part(10))
        self.assertEqual(result["pins"], 10)
        self.assertEqual(result["flow_planar"], 10)

    def test_a_wall_too_tight_for_a_track_lets_nothing_out(self):
        # 0.55 mm holds no track at 0.2 / 0.2; the wall's ring sits inside
        # the sink ring, so only via sites inside could help.
        result = self.flow(part(10) + [wall(0.55)])
        self.assertEqual(result["flow_planar"], 0)
        # With 1.2 mm between the row and the wall a via (0.6 + 2 x 0.2)
        # fits beside every pin; with 0.55 mm none does.
        self.assertEqual(result["flow_vias"], 10)
        result = self.flow(part(10) + [wall(0.55, offset=0.55)])
        self.assertEqual(result["flow_vias"], 0)

    def test_one_track_opening_lets_one_out(self):
        result = self.flow(part(10) + [wall(0.55, opening=0.62)])
        self.assertEqual(result["flow_planar"], 1)

    def test_two_track_opening_lets_two_out(self):
        result = self.flow(part(10) + [wall(0.55, opening=1.02)])
        self.assertEqual(result["flow_planar"], 2)


class GapTest(unittest.TestCase):
    def test_quanta_and_tracks(self):
        b = cuts.Board(board([]))
        self.assertAlmostEqual(b.quantum(1), 0.6)
        self.assertAlmostEqual(b.quantum(2), 1.0)
        self.assertEqual(b.tracks(0.55), 0)
        self.assertEqual(b.tracks(0.60), 1)
        self.assertEqual(b.tracks(0.99), 1)
        self.assertEqual(b.tracks(1.00), 2)

    def test_a_pad_between_hides_the_pair_behind_it(self):
        footprints = [
            {"reference": "A", "back": False, "at": [0, 0, 0], "pads": [pad(1, "a", 0.0, 0.0, 0.5, 0.5)]},
            {"reference": "B", "back": False, "at": [0, 0, 0], "pads": [pad(1, "b", 1.0, 0.0, 0.5, 0.5)]},
            {"reference": "C", "back": False, "at": [0, 0, 0], "pads": [pad(1, "c", 2.0, 0.0, 0.5, 0.5)]},
        ]
        found = cuts.cuts(cuts.Board(board(footprints)))
        pairs = sorted((a, b) for _, a, b, _ in found)
        self.assertEqual(pairs, [(0, 1), (1, 2)])
        self.assertTrue(all(abs(gap - 0.5) < 1e-9 for *_, gap in found))


if __name__ == "__main__":
    unittest.main()
