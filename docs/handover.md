# Hand-over (state as of 2026-11-10; measured by layout sweep v37, the nightly of 2026-11-10 on main 8bfeb31)

Read this first. It says what the tool does today, which benchmarks
count, what is known to be wrong, how to work on the code and what still
bites. [architecture.md](architecture.md) maps the code: the crates, the
stages of a layout and a route, and the file that owns each.
[config-reference.md](config-reference.md) lists every setting with its
default (generated from the code). The history of each change is in git
(`git log`, `git log -S <name>`); this file keeps only the current state.

## What the tool does today

pcb-maker places and routes KiCad 10 boards for agents. The agent
commands (`pcb-maker --help`; the contract in [agents.md](agents.md)) are
`import-kicad-netlist` (a board from a schematic, with starter
constraints), `describe-kicad-board`, `layout-kicad-board` (place and
route), `place-kicad-board` (the layout's placement alone),
`route-kicad-board` (route a placed board), `inspect-routing-run`,
`drc-kicad-board`, `fab-kicad-board` (a release gated by KiCad's checks)
and a few tools around them. Every run ends with KiCad's own ERC, DRC
and connectivity, a `verdict.json` and an exit code (0 complete, 1
incomplete, 2 input error, 3 cut by the wall clock, 4 internal error,
the memory ledger's abort included). KiCad decides pass or fail, not the router's own count.

Budgets are counted in work (search expansions and placement units at
rates measured at the start, or pinned with `PCB_WORK_RATE`), not in
wall time, so a run is deterministic for its input, binary and rates
whatever the thread count, unless the wall-clock guard (`wall_guard`,
twice the budget) cuts it. A layout's default budget is 1500 s of work:
the placement race gets up to 40 %, the moves up to 65 % of the total,
the final ladder what is left.

What a layout does by default ([architecture.md](architecture.md) has
each stage with its file): it places up to sixteen seeds (a quadratic
start from the netlist, an electrostatic global placement in which banks
on a wide bus move as one body, an anneal with the ratsnest crossings in
its cost, legalisation and refinement; with 500 s of placement work or
more, the race over skeletons seats the active parts before the
passives), ranks them by RUDY congestion, probes the best three with a
short route and keeps the one with the fewest open terminals after the
rip-up. It then routes with the lattice router (PathFinder negotiation,
pours as KiCad 10 fills them with thermal spokes, dog-bones for grid
arrays, corridor stubs for pocketed pads, lattice lines at fine-pitch
rows where a probe leaves them open), tries footprint moves where nets
fought, and runs route mode's attempt ladder while connections stay
open. KiCad's connection-width findings are then repaired with KiCad's
own fill, starved thermals get a solid connection where KiCad finds the
board better, and `length` and `skew` rules are kept by meanders.

Off by default, each a setting: blind, buried and micro vias
(`blind_vias`), differential pairs in a layout (`diff_pairs`; route mode
turns them on where the project's rules ask for pairs), `pad_exits`,
`pair_rows`, `length_minima`, `length_reroute`, the topological engine
(`engine: "topological"`). [config-reference.md](config-reference.md)
has each setting with its doc comment.

## Benchmarks that count

The nightly (`scripts/nightly.sh`, a systemd user timer at 02:00) runs
every stage on main's head and writes `build/nightly/<date>/summary.md`.
The line below and the next paragraph are rewritten by the nightly;
[benchmarks.md](benchmarks.md) has the table of the last 14 nights.

<!-- nightly-state:begin -->
**Nightly** 2026-11-10 (main 8bfeb313afbf, `build/nightly/2026-11-10/summary.md`): fingerprints identical; expansion 26/46 (strict 26), 556 open; short 9/12 (strict 9), 16 open; sweep 69/109 (strict 70), 1552 open; designer 75/109 (strict 76), 2672 open at 0.613; gap placement 67%, routing 33%.
<!-- nightly-state:end -->

**Layout sweep v37** (the nightly of 2026-11-10, main 8bfeb31, written
by the nightly): **70 of 109 pass the strict rule** (69 the per-type
rule), 1552 unconnected in all, strict errors 15, no row cut or killed,
3 without a result; medians against the designer's board: vias x0.65,
length x0.98, open 0.000 (mean 0.063), bends x1.01, stair-steps +33.5,
crossings +1. Against the nightly of 2026-11-09: better 0, same 106,
worse 0, strict passes 70 -> 70. The matched designer tier on the same
binary (budget scale 0.613): 76 strict passes (75 per type); over the
100 target boards placement 67 %, routing 33 % of the open connections
(placement 75 %, routing 25 % without MegaDrive).
`build/nightly/2026-11-10/`; table, gap split, the costliest placements
and the losers in
[benchmarks.md](benchmarks.md#layout-sweep-v37-2026-11-10).

The stages, all on the committed task list `benchmarks/agent-tasks/tasks/v7`:

| Stage | Boards | What it measures |
| --- | ---: | --- |
| Layout sweep | 109 | Full layouts from stacked parts and constraints, 1500 s of work each, 10 jobs x 2 threads. The headline: strict passes (0 unconnected, no copper error beyond the designer's own at the same place). |
| Matched designer tier | 109 | The same binary routing the designer's placement at the layout's routing budget (the previous sweep's median final-ladder share). |
| The gap split | 100 | `compare.py --gap --targets` over the sweep and the designer tier: how much of what stays open is placement and how much routing. |
| Expansion tier | 46 | Route mode on the designer's placement, 8 x 1. |
| Short tier | 12 | Boards that flip, a third of the budget, 3 x 2. A package's iteration tier. |
| Fingerprints | 4 | Castor, k30-SBC, jetson, Sisu in route mode at a quarter budget with `PCB_ROUTER_FINGERPRINT=1`: identical night to night unless the routing changed. |

Not run by the nightly: the quick tier (30 boards at the full budget)
and the schematic tier (12 boards from the designer's schematic). Run
them when a change touches their path. The head-to-heads against
Freerouting and KiCadRoutingTools are complete and are not rerun
(README.md, "Status").

How to run a tier (each board is admitted against the machine's memory
ledger; keep the load under 40; the nightly's own arguments are in
`scripts/nightly.sh`):

```sh
B=build/bin/pcb-maker-x<n>
benchmarks/agent-tasks/run.py build/short-x<n> --tier short --jobs 3 --threads 2 --binary $B > build/short-x<n>.log
benchmarks/agent-tasks/run.py build/exp-x<n> --tier expansion --jobs 8 --threads 1 --binary $B > build/exp-x<n>.log
benchmarks/agent-tasks/compare.py build/nightly/<date>/short.log build/short-x<n>.log
```

Router questions are measured in route mode on fixed placements
(`--route-from <earlier output dir>`), CPU claims in pinned simultaneous
pairs. The discipline is in benchmarks/agent-tasks/README.md, "How to
measure".

## Not targets

compare.py's `NOT_TARGETS` leaves nine boards out of the gap split: the
five PolyKybd molecule panels (tile panels with duplicated references:
SW1 sixteen times), the OpenESC 4in1 panel and the OpenRX panel, and
krishveercard and SUMEC, whose own rules leave pads no track of their
class can enter. They stay in the sweep and are not worked on.

## Open problems, ranked

The direction (Florian, 2026-11-04): placement is most of what stays
open, so one placement bet at a time, each judged by the placer-default
protocol (below); a lever that loses or is null is removed, not kept as
an off switch; a fresh-agent trial every few days.

1. **Placement: two thirds of what stays open.** On the 100 target
   boards our layouts leave 706 connections open where the same binary
   routing the designer's placement leaves 233 (nightly of 2026-11-10).
   The costliest: Batch_1 (142 against 2), protoesp (87 / 1), Glasgow
   (85 / 17), the OpenESC 4in1 (79 / 12), the 4in1-mini (88 / 37),
   SmartSpin2k (51 / 0). Pinning the designer's non-passive parts (about
   a quarter of the movable parts) recovers most of the designer's
   first-route advantage, and our objective accepts that arrangement: the
   search does not reach it. Two losses:
   - *The coarse arrangement* is decided by the global placement. Its
     quadratic start from the netlist, the race over skeletons and the
     glued bus banks each moved it nearer the designer's. Open: the
     4in1-mini's skeletons, which only seat at the courtyard level (its
     22 bulk capacitors); the anneal taking a glued capacitor bank apart;
     a judge that tells boards whose passives route better at the full
     halos (DSP-ADAU1452) from those that route worse (SNSP-CPU-01): the
     passives start at level 2 (`PASSIVES_FIRST_LEVEL`) for all.
   - *The fine arrangement* is lost by the anneal's objective on the
     densest boards: on the 4in1 and Glasgow the anneal lowers our
     objective while the routes get worse, and the decoupling relations
     are most of that objective. Skipping the anneal is worse on most
     boards. The lever left is the relations' weight
     (`constraint_weight`).
   - `pad_exits` (off) passed the first-route screen on its four boards
     (-3.0 %, p 0.228) but the routes do not connect most pads it opens.
     It waits for the full protocol.
2. **Deciding a placer default costs a day.** The full protocol is about
   110 layouts per lever (88 CPU hours, 18 hours of wall beside the
   nightly); the first-route screen costs about half of it. Open: a
   stage that keeps the placement's budget and cuts only the routing.
3. **The router's ceiling.** The matched designer tier fails 33 boards
   under the strict rule. The last census of its failures
   ([router.md](router.md)) found about a third the
   input's or the budget's, and of ours congestion first (a residue that
   single-net exchanges only trade), then pad access and fine-pitch rows,
   plus a few plain bugs: the solid-thermal step runs once (k30-SBC,
   ghoul), copper texts lowered from their render cache (Quanta75), large
   enclosed SMD pads without a via (Sisu), a custom pad's anchor inside
   a neighbour's clearance (Hub).
4. **Crossings at fine-pitch rows.** Lattice lines at pin rows close the
   pads the regular lattice's phase held; route mode and a layout turn
   them on by themselves where probes leave pads open at missed rows.
   Left: crossing pairs on a row (a pad closed by a neighbour's via just
   beyond its tip; Quanta75's RP2040), pads with no straight way out at
   all (OpenAirScope's UDFN, OpenRX's U201/U301, DSP-ADAU1452), and a
   tight two-track channel (the keyboard's F23). Fixed copper before the
   negotiation costs more room than it gives (see "Tried and not taken");
   the crossing has to be decided inside the negotiation, for example by
   pricing a via's landing by what it displaces on the other layer.
5. **The ladder's judge.** The judge (unfinished nets after a 75 s probe)
   picks the best rung about three times in four; on some boards (Hub,
   DSP-ADAU1452, koeg-board, zpn_devboard) the probe is too short to
   tell. Since the leader runs first, a misjudged probe also decides
   which rungs run at all. The ladder stops at the first complete rung
   whatever its starved thermals: rank by (open, starved) before vias.
   A 30 s continuation per probed rung is the judge to try.
6. **Differential pairs cost connections in a layout.** With
   `diff_pairs` on, every routed stage of a layout plans the pairs, and
   on dense boards that costs connections (jetson 23 and 44 unconnected
   at two seeds where it had 0 and 9). Giving up a link that keeps
   conflicting (`PCB_ROUTER_DIFF_PAIRS_GIVE_UP`, off) halves that on
   jetson but loses coupled pairs on ducktop2. Until this is solved,
   `diff_pairs` stays off in a layout.
7. **Length minima without room.** Meanders meet a `length` minimum
   where the route has room beside it, not inside a dense bus (the
   firestarter's DVP bus: about half its nets within the rule). Open: a
   placer floor that keeps the connections, and room for meanders inside
   a bus on a fixed placement. An agent keeps a designer's bus with
   `strip-kicad-tracks --keep-nets`.
8. **Blind, buried and micro vias** (`blind_vias`, off). No task board
   allows a via span, so it is measured on the expansion intake's HDI
   boards, where route mode gains. Open: stacked and staggered chains at
   one cell, spans for pour nets, the micro via's own size in the maps,
   the `.kicad_dru`'s via-type rules.
9. **Fill fidelity.** Our pour map is in far more pieces than KiCad's
   fill (lattice sampling of necks). Not modelled: KiCad's teardrops,
   hatched iterative refill, custom spoke templates, per-zone fill
   settings. Necks and gaps are repaired after the route with KiCad's
   own fill. Starved thermals are the only failure on a few complete
   boards; `solid_starved_thermals` (on) connects such a pad solid where
   KiCad finds the board better for it, in one round only.
10. **The near misses.** Of the 39 boards the sweep fails, 18 are within
    five connections and five errors of clean. The list changes nightly;
    a diagnosis takes the night's list (`summary.md`, `diff-sweep.txt`).
11. **The agent flow's gaps.** A crowded side (90 to 99 %) that no exact
    test calls hopeless still runs the placer's levels to the budget; the
    placer does not choose sides by itself. Starters are missing for LED
    bars and other arrays, thumb clusters and column stagger. No tabs or
    mouse bites between outline pieces, and no automatic size for a
    polygon outline. A netlist board gets no router starter config (a
    power class for rails, a GND pour on two layers). Rows cannot turn
    every other line by half a turn (a bank whose facing pads share a
    net). A template's row without a `place` leaves parts without a
    place. `describe-kicad-board` does not say when a class holds no net
    while nets of its name sit in Default.
12. **The schematic import's friction.** Libraries outside the project
    found nowhere get a `missing` line without candidates further up;
    legacy `.mod` libraries are not read (convert them with `kicad-cli fp
    upgrade`); a board KiCad cannot load gives no message of its own.
13. **Wall-clock slack under load.** A work second is a wall second on
    an idle core, so the guard's margin is thin on a busy machine; KiCad
    checks are charged by a model (3.6 s plus 1.7 s per thousand hatch
    holes) that misses complex outlines (18 to 33 s a check on ULK,
    koeg, ghoul).
14. **Smaller threads.** The search is 70 to 80 % of route mode (the
    negotiation's stamp counts are the next speed target); the narrow
    step stalls at the price cap on DSP-ADAU1452; OpenESC gets hundreds
    of connection-width warnings from our own 0.09 mm tracks; the
    router's own open count differs from KiCad's (per net against per
    missing pair), which is why only KiCad's counts for passing.
15. **Stretch goal: Allegro-class constraints** (Florian, 2026-10-09).
    Differential pairs, length and skew rules, per-class widths and layer
    preferences exist; return-path awareness over plane splits does not.

## Tried and not taken

One line each, so that nobody builds them again without a new reason.
The code and the measurements are in git history (`git log -S <name>`).

- Net weightings (`rail`, `flat`): the first-route screen +5.6 % and
  +4.0 % (worse).
- Rows of repeated parts in the anneal (`align_rows`): +4.2 %.
- KiCad's legality at every placer level (`legality: "kicad"`): two of
  three seed pairs worse.
- Halos rescaled by RUDY, wire area as charge, shared halos, the
  legality gaps in the anneal: within the seed noise or worse.
- Reordering large parts after the global placement by crossings: the
  placed board came out the same.
- Skipping the anneal: +1.4 %, seven of eleven dense boards worse.
- A level guard on the race over skeletons: gave back the race's gains.
- Gluing every netlist stack (pairs, parts with three nets or more):
  SNSP-CPU-01, katia and RP2040_base worse; only banks on a wide bus
  are glued.
- Without the passives' stall stop: -2.2 %, p 0.17 (null).
- A per-board first level for the passives: no rule found.
- Pair rows that follow `diff_pairs`: more pairs coupled, more
  connections open.
- The placer's floor for length minima: lengths gained, connections
  lost; the length reroute: no length finding went.
- More placer seeds: no seed holds the designer's arrangement.
- The neural congestion predictor: ranks placements no better than RUDY.
- An exact pairwise overlap force, or a minimum-displacement relaxation
  in place of the anneal: not better than the seed spread.
- Fixed copper before the negotiation: the fan-out step, prepared
  fan-outs for fine-pitch rows, reserved corridors. Each traded which
  pad stays open or lost strict passes.
- The crossing dive in the trouble repair: closed the crossing and
  opened a third net.
- Ejection chains for the last connections (`last_connections`): closed
  none.
- Capacitated corridors from the global route: worse on every tier.
- Island stubs: changed no connection, only silenced KiCad's
  isolated-island finding.
- The final route started from the router's state: worse on the dense
  boards.
- The residue renegotiated in a window: stalls at the price cap where
  the board-wide negotiation does.
- Necking a blocked net down everywhere: replaced by the ladder's
  narrow-signal step.
- A plain pad leaving from the free part of its copper: no tier changed.
- As cheaper placer screens: a quarter budget (boards find no legal
  placement), condensed boards, coarse whole first routes.

## Working on it

Work is done in packages: one agent, one branch, one worktree, one
question. A coordinator merges the branches into main through a gate.

### A worktree per package

`bash scripts/worktree.sh <name> <branch>` creates `../pcb-maker-<name>`
on a new branch from main and links the main checkout's
`benchmarks/real/external`, `build` and `artifacts` into it, so run.py,
the tiers and `build/bin` work there as in the main checkout. Snapshot
binaries are `build/bin/pcb-maker-x<n>` and numbers are never reused:
the four-digit ones are main's, made by the gate; a package takes a free
range of ten five-digit numbers above the highest in use (or names its
builds by the package, `build/bin/pcb-maker-<name>-a`) and names its
range in its hand-off. `bash scripts/snapshot.sh <n>` builds and copies
one and refuses a number that is taken.

### The gate's rules

A package works in its worktree only. It never checks out, commits to,
merges, rebases onto or pushes main; the coordinator says when main
moved (rebase then). Stage paths by name (`git add docs/handover.md`),
never `git add -A` or `git commit -a`, and never commit anything under
`artifacts/`, `build/` or `benchmarks/real/external`: they are links into
the main checkout, and a committed link once wiped the harvested boards
on a merge (`tests/tracked_paths.rs` guards it). The gate runs `cargo
fmt --check` (the repository is rustfmt-clean), workspace clippy and
`bash scripts/check.sh`. A docs change must add no broken link among the
repository's own files (`python3 -I scripts/check_links.py`).

### Before a commit

`bash scripts/check.sh` runs the test set and builds the release binary
last (a cargo test of the library crates alone leaves
`target/release/pcb-maker` stale): every crate's tests, the root
binary's unit tests, the CLI tests, the fixture tests, the Python tests,
workspace clippy (`--all-targets`, warnings as errors) and the slow
tests that depend on the load (the agent walkthrough, the placer's
placement tests). `bash scripts/check.sh --fast` is the gate CI runs on
every push: the same without the slow tests, about 30 s once built. CI
runs the full set every night with KiCad 10 installed. Without kicad-cli
the KiCad tests skip and the script says so first; with it, run the
script under the machine's KiCad lock. Expect about five minutes in a
fresh worktree, half of it compiling.

### The fixture boards

`tests/fixtures/f1` is a small two-layer board in KiCad 10's format,
written by hand to hold what the tiers found the router and the lowering
getting wrong (a QFN with thermal vias, a net tie, stacked pads of one
number, an unplated slot, a padstack, a net named "14", a hatched zone,
net classes by regular expression, a `.kicad_dru` with a pair rule, and
more); `tests/fixtures/f2/layout.json` lays it out with a free edge.
`tests/fixtures.rs` routes and lays them out through the binary against
golden files in `tests/fixtures/golden` (fingerprints at 1 and 4
threads, the rules end to end, KiCad's connectivity);
`crates/pcb-kicad-route-place/src/tests/fixtures.rs` checks the lowering against a
golden table. A behaviour change shows as a golden diff:
`PCB_UPDATE_GOLDEN=1 cargo test --release -p pcb-maker --test fixtures`
(and `-p pcb-kicad-route-place fixture_tests`) rewrites them, to commit with the
change; the KiCad goldens need kicad-cli 10.0.7.

### What a package measures

A router package runs the fingerprints (route mode from
`build/github-layout-v16`, 1 x 4, budget scale 0.25,
`PCB_ROUTER_FINGERPRINT=1 PCB_ROUTER_NO_WALL_GUARD=1`; the exact command
is the `stage fingerprints` line of `scripts/nightly.sh`) and the
expansion tier, each against the newest nightly's log. A placer or
layout package adds the short tier and, for identity, the eleven dense
boards at seed 1. No package runs the full sweep or the designer tier:
the nightly does, and the first nightly after a merge checks a
package's claim about the sweep.

### Deciding a placer default

The protocol (benchmarks.md, "Measurement discipline: placer defaults"):
the eleven dense boards in layout mode at the task budgets (tasks v7,
pinned rates, 3 x 2, through the ledger), five seed pairs at placer seeds
1, 101, 201, 301, 401 (the race places seeds N to N + 15), judged by
`benchmarks/agent-tasks/placer_ab.py compare --off ... --on ...`: the
weighted relative change against the noise file
`benchmarks/agent-tasks/dense-noise.json`. BETTER at p <= 0.03; at 0.03 <
p <= 0.25 run seeds 501 to 801 and judge the nine pairs at p <= 0.03;
otherwise not better. The short tier must lose no strict pass and gain
no DRC error; a strict pass the short tier loses at its seed is checked
on that board alone at the five seed pairs, and holds when the on side
passes as often as the off side.

Screen first: the same five pairs with `--layout
'{"stop_after_first_route": true}'`, judged by `--metric first_open`
(`experiments/placement-gap/screen.py`); a lever that fails the screen
costs about half the full protocol. With the lever as an option on one
binary, run.py `--reuse <dir>` takes every row whose placement digest a
stored row has (benchmarks/agent-tasks/README.md, "Reused layouts"). When
main changes a dense board's layouts, its off rows and its noise-file
rows need running again.

### The memory ledger's rules

Routes, layouts and KiCad's checks reserve their memory in the machine's
ledger (`$XDG_RUNTIME_DIR/pcb-maker-ledger.json`; `pcb-maker ledger`
lists it) before they allocate, and wait for room. run.py declares each
board's expected peak from `benchmarks/agent-tasks/estimates.json`
(`--dry-run` prints each declaration and its source). An `--expected-gb`
given by hand overrides every board's: give it only from a measured peak,
never a round placeholder, and a harness of your own reserves what it
measured too. A reservation is held whether used or not, so a
placeholder far above use makes every other run and the nightly wait.
CPU pairs reserve for both sides at once and start both inside it
(`PCB_MAKER_LEDGER_INSIDE`); a pair whose one side waited is void.

## Where things are

| What | Where |
| --- | --- |
| Current state, open problems, how to work | this file |
| The code's map: crates, stages, files | [architecture.md](architecture.md) |
| The KiCad side's crates, bottom up | `crates/pcb-kicad-file` (KiCad's files as data), `pcb-kicad-check` (kicad-cli, the DRC cache, the ledger, the work rate), `pcb-kicad-route-place` (rules, lowering, the route and place flows), `pcb-kicad` (the layout, the verdict, the reports), beside it `pcb-kicad-import`, on top `pcb-kicad-lab`; each `lib.rs` says what it holds |
| Every setting with its default | [config-reference.md](config-reference.md) (generated: `cargo run -p config-reference`) |
| The agent contract | [agents.md](agents.md), [constraints.md](constraints.md), [skills/pcb-maker/SKILL.md](../skills/pcb-maker/SKILL.md) |
| Design notes | [router.md](router.md), [placer.md](placer.md), [global-route.md](global-route.md), [pin-swap.md](pin-swap.md), [profiling.md](profiling.md) |
| Results | [benchmarks.md](benchmarks.md); the tiers and the measurement discipline in [benchmarks/agent-tasks/README.md](../benchmarks/agent-tasks/README.md); the harvest in [benchmarks/github/README.md](../benchmarks/github/README.md) |
| Nightly logs | `build/nightly/<date>/` (`summary.md`, `gap.txt`, `diff-*.txt`, one directory and log per stage); the newest 14 nights keep their board directories |
| Tasks | `benchmarks/agent-tasks/tasks/v7/` (every tier); tier lists in `benchmarks/agent-tasks/{quick,short,schematic}.json` |
| Harvested boards | `benchmarks/real/external/github/` (not in git; `benchmarks/github/manifest.json` has origin and licence), their schematics in `benchmarks/real/external/github-sch/` |
| Release | the rolling `snapshot` pre-release on GitHub, remade by `.github/workflows/release.yml` on every push to main that passes `scripts/check.sh --fast`; `scripts/release.sh` does it by hand |
| Agent memory | `~/.claude/projects/-home-flo-programming-pcb-maker/memory/` |

## Gotchas

- The repository is rustfmt-clean and clippy-clean: format Rust changes
  before committing.
- A cargo test of the library crates leaves `target/release/pcb-maker`
  stale: build before copying a snapshot (`scripts/snapshot.sh` does).
- `pgrep -f` and `pkill -f` match their own shell: a waiter on `pgrep`
  never ends, `pkill` kills the session. Wait on a PID (`tail
  --pid=<pid> -f /dev/null`) or a done-file; kill by PID, children by
  `pgrep -P <parent>`.
- Never edit a bash script while an instance of it runs: bash reads the
  file as it executes, and the edit lands in the middle of the run
  (`nightly.sh` included). Copy it to a new name, or wait.
- No backticks in `git commit -m`.
- A layout's verdict has no reference by default: every KiCad finding
  counts except a footprint's own (`footprint_findings`) and the input's
  own edge clearances, mask bridges and courtyards of kept parts
  (`input_findings`). run.py's strict rule subtracts the designer's
  findings at the same place, so a verdict's exit 1 and a strict pass can
  both be right. `drc-kicad-board --input <project-dir>` judges as the
  layout did.
- The nightly rewrites parts of README.md, this file and
  docs/benchmarks.md and commits them on main
  (`scripts/nightly-report.py --docs`): README's "Boards harvested from
  GitHub" bullet from "pcb-maker places and routes" on and the table's
  rows that name a nightly; this file's title line, the `nightly-state`
  block and the paragraph that begins `**Layout sweep v` and says
  "written by the nightly". Edit around them.
- The environment index at the end of this file is generated
  (`cargo run -p config-reference`); `cargo test --test
  config_reference` fails while it is stale or a `PCB_*` name in the
  code lacks its line.
- skills/pcb-maker/SKILL.md is tested (tests/cli_exit_codes.rs): every
  `<verb>-kicad-<noun>` or `inspect-...` word in it must be an agent
  command named in docs/agents.md, and every agent command must be in
  it. The binary prints agents.md's section names in its next actions:
  do not rename them.
- One task list per comparison: every tier defaults to
  `benchmarks/agent-tasks/tasks/v7`. compare.py's first line says TASK
  LISTS DIFFER when two logs ran on different lists; rows on other
  inputs are marked INPUT.
- Benchmarks use a copied binary from `build/bin`. Above a load of about
  45 the wall guard cuts rows (CUT is no evidence). Pin CPU experiments
  with `taskset` on symmetric halves (logical CPUs n and n+16 share a
  core).
- The machine (Ryzen 9 5950X, 16 cores, 32 threads, 62 GB) is shared
  with Florian and the nightly (20 CPUs from 02:00 to about 07:00):
  leave him 4 cores and 4 GB. A run that grows past its reservation with
  no room left ends with exit 4 (not the OOM killer); run.py retries such
  a row once.
- KiCad 10: pinfunction is `NAME_NUMBER`; pad and text angles are
  absolute; copper items reference nets by name, and an unknown name
  becomes a new net unless a touching pad supplies one; DRC caps a
  violation type at 499 (199 for some); its DRC is not byte-deterministic
  (finding order, which pair names a missing connection).
- KiCad check cost: a DRC with the user's global library tables costs
  seconds and gigabytes per call on any board, so the checks run with a
  library-free configuration (`kicad_config.rs`;
  `PCB_MAKER_KICAD_USER_LIBS` turns that off).
- Freerouting's jar opens its GUI without `-de`/`-do` (`--help` too):
  always `-Djava.awt.headless=true -de ... -do ...`, or use
  `benchmarks/github/freerouting.py`.

## Switches

Environment variables the binary reads, one line each. Defaults are what
an agent gets; none is needed for normal use. The settings files' keys
are in [config-reference.md](config-reference.md).

<!-- The environment index is generated: cargo run -p config-reference. -->

User-facing (`pcb-maker --help` lists them):

- `RAYON_NUM_THREADS`: threads of the router's searches and of the placer's seed pool (default: every core); results do not depend on it.
- `PCB_WORK_RATE`: `<expansions/s>[,<placer units/s>]`: the work rates pinned instead of measured at the start (a verdict's `work_rate` and `placer_work_rate` repeat its run; `3.15e6,150e6` is the reference's, run.py's default); wins over the router settings `work_rate` and `placer_work_rate`. `place-kicad-board` pins the reference's unless this or `--measure-work-rate` says otherwise. `pcb-maker calibrate-work-rate [rounds]` prints the measurement alone.
- `PCB_ROUTER_SEEDS`: `N`: with the `auto` router config, N seeds routed side by side and the best kept (the router setting `seeds`).
- `PCB_SKIP_NATIVE`: with any router config (a file too, since 2026-10-21), no KiCad checks (`skip_native_verification`; machines without kicad-cli): the verdict is `router_complete_unverified` at best, exit 1.
- `PCB_KICAD_CLI`: the kicad-cli program to run (default `kicad-cli` on the PATH; on macOS `/Applications/KiCad/KiCad.app/Contents/MacOS/kicad-cli`). The agent commands ask its version before any work: missing, failing or older than KiCad 10 is an input error, exit 2 (a warning where the run needs no KiCad).
- `KICAD10_FOOTPRINT_DIR`: KiCad's own variable (`KICAD9_FOOTPRINT_DIR`, `KICAD8_FOOTPRINT_DIR` after it): import-kicad-netlist's folder of KiCad's footprint libraries, before the standard install folders (Linux, macOS, Windows).
- `PCB_ROUTER_NO_WALL_GUARD`: no wall-clock guard in the router, and the layout sets no wall-clock deadline: work alone ends runs (determinism checks, the walkthrough test).
- `PCB_PLACER_WORK_SECONDS`: place-kicad-board's placement work budget in seconds of work (default the layout's share: 40 % of the layout config's total_seconds, 600 by default).
- `PCB_KICAD_TIMEOUT`: seconds one kicad-cli report may take before it is killed (default 900).
- `PCB_MIN_CONNECTION_FLOOR`: `errors`: floor the router's track widths at the board's minimum connection width (and a custom `connection_width` rule's) only when KiCad rates connection_width an error; `off`: never (default since 2026-10-24: at error and warning severity).
- `PCB_KICAD_NO_NECK_REPAIR`: no connection-width neck repair after the final DRC.
- `PCB_MAKER_CACHE_DIR`: root of the DRC cache and of the library-free kicad-cli configuration (default `$XDG_CACHE_HOME/pcb-maker`, else `~/.cache/pcb-maker`).
- `PCB_MAKER_NO_CACHE`: anything but `0`: no DRC cache (both sides of a CPU pair).
- `PCB_MAKER_KICAD_USER_LIBS`: anything but `0`: kicad-cli's DRC and ERC run with the user's KiCad configuration (global libraries) instead of empty library tables.
- `PCB_MAKER_MEMORY_MB`: a resident memory limit in megabytes beside the router setting `memory_mb` and the limits the process reads itself (RLIMIT_AS, the cgroup's): the ladder runs as many routers side by side as fit the tightest.
- `PCB_MAKER_LEDGER`: the machine's memory ledger, where routes, layouts and KiCad's checks reserve their memory before they allocate and wait for room (default `$XDG_RUNTIME_DIR/pcb-maker-ledger.json`, else `/tmp/pcb-maker-ledger.json`; `off`: none; `pcb-maker ledger` lists it). Reservations do not change results.
- `PCB_MAKER_LEDGER_MARGIN_MB`: the least memory the ledger keeps free of reservations (default 6144): all reservations together take at most MemTotal less the memory outside the ledger (the median of the headroom samples beside it, else 25 % of MemTotal; since 2026-10-27), and never more than MemTotal less this margin.
- `PCB_MAKER_LEDGER_TOTAL_MB`: what the ledger's reservations may take together, in place of the capacity from the machine's memory (tests).
- `PCB_MAKER_LEDGER_INSIDE`: `<pid> <started>`: the reservation in the memory ledger this process runs inside (run.py sets it for the binaries of a board whose memory it holds; since 2026-10-27): the process reserves only what it needs beyond it.
- `PCB_MAKER_LEDGER_SHORT`: `1`: the command's requests of at most 2 GB of its own beside one KiCad check are short: they go once the capacity has their room and the memory available less the reservations' growth covers them, never behind the ledger's order or a drain; `0`: none are (default: every command that reserves but a route and a layout; docs/benchmarks.md, "Memory ledger").

Switches and experiment hooks (change the result):

- `PCB_ROUTER_CORRIDOR_ESCAPES`: overrides the router's `corridor_escapes` (`0` off, anything else on; on by default since 2026-11-05): a pad without a legal node of its own whose escape nodes lie in a pocket the lattice closes leaves by a corridor stub off the lattice, a path through the region its track keeps its clearances in to a legal node beyond the pocket (OpenFC's U10.5; docs/router.md, "Corridors of pocketed pads").
- `PCB_ROUTER_PIN_LINES`: overrides the router settings' `pin_lines` (`0` or `off`: the regular lattice; `1` or `exits`: lattice lines through the pads of pin rows the regular lattice misses; off by default): a part whose row exits share a phase gets the lattice at that phase over its rows and 1 mm around (a band, where the narrow pads in it vote for that phase), other exits a line added or moved onto them; the lattice's lines are then per-axis coordinate tables (docs/router.md, "Lattice lines at pin rows").
- `PCB_ROUTER_BLIND_VIAS`: overrides the router's `blind_vias` (`0` off, anything else on; off by default): where the board allows via spans short of the stack (`via_spans`, the designer's blind, buried and micro vias, a KiCad 6 or 7 project's switches), a via takes the shortest allowed span holding the two layers it joins, copper and a hole only there, written as KiCad's blind, buried or micro via; a BGA's dog-bone takes the shortest span to a routable layer (docs/router.md, "Blind, buried and micro vias").
- `PCB_ROUTER_TROUBLE_PRESENT`: the trouble-makers' repair: the price of sharing a node in the trouble-maker's search (default 20).
- `PCB_ROUTER_TROUBLE_SHARE`: the trouble-makers' repair: its work as a share of the attempt's negotiation (default 0.5, at least 30 s).
- `PCB_ROUTER_TROUBLE_ROUNDS`: the trouble-makers' repair: rounds over the open nets (default 3).
- `PCB_ROUTER_TROUBLE_VICTIMS`: the trouble-makers' repair: the most nets one exchange may displace (default 12).
- `PCB_ROUTER_TROUBLE_PER_OPEN`: the trouble-makers' repair: the nets an exchange may displace per open terminal of the trouble-maker; its limit is the larger of this times its open terminals and `PCB_ROUTER_TROUBLE_VICTIMS` (default 2; 0 the flat limit).
- `PCB_PLACER_PAD_EXITS`: overrides the placer's `pad_exits` (`1` on with the defaults where the config has none, `0` off; off by default): after each seed's placement the router's test of every pad's way out on the statics, the parts whose pads another part or the edge closes moved or turned, the pads left closed in board-placer.json's `pad_exits` and the verdict (docs/placer.md, "Pads without a way out").
- `PCB_ROUTER_DIFF_PAIRS`: turns differential pairs on (1) or off (0) where the settings name none (`diff_pairs`; unset, route mode turns them on where the project's custom rules ask for pairs, else off): route mode routes the pairs KiCad names (`_P`/`_N`, `+`/`-`, `P`/`N`) as coupled runs at the rules' width and gap, each link a net of the route negotiated with the others (docs/router.md, "Differential pairs").
- `PCB_ROUTER_DIFF_PAIRS_WORK`: differential pairs: the fallback router's work deadline in seconds, which routes what the coupled runs left open (default 0.05 of the ladder's budget, at least 1 s; for experiments).
- `PCB_ROUTER_DIFF_PAIRS_GIVE_UP`: differential pairs: `1` gives a link without a path, or whose conflicts stop falling, up in the negotiation: its pair's nets route uncoupled from then on, in the same attempt, and the hard repair tries it once more; where the settings name no `diff_pair_give_up` (default off: jetson-nano-baseboard's unconnected 23 and 44 down to 10 and 16, but ducktop2's left_io lost 3 of 6 coupled pairs on one seed; docs/router.md, "Giving up on a link").
- `PCB_ROUTER_GIVE_UP`: the give-up's rounds as `min,stall,pathless,max[,present]` (default `8,4,1,16,100`): rounds in conflict count once the price of sharing has reached `present`; no link goes before `min` such rounds, then once its conflicts have not fallen below their lowest for `stall` rounds, and after `max` rounds whatever; a link without any path goes after `pathless` rounds (for experiments; docs/router.md, "Giving up on a link").
- `PCB_ROUTER_DIFF_PAIRS_SKEW`: differential pairs: `0` turns off the meanders that match a coupled pair's two lengths to the rules' skew (else the pair's gap) or length (the skew is still measured and reported; docs/router.md, "Skew and length").
- `PCB_ROUTER_DIFF_PAIRS_RETURN_VIAS`: differential pairs: `0` turns off the return vias of the reference net (GND) beside a coupled run's vias, `1` turns them on (default on; the settings' `diff_pair_return_vias` wins; docs/router.md, "Return vias").
- `PCB_ROUTER_LENGTH_MATCHING`: turns the length and skew rules' meanders off (0) or on (1) where the settings name none (`length_matching`; unset, on): a net shorter than a custom rule's `length` minimum, or further than a `skew` maximum below the longest net of its rule, gets a meander on a straight track of its route, written as a KiCad tuning pattern (in route mode and on a layout's own routes), and the verdict's `lengths` reports each such net; a net over a `length` maximum is routed again the short way where the nets in its way can go round (off: none of it; docs/router.md, "Length and skew rules").
- `PCB_ROUTER_SEED`: `N`: perturbs the net order and seeds the history with noise, so that one board routes many ways.
- `PCB_NARROW_RESERVE`: `share`: the share of the ladder's first attempt's work kept back for the narrow-signal step (wins over the router setting `narrow_reserve_share`; default 0.75).
- `PCB_WALL_STRETCH_MAX`: `cap`: the most the wall-clock guards stretch for pinned rates faster than the machine measured (the pinned rate over the measured; default 4, 1: no stretch, the guards before 2026-10-21).
- `PCB_LADDER_LINES_PROBE`: `0`: route mode's ladder does not probe its leader's rung once more with lattice lines at the pin rows where the leader's probe leaves pads unfinished at rows the regular lattice misses; the trigger after the leader's run alone decides (as before 2026-11-08's "The rows decided at the probes"; for A/B on one binary).
- `PCB_LAYOUT_LINES_PROBE`: `0`: a layout decides no lattice lines at the pin rows itself (the layout before 2026-11-08's "A layout's rows decided by its probe's open pads"; for A/B on one binary); `race`: only at the kept placement's probe; `ladder`: only at the final ladder's probes; unset: both (the router setting `pin_lines` and `PCB_ROUTER_PIN_LINES` win over it).
- `PCB_PROBES_SIDE_BY_SIDE`: `N`: at most N probes of the probe ladder at once (1: one after the other).
- `PCB_PLACER_START`: `<board>`: a diagnostic start: the placer's stages from `PCB_PLACER_START_STAGE` on start from this board's poses, by reference (docs/placer.md, "Where the arrangement is lost").
- `PCB_PLACER_START_STAGE`: with `PCB_PLACER_START`: `global` (default: the global placement starts from the poses), `anneal` (no global placement), `legalize` (no global placement and no anneal: the legaliser seats the poses, the refinement follows).
- `PCB_PLACER_SKIP_ANNEAL`: a diagnostic: no anneal at any level; each level's legaliser seats the global placement's poses (or `PCB_PLACER_START`'s), the refinement follows (docs/placer.md, "Where the arrangement is lost").
- `PCB_PLACER_START_PARTS`: `<file>`: with `PCB_PLACER_START_STAGE=global`, a JSON list of `REF#n` (the n-th footprint of that reference): only these start from the board's poses, the others from the placer's own start.

Diagnostics (stderr unless said; the result does not change):

- `PCB_ROUTER_CORRIDOR_CENSUS`: set: the router prints one `corridor census:` line per escape pad and lattice on stderr (its escape nodes' pocket open or closed on the lattice, and whether a corridor stub leaves it); `experiments/corridors/census.py` reads them (docs/router.md, "Corridors of pocketed pads").
- `PCB_ROUTER_PIN_LINES_PROBE`: set: the router prints `pin lines:` lines on stderr for each lattice it builds and the finer pitches 0.075 and 0.05 mm: the lines the pin rows and pad gaps ask for, which the regular lattice misses, and what the bands and single lines would place for the exits (`2`: every missed line too) (docs/router.md, "Lattice lines at pin rows").
- `PCB_ROUTER_TROUBLE_WHY`: the trouble-makers' repair prints every exchange's outcome (kept, or why refused; a refusal names the nets left in conflict and what their conflicts name), and after the repair the crossing census: every open pad of a fine-pitch row, what closes its straight ways out, and whether a row neighbour's target crosses its own (docs/router.md, "The crossing census").
- `PCB_ROUTER_BGA_FANOUT_WHY`: the prepared fan-outs (`bga_fanout`) print each grid array's plan once a process: pitch, rows and columns, the rings that leave on the surface, the inner balls, the dog-bones planned by via size, the balls kept (a via in the pad) and those no gap fits.
- `PCB_ROUTER_NET_WORK`: `<file>`: the search expansions per net over the process (every router, every phase), written to the file as JSON at each router's finish; the last write holds the whole run (docs/router.md, "Prepared fan-outs for BGAs").
- `PCB_PAD_EXITS_WHY`: the pad-exits test prints, per closed pad, every rim node of its pocket and the item that closes it, and per open pad where its pocket opened.
- `PCB_PAD_EXITS_COUNTS`: pad-exits-kicad-board prints the probe's counts beside the CPU time of the lowering and the probe (for calibrating the test's work).
- `PCB_ROUTER_LENGTH_DEBUG`: the length repair: prints per net over its maximum what the priced search found, the victims and why the exchange was kept or undone; the length reroute: its router's log and per net the route's status.
- `PCB_ROUTER_DIFF_PAIRS_DEBUG`: differential pairs: prints each launch, each link's path, the copper refused with the reason, each stub straightened or not, and the fallback's work.
- `PCB_ROUTER_LINK_TRACE`: differential pairs: per negotiation iteration and after the hard repair, each link's state, conflicts and search work, and the links given up or standing again (docs/router.md, "Giving up on a link").
- `PCB_ROUTER_PAIRS_WHY`: differential pairs: per link that falls back for its stubs the geometry (`pairs why: stub ...`: the pad, the stub's end, the run's start and direction, what the pad's copper reaches: its end, other pads, the link's other end, or nothing), per finding that refuses a link's copper what of it is hit (the run, an end's jog, a via) and by what (`[partner]`: the pair's other net), per link whose stubs were routed after the attempt the outcome, per write a summary (docs/router.md, "Stubs and copper"), and per stub of an open net what keeps its pad from its stub's end (`pairs why: open ...`: the end crossed or straight, the hard repair's record of the net, the first of the other nets' routes, the link's route, the partner's routes and the plan's copper whose removal lets the pad reach the end; "A pair's stubs in the plan").
- `PCB_ROUTER_PAIRS_REACH`: differential pairs: per pairs' plan, per stub the router is to route, whether its pad reaches its stub's end on the planned board alone (no route yet), and where not what frees it: the link's lead, the partner's plan copper, the other links' plan copper, or the board's other pads (`pairs reach: ...`, with the pads and ends along and across the run and what lies near the straight stub; docs/router.md, "A pair's stubs in the plan"). Slow: a route per stub per plan.
- `PCB_ROUTER_DEBUG`: the router's verbose lines (failed searches, the resolution, what blocks a stub, lattice details).
- `PCB_ROUTER_DUMP`: `x0,y0,x1,y1`: prints that region of the lattice per layer (static blocks, own pads, occupancy, where vias may not go).
- `PCB_ROUTER_FINGERPRINT`: a hash of the routing state at each phase (determinism and byte-identity checks; compare as a multiset when attempts run side by side).
- `PCB_ROUTER_FINGERPRINT_ALL`: with `PCB_ROUTER_FINGERPRINT`, also hashes every occupancy map and pour mask into each fingerprint.
- `PCB_ROUTER_MEM_PROFILE`: the memory profile: the resident size's highs every 10 ms, the live heap, the phases and every live router's parts, the peak at the end (`mem-profile:` lines; docs/profiling.md, "Memory").
- `PCB_ROUTER_POUR_TRACE`: per pour analysis its counts, modelled work and wall time (calibrating the pour work constants).
- `PCB_ROUTER_SEVER_TRACE`: per refresh of what a cut severs of the pours (`pour_integrity`) its nodes, wall time and marked nodes per layer (calibrating `SEVER_NODE_NANOS`).
- `PCB_ROUTER_BUILD_TRACE`: per router build and per lowering their counts, modelled work and wall time (calibrating the build and lowering work constants).
- `PCB_ROUTER_POUR_DUMP`: `<dir>`: writes the pour maps at each finish.
- `PCB_ROUTER_SEARCH_RATE`: the search times itself for the `profile rate:` line (calibrating `REFERENCE_EXPANSIONS_PER_SECOND`).
- `PCB_ROUTER_TIMING`: where a reroute's time goes (layout moves).
- `PCB_ROUTER_SUB_BOARD_DEBUG`: each lowering prints the nets split by sub-board and those kept whole.
- `PCB_ROUTER_SPOKE_DEBUG`: the spoke model's decisions per pad.
- `PCB_ROUTER_DUMP_MANUAL_SPOKES`: dump-pour-map also adds the manual spokes and says how many.
- `PCB_PLACER_DEBUG`: the placer's progress lines.
- `PCB_GLOBAL_DEBUG`: the global route's negotiation, a line per iteration (nets to reroute, overflow, search work; docs/global-route.md).
- `PCB_PLACER_TRACE`: `<file>`: per seed (its number, whether it was kept) the weighted wirelength after each stage, the placer's objective at each stage split by term (at the placement's level and without halos; at the designer's poses too with `PCB_PLACER_DESIGNER`), and per part each stage's pose and how far each stage moved it.
- `PCB_PLACER_DESIGNER`: `<board>`: the designer's poses judged by the placer's model (their wirelength, what each illegal movable part meets), and with `PCB_PLACER_TRACE` in the trace beside ours.
- `PCB_PLACER_DESIGNER_LEVEL`: `full`: judge the designer's poses at the full spacing; `tight`, `touch`: with tight bodies, and courtyards touching (default: the smallest spacing, no halos).
- `PCB_PLACER_CROSSINGS_PROFILE`: with `PCB_PLACER_DEBUG`: per placer phase a `placer crossings` line, the wall time inside the ratsnest-crossings count against the work it booked and the rest of the phase's (the booking's calibration); `ops` times each of its operations too (docs/placer.md, "Work, budgets and determinism").
- `PCB_PLACER_CHECK_OVERLAP`: the annealer checks its incremental overlap against a full recount, and the placer its caches against the direct computation at every use (the anneal's wirelength terms, the escape term's prepared pads, the relation repair's sweep, the bucket grids' kept answers, the legality tests' parts apart by their boxes and their kept facts, the outlines by bands, the crossings of fresh trees beside the grid, the anneal's far pairs by their boxes, the kept lines' crossing counts and the new lines that are a kept one again; docs/profiling.md, "The second placer round", "The third placer round", "The fourth placer round"). The checks book no work, draw no random number and leave the caches as the unchecked path does: the placement is the one without them (docs/profiling.md, "How to profile").
- `PCB_ROUTER_CHECK_RIP_UP`: the race's ripped count checks the most conflicted net its heap finds against the scan of every net at every rip-up (docs/profiling.md, "The fourth placer round"; debug builds always do); the routes are the ones without it.
- `PCB_LADDER_STATS`: the probe ladder prints each probe's other statistics (the ladder's judge).
- `PCB_KICAD_NECKS_DEBUG`: the neck finder per net and layer.
- `PCB_LABELS_DEBUG`: copper texts the label mover could not place.
- `PCB_OUTLINE_DEBUG`: the outline's area, vertices and cutouts.
- `PCB_TOPO_DEBUG`: the topological engine's failures in short (overflowing edges by kind, failed and unrouted wires).
- `PCB_TOPO_SVG`: `<dir>`: the topological engine writes an SVG picture of every stage there.
- `PCB_TOPO_TAUT`: the topological engine reports taut wires that do not settle.
- `PCB_TOPO_TRACE`: `N`: the topological engine traces the radii of wire N.

Tests and research commands:

- `PCB_MAKER_KICAD_VERIFICATION_CACHE`: `<dir>`: verify-kicad-rung caches its KiCad checks there.
- `PCB_MAKER_KICAD_VERIFICATION_CACHE_EVENTS`: `<file>`: verify-kicad-rung's cache appends its hits and misses there (tests/native_verification.rs).
- `PCB_MAKER_TEST_PANIC`: panic once a run has started (tests/cli_exit_codes.rs).
- `PCB_MAKER_TEST_PANIC_ATTEMPT`: `<kind>[:<n>|:all]`: the n-th (default the first) attempt of the router's ladder of a kind (probe, continuation, rung, seeds, step), or every one, panics; the ladder records it in `attempts_failed` and goes on (docs/router.md, "The ladder in code, and its faults"; tests/cli_exit_codes.rs).
- `PCB_MAKER_TEST_DROP_DRC`: the route's drc.json removed before the verdict reads it (tests/cli_exit_codes.rs).

<!-- End of the generated environment index. -->
