# Board corpus and results

`benchmarks/corpus/run.py` copies each board, strips every track, via and
copper zone, and then runs two modes:

- **Route** keeps the designer's placement and tests the router alone.
- **Place + route** re-places every footprint that is not mechanically fixed
  (locked, without nets, or on the board edge) and then routes, using the
  coupled loop from [placer.md](placer.md).

Rules are read from each board's `.kicad_pro`; nothing is configured per
board. A result counts as clean when native KiCad reports no unconnected items
and no DRC error other than silkscreen or library cosmetics, *beyond what the
stripped source already had* (for example Olimex's fiducials next to its
mounting holes), and the internal exact verifier reports nothing.

```sh
benchmarks/corpus/fetch.sh                      # downloads the KiCad demos
cargo build --release
python3 benchmarks/corpus/run.py build/corpus   # add --only NAME... or --skip-layout
```

## Nightly

Main measured every night by `scripts/nightly.sh`
([benchmarks/agent-tasks/README.md](../benchmarks/agent-tasks/README.md#nightly),
"Nightly"): the fingerprints (Castor, k30-SBC, jetson, Sisu in route mode
from v16), the route-mode expansion tier, the short tier and, on the
configured days, the full layout sweep (tasks v7, `--quality`) with the
matched designer tier. One row a night, newest first, the last 14 nights,
written by `scripts/nightly-report.py` from `build/nightly/<night>/`
(`summary.md` there has the per-board detail). Cells: passes / rows
(strict passes), KiCad's unconnected items, rows cut by the wall clock
and killed; the designer tier's budget scale; the gap split over the
target boards (`compare.py --gap --targets`); against the previous
nightly that ran each stage, better / same / worse (compare.py) and the
boards that gained or lost the pass.

**How the nightly publishes** (since 2026-10-27). The sweep write-ups
v19 to v21 were written by hand from the logs, so README.md and this
file went stale between them. Now the nightly writes them:
`scripts/nightly-report.py --commit` (nightly.sh's last step, on by
default; `--no-commit-report` or `PCB_NIGHTLY_COMMIT_REPORT=0` turns it
off) publishes each night into

- this file: a section "Layout sweep vN (<date>)", vN one more than the
  newest numbered sweep (a night written again keeps its number; a
  marker comment under the heading names the night, its sweep log and
  main), placed after the newest numbered sweep, in the hand-written
  sections' shape: the totals table against the previous night that ran
  the sweep and the previous numbered sweep; better / same / worse,
  unconnected where both have a result, the strict passes gained and
  lost with the rows' finding types; the rows without a result; the
  strict errors by board; the matched designer tier with its gap split
  (`compare.py --gap --targets`, also without the board whose designer
  route leaves most, and over all boards), the five costliest
  placements, the next five and the boards where routing comes first;
  the losers: the boards worse than the previous night, beyond noise
  when the loss exceeds the board's seed spread in
  `benchmarks/agent-tasks/dense-noise.json` (every loss of a board
  without a measured spread counts). Boards whose inputs changed
  (compare.py's INPUT: project files, task, board or constraints) are
  named and left out of each comparison. And the night's row below;
- README.md: the headline (the "Boards harvested from GitHub" bullet
  from "pcb-maker places and routes" on) and the two newest table rows
  (layout and matched designer tier; the previous nightly's two are
  replaced, the hand-written rows stay below);
- docs/handover.md: the title's state, the nightly state line and a
  night note after the last "Layout sweep vN" paragraph (the previous
  nightly's note is replaced);
- docs/changelog.md: "Measured on <date> (nightly: layout sweep vN,
  matched designer tier)".

**The guard.** A night is published only when every stage ran, no row
of any stage was cut by the wall clock or killed, and the sweep has at
least 100 rows with a result. Otherwise the night is summarised
(`summary.md`, `nightly.json`) but nothing is written; the summary's
last line in `nightly.log` says why ("not published: ...").

**The race with the supervisor's merges.** The supervisor merges into
the main checkout during the day, when the night's sweep may still be
running. The report commits only when the main checkout is on main, its
working tree has no change to a tracked file (untracked files do not
count) and `git pull --ff-only origin main` succeeds; then it writes the
four files and commits them alone (`git commit -- <files>`), with the
message "nightly <date>: layout sweep vN, <strict passes> strict (<change>
against the previous night), matched designer tier <strict> strict, gap
placement <p> / routing <r>", and pushes main (with any merge not yet
pushed). Otherwise it writes nothing and says so in `nightly.log`; the
next night publishes every guarded night after the newest one this file
has (at most three, oldest first), so a skipped night is not lost. A
merge that touches these files while the report writes them fails on
the changed files instead of mixing with them. By hand:
`scripts/nightly-report.py --docs build/nightly/<night>` (`--dry-run`
prints the section; `--checkout` writes another checkout's docs).

<!-- nightly:begin -->
| Night | main | Fingerprints | Expansion | Short | Sweep | Designer | Gap (placement / routing) | Against the previous night (better/same/worse) |
| --- | --- | --- | --- | --- | --- | --- | --- | --- |
| 2026-10-31 | `56e15e9be431` | identical | 24/46 (strict 24), 558 open | 8/12 (strict 8), 23 open | 62/109 (strict 62), 2658 open | 71/109 (strict 68), 2759 open at 0.675 | 76% / 24% | fingerprints 0/4/0; expansion 0/46/0; short 0/12/0; sweep 0/107/0; designer 0/109/0 |
| 2026-10-30 | `288051a3e337` | identical | 24/46 (strict 24), 558 open | 8/12 (strict 8), 23 open | 62/109 (strict 62), 2658 open | 71/109 (strict 68), 2759 open at 0.675 | 76% / 24% | fingerprints 0/4/0; expansion 0/46/0; short 0/12/0; sweep 0/107/0; designer 2/107/0 |
| 2026-10-29 | `1f0ca50cccfb` | identical | 24/46 (strict 24), 558 open | 8/12 (strict 8), 23 open | 62/109 (strict 62), 2658 open | 71/109 (strict 68), 2795 open at 0.66 | 75% / 25% | fingerprints 0/4/0; expansion 0/46/0; short 1/11/0 (pass +1 -0); sweep 2/105/0 (pass +1 -0); designer 4/102/3 (pass +2 -1) |
| 2026-10-28T1429 | `c68e9a8f506f` | changed: Sisu_SSE-9 | 24/46 (strict 24), 558 open | 7/12 (strict 7), 28 open | 61/109 (strict 61), 2667 open | 70/109 (strict 68), 2804 open at 0.637 | 74% / 26% | fingerprints 0/4/0; expansion 0/46/0; short 1/9/2 (pass +0 -1); sweep 21/66/20 (pass +5 -3); designer 6/99/4 (pass +2 -0) |
| 2026-10-28 | `be0e62984304` | identical | 24/46 (strict 24), 558 open | 8/12 (strict 8), 20 open | 59/109 (strict 59), 2924 open | 68/109 (strict 64), 3198 open at 0.68 | 53% / 47% | fingerprints 0/4/0; expansion 0/45/1; short 0/12/0; sweep 6/93/4 (pass +2 -0); designer 2/104/3 (pass +1 -0) |
| 2026-10-27 | `e7a12a80e4fe` | identical | 24/46 (strict 24), 557 open | 8/12 (strict 8), 20 open | 57/109 (strict 57), 2140 open | 67/109 (strict 63), 3335 open at 0.69 | 52% / 48% | fingerprints 0/4/0; expansion 0/46/0; short 0/12/0; sweep 4/99/0 (pass +1 -1); designer 5/100/4 (pass +0 -1) |
<!-- nightly:end -->

## Known issues of the harness

- **A waiting large job starves behind a stream of small ones** (seen
  2026-10-23/24; ordered admission since 2026-10-24 and the drain since
  2026-10-25, "Memory ledger: who runs next" below). The memory ledger admits whichever request
  fits when memory frees, so a job that waits for a large reservation
  waits while smaller requests, its own run's or other agents', take
  each freed gigabyte. run.py's tiers start the longest boards first,
  and those are the largest: the matched designer tier of x1725 had all
  eight job slots waiting on neotron-pico (27.2 GB expected), the ULK
  boards, katia and the 0xCB panel (17 to 24 GB) for four hours beside
  the v20 sweep and other agents' runs, with one row written and no
  route running. It was restarted in parts, the boards of up to 6 GB
  expected first (layout sweep v20 below). run.py should admit smaller
  boards past a waiting large one (or hold memory for it in turn); a
  harness item, not done yet.

## Measurement discipline in the tests (2026-10-26)

A test's outcome is decided by work, never by the wall clock or by a
measured rate: the tests run on a shared machine at loads of 30 to 60.

- **Tests pin the rates.** Every run of the fixtures, the walkthrough,
  the fingerprints and the hostile inputs sets
  `PCB_WORK_RATE=3.15e6,150e6` (the reference) and
  `PCB_ROUTER_NO_WALL_GUARD=1` (it turns off the layout's `wall_guard`
  deadline too), and the fixtures `PCB_MAKER_MEMORY_MB`. A rate measured under load is a third of the reference
  or less, and work that does not stop at a budget counts that much more
  seconds: the free dimension's probes of F2 (6 s budgets) counted 14 to
  31 s of work at measured rates of 0.4e6 to 0.97e6 and 4.2 and 6.4 s at
  the reference, so under load the confirmation did not fit the search's
  60 s and `f2s_free_dimension_stops_at_the_first_clean_size` failed (in
  four merge gates on 2026-10-26; at a pinned 0.8e6 it fails every time).
- **A test of the measurement asserts the measurement only.** Where a
  test removes `PCB_WORK_RATE` to check the measured path (the free
  dimension's children at the parent's rates; cli_exit_codes.rs, whose
  routes of the three-pad board take 8 ms of work), it checks the rates
  and their source, no outcome that the rate could change.
- **No wall-clock bound decides a test.** "At once" is checked by what
  ran (no `placer:` line before an input error) or by the verdict's
  `work_seconds`, not by `Instant`: the walkthrough's 60 s bound on a run
  of 8.6 s of work took 16 s of wall under a load of 50. A wall bound is
  only a hang guard far above any load (the ledger test's 600 s wait).

## Measurement discipline: placer defaults (2026-10-26)

**The rule for a placer default** (since 2026-10-26; replaces "the dense
sum of medians falls and every pair's sum falls"). On the eleven dense
boards in layout mode at the task budgets (tasks v7, pinned rates, 3 x 2,
at most six layouts in flight, through the ledger):

1. Run **five seed pairs**, placer seeds 1, 101, 201, 301, 401 on both
   sides (`--placer '{"seed": N, ...}'`). Seeds must lie at least 16
   apart: the race places seeds N to N + 15 (`board_placer/place.rs`,
   `config.seed + offset`), so seeds 2, 3, 5 ... share 15 of seed 1's
   16 candidates and are not independent draws.
2. Judge with `benchmarks/agent-tasks/placer_ab.py compare --off DIR...
   --on DIR...` (each directory's seed from its `-s<N>`, or `DIR@N`).
   The statistic is the **weighted relative change**: each board's mean
   change over its seeds divided by its noise mean, weighted by
   1 / max(cv, 0.10)^2 from the noise file
   (`benchmarks/agent-tasks/dense-noise.json`). Its one-sided p is the
   larger of a paired sign-flip randomisation test (this comparison's
   own spread) and the noise file's null (the tier's spread over nine
   seeds).
3. **Better** when p <= 0.03. When 0.03 < p <= 0.25, run seeds 501, 601,
   701, 801 too and judge the nine pairs again at p <= 0.03. Otherwise
   not better. Worse when the smaller p is at least 0.97.
4. The other conditions stay as each lever sets them (the short tier
   loses no strict pass, no new DRC error). A strict pass lost at the
   short tier's seed is checked on that board alone at the five seed
   pairs (the tier's task and budget): the condition holds when the on
   side passes on at least as many of the five as the off side (since
   2026-10-28; placer.md, "Under the placer-default protocol
   (2026-10-27)").

Simulated from the noise file (`placer_ab.py protocol --flip-rounds 400`,
600 comparisons, the same relative effect on every board): the rule
calls a lever with no effect better 3.5 % of the time and extends 21 %
of such comparisons (5.8 seed pairs on average); it finds a 5 % effect
67 % of the time, 6.5 % 90 %, 8 % 97 %, 10 % 99.8 %. With the noise
file's p alone (`protocol`, 2000 comparisons): 5.2 %, 70 %, 89 %, 98 %,
100 %. The off side's rows can be reused while main's layouts of the
boards stay the same (a layout is deterministic in binary, tasks, rates
and placer settings); when main changes a board's layouts the off rows
and the noise file's rows of that board need running again (on x1842 and
x1849 the escape guard and the panel copies changed protoesp, the 0xCB
panel and SmartSpin2k; their rows were run again on x1851 on 2026-10-27,
"The noise file refreshed" below; the other eight boards' rows are
x1828's). On the refreshed file the rule calls a lever with no effect
better 4.4 % of the time (`protocol`, the noise file's p, 2000
comparisons) and finds a 5 % effect 55 % of the time, 6.5 % 75 %, 8 %
90 %, 10 % 98 %: the panel, which routes completely on every seed now,
carries no weight, and the weighted change resolves 8.5 % with five
pairs and 6.5 % with nine (against 7 and 5.5 % on x1828's file).

**Why.** The weekend's levers (crossings in the anneal, bus-group moves,
`legality: "kicad"`, and `decoupling_near`) moved the dense summed
medians by 1 to 33 and single pairs' sums by 37 to 194, against
per-board seed spreads of 11 to 193; the old rule rejected
all of them, and on the noise below it could not have done otherwise:
with three pairs it calls a lever with no effect better 12 % of the
time and finds a true 5 % gain 32 % of the time.

### Seed noise of the off binary (x1828, nine seeds)

The eleven dense boards on x1828 (main 991ad37) at nine placer seeds:
1, 101, 201 from the legality and bus-group rounds
(`build/legal1-dense-s{1,101,201,201b}-off`), 301 to 801 new
(`build/eval1-dense-s{301,...,801}-off`, 3 x 2 per seed, two seeds side by
side, `--min-free-gb 10 --expected-gb 3.5`, 2026-10-26 15:00 to 2026-10-27
00:50 beside the nightly and four agents; no row cut, killed or in
error; the same tasks, project files and constraints on every row).
KiCad unconnected; the first route's open terminals
(`first_unconnected_terminals`); the placement objective (the placed
board's unweighted wirelength plus 3 mm per ratsnest crossing, counted
by `ablate-kicad-placement` of x1828) and the crossings:

| Board | KiCad unconnected, seeds 1, 101, ..., 801 | mean / sd / spread | first-route open: mean / sd / spread | objective (mm): mean / sd (cv) | crossings: mean / sd (cv) |
| --- | --- | ---: | ---: | ---: | ---: |
| OpenESC 4in1-mini | 81, 82, 99, 100, 83, 105, 102, 94, 76 | 91 / 10.8 / 29 | 143 / 14.4 / 47 | 1058 / 41 (0.04) | 86 / 10 (0.12) |
| OpenESC 4in1 | 106, 125, 125, 109, 155, 125, 126, 141, 106 | 124 / 16.3 / 49 | 180 / 26.0 / 80 | 2110 / 269 (0.13) | 246 / 60 (0.24) |
| split72_left | 23, 21, 33, 35, 26, 29, 31, 30, 38 | 30 / 5.5 / 17 | 40 / 9.4 / 31 | 24440 / 823 (0.03) | 3745 / 154 (0.04) |
| video | 190, 287, 178, 134, 327, 263, 265, 256, 262 | 240 / 60.5 / 193 | 311 / 88.5 / 254 | 87537 / 12575 (0.14) | 15988 / 2839 (0.18) |
| DSP-ADAU1452 | 17, 7, 28, 21, 9, 11, 14, 8, 11 | 14 / 6.9 / 21 | 66 / 12.3 / 37 | 6516 / 447 (0.07) | 737 / 95 (0.13) |
| 0xCB panel | 50, 50, 51, 51, 51, 45, 54, 56, 53 | 51 / 3.1 / 11 | 105 / 60.7 / 190 | 7036 / 172 (0.02) | 472 / 25 (0.05) |
| Glasgow | 143, 149, 173, 134, 153, 158, 145, 124, 127 | 145 / 15.5 / 49 | 169 / 20.3 / 73 | 5997 / 511 (0.09) | 888 / 127 (0.14) |
| SmartSpin2k | 34, 59, 52, 49, 52, 59, 51, 78, 67 | 56 / 12.3 / 44 | 60 / 12.1 / 42 | 28195 / 585 (0.02) | 3281 / 121 (0.04) |
| SNSP-CPU-01 | 53, 95, 79, 43, 104, 59, 108, 111, 94 | 83 / 25.5 / 68 | 105 / 51.7 / 175 | 32379 / 930 (0.03) | 4485 / 247 (0.06) |
| protoesp | 110, 76, 90, 64, 53, 77, 49, 72, 83 | 75 / 18.7 / 61 | 108 / 34.4 / 100 | 8380 / 1318 (0.16) | 899 / 209 (0.23) |
| MegaDrive | 110, 108, 157, 103, 146, 169, 143, 86, 121 | 127 / 27.9 / 83 | 156 / 34.5 / 98 | 60526 / 6768 (0.11) | 9694 / 1476 (0.15) |
| sum | 917, 1059, 1065, 843, 1159, 1100, 1088, 1056, 1038 | 1036 / 96.9 / 316 | 1444 / 160.5 / 472 | 264174 / 14178 (0.05) | 40522 / 3230 (0.08) |

The tier's sum over the boards spreads 843 to 1159 between seeds (sd 97):
one seed pair's summed change has an sd of about 137, three pairs'
mean about 79 (8 % of the sum). Video alone spreads 134 to 327.

**Proxies.** Correlation across the nine seeds within each board,
against the final unconnected count (Pearson / Spearman; pooled: each
board's values standardised):

| Board | first-route open | objective | crossings |
| --- | ---: | ---: | ---: |
| OpenESC 4in1 | +0.72 / +0.63 | +0.43 / +0.60 | +0.47 / +0.51 |
| OpenESC 4in1-mini | +0.60 / +0.48 | +0.28 / +0.12 | +0.19 / +0.12 |
| DSP-ADAU1452 | +0.85 / +0.80 | +0.07 / +0.28 | +0.22 / +0.36 |
| MegaDrive | +0.89 / +0.92 | +0.62 / +0.45 | +0.68 / +0.48 |
| 0xCB panel | +0.63 / +0.86 | +0.72 / +0.63 | +0.27 / +0.19 |
| split72_left | -0.01 / -0.14 | +0.43 / +0.52 | +0.38 / +0.37 |
| protoesp | +0.97 / +0.98 | +0.78 / +0.77 | +0.81 / +0.73 |
| Glasgow | +0.86 / +0.85 | +0.91 / +0.93 | +0.93 / +0.95 |
| SmartSpin2k | +0.96 / +0.82 | -0.04 / -0.21 | -0.21 / -0.28 |
| SNSP-CPU-01 | +0.63 / +0.83 | +0.38 / +0.30 | +0.50 / +0.43 |
| video | +0.96 / +0.92 | +0.64 / +0.53 | +0.47 / +0.42 |
| within boards, pooled | +0.73 (99) | +0.47 (99) | +0.43 (99) |

- **The first route's open terminals are not a lower-noise proxy**: they
  follow the final count closely (pooled r 0.73; 0.85 to 0.97 on six
  boards) because they are the same thing measured earlier, with the
  same or more relative spread (the 0xCB panel's first route spreads 76
  to 266 where the final stays 45 to 56). They resolve a relative effect
  no better than the final count (`resolution --metric first_open`: 8.5
  % at five pairs against 7 %) and are not what the rule judges.
- **The objective is low-noise and weakly tied to routing**: its spread
  is 2 to 16 % of its mean (the sum's 5 %) and the weighted statistic
  resolves 3.5 % of it with five pairs, but within a board it explains
  about a fifth of the unconnected count's spread (pooled r 0.47; 0.91
  on Glasgow, 0.78 on protoesp, about 0 on SmartSpin2k and DSP-ADAU1452).
  A lever that lowers the objective is a placement change, not yet a
  routing gain (the bus-group row below).

### The noise file refreshed (2026-10-27)

Three boards' layouts changed after x1828 by design: protoesp (the
escape guard, x1842), the 0xCB panel and SmartSpin2k_Panelized (the
panel copies, x1849; SmartSpin2k's placed objective halved, 28195 to
14431 mm, its crossings 3281 to 1533). Their nine seeds on x1851 (main
71953b0, the off rows of seeds 1 to 401 of the crossings-in-the-anneal
pairs and four seeds more, `build/cross3-u/noise-s{501,...,801}-off-*`;
2 threads, through the ledger with 10 GB kept free) replace their rows
in `dense-noise.json` (`placer_ab.py noise ... --boards protoesp panel
SmartSpin2k_Panelized --update`; each board entry now names its
binary). The other eight boards keep their x1828 rows. Not refreshed,
and stated: on x1851 the five off seeds of split72_left differ from
x1828's (KiCad unconnected 0 of 5 the same, the placed boards 1 of 5),
DSP-ADAU1452 1 of 5 (its placements 4 of 5 the same), video and
SNSP-CPU-01 4 of 5; Glasgow, MegaDrive and both OpenESC boards are the
same on all five. Those are other draws of the same noise, not a
changed layout.

| Board | KiCad unconnected, seeds 1, 101, ..., 801 | mean / sd / spread | first-route open: mean / sd / spread | objective (mm): mean / sd (cv) | crossings: mean / sd (cv) |
| --- | --- | ---: | ---: | ---: | ---: |
| 0xCB panel (x1851) | 0, 0, 0, 0, 0, 0, 0, 0, 0 | 0 / 0 / 0 | 0.4 / 0.7 / 2 | 6628 / 342 (0.05) | 437 / 38 (0.09) |
| SmartSpin2k (x1851) | 21, 28, 94, 36, 33, 43, 49, 20, 72 | 44 / 24.6 / 74 | 46 / 25.1 / 78 | 14431 / 1136 (0.08) | 1533 / 198 (0.13) |
| protoesp (x1851) | 73, 75, 59, 70, 65, 72, 56, 89, 78 | 71 / 10.0 / 33 | 111 / 20.1 / 56 | 8156 / 479 (0.06) | 884 / 90 (0.10) |
| sum (three boards x1851, eight x1828) | 817, 977, 1025, 785, 1101, 1034, 1039, 959, 985 | 969 / 104.3 / 316 | 1329 / 164.4 / 476 | 249779 / 15159 (0.06) | 38723 / 3434 (0.09) |

- **The panel** routes completely on every seed (first route 0 to 2
  open): with a noise mean of 0 it has no weight in the weighted change
  (it had 23 % of it on x1828's file; `compare` lists it as "weight 0").
- **SmartSpin2k** became the noisiest board relative to its mean (cv
  0.56, 20 to 94; x1828: 0.22): weight 3 instead of 20.
- **protoesp** spreads less than on x1828 (cv 0.14 against 0.25): weight
  50 instead of 16. Within the board its first-route open terminals no
  longer follow the final count (r 0.28 against 0.97).
- The pooled within-board correlations with the final count: first
  route 0.68, objective 0.37, crossings 0.38 (x1828: 0.73, 0.47, 0.43).

### Resolution per k and statistic

`placer_ab.py resolution` (2000 simulated comparisons per k): each board's
values drawn from its nine seeds (a smoothed bootstrap that keeps their
mean and variance), the on side times 1 - delta on every board; each
statistic's threshold set at a 5 % one-sided false positive rate under
no effect (the old rule has a fixed threshold: its false positive rate in
brackets). The smallest relative improvement found with power 0.8:

| k pairs | sum of medians | mean of pair sums | old rule (medians and every pair) | sign test (boards better) | weighted relative |
| ---: | ---: | ---: | ---: | ---: | ---: |
| 3 | 18.5 % | 16.0 % | 15.0 % (fp 0.12) | 16.0 % | 9.5 % |
| 5 | 15.0 % | 12.0 % | 17.5 % (fp 0.03) | 12.5 % | 7.0 % |
| 9 | 12.0 % | 9.0 % | 19.5 % (fp 0.00) | 9.5 % | 5.5 % |

Power at a 3 %, 5 % and 10 % improvement:

| k pairs | sum of medians | mean of pair sums | old rule | sign test | weighted relative |
| ---: | --- | --- | --- | --- | --- |
| 3 | 0.10 / 0.15 / 0.36 | 0.12 / 0.18 / 0.45 | 0.23 / 0.32 / 0.58 | 0.11 / 0.20 / 0.51 | 0.20 / 0.40 / 0.87 |
| 5 | 0.13 / 0.21 / 0.53 | 0.15 / 0.26 / 0.66 | 0.09 / 0.15 / 0.41 | 0.15 / 0.29 / 0.66 | 0.28 / 0.55 / 0.98 |
| 9 | 0.15 / 0.27 / 0.69 | 0.20 / 0.39 / 0.89 | 0.01 / 0.04 / 0.20 | 0.21 / 0.40 / 0.84 | 0.40 / 0.78 / 1.00 |

- **The summed medians cannot resolve a 3 to 5 % effect at any k we can
  afford**: video, MegaDrive and SNSP-CPU-01 carry the sum and are the
  noisiest boards (cv 0.22 to 0.31). The old rule's every-pair condition
  makes it weaker with more seeds (at nine pairs one of nine falling by
  chance the other way is likely).
- **The weighted relative change resolves about twice as small an
  effect** for the same seeds, because it listens to the steady boards:
  the 0xCB panel 23 % of the weight, Glasgow 20 %, the 4in1-mini 16 %,
  the 4in1 13 %, video 4 %. It answers "does the typical board route
  better", not "does the tier's count fall"; a lever that helps only
  video will not pass it (nor would it pass any statistic on this
  noise). The cv floor matters: 0.05 gives 6.5 / 4.5 % at five / nine
  pairs (the panel then carries the tier), 0.20 gives 8.5 / 6.5 %;
  0.10 keeps the panel at most a quarter.
- **k**: five pairs resolve about 7 %, nine about 5.5 %; the two-stage
  rule spends nine only where five leave it open (a fifth of the null
  comparisons, two fifths at a 3-5 % effect).

### The weekend's levers re-judged (no new runs)

Each lever's own rows (KiCad unconnected; on the boards a switch did not
change, off and on are the same layout), judged by `placer_ab.py
compare` with the x1828 noise file; the objective by `--metric
objective --tool build/bin/pcb-maker-x1828`:

| Lever (off / on, rows) | pairs | sum of medians | old rule | weighted change (90 % from the noise file) | p sign-flip / noise | verdict | pairs to resolve the observed size | objective: sum of medians, weighted, verdict |
| --- | ---: | ---: | --- | ---: | ---: | --- | ---: | --- |
| crossings in the anneal (x1817 / x14312, `build/cross2-dense-*`) | 3 | 926 -> 927 | no | -8.1 % (-14.0 to -2.0) | 0.047 / 0.013 | extend to 5 (seeds 301, 401) | 5 | -4.3 %, -1.2 %, not better |
| bus groups (x1828 / x14560, `build/group1-dense-*`, off `legal1`) | 3 | 967 -> 960 | no | -0.5 % (-6.5 to +5.5) | 0.453 / 0.440 | not better | more than 40 | -9.4 %, -3.9 %, **better** (p 0.012) |
| `legality: "kicad"` (x1828 / x14502, `build/legal1-dense-*`) | 3 | 967 -> 934 | no | +2.8 % (-1.9 to +7.5; on the 4 boards it moves +5.3 %) | 0.585 / 0.837 | not better | 24 | -0.2 %, -1.7 %, extend |
| `decoupling_near` (x1835 / x14741, `build/ux2-dense-*`) | 2 | 974 -> 987 | no | +4.5 % (-2.9 to +11.7) | 0.897 / 0.840 | not better | 14 | -2.3 %, +5.4 %, worse |
| escape demand, for reference (x1751 / x1752, `build/escape{1,2}-dense-*`) | 3 | 968 -> 933 | yes | -1.5 % (-7.4 to +4.6) | 0.359 / 0.339 | not better | more than 40 | (seed 1 partly without placed boards) |
| ratsnest crossings, for reference (x1813 / x14270, `build/cross1-dense-*`) | 3 | 1027 -> 966 | yes | -4.1 % (-10.0 to +2.0) | 0.029 / 0.134 | extend to 5 | 18 | -7.1 %, -14.0 %, **better** (p 0.000) |

- **Crossings in the anneal** is the one with a likely real routing
  effect: -8.1 % weighted, 8 of 11 boards better by the mean (Glasgow,
  the 4in1, MegaDrive, SNSP-CPU-01 by 16 to 21 %), its sum of medians
  flat only because video rose by 63 on the mean (its widest swing,
  +26 %, on a board that carries 4 % of the weight). The rule's first
  stage is not complete: two more pairs (seeds 301 and 401 on both
  x1817 and x14312, or the switch on today's main against its off rows)
  settle it; at -8.1 % five pairs find it with power 0.8. Run on
  2026-10-27 (x1851 both sides, five pairs, the refreshed noise file):
  -10.8 % (p 0.002), better on the dense boards; the short tier's lost
  pass (framework_mobo_lefthalf) was noise at five seeds (4 passes on,
  3 off): on by default since 2026-10-28 (placer.md, "Under the
  placer-default protocol (2026-10-27)").
- **Bus groups** lowers the objective with confidence (the only lever
  that does: -9.4 % on the summed medians, p 0.012) and leaves routing
  where it was (-0.5 %): no number of seeds would confirm the observed
  size. The objective's weak tie to routing (above) is the reading.
- **`legality: "kicad"`** moves four boards and is, if anything, worse on
  them (+5.3 %); confirming even that would take about 24 pairs.
- **`decoupling_near`** has two pairs (x1835 off: a later binary than the
  noise file's, so its p is approximate); +4.5 %, not better; 14 pairs
  would resolve its size either way.
- Of the two defaults shipped under the old rule, escape demand has no
  resolved routing effect (-1.5 %) and ratsnest crossings an
  unresolved -4.1 % that would take about 18 pairs to confirm; both
  passed the old rule, as a lever with no effect does 12 % of the time.
- The levers' objective changes are mostly smaller than "5 to 12 %": the
  bus groups' -9.4 % and the crossings' -7.1 % on the summed medians;
  crossings in the anneal -4.3 %, `legality: "kicad"` -0.2 %.

**Usage.**

    python3 benchmarks/agent-tasks/placer_ab.py compare \
      --off build/X-dense-s{1,101,201,301,401}-off --on build/X-dense-s{1,101,201,301,401}-on
    python3 benchmarks/agent-tasks/placer_ab.py resolution --k 3 5 9 --at 0.03 0.05 0.10
    python3 benchmarks/agent-tasks/placer_ab.py protocol
    python3 benchmarks/agent-tasks/placer_ab.py noise build/Y-dense-s{1,...,801}-off \
      --out benchmarks/agent-tasks/dense-noise.json --tool build/bin/pcb-maker-xNNNN

`compare` prints the per-board table, every statistic (the sum of
medians, the pair sums, the old rule, the sign test, the weighted
change with its p values and interval), the verdict and the pairs a
change of the observed size needs; `--metric first_open|objective|crossings`
judges a proxy instead (`--tool` counts the objective and the crossings
of each placed board, cached by its sha256 in `build/placer-ab-signals`).

### The ladder of placer measurements (2026-10-28)

The protocol above costs 110 full layouts a lever (about 88 CPU hours,
18 hours of wall beside the nightly). Calibrated against its two complete
experiments (crossings in the anneal: BETTER, -10.8 %; the race's ripped
count: EXTEND, -2.4 %; placer.md, "Cheaper stages of the placer
protocol, calibrated (2026-10-28)"), a lever climbs three stages and
stops where it fails:

| stage | what runs | cost (the crossings experiment) | calibration | false-negative risk |
| --- | --- | --- | --- | --- |
| 0: placement | the placement alone, or the placed boards of stage 1's rows; `ablate-kicad-placement` metrics (crossings, objective, wirelength, blocked exits, overflows) | a quarter of a layout's wall (0.25 to 0.28: about 22 CPU h for 110 placements at the task budget; main's placer is 22 % cheaper since 2026-10-28) | row Spearman with the final count +0.10 to +0.44; agrees with the full verdict only where the metric is the lever's own (crossings on the crossings lever); blind to anything after the placement (the race) | high: a check that the lever moved what it meant to move, not a filter |
| 1: first route | the five seed pairs with `--layout '{"stop_after_first_route": true}'` (or any full rows' `first_unconnected_terminals`); `placer_ab.py compare --metric first_open` | 0.55 to 0.57 of the full stage (about 48 CPU h); per board 0.19 (0xCB panel) to 0.99 (video) | the full verdict on both: BETTER -8.5 % (p 0.014 / 0.006) and EXTEND -2.3 % (p 0.248); rows +0.78 / +0.82 with the finals, per-pair changes +0.66 / +0.69, boards' signs 9/10 and 7/7 | low on these two: it passed on every resampled BETTER (1000 resamples of the seeds); it passed 60 % of the race's resamples whose full change was negative |
| 1': quarter budget | the five seed pairs at `--budget-scale 0.25` | about a quarter (pending) | pending: 165 layouts queued 2026-10-28, none admitted by the ledger by 13:35 | pending |
| 2: full | the protocol above | 110 layouts, 88 CPU h | the ground truth | - |

- **A lever passes stage 1** when the first route's weighted change is
  negative at the noise file's p <= 0.25 (`placer_ab.py compare --metric
  first_open`); then stage 2 on the same seeds, which repeats stage 1's
  placements and first routes (a full layout cannot start from a stopped
  one, and `--reuse` does not cross the option: it is in the key).
- **What it saves** is half, not an order of magnitude: the first route
  is most of a dense layout's routing (video's runs to the budget's end).
  Stage 1 is worth it where most levers fail: a lever that fails stage 1
  costs 48 CPU hours instead of 88. The quarter budget, if its
  calibration agrees, is the stage that would cut by four.
- **Reuse** (run.py `--reuse build/layout-reuse`, README.md "Reused
  layouts"): run both sides on one binary with the lever as an option;
  boards the lever leaves alone then cost their placement only (about a
  quarter), and every rerun of a row (the next lever's off side, the noise
  file) too. On the two calibrated experiments it would have reused
  nothing (the anneal's term moved all 55 placements; the race used two
  binaries); the race's own rule (reuse the on row where the race kept
  the same placement) saved 34 of 110 layouts.
- **Condensed boards** (a window around a crowded cluster) are not a
  stage: placer.md, "Condensed boards as a fast benchmark: an assessment
  (2026-10-28)", no-go.

## Memory ledger: who runs next (2026-10-24, the drain 2026-10-25, its feasibility cap, its end and short requests 2026-10-26, the measured floor, the capacity from the machine, kicad-cli under a reservation, the hand-over and the wait past a reservation 2026-10-27, the drain past its bound 2026-10-28)

Benchmarks share the machine through the memory ledger
(`$XDG_RUNTIME_DIR/pcb-maker-ledger.json`; how a run reserves and waits:
[benchmarks/agent-tasks/README.md](../benchmarks/agent-tasks/README.md),
"Runs side by side, kept apart by the memory ledger", and
[agents.md](agents.md), "Memory: the machine's ledger"). run.py's floor
of available memory is measured (since 2026-10-27, "The floor from
measured headroom" below; 10 GB without samples); `--min-free-gb`
overrides it. `--lock` still takes the tier lock for wall or CPU
measurements and works as before.

**Admission is ordered.** A request that waits is in the ledger (megabytes
0, `waiting_mb`, `waiting_since`; `pcb-maker ledger` lists it): run.py's
boards waiting in `harness_common.admit` as well as binaries waiting for
their first reservation. A new request (a board run.py admits, a binary
that holds nothing yet and was not admitted by a harness above it) may
not pass the request that has waited longest:

- while the oldest would fit now (it has not polled yet), the newer one
  goes only if both fit;
- while the oldest cannot fit, the newer one goes only if the oldest
  would still fit beside it and every reservation that began holding after
  the oldest began waiting (`held_since`): once the runs it waits on end,
  it fits, so small runs keep going but never starve it;
- a request larger than the capacity runs alone: nothing newer passes it;
- a run that holds memory and grows (the router's ladder, KiCad's check)
  is not held back, since the oldest may be waiting for it to end; nor is
  a binary run.py admitted (its reservation handed to the binary);
- held back only by its harness's floor of available memory
  (`--min-free-gb`; the ledger has its room: `waits_on_floor`), the
  oldest is passed as when it cannot fit, so a floor no other run can see
  does not stop the machine.

**The ledger drains for a starved waiter** (2026-10-25). Ordered
admission alone did not stop a waiter held back by its floor from
starving: on 2026-10-25 the nightly's last designer boards (katia 19 GB
expected, the others 6-27 GB, `--min-free-gb 10`) waited from about
11:00 to past 20:00 ("katia started after waiting 33034 s") while five
agents' 1-8 GB routes kept arriving: each fit the ledger's capacity and
left the waiter its room there, so the running set never drained to the
29 GB of available memory katia's floor asks for. Now, once the first
waiter has waited longer than its bound (`drain_after` in its entry;
run.py's `--ledger-drain-after`, default 600 s), a new request goes only
if

- after it, the waiter's room still covers what it waits for: the
  capacity less every other reservation, and the memory available less
  what the reservations still grow by and less the waiter's floor
  (`floor_mb`, run.py's `--min-free-gb`), so the reservations shrink
  toward what lets the waiter in as runs end; or
- it is expected to end no later than every run that holds memory
  (`expected_end`: run.py's boards of a tier carry the tier's seconds),
  so it does not extend the drain. Binaries do not know their end: in a
  drain they go by the waiter's room alone.

A run that holds memory and grows, and a binary run.py admitted, are not
held back, as before. `drained_for` and `drain_allows` in ledger.rs and
harness_common.py (the same rule); tests
`a_stream_of_small_requests_starves_a_waiter_until_the_ledger_drains`
(a simulation of that day: a 19 GB waiter with a 10 GB floor, 30 GB
available, a 2 GB request every 5 s that runs 60 s: without the drain it
never starts in four hours beside 2939 small runs; with it, it starts
660 s after it began waiting), `a_drain_admits_what_leaves_the_waiter_its_room_or_ends_first`
and test_run.py's `MemoryLedger` (the same simulation on the Python
side).

**The nightly's priority.** `--ledger-priority nightly` (scripts/nightly.sh
sets it on every stage) marks run.py's requests `priority: "nightly"`: a
nightly request goes first among the waiters (before an older one that is
not the nightly's) and is never passed once waiting: the ledger drains
for it at once and without the end exception. `pcb-maker ledger` marks
it `[nightly]`.

**The drain's feasibility cap** (2026-10-26). A drain for a waiter that
cannot fit before a large run ends holds back requests that would not
delay it. On 2026-10-25 from 22:30 the nightly's last designer boards
(neotron-pico, ULK_sl_PG1316s, ULK: 27.9, 24.7 and 23.2 GB expected, with
the 10 GB floor 33-38 GB of the roughly 42 GB available on an idle
machine) could only run one at a time, and each next one's drain would
hold every other request back while the one before it ran. Now, while
the drained-for waiter is `blocked_by_large` (a run larger than
`SMALL_REQUEST_MB`, 4096, holds memory, and with every smaller run gone
the waiter's room, the capacity and the memory available less its
floor, still does not cover it), a request of at most 4096 MB goes; the
large ones wait. Once the large run ends (or none holds memory), the
drain holds everything as before and the small runs end before the
waiter goes. Tests `small_requests_go_while_the_waiter_waits_for_a_large_run_to_end`
(ledger.rs) and test_run.py's `MemoryLedger` (the same simulation:
58 066 MB, 42 000 available, an 8.8 GB run until 900 s, the three nightly
boards each running 1500 s, three agents' chains of 2 GB binaries of
120 s): without the cap a binary waits up to 3900 s (264 run in four
hours), with it 120 s (348); the boards start at 960, 2460 and 3960 s
instead of 900, 2400 and 3900. A drain deadline (the drain gives up after
the waiter's run time) was simulated too and not taken: the second and
third boards never started in eight hours, as before the drain.

That night itself was not this rule's case: the nightly ran main
356c8db, from before the drain (2026-10-25 21:00), so its waiting
entries had no `floor_mb` and no `priority`. The other agents' harnesses
drained for it with a floor of 0 and the end exception: they held each
other back to keep 23-28 GB of room it could not use (it needed 33-38 GB
by its own floor, or an empty ledger), while newcomers that ended before
every holder kept the ledger from emptying. And the estimates were wrong:
the designer tier's peaks were its boards' v17 layout rows (x1170,
placement and routing), not routes on the designer's placement. Measured
(five designer runs, the largest row): neotron-pico 5497 MB, ULK 1512,
ULK_sl_PG1316s 1389, katia 6550, the 0xCB panel 1534; the median board's
estimate was 1.8 times its measured peak. designer.json now carries the
measured peaks and seconds (`benchmarks/agent-tasks/tier_estimates.py`),
and scripts/nightly.sh runs the designer tier before the sweep, its
largest boards first (`run.py --order peak-desc`).

**The ledger drains only for a waiter it could start** (2026-10-26).
From the evening of 2026-10-25 for more than 27 hours the 2026-10-25
nightly's designer stage (pid 4134524, a harness copy of 356c8db, from
before every drain change) waited for neotron-pico, ULK_sl_PG1316s and
ULK (27.9, 24.7 and 23.2 GB expected) with its own `--min-free-gb 10`.
Its entries were the oldest waiters, so every newer harness and binary
drained for it: with the feasibility cap only requests of 4 GB or less
went (katia 7 GB, video 8 GB, ducktop2 16 GB waited for hours), and a
fresh agent's trial saw 3-7 minutes of waiting per run even for small
boards (docs/reviews/2026-10-26-fresh-agent-trial.md, friction 1). By the
ledger's room it fit (26 GB reserved of 58), but its floor and the
other runs' growth kept it out, and the stream of the other runs that
the drain does not hold back (runs that grow, the nightly's own
binaries, the small requests) never let it in. Now a waiter counts for
the drain (`drain_state` in ledger.rs and harness_common.py, the same
rule) only

- while, with every reservation that began holding since its wait began
  removed, its room covers what it waits for: the capacity less the
  older reservations, and the memory available less the reservations'
  growth, with the removed ones' megabytes back, less its own floor
  (`floor_mb`; an older writer's entry without one, recognisable by a
  missing `refreshed`, is assumed run.py's 10 GB, `DEFAULT_FLOOR_MB`);
  otherwise the ledger could not start it before older runs end
  (`not drained for: ... too little to start it`);
- while its wait is within its bound (`drain_max` in its entry, run.py's
  `--ledger-drain-max`, default 7200 s, `DEFAULT_DRAIN_MAX_SECONDS`; the
  nightly's as well);
- while its writer rewrites its entry: new harnesses and binaries write
  `refreshed` on every poll (15 s and 2 s), and an entry not rewritten
  for more than 120 s (`STALE_AFTER_SECONDS`) is stale; an entry without
  `refreshed` (an older writer's) is assumed live.

A waiter that does not count keeps its place: newer requests pass it by
ordered admission alone (room for it beside the runs since its wait
began), and once it counts again (older runs ended, or room appears)
the drain resumes, until the bound (since 2026-10-28 strictly past it, below). There is no drain for the next
waiter meanwhile. `pcb-maker ledger` ends with the first waiter and its
state (`first waiter: pid ..., waiting ... s for ... MB: the ledger
drains for it` or `not drained for: ...`; `--json`: `first_waiter`), and
the waiting lines name it (`behind an older request, ... ; not drained
for: it has waited past the drain's bound of 7200 s`). Tests
`the_ledger_drains_only_for_a_waiter_it_could_start` (ledger.rs) and
test_run.py's `test_the_ledger_drains_only_for_a_waiter_it_could_start`,
the same numbers: a 58 066 MB ledger, the old harness's 24 724 MB waiter
(no floor, no `refreshed`), 26 GB reserved by two runs (the smaller
3 GB, holding another hour), 31 000 MB available less growth, a 7 GB
request. With the runs older than its wait its room is 31 000 - 10 240
= 20 760 MB: before, the drain held the 7 GB request back; now it is not
drained for and the request goes by ordered admission (7168 + 24 724 of
58 066). With the runs newer than its wait (24 GB, the live ledger's
case) it could start once they end, and the ledger drains for it until
7200 s; then the 7 GB request goes, a 16 GB one still waits for it
(24 576 + 16 384 + 24 724 > 58 066). The simulations of the drain and
the cap above give the same numbers as before. Smoke (a private ledger,
`PCB_MAKER_LEDGER_TOTAL_MB` 8000; a holder of 4000 MB since before the
wait; the 356c8db harness_common.py waiting for 4500 MB with a 10 GB
floor, its clock 1000 s back): x1834's route of the fence board waited
`behind an older request ... the ledger drains for it` until its 25 s
timeout; the branch's reserved its 795 MB at once, and `pcb-maker
ledger` named the waiter `not drained for: ... its room would be
4000 MB ... too little to start it`. On the live ledger the same day
(read only) the branch's `pcb-maker ledger` named pid 4134524's
ULK_sl_PG1316s, waiting 97 967 s for 24 724 MB, `not drained for: it
has waited past the drain's bound of 7200 s`. Harness copies and
binaries before this change still drain for it until they are rebuilt
or end.

**Short requests never wait behind a drain** (2026-10-26). On the
evening of 2026-10-26 a fresh agent's `drc-kicad-board` of a small board
(1.5 s of work, 2253 MB: KiCad's check) waited 657 s
(docs/reviews/2026-10-26-schematic-trial.md, friction 1) `behind an older
request, run.py: stonedDiscord__nonSNES__SNSP-CPU-01 ... (waiting 4312 s
for 3004 MB; the ledger drains for it)`: pid 3598969, `--min-free-gb 10`,
four route boards of 0.6-4.3 GB that waited 2210-4371 s each. That
waiter was held by real memory, not by the ledger: its log
(build/pour6-dense-off-x1844.err) reads, when its boards began waiting
(20:37), 34.5 GB reserved by 12 runs of 56.7 GB, 30 waiting, 30.1 GB
available and 20.7 GB still to grow (9.4 GB beside the growth; the board
needs 2.9 + 10 GB), and when stonedDiscord started (21:49) 30.6 GB
reserved, 31.4 GB available, 18.3 GB to grow (13.1 GB). About 19 GB of
the machine's 62.7 GB were outside the ledger at both ends (the desktop
and its browser, the agents' own processes, builds). The drain held every
new request for it (it could start once the runs since its wait began
end, and no run of more than 4 GB held memory, so the feasibility cap did
not apply). Now a short request goes whenever the memory for it is there,
whatever the order and the drain say:

- short: the command holds its memory for seconds (`arm`'s class: every
  command that reserves but `route-kicad-board` and `layout-kicad-board`,
  so `drc-kicad-board`, `inspect-routing-run --native-check` and
  `place-kicad-board`, which reserves only for its check;
  `PCB_MAKER_LEDGER_SHORT=1` makes a command short, `0` long), and the
  request is new (the process holds nothing, nor a harness above it) and
  at most `SHORT_OWN_MB` (2048) of its own beside at most one KiCad check
  (`KICAD_CHECK_MB`, 2.2 GB);
- the memory is there: the capacity has its room (as for any request)
  and the memory available less what the reservations still grow by
  covers it (`short_allows`; a binary keeps no floor of its own, the
  waiter's floor does not count);
- its reservation is marked `short` (`pcb-maker ledger`: `[short]`) and
  counts as gone in the order's and the drain's sums for every other
  request (`order_allows`, `drain_state`, `drain_allows`,
  `blocked_by_large`): it never passes a waiter there, and a route that
  arrives beside it is admitted as without it. The capacity counts it.

A short request the memory does not cover waits as before (ordered, and
drained for in turn). Everything else is unchanged. harness_common.py's
`try_admit(..., short=True)` is the same rule for a harness's request (at
most 2048 + 2253 MB in all, against its own `min_free_mb`); run.py's
boards are not short. Tests `short_requests_never_wait_behind_a_drain`
(ledger.rs, a simulation of the evening: 58 066 MB, 44 000 available
beside the reservations, eight runs of 3875 MB since the waiter's wait
began ending one every 120 s from 600 s, the 3004 MB board with its
10 240 MB floor waiting since -4312 s, the check at 0 s, other agents'
checks every 20 s, a 2000 MB route at 30 s): before, the check waited
600 s, the other checks up to 592 s, the route started at 606 s and the
board at 608 s; with the rule every check goes at once, the board still
starts at 608 s and the route at 600 s.
`a_short_request_needs_the_memory_and_never_passes_in_the_bookkeeping`
and test_run.py's `MemoryLedger` (the same simulation and numbers, and
`try_admit` on a file: a 500 MB request behind a drained-for waiter
waits, declared short it goes, marked `short`). Smoke (a private ledger,
`PCB_MAKER_LEDGER_TOTAL_MB` 8000, `PCB_MAKER_NO_CACHE=1`; a holder of
4000 MB since after the wait began; harness_common.admit waiting for
4500 MB with `drain_after` 0, `the ledger drains for it`; then
`drc-kicad-board` of the trial's little_red_rover layout): x1846 waited
`behind an older request, smoke waiter ... the ledger drains for it`
until its 40 s timeout, as did the branch's binary with
`PCB_MAKER_LEDGER_SHORT=0`; the branch's binary held its 2253 MB
(`[short]` in `pcb-maker ledger`) 0.8 s after it was started and ended
with exit 0 in 1.7 s, as long as the same check takes on an empty ledger.

The waiter's floor: `--min-free-gb` does not add up over harnesses. Each
harness compares the one MemAvailable of the machine, less the growth of
every reservation, with its own floor, so five harnesses with 10 GB each
keep 10 GB, not 50; a floor shared among them is what it is already, and
one divided by their number would only be a lower floor. What held the
board was the 19 GB outside the ledger together with the growth term,
which counts every reservation at its expected peak while most runs are
below theirs (18-21 GB still to grow beside 30-34 GB reserved): with the 10 GB floor the reservations could take about 34 GB
of the 56.7. A floor of 6 GB would have started the board at once
(9.4 - 2.9 = 6.5 GB) and left 6.5 GB for the memory outside the ledger to
grow into while every run peaks. Whether that is safe depends on how far
that memory moves (the browser, builds), which was not measured then; it
is now (next paragraphs): 6 GB is not enough, 8.8 GB is.

**The floor from measured headroom** (2026-10-27). run.py's floor of
available memory is no longer a number by opinion: unless
`--min-free-gb` is given (an explicit override, as before) it is, per
board,

- `FLOOR_KEEP_MB`, 2 GB: the MemAvailable never to go below (the
  kernel then reclaims and swaps hard);
- plus the memory outside the ledger's growth: the 0.95 quantile of how
  far it rose within 10 minutes of a sample (`HEADROOM_QUANTILE`,
  `HEADROOM_WINDOW_SECONDS`), over the samples of the last six hours;
- plus the board's tier's overshoot: the 0.95 quantile of measured peak
  less what the estimates declared, at least 0, per kind
  (estimates.json's `overshoot`; a kind without one falls back as the
  estimates do, short and schematic to layout; else 0);

and 10 GB (`DEFAULT_FLOOR_MB`) without an hour of samples
(`HEADROOM_MIN_SAMPLES`, 360) whose newest is at most a day old. The
waiting entries carry it (`floor_mb`), so the drain reads it as before.
`measured_floor_mb` in ledger.rs and harness_common.py (the same rule);
run.py prints it at the start (`memory floor: layout 8.8 GB (measured:
2.0 GB to keep + 6.7 GB outside growth + 0.1 GB overshoot (1091
samples))`) and reads it again on every poll of a wait.

*The sampler.* The samples live beside the ledger,
`<ledger>.headroom.json` (`$XDG_RUNTIME_DIR/pcb-maker-ledger.json.headroom.json`):
one every 10 s at most, whoever writes it, each `[time, MemAvailable, the
memory outside the ledger, reserved, still to grow, the holders' memory
above their reservations]` in MB (outside: MemTotal less MemAvailable less
the resident memory of every holder's process tree, a tree stopping at
another holder), and a `summary` the floor reads. run.py's MemoryGate
samples on a thread while it runs (so the nightly's stages and every
agent's run.py keep it fresh, and the window is the busy hours);
`python3 benchmarks/ledger_headroom.py --sample [--for S]` samples for
anyone else and `python3 benchmarks/ledger_headroom.py` prints the
summary and the floor per kind. One sample reads /proc once (about 30 ms
of CPU). `pcb-maker ledger` ends with run.py's floor before a tier's
overshoot. `tier_estimates.py --refresh` computes the overshoot: each
new source's rows against what the sidecar declared for them before it
(the newest three sources per kind); the committed sidecar's section is
the replay of its own twelve sources
(`experiments/ledger-floor/overshoot.py --sidecar`), its rows unchanged.
scripts/nightly.sh no longer passes `--min-free-gb 10`; ablation.py
passes it only when given.

*The measurement* (experiments/ledger-floor: sampler.py, samples.py,
overshoot.py, waits.py, replay.py). A read-only sampler every 10 s from
2026-10-26 23:39 to 02:41 (1091 samples, four agents and the nightly's
designer stage; MemTotal 62.7 GB, 32.5-33.4 GB of the 48 GB of swap in
use throughout):

| | p5 | p50 | p95 | p99 | max |
| --- | --- | --- | --- | --- | --- |
| outside the ledger (GB) | 16.7 | 18.1 | 23.1 | 25.5 | 27.5 |
| its rise within 1 min | 0.0 | 0.5 | 2.9 | 3.8 | 5.2 |
| its rise within 10 min | 0.1 | 2.5 | **6.7** | 8.1 | 8.9 |
| its rise within 30 / 60 min | 0.5 / 0.7 | 4.6 / 5.9 | 9.4 / 9.6 | 10.5 / 10.6 | 11.2 |
| MemAvailable | 24.6 | 30.4 | 36.3 | 38.6 | 40.4 (least 17.3) |
| MemAvailable less growth (the floor's view) | -1.4 | 10.3 | 18.3 | 22.5 | 26.2 |
| reserved / the holders' trees resident | 25.6 / 9.1 | 33.7 / 13.4 | 45.2 / 18.5 | 50.6 / 26.8 | 55.0 / 28.5 |

Outside the ledger: the browser 8.0-8.4 GB, the agents' builds (rustc
up to 7.5 GB, clippy-driver 1.9; 26 episodes in three hours; rustc grew
most in 333 of the 380 ten-minute rises above 4 GB, addr2line, up to
4.9 GB, in the other 47), kicad-cli 2.1-2.2 GB and the nightly's binary
1.3-2.1 GB not under any holder, the agents' own processes (claude 1.9-2.1,
codex 1.2), perf up to 2.0. Without the windows of this work's own two
builds the 10-minute p95 is 7.1 GB. The floor's view went below 0 at about
5 % of the samples (binaries keep no floor and admit by the capacity
alone) while MemAvailable never went below 17.3 GB: the growth term counts
every reservation at its expected peak, and the holders' trees held
13.4 GB beside 33.7 GB reserved at the median. No binary's process tree was above
its reservation at any sample (a harness's own reservation is left out:
its tree holds its boards' binaries, which hold their own). In the logs
of the week two routes were ended by the watchdog for growing past their
reservation with no room (ducktop2 in dp5-rules: 5398 past 5310 MB, 4618
past 4425).

The overshoot, a time-ordered replay of the committed estimates rule over
the sidecar's sources and the 2026-10-26 night (each source against the
estimates of the sources before it; peak less declared, MB):

| kind | rows | over | p50 | p95 | p99 | max |
| --- | --- | --- | --- | --- | --- | --- |
| designer | 333 | 16 | -385 | -27 | 973 | 2158 |
| expansion | 92 | 0 | -79 | -15 | -14 | -14 |
| layout | 327 | 18 | -855 | 52 | 1969 | 3110 (ULK, from the size model) |
| short | 36 | 0 | -195 | -67 | 0 | 0 |

The committed sidecar's section (its twelve sources, the newest three per
kind): layout 56 MB, designer 91, short and expansion 0. The 1.25 margin of
the estimates covers 95 % of the rows; the overshoot is the small part of
the floor. The floor from these: layout 8990 MB (8.8 GB), designer 9025,
short and expansion 8934.

OOM kills in the week (`journalctl -k`): on 2026-10-21 four pcb-maker
routes (before the ledger); on 2026-10-24 15:06 and 2026-10-25 18:10 the
kernel killed browser processes and two coredump helpers, no pcb-maker
run, with the swap (then 16 GB) full: an agent's copied binary
(dp4-exp-via: 21 GB in all; dp5-dev-bin5: 11.8 GB), the browser (12.1 and
8.8 GB), codex (7.3, 5.5) and 8.6 and 12.3 GB of shared memory (tmpfs)
outside the ledger. Whether those binaries kept to their reservations the
kernel's log does not say. Given the admissions that happened tonight a
floor lower by D lets the harnesses commit at most D more (each checks the
one MemAvailable less growth, and an admission lowers it at once), so with
the least MemAvailable of 17.3 GB any floor kept 2 GB.

*What the 10 GB floor cost.* The harnesses' waiting lines since
2026-10-22 (experiments/ledger-floor/waits.py over the 520 logs with
them): 3893 waits of run.py's boards, 3.27 million board-seconds; at the
wait's beginning the ledger had room in all but one, and the floor alone
held 3095 of them (2.89 million board-seconds; ordered admission and the
drain 797, 0.37 million). The slack then (MemAvailable less growth less
the board): p5 2.2, p50 8.6, p95 9.8 GB. A floor of 8.8 GB would have
started 1381 of them at once (0.54 million board-seconds of their
waiting), 6 GB 2597 (1.51 million). In tonight's samples run.py's boards
waited 18.9 board-hours, 13.1 of them held by the floor alone; at
8.8 GB 3.8 board-hours of that had room (1.2 at 10 GB: the 15 s look).

*The replays* (in both languages: ledger.rs `floor_replay`, test_run.py
`MeasuredFloor.replay`; the night's in full in replay.py). A harness's
boards against the machine as it was without them (the floor's view and
MemAvailable with their reservations and resident memory added back), a
look every 15 s, a board starting when the view less the boards running
less its expected peak leaves the floor, then holding its measured
memory:

| | floor | boards started | waited | last end | least MemAvailable |
| --- | --- | --- | --- | --- | --- |
| the evening (4 boards, from 20:37) | 10 GB | 4 | 14 220 s | 6095 s | 29.7 GB |
| | measured, 8.8 GB | 4 | 9 840 s | 5877 s | 28.9 GB |
| tonight's first hour (14 boards over 3 GB, 120 s steps) | 10 GB | 9 | 20 766 s | | 20.0 GB |
| | measured | 10 | 18 376 s | | 17.5 GB |
| tonight, three hours (123 boards, 10 s, their measured memory) | 10 GB | 123 | 10.1 board-hours | | 12.6 GB |
| | measured | 123 | 8.5 board-hours | | 9.2 GB |
| | 6 GB | 123 | 5.8 board-hours | | 9.4 GB |
| the same, each board resident at its expected peak | 10 GB | 114 | 37.4 board-hours | | 7.5 GB |
| | measured | 118 | 29.4 board-hours | | 4.1 GB |
| | 6 GB | 120 | 27.4 board-hours | | **1.4 GB** |

With 10 GB the evening's replay gives the evening's starts (2220, 3375,
4245, 4380 s against the log's 2210, 3364, 4235, 4371). The measured
floor keeps 2 GB in every replay, in the worst case too; 6 GB does not
(the finding's "6 GB would have started the board at once" holds for that
board alone; beside the night's other boards at their expected peaks the
machine would have come within 1.4 GB). With the outside memory's 10-minute
p95 subtracted once more at every second (a build beyond the night's own)
the measured floor's three-hour replay keeps 2.5 GB. The other runs are
replayed as they happened: boards that start sooner would have let the
drain end sooner too, not counted. Tests
`the_floor_is_measured_from_the_headroom_samples`,
`the_evening_replayed_with_the_measured_floor`,
`tonight_replayed_with_the_measured_floor` (ledger.rs) and test_run.py's
`MeasuredFloor` (the same numbers; and the sample of a process tree,
the samples' bookkeeping, the gate's floor and its sampler);
test_tier_estimates.py's overshoot. Routing and placement are unchanged.

What the measurement also said: the ledger's capacity (MemTotal less the
6 GB margin, 56.7 GB) was 11-21 GB more than the machine can give beside
the 16.7-27.5 GB outside it, binaries admitted by the capacity alone, and
kicad-cli (2.1-2.2 GB) and the nightly's binary (1.3-2.1 GB) ran outside
any holder; what kept MemAvailable high is that runs stay far below their
reservations. Changed the same day (next paragraphs).

**The capacity from the machine** (2026-10-27). The capacity is MemTotal
less the memory outside the ledger: the median of the headroom samples
(`summary`'s `outside_mb`, the samples of the last six hours) when they
are fresh (at least 360, the newest at most a day old: the floor's rule,
`headroom_fresh`), else 25 % of MemTotal (`DEFAULT_OUTSIDE_SHARE`); at
most MemTotal less the margin (`PCB_MAKER_LEDGER_MARGIN_MB`, 6 GB), the
static bound it was. Here: 64 210 - 18 534 = 45 676 MB (44.6 GB) from the
night's samples, 48 157 MB (47.0 GB) without samples. Every process
reads it from the samples beside the ledger (a binary every 30 s, a
harness on every poll); `PCB_MAKER_LEDGER_TOTAL_MB` still replaces it.
`machine_capacity_mb` and `capacity_described` in ledger.rs,
`machine_capacity_mb` and `ledger_capacity_described` in
harness_common.py (the same rule); `pcb-maker ledger` says what it is
made of (`48157 MB (MemTotal 62.7 GB less 15.7 GB outside the ledger (25 %
of MemTotal without fresh headroom samples); at most 56.7 GB, MemTotal
less the margin)`), run.py's start line too (`ledger capacity ...`).

*Another machine.* Without samples the capacity is the smaller of
MemTotal less 6 GB and three quarters of MemTotal: 10 GB of a 16 GB
machine, 18 of 24, 24 of 32, 96 of 128 (test
`the_capacity_is_the_machines_memory_less_what_is_outside_the_ledger`).
The quarter is below what this machine measured outside the ledger
(15.7 GB of 62.7 against the night's 16.7 GB p5 and 18.1 GB median), a
desktop with a browser and an editor beside the runs. The samples
replace it once run.py has run for an hour (its gate samples every 10 s),
or after `python3 benchmarks/ledger_headroom.py --sample --for 3600`; a
machine that runs only binaries keeps the quarter. A dedicated machine
with nothing beside the runs gets the static bound from its samples (its
outside memory is small).

*Binaries keep the floor.* A binary's new request (it holds nothing, and
no harness above it holds memory or names the reservation it runs
inside) now keeps the harnesses' floor beside it: the memory available
less what the reservations still grow by, less its request, must leave
the measured floor (`measured_floor_mb` of the samples without a tier's
overshoot: 8.8 GB here, 10 GB without samples), a short request
(`drc-kicad-board`, `place-kicad-board`, `inspect-routing-run
--native-check`, a check of a command armed on demand) the 2 GB to keep
(`FLOOR_KEEP_MB`): it holds its memory for seconds, and the 10-minute
rise of the outside memory is the long runs' term (with the measured
floor the 2026-10-26 evening's 1.5 s check, 7.4 GB beside the growth,
would have waited again). Alone (no other run holds memory) it goes, as a
harness's board. A binary held back by the floor waits in the ledger with
`floor_mb` and `waits_on_floor`, as a harness's board, and says so
(`memory ledger: waiting to reserve ...; the memory available less what
the runs still grow by, <X> GB, would leave less than the floor of
<Y> GB (...)`). A run that holds memory and grows (the
ladder, the watchdog's raise, a route's check) keeps no floor: held back
it would end with exit 4. `binary_floor_allows` in ledger.rs; the
harnesses' rule is `memory_room`, as before.

*Why the capacity changes no harness's wait.* A holder's growth counts
its own resident memory, which its process tree holds and the outside
memory does not, so at any time the other runs' reservations plus the
floor's view (MemAvailable less growth) are at most MemTotal less the
outside memory. A board the floor admits (the view less the boards
running less its own at least the floor) therefore fits a capacity of
MemTotal less the median outside memory whenever the outside memory is
at least its median less the floor: 18.1 - 8.8 = 9.3 GB. The night's
least was above 16 GB (p5 16.7; the browser alone 8 GB). The replays
(ledger.rs `the_evening_and_tonight_replayed_with_the_capacity_from_the_machine`,
test_run.py's test of the same name; `floor_replay` and
`MeasuredFloor.replay` with a capacity and the other runs' reservations
per knot):

| replay | floor | with the capacity of 44.6 GB |
| --- | --- | --- |
| the evening (the others' reservations from the log's lines: 35.3, 30.5, 28.6, 28.4, 25.2 GB) | 10 GB, measured 8.8 GB | the same starts (2220, 3375, 4245, 4380 s; 0, 3375, 2220, 4245 s), waits (14 220 s, 9840 s) and least MemAvailable (29.7, 28.9 GB) |
| tonight's first hour (the others' reservations at most MemTotal less 16.7 GB, the outside memory's p5, less the view) | 10 GB, measured | the same: 9 and 10 boards, 20 766 and 18 376 s, the least MemAvailable 20.0 and 17.5 GB |
| the same with the outside memory at 9 GB | measured | a board held back (the bound's case) |

None of the replays' admissions takes MemAvailable below 2 GB, and the
capacity adds no wait beyond the floor's. The night's samples (and their
three-hour replay of 123 boards) were not kept, so the three-hour replay
was not rerun; replay.py takes `--capacity` now.
Instead a fresh read-only sampler (sampler.py, every 10 s, 2026-10-27
03:17-03:56, 237 samples; four agents and the nightly's sweep): outside
the ledger 16.2-23.4 GB (median 17.3: a capacity of 45.4 GB), reserved
at most 43.4 GB (p50 35.3), MemAvailable at least 29.1 GB.
`replay.py --capacity` of its 21 run.py boards that only the floor held
back at some sample: with the capacity the same starts, waits (0.89
board-hours with 10 GB, 0.49 with 8.8 GB; at their expected peaks 0.95
and 0.71) and least MemAvailable (28.8 GB measured floor; 16.5 GB with
every board at its expected peak) as without it; no sample had more
reserved than the capacity. Of the 97 binaries that began holding then,
18 (all `probe-coarse-placement` of one agent's experiment, 1.0-1.2 GB
each, started with the floor's view at 2.6-9.7 GB) began below the floor
a binary keeps now: those would have waited.

**kicad-cli under a reservation** (2026-10-27). Every kicad-cli that
loads a board or a schematic runs inside its process's reservation
(`ledger::kicad_check`): DRC and zone fill (native_report.rs), the fab
exports (gerbers, drill, positions, BOM: fab_export.rs, fab_release.rs),
the verification preview's SVG, the netlist import's schematic export
and pad check, the research renderer. `kicad-cli version` (64 MB) is not
reserved.

- A command whose memory is its checks (`drc-kicad-board`,
  `place-kicad-board`, `inspect-routing-run --native-check`) reserves one
  check (`KICAD_CHECK_MB`, 2253 MB: the short rule's figure) up front,
  when it arms, and holds it until it ends; its checks run inside it
  (`reserve_check_up_front`). Until then it reserved when the check
  came: its own memory before (a designer board's 1-2 GB) ran outside.
- Any other command that runs kicad-cli (`export-kicad-fab`,
  `fab-kicad-board`, `import-kicad-netlist`, the research commands) arms
  on demand, as a short command, at its first check, with the check
  reserved (`arm_on_demand`, set by the executable for every command;
  the library's callers and tests reserve nothing). Until then these ran
  kicad-cli outside any reservation.
- A route and a layout raise their reservation by the check while it
  runs, as before (reserving 2.2 GB up front for an hour's route would
  take it from the other runs all that time).
- The entry says so: `check_mb` (the part held for checks) and `checking`
  (one runs); `pcb-maker ledger` prints `[KiCad's check: 2253 MB,
  reserved]` or `running`. `while_reserved` takes a check inside what the
  process holds for its children before it raises.

*The binaries of a run.py board inside its reservation.* run.py's gate
admits a board and hands its reservation to the board's binary when it
starts, as before; every other binary of the board now runs inside it:
the designer's DRC and strip before the run get `PCB_MAKER_LEDGER_INSIDE`
(`<pid> <started>` of the board's reservation; harness_common.run_group
adds it on the board's thread) and reserve only what they need beyond
it (a check of 2253 MB inside a board's 3000: an entry of 0 MB, `[inside
pid ...'s reservation]`). Once the board's binary has ended, the
binaries after it (the strict baseline's strip and DRC, a route's check
of its result) run inside a short reservation of 4301 MB (2048 + 2253:
`AFTER_RUN_MB`, `run.py: <board> (after the run)`) that the gate admits
at the first of them. Until then they ran outside the board's
reservation (the drc child reserved its check alone, strip-kicad-tracks
nothing). `MemoryGate.inside_environment`; test_run.py's
`test_every_binary_of_a_board_runs_inside_its_reservation`.

*Smoke* (a private ledger, the debug binary, `PCB_MAKER_NO_CACHE=1`, the
f1 fixture; the ledger read every 0.2 s by a second process): a harness
holding 3000 MB for the board started `drc-kicad-board` through
run_group; the binary's entry was `0 MB ... [KiCad's check: 2253 MB,
reserved] [inside pid ...'s reservation]` 0.3 s in, `running` from 0.5 s
to 1.6 s, gone at 1.9 s (exit 1, the board's own findings, in 1.9 s).
The same binary alone: `2253 MB ... [short] [KiCad's check: 2253 MB,
reserved]` at 0.1 s, `running` from 0.6 s, the ledger empty after.
`export-kicad-fab` of f1: `2253 MB export-kicad-fab f1 ... [short]
[KiCad's check: 2253 MB, running]` for its 5 s, exit 0. On the live
ledger (read only by `pcb-maker ledger`, a check of f1 by the branch's
binary): `39101 MB reserved of 48157 MB (MemTotal 62.7 GB less 15.7 GB
outside the ledger (25 % of MemTotal without fresh headroom samples); at
most 56.7 GB ...) by 26 runs` and `drc-kicad-board f1 ... [short]
[KiCad's check: 2253 MB, running]` 0.6 s into the check. Routing and
placement are unchanged.

**A board's reservation is its binary's, and a run past it waits**
(2026-10-27). In the nightly of 2026-10-27 the sweep's Quanta75
(`obsilab__Quanta75__Quanta75_RP2040Stamp_JLCPCBAoptimized`) lost its
strict pass with exit 4: `the process grew to 1474 MB resident, past its
reservation of 1415 MB ... and the ledger had no room for more`. run.py
had admitted it with 2.4 GB (sweep.err: `2.4 GB expected`; estimates.json,
its peaks 1582-1968 MB). The cause: run.py's gate hands the board's
reservation to the board's binary (`harness_common.Ledger.move`
re-points it to the binary's pid), and the binary's first reservation
replaced that entry with its own estimate (`try_hold`: "a harness's
reservation for this process gives way to the process's own"). Measured
(x1860, a private ledger of 100 000 MB, run.py `--expected-gb 2.4`, the
ledger read every 0.2 s): 2458 MB until 58 s, then the binary's own
1331 MB, 1405 MB when it began routing (385 s), raised by the watchdog to
1686 and 2245 MB as it grew (its own peak 1523 MB). In the night 21 runs
waited (`33.8 GB reserved by 21 run(s) of 45.2 GB, 21 waiting`): the
room it gave back was taken, and its raise past 1415 MB found none.
Neither the KiCad check's share (a check is reserved on top of the
process's own limit) nor the drain (it never holds back a run that holds
memory) took part.

- *The hand-over is the least of the binary's own* (`granted_mb`,
  `granted_of` in ledger.rs). `Ledger.move` writes the reservation's
  megabytes as `granted_mb` on the moved entry, and the binary's own limit
  is at least that: its entry keeps the row's reservation while its own
  estimate is smaller, and grows past it as before (the ladder, the
  watchdog); a KiCad check comes on top, as on top of its own estimate
  before. A hand-over of a harness before this change (no `granted_mb`)
  is recognised as an entry under the binary's pid that the binary did not
  write (another label or `started`). Where the binary had reserved before
  the move, the move used to drop the harness's reservation; now the
  binary's entry gets the larger of the two and the grant. `pcb-maker
  ledger` notes `[at least the 2458 MB its harness reserved]`.
- *A run past its reservation waits* (`start_watchdog`, `overgrown_ends`).
  A run whose resident size passes its reservation while the ledger has
  no room for the raise (1.2 times its resident size) no longer ends at
  once: it runs on, and its entry waits for the raise (`waiting_mb`) as
  of when it began holding (`waiting_since` its `held_since`: its request
  is as old as its admission), so it comes before the waiters that began
  waiting after it was admitted; newer requests leave it its room
  (`order_allows`), and since its wait is older than `drain_after` the
  ledger drains for it while it could start it (`drain_state`). It asks
  again every 2 s, as a holder by the capacity alone (`may_hold`: runs
  that hold memory are not held back by the order or the drain), and goes
  on as soon as a run ends. It ends with exit 4 only when the machine's
  memory available falls below `FLOOR_KEEP_MB` (2 GB, at once: the kernel
  would otherwise end a process, maybe another one), or once it has waited
  `GROWTH_WAIT_SECONDS` (600 s, the drain's `drain_after`) and the memory
  available less what the other runs still grow by is below 2 GB or
  unknown. Otherwise it goes on past its reservation: every other
  admission sees its memory in MemAvailable. stderr says `memory ledger:
  the process grew to ... it runs on and waits for the room`, then
  `reserved ... after ... s past the reservation` or `back within the
  reservation`; the verdict's error of a run that still ends adds `it
  waited ... s for room and ended because ...`.
- *run.py retries a row the ledger ended* (`ledger_ended`, `run_task`):
  exit 4 with `past its reservation of <n> MB in the memory ledger` in its
  error (binaries before this change, or a run that the rule above still
  ends) runs once more, admitted again by the gate (it waits for the
  ledger's room as any board) with its expected peak or 1.25 times what
  it reached, whichever is more. The row carries `ledger_retry` (the
  first run's error, seconds, CPU seconds, peak and the new reservation);
  `summary.json` counts `ledger_retries` and `ledger_killed`, and run.py
  prints them. A row the ledger ends twice is `killed` (`cut:
  ["killed"]`): no result, the machine's doing, so compare.py leaves it
  out of better and worse. nightly-report.py lists the retried rows under
  "Cut and killed rows" (`ended by the memory ledger and retried once`)
  and in the stage's cell; a row ended again counts as killed there (the
  night is not published).

Tests: ledger.rs `a_harness_s_hand_over_is_the_least_the_process_holds`,
`a_run_past_its_reservation_waits_for_room_and_ends_only_short_of_memory`,
`a_run_past_its_reservation_waits_in_order_as_of_its_admission` (the
night in small: a 45 200 MB ledger, the grower holding 1415 MB and waiting
for 1769 is the first waiter before a board that began waiting after its
admission, a 2000 MB request goes beside 33 800 MB reserved and waits
beside 43 300, the ledger drains for it); cli_exit_codes.rs
`a_harness_s_reservation_handed_to_the_binary_is_its_own` (a route
handed 3000 MB holds at least 3000 with `granted_mb`); test_run.py
`test_the_reservation_goes_with_the_binary_it_was_made_for` (the grant,
the merge) and `test_a_row_the_ledger_ended_is_retried_once_when_it_has_room`
(the night's message; admitted with 1000 MB, retried with 1841; ended
twice: killed); test_nightly.py (the retried row in the summary).

*Reproduction* (Quanta75 and the 0xCB 1337 v4.0 through run.py, `--jobs
2 --threads 2`, a private ledger of 6000 MB, `PCB_MAKER_LEDGER_TOTAL_MB`;
the night's other runs as a filler, a sleep process whose entry takes
every megabyte the others leave free once Quanta75's binary holds its own
entry, and gives its room back for 10 s whenever a run has waited 60 s):

| | binary, `--expected-gb` | Quanta75 | its result |
| --- | --- | --- | --- |
| before | x1860, 2.4 | its entry fell from 2458 to 1331 MB, the filler took the rest; exit 4 at 901 s, `grew to 1378 MB resident, past its reservation of 1376 MB`; run.py retried it with 2458 MB: strict pass | 162/162, strict pass |
| the hand-over | branch, 2.4 | its entry 2458 MB (`granted_mb`) throughout, never past it; its KiCad check waited 4 s for room | 162/162, strict pass |
| the wait | branch, 1.0 (the grant below its own estimate) | `grew to 1407 MB resident, past its reservation of 1373 MB ... it runs on and waits for the room`, back within it after 28 s; its raises waited 62 s each for a run to end | 162/162, strict pass |

The result boards of every row (Quanta75's and the 1337's) are byte
identical to each other and to x1860's on a roomy ledger. The short tier
(12 boards) with x1861 (main 58e31c0b) and the branch on it: 12 of 12
result boards byte identical, the same objectives (unconnected 20,
vias 1245, length 21 565 mm, work 2489 s). Routing and placement are
unchanged.

**The drain past its bound, and for the waiter behind one it does not
drain for** (2026-10-28). On 2026-10-28 `pcb-maker ledger` said `first
waiter: pid 3306359, run.py: video (expected peak), waiting 19201 s for
4389 MB: not drained for: it has waited past the drain's bound of 7200
s`: two 4.4 GB boards of one agent had waited 5.3 and 3.2 hours while
smaller requests went ahead. Their floor held them back (the ledger had
the room: `waits_on_floor`), and ordered admission passes such a waiter
by the capacity alone, so only the drain kept their room, and the drain
ended at `drain_max`: past it a waiter was worse off than a fresh one.
Later the same day the nightly's designer stage (from 14:29) put a second
cause beside it: its waiters come first (`priority: nightly`), and its
small boards were held back by their own floor (`poly_kb_molecule_4x4`,
487 MB with a floor of 7128 MB beside 6922 MB of memory available less
growth: `cannot start`), so the first waiter was never drained for and
the drain went to nobody, whatever the bound, for as long as the nightly
ran. Now:

- *The drain goes on past the bound, strictly* (`drain_state` gives
  `PastBound`, which drains: `Drain::drains`). Past its `drain_max` the
  ledger drains for a waiter as for the nightly's: a newer request goes
  only if it leaves the waiter its room (or is small while a larger run
  blocks the waiter anyway: the feasibility cap), not on an earlier
  expected end (harness_common's try_admit passes no `expected_end` for
  such a waiter; binaries never pass one). `pcb-maker ledger` says `the
  ledger drains for it strictly: it has waited past the drain's bound of
  7200 s`. What ends a drain is unchanged otherwise: a stale entry, and a
  waiter the ledger could not start even with every run since its wait
  began gone (`cannot start`), the guard that had made the bound
  necessary on 2026-10-26 (the 27-hour waiter of 24.7 GB).
- *The drain is for the first waiter it can drain for* (`drained_for`):
  of the waiters ahead of a request, in their order (the nightly's
  first, then by when their wait began), the first that is drained for.
  A waiter ahead that is not (not yet, stale, cannot start) keeps its
  place by ordered admission (`order_allows`, unchanged) and no longer
  stops the drain for the waiters behind it. The nightly's priority
  stands: its waiters rank first, a drain for one of them comes first,
  and the nightly's own new requests rank before every other waiter, so
  a drain for another waiter never holds them back. `pcb-maker ledger`
  adds `drained for: pid ...` when that waiter is not the first.
- *A request larger than the capacity ends.* It runs only alone
  (`may_hold`) and no newer request passes it (`order_allows`); the drain
  cannot start it. Once it has waited `drain_max` (7200 s, run.py's
  `--ledger-drain-max`) for the ledger to empty, it ends instead of
  waiting for ever: a binary with exit 4 and a verdict naming the
  capacity (`too_large_ends` in `hold`), a harness's board with
  `harness_common.TooLarge` (run.py's row: `runner: TooLarge: memory:
  ... waited 7200 s for <n> MB, more than the memory ledger's capacity of
  <c> MB for every run: it can run only alone, and the ledger did not
  empty ...`).

Tests (ledger.rs; test_run.py's
`test_a_waiter_past_the_bound_is_drained_for_strictly_and_before_newer_ones`
the same numbers and `admit` raising `TooLarge` on a file):
`a_waiter_past_the_drain_s_bound_still_goes_before_newer_smaller_requests`
(43 084 MB; 33 389 MB reserved by nine runs that began after the video
board's wait; 6922 MB available less growth; the video board waiting
19 201 s for 4389 MB with a floor of 7184 MB: drained for, strictly; a
1536 MB request that ordered admission lets go waits; once 13 000 MB are
available less growth the board goes and the 1536 MB request beside it
only at 13 200), `the_nightly_s_requests_still_go_first_beside_a_waiter_past_the_bound`
(the nightly's 487 MB board first and `cannot start`: the drain is the
video board's; startable at 8000 MB, the nightly's is drained for first
and a 1536 MB request waits for it), `a_waiter_larger_than_the_capacity_ends_after_the_drain_s_bound`;
`the_ledger_drains_only_for_a_waiter_it_could_start` now drains past 7200 s
(the 7 GB request waits). On the live ledger (read only) the rule named
the video board drained for. The simulations of the drain, the cap and
the short requests give the same numbers as before. Routing and
placement are unchanged.

Older binaries and harness copies ignore the new fields (`floor_mb`,
`drain_after`, `expected_end`, `priority`) and do not drain; an older
binary that rewrites the ledger drops them until the harness writes its
waiting entry again (every poll, 15 s). A binary of this change keeps the
fields it does not know when it rewrites the ledger.

The rule is `order_allows` in crates/pcb-kicad/src/ledger.rs and in
benchmarks/harness_common.py (the same rule; tests
`the_request_that_waited_longest_is_not_passed` and test_run.py's
`MemoryLedger`). Why: a large board waiting for memory was passed by every
smaller one that arrived while memory freed; a designer-tier run got 3
rows in 75 minutes beside another agent's stream of small layouts. In the
two-process smoke (4000 MB in all, a holder of 3000, a harness waiting
for 3500, then a route of 773 MB that fits beside the holder but would
leave the harness short): x1754's route went at once and ended 5 s
before the harness got in; the branch's route waited "behind an older
request" and reserved after the harness's run released. Binaries
before the branch and older harness copies neither leave waiting entries
nor respect them, and an older binary that rewrites the ledger drops the
new fields (`started` stands in for both times). The expansion intake's
triage runner (`benchmarks/expansion/triage.py`) admits by its own rule,
not ordered.

## Per-board estimates for every tier (2026-10-26)

run.py's memory gate reserves each board's expected peak in the ledger,
and `--order peak-desc` starts the largest first. Until 2026-10-26 only
the tiers' files carried estimates (designer.json, quick.json, ...): the
layout sweep's task list has none, so run.py declared one
`--expected-gb` for every board and the v21 sweep ran in three parts to
keep each declaration above the board's measured peak (5 boards at 4 GB,
7 at 8 GB, 97 at 3.5 GB; "Layout sweep v21" below), and `--order
peak-desc` did nothing there.

Now `benchmarks/agent-tasks/estimates.json` (committed) gives every
board of tasks/v7 and the expansion list its expected peak and seconds
per kind of run (layout: the sweep, the quick tier, `--route-from`;
short, schematic, designer, expansion), and run.py reads it by default
(`--estimates` another; `--expected-gb` on the command line overrides
every board's). `tier_estimates.py --refresh` writes it from run.py logs:
per board and kind the rows of the newest three sources that measured
it (spliced rows left out), the estimate their largest `peak_rss_mb`
times 1.25 and their largest seconds. The margin: the matched designer
tiers of x1832 and the 2026-10-25 nightly (x1817) differ by a factor of
1.16 at the median and by at most 1.25 on 86 % of their 106 common
boards. A board without a row of its kind takes, in order, its layout
rows' estimate (short, schematic, designer), the tier file's own entry,
the size model (per family, layout or route: a line over pads x copper
layers fitted to the measured peaks, raised to their 90th percentile,
times 1.25; for the first file 3223 MB + 0.054 MB a pad and layer for a
layout, 1350 MB + 0.19 MB for a route), then 3 GB. `run.py --dry-run`
prints the start order with each board's declaration and its source;
every run prints the sources counted on stderr.

The first file's sources: the v21 sweep and its `-ab` part, the sweeps,
designer, short and expansion stages of the nightlies of 2026-10-24 and
25, `build/designer-x1832-matched.log` and the schematic tier's
`build/sch-x1250.log`. All 155 boards have a measured row of their
kind (layout 109, designer 109, short 12, schematic 12, expansion 46);
the size model estimates none of them (it covers boards outside these
lists, e.g. tasks.json's). The sweep's declarations: median 1779 MB,
16 boards above 3.5 GB, the largest ULK's 8982 MB (7185 MB measured by
the 2026-10-24 nightly; v21's 4644 MB is the newest of its three rows);
the designer tier's median 1073 MB, the largest link's 7564 MB.

**The refresh.** After a sweep or a tier run whose rows should count:
`tier_estimates.py --refresh --layout <log>` (`--designer`, `--short`,
`--schematic`, `--expansion`, `--night build/nightly/<date>`; without a
log, the newest night), then commit the file. scripts/nightly.sh keeps
its own copy, `build/nightly/estimates.json`: every stage declares from
it (the first night from the committed file), and at the end of the
night the night's rows are added to it with the committed file's rows
merged in, so the estimates follow the nightly binary within three
nights (`build/nightly/<date>/estimates.log`). The nightly's sweep now
starts its largest boards first (`--order peak-desc`). Tests:
test_tier_estimates.py (the window, the margin, spliced rows, a source
read again, the size model, the fallbacks, the committed file covering
every tier's boards), test_run.py `Estimates` (a dry run: loading, the
size model on a board's file, the default, `--expected-gb`; the
estimates are not part of a task's digest) and test_nightly.py (the
second night's stages read the first night's refresh). The ledger's
rules and simulations are unchanged; routing is untouched.

## Fence (2026-10-21)

Every breadboard-fence benchmark run once with x1404 (main at the fence
report's router side) and, where this pass changed something, with
x1462 (branch fence-benchmark): the table is in
[benchmarks/fence/README.md](../benchmarks/fence/README.md#results)
("benchmark pass" rows), with vias in pads by pcbnew's HitTest (0 on
every board) and the GND pour by KiCad's fill (`pour-pieces.py`) and by
the router's pour model (the verdict's new `pours`). In short: v5 four
layers complete and clean; v5 two layers at 34 mm 5 unconnected; at 46 mm
every connection with one starved thermal, and complete and clean with
a `keep_whole` strip that grew the pour's largest piece from 293 to 373
mm² (at 44 mm, the free dimension's size: 389 to 630 mm², clean both); v1 two layers 100/100 for the first time (one starved thermal);
the corpus's v4 four-layer boards complete and clean in both modes.

## Results (route column 2026-09-22 evening, layout column earlier that day; one thread)

Tracks and vias are stripped; copper pours stay (where copper is poured is a
design decision like the placement). `pours connect` means pads reached their
pour through stubs, vias and stitching; `pours connect+skeleton` adds the
fixed plane skeleton; `pours tracks` means the pour nets were routed as
tracks because connecting through the pour left something open (the
[attempt ladder](router.md#the-attempt-ladder) tries these in turn and keeps
the best board). Route times are the winning attempt's routing, with the
whole ladder's wall time including native checks in parentheses; place +
route times are the whole coupled loop including its routing probes and the
final native check. "Clean" means no unconnected items and no copper
DRC error beyond what the stripped source or the designer's own board has.

| Board | Reference copper | Route (designer placement) | Place + route (automatic) |
| --- | --- | --- | --- |
| ecc83 | 59 tracks, 0 vias, 1 zones | 9/9, 0 vias, 160 mm, 0.19 s (ladder 6 s), pours connect — clean | 9/9, 0 vias, 140 mm, 5.8 s, pours connect — clean |
| hierarchy | 364 tracks, 0 vias, 166 zones | 50/50, 0 vias, 1003 mm, 1.65 s (ladder 9 s), pours connect — clean | 50/50, 0 vias, 1119 mm, 10.0 s, pours connect — clean |
| pic | 370 tracks, 6 vias, 1 zones | 34/34, 2 vias, 1520 mm, 8.18 s (ladder 15 s), pours connect — clean | 34/34, 6 vias, 2100 mm, 23.0 s, pours connect — clean |
| interf-u | 731 tracks, 84 vias, 1 zones | 110/110, 28 vias, 4735 mm, 75 s (ladder 233 s), pours tracks — clean | 110/110, 63 vias, 4674 mm, 361.8 s, pours tracks — clean |
| olimex-c3 | 768 tracks, 88 vias, 160 zones | 34/34, 49 vias, 965 mm, 19 s (ladder 67 s), pours tracks — clean | 34/34, 61 vias, 912 mm, 28.8 s, pours tracks — clean |
| dut-c3 | 321 tracks, 57 vias, 2 zones | 42/42, 25 vias, 1102 mm, 23 s (ladder 30 s), pours connect — clean | 42/42, 23 vias, 956 mm, 17.3 s, pours connect — clean |
| dut-c6 | 359 tracks, 68 vias, 2 zones | 42/42, 32 vias, 1034 mm, 26 s (ladder 32 s), pours connect — clean | 42/42, 24 vias, 1020 mm, 14.4 s, pours connect — clean |
| dut-s2 | 432 tracks, 83 vias, 2 zones | 46/46, 44 vias, 1252 mm, 38 s (ladder 86 s), pours connect+skeleton — clean | 46/46, 51 vias, 1356 mm, 24.3 s, pours connect — clean |
| dut-s3 | 392 tracks, 64 vias, 2 zones | 46/46, 38 vias, 1147 mm, 39 s (ladder 45 s), pours connect — clean | 46/46, 42 vias, 1144 mm, 17.7 s, pours connect — clean |
| dut-esp32 | 345 tracks, 64 vias, 2 zones | 46/46, 41 vias, 1173 mm, 30 s (ladder 36 s), pours connect — clean | 46/46, 43 vias, 1446 mm, 29.7 s, pours tracks — clean |
| ngdevkit (designer never finished it; headers fixed by `corpus.json`) | 115 tracks, 29 vias, 2 zones | 153/180, 1060 vias, 18 730 mm, 1098 s (negotiation capped at 900 s), pours connect — **151 unconnected**; starved_thermal×41 | 161/180, 741 vias, 18 567 mm, 2566 s, pours tracks — **90 unconnected**; starved_thermal×8 |
| sonde-xilinx | 208 tracks, 3 vias, 1 zones | 26/26, 2 vias, 575 mm, 3.42 s (ladder 18 s), pours connect+skeleton — clean | 26/26, 1 vias, 789 mm, 15.3 s, pours tracks — clean |
| multichannel | 576 tracks, 29 vias, 2 zones | 79/79, 18 vias, 2090 mm, 40 s (ladder 47 s), pours connect — clean | 79/79, 39 vias, 2077 mm, 52.5 s, pours tracks — starved_thermal×1 |
| stickhub | 1113 tracks, 87 vias, 5 zones | 44/45, 56 vias, 572 mm, 25 s (ladder 198 s), pours connect — **1 unconnected**; solder_mask_bridge×8 | **placement not legal** |
| coldfire (4 layers, 209 nets) | 2935 tracks, 253 vias, 3 zones | 208/209, 198 vias, 9155 mm, 308 s (ladder 621 s), pours tracks — **11 unconnected** | 208/209, 207 vias, 8890 mm, 1189 s — 12 unconnected |
| video (4 layers, 371 nets, 189 footprints) | 7932 tracks, 808 vias, 2 zones | 362/371, 669 vias, 34180 mm, 936 s (ladder 950 s), pours connect — **45 unconnected**; starved_thermal×8 | timed out |
| openair-max (4 layers, 120 nets, 210 footprints) | 1638 tracks, 410 vias, 34 zones | 120/120, 114 vias, 4365 mm, 180 s (ladder 467 s), pours tracks — solder_mask_bridge×2 | **placement not legal** |
| tiny_tapeout (4 layers; its source fails KiCad DRC: 8 clearance errors, 3 shorts) | 2143 tracks, 405 vias, 2 zones | 78/108, 139 vias, 7053 mm, 189 s (ladder 583 s), pours tracks — **42 unconnected** | **placement not legal** |

The ngdevkit row is from 2026-09-23 (after the cutout, edge-stub and
unrouted-pad clearance fixes): no clearance or internal findings remain, only
the designer's own USB-C shell pads on the edge; completion varies between
runs (153–160 route, 161–173 layout) because the ladder's budgets are
wall-clock based.

Twelve of the thirteen two-layer boards are complete and clean in route
mode (StickHub is one pad short with its pours kept; on the cold board it
completes on the 0.075 mm rung), twelve in both modes. Of the four-layer
boards, openair-max (210 footprints) is complete, ColdFire is one net short,
video nine: video's single pour-connect attempt runs into the 900 s
negotiation cap and the ladder does not try the pour-as-tracks rung after an
attempt that slow (an earlier build reached 364/371 that way), and
tiny_tapeout's source fails KiCad's own checks.

## What the failures say

- **StickHub** (16 x 40 mm, two-sided SMD, 0.15 mm rules): on the regular
  0.1 mm lattice the 17-pin +3.3 V net does not fit once the other nets are
  in; the 0.075 mm rung completes the cold board (45/45, 47 vias, 41 s) and
  with the small +3.3 V pour regions kept the 0.05 mm rung gets to one open
  pad. Since 2026-09-29 automatic placement is legal (bodies closer to
  the edge as their copper allows, legalization retried with the stuck
  parts first). Placed and routed it reaches 41/45, with solder-mask
  bridges as in route mode.
- **ngdevkit** (174 x 134 mm, 186 footprints, about 1070 pads, two layers,
  0.15 mm escape rules): two separate problems. First, its PSRAM is a 48-ball
  0.75 mm BGA whose inter-ball channels leave a 0.15 mm track 0.01 mm of
  slack, so the lattice must put a node row exactly mid-channel — a 0.1 mm
  lattice cannot, which made the inner balls unreachable and every net to
  them a permanent failure; the lattice choice now scores such tight channels
  and picks 0.125 mm (see [router.md](router.md)). Second, with the balls
  reachable, the negotiation still does not converge: 50-90 nets stay in
  conflict at the present-cost cap, and the congestion history piles up on
  the front layer at the BGA (x 145-151, y 78-84 mm), where the 16-bit data
  and 20-bit address buses from the level translators, the expander bus and
  the BGA escape all meet. The ground pour (224 pads on both layers) is
  shredded into 1500 pieces on the way; a plane skeleton keeps its pads
  connected but does not make the signals fit. As placed by its (unfinished)
  designer the board is most likely not routable on two layers; it is the
  test case for automatic re-placement, not for the router alone. With the
  53 headers and the ESP32 module held (the headers are the ribbon
  interface; the module sits at its antenna keepout) and everything else
  re-placed, the coupled loop gets to 173/180 in 43 minutes — better than
  the designer's placement, still not a board.
- **ColdFire / video** (4 layers): the remaining opens are ground pads whose
  inner-plane island cannot be stitched back to the main plane; same cause as
  ngdevkit, on inner layers.
- **Automatic placement on dense two-sided boards.** Since 2026-09-29
  StickHub and openair-max place legally. tiny_tapeout still leaves one
  connector (J4) without a place, with its front 78 % full.

## Coupled loop (2026-09-22, later the same day)

`layout-kicad-board` now nudges congested footprints and reroutes
incrementally instead of re-placing from scratch
([coupling.md](coupling.md)). Place + route results, same boards:

| Board | Halo rounds (old) | Incremental moves (new) |
| --- | --- | --- |
| ecc83 | 9/9, 0 vias, 140 mm, 6 s | 9/9, 0 vias, 140 mm, 6 s |
| hierarchy | 50/50, 0 vias, 1119 mm, 10 s | 50/50, 0 vias, 1194 mm, 29 s |
| pic | 34/34, 6 vias, 2100 mm, 23 s | 34/34, 4 vias, 1920 mm, 480 s |
| interf-u | 110/110, 63 vias, 4674 mm, 362 s | 110/110, 56 vias, 4399 mm, 235 s |
| olimex-c3 | 34/34, 61 vias, 912 mm, 29 s | 34/34, 65 vias, 960 mm, 19 s |
| dut-c3 | 42/42, 23 vias, 956 mm, 17 s | 42/42, 22 vias, 991 mm, 67 s |
| dut-c6 | 42/42, 24 vias, 1020 mm, 14 s | 42/42, 24 vias, 985 mm, 35 s |
| dut-s2 | 46/46, 51 vias, 1356 mm, 24 s | 46/46, 44 vias, 1317 mm, 205 s |
| dut-s3 | 46/46, 42 vias, 1144 mm, 18 s | 46/46, 31 vias, 1130 mm, 64 s |
| dut-esp32 | 46/46, 43 vias, 1446 mm, 30 s | 46/46, 42 vias, 1217 mm, 22 s |
| sonde-xilinx | 26/26, 1 via, 789 mm, 15 s | 26/26, 1 via, 789 mm, 13 s |
| multichannel | 79/79, 39 vias, 2077 mm, 52 s | 79/79, 42 vias, 1664 mm, 14 s |

All clean in native KiCad (multichannel keeps one starved thermal spoke in
both). Fewer vias or less copper on nine of twelve boards; the price is time on
boards where many nudges are tried.

## Head-to-head with Freerouting and tscircuit (2026-09-22)

`run.py --freerouting benchmarks/corpus/freerouting.json --tscircuit` routes
every two-layer board with three routers on the same *cold* board (tracks,
vias and pours stripped, since the Specctra exchange carries no pours), with
the same KiCad rules, and judges all three with the same native KiCad DRC
minus what the source already had:

- **pcb-maker**, one thread (plus parallel batches where nets are disjoint).
- **Freerouting 2.2.4** through the project's adapter
  (`route-kicad-board-freerouting`), which translates the board minimum into
  the DSN class rules and the edge clearance; one thread, optimizer off.
- **tscircuit capacity-autorouter 0.0.919** (the other actively developed
  open-source PCB autorouter) through `benchmarks/tscircuit/route_dsn.mjs`,
  which turns the same DSN into its SimpleRouteJson (pads as rectangles, nets
  as pad centres, the board's width, clearance and via rules through the
  solver's fields) and writes a Specctra session for the same import.

"Bad" counts KiCad DRC errors plus unconnected items.

| Board | pcb-maker | Freerouting | tscircuit |
| --- | --- | --- | --- |
| ecc83 | 9/9, 0 vias, 249 mm, 0.1 s, clean | complete, 0 vias, 253 mm, 1.5 s, clean | complete, 0 vias, 273 mm, 3.4 s, **7 bad** |
| hierarchy | 50/50, 0 vias, 1330 mm, 1 s, clean | **1 open**, 0 vias, 1377 mm, 7.8 s | complete, 18 vias, 1434 mm, 12 s, **10 bad** |
| pic | 34/34, 1 via, 1911 mm, 4 s, clean | **1 open**, 0 vias, 2101 mm, 7.2 s | complete, 16 vias, 2005 mm, 30 s, **180 bad** |
| interf-u | 110/110, **30 vias**, 4654 mm, 109 s, clean | complete, 44 vias, 5051 mm, 44 s, clean | **timed out** (25 min) |
| olimex-c3 | 34/34, 44 vias, 936 mm, 8 s, clean | **16 new hole-clearance errors** (DSN drops local clearances), 24 s | **61 open**, 101 vias, 84 s, 407 bad |
| dut-c3 | 42/42, 27 vias, 1295 mm, 7 s, clean | **2 open**, 23 vias, 1322 mm, 14 s | complete, 77 vias, 1377 mm, 27 s, **12 bad** |
| dut-c6 | 42/42, **26 vias**, 1274 mm, 22 s, clean | complete, 29 vias, 1355 mm, 14 s, clean | complete, 71 vias, 1301 mm, 10 s, clean |
| dut-s2 | 46/46, **34 vias**, 1492 mm, 17 s, clean | complete, 36 vias, 1582 mm, 13 s, clean | complete, 109 vias, 1548 mm, 20 s, clean |
| dut-s3 | 46/46, 33 vias, 1409 mm, 27 s, clean | complete, **31 vias**, 1472 mm, 11 s, clean | **1 open**, 108 vias, 1463 mm, 96 s, 8 bad |
| dut-esp32 | 46/46, **30 vias**, 1401 mm, 24 s, clean | complete, 35 vias, 1449 mm, 14 s, clean | complete, 90 vias, 1408 mm, 15 s, **1 bad** |
| sonde-xilinx | 26/26, 2 vias, 700 mm, 1 s, clean | complete, **0 vias**, 757 mm, 3.1 s, clean | **4 open**, 25 vias, 883 mm, 16 s, 52 bad |
| multichannel | 79/79, 20 vias, 2578 mm, 75 s, clean | **180 open** (via class below board minimum; suspected setup problem, see [todo](todo.md)), 16 s | **81 open**, 130 vias, 20 s, 1194 bad |
| stickhub | 43/45, 53 vias, 752 mm, 13 s, **4 open** | **2 open**, 44 vias, 851 mm, 66 s | **226 open** (routing failed), 20 s |

Clean completions: pcb-maker 12 of 13, Freerouting 8 of 13, tscircuit 3 of 13
(dut-c6, dut-s2 and ecc83 apart from its 7 findings). Copper: pcb-maker
least on every board. Vias among the boards pcb-maker and Freerouting both
complete: pcb-maker fewer on four, Freerouting on two, equal on one; tscircuit
uses two to four times as many. Time: Freerouting is the fastest on the
mid-size boards; pcb-maker's via-reduction phase (three renegotiation rounds)
is what it spends its extra time on and can be switched off
(`via_reduction_rounds: 0`), which makes it 3-10x faster than either at
20-40 % more vias. StickHub remains the one board where Freerouting gets
further than pcb-maker.

tscircuit's numbers should be read with care: the bridge is new, the solver
has no trace-to-trace clearance field (spacing is encoded as width plus
clearance), and its own benchmarks are tscircuit-native boards rather than
KiCad projects. It is included because it is the other open-source router
being actively developed.

With pours kept (pcb-maker's normal route mode, which neither external router
can run) the same boards route with the same completion, fewer vias on most,
and 15-30 % less copper.

## Strength: the same design on a smaller board

`run.py --shrink` (default factors 0.9, 0.8, 0.7, 0.6, 0.5) scales a board's
outline and every position on it towards the centre by a factor, keeps part
sizes, pushes parts back inside the outline where they would stick out, and
places and routes the result from scratch (`pcb-maker shrink-kicad-board`
does the transformation alone). Only the designer's own findings are
forgiven. The table's "smallest clean board" column is the last factor that
routed clean, with what stopped the next one: the strength of the
place-and-route flow in one number per board. The limit is often physical
before it is algorithmic — dut-c3 at 0.8 puts a 50 mm header on a 52 mm
board, into the corner mounting holes. Per-board placement constraints
(`"layout"` in `corpus.json`, e.g. ngdevkit's headers) apply to the shrunk
boards as well.

## Growing the corpus

The KiCad demo repository also has two very large boards (jetson-agx-thor, vme-wren: 1100-1500
footprints, 10-12 layers) as long-term targets. More open boards (Olimex and
others) can be added to `corpus.json`; boards whose source fails KiCad's checks
for other reasons than the baseline subtraction covers should be skipped.

## Layout sweep v11 on the harvested boards (2026-10-09)

109 boards, each reduced to what an agent starts from (parts stacked, the
designer's placement gone, a `constraints.json` derived from it: fixed
connectors, edges), laid out with `pcb-maker-x136` under 1800 s per board,
three at a time; boards killed or panicked rerun on x137-x146. A pass is
KiCad's verdict: 0 unconnected, no copper error beyond the designer's own
board, constraints met. **38 of 109 pass.** The table compares with the
x112 baseline (29 boards both have; before / after as routed, KiCad
unconnected, `+Ne` copper errors, seconds):

    benchmarks/agent-tasks/compare.py build/github-layout-v8-x112.log build/github-layout-v11.log --splice <reruns> --markdown

board                                                                |                   before |                    after |      s
-------------------------------------------------------------------- | ------------------------ | ------------------------ | ------
vd-rd__sbc_allwinner_a13__module                                     |              173/223 125 |           216/223 26 +1e |   1538
briskspirit__Sisu_SSE-9__Sisu_SSE-9                                  |          160/192 108 +2e |               185/192 26 |   1638
OpenDrone-hw__OpenESC-30x30__4in1                                    |               87/152 146 |               120/152 84 |    969
OpenDrone-hw__OpenESC-20x20__4in1-mini                               |               109/152 97 |               122/152 51 |    802
oro-os__link__link                                                   |              224/251 105 |               230/251 70 |   1662
CRImier__MyKiCad__vaio_re                                            |               256/267 31 |                262/267 5 |   1407
thpoll83__PolyKybd__poly_corne_split42_right                         |               419/420 16 |            419/420 9 +2e |   1662
antmicro__jetson-nano-baseboard__jetson-nano-baseboard               |                337/340 8 |                338/340 2 |   1525
Twisted-Fields__rp2040-motor-controller__RP2040_base                 |               151/186 86 |               151/186 82 |    925
earth75__atat-1800__ATAT1800                                         |           249/278 44 +7e |           254/278 40 +5e |   1695
MbFredys__PCB-Modular-Multi-Protocol-Hub__Hub                        |           144/159 20 +4e |           146/159 17 +3e |   1372
emertcakir__OpenAirScope__OpenAirScope                               |                176/184 8 |                179/184 5 |   1185
thpoll83__PolyKybd__poly_kybd_split72_right                          |               416/420 21 |               419/420 18 |   1665
apfaudio__eurorack-pmod__eurorack-pmod-pcb                           |                131/133 2 |                132/133 1 |    889
CRImier__MyKiCad__framework_mobo_lefthalf                            |             71/71 0 pass |             71/71 0 pass |    215
CRImier__MyKiCad__zpn_devboard                                       |           167/167 0 pass |           167/167 0 pass |    470
ISSUIUC__ISS-PCB__BAGEL-MK1                                          |            138/138 0 +1e |            138/138 0 +1e |    248
ISSUIUC__ISS-PCB__BAGEL-MK1.1-Avocado                                |                141/144 5 |                141/144 5 |    789
ISSUIUC__ISS-PCB__MIDAS-MK2                                          |           156/156 0 pass |           156/156 0 pass |    142
ISSUIUC__ISS-PCB__MIDAS-MK2.1__MIDAS-MK2.1-revA                      |           154/154 0 pass |           154/154 0 pass |    329
OpenDrone-hw__OpenESC-30x30__4in1-panel                              |               43/184 499 |               32/184 499 |    701
OpenDrone-hw__OpenFC-Lite__OpenFC                                    |                  81/82 2 |                  80/82 2 |    671
Seeed-Studio__OSHW-reCamera-Series__reCamera_S101_v1.1               |                  55/56 4 |                  55/56 4 |    228
byrantech__laptop__motherboard                                       |                234/236 2 |                234/236 2 |   1434
tengigabytes__MokyaLora__MokyaLora                                   |           261/262 0 pass |           261/262 0 pass |   1196
thpoll83__PolyKybd__poly_corne_split42_left                          |                245/247 3 |                245/247 3 |   1546
thpoll83__PolyKybd__poly_kybd_split72_left                           |                417/420 6 |                417/420 6 |   1662
hackclub__OnBoard__krishveercard                                     |                 28/43 20 |                 23/43 21 |    830
OpenDrone-hw__OpenRX__OpenRX-panel-rev2                              |               53/127 240 |               39/127 253 |    719
0xCB-dev__0xCB-1337__1337-v4.0                                       |                        - |             47/47 0 pass |     90
0xCB-dev__0xCB-1337__panel                                           |                        - |           188/188 0 pass |    533
0xCB-dev__0xCB-1337__pcb                                             |                        - |             85/85 0 pass |    207
0xCB-dev__0xCB-1337__pcb-panel                                       |                        - | ERROR (error: kicad-cli timed out aft) |    993
0xCB-dev__0xCB-Static__0xcb-static                                   |                        - |             74/74 0 pass |     25
ASH-ART__Qfwfq__qfwfq                                                |                        - |             64/64 0 pass |    205
CDFER__Business-Cards__Batch_1                                       |                        - |              124/204 468 |   1077
CDFER__Business-Cards__USB_Cable_Tester__PCB                         |                        - |              54/54 0 +1e |    242
CDFER__Business-Cards__USB_Keypad                                    |                        - |             39/39 0 pass |    117
CDFER__Business-Cards__WLED_Matrix                                   |                        - |                101/104 1 |    380
CRImier__MyKiCad__protoesp                                           |                        - |            93/104 21 +1e |    936
GlasgowEmbedded__glasgow__glasgow__revC3                             |                        - |               177/226 90 |   1144
Goga64__ULK__ULK                                                     |                        - |              70/70 0 +1e |    514
Goga64__ULK__ULK_sl_PG1316s                                          |                        - |              70/70 0 +3e |    338
Huaqiu-Electronics__ecad-viewer__video                               |                        - |           371/371 0 pass |   1012
ISSUIUC__ISS-PCB__MIDAS-MK1.1__MIDAS-MK1.1-revA                      |                        - |                151/156 8 |    584
ISSUIUC__ISS-PCB__MIDAS-MK1__MIDAS-MK1-revA                          |                        - |                154/159 8 |    556
Jana-Marie__ligra__ligra_back                                        |                        - |             33/33 0 pass |     41
Ladniy__jiran-ble-lite__jiran-ble-lite                               |                        - |               104/105 20 |    480
Neotron-Compute__Neotron-Pico__neotron-pico                          |                        - |           176/190 24 +2e |   1186
Open-Muscle__OpenMuscle-FlexGrid__OM-60-Flex                         |                        - |             21/21 0 pass |     98
Open-Muscle__OpenMuscle-FlexGrid__OM-FlexGrid-Flex__OM-FlexGrid-Flex |                        - |             19/19 0 pass |    109
Spaceflight-Rocketry-Giessen-e-V__Telemetry__TelemetryOnboard        |                        - |           108/108 0 pass |    660
anyshake__explorer__Explorer                                         |                        - |               140/150 20 |   1140
baldengineer__bit-preserve__coco2                                    |                        - |           173/173 0 +12e |    442
bismarx-v1__Sumec-MiniSumo__SUMEC_MK_IV                              |                        - |                 57/69 14 |    550
bitshiftcrazy__d20_pcb__d20_pcb                                      |                        - |             1/19 63 +27e |   1398
bitshiftcrazy__spell_tome__spell_tome_bottom                         |                        - |             18/18 0 pass |     29
byrantech__laptop__keyboard                                          |                        - |                132/136 3 |    784
byrantech__laptop__power                                             |                        - |           115/115 0 pass |    210
crmaykish__mackerel-68k__mackerel-08-v1                              |                        - |              96/96 0 +5e |    281
crmaykish__mackerel-68k__mackerel-10-v1                              |                        - |           155/155 0 pass |    911
crmaykish__mackerel-68k__mackerel-30-proto                           |                        - |           217/217 0 +15e |    613
doudar__SmartSpin2k__SmartSpin2k_Panelized                           |                        - |           585/585 0 pass |     81
duckyb__eternal-keypad__eternal-keypad                               |                        - |              69/69 0 +2e |    237
duckyb__urchin__main                                                 |                        - |             68/68 0 pass |     13
ebastler__osprey__osprey_rev_a                                       |                        - |                101/103 1 |    304
greatscottgadgets__hackrf__hackrf-one                                |                        - |            318/319 1 +1e |    780
hackclub__OnBoard__E-Fidget-Lite                                     |                        - |             20/20 0 pass |     14
hackclub__OnBoard__MotionCubeViewAllForces                           |                        - |             20/20 0 pass |    224
hackclub__OnBoard__PixelWave                                         |                        - |            343/343 0 +1e |    328
hackclub__OnBoard__keyboar_                                          |                        - |           102/102 0 pass |    268
hackclub__OnBoard__koeg-board-pcb                                    |                        - |           170/175 6 +15e |   1377
hackclub__OnBoard__woagboard                                         |                        - |                  90/91 2 |    850
headblockhead__slab-pcb__interchange-pcb-right                       |                        - |              83/83 8 +1e |   1016
headblockhead__slab-pcb__slab-pcb-left                               |                        - |             76/76 0 pass |    488
iandchasse__silkscreen-pcb__silkscreen_pcb                           |                        - |            113/113 1 +1e |    302
ikajdan__katia__katia                                                |                        - | ERROR (note: run with `RUST_BACKTRACE) |   1762
little-red-rover__little-red-rover__little_red_rover                 |                        - |             64/64 0 pass |    771
maniekx86__M8SBC-486__homebrew_486                                   |                        - |           190/190 0 pass |   1233
obsilab__Quanta75__Quanta75_BareRP2040_JLCPCBAoptimized              |                        - |          169/172 11 +29e |   1536
obsilab__Quanta75__Quanta75_RP2040Stamp_JLCPCBAoptimized             |                        - |            162/162 0 +4e |    434
ohdsp__DSP-ADAU1452__DSP-ADAU1452                                    |                        - |            189/190 5 +1e |    841
rosco-m68k__rosco_m68k__rosco_m68k__kicad                            |                        - |            148/148 0 +6e |   1120
siderakb__ergo-snm-keyboard__ErgoSNM_keyboard                        |                        - |              58/58 0 +2e |     76
sporkus__capybully_keyboard__capybully                               |                        - |                  32/32 1 |   1383
sporkus__le_chiffre_keyboard_stm32__stm32_chiffre_36keys             |                        - |             76/76 0 pass |    190
sporkus__le_chiffre_keyboard_stm32__stm32_hotswap_chiffre            |                        - |             75/75 0 pass |    298
stonedDiscord__MegaDrive__MegaDrive                                  |                        - |              274/298 107 |   1581
stonedDiscord__nonSNES__SNSP-CPU-01                                  |                        - |         333/363 116 +58e |   1666
stonedDiscord__nonSNES__SNSP-CPU-1CHIP                               |                        - |           206/212 23 +5e |   1461
thpoll83__PolyKybd__poly_kb_molecule_4x4                             |                        - |            8/28 499 +11e |   1261
thpoll83__PolyKybd__poly_kb_molecule_4x5                             |                        - |            5/28 499 +11e |   1279
thpoll83__PolyKybd__poly_kb_molecule_5x2_shifted                     |                        - |            4/28 499 +10e |   1035
thpoll83__PolyKybd__poly_kb_molecule_5x4_wave                        |                        - |            5/28 499 +18e |   1293
thpoll83__PolyKybd__poly_kb_molecule_7x5_wave_left                   |                        - |            5/28 499 +19e |   1309
tomunderwood99__CharlieBoard__Blue_Line                              |                        - |             26/26 0 pass |     35
transistorfet__computie__k30-SBC                                     |                        - |           173/173 0 +10e |    594
tubbytwins__bumwings-kbd__bumwings_v001                              |                        - |             72/72 0 pass |     78
tubbytwins__bumwings-kbd__bumwings_v001R55_rp2040zero_sd             |                        - |             70/70 0 pass |     51
tubbytwins__bumwings-kbd__bumwings_v001R55_xiao_sd                   |                        - |             75/75 0 pass |    178
tubbytwins__bumwings-kbd__bumwings_v001R64_nano_sd                   |                        - |             85/85 0 pass |    220
tubbytwins__bumwings-kbd__bumwings_v001R64_rp2040zero_sd             |                        - |             81/81 0 pass |     45
tubbytwins__bumwings-kbd__bumwings_v001R64_xiao_sd                   |                        - |             89/89 0 pass |     73
tubbytwins__bumwings-kbd__bumwings_v001_core                         |                        - |              98/98 0 +4e |    262
tubbytwins__bumwings-kbd__bumwings_v001_xiao                         |                        - |             81/81 0 pass |    393
tubbytwins__bumwings-kbd__bumwings_v001_xiao_s                       |                        - |              78/78 0 +2e |    220
tzarc__keyboards__ghoul                                              |                        - |                142/148 6 |    821
wntrblm__Castor_and_Pollux__mainboard                                |                        - |           126/126 0 +17e |    282
zli117__CyberKeeb2040__MainBoard                                     |                        - |           118/118 0 pass |    144

29 boards in both: better 14, same 13, worse 2; unconnected 1598 -> 1231; passes 5 -> 5
80 boards only after: 33 pass

Not targets among the boards only the after-run has: the five PolyKybd
molecule panels (duplicated references, the designer's board has 266
unconnected), the OpenESC and OpenRX panels, krishveercard and SUMEC (rules
no track of their class can satisfy).

## Layout sweep v14 (2026-10-10): the 16-seed race, the placer's fixes, tasks v6

`pcb-maker-x510` (aa4e9e8: the RUDY-ranked 16-seed race, the placer's
correctness rounds, the corridor schedule and the stall stop) on the v6
tasks (overhang tested against the outline polygon: 35 tasks gain fixed
parts), 10 jobs, 12668 s of wall clock: **34 of 109 pass** (v13: 31);
against v13 better on 19, same on 51, worse on 24, 3151 to 3065 open in
all (tasks differ on 35 boards, so this is a new baseline rather than a
clean A/B). Error rows: the five molecule panels and the OpenESC 4in1
panel find no legal placement; link, katia, MegaDrive and the 0xCB panel
hit the 1800 s limit (PixelWave and SmartSpin2k no longer do).

board                                                                |                   before |                    after |      s
-------------------------------------------------------------------- | ------------------------ | ------------------------ | ------
Huaqiu-Electronics__ecad-viewer__video                               |              140/371 499 |              253/371 315 |   1671
CRImier__MyKiCad__vaio_re                                            |          182/267 232 +2e |           240/267 81 +1e |   1270
vd-rd__sbc_allwinner_a13__module                                     |           176/223 97 +1e |            221/223 8 +1e |   1122
hackclub__OnBoard__koeg-board-pcb                                    |          156/175 53 +16e |          166/175 11 +15e |   1256
CRImier__MyKiCad__zpn_devboard                                       |          117/167 107 +1e |               128/167 71 |    854
briskspirit__Sisu_SSE-9__Sisu_SSE-9                                  |           183/192 42 +2e |           188/192 18 +1e |   1529
CDFER__Business-Cards__WLED_Matrix                                   |           88/104 28 +72e |             99/104 9 +8e |    831
hackclub__OnBoard__woagboard                                         |            75/91 27 +55e |            85/91 11 +34e |   1706
apfaudio__eurorack-pmod__eurorack-pmod-pcb                           |               118/133 38 |               116/133 27 |    892
Ladniy__jiran-ble-lite__jiran-ble-lite                               |                98/105 28 |           104/105 20 +2e |    562
bismarx-v1__Sumec-MiniSumo__SUMEC_MK_IV                              |                 65/69 35 |                 66/69 28 |    923
Goga64__ULK__ULK                                                     |              65/70 8 +2e |                  68/70 2 |   1742
byrantech__laptop__motherboard                                       |                233/236 7 |                233/236 3 |   1693
maniekx86__M8SBC-486__homebrew_486                                   |                189/190 3 |           190/190 0 pass |   1145
anyshake__explorer__Explorer                                         |            147/150 3 +2e |                149/150 1 |    596
duckyb__urchin__main                                                 |                  66/68 2 |             68/68 0 pass |     54
MbFredys__PCB-Modular-Multi-Protocol-Hub__Hub                        |           139/159 33 +1e |               139/159 32 |   1099
ebastler__osprey__osprey_rev_a                                       |            101/103 2 +4e |            102/103 1 +3e |    374
sporkus__capybully_keyboard__capybully                               |                  32/32 3 |                  32/32 2 |   1286
0xCB-dev__0xCB-1337__1337-v4.0                                       |             47/47 0 pass |             47/47 0 pass |    166
0xCB-dev__0xCB-1337__pcb                                             |             85/85 0 pass |             85/85 0 pass |    176
0xCB-dev__0xCB-Static__0xcb-static                                   |             74/74 0 pass |             74/74 0 pass |    154
ASH-ART__Qfwfq__qfwfq                                                |             64/64 0 pass |             64/64 0 pass |    297
CDFER__Business-Cards__USB_Cable_Tester__PCB                         |              54/54 0 +1e |             54/54 0 pass |    252
CDFER__Business-Cards__USB_Keypad                                    |                  37/39 2 |                  37/39 2 |    411
Goga64__ULK__ULK_sl_PG1316s                                          |                  68/70 2 |                  68/70 2 |   1678
ISSUIUC__ISS-PCB__BAGEL-MK1                                          |            138/138 0 +2e |           138/138 0 pass |    836
ISSUIUC__ISS-PCB__BAGEL-MK1.1-Avocado                                |           144/144 0 pass |           144/144 0 pass |    778
ISSUIUC__ISS-PCB__MIDAS-MK2                                          |           156/156 0 pass |           156/156 0 pass |    455
ISSUIUC__ISS-PCB__MIDAS-MK2.1__MIDAS-MK2.1-revA                      |           154/154 0 pass |           154/154 0 pass |    418
Jana-Marie__ligra__ligra_back                                        |             33/33 0 pass |             33/33 0 pass |    110
Neotron-Compute__Neotron-Pico__neotron-pico                          |               176/190 24 |           176/190 24 +1e |   1001
Open-Muscle__OpenMuscle-FlexGrid__OM-60-Flex                         |             21/21 0 pass |             21/21 0 pass |    183
Open-Muscle__OpenMuscle-FlexGrid__OM-FlexGrid-Flex__OM-FlexGrid-Flex |             19/19 0 pass |             19/19 0 pass |    301
OpenDrone-hw__OpenFC-Lite__OpenFC                                    |                  81/82 2 |                  81/82 2 |    727
Spaceflight-Rocketry-Giessen-e-V__Telemetry__TelemetryOnboard        |            108/108 0 +3e |            108/108 0 +4e |    730
antmicro__jetson-nano-baseboard__jetson-nano-baseboard               |            332/340 9 +3e |           333/340 9 +11e |   1677
baldengineer__bit-preserve__coco2                                    |           173/173 0 pass |           173/173 0 pass |    788
bitshiftcrazy__d20_pcb__d20_pcb                                      |                  18/19 3 |                  18/19 3 |    831
bitshiftcrazy__spell_tome__spell_tome_bottom                         |             18/18 0 pass |              18/18 0 +1e |    112
byrantech__laptop__keyboard                                          |            132/136 3 +6e |                132/136 3 |    552
crmaykish__mackerel-68k__mackerel-08-v1                              |              96/96 0 +6e |              96/96 0 +3e |    476
crmaykish__mackerel-68k__mackerel-10-v1                              |           155/155 0 pass |           155/155 0 pass |    870
crmaykish__mackerel-68k__mackerel-30-proto                           |            217/217 0 +7e |            217/217 0 +5e |   1088
duckyb__eternal-keypad__eternal-keypad                               |             69/69 0 pass |              69/69 0 +1e |    266
hackclub__OnBoard__E-Fidget-Lite                                     |             20/20 0 pass |             20/20 0 pass |     62
hackclub__OnBoard__MotionCubeViewAllForces                           |             20/20 0 pass |             20/20 0 pass |    181
hackclub__OnBoard__keyboar_                                          |            102/102 0 +1e |           102/102 0 pass |    419
headblockhead__slab-pcb__slab-pcb-left                               |             76/76 0 pass |             76/76 0 pass |    533
iandchasse__silkscreen-pcb__silkscreen_pcb                           |            113/113 1 +1e |            113/113 1 +2e |    752
little-red-rover__little-red-rover__little_red_rover                 |             64/64 0 pass |             64/64 0 pass |    236
obsilab__Quanta75__Quanta75_RP2040Stamp_JLCPCBAoptimized             |           162/162 0 +49e |           162/162 0 +47e |   1130
rosco-m68k__rosco_m68k__rosco_m68k__kicad                            |            148/148 0 +4e |            148/148 0 +4e |    731
siderakb__ergo-snm-keyboard__ErgoSNM_keyboard                        |             58/58 0 pass |             58/58 0 pass |    244
sporkus__le_chiffre_keyboard_stm32__stm32_chiffre_36keys             |             76/76 0 pass |             76/76 0 pass |    440
sporkus__le_chiffre_keyboard_stm32__stm32_hotswap_chiffre            |             75/75 0 pass |             75/75 0 pass |    381
tengigabytes__MokyaLora__MokyaLora                                   |            261/262 0 +3e |            261/262 0 +1e |   1284
tomunderwood99__CharlieBoard__Blue_Line                              |             26/26 0 pass |             26/26 0 pass |     24
transistorfet__computie__k30-SBC                                     |           173/173 0 +10e |           173/173 0 +15e |    617
tubbytwins__bumwings-kbd__bumwings_v001                              |             72/72 0 pass |             72/72 0 pass |    121
tubbytwins__bumwings-kbd__bumwings_v001R55_rp2040zero_sd             |             70/70 0 pass |             70/70 0 pass |    129
tubbytwins__bumwings-kbd__bumwings_v001R55_xiao_sd                   |             75/75 0 pass |             75/75 0 pass |    180
tubbytwins__bumwings-kbd__bumwings_v001R64_nano_sd                   |              85/85 0 +1e |             85/85 0 pass |    271
tubbytwins__bumwings-kbd__bumwings_v001R64_rp2040zero_sd             |              81/81 0 +1e |              81/81 0 +1e |    149
tubbytwins__bumwings-kbd__bumwings_v001R64_xiao_sd                   |             89/89 0 pass |             89/89 0 pass |    268
tubbytwins__bumwings-kbd__bumwings_v001_core                         |             98/98 0 pass |             98/98 0 pass |    454
tubbytwins__bumwings-kbd__bumwings_v001_xiao                         |             81/81 0 pass |             81/81 0 pass |    415
tubbytwins__bumwings-kbd__bumwings_v001_xiao_s                       |             78/78 0 pass |              78/78 0 +1e |    242
wntrblm__Castor_and_Pollux__mainboard                                |           126/126 0 +14e |            126/126 0 +6e |    716
zli117__CyberKeeb2040__MainBoard                                     |           118/118 0 pass |           118/118 0 pass |    645
ISSUIUC__ISS-PCB__MIDAS-MK1.1__MIDAS-MK1.1-revA                      |               151/156 11 |               150/156 12 |    844
byrantech__laptop__power                                             |                113/115 2 |                112/115 3 |   1099
greatscottgadgets__hackrf__hackrf-one                                |           319/319 0 +20e |            318/319 1 +6e |   1106
hackclub__OnBoard__krishveercard                                     |                 30/43 13 |                 34/43 14 |    769
headblockhead__slab-pcb__interchange-pcb-right                       |                  83/83 7 |                  83/83 8 |   1004
stonedDiscord__nonSNES__SNSP-CPU-1CHIP                               |           212/212 0 pass |           210/212 2 +16e |    735
ISSUIUC__ISS-PCB__MIDAS-MK1__MIDAS-MK1-revA                          |               152/159 12 |               151/159 16 |    836
Seeed-Studio__OSHW-reCamera-Series__reCamera_S101_v1.1               |                  55/56 4 |                 53/56 10 |    387
emertcakir__OpenAirScope__OpenAirScope                               |            184/184 0 +1e |                178/184 6 |   1003
OpenDrone-hw__OpenRX__OpenRX-panel-rev2                              |               50/127 249 |               44/127 257 |   1280
thpoll83__PolyKybd__poly_corne_split42_left                          |               225/247 33 |               218/247 43 |   1338
CRImier__MyKiCad__protoesp                                           |            87/104 55 +3e |            82/104 66 +5e |    977
earth75__atat-1800__ATAT1800                                         |           255/278 36 +1e |           242/278 49 +7e |   1409
thpoll83__PolyKybd__poly_kybd_split72_right                          |               399/420 44 |               389/420 58 |   1677
stonedDiscord__nonSNES__SNSP-CPU-01                                  |              318/363 224 |         302/363 239 +22e |   1481
GlasgowEmbedded__glasgow__glasgow__revC3                             |              111/226 237 |              113/226 254 |   1139
Twisted-Fields__rp2040-motor-controller__RP2040_base                 |               149/186 81 |              148/186 100 |   1052
thpoll83__PolyKybd__poly_corne_split42_right                         |               404/420 36 |           393/420 57 +1e |   1716
obsilab__Quanta75__Quanta75_BareRP2040_JLCPCBAoptimized              |          145/172 96 +68e |         146/172 128 +70e |   1671
thpoll83__PolyKybd__poly_kybd_split72_left                           |               409/420 23 |               394/420 55 |   1670
OpenDrone-hw__OpenESC-20x20__4in1-mini                               |               128/152 59 |                95/152 96 |   1401
CDFER__Business-Cards__Batch_1                                       |              118/204 447 |              102/204 499 |   1787
ohdsp__DSP-ADAU1452__DSP-ADAU1452                                    |          168/190 65 +14e |         118/190 167 +49e |   1010
OpenDrone-hw__OpenESC-30x30__4in1                                    |               89/152 124 |               61/152 239 |   1663
0xCB-dev__0xCB-1337__panel                                           | ERROR (timed out after 1800 s) |          145/188 129 +2e |   1660
0xCB-dev__0xCB-1337__pcb-panel                                       | ERROR (timed out after 1800 s) | ERROR (timed out after 1800 s) |   1800
CRImier__MyKiCad__framework_mobo_lefthalf                            | ERROR (error: placement is not legal ) |             71/71 0 pass |    450
OpenDrone-hw__OpenESC-30x30__4in1-panel                              |               42/184 499 | ERROR (error: placement is not legal ) |   1204
doudar__SmartSpin2k__SmartSpin2k_Panelized                           | ERROR (timed out after 1800 s) |              278/585 499 |   1742
hackclub__OnBoard__PixelWave                                         | ERROR (timed out after 1800 s) |           343/343 0 +16e |   1419
ikajdan__katia__katia                                                | ERROR (timed out after 1800 s) | ERROR (timed out after 1800 s) |   1801
oro-os__link__link                                                   |               242/251 15 | ERROR (timed out after 1800 s) |   1802
stonedDiscord__MegaDrive__MegaDrive                                  | ERROR (timed out after 1800 s) | ERROR (timed out after 1800 s) |   1800
thpoll83__PolyKybd__poly_kb_molecule_4x4                             | ERROR (error: placement is not legal ) | ERROR (error: placement is not legal ) |   1345
thpoll83__PolyKybd__poly_kb_molecule_4x5                             | ERROR (error: placement is not legal ) | ERROR (error: placement is not legal ) |   1659
thpoll83__PolyKybd__poly_kb_molecule_5x2_shifted                     | ERROR (error: placement is not legal ) | ERROR (error: placement is not legal ) |    825
thpoll83__PolyKybd__poly_kb_molecule_5x4_wave                        | ERROR (error: placement is not legal ) | ERROR (error: placement is not legal ) |    947
thpoll83__PolyKybd__poly_kb_molecule_7x5_wave_left                   | ERROR (error: placement is not legal ) | ERROR (error: placement is not legal ) |    814
tzarc__keyboards__ghoul                                              | ERROR (error: placement is not legal ) |                142/148 7 |   1474

109 boards in both: better 19, same 51, worse 24; unconnected 3151 -> 3065; passes 31 -> 34

## Layout sweep v13 (2026-10-10): every board placed from stacked parts

The first sweep after the task-preparation fix (`run.py`'s unplace now
reads pre-KiCad-10 net syntax, so all 109 boards lose the designer's
placement, not 22), on `pcb-maker-x503` (3b890fa: the pour model, the
parallel ladder, work-based and deterministic layout decisions, the
placer's correctness fixes; the 16-seed RUDY race came one commit later),
12 jobs, load 25-40, 14525 s of wall clock: **31 of 109 pass**, 3586
connections open in all against v12's 2151 on the same boards (v12 kept
the designer's placement on 80 of them). Six boards gained a pass through
the fixes (BAGEL-MK1.1, SNSP-CPU-1CHIP, two bumwings, coco2, Qfwfq); 16
lost theirs to real placement (MokyaLora, zpn, video, homebrew 486,
hackrf, laptop power, ...). Error rows: 7 boards where the placer finds
no legal placement (the five molecule panels, framework_mobo's M1,
ghoul's switches), 6 at the 1800 s limit (katia, PixelWave, SmartSpin2k,
MegaDrive, the two 0xCB panels).

board                                                                |                   before |                    after |      s
-------------------------------------------------------------------- | ------------------------ | ------------------------ | ------
bitshiftcrazy__d20_pcb__d20_pcb                                      |             1/19 63 +27e |                  18/19 3 |   1161
oro-os__link__link                                                   |               232/251 56 |               242/251 15 |   1748
hackclub__OnBoard__krishveercard                                     |                 23/43 21 |                 30/43 13 |   1182
ISSUIUC__ISS-PCB__BAGEL-MK1.1-Avocado                                |                141/144 5 |           144/144 0 pass |    683
emertcakir__OpenAirScope__OpenAirScope                               |                179/184 5 |            184/184 0 +1e |   1648
anyshake__explorer__Explorer                                         |            147/150 7 +1e |            147/150 3 +2e |   1024
Twisted-Fields__rp2040-motor-controller__RP2040_base                 |               150/186 84 |               149/186 81 |   1155
stonedDiscord__nonSNES__SNSP-CPU-1CHIP                               |                210/212 3 |           212/212 0 pass |   1441
sporkus__capybully_keyboard__capybully                               |                  32/32 5 |                  32/32 3 |   1699
0xCB-dev__0xCB-1337__1337-v4.0                                       |             47/47 0 pass |             47/47 0 pass |    272
0xCB-dev__0xCB-1337__pcb                                             |             85/85 0 pass |             85/85 0 pass |    301
0xCB-dev__0xCB-Static__0xcb-static                                   |             74/74 0 pass |             74/74 0 pass |    205
CDFER__Business-Cards__USB_Cable_Tester__PCB                         |              54/54 0 +1e |              54/54 0 +1e |    220
ISSUIUC__ISS-PCB__BAGEL-MK1                                          |           138/138 0 pass |            138/138 0 +2e |    800
ISSUIUC__ISS-PCB__MIDAS-MK2                                          |           156/156 0 pass |           156/156 0 pass |    736
ISSUIUC__ISS-PCB__MIDAS-MK2.1__MIDAS-MK2.1-revA                      |           154/154 0 pass |           154/154 0 pass |    753
Jana-Marie__ligra__ligra_back                                        |             33/33 0 pass |             33/33 0 pass |    124
Neotron-Compute__Neotron-Pico__neotron-pico                          |           176/190 24 +1e |               176/190 24 |   1403
Open-Muscle__OpenMuscle-FlexGrid__OM-60-Flex                         |             21/21 0 pass |             21/21 0 pass |    214
Open-Muscle__OpenMuscle-FlexGrid__OM-FlexGrid-Flex__OM-FlexGrid-Flex |             19/19 0 pass |             19/19 0 pass |    596
OpenDrone-hw__OpenESC-20x20__4in1-mini                               |               128/152 59 |               128/152 59 |    976
OpenDrone-hw__OpenESC-30x30__4in1-panel                              |               32/184 499 |               42/184 499 |   1524
OpenDrone-hw__OpenFC-Lite__OpenFC                                    |                  81/82 2 |                  81/82 2 |    690
Seeed-Studio__OSHW-reCamera-Series__reCamera_S101_v1.1               |                  55/56 4 |                  55/56 4 |    553
bitshiftcrazy__spell_tome__spell_tome_bottom                         |             18/18 0 pass |             18/18 0 pass |     78
byrantech__laptop__keyboard                                          |                132/136 3 |            132/136 3 +6e |   1131
crmaykish__mackerel-68k__mackerel-08-v1                              |              96/96 0 +3e |              96/96 0 +6e |   1086
crmaykish__mackerel-68k__mackerel-10-v1                              |           155/155 0 pass |           155/155 0 pass |    876
crmaykish__mackerel-68k__mackerel-30-proto                           |            217/217 0 +6e |            217/217 0 +7e |   1659
duckyb__eternal-keypad__eternal-keypad                               |             69/69 0 pass |             69/69 0 pass |    324
earth75__atat-1800__ATAT1800                                         |          251/278 36 +18e |           255/278 36 +1e |   1715
greatscottgadgets__hackrf__hackrf-one                                |           319/319 0 pass |           319/319 0 +20e |   1272
hackclub__OnBoard__E-Fidget-Lite                                     |             20/20 0 pass |             20/20 0 pass |    123
hackclub__OnBoard__MotionCubeViewAllForces                           |             20/20 0 pass |             20/20 0 pass |    213
hackclub__OnBoard__keyboar_                                          |           102/102 0 pass |            102/102 0 +1e |    832
headblockhead__slab-pcb__interchange-pcb-right                       |              83/83 7 +2e |                  83/83 7 |   1336
headblockhead__slab-pcb__slab-pcb-left                               |             76/76 0 pass |             76/76 0 pass |   1217
iandchasse__silkscreen-pcb__silkscreen_pcb                           |            113/113 1 +1e |            113/113 1 +1e |   1419
little-red-rover__little-red-rover__little_red_rover                 |             64/64 0 pass |             64/64 0 pass |    554
obsilab__Quanta75__Quanta75_RP2040Stamp_JLCPCBAoptimized             |            162/162 0 +2e |           162/162 0 +49e |    964
rosco-m68k__rosco_m68k__rosco_m68k__kicad                            |            148/148 0 +3e |            148/148 0 +4e |    979
siderakb__ergo-snm-keyboard__ErgoSNM_keyboard                        |             58/58 0 pass |             58/58 0 pass |    512
sporkus__le_chiffre_keyboard_stm32__stm32_chiffre_36keys             |             76/76 0 pass |             76/76 0 pass |    304
sporkus__le_chiffre_keyboard_stm32__stm32_hotswap_chiffre            |             75/75 0 pass |             75/75 0 pass |    240
tengigabytes__MokyaLora__MokyaLora                                   |           261/262 0 pass |            261/262 0 +3e |   1325
tomunderwood99__CharlieBoard__Blue_Line                              |             26/26 0 pass |             26/26 0 pass |    114
transistorfet__computie__k30-SBC                                     |           173/173 0 +11e |           173/173 0 +10e |   1032
tubbytwins__bumwings-kbd__bumwings_v001                              |             72/72 0 pass |             72/72 0 pass |    189
tubbytwins__bumwings-kbd__bumwings_v001R55_rp2040zero_sd             |             70/70 0 pass |             70/70 0 pass |    281
tubbytwins__bumwings-kbd__bumwings_v001R55_xiao_sd                   |             75/75 0 pass |             75/75 0 pass |    205
tubbytwins__bumwings-kbd__bumwings_v001R64_nano_sd                   |             85/85 0 pass |              85/85 0 +1e |    221
tubbytwins__bumwings-kbd__bumwings_v001R64_rp2040zero_sd             |             81/81 0 pass |              81/81 0 +1e |    158
tubbytwins__bumwings-kbd__bumwings_v001_core                         |             98/98 0 pass |             98/98 0 pass |    815
tubbytwins__bumwings-kbd__bumwings_v001_xiao                         |             81/81 0 pass |             81/81 0 pass |    867
tubbytwins__bumwings-kbd__bumwings_v001_xiao_s                       |              78/78 0 +1e |             78/78 0 pass |    456
wntrblm__Castor_and_Pollux__mainboard                                |            126/126 0 +2e |           126/126 0 +14e |    990
zli117__CyberKeeb2040__MainBoard                                     |           118/118 0 pass |           118/118 0 pass |    671
ebastler__osprey__osprey_rev_a                                       |                101/103 1 |            101/103 2 +4e |    384
CDFER__Business-Cards__USB_Keypad                                    |             39/39 0 pass |                  37/39 2 |    531
Goga64__ULK__ULK_sl_PG1316s                                          |              70/70 0 +3e |                  68/70 2 |   1687
byrantech__laptop__power                                             |           115/115 0 pass |                113/115 2 |   1363
duckyb__urchin__main                                                 |             68/68 0 pass |                  66/68 2 |    339
ISSUIUC__ISS-PCB__MIDAS-MK1.1__MIDAS-MK1.1-revA                      |                151/156 8 |               151/156 11 |    696
maniekx86__M8SBC-486__homebrew_486                                   |           190/190 0 pass |                189/190 3 |   1667
ISSUIUC__ISS-PCB__MIDAS-MK1__MIDAS-MK1-revA                          |                154/159 8 |               152/159 12 |   1216
byrantech__laptop__motherboard                                       |                234/236 2 |                233/236 7 |   1675
antmicro__jetson-nano-baseboard__jetson-nano-baseboard               |                338/340 2 |            332/340 9 +3e |   1675
Goga64__ULK__ULK                                                     |              70/70 0 +1e |              65/70 8 +2e |   1680
Ladniy__jiran-ble-lite__jiran-ble-lite                               |               104/105 20 |                98/105 28 |    848
OpenDrone-hw__OpenRX__OpenRX-panel-rev2                              |               54/127 237 |               50/127 249 |   1082
MbFredys__PCB-Modular-Multi-Protocol-Hub__Hub                        |           146/159 17 +2e |           139/159 33 +1e |   1313
stonedDiscord__nonSNES__SNSP-CPU-01                                  |              308/363 206 |              318/363 224 |   1720
thpoll83__PolyKybd__poly_kybd_split72_left                           |                418/420 3 |               409/420 23 |   1719
hackclub__OnBoard__woagboard                                         |                  90/91 2 |            75/91 27 +55e |   1667
CDFER__Business-Cards__WLED_Matrix                                   |                101/104 1 |           88/104 28 +72e |    486
thpoll83__PolyKybd__poly_corne_split42_left                          |                246/247 4 |               225/247 33 |   1681
vd-rd__sbc_allwinner_a13__module                                     |           181/223 68 +1e |           176/223 97 +1e |   1146
briskspirit__Sisu_SSE-9__Sisu_SSE-9                                  |               183/192 12 |           183/192 42 +2e |   1408
thpoll83__PolyKybd__poly_corne_split42_right                         |                418/420 4 |               404/420 36 |   1754
CDFER__Business-Cards__Batch_1                                       |              125/204 414 |              118/204 447 |   1547
CRImier__MyKiCad__protoesp                                           |                93/104 18 |            87/104 55 +3e |    956
OpenDrone-hw__OpenESC-30x30__4in1                                    |               118/152 87 |               89/152 124 |   1316
apfaudio__eurorack-pmod__eurorack-pmod-pcb                           |                132/133 1 |               118/133 38 |    926
hackclub__OnBoard__koeg-board-pcb                                    |           168/175 9 +15e |          156/175 53 +16e |   1683
obsilab__Quanta75__Quanta75_BareRP2040_JLCPCBAoptimized              |          147/172 36 +26e |          145/172 96 +68e |   1687
ohdsp__DSP-ADAU1452__DSP-ADAU1452                                    |                189/190 5 |          168/190 65 +14e |   1357
CRImier__MyKiCad__zpn_devboard                                       |           167/167 0 pass |          117/167 107 +1e |   1150
GlasgowEmbedded__glasgow__glasgow__revC3                             |               173/226 93 |              111/226 237 |   1572
CRImier__MyKiCad__vaio_re                                            |                260/267 9 |          182/267 232 +2e |   1670
Huaqiu-Electronics__ecad-viewer__video                               |           371/371 0 pass |              140/371 499 |   1728
0xCB-dev__0xCB-1337__panel                                           |           188/188 0 pass | ERROR (timed out after 1800 s) |   1801
0xCB-dev__0xCB-1337__pcb-panel                                       | ERROR (error: kicad-cli timed out aft) | ERROR (timed out after 1800 s) |   1800
ASH-ART__Qfwfq__qfwfq                                                | ERROR (note: run with `RUST_BACKTRACE) |             64/64 0 pass |    637
CRImier__MyKiCad__framework_mobo_lefthalf                            |             71/71 0 pass | ERROR (error: placement is not legal ) |     13
Spaceflight-Rocketry-Giessen-e-V__Telemetry__TelemetryOnboard        | ERROR (error: rule area polygon has f) |            108/108 0 +3e |    989
baldengineer__bit-preserve__coco2                                    | ERROR (note: run with `RUST_BACKTRACE) |           173/173 0 pass |   1335
bismarx-v1__Sumec-MiniSumo__SUMEC_MK_IV                              | ERROR (note: run with `RUST_BACKTRACE) |                 65/69 35 |   1271
doudar__SmartSpin2k__SmartSpin2k_Panelized                           |           585/585 0 pass | ERROR (timed out after 1800 s) |   1800
hackclub__OnBoard__PixelWave                                         |           343/343 0 pass | ERROR (timed out after 1800 s) |   1800
ikajdan__katia__katia                                                |               148/162 99 | ERROR (timed out after 1800 s) |   1801
stonedDiscord__MegaDrive__MegaDrive                                  |              271/298 119 | ERROR (timed out after 1800 s) |   1801
thpoll83__PolyKybd__poly_kb_molecule_4x4                             |            4/28 499 +12e | ERROR (error: placement is not legal ) |    352
thpoll83__PolyKybd__poly_kb_molecule_4x5                             |            4/28 499 +20e | ERROR (error: placement is not legal ) |    646
thpoll83__PolyKybd__poly_kb_molecule_5x2_shifted                     |             4/28 499 +7e | ERROR (error: placement is not legal ) |    426
thpoll83__PolyKybd__poly_kb_molecule_5x4_wave                        |            5/28 499 +34e | ERROR (error: placement is not legal ) |    328
thpoll83__PolyKybd__poly_kb_molecule_7x5_wave_left                   |            3/28 499 +29e | ERROR (error: placement is not legal ) |    661
thpoll83__PolyKybd__poly_kybd_split72_right                          | ERROR (timed out after 1800 s) |               399/420 44 |   1725
tubbytwins__bumwings-kbd__bumwings_v001R64_xiao_sd                   | ERROR (note: run with `RUST_BACKTRACE) |             89/89 0 pass |    316
tzarc__keyboards__ghoul                                              |                142/148 6 | ERROR (error: placement is not legal ) |    944

109 boards in both: better 9, same 48, worse 33; unconnected 2151 -> 3586; passes 41 -> 31

## Layout sweep v12 (2026-10-09, later): the pour model and the parallel ladder

The same 109 tasks on `pcb-maker-x165` (HEAD 1c7a557: diagonal pour necks,
fill-keepout rule areas, the router at a quarter to a half less memory, the
probe ladder's rungs and the placement race side by side), 14 boards at a
time under the runner's memory gate: **41 of 109 pass** (v11: 38) in 10375 s
of wall clock for the whole sweep (v11 took about a day at three jobs).
Against v11 (with its reruns spliced): better on 12, same on 77, worse on
12; unconnected 4722 to 4771 in all. The three big losses are known costs
or open bugs: SNSP-CPU-01 116 to 206 (J2's mask opening as its body),
Quanta75 11 to 36 (the knockout plate as an obstacle), the A13 module 26
to 68 (a different race winner; being rerun). Seven error rows: Telemetry
(a degenerate fill-keepout rule area refused by the new lowering), four
panics in `refresh_pour` (SUMEC, coco2, bumwings xiao_sd, Qfwfq; a stale
index into the pour analysis), PolyKybd split72 right at the 1800 s limit
under the sweep's load, the 0xCB panel's kicad-cli refill timeout.

board                                                                |                   before |                    after |      s
-------------------------------------------------------------------- | ------------------------ | ------------------------ | ------
CDFER__Business-Cards__Batch_1                                       |              124/204 468 |              125/204 414 |   1709
stonedDiscord__nonSNES__SNSP-CPU-1CHIP                               |           206/212 23 +5e |                210/212 3 |   1295
OpenDrone-hw__OpenRX__OpenRX-panel-rev2                              |               39/127 253 |               54/127 237 |    939
briskspirit__Sisu_SSE-9__Sisu_SSE-9                                  |               185/192 26 |               183/192 12 |   1094
oro-os__link__link                                                   |               230/251 70 |               232/251 56 |   1565
anyshake__explorer__Explorer                                         |               140/150 20 |            147/150 7 +1e |   1452
thpoll83__PolyKybd__poly_corne_split42_right                         |            419/420 9 +2e |                418/420 4 |   1704
earth75__atat-1800__ATAT1800                                         |           254/278 40 +5e |          251/278 36 +18e |   1678
CRImier__MyKiCad__protoesp                                           |            93/104 21 +1e |                93/104 18 |   1458
thpoll83__PolyKybd__poly_kybd_split72_left                           |                417/420 6 |                418/420 3 |   1669
greatscottgadgets__hackrf__hackrf-one                                |            318/319 1 +1e |           319/319 0 pass |    359
headblockhead__slab-pcb__interchange-pcb-right                       |              83/83 8 +1e |              83/83 7 +2e |   1303
0xCB-dev__0xCB-1337__1337-v4.0                                       |             47/47 0 pass |             47/47 0 pass |    114
0xCB-dev__0xCB-1337__panel                                           |           188/188 0 pass |           188/188 0 pass |    471
0xCB-dev__0xCB-1337__pcb                                             |             85/85 0 pass |             85/85 0 pass |    141
0xCB-dev__0xCB-Static__0xcb-static                                   |             74/74 0 pass |             74/74 0 pass |     34
CDFER__Business-Cards__USB_Cable_Tester__PCB                         |              54/54 0 +1e |              54/54 0 +1e |    237
CDFER__Business-Cards__USB_Keypad                                    |             39/39 0 pass |             39/39 0 pass |    148
CDFER__Business-Cards__WLED_Matrix                                   |                101/104 1 |                101/104 1 |    624
CRImier__MyKiCad__framework_mobo_lefthalf                            |             71/71 0 pass |             71/71 0 pass |    362
CRImier__MyKiCad__zpn_devboard                                       |           167/167 0 pass |           167/167 0 pass |    522
Goga64__ULK__ULK                                                     |              70/70 0 +1e |              70/70 0 +1e |   1052
Goga64__ULK__ULK_sl_PG1316s                                          |              70/70 0 +3e |              70/70 0 +3e |    676
Huaqiu-Electronics__ecad-viewer__video                               |           371/371 0 pass |           371/371 0 pass |   1424
ISSUIUC__ISS-PCB__BAGEL-MK1                                          |            138/138 0 +1e |           138/138 0 pass |     99
ISSUIUC__ISS-PCB__BAGEL-MK1.1-Avocado                                |                141/144 5 |                141/144 5 |    982
ISSUIUC__ISS-PCB__MIDAS-MK1.1__MIDAS-MK1.1-revA                      |                151/156 8 |                151/156 8 |    676
ISSUIUC__ISS-PCB__MIDAS-MK1__MIDAS-MK1-revA                          |                154/159 8 |                154/159 8 |    614
ISSUIUC__ISS-PCB__MIDAS-MK2                                          |           156/156 0 pass |           156/156 0 pass |    273
ISSUIUC__ISS-PCB__MIDAS-MK2.1__MIDAS-MK2.1-revA                      |           154/154 0 pass |           154/154 0 pass |    106
Jana-Marie__ligra__ligra_back                                        |             33/33 0 pass |             33/33 0 pass |     73
Ladniy__jiran-ble-lite__jiran-ble-lite                               |               104/105 20 |               104/105 20 |    862
MbFredys__PCB-Modular-Multi-Protocol-Hub__Hub                        |           146/159 17 +3e |           146/159 17 +2e |   1355
Neotron-Compute__Neotron-Pico__neotron-pico                          |           176/190 24 +2e |           176/190 24 +1e |   1329
Open-Muscle__OpenMuscle-FlexGrid__OM-60-Flex                         |             21/21 0 pass |             21/21 0 pass |    128
Open-Muscle__OpenMuscle-FlexGrid__OM-FlexGrid-Flex__OM-FlexGrid-Flex |             19/19 0 pass |             19/19 0 pass |    187
OpenDrone-hw__OpenESC-30x30__4in1-panel                              |               32/184 499 |               32/184 499 |   1122
OpenDrone-hw__OpenFC-Lite__OpenFC                                    |                  80/82 2 |                  81/82 2 |    852
Seeed-Studio__OSHW-reCamera-Series__reCamera_S101_v1.1               |                  55/56 4 |                  55/56 4 |    279
antmicro__jetson-nano-baseboard__jetson-nano-baseboard               |                338/340 2 |                338/340 2 |   1581
apfaudio__eurorack-pmod__eurorack-pmod-pcb                           |                132/133 1 |                132/133 1 |   1038
bitshiftcrazy__d20_pcb__d20_pcb                                      |             1/19 63 +27e |             1/19 63 +27e |   1425
bitshiftcrazy__spell_tome__spell_tome_bottom                         |             18/18 0 pass |             18/18 0 pass |     69
byrantech__laptop__keyboard                                          |                132/136 3 |                132/136 3 |   1070
byrantech__laptop__motherboard                                       |                234/236 2 |                234/236 2 |   1419
byrantech__laptop__power                                             |           115/115 0 pass |           115/115 0 pass |     80
crmaykish__mackerel-68k__mackerel-08-v1                              |              96/96 0 +5e |              96/96 0 +3e |    440
crmaykish__mackerel-68k__mackerel-10-v1                              |           155/155 0 pass |           155/155 0 pass |   1638
crmaykish__mackerel-68k__mackerel-30-proto                           |           217/217 0 +15e |            217/217 0 +6e |    717
doudar__SmartSpin2k__SmartSpin2k_Panelized                           |           585/585 0 pass |           585/585 0 pass |    145
duckyb__eternal-keypad__eternal-keypad                               |              69/69 0 +2e |             69/69 0 pass |    102
duckyb__urchin__main                                                 |             68/68 0 pass |             68/68 0 pass |     16
ebastler__osprey__osprey_rev_a                                       |                101/103 1 |                101/103 1 |    381
emertcakir__OpenAirScope__OpenAirScope                               |                179/184 5 |                179/184 5 |   1000
hackclub__OnBoard__E-Fidget-Lite                                     |             20/20 0 pass |             20/20 0 pass |     56
hackclub__OnBoard__MotionCubeViewAllForces                           |             20/20 0 pass |             20/20 0 pass |    204
hackclub__OnBoard__PixelWave                                         |            343/343 0 +1e |           343/343 0 pass |    180
hackclub__OnBoard__keyboar_                                          |           102/102 0 pass |           102/102 0 pass |    270
hackclub__OnBoard__krishveercard                                     |                 23/43 21 |                 23/43 21 |    864
hackclub__OnBoard__woagboard                                         |                  90/91 2 |                  90/91 2 |   1357
headblockhead__slab-pcb__slab-pcb-left                               |             76/76 0 pass |             76/76 0 pass |    146
iandchasse__silkscreen-pcb__silkscreen_pcb                           |            113/113 1 +1e |            113/113 1 +1e |    399
little-red-rover__little-red-rover__little_red_rover                 |             64/64 0 pass |             64/64 0 pass |    715
maniekx86__M8SBC-486__homebrew_486                                   |           190/190 0 pass |           190/190 0 pass |   1406
obsilab__Quanta75__Quanta75_RP2040Stamp_JLCPCBAoptimized             |            162/162 0 +4e |            162/162 0 +2e |    374
ohdsp__DSP-ADAU1452__DSP-ADAU1452                                    |            189/190 5 +1e |                189/190 5 |   1059
rosco-m68k__rosco_m68k__rosco_m68k__kicad                            |            148/148 0 +6e |            148/148 0 +3e |    781
siderakb__ergo-snm-keyboard__ErgoSNM_keyboard                        |              58/58 0 +2e |             58/58 0 pass |     58
sporkus__le_chiffre_keyboard_stm32__stm32_chiffre_36keys             |             76/76 0 pass |             76/76 0 pass |    550
sporkus__le_chiffre_keyboard_stm32__stm32_hotswap_chiffre            |             75/75 0 pass |             75/75 0 pass |    576
tengigabytes__MokyaLora__MokyaLora                                   |           261/262 0 pass |           261/262 0 pass |   1348
thpoll83__PolyKybd__poly_kb_molecule_4x4                             |            8/28 499 +11e |            4/28 499 +12e |   1333
thpoll83__PolyKybd__poly_kb_molecule_4x5                             |            5/28 499 +11e |            4/28 499 +20e |   1584
thpoll83__PolyKybd__poly_kb_molecule_5x2_shifted                     |            4/28 499 +10e |             4/28 499 +7e |   1420
thpoll83__PolyKybd__poly_kb_molecule_5x4_wave                        |            5/28 499 +18e |            5/28 499 +34e |   1457
thpoll83__PolyKybd__poly_kb_molecule_7x5_wave_left                   |            5/28 499 +19e |            3/28 499 +29e |   1540
tomunderwood99__CharlieBoard__Blue_Line                              |             26/26 0 pass |             26/26 0 pass |     31
transistorfet__computie__k30-SBC                                     |           173/173 0 +10e |           173/173 0 +11e |   1206
tubbytwins__bumwings-kbd__bumwings_v001                              |             72/72 0 pass |             72/72 0 pass |    112
tubbytwins__bumwings-kbd__bumwings_v001R55_rp2040zero_sd             |             70/70 0 pass |             70/70 0 pass |    118
tubbytwins__bumwings-kbd__bumwings_v001R55_xiao_sd                   |             75/75 0 pass |             75/75 0 pass |    284
tubbytwins__bumwings-kbd__bumwings_v001R64_nano_sd                   |             85/85 0 pass |             85/85 0 pass |     54
tubbytwins__bumwings-kbd__bumwings_v001R64_rp2040zero_sd             |             81/81 0 pass |             81/81 0 pass |     51
tubbytwins__bumwings-kbd__bumwings_v001_core                         |              98/98 0 +4e |             98/98 0 pass |    345
tubbytwins__bumwings-kbd__bumwings_v001_xiao                         |             81/81 0 pass |             81/81 0 pass |    427
tubbytwins__bumwings-kbd__bumwings_v001_xiao_s                       |              78/78 0 +2e |              78/78 0 +1e |    304
tzarc__keyboards__ghoul                                              |                142/148 6 |                142/148 6 |   1062
wntrblm__Castor_and_Pollux__mainboard                                |           126/126 0 +17e |            126/126 0 +2e |    277
zli117__CyberKeeb2040__MainBoard                                     |           118/118 0 pass |           118/118 0 pass |    338
thpoll83__PolyKybd__poly_corne_split42_left                          |                245/247 3 |                246/247 4 |   1617
Twisted-Fields__rp2040-motor-controller__RP2040_base                 |               151/186 82 |               150/186 84 |    896
GlasgowEmbedded__glasgow__glasgow__revC3                             |               177/226 90 |               173/226 93 |   1219
OpenDrone-hw__OpenESC-30x30__4in1                                    |               120/152 84 |               118/152 87 |   1207
hackclub__OnBoard__koeg-board-pcb                                    |           170/175 6 +15e |           168/175 9 +15e |   1472
CRImier__MyKiCad__vaio_re                                            |                262/267 5 |                260/267 9 |   1533
sporkus__capybully_keyboard__capybully                               |                  32/32 1 |                  32/32 5 |   1450
OpenDrone-hw__OpenESC-20x20__4in1-mini                               |               122/152 51 |               128/152 59 |    967
stonedDiscord__MegaDrive__MegaDrive                                  |              274/298 107 |              271/298 119 |   1479
obsilab__Quanta75__Quanta75_BareRP2040_JLCPCBAoptimized              |          169/172 11 +29e |          147/172 36 +26e |   1480
vd-rd__sbc_allwinner_a13__module                                     |           216/223 26 +1e |           181/223 68 +1e |   1270
stonedDiscord__nonSNES__SNSP-CPU-01                                  |         333/363 116 +58e |              308/363 206 |   1419
0xCB-dev__0xCB-1337__pcb-panel                                       | ERROR (error: kicad-cli timed out aft) | ERROR (error: kicad-cli timed out aft) |   1043
ASH-ART__Qfwfq__qfwfq                                                |             64/64 0 pass | ERROR (note: run with `RUST_BACKTRACE) |    209
Spaceflight-Rocketry-Giessen-e-V__Telemetry__TelemetryOnboard        |           108/108 0 pass | ERROR (error: rule area polygon has f) |      0
baldengineer__bit-preserve__coco2                                    |           173/173 0 +12e | ERROR (note: run with `RUST_BACKTRACE) |    273
bismarx-v1__Sumec-MiniSumo__SUMEC_MK_IV                              |                 57/69 14 | ERROR (note: run with `RUST_BACKTRACE) |    217
ikajdan__katia__katia                                                | ERROR (note: run with `RUST_BACKTRACE) |               148/162 99 |   1783
thpoll83__PolyKybd__poly_kybd_split72_right                          |               419/420 18 | ERROR (timed out after 1800 s) |   1800
tubbytwins__bumwings-kbd__bumwings_v001R64_xiao_sd                   |             89/89 0 pass | ERROR (note: run with `RUST_BACKTRACE) |     68

109 boards in both: better 12, same 77, worse 12; unconnected 4722 -> 4771; passes 38 -> 41

## Freerouting against our cold route on the harvested boards (2026-10-09)

Both route the designer's placement with every track and via removed, 1500 s
each, one board at a time; Freerouting 2.x headless through its CLI
(`benchmarks/github/freerouting.py`), the result imported as a session file
and refilled; counts are KiCad's, errors beyond the designer's board. All 109
boards (`freerouting_compare.py`; our rows are x136 for the first 40 boards
and x155 for the rest, so the two binaries' pour models are mixed on our
side; Freerouting's "failed" rows are a DSN export or session import that
did not work):

board                                                                |                    Freerouting |                          pcb-maker
-------------------------------------------------------------------- | ------------------------------ | ----------------------------------
0xCB-dev__0xCB-1337__1337-v4.0                                       |                7 open +0e 30 s |              47/47 0 open +0e 63 s
0xCB-dev__0xCB-1337__panel                                           |              28 open +0e 185 s |           188/188 0 open +0e 677 s
0xCB-dev__0xCB-1337__pcb                                             |               3 open +86e 33 s |             85/85 0 open +0e 169 s
0xCB-dev__0xCB-1337__pcb-panel                                       |            29 open +201e 603 s |           340/340 0 open +0e 371 s
0xCB-dev__0xCB-Static__0xcb-static                                   |                0 open +0e 10 s |              74/74 0 open +0e 34 s
ASH-ART__Qfwfq__qfwfq                                                |               1 open +0e 100 s |              64/64 0 open +0e 63 s
CDFER__Business-Cards__Batch_1                                       |                 timeout 1500 s |         127/204 499 open +0e 488 s
CDFER__Business-Cards__USB_Cable_Tester__PCB                         |              1 open +172e 24 s |              54/54 0 open +0e 57 s
CDFER__Business-Cards__USB_Keypad                                    |               8 open +39e 35 s |              39/39 0 open +0e 43 s
CDFER__Business-Cards__WLED_Matrix                                   |             12 open +36e 116 s |            101/104 4 open +0e 98 s
CRImier__MyKiCad__framework_mobo_lefthalf                            |              42 open +0e 305 s |              71/71 0 open +0e 64 s
CRImier__MyKiCad__protoesp                                           |              65 open +3e 529 s |           93/104 24 open +0e 232 s
CRImier__MyKiCad__vaio_re                                            |                 timeout 1500 s |         261/267 26 open +0e 1166 s
CRImier__MyKiCad__zpn_devboard                                       |                 no session 3 s |           167/167 0 open +1e 289 s
GlasgowEmbedded__glasgow__glasgow__revC3                             |                 timeout 1500 s |         172/226 86 open +0e 1166 s
Goga64__ULK__ULK                                                     |               0 open +51e 15 s |             70/70 0 open +0e 122 s
Goga64__ULK__ULK_sl_PG1316s                                          |               0 open +10e 30 s |             70/70 0 open +0e 127 s
Huaqiu-Electronics__ecad-viewer__video                               |              1 open +0e 1065 s |          371/371 0 open +0e 1728 s
ISSUIUC__ISS-PCB__BAGEL-MK1                                          |                 timeout 1500 s |           138/138 0 open +0e 122 s
ISSUIUC__ISS-PCB__BAGEL-MK1.1-Avocado                                |                 timeout 1500 s |           142/144 2 open +0e 608 s
ISSUIUC__ISS-PCB__MIDAS-MK1.1__MIDAS-MK1.1-revA                      |             23 open +23e 602 s |           151/156 8 open +0e 482 s
ISSUIUC__ISS-PCB__MIDAS-MK1__MIDAS-MK1-revA                          |              18 open +0e 923 s |           154/159 8 open +0e 539 s
ISSUIUC__ISS-PCB__MIDAS-MK2                                          |               7 open +0e 199 s |           156/156 0 open +0e 179 s
ISSUIUC__ISS-PCB__MIDAS-MK2.1__MIDAS-MK2.1-revA                      |               5 open +0e 146 s |           154/154 0 open +0e 106 s
Jana-Marie__ligra__ligra_back                                        |                0 open +0e 12 s |              33/33 0 open +0e 69 s
Ladniy__jiran-ble-lite__jiran-ble-lite                               |                5 open +5e 81 s |          104/105 20 open +0e 297 s
MbFredys__PCB-Modular-Multi-Protocol-Hub__Hub                        |                 no session 3 s |         148/159 22 open +0e 1000 s
Neotron-Compute__Neotron-Pico__neotron-pico                          |              14 open +2e 144 s |          176/190 24 open +0e 966 s
Open-Muscle__OpenMuscle-FlexGrid__OM-60-Flex                         |                0 open +1e 10 s |             21/21 0 open +0e 118 s
Open-Muscle__OpenMuscle-FlexGrid__OM-FlexGrid-Flex__OM-FlexGrid-Flex |                0 open +0e 35 s |             19/19 0 open +0e 218 s
OpenDrone-hw__OpenESC-20x20__4in1-mini                               |                 timeout 1500 s |           151/152 1 open +0e 482 s
OpenDrone-hw__OpenESC-30x30__4in1                                    |                 timeout 1501 s |           152/152 0 open +0e 442 s
OpenDrone-hw__OpenESC-30x30__4in1-panel                              |                 timeout 1500 s |          49/184 499 open +0e 323 s
OpenDrone-hw__OpenFC-Lite__OpenFC                                    |             8 open +143e 942 s |             80/82 2 open +0e 421 s
OpenDrone-hw__OpenRX__OpenRX-panel-rev2                              |                 timeout 1500 s |          43/127 266 open +4e 544 s
Seeed-Studio__OSHW-reCamera-Series__reCamera_S101_v1.1               |              0 open +18e 194 s |             55/56 4 open +0e 216 s
Spaceflight-Rocketry-Giessen-e-V__Telemetry__TelemetryOnboard        |               17 open +0e 95 s |           108/108 0 open +0e 484 s
Twisted-Fields__rp2040-motor-controller__RP2040_base                 |                 timeout 1500 s |          151/186 91 open +0e 709 s
antmicro__jetson-nano-baseboard__jetson-nano-baseboard               |                 timeout 1501 s |          338/340 2 open +0e 1298 s
anyshake__explorer__Explorer                                         |              15 open +1e 898 s |           150/150 0 open +0e 214 s
apfaudio__eurorack-pmod__eurorack-pmod-pcb                           |                 timeout 1500 s |           132/133 1 open +0e 585 s
baldengineer__bit-preserve__coco2                                    |               0 open +0e 151 s |           173/173 0 open +0e 350 s
bismarx-v1__Sumec-MiniSumo__SUMEC_MK_IV                              |            import failed 187 s |             66/69 9 open +0e 599 s
bitshiftcrazy__d20_pcb__d20_pcb                                      |              0 open +151e 10 s |             18/19 3 open +0e 435 s
bitshiftcrazy__spell_tome__spell_tome_bottom                         |                 0 open +2e 6 s |              18/18 0 open +0e 18 s
briskspirit__Sisu_SSE-9__Sisu_SSE-9                                  |                 timeout 1501 s |         184/192 11 open +2e 1420 s
byrantech__laptop__keyboard                                          |             24 open +11e 644 s |           131/136 4 open +0e 478 s
byrantech__laptop__motherboard                                       |                 timeout 1500 s |           234/236 2 open +0e 997 s
byrantech__laptop__power                                             |              0 open +100e 24 s |            115/115 0 open +0e 91 s
crmaykish__mackerel-68k__mackerel-08-v1                              |               3 open +0e 101 s |             96/96 0 open +0e 230 s
crmaykish__mackerel-68k__mackerel-10-v1                              |               1 open +5e 263 s |           155/155 0 open +0e 544 s
crmaykish__mackerel-68k__mackerel-30-proto                           |             0 open +589e 257 s |          217/217 0 open +13e 501 s
doudar__SmartSpin2k__SmartSpin2k_Panelized                           |                 timeout 1500 s |           585/585 0 open +0e 113 s
duckyb__eternal-keypad__eternal-keypad                               |                0 open +0e 22 s |              69/69 0 open +0e 74 s
duckyb__urchin__main                                                 |                6 open +0e 80 s |              68/68 0 open +0e 13 s
earth75__atat-1800__ATAT1800                                         |                 timeout 1500 s |         255/278 31 open +0e 1800 s
ebastler__osprey__osprey_rev_a                                       |             21 open +6e 1069 s |           101/103 2 open +0e 343 s
emertcakir__OpenAirScope__OpenAirScope                               |               3 open +1e 282 s |           179/184 5 open +0e 727 s
greatscottgadgets__hackrf__hackrf-one                                |             5 open +220e 750 s |           319/319 0 open +0e 441 s
hackclub__OnBoard__E-Fidget-Lite                                     |                 0 open +0e 7 s |              20/20 0 open +0e 15 s
hackclub__OnBoard__MotionCubeViewAllForces                           |               10 open +5e 21 s |              20/20 0 open +0e 77 s
hackclub__OnBoard__PixelWave                                         |                 timeout 1500 s |           343/343 0 open +0e 484 s
hackclub__OnBoard__keyboar_                                          |                 timeout 1500 s |           102/102 0 open +0e 128 s
hackclub__OnBoard__koeg-board-pcb                                    |              export failed 0 s |        169/175 11 open +15e 1220 s
hackclub__OnBoard__krishveercard                                     |               93 open +0e 86 s |            23/43 21 open +0e 699 s
hackclub__OnBoard__woagboard                                         |                1 open +0e 92 s |            90/91 47 open +0e 206 s
headblockhead__slab-pcb__interchange-pcb-right                       |             47 open +30e 413 s |             83/83 8 open +0e 313 s
headblockhead__slab-pcb__slab-pcb-left                               |              54 open +0e 452 s |             76/76 0 open +0e 156 s
iandchasse__silkscreen-pcb__silkscreen_pcb                           |               9 open +26e 93 s |            113/113 0 open +0e 70 s
ikajdan__katia__katia                                                |                 timeout 1500 s |         136/162 84 open +0e 1800 s
little-red-rover__little-red-rover__little_red_rover                 |              8 open +19e 129 s |             64/64 0 open +0e 258 s
maniekx86__M8SBC-486__homebrew_486                                   |               0 open +3e 363 s |           190/190 0 open +0e 724 s
obsilab__Quanta75__Quanta75_BareRP2040_JLCPCBAoptimized              |              63 open +0e 206 s |           167/172 9 open +0e 675 s
obsilab__Quanta75__Quanta75_RP2040Stamp_JLCPCBAoptimized             |                2 open +0e 20 s |           162/162 0 open +0e 102 s
ohdsp__DSP-ADAU1452__DSP-ADAU1452                                    |             58 open +0e 1210 s |           189/190 7 open +0e 678 s
oro-os__link__link                                                   |                 timeout 1501 s |         248/251 10 open +0e 1734 s
rosco-m68k__rosco_m68k__rosco_m68k__kicad                            |               1 open +6e 184 s |           148/148 0 open +2e 347 s
siderakb__ergo-snm-keyboard__ErgoSNM_keyboard                        |              17 open +1e 215 s |              58/58 0 open +0e 39 s
sporkus__capybully_keyboard__capybully                               |                 timeout 1500 s |            32/32 50 open +0e 622 s
sporkus__le_chiffre_keyboard_stm32__stm32_chiffre_36keys             |               6 open +0e 145 s |              76/76 0 open +0e 91 s
sporkus__le_chiffre_keyboard_stm32__stm32_hotswap_chiffre            |               8 open +0e 276 s |              75/75 0 open +0e 78 s
stonedDiscord__MegaDrive__MegaDrive                                  |                 timeout 1500 s |          274/298 95 open +0e 387 s
stonedDiscord__nonSNES__SNSP-CPU-01                                  |                 timeout 1500 s |          361/363 55 open +0e 519 s
stonedDiscord__nonSNES__SNSP-CPU-1CHIP                               |              2 open +16e 147 s |           212/212 0 open +0e 178 s
tengigabytes__MokyaLora__MokyaLora                                   |                 timeout 1500 s |           261/262 1 open +0e 767 s
thpoll83__PolyKybd__poly_corne_split42_left                          |                 timeout 1500 s |          247/247 0 open +0e 1463 s
thpoll83__PolyKybd__poly_corne_split42_right                         |                 timeout 1500 s |           420/420 0 open +0e 989 s
thpoll83__PolyKybd__poly_kb_molecule_4x4                             |                 timeout 1500 s |            9/28 499 open +0e 870 s
thpoll83__PolyKybd__poly_kb_molecule_4x5                             |                 timeout 1500 s |            6/28 499 open +0e 871 s
thpoll83__PolyKybd__poly_kb_molecule_5x2_shifted                     |                 timeout 1500 s |            6/28 499 open +0e 304 s
thpoll83__PolyKybd__poly_kb_molecule_5x4_wave                        |                 timeout 1500 s |            6/28 499 open +0e 529 s
thpoll83__PolyKybd__poly_kb_molecule_7x5_wave_left                   |                 timeout 1500 s |            5/28 499 open +0e 658 s
thpoll83__PolyKybd__poly_kybd_split72_left                           |                 timeout 1500 s |          420/420 0 open +0e 1735 s
thpoll83__PolyKybd__poly_kybd_split72_right                          |                 timeout 1500 s |           420/420 0 open +0e 967 s
tomunderwood99__CharlieBoard__Blue_Line                              |                0 open +0e 10 s |              26/26 0 open +0e 21 s
transistorfet__computie__k30-SBC                                     |               0 open +0e 269 s |           173/173 0 open +0e 528 s
tubbytwins__bumwings-kbd__bumwings_v001                              |              16 open +0e 388 s |              72/72 0 open +0e 75 s
tubbytwins__bumwings-kbd__bumwings_v001R55_rp2040zero_sd             |              14 open +0e 116 s |              70/70 0 open +0e 55 s
tubbytwins__bumwings-kbd__bumwings_v001R55_xiao_sd                   |              14 open +0e 270 s |              75/75 0 open +0e 66 s
tubbytwins__bumwings-kbd__bumwings_v001R64_nano_sd                   |            import failed 266 s |              85/85 0 open +0e 52 s
tubbytwins__bumwings-kbd__bumwings_v001R64_rp2040zero_sd             |              17 open +0e 111 s |              81/81 0 open +0e 57 s
tubbytwins__bumwings-kbd__bumwings_v001R64_xiao_sd                   |              17 open +0e 397 s |              89/89 0 open +0e 86 s
tubbytwins__bumwings-kbd__bumwings_v001_core                         |             20 open +0e 1094 s |             98/98 0 open +0e 102 s
tubbytwins__bumwings-kbd__bumwings_v001_xiao                         |              21 open +0e 762 s |             81/81 0 open +0e 138 s
tubbytwins__bumwings-kbd__bumwings_v001_xiao_s                       |              19 open +0e 401 s |              78/78 0 open +0e 92 s
tzarc__keyboards__ghoul                                              |             10 open +59e 358 s |           142/148 6 open +0e 333 s
vd-rd__sbc_allwinner_a13__module                                     |                 timeout 1500 s |          218/223 22 open +3e 981 s
wntrblm__Castor_and_Pollux__mainboard                                |                1 open +0e 28 s |           126/126 0 open +0e 104 s
zli117__CyberKeeb2040__MainBoard                                     |              13 open +0e 676 s |           118/118 0 open +0e 246 s

109 boards; Freerouting finished 70, timed out 34, failed 5
where Freerouting finished: pcb-maker fewer unconnected on 49, Freerouting fewer on 6; unconnected in all: Freerouting 913, pcb-maker 206; clean boards: Freerouting 8, pcb-maker 50

This supersedes the 2026-09-22 head-to-head below, which used the earlier
adapter.

## Boards harvested from GitHub (added 2026-09-30)

Florian's direction: more benchmarks, from open-source boards on GitHub,
routed on the designer's placement and placed under constraints derived from
it. [benchmarks/github/README.md](../benchmarks/github/README.md) describes
the harvest (`harvest.py search | fetch | boards`): KiCad 7+ boards found by
code search (generator version and package kind) and by repository topic
(files over 300 kB), fetched at the head commit, kept when they are finished
designs of some size. First harvest: 109 boards from 62 repositories, 44
with four copper layers, 12 with six, 2 with eight; 38 with 100 or more
fine pads (a side of 0.3 mm or less). `manifest.json` records origin,
commit, licence and the designer's numbers; the files stay out of the
repository.

Route mode: `benchmarks/corpus/run.py <out> --corpus benchmarks/github/boards.json --skip-layout`,
summarised by cause with `benchmarks/github/summary.py <out>`. Layout tasks:
`benchmarks/agent-tasks/from_pcbench.py <tasks> --corpus benchmarks/github/boards.json`
then `benchmarks/agent-tasks/run.py <out> --tasks <tasks>/tasks.json`.
To measure a change (short tier, route mode from fixed placements, spliced
reruns, the objectives line), see benchmarks/agent-tasks/README.md, "How to
measure".

From the schematic (2026-10-15): `harvest_schematics.py` fetched the
schematics and project libraries of 79 of the 109 boards; the schematic
tier (`run.py --tier schematic`, benchmarks/agent-tasks/README.md, "The
schematic tier") imports 12 of them with `import-kicad-netlist` and lays
them out at a third of the budget: 4/12 pass on x1002, 5/12 with the
branch's import fixes (x1030); where the import and the libraries agree
with the designer's board they do as the layout tier does.

What the first boards found, before any full run finished (fixed the same
day): chamfered roundrect pads were refused (4 boards), a rule area's holes
were read as more keepouts (Glasgow's rim keepout covered the whole board,
0/226), and the ladder's first rung could spend the whole budget on a
hopeless configuration (video).

### Second sweep, 92 boards, with the day's fixes (`pcb-maker-x8`, 900 s each, 2026-10-01)

17 clean, 42 open, 11 with copper errors beyond the designer's (KiCad DRC),
1 mismatch (KiCad finds an unconnected item the router does not), 21
errors. Corrected on 2026-10-03: the first version of this table said 25
clean, because `summary.py` read a key the runner does not write and so
never counted DRC errors (one board it called clean had 128 hole
clearance errors, the copper-to-drill rule the router did not know yet).
Via counts are mostly below the designer's. 17 of the 109 boards were
skipped (strip failures since fixed).

| Board | Layers | Footprints | Result | Vias (designer) | Time |
| --- | --- | --- | --- | --- | --- |
| 0xCB-dev__0xCB-Static__0xcb-static | 2 | 129 | clean: 74/74, 2 vias (designer 0), 25 s | 0 | |
| CRImier__MyKiCad__framework_mobo_lefthalf | 6 | 45 | clean: 71/71, 76 vias (designer 203), 101 s | 203 | |
| ISSUIUC__ISS-PCB__BAGEL-MK1 | 4 | 199 | clean: 138/138, 151 vias (designer 525), 395 s | 525 | |
| Jana-Marie__ligra__ligra_back | 2 | 61 | clean: 33/33, 11 vias (designer 251), 62 s | 251 | |
| Open-Muscle__OpenMuscle-FlexGrid__OM-60-Flex | 2 | 62 | clean: 21/21, 39 vias (designer 120), 146 s | 120 | |
| Open-Muscle__OpenMuscle-FlexGrid__OM-FlexGrid-Flex__OM-FlexGrid-Flex | 2 | 61 | clean: 19/19, 65 vias (designer 120), 275 s | 120 | |
| bitshiftcrazy__d20_pcb__d20_pcb | 2 | 39 | clean: 19/19, 34 vias (designer 46), 57 s | 46 | |
| crmaykish__mackerel-68k__mackerel-08-v1 | 4 | 86 | clean: 96/96, 64 vias (designer 208), 488 s | 208 | |
| duckyb__urchin__main | 2 | 122 | clean: 68/68, 7 vias (designer 53), 14 s | 53 | |
| hackclub__OnBoard__E-Fidget-Lite | 2 | 29 | clean: 20/20, 7 vias (designer 0), 7 s | 0 | |
| hackclub__OnBoard__MotionCubeViewAllForces | 2 | 30 | clean: 20/20, 35 vias (designer 29), 56 s | 29 | |
| hackclub__OnBoard__keyboar_ | 2 | 158 | clean: 104/104, 20 vias (designer 0), 176 s | 0 | |
| sporkus__le_chiffre_keyboard_stm32__stm32_chiffre_36keys | 2 | 118 | clean: 76/76, 61 vias (designer 0), 175 s | 0 | |
| sporkus__le_chiffre_keyboard_stm32__stm32_hotswap_chiffre | 2 | 110 | clean: 75/75, 42 vias (designer 0), 111 s | 0 | |
| tomunderwood99__CharlieBoard__Blue_Line | 2 | 25 | clean: 26/26, 26 vias (designer 28), 4 s | 28 | |
| tubbytwins__bumwings-kbd__bumwings_v001 | 2 | 120 | clean: 72/72, 22 vias (designer 0), 93 s | 0 | |
| tubbytwins__bumwings-kbd__bumwings_v001_core | 2 | 153 | clean: 98/98, 71 vias (designer 0), 119 s | 0 | |
| CRImier__MyKiCad__vaio_re | 4 | 289 | open: 37 open; 230/267, 397 vias (designer 523), 464 s | 523 | |
| CRImier__MyKiCad__zpn_devboard | 4 | 168 | open: 1 open; 166/167, 149 vias (designer 298), 641 s | 298 | |
| GlasgowEmbedded__glasgow__glasgow__revC3 | 4 | 272 | open: 61 open; 165/226, 206 vias (designer 0), 454 s | 0 | |
| Huaqiu-Electronics__ecad-viewer__video | 4 | 189 | open: 48 open; 323/371, 990 vias (designer 808), 846 s | 808 | |
| ISSUIUC__ISS-PCB__BAGEL-MK1.1-Avocado | 4 | 212 | open: 9 open; 135/144, 266 vias (designer 820), 322 s | 820 | |
| ISSUIUC__ISS-PCB__MIDAS-MK1.1__MIDAS-MK1.1-revA | 4 | 216 | open: 5 open; 151/156, 131 vias (designer 0), 73 s | 0 | |
| ISSUIUC__ISS-PCB__MIDAS-MK1__MIDAS-MK1-revA | 4 | 221 | open: 5 open; 154/159, 139 vias (designer 0), 45 s | 0 | |
| ISSUIUC__ISS-PCB__MIDAS-MK2 | 4 | 217 | open: 1 open; 155/156, 167 vias (designer 401), 157 s | 401 | |
| ISSUIUC__ISS-PCB__MIDAS-MK2.1__MIDAS-MK2.1-revA | 4 | 219 | open: 1 open; 153/154, 163 vias (designer 432), 337 s | 432 | |
| Ladniy__jiran-ble-lite__jiran-ble-lite | 2 | 154 | open: 1 open; 104/105, 54 vias (designer 0), 48 s | 0 | |
| MbFredys__PCB-Modular-Multi-Protocol-Hub__Hub | 4 | 235 | open: 56 open; 103/159, 102 vias (designer 331), 224 s | 331 | |
| Neotron-Compute__Neotron-Pico__neotron-pico | 4 | 242 | open: 14 open; 176/190, 155 vias (designer 0), 505 s | 0 | |
| OpenDrone-hw__OpenFC-Lite__OpenFC | 6 | 164 | open: 1 open; 81/82, 121 vias (designer 596), 140 s | 596 | |
| OpenDrone-hw__OpenRX__OpenRX-panel-rev2 | 6 | 254 | open: 50 open; 53/103, 125 vias (designer 538), 465 s | 538 | |
| Seeed-Studio__OSHW-reCamera-Series__reCamera_S101_v1.1 | 4 | 92 | open: 2 open; 54/56, 44 vias (designer 334), 17 s | 334 | |
| Spaceflight-Rocketry-Giessen-e-V__Telemetry__TelemetryOnboard | 4 | 143 | open: 3 open; 105/108, 71 vias (designer 269), 148 s | 269 | |
| Twisted-Fields__rp2040-motor-controller__RP2040_base | 4 | 325 | open: 71 open; 115/186, 233 vias (designer 553), 82 s | 553 | |
| apfaudio__eurorack-pmod__eurorack-pmod-pcb | 6 | 155 | open: 2 open; 131/133, 133 vias (designer 331), 242 s | 331 | |
| baldengineer__bit-preserve__coco2 | 2 | 156 | open: 1 open; 172/173, 315 vias (designer 0), 713 s | 0 | |
| bismarx-v1__Sumec-MiniSumo__SUMEC_MK_IV | 4 | 122 | open: 12 open; 57/69, 89 vias (designer 541), 33 s | 541 | |
| briskspirit__Sisu_SSE-9__Sisu_SSE-9 | 6 | 296 | open: 8 open; 184/192, 239 vias (designer 1076), 492 s | 1076 | |
| byrantech__laptop__keyboard | 2 | 220 | open: 4 open; 132/136, 173 vias (designer 571), 276 s | 571 | |
| byrantech__laptop__motherboard | 6 | 264 | open: 3 open; 233/236, 378 vias (designer 734), 641 s | 734 | |
| ebastler__osprey__osprey_rev_a | 2 | 160 | open: 3 open; 100/103, 120 vias (designer 0), 87 s | 0 | |
| emertcakir__OpenAirScope__OpenAirScope | 4 | 282 | open: 6 open; 178/184, 146 vias (designer 473), 254 s | 473 | |
| greatscottgadgets__hackrf__hackrf-one | 4 | 437 | open: 3 open; 316/319, 396 vias (designer 0), 664 s | 0 | |
| hackclub__OnBoard__krishveercard | 4 | 39 | open: 14 open; 29/43, 38 vias (designer 87), 167 s | 87 | |
| headblockhead__slab-pcb__interchange-pcb-right | 2 | 178 | open: 2 open; 81/83, 242 vias (designer 203), 236 s | 203 | |
| headblockhead__slab-pcb__slab-pcb-left | 2 | 128 | open: 2 open; 76/78, 192 vias (designer 239), 192 s | 239 | |
| little-red-rover__little-red-rover__little_red_rover | 2 | 65 | open: 1 open; 63/64, 110 vias (designer 0), 76 s | 0 | |
| maniekx86__M8SBC-486__homebrew_486 | 4 | 130 | open: 2 open; 188/190, 1102 vias (designer 1150), 717 s | 1150 | |
| obsilab__Quanta75__Quanta75_BareRP2040_JLCPCBAoptimized | 2 | 276 | open: 25 open; 147/172, 53 vias (designer 95), 769 s | 95 | |
| obsilab__Quanta75__Quanta75_RP2040Stamp_JLCPCBAoptimized | 2 | 253 | open: 1 open; 161/162, 31 vias (designer 0), 290 s | 0 | |
| ohdsp__DSP-ADAU1452__DSP-ADAU1452 | 4 | 377 | open: 27 open; 163/190, 41 vias (designer 355), 94 s | 355 | |
| rosco-m68k__rosco_m68k__rosco_m68k__kicad | 4 | 140 | open: 1 open; 147/148, 416 vias (designer 402), 648 s | 402 | |
| stonedDiscord__nonSNES__SNSP-CPU-01 | 2 | 400 | open: 2 open; 361/363, 1202 vias (designer 1197), 319 s | 1197 | |
| stonedDiscord__nonSNES__SNSP-CPU-1CHIP | 2 | 241 | open: 1 open; 211/212, 359 vias (designer 686), 555 s | 686 | |
| tengigabytes__MokyaLora__MokyaLora | 6 | 443 | open: 4 open; 258/262, 172 vias (designer 836), 611 s | 836 | |
| thpoll83__PolyKybd__poly_kb_molecule_5x2_shifted | 2 | 392 | open: 23 open; 4/27, 199 vias (designer 0), 701 s | 0 | |
| tzarc__keyboards__ghoul | 2 | 191 | open: 6 open; 142/148, 244 vias (designer 0), 57 s | 0 | |
| vd-rd__sbc_allwinner_a13__module | 4 | 179 | open: 41 open; 182/223, 361 vias (designer 501), 366 s | 501 | |
| wntrblm__Castor_and_Pollux__mainboard | 4 | 235 | open: 1 open; 125/126, 121 vias (designer 358), 234 s | 358 | |
| 0xCB-dev__0xCB-1337__1337-v4.0 | 2 | 64 | drc: {'starved_thermal': 3, 'copper_sliver': 6, 'track_dangling': 1}; 47/47, 44 vias (designer 0), 161 s | 0 | |
| 0xCB-dev__0xCB-1337__pcb | 4 | 119 | drc: {'connection_width': 3}; 85/85, 87 vias (designer 0), 107 s | 0 | |
| anyshake__explorer__Explorer | 2 | 266 | drc: {'starved_thermal': 1}; 150/150, 117 vias (designer 1057), 265 s | 1057 | |
| bitshiftcrazy__spell_tome__spell_tome_bottom | 2 | 25 | drc: {'starved_thermal': 2}; 18/18, 7 vias (designer 6), 15 s | 6 | |
| byrantech__laptop__power | 4 | 179 | drc: {'hole_clearance': 93}; 115/115, 91 vias (designer 167), 285 s | 167 | |
| hackclub__OnBoard__PixelWave | 2 | 663 | drc: {'starved_thermal': 5, 'track_dangling': 8}; 343/343, 708 vias (designer 705), 117 s | 705 | |
| iandchasse__silkscreen-pcb__silkscreen_pcb | 2 | 188 | drc: {'starved_thermal': 2}; 113/113, 138 vias (designer 192), 28 s | 192 | |
| siderakb__ergo-snm-keyboard__ErgoSNM_keyboard | 2 | 98 | drc: {'clearance': 8}; 58/58, 12 vias (designer 0), 39 s | 0 | |
| sporkus__capybully_keyboard__capybully | 4 | 114 | drc: {'copper_sliver': 3, 'starved_thermal': 2, 'via_dangling': 4}; 32/32, 19 vias (designer 0), 236 s | 0 | |
| tubbytwins__bumwings-kbd__bumwings_v001_xiao | 2 | 127 | drc: {'clearance': 15}; 81/81, 47 vias (designer 0), 127 s | 0 | |
| zli117__CyberKeeb2040__MainBoard | 2 | 182 | drc: {'hole_to_hole': 13, 'via_dangling': 1}; 118/118, 169 vias (designer 0), 474 s | 0 | |
| duckyb__eternal-keypad__eternal-keypad | 2 | 93 | mismatch: 2 unconnected in KiCad; 63/63, 35 vias (designer 0), 73 s | 0 | |
| antmicro__jetson-nano-baseboard__jetson-nano-baseboard | 8 | 391 | error: iteration 24: rerouted 31, conflicted 28, present 8417.06, 613.61s (search 609.15s, stamp 4.41s, 887 | 794 | |
| crmaykish__mackerel-68k__mackerel-10-v1 | 4 | 114 | error: iteration 18: rerouted 2, conflicted 0, present 738.95, 284.20s (search 618.13s, stamp 23.70s, 12682 | 200 | |
| crmaykish__mackerel-68k__mackerel-30-proto | 4 | 133 | error: iteration 25: rerouted 2, conflicted 0, present 10000.00, 235.74s (search 544.77s, stamp 14.97s, 156 | 294 | |
| earth75__atat-1800__ATAT1800 | 4 | 342 | error: iteration 33: rerouted 58, conflicted 57, present 10000.00, 613.74s (search 598.48s, stamp 15.17s, 5 | 680 | |
| hackclub__OnBoard__koeg-board-pcb | 4 | 290 | error: error: kicad-cli failed with exit status: 3: Failed to load board: One or more items were found on u | 417 | |
| oro-os__link__link | 8 | 381 | error: cleanup: 329 improvements, 181.95s | 806 | |
| stonedDiscord__MegaDrive__MegaDrive | 2 | 291 | error: pour GND: 2242 pieces, 70 stranded terminals (62 touch a piece) | 785 | |
| thpoll83__PolyKybd__poly_corne_split42_right | 4 | 573 | error: iteration 79: rerouted 8, conflicted 7, present 10000.00, 602.03s (search 586.43s, stamp 15.18s, 131 | 1372 | |
| thpoll83__PolyKybd__poly_kb_molecule_4x4 | 2 | 576 | error: iteration 26: rerouted 26, conflicted 26, present 10000.00, 315.43s (search 307.18s, stamp 8.12s, 23 | 0 | |
| thpoll83__PolyKybd__poly_kb_molecule_4x5 | 2 | 716 | error: iteration 26: rerouted 26, conflicted 26, present 10000.00, 453.07s (search 442.42s, stamp 10.47s, 2 | 0 | |
| thpoll83__PolyKybd__poly_kb_molecule_5x4_wave | 2 | 393 | error: iteration 26: rerouted 26, conflicted 26, present 10000.00, 349.15s (search 339.69s, stamp 9.33s, 22 | 0 | |
| thpoll83__PolyKybd__poly_kb_molecule_7x5_wave_left | 2 | 629 | error: iteration 24: rerouted 26, conflicted 26, present 8417.06, 626.26s (search 612.42s, stamp 13.62s, 30 | 0 | |
| thpoll83__PolyKybd__poly_kybd_split72_left | 4 | 573 | error: iteration 49: rerouted 2, conflicted 0, present 10000.00, 351.86s (search 343.13s, stamp 8.51s, 1082 | 1354 | |
| thpoll83__PolyKybd__poly_kybd_split72_right | 4 | 573 | error: iteration 79: rerouted 4, conflicted 4, present 10000.00, 488.28s (search 477.32s, stamp 10.63s, 113 | 1368 | |
| transistorfet__computie__k30-SBC | 4 | 88 | error: iteration 18: rerouted 4, conflicted 0, present 738.95, 480.52s (search 665.08s, stamp 10.06s, 12090 | 0 | |
| tubbytwins__bumwings-kbd__bumwings_v001R55_rp2040zero_sd | 2 | 120 | error: error: unknown copper layer "In1.Cu" | 0 | |
| tubbytwins__bumwings-kbd__bumwings_v001R55_xiao_sd | 2 | 121 | error: error: unknown copper layer "In1.Cu" | 0 | |
| tubbytwins__bumwings-kbd__bumwings_v001R64_nano_sd | 2 | 135 | error: error: unknown copper layer "In1.Cu" | 0 | |
| tubbytwins__bumwings-kbd__bumwings_v001R64_rp2040zero_sd | 2 | 130 | error: error: unknown copper layer "In1.Cu" | 0 | |
| tubbytwins__bumwings-kbd__bumwings_v001R64_xiao_sd | 2 | 132 | error: error: unknown copper layer "In1.Cu" | 0 | |
| tubbytwins__bumwings-kbd__bumwings_v001_xiao_s | 2 | 122 | error: error: unknown copper layer "In1.Cu" | 0 | |

92 boards: clean 17, open 42, drc 11, mismatch 1, error 21


### First sweep, 38 boards, before the day's fixes (`pcb-maker-x4`, 900 s each)

Stopped after 38 boards once its failure classes were clear, so the machine
could rerun everything with the fixes (`build/github-route-v2`, table to
follow). Times are under a loaded machine.

| Board | Layers | Footprints | Result | Vias (designer) | Time |
| --- | --- | --- | --- | --- | --- |
| ISSUIUC__ISS-PCB__BAGEL-MK1 | 4 | 199 | clean: 138/138, 156 vias (designer 525), 420 s | 525 | |
| CRImier__MyKiCad__framework_mobo_lefthalf | 6 | 45 | open: 1 open; 70/71, 81 vias (designer 203), 41 s | 203 | |
| CRImier__MyKiCad__vaio_re | 4 | 289 | open: 31 open; 236/267, 405 vias (designer 523), 485 s | 523 | |
| CRImier__MyKiCad__zpn_devboard | 4 | 168 | open: 1 open; 166/167, 164 vias (designer 298), 348 s | 298 | |
| GlasgowEmbedded__glasgow__glasgow | ? | ? | open: 226 open; 0/226, 0 vias (designer None), 2 s | ? | |
| GlasgowEmbedded__glasgow__glasgow | ? | ? | open: 226 open; 0/226, 0 vias (designer None), 2 s | ? | |
| ISSUIUC__ISS-PCB__BAGEL-MK1.1-Avocado | 4 | 212 | open: 10 open; 134/144, 269 vias (designer 820), 170 s | 820 | |
| Neotron-Compute__Neotron-Pico__neotron-pico | 4 | 242 | open: 14 open; 176/190, 185 vias (designer 0), 298 s | 0 | |
| OpenDrone-hw__OpenFC-Lite__OpenFC | 6 | 164 | open: 1 open; 81/82, 121 vias (designer 596), 82 s | 596 | |
| OpenDrone-hw__OpenRX__OpenRX-panel-rev2 | 6 | 254 | open: 47 open; 56/103, 117 vias (designer 538), 482 s | 538 | |
| Seeed-Studio__OSHW-reCamera-Series__reCamera_S101_v1.1 | 4 | 92 | open: 2 open; 54/56, 44 vias (designer 334), 18 s | 334 | |
| Spaceflight-Rocketry-Giessen-e-V__Telemetry__TelemetryOnboard | 4 | 143 | open: 3 open; 105/108, 73 vias (designer 269), 82 s | 269 | |
| Twisted-Fields__rp2040-motor-controller__RP2040_base | 4 | 325 | open: 71 open; 115/186, 243 vias (designer 553), 98 s | 553 | |
| apfaudio__eurorack-pmod__eurorack-pmod-pcb | 6 | 155 | open: 2 open; 131/133, 130 vias (designer 331), 279 s | 331 | |
| bismarx-v1__Sumec-MiniSumo__SUMEC_MK_IV | 4 | 122 | open: 12 open; 57/69, 83 vias (designer 541), 64 s | 541 | |
| briskspirit__Sisu_SSE-9__Sisu_SSE-9 | 6 | 296 | open: 10 open; 182/192, 466 vias (designer 1076), 303 s | 1076 | |
| byrantech__laptop__motherboard | 6 | 264 | open: 3 open; 233/236, 378 vias (designer 734), 656 s | 734 | |
| emertcakir__OpenAirScope__OpenAirScope | 4 | 282 | open: 6 open; 178/184, 150 vias (designer 473), 328 s | 473 | |
| greatscottgadgets__hackrf__hackrf-one | 4 | 437 | open: 4 open; 315/319, 394 vias (designer 0), 150 s | 0 | |
| hackclub__OnBoard__krishveercard | 4 | 39 | open: 30 open; 13/43, 27 vias (designer 87), 2 s | 87 | |
| maniekx86__M8SBC-486__homebrew_486 | 4 | 130 | open: 6 open; 184/190, 1018 vias (designer 1150), 713 s | 1150 | |
| ohdsp__DSP-ADAU1452__DSP-ADAU1452 | 4 | 377 | open: 28 open; 162/190, 48 vias (designer 355), 166 s | 355 | |
| wntrblm__Castor_and_Pollux__mainboard | 4 | 235 | open: 1 open; 125/126, 125 vias (designer 358), 239 s | 358 | |
| 0xCB-dev__0xCB-1337__pcb | 4 | 119 | error: error: chamfered roundrect pads have no exact geometry lowering | 0 | |
| Huaqiu-Electronics__ecad-viewer__video | 4 | 189 | error: cleanup: 582 improvements, 72.89s | 808 | |
| ISSUIUC__ISS-PCB__MIDAS-MK2 | 4 | 217 | error: error: chamfered roundrect pads have no exact geometry lowering | 401 | |
| MbFredys__PCB-Modular-Multi-Protocol-Hub__Hub | 4 | 235 | error: error: chamfered roundrect pads have no exact geometry lowering | 331 | |
| antmicro__jetson-nano-baseboard__jetson-nano-baseboard | 8 | 391 | error: iteration 17: rerouted 72, conflicted 60, present 492.63, 607.19s (search 603.21s, stamp 3.93s, 8240 | 794 | |
| byrantech__laptop__power | 4 | 179 | error: error: chamfered roundrect pads have no exact geometry lowering | 167 | |
| crmaykish__mackerel-68k__mackerel-30-proto | 4 | 133 | error: iteration 20: rerouted 6, conflicted 0, present 1662.63, 230.77s (search 509.21s, stamp 14.17s, 1548 | 294 | |
| earth75__atat-1800__ATAT1800 | 4 | 342 | error: iteration 32: rerouted 56, conflicted 53, present 10000.00, 605.68s (search 590.51s, stamp 15.06s, 4 | 680 | |
| hackclub__OnBoard__koeg-board-pcb | 4 | 290 | error: error: chamfered roundrect pads have no exact geometry lowering | 417 | |
| oro-os__link__link | 8 | 381 | error: error: chamfered roundrect pads have no exact geometry lowering | 806 | |
| tengigabytes__MokyaLora__MokyaLora | 6 | 443 | error: error: per-layer roundrect padstacks have no exact geometry lowering | 836 | |
| thpoll83__PolyKybd__poly_corne_split42_right | 4 | 573 | error: error: chamfered roundrect pads have no exact geometry lowering | 1372 | |
| thpoll83__PolyKybd__poly_kybd_split72_left | 4 | 573 | error: error: chamfered roundrect pads have no exact geometry lowering | 1354 | |
| thpoll83__PolyKybd__poly_kybd_split72_right | 4 | 573 | error: error: chamfered roundrect pads have no exact geometry lowering | 1368 | |
| vd-rd__sbc_allwinner_a13__module | 4 | 179 | error: error: chamfered roundrect pads have no exact geometry lowering | 501 | |

38 boards: clean 1, open 22, drc 0, mismatch 0, error 15


## Breadboard fence boards (added 2026-09-25)

[`benchmarks/fence/`](../benchmarks/fence/README.md) holds the boards of a
separate hardware project that uses pcb-maker as its placer and router: a
dense 76 × 31 mm board where 30 row nets each cross between a resistor
network and one of two LQFP-44 crosspoint switches. Five of them are
`corpus.json` entries (`fence-*`): the current 4-layer boards (complete so
far), the same boards on 2 layers (not routable so far) and a footprint
keepout test. Its README also carries an issue log of what pcb-maker got
wrong on these boards with a benchmark and a pass criterion for each, and a
re-run of everything on 2026-10-07: footprint keepouts, exclusive planes,
constraints and pin swapping all work there now; the 31 mm boards still do
not route on 2 layers (8 mm taller they do, with the GND pour in pieces),
silkscreen stays unplaced.

**v5 (2026-10-21):** `benchmarks/fence/v5-4layer/fence` (the shipped board:
41 LEDs in a 2.54 mm row, a stack header, three `keepout` constraints;
complete with `router-seeds.json`, 134/134, 196 vias) and
`v5-2layer/fence` (the same board on 2 layers at 34 mm with 0.127 mm rules:
7 open at best, the B.Cu GND pour in 43 pieces; 12 mm taller it completes
with the pour in 37 pieces). Corpus entries `fence-v5-4l` and `fence-v5-2l`.
The pass criterion for the 2-layer board includes the pour
(`benchmarks/fence/pour-pieces.py`). New issues 13–17 in that README:
vias inside LED pads, moved silkscreen text, keepouts judging fixed parts,
`repair-kicad-silkscreen` on a `gr_rect` outline, and no way to keep a
2-layer pour whole.

## Layout sweep v15 (2026-10-14): the spoke model, solid bodies, moving rule areas

`pcb-maker-x723` (b63a604: the KiCad-exact thermal spoke model with manual
spokes, island vias and stubs, the mask-opening clearance from the project,
courtyard-less pin-row footprints blocking with their pad box, mask-only
pads as openings, footprint rule areas moving with their footprint,
`keep_on_pieces`, far-side and inner-layer pad keepouts) on the v6 tasks, 6
jobs, 17910 s of wall clock: **45 of 109 pass** (v14: 34); against v14
better on 20, same on 63, worse on 16; 3700 to 3829 open in all (video's
499 is KiCad's cap; the 0xCB panel 129 to 210, the allwinner module 8 to
81 and RP2040_base 100 to 173 carry the rise). Eleven boards that failed
only on starved thermals or a stray error pass now (PixelWave, k30,
mackerel-08 and -30, rosco, Castor, Telemetry, MokyaLora, spell_tome, two
bumwings), zpn_devboard routes clean (its parts had spread over an empty
breakaway), and link finishes inside the limit. Error rows: the five
molecule panels and the OpenESC 4in1 panel find no legal placement; katia,
MegaDrive and the 0xCB pcb-panel hit the 1800 s limit. The worse rows are
the crowded boards (module, jetson, vaio, woagboard, protoesp, the 0xCB
panel, video), where a placement change flips the result by tens of
connections run to run (the routing-room agent's seed pairs; video is not
even deterministic for one placement, under investigation), and where the
pin-row rule now forbids the parts-inside-bodies placements that v14
counted.

Better (first 20 rows of `compare.py`):

board                                                                |                   before |                    after |      s
-------------------------------------------------------------------- | ------------------------ | ------------------------ | ------
OpenDrone-hw__OpenESC-30x30__4in1                                    |               61/152 239 |               97/152 124 |    935
CRImier__MyKiCad__zpn_devboard                                       |               128/167 71 |           167/167 0 pass |    855
ohdsp__DSP-ADAU1452__DSP-ADAU1452                                    |         118/190 167 +49e |         146/190 105 +59e |   1713
OpenDrone-hw__OpenESC-20x20__4in1-mini                               |                95/152 96 |               123/152 38 |    675
OpenDrone-hw__OpenRX__OpenRX-panel-rev2                              |               44/127 257 |               56/127 236 |   1263
thpoll83__PolyKybd__poly_corne_split42_right                         |           393/420 57 +1e |               398/420 46 |   1719
briskspirit__Sisu_SSE-9__Sisu_SSE-9                                  |           188/192 18 +1e |                188/192 9 |   1206
apfaudio__eurorack-pmod__eurorack-pmod-pcb                           |               116/133 27 |               125/133 20 |   1014
Seeed-Studio__OSHW-reCamera-Series__reCamera_S101_v1.1               |                 53/56 10 |                  55/56 4 |    402
CDFER__Business-Cards__WLED_Matrix                                   |             99/104 9 +8e |           102/104 4 +13e |    690
obsilab__Quanta75__Quanta75_BareRP2040_JLCPCBAoptimized              |         146/172 128 +70e |         148/172 123 +73e |   1708
thpoll83__PolyKybd__poly_kybd_split72_left                           |               394/420 55 |               396/420 52 |   1696
ISSUIUC__ISS-PCB__MIDAS-MK1.1__MIDAS-MK1.1-revA                      |               150/156 12 |               151/156 11 |    743
OpenDrone-hw__OpenFC-Lite__OpenFC                                    |                  81/82 2 |                  81/82 1 |    300
byrantech__laptop__motherboard                                       |                233/236 3 |                234/236 2 |   1602
byrantech__laptop__power                                             |                112/115 3 |                113/115 2 |   1320
emertcakir__OpenAirScope__OpenAirScope                               |                178/184 6 |                179/184 5 |   1190
greatscottgadgets__hackrf__hackrf-one                                |            318/319 1 +6e |            319/319 0 +1e |   1044
sporkus__capybully_keyboard__capybully                               |                  32/32 2 |                  32/32 1 |   1777
tzarc__keyboards__ghoul                                              |                142/148 7 |            142/148 6 +1e |   1762

Worse:

board                                                                |                   before |                    after |      s
-------------------------------------------------------------------- | ------------------------ | ------------------------ | ------
hackclub__OnBoard__koeg-board-pcb                                    |          166/175 11 +15e |          165/175 12 +17e |   1711
Goga64__ULK__ULK                                                     |                  68/70 2 |              66/70 4 +1e |   1688
ebastler__osprey__osprey_rev_a                                       |            102/103 1 +3e |            100/103 3 +2e |    822
MbFredys__PCB-Modular-Multi-Protocol-Hub__Hub                        |               139/159 32 |               137/159 35 |    910
stonedDiscord__nonSNES__SNSP-CPU-1CHIP                               |           210/212 2 +16e |           206/212 5 +20e |   1069
earth75__atat-1800__ATAT1800                                         |           242/278 49 +7e |           244/278 53 +7e |   1484
stonedDiscord__nonSNES__SNSP-CPU-01                                  |         302/363 239 +22e |         300/363 246 +16e |   1702
GlasgowEmbedded__glasgow__glasgow__revC3                             |              113/226 254 |              112/226 263 |   1318
hackclub__OnBoard__woagboard                                         |            85/91 11 +34e |            76/91 26 +26e |   1046
antmicro__jetson-nano-baseboard__jetson-nano-baseboard               |           333/340 9 +11e |           325/340 25 +1e |   1662
CRImier__MyKiCad__protoesp                                           |            82/104 66 +5e |            76/104 83 +2e |   1342
CRImier__MyKiCad__vaio_re                                            |           240/267 81 +1e |              225/267 101 |   1315
Twisted-Fields__rp2040-motor-controller__RP2040_base                 |              148/186 100 |              151/186 173 |   1015
vd-rd__sbc_allwinner_a13__module                                     |            221/223 8 +1e |           186/223 81 +1e |   1227
0xCB-dev__0xCB-1337__panel                                           |          145/188 129 +2e |              137/188 210 |   1720
Huaqiu-Electronics__ecad-viewer__video                               |              253/371 315 |              252/371 499 |   1673

## Freerouting against x1367 (2026-10-21)

**Complete: all 109 boards** (measured 2026-10-21 to 2026-10-23), and the
ten-board speed study. Our side is pcb-maker-x1367 throughout, for
comparability with the rows Freerouting was given its wall from; main has
moved on since (x1573 and later: the memory ledger, pour integrity off by
default, the silkscreen placement, router memory, trace aesthetics and the
trouble repair default; see [changelog.md](changelog.md)), so these are not
main's numbers.

Same placements on both sides: route mode on the v17 layout placements
(`build/github-layout-v17/<task>/layout/placed`: the layout's parts, the
designer's keepouts and pours, no tracks), tasks v7, the task budgets.
Ours: `run.py --route-from build/github-layout-v17` with
pcb-maker-x1367, 8 jobs x 2 threads at a load of 12 to 50
(`build/fr2-route-x1367.log`). Rows the run lost were rerun alone and
spliced (`build/fr2-route-final.log`): bumwings_v001_core and ghoul
(killed by the kernel's OOM killer, exit -9: such a row still carries a
result board from the ladder's last best attempt and is not the binary's
answer) and the 0xCB pcb-panel on x1367; the rows cut by the wall clock or
the harness timeout (katia, Batch_1, the 0xCB panel, poly_kb_molecule_4x4)
on x1371, which stretches the wall guard under load and differs from x1367
only in work accounting. katia hits the harness's 2700 s timeout again on
x1371 alone (8.8 GB peak) and the pcb-panel's KiCad refill times out at
900 s again: both stay rows without a result. No row with exit -9 or 137
is left.

Freerouting 2.2.4 headless (`benchmarks/github/freerouting_route_mode.py`,
freerouting.py's pipeline: KiCad's DSN export, the class rules raised to the
board minimums, its batch passes, KiCad's session import), `-mt 1`,
`-Xmx4g`, with its own job timeout (`--router.job_timeout`) set to the wall
our row of the board took: its passes until it stops (it stops by itself
when a pass no longer improves) or that wall, and then it writes the session
of what it has; the process is killed at that wall plus 300 s. The
designer's pours stay on the board: KiCad's DSN export makes them planes,
Freerouting reaches them with vias, and KiCad's DRC refills them, as on our
side (where the router fills them itself). Both results are judged by
run.py's own functions: KiCad's DRC (`drc-kicad-board`, zones refilled),
the pass rule (no unconnected item, no copper error beyond the designer's
count of its type) and the strict rule (only the findings the designer's
stripped board has at the same place are subtracted). Wall and CPU are each
side's whole pipeline: ours the route process with its KiCad checks,
Freerouting's the JVM plus the DSN export, the session import and the DRC.
Freerouting ran three boards at a time beside other agents' runs, at a
load of 1 to 49 (`build/fr2-freerouting-x1367.log`).

**Summary** (`benchmarks/github/freerouting_route_compare.py
build/fr2-route-final.log build/fr2-freerouting-x1367`):

| | pcb-maker x1367 | Freerouting 2.2.4 |
| --- | ---: | ---: |
| boards with a KiCad-checked result (of 109) | 107 | 75 |
| pass (designer-subtracted per type) | 51 | 3 |
| strict pass | 44 | 3 |
| better on KiCad's unconnected, then errors, then strict findings | 101 | 4 (4 ties) |
| 74 boards both have: KiCad unconnected | 2 700 | 9 619 |
| same: counting errors (pass rule) | 141 | 3 016 |
| same: strict new findings | 1 254 | 4 603 |
| same: vias | 18 799 | 7 822 |
| same: track length | 538.0 m | 366.4 m |
| same: CPU seconds | 82 286 | 96 899 |
| per board, Freerouting over ours: CPU median / wall median | | x1.31 / x1.01 |
| stopped by the wall (its job timeout) | | 70 |

On the 74 boards both finish Freerouting leaves 3.6 times our unconnected
items and 21 times our counting errors; its copper is shorter and has
fewer vias because so much stays open. The three boards it passes
(0xcb-static, OM-60-Flex, CharlieBoard Blue_Line) are three of the four
ties: ours strict-passes them too, on 0xcb-static and Blue_Line in less
wall and a quarter or less of its CPU, on OM-60-Flex in 121 s against its 55 s
(148 against 129 CPU seconds). Freerouting at `-mt 1` still uses a median
2.2 CPU seconds per wall second (0.8 to 4.4: the JVM's compiler and
collector threads); ours ran at two search threads.

Freerouting's 34 boards without a result:
- 25 it did not stop within its job timeout plus 300 s (a pass does not
  look at the timeout; on a loaded machine that includes small boards):
  laptop motherboard, OpenESC 4in1-panel, 4in1 and 4in1-mini, Sisu_SSE-9,
  eurorack-pmod, vaio_re, poly_corne_split42_left, BAGEL-MK1 and
  MK1.1-Avocado, MIDAS-MK2 and MK1.1, koeg-board, video, rosco_m68k,
  hackrf-one, PixelWave, SmartSpin2k_Panelized, CyberKeeb2040, four
  bumwings variants and stm32_chiffre_36keys, ErgoSNM.
- 5 whose session KiCad cannot import: SUMEC_MK_IV, poly_kb_molecule
  7x5_wave_left, 5x4_wave and 5x2_shifted, bumwings_v001R64_nano_sd.
- 3 it cannot read: zpn_devboard and the Hub (a NullPointerException in
  `applyBoardSpecificOptimizations`, as on 2026-10-09: no session) and
  OnBoard keyboar_ (the DSN is not read).
- The 0xCB pcb-panel, whose KiCad check times out on both sides.

Ours has no result on two: katia (the harness's 2700 s timeout) and the
pcb-panel.

**Where Freerouting is better** (fewer KiCad-unconnected items; it passes
none of the four):

- **Business-Cards Batch_1** (a panel): ours 106/204, KiCad 441 unconnected,
  no counting error but 261 clearance findings the strict rule counts (the
  designer's board has clearance findings at KiCad's cap); Freerouting 307
  unconnected with 158 counting errors (34 shorting_items, 96 copper edge
  clearance, 8 invalid_outline, 16 starved thermals, 4 hole_to_hole).
- **katia**: ours has no result (the harness's 2700 s timeout on x1367 and
  again on x1371 alone); Freerouting 90 unconnected and 126 hole_clearance,
  in 2643 s.
- **jiran-ble-lite**: ours 104/105, KiCad 20 unconnected, no error (20 GND
  pads on the far half of a split keyboard joined by a frame no track
  crosses); Freerouting 2 unconnected and 23 clearance errors in 75 s.
  Main since x1434 (net ties and apart trees) routes it to KiCad's 1, the
  designer's own.
- **SNSP-CPU-01**: ours 301/363, KiCad 161 unconnected, one starved thermal
  as the strict rule's only finding; Freerouting 151 unconnected with 194
  solder mask bridges, 12 clearance and 3 starved thermals.

**Why we lost there, and main** (2026-10-23, docs/changelog.md, "the
boards other routers beat us on"; route mode on the same placements at
the task budget, main x1647 and the branch's x1714, `build/beat1-*`):
- katia: x1367 and x1371 estimated a router at 7.3 GB, so the ladder ran
  its probes, rungs and seeds one at a time and the wall clock ran past
  the harness's 2700 s (no phase uncharged, KiCad's check not reached).
  Since the memory rounds of 2026-10-22/23 (2.5 GB a router, four side by
  side) main routes it to 155/162, KiCad 9 unconnected and no error, in
  1299 s for 1133 s of work: better than Freerouting's 90.
- Batch_1: the 261 clearance findings are the panel's own (238 pairs of
  a mounting hole's overlapping plated pads, 6 of HC1's and AE1's own
  copper, 17 of R62 and HC1 at the designer's places against the
  heater's copper on the board); the stripped designer board has 284,
  past KiCad's cap of 199, so the strict rule counted all of ours as
  new. Strict rule 4 counts a finding of one footprint's own items as
  the designer's, subtracts findings at the same place also at the cap,
  and matches a copper_edge_clearance by its copper item: ours now has
  copper_edge_clearance 2 (AE1, the placement's). The unconnected items
  (main 455 against Freerouting's 307, the designer's own board 132:
  six cards joined by tabs) stay: the router spends its budget on the
  connections between the cards.
- SNSP-CPU-01: main is better since x1367 (not re-measured here).

Per board (FR open: Freerouting's own count of unrouted connections from
its log, "(wall)" where its job timeout stopped it; "-": no result; routed
CUT: the harness's timeout):

| Board | ours routed | ours KiCad open | ours errors | ours strict | ours wall s | ours CPU s | FR open (its count) | FR KiCad open | FR errors | FR strict | FR wall s | FR CPU s |
| --- | ---: | ---: | ---: | :---: | ---: | ---: | ---: | ---: | ---: | :---: | ---: | ---: |
| 0xCB-1337__1337-v4.0 | 47/47 | 0 | 0 | pass | 100.1 | 100.3 | 1 | 16 | 13 | no | 86.7 | 156.8 |
| 0xCB-1337__panel | 149/188 | 65 | 0 | no | 2355.1 | 2921.2 | 49 | 113 | 20 | no | 954.6 | 2105.1 |
| 0xCB-1337__pcb | 85/85 | 0 | 0 | pass | 405.0 | 729.0 | 1 | 26 | 139 | no | 163.5 | 372.4 |
| 0xCB-1337__pcb-panel | 263/340 | - | - | - | 1306.9 | 673.9 | 125 (wall) | - | - | - | 2215.4 | 2862.6 |
| 0xCB-Static__0xcb-static | 74/74 | 0 | 0 | pass | 31.0 | 31.3 | 197 | 0 | 0 | pass | 40.4 | 123.1 |
| jetson-nano-baseboard__jetson-nano-baseboard | 325/340 | 19 | 0 | no | 820.7 | 1546.2 | 677 (wall) | 499 | 21 | no | 836.5 | 1891.7 |
| explorer__Explorer | 148/150 | 2 | 0 | no | 380.8 | 637.4 | 237 (wall) | 238 | 1 | no | 412.1 | 749.8 |
| eurorack-pmod__eurorack-pmod-pcb | 128/133 | 4 | 0 | no | 465.2 | 1163.3 | 33 (wall) | - | - | - | 768.6 | 1190.6 |
| Qfwfq__qfwfq | 64/64 | 0 | 0 | no | 290.3 | 296.8 | 1 | 15 | 0 | no | 261.1 | 537.8 |
| bit-preserve__coco2 | 173/173 | 0 | 0 | pass | 657.1 | 495.9 | 43 (wall) | 83 | 10 | no | 666.0 | 1520.0 |
| Sumec-MiniSumo__SUMEC_MK_IV | 66/69 | 5 | 0 | no | 974.1 | 1902.8 | 44 | - | - | - | 120.6 | 292.6 |
| d20_pcb__d20_pcb | 18/19 | 1 | 0 | no | 294.2 | 502.2 | 62 | 2 | 152 | no | 58.4 | 98.1 |
| spell_tome__spell_tome_bottom | 18/18 | 0 | 0 | pass | 8.5 | 8.4 | 43 | 1 | 4 | no | 21.6 | 42.0 |
| Sisu_SSE-9__Sisu_SSE-9 | 179/192 | 15 | 0 | no | 792.7 | 1608.2 | 79 (wall) | - | - | - | 1094.7 | 2149.8 |
| laptop__keyboard | 132/136 | 4 | 0 | no | 674.7 | 1197.5 | 27 (wall) | 42 | 12 | no | 684.2 | 1684.0 |
| laptop__motherboard | 232/236 | 4 | 0 | no | 1279.4 | 2055.4 | 6 (wall) | - | - | - | 1580.9 | 3601.0 |
| laptop__power | 110/115 | 5 | 0 | no | 328.2 | 885.3 | 6 (wall) | 50 | 155 | no | 347.5 | 694.8 |
| Business-Cards__Batch_1 | 106/204 | 441 | 0 | no | 810.4 | 1664.9 | 234 (wall) | 307 | 158 | no | 826.2 | 1889.9 |
| Business-Cards__USB_Cable_Tester__PCB | 54/54 | 0 | 0 | pass | 121.6 | 122.1 | 4 (wall) | 23 | 132 | no | 150.0 | 282.0 |
| Business-Cards__USB_Keypad | 37/39 | 2 | 0 | no | 381.4 | 670.3 | 2 | 27 | 23 | no | 81.2 | 181.3 |
| Business-Cards__WLED_Matrix | 99/104 | 11 | 2 | no | 583.0 | 1046.4 | 9 | 75 | 34 | no | 318.3 | 711.5 |
| MyKiCad__framework_mobo_lefthalf | 71/71 | 0 | 0 | pass | 266.2 | 704.4 | 55 (wall) | 63 | 4 | no | 283.7 | 534.6 |
| MyKiCad__protoesp | 67/104 | 58 | 1 | no | 1180.3 | 1805.1 | 90 | 115 | 4 | no | 767.7 | 1684.2 |
| MyKiCad__vaio_re | 262/267 | 4 | 6 | no | 999.7 | 1559.1 | 44 (wall) | - | - | - | 1301.5 | 2613.6 |
| MyKiCad__zpn_devboard | 166/167 | 1 | 0 | no | 823.6 | 1479.4 | - | - | - | - | 4.3 | 4.5 |
| mackerel-68k__mackerel-08-v1 | 96/96 | 0 | 0 | pass | 467.0 | 587.3 | 6 (wall) | 56 | 15 | no | 483.1 | 872.7 |
| mackerel-68k__mackerel-10-v1 | 155/155 | 0 | 0 | pass | 662.2 | 921.2 | 2 (wall) | 121 | 4 | no | 670.8 | 1569.9 |
| mackerel-68k__mackerel-30-proto | 217/217 | 0 | 2 | no | 616.1 | 870.4 | 68 (wall) | 198 | 586 | no | 624.4 | 1501.7 |
| SmartSpin2k__SmartSpin2k_Panelized | 550/585 | 45 | 0 | no | 730.3 | 723.6 | 948 (wall) | - | - | - | 1031.9 | 1924.3 |
| eternal-keypad__eternal-keypad | 65/65 | 0 | 0 | no | 95.6 | 82.9 | 14 (wall) | 19 | 0 | no | 114.2 | 220.2 |
| urchin__main | 68/68 | 0 | 0 | pass | 21.0 | 24.2 | 243 (wall) | 107 | 3 | no | 36.6 | 83.3 |
| atat-1800__ATAT1800 | 253/278 | 30 | 0 | no | 1880.8 | 2218.9 | 38 (wall) | 91 | 27 | no | 1912.4 | 4138.4 |
| osprey__osprey_rev_a | 99/103 | 3 | 1 | no | 2164.8 | 2280.9 | 23 (wall) | 50 | 26 | no | 2179.1 | 4730.6 |
| OpenAirScope__OpenAirScope | 179/184 | 5 | 0 | no | 550.9 | 1221.8 | 6 | 54 | 0 | no | 482.1 | 1032.3 |
| glasgow__glasgow__revC3 | 124/226 | 180 | 0 | no | 883.3 | 1247.7 | 385 (wall) | 390 | 2 | no | 890.0 | 2017.2 |
| ULK__ULK | 68/70 | 2 | 0 | no | 1668.0 | 2022.1 | 3 | 13 | 41 | no | 43.9 | 99.8 |
| ULK__ULK_sl_PG1316s | 68/70 | 3 | 0 | no | 1783.6 | 2382.3 | 5 | 7 | 9 | no | 90.5 | 186.4 |
| hackrf__hackrf-one | 319/319 | 0 | 0 | pass | 284.2 | 423.7 | 726 (wall) | - | - | - | 586.8 | 823.6 |
| OnBoard__E-Fidget-Lite | 20/20 | 0 | 0 | pass | 27.0 | 26.5 | 63 | 2 | 0 | no | 35.9 | 92.1 |
| OnBoard__keyboar_ | 102/102 | 0 | 0 | no | 540.0 | 359.2 | - | - | - | - | 301.7 | 294.0 |
| OnBoard__koeg-board-pcb | 150/175 | 33 | 23 | no | 1630.5 | 2168.0 | 73 (wall) | - | - | - | 1931.7 | 4068.4 |
| OnBoard__krishveercard | 43/43 | 0 | 0 | no | 617.0 | 1658.9 | 86 | 86 | 0 | no | 78.7 | 200.0 |
| OnBoard__MotionCubeViewAllForces | 20/20 | 0 | 0 | pass | 83.1 | 130.8 | 9 | 12 | 8 | no | 86.9 | 184.1 |
| OnBoard__PixelWave | 343/343 | 0 | 0 | pass | 359.3 | 350.7 | 688 (wall) | - | - | - | 663.7 | 962.9 |
| OnBoard__woagboard | 84/91 | 9 | 0 | no | 1410.6 | 2711.4 | 7 | 49 | 0 | no | 793.9 | 2011.4 |
| slab-pcb__interchange-pcb-right | 83/83 | 0 | 0 | no | 983.1 | 795.9 | 16 | 112 | 99 | no | 882.3 | 1957.4 |
| slab-pcb__slab-pcb-left | 75/76 | 1 | 0 | no | 1839.7 | 2228.6 | 9 | 107 | 0 | no | 1136.6 | 2126.3 |
| ecad-viewer__video | 267/371 | 254 | 0 | no | 1368.9 | 1450.3 | 311 (wall) | - | - | - | 1670.8 | 3431.6 |
| silkscreen-pcb__silkscreen_pcb | 113/113 | 0 | 0 | pass | 149.1 | 149.9 | 39 (wall) | 64 | 40 | no | 168.0 | 385.3 |
| katia__katia | - CUT | - | - | - | 2701.0 | 2805.0 | 27 | 90 | 126 | no | 2643.2 | 5879.3 |
| ISS-PCB__BAGEL-MK1 | 138/138 | 0 | 0 | pass | 174.6 | 231.7 | 334 (wall) | - | - | - | 477.5 | 686.7 |
| ISS-PCB__BAGEL-MK1.1-Avocado | 142/144 | 2 | 0 | no | 558.9 | 1295.8 | 242 (wall) | - | - | - | 861.6 | 1592.2 |
| ISS-PCB__MIDAS-MK1.1__MIDAS-MK1.1-revA | 151/156 | 5 | 0 | no | 598.7 | 1429.7 | 15 (wall) | - | - | - | 900.6 | 1727.1 |
| ISS-PCB__MIDAS-MK1__MIDAS-MK1-revA | 151/159 | 8 | 9 | no | 670.0 | 1281.3 | 36 (wall) | 63 | 12 | no | 679.2 | 1611.8 |
| ISS-PCB__MIDAS-MK2 | 156/156 | 0 | 0 | pass | 242.6 | 299.6 | 65 (wall) | - | - | - | 547.8 | 769.5 |
| ISS-PCB__MIDAS-MK2.1__MIDAS-MK2.1-revA | 154/154 | 0 | 0 | pass | 602.4 | 1194.8 | 8 | 46 | 1 | no | 541.6 | 1235.8 |
| ligra__ligra_back | 33/33 | 0 | 0 | pass | 39.1 | 29.5 | 77 | 5 | 0 | no | 26.3 | 68.0 |
| jiran-ble-lite__jiran-ble-lite | 104/105 | 20 | 0 | no | 1318.8 | 1357.0 | 2 | 2 | 23 | no | 74.9 | 196.2 |
| little-red-rover__little_red_rover | 64/64 | 0 | 0 | pass | 317.8 | 553.9 | 11 | 30 | 6 | no | 191.0 | 400.3 |
| M8SBC-486__homebrew_486 | 190/190 | 0 | 0 | pass | 536.4 | 975.4 | 460 (wall) | 488 | 0 | no | 586.1 | 606.0 |
| PCB-Modular-Multi-Protocol-Hub__Hub | 144/159 | 22 | 0 | no | 971.2 | 1945.3 | - | - | - | - | 4.3 | 4.6 |
| Neotron-Pico__neotron-pico | 176/190 | 24 | 0 | no | 2426.6 | 2872.0 | 97 | 103 | 2 | no | 800.8 | 1700.6 |
| Quanta75__Quanta75_BareRP2040_JLCPCBAoptimized | 156/172 | 18 | 1 | no | 1772.8 | 2006.5 | 59 | 76 | 12 | no | 1762.5 | 3409.9 |
| Quanta75__Quanta75_RP2040Stamp_JLCPCBAoptimized | 162/162 | 0 | 0 | pass | 694.6 | 529.8 | 1 | 30 | 9 | no | 172.7 | 402.0 |
| DSP-ADAU1452__DSP-ADAU1452 | 156/190 | 61 | 4 | no | 917.2 | 1578.4 | 152 (wall) | 212 | 61 | no | 924.3 | 2164.1 |
| OpenMuscle-FlexGrid__OM-60-Flex | 21/21 | 0 | 0 | pass | 120.6 | 147.8 | 141 | 0 | 0 | pass | 54.9 | 129.1 |
| OpenMuscle-FlexGrid__OM-FlexGrid-Flex__OM-FlexGrid-Flex | 19/19 | 0 | 0 | pass | 281.3 | 726.1 | 3 | 6 | 0 | no | 171.3 | 373.5 |
| OpenESC-20x20__4in1-mini | 126/152 | 43 | 0 | no | 508.5 | 1197.4 | 347 (wall) | - | - | - | 811.3 | 676.3 |
| OpenESC-30x30__4in1 | 91/152 | 111 | 0 | no | 407.7 | 701.6 | 412 (wall) | - | - | - | 710.1 | 1001.7 |
| OpenESC-30x30__4in1-panel | 47/184 | 485 | 220 | no | 975.2 | 2051.6 | 881 (wall) | - | - | - | 1276.6 | 2403.1 |
| OpenFC-Lite__OpenFC | 82/82 | 0 | 0 | pass | 279.7 | 840.6 | 7 (wall) | 13 | 84 | no | 295.6 | 578.5 |
| OpenRX__OpenRX-panel-rev2 | 42/127 | 229 | 0 | no | 519.5 | 905.6 | 526 (wall) | 499 | 125 | no | 555.7 | 552.9 |
| link__link | 245/251 | 6 | 0 | no | 1770.2 | 2081.4 | 294 (wall) | 420 | 191 | no | 1786.8 | 3460.4 |
| rosco_m68k__rosco_m68k__kicad | 148/148 | 0 | 0 | pass | 597.0 | 728.2 | 21 (wall) | - | - | - | 898.6 | 1698.4 |
| OSHW-reCamera-Series__reCamera_S101_v1.1 | 55/56 | 4 | 0 | no | 116.1 | 310.0 | 56 (wall) | 88 | 0 | no | 136.5 | 216.8 |
| ergo-snm-keyboard__ErgoSNM_keyboard | 58/58 | 0 | 0 | pass | 270.4 | 245.1 | 48 (wall) | - | - | - | 573.2 | 800.0 |
| Telemetry__TelemetryOnboard | 108/108 | 0 | 0 | no | 364.8 | 960.3 | 14 (wall) | 28 | 10 | no | 385.4 | 740.5 |
| capybully_keyboard__capybully | 32/32 | 0 | 0 | pass | 730.2 | 811.1 | 125 (wall) | 57 | 0 | no | 755.4 | 1776.4 |
| le_chiffre_keyboard_stm32__stm32_chiffre_36keys | 76/76 | 0 | 0 | pass | 417.0 | 458.6 | 9 (wall) | - | - | - | 719.5 | 1061.6 |
| le_chiffre_keyboard_stm32__stm32_hotswap_chiffre | 75/75 | 0 | 0 | pass | 454.0 | 486.4 | 6 | 25 | 0 | no | 335.4 | 624.3 |
| MegaDrive__MegaDrive | 243/298 | 168 | 0 | no | 2102.8 | 1533.2 | 193 (wall) | 251 | 35 | no | 2123.5 | 4479.8 |
| nonSNES__SNSP-CPU-01 | 301/363 | 161 | 0 | no | 1632.3 | 1786.1 | 125 (wall) | 151 | 209 | no | 1641.1 | 3695.1 |
| nonSNES__SNSP-CPU-1CHIP | 212/212 | 0 | 0 | pass | 599.9 | 460.7 | 2 | 70 | 22 | no | 497.9 | 1155.5 |
| MokyaLora__MokyaLora | 261/262 | 0 | 0 | pass | 840.6 | 1492.5 | 178 (wall) | 221 | 0 | no | 849.7 | 1998.3 |
| PolyKybd__poly_corne_split42_left | 240/247 | 7 | 0 | no | 1387.5 | 2224.0 | 212 (wall) | - | - | - | 1689.6 | 3479.0 |
| PolyKybd__poly_corne_split42_right | 404/420 | 22 | 0 | no | 1878.8 | 2295.6 | 725 (wall) | 499 | 2 | no | 1899.0 | 4016.8 |
| PolyKybd__poly_kb_molecule_4x4 | 1/28 | 499 | 16 | no | 1300.0 | 2406.9 | 554 (wall) | 499 | 60 | no | 1308.7 | 2960.5 |
| PolyKybd__poly_kb_molecule_4x5 | 0/28 | 499 | 101 | no | 984.1 | 2522.8 | 732 (wall) | 499 | 156 | no | 993.0 | 2248.6 |
| PolyKybd__poly_kb_molecule_5x2_shifted | 1/28 | 409 | 32 | no | 1464.8 | 1857.8 | 301 (wall) | - | - | - | 1470.3 | 3316.4 |
| PolyKybd__poly_kb_molecule_5x4_wave | 2/28 | 499 | 0 | no | 643.4 | 933.6 | 486 (wall) | - | - | - | 648.4 | 1452.8 |
| PolyKybd__poly_kb_molecule_7x5_wave_left | 3/28 | 499 | 0 | no | 926.7 | 1299.8 | 784 (wall) | - | - | - | 932.5 | 2099.1 |
| PolyKybd__poly_kybd_split72_left | 393/420 | 37 | 0 | no | 1677.1 | 2433.3 | 671 (wall) | 499 | 1 | no | 1691.4 | 3840.4 |
| PolyKybd__poly_kybd_split72_right | 405/420 | 24 | 0 | no | 1927.8 | 2388.5 | 747 (wall) | 499 | 0 | no | 1960.2 | 4083.8 |
| CharlieBoard__Blue_Line | 26/26 | 0 | 0 | pass | 11.0 | 11.1 | 72 | 0 | 0 | pass | 20.3 | 52.9 |
| computie__k30-SBC | 173/173 | 0 | 0 | no | 645.6 | 814.2 | 1 | 79 | 8 | no | 532.7 | 1238.2 |
| bumwings-kbd__bumwings_v001 | 72/72 | 0 | 0 | pass | 155.6 | 155.6 | 61 (wall) | - | - | - | 458.5 | 650.3 |
| bumwings-kbd__bumwings_v001_core | 98/98 | 0 | 0 | pass | 592.2 | 706.7 | 36 (wall) | 31 | 0 | no | 602.3 | 1428.3 |
| bumwings-kbd__bumwings_v001_xiao | 81/81 | 0 | 0 | pass | 556.3 | 437.3 | 62 (wall) | - | - | - | 858.3 | 1466.1 |
| bumwings-kbd__bumwings_v001_xiao_s | 78/78 | 0 | 0 | pass | 130.6 | 138.5 | 103 (wall) | 106 | 1 | no | 155.8 | 349.4 |
| bumwings-kbd__bumwings_v001R55_rp2040zero_sd | 70/70 | 0 | 0 | pass | 84.6 | 85.4 | 52 (wall) | - | - | - | 387.7 | 489.1 |
| bumwings-kbd__bumwings_v001R55_xiao_sd | 75/75 | 0 | 0 | pass | 163.6 | 163.8 | 74 (wall) | - | - | - | 468.5 | 668.8 |
| bumwings-kbd__bumwings_v001R64_nano_sd | 85/85 | 0 | 0 | pass | 72.5 | 74.7 | 100 (wall) | - | - | - | 82.1 | 176.2 |
| bumwings-kbd__bumwings_v001R64_rp2040zero_sd | 81/81 | 0 | 0 | pass | 100.1 | 89.9 | 79 (wall) | 79 | 1 | no | 118.0 | 221.1 |
| bumwings-kbd__bumwings_v001R64_xiao_sd | 89/89 | 0 | 0 | pass | 127.1 | 127.8 | 130 (wall) | 132 | 2 | no | 147.5 | 312.9 |
| rp2040-motor-controller__RP2040_base | 147/186 | 50 | 0 | no | 617.4 | 1150.0 | 149 (wall) | 162 | 0 | no | 624.7 | 1407.6 |
| keyboards__ghoul | 142/148 | 6 | 1 | no | 2109.3 | 2740.9 | 7 | 121 | 100 | no | 850.0 | 1906.7 |
| sbc_allwinner_a13__module | 208/223 | 23 | 3 | no | 1023.8 | 1808.4 | 195 (wall) | 256 | 112 | no | 1033.8 | 2328.8 |
| Castor_and_Pollux__mainboard | 126/126 | 0 | 0 | pass | 567.9 | 804.3 | 1 | 41 | 29 | no | 98.7 | 258.8 |
| CyberKeeb2040__MainBoard | 118/118 | 0 | 0 | pass | 350.7 | 378.5 | 99 (wall) | - | - | - | 655.5 | 1013.5 |

**Speed: ten boards, 1 and 16 threads.** 0xcb-static, ligra_back,
1337-v4.0, USB_Cable_Tester, silkscreen_pcb, MIDAS-MK2, little_red_rover,
Castor_and_Pollux mainboard, mackerel-08-v1 and coco2; ours at 1 and 16
threads (`PCB_MAKER_NO_CACHE=1`, one board at a time under the tier
lock, `build/fr2-speed/x1367-t<n>-<board>`), Freerouting at `-mt 1` and
`-mt 16` with a 1500 s job timeout, one board at a time without the lock
(`build/fr2-speed/cmp1-freerouting-mt<n>`), admitted by the memory ledger
(`--min-free-gb 10`; its JVM reserved 4.5 GB there through a wrapper
around `freeroute`). The machine was shared with other agents' runs
throughout; Freerouting's walls below leave out the time it waited for
the ledger. Each cell: wall / CPU seconds, then
KiCad's unconnected items / counting errors ("pass": the strict rule).

| board | ours t1: wall / CPU s | open/errors | ours t16: wall / CPU s | open/errors | FR -mt 1: wall / CPU s | open/errors | FR -mt 16: wall / CPU s | open/errors |
|---|---:|---|---:|---|---:|---|---:|---|
| 0xcb-static | 37 / 34 | 0/0 pass | 40 / 36 | 0/0 pass | 23 / 78 | 0/0 pass | 17 / 60 | 0/0 pass |
| ligra_back | 42 / 36 | 0/0 pass | 34 / 35 | 0/0 pass | 12 / 40 | 5/0 | 14 / 42 | 5/0 |
| 1337-v4.0 | 112 / 114 | 0/0 pass | 123 / 126 | 0/0 pass | 25 / 85 | 16/13 | 27 / 91 | 16/13 |
| PCB | 113 / 116 | 0/0 pass | 136 / 139 | 0/0 pass | 86 / 228 | 23/132 | 118 / 297 | 23/132 |
| silkscreen_pcb | 174 / 173 | 0/0 pass | 188 / 180 | 0/0 pass | 280 / 650 | 59/17 | 374 / 760 | 59/17 |
| MIDAS-MK2 | 409 / 487 | 0/0 pass | 392 / 485 | 0/0 pass | 138 / 328 | 32/1 | 207 / 436 | 32/1 |
| little_red_rover | 226 / 533 | 0/0 pass | 229 / 581 | 0/0 pass | 98 / 250 | 30/6 | 91 / 226 | 30/6 |
| mainboard | 776 / 1131 | 0/0 pass | 664 / 1023 | 0/0 pass | 116 / 282 | 41/29 | 95 / 220 | 41/29 |
| mackerel-08-v1 | 374 / 473 | 0/0 pass | 455 / 563 | 0/0 pass | 257 / 604 | 53/16 | 177 / 426 | 53/16 |
| coco2 | 360 / 352 | 0/0 pass | 278 / 284 | 0/0 pass | 1802 / 3616 | timeout | 1802 / 3772 | timeout |

Ours strict-passes all ten at both thread counts; Freerouting passes
0xcb-static only, leaves 5 to 59 KiCad-unconnected items on eight, and
on coco2 does not stop within the 1500 s job timeout plus 300 s at either
thread count (no result). It stops by itself early: its wall is a median
0.53 times ours at 1 thread and 0.43 times at 16, while its CPU is a
median 1.20 and 1.05 times ours (the JVM's own threads: 2.0 to 3.4 CPU
seconds per wall second at `-mt 1`). Neither router gets faster with 16
threads here: Freerouting's `-mt 16` over `-mt 1` is a median x1.05 CPU
and x1.03 wall with identical results on every board, ours at 16 threads
over 1 is x1.05 CPU and x1.05 wall with the same verdicts (the route
mode's budget is work, not wall). Our runs saw a load of 15 to 39, Freerouting's 9 to 36.

Logs: `build/fr2-route-final.log` (ours), `build/fr2-freerouting-x1367.log`
and `.err`, the table and summary in `build/fr2-compare.txt`
(`freerouting_route_compare.py --markdown`), the speed runs in
`build/fr2-speed/`.

## Freerouting against our cold route, x738 (2026-10-15)

Same setup as the 2026-10-09 round: the designer's placement with every
track, via and copper zone removed, 1500 s wall each, Freerouting 2.2.4
headless through `benchmarks/github/freerouting.py` (now with `-Xmx4g`; an
A/B on hackrf showed the limit costs no CPU, and no board hit it), both
results judged by KiCad's DRC with `--severity-all` against the same
baseline. Our side is pcb-maker-x738 on all 109 boards. Boards ran side by
side (4 jobs, at most 2 Freerouting JVMs and 2 routes at a time), with the
load recorded per row. Freerouting timeouts at a load above 45 were rerun,
and eight boards Freerouting had finished last round but timed out on here
were rerun once Freerouting-only (four then finished).

| | x738 round | x136/x155 round (2026-10-09) |
|---|---|---|
| Freerouting finished / timed out / failed | 67 / 36 / 6 | 70 / 34 / 5 |
| fewer unconnected: pcb-maker / Freerouting | 47 / 6 | 49 / 6 |
| fewer DRC errors: pcb-maker / Freerouting | 33 / 1 | 33 / 0 |
| verdict (unconnected, then errors): ours / FR / tie | 53 / 6 / 8 | 56 / 6 / 8 |
| both finished: unconnected FR / ours | 873 / 184 | 913 / 206 |
| both finished: DRC errors FR / ours | 1857 / 8 | 2041 / 15 |
| both finished: clean boards FR / ours | 8 / 51 | 8 / 50 |
| both finished: vias FR / ours | 5708 / 6785 | 6575 / 7205 |
| both finished: length FR / ours | 409.5 / 409.2 m | 461.4 / 460.4 m |
| both finished: CPU FR / ours | 62688 s / 40885 s | not recorded |
| clean boards over all 109, FR / ours | 8 / 62 | 8 / 60 |
| our unconnected over all 109 | 4785 | 4577 |

Freerouting finished 67, timed out 36, failed 6 (keyboar_, formerly a
"timeout", is a DSN its parser cannot read: a part number with a backtick
loops it forever). On the 67 boards both finish, pcb-maker has fewer
unconnected on 47 and Freerouting on 6; length is equal and we use 19 %
more vias, at 0.65 times Freerouting's CPU seconds (ours including its own
KiCad checks; Freerouting burns about 2 CPU seconds per wall second even
with `-mt 1`). No board flipped sides; the verdict changes are boards where
Freerouting has a result in one round and not in the other (0xCB pcb-panel,
video, osprey, bumwings core: timeouts now; eurorack-pmod: finished now).
Freerouting's wall time on one board varies by more than two times between
runs (hackrf's first pass took 386 s in one run and more than 900 s in two
others), so its timeouts near the limit are noisy.

Against the x136/x155 round our cold route is better on 16 boards, the same
on 82 and worse on 11 (4577 to 4785 unconnected): SNSP-CPU-01 55 to 178
(all GND; the router reports 362 of 363 connections routed while KiCad
finds 178 unconnected terminals) and MegaDrive 95 to 137 repeat exactly
when run alone, and katia (84 to 183) is cut by the 1800 s wall in both of
its runs at a 14.9 GB peak. Better rows include vaio 26 to 8, the A13
module 22 to 9, DSP 7 to 0 and mackerel-30's 13 errors to 0.

The runner changes (CPU, load and tier per row, the jobs file,
`--resume`/`--redo`, `--java-heap`, stall detection, `--severity-all`)
landed in ef980b2; the DRC-cache round made both DRCs go through
`pcb-maker drc-kicad-board` when the binary has it (047f6ff; x738 has not,
so this round used kicad-cli on both sides). Logs:
`build/github-freerouting-x738.log`, reruns `-fr2.log` and `-katia.log`,
table `-compare.txt`. Reproduce the summary with
`freerouting_compare.py build/github-freerouting-x738.log build/github-freerouting-x738.log build/github-freerouting-x738-fr2.log --ours-splice build/github-freerouting-x738-katia.log --previous-ours build/github-ours-cold-all.log --previous-freerouting build/github-freerouting.log build/github-freerouting-2.log build/github-freerouting-3.log`.

## Layout sweep v16 (2026-10-15)

`build/github-layout-v16.log`, binary x862 (main 068de8b: determinism,
fill rounds two and three, CLI exit codes, DRC cache, placer fixes, ladder
reservation, clean-up work accounting, inner spokes, hatched zones in the
tracks rung; it predates the first-route budget, the hole-clearance fix,
the router overhead round and the second narrow attempt), tasks v6, 5 jobs
beside other agents (load 16 to 35 for most rows), 20876 s wall.

| | v15 (x723) | v16 (x862) |
|---|---|---|
| pass | 45 of 109 | 44 of 109 |
| unconnected (all rows) | 3868 | 3527 |
| new DRC errors | 289 | 82 |
| vias | 25759 | 25335 |
| CPU s (unpaired) | 189814 | 145292 |
| rows cut by the wall clock | not reported | 17 |
| rows without a result | 9 | 10 |

Better 13, same 58, worse 13, 13 cut rows left out of that count. Large
moves: video 499 to 300 open, SNSP-CPU-01 246 to 168, RP2040_base 173 to
96, 0xCB panel 210 to 86 (cut), woagboard 26 open and 26 errors to 8 and
1 (cut), SUMEC 28 to 18, Hub 35 to 30; katia now finishes (104 open);
molecule 5x4_wave places legally. Worse: ULK and poly_kybd_split72_left
now time out, the 0xCB pcb-panel ends on kicad-cli's 900 s zone refill,
SmartSpin2k_Panelized and OpenESC 4in1-panel stop with parts the placer
refuses, Quanta75 Bare 123 to 132 open (errors 73 to 6), split42_right 46
to 61 (cut). The 17 cut rows at modest load say some phase's wall runs
well ahead of its counted work; triage in
[changelog.md](changelog.md#triage-of-v16-2026-10-15-x931--x932). Compare with
`python3 benchmarks/agent-tasks/compare.py build/github-layout-v15.log build/github-layout-v16.log`.

## Layout sweep v17 (2026-10-16)

`build/github-layout-v17.log`, binary x1170 (main 54eff39, the one-time
rustfmt commit; code = everything up to the KiCad check charge: first-route
budget, hole clearance, router overhead and search core and stamp rounds,
pour-work recalibration, second narrow attempt, zone seams, connection
width, min connection, mask bridges and mask openings, placement room,
panel fixes, orientation search, order independence, calibration 3.15e6,
check work), tasks v7, tier default 8 jobs x 2 threads, 65959 s wall.

| | v16 (x862) | v17 (x1170) |
|---|---|---|
| pass | 44 of 109 | 48 of 109 |
| unconnected (all rows) | 3527 | 2708 |
| new DRC errors | 82 | 38 |
| vias | 25335 | 30601 |
| CPU s (unpaired) | 145292 | 101279 |
| rows cut by the wall clock | 17 | 2 |
| rows without a result | 10 | 7 |

Better 25, same 50, worse 8 (14 cut rows and 1 on other inputs left out).
ULK, SmartSpin2k_Panelized and poly_kybd_split72_left now finish (68/70,
553/585 and 394/420); MegaDrive finishes but is cut at 163 open. The 0xCB
pcb-panel still ends on kicad-cli's 900 s zone refill, the four molecule
panels still leave a few back connectors unplaced (designer stacking at
0.03 mm), OpenESC 4in1-panel is crowded, and MIDAS-MK1 is a new refusal
(6 parts without a legal place: triage assigned). Vias rose 21 percent
with the 1.57x larger work budget. The two cuts ran at loads 1 and 66.
Compare with
`python3 benchmarks/agent-tasks/compare.py build/github-layout-v16.log build/github-layout-v17.log`.

## Layout sweep v18 (2026-10-21)

**Matched-budget designer tier (2026-10-22):** with the designer's placement routed at the layout's median final-ladder share (budget scale 0.695, x1404, strict rule 3), 59 of 109 pass (51 strict) and the gap over the 99 target boards is placement 49 %, routing 51 % (layout open 1973, designer-route open 1016), against 71/29 at the full 1500 s. The placement share above is therefore an upper bound; routing at the layout's budget is as large a lever as placement. Log: `build/designer-x1404-matched.log`.

`build/github-layout-v18.log`, binary x1315 (main 35456a1: project rules
at scale, runtime calibration, the ladder's work bound and fault
isolation; 6f10989 differs only in doc comments), tasks v7 (the committed
`benchmarks/agent-tasks/tasks/v7`, the same files as v17's
`build/github-tasks-v7`: tasks.json sha256 c8f95caf5089 and every
constraint file byte-identical; compare.py marks no board INPUT), with
`--quality`, at the pinned work rates (every row: `work_rate` 3.15e6,
`placer_work_rate` 150e6, `work_rate_source` PCB_WORK_RATE; the measured
router rate had a median of 2.3e6, 0.99e6 to 3.3e6, the placer's 6.5e7,
work over wall 0.77). The sweep ran in three parts, all on x1315, beside
other agents' runs (load median 20, peaks of 77):

- `build/github-layout-v18a.log`: 6 jobs x 2 threads, 20 GB of memory
  kept free, about 4.9 hours, 41 boards; memory admission held it to two
  or three boards at a time (hackrf waited 3120 s), so it was stopped (its
  boards in flight lost);
- `build/github-layout-v18b.log`: the other 68 boards at 10 x 2 with 6 GB
  kept free (`--splice-from` the first part), 8834 s wall; four rows cut
  by the wall guard (10 layouts and the designer tier side by side);
- `build/github-layout-v18c.log`: the four cut boards again at 2 x 2
  (`--splice-from` the second part), 5211 s wall. 0xCB panel and Quanta75
  Bare finished uncut with the same rows as when cut; katia and Batch_1
  were cut again (below). This log is `build/github-layout-v18.log`; the
  output directories are `build/github-layout-v18`, `-v18b` and `-v18c`.

| | v17 (x1170) | v18 (x1315) |
|---|---|---|
| pass, per-type rule | 48 of 109 | 46 of 109 |
| pass, strict rule | not recorded | **42 of 109** |
| unconnected (all rows) | 2708 | 2712 |
| new DRC errors (per-type rule) | 38 | 50 |
| vias | 30601 | 31044 |
| length | 779 m | 788 m |
| work s | 80276 (102 rows) | 86094 (103 rows) |
| CPU s (unpaired; v18 oversubscribed) | 101279 | 152463 |
| rows cut by the wall clock | 2 | 2 |
| rows without a result | 7 | 6 |
| vias against the designer's (median) | x0.70 | x0.71 |
| length against the designer's (median) | x1.03 | x1.03 |
| open over connections (median, mean) | 0.011, 0.287 | 0.009, 0.284 |
| ground pour cut, ours less the designer's (median) | +0 mm (95 boards, 1 lost the pour) | +0 mm (96 boards, 1 lost the pour) |

(v17's pour cut from `compare.py --score build/bin/pcb-maker-x1315`.)
Better 5, same 87, worse 7 (3 rows cut in either run left out). Better:
Glasgow 165 to 160 open, DSP-ADAU1452 68 to 64, split72_left 39 to 36,
video 201 to 199, BAGEL-MK1.1-Avocado 2 to 1. Worse: split42_right 19 to
26, OpenRX panel 219 to 226, the A13 module 8 to 12, RP2040_base 44 to
47, Sisu 13 to 15, split72_right 28 to 30, laptop power 3 to 4.
mackerel-30-proto and rosco lose their pass to starved thermals (6 and 2
under the strict rule); MIDAS-MK1 places now (153/159, 6 open; a refusal
in v17). Under the strict rule four plain passes fail: Qfwfq
(hole_clearance) and krishveercard (clearance), each a type the
designer's stripped board has at KiCad's cap of 199, which the strict rule
cannot place and counts whole, and coco2 and k30-SBC, one starved thermal
each.

Boards that fail the strict rule on findings of one type alone (nothing
unconnected): starved_thermal on four (mackerel-30-proto 6, rosco 2,
coco2 1, k30-SBC 1), and the two capped types above (Qfwfq, krishveercard,
which the route on the designer's placement fails the same way).

Cut rows, at 2 x 2 as at 10 x 2 and with the same rows both times: katia
(157/162, 75 open; cut at 3040 s of wall with 1155 of its 1500 s of work
done, work over wall 0.38) and Batch_1 (115/204, 415 open, a panel of
business cards; cut at 3296 s with 1280 s of work done, 0.39).

### Placement or routing: the designer tier (x1315)

`build/designer-x1315b.log` (`--tier designer` through the lock, 8 x 2,
30179 s wall: `build/designer-x1315.log`; its two timeouts again at 2 x 2
with `--splice-from`): every board routed by the same binary on the
designer's own placement, 1500 s of work, the layout's pass rules, the
same pinned rates (106 rows; the three without a result carry none).
**50 of 109 pass the strict rule** (58 the per-type rule; 49 under strict
rule 3 of 2026-10-22, re-judged without rerouting: USB_Keypad's one
connection_width on its GND pour had been subtracted against the
designer's 45 on that pour, all at (0, 0); the layout sweep stays at 42,
its strict errors 1536 -> 1543); vias x0.47 of
the designer's, length x0.99, the ground pours cut 38 mm less than the
designer's (medians). katia and Batch_1 hit the harness's 3300 s in both
runs: katia's ladder runs one router at a time for memory (7.3 GB each)
and had done its probes, four attempts and a seed round (best 279 open
terminals by the router's count); Batch_1 was in its finer pitch's seed
round at 1241 open terminals. The 0xCB pcb-panel has no result on either
side (kicad-cli times out after 900 s; on the designer's placement the
router itself completes 340/340). The count is not the x738 cold route's 62 clean
(2026-10-15): that round removed every copper zone and judged against the
stripped source's own DRC; the designer tier keeps the large pours, as
the layout does, and applies the layout's two pass rules.

`compare.py --gap build/github-layout-v18.log build/designer-x1315b.log`:
over the 101 boards with both results, layout open 2222, designer-route
open 1153, placement gap +1069, routing gap 1153 (48 % and 52 %). But
723 of those 1153 are on boards that are not targets (docs/handover.md,
"Not targets": the molecule 5x4_wave panel 499, the OpenRX panel 213,
SUMEC 11, krishveercard 0; the other molecule panels and the OpenESC 4in1
panel have no layout result). Over the 97 target boards with both
results: **layout open 1483, designer-route open 430: placement gap
+1053 (71 %), routing gap 430 (29 %)**; the placement gap is +1064 on the boards where ours is
worse and -11 where it is better. 22 % of the layout's open items are on
supply nets, 34 % of the designer route's.

By the strict pass on both placements (100 target boards):

- **Placement** (the designer's placement passes, ours does not), 13
  (12 under strict rule 3: USB_Keypad fails on both placements):
  video (ours 199 open), OpenESC-30x30 4in1 (111), 0xCB panel (65,
  starved_thermal 1), OpenAirScope (5), laptop power (4), ULK_sl_PG1316s
  (3), ULK (2), USB_Keypad (2), Explorer (2), zpn_devboard (1),
  slab-pcb-left (1), mackerel-30-proto (0 open, starved_thermal 6), rosco
  (0 open, starved_thermal 2).
- **Neither passes**, 44. Both complete, both failing on findings: 4
  (Qfwfq, coco2, eternal-keypad, k30-SBC). The designer route leaves as
  many open or more, so routing alone: 14, 162 open ours against 172
  (RP2040_base 47 / 47, ATAT1800 29 / 30, neotron-pico 24 / 24, jiran
  20 / 20, Sisu 15 / 18, ghoul 6 / 6, MIDAS-MK1.1 5 / 6, reCamera 4 / 4,
  laptop keyboard 4 / 4, interchange-pcb-right 4 / 7, capybully 1 / 3,
  BAGEL-MK1.1-Avocado, d20 and silkscreen_pcb 1 / 1). Ours leaves more,
  so both: 24 (ours / designer route): Glasgow 160 / 69, MegaDrive
  150 / 71, SNSP-CPU-01 136 / 23, DSP-ADAU1452 64 / 3, protoesp 58 / 18,
  4in1-mini 46 / 4, SmartSpin2k 39 / 0 (the designer route fails only on
  the capped invalid_outline), split72_left 36 / 4, Quanta75 Bare
  34 / 13, split72_right 30 / 5, split42_right 26 / 5, koeg 25 / 1,
  jetson 24 / 2, Hub 22 / 16, WLED_Matrix 12 / 1, the A13 module 12 / 7,
  link 11 / 1, split42_left 11 / 1, woagboard 8 / 1, eurorack-pmod
  6 / 1, MIDAS-MK1 6 / 5, laptop motherboard 4 / 2, vaio_re 4 / 3,
  osprey 2 / 1. No designer result: katia (ours 75) and Batch_1 (415).
- **Ours passes, the designer's placement does not**, 5: OpenFC (the
  designer route 1 open), framework_mobo_lefthalf, hackrf-one and
  keyboar_ (one starved thermal each), Quanta75 RP2040Stamp (clearance 2,
  shorting_items 1, solder_mask_bridge 1).
- Both pass: 37. No layout result: the 0xCB pcb-panel.

Two router gaps that are modelling, not search: **net ties** are not
modelled (no code mentions them): every one of neotron-pico's 24 open
connections on both placements is a pad of one of its twelve net-tie
footprints (NT901 to NT912, an EEPROM address line to +3V3 or GND), and
so are all six of ghoul's and 10 to 13 of protoesp's (40 of the designer
route's 430 on target boards). And **pads of one footprint with the same
number** (jiran-ble-lite's USB shield pads S1 and the stacked GND pads,
interchange-pcb-right, capybully: 15 more), which KiCad counts as
separate items to join.

**What it says.** Placement is the larger lever: on the target boards our
placement adds 1053 open connections to the 430 our router leaves on the
designer's placement, 13 boards pass on the designer's placement and not
on ours against 5 the other way, and 24 of the 44 boards that fail on
both leave more open on ours. The placement gap is concentrated: video
199, SNSP-CPU-01 113, the OpenESC 4in1 111, Glasgow 91, MegaDrive 79, the
0xCB panel 65, DSP-ADAU1452 61, 4in1-mini 42, protoesp 40, SmartSpin2k 39
and split72_left 32 hold 872 of the 1064 (82 %), dense or bus-heavy
boards that the designer's placement makes routable for this router. The
routing gap is spread over 38 target boards, more than half of it on
five (MegaDrive 71, Glasgow 69, RP2040_base 47, ATAT1800 30, neotron-pico 24), and 55 of
the 430 are the two modelling gaps above. Earlier placement knobs that
counted pins (routing room, routing demand) did not move the dense
boards (docs/handover.md, open problem 4), RUDY predicts a final result
at a Spearman of 0.30 and the 75 s probe at 0.55 (2026-10-10): what the
placement lacks is a routability measure that is both fast and good. A
coarse global route (tiles with per-layer capacity, negotiated, seconds a
board) is that measure, and is the corridor plan the detailed router
could follow later. Cheaper first: net ties (neotron-pico's and ghoul's
designer routes would be complete) and the starved thermals that alone
fail four boards (open problem 5).

## Layout sweep v19 (2026-10-22)

The first full measurement on one binary after the packages of
2026-10-21 and 22 (net ties and solid thermals, no vias in SMD pads,
all pour work charged, probe overhead, strict rule 3, the free dimension
and the layout budget fixes, the harness review fixes).
`build/github-layout-v19.log`, binary x1539 (main af9d32a, sha256
ded67b2895c1), tasks v7 (`benchmarks/agent-tasks/tasks/v7/tasks.json`,
sha256 c8f95caf5089, the same list as v18), with `--quality`, at the pinned
work rates (`work_rate` 3.15e6, `placer_work_rate` 150e6, PCB_WORK_RATE;
the measured router rate had a median of 2.2e6, work over wall 0.87),
10 jobs x 2 threads through the tier lock with 14 GB kept free, in one
part: 18399 s wall, largest peak 10.4 GB, beside other agents' runs (load
median 12, peaks of 39). v18 is on x1315, so the comparison below is
between two binaries. v18's rows were judged under strict rule 1 (four
under rule 3); `rejudge.py` judged them again under rule 3
(`build/github-layout-v18-rule3.log`) and no row changed: 46 / 42.

| | v18 (x1315, rule 3) | v19 (x1539) |
|---|---|---|
| pass, per-type rule | 46 of 109 | **52 of 109** |
| pass, strict rule (rule 3) | 42 of 109 | **49 of 109** |
| unconnected (all rows) | 2712 | 1827 |
| unconnected, the 99 boards with a result in both | 1948 | 1827 |
| new DRC errors (per-type rule) | 50 | 43 |
| new DRC errors (strict rule) | 1543 | 1326 |
| vias | 31044 (103 rows) | 31642 (99 rows) |
| length | 788 m | 789 m |
| work s | 86094 (103 rows) | 85552 (99 rows) |
| CPU s (unpaired) | 152463 | 134523 |
| rows cut by the wall clock | 2 (katia, Batch_1) | **0** |
| rows killed (exit -9 or 137) | 0 | 0 |
| rows without a result | 6 | 10 |
| our vias in small SMD pads (score-kicad-board) | 1299 on 74 of 103 boards | **0 on all 99** |
| vias against the designer's (median) | x0.71 | x0.74 |
| length against the designer's (median) | x1.03 | x1.08 |
| open over connections (median, mean) | 0.009, 0.284 | 0.000, 0.090 |
| final ladder's share of the budget (median) | 0.695 (62 rows) | 0.640 (58 rows) |

The unconnected total of all rows falls by 885, but 764 of that is four
boards that have no result in v19 (below); on the 99 boards with a result
in both it falls by 121. compare.py: better 28, same 53, worse 16 (the two
v18 rows cut left out). The medians are over different board sets (103
and 99). **The ground pour cut is not comparable**: x1539's
score-kicad-board gives the designer's own boards no cut on 79 of 94 (x1315:
19 of 96; on hackrf-one's source the same file scores 76.95 mm under x1315
and 0 under x1539, our v19 result 2294.93 mm under both), so the median
"+2142 mm against the designer's" is a scorer change, not a routing one; our
raw median is 2869 mm in v18 and 3166 mm in v19. The designer's own vias in
pads also count differently (3211 on 65 boards under x1315, 2218 on 53
under x1539: the exposed-pad and same-number pad grouping).

Better, the largest: DSP-ADAU1452 64 to 30 open, neotron-pico
24 to 0 (every open connection was a net-tie pad), SNSP-CPU-01 136 to 114,
jiran 20 to 1, jetson 24 to 5, koeg 25 to 9, the 0xCB panel 65 to 53,
split72_left 36 to 24, the A13 module 12 to 0, SUMEC 14 to 4. Uncut now:
katia 75 to 38 and Batch_1 415 to 234. Worse, the largest: vaio_re 4 to 89, MegaDrive
150 to 222, protoesp 58 to 106, 4in1-mini 46 to 81, Glasgow 160 to 191,
the OpenESC 4in1 111 to 127, Sisu 15 to 27, split42_right 26 to 31,
ATAT1800 29 to 34, eurorack-pmod 6 to 11.

Pass status that changed (strict rule):

- **Gained, 9.** Starved thermals gone (solid thermals): mackerel-30-proto
  (6), rosco's `kicad` (2), coco2 (1) and k30-SBC (1); copper to the edge
  gone: silkscreen_pcb (copper_edge_clearance 6); the last open
  connection routed: zpn_devboard, BAGEL-MK1.1-Avocado, Explorer (2) and
  slab-pcb-left. interchange-pcb-right gains the per-type pass only (4
  open to 0; copper_edge_clearance 24, which the route on the designer's
  placement has as well).
- **Lost, 2.** OM-FlexGrid-Flex has no result: "1 parts without a legal
  place (J1)", J1 held by an edge, region or overhang constraint (the
  held-parts and free dimension changes; it passed in v18). homebrew_486
  leaves one connection open: its GND zone on F.Cu, B.Cu and In2.Cu in two
  islands (KiCad's unconnected item is the zone against itself at
  (170.9, 23.4)).

No vias in SMD pads anywhere, so no board changed by that rule's findings.
Strict rule 3 changed no v18 row.

Rows without a result, 10 (v18: 6): the 0xCB pcb-panel (kicad-cli's DRC
times out after 900 s, as in v18), and nine refusals with parts without a
legal place: the five molecule panels (4x4, 4x5, 5x2_shifted,
7x5_wave_left as in v18 with more parts each; 5x4_wave new, "J3, J4,
silkscreen mark", 499 open in v18), the OpenESC 4in1 panel (42 parts, 216
in v18), and three that had a result in v18: the OpenRX panel (81 parts;
226 open in v18), SmartSpin2k_Panelized (12 parts; 39 open) and
OM-FlexGrid-Flex (J1; a pass). All but OM-FlexGrid-Flex are panels and
not targets (docs/handover.md, "Not targets"). A drawn item ending beyond
the outline is infeasible since the free dimension fixes (e4cc2cc), which
is the likely cause of the "silkscreen mark" refusals.

Boards that fail the strict rule with nothing open: Qfwfq
(hole_clearance) and krishveercard (clearance), each at KiCad's cap of
199 (per-type passes); eternal-keypad (shorting_items 1, hole_clearance
199); neotron-pico (hole_clearance 2, the designer route 1); the A13
module (items_not_allowed 2; the designer route items_not_allowed 1 and 1 open);
interchange-pcb-right (copper_edge_clearance 24, the designer route the
same).

### Placement or routing on one binary (matched designer tier, x1539)

`build/designer-x1539-matched.log`: every board routed by x1539 on the
designer's own placement at `--budget-scale 0.695` (1042.5 s of work, v18's
median final-ladder share), 8 x 1 through the lock, 14 GB kept free; in
two parts on the same binary (`build/designer-x1539-matched-a.log`, 11
rows, stopped and spliced in with `--splice-from`; the rest 19173 s wall),
no row cut or killed, one without a result (the 0xCB pcb-panel, kicad-cli
times out). **54 of 109 pass the strict rule** (61 the per-type rule);
vias x0.49 of the designer's, length x0.99 (medians). Against the matched
x1404 run (`build/designer-x1404-matched.log`): better 17, same 90, worse
1, unconnected 4098 -> 3594, passes 59 -> 61 (strict 51 -> 54).

v19's own final-ladder share is lower than v18's: median 0.640 over the 58
rows where it ran (0.624 over the 39 of them left open; compare.py --gap
says 0.638 over 56 target rows). So at 0.695 the designer route had some
8 % more routing work than v19's final ladder, and the routing gap below is
a slight underestimate (the placement share a slight overestimate).

`compare.py --gap build/github-layout-v19.log build/designer-x1539-matched.log --targets`:
over the 97 target boards with both results, **layout open 1823,
designer-route open 747: placement gap +1076 (59 %), routing gap 747
(41 %)**; ours is worse on 33 boards, the same on 58, better on 6. No
layout result: the 0xCB pcb-panel, OM-FlexGrid-Flex and
SmartSpin2k_Panelized. (x1404 matched against v18: 1973 and 1016, 49 / 51.)

By the strict pass on both placements (100 target boards):

- **Placement** (the designer's placement passes, ours does not), 9:
  video (ours 199 open), the OpenESC 4in1 (127), the 0xCB panel (53), ULK
  (4), ULK_sl_PG1316s (2), laptop power (2), homebrew_486 (1), capybully
  (1), OM-FlexGrid-Flex (no legal place for J1).
- **Ours passes, the designer's placement does not**, 4: Quanta75
  RP2040Stamp (the designer route: clearance 2, shorting_items 2,
  solder_mask_bridge 1), k30-SBC (one starved thermal), OpenFC (1 open),
  BAGEL-MK1.1-Avocado (2 open).
- Both pass: 45. Neither passes: 42, among them the largest placement
  gaps (ours / designer route open): Glasgow 191 / 50, MegaDrive 222 / 109,
  protoesp 106 / 1, 4in1-mini 81 / 2, vaio_re 89 / 10, SNSP-CPU-01
  114 / 46, split42_right 31 / 3, DSP-ADAU1452 30 / 4; and routing alone
  (the designer route leaves as many): RP2040_base 47 / 48, ATAT1800
  34 / 34, Sisu 27 / 26, OpenAirScope 4 / 4, reCamera 4 / 4.

**Where a fix gains most** (target boards by the open connections our
placement adds, with what KiCad reports on our result): video 199 open
(267/371 routed, no other finding; the designer route 0), Glasgow 191
(113/226 routed; designer route 50), the OpenESC 4in1 127 (88/152;
designer route 0), MegaDrive 222 (224/298; designer route 109, so +113),
protoesp 106 (65/104, solder_mask_bridge at KiCad's cap and clearance 2;
designer route 1 open and the same capped mask bridges). For passes, the
nearest are the placement boards above with one to four open (ULK,
ULK_sl_PG1316s, laptop power, homebrew_486, capybully) and OM-FlexGrid-Flex's
refusal.

## Layout sweep v20 (2026-10-23)

The second full measurement on one binary, after the packages of
2026-10-23: trouble repair on by default (in the first route too; its
victims by the trouble-maker's open terminals; cheaper exchanges, the
victims on their corridors), the pin-row cost on at 2 mm, straightening
after the thermal spoke repairs, the placer refusals (a held part's
narrow window found by the eviction), touch electrodes (vias into an
electrode under its own mask opening, antenna copper in the placer's
body), the silkscreen fields placed where KiCad finds nothing, router
memory rounds 1 and 2 and the memory ledger; differential pairs, bus
bundles, global guidance and the move descent are in the binary but off.
`build/github-layout-v20.log`, binary x1725 (main 38edddd, sha256
bc12878b8f20), tasks v7 (sha256 c8f95caf5089, the same list as v18 and
v19), `--quality`, at the pinned work rates (the measured router rate had
a median of 2.1e6, work over wall 0.85), 10 jobs x 2 threads through the
memory ledger (no lock) with 10 GB kept free, in one part: 26143 s wall
(v19: 18399 s; most of the difference is admission: the last boards
waited up to 2.7 hours for memory beside other agents' runs), largest
peak 7.0 GB (v19: 10.4), load median 11.5.

**Two harness changes make the comparison with v19 inexact.** (1) The
task labels (14831a6, benchmarks/agent-tasks/README.md "Tasks"): run.py's
`unplace` now takes off the board the texts and silkscreen graphics that
label a stacked part. 77 of the 109 tasks drop labels (2824 in all), so
those boards start from a prepared board that differs from v19's, and
their placements differ for that reason as well as the binary's; the
task and board digests are the same, so compare.py does not mark them
INPUT. Of the 99 boards with a result in both, 18 of the 23 better and
13 of the 17 worse drop labels (38 of the 59 the same). (2) Strict rule 4
(a finding between items of one footprint is the designer's; a finding
at the same place is subtracted also of a type at KiCad's cap; a
copper-to-edge finding is placed by its copper item). v20 is judged under
rule 4; v19's rows were rule 3 and `rejudge.py` judged them again
(`build/github-layout-v19-rule4.log`): 3 rows changed, all gaining the
strict pass with nothing open: krishveercard (clearance 199 to none),
Qfwfq (hole_clearance 199 to none) and interchange-pcb-right
(copper_edge_clearance 24 to none); v19 strict 49 -> 52, strict errors
1326 -> 34. The table compares v19 under rule 4. The aesthetics and the
ground pour cut of v19 are scored again by x1725's score-kicad-board
(`compare.py --score`, the rows' own scores of x1539 removed: x1539 read
no net on a KiCad 9 board's tracks), so both columns are scored alike.

| | v19 (x1539, rule 4) | v20 (x1725) |
|---|---|---|
| pass, per-type rule | 52 of 109 | **53 of 109** |
| pass, strict rule (rule 4) | 52 of 109 | **53 of 109** |
| unconnected (all rows) | 1827 | 1848 |
| unconnected, the 99 boards with a result in both | 1827 | **1568** |
| new DRC errors (per-type rule) | 43 | 34 |
| new DRC errors (strict rule) | 34 | 26 |
| vias | 31642 (99 rows) | 33748 (102 rows); 31688 on the 99 |
| length | 789 m (99 rows) | 809 m (102 rows); 778 m on the 99 |
| work s | 85552 (99 rows) | 83334 (102 rows); 80261 on the 99 |
| CPU s (unpaired) | 134523 | 134234 |
| first-route open terminals (the 99) | 3997 | 3395 |
| rows cut by the wall clock | 0 | **0** |
| rows killed (exit -9 or 137) | 0 | **0** |
| rows without a result | 10 | **7** |
| our vias in small SMD pads (score-kicad-board) | 0 on all 99 | **0 on all 102** |
| vias against the designer's (median, the 99) | x0.74 | x0.76 |
| length against the designer's (median, the 99) | x1.08 | x1.05 |
| open over connections (median, mean; the 99) | 0.000, 0.090 | 0.000, 0.077 |
| bends per connection against the designer's (median, the 99) | x1.20 | **x1.11** |
| stair-steps, ours less the designer's (median, the 99) | +88 | **+59** |
| pin-row crossings, ours less the designer's (median, the 99) | +1 | +1 |
| ground pour cut, ours less the designer's (median) | +0 mm (94 boards) | +0 mm (94 boards) |
| final ladder's share of the budget (median) | 0.640 (58 rows) | **0.721** (49 rows) |

Over all 102 boards with a result v20's medians are vias x0.76, length
x1.05, open 0.000 (mean 0.091), bends x1.11, stair-steps +60.5, crossings
+1, the ground pour cut +0 mm (95 boards; on one, a board without a
result in v19, ours lost the designer's ground pour). compare.py: better 23, same 59, worse 17; on the 99 boards with a
result in both, unconnected 1827 -> 1568 (-259). The all-rows total rises
by 21 because two panels that refused in v19 have a result now (the
OpenRX panel 200 open, SmartSpin2k_Panelized 80).

Better, the largest: vaio_re 89 to 10, the OpenESC 4in1 127 to 79,
4in1-mini 81 to 37, Glasgow 191 to 148, video 199 to 165, MegaDrive 222 to
191, protoesp 106 to 76, SNSP-CPU-01 114 to 91, DSP-ADAU1452 30 to 15,
ATAT1800 34 to 28. Worse, the largest: Batch_1 234 to 268, the A13 module
0 to 18, split72_left 24 to 38, eurorack-pmod 11 to 21, jetson 5 to 15,
SUMEC 4 to 12, RP2040_base 47 to 52, Quanta75 BareRP2040 30 to 34, the
Hub 24 to 28, Explorer 0 to 3.

Pass status that changed (strict and per-type alike):

- **Gained, 4.** OM-FlexGrid-Flex places again (v19: "1 parts without a
  legal place (J1)"; the placer refusals' eviction finds the held
  connector's narrow window): 19/19, clean. homebrew_486: v19's one open
  item was its GND zone in two islands (F.Cu, B.Cu, In2.Cu at (170.9,
  23.4)), joined now. capybully: v19's one open item was SW20's GND pad 3
  between F.Cu and B.Cu (a through pad's two sides unjoined), routed now.
  d20_pcb: v19 left J2's pad 1 (net PAD) apart from its B.Cu track,
  routed now.
- **Lost, 3**, each by open connections, no new error: BAGEL-MK1.1-Avocado
  (1 open: U401 pad 14 to U402 pad 6, Net-(U401-OUTA), both on B.Cu;
  labels dropped 50), Explorer (3 open: two of them a track stub of
  D+3V3 and MCU_SDA by U16 on F.Cu that does not reach its pad, one
  RS232F_TXD from a B.Cu stub to U12; no labels dropped), hackrf-one (1
  open: U21 pad 2 to R51 pad 1, Net-(R51-Pad1), on C1F; labels dropped 9).

No vias in SMD pads on any board, so no board changed by that rule.

Rows without a result, 7 (v19: 10): the 0xCB pcb-panel (kicad-cli's DRC
times out after 900 s, as in v18 and v19) and six panels refused with
parts without a legal place, all fewer than in v19: the OpenESC 4in1
panel (26 parts; 42), the molecule 4x4 (18; 27), 4x5 (35; 42),
5x2_shifted (10; 25), 7x5_wave_left (3; 4) and 5x4_wave (J3, J4; 3). None
is a target. The OpenRX panel and SmartSpin2k_Panelized have a result
now (200 and 80 open).

Boards that fail the strict rule with nothing open, 3: neotron-pico
(hole_clearance 3), ghoul (clearance 1; 2 open in v19) and eternal-keypad
(shorting_items 1). Of the 56 that fail, 18 are within five connections
and five errors of clean, 20 leave more than 20 open, 7 have no result.

**Since v20.** x1753 (the narrow step's bound, which affects
DSP-ADAU1452 and Glasgow in layout mode) and x1754 (escape demand on by
default; dense tier sum of medians 968 -> 933 over three seed pairs)
post-date this sweep, so v21 is expected to move.

### Placement or routing on one binary (matched designer tier, x1725; partial)

v20's final ladder had a median share of 0.721 of the budget (49 rows;
0.721 over the 47 of them left open; compare.py --gap says 0.714 over 46
target rows), not v19's 0.640, so the matched tier ran at
`--budget-scale 0.721` (1081.5 s of work). A first start at 0.64 (before
the sweep's share was known; 2 rows, `build/designer-x1725-s064-a`, and 4
of a second part, `-s064-b`) was stopped and is not used.

**The tier is not complete.** Started beside the sweep, its eight job
slots waited four hours on the largest boards (known issues above); then
the machine stayed full (other agents' runs and the nightly of
2026-10-24 on 4d9965e, about 20 GB available with 10 kept free), and it
was cut to the boards that matter for the matched comparison, the
sweep's 48 strict-failing target boards, in parts on the same binary:
`build/designer-x1725-matched-a.log` (4 rows), `-b` (23 more, spliced
from -a), `build/designer-x1725-matched.log` (the rest, spliced from -b).
At the time of writing the last part had routed none of its 23 boards
(the five of 5.8 to 6 GB expected: split72_right, DSP-ADAU1452, the
laptop keyboard, SmartSpin2k_Panelized, SNSP-CPU-01; and the 18 of 6 to
27 GB: the laptop motherboard, link, split72_left, ATAT1800, the Hub,
split42_right, koeg, katia, neotron-pico, video, Quanta75 BareRP2040, the
0xCB panel, woagboard, ghoul, jiran, ULK_sl_PG1316s, ULK, osprey). So the
gap below is over the 25 target boards with both results, about half of
the failing ones, and is not comparable with v19's 97-board split.
Rows: 27, none cut or killed; 5 pass the strict rule:
interchange-pcb-right and four that ours fails, the OpenESC 4in1 (ours
79 open), laptop power (1), hackrf-one (1) and Explorer (3); USB_Keypad
passes the per-type rule only (connection_width 1). eternal-keypad fails
on both placements by the same shorting_items 1. Over the same
26 boards x1539's matched route left 1094 open, x1725's 998.

`compare.py --gap build/github-layout-v20.log build/designer-x1725-matched.log --targets`
over the 25 target boards with both results: **layout open 986,
designer-route open 564: placement gap +422 (43 %), routing gap 564
(57 %)**; ours is worse on 18, the same on 4, better on 3. One board
dominates the routing side: Batch_1, where the designer route leaves 342
open and ours 268 (its business cards' GND rings; placement gap -74).
Without it, 718 against 222: placement 69 %, routing 31 %.

**Where a fix gains most**, by the open connections our placement adds
on the boards measured (ours / designer route): MegaDrive 191 / 56
(+135), Glasgow 148 / 55 (+93), the OpenESC 4in1 79 / 0 (+79), protoesp
76 / 1 (+75), 4in1-mini 37 / 2 (+35). Not measured on x1725 but larger
on v19's matched run: video (ours 165 open in v20; the designer route
left 0 under x1539) and SNSP-CPU-01 (91; 46 under x1539). Routing alone
(the designer route leaves as many): RP2040_base 52 / 46, Sisu 28 / 22,
OpenAirScope 4 / 4, reCamera 4 / 4. For passes, the nearest are laptop
power, hackrf-one and BAGEL-MK1.1-Avocado (one open each; the designer
route passes the first two and leaves BAGEL two), Explorer (3 open, the
designer route passes) and the boards with nothing open above.

## Layout sweep v21 (2026-10-26)

The full measurement after the packages of 2026-10-24 to 26: ratsnest
crossings on by default (x1817), the race gate fix with its 5 % pick
margin (x1828), the thermals solidified first and ordered admission
(x1790), the pad-clearance floor from the layer rules (x1814),
per-layer widths and the connection-width floor (x1815, x1816),
differential pairs by rule (x1798, x1799), the panels fixes (x1825), the
net tie's vias (x1830), the default-losers switches (x1831) and memory
round 3 (x1832, routes identical). `build/github-layout-v21.log`, binary
x1832 (main 2087046, sha256 5e9898f282f3), tasks v7 (sha256
c8f95caf5089, the same list as v18 to v20), `--quality`, at the pinned
work rates (the measured router rate had a median of 1.5e6, work over
wall 0.67: four other agents shared the machine), 10 jobs x 2 threads
through the memory ledger (no lock) with 10 GB kept free and
`--ledger-priority nightly` (and `--order peak-desc`, which does nothing
without estimates), as scripts/nightly.sh runs it, except for
the memory estimates: the task list carries none, so run.py declares
`--expected-gb` for every board, and the sweep ran in three parts by
the boards' measured peaks (the largest of v20's and the nightly's rows)
so that no board is declared below its measured peak:
`build/github-layout-v21-a.log` (5 boards of 3.1-3.6 GB measured,
`--expected-gb 4`, 1407 s) and `-b` (7 boards of 4.5-7.2 GB, 8 GB,
2486 s) side by side, both merged into `build/github-layout-v21-ab.log`,
then the other 97 boards (at most 3.1 GB measured, 3.5 GB) with
`--splice-from` that log (10954 s): 12 rows spliced, all of x1832.
07:31 to 11:13, largest peak 4.6 GB (ULK; v20 7.0 GB, the nightly
6.0), no row cut or killed.

**Inputs.** 14 boards gained `.kicad_dru` files on 2026-10-24 (12:38)
after v20, so compare.py marks them INPUT against v20 and the comparison
with v20 is over the other 95; the nightly of 2026-10-25 ran on the
same inputs as v21 (no board INPUT).

| | v20 (x1725) | nightly 2026-10-25 (356c8db, x1817) | v21 (x1832) |
|---|---|---|---|
| pass, per-type rule | 53 of 109 | 53 of 109 | **56 of 109** |
| pass, strict rule (rule 4) | 53 of 109 | 53 of 109 | **56 of 109** |
| strict passes on the 95 boards of the same input as v20 | 49 | - | 53 |
| unconnected (all rows) | 1848 | 1788 | 2444 |
| unconnected where both have a result (that run / v21) | 1651 / **1561** (88 boards, same input) | 1788 / **1672** (100 boards) | - |
| new DRC errors (per-type rule) | 34 | 56 | 50 |
| new DRC errors (strict rule) | 26 | 49 | 44 |
| vias (rows with a result) | 33748 (102) | 32671 (100) | 33673 (104) |
| length (rows with a result) | 809 m (102) | 791 m (100) | 800 m (104) |
| work s | 83334 (102) | 78342 (100) | 83193 (104) |
| CPU s (unpaired, a loaded machine) | 134234 | 135906 | 162932 |
| first-route open terminals | 3878 (102) | 3672 (100) | 4908 (104) |
| rows cut by the wall clock / killed | 0 / 0 | 0 / 0 | **0 / 0** |
| rows without a result | 7 | 9 | **5** |
| our vias in pads (score-kicad-board) | 0 on all 102 | 0 on all 100 | **0 on all 104** |
| vias against the designer's (median) | x0.76 (102) | x0.72 (100) | x0.72 (104) |
| length against the designer's (median) | x1.05 | x1.05 | x1.04 |
| open over connections (median, mean) | 0.000, 0.091 | 0.000, 0.097 | 0.000, 0.124 |
| bends per connection against the designer's (median) | x1.11 | x1.05 | x1.06 |
| stair-steps, ours less the designer's (median) | +60.5 | +56 | +63.5 |
| pin-row crossings, ours less the designer's (median) | +1 | +0 | +0.5 |
| ground pour cut, ours less the designer's (median) | +0 mm (95; 1 lost) | +0 mm (93; 1 lost) | +0 mm (97; 1 lost) |
| final ladder's share of the budget (median) | 0.721 (49 rows) | 0.772 (41 rows) | **0.762** (45 rows) |

The all-rows unconnected total rises because four boards without a
result in the nightly have one now, all with many open: the OpenESC
4in1 panel 499, katia 162, the 0xCB pcb-panel 93 and laptop power 18
(772 in all). On the boards with a result in both, v21 leaves fewer open
than either: 1672 against the nightly's 1788 (100 boards) and 1561
against v20's 1651 (the 88 of the same input with a result in both;
vias 29498 against 30189, length 728 m against 745). Over the 95 boards
of the same input compare.py's medians move: vias x0.77 -> x0.75, length
x1.05 -> x1.03, bends x1.10 -> x1.05, stair-steps +59 -> +63.5, crossings
+0.5 -> +0, strict errors 19 -> 9.

**Against the nightly** (compare.py, 109 boards, same inputs): better 6,
same 91, worse 3; 86 rows are identical in routed count, unconnected,
vias, length and strict findings. Better: the OpenRX panel 248 -> 200,
protoesp 105 -> 70, the OpenESC 4in1 139 -> 106, Glasgow 145 -> 143,
koeg 13 -> 11, homebrew_486 1 -> 0. Worse: RP2040_base 47 -> 50, the Hub
21 -> 22, woagboard 4 -> 5. Strict pass gained, 3, none lost:
homebrew_486 (its one open item routed), neotron-pico (hole_clearance 4
-> none) and ghoul (hole_clearance 1 -> none), both with nothing open
before and after.

**Against v20** (compare.py, the 95 boards of the same input): better
20, same 59, worse 9. Better, the largest: MegaDrive 191 -> 110,
SmartSpin2k_Panelized 80 -> 34, SNSP-CPU-01 91 -> 53, ATAT1800 28 -> 5,
split72_left 38 -> 23, jetson 15 -> 1, eurorack-pmod 21 -> 10,
split42_right 27 -> 20, protoesp 76 -> 70, the Hub 28 -> 22. Worse:
katia 37 -> 162, video 165 -> 190, split42_left 5 -> 18, split72_right
20 -> 24, koeg 7 -> 11, DSP-ADAU1452 15 -> 17, woagboard 3 -> 5,
ULK_sl_PG1316s 2 -> 3, Quanta75 BareRP2040 34 -> 35. Strict pass gained,
4: hackrf-one, eternal-keypad, ghoul, neotron-pico; lost, 1, of the
INPUT boards: MokyaLora (clearance 3, nothing open; it gained a
`.kicad_dru`). Of the other INPUT boards, laptop motherboard 3 -> 38,
4in1-mini 37 -> 81, the OpenESC 4in1 79 -> 106 and laptop power 1 -> 18
leave more open, Sisu 28 -> 19, the A13 module 18 -> 5 and SUMEC 12 -> 5
fewer.

**The nine boards without a result in the nightly**:

| Board | nightly 2026-10-25 | v21 |
| --- | --- | --- |
| OpenESC-30x30 4in1-panel | 34 parts without a legal place | 43/184 routed, 499 open |
| katia | 10 parts without a legal place | 72/162 routed, 162 open (v20: 37) |
| laptop power | 27 parts without a legal place | 99/115 routed, 18 open (v20: 1; INPUT) |
| 0xCB-1337 pcb-panel | kicad-cli's DRC timed out after 900 s | 274/340 routed, 93 open |
| PolyKybd molecule 4x4 | 48 parts without a legal place | 29 parts without a legal place |
| molecule 4x5 | 60 | 45 |
| molecule 5x2_shifted | 42 | 3 |
| molecule 5x4_wave | 21 | 5 |
| molecule 7x5_wave_left | 48 | 9 |

Four have a result now, none passes; the five molecule panels still
refuse, with fewer parts left without a place (v20 had 18, 35, 10, 2 and
3). None of the nine is a target.

**Strict errors, 44 on 10 boards**: link length_out_of_range 12, Sisu
items_not_allowed 8 and diff_pair_uncoupled_length_too_long 1, vaio_re
solder_mask_bridge 5, USB_Keypad connection_width 4, WLED_Matrix
connection_width 3 and track_dangling 1, the A13 module items_not_allowed
3, MokyaLora clearance 3, protoesp solder_mask_bridge 1 and clearance 1,
the 0xCB panel starved_thermal 1, osprey copper_edge_clearance 1. One
board fails the strict rule with nothing open: MokyaLora (clearance 3).
Of the 53 that fail, 17 are within five connections and five errors of
clean, 20 leave more than 20 open, 5 have no result.

### Placement or routing on one binary (matched designer tier, x1832)

`build/designer-x1832-matched.log`: all 109 boards routed by x1832 on the
designer's placement at `--budget-scale 0.762` (v21's median final-ladder
share over 45 rows, `nightly-report.py --ladder-share`; 1143 s of work),
`--quality`, 8 jobs x 1 thread, `--order peak-desc` (designer.json's
measured peaks, the largest first), `--min-free-gb 10`, `--ledger-priority
nightly`, in one part after the sweep: 11:14 to 14:33, 11958 s wall,
largest peak 5.0 GB, no row cut or killed, every row with a result.
**66 of 109 pass the per-type rule, 62 the strict rule** (x1539's matched
tier: 61 / 54); 3465 unconnected in all, of which the five PolyKybd
molecule panels leave 2276 (the layout refuses them) and Batch_1 326;
strict errors 55; medians against the designer's own board: vias x0.53,
length x0.98, bends x0.86, stair-steps +7.

`compare.py --gap build/github-layout-v21.log build/designer-x1832-matched.log --targets`
over the **100 target boards with both results** (every target board):
**layout open 1740, designer-route open 742: placement gap +998 (57 %),
routing gap 742 (43 %)**; ours is worse on 33, the same on 60, better on
7. Batch_1 again dominates the routing side (the designer route 326
open, ours 268); without it 1472 against 416: placement 72 %, routing
28 %. Over all 104 boards with both results (the panels included):
2444 against 1189, placement 51 %, routing 49 %. v19's matched split
(x1539, 97 target boards) was 59 / 41; v20's (25 boards) is not
comparable.

**The five boards whose placement costs most** (ours / designer route,
open connections):

| Board | Connections | Layout open | Designer route open | Placement gap |
| --- | --- | --- | --- | --- |
| video | 371 | 190 | 1 | +189 |
| katia | 162 | 162 | 10 | +152 |
| 0xCB-1337 pcb-panel | 340 | 93 | 0 | +93 |
| OpenESC-30x30 4in1 | 152 | 106 | 15 | +91 |
| glasgow revC3 | 226 | 143 | 55 | +88 |

Next: protoesp +69 (70 / 1), MegaDrive +54 (110 / 56), the 0xCB panel
+50 (50 / 0), 4in1-mini +44 (81 / 37), SmartSpin2k_Panelized +34 (34 /
0). Routing first (the designer route leaves as much or more): Batch_1
326 / ours 268, ATAT1800 35 / 5, Sisu 21 / 19, vaio_re 11 / 9, and
RP2040_base 46 / 50, where the placement adds 4.

**Worse than the nightly, for the next triage.** The rows are
deterministic for their binary, so every change is the binary's; by
open connections only three boards are worse than the nightly, all by
one to three: RP2040_base 47 -> 50 (vias 399 -> 482), the Hub 21 -> 22,
woagboard 4 -> 5 (a spliced row of x1832). None loses a pass. The
boards the nightly had no result for and v21 lays out badly are the
larger triage items: katia 162 open (v20 37; the designer route 10) and
laptop power 18 (v20 1; the designer route passes), and, against v20 on
the same input, split42_left 5 -> 18, video 165 -> 190 and split72_right
20 -> 24, the same in the nightly (18, 190, 24), and koeg 7 -> 11 (the
nightly 13); the designer route leaves 4, 1, 1 and 1 on them.

**Since v21**: x1833 (main e935324: exact escape stubs on by default and
the pour model matching KiCad; the expansion tier 589 -> 557 open,
passes 21 -> 24) post-dates this sweep and moves routing, so v22 is
expected to improve.

## Layout sweep v22 (2026-10-27)

<!-- nightly-sweep night=2026-10-27 sweep=build/nightly/2026-10-27/sweep.log main=e7a12a80e4fead0c69e1869bd62d07fc6fa979e7 -->

Written by the nightly (`scripts/nightly-report.py --docs`) from
`build/nightly/2026-10-27/`: main e7a12a8 (72 commits after the nightly
of 2026-10-26's main 5255222), binary
`pcb-maker-nightly-2026-10-27-e7a12a80e4fe` (sha256 b0e3cc1557d2), tasks
v7 (sha256 c8f95caf5089), `--quality`, at the pinned work rates (the
measured router rate had a median of 1.1e6, work over wall 0.46), 10
jobs x 2 threads, `--order peak-desc`, through the memory ledger with
`--ledger-priority nightly`: 19232 s wall, largest peak 4.7 GB, no row
cut or killed in any stage. Per board:
`build/nightly/2026-10-27/summary.md`, `diff-sweep.txt` and `gap.txt`.

**Inputs.** Every board in both ran on the same input as in nightly
2026-10-26; every board in both ran on the same input as in v21 (x1832).

| | v21 (x1832) | nightly 2026-10-26 (5255222) | v22 (nightly 2026-10-27, e7a12a8) |
|---|---|---|---|
| pass, per-type rule | 56 of 109 | 57 of 109 | **57 of 109** |
| pass, strict rule (rule 4) | 56 of 109 | 57 of 109 | **57 of 109** |
| unconnected (all rows) | 2444 | 2284 | 2140 |
| unconnected where both have a result on the same input (that run / this) | 2444 / **2140** (103 boards) | 2284 / **2140** (103 boards) | - |
| new DRC errors (per-type rule) | 50 | 62 | 61 |
| new DRC errors (strict rule) | 44 | 56 | 53 |
| vias (rows with a result) | 33673 (104) | 33853 (104) | 32910 (103) |
| length (rows with a result) | 800 m (104) | 805 m (104) | 786 m (103) |
| work s | 83193 (104) | 80411 (104) | 78901 (103) |
| CPU s (unpaired, a shared machine) | 162932 | 219121 | 211436 |
| first-route open terminals | 4908 (104) | 4844 (104) | 4453 (103) |
| rows cut by the wall clock / killed | 0 / 0 | 0 / 0 | **0 / 0** |
| rows without a result | 5 | 5 | 6 |
| our vias in small SMD pads (score-kicad-board) | 0 on all 104 | 3 on 2 of 104 (koeg-board-pcb, bumwings_v001_core) | 3 on 2 of 103 (koeg-board-pcb, bumwings_v001_core) |
| vias against the designer's (median) | x0.72 (104) | x0.75 (104) | x0.73 (103) |
| length against the designer's (median) | x1.04 (104) | x1.05 (104) | x1.07 (103) |
| open over connections (median, mean) | 0.000, 0.124 | 0.000, 0.115 | 0.000, 0.110 |
| bends per connection against the designer's (median) | x1.06 | x1.13 | x1.15 |
| stair-steps, ours less the designer's (median) | +63.5 | +60.5 | +60 |
| pin-row crossings, ours less the designer's (median) | +0.5 | +1 | +1 |
| ground pour cut, ours less the designer's (median) | +0 mm (97; 1 lost) | +0 mm (97; 1 lost) | +0 mm (96; 1 lost) |
| final ladder's share of the budget (median) | 0.762 (45 rows) | 0.690 (41 rows) | **0.680** (40 rows) |

**Against the nightly of 2026-10-26** (compare.py, 109 boards in both on
the same input): better 4, same 99, worse 0; 98 of the 103 with a result
in both are identical in routed count, unconnected, vias, length and
strict findings; unconnected there 2284 -> 2140. Better, the largest:
0xCB-1337 panel 60 -> 0, 0xCB-1337 pcb-panel 93 -> 43, OpenRX-panel-rev2
213 -> 191, SmartSpin2k_Panelized 33 -> 21. Strict passes 57 -> 57:
gained 1: 0xCB-1337 panel (60 open, starved_thermal 2 -> none); lost 1:
Quanta75_RP2040Stamp_JLCPCBAoptimized (internal error: memory: the
process grew to 1474 MB resident, past its reservati).

**Against v21** (compare.py, 109 boards in both on the same input):
better 21, same 76, worse 6; 33 of the 103 with a result in both are
identical in routed count, unconnected, vias, length and strict
findings; unconnected there 2444 -> 2140. Better, the largest: katia 162
-> 39, 0xCB-1337 panel 50 -> 0, 0xCB-1337 pcb-panel 93 -> 43,
Quanta75_BareRP2040_JLCPCBAoptimized 35 -> 4, RP2040_base 50 -> 37,
SmartSpin2k_Panelized 34 -> 21, OpenRX-panel-rev2 200 -> 191, laptop
motherboard 38 -> 29, DSP-ADAU1452 17 -> 11, link 10 -> 4 and 11 more.
Worse: ATAT1800 5 -> 19, poly_corne_split42_right 20 -> 25,
koeg-board-pcb 11 -> 15, protoesp 70 -> 73, ULK 2 -> 4, vaio_re 9 -> 10.
Strict passes 56 -> 57: gained 2: 0xCB-1337 panel (50 open,
starved_thermal 1 -> none), Explorer (1 open -> none); lost 1:
Quanta75_RP2040Stamp_JLCPCBAoptimized (internal error: memory: the
process grew to 1474 MB resident, past its reservati).

**Rows without a result, 6** (the nightly of 2026-10-26: 5):
poly_kb_molecule_4x4 (31 parts without a legal place; the previous night
31), poly_kb_molecule_4x5 (42 parts without a legal place; the previous
night 42), poly_kb_molecule_5x2_shifted (22 parts without a legal place;
the previous night 22), poly_kb_molecule_5x4_wave (5 parts without a
legal place; the previous night 5), poly_kb_molecule_7x5_wave_left (6
parts without a legal place; the previous night 6),
Quanta75_RP2040Stamp_JLCPCBAoptimized (internal error: memory: the
process grew to 1474 MB resident, past its reservati; the previous night
none).

**Strict errors, 53 on 11 boards**: link (length_out_of_range 12),
Sisu_SSE-9 (items_not_allowed 10, diff_pair_uncoupled_length_too_long 1,
skew_out_of_range 1), poly_kybd_split72_right (clearance 5), vaio_re
(solder_mask_bridge 5), MokyaLora (clearance 4), USB_Keypad
(connection_width 4), WLED_Matrix (connection_width 3, track_dangling
1), koeg-board-pcb (clearance 2, shorting_items 1), sbc_allwinner_a13
module (items_not_allowed 2), osprey_rev_a (copper_edge_clearance 1),
protoesp (clearance 1). Failing the strict rule with nothing open, 1:
MokyaLora (clearance 4). Of the 52 that fail, 16 are within five
connections and five errors of clean, 18 leave more than 20 open, 6 have
no result.

### Placement or routing on one binary (matched designer tier, nightly 2026-10-27)

`build/nightly/2026-10-27/designer.log`: 109 boards routed by the same
binary on the designer's placement at `--budget-scale 0.69` (the median
final-ladder share of the previous night's sweep, `nightly-report.py
--ladder-share`; 1035 s of work), `--quality`, 8 jobs x 1 thread,
`--order peak-desc`: 12283 s wall, largest peak 5.0 GB, no row cut or
killed, 109 with a result. **67 of 109 pass the per-type rule, 63 the
strict rule**; 3335 unconnected in all; strict errors 37; medians
against the designer's own board: vias x0.53, length x1.00, bends x0.87,
stair-steps +10.

`compare.py --gap build/nightly/2026-10-27/sweep.log
build/nightly/2026-10-27/designer.log --targets` over the **99 target
boards with both results** (of 100): **layout open 1445, designer-route
open 688: placement gap +757 (52 %), routing gap 688 (48 %)**; ours is
worse on 31, the same on 62, better on 6. Batch_1 dominates the routing
side (the designer route 322 open, ours 268); without it 1177 against
366: placement 69 %, routing 31 %. Over all 103 boards with both results
(the panels included): 2140 against 1085, placement 49 %, routing 51 %.
The previous night's split (2026-10-26, 100 target boards): 57 % / 43 %.

**The five boards whose placement costs most** (ours / designer route, open connections):

| Board | Connections | Layout open | Designer route open | Placement gap |
| --- | --- | --- | --- | --- |
| video | 371 | 190 | 1 | +189 |
| OpenESC-30x30 4in1 | 152 | 106 | 15 | +91 |
| glasgow revC3 | 226 | 143 | 55 | +88 |
| protoesp | 104 | 73 | 1 | +72 |
| 4in1-mini | 152 | 81 | 37 | +44 |

Next: 0xCB-1337 pcb-panel +43 (43 / 0), MegaDrive +39 (110 / 71),
poly_corne_split42_right +24 (25 / 1), SmartSpin2k_Panelized +21 (21 /
0), katia +21 (39 / 18). Routing first (the designer route leaves as
much or more): Batch_1 322 / ours 268, Sisu_SSE-9 21 / ours 18, vaio_re
11 / ours 10, MIDAS-MK1.1-revA 5 / ours 5, reCamera_S101_v1.1 4 / ours
4, BAGEL-MK1.1-Avocado 2 / ours 1 and 5 more.

**Worse than the nightly of 2026-10-26, beyond noise** (the next triage;
a layout is deterministic for its binary, so every change is the
binary's; on the dense boards of `dense-noise.json` a loss counts only
when it exceeds the spread of the board's unconnected count over the
placer seeds there), 0: none. Losing the strict pass:
Quanta75_RP2040Stamp_JLCPCBAoptimized (internal error: memory: the
process grew to 1474 MB resident, past its reservati).

## Layout sweep v23 (2026-10-28)

<!-- nightly-sweep night=2026-10-28 sweep=build/nightly/2026-10-28/sweep.log main=be0e62984304dfa3a6a21531e675b3548779512b -->

Written by the nightly (`scripts/nightly-report.py --docs`) from
`build/nightly/2026-10-28/`: main be0e629 (99 commits after v22's main
e7a12a8; 99 commits after the nightly of 2026-10-27's main e7a12a8),
binary `pcb-maker-nightly-2026-10-28-be0e62984304` (sha256
11b13f9842bc), tasks v7 (sha256 c8f95caf5089), `--quality`, at the
pinned work rates (the measured router rate had a median of 1.2e6, work
over wall 0.57), 10 jobs x 2 threads, `--order peak-desc`, through the
memory ledger with `--ledger-priority nightly`: 23264 s wall, largest
peak 4.7 GB, no row cut or killed in any stage. Per board:
`build/nightly/2026-10-28/summary.md`, `diff-sweep.txt` and `gap.txt`.

**Inputs.** Every board in both ran on the same input as in v22 (nightly
2026-10-27).

| | v22 (nightly 2026-10-27) | v23 (nightly 2026-10-28, be0e629) |
|---|---|---|
| pass, per-type rule | 57 of 109 | **59 of 109** |
| pass, strict rule (rule 4) | 57 of 109 | **59 of 109** |
| unconnected (all rows) | 2140 | 2924 |
| unconnected where both have a result on the same input (that run / this) | 2140 / **2018** (103 boards) | - |
| new DRC errors (per-type rule) | 61 | 49 |
| new DRC errors (strict rule) | 53 | 43 |
| vias (rows with a result) | 32910 (103) | 34477 (108) |
| length (rows with a result) | 786 m (103) | 822 m (108) |
| work s | 78901 (103) | 84669 (108) |
| CPU s (unpaired, a shared machine) | 211436 | 186412 |
| first-route open terminals | 4453 (103) | 4839 (108) |
| rows cut by the wall clock / killed | 0 / 0 | **0 / 0** |
| rows without a result | 6 | 1 |
| our vias in small SMD pads (score-kicad-board) | 3 on 2 of 103 (koeg-board-pcb, bumwings_v001_core) | 0 on all 108 |
| vias against the designer's (median) | x0.73 (103) | x0.72 (108) |
| length against the designer's (median) | x1.07 (103) | x1.06 (108) |
| open over connections (median, mean) | 0.000, 0.110 | 0.000, 0.119 |
| bends per connection against the designer's (median) | x1.15 | x1.16 |
| stair-steps, ours less the designer's (median) | +60 | +58 |
| pin-row crossings, ours less the designer's (median) | +1 | +1 |
| ground pour cut, ours less the designer's (median) | +0 mm (96; 1 lost) | +0 mm (101; 1 lost) |
| final ladder's share of the budget (median) | 0.680 (40 rows) | **0.637** (45 rows) |

**Against the nightly of 2026-10-27 (v22)** (compare.py, 109 boards in
both on the same input): better 6, same 93, worse 4; 81 of the 103 with
a result in both are identical in routed count, unconnected, vias,
length and strict findings; unconnected there 2140 -> 2018. Better, the
largest: OpenESC-30x30 4in1-panel 499 -> 353, eurorack-pmod-pcb 10 -> 5,
vaio_re 10 -> 7, koeg-board-pcb 15 -> 12, woagboard 4 -> 1,
poly_corne_split42_right 25 -> 24. Worse: Batch_1 268 -> 294, protoesp
73 -> 79, OpenRX-panel-rev2 191 -> 197, Sisu_SSE-9 18 -> 19. Strict
passes 57 -> 59: gained 2: Quanta75_RP2040Stamp_JLCPCBAoptimized
(internal error: memory: the process grew to 1474 MB resident, past its
reservati -> none), MokyaLora (clearance 4 -> none); lost 0.

**Rows without a result, 1** (the nightly of 2026-10-27: 6):
poly_kb_molecule_7x5_wave_left (1 parts without a legal place; the
previous night 6).

**Strict errors, 43 on 9 boards**: link (length_out_of_range 12),
protoesp (clearance 7), Sisu_SSE-9 (diff_pair_gap_out_of_range 5,
diff_pair_uncoupled_length_too_long 2), poly_kybd_split72_right
(clearance 5), USB_Keypad (connection_width 4), WLED_Matrix
(connection_width 3, track_dangling 1), sbc_allwinner_a13 module
(items_not_allowed 2), poly_kb_molecule_4x4 (starved_thermal 1),
poly_kb_molecule_4x5 (starved_thermal 1). No board fails the strict rule
with nothing open. Of the 50 that fail, 16 are within five connections
and five errors of clean, 22 leave more than 20 open, 1 have no result.

### Placement or routing on one binary (matched designer tier, nightly 2026-10-28)

`build/nightly/2026-10-28/designer.log`: 109 boards routed by the same
binary on the designer's placement at `--budget-scale 0.68` (the median
final-ladder share of the previous night's sweep, `nightly-report.py
--ladder-share`; 1020 s of work), `--quality`, 8 jobs x 1 thread,
`--order peak-desc`: 16065 s wall, largest peak 5.0 GB, no row cut or
killed, 109 with a result. **68 of 109 pass the per-type rule, 64 the
strict rule**; 3198 unconnected in all; strict errors 26; medians
against the designer's own board: vias x0.53, length x1.00, bends x0.87,
stair-steps +10.

`compare.py --gap build/nightly/2026-10-28/sweep.log
build/nightly/2026-10-28/designer.log --targets` over the **100 target
boards with both results** (of 100): **layout open 1463, designer-route
open 686: placement gap +777 (53 %), routing gap 686 (47 %)**; ours is
worse on 30, the same on 64, better on 6. Batch_1 dominates the routing
side (the designer route 322 open, ours 294); without it 1169 against
364: placement 69 %, routing 31 %. Over all 108 boards with both results
(the panels included): 2924 against 2699, placement 8 %, routing 92 %.
The previous night's split (2026-10-27, 99 target boards): 52 % / 48 %.

**The five boards whose placement costs most** (ours / designer route, open connections):

| Board | Connections | Layout open | Designer route open | Placement gap |
| --- | --- | --- | --- | --- |
| video | 371 | 190 | 1 | +189 |
| OpenESC-30x30 4in1 | 152 | 106 | 15 | +91 |
| glasgow revC3 | 226 | 143 | 55 | +88 |
| protoesp | 104 | 79 | 1 | +78 |
| 4in1-mini | 152 | 81 | 37 | +44 |

Next: 0xCB-1337 pcb-panel +43 (43 / 0), MegaDrive +39 (110 / 71),
poly_corne_split42_right +23 (24 / 1), SmartSpin2k_Panelized +21 (21 /
0), poly_kybd_split72_left +21 (21 / 0). Routing first (the designer
route leaves as much or more): Batch_1 322 / ours 294, Sisu_SSE-9 21 /
ours 19, vaio_re 11 / ours 7, MIDAS-MK1.1-revA 5 / ours 5,
reCamera_S101_v1.1 4 / ours 4, BAGEL-MK1.1-Avocado 2 / ours 1 and 6
more.

**Worse than the nightly of 2026-10-27, beyond noise** (the next triage;
a layout is deterministic for its binary, so every change is the
binary's; on the dense boards of `dense-noise.json` a loss counts only
when it exceeds the spread of the board's unconnected count over the
placer seeds there), 3: Batch_1 268 -> 294, OpenRX-panel-rev2 191 ->
197, Sisu_SSE-9 18 -> 19. Within the seed spread: protoesp 73 -> 79
(spread 33).

## Layout sweep v24 (2026-10-28)

<!-- nightly-sweep night=2026-10-28T1429 sweep=build/nightly/2026-10-28T1429/sweep.log main=c68e9a8f506f3b97f78c4b31677f968ea550d885 -->

Written by the nightly (`scripts/nightly-report.py --docs`) from
`build/nightly/2026-10-28T1429/`: main c68e9a8 (33 commits after v23's
main be0e629; 33 commits after the nightly of 2026-10-28's main
be0e629), binary `pcb-maker-nightly-2026-10-28-c68e9a8f506f` (sha256
60ae8b1c4196), tasks v7 (sha256 c8f95caf5089), `--quality`, at the
pinned work rates (the measured router rate had a median of 1.3e6, work
over wall 0.64), 10 jobs x 2 threads, `--order peak-desc`, through the
memory ledger with `--ledger-priority nightly`: 14327 s wall, largest
peak 4.5 GB, no row cut or killed in any stage. Per board:
`build/nightly/2026-10-28T1429/summary.md`, `diff-sweep.txt` and
`gap.txt`.

**Inputs.** Every board in both ran on the same input as in v23 (nightly
2026-10-28).

| | v23 (nightly 2026-10-28) | v24 (nightly 2026-10-28T1429, c68e9a8) |
|---|---|---|
| pass, per-type rule | 59 of 109 | **61 of 109** |
| pass, strict rule (rule 4) | 59 of 109 | **61 of 109** |
| unconnected (all rows) | 2924 | 2667 |
| unconnected where both have a result on the same input (that run / this) | 2720 / **2667** (107 boards) | - |
| new DRC errors (per-type rule) | 49 | 43 |
| new DRC errors (strict rule) | 43 | 40 |
| vias (rows with a result) | 34477 (108) | 31598 (107) |
| length (rows with a result) | 822 m (108) | 804 m (107) |
| work s | 84669 (108) | 81112 (107) |
| CPU s (unpaired, a shared machine) | 186412 | 158058 |
| first-route open terminals | 4839 (108) | 3400 (107) |
| rows cut by the wall clock / killed | 0 / 0 | **0 / 0** |
| rows without a result | 1 | 2 |
| our vias in small SMD pads (score-kicad-board) | 0 on all 108 | 0 on all 107 |
| vias against the designer's (median) | x0.72 (108) | x0.68 (107) |
| length against the designer's (median) | x1.06 (108) | x1.04 (107) |
| open over connections (median, mean) | 0.000, 0.119 | 0.000, 0.100 |
| bends per connection against the designer's (median) | x1.16 | x1.12 |
| stair-steps, ours less the designer's (median) | +58 | +56 |
| pin-row crossings, ours less the designer's (median) | +1 | +1 |
| ground pour cut, ours less the designer's (median) | +0 mm (101; 1 lost) | +0 mm (100; 1 lost) |
| final ladder's share of the budget (median) | 0.637 (45 rows) | **0.660** (39 rows) |

**Against the nightly of 2026-10-28 (v23)** (compare.py, 109 boards in
both on the same input): better 21, same 66, worse 20; 3 of the 107 with
a result in both are identical in routed count, unconnected, vias,
length and strict findings; unconnected there 2720 -> 2667. Better, the
largest: Batch_1 294 -> 156, OpenRX-panel-rev2 197 -> 85, OpenESC-30x30
4in1 106 -> 88, glasgow revC3 143 -> 127, poly_kybd_split72_left 21 ->
7, protoesp 79 -> 66, koeg-board-pcb 12 -> 0,
poly_kb_molecule_5x2_shifted 219 -> 208, DSP-ADAU1452 11 -> 3, vaio_re 7
-> 1 and 11 more. Worse: SmartSpin2k_Panelized 21 -> 115, SNSP-CPU-01 53
-> 111, poly_kb_molecule_4x4 244 -> 289, poly_kb_molecule_4x5 239 ->
272, 0xCB-1337 pcb-panel 43 -> 62, video 190 -> 202, OpenESC-30x30
4in1-panel 353 -> 364, 4in1-mini 81 -> 91, MegaDrive 110 -> 118,
poly_kybd_split72_right 21 -> 28 and 10 more. Strict passes 59 -> 61:
gained 6: ULK (4 open -> none), ULK_sl_PG1316s (2 open -> none),
BAGEL-MK1.1-Avocado (1 open -> none), jiran-ble-lite (1 open -> none),
OpenAirScope (1 open -> none), koeg-board-pcb (12 open -> none); lost 4:
framework_mobo_lefthalf (1 open), capybully (1 open), k30-SBC
(starved_thermal 1), ghoul (clearance 2).

**Rows without a result, 2** (the nightly of 2026-10-28: 1):
poly_kb_molecule_5x4_wave (1 parts without a legal place; the previous
night 204 open), poly_kb_molecule_7x5_wave_left (2 parts without a legal
place; the previous night 1).

**Strict errors, 40 on 12 boards**: link (length_out_of_range 12),
protoesp (clearance 3, solder_mask_bridge 3), vaio_re
(solder_mask_bridge 5), poly_corne_split42_right (clearance 3),
Sisu_SSE-9 (diff_pair_uncoupled_length_too_long 3), USB_Keypad
(connection_width 3), Batch_1 (starved_thermal 2), ghoul (clearance 2),
jetson-nano-baseboard (items_not_allowed 1), k30-SBC (starved_thermal
1), sbc_allwinner_a13 module (items_not_allowed 1), WLED_Matrix
(connection_width 1). Failing the strict rule with nothing open, 2:
ghoul (clearance 2), k30-SBC (starved_thermal 1). Of the 48 that fail,
17 are within five connections and five errors of clean, 21 leave more
than 20 open, 2 have no result.

### Placement or routing on one binary (matched designer tier, nightly 2026-10-28T1429)

`build/nightly/2026-10-28T1429/designer.log`: 109 boards routed by the
same binary on the designer's placement at `--budget-scale 0.637` (the
median final-ladder share of the previous night's sweep,
`nightly-report.py --ladder-share`; 955.5 s of work), `--quality`, 8
jobs x 1 thread, `--order peak-desc`: 10264 s wall, largest peak 5.0 GB,
no row cut or killed, 109 with a result. **70 of 109 pass the per-type
rule, 68 the strict rule**; 2804 unconnected in all; strict errors 25;
medians against the designer's own board: vias x0.50, length x1.00,
bends x0.86, stair-steps +6.

`compare.py --gap build/nightly/2026-10-28T1429/sweep.log
build/nightly/2026-10-28T1429/designer.log --targets` over the **100
target boards with both results** (of 100): **layout open 1444,
designer-route open 369: placement gap +1075 (74 %), routing gap 369
(26 %)**; ours is worse on 29, the same on 65, better on 6. Over all 107
boards with both results (the panels included): 2667 against 1831,
placement 31 %, routing 69 %. The previous night's split (2026-10-28,
100 target boards): 53 % / 47 %.

**The five boards whose placement costs most** (ours / designer route, open connections):

| Board | Connections | Layout open | Designer route open | Placement gap |
| --- | --- | --- | --- | --- |
| video | 371 | 202 | 1 | +201 |
| Batch_1 | 254 | 156 | 3 | +153 |
| SmartSpin2k_Panelized | 585 | 115 | 0 | +115 |
| OpenESC-30x30 4in1 | 152 | 88 | 15 | +73 |
| glasgow revC3 | 226 | 127 | 58 | +69 |

Next: SNSP-CPU-01 +69 (111 / 42), protoesp +65 (66 / 1), 0xCB-1337
pcb-panel +62 (62 / 0), 4in1-mini +54 (91 / 37), MegaDrive +47 (118 /
71). Routing first (the designer route leaves as much or more):
RP2040_base 36 / ours 35, Sisu_SSE-9 21 / ours 19, vaio_re 11 / ours 1,
MIDAS-MK1.1-revA 5 / ours 5, MIDAS-MK1-revA 5 / ours 5,
reCamera_S101_v1.1 4 / ours 4 and 5 more.

**Worse than the nightly of 2026-10-28, beyond noise** (the next triage;
a layout is deterministic for its binary, so every change is the
binary's; on the dense boards of `dense-noise.json` a loss counts only
when it exceeds the spread of the board's unconnected count over the
placer seeds there), 16: SmartSpin2k_Panelized 21 -> 115 (spread 74),
poly_kb_molecule_4x4 244 -> 289, poly_kb_molecule_4x5 239 -> 272,
0xCB-1337 pcb-panel 43 -> 62, OpenESC-30x30 4in1-panel 353 -> 364,
poly_kybd_split72_right 21 -> 28, link 4 -> 10, WLED_Matrix 7 -> 10,
ATAT1800 19 -> 22, framework_mobo_lefthalf 0 -> 1, jetson-nano-baseboard
1 -> 2, eurorack-pmod-pcb 5 -> 6, laptop power 13 -> 14, katia 39 -> 40,
capybully 0 -> 1, poly_corne_split42_left 16 -> 17. Losing the strict
pass: framework_mobo_lefthalf (1 open), capybully (1 open), k30-SBC
(starved_thermal 1), ghoul (clearance 2). Within the seed spread:
SNSP-CPU-01 53 -> 111 (spread 68), video 190 -> 202 (spread 193),
4in1-mini 81 -> 91 (spread 29), MegaDrive 110 -> 118 (spread 83).

## Layout sweep v25 (2026-10-29)

<!-- nightly-sweep night=2026-10-29 sweep=build/nightly/2026-10-29/sweep.log main=1f0ca50cccfb5087f3f8c701f5873e33dc2a637c -->

Written by the nightly (`scripts/nightly-report.py --docs`) from
`build/nightly/2026-10-29/`: main 1f0ca50 (46 commits after v24's main
c68e9a8; 46 commits after the nightly of 2026-10-28T1429's main
c68e9a8), binary `pcb-maker-nightly-2026-10-29-1f0ca50cccfb` (sha256
dfaceb121e3d), tasks v7 (sha256 c8f95caf5089), `--quality`, at the
pinned work rates (the measured router rate had a median of 2.5e6, work
over wall 1.02), 10 jobs x 2 threads, `--order peak-desc`, through the
memory ledger with `--ledger-priority nightly`: 9657 s wall, largest
peak 4.9 GB, no row cut or killed in any stage. Per board:
`build/nightly/2026-10-29/summary.md`, `diff-sweep.txt` and `gap.txt`.

**Inputs.** Every board in both ran on the same input as in v24 (nightly
2026-10-28T1429).

| | v24 (nightly 2026-10-28T1429) | v25 (nightly 2026-10-29, 1f0ca50) |
|---|---|---|
| pass, per-type rule | 61 of 109 | **62 of 109** |
| pass, strict rule (rule 4) | 61 of 109 | **62 of 109** |
| unconnected (all rows) | 2667 | 2658 |
| unconnected where both have a result on the same input (that run / this) | 2667 / **2658** (107 boards) | - |
| new DRC errors (per-type rule) | 43 | 43 |
| new DRC errors (strict rule) | 40 | 40 |
| vias (rows with a result) | 31598 (107) | 31682 (107) |
| length (rows with a result) | 804 m (107) | 804 m (107) |
| work s | 81112 (107) | 80370 (107) |
| CPU s (unpaired, a shared machine) | 158058 | 100692 |
| first-route open terminals | 3400 (107) | 3394 (107) |
| rows cut by the wall clock / killed | 0 / 0 | **0 / 0** |
| rows without a result | 2 | 2 |
| our vias in small SMD pads (score-kicad-board) | 0 on all 107 | 0 on all 107 |
| vias against the designer's (median) | x0.68 (107) | x0.68 (107) |
| length against the designer's (median) | x1.04 (107) | x1.04 (107) |
| open over connections (median, mean) | 0.000, 0.100 | 0.000, 0.100 |
| bends per connection against the designer's (median) | x1.12 | x1.12 |
| stair-steps, ours less the designer's (median) | +56 | +56 |
| pin-row crossings, ours less the designer's (median) | +1 | +1 |
| ground pour cut, ours less the designer's (median) | +0 mm (100; 1 lost) | +0 mm (100; 1 lost) |
| final ladder's share of the budget (median) | 0.660 (39 rows) | **0.675** (38 rows) |

**Against the nightly of 2026-10-28T1429 (v24)** (compare.py, 109 boards
in both on the same input): better 2, same 105, worse 0; 105 of the 107
with a result in both are identical in routed count, unconnected, vias,
length and strict findings; unconnected there 2667 -> 2658. Better, the
largest: MIDAS-MK1.1-revA 5 -> 0, MIDAS-MK1-revA 5 -> 1. Strict passes
61 -> 62: gained 1: MIDAS-MK1.1-revA (5 open -> none); lost 0.

**Rows without a result, 2** (the nightly of 2026-10-28T1429: 2):
poly_kb_molecule_5x4_wave (1 parts without a legal place; the previous
night 1), poly_kb_molecule_7x5_wave_left (2 parts without a legal place;
the previous night 2).

**Strict errors, 40 on 12 boards**: link (length_out_of_range 12),
protoesp (clearance 3, solder_mask_bridge 3), vaio_re
(solder_mask_bridge 5), poly_corne_split42_right (clearance 3),
Sisu_SSE-9 (diff_pair_uncoupled_length_too_long 3), USB_Keypad
(connection_width 3), Batch_1 (starved_thermal 2), ghoul (clearance 2),
jetson-nano-baseboard (items_not_allowed 1), k30-SBC (starved_thermal
1), sbc_allwinner_a13 module (items_not_allowed 1), WLED_Matrix
(connection_width 1). Failing the strict rule with nothing open, 2:
ghoul (clearance 2), k30-SBC (starved_thermal 1). Of the 47 that fail,
16 are within five connections and five errors of clean, 21 leave more
than 20 open, 2 have no result.

### Placement or routing on one binary (matched designer tier, nightly 2026-10-29)

`build/nightly/2026-10-29/designer.log`: 109 boards routed by the same
binary on the designer's placement at `--budget-scale 0.66` (the median
final-ladder share of the previous night's sweep, `nightly-report.py
--ladder-share`; 990 s of work), `--quality`, 8 jobs x 1 thread,
`--order peak-desc`: 7048 s wall, largest peak 7.1 GB, no row cut or
killed, 109 with a result. **71 of 109 pass the per-type rule, 68 the
strict rule**; 2795 unconnected in all; strict errors 25; medians
against the designer's own board: vias x0.53, length x1.00, bends x0.86,
stair-steps +6.

`compare.py --gap build/nightly/2026-10-29/sweep.log
build/nightly/2026-10-29/designer.log --targets` over the **100 target
boards with both results** (of 100): **layout open 1435, designer-route
open 360: placement gap +1075 (75 %), routing gap 360 (25 %)**; ours is
worse on 29, the same on 64, better on 7. Over all 107 boards with both
results (the panels included): 2658 against 1822, placement 31 %,
routing 69 %. The previous night's split (2026-10-28T1429, 100 target
boards): 74 % / 26 %.

**The five boards whose placement costs most** (ours / designer route, open connections):

| Board | Connections | Layout open | Designer route open | Placement gap |
| --- | --- | --- | --- | --- |
| video | 371 | 202 | 1 | +201 |
| Batch_1 | 254 | 156 | 3 | +153 |
| SmartSpin2k_Panelized | 585 | 115 | 0 | +115 |
| OpenESC-30x30 4in1 | 152 | 88 | 15 | +73 |
| glasgow revC3 | 226 | 127 | 55 | +72 |

Next: SNSP-CPU-01 +67 (111 / 44), protoesp +65 (66 / 1), 0xCB-1337
pcb-panel +62 (62 / 0), 4in1-mini +54 (91 / 37), MegaDrive +47 (118 /
71). Routing first (the designer route leaves as much or more):
RP2040_base 36 / ours 35, Sisu_SSE-9 21 / ours 19, vaio_re 11 / ours 1,
reCamera_S101_v1.1 4 / ours 4, BAGEL-MK1.1-Avocado 2 / ours 0,
jetson-nano-baseboard 2 / ours 2 and 5 more.

**Worse than the nightly of 2026-10-28T1429, beyond noise** (the next
triage; a layout is deterministic for its binary, so every change is the
binary's; on the dense boards of `dense-noise.json` a loss counts only
when it exceeds the spread of the board's unconnected count over the
placer seeds there), 0: none.

## Layout sweep v26 (2026-10-30)

<!-- nightly-sweep night=2026-10-30 sweep=build/nightly/2026-10-30/sweep.log main=288051a3e337df0e30a5420a0db4675bd5a3b0c1 -->

Written by the nightly (`scripts/nightly-report.py --docs`) from
`build/nightly/2026-10-30/`: main 288051a (1 commit after v25's main
1f0ca50; 1 commit after the nightly of 2026-10-29's main 1f0ca50),
binary `pcb-maker-nightly-2026-10-30-288051a3e337` (sha256
d84e05d9850b), tasks v7 (sha256 c8f95caf5089), `--quality`, at the
pinned work rates (the measured router rate had a median of 2.6e6, work
over wall 1.09), 10 jobs x 2 threads, `--order peak-desc`, through the
memory ledger with `--ledger-priority nightly`: 7894 s wall, largest
peak 4.9 GB, no row cut or killed in any stage. Per board:
`build/nightly/2026-10-30/summary.md`, `diff-sweep.txt` and `gap.txt`.

**Inputs.** Every board in both ran on the same input as in v25 (nightly
2026-10-29).

| | v25 (nightly 2026-10-29) | v26 (nightly 2026-10-30, 288051a) |
|---|---|---|
| pass, per-type rule | 62 of 109 | **62 of 109** |
| pass, strict rule (rule 4) | 62 of 109 | **62 of 109** |
| unconnected (all rows) | 2658 | 2658 |
| unconnected where both have a result on the same input (that run / this) | 2658 / **2658** (107 boards) | - |
| new DRC errors (per-type rule) | 43 | 43 |
| new DRC errors (strict rule) | 40 | 40 |
| vias (rows with a result) | 31682 (107) | 31682 (107) |
| length (rows with a result) | 804 m (107) | 804 m (107) |
| work s | 80370 (107) | 80370 (107) |
| CPU s (unpaired, a shared machine) | 100692 | 96365 |
| first-route open terminals | 3394 (107) | 3394 (107) |
| rows cut by the wall clock / killed | 0 / 0 | **0 / 0** |
| rows without a result | 2 | 2 |
| our vias in small SMD pads (score-kicad-board) | 0 on all 107 | 0 on all 107 |
| vias against the designer's (median) | x0.68 (107) | x0.68 (107) |
| length against the designer's (median) | x1.04 (107) | x1.04 (107) |
| open over connections (median, mean) | 0.000, 0.100 | 0.000, 0.100 |
| bends per connection against the designer's (median) | x1.12 | x1.12 |
| stair-steps, ours less the designer's (median) | +56 | +56 |
| pin-row crossings, ours less the designer's (median) | +1 | +1 |
| ground pour cut, ours less the designer's (median) | +0 mm (100; 1 lost) | +0 mm (100; 1 lost) |
| final ladder's share of the budget (median) | 0.675 (38 rows) | **0.675** (38 rows) |

**Against the nightly of 2026-10-29 (v25)** (compare.py, 109 boards in
both on the same input): better 0, same 107, worse 0; 107 of the 107
with a result in both are identical in routed count, unconnected, vias,
length and strict findings; unconnected there 2658 -> 2658. Strict
passes 62 -> 62: gained 0; lost 0.

**Rows without a result, 2** (the nightly of 2026-10-29: 2):
poly_kb_molecule_5x4_wave (1 parts without a legal place; the previous
night 1), poly_kb_molecule_7x5_wave_left (2 parts without a legal place;
the previous night 2).

**Strict errors, 40 on 12 boards**: link (length_out_of_range 12),
protoesp (clearance 3, solder_mask_bridge 3), vaio_re
(solder_mask_bridge 5), poly_corne_split42_right (clearance 3),
Sisu_SSE-9 (diff_pair_uncoupled_length_too_long 3), USB_Keypad
(connection_width 3), Batch_1 (starved_thermal 2), ghoul (clearance 2),
jetson-nano-baseboard (items_not_allowed 1), k30-SBC (starved_thermal
1), sbc_allwinner_a13 module (items_not_allowed 1), WLED_Matrix
(connection_width 1). Failing the strict rule with nothing open, 2:
ghoul (clearance 2), k30-SBC (starved_thermal 1). Of the 47 that fail,
16 are within five connections and five errors of clean, 21 leave more
than 20 open, 2 have no result.

### Placement or routing on one binary (matched designer tier, nightly 2026-10-30)

`build/nightly/2026-10-30/designer.log`: 109 boards routed by the same
binary on the designer's placement at `--budget-scale 0.675` (the median
final-ladder share of the previous night's sweep, `nightly-report.py
--ladder-share`; 1012.5 s of work), `--quality`, 8 jobs x 1 thread,
`--order peak-desc`: 5884 s wall, largest peak 7.1 GB, no row cut or
killed, 109 with a result. **71 of 109 pass the per-type rule, 68 the
strict rule**; 2759 unconnected in all; strict errors 25; medians
against the designer's own board: vias x0.53, length x1.00, bends x0.86,
stair-steps +6.

`compare.py --gap build/nightly/2026-10-30/sweep.log
build/nightly/2026-10-30/designer.log --targets` over the **100 target
boards with both results** (of 100): **layout open 1435, designer-route
open 351: placement gap +1084 (76 %), routing gap 351 (24 %)**; ours is
worse on 29, the same on 64, better on 7. Over all 107 boards with both
results (the panels included): 2658 against 1786, placement 33 %,
routing 67 %. The previous night's split (2026-10-29, 100 target
boards): 75 % / 25 %.

**The five boards whose placement costs most** (ours / designer route, open connections):

| Board | Connections | Layout open | Designer route open | Placement gap |
| --- | --- | --- | --- | --- |
| video | 371 | 202 | 1 | +201 |
| Batch_1 | 254 | 156 | 3 | +153 |
| SmartSpin2k_Panelized | 585 | 115 | 0 | +115 |
| SNSP-CPU-01 | 363 | 111 | 35 | +76 |
| OpenESC-30x30 4in1 | 152 | 88 | 15 | +73 |

Next: glasgow revC3 +72 (127 / 55), protoesp +65 (66 / 1), 0xCB-1337
pcb-panel +62 (62 / 0), 4in1-mini +54 (91 / 37), MegaDrive +47 (118 /
71). Routing first (the designer route leaves as much or more):
RP2040_base 36 / ours 35, Sisu_SSE-9 21 / ours 19, vaio_re 11 / ours 1,
reCamera_S101_v1.1 4 / ours 4, BAGEL-MK1.1-Avocado 2 / ours 0,
jetson-nano-baseboard 2 / ours 2 and 5 more.

**Worse than the nightly of 2026-10-29, beyond noise** (the next triage;
a layout is deterministic for its binary, so every change is the
binary's; on the dense boards of `dense-noise.json` a loss counts only
when it exceeds the spread of the board's unconnected count over the
placer seeds there), 0: none.

## Layout sweep v27 (2026-10-31)

<!-- nightly-sweep night=2026-10-31 sweep=build/nightly/2026-10-31/sweep.log main=56e15e9be4310b24d5d457e02b2ed35f6dc7ad14 -->

Written by the nightly (`scripts/nightly-report.py --docs`) from
`build/nightly/2026-10-31/`: main 56e15e9 (1 commit after v26's main
288051a; 1 commit after the nightly of 2026-10-30's main 288051a),
binary `pcb-maker-nightly-2026-10-31-56e15e9be431` (sha256
293039bfd681), tasks v7 (sha256 c8f95caf5089), `--quality`, at the
pinned work rates (the measured router rate had a median of 2.6e6, work
over wall 1.06), 10 jobs x 2 threads, `--order peak-desc`, through the
memory ledger with `--ledger-priority nightly`: 7910 s wall, largest
peak 4.9 GB, no row cut or killed in any stage. Per board:
`build/nightly/2026-10-31/summary.md`, `diff-sweep.txt` and `gap.txt`.

**Inputs.** Every board in both ran on the same input as in v26 (nightly
2026-10-30).

| | v26 (nightly 2026-10-30) | v27 (nightly 2026-10-31, 56e15e9) |
|---|---|---|
| pass, per-type rule | 62 of 109 | **62 of 109** |
| pass, strict rule (rule 4) | 62 of 109 | **62 of 109** |
| unconnected (all rows) | 2658 | 2658 |
| unconnected where both have a result on the same input (that run / this) | 2658 / **2658** (107 boards) | - |
| new DRC errors (per-type rule) | 43 | 43 |
| new DRC errors (strict rule) | 40 | 40 |
| vias (rows with a result) | 31682 (107) | 31682 (107) |
| length (rows with a result) | 804 m (107) | 804 m (107) |
| work s | 80370 (107) | 80370 (107) |
| CPU s (unpaired, a shared machine) | 96365 | 96576 |
| first-route open terminals | 3394 (107) | 3394 (107) |
| rows cut by the wall clock / killed | 0 / 0 | **0 / 0** |
| rows without a result | 2 | 2 |
| our vias in small SMD pads (score-kicad-board) | 0 on all 107 | 0 on all 107 |
| vias against the designer's (median) | x0.68 (107) | x0.68 (107) |
| length against the designer's (median) | x1.04 (107) | x1.04 (107) |
| open over connections (median, mean) | 0.000, 0.100 | 0.000, 0.100 |
| bends per connection against the designer's (median) | x1.12 | x1.12 |
| stair-steps, ours less the designer's (median) | +56 | +56 |
| pin-row crossings, ours less the designer's (median) | +1 | +1 |
| ground pour cut, ours less the designer's (median) | +0 mm (100; 1 lost) | +0 mm (100; 1 lost) |
| final ladder's share of the budget (median) | 0.675 (38 rows) | **0.675** (38 rows) |

**Against the nightly of 2026-10-30 (v26)** (compare.py, 109 boards in
both on the same input): better 0, same 107, worse 0; 107 of the 107
with a result in both are identical in routed count, unconnected, vias,
length and strict findings; unconnected there 2658 -> 2658. Strict
passes 62 -> 62: gained 0; lost 0.

**Rows without a result, 2** (the nightly of 2026-10-30: 2):
poly_kb_molecule_5x4_wave (1 parts without a legal place; the previous
night 1), poly_kb_molecule_7x5_wave_left (2 parts without a legal place;
the previous night 2).

**Strict errors, 40 on 12 boards**: link (length_out_of_range 12),
protoesp (clearance 3, solder_mask_bridge 3), vaio_re
(solder_mask_bridge 5), poly_corne_split42_right (clearance 3),
Sisu_SSE-9 (diff_pair_uncoupled_length_too_long 3), USB_Keypad
(connection_width 3), Batch_1 (starved_thermal 2), ghoul (clearance 2),
jetson-nano-baseboard (items_not_allowed 1), k30-SBC (starved_thermal
1), sbc_allwinner_a13 module (items_not_allowed 1), WLED_Matrix
(connection_width 1). Failing the strict rule with nothing open, 2:
ghoul (clearance 2), k30-SBC (starved_thermal 1). Of the 47 that fail,
16 are within five connections and five errors of clean, 21 leave more
than 20 open, 2 have no result.

### Placement or routing on one binary (matched designer tier, nightly 2026-10-31)

`build/nightly/2026-10-31/designer.log`: 109 boards routed by the same
binary on the designer's placement at `--budget-scale 0.675` (the median
final-ladder share of the previous night's sweep, `nightly-report.py
--ladder-share`; 1012.5 s of work), `--quality`, 8 jobs x 1 thread,
`--order peak-desc`: 5710 s wall, largest peak 7.2 GB, no row cut or
killed, 109 with a result. **71 of 109 pass the per-type rule, 68 the
strict rule**; 2759 unconnected in all; strict errors 25; medians
against the designer's own board: vias x0.53, length x1.00, bends x0.86,
stair-steps +6.

`compare.py --gap build/nightly/2026-10-31/sweep.log
build/nightly/2026-10-31/designer.log --targets` over the **100 target
boards with both results** (of 100): **layout open 1435, designer-route
open 351: placement gap +1084 (76 %), routing gap 351 (24 %)**; ours is
worse on 29, the same on 64, better on 7. Over all 107 boards with both
results (the panels included): 2658 against 1786, placement 33 %,
routing 67 %. The previous night's split (2026-10-30, 100 target
boards): 76 % / 24 %.

**The five boards whose placement costs most** (ours / designer route, open connections):

| Board | Connections | Layout open | Designer route open | Placement gap |
| --- | --- | --- | --- | --- |
| video | 371 | 202 | 1 | +201 |
| Batch_1 | 254 | 156 | 3 | +153 |
| SmartSpin2k_Panelized | 585 | 115 | 0 | +115 |
| SNSP-CPU-01 | 363 | 111 | 35 | +76 |
| OpenESC-30x30 4in1 | 152 | 88 | 15 | +73 |

Next: glasgow revC3 +72 (127 / 55), protoesp +65 (66 / 1), 0xCB-1337
pcb-panel +62 (62 / 0), 4in1-mini +54 (91 / 37), MegaDrive +47 (118 /
71). Routing first (the designer route leaves as much or more):
RP2040_base 36 / ours 35, Sisu_SSE-9 21 / ours 19, vaio_re 11 / ours 1,
reCamera_S101_v1.1 4 / ours 4, BAGEL-MK1.1-Avocado 2 / ours 0,
jetson-nano-baseboard 2 / ours 2 and 5 more.

**Worse than the nightly of 2026-10-30, beyond noise** (the next triage;
a layout is deterministic for its binary, so every change is the
binary's; on the dense boards of `dense-noise.json` a loss counts only
when it exceeds the spread of the board's unconnected count over the
placer seeds there), 0: none.

## Expansion intake: 5 834 boards routed on the designer's placement (2026-10-18)

`benchmarks/expansion/` (README there) harvested 5 834 native KiCad boards
from 1 146 repositories (plus 1 225 Eagle and Altium conversions, kept
apart) and routed each on the designer's placement with tracks, vias and
small zones stripped, at a 30-second ladder budget on the be4b012 binary,
one search thread at nice 19 on a loaded machine; 2 635 of them again at
300 s; 2 168 placement variants with features for placement data. The
analysis tables are in `benchmarks/expansion/analysis/` (the raw data,
54 GB, stays on disk under `data/`).

| | boards | share |
|---|---|---|
| native-clean within 30 s of routing | 2 309 | 39.6 % |
| native-clean at any time on the quick run | 2 626 | 45.0 % |
| designer-relative pass (no finding type above the designer's, nothing unconnected) | 3 896 | 66.8 % |
| complete by the router's count | 4 367 | 74.9 % |
| complete at 300 s | 4 528 | 77.6 % |
| boards with a native-clean source and their own rules: clean within 30 s | 71.7 % of 1 995 | |

Median routing time of the clean boards 3.4 s (p90 34.5 s). Caveats: 59 %
of the designer boards have DRC findings of their own; 6.9 % of boards use
inferred rules; 75 % are 2-layer; resource limits (199, a 3 GiB cap the
ladder's memory sizing cannot see) and wall cuts (279, overrun work beyond
the ladder budget) are harness and sizing effects, not routing failures
(since 2026-10-21 the ladder sizes its routers by the process's limits and
keeps its work within its budget: docs/changelog.md, "ladder memory and
work bound").

What blocks boards at scale, ranked by boards a single new finding type
blocks: KiCad disagreeing with the router's "complete" (119: stacked
same-number F/B pads, tracks ending at a pad on the other side, pour to pad
after refill; 192 boards complete by the router's count with KiCad
unconnected items in all, classified and fixed on 2026-10-21: docs/changelog.md,
"KiCad connectivity agreement"), starved thermals (103), track_width (82, almost all inferred
rules against KiCad's 0.2 mm default), clearance from `.kicad_dru` and
netclass rules (16), copper edge clearance (15), mask bridges (8). 1 297 of
the 1 725 "DRC failures" fail only on the designer's own findings. Of the
63 model errors, 9 are ours (degenerate polygons, `gr_bbox` pads, a missing
roundrect ratio, `/A15` vs `A15` net identity, one unexplained "no routable
connections"); 54 are unloadable or netless sources. More budget rarely
helps: of 1 258 boards incomplete at 30 s with a 300 s run, 161 became
complete. A 46-board "expansion" tier spanning the hardness axis is
proposed in `analysis/expansion-tier.json`.

The tier is built (2026-10-21): `run.py --tier expansion` routes the 46
boards on the designer's placement at 30 s of work (`--budget-scale 10`
for 300 s), fetching each at its pinned commit with SHA-256 checks and a
licence rule (benchmarks/agent-tasks/README.md, "The expansion tier").
x1211, 8 jobs x 1 thread: 14/46 pass, 45 of 46 boards in the intake's
outcome class, every stripped board identical and every route the wall
guard cut in neither run identical (32 boards); 13 rows are cut by the
wall guard at that load.

## KiCadRoutingTools against x1367 (2026-10-21)

**Complete: all 109 boards judged** (measured 2026-10-21 to 2026-10-23),
and the ten-board timing with ours at 1 and 16 threads. Our side is
pcb-maker-x1367 throughout (`build/fr2-route-final.log`, the rows of the
Freerouting refresh above); main has moved on since (x1573 and later, see
[changelog.md](changelog.md)), so these are not main's numbers.

[KiCadRoutingTools](https://github.com/drandyhaas/KiCadRoutingTools) (MIT,
commit cddf4cd of 2026-10-21, version 0.23.1) is the autorouter a Hacker
News commenter found "routed more sensibly" than Freerouting: Python around
a Rust A* core, an octilinear 0.1 mm grid on every copper layer, rip-up and
reroute, plane pours, fanout, a CLI and a KiCad plugin. It ran headless
through its CLI router (`py_router/route.py <board> <result> --nets '*'
--no-fix-drc-settings --escalation board`, through
`benchmarks/github/kicadroutingtools.py`; setup in
[benchmarks/github/README.md](../benchmarks/github/README.md#kicadroutingtools-kicadroutingtoolspy)).
It routed the placements of the Freerouting refresh
(`build/github-layout-v17/<task>/layout/placed`, the 109 tasks of v7) with
the designer's pours kept; route.py treats them as planes.

Both results are judged alike:
- KiCad's DRC through `drc-kicad-board`, with the zones refilled.
- The pass: unconnected items and copper errors beyond the designer's, per
  type.
- The strict rule.

Two flags keep the judging ours. By default route.py loosens the written
project's Board Setup floors to whatever it routed, and lets a failing net
drop below the board's minimums to the fab tier's. The result is judged
with the placement's project files; route.py rewrote none of them.

route.py has no time limit and no thread setting. Its search runs on one
thread: the Rust core has no threads, and the BLAS pools were pinned to
one. A killed run leaves no board unless it had already written one: it
writes the board after its rescue pass, before its plane finalize and
reconciliation. So each board ran to completion or to a cap, and each row
records its wall time next to ours. The cap:
- The 32 largest boards ran first, eight at a time, with three, later two,
  times our wall, at most 3600 s (`build/krt-route-x1367`, `-b`).
- The other 67 boards: our wall, at least 120 s, two routers at a time
  (`build/krt-route-x1367-c`; their KiCad checks waited for the tier lock
  and finished on 2026-10-22).
- The ten timing boards ran to completion, at most 3600 s
  (`build/krt-timing`); their rows replace the capped ones in the table.

Our walls were measured at a load of 12 to 50 (8 x 2 beside other agents);
its runs saw a load of 2 to 46 (at the start of a board), most of the capped ones 2 to 15, which
favours it on "within our wall".

**Summary** (`benchmarks/github/kicadroutingtools_compare.py
build/krt-route-x1367 build/krt-route-x1367-b build/krt-route-x1367-c
build/krt-timing --ours build/fr2-route-final.log`):

| | pcb-maker x1367 | KiCadRoutingTools cddf4cd |
| --- | ---: | ---: |
| boards with a KiCad-checked result (of 109) | 107 | 66 (36 finished, 30 cut after writing its board) |
| cut before writing a board | | 43 |
| pass (designer-subtracted per type) | 51 | 7 (3 within our wall) |
| strict pass | 44 | 5 (1 within our wall) |
| better on KiCad's unconnected, then errors | 93 | 6 (10 ties) |
| 66 boards both have: KiCad unconnected | 1 778 | 3 296 |
| same: copper errors (pass rule) | 69 | 3 661 |
| same: vias | 10 985 | 25 200 |
| same: track length | 320.2 m | 319.1 m |
| finished within our wall | | 24 |
| wall over ours, all 109, a cut run at its cap (a lower bound) | | median x1.00, at least ours on 85 |
| CPU / wall over ours where it finished (36) | | median x0.60 / x0.75 |

Every board it passes ours passes too, under the strict rule as well:
spell_tome_bottom, Blue_Line, urchin, ligra_back, bumwings_v001_core,
bumwings_v001_xiao and CyberKeeb2040 (its strict passes: not ligra_back
and bumwings_v001_core). It is faster on three of them (bumwings_v001_core
164 s against our 592, bumwings_v001_xiao 268 against 556, ligra_back 30 against 39) and
slower on four (CyberKeeb2040 1870 s against 351). The 36 boards it
finishes are the ones it finds easy: there its median CPU is 0.60 times
ours, because ours spends its budget on the boards it cannot close and
KiCadRoutingTools stops when its passes are done, complete or not (29 of
the 36 do not pass). Its copper errors are mostly hole_clearance (1 037),
solder_mask_bridge (919), annular_width (392), via_diameter (367),
starved_thermal (240), copper_edge_clearance (232) and drill_out_of_range
(225): vias at or below the board's floors, often in a pad. Peak memory
was 0.2 to 5.3 GB a board.

**Where KiCadRoutingTools is better** (fewer KiCad-unconnected items,
then errors; it passes none of the six), with what KiCad reports on ours:
- **d20_pcb**: KiCadRoutingTools 18/18 nets, KiCad 0 unconnected but 199
  solder_mask_bridge, in 193 s. Ours 18/19, 1 unconnected, no error, in
  294 s.
- **Business-Cards WLED_Matrix**: KiCadRoutingTools 10 unconnected with 14
  connection_width, 73 hole_clearance, 2 copper_edge_clearance and 4
  dangling items, in 268 s. Ours 11 unconnected and 2
  copper_edge_clearance, in 583 s.
- **jiran-ble-lite**: KiCadRoutingTools 1 unconnected (the designer's own
  count), no error, in 141 s. Ours 20 unconnected (the far half's GND
  pads), no error, in 1319 s; main since x1434 routes it to the
  designer's 1.
- **0xCB-1337 panel**: KiCadRoutingTools 50 unconnected, 84
  solder_mask_bridge and 17 starved_thermal, cut at 3600 s. Ours 149/188,
  65 unconnected, no error, in 2355 s.
- **Neotron-Pico**: KiCadRoutingTools 190/190 nets, KiCad 0 unconnected,
  2 annular_width, 2 hole_clearance and 1 hole_to_hole (its vias at 0.25
  mm, the board floor), 727 vias, in 971 s. Ours 176/190, 24 unconnected,
  no error, 212 vias, in 2427 s (1177 s of work at a load of 25 to 49).
- **glasgow revC3**: KiCadRoutingTools 163 unconnected, cut at 2650 s
  with its board written. Ours 124/226, 180 unconnected, no error, in
  883 s.

**Why we lost there, and main** (2026-10-23, docs/changelog.md, "the
boards other routers beat us on"; route mode, main x1647 and the
branch's x1714):
- d20_pcb: the open connection was the touch electrode J2 (one custom
  pad, the die's face, enclosed by its footprint's copper frame on F.Cu
  and a keepout on B.Cu, under the footprint's mask opening): no via was
  allowed in it, neither as an SMD pad nor under the mask opening.
  x1714 lets a touch electrode hold its net's vias, and its own opening
  leave them there: 19/19, KiCad 0, strict pass, in 12.7 s of work.
- WLED_Matrix: main x1647 has KiCad 6 unconnected (KiCadRoutingTools 10)
  and the same 2 copper_edge_clearance, AE1's antenna copper over the
  outline: the v17 placement's, which every router gets (theirs has the
  same two). The placer now counts a footprint's copper graphics in its
  body (x1712 in layout mode: no copper_edge_clearance, KiCad 6).
- 0xCB panel, Neotron-Pico, glasgow: not re-measured here.

Per board (`build/krt-compare.txt`; "nets" is KiCadRoutingTools' own
count; a cut row says how far its log got):

| board | conn. | ours: unconn. / errors / strict | KRT: nets / unconn. / errors / strict | wall s ours / KRT | CPU s ours / KRT |
|---|---:|---|---|---:|---:|
| katia/katia | ? | timed out | cut at 3600 s (main loop 162/162 nets, 82 failed) | 2701 / 3600 | 2805 / 2518 |
| spell_tome/spell_tome_bottom | 18 | 0 / 0 / pass | 18/18 / 0 / 0 / pass | 8 / 28 | 8 / 45 |
| OpenMuscle-FlexGrid/OM-FlexGrid-Flex | 19 | 0 / 0 / pass | cut at 281 s (before the main loop) | 281 / 281 | 726 / 280 |
| d20_pcb/d20_pcb | 19 | 1 / 0 / fail | 18/18 / 0 / 199 / fail | 294 / 193 | 502 / 213 |
| OnBoard/E-Fidget-Lite | 20 | 0 / 0 / pass | 17/19 / 2 / 0 / fail | 27 / 32 | 26 / 52 |
| OnBoard/MotionCubeViewAllForces | 20 | 0 / 0 / pass | 18/20 / 2 / 0 / fail | 83 / 44 | 131 / 62 |
| OpenMuscle-FlexGrid/OM-60-Flex | 21 | 0 / 0 / pass | cut at 121 s (before the main loop) | 121 / 121 | 148 / 120 |
| CharlieBoard/Blue_Line | 26 | 0 / 0 / pass | 26/26 / 0 / 0 / pass | 11 / 60 | 11 / 76 |
| PolyKybd/poly_kb_molecule_4x4 | 28 | 499 / 16 / fail | ? / 499 / 98 / fail (cut at 3552 s, board written) | 1300 / 3552 | 2407 / 3383 |
| PolyKybd/poly_kb_molecule_4x5 | 28 | 499 / 101 / fail | cut at 984 s (before the main loop) | 984 / 985 | 2523 / 979 |
| PolyKybd/poly_kb_molecule_5x2_shifted | 28 | 409 / 32 / fail | ? / 447 / 81 / fail (cut at 1465 s, board written) | 1465 / 1465 | 1858 / 1516 |
| PolyKybd/poly_kb_molecule_5x4_wave | 28 | 499 / 0 / fail | cut at 643 s (main loop 27/27 nets, 2 failed) | 643 / 644 | 934 / 639 |
| PolyKybd/poly_kb_molecule_7x5_wave_left | 28 | 499 / 0 / fail | cut at 927 s (main loop 27/27 nets, 1 failed) | 927 / 927 | 1300 / 921 |
| capybully_keyboard/capybully | 32 | 0 / 0 / pass | ? / 18 / 0 / fail (cut at 730 s, board written) | 730 / 730 | 811 / 751 |
| ligra/ligra_back | 33 | 0 / 0 / pass | 32/32 / 0 / 0 / fail | 39 / 30 | 30 / 48 |
| Business-Cards/USB_Keypad | 39 | 2 / 0 / fail | ? / 6 / 77 / fail (cut at 3600 s, board written) | 381 / 3600 | 670 / 3607 |
| OnBoard/krishveercard | 43 | 0 / 0 / fail | 32/43 / 36 / 0 / fail | 617 / 389 | 1659 / 405 |
| 0xCB-1337/1337-v4.0 | 47 | 0 / 0 / pass | 43/46 / 4 / 22 / fail | 100 / 77 | 100 / 131 |
| Business-Cards/PCB | 54 | 0 / 0 / pass | ? / 2 / 307 / fail (cut at 122 s, board written) | 122 / 122 | 122 / 126 |
| OSHW-reCamera-Series/reCamera_S101_v1.1 | 56 | 4 / 0 / fail | 51/56 / 6 / 39 / fail (cut at 120 s, board written) | 116 / 120 | 310 / 153 |
| ergo-snm-keyboard/ErgoSNM_keyboard | 58 | 0 / 0 / pass | ? / 11 / 7 / fail (cut at 3600 s, board written) | 270 / 3600 | 245 / 3568 |
| Qfwfq/qfwfq | 64 | 0 / 0 / fail | cut at 290 s (main loop 63/63 nets, 4 failed; rescue pass done, 4 recovered) | 290 / 290 | 297 / 298 |
| little-red-rover/little_red_rover | 64 | 0 / 0 / pass | ? / 6 / 0 / fail (cut at 318 s, board written) | 318 / 318 | 554 / 353 |
| eternal-keypad/eternal-keypad | 65 | 0 / 0 / fail | 63/63 / 2 / 1 / fail | 96 / 1037 | 83 / 1082 |
| urchin/main | 68 | 0 / 0 / pass | 68/68 / 0 / 0 / pass | 21 / 90 | 24 / 92 |
| Sumec-MiniSumo/SUMEC_MK_IV | 69 | 5 / 0 / fail | 62/69 / 15 / 77 / fail | 974 / 1830 | 1903 / 1496 |
| ULK/ULK | 70 | 2 / 0 / fail | 67/69 / 2 / 3 / fail | 1668 / 634 | 2022 / 679 |
| ULK/ULK_sl_PG1316s | 70 | 3 / 0 / fail | 66/69 / 3 / 0 / fail | 1784 / 1255 | 2382 / 1198 |
| bumwings-kbd/bumwings_v001R55_rp2040zero_sd | 70 | 0 / 0 / pass | 70/70 / 0 / 1 / fail | 85 / 111 | 85 / 132 |
| MyKiCad/framework_mobo_lefthalf | 71 | 0 / 0 / pass | cut at 266 s (main loop 71/71 nets, 7 failed) | 266 / 266 | 704 / 264 |
| bumwings-kbd/bumwings_v001 | 72 | 0 / 0 / pass | 72/72 / 1 / 0 / fail | 156 / 90 | 156 / 111 |
| 0xCB-Static/0xcb-static | 74 | 0 / 0 / pass | cut at 120 s (before the main loop) | 31 / 120 | 31 / 120 |
| le_chiffre_keyboard_stm32/stm32_hotswap_chi… | 75 | 0 / 0 / pass | ? / 4 / 0 / fail (cut at 454 s, board written) | 454 / 454 | 486 / 520 |
| bumwings-kbd/bumwings_v001R55_xiao_sd | 75 | 0 / 0 / pass | cut at 164 s (before the main loop) | 164 / 164 | 164 / 163 |
| slab-pcb/slab-pcb-left | 76 | 1 / 0 / fail | 68/78 / 18 / 8 / fail | 1840 / 2476 | 2229 / 2381 |
| le_chiffre_keyboard_stm32/stm32_chiffre_36k… | 76 | 0 / 0 / pass | ? / 9 / 0 / fail (cut at 417 s, board written) | 417 / 417 | 459 / 462 |
| bumwings-kbd/bumwings_v001_xiao_s | 78 | 0 / 0 / pass | cut at 131 s (main loop 78/78 nets, 2 failed) | 131 / 131 | 138 / 130 |
| bumwings-kbd/bumwings_v001R64_rp2040zero_sd | 81 | 0 / 0 / pass | cut at 120 s (before the main loop) | 100 / 120 | 90 / 120 |
| bumwings-kbd/bumwings_v001_xiao | 81 | 0 / 0 / pass | 81/81 / 0 / 0 / pass | 556 / 268 | 437 / 290 |
| OpenFC-Lite/OpenFC | 82 | 0 / 0 / pass | 76/82 / 7 / 40 / fail | 280 / 147 | 841 / 170 |
| slab-pcb/interchange-pcb-right | 83 | 0 / 0 / fail | ? / 91 / 0 / fail (cut at 983 s, board written) | 983 / 983 | 796 / 1010 |
| 0xCB-1337/pcb | 85 | 0 / 0 / pass | ? / 2 / 8 / fail (cut at 405 s, board written) | 405 / 405 | 729 / 449 |
| bumwings-kbd/bumwings_v001R64_nano_sd | 85 | 0 / 0 / pass | cut at 120 s (before the main loop) | 72 / 120 | 75 / 120 |
| bumwings-kbd/bumwings_v001R64_xiao_sd | 89 | 0 / 0 / pass | cut at 127 s (before the main loop) | 127 / 127 | 128 / 127 |
| OnBoard/woagboard | 91 | 9 / 0 / fail | 80/91 / 15 / 4 / fail | 1411 / 476 | 2711 / 520 |
| mackerel-68k/mackerel-08-v1 | 96 | 0 / 0 / pass | 96/96 / 0 / 92 / fail | 467 / 339 | 587 / 359 |
| bumwings-kbd/bumwings_v001_core | 98 | 0 / 0 / pass | 98/98 / 0 / 0 / fail | 592 / 164 | 707 / 190 |
| OnBoard/keyboar_ | 102 | 0 / 0 / fail | 102/103 / 2 / 2 / fail | 540 / 162 | 359 / 208 |
| osprey/osprey_rev_a | 103 | 3 / 1 / fail | 90/103 / 18 / 3 / fail | 2165 / 1547 | 2281 / 1464 |
| Business-Cards/WLED_Matrix | 104 | 11 / 2 / fail | 98/104 / 10 / 93 / fail | 583 / 268 | 1046 / 378 |
| MyKiCad/protoesp | 104 | 58 / 1 / fail | ? / 79 / 4 / fail (cut at 1180 s, board written) | 1180 / 1180 | 1805 / 1184 |
| jiran-ble-lite/jiran-ble-lite | 105 | 20 / 0 / fail | 104/105 / 1 / 0 / fail | 1319 / 141 | 1357 / 148 |
| Telemetry/TelemetryOnboard | 108 | 0 / 0 / fail | ? / 16 / 6 / fail (cut at 365 s, board written) | 365 / 365 | 960 / 428 |
| silkscreen-pcb/silkscreen_pcb | 113 | 0 / 0 / pass | cut at 149 s (main loop 112/112 nets, 13 failed) | 149 / 149 | 150 / 148 |
| laptop/power | 115 | 5 / 0 / fail | 107/114 / 15 / 260 / fail | 328 / 326 | 885 / 377 |
| CyberKeeb2040/MainBoard | 118 | 0 / 0 / pass | 118/118 / 0 / 0 / pass | 351 / 1870 | 378 / 1795 |
| Castor_and_Pollux/mainboard | 126 | 0 / 0 / pass | 120/126 / 6 / 60 / fail | 568 / 417 | 804 / 479 |
| OpenRX/OpenRX-panel-rev2 | 127 | 229 / 0 / fail | ? / 388 / 537 / fail (cut at 520 s, board written) | 520 / 520 | 906 / 520 |
| eurorack-pmod/eurorack-pmod-pcb | 133 | 4 / 0 / fail | ? / 15 / 251 / fail (cut at 465 s, board written) | 465 / 465 | 1163 / 528 |
| laptop/keyboard | 136 | 4 / 0 / fail | 123/136 / 14 / 2 / fail | 675 / 643 | 1198 / 661 |
| ISS-PCB/BAGEL-MK1 | 138 | 0 / 0 / pass | 124/138 / 22 / 197 / fail | 175 / 1021 | 232 / 1163 |
| ISS-PCB/BAGEL-MK1.1-Avocado | 144 | 2 / 0 / fail | ? / 55 / 55 / fail (cut at 559 s, board written) | 559 / 559 | 1296 / 576 |
| rosco_m68k/kicad | 148 | 0 / 0 / pass | cut at 597 s (main loop 148/148 nets, 10 failed) | 597 / 597 | 728 / 594 |
| keyboards/ghoul | 148 | 6 / 1 / fail | ? / 83 / 23 / fail (cut at 3600 s, board written) | 2109 / 3601 | 2741 / 3588 |
| explorer/Explorer | 150 | 2 / 0 / fail | cut at 381 s (main loop 150/150 nets, 18 failed) | 381 / 381 | 637 / 378 |
| OpenESC-20x20/4in1-mini | 152 | 43 / 0 / fail | 118/152 / 57 / 105 / fail | 508 / 335 | 1197 / 371 |
| OpenESC-30x30/4in1 | 152 | 111 / 0 / fail | cut at 408 s (main loop 152/152 nets, 14 failed; rescue pass done, 3 recovered) | 408 / 408 | 702 / 404 |
| ISS-PCB/MIDAS-MK2.1-revA | 154 | 0 / 0 / pass | cut at 602 s (main loop 153/153 nets, 10 failed) | 602 / 603 | 1195 / 598 |
| mackerel-68k/mackerel-10-v1 | 155 | 0 / 0 / pass | ? / 2 / 0 / fail (cut at 662 s, board written) | 662 / 662 | 921 / 692 |
| ISS-PCB/MIDAS-MK1.1-revA | 156 | 5 / 0 / fail | 149/156 / 8 / 0 / fail | 599 / 598 | 1430 / 675 |
| ISS-PCB/MIDAS-MK2 | 156 | 0 / 0 / pass | 125/156 / 53 / 23 / fail | 243 / 1044 | 300 / 1122 |
| ISS-PCB/MIDAS-MK1-revA | 159 | 8 / 9 / fail | 143/159 / 22 / 9 / fail | 670 / 592 | 1281 / 642 |
| PCB-Modular-Multi-Protocol-Hub/Hub | 159 | 22 / 0 / fail | ? / 30 / 21 / fail (cut at 2914 s, board written) | 971 / 2914 | 1945 / 2409 |
| Quanta75/Quanta75_RP2040Stamp_JLCPCBAoptimi… | 162 | 0 / 0 / pass | ? / 22 / 26 / fail (cut at 695 s, board written) | 695 / 695 | 530 / 698 |
| MyKiCad/zpn_devboard | 167 | 1 / 0 / fail | ? / 22 / 205 / fail (cut at 2471 s, board written) | 824 / 2471 | 1479 / 2374 |
| Quanta75/Quanta75_BareRP2040_JLCPCBAoptimiz… | 172 | 18 / 1 / fail | cut at 3600 s (main loop 172/172 nets, 31 failed) | 1773 / 3600 | 2006 / 2512 |
| bit-preserve/coco2 | 173 | 0 / 0 / pass | cut at 657 s (main loop 172/172 nets, 2 failed) | 657 / 657 | 496 / 653 |
| computie/k30-SBC | 173 | 0 / 0 / fail | cut at 646 s (before the main loop) | 646 / 646 | 814 / 642 |
| OnBoard/koeg-board-pcb | 175 | 33 / 23 / fail | cut at 3600 s (main loop 175/175 nets, 9 failed) | 1630 / 3600 | 2168 / 2531 |
| OpenESC-30x30/4in1-panel | 184 | 485 / 220 / fail | cut at 2926 s (main loop 177/177 nets, 37 failed) | 975 / 2926 | 2052 / 2423 |
| OpenAirScope/OpenAirScope | 184 | 5 / 0 / fail | cut at 551 s (main loop 184/184 nets, 27 failed; rescue pass done, 18 recovered) | 551 / 551 | 1222 / 548 |
| rp2040-motor-controller/RP2040_base | 186 | 50 / 0 / fail | ? / 216 / 56 / fail (cut at 617 s, board written) | 617 / 618 | 1150 / 616 |
| 0xCB-1337/panel | 188 | 65 / 0 / fail | ? / 50 / 101 / fail (cut at 3600 s, board written) | 2355 / 3601 | 2921 / 4802 |
| Neotron-Pico/neotron-pico | 190 | 24 / 0 / fail | 190/190 / 0 / 5 / fail | 2427 / 971 | 2872 / 821 |
| M8SBC-486/homebrew_486 | 190 | 0 / 0 / pass | cut at 536 s (main loop 190/190 nets, 16 failed) | 536 / 536 | 975 / 535 |
| DSP-ADAU1452/DSP-ADAU1452 | 190 | 61 / 4 / fail | ? / 210 / 74 / fail (cut at 2752 s, board written) | 917 / 2752 | 1578 / 2398 |
| Sisu_SSE-9/Sisu_SSE-9 | 192 | 15 / 0 / fail | ? / 114 / 187 / fail (cut at 3600 s, board written) | 793 / 3600 | 1608 / 3660 |
| Business-Cards/Batch_1 | 204 | 441 / 0 / fail | cut at 3600 s (main loop 183/183 nets, 28 failed; rescue pass done, 7 recovered) | 810 / 3601 | 1665 / 3536 |
| nonSNES/SNSP-CPU-1CHIP | 212 | 0 / 0 / pass | cut at 600 s (before the main loop) | 600 / 600 | 461 / 596 |
| mackerel-68k/mackerel-30-proto | 217 | 0 / 2 / fail | cut at 616 s (main loop 217/217 nets, 12 failed; rescue pass done, 57 recovered) | 616 / 616 | 870 / 613 |
| sbc_allwinner_a13/module | 223 | 23 / 3 / fail | ? / 218 / 117 / fail (cut at 3071 s, board written) | 1024 / 3072 | 1808 / 2516 |
| glasgow/revC3 | 226 | 180 / 0 / fail | ? / 163 / 0 / fail (cut at 2650 s, board written) | 883 / 2650 | 1248 / 2490 |
| laptop/motherboard | 236 | 4 / 0 / fail | ? / 147 / 155 / fail (cut at 3600 s, board written) | 1279 / 3600 | 2055 / 3112 |
| PolyKybd/poly_corne_split42_left | 247 | 7 / 0 / fail | cut at 3600 s (main loop 247/247 nets, 82 failed) | 1388 / 3601 | 2224 / 2368 |
| link/link | 251 | 6 / 0 / fail | cut at 3600 s (main loop 251/251 nets, 125 failed) | 1770 / 3600 | 2081 / 2388 |
| MokyaLora/MokyaLora | 262 | 0 / 0 / pass | ? / 27 / 0 / fail (cut at 2522 s, board written) | 841 / 2522 | 1492 / 2479 |
| MyKiCad/vaio_re | 267 | 4 / 6 / fail | cut at 2999 s (main loop 266/266 nets, 29 failed) | 1000 / 2999 | 1559 / 2445 |
| atat-1800/ATAT1800 | 278 | 30 / 0 / fail | cut at 3600 s (main loop 278/278 nets, 96 failed; rescue pass done, 80 recovered) | 1881 / 3600 | 2219 / 2326 |
| MegaDrive/MegaDrive | 298 | 168 / 0 / fail | cut at 3600 s (main loop 298/298 nets, 2 failed) | 2103 / 3600 | 1533 / 3519 |
| hackrf/hackrf-one | 319 | 0 / 0 / pass | cut at 284 s (main loop 319/319 nets, 1 failed) | 284 / 284 | 424 / 283 |
| 0xCB-1337/pcb-panel | 340 | no result | cut at 3548 s (main loop 340/340 nets, 51 failed; rescue pass done, 7 recovered) | 1307 / 3548 | 674 / 2872 |
| jetson-nano-baseboard/jetson-nano-baseboard | 340 | 19 / 0 / fail | cut at 2462 s (main loop 340/340 nets, 23 failed) | 821 / 2462 | 1546 / 2337 |
| OnBoard/PixelWave | 343 | 0 / 0 / pass | 340/343 / 3 / 20 / fail | 359 / 964 | 351 / 1019 |
| nonSNES/SNSP-CPU-01 | 363 | 161 / 0 / fail | cut at 1632 s (main loop 363/363 nets, 5 failed) | 1632 / 1633 | 1786 / 1623 |
| ecad-viewer/video | 371 | 254 / 0 / fail | cut at 3600 s (main loop 371/371 nets, 24 failed) | 1369 / 3601 | 1450 / 2395 |
| PolyKybd/poly_corne_split42_right | 420 | 22 / 0 / fail | cut at 3600 s (main loop 420/420 nets, 145 failed) | 1879 / 3600 | 2296 / 2484 |
| PolyKybd/poly_kybd_split72_left | 420 | 37 / 0 / fail | cut at 3600 s (main loop 420/420 nets, 137 failed) | 1677 / 3600 | 2433 / 2474 |
| PolyKybd/poly_kybd_split72_right | 420 | 24 / 0 / fail | cut at 3600 s (main loop 420/420 nets, 148 failed) | 1928 / 3600 | 2388 / 2483 |
| SmartSpin2k/SmartSpin2k_Panelized | 585 | 45 / 0 / fail | cut at 730 s (main loop 585/585 nets, 137 failed) | 730 / 730 | 724 / 726 |

**Timing on ten boards.** These span the sizes: spell_tome_bottom,
USB_Keypad, ErgoSNM, eternal-keypad, mackerel-08, CyberKeeb2040, BAGEL-MK1,
MIDAS-MK2, Sisu and PixelWave. KiCadRoutingTools ran each to completion
(at most 3600 s); it has no thread setting, so 1 and 16 threads are the
same run for it, and its CPU seconds equal its wall to within a few
percent. Ours ran at 1 and 16 threads with `PCB_MAKER_NO_CACHE=1`, one
board at a time beside other agents' runs: at 1 thread under the tier
lock (`build/krt-ours-t1`), at 16 without it, admitted by the memory
ledger (`--min-free-gb 10`, `build/krt-ours-t16`; MIDAS-MK2's and
mackerel-08's 16-thread rows are the Freerouting study's). Each cell: wall /
CPU seconds, then KiCad's unconnected items / copper errors ("pass": the
strict rule).

| board | ours t1: wall / CPU s | open/errors | ours t16: wall / CPU s | open/errors | KRT: wall / CPU s | open/errors | KRT CPU over ours t1 / t16 |
|---|---:|---|---:|---|---:|---|---:|
| spell_tome_bottom | 8 / 8 | 0/0 pass | 8 / 9 | 0/0 pass | 28 / 45 | 0/0 pass | x6.0 / x5.1 |
| USB_Keypad | 195 / 349 | 2/0 | 397 / 734 | 2/0 | 3600 / 3607 | 6/77 (cut at 3600 s) | > x10.3 / > x4.9 |
| ErgoSNM_keyboard | 88 / 89 | 0/0 pass | 153 / 156 | 0/0 pass | 3600 / 3568 | 11/7 (cut at 3600 s) | > x40.0 / > x22.9 |
| eternal-keypad | 31 / 31 | 0/0 | 46 / 48 | 0/0 | 1037 / 1082 | 2/1 | x34.7 / x22.8 |
| mackerel-08-v1 | 207 / 268 | 0/0 pass | 455 / 563 | 0/0 pass | 339 / 359 | 0/92 | x1.3 / x0.6 |
| MainBoard | 226 / 226 | 0/0 pass | 344 / 372 | 0/0 pass | 1870 / 1795 | 0/0 pass | x7.9 / x4.8 |
| BAGEL-MK1 | 193 / 253 | 0/0 pass | 239 / 322 | 0/0 pass | 1021 / 1163 | 22/197 | x4.6 / x3.6 |
| MIDAS-MK2 | 313 / 372 | 0/0 pass | 392 / 485 | 0/0 pass | 1044 / 1122 | 53/23 | x3.0 / x2.3 |
| Sisu_SSE-9 | 696 / 1309 | 15/0 | 623 / 1293 | 15/0 | 3600 / 3660 | 114/187 (cut at 3600 s) | > x2.8 / > x2.8 |
| PixelWave | 551 / 439 | 0/0 pass | 289 / 302 | 0/0 pass | 964 / 1019 | 3/20 | x2.3 / x3.4 |

Over the seven boards it finished, KiCadRoutingTools' CPU is a median 4.6
times ours at 1 thread and 3.6 times ours at 16 threads (wall: 3.5 and 3.3
times); its three cut runs (USB_Keypad, ErgoSNM, Sisu) are at least 2.8
times. The one board it finished on less CPU is mackerel-08 against our
16-thread run (359 against 563 s), with 92 copper errors to our none; ours
is the better result on five of the seven, and both strict-pass the
other two (spell_tome_bottom, CyberKeeb2040). 16 threads did not make ours faster here: half of the
1-thread runs (the small boards) ran at a load of 1 to 3, the 16-thread
runs at 13 to 34, and the route mode's budget is work, not wall.

### What it has that we lack (for the roadmap)

Read from its source at cddf4cd (2026-10-21; paths relative to its
checkout). Size first: about 776 000 lines of Python (py_router 184 000,
py_placer 85 000, py_tools 19 000, the KiCad plugin 21 000, tests and
measurement scripts 398 000, an experimental CP-SAT bus router `awx/`
64 000) around 5 055 lines of Rust; `py_router/route.py` alone is 9 496
lines. The search is single-ended A* in Rust over (x, y, layer) on a
0.1 mm grid (0.05 mm for tap retries, 0.025 mm for rescues), eight
directions, a via a layer change in place, octile heuristic weighted 2.3;
obstacle rasterisation is Python. Rip-up takes up to three blockers from
the failed search's frontier (ranked by count, nearness to the target,
both directions or a soft-cost min-cut probe), with ghost costs on ripped
corridors and PathFinder-style history costs. Nothing runs in parallel:
no threads in the Rust core, one Python process.

**Differential pairs with polarity.** A centreline is routed by a
pose-based A* in Rust (`pose_router.rs`, `dubins.rs`: x, y, eight
headings and the layer; moves straight, a 45-degree turn followed by
`min_radius` straight steps, or a via that keeps the heading; the
heuristic is the Dubins path length at the turning radius), with the
pair's half-width plus clearance as its clearance, a cumulative-turn
limit against loops, vias only straight in and straight out, and both
P/N via sites checked. P and N are offsets of the simplified centreline
(miter-scaled at bends, per-layer pitch). Polarity is the sign of P's
side at both ends; a mismatch is fixed by leaving one end on the other
side ("connector flip") or by swapping the target pads' nets (opt-in by
pattern), applied to the board and, with `--schematic-dir`, to the pin
numbers in the schematic's lib_symbols. Fallbacks: a coupled middle with
single-ended legs, and single-ended routing for electrically short pairs.
GND return vias beside the signal vias are chosen in Rust during the via
expansion. Intra-pair skew is matched with meanders on the shorter leg.
About 15 500 lines (Python core 12 700: route_diff.py 2 961,
diff_pair_routing.py 5 431, diff_pair_loop.py 1 222,
diff_pair_multipoint.py 1 239; Rust 931; GUI 1 630).

**Length and time matching.** After routing; a member's length is track
length plus via barrels (stackup thickness between the joined layers)
plus the pad's die length; the longest member is the target and the
others get trombone meanders (rectangular bumps with 45-degree chamfers
at a pitch of a multiple of the width) on their longest single-layer
straight runs, each bump checked against a clearance index and shrunk
(0.7 times, down to 0.2 mm) or shifted until it fits, iterated to the
tolerance; pairs are meandered on the centreline and regenerated. Groups
are given by pattern or `auto`, a DDR3/DDR4 name parser that forms byte
lanes (DQ, DQS, DM) and command/address groups per interface and
channel. Time matching converts with sqrt(eps_eff)/c per segment (outer
layers (eps_r+1)/2 or Hammerstad's width-dependent value, inner eps_r,
from the stackup). `.kicad_dru` length windows are checked, not routed
to. About 4 300 lines (length_matching.py 4 185).

**Impedance-controlled widths.** `impedance.py` (1 793 lines):
IPC-2141/Hammerstad microstrip with Hammerstad-Jensen thickness
correction, IPC-2141 symmetric and asymmetric stripline, edge-coupled
pairs by an empirical coupling factor, coplanar waveguide over ground by
conformal mapping (elliptic integrals by AGM); the width for a target
impedance by bisection, per layer, from the board's stackup (dielectric
height, eps_r, copper thickness), clamped at the fab floor. The router
then takes a static width per layer and net. `check_impedance.py`
(1 112) audits reference-plane voids and split crossings under the
routed tracks. About 2 900 lines.

**BGA and QFN fanout.** BGA (`bga_fanout/`, 14 600 lines): the default
escapes through channels between ball rows (a 45-degree stub into a
channel, out past the array, layers assigned with collision repair and
rebalance); `underpad` puts a via in each signal pad and runs a small
per-ball A* on an inner layer under the array, deepest balls first;
`dogbone` puts the via in the diagonal gap; rotated and back-side parts
by frame transforms. QFN/QFP (`qfn_fanout/`, 2 700): two-segment stubs
along each pad's axis, optionally a via just past the pad. A Hungarian
assignment (scipy, `target_swap.py` 1 756) swaps swappable target pads
to minimise distance plus crossings. Decoupling capacitors are nudged
and rotated off foreign fanout vias toward same-net balls
(`fanout_clearance.py` 5 060). About 27 700 lines with the GUI.

**Placement optimisation.** `place_optimize.py` runs a zero-temperature
quench from the existing placement (`quench.py` 5 431): nudges within a
displacement radius, rotations, same-footprint swaps; the objective is
the per-net MST airwire length plus a penalty per airwire crossing, a
whitespace halo and an edge margin; legality (courtyards per side,
Edge.Cuts containment, pad clearance, declared floorplan zones) is hard
(`legality.py` 7 301). `place_route_loop.py` (1 594) puts the router in
the loop: route, read the failed nets and their blockers, micro-quench
the small parts on them with those nets weighted, accept only if
failures and effort improve, else revert and widen. It does not place
from scratch except from a declared floorplan intent (`seeder.py`,
`floorplan.py`, 23 000 lines together). py_placer is 85 000 lines; the
quench and the loop about 14 700.

**DRC and connectivity checkers.** `check_drc.py` (6 248) is its own
geometric DRC on the parsed board (clearances between tracks, vias and
pads, holes, board edge, same-net crossings, fab minimums, vias in paste
openings, `.kicad_dru` length windows) with KiCad's netclass and rule
resolution, but no zone fill; `check_connected.py` (2 448) is union-find
connectivity with zones by a raster fill model; `check_weird.py` (1 012)
copper hygiene (dangling ends, redundant loops, stacked duplicates).
KiCad is the oracle: `kicad_oracle.py` (3 523) runs `kicad-cli pcb drc
--refill-zones` on a copy whose severities are all off but
unconnected_items, and routes exactly the links KiCad reports open;
`kicad_exact_fill.py` (1 193) drives pcbnew's zone filler for exact
islands; every route.py ends with a plane finalize (region joins, pad
taps, clean-up) and that oracle pass. About 20 000 lines (24 000 with
the plane repair).

**The AI tab and Claude skills.** The plugin spawns Claude Code (`claude
-p "/<skill> ..." --output-format stream-json --allowedTools
Read,Glob,Grep,Bash,Agent,Task,WebSearch,WebFetch`) or opencode headless,
streams the transcript and reads a final `RESULT=` line back into the
dialog's fields. `/plan-pcb-routing` (a 3 504-line SKILL.md) reads the
board (stackup, parts needing fanout, diff pairs, DDR groups, power and
speed classes) and returns a JSON plan of steps (fanout, cap clean-up,
route_diff, route, route_planes) that the plugin executes in-process
through each tab's own handler; the AI only plans. Plans can be saved,
replayed without a model, built from a recorded command chain
(`make_plan.py`) and run headless (`run_plan.py`). Other skills: power
nets and widths and high-speed nets from datasheets found by web search,
diff pairs by pin function, stackup, plane mappings, failure diagnosis,
post-route review, an end-to-end place-and-route agent, and a corpus
stress test. About 14 300 lines (plugin AI code and plan tools 5 300,
SKILL.md files 6 700, their scripts 1 800).

Logs: `build/krt-route-x1367{,-b,-c}.log`, `build/krt-timing.log`,
`build/krt-ours-t1.log`, `build/krt-ours-t16.log`, `build/krt-chain.log`;
the whole table and summary in `build/krt-compare.txt`. Our timing runs
are a loop of `PCB_MAKER_NO_CACHE=1 run.py build/krt-ours-t<threads>/<task>
--route-from build/github-layout-v17 --only <task> --jobs 1 --threads
<threads> --min-free-gb 10 --binary build/bin/pcb-maker-x1367`.

## Designer vias in small SMD pads, the 109 boards of tasks v7 (2026-10-28)

score-kicad-board's `vias_in_pads_at` (filled or not) on each designer's board
(build/bin/pcb-maker-vip1-a), and the pads the via-in-pad rule admits on
it (`describe-kicad-board`'s `vias_in_pads`: the enclosed pads of a
footprint the board's own vias lie in; docs/router.md, "Vias in BGA
pads"). 61 boards, 3138 vias; the 48 boards without any are left out.
Enclosed pads of footprints without a designer via (not admitted):
vd-rd's sbc module U200 (BGA-96, 16 balls), byrantech's laptop keyboard
U2 (aQFN-73, AC9, AC11, AC13).

| Board | Vias in small SMD pads | Parts | Footprints (vias) | Enclosed pads the rule admits |
| --- | --- | --- | --- | --- |
| OpenDrone-hw__OpenESC-30x30__4in1-panel | 601 | 140 | PDFN-8L_L6.0-W5.0-P1.27 236, C_1206_3216Metric 184, Diodes_PowerDI3333-8 70, R_0201_0603Metric 42, 10 more | - |
| OpenDrone-hw__OpenESC-30x30__4in1 | 416 | 82 | PDFN-8L_L6.0-W5.0-P1.27 281, C_1206_3216Metric 89, R_0201_0603Metric 20, C_0201_0603Metric 12, 4 more | - |
| OpenDrone-hw__OpenRX__OpenRX-panel-rev2 | 215 | 102 | C_0201_0603Metric 52, QFN-32-1EP_5x5mm_P0.5mm_EP3.7x3.7mm 46, QFN-16_L3.0-W3.0-P0.50-BL-EP1.7 19, QFN-24_L4.0-W4.0-P0.50-TL-EP2.6 18, 14 more | - |
| MbFredys__PCB-Modular-Multi-Protocol-Hub__Hub | 208 | 118 | C_0402_1005Metric 57, C_0603_1608Metric 44, R_0603_1608Metric 34, ESP32-MINI 26, 15 more | - |
| OpenDrone-hw__OpenESC-20x20__4in1-mini | 199 | 72 | C_1206_3216Metric 75, Diodes_PowerDI3333-8 70, R_0201_0603Metric 21, C_0402_WIDE 8, 7 more | - |
| briskspirit__Sisu_SSE-9__Sisu_SSE-9 | 183 | 114 | C_0402_1005Metric 56, R_0402_1005Metric 32, C_0603_1608Metric 14, C_0805_2012Metric 9, 23 more | - |
| oro-os__link__link | 155 | 104 | CAP_0402_N_WAL 58, 150120M173000 15, STA_RMCF0402_STP 11, CAP_MSAS1L-3_TAY 8, 20 more | - |
| OpenDrone-hw__OpenFC-Lite__OpenFC | 125 | 54 | R_0201_0603Metric 17, CONN-SMD_SM06B-SRSS-TB-LF-SN 16, C_0201_0603Metric 14, USB-TYPE-C-SMD_TYPE-C-16P-QTWT 14, 20 more | - |
| CRImier__MyKiCad__vaio_re | 70 | 59 | C_0402_1005Metric 14, R_0402_1005Metric 11, USON-10_2.5x1.0mm_P0.5mm 9, C_0805_2012Metric 8, 15 more | - |
| 0xCB-dev__0xCB-1337__pcb-panel | 64 | 16 | C_0201_0603Metric 52, R_0201_0603Metric 12 | - |
| tubbytwins__bumwings-kbd__bumwings_v001R64_nano_sd | 64 | 64 | Diode_D_SOD-123 64 | - |
| tubbytwins__bumwings-kbd__bumwings_v001R64_rp2040zero_sd | 64 | 64 | Diode_D_SOD-123 64 | - |
| tubbytwins__bumwings-kbd__bumwings_v001R64_xiao_sd | 64 | 64 | Diode_D_SOD-123 64 | - |
| Twisted-Fields__rp2040-motor-controller__RP2040_base | 59 | 54 | C_0603_1608Metric 17, C_0805_2012Metric 12, TestPoint_Pad_D1.0mm 6, SOT-23-6 6, 9 more | - |
| thpoll83__PolyKybd__poly_kb_molecule_7x5_wave_left | 56 | 2 | C_0402_1005Metric 28, C_0603_1608Metric 28 | - |
| tubbytwins__bumwings-kbd__bumwings_v001R55_rp2040zero_sd | 55 | 55 | Diode_D_SOD-123 55 | - |
| tubbytwins__bumwings-kbd__bumwings_v001R55_xiao_sd | 55 | 55 | Diode_D_SOD-123 55 | - |
| tubbytwins__bumwings-kbd__bumwings_v001_xiao_s | 53 | 53 | Diode_D_SOD-123 53 | - |
| tengigabytes__MokyaLora__MokyaLora | 45 | 43 | TestPoint_Pad_D1.0mm 45 | - |
| thpoll83__PolyKybd__poly_kb_molecule_4x5 | 40 | 2 | C_0402_1005Metric 20, C_0603_1608Metric 20 | - |
| thpoll83__PolyKybd__poly_kb_molecule_5x4_wave | 40 | 2 | C_0402_1005Metric 20, C_0603_1608Metric 20 | - |
| bismarx-v1__Sumec-MiniSumo__SUMEC_MK_IV | 34 | 23 | C_0805_2012Metric_Pad1.18x1.45mm_HandSolder 6, ESP32-S3-WROOM-1 6, R_0805_2012Metric_Pad1.20x1.40mm_HandSolder 5, TestPoint_Pad_1.0x1.0mm 4, 6 more | - |
| thpoll83__PolyKybd__poly_kb_molecule_4x4 | 32 | 2 | C_0402_1005Metric 16, C_0603_1608Metric 16 | - |
| CRImier__MyKiCad__zpn_devboard | 28 | 26 | R_0402_1005Metric 7, C_0402_1005Metric 6, JST_SH_BM04B-SRSS-TB_1x04-1MP_P1.00mm_Vertical 4, USON-10_2.5x1.0mm_P0.5mm 4, 6 more | - |
| CDFER__Business-Cards__USB_Cable_Tester__PCB | 22 | 4 | USB-A-SMD_HC-USB3.0-C225-P 12, USB-SMD-LIGHTNING_USB-PG 7, USB_Micro-B_XKB_U254-051T-4BH83-F1S 2, TYPE-C-USB-SMD_QCHT-143 1 | - |
| thpoll83__PolyKybd__poly_kb_molecule_5x2_shifted | 20 | 2 | C_0402_1005Metric 10, C_0603_1608Metric 10 | - |
| 0xCB-dev__0xCB-1337__panel | 16 | 4 | R_0603_1608Metric 8, SOT-23-6 4, C_0603_1608Metric 4 | - |
| 0xCB-dev__0xCB-1337__pcb | 16 | 16 | C_0201_0603Metric 13, R_0201_0603Metric 3 | - |
| bitshiftcrazy__d20_pcb__d20_pcb | 16 | 13 | LED_0603_1608Metric 15, SOIC-14_3.9x8.7mm_P1.27mm 1 | - |
| ISSUIUC__ISS-PCB__MIDAS-MK1.1__MIDAS-MK1.1-revA | 10 | 1 | TFBGA-153_11.5x13mm 10 | U106 5 |
| ISSUIUC__ISS-PCB__MIDAS-MK1__MIDAS-MK1-revA | 10 | 1 | TFBGA-153_11.5x13mm 10 | U106 5 |
| CRImier__MyKiCad__protoesp | 9 | 5 | SOIC-28W_SOIC16_CH34x_Combined 5, R_0603_1608Metric 3, C_0805_2012Metric 1 | - |
| Huaqiu-Electronics__ecad-viewer__video | 9 | 9 | R_1206_3216Metric_Pad1.30x1.75mm_HandSolder 9 | - |
| GlasgowEmbedded__glasgow__glasgow__revC3 | 8 | 7 | TestPoint_Pad_D1.0mm 6, R_0402_1005Metric_DNP 2 | - |
| ISSUIUC__ISS-PCB__BAGEL-MK1.1-Avocado | 7 | 7 | R_0402_1005Metric 6, C_0402_1005Metric 1 | - |
| antmicro__jetson-nano-baseboard__jetson-nano-baseboard | 7 | 6 | SOT-23-3 3, C_1206_3216Metric 2, FB_0603_1608Metric 1, C_0402_1005Metric 1 | - |
| stonedDiscord__nonSNES__SNSP-CPU-1CHIP | 7 | 2 | SO-24_5.3x15mm_P1.27mm 6, SO-8_3.9x4.9mm_P1.27mm 1 | - |
| 0xCB-dev__0xCB-1337__1337-v4.0 | 4 | 4 | R_0603_1608Metric 2, SOT-23-6 1, C_0603_1608Metric 1 | - |
| CDFER__Business-Cards__Batch_1 | 4 | 4 | SOT-23-3_L2.9-W1.6-P1.90-LS2.8-BR 2, SW-SMD_4P-L5.1-W5.1-P3.70-LS6.5-TL-2 1, 0402_1005Metric 1 | - |
| CRImier__MyKiCad__framework_mobo_lefthalf | 4 | 3 | R_0805_2012Metric 2, R_Array_Concave_4x0603 1, SOT-23 1 | - |
| Jana-Marie__ligra__ligra_back | 4 | 2 | R_0805 2, Diodes_SO-8EP 2 | - |
| byrantech__laptop__power | 4 | 4 | R_0603_1608Metric 3, C_0603_1608Metric 1 | - |
| stonedDiscord__nonSNES__SNSP-CPU-01 | 4 | 2 | SOIC-14_3.9x8.7mm_P1.27mm 2, SO-14_3.9x8.65mm_P1.27mm 2 | - |
| zli117__CyberKeeb2040__MainBoard | 4 | 4 | C_1206_3216Metric_Pad1.33x1.80mm_HandSolder 2, D_DO-35_SOD27_P7.62mm_Horizontal 2 | - |
| hackclub__OnBoard__PixelWave | 3 | 3 | C_1206_3216Metric 1, TVS_ESDA25P35-1U1M 1, SOT-23-3 1 | - |
| tzarc__keyboards__ghoul | 3 | 3 | R_Array_Convex_4x0603 2, NetTie-2_SMD_Pad0.2mm 1 | - |
| apfaudio__eurorack-pmod__eurorack-pmod-pcb | 2 | 2 | LED_0402_1005Metric 1, R_0402_1005Metric 1 | - |
| bitshiftcrazy__spell_tome__spell_tome_bottom | 2 | 2 | C_0805_2012Metric 1, R_0603_1608Metric 1 | - |
| byrantech__laptop__motherboard | 2 | 2 | WLCSP9_1P2X1P2_ONS 2 | U7 1; U13 1 |
| greatscottgadgets__hackrf__hackrf-one | 2 | 2 | SKY13350-385LF 2 | - |
| hackclub__OnBoard__krishveercard | 2 | 2 | C_0603_1608Metric 1, HRS_DF40C-100DP-0.4V(51) 1 | - |
| sporkus__le_chiffre_keyboard_stm32__stm32_chiffre_36keys | 2 | 2 | D_SOD-123 1, LED_SK6812MINI_PLCC4_3.5x3.5mm_P1.75mm 1 | - |
| sporkus__le_chiffre_keyboard_stm32__stm32_hotswap_chiffre | 2 | 2 | D_SOD-123 1, LED_SK6812MINI_PLCC4_3.5x3.5mm_P1.75mm 1 | - |
| CDFER__Business-Cards__USB_Keypad | 1 | 1 | SW-SMD_4P-L5.1-W5.1-P3.70-LS6.5-TL-2 1 | - |
| CDFER__Business-Cards__WLED_Matrix | 1 | 1 | 0402_1005Metric 1 | - |
| ISSUIUC__ISS-PCB__BAGEL-MK1 | 1 | 1 | R_0402_1005Metric 1 | - |
| Seeed-Studio__OSHW-reCamera-Series__reCamera_S101_v1.1 | 1 | 1 | X2SON4-0.625-1x1x0.6mm 1 | - |
| emertcakir__OpenAirScope__OpenAirScope | 1 | 1 | XDCR_SGP41-D-R4 1 | - |
| ikajdan__katia__katia | 1 | 1 | C_0603_1608Metric 1 | - |
| obsilab__Quanta75__Quanta75_BareRP2040_JLCPCBAoptimized | 1 | 1 | R_0603_1608Metric_Pad0.98x0.95mm_HandSolder 1 | - |
| thpoll83__PolyKybd__poly_kybd_split72_left | 1 | 1 | RP2040-QFN-56 1 | - |
