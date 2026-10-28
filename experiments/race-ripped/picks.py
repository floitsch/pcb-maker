# Copyright (C) 2026 Toit contributors.
#
# The race's picks in the ripped-count A/B (docs/placer.md, "The race ranks
# its probes by the ripped count (2026-10-27)"). The on side's races report
# each probe's unfinished nets and its ripped count, so the placement the
# unfinished count keeps (off: RACE_CONFIDENT_MARGIN, 5 %) is known from the
# on run; where it is the one the ripped count kept, the layouts are
# byte-identical (the probes and the kept router are the same, the rip-up
# runs on copies and charges no work), and the off side reuses the on row.
# Only the races whose pick changed ran on the off binary.
#
#   python3 -I experiments/race-ripped/picks.py table SEED...      # per race: the probes, both picks, both finals
#   python3 -I experiments/race-ripped/picks.py compare SEED...    # placer_ab compare on the spliced off side
#
# Directories (under build/): race3-dense-s<N>-on (whole seeds),
# race3-q/dense-s<N>-on-<board> and race3-q/dense-s<N>-off-<board> (one
# board each). Run from the repository root.
import glob, json, os, subprocess, sys

HERE = os.path.dirname(os.path.abspath(__file__))
sys.path.insert(0, os.path.join(HERE, '..', '..', 'benchmarks', 'agent-tasks'))
import compare  # noqa: E402

B = 'build'
OFF_MARGIN = 0.05


def rows(directory):
    path = directory + '/results.json'
    if os.path.exists(path):
        return {row['name']: row for row in json.load(open(path))}
    return {}


def race_of(directory, board):
    path = '%s/%s/layout/board-layout.json' % (directory, board)
    return json.load(open(path)).get('race') if os.path.exists(path) else None


def off_pick(unfinished):
    best = min(range(len(unfinished)), key=lambda i: (unfinished[i], i))
    if best == 0 or not unfinished[0]:
        return 0
    return best if (unfinished[0] - unfinished[best]) / unfinished[0] >= OFF_MARGIN else 0


def dirs(seed, side):
    found = glob.glob('%s/race3-q/dense-s%d-%s-*' % (B, seed, side))
    if side == 'on':
        found = ['%s/race3-dense-s%d-on' % (B, seed)] + found
    return [d for d in sorted(found) if os.path.isdir(d)]


def side(seed, which):
    table = {}
    for directory in dirs(seed, which):
        for name, row in rows(directory).items():
            table[name] = (row, directory)
    return table


def table(seeds):
    print('| seed | board | probes | unfinished | ripped | kept on | off | on / off unconnected | right |')
    print('| ---: | --- | --- | --- | --- | ---: | ---: | --- | --- |')
    changed = right = wrong = 0
    for seed in seeds:
        on, off = side(seed, 'on'), side(seed, 'off')
        for board, (row, directory) in sorted(on.items()):
            race = race_of(directory, board)
            if not race:
                continue
            probes = race['probe']
            labels = [p['placement'] for p in probes]
            unfinished = [p['unfinished'] for p in probes]
            ripped = [p.get('ripped_open_terminals') for p in probes]
            pick = labels[off_pick(unfinished)]
            mark = ''
            if pick != race['kept']:
                changed += 1
                unc_on = compare.unconnected(row)
                unc_off = compare.unconnected(off[board][0]) if board in off else None
                if unc_off is not None:
                    mark = 'yes' if unc_on < unc_off else ('no' if unc_on > unc_off else 'tie')
                    right += unc_on < unc_off
                    wrong += unc_on > unc_off
                print('| %d | %s | %s | %s | %s | %d | %d | %s / %s | %s |' % (
                    seed, board.split('__')[-1], labels, unfinished, ripped, race['kept'], pick, unc_on, unc_off, mark))
    print('changed picks %d: right %d, wrong %d' % (changed, right, wrong))


def run_compare(seeds):
    on_args, off_args = [], []
    for seed in seeds:
        on_dirs, off_dirs = dirs(seed, 'on'), dirs(seed, 'off')
        on_args += ['%s@%d' % (d, seed) for d in on_dirs]
        # A later directory's boards replace an earlier one's: the off runs over the on rows.
        off_args += ['%s@%d' % (d, seed) for d in on_dirs + off_dirs]
    subprocess.run([sys.executable, '-I', os.path.join(HERE, '..', '..', 'benchmarks', 'agent-tasks', 'placer_ab.py'),
                    'compare', '--off'] + off_args + ['--on'] + on_args, check=True)


if __name__ == '__main__':
    command, seeds = sys.argv[1], [int(s) for s in sys.argv[2:]]
    {'table': table, 'compare': run_compare}[command](seeds)
