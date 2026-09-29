#!/usr/bin/env python3
# Copyright (C) 2026 Toit contributors.
"""Runs the agent-style layout tasks of tasks.json.

    benchmarks/agent-tasks/run.py <output> [--only NAME...] [--binary PATH]

Each task copies a real board, strips its tracks, optionally stacks every
movable footprint at one point and removes the outline (a fresh netlist
import), and lays it out with `layout-kicad-board` from the task's
constraints. A task passes when every constraint holds (except the ones the
task lists as geometrically impossible), every connection is routed, and
native KiCad reports no unconnected item and no copper error."""

import argparse
import json
import re
import shutil
import subprocess
import time
from pathlib import Path

HERE = Path(__file__).resolve().parent
ROOT = HERE.parents[1]
COSMETIC = re.compile(r"^(silk_|text_|lib_footprint|footprint_type_mismatch|missing_courtyard|isolated_copper|nonmirrored_text)")


def block_end(text, start):
    depth, quoted = 0, False
    for index in range(start, len(text)):
        character = text[index]
        if character == '"' and text[index - 1] != "\\":
            quoted = not quoted
        elif not quoted:
            if character == "(":
                depth += 1
            elif character == ")":
                depth -= 1
                if depth == 0:
                    return index + 1
    raise ValueError("unbalanced expression")


def unplace(board, point, remove_outline, keep=()):
    """Stacks every footprint with a net that is not locked (or in `keep`) at
    `point`; drops the Edge.Cuts graphics if asked. Returns the references
    of the footprints without nets, which stay where the designer put
    them."""
    text = board.read_text()
    pieces, last, netless = [], 0, []
    pattern = re.compile(r"\((footprint|gr_line|gr_rect|gr_arc|gr_circle|gr_poly)[\s\"]")
    for match in pattern.finditer(text):
        start = match.start()
        if start < last:
            continue
        end = block_end(text, start)
        block = text[start:end]
        pieces.append(text[last:start])
        if match.group(1) == "footprint":
            reference = re.search(r'\(property "Reference" "([^"]*)"', block)
            movable = re.search(r'\(pad [^\n]*[\s\S]*?\(net "(?!unconnected-)[^"]+"\)', block) and not re.search(
                r"^\(footprint \"[^\"]*\"\s+(?:\(locked (?:yes)?\)|locked)", block) and not (
                reference and reference.group(1) in keep)
            if reference and reference.group(1) and not re.search(r"[*?\[]", reference.group(1)) and not re.search(
                    r'\(pad [^\n]*[\s\S]*?\(net "(?!unconnected-)[^"]+"\)', block):
                netless.append(reference.group(1))
            if movable:
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
                            break
                    elif character == ")":
                        depth -= 1
            pieces.append(block)
        elif remove_outline and '"Edge.Cuts"' in block:
            pass
        else:
            pieces.append(block)
        last = end
    pieces.append(text[last:])
    board.write_text("".join(pieces))
    return netless


def native(result_directory):
    report = result_directory / "drc.json"
    if not report.exists():
        return None
    drc = json.loads(report.read_text())
    errors = {}
    for violation in drc.get("violations", []):
        if violation.get("severity") == "error" and not COSMETIC.match(violation["type"]):
            errors[violation["type"]] = errors.get(violation["type"], 0) + 1
    return {"errors": errors, "unconnected": len(drc.get("unconnected_items", []))}


def run_task(task, arguments):
    name = task["name"]
    directory = Path(task["directory"])
    if not directory.is_absolute():
        directory = ROOT / directory
    work = arguments.output / name
    if work.exists():
        shutil.rmtree(work)
    source = work / "source"
    shutil.copytree(directory, source, ignore=shutil.ignore_patterns(
        ".history", "*-backups", "Gerbers", "packages3D", "*.pdf", "*.zip", "fp-info-cache"))
    board = source / f"{task['board_id']}.kicad_pcb"
    subprocess.run([str(arguments.binary), "strip-kicad-tracks", str(board), str(board)],
                   capture_output=True, check=True)
    constraints = json.loads((arguments.tasks.parent / task["constraints"]).read_text())
    if task.get("unplace"):
        outline = constraints.get("outline", {})
        point = task.get("stack_at") or [outline.get("x", 0) + outline.get("width", 40) / 2,
                                          outline.get("y", 0) + outline.get("height", 40) / 2]
        netless = unplace(board, point, task.get("remove_outline", False), set(constraints.get("fixed", [])))
        # move_all would move them too.
        constraints["fixed"] = constraints.get("fixed", []) + [r for r in netless if r not in constraints.get("fixed", [])]
    (source / "constraints.json").write_text(json.dumps(constraints))
    layout = {"placer": {"constraints": "constraints.json", **task.get("placer", {})}, **task.get("layout", {})}
    (work / "layout.json").write_text(json.dumps(layout))
    router = task.get("router") or {}
    router_argument = "auto"
    if router:
        (work / "router.json").write_text(json.dumps(router))
        router_argument = str(work / "router.json")
    started = time.monotonic()
    try:
        process = subprocess.run([str(arguments.binary), "layout-kicad-board", str(source), task["board_id"],
                                  str(work / "layout"), router_argument, str(work / "layout.json")],
                                 stdout=subprocess.PIPE, stderr=subprocess.STDOUT, text=True, timeout=arguments.timeout)
    except subprocess.TimeoutExpired as expired:
        output = expired.stdout.decode(errors="replace") if isinstance(expired.stdout, bytes) else (expired.stdout or "")
        (work / "layout.log").write_text(output)
        return {"name": name, "seconds": round(time.monotonic() - started, 1),
                "error": f"timed out after {arguments.timeout} s", "pass": False}
    (work / "layout.log").write_text(process.stdout)
    row = {"name": name, "seconds": round(time.monotonic() - started, 1)}
    report = work / "layout/board-layout.json"
    if not report.exists():
        row["error"] = process.stdout.strip().splitlines()[-1][:300] if process.stdout.strip() else "no output"
        row["pass"] = False
        return row
    result = json.loads(report.read_text())
    routed = result["routed"]
    expected = {tuple(miss) for miss in task.get("expected_misses", [])}
    missed = [(c["kind"], c["part"], c.get("other")) for c in result["constraints"] if not c["satisfied"]]
    unexpected = [miss for miss in missed if miss not in expected]
    copper = native(work / "layout/result")
    row.update(
        routed=f"{routed['routed_connections']}/{routed['routable_connections']}",
        vias=routed["vias"], length_mm=round(routed["length_mm"], 1),
        constraints=len(result["constraints"]), missed=missed, unexpected_misses=unexpected,
        native=copper,
    )
    row["pass"] = bool(routed["routed_connections"] == routed["routable_connections"] and not unexpected
                       and copper is not None and copper["unconnected"] == 0 and not copper["errors"])
    return row


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("output", type=Path)
    parser.add_argument("--only", nargs="*")
    parser.add_argument("--binary", type=Path, default=ROOT / "target/release/pcb-maker")
    parser.add_argument("--timeout", type=int, default=1800)
    parser.add_argument("--tasks", type=Path, default=HERE / "tasks.json",
                        help="task list; constraint files are relative to it")
    parser.add_argument("--jobs", type=int, default=1)
    arguments = parser.parse_args()
    arguments.output.mkdir(parents=True, exist_ok=True)
    tasks = []
    for task in json.loads(arguments.tasks.read_text())["tasks"]:
        if arguments.only and task["name"] not in arguments.only:
            continue
        directory = Path(task["directory"]) if Path(task["directory"]).is_absolute() else ROOT / task["directory"]
        if not directory.exists():
            print(f"skipping {task['name']}: {directory} not found")
            continue
        tasks.append(task)
    rows = []
    from concurrent.futures import ThreadPoolExecutor
    with ThreadPoolExecutor(arguments.jobs) as pool:
        for row in pool.map(lambda task: run_task(task, arguments), tasks):
            rows.append(row)
            print(json.dumps(row), flush=True)
    (arguments.output / "results.json").write_text(json.dumps(rows, indent=1))
    passed = sum(1 for row in rows if row["pass"])
    print(f"{passed}/{len(rows)} tasks pass")


if __name__ == "__main__":
    main()
