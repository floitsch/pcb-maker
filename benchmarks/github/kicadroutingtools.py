#!/usr/bin/env python3
# Copyright (C) 2026 Toit contributors.
"""KiCadRoutingTools on the placements of a route-mode run, judged as run.py
judges ours.

    benchmarks/github/kicadroutingtools.py <output> --ours build/fr2-route-x1367 \\
        --route-from build/github-layout-v17 --binary build/bin/pcb-maker-x1367 --jobs 8

KiCadRoutingTools (github.com/drandyhaas/KiCadRoutingTools, MIT; Python with
a Rust A* core) is third-party code: clone it into an empty directory of its
own (build/krt/src), build its Rust module there in a venv (README,
"KiCadRoutingTools"), and run it only from a work directory of ours, never
from inside its checkout. This script runs its CLI router headless,
`py_router/route.py <board> <result> --nets '*'`, on each board's placement
(`<route-from>/<task>/layout/placed`: the layout's parts, keepouts and the
designer's pours, no tracks) as it is; route.py takes the pours as planes,
welds their pads into the fill and finishes with its own KiCad-oracle
completion pass. Two flags keep the judging ours: `--no-fix-drc-settings`
(by default route.py loosens the written project's Board Setup floors to
what it routed) and `--escalation board` (by default a failing net may
descend below the board's own minimums to the fab tier's). The project
files the result is judged with are the placement's, copied over whatever
route.py wrote.

route.py has no time limit and no thread setting (its search is one thread;
the BLAS pools are pinned to one with OMP_NUM_THREADS and friends), and a
killed run leaves no board. So each board runs to completion or to a cap
(`--cap-factor` times our row's wall, between `--cap-min` and `--cap-max`
seconds) and the row records its wall next to ours: `within_wall` says
whether it finished within the wall our row took, `pass_in_wall` whether it
also passed. The result is judged with run.py's own functions: KiCad's
unconnected items and copper errors (zones refilled) less the designer's per
type (`pass`), and the strict rule against the designer's stripped board at
the same place (`pass_strict`). CPU seconds and peak resident set come from
wait4 (route.py and the kicad-cli children it waited for); the row's
`cpu_seconds` adds the judging DRC, as ours counts its own KiCad checks.
Rows go to stdout as JSON lines and to <output>/results.json."""

import argparse
import contextlib
import fcntl
import importlib.util
import json
import math
import os
import re
import shutil
import sys
import threading
import time
from pathlib import Path

ROOT = Path(__file__).resolve().parents[2]
sys.path.insert(0, str(ROOT / "benchmarks/github"))
import freerouting  # noqa: E402

sys.path.insert(1, str(ROOT / "benchmarks/agent-tasks"))
_spec = importlib.util.spec_from_file_location("agent_run", ROOT / "benchmarks/agent-tasks/run.py")
agent_run = importlib.util.module_from_spec(_spec)
_spec.loader.exec_module(agent_run)

# The BLAS and OpenMP pools numpy and scipy start: one thread, as the router's
# search is one (`--blas-threads` changes it for the thread comparison).
POOLS = ("OMP_NUM_THREADS", "OPENBLAS_NUM_THREADS", "MKL_NUM_THREADS", "NUMEXPR_NUM_THREADS")


def our_rows(path):
    """The rows of a run.py output directory (results.json) or log, by name
    (the last row of a board wins)."""
    if path is None:
        return {}
    path = Path(path)
    if path.is_dir():
        rows = json.loads((path / "results.json").read_text())
    else:
        rows = [json.loads(line) for line in path.read_text().splitlines() if line.startswith("{")]
    return {row["name"]: row for row in rows}


def copper_stats(board):
    """Vias and track length (mm) of a .kicad_pcb, from its text: board-level
    segments and arcs (an arc by its chord's circle through the mid point)."""
    text = board.read_text(errors="replace")
    vias = len(re.findall(r"^\s*\(via\b", text, re.M))
    length = 0.0
    for start, end in re.findall(r"\(segment\s+\(start ([-\d.e]+ [-\d.e]+)\)\s+\(end ([-\d.e]+ [-\d.e]+)\)", text):
        (x0, y0), (x1, y1) = (map(float, start.split())), (map(float, end.split()))
        length += math.hypot(x1 - x0, y1 - y0)
    for start, mid, end in re.findall(
            r"\(arc\s+\(start ([-\d.e]+ [-\d.e]+)\)\s+\(mid ([-\d.e]+ [-\d.e]+)\)\s+\(end ([-\d.e]+ [-\d.e]+)\)", text):
        points = [tuple(map(float, p.split())) for p in (start, mid, end)]
        length += arc_length(*points)
    return vias, round(length, 1)


def arc_length(a, m, b):
    """The length of the circular arc from a through m to b."""
    ax, ay = a
    bx, by = b
    mx, my = m
    d = 2 * (ax * (my - by) + mx * (by - ay) + bx * (ay - my))
    if abs(d) < 1e-12:
        return math.hypot(bx - ax, by - ay)
    ux = ((ax * ax + ay * ay) * (my - by) + (mx * mx + my * my) * (by - ay) + (bx * bx + by * by) * (ay - my)) / d
    uy = ((ax * ax + ay * ay) * (bx - mx) + (mx * mx + my * my) * (ax - bx) + (bx * bx + by * by) * (mx - ax)) / d
    r = math.hypot(ax - ux, ay - uy)
    angle = lambda p: math.atan2(p[1] - uy, p[0] - ux)
    t0, tm, t1 = angle(a), angle(m), angle(b)
    sweep = (t1 - t0) % (2 * math.pi)
    if (tm - t0) % (2 * math.pi) > sweep:
        sweep = 2 * math.pi - sweep
    return r * sweep


def krt_summary(path):
    """The merged JSON_SUMMARY route.py wrote with --json-out (the parts this
    row keeps), or None."""
    try:
        summary = json.loads(path.read_text())
    except (OSError, ValueError):
        return None
    if isinstance(summary, list):
        summary = summary[-1] if summary else {}
    kept = {}
    for key in ("successful", "failed", "total_vias", "pad_pairs_connected", "pad_pairs_total", "total_time",
                "min_clearance_used", "oracle_check"):
        if key in summary:
            kept[key] = summary[key]
    if isinstance(summary.get("via_in_pad"), dict):
        kept["via_in_pad"] = summary["via_in_pad"].get("count")
    if isinstance(summary.get("design_rules"), dict) and "count" in summary["design_rules"]:
        # Features delivered below their requested size (down to the board's floors).
        kept["narrowed"] = summary["design_rules"]["count"]
    return kept or {"keys": sorted(summary)[:40]}


@contextlib.contextmanager
def machine_lock(path):
    """Holds an exclusive flock on `path` (the machine's tier lock,
    build/quick-tier.lock: one heavy run at a time), waiting for it; no
    lock when `path` is None."""
    if path is None:
        yield 0.0
        return
    started = time.monotonic()
    with open(path, "a") as handle:
        fcntl.flock(handle, fcntl.LOCK_EX)
        try:
            yield round(time.monotonic() - started, 1)
        finally:
            fcntl.flock(handle, fcntl.LOCK_UN)


def wait_for_memory(minimum_gb, what, poll=20):
    """Waits until /proc/meminfo's MemAvailable is at least `minimum_gb`,
    looking every `poll` seconds (the admission check: a board starts only
    then)."""
    waited = 0
    while True:
        available = freerouting.available_gb()
        if available is None or available >= minimum_gb:
            if waited:
                print(f"{what}: {available:.1f} GB available after waiting {waited} s", file=sys.stderr, flush=True)
            return
        if waited == 0:
            print(f"{what}: waiting for {minimum_gb} GB of memory ({available:.1f} GB available)",
                  file=sys.stderr, flush=True)
        time.sleep(poll)
        waited += poll


def route_board(task, ours, arguments):
    """One board: route.py outside the machine lock (one thread, its memory
    admitted by --min-free-gb), KiCad's checks (the designer's board, the
    result, the strict baseline) inside it (`--lock`); the waits for the
    lock are kept out of the wall and recorded."""
    name, board_id = task["name"], task["board_id"]
    work = arguments.output / name
    if work.exists():
        shutil.rmtree(work)
    work.mkdir(parents=True)
    placed = arguments.route_from / name / "layout" / "placed"
    row = {"name": name, "mode": "krt-route", "route_from": str(placed), "krt_commit": arguments.krt_commit}
    if not (placed / f"{board_id}.kicad_pcb").exists():
        row.update(error=f"no placement in {placed}", **{"pass": False, "pass_strict": False})
        return row
    ours_seconds = ours.get("seconds") if ours else None
    reference_wall = ours_seconds or arguments.default_wall
    cap = min(arguments.cap_max, max(arguments.cap_min, arguments.cap_factor * reference_wall))
    row.update(ours_seconds=ours_seconds, cap_seconds=round(cap))
    source = work / "input"
    source.mkdir()
    for extension in (".kicad_pcb", ".kicad_pro", ".kicad_dru"):
        path = placed / f"{board_id}{extension}"
        if path.exists():
            shutil.copyfile(path, source / path.name)
    result = work / "result"
    result.mkdir()
    result_board = result / f"{board_id}.kicad_pcb"
    directory = agent_run.task_directory(task)
    row["lock_wait_seconds"] = 0.0
    baseline = agent_run.StrictBaseline(directory, board_id, work, arguments.binary, bool(task.get("designer_budget")))
    argv = [arguments.python, "-E", "-s", arguments.krt / "py_router/route.py", source / f"{board_id}.kicad_pcb",
            result_board, "--nets", "*", "--no-fix-drc-settings", "--escalation", "board",
            "--json-out", work / "krt-summary.json", *arguments.krt_args]
    # At most --krt-jobs routers at a time; boards whose router is done wait
    # for the lock to be judged without holding a router's place.
    with arguments.router_slots:
        wait_for_memory(arguments.min_free_gb, name + " (KiCadRoutingTools)")
        load_before = os.getloadavg()[0]
        # The work directory is the current directory: never the checkout's.
        routed = freerouting.measured(argv, work / "krt.log", cap, cwd=work)
    row.update(exit=routed["code"], seconds=routed["wall"], krt_cpu_seconds=routed["cpu"], peak_mb=routed["peak_mb"],
               load=[round(load_before, 1), round(os.getloadavg()[0], 1)], load_mean=routed["load"],
               beside_tier=routed["beside_tier"], blas_threads=arguments.blas_threads)
    row["timed_out"] = routed["code"] == "timeout"
    summary = krt_summary(work / "krt-summary.json")
    if summary is not None:
        row["krt_summary"] = summary
    if not result_board.exists():
        row.update(status="timeout" if row["timed_out"] else "no board", **{"pass": False, "pass_strict": False})
        row["cpu_seconds"] = row["krt_cpu_seconds"]
        row["within_wall"] = False
        row["pass_in_wall"] = False
        return row
    # Judge with the placement's project, whatever route.py wrote beside the
    # board (its Board Setup floors stay the designer's).
    for extension in (".kicad_pro", ".kicad_dru"):
        written = result / f"{board_id}{extension}"
        given = source / f"{board_id}{extension}"
        if written.exists() and given.exists() and written.read_bytes() != given.read_bytes():
            row.setdefault("project_rewritten", []).append(extension)
        if given.exists():
            shutil.copyfile(given, written)
        elif written.exists():
            written.unlink()
            row.setdefault("project_removed", []).append(extension)
    row["vias"], row["length_mm"] = copper_stats(result_board)
    with machine_lock(arguments.lock) as waited:
        row["lock_wait_seconds"] += waited
        allowed = {}
        if task.get("designer_budget"):
            # Also keeps the designer's own solder mask bridges for the strict
            # rule (designer-mask-bridges.json), as run.py does.
            allowed = agent_run.designer_findings(directory, board_id, work, arguments.binary)
        if ours is not None and "designer_findings" in ours:
            allowed = ours["designer_findings"]
        checked = freerouting.measured([arguments.binary, "drc-kicad-board", result, board_id, result / "drc.json"],
                                       work / "drc.log", 1800)
        strict = agent_run.strict_native(result, baseline)
    row["drc_cpu_seconds"] = checked["cpu"]
    row["cpu_seconds"] = round(row["krt_cpu_seconds"] + checked["cpu"], 1)
    copper = agent_run.native(result, allowed)
    if allowed:
        row["designer_findings"] = allowed
    row["native"] = copper
    row["pass"] = bool(copper is not None and copper["unconnected"] == 0 and not copper["errors"])
    agent_run.strict_row(row, strict)
    if copper is None:
        row["error"] = "no DRC report"
    row["status"] = "finished" if routed["code"] == 0 else f"exit {routed['code']}"
    row["within_wall"] = ours_seconds is not None and row["seconds"] <= ours_seconds
    row["pass_in_wall"] = row["within_wall"] and row["pass"]
    row["pass_strict_in_wall"] = row["within_wall"] and row["pass_strict"]
    return row


def main():
    parser = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    parser.add_argument("output", type=Path)
    parser.add_argument("--route-from", type=Path, required=True)
    parser.add_argument("--ours", type=Path, help="run.py's route-mode output (or log): each board's wall")
    parser.add_argument("--default-wall", type=float, default=1500.0, help="for boards without a row of ours")
    parser.add_argument("--cap-factor", type=float, default=2.0, help="the kill limit in multiples of our wall")
    parser.add_argument("--cap-min", type=float, default=600.0)
    parser.add_argument("--cap-max", type=float, default=3600.0)
    parser.add_argument("--krt", type=Path, default=ROOT / "build/krt/src", help="the KiCadRoutingTools checkout")
    parser.add_argument("--python", type=Path, default=ROOT / "build/krt/venv/bin/python",
                        help="the venv with numpy, scipy and shapely")
    parser.add_argument("--krt-args", nargs=argparse.REMAINDER, default=[],
                        help="more route.py options (last on the command line)")
    parser.add_argument("--blas-threads", type=int, default=1, help="OMP_NUM_THREADS and friends for route.py")
    parser.add_argument("--tasks", type=Path, default=ROOT / "benchmarks/agent-tasks/tasks/v7/tasks.json")
    parser.add_argument("--only", nargs="*")
    parser.add_argument("--binary", type=Path, required=True, help="for KiCad's DRC (drc-kicad-board) and the baseline")
    parser.add_argument("--jobs", type=int, default=1,
                        help="boards side by side (a file <output>/jobs, if present, overrides it while running)")
    parser.add_argument("--krt-jobs", type=int, default=0,
                        help="at most this many route.py processes at a time (0: --jobs); the other boards in "
                             "flight wait for the lock to be judged")
    parser.add_argument("--min-free-gb", type=float, default=12.0)
    parser.add_argument("--resume", action="store_true")
    parser.add_argument("--shortest-first", action="store_true", help="the boards with the shortest walls of ours first")
    parser.add_argument("--lock", type=Path,
                        help="hold this flock around KiCad's checks of each board (the machine's tier lock, "
                             "build/quick-tier.lock); route.py itself runs outside it")
    arguments = parser.parse_args()
    # Absolute: route.py runs in the board's work directory.
    arguments.output = arguments.output.resolve()
    arguments.route_from = arguments.route_from.resolve()
    arguments.binary = arguments.binary.resolve()
    arguments.python = arguments.python.absolute()
    arguments.output.mkdir(parents=True, exist_ok=True)
    # Set once, before the boards' threads start: route.py inherits them.
    for key in POOLS:
        os.environ[key] = str(arguments.blas_threads)
    arguments.krt = arguments.krt.resolve()
    head = arguments.krt / ".git" / "HEAD"
    try:
        reference = head.read_text().strip()
        if reference.startswith("ref: "):
            reference = (arguments.krt / ".git" / reference[5:]).read_text().strip()
        arguments.krt_commit = reference[:12]
    except OSError:
        arguments.krt_commit = None
    tasks = json.loads(arguments.tasks.read_text())
    tasks = tasks["tasks"] if isinstance(tasks, dict) else tasks
    if arguments.only:
        tasks = [task for task in tasks if task["name"] in arguments.only]
    ours = our_rows(arguments.ours)
    order = {task["name"]: index for index, task in enumerate(tasks)}
    rows = []
    results = arguments.output / "results.json"
    if arguments.resume and results.exists():
        rows = [row for row in json.loads(results.read_text()) if row["name"] in order]
        done = {row["name"] for row in rows}
        tasks = [task for task in tasks if task["name"] not in done]
    lock = threading.Lock()
    arguments.router_slots = threading.Semaphore(arguments.krt_jobs or max(1, arguments.jobs))

    def finish(row):
        with lock:
            rows[:] = [r for r in rows if r["name"] != row["name"]] + [row]
            rows.sort(key=lambda r: order[r["name"]])
            print(json.dumps(row), flush=True)
            results.write_text(json.dumps(rows, indent=1))

    def jobs():
        try:
            return max(0, int((arguments.output / "jobs").read_text().strip()))
        except (OSError, ValueError):
            return max(1, arguments.jobs)

    def guarded(task):
        try:
            return route_board(task, ours.get(task["name"]), arguments)
        except Exception as error:  # a board's failure is its row, not the sweep's end
            return {"name": task["name"], "mode": "krt-route", "error": repr(error)[:300], "pass": False,
                    "pass_strict": False}

    # The longest walls first: the sweep is as long as its longest board
    # (--shortest-first: the most boards soonest, one at a time).
    sign = 1 if arguments.shortest_first else -1
    tasks.sort(key=lambda task: sign * ((ours.get(task["name"]) or {}).get("seconds") or arguments.default_wall))
    running = []
    for task in tasks:
        while True:
            running = [thread for thread in running if thread.is_alive()]
            if len(running) < jobs():
                break
            time.sleep(5)
        wait_for_memory(arguments.min_free_gb, task["name"])
        thread = threading.Thread(target=lambda t=task: finish(guarded(t)))
        thread.start()
        running.append(thread)
        print(f"{time.strftime('%H:%M:%S')} started {task['name']} ({len(running)} running, "
              f"load {os.getloadavg()[0]:.1f})", file=sys.stderr, flush=True)
        if jobs() > 1:
            time.sleep(20)
    for thread in running:
        thread.join()
    passing = sum(1 for row in rows if row.get("pass"))
    strict = sum(1 for row in rows if row.get("pass_strict"))
    in_wall = sum(1 for row in rows if row.get("pass_in_wall"))
    unconnected = sum((row.get("native") or {}).get("unconnected", 0) for row in rows)
    print(f"{passing}/{len(rows)} boards pass ({in_wall} within our wall); strict rule: {strict} pass; "
          f"KiCad unconnected {unconnected}", file=sys.stderr, flush=True)


if __name__ == "__main__":
    main()
