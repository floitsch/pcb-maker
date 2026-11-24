#!/usr/bin/env python3
# Copyright (C) 2026 Florian Loitsch.
"""Routing proxies of a placed board, from its pads alone, for ranking the
designer's placement against our seeds (docs/experiments/objective-terms.md).

    measures.py compute --out FILE
    measures.py traces --runs DIR --out FILE
    measures.py table --data FILE

`compute` reads the boards the cut census extracted
(experiments/cut-census/cuts.py's `extract`: pads per outer layer with
their nets, the outline, the zone nets, the rules): per dense board the
designer's placement and ours of the night of 2026-11-18
(`build/scratch/fund-cuts/census1/extract/<board>/`), and nine seeds of
`dense-noise.json` with their first route's open terminals
(`build/scratch/fund-cuts/seeds/`). Per placement:

- `hp2`, `hp3_8`, `hp9`: half perimeters of the pad centres per net size
  (nets of two or more pads, `unconnected-` nets left out), and `wire`:
  their sum weighted as the placer weights nets (`min(1, 3 / (pins - 1))`);
- `crossings`: pairs of minimum spanning tree edges of different nets that
  cross (the nets no zone carries, as the placer's ratsnest crossings);
- `rudy_*`: RUDY (Spindler and Johannes) on 0.5 mm bins, horizontal and
  vertical apart: a net's box w x h adds 1/h of horizontal and 1/w of
  vertical wire per mm² over the box (both at least one track pitch); the
  capacity of a bin per direction is the routable layers over the track
  pitch times the share of the bin free of pad copper (outer layers from
  the pads, inner layers blocked by plated holes only). `rudy_over` is the
  overflow summed over bins and directions (mm of wire that does not fit),
  at the capacity scaled by 0.5 (`rudy_over_half`) and 0.25
  (`rudy_over_quarter`) too; `rudy_p95` the 95th percentile of
  demand over capacity among bins with demand;
- `crossings_hot`: crossings weighted by demand over capacity at their
  point (a crossing in free room counts little, one in a full bin more);
- `pin_density_p95`: pads per mm² in 2 mm windows, the 95th percentile over
  windows with pads;
- `closed`, `narrow`: pairs of pads of different footprints and nets on a
  shared outer layer whose copper boxes lie closer than one track's
  quantum (`w + 2 s`: no track passes) or two tracks' (`2 w + 3 s`);
  `closed_active`, `narrow_active`: the same pairs where at least one of
  the two parts is not a free passive (a part of at most two pads whose
  reference starts R, C, L, FB, F, FL or FER, the skeleton's rule
  without its footprint names);
- `foreign_1_5`: per pad the pads of other footprints within 1.5 mm
  (centre to centre, same outer layer), summed;
- `spread`: the root mean square distance of the footprints' centres from
  their centroid over the board's (outline box) half diagonal.

`traces` measures today's placements instead: per board of `--runs`
(`<board>/<variant>/trace.json` and `placed/`, as labelled.py and
skeleton_gap.py write them) every traced seed's final poses (the placed
board re-extracted with those poses), labelled `<variant>@<seed>`, and the
designer's own placement (`designer-raw`). Their first routes, where
labelled.py routed them, come from `<runs>/../firstroute`.

`table` gives per board the designer's value against our seeds' (rank and
distance in spreads), and per measure the Spearman correlation with the
first route's open terminals across the seeds of each board.
"""

import argparse
import json
import math
import re
import statistics
import sys
from pathlib import Path

import numpy as np

SCRATCH = Path("/home/flo/programming/pcb-maker/build/scratch/fund-cuts")
BIN = 0.5
MEASURES = ["wire", "hp2", "hp3_8", "hp9", "crossings", "rudy_over", "rudy_over_half", "rudy_over_quarter",
            "rudy_p95", "crossings_hot", "pin_density_p95", "spread", "closed", "closed_active", "narrow", "narrow_active",
            "foreign_1_5"]


def net_weight(pins):
    return min(1.0, 3.0 / (pins - 1)) if pins > 1 else 0.0


def pads_of(board):
    """[(x, y, net, layers, box, hole)] of the copper pads: centre of the
    first layer's polygons' box, the layers it is on, its box."""
    result = []
    for footprint in board["footprints"]:
        for pad in footprint["pads"]:
            if pad["npth"] or not pad["layers"]:
                continue
            points = [p for polygons in pad["layers"].values() for polygon in polygons for p in polygon]
            if not points:
                continue
            xs = [p[0] for p in points]
            ys = [p[1] for p in points]
            box = (min(xs), min(ys), max(xs), max(ys))
            result.append(((box[0] + box[2]) / 2, (box[1] + box[3]) / 2, pad["net"], set(pad["layers"]), box,
                           "hole" in pad, footprint["reference"]))
    return result


def mst(points):
    """Edges (i, j) of the Euclidean minimum spanning tree (Prim)."""
    n = len(points)
    if n < 2:
        return []
    p = np.asarray(points)
    inside = np.zeros(n, bool)
    inside[0] = True
    best = np.linalg.norm(p - p[0], axis=1)
    parent = np.zeros(n, int)
    edges = []
    for _ in range(n - 1):
        candidates = np.where(inside, np.inf, best)
        j = int(np.argmin(candidates))
        edges.append((int(parent[j]), j))
        inside[j] = True
        distance = np.linalg.norm(p - p[j], axis=1)
        closer = distance < best
        parent[closer] = j
        best = np.minimum(best, distance)
    return edges


def crossing_points(segments, nets):
    """Points where two segments of different nets cross properly."""
    if len(segments) < 2:
        return np.zeros((0, 2))
    s = np.asarray(segments)  # (n, 4)
    a, b = s[:, :2], s[:, 2:]
    nets = np.asarray(nets)
    result = []
    lo = np.minimum(a, b)
    hi = np.maximum(a, b)
    for i in range(len(s) - 1):
        j = np.arange(i + 1, len(s))
        j = j[(nets[j] != nets[i]) & (lo[j, 0] <= hi[i, 0]) & (hi[j, 0] >= lo[i, 0]) &
              (lo[j, 1] <= hi[i, 1]) & (hi[j, 1] >= lo[i, 1])]
        if len(j) == 0:
            continue
        p, r = a[i], b[i] - a[i]
        q, t = a[j], b[j] - a[j]
        denominator = r[0] * t[:, 1] - r[1] * t[:, 0]
        ok = np.abs(denominator) > 1e-12
        qp = q - p
        u = np.where(ok, (qp[:, 0] * t[:, 1] - qp[:, 1] * t[:, 0]) / np.where(ok, denominator, 1), -1)
        v = np.where(ok, (qp[:, 0] * r[1] - qp[:, 1] * r[0]) / np.where(ok, denominator, 1), -1)
        eps = 1e-9
        hit = ok & (u > eps) & (u < 1 - eps) & (v > eps) & (v < 1 - eps)
        if hit.any():
            result.append(p + u[hit, None] * r)
    return np.concatenate(result) if result else np.zeros((0, 2))


def measure(board):
    pads = pads_of(board)
    zone = set(board.get("zone_nets") or [])
    rules = board["rules"]
    pitch = rules["track"] + rules["clearance"]
    layers = board["copper_layers"]
    outline = [p for ring in board["outline"] for p in ring]
    x0, y0 = min(p[0] for p in outline), min(p[1] for p in outline)
    x1, y1 = max(p[0] for p in outline), max(p[1] for p in outline)
    nx, ny = int(math.ceil((x1 - x0) / BIN)) + 1, int(math.ceil((y1 - y0) / BIN)) + 1

    nets = {}
    for pad in pads:
        if pad[2] and not pad[2].startswith("unconnected-"):
            nets.setdefault(pad[2], []).append((pad[0], pad[1]))
    result = {"hp2": 0.0, "hp3_8": 0.0, "hp9": 0.0, "wire": 0.0}
    demand_h = np.zeros((nx, ny))
    demand_v = np.zeros((nx, ny))
    segments, segment_nets = [], []
    for index, (name, points) in enumerate(sorted(nets.items())):
        if len(points) < 2:
            continue
        xs, ys = [p[0] for p in points], [p[1] for p in points]
        w, h = max(xs) - min(xs), max(ys) - min(ys)
        bucket = "hp2" if len(points) == 2 else "hp3_8" if len(points) <= 8 else "hp9"
        result[bucket] += w + h
        result["wire"] += net_weight(len(points)) * (w + h)
        if name in zone:
            continue
        we, he = max(w, pitch), max(h, pitch)
        i0, i1 = (min(max(int((v - x0) / BIN), 0), nx - 1) for v in (min(xs), max(xs)))
        j0, j1 = (min(max(int((v - y0) / BIN), 0), ny - 1) for v in (min(ys), max(ys)))
        demand_h[i0:i1 + 1, j0:j1 + 1] += 1.0 / he
        demand_v[i0:i1 + 1, j0:j1 + 1] += 1.0 / we
        for i, j in mst(points):
            segments.append((*points[i], *points[j]))
            segment_nets.append(index)
    # Capacity: the share of each bin free of pad copper, per layer.
    blocked = np.zeros((nx, ny))  # summed over layers, in layer-shares
    for pad in pads:
        box = pad[4]
        on = len(pad[3]) + (max(layers - 2, 0) if pad[5] else 0)
        i0, i1 = int((box[0] - x0) / BIN), int((box[2] - x0) / BIN)
        j0, j1 = int((box[1] - y0) / BIN), int((box[3] - y0) / BIN)
        for i in range(max(i0, 0), min(i1, nx - 1) + 1):
            for j in range(max(j0, 0), min(j1, ny - 1) + 1):
                bx0, by0 = x0 + i * BIN, y0 + j * BIN
                overlap = (max(0.0, min(box[2], bx0 + BIN) - max(box[0], bx0)) *
                           max(0.0, min(box[3], by0 + BIN) - max(box[1], by0)))
                blocked[i, j] += on * overlap / (BIN * BIN)
    free_layers = np.clip(layers - blocked, 0.0, None)
    capacity = free_layers / pitch / 2.0  # half the layers' tracks per direction
    for name, scale in (("rudy_over", 1.0), ("rudy_over_half", 0.5), ("rudy_over_quarter", 0.25)):
        c = capacity * scale
        result[name] = float((np.clip(demand_h - c, 0, None).sum() + np.clip(demand_v - c, 0, None).sum())
                             * BIN * BIN)
    ratio = np.maximum(demand_h, demand_v) / np.maximum(capacity, 1e-6)
    used = ratio[(demand_h + demand_v) > 0]
    result["rudy_p95"] = float(np.percentile(used, 95)) if used.size else 0.0
    points = crossing_points(segments, segment_nets)
    result["crossings"] = int(len(points))
    if len(points):
        i = np.clip(((points[:, 0] - x0) / BIN).astype(int), 0, nx - 1)
        j = np.clip(((points[:, 1] - y0) / BIN).astype(int), 0, ny - 1)
        result["crossings_hot"] = float(np.minimum(ratio[i, j], 10.0).sum())
    else:
        result["crossings_hot"] = 0.0
    # Pads per mm² in 2 mm windows.
    window = 4
    count = np.zeros((nx, ny))
    for pad in pads:
        count[min(max(int((pad[0] - x0) / BIN), 0), nx - 1), min(max(int((pad[1] - y0) / BIN), 0), ny - 1)] += 1
    summed = np.cumsum(np.cumsum(np.pad(count, ((1, 0), (1, 0))), 0), 1)
    windows = (summed[window:, window:] - summed[:-window, window:] - summed[window:, :-window]
               + summed[:-window, :-window]) / (window * BIN) ** 2
    occupied = windows[windows > 0]
    result["pin_density_p95"] = float(np.percentile(occupied, 95)) if occupied.size else 0.0
    result.update(crowding(pads, rules))
    centres = np.asarray([f["at"][:2] for f in board["footprints"] if f["pads"]])
    rms = math.sqrt(((centres - centres.mean(axis=0)) ** 2).sum(axis=1).mean())
    result["spread"] = rms / (math.hypot(x1 - x0, y1 - y0) / 2)
    return result


PASSIVE = re.compile(r"(R|C|L|FB|F|FL|FER)\d")


def passives(pads):
    """The references of the free passives: at most two pads and a
    passive's reference prefix."""
    count = {}
    for pad in pads:
        count[pad[6]] = count.get(pad[6], 0) + 1
    return {reference for reference, n in count.items() if n <= 2 and PASSIVE.match(reference or "")}


def crowding(pads, rules):
    from scipy.spatial import cKDTree  # noqa: PLC0415

    w, s = rules["track"], rules["clearance"]
    q1, q2 = w + 2 * s, 2 * w + 3 * s
    result = {"closed": 0, "closed_active": 0, "narrow": 0, "narrow_active": 0, "foreign_1_5": 0}
    passive = passives(pads)
    for layer in ("F.Cu", "B.Cu"):
        on = [p for p in pads if layer in p[3]]
        if len(on) < 2:
            continue
        centres = np.asarray([(p[0], p[1]) for p in on])
        boxes = np.asarray([p[4] for p in on])
        half = max(float(np.max(boxes[:, 2] - boxes[:, 0])), float(np.max(boxes[:, 3] - boxes[:, 1])))
        tree = cKDTree(centres)
        for i, j in tree.query_pairs(half + q2):
            a, b = on[i], on[j]
            if a[6] == b[6] or (a[2] is not None and a[2] == b[2]):
                continue
            dx = max(0.0, max(a[4][0], b[4][0]) - min(a[4][2], b[4][2]))
            dy = max(0.0, max(a[4][1], b[4][1]) - min(a[4][3], b[4][3]))
            gap = math.hypot(dx, dy)
            active = not (a[6] in passive and b[6] in passive)
            result["closed"] += gap < q1
            result["narrow"] += gap < q2
            result["closed_active"] += active and gap < q1
            result["narrow_active"] += active and gap < q2
        for i, j in tree.query_pairs(1.5):
            if on[i][6] != on[j][6]:
                result["foreign_1_5"] += 2
    return result


def compute(arguments):
    rows = []
    seeds = {(u["board"], u["label"]): u for u in json.loads((SCRATCH / "seeds/units-r2.5.json").read_text())}
    for board_dir in sorted((SCRATCH / "census1/extract").iterdir()):
        board = board_dir.name
        entries = [("designer", board_dir / "designer.json", None), ("ours", board_dir / "ours-placed.json", None)]
        for path in sorted((SCRATCH / "seeds/extract" / board).glob("s*.json")):
            unit = seeds.get((board, path.stem), {})
            entries.append((path.stem, path, unit.get("first_open")))
        for label, path, first_open in entries:
            row = {"board": board, "label": label, "first_open": first_open, **measure(json.loads(path.read_text()))}
            rows.append(row)
            print(board, label, first_open, {k: round(v, 1) for k, v in row.items() if k in MEASURES}, flush=True)
    Path(arguments.out).write_text(json.dumps(rows, indent=1))


def posed(cuts, placed, seed, index, cache):
    """The placed board extracted with the traced seed's poses at stage
    `index` (parts matched as `REF#n`, the n-th footprint of a reference in
    board order: copies share references)."""
    if cache.exists():
        return json.loads(cache.read_text())
    import pcbnew  # noqa: PLC0415

    poses, seen = {}, {}
    for part in seed["parts"]:
        n = seen.get(part["reference"], 0)
        seen[part["reference"]] = n + 1
        poses[(part["reference"], n)] = part["poses"][index]
    board = pcbnew.LoadBoard(str(placed))
    seen = {}
    for footprint in board.GetFootprints():
        reference = footprint.GetReference()
        n = seen.get(reference, 0)
        seen[reference] = n + 1
        pose = poses.get((reference, n))
        if pose is not None:
            footprint.SetPosition(pcbnew.VECTOR2I(int(round(pose[0] * 1e6)), int(round(pose[1] * 1e6))))
            footprint.SetOrientationDegrees(pose[2])
    cache.parent.mkdir(parents=True, exist_ok=True)
    temporary = cache.with_suffix(".kicad_pcb")
    board.Save(str(temporary))
    data = cuts.extract(temporary)
    temporary.unlink()
    cache.write_text(json.dumps(data))
    return data


def traces(arguments):
    sys.path.insert(0, str(Path(__file__).resolve().parents[1] / "cut-census"))
    import cuts  # noqa: PLC0415  (its extraction needs KiCad)
    import labelled  # noqa: PLC0415

    runs = Path(arguments.runs)
    routes = labelled.results(runs.parent)
    rows = []
    cache = Path(arguments.out).with_suffix(".extract")
    for board_dir in sorted(p for p in runs.iterdir() if p.is_dir()):
        board = board_dir.name
        short = board.split("__")[-1]
        designer = labelled.designer_board(board)
        data = cuts.extracted(designer, cache / board / "designer-raw.json")
        first = routes.get(f"{short}-designer-s1", {}).get("first_open")
        rows.append({"board": board, "label": "designer-raw", "first_open": first, **measure(data)})
        for variant in sorted(p for p in board_dir.iterdir() if (p / "trace.json").exists()):
            trace = json.loads((variant / "trace.json").read_text())
            placed = next((variant / "placed").glob("*.kicad_pcb"))
            for seed in trace["seeds"]:
                index = seed["stages"].index("final")
                label = f"{variant.name}@{seed['seed']}"
                data = posed(cuts, placed, seed, index, cache / board / f"{label}.json")
                first = routes.get(f"{short}-{variant.name}-at-{seed['seed']}-s1", {}).get("first_open")
                if first is None and seed["kept"]:
                    first = routes.get(f"{short}-{variant.name}-at-kept-s1", {}).get("first_open")
                rows.append({"board": board, "label": label, "kept": seed["kept"], "first_open": first,
                             **measure(data)})
                print(board, label, first, flush=True)
    Path(arguments.out).write_text(json.dumps(rows, indent=1))


def ranks(values):
    order = sorted(range(len(values)), key=lambda i: values[i])
    result = [0.0] * len(values)
    i = 0
    while i < len(order):
        j = i
        while j + 1 < len(order) and values[order[j + 1]] == values[order[i]]:
            j += 1
        for k in range(i, j + 1):
            result[order[k]] = (i + j) / 2.0
        i = j + 1
    return result


def spearman(xs, ys):
    if len(xs) < 3 or len(set(xs)) < 2 or len(set(ys)) < 2:
        return None
    rx, ry = ranks(xs), ranks(ys)
    mx, my = statistics.mean(rx), statistics.mean(ry)
    num = sum((a - mx) * (b - my) for a, b in zip(rx, ry))
    den = math.sqrt(sum((a - mx) ** 2 for a in rx) * sum((b - my) ** 2 for b in ry))
    return num / den if den else None


def designer_position(rows, board, measure_name):
    seeds = [r for r in rows if r["board"] == board and r["label"].startswith("s")]
    designer = next(r for r in rows if r["board"] == board and r["label"] == "designer")
    values = sorted(r[measure_name] for r in seeds)
    spread = values[-1] - values[0]
    d = designer[measure_name]
    return {"designer": d, "median": statistics.median(values), "spread": spread,
            "below": sum(1 for v in values if v < d), "seeds": len(values),
            "spreads": (d - statistics.median(values)) / spread if spread else None}


def table(arguments):
    rows = json.loads(Path(arguments.data).read_text())
    boards = sorted({r["board"] for r in rows})
    print("| board | " + " | ".join(MEASURES) + " |")
    print("| --- " * (len(MEASURES) + 1) + "|")
    for board in boards:
        cells = []
        for m in MEASURES:
            p = designer_position(rows, board, m)
            ratio = p["designer"] / p["median"] if p["median"] else float("nan")
            cells.append(f"{ratio:.2f} ({p['below']}/{p['seeds']})")
        print(f"| {board.split('__')[-1]} | " + " | ".join(cells) + " |")
    print()
    print("| measure | " + " | ".join(b.split("__")[-1] for b in boards) + " | mean |")
    print("| --- " * (len(boards) + 2) + "|")
    for m in MEASURES:
        values = []
        cells = []
        for board in boards:
            seeds = [r for r in rows if r["board"] == board and r["label"].startswith("s") and r["first_open"] is not None]
            rho = spearman([r[m] for r in seeds], [r["first_open"] for r in seeds])
            cells.append("" if rho is None else f"{rho:+.2f}")
            if rho is not None:
                values.append(rho)
        mean = statistics.mean(values) if values else float("nan")
        print(f"| {m} | " + " | ".join(cells) + f" | {mean:+.2f} ({len(values)}) |")


def main():
    parser = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    sub = parser.add_subparsers(dest="command", required=True)
    c = sub.add_parser("compute")
    c.add_argument("--out", required=True)
    r = sub.add_parser("traces")
    r.add_argument("--runs", required=True)
    r.add_argument("--out", required=True)
    t = sub.add_parser("table")
    t.add_argument("--data", required=True)
    arguments = parser.parse_args()
    {"compute": compute, "traces": traces, "table": table}[arguments.command](arguments)


if __name__ == "__main__":
    sys.exit(main())
