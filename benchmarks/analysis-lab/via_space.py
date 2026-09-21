#!/usr/bin/env python3
# Copyright (C) 2026 Toit contributors.
"""Render explicit synthetic through-via center clearance space.

Uses public geometry only. No optimization, search, automatic crop selection,
native validation, or witness lookup is performed. Pillow draws analytic
Minkowski shapes with circular corners and caps.
"""

import argparse
import json
import math
from pathlib import Path
import subprocess

from PIL import Image, ImageChops, ImageDraw, ImageFont

from inspect_board import point_segment_distance, probe_via, route_summary

SCOPE = "Through-via copper-center clearance on top and bottom. Excludes connectivity, board-edge, drill, and full-route checks."
COLORS = {"top": (245, 178, 168), "bottom": (159, 199, 243), "both": (186, 155, 217)}


def blockers(board, net):
    """Actual continuous exclusion primitives, before any pixel rasterization."""
    via_radius = board["rules"]["via_diameter"]/2
    clearance = board["rules"]["clearance"]
    result = []
    for obstacle in board.get("obstacles", []):
        layers = sorted(set(obstacle["layers"]) & {"top", "bottom"})
        if layers:
            result.append({"id": obstacle["id"], "kind": "obstacle", "shape": "rounded_rect",
                "rect": obstacle["rect"], "radius": via_radius+clearance, "layers": layers})
    for pad in board["pads"]:
        if pad["net"] != net:
            result.append({"id": pad["id"], "kind": "pad", "shape": "circle", "at": pad["at"],
                "radius": via_radius+clearance+pad["diameter"]/2, "layers": pad["layers"]})
    for route in board["routes"]:
        if route["net"] == net:
            continue
        for index, (a, b) in enumerate(zip(route["points"], route["points"][1:])):
            result.append({"id": f"{route['id']}:s{index}", "kind": "trace", "shape": "capsule",
                "start": a, "end": b, "radius": via_radius+clearance+board["rules"]["trace_width"]/2,
                "layers": [route["layers"][index]]})
        for via in route_summary(route)["transitions"]:
            result.append({"id": f"{route['id']}:v{via['point_index']}", "kind": "via", "shape": "circle",
                "at": via["at"], "radius": 2*via_radius+clearance, "layers": ["top", "bottom"]})
    return sorted(result, key=lambda item: item["id"])


def shape_margin(shape, at):
    """Continuous signed clearance margin; negative means blocked."""
    if shape["shape"] == "rounded_rect":
        x0, y0, x1, y1 = shape["rect"]
        distance = math.hypot(max(x0-at[0], 0, at[0]-x1), max(y0-at[1], 0, at[1]-y1))
    elif shape["shape"] == "capsule":
        distance = point_segment_distance(at, shape["start"], shape["end"])
    else:
        distance = math.dist(at, shape["at"])
    return distance-shape["radius"]


def shape_bounds(shape):
    radius = shape["radius"]
    if shape["shape"] == "rounded_rect":
        x0, y0, x1, y1 = shape["rect"]
    elif shape["shape"] == "capsule":
        a, b = shape["start"], shape["end"]
        x0, y0, x1, y1 = min(a[0], b[0]), min(a[1], b[1]), max(a[0], b[0]), max(a[1], b[1])
    else:
        x0, y0 = shape["at"]
        x1, y1 = x0, y0
    return [x0-radius, y0-radius, x1+radius, y1+radius]


def intersects(a, b):
    return not (a[2] < b[0] or a[0] > b[2] or a[3] < b[1] or a[1] > b[3])


def make_masks(shapes, bounds, size, supersample=2):
    """Rasterize round rectangles/capsules without square corner inflation."""
    width, height = size
    large_size = (width*supersample, height*supersample)
    masks = {layer: Image.new("L", large_size, 0) for layer in ["top", "bottom"]}
    x0, y0, x1, y1 = bounds
    sx, sy = (large_size[0]-1)/(x1-x0), (large_size[1]-1)/(y1-y0)
    def xy(p):
        return ((p[0]-x0)*sx, (p[1]-y0)*sy)
    for shape in shapes:
        if not intersects(shape_bounds(shape), bounds):
            continue
        radius = shape["radius"]
        for layer in set(shape["layers"]) & set(masks):
            draw = ImageDraw.Draw(masks[layer])
            def circle(center):
                draw.ellipse([xy((center[0]-radius, center[1]-radius)),
                              xy((center[0]+radius, center[1]+radius))], fill=255)
            if shape["shape"] == "rounded_rect":
                a, b, c, d = shape["rect"]
                draw.rectangle([xy((a-radius, b)), xy((c+radius, d))], fill=255)
                draw.rectangle([xy((a, b-radius)), xy((c, d+radius))], fill=255)
                for center in [(a, b), (c, b), (c, d), (a, d)]:
                    circle(center)
            elif shape["shape"] == "capsule":
                a, b = shape["start"], shape["end"]
                length = math.dist(a, b)
                if length:
                    normal = (-(b[1]-a[1])*radius/length, (b[0]-a[0])*radius/length)
                    draw.polygon([xy((a[0]+normal[0], a[1]+normal[1])),
                                  xy((b[0]+normal[0], b[1]+normal[1])),
                                  xy((b[0]-normal[0], b[1]-normal[1])),
                                  xy((a[0]-normal[0], a[1]-normal[1]))], fill=255)
                circle(a)
                circle(b)
            else:
                circle(shape["at"])
    return {layer: mask.resize(size, Image.Resampling.LANCZOS) for layer, mask in masks.items()}


def ticks(lo, hi, count=6):
    target = (hi-lo)/count
    power = 10**math.floor(math.log10(target))
    step = next((factor*power for factor in [1, 2, 2.5, 5, 10] if factor*power >= target), 10*power)
    start, finish = math.ceil(lo/step-1e-10), math.floor(hi/step+1e-10)
    return [i*step for i in range(start, finish+1)], max(0, -math.floor(math.log10(step))+1)


def font(size):
    try:
        return ImageFont.truetype("DejaVuSans.ttf", size)
    except OSError:
        path = subprocess.check_output(["fc-match", "-f", "%{file}", "sans"], text=True).strip()
        return ImageFont.truetype(path, size)


def render(board, net, bounds, output, at=None, proposal_applied=False):
    if len(bounds) != 4 or any(not math.isfinite(value) for value in bounds):
        raise ValueError("Bounds must contain four finite coordinates")
    x0, y0, x1, y1 = bounds
    if x1 <= x0 or y1 <= y0:
        raise ValueError("Bounds must have positive extent")
    if at is not None and (len(at) != 2 or any(not math.isfinite(value) for value in at)):
        raise ValueError("Probe point must contain two finite coordinates")
    if net not in {item["net"] for section in ["routes", "pads"] for item in board[section]}:
        raise ValueError("Selected net is not present in the public board")
    shapes = blockers(board, net)
    visible = [shape for shape in shapes if intersects(shape_bounds(shape), bounds)]
    scale = min(820/(x1-x0), 640/(y1-y0))
    width, height = max(2, round((x1-x0)*scale)), max(2, round((y1-y0)*scale))
    plot_x, plot_y = 105, 100+(640-height)//2
    image = Image.new("RGB", (1390, 875), "white")
    draw = ImageDraw.Draw(image)
    normal, small, title = font(15), font(13), font(22)
    draw.text((30, 22), f"Through-via center space  |  {board['id']}  |  {net}", fill="#202020", font=title)
    draw.text((30, 57), f"Via diameter {board['rules']['via_diameter']:.6g} mm  ·  edge clearance {board['rules']['clearance']:.6g} mm  ·  +Y downward", fill="#444444", font=normal)
    masks = make_masks(shapes, bounds, (width, height))
    field = Image.new("RGB", (width, height), "white")
    field.paste(COLORS["top"], (0, 0, width, height), masks["top"])
    field.paste(COLORS["bottom"], (0, 0, width, height), masks["bottom"])
    field.paste(COLORS["both"], (0, 0, width, height), ImageChops.multiply(masks["top"], masks["bottom"]))
    fd = ImageDraw.Draw(field)
    def local(p):
        return ((p[0]-x0)*(width-1)/(x1-x0), (p[1]-y0)*(height-1)/(y1-y0))
    def global_xy(p):
        a, b = local(p)
        return (a+plot_x, b+plot_y)
    # Same-net centerlines provide context but never contribute to blockers.
    for route in board["routes"]:
        if route["net"] == net:
            for a, b in zip(route["points"], route["points"][1:]):
                fd.line([local(a), local(b)], fill="#bbbbbb", width=1)
    image.paste(field, (plot_x, plot_y))
    draw.rectangle([plot_x, plot_y, plot_x+width-1, plot_y+height-1], outline="#222222", width=1)
    xticks, xdigits = ticks(x0, x1)
    yticks, ydigits = ticks(y0, y1)
    for value in xticks:
        px, py = global_xy((value, y1))
        label = f"{value:.{xdigits}f}"
        draw.line([(px, py), (px, py+6)], fill="#333333")
        draw.text((px-draw.textlength(label, font=small)/2, py+9), label, fill="#333333", font=small)
    for value in yticks:
        px, py = global_xy((x0, value))
        label = f"{value:.{ydigits}f}"
        draw.line([(px-6, py), (px, py)], fill="#333333")
        draw.text((px-12-draw.textlength(label, font=small), py-7), label, fill="#333333", font=small)
    draw.text((plot_x+width/2-42, plot_y+height+35), "x position / mm", fill="#333333", font=small)
    draw.text((plot_x-83, plot_y-24), "y / mm", fill="#333333", font=small)
    sidebar_x = 970
    draw.text((sidebar_x, 99), "Excluded center positions", fill="#202020", font=normal)
    for index, (kind, label) in enumerate([("top", "Top-layer blocker"), ("bottom", "Bottom-layer blocker"), ("both", "Blocked on both layers")]):
        yy = 130+index*28
        draw.rectangle([sidebar_x, yy, sidebar_x+20, yy+16], fill=COLORS[kind], outline="#555555")
        draw.text((sidebar_x+30, yy), label, fill="#333333", font=normal)
    draw.text((sidebar_x, 225), "White: clear in this copper model", fill="#333333", font=small)
    draw.text((sidebar_x, 247), "Gray line: selected-net context", fill="#777777", font=small)
    draw.text((sidebar_x, 281), "Visible blockers", fill="#202020", font=normal)
    for index, shape in enumerate(visible[:14]):
        yy = 310+index*24
        layer_label = "+".join(shape["layers"])
        color = COLORS["both" if len(shape["layers"]) == 2 else shape["layers"][0]]
        draw.rectangle([sidebar_x, yy+2, sidebar_x+10, yy+12], fill=color, outline="#777777")
        draw.text((sidebar_x+18, yy), f"{shape['id']}  {layer_label}  {shape['kind']}", fill="#333333", font=small)
    if len(visible) > 14:
        draw.text((sidebar_x, 652), f"+ {len(visible)-14} blockers listed in JSON output", fill="#555555", font=small)
    # Exact crop coordinates remain visible even when nice tick spacing omits an endpoint.
    draw.text((30, 796), f"Crop x [{x0:.6f}, {x1:.6f}]  y [{y0:.6f}, {y1:.6f}] mm. Frame is the requested viewport.", fill="#333333", font=small)
    draw.text((30, 818), SCOPE, fill="#444444", font=small)
    context = "Supplied simultaneous proposal applied; no validator run." if proposal_applied else "Current supplied board geometry."
    draw.text((30, 840), context+" Pixel boundaries are approximate; point evidence uses continuous geometry.", fill="#555555", font=small)
    evidence = None
    if at is not None:
        evidence = probe_via(board, at, net)
        if x0 <= at[0] <= x1 and y0 <= at[1] <= y1:
            px, py = global_xy(at)
            draw.ellipse([px-7, py-7, px+7, py+7], outline="black", width=2)
            draw.line([(px-12, py), (px+12, py)], fill="black", width=2)
            draw.line([(px, py-12), (px, py+12)], fill="black", width=2)
        draw.text((sidebar_x, 691), f"Probe ({at[0]:.6f}, {at[1]:.6f})", fill="#222222", font=small)
        draw.text((sidebar_x, 715), f"Copper blockers: {len(evidence['blockers'])}", fill="#222222", font=normal)
        if evidence["nearest"]:
            draw.text((sidebar_x, 744), f"Smallest margin {evidence['nearest'][0]['margin_mm']:+.6f} mm", fill="#333333", font=small)
    output = Path(output)
    output.parent.mkdir(parents=True, exist_ok=True)
    image.save(output, format="PNG")
    return {"image": str(output), "board_id": board["id"], "net": net, "bounds_mm": bounds,
        "scope": SCOPE, "proposal_applied": proposal_applied,
        "visible_blockers": visible, "probe": evidence,
        "sampling": "Analytic circular-corner/cap exclusion primitives, supersampled2x. No search, optimizer, or automatic crop selection."}


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("board", type=Path)
    parser.add_argument("--net", required=True)
    parser.add_argument("--bounds", nargs=4, required=True, type=float, metavar=("X0", "Y0", "X1", "Y1"))
    parser.add_argument("--output", type=Path, required=True)
    parser.add_argument("--proposal", type=Path)
    parser.add_argument("--at", nargs=2, type=float, metavar=("X", "Y"))
    args = parser.parse_args()
    board = json.loads(args.board.read_text())
    if args.proposal:
        from generate import apply_proposal
        board = apply_proposal(board, json.loads(args.proposal.read_text()))
    print(json.dumps(render(board, args.net, args.bounds, args.output, args.at, bool(args.proposal)), indent=2))


if __name__ == "__main__":
    main()
