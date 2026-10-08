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
designer's)."""

import argparse
import json
import re
import shutil
import subprocess
import sys
import time
from collections import Counter
from pathlib import Path

ROOT = Path(__file__).resolve().parents[2]
sys.path.insert(0, str(ROOT / "benchmarks/corpus"))
from run import COSMETIC, drc_summary, finding_key, run  # noqa: E402

IGNORE = shutil.ignore_patterns(".history", "*-backups", "Gerbers", "packages3D", "*.pdf", "*.zip", "fp-info-cache")
WHOLE_BOARD = ROOT / "experiments/whole-board"


def python(code, log, timeout=600):
    """Runs pcbnew code in its own process (SWIG boards do not share well)."""
    started = time.monotonic()
    try:
        process = subprocess.run([sys.executable, "-c", code], stdout=subprocess.PIPE, stderr=subprocess.STDOUT,
                                 text=True, timeout=timeout)
        output, code_ = process.stdout, process.returncode
    except subprocess.TimeoutExpired as error:
        output, code_ = str(error.stdout or ""), "timeout"
    log.write_text("\n".join(l for l in output.splitlines() if "PROPERTY_ENUM" not in l))
    return code_, time.monotonic() - started



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


def freeroute(cold, board_id, work, config, timeout):
    """Freerouting on the cold board: the result board in <work>/result, a
    dictionary of what happened."""
    work.mkdir(parents=True, exist_ok=True)
    board = cold / f"{board_id}.kicad_pcb"
    dsn = work / "input.dsn"
    row = {}
    code, _ = python(f"import pcbnew; b = pcbnew.LoadBoard({str(board)!r}); assert pcbnew.ExportSpecctraDSN(b, {str(dsn)!r})",
                     work / "export.log")
    if code != 0 or not dsn.exists():
        return {"status": "export failed", "error": (work / "export.log").read_text()[-300:]}
    # KiCad writes each class's own width and clearance; the effective rule
    # is at least the board minimum (best effort: boards with custom rules
    # or without a project keep the exported values).
    rules = work / "rules.json"
    code, _ = python(
        f"import sys, json; sys.path.insert(0, {str(WHOLE_BOARD)!r}); from compile_net_classes import compile_rules; "
        f"rules, evidence = compile_rules(__import__('pathlib').Path({str(board)!r})); "
        f"open({str(rules)!r}, 'w').write(json.dumps(dict(rules=rules)))",
        work / "rules.log")
    if code == 0 and rules.exists():
        code, _ = run(sys.executable, [WHOLE_BOARD / "translate_dsn_classes.py", dsn, rules, work / "class-translation.json"],
                      work / "translate.log", 300)
        row["class_translation"] = code == 0
    else:
        row["class_translation"] = False
    session = work / "output.ses"
    argv = [config["java"], "-Djava.awt.headless=true", "--enable-final-field-mutation=ALL-UNNAMED",
            "-jar", str(ROOT / "benchmarks/corpus" / config["jar"]), "-de", str(dsn), "-do", str(session),
            "-mp", str(config.get("maximum_passes", 100)), "-mt", "1"]
    started = time.monotonic()
    try:
        with open(work / "freerouting.log", "w") as log:
            process = subprocess.run(argv, stdout=log, stderr=subprocess.STDOUT, timeout=timeout)
        row["router_exit"] = process.returncode
    except subprocess.TimeoutExpired:
        row["router_exit"] = "timeout"
    row["router_seconds"] = round(time.monotonic() - started, 1)
    log_text = (work / "freerouting.log").read_text(errors="replace")
    # Freerouting's own count of what stayed unrouted, from its log.
    found = re.findall(r"(\d+) (?:unrouted|incomplete)", log_text, re.I)
    row["log_unrouted"] = int(found[-1]) if found else None
    if not session.exists():
        row["status"] = "timeout" if row["router_exit"] == "timeout" else "no session"
        dsn.unlink(missing_ok=True)
        return row
    result = work / "result"
    shutil.copytree(cold, result)
    result_board = result / f"{board_id}.kicad_pcb"
    code, _ = python(
        f"import pcbnew; b = pcbnew.LoadBoard({str(result_board)!r}); assert pcbnew.ImportSpecctraSES(b, {str(session)!r}); "
        f"pcbnew.SaveBoard({str(result_board)!r}, b); "
        # Index access: under Python 3.14 the SWIG containers cannot be iterated.
        f"ts = b.Tracks(); tracks = [ts[i] for i in range(ts.size())]; vias = sum(1 for t in tracks if t.GetClass() == 'PCB_VIA'); "
        f"length = sum(t.GetLength() for t in tracks if t.GetClass() != 'PCB_VIA') / 1e6; "
        f"print('STATS', vias, round(length, 1))",
        work / "import.log")
    if code != 0:
        row["status"] = "import failed"
        return row
    stats = re.search(r"STATS (\d+) ([\d.]+)", (work / "import.log").read_text())
    if stats:
        row["vias"] = int(stats.group(1))
        row["length_mm"] = float(stats.group(2))
    try:
        subprocess.run(["kicad-cli", "pcb", "drc", "--format", "json", "-o", "drc.json", f"{board_id}.kicad_pcb"],
                       cwd=result, stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL, timeout=900)
    except (OSError, subprocess.TimeoutExpired):
        pass
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
    arguments = parser.parse_args()
    arguments.output.mkdir(parents=True, exist_ok=True)
    config = json.loads(arguments.freerouting.read_text())
    rows = []
    for board in json.loads(arguments.corpus.read_text())["boards"]:
        if arguments.only and board["name"] not in arguments.only:
            continue
        directory = Path(board["directory"])
        if not directory.is_absolute():
            directory = ROOT / directory
        board_id = board["board_id"]
        if not (directory / f"{board_id}.kicad_pcb").exists():
            print(f"skipping {board['name']}: {directory} not found", file=sys.stderr)
            continue
        wait_for_memory(arguments.min_free_gb, board["name"])
        work = arguments.output / board["name"]
        if work.exists():
            shutil.rmtree(work)
        cold = work / "cold-source"
        shutil.copytree(directory, cold, ignore=IGNORE)
        row = {"name": board["name"]}
        code, _ = run(arguments.binary, ["strip-kicad-copper", directory / f"{board_id}.kicad_pcb",
                                         cold / f"{board_id}.kicad_pcb", work / "strip.json"],
                      work / "strip.log", 300)
        if code != 0:
            row["error"] = "strip failed: " + (work / "strip.log").read_text()[-300:]
            rows.append(row)
            print(json.dumps(row), flush=True)
            continue
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

        if not arguments.skip_freerouting:
            entry = freeroute(cold, board_id, work / "freerouting", config, arguments.timeout)
            if (work / "freerouting/result/drc.json").exists():
                entry["native"] = drc_summary(work / "freerouting/result", baseline, reference)
                entry["complete"] = entry["native"]["unconnected"] == 0
            row["freerouting"] = entry

        if arguments.ours:
            code, seconds = run(arguments.binary, ["route-kicad-board", cold, board_id, work / "cold-routed", "auto"],
                                work / "cold-route.log", arguments.timeout + 300)
            report = work / "cold-routed/board-router.json"
            if report.exists():
                result = json.loads(report.read_text())
                row["cold_route"] = {
                    "routed": result["routed_connections"], "connections": result["routable_connections"],
                    "unconnected_terminals": result["unconnected_terminals"],
                    "vias": result["vias"], "length_mm": round(result["length_mm"], 1),
                    "routing_seconds": round(result["routing_seconds"], 2),
                    "wall_seconds": round(seconds, 1),
                    "internal_violations": len(result["internal_violations"]),
                    "native": drc_summary(work / "cold-routed", baseline, reference),
                }
            else:
                row["cold_route"] = {"error": (work / "cold-route.log").read_text()[-300:], "exit": code}
        shutil.rmtree(baseline_directory, ignore_errors=True)
        rows.append(row)
        print(json.dumps(row), flush=True)
        (arguments.output / "results.json").write_text(json.dumps(rows, indent=1))


if __name__ == "__main__":
    main()
