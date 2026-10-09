#!/usr/bin/env python3
# Copyright (C) 2026 Toit contributors.
"""Prepares the congestion model's boards and sample jobs.

    experiments/congestion/prepare.py build/congestion [--sets github pcbench training]
        [--probe-seconds 25] [--only NAME...]

Per board, under <root>/sources/<name>/:
  designer/  the designer's project (tracks are stripped by the exporter);
  free/      every footprint with a net stacked at the outline's centre and
             constraints {"move_all": true} (netless parts fixed): what our
             placer makes of the board from scratch;
  task/      for the harvested benchmark boards whose task moves parts: the
             task exactly as benchmarks/agent-tasks/run.py sets it up, so
             the placer seeds are the candidates the layout's race sees.
and a job (<root>/jobs/<name>.json) listing the variants to sample. The
split (<root>/split.json) holds out whole boards: every metric is reported
on those."""

import argparse
import hashlib
import importlib.util
import json
import re
import shutil
import subprocess
import sys
from pathlib import Path

ROOT = Path(__file__).resolve().parents[2]
spec = importlib.util.spec_from_file_location("agent_run", ROOT / "benchmarks/agent-tasks/run.py")
agent_run = importlib.util.module_from_spec(spec)
spec.loader.exec_module(agent_run)

GITHUB_TASKS = ROOT / "build/github-tasks-v5/tasks.json"
PCBENCH = ROOT / "benchmarks/pcbench/manifest.json"
PCBENCH_WORK = ROOT / "benchmarks/real/external/pcbench-work/newdrc"
TRAINING = ROOT / "benchmarks/real/external/training"

# Boards the layout really races placements on (v11: parts move and the
# race ran); half of them are held out, the other half trains.
RACE_BOARDS = [
    "MbFredys__PCB-Modular-Multi-Protocol-Hub__Hub",
    "OpenDrone-hw__OpenESC-20x20__4in1-mini",
    "OpenDrone-hw__OpenESC-30x30__4in1",
    "OpenDrone-hw__OpenFC-Lite__OpenFC",
    "OpenDrone-hw__OpenRX__OpenRX-panel-rev2",
    "anyshake__explorer__Explorer",
    "bismarx-v1__Sumec-MiniSumo__SUMEC_MK_IV",
    "bitshiftcrazy__d20_pcb__d20_pcb",
    "bitshiftcrazy__spell_tome__spell_tome_bottom",
    "briskspirit__Sisu_SSE-9__Sisu_SSE-9",
    "emertcakir__OpenAirScope__OpenAirScope",
    "headblockhead__slab-pcb__interchange-pcb-right",
    "ikajdan__katia__katia",
    "oro-os__link__link",
    "stonedDiscord__nonSNES__SNSP-CPU-01",
    "stonedDiscord__nonSNES__SNSP-CPU-1CHIP",
    "tomunderwood99__CharlieBoard__Blue_Line",
    "tubbytwins__bumwings-kbd__bumwings_v001R64_nano_sd",
    "vd-rd__sbc_allwinner_a13__module",
    "OpenDrone-hw__OpenESC-30x30__4in1-panel",
]
# Held-out race boards: the quick tier's movers and a spread of sizes.
HELD_OUT_RACE = {
    "OpenDrone-hw__OpenFC-Lite__OpenFC",
    "bitshiftcrazy__spell_tome__spell_tome_bottom",
    "tomunderwood99__CharlieBoard__Blue_Line",
    "anyshake__explorer__Explorer",
    "emertcakir__OpenAirScope__OpenAirScope",
    "stonedDiscord__nonSNES__SNSP-CPU-1CHIP",
    "OpenDrone-hw__OpenESC-20x20__4in1-mini",
    "MbFredys__PCB-Modular-Multi-Protocol-Hub__Hub",
    "bitshiftcrazy__d20_pcb__d20_pcb",
}


def held_out(name, fraction=0.15):
    """Whole boards kept out of training, by a hash of the name."""
    digest = int(hashlib.sha256(name.encode()).hexdigest()[:8], 16)
    return digest / 0xFFFFFFFF < fraction


def outline_centre(board_text):
    """The centre of the Edge.Cuts graphics' bounding box, or None."""
    xs, ys = [], []
    for match in re.finditer(r"\((gr_line|gr_rect|gr_arc|gr_poly|gr_circle)[\s\"]", board_text):
        block = board_text[match.start():agent_run.block_end(board_text, match.start())]
        if '"Edge.Cuts"' not in block:
            continue
        for point in re.finditer(r"\((?:start|end|mid|center|xy) ([-\d.]+) ([-\d.]+)\)", block):
            xs.append(float(point.group(1)))
            ys.append(float(point.group(2)))
    if not xs:
        return None
    return [(min(xs) + max(xs)) / 2, (min(ys) + max(ys)) / 2]


def copy_project(source_board, source_project, destination, board_id):
    destination.mkdir(parents=True, exist_ok=True)
    shutil.copy(source_board, destination / f"{board_id}.kicad_pcb")
    if source_project and source_project.exists():
        shutil.copy(source_project, destination / f"{board_id}.kicad_pro")


# A pad's net, in KiCad 10's syntax `(net "name")` and the older
# `(net 5 "name")` (benchmarks/agent-tasks/run.py's `unplace` knows only the
# first and takes every footprint of an older board for netless).
PAD_NET = r'\(pad [^\n]*[\s\S]*?\(net (?:\d+ )?"(?!unconnected-)[^"]+"\)'


def unplace(board, point, keep=()):
    """Stacks every footprint with a net that is not locked (or in `keep`) at
    `point`. Returns the references of the footprints without nets."""
    text = board.read_text()
    pieces, last, netless = [], 0, []
    for match in re.finditer(r"\(footprint[\s\"]", text):
        start = match.start()
        if start < last:
            continue
        end = agent_run.block_end(text, start)
        block = text[start:end]
        pieces.append(text[last:start])
        reference = re.search(r'\((?:property "Reference"|fp_text reference) "([^"]*)"', block)
        has_net = re.search(PAD_NET, block)
        locked = re.search(r"^\(footprint \"[^\"]*\"\s+(?:\(locked (?:yes)?\)|locked)", block)
        if reference and reference.group(1) and not has_net:
            netless.append(reference.group(1))
        if has_net and not locked and not (reference and reference.group(1) in keep):
            depth = 0
            for index, character in enumerate(block):
                if character == "(":
                    depth += 1
                    if depth == 2 and block.startswith("(at ", index):
                        close = block.index(")", index)
                        parts = block[index + 4:close].split()
                        angle = f" {parts[2]}" if len(parts) > 2 else ""
                        block = block[:index] + f"(at {point[0]} {point[1]}{angle})" + block[close + 1:]
                        break
                elif character == ")":
                    depth -= 1
        pieces.append(block)
        last = end
    pieces.append(text[last:])
    board.write_text("".join(pieces))
    return netless


def outline_footprints(board_text):
    """References of footprints that draw part of the board outline (they
    stay where they are, or the outline falls apart)."""
    references = []
    for match in re.finditer(r"\(footprint[\s\"]", board_text):
        block = board_text[match.start():agent_run.block_end(board_text, match.start())]
        if '"Edge.Cuts"' in block:
            reference = re.search(r'\((?:property "Reference"|fp_text reference) "([^"]*)"', block)
            if reference:
                references.append(reference.group(1))
    return references


def stacked(designer, destination, board_id, constraints, point=None, fixed=(), benchmark=False):
    """A copy of `designer` with every movable footprint stacked."""
    if destination.exists():
        shutil.rmtree(destination)
    shutil.copytree(designer, destination)
    board = destination / f"{board_id}.kicad_pcb"
    text = board.read_text()
    point = point or outline_centre(text) or [100.0, 100.0]
    edge = outline_footprints(text)
    fixed = list(fixed) + [reference for reference in edge if reference not in fixed]
    constraints = dict(constraints)
    if edge:
        escaped_edge = [re.sub(r"([*?\\])", r"\\\1", r) for r in edge]
        constraints["fixed"] = list(constraints.get("fixed", [])) + [r for r in escaped_edge if r not in constraints.get("fixed", [])]
    # The race boards' task source exactly as the benchmark stacks it.
    netless = agent_run.unplace(board, point, False, set(fixed)) if benchmark else unplace(board, point, set(fixed))
    escaped = [re.sub(r"([*?\\])", r"\\\1", r) for r in netless]
    constraints = dict(constraints)
    constraints["fixed"] = list(constraints.get("fixed", [])) + [r for r in escaped if r not in constraints.get("fixed", [])]
    (destination / "constraints.json").write_text(json.dumps(constraints))


def variants(kind, race, task_seeds=16, task_perturbations=True):
    """The placements sampled per board."""
    out = [{"name": "designer", "source": "designer"}]
    sizes = {"s": (5, 1, 1, 2.0), "m": (15, 3, 3, 5.0), "l": (40, 10, 10, 10.0)}
    for size, (moves, swaps, rotations, reach) in sizes.items():
        for seed in (1, 2):
            out.append({"name": f"designer-{size}{seed}", "source": "designer", "base": "designer",
                        "perturb": {"moves": moves, "swaps": swaps, "rotations": rotations, "max_mm": reach, "seed": seed}})
    for seed in (1, 2):
        out.append({"name": f"shuffle{seed}", "source": "designer", "shuffle": seed})
    seeds = (1, 2, 3, 4) if kind == "github" else (1, 2, 3)
    for seed in seeds:
        out.append({"name": f"free-seed{seed}", "source": "free", "placer_seed": seed})
    for size in ("s", "m"):
        moves, swaps, rotations, reach = sizes[size]
        out.append({"name": f"free-seed1-{size}1", "source": "free", "base": "free-seed1",
                    "perturb": {"moves": moves, "swaps": swaps, "rotations": rotations, "max_mm": reach, "seed": 1}})
    if race:
        for seed in range(1, task_seeds + 1):
            out.append({"name": f"task-seed{seed}", "source": "task", "placer_seed": seed})
        for seed in ((1, 2, 3) if task_perturbations else ()):
            out.append({"name": f"task-seed1-s{seed}", "source": "task", "base": "task-seed1",
                        "perturb": {"moves": 5, "swaps": 1, "rotations": 1, "max_mm": 2.0, "seed": seed}})
            out.append({"name": f"task-seed1-m{seed}", "source": "task", "base": "task-seed1",
                        "perturb": {"moves": 15, "swaps": 3, "rotations": 3, "max_mm": 5.0, "seed": seed}})
    return out


def github_boards():
    tasks = json.loads(GITHUB_TASKS.read_text())["tasks"]
    for task in tasks:
        yield {
            "name": task["name"], "kind": "github", "board_id": task["board_id"],
            "board": Path(task["directory"]) / f"{task['board_id']}.kicad_pcb",
            "project": Path(task["directory"]) / f"{task['board_id']}.kicad_pro",
            "task": task,
        }


def pcbench_boards():
    manifest = json.loads(PCBENCH.read_text())
    stem = manifest["board_file"]
    for board in manifest["boards"]:
        if "error" in board:
            continue
        directory = PCBENCH_WORK / board["name"]
        yield {
            "name": "pcbench__" + board["name"], "kind": "pcbench", "board_id": "board",
            "board": directory / f"{stem}_unrouted.kicad_pcb", "project": directory / f"{stem}.kicad_pro",
            "layers": board["copper_layers"],
        }


def training_boards():
    manifest = ROOT / "benchmarks/github/training-manifest.json"
    if not manifest.exists():
        return
    for board in json.loads(manifest.read_text())["boards"]:
        directory = TRAINING / board["name"]
        yield {
            "name": "training__" + board["name"], "kind": "training", "board_id": board["board_id"],
            "board": directory / f"{board['board_id']}.kicad_pcb", "project": directory / f"{board['board_id']}.kicad_pro",
        }


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("root", type=Path)
    parser.add_argument("--sets", nargs="*", default=["github", "pcbench"])
    parser.add_argument("--probe-seconds", type=float, default=25.0)
    parser.add_argument("--placement-seconds", type=float, default=60.0)
    parser.add_argument("--only", nargs="*")
    parser.add_argument("--task-only", action="store_true",
                        help="rebuild only the task sources and the jobs (designer and free kept)")
    parser.add_argument("--binary", type=Path, default=ROOT / "build/bin/pcb-maker-x400")
    arguments = parser.parse_args()
    root = arguments.root
    (root / "jobs").mkdir(parents=True, exist_ok=True)
    split_path = root / "split.json"
    split = json.loads(split_path.read_text()) if split_path.exists() else {}
    sources = {"github": github_boards, "pcbench": pcbench_boards, "training": training_boards}
    prepared = 0
    for set_name in arguments.sets:
        for board in sources[set_name]():
            name = board["name"]
            if arguments.only and name not in arguments.only:
                continue
            if not board["board"].exists():
                print(f"skipping {name}: {board['board']} missing", file=sys.stderr)
                continue
            base = root / "sources" / name
            board_id = board["board_id"]
            designer = base / "designer"
            if arguments.task_only and not designer.exists():
                continue
            if not arguments.task_only and designer.exists():
                shutil.rmtree(designer)
            if not arguments.task_only:
                copy_project(board["board"], board["project"], designer, board_id)
            if board["kind"] != "pcbench" and not arguments.task_only:
                # As the benchmark does: tracks, vias and small zones out.
                path = designer / f"{board_id}.kicad_pcb"
                stripped = subprocess.run([str(arguments.binary), "strip-kicad-tracks", str(path), str(path)],
                                          capture_output=True, text=True)
                if stripped.returncode != 0:
                    print(f"skipping {name}: {stripped.stderr.strip()[-200:]}", file=sys.stderr)
                    shutil.rmtree(base, ignore_errors=True)
                    continue
            if not arguments.task_only:
                stacked(designer, base / "free", board_id, {"version": 1, "move_all": True})
            # Since run.py reads pre-KiCad-10 nets (baa5817) every harvested
            # task places parts: every github board races.
            race = board["kind"] == "github"
            if race:
                task = board["task"]
                constraints = json.loads((GITHUB_TASKS.parent / task["constraints"]).read_text())
                stacked(designer, base / "task", board_id, constraints, task.get("stack_at"),
                        constraints.get("fixed", []), benchmark=True)
                (base / "task" / "layout.json").write_text(json.dumps(
                    {"placer": {"constraints": "constraints.json", **task.get("placer", {})}, **task.get("layout", {})}))
            if name not in split:
                split[name] = "test" if (name in HELD_OUT_RACE or (name not in RACE_BOARDS and held_out(name))) else "train"
            job = {
                "name": name, "kind": board["kind"], "board_id": board_id, "race": race,
                "probe_seconds": arguments.probe_seconds, "placement_seconds": arguments.placement_seconds,
                "variants": variants(board["kind"], race,
                                     16 if (name in RACE_BOARDS or split[name] == "test") else 8,
                                     name in RACE_BOARDS),
            }
            (root / "jobs" / f"{name}.json").write_text(json.dumps(job, indent=1))
            prepared += 1
    split_path.write_text(json.dumps(split, indent=1, sort_keys=True))
    held = sum(1 for value in split.values() if value == "test")
    print(f"{prepared} boards prepared; split: {len(split) - held} train, {held} held out")


if __name__ == "__main__":
    main()
