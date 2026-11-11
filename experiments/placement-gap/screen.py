#!/usr/bin/env python3
# Copyright (C) 2026 Toit contributors.
# Use of this source code is governed by an MIT-style license that can be
# found in the LICENSE file.
"""Stage 1 of the placer protocol for a placer switch: the five seed pairs
on the eleven dense boards with `stop_after_first_route` (docs/benchmarks.md,
"The ladder of placer measurements"); with `--full`, stage 2:
the same units as full layouts.

    screen.py --out DIR --off-binary PATH --on-binary PATH --on '{"tuning": {"align_rows": {}}}' [--slots 5]
    screen.py --out DIR --off-binary PATH --on-binary PATH --on '{}' --on-env PCB_PLACER_SKIP_ANNEAL=1
    benchmarks/agent-tasks/placer_ab.py compare --metric first_open \\
        --off DIR/dense-s*-off-* --on DIR/dense-s*-on-*

One run.py per (seed, side, board) at --jobs 1 --threads 2 (a layout is
the same at any job count), at most `--slots` in flight, the largest
expected layouts first, each with --expected-gb from the board's measured
layout peak (`--estimates`, default benchmarks/agent-tasks/estimates.json;
the nightly's sidecar build/nightly/estimates.json is the freshest). Units with a `.done`
file are skipped (restartable). `--on-env KEY=VALUE` (repeatable) sets an
environment variable for the on side's run.py and its binary only (a
switch read from the environment, not a settings key); the off side runs
with those variables unset.
"""

import argparse
import json
import os
import subprocess
import sys
import time
from pathlib import Path

ROOT = Path(__file__).resolve().parents[2]
TASKS = ROOT / "benchmarks/agent-tasks/tasks/v7/tasks.json"
ESTIMATES = ROOT / "benchmarks/agent-tasks/estimates.json"
DENSE = ["thpoll83__PolyKybd__poly_kybd_split72_left", "stonedDiscord__MegaDrive__MegaDrive",
         "stonedDiscord__nonSNES__SNSP-CPU-01", "doudar__SmartSpin2k__SmartSpin2k_Panelized",
         "GlasgowEmbedded__glasgow__glasgow__revC3", "CRImier__MyKiCad__protoesp",
         "ohdsp__DSP-ADAU1452__DSP-ADAU1452", "OpenDrone-hw__OpenESC-30x30__4in1",
         "Huaqiu-Electronics__ecad-viewer__video", "0xCB-dev__0xCB-1337__panel",
         "OpenDrone-hw__OpenESC-20x20__4in1-mini"]
SEEDS = (1, 101, 201, 301, 401)


def estimate(board, estimates=ESTIMATES):
    entry = json.loads(Path(estimates).read_text())["boards"][board]["layout"]
    return round(entry["peak_rss_mb"] / 1024 + 0.05, 1), entry["seconds"]


def on_environment(pairs):
    """The on side's environment additions from `--on-env KEY=VALUE` items."""
    result = {}
    for pair in pairs or []:
        key, separator, value = pair.partition("=")
        if not separator or not key:
            raise ValueError(f"--on-env wants KEY=VALUE, not {pair!r}")
        result[key] = value
    return result


def environment(base, extra):
    """`base` with `extra` applied (a value of None removes the variable)."""
    result = {key: value for key, value in base.items() if key not in extra}
    result.update({key: value for key, value in extra.items() if value is not None})
    return result


def units(arguments):
    """(name, command, environment changes: None unsets) per unit, in the order to start them."""
    on = json.loads(arguments.on)
    extra = on_environment(arguments.on_env)
    result = []
    seeds = [int(seed) for seed in arguments.seeds.split(",")] if arguments.seeds else SEEDS
    pool = DENSE
    if arguments.board_file:
        pool = [line.strip() for line in Path(arguments.board_file).read_text().splitlines() if line.strip()]
    boards = [b for b in pool if not arguments.boards or b.split("__")[-1] in arguments.boards.split(",")]
    for seed in seeds:
        for board in sorted(boards, key=lambda b: -estimate(b, arguments.estimates)[1]):
            for side in arguments.sides.split(","):
                lever = on if side == "on" else {}
                placer = {**lever, "tuning": {"seed": seed, **lever.get("tuning", {})}}
                name = f"dense-s{seed}-{side}-{board.split('__')[-1]}"
                binary = arguments.on_binary if side == "on" else arguments.off_binary
                command = [sys.executable, str(ROOT / "benchmarks/agent-tasks/run.py"), str(Path(arguments.out) / name),
                           "--tasks", str(TASKS), "--only", board, "--binary", binary, "--jobs", "1",
                           "--threads", "2", "--expected-gb", str(estimate(board, arguments.estimates)[0]),
                           "--estimates", str(arguments.estimates), "--placer", json.dumps(placer)]
                if arguments.reuse:
                    command += ["--reuse", arguments.reuse]
                if getattr(arguments, "quality", False):
                    command += ["--quality"]
                if not arguments.full:
                    command += ["--layout", json.dumps({"stop_after_first_route": True})]
                result.append((name, command, extra if side == "on" else dict.fromkeys(extra)))
    return result


def main():
    parser = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    parser.add_argument("--out", required=True)
    parser.add_argument("--off-binary", required=True)
    parser.add_argument("--on-binary", required=True)
    parser.add_argument("--on", required=True, help="the on side's placer settings, as JSON")
    parser.add_argument("--sides", default="off,on", help="the sides to run: off,on (the default), off or on "
                        "(one off side for several on sides, each in the off side's directory under its own --out)")
    parser.add_argument("--on-env", action="append", metavar="KEY=VALUE",
                        help="an environment variable for the on side only (repeatable)")
    parser.add_argument("--seeds", help="placer seeds, comma-separated (default: 1,101,201,301,401)")
    parser.add_argument("--boards", help="boards by their last name part, comma-separated (default: all eleven)")
    parser.add_argument("--board-file", help="the boards (full task names, one per line) instead of the eleven "
                        "dense boards: a stage 2 scoped to the boards a change acts on")
    parser.add_argument("--full", action="store_true",
                        help="full layouts (stage 2 of the protocol), not stopped after the first route")
    parser.add_argument("--slots", type=int, default=5)
    parser.add_argument("--slots-file", help="a file holding the slots, read again every 20 s (to share the "
                        "machine's layout limit with other runs)")
    parser.add_argument("--estimates", default=str(ESTIMATES),
                        help="the per-board estimates for --expected-gb and the ledger (default %(default)s; "
                             "the nightly's sidecar is build/nightly/estimates.json)")
    parser.add_argument("--reuse", help="run.py's --reuse cache directory (one binary on both sides: rows whose "
                        "placement is the same reuse the earlier layout)")
    parser.add_argument("--quality", action="store_true",
                        help="run.py's --quality (the sweep's designer-vs-ours measures; the layout is the same)")
    parser.add_argument("--dry-run", action="store_true")
    arguments = parser.parse_args()
    out = Path(arguments.out)
    out.mkdir(parents=True, exist_ok=True)
    queue = [u for u in units(arguments) if not (out / f"{u[0]}.done").exists()]
    if arguments.dry_run:
        for name, command, extra in queue:
            changes = " ".join(f"-u {k}" if v is None else f"{k}={v}" for k, v in sorted(extra.items()))
            print(name, changes, " ".join(command))
        return
    running = {}
    while queue or running:
        for name, (process, log) in list(running.items()):
            if process.poll() is not None:
                log.close()
                (out / f"{name}.done").write_text(f"{process.returncode}\n")
                print(time.strftime("%H:%M:%S"), "done", name, process.returncode, flush=True)
                del running[name]
        slots = arguments.slots
        if arguments.slots_file and Path(arguments.slots_file).exists():
            try:
                slots = int(Path(arguments.slots_file).read_text().strip())
            except ValueError:
                pass
        while queue and len(running) < slots:
            name, command, extra = queue.pop(0)
            log = open(out / f"{name}.out", "w")
            env = environment(os.environ, extra) if extra else None
            running[name] = (subprocess.Popen(command, cwd=ROOT, stdout=log, stderr=subprocess.STDOUT, env=env),
                             log)
            print(time.strftime("%H:%M:%S"), "start", name, flush=True)
        time.sleep(20)
    print(time.strftime("%H:%M:%S"), "pool empty", flush=True)


if __name__ == "__main__":
    main()
