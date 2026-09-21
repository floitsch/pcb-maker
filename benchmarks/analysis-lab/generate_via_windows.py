#!/usr/bin/env python3
# Copyright (C) 2026 Toit contributors.
"""Private construction of fixtures separating trace and through-via clearance."""

import argparse
import copy
import math
from pathlib import Path
import random

import generate as lab

OUTPUT = lab.DEFAULT_OUTPUT / "via-windows"


def walls(height=30, narrow=True):
    if narrow:
        return [{"id": "O1", "rect": [18, 0, 19.453, height], "layers": ["bottom"]},
                {"id": "O2", "rect": [20.813, 0, 22, height], "layers": ["top"]}]
    return [{"id": "O1", "rect": [18, 0, 19.45, 23], "layers": ["bottom"]},
            {"id": "O2", "rect": [20.65, 7, 22, 30], "layers": ["top"]}]


def base(case_id, via_at, y=15, height=30, narrow=True):
    item = lab.board(case_id, [[5, y], via_at, [35, y]], ["top", "bottom"], walls(height, narrow))
    item["bounds"] = [0, 0, 40, height]
    item["pads"][1]["layers"] = ["bottom"]
    return item


def transform(item, proposals, fn):
    for route in item["routes"]:
        route["points"] = [fn(p) for p in route["points"]]
    for pad in item["pads"]:
        pad["at"] = fn(pad["at"])
    def rect(values):
        a, b = fn(values[:2]), fn(values[2:])
        return [min(a[0], b[0]), min(a[1], b[1]), max(a[0], b[0]), max(a[1], b[1])]
    item["bounds"] = rect(item["bounds"])
    for keepout in item["obstacles"]:
        keepout["rect"] = rect(keepout["rect"])
    for proposal in proposals:
        for replacement in proposal["replacements"]:
            replacement["points"] = [fn(p) for p in replacement["points"]]


def anonymize(item, proposals, seed):
    rng = random.Random(seed)
    route_ids = {r["id"]: f"R{number}" for r, number in zip(item["routes"], rng.sample(range(100, 999), len(item["routes"])))}
    nets = sorted({route["net"] for route in item["routes"]})
    net_ids = {net: f"N{number}" for net, number in zip(nets, rng.sample(range(100, 999), len(nets)))}
    for route in item["routes"]:
        route["id"], route["net"] = route_ids[route["id"]], net_ids[route["net"]]
    for pad, number in zip(item["pads"], rng.sample(range(100, 999), len(item["pads"]))):
        pad["id"], pad["net"] = f"P{number}", net_ids[pad["net"]]
    for keepout, number in zip(item["obstacles"], rng.sample(range(100, 999), len(item["obstacles"]))):
        keepout["id"] = f"O{number}"
    for proposal in proposals:
        for replacement in proposal["replacements"]:
            replacement["route_id"] = route_ids[replacement["route_id"]]
    for key in ["routes", "pads", "obstacles"]:
        rng.shuffle(item[key])


def fixtures():
    narrow = base("v09", [20.133, 25])
    narrow_repair = lab.replacement([[5, 15], [20.133, 15], [35, 15]], ["top", "bottom"])
    narrow_negative = lab.replacement([[5, 15], [20, 15], [35, 15]], ["top", "bottom"])
    cases = [(narrow, narrow_repair, narrow_negative, {
        "category": "narrow_through_via_window",
        "intent": "Moving the necessary via onto the endpoint line is beneficial, but it must fit a 0.060mm-wide x interval. Planar clearance alone admits incorrect positions.",
        "proof": "Before rigid transforms: bottom barrier ends at x19.453, top barrier starts at x20.813. Via radius0.4 plus clearance0.25 gives allowable center x=[20.103,20.163], width0.060mm. The two planar traces need only radius0.2+clearance0.25; their overlapping allowable interval is x=[19.903,20.363].",
        "negative_geometry": "Via x20 is0.547mm from the bottom-layer barrier: enough for a trace centerline (0.45mm), insufficient for via centerline (0.65mm). Both planar segments are otherwise legal.",
        "known_cost_lower_bound": 32.0,
        "lower_bound_proof": "Top-only and bottom-only endpoint pads require at least one via. Endpoint distance30mm plus2mm per via gives lower bound32mm, attained by the witness."})]

    blocked = base("v26", [20.2, 5], narrow=False)
    blocked_repair = lab.replacement([[5, 15], [19.95, 23.5], [35, 15]], ["top", "bottom"])
    blocked_negative = lab.replacement([[5, 15], [20.05, 15], [35, 15]], ["top", "bottom"])
    cases.append((blocked, blocked_repair, blocked_negative, {
        "category": "planar_overlap_without_through_via_window",
        "intent": "Tempting planar straightening passes through a region where no through-via fits. A beneficial repair moves the transition around a barrier end and changes both branch geometries.",
        "proof": "Before rotation: at shortcut y15, bottom barrier ends x19.45 and top barrier starts x20.65. Physical gap1.20mm is enough for0.90mm trace+clearances, but less than1.30mm via+clearances. Via x20.05 is0.60mm from both barriers, below0.65mm required. Known via(19.95,23.5) clears top barrier by0.70mm and bottom barrier corner by sqrt(0.5^2+0.5^2)=0.7071mm.",
        "negative_geometry": "Both negative planar segments maintain at least0.60mm centerline clearance to their layer's wall, exceeding0.45mm required; the via fails on both layers.",
        "restriction": "The supplied repair is a validated improvement, not a global-optimality claim. Other beneficial via positions or routes must receive credit."}))

    dense = base("v58", [20.133, 33], y=30, height=60)
    for index, y in enumerate([4, 8, 12, 16, 20, 24, 36, 40, 44, 48, 52, 56], 2):
        net = f"N{index}"
        dense["pads"].extend([
            {"id": f"P{2*index-1}", "net": net, "at": [5, y], "layers": ["top"], "diameter": 1},
            {"id": f"P{2*index}", "net": net, "at": [35, y], "layers": ["bottom"], "diameter": 1}])
        dense["routes"].append({"id": f"R{index}", "net": net,
            "points": [[5, y], [20.133, y], [35, y]], "layers": ["top", "bottom"]})
    dense_repair = lab.replacement([[5, 30], [20.133, 30], [35, 30]], ["top", "bottom"])
    dense_negative = lab.replacement([[5, 30], [20, 30], [35, 30]], ["top", "bottom"])
    cases.append((dense, dense_repair, dense_negative, {
        "category": "localized_narrow_window_among_necessary_vias",
        "intent": "One slightly bent route among12 unrelated straight optimal routes. All13 routes need a via and share a narrow permissible transition strip; deleting vias/cycles cannot solve the geometric opportunity.",
        "proof": "Before rotation/translation, all through-via centers must lie in x=[20.103,20.163]. After the90-degree transform this becomes y=[20.322,20.382]. Known transition y20.352 is centered with0.030mm surplus on each side.",
        "negative_geometry": "The negative relocation is planar-safe but leaves via center0.547mm from the opposite-layer barrier, below0.65mm required.",
        "known_cost_lower_bound": 13*32.0,
        "lower_bound_proof": "Each of13 distinct nets has30mm endpoint distance and top-only/bottom-only pads requiring at least one via. Per-net lower bounds sum to416mm and are attained simultaneously by the witness."}))

    for index, (item, repair, negative, notes) in enumerate(cases):
        if index == 1:
            transform(item, [repair, negative], lambda p: [40-p[0], 30-p[1]])
            notes["coordinate_variant"] = "180-degree rigid rotation; proof coordinates above refer to the pre-transform construction."
        elif index == 2:
            transform(item, [repair, negative], lambda p: [round(60-p[1]+.173, 6), round(p[0]+.219, 6)])
            notes["coordinate_variant"] = "90-degree rigid rotation plus fractional translation; final board bounds60x40mm with offset."
        anonymize(item, [repair, negative], 841972+index)
        yield item, repair, negative, notes


def generate(output=OUTPUT, validator=lab.DEFAULT_VALIDATOR):
    manifest = {"schema_version": 1, "cases": [],
        "intent": "Distinguish planar trace clearance from through-via clearance on every spanned layer.",
        "blinding": "Private by protocol only; do not expose construction source, witnesses, or diagnostic negatives to blind solvers."}
    for item, repair, negative, notes in fixtures():
        positive_result = lab.evaluate(item, repair, validator)
        negative_result = lab.evaluate(item, negative, validator)
        if not positive_result["valid"] or not positive_result["improved"]:
            raise ValueError(f"Positive witness failed for {item['id']}: {positive_result}")
        if negative_result["valid"]:
            raise ValueError(f"Expected via-clearance failure for {item['id']}: {negative_result}")
        # Private diagnostic only: shrink the via envelope to the trace envelope
        # to isolate the via-specific cause. This is NOT an accepted repair or
        # any change to the real/public benchmark rules.
        planar_probe = copy.deepcopy(item)
        planar_probe["rules"]["via_diameter"] = item["rules"]["trace_width"]
        planar_probe["rules"]["via_drill"] = item["rules"]["trace_width"]/2
        planar_result = lab.evaluate(planar_probe, negative, validator)
        if not planar_result["valid"]:
            raise ValueError(f"Negative geometry also fails its trace-sized-envelope diagnostic for {item['id']}: {planar_result}")
        lower = notes.get("known_cost_lower_bound")
        if lower is not None and not math.isclose(positive_result["new"]["cost_mm"], lower, abs_tol=1e-8):
            raise ValueError("Known witness does not achieve its stated lower bound")
        private = output / "private" / item["id"]
        problem = lab.semantic_problem(item)
        lab.write_json(output / "public" / f"{item['id']}.json", item)
        lab.write_json(private / "problem.json", problem)
        lab.write_json(private / "candidate.json", lab.candidate(item, problem))
        lab.write_json(private / "answer.json", {"notes": notes, "proposal": repair,
            "assessment": positive_result, "negative": {"proposal": negative, "assessment": negative_result},
            "trace_envelope_counterfactual": {"assessment": planar_result,
                "changed_rules": {"via_diameter": planar_probe["rules"]["via_diameter"],
                                  "via_drill": planar_probe["rules"]["via_drill"]},
                "status": "Diagnostic only. Invalid under original via rules; this counterfactual is not a benchmark repair."}})
        manifest["cases"].append({"id": item["id"], "category": notes["category"], "assessment": positive_result})
    lab.write_json(output / "private/manifest.json", manifest)
    lab.write_json(output / "public/objective.json", {
        "objective": "Minimize total trace centerline length_mm + 2mm per via.",
        "constraints": "Preserve exact endpoint pads and their copper layers, connections, obstacles, widths, via dimensions, clearance, and board bounds. Only route geometry/layer replacements allowed. Arbitrary segment angles allowed. Some boards may have no improvement.",
        "via_rules": "Each internal change of segment layer inserts a via of the declared diameter/drill. The via must clear obstacles and foreign copper on both layers. Endpoint vias are unsupported.",
        "transaction": "Multiple route replacements are validated together.",
        "proposal_format": {"replacements": [{"route_id": "R123", "points": [[0, 0], [1, 1]], "layers": ["top"]}]},
        "units": "mm"})
    return {"generated": len(manifest["cases"]), "public": str(output / "public"), "private": str(output / "private")}


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--output", type=Path, default=OUTPUT)
    parser.add_argument("--validator", type=Path, default=lab.DEFAULT_VALIDATOR)
    args = parser.parse_args()
    print(generate(args.output, args.validator))


if __name__ == "__main__":
    main()
