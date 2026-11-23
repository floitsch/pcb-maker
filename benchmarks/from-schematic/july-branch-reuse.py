#!/usr/bin/env python3
# Copyright (C) 2026 Florian Loitsch.
"""Prepare and measure the July powder-doser C3 move."""

import argparse
from collections import Counter
import json
from pathlib import Path
import re

import comparison_scoring as common
from returned_board import record
from routed_supply_paths import UnsupportedPath, make_graph, measure_supply_paths, shortest

BRIEF = Path(__file__).with_name("july-six-mm.json")
ORIGINAL_C3 = (99_096_196, 16_070_861, 0, False)
MOVED_C3 = (99_096_196, 14_950_000, 0.0, False)
SUPPLY = "+12V"
NETS = {SUPPLY: ("C3:1", "U5:16", 39, 12), "GND": ("C3:2", "U5:17", 146, 35)}


def load(pcbnew, path):
    board = pcbnew.LoadBoard(str(path))
    poses = common.poses(board)
    common.require(len(poses) == len(board.GetFootprints()) == 15,
                   f"expected 15 distinct July footprints in {path}")
    return board, poses


def prepare(pcbnew, prior, output):
    _, poses = load(pcbnew, prior)
    common.require(poses["C3"] == ORIGINAL_C3, "previous board lacks the original July C3 pose")
    brief = json.loads(BRIEF.read_text())
    held = {entry["part"] for entry in brief["place"]}
    common.require(poses.keys() - held == {"C1", "C2", "C3"},
                   "the original brief should hold the 12 non-capacitor parts")
    for reference, (x, y, angle, _) in sorted({**poses, "C3": MOVED_C3}.items()):
        if reference not in held:
            brief["place"].append({"part": reference, "x": pcbnew.ToMM(x), "y": pcbnew.ToMM(y),
                                   "angle": angle, "origin": "absolute"})
    with output.open("x") as stream:
        json.dump(brief, stream, indent=2)
        stream.write("\n")


def pad_contacts(board, edited):
    return sorted((footprint.GetReference(), pad.GetNumber(), pad.GetNetname(),
                   common.point(pad.GetPosition()) if footprint.GetReference() != edited else ())
                  for footprint in board.GetFootprints() for pad in footprint.Pads())


def segment_key(item):
    return (item.GetLayer(), item.GetWidth(),
            *sorted((common.point(item.GetStart()), common.point(item.GetEnd()))))


def branch_segments(board, net):
    return [item for item in board.GetTracks() if item.GetNetname() == net
            and item.GetClass() == "PCB_TRACK"]


def contacts_on_net(board, net):
    return {f"{footprint.GetReference()}:{pad.GetNumber()}": pad for footprint in board.GetFootprints()
            for pad in footprint.Pads() if pad.GetNetname() == net}


def expected_splice(prior, board, net):
    moved, _, _, _ = NETS[net]
    old = common.point(contacts_on_net(prior, net)[moved].GetPosition())
    new = common.point(contacts_on_net(board, net)[moved].GetPosition())
    matches = [(layer, width, end if start == old else start)
               for layer, width, start, end in map(segment_key, branch_segments(prior, net))
               if old == start or old == end]
    common.require(len(matches) == 1, f"{moved} must have one prior segment at its pad centre")
    layer, width, predecessor = matches[0]
    return layer, width, *sorted((new, predecessor))


def independent_score(board, directory):
    row, = json.loads((directory / "results.json").read_text())
    common.require(row.get("mode") == "returned-board" and row.get("kicad_checked") is True
                   and row.get("candidate_board", {}).get("sha256") == record(board)["sha256"],
                   f"missing matching KiCad score for {board}")
    return row


def measure(pcbnew, board, net, original_segments):
    moved, anchor, _, _ = NETS[net]
    segments = branch_segments(board, net)
    overlap = Counter(segment_key(item) for item in segments) & original_segments
    subset = overlap.copy()
    retained = []
    for item in segments:
        key = segment_key(item)
        if subset[key]:
            subset[key] -= 1
            retained.append(item)
    pads = contacts_on_net(board, net)
    graph = make_graph(pcbnew, board, pads[anchor].GetNetCode(), pads, retained, track_only=True)
    connected = [anchor]
    for target in sorted(pads.keys() - {anchor}):
        try:
            shortest(graph, ("pad", anchor), ("pad", target))
        except UnsupportedPath:
            continue
        connected.append(target)
    evidence = {"original_segments_reappearing": overlap.total(),
                "original_only_connected_contacts": connected,
                "segment_layers": sorted({board.GetStandardLayerName(item.GetLayer()) for item in segments}),
                "zone_layers": sorted({board.GetStandardLayerName(layer) for zone in board.Zones()
                                       if zone.GetNetname() == net and not zone.GetIsRuleArea()
                                       for layer in zone.GetLayerSet().Seq()})}
    if net == SUPPLY:
        path, = measure_supply_paths(pcbnew, board, [(moved, anchor)])
        common.require(path["status"] == "measured", f"no track-centreline path from {moved} to {anchor}")
        evidence["moved_terminal_path"] = path
    return evidence


def compare(pcbnew, args):
    prior, prior_poses = load(pcbnew, args.prior)
    current, current_poses = load(pcbnew, args.current)
    common.require(prior_poses["C3"] == ORIGINAL_C3 and current_poses == {**prior_poses, "C3": MOVED_C3},
                   "the input pose edit must change only C3")
    common.require(pad_contacts(current, "C3") == pad_contacts(prior, "C3"),
                   "pad membership and pads outside C3 must match the prior")
    common.require(not current.GetTracks(), "the current board must be copper-free before routing")
    original_segments = {}
    contacts = {}
    for net, (_, _, expected_segments, expected_contacts) in NETS.items():
        original_segments[net] = Counter(segment_key(item) for item in branch_segments(prior, net))
        terminals = [f"{part}:{pad}" for part, pad, name, _ in pad_contacts(prior, "") if name == net]
        common.require(len(terminals) == len(set(terminals)) == expected_contacts
                       and original_segments[net].total() == expected_segments,
                       f"the prior did not reproduce the July {net} branch fixture")
        contacts[net] = set(terminals)
    original = independent_score(args.prior, args.prior_score)
    common.require(original.get("pass") and original.get("parity") == 0
                   and original["inputs"]["constraints"]["sha256"] == record(BRIEF)["sha256"],
                   "the saved prior must pass native verification and its original brief")
    updated_brief = record(args.updated_brief)
    evidence = {"prior": record(args.prior), "current": record(args.current),
                "original_brief": record(BRIEF), "updated_brief": updated_brief,
                "prior_branches": {net: measure(pcbnew, prior, net, original_segments[net]) for net in NETS},
                "arms": {}}
    for name, directory, score_directory, log in args.arms:
        path = directory / args.current.name
        independent = independent_score(path, score_directory)
        common.require(independent["inputs"]["constraints"]["sha256"] == updated_brief["sha256"]
                       and independent["project_files_sha256"] == original["project_files_sha256"]
                       and independent["outline_sha256"] == original["outline_sha256"]
                       and independent["inputs"]["footprints_from"]["sha256"]
                       == original["inputs"]["footprints_from"]["sha256"]
                       and independent.get("pass") and independent.get("parity") == 0,
                       f"{name} failed the revised brief or native verification")
        board, poses = load(pcbnew, path)
        common.require(poses == current_poses and pad_contacts(board, "") == pad_contacts(current, ""),
                       f"{name} changed a supplied pose or physical pad contact")
        reuse_report = json.loads((directory / "board-router.json").read_text())
        reuse = reuse_report["route_reuse"]
        common.require(reuse["sha256"] == evidence["prior"]["sha256"], f"{name} used a different prior")
        expansions = re.findall(r"(?m)^profile rate: phases ([0-9]+) expansions", log.read_text())
        tracks = list(board.GetTracks())
        copper = {"nets": {net: measure(pcbnew, board, net, original_segments[net]) for net in NETS},
                  "total_track_length_mm": round(pcbnew.ToMM(sum(
                      item.GetLength() for item in tracks if not isinstance(item, pcbnew.PCB_VIA))), 6),
                  "vias": sum(isinstance(item, pcbnew.PCB_VIA) for item in tracks)}
        if name in args.expect_branch:
            branches = reuse.get("partial_nets", {})
            common.require(reuse["status"] == "accepted" and SUPPLY in branches,
                           f"{name} did not accept the {SUPPLY} branch")
            verdict = json.loads((directory / "verdict.json").read_text())
            common.require(verdict.get("route_reuse", {}).get("partial_nets") == branches,
                           f"{name} branch evidence differs between the route report and verdict")
            for net, branch in branches.items():
                moved, _, expected_segments, expected_contacts = NETS[net]
                common.require(branch["pruned_terminal"] == moved
                               and branch["retained_terminals"] == expected_contacts - 1
                               and branch["retained_segments"] + branch["pruned_segments"] == expected_segments,
                               f"{name} reported unexpected {net} branch counts")
                actual = copper["nets"][net]
                common.require(actual["original_segments_reappearing"] >= branch["retained_segments"]
                               and set(actual["original_only_connected_contacts"]) == contacts[net] - {moved},
                               f"{name} did not preserve a connected prior {net} trunk to every stationary contact")
                if branch.get("spliced_segments"):
                    splice = expected_splice(prior, board, net)
                    common.require(branch["pending_terminals"] == 0 and branch["spliced_segments"] == 1
                                   and branch["pruned_segments"] == 1
                                   and splice in map(segment_key, branch_segments(board, net)),
                                   f"{name} did not save the declared {net} splice to the old predecessor")
                    layer, width, start, end = splice
                    actual["splice"] = {"layer": board.GetStandardLayerName(layer), "width_mm": pcbnew.ToMM(width),
                                        "from_mm": pcbnew.ToMM(pcbnew.VECTOR2I(*start)),
                                        "to_mm": pcbnew.ToMM(pcbnew.VECTOR2I(*end))}
        evidence["arms"][name] = {"board": record(path), "reuse": reuse,
                                   "work_seconds": reuse_report["work_seconds"],
                                   "profile_astar_expansions": [int(value) for value in expansions],
                                   "native_drc_errors": independent["drc_errors"],
                                   "native_drc_warnings": independent["drc_warnings"],
                                   "native_opens": independent["unconnected"],
                                   "native_parity": independent["parity"],
                                   "copper": copper}
    return evidence


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    commands = parser.add_subparsers(dest="command", required=True)
    brief = commands.add_parser("brief", help="write the revised mechanical brief")
    brief.add_argument("prior", type=Path)
    brief.add_argument("output", type=Path)
    score = commands.add_parser("compare", help="compare saved route arms and independent scores")
    score.add_argument("prior", type=Path)
    score.add_argument("current", type=Path)
    score.add_argument("prior_score", type=Path)
    score.add_argument("updated_brief", type=Path)
    score.add_argument("--arm", dest="arms", nargs=4, action="append", required=True,
                       metavar=("NAME", "RESULT_DIR", "SCORE_DIR", "ROUTE_LOG"))
    score.add_argument("--expect-branch", action="append", default=[], metavar="NAME")
    args = parser.parse_args()
    try:
        import pcbnew
        if args.command == "brief":
            prepare(pcbnew, args.prior, args.output)
        else:
            args.arms = [(name, *(Path(path) for path in paths)) for name, *paths in args.arms]
            if not set(args.expect_branch).issubset(name for name, *_ in args.arms):
                parser.error("--expect-branch must name an --arm")
            print(json.dumps(compare(pcbnew, args), indent=2))
    except (ImportError, KeyError, OSError, RuntimeError, TypeError, ValueError) as error:
        parser.exit(2, f"{error}\n")


if __name__ == "__main__":
    main()
