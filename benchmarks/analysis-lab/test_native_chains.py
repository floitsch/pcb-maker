# Copyright (C) 2026 Toit contributors.
from collections import Counter
import math
import unittest

import native_chains
import native_graph
from test_native_graph import board, pad, track


class NativeChainsTests(unittest.TestCase):
    def test_native_contact_packet_is_supported_and_stale_packet_rejected(self):
        from test_native_contacts import contact_fixture
        source, packet = contact_fixture()
        result = native_chains.inspect_chains(source, "N", native_contacts=packet)
        self.assertEqual(result["native_contacts_sha256"], packet["packet_sha256"])
        chain = result["nets"][0]["chains"][0]
        self.assertTrue(all("native_pad_contact" in boundary for boundary in chain["endpoint_boundaries"]))
        source["native_sha256"] = "changed"
        with self.assertRaisesRegex(native_graph.UnsupportedGeometry, "mismatch"):
            native_chains.inspect_chains(source, native_contacts=packet)

    def inspect(self, source, *, pad_contacts=False):
        graph = native_graph.build_graph(source, "N", pad_contacts=pad_contacts)
        result = native_chains.chains_for_graph(graph)
        self.assertEqual(Counter(edge for chain in result["chains"] for edge in chain["edge_ids"]),
                         Counter(edge["id"] for edge in graph["edges"] if edge["type"] == "track"))
        self.assertAlmostEqual(result["summary"]["path_length_mm"], graph["summary"]["physical_atomic_length_mm"])
        for chain in result["chains"]:
            self.assertEqual(len(chain["points"]), len(chain["edge_ids"])+1)
            self.assertEqual(len(chain["widths_mm"]), 1)
        return result

    def test_open_chain_preserves_exact_points_and_source_atoms(self):
        source = board([track("T1", [0, 0], [1, 1]), track("T2", [1, 1], [2, 0])])
        result = self.inspect(source)
        self.assertEqual(result["summary"]["chains"], 1)
        chain = result["chains"][0]
        self.assertEqual(chain["source_track_ids"], ["T1", "T2"])
        self.assertAlmostEqual(chain["path_length_mm"], 2*math.sqrt(2))
        self.assertEqual(chain["endpoint_distance_mm"], 2)
        self.assertEqual(chain["points"][1], [1, 1])
        self.assertEqual(chain["points_nm"][1], [1000000, 1000000])
        self.assertFalse(chain["closed"])
        source["tracks"].reverse()
        self.assertEqual(result, self.inspect(source))

    def test_branch_splits_atoms_and_reports_partial_source_coverage(self):
        source = board([track("long", [0, 0], [4, 0]), track("branch", [2, 0], [2, 3])])
        result = self.inspect(source)
        self.assertEqual(result["summary"]["chains"], 3)
        coverage = [entry for chain in result["chains"] for entry in chain["source_track_coverage"] if entry["id"] == "long"]
        self.assertEqual(len(coverage), 2)
        self.assertTrue(all(entry["total_graph_atomic_edges"] == 2 and not entry["covers_all_graph_atoms"] for entry in coverage))

    def test_pad_and_via_are_barriers(self):
        source = board([track("T", [0, 0], [6, 0])], [pad("P", [2, 0])],
                       [{"id": "V", "net": "N", "at": [4, 0], "layers": ["front", "back"], "diameter": .8, "drill": .4}])
        result = self.inspect(source)
        self.assertEqual(result["summary"]["chains"], 3)
        reasons = {reason for chain in result["chains"] for endpoint in chain["endpoint_boundaries"] for reason in endpoint}
        self.assertTrue({"pad_terminal", "via"} <= reasons)

    def test_fixed_pad_contacts_are_barriers_only_when_enabled(self):
        source = board([track("T1", [0, 0], [2, 0]), track("T2", [2, 0], [4, 0])], [pad("P", [2, .5])])
        self.assertEqual(self.inspect(source)["summary"]["chains"], 1)
        result = self.inspect(source, pad_contacts=True)
        self.assertEqual(result["summary"]["chains"], 2)
        self.assertTrue(any("pad_contact" in endpoint for chain in result["chains"] for endpoint in chain["endpoint_boundaries"]))

    def test_plated_pad_bridge_and_same_position_layers_do_not_join_chains(self):
        source = board([track("front", [0, 0], [4, 0]), track("back", [0, 0], [4, 0], "back")],
                       [pad("P", [2, 0], ["front", "back"], True)])
        result = self.inspect(source)
        self.assertEqual(result["summary"]["chains"], 4)
        self.assertEqual(Counter(chain["layer"] for chain in result["chains"]), {"front": 2, "back": 2})
        self.assertTrue(all("pad_bridge" in sum(chain["endpoint_boundaries"], []) for chain in result["chains"]))

    def test_width_changes_are_boundaries(self):
        source = board([track("T1", [0, 0], [2, 0], width=.3), track("T2", [2, 0], [4, 0], width=.6)])
        result = self.inspect(source)
        self.assertEqual(result["summary"]["chains"], 2)
        self.assertEqual(sorted(chain["widths_mm"] for chain in result["chains"]), [[.3], [.6]])
        self.assertTrue(all("width_change" in sum(chain["endpoint_boundaries"], []) for chain in result["chains"]))

    def test_closed_cycle_is_one_ordered_path_with_repeated_endpoint(self):
        source = board([track("T1", [0, 0], [4, 0]), track("T2", [4, 0], [4, 4]),
                        track("T3", [4, 4], [0, 4]), track("T4", [0, 4], [0, 0])])
        result = self.inspect(source)
        self.assertEqual(result["summary"]["chains"], 1)
        chain = result["chains"][0]
        self.assertTrue(chain["closed"])
        self.assertEqual(chain["points"][0], chain["points"][-1])
        self.assertEqual(chain["endpoint_distance_mm"], 0)
        self.assertEqual(chain["path_length_mm"], 16)
        self.assertEqual(chain["endpoint_boundaries"], [[], []])
        source["tracks"].reverse()
        self.assertEqual(result, self.inspect(source))

    def test_cycle_touching_a_branch_is_not_split_at_arbitrary_node(self):
        source = board([track("T1", [0, 0], [4, 0]), track("T2", [4, 0], [2, 2]),
                        track("T3", [2, 2], [0, 0]), track("T4", [0, 0], [-2, 0])])
        result = self.inspect(source)
        self.assertEqual(result["summary"]["chains"], 2)
        self.assertEqual(result["summary"]["closed_chains"], 1)
        closed = next(chain for chain in result["chains"] if chain["closed"])
        self.assertEqual(closed["endpoint_boundaries"], [["branch"], ["branch"]])

    def test_overlapping_source_objects_share_one_atom(self):
        source = board([track("T1", [0, 0], [4, 0]), track("T2", [1, 0], [3, 0])])
        result = self.inspect(source)
        self.assertEqual(result["summary"]["atomic_track_edges"], 3)
        self.assertEqual(result["summary"]["chains"], 1)
        self.assertTrue(all(entry["covers_all_graph_atoms"] for entry in result["chains"][0]["source_track_coverage"]))

    def test_empty_track_net_and_unsupported_net_are_explicit(self):
        result = native_chains.inspect_chains(board([], [pad("P", [0, 0])]))
        self.assertEqual(result["nets"][0]["chains"], [])
        self.assertTrue(result["nets"][0]["caveats"])
        result = native_chains.inspect_chains(board([track("T", [0, 0], [0, 0])]))
        self.assertTrue(result["nets"][0]["unsupported"])


if __name__ == "__main__":
    unittest.main()
