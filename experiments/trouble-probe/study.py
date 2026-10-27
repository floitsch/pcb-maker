# Copyright (C) 2026 Toit contributors.
#
# The trouble-maker probe as a placement ranker (docs/placer.md, "The
# trouble-maker probe (2026-10-26)"): every placement of the eleven dense
# boards over the layout seeds of one binary family is probed with
# `pcb-maker probe-trouble-placement` (each background mode, two budgets,
# two trouble sets), and the probe's ranking of the seeds is compared with
# the final KiCad unconnected, beside the race's 75 s probe (as the runs
# recorded it) and the first route's open terminals.
#
#   python3 -I experiments/trouble-probe/study.py run <out-dir> --binary build/bin/pcb-maker-xNNNN [--jobs 4]
#   python3 -I experiments/trouble-probe/study.py analyse <out-dir>
#
# Run from the repository root. Probes run 4 at a time, each started only
# while the machine has 10 GB available (and through the memory ledger,
# which the command arms itself); every probe's report is cached in
# <out-dir>/<board>/<seed>/<variant>.json and not run again.
import argparse, json, os, subprocess, sys, time, itertools, statistics

B = 'build/'
SHORT = {'OpenDrone-hw__OpenESC-20x20__4in1-mini': 'OpenESC 4in1-mini', 'OpenDrone-hw__OpenESC-30x30__4in1': 'OpenESC 4in1',
         'ohdsp__DSP-ADAU1452__DSP-ADAU1452': 'DSP-ADAU1452', 'thpoll83__PolyKybd__poly_kybd_split72_left': 'split72_left',
         'Huaqiu-Electronics__ecad-viewer__video': 'video', 'GlasgowEmbedded__glasgow__glasgow__revC3': 'Glasgow',
         'doudar__SmartSpin2k__SmartSpin2k_Panelized': 'SmartSpin2k', '0xCB-dev__0xCB-1337__panel': '0xCB panel',
         'CRImier__MyKiCad__protoesp': 'protoesp', 'stonedDiscord__MegaDrive__MegaDrive': 'MegaDrive',
         'stonedDiscord__nonSNES__SNSP-CPU-01': 'SNSP-CPU-01'}
BOARDS = list(SHORT)
# x1828, the default layouts at the task budgets (tasks v7, pinned rates):
# legal1's off rows and the evaluation agent's seeds.
FAMILY = {1: 'legal1-dense-s1-off', 101: 'legal1-dense-s101-off', 201: 'legal1-dense-s201b-off',
          301: 'eval1-dense-s301-off', 401: 'eval1-dense-s401-off', 501: 'eval1-dense-s501-off',
          601: 'eval1-dense-s601-off', 701: 'eval1-dense-s701-off', 801: 'eval1-dense-s801-off',
          901: 'eval1-dense-s901-off'}
BINARY = 'build/bin/pcb-maker-x1828'
REFERENCE = 1          # the trouble set's run (seed A)
SECOND_REFERENCE = 101  # the dependence test's other seed
MODES = ('none', 'stubs', 'global')
SHARES = (0.05, 0.02)
FLOOR_MB = 10240


def rows():
    """Per board and seed: the placed board, the final unconnected, the
    first route's open terminals, the race's probe of the kept placement."""
    out = {}
    for seed, directory in FAMILY.items():
        path = B + directory + '/results.json'
        if not os.path.exists(path):
            continue
        for row in json.load(open(path)):
            if row.get('binary') != BINARY or row['name'] not in SHORT:
                continue
            unconnected = (row.get('native') or {}).get('unconnected')
            if unconnected is None:
                continue
            name = row['name']
            layout = B + directory + '/' + name + '/layout/'
            try:
                report = json.load(open(layout + 'board-layout.json'))
            except Exception:
                continue
            race = report.get('race') or {}
            kept = [p for p in race.get('probe', []) if p['placement'] == race.get('kept')]
            placed = [f for f in os.listdir(layout + 'placed') if f.endswith('.kicad_pcb')]
            out[(name, seed)] = dict(
                unc=unconnected, first=row.get('first_unconnected_terminals'),
                probe=kept[0]['unfinished'] if kept else None,
                probe_open=kept[0]['open_terminals'] if kept else None,
                placed=layout + 'placed/' + placed[0], run=layout, sha=row.get('project_sha256'),
                total=report.get('verdict', {}).get('work_seconds'))
    return out


def mem_available_mb():
    for line in open('/proc/meminfo'):
        if line.startswith('MemAvailable:'):
            return int(line.split()[1]) / 1024
    return 0


def variants(name, seed, data, out):
    """(variant, arguments) for one placement."""
    result = []
    names = {}
    for reference in (REFERENCE, SECOND_REFERENCE):
        if (name, reference) in data and seed != reference:
            names['run%d' % reference] = ['--trouble-from', data[(name, reference)]['run']]
    auto = os.path.join(out, name, 'auto-names.txt')
    names['auto'] = ['--trouble', auto] if seed != REFERENCE else ['--auto']
    for set_name, set_args in names.items():
        for mode in MODES:
            for share in SHARES:
                if set_name == 'run%d' % SECOND_REFERENCE and (mode != 'stubs' or share != 0.05):
                    continue
                result.append(('%s-%s-%g' % (set_name, mode, share),
                               set_args + ['--background', mode, '--share', str(share)]))
    return result


def run_one(binary, placed, arguments, target):
    # Work alone ends a probe: no wall-clock guard (under a load of 28 it
    # cut six 2 % probes of split72_left).
    env = dict(os.environ, PCB_WORK_RATE='3.15e6,150e6', RAYON_NUM_THREADS='1', PCB_ROUTER_NO_WALL_GUARD='1')
    command = [binary, 'probe-trouble-placement', placed, 'auto', '--total-seconds', '1500',
               '--out', target + '.tmp'] + arguments
    started = time.time()
    process = subprocess.Popen(command, env=env, stdout=subprocess.DEVNULL, stderr=open(target + '.err', 'w'))
    return process, started, target


def schedule(jobs, binary, parallel):
    running = []
    jobs = list(jobs)
    while jobs or running:
        while jobs and len(running) < parallel and mem_available_mb() > FLOOR_MB + 2048:
            placed, arguments, target = jobs.pop(0)
            os.makedirs(os.path.dirname(target), exist_ok=True)
            running.append(run_one(binary, placed, arguments, target))
        time.sleep(1)
        still = []
        for process, started, target in running:
            pid, status, usage = os.wait4(process.pid, os.WNOHANG)
            if pid == 0:
                still.append((process, started, target))
                continue
            code = os.waitstatus_to_exitcode(status)
            if code != 0 or not os.path.exists(target + '.tmp'):
                print('failed:', target, open(target + '.err').read()[-400:], file=sys.stderr)
                continue
            report = json.load(open(target + '.tmp'))
            report['outer'] = dict(wall=time.time() - started,
                                   cpu=usage.ru_utime + usage.ru_stime, peak_mb=usage.ru_maxrss / 1024)
            json.dump(report, open(target, 'w'), indent=1)
            os.remove(target + '.tmp')
            print('done', target, 'wall %.0f' % report['outer']['wall'], 'work %.1f' % report['work_seconds'],
                  'left', len(jobs), flush=True)
        running = still


def command_run(arguments):
    data = rows()
    out = arguments.out
    os.makedirs(out, exist_ok=True)
    json.dump({'%s|%d' % key: value for key, value in data.items()}, open(os.path.join(out, 'rows.json'), 'w'), indent=1)
    boards = [b for b in BOARDS if not arguments.boards or SHORT[b] in arguments.boards.split(',')]

    def target(name, seed, variant):
        return os.path.join(out, name, str(seed), variant + '.json')

    def pending(phase):
        jobs = []
        for name in boards:
            for seed in sorted(s for (n, s) in data if n == name):
                if phase == 'reference' and seed != REFERENCE:
                    continue
                if phase == 'rest' and seed == REFERENCE:
                    continue
                for variant, args in variants(name, seed, data, out):
                    path = target(name, seed, variant)
                    if os.path.exists(path):
                        continue
                    if phase == 'rest' and variant.startswith('auto') and not os.path.exists(os.path.join(out, name, 'auto-names.txt')):
                        continue
                    # A 2 % probe the 5 % one did not reach is the same probe.
                    if variant.endswith('-0.02'):
                        five = target(name, seed, variant[:-len('0.02')] + '0.05')
                        if not os.path.exists(five):
                            continue
                        report = json.load(open(five))
                        if report['search_work_seconds'] < 0.02 * 1500 * 0.98 and report['stop'] != 'probe_work_limit':
                            copy = dict(report, budget_seconds=0.02 * 1500, copied_from=five)
                            json.dump(copy, open(path, 'w'), indent=1)
                            continue
                    jobs.append((data[(name, seed)]['placed'], args, path))
                if phase == 'full' and seed in arguments.full_seeds:
                    path = target(name, seed, 'full-75')
                    if not os.path.exists(path):
                        jobs.append((data[(name, seed)]['placed'], ['--background', 'full', '--work-seconds', '75'], path))
        return jobs

    # The reference seed first: its auto probe names the cold-start set.
    for phase in ('reference', 'reference', 'rest', 'rest', 'full'):
        jobs = pending(phase) if phase != 'full' else [j for j in pending('full') if j[2].endswith('full-75.json')]
        if phase == 'full' and not arguments.full_seeds:
            jobs = []
        print(phase, len(jobs), 'probes', flush=True)
        schedule(jobs, arguments.binary, arguments.jobs)
        if phase == 'reference':
            for name in boards:
                auto = target(name, REFERENCE, 'auto-none-0.05')
                if os.path.exists(auto):
                    open(os.path.join(out, name, 'auto-names.txt'), 'w').write('\n'.join(json.load(open(auto))['trouble']) + '\n')
        # the 2 % copies need the 5 % reports: one more pass of the same phase
    print('done')


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


def key_of(report, measure):
    if measure == 'standing':
        return (report['open_terminals'], report['unfinished'], report['work_ms_per_mm'])
    return report[measure]


def command_iterate(arguments):
    """The iteration: the race picks by the seed-A set (run1, 5 %), the
    picked placement's full route names its trouble-makers, and the union
    of both sets probes the other placements again (`iter-<mode>-0.05`,
    the seeds other than A and the pick)."""
    out = arguments.out
    data = {tuple([k.split('|')[0], int(k.split('|')[1])]): v for k, v in json.load(open(os.path.join(out, 'rows.json'))).items()}
    jobs = []
    picks = {}
    for name in BOARDS:
        seeds = sorted(s for (n, s) in data if n == name and s != REFERENCE)
        for mode in arguments.modes.split(','):
            scores = {}
            for seed in seeds:
                path = os.path.join(out, name, str(seed), 'run%d-%s-0.05.json' % (REFERENCE, mode))
                if os.path.exists(path):
                    scores[seed] = key_of(json.load(open(path)), 'standing')
            if len(scores) < len(seeds) or (name, REFERENCE) not in data:
                continue
            pick = min(seeds, key=lambda seed: (scores[seed], seed))
            picks['%s|%s' % (name, mode)] = pick
            for seed in seeds:
                if seed == pick:
                    continue
                target = os.path.join(out, name, str(seed), 'iter-%s-0.05.json' % mode)
                if not os.path.exists(target):
                    jobs.append((data[(name, seed)]['placed'],
                                 ['--trouble-from', data[(name, REFERENCE)]['run'], '--trouble-from', data[(name, pick)]['run'],
                                  '--background', mode, '--share', '0.05'], target))
    json.dump(picks, open(os.path.join(out, 'iter-picks.json'), 'w'), indent=1)
    print('iterate', len(jobs), 'probes', flush=True)
    schedule(jobs, arguments.binary, arguments.jobs)


def command_analyse(arguments):
    out = arguments.out
    data = {tuple([k.split('|')[0], int(k.split('|')[1])]): v for k, v in json.load(open(os.path.join(out, 'rows.json'))).items()}
    measures = ['standing', 'open_terminals', 'unfinished', 'conflicted', 'work_ms_per_mm', 'corridor_misses']

    def load(name, seed, variant):
        path = os.path.join(out, name, str(seed), variant + '.json')
        return json.load(open(path)) if os.path.exists(path) else None

    variant_names = set()
    for name in BOARDS:
        for seed in [s for (n, s) in data if n == name]:
            d = os.path.join(out, name, str(seed))
            if os.path.isdir(d):
                variant_names |= {f[:-5] for f in os.listdir(d) if f.endswith('.json')}
    table = {}
    for variant in sorted(variant_names):
        for measure in (measures if not variant.startswith('full') else ['standing', 'unfinished']):
            table[(variant, measure)] = {}
    print('# per board: seeds, finals')
    summary = {}
    for name in BOARDS:
        seeds = sorted(s for (n, s) in data if n == name)
        finals = {s: data[(name, s)]['unc'] for s in seeds}
        print(SHORT[name], {s: finals[s] for s in seeds})

    def evaluate(scores, seeds, name):
        """Spearman against the finals, top-1 hit and regret over `seeds`
        with a score."""
        seeds = [s for s in seeds if scores.get(s) is not None]
        if len(seeds) < 3:
            return None
        finals = [data[(name, s)]['unc'] for s in seeds]
        values = [scores[s] for s in seeds]
        # Ranks of tuples: compare as tuples.
        order = sorted(range(len(seeds)), key=lambda i: (values[i], seeds[i]))
        rank_values = [0] * len(seeds)
        for r, i in enumerate(order):
            rank_values[i] = r
        # Ties of equal tuples share the lower rank (for Spearman).
        for i in range(len(seeds)):
            for j in range(len(seeds)):
                if values[i] == values[j]:
                    rank_values[i] = min(rank_values[i], rank_values[j])
        rho = spearman(rank_values, finals)
        pick = order[0]
        best = min(finals)
        return dict(rho=rho, hit=finals[pick] == best, regret=finals[pick] - best, n=len(seeds),
                    pick=seeds[pick])

    results = {}
    baselines = {'75 s probe (race)': lambda n, s: data[(n, s)]['probe'],
                 'first-route open': lambda n, s: data[(n, s)]['first']}

    def row_for(label, scorer, seeds_of):
        cells = {}
        for name in BOARDS:
            seeds = seeds_of(name)
            scores = {s: scorer(name, s) for s in seeds}
            cells[name] = evaluate(scores, seeds, name)
        results[label] = cells

    # The seeds of each comparison: the run sets exclude their reference.
    def seeds_excluding(reference):
        return lambda name: sorted(s for (n, s) in data if n == name and s != reference)

    all_seeds = lambda name: sorted(s for (n, s) in data if n == name)
    for label, scorer in baselines.items():
        row_for(label + ' [all]', scorer, all_seeds)
        row_for(label + ' [no s%d]' % REFERENCE, scorer, seeds_excluding(REFERENCE))
        row_for(label + ' [no s%d]' % SECOND_REFERENCE, scorer, seeds_excluding(SECOND_REFERENCE))
    for (variant, measure) in table:
        def scorer(name, seed, variant=variant, measure=measure):
            report = load(name, seed, variant)
            return key_of(report, measure) if report else None
        if variant.startswith('iter-'):
            picks = json.load(open(os.path.join(out, 'iter-picks.json')))
            mode = variant.split('-')[1]
            seeds_of = (lambda name, mode=mode: sorted(s for (n, s) in data if n == name and s != REFERENCE
                                                         and s != picks.get('%s|%s' % (name, mode))))
            # The baselines and the first set on the same seeds.
            if measure == 'standing':
                for label, base in baselines.items():
                    row_for('%s [as %s]' % (label, variant), base, seeds_of)
                first_set = 'run%d-%s-0.05' % (REFERENCE, mode)
                row_for('%s standing [as %s]' % (first_set, variant),
                        lambda name, seed, first_set=first_set: key_of(load(name, seed, first_set), 'standing')
                        if load(name, seed, first_set) else None, seeds_of)
        elif variant.startswith('run%d' % REFERENCE):
            seeds_of = seeds_excluding(REFERENCE)
        elif variant.startswith('run%d' % SECOND_REFERENCE):
            seeds_of = seeds_excluding(SECOND_REFERENCE)
        else:
            seeds_of = all_seeds
        row_for('%s %s' % (variant, measure), scorer, seeds_of)
    print()
    print('| ranker | ' + ' | '.join(SHORT[b] for b in BOARDS) + ' | mean rho | top-1 | regret |')
    print('| --- |' + ' ---: |' * (len(BOARDS) + 3))
    for label, cells in results.items():
        # A score that ties every seed says nothing: 0.
        rhos = [c['rho'] if c['rho'] is not None else 0.0 for c in cells.values() if c]
        hits = sum(1 for c in cells.values() if c and c['hit'])
        boards = sum(1 for c in cells.values() if c)
        regret = sum(c['regret'] for c in cells.values() if c)
        text = [('%+.2f' % c['rho'] if c['rho'] is not None else 'tie') if c else '-' for c in cells.values()]
        print('| %s | %s | %s | %d/%d | %d |' % (label, ' | '.join(text),
              '%+.2f' % statistics.mean(rhos) if rhos else '-', hits, boards, regret))
    json.dump({label: cells for label, cells in results.items()}, open(os.path.join(out, 'analysis.json'), 'w'), indent=1)


def main():
    parser = argparse.ArgumentParser()
    sub = parser.add_subparsers(dest='command', required=True)
    run = sub.add_parser('run')
    run.add_argument('out')
    run.add_argument('--binary', required=True)
    run.add_argument('--jobs', type=int, default=4)
    run.add_argument('--boards', default='')
    run.add_argument('--full-seeds', type=lambda text: [int(x) for x in text.split(',') if x], default=[])
    iterate = sub.add_parser('iterate')
    iterate.add_argument('out')
    iterate.add_argument('--binary', required=True)
    iterate.add_argument('--jobs', type=int, default=4)
    iterate.add_argument('--modes', default='none')
    analyse = sub.add_parser('analyse')
    analyse.add_argument('out')
    arguments = parser.parse_args()
    {'run': command_run, 'iterate': command_iterate, 'analyse': command_analyse}[arguments.command](arguments)


if __name__ == '__main__':
    main()
