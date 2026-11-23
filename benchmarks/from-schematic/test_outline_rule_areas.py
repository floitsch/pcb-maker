# Copyright (C) 2026 Florian Loitsch.

import unittest

import comparison_scoring  # Locate the benchmark's shared modules.
import outline_rule_areas as rules


def pcb(*items):
    return "(kicad_pcb " + " ".join(items) + ")"


def keepout(name, uid, *, layer="B.Cu", pads="not_allowed", polygons=None):
    if polygons is None:
        polygons = "(polygon (pts (xy 1 2) (xy 4 2) (xy 4 5)))"
    return (f'(zone (name "{name}") (layer "{layer}") (uuid "{uid}")'
            f' (keepout (tracks not_allowed) (vias not_allowed) (pads {pads})'
            f' (copperpour not_allowed) (footprints allowed)) {polygons})')


class OutlineRuleAreas(unittest.TestCase):
    def test_only_top_level_keepouts_are_transferred(self):
        area = keepout("Carrier", "protected")
        normal_zone = "(zone (filled_polygon (pts)) (filled_polygon (pts)))"
        placed = f'(footprint "Test:A" (at 2 2) {keepout("Inner", "nested")})'
        source = pcb(area, normal_zone, placed, "(segment (start 1 1) (end 2 2))")
        forms, signatures = rules.collect(source, 2)
        self.assertEqual(forms, [area])
        self.assertEqual(signatures.total(), 1)
        self.assertEqual(rules.record_source(source, 2)["count"], 1)

    def test_named_areas_with_the_same_polygon_keep_distinct_identities(self):
        a = keepout("Sensitive", "a")
        b = keepout("Mechanical", "b")
        shapes = (a, b)
        self.assertTrue(rules.check(shapes, pcb(a, b), 2)["satisfied"])
        result = rules.check(shapes, pcb(a), 2)
        self.assertFalse(result["satisfied"])
        self.assertEqual(result["missing_or_changed"], 1)
        wrong = keepout("Sensitive", "c", pads="allowed")
        result = rules.check(shapes, pcb(a, b, wrong), 2)
        self.assertFalse(result["satisfied"])
        self.assertEqual(result["unexpected_named"], 1)

    def test_named_region_can_be_used_by_project_custom_rules(self):
        area = keepout("Custom-rule Region", "a").replace("not_allowed", "allowed")
        anonymous = keepout("", "b").replace("not_allowed", "allowed")
        forms, _ = rules.collect(pcb(area, anonymous), 2)
        self.assertEqual(forms, [area])
        self.assertFalse(rules.check([area], pcb(anonymous), 2)["satisfied"])

    def test_saved_rule_matches_native_polygon_order_and_dimensions(self):
        area = keepout("", "x")
        reordered = keepout("", "y", polygons="(polygon (pts (xy 4.0 5) (xy 4 2) (xy 1 2)))")
        self.assertTrue(rules.check([area], pcb(reordered), 2)["satisfied"])
        changed = keepout("", "y", polygons="(polygon (pts (xy 4.000001 5) (xy 4 2) (xy 1 2)))")
        self.assertFalse(rules.check([area], pcb(changed), 2)["satisfied"])
        self.assertFalse(rules.check([area], pcb(keepout("", "y", layer="F.Cu")), 2)["satisfied"])
        with self.assertRaisesRegex(ValueError, "outside the target stack"):
            rules.collect(pcb(keepout("", "y", layer="In1.Cu")), 2)

    def test_duplicate_ids_are_refused_without_editing_other_items(self):
        area = keepout("Board", "repeated")
        existing = '(gr_line (uuid "repeated") (start 0 0) (end 0 2))'
        with self.assertRaisesRegex(ValueError, "repeats an existing native item ID"):
            rules.ensure_unique_ids(existing, [area])
        with self.assertRaisesRegex(ValueError, "repeats an existing native item ID"):
            rules.ensure_unique_ids("", [area, keepout("Other", "repeated")])
        result = rules.check([area], pcb(area, existing), 2)
        self.assertFalse(result["satisfied"])
        self.assertEqual(result["duplicate_item_ids"], 1)

    def test_mixed_copper_and_technical_layers_survive_native_alias_expansion(self):
        layers = '(layers (0 "F.Cu" signal) (2 "B.Cu" signal) (43 "Eco1.User" user "Eco1"))'
        area = keepout("Clear out", "a").replace(
            '(layer "B.Cu")', '(layers "F&B.Cu" "Eco1.User")')
        source = pcb(layers, area)
        forms, _ = rules.collect(source, 2)
        saved = area.replace('"F&B.Cu"', '"B.Cu" "F.Cu"')
        self.assertTrue(rules.check(forms, pcb(layers, saved), 2)["satisfied"])
        for changed in (saved.replace(' "Eco1.User"', ''), saved.replace(' "F.Cu"', '')):
            result = rules.check(forms, pcb(layers, changed), 2)
            self.assertFalse(result["satisfied"])
            self.assertEqual(result["missing_or_changed"], 1)
        self.assertFalse(rules.check(forms, pcb(saved), 2)["satisfied"])
        with self.assertRaisesRegex(ValueError, "unavailable PCB layers"):
            rules.collect(pcb(area), 2)

    def test_unrelated_unsupported_rule_area_does_not_reject_a_saved_match(self):
        area = keepout("Authored", "a")
        unsupported = keepout("Unrelated", "x", polygons="(polygon (pts (arc (start 0 0))))")
        self.assertTrue(rules.check([area], pcb(area, unsupported), 2)["satisfied"])
        self.assertFalse(rules.check([area], pcb(unsupported), 2)["satisfied"])


if __name__ == "__main__":
    unittest.main()
