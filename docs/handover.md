# Hand-over (state as of 2026-10-16, main 54eff39, snapshot x1170)

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
a canonical orientation, annealing, legalisation and refinement; up to 16
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
work (search expansions at 3.15e6 a second, pour analyses and KiCad checks
by a model), so a layout is deterministic for its input and binary,
whatever the thread count, unless the wall-clock guard (1.1 times the
budget) cuts it (`cut` in the verdict). KiCad's ERC/DRC/connectivity
decide pass or fail (exit codes 0-4 and `verdict.json`).
`route-kicad-board` is the same without placement; `import-kicad-netlist`
starts an agent's board from a schematic (`--footprints-from` a designer's
board; starter constraints for key matrices). See
[router.md](router.md), [placer.md](placer.md),
[constraints.md](constraints.md), [agents.md](agents.md).

## Benchmarks that count

Only the current references; older sweeps and tiers are in
[changelog.md](changelog.md) and [benchmarks.md](benchmarks.md).

**Layout sweep v16** (x862, tasks v6, 109 boards harvested from GitHub,
each reduced to stacked parts plus a `constraints.json` derived from the
designer's board, 1800 s each, five jobs beside other agents):
**44 of 109 pass** (KiCad: 0 unconnected, no copper error beyond the
designer's, constraints met); 3527 unconnected in all, 82 new DRC errors,
17 rows cut by the wall clock. `build/github-layout-v16.log`; table in
[benchmarks.md](benchmarks.md#layout-sweep-v16-2026-10-15), triage in
[changelog.md](changelog.md#triage-of-v16-2026-10-15-x931--x932).

**Layout sweep v17**: running (tasks v7, the tier default parallelism,
under the lock). Fill in when it lands:

    v17 (x____, tasks v7): __ of 109 pass, __ unconnected, __ new errors, __ cut; build/github-layout-v17.log

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

How to run them (one tier at a time on the machine, under the lock, load
under 40 and 20 GB free; the tiers default to 2 search threads a board and
physical cores / 2 boards, 8 x 2 on the 5950X; quick and short tiers on
tasks v6, the schematic tier and v17 on tasks v7):

```sh
B=build/bin/pcb-maker-x1180
flock build/quick-tier.lock benchmarks/agent-tasks/run.py build/short-x1180 --tier short --binary $B > build/short-x1180.log
flock build/quick-tier.lock benchmarks/agent-tasks/run.py build/quick30-x1180 --tier quick --binary $B > build/quick30-x1180.log
flock build/quick-tier.lock benchmarks/agent-tasks/run.py build/sch-x1180 --tier schematic --binary $B > build/sch-x1180.log
benchmarks/agent-tasks/compare.py build/short-x1170.log build/short-x1180.log
```

The full sweep: `benchmarks/agent-tasks/run.py build/github-layout-v17
--tasks build/github-tasks-v7/tasks.json --binary $B --jobs 8 --threads 2`
under the same lock. Router questions go to route mode on fixed
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
   work). Built and off: `replicate_channels`, `supply_via_room`,
   `routing_demand`, `routing_room`, `bus_rows`, `shared_halos`,
   `anneal_far_side_gaps`. The legaliser is not the lever (4 %); removing
   halos loses routing. Next: routing room from congestion rather than pin
   count, bus-aligned rows for video, seeded orientation starts in the
   race, and seed-repeated pairs as the judge: the race's probes pick by
   margins smaller than their own noise (hackrf, 4in1-mini).
5. **Starved thermals as the only failure** on complete boards (rosco,
   Castor, mackerel-30, k30, mackerel-08, Hub, PixelWave, BAGEL; counts
   fell to 0-5 since the spoke model). `solid_starved_thermals` (opt-in: a
   starved pad gets a solid zone connection) would pass them; Florian has
   not decided whether it should be the default.
6. **Our open count against KiCad's.** The router counts connections per
   net, KiCad per missing pair; and the pour model's optimism adds. The
   pass rule trusts KiCad; the router's own count is for steering only.
7. jiran-ble-lite's GND (no pour, 2 layers, 20 of 40 pads unreached); d20's
   keepout pads; the 0xCB pcb-panel's zone refill, which kicad-cli does not
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
| Design notes | [router.md](router.md), [placer.md](placer.md), [constraints.md](constraints.md), [agents.md](agents.md), [congestion-model.md](congestion-model.md), [coupling.md](coupling.md) |
| Results tables | [benchmarks.md](benchmarks.md), [benchmarks/agent-tasks/README.md](../benchmarks/agent-tasks/README.md) (tiers), [benchmarks/github/README.md](../benchmarks/github/README.md) (harvest, Freerouting) |
| Sweep logs | `build/github-layout-v16.log` and `build/github-layout-v16/` (v17: `build/github-layout-v17*` once it lands), `build/github-freerouting-x738*.log` |
| Tier references | `build/quick30-x1152.log`, `build/short-x1170.log` (complete: `build/short-x1083.log`), `build/sch-x1161.log` |
| Snapshot binaries | `build/bin/pcb-maker-xNNN` (x1170 = main 54eff39); `scripts/snapshot.sh <n>` builds and copies one, never over an existing number |
| Tasks | `build/github-tasks-v6/` (quick and short tiers) and `build/github-tasks-v7/` (v6 plus uuids on six panels: schematic tier, v17), from `benchmarks/agent-tasks/from_pcbench.py --corpus benchmarks/github/boards.json`; tier lists in `benchmarks/agent-tasks/{quick,short,schematic}.json` |
| Harvested boards | `benchmarks/real/external/github/` (not in git; `benchmarks/github/manifest.json` has origin and licence); their schematics in `benchmarks/real/external/github-sch/` (`benchmarks/github/schematics.json`) |
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
`target/release/pcb-maker` stale). Measured on 2026-10-16 in a fresh
worktree at a load of 20-27 on the 5950X: about five minutes in all, half
of it compiling.

| Step | Time |
| --- | --- |
| `cargo test --release -p pcb-router -p pcb-kicad -p pcb-placer` | 165 s with the first build; the tests themselves about 15 s (455 tests) |
| `cargo test --release -p pcb-maker --test cli_exit_codes --test agent_walkthrough --test native_verification` (needs kicad-cli) | 125 s: a minute building the root package, 64 s for agent_walkthrough's three layouts, a second for the others |
| `python3 -m pytest -q benchmarks/agent-tasks` | 1 s |
| `cargo build --release` | seconds after the tests, which built the binary already |

Then `bash scripts/snapshot.sh <n>` for a binary to benchmark, the short
tier for iterating, the quick tier once as the merge gate.
`python3 scripts/check_links.py` checks the Markdown links (`--into
handover.md changelog.md` for the links into these two files).

## Switches

Environment variables the binary reads. Defaults are what an agent gets;
none is needed for normal use.

User-facing:

- `PCB_SKIP_NATIVE`: with the `auto` router config, no KiCad checks
  (machines without kicad-cli); the router's own count decides.
- `PCB_ROUTER_SEEDS=N`: with the `auto` router config, N seeds routed side
  by side and the best kept (the seeded portfolio).
- `PCB_ROUTER_NO_WALL_GUARD`: no wall-clock guard in the router, and the
  layout sets no wall-clock deadline; work alone decides (determinism
  checks, the walkthrough test).
- `PCB_PLACER_WORK_SECONDS`: place-kicad-board's placement work budget
  (default 600 s, the layout's share).
- `PCB_MAKER_CACHE_DIR`: root of the DRC cache and of the library-free
  kicad-cli configuration (default `$XDG_CACHE_HOME/pcb-maker`, else
  `~/.cache/pcb-maker`).
- `PCB_MAKER_NO_CACHE=1`: no DRC cache (both sides of a CPU pair).
- `PCB_MAKER_KICAD_USER_LIBS=1`: kicad-cli's DRC and ERC run with the
  user's KiCad configuration (global libraries) instead of empty tables.
- `PCB_KICAD_TIMEOUT`: seconds a kicad-cli report may take (default 900).
- `PCB_MIN_CONNECTION_FLOOR=warning|off`: floor track widths at the board's
  minimum connection width also when connection_width is a warning, or
  never (default: only at error severity).
- `PCB_KICAD_NO_NECK_REPAIR`: no connection-width neck repair after the
  final DRC.
- `PCB_MAKER_TEST_PANIC`: panic once a run has started
  (tests/cli_exit_codes.rs).

Diagnostics (stderr unless said):

- `PCB_ROUTER_DEBUG`: the router's verbose lines (failed searches,
  resolve, what blocks a stub, lattice details).
- `PCB_ROUTER_DUMP=x0,y0,x1,y1`: prints that region of the lattice per
  layer (static blocks, own pads, occupancy, where vias may not go).
- `PCB_ROUTER_FINGERPRINT`: a hash of the routing state at each phase
  (determinism and byte-identity checks; compare as a multiset when
  attempts run side by side).
- `PCB_ROUTER_FINGERPRINT_ALL`: also hashes every occupancy map and pour
  mask into each fingerprint.
- `PCB_ROUTER_POUR_TRACE`: per pour analysis its counts, modelled work and
  wall time (calibrating the pour work constants).
- `PCB_ROUTER_POUR_DUMP=<dir>`: writes the pour maps at each finish.
- `PCB_ROUTER_SEARCH_RATE`: the search times itself for the
  `profile rate:` line (calibrating `EXPANSIONS_PER_SECOND`).
- `PCB_ROUTER_TIMING`: where a reroute's time goes (layout moves).
- `PCB_ROUTER_SPOKE_DEBUG`: the spoke model's decisions per pad.
- `PCB_ROUTER_DUMP_MANUAL_SPOKES`: dump-pour-map also adds the manual
  spokes and says how many.
- `PCB_PLACER_DEBUG`: the placer's progress lines.
- `PCB_PLACER_TRACE=<file>`: per seed the weighted wirelength after each
  stage and per part how far each stage moved it.
- `PCB_PLACER_DESIGNER=<board>`: with the trace, the designer's poses
  beside ours and whether the placer would call them legal.
- `PCB_PLACER_DESIGNER_LEVEL=full`: judge the designer's poses at the full
  spacing (default: the loosest level).
- `PCB_PLACER_CHECK_OVERLAP`: the annealer checks its incremental overlap
  against a full recount.
- `PCB_LADDER_STATS`: the probe ladder prints each probe's other
  statistics (the ladder's judge).
- `PCB_KICAD_NECKS_DEBUG`: the neck finder per net and layer.
- `PCB_LABELS_DEBUG`: copper texts the label mover could not place.
- `PCB_OUTLINE_DEBUG`: the outline's area, vertices and cutouts.
- `PCB_PROBES_SIDE_BY_SIDE=N`: at most N probes at once (1: one after the
  other).
- `PCB_NARROW_RESERVE=share`: the share of the first attempt's work kept
  for the narrow-signal step (default 0.75).
- `PCB_ROUTER_SEED=N`: perturbs the net order and seeds the history with
  noise, so one board routes many ways.

Old-path A/B switches, **being removed** by the router and kicad quality
packages (each restores the code before a change, for A/B runs):
`PCB_ROUTER_OLD_*` (`FILL_CLEARANCE`, `HOPELESS`, `POLYGON_DISTANCES`,
`RESOLVE`, `SPOKE_CLEAR`, `SPOKE_FRAMES`, `SPOKES`, `TRACKED`,
`TRACK_ENDS`, `TRACK_PIECES`, `TRACK_RUNS`, `TRACKS_FILL`),
`PCB_ROUTER_NO_*` (`FOREIGN_SPOKES`, `INNER_PADS`, `INNER_SPOKES`,
`ISLAND_STUBS`, `ISLAND_VIAS`, `MANUAL_SPOKES`, `NECK_JOINS`,
`ZONE_SEAMS`), `PCB_KICAD_OLD_ZONE_ARCS`, `PCB_LADDER_OLD_RESERVE`,
`PCB_LAYOUT_OLD_FIRST_ROUTE_BUDGET`, `PCB_LAYOUT_NARROW_CONTINUATION` and
`_FLOOR`. Other experiment hooks in the code, undocumented on purpose:
`PCB_JACOBI_BATCH`, `PCB_LADDER_FORCE_RUNG`, `PCB_LADDER_SHORT_RESUME`,
`PCB_ROUTER_CLEANUP_WINDOW`, `PCB_ROUTER_REROUTE_FRACTION`,
`PCB_ROUTER_NO_SPECULATION`, `PCB_ROUTER_SPECULATE_PATHS`,
`PCB_ROUTER_SPECULATION_CHECK`, `PCB_ROUTER_SPECULATION_NO_TILES`,
`PCB_PLACER_CHANNEL_SELF`, `PCB_TOPO_*`,
`PCB_MAKER_KICAD_VERIFICATION_CACHE*`; test fixtures
`PCB_MAKER_JOINT_RETAINED_FIXTURE_ROOT`, `PCB_MAKER_KEEP_TEST_ARTIFACTS`.

## Gotchas

- The repository is rustfmt-clean since 54eff39 and stays so: format Rust
  changes before committing (the old "never run cargo fmt" rule is gone).
- A cargo test of the library crates leaves `target/release/pcb-maker`
  stale: build before copying a snapshot (`scripts/snapshot.sh` does).
- `pgrep -f` and `pkill -f` match their own shell: a waiter on `pgrep`
  never ends, `pkill` kills the session. Kill by PID.
- No backticks in `git commit -m`.
- One task list per benchmark comparison: quick and short tiers default to
  github-tasks-v6, the schematic tier to v7; a log against one on the
  other list compares inputs, not binaries. compare.py's first line names
  both lists and says TASK LISTS DIFFER; rows on other prepared constraints
  are marked INPUT.
- Benchmarks use a copied binary (`build/bin`); one tier at a time under
  `flock build/quick-tier.lock`, load under 40, 20 GB free; above a load
  of about 45 the wall guard cuts rows (CUT is no evidence). Pin CPU
  experiments with `taskset` on symmetric halves (logical CPUs n and n+16
  share a core).
- The machine (Ryzen 9 5950X, 16 cores, 32 threads, 62 GB, 32 GB swap):
  the runners wait for free memory (`--min-free-gb`) and the ladder
  projects its own.
- KiCad 10: pinfunction is `NAME_NUMBER`; pad and text angles are absolute;
  copper items reference nets by name, and an unknown name becomes a new net
  unless a touching pad supplies one; DRC caps a violation type at 499 (199
  for some); its silkscreen DRC sometimes names the wrong pad; its DRC is
  not byte-deterministic (finding order, which pair names a missing
  connection).
- Freerouting's jar opens its GUI without `-de`/`-do` (`--help` too): always
  `-Djava.awt.headless=true -de ... -do ...`, or use `freerouting.py`.
