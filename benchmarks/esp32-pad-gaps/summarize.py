# Copyright (C) 2026 Toit contributors.
"""Inspect native results of the paired pad-gap benchmarks; retain a summary."""
import argparse
import itertools
import json
from pathlib import Path

import pcbnew

# KiCad 10's generated container iterator still calls the Python 2 spelling.
if not hasattr(pcbnew.SwigPyIterator, "next"):
    pcbnew.SwigPyIterator.next = pcbnew.SwigPyIterator.__next__


def crossings(board_path):
    board = pcbnew.LoadBoard(str(board_path))
    found = set()
    gaps = []
    for footprint in board.GetFootprints():
        if "ESP" not in str(footprint.GetFPID().GetLibItemName()):
            continue
        pads = list(footprint.Pads())
        for first, second in itertools.combinations(pads, 2):
            a, b = first.GetBoundingBox(), second.GetBoundingBox()
            boxes = [
                [pcbnew.ToMM(r.GetX()), pcbnew.ToMM(r.GetY()),
                 pcbnew.ToMM(r.GetRight()), pcbnew.ToMM(r.GetBottom())]
                for r in [a, b]
            ]
            for axis in [0, 1]:
                cross_axis = 1 - axis
                left, right = sorted(boxes, key=lambda box: box[axis])
                if abs(right[axis] - left[axis + 2] - 0.37) > 1e-4:
                    continue
                if (abs(left[cross_axis] - right[cross_axis]) > 1e-4
                        or abs(left[cross_axis + 2] - right[cross_axis + 2]) > 1e-4):
                    continue
                gaps.append((footprint.GetReference(), first.GetNumber(), second.GetNumber(),
                             axis, left[axis + 2], right[axis],
                             (left[cross_axis] + left[cross_axis + 2]) / 2))
    arcs = 0
    for track in board.GetTracks():
        if isinstance(track, pcbnew.PCB_VIA) or track.GetLayer() != pcbnew.F_Cu:
            continue
        if isinstance(track, pcbnew.PCB_ARC):
            arcs += 1
            continue
        start = [pcbnew.ToMM(v) for v in track.GetStart()]
        end = [pcbnew.ToMM(v) for v in track.GetEnd()]
        for ref, first, second, axis, low, high, mid in gaps:
            perpendicular = 1 - axis
            delta = end[perpendicular] - start[perpendicular]
            if abs(delta) < 1e-9:
                continue
            t = (mid - start[perpendicular]) / delta
            if -1e-9 <= t <= 1 + 1e-9:
                at = start[axis] + t * (end[axis] - start[axis])
                if low + 1e-6 < at < high - 1e-6:
                    found.add((ref, first, second, track.GetNetname()))
    return {"available_pad_gaps": len(gaps), "crossed_pad_gaps": len(found),
            "crossings": sorted(found), "unsupported_front_arcs": arcs}


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("directory", type=Path, help="Corpus directory and summary destination")
    parser.add_argument("--case", action="append", default=[], metavar="NAME=DIRECTORY",
                        help="Read a named existing comparison; repeat for blocked/open partners")
    args = parser.parse_args()
    paths = {}
    if args.case:
        for entry in args.case:
            name, separator, directory = entry.partition("=")
            if not separator or not directory or not name.endswith(("-blocked", "-open")):
                parser.error("--case requires NAME=DIRECTORY with a -blocked or -open name")
            if name in paths:
                parser.error(f"duplicate case name: {name}")
            paths[name] = Path(directory) / "competitive-comparison.json"
    else:
        paths = {path.parent.name: path for path in
                 sorted(args.directory.glob("*/competitive-comparison.json"))}
    rows = {}
    checks = []
    for name, path in sorted(paths.items()):
        comparison = json.loads(path.read_text())
        native = json.loads((path.parent / "competitive-result.json").read_text())
        row = {"comparison": str(path.resolve()),
               "status": comparison["status"], "failure": comparison["failure"]}
        for engine, data in [("pcb_maker", comparison.get("pcb_maker")),
                             ("freerouting", native.get("result"))]:
            if not data:
                row[engine] = None
                continue
            statistics = data["statistics"]
            admitted = (data.get("route_admission") or data["verification"])["complete"]
            target_complete = data.get("solved_target", True)
            record = {"board": data["board"],
                      "complete": data["verification"]["complete"] and admitted and target_complete,
                      "native_complete": data["verification"]["complete"],
                      "target_complete": target_complete,
                      "route_admission_complete": admitted,
                      "length_mm": statistics["physical_copper"]["physical_centerline_length_mm"],
                      "vias": statistics["vias"],
                      "placement_sha256": statistics["component_placement_100nm_sha256"],
                      **crossings(data["board"])}
            if engine == "pcb_maker":
                record.update({key: data[key] for key in ["final_rung", "target_rung", "total_route_expansions", "elapsed_micros", "timed_out"]})
                record["executable_sha256"] = data["executable_sha256"]
            row[engine] = record
        rows[name] = row
    for prefix in sorted({name.rsplit("-", 1)[0] for name in rows}):
        closed, opened = rows.get(prefix + "-blocked"), rows.get(prefix + "-open")
        if not closed or not opened:
            checks.append(f"missing profile partner for {prefix}")
            continue
        for engine in ["pcb_maker", "freerouting"]:
            a, b = closed[engine], opened[engine]
            if not a or not b:
                checks.append(f"missing {engine} result for {prefix}")
                continue
            if a["placement_sha256"] != b["placement_sha256"]:
                checks.append(f"profile placements differ: {prefix}/{engine}")
            if engine == "pcb_maker" and a["executable_sha256"] != b["executable_sha256"]:
                checks.append(f"profile executables differ: {prefix}/{engine}")
            if a["crossed_pad_gaps"]:
                checks.append(f"blocked profile crosses pads: {prefix}/{engine}")
            if a["unsupported_front_arcs"] or b["unsupported_front_arcs"]:
                checks.append(f"gap detector has unsupported arcs: {prefix}/{engine}")
            if prefix == "gap" and (not a["complete"] or not b["complete"] or b["crossed_pad_gaps"] != 1):
                checks.append(f"focused native passage regression failed: {engine}")
    if not rows:
        checks.append("no benchmark results found")
    result = {"cases": rows, "failed_checks": checks,
              "scope": "Gap crossings are straight F.Cu track centerlines through adjacent ESP pad-row midplanes. Native DRC remains clearance authority. Elapsed time includes native orchestration; expansions measure search work."}
    args.directory.mkdir(parents=True, exist_ok=True)
    (args.directory / "pad-gap-summary.json").write_text(json.dumps(result, indent=2) + "\n")
    for name, row in rows.items():
        print(name, {engine: None if not row[engine] else {k: row[engine][k] for k in ["complete", "length_mm", "vias", "crossed_pad_gaps"]} for engine in ["pcb_maker", "freerouting"]})
    if checks:
        raise SystemExit("; ".join(checks))


if __name__ == "__main__":
    main()
