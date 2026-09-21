# Copyright (C) 2026 Toit contributors.
import unittest

import via_space as space
from inspect_board import probe_via


def board():
    return {"schema_version": 1, "id": "unit", "bounds": [-5, -5, 20, 20],
        "rules": {"clearance": .25, "trace_width": .4, "via_diameter": .8, "via_drill": .4},
        "obstacles": [{"id": "O", "rect": [0, 0, 1, 1], "layers": ["top"]}],
        "pads": [{"id": "P", "net": "B", "at": [10, 10], "diameter": 1, "layers": ["bottom"]}],
        "routes": [{"id": "R", "net": "B", "points": [[5, 0], [7, 0], [7, 4], [9, 4]],
                    "layers": ["top", "bottom", "top"]},
                   {"id": "A", "net": "A", "points": [[-2, 5], [-2, 9]], "layers": ["top"]}]}


class ViaSpaceTests(unittest.TestCase):
    def test_rounded_corner_is_not_square_inflation_in_geometry_or_pixels(self):
        shape = space.blockers(board(), "A")[0]
        self.assertEqual(shape["id"], "O")
        self.assertGreater(space.shape_margin(shape, [-.5, -.5]), 0)
        self.assertLess(space.shape_margin(shape, [-.4, -.4]), 0)
        masks = space.make_masks([shape], [-1, -1, 2, 2], (601, 601))
        self.assertLess(masks["top"].getpixel((100, 100)), 5)
        self.assertGreater(masks["top"].getpixel((120, 120)), 250)
        self.assertEqual(masks["bottom"].getbbox(), None)

    def test_continuous_blockers_match_via_probe_for_all_primitive_types(self):
        source = board()
        shapes = space.blockers(source, "A")
        self.assertEqual({s["kind"] for s in shapes}, {"obstacle", "pad", "trace", "via"})
        for at in [[-.5, -.5], [-.4, -.4], [0, 0], [4.3, 0], [4.3, .7],
                   [7.9, 0], [7.9, 2], [7, 4.9], [10, 10], [11.2, 10], [-2, 7]]:
            model = {s["id"] for s in shapes if space.shape_margin(s, at) < -1e-9}
            reference = {item["object_id"] for item in probe_via(source, at, "A")["blockers"]}
            self.assertEqual(model, reference, at)
        self.assertFalse(any(s["id"].startswith("A:") for s in shapes))

    def test_transactional_geometry_moves_foreign_via_exclusion(self):
        from generate import apply_proposal
        source = board()
        updated = apply_proposal(source, {"replacements": [{"route_id": "R",
            "points": [[5, 0], [6, 0], [6, 4], [9, 4]], "layers": ["top", "bottom", "top"]}]})
        old = next(s for s in space.blockers(source, "A") if s["id"] == "R:v1")
        new = next(s for s in space.blockers(updated, "A") if s["id"] == "R:v1")
        self.assertEqual(old["at"], [7, 0])
        self.assertEqual(new["at"], [6, 0])


if __name__ == "__main__":
    unittest.main()
