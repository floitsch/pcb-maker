# Copyright (C) 2026 Toit contributors.
#
# A coarse whole first route as a placement ranker (docs/placer.md, "A
# coarse whole first route as the race probe (2026-10-27)"): the trouble
# study's 75 placements (build/trouble1-study/rows.json: the eleven dense
# boards over the layout seeds of x1828, their final KiCad unconnected, the
# race's 75 s probe as the runs recorded it and the first route's open
# terminals) are routed whole by `pcb-maker probe-coarse-placement` at a
# reduced fidelity (pitch factor, iteration cap, repairs), at two budgets,
# and each setting's ranking of the seeds is compared with the finals as
# the trouble study compared its probes: Spearman, top-1, regret per board
# and summed, a race of k placements, and the cost.
#
#   python3 -I experiments/coarse-probe/study.py run <out-dir> --binary build/bin/pcb-maker-xNNNN [--jobs 4] [--variants a,b]
#   python3 -I experiments/coarse-probe/study.py analyse <out-dir>
#
# Run from the repository root. Probes run 4 at a time, each started only
# while the machine has 10 GB available beyond the probe's room (and
# through the memory ledger, which the command arms itself); every report is
# cached in <out-dir>/<board>/<seed>/<variant>.json and not run again. A
# 10 % probe whose 5 % probe ended before its budget (a stall, the
# iteration cap, no conflicts) is that probe and is copied, not run.
import argparse, itertools, json, os, statistics, subprocess, sys, time

ROWS = 'build/trouble1-study/rows.json'
SHORT = {'OpenDrone-hw__OpenESC-20x20__4in1-mini': 'OpenESC 4in1-mini', 'OpenDrone-hw__OpenESC-30x30__4in1': 'OpenESC 4in1',
         'ohdsp__DSP-ADAU1452__DSP-ADAU1452': 'DSP-ADAU1452', 'thpoll83__PolyKybd__poly_kybd_split72_left': 'split72_left',
         'Huaqiu-Electronics__ecad-viewer__video': 'video', 'GlasgowEmbedded__glasgow__glasgow__revC3': 'Glasgow',
         'doudar__SmartSpin2k__SmartSpin2k_Panelized': 'SmartSpin2k', '0xCB-dev__0xCB-1337__panel': '0xCB panel',
         'CRImier__MyKiCad__protoesp': 'protoesp', 'stonedDiscord__MegaDrive__MegaDrive': 'MegaDrive',
         'stonedDiscord__nonSNES__SNSP-CPU-01': 'SNSP-CPU-01'}
BOARDS = list(SHORT)
TOTAL = 1500.0
# The settings (fixed before any run): name -> arguments.
SETTINGS = {
    'p2-i25': ['--pitch-factor', '2', '--iterations', '25'],
    'p1.5-i40': ['--pitch-factor', '1.5', '--iterations', '40'],
    'p1-i25': ['--pitch-factor', '1', '--iterations', '25'],
}
SHARES = {'5': 0.05, '10': 0.10}
# Follow-ups (chosen after the 5 % rows: the rip-up count ranks at every
# pitch, the cost is the question): the 2 % budget (30 s; a probe whose
# 5 % probe ended before 30 s of search is that probe and is copied), and
# the hard repair (half the budget negotiating).
SHORTER = {'p2-i25-2': 'p2-i25-5', 'p1-i25-2': 'p1-i25-5'}
EXTRA = {
    'p2-i25-2': ['--pitch-factor', '2', '--iterations', '25', '--share', '0.02'],
    'p1-i25-2': ['--pitch-factor', '1', '--iterations', '25', '--share', '0.02'],
    'p2-i25-hard-10': ['--pitch-factor', '2', '--iterations', '25', '--hard-repair', '--share', '0.1'],
    'p1-i25-hard-10': ['--pitch-factor', '1', '--iterations', '25', '--hard-repair', '--share', '0.1'],
}
FLOOR_MB = 10240


def variants():
    result = []
    for share_name, share in SHARES.items():
        for name, args in SETTINGS.items():
            result.append(('%s-%s' % (name, share_name), args + ['--share', str(share)]))
    result += list(EXTRA.items())
    return result


def load_rows():
    data = json.load(open(ROWS))
    return {(key.split('|')[0], int(key.split('|')[1])): value for key, value in data.items()}


def mem_available_mb():
    for line in open('/proc/meminfo'):
        if line.startswith('MemAvailable:'):
            return int(line.split()[1]) / 1024
    return 0


def run_one(binary, placed, arguments, target):
    # Work alone ends a probe: no wall-clock guard.
    env = dict(os.environ, PCB_WORK_RATE='3.15e6,150e6', RAYON_NUM_THREADS='1', PCB_ROUTER_NO_WALL_GUARD='1')
    command = [binary, 'probe-coarse-placement', placed, 'auto', '--total-seconds', str(TOTAL),
               '--out', target + '.tmp'] + arguments
    process = subprocess.Popen(command, env=env, stdout=subprocess.DEVNULL, stderr=open(target + '.err', 'w'))
    return process, time.time(), target


def schedule(jobs, binary, parallel):
    running = []
    jobs = list(jobs)
    while jobs or running:
        while jobs and len(running) < parallel and mem_available_mb() > FLOOR_MB + 2048:
            placed, arguments, target = jobs.pop(0)
            os.makedirs(os.path.dirname(target), exist_ok=True)
            running.append(run_one(binary, placed, arguments, target))
        time.sleep(2)
        still = []
        for process, started, target in running:
            pid, status, usage = os.wait4(process.pid, os.WNOHANG)
            if pid == 0:
                still.append((process, started, target))
                continue
            code = os.waitstatus_to_exitcode(status)
            if code != 0 or not os.path.exists(target + '.tmp'):
                print('failed:', target, open(target + '.err').read()[-400:], file=sys.stderr, flush=True)
                continue
            report = json.load(open(target + '.tmp'))
            report['outer'] = dict(wall=time.time() - started, cpu=usage.ru_utime + usage.ru_stime,
                                   peak_mb=usage.ru_maxrss / 1024)
            json.dump(report, open(target, 'w'), indent=1)
            os.remove(target + '.tmp')
            print('done', target, 'wall %.0f' % report['outer']['wall'], 'work %.1f' % report['work_seconds'],
                  'left', len(jobs), flush=True)
        running = still


def target_of(out, name, seed, variant):
    return os.path.join(out, name, str(seed), variant + '.json')


def ended_early(report):
    """A probe that ended before its budget (the 10 % probe is the same)."""
    return (report['stop'] not in ('probe_work_limit', 'negotiation_work_limit')
            and report['search_work_seconds'] < 0.98 * report['budget_seconds'])


def command_run(arguments):
    data = load_rows()
    out = arguments.out
    os.makedirs(out, exist_ok=True)
    wanted = [v for v in variants() if not arguments.variants or v[0] in arguments.variants.split(',')]
    for phase in ('5', '10', 'extra'):
        jobs = []
        for variant, args in wanted:
            kind = 'extra' if variant in EXTRA else variant.rsplit('-', 1)[1]
            if kind != phase:
                continue
            for name in BOARDS:
                for seed in sorted(s for (n, s) in data if n == name):
                    path = target_of(out, name, seed, variant)
                    if os.path.exists(path):
                        continue
                    if variant in SHORTER:
                        five = target_of(out, name, seed, SHORTER[variant])
                        if not os.path.exists(five):
                            continue
                        report = json.load(open(five))
                        if ended_early(report) and report['search_work_seconds'] < 0.98 * 0.02 * TOTAL:
                            json.dump(dict(report, budget_seconds=0.02 * TOTAL, copied_from=five), open(path, 'w'), indent=1)
                            continue
                    if phase == '10':
                        five = target_of(out, name, seed, variant[:-len('10')] + '5')
                        if not os.path.exists(five):
                            continue
                        report = json.load(open(five))
                        if ended_early(report):
                            json.dump(dict(report, budget_seconds=0.10 * TOTAL, copied_from=five), open(path, 'w'), indent=1)
                            continue
                    jobs.append((data[(name, seed)]['placed'], args, path))
        print(phase, len(jobs), 'probes', flush=True)
        schedule(jobs, arguments.binary, arguments.jobs)
    print('done', flush=True)


def spearman(xs, ys):
    def ranks(values):
        order = sorted(range(len(values)), key=lambda i: values[i])
        result = [0.0] * len(values)
        i = 0
        while i < len(order):
            j = i
            while j + 1 < len(order) and values[order[j + 1]] == values[order[i]]:
                j += 1
            for k in range(i, j + 1):
                result[order[k]] = (i + j) / 2.0
            i = j + 1
        return result
    if len(xs) < 3:
        return None
    rx, ry = ranks(xs), ranks(ys)
    mx, my = statistics.mean(rx), statistics.mean(ry)
    sx = sum((a - mx) ** 2 for a in rx) ** 0.5
    sy = sum((b - my) ** 2 for b in ry) ** 0.5
    if sx == 0 or sy == 0:
        return None
    return sum((a - mx) * (b - my) for a, b in zip(rx, ry)) / (sx * sy)


def evaluate(scores, finals):
    """As the trouble study: Spearman of the score's ranks (equal scores
    share the lower rank) against the finals, the top-1 hit (ties to the
    lower seed) and the regret."""
    seeds = sorted(s for s in scores if scores[s] is not None)
    if len(seeds) < 3:
        return None
    values = [scores[s] for s in seeds]
    order = sorted(range(len(seeds)), key=lambda i: (values[i], seeds[i]))
    rank = [0] * len(seeds)
    for r, i in enumerate(order):
        rank[i] = r
    for i in range(len(seeds)):
        for j in range(len(seeds)):
            if values[i] == values[j]:
                rank[i] = min(rank[i], rank[j])
    values_finals = [finals[s] for s in seeds]
    rho = spearman(rank, values_finals)
    pick = seeds[order[0]]
    best = min(values_finals)
    return dict(rho=rho, hit=finals[pick] == best, regret=finals[pick] - best, n=len(seeds), pick=pick)


def race(scores, finals, seeds, k):
    """The mean final of the placement the score picks among every subset
    of k of `seeds` (k capped at their number; ties to the lower seed)."""
    k = min(k, len(seeds))
    picks = [finals[min(subset, key=lambda s: (scores[s], s))] for subset in itertools.combinations(seeds, k)]
    return statistics.mean(picks)


def command_analyse(arguments):
    out = arguments.out
    data = load_rows()
    finals_of = {name: {s: data[(n, s)]['unc'] for (n, s) in data if n == name} for name in BOARDS}

    def report(name, seed, variant):
        path = target_of(out, name, seed, variant)
        return json.load(open(path)) if os.path.exists(path) else None

    rankers = {'75 s probe (race)': lambda n, s: data[(n, s)]['probe'],
               'first-route open': lambda n, s: data[(n, s)]['first']}
    names = [v for v, _ in variants()]
    # The ranked measure fixed before the runs: the probe's open terminals
    # (after the rip-up, or the repair), ties by the standing at the
    # negotiation's end. Post hoc beside it: the standing at the
    # negotiation's end alone, and the nets unfinished.
    for variant in names:
        def primary(n, s, variant=variant):
            r = report(n, s, variant)
            return (r['open_terminals'], r['negotiated_open_terminals'], r['unfinished']) if r else None

        def standing(n, s, variant=variant):
            r = report(n, s, variant)
            return (r['negotiated_open_terminals'], r['unfinished'], r['conflicted']) if r else None

        def unfinished(n, s, variant=variant):
            r = report(n, s, variant)
            return r['unfinished'] if r else None
        rankers[variant] = primary
        rankers[variant + ' (standing)'] = standing
        rankers[variant + ' (unfinished)'] = unfinished
    complete = lambda label: all(rankers[label](n, s) is not None for (n, s) in data
                                 if not label.startswith('75 s'))
    lines = []

    def table(title, seeds_of):
        lines.append('')
        lines.append('### ' + title)
        lines.append('')
        lines.append('| ranker | ' + ' | '.join(SHORT[b] for b in BOARDS) + ' | mean | top-1 | regret | placements |')
        lines.append('|---|' + '---:|' * (len(BOARDS) + 4))
        result = {}
        for label, scorer in rankers.items():
            cells = []
            count = 0
            for name in BOARDS:
                seeds = seeds_of(name)
                scores = {s: scorer(name, s) for s in seeds}
                count += sum(1 for v in scores.values() if v is not None)
                cells.append(evaluate(scores, finals_of[name]))
            if not any(cells):
                continue
            rhos = [c['rho'] if c['rho'] is not None else 0.0 for c in cells if c]
            hits = sum(1 for c in cells if c and c['hit'])
            regret = sum(c['regret'] for c in cells if c)
            text = [('%+.2f' % c['rho'] if c['rho'] is not None else 'tie') if c else '-' for c in cells]
            lines.append('| %s | %s | %+.2f | %d/%d | %d | %d |' % (label, ' | '.join(text), statistics.mean(rhos),
                                                                 hits, sum(1 for c in cells if c), regret, count))
            result[label] = dict(cells=cells, mean=statistics.mean(rhos), hits=hits, regret=regret)
        return result

    all_seeds = lambda name: sorted(s for (n, s) in data if n == name)
    common = lambda name: sorted(s for (n, s) in data if n == name and data[(n, s)]['probe'] is not None)
    summary = {'all': table('All seeds (the 75 s probe on the 73 it has)', all_seeds),
               'common': table('The 73 placements with a race probe', common)}
    # The race of k on the common seeds.
    lines.append('')
    lines.append('### A race of k placements (the 73 with a race probe; mean final of the pick, summed over the boards)')
    lines.append('')
    ks = (2, 3, 4, 5, 6)
    lines.append('| ranker | ' + ' | '.join('k = %d' % k for k in ks) + ' |')
    lines.append('|---|' + '---:|' * len(ks))
    random_row = []
    best_row = 0
    for k in ks:
        total = 0
        for name in BOARDS:
            seeds = common(name)
            kk = min(k, len(seeds))
            total += statistics.mean(statistics.mean(finals_of[name][s] for s in subset)
                                     for subset in itertools.combinations(seeds, kk))
        random_row.append(total)
    best_row = sum(min(finals_of[name][s] for s in common(name)) for name in BOARDS)
    lines.append('| random | ' + ' | '.join('%.0f' % v for v in random_row) + ' |')
    races = {}
    for label, scorer in rankers.items():
        row = []
        for k in ks:
            total = 0
            ok = True
            for name in BOARDS:
                seeds = common(name)
                scores = {s: scorer(name, s) for s in seeds}
                if any(v is None for v in scores.values()):
                    ok = False
                    break
                total += race(scores, finals_of[name], seeds, k)
            row.append(total if ok else None)
        if row[0] is None:
            continue
        races[label] = row
        lines.append('| %s | %s |' % (label, ' | '.join('%.0f' % v for v in row)))
    lines.append('| best of all | %d |' % best_row)
    # The cost per board: median over the seeds.
    lines.append('')
    lines.append('### Cost (median over the seeds; work s at the pinned rates, the command\'s wall s under load)')
    lines.append('')
    header = ['race 75 s probe work'] + ['%s %s' % (v, m) for v in names for m in ('work', 'wall')]
    lines.append('| board | first-route wall | ' + ' | '.join(header) + ' |')
    lines.append('|---|' + '---:|' * (len(header) + 1))
    sums = [0.0] * (len(header) + 1)
    for name in BOARDS:
        seeds = all_seeds(name)
        cells = []
        firsts = []
        probes = []
        for s in seeds:
            layout = os.path.join(os.path.dirname(data[(name, s)]['placed']), '..', 'board-layout.json')
            try:
                lay = json.load(open(layout))
                firsts.append(lay['first_route_seconds'])
                race_report = lay.get('race') or {}
                kept = [p for p in race_report.get('probe', []) if p['placement'] == race_report.get('kept')]
                if kept:
                    probes.append(kept[0]['work_seconds'])
            except Exception:
                pass
        cells.append(statistics.median(firsts) if firsts else None)
        cells.append(statistics.median(probes) if probes else None)
        for variant in names:
            reports = [report(name, s, variant) for s in seeds]
            reports = [r for r in reports if r]
            # A copied report (the 10 % probe that is the 5 % one) costs its
            # probe's work and wall.
            cells.append(statistics.median(r['work_seconds'] for r in reports) if reports else None)
            # The command's own wall clock (the outer one holds the wait for
            # the memory ledger).
            cells.append(statistics.median(r['wall_seconds'] for r in reports) if reports else None)
        for i, c in enumerate(cells):
            if c is not None:
                sums[i] += c
        lines.append('| %s | %s |' % (SHORT[name], ' | '.join('%.0f' % c if c is not None else '-' for c in cells)))
    lines.append('| sum | %s |' % ' | '.join('%.0f' % c for c in sums))
    # Lattice and stop per variant.
    lines.append('')
    lines.append('### Stops (count over the placements)')
    lines.append('')
    for variant in names:
        stops = {}
        for (n, s) in data:
            r = report(n, s, variant)
            if r:
                stops[r['stop']] = stops.get(r['stop'], 0) + 1
        lines.append('- %s: %s' % (variant, ', '.join('%s %d' % kv for kv in sorted(stops.items()))))
    text = '\n'.join(lines)
    print(text)
    open(os.path.join(out, 'analysis.md'), 'w').write(text + '\n')
    json.dump(dict(summary={k: {label: dict(mean=v['mean'], hits=v['hits'], regret=v['regret']) for label, v in t.items()}
                            for k, t in summary.items()}, races=races), open(os.path.join(out, 'analysis.json'), 'w'), indent=1)


def main():
    parser = argparse.ArgumentParser()
    sub = parser.add_subparsers(dest='command', required=True)
    run = sub.add_parser('run')
    run.add_argument('out')
    run.add_argument('--binary', required=True)
    run.add_argument('--jobs', type=int, default=4)
    run.add_argument('--variants', default='')
    analyse = sub.add_parser('analyse')
    analyse.add_argument('out')
    arguments = parser.parse_args()
    {'run': command_run, 'analyse': command_analyse}[arguments.command](arguments)


if __name__ == '__main__':
    main()
