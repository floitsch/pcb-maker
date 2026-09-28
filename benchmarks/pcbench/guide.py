#!/usr/bin/env python3
# Copyright (C) 2026 Toit contributors.
"""Guide boards for KiCad 10 files: D3's step 2, reimplemented.

PCBWorld's make_guide.py reads nets as numbers, which KiCad 10 no longer
writes (tracks name their net). This follows the same algorithm
(PCBWorld tools/datagen/pcbench_prep/make_guide.md):

- **Width per net.** Every net gets one width: the widest track the designer
  used on it. It lands in its own net class (a copy of the net's class with
  that track width) through an exact `netclass_patterns` entry, so a router
  that reads only net classes reproduces the designer's widths.
- **Trials.** The routed board with every track rewritten to its net's width
  is checked by KiCad's DRC. Nets involved in track violations fall back to
  their next narrower observed width, for up to five trials.
- **Output.** For every board folder under `--work`:
  `processed_v9_guide_v3.kicad_pcb` and `.kicad_pro`, plus
  `processed_v9_guide_v3_unrouted.kicad_pcb` (tracks and vias removed;
  zones stay).
- **Status.** A board whose guide board ends DRC-clean is `pass`; others are
  written but reported `fail`, and the manifest leaves them out."""

import argparse
import json
import re
import subprocess
import sys
import tempfile
from concurrent.futures import ProcessPoolExecutor
from pathlib import Path

ROOT = Path(__file__).resolve().parents[2]
WORK = ROOT / "benchmarks/real/external/pcbench-work/newdrc"
STEM = "processed_v9"
GUIDE = STEM + "_guide_v3"
TRIALS = 5
TRACK_KINDS = {"clearance", "track_width", "shorting_items", "tracks_crossing", "hole_clearance",
               "copper_edge_clearance", "track_dangling", "connection_width"}


def blocks(text, head):
    """(start, end) of every top-level-ish `(head ...)` block."""
    spans = []
    index = 0
    token = "(" + head
    while True:
        start = text.find(token, index)
        if start < 0:
            return spans
        following = text[start + len(token): start + len(token) + 1]
        if following not in (" ", "\n", "\t", "\r"):
            index = start + 1
            continue
        depth = 0
        quoted = False
        for position in range(start, len(text)):
            character = text[position]
            if character == '"' and text[position - 1] != "\\":
                quoted = not quoted
            elif not quoted:
                if character == "(":
                    depth += 1
                elif character == ")":
                    depth -= 1
                    if depth == 0:
                        break
        spans.append((start, position + 1))
        index = position + 1


NET = re.compile(r'\(net\s+"((?:[^"\\]|\\.)*)"\)')
WIDTH = re.compile(r"\(width\s+([0-9.]+)\)")


def track_widths(text):
    widths = {}
    for head in ("segment", "arc"):
        for start, end in blocks(text, head):
            block = text[start:end]
            net = NET.search(block)
            width = WIDTH.search(block)
            if net and width and net.group(1):
                widths.setdefault(net.group(1), set()).add(round(float(width.group(1)), 4))
    return {net: sorted(values) for net, values in widths.items()}


def rewrite_widths(text, chosen):
    pieces = []
    last = 0
    spans = sorted(blocks(text, "segment") + blocks(text, "arc"))
    for start, end in spans:
        block = text[start:end]
        net = NET.search(block)
        if net and net.group(1) in chosen:
            block = WIDTH.sub(f"(width {chosen[net.group(1)]:g})", block, count=1)
        pieces.append(text[last:start])
        pieces.append(block)
        last = end
    pieces.append(text[last:])
    return "".join(pieces)


def strip_copper(text):
    spans = sorted(blocks(text, "segment") + blocks(text, "arc") + blocks(text, "via"))
    pieces = []
    last = 0
    for start, end in spans:
        # Drop the block with the whitespace before it.
        cut = start
        while cut > last and text[cut - 1] in " \t\n\r":
            cut -= 1
        pieces.append(text[last:cut])
        last = end
    pieces.append(text[last:])
    return "".join(pieces)


def net_class(project, net):
    settings = project.get("net_settings", {})
    assignments = settings.get("netclass_assignments") or {}
    if net in assignments:
        value = assignments[net]
        return value[0] if isinstance(value, list) else value
    for entry in settings.get("netclass_patterns") or []:
        pattern = entry.get("pattern", "")
        regex = "^" + re.escape(pattern).replace(r"\*", ".*").replace(r"\?", ".") + "$"
        if re.match(regex, net):
            return entry.get("netclass", "Default")
    return "Default"


def guide_project(project, chosen):
    """Copies the project with one class per (original class, width)."""
    guide = json.loads(json.dumps(project))
    settings = guide.setdefault("net_settings", {})
    classes = {entry["name"]: entry for entry in settings.get("classes", [])}
    new_classes = [entry for entry in settings.get("classes", [])]
    patterns = []
    made = {}
    for net, width in sorted(chosen.items()):
        original = net_class(project, net)
        base = classes.get(original) or classes.get("Default")
        if base is None:
            continue
        name = f"{original}_w{width:g}"
        if name not in made:
            entry = dict(base)
            entry["name"] = name
            entry["track_width"] = width
            entry["priority"] = len(new_classes)
            made[name] = entry
            new_classes.append(entry)
        patterns.append({"netclass": name, "pattern": net.replace("*", "?")})
    settings["classes"] = new_classes
    settings["netclass_patterns"] = patterns
    settings["netclass_assignments"] = None
    return guide


def drc(board):
    report = board.with_suffix(".drc.json")
    subprocess.run(["kicad-cli", "pcb", "drc", "--format", "json", "--severity-error", "--all-track-errors",
                    "-o", str(report), str(board)], capture_output=True, timeout=900)
    return json.loads(report.read_text()) if report.exists() else None


def offenders(report, widths_by_uuid):
    nets = set()
    for violation in report.get("violations", []):
        if violation["type"] not in TRACK_KINDS:
            continue
        for item in violation.get("items", []):
            net = widths_by_uuid.get(item.get("uuid"))
            if net:
                nets.add(net)
    return nets


def track_uuids(text):
    uuids = {}
    for head in ("segment", "arc"):
        for start, end in blocks(text, head):
            block = text[start:end]
            net = NET.search(block)
            uuid = re.search(r'\(uuid\s+"([^"]+)"\)', block)
            if net and uuid:
                uuids[uuid.group(1)] = net.group(1)
    return uuids


def process(folder):
    result = {"name": folder.name, "status": "error"}
    try:
        text = (folder / f"{STEM}.kicad_pcb").read_text()
        project = json.loads((folder / f"{STEM}.kicad_pro").read_text())
        widths = track_widths(text)
        if not widths:
            result.update(status="skip", message="no routed tracks")
            return result
        chosen = {net: values[-1] for net, values in widths.items()}
        uuids = track_uuids(text)
        with tempfile.TemporaryDirectory() as scratch:
            board = Path(scratch) / "guide.kicad_pcb"
            for trial in range(TRIALS):
                board.write_text(rewrite_widths(text, chosen))
                board.with_suffix(".kicad_pro").write_text(json.dumps(guide_project(project, chosen), indent=2))
                report = drc(board)
                if report is None:
                    result.update(status="error", message="DRC produced no report")
                    return result
                errors = len(report.get("violations", []))
                result.update(trial=trial, errors=errors, unconnected=len(report.get("unconnected_items", [])))
                if errors == 0:
                    break
                narrowed = False
                for net in offenders(report, uuids):
                    values = widths[net]
                    position = values.index(chosen[net])
                    if position > 0:
                        chosen[net] = values[position - 1]
                        narrowed = True
                if not narrowed:
                    break
        guide_text = rewrite_widths(text, chosen)
        (folder / f"{GUIDE}.kicad_pcb").write_text(guide_text)
        (folder / f"{GUIDE}.kicad_pro").write_text(json.dumps(guide_project(project, chosen), indent=2))
        (folder / f"{GUIDE}_unrouted.kicad_pcb").write_text(strip_copper(guide_text))
        result["status"] = "pass" if result.get("errors") == 0 else "fail"
        result["narrowed_nets"] = sum(1 for net, values in widths.items() if chosen[net] != values[-1])
    except Exception as error:  # noqa: BLE001 - reported per board
        result.update(status="error", message=repr(error)[:300])
    return result


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--work", type=Path, default=WORK)
    parser.add_argument("--workers", type=int, default=6)
    parser.add_argument("--only", nargs="*")
    arguments = parser.parse_args()
    folders = sorted(folder for folder in arguments.work.iterdir()
                     if (folder / f"{STEM}.kicad_pcb").exists()
                     and (not arguments.only or folder.name in arguments.only))
    results = []
    with ProcessPoolExecutor(arguments.workers) as pool:
        for count, result in enumerate(pool.map(process, folders), 1):
            results.append(result)
            if count % 50 == 0:
                print(f"{count}/{len(folders)}", file=sys.stderr, flush=True)
    (arguments.work.parent / "guide.json").write_text(json.dumps(results, indent=1))
    summary = {}
    for result in results:
        summary[result["status"]] = summary.get(result["status"], 0) + 1
    print(f"guide boards: {summary}", file=sys.stderr)


if __name__ == "__main__":
    main()
