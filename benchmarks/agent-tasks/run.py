#!/usr/bin/env python3
# Copyright (C) 2026 Toit contributors.
"""Runs the agent-style layout tasks of tasks.json.

    benchmarks/agent-tasks/run.py <output> [--only NAME...] [--binary PATH]
    benchmarks/agent-tasks/run.py <output> --tier quick --jobs 12 --binary PATH
    benchmarks/agent-tasks/run.py <output> --tier short --jobs 6 --binary PATH
    benchmarks/agent-tasks/run.py <output> --tier quick --route-from build/quick30-x782 --binary PATH
    benchmarks/agent-tasks/run.py <output> --tier quick --only A B --splice-from build/quick30-x782.log

Each task copies a real board, strips its tracks, optionally stacks every
movable footprint at one point and removes the outline (a fresh netlist
import), and lays it out with `layout-kicad-board` from the task's
constraints. A task passes when every constraint holds (except the ones the
task lists as geometrically impossible), every connection is routed, and
native KiCad reports no unconnected item and no copper error.

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
marked `spliced`. Every row has `mode` (layout or route); the run ends with
the pass line and a line of continuous objectives (README.md, "How to
measure"), also written to summary.json."""

import argparse
import json
import os
import re
import shutil
import subprocess
import sys
import threading
import time
from pathlib import Path

HERE = Path(__file__).resolve().parent
sys.path.insert(0, str(HERE))
from compare import by_name, objectives, objectives_line, rows_of  # noqa: E402
ROOT = HERE.parents[1]
# Benchmark tiers: subsets of a task list with their reasons (README.md).
TIERS = {"quick": HERE / "quick.json", "short": HERE / "short.json"}
# The harness's timeout less this is scaled with the budget: KiCad's checks
# and the file work do not shrink with it.
FIXED_TIMEOUT_SECONDS = 300
COSMETIC = re.compile(r"^(silk_|text_|lib_footprint|footprint_type_mismatch|missing_courtyard|isolated_copper|nonmirrored_text)")


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
            reference = re.search(r'\((?:property "Reference"|fp_text reference) "([^"]*)"', block)
            # A pad's net is `(net "X")` in KiCad 10 files and `(net 5 "X")`
            # before; only the first form was read, so every footprint of an
            # older file counted as netless and stayed fixed (89 of the 109
            # harvested tasks never placed anything until 2026-10-09).
            connected = r'\(pad [^\n]*[\s\S]*?\(net (?:\d+ )?"(?!unconnected-)[^"]+"\)'
            movable = re.search(connected, block) and not re.search(
                r"^\(footprint \"[^\"]*\"\s+(?:\(locked (?:yes)?\)|locked)", block) and not (
                reference and reference.group(1) in keep)
            if reference and reference.group(1) and not re.search(connected, block):
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
                            block = move_zones(block, point[0] - float(parts[0]), point[1] - float(parts[1]))
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


def native(result_directory, allowed=None):
    """KiCad's copper errors and unconnected items on the result; `allowed`
    errors per type (the designer's own board's) are not counted."""
    report = result_directory / "drc.json"
    if not report.exists():
        return None
    drc = json.loads(report.read_text())
    errors = {}
    for violation in drc.get("violations", []):
        if violation.get("severity") == "error" and not violation.get("excluded") and not COSMETIC.match(violation["type"]):
            errors[violation["type"]] = errors.get(violation["type"], 0) + 1
    for kind, count in (allowed or {}).items():
        if kind in errors:
            errors[kind] -= count
            if errors[kind] <= 0:
                del errors[kind]
    return {"errors": errors, "unconnected": len(drc.get("unconnected_items", []))}


def designer_findings(directory, board_id, work):
    """Copper errors per type on the designer's own routed board, which a
    task from a real board need not do better than (harvested boards come
    with their makers' DRC findings: fiducials near holes, rules tightened
    later)."""
    reference = work / "reference"
    shutil.copytree(directory, reference, ignore=shutil.ignore_patterns(
        ".history", "*-backups", "Gerbers", "packages3D", "*.pdf", "*.zip", "fp-info-cache"))
    try:
        subprocess.run(["kicad-cli", "pcb", "drc", "--refill-zones", "--severity-error", "--format", "json",
                        "-o", "drc.json", f"{board_id}.kicad_pcb"], cwd=reference,
                       stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL, timeout=600)
    except (OSError, subprocess.TimeoutExpired):
        return {}
    report = reference / "drc.json"
    if not report.exists():
        return {}
    counts = {}
    for violation in json.loads(report.read_text()).get("violations", []):
        if violation.get("severity") == "error" and not violation.get("excluded") and not COSMETIC.match(violation["type"]):
            counts[violation["type"]] = counts.get(violation["type"], 0) + 1
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


def run_measured(command, timeout=None, env=None, on_start=None):
    """Runs `command` with its output captured. Returns (exit code, or
    "timeout" after killing it; output; peak resident set in MB; the
    command's own peak in MB; CPU seconds of the process tree). The first peak is wait4's ru_maxrss: the
    largest of the process and the children it waited for (kicad-cli's DRC
    alone takes 2.2 GB, on any board), for this process alone, so that jobs
    side by side do not see each other's peaks. The second is the
    process's own VmHWM, sampled twice a second: the router's memory.
    `on_start` gets the pid."""
    process = subprocess.Popen(command, stdout=subprocess.PIPE, stderr=subprocess.STDOUT, env=env)
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
            process.kill()
            _, status, usage = os.wait4(process.pid, 0)
            code = "timeout"
            break
        time.sleep(0.5)
    # Reaped here: Popen must not wait for it again.
    process.returncode = code if isinstance(code, int) else -9
    # A killed board's kicad-cli may hold the pipe a little longer.
    reader.join(60)
    output = (chunks[0] if chunks else b"").decode(errors="replace")
    return code, output, round(usage.ru_maxrss / 1024), round(own), round(usage.ru_utime + usage.ru_stime, 1)


def meminfo_gb(field="MemAvailable:"):
    try:
        with open("/proc/meminfo") as meminfo:
            for line in meminfo:
                if line.startswith(field):
                    return int(line.split()[1]) / (1024 * 1024)
    except OSError:
        pass
    return None


def resident_mb(pid):
    try:
        with open(f"/proc/{pid}/status") as status:
            for line in status:
                if line.startswith("VmRSS:"):
                    return int(line.split()[1]) / 1024
    except OSError:
        pass
    return 0.0


class MemoryGate:
    """Admits a board when the memory available, less what the boards
    already running are still expected to grow by, leaves `minimum_gb`
    after this board's own expected peak. Many jobs started at once all see
    the memory free that none of them has taken yet (jobs grow for minutes:
    the router's maps, then the ladder's seeds side by side); the
    reservation keeps them from overcommitting together."""

    def __init__(self, minimum_gb):
        self.minimum_gb = minimum_gb
        self.lock = threading.Lock()
        # token -> [expected peak in MB, pid or None]
        self.running = {}
        self.next_token = 0

    def admit(self, what, expected_mb):
        if self.minimum_gb <= 0:
            return None
        waited = 0
        while True:
            with self.lock:
                available = meminfo_gb()
                if available is None:
                    return None
                growth = sum(max(0.0, expected - (resident_mb(pid) if pid else 0.0))
                             for expected, pid in self.running.values()) / 1024
                if available - growth - expected_mb / 1024 >= self.minimum_gb or not self.running:
                    token = self.next_token
                    self.next_token += 1
                    self.running[token] = [expected_mb, None]
                    if waited:
                        print(f"{what}: started after waiting {waited} s ({available:.0f} GB available, "
                              f"{growth:.0f} GB reserved)", file=sys.stderr)
                    return token
            if waited == 0:
                print(f"{what}: waiting for memory ({available:.1f} GB available, {growth:.1f} GB reserved, "
                      f"{expected_mb / 1024:.1f} GB expected, {self.minimum_gb} GB to keep)", file=sys.stderr)
            time.sleep(15)
            waited += 15

    def started(self, token, pid):
        if token is not None:
            with self.lock:
                self.running[token][1] = pid

    def done(self, token):
        if token is not None:
            with self.lock:
                self.running.pop(token, None)


def run_task(task, arguments):
    # Admitted before the designer's DRC: kicad-cli takes 2.2 GB on any
    # board, and twelve jobs start with it at once.
    token = arguments.gate.admit(task["name"], task.get("peak_rss_mb", arguments.expected_mb))
    try:
        return run_admitted(task, arguments, token)
    finally:
        arguments.gate.done(token)


# The layout's budgets (KiCadBoardLayoutConfig's defaults), scaled by
# --budget-scale; the router's ladder budget in route mode.
LAYOUT_BUDGETS = {"total_seconds": 1500.0, "move_seconds": 600.0, "polish_seconds": 60.0}
ROUTE_BUDGET = ("ladder_budget_seconds", 1200.0)


def scaled_timeout(arguments):
    if arguments.budget_scale == 1:
        return arguments.timeout
    fixed = min(FIXED_TIMEOUT_SECONDS, arguments.timeout)
    return round(fixed + (arguments.timeout - fixed) * arguments.budget_scale)


def run_admitted(task, arguments, token):
    if arguments.route_from:
        return route_admitted(task, arguments, token)
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
    allowed = designer_findings(directory, task["board_id"], work) if task.get("designer_budget") else {}
    subprocess.run([str(arguments.binary), "strip-kicad-tracks", str(board), str(board)],
                   capture_output=True, check=True)
    constraints = json.loads((arguments.tasks.parent / task["constraints"]).read_text())
    if task.get("unplace"):
        outline = constraints.get("outline", {})
        point = task.get("stack_at") or [outline.get("x", 0) + outline.get("width", 40) / 2,
                                          outline.get("y", 0) + outline.get("height", 40) / 2]
        netless = unplace(board, point, task.get("remove_outline", False), set(constraints.get("fixed", [])))
        # move_all would move them too.
        # Fixed parts are glob patterns: references like KiCad's REF** are
        # escaped to match only themselves.
        escaped = [re.sub(r"([*?\\])", r"\\\1", r) for r in netless]
        constraints["fixed"] = constraints.get("fixed", []) + [r for r in escaped if r not in constraints.get("fixed", [])]
    (source / "constraints.json").write_text(json.dumps(constraints))
    layout = {"placer": {"constraints": "constraints.json", **task.get("placer", {}), **arguments.placer},
              **task.get("layout", {})}
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
    timeout = scaled_timeout(arguments)
    code, output, peak_mb, own_mb, cpu_seconds = run_measured(command, timeout, env,
                                                              lambda pid: arguments.gate.started(token, pid))
    (work / "layout.log").write_text(output)
    # CPU seconds are the speed measure: other work on the machine (sweeps,
    # Florian's other projects) stretches the wall clock, not the CPU time.
    # The load averages say how loaded the machine was for this row.
    row = {"name": name, "seconds": round(time.monotonic() - started, 1), "cpu_seconds": cpu_seconds,
           "load": [round(load_before, 1), round(os.getloadavg()[0], 1)],
           "peak_rss_mb": peak_mb, "layout_rss_mb": own_mb, "mode": "layout"}
    if arguments.budget_scale != 1:
        row["budget_scale"] = arguments.budget_scale
    if code == "timeout":
        # The harness's limit cut the run: what it would have done depends
        # on the machine's load.
        row.update(error=f"timed out after {timeout} s", cut=["timeout"], **{"pass": False})
        return row
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
    copper = native(work / "layout/result", allowed)
    if allowed:
        row["designer_findings"] = allowed
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
    if result.get("first_unconnected_terminals") is not None:
        row["first_unconnected_terminals"] = result["first_unconnected_terminals"]
    # A board passes on KiCad's connectivity, not on the router's own count
    # (Florian, 2026-10-07): KiCad joins a pad through a pour's fill that
    # the router, routing pours as tracks, does not credit (MokyaLora's
    # one open pad, 261/262 with KiCad at 0 unconnected).
    row["pass"] = bool(not unexpected and copper is not None and copper["unconnected"] == 0 and not copper["errors"])
    return row


def route_admitted(task, arguments, token):
    """Route mode (`--route-from`): routes the task's placement from an
    earlier sweep (`<sweep>/<task>/layout/placed`) with `route-kicad-board`,
    for router experiments without the placement race and the legaliser.
    The row and the pass rule are the layout's; constraints are not checked
    (the placement is given)."""
    name = task["name"]
    directory = Path(task["directory"])
    if not directory.is_absolute():
        directory = ROOT / directory
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
    allowed = designer_findings(directory, task["board_id"], work) if task.get("designer_budget") else {}
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
    timeout = scaled_timeout(arguments)
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
    row["exit"] = code
    report = work / "route/board-router.json"
    if not report.exists():
        verdict_file = work / "route/verdict.json"
        verdict = json.loads(verdict_file.read_text()) if verdict_file.exists() else None
        if verdict:
            row["verdict"] = verdict.get("status")
            row["error"] = verdict.get("reason", "")[:300]
        else:
            row["error"] = output.strip().splitlines()[-1][:300] if output.strip() else "no output"
        row["pass"] = False
        return row
    result = json.loads(report.read_text())
    if result.get("verdict"):
        row["verdict"] = result["verdict"].get("status")
    copper = native(work / "route", allowed)
    if allowed:
        row["designer_findings"] = allowed
    if result.get("cut"):
        row["cut"] = result["cut"]
    row.update(
        routed=f"{result['routed_connections']}/{result['routable_connections']}",
        vias=result["vias"], length_mm=round(result["length_mm"], 1),
        constraints=0, missed=[], unexpected_misses=[], native=copper,
    )
    if result.get("work_seconds") is not None:
        row["work_seconds"] = round(result["work_seconds"], 1)
    row["pass"] = bool(copper is not None and copper["unconnected"] == 0 and not copper["errors"])
    return row


def spliced_rows(path, names):
    """The rows of `names` in an earlier log (compare.rows_of: a log or an
    output directory), the last of each board, marked spliced."""
    rows = by_name(rows_of(path))
    return {name: {**rows[name], "spliced": True} for name in names if name in rows}


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("output", type=Path)
    parser.add_argument("--only", nargs="*")
    parser.add_argument("--binary", type=Path, default=ROOT / "target/release/pcb-maker")
    parser.add_argument("--timeout", type=int, default=1800)
    parser.add_argument("--tasks", type=Path, default=None,
                        help="task list; constraint files are relative to it (default: tasks.json here, "
                             "or the tier's own list)")
    parser.add_argument("--tier", choices=sorted(TIERS),
                        help="only the boards of a tier (quick: quick.json, many small boards side by side)")
    parser.add_argument("--placer", type=json.loads, default={},
                        help="placer settings for every board, as JSON (e.g. '{\"halo_scale\": 0.75}')")
    parser.add_argument("--jobs", type=int, default=1)
    parser.add_argument("--threads", type=int, default=None,
                        help="search threads per board (RAYON_NUM_THREADS; default with several jobs: "
                             "the cores shared among them, at most 4; with one job: all)")
    parser.add_argument("--min-free-gb", type=float, default=12.0,
                        help="start a board only when this much memory stays available after the "
                             "expected peaks of it and the boards running (0: always)")
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
    arguments = parser.parse_args()
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
    if arguments.route_from is not None and not arguments.route_from.is_dir():
        parser.error(f"--route-from: {arguments.route_from} is not a directory")
    if arguments.splice_from is not None and not arguments.splice_from.exists():
        parser.error(f"--splice-from: {arguments.splice_from} not found")
    if arguments.threads is None and arguments.jobs > 1:
        # Beyond a few threads a board routes hardly faster (k30-SBC: 34.2,
        # 32.4 and 31.3 s of search on 1, 2 and 4) but every thread holds a
        # search scratch of the board's size.
        arguments.threads = max(1, min(4, (os.cpu_count() or 1) // arguments.jobs))
    arguments.expected_mb = arguments.expected_gb * 1024
    arguments.gate = MemoryGate(arguments.min_free_gb)
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
        directory = Path(task["directory"]) if Path(task["directory"]).is_absolute() else ROOT / task["directory"]
        if not directory.exists():
            print(f"skipping {task['name']}: {directory} not found")
            continue
        tasks.append(task)
    if tier is not None:
        missing = sorted(set(tier["boards"]) - {task["name"] for task in tasks})
        if missing and not arguments.only:
            print(f"tier {arguments.tier}: {len(missing)} boards not in {arguments.tasks}: {missing}", file=sys.stderr)
        # The longest first, so that the last job is not a slow one.
        tasks.sort(key=lambda task: -(task.get("expected_seconds") or 0))
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
    if arguments.route_from is not None or arguments.budget_scale != 1:
        print(f"mode {'route from ' + str(arguments.route_from) if arguments.route_from else 'layout'}, "
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
                        "mode": "route" if arguments.route_from else "layout"}

        for future in as_completed([pool.submit(guarded, task) for task in tasks]):
            row = future.result()
            rows[row["name"]] = row
            print(json.dumps(row), flush=True)
    wall = time.monotonic() - started
    ran = [rows[task["name"]] for task in tasks if task["name"] in rows]
    ordered = ran + [rows[name] for name in left_out if name in rows]
    (arguments.output / "results.json").write_text(json.dumps(ordered, indent=1))
    passed = sum(1 for row in ordered if row["pass"])
    peak = max((row.get("peak_rss_mb") or 0 for row in ran), default=0)
    spliced_count = len(ordered) - len(ran)
    print(f"{passed}/{len(ordered)} tasks pass ({wall:.0f} s wall, "
          f"largest peak {peak / 1024:.1f} GB"
          + (f"; {spliced_count} rows spliced from {arguments.splice_from}" if spliced_count else "") + ")")
    # Continuous measures, usable from one run (README.md, "How to measure").
    summary = objectives(ordered)
    summary.update(wall_seconds=round(wall), budget_scale=arguments.budget_scale,
                   mode="route" if arguments.route_from else "layout", spliced=spliced_count)
    print(objectives_line(summary))
    (arguments.output / "summary.json").write_text(json.dumps(summary, indent=1))


if __name__ == "__main__":
    main()
