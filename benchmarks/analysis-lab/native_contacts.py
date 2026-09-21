#!/usr/bin/env python3
# Copyright (C) 2026 Toit contributors.
"""Export direct native track-endpoint/pad contacts without changing a board."""

import argparse
from collections import Counter, defaultdict
import hashlib
import json
from pathlib import Path

import native_graph


FORMAT = "analysis-lab-native-pad-contacts"
METHOD = "BuildConnectivity; GetConnectedPads(track); same net/layer; pad.HitTest(endpoint, 0)"
CAVEATS = [
    "Evidence is limited to original straight-track endpoints directly connected to a pad and accepted by native PAD.HitTest with zero accuracy on the same copper layer.",
    "HitTest describes native pad hit geometry, not a drill-subtracted copper occupancy proof. Plated-hole center/outline membership can include the hole; no independent hole or annular-ring validity is claimed.",
    "No track-interior, track/via capsule, transitive connectivity, or new-position contacts are inferred. Exact default graph contacts still apply.",
    "Fixed endpoint-to-pad edges represent the unchanged pad; source tracks are evidence, not required selections. Native DRC and connectivity must validate every materialized selection.",
    "Hashes bind this inspection packet to source geometry; the packet is not a signed attestation or a substitute for native validation.",
]


def file_sha256(path):
    return hashlib.sha256(Path(path).read_bytes()).hexdigest()


def make_packet(geometry, contacts, kicad_version):
    packet = {"schema_version": 1, "format": FORMAT, "board_id": geometry["id"],
        "native_sha256": geometry["native_sha256"], "project_sha256": geometry.get("project_sha256"),
        "geometry_sha256": native_graph.digest(geometry),
        "geometry_hash_method": "SHA256 of sorted-key compact JSON",
        "method": METHOD, "kicad_version": kicad_version, "units": "nm",
        "contacts": sorted(contacts, key=lambda c: (c["net"], c["track_id"], c["endpoint"], c["pad_id"])),
        "caveats": CAVEATS}
    packet["packet_sha256"] = native_graph.digest(packet)
    return packet


def validate_packet(geometry, packet):
    """Reject stale/malformed evidence before any net is built (pure Python)."""
    def reject(message):
        raise native_graph.UnsupportedGeometry("Native contact packet: " + message)

    if not isinstance(packet, dict) or packet.get("format") != FORMAT or packet.get("schema_version") != 1:
        reject("unsupported format/version")
    if packet.get("packet_sha256") != native_graph.digest({k: v for k, v in packet.items() if k != "packet_sha256"}):
        reject("packet hash mismatch")
    for key, expected in [("board_id", geometry["id"]), ("native_sha256", geometry.get("native_sha256")),
                          ("project_sha256", geometry.get("project_sha256")),
                          ("geometry_sha256", native_graph.digest(geometry))]:
        if packet.get(key) != expected or (key != "project_sha256" and not expected):
            reject(key + " mismatch")
    if packet.get("method") != METHOD or packet.get("units") != "nm" or not packet.get("kicad_version"):
        reject("unsupported evidence method, units, or missing native version")
    if packet.get("geometry_hash_method") != "SHA256 of sorted-key compact JSON":
        reject("unsupported geometry hash method")
    objects = [item for section in ["tracks", "vias", "pads"] for item in geometry[section]]
    if len({item["id"] for item in objects}) != len(objects):
        reject("geometry object IDs must be unique")
    tracks = {item["id"]: item for item in geometry["tracks"]}
    pads = {item["id"]: item for item in geometry["pads"]}
    layer_names = {item["name"] for item in geometry["layers"]}
    if not isinstance(packet.get("contacts"), list):
        reject("contacts must be a list")
    seen = set()
    for contact in packet["contacts"]:
        if not isinstance(contact, dict):
            reject("contact must be an object")
        track = tracks.get(contact.get("track_id"))
        pad = pads.get(contact.get("pad_id"))
        if track is None or pad is None:
            reject("unknown track or pad object")
        if not track.get("uuid") or not pad.get("uuid") or contact.get("track_uuid") != track["uuid"] or contact.get("pad_uuid") != pad["uuid"]:
            reject("object UUID mismatch")
        if contact.get("net") != track["net"] or contact.get("net") != pad["net"]:
            reject("wrong net")
        layer = contact.get("layer")
        if layer not in layer_names or layer != track["layer"] or layer not in pad["layers"]:
            reject("wrong layer")
        endpoint = contact.get("endpoint")
        at = contact.get("at_nm")
        if endpoint not in ["start", "end"] or not isinstance(at, list) or len(at) != 2 or any(type(v) is not int for v in at):
            reject("invalid exact endpoint")
        if tuple(at) != native_graph.point(track[endpoint]):
            reject("position is not the named original track endpoint")
        key = (track["id"], endpoint, pad["id"])
        if key in seen:
            reject("duplicate contact record")
        seen.add(key)
    return packet["contacts"]


def collect_contacts(board, geometry):
    """Read loaded native objects; never use transitive GetConnectedItems."""
    tracks = {item["uuid"]: item for item in geometry["tracks"]}
    pads = defaultdict(list)
    for item in geometry["pads"]:
        pads[item["uuid"]].append(item)
    if len(tracks) != len(geometry["tracks"]):
        raise ValueError("Geometry track UUIDs must be unique")
    # Some exported boards contain repeated, identical pads with the same UUID.
    # Retain every public alias, but never resolve conflicting UUIDs arbitrarily.
    for aliases in pads.values():
        if len({native_graph.digest({k: v for k, v in pad.items() if k != "id"}) for pad in aliases}) != 1:
            raise ValueError("Ambiguous duplicate pad UUID geometry")
    native_tracks = {item.m_Uuid.AsString(): item for item in board.GetTracks() if item.GetClass() == "PCB_TRACK"}
    native_pads = [item for footprint in board.GetFootprints() for item in footprint.Pads()]
    if set(tracks) != set(native_tracks) or Counter(item["uuid"] for item in geometry["pads"]) != Counter(item.m_Uuid.AsString() for item in native_pads):
        raise ValueError("Geometry/native track or pad UUID coverage mismatch")
    copper_layers = list(board.GetEnabledLayers().CuStack())
    for pad in native_pads:
        record = pads[pad.m_Uuid.AsString()][0]
        at = pad.GetPosition()
        layers = {board.GetLayerName(layer) for layer in copper_layers if pad.IsOnLayer(layer)}
        if (native_graph.point(record["at"]) != (at.x, at.y) or record["net"] != f"N{pad.GetNetCode()}"
                or set(record["layers"]) != layers):
            raise ValueError("Geometry/native pad position, net, or layers mismatch")
    board.BuildConnectivity()
    connectivity = board.GetConnectivity()
    contacts = {}
    for uuid, track in sorted(native_tracks.items()):
        record = tracks[uuid]
        layer = board.GetLayerName(track.GetLayer())
        endpoints = {"start": track.GetStart(), "end": track.GetEnd()}
        if record["net"] != f"N{track.GetNetCode()}" or record["layer"] != layer:
            raise ValueError("Geometry/native track net or layer mismatch")
        if any(native_graph.point(record[key]) != (p.x, p.y) for key, p in endpoints.items()):
            raise ValueError("Geometry/native track endpoint mismatch")
        for pad in connectivity.GetConnectedPads(track):
            if pad.GetNetCode() != track.GetNetCode() or not pad.IsOnLayer(track.GetLayer()):
                continue
            pad_uuid = pad.m_Uuid.AsString()
            for endpoint, p in endpoints.items():
                if pad.HitTest(p, 0):
                    for alias in pads[pad_uuid]:
                        contacts[(record["id"], endpoint, alias["id"])] = {
                            "track_id": record["id"], "track_uuid": uuid,
                            "pad_id": alias["id"], "pad_uuid": pad_uuid,
                            "net": record["net"], "layer": layer, "endpoint": endpoint,
                            "at_nm": [p.x, p.y]}
    return list(contacts.values())


def export_contacts(board_path, geometry_path):
    board_path, geometry_path = Path(board_path), Path(geometry_path)
    geometry = json.loads(geometry_path.read_text())
    if geometry.get("format") != "analysis-lab-native-geometry":
        raise ValueError("Expected native geometry inspection JSON")
    if file_sha256(board_path) != geometry.get("native_sha256"):
        raise ValueError("Geometry/native board hash mismatch")
    project_path = board_path.with_suffix(".kicad_pro")
    if not project_path.is_file() or file_sha256(project_path) != geometry.get("project_sha256"):
        raise ValueError("Geometry/native project hash mismatch")
    import pcbnew
    if not hasattr(pcbnew.SwigPyIterator, "next"):
        pcbnew.SwigPyIterator.next = pcbnew.SwigPyIterator.__next__
    board = pcbnew.LoadBoard(str(board_path))
    packet = make_packet(geometry, collect_contacts(board, geometry), pcbnew.GetBuildVersion())
    validate_packet(geometry, packet)
    # Also fail if either source changed while the native board was inspected.
    if file_sha256(board_path) != packet["native_sha256"] or file_sha256(project_path) != packet["project_sha256"]:
        raise ValueError("Native sources changed during contact extraction")
    return packet


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("board", type=Path)
    parser.add_argument("geometry", type=Path)
    parser.add_argument("--output", type=Path, required=True)
    args = parser.parse_args()
    if args.output.resolve() in {args.board.resolve(), args.geometry.resolve(), args.board.with_suffix(".kicad_pro").resolve()}:
        parser.error("Output must not overwrite a source file")
    packet = export_contacts(args.board, args.geometry)
    native_graph.write_json(args.output, packet)
    print(json.dumps({"output": str(args.output), "contacts": len(packet["contacts"]), "packet_sha256": packet["packet_sha256"]}))


if __name__ == "__main__":
    main()
