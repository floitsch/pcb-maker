# Copyright (C) 2026 Toit contributors.
"""Tests of the schematic tier's board preparation (schematic.py):
`python3 -m unittest benchmarks/agent-tasks/test_schematic.py`."""
import sys
import unittest
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parent))
import schematic  # noqa: E402

# A KiCad 7 designer's board: R1 on GND and Net-(R1-Pad2), an outline
# segment with its tstamp, a ground pour, a keepout and a pour of a net no
# matched pad shows.
DESIGNER = """(kicad_pcb (version 20221018) (generator pcbnew)
  (footprint "R:R_0603" (layer "F.Cu") (tstamp 11111111-0000-4000-8000-000000000001) (at 10 10)
    (path "/aaaa")
    (fp_text reference "R1" (at 0 -1) (layer "F.SilkS"))
    (pad "1" smd rect (at -0.8 0) (size 0.8 0.8) (layers "F.Cu") (net 1 "GND") (tstamp 11111111-0000-4000-8000-000000000002))
    (pad "2" smd rect (at 0.8 0) (size 0.8 0.8) (layers "F.Cu") (net 2 "Net-(R1-Pad2)") (tstamp 11111111-0000-4000-8000-000000000003)))
  (gr_line (start 0 0) (end 20 0) (layer "Edge.Cuts") (width 0.1) (tstamp 33333333-0000-4000-8000-000000000001))
  (gr_line (start 20 0) (end 20 20) (layer "Edge.Cuts") (stroke (width 0.1) (type solid)))
  (zone (net 2) (net_name "Net-(R1-Pad2)") (layer "B.Cu") (tstamp 44444444-0000-4000-8000-000000000001) (hatch edge 0.5)
    (connect_pads (clearance 0.5)) (min_thickness 0.25)
    (fill yes (thermal_gap 0.5) (thermal_bridge_width 0.5))
    (polygon (pts (xy 0 0) (xy 20 0) (xy 20 20) (xy 0 20))))
  (zone (net 0) (net_name "") (layer "F.Cu") (tstamp 44444444-0000-4000-8000-000000000002) (hatch edge 0.5)
    (connect_pads (clearance 0)) (min_thickness 0.25)
    (keepout (tracks not_allowed) (vias not_allowed) (pads allowed) (copperpour not_allowed) (footprints allowed))
    (fill (thermal_gap 0.5) (thermal_bridge_width 0.5))
    (polygon (pts (xy 1 1) (xy 2 1) (xy 2 2))))
  (zone (net 3) (net_name "+5V") (layer "F.Cu") (tstamp 44444444-0000-4000-8000-000000000003) (hatch edge 0.5)
    (polygon (pts (xy 0 0) (xy 20 0) (xy 20 20)))))
"""

# The imported board (KiCad 10): R1 by its symbol's path, its pad 2 on
# KiCad 10's name of the unnamed net.
IMPORTED = """(kicad_pcb (version 20260206) (generator "pcb-maker")
  (footprint "R:R_0603" (layer "F.Cu") (uuid "55555555-0000-4000-8000-000000000001") (at 0 0)
    (path "/aaaa")
    (property "Reference" "R1" (at 0 -1 0) (layer "F.SilkS"))
    (pad "1" smd rect (at -0.8 0) (size 0.8 0.8) (layers "F.Cu") (net "GND"))
    (pad "2" smd rect (at 0.8 0) (size 0.8 0.8) (layers "F.Cu") (net "Net-(R1-2)"))))
"""


class EdgeCuts(unittest.TestCase):
    def test_shapes_keep_the_designers_ids(self):
        shapes = schematic.edge_cuts(DESIGNER)
        self.assertEqual(len(shapes), 2)
        # The project's DRC exclusions name the edge by its id.
        self.assertIn('(uuid "33333333-0000-4000-8000-000000000001")', shapes[0])
        self.assertIn("(width 0.1)", shapes[0])
        # A shape without an id gets one of its own.
        self.assertIn('(uuid "5c4e0000-0000-4000-8000-000000000001")', shapes[1])


class Zones(unittest.TestCase):
    def test_pours_and_rule_areas_in_kicad_10_syntax_on_the_imported_nets(self):
        zones = schematic.zones(DESIGNER, DESIGNER, IMPORTED)
        self.assertEqual(len(zones), 3)
        pour, keepout, other = zones
        # The pour's net by the matched pads, KiCad 10's `(net "name")`.
        self.assertTrue(pour.startswith('\t(zone (net "Net-(R1-2)") (layer "B.Cu")'), pour)
        self.assertNotIn("net_name", pour)
        self.assertIn('(uuid "44444444-0000-4000-8000-000000000001")', pour)
        self.assertNotIn("tstamp", pour)
        # A rule area has no net.
        self.assertNotIn("(net", keepout)
        self.assertIn("(keepout (tracks not_allowed)", keepout)
        # A net no matched pad shows keeps its name.
        self.assertTrue(other.startswith('\t(zone (net "+5V")'), other)

    def test_kicad_10_zones_keep_their_net(self):
        designer = DESIGNER.replace('(net 2) (net_name "Net-(R1-Pad2)")', '(net "Net-(R1-Pad2)")')
        zones = schematic.zones(designer, DESIGNER, IMPORTED)
        self.assertTrue(zones[0].startswith('\t(zone (net "Net-(R1-2)") (layer "B.Cu")'), zones[0])


if __name__ == "__main__":
    unittest.main()
