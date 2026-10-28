# Copyright (C) 2026 Toit contributors.
#
# Cheaper stages of the placer-default protocol calibrated against two
# experiments with complete full-budget rows (docs/placer.md, "Cheaper
# stages of the placer protocol, calibrated (2026-10-28)"; docs/benchmarks.md,
# "The ladder of placer measurements (2026-10-28)"):
#
# - cross: crossings in the anneal, x1851 both sides (off: defaults; on:
#   `ratsnest_crossings.anneal`), build/cross3-*;
# - race: the race's ripped count, x1859 off / x15190 on with
#   `race_count: "ripped"`, build/race3-*; the off side reuses the on row
#   where the race kept the same placement (only the 19 races whose pick
#   changed ran on the off binary; experiments/race-ripped/picks.py on the
#   race-ripped-count branch).
#
# Stages: full (the task budgets), quarter (`--budget-scale 0.25`, the same
# seeds and binaries, build/proto1-q), first (the first route's open
# terminals of a stage's rows: what a layout stopped after its first route,
# `stop_after_first_route`, reports), and the placement-only metrics of
# each row's placed board (`ablate-kicad-placement`).
#
#   python3 -I experiments/fast-protocol/calibrate.py analyse [--tool build/bin/pcb-maker-x1873] [--rounds 20000]
#   python3 -I experiments/fast-protocol/calibrate.py race-off-plan     # the quarter off rows to run
#   python3 -I experiments/fast-protocol/calibrate.py reuse             # rows a placement cache would reuse
#
# Run from the repository root (build/ is the main checkout's).
import argparse, hashlib, json, math, os, random, shutil, statistics, subprocess, sys
from pathlib import Path

HERE = Path(__file__).resolve().parent
ROOT = HERE.parents[1]
sys.path.insert(0, str(ROOT / "benchmarks" / "agent-tasks"))
import compare  # noqa: E402
import placer_ab  # noqa: E402

BUILD = ROOT / "build"
BOARDS = ["OpenDrone-hw__OpenESC-30x30__4in1", "OpenDrone-hw__OpenESC-20x20__4in1-mini",
          "ohdsp__DSP-ADAU1452__DSP-ADAU1452", "stonedDiscord__MegaDrive__MegaDrive", "0xCB-dev__0xCB-1337__panel",
          "thpoll83__PolyKybd__poly_kybd_split72_left", "CRImier__MyKiCad__protoesp",
          "GlasgowEmbedded__glasgow__glasgow__revC3", "doudar__SmartSpin2k__SmartSpin2k_Panelized",
          "stonedDiscord__nonSNES__SNSP-CPU-01", "Huaqiu-Electronics__ecad-viewer__video"]
SHORT = {b: b.split("__")[-1] for b in BOARDS}
NAMES = {"4in1": "OpenESC 4in1", "4in1-mini": "OpenESC 4in1-mini", "DSP-ADAU1452": "DSP-ADAU1452",
         "MegaDrive": "MegaDrive", "panel": "0xCB panel", "poly_kybd_split72_left": "split72_left",
         "protoesp": "protoesp", "revC3": "Glasgow", "SmartSpin2k_Panelized": "SmartSpin2k",
         "SNSP-CPU-01": "SNSP-CPU-01", "video": "video"}
SEEDS = (1, 101, 201, 301, 401)
SIDES = ("off", "on")
SIGNALS = BUILD / "proto1-signals"
OFF_MARGIN = 0.05  # the unfinished count's margin (RACE_CONFIDENT_MARGIN): off's pick


def name(board):
    return NAMES[SHORT[board]]


# --- the rows -----------------------------------------------------------------

def directories(experiment, stage, side, seed):
    """The run directories of one side and seed, later ones over earlier ones."""
    out = []
    if experiment == "cross" and stage == "full":
        if seed == 1:
            out = [BUILD / f"cross3-dense-s1-{side}"]
        else:
            out = [BUILD / "cross3-u" / f"dense-s{seed}-{side}-{SHORT[b]}" for b in BOARDS]
    elif experiment == "race" and stage == "full":
        on = ([BUILD / f"race3-dense-s{seed}-on"] if seed in (1, 101) else
              [BUILD / "race3-q" / f"dense-s{seed}-on-{SHORT[b]}" for b in BOARDS])
        out = on if side == "on" else on + [BUILD / "race3-q" / f"dense-s{seed}-off-{SHORT[b]}" for b in BOARDS]
    elif stage == "quarter":
        if experiment == "cross":
            out = [BUILD / "proto1-q" / f"cross-q-s{seed}-{side}-{SHORT[b]}" for b in BOARDS]
        else:
            on = [BUILD / "proto1-q" / f"race-q-s{seed}-on-{SHORT[b]}" for b in BOARDS]
            out = on if side == "on" else on + [BUILD / "proto1-q" / f"race-q-s{seed}-off-{SHORT[b]}" for b in BOARDS]
    return [d for d in out if d.is_dir()]


def rows(experiment, stage):
    """{(side, seed, board): record}: the row, its directory, whether the
    off row is the on row (race: the same pick)."""
    table = {}
    for side in SIDES:
        for seed in SEEDS:
            for directory in directories(experiment, stage, side, seed):
                for board, row in placer_ab.rows_of(directory).items():
                    if board not in SHORT or compare.unconnected(row) is None:
                        continue
                    reused = experiment == "race" and side == "off" and "-on" in directory.name
                    table[(side, seed, board)] = {"row": row, "directory": directory, "reused": reused}
    return table


def placed(record, board):
    return placer_ab.placed_board(record["directory"], board)


def sha(path):
    return hashlib.sha256(Path(path).read_bytes()).hexdigest() if path and Path(path).exists() else None


def signals(tool, record, board, tasks):
    """All of ablate-kicad-placement's signals of a row's placed board,
    cached by the board's sha256 (build/proto1-signals)."""
    board_path = placed(record, board)
    if board_path is None:
        return None
    digest = sha(board_path)
    cached = SIGNALS / f"{digest}.json"
    if cached.exists():
        return json.loads(cached.read_text())
    if tool is None:
        return None
    SIGNALS.mkdir(parents=True, exist_ok=True)
    task = tasks[board]
    work = SIGNALS / f"{digest}.d"
    if work.exists():
        shutil.rmtree(work)
    config = SIGNALS / "config.json"
    config.write_text(json.dumps({"groups": ["__none__"], "nulls": 0}))
    designer = Path(task["directory"]) / (task["board_id"] + ".kicad_pcb")
    if not designer.is_absolute():
        designer = ROOT / designer
    done = subprocess.run([str(tool), "ablate-kicad-placement", str(designer), str(board_path.parent),
                           task["board_id"], str(work), str(config)], capture_output=True, text=True)
    report = work / "ablation.json"
    if done.returncode != 0 or not report.exists():
        print(f"{board}: ablate-kicad-placement failed: {done.stderr[-300:]}", file=sys.stderr)
        return None
    ours = {v["name"]: v["signals"] for v in json.loads(report.read_text())["variants"]}["ours"]
    cached.write_text(json.dumps(ours))
    shutil.rmtree(work)
    return ours


PLACEMENT_METRICS = {
    "wirelength": lambda s: s["wirelength_mm"],
    "crossings": lambda s: s["ratsnest_crossings"],
    "objective": lambda s: s["wirelength_mm"] + placer_ab.CROSSING_MM * s["ratsnest_crossings"],
    "blocked_exits": lambda s: s["blocked_exits"],
    "channel_overflow": lambda s: s["channel_overflow"],
    "global_overflow": lambda s: (s.get("global") or {}).get("total_overflow"),
}


def value(record, board, metric, tool=None, tasks=None):
    row = record["row"]
    if metric == "unconnected":
        return compare.unconnected(row)
    if metric == "first_open":
        return row.get("first_unconnected_terminals")
    found = signals(tool, record, board, tasks or {})
    return None if found is None else PLACEMENT_METRICS[metric](found)


def pairs_of(table, metric, tool=None, tasks=None):
    """{board: {seed: (off, on)}} where both sides have a value."""
    out = {}
    for board in BOARDS:
        for seed in SEEDS:
            off, on = table.get(("off", seed, board)), table.get(("on", seed, board))
            if off is None or on is None:
                continue
            a, b = value(off, board, metric, tool, tasks), value(on, board, metric, tool, tasks)
            if a is None or b is None:
                continue
            out.setdefault(board, {})[seed] = (float(a), float(b))
    return out


# --- the statistics -------------------------------------------------------------

def own_noise(table, metric, tool=None, tasks=None):
    """{board: [values]}: a stage's own noise, its off side's five seeds."""
    out = {}
    for board in BOARDS:
        xs = [value(table[("off", s, board)], board, metric, tool, tasks) for s in SEEDS
              if ("off", s, board) in table]
        xs = [float(x) for x in xs if x is not None]
        if len(xs) >= 2:
            out[board] = xs
    return out


def judge(pairs, noise_values, rounds):
    """The protocol's numbers on `pairs`: the weighted relative change, its
    sign-flip p and the noise null's p, the verdict; the sum of medians."""
    reference = placer_ab.weights_of({b: v for b, v in noise_values.items() if b in pairs})
    stats = placer_ab.statistics_of(pairs, reference)
    p_flip = placer_ab.sign_flip(pairs, reference, "weighted", rounds)
    k = min(len(v) for v in pairs.values()) if pairs else 0
    fire = [b for b in pairs if any(p[0] != p[1] for p in pairs[b].values())]
    values = {b: v for b, v in noise_values.items() if b in pairs}
    p_model = None
    if values and fire and k >= 1:
        null = placer_ab.noise_null(values, k, fire=fire)
        p_model = placer_ab.noise_p(null, stats["weighted"])
    verdict = placer_ab.verdict(p_flip, k, p_model=p_model).split(" (")[0]
    medians = stats["sum_medians"]
    return {"weighted": stats["weighted"], "p_flip": p_flip, "p_model": p_model, "verdict": verdict,
            "medians": medians, "k": k, "boards_better": stats["sign"][0], "boards_worse": stats["sign"][1]}


def zscores(xs):
    if len(xs) < 2 or statistics.pstdev(xs) == 0:
        return None
    m, s = statistics.fmean(xs), statistics.pstdev(xs)
    return [(x - m) / s for x in xs]


def correlation(table_x, metric_x, table_y, metric_y, tool=None, tasks=None):
    """Per board the Spearman over its rows (seed x side) between metric_x
    of table_x and metric_y of table_y; the mean of them and the pooled
    Pearson of the within-board ranks' z-scores."""
    per, pooled_x, pooled_y = {}, [], []
    for board in BOARDS:
        xs, ys = [], []
        for side in SIDES:
            for seed in SEEDS:
                key = (side, seed, board)
                if key in table_x and key in table_y:
                    x = value(table_x[key], board, metric_x, tool, tasks)
                    y = value(table_y[key], board, metric_y, tool, tasks)
                    if x is not None and y is not None:
                        xs.append(float(x))
                        ys.append(float(y))
        if len(xs) >= 3:
            per[board] = placer_ab.spearman(xs, ys)
            zx, zy = zscores(placer_ab.ranks(xs)), zscores(placer_ab.ranks(ys))
            if zx and zy:
                pooled_x += zx
                pooled_y += zy
    defined = [r for r in per.values() if r is not None]
    return per, (statistics.fmean(defined) if defined else None), placer_ab.pearson(pooled_x, pooled_y)


def change_agreement(pairs_x, pairs_y, scale):
    """The per-pair changes (on - off, relative to the board's noise mean
    `scale`) of two stages: their Spearman over the pairs both have, the
    share of the pairs the full stage moved whose sign the cheap one gives
    (a pair it leaves unchanged counts as wrong), and the same over the
    boards' mean changes."""
    xs, ys, agree, moved = [], [], 0, 0
    board_agree = board_moved = 0
    for board in BOARDS:
        if board not in pairs_x or board not in pairs_y or not scale.get(board):
            continue
        mx, my = [], []
        for seed in SEEDS:
            if seed in pairs_x[board] and seed in pairs_y[board]:
                dx = (pairs_x[board][seed][1] - pairs_x[board][seed][0]) / scale[board]
                dy = (pairs_y[board][seed][1] - pairs_y[board][seed][0]) / scale[board]
                xs.append(dx)
                ys.append(dy)
                mx.append(dx)
                my.append(dy)
                if dy != 0:
                    moved += 1
                    agree += (dx > 0) == (dy > 0) and dx != 0
        if mx and statistics.fmean(my) != 0:
            board_moved += 1
            board_agree += (statistics.fmean(mx) > 0) == (statistics.fmean(my) > 0) and statistics.fmean(mx) != 0
    return {"spearman": placer_ab.spearman(xs, ys) if len(xs) >= 3 else None, "n": len(xs),
            "sign_agree": agree, "sign_moved": moved, "board_agree": board_agree, "board_moved": board_moved}


def cost(table):
    """CPU and work summed over the rows a stage ran (a reused off row
    counts once)."""
    cpu = work = wall = 0.0
    runs = 0
    for (side, seed, board), record in table.items():
        if record["reused"]:
            continue
        row = record["row"]
        cpu += row.get("cpu_seconds") or 0
        work += row.get("work_seconds") or 0
        wall += row.get("seconds") or 0
        runs += 1
    return {"runs": runs, "cpu_hours": cpu / 3600, "work_hours": work / 3600, "wall_hours": wall / 3600}


def first_route_share(table):
    """The share of a row's wall clock up to the end of its first route
    (board-layout.json: placement_seconds + first_route_seconds over the
    row's seconds), summed over the rows: the first-route stage's cost
    as the full rows' own clocks put it."""
    upto = placing = total = 0.0
    for (side, seed, board), record in table.items():
        if record["reused"]:
            continue
        report = Path(record["directory"]) / board / "layout" / "board-layout.json"
        if not report.exists():
            continue
        r = json.loads(report.read_text())
        placing += r.get("placement_seconds") or 0
        upto += (r.get("placement_seconds") or 0) + (r.get("first_route_seconds") or 0)
        total += record["row"].get("seconds") or 0
    return (upto / total, placing / total) if total else (None, None)


# --- the commands ----------------------------------------------------------------

def fmt(x, digits=2, sign=True):
    if x is None or (isinstance(x, float) and math.isnan(x)):
        return "-"
    return f"{x:+.{digits}f}" if sign else f"{x:.{digits}f}"


def analyse(arguments):
    tasks = placer_ab.tasks_by_name()
    tool = arguments.tool
    noise_file = placer_ab.load_noise(placer_ab.DEFAULT_NOISE)
    out = {}
    for experiment in ("cross", "race"):
        full = rows(experiment, "full")
        quarter = rows(experiment, "quarter")
        print(f"## {experiment}: full {len(full)} rows, quarter {len(quarter)} rows\n")
        stages = {"full": (full, "unconnected"), "full first route": (full, "first_open"),
                  "quarter": (quarter, "unconnected"), "quarter first route": (quarter, "first_open")}
        for metric in PLACEMENT_METRICS:
            stages[f"placement {metric}"] = (full, metric)
            stages[f"quarter placement {metric}"] = (quarter, metric)
        full_pairs = pairs_of(full, "unconnected")
        scale = {b: statistics.fmean(v) for b, v in placer_ab.board_values(noise_file).items()}
        print("| stage | pairs | weighted (file noise) | p flip / file | verdict | weighted (own noise) "
              "| p flip / own | verdict | sum of medians | Spearman rows vs full final (mean / pooled) "
              "| changes: Spearman, pair signs, board signs |")
        print("| --- | ---: | ---: | --- | --- | ---: | --- | --- | --- | --- | --- |")
        result = {}
        for label, (table, metric) in stages.items():
            if not table:
                continue
            pairs = pairs_of(table, metric, tool, tasks)
            if not pairs:
                continue
            file_values = placer_ab.board_values(noise_file, metric) if metric in placer_ab.METRICS else {}
            own = own_noise(table, metric, tool, tasks)
            j_file = judge(pairs, file_values, arguments.rounds) if file_values else None
            j_own = judge(pairs, own, arguments.rounds)
            per, mean_r, pooled_r = correlation(table, metric, full, "unconnected", tool, tasks)
            agreement = change_agreement(pairs, full_pairs, scale) if label != "full" else None
            n = sum(len(v) for v in pairs.values())
            medians = j_own["medians"]
            cells = [label, str(n),
                     "-" if j_file is None else f"{100 * j_file['weighted']:+.1f} %",
                     "-" if j_file is None else f"{j_file['p_flip']:.3f} / {fmt(j_file['p_model'], 3, False)}",
                     "-" if j_file is None else j_file["verdict"],
                     f"{100 * j_own['weighted']:+.1f} %",
                     f"{j_own['p_flip']:.3f} / {fmt(j_own['p_model'], 3, False)}", j_own["verdict"],
                     f"{medians[0]:.0f} -> {medians[1]:.0f}",
                     f"{fmt(mean_r)} / {fmt(pooled_r)}",
                     "-" if agreement is None else
                     f"{fmt(agreement['spearman'])}, {agreement['sign_agree']}/{agreement['sign_moved']}, "
                     f"{agreement['board_agree']}/{agreement['board_moved']}"]
            print("| " + " | ".join(cells) + " |")
            result[label] = {"file": j_file, "own": j_own, "spearman_rows": per and {name(b): r for b, r in per.items()},
                             "spearman_mean": mean_r, "spearman_pooled": pooled_r, "changes": agreement,
                             "pairs": {name(b): v for b, v in pairs.items()}}
        print()
        for label, table in (("full", full), ("quarter", quarter)):
            if table:
                c = cost(table)
                share, placing = first_route_share(table)
                print(f"cost {label}: {c['runs']} layouts, {c['cpu_hours']:.1f} CPU h, {c['work_hours']:.1f} h of "
                      f"work, {c['wall_hours']:.1f} h of wall summed; the placement {fmt(placing, 2, False)} of "
                      f"the rows' wall, up to the end of the first route {fmt(share, 2, False)}")
                result[f"cost {label}"] = dict(c, first_route_share=share, placement_share=placing)
        print()
        # Per board: the full and the quarter stage's KiCad unconnected, off / on.
        if quarter:
            qpairs = pairs_of(quarter, "unconnected")
            print("| Board | " + " | ".join(f"s{s} full / quarter (off; on)" for s in SEEDS) + " | Spearman rows |")
            print("| --- |" + " --- |" * (len(SEEDS) + 1))
            per, _, _ = correlation(quarter, "unconnected", full, "unconnected")
            for board in BOARDS:
                cells = []
                for seed in SEEDS:
                    f = full_pairs.get(board, {}).get(seed)
                    q = qpairs.get(board, {}).get(seed)
                    cells.append("-" if f is None or q is None else
                                 f"{f[0]:.0f} / {q[0]:.0f}; {f[1]:.0f} / {q[1]:.0f}")
                print(f"| {name(board)} | " + " | ".join(cells) + f" | {fmt(per.get(board))} |")
            print()
        out[experiment] = result
    if arguments.json:
        Path(arguments.json).write_text(json.dumps(out, indent=1, default=str))


SCREEN_P = 0.25  # a cheap stage passes a lever on to the full protocol at this noise-file p or below


def bootstrap(arguments):
    """The screen's false-negative risk on these experiments: the five
    seeds resampled with replacement (a resample keeps each seed's eleven
    boards together), each resample judged by the full stage and by each
    cheap stage (the weighted change on the noise file's weights and its
    noise-null p, the null computed once per stage). A cheap stage passes
    a lever on when its weighted change is negative and its p is at most
    SCREEN_P. Printed: how often it passes where the full stage's change
    is negative, where the full stage says BETTER (its noise p at most
    0.03), and how often the signs agree."""
    rng = random.Random(11)
    noise_file = placer_ab.load_noise(placer_ab.DEFAULT_NOISE)
    tasks = placer_ab.tasks_by_name()
    for experiment in ("cross", "race"):
        full = rows(experiment, "full")
        quarter = rows(experiment, "quarter")
        stages = {"full first route": (full, "first_open"), "placement crossings": (full, "crossings"),
                  "placement objective": (full, "objective")}
        if quarter:
            stages.update({"quarter": (quarter, "unconnected"), "quarter first route": (quarter, "first_open")})
        base = pairs_of(full, "unconnected")
        file_values = placer_ab.board_values(noise_file, "unconnected")
        reference = placer_ab.weights_of({b: v for b, v in file_values.items() if b in base})
        fire = [b for b in base if any(p[0] != p[1] for p in base[b].values())]
        null = placer_ab.noise_null({b: v for b, v in file_values.items() if b in base}, 5, fire=fire)
        print(f"## {experiment}: {arguments.resamples} resamples of the five seeds\n")
        print("| stage | full change < 0 | full BETTER | stage passes where the full change < 0 "
              "| where the full says BETTER | signs agree |")
        print("| --- | ---: | ---: | ---: | ---: | ---: |")
        for label, (table, metric) in stages.items():
            cheap = pairs_of(table, metric, arguments.tool, tasks)
            boards = [b for b in BOARDS if b in base and b in cheap]
            values = placer_ab.board_values(noise_file, metric) if metric in placer_ab.METRICS else own_noise(table, metric, arguments.tool, tasks)
            values = {b: v for b, v in values.items() if b in boards}
            ref = placer_ab.weights_of(values)
            cheap_fire = [b for b in boards if any(p[0] != p[1] for p in cheap[b].values())]
            cheap_null = placer_ab.noise_null(values, 5, fire=cheap_fire) if cheap_fire else None
            negative = better = passed_negative = passed_better = agree = 0
            for _ in range(arguments.resamples):
                picks = [rng.choice(SEEDS) for _ in SEEDS]
                def resample(pairs):
                    return {b: {i: pairs[b][s] for i, s in enumerate(picks) if s in pairs[b]} for b in boards}
                f = placer_ab.statistics_of(resample(base), reference)["weighted"]
                c = placer_ab.statistics_of(resample(cheap), ref)["weighted"]
                p_full = placer_ab.noise_p(null, f)
                p_cheap = placer_ab.noise_p(cheap_null, c) if cheap_null else 1.0
                passes = c < 0 and p_cheap <= SCREEN_P
                if f < 0:
                    negative += 1
                    passed_negative += passes
                if p_full <= placer_ab.ALPHA:
                    better += 1
                    passed_better += passes
                agree += (f < 0) == (c < 0)
            n = arguments.resamples
            print(f"| {label} | {negative / n:.2f} | {better / n:.2f} | "
                  f"{passed_negative / negative if negative else float('nan'):.2f} | "
                  f"{passed_better / better if better else float('nan'):.2f} | {agree / n:.2f} |")
        print()


def race_off_plan(arguments):
    """The quarter stage's race rows whose off pick (the unfinished count,
    5 % margin) differs from the on pick: those run on the off binary; plus
    `--identity` boards whose pick is the same (byte-identity checks)."""
    plan, same = [], []
    for seed in SEEDS:
        for board in BOARDS:
            directory = BUILD / "proto1-q" / f"race-q-s{seed}-on-{SHORT[board]}"
            report = directory / board / "layout" / "board-layout.json"
            if not report.exists():
                print(f"not yet: {directory.name}", file=sys.stderr)
                continue
            race = json.loads(report.read_text()).get("race")
            if not race:
                same.append((seed, board))
                continue
            probes = race["probe"]
            unfinished = [p["unfinished"] for p in probes]
            best = min(range(len(unfinished)), key=lambda i: (unfinished[i], i))
            pick = best if best and unfinished[0] and (unfinished[0] - unfinished[best]) / unfinished[0] >= OFF_MARGIN else 0
            if probes[pick]["placement"] != race["kept"]:
                plan.append({"seed": seed, "board": board, "side": "off",
                             "on_kept": race["kept"], "off_kept": probes[pick]["placement"]})
            else:
                same.append((seed, board))
    for seed, board in same[:arguments.identity]:
        plan.append({"seed": seed, "board": board, "side": "off", "identity": True})
    (BUILD / "proto1-q" / "race-off-plan.json").write_text(json.dumps(plan, indent=1))
    print(f"{sum(1 for e in plan if not e.get('identity'))} changed picks, {len(same)} the same; "
          f"{arguments.identity} identity checks; build/proto1-q/race-off-plan.json")


def reuse(arguments):
    """How many rows a placement cache keyed by (placed board, binary,
    task) would have reused: per experiment and stage, the pairs whose two
    sides ran on one binary with byte-identical placed boards, and whether
    their results were identical too."""
    for experiment in ("cross", "race"):
        for stage in ("full", "quarter"):
            table = rows(experiment, stage)
            if not table:
                continue
            same_binary = same_placed = same_result = ran_both = 0
            for board in BOARDS:
                for seed in SEEDS:
                    off, on = table.get(("off", seed, board)), table.get(("on", seed, board))
                    if off is None or on is None:
                        continue
                    if off["reused"]:
                        continue
                    ran_both += 1
                    if off["row"].get("binary_sha256") != on["row"].get("binary_sha256"):
                        continue
                    same_binary += 1
                    if sha(placed(off, board)) == sha(placed(on, board)):
                        same_placed += 1
                        r_off = placer_ab.placed_board(off["directory"], board)
                        res = lambda rec: next(iter(sorted((Path(rec["directory"]) / board / "layout" / "result")
                                                           .glob("*.kicad_pcb"))), None)
                        same_result += sha(res(off)) == sha(res(on))
            reused_by_pick = sum(1 for k, r in table.items() if r["reused"])
            print(f"{experiment} {stage}: {len(table)} rows; pairs run on both sides {ran_both}, on one binary "
                  f"{same_binary}, placed boards identical {same_placed} (results identical {same_result}); "
                  f"off rows taken from the on side by the race's pick {reused_by_pick}")


PAD = __import__("re").compile(r"^Pad \S+ \[.*?\] of (\S+) on ")


def locality_of(directory, board):
    """How local a row's open connections are: KiCad's unconnected items
    of its result (drc.json), each by its midpoint and the parts whose pads
    it names; the share naming the part named most and the three named
    most; the largest share inside a window of a quarter (half the width,
    half the height) and of a ninth of the parts' box, slid in steps of a
    tenth, with the share of the parts inside that quarter window."""
    result = Path(directory) / board / "layout" / "result"
    drc, placer_report = result / "drc.json", result / "board-placer.json"
    if not drc.exists() or not placer_report.exists():
        return None
    items = json.loads(drc.read_text()).get("unconnected_items", [])
    if not items:
        return {"open": 0}
    parts = [f["at"][:2] for f in json.loads(placer_report.read_text()).get("footprints", []) if f.get("at")]
    xs, ys = [p[0] for p in parts], [p[1] for p in parts]
    box = (min(xs), min(ys), max(xs), max(ys))
    width, height = box[2] - box[0], box[3] - box[1]
    points, refs = [], []
    for item in items:
        pos = [i["pos"] for i in item.get("items", []) if "pos" in i]
        if pos:
            points.append((statistics.fmean(p["x"] for p in pos), statistics.fmean(p["y"] for p in pos)))
        refs.append({m.group(1) for i in item.get("items", []) for m in [PAD.match(i.get("description", ""))] if m})
    counts = {}
    for named in refs:
        for ref in named:
            counts[ref] = counts.get(ref, 0) + 1
    top = sorted(counts, key=lambda r: -counts[r])
    def share(names):
        return sum(1 for named in refs if named & set(names)) / len(refs)

    def window(fraction):
        best, at = 0.0, None
        w, h = width * fraction, height * fraction
        for i in range(11):
            for j in range(11):
                x0 = box[0] + (width - w) * i / 10
                y0 = box[1] + (height - h) * j / 10
                inside = sum(1 for x, y in points if x0 <= x <= x0 + w and y0 <= y <= y0 + h)
                if inside > best:
                    best, at = inside, (x0, y0, x0 + w, y0 + h)
        return (best / len(points) if points else 0.0), at
    quarter, at = window(0.5)
    ninth, _ = window(1 / 3)
    parts_inside = sum(1 for x, y in parts if at and at[0] <= x <= at[2] and at[1] <= y <= at[3]) / len(parts)
    return {"open": len(items), "top": top[:3], "top1": share(top[:1]) if top else 0.0,
            "top3": share(top[:3]) if top else 0.0, "quarter": quarter, "ninth": ninth,
            "parts_in_quarter": parts_inside}


def locality(arguments):
    """Per board over the full rows of both experiments (the race's on
    side and its off rows that ran): how local its open connections are
    (locality_of), the mean over the rows, and how often the part named
    most is the same."""
    records = {}
    for experiment in ("cross", "race"):
        for (side, seed, board), record in rows(experiment, "full").items():
            if not record["reused"]:
                records.setdefault(board, []).append(record)
    print("| Board | rows | open (mean) | part named most (rows) | top-1 share | top-3 share | quarter window "
          "| ninth window | parts in the quarter window |")
    print("| --- | ---: | ---: | --- | ---: | ---: | ---: | ---: | ---: |")
    for board in BOARDS:
        found = [locality_of(r["directory"], board) for r in records.get(board, [])]
        found = [f for f in found if f and f["open"]]
        if not found:
            print(f"| {name(board)} | {len(records.get(board, []))} | 0 | - | - | - | - | - | - |")
            continue
        tops = {}
        for f in found:
            tops[f["top"][0]] = tops.get(f["top"][0], 0) + 1
        common = max(tops, key=tops.get)
        mean = lambda key: statistics.fmean(f[key] for f in found)
        print(f"| {name(board)} | {len(found)} | {mean('open'):.0f} | {common} ({tops[common]}) | "
              f"{mean('top1'):.2f} | {mean('top3'):.2f} | {mean('quarter'):.2f} | {mean('ninth'):.2f} | "
              f"{mean('parts_in_quarter'):.2f} |")


def main():
    parser = argparse.ArgumentParser()
    commands = parser.add_subparsers(dest="command", required=True)
    p = commands.add_parser("analyse")
    p.add_argument("--tool", type=Path, default=None)
    p.add_argument("--rounds", type=int, default=20000)
    p.add_argument("--json", type=Path, default=None)
    p = commands.add_parser("race-off-plan")
    p.add_argument("--identity", type=int, default=3)
    commands.add_parser("reuse")
    commands.add_parser("locality")
    p = commands.add_parser("bootstrap")
    p.add_argument("--resamples", type=int, default=2000)
    p.add_argument("--tool", type=Path, default=None)
    arguments = parser.parse_args()
    {"analyse": analyse, "race-off-plan": race_off_plan, "reuse": reuse,
     "locality": locality, "bootstrap": bootstrap}[arguments.command](arguments)


if __name__ == "__main__":
    main()
