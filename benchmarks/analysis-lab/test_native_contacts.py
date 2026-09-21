# Copyright (C) 2026 Toit contributors.
import copy
import json
from pathlib import Path
import tempfile
from types import SimpleNamespace
import unittest
from unittest.mock import Mock

import native_contacts
import native_graph
from test_native_graph import board, pad, track


def contact_fixture():
    source = board([track("T1", [.5, 0], [4.5, 0])], [pad("P1", [0, 0]), pad("P2", [5, 0])])
    for section in ["tracks", "pads"]:
        for item in source[section]:
            item["uuid"] = "uuid-" + item["id"]
    records = [{"track_id": "T1", "track_uuid": "uuid-T1", "pad_id": pad_id, "pad_uuid": "uuid-"+pad_id,
                "net": "N", "layer": "front", "endpoint": endpoint, "at_nm": at}
               for pad_id, endpoint, at in [("P1", "start", [500000, 0]), ("P2", "end", [4500000, 0])]]
    return source, native_contacts.make_packet(source, records, "test-native")


def rehash(packet):
    packet["packet_sha256"] = native_graph.digest({k: v for k, v in packet.items() if k != "packet_sha256"})
    return packet


class NativeContactTests(unittest.TestCase):
    def test_valid_packet_and_stale_sources(self):
        source, packet = contact_fixture()
        self.assertEqual(native_contacts.validate_packet(source, packet), packet["contacts"])
        for change in [{"native_sha256": "changed"}, {"project_sha256": "changed"}, {"id": "different"}]:
            with self.subTest(change=change), self.assertRaisesRegex(native_graph.UnsupportedGeometry, "mismatch"):
                native_contacts.validate_packet(dict(source, **change), packet)
        altered = copy.deepcopy(source)
        altered["tracks"][0]["width"] = .5
        with self.assertRaisesRegex(native_graph.UnsupportedGeometry, "geometry_sha256 mismatch"):
            native_contacts.validate_packet(altered, packet)
        altered = copy.deepcopy(packet)
        altered["contacts"][0]["at_nm"][0] += 1
        with self.assertRaisesRegex(native_graph.UnsupportedGeometry, "packet hash"):
            native_contacts.validate_packet(source, altered)

    def test_malformed_contact_records_fail_closed_even_with_recomputed_hash(self):
        source, packet = contact_fixture()
        for changes, message in [({"track_id": "unknown"}, "unknown"), ({"pad_id": "unknown"}, "unknown"),
                                 ({"track_uuid": "wrong"}, "UUID"), ({"net": "foreign"}, "net"),
                                 ({"layer": "back"}, "layer"), ({"endpoint": "middle"}, "endpoint"),
                                 ({"at_nm": [2500000, 0]}, "endpoint"), ({"at_nm": [500000.0, 0]}, "endpoint")]:
            with self.subTest(changes=changes):
                altered = copy.deepcopy(packet)
                altered["contacts"][0].update(changes)
                with self.assertRaisesRegex(native_graph.UnsupportedGeometry, message):
                    native_contacts.validate_packet(source, rehash(altered))
        altered = copy.deepcopy(packet)
        altered["contacts"].append(altered["contacts"][0])
        with self.assertRaisesRegex(native_graph.UnsupportedGeometry, "duplicate"):
            native_contacts.validate_packet(source, rehash(altered))

    def test_unsupported_method_or_version_is_rejected(self):
        source, packet = contact_fixture()
        for changes in [{"schema_version": 2}, {"method": "GetConnectedItems"}, {"kicad_version": ""}, {"units": "mm"}]:
            with self.subTest(changes=changes), self.assertRaises(native_graph.UnsupportedGeometry):
                native_contacts.validate_packet(source, rehash(dict(packet, **changes)))

    def test_export_checks_source_hashes_before_loading_native_board(self):
        with tempfile.TemporaryDirectory() as temporary:
            directory = Path(temporary)
            pcb = directory/"board.kicad_pcb"
            geometry = directory/"geometry.json"
            pcb.write_text("source")
            source, _ = contact_fixture()
            geometry.write_text(json.dumps(source))
            with self.assertRaisesRegex(ValueError, "board hash"):
                native_contacts.export_contacts(pcb, geometry)
            source["native_sha256"] = native_contacts.file_sha256(pcb)
            geometry.write_text(json.dumps(source))
            with self.assertRaisesRegex(ValueError, "project hash"):
                native_contacts.export_contacts(pcb, geometry)
            self.assertEqual(pcb.read_text(), "source")

    def test_collector_requires_direct_contact_endpoint_hit_same_net_and_layer(self):
        source, _ = contact_fixture()
        source["tracks"][0]["net"] = "N1"
        for p in source["pads"]:
            p["net"] = "N1"
        native = Mock()
        native.GetLayerName.side_effect = {0: "front", 2: "back"}.__getitem__
        native.GetEnabledLayers.return_value.CuStack.return_value = [0, 2]
        native_track = Mock()
        native_track.GetClass.return_value = "PCB_TRACK"
        native_track.m_Uuid.AsString.return_value = "uuid-T1"
        native_track.GetLayer.return_value = 0
        native_track.GetNetCode.return_value = 1
        native_track.GetStart.return_value = SimpleNamespace(x=500000, y=0)
        native_track.GetEnd.return_value = SimpleNamespace(x=4500000, y=0)
        native.GetTracks.return_value = [native_track]
        native_pads = []
        for record in source["pads"]:
            item = Mock()
            item.m_Uuid.AsString.return_value = record["uuid"]
            item.GetPosition.return_value = SimpleNamespace(x=int(record["at"][0]*1000000), y=0)
            item.GetNetCode.return_value = 1
            item.IsOnLayer.side_effect = lambda layer: layer == 0
            item.HitTest.side_effect = lambda p, accuracy: p.x == 500000 and accuracy == 0
            native_pads.append(item)
        native.GetFootprints.return_value = [Mock(Pads=Mock(return_value=native_pads))]
        connectivity = native.GetConnectivity.return_value
        connectivity.GetConnectedPads.return_value = [native_pads[0]]
        contacts = native_contacts.collect_contacts(native, source)
        self.assertEqual([(c["pad_id"], c["endpoint"]) for c in contacts], [("P1", "start")])
        connectivity.GetConnectedItems.assert_not_called()
        native.BuildConnectivity.assert_called_once_with()
        native_pads[1].HitTest.assert_not_called()  # Transitive membership never consulted.
        # Foreign-net and other-layer pads are excluded even if native reports
        # them and a mocked outline HitTest would accept the endpoint.
        for foreign_net, layers in [(2, ["front"]), (1, ["back"])]:
            source["pads"][0]["net"] = "N"+str(foreign_net)
            source["pads"][0]["layers"] = layers
            native_pads[0].GetNetCode.return_value = foreign_net
            native_pads[0].IsOnLayer.side_effect = lambda layer, layers=layers: {0: "front", 2: "back"}[layer] in layers
            self.assertEqual(native_contacts.collect_contacts(native, source), [])


if __name__ == "__main__":
    unittest.main()
