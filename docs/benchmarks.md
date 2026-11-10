# Benchmarks

What is measured, how to run it, how a row is judged, and what main does
tonight. The runner and its tiers are `benchmarks/agent-tasks/run.py`
([benchmarks/agent-tasks/README.md](../benchmarks/agent-tasks/README.md)
has every tier's details); `compare.py` diffs two runs; `placer_ab.py`
judges a placer change; `scripts/nightly.sh` measures main every night
and `scripts/nightly-report.py` writes the results into this file.
Older measurements and studies are in git history.

## The tiers

Every task starts where an agent starts: a harvested board with its
tracks stripped, its movable parts stacked at one point, and a
`constraints.json` (tasks v7, `benchmarks/agent-tasks/tasks/v7`, 109
boards). A layout task runs `layout-kicad-board`; a route task keeps the
designer's placement and runs `route-kicad-board`.

| Tier | Boards | Mode and budget | What it is for | Who runs it |
| --- | --- | --- | --- | --- |
| Fingerprints | Castor_and_Pollux, k30-SBC, jetson-nano, Sisu_SSE-9 | route mode from the v16 placements (`build/github-layout-v16`), budget scale 0.25, 1 x 4, `PCB_ROUTER_FINGERPRINT=1 PCB_ROUTER_NO_WALL_GUARD=1` | identity: a router change that should change nothing shows identical fingerprints and routed boards | a router package; the nightly |
| Short (`--tier short`) | 12 boards of the quick tier, chosen by how often their result flipped between rounds | layout, a third of the budget | iterating on a placer or layout change; about ten minutes | a placer or layout package; the nightly (3 x 2) |
| Quick (`--tier quick`) | 30 boards that lay out in about 1100 s or less | layout, the full budget | a wider check of a layout change, diffed against its last run | by hand |
| Expansion (`--tier expansion`) | 46 boards of the expansion intake, many designers and tools | route on the designer's placement, 30 s of work (`--budget-scale 10` for 300 s) | a router tier over a wider spread of boards than the harvested ones | a router package; the nightly (8 x 1) |
| Schematic (`--tier schematic`) | 12 harvested boards with a usable schematic | import from the schematic (`import-kicad-netlist`), the task's constraints moved onto the new parts, layout at a third of the budget | the flow an agent takes from a KiCad project: import, libraries, constraints and layout together | by hand |
| Layout sweep | all 109 boards of tasks v7 | layout, 1500 s of work each, `--quality` | the headline: how many boards pcb-maker places and routes to a clean KiCad verdict | the nightly only (10 x 2, `--order peak-desc`) |
| Matched designer tier (`--tier designer`) | the same 109 boards | route on the designer's own placement, 1500 s of work scaled by the median final-ladder share of the previous night's sweep (`nightly-report.py --ladder-share`; 0.695 without one) | splits what the layout leaves open into a placement gap and a routing gap (`compare.py --gap`) | the nightly only (8 x 1, `--order peak-desc`) |
| From schematic (`benchmarks/from-schematic/run.py`) | the KiCad demos whose footprints the libraries have | import, then `layout-kicad-board` with the starter `layout.json` unchanged | the whole way from a schematic with nothing but the starter files | by hand |
| KiCad demo corpus (`benchmarks/corpus/run.py`) | the KiCad demo projects (`benchmarks/corpus/fetch.sh`) | route and place + route | a small, quick sanity check on boards everyone knows | by hand |

The eleven **dense boards** (the boards of
`benchmarks/agent-tasks/dense-noise.json`: the 0xCB 1337 panel, protoesp,
glasgow revC3, video, OpenESC 4in1-mini and 4in1, SmartSpin2k_Panelized,
DSP-ADAU1452, MegaDrive, SNSP-CPU-01, split72_left) are the placer's
measurement set: layout mode at the task budgets, 3 x 2, `--only` with
their names.

The schematic tier's `--import-near` imports with `import-kicad-netlist
--near-decoupling` (the decoupling capacitors' `near` entries in the new
`constraints.json`). Measured on the tier, it keeps the unconnected count
(2 against 2) but the placer misses most of the entries, so the tier's
passes fall from 10 to 2: the import's flag stays off by default.

**What a package runs.** A router package runs the fingerprints and the
expansion tier against main's numbers; a placer or layout package adds
the short tier and, for identity, the dense boards at seed 1. A placer
default goes through the protocol below. No package runs the layout
sweep or the designer tier: the nightly runs them on main, and a
package's claim about the sweep is checked by the first nightly after its
merge.

### How to run each

Benchmark a copied binary (`bash scripts/snapshot.sh <n>` builds and
copies it to `build/bin/pcb-maker-x<n>`), never `target/release`. Every
run goes through the memory ledger (below) and may run beside others.

```sh
B=build/bin/pcb-maker-x15210
R=benchmarks/agent-tasks/run.py
# fingerprints (compare the sorted fingerprint lines of each route.log and the routed boards)
PCB_ROUTER_FINGERPRINT=1 PCB_ROUTER_NO_WALL_GUARD=1 $R build/fp-x15210 \
  --tasks benchmarks/agent-tasks/tasks/v7/tasks.json --route-from build/github-layout-v16 \
  --budget-scale 0.25 --jobs 1 --threads 4 --binary $B \
  --only wntrblm__Castor_and_Pollux__mainboard transistorfet__computie__k30-SBC \
         antmicro__jetson-nano-baseboard__jetson-nano-baseboard briskspirit__Sisu_SSE-9__Sisu_SSE-9
$R build/short-x15210 --tier short --jobs 3 --threads 2 --binary $B > build/short-x15210.log
$R build/expansion-x15210 --tier expansion --jobs 8 --threads 1 --binary $B > build/expansion-x15210.log
$R build/quick-x15210 --tier quick --binary $B > build/quick-x15210.log
$R build/sch-x15210 --tier schematic --binary $B > build/sch-x15210.log
# the layout sweep and the matched designer tier, as the nightly runs them
$R build/sweep-x15210 --tasks benchmarks/agent-tasks/tasks/v7/tasks.json --jobs 10 --threads 2 \
  --quality --order peak-desc --binary $B > build/sweep-x15210.log
$R build/designer-x15210 --tier designer --budget-scale 0.613 --quality --jobs 8 --threads 1 \
  --order peak-desc --binary $B > build/designer-x15210.log
# against a reference run
benchmarks/agent-tasks/compare.py build/short-main.log build/short-x15210.log
benchmarks/agent-tasks/compare.py --gap build/sweep-x15210.log build/designer-x15210.log --targets
# from a schematic; the KiCad demo corpus
benchmarks/from-schematic/run.py build/from-schematic --jobs 2
benchmarks/corpus/fetch.sh && python3 benchmarks/corpus/run.py build/corpus
```

Useful options of run.py: `--only NAME...` (a subset), `--route-from DIR`
(route the placements of an earlier layout run: router experiments
without the placement), `--splice-from LOG` (run only `--only`'s boards
and copy the other rows from a log, marked spliced), `--reuse DIR` (a
layout whose placement digest matches a stored one skips its routing),
`--placer '{"seed": N}'` and `--layout '{...}'` (settings), `--budget-scale`,
`--dry-run` (the start order and each board's memory declaration).

`compare.py` prints, over the boards in both runs, better / same / worse
by KiCad's unconnected count, the passes gained and lost, the summed
objectives and the medians against the designer's board. It marks rows
cut (CUT), killed (KILLED), spliced (SPL) and boards whose input changed
(INPUT: task, constraints, board or project files), which it leaves out
of better and worse.

## The pass rules

A row passes when every connection is routed and KiCad's check finds no
copper error beyond the designer's, and every constraint holds but those
the task lists as impossible. Silkscreen and library cosmetics never
count. Two rules decide "beyond the designer's":

- **The per-type rule** (`pass`): per finding type, KiCad's count on our
  board less the count on the designer's own routed board. Lenient: a
  finding of ours hides behind one of the designer's anywhere on the
  board.
- **The strict rule** (`pass_strict`, `new_findings_strict`, rule 4,
  `strict_rule` in every row; the headline counts this one): a finding of
  ours counts unless the designer's board with its tracks, vias and small
  zones stripped (`baseline-drc.json`) has it at the same place: the same
  type and the same items the same distances apart, or items at the same
  positions within 0.05 mm, each designer finding answering for one. A
  finding of one item is the designer's only where that item lies within
  0.05 mm of theirs. A finding between items of one footprint is always
  the designer's (its pads and graphics, which no placement or route
  changes). A solder mask bridge of a mask opening that is bridged on
  the designer's boards already is not counted (`strict_mask_openings`).
  `rejudge.py` judges a stored run again under the current rule.

A row without a result (no legal placement, an error, a timeout) fails
under both rules. A row the machine ended (a signal, or the memory ledger
twice) is `killed`: no result, and compare.py leaves it out of better and
worse. A row the wall clock cut is marked `cut`. The schematic tier also
needs every footprint found (`missing`).

## Measurement discipline

A layout is deterministic for its binary, task, rates and settings,
whatever its thread count, unless the wall clock cuts it (such rows carry
`cut`). One run per side answers a quality question; a repeat gives the
same rows.

- **Pinned work rates.** Budgets are in seconds of work, not wall time.
  run.py pins `PCB_WORK_RATE=3.15e6,150e6` (the reference) for every
  board, so rows compare across machines and loads; `--work-rate
  measured` lets each binary measure its machine instead, which ties the
  rows to the machine and its load. The wall-clock guards stretch by the
  pinned rate over the measured one (at most four times,
  `PCB_WALL_STRETCH_MAX`), so a loaded machine takes longer instead of
  cutting rows; a row is cut only below a quarter of the reference rate.
- **Tests pin the rates too.** Every test run of the fixtures, the
  walkthrough, the fingerprints and the hostile inputs sets
  `PCB_WORK_RATE=3.15e6,150e6` and `PCB_ROUTER_NO_WALL_GUARD=1` (and the
  fixtures `PCB_MAKER_MEMORY_MB`). A test that checks the measured rates
  asserts the rates only, nothing a rate could change. No wall-clock bound
  decides a test; a wall bound is only a hang guard far above any load.
- **Judge by continuous objectives from one run**, not by pass counts,
  which move by single-error flips: KiCad's unconnected items, new copper
  errors, vias, length, work seconds, the first route's open terminals,
  and the medians against the designer. Totals compare only over the same
  rows with a result.
- **One task list per comparison.** Pass the same `--tasks` on both sides
  or rely on the tier's default on both. compare.py says TASK LISTS DIFFER
  and marks changed boards INPUT.
- **Threads.** Tiers default to 2 search threads a board and physical
  cores / 2 boards at once (`TIER_THREADS`). Rows are comparable across
  thread counts as long as no row is cut; CPU and wall seconds are not.
- **Paired CPU runs only for CPU claims.** Both binaries at the same time
  on symmetric halves of the cores (`taskset -c 0-7,16-23` against
  `8-15,24-31`: logical CPUs n and n + 16 share a physical core, never
  give the two sides siblings), CPU seconds from rusage (the rows'
  `cpu_seconds`), never wall seconds. Reserve the memory for both sides
  at once and start both inside it (`PCB_MAKER_LEDGER_INSIDE`): a pair
  whose one side waited is void. KiCad's DRC reports are cached
  (`~/.cache/pcb-maker/drc`); the side that checks a board first pays and
  the other hits, so CPU pairs run with `PCB_MAKER_NO_CACHE=1` on both
  sides, or both warm. Quality comparisons are unaffected by the cache.
- **Rows say what made them** (`binary_sha256`, the task, board,
  constraints and project hashes, `strict_rule`, `spliced_from`, the rate
  fields). A spliced row is the old binary's: say so when quoting it.

## Measurement discipline: placer defaults

A placer change moves every board to another draw of its seed noise: on
the dense boards one board's unconnected count spreads by up to 193
(video) over placer seeds. The rule for a placer default, on the eleven dense boards in
layout mode at the task budgets (tasks v7, pinned rates, 3 x 2, through
the ledger):

1. Run **five seed pairs**, placer seeds 1, 101, 201, 301, 401 on both
   sides (`--placer '{"seed": N, ...}'`). Seeds lie at least 16 apart: the
   race places seeds N to N + 15 (`config.seed + offset` in
   `board_placer/place.rs`), so nearer seeds share candidates and are not
   independent draws.
2. Judge with `placer_ab.py compare --off DIR... --on DIR...` (each
   directory's seed from its `-s<N>`, or `DIR@N`). The statistic is the
   **weighted relative change**: each board's mean change over its seeds
   divided by its noise mean, weighted by 1 / max(cv, 0.10)^2 from the
   noise file. Its one-sided p is the larger of a paired sign-flip
   randomisation test (this comparison's own spread) and the noise file's
   null.
3. **Better** when p <= 0.03. When 0.03 < p <= 0.25, run seeds 501, 601,
   701, 801 too and judge the nine pairs again at p <= 0.03. Otherwise not
   better. Worse when the smaller p is at least 0.97.
4. The other conditions stay: the short tier loses no strict pass and
   gains no DRC error. A strict pass the short tier loses at its seed is
   checked on that board alone at the five seed pairs; the condition
   holds when the on side passes on at least as many of the five as the
   off side.

Simulated from the noise file (`placer_ab.py protocol`), the rule calls a
lever with no effect better about 4 % of the time and finds a 5 % effect
about 55 % of the time, 8 % 90 %, 10 % 98 %. The weighted change answers
"does the typical board route better", not "does the tier's count fall":
it listens to the steady boards, and a lever that helps only video (the
noisiest) does not pass it, nor would it pass any statistic on this
noise. The summed medians cannot resolve a 3 to 5 % effect at any number
of seeds that is affordable.

The off side's rows can be reused while main's layouts of the boards stay
the same; when main changes a dense board's layouts, its off rows and its
rows in the noise file need running again.

```sh
P=benchmarks/agent-tasks/placer_ab.py
$P compare --off build/X-dense-s{1,101,201,301,401}-off --on build/X-dense-s{1,101,201,301,401}-on
$P compare ... --metric first_open|objective|crossings --tool build/bin/pcb-maker-xNNNN   # a proxy
$P resolution --k 3 5 9 --at 0.03 0.05 0.10     # the smallest effect each statistic resolves
$P protocol                                     # the rule's false positive rate and power
$P noise build/Y-dense-s{1,...,801}-off --out benchmarks/agent-tasks/dense-noise.json \
  --tool build/bin/pcb-maker-xNNNN [--boards NAME... --update]
$P report                                       # the noise table and the proxies' correlations
```

### The noise file

`benchmarks/agent-tasks/dense-noise.json` holds, per dense board and for
nine seeds (1, 101, ..., 801) of one binary, KiCad's unconnected count,
the first route's open terminals, and with `--tool` the placed board's
wirelength and ratsnest crossings (the placement objective is the
wirelength plus 3 mm per crossing). Each board's entry names its binary;
`--update` replaces the named boards' rows. The nightly's losers use it
too: a loss on a dense board counts only beyond the board's seed spread.

Within a board across the seeds, the first route's open terminals follow
the final count (pooled r about 0.7) with as much relative spread; the
objective has little noise but explains a fifth of the final count's
spread. A lever that lowers the objective is not yet a routing gain.

### The ladder of placer measurements

The full protocol costs 110 layouts a lever (about 88 CPU hours). A lever
climbs cheaper stages first and stops where it fails:

| Stage | What runs | Cost | Calibration |
| --- | --- | --- | --- |
| 0: placement | the placement alone; `ablate-kicad-placement` metrics (crossings, objective, wirelength, blocked exits, overflows) | about a quarter of a layout | a check that the lever moved what it meant to move, not a filter: weakly tied to the final count |
| 1: first route | the five seed pairs with `--layout '{"stop_after_first_route": true}'`; `placer_ab.py compare --metric first_open` | 0.55 to 0.57 of the full stage | agreed with the full verdict on both calibration experiments (crossings in the anneal, the race's ripped count) |
| 2: full | the protocol above | 110 layouts | the ground truth |

- **A lever passes stage 1** when the first route's weighted change is
  negative at the noise file's p <= 0.25; then stage 2 on the same seeds.
  A full layout cannot start from a stopped one, and `--reuse` does not
  cross the option (it is in the key).
- **What it saves** is half: the first route is most of a dense
  layout's routing.
- **Reuse** (`run.py --reuse build/layout-reuse`): run both sides on one
  binary with the lever as an option; boards the lever leaves alone then
  cost their placement only. The key holds the placement digest
  (`layout: placement digest` on stderr, `placement_digest` in
  board-layout.json), the binary, the task, the settings, the rates, the
  threads and the `PCB_*` environment; a switch that lands in the lowered
  placement problem, or two binaries, never hit.
- **Not stages**: a quarter budget changes the placement itself (two
  dense boards find no legal placement) and does not rank the full rows;
  condensed boards (a window around a crowded cluster) were assessed and
  rejected ([placer.md](placer.md)).

## Memory ledger

Benchmarks and agents share the machine through the memory ledger
(`$XDG_RUNTIME_DIR/pcb-maker-ledger.json`, `PCB_MAKER_LEDGER`; how a run
reserves and waits: [agents.md](agents.md)). Every route, layout and KiCad
check of a binary reserves its expected memory before it allocates, and
run.py admits each board by the same ledger. `pcb-maker ledger` lists
who holds and who waits; `--lock` still takes the exclusive tier lock
(`build/quick-tier.lock`) for a wall or CPU measurement that must not
share the machine. The rules are the same in
`crates/pcb-kicad/src/ledger.rs` and `benchmarks/harness_common.py`.

- **Capacity**: MemTotal less the memory outside the ledger (the median
  of the headroom samples of the last six hours when they are fresh: at
  least 360, the newest at most a day old; else 25 % of MemTotal), at
  most MemTotal less 6 GB (`PCB_MAKER_LEDGER_MARGIN_MB`).
  `PCB_MAKER_LEDGER_TOTAL_MB` replaces it.
- **The floor**: run.py keeps the memory available, less what the
  reservations still grow by, above a floor per board: 2 GB to keep
  (`FLOOR_KEEP_MB`), plus the 0.95 quantile of how far the memory outside
  the ledger rose within 10 minutes over the last six hours of samples,
  plus the tier's overshoot (measured peak less declared, from
  estimates.json). Without an hour of fresh samples it is 10 GB.
  `--min-free-gb` overrides it. A binary's new request keeps the same
  floor (without the overshoot); a short request keeps 2 GB. The samples
  live in `<ledger>.headroom.json`; run.py samples while it runs, and
  `python3 benchmarks/ledger_headroom.py [--sample --for S]` samples or
  prints the floor per kind.
- **Ordered admission**: a waiting request is in the ledger, and a newer
  request may not pass the one that has waited longest: it goes only if
  the oldest would still fit beside it and every reservation that began
  after the oldest began waiting. A request larger than the capacity runs
  alone; after `--ledger-drain-max` (7200 s) of waiting for the ledger to
  empty it ends with an error.
- **The drain**: once the first waiter has waited `--ledger-drain-after`
  (600 s), a new request goes only if it leaves the waiter its room, or
  ends no later than every run that holds memory. The ledger drains only
  for a waiter it could start once the runs since its wait began end, and
  whose entry is fresh (rewritten within 120 s); past 7200 s it drains
  strictly (no early-end exception). The drain is for the first waiter
  that qualifies; the others keep their place by ordered admission.
  While the waiter is blocked by a run larger than 4 GB, requests of at
  most 4 GB still go (the feasibility cap).
- **The nightly's priority**: `--ledger-priority nightly` puts the
  nightly's requests first among the waiters; they are never passed, and
  the ledger drains for them at once.
- **Short requests never wait behind a drain**: a command that holds its
  memory for seconds (`drc-kicad-board`, `place-kicad-board`,
  `inspect-routing-run --native-check`, a command arming on demand for
  kicad-cli; `PCB_MAKER_LEDGER_SHORT` overrides) with at most 2 GB of its
  own beside one KiCad check (2253 MB) goes whenever the memory is there.
- **kicad-cli runs under a reservation**: a check command reserves one
  check up front; other commands that run kicad-cli arm on demand; a
  route or layout raises its reservation by the check while it runs.
- **A board's reservation is its binary's**: run.py hands the board's
  reservation to its binary, whose own limit is then at least that
  (`granted_mb`); the board's other binaries run inside it
  (`PCB_MAKER_LEDGER_INSIDE`), and those after it inside a short
  reservation of 4301 MB.
- **A run past its reservation waits** for room instead of ending. It
  ends with exit 4 only when the memory available falls below 2 GB, or
  after 600 s when the memory available less the other runs' growth is
  below 2 GB. run.py runs a row the ledger ended once more
  (`ledger_retry`); ended twice, it is killed.

### Per-board estimates

run.py declares each board's expected peak from
`benchmarks/agent-tasks/estimates.json` (`--estimates` another;
`--expected-gb` overrides every board's: give it only from a measured
peak, never a round placeholder). Per board and kind (layout, short,
schematic, designer, expansion) it holds the largest `peak_rss_mb` of the
newest three sources that measured it, times 1.25, and their largest
seconds. A board without a row of its kind falls back to its layout rows,
the tier file's entry, a size model over pads times copper layers, then
3 GB. `tier_estimates.py --refresh --layout <log>` (`--designer`,
`--short`, `--schematic`, `--expansion`, `--night build/nightly/<date>`)
writes it; commit the file after a refresh. The nightly keeps its own
copy, `build/nightly/estimates.json`, refreshed from each night's rows,
so its estimates follow the binary.

## Nightly

`scripts/nightly.sh` measures main every night at 02:00 (a systemd user
timer; `--install` sets it up, `--quick` leaves out the sweep and the
designer tier, `--no-commit-report` publishes nothing). It builds main's
head in its own checkout (`../pcb-maker-nightly`), snapshots the binary
as `build/bin/pcb-maker-nightly-<date>-<sha12>`, and runs, through the
memory ledger with `--ledger-priority nightly`: the fingerprints, the
expansion tier, the short tier, the matched designer tier and the layout
sweep. It writes `build/nightly/<date>/`: each stage's log and output,
`summary.md` (per stage and board), `nightly.json`, `gap.txt` and
`diff-<stage>.txt` against the previous night that ran the stage.

**Publishing.** `scripts/nightly-report.py --commit` (the last step)
writes the night into the main checkout and commits it on main as
"nightly <date>: layout sweep vN, <strict> strict (<change> against the
previous night), matched designer tier <strict> strict, gap placement
<p> / routing <r>", then pushes. It writes

- this file: the night's row of the table below, and a section "Layout
  sweep vN (<date>)" after the newest numbered sweep; only the newest two
  such sections stay (the next night compares with the previous one);
- README.md: the headline's measured part and the two newest table rows;
- docs/handover.md: the title's state, the nightly state line and the
  night note.

**The guard.** A night is published only when every stage ran, no row of
any stage was cut or killed, and the sweep has at least 100 rows with a
result; otherwise it is only summarised and `nightly.log` says why. The
report commits only when the main checkout is on main, has no change to
a tracked file, and `git pull --ff-only origin main` succeeds; otherwise
the next night publishes every guarded night after the newest one this
file has (at most three). By hand: `scripts/nightly-report.py --docs
build/nightly/<night>` (`--dry-run` prints the section and writes
nothing; `--checkout` writes another checkout's docs).

The table: one row a night, newest first, the last 14 nights. Cells:
passes / rows (strict passes), KiCad's unconnected items, rows cut and
killed; the designer tier's budget scale; the gap split over the target
boards (`compare.py --gap --targets`); against the previous night,
better / same / worse per stage and the passes gained and lost.
What keeps the matched designer tier's failing boards from the strict
pass is analysed in [router.md](router.md).

<!-- nightly:begin -->
| Night | main | Fingerprints | Expansion | Short | Sweep | Designer | Gap (placement / routing) | Against the previous night (better/same/worse) |
| --- | --- | --- | --- | --- | --- | --- | --- | --- |
| 2026-11-10 | `8bfeb313afbf` | identical | 26/46 (strict 26), 556 open | 9/12 (strict 9), 16 open | 69/109 (strict 70), 1552 open | 75/109 (strict 76), 2672 open at 0.613 | 67% / 33% | fingerprints 0/4/0; expansion 0/46/0; short 0/12/0; sweep 0/106/0; designer 2/102/5 (pass +1 -1) |
| 2026-11-09 | `80c050719bf8` | changed: jetson-nano-baseboard | 26/46 (strict 26), 556 open | 9/12 (strict 9), 16 open | 69/109 (strict 70), 1552 open | 75/109 (strict 76), 2634 open at 0.676 | 68% / 32% | fingerprints 0/4/0; expansion 1/44/1 (pass +1 -0); short 0/12/0; sweep 6/99/1 (pass +2 -0); designer 5/103/1 (pass +2 -0) |
| 2026-11-08 | `b090e40d64d3` | changed: Sisu_SSE-9, computie__k30-SBC | 25/46 (strict 25), 557 open | 9/12 (strict 9), 16 open | 67/109 (strict 68), 1697 open | 73/109 (strict 74), 2685 open at 0.663 | 71% / 29% | fingerprints 1/3/0; expansion 1/44/1 (pass +1 -0); short 3/9/0 (pass +2 -0); sweep 22/72/12 (pass +4 -0); designer 14/91/4 (pass +4 -1) |
| 2026-11-07 | `406dcb94a0f6` | changed: jetson-nano-baseboard, Sisu_SSE-9, computie__k30-SBC, Castor_and_Pollux__mainboard | 24/46 (strict 24), 558 open | 7/12 (strict 7), 21 open | 63/109 (strict 63), 1957 open | 70/109 (strict 67), 2756 open at 0.662 | 70% / 30% | fingerprints 0/4/0; expansion 0/46/0; short 0/12/0; sweep 11/87/8 (pass +3 -3); designer 1/107/1 (pass +0 -1) |
| 2026-11-06 | `66115bad368e` | identical | 24/46 (strict 24), 558 open | 7/12 (strict 7), 21 open | 63/109 (strict 63), 2100 open | 71/109 (strict 68), 2756 open at 0.647 | 74% / 26% | fingerprints 0/4/0; expansion 0/46/0; short 0/12/0; sweep 16/70/20 (pass +6 -4); designer 1/104/4 |
| 2026-11-05 | `93162bd9ecb2` | identical | 24/46 (strict 24), 558 open | 7/12 (strict 7), 21 open | 61/109 (strict 61), 2165 open | 71/109 (strict 68), 2719 open at 0.673 | 76% / 24% | fingerprints 0/4/0; expansion 0/46/0; short 1/11/0; sweep 12/82/12 (pass +1 -1); designer 1/108/0 |
| 2026-11-04 | `688b59ca8682` | identical | 24/46 (strict 24), 558 open | 7/12 (strict 7), 22 open | 61/109 (strict 61), 2514 open | 71/109 (strict 68), 2759 open at 0.675 | 73% / 27% | fingerprints 0/4/0; expansion 0/46/0; short 1/10/1 (pass +0 -1); sweep 9/94/4 (pass +1 -2); designer 0/109/0 |
| 2026-11-03 | `5657f451024a` | identical | 24/46 (strict 24), 558 open | 8/12 (strict 8), 23 open | 62/109 (strict 62), 2658 open | 71/109 (strict 68), 2759 open at 0.675 | 76% / 24% | fingerprints 0/4/0; expansion 0/46/0; short 0/12/0; sweep 0/107/0; designer 0/109/0 |
| 2026-11-02 | `912a1de89d28` | identical | 24/46 (strict 24), 558 open | 8/12 (strict 8), 23 open | 62/109 (strict 62), 2658 open | 71/109 (strict 68), 2759 open at 0.675 | 76% / 24% | fingerprints 0/4/0; expansion 0/46/0; short 0/12/0; sweep 0/107/0; designer 0/109/0 |
| 2026-11-01 | `593aae46ef99` | identical | 24/46 (strict 24), 558 open | 8/12 (strict 8), 23 open | 62/109 (strict 62), 2658 open | 71/109 (strict 68), 2759 open at 0.675 | 76% / 24% | fingerprints 0/4/0; expansion 0/46/0; short 0/12/0; sweep 0/107/0; designer 0/109/0 |
| 2026-10-31 | `56e15e9be431` | identical | 24/46 (strict 24), 558 open | 8/12 (strict 8), 23 open | 62/109 (strict 62), 2658 open | 71/109 (strict 68), 2759 open at 0.675 | 76% / 24% | fingerprints 0/4/0; expansion 0/46/0; short 0/12/0; sweep 0/107/0; designer 0/109/0 |
| 2026-10-30 | `288051a3e337` | identical | 24/46 (strict 24), 558 open | 8/12 (strict 8), 23 open | 62/109 (strict 62), 2658 open | 71/109 (strict 68), 2759 open at 0.675 | 76% / 24% | fingerprints 0/4/0; expansion 0/46/0; short 0/12/0; sweep 0/107/0; designer 2/107/0 |
| 2026-10-29 | `1f0ca50cccfb` | identical | 24/46 (strict 24), 558 open | 8/12 (strict 8), 23 open | 62/109 (strict 62), 2658 open | 71/109 (strict 68), 2795 open at 0.66 | 75% / 25% | fingerprints 0/4/0; expansion 0/46/0; short 1/11/0 (pass +1 -0); sweep 2/105/0 (pass +1 -0); designer 4/102/3 (pass +2 -1) |
| 2026-10-28T1429 | `c68e9a8f506f` | changed: Sisu_SSE-9 | 24/46 (strict 24), 558 open | 7/12 (strict 7), 28 open | 61/109 (strict 61), 2667 open | 70/109 (strict 68), 2804 open at 0.637 | 74% / 26% | fingerprints 0/4/0; expansion 0/46/0; short 1/9/2 (pass +0 -1); sweep 21/66/20 (pass +5 -3); designer 6/99/4 (pass +2 -0) |
<!-- nightly:end -->

## Layout sweep v37 (2026-11-10)

<!-- nightly-sweep night=2026-11-10 sweep=build/nightly/2026-11-10/sweep.log main=8bfeb313afbf25b11d2e2594353470e0d5f01066 -->

Written by the nightly (`scripts/nightly-report.py --docs`) from
`build/nightly/2026-11-10/`: main 8bfeb31 (1 commit after the nightly of
2026-11-09's main 80c0507), binary
`pcb-maker-nightly-2026-11-10-8bfeb313afbf` (sha256 5fad98b37cfc), tasks
v7 (sha256 c8f95caf5089), `--quality`, at the pinned work rates (the
measured router rate had a median of 2.6e6, work over wall 1.08), 10
jobs x 2 threads, `--order peak-desc`, through the memory ledger with
`--ledger-priority nightly`: 8423 s wall, largest peak 3.2 GB, no row
cut or killed in any stage. Per board:
`build/nightly/2026-11-10/summary.md`, `diff-sweep.txt` and `gap.txt`.

**Inputs.** Every board in both ran on the same input as in nightly
2026-11-09.

| | nightly 2026-11-09 (80c0507) | v37 (nightly 2026-11-10, 8bfeb31) |
|---|---|---|
| pass, per-type rule | 69 of 109 | **69 of 109** |
| pass, strict rule (rule 4) | 70 of 109 | **70 of 109** |
| unconnected (all rows) | 1552 | 1552 |
| unconnected where both have a result on the same input (that run / this) | 1552 / **1552** (106 boards) | - |
| new DRC errors (per-type rule) | 27 | 27 |
| new DRC errors (strict rule) | 15 | 15 |
| vias (rows with a result) | 29148 (106) | 29148 (106) |
| length (rows with a result) | 752 m (106) | 752 m (106) |
| work s | 72678 (106) | 72678 (106) |
| CPU s (unpaired, a shared machine) | 87973 | 82367 |
| first-route open terminals | 2002 (106) | 2002 (106) |
| rows cut by the wall clock / killed | 0 / 0 | **0 / 0** |
| rows without a result | 3 | 3 |
| our vias in small SMD pads (score-kicad-board) | 0 on all 106 | 0 on all 106 |
| vias against the designer's (median) | x0.65 (106) | x0.65 (106) |
| length against the designer's (median) | x0.98 (106) | x0.98 (106) |
| open over connections (median, mean) | 0.000, 0.063 | 0.000, 0.063 |
| bends per connection against the designer's (median) | x1.01 | x1.01 |
| stair-steps, ours less the designer's (median) | +33.5 | +33.5 |
| pin-row crossings, ours less the designer's (median) | +1 | +1 |
| ground pour cut, ours less the designer's (median) | -56.6 mm (99; 1 lost) | -56.6 mm (99; 1 lost) |
| final ladder's share of the budget (median) | 0.613 (34 rows) | **0.613** (34 rows) |

**Against the nightly of 2026-11-09** (compare.py, 109 boards in both on
the same input): better 0, same 106, worse 0; 106 of the 106 with a
result in both are identical in routed count, unconnected, vias, length
and strict findings; unconnected there 1552 -> 1552. Strict passes 70 ->
70: gained 0; lost 0.

**Rows without a result, 3** (the nightly of 2026-11-09: 3):
OpenESC-30x30 4in1-panel (the panel's 2 boards' parts are mixed: 392
movable parts belong to no sub-board; the previous night the panel's 2
boards' parts are mixed: 392 movable parts belong to no sub-board),
poly_kb_molecule_5x4_wave (2 parts without a legal place; the previous
night 2), poly_kb_molecule_7x5_wave_left (2 parts without a legal place;
the previous night 2).

**Strict errors, 15 on 7 boards**: link (length_out_of_range 6),
Sisu_SSE-9 (diff_pair_uncoupled_length_too_long 2), USB_Keypad
(connection_width 2), vaio_re (solder_mask_bridge 2), Batch_1
(starved_thermal 1), eurorack-pmod-pcb (solder_mask_bridge 1),
sbc_allwinner_a13 module (items_not_allowed 1). No board fails the
strict rule with nothing open. Of the 39 that fail, 18 are within five
connections and five errors of clean, 11 leave more than 20 open, 3 have
no result.

### Placement or routing on one binary (matched designer tier, nightly 2026-11-10)

`build/nightly/2026-11-10/designer.log`: 109 boards routed by the same
binary on the designer's placement at `--budget-scale 0.613` (the median
final-ladder share of the previous night's sweep, `nightly-report.py
--ladder-share`; 919.5 s of work), `--quality`, 8 jobs x 1 thread,
`--order peak-desc`: 5302 s wall, largest peak 2.8 GB, no row cut or
killed, 109 with a result. **75 of 109 pass the per-type rule, 76 the
strict rule**; 2672 unconnected in all; strict errors 9; medians against
the designer's own board: vias x0.50, length x1.00, bends x0.83,
stair-steps +1.

`compare.py --gap build/nightly/2026-11-10/sweep.log
build/nightly/2026-11-10/designer.log --targets` over the **100 target
boards with both results** (of 100): **layout open 706, designer-route
open 233: placement gap +473 (67 %), routing gap 233 (33 %)**; ours is
worse on 24, the same on 69, better on 7. MegaDrive dominates the
routing side (the designer route 71 open, ours 57); without it 649
against 162: placement 75 %, routing 25 %. Over all 106 boards with both
results (the panels included): 1552 against 1661, placement -7 %,
routing 107 %. The previous night's split (2026-11-09, 100 target
boards): 68 % / 32 %.

**The five boards whose placement costs most** (ours / designer route, open connections):

| Board | Connections | Layout open | Designer route open | Placement gap |
| --- | --- | --- | --- | --- |
| Batch_1 | 254 | 142 | 2 | +140 |
| protoesp | 104 | 87 | 1 | +86 |
| glasgow revC3 | 226 | 85 | 17 | +68 |
| OpenESC-30x30 4in1 | 152 | 79 | 12 | +67 |
| 4in1-mini | 152 | 88 | 37 | +51 |

Next: SmartSpin2k_Panelized +51 (51 / 0), eurorack-pmod-pcb +17 (18 /
1), PCB-Modular-Multi-Protocol-Hub Hub +16 (18 / 2), laptop power +9 (9
/ 0), RP2040_base +6 (11 / 5). Routing first (the designer route leaves
as much or more): MegaDrive 71 / ours 57, SNSP-CPU-01 42 / ours 2, katia
15 / ours 3, reCamera_S101_v1.1 4 / ours 4, vaio_re 3 / ours 1,
jetson-nano-baseboard 2 / ours 1 and 3 more.

**Worse than the nightly of 2026-11-09, beyond noise** (the next triage;
a layout is deterministic for its binary, so every change is the
binary's; on the dense boards of `dense-noise.json` a loss counts only
when it exceeds the spread of the board's unconnected count over the
placer seeds there), 0: none.
