#!/usr/bin/env python3
# Copyright (C) 2026 Toit contributors.
"""Inspect overlap-aware native centerline graphs; never choose improvements.

This graph is a conservative inspection representation, not a native DRC or
connectivity replacement. An opt-in pad-contact model connects existing nodes
inside supported pad outlines. Native KiCad must validate every proposal.
"""

import argparse
from collections import defaultdict
from decimal import Decimal
import hashlib
import json
import math
from pathlib import Path

SCALE = 1_000_000  # Native KiCad integer nanometers per millimeter.
CAVEATS = [
    "Only exact centerline contacts at supplied track endpoints or pad/via centers are represented.",
    "Proper interior/interior crossings without an existing endpoint or center are not split or connected.",
    "Finite-width copper contacts, pad boundary contacts away from centers, and nonzero pad copper offsets are not inferred.",
    "Plated through-hole pad centers bridge their supplied copper layers; other pad types do not imply layer bridges.",
    "Track widths and existing geometry are preserved. Coincident atomic edges with differing widths are unsupported.",
    "Graph cycles and connectivity describe this limited representation, not proof of a valid or beneficial native repair.",
    "Native KiCad DRC, including dangling-track and clearance checks, remains required after materialization.",
]
PAD_CONTACT_CAVEATS = [
    "Pad contacts are inferred only between existing same-net, same-layer nodes and undrilled SMD pad terminals, strictly inside supported pad copper; no new positions or track geometry are created.",
    "Circle, rect, oval, and roundrect outlines include native rotation and copper offset; a 1e-9 mm interior guard rejects boundary/numerically ambiguous contacts. New contacts to drilled or non-SMD pads are excluded; exact pad-center semantics remain unchanged.",
    "Fixed zero-cost pad_contact edges represent existing pad copper, not physical tracks; all remain present regardless of track selection.",
    CAVEATS[1],
    "Finite-width track/via contacts and pad boundary contacts are not inferred.",
    *CAVEATS[3:],
]
NATIVE_CONTACT_CAVEATS = [
    "Native pad contacts use a source-bound packet of directly connected original track endpoints accepted by same-net, same-layer native pad HitTest(endpoint, 0).",
    "Native HitTest is pad hit geometry, not a drill-subtracted copper occupancy proof; plated-hole endpoint membership does not establish annular-ring or hole validity.",
    "Fixed zero-cost native_pad_contact edges represent unchanged pads and never emit copper; their witness source tracks need not be selected.",
    CAVEATS[1],
    "Other finite-width copper contacts, track-interior pad contacts, and transitive native connections are not added to this centerline model.",
    *CAVEATS[3:],
]


class UnsupportedGeometry(ValueError):
    pass


def integer(value):
    numeric = Decimal(str(value)) * SCALE
    if not numeric.is_finite() or numeric != numeric.to_integral_value():
        raise UnsupportedGeometry(f"Coordinate/width {value!r} is not an exact finite integer nanometer value")
    return int(numeric)


def point(values):
    if len(values) != 2:
        raise UnsupportedGeometry("Expected a two-coordinate point")
    return tuple(integer(value) for value in values)


def mm(values):
    return [value/SCALE for value in values]


def on_segment(p, a, b):
    dx, dy = b[0]-a[0], b[1]-a[1]
    px, py = p[0]-a[0], p[1]-a[1]
    return dx*py == dy*px and 0 <= px*dx+py*dy <= dx*dx+dy*dy


def pad_interior_distance(p, pad):
    """Signed point distance using native_probe.pad_distance's outline model.

    The probe clamps distance to zero, so it cannot distinguish the boundary
    from the interior. Keep its native-angle and local-offset conventions here.
    Negative values are inside the filled outline (drills are not subtracted).
    """
    try:
        w, h = pad["size"]
        x, y = pad["at"]
        ox, oy = pad.get("offset", [0, 0])
        angle = math.radians(pad["rotation_degrees"])
        values = [w, h, x, y, ox, oy, angle]
        if not all(math.isfinite(value) for value in values) or min(w, h) <= 0:
            raise ValueError("nonfinite coordinates or nonpositive size")
        dx, dy = p[0]-x, p[1]-y
        px = abs(dx*math.cos(angle)-dy*math.sin(angle)-ox)
        py = abs(dx*math.sin(angle)+dy*math.cos(angle)-oy)
        shape = pad["shape"]
        if shape == "circle":
            if w != h:
                raise ValueError("circle size is not square")
            return math.hypot(px, py)-w/2
        if shape == "rect":
            radius = 0
        elif shape == "oval":
            radius = min(w, h)/2
        elif shape == "roundrect":
            radius = pad["roundrect_radius"]
            if not math.isfinite(radius) or not 0 <= radius <= min(w, h)/2:
                raise ValueError("invalid rounded rectangle radius")
        else:
            raise ValueError("unsupported pad shape " + str(shape))
        qx, qy = px-w/2+radius, py-h/2+radius
        return math.hypot(max(qx, 0), max(qy, 0))+min(max(qx, qy), 0)-radius
    except (KeyError, TypeError, ValueError) as error:
        raise UnsupportedGeometry(f"Pad {pad.get('id')} cannot infer copper contacts: {error}") from error


def digest(value):
    return hashlib.sha256(json.dumps(value, sort_keys=True, separators=(",", ":")).encode()).hexdigest()


def stable_id(prefix, value):
    return prefix + digest(value)[:16]


class Components:
    def __init__(self, nodes):
        self.parent = {node: node for node in nodes}

    def root(self, node):
        while self.parent[node] != node:
            self.parent[node] = self.parent[self.parent[node]]
            node = self.parent[node]
        return node

    def join(self, a, b):
        a, b = self.root(a), self.root(b)
        if a != b:
            self.parent[max(a, b)] = min(a, b)


def build_graph(board, net, *, pad_contacts=False, native_contacts=None):
    if pad_contacts and native_contacts is not None:
        raise UnsupportedGeometry("Choose only one pad contact mode")
    contacts = []
    if native_contacts is not None:
        from native_contacts import validate_packet
        contacts = validate_packet(board, native_contacts)
    if board.get("format") != "analysis-lab-native-geometry":
        raise UnsupportedGeometry("Expected analysis-lab-native-geometry inspection JSON")
    if board.get("zones"):
        raise UnsupportedGeometry("Copper zones are unsupported")
    tracks = sorted((item for item in board["tracks"] if item["net"] == net), key=lambda item: item["id"])
    vias = sorted((item for item in board["vias"] if item["net"] == net), key=lambda item: item["id"])
    pads = sorted((item for item in board["pads"] if item["net"] == net), key=lambda item: item["id"])
    if not tracks and not vias and not pads:
        raise UnsupportedGeometry(f"Unknown or empty net {net}")
    ids = [item["id"] for item in tracks+vias+pads]
    if len(ids) != len(set(ids)):
        raise UnsupportedGeometry("Object IDs must be unique within a net")
    layer_order = {item["name"]: i for i, item in enumerate(board["layers"])}
    positions = defaultdict(set)
    track_geometry = {}
    for track in tracks:
        layer = track["layer"]
        if layer not in layer_order:
            raise UnsupportedGeometry(f"Unknown layer {layer}")
        a, b, width = point(track["start"]), point(track["end"]), integer(track["width"])
        if a == b or width <= 0:
            raise UnsupportedGeometry("Zero-length tracks and nonpositive widths are unsupported")
        positions[layer].update([a, b])
        track_geometry[track["id"]] = (layer, a, b, width)
    for item in vias+pads:
        if not item["layers"] or len(set(item["layers"])) != len(item["layers"]):
            raise UnsupportedGeometry("Pad/via layers must be nonempty and unique")
        for layer in item["layers"]:
            if layer not in layer_order:
                raise UnsupportedGeometry(f"Unknown layer {layer}")
            positions[layer].add(point(item["at"]))
    keys = sorted((layer, *p) for layer, points in positions.items() for p in points)
    node_ids = {key: stable_id("n-", [net, *key]) for key in keys}
    nodes = [{"id": node_ids[key], "type": "copper_position", "layer": key[0],
              "at": mm(key[1:]), "at_nm": list(key[1:]), "terminals": []} for key in keys]
    node_by_id = {item["id"]: item for item in nodes}
    atomic = {}
    for track in tracks:
        layer, a, b, width = track_geometry[track["id"]]
        split = sorted((p for p in positions[layer] if on_segment(p, a, b)),
                       key=lambda p: (p[0]-a[0])*(b[0]-a[0])+(p[1]-a[1])*(b[1]-a[1]))
        for first, last in zip(split, split[1:]):
            first, last = sorted([first, last])
            record = atomic.setdefault((layer, first, last), {"widths": set(), "source_ids": []})
            record["widths"].add(width)
            record["source_ids"].append(track["id"])
    edges = []
    for (layer, a, b), record in sorted(atomic.items()):
        if len(record["widths"]) != 1:
            raise UnsupportedGeometry("Coincident atomic edge has nonuniform widths: " + ", ".join(record["source_ids"]))
        length = math.hypot(b[0]-a[0], b[1]-a[1])/SCALE
        edge_id = stable_id("e-", [net, "track", layer, a, b])
        edges.append({"id": edge_id, "type": "track", "from": node_ids[(layer, *a)],
            "to": node_ids[(layer, *b)], "layer": layer, "start": mm(a), "end": mm(b),
            "width": next(iter(record["widths"]))/SCALE, "length_mm": length, "cost_mm": length,
            "source_ids": sorted(record["source_ids"]), "fixed": False})
    terminals = []
    for pad in pads:
        at = point(pad["at"])
        layers = sorted(pad["layers"], key=layer_order.get)
        terminal_nodes = [node_ids[(layer, *at)] for layer in layers]
        for node_id in terminal_nodes:
            node_by_id[node_id]["terminals"].append(pad["id"])
        # PAD_ATTRIB_PTH is 0 in the native geometry schema. A drill is also
        # required, so unspecified or non-plated multi-layer shapes fail closed.
        plated = pad.get("attribute") == 0 and any(integer(v) > 0 for v in pad.get("drill", []))
        terminals.append({"id": pad["id"], "nodes": terminal_nodes,
            "layer_bridge": plated and len(layers) > 1, "shape": pad.get("shape"),
            "footprint": pad.get("footprint"), "pad_number": pad.get("pad_number")})
        eligible = pad.get("attribute") == 1 and pad.get("drill") == [0, 0]
        if pad_contacts:
            terminals[-1]["pad_contact_inference_eligible"] = eligible
        if pad_contacts and eligible:
            # Validate even if this pad has no candidate nodes; fail closed for
            # unsupported outlines rather than silently claiming full coverage.
            pad_interior_distance(pad["at"], pad)
            for layer, terminal_node in zip(layers, terminal_nodes):
                for position in sorted(positions[layer]):
                    if position == at:
                        continue
                    distance = pad_interior_distance(mm(position), pad)
                    if distance < -1e-9:
                        other = node_ids[(layer, *position)]
                        edges.append({"id": stable_id("e-", [net, "pad_contact", pad["id"], terminal_node, other]),
                            "type": "pad_contact", "from": terminal_node, "to": other,
                            "layer": layer, "length_mm": 0, "cost_mm": 0,
                            "width": None, "source_ids": [pad["id"]], "fixed": True,
                            "evidence": {"confidence": "inference", "source": "native pad outline geometry",
                                "method": "existing same-layer node strictly inside pad outline",
                                "interior_margin_mm": -distance}})
        if plated:
            for a, b in zip(terminal_nodes, terminal_nodes[1:]):
                edges.append({"id": stable_id("e-", [net, "pad_bridge", pad["id"], a, b]),
                    "type": "pad_bridge", "from": a, "to": b, "length_mm": 0, "cost_mm": 0,
                    "width": None, "source_ids": [pad["id"]], "fixed": True})
    if native_contacts is not None:
        pad_by_id = {pad["id"]: pad for pad in pads}
        native_edges = {}
        for contact in contacts:
            if contact["net"] != net:
                continue
            pad = pad_by_id[contact["pad_id"]]
            layer = contact["layer"]
            a = node_ids[(layer, *point(pad["at"]))]
            b = node_ids[(layer, *contact["at_nm"])]
            if a == b:
                continue  # Already an exact center contact; no self-loop.
            key = (pad["id"], a, b)
            edge = native_edges.setdefault(key, {
                "id": stable_id("e-", [net, "native_pad_contact", *key]),
                "type": "native_pad_contact", "from": a, "to": b, "layer": layer,
                "length_mm": 0, "cost_mm": 0, "width": None, "source_ids": [pad["id"]], "fixed": True,
                "evidence": {"confidence": "native", "method": native_contacts["method"],
                    "packet_sha256": native_contacts["packet_sha256"],
                    "kicad_version": native_contacts["kicad_version"], "witnesses": []}})
            edge["evidence"]["witnesses"].append({"track_id": contact["track_id"], "endpoint": contact["endpoint"]})
        for edge in native_edges.values():
            edge["evidence"]["witnesses"].sort(key=lambda witness: (witness["track_id"], witness["endpoint"]))
        edges.extend(native_edges.values())
    for via in vias:
        at = point(via["at"])
        layers = sorted(via["layers"], key=layer_order.get)
        if len(layers) < 2:
            raise UnsupportedGeometry("Via must bridge at least two supplied copper layers")
        group = "via:" + via["id"]
        for i, (first, last) in enumerate(zip(layers, layers[1:])):
            a, b = node_ids[(first, *at)], node_ids[(last, *at)]
            edges.append({"id": stable_id("e-", [net, "via", via["id"], first, last]),
                "type": "via", "from": a, "to": b, "length_mm": 0,
                "cost_mm": 2 if i == 0 else 0, "width": None,
                "diameter": via["diameter"], "drill": via["drill"], "source_ids": [via["id"]],
                "selection_group": group, "fixed": False})
    edges.sort(key=lambda edge: edge["id"])
    graph = {"schema_version": 1, "format": "analysis-lab-native-centerline-graph", "board_id": board["id"],
        "net": net, "net_name": board.get("nets", {}).get(net), "units": "mm",
        "native_sha256": board.get("native_sha256"), "nodes": nodes, "edges": edges,
        "terminals": terminals, "source_track_ids": [item["id"] for item in tracks],
        "source_via_ids": [item["id"] for item in vias], "caveats": CAVEATS,
        "selection_contract": "Select edge IDs explicitly. Fixed pad bridges are always retained. Every edge of a via selection_group must be retained or removed together; total group cost is 2mm per via."}
    if pad_contacts:
        graph["pad_contacts"] = True
        graph["caveats"] = PAD_CONTACT_CAVEATS
        graph["selection_contract"] = graph["selection_contract"].replace(
            "Fixed pad bridges", "Fixed pad bridges and pad contacts")
    if native_contacts is not None:
        graph["native_contacts"] = {key: native_contacts[key] for key in
                                    ["packet_sha256", "geometry_sha256", "method", "kicad_version"]}
        graph["caveats"] = NATIVE_CONTACT_CAVEATS
        graph["selection_contract"] = graph["selection_contract"].replace(
            "Fixed pad bridges", "Fixed pad bridges and native pad contacts")
    graph["summary"] = graph_summary(graph)
    graph["graph_sha256"] = digest(graph)
    return graph


def graph_summary(graph):
    components = Components(item["id"] for item in graph["nodes"])
    incident_tracks = set()
    for edge in graph["edges"]:
        components.join(edge["from"], edge["to"])
        if edge["type"] == "track":
            incident_tracks.update([edge["from"], edge["to"]])
    component_count = len({components.root(node) for node in components.parent})
    pad_roots = {components.root(node) for terminal in graph["terminals"] for node in terminal["nodes"]}
    covered = [terminal["id"] for terminal in graph["terminals"] if any(node in incident_tracks for node in terminal["nodes"])]
    uncovered = sorted({terminal["id"] for terminal in graph["terminals"]} - set(covered))
    summary = {"net": graph["net"], "net_name": graph["net_name"],
        "source_tracks": len(graph["source_track_ids"]), "source_vias": len(graph["source_via_ids"]),
        "nodes": len(graph["nodes"]), "edges": len(graph["edges"]),
        "track_atomic_edges": sum(edge["type"] == "track" for edge in graph["edges"]),
        "physical_atomic_length_mm": sum(edge["length_mm"] for edge in graph["edges"]),
        "cost_mm": sum(edge["cost_mm"] for edge in graph["edges"]),
        "connected_components": component_count,
        "cycle_rank": len(graph["edges"])-len(graph["nodes"])+component_count,
        "pad_coverage": {"total": len(graph["terminals"]), "with_exact_track_centerline_contact": len(covered),
            "without_exact_track_centerline_contact": uncovered, "terminal_components": len(pad_roots),
            "all_terminals_connected_in_model": len(pad_roots) <= 1},
        "widths_mm": sorted({edge["width"] for edge in graph["edges"] if edge["type"] == "track"}),
        "overlap_atomic_edges": sum(len(edge["source_ids"]) > 1 for edge in graph["edges"] if edge["type"] == "track")}
    if graph.get("pad_contacts") or graph.get("native_contacts"):
        fixed = Components(item["id"] for item in graph["nodes"])
        for edge in graph["edges"]:
            if edge["fixed"]:
                fixed.join(edge["from"], edge["to"])
        track_roots = {fixed.root(node) for node in incident_tracks}
        model_covered = {terminal["id"] for terminal in graph["terminals"]
                         if any(fixed.root(node) in track_roots for node in terminal["nodes"])}
        summary["pad_coverage"].update({
            "with_track_contact_in_model": len(model_covered),
            "without_track_contact_in_model": sorted({terminal["id"] for terminal in graph["terminals"]}-model_covered),
        })
        if graph.get("pad_contacts"):
            summary["pad_contact_edges"] = sum(edge["type"] == "pad_contact" for edge in graph["edges"])
            summary["pad_coverage"].update({
                "with_inferred_contact_only": sorted(model_covered-set(covered)),
                "excluded_from_pad_contact_inference": sorted(terminal["id"] for terminal in graph["terminals"]
                                                              if not terminal["pad_contact_inference_eligible"]),
            })
        else:
            summary["native_pad_contact_edges"] = sum(edge["type"] == "native_pad_contact" for edge in graph["edges"])
            summary["pad_coverage"]["with_native_contact_only"] = sorted(model_covered-set(covered))
    return summary


def materialize(graph, selection):
    if selection.get("net", graph["net"]) != graph["net"]:
        raise ValueError("Selection net does not match graph")
    if selection.get("graph_sha256", graph["graph_sha256"]) != graph["graph_sha256"]:
        raise ValueError("Selection graph hash does not match current graph")
    selected = selection.get("selected_edges")
    if not isinstance(selected, list) or any(not isinstance(item, str) for item in selected):
        raise ValueError("Selection must supply selected_edges as an explicit list of edge IDs")
    if len(selected) != len(set(selected)):
        raise ValueError("Selection contains duplicate edge IDs")
    edge_by_id = {edge["id"]: edge for edge in graph["edges"]}
    unknown = set(selected) - set(edge_by_id)
    if unknown:
        raise ValueError("Unknown selected edge IDs: " + ", ".join(sorted(unknown)))
    selected = set(selected) | {edge["id"] for edge in graph["edges"] if edge["fixed"]}
    via_groups = defaultdict(set)
    for edge in graph["edges"]:
        if edge["type"] == "via":
            via_groups[edge["selection_group"]].add(edge["id"])
    for group, members in via_groups.items():
        if selected & members and not members <= selected:
            raise ValueError("Partial via selection is unsupported: " + group)
    components = Components(item["id"] for item in graph["nodes"])
    for edge_id in selected:
        edge = edge_by_id[edge_id]
        components.join(edge["from"], edge["to"])
    terminal_roots = {components.root(node) for terminal in graph["terminals"] for node in terminal["nodes"]}
    if len(terminal_roots) > 1:
        raise ValueError("Selected graph does not retain connectivity among all supplied pad terminals; native-only contacts cannot be assumed")
    tracks = [edge_by_id[edge_id] for edge_id in sorted(selected) if edge_by_id[edge_id]["type"] == "track"]
    kept_vias = {edge["source_ids"][0] for edge in graph["edges"] if edge["type"] == "via" and edge["id"] in selected}
    proposal = {"remove_tracks": graph["source_track_ids"],
        "remove_vias": sorted(set(graph["source_via_ids"])-kept_vias),
        "add_tracks": [{"net": graph["net"], "start": edge["start"], "end": edge["end"],
                        "width": edge["width"], "layer": edge["layer"]} for edge in tracks],
        "inspection_provenance": {"graph_sha256": graph["graph_sha256"], "net": graph["net"],
            "explicit_selected_edges": sorted(selected), "fixed_pad_bridges_retained": True,
            "normalization": "Every selected physical atomic track is emitted once. No MST, pruning, rerouting, or edge selection was performed by this tool.",
            "native_validation_required": True}}
    if graph.get("pad_contacts"):
        proposal["inspection_provenance"]["fixed_pad_contacts_retained"] = True
        proposal["inspection_provenance"]["pad_contact_evidence"] = "inference; existing pad outline geometry only; no contact edges emitted as copper"
    if graph.get("native_contacts"):
        proposal["inspection_provenance"]["fixed_native_pad_contacts_retained"] = True
        proposal["inspection_provenance"]["native_contacts"] = graph["native_contacts"]
    return proposal


def write_json(path, value):
    path.parent.mkdir(parents=True, exist_ok=True)
    path.write_text(json.dumps(value, indent=2) + "\n")


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("board", type=Path)
    sub = parser.add_subparsers(dest="command", required=True)
    summary = sub.add_parser("summary")
    summary.add_argument("--net")
    graph_parser = sub.add_parser("graph")
    graph_parser.add_argument("--net", required=True)
    graph_parser.add_argument("--output", required=True, type=Path)
    emit = sub.add_parser("materialize")
    emit.add_argument("--net", required=True)
    emit.add_argument("--selection", required=True, type=Path)
    emit.add_argument("--output", required=True, type=Path)
    for command in [summary, graph_parser, emit]:
        modes = command.add_mutually_exclusive_group()
        modes.add_argument("--pad-contacts", action="store_true",
                             help="Infer fixed contacts at existing nodes strictly inside supported pad outlines")
        modes.add_argument("--native-contacts", type=Path, help="Use a source-bound native endpoint/pad contact packet")
    args = parser.parse_args()
    board = json.loads(args.board.read_text())
    contacts = json.loads(args.native_contacts.read_text()) if args.native_contacts else None
    if contacts is not None:
        from native_contacts import validate_packet
        validate_packet(board, contacts)
    if args.command == "summary":
        nets = [args.net] if args.net else sorted({item["net"] for section in ["tracks", "vias", "pads"] for item in board[section]})
        summaries = []
        for net in nets:
            try:
                summaries.append(build_graph(board, net, pad_contacts=args.pad_contacts, native_contacts=contacts)["summary"])
            except UnsupportedGeometry as error:
                summaries.append({"net": net, "unsupported": True, "error": str(error)})
        print(json.dumps({"board_id": board["id"], "nets": summaries,
                          "caveats": NATIVE_CONTACT_CAVEATS if contacts is not None else PAD_CONTACT_CAVEATS if args.pad_contacts else CAVEATS}, indent=2))
    else:
        graph = build_graph(board, args.net, pad_contacts=args.pad_contacts, native_contacts=contacts)
        result = graph if args.command == "graph" else materialize(graph, json.loads(args.selection.read_text()))
        write_json(args.output, result)
        print(json.dumps({"output": str(args.output), "net": args.net, "graph_sha256": graph["graph_sha256"]}))


if __name__ == "__main__":
    main()
