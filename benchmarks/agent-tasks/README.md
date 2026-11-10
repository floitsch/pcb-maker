# Agent layout tasks

Can pcb-maker turn what an agent has into a finished board? Each task
starts where an agent starts:

- a real board with its tracks stripped;
- usually every movable footprint stacked at one point and no outline, the
  way a fresh netlist import looks;
- a `constraints.json` like an agent would write ([docs/constraints.md](../../docs/constraints.md)).

Then `layout-kicad-board` places and routes it.

**Pass criteria**, all of them:
- every constraint holds, except those the task lists as geometrically
  impossible;
- every connection is routed;
- native KiCad reports no unconnected item and no copper error beyond the
  designer's board's (`pass`; the stricter rule is in "How to measure").

```sh
benchmarks/agent-tasks/run.py build/agent-tasks
```

`run.py --help` lists every option. The current numbers of the tiers and
the nightly are in [docs/benchmarks.md](../../docs/benchmarks.md).

## Files

| File | What it is for |
| --- | --- |
| `run.py` | The runner: prepares each task, runs the binary, judges the result with KiCad, writes one JSON row per board (the log), `results.json` and `summary.json`. |
| `compare.py` | Two runs board by board (better / same / worse, the objectives, the normalised measures); `--gap` splits a layout run's open connections with a designer-tier run. |
| `rejudge.py` | Judges a stored run again under the current strict rule, without routing. |
| `quality.py` | Score measures of a run's boards; `--designer-numbers` writes `designer-numbers.json`. |
| `tier_estimates.py` | Writes `estimates.json`: each board's expected peak memory and seconds per kind of run. |
| `placer_ab.py` | Judges a placer A/B over seed pairs on the dense boards against `dense-noise.json`. |
| `flippers.py` | Counts how often each board's verdict flipped over a series of logs (how the short tier was chosen). |
| `from_pcbench.py` | Turns PCBench/D3 or harvested GitHub boards into tasks. |
| `from_expansion.py` | Writes the expansion tier's `expansion.json` and `expansion-tasks.json`. |
| `schematic.py` | The schematic tier's steps. |
| `ablation.py`, `legality_census.py` | Placement ablation and the legality census (docs/placer.md). |
| `tasks.json` | The three hand-written tasks below (the default list). |
| `tasks/v7` | The harvested corpus as 109 tasks: every tier's default list (`tasks/v6` differs only in six panels). |
| `quick.json`, `short.json`, `schematic.json`, `expansion.json`, `designer.json` | The tiers: boards, task list, budget scale, and per board its reasons and estimates. |
| `estimates.json`, `designer-numbers.json`, `dense-noise.json` | Per-board estimates; the designer's vias, length, pour cut and aesthetics; the seed noise of the eleven dense boards. |
| `connectivity-tasks.json`, `rules-tasks.json` | Route-mode task lists of two experiments (experiments/kicad-connectivity, experiments/project-rules). |

## Tasks

| Task | What the agent asks for |
| --- | --- |
| `hierarchy-from-netlist` | KiCad's complex-hierarchy demo from a netlist: a 100 × 80 mm board, power connectors on the left edge, signal connectors on the right, a decoupling capacitor at its amplifier, a ground plane on the top. |
| `esp32-dut-from-netlist` | An ESP32-C3 test board from a netlist: 65 × 56 mm, the controller module on the left edge, the device under test on the right, service headers on the top edge, the system header flush along the bottom, the power header on the left, decoupling within 4 mm of each module. |
| `esp-usb-corner` | The breadboard fence's USB/ESP corner: an ESP32-C3 module flush in the corner, its antenna keepout beyond the top edge; a USB-C receptacle below it, opening outwards; the regulator between them; capacitors at their pins. Two distance limits there are impossible (a pad 8.4 mm inside the module's courtyard) and are expected to be missed. |

A task that stacks its footprints (`unplace`) also takes off the board
the texts and silkscreen graphics that label a stacked part (by the
binary's attachment rule, docs/placer.md): left at the designer's spot
they would be obstacles for parts that are no longer there. Texts over
fixed parts and over free space stay. Rows record `labels_dropped`.

## Tasks generated from PCBench and GitHub

`from_pcbench.py` turns PCBench/D3 boards ([../pcbench/README.md](../pcbench/README.md)
prepares them) or the harvested GitHub boards (`--corpus
benchmarks/github/boards.json`) into tasks:

- **Kept from the design:** the outline, and every connector the designer
  placed within 2 mm of an edge, on that edge and in its orientation (a
  repeated reference, as on a panel, is named by `uuid`).
- **Kept where they are:** parts hanging over the outline, and parts whose
  courtyard holds other parts (a shield, a module above parts), which
  become `hollow`: other parts may sit inside them.
- **Placed from scratch** (`move_all`): everything else, from one stack.

```sh
benchmarks/agent-tasks/from_pcbench.py build/agent-pcbench --subset d3-test
benchmarks/agent-tasks/run.py build/agent-pcbench-run --tasks build/agent-pcbench/tasks.json --jobs 3
```

The harvested corpus's list is committed as `tasks/v7`. It keeps the main
checkout's absolute board directories (so that its SHA-256 stays the
reference logs'); run.py finds the same `benchmarks/real/external/...` path
under its own checkout where that one is not there.

## How to measure

A layout is deterministic for a given input and binary, whatever its
thread count, unless the wall clock cuts it (`cut`, compare.py's CUT: no
evidence). One run per side answers a quality question. run.py pins the
work rates at the reference's (`PCB_WORK_RATE=3.15e6,150e6`; `--work-rate
measured` lets each binary measure its machine), and with pinned rates a
binary stretches its wall-clock guards by up to four times on a loaded
machine (`wall_stretch`) instead of cutting rows (docs/router.md).

- **Binaries and worktrees.** Work in a worktree of your own
  (`bash scripts/worktree.sh <name> <branch>`: `../pcb-maker-<name>` with
  the main checkout's `build`, `artifacts` and harvested boards linked in),
  run `bash scripts/check.sh` before a commit, and benchmark a copied
  binary: `bash scripts/snapshot.sh <n>` builds it and copies it to
  `build/bin/pcb-maker-x<n>`, refusing a number that is taken. Never
  benchmark `target/release`, which the next build replaces. The current
  references are in docs/handover.md.
- **What to run.** Iterate on the short tier. A router package runs the
  fingerprints and the route-mode expansion tier, a placer package the
  short tier too, each against main's numbers. The quick tier at the full
  budget is a wider check of a layout change, diffed with `compare.py`
  against its last run; it is not the merge gate. Full sweeps and the
  designer tier run nightly on main, never per package.
- **Judge by continuous objectives from one run**, not by pass counts,
  which move by single-error flips. The run's last line (also
  `summary.json`) and compare.py give KiCad's unconnected items, new copper
  errors, vias, length, CPU seconds, work seconds, cut rows and, for
  layouts, the router's open terminals after the first route
  (`first_unconnected_terminals`), summed over the boards both runs have.
- **Normalised against the designer:** raw sums are dominated by a few
  large boards, so summary.json's `normalised` and compare.py give
  per-board medians against `designer-numbers.json`: vias and track length
  over the designer's, unconnected items over the board's connections, the
  ground pours' cut and the aesthetics (docs/quality.md) less the
  designer's. The last two need the result scored: `run.py --quality`, or
  `compare.py --score <binary>` for older runs. run.py gives
  score-kicad-board `SCORE_TIMEOUT_SECONDS` a board (on the designer's
  board for its vias in pads, and for `--quality`); past it the score is
  missing and the row's `score_missing` says which side and why.
- **The strict rule** (`pass_strict`, `new_findings_strict`; run.py's
  `strict_findings`, docs/agents.md): a KiCad finding of ours counts
  unless the designer's board with its tracks, vias and small zones
  stripped (`baseline-drc.json`) has it at the same place: the same type
  and items the same distances apart, or items at the same positions
  within 0.05 mm, each designer finding answering for one. Findings
  between items of one footprint (`footprint_own`) are the designer's, and
  so are mask bridges at an opening that bridges on the designer's boards
  already (`strict_mask_openings`). Rows record the rule's version
  (`strict_rule`, now 4; run.py's `STRICT_RULE` has the details);
  compare.py marks a count over several MIXED RULES, and `rejudge.py
  <log>[:<dir>] ... [--output FILE]` judges a stored run again.
- **A killed run is no result:** a binary that exits with none of its
  verdict's codes (0 to 4) gets a row with `error`, `killed` and `cut:
  ["killed"]`, nothing it left judged (compare.py: KILLED). Each binary runs
  in a process group of its own; a timeout kills the group, kicad-cli with
  it.
- **Rows say what made them:** `binary_sha256`, `strict_rule`, and the
  input's digests (`tasks_sha256`, `constraints_sha256`, `task_sha256`,
  `board_sha256`, `project_sha256`). compare.py marks a board whose input
  differs INPUT, out of better / same / worse. Pass `--tasks` on both
  sides or neither.
- **Placement or routing?** `compare.py --gap <layout log> <designer log>`
  splits a layout's open connections into a placement gap and a routing
  gap (the designer tier below). Run it before deciding which side the next
  work goes to. Router questions go to route mode (`--route-from`).
- **CPU claims:** pinned simultaneous pairs only, both binaries at once on
  symmetric halves of the cores (`taskset -c 0-7,16-23` against
  `8-15,24-31`: logical CPUs n and n+16 share a physical core), CPU seconds
  from rusage (`cpu_seconds`), never wall seconds. KiCad DRC reports are
  cached (`~/.cache/pcb-maker/drc`) and the side that checks a board first
  pays for it, so CPU pairs run with `PCB_MAKER_NO_CACHE=1` on both sides,
  or both warm. Quality comparisons are unaffected.
- **Placer defaults need seed pairs:** a placer change reshuffles crowded
  boards at random, so it is judged on five seed pairs (`--placer
  '{"seed": N}'`, N = 1, 101, 201, 301, 401) on the eleven dense boards of
  `dense-noise.json` by `placer_ab.py compare --off DIR... --on DIR...`:
  better at p <= 0.03; at 0.03 < p <= 0.25 four more pairs (501 to 801) and
  better at p <= 0.03 over the nine.
- **CI** guards the code, not the benchmarks: tiers run here.

**The memory ledger.** Runs go side by side. Every route, layout and KiCad
check reserves its memory in the machine's ledger
(`$XDG_RUNTIME_DIR/pcb-maker-ledger.json`) before it allocates and waits
while the reservations exceed its capacity; `pcb-maker ledger` lists who
holds what. run.py admits each board by the same ledger with its expected
peak from `estimates.json`; a row the ledger ended (exit 4) runs once more
(`ledger_retry`). `--ledger-drain-after`, `--ledger-drain-max` and
`--ledger-priority` order the waiting boards, `--min-free-gb` sets a floor
beside the ledger (by default measured: `python3
benchmarks/ledger_headroom.py`), and `--lock` takes `build/quick-tier.lock`
for a wall or CPU measurement that must not share the machine. Details:
[docs/benchmarks.md](../../docs/benchmarks.md).

**Parallelism.** A tier given neither `--jobs` nor `--threads` runs 2
search threads a board (`TIER_THREADS`) and the physical cores / 2 boards
at once (8 x 2 on 16 cores). Rows compare across thread counts as long as
none is CUT; CPU and wall seconds do not.

**Per-board estimates.** `estimates.json` gives each board's expected peak
and seconds per kind of run (`layout`, `short`, `schematic`, `designer`,
`expansion`): the largest `peak_rss_mb` of the newest three sources times
1.25, and their largest seconds. `--expected-gb` overrides it, `--dry-run`
prints the start order with each board's declaration and runs nothing,
`--order` is `seconds-desc` (the default) or `peak-desc`. Refresh it after a
run whose rows should count (`tier_estimates.py --refresh --layout <log>`;
also `--short`, `--schematic`, `--designer`, `--expansion`, `--night
<dir>`) and commit it.

## The quick tier (`--tier quick`)

A wider check of a layout change than the short tier: 30 harvested boards that lay out in about 1100 s or less
and peak at about 3.7 GB or less (KiCad's DRC alone takes 2.2 GB), run side
by side at the full budget, about half an hour at 8 jobs.

```sh
benchmarks/agent-tasks/run.py build/quick-xNNNN --tier quick \
  --binary build/bin/pcb-maker-xNNNN > build/quick-xNNNN.log
benchmarks/agent-tasks/compare.py build/quick-xMMMM.log build/quick-xNNNN.log --markdown
```

`quick.json` lists the boards (task names in `tasks/v7`) with their layers,
the failure kinds each covers, and the time, peak and result of the run
they were chosen from: 20 two-layer, 8 four-layer and 2 six-layer boards,
passing and failing ones, framework_mobo_lefthalf as the cheap case the
placer finds no legal placement for. Panels, non-targets, more than three
boards of a family and the slow or large boards stay for the full sweep.

## The short tier (`--tier short`)

For iterating on a change: 12 boards of the quick tier at a third of the
work budget, about seven minutes at the tier default (8 x 2).

```sh
benchmarks/agent-tasks/run.py build/short-xNNNN --tier short \
  --binary build/bin/pcb-maker-xNNNN > build/short-xNNNN.log
benchmarks/agent-tasks/compare.py build/short-xMMMM.log build/short-xNNNN.log
```

**Selection, by flip history.** `flippers.py <logs...> --markdown` counts,
per board over a series of quick-tier logs, how often its pass verdict,
KiCad's unconnected count and its copper error count changed from one
round to the next. The boards that flip carry the signal; the boards that
never flip would only spend the budget. Chosen over 12 rounds: every board
with two flips or more (nine), the fastest with one (spell_tome), and two
fast boards that never changed as controls (Blue_Line, urchin).
MIDAS-MK1.1 and WLED_Matrix are the slow ones kept: both had connections
open in every round. 8 two-layer, 2 four-layer, 2 six-layer boards.

**Budget.** `short.json`'s `budget_scale` (0.3333; `--budget-scale`
overrides it) scales `total_seconds` 1500 to 500, `move_seconds` 600 to 200
and `polish_seconds` 60 to 20; the harness's timeout is the budget times
the wall guard (2) times the wall-stretch allowance, plus 300 s, at least
1800 s. The scale bites where the final ladder runs, which is where the
flips are: compare short-tier runs with short-tier runs.

## The schematic tier (`--tier schematic`)

The other tiers start from the designer's board with its parts stacked. An
agent starts from the KiCad project. This tier runs the documented flow
(docs/agents.md) on 12 harvested boards from their schematics
([../github/README.md](../github/README.md), "Schematics"), so that the
import, the libraries and the constraints are measured with the layout:

1. **Import** of the fetched project (`benchmarks/real/external/github-sch/<board>/repo`)
   with `import-kicad-netlist <root sheet> <project> <board-id> --layers N
   --footprints-from <designer's board>`. Libraries `harvest_schematics.py`
   found elsewhere in the repository go into the project's `fp-lib-table`
   and the import runs again, as agents.md tells an agent to do.
2. **Constraints.** `schematic.py` moves the task's constraints onto the
   new footprints (by symbol `path`, else by a unique reference) and copies
   the designer's outline, zones and board-only footprints (those with nets
   as `add` entries), keeping the designer's ids so that the project's DRC
   exclusions still apply.
3. **Layout** with `layout-kicad-board <project> <board-id> <out> auto
   <project>/layout.json` at a third of the budget, then `drc-kicad-board`.

`--import-near` also adds the import's decoupling `near` entries
(`--near-decoupling`); off by default.

**Pass rule:** the layout's, and the import found every footprint. KiCad's
schematic parity findings (`parity`) are recorded, not judged. A row adds
`stopped` and `why` (the first stage that fell short: `import`,
`constraints`, `placement`, `routing` or `drc`), the import's report, the
footprints taken from the board and the parts matched, copied and added;
the run ends with the stages' counts (`stopped at: ...`).

```sh
benchmarks/agent-tasks/run.py build/sch-xNNNN --tier schematic \
  --binary build/bin/pcb-maker-xNNNN > build/sch-xNNNN.log
```

**Selection** (`schematic.json`, each board's `why`): boards whose import
is complete after the library step, 25 to 443 parts, 8 two-layer, 2
four-layer and 2 six-layer, covering a KiCad 6 project, board-only holes
and logos, a library outside the project, substituted footprints,
references renamed after the schematic and KiCad 5 libraries.

## The expansion tier (`--tier expansion`)

46 boards of the expansion intake ([benchmarks/expansion](../expansion/README.md)),
routed on the **designer's placement** with the tracks stripped, at the
intake's 30 seconds of work: a router tier over a wider spread of
designers, tools and hardness than the harvested boards. Each task is a
route-mode task (`"mode": "route"` in the task): no placement, no
constraints. Run it with `--jobs 8 --threads 1`, as the intake did.

```sh
benchmarks/agent-tasks/run.py build/expansion-xNNNN --tier expansion \
  --jobs 8 --threads 1 --binary build/bin/pcb-maker-xNNNN > build/expansion-xNNNN.log
# the intake's deep run (300 s of work)
benchmarks/agent-tasks/run.py build/expansion300-xNNNN --tier expansion \
  --budget-scale 10 --jobs 8 --threads 1 --binary build/bin/pcb-maker-xNNNN > build/expansion300-xNNNN.log
```

`expansion.json` and its task list `expansion-tasks.json` are written by
`from_expansion.py` from `benchmarks/expansion/analysis/`. The boards'
designer boards are native-DRC clean under their own rules, with a
licence in harvest.py's `USABLE_LICENSES`, one per repository, 27 2-layer,
15 4-layer and 4 6-layer, 44 to 1887 pads, in five buckets:

| Bucket | Rule (the intake's outcomes) | Boards |
| --- | --- | ---: |
| B1 clean beyond 30 s | complete and native-clean after more than 30 s of routing | 12 |
| B2 near | incomplete at 30 s, 1-2 connections open | 9 |
| B3 far | incomplete at 30 s with 3+ open, or cut by the wall guard | 12 |
| B4 clean only at 300 s | incomplete or cut at 30 s, native-clean at 300 s | 9 |
| B5 gives up early | `incomplete_early` (the router stops with connections open) | 4 |

**The fetch and the licence rule.** A task carries its source (repository,
pinned commit, path, licence, each file's SHA-256). run.py copies the files
from the harvest (`--harvest-data`, default `benchmarks/expansion/data`)
when the digests match, else fetches them from GitHub at the pinned commit,
verifies each before use (a mismatch fails the row: `fetch_error`) and runs
nothing in them. It refuses a board whose licence is missing, empty or
`NOASSERTION`. The boards are for local benchmarking only.

**What a task runs**, as the intake did (benchmarks/expansion/triage.py):
`strip-kicad-tracks`, `route-kicad-board ... --reference none` with the
task's router settings and `ladder_budget_seconds` = 30 s times the budget
scale, then `drc-kicad-board`. The row is a route row with `license`,
`fetched`, `open` and `outcome`, the intake's class (`clean_fast`,
`clean_medium`, `clean_slow`, `cut_wall_clock`, `incomplete_at_budget`,
`incomplete_early`, ...); the run ends with an `outcomes:` line.

## The designer tier (`--tier designer`)

**The deciding experiment for placement versus routing work.** A layout
that leaves connections open mixes two causes: a placement that cannot be
routed, and a router that cannot route what a human could. The designer
tier routes every board of the corpus (tasks/v7, `designer.json`) on the
designer's own placement, and `compare.py --gap` joins it with a layout
run of the same list:

- **routing gap** = KiCad's unconnected items of the route on the
  designer's placement. Router work moves it.
- **placement gap** = the layout's unconnected items less the routing gap
  (negative where our placement routes better). Placement work moves it.

Each board is the designer's project with its tracks and vias stripped as
the layout tier strips them, routed by `route-kicad-board` with the
project's rules, the task's router settings and the layout's budget
(`route_budget_seconds`, 1500 s of work), judged by the layout's pass rule
(rows with `placement: "designer"`). Compare with a layout run of the same
binary and list; the gap is only as sound as the rows are uncut.

**Matched budgets.** In a layout the placement race and the moves take
their share of `total_seconds` first and the final ladder gets the rest,
so routing the designer's placement with the whole 1500 s would give our
router more work than the layout's had. Layout rows carry the final
ladder's budget (`ladder_budget_seconds`, `ladder_budget_share`) and
compare.py --gap prints both sides'. The matched comparison routes at the
layout run's median share, `--budget-scale <share>` (`scripts/nightly-report.py
--ladder-share <sweep.log>` computes it; 0.695 without a sweep):

```sh
benchmarks/agent-tasks/run.py build/designer-xNNNN-matched --tier designer --binary build/bin/pcb-maker-xNNNN \
  --budget-scale 0.695 --quality --jobs 8 --threads 1 > build/designer-xNNNN-matched.log
benchmarks/agent-tasks/compare.py --gap build/github-layout-vNN.log build/designer-xNNNN-matched.log --targets
```

The layout's router also had the first route, and splitting by subtraction
assumes the causes add up: the split is an accounting of where the open
connections are, not a measured cause.

## Route mode (`--route-from`)

For router experiments: `--route-from <earlier output directory>` routes
each board's placement from that run (`<dir>/<task>/layout/placed`) with
`route-kicad-board` instead of laying it out: no placement race, the same
placement for both binaries. The task's router settings go with it,
`--budget-scale` scales `ladder_budget_seconds` (default 1200 s of work;
with `--tier short` the tier's third applies), and the row and pass rule
are the layout's with `mode: "route"`, `route_from`, `constraints: 0` and
`work_seconds`.

```sh
benchmarks/agent-tasks/run.py build/route-xNNNN --tier short \
  --route-from build/quick30-xMMMM --binary build/bin/pcb-maker-xNNNN > build/route-xNNNN.log
```

## Incremental sweeps (`--splice-from`)

`--splice-from <log>` with `--only A B ...` runs only those boards and
writes a log (and results.json) where the tier's other rows are copied from
the given log or output directory, marked `spliced: true` and
`spliced_from`. The pass and objectives lines count the whole tier and say
how many rows are spliced; `compare.py` marks them SPL. Use it to rerun the
boards a change can touch, or the rows a run lost to a cut. A spliced row is
the old binary's: say so when the result is quoted. summary.json lists the
`parts` (per log) and `strict_rules` when the rows were judged under
several rules (`rejudge.py` judges them again under one).

```sh
benchmarks/agent-tasks/run.py build/quick-xNNNN-two --tier quick \
  --only bitshiftcrazy__spell_tome__spell_tome_bottom tomunderwood99__CharlieBoard__Blue_Line \
  --splice-from build/quick30-xMMMM.log --jobs 2 --binary build/bin/pcb-maker-xNNNN > build/quick-xNNNN-two.log
```

## Reused layouts (`--reuse`)

`--reuse <dir>` (layout mode) caches layouts by their placement. A layout
prints `layout: placement digest <sha256>` when its placement is done
(the placed board, the lowered problem with its placer settings, the
race's other placements, the pin swaps); run.py's key adds the binary, the
task, the layout settings, the rates, the threads and the `PCB_*`
environment. On a hit run.py stops the layout there and returns the earlier
row with `reused_from`. Finished rows are stored as `<key>.json` beside
their directory, which must stay where it is. It saves most of a dense
layout's wall clock on the boards a placer switch leaves alone (both sides
on one binary, the switch as an option) and on every rerun.

```sh
benchmarks/agent-tasks/run.py build/X-dense-s1-on --tasks benchmarks/agent-tasks/tasks/v7/tasks.json \
  --binary build/bin/pcb-maker-xNNNN --placer '{"seed": 1, "my_switch": true}' --reuse build/layout-reuse
```

## Nightly

The full layout sweep and the designer tier run on main every night:

```sh
bash scripts/nightly.sh                 # tonight's measurement, by hand
bash scripts/nightly.sh --quick         # without the sweep and the designer tier
bash scripts/nightly.sh --install       # once: the systemd user timer, 02:00 local
bash scripts/nightly.sh --print-units   # the units --install writes
scripts/nightly-report.py --print       # the newest night as a table row and a state line
scripts/nightly-report.py --docs build/nightly/<night> --dry-run   # the night's write-up, printed
```

`scripts/nightly.sh` refuses when the main checkout has uncommitted changes
to tracked files, builds main's head in a detached worktree of its own
(`../pcb-maker-nightly`), snapshots the binary as
`build/bin/pcb-maker-nightly-<date>-<sha12>` (the newest 7 stay) and runs
the stages one after the other with `--ledger-priority nightly`:

| Stage | Command (run.py) | Wall (the nights of 2026-11-09 and 10) |
| --- | --- | --- |
| fingerprints | Castor, k30-SBC, jetson, Sisu; `--route-from build/github-layout-v16 --tasks tasks/v7`, 1 x 4, `--budget-scale 0.25`, `PCB_ROUTER_FINGERPRINT=1 PCB_ROUTER_NO_WALL_GUARD=1` | 8-9 min |
| expansion | `--tier expansion`, 8 x 1 | 2 min |
| short | `--tier short`, 3 x 2 | 8 min |
| designer | `--tier designer --quality`, 8 x 1, `--order peak-desc`, `--budget-scale` the median `ladder_budget_share` of the newest earlier night's sweep (0.695 without one) | 1.5 h |
| sweep | `--tasks tasks/v7`, 10 x 2, `--quality`, `--order peak-desc` | 2.5 h |

The designer tier runs before the sweep, largest boards first, while the
machine is quiet. Every stage takes its boards' estimates from
`build/nightly/estimates.json` (the committed `estimates.json` on the first
night), which `tier_estimates.py --refresh` updates with the night's rows
after the summary.

**Options.** The sweep and the designer tier run on the days of
`--sweep-days` (`PCB_NIGHTLY_SWEEP_DAYS`: `all`, the default, or ISO
weekdays `1`-`7` / `mon`-`sun`, comma separated); `--no-sweep`,
`--no-designer` and `--quick` leave them out for a night. When the newest
nightly measured the same commit with every stage planned, the night does
nothing (`--force` runs anyway). One night at a time (`build/nightly/.lock`).

**What a night leaves**, in `build/nightly/<date>/`: each stage's output
directory, `<stage>.log` (the rows) and `<stage>.err`, `nightly.log`,
`gap.txt` (`compare.py --gap --targets`), `diff-<stage>.txt` (compare.py
against the previous night that ran the stage), `nightly.json` and
`summary.md`. Nights beyond the newest 14 keep only their logs and
summaries.

**Into the docs** (on by default; `--no-commit-report` or
`PCB_NIGHTLY_COMMIT_REPORT=0` turns it off). `scripts/nightly-report.py
--commit` writes the night into the main checkout: a "Layout sweep vN"
section in docs/benchmarks.md (only the two newest are kept) and a row of
its "Nightly" table, the README's headline bullet and its two table rows,
and docs/handover.md's title, state line and night note. It commits those
files alone on main and pushes. A night is published only when every stage
ran, no row was cut or killed and the sweep has at least 100 rows with a
result, and only when the main checkout is clean on main and `git pull
--ff-only` succeeds (else the next night publishes both).

**The timer.** `--install` writes `scripts/nightly.service` and
`scripts/nightly.timer` to `~/.config/systemd/user/pcb-maker-nightly.*` and
enables the timer (without `systemctl --user` it prints a cron line); it
needs `loginctl enable-linger` to run while the user is logged out.
`scripts/test_nightly.py` tests the script with run.py replaced by a stub.
