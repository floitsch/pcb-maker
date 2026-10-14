#!/usr/bin/env python3
# Copyright (C) 2026 Toit contributors.
"""Routes the harvested boards with Freerouting, for comparison.

Freerouting takes a Specctra DSN, which carries no copper pours, so every
board is routed cold (tracks, vias and pours stripped), as
benchmarks/corpus/run.py's matched comparison does. The steps are KiCad's
own DSN export (pcbnew), the class rules raised to the board minimums (as
experiments/whole-board does), Freerouting headless with a wall-clock
limit, KiCad's session import, and KiCad's DRC. With --ours the same cold
board is also routed by pcb-maker. Rows go to stdout as JSON lines and to
<output>/results.json; findings are judged as the corpus runner judges them
(native unconnected items, DRC errors beyond the cold board's own and the
designer's). Each router's row records its CPU seconds (rusage), peak
resident set and the load average while it ran: with --jobs several boards
run side by side, and speed is compared in CPU seconds, not wall time."""

import argparse
import json
import os
import re
import shutil
import subprocess
import sys
import threading
import time
from collections import Counter
from pathlib import Path

ROOT = Path(__file__).resolve().parents[2]
sys.path.insert(0, str(ROOT / "benchmarks/corpus"))
from run import COSMETIC, drc_summary, finding_key, run  # noqa: E402

IGNORE = shutil.ignore_patterns(".history", "*-backups", "Gerbers", "packages3D", "*.pdf", "*.zip", "fp-info-cache")
WHOLE_BOARD = ROOT / "experiments/whole-board"


def measured(argv, log, timeout, cwd=None, stall=None):
    """Runs `argv` with its output in `log` (None: discarded) and a wall-clock
    limit; returns the exit code ("timeout" when killed), the wall seconds
    and what wait4 says the process and the children it waited for used:
    CPU seconds (the speed measure; the wall clock stretches with the
    machine's load) and peak resident set. Also the 1-minute load average
    sampled while it ran. No globals: boards run side by side (--jobs).
    `stall`, a pair (seconds, check), kills the process with code "stalled"
    when the check fails once that many seconds have passed."""
    started = time.monotonic()
    sink = open(log, "w") if log is not None else subprocess.DEVNULL
    try:
        process = subprocess.Popen([str(a) for a in argv], stdout=sink, stderr=subprocess.STDOUT, cwd=cwd)
        loads = [os.getloadavg()[0]]
        tier = tier_running()
        last_sample = started
        while True:
            pid, status, usage = os.wait4(process.pid, os.WNOHANG)
            if pid:
                code = os.waitstatus_to_exitcode(status)
                break
            now = time.monotonic()
            stalled = stall is not None and now - started > stall[0] and not stall[1]()
            if now - started > timeout or stalled:
                process.kill()
                _, status, usage = os.wait4(process.pid, 0)
                code = "stalled" if stalled else "timeout"
                break
            if stall is not None and now - started > stall[0]:
                stall = None
            if now - last_sample >= 10:
                loads.append(os.getloadavg()[0])
                tier = tier or tier_running()
                last_sample = now
            time.sleep(0.25)
        process.returncode = code if isinstance(code, int) else -9
    finally:
        if log is not None:
            sink.close()
    return {"code": code, "wall": round(time.monotonic() - started, 1), "load_start": round(loads[0], 1), "beside_tier": tier,
            "cpu": round(usage.ru_utime + usage.ru_stime, 1), "peak_mb": round(usage.ru_maxrss / 1024),
            "load": round(sum(loads) / len(loads), 1)}


class Slots:
    """A semaphore whose size a file can change while the sweep runs
    (<output>/freerouting-jobs, <output>/ours-jobs): big boards' routers take
    more memory than the gate can foresee."""

    def __init__(self, default, path):
        self.default, self.path, self.used = default, path, 0
        self.condition = threading.Condition()

    def size(self):
        try:
            return max(0, int(self.path.read_text().strip()))
        except (OSError, ValueError):
            return self.default

    def acquire(self, blocking=True):
        with self.condition:
            while self.used >= self.size():
                if not blocking:
                    return False
                self.condition.wait(5)
            self.used += 1
            return True

    def release(self):
        with self.condition:
            self.used -= 1
            self.condition.notify_all()

    def __enter__(self):
        self.acquire()

    def __exit__(self, *_):
        self.release()


TIER_LOCK = ROOT / "build/quick-tier.lock"


def tier_running():
    """Whether a quick tier holds build/quick-tier.lock (other agents run
    one beside a sweep; its rows are then measured on a busier machine)."""
    try:
        inode = TIER_LOCK.stat().st_ino
        with open("/proc/locks") as locks:
            return any(line.split()[5].endswith(f":{inode}") for line in locks if len(line.split()) > 5)
    except OSError:
        return False


def python(code, log, timeout=600):
    """Runs pcbnew code in its own process (SWIG boards do not share well);
    returns the exit code, the wall and the CPU seconds."""
    raw = log.with_suffix(".raw")
    result = measured([sys.executable, "-c", code], raw, timeout)
    output = raw.read_text(errors="replace")
    raw.unlink(missing_ok=True)
    log.write_text("\n".join(l for l in output.splitlines() if "PROPERTY_ENUM" not in l))
    return result["code"], result["wall"], result["cpu"]


def available_gb():
    try:
        with open("/proc/meminfo") as meminfo:
            for line in meminfo:
                if line.startswith("MemAvailable:"):
                    return int(line.split()[1]) / (1024 * 1024)
    except OSError:
        pass
    return None


def wait_for_memory(minimum_gb, what):
    """Waits until `minimum_gb` of memory is available: with little swap,
    Linux thrashes instead of killing when memory runs out, and the machine
    looks frozen (2026-10-08: 64 GB used, 0.3 GB available, six hard
    freezes). A board that starts late is better than a machine that
    stops."""
    if minimum_gb <= 0:
        return
    waited = 0
    while True:
        available = 0
        try:
            with open("/proc/meminfo") as meminfo:
                for line in meminfo:
                    if line.startswith("MemAvailable:"):
                        available = int(line.split()[1]) / (1024 * 1024)
        except OSError:
            return
        if available >= minimum_gb:
            if waited:
                print(f"{what}: {available:.0f} GB available after waiting {waited} s", file=sys.stderr)
            return
        if waited == 0:
            print(f"{what}: waiting for {minimum_gb} GB of memory ({available:.1f} GB available)", file=sys.stderr)
        time.sleep(15)
        waited += 15


def reference_findings(name, board_id, directory, work):
    """The designer's own DRC errors by type (a sweep's saved report when
    there is one, else computed here)."""
    saved = ROOT / "build/github-route-v2" / name / "reference/drc.json"
    report = None
    if saved.exists():
        report = json.loads(saved.read_text())
    else:
        reference = work / "reference"
        shutil.copytree(directory, reference, ignore=IGNORE)
        try:
            subprocess.run(["kicad-cli", "pcb", "drc", "--refill-zones", "--severity-error", "--format", "json",
                            "-o", "drc.json", f"{board_id}.kicad_pcb"], cwd=reference,
                           stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL, timeout=600)
        except (OSError, subprocess.TimeoutExpired):
            pass
        if (reference / "drc.json").exists():
            report = json.loads((reference / "drc.json").read_text())
    if report is None:
        return {}, None
    findings = dict(Counter(v["type"] for v in report.get("violations", []) if not COSMETIC.match(v["type"])))
    return findings, len(report.get("unconnected_items", []))


def freeroute(cold, board_id, work, config, timeout, java_heap=None, slot=None, before=None):
    """Freerouting on the cold board: the result board in <work>/result, a
    dictionary of what happened."""
    work.mkdir(parents=True, exist_ok=True)
    board = cold / f"{board_id}.kicad_pcb"
    dsn = work / "input.dsn"
    row = {}
    pipeline_cpu = 0.0
    code, _, cpu = python(f"import pcbnew; b = pcbnew.LoadBoard({str(board)!r}); assert pcbnew.ExportSpecctraDSN(b, {str(dsn)!r})",
                          work / "export.log")
    pipeline_cpu += cpu
    if code != 0 or not dsn.exists():
        return {"status": "export failed", "error": (work / "export.log").read_text()[-300:]}
    # KiCad writes each class's own width and clearance; the effective rule
    # is at least the board minimum (best effort: boards with custom rules
    # or without a project keep the exported values).
    rules = work / "rules.json"
    code, _, cpu = python(
        f"import sys, json; sys.path.insert(0, {str(WHOLE_BOARD)!r}); from compile_net_classes import compile_rules; "
        f"rules, evidence = compile_rules(__import__('pathlib').Path({str(board)!r})); "
        f"open({str(rules)!r}, 'w').write(json.dumps(dict(rules=rules)))",
        work / "rules.log")
    pipeline_cpu += cpu
    if code == 0 and rules.exists():
        translated = measured([sys.executable, WHOLE_BOARD / "translate_dsn_classes.py", dsn, rules,
                               work / "class-translation.json"], work / "translate.log", 300)
        pipeline_cpu += translated["cpu"]
        row["class_translation"] = translated["code"] == 0
    else:
        row["class_translation"] = False
    session = work / "output.ses"
    heap = [f"-Xmx{java_heap}"] if java_heap else []
    argv = [config["java"], *heap, "-Djava.awt.headless=true", "--enable-final-field-mutation=ALL-UNNAMED",
            "-jar", str(ROOT / "benchmarks/corpus" / config["jar"]), "-de", str(dsn), "-do", str(session),
            "-mp", str(config.get("maximum_passes", 100)), "-mt", "1"]
    # Freerouting 2.2.4 loops forever on some DSNs it cannot read (keyboar_:
    # a part number with a backtick, "Component.read_place_scope: ) expected")
    # and never starts its job; such a run is a failure, not a timeout.
    def job_started():
        try:
            return re.search(r"Job .* started", (work / "freerouting.log").read_text(errors="replace")) is not None
        except OSError:
            return False

    # At most --freerouting-jobs JVMs at a time (each grows to several GB:
    # G1 expands toward a quarter of the machine's memory without -Xmx).
    if slot is not None:
        slot.acquire()
    try:
        if before is not None:
            before()
        routed = measured(argv, work / "freerouting.log", timeout, stall=(300, job_started))
    finally:
        if slot is not None:
            slot.release()
    if java_heap:
        row["java_heap"] = java_heap
    row["router_exit"] = routed["code"]
    row["router_seconds"] = routed["wall"]
    row["cpu_seconds"] = routed["cpu"]
    row["peak_mb"] = routed["peak_mb"]
    row["load"] = routed["load"]
    row["load_start"] = routed["load_start"]
    row["beside_tier"] = routed["beside_tier"]
    log_text = (work / "freerouting.log").read_text(errors="replace")
    # Freerouting's own count of what stayed unrouted, from its log.
    found = re.findall(r"(\d+) (?:unrouted|incomplete)", log_text, re.I)
    row["log_unrouted"] = int(found[-1]) if found else None
    if not session.exists():
        row["status"] = {"timeout": "timeout", "stalled": "DSN not read"}.get(row["router_exit"], "no session")
        if "OutOfMemoryError" in log_text:
            row["status"] = "out of memory"
        row["pipeline_cpu_seconds"] = round(pipeline_cpu, 1)
        dsn.unlink(missing_ok=True)
        return row
    result = work / "result"
    shutil.copytree(cold, result)
    result_board = result / f"{board_id}.kicad_pcb"
    code, _, cpu = python(
        f"import pcbnew; b = pcbnew.LoadBoard({str(result_board)!r}); assert pcbnew.ImportSpecctraSES(b, {str(session)!r}); "
        f"pcbnew.SaveBoard({str(result_board)!r}, b); "
        # Index access: under Python 3.14 the SWIG containers cannot be iterated.
        f"ts = b.Tracks(); tracks = [ts[i] for i in range(ts.size())]; vias = sum(1 for t in tracks if t.GetClass() == 'PCB_VIA'); "
        f"length = sum(t.GetLength() for t in tracks if t.GetClass() != 'PCB_VIA') / 1e6; "
        f"print('STATS', vias, round(length, 1))",
        work / "import.log")
    pipeline_cpu += cpu
    if code != 0:
        row["status"] = "import failed"
        row["pipeline_cpu_seconds"] = round(pipeline_cpu, 1)
        return row
    stats = re.search(r"STATS (\d+) ([\d.]+)", (work / "import.log").read_text())
    if stats:
        row["vias"] = int(stats.group(1))
        row["length_mm"] = float(stats.group(2))
    # KiCad's DRC judges the imported board as ours is judged (our router
    # runs the same kicad-cli check, --severity-all, on its own result). The
    # cold board has no copper zones, so there is nothing to refill.
    try:
        checked = measured(["kicad-cli", "pcb", "drc", "--format", "json", "--severity-all", "-o", "drc.json",
                            f"{board_id}.kicad_pcb"], None, 900, cwd=result)
        row["drc_cpu_seconds"] = checked["cpu"]
        pipeline_cpu += checked["cpu"]
    except OSError:
        pass
    row["pipeline_cpu_seconds"] = round(pipeline_cpu, 1)
    row["status"] = "finished" if row["router_exit"] == 0 else str(row["router_exit"])
    for name in ("input.dsn", "output.ses"):
        (work / name).unlink(missing_ok=True)
    return row


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("output", type=Path)
    parser.add_argument("--corpus", type=Path, default=Path(__file__).with_name("boards.json"))
    parser.add_argument("--binary", type=Path, default=ROOT / "target/release/pcb-maker")
    parser.add_argument("--freerouting", type=Path, default=ROOT / "benchmarks/corpus/freerouting.json")
    parser.add_argument("--only", nargs="*")
    parser.add_argument("--ours", action="store_true", help="also route the cold board with pcb-maker")
    parser.add_argument("--skip-freerouting", action="store_true")
    parser.add_argument("--timeout", type=int, default=1500, help="Freerouting's wall-clock limit (and pcb-maker's)")
    parser.add_argument("--min-free-gb", type=float, default=12.0,
                        help="start a board only with this much memory available (0: always)")
    parser.add_argument("--board-gb", type=float, default=0.0,
                        help="with --jobs: a board starts only with --min-free-gb plus this much available "
                             "(what one board's routers grow to after the start)")
    parser.add_argument("--freerouting-jobs", type=int, default=0,
                        help="at most this many Freerouting JVMs at a time (0: --jobs); a board whose turn has not "
                             "come routes with pcb-maker first")
    parser.add_argument("--ours-jobs", type=int, default=0,
                        help="at most this many pcb-maker routes at a time (0: --jobs); the router sizes its "
                             "side-by-side rungs to half the memory available when it starts")
    parser.add_argument("--java-heap", help="Freerouting's -Xmx (e.g. 4g); a run that exceeds it reports 'out of memory'")
    parser.add_argument("--resume", action="store_true", help="skip boards that <output>/results.json already has")
    parser.add_argument("--redo", nargs="*", default=[], help="with --resume: route these boards again (their rows are replaced)")
    parser.add_argument("--jobs", type=int, default=1,
                        help="boards side by side (a file <output>/jobs, if present, overrides it while running)")
    arguments = parser.parse_args()
    arguments.output.mkdir(parents=True, exist_ok=True)
    config = json.loads(arguments.freerouting.read_text())
    boards = []
    for board in json.loads(arguments.corpus.read_text())["boards"]:
        if arguments.only and board["name"] not in arguments.only:
            continue
        directory = Path(board["directory"])
        if not directory.is_absolute():
            directory = ROOT / directory
        if not (directory / f"{board['board_id']}.kicad_pcb").exists():
            print(f"skipping {board['name']}: {directory} not found", file=sys.stderr)
            continue
        boards.append((board, directory))
    order = {board["name"]: index for index, (board, _) in enumerate(boards)}
    rows = []
    if arguments.resume and (arguments.output / "results.json").exists():
        rows = [row for row in json.loads((arguments.output / "results.json").read_text()) if row["name"] in order]
        rows = [row for row in rows if row["name"] not in arguments.redo]
        done = {row["name"] for row in rows}
        boards = [(board, directory) for board, directory in boards if board["name"] not in done]
        print(f"resuming: {len(done)} boards done, {len(boards)} to go", file=sys.stderr, flush=True)
    lock = threading.Lock()
    arguments.slot = Slots(arguments.freerouting_jobs or max(1, arguments.jobs), arguments.output / "freerouting-jobs")
    arguments.ours_slot = Slots(arguments.ours_jobs or max(1, arguments.jobs), arguments.output / "ours-jobs")

    def finish(row):
        with lock:
            rows[:] = [r for r in rows if r["name"] != row["name"]]
            rows.append(row)
            rows.sort(key=lambda r: order[r["name"]])
            print(json.dumps(row), flush=True)
            (arguments.output / "results.json").write_text(json.dumps(rows, indent=1))

    def jobs():
        try:
            # 0 pauses: boards in flight finish, no new one starts.
            return max(0, int((arguments.output / "jobs").read_text().strip()))
        except (OSError, ValueError):
            return max(1, arguments.jobs)

    running = []
    for board, directory in boards:
        while True:
            running = [thread for thread in running if thread.is_alive()]
            if len(running) < jobs():
                break
            time.sleep(5)
        wait_for_memory(arguments.min_free_gb + arguments.board_gb, board["name"])
        thread = threading.Thread(target=lambda b=board, d=directory: finish(route_board(b, d, arguments, config)))
        thread.start()
        running.append(thread)
        print(f"{time.strftime('%H:%M:%S')} started {board['name']} ({len(running)} running, "
              f"{available_gb() or 0:.0f} GB available, load {os.getloadavg()[0]:.1f})", file=sys.stderr, flush=True)
        if jobs() > 1:
            # A board's memory grows after its start: the next waits a little
            # so the gate sees it.
            time.sleep(30)
    for thread in running:
        thread.join()


def route_board(board, directory, arguments, config):
    board_id = board["board_id"]
    work = arguments.output / board["name"]
    if work.exists():
        shutil.rmtree(work)
    cold = work / "cold-source"
    shutil.copytree(directory, cold, ignore=IGNORE)
    row = {"name": board["name"], "load_at_start": round(os.getloadavg()[0], 1)}
    code, _ = run(arguments.binary, ["strip-kicad-copper", directory / f"{board_id}.kicad_pcb",
                                     cold / f"{board_id}.kicad_pcb", work / "strip.json"],
                  work / "strip.log", 300)
    if code != 0:
        row["error"] = "strip failed: " + (work / "strip.log").read_text()[-300:]
        return row
    strip = json.loads((work / "strip.json").read_text())
    row["reference"] = {"segments": strip.get("removed_segments"), "vias": strip.get("removed_vias"),
                        "zones": strip.get("removed_copper_zones")}
    # Findings the cold board already has are not the router's.
    baseline_directory = work / "baseline"
    shutil.copytree(cold, baseline_directory)
    run(arguments.binary, ["verify-kicad-rung", baseline_directory, board_id], work / "baseline.log", 600)
    baseline = frozenset()
    if (baseline_directory / "drc.json").exists():
        baseline = frozenset(
            finding_key(v)
            for v in json.loads((baseline_directory / "drc.json").read_text()).get("violations", []))
    reference, reference_unconnected = reference_findings(board["name"], board_id, directory, work)
    row["reference_findings"] = reference
    row["reference_unconnected"] = reference_unconnected

    def freerouting():
        heap_gb = 0.0
        if arguments.java_heap:
            heap_gb = float(arguments.java_heap.rstrip("gG")) if arguments.java_heap[-1] in "gG" else 0.0
        entry = freeroute(cold, board_id, work / "freerouting", config, arguments.timeout, arguments.java_heap,
                          arguments.slot,
                          lambda: wait_for_memory(arguments.min_free_gb + max(heap_gb + 0.5, arguments.board_gb / 2),
                                                  board["name"] + " (Freerouting)"))
        if (work / "freerouting/result/drc.json").exists():
            entry["native"] = drc_summary(work / "freerouting/result", baseline, reference)
            entry["complete"] = entry["native"]["unconnected"] == 0
        row["freerouting"] = entry

    def ours():
        with arguments.ours_slot:
            wait_for_memory(arguments.min_free_gb + arguments.board_gb / 2, board["name"] + " (pcb-maker)")
            routed = measured([arguments.binary, "route-kicad-board", cold, board_id, work / "cold-routed", "auto"],
                              work / "cold-route.log", arguments.timeout + 300)
        report = work / "cold-routed/board-router.json"
        if report.exists():
            result = json.loads(report.read_text())
            row["cold_route"] = {
                "routed": result["routed_connections"], "connections": result["routable_connections"],
                "unconnected_terminals": result["unconnected_terminals"],
                "vias": result["vias"], "length_mm": round(result["length_mm"], 1),
                "routing_seconds": round(result["routing_seconds"], 2),
                "native_verification_seconds": round(result.get("native_verification_seconds") or 0, 1),
                "wall_seconds": routed["wall"],
                # The whole process, its KiCad checks (kicad-cli children) included.
                "cpu_seconds": routed["cpu"], "peak_mb": routed["peak_mb"], "load": routed["load"],
                "load_start": routed["load_start"], "beside_tier": routed["beside_tier"],
                "exit": routed["code"],
                "internal_violations": len(result["internal_violations"]),
                "native": drc_summary(work / "cold-routed", baseline, reference),
            }
        else:
            row["cold_route"] = {"error": (work / "cold-route.log").read_text()[-300:], "exit": routed["code"],
                                 "wall_seconds": routed["wall"], "cpu_seconds": routed["cpu"]}

    steps = ([] if arguments.skip_freerouting else [freerouting]) + ([ours] if arguments.ours else [])
    if len(steps) == 2 and not arguments.slot.acquire(blocking=False):
        steps.reverse()
    elif len(steps) == 2:
        arguments.slot.release()
    for step in steps:
        step()
    shutil.rmtree(baseline_directory, ignore_errors=True)
    return row


if __name__ == "__main__":
    main()
