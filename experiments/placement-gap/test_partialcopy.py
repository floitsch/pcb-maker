#!/usr/bin/env python3
# Copyright (C) 2026 Toit contributors.
# Use of this source code is governed by an MIT-style license that can be
# found in the LICENSE file.
"""Tests of partialcopy.py on boards built by hand (no KiCad needed).

    python3 -I -m unittest experiments/placement-gap/test_partialcopy.py
"""

import json
import sys
import unittest
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parent))

import partialcopy  # noqa: E402


def part(reference, x, y, nets, footprint="Resistor_SMD:R_0402", angle=0.0, back=False, locked=False,
         uuid=None):
    return {"reference": reference, "uuid": uuid or f"uuid-{reference}-{x}", "footprint": footprint,
            "value": "", "x": x, "y": y, "angle": angle, "back": back, "locked": locked,
            "pads": [{"number": str(i + 1), "net": net} for i, net in enumerate(nets)]}


def ic(reference, x, y, count, nets=(), footprint="Package_QFP:LQFP-48"):
    nets = list(nets) + [f"{reference}-n{i}" for i in range(count - len(nets))]
    return part(reference, x, y, nets, footprint=footprint)


def bus_board():
    """Four ICs of 48, 32, 20 and 16 pins and a small one of 8 on a 9-pin
    bus, a connector, a mounting hole on GND, a netless hole, a locked
    part, passives."""
    bus = ["BUS"]
    footprints = [
        ic("U1", 10, 10, 48), ic("U2", 20, 10, 32), ic("U3", 30, 10, 20), ic("U4", 40, 10, 16),
        ic("U5", 50, 10, 8, nets=bus),
        part("J1", 0, 0, ["BUS", "GND"], footprint="Connector_PinHeader_2.54mm:PinHeader_1x02"),
        part("H1", 5, 5, ["GND"], footprint="MountingHole:MountingHole_3.2mm_M3_Pad"),
        part("H2", 6, 6, [None], footprint="MountingHole:MountingHole_3.2mm_M3"),
        part("Q1", 7, 7, ["BUS", "A", "B"], footprint="Package_TO_SOT_SMD:SOT-23", locked=True),
        part("D1", 8, 8, ["BUS", "A"], footprint="LED_SMD:LED_0603"),
    ]
    footprints += [part(f"R{i}", 60 + i, 20, ["BUS", f"r{i}"]) for i in range(1, 7)]
    footprints += [part("C1", 70, 20, ["BUS", "GND"], footprint="Capacitor_SMD:C_0402"),
                   part("FB1", 71, 20, ["A", "GND"], footprint="Inductor_SMD:L_0603")]
    return {"pour_nets": ["GND"], "footprints": footprints}


def refs(footprints):
    return sorted(f["reference"] for f in footprints)


class Classes(unittest.TestCase):
    def test_kinds(self):
        board = {f["reference"]: f for f in bus_board()["footprints"]}
        self.assertTrue(partialcopy.is_mechanical(board["H1"]))
        self.assertTrue(partialcopy.is_passive(board["R1"]))
        self.assertTrue(partialcopy.is_passive(board["FB1"]))
        self.assertFalse(partialcopy.is_passive(board["D1"]))
        self.assertTrue(partialcopy.is_ic(board["U5"]))
        self.assertFalse(partialcopy.is_ic(board["J1"]))
        self.assertFalse(partialcopy.is_ic(board["D1"]))

    def test_large_nets_leave_out_the_pours(self):
        self.assertEqual(partialcopy.large_nets(bus_board()), {"BUS"})

    def test_globs(self):
        self.assertTrue(partialcopy.glob_matches("R?", "R1"))
        self.assertTrue(partialcopy.glob_matches("SW_K_*", "SW_K_12"))
        self.assertFalse(partialcopy.glob_matches("R?", "R12"))
        self.assertTrue(partialcopy.glob_matches("REF\\*\\*", "REF**"))
        self.assertFalse(partialcopy.glob_matches("REF\\*\\*", "REF12"))


class Subsets(unittest.TestCase):
    def test_movable_leaves_out_locked_netless_and_fixed(self):
        board = bus_board()
        parts = partialcopy.movable(partialcopy.numbered(board["footprints"]), ["R1", "U4"])
        names = refs(parts)
        for absent in ("H2", "Q1", "R1", "U4"):
            self.assertNotIn(absent, names)
        self.assertIn("H1", names)

    def test_a_netless_namesake_holds_its_reference(self):
        board = {"pour_nets": [], "footprints": [part("M1", 0, 0, [None]), part("M1", 5, 0, ["GND"])]}
        self.assertEqual(partialcopy.movable(partialcopy.numbered(board["footprints"]), []), [])

    def test_the_rules_nest(self):
        chosen = partialcopy.subsets(bus_board(), ["U4"])
        self.assertEqual(refs(chosen["connectors"]), ["H1", "J1"])
        # The three largest movable ICs (U4 is fixed by the task).
        self.assertEqual(refs(chosen["anchors"]), ["H1", "J1", "U1", "U2", "U3"])
        # U5 has a pin on the 9-pin bus; GND is a pour and does not count.
        self.assertEqual(refs(chosen["bus_ics"]), ["H1", "J1", "U1", "U2", "U3", "U5"])
        self.assertEqual(refs(chosen["nonpassive"]), ["D1", "H1", "J1", "U1", "U2", "U3", "U5"])
        self.assertEqual(len(chosen["designer"]), 15)
        self.assertNotIn("displaced", chosen)
        for small, large in (("connectors", "anchors"), ("anchors", "bus_ics"), ("bus_ics", "nonpassive"),
                             ("nonpassive", "designer")):
            self.assertLessEqual(set(refs(chosen[small])), set(refs(chosen[large])))

    def test_displaced_takes_the_farthest_tenth(self):
        board = bus_board()
        ours = json.loads(json.dumps(board))
        moves = {"R3": 9.0, "C1": 30.0, "U2": 1.0}
        for footprint in ours["footprints"]:
            footprint["x"] += moves.get(footprint["reference"], 0.0)
        chosen = partialcopy.subsets(board, [], ours)
        # 16 movable parts: k = 2.
        self.assertEqual(len(chosen["designer"]), 16)
        self.assertEqual(refs(chosen["displaced"]), ["C1", "R3"])

    def test_duplicate_references_pair_by_order(self):
        designer = {"pour_nets": [], "footprints": [part("C1", 0, 0, ["A"]), part("C1", 10, 0, ["A"])]}
        ours = {"pour_nets": [], "footprints": [part("C1", 0, 0, ["A"]), part("C1", 13, 4, ["A"])]}
        self.assertEqual(partialcopy.displacement(designer, ours), {("C1", 0): 0.0, ("C1", 1): 5.0})


class Constraints(unittest.TestCase):
    def test_place_entries_are_the_designers_poses(self):
        board = bus_board()
        footprints = [f for f in partialcopy.numbered(board["footprints"]) if f["reference"] in ("U1", "J1")]
        footprints[0]["angle"] = 90.0
        entries = partialcopy.place_entries(footprints, board)
        self.assertEqual(entries[0], {"part": "U1", "x": 10, "y": 10, "angle": 90.0, "origin": "absolute"})
        self.assertEqual(entries[1]["part"], "J1")
        self.assertNotIn("uuid", entries[1])

    def test_a_shared_reference_gets_its_uuid(self):
        board = {"pour_nets": [], "footprints": [part("C1", 0, 0, ["A"], uuid="a"),
                                                 part("C1", 10, 0, ["A"], uuid="b")]}
        entries = partialcopy.place_entries(partialcopy.numbered(board["footprints"])[1:], board)
        self.assertEqual(entries[0]["uuid"], "b")

    def test_the_tasks_constraints_are_kept(self):
        task = {"version": 1, "move_all": True, "fixed": ["U4"], "place": [{"part": "SW1", "at": "front"}]}
        result = partialcopy.with_places(task, [{"part": "U1", "x": 1, "y": 2, "angle": 0, "origin": "absolute"}])
        self.assertEqual(result["fixed"], ["U4"])
        self.assertEqual([e["part"] for e in result["place"]], ["SW1", "U1"])
        self.assertEqual(len(task["place"]), 1)


class Recovery(unittest.TestCase):
    def test_the_share(self):
        self.assertEqual(partialcopy.recovery(100, 20, 100), 0.0)
        self.assertEqual(partialcopy.recovery(100, 20, 20), 1.0)
        self.assertEqual(partialcopy.recovery(100, 20, 60), 0.5)
        self.assertIsNone(partialcopy.recovery(20, 20, 30))
        self.assertIsNone(partialcopy.recovery(100, None, 30))

    def test_unit_names_put_the_ends_first(self):
        names = partialcopy.unit_names("a__b")
        self.assertEqual(names[:4], ["ours-s1", "ours-s101", "designer-s1", "designer-s101"])
        self.assertEqual(names[4:], [f"{rule}-s1" for rule in partialcopy.RULES])


if __name__ == "__main__":
    unittest.main()
