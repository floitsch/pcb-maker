#!/usr/bin/env python3
"""Animated GIFs of a placement run and of a routing run.

    animate.py placement <placement.html> <out.gif> [--frames N] [--width W]
    animate.py routing <frame-dir> <final.kicad_pcb> <out.gif> [--frames N] [--width W]

`placement.html` is what `place-kicad-board` / `layout-kicad-board` write
next to the placed board; the frames are drawn directly from its data.
`<frame-dir>` is the router's `frame_directory` (one `attempt-NN`
subdirectory per ladder rung, `frame-NNNN.kicad_pcb` inside); the frames are
rendered through `kicad-cli pcb export svg`, so KiCad must be installed.
Needs Pillow, rsvg-convert and ffmpeg.
"""

import argparse
import json
import math
import re
import shutil
import subprocess
import sys
import tempfile
from pathlib import Path

from PIL import Image, ImageDraw, ImageFont

BACKGROUND = (16, 21, 28)
CAPTION = (190, 200, 214)


def font(size):
    for candidate in [
        "/usr/share/fonts/TTF/DejaVuSans.ttf",
        "/usr/share/fonts/dejavu/DejaVuSans.ttf",
        "/usr/share/fonts/truetype/dejavu/DejaVuSans.ttf",
    ]:
        if Path(candidate).exists():
            return ImageFont.truetype(candidate, size)
    return ImageFont.load_default()


def sample(items, count):
    """`count` items spread evenly, always including the first and last."""
    if len(items) <= count:
        return list(items)
    picks = sorted({round(index * (len(items) - 1) / (count - 1)) for index in range(count)})
    return [items[index] for index in picks]


def caption(image, text):
    draw = ImageDraw.Draw(image)
    draw.text((12, image.height - 26), text, fill=CAPTION, font=font(15))


def write_gif(frames, output, fps, hold_last=2.0):
    """Palette-optimised GIF through ffmpeg; the last frame stays a while."""
    with tempfile.TemporaryDirectory() as directory:
        for index, frame in enumerate(frames):
            frame.save(Path(directory) / f"{index:05}.png")
        # Repeating the final frame keeps the finished board on screen.
        extra = int(hold_last * fps)
        for index in range(len(frames), len(frames) + extra):
            frames[-1].save(Path(directory) / f"{index:05}.png")
        pattern = str(Path(directory) / "%05d.png")
        filters = (
            "split[a][b];[a]palettegen=stats_mode=diff[p];"
            "[b][p]paletteuse=dither=bayer:bayer_scale=5:diff_mode=rectangle"
        )
        subprocess.run(
            ["ffmpeg", "-y", "-loglevel", "error", "-framerate", str(fps), "-i", pattern,
             "-vf", filters, "-loop", "0", str(output)],
            check=True,
        )


# --- placement ---------------------------------------------------------------

def placement_data(html):
    match = re.search(r"const D = (\{.*?\});\n", html, re.S)
    if not match:
        raise SystemExit("no placement data found in the HTML")
    return json.loads(match.group(1))


def rotate(point, angle):
    radians = -math.radians(angle)
    cosine, sine = math.cos(radians), math.sin(radians)
    return point[0] * cosine - point[1] * sine, point[0] * sine + point[1] * cosine


def draw_placement_frame(data, frame, width):
    x0, y0, x1, y1 = data["bounds"]
    pad = 16
    scale = (width - 2 * pad) / (x1 - x0)
    height = int((y1 - y0) * scale + 2 * pad + 30)
    image = Image.new("RGB", (width, height), BACKGROUND)
    draw = ImageDraw.Draw(image, "RGBA")
    X = lambda x: (x - x0) * scale + pad
    Y = lambda y: (y - y0) * scale + pad

    outline = [(X(p[0]), Y(p[1])) for p in data["outline"]]
    if outline:
        draw.polygon(outline, fill=(13, 26, 20, 255), outline=(136, 170, 170, 255))

    poses = frame["p"]
    nets = {}
    for component, (px, py, angle) in zip(data["components"], poses):
        for pin in component["pins"]:
            ox, oy = rotate(pin, angle)
            nets.setdefault(pin[2], []).append((px + ox, py + oy))
    for pins in nets.values():
        if len(pins) < 2 or len(pins) > 12:
            continue
        cx = sum(p[0] for p in pins) / len(pins)
        cy = sum(p[1] for p in pins) / len(pins)
        for p in pins:
            draw.line([(X(cx), Y(cy)), (X(p[0]), Y(p[1]))], fill=(255, 200, 80, 90), width=1)

    for component, (px, py, angle) in zip(data["components"], poses):
        fixed = component["fixed"]
        fill = (150, 150, 170, 90) if fixed else (80, 160, 255, 80)
        edge = (153, 153, 170, 255) if fixed else (85, 170, 255, 255)
        center, size = component["center"], component["size"]
        if component["round"]:
            mx, my = rotate(center, angle)
            radius = size[0] / 2 * scale
            draw.ellipse(
                [X(px + mx) - radius, Y(py + my) - radius, X(px + mx) + radius, Y(py + my) + radius],
                fill=fill, outline=edge,
            )
        else:
            corners = []
            for sx, sy in [(-1, -1), (1, -1), (1, 1), (-1, 1)]:
                ox, oy = rotate((center[0] + sx * size[0] / 2, center[1] + sy * size[1] / 2), angle)
                corners.append((X(px + ox), Y(py + oy)))
            draw.polygon(corners, fill=fill, outline=edge)
        for pin in component["pins"]:
            ox, oy = rotate(pin, angle)
            draw.rectangle([X(px + ox) - 1, Y(py + oy) - 1, X(px + ox) + 1, Y(py + oy) + 1], fill=(232, 192, 112, 255))
    caption(image, f"placement iteration {frame['i']}   overflow {frame['o']:.3f}   wirelength {frame['w']:.0f} mm")
    return image


def animate_placement(arguments):
    data = placement_data(Path(arguments.placement_html).read_text())
    frames = sample(data["frames"], arguments.frames)
    images = [draw_placement_frame(data, frame, arguments.width) for frame in frames]
    write_gif(images, arguments.output, arguments.fps)
    print(f"wrote {arguments.output}: {len(images)} frames from {len(data['frames'])}")


# --- routing -----------------------------------------------------------------

def render_board(board, width, layers):
    """A KiCad SVG export of the board's copper, rasterised to `width` px."""
    with tempfile.TemporaryDirectory() as directory:
        svg = Path(directory) / "board.svg"
        subprocess.run(
            ["kicad-cli", "pcb", "export", "svg", "--mode-single", "--layers", layers,
             "--page-size-mode", "2", "--exclude-drawing-sheet", "--output", str(svg), str(board)],
            check=True, capture_output=True,
        )
        png = Path(directory) / "board.png"
        subprocess.run(
            ["rsvg-convert", "-w", str(width), "-b", "#%02x%02x%02x" % BACKGROUND, "-o", str(png), str(svg)],
            check=True,
        )
        return Image.open(png).convert("RGB")


def animate_routing(arguments):
    root = Path(arguments.frame_dir)
    boards = []
    for attempt in sorted(root.glob("attempt-*")):
        boards.extend(sorted(attempt.glob("frame-*.kicad_pcb")))
    if not boards:
        raise SystemExit(f"no frames under {root}")
    picked = sample(boards, arguments.frames - 1)
    images = []
    for board in picked:
        image = render_board(board, arguments.width, arguments.layers)
        padded = Image.new("RGB", (image.width, image.height + 30), BACKGROUND)
        padded.paste(image, (0, 0))
        iteration = int(board.stem.split("-")[1])
        caption(padded, f"routing iteration {iteration}")
        images.append(padded)
    final = render_board(Path(arguments.final_board), arguments.width, arguments.layers)
    padded = Image.new("RGB", (final.width, final.height + 30), BACKGROUND)
    padded.paste(final, (0, 0))
    caption(padded, "routed, cleaned up and verified by KiCad")
    images.append(padded)
    size = images[-1].size
    images = [image if image.size == size else image.resize(size) for image in images]
    write_gif(images, arguments.output, arguments.fps)
    print(f"wrote {arguments.output}: {len(images)} frames from {len(boards)}")


def main():
    parser = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    commands = parser.add_subparsers(dest="command", required=True)
    placement = commands.add_parser("placement")
    placement.add_argument("placement_html")
    placement.add_argument("output")
    routing = commands.add_parser("routing")
    routing.add_argument("frame_dir")
    routing.add_argument("final_board")
    routing.add_argument("output")
    routing.add_argument("--layers", default="F.Cu,B.Cu,Edge.Cuts")
    for command in (placement, routing):
        command.add_argument("--frames", type=int, default=60)
        command.add_argument("--width", type=int, default=720)
        command.add_argument("--fps", type=int, default=12)
    arguments = parser.parse_args()
    for tool in ("ffmpeg", "rsvg-convert") + (("kicad-cli",) if arguments.command == "routing" else ()):
        if shutil.which(tool) is None:
            raise SystemExit(f"{tool} is not installed")
    if arguments.command == "placement":
        animate_placement(arguments)
    else:
        animate_routing(arguments)


if __name__ == "__main__":
    sys.exit(main())
