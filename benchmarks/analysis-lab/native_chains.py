#!/usr/bin/env python3
# Copyright (C) 2026 Toit contributors.
"""List maximal unbranched native track chains without choosing routing edits."""

import argparse
from collections import defaultdict
import json
import math
from pathlib import Path

import native_graph


CAVEATS = [
    "Chains are a factual partition of existing atomic track edges, not shortcut candidates, clearance evidence, or edit proposals.",
    "Chains stop at pads, via contacts, fixed pad contacts/bridges, branches, and width or layer changes. Closed paths repeat their first point at the end.",
    "Source track IDs are provenance only: a chain can cover only part of an original object. Never remove every listed source object without accounting for all its remaining atoms.",
    "Endpoint distance is a geometric measurement only; a direct endpoint segment may cross obstacles or disconnect copper.",
]


def chains_for_graph(graph):
    """Partition graph track atoms exactly once, retaining their exact points."""
    nodes = {node["id"]: node for node in graph["nodes"]}
    edges = {edge["id"]: edge for edge in graph["edges"]}
    incident = defaultdict(list)
    source_atoms = defaultdict(set)
    for edge in graph["edges"]:
        incident[edge["from"]].append(edge["id"])
        incident[edge["to"]].append(edge["id"])
        if edge["type"] == "track":
            for source in edge["source_ids"]:
                source_atoms[source].add(edge["id"])
    terminal_nodes = {node for terminal in graph["terminals"] for node in terminal["nodes"]}
    boundaries = {}
    for node_id in sorted(nodes):
        contacts = [edges[edge_id] for edge_id in sorted(incident[node_id])]
        tracks = [edge for edge in contacts if edge["type"] == "track"]
        reasons = []
        if node_id in terminal_nodes:
            reasons.append("pad_terminal")
        reasons.extend(sorted({edge["type"] for edge in contacts if edge["type"] != "track"}))
        if len(tracks) != 2:
            reasons.append("branch" if len(tracks) > 2 else "track_endpoint")
        elif tracks[0]["width"] != tracks[1]["width"]:
            reasons.append("width_change")
        elif tracks[0]["layer"] != tracks[1]["layer"]:
            reasons.append("layer_change")
        if reasons:
            boundaries[node_id] = reasons

    unvisited = {edge_id for edge_id, edge in edges.items() if edge["type"] == "track"}
    chains = []

    def walk(start, first_edge):
        path_nodes, path_edges = [start], []
        current, edge_id = start, first_edge
        while True:
            edge = edges[edge_id]
            unvisited.remove(edge_id)
            path_edges.append(edge_id)
            current = edge["to"] if edge["from"] == current else edge["from"]
            path_nodes.append(current)
            if current in boundaries or current == start:
                break
            remaining = sorted(set(incident[current]) & unvisited)
            if not remaining:
                break
            edge_id = remaining[0]
        points_nm = [nodes[node]["at_nm"] for node in path_nodes]
        sources = sorted({source for edge_id in path_edges for source in edges[edge_id]["source_ids"]})
        selected = set(path_edges)
        chains.append({
            "id": native_graph.stable_id("chain-", [graph["net"], path_edges]),
            "net": graph["net"], "layer": edges[first_edge]["layer"],
            "closed": path_nodes[0] == path_nodes[-1],
            "node_ids": path_nodes, "points": [native_graph.mm(p) for p in points_nm],
            "points_nm": points_nm, "edge_ids": path_edges,
            "source_track_ids": sources,
            "source_track_coverage": [{"id": source,
                "chain_edge_ids": sorted(source_atoms[source] & selected),
                "total_graph_atomic_edges": len(source_atoms[source]),
                "covers_all_graph_atoms": source_atoms[source] <= selected} for source in sources],
            "widths_mm": sorted({edges[edge_id]["width"] for edge_id in path_edges}),
            "path_length_mm": sum(edges[edge_id]["length_mm"] for edge_id in path_edges),
            "endpoint_distance_mm": math.dist(points_nm[0], points_nm[-1])/native_graph.SCALE,
            "endpoint_boundaries": [boundaries.get(path_nodes[0], []), boundaries.get(path_nodes[-1], [])],
        })

    # Start at barriers first so open chains cannot be split by an arbitrary
    # interior seed. What remains consists solely of closed degree-two cycles.
    for node_id in sorted(boundaries):
        for edge_id in sorted(incident[node_id]):
            if edge_id in unvisited:
                walk(node_id, edge_id)
    while unvisited:
        edge_id = min(unvisited)
        edge = edges[edge_id]
        walk(min(edge["from"], edge["to"]), edge_id)
    chains.sort(key=lambda chain: chain["id"])
    return {"net": graph["net"], "net_name": graph["net_name"],
        "graph_sha256": graph["graph_sha256"], "caveats": list(graph["caveats"]),
        "summary": {"chains": len(chains), "closed_chains": sum(chain["closed"] for chain in chains),
            "atomic_track_edges": sum(len(chain["edge_ids"]) for chain in chains),
            "source_tracks": len(graph["source_track_ids"]),
            "path_length_mm": sum(chain["path_length_mm"] for chain in chains)},
        "chains": chains}


def inspect_chains(board, net=None, *, pad_contacts=False, native_contacts=None):
    if pad_contacts and native_contacts is not None:
        raise native_graph.UnsupportedGeometry("Choose only one pad contact mode")
    if native_contacts is not None:
        from native_contacts import validate_packet
        validate_packet(board, native_contacts)
    nets = [net] if net is not None else sorted({item["net"] for section in ["tracks", "vias", "pads"] for item in board[section]})
    results = []
    for current in nets:
        try:
            results.append(chains_for_graph(native_graph.build_graph(board, current, pad_contacts=pad_contacts,
                                                                     native_contacts=native_contacts)))
        except native_graph.UnsupportedGeometry as error:
            results.append({"net": current, "unsupported": True, "error": str(error)})
    result = {"schema_version": 1, "format": "analysis-lab-native-track-chains",
        "board_id": board["id"], "units": "mm", "native_sha256": board.get("native_sha256"),
        "pad_contacts": pad_contacts, "caveats": CAVEATS, "nets": results}
    if native_contacts is not None:
        result["native_contacts_sha256"] = native_contacts["packet_sha256"]
    return result


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("board", type=Path)
    parser.add_argument("--net")
    modes = parser.add_mutually_exclusive_group()
    modes.add_argument("--pad-contacts", action="store_true")
    modes.add_argument("--native-contacts", type=Path)
    parser.add_argument("--output", type=Path)
    args = parser.parse_args()
    contacts = json.loads(args.native_contacts.read_text()) if args.native_contacts else None
    result = inspect_chains(json.loads(args.board.read_text()), args.net, pad_contacts=args.pad_contacts, native_contacts=contacts)
    if args.output:
        native_graph.write_json(args.output, result)
        print(json.dumps({"output": str(args.output), "nets": len(result["nets"])}))
    else:
        print(json.dumps(result, indent=2))


if __name__ == "__main__":
    main()
