# Copyright (C) 2026 Toit contributors.
import math
import unittest

import native_graph as graph


def board(tracks, pads=(), vias=(), layers=("front", "back")):
    return {"schema_version": 1, "format": "analysis-lab-native-geometry", "id": "unit",
            "native_sha256": "unit-source", "nets": {"N": "unit"},
            "layers": [{"name": layer} for layer in layers], "tracks": tracks,
            "pads": list(pads), "vias": list(vias), "zones": []}


def track(object_id, start, end, layer="front", width=.3):
    return {"id": object_id, "net": "N", "start": start, "end": end, "width": width, "layer": layer}


def pad(object_id, at, layers=("front",), plated=False):
    return {"id": object_id, "net": "N", "at": at, "layers": list(layers),
            "attribute": 0 if plated else 1, "drill": [1, 1] if plated else [0, 0],
            "shape": "circle", "size": [2, 2], "rotation_degrees": 0}


class NativeGraphTests(unittest.TestCase):
    def test_native_contact_edges_are_fixed_deduplicated_and_do_not_emit_copper(self):
        from test_native_contacts import contact_fixture
        source, packet = contact_fixture()
        result = graph.build_graph(source, "N", native_contacts=packet)
        contacts = [e for e in result["edges"] if e["type"] == "native_pad_contact"]
        self.assertEqual(len(contacts), 2)
        self.assertTrue(all(e["fixed"] and e["cost_mm"] == 0 and e["evidence"]["confidence"] == "native" for e in contacts))
        self.assertTrue(result["summary"]["pad_coverage"]["all_terminals_connected_in_model"])
        self.assertEqual(result["summary"]["pad_coverage"]["with_native_contact_only"], ["P1", "P2"])
        selected = [e["id"] for e in result["edges"] if e["type"] == "track"]
        proposal = graph.materialize(result, {"selected_edges": selected})
        self.assertEqual(len(proposal["add_tracks"]), 1)
        self.assertEqual(proposal["add_tracks"][0]["start"], [.5, 0])
        self.assertTrue(proposal["inspection_provenance"]["fixed_native_pad_contacts_retained"])
        with self.assertRaisesRegex(ValueError, "connectivity"):
            graph.materialize(result, {"selected_edges": []})
        with self.assertRaisesRegex(graph.UnsupportedGeometry, "one pad contact mode"):
            graph.build_graph(source, "N", pad_contacts=True, native_contacts=packet)

    def test_native_contact_witness_track_may_be_removed_and_shared_contacts_deduplicate(self):
        import native_contacts
        source = board([track("left", [-4, 0], [-.5, 0]), track("right", [.5, 0], [4, 0]),
                        track("witness", [-.5, 0], [.5, 0])],
                       [pad("P1", [-4, 0]), pad("P2", [0, 0]), pad("P3", [4, 0])])
        for section in ["tracks", "pads"]:
            for item in source[section]:
                item["uuid"] = "uuid-"+item["id"]
        contacts = [{"track_id": track_id, "track_uuid": "uuid-"+track_id, "pad_id": "P2", "pad_uuid": "uuid-P2",
                     "net": "N", "layer": "front", "endpoint": endpoint, "at_nm": at}
                    for track_id, endpoint, at in [("witness", "start", [-500000, 0]),
                                                  ("witness", "end", [500000, 0]), ("left", "end", [-500000, 0])]]
        packet = native_contacts.make_packet(source, contacts, "test-native")
        result = graph.build_graph(source, "N", native_contacts=packet)
        self.assertEqual(result["summary"]["native_pad_contact_edges"], 2)
        self.assertEqual(sorted(len(e["evidence"]["witnesses"]) for e in result["edges"] if e["type"] == "native_pad_contact"), [1, 2])
        selected = [e["id"] for e in result["edges"] if e["type"] == "track" and "witness" not in e["source_ids"]]
        proposal = graph.materialize(result, {"selected_edges": selected})
        self.assertEqual(len(proposal["add_tracks"]), 2)
        self.assertEqual(sum(math.dist(t["start"], t["end"]) for t in proposal["add_tracks"]), 7)

    def test_overlap_reversed_diagonal_is_atomic_union_and_stable(self):
        source = board([track("T1", [0, 0], [4, 4]), track("T2", [3, 3], [1, 1])])
        result = graph.build_graph(source, "N")
        self.assertEqual(result["summary"]["track_atomic_edges"], 3)
        self.assertAlmostEqual(result["summary"]["physical_atomic_length_mm"], 4*math.sqrt(2))
        self.assertEqual([e["source_ids"] for e in result["edges"] if len(e["source_ids"]) == 2], [["T1", "T2"]])
        source["tracks"].reverse()
        self.assertEqual(result["graph_sha256"], graph.build_graph(source, "N")["graph_sha256"])

    def test_nonuniform_coincident_width_fails_closed(self):
        source = board([track("T1", [0, 0], [4, 4]), track("T2", [3, 3], [1, 1], width=.4)])
        with self.assertRaisesRegex(graph.UnsupportedGeometry, "nonuniform"):
            graph.build_graph(source, "N")

    def test_plated_pad_layer_bridge_is_fixed(self):
        source = board([track("T1", [0, 0], [5, 0]), track("T2", [5, 0], [10, 0], "back")],
            [pad("P1", [0, 0]), pad("P2", [5, 0], ["front", "back"], True), pad("P3", [10, 0], ["back"])])
        result = graph.build_graph(source, "N")
        self.assertEqual(result["summary"]["connected_components"], 1)
        selection = {"selected_edges": [e["id"] for e in result["edges"] if e["type"] == "track"]}
        proposal = graph.materialize(result, selection)
        self.assertEqual(len(proposal["add_tracks"]), 2)
        self.assertTrue(proposal["inspection_provenance"]["fixed_pad_bridges_retained"])
        source["pads"][1]["attribute"] = 1
        result = graph.build_graph(source, "N")
        self.assertEqual(result["summary"]["connected_components"], 2)

    def test_cycles_and_boundary_selection(self):
        source = board([track("T1", [0, 0], [4, 0]), track("T2", [4, 0], [4, 4]),
                        track("T3", [4, 4], [0, 4]), track("T4", [0, 4], [0, 0])],
                       [pad("P1", [0, 0]), pad("P2", [4, 4])])
        result = graph.build_graph(source, "N")
        self.assertEqual(result["summary"]["cycle_rank"], 1)
        selected = [edge["id"] for edge in result["edges"] if edge["source_ids"][0] in ["T1", "T2"]]
        proposal = graph.materialize(result, {"selected_edges": selected})
        self.assertEqual(len(proposal["remove_tracks"]), 4)
        self.assertEqual(len(proposal["add_tracks"]), 2)
        with self.assertRaisesRegex(ValueError, "connectivity"):
            graph.materialize(result, {"selected_edges": selected[:1]})
        with self.assertRaisesRegex(ValueError, "Unknown"):
            graph.materialize(result, {"selected_edges": ["not-an-edge"]})
        with self.assertRaisesRegex(ValueError, "hash"):
            graph.materialize(result, {"selected_edges": selected, "graph_sha256": "stale"})

    def test_via_groups_are_atomic_on_arbitrary_layers(self):
        source = board([], [pad("P1", [1, 1], ["a"]), pad("P2", [1, 1], ["c"])],
            [{"id": "V1", "net": "N", "at": [1, 1], "layers": ["a", "b", "c"], "diameter": .8, "drill": .4}],
            layers=["a", "b", "c"])
        result = graph.build_graph(source, "N")
        self.assertEqual(result["summary"]["cost_mm"], 2)
        selected = [edge["id"] for edge in result["edges"]]
        with self.assertRaisesRegex(ValueError, "Partial via"):
            graph.materialize(result, {"selected_edges": selected[:1]})
        self.assertEqual(graph.materialize(result, {"selected_edges": selected})["remove_vias"], [])

    def test_nonendpoint_crossing_is_explicitly_not_inferred(self):
        source = board([track("T1", [0, 2], [4, 2]), track("T2", [2, 0], [2, 4])])
        result = graph.build_graph(source, "N")
        self.assertEqual(result["summary"]["connected_components"], 2)
        self.assertTrue(any("interior/interior" in caveat for caveat in result["caveats"]))

    def test_fractional_nanometer_and_zero_length_are_rejected(self):
        for source in [board([track("T", [0, 0], [.0000001, 1])]), board([track("T", [0, 0], [0, 0])])]:
            with self.assertRaises(graph.UnsupportedGeometry):
                graph.build_graph(source, "N")

    def test_off_center_pad_contact_is_opt_in_and_emits_only_selected_copper(self):
        source = board([track("T1", [.5, 0], [4.5, 0])],
                       [pad("P1", [0, 0]), pad("P2", [5, 0])])
        default = graph.build_graph(source, "N")
        self.assertEqual(default, graph.build_graph(source, "N", pad_contacts=False))
        self.assertNotIn("pad_contacts", default)
        self.assertFalse(default["summary"]["pad_coverage"]["all_terminals_connected_in_model"])
        result = graph.build_graph(source, "N", pad_contacts=True)
        coverage = result["summary"]["pad_coverage"]
        self.assertEqual(coverage["with_exact_track_centerline_contact"], 0)
        self.assertEqual(coverage["with_track_contact_in_model"], 2)
        self.assertEqual(coverage["with_inferred_contact_only"], ["P1", "P2"])
        self.assertTrue(coverage["all_terminals_connected_in_model"])
        self.assertEqual(result["summary"]["pad_contact_edges"], 2)
        contacts = [e for e in result["edges"] if e["type"] == "pad_contact"]
        self.assertTrue(all(e["fixed"] and e["cost_mm"] == 0 and
                            e["evidence"]["confidence"] == "inference" for e in contacts))
        selected = [e["id"] for e in result["edges"] if e["type"] == "track"]
        proposal = graph.materialize(result, {"selected_edges": selected})
        self.assertEqual(proposal["add_tracks"], [{k: v for k, v in source["tracks"][0].items() if k != "id"}])
        self.assertEqual(len(proposal["inspection_provenance"]["explicit_selected_edges"]), 3)
        self.assertTrue(proposal["inspection_provenance"]["fixed_pad_contacts_retained"])
        with self.assertRaisesRegex(ValueError, "connectivity"):
            graph.materialize(result, {"selected_edges": []})

    def test_pad_outline_rejects_boundary_outside_and_rounded_corners(self):
        cases = [
            ("circle", [2, 2], None, [.7, .7], [.9, .9]),
            ("rect", [2, 2], None, [.9, .9], [1.01, 0]),
            ("oval", [4, 2], None, [1.5, .5], [1.9, .9]),
            ("roundrect", [2, 2], .5, [.7, .7], [.99, .99]),
        ]
        for shape, size, radius, inside, outside in cases:
            with self.subTest(shape=shape):
                terminal = dict(pad("P1", [0, 0]), shape=shape, size=size)
                if radius is not None:
                    terminal["roundrect_radius"] = radius
                source = board([track("inside", inside, [10, 10]),
                                track("outside", outside, [10, 11]),
                                track("boundary", [size[0]/2, 0], [10, 12])], [terminal])
                result = graph.build_graph(source, "N", pad_contacts=True)
                contacts = [e for e in result["edges"] if e["type"] == "pad_contact"]
                self.assertEqual(len(contacts), 1)
                nodes = {n["id"]: n for n in result["nodes"]}
                self.assertEqual(nodes[contacts[0]["to"]]["at"], inside)

    def test_rotated_offset_outlines_agree_with_probe_for_outside_distance(self):
        from native_probe import pad_distance
        for shape in ["circle", "rect", "oval", "roundrect"]:
            terminal = dict(pad("P1", [3, 4]), shape=shape, size=[4, 2] if shape != "circle" else [2, 2],
                            rotation_degrees=90, offset=[1, 0], roundrect_radius=.4)
            self.assertLess(graph.pad_interior_distance([3, 3], terminal), 0)
            self.assertGreater(graph.pad_interior_distance([3, 5.1], terminal), 0)
            for p in [[3, 3], [3, 5], [2.1, 1.1], [4, 4], [0, 0]]:
                self.assertAlmostEqual(max(0, graph.pad_interior_distance(p, terminal)), pad_distance(p, p, terminal))

    def test_smd_contact_layer_isolation_and_pth_exclusion(self):
        for plated in [False, True]:
            with self.subTest(plated=plated):
                terminal = pad("P1", [0, 0], ["front", "back"] if plated else ["front"], plated)
                source = board([track("T1", [.75, 0], [3, 0]),
                                track("T2", [.75, 0], [3, 0], "back")], [terminal])
                result = graph.build_graph(source, "N", pad_contacts=True)
                contacts = [e for e in result["edges"] if e["type"] == "pad_contact"]
                self.assertEqual(len(contacts), 0 if plated else 1)
                if contacts:
                    self.assertEqual(contacts[0]["layer"], "front")
                self.assertEqual(result["summary"]["pad_coverage"]["excluded_from_pad_contact_inference"],
                                 ["P1"] if plated else [])
                self.assertEqual(sum(e["type"] == "pad_bridge" for e in result["edges"]), int(plated))

    def test_fixed_pad_contacts_connect_distinct_points_after_track_removal(self):
        source = board([track("left", [-4, 0], [-.5, 0]),
                        track("right", [.5, 0], [4, 0]),
                        track("redundant", [-.5, 0], [.5, 0])],
                       [pad("P1", [-4, 0]), pad("P2", [0, 0]), pad("P3", [4, 0])])
        result = graph.build_graph(source, "N", pad_contacts=True)
        selected = [e["id"] for e in result["edges"] if e["type"] == "track" and "redundant" not in e["source_ids"]]
        proposal = graph.materialize(result, {"selected_edges": selected})
        self.assertEqual(len(proposal["add_tracks"]), 2)
        self.assertEqual(sum(math.dist(t["start"], t["end"]) for t in proposal["add_tracks"]), 7)
        with self.assertRaisesRegex(ValueError, "connectivity"):
            graph.materialize(result, {"selected_edges": selected[:1]})

    def test_inference_rejects_unsupported_or_incomplete_smd_geometry(self):
        for changes in [{"shape": "custom"}, {"size": [0, 2]}, {"roundrect_radius": 2, "shape": "roundrect"}]:
            source = board([], [dict(pad("P1", [0, 0]), **changes)])
            graph.build_graph(source, "N")
            with self.assertRaisesRegex(graph.UnsupportedGeometry, "cannot infer"):
                graph.build_graph(source, "N", pad_contacts=True)


if __name__ == "__main__":
    unittest.main()
