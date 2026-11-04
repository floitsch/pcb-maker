# Hand-over (state as of 2026-11-04; measured by layout sweep v31, the nightly of 2026-11-04 on main 688b59c)

What the tool does today, which benchmarks count, what is known to be
wrong, where everything is and how to work on it. What each round of work
measured and changed is in [changelog.md](changelog.md), newest first
(the dated "Committed on ...", triage, sweep and measurement sections that
used to be here; code comments and docs that cite `docs/handover.md,
"Committed on ..."` mean those sections). Longer-lived ideas are in
[todo.md](todo.md); the commits themselves in git.

## State on 2026-11-03

- main 3d20d0a8 (x1882). The nightly of 2026-11-03 (main 5657f45, before the race's count flip, x1881, and the pin-row gaps after a move, x1882): sweep v30 **62 of 109 strict** (62 since v25 on 2026-10-29), matched designer tier 68 strict, the gap over its 100 target boards placement 76 % / routing 24 %, expansion 24/46, short 8/12, fingerprints identical.
- Two placer defaults won by the placer-default protocol: crossings in the anneal (-10.8 %, p 0.002; on since 2026-10-28) and the race's ripped count (nine pairs, -5.6 %, p 0.014; `race_count` `"ripped"` since 2026-11-03).
- Built and on: panels of different boards with mouse-bite and V-cut tabs and frames kept clear (`sub_boards`), filled and capped vias in BGA pads where the designer's board has vias there (`via_in_pad`), length and skew rules on a layout's own routes, the input's own findings set apart without a reference, the place check placing as the layout does, `drc-kicad-board` / `fab-kicad-board --input`.
- Built, measured and off: the fan-out step (`fanout`), the last connections' ejection chains (`last_connections`), frames as keepouts on panels of copies.
- Cheaper with the same results: the placement -22.6 % and -29.4 % CPU (two placer rounds, dense geometric means), the router -3.9 % (round 3).
- Measuring: the ladder of placer measurements (stage 1 the first route, `stop_after_first_route`; the quarter budget rejected, condensed boards a no-go), run.py `--reuse`, the memory ledger draining past its bound.
- In flight: panels of copies' frames, placer rules, the designer as teacher on the largest-gap boards (the placement gap's diagnosis), placer round 4.

## The flow in one paragraph

`layout-kicad-board` reads the board in a canonical order (footprints by
reference, their pads and graphics sorted, so two saves of one design lay
out alike), places the free footprints (ePlace-style global placement from
a canonical orientation, annealing with the ratsnest crossings in its cost
since 2026-10-28, legalisation and refinement, with the escape demand of
each part's sides in the density, the rotations, the legaliser and the
refinement since 2026-10-24; a panel of copies places one copy and
replicates it (`panel_copies`), a panel of different boards places each
board in its own piece with its nets split and no part on a tab or a
frame (`sub_boards`: the outline cut at necks, mouse-bite rows and V-cuts);
up to 16 placer seeds run in a pool of the search threads and are judged
in seed order; the race ranks them by RUDY and their crossings and probes
the best three with a 75 s route, judged since 2026-11-03 by the open
terminals after ripping the conflicted branches, docs/placer.md "The race
ranks its probes by the ripped count"), routes the board with the lattice
router (PathFinder negotiation; pours as fill the pads join through
thermal spokes modelled on KiCad 10's filler and DRC, stranded pads routed
as tracks, islands stitched, manual spokes where the model finds a pad
short), tries footprint moves (the pin-row gaps follow a moved part since
2026-11-03), resumes the first route briefly, polishes (clean-up and via
reduction) without renegotiating, and - when connections stay open - runs
the attempt ladder (probe every pour mode, continue the best; then the
regular rungs, finer pitches and seeds; with work kept in reserve, a last
attempt with signals at the project's narrowest predefined width). A
four-layer board with pours that is far from complete after the first
route takes that narrow attempt directly, then, with 240 s of work left,
the same attempt in the other pour mode. KiCad's connection-width findings
(necks and gaps) are repaired afterwards with KiCad's own fill. The
project's custom rules are kept where the router can (the verdict's
`custom_rules` says which and how), `length` and `skew` rules by meanders
and a length repair on the layout's own routes too, and a footprint whose
small pads the designer's board holds vias in gets a filled and capped
via in each pad no track can leave (`via_in_pad`). Every budget is in work
(search expansions and placement units at rates measured at the start,
the reference's 3.15e6 and 150e6 a second scaled to the machine; pour
analyses and KiCad checks by a model), so a layout is deterministic for
its input, binary and rates (`PCB_WORK_RATE` pins them; the tiers pin the
reference's), whatever the thread count, unless the wall-clock guard
(`wall_guard`, 2 times the budget) cuts it (`cut` in the verdict). KiCad's
ERC/DRC/connectivity decide pass or fail (exit codes 0-4 and
`verdict.json`; without a reference, a footprint's own findings and the
input's own edge, mask and courtyard findings do not count).
`place-kicad-board` is the layout's placement alone (its budget share,
seeds and router rules, at the reference's rates by default: at the same
rates the layout's placement byte for byte). `route-kicad-board` is the
layout without placement; `import-kicad-netlist` starts an agent's board
from a schematic (`--footprints-from` a designer's board; starter
constraints for key matrices). See [router.md](router.md),
[placer.md](placer.md), [constraints.md](constraints.md),
[agents.md](agents.md).

## Benchmarks that count

Only the current references; older sweeps and tiers are in
[changelog.md](changelog.md) and [benchmarks.md](benchmarks.md).

The newest nightly measurement of main (scripts/nightly.sh; the table of
the last 14 nights is in [benchmarks.md](benchmarks.md#nightly)):

<!-- nightly-state:begin -->
**Nightly** 2026-11-04 (main 688b59ca8682, `build/nightly/2026-11-04/summary.md`): fingerprints identical; expansion 24/46 (strict 24), 558 open; short 7/12 (strict 7), 22 open; sweep 61/109 (strict 61), 2514 open; designer 71/109 (strict 68), 2759 open at 0.675; gap placement 73%, routing 27%.
<!-- nightly-state:end -->

What main does now, by the nightly of 2026-11-03 (main 5657f45; it ran
before the race's count flip, x1881, and the pin-row gaps after a move,
x1882: the next night is the first to measure them):

| Stage | Boards | Result | Unconnected |
| --- | ---: | --- | ---: |
| Layout sweep (v30; stacked parts and constraints, 1500 s of work) | 109 | **62 strict** (62 per type), 2 without a result; 62 on every night since v25 (2026-10-29), 61 on v24, 59 on v23, 57 on v22 | 2658 |
| Matched designer tier (the designer's placement at the layout's routing budget, scale 0.675) | 109 | **68 strict** (71 per type) | 2759 |
| The gap over the 100 target boards (`compare.py --gap`) | 100 | placement **76 %** (+1084), routing **24 %** (351) of the layout's 1435 open | - |
| Expansion tier (route mode, 8 x 1) | 46 | **24 strict** | 558 |
| Short tier (12 flip-prone boards, a third of the budget, 3 x 2) | 12 | **8 strict** | 23 |
| Fingerprints (Castor, k30-SBC, jetson, Sisu; route mode, a quarter budget) | 4 | identical night to night | 42 |

The short tier for the two since: the race's ripped count by default
(x15192, the flip) against x1880: 8 / 8 strict passes, 11 of 12 boards
identical, WLED_Matrix 16 to 14 unconnected; the pin-row gaps after a
move (`pcb-maker-gaps1-a`) against x1877: 23 unconnected and 8 strict
passes on both sides (changelog, 2026-11-03).

**Layout sweep v31** (the nightly of 2026-11-04, main 688b59c, written
by the nightly): **61 of 109 pass the strict rule** (61 the per-type
rule), 2514 unconnected in all, strict errors 35, no row cut or killed,
2 without a result; medians against the designer's board: vias x0.69,
length x1.04, open 0.000 (mean 0.094), bends x1.12, stair-steps +50,
crossings +1. Against the nightly of 2026-11-03: better 9, same 94,
worse 4, strict passes 62 -> 61 (gained 0xCB-1337 pcb-panel; lost
MIDAS-MK1.1-revA, OpenAirScope). The matched designer tier on the same
binary (budget scale 0.675): 68 strict passes (71 per type); over the
100 target boards placement 73 %, routing 27 % of the open connections.
`build/nightly/2026-11-04/`; table, gap split, the costliest placements
and the losers in
[benchmarks.md](benchmarks.md#layout-sweep-v31-2026-11-04).

The sweeps before (v16 to v21 by hand, v22 to v29 written by the
nightly), their tables, gap splits and triage are in
[benchmarks.md](benchmarks.md) and [changelog.md](changelog.md).

**Not run by the nightly.** The quick tier (30 boards, full budget; last
reference `build/quick30-x1152.log`, tasks v6: 24 of 30, predating order
independence, so not a base) and the schematic tier (12 boards from the
designer's schematic, a third of the budget; last `build/sch-x1161.log`
of 2026-10-16: 10 of 12). Run them when a change touches their path.

**Against other routers** (complete; Florian 2026-10-23: the head-to-heads
stand and are not rerun): Freerouting 2.2.4 and KiCadRoutingTools on the
v17 placements (x1367, 2026-10-21 to 23) and Freerouting against our cold
route (x738, 2026-10-15): README.md, "Status", and
[benchmarks.md](benchmarks.md#freerouting-against-x1367-2026-10-21).

**PCBench / PCBWorld D3** (2026-09-29): 110 of the 114 D3 agent tasks
laid out clean ([the tasks README](../benchmarks/agent-tasks/README.md#2026-09-29-the-114-d3-test-boards)).

How to run them (side by side: run.py admits each board against the
machine's memory ledger, which every binary reserves in too; `--lock`
takes build/quick-tier.lock for one tier at a time; load under 40; the
tiers default to 2 search threads a board and physical cores / 2 boards,
8 x 2 on the 5950X; every tier on tasks v7; the nightly's own arguments
are in scripts/nightly.sh):

```sh
B=build/bin/pcb-maker-x1882
benchmarks/agent-tasks/run.py build/short-x1882 --tier short --jobs 3 --threads 2 --binary $B > build/short-x1882.log
benchmarks/agent-tasks/run.py build/exp-x1882 --tier expansion --jobs 8 --threads 1 --binary $B > build/exp-x1882.log
benchmarks/agent-tasks/compare.py build/nightly/2026-11-03/short.log build/short-x1882.log
```

The full sweep: `benchmarks/agent-tasks/run.py build/layout-x1882
--tasks benchmarks/agent-tasks/tasks/v7/tasks.json --binary $B --jobs 10 --threads 2 --quality --order peak-desc`
admitted by the same ledger (the nightly runs it; a package does not).
Router questions go to route mode on fixed placements (`--route-from
<earlier output dir>`), CPU claims to pinned simultaneous pairs; the
discipline is in the tasks README, "How to measure"; placer defaults
under "Working on it" below.

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

The numbers are kept from earlier hand-overs (other docs cite "open
problem 2", "4", "5", "8"); done items are gone, their sections are in the
changelog. **The rank by the evidence of 2026-11-03**: 4 (placement, 76 %
of what stays open), 13 (deciding a placer default costs a day), 12 (the
near misses), 2 (crossings at fine-pitch rows), 1 (fill fidelity), 3 (the
ladder's judge), 5, then the rest.

1. **Fill fidelity, what is left.** The starved-thermal census (44 boards,
   `experiments/pour-fidelity/census.py`) stands at 162 predicted, 7
   missed, 26 false alarms, 2 not modelled (x1054). Left: spoke landing and
   piece fidelity against KiCad's kept outlines (our map is in far more
   pieces: lattice sampling of necks), island-rule and track-rule edge
   cases, custom spoke templates, hatched fill over thermal pads without
   relief, per-zone fill settings and islands a zone seam alone makes,
   fine-pitch clearance within a few microns. KiCad's teardrops and its
   hatched iterative refill are not modelled; connection-width necks are
   repaired after the fact with KiCad's fill, gaps too since 2026-10-27
   (bridges over every slot that can be one, pruned by KiCad's checks:
   docs/router.md, "KiCad's connection width (necks)").
2. **Crossings at fine-pitch rows** (re-scoped 2026-10-28; it was "one-spoke
   pads on fine-pitch rows", k30's U10 pad 8). The fan-out step, every
   pad of a dense row given a way out jointly before the negotiation, is
   built, measured and off (`fanout`; docs/router.md, "Fan-out before
   negotiation"): on the near misses' fine-pitch rows (Quanta75's RP2040
   and USB-C, OpenAirScope's UDFN) every pad has a straight way out on
   the statics and the row's joint assignment serves all of them; what
   closes a pad is a neighbour's via just beyond its tip or a neighbour's
   track turning across it, where the two nets' targets cross (one needs a
   via, which in a 0.4 mm row fits only beyond the neighbours' tips).
   Reserving the chosen ways traded which pad stays open (OpenAirScope's
   CMD for DAT3) and lost passes elsewhere (the expansion tier 558 to 510
   unconnected but strict 24 to 23). k30's U10 pad 8 is a pour net's,
   which the step leaves out; the old repair (`free_thermal_spokes`)
   moves 2 of 10 nets there. Found on the way and fixed on
   2026-11-03 (x1882): the layout's moves rebuild the pin-row gaps
   (docs/router.md, "Pin-row cost"). The crossing decided in the repair
   is built, measured and off (`crossing_dive`; docs/router.md,
   "Crossings decided in the repair (2026-11-03)"): where an open net's
   exchange is refused and a row neighbour whose target crosses closes
   its pad's way out, one net of the pair takes a via at a fan-out via
   site beyond the row and the other stays, both tried, the best kept.
   The census (`PCB_ROUTER_TROUBLE_WHY`) puts 3 of the five boards' 9
   open items down to crossing pairs, all Quanta75's RP2040 row; there
   every dive closes the crossing and opens a third net (the via lands
   on the key matrix's rows on the other layer: "1 open to 1"), so the
   trade moved one net further on. Left: the via's landing chosen with
   the other layer's routes (a site priced by what it displaces there,
   or the displaced net's own dive), and OpenAirScope's UDFN and the
   keyboard's aQFN pads, which have no straight way out at all.
   Grid arrays are the other case, measured on 2026-11-04 (docs/router.md,
   "Prepared fan-outs for BGAs"): where a ball has a way out the search
   can take (the outer rings, a depopulated middle, a via in the pad) it
   finds it (vd-rd's BGA-96, MIDAS, the aQFNs); where the class's via
   does not fit a diagonal gap (Glasgow's U30: 0.6 mm via, 0.8 mm pitch,
   0.35 mm balls) the inner rings' balls stayed open and their 34 nets
   took 27 to 47 % of the search. The prepared fan-out (`bga_fanout`,
   **on**) gives each such ball a stub and a 0.4/0.2 via in its gap
   before routing, as the designer did: Glasgow on the designer's
   placement 55 to 15 open, on the night's placement 123 to 107, layout
   over three seeds 372 to 343, nothing else moves. What stays on
   Glasgow is its outer rings' surface escapes, ringed by our passives
   (open problem 4).
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
4. **Placement: 76 % of what stays open.** The matched designer tier of
   2026-11-03 routes the designer's placement of the 100 target boards to
   351 open connections where our layouts leave 1435: placement +1084
   (worse on 29 boards, the same on 64, better on 7), routing 351. The
   costliest: video +201 (202 / 1), Batch_1 +153 (156 / 3),
   SmartSpin2k_Panelized +115 (115 / 0), SNSP-CPU-01 +76 (111 / 35), the
   OpenESC 4in1 +73 (88 / 15). The share was 52 to 57 % up to the nightly
   of 2026-10-28 (69 to 73 % without Batch_1) and 74 to 76 % since.
   The week's levers judged by the protocol: crossings in the anneal
   BETTER (on), the race's ripped count BETTER at nine pairs (on); judged
   before it and not taken: bus-group moves (the objective falls, routing
   does not), `legality: "kicad"`, `decoupling_near`; built and off
   earlier: `routing_demand`, `routing_room`, `shared_halos`,
   `anneal_far_side_gaps`; removed on 2026-10-16: `replicate_channels`,
   `supply_via_room`, `bus_rows`. The legaliser is not the lever (4 %);
   removing halos loses routing; more seeds do not close it (the
   designer's placement routes the OpenESC 4in1 to 15 open where ours
   leaves 88). Untested: exit blockage (the fan-out assignment) as a placement
   cost, length maxima known to the placer (oro link's D5 and D6 are over
   their maximum by their pads' distance), routing room from congestion
   rather than pin count, seeded orientation starts in the race. The back
   is kept free under a BGA's enclosed pads (`via_in_pad_far_side`, on
   since 2026-10-28; MIDAS-MK1's B2 holds its via with the room below
   on and off), and since 2026-11-04 the room around a grid array whose
   inner balls take the fan-out's dog-bones is **on** (`bga_room`;
   docs/placer.md, "Room around grid arrays (2026-11-04)"): on Glasgow
   the designer keeps U30's outer rings' surface escapes 1.3 mm and more
   free on its side (nine in ten channels 1.66 mm, the median 6.6) where
   ours packed parts 0.25 mm from the ball field; with a band of 1.5 mm
   on its side and the back under it for its own decoupling, Glasgow's
   layouts go 343 to 262 unconnected over seeds 1, 101 and 301 (each
   seed better, the outer rings' open balls 125 to 89) and its first
   route is better on all five protocol seeds (716 to 571). A room for
   MIDAS's U106 too (vias in pads) gained MK1.1's strict pass and cost
   MK1's at one of three seeds; it is left out, as is vd-rd's U200
   (every ball on the surface: 3 to 6 open with a band). Every other
   board places byte for byte as before. In flight on
   2026-11-03: the designer as teacher on the largest-gap boards, placer
   rules, panels of copies' frames (left out on 2026-10-28: SmartSpin2k
   118 to 89, but the 0xCB panel lost its strict pass), placer round 4.
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
7. d20's keepout pads (`bitshiftcrazy__d20_pcb`, "Not targets"); the
   0xCB pcb-panel's zone refill, which kicad-cli does not finish in 900 s
   (`PCB_KICAD_TIMEOUT`). jiran-ble-lite passes since 2026-10-28: its
   halves are sub-boards by their mouse bites and its GND across them is
   the designer's own, not counted.
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
    warnings from our own 0.09 mm tracks; the neck repair's Keypad
    unconnected item; a pad's own mask expansion in the placer's
    opening rule; the molecule panels' back connectors need the designer's
    0.03 mm gaps; the board's top-level graphics, zones and tracks still
    follow the file's order; the import's netlist export, preview SVG and
    fab exports still run kicad-cli on the user's library configuration.
12. **The near misses.** Of the 47 boards the sweep v30 fails, 16 are
    within five connections and five errors of clean (README.md's
    headline; the list changes with every night). The last diagnosis,
    2026-10-27 (12 boards, 34 open items; docs/router.md, "The last
    connections"): 10 the input's (the designer's board has them), 5 a
    rule, 10 placement, 1 the lattice at the class's width, 8 routing; no
    routing item was a budget that ran out (the trouble repair refused
    every exchange as a pure trade). Closed since: the panel tabs and
    frames (ULK 4 to 0, ULK_sl_PG1316s 5 to 0, jiran 1 to 0, all strict
    passes; woagboard 4 to 1, its designer's own GND island), vias in BGA
    pads (MIDAS-MK1.1 5 to 0, a pass), two faults of the trouble repair.
    `last_connections` (ejection chains three deep, board-wide victims)
    closed none and is off. Left: the fine-pitch rows (problem 2), the
    keyboard's RESET (open on the designer's placement too), MIDAS-MK1's
    B2 (its via refused: our placement put an 0603 under it on the back).
    The next diagnosis takes the night's list, not 2026-10-27's.
13. **Deciding a placer default costs a day.** The placer-default
    protocol (benchmarks.md, "Measurement discipline: placer defaults")
    is 110 full layouts a lever, about 88 CPU hours and 18 hours of wall
    beside the nightly; five seed pairs resolve a weighted change of
    about 8.5 %, nine about 6.5 %. The ladder (benchmarks.md, "The ladder
    of placer measurements"): stage 0 the placement's own metrics (a
    sanity check, not a filter), stage 1 the first route alone
    (`stop_after_first_route`: 0.55 to 0.57 of the full stage's cost in
    the calibration, 0.67 of the CPU beside full layouts of one binary;
    the full verdict on both calibrated experiments), stage 2 the full
    protocol. Rejected: the quarter budget (2026-11-03: two boards find
    no legal placement, the rows do not rank the full rows), condensed
    boards, coarse whole first routes as the race's probe. run.py
    `--reuse` saves the rows a lever leaves alone when both sides run on
    one binary with the lever as an option. Open: a stage that keeps the
    placement's budget and cuts only the routing (a layout option not
    built; placer.md, "The quarter budget (2026-11-03)").

## Where things are

| What | Where |
| --- | --- |
| Current state, open problems | this file |
| What each round measured and changed | [changelog.md](changelog.md), newest first |
| Design notes | [router.md](router.md), [placer.md](placer.md), [constraints.md](constraints.md), [agents.md](agents.md), [congestion-model.md](congestion-model.md), [coupling.md](coupling.md), [global-route.md](global-route.md) |
| Results tables | [benchmarks.md](benchmarks.md), [benchmarks/agent-tasks/README.md](../benchmarks/agent-tasks/README.md) (tiers), [benchmarks/github/README.md](../benchmarks/github/README.md) (harvest, Freerouting) |
| Sweep logs | the nightly's `build/nightly/<date>/` (`sweep/`, `designer/`, `short/`, `expansion/`, `fingerprints/`, `summary.md`, `gap.txt`); the hand-run sweeps `build/github-layout-v16*` to `-v21*`; `build/github-freerouting-x738*.log` |
| Tier references | the newest nightly's `short.log`, `expansion.log` and `fingerprints.log`; not in the nightly: `build/quick30-x1152.log`, `build/sch-x1161.log`; the placer protocol's noise file `benchmarks/agent-tasks/dense-noise.json` |
| Snapshot binaries | `build/bin/pcb-maker-xNNNN` (four digits: main's, made by the gate; x1882 = main 3d20d0a8), a package's own as `x<five digits>` from its range or by its name (`pcb-maker-fan1-a`); `scripts/snapshot.sh <n>` builds and copies one, never over an existing number; the nightly's `pcb-maker-nightly-<date>-<sha>` |
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
use and name it in the hand-off. The four-digit numbers (`x1882`) are
main's, made by the gate when it merges: a package never names a build
`pcb-maker-xNNNN`; it takes a five-digit range (`x15210`-`x15219`) or
names its builds by the package (`build/bin/pcb-maker-fan1-a`). Commit
on the branch; main is merged by the coordinator.

**The gate's rules.** A package works in its worktree only: it never
checks out, commits to, merges, rebases onto or pushes main (the
coordinator merges through a gate and says when main moved: rebase
then). Stage paths by name (`git add docs/handover.md`), never `git add
-A` or `git commit -a`, and never commit anything under `artifacts/`,
`build/` or `benchmarks/real/external` (links into the main checkout:
a committed link once wiped the harvested boards on a merge). The gate
runs `cargo fmt --check` (the repository is rustfmt-clean), workspace
clippy and `bash scripts/check.sh`; a docs-only package must add no
broken link among the repository's own files (`python3 -I
scripts/check_links.py`; the links into node_modules are old noise).

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

Then `bash scripts/snapshot.sh <n>` for a binary to benchmark and the
short tier for iterating; the quick tier is no longer the merge gate
(what a package measures is below).

**What a package measures, and what the nightly does** (since
2026-10-23: validating every package with every tier slowed the project).
A router package runs the fingerprints (Castor, k30-SBC, jetson, Sisu in
route mode from v16, 1 x 4, budget scale 0.25, `PCB_ROUTER_FINGERPRINT=1
PCB_ROUTER_NO_WALL_GUARD=1`) and the route-mode expansion tier (8 x 1),
each against main's; a placer or layout package adds the short tier
(3 x 2) and, for identity, the eleven dense boards at seed 1 (layout,
3 x 2). No package runs a full sweep or the designer tier: `scripts/nightly.sh`
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

**Deciding a placer default** (benchmarks.md, "Measurement discipline:
placer defaults"; the 5-pair rule). On the eleven dense boards in layout
mode at the task budgets (tasks v7, pinned rates, 3 x 2, through the
ledger): five seed pairs, placer seeds 1, 101, 201, 301, 401 (16 apart:
the race places seeds N to N + 15), judged by `placer_ab.py compare
--off ... --on ...` (the weighted relative change against the noise
file `dense-noise.json`). BETTER at p <= 0.03; at 0.03 < p <= 0.25 run
seeds 501 to 801 and judge the nine pairs at p <= 0.03; otherwise not
better. The other conditions stay: the short tier loses no strict pass
and gains no DRC error; **a strict pass the short tier loses at its
seed** is checked on that board alone at the five seed pairs, and the
condition holds when the on side passes as often as the off side
(framework_mobo_lefthalf under crossings in the anneal: 4 of 5 on, 3 of
5 off). Climb the ladder first (benchmarks.md, "The ladder of placer
measurements"): stage 1 is the same five pairs with `--layout
'{"stop_after_first_route": true}'` judged by `--metric first_open`; a
lever that fails it costs about half the full stage. With the lever as
an option on one binary, run.py `--reuse build/layout-reuse` takes every
row whose placement digest (`layout: placement digest` on stderr,
`placement_digest` in board-layout.json) a stored row has
(benchmarks/agent-tasks/README.md, "Reused layouts"); a switch that lands
in the lowered problem, or two binaries, never hit. When main changes a
dense board's layouts, its off rows and its noise-file rows need running
again.

**The memory ledger's rules** (benchmarks.md, "Memory ledger"). run.py
declares each board's expected peak from `estimates.json` (the measured
rows of its kind; `--dry-run` prints each declaration and its source).
An `--expected-gb` given by hand overrides every board's: give it only
from a measured peak (estimates.json, an earlier row's `peak_rss_mb`),
never a round placeholder, and a harness of your own reserves what it
measured too: placeholders far above use hold room other agents and the
nightly wait for (2026-10-28: one agent's pairs held most of the
ledger's capacity while using a few per cent of it). CPU pairs reserve
for both sides at once and start both inside it
(`PCB_MAKER_LEDGER_INSIDE`): a pair whose one side waited is void.

## Switches

Environment variables the binary reads, one line each. Defaults are what
an agent gets; none is needed for normal use. The index is generated from
the code (tools/config-reference: every `PCB_*` name in the crates'
sources needs its line there, and `cargo test --test config_reference`
fails until the index is regenerated); the config keys are in
[config-reference.md](config-reference.md).

**Settings whose defaults changed or were decided since 2026-10-27**
(config keys; [config-reference.md](config-reference.md) has each with
its doc comment):

- `ratsnest_crossings.anneal` (placer): **on** since 2026-10-28 (x15201;
  the placer-default protocol: -10.8 %, p 0.002).
- `race_count` (layout): **`"ripped"`** since 2026-11-03 (x1881; nine
  pairs: -5.6 %, p 0.014): the race ranks its probes by the open
  terminals after the rip-up of the conflicted branches, with a 2 %
  margin over the first-ranked placement; `"unfinished"` is the race
  before.
- `bga_fanout` (router): **on** since 2026-11-04 (`PCB_ROUTER_BGA_FANOUT`;
  docs/router.md, "Prepared fan-outs for BGAs"): the inner balls of a
  grid array whose net's via does not fit a diagonal gap get a dog-bone
  (stub and via of the project's sizes) before routing, written where the
  route uses it; the placer keeps the far side free under those vias.
  Only Glasgow's U30 among the task boards takes a plan.
- `bga_room` (placer): **on** since 2026-11-04 (docs/placer.md, "Room
  around grid arrays"): around a grid array whose inner balls take the
  fan-out's dog-bones (Glasgow's U30, the only one among the task
  boards), a band on its side for the outer rings' surface escapes and
  only its own decoupling under it on the far side. Glasgow 343 -> 262
  unconnected over three seeds, its first route -20 % over five; nothing
  else moves. `"bga_room": false` turns it off.
- `fanout` (router): **off** (`PCB_ROUTER_FANOUT`): measured on
  2026-10-28, it traded which pad of a crossing pair stays open.
- `last_connections` (router): **0, off** (`PCB_ROUTER_LAST_CONNECTIONS`):
  it closed none of the near misses' connections.
- `via_in_pad` (router): **unset, the evidence rule** since 2026-10-28: a
  filled and capped via goes only into the enclosed pads (no track at
  the neck width leaves between the neighbours) of a footprint whose
  small SMD pads the designer's board holds vias in (the input board's
  own vias before `--strip`, or the `--reference` board's); a list
  (`["U106"]`) opts footprints in, `true` and `false` as before. run.py
  passes the designer's vias in pads as entries (`--designer-via-in-pad
  off` turns that off).
- `sub_boards` (placer) and `sub_board_nets` (router): **on**; since
  2026-10-28 rows of mouse-bite holes and V-cut lines are tabs too, and
  no part is placed on a frame (a piece without parts) or within 1.5 mm
  of a tab.
- `length_matching` (router): on, and since 2026-10-28 on a layout's own
  routes too (meanders, the length repair).
- `stop_after_first_route` (layout): **off**; the ladder's stage 1 (the
  layout ends with its first route's board, KiCad's check and the report,
  `stopped_after_first_route`).
- run.py `--reuse DIR`: off unless given; the placement digest's cache
  of layouts (benchmarks/agent-tasks/README.md, "Reused layouts").

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
- `PCB_ROUTER_ISLAND_STUBS`: turns the router's island stubs on (`island_stubs`, off since 2026-10-21): a pad's straight track into a fill island its spokes reach, which only silences KiCad's isolated-island finding.
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

- `PCB_ROUTER_PIN_ROW_COST`: the router's `pin_row_cost` in millimetres where the settings name none (`pin_row_cost_mm`): the cost of another net's step between two adjacent SMD pads of a part's pin row (docs/router.md, "Pin-row cost"; for measuring other values).
- `PCB_ROUTER_NO_STRAIGHTEN`: turns the router's straightening off (`straighten`, on since 2026-10-23): after the via reduction and the thermal spoke repairs, stair-steps and jogs taken out of the signal nets' paths where the clearance allows (docs/router.md, "Straightening"; for attributing rows).
- `PCB_ROUTER_TROUBLE`: overrides the router's `trouble_repair` (`0` off, anything else on): the trouble-makers' repair after the hard repair (an open net routed through others at a price, the nets it displaces routed again hard, kept when the open terminals drop; docs/experiments/human-level-routing.md).
- `PCB_ROUTER_EXACT_ESCAPES`: overrides the router's `exact_escape_stubs` (`0` off, anything else on; off by default): escape stubs stamp exactly (along a lattice line as lattice nodes, oblique ones by their measured footprint) and a pad leaves by straight stubs along a lattice line where it has any; off, the stubs stamp the lattice's discs grown by the snapping error (docs/router.md, "Fine-pitch escapes").
- `PCB_ROUTER_FANOUT`: overrides the router's `fanout` (`0` off, anything else on; off by default): before the negotiation every pad of a dense row gets one straight way out, chosen jointly with its neighbours, that other nets pay `fanout_cost` to cross or to put a via beside (docs/router.md, "Fan-out before negotiation").
- `PCB_ROUTER_CROSSING_DIVE`: overrides the router's `crossing_dive` (`0` off, anything else on; off by default): where the trouble repair's exchange for an open net is refused and the net has a pad of a fine-pitch row whose straight way out a row neighbour's copper closes while their targets cross, one net of the pair takes a via at a fan-out via site beyond the row (docs/router.md, "Crossings decided in the repair (2026-11-03)").
- `PCB_ROUTER_BGA_FANOUT`: overrides the router's `bga_fanout` (`0` off, anything else on; on by default): before routing, the inner balls of every grid array (those no track between the outer balls serves) whose net's own via does not fit a diagonal gap get a stub and a through via in a diagonal gap, the largest of the class's and the project's via sizes that keeps every clearance, as fixed copper of the ball's net that its route may end on; written only where the route uses it; the placer keeps the far side free under those vias (docs/router.md, "Prepared fan-outs for BGAs (2026-11-04)").
- `PCB_ROUTER_FANOUT_COST`: the router's `fanout_cost` in millimetres (default 6; with `fanout`): what another net pays to cross a reserved fan-out run or to put a via within reach of it.
- `PCB_ROUTER_FANOUT_DEPTH`: the router's `fanout_depth` in millimetres (default 0.5; with `fanout`): how far beyond its pad a fan-out run reaches.
- `PCB_ROUTER_FANOUT_FREE`: with `fanout`: a pad of a row coarser than 0.65 mm takes part when it has at most this many straight ways out along the axes (default 2; 0: fine-pitch rows only; an experiment).
- `PCB_ROUTER_FANOUT_VIAS`: with `fanout`: `0` charges no via beside a reserved run, only the crossings (an experiment).
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
- `PCB_ROUTER_LAST_CONNECTIONS`: overrides the router's `last_connections` (a number; `0` off, the default): once the trouble repair's rounds leave at most that many open terminals, a last pass with ejection chains three deep, the victims routed board-wide and pour nets taking part (docs/router.md, "The last connections").
- `PCB_PLACER_PAD_EXITS`: overrides the placer's `pad_exits` (`1` on with the defaults where the config has none, `0` off; off by default): after each seed's placement the router's test of every pad's way out on the statics, the parts whose pads another part or the edge closes moved or turned, the pads left closed in board-placer.json's `pad_exits` and the verdict (docs/placer.md, "Pads without a way out (2026-11-04)").
- `PCB_ROUTER_DIFF_PAIRS`: turns differential pairs on (1) or off (0) where the settings name none (`diff_pairs`; unset, route mode turns them on where the project's custom rules ask for pairs, else off): route mode routes the pairs KiCad names (`_P`/`_N`, `+`/`-`, `P`/`N`) as coupled runs at the rules' width and gap, each link a net of the route negotiated with the others (docs/router.md, "Differential pairs").
- `PCB_ROUTER_DIFF_PAIRS_WORK`: differential pairs: the fallback router's work deadline in seconds, which routes what the coupled runs left open (default 0.05 of the ladder's budget, at least 1 s; for experiments).
- `PCB_ROUTER_DIFF_PAIRS_CROSSOVER`: differential pairs: `1` gives a crossed link end a crossover in its lead (one net dives under the other, a via each side), with the links' hairpins removed and the kept links' via holes kept clear in the fallback (default off: more links coupled, more errors on the rule boards; docs/router.md, "Polarity"); since 2026-10-27 the crossover keeps room beside the neighbouring pairs' launches and a link's vias spread where they crowd the other net, measured again and still off (ducktop2's io boards worse at 30 s; "The safe crossover").
- `PCB_ROUTER_DIFF_PAIRS_SKEW`: differential pairs: `0` turns off the meanders that match a coupled pair's two lengths to the rules' skew (else the pair's gap) or length (the skew is still measured and reported; docs/router.md, "Skew and length").
- `PCB_ROUTER_DIFF_PAIRS_RETURN_VIAS`: differential pairs: `0` turns off the return vias of the reference net (GND) beside a coupled run's vias, `1` turns them on (default on; the settings' `diff_pair_return_vias` wins; docs/router.md, "Return vias").
- `PCB_ROUTER_LENGTH_MATCHING`: turns the length and skew rules' meanders off (0) or on (1) where the settings name none (`length_matching`; unset, on): a net shorter than a custom rule's `length` minimum, or further than a `skew` maximum below the longest net of its rule, gets a meander on a straight track of its route, written as a KiCad tuning pattern (in route mode and on a layout's own routes), and the verdict's `lengths` reports each such net; a net over a `length` maximum is routed again the short way where the nets in its way can go round (`PCB_ROUTER_LENGTH_REPAIR_PRESENT`) (off: none of it; docs/router.md, "Length and skew rules").
- `PCB_ROUTER_LENGTH_REPAIR_PRESENT`: the price of sharing a node for the length repair's search (default 2): a net longer than its custom `length` rule's maximum is routed again through the nets in its way, which then go round (docs/router.md, "Length and skew rules").
- `PCB_ROUTER_BUS_BUNDLES`: sets the router's `bus_bundles` (an experiment, off): buses of four or more nets joining the same footprints route first, in pad order, inside one corridor, side by side (docs/experiments/bus-bundles.md).
- `PCB_ROUTER_BUS_LANE`: the router's `bus_lane_cost`: the factor on a bus member's step off the lane beside its neighbours (default 1.5).
- `PCB_ROUTER_BUS_MARGIN`: bus bundles: tiles a bus's corridor is grown by beyond half its width (default 4).
- `PCB_ROUTER_BUS_NO_CORRIDOR`: bus bundles, an ablation: the members keep their own corridors (the lane and the order stay).
- `PCB_ROUTER_BUS_NO_ORDER`: bus bundles, an ablation: the members keep their place in the usual routing order (the corridor and the lane stay).
- `PCB_ROUTER_BUS_PATIENCE`: bus bundles: the reroutes after which a member leaves its bundle and negotiates as any other net (default 8).
- `PCB_ROUTER_NO_SPOKELESS_TRACKS`: turns off the router's `spokeless_tracks` (a straight track into kept fill for a pad the spoke model sees without a spoke).
- `PCB_ROUTER_NO_APART_TREES`: turns off the router's `apart_trees` (terminals no path reaches from their net's tree joined among themselves).
- `PCB_ROUTER_NO_LOST_SPOKES`: turns off the router's `lost_spoke_repair` (other nets rerouted around a starved pad's cut spokes after the clean-up).
- `PCB_ROUTER_MEASURED_THERMALS`: turns on the router's `measured_thermals` while routing too (the measurement's pour model in the in-route repairs: thermal vias by their own zone connection, custom pads' spokes in KiCad's frame, blocked spokes over the pad and necks the lattice misses in the DRC's track rule).
- `PCB_ROUTER_NO_RECLAIM`: turns off the pour model's reclaimed fill (a pour's fill inside another net's zone of a higher priority where that zone's fill leaves room, `Router::reclaimed_fill`).
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

- `PCB_ROUTER_TROUBLE_WHY`: the trouble-makers' repair prints every exchange's outcome (kept, or why refused; a refusal names the nets left in conflict and what their conflicts name).
- `PCB_ROUTER_BGA_FANOUT_WHY`: the prepared fan-outs (`bga_fanout`) print each grid array's plan once a process: pitch, rows and columns, the rings that leave on the surface, the inner balls, the dog-bones planned by via size, the balls kept (a via in the pad) and those no gap fits.
- `PCB_ROUTER_NET_WORK`: `<file>`: the search expansions per net over the process (every router, every phase), written to the file as JSON at each router's finish; the last write holds the whole run (docs/router.md, "Prepared fan-outs for BGAs (2026-11-04)").
- `PCB_ROUTER_FANOUT_WHY`: the fan-out step (`fanout`) prints every dense pad with its ways out, their costs, the one chosen and the pads left short.
- `PCB_PAD_EXITS_WHY`: the pad-exits test prints, per closed pad, every rim node of its pocket and the item that closes it, and per open pad where its pocket opened.
- `PCB_PAD_EXITS_COUNTS`: pad-exits-kicad-board prints the probe's counts beside the CPU time of the lowering and the probe (for calibrating the test's work).
- `PCB_ROUTER_LENGTH_DEBUG`: the length repair: prints per net over its maximum what the priced search found, the victims and why the exchange was kept or undone.
- `PCB_ROUTER_DIFF_PAIRS_DEBUG`: differential pairs: prints each launch, each link's path, the copper refused with the reason, each stub straightened or not, and the fallback's work.
- `PCB_ROUTER_BUS_WHY`: bus bundles: prints every bus found and every member broken out of its bundle.
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
- `PCB_LAYOUT_DESCENT_CLASSIC`: `<n>`: the move descent's first `n` router trials are also tried and timed as the coupled loop tries a move (a clone, `Router::update`, a reroute without the polish) on a copy; the result does not change.
- `PCB_PLACER_TRACE`: `<file>`: per seed the weighted wirelength after each stage and per part how far each stage moved it.
- `PCB_PLACER_DESIGNER`: `<board>`: the designer's poses judged by the placer's model (their wirelength, what each illegal movable part meets), and with `PCB_PLACER_TRACE` in the trace beside ours.
- `PCB_PLACER_DESIGNER_LEVEL`: `full`: judge the designer's poses at the full spacing; `tight`, `touch`: with tight bodies, and courtyards touching (default: the smallest spacing, no halos).
- `PCB_PLACER_CROSSINGS_PROFILE`: with `PCB_PLACER_DEBUG`: per placer phase a `placer crossings` line, the wall time inside the ratsnest-crossings count against the work it booked and the rest of the phase's (the booking's calibration); `ops` times each of its operations too (docs/placer.md, "The crossings' work booking").
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
  (`until ! pgrep -f <name>`) never ends, `pkill` kills the session, and
  a `for p in $(pgrep -f ...)` kill chain in one command kills its own
  shell. Wait on a PID (`tail --pid=<pid> -f /dev/null`) or a done-file;
  kill by PID, children by `pgrep -P <parent>`.
- Never edit a bash script while an instance of it runs: bash reads the
  file as it executes, and an edit lands in the middle of the run (a
  gate died with a syntax error after its check had passed, 2026-10-28;
  nightly.sh alike). Copy it to a new name, or wait.
- Memory ledger placeholders starve the machine: a reservation is held
  whether it is used or not, so a harness or an `--expected-gb` that
  reserves a round guess far above the measured peak makes every other
  run wait (the ledger's rules under "Working on it").
- A layout's verdict has no reference by default: every KiCad finding
  counts but a footprint's own (`footprint_findings`) and, since
  2026-10-27, the input's own edge clearances, mask bridges and
  courtyards of parts the run kept where the input has them
  (`input_findings`). Clearances, hole clearances and shorts between
  fixed parts still fail it (framework_mobo_lefthalf's R31 and R32 at
  J1: exit 1), while run.py's strict rule subtracts the designer's
  findings at the same place: a verdict's exit 1 and a strict pass can
  both be right. `drc-kicad-board` and `fab-kicad-board` on a board alone
  take every part as the input's (a moved part's mask bridges excused:
  vaio_re); `--input <project-dir>` judges as the layout did.
- The nightly rewrites parts of README.md, docs/handover.md,
  docs/benchmarks.md and docs/changelog.md and commits them on main
  (scripts/nightly-report.py, `--docs`): README's "Boards harvested from
  GitHub" bullet from "pcb-maker places and routes" on and the table's
  rows that name a nightly; the hand-over's title, the
  `nightly-state` line and the paragraph that begins
  `**Layout sweep v` and says "written by the nightly". Edit around them.
- skills/pcb-maker/SKILL.md is tested (tests/cli_exit_codes.rs): every
  word of the form `<verb>-kicad-<noun>` or `inspect-...` in it must be
  an agent command named in docs/agents.md, and every agent command must
  be in it.
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
  past its reservation with no room left waits in the ledger for the
  room and ends with exit 4 (not the OOM killer) only when the memory
  available falls below 2 GB, or after 600 s when the others' growth
  would take it there; run.py retries such a row once (since 2026-10-27).
- KiCad 10: pinfunction is `NAME_NUMBER`; pad and text angles are absolute;
  copper items reference nets by name, and an unknown name becomes a new net
  unless a touching pad supplies one; DRC caps a violation type at 499 (199
  for some); its silkscreen DRC sometimes names the wrong pad; its DRC is
  not byte-deterministic (finding order, which pair names a missing
  connection).
- Freerouting's jar opens its GUI without `-de`/`-do` (`--help` too): always
  `-Djava.awt.headless=true -de ... -do ...`, or use `freerouting.py`.
