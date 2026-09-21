#!/usr/bin/env python3
# Copyright (C) 2026 Toit contributors.
"""Deterministic, deliberately small routing diagnosis experiments.

Generation is private experiment infrastructure, never part of agent packets.
The filesystem does not enforce blinding: keep agents away from this source and
the private directory. Acceptance uses pcb-maker's existing exact validator.
"""

import argparse
import copy
import json
import math
from pathlib import Path
import subprocess
import tempfile
from inspect_board import resolve_proposal

ROOT = Path(__file__).resolve().parents[2]
DEFAULT_OUTPUT = ROOT / "build/analysis-lab"
DEFAULT_VALIDATOR = ROOT / "target/release/pcb-maker"


def vector(p):
    return {"x": p[0], "y": p[1]}


def write_json(path, value):
    path.parent.mkdir(parents=True, exist_ok=True)
    path.write_text(json.dumps(value, indent=2) + "\n")


def measurements(board):
    length = sum(math.dist(a, b) for r in board["routes"]
                 for a, b in zip(r["points"], r["points"][1:]))
    vias = sum(a != b for r in board["routes"]
               for a, b in zip(r["layers"], r["layers"][1:]))
    return {"length_mm": length, "vias": vias, "cost_mm": length + 2 * vias}


def semantic_problem(board):
    """Translate public physical geometry, without deriving rules from repairs."""
    components = []
    for pad in board["pads"]:
        components.append({
            "id": pad["id"], "position": vector(pad["at"]),
            "size": vector([pad["diameter"], pad["diameter"]]),
            "rotation_degrees": 0,
            "constraints": {"movement": "fixed", "rotation": "fixed"},
            "body_is_routing_keepout": False,
            "pins": [{"id": "1", "offset": vector([0, 0]), "pads": [
                {"id": "pad-" + layer, "layer": layer,
                 "shape": {"kind": "circle", "diameter": pad["diameter"]}}
                for layer in pad["layers"]]}]})
    for obstacle in board["obstacles"]:
        x0, y0, x1, y1 = obstacle["rect"]
        components.append({
            "id": obstacle["id"], "position": vector([(x0+x1)/2, (y0+y1)/2]),
            "size": vector([x1-x0, y1-y0]), "rotation_degrees": 0,
            "constraints": {"movement": "fixed", "rotation": "fixed"},
            "body_is_routing_keepout": False, "pins": [],
            "routing_keepouts": [{"layer": layer, "shape": {
                "kind": "rect", "size": vector([x1-x0, y1-y0])}}
                for layer in obstacle["layers"]]})
    nets = []
    for route in board["routes"]:
        endpoints = []
        for point in [route["points"][0], route["points"][-1]]:
            matches = [p for p in board["pads"]
                       if p["net"] == route["net"] and p["at"] == point]
            if len(matches) != 1:
                raise ValueError("Each route endpoint must match exactly one same-net pad")
            endpoints.append({"component": matches[0]["id"], "pin": "1"})
        nets.append({"id": route["id"], "electrical_net": route["net"],
                     "width": board["rules"]["trace_width"], "layer": "top",
                     "allowed_layers": ["top", "bottom"],
                     "from": endpoints[0], "to": endpoints[1]})
    return {"schema_version": 1, "board": {
        "bounds": {"min": vector(board["bounds"][:2]),
                   "max": vector(board["bounds"][2:])},
        "layers": [{"id": "top"}, {"id": "bottom"}]},
        "rules": {k: v for k, v in board["rules"].items() if k != "trace_width"},
        "components": components, "nets": nets}


def candidate(board, problem):
    graphs = {}
    traces = []
    for route, net in zip(board["routes"], problem["nets"]):
        graph = graphs.setdefault(route["net"], {
            "electrical_net": route["net"], "nodes": [], "branches": []})
        graph["branches"].append(route["id"])
        terminal_ids = []
        for end, point in zip([net["from"], net["to"]],
                              [route["points"][0], route["points"][-1]]):
            node_id = f"terminal:{route['net']}/{end['component']}.1"
            terminal_ids.append(node_id)
            existing = next((n for n in graph["nodes"] if n["id"] == node_id), None)
            if existing:
                existing["incident_branches"].append(route["id"])
            else:
                graph["nodes"].append({"id": node_id, "electrical_net": route["net"],
                    "position": vector(point), "incident_branches": [route["id"]],
                    "kind": "terminal", "component": end["component"], "pin": "1"})
        vias = []
        for i in range(1, len(route["layers"])):
            before, after = route["layers"][i-1:i+1]
            if before == after:
                continue
            via = {"position": vector(route["points"][i]), "from_layer": before,
                   "to_layer": after, "diameter": board["rules"]["via_diameter"],
                   "drill": board["rules"]["via_drill"], "point_index": i}
            vias.append(via)
            graph["nodes"].append({"id": f"via:{route['id']}:{i}",
                "electrical_net": route["net"], "incident_branches": [route["id"]],
                "kind": "via", **via})
        traces.append({"branch": route["id"], "electrical_net": route["net"],
            "net": route["id"], "from_node": terminal_ids[0], "to_node": terminal_ids[1],
            "width": board["rules"]["trace_width"], "tension_weight": 1,
            "layer": "top", "segment_layers": route["layers"], "vias": vias,
            "points": [vector(p) for p in route["points"]],
            "route_class": {"layer": "top", "homotopy": [],
                "from": {"component": net["from"]["component"], "sector": "1"},
                "to": {"component": net["to"]["component"], "sector": "1"}},
            "route_basis_fingerprint": None})
    return {"schema_version": 1, "components": [
        {k: c[k] for k in ["id", "position", "size", "rotation_degrees"]}
        for c in problem["components"]], "traces": traces,
        "route_graphs": list(graphs.values())}


def exact_validate(board, problem, validator=DEFAULT_VALIDATOR):
    with tempfile.TemporaryDirectory(prefix="analysis-lab-") as temp:
        temp = Path(temp)
        write_json(temp / "problem.json", problem)
        write_json(temp / "candidate.json", candidate(board, problem))
        result = subprocess.run([str(validator), "validate", str(temp / "problem.json"),
                                 str(temp / "candidate.json")], capture_output=True, text=True)
        return {"valid": result.returncode == 0,
                "validator_stdout": result.stdout.strip(),
                "validator_stderr": result.stderr.strip()}


def apply_proposal(board, proposal):
    proposal = resolve_proposal(board, proposal)
    updated = copy.deepcopy(board)
    routes = {r["id"]: r for r in updated["routes"]}
    seen = set()
    for replacement in proposal.get("replacements", []):
        route_id = replacement["route_id"]
        if route_id not in routes or route_id in seen:
            raise ValueError("Unknown or duplicate route_id: " + route_id)
        seen.add(route_id)
        points, layers = replacement["points"], replacement["layers"]
        if len(points) < 2 or len(layers) != len(points) - 1:
            raise ValueError("Need at least two points and one layer per segment")
        if any(len(p) != 2 or any(not isinstance(v, (int, float)) or
               not math.isfinite(v) for v in p) for p in points):
            raise ValueError("Points must be finite numeric coordinate pairs")
        if any(layer not in ["top", "bottom"] for layer in layers):
            raise ValueError("Unknown copper layer")
        if points[0] != routes[route_id]["points"][0] or points[-1] != routes[route_id]["points"][-1]:
            raise ValueError("Terminal endpoints are fixed")
        if any(a == b for a, b in zip(points, points[1:])):
            raise ValueError("Zero-length segments are not allowed")
        routes[route_id].update(points=points, layers=layers)
    return updated


def evaluate(board, proposal, validator=DEFAULT_VALIDATOR):
    problem = semantic_problem(board)
    baseline = exact_validate(board, problem, validator)
    if not baseline["valid"]:
        raise ValueError("Invalid baseline: " + json.dumps(baseline))
    try:
        updated = apply_proposal(board, proposal)
    except (KeyError, TypeError, ValueError) as error:
        return {"valid": False, "improved": False, "error": str(error),
                "original": measurements(board)}
    validation = exact_validate(updated, problem, validator)
    old, new = measurements(board), measurements(updated)
    return {**validation, "original": old, "new": new,
            "improved": validation["valid"] and new["cost_mm"] < old["cost_mm"] - 1e-7,
            "cost_reduction_mm": old["cost_mm"] - new["cost_mm"]}


def board(case_id, points, layers, obstacles=None):
    return {"schema_version": 1, "id": case_id, "bounds": [0, 0, 40, 30],
        "rules": {"clearance": .25, "trace_width": .4, "via_diameter": .8, "via_drill": .4},
        "pads": [{"id": "P1", "net": "N1", "at": points[0], "layers": ["top"], "diameter": 1},
                 {"id": "P2", "net": "N1", "at": points[-1], "layers": ["top"], "diameter": 1}],
        "obstacles": obstacles or [],
        "routes": [{"id": "R1", "net": "N1", "points": points, "layers": layers}]}


def replacement(points, layers=None, route_id="R1"):
    return {"replacements": [{"route_id": route_id, "points": points,
                               "layers": layers or ["top"] * (len(points) - 1)}]}


def fixtures():
    # IDs and order intentionally do not expose categories to agents.
    straight = [[5, 15], [35, 15]]
    excursion = [[5, 15], [12, 15], [28, 15], [35, 15]]
    cases = [
        (board("b07", excursion, ["top", "bottom", "top"]),
         "redundant_excursion", replacement(straight)),
        (board("b12", [[5, 15], [10, 15], [10, 23], [30, 23], [30, 15], [35, 15]], ["top"] * 5),
         "detour", replacement(straight)),
        (board("b19", excursion, ["top", "bottom", "top"],
               [{"id": "O1", "rect": [18, 0, 22, 30], "layers": ["top"]}]),
         "necessary_excursion", {"replacements": []}),
        (board("b23", straight, ["top"]), "straight_control", {"replacements": []}),
        (board("b31", excursion, ["top", "bottom", "top"],
               [{"id": "O1", "rect": [18, 14, 22, 16], "layers": ["top"]}]),
         "dogleg_repair", replacement([[5, 15], [17.5, 16.5], [22.5, 16.5], [35, 15]])),
    ]
    dense = board("b44", excursion, ["top", "bottom", "top"],
                  [{"id": "O1", "rect": [18, 20, 22, 23], "layers": ["top"]},
                   {"id": "O2", "rect": [18, 6, 22, 9], "layers": ["bottom"]}])
    for i, (y, layer) in enumerate([(3, "top"), (6, "top"), (9, "top"), (12, "bottom"),
                                   (18, "bottom"), (24, "top"), (27, "bottom")], 2):
        first, last = [3, y], [37, y]
        net = f"N{i}"
        dense["pads"].extend([
            {"id": f"P{2*i-1}", "net": net, "at": first, "layers": [layer], "diameter": 1},
            {"id": f"P{2*i}", "net": net, "at": last, "layers": [layer], "diameter": 1}])
        dense["routes"].append({"id": f"R{i}", "net": net, "points": [first, last], "layers": [layer]})
    cases.append((dense, "clutter_excursion", replacement(straight)))
    return cases


def generate(output=DEFAULT_OUTPUT, validator=DEFAULT_VALIDATOR):
    manifest = {"schema_version": 1, "cases": [],
        "blinding": "Private by protocol only; shared filesystem does not enforce isolation.",
        "validator": str(validator), "objective": "length_mm + 2 * via_count",
        "limitations": "Fixed point terminals, circular pads, rectangular keepouts, two layers; semantic exact validation, not native KiCad DRC."}
    for item, category, repair in fixtures():
        problem = semantic_problem(item)
        assessment = evaluate(item, repair, validator)
        if not assessment["valid"] or (repair["replacements"] and not assessment["improved"]):
            raise ValueError(f"Fixture {item['id']} failed: {assessment}")
        private = output / "private" / item["id"]
        write_json(output / "public" / (item["id"] + ".json"), item)
        write_json(private / "problem.json", problem)
        write_json(private / "candidate.json", candidate(item, problem))
        write_json(private / "answer.json", {"category": category, "proposal": repair,
                                              "assessment": assessment})
        manifest["cases"].append({"id": item["id"], "category": category, "assessment": assessment})
    write_json(output / "private/manifest.json", manifest)
    write_json(output / "public/objective.json", {
        "objective": "Minimize total trace centerline length in mm + 2 mm per via.",
        "constraints": "Preserve every connection, route endpoint, pad, obstacle, board bounds, layer and clearance rule. Only route geometry/layer replacements are allowed. Arbitrary straight-segment angles are allowed. Some boards may have no improvement.",
        "proposal_format": {"replacements": [{"route_id": "R1", "points": [[0, 0], [1, 1]], "layers": ["top"]}]},
        "units": "mm", "clearance": "Copper edge to unrelated copper/obstacle edge; actual widths and via diameters apply.",
        "layer_transition": "A via is inserted at each internal point where incident segment layers differ. Endpoint vias are not part of this benchmark."})
    return {"generated": len(manifest["cases"]), "public": str(output / "public"),
            "private": str(output / "private")}


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--validator", type=Path, default=DEFAULT_VALIDATOR)
    sub = parser.add_subparsers(dest="command", required=True)
    make = sub.add_parser("generate")
    make.add_argument("--output", type=Path, default=DEFAULT_OUTPUT)
    check = sub.add_parser("evaluate")
    check.add_argument("board", type=Path)
    check.add_argument("proposal", type=Path)
    args = parser.parse_args()
    if args.command == "generate":
        result = generate(args.output, args.validator)
    else:
        result = evaluate(json.loads(args.board.read_text()),
                          json.loads(args.proposal.read_text()), args.validator)
    print(json.dumps(result, indent=2))


if __name__ == "__main__":
    main()
