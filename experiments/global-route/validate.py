#!/usr/bin/env python3
# Copyright (C) 2026 Toit contributors.
"""The global route as a routability measure, against final results.

Two placements per board of tasks/v7: ours (the layout sweep v18's final
placement, `<sweep>/<task>/layout/placed`, from v18c, else v18b, else
v18) and the designer's (the designer tier's input,
`build/designer-x1315{b,}/<task>/placed`). Each is scored by
`global-route-kicad-board` (the global route, the race's RUDY score of the
same lowering and, with --probe, the race's 75 s router probe); the
scores are then joined with the final results of both placements
(`build/github-layout-v18c/results.json`, `build/designer-x1315b/
results.json`: KiCad's unconnected items and the strict pass).

  experiments/global-route/validate.py run build/global-route-x1440 --binary build/bin/pcb-maker-x1440 --jobs 8
  experiments/global-route/validate.py run build/global-route-x1440 --binary ... --probe 75 --jobs 4
  experiments/global-route/validate.py analyse build/global-route-x1440

Reads only the boards (no code from inside them); run with `python3 -I`.
"""

import argparse
import concurrent.futures
import json
import os
import subprocess
import sys
import time
from pathlib import Path

ROOT = Path(__file__).resolve().parents[2]
TASKS = ROOT / "benchmarks/agent-tasks/tasks/v7/tasks.json"
LAYOUT_DIRS = ["build/github-layout-v18c", "build/github-layout-v18b", "build/github-layout-v18"]
DESIGNER_DIRS = ["build/designer-x1315b", "build/designer-x1315"]
LAYOUT_RESULTS = "build/github-layout-v18c/results.json"
DESIGNER_RESULTS = "build/designer-x1315b/results.json"
# The eleven dense or bus-heavy boards holding 82 % of the placement gap
# (docs/benchmarks.md, "Layout sweep v18").
DENSE = [
    "Huaqiu-Electronics__ecad-viewer__video",
    "stonedDiscord__nonSNES__SNSP-CPU-01",
    "OpenDrone-hw__OpenESC-30x30__4in1",
    "GlasgowEmbedded__glasgow__glasgow__revC3",
    "stonedDiscord__MegaDrive__MegaDrive",
    "0xCB-dev__0xCB-1337__panel",
    "ohdsp__DSP-ADAU1452__DSP-ADAU1452",
    "OpenDrone-hw__OpenESC-20x20__4in1-mini",
    "CRImier__MyKiCad__protoesp",
    "doudar__SmartSpin2k__SmartSpin2k_Panelized",
    "thpoll83__PolyKybd__poly_kybd_split72_left",
]


def tasks():
    return json.loads(TASKS.read_text())["tasks"]


def placements(task):
    """(side, placed directory) for both placements of a task."""
    name, board_id = task["name"], task["board_id"]
    found = []
    for directory in LAYOUT_DIRS:
        placed = ROOT / directory / name / "layout/placed"
        if (placed / f"{board_id}.kicad_pcb").exists():
            found.append(("ours", placed))
            break
    for directory in DESIGNER_DIRS:
        placed = ROOT / directory / name / "placed"
        if (placed / f"{board_id}.kicad_pcb").exists():
            found.append(("designer", placed))
            break
    return found


def score_one(binary, placed, board_id, output, probe, global_json):
    command = [str(binary), "global-route-kicad-board", str(placed), board_id, "auto"]
    if global_json:
        command += ["--global", str(global_json)]
    if probe:
        command += ["--probe", str(probe)]
    env = dict(os.environ, PCB_WORK_RATE="3.15e6,150e6", RAYON_NUM_THREADS="1")
    started = time.monotonic()
    completed = subprocess.run(command, capture_output=True, text=True, env=env, timeout=3600)
    wall = time.monotonic() - started
    if completed.returncode != 0:
        output.with_suffix(".err").write_text(completed.stderr)
        return f"{output.name}: exit {completed.returncode}"
    report = json.loads(completed.stdout.strip().splitlines()[-1])
    report["wall_seconds"] = round(wall, 3)
    output.write_text(json.dumps(report, indent=1))
    return f"{output.name}: {wall:.1f} s"


def run(arguments):
    out = Path(arguments.out)
    out.mkdir(parents=True, exist_ok=True)
    jobs = []
    peaks = {row["name"]: row.get("peak_rss_mb") or 0
             for row in json.loads((ROOT / LAYOUT_RESULTS).read_text())}
    for task in tasks():
        if arguments.only and not any(part in task["name"] for part in arguments.only):
            continue
        # The layout's peak memory sorts out the boards whose router
        # probes need gigabytes (run those with fewer jobs).
        peak = peaks.get(task["name"], 0)
        if arguments.peak_below and peak >= arguments.peak_below:
            continue
        if arguments.peak_from and peak < arguments.peak_from:
            continue
        for side, placed in placements(task):
            suffix = f".probe{arguments.probe:g}" if arguments.probe else ""
            output = out / f"{task['name']}.{side}{suffix}.json"
            if output.exists() and not arguments.force:
                continue
            jobs.append((placed, task["board_id"], output))
    # Large boards first, so the pool does not end on one (probes: small
    # first, memory grows with the board).
    jobs.sort(key=lambda job: (job[0] / f"{job[1]}.kicad_pcb").stat().st_size * (1 if arguments.probe else -1))
    print(f"{len(jobs)} placements to score", file=sys.stderr)
    with concurrent.futures.ThreadPoolExecutor(arguments.jobs) as pool:
        futures = [pool.submit(score_one, arguments.binary, placed, board_id, output, arguments.probe,
                               arguments.global_json) for placed, board_id, output in jobs]
        for future in concurrent.futures.as_completed(futures):
            print(future.result(), file=sys.stderr, flush=True)


# --- analysis ---------------------------------------------------------------

def ranks(values):
    order = sorted(range(len(values)), key=lambda index: values[index])
    result = [0.0] * len(values)
    position = 0
    while position < len(order):
        end = position
        while end + 1 < len(order) and values[order[end + 1]] == values[order[position]]:
            end += 1
        for index in order[position:end + 1]:
            result[index] = (position + end) / 2.0
        position = end + 1
    return result


def spearman(xs, ys):
    if len(xs) < 3:
        return float("nan")
    rx, ry = ranks(xs), ranks(ys)
    mx, my = sum(rx) / len(rx), sum(ry) / len(ry)
    cov = sum((a - mx) * (b - my) for a, b in zip(rx, ry))
    vx = sum((a - mx) ** 2 for a in rx) ** 0.5
    vy = sum((b - my) ** 2 for b in ry) ** 0.5
    return cov / (vx * vy) if vx and vy else float("nan")


def unconnected(row):
    native = row.get("native") or {}
    return native.get("unconnected")


def load(out):
    """Per task name, per side: the score report (and the probe's)."""
    scores = {}
    for path in sorted(Path(out).glob("*.json")):
        parts = path.name[:-len(".json")].split(".")
        if len(parts) < 2:
            continue
        name, side = ".".join(parts[:-1]), parts[-1]
        probe = None
        if side.startswith("probe"):
            name, side, probe = ".".join(parts[:-2]), parts[-2], parts[-1]
        report = json.loads(path.read_text())
        entry = scores.setdefault(name, {}).setdefault(side, {})
        if probe:
            entry["probe"] = report.get("probe")
        else:
            entry.update(report)
    return scores


STATISTICS = {
    "total_overflow": lambda r: r["score"]["total_overflow"],
    "overflow+vias": lambda r: r["score"]["total_overflow"] + r["score"]["via_overflow"],
    "max_overflow": lambda r: r["score"]["max_overflow"],
    "soft_overflow_0.5": lambda r: r["score"]["soft_overflow"][0],
    "soft_overflow_0.75": lambda r: r["score"]["soft_overflow"][1],
    "ace_0.5%": lambda r: r["score"]["ace"][0],
    "ace_2%": lambda r: r["score"]["ace"][1],
    "ace_5%": lambda r: r["score"]["ace"][2],
    "overflowed_nets": lambda r: r["score"]["overflowed_nets"] + r["score"]["unrouted"],
    "detour_fraction": lambda r: r["score"]["detour_fraction"],
    "wirelength_mm": lambda r: r["score"]["wirelength_mm"],
    "rudy": lambda r: r["rudy"],
    "probe_unfinished": lambda r: (r.get("probe") or {}).get("unfinished"),
}


def analyse(arguments):
    scores = load(arguments.out)
    layout = {row["name"]: row for row in json.loads((ROOT / LAYOUT_RESULTS).read_text())}
    designer = {row["name"]: row for row in json.loads((ROOT / DESIGNER_RESULTS).read_text())}
    finals = {"ours": layout, "designer": designer}
    lines = []
    say = lines.append
    # Per statistic: pooled Spearman over all placements with a final
    # result, within-board pair accuracy over boards whose finals differ,
    # and the strict-pass discordant boards.
    say("| statistic | placements | pooled Spearman | pairs (finals differ) | pair accuracy | within-board Spearman | strict-discordant right |")
    say("| --- | ---: | ---: | ---: | ---: | ---: | ---: |")
    for statistic, get in STATISTICS.items():
        xs, ys = [], []
        pairs, right = 0, 0.0
        discordant, discordant_right = 0, 0.0
        for name, sides in scores.items():
            values = {}
            for side in ("ours", "designer"):
                report = sides.get(side)
                row = finals[side].get(name)
                if not report or not row or unconnected(row) is None:
                    continue
                try:
                    value = get(report)
                except (KeyError, TypeError):
                    value = None
                if value is None:
                    continue
                values[side] = (value, unconnected(row), bool(row.get("pass_strict")))
                xs.append(value)
                ys.append(unconnected(row))
            if len(values) < 2:
                continue
            (a, ua, pa), (b, ub, pb) = values["ours"], values["designer"]
            if ua != ub:
                pairs += 1
                if a == b:
                    right += 0.5
                elif (a < b) == (ua < ub):
                    right += 1
            if pa != pb:
                discordant += 1
                if a == b:
                    discordant_right += 0.5
                elif (a < b) == pa:
                    discordant_right += 1
        accuracy = right / pairs if pairs else float("nan")
        say(f"| {statistic} | {len(xs)} | {spearman(xs, ys):.3f} | {pairs} | {accuracy:.3f} | "
            f"{2 * accuracy - 1:.3f} | {discordant_right:g} of {discordant} |")
    # Normalised: open over the board's connections, overflow over nets.
    xs, ys = [], []
    for name, sides in scores.items():
        for side in ("ours", "designer"):
            report, row = sides.get(side), finals[side].get(name)
            if not report or not row or unconnected(row) is None:
                continue
            connections = max(1, report["terminals"] - report["board_nets"])
            xs.append(report["score"]["total_overflow"] / max(1, report["score"]["nets"]))
            ys.append(unconnected(row) / connections)
    say("")
    say(f"Normalised (overflow per net against open per connection), {len(xs)} placements: "
        f"Spearman {spearman(xs, ys):.3f}")
    # The 18 strict-discordant boards and the dense boards, row by row.
    say("")
    say("| board | side passing | open ours / designer | overflow ours / designer | overflowed nets | RUDY | probe | right |")
    say("| --- | --- | --- | --- | --- | --- | --- | --- |")
    for name, sides in sorted(scores.items()):
        rows = {side: finals[side].get(name) for side in ("ours", "designer")}
        if not all(rows.values()) or not all(side in sides for side in rows):
            continue
        pa, pb = bool(rows["ours"].get("pass_strict")), bool(rows["designer"].get("pass_strict"))
        dense = name in DENSE
        if pa == pb and not dense:
            continue
        o, d = sides["ours"], sides["designer"]
        passing = "ours" if pa and not pb else "designer" if pb and not pa else ("both" if pa else "neither")
        verdict = ""
        if pa != pb:
            a, b = o["score"]["total_overflow"], d["score"]["total_overflow"]
            verdict = "tie" if a == b else ("yes" if (a < b) == pa else "no")
        probe = lambda r: (r.get("probe") or {}).get("unfinished", "-")
        say(f"| {name.split('__')[-1]}{' (dense)' if dense else ''} | {passing} | "
            f"{unconnected(rows['ours'])} / {unconnected(rows['designer'])} | "
            f"{o['score']['total_overflow']:.1f} / {d['score']['total_overflow']:.1f} | "
            f"{o['score']['overflowed_nets']} / {d['score']['overflowed_nets']} | "
            f"{o['rudy']:.1f} / {d['rudy']:.1f} | {probe(o)} / {probe(d)} | {verdict} |")
    # Time.
    times = sorted((r.get("global_seconds", 0), r.get("lower_seconds", 0), name, side)
                   for name, sides in scores.items() for side, r in sides.items() if "global_seconds" in r)
    if times:
        values = [t[0] for t in times]
        say("")
        say(f"Global route seconds over {len(values)} placements: median {values[len(values) // 2]:.3f}, "
            f"p90 {values[int(len(values) * 0.9)]:.3f}, max {values[-1]:.3f} ({times[-1][2]} {times[-1][3]}); "
            f"lowering median {sorted(t[1] for t in times)[len(times) // 2]:.3f} s")
        for label in ("katia", "Batch_1", "laptop__motherboard"):
            for t in times:
                if label in t[2]:
                    say(f"  {t[2]} {t[3]}: global {t[0]:.2f} s, lowering {t[1]:.2f} s")
    text = "\n".join(lines)
    print(text)
    if arguments.write:
        Path(arguments.write).write_text(text + "\n")


# --- the historical cohort of final layouts ---------------------------------
#
# build/seed-finals (2026-10-10, x401): 23 held-out boards, the two best and
# two worst of 16 placer seeds by one 75 s probe, each laid out for 1800 s;
# 82 with a KiCad result. The placements as the race probed them are
# reconstructed in build/label-audit/initial-features/<name>/source
# (experiments/congestion/reconstruct_finals.py; nine do not match the
# probed input exactly: BAGEL's four, PixelWave's four, capybully seed 7).
# build/seed-finals-analysis.json holds per placement the final KiCad
# unconnected count and the earlier predictors (finals.py): the probe that
# selected the seeds, independent probes, RUDY, wirelength. This is where
# the probe's 0.55 and RUDY's 0.30 were measured.

FINALS_FEATURES = "build/label-audit/initial-features"
FINALS_ANALYSIS = "build/seed-finals-analysis.json"
FINALS_MISMATCHED = {"ISSUIUC__ISS-PCB__BAGEL-MK1", "hackclub__OnBoard__PixelWave"}


def finals_run(arguments):
    out = Path(arguments.out)
    out.mkdir(parents=True, exist_ok=True)
    jobs = []
    for directory in sorted((ROOT / FINALS_FEATURES).iterdir()):
        source = directory / "source"
        boards = list(source.glob("*.kicad_pcb"))
        if len(boards) != 1:
            continue
        output = out / f"{directory.name}.json"
        if output.exists() and not arguments.force:
            continue
        jobs.append((source, boards[0].name[:-len(".kicad_pcb")], output))
    print(f"{len(jobs)} placements to score", file=sys.stderr)
    with concurrent.futures.ThreadPoolExecutor(arguments.jobs) as pool:
        futures = [pool.submit(score_one, arguments.binary, source, board_id, output, 0,
                               arguments.global_json) for source, board_id, output in jobs]
        for future in concurrent.futures.as_completed(futures):
            print(future.result(), file=sys.stderr, flush=True)


def finals_analyse(arguments):
    rows = json.loads((ROOT / FINALS_ANALYSIS).read_text())
    out = Path(arguments.out)
    for row in rows:
        path = out / f"{row['board']}__seed{row['seed']}.json"
        if not path.exists():
            continue
        report = json.loads(path.read_text())
        for statistic, get in STATISTICS.items():
            if statistic in ("rudy", "probe_unfinished"):
                continue
            row["global:" + statistic] = get(report)
        row["global_seconds"] = report.get("global_seconds")
    lines = []
    say = lines.append
    for cohort, keep in (("all 82", lambda row: True),
                         ("73 matched", lambda row: not (row["board"] in FINALS_MISMATCHED
                                                         or (row["board"].endswith("capybully")
                                                             and row["seed"] == 7)))):
        chosen = [row for row in rows if keep(row)]
        by_board = {}
        for row in chosen:
            by_board.setdefault(row["board"], []).append(row)
        say(f"\nCohort: {cohort} ({len(chosen)} placements, {len(by_board)} boards)\n")
        say("| predictor | within-board Spearman (boards) | pair accuracy (pairs) | regret |")
        say("| --- | ---: | ---: | ---: |")
        predictors = ["probe", "independent", "mean_r12", "rudy_over", "wirelength"] + \
            ["global:" + name for name in STATISTICS if name not in ("rudy", "probe_unfinished")]
        for predictor in predictors:
            rhos, right, pairs, regret = [], 0.0, 0, []
            for board_rows in by_board.values():
                usable = [r for r in board_rows if r.get(predictor) is not None]
                if len(usable) < 2 or len({r["final"] for r in usable}) < 2:
                    continue
                rho = spearman([r[predictor] for r in usable], [r["final"] for r in usable])
                if rho == rho:
                    rhos.append(rho)
                for i in range(len(usable)):
                    for j in range(i + 1, len(usable)):
                        a, b = usable[i], usable[j]
                        if a["final"] == b["final"]:
                            continue
                        pairs += 1
                        if a[predictor] == b[predictor]:
                            right += 0.5
                        else:
                            right += (a[predictor] < b[predictor]) == (a["final"] < b["final"])
                chosen_row = min(usable, key=lambda r: (r[predictor], r["seed"]))
                regret.append(chosen_row["final"] - min(r["final"] for r in usable))
            if not pairs:
                continue
            say(f"| {predictor} | {sum(rhos) / len(rhos):+.3f} ({len(rhos)}) | {right / pairs:.3f} ({pairs}) | "
                f"{sum(regret) / len(regret):.2f} |")
    seconds = sorted(row["global_seconds"] for row in rows if row.get("global_seconds") is not None)
    if seconds:
        say(f"\nGlobal route seconds: median {seconds[len(seconds) // 2]:.3f}, max {seconds[-1]:.3f}")
    text = "\n".join(lines)
    print(text)
    if arguments.write:
        Path(arguments.write).write_text(text + "\n")


def main():
    parser = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    sub = parser.add_subparsers(dest="command", required=True)
    run_parser = sub.add_parser("run")
    run_parser.add_argument("out")
    run_parser.add_argument("--binary", required=True)
    run_parser.add_argument("--jobs", type=int, default=8)
    run_parser.add_argument("--probe", type=float, default=0.0)
    run_parser.add_argument("--global-json", dest="global_json")
    run_parser.add_argument("--only", nargs="*")
    run_parser.add_argument("--force", action="store_true")
    run_parser.add_argument("--peak-below", type=float, default=0.0,
                            help="only boards whose v18 layout peaked below this many MB")
    run_parser.add_argument("--peak-from", type=float, default=0.0,
                            help="only boards whose v18 layout peaked at this many MB or more")
    analyse_parser = sub.add_parser("analyse")
    analyse_parser.add_argument("out")
    analyse_parser.add_argument("--write")
    finals_run_parser = sub.add_parser("finals-run", help="score the historical cohort's placements")
    finals_run_parser.add_argument("out")
    finals_run_parser.add_argument("--binary", required=True)
    finals_run_parser.add_argument("--jobs", type=int, default=8)
    finals_run_parser.add_argument("--global-json", dest="global_json")
    finals_run_parser.add_argument("--force", action="store_true")
    finals_parser = sub.add_parser("finals", help="the historical cohort against its finals")
    finals_parser.add_argument("out")
    finals_parser.add_argument("--write")
    arguments = parser.parse_args()
    {"run": run, "analyse": analyse, "finals-run": finals_run, "finals": finals_analyse}[arguments.command](arguments)


if __name__ == "__main__":
    main()
