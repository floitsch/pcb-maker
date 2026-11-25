#!/usr/bin/env python3
# Copyright (C) 2026 Florian Loitsch.
"""Tables of the noise-source experiments (docs/experiments/noise-sources.md).

    experiments/noise-sources/analyze.py e3 [--out DIR]
    experiments/noise-sources/analyze.py e2 [--out DIR]
    experiments/noise-sources/analyze.py identity [--out DIR]

Reads grid.py's cells (`<out>/<cell>/results.json` and the board's
`layout/` directory). `e3` decomposes each board's first-route open
terminals over placer seeds x router seeds into the placer's, the
router's and their interaction's variance (a two-way random-effects
model without replication: a cell is deterministic, so the interaction
is the residual), and asks how often one router seed picks the placer
seed that is best on the mean of all. `e2` compares each placebo with its
base at the same board, placer seed and random numbers: the parts whose
routed pose differs, the race's probes whose result changed (a
placement's probe is deterministic, so a changed probe is a changed
placement), the first route's open terminals, and the
paired spread against two independent draws."""

import argparse
import itertools
import json
import math
import statistics
import sys
from pathlib import Path

HERE = Path(__file__).resolve().parent
sys.path.insert(0, str(HERE))
import grid  # noqa: E402


def cell_row(out, name):
    """The cell's results row and its board's layout directory, or None."""
    directory = out / name
    results = directory / "results.json"
    if not results.exists():
        return None
    rows = json.loads(results.read_text())
    rows = [row for row in rows if row.get("first_unconnected_terminals") is not None]
    if not rows:
        return None
    row = rows[0]
    return row, directory / row["name"] / "layout"


def first_open(out, name):
    found = cell_row(out, name)
    return None if found is None else found[0]["first_unconnected_terminals"]


def layout_facts(out, name):
    """What a cell's layout chose: the routed placement's poses by
    reference, the placer's kept poses, the race's candidates (score,
    crossings) and the probe picked."""
    found = cell_row(out, name)
    if found is None:
        return None
    row, layout = found
    board_layout = json.loads((layout / "board-layout.json").read_text())

    def poses(directory):
        """Footprint poses of the directory's board by time stamp: the
        layer and the `(at ...)`."""
        boards = sorted(directory.glob("*.kicad_pcb"))
        if not boards:
            return {}
        out, stamp, layer = {}, None, None
        inside = False
        for line in boards[0].read_text(errors="replace").splitlines():
            if line.startswith("  (footprint "):
                inside, stamp, layer = True, None, None
                continue
            if not inside:
                continue
            text = line.strip()
            if text.startswith("(layer ") and layer is None:
                layer = text
            elif text.startswith(("(tstamp ", "(uuid ")) and stamp is None:
                stamp = text.split()[1].rstrip(")")
            elif text.startswith("(at "):
                values = tuple(float(v) for v in text[4:].rstrip(")").split())
                out[stamp or str(len(out))] = (layer,) + values
                inside = False
        return out

    race = board_layout.get("race") or {}
    probes = [(probe.get("placement"), probe.get("open_terminals"), probe.get("ripped_open_terminals"))
              for probe in race.get("probe", [])]
    return {
        "first_open": row["first_unconnected_terminals"],
        "routed": poses(layout / "result"),
        "kept": poses(layout / "placed"),
        "candidates": probes,
        "probed": len(probes),
        "best": race.get("best"),
        "movable": json.loads((layout / "placed" / "board-placer.json").read_text()).get("movable"),
        "digest": board_layout.get("placement_digest"),
        "seconds": row.get("seconds"),
        "cpu_seconds": row.get("cpu_seconds"),
    }


def moved(a, b, tolerance=1.0e-6):
    """Parts whose pose differs between two pose maps (and how many)."""
    keys = sorted(set(a) | set(b))
    differ = [key for key in keys
              if key not in a or key not in b
              or a[key][0] != b[key][0]
              or any(abs(x - y) > tolerance for x, y in zip(a[key][1:], b[key][1:]))]
    return len(differ), len(keys)


def variance_components(table):
    """Two-way random effects without replication on a p x r table:
    (placer, router, interaction) variance components, floored at 0, and
    the mean squares."""
    p, r = len(table), len(table[0])
    grand = statistics.fmean(value for line in table for value in line)
    rows = [statistics.fmean(line) for line in table]
    columns = [statistics.fmean(table[i][j] for i in range(p)) for j in range(r)]
    ss_rows = r * sum((mean - grand) ** 2 for mean in rows)
    ss_columns = p * sum((mean - grand) ** 2 for mean in columns)
    ss_total = sum((value - grand) ** 2 for line in table for value in line)
    ss_residual = ss_total - ss_rows - ss_columns
    ms_rows = ss_rows / (p - 1)
    ms_columns = ss_columns / (r - 1)
    ms_residual = ss_residual / ((p - 1) * (r - 1))
    placer = max(0.0, (ms_rows - ms_residual) / r)
    router = max(0.0, (ms_columns - ms_residual) / p)
    return placer, router, ms_residual, (ms_rows, ms_columns, ms_residual), grand


def spearman(a, b):
    def ranks(values):
        order = sorted(range(len(values)), key=lambda i: values[i])
        out = [0.0] * len(values)
        i = 0
        while i < len(order):
            j = i
            while j + 1 < len(order) and values[order[j + 1]] == values[order[i]]:
                j += 1
            for k in range(i, j + 1):
                out[order[k]] = (i + j) / 2
            i = j + 1
        return out
    ra, rb = ranks(a), ranks(b)
    ma, mb = statistics.fmean(ra), statistics.fmean(rb)
    num = sum((x - ma) * (y - mb) for x, y in zip(ra, rb))
    den = math.sqrt(sum((x - ma) ** 2 for x in ra) * sum((y - mb) ** 2 for y in rb))
    return num / den if den else float("nan")


def e3(out):
    print("## E3: first-route open terminals, placer seed (rows) x router seed (columns)\n")
    shares = []
    agreements = []
    picks = []
    for short in grid.BOARDS:
        table = [[first_open(out, f"e3-{short}-p{p}-r{r}") for r in grid.ROUTER_SEEDS]
                 for p in grid.PLACER_SEEDS]
        if any(value is None for line in table for value in line):
            missing = sum(value is None for line in table for value in line)
            print(f"{short}: {missing} cells missing\n")
            continue
        print(f"### {short}\n")
        print("| placer \\ router | " + " | ".join(str(r) for r in grid.ROUTER_SEEDS) + " | mean |")
        print("|---" * (len(grid.ROUTER_SEEDS) + 2) + "|")
        for p, line in zip(grid.PLACER_SEEDS, table):
            print(f"| {p} | " + " | ".join(str(v) for v in line) + f" | {statistics.fmean(line):.1f} |")
        columns = [statistics.fmean(table[i][j] for i in range(len(table))) for j in range(len(table[0]))]
        print("| mean | " + " | ".join(f"{c:.1f}" for c in columns) + " | |\n")
        # Whether the router seed changed the race's pick: distinct routed
        # placements over the router seeds of one placer seed.
        distinct = []
        for p in grid.PLACER_SEEDS:
            seen = set()
            for r in grid.ROUTER_SEEDS:
                facts = layout_facts(out, f"e3-{short}-p{p}-r{r}")
                seen.add(tuple(sorted(facts["routed"].items())))
            distinct.append(len(seen))
        print(f"distinct routed placements over the router seeds, per placer seed: {distinct}\n")
        placer, router, interaction, ms, grand = variance_components(table)
        total = placer + router + interaction
        shares.append((short, placer / total, router / total, interaction / total, grand, total))
        print(f"variance components: placer {placer:.1f} ({placer / total:.0%}), router {router:.1f} "
              f"({router / total:.0%}), interaction+residual {interaction:.1f} ({interaction / total:.0%}); "
              f"sd of one cell {math.sqrt(total):.1f} on a mean of {grand:.1f}; "
              f"mean squares {ms[0]:.1f} / {ms[1]:.1f} / {ms[2]:.1f}\n")
        # Agreement of two router seeds on the placer seeds' order.
        for j, k in itertools.combinations(range(len(grid.ROUTER_SEEDS)), 2):
            agreements.append(spearman([line[j] for line in table], [line[k] for line in table]))
        means = [statistics.fmean(line) for line in table]
        best = min(range(len(means)), key=lambda i: means[i])
        for j in range(len(grid.ROUTER_SEEDS)):
            column = [line[j] for line in table]
            chosen = min(range(len(column)), key=lambda i: column[i])
            picks.append((short, chosen == best, means[chosen] - means[best]))
    if shares:
        print("### Summary\n")
        print("| board | placer | router | interaction | mean | sd of a cell |")
        print("|---|---|---|---|---|---|")
        for short, a, b, c, grand, total in shares:
            print(f"| {short} | {a:.0%} | {b:.0%} | {c:.0%} | {grand:.1f} | {math.sqrt(total):.1f} |")
        print()
        print(f"router seeds' Spearman agreement on the placer seeds' order: mean "
              f"{statistics.fmean(agreements):+.2f} over {len(agreements)} pairs of router seeds "
              f"(4 placer seeds each)")
        right = sum(1 for _, ok, _ in picks if ok)
        print(f"one router seed picks the placer seed best on the mean: {right} of {len(picks)}; "
              f"mean regret {statistics.fmean(r for _, _, r in picks):.1f} open terminals")


def e2(out):
    print("## E2: placebos against their base (same board, placer seed, random numbers)\n")
    deltas = {}
    print("| board | seed | rng | placebo | routed parts moved | kept parts moved | probes changed | "
          "race pick same | first open base -> placebo |")
    print("|---|---|---|---|---|---|---|---|---|")
    for short in grid.BOARDS:
        for p in grid.PLACER_SEEDS:
            for rng in ("seq", "ctr"):
                base = layout_facts(out, f"e2-{short}-p{p}-{rng}-base")
                if base is None:
                    continue
                for placebo in ("draw", "nudge"):
                    other = layout_facts(out, f"e2-{short}-p{p}-{rng}-{placebo}")
                    if other is None:
                        continue
                    routed, _ = moved(base["routed"], other["routed"])
                    parts = base["movable"]
                    kept, _ = moved(base["kept"], other["kept"])
                    candidates = list(zip(base["candidates"], other["candidates"]))
                    # By result, not by index: the pre-ranking may order
                    # the same placements otherwise.
                    changed = len(candidates) - len(
                        set((a[1], a[2]) for a, _ in candidates) & set((b[1], b[2]) for _, b in candidates))
                    same_pick = (base["probed"], base["best"]) == (other["probed"], other["best"])
                    delta = other["first_open"] - base["first_open"]
                    deltas.setdefault((rng, placebo), []).append((short, p, delta, base["first_open"],
                                                                 routed, parts, changed, len(candidates)))
                    print(f"| {short} | {p} | {rng} | {placebo} | {routed}/{parts} | {kept}/{parts} | "
                          f"{changed}/{len(candidates)} | {'yes' if same_pick else 'no'} | "
                          f"{base['first_open']} -> {other['first_open']} ({delta:+d}) |")
    print()
    noise = e3_noise(out)
    print("| rng | placebo | pairs | identical routed placements | median share of movable parts moved | "
          "probes changed | sd of the first-open change | relative sd | "
          "against two independent draws |")
    print("|---|---|---|---|---|---|---|---|---|")
    for (rng, placebo), rows in sorted(deltas.items()):
        identical = sum(1 for row in rows if row[4] == 0)
        relative = [row[2] / statistics.fmean(noise[row[0]]["means"]) for row in rows if row[0] in noise]
        sd = math.sqrt(statistics.fmean(row[2] ** 2 for row in rows))
        rel_sd = math.sqrt(statistics.fmean(x ** 2 for x in relative)) if relative else float("nan")
        ratio = []
        for row in rows:
            if row[0] in noise:
                ratio.append(row[2] ** 2 / (2 * noise[row[0]]["total"]))
        ratio_text = f"{statistics.fmean(ratio):.2f}" if ratio else "-"
        print(f"| {rng} | {placebo} | {len(rows)} | {identical} | "
              f"{statistics.median(row[4] / row[5] for row in rows):.0%} | "
              f"{sum(row[6] for row in rows)} of {sum(row[7] for row in rows)} | {sd:.1f} | "
              f"{rel_sd:.1%} | {ratio_text} |")
    print("\n(the last column: the mean squared change over twice the board's variance of one cell "
          "from E3; 1 means the pair is two independent draws, 0 a perfect pairing)\n")
    print("Per board, the root mean square of the first-open change: placebo pairs against two "
          "independent placer seeds (the bases of different seeds, the router unset on both) and, "
          "from E3, two router seeds on one placer seed and two placer seeds on one router seed.\n")
    print("| board | seq: independent | seq: draw | seq: nudge | ctr: independent | ctr: draw | ctr: nudge | "
          "E3: other router seed | E3: other placer seed |")
    print("|---|---|---|---|---|---|---|---|---|")

    def rms(values):
        return f"{math.sqrt(statistics.fmean(values)):.1f} ({len(values)})" if values else "-"
    for short in grid.BOARDS:
        cells = []
        for rng in ("seq", "ctr"):
            base = {p: first_open(out, f"e2-{short}-p{p}-{rng}-base") for p in grid.PLACER_SEEDS}
            known = [value for value in base.values() if value is not None]
            cells.append(rms([(x - y) ** 2 for x, y in itertools.combinations(known, 2)]))
            for placebo in ("draw", "nudge"):
                pairs = [(first_open(out, f"e2-{short}-p{p}-{rng}-{placebo}"), base[p]) for p in grid.PLACER_SEEDS]
                cells.append(rms([(x - y) ** 2 for x, y in pairs if x is not None and y is not None]))
        table = [[first_open(out, f"e3-{short}-p{p}-r{r}") for r in grid.ROUTER_SEEDS] for p in grid.PLACER_SEEDS]
        if all(value is not None for line in table for value in line):
            cells.append(rms([(a - b) ** 2 for line in table for a, b in itertools.combinations(line, 2)]))
            cells.append(rms([(table[i][j] - table[k][j]) ** 2 for j in range(len(table[0]))
                              for i, k in itertools.combinations(range(len(table)), 2)]))
        print(f"| {short} | " + " | ".join(cells) + " |")
    print("\n(in brackets the number of pairs)")


def e3_noise(out):
    """Per board the E3 variance of one cell and the placer seeds' means."""
    noise = {}
    for short in grid.BOARDS:
        table = [[first_open(out, f"e3-{short}-p{p}-r{r}") for r in grid.ROUTER_SEEDS]
                 for p in grid.PLACER_SEEDS]
        if any(value is None for line in table for value in line):
            continue
        placer, router, interaction, _, _ = variance_components(table)
        noise[short] = {"total": placer + router + interaction,
                        "means": [statistics.fmean(line) for line in table]}
    return noise


def identity(out):
    a = layout_facts(out, "id-main-protoesp-p1")
    b = layout_facts(out, "id-branch-protoesp-p1")
    if a is None or b is None:
        print("identity: missing")
        return
    print(f"identity: digest main {a['digest']} branch {b['digest']}; first open {a['first_open']} / "
          f"{b['first_open']}; routed parts moved {moved(a['routed'], b['routed'])}")
    for name in ("e3-protoesp-p101-r1002", "e3rep-protoesp-p101-r1002"):
        facts = layout_facts(out, name)
        if facts:
            print(f"{name}: digest {facts['digest']} first open {facts['first_open']}")


def main():
    parser = argparse.ArgumentParser()
    parser.add_argument("what", choices=("e3", "e2", "identity"))
    parser.add_argument("--out", type=Path, default=grid.OUT)
    arguments = parser.parse_args()
    {"e3": e3, "e2": e2, "identity": identity}[arguments.what](arguments.out)


if __name__ == "__main__":
    main()
