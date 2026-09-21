#!/usr/bin/env python3
# Copyright (C) 2026 Toit contributors.
"""Private construction of harder analysis-lab fixtures; do not expose to solvers."""

import argparse
import copy
import random
from pathlib import Path

import generate as lab

OUTPUT = lab.DEFAULT_OUTPUT / "harder"


def obstacle(object_id, rect, layers=("top",)):
    return {"id": object_id, "rect": rect, "layers": list(layers)}


def add_route(board, route_id, net, points, layers, pad_ids, pad_layers):
    for point, pad_id, layer in zip([points[0], points[-1]], pad_ids, pad_layers):
        board["pads"].append({"id": pad_id, "net": net, "at": point,
                              "layers": [layer], "diameter": 1})
    board["routes"].append({"id": route_id, "net": net, "points": points, "layers": layers})


def fixtures():
    rotated = lab.board("h04", [[20.137, 3.271], [20.137, 9.271],
                                [20.137, 21.271], [20.137, 27.271]],
                        ["top", "bottom", "top"])
    rotated["obstacles"] = [obstacle("O1", [10.183, 8.247, 14.183, 20.247]),
                             obstacle("O2", [26.249, 10.137, 28.249, 24.137], ["bottom"])]
    yield rotated, "fractional_rotated_excursion", lab.replacement(
        [[20.137, 3.271], [20.137, 27.271]]), {
        "proof": "Straight repair reaches Euclidean distance lower bound with no vias.",
        "challenge": "Exact terminal coordinates have three decimal places; vertical orientation."}

    narrow = lab.board("h17", [[5, 15], [12, 15], [28, 15], [35, 15]],
                       ["top", "bottom", "top"], [
                           obstacle("O1", [18, 0, 22, 15.133]),
                           obstacle("O2", [18, 16.093, 22, 30])])
    yield narrow, "narrow_passage", lab.replacement(
        [[5, 15], [17.5, 15.613], [22.5, 15.613], [35, 15]]), {
        "proof": "Top centerline interval in x=18..22 is y=[15.583,15.643], width 0.060mm. Repair uses midpoint.",
        "challenge": "Layer removal must dogleg through a gap where rounding to 0.1mm can fail.",
        "negative_proposals": [lab.replacement([[5, 15], [35, 15]]),
            lab.replacement([[5, 15], [17.5, 15.65], [22.5, 15.65], [35, 15]])]}

    control = lab.board("h29", [[5, 15], [12, 15], [28, 15], [35, 15]],
                        ["top", "bottom", "top"], [
                            obstacle("O1", [18, 2, 22, 15.203]),
                            obstacle("O2", [18, 16.023, 22, 28])])
    yield control, "closed_passage_control", {"replacements": []}, {
        "proof": "Gap is 0.820mm < trace_width + 2*clearance = 0.900mm. Every top path must bypass y<1.55 or y>28.45. Even ignoring rectangle width, such a path has length at least 2*sqrt(15^2+13.45^2)=40.29mm > current cost34mm. Any route retaining vias has at least two and length>=30mm.",
        "challenge": "Nearly identical visible gap to h17, but no beneficial change exists under fixed endpoints and objective.",
        "negative_proposals": [lab.replacement([[5, 15], [17.5, 15.613], [22.5, 15.613], [35, 15]])],
        "valid_worse_proposal": lab.replacement([[5, 15], [17.5, 1.5], [22.5, 1.5], [35, 15]])}

    via = lab.board("h38", [[5, 14.3], [10, 12], [30, 12], [35, 14.3]], ["top"] * 3)
    add_route(via, "R2", "N2", [[20, 3], [20, 15], [20, 17], [20, 27]],
              ["bottom", "top", "bottom"], ["P3", "P4"], ["bottom", "bottom"])
    yield via, "foreign_via_clearance", lab.replacement(
        [[5, 14.3], [19, 14.1], [21, 14.1], [35, 14.3]]), {
        "proof": "Direct y14.3 line is 0.7mm from top foreign segment (>=0.65mm needed), but only 0.7mm from via center (<0.85mm needed). Dogleg y14.1 clears via. Foreign bottom copper may cross top route.",
        "challenge": "Must include vias in local obstacles, despite foreign straight segment at crossing being on bottom.",
        "negative_proposals": [lab.replacement([[5, 14.3], [35, 14.3]])],
        "restrictions": "No claim that known repair is globally optimal or unique; any exact valid improvement counts."}

    paired = lab.board("h46", [[1, 15], [3, 15], [37, 15], [39, 15]],
                       ["top", "bottom", "top"])
    add_route(paired, "R2", "N2", [[20, 1], [20, 3], [20, 27], [20, 29]],
              ["bottom", "top", "bottom"], ["P3", "P4"], ["bottom", "bottom"])
    one = lab.replacement([[1, 15], [39, 15]])
    two = lab.replacement([[20, 1], [20, 29]], ["bottom"], "R2")
    yield paired, "interacting_excursions", {
        "replacements": one["replacements"] + two["replacements"]}, {
        "proof": "Both routes already have Euclidean-minimum length. Removing only R1's two vias makes it cross R2's top segment; removing only R2's vias makes it cross R1's bottom segment. Any single-route planar bypass is > its current length+4mm (crossing wall extends almost across board). Both removals together leave perpendicular routes on distinct layers: 0 vias and length66mm versus cost74mm.",
        "challenge": "A tool testing only independent single-route shortcuts reports both as invalid, so coordinated hypotheses matter.",
        "negative_proposals": [one, two]}

    dense = {"schema_version": 1, "id": "h63", "bounds": [0, 0, 80, 102],
             "rules": copy.deepcopy(paired["rules"]), "pads": [], "routes": [], "obstacles": []}
    rng = random.Random(7741903)
    route_numbers = rng.sample(range(10, 98), 24)
    net_numbers = rng.sample(range(101, 199), 24)
    target_row = 13
    for i in range(24):
        y = 3 + 4*i
        rid, net = f"R{route_numbers[i]}", f"N{net_numbers[i]}"
        add_route(dense, rid, net, [[5, y], [30, y], [50, y], [75, y]],
                  ["top", "bottom", "top"], [f"P{2*i+1}", f"P{2*i+2}"], ["top", "top"])
        extent = .2 if i == target_row else 1.1
        dense["obstacles"].append(obstacle(f"O{i+1}", [38, y-extent, 42, y+extent]))
    dense["obstacles"].append(obstacle("O101", [0, 0, 80, 1.7], ["top", "bottom"]))
    for i in range(23):
        y = 3 + 4*i
        dense["obstacles"].append(obstacle(f"O{102+i}", [0, y+1.3, 80, y+2.7], ["top", "bottom"]))
    dense["obstacles"].append(obstacle("O125", [0, 96.3, 80, 102], ["top", "bottom"]))
    rng.shuffle(dense["routes"])
    rng.shuffle(dense["pads"])
    rng.shuffle(dense["obstacles"])
    target_y = 3 + 4*target_row
    target_id = f"R{route_numbers[target_row]}"
    yield dense, "dense_compartmentalized_excursions", lab.replacement(
        [[5, target_y], [37.5, target_y+.7], [42.5, target_y+.7], [75, target_y]],
        route_id=target_id), {
        "proof": "24 independently compartmentalized nets. Horizontal all-layer barriers prevent changing rows. 23 top walls leave only 0.2mm gaps to barriers, too small for width+clearance; their two vias are necessary. One shorter top wall leaves 1.1mm passage and admits a via-free dogleg.",
        "challenge": "24 similar route chains, shuffled order and IDs; identify one viable local change while preserving all others.",
        "target_route": target_id,
        "negative_proposals": [lab.replacement([[5, target_y], [75, target_y]], route_id=target_id)]}


def generate(output=OUTPUT, validator=lab.DEFAULT_VALIDATOR):
    manifest = {"schema_version": 1, "cases": [],
                "objective": "length_mm + 2 * via_count",
                "blinding": "Private by protocol only. Do not expose generator or private files to solvers.",
                "restrictions": "Fixed endpoints/components; two layers, circular pads, rectangular keepouts; no endpoint vias. Exact semantic validator is authority, not native KiCad DRC."}
    for board, category, proposal, notes in fixtures():
        assessment = lab.evaluate(board, proposal, validator)
        if not assessment["valid"] or (proposal["replacements"] and not assessment["improved"]):
            raise ValueError(f"{board['id']}: {assessment}")
        negative_results = []
        for negative in notes.get("negative_proposals", []):
            result = lab.evaluate(board, negative, validator)
            if result["valid"]:
                raise ValueError(f"Expected invalid negative for {board['id']}: {result}")
            negative_results.append(result)
        if "valid_worse_proposal" in notes:
            result = lab.evaluate(board, notes["valid_worse_proposal"], validator)
            if not result["valid"] or result["improved"]:
                raise ValueError(f"Expected valid worse bypass for {board['id']}: {result}")
            notes["valid_worse_assessment"] = result
        problem = lab.semantic_problem(board)
        private = output / "private" / board["id"]
        lab.write_json(output / "public" / f"{board['id']}.json", board)
        lab.write_json(private / "problem.json", problem)
        lab.write_json(private / "candidate.json", lab.candidate(board, problem))
        lab.write_json(private / "answer.json", {"category": category, "proposal": proposal,
            "assessment": assessment, "notes": notes, "negative_results": negative_results})
        manifest["cases"].append({"id": board["id"], "category": category,
                                   "assessment": assessment})
        print(f"Validated {board['id']}; public fixture ready", flush=True)
    lab.write_json(output / "private/manifest.json", manifest)
    lab.write_json(output / "public/objective.json", {
        "objective": "Minimize total trace centerline length in mm + 2 mm per via.",
        "constraints": "Preserve every connection, exact route endpoint, pad, obstacle, board bounds, layer and clearance rule. Only route geometry/layer replacements allowed. Arbitrary straight-segment angles allowed. Some boards may have no improvement. Multiple route replacements are validated as one simultaneous action.",
        "proposal_format": {"replacements": [{"route_id": "R1", "points": [[0, 0], [1, 1]], "layers": ["top"]}]},
        "units": "mm", "clearance": "Copper edge to unrelated copper/obstacle edge; actual widths and via diameters apply.",
        "layer_transition": "A via is inserted at each internal point where incident segment layers differ. Endpoint vias are not part of this benchmark."})
    return manifest


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--output", type=Path, default=OUTPUT)
    parser.add_argument("--validator", type=Path, default=lab.DEFAULT_VALIDATOR)
    args = parser.parse_args()
    result = generate(args.output, args.validator)
    print(f"Generated {len(result['cases'])} exact-validated harder fixtures under {args.output}")


if __name__ == "__main__":
    main()
