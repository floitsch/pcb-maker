#!/usr/bin/env python3
# Copyright (C) 2026 Toit contributors.
"""Runs the agent-style layout tasks of tasks.json.

    benchmarks/agent-tasks/run.py <output> [--only NAME...] [--binary PATH]
    benchmarks/agent-tasks/run.py <output> --tier quick --jobs 12 --binary PATH


Each task copies a real board, strips its tracks, optionally stacks every
movable footprint at one point and removes the outline (a fresh netlist
import), and lays it out with `layout-kicad-board` from the task's
constraints. A task passes when every constraint holds (except the ones the
task lists as geometrically impossible), every connection is routed, and
native KiCad reports no unconnected item and no copper error.

Each row records the board's peak memory: `peak_rss_mb` for the process
tree (kicad-cli's DRC included, 2.2 GB on any board) and `layout_rss_mb` for
the layout process alone."""

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
ROOT = HERE.parents[1]
# Benchmark tiers: subsets of a task list with their reasons (README.md).
TIERS = {"quick": HERE / "quick.json"}
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
            movable = re.search(r'\(pad [^\n]*[\s\S]*?\(net "(?!unconnected-)[^"]+"\)', block) and not re.search(
                r"^\(footprint \"[^\"]*\"\s+(?:\(locked (?:yes)?\)|locked)", block) and not (
                reference and reference.group(1) in keep)
            if reference and reference.group(1) and not re.search(
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
    command's own peak in MB). The first peak is wait4's ru_maxrss: the
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
    return code, output, round(usage.ru_maxrss / 1024), round(own)


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


def run_admitted(task, arguments, token):
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
    layout = {"placer": {"constraints": "constraints.json", **task.get("placer", {})}, **task.get("layout", {})}
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
    code, output, peak_mb, own_mb = run_measured(command, arguments.timeout, env,
                                                 lambda pid: arguments.gate.started(token, pid))
    (work / "layout.log").write_text(output)
    row = {"name": name, "seconds": round(time.monotonic() - started, 1), "peak_rss_mb": peak_mb,
           "layout_rss_mb": own_mb}
    if code == "timeout":
        row.update(error=f"timed out after {arguments.timeout} s", **{"pass": False})
        return row
    report = work / "layout/board-layout.json"
    if not report.exists():
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
    row.update(
        routed=f"{routed['routed_connections']}/{routed['routable_connections']}",
        vias=routed["vias"], length_mm=round(routed["length_mm"], 1),
        constraints=len(result["constraints"]), missed=missed, unexpected_misses=unexpected,
        native=copper,
    )
    # A board passes on KiCad's connectivity, not on the router's own count
    # (Florian, 2026-10-07): KiCad joins a pad through a pour's fill that
    # the router, routing pours as tracks, does not credit (MokyaLora's
    # one open pad, 261/262 with KiCad at 0 unconnected).
    row["pass"] = bool(not unexpected and copper is not None and copper["unconnected"] == 0 and not copper["errors"])
    return row


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
    parser.add_argument("--jobs", type=int, default=1)
    parser.add_argument("--threads", type=int, default=None,
                        help="search threads per board (RAYON_NUM_THREADS; default with several jobs: "
                             "the cores shared among them, at most 4; with one job: all)")
    parser.add_argument("--min-free-gb", type=float, default=12.0,
                        help="start a board only when this much memory stays available after the "
                             "expected peaks of it and the boards running (0: always)")
    parser.add_argument("--expected-gb", type=float, default=3.0,
                        help="expected peak of a board the tier does not estimate")
    arguments = parser.parse_args()
    arguments.output.mkdir(parents=True, exist_ok=True)
    tier = None
    if arguments.tier:
        tier = json.loads(TIERS[arguments.tier].read_text())
        if arguments.tasks is None:
            arguments.tasks = ROOT / tier["tasks"]
    if arguments.tasks is None:
        arguments.tasks = HERE / "tasks.json"
    if arguments.threads is None and arguments.jobs > 1:
        # Beyond a few threads a board routes hardly faster (k30-SBC: 34.2,
        # 32.4 and 31.3 s of search on 1, 2 and 4) but every thread holds a
        # search scratch of the board's size.
        arguments.threads = max(1, min(4, (os.cpu_count() or 1) // arguments.jobs))
    arguments.expected_mb = arguments.expected_gb * 1024
    arguments.gate = MemoryGate(arguments.min_free_gb)
    tasks = []
    for task in json.loads(arguments.tasks.read_text())["tasks"]:
        if arguments.only and task["name"] not in arguments.only:
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
    started = time.monotonic()
    from concurrent.futures import ThreadPoolExecutor, as_completed
    with ThreadPoolExecutor(arguments.jobs) as pool:
        def guarded(task):
            # A runner failure on one task fails that task, not the run.
            try:
                return run_task(task, arguments)
            except Exception as error:
                return {"name": task["name"], "pass": False, "error": f"runner: {type(error).__name__}: {error}"}

        for future in as_completed([pool.submit(guarded, task) for task in tasks]):
            row = future.result()
            rows[row["name"]] = row
            print(json.dumps(row), flush=True)
    ordered = [rows[task["name"]] for task in tasks if task["name"] in rows]
    (arguments.output / "results.json").write_text(json.dumps(ordered, indent=1))
    passed = sum(1 for row in ordered if row["pass"])
    peak = max((row.get("peak_rss_mb") or 0 for row in ordered), default=0)
    print(f"{passed}/{len(ordered)} tasks pass ({time.monotonic() - started:.0f} s wall, "
          f"largest peak {peak / 1024:.1f} GB)")


if __name__ == "__main__":
    main()
