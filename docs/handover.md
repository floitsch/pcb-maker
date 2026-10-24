# Hand-over (state as of 2026-10-23, main 38edddd, snapshot x1725; measured by layout sweep v20; x1753 and x1754 since)

What the tool does today, which benchmarks count, what is known to be
wrong, where everything is and how to work on it. What each round of work
measured and changed is in [changelog.md](changelog.md), newest first
(the dated "Committed on ...", triage, sweep and measurement sections that
used to be here; code comments and docs that cite `docs/handover.md,
"Committed on ..."` mean those sections). Longer-lived ideas are in
[todo.md](todo.md); the commits themselves in git.

## The flow in one paragraph

`layout-kicad-board` reads the board in a canonical order (footprints by
reference, their pads and graphics sorted, so two saves of one design lay
out alike), places the free footprints (ePlace-style global placement from
a canonical orientation, annealing, legalisation and refinement, with
the escape demand of each part's sides in the density, the rotations,
the legaliser and the refinement since 2026-10-24; up to 16
placer seeds run in a pool of the search threads and are judged in seed
order; the race ranks them by RUDY and probes the best three with a 75 s
route), routes the board with the lattice router (PathFinder negotiation;
pours as fill the pads join through thermal spokes modelled on KiCad 10's
filler and DRC, stranded pads routed as tracks, islands stitched, manual
spokes where the model finds a pad short), tries footprint moves, resumes
the first route briefly, polishes (clean-up and via reduction) without
renegotiating, and - when connections stay open - runs the attempt ladder
(probe every pour mode, continue the best; then the regular rungs, finer
pitches and seeds; with work kept in reserve, a last attempt with signals
at the project's narrowest predefined width). A four-layer board with
pours that is far from complete after the first route takes that narrow
attempt directly, then, with 240 s of work left, the same attempt in the
other pour mode. KiCad's connection-width
findings are repaired afterwards with KiCad's own fill. Every budget is in
work (search expansions and placement units at rates measured at the
start, the reference's 3.15e6 and 150e6 a second scaled to the machine;
pour analyses and KiCad checks by a model), so a layout is deterministic
for its input, binary and rates (`PCB_WORK_RATE` pins them; the tiers pin
the reference's), whatever the thread count, unless the wall-clock guard
(`wall_guard`, 2 times the budget) cuts it (`cut` in the verdict). KiCad's ERC/DRC/connectivity
decide pass or fail (exit codes 0-4 and `verdict.json`).
`route-kicad-board` is the same without placement; `import-kicad-netlist`
starts an agent's board from a schematic (`--footprints-from` a designer's
board; starter constraints for key matrices). See
[router.md](router.md), [placer.md](placer.md),
[constraints.md](constraints.md), [agents.md](agents.md).

## Benchmarks that count

Only the current references; older sweeps and tiers are in
[changelog.md](changelog.md) and [benchmarks.md](benchmarks.md).

The newest nightly measurement of main (scripts/nightly.sh; the table of
the last 14 nights is in [benchmarks.md](benchmarks.md#nightly)):

<!-- nightly-state:begin -->
**Nightly**: none yet (`bash scripts/nightly.sh --install` installs the timer; `scripts/nightly-report.py` writes this line).
<!-- nightly-state:end -->

**Layout sweep v16** (x862, tasks v6, 109 boards harvested from GitHub,
each reduced to stacked parts plus a `constraints.json` derived from the
designer's board, 1800 s each, five jobs beside other agents):
**44 of 109 pass** (KiCad: 0 unconnected, no copper error beyond the
designer's, constraints met); 3527 unconnected in all, 82 new DRC errors,
17 rows cut by the wall clock. `build/github-layout-v16.log`; table in
[benchmarks.md](benchmarks.md#layout-sweep-v16-2026-10-15), triage in
[changelog.md](changelog.md#triage-of-v16-2026-10-15-x931--x932).

**Layout sweep v17** (x1170, tasks v7, 8 jobs x 2 threads): **48 of 109
pass**, 2708 unconnected, 38 new DRC errors, 2 rows cut.
`build/github-layout-v17.log`; table in
[benchmarks.md](benchmarks.md#layout-sweep-v17-2026-10-16).

**Layout sweep v18** (x1315 = main 35456a1, tasks v7, run in three parts
spliced on one binary): **42 of 109 pass the strict rule** (46 the
per-type rule), 2712 unconnected, 2 rows cut (katia, Batch_1, cut at 2
jobs too); medians against the designer's board: vias x0.71, length
x1.03, open 0.009 of the connections. The designer tier on the same
binary (`build/designer-x1315b.log`): 50 strict passes; on the 97 target
boards with both results, placement 71 % and routing 29 % of the open
connections (`compare.py --gap`). `build/github-layout-v18.log`; table,
gap split and the boards in
[benchmarks.md](benchmarks.md#layout-sweep-v18-2026-10-21).

**Layout sweep v19** (x1539 = main af9d32a, tasks v7, 10 jobs x 2
threads, one part): **49 of 109 pass the strict rule** (52 the per-type
rule; v18 re-judged under rule 3 stays at 42 / 46), 1827 unconnected, no
row cut or killed, ten without a result (nine refusals, eight of them
panels, and the 0xCB pcb-panel's DRC timeout); medians against the
designer's board: vias x0.74, length x1.08, open 0.000 of the connections
(mean 0.090); no via in a small SMD pad on any board. The matched designer
tier on the same binary (`build/designer-x1539-matched.log`, budget scale
0.695): 54 strict passes; on the 97 target boards with both results,
placement 59 % and routing 41 % of the open connections.
`build/github-layout-v19.log`; table, gap split and the boards in
[benchmarks.md](benchmarks.md#layout-sweep-v19-2026-10-22).

**Layout sweep v20** (x1725 = main 38edddd, tasks v7, 10 jobs x 2
threads, one part): **53 of 109 pass the strict rule** (rule 4; 53 the
per-type rule; v19 re-judged under rule 4: 52 / 52), 1848 unconnected
(1568 on the 99 boards with a result in v19 too, v19 1827), no row cut
or killed, seven without a result (six panels refused, the 0xCB
pcb-panel's DRC timeout); medians against the designer's board: vias
x0.76, length x1.05, open 0.000 (mean 0.091), bends per connection x1.11,
stair-steps +60.5, pin-row crossings +1; no via in a small SMD pad. 77
tasks drop labels at run time since 14831a6, so their placements differ
from v19's for that reason too. The matched designer tier on the same
binary (budget scale 0.721, v20's median final-ladder share) is partial
(the machine stayed full): over the 25 failing target boards with both
results placement 43 % and routing 57 % (69 / 31 without Batch_1).
x1753 (the narrow step's bound) and x1754 (escape demand on) post-date
it. `build/github-layout-v20.log`, `build/designer-x1725-matched.log`;
table, gap split and the boards in
[benchmarks.md](benchmarks.md#layout-sweep-v20-2026-10-23).

**Quick tier** (30 boards, full budget, the merge gate):
`build/quick30-x1152.log` (x1152 = main 015676c, tasks v6): **24 of 30
pass**, unconnected 29, new errors 4, vias 2693, length 99810 mm, work
9603 s, cut 0, first-route open terminals 61. Failing: WLED_Matrix (11,
copper_edge_clearance 2), MIDAS-MK1.1 (10), reCamera (5, hole_clearance
1), USB_Keypad (2), OpenFC (1), eternal-keypad (shorting_items 1). It
predates order independence (x1161), which changed every placement: the
next quick tier is a new base, not a regression test against this one.

**Short tier** (12 flip-prone boards at a third of the budget, for
iterating): `build/short-x1170.log` (x1170 = main 54eff39; running at the
time of writing). The last complete reference, `build/short-x1083.log`
(the KiCad check work on 28b9bf8), equals `build/short-x1161.log` but for
the charged check work: **7 of 12 pass**, unconnected 19, new errors 2,
vias 1134, length 21228 mm, work 2501 s, cut 0. x1170 should give those
rows (54eff39 is a rustfmt pass).

**Schematic tier** (12 boards from the designer's schematic, a third of
the budget): `build/sch-x1161.log`: **10 of 12 pass**, unconnected 4, new
errors 0, vias 3184, parity issues 40. Failing: slab-pcb-left (74/76,
KiCad 2; passes at the full budget) and USB_Keypad (37/39, KiCad 2; the
layout tier's own loss).

**Freerouting against our cold route** (x738 round, 2026-10-15; the
designer's placement with all copper removed, 1500 s each, both judged by
KiCad): Freerouting finishes 67, times out on 36, fails on 6; where both
finish we leave fewer connections open on 47 boards, Freerouting on 6
(184 against 873 unconnected, 51 clean boards against 8), at 0.65 times
its CPU; over all 109, 62 of our cold routes are clean. Our cold route
since gained SNSP-CPU-01 (178 to 5) and MegaDrive (137 to 70). Logs
`build/github-freerouting-x738*.log`; table in
[benchmarks.md](benchmarks.md#freerouting-against-our-cold-route-x738-2026-10-15).

**PCBench / PCBWorld D3** (2026-09-29): 110 of the 114 D3 agent tasks
laid out clean ([the tasks README](../benchmarks/agent-tasks/README.md#2026-09-29-the-114-d3-test-boards)).

How to run them (side by side since 2026-10-22: run.py admits each board
against the machine's memory ledger, which every binary since x1590
reserves in too, so tiers of several agents run at once without the old
lock; `--lock` takes build/quick-tier.lock for one tier at a time; load
under 40; the tiers default to 2 search threads a board and
physical cores / 2 boards, 8 x 2 on the 5950X; quick and short tiers on
tasks v6, the schematic tier and v17 on tasks v7):

```sh
B=build/bin/pcb-maker-x1180
benchmarks/agent-tasks/run.py build/short-x1180 --tier short --binary $B > build/short-x1180.log
benchmarks/agent-tasks/run.py build/quick30-x1180 --tier quick --binary $B > build/quick30-x1180.log
benchmarks/agent-tasks/run.py build/sch-x1180 --tier schematic --binary $B > build/sch-x1180.log
benchmarks/agent-tasks/compare.py build/short-x1170.log build/short-x1180.log
```

The full sweep: `benchmarks/agent-tasks/run.py build/github-layout-v17
--tasks benchmarks/agent-tasks/tasks/v7/tasks.json --binary $B --jobs 8 --threads 2`
admitted by the same ledger. Router questions go to route mode on fixed
placements (`--route-from <earlier output dir>`), CPU claims to pinned
simultaneous pairs; the discipline is in the tasks README, "How to
measure".

## Not targets

- `thpoll83__PolyKybd__poly_kb_molecule_*` (5 tasks): tile panels with
  duplicated references (SW1 sixteen times, 576 footprints on 29 nets); the
  designer's own board has 266 unconnected. Kept in the runner, ignored.
- The OpenESC 4in1 panel, OpenRX panel, krishveercard, SUMEC (48 pads no
  track of their class can enter): the board's rules, not the flow.
- `bitshiftcrazy__d20_pcb`: 39 parts, a back-side keepout that allows
  footprints yet KiCad flags 27 of our pads inside it; 6 of 14 nets have no
  path. Not understood.
- SmartSpin2k (a 585-connection panel whose first route alone needs about
  1000 s) and panels or boards without a schematic of their own (in the
  schematic tier).

## Open problems, ranked

Numbers are kept from earlier hand-overs (other docs cite "open problem
8"); done items are gone, their sections are in the changelog.

1. **Fill fidelity, what is left.** The starved-thermal census (44 boards,
   `experiments/pour-fidelity/census.py`) stands at 162 predicted, 7
   missed, 26 false alarms, 2 not modelled (x1054). Left: spoke landing and
   piece fidelity against KiCad's kept outlines (our map is in far more
   pieces: lattice sampling of necks), island-rule and track-rule edge
   cases, custom spoke templates, hatched fill over thermal pads without
   relief, per-zone fill settings and islands a zone seam alone makes,
   fine-pitch clearance within a few microns. KiCad's teardrops and its
   hatched iterative refill are not modelled; connection-width necks are
   repaired after the fact with KiCad's fill (gaps are not).
2. **One-spoke pads on fine-pitch rows** (k30's U10 pad 8, 1.77 x 0.50 mm:
   the two corridors along the row are closed by the neighbours, and the
   neighbours' exit tracks run through the two left). A hard block failed;
   the repair (`free_thermal_spokes`, budgeted, only when polishing) moves
   2 of 10 nets. Probably needs the exit direction of the neighbouring
   pads decided before negotiation (a fan-out step), not a cost.
3. **The ladder's judge.** On 21 boards with every rung continued to its
   final (2026-10-13), today's judge (unfinished nets after the probe)
   picks the best rung 16 times; no other count does better, and a class
   of boards (Hub, DSP-ADAU1452, koeg-board, zpn_devboard) probes tracks
   far ahead while a fresh connect attempt ends far better: the 75 s probe
   is too short there. If anything, a 30 s continuation per probed rung is
   the judge to try. The ladder stops at the first complete rung whatever
   its starved thermals (Castor: 20 starved kept against tracks' 12); rank
   by (open, starved) before vias, and let a complete-but-starved rung be
   rivalled. Details in [congestion-model.md](congestion-model.md) and the
   changelog's 2026-10-13 sections.
4. **Crowded placement** (OpenESC, SNSP, video, hackrf, BAGEL, 4in1-mini).
   The designer's placement routes 4in1-mini to 5 open where ours ends at
   48-56, and more seeds do not close it (placement seeds after honest
   work). Built and off: `routing_demand`, `routing_room`,
   `shared_halos`, `anneal_far_side_gaps`; built, off and removed on
   2026-10-16: `replicate_channels`, `supply_via_room`, `bus_rows`. The legaliser is not the lever (4 %); removing
   halos loses routing. Next: routing room from congestion rather than pin
   count, bus-aligned rows for video, seeded orientation starts in the
   race, and seed-repeated pairs as the judge: the race's probes pick by
   margins smaller than their own noise (hackrf, 4in1-mini).
5. **Starved thermals as the only failure** on complete boards (rosco,
   Castor, mackerel-30, k30, mackerel-08, Hub, PixelWave, BAGEL; counts
   fell to 0-5 since the spoke model). Since 2026-10-21
   `solid_starved_thermals` is on by default (a starved pad, or one whose
   spokes reach only an island, gets a solid zone connection, kept when
   KiCad finds the board better): mackerel-30 and Castor pass; k30-SBC's
   solid pads cost an unconnected neighbour and are undone (docs/changelog.md,
   "net ties, same-number pads, starved thermals"). Left: pads where solid
   does not help.
6. **Our open count against KiCad's.** The router counts connections per
   net, KiCad per missing pair; and the pour model's optimism adds. The
   pass rule trusts KiCad; the router's own count is for steering only.
7. d20's keepout pads (jiran-ble-lite's GND, 20 of 40 pads unreached: a
   split keyboard's two halves joined by a frame no track crosses; since
   2026-10-21 the other half's pads are joined among themselves, KiCad 20
   to 1 unconnected, the designer's 1); the 0xCB pcb-panel's zone refill, which kicad-cli does not
   finish in 900 s (`PCB_KICAD_TIMEOUT`).
8. **The agent flow's gaps** (the 2026-10-15 walkthrough; done: the fast
   no, room as numbers, what is kept, starter constraints for key
   matrices, outline polygons, the schematic tier). Left:
   - item 1: a crowded side (90-99 %) that no exact test calls hopeless
     still runs the placer's levels up to the budget;
   - item 4: starters for LED bars and other arrays, thumb cluster angles,
     column stagger, the controller's and USB connector's edge, keys wired
     to their own pins (no matrix);
   - item 5: tabs or mouse bites between outline pieces, an automatic size
     for a polygon, `room` suggesting a scale for a polygon;
   - item 6, *a router starter config*: a board from a netlist has the
     project's net classes or the default 0.2 mm for every net. The import
     could write `router.json` beside `layout.json` with a power class for
     rails (by name and by `power_in` pin type) and, on two layers,
     `add_pours` GND on B.Cu, listed in the report for the agent to keep or
     drop. Small to medium: a day with the from-schematic benchmark.
9. **The schematic tier's friction** (done: footprints from the board,
   library drift, board-only parts, orientation and order independence).
   Left:
   - item 3: libraries outside the project or named by absolute paths (22
     boards): the import says "no library X". Its `missing` line should
     name candidates: a folder `<library>.pretty`, or one holding the
     footprint's file, under the project's ancestors up to the repository
     root, and the longest existing suffix of an absolute table path; not
     used silently. Small.
   - item 5: a legacy `.mod` library (hackrf's 395 footprints) is unread;
     convert a `Legacy` table entry with `kicad-cli fp upgrade` into a
     temporary `.pretty`. Small.
   - item 6: a board KiCad cannot load says nothing (kicad-cli exit 3 with
     Debug lines only): say "KiCad could not load the board" and name the
     first item out of KiCad 10's order or form. Small.
   - item 9: the tier's two failures are the layout tier's (slab-pcb-left
     passes at the full budget; USB_Keypad's decoupling capacitor at -1.9,
     12.7 leaves its GND pad off the hatched pour).
10. **Wall-clock slack under load.** Since the 3.15e6 calibration a work
    second is a wall second on an idle core, so the guard's 10 % has little
    room: a sweep at five jobs of four threads beside other agents (v16:
    load 16-35) would cut rows the old constant's slack hid. Keep tiers at
    the default parallelism and under the lock. Open: a harness timeout and
    wall guard that follow the measured load. KiCad checks are charged by a
    model (3.6 s plus 1.7 s per thousand hatch holes) that does not see
    complex outlines (ULK, koeg, ghoul: 18-33 s a check); the first route,
    moves and polish do not reserve the kept board's check.
11. **Smaller threads left by recent rounds** (each in its changelog
    section): the search proper is 70-80 % of route mode, and the
    negotiation's stamp counts (a ripped branch's stamps out, the new
    route's in, two thirds changing each call) are the next speed target;
    the cold route's walled-in pads could be joined among themselves; the
    narrow step stalls at the price cap on DSP-ADAU1452 (try the other pour
    mode or the runner-up attempt); OpenESC's 700-1000 connection-width
    warnings from our own 0.09 mm tracks; the neck repair's gaps and its
    Keypad unconnected item; a pad's own mask expansion in the placer's
    opening rule; the molecule panels' back connectors need the designer's
    0.03 mm gaps; the board's top-level graphics, zones and tracks still
    follow the file's order; the import's netlist export, preview SVG and
    fab exports still run kicad-cli on the user's library configuration.

## Where things are

| What | Where |
| --- | --- |
| Current state, open problems | this file |
| What each round measured and changed | [changelog.md](changelog.md), newest first |
| Design notes | [router.md](router.md), [placer.md](placer.md), [constraints.md](constraints.md), [agents.md](agents.md), [congestion-model.md](congestion-model.md), [coupling.md](coupling.md), [global-route.md](global-route.md) |
| Results tables | [benchmarks.md](benchmarks.md), [benchmarks/agent-tasks/README.md](../benchmarks/agent-tasks/README.md) (tiers), [benchmarks/github/README.md](../benchmarks/github/README.md) (harvest, Freerouting) |
| Sweep logs | `build/github-layout-v16.log` and `build/github-layout-v16/` (v17: `build/github-layout-v17*` once it lands), `build/github-freerouting-x738*.log` |
| Tier references | `build/quick30-x1152.log`, `build/short-x1170.log` (complete: `build/short-x1083.log`), `build/sch-x1161.log` |
| Snapshot binaries | `build/bin/pcb-maker-xNNN` (x1170 = main 54eff39); `scripts/snapshot.sh <n>` builds and copies one, never over an existing number |
| Release | the rolling `snapshot` pre-release on GitHub, remade by `.github/workflows/release.yml` on every push to `main` that passes `scripts/check.sh --fast` (linux-x86_64, macos-aarch64 when its build succeeds, `VERSION`, `SHA256SUMS`); `scripts/release.sh` does it by hand (`--dry-run` packages into `dist/` only); `pcb-maker --version` and the verdict's `environment.pcb_maker_version` name the commit and build date |
| Tasks | committed in `benchmarks/agent-tasks/tasks/v7/` (every tier since 2026-10-21) and `tasks/v6/` (the quick and short tiers before; v7 is v6 plus uuids on six panels, none in those tiers), copies of `build/github-tasks-v6/` and `-v7/`, from `benchmarks/agent-tasks/from_pcbench.py --corpus benchmarks/github/boards.json`; tier lists in `benchmarks/agent-tasks/{quick,short,schematic}.json` |
| Harvested boards | `benchmarks/real/external/github/` (not in git; `benchmarks/github/manifest.json` has origin and licence); their schematics in `benchmarks/real/external/github-sch/` (`benchmarks/github/schematics.json`) |
| Code | `crates/pcb-router`, `pcb-placer`, `pcb-kicad`, `pcb-congestion`, `pcb-global` (the coarse global route), `pcb-topo`; the command line in `src/` (agent commands in `agent_cli.rs`, the rest in `main.rs`); `pcb-kicad/src/lib.rs` lists its modules (the file layer `kicad_sexpr`, `kicad_nets`, `copper_emit`; `geometry`, `copper_geometry`; `board_placer/`, `board_router/`, `board_layout`; `verification`, `native_report`, `drc_cache`, `verdict`; [README](../README.md#repository)) |
| The removed research stack | git history up to be4b012: the crates layout-trace-routing, pcb-routing, pcb-coordinator, pcb-placement, pcb-validate, pcb-viewer, pcb-benchmark and pcb-pipeline, 54 research commands (check, validate, place, route-grid, the semantic ladder, the route candidates, benchmark-route-*, ...), their tests and fixtures; list in [changelog.md](changelog.md#committed-on-2026-10-20-quality-research-stack-removed); pcb-kicad's research modules (the semantic ladder, the sequential and adaptive routers, the grid router and its inspections, via actions, route relaxation), the crates pcb-engine, pcb-grid-router, layout-trace-model and pcb-core and nine more commands in git history up to b306347 ([changelog.md](changelog.md#committed-on-2026-10-21-quality-pcb-kicad-research-removed-librs-split)) |
| Worktrees | one per package of work, `../pcb-maker-<name>`, made by `scripts/worktree.sh` |
| Agent memory | `~/.claude/projects/-home-flo-programming-pcb-maker/memory/` |
| Untracked in the main checkout | `pcb_pr_agent_brief.md`, `target-bisect/` (not from these sessions), `benchmarks/github/boards-guided.json`, `experiments/congestion/{audit_labels,reconstruct_finals,test_audit_labels}.py`: Florian decides |

## Working on it

**A worktree per package.** `bash scripts/worktree.sh <name> <branch>`
creates `../pcb-maker-<name>` on a new branch from main and links the main
checkout's `benchmarks/real/external`, `build` and `artifacts` into it, so
run.py, the tiers and `build/bin` work there as in the main checkout. It
prints the snapshot rule: snapshots are `build/bin/pcb-maker-x<n>`, never
reused; list `build/bin`, take a free range of ten above the highest in
use and name it in the hand-off. Commit on the branch; main is merged by
the coordinator.

**Before a commit:** `bash scripts/check.sh` runs the test set and builds
the release binary last (a cargo test of the library crates alone leaves
`target/release/pcb-maker` stale): the tests of every crate of the
workspace and the root binary's unit tests, the CLI tests, the fixture
tests, the Python tests, workspace clippy (`--all-targets`, warnings as
errors) and the slow tests that depend on the load (the agent
walkthrough, the placer's placement.rs). `bash scripts/check.sh --fast`
is the gate CI runs on every push: the same without the walkthrough and
placement.rs, about 30 s of tests once built. `--full` (the default)
runs on CI every night with KiCad 10 installed. Without kicad-cli the
KiCad tests skip and the script says so first; with it, run the script
under the machine's KiCad lock. Measured on 2026-10-16 in a fresh
worktree at a load of 20-27 on the 5950X: about five minutes in all,
half of it compiling.

| Step | Time |
| --- | --- |
| `cargo test --release -p pcb-router -p pcb-kicad -p pcb-placer` | 165 s with the first build; the tests themselves about 15 s (455 tests) |
| `cargo test --release -p pcb-maker --test cli_exit_codes --test agent_walkthrough --test native_verification` (needs kicad-cli) | 125 s: a minute building the root package, 64 s for agent_walkthrough's three layouts, a second for the others |
| `python3 -m pytest -q benchmarks/agent-tasks` | 1 s |
| `cargo build --release` | seconds after the tests, which built the binary already |

**The fixture boards** (`tests/fixtures`; review 2026-10-21's proposed
fixture tests). F1 is a 30 x 25 mm two-layer board of 13 parts in KiCad
10's format, written by hand to hold what the tiers found the router and
the lowering getting wrong: a QFN with an exposed pad and thermal vias,
a SOT-223, an LED with 0.7 mm pads, a GND test pad alone on F.Cu, a net
tie, a stacked front and back pad of one number, a jumper group, a pad
with zone_connect none, an arc track the route keeps, a Margin
rectangle, an unplated slot, a plated hole listing only `*.Mask`, a text
box and a dimension on copper, a padstack, a net named "14", GND and +5V
zones of equal priority, a hatched zone, net classes (one by a regular
expression) and a JLCPCB-style .kicad_dru with a pair rule. F2
(`tests/fixtures/f2/layout.json`) lays F1 out with a free bottom edge.
`tests/fixtures.rs` routes and lays them out through the binary (no via
in an SMD pad and no hop within hole_to_hole, the rules end to end,
golden fingerprints at 1 and 4 threads, KiCad's connectivity and pours
on the routed board, the free dimension's clean path with a fake
kicad-cli, a plane pour and a keepout over a fixed part);
crates/pcb-kicad/src/tests/fixtures.rs checks the static maps' cache
key, the lowering against a golden table and the order test without its
race. Every route pins `PCB_WORK_RATE`, the memory and the wall guard.
The golden files are `tests/fixtures/golden`; a behaviour change shows
as a golden diff, and `PCB_UPDATE_GOLDEN=1 cargo test --release -p
pcb-maker --test fixtures` (and `-p pcb-kicad fixture_tests`) rewrites
them, to commit with the change. The KiCad goldens (`f1-drc.txt`,
`f1-routed-connectivity.txt`) need kicad-cli 10.0.7 to rewrite.

Then `bash scripts/snapshot.sh <n>` for a binary to benchmark, the short
tier for iterating, the quick tier once as the merge gate.

**What a package measures, and what the nightly does** (since
2026-10-23: validating every package with every tier slowed the project).
A router package runs the fingerprints (Castor, k30-SBC, jetson, Sisu in
route mode from v16, 1 x 4, budget scale 0.25, `PCB_ROUTER_FINGERPRINT=1
PCB_ROUTER_NO_WALL_GUARD=1`) and the route-mode expansion tier (8 x 1),
each against main's; a placer package adds the short tier (3 x 2). No
package runs a full sweep or the designer tier: `scripts/nightly.sh`
runs them on main every night at 02:00 (a systemd user timer), after the
fingerprints, the expansion tier and the short tier of main's head,
through the memory ledger beside the day's work, and writes
`build/nightly/<date>/summary.md` (pass and strict counts, the medians
against the designer, the gap split, the cut and killed rows, the diff
against the previous night). A package's claim about the sweep is
checked by the first nightly after its merge; the line under "Benchmarks
that count" and benchmarks.md's "Nightly" table say what main does now
(benchmarks/agent-tasks/README.md, "Nightly").
`python3 scripts/check_links.py` checks the Markdown links (`--into
handover.md changelog.md` for the links into these two files).

## Switches

Environment variables the binary reads, one line each. Defaults are what
an agent gets; none is needed for normal use. The index is generated from
the code (tools/config-reference: every `PCB_*` name in the crates'
sources needs its line there, and `cargo test --test config_reference`
fails until the index is regenerated); the config keys are in
[config-reference.md](config-reference.md).

<!-- The environment index is generated: cargo run -p config-reference. -->

User-facing (`pcb-maker --help` lists them):

- `RAYON_NUM_THREADS`: threads of the router's searches and of the placer's seed pool (default: every core); results do not depend on it.
- `PCB_WORK_RATE`: `<expansions/s>[,<placer units/s>]`: the work rates pinned instead of measured at the start (a verdict's `work_rate` and `placer_work_rate` repeat its run; `3.15e6,150e6` is the reference's, run.py's default); wins over the router settings `work_rate` and `placer_work_rate`. `pcb-maker calibrate-work-rate [rounds]` prints the measurement alone.
- `PCB_ROUTER_SEEDS`: `N`: with the `auto` router config, N seeds routed side by side and the best kept (the router setting `seeds`).
- `PCB_SKIP_NATIVE`: with any router config (a file too, since 2026-10-21), no KiCad checks (`skip_native_verification`; machines without kicad-cli): the verdict is `router_complete_unverified` at best, exit 1.
- `PCB_KICAD_CLI`: the kicad-cli program to run (default `kicad-cli` on the PATH; on macOS `/Applications/KiCad/KiCad.app/Contents/MacOS/kicad-cli`). The agent commands ask its version before any work: missing, failing or older than KiCad 10 is an input error, exit 2 (a warning where the run needs no KiCad).
- `KICAD10_FOOTPRINT_DIR`: KiCad's own variable (`KICAD9_FOOTPRINT_DIR`, `KICAD8_FOOTPRINT_DIR` after it): import-kicad-netlist's folder of KiCad's footprint libraries, before the standard install folders (Linux, macOS, Windows).
- `PCB_ROUTER_NO_WALL_GUARD`: no wall-clock guard in the router, and the layout sets no wall-clock deadline: work alone ends runs (determinism checks, the walkthrough test).
- `PCB_PLACER_WORK_SECONDS`: place-kicad-board's placement work budget in seconds of work (default 600, the layout's share).
- `PCB_KICAD_TIMEOUT`: seconds one kicad-cli report may take before it is killed (default 900).
- `PCB_MIN_CONNECTION_FLOOR`: `warning`: floor the router's track widths at the board's minimum connection width also when KiCad rates connection_width a warning; `off`: never (default: only at error severity).
- `PCB_KICAD_NO_NECK_REPAIR`: no connection-width neck repair after the final DRC.
- `PCB_ROUTER_ISLAND_STUBS`: turns the router's island stubs on (`island_stubs`, off since 2026-10-21): a pad's straight track into a fill island its spokes reach, which only silences KiCad's isolated-island finding.
- `PCB_MAKER_CACHE_DIR`: root of the DRC cache and of the library-free kicad-cli configuration (default `$XDG_CACHE_HOME/pcb-maker`, else `~/.cache/pcb-maker`).
- `PCB_MAKER_NO_CACHE`: anything but `0`: no DRC cache (both sides of a CPU pair).
- `PCB_MAKER_KICAD_USER_LIBS`: anything but `0`: kicad-cli's DRC and ERC run with the user's KiCad configuration (global libraries) instead of empty library tables.
- `PCB_MAKER_MEMORY_MB`: a resident memory limit in megabytes beside the router setting `memory_mb` and the limits the process reads itself (RLIMIT_AS, the cgroup's): the ladder runs as many routers side by side as fit the tightest.
- `PCB_MAKER_LEDGER`: the machine's memory ledger, where routes, layouts and KiCad's checks reserve their memory before they allocate and wait for room (default `$XDG_RUNTIME_DIR/pcb-maker-ledger.json`, else `/tmp/pcb-maker-ledger.json`; `off`: none; `pcb-maker ledger` lists it). Reservations do not change results.
- `PCB_MAKER_LEDGER_MARGIN_MB`: the memory the ledger keeps free of reservations (default 6144): all reservations together take at most MemTotal less it.
- `PCB_MAKER_LEDGER_TOTAL_MB`: what the ledger's reservations may take together, in place of MemTotal less the margin (tests).

Switches and experiment hooks (change the result):

- `PCB_ROUTER_PIN_ROW_COST`: the router's `pin_row_cost` in millimetres where the settings name none (`pin_row_cost_mm`): the cost of another net's step between two adjacent SMD pads of a part's pin row (docs/router.md, "Pin-row cost"; for measuring other values).
- `PCB_ROUTER_NO_STRAIGHTEN`: turns the router's straightening off (`straighten`, on since 2026-10-23): after the via reduction and the thermal spoke repairs, stair-steps and jogs taken out of the signal nets' paths where the clearance allows (docs/router.md, "Straightening"; for attributing rows).
- `PCB_ROUTER_TROUBLE`: overrides the router's `trouble_repair` (`0` off, anything else on): the trouble-makers' repair after the hard repair (an open net routed through others at a price, the nets it displaces routed again hard, kept when the open terminals drop; docs/experiments/human-level-routing.md).
- `PCB_ROUTER_CAPACITATED_CORRIDORS`: overrides the router's `capacitated_corridors` (`0` off, anything else on; off by default): the negotiation's corridors from the global route with hard capacities, a search that misses its corridor lowering the capacity of the boundary it failed at and re-planning the nets through it (docs/router.md, "Capacitated corridors (2026-10-24)"; an experiment).
- `PCB_ROUTER_CAPACITATED_GROWTH`: the router's `capacitated_growth` (default 0; with `capacitated_corridors`): the capacitated corridor grows by a plan tile every this many reroutes of the net, as the tile planner's corridors grow; 0 keeps one tile (docs/router.md, "Capacitated corridors (2026-10-24)").
- `PCB_ROUTER_CAPACITATED_STUCK`: the router's `capacitated_stuck` (default 4; with `capacitated_corridors`): a net in conflict for this many iterations running lowers the capacity of its global route's boundary with the most conflicts, every second iteration; 0 turns that learning off (docs/router.md, "Capacitated corridors (2026-10-24)").
- `PCB_LAYOUT_MOVE_DESCENT`: overrides the layout's `move_descent` (`0` off, anything else on with layout.json's settings or the defaults): the move descent after the first route (docs/experiments/move-descent.md).
- `PCB_LAYOUT_FINAL_FROM_STATE`: overrides the layout's `final_route_from_state` (`0` off, anything else on): the final route starts from the router's state, its complete and legal nets kept (docs/placer.md, "Routes survive moves").
- `PCB_ROUTER_TROUBLE_PRESENT`: the trouble-makers' repair: the price of sharing a node in the trouble-maker's search (default 20).
- `PCB_ROUTER_TROUBLE_SHARE`: the trouble-makers' repair: its work as a share of the attempt's negotiation (default 0.5, at least 30 s).
- `PCB_ROUTER_TROUBLE_ROUNDS`: the trouble-makers' repair: rounds over the open nets (default 3).
- `PCB_ROUTER_TROUBLE_VICTIMS`: the trouble-makers' repair: the most nets one exchange may displace (default 12).
- `PCB_ROUTER_TROUBLE_PER_OPEN`: the trouble-makers' repair: the nets an exchange may displace per open terminal of the trouble-maker; its limit is the larger of this times its open terminals and `PCB_ROUTER_TROUBLE_VICTIMS` (default 2; 0 the flat limit).
- `PCB_ROUTER_TROUBLE_TIES`: the trouble-makers' repair: `1` keeps an exchange that closes as many terminals as it opens when fewer of the nets involved stay open (default 1; `0` refuses every tie).
- `PCB_ROUTER_TROUBLE_ORDER`: the trouble-makers' repair: the order of a round's trouble-makers, `open` (most open terminals first) or `few` (each surveyed by its priced search alone, the fewest victims per terminal closed first; default `open`).
- `PCB_ROUTER_TROUBLE_DEPTH`: the trouble-makers' repair: 2 lets a displaced net that came out worse displace others in turn (default 1).
- `PCB_ROUTER_TROUBLE_POURS`: the trouble-makers' repair: `1` lets pour nets take part, as trouble-makers and as victims, weighed by their pads off the pour's main piece (an experiment, off).
- `PCB_ROUTER_TROUBLE_LOCAL`: the trouble-makers' repair: `1` routes an exchange's victims again on their corridors first, and a victim left with more open terminals than it had in its window (not grown, never board-wide) while that could still make the exchange pass (default 1; `0` the full hard search for every victim).
- `PCB_ROUTER_TROUBLE_EARLY`: the trouble-makers' repair: `1` undoes an exchange as soon as the victims routed so far leave it no way to pass (default 1; `0` routes every victim first).
- `PCB_ROUTER_DIFF_PAIRS`: turns differential pairs on where the settings name none (`diff_pairs`, off; 0 off): route mode routes the pairs KiCad names (`_P`/`_N`, `+`/`-`, `P`/`N`) as coupled runs at the rules' width and gap, each link a net of the route negotiated with the others (docs/router.md, "Differential pairs").
- `PCB_ROUTER_DIFF_PAIRS_WORK`: differential pairs: the fallback router's work deadline in seconds, which routes what the coupled runs left open (default 0.05 of the ladder's budget, at least 1 s; for experiments).
- `PCB_ROUTER_BUS_BUNDLES`: sets the router's `bus_bundles` (an experiment, off): buses of four or more nets joining the same footprints route first, in pad order, inside one corridor, side by side (docs/experiments/bus-bundles.md).
- `PCB_ROUTER_BUS_LANE`: the router's `bus_lane_cost`: the factor on a bus member's step off the lane beside its neighbours (default 1.5).
- `PCB_ROUTER_BUS_MARGIN`: bus bundles: tiles a bus's corridor is grown by beyond half its width (default 4).
- `PCB_ROUTER_BUS_NO_CORRIDOR`: bus bundles, an ablation: the members keep their own corridors (the lane and the order stay).
- `PCB_ROUTER_BUS_NO_ORDER`: bus bundles, an ablation: the members keep their place in the usual routing order (the corridor and the lane stay).
- `PCB_ROUTER_BUS_PATIENCE`: bus bundles: the reroutes after which a member leaves its bundle and negotiates as any other net (default 8).
- `PCB_ROUTER_NO_SPOKELESS_TRACKS`: turns off the router's `spokeless_tracks` (a straight track into kept fill for a pad the spoke model sees without a spoke).
- `PCB_ROUTER_NO_APART_TREES`: turns off the router's `apart_trees` (terminals no path reaches from their net's tree joined among themselves).
- `PCB_ROUTER_NO_LOST_SPOKES`: turns off the router's `lost_spoke_repair` (other nets rerouted around a starved pad's cut spokes after the clean-up).
- `PCB_ROUTER_SEED`: `N`: perturbs the net order and seeds the history with noise, so that one board routes many ways.
- `PCB_NARROW_RESERVE`: `share`: the share of the ladder's first attempt's work kept back for the narrow-signal step (wins over the router setting `narrow_reserve_share`; default 0.75).
- `PCB_WALL_STRETCH_MAX`: `cap`: the most the wall-clock guards stretch for pinned rates faster than the machine measured (the pinned rate over the measured; default 4, 1: no stretch, the guards before 2026-10-21).
- `PCB_LADDER_SEARCH_ONLY`: route mode's ladder counts the attempts' search expansions alone: no manual spokes, pour analyses, lattices or lowerings, and its shares stay search work (the count before 2026-10-21; for attributing rows; was `PCB_LADDER_UNCHARGED_SPOKES`, which named only the spokes).
- `PCB_LADDER_UNCHARGED_POUR`: route mode's ladder counts the attempts' search expansions and manual spokes only, not their other pour analyses, lattice builds and lowerings, and its shares stay search work (the count before 2026-10-21, all pour work charged; for attributing rows).
- `PCB_LADDER_SEARCH_SHARES`: route mode's ladder charges all the work but gives each router its share as search work, not scaled by the routers' share of search so far (for attributing rows).
- `PCB_LADDER_NO_FIRST_ITERATION_GATE`: route mode's steps start without room for their first negotiation iteration (the rule before 2026-10-21; for attributing rows).
- `PCB_LADDER_NO_STEP_COST_GATE`: route mode's steps start with less left under the bound than a fifth of the ladder's first attempt's work (the rule before 2026-10-21, probe overhead; for attributing rows).
- `PCB_LADDER_SEARCH_LIMITS`: route mode's first attempt stops on its search alone, its pour analyses and lattice on top (the rule before 2026-10-21, probe overhead; for attributing rows).
- `PCB_LADDER_PROBES_COUNT_ALL`: route mode's rung probes stop on all their work (search, pour analyses, lattice and their part of the static maps) instead of on their search (an experiment: leaders changed; docs/changelog.md, probe overhead).
- `PCB_LADDER_CONTINUATION_LATTICE`: route mode's probed rung's continuation takes its share in search work by the routers' proportion of search with their lattices counted, as the fresh routers do (the rule before 2026-10-21, probe overhead; for attributing rows).
- `PCB_ROUTER_NO_SHARED_STATICS`: every router builds its own static maps and pour masks instead of sharing those of routers of the same board, lattice and classes (for attributing rows; the routing is the same).
- `PCB_ROUTER_LIMITS_BETWEEN_ITERATIONS`: a negotiation's work limits (its own and the work deadline) bind only after a whole iteration, not between its batches too (the rule before 2026-10-22, probe overshoot; for attributing rows).
- `PCB_PROBES_SIDE_BY_SIDE`: `N`: at most N probes of the probe ladder at once (1: one after the other).

Diagnostics (stderr unless said; the result does not change):

- `PCB_ROUTER_TROUBLE_WHY`: the trouble-makers' repair prints every exchange's outcome (kept, or why refused).
- `PCB_ROUTER_DIFF_PAIRS_DEBUG`: differential pairs: prints each launch, each link's path, the copper refused with the reason, each stub straightened or not, and the fallback's work.
- `PCB_ROUTER_BUS_WHY`: bus bundles: prints every bus found and every member broken out of its bundle.
- `PCB_ROUTER_DEBUG`: the router's verbose lines (failed searches, the resolution, what blocks a stub, lattice details).
- `PCB_ROUTER_DUMP`: `x0,y0,x1,y1`: prints that region of the lattice per layer (static blocks, own pads, occupancy, where vias may not go).
- `PCB_ROUTER_FINGERPRINT`: a hash of the routing state at each phase (determinism and byte-identity checks; compare as a multiset when attempts run side by side).
- `PCB_ROUTER_FINGERPRINT_ALL`: with `PCB_ROUTER_FINGERPRINT`, also hashes every occupancy map and pour mask into each fingerprint.
- `PCB_ROUTER_POUR_TRACE`: per pour analysis its counts, modelled work and wall time (calibrating the pour work constants).
- `PCB_ROUTER_SEVER_TRACE`: per refresh of what a cut severs of the pours (`pour_integrity`) its nodes, wall time and marked nodes per layer (calibrating `SEVER_NODE_NANOS`).
- `PCB_ROUTER_BUILD_TRACE`: per router build and per lowering their counts, modelled work and wall time (calibrating the build and lowering work constants).
- `PCB_ROUTER_POUR_DUMP`: `<dir>`: writes the pour maps at each finish.
- `PCB_ROUTER_SEARCH_RATE`: the search times itself for the `profile rate:` line (calibrating `REFERENCE_EXPANSIONS_PER_SECOND`).
- `PCB_ROUTER_TIMING`: where a reroute's time goes (layout moves).
- `PCB_ROUTER_SPOKE_DEBUG`: the spoke model's decisions per pad.
- `PCB_ROUTER_DUMP_MANUAL_SPOKES`: dump-pour-map also adds the manual spokes and says how many.
- `PCB_PLACER_DEBUG`: the placer's progress lines.
- `PCB_GLOBAL_DEBUG`: the global route's negotiation, a line per iteration (nets to reroute, overflow, search work; docs/global-route.md).
- `PCB_LAYOUT_DESCENT_CLASSIC`: `<n>`: the move descent's first `n` router trials are also tried and timed as the coupled loop tries a move (a clone, `Router::update`, a reroute without the polish) on a copy; the result does not change.
- `PCB_PLACER_TRACE`: `<file>`: per seed the weighted wirelength after each stage and per part how far each stage moved it.
- `PCB_PLACER_DESIGNER`: `<board>`: the designer's poses judged by the placer's model (their wirelength, what each illegal movable part meets), and with `PCB_PLACER_TRACE` in the trace beside ours.
- `PCB_PLACER_DESIGNER_LEVEL`: `full`: judge the designer's poses at the full spacing; `tight`, `touch`: with tight bodies, and courtyards touching (default: the smallest spacing, no halos).
- `PCB_PLACER_CHECK_OVERLAP`: the annealer checks its incremental overlap against a full recount.
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

The old-path A/B switches (each restored the code before a change) were
removed on 2026-10-16: the router's (`PCB_ROUTER_OLD_*`,
`PCB_ROUTER_NO_*` but `NO_WALL_GUARD`, `NO_SPOKELESS_TRACKS` and
`NO_LOST_SPOKES`, `PCB_ROUTER_REROUTE_FRACTION`, the speculation
switches, `PCB_ROUTER_CLEANUP_WINDOW`; changelog.md, "quality:
pcb-router") and the layout's, ladder's and placer's
(`PCB_KICAD_OLD_ZONE_ARCS`, `PCB_LADDER_OLD_RESERVE`,
`PCB_LADDER_FORCE_RUNG`, `PCB_LADDER_SHORT_RESUME`,
`PCB_LAYOUT_OLD_FIRST_ROUTE_BUDGET`, `PCB_LAYOUT_NARROW_CONTINUATION` and
`_FLOOR`, `PCB_PLACER_CHANNEL_SELF`, `PCB_JACOBI_BATCH`; changelog.md,
"quality: pcb-kicad"). `PCB_ROUTER_NO_ISLAND_STUBS` became
`PCB_ROUTER_ISLAND_STUBS` when the island stubs went off on 2026-10-21.

## Gotchas

- The repository is rustfmt-clean since 54eff39 and stays so: format Rust
  changes before committing (the old "never run cargo fmt" rule is gone).
- A cargo test of the library crates leaves `target/release/pcb-maker`
  stale: build before copying a snapshot (`scripts/snapshot.sh` does).
- `pgrep -f` and `pkill -f` match their own shell: a waiter on `pgrep`
  never ends, `pkill` kills the session. Kill by PID.
- No backticks in `git commit -m`.
- One task list per benchmark comparison: every tier defaults to the
  committed benchmarks/agent-tasks/tasks/v7 (the quick and short tiers to
  github-tasks-v6 before 2026-10-21; their boards are the same in both); a
  log against another list compares inputs, not binaries. compare.py's
  first line names both lists and says TASK LISTS DIFFER; rows on other
  prepared constraints, task entries or designer boards are marked INPUT.
- Benchmarks use a copied binary (`build/bin`); runs go side by side and
  the memory ledger keeps them apart (`pcb-maker ledger` lists who holds
  what; the tier lock only with `--lock`), load under 40; above a load
  of about 45 the wall guard cuts rows (CUT is no evidence). Pin CPU
  experiments with `taskset` on symmetric halves (logical CPUs n and n+16
  share a core).
- The machine (Ryzen 9 5950X, 16 cores, 32 threads, 62 GB, 32 GB swap):
  routes, layouts and KiCad's checks reserve their memory in the ledger
  ($XDG_RUNTIME_DIR/pcb-maker-ledger.json) before they allocate and wait
  for room (MemTotal less 6 GB for all reservations together); run.py
  admits a board when the ledger has room for its expected peak, with
  `--min-free-gb` a floor on MemAvailable beside it; a run that grows
  past its reservation with no room left ends with exit 4, not the OOM
  killer.
- KiCad 10: pinfunction is `NAME_NUMBER`; pad and text angles are absolute;
  copper items reference nets by name, and an unknown name becomes a new net
  unless a touching pad supplies one; DRC caps a violation type at 499 (199
  for some); its silkscreen DRC sometimes names the wrong pad; its DRC is
  not byte-deterministic (finding order, which pair names a missing
  connection).
- Freerouting's jar opens its GUI without `-de`/`-do` (`--help` too): always
  `-Djava.awt.headless=true -de ... -do ...`, or use `freerouting.py`.
