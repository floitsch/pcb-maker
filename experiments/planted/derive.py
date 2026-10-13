#!/usr/bin/env python3
# Copyright (C) 2026 Toit contributors.
"""Derives a planted board from a routed real board: the routed copper
(the designer's, or a passing result of our tracer) is the planted
solution, and netless blockers are dropped into the space it leaves, on
the straight lines between the pads of detoured nets first, keeping
`corridor` free tracks beside every piece of copper.

    experiments/planted/derive.py <routed.kicad_pcb> <out-dir> [--density D] [--corridor K]
        [--board-id ID] [--seed S] [--source-kind designer|ours]

Copper pours stay; a blocker may cut into a pour's fill, so the result is
checked with KiCad (zones refilled) against the source board: blockers are
taken out again until the unconnected count and the copper findings are
no worse than the source's. Writes the board, its project, planted.json."""

import argparse
import json
import math
import random
import re
import shutil
import subprocess
import sys
import time
from pathlib import Path

import numpy as np

HERE = Path(__file__).resolve().parent
sys.path.insert(0, str(HERE))

import grid as G  # noqa: E402
from planted import Library, nm, mm  # noqa: E402
from stats import board_stats, mst_length, point_in_polygon  # noqa: E402

COSMETIC = re.compile(r"^(silk_|text_|lib_footprint|footprint_type_mismatch|missing_courtyard|isolated_copper|nonmirrored_text)")
BLOCKERS = "MountingHole_2.2mm_M2_DIN965,TestPoint_Pad_D1.0mm,C_0805_2012Metric,R_0603_1608Metric,C_1206_3216Metric,PinHeader_1x02_P2.54mm_Vertical"


def drc(board_path):
    report = board_path.with_suffix(".drc.json")
    subprocess.run(["kicad-cli", "pcb", "drc", "--format", "json", "--severity-all", "--refill-zones",
                    "--output", str(report), str(board_path)], stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL,
                   timeout=900)
    data = json.loads(report.read_text())
    errors = {}
    for v in data.get("violations", []):
        if v.get("severity") == "error" and not COSMETIC.match(v["type"]):
            errors[v["type"]] = errors.get(v["type"], 0) + 1
    return {"errors": errors, "unconnected": len(data.get("unconnected_items", []))}, data


def no_worse(result, base):
    if result["unconnected"] > base["unconnected"]:
        return False
    for kind, count in result["errors"].items():
        if count > base["errors"].get(kind, 0):
            return False
    return True


class Deriver:
    def __init__(self, source, args):
        import pcbnew
        self.pcbnew = pcbnew
        self.args = args
        self.rng = random.Random(args.seed)
        self.board = pcbnew.LoadBoard(str(source))
        self.library = Library()
        ds = self.board.GetDesignSettings()
        netclass = ds.m_NetSettings.GetDefaultNetclass()
        self.clearance = mm(netclass.GetClearance())
        self.track = mm(netclass.GetTrackWidth())
        self.edge = max(mm(ds.m_CopperEdgeClearance), 0.3)
        self.hole_clearance = max(mm(ds.m_HoleClearance), 0.25)
        self.layers = list(self.board.GetEnabledLayers().CuStack())
        polys = pcbnew.SHAPE_POLY_SET()
        self.board.GetBoardPolygonOutlines(polys, True)
        self.outline = []
        for i in range(polys.OutlineCount()):
            line = polys.Outline(i)
            self.outline.append([(mm(line.CPoint(k).x), mm(line.CPoint(k).y)) for k in range(line.PointCount())])
        xs = [p[0] for o in self.outline for p in o]
        ys = [p[1] for o in self.outline for p in o]
        pitch = args.pitch
        self.grid = G.Grid(min(xs) - 1, min(ys) - 1, max(xs) + 1, max(ys) + 1, pitch, len(self.layers))
        g = self.grid
        X, Y = np.meshgrid(g.x0 + np.arange(g.nx) * pitch, g.y0 + np.arange(g.ny) * pitch)
        inside = np.zeros(X.shape, dtype=bool)
        for o in self.outline:
            inside ^= point_in_polygon(X, Y, o)
        self.inside = inside
        self.blocked = np.zeros((len(self.layers), g.ny, g.nx), dtype=bool)  # copper and keep-outs
        self.court = np.zeros((2, g.ny, g.nx), dtype=bool)
        self._rasterize()

    def layer_index(self, layer):
        return self.layers.index(layer) if layer in self.layers else None

    def _rasterize(self):
        pcbnew = self.pcbnew
        g = self.grid
        for fp in self.board.GetFootprints():
            fp.BuildCourtyardCaches()
            for side, layer in ((0, pcbnew.F_CrtYd), (1, pcbnew.B_CrtYd)):
                c = fp.GetCourtyard(layer)
                for i in range(c.OutlineCount()):
                    line = c.Outline(i)
                    pts = [(mm(line.CPoint(k).x), mm(line.CPoint(k).y)) for k in range(line.PointCount())]
                    if len(pts) >= 3:
                        g.mark(self.court[side], G.Shape("poly", points=pts), 0.0)
            if not any(fp.GetCourtyard(l).OutlineCount() for l in (pcbnew.F_CrtYd, pcbnew.B_CrtYd)):
                box = fp.GetBoundingBox(False)
                pts = [(mm(box.GetLeft()), mm(box.GetTop())), (mm(box.GetRight()), mm(box.GetTop())),
                       (mm(box.GetRight()), mm(box.GetBottom())), (mm(box.GetLeft()), mm(box.GetBottom()))]
                g.mark(self.court[1 if fp.IsFlipped() else 0], G.Shape("poly", points=pts), 0.0)
            for pad in fp.Pads():
                self._mark_pad(pad)
        for item in self.board.GetTracks():
            cls = item.GetClass()
            if cls == "PCB_VIA":
                p = item.GetPosition()
                disc = G.Shape("disc", centre=(mm(p.x), mm(p.y)), radius=mm(item.GetWidth(pcbnew.F_Cu)) / 2)
                for i in range(len(self.layers)):
                    g.mark(self.blocked[i], disc, 0.75 * g.pitch)
                continue
            li = self.layer_index(item.GetLayer())
            if li is None:
                continue
            if cls == "PCB_ARC":
                s, m, e = item.GetStart(), item.GetMid(), item.GetEnd()
                pts = [(mm(s.x), mm(s.y)), (mm(m.x), mm(m.y)), (mm(e.x), mm(e.y))]
            else:
                s, e = item.GetStart(), item.GetEnd()
                pts = [(mm(s.x), mm(s.y)), (mm(e.x), mm(e.y))]
            for a, b in zip(pts, pts[1:]):
                g.mark(self.blocked[li], G.Shape("capsule", a=a, b=b, half=mm(item.GetWidth()) / 2), 0.75 * g.pitch)
        # Rule areas are no-go on their layers; copper zones are not (their
        # fill gives way, and KiCad's check decides).
        for zone in self.board.Zones():
            if not zone.GetIsRuleArea():
                continue
            outline = zone.Outline()
            for i in range(outline.OutlineCount()):
                line = outline.Outline(i)
                pts = [(mm(line.CPoint(k).x), mm(line.CPoint(k).y)) for k in range(line.PointCount())]
                for layer in zone.GetLayerSet().CuStack():
                    li = self.layer_index(layer)
                    if li is not None and len(pts) >= 3:
                        g.mark(self.blocked[li], G.Shape("poly", points=pts), 0.5)
        # Graphic copper (logos, texts) is copper too.
        for drawing in self.board.GetDrawings():
            li = self.layer_index(drawing.GetLayer())
            if li is None:
                continue
            box = drawing.GetBoundingBox()
            pts = [(mm(box.GetLeft()), mm(box.GetTop())), (mm(box.GetRight()), mm(box.GetTop())),
                   (mm(box.GetRight()), mm(box.GetBottom())), (mm(box.GetLeft()), mm(box.GetBottom()))]
            g.mark(self.blocked[li], G.Shape("poly", points=pts), 0.0)

    def _pad_shape(self, pad, layer):
        pcbnew = self.pcbnew
        poly = pcbnew.SHAPE_POLY_SET()
        pad.TransformShapeToPolygon(poly, layer, 0, nm(0.002), pcbnew.ERROR_OUTSIDE)
        if poly.OutlineCount() == 0:
            return None
        line = poly.Outline(0)
        pts = [(mm(line.CPoint(k).x), mm(line.CPoint(k).y)) for k in range(line.PointCount())]
        return G.Shape("poly", points=pts) if len(pts) >= 3 else None

    def _mark_pad(self, pad):
        g = self.grid
        for layer in pad.GetLayerSet().CuStack():
            li = self.layer_index(layer)
            if li is None:
                continue
            shape = self._pad_shape(pad, layer)
            if shape is not None:
                g.mark(self.blocked[li], shape, 0.75 * g.pitch)
        if pad.HasHole():
            d = pad.GetDrillSize()
            p = pad.GetPosition()
            hole = G.Shape("disc", centre=(mm(p.x), mm(p.y)), radius=mm(max(d.x, d.y)) / 2)
            for i in range(len(self.layers)):
                g.mark(self.blocked[i], hole, self.hole_clearance - self.clearance + 0.75 * g.pitch)

    # Blockers ------------------------------------------------------------
    def try_blocker(self, name, x, y, rotation, back):
        pcbnew = self.pcbnew
        g = self.grid
        fp = pcbnew.FOOTPRINT(self.library.footprints[name])
        fp.SetPosition(pcbnew.VECTOR2I(nm(x), nm(y)))
        fp.SetOrientationDegrees(rotation)
        if back:
            fp.Flip(fp.GetPosition(), pcbnew.FLIP_DIRECTION_TOP_BOTTOM)
        fp.BuildCourtyardCaches()
        marks = []
        for side, layer in ((0, pcbnew.F_CrtYd), (1, pcbnew.B_CrtYd)):
            c = fp.GetCourtyard(layer)
            for i in range(c.OutlineCount()):
                line = c.Outline(i)
                pts = [(mm(line.CPoint(k).x), mm(line.CPoint(k).y)) for k in range(line.PointCount())]
                index, m = g.mask(G.Shape("poly", points=pts), 0.06)
                if index is None or (self.court[side][index] & m).any() or not (self.inside[index][m]).all():
                    return None
                marks.append((side, index, m))
        margin = self.args.corridor * (self.track + self.clearance)
        grow = self.clearance + margin + g.pitch * 0.75
        pad_marks = []
        for pad in fp.Pads():
            hole_grow = 0.0
            if pad.HasHole():
                hole_grow = self.hole_clearance - self.clearance
            layers = [layer for layer in pad.GetLayerSet().CuStack() if layer in self.layers]
            if pad.GetAttribute() == pcbnew.PAD_ATTRIB_NPTH:
                layers = list(self.layers)
            for layer in layers:
                li = self.layer_index(layer)
                shape = self._pad_shape(pad, layer)
                if shape is None:
                    p = pad.GetPosition()
                    d = pad.GetDrillSize()
                    shape = G.Shape("disc", centre=(mm(p.x), mm(p.y)), radius=mm(max(d.x, d.y)) / 2)
                index, m = g.mask(shape, grow + max(hole_grow, 0.0))
                if index is None or (self.blocked[li][index] & m).any():
                    return None
                # Edge clearance: every node within edge + a pitch is inside.
                index2, m2 = g.mask(shape, self.edge + g.pitch)
                if not self.inside[index2][m2].all():
                    return None
                index3, m3 = g.mask(shape, 0.75 * g.pitch)
                pad_marks.append((li, index3, m3))
        for side, index, m in marks:
            self.court[side][index] |= m
        for li, index, m in pad_marks:
            self.blocked[li][index] |= m
        fp.SetReference(f"BLK{len(self.placed) + 1}")
        for pad in fp.Pads():
            pad.SetNetCode(0)
        self.board.Add(fp)
        self.placed.append(fp)
        return fp

    def lines(self):
        """Straight lines between the pads of detoured, routed signal nets."""
        stats = board_stats(self.source_path)
        detoured = {r["net"] for r in stats["per_net"] if r.get("detour", 0) > self.args.min_detour}
        pads = {}
        for fp in self.board.GetFootprints():
            for pad in fp.Pads():
                if pad.GetNetname() in detoured:
                    p = pad.GetPosition()
                    pads.setdefault(pad.GetNetname(), []).append((mm(p.x), mm(p.y)))
        out = []
        for name, pts in pads.items():
            # MST edges.
            n = len(pts)
            inside = [0]
            rest = list(range(1, n))
            while rest:
                best = min(((math.dist(pts[i], pts[j]), i, j) for i in inside for j in rest))
                _, i, j = best
                if math.dist(pts[i], pts[j]) > 2.0:
                    out.append((pts[i], pts[j]))
                inside.append(j)
                rest.remove(j)
        return out, stats

    def drop(self):
        g = self.grid
        rng = self.rng
        a = self.args
        names = a.blockers.split(",")
        self.placed = []
        lines, stats = self.lines()
        area = float(self.inside.sum()) * g.pitch * g.pitch
        attempts = min(int(a.density * area / 4), a.max_attempts)
        free_nodes = None
        for k in range(attempts):
            if lines and k < attempts * a.straight_share:
                (ax, ay), (bx, by) = rng.choice(lines)
                t = rng.uniform(0.2, 0.8)
                x, y = ax + t * (bx - ax), ay + t * (by - ay)
            else:
                if free_nodes is None or k % 100 == 0:
                    free = self.inside & ~self.court[0]
                    for layer in range(len(self.layers)):
                        free &= ~self.blocked[layer]
                    free_nodes = np.nonzero(free)
                iy, ix = free_nodes
                if len(ix) == 0:
                    break
                j = rng.randrange(len(ix))
                x, y = g.x(int(ix[j])), g.y(int(iy[j]))
            x = round(x / 0.05) * 0.05
            y = round(y / 0.05) * 0.05
            name = rng.choice(names)
            back = rng.random() < 0.3
            self.try_blocker(name, x, y, rng.choice([0, 90]), back)
        return stats


def main():
    parser = argparse.ArgumentParser()
    parser.add_argument("source", type=Path)
    parser.add_argument("out", type=Path)
    parser.add_argument("--board-id", default="planted")
    parser.add_argument("--source-kind", default="designer")
    parser.add_argument("--level", default="")
    parser.add_argument("--seed", type=int, default=1)
    parser.add_argument("--density", type=float, default=1.0, help="blocker attempts per 4 mm^2 of board")
    parser.add_argument("--straight-share", type=float, default=0.6)
    parser.add_argument("--min-detour", type=float, default=1.25)
    parser.add_argument("--corridor", type=float, default=1.0)
    parser.add_argument("--pitch", type=float, default=0.1)
    parser.add_argument("--max-attempts", type=int, default=4000)
    parser.add_argument("--blockers", default=BLOCKERS)
    parser.add_argument("--no-verify", action="store_true")
    args = parser.parse_args()
    started = time.process_time()
    pcbnew = __import__("pcbnew")
    args.out.mkdir(parents=True, exist_ok=True)
    board_path = args.out / f"{args.board_id}.kicad_pcb"
    deriver = Deriver(args.source, args)
    deriver.source_path = args.source
    stats = deriver.drop()
    pcbnew.SaveBoard(str(board_path), deriver.board)
    project = args.source.with_suffix(".kicad_pro")
    if project.exists():
        shutil.copy(project, args.out / f"{args.board_id}.kicad_pro")
    record = {"knobs": {**{k: (str(v) if isinstance(v, Path) else v) for k, v in vars(args).items()},
                        "mode": f"derived-{args.source_kind}", "layers": len(deriver.layers)},
              "blockers": len(deriver.placed)}
    if not args.no_verify:
        base_dir = args.out / "source-check"
        base_dir.mkdir(exist_ok=True)
        base_board = base_dir / f"{args.board_id}.kicad_pcb"
        shutil.copy(args.source, base_board)
        if project.exists():
            shutil.copy(project, base_dir / f"{args.board_id}.kicad_pro")
        base, _ = drc(base_board)
        record["source_drc"] = base
        result, data = drc(board_path)
        rounds = 0
        while not no_worse(result, base) and deriver.placed and rounds < 8:
            rounds += 1
            # Take out the blockers near what KiCad reports, else half of them.
            spots = []
            for v in data.get("violations", []) + data.get("unconnected_items", []):
                for item in v.get("items", []):
                    spots.append((item["pos"]["x"], item["pos"]["y"]))
            removed = []
            for fp in list(deriver.placed):
                p = fp.GetPosition()
                if any(math.dist((mm(p.x), mm(p.y)), s) < 6.0 for s in spots):
                    removed.append(fp)
            if not removed or rounds > 4:
                removed = deriver.placed[::2]
            for fp in removed:
                deriver.board.Remove(fp)
                deriver.placed.remove(fp)
            pcbnew.SaveBoard(str(board_path), deriver.board)
            result, data = drc(board_path)
        record["drc"] = result
        record["verify_rounds"] = rounds
        record["blockers"] = len(deriver.placed)
        shutil.rmtree(base_dir, ignore_errors=True)
    final = board_stats(board_path)
    signal = [r for r in final["per_net"] if not r["pour"]]
    record["planted"] = {"length_mm": round(sum(r["length_mm"] for r in final["per_net"]), 3),
                         "vias": sum(r["vias"] for r in final["per_net"])}
    record["netlist"] = {"nets": final["nets"], "connections": sum(r["pads"] - 1 for r in final["per_net"]),
                         "pour_nets": final["pour_nets"], "signal_nets": len(signal)}
    record["derive_cpu_seconds"] = round(time.process_time() - started, 1)
    (args.out / "planted.json").write_text(json.dumps(record, indent=1))
    print(json.dumps({k: v for k, v in record.items() if k != "knobs"}))


if __name__ == "__main__":
    main()
