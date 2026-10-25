#!/usr/bin/env python3
# Copyright (C) 2026 Toit contributors.
"""Runs the agent-style layout tasks of tasks.json.

    benchmarks/agent-tasks/run.py <output> [--only NAME...] [--binary PATH]
    benchmarks/agent-tasks/run.py <output> --tier quick --binary PATH
    benchmarks/agent-tasks/run.py <output> --tier short --binary PATH
    benchmarks/agent-tasks/run.py <output> --tier quick --route-from build/quick30-x782 --binary PATH
    benchmarks/agent-tasks/run.py <output> --tier quick --only A B --splice-from build/quick30-x782.log
    benchmarks/agent-tasks/run.py <output> --tier schematic --binary PATH
    benchmarks/agent-tasks/run.py <output> --tier expansion --jobs 8 --threads 1 --binary PATH
    benchmarks/agent-tasks/run.py <output> --tier designer --binary PATH

Each task copies a real board, strips its tracks, optionally stacks every
movable footprint at one point and removes the outline (a fresh netlist
import), and lays it out with `layout-kicad-board` from the task's
constraints. A task passes when every constraint holds (except the ones the
task lists as geometrically impossible), every connection is routed, and
native KiCad reports no unconnected item and no copper error beyond the
designer's board's count of its type (`pass`). The strict rule
(`pass_strict`, `new_findings_strict`) subtracts only the findings the
designer's board has at the same place: KiCad's DRC of the designer's board
with its tracks, vias and small zones stripped (what the fixed geometry
brings on its own; `strict_findings`), of a type it has at KiCad's cap only
those at the same place, and since rule 4 every finding between items of
one footprint (`footprint_own`); since rule 2 (`strict_rule`) neither a solder mask bridge between one
of our tracks and a designer's mask opening that bridges on the stripped
or the designer's own board already (`strict_mask_openings`, README.md
"How to measure"). `clean_without_subtraction` says the result has no error that counts
before any subtraction. `repairs` is the binary's verdict's (island stubs,
connection-width repair tracks, the pour mode kept).

Each row records the board's peak memory: `peak_rss_mb` for the process
tree (kicad-cli's DRC included, 2.2 GB on any board) and `layout_rss_mb` for
the layout process alone (the route process in route mode).

`--budget-scale` scales the layout's work budget (`total_seconds`,
`move_seconds`, `polish_seconds`; in route mode the router's
`ladder_budget_seconds`) and the harness's timeout (its fixed 300 s for
KiCad's checks stays). `--route-from <sweep>` routes each board's placement
from an earlier sweep (`<sweep>/<task>/layout/placed`) with
`route-kicad-board` instead of laying it out. `--splice-from <log>` copies
the rows of the boards not run (outside `--only`) from an earlier log,
marked `spliced`. `--tier schematic` starts each board from its schematic
instead (`import-kicad-netlist`, schematic.py; rows say where the flow
stopped). A task with `"mode": "route"` (the expansion tier) routes the
designer's own placement: run.py fetches its files at the pinned commit
when they are not there (verified against the task's SHA-256; never
without a licence), strips the tracks as the intake did and routes with
`route-kicad-board` and the task's `budget_seconds` of work (README.md,
"The expansion tier"). `--tier designer` routes every board on the
designer's own placement (designer_admitted; compare.py --gap joins it
with a layout run: README.md, "The designer tier"). Every run
admits each board against the machine's memory ledger (harness_common.admit;
README.md, "How to measure"); `--lock` takes build/quick-tier.lock first and
waits for it (one tier at a time on the machine, the default until
2026-10-22). Every row has `mode` (layout, route or schematic); the run ends with
the pass line and a line of continuous objectives (README.md, "How to
measure"), also written to summary.json."""

import argparse
import fcntl
import hashlib
import json
import math
import os
import re
import shutil
import signal
import subprocess
import sys
import threading
import time
from pathlib import Path, PurePosixPath

HERE = Path(__file__).resolve().parent
sys.path.insert(0, str(HERE))
sys.path.append(str(HERE.parent))
from compare import (by_name, constraints_digest, normalised, normalised_line, objectives,  # noqa: E402
                     objectives_line, result_directory, rows_of, task_digest)
import compare  # noqa: E402
import harness_common  # noqa: E402
from harness_common import COSMETIC, counts_as_error, error_counts, file_sha256  # noqa: E402, F401
import quality  # noqa: E402
import schematic  # noqa: E402
ROOT = HERE.parents[1]
# Benchmark tiers: subsets of a task list with their reasons (README.md).
TIERS = {"quick": HERE / "quick.json", "short": HERE / "short.json", "schematic": HERE / "schematic.json",
         "expansion": HERE / "expansion.json", "designer": HERE / "designer.json"}
# A tier's default parallelism (no --jobs and --threads given): this many
# search threads a board and the physical cores / this many boards at once.
# A layout is the same at any thread count, so this only sets the wall: the
# short tier on x1151 (2026-10-16, under the lock, one run each, all rows
# identical, none cut) took 448 s at 16 x 1, 401 s at 8 x 2, 570 s at 4 x 4
# and 482 s at 6 x 4; its longest board (WLED_Matrix: 433, 339 and 327 s)
# bounds it, and its placement pool and parallel batches gain from a second
# thread (README.md, "How to measure").
TIER_THREADS = 2
# The harness's timeout less this is scaled with the budget: KiCad's checks
# and the file work do not shrink with it.
FIXED_TIMEOUT_SECONDS = 300
# The harness's timeout without --timeout: at least this, else the layout's
# wall-clock guard (KiCadBoardLayoutConfig's wall_guard default, 2 times the
# budget) past the budget plus FIXED_TIMEOUT_SECONDS (scaled_timeout).
DEFAULT_TIMEOUT_SECONDS = 1800
WALL_GUARD = 2.0
# With the rates pinned, a binary stretches its wall-clock guards by the
# pinned rate over the one it measured at its start, at most this much
# (pcb-kicad's MAX_WALL_STRETCH, or PCB_WALL_STRETCH_MAX where that is a
# number of at least 1: wall_stretch_max; docs/router.md, "Determinism and
# the wall clock"): the harness's timeouts allow for it.
WALL_STRETCH_MAX = 4.0
WALL_STRETCH_VARIABLE = "PCB_WALL_STRETCH_MAX"
# The work rates every board runs at unless --work-rate says otherwise
# (PCB_WORK_RATE: search expansions and placement units a second of work
# stands for): the reference's, so that rows compare across machines and
# loads; `measured` lets each binary measure its machine at its start
# (docs/router.md, "Determinism and the wall clock").
REFERENCE_WORK_RATE = "3.15e6,150e6"
# What a copy of a designer's project leaves out (the layout's source, the
# designer tier's placement, the designer's DRC with an old binary).
COPY_IGNORE = shutil.ignore_patterns(".history", "*-backups", "Gerbers", "packages3D", "*.pdf", "*.zip",
                                     "fp-info-cache")


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


def move_zones(block, dx, dy):
    """Shifts the points of a footprint's zones by (dx, dy): they are in
    board coordinates and go with the footprint (MokyaLora's U.FL left its
    keepout at the designer's place, where KiCad found our pads in it)."""
    pieces, last = [], 0
    for match in re.finditer(r"\(zone[\s\n]", block):
        if match.start() < last:
            continue
        end = block_end(block, match.start())
        zone = re.sub(r"\(xy ([-\d.eE+]+) ([-\d.eE+]+)\)",
                      lambda xy: f"(xy {round(float(xy.group(1)) + dx, 6)} {round(float(xy.group(2)) + dy, 6)})",
                      block[match.start():end])
        pieces += [block[last:match.start()], zone]
        last = end
    pieces.append(block[last:])
    return "".join(pieces)


# The board's texts and silkscreen graphics that label a movable part
# (docs/placer.md, "The board's own texts and silkscreen graphics"): the
# binary attaches a mark to the movable part it overlaps at the input, but
# a task stacks its movable parts at one point and parts sharing an origin
# claim nothing. A label left at the designer's spot was then an obstacle
# where the designer's part had been (OpenFC's 17 connectors and headers
# seated on their stale pin names). Moving the label with its part to the
# stack attaches it to nothing either (and blocks the stack's spot), so
# unplace drops such labels (`labels_dropped`); texts over fixed parts and
# over free space stay.
# What unplace does with them, in the rows and summary.json (`labels`):
# "dropped" since 2026-10-23; a row without it kept every label.
LABELS = "dropped"
MARK_LAYERS = ("F.SilkS", "B.SilkS", "F.Cu", "B.Cu")
SILK_LAYERS = ("F.SilkS", "B.SilkS")
MARK_TEXTS = ("gr_text", "gr_text_box")
MARK_GRAPHICS = ("gr_line", "gr_rect", "gr_arc", "gr_circle", "gr_poly", "gr_curve")
# The outline's graphics unplace drops with `remove_outline`.
OUTLINE_HEADS = ("gr_line", "gr_rect", "gr_arc", "gr_circle", "gr_poly")
# board_placer::marks: graphics within CLUSTER_GAP of each other are one
# mark; a cluster spanning more than FRAME_SHARE of the outline's width or
# height (a frame, an image behind every part) is none.
CLUSTER_GAP = 0.5
FRAME_SHARE = 0.8
# A stroke-font glyph's advance per text width (pcbnew's boxes: 0.54 for
# "iiii", 1.2 for "WWWW", 0.82 to 1.1 for labels like "GND" or "3V3").
GLYPH_ADVANCE = 0.95
LINE_PITCH = 1.62
SEXP_TOKEN = re.compile(r'"(?:[^"\\]|\\.)*"|[()]|[^\s()"]+')
TOP_TOKEN = re.compile(r'"(?:[^"\\]|\\.)*"|[()]')


def sexp(text):
    """One s-expression as nested lists; a quoted string keeps its quotes."""
    stack = [[]]
    for token in SEXP_TOKEN.findall(text):
        if token == "(":
            stack.append([])
        elif token == ")":
            done = stack.pop()
            stack[-1].append(done)
        else:
            stack[-1].append(token)
    return stack[0][0]


def top_level(text):
    """(start, end) of every child of the file's root expression."""
    spans, depth, start = [], 0, None
    for match in TOP_TOKEN.finditer(text):
        token = match.group()
        if token == "(":
            depth += 1
            if depth == 2:
                start = match.start()
        elif token == ")":
            if depth == 2:
                spans.append((start, match.end()))
            depth -= 1
    return spans


def child(node, head):
    return next((item for item in node[1:] if isinstance(item, list) and item and item[0] == head), None)


NUMBER = re.compile(r"-?(?:\d+\.?\d*|\.\d+)(?:[eE][-+]?\d+)?$")


def numbers(node, head):
    """The numbers of `node`'s child `head` (`(drill oval 1 2)`: 1, 2)."""
    found = child(node, head)
    return [float(value) for value in found[1:] if isinstance(value, str) and NUMBER.match(value)] if found else []


def unquote(atom):
    return atom[1:-1] if isinstance(atom, str) and atom.startswith('"') else atom


def placed(point, origin, angle):
    """A footprint-frame (or text-frame) point on the board: KiCad's y
    points down and angles turn counterclockwise on the screen."""
    radians = math.radians(angle)
    x, y = point
    return (origin[0] + x * math.cos(radians) + y * math.sin(radians),
            origin[1] - x * math.sin(radians) + y * math.cos(radians))


def box_of(points):
    xs, ys = [p[0] for p in points], [p[1] for p in points]
    return (min(xs), min(ys), max(xs), max(ys))


def grown(box, margin):
    return (box[0] - margin, box[1] - margin, box[2] + margin, box[3] + margin)


def overlap_area(a, b):
    width = min(a[2], b[2]) - max(a[0], b[0])
    height = min(a[3], b[3]) - max(a[1], b[1])
    return width * height if width > 0 and height > 0 else 0.0


def stroke_width(node):
    stroke = child(node, "stroke")
    return (numbers(stroke, "width") or [0.0])[0] if stroke else (numbers(node, "width") or [0.0])[0]


def corners(box):
    return [(box[0], box[1]), (box[2], box[1]), (box[2], box[3]), (box[0], box[3])]


def footprint_geometry(node):
    """(back, origin, body, far): the footprint's side, its origin, the box
    of its courtyard and pads on the board, and the boxes of its holes and
    of its pads with copper on the other side (board_placer's `far_boxes`
    and `far_copper`)."""
    at = numbers(node, "at")
    origin, angle = (at[0], at[1]), at[2] if len(at) > 2 else 0.0
    layer = child(node, "layer")
    back = bool(layer) and unquote(layer[1]).startswith("B.")
    other = "F.Cu" if back else "B.Cu"
    local, far = [], []
    for item in node[1:]:
        if not isinstance(item, list) or not item:
            continue
        if item[0] in ("fp_line", "fp_rect", "fp_arc", "fp_circle", "fp_poly"):
            item_layer = child(item, "layer")
            if not item_layer or not unquote(item_layer[1]).endswith("CrtYd"):
                continue
            if item[0] == "fp_circle":
                center, end = numbers(item, "center"), numbers(item, "end")
                if center and end:
                    radius = math.hypot(end[0] - center[0], end[1] - center[1])
                    local += corners((center[0] - radius, center[1] - radius, center[0] + radius, center[1] + radius))
            elif item[0] == "fp_poly":
                pts = child(item, "pts")
                local += [(float(xy[1]), float(xy[2])) for xy in (pts or [])[1:] if isinstance(xy, list) and xy[0] == "xy"]
            else:
                local += [tuple(numbers(item, key)[:2]) for key in ("start", "mid", "end") if len(numbers(item, key)) >= 2]
        elif item[0] == "pad":
            pad_at, size = numbers(item, "at"), numbers(item, "size")
            if len(pad_at) < 2 or len(size) < 2:
                continue
            drill = numbers(item, "drill")
            width, height = max(size[0], drill[0] if drill else 0.0), max(size[1], drill[-1] if drill else 0.0)
            turn = (pad_at[2] if len(pad_at) > 2 else 0.0) - angle
            pad = [placed(corner, (pad_at[0], pad_at[1]), turn)
                   for corner in corners((-width / 2, -height / 2, width / 2, height / 2))]
            local += pad
            layers = [unquote(value) for value in (child(item, "layers") or [])[1:]]
            if len(item) > 2 and item[2] in ("thru_hole", "np_thru_hole") or "*.Cu" in layers or other in layers:
                far.append(box_of([placed(corner, origin, angle) for corner in pad]))
    if not local:
        return None
    body = box_of([placed(corner, origin, angle) for corner in corners(box_of(local))])
    return back, origin, body, far


def hidden(node):
    for holder in (node, child(node, "effects") or []):
        if "hide" in holder[1:]:
            return True
        flag = child(holder, "hide") if holder else None
        if flag and (len(flag) < 2 or flag[1] != "no"):
            return True
    return False


def text_boxes(node):
    """The box of a board text or text box (its layer, its box) or None."""
    layer = child(node, "layer")
    if not layer or unquote(layer[1]) not in MARK_LAYERS or hidden(node):
        return None
    name = unquote(layer[1])
    if node[0] == "gr_text_box":
        pts = child(node, "pts")
        points = [(float(xy[1]), float(xy[2])) for xy in (pts or [])[1:] if isinstance(xy, list) and xy[0] == "xy"]
        points += [tuple(numbers(node, key)[:2]) for key in ("start", "end") if len(numbers(node, key)) >= 2]
        return (name, box_of(points)) if points else None
    content = next((unquote(atom) for atom in node[1:] if isinstance(atom, str) and atom.startswith('"')), "")
    at = numbers(node, "at")
    if len(at) < 2:
        return None
    effects = child(node, "effects") or []
    font = child(effects, "font") if effects else None
    size = numbers(font, "size") if font else []
    height, width = (size + [1.0, 1.0])[:2] if len(size) < 2 else size[:2]
    thickness = (numbers(font, "thickness") or [0.15])[0] if font else 0.15
    lines = content.split("\\n")
    w = max(len(line) for line in lines) * GLYPH_ADVANCE * width + thickness
    h = height * (1 + LINE_PITCH * (len(lines) - 1)) + thickness
    justify = (child(effects, "justify") or [])[1:] if effects else []
    left = {"left": 0.0, "right": -w}.get(next((j for j in justify if j in ("left", "right")), ""), -w / 2)
    if "mirror" in justify:
        left = -w - left
    top = {"top": 0.0, "bottom": -h}.get(next((j for j in justify if j in ("top", "bottom")), ""), -h / 2)
    angle = at[2] if len(at) > 2 else 0.0
    return name, box_of([placed(corner, (at[0], at[1]), angle) for corner in corners((left, top, left + w, top + h))])


def graphic_boxes(node, layers=SILK_LAYERS):
    """A graphic's layer (one of `layers`) and the boxes it blocks with (a
    rectangle's four sides unless filled), or None."""
    layer = child(node, "layer")
    if not layer or unquote(layer[1]) not in layers:
        return None
    half = stroke_width(node) / 2
    head = node[0]
    if head in ("gr_poly", "gr_curve"):
        pts = child(node, "pts")
        points = [(float(xy[1]), float(xy[2])) for xy in (pts or [])[1:] if isinstance(xy, list) and xy[0] == "xy"]
        return (unquote(layer[1]), [grown(box_of(points), half)]) if points else None
    if head == "gr_circle":
        center, end = numbers(node, "center"), numbers(node, "end")
        if len(center) < 2 or len(end) < 2:
            return None
        radius = math.hypot(end[0] - center[0], end[1] - center[1]) + half
        return unquote(layer[1]), [(center[0] - radius, center[1] - radius, center[0] + radius, center[1] + radius)]
    start, end = numbers(node, "start"), numbers(node, "end")
    if len(start) < 2 or len(end) < 2:
        return None
    if head == "gr_rect":
        x0, y0, x1, y1 = box_of([start[:2], end[:2]])
        fill = child(node, "fill")
        if fill and len(fill) > 1 and fill[1] in ("solid", "yes"):
            return unquote(layer[1]), [grown((x0, y0, x1, y1), half)]
        return unquote(layer[1]), [grown(side, half) for side in
                                   ((x0, y0, x1, y0), (x0, y1, x1, y1), (x0, y0, x0, y1), (x1, y0, x1, y1))]
    points = [tuple(start[:2]), tuple(end[:2])]
    mid = numbers(node, "mid")
    if head == "gr_arc" and len(mid) >= 2:
        points.append(tuple(mid[:2]))
        (ax, ay), (bx, by), (cx, cy) = points[0], points[2], points[1]
        d = 2 * (ax * (by - cy) + bx * (cy - ay) + cx * (ay - by))
        if abs(d) > 1e-9:
            ux = ((ax * ax + ay * ay) * (by - cy) + (bx * bx + by * by) * (cy - ay) + (cx * cx + cy * cy) * (ay - by)) / d
            uy = ((ax * ax + ay * ay) * (cx - bx) + (bx * bx + by * by) * (ax - cx) + (cx * cx + cy * cy) * (bx - ax)) / d
            radius = math.hypot(ax - ux, ay - uy)
            a0, am, a1 = (math.atan2(p[1] - uy, p[0] - ux) for p in ((ax, ay), (bx, by), (cx, cy)))
            sweep = (a1 - a0) % (2 * math.pi)
            if (am - a0) % (2 * math.pi) > sweep:
                sweep -= 2 * math.pi
            points += [(ux + radius * math.cos(a0 + sweep * k / 32), uy + radius * math.sin(a0 + sweep * k / 32))
                       for k in range(33)]
    return unquote(layer[1]), [grown(box_of(points), half)]


def clusters(graphics):
    """Groups (index, layer, boxes) whose boxes lie within CLUSTER_GAP of
    each other on one layer (union-find over a grid)."""
    parent = list(range(len(graphics)))

    def find(i):
        while parent[i] != i:
            parent[i] = parent[parent[i]]
            i = parent[i]
        return i
    cell = 5.0
    buckets = {}
    for index, (_, layer, boxes) in enumerate(graphics):
        for box in boxes:
            g = grown(box, CLUSTER_GAP / 2)
            for gx in range(math.floor(g[0] / cell), math.floor(g[2] / cell) + 1):
                for gy in range(math.floor(g[1] / cell), math.floor(g[3] / cell) + 1):
                    for other, other_box in buckets.get((layer, gx, gy), []):
                        if find(other) != find(index) and within(box, other_box, CLUSTER_GAP):
                            parent[find(other)] = find(index)
                    buckets.setdefault((layer, gx, gy), []).append((index, box))
    groups = {}
    for index in range(len(graphics)):
        groups.setdefault(find(index), []).append(index)
    return list(groups.values())


def within(a, b, gap):
    """board_placer::marks' `overlaps`: the boxes lie within `gap`."""
    return a[0] < b[2] + gap and b[0] < a[2] + gap and a[1] < b[3] + gap and b[1] < a[3] + gap


def labels_of_movable(text, movable_spans):
    """The spans (start, end) of the board's marks that label a movable
    footprint at its designer position, as board_placer's attach_marks
    would at the input, and how many marks they make (a cluster of
    graphics is one). `movable_spans` are the movable footprints' spans."""
    parts = []
    for start, end in movable_spans:
        geometry = footprint_geometry(sexp(text[start:end]))
        if geometry:
            parts.append(geometry)
    # Parts sharing an origin claim nothing (attach_marks).
    parts = [part for part in parts if not any(
        other is not part and abs(other[1][0] - part[1][0]) < 0.01 and abs(other[1][1] - part[1][1]) < 0.01
        for other in parts)]
    if not parts:
        return [], 0
    marks, graphics, outline = [], [], []
    for start, end in top_level(text):
        head = re.match(r"\(([a-z_]+)", text[start:end])
        head = head.group(1) if head else ""
        if head in MARK_TEXTS:
            found = text_boxes(sexp(text[start:end]))
            if found:
                marks.append(([(start, end)], found[0], [found[1]]))
        elif head in MARK_GRAPHICS:
            node = sexp(text[start:end])
            edge = graphic_boxes(node, ("Edge.Cuts",))
            if edge:
                outline += edge[1]
                continue
            found = graphic_boxes(node)
            if found:
                graphics.append(((start, end), found[0], found[1]))
    frame = box_of([(b[0], b[1]) for b in outline] + [(b[2], b[3]) for b in outline]) if outline else None
    for group in clusters(graphics):
        boxes = [box for index in group for box in graphics[index][2]]
        bounds = box_of([(b[0], b[1]) for b in boxes] + [(b[2], b[3]) for b in boxes])
        if frame and (bounds[2] - bounds[0] > FRAME_SHARE * (frame[2] - frame[0])
                      or bounds[3] - bounds[1] > FRAME_SHARE * (frame[3] - frame[1])):
            continue
        marks.append(([graphics[index][0] for index in group], graphics[group[0]][1], boxes))
    dropped, count = [], 0
    for spans, layer, boxes in marks:
        back = layer.startswith("B.")
        for part_back, _, body, far in parts:
            mine = [body] if part_back == back else far
            if any(overlap_area(a, b) > 0 for a in mine for b in boxes):
                dropped += spans
                count += 1
                break
    return dropped, count


def unplace(board, point, remove_outline, keep=()):
    """Stacks every footprint with a net that is not locked (or in `keep`) at
    `point`; drops the Edge.Cuts graphics if asked, and the board's texts
    and silkscreen graphics that label a movable footprint at the
    designer's position (labels_of_movable). Returns the references of the
    footprints without nets, which stay where the designer put them, and
    how many labels were dropped (a cluster of graphics counts once)."""
    text = board.read_text()
    pieces, last, netless = [], 0, []
    pattern = re.compile(r"\((footprint|gr_line|gr_rect|gr_arc|gr_circle|gr_poly|gr_curve|gr_text_box|gr_text)[\s\"]")
    connected = r'\(pad [^\n]*[\s\S]*?\(net (?:\d+ )?"(?!unconnected-)[^"]+"\)'
    reference_pattern = r'\((?:property "Reference"|fp_text reference) "([^"]*)"'
    # The netless footprints stay, fixed by reference: a footprint with a
    # net that shares its reference stays too (Quanta75's mounting holes
    # are all "M2.5", some on GND: stacked, they were held at the stack by
    # the netless ones' "M2.5" and overlapped there).
    staying = set()
    for match in pattern.finditer(text):
        if match.group(1) == "footprint":
            block = text[match.start():block_end(text, match.start())]
            reference = re.search(reference_pattern, block)
            if reference and reference.group(1) and not re.search(connected, block):
                staying.add(reference.group(1))

    def movable(block):
        reference = re.search(reference_pattern, block)
        # A pad's net is `(net "X")` in KiCad 10 files and `(net 5 "X")`
        # before; only the first form was read, so every footprint of an
        # older file counted as netless and stayed fixed (89 of the 109
        # harvested tasks never placed anything until 2026-10-09).
        return re.search(connected, block) and not re.search(
            r"^\(footprint \"[^\"]*\"\s+(?:\(locked (?:yes)?\)|locked)", block) and not (
            reference and (reference.group(1) in keep or reference.group(1) in staying))
    spans = [(match.start(), block_end(text, match.start())) for match in pattern.finditer(text)
             if match.group(1) == "footprint"]
    dropped, labels = labels_of_movable(text, [(s, e) for s, e in spans if movable(text[s:e])])
    dropped = {start for start, _ in dropped}
    for match in pattern.finditer(text):
        start = match.start()
        if start < last:
            continue
        end = block_end(text, start)
        block = text[start:end]
        pieces.append(text[last:start])
        if start in dropped:
            pass
        elif match.group(1) == "footprint":
            reference = re.search(reference_pattern, block)
            if reference and reference.group(1) and not re.search(connected, block):
                netless.append(reference.group(1))
            if movable(block):
                # The footprint's own (at ...) is its first direct child.
                depth = 0
                for index, character in enumerate(block):
                    if character == "(":
                        depth += 1
                        if depth == 2 and block.startswith("(at ", index):
                            close = block.index(")", index)
                            parts = block[index + 4:close].split()
                            angle = f" {parts[2]}" if len(parts) > 2 else ""
                            block = block[:index] + f"(at {point[0]} {point[1]}{angle})" + block[close + 1:]
                            block = move_zones(block, point[0] - float(parts[0]), point[1] - float(parts[1]))
                            break
                    elif character == ")":
                        depth -= 1
            pieces.append(block)
        elif remove_outline and match.group(1) in OUTLINE_HEADS and '"Edge.Cuts"' in block:
            pass
        else:
            pieces.append(block)
        last = end
    pieces.append(text[last:])
    board.write_text("".join(pieces))
    return netless, labels


# KiCad's DRC reports at most this many findings of a type (verdict.rs,
# KICAD_FINDING_CAP): a designer count at or above it is a floor, so the
# strict rule subtracts only the findings it has at the same place.
KICAD_FINDING_CAP = 199
# How far (mm) an item may lie from the designer's for the finding to be at
# the same place (verdict.rs, SAME_PLACE_MM).
SAME_PLACE_MM = 0.05


# The strict rule's version, in every judged row (`strict_rule`) and in
# summary.json; compare.py warns when two runs were judged under different
# ones. 1: the designer's findings at the same place (2026-10-19). 2: also
# solder mask bridges of a designer's opening that bridges on the
# designer's board already (2026-10-21, `strict_mask_openings`). 3: a
# finding with one item is at the same place where its item is, within
# SAME_PLACE_MM, or where it is the same item (not a zone) with the same
# finding word for word; the same uuid alone no longer does, and a zone
# item KiCad places at (0, 0) has no place (2026-10-22, review 2026-10-21
# H2). 4: a finding between items of one footprint only is the reference's
# (`footprint_own`); a finding at the same place is the reference's also
# of a type the reference has at KiCad's cap; a copper_edge_clearance is at
# the same place by its copper item, whichever piece of the outline KiCad
# names beside it (2026-10-23, Batch_1's mounting holes and GND ring).
STRICT_RULE = 4
# The designer's own board's solder mask bridges, kept in a task's work
# directory by designer_findings for the strict rule.
DESIGNER_BRIDGES = "designer-mask-bridges.json"
# KiCad's item descriptions: a graphic or a pad on a mask layer names the
# layer ("Rectangle of J_LCD1 on F.Mask", "PCB text 'RX' on B.Mask"); a
# pad's aperture is the pad's ("Pad 3 [GND] of J2 on F.Cu", "PTH pad 1
# [VCC] of SW4", "NPTH pad of SW4").
MASK_LAYER = re.compile(r"\b[FB]\.Mask\b")
PAD_ITEM = re.compile(r"^(?:N?PTH )?[Pp]ad\b")


def counting_findings(drc):
    """The findings of a DRC report that count (errors, not excluded, not
    cosmetic): (type, [(uuid, (x, y), description, the finding's
    description)]) each."""
    findings = []
    for violation in drc.get("violations", []):
        if not counts_as_error(violation):
            continue
        items = []
        for item in violation.get("items", []):
            position = item.get("pos")
            at = (position["x"], position["y"]) if position else None
            description = item.get("description") or ""
            # KiCad places a zone item at (0, 0) whatever the problem's
            # place: the finding's own position stands in where a report
            # has one (KiCad 10's has none).
            if zone_origin((None, at, description)) and violation.get("pos"):
                at = (violation["pos"]["x"], violation["pos"]["y"])
            items.append((item.get("uuid"), at, description, violation.get("description") or ""))
        findings.append((violation["type"], items))
    return findings


def zone_item(item):
    """Whether a finding's item is a zone (KiCad's description)."""
    return (item[2] if len(item) > 2 else "").startswith("Zone")


def zone_origin(item):
    """Whether a finding's item is a zone KiCad places at (0, 0): a place
    that says nothing about where the problem is."""
    return zone_item(item) and item[1] is not None and tuple(item[1]) == (0.0, 0.0)


def finding_text(item):
    """The description of the finding an item is of (counting_findings)."""
    return item[3] if len(item) > 3 else ""


def located(item):
    """Where a finding's item is, None where KiCad does not say (no
    position, or a zone at (0, 0): zone_origin)."""
    return None if zone_origin(item) else item[1]


def mask_opening(item):
    """Whether a finding's item is a mask opening: a graphic (or a pad
    without copper there) on F.Mask or B.Mask, or a pad (its aperture)."""
    description = item[2] if len(item) > 2 else ""
    if description.startswith(("Track", "Via", "Zone")):
        return False
    return bool(PAD_ITEM.match(description) or MASK_LAYER.search(description))


def bridged_openings(findings):
    """The mask openings of the solder_mask_bridge findings among
    `findings` (counting_findings of a designer's board): [(uuid, at)]."""
    return [(item[0], item[1]) for kind, items in findings or [] if kind == "solder_mask_bridge"
            for item in items if mask_opening(item)]


def prebridged(finding, openings):
    """Whether `finding` is a solder mask bridge between a mask opening
    that bridges on the designer's board already (`openings`, matched by
    uuid or by position within SAME_PLACE_MM) and a track or a zone: the
    bridge is the opening's (the designer's tracks along it stripped, the
    pour shows through; an aperture over pads of several nets bridges
    whatever leaves a pad inside). A via under an opening, a pad of
    another part and two items of ours are not."""
    kind, items = finding
    if kind != "solder_mask_bridge" or len(items) != 2:
        return False
    for index, item in enumerate(items):
        other = items[1 - index]
        other_description = other[2] if len(other) > 2 else ""
        if not mask_opening(item) or not other_description.startswith(("Track", "Zone")):
            continue
        uuid, at = item[0], item[1]
        for theirs, there in openings:
            if uuid is not None and uuid == theirs:
                return True
            if at is not None and there is not None and _distance(at, there) <= SAME_PLACE_MM:
                return True
    return False


# A finding's item that belongs to a footprint names it ("PTH pad 1 [GND]
# of M15", "Segment [<no net>] of AE1 on F.Cu"); a track, a via, a zone
# or a board graphic does not.
FOOTPRINT_ITEM = re.compile(r"\bof (\S+)(?: on [^ ]+)?$")


def footprint_own(finding):
    """Whether a finding is between items of one footprint only (its pads
    and its graphics: a mounting hole's ring of overlapping pads, an
    antenna's copper at its feed pad): the footprint's geometry and its
    pads' nets, which neither a placement nor a route changes (verdict.rs,
    footprint_own)."""
    owners = set()
    for item in finding[1]:
        match = FOOTPRINT_ITEM.search(item[2] if len(item) > 2 else "")
        if match is None:
            return False
        owners.add(match.group(1))
    return len(finding[1]) > 1 and len(owners) == 1


def outline_item(item):
    """Whether a finding's item is a piece of the board outline (KiCad's
    description: "Segment on Edge.Cuts", "Arc of REF** on Edge.Cuts")."""
    return (item[2] if len(item) > 2 else "").endswith("on Edge.Cuts")


def same_place(ours, theirs):
    """Whether finding `ours` is `theirs` at the same place: the same type
    and either the same items (uuids) moved together (the distances between
    them kept: a part's own finding goes with the part), or items at the
    same positions (verdict.rs, same_place). A finding of one item has no
    distances to keep: it is at the same place where its item is within
    SAME_PLACE_MM of theirs (`located`: never a zone KiCad places at
    (0, 0), as for a connection_width on a pour: the designer's 45 on
    USB_Keypad's GND pour answered for any of ours there until rule 3), or
    where it is the same item, not a zone, with the same finding word for
    word, KiCad's measured values included (a pad's annular ring, a
    footprint's courtyard: a property of the item, which moved with its
    part)."""
    if ours[0] != theirs[0] or not ours[1]:
        return False
    if ours[0] == "copper_edge_clearance":
        # The copper item decides: which piece of the outline KiCad names
        # beside it varies (the nearest of several within the clearance).
        mine = [item for item in ours[1] if not outline_item(item)]
        reference = [item for item in theirs[1] if not outline_item(item)]
        if mine and reference and (len(mine), len(reference)) != (len(ours[1]), len(theirs[1])):
            return same_place((ours[0], mine), (theirs[0], reference))
    if len(ours[1]) != len(theirs[1]):
        return False
    if len(ours[1]) == 1:
        mine, reference = ours[1][0], theirs[1][0]
        here, there = located(mine), located(reference)
        if here is not None and there is not None and _distance(here, there) <= SAME_PLACE_MM:
            return True
        text = finding_text(mine)
        return (mine[0] is not None and mine[0] == reference[0] and not zone_item(mine) and bool(text)
                and text == finding_text(reference))
    mine = [item[0] for item in ours[1]]
    reference = [item[0] for item in theirs[1]]
    if None not in mine and None not in reference and sorted(mine) == sorted(reference):
        at_ours = {item[0]: item[1] for item in ours[1]}
        at_theirs = {item[0]: item[1] for item in theirs[1]}
        rigid = True
        for index, first in enumerate(mine):
            for second in mine[index + 1:]:
                points = (at_ours[first], at_ours[second], at_theirs[first], at_theirs[second])
                if None in points:
                    continue
                if abs(_distance(points[0], points[1]) - _distance(points[2], points[3])) > SAME_PLACE_MM:
                    rigid = False
        if rigid:
            return True
    used = [False] * len(theirs[1])
    for item in ours[1]:
        at = item[1]
        if at is None:
            return False
        for index, their_item in enumerate(theirs[1]):
            other = their_item[1]
            if not used[index] and other is not None and _distance(at, other) <= SAME_PLACE_MM:
                used[index] = True
                break
        else:
            return False
    return True


def _distance(a, b):
    return ((a[0] - b[0]) ** 2 + (a[1] - b[1]) ** 2) ** 0.5


def strict_findings(ours, reference, designer=None):
    """Splits our findings (counting_findings) into the reference's at the
    same place and our own: {"new": {type: n}, "preexisting": {type: n},
    "unknown": [types the reference has at KiCad's cap], "mask_openings":
    n}. A finding of one footprint's own items (`footprint_own`) is the
    reference's: the footprint is there too, whether KiCad's capped report
    names it or not. A finding the reference has at the same place is the
    reference's even of a type at KiCad's cap; every other finding of such
    a type is new (its type in "unknown"). `reference` None: no
    baseline, all are new. A solder mask bridge that is not the
    reference's is still not ours when a mask opening of it bridges on the
    reference (the stripped designer board) or on `designer` (the
    designer's own board, tracks and all: shipped bridged) and the other
    item is a track or a zone (`prebridged`); those are counted in
    "mask_openings", in neither "new" nor "preexisting"."""
    if reference is None:
        reference, designer = [], None
    counts = {}
    for kind, _ in reference:
        counts[kind] = counts.get(kind, 0) + 1
    openings = bridged_openings(reference) + bridged_openings(designer)
    new, preexisting, unknown, mask_openings = {}, {}, [], 0
    used = [False] * len(reference)
    for finding in ours:
        kind = finding[0]
        if footprint_own(finding):
            preexisting[kind] = preexisting.get(kind, 0) + 1
            continue
        for index, theirs in enumerate(reference):
            if not used[index] and same_place(finding, theirs):
                used[index] = True
                preexisting[kind] = preexisting.get(kind, 0) + 1
                break
        else:
            if counts.get(kind, 0) >= KICAD_FINDING_CAP:
                if kind not in unknown:
                    unknown.append(kind)
                new[kind] = new.get(kind, 0) + 1
            elif prebridged(finding, openings):
                mask_openings += 1
            else:
                new[kind] = new.get(kind, 0) + 1
    return {"new": new, "preexisting": preexisting, "unknown": unknown, "mask_openings": mask_openings}


class StrictBaseline:
    """The strict rule's baseline for one task: KiCad's DRC of the
    designer's board stripped as run.py strips it (`strip-kicad-tracks`:
    tracks, vias and small zones go; pours and the placement stay), through
    drc-kicad-board's cache. Taken only when a result has errors that count;
    `findings()` is None without a baseline (the task has no designer
    budget, or the check failed: then every error counts)."""

    def __init__(self, directory, board_id, work, binary, enabled=True):
        self.directory, self.board_id, self.work, self.binary = Path(directory), board_id, work, binary
        self.enabled = enabled and binary is not None
        self.taken = False
        self.result = None
        self.error = None

    def findings(self):
        if not self.enabled or self.taken:
            return self.result
        self.taken = True
        project = self.work / "baseline"
        try:
            if not has_drc_command(self.binary):
                self.error = "the binary has no drc-kicad-board"
                return None
            project.mkdir(parents=True, exist_ok=True)
            original = self.directory / f"{self.board_id}.kicad_pcb"
            for extension in (".kicad_pro", ".kicad_dru"):
                sidecar = original.with_suffix(extension)
                if sidecar.is_file():
                    shutil.copyfile(sidecar, project / sidecar.name)
            board = project / original.name
            harness_common.run_group(strip_command(self.binary, original, board),
                                     capture_output=True, check=True, timeout=600)
            report = self.work / "baseline-drc.json"
            harness_common.run_group([str(self.binary), "drc-kicad-board", str(project), self.board_id,
                                      str(report)], capture_output=True, timeout=900)
            self.result = counting_findings(json.loads(report.read_text()))
        except (OSError, ValueError, subprocess.SubprocessError) as error:
            self.error = str(error)[:200]
            self.result = None
        finally:
            shutil.rmtree(project, ignore_errors=True)
        return self.result

    def designer(self):
        """The designer's own board's solder mask bridges (designer_findings
        keeps them as `designer-mask-bridges.json`): counting_findings, or
        None where there is no such file."""
        report = self.work / DESIGNER_BRIDGES
        try:
            return counting_findings(json.loads(report.read_text()))
        except (OSError, ValueError):
            return None


def strict_native(result_directory, baseline):
    """The strict rule on the result's drc.json: strict_findings against the
    baseline (taken only when the result has errors that count), and
    whether it is clean without any subtraction; None without a report."""
    report = result_directory / "drc.json"
    if not report.exists():
        return None
    drc = json.loads(report.read_text())
    ours = counting_findings(drc)
    reference = baseline.findings() if (ours and baseline is not None) else None
    designer = baseline.designer() if (reference is not None and any(
        kind == "solder_mask_bridge" for kind, _ in ours)) else None
    split = strict_findings(ours, reference, designer)
    split["clean"] = not ours and not drc.get("unconnected_items")
    if ours and baseline is not None and baseline.enabled and reference is None:
        split["baseline_error"] = baseline.error or "no baseline"
    return split


def strict_row(row, strict, unexpected=()):
    """The strict rule's fields of a row (after `native` was set)."""
    if strict is None:
        row["pass_strict"] = False
        return
    row["strict_rule"] = STRICT_RULE
    row["new_findings_strict"] = strict["new"]
    if strict["preexisting"]:
        row["preexisting_strict"] = strict["preexisting"]
    if strict.get("mask_openings"):
        # Solder mask bridges of a designer's opening that bridges on the
        # designer's board already (prebridged): not counted, not hidden.
        row["strict_mask_openings"] = strict["mask_openings"]
    if strict["unknown"]:
        row["strict_unknown"] = strict["unknown"]
    if strict.get("baseline_error"):
        row["strict_baseline_error"] = strict["baseline_error"]
    row["clean_without_subtraction"] = strict["clean"]
    # The pass rule's own function (compare.py's), on the fields just set.
    row["pass_strict"] = bool(not unexpected and compare.passes_strict(row))


def pin_work_rate(given, environ):
    """Sets PCB_WORK_RATE in `environ`, which every board's binary inherits:
    `given` (--work-rate), else what it holds, else the reference's;
    `measured` removes it (each binary measures its machine). Returns the
    setting."""
    rate = given or environ.get("PCB_WORK_RATE") or REFERENCE_WORK_RATE
    if rate == "measured":
        environ.pop("PCB_WORK_RATE", None)
    else:
        environ["PCB_WORK_RATE"] = rate
    return rate


def measured_rates(rows):
    """The median of the work rates the rows' binaries measured, and their
    work over wall clock (summary.json)."""
    def median(key):
        values = sorted(row[key] for row in rows if row.get(key) is not None)
        return values[len(values) // 2] if values else None
    return {"work_rate_measured_median": median("work_rate_measured"),
            "placer_work_rate_measured_median": median("placer_work_rate_measured"),
            "work_wall_ratio_median": median("work_wall_ratio")}


def verdict_extras(row, verdict):
    """The binary's repairs and the result-changing environment it echoed
    (verdict.json; binaries since the verdict-integrity change)."""
    if not verdict:
        return
    if verdict.get("repairs") is not None:
        row["repairs"] = verdict["repairs"]
    if verdict.get("environment"):
        row["environment"] = verdict["environment"]
    # The work rates the binary ran at and what it measured, and its work
    # over its wall clock (binaries since the runtime calibration).
    for key in ("work_rate", "placer_work_rate", "work_rate_source", "work_rate_measured",
                "placer_work_rate_measured", "work_wall_ratio", "wall_stretch"):
        if verdict.get(key) is not None:
            row[key] = round(verdict[key], 3) if key == "work_wall_ratio" else verdict[key]


def killed(code):
    """Whether a binary's exit code is none of its verdict's (0 to 4,
    docs/agents.md "Exit codes"): a signal (negative: the OOM killer's -9,
    a crash's -11), a shell's 128 + n, a panic past the verdict. Such a
    run wrote whatever it had at that moment (route mode's best attempt
    before x1500): its files are not a result."""
    return isinstance(code, int) and not isinstance(code, bool) and not 0 <= code <= 4


def killed_text(code, output):
    """The `error` of a killed run."""
    if code < 0:
        try:
            name = signal.Signals(-code).name
        except ValueError:
            name = "?"
        return f"killed by signal {-code} ({name})"
    last = output.strip().splitlines()[-1][:200] if output.strip() else "no output"
    if code > 128:
        return f"exit {code} (a signal {code - 128} through a shell?): {last}"
    return f"exit {code}, none of the verdict's: {last}"


def killed_row(row, code, output):
    """The row of a run that was killed (`killed`): `error` and `killed`
    set, `cut: ["killed"]` (the machine, not the binary, ended it), no
    judging of whatever it left (compare.py counts it as a failure)."""
    row.update(exit=code, killed=True, error=killed_text(code, output), cut=["killed"], **{"pass": False})
    return row


def native(result_directory, allowed=None):
    """KiCad's copper errors and unconnected items on the result; `allowed`
    errors per type (the designer's own board's) are not counted."""
    report = result_directory / "drc.json"
    if not report.exists():
        return None
    drc = json.loads(report.read_text())
    errors = error_counts(drc)
    for kind, count in (allowed or {}).items():
        if kind in errors:
            errors[kind] -= count
            if errors[kind] <= 0:
                del errors[kind]
    return {"errors": errors, "unconnected": len(drc.get("unconnected_items", []))}


_DRC_COMMAND = {}


def has_drc_command(binary):
    """Whether `binary` has `drc-kicad-board` (KiCad's DRC through its DRC
    cache, x802 and later); asked once per binary, so that runs of older
    snapshots keep calling kicad-cli."""
    key = str(binary)
    if key not in _DRC_COMMAND:
        try:
            listed = harness_common.run_group([key, "--help"], capture_output=True, text=True, timeout=60).stdout
        except (OSError, subprocess.TimeoutExpired):
            listed = ""
        _DRC_COMMAND[key] = "drc-kicad-board" in listed
    return _DRC_COMMAND[key]


def designer_findings(directory, board_id, work, binary=None):
    """Copper errors per type on the designer's own routed board, which a
    task from a real board need not do better than (harvested boards come
    with their makers' DRC findings: fiducials near holes, rules tightened
    later). With a binary that has `drc-kicad-board` the check goes through
    its DRC cache (every severity in the report; errors are counted alike)."""
    reference = work / "reference"
    report = reference / "drc.json"
    if binary is not None and has_drc_command(binary):
        reference.mkdir(parents=True, exist_ok=True)
        command, cwd = [str(binary), "drc-kicad-board", str(directory), board_id, str(report)], None
    else:
        shutil.copytree(directory, reference, ignore=COPY_IGNORE)
        command = ["kicad-cli", "pcb", "drc", "--refill-zones", "--severity-error", "--format", "json",
                   "-o", "drc.json", f"{board_id}.kicad_pcb"]
        cwd = reference
    try:
        # A process group of its own: a timeout kills kicad-cli with it.
        harness_common.run_group(command, cwd=cwd, stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL,
                                 timeout=600)
    except (OSError, subprocess.TimeoutExpired):
        return {}
    if not report.exists():
        return {}
    violations = json.loads(report.read_text()).get("violations", [])
    counts = error_counts({"violations": violations})
    # The strict rule's openings that the designer shipped bridged
    # (StrictBaseline.designer): the solder mask bridges alone, small.
    bridges = [violation for violation in violations if violation.get("type") == "solder_mask_bridge"]
    if bridges:
        (work / DESIGNER_BRIDGES).write_text(json.dumps({"violations": bridges}))
    shutil.rmtree(reference, ignore_errors=True)
    return counts



def own_peak_mb(pid):
    """The process's own peak resident set so far (VmHWM), in MB."""
    try:
        with open(f"/proc/{pid}/status") as status:
            for line in status:
                if line.startswith("VmHWM:"):
                    return int(line.split()[1]) / 1024
    except OSError:
        pass
    return 0.0


# One tier at a time on the machine (README.md, "How to measure"): run.py
# takes this lock itself (flock) for a tier run or a whole task list, so the
# discipline no longer depends on each agent writing `flock ... run.py`.
LOCK_FILE = ROOT / "build/quick-tier.lock"


def lock_holders(path):
    """The pids that hold a flock on `path` (from /proc/locks; blocked
    waiters are not listed as holders). Empty when unknown."""
    try:
        status = os.stat(path)
        with open("/proc/locks") as locks:
            lines = locks.readlines()
    except OSError:
        return []
    device = f"{os.major(status.st_dev):02x}:{os.minor(status.st_dev):02x}:{status.st_ino}"
    holders = []
    for line in lines:
        fields = line.split()
        if "->" in fields or len(fields) < 6 or fields[1] != "FLOCK":
            continue
        if fields[5] == device:
            holders.append(int(fields[4]))
    return holders


def ancestors(pid=None):
    """The pids above `pid` (default: this process) up to init."""
    found, pid = [], pid or os.getpid()
    while pid > 1:
        try:
            with open(f"/proc/{pid}/stat") as stat:
                # The command may hold spaces and parentheses: the fields
                # after the last ")" are fixed.
                pid = int(stat.read().rsplit(")", 1)[1].split()[1])
        except (OSError, IndexError, ValueError):
            break
        found.append(pid)
    return found


def describe_process(pid):
    """`pid` and its command line, for the waiting message."""
    try:
        command = Path(f"/proc/{pid}/cmdline").read_bytes().replace(b"\0", b" ").decode(errors="replace").strip()
    except OSError:
        command = "?"
    return f"pid {pid}: {command[:300]}"


def lock_holder_text(path, holders):
    """Who holds the lock: the holders' command lines, else what the
    holder wrote into the lock file (run.py writes its pid and command)."""
    if holders:
        return "; ".join(describe_process(pid) for pid in holders)
    try:
        written = Path(path).read_text().strip()
    except OSError:
        written = ""
    return written[:300] or "unknown (no holder in /proc/locks)"


def take_lock(path=LOCK_FILE, poll_seconds=30, log=None):
    """Takes the machine's tier lock (an exclusive flock on `path`), waiting
    as long as it takes and saying who holds it (again whenever the holder
    changes). Returns the open file descriptor (kept open for the run's
    life; never inherited by the boards' processes), or None when a process
    above this one holds it already (the older `flock
    build/quick-tier.lock run.py ...`). The wait is a blocking flock, as
    flock(1)'s, so that a run waits its turn beside those: a waiter that
    polled would lose every release to them."""
    log = log or (lambda text: print(text, file=sys.stderr, flush=True))
    path = Path(path)
    path.parent.mkdir(parents=True, exist_ok=True)
    descriptor = os.open(path, os.O_RDWR | os.O_CREAT, 0o644)
    started = time.monotonic()
    try:
        fcntl.flock(descriptor, fcntl.LOCK_EX | fcntl.LOCK_NB)
    except BlockingIOError:
        holders = lock_holders(path)
        if set(holders) & set(ancestors()):
            os.close(descriptor)
            log(f"lock {path}: held by this run's caller (pid {', '.join(map(str, holders))})")
            return None
        said = lock_holder_text(path, holders)
        log(f"lock {path}: waiting for {said}")
        done = threading.Event()

        def watch():
            nonlocal said
            while not done.wait(poll_seconds):
                text = lock_holder_text(path, lock_holders(path))
                if text != said:
                    log(f"lock {path}: waiting for {text} (waited {time.monotonic() - started:.0f} s)")
                    said = text

        watcher = threading.Thread(target=watch, daemon=True)
        watcher.start()
        try:
            fcntl.flock(descriptor, fcntl.LOCK_EX)
        finally:
            done.set()
            watcher.join()
    waited = time.monotonic() - started
    os.ftruncate(descriptor, 0)
    os.write(descriptor, f"pid {os.getpid()}: {' '.join(sys.argv)[:1000]}\n".encode())
    log(f"lock {path}: taken" + (f" after {waited:.0f} s" if waited >= 1 else ""))
    return descriptor


def run_measured(command, timeout=None, env=None, on_start=None):
    """Runs `command` with its output captured, in a process group of its
    own. Returns (exit code, or "timeout" after killing its group; output; peak resident set in MB; the
    command's own peak in MB; CPU seconds of the process tree). The first peak is wait4's ru_maxrss: the
    largest of the process and the children it waited for (kicad-cli's DRC
    alone takes 2.2 GB, on any board), for this process alone, so that jobs
    side by side do not see each other's peaks. The second is the
    process's own VmHWM, sampled twice a second: the router's memory.
    `on_start` gets the pid."""
    # A process group of its own: a timeout kills the binary and its
    # kicad-cli together, and whatever the binary leaves behind is killed
    # when it ends (end_group).
    process = subprocess.Popen(command, stdout=subprocess.PIPE, stderr=subprocess.STDOUT, env=env,
                               start_new_session=True)
    if on_start:
        on_start(process.pid)
    chunks = []
    reader = threading.Thread(target=lambda: chunks.append(process.stdout.read()), daemon=True)
    reader.start()
    deadline = None if timeout is None else time.monotonic() + timeout
    own = 0.0
    while True:
        own = max(own, own_peak_mb(process.pid))
        pid, status, usage = os.wait4(process.pid, os.WNOHANG)
        if pid:
            code = os.waitstatus_to_exitcode(status)
            break
        if deadline is not None and time.monotonic() > deadline:
            harness_common.kill_group(process.pid)
            _, status, usage = os.wait4(process.pid, 0)
            code = "timeout"
            break
        time.sleep(0.5)
    # Reaped here: Popen must not wait for it again.
    process.returncode = code if isinstance(code, int) else -9
    # The rest of its group (a timed-out board's kicad-cli, a child the
    # binary left running): killed, and waited for until it is gone, so
    # that the memory gate admits the next board only once this board's
    # whole tree has ended.
    harness_common.end_group(process.pid)
    reader.join(60)
    output = (chunks[0] if chunks else b"").decode(errors="replace")
    return code, output, round(usage.ru_maxrss / 1024), round(own), round(usage.ru_utime + usage.ru_stime, 1)


class MemoryGate:
    """Admits a board against the machine's memory ledger
    (harness_common.admit: the live reservations of every run on the
    machine and this board's expected peak within the machine's memory
    less the ledger's margin, and MemAvailable less what the reserving
    runs are still expected to grow by leaving `minimum_gb`), and reserves
    the expected peak for it there at once, under the ledger's lock. The
    reservation is handed to the board's binary when it starts (`started`:
    it then lives as long as that process; a binary since x1590 replaces
    it with its own, which grows with the router's maps and KiCad's
    check) and released when the board is done. Until 2026-10-22 the
    reservations were this process's alone (other runs on the machine did
    not see them) and only MemAvailable counted. A board that has waited
    `drain_after` seconds has the ledger drain for it (harness_common.
    drain_allows; --ledger-drain-after); `priority` "nightly" (--ledger-
    priority nightly) puts its boards first among the waiters and has the
    ledger drain for them at once. A board's reservation carries its
    expected end (the tier's seconds), which the drain reads."""

    def __init__(self, minimum_gb, ledger=None, poll_seconds=15.0, drain_after=None, priority=None):
        self.minimum_gb = max(0.0, minimum_gb)
        self.ledger = ledger if ledger is not None else harness_common.Ledger.from_env()
        self.poll_seconds = poll_seconds
        self.drain_after = harness_common.DEFAULT_DRAIN_AFTER_SECONDS if drain_after is None else drain_after
        self.priority = priority
        self.lock = threading.Lock()
        # token -> the reservation's key in the ledger
        self.keys = {}
        self.next_token = 0

    def admit(self, what, expected_mb, expected_seconds=None):
        key = harness_common.admit(self.ledger, expected_mb, f"run.py: {what} (expected peak)",
                                   self.minimum_gb * 1024, poll_seconds=self.poll_seconds,
                                   log=lambda text: print(text, file=sys.stderr, flush=True),
                                   expected_seconds=expected_seconds, drain_after=self.drain_after,
                                   priority=self.priority)
        with self.lock:
            token = self.next_token
            self.next_token += 1
            self.keys[token] = key
        return token

    def started(self, token, pid):
        if token is not None:
            with self.lock:
                key = self.keys.get(token)
            if key is not None:
                moved = self.ledger.move(key, pid)
                with self.lock:
                    self.keys[token] = moved

    def done(self, token):
        if token is not None:
            with self.lock:
                key = self.keys.pop(token, None)
            self.ledger.release(key)


def order_tasks(tasks, order, tier, expected_mb):
    """The tasks in the order their boards start: `order` seconds-desc
    (only for a tier: a task list keeps its order) the longest expected
    first, so that the last job is not a slow one; peak-desc the largest
    expected peak first (a task without one: `expected_mb`), then the
    longest. Python's sort is stable: ties keep the list's order."""
    if order == "peak-desc":
        return sorted(tasks, key=lambda task: (-task.get("peak_rss_mb", expected_mb),
                                               -(task.get("expected_seconds") or 0)))
    if tier:
        return sorted(tasks, key=lambda task: -(task.get("expected_seconds") or 0))
    return tasks


def task_directory(task):
    """The task's board directory: relative to the checkout, or absolute.
    The committed lists (tasks/v6, tasks/v7) name the main checkout's
    absolute paths, kept so that their SHA-256 stays the reference logs';
    where such a path is not there (another machine, CI), the same
    benchmarks/real/external path under this checkout is used."""
    directory = Path(task["directory"])
    if not directory.is_absolute():
        return ROOT / directory
    if not directory.exists():
        parts = directory.parts
        for index in range(len(parts) - 2):
            if parts[index:index + 3] == ("benchmarks", "real", "external"):
                candidate = ROOT.joinpath(*parts[index:])
                if candidate.exists():
                    return candidate
    return directory


def run_task(task, arguments):
    # Admitted before the designer's DRC: kicad-cli takes 2.2 GB on any
    # board, and twelve jobs start with it at once.
    token = arguments.gate.admit(task["name"], task.get("peak_rss_mb", arguments.expected_mb),
                                 task.get("expected_seconds"))
    try:
        row = run_admitted(task, arguments, token)
    finally:
        arguments.gate.done(token)
    if arguments.quality:
        add_quality(row, task, arguments)
    return row


def add_quality(row, task, arguments):
    """--quality: quality.py's designer-vs-ours measures (score-kicad-board
    on the designer's board and on the result, about 0.1 s each) into the
    row's `quality` ({"designer": ..., "ours": ...}); quality.py and
    compare.py's normalised line read them."""
    if row.get("native") is None:
        return
    result = result_directory(row, arguments.output)
    designer = quality.score(arguments.binary, task_directory(task), task["board_id"])
    ours = quality.score(arguments.binary, result, task["board_id"])
    row["quality"] = {"designer": quality.measures(designer) if designer else None,
                      "ours": quality.measures(ours) if ours else None}


# The layout's budgets (KiCadBoardLayoutConfig's defaults), scaled by
# --budget-scale; the router's ladder budget in route mode.
LAYOUT_BUDGETS = {"total_seconds": 1500.0, "move_seconds": 600.0, "polish_seconds": 60.0}
ROUTE_BUDGET = ("ladder_budget_seconds", 1200.0)


def wall_stretch_max(environ=None):
    """The binary's cap on its stretch, as it reads it (work_rate.rs,
    wall_stretch_cap): PCB_WALL_STRETCH_MAX when it is a finite number of
    at least 1, else WALL_STRETCH_MAX. The boards inherit run.py's
    environment."""
    text = (os.environ if environ is None else environ).get(WALL_STRETCH_VARIABLE)
    try:
        cap = float(text.strip()) if text is not None else None
    except ValueError:
        cap = None
    return cap if cap is not None and cap >= 1.0 and cap != float("inf") else WALL_STRETCH_MAX


def wall_stretch_allowance(arguments):
    """How far a binary may stretch its wall-clock guards: wall_stretch_max
    with the rates pinned (--work-rate other than `measured`), else 1."""
    return 1.0 if getattr(arguments, "work_rate", "measured") == "measured" else wall_stretch_max()


def scaled_timeout(arguments, budget=None, guard=None):
    """The harness's timeout for a board: --timeout as given (beyond its
    fixed 300 s scaled with the budget), else the layout's wall-clock guard
    past the board's budget in work (`budget`: the layout's total_seconds,
    route mode's ladder budget; default the layout's, scaled), stretched as
    far as the binary may stretch it (`wall_stretch_allowance`), plus the
    fixed 300 s, at least 1800 s. The guard (`guard`, default WALL_GUARD,
    the layout's wall_guard) is a load margin: the harness waits for it."""
    if arguments.timeout_given:
        if arguments.budget_scale == 1:
            return arguments.timeout
        fixed = min(FIXED_TIMEOUT_SECONDS, arguments.timeout)
        return round(fixed + (arguments.timeout - fixed) * arguments.budget_scale)
    if budget is None:
        budget = LAYOUT_BUDGETS["total_seconds"] * arguments.budget_scale
    stretched = budget * (guard or WALL_GUARD) * wall_stretch_allowance(arguments)
    return round(max(DEFAULT_TIMEOUT_SECONDS, stretched + FIXED_TIMEOUT_SECONDS))


def run_admitted(task, arguments, token):
    if arguments.schematic:
        return schematic_admitted(task, arguments, token)
    if arguments.designer:
        return designer_admitted(task, arguments, token)
    if arguments.route_from:
        return route_admitted(task, arguments, token)
    if task.get("mode") == "route":
        return route_task_admitted(task, arguments, token)
    name = task["name"]
    directory = task_directory(task)
    work = arguments.output / name
    if work.exists():
        shutil.rmtree(work)
    source = work / "source"
    shutil.copytree(directory, source, ignore=COPY_IGNORE)
    board = source / f"{task['board_id']}.kicad_pcb"
    designer_started = time.monotonic()
    allowed = designer_findings(directory, task["board_id"], work, arguments.binary) if task.get("designer_budget") else {}
    designer_seconds = round(time.monotonic() - designer_started, 1)
    baseline = StrictBaseline(directory, task["board_id"], work, arguments.binary, bool(task.get("designer_budget")))
    harness_common.run_group(strip_command(arguments.binary, board, board), capture_output=True, check=True)
    # Constraints are optional: without them the layout gets an empty set.
    constraints = json.loads((arguments.tasks.parent / task["constraints"]).read_text()) \
        if task.get("constraints") else {}
    labels_dropped = None
    if task.get("unplace"):
        outline = constraints.get("outline", {})
        point = task.get("stack_at") or [outline.get("x", 0) + outline.get("width", 40) / 2,
                                          outline.get("y", 0) + outline.get("height", 40) / 2]
        netless, labels_dropped = unplace(board, point, task.get("remove_outline", False),
                                          set(constraints.get("fixed", [])))
        # move_all would move them too.
        # Fixed parts are glob patterns: references like KiCad's REF** are
        # escaped to match only themselves.
        escaped = [re.sub(r"([*?\\])", r"\\\1", r) for r in netless]
        constraints["fixed"] = constraints.get("fixed", []) + [r for r in escaped if r not in constraints.get("fixed", [])]
    (source / "constraints.json").write_text(json.dumps(constraints))
    # What the layout got: compare.py marks a board whose prepared
    # constraints differ between two runs (another task list, or run.py's
    # rules) instead of comparing it.
    constraints_sha256 = constraints_digest(constraints)
    layout = {"placer": {"constraints": "constraints.json", **task.get("placer", {}), **arguments.placer},
              **task.get("layout", {}), **arguments.layout}
    if arguments.budget_scale != 1:
        # The layout's budgets in work (docs/router.md, "Determinism and the
        # wall clock"); the defaults are KiCadBoardLayoutConfig's.
        for key, default in LAYOUT_BUDGETS.items():
            layout[key] = round(layout.get(key, default) * arguments.budget_scale)
    (work / "layout.json").write_text(json.dumps(layout))
    router = task.get("router") or {}
    router_argument = "auto"
    if router:
        (work / "router.json").write_text(json.dumps(router))
        router_argument = str(work / "router.json")
    env = None
    if arguments.threads:
        # Search threads of the router (rayon): each holds a search scratch
        # of the board's size, so jobs side by side want few each.
        env = dict(os.environ, RAYON_NUM_THREADS=str(arguments.threads))
    started = time.monotonic()
    command = [str(arguments.binary), "layout-kicad-board", str(source), task["board_id"],
               str(work / "layout"), router_argument, str(work / "layout.json")]
    load_before = os.getloadavg()[0]
    timeout = scaled_timeout(arguments, layout.get("total_seconds", LAYOUT_BUDGETS["total_seconds"]),
                             layout.get("wall_guard"))
    code, output, peak_mb, own_mb, cpu_seconds = run_measured(command, timeout, env,
                                                              lambda pid: arguments.gate.started(token, pid))
    (work / "layout.log").write_text(output)
    # CPU seconds are the speed measure: other work on the machine (sweeps,
    # Florian's other projects) stretches the wall clock, not the CPU time.
    # The load averages say how loaded the machine was for this row.
    row = {"name": name, "seconds": round(time.monotonic() - started, 1), "cpu_seconds": cpu_seconds,
           "load": [round(load_before, 1), round(os.getloadavg()[0], 1)],
           "peak_rss_mb": peak_mb, "layout_rss_mb": own_mb, "mode": "layout",
           "constraints_sha256": constraints_sha256}
    if labels_dropped is not None:
        # The labels of movable parts unplace took off the board (LABELS).
        row.update(labels=LABELS, labels_dropped=labels_dropped)
    if arguments.budget_scale != 1:
        row["budget_scale"] = arguments.budget_scale
    if code == "timeout":
        # The harness's limit cut the run: what it would have done depends
        # on the machine's load.
        row.update(error=f"timed out after {timeout} s", cut=["timeout"], **{"pass": False})
        return row
    if killed(code):
        return killed_row(row, code, output)
    return layout_result(row, task, work, code, output, allowed, designer_seconds, baseline=baseline,
                         total_seconds=layout.get("total_seconds", LAYOUT_BUDGETS["total_seconds"]))


# A line of the layout's log after "layout: routed (" that only its final
# ladder prints: an attempt, the ladder's own lines, the narrow-signal
# attempt that stands in for it.
FINAL_LADDER_LINE = re.compile(r"^(attempt |ladder:|layout: \d+ s of work left: the narrow)", re.M)
POOL_EXCESS = re.compile(r"layout: placed \(.*?; (\d+(?:\.\d+)?) s more of it in the pool of seeds")


def final_ladder_budget(result, output, total):
    """The work the layout's final ladder had (board_layout.rs: the layout's
    `total_seconds` less its work so far and the pool of seeds' excess),
    None where no final ladder ran, and where the number is from: the
    report's `final_ladder_budget_seconds` (x1500 and later), else the
    layout's log (`output`: the report's work at "layout: routed" and the
    "placed" line's pool excess)."""
    if "final_ladder_budget_seconds" in result:
        return result["final_ladder_budget_seconds"], "report"
    parts = output.split("layout: routed (", 1)
    if len(parts) < 2 or not FINAL_LADDER_LINE.search(parts[1]) or result.get("work_seconds") is None:
        return None, "log"
    excess = POOL_EXCESS.findall(output)
    return max(0.0, total - result["work_seconds"] - (float(excess[-1]) if excess else 0.0)), "log"


def layout_result(row, task, work, code, output, allowed, designer_seconds, drc_directory=None, baseline=None,
                  total_seconds=None):
    """The row of a layout that ended (not by the harness's timeout): the
    binary's verdict, the report's numbers and the pass rule. KiCad's
    findings come from `drc_directory`/drc.json (default: the result's).
    `total_seconds` is the layout's budget (for the final ladder's share)."""
    report = work / "layout/board-layout.json"
    # The binary's own verdict (docs/agents.md, "Exit codes"): its status
    # and exit code go into the row; the pass rule below stays the
    # runner's (it knows the task's expected misses and the designer's
    # findings).
    row["exit"] = code
    verdict_file = work / "layout/verdict.json"
    verdict = json.loads(verdict_file.read_text()) if verdict_file.exists() else None
    if verdict:
        row["verdict"] = verdict.get("status")
    verdict_extras(row, verdict)
    if not report.exists():
        if verdict:
            # Parts without a legal place, or an error: the verdict says which.
            row["error"] = verdict.get("reason", "")[:300]
            if verdict.get("cut"):
                row["cut"] = verdict["cut"]
        else:
            row["error"] = output.strip().splitlines()[-1][:300] if output.strip() else "no output"
        row["pass"] = False
        return row
    result = json.loads(report.read_text())
    routed = result["routed"]
    expected = {tuple(miss) for miss in task.get("expected_misses", [])}
    missed = [(c["kind"], c["part"], c.get("other")) for c in result["constraints"] if not c["satisfied"]]
    unexpected = [miss for miss in missed if miss not in expected]
    copper = native(drc_directory or work / "layout/result", allowed)
    if allowed:
        row["designer_findings"] = allowed
    if task.get("designer_budget"):
        row["designer_drc_seconds"] = designer_seconds
    # What the wall clock cut short rather than the work (the layout's
    # deadline under load, the route-mode guard): such a row depends on the
    # machine's load and is no evidence for an A/B; without it the same
    # binary and thread count give the same board.
    if result.get("cut"):
        row["cut"] = result["cut"]
    row.update(
        routed=f"{routed['routed_connections']}/{routed['routable_connections']}",
        vias=routed["vias"], length_mm=round(routed["length_mm"], 1),
        constraints=len(result["constraints"]), missed=missed, unexpected_misses=unexpected,
        native=copper,
    )
    # The layout's work (its budget's measure: the verdict's, the final
    # ladder's included; the report's own stops before that ladder) and
    # the router's count of unconnected terminals after the first route
    # (before the moves and the final ladder; the router's count, not
    # KiCad's).
    work_seconds = (verdict or {}).get("work_seconds")
    if work_seconds is None:
        work_seconds = result.get("work_seconds")
    if work_seconds is not None:
        row["work_seconds"] = round(work_seconds, 1)
    # The part of it that is KiCad's checks inside the layout (a modelled
    # cost per check, the same cached or not; binaries since x1080).
    check_work = (verdict or {}).get("kicad_check_work_seconds")
    if check_work is not None:
        row["kicad_check_work_seconds"] = round(check_work, 1)
    if result.get("first_unconnected_terminals") is not None:
        row["first_unconnected_terminals"] = result["first_unconnected_terminals"]
    # The final ladder's work and its share of the layout's budget (review
    # 2026-10-21, H3): a route on the designer's placement at this share
    # (run.py --tier designer --budget-scale) gets the routing work this
    # layout's last router got. None where no final ladder ran.
    total = total_seconds or LAYOUT_BUDGETS["total_seconds"]
    ladder, source = final_ladder_budget(result, output, total)
    row["ladder_budget_seconds"] = None if ladder is None else round(ladder, 1)
    row["ladder_budget_share"] = None if ladder is None else round(ladder / total, 3)
    row["ladder_budget_source"] = source
    # A board passes on KiCad's connectivity, not on the router's own count
    # (Florian, 2026-10-07): KiCad joins a pad through a pour's fill that
    # the router, routing pours as tracks, does not credit (MokyaLora's
    # one open pad, 261/262 with KiCad at 0 unconnected).
    row["pass"] = compare.passes(row)
    strict_row(row, strict_native(drc_directory or work / "layout/result", baseline), unexpected)
    return row


def schematic_admitted(task, arguments, token):
    """The schematic tier (`--tier schematic`, schematic.py): the board
    imported from its schematic with `import-kicad-netlist`, the task's
    constraints moved onto it, laid out and checked with `drc-kicad-board`.
    The pass rule is the layout's, and every footprint must have been
    found; the row says where the flow stopped and why."""
    name = task["name"]
    directory = task_directory(task)
    board_id = task["board_id"]
    work = arguments.output / name
    if work.exists():
        shutil.rmtree(work)
    work.mkdir(parents=True)
    row = {"name": name, "mode": "schematic"}
    if arguments.budget_scale != 1:
        row["budget_scale"] = arguments.budget_scale

    def stop(stage, why):
        row.update(stopped=stage, why=why, **{"pass": False})
        return row

    record_file = schematic.STORE / name / "schematic.json"
    if not record_file.exists():
        return stop("import", "no schematic fetched (benchmarks/github/harvest_schematics.py)")
    record = json.loads(record_file.read_text())
    designer_file = directory / f"{board_id}.kicad_pcb"
    code, stdout, stderr = schematic.run([arguments.binary, "describe-kicad-board", directory, board_id])
    if code != 0:
        return stop("constraints", f"describe-kicad-board on the designer's board: {stderr.strip()[-200:]}")
    description = json.loads(stdout)
    designer_started = time.monotonic()
    allowed = designer_findings(directory, board_id, work, arguments.binary) if task.get("designer_budget") else {}
    designer_seconds = round(time.monotonic() - designer_started, 1)
    baseline = StrictBaseline(directory, board_id, work, arguments.binary, bool(task.get("designer_budget")))
    started = time.monotonic()
    load_before = os.getloadavg()[0]
    # The designer's board gives the footprints no library has, and its
    # versions of the others (import-kicad-netlist --footprints-from).
    report, steps, error = schematic.import_schematic(arguments.binary, record, work, board_id,
                                                      len(description["copper_layers"]), designer_file)
    row["import"] = {key: value for key, value in steps.items() if key != "command"}
    if error:
        return stop("import", f"import-kicad-netlist: {error}")
    project = work / "project"
    board = project / f"{board_id}.kicad_pcb"
    try:
        task_constraints = json.loads((arguments.tasks.parent / task["constraints"]).read_text())
        designer_text = designer_file.read_text()
        constraints, facts, copies = schematic.constraints_for_import(task_constraints, description, designer_text,
                                                                      board.read_text())
        # The designer's parts with nets that the schematic does not have
        # (alternative switch footprints, plated mounting holes): `add`
        # entries, which the import puts on the board.
        added, add_dropped = schematic.additions(designer_text, board.read_text(), facts["board_only"])
        if added:
            (project / "constraints.json").write_text(json.dumps({"version": 1, "add": added}, indent=1))
            report, error = schematic.import_again(work, steps)
            row["import"]["exit"] = steps["exit"]
            if error:
                return stop("import", f"import-kicad-netlist with add: {error}")
            constraints, facts, copies = schematic.constraints_for_import(task_constraints, description,
                                                                          designer_text, board.read_text())
            constraints["add"] = added
        facts["dropped"] += add_dropped
        shapes = schematic.edge_cuts(designer_text)
        if not shapes:
            raise ValueError("the designer's board draws no outline of its own on Edge.Cuts")
        # The designer's pours and rule areas, as the layout tier keeps
        # them (strip-kicad-tracks: the large pours and every rule area,
        # unfilled).
        stripped = work / "designer-stripped.kicad_pcb"
        strip_code, _, strip_err = schematic.run([arguments.binary, "strip-kicad-tracks", designer_file, stripped])
        if strip_code != 0:
            raise ValueError(f"strip-kicad-tracks on the designer's board: {strip_err.strip()[-200:]}")
        pours = schematic.zones(stripped.read_text(), designer_text, board.read_text())
        schematic.append_items(board, shapes + pours + copies)
    except Exception as error:  # noqa: BLE001
        return stop("constraints", f"{type(error).__name__}: {error}")
    row.update(missing=len(report["missing"]), substituted=len(report["substituted"]),
               unmatched_pins=len(report["unmatched_pins"]),
               from_board=len(report.get("footprints_from_board") or []),
               board_versions=report.get("board_versions") or 0,
               drifted=len(report.get("drifted_footprints") or []))
    if report["missing"]:
        row["missing_parts"] = report["missing"][:12]
    if report.get("drifted_footprints"):
        row["drifted_footprints"] = report["drifted_footprints"][:12]
    row["zones"] = len(pours)
    row["parts"] = {"designer": facts["designer_parts"], "imported": facts["imported_parts"],
                    "matched": facts["matched"], "copied": facts["copied"], "renamed": len(facts["renamed"]),
                    "added": len(report.get("added") or [])}
    for key in ("board_only", "schematic_only", "dropped"):
        if facts[key]:
            row["parts"][key] = facts[key][:12] + ([f"... {len(facts[key])} in all"] if len(facts[key]) > 12 else [])
    (project / "constraints.json").write_text(json.dumps(constraints, indent=1))
    row["constraints_sha256"] = constraints_digest(constraints)
    layout = {"placer": {"constraints": "constraints.json", **task.get("placer", {}), **arguments.placer},
              **task.get("layout", {}), **arguments.layout}
    if arguments.budget_scale != 1:
        for key, default in LAYOUT_BUDGETS.items():
            layout[key] = round(layout.get(key, default) * arguments.budget_scale)
    (project / "layout.json").write_text(json.dumps(layout))
    router = task.get("router") or {}
    router_argument = "auto"
    if router:
        (work / "router.json").write_text(json.dumps(router))
        router_argument = str(work / "router.json")
    env = dict(os.environ, RAYON_NUM_THREADS=str(arguments.threads)) if arguments.threads else None
    command = [str(arguments.binary), "layout-kicad-board", str(project), board_id, str(work / "layout"),
               router_argument, str(project / "layout.json")]
    timeout = scaled_timeout(arguments, layout.get("total_seconds", LAYOUT_BUDGETS["total_seconds"]),
                             layout.get("wall_guard"))
    code, output, peak_mb, own_mb, cpu_seconds = run_measured(command, timeout, env,
                                                              lambda pid: arguments.gate.started(token, pid))
    (work / "layout.log").write_text(output)
    row.update(seconds=round(time.monotonic() - started, 1), cpu_seconds=cpu_seconds,
               load=[round(load_before, 1), round(os.getloadavg()[0], 1)], peak_rss_mb=peak_mb,
               layout_rss_mb=own_mb)
    if code == "timeout":
        row.update(error=f"timed out after {timeout} s", cut=["timeout"])
        placed = (work / "layout/placed" / f"{board_id}.kicad_pcb").exists()
        return stop("routing" if placed else "placement", f"the harness's timeout ({timeout} s)")
    if killed(code):
        killed_row(row, code, output)
        placed = (work / "layout/placed" / f"{board_id}.kicad_pcb").exists()
        return stop("routing" if placed else "placement", row["error"])
    # The documented check of the result: drc-kicad-board (KiCad's DRC
    # through its cache; the layout's own DRC ran on the same board).
    result = work / "layout/result"
    if (result / f"{board_id}.kicad_pcb").exists():
        (work / "drc").mkdir()
        drc_code, drc_out, drc_err = schematic.run([arguments.binary, "drc-kicad-board", result, board_id,
                                                    work / "drc/drc.json"])
        row["drc_line"] = (drc_out.strip().splitlines() or [""])[-1][:200]
    layout_result(row, task, work, code, output, allowed, designer_seconds, drc_directory=work / "drc",
                  baseline=baseline, total_seconds=layout.get("total_seconds", LAYOUT_BUDGETS["total_seconds"]))
    verdict_file = work / "layout/verdict.json"
    verdict = json.loads(verdict_file.read_text()) if verdict_file.exists() else {}
    if verdict.get("schematic_parity_issues") is not None:
        row["parity"] = verdict["schematic_parity_issues"]
        row["parity_by_type"] = verdict.get("schematic_parity_by_type")
    # Every footprint found: the pass rules' own condition (compare.passes,
    # compare.passes_strict: the row's `missing`, set above), not again here.
    # Where the flow stopped: the first stage that fell short.
    if report["missing"]:
        row.update(stopped="import", why=f"{len(report['missing'])} footprints missing: "
                   + "; ".join(report["missing"][:3])[:300])
    elif code == 2:
        row.update(stopped="constraints", why=verdict.get("reason", row.get("error", ""))[:300])
    elif "routed" not in row:
        row.update(stopped="placement", why=row.get("error", "no layout report")[:300])
    elif row.get("unexpected_misses"):
        row.update(stopped="placement", why="constraints missed: " + ", ".join(
            " ".join(str(part) for part in miss if part) for miss in row["unexpected_misses"][:5]))
    elif row.get("native") is None:
        row.update(stopped="drc", why="drc-kicad-board wrote no report: " + row.get("drc_line", ""))
    elif row["native"]["unconnected"]:
        # KiCad's connectivity, as the pass rule: a pad the router leaves
        # open that a pour joins is connected (MokyaLora's motor shield,
        # 261/262 with KiCad at 0, passed but said stopped at routing).
        row.update(stopped="routing", why=f"routed {row['routed']}, KiCad {row['native']['unconnected']} unconnected")
    elif row["native"]["errors"]:
        row.update(stopped="drc", why="new errors: " + ", ".join(f"{k} {v}" for k, v in row["native"]["errors"].items()))
    else:
        row["stopped"] = None
    return row


def route_admitted(task, arguments, token):
    """Route mode (`--route-from`): routes the task's placement from an
    earlier sweep (`<sweep>/<task>/layout/placed`) with `route-kicad-board`,
    for router experiments without the placement race and the legaliser.
    The row and the pass rule are the layout's; constraints are not checked
    (the placement is given)."""
    name = task["name"]
    directory = task_directory(task)
    work = arguments.output / name
    if work.exists():
        shutil.rmtree(work)
    work.mkdir(parents=True)
    placed = arguments.route_from / name / "layout" / "placed"
    row = {"name": name, "mode": "route"}
    if not (placed / f"{task['board_id']}.kicad_pcb").exists():
        row.update(error=f"no placement in {placed}", **{"pass": False})
        return row
    shutil.copytree(placed, work / "placed")
    designer_started = time.monotonic()
    allowed = designer_findings(directory, task["board_id"], work, arguments.binary) if task.get("designer_budget") else {}
    designer_seconds = round(time.monotonic() - designer_started, 1)
    baseline = StrictBaseline(directory, task["board_id"], work, arguments.binary, bool(task.get("designer_budget")))
    router = dict(task.get("router") or {})
    if arguments.budget_scale != 1:
        key, default = ROUTE_BUDGET
        router[key] = round(router.get(key, default) * arguments.budget_scale)
    router_argument = "auto"
    if router:
        (work / "router.json").write_text(json.dumps(router))
        router_argument = str(work / "router.json")
    env = None
    if arguments.threads:
        env = dict(os.environ, RAYON_NUM_THREADS=str(arguments.threads))
    # The binary's verdict counts every error (like a layout's); the row's
    # pass rule subtracts the designer's findings, as for a layout.
    command = [str(arguments.binary), "route-kicad-board", str(work / "placed"), task["board_id"],
               str(work / "route"), router_argument, "--reference", "none"]
    started = time.monotonic()
    load_before = os.getloadavg()[0]
    timeout = scaled_timeout(arguments, router.get(ROUTE_BUDGET[0], ROUTE_BUDGET[1]))
    code, output, peak_mb, own_mb, cpu_seconds = run_measured(command, timeout, env,
                                                              lambda pid: arguments.gate.started(token, pid))
    (work / "route.log").write_text(output)
    row.update({"seconds": round(time.monotonic() - started, 1), "cpu_seconds": cpu_seconds,
                "load": [round(load_before, 1), round(os.getloadavg()[0], 1)],
                "peak_rss_mb": peak_mb, "layout_rss_mb": own_mb, "route_from": str(placed)})
    if arguments.budget_scale != 1:
        row["budget_scale"] = arguments.budget_scale
    if code == "timeout":
        row.update(error=f"timed out after {timeout} s", cut=["timeout"], **{"pass": False})
        return row
    if killed(code):
        return killed_row(row, code, output)
    return route_result(row, task, work, code, output, allowed, designer_seconds, baseline)


def router_report(work):
    """`work`/route/board-router.json, None where there is none."""
    report = work / "route/board-router.json"
    return json.loads(report.read_text()) if report.exists() else None


def route_result(row, task, work, code, output, allowed, designer_seconds, baseline=None, result=None):
    """The row of a route that ended (not by the harness's timeout):
    `work`/route's report (`result`, router_report's, read here when not
    given), KiCad's findings in `work`/route/drc.json less the designer's
    (`allowed`), and the pass rule (a layout's)."""
    row["exit"] = code
    if result is None:
        result = router_report(work)
    if result is None:
        verdict_file = work / "route/verdict.json"
        verdict = json.loads(verdict_file.read_text()) if verdict_file.exists() else None
        verdict_extras(row, verdict)
        if verdict:
            row["verdict"] = verdict.get("status")
            row["error"] = verdict.get("reason", "")[:300]
        else:
            row["error"] = output.strip().splitlines()[-1][:300] if output.strip() else "no output"
        row["pass"] = False
        return row
    if result.get("verdict"):
        row["verdict"] = result["verdict"].get("status")
    verdict_extras(row, result.get("verdict"))
    copper = native(work / "route", allowed)
    if allowed:
        row["designer_findings"] = allowed
    if task.get("designer_budget"):
        row["designer_drc_seconds"] = designer_seconds
    if result.get("cut"):
        row["cut"] = result["cut"]
    row.update(
        routed=f"{result['routed_connections']}/{result['routable_connections']}",
        vias=result["vias"], length_mm=round(result["length_mm"], 1),
        constraints=0, missed=[], unexpected_misses=[], native=copper,
    )
    if result.get("work_seconds") is not None:
        row["work_seconds"] = round(result["work_seconds"], 1)
    row["pass"] = compare.passes(row)
    strict_row(row, strict_native(work / "route", baseline))
    return row


# The designer tier's ladder budget, in work seconds: the layout tier's
# total_seconds, so that the two runs of a board get the same work.
DESIGNER_ROUTE_BUDGET = LAYOUT_BUDGETS["total_seconds"]


def designer_admitted(task, arguments, token):
    """The designer tier (`--tier designer`): the task's board on the
    designer's placement, its tracks and vias stripped as the layout tier
    strips them (strip-kicad-tracks: the pours that cover a good part of
    the board and every rule area stay, unfilled; small filled zones are
    hand routing and go, as in the layout tier), the project's own files and
    rules with it, routed with `route-kicad-board`, the task's router
    settings and the tier's ladder budget (the layout tier's 1500 s of
    work, times the budget scale). The row is a route row (route_result:
    KiCad's findings less the designer's) with `placement: "designer"`;
    compare.py --gap joins it with a layout row of the same board."""
    name = task["name"]
    directory = task_directory(task)
    board_id = task["board_id"]
    work = arguments.output / name
    if work.exists():
        shutil.rmtree(work)
    work.mkdir(parents=True)
    row = {"name": name, "mode": "route", "placement": "designer"}
    if arguments.budget_scale != 1:
        row["budget_scale"] = arguments.budget_scale
    prepared = work / "placed"
    # The designer's project as the layout tier copies it.
    shutil.copytree(directory, prepared, ignore=COPY_IGNORE)
    board = prepared / f"{board_id}.kicad_pcb"
    strip = harness_common.run_group(strip_command(arguments.binary, board, board), capture_output=True, text=True)
    if strip.returncode != 0:
        row.update(error=f"strip-kicad-tracks: {(strip.stderr or strip.stdout).strip()[-200:]}", **{"pass": False})
        return row
    row["stripped_sha256"] = file_sha256(board)
    designer_started = time.monotonic()
    allowed = designer_findings(directory, board_id, work, arguments.binary) if task.get("designer_budget") else {}
    designer_seconds = round(time.monotonic() - designer_started, 1)
    # The strict rule's baseline, as for the other modes.
    baseline = StrictBaseline(directory, board_id, work, arguments.binary, bool(task.get("designer_budget")))
    router = dict(task.get("router") or {})
    key, _ = ROUTE_BUDGET
    router[key] = round(arguments.designer_budget * arguments.budget_scale, 3)
    (work / "router.json").write_text(json.dumps(router))
    row["budget_seconds"] = router[key]
    env = dict(os.environ, RAYON_NUM_THREADS=str(arguments.threads)) if arguments.threads else None
    command = [str(arguments.binary), "route-kicad-board", str(prepared), board_id, str(work / "route"),
               str(work / "router.json"), "--reference", "none"]
    started = time.monotonic()
    load_before = os.getloadavg()[0]
    timeout = scaled_timeout(arguments, router.get(ROUTE_BUDGET[0], ROUTE_BUDGET[1]))
    code, output, peak_mb, own_mb, cpu_seconds = run_measured(command, timeout, env,
                                                              lambda pid: arguments.gate.started(token, pid))
    (work / "route.log").write_text(output)
    row.update(seconds=round(time.monotonic() - started, 1), cpu_seconds=cpu_seconds,
               load=[round(load_before, 1), round(os.getloadavg()[0], 1)], peak_rss_mb=peak_mb,
               layout_rss_mb=own_mb)
    if code == "timeout":
        row.update(error=f"timed out after {timeout} s", cut=["timeout"], **{"pass": False})
        return row
    if killed(code):
        return killed_row(row, code, output)
    result = router_report(work)
    route_result(row, task, work, code, output, allowed, designer_seconds, baseline, result)
    if result is not None:
        row["open"] = result["routable_connections"] - result["routed_connections"]
    return row


# Route-mode tasks (`"mode": "route"`, the expansion tier): the designer's
# placement, fetched at a pinned commit, stripped and routed as the
# 2026-10-18 intake did (benchmarks/expansion/triage.py and run.py).

# The largest file fetched (the harvest's --max-bytes).
FETCH_MAX_BYTES = 40_000_000
# Licence fields that name no licence: such a board is not fetched.
NO_LICENCE = (None, "", "NOASSERTION", "NONE")
# The intake's classes of a quick route's outcome (triage.py's classify):
# a complete route at most this many routing seconds is clean_fast.
INTAKE_SIMPLE_SECONDS = 3


class FetchRefused(Exception):
    """A task's files may not be fetched (no licence)."""


def intake_deadline(budget):
    """The router's wall guard the intake gave a ladder of `budget` work
    seconds (benchmarks/expansion/run.py: 90 s at 30 s, 900 s at 300 s)."""
    return max(90.0, 3 * budget)


def intake_timeout(budget, stretch=1.0):
    """The intake's whole-process guard for that ladder (180 s at 30 s,
    960 s at 300 s), its wall guard stretched `stretch` times (the binary's
    stretch for pinned rates: `wall_stretch_allowance`)."""
    return max(180.0, 3 * budget * stretch + 60)


def strip_command(binary, original, stripped):
    """The intake's strip, with its exact arguments (triage.py): the
    designer's board in, a new board out; `strip-kicad-tracks` drops
    segments, arcs, vias, cached fills and copper zones under 10% of the
    board's area, and keeps the large pours and the rule areas."""
    return [str(binary), "strip-kicad-tracks", str(original), str(stripped)]


def expansion_module(name):
    """benchmarks/expansion/<name>.py, loaded once (harvest.py's GitHub
    fetch, triage.py's outcome classes): that directory is not a package."""
    with _EXPANSION_LOCK:
        if name not in _EXPANSION:
            import importlib.util
            spec = importlib.util.spec_from_file_location(f"expansion_{name}", ROOT / f"benchmarks/expansion/{name}.py")
            module = importlib.util.module_from_spec(spec)
            spec.loader.exec_module(module)
            _EXPANSION[name] = module
    return _EXPANSION[name]


_EXPANSION = {}
_EXPANSION_LOCK = threading.Lock()


def github_raw(repository, commit, path, maximum):
    """A file of `repository` at `commit` (None when it is not there), with
    the harvest's own fetch (benchmarks/expansion/harvest.py, GitHub.raw)."""
    return expansion_module("harvest").GitHub().raw(repository, commit, path, maximum)


def ensure_source(task, directory, harvest_boards=None, raw=github_raw):
    """Puts the task's files (`source.files`: file name -> SHA-256) into
    `directory` unless it holds them already. Returns None when it did,
    else where they came from: "harvest" (the harvest's copy,
    `harvest_boards`/`source.harvest_name`, when it matches) or "github"
    (`raw` at `source.commit`, next to `source.path`). Every file is
    verified against its SHA-256 before it is used; a task without a
    licence is not fetched (FetchRefused). The files arrive in a staging
    directory that replaces `directory` when all are there."""
    source = task["source"]
    files = source["files"]

    def verified(folder):
        return all((folder / name).is_file() and file_sha256(folder / name) == digest
                   for name, digest in files.items())

    if directory.is_dir() and verified(directory):
        return None
    if source.get("license") in NO_LICENCE:
        raise FetchRefused(f"no licence for {source.get('repository')}: not fetched")
    staging = directory.parent / (directory.name + ".partial")
    shutil.rmtree(staging, ignore_errors=True)
    staging.mkdir(parents=True)
    try:
        harvested = Path(harvest_boards) / source["harvest_name"] \
            if harvest_boards is not None and source.get("harvest_name") else None
        if harvested is not None and verified(harvested):
            for name in files:
                shutil.copyfile(harvested / name, staging / name)
            origin = "harvest"
        else:
            folder = PurePosixPath(source["path"]).parent
            for name, digest in files.items():
                path = name if str(folder) == "." else str(folder / name)
                data = raw(source["repository"], source["commit"], path, FETCH_MAX_BYTES)
                if data is None:
                    raise ValueError(f"{path} is not in {source['repository']} at {source['commit'][:12]}")
                actual = hashlib.sha256(data).hexdigest()
                if actual != digest:
                    raise ValueError(f"{path}: SHA-256 {actual[:12]} is not the task's {digest[:12]}")
                (staging / name).write_bytes(data)
            origin = "github"
        if directory.exists():
            shutil.rmtree(directory)
        staging.replace(directory)
    finally:
        shutil.rmtree(staging, ignore_errors=True)
    return origin


def intake_outcome(result, kicad, budget, simple_seconds=INTAKE_SIMPLE_SECONDS):
    """The intake's outcome class of a route: triage.py's classify itself,
    on its report `result`, KiCad's findings on the result (`kicad`,
    run.py's native() without the designer's findings, None when KiCad did
    not check it) and the ladder's budget."""
    row = {"route_process": {}, "route": result,
           "native_result": {} if kicad is None else
           {"summary": {"clean": not kicad["errors"] and not kicad["unconnected"]}}}
    return expansion_module("triage").classify(row, simple_seconds, budget)


def route_task_config(task, budget_scale):
    """The router's settings for a route-mode task: its `router`, the
    ladder's budget (`budget_seconds`, else the router's own or 1200 s,
    times `budget_scale`) and, with `budget_seconds`, the intake's wall
    guard for that budget unless the task sets one."""
    router = dict(task.get("router") or {})
    key, default = ROUTE_BUDGET
    budget = round(task.get("budget_seconds", router.get(key, default)) * budget_scale, 3)
    router[key] = budget
    if task.get("budget_seconds") is not None:
        router.setdefault("deadline_seconds", intake_deadline(budget))
    return router


def route_task_admitted(task, arguments, token):
    """A route-mode task (`"mode": "route"`): fetches the board if needed
    (ensure_source), copies it with its .kicad_pro and .kicad_dru, strips
    it (strip_command), routes it with `route-kicad-board` and the task's
    router settings, `budget_seconds` of work times the budget scale and
    the intake's wall guard, and checks the result with `drc-kicad-board`
    when the router did not. The row is a route row (route_result) with
    the licence, `fetched`, `source_sha256` and the intake's outcome class
    (`outcome`)."""
    name = task["name"]
    directory = task_directory(task)
    board_id = task["board_id"]
    work = arguments.output / name
    if work.exists():
        shutil.rmtree(work)
    work.mkdir(parents=True)
    source = task.get("source") or {}
    row = {"name": name, "mode": "route", "license": source.get("license")}
    if arguments.budget_scale != 1:
        row["budget_scale"] = arguments.budget_scale
    if source:
        try:
            fetched = ensure_source(task, directory, arguments.harvest_boards)
        except (FetchRefused, ValueError, OSError) as error:
            row.update(fetched=False, error=f"fetch: {error}"[:300], outcome="fetch_error", **{"pass": False})
            return row
        row["fetched"] = fetched is not None
        if fetched:
            row["fetched_from"] = fetched
    original = directory / f"{board_id}.kicad_pcb"
    if not original.is_file():
        row.update(error=f"no board {original}", outcome="source_issue", **{"pass": False})
        return row
    row["source_sha256"] = file_sha256(original)
    prepared = work / "source"
    prepared.mkdir()
    # The project's rules come along; nothing else of the repository does.
    for extension in (".kicad_pro", ".kicad_dru"):
        sidecar = original.with_suffix(extension)
        if sidecar.is_file():
            shutil.copyfile(sidecar, prepared / sidecar.name)
    board = prepared / original.name
    strip = harness_common.run_group(strip_command(arguments.binary, original, board), capture_output=True,
                                     text=True)
    if strip.returncode != 0:
        row.update(error=f"strip-kicad-tracks: {(strip.stderr or strip.stdout).strip()[-200:]}",
                   outcome="source_issue", **{"pass": False})
        return row
    row["stripped_sha256"] = file_sha256(board)
    designer_started = time.monotonic()
    allowed = designer_findings(directory, board_id, work, arguments.binary) if task.get("designer_budget") else {}
    designer_seconds = round(time.monotonic() - designer_started, 1)
    baseline = StrictBaseline(directory, board_id, work, arguments.binary, bool(task.get("designer_budget")))
    router = route_task_config(task, arguments.budget_scale)
    budget = router[ROUTE_BUDGET[0]]
    (work / "router.json").write_text(json.dumps(router))
    row["budget_seconds"] = budget
    env = dict(os.environ, RAYON_NUM_THREADS=str(arguments.threads)) if arguments.threads else None
    command = [str(arguments.binary), "route-kicad-board", str(prepared), board_id, str(work / "route"),
               str(work / "router.json"), "--reference", "none"]
    timeout = (scaled_timeout(arguments) if arguments.timeout_given
               else intake_timeout(budget, wall_stretch_allowance(arguments)))
    started = time.monotonic()
    load_before = os.getloadavg()[0]
    code, output, peak_mb, own_mb, cpu_seconds = run_measured(command, timeout, env,
                                                              lambda pid: arguments.gate.started(token, pid))
    (work / "route.log").write_text(output)
    row.update(seconds=round(time.monotonic() - started, 1), cpu_seconds=cpu_seconds, peak_rss_mb=peak_mb,
               layout_rss_mb=own_mb)
    if code == "timeout":
        row.update(load=[round(load_before, 1), round(os.getloadavg()[0], 1)], error=f"timed out after {timeout} s",
                   cut=["timeout"], outcome="unresolved_timeout", **{"pass": False})
        return row
    if killed(code):
        # Nothing it left is judged, and no DRC runs on a partial board
        # (the intake's class of a signal: triage.py's process_signal).
        row["load"] = [round(load_before, 1), round(os.getloadavg()[0], 1)]
        killed_row(row, code, output)
        row["outcome"] = "process_signal"
        return row
    # KiCad's check of the result where the router skipped its own (the
    # intake's skip_native_verification): drc-kicad-board, refilled.
    routed_board = work / "route" / f"{board_id}.kicad_pcb"
    if routed_board.exists() and not (work / "route/drc.json").exists():
        drc_started = time.monotonic()
        drc_code, drc_output, drc_peak, _, drc_cpu = run_measured(
            [str(arguments.binary), "drc-kicad-board", str(work / "route"), board_id, str(work / "route/drc.json")],
            FIXED_TIMEOUT_SECONDS)
        (work / "drc.log").write_text(drc_output)
        row.update(drc_seconds=round(time.monotonic() - drc_started, 1), peak_rss_mb=max(peak_mb, drc_peak),
                   cpu_seconds=round(cpu_seconds + drc_cpu, 1))
    row["load"] = [round(load_before, 1), round(os.getloadavg()[0], 1)]
    result = router_report(work)
    route_result(row, task, work, code, output, allowed, designer_seconds, baseline, result)
    if result is not None:
        row["routing_seconds"] = round(result.get("routing_seconds", 0), 1)
        row["open"] = result["routable_connections"] - result["routed_connections"]
        row["outcome"] = intake_outcome(result, native(work / "route"), budget)
    else:
        row["outcome"] = "import_error" if code == 2 else "model_error"
    return row


def run_mode(tasks):
    """The run's mode from its tasks: "route" when every task is a
    route-mode task, else "layout"."""
    return "route" if tasks and all(task.get("mode") == "route" for task in tasks) else "layout"


def physical_cores():
    """The machine's physical cores (logical CPUs that share a core count
    once), from /sys, else lscpu, else the logical count."""
    cores = set()
    for topology in Path("/sys/devices/system/cpu").glob("cpu[0-9]*/topology"):
        try:
            package = (topology / "physical_package_id").read_text().strip()
            core = (topology / "core_id").read_text().strip()
        except OSError:
            continue
        cores.add((package, core))
    if cores:
        return len(cores)
    try:
        listing = subprocess.run(["lscpu", "-p=core,socket"], capture_output=True, text=True, check=True).stdout
        cores = {line for line in listing.splitlines() if line and not line.startswith("#")}
        if cores:
            return len(cores)
    except (OSError, subprocess.CalledProcessError):
        pass
    return os.cpu_count() or 1


def default_threads(jobs, cores):
    """Search threads per board when --threads is not given: the physical
    cores shared among the jobs (jobs x threads <= cores), at most 4, at
    least 1; None with one job (all CPUs)."""
    if jobs <= 1:
        return None
    return max(1, min(4, cores // jobs))


def parallelism(tiered, jobs, threads, cores):
    """Jobs, threads (None: as default_threads decides) and where they came
    from: a tier given neither runs TIER_THREADS a board and cores /
    TIER_THREADS boards at once ("tier default"); otherwise what was given,
    jobs 1 if not ("command line")."""
    if tiered and jobs is None and threads is None:
        return max(1, cores // TIER_THREADS), TIER_THREADS, "tier default"
    return jobs or 1, threads, "command line"


def spliced_rows(path, names):
    """The rows of `names` in an earlier log (compare.rows_of: a log or an
    output directory), the last of each board, marked spliced and with
    the log they came from (`spliced_from`: the row's own where it was
    spliced there already and says from where, else `path`)."""
    rows = by_name(rows_of(path))
    return {name: {**rows[name], "spliced": True, "spliced_from": rows[name].get("spliced_from") or str(path)}
            for name in names if name in rows}


def run_parts(ordered, ran, arguments, wall):
    """The parts a spliced run's rows came from (summary.json's `parts`):
    this run's (its rows, jobs, threads, wall and binary) and, per log the
    spliced rows name (`spliced_from`), their count with that run's jobs,
    threads and wall from its summary.json where it has one (its own part
    of it when it was spliced too) and the binaries its rows name."""
    parts = [{"log": "this run", "output": str(arguments.output), "rows": len(ran), "jobs": arguments.jobs,
              "threads": arguments.threads, "wall_seconds": round(wall), "binary": str(arguments.binary),
              "binary_sha256": arguments.binary_sha256}]
    sources = {}
    for row in ordered:
        if row.get("spliced"):
            sources.setdefault(row.get("spliced_from") or "unknown", []).append(row)
    for log, rows in sources.items():
        part = {"log": log, "rows": len(rows)}
        recorded = harness_common.read_json(compare.output_directory(log) / "summary.json") if log != "unknown" \
            else None
        if isinstance(recorded, dict):
            own = next((entry for entry in recorded.get("parts") or [] if entry.get("log") == "this run"), recorded)
            for key in ("jobs", "threads", "wall_seconds"):
                if own.get(key) is not None:
                    part[key] = own[key]
        binaries = sorted({row.get("binary") for row in rows if row.get("binary")})
        if binaries:
            part["binaries"] = binaries
        parts.append(part)
    return parts


def rules_of(rows):
    """{strict rule: rows judged under it} (compare.strict_rule; rows
    without a strict judgement are left out)."""
    rules = {}
    for row in rows:
        rule = compare.strict_rule(row)
        if rule is not None:
            rules[rule] = rules.get(rule, 0) + 1
    return rules


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("output", type=Path)
    parser.add_argument("--only", nargs="*")
    parser.add_argument("--binary", type=Path, default=ROOT / "target/release/pcb-maker")
    parser.add_argument("--timeout", type=int, default=None,
                        help="the harness's timeout per board in seconds (default 1800; route-mode tasks: the "
                             "intake's, 3 x the budget + 60 s, at least 180 s)")
    parser.add_argument("--harvest-data", type=Path, default=ROOT / "benchmarks/expansion/data",
                        help="route-mode tasks: the expansion harvest's data directory, whose boards/<name> "
                             "copies are used before GitHub when their SHA-256 matches")
    parser.add_argument("--tasks", type=Path, default=None,
                        help="task list; constraint files are relative to it (default: tasks.json here, "
                             "or the tier's own list)")
    parser.add_argument("--tier", choices=sorted(TIERS),
                        help="only the boards of a tier (quick: quick.json, many small boards side by side)")
    parser.add_argument("--placer", type=json.loads, default={},
                        help="placer settings for every board, as JSON (e.g. '{\"halo_scale\": 0.75}')")
    parser.add_argument("--layout", type=json.loads, default={},
                        help="layout settings for every board, as JSON, over the task's "
                             "(e.g. '{\"congestion_score\": \"global\"}')")
    parser.add_argument("--jobs", type=int, default=None,
                        help="boards at once (default: with --tier and no --threads the physical cores / "
                             "TIER_THREADS, else 1)")
    parser.add_argument("--threads", type=int, default=None,
                        help="search threads per board (RAYON_NUM_THREADS; default with --tier and no --jobs: "
                             "TIER_THREADS; else with several jobs the physical cores shared among them, "
                             "at most 4; with one job: all)")
    parser.add_argument("--min-free-gb", type=float, default=12.0,
                        help="a floor beside the memory ledger: start a board only when this much memory "
                             "stays available after its expected peak and what the runs in the ledger are "
                             "still expected to grow by (0: the ledger alone)")
    parser.add_argument("--ledger-drain-after", type=float, default=harness_common.DEFAULT_DRAIN_AFTER_SECONDS,
                        help="seconds a board waits for memory before the ledger drains for it: newer requests "
                             "then go only if they leave it its room or end before every running board "
                             "(default %(default).0f; docs/benchmarks.md, \"Memory ledger\")")
    parser.add_argument("--ledger-priority", choices=("normal", harness_common.PRIORITY_NIGHTLY), default="normal",
                        help="nightly: this run's boards go first among the waiters and are never passed once "
                             "waiting (the ledger drains for them at once); scripts/nightly.sh sets it")
    parser.add_argument("--order", choices=("seconds-desc", "peak-desc"), default="seconds-desc",
                        help="the order the boards start in: seconds-desc (a tier's default) the longest "
                             "expected first, so that the last job is not a slow one; peak-desc the largest "
                             "expected peak first (scripts/nightly.sh's designer stage: the large boards while "
                             "the machine is quiet), then the longest")
    parser.add_argument("--expected-gb", type=float, default=3.0,
                        help="expected peak of a board the tier does not estimate")
    parser.add_argument("--budget-scale", type=float, default=None,
                        help="scale the work budget (layout: total_seconds, move_seconds, polish_seconds; "
                             "route mode: ladder_budget_seconds) and the timeout beyond its fixed 300 s "
                             "(default: the tier's budget_scale, else 1)")
    parser.add_argument("--route-from", type=Path, default=None,
                        help="route mode: route each board's placement from this earlier output directory "
                             "(<dir>/<task>/layout/placed) with route-kicad-board instead of laying it out")
    parser.add_argument("--splice-from", type=Path, default=None,
                        help="an earlier log (or output directory): the rows of the boards not run "
                             "(outside --only) are copied from it, marked spliced, into this run's log "
                             "and results.json")
    parser.add_argument("--quality", action="store_true",
                        help="score the designer's board and the result with score-kicad-board (quality.py's "
                             "measures, about 0.1 s a board) into each row's quality")
    parser.add_argument("--work-rate", default=None,
                        help="the work rates every board runs at (PCB_WORK_RATE: '<expansions/s>[,<placer "
                             "units/s>]'; 'measured': each binary measures its machine at its start; default: "
                             "PCB_WORK_RATE if set, else the reference's " + REFERENCE_WORK_RATE + ")")
    parser.add_argument("--lock", action="store_true",
                        help="take build/quick-tier.lock and wait for it (one tier at a time on the machine); "
                             "by default runs go side by side and the memory ledger keeps them apart "
                             "(README.md, \"How to measure\")")
    parser.add_argument("--no-lock", action="store_true",
                        help="the default since 2026-10-22 (no lock); kept for older command lines")
    arguments = parser.parse_args()
    arguments.timeout_given = arguments.timeout is not None
    if arguments.timeout is None:
        arguments.timeout = DEFAULT_TIMEOUT_SECONDS
    arguments.work_rate = pin_work_rate(arguments.work_rate, os.environ)
    print(f"work rate {arguments.work_rate}" + (" (the reference's)" if arguments.work_rate == REFERENCE_WORK_RATE
                                                else ""), file=sys.stderr)
    arguments.harvest_boards = arguments.harvest_data / "boards"
    arguments.output.mkdir(parents=True, exist_ok=True)
    tier = None
    if arguments.tier:
        tier = json.loads(TIERS[arguments.tier].read_text())
        if arguments.tasks is None:
            arguments.tasks = ROOT / tier["tasks"]
    if arguments.tasks is None:
        arguments.tasks = HERE / "tasks.json"
    if arguments.budget_scale is None:
        arguments.budget_scale = (tier or {}).get("budget_scale", 1.0)
    if arguments.budget_scale <= 0:
        parser.error("--budget-scale must be positive")
    arguments.schematic = arguments.tier == "schematic"
    if arguments.schematic and arguments.route_from is not None:
        parser.error("--route-from does not go with --tier schematic (it lays out from the schematic)")
    arguments.designer = arguments.tier == "designer"
    arguments.designer_budget = (tier or {}).get("route_budget_seconds", DESIGNER_ROUTE_BUDGET)
    if arguments.designer and arguments.route_from is not None:
        parser.error("--route-from does not go with --tier designer (it routes the designer's placement)")
    if arguments.route_from is not None and not arguments.route_from.is_dir():
        parser.error(f"--route-from: {arguments.route_from} is not a directory")
    if arguments.splice_from is not None and not arguments.splice_from.exists():
        parser.error(f"--splice-from: {arguments.splice_from} not found")
    # One tier at a time: a tier run, or a whole task list, waits for the
    # machine's lock (a few boards given with --only and no tier do not).
    # The machine's lock, when asked for (--lock): the memory ledger lets
    # runs go side by side without it.
    if arguments.lock and not arguments.no_lock:
        arguments.lock = take_lock()
    # Physical cores, not logical CPUs: the work budgets are calibrated on
    # one search thread a core, and v16's five jobs of four threads on the
    # 16 cores of a 5950X (beside other agents) ran the search 1.3-1.6 times
    # slower than its work counted, so that 8 of its 17 cut rows were load.
    arguments.physical_cores = physical_cores()
    # A tier given neither --jobs nor --threads runs at its default
    # parallelism; either one given keeps the older defaults for the other.
    arguments.jobs, arguments.threads, arguments.parallelism = parallelism(
        tier is not None, arguments.jobs, arguments.threads, arguments.physical_cores)
    if arguments.threads is None:
        # Beyond a few threads a board routes hardly faster (k30-SBC: 34.2,
        # 32.4 and 31.3 s of search on 1, 2 and 4) but every thread holds a
        # search scratch of the board's size.
        arguments.threads = default_threads(arguments.jobs, arguments.physical_cores)
    elif arguments.jobs * arguments.threads > arguments.physical_cores:
        print(f"warning: {arguments.jobs} jobs x {arguments.threads} threads exceed the "
              f"{arguments.physical_cores} physical cores: the search runs slower than its work counts "
              f"and the wall clock may cut rows (cut)", file=sys.stderr)
    if arguments.threads is None and arguments.jobs > arguments.physical_cores:
        print(f"warning: {arguments.jobs} jobs exceed the {arguments.physical_cores} physical cores",
              file=sys.stderr)
    print(f"{arguments.jobs} jobs, {arguments.threads or 'all'} search threads each, "
          f"{arguments.physical_cores} physical cores ({arguments.parallelism})", file=sys.stderr)
    # Every row and the summary name the task list (compare.py checks that
    # two runs used the same: quick30-x768 on v5 against x776 on v6 looked
    # like a routing change).
    tasks_path = arguments.tasks.resolve()
    try:
        arguments.tasks_label = str(tasks_path.relative_to(ROOT))
    except ValueError:
        arguments.tasks_label = str(tasks_path)
    arguments.tasks_sha256 = hashlib.sha256(tasks_path.read_bytes()).hexdigest()
    print(f"tasks {arguments.tasks_label} (sha256 {arguments.tasks_sha256[:12]})", file=sys.stderr)
    # Every row names the binary it ran (review 2026-10-21, M2): a
    # snapshot's path alone does not say which build it is.
    arguments.binary_sha256 = file_sha256(arguments.binary) if arguments.binary.is_file() else None
    print(f"binary {arguments.binary} (sha256 {(arguments.binary_sha256 or '?')[:12]})", file=sys.stderr)
    arguments.expected_mb = arguments.expected_gb * 1024
    if not 0 <= arguments.ledger_drain_after < float("inf"):
        parser.error("--ledger-drain-after must be a finite number of seconds, at least 0")
    arguments.gate = MemoryGate(arguments.min_free_gb, drain_after=arguments.ledger_drain_after,
                                priority=None if arguments.ledger_priority == "normal" else arguments.ledger_priority)
    tasks = []
    # The boards of the tier (or task list) that --only leaves out: with
    # --splice-from their rows come from the earlier log.
    left_out = []
    for task in json.loads(arguments.tasks.read_text())["tasks"]:
        if tier is not None and task["name"] not in tier["boards"]:
            continue
        if arguments.only and task["name"] not in arguments.only:
            left_out.append(task["name"])
            continue
        if tier is not None:
            entry = tier["boards"].get(task["name"])
            if entry is None:
                continue
            task = {**task, "peak_rss_mb": entry.get("peak_rss_mb"), "expected_seconds": entry.get("seconds")}
            if task["peak_rss_mb"] is None:
                del task["peak_rss_mb"]
        directory = task_directory(task)
        # A route-mode task with a source is fetched when its job runs.
        if not directory.exists() and not (task.get("mode") == "route" and task.get("source")):
            print(f"skipping {task['name']}: {directory} not found")
            continue
        tasks.append(task)
    if tier is not None:
        missing = sorted(set(tier["boards"]) - {task["name"] for task in tasks})
        if missing and not arguments.only:
            print(f"tier {arguments.tier}: {len(missing)} boards not in {arguments.tasks}: {missing}", file=sys.stderr)
    tasks = order_tasks(tasks, arguments.order, tier is not None, arguments.expected_mb)
    # Each row names its input beyond the constraints compare.py checks:
    # the task's entry, the designer's board and its project files, the
    # .kicad_pro and .kicad_dru (compare.py marks a board whose task,
    # board or project differ INPUT).
    identities = {}
    for task in tasks:
        constraints = None
        if task.get("constraints"):
            try:
                constraints = json.loads((arguments.tasks.parent / task["constraints"]).read_text())
            except (OSError, ValueError):
                pass
        identity = {"task_sha256": task_digest(task, constraints)}
        board = task_directory(task) / f"{task['board_id']}.kicad_pcb"
        if board.is_file():
            identity["board_sha256"] = file_sha256(board)
            identity["project_sha256"] = compare.project_files_digest(
                [path for path in (board.with_suffix(extension) for extension in compare.PROJECT_EXTENSIONS)
                 if path.is_file()])
        identities[task["name"]] = identity
    rows = {}
    if arguments.splice_from is not None:
        spliced = spliced_rows(arguments.splice_from, left_out)
        missing = [name for name in left_out if name not in spliced]
        if missing:
            print(f"--splice-from: {len(missing)} boards not run have no row in {arguments.splice_from}: "
                  f"{missing}", file=sys.stderr)
        for name in left_out:
            if name in spliced:
                rows[name] = spliced[name]
                print(json.dumps(spliced[name]), flush=True)
    if run_mode(tasks) == "route" and not arguments.route_from:
        budgets = sorted({task.get("budget_seconds") for task in tasks if task.get("budget_seconds") is not None})
        print(f"mode route (the designer's placement), budget scale {arguments.budget_scale:g}"
              + (f", {', '.join(f'{b * arguments.budget_scale:g}' for b in budgets)} s of work" if budgets else "")
              + (f", timeout {scaled_timeout(arguments)} s" if arguments.timeout_given
                 else ", the intake's timeouts"), file=sys.stderr)
    elif arguments.designer:
        print(f"mode route on the designer's placement, {arguments.designer_budget * arguments.budget_scale:g} s "
              f"of work, timeout {scaled_timeout(arguments, arguments.designer_budget * arguments.budget_scale)} s",
              file=sys.stderr)
    elif arguments.route_from is not None or arguments.budget_scale != 1:
        mode = "schematic" if arguments.schematic else "route from " + str(arguments.route_from) if arguments.route_from \
            else "layout"
        print(f"mode {mode}, "
              f"budget scale {arguments.budget_scale:g}, timeout {scaled_timeout(arguments)} s", file=sys.stderr)
    started = time.monotonic()
    from concurrent.futures import ThreadPoolExecutor, as_completed
    with ThreadPoolExecutor(arguments.jobs) as pool:
        def guarded(task):
            # A runner failure on one task fails that task, not the run.
            try:
                return run_task(task, arguments)
            except Exception as error:
                return {"name": task["name"], "pass": False, "error": f"runner: {type(error).__name__}: {error}",
                        "mode": "schematic" if arguments.schematic else "route" if arguments.route_from
                        or arguments.designer else task.get("mode", "layout")}

        for future in as_completed([pool.submit(guarded, task) for task in tasks]):
            row = future.result()
            row.update(tasks=arguments.tasks_label, tasks_sha256=arguments.tasks_sha256,
                       binary=str(arguments.binary), binary_sha256=arguments.binary_sha256,
                       **identities.get(row["name"], {}))
            # The strict rule this run judges by, also on rows it could
            # not judge (no result).
            row.setdefault("strict_rule", STRICT_RULE)
            rows[row["name"]] = row
            print(json.dumps(row), flush=True)
    wall = time.monotonic() - started
    ran = [rows[task["name"]] for task in tasks if task["name"] in rows]
    ordered = ran + [rows[name] for name in left_out if name in rows]
    (arguments.output / "results.json").write_text(json.dumps(ordered, indent=1))
    passed = sum(1 for row in ordered if row["pass"])
    peak = max((row.get("peak_rss_mb") or 0 for row in ran), default=0)
    spliced_count = len(ordered) - len(ran)
    # The strict rule beside it: rows of older runs spliced in were judged
    # under the rule of their run (compare.strict_rule; rows without
    # `strict_rule` are rule 1), and a sum over several rules says so.
    strict = sum(1 for row in ordered if row.get("pass_strict"))
    rules = rules_of(ordered)
    if len(rules) > 1:
        strict_text = ("strict rules " + ", ".join(f"{rule} ({count} rows)" for rule, count in sorted(rules.items()))
                       + f": {strict} pass, A MIX OF RULES (rejudge.py judges the spliced rows again)")
    else:
        strict_text = f"strict rule {next(iter(rules), STRICT_RULE)}: {strict} pass"
    print(f"{passed}/{len(ordered)} tasks pass ({wall:.0f} s wall, "
          f"largest peak {peak / 1024:.1f} GB"
          + (f"; {spliced_count} rows spliced from {arguments.splice_from}" if spliced_count else "") + ")"
          + f"; {strict_text}")
    # Continuous measures, usable from one run (README.md, "How to measure").
    summary = objectives(ordered)
    summary.update(wall_seconds=round(wall), budget_scale=arguments.budget_scale,
                   strict_rule=next(iter(rules)) if len(rules) == 1 else STRICT_RULE,
                   mode="schematic" if arguments.schematic else "route" if arguments.route_from
                   or arguments.designer else run_mode(tasks),
                   spliced=spliced_count,
                   tasks=arguments.tasks_label, tasks_sha256=arguments.tasks_sha256,
                   # Search threads per board (None: all CPUs, one job) and
                   # the cores they share.
                   jobs=arguments.jobs, threads=arguments.threads, physical_cores=arguments.physical_cores,
                   # Whether jobs and threads were the tier's default
                   # (TIER_THREADS) or given.
                   parallelism=arguments.parallelism,
                   # The work rates every board was pinned at (PCB_WORK_RATE;
                   # "measured": each binary measured its machine) and the
                   # median of what the binaries measured.
                   work_rate=arguments.work_rate, binary=str(arguments.binary),
                   binary_sha256=arguments.binary_sha256,
                   # Over every row, the spliced ones too (each its own
                   # binary's measurement); this run's own rows' beside.
                   **measured_rates(ordered))
    if len(rules) > 1:
        summary["strict_rules"] = {str(rule): count for rule, count in sorted(rules.items())}
    if spliced_count:
        # The parts the rows came from, with each part's jobs and wall:
        # `wall_seconds`, `jobs` and `threads` above are this run's own.
        summary["parts"] = run_parts(ordered, ran, arguments, wall)
        summary["this_run_rates"] = measured_rates(ran)
    if any("labels_dropped" in row for row in ordered):
        summary.update(labels=LABELS, labels_dropped=sum(row.get("labels_dropped", 0) for row in ordered))
    if arguments.designer:
        summary["placement"] = "designer"
    outcomes = {}
    for row in ordered:
        if row.get("outcome"):
            outcomes[row["outcome"]] = outcomes.get(row["outcome"], 0) + 1
    if outcomes:
        # The intake's outcome classes (route-mode tasks, intake_outcome).
        summary["outcomes"] = outcomes
        print("outcomes: " + ", ".join(f"{outcome} {count}" for outcome, count in sorted(outcomes.items())))
    if arguments.schematic:
        # Where the flow stopped, by stage (schematic tier).
        stages = {}
        for row in ordered:
            stage = row.get("stopped", "runner" if row.get("error", "").startswith("runner") else None) or "passed"
            stages[stage] = stages.get(stage, 0) + 1
        summary["stopped"] = stages
        summary["missing_footprints"] = sum(row.get("missing", 0) for row in ordered)
        summary["parity"] = sum(row.get("parity") or 0 for row in ordered)
        print("stopped at: " + ", ".join(f"{stage} {count}" for stage, count in sorted(stages.items()))
              + f"; missing footprints {summary['missing_footprints']}, parity issues {summary['parity']}")
    print(objectives_line(summary) + f"; tasks {arguments.tasks_label} (sha256 {arguments.tasks_sha256[:12]})")
    # The same against the designer's boards, per board (compare.py).
    summary["normalised"] = normalised(ordered)
    print(normalised_line(summary["normalised"]))
    (arguments.output / "summary.json").write_text(json.dumps(summary, indent=1))


if __name__ == "__main__":
    main()
