#!/usr/bin/env python3
# Copyright (C) 2026 Toit contributors.
"""Private synthetic geometry-change fixtures, complementary to cycle tests."""

import argparse
import copy
import math
from pathlib import Path
import random

import generate as lab

OUTPUT = lab.DEFAULT_OUTPUT / "relocation"


def obstacle(object_id, rect, layer):
    return {"id": object_id, "rect": rect, "layers": [layer]}


def barriers():
    return [obstacle("O1", [18, 0, 22, 17], "top"),
            obstacle("O2", [18, 19, 22, 30], "bottom")]


def single(case_id, points, layers):
    item = lab.board(case_id, points, layers, barriers())
    item["pads"][1]["layers"] = ["bottom"]
    return item


def anonymize(item, proposal, negative_proposals, seed):
    """Stable neutral object IDs/order; no category labels enter public files."""
    rng = random.Random(seed)
    route_ids = {r["id"]: f"R{n}" for r, n in zip(item["routes"], rng.sample(range(10, 99), len(item["routes"])))}
    nets = sorted({r["net"] for r in item["routes"]})
    net_ids = {net: f"N{n}" for net, n in zip(nets, rng.sample(range(10, 99), len(nets)))}
    for route in item["routes"]:
        route["id"] = route_ids[route["id"]]
        route["net"] = net_ids[route["net"]]
    for pad, number in zip(item["pads"], rng.sample(range(10, 99), len(item["pads"]))):
        pad["id"] = f"P{number}"
        pad["net"] = net_ids[pad["net"]]
    for keepout, number in zip(item["obstacles"], rng.sample(range(10, 99), len(item["obstacles"]))):
        keepout["id"] = f"O{number}"
    for action in [proposal, *negative_proposals]:
        for replacement in action.get("replacements", []):
            replacement["route_id"] = route_ids[replacement["route_id"]]
    for key in ["routes", "pads", "obstacles"]:
        rng.shuffle(item[key])
    return item, proposal, negative_proposals


def transform(item, proposals, fn):
    """Rigid coordinate variants avoid showing an identical witness as a control."""
    for route in item["routes"]:
        route["points"] = [fn(p) for p in route["points"]]
    for pad in item["pads"]:
        pad["at"] = fn(pad["at"])
    def rectangle(rect):
        a, b = fn(rect[:2]), fn(rect[2:])
        return [min(a[0], b[0]), min(a[1], b[1]), max(a[0], b[0]), max(a[1], b[1])]
    for keepout in item["obstacles"]:
        keepout["rect"] = rectangle(keepout["rect"])
    item["bounds"] = rectangle(item["bounds"])
    for proposal in proposals:
        for replacement in proposal.get("replacements", []):
            replacement["points"] = [fn(p) for p in replacement["points"]]


def fixtures():
    straight = [[5, 8], [12.5, 11.5], [35, 22]]
    repair = lab.replacement(straight, ["top", "bottom"])
    cases = [
        (single("s11", [[5, 8], [10, 24], [15, 17], [25, 17], [35, 22]],
                ["top", "bottom", "bottom", "bottom"]),
         "necessary_via_relocation", copy.deepcopy(repair), {
            "intent": "One necessary via must move; the old bottom branch also changes. Existing connectivity graph is an acyclic chain, so choosing a subset of graph edges cannot express this repair.",
            "proof": "Start pad is top-only, finish pad bottom-only, so at least one layer transition remains necessary. Known repair lies on the endpoint straight line and reaches Euclidean distance plus one 2mm via: sqrt(30^2+14^2)+2.",
            "barrier_roles": "Top barrier blocks later transitions along the straight line; bottom barrier rejects the old upper crossing corridor. Witness transitions before the top barrier and crosses below the bottom barrier.",
            "negative_proposals": [lab.replacement([[5, 8], [35, 22]], ["top"]),
                lab.replacement([[5, 8], [10, 24], [35, 22]], ["top", "bottom"])]}),
        (single("s28", straight, ["top", "bottom"]),
         "necessary_via_control", {"replacements": []}, {
            "intent": "Unchanged geometric lookalike with a necessary single via; no legal improvement under the declared objective.",
            "proof": "Every connected route length is at least Euclidean endpoint distance sqrt(1096). Top-only and bottom-only endpoint pads require at least one internal via. Current trace lies exactly on the straight line and has exactly one via, attaining the lower bound sqrt(1096)+2.",
            "negative_proposals": [lab.replacement([[5, 8], [35, 22]], ["bottom"])]}),
        (single("s43", [[5, 8], [12.5, 11.5], [15, 17], [25, 17], [35, 22]],
                ["top", "bottom", "bottom", "bottom"]),
         "branch_path_replacement", copy.deepcopy(repair), {
            "intent": "Via location and count are already suitable, but the bottom branch takes an unnecessary bend. New geometric edges are required; graph-cycle deletion alone cannot shorten an acyclic chain.",
            "proof": "The known replacement keeps the same necessary via and reaches the straight-line plus one-via lower bound. It crosses the bottom barrier's x-range below its lower edge with exact clearance.",
            "negative_proposals": [lab.replacement([[5, 8], [35, 22]], ["top"])]}),
    ]
    pair = lab.board("s76", [[5, 10], [20, 20], [35, 10]], ["top", "bottom"])
    pair["pads"][1]["layers"] = ["bottom"]
    pair["pads"].extend([
        {"id": "P3", "net": "N2", "at": [5, 20], "layers": ["bottom"], "diameter": 1},
        {"id": "P4", "net": "N2", "at": [35, 20], "layers": ["top"], "diameter": 1}])
    pair["routes"].append({"id": "R2", "net": "N2", "points": [[5, 20], [20, 10], [35, 20]],
                           "layers": ["bottom", "top"]})
    first = lab.replacement([[5, 10], [20, 10], [35, 10]], ["top", "bottom"])
    second = lab.replacement([[5, 20], [20, 20], [35, 20]], ["bottom", "top"], "R2")
    joint = {"replacements": first["replacements"] + second["replacements"]}
    cases.append((pair, "coordinated_via_relocations", joint, {
        "intent": "The known straightening action moves both via positions together. Either constituent action alone collides with the other net's old via/path; the simultaneous action is valid.",
        "proof": "Opposite-layer endpoints require at least one via per net, and endpoint distances are each30mm. Joint witness has two30mm straight traces and two vias, attaining global lower bound64mm.",
        "restriction": "Only the specified full-benefit witness is proven to require both edits. Smaller valid single-route improvements may exist; they must earn credit if independently validated. No universal no-single-edit claim is made.",
        "negative_proposals": [first, second]}))
    for index, (item, category, proposal, notes) in enumerate(cases):
        negatives = copy.deepcopy(notes.pop("negative_proposals", []))
        item, proposal, negatives = anonymize(copy.deepcopy(item), copy.deepcopy(proposal), negatives, 921731+index)
        if index == 1:
            transform(item, [proposal, *negatives], lambda p: [40-p[0], 30-p[1]])
            notes["coordinate_variant"] = "180-degree rigid rotation; lower bound unchanged."
        elif index == 2:
            transform(item, [proposal, *negatives], lambda p: [round(30-p[1]+.137, 6), round(p[0]+.271, 6)])
            notes["coordinate_variant"] = "90-degree rigid rotation and fractional translation; lower bound unchanged."
        yield item, category, proposal, notes, negatives


def generate(output=OUTPUT, validator=lab.DEFAULT_VALIDATOR):
    manifest = {"schema_version": 1, "cases": [], "objective": "length_mm + 2*via_count",
        "intent": "Fresh acyclic route geometries needing relocation or replacement, not only removal of graph cycles.",
        "blinding": "Private by protocol only. Do not expose generator, witnesses, classifications, or parent experiment discussions to blind solvers.",
        "limits": "Synthetic two-layer centerline chains with circular endpoint pads and rectangular keepouts; exact semantic validation, not native KiCad DRC."}
    for item, category, proposal, notes, negatives in fixtures():
        result = lab.evaluate(item, proposal, validator)
        if not result["valid"] or (proposal["replacements"] and not result["improved"]):
            raise ValueError(f"Invalid relocation fixture/witness {item['id']}: {result}")
        negative_results = []
        for negative in negatives:
            assessment = lab.evaluate(item, negative, validator)
            if assessment["valid"]:
                raise ValueError(f"Negative witness unexpectedly valid for {item['id']}: {assessment}")
            negative_results.append({"proposal": negative, "assessment": assessment})
        lower_bound = 64.0 if len(item["routes"]) == 2 else math.sqrt(1096)+2
        if not math.isclose(result["new"]["cost_mm"], lower_bound, abs_tol=1e-9):
            raise ValueError(f"Witness did not attain recorded lower bound for {item['id']}")
        private = output / "private" / item["id"]
        problem = lab.semantic_problem(item)
        lab.write_json(output / "public" / f"{item['id']}.json", item)
        lab.write_json(private / "problem.json", problem)
        lab.write_json(private / "candidate.json", lab.candidate(item, problem))
        lab.write_json(private / "answer.json", {"category": category, "proposal": proposal,
            "assessment": result, "lower_bound_cost_mm": lower_bound,
            "notes": notes, "negative_witnesses": negative_results})
        manifest["cases"].append({"id": item["id"], "category": category, "assessment": result})
    lab.write_json(output / "private/manifest.json", manifest)
    lab.write_json(output / "public/objective.json", {
        "objective": "Minimize total trace centerline length_mm + 2mm per via.",
        "constraints": "Preserve every connection, exact endpoint, pad, obstacle, layer availability, width, clearance rule, and board bounds. Only route geometry/layer replacements allowed. Endpoint pads conduct only on their stated layers. Arbitrary straight-segment angles allowed. Some boards may have no improvement.",
        "transaction": "Multiple replacements are applied together and validated as one candidate.",
        "proposal_format": {"replacements": [{"route_id": "R42", "points": [[0, 0], [1, 1]], "layers": ["top"]}]},
        "units": "mm", "layer_transition": "Internal changes between successive segment layers insert vias. Endpoint vias are outside this benchmark.",
        "clearance": "Edge-to-edge copper and obstacle clearance, accounting for actual trace width and via diameter."})
    return {"generated": len(manifest["cases"]), "public": str(output / "public"), "private": str(output / "private")}


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--output", type=Path, default=OUTPUT)
    parser.add_argument("--validator", type=Path, default=lab.DEFAULT_VALIDATOR)
    args = parser.parse_args()
    print(generate(args.output, args.validator))


if __name__ == "__main__":
    main()
