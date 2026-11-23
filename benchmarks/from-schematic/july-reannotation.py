#!/usr/bin/env python3
# Copyright (C) 2026 Florian Loitsch.
"""Stage and measure stable-symbol reannotation for the July powder doser."""

import argparse
from collections import Counter
import json
import os
from pathlib import Path
import re
import shutil
from subprocess import run

import comparison_scoring as common
from returned_board import children, native_counts, normalized_footprints, record

VARIANTS = {
    "original": {},
    "rename": {"C2": "C7", "U2": "U7"},
    "swap": {"C2": "C3", "C3": "C2"},
    "recycled": {},
}
ORIGINAL_C3 = "00000000-0000-0000-0001-0d073b7df79b"
RECYCLED_C3 = "fb1c2c0f-1436-4d57-8c92-768cd49fb622"
BOARD = "test_module_unplaced"
BRIEF = Path(__file__).with_name("july-six-mm.json")
GPIO_PATCH = BRIEF.with_name("july-edits") / "move-gpio.patch"


def renamed(brief, changes):
    """Keep each independently authored constraint on its original symbol."""
    for row in brief["near"]:
        row["part"] = changes.get(row["part"], row["part"])
        other, pin = row["pin_of"].split(":")
        row["pin_of"] = f"{changes.get(other, other)}:{pin}"
    for row in brief["place"]:
        row["part"] = changes.get(row["part"], row["part"])
    return brief


def prepare(args):
    original = args.original / f"{BOARD}.kicad_sch"
    args.output.mkdir(parents=True)
    text = original.read_text()
    if args.move_gpio:
        common.require(args.variant == "swap", "--move-gpio applies to the swap variant")
        text = run(["patch", "-s", "-o", "-", str(original), str(GPIO_PATCH)],
                   check=True, capture_output=True, text=True).stdout
    changes = VARIANTS[args.variant]
    if args.variant == "recycled":
        original_id = f'(uuid "{ORIGINAL_C3}")'
        common.require(text.count(original_id) == 1 and RECYCLED_C3 not in text,
                       "the original July C3 UUID is missing or ambiguous")
        text = text.replace(original_id, f'(uuid "{RECYCLED_C3}")')
    if changes:
        for prefix in ('(property "Reference" ', '(reference '):
            counts = Counter()

            def replace(match):
                ref = match[1]
                counts[ref] += 1
                return f'{prefix}"{changes[ref]}"'

            pattern = re.escape(prefix) + r'"(' + "|".join(changes) + r')"'
            text = re.sub(pattern, replace, text)
            common.require(counts == {key: 1 for key in changes},
                           f"unexpected July symbol references for {prefix}: {counts}")
    (args.output / original.name).write_text(text)
    shutil.copyfile(original.with_suffix(".kicad_pro"), args.output / f"{BOARD}.kicad_pro")
    brief = renamed(json.loads(BRIEF.read_text()), changes)
    (args.output / "constraints.json").write_text(json.dumps(brief, indent=2) + "\n")
    (args.output / "placer.json").write_text('{"tuning":{"refine_pad_near":true}}\n')


def legacy_donor(args):
    text = args.prior.read_text()
    text, count = re.subn(r'^    \(path "[^"]+"\)\n', "", text, flags=re.MULTILINE)
    common.require(count == 15, "the native July donor must have 15 footprint paths")
    with args.output.open("x") as stream:
        stream.write(text)


def board_identity(pcbnew, path):
    board = pcbnew.LoadBoard(str(path))
    paths = {}
    for key, form in children(path.read_text()):
        if key == "footprint":
            ref = re.search(r'\(property "Reference" "([^"]+)"', form)
            symbol = re.search(r'\(path "([^"]+)"', form)
            common.require(ref and symbol, f"unresolved footprint identity in {path}")
            paths[ref[1]] = symbol[1]
    positions = common.poses(board)
    common.require(len(paths) == len(positions) == len(set(paths.values())) == len(board.GetFootprints()) == 15,
                   f"expected 15 distinct footprint paths and references in {path}")
    pads = {paths[footprint.GetReference()]: sorted(
        (pad.GetNumber(), pad.GetNetname(), common.point(pad.GetPosition())) for pad in footprint.Pads()
    ) for footprint in board.GetFootprints()}
    fingerprints = normalized_footprints(pcbnew, board, electrical=False)
    return board, paths, {path: positions[ref] for ref, path in paths.items()}, pads, {
        paths[ref]: group["occurrences"][0]["signature"] for ref, group in fingerprints.items()
    }


def copper(board):
    result = {}
    for item in board.GetTracks():
        identity = item.m_Uuid.AsString()
        common.require(item.GetClass() == "PCB_TRACK" and identity not in result,
                       "July expects unique track UUIDs and straight segments")
        geometry = (item.GetClass(), item.GetLayer(), item.GetWidth(),
                    *sorted((common.point(item.GetStart()), common.point(item.GetEnd()))))
        result[identity] = (item.GetNetname(), geometry)
    return result


def native_ok(path, log):
    report = json.loads(path.read_text())
    counts = native_counts(report, f"{BOARD}.kicad_pcb", log.read_text())
    common.require(counts["drc_errors"] == counts["unconnected"] == counts["parity"] == 0,
                   f"active native errors, opens, or parity on {path}")
    return counts


def check_native(args):
    args.output.mkdir(parents=True)
    board = record(args.board)
    env = dict(os.environ, LC_ALL="C", XDG_CONFIG_HOME=str(args.output.resolve() / "config"))
    kicad = os.environ.get("PCB_KICAD_CLI", "kicad-cli")
    for mode, flags in (("stored", common.NATIVE_CHECKS["as_saved"]),
                        ("refilled", common.NATIVE_CHECKS["refilled"])):
        report = args.output / f"{mode}.json"
        log = args.output / f"{mode}.log"
        process = run([kicad, *flags, "--output", str(report), str(args.board.resolve())],
                      capture_output=True, text=True, env=env, timeout=180)
        log.write_text(process.stdout + process.stderr)
        common.require(process.returncode == 0, f"KiCad rejected the {mode} check: {log}")
        native_ok(report, log)
    common.require(board == record(args.board), "KiCad changed the submitted board")
    (args.output / "board.json").write_text(json.dumps(board) + "\n")


def terminal_groups(pads):
    groups = {}
    for path, contacts in pads.items():
        for number, net, _ in contacts:
            if net:
                groups.setdefault(net, set()).add((path, number))
    return {frozenset(terminals) for terminals in groups.values()}


def unchanged(expected, actual):
    return sum(value == actual.get(key) for key, value in expected.items())


def compare(args):
    import pcbnew

    original, refs, poses, pads, footprints = board_identity(pcbnew, args.prior)
    prior_copper = copper(original)
    output = {"prior": record(args.prior), "donor": record(args.donor or args.prior),
              "prior_segments": len(prior_copper), "arms": {}}
    for label, run_dir, score_dir, checks in args.arm:
        run_dir, score_dir, checks = map(Path, (run_dir, score_dir, checks))
        layout = run_dir / "layout"
        path = layout / "result" / args.prior.name
        board, current, positions, contacts, signatures = board_identity(pcbnew, path)
        refs_by_path = {symbol: ref for ref, symbol in current.items()}
        renamed_refs = {old: refs_by_path[symbol] for old, symbol in refs.items()
                        if symbol in refs_by_path and old != refs_by_path[symbol]}
        common.require(renamed_refs == VARIANTS[args.variant], f"unexpected annotation in {label}: {renamed_refs}")
        wrapper, = json.loads((run_dir / "results.json").read_text())
        score, = json.loads((score_dir / "results.json").read_text())
        evidence = record(path)
        common.require(score["pass"] and score["candidate_board"]["sha256"] == evidence["sha256"]
                       and score["brief_sha256"] == wrapper["brief_sha256"],
                       f"independent mechanical or native score failed for {label}")
        report = json.loads((layout / "board-layout.json").read_text())
        route = report["routed"]
        verdict = json.loads((layout / "verdict.json").read_text())
        common.require(verdict["status"] == "complete"
                       and verdict["exit_code"] == wrapper["layout_exit_code"] == 0,
                       f"{label} did not complete the normal layout workflow")
        result = copper(board)
        reuse = route.get("route_reuse")
        if reuse:
            common.require(reuse["sha256"] == output["donor"]["sha256"],
                           f"{label} used a different routed donor")
            if "renamed_references" in reuse:
                common.require(reuse["renamed_references"] == renamed_refs
                               == verdict["route_reuse"]["renamed_references"],
                               f"{label} reported the wrong schematic path matches")
        retained = Counter(net for uuid, (net, geom) in prior_copper.items()
                           if result.get(uuid) == (net, geom))
        native = {mode: native_ok(checks / f"{mode}.json", checks / f"{mode}.log")
                  for mode in ("stored", "refilled")}
        changed_nets = {ref: [(old[0], old[1], new[1])
                              for old, new in zip(pads[symbol], contacts[symbol]) if old[1] != new[1]]
                        for ref, symbol in refs.items() if symbol in contacts}
        common.require(json.loads((checks / "board.json").read_text())["sha256"] == evidence["sha256"],
                       f"{label} native evidence belongs to a different board")
        output["arms"][label] = {
            "binary": wrapper["inputs"]["binary"], "board": evidence, "reannotated": renamed_refs,
            "recycled_references": sorted(ref for ref, symbol in refs.items()
                                          if ref in current and symbol not in positions),
            "footprint_geometry_by_symbol_path_unchanged": unchanged(footprints, signatures),
            "poses_by_symbol_path_unchanged": unchanged(poses, positions),
            "pad_centres_by_symbol_path_unchanged": unchanged(
                {key: [(num, pos) for num, _, pos in values] for key, values in pads.items()},
                {key: [(num, pos) for num, _, pos in values] for key, values in contacts.items()},
            ),
            "net_changes": {ref: changes for ref, changes in changed_nets.items() if changes},
            "prior_electrical_groups_unchanged": len(terminal_groups(pads) & terminal_groups(contacts)),
            "prior_electrical_groups": len(terminal_groups(pads)),
            "track_uuids_with_unchanged_geometry_and_net": retained.total(),
            "retained_tracks_by_net": dict(retained),
            "track_uuids_with_unchanged_geometry": unchanged(
                {uuid: geom for uuid, (_, geom) in prior_copper.items()},
                {uuid: geom for uuid, (_, geom) in result.items()},
            ),
            "track_geometry_overlap": (Counter(prior_copper.values()) & Counter(result.values())).total(),
            "tracks": len(result), "as_saved_trace_mm": pcbnew.ToMM(sum(item.GetLength() for item in board.GetTracks())),
            "reuse": reuse, "expansions": route["expansions"],
            "modeled_work_seconds": verdict["work_seconds"], "elapsed_seconds": wrapper["seconds"],
            "constraint_count": len(score["constraints"]), "native": native,
        }
    print(json.dumps(output, indent=2))


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    commands = parser.add_subparsers(dest="command", required=True)
    stage = commands.add_parser("prepare")
    stage.add_argument("original", type=Path)
    stage.add_argument("output", type=Path)
    stage.add_argument("--variant", choices=VARIANTS, required=True)
    stage.add_argument("--move-gpio", action="store_true")
    stage.set_defaults(action=prepare)
    legacy = commands.add_parser("legacy-donor")
    legacy.add_argument("prior", type=Path)
    legacy.add_argument("output", type=Path)
    legacy.set_defaults(action=legacy_donor)
    native = commands.add_parser("native")
    native.add_argument("board", type=Path)
    native.add_argument("output", type=Path)
    native.set_defaults(action=check_native)
    check = commands.add_parser("compare")
    check.add_argument("prior", type=Path)
    check.add_argument("--donor", type=Path, help="the donor used for reuse if distinct from the original PCB")
    check.add_argument("--variant", choices=VARIANTS, required=True)
    check.add_argument("--arm", nargs=4, metavar=("LABEL", "LAYOUT", "SCORE", "NATIVE"), action="append", required=True)
    check.set_defaults(action=compare)
    args = parser.parse_args()
    args.action(args)


if __name__ == "__main__":
    main()
