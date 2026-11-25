# Copyright (C) 2026 Florian Loitsch.
"""placeview: what a placement looks like to the router, before routing.

A lab prototype for docs/experiments/evaluation-representations.md. It reads
a placed (unrouted) board with KiCad 10's pcbnew and writes

- metrics (JSON): the ratsnest's length and crossings, a coarse map of
  routing demand against room with its fullest cells and the parts there,
  and per many-pin part how much of the ring around it other parts' pads
  close;
- a picture (PNG): the parts, pads and ratsnest on the board, with the
  fullest cells shaded.

Usage: placeview.py BOARD.kicad_pcb --out STEM [--label NAME] [--cell 2.0]
"""

import argparse
import json
import math
import os

import numpy as np
from PIL import Image, ImageDraw, ImageFont
from scipy import ndimage
from scipy.sparse import coo_matrix
from scipy.sparse.csgraph import minimum_spanning_tree

import pcbnew

FONT = "/usr/share/fonts/TTF/DejaVuSans.ttf"


def mm(v):
    return v / 1e6


def read(path):
    board = pcbnew.LoadBoard(path)
    ds = board.GetDesignSettings()
    layers = ds.GetCopperLayerCount()
    box = board.GetBoardEdgesBoundingBox()
    out = {
        "layers": layers,
        "box": [mm(box.GetLeft()), mm(box.GetTop()), mm(box.GetRight()), mm(box.GetBottom())],
        "parts": [],
        "pads": [],
    }
    nc = board.FindNet("GND")
    default = None
    for net in board.GetNetInfo().NetsByName().values():
        c = net.GetNetClassSlow()
        if c and c.GetName() == "Default":
            default = c
            break
    out["track"] = mm(default.GetTrackWidth()) if default else 0.2
    out["clearance"] = max(mm(default.GetClearance()) if default else 0.2, mm(ds.m_MinClearance))
    del nc
    zone_nets = {z.GetNetname() for z in board.Zones() if not z.GetIsRuleArea()}
    out["pour_nets"] = sorted(n for n in zone_nets if n)
    for fp in board.GetFootprints():
        pads = []
        for pad in fp.Pads():
            bb = pad.GetBoundingBox()
            pos = pad.GetPosition()
            on_f = pad.IsOnLayer(pcbnew.F_Cu)
            on_b = pad.IsOnLayer(pcbnew.B_Cu)
            if not (on_f or on_b):
                continue
            pads.append(
                {
                    "ref": fp.GetReference(),
                    "number": pad.GetNumber(),
                    "net": pad.GetNetname(),
                    "x": mm(pos.x),
                    "y": mm(pos.y),
                    "box": [mm(bb.GetLeft()), mm(bb.GetTop()), mm(bb.GetRight()), mm(bb.GetBottom())],
                    "front": on_f,
                    "back": on_b,
                }
            )
        if not pads:
            continue
        x0 = min(p["box"][0] for p in pads)
        y0 = min(p["box"][1] for p in pads)
        x1 = max(p["box"][2] for p in pads)
        y1 = max(p["box"][3] for p in pads)
        out["parts"].append(
            {
                "ref": fp.GetReference(),
                "value": fp.GetValue(),
                "back": fp.GetLayer() == pcbnew.B_Cu,
                "pads": len(pads),
                "box": [x0, y0, x1, y1],
                "through": any(p["front"] and p["back"] for p in pads),
            }
        )
        out["pads"].extend(pads)
    return out


def ratsnest(b):
    """A minimum spanning tree per net over its pads' centres."""
    nets = {}
    for p in b["pads"]:
        if p["net"] and p["net"] not in b["pour_nets"]:
            nets.setdefault(p["net"], []).append(p)
    edges = []
    for net, pads in nets.items():
        if len(pads) < 2:
            continue
        pts = np.array([(p["x"], p["y"]) for p in pads])
        d = np.hypot(pts[:, None, 0] - pts[None, :, 0], pts[:, None, 1] - pts[None, :, 1]) + 1e-6
        t = minimum_spanning_tree(coo_matrix(d)).tocoo()
        for i, j in zip(t.row, t.col):
            edges.append((net, pts[i], pts[j], pads[i]["ref"], pads[j]["ref"]))
    return edges


def crossings(edges):
    n = 0
    segs = [(e[1], e[2], e[0]) for e in edges]
    a = np.array([s[0] for s in segs])
    b = np.array([s[1] for s in segs])
    for i in range(len(segs)):
        p, r = a[i], b[i] - a[i]
        q, s = a[i + 1 :], b[i + 1 :] - a[i + 1 :]
        rxs = r[0] * s[:, 1] - r[1] * s[:, 0]
        qp = q - p
        with np.errstate(divide="ignore", invalid="ignore"):
            t = (qp[:, 0] * s[:, 1] - qp[:, 1] * s[:, 0]) / rxs
            u = (qp[:, 0] * r[1] - qp[:, 1] * r[0]) / rxs
        hit = (np.abs(rxs) > 1e-9) & (t > 0.01) & (t < 0.99) & (u > 0.01) & (u < 0.99)
        n += int(hit.sum())
    return n


def demand_map(b, edges, cell):
    x0, y0, x1, y1 = b["box"]
    w = max(1, int(math.ceil((x1 - x0) / cell)))
    h = max(1, int(math.ceil((y1 - y0) / cell)))
    demand = np.zeros((h, w))
    for _, p, q, _, _ in edges:
        length = math.hypot(*(q - p))
        n = max(1, int(length / (cell / 4)))
        for k in range(n + 1):
            x = p[0] + (q[0] - p[0]) * k / n
            y = p[1] + (q[1] - p[1]) * k / n
            cx, cy = int((x - x0) / cell), int((y - y0) / cell)
            if 0 <= cx < w and 0 <= cy < h:
                demand[cy, cx] += length / (n + 1)
    demand /= cell  # tracks crossing a cell, roughly
    # Room: per outer layer the share of the cell not under pads (grown by the
    # clearance), in tracks across the cell; inner layers count in full.
    fine = 0.1
    fw, fh = int(w * cell / fine), int(h * cell / fine)
    occ = [np.zeros((fh, fw), bool), np.zeros((fh, fw), bool)]
    g = b["clearance"]
    for p in b["pads"]:
        bx = p["box"]
        ix0, iy0 = int((bx[0] - g - x0) / fine), int((bx[1] - g - y0) / fine)
        ix1, iy1 = int((bx[2] + g - x0) / fine) + 1, int((bx[3] + g - y0) / fine) + 1
        for side, on in ((0, p["front"]), (1, p["back"])):
            if on:
                occ[side][max(0, iy0) : max(0, iy1), max(0, ix0) : max(0, ix1)] = True
    k = int(cell / fine)
    free = np.zeros((h, w))
    for side in (0, 1):
        f = 1 - occ[side][: h * k, : w * k].reshape(h, k, w, k).mean(axis=(1, 3))
        free += f
    free += max(0, b["layers"] - 2)
    room = free * cell / (b["track"] + b["clearance"])
    return demand, room


def ring_closed(b, ring=1.5):
    """Per part of ten pads or more: the share of the ring around its pads
    that other parts' pads (grown by the clearance) cover, on its side."""
    out = []
    g = b["clearance"]
    for part in b["parts"]:
        if part["pads"] < 10:
            continue
        x0, y0, x1, y1 = part["box"]
        if (x1 - x0) * (y1 - y0) > 0.25 * (b["box"][2] - b["box"][0]) * (b["box"][3] - b["box"][1]):
            continue
        fine = 0.05
        w = int((x1 - x0 + 2 * ring) / fine)
        h = int((y1 - y0 + 2 * ring) / fine)
        m = np.zeros((h, w), bool)
        ox, oy = x0 - ring, y0 - ring
        near = set()
        for p in b["pads"]:
            if p["ref"] == part["ref"]:
                continue
            if not ((p["back"] and part["back"]) or (p["front"] and not part["back"]) or part["through"]):
                continue
            bx = p["box"]
            if bx[2] + g < ox or bx[0] - g > ox + w * fine or bx[3] + g < oy or bx[1] - g > oy + h * fine:
                continue
            ix0, iy0 = int((bx[0] - g - ox) / fine), int((bx[1] - g - oy) / fine)
            ix1, iy1 = int((bx[2] + g - ox) / fine) + 1, int((bx[3] + g - oy) / fine) + 1
            m[max(0, iy0) : max(0, iy1), max(0, ix0) : max(0, ix1)] = True
            near.add(p["ref"])
        inner = np.zeros((h, w), bool)
        k = int(ring / fine)
        inner[k : h - k, k : w - k] = True
        ringmask = ~inner
        out.append(
            {
                "ref": part["ref"],
                "value": part["value"],
                "pads": part["pads"],
                "side": "back" if part["back"] else "front",
                "at": [round((x0 + x1) / 2, 1), round((y0 + y1) / 2, 1)],
                "ring_closed": round(float(m[ringmask].mean()), 2),
                "parts_in_ring": len(near),
            }
        )
    out.sort(key=lambda e: -e["pads"])
    return out


def metrics(b, cell):
    edges = ratsnest(b)
    demand, room = demand_map(b, edges, cell)
    ratio = demand / np.maximum(room, 0.5)
    x0, y0 = b["box"][0], b["box"][1]
    hot = []
    order = np.dstack(np.unravel_index(np.argsort(-ratio, axis=None), ratio.shape))[0]
    for cy, cx in order[:12]:
        if ratio[cy, cx] < 0.5:
            break
        cxm, cym = x0 + (cx + 0.5) * cell, y0 + (cy + 0.5) * cell
        parts = sorted(
            b["parts"],
            key=lambda p: max(p["box"][0] - cxm, cxm - p["box"][2], 0) + max(p["box"][1] - cym, cym - p["box"][3], 0),
        )[:4]
        hot.append(
            {
                "at": [round(cxm, 1), round(cym, 1)],
                "demand_tracks": round(float(demand[cy, cx]), 1),
                "room_tracks": round(float(room[cy, cx]), 1),
                "fill": round(float(ratio[cy, cx]), 2),
                "parts": [p["ref"] for p in parts],
            }
        )
    lengths = [math.hypot(*(e[2] - e[1])) for e in edges]
    area = (b["box"][2] - b["box"][0]) * (b["box"][3] - b["box"][1])
    part_area = sum((p["box"][2] - p["box"][0]) * (p["box"][3] - p["box"][1]) for p in b["parts"])
    rings = ring_closed(b)
    m = {
        "board_mm": [round(b["box"][2] - b["box"][0], 1), round(b["box"][3] - b["box"][1], 1)],
        "copper_layers": b["layers"],
        "default_track_mm": b["track"],
        "default_clearance_mm": b["clearance"],
        "parts": len(b["parts"]),
        "parts_on_back": sum(1 for p in b["parts"] if p["back"]),
        "pads_area_share_of_board": round(part_area / area, 2),
        "ratsnest": {
            "connections": len(edges),
            "length_mm": round(sum(lengths)),
            "crossings": crossings(edges),
            "longest_mm": [round(v, 1) for v in sorted(lengths)[-5:]],
            "nets_left_to_pours": b["pour_nets"],
        },
        "demand_against_room": {
            "cell_mm": cell,
            "cells": int(ratio.size),
            "cells_over_half_full": int((ratio > 0.5).sum()),
            "cells_over_full": int((ratio > 1.0).sum()),
            "mean_fill_of_the_fullest_tenth": round(float(np.sort(ratio, axis=None)[-max(1, ratio.size // 10) :].mean()), 2),
            "fullest_cells": hot,
        },
        "many_pin_parts": {
            "note": "ring_closed: the share of the 1.5 mm ring around the part's pads covered by other parts' pads and their clearance, on the part's side",
            "mean_ring_closed": round(float(np.mean([r["ring_closed"] for r in rings])), 2) if rings else None,
            "parts": rings[:25],
        },
    }
    return m, edges, ratio


def render(b, edges, ratio, cell, out, label):
    x0, y0, x1, y1 = b["box"]
    scale = min(1500 / (x1 - x0), 1500 / (y1 - y0), 40)
    W, H = int((x1 - x0) * scale) + 20, int((y1 - y0) * scale) + 60
    img = Image.new("RGB", (W, H), (16, 20, 24))
    d = ImageDraw.Draw(img, "RGBA")
    f = ImageFont.truetype(FONT, 13)
    fb = ImageFont.truetype(FONT, 18)

    def pt(x, y):
        return (10 + (x - x0) * scale, 50 + (y - y0) * scale)

    d.rectangle([pt(x0, y0), pt(x1, y1)], fill=(28, 34, 40), outline=(200, 200, 200))
    for cy in range(ratio.shape[0]):
        for cx in range(ratio.shape[1]):
            r = ratio[cy, cx]
            if r > 0.5:
                a = int(min(1.0, (r - 0.5) / 1.5) * 150) + 40
                d.rectangle(
                    [pt(x0 + cx * cell, y0 + cy * cell), pt(x0 + (cx + 1) * cell, y0 + (cy + 1) * cell)],
                    fill=(255, 60, 40, a),
                )
    for part in b["parts"]:
        bx = part["box"]
        col = (90, 140, 220) if part["back"] else (220, 170, 80)
        d.rectangle([pt(bx[0], bx[1]), pt(bx[2], bx[3])], outline=col + (255,), fill=col + (50,))
    for p in b["pads"]:
        bx = p["box"]
        col = (120, 170, 255) if (p["back"] and not p["front"]) else (255, 205, 110)
        d.rectangle([pt(bx[0], bx[1]), pt(bx[2], bx[3])], fill=col + (255,))
    for _, p, q, _, _ in edges:
        d.line([pt(*p), pt(*q)], fill=(230, 230, 230, 110), width=1)
    placed = []
    for part in sorted(b["parts"], key=lambda p: -p["pads"]):
        bx = part["box"]
        if (bx[2] - bx[0]) * scale < 26 and part["pads"] < 6:
            continue
        cx, cy = pt((bx[0] + bx[2]) / 2, (bx[1] + bx[3]) / 2)
        tw = d.textlength(part["ref"], font=f)
        r = [cx - tw / 2, cy - 8, cx + tw / 2, cy + 8]
        if any(not (r[2] < q[0] or r[0] > q[2] or r[3] < q[1] or r[1] > q[3]) for q in placed):
            continue
        placed.append(r)
        d.text((r[0], r[1]), part["ref"], font=f, fill=(255, 255, 255), stroke_width=2, stroke_fill=(0, 0, 0))
    d.text((10, 4), f"placement {label}: parts (orange front, blue back), pads, ratsnest (white), cells where the ratsnest's demand passes half the room (red, darker = fuller)", font=f, fill=(230, 230, 230))
    d.text((10, 24), f"board {x1 - x0:.1f} x {y1 - y0:.1f} mm, {b['layers']} copper layers, {len(b['parts'])} parts", font=fb, fill=(230, 230, 230))
    img.save(out)


def main():
    ap = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    ap.add_argument("board")
    ap.add_argument("--out", required=True)
    ap.add_argument("--label", default="")
    ap.add_argument("--cell", type=float, default=2.0)
    args = ap.parse_args()
    b = read(args.board)
    m, edges, ratio = metrics(b, args.cell)
    os.makedirs(os.path.dirname(os.path.abspath(args.out)), exist_ok=True)
    m = {"placement": args.label, **m}
    json.dump(m, open(args.out + ".json", "w"), indent=1)
    render(b, edges, ratio, args.cell, args.out + ".png", args.label)
    print(json.dumps({k: m[k] for k in ("placement", "parts", "ratsnest")}))


if __name__ == "__main__":
    main()
