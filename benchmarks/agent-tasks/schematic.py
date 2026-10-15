# Copyright (C) 2026 Toit contributors.
"""The schematic tier's task (run.py --tier schematic): the agent's own flow
from a harvested board's schematic.

    import-kicad-netlist <schematic> <project> <board-id> --layers N
      (missing footprints: the libraries harvest_schematics.py found
      elsewhere in the repository go into the project's fp-lib-table, and
      the import runs again, as docs/agents.md section 0 says)
    constraints.json from the designer's board: the task list's own
      constraints (from_pcbench.py) moved onto the new footprints, the
      designer's outline drawn on Edge.Cuts
    layout-kicad-board <project> <board-id> <out> auto <project>/layout.json
    drc-kicad-board <out>/result <board-id> drc.json

Every row says where the flow stopped (`stopped`: import, constraints,
placement, routing or drc; null when it passed) and why (`why`)."""

import fnmatch
import json
import posixpath
import re
import shutil
import subprocess
import time
from pathlib import Path

HERE = Path(__file__).resolve().parent
ROOT = HERE.parents[1]
STORE = ROOT / "benchmarks/real/external/github-sch"
SHAPES = ("gr_line", "gr_arc", "gr_circle", "gr_rect", "gr_poly", "gr_curve")


def block_end(text, start):
    depth, quoted, escaped = 0, False, False
    for index in range(start, len(text)):
        character = text[index]
        if escaped:
            escaped = False
        elif quoted and character == "\\":
            escaped = True
        elif character == '"':
            quoted = not quoted
        elif not quoted:
            if character == "(":
                depth += 1
            elif character == ")":
                depth -= 1
                if depth == 0:
                    return index + 1
    raise ValueError("unbalanced expression")


def top_level_items(text, heads):
    """(head, block) of the board's own items with one of `heads` (not the
    ones inside footprints)."""
    items, last = [], 0
    pattern = re.compile(r"\((footprint|" + "|".join(heads) + r")[\s\"]")
    for match in pattern.finditer(text):
        if match.start() < last:
            continue
        end = block_end(text, match.start())
        last = end
        if match.group(1) != "footprint":
            items.append((match.group(1), text[match.start():end]))
    return items


def footprints_of(text):
    """Per footprint of a board file: reference, uuid (`uuid`, or `tstamp` in
    KiCad 5 to 7 files) and the symbol path."""
    result, last = [], 0
    for match in re.finditer(r"\(footprint[\s\"]", text):
        if match.start() < last:
            continue
        last = block_end(text, match.start())
        block = text[match.start():last]
        # The footprint's own fields come before its first pad or graphic.
        head = block[:min([i for i in (block.find("(pad "), block.find("(fp_")) if i >= 0] or [len(block)])]
        reference = re.search(r'\((?:property "Reference"|fp_text reference) "([^"]*)"', block)
        uuid = re.search(r'\((?:uuid|tstamp) "?([0-9a-fA-F-]{36})"?\)', head)
        path = re.search(r'\(path "([^"]*)"\)', head)
        layer = re.search(r'\(layer "?([FB])\.Cu"?\)', head)
        result.append({"reference": reference.group(1) if reference else "", "block": block,
                       "uuid": uuid.group(1) if uuid else None,
                       "path": path.group(1) if path else None,
                       "layer": "B.Cu" if layer and layer.group(1) == "B" else "F.Cu"})
    return result


def edge_cuts(text):
    """The board's own Edge.Cuts shapes in KiCad 10's syntax (the import
    writes a KiCad 10 board; KiCad 6 files give a shape's width as
    `(width w)` and its id as `tstamp`). Each shape keeps the designer's
    id: the project's DRC exclusions name the edge by it (urchin's and
    le_chiffre's pads over the edge, `copper_edge_clearance|x|y|<edge>|<pad>`)."""
    shapes = []
    for index, (head, block) in enumerate(top_level_items(text, SHAPES)):
        if not re.search(r'\(layer "?Edge\.Cuts"?\)', block):
            continue
        width = re.search(r"\(width ([-\d.]+)\)", block)
        fields = []
        # KiCad reads a shape's points in its own order (a circle's centre
        # first).
        for key in ("center", "end") if head == "gr_circle" else ("start", "mid", "end"):
            point = re.search(r"\(" + key + r" ([-\d.eE+]+) ([-\d.eE+]+)\)", block)
            if point:
                fields.append(f"({key} {point.group(1)} {point.group(2)})")
        points = re.search(r"\(pts\s", block)
        if points:
            fields.append(" ".join(block[points.start():block_end(block, points.start())].split()))
        fields.append(f"(stroke (width {width.group(1) if width else '0.1'}) (type solid))")
        if head in ("gr_circle", "gr_rect", "gr_poly"):
            fields.append("(fill no)")
        own = re.search(r'\((?:uuid|tstamp) "?([0-9a-fA-F-]{36})"?\)', block)
        uuid = own.group(1) if own else f"5c4e0000-0000-4000-8000-{index:012d}"
        shapes.append(f"\t({head} {' '.join(fields)} (layer \"Edge.Cuts\") (uuid \"{uuid}\"))")
    return shapes


def run(command, timeout=600, env=None):
    try:
        process = subprocess.run([str(part) for part in command], capture_output=True, text=True, timeout=timeout,
                                 env=env)
        return process.returncode, process.stdout, process.stderr
    except subprocess.TimeoutExpired:
        return "timeout", "", ""


def add_libraries(table, libraries, project):
    """Adds (or replaces) `libraries` ({nickname: absolute folder}) in the
    project's fp-lib-table, as `${KIPRJMOD}`-relative entries."""
    text = table.read_text() if table.exists() else "(fp_lib_table\n\t(version 7)\n)\n"
    for nickname in libraries:
        text = re.sub(r'\s*\(lib\s+\(name\s+"?' + re.escape(nickname) + r'"?\)[^\n]*', "", text)
    lines = []
    for nickname, folder in libraries.items():
        relative = posixpath.relpath(str(folder), str(project))
        lines.append(f'\t(lib (name "{nickname}") (type "KiCad") (uri "${{KIPRJMOD}}/{relative}") (options "") '
                     f'(descr "added by the schematic tier"))')
    end = text.rstrip().rfind(")")
    table.write_text(text[:end].rstrip() + "\n" + "\n".join(lines) + "\n)\n")


def import_schematic(binary, record, work, board_id, layers, footprints_from=None):
    """Copies the fetched project and imports its schematic (with
    `--footprints-from` the designer's board, when given). Returns the
    import's report (or an error string), with what was done to it."""
    source = work / "sch"
    shutil.copytree(STORE / record["name"] / "repo", source)
    schematic = source / record["schematic"][len("repo/"):]
    project = work / "project"
    command = [binary, "import-kicad-netlist", schematic, project, board_id, "--layers", str(layers)]
    if footprints_from is not None:
        command += ["--footprints-from", footprints_from]
    steps = {"command": command}
    code, stdout, stderr = run(command)
    (work / "import.log").write_text(stdout + stderr)
    steps["exit"] = code
    if code not in (0, 1) or not stdout.strip():
        return None, steps, (stderr.strip().splitlines() or ["no output"])[-1][:300]
    report = json.loads(stdout)
    steps["missing_first"] = len(report["missing"])
    if report["missing"] and record.get("library_fixes"):
        # What an agent does with `missing` (docs/agents.md, section 0): the
        # library goes into the project's fp-lib-table, then import again
        # into the same directory.
        libraries = {nickname: source / folder for nickname, folder in record["library_fixes"].items()}
        add_libraries(schematic.parent / "fp-lib-table", libraries, schematic.parent)
        steps["libraries_added"] = sorted(libraries)
        code, stdout, stderr = run(command)
        (work / "import-again.log").write_text(stdout + stderr)
        steps["exit"] = code
        if code not in (0, 1) or not stdout.strip():
            return None, steps, (stderr.strip().splitlines() or ["no output"])[-1][:300]
        report = json.loads(stdout)
    (work / "import.json").write_text(json.dumps(report, indent=1))
    return report, steps, None


def import_again(work, steps):
    """Imports into the same project again (after constraints.json got
    `add` entries, which the import puts on the board)."""
    code, stdout, stderr = run(steps["command"])
    (work / "import-add.log").write_text(stdout + stderr)
    steps["exit"] = code
    if code not in (0, 1) or not stdout.strip():
        return None, (stderr.strip().splitlines() or ["no output"])[-1][:300]
    report = json.loads(stdout)
    (work / "import.json").write_text(json.dumps(report, indent=1))
    return report, None


NET = re.compile(r'\(net\s+(?:\d+\s+)?"((?:[^"\\]|\\.)*)"\s*\)')


def pad_nets(block):
    """(number, net or None) of a footprint block's pads."""
    pads = []
    for match in re.finditer(r'\(pad\s+("(?:[^"\\]|\\.)*"|[^\s()]+)', block):
        end = block_end(block, match.start())
        net = NET.search(block[match.start():end])
        name = net.group(1).replace('\\"', '"').replace("\\\\", "\\") if net else None
        pads.append((match.group(1).strip('"'), name))
    return pads


def net_names(designer_footprints, imported_footprints, matched):
    """The imported board's name of each designer net, by the pads of the
    matched footprints (KiCad 6 and 10 name a net of unnamed pins
    differently: Net-(D1-Pad2), Net-(D1-A)). A net no matched pad shows
    keeps its name."""
    votes = {}
    imported = {id(f): f for f in imported_footprints}
    for designer in designer_footprints:
        other = matched.get(designer["uuid"])
        if other is None or id(other) not in imported:
            continue
        theirs = dict(pad_nets(other["block"]))
        for number, net in pad_nets(designer["block"]):
            if net and theirs.get(number):
                votes.setdefault(net, {}).setdefault(theirs[number], 0)
                votes[net][theirs[number]] += 1
    return {net: max(names, key=names.get) for net, names in votes.items()}


def designer_matches(designer_footprints, imported):
    """The imported footprint of each designer footprint (by index) and the
    imported board's name of each designer net: footprints matched by the
    symbol's path, then by a reference the imported board has once and no
    path claimed (a copy of a footprint is not its original)."""
    matched = {}
    by_path = {f["path"]: f for f in imported if f["path"]}
    by_reference = {}
    for footprint in imported:
        by_reference.setdefault(footprint["reference"], []).append(footprint)
    for index, footprint in enumerate(designer_footprints):
        if footprint["path"] and footprint["path"] in by_path:
            matched[index] = by_path[footprint["path"]]
    claimed = {id(other) for other in matched.values()}
    for index, footprint in enumerate(designer_footprints):
        candidates = by_reference.get(footprint["reference"], [])
        if index not in matched and len(candidates) == 1 and id(candidates[0]) not in claimed:
            matched[index] = candidates[0]
            claimed.add(id(candidates[0]))
    names = net_names(designer_footprints, imported,
                      {f["uuid"]: matched[index] for index, f in enumerate(designer_footprints) if index in matched})
    return matched, names


ZONE_NET = re.compile(r'\(net\s+(?:\d+|"((?:[^"\\]|\\.)*)")\s*\)\s*')
ZONE_NET_NAME = re.compile(r'\(net_name\s+"((?:[^"\\]|\\.)*)"\s*\)\s*')


def zones(stripped_text, designer_text, imported_text):
    """The designer's zones as the layout tier keeps them (`stripped_text`:
    the designer's board through `strip-kicad-tracks`, which keeps rule
    areas and the pours that cover a good part of the board, without their
    fill), in KiCad 10's syntax (`(net "GND")`, `uuid`), each pour on the
    imported board's name of its net (a net no matched pad shows keeps its
    name). Without them a 2-layer board was routed without its ground
    pours and a 4-layer one with the router's automatic planes, and the
    designer's keepouts were gone (MokyaLora's GND pour on six layers
    joined the motor's shield pad in the layout tier)."""
    _, names = designer_matches(footprints_of(designer_text), footprints_of(imported_text))
    result = []
    for _, block in top_level_items(stripped_text, ["zone"]):
        # KiCad 6 to 9: `(net 1) (net_name "GND")`; KiCad 10: `(net "GND")`.
        net, name = ZONE_NET.search(block), ZONE_NET_NAME.search(block)
        value = name.group(1) if name else net.group(1) if net and net.group(1) is not None else ""
        body = ZONE_NET_NAME.sub("", ZONE_NET.sub("", block, count=1), count=1)
        body = re.sub(r'\(tstamp\s+"?([0-9a-fA-F-]{36})"?\)', r'(uuid "\1")', body)
        if value:
            mapped = names.get(value.replace('\\"', '"').replace("\\\\", "\\"), None)
            quoted = value if mapped is None else mapped.replace("\\", "\\\\").replace('"', '\\"')
            body = "(zone (net \"" + quoted + "\") " + body[len("(zone"):].lstrip()
        result.append("\t" + body)
    return result


def additions(designer_text, imported_text, board_only):
    """`add` entries (constraints.json) for the designer's footprints with
    nets that the schematic does not have (0xcb-static's footprints for
    other switches, USB_Keypad's plated mounting holes): each taken from
    the designer's board (`from_board`, given to the import with
    `--footprints-from`) and kept where the designer put it (`x`, `y`,
    absolute), its pads on the imported board's names of their nets. A
    reference the designer's board repeats (`REF**`, mounting holes all
    called `M2.5`) or the imported board has gets a new one (`M2.5_1`, the
    board's in `board_reference`). Returns the entries and the
    references left out."""
    designer_footprints = footprints_of(designer_text)
    imported = footprints_of(imported_text)
    references = {}
    for footprint in designer_footprints:
        references[footprint["reference"]] = references.get(footprint["reference"], 0) + 1
    taken = {f["reference"] for f in imported} | set(references)
    by_reference = {}
    for footprint in imported:
        by_reference.setdefault(footprint["reference"], []).append(footprint)
    matched, names = designer_matches(designer_footprints, imported)
    entries, dropped, used = [], [], set()
    wanted = set(board_only)
    for index, footprint in enumerate(designer_footprints):
        if footprint["reference"] not in wanted or index in matched:
            continue
        nets = {number: names.get(net, net) for number, net in pad_nets(footprint["block"]) if net}
        if not nets:
            continue
        at = re.search(r"\(at ([-\d.eE+]+) ([-\d.eE+]+)", footprint["block"])
        if not at:
            dropped.append(f"add {footprint['reference']} (no position)")
            continue
        reference = footprint["reference"]
        part = reference
        if not part or references[reference] > 1 or part in by_reference or part in used:
            base, number = reference or "X", 1
            while f"{base}_{number}" in taken or f"{base}_{number}" in used:
                number += 1
            part = f"{base}_{number}"
        used.add(part)
        entry = {"part": part, "from_board": True, "nets": nets, "x": float(at.group(1)), "y": float(at.group(2)),
                 "origin": "absolute"}
        if part != reference:
            entry["board_reference"] = reference
        entries.append(entry)
    return entries, dropped


def constraints_for_import(task_constraints, description, designer_text, imported_text):
    """The task's constraints (written for the designer's board by
    from_pcbench.py) for the imported board: footprints matched by the
    symbol they come from (`path`), else by reference; parts the task keeps
    where they are (`fixed`, netless and locked parts, which run.py keeps
    too) at the designer's spot by `place`; every part on the designer's
    side (`back`: the task's stack keeps each part's side). Returns the
    constraints and the matching's facts."""
    designer_footprints_in_file = footprints_of(designer_text)
    designer_paths = {f["uuid"]: f["path"] for f in designer_footprints_in_file}
    # The footprint's layer, not describe-kicad-board's `side` (where its
    # pads and body are): urchin's key sockets are front footprints with
    # their pads on B.Cu, and `back` would have flipped them.
    designer_layers = {f["uuid"]: f["layer"] for f in designer_footprints_in_file}
    imported = footprints_of(imported_text)
    by_path = {f["path"]: f for f in imported if f["path"]}
    by_reference = {}
    for footprint in imported:
        by_reference.setdefault(footprint["reference"], []).append(footprint)
    designer = [f for f in description["footprints"] if f["reference"]]
    # The designer's footprints without a symbol and without a net
    # (mounting holes, logos): the task keeps them where they are (run.py
    # fixes netless parts), so they are copied onto the new board, fixed.
    blocks = {f["uuid"]: f["block"] for f in designer_footprints_in_file}
    netless_uuids = {f["uuid"] for f in description["footprints"] if not any(net for _, net in f["pads"])}
    copies = []
    counts = {}
    for footprint in imported:
        counts[footprint["reference"]] = counts.get(footprint["reference"], 0) + 1

    def match(footprint):
        path = designer_paths.get(footprint["uuid"])
        if path and path in by_path:
            return by_path[path]
        candidates = by_reference.get(footprint["reference"], [])
        return candidates[0] if len(candidates) == 1 else None

    def entry(other):
        """The imported footprint as a constraint names it."""
        named = {"part": other["reference"]}
        if counts.get(other["reference"], 0) > 1 and other["uuid"]:
            named["uuid"] = other["uuid"]
        return named

    matched = {}
    board_only = []
    for footprint in designer:
        other = match(footprint)
        if other is None:
            board_only.append(footprint["reference"])
            if footprint["uuid"] in netless_uuids and footprint["uuid"] in blocks:
                copy = {"reference": footprint["reference"], "uuid": footprint["uuid"], "path": None,
                        "block": blocks[footprint["uuid"]]}
                copies.append(copy)
                matched[footprint["uuid"]] = copy
                counts[footprint["reference"]] = counts.get(footprint["reference"], 0) + 1
        else:
            matched[footprint["uuid"]] = other
    renamed = sorted(f"{f['reference']}->{matched[f['uuid']]['reference']}" for f in designer
                     if f["uuid"] in matched and matched[f["uuid"]]["reference"] != f["reference"])
    used = {id(other) for other in matched.values()}
    schematic_only = sorted(f["reference"] for f in imported if id(f) not in used)

    def designer_footprints(reference, uuid=None):
        return [f for f in designer if (uuid is None or f["uuid"] == uuid) and f["reference"] == reference]

    constraints = {"version": 1, "move_all": True}
    places, dropped, seen = [], [], set()
    netless = [f for f in designer if not any(net for _, net in f["pads"])]
    kept = [f for f in designer if any(fnmatch.fnmatchcase(f["reference"], pattern)
                                       for pattern in task_constraints.get("fixed", []))]
    kept += netless + [f for f in designer if f.get("locked")]
    for footprint in kept:
        if footprint["uuid"] in seen:
            continue
        seen.add(footprint["uuid"])
        other = matched.get(footprint["uuid"])
        if other is None:
            # Not in the schematic (board_only says so).
            continue
        x, y, angle = footprint["at"]
        places.append({**entry(other), "x": x, "y": y, "angle": angle % 360, "origin": "absolute"})
    if places:
        constraints["place"] = places
    for kind in ("edge", "rotation"):
        moved = []
        for item in task_constraints.get(kind, []):
            sources = designer_footprints(item["part"], item.get("uuid"))
            if len(sources) != 1:
                dropped.append(f"{kind} {item['part']} ({len(sources)} footprints of that reference)")
                continue
            if sources[0]["uuid"] not in matched:
                continue
            moved.append({**{k: v for k, v in item.items() if k not in ("part", "uuid")},
                          **entry(matched[sources[0]["uuid"]])})
        if moved:
            constraints[kind] = moved
    hollow = []
    for reference in task_constraints.get("hollow", []):
        sources = designer_footprints(reference)
        others = [matched[f["uuid"]]["reference"] for f in sources if f["uuid"] in matched]
        hollow += others
    if hollow:
        constraints["hollow"] = sorted(set(hollow))
    # The copies lie on their side already.
    copy_ids = {id(copy) for copy in copies}
    imported_from = {uuid for uuid, other in matched.items() if id(other) not in copy_ids}
    on_back = {f["uuid"] for f in designer if designer_layers.get(f["uuid"]) == "B.Cu"}
    back = sorted({matched[f["uuid"]]["reference"] for f in designer if f["uuid"] in on_back and f["uuid"] in imported_from})
    front = {matched[f["uuid"]]["reference"] for f in designer if f["uuid"] not in on_back and f["uuid"] in imported_from}
    # A reference on both sides (copies of one part) stays where the import
    # put it: `back` names references, not footprints.
    back = [reference for reference in back if reference not in front]
    if back:
        constraints["back"] = back
    # A library footprint drawn on the back that the designer put on the
    # front.
    front_only = sorted(reference for reference in front - set(back)
                        if any(f["layer"] == "B.Cu" for f in by_reference.get(reference, [])))
    if front_only:
        constraints["front"] = front_only
    facts = {"designer_parts": len(designer), "imported_parts": len(imported), "matched": len(matched) - len(copies),
             "board_only": sorted(board_only), "copied": len(copies), "schematic_only": schematic_only,
             "renamed": renamed, "dropped": dropped}
    return constraints, facts, [board_only_block(copy["block"]) for copy in copies]


def board_only_block(block):
    """A copied footprint marked `board_only` ("not in schematic"), so that
    KiCad's schematic parity check leaves it out, as it does on a designer's
    board where it is so marked."""
    attr = re.search(r"\(attr\b([^()]*)\)", block)
    if attr:
        if "board_only" in attr.group(1).split():
            return block
        return block[:attr.end() - 1] + " board_only" + block[attr.end() - 1:]
    pad = re.search(r"\(pad\s|\(fp_", block)
    at = pad.start() if pad else len(block) - 1
    return block[:at] + "(attr board_only) " + block[at:]


def append_items(board, items):
    """Adds board items (the outline's shapes, copied footprints) at the
    end of the board file."""
    text = board.read_text()
    end = text.rstrip().rfind(")")
    board.write_text(text[:end].rstrip() + "\n" + "\n".join(items) + "\n)\n")
