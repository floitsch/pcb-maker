#!/usr/bin/env python3
# Copyright (C) 2026 Toit contributors.
"""A placer A/B on the dense boards, judged with a known resolution.

    benchmarks/agent-tasks/placer_ab.py compare --off DIR... --on DIR... [--noise FILE] [--metric M] [--tool BINARY]
    benchmarks/agent-tasks/placer_ab.py resolution [--noise FILE] [--k 3 5 9] [--alpha 0.05] [--power 0.8]
    benchmarks/agent-tasks/placer_ab.py noise DIR... --out FILE [--tool BINARY]
    benchmarks/agent-tasks/placer_ab.py report [--noise FILE]

docs/benchmarks.md, "Measurement discipline: placer defaults". A layout is
deterministic in its binary, tasks, rates and placer settings, but a placer
change moves every board to another draw of its seed noise: on the eleven
dense boards one board's KiCad unconnected count spreads by 13 to 30 (video
by 200) over seeds. A comparison of k seed pairs (`--placer '{"seed": N}'`
on both sides) resolves an effect only when it is larger than that noise
allows; this tool says how large that is and whether a comparison
resolved its effect.

Run directories are run.py's outputs (results.json; a directory without
it is read from `<dir>.out` or `<dir>.log`, the runner's JSON lines). The
seed of a directory is its `-s<N>` (`build/legal1-dense-s201b-off` is seed
201), or `DIR@N`; several directories of one seed and side are merged, a
later one's boards over an earlier one's.

`noise` writes the noise file from runs of one binary at several seeds:
per board and seed KiCad unconnected, the first route's open terminals and,
with `--tool` (a pcb-maker with `ablate-kicad-placement`), the placed
board's unweighted wirelength and ratsnest crossings (the placement
objective is wirelength plus 3 mm per crossing). `report` prints the
noise table and the correlations across seeds between each proxy and the
final unconnected count. `resolution` simulates k-seed comparisons from
the noise file (each board's values by a smoothed bootstrap, the on side
scaled by 1 - delta on every board) and prints, per statistic, the false
positive rate at its threshold and the smallest effect it detects with
the given power. `compare` judges one A/B: the per-board table, every
statistic, the verdict of the recommended one (the weighted relative
change, below) with its confidence (a paired sign-flip randomisation
test) and its interval from the noise file, and the seeds a confirmation
would need.

The statistics (change = on - off; negative is better):
- `sum_medians`: the summed per-board medians (the rule until 2026-10-26),
- `pair_sums`: the mean over the seeds of each pair's summed change,
- `old_rule`: the sum of medians falls and every pair's sum falls,
- `sign`: boards better against worse by their mean change (binomial),
- `weighted`: each board's mean change relative to its noise mean,
  weighted by the inverse square of its coefficient of variation
  (floored at `CV_FLOOR`), as a relative change of the tier."""

import argparse
import hashlib
import json
import math
import random
import re
import shutil
import statistics
import subprocess
import sys
from pathlib import Path

HERE = Path(__file__).resolve().parent
ROOT = HERE.parents[1]
sys.path.insert(0, str(HERE))
import compare  # noqa: E402

DEFAULT_NOISE = HERE / "dense-noise.json"
DEFAULT_TASKS = HERE / "tasks" / "v7" / "tasks.json"
SIGNAL_CACHE = ROOT / "build" / "placer-ab-signals"
CROSSING_MM = 3.0
METRICS = ("unconnected", "first_open", "objective", "crossings")
STATISTICS = ("sum_medians", "pair_sums", "old_rule", "sign", "weighted")
# A board's coefficient of variation is floored here for its weight: a
# board that routed nearly alike on every seed (the 0xCB panel, 50 to 57)
# would otherwise carry the tier alone.
CV_FLOOR = 0.10


# --- reading runs -----------------------------------------------------------

def seed_of(argument):
    """(directory, seed) of `DIR` or `DIR@N`; the seed is the directory
    name's `-s<N>`."""
    text = str(argument)
    if "@" in text:
        directory, _, seed = text.rpartition("@")
        return Path(directory), int(seed)
    match = re.search(r"-s(\d+)", Path(text).name)
    if not match:
        raise SystemExit(f"{text}: no -s<N> in the name; pass {text}@<seed>")
    return Path(text), int(match.group(1))


def json_lines(path):
    rows = []
    if path.exists():
        for line in path.read_text(errors="replace").splitlines():
            if line.startswith("{"):
                try:
                    rows.append(json.loads(line))
                except json.JSONDecodeError:
                    pass
    return [row for row in rows if "name" in row]


def rows_of(directory):
    """{board: row} of a run directory: its results.json, else the JSON
    lines of `<dir>.out` or `<dir>.log` (a run stopped before its end)."""
    directory = Path(directory)
    results = directory / "results.json"
    if results.exists():
        rows = json.loads(results.read_text())
    else:
        rows = json_lines(directory.with_name(directory.name + ".out")) or \
            json_lines(directory.with_name(directory.name + ".log"))
    return {row["name"]: row for row in rows}


def side_of(arguments):
    """{seed: {board: (row, directory)}} of a side's run directories."""
    side = {}
    for argument in arguments:
        directory, seed = seed_of(argument)
        for name, row in rows_of(directory).items():
            side.setdefault(seed, {})[name] = (row, directory)
    return side


def placed_board(directory, name):
    placed = Path(directory) / name / "layout" / "placed"
    boards = sorted(placed.glob("*.kicad_pcb")) if placed.exists() else []
    return boards[0] if boards else None


def tasks_by_name(path=DEFAULT_TASKS):
    return {task["name"]: task for task in json.loads(Path(path).read_text())["tasks"]}


def signals(tool, directory, name, tasks, cache=SIGNAL_CACHE):
    """{"wirelength_mm", "ratsnest_crossings"} of a run's placed board, by
    `ablate-kicad-placement` (no groups), cached by the board's sha256;
    None without a placed board or a tool."""
    board = placed_board(directory, name)
    if board is None or name not in tasks:
        return None
    digest = hashlib.sha256(board.read_bytes()).hexdigest()
    cached = cache / f"{digest}.json"
    if cached.exists():
        return json.loads(cached.read_text())
    if tool is None:
        return None
    cache.mkdir(parents=True, exist_ok=True)
    task = tasks[name]
    work = cache / f"{digest}.d"
    if work.exists():
        shutil.rmtree(work)
    config = cache / "config.json"
    config.write_text(json.dumps({"groups": ["__none__"], "nulls": 0}))
    designer = Path(task["directory"]) / (task["board_id"] + ".kicad_pcb")
    if not designer.is_absolute():
        designer = ROOT / designer
    result = subprocess.run([str(tool), "ablate-kicad-placement", str(designer), str(board.parent),
                             task["board_id"], str(work), str(config)], capture_output=True, text=True)
    report = work / "ablation.json"
    if result.returncode != 0 or not report.exists():
        print(f"{name}: ablate-kicad-placement failed ({result.returncode}): {result.stderr[-300:]}",
              file=sys.stderr)
        return None
    variants = {variant["name"]: variant["signals"] for variant in json.loads(report.read_text())["variants"]}
    keep = {key: variants["ours"][key] for key in ("wirelength_mm", "ratsnest_crossings")}
    cached.write_text(json.dumps(keep))
    shutil.rmtree(work)
    return keep


def metric_of(row, directory, metric, tool=None, tasks=None):
    """A row's value of `metric`, None where it has none."""
    if metric == "unconnected":
        return compare.unconnected(row)
    if metric == "first_open":
        return row.get("first_unconnected_terminals") if compare.unconnected(row) is not None else None
    found = signals(tool, directory, row["name"], tasks or {})
    if found is None:
        return None
    if metric == "crossings":
        return found["ratsnest_crossings"]
    return found["wirelength_mm"] + CROSSING_MM * found["ratsnest_crossings"]


def short(name):
    return name.split("__")[-1]


# --- the noise file ---------------------------------------------------------

def write_noise(arguments, out, tool=None, tasks_path=DEFAULT_TASKS):
    side = side_of(arguments)
    tasks = tasks_by_name(tasks_path)
    seeds = sorted(side)
    boards = sorted({name for rows in side.values() for name in rows})
    binaries = sorted({row.get("binary", "?") for rows in side.values() for row, _ in rows.values()})
    noise = {"binaries": binaries, "seeds": seeds, "sources": [str(argument) for argument in arguments],
             "boards": {}}
    for name in boards:
        entry = {metric: [] for metric in METRICS}
        entry["seeds"] = []
        for seed in seeds:
            if name not in side[seed]:
                continue
            row, directory = side[seed][name]
            values = [metric_of(row, directory, metric, tool, tasks) for metric in METRICS]
            if values[0] is None:
                continue
            entry["seeds"].append(seed)
            for metric, value in zip(METRICS, values):
                entry[metric].append(value)
        noise["boards"][name] = entry
    Path(out).write_text(json.dumps(noise, indent=1) + "\n")
    return noise


def load_noise(path=DEFAULT_NOISE):
    return json.loads(Path(path).read_text())


def board_values(noise, metric="unconnected"):
    """{board: [values]} of the noise file, boards with at least two seeds
    and a value on each."""
    out = {}
    for name, entry in noise["boards"].items():
        values = entry.get(metric) or []
        if len(values) >= 2 and all(value is not None for value in values):
            out[name] = [float(value) for value in values]
    return out


def weights_of(values, cv_floor=CV_FLOOR):
    """{board: (mean, weight)}: the weighted statistic's reference mean and
    weight 1 / max(cv, floor)^2."""
    out = {}
    for name, xs in values.items():
        mean = statistics.fmean(xs)
        sd = statistics.stdev(xs)
        cv = sd / mean if mean > 0 else math.inf
        out[name] = (mean, 1.0 / max(cv, cv_floor) ** 2 if math.isfinite(cv) else 0.0)
    return out


def pearson(xs, ys):
    if len(xs) < 3:
        return None
    mx, my = statistics.fmean(xs), statistics.fmean(ys)
    sxy = sum((x - mx) * (y - my) for x, y in zip(xs, ys))
    sxx = sum((x - mx) ** 2 for x in xs)
    syy = sum((y - my) ** 2 for y in ys)
    return sxy / math.sqrt(sxx * syy) if sxx > 0 and syy > 0 else None


def ranks(xs):
    order = sorted(range(len(xs)), key=lambda i: xs[i])
    out = [0.0] * len(xs)
    i = 0
    while i < len(order):
        j = i
        while j + 1 < len(order) and xs[order[j + 1]] == xs[order[i]]:
            j += 1
        for position in range(i, j + 1):
            out[order[position]] = (i + j) / 2.0
        i = j + 1
    return out


def spearman(xs, ys):
    return pearson(ranks(xs), ranks(ys))


def report(noise):
    """The noise table and the correlations, as text lines."""
    lines = [f"noise: {', '.join(noise['binaries'])}; seeds {', '.join(map(str, noise['seeds']))}", ""]
    lines.append("| Board | seeds | " + " | ".join(f"{m} mean / sd / spread (cv)" for m in METRICS) + " |")
    lines.append("| --- | ---: |" + " ---: |" * len(METRICS))
    pooled = {metric: ([], []) for metric in METRICS[1:]}
    correlations = []
    for name, entry in sorted(noise["boards"].items(), key=lambda item: short(item[0]).lower()):
        cells = []
        for metric in METRICS:
            xs = [x for x in entry.get(metric) or [] if x is not None]
            if len(xs) < 2:
                cells.append("-")
                continue
            mean, sd = statistics.fmean(xs), statistics.stdev(xs)
            cells.append(f"{mean:.0f} / {sd:.1f} / {min(xs):.0f}-{max(xs):.0f} ({sd / mean:.2f})"
                         if mean else f"{mean:.0f} / {sd:.1f}")
        lines.append(f"| {short(name)} | {len(entry['seeds'])} | " + " | ".join(cells) + " |")
        final = entry["unconnected"]
        row = [short(name)]
        for metric in METRICS[1:]:
            xs = entry.get(metric) or []
            if len(xs) == len(final) and all(x is not None for x in xs):
                row.append((pearson(xs, final), spearman(xs, final)))
                if statistics.pstdev(xs) > 0 and statistics.pstdev(final) > 0:
                    zx = [(x - statistics.fmean(xs)) / statistics.pstdev(xs) for x in xs]
                    zy = [(y - statistics.fmean(final)) / statistics.pstdev(final) for y in final]
                    pooled[metric][0].extend(zx)
                    pooled[metric][1].extend(zy)
            else:
                row.append(None)
        correlations.append(row)
    lines += ["", "Correlation across seeds with the final unconnected count (Pearson / Spearman):", "",
              "| Board | " + " | ".join(METRICS[1:]) + " |", "| --- |" + " ---: |" * (len(METRICS) - 1)]
    for row in correlations:
        cells = ["-" if c is None or c[0] is None else f"{c[0]:+.2f} / {c[1]:+.2f}" for c in row[1:]]
        lines.append(f"| {row[0]} | " + " | ".join(cells) + " |")
    cells = []
    for metric in METRICS[1:]:
        zx, zy = pooled[metric]
        r = pearson(zx, zy)
        cells.append("-" if r is None else f"{r:+.2f} ({len(zx)})")
    lines.append("| within boards, pooled | " + " | ".join(cells) + " |")
    return lines


# --- statistics -------------------------------------------------------------

def binomial_tail(n, k):
    """P(X >= k), X ~ Binomial(n, 1/2)."""
    return sum(math.comb(n, i) for i in range(k, n + 1)) / 2 ** n if n else 1.0


def statistics_of(pairs, reference):
    """The statistics of paired data {board: {seed: (off, on)}}.
    `reference` {board: (mean, weight)} weights the relative statistic;
    boards it lacks weigh nothing there."""
    boards = sorted(pairs)
    medians_off = sum(statistics.median([p[0] for p in pairs[b].values()]) for b in boards)
    medians_on = sum(statistics.median([p[1] for p in pairs[b].values()]) for b in boards)
    seeds = sorted(set.intersection(*(set(pairs[b]) for b in boards))) if boards else []
    pair_changes = [sum(pairs[b][s][1] - pairs[b][s][0] for b in boards) for s in seeds]
    pair_offs = [sum(pairs[b][s][0] for b in boards) for s in seeds]
    means = {b: (statistics.fmean([p[0] for p in pairs[b].values()]),
                 statistics.fmean([p[1] for p in pairs[b].values()])) for b in boards}
    change = sum(on - off for off, on in means.values())
    better = sum(1 for off, on in means.values() if on < off)
    worse = sum(1 for off, on in means.values() if on > off)
    numerator = denominator = 0.0
    for b in boards:
        if b in reference and reference[b][0] > 0:
            mean, weight = reference[b]
            numerator += weight * (means[b][1] - means[b][0]) / mean
            denominator += weight
    return {
        "boards": len(boards),
        "seeds": seeds,
        "sum_medians": (medians_off, medians_on),
        "sum_medians_change": medians_on - medians_off,
        "pair_changes": pair_changes,
        "pair_offs": pair_offs,
        "pair_sums_change": statistics.fmean(pair_changes) if pair_changes else 0.0,
        "mean_change": change,
        "mean_off": sum(off for off, _ in means.values()),
        "old_rule": medians_on < medians_off and bool(pair_changes) and all(c < 0 for c in pair_changes),
        "sign": (better, worse, binomial_tail(better + worse, better)),
        "weighted": numerator / denominator if denominator else 0.0,
    }


def sign_flip(pairs, reference, key, rounds=20000, rng=None):
    """One-sided p of statistic `key` (sum_medians_change, pair_sums_change
    or weighted): the share of random swaps of off and on within pairs
    (each board and seed alike likely) whose statistic is at most the
    observed one. Small p: on is better; p near 1: worse."""
    rng = rng or random.Random(1)
    observed = statistics_of(pairs, reference)[key]
    flat = [(b, s, p) for b in pairs for s, p in pairs[b].items()]
    at_most = 0
    for _ in range(rounds):
        swapped = {}
        for b, s, (off, on) in flat:
            swapped.setdefault(b, {})[s] = (on, off) if rng.random() < 0.5 else (off, on)
        if statistics_of(swapped, reference)[key] <= observed + 1e-12:
            at_most += 1
    return (at_most + 1) / (rounds + 1)


def noise_interval(values, k, observed, fire=None, level=0.90, reps=2000, cv_floor=CV_FLOOR):
    """The weighted statistic's interval from the noise file: the observed
    value less the null distribution's quantiles of a k-seed comparison on
    the boards in `fire` (the others alike on both sides). A bootstrap
    over two or three seeds of the comparison itself is far too narrow."""
    null = sorted(row["weighted"] for row in simulate(values, k, [], reps=reps, cv_floor=cv_floor, fire=fire)[0.0])
    lo = null[int((1 - level) / 2 * reps)]
    hi = null[min(reps - 1, int((1 + level) / 2 * reps))]
    return observed - hi, observed - lo


# --- resolution by simulation ------------------------------------------------

def smoothed(xs):
    """A draw from a board's noise values: the smoothed bootstrap that keeps
    their mean and sample variance (a value drawn with replacement, plus a
    normal kernel of Silverman's width, shrunk towards the mean so that the
    variance stays the sample's), at least 0. A plain bootstrap of nine
    values (three for an older tier) gives medians that move only in steps
    of the values themselves."""
    n = len(xs)
    mean = statistics.fmean(xs)
    sample_var = statistics.variance(xs)
    population_var = statistics.pvariance(xs)
    width = 1.06 * math.sqrt(sample_var) * n ** -0.2
    shrink = math.sqrt(sample_var / (population_var + width ** 2)) if population_var + width ** 2 > 0 else 0.0

    def draw(rng):
        x = rng.choice(xs)
        return max(0.0, mean + (x - mean + width * rng.gauss(0.0, 1.0)) * shrink)
    return draw


def simulate(values, k, deltas, reps=2000, seed=7, cv_floor=CV_FLOOR, fire=None):
    """For each delta: the share of `reps` simulated k-seed comparisons each
    statistic calls better, at its threshold from the null (delta 0).
    Each board's off and on values are drawn from its noise values
    (`smoothed`), the on side's times 1 - delta on
    the boards in `fire` (default all); a board outside it gives the
    same layout on both sides, as a switch that does not fire there.
    Returns {delta: [statistics_of of each comparison]}, delta 0 the
    null."""
    rng = random.Random(seed)
    reference = weights_of(values, cv_floor)
    boards = sorted(values)
    fire = set(boards if fire is None else fire)
    draw = {b: smoothed(values[b]) for b in boards}
    base = []
    for _ in range(reps):
        off = {b: [draw[b](rng) for _ in range(k)] for b in boards}
        on = {b: [draw[b](rng) for _ in range(k)] for b in boards}
        base.append((off, on))
    samples = {}
    for delta in [0.0] + [d for d in deltas if d != 0.0]:
        rows = []
        for off, on in base:
            scaled = {b: [x * (1 - delta) for x in on[b]] if b in fire else off[b] for b in boards}
            pairs = {b: {s: (off[b][s], scaled[b][s]) for s in range(k)} for b in boards}
            rows.append(statistics_of(pairs, reference))
        samples[delta] = rows
    return samples


def thresholds(null_rows, alpha):
    """Per continuous statistic the value at or below which a comparison is
    called better at false-positive rate alpha; for `sign` the least
    number of better boards."""
    out = {}
    for key in ("sum_medians_change", "pair_sums_change", "weighted"):
        xs = sorted(row[key] for row in null_rows)
        out[key] = xs[max(0, int(math.floor(alpha * len(xs))) - 1)]
    n = null_rows[0]["boards"]
    out["sign"] = next((t for t in range(n + 1) if binomial_tail(n, t) <= alpha), n + 1)
    return out


def called(row, key, limit):
    if key == "old_rule":
        return row["old_rule"]
    if key == "sign":
        return row["sign"][0] >= limit
    return row[key] <= limit


STAT_KEYS = {"sum_medians": "sum_medians_change", "pair_sums": "pair_sums_change", "old_rule": "old_rule",
             "sign": "sign", "weighted": "weighted"}


def resolution(values, ks, alpha=0.05, power=0.8, reps=2000, grid=None, cv_floor=CV_FLOOR, fire=None):
    """{k: {statistic: (false positive rate, smallest delta with the power or
    None)}}."""
    grid = grid or [i / 200 for i in range(1, 81)]
    out = {}
    for k in ks:
        samples = simulate(values, k, grid, reps=reps, cv_floor=cv_floor, fire=fire)
        limits = thresholds(samples[0.0], alpha)
        result = {}
        for name, key in STAT_KEYS.items():
            limit = limits.get(key)
            false_positive = sum(called(row, key, limit) for row in samples[0.0]) / reps
            smallest = None
            for delta in grid:
                if sum(called(row, key, limit) for row in samples[delta]) / reps >= power:
                    smallest = delta
                    break
            result[name] = (false_positive, smallest)
        out[k] = result
    return out


def seeds_needed(values, delta, alpha=0.05, power=0.8, reps=1000, cv_floor=CV_FLOOR, fire=None, most=40,
                 key="weighted"):
    """The fewest seed pairs at which statistic `key` (weighted,
    pair_sums_change or sum_medians_change) detects a relative improvement
    of `delta` on the boards in `fire` with `power` at `alpha` (None beyond
    `most`)."""
    if delta <= 0:
        return None
    k = 2
    while k <= most:
        samples = simulate(values, k, [delta], reps=reps, cv_floor=cv_floor, fire=fire)
        limit = thresholds(samples[0.0], alpha)[key]
        if sum(row[key] <= limit for row in samples[delta]) / reps >= power:
            return k
        k = k + 1 if k < 10 else k + 2
    return None


# --- the A/B ------------------------------------------------------------------

def paired(off, on, metric="unconnected", tool=None, tasks=None):
    """{board: {seed: (off, on)}} where both sides have the seed and a value,
    and the boards or seeds left out."""
    pairs = {}
    missing = []
    for seed in sorted(set(off) | set(on)):
        names = set(off.get(seed, {})) | set(on.get(seed, {}))
        for name in sorted(names):
            values = []
            for side in (off, on):
                entry = side.get(seed, {}).get(name)
                values.append(None if entry is None else metric_of(entry[0], entry[1], metric, tool, tasks))
            if None in values:
                missing.append((short(name), seed))
                continue
            pairs.setdefault(name, {})[seed] = tuple(float(v) for v in values)
    return pairs, missing


def compare_lines(pairs, missing, noise_values, alpha=0.05, power=0.8, rounds=20000, cv_floor=CV_FLOOR,
                  metric="unconnected"):
    reference = weights_of({b: v for b, v in noise_values.items() if b in pairs}, cv_floor)
    seeds = sorted({s for by_seed in pairs.values() for s in by_seed})
    lines = [f"{metric}, seeds {', '.join(map(str, seeds))}" +
             (f"; left out (board, seed): {missing}" if missing else ""), ""]
    lines.append("| Board | " + " | ".join(f"s{s} off / on" for s in seeds) +
                 " | median off / on | mean change | relative (weight) |")
    lines.append("| --- |" + " ---: |" * (len(seeds) + 3))
    for b in sorted(pairs, key=lambda name: short(name).lower()):
        cells = [f"{pairs[b][s][0]:.0f} / {pairs[b][s][1]:.0f}" if s in pairs[b] else "-" for s in seeds]
        offs = [p[0] for p in pairs[b].values()]
        ons = [p[1] for p in pairs[b].values()]
        change = statistics.fmean(ons) - statistics.fmean(offs)
        if b in reference:
            mean, weight = reference[b]
            relative = f"{100 * change / mean:+.1f} % ({weight:.0f})"
        else:
            relative = "- (no noise)"
        lines.append(f"| {short(b)} | " + " | ".join(cells) +
                     f" | {statistics.median(offs):.0f} / {statistics.median(ons):.0f} | {change:+.1f} | {relative} |")
    s = statistics_of(pairs, reference)
    lines.append("")
    lines.append(f"sum of medians {s['sum_medians'][0]:.0f} -> {s['sum_medians'][1]:.0f} "
                 f"({s['sum_medians_change']:+.0f}, {100 * s['sum_medians_change'] / s['sum_medians'][0]:+.1f} %)")
    lines.append("pair sums " + ", ".join(f"s{seed} {off:.0f} -> {off + c:.0f}"
                                          for seed, off, c in zip(s["seeds"], s["pair_offs"], s["pair_changes"])) +
                 f"; mean change {s['pair_sums_change']:+.1f}")
    lines.append(f"old rule (sum of medians and every pair falls): {'yes' if s['old_rule'] else 'no'}")
    better, worse, tail = s["sign"]
    lines.append(f"sign: {better} boards better, {worse} worse, {s['boards'] - better - worse} equal "
                 f"(one-sided binomial p {tail:.3f})")
    p_medians = sign_flip(pairs, reference, "sum_medians_change", rounds)
    p_sums = sign_flip(pairs, reference, "pair_sums_change", rounds)
    p_weighted = sign_flip(pairs, reference, "weighted", rounds)
    k = min(len(by_seed) for by_seed in pairs.values()) if pairs else 0
    values = {b: v for b, v in noise_values.items() if b in pairs}
    fire = [b for b in pairs if any(p[0] != p[1] for p in pairs[b].values())]
    lines.append(f"sign-flip p (on better): sum of medians {p_medians:.3f}, pair sums {p_sums:.3f}, "
                 f"weighted {p_weighted:.3f}")
    interval = ""
    if values and fire and k >= 1:
        lo, hi = noise_interval(values, k, s["weighted"], fire=fire)
        interval = f" (90 % interval from the noise file {100 * lo:+.1f} to {100 * hi:+.1f} %)"
    lines.append(f"weighted relative change {100 * s['weighted']:+.1f} %{interval}")
    if p_weighted <= alpha:
        verdict = f"BETTER at {100 * (1 - p_weighted):.1f} % confidence"
    elif 1 - p_weighted <= alpha:
        verdict = f"WORSE at {100 * p_weighted:.1f} % confidence"
    else:
        verdict = (f"NOT RESOLVED (confidence on better {100 * (1 - p_weighted):.1f} %, "
                   f"the rule asks {100 * (1 - alpha):.0f} %)")
    lines.append("")
    lines.append(f"verdict ({k} seed pairs, weighted, one-sided alpha {alpha}): {verdict}")
    if values and fire and s["weighted"] != 0:
        # The observed change as a relative effect on the boards it moved:
        # the weighted statistic over those boards alone.
        moved = statistics_of({b: pairs[b] for b in fire}, reference)["weighted"]
        needed = seeds_needed(values, abs(moved), alpha, power, fire=fire)
        lines.append(f"seeds to resolve a change of {100 * abs(moved):.1f} % on the {len(fire)} boards it moved "
                     f"(power {power}, alpha {alpha}, weighted): " + (str(needed) if needed else "more than 40"))
    return lines


def main():
    parser = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    commands = parser.add_subparsers(dest="command", required=True)
    p = commands.add_parser("compare")
    p.add_argument("--off", nargs="+", required=True)
    p.add_argument("--on", nargs="+", required=True)
    p.add_argument("--noise", type=Path, default=DEFAULT_NOISE)
    p.add_argument("--metric", choices=METRICS, default="unconnected")
    p.add_argument("--tool", type=Path, help="pcb-maker with ablate-kicad-placement (objective, crossings)")
    p.add_argument("--alpha", type=float, default=0.05)
    p.add_argument("--power", type=float, default=0.8)
    p.add_argument("--rounds", type=int, default=20000)
    p = commands.add_parser("resolution")
    p.add_argument("--noise", type=Path, default=DEFAULT_NOISE)
    p.add_argument("--metric", choices=METRICS, default="unconnected")
    p.add_argument("--k", type=int, nargs="+", default=[3, 5, 9])
    p.add_argument("--alpha", type=float, default=0.05)
    p.add_argument("--power", type=float, default=0.8)
    p.add_argument("--reps", type=int, default=2000)
    p.add_argument("--cv-floor", type=float, default=CV_FLOOR)
    p = commands.add_parser("noise")
    p.add_argument("runs", nargs="+")
    p.add_argument("--out", type=Path, required=True)
    p.add_argument("--tool", type=Path)
    p = commands.add_parser("report")
    p.add_argument("--noise", type=Path, default=DEFAULT_NOISE)
    arguments = parser.parse_args()

    if arguments.command == "noise":
        write_noise(arguments.runs, arguments.out, arguments.tool)
        print("\n".join(report(load_noise(arguments.out))))
    elif arguments.command == "report":
        print("\n".join(report(load_noise(arguments.noise))))
    elif arguments.command == "resolution":
        values = board_values(load_noise(arguments.noise), arguments.metric)
        table = resolution(values, arguments.k, arguments.alpha, arguments.power, arguments.reps,
                           cv_floor=arguments.cv_floor)
        print(f"{arguments.metric}: smallest relative improvement on every board detected with power "
              f"{arguments.power} at one-sided alpha {arguments.alpha} (false positive rate at the threshold); "
              f"{arguments.reps} simulated comparisons, {len(values)} boards")
        print("| k | " + " | ".join(STAT_KEYS) + " |")
        print("| ---: |" + " ---: |" * len(STAT_KEYS))
        for k, result in table.items():
            cells = [f"{'-' if d is None else f'{100 * d:.1f} %'} ({fp:.2f})" for fp, d in result.values()]
            print(f"| {k} | " + " | ".join(cells) + " |")
    else:
        tasks = tasks_by_name()
        pairs, missing = paired(side_of(arguments.off), side_of(arguments.on), arguments.metric,
                                arguments.tool, tasks)
        values = board_values(load_noise(arguments.noise), arguments.metric)
        print("\n".join(compare_lines(pairs, missing, values, arguments.alpha, arguments.power,
                                      arguments.rounds, metric=arguments.metric)))


if __name__ == "__main__":
    main()
