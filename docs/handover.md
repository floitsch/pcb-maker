# Hand-over (state as of 2026-11-07; measured by layout sweep v34, the nightly of 2026-11-07 on main 406dcb9)

What the tool does today, which benchmarks count, what is known to be
wrong, where everything is and how to work on it. What each round of work
measured and changed is in [changelog.md](changelog.md), newest first
(the dated "Committed on ...", triage, sweep and measurement sections that
used to be here; code comments and docs that cite `docs/handover.md,
"Committed on ..."` mean those sections). Longer-lived ideas are in
[todo.md](todo.md); the commits themselves in git.

## State on 2026-11-05

- **main 1318b334 (x1911).** The numbers are the nightly of 2026-11-04
  (main 688b59c, x1887), the newest that has finished: it ran before
  nearly all of 2026-11-04's work (the prepared BGA fan-outs, the room
  around grid arrays, the courtless rule, the fourth router round, the
  second memory round, the switch removal). The nightly of 2026-11-05
  runs on x1909 (main 93162bd9: all of that, `skeleton_race` off) and is
  the first to measure it; it had not finished when this was written (it
  runs from 02:00 to about 15:00). By 2026-11-04's: sweep v31 **61 of
  109 strict** (61 per type; 62 on every night from v25 to v30), 2514
  unconnected; the matched designer tier 68 strict (71 per type); over
  the 100 target boards our layouts leave 1297 open connections where
  the designer's placement, routed by the same binary at the layout's
  routing budget, leaves 351: **placement 73 %** (+946; ours worse on 29
  boards, the same on 65, better on 6), routing 27 % (76 % / 24 % the
  night before); the expansion tier 24 of 46 strict (558 unconnected),
  the short tier 7 of 12 strict (22; MIDAS-MK1.1-revA lost its pass, in
  the sweep too), the fingerprints identical.
- **Corridor stubs (branch lattice-corridors, 2026-11-05).** OpenFC's
  U10.5, the one connection behind every lost OpenFC pass, connects
  phase-free: a pad whose escape nodes the lattice pockets leaves by a
  stub off the lattice (`corridor_escapes`, on; open problem 4). With
  it OpenFC strict over ten seeds: plain 10 of 10, the skeleton race 7
  of 10 (pour islands) at the short tier's budget, 10 of 10 at the task
  budget; the tiers unchanged.
- **The race over skeletons on by default (branch skeleton-race-4,
  2026-11-05)**, where a seed's placement work is at least 500 s (the
  task budget's 600: on; the short tier's 200: plain, byte for byte):
  OpenFC's lost pass was the short budget's skip to the tight levels
  (open problem 4; placer.md "The skeleton's budget").
- **Placement since 2026-11-03.** Two measurements say where the gap is.
  Pinning the designer's non-passive parts (a median 28 % of the movable
  parts) recovers a median 0.92 of the designer's first-route advantage
  on seven boards, the connectors alone -0.16, with the three largest
  ICs 0.15, and our objective accepts that arrangement (docs/placer.md,
  "Which placement decisions matter (2026-11-04)"). Stage by stage, the
  global placement loses the coarse arrangement (the active parts a
  shuffle's distance from the designer's, the seeds as far from each
  other; it decides video and SNSP-CPU-01), and on the dense boards the
  anneal loses the fine one while it lowers our objective (the 4in1 and
  Glasgow without it: 142 against 174 and 95 against 149 first-route
  open terminals; "Where the arrangement is lost (2026-11-04)"). The
  race over skeletons (`skeleton_race`) passes the full protocol on
  the dense boards (stage 2 KiCad unconnected -12.1 %, p 0.002; video
  -33 %, protoesp -40 %) and is **on by default since 2026-11-05** where
  a seed's placement work reaches 500 s: OpenFC's lost strict pass was the
  short tier's budget (at the task budget 10 of 10 strict in both arms;
  "The skeleton's budget (2026-11-05)"). Also on by default since: the room
  around dog-boned grid arrays (`bga_room`: Glasgow 343 to 262
  unconnected over three seeds; "Room around grid arrays") and the
  courtless rule (two footprints without a courtyard keep the boxes of
  their pads and outlines apart; its stage-1 screen +1.7 %, null, where
  the off side shipped 292 overlapping pairs; "Footprints without a
  courtyard, and their slots"). Off: `pad_exits` (stage 1 on its four
  boards -3.0 %, p 0.228: a survivor, pending the full protocol),
  `pair_rows`, `length_minima`. Failed stage 1 and removed: the net
  weightings (`rail` +5.6 %, `flat` +4.0 %), rows of repeated parts
  (`align_rows`, +4.2 %).
- **Routing since 2026-11-03.** The prepared fan-outs for BGAs are on
  (`bga_fanout`: Glasgow's U30 on the designer's placement 55 to 15
  open, layouts 372 to 343 over three seeds; no other task board takes a
  plan; docs/router.md, "Prepared fan-outs for BGAs (2026-11-04)"). With
  the router's `diff_pairs` on, every routed stage of a layout plans the
  pairs, not only the final ladder (docs/router.md, "The layout's own
  routes (2026-11-04)"), at a price in connections on dense boards
  (jetson 23 and 44 unconnected where 0 and 9; open problem 15). The
  length reroute is off (`length_reroute`: no length finding went;
  open problem 16). The crossing dive was null on the five boards it was
  built for and was removed with the fan-out step, the last
  connections' ejection chains, the capacitated corridors, the island
  stubs and the final route from the router's state ("Removed
  experiments (2026-11-04)"). Blind, buried and micro vias landed off on
  2026-11-05 (x1911, `blind_vias`: no task board allows a span; route
  mode on the intake's HDI boards fpga_large 360 to 275 KiCad
  unconnected, OpenAIO-Whoop 135 to 117; one layout seed lost one; open
  problem 14).
- **Agent flow since 2026-11-03.** Three fresh-agent trials on
  2026-11-04 (docs/reviews/2026-11-04-fresh-agent-trial-4.md, -5.md,
  -6.md: Goga64's ULK keyboard panel from its netlist, Quanta75, oro's
  link; ducktop2 right_io's 21 pairs, jetson, the SmartSpin2k panel;
  eurorack-pmod from its schematic, the firestarter's DVP length rule,
  the laptop's power on an outline of its own) and their leftovers:
  describe-kicad-board's `outline.cutouts`, a footprint's own slots
  moving with it, the pairs' next action, a row whose pads would short
  refused before the run, the input's observations folded, libraries
  found where a table names another machine's folder
  (`relocated_libraries`), `strip-kicad-tracks --keep-nets`, the rate's
  action first where the first route took the budget, protectors pulled
  to their IC, the missing list folded by footprint, an RF module's
  antenna in the starter. The settings no longer accept the removed
  switches' keys (the structs deny unknown fields): a layout.json or
  router.json that names one is refused.
- **Speed and memory since 2026-11-03.** The same placements and routes,
  cheaper: the fourth placer round 1.4 % less placement CPU, the fourth
  router round 11.9 % less router CPU, the second memory round the
  layout's peaks -48.8 % (geometric mean over the ten largest-peak
  boards, the largest 5160 to 3156 MB; the short tier -27 %; CPU -1.5 %
  in pinned pairs; the nightly's estimates follow once three nights
  carry the binary). The switch removal took 6989 lines of Rust out, the
  default path byte for byte (docs/profiling.md; changelog, 2026-11-04
  and 2026-11-05).
- **The direction, decided on 2026-11-04.** Placement is 73 % of what
  stays open, and the day said which part of it: the active parts'
  arrangement as a whole, lost coarse by the global placement and fine
  by the anneal's objective. So: **one placement bet at a time**, judged
  by stage 1 (the first route, five seed pairs on the eleven dense
  boards; the skeleton race's took 23.7 CPU hours for both arms) and
  then by the full protocol (benchmarks.md, "Measurement discipline:
  placer defaults", "The ladder of placer measurements"). The current
  bet was the skeleton race: BETTER by the full protocol on the dense
  boards, blocked by OpenFC's lost strict pass (a skeleton that seats only
  at a tight level packs a small board tighter than the plain placement); the next question is the anneal's
  judge (the stage-1 screen without the anneal, `PCB_PLACER_SKIP_ANNEAL`,
  is running). **No more single-term levers**: the day's (the net
  weightings, rows of repeated parts, length minima, pair rows) moved
  the term they aimed at and not the routes. A lever that loses or is
  null under its measurement is removed, not kept as an off switch
  ("Removed experiments (2026-11-04)"). A fresh-agent trial every few
  days (trials 4 to 6 on 2026-11-04).
- **In flight**: "Working on it" below.

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
via in each pad no track can leave (`via_in_pad`); a grid array's inner
balls whose class's via does not fit a diagonal gap get a dog-bone before
routing (`bga_fanout`, since 2026-11-04). Every budget is in work
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
**Nightly** 2026-11-07 (main 406dcb94a0f6, `build/nightly/2026-11-07/summary.md`): fingerprints changed: jetson-nano-baseboard, Sisu_SSE-9, computie__k30-SBC, Castor_and_Pollux__mainboard; expansion 24/46 (strict 24), 558 open; short 7/12 (strict 7), 21 open; sweep 63/109 (strict 63), 1957 open; designer 70/109 (strict 67), 2756 open at 0.662; gap placement 70%, routing 30%.
<!-- nightly-state:end -->

What main does now, by the nightly of 2026-11-04 (main 688b59c, x1887;
it ran before the prepared BGA fan-outs, the room around grid arrays,
the courtless rule and the switch removal: the nightly of 2026-11-05, on
x1909, is the first to measure them):

| Stage | Boards | Result | Unconnected |
| --- | ---: | --- | ---: |
| Layout sweep (v31; stacked parts and constraints, 1500 s of work) | 109 | **61 strict** (61 per type), 2 without a result; 62 on every night from v25 (2026-10-29) to v30, 61 on v24, 59 on v23, 57 on v22 | 2514 |
| Matched designer tier (the designer's placement at the layout's routing budget, scale 0.675) | 109 | **68 strict** (71 per type) | 2759 |
| The gap over the 100 target boards (`compare.py --gap`) | 100 | placement **73 %** (+946), routing **27 %** (351) of the layout's 1297 open | - |
| Expansion tier (route mode, 8 x 1) | 46 | **24 strict** | 558 |
| Short tier (12 flip-prone boards, a third of the budget, 3 x 2) | 12 | **7 strict** | 22 |
| Fingerprints (Castor, k30-SBC, jetson, Sisu; route mode, a quarter budget) | 4 | identical night to night | 42 |

The night's losses: MIDAS-MK1.1-revA's strict pass (0 to 2 open) in the
short tier and the sweep, and OpenAirScope's (0 to 1) in the sweep
([benchmarks.md](benchmarks.md#layout-sweep-v31-2026-11-04)). The day's
packages measured their own changes on the short tier, the fingerprints
and place checks (changelog, 2026-11-04 and 2026-11-05).

**Layout sweep v34** (the nightly of 2026-11-07, main 406dcb9, written
by the nightly): **63 of 109 pass the strict rule** (63 the per-type
rule), 1957 unconnected in all, strict errors 26, no row cut or killed,
3 without a result; medians against the designer's board: vias x0.65,
length x0.99, open 0.000 (mean 0.076), bends x1.00, stair-steps +29,
crossings +1. Against the nightly of 2026-11-06: better 11, same 87,
worse 8, strict passes 63 -> 63 (gained USB_Cable_Tester PCB,
zpn_devboard, MokyaLora; lost DSP-ADAU1452, capybully, ghoul). The
matched designer tier on the same binary (budget scale 0.662): 67 strict
passes (70 per type); over the 100 target boards placement 70 %, routing
30 % of the open connections. `build/nightly/2026-11-07/`; table, gap
split, the costliest placements and the losers in
[benchmarks.md](benchmarks.md#layout-sweep-v34-2026-11-07).

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
B=build/bin/pcb-maker-x1911
benchmarks/agent-tasks/run.py build/short-x1911 --tier short --jobs 3 --threads 2 --binary $B > build/short-x1911.log
benchmarks/agent-tasks/run.py build/exp-x1911 --tier expansion --jobs 8 --threads 1 --binary $B > build/exp-x1911.log
benchmarks/agent-tasks/compare.py build/nightly/2026-11-04/short.log build/short-x1911.log
```

The full sweep: `benchmarks/agent-tasks/run.py build/layout-x1911
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
changelog (on 2026-11-04 and 2026-11-05: the prepared BGA fan-outs and
the room around grid arrays, on; the fan-out step, the crossing dive and
the last connections, removed; the designer as teacher, the partial copy,
the skeleton race's stage 1 and the pipeline's stages, measured). **The
rank by the evidence of 2026-11-05**: 4 (placement, 73 % of what stays
open: (a) the coarse arrangement, lost by the global placement; (b) the
fine one, lost by the anneal's objective), 13 (deciding a placer
default), 17 (pad exits, a stage-1 survivor), 12 (the near misses), 2
(crossings at fine-pitch rows), 15 (the pairs' cost in a layout), 16
(length minima without room), 1 (fill fidelity), 3 (the ladder's judge),
5, 14 (blind vias: their chains, and no task board that allows a span),
then the rest. For the router alone, 18 (the router's ceiling, 2026-11-06)
ranks what keeps the designer tier's 41 failing boards open.

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
   pads on fine-pitch rows", k30's U10 pad 8). On the near misses'
   fine-pitch rows (Quanta75's RP2040 and USB-C, OpenAirScope's UDFN)
   every pad has a straight way out on the statics; what closes a pad is
   a neighbour's via just beyond its tip or a neighbour's track turning
   across it, where the two nets' targets cross (one needs a via, which
   in a 0.4 mm row fits only beyond the neighbours' tips). Two repairs
   were built, measured and removed on 2026-11-04 ("Removed experiments
   (2026-11-04)"): the fan-out step before the negotiation (`fanout`: it
   traded which pad stays open, OpenAirScope's CMD for DAT3) and the
   crossing dive in the trouble repair (`crossing_dive`: on Quanta75's
   RP2040 row every dive closed the crossing and opened a third net, its
   via landing on the key matrix's rows on the other layer). The crossing
   census stays (`PCB_ROUTER_TROUBLE_WHY`: 3 of the five boards' 9 open
   items are crossing pairs, all on Quanta75's RP2040 row). Left: the
   via's landing chosen with the other layer's routes (a site priced by
   what it displaces there, or the displaced net's own dive), and
   OpenAirScope's UDFN and the keyboard's aQFN pads, which have no
   straight way out at all. k30's U10 pad 8 is a pour net's
   (`free_thermal_spokes` moves 2 of 10 nets there). Grid arrays are done
   for the case the census found (docs/router.md, "Prepared fan-outs for
   BGAs (2026-11-04)"): where the class's via does not fit a diagonal gap
   (Glasgow's U30) the inner balls get a dog-bone before routing
   (`bga_fanout`, on); what stays on Glasgow is its outer rings' surface
   escapes, ringed by our passives (open problem 4, `bga_room`).
3. **The ladder's judge.** On 21 boards with every rung continued to its
   final (2026-10-13), today's judge (unfinished nets after the probe)
   picks the best rung 16 times; no other count does better, and a class
   of boards (Hub, DSP-ADAU1452, koeg-board, zpn_devboard) probes tracks
   far ahead while a fresh connect attempt ends far better: the 75 s probe
   is too short there. If anything, a 30 s continuation per probed rung is
   the judge to try. On the designer tier's 27 boards with open items of
   ours the kept attempt uses a median 15 % of the ladder's work and 28 %
   goes to rungs that end far worse (router-ceiling.md, design 1). The
   ladder stops at the first complete rung whatever its starved thermals
   (Castor: 20 starved kept against tracks' 12); rank by (open, starved)
   before vias, and let a complete-but-starved rung be rivalled. Since
   2026-11-07 route mode runs the leader's variants first and the rungs
   of other pour modes only where their probe left at most twice the
   leader's unfinished nets (router.md, "The leader first"; designer
   tier 68 -> 71 strict): the probe's misjudgment now also decides which
   rungs run at all (module: connect kept out at 161 against tracks' 33,
   1 -> 3 unconnected), so a better judge matters more. Details
   in [congestion-model.md](congestion-model.md) and the changelog's
   2026-10-13 sections.
4. **Placement: 73 % of what stays open.** The matched designer tier of
   2026-11-04 routes the designer's placement of the 100 target boards to
   351 open connections where our layouts leave 1297: placement +946
   (worse on 29 boards, the same on 65, better on 6), routing 351. The
   costliest: video +201 (202 / 1), Batch_1 +137 (140 / 3),
   SmartSpin2k_Panelized +105 (105 / 0), the OpenESC 4in1 +73 (88 / 15),
   Glasgow +72 (127 / 55), SNSP-CPU-01 +62 (97 / 35). The share was 52 to
   57 % up to the nightly of 2026-10-28 and 73 to 76 % since. What
   carries it (docs/placer.md, "Which placement decisions matter
   (2026-11-04)"): the designer's non-passive parts pinned recover a
   median 0.92 of the designer's first-route advantage on seven boards
   with 28 % of the parts; the connectors alone -0.16, with the three
   largest ICs 0.15, the tenth we misplace most 0.22; with the
   designer's non-passives our wire is lower or within 2 % of our own on
   six of seven boards, so our objective accepts the arrangement and our
   search does not reach it. Where it is lost ("Where the arrangement is
   lost (2026-11-04)"), two losses and no single stage:
   - **(a) The coarse arrangement: the global placement.** From our
     start it leaves the active parts a shuffle's distance from the
     designer's (displacement share 0.77 to 1.14, pairwise correlation
     0.03 to 0.36) and the seeds disagree with each other as much; the
     anneal, the legaliser and the refinement keep what they are given.
     It decides video and SNSP-CPU-01: the global placement started from
     the designer's poses leaves 69 first-route open terminals on video
     against our 304, and 22 on SNSP against the designer's 23. The bet
     on it is the race over skeletons (`skeleton_race`, off; placer.md,
     "A race over skeletons (2026-11-04)" and "The fixes, and stage 1
     again (2026-11-05)"). Fixed on 2026-11-05: the fallback is the plain
     seed at its full work; the skeleton starts at the first level where
     the whole board fits; held and fixed parts are judged at the smaller
     halos; under the switch the global placement stops where its
     overflow stalls and never turns a part longer than the board (video's
     global never met its stop: SIM72 modules turned upright, ten parts
     turning back and forth every 15 iterations; the stall stop alone, as
     a default, NOT BETTER on its three boards, +2.7 %, p 0.62). Stage 1
     against main: -8.9 %, p 0.007, no seed refused (video -28.9 %).
     **Stage 2: BETTER**, KiCad unconnected -12.1 % (p 0.002, the noise
     file's 0.000), every pair sum falls, 765 to 615 summed medians; video
     189 to 106, protoesp 61 to 31, DSP-ADAU1452 complete on four seeds,
     the 4in1 -16 %, Glasgow -12.5 %; SNSP-CPU-01 +34 % (weight 11);
     no dense strict pass lost. **Not a default**: the short tier's OpenFC
     passes strictly on five of five seeds off and one of five on (its
     skeleton seats only at level 7, the placement much tighter).
     **The level guard (2026-11-05, placer.md "The level guard"), not
     taken**: OpenFC's level 7 is the skip to the tight levels
     (`skip_to_tight`) that the skeleton's half of the work triggers at
     the short tier's budget, and protoesp's winning skeletons skip the
     same way (to the fine grid). Bounding the phases by a plain probe's
     level (a `level_guard`, `skips` or `walk`, built and removed) keeps OpenFC's strict pass
     on 7 of 10 seeds against off's 9 of 10, the mixed race on 2 of 5,
     and `skips` gives back protoesp's gain (-2.5 % against -38.9 %).
     Every OpenFC failure, off's seed 801 too, is one connection: U10's
     +3.3V pad 5 (X2SON-6 on B.Cu). **Not the spot under U2**
     (2026-11-05, placer.md "A far-side pad under fine pitch, and what it
     was"): on 3 of the 15 failing rows U10 sits under U2's pins, on 8 it
     is 0.7 to 10.6 mm from U2's field; U10's pads have no legal lattice
     node of their own and pad 5 leaves its pocket only through a 0.3 mm
     corridor between U10's corner pads, which the 0.09 mm class passes
     within about 15 um of its centre, and the lattice chooser does not
     steer to it (`tight_channels` counts enclosed pads only). Moving
     U10 by 0.02 to 0.05 mm onto the router's lattice connects it on four
     of five failing placements (two under U2's pins); the fifth sits
     under two large F.Cu test pads. `pad_exits` never tested the pad
     (+3.3V has a connected pour). The via-room census of the night (106
     boards): pads the other side boxes in are open 2.5 times as often
     (78 of 539) but lie on four boards (the OpenRX panel, the two
     OpenESCs, the PolyKybd molecules); no placement rule was built.
     **Corridor stubs (2026-11-05, router.md "Corridors of pocketed
     pads")**: where a pad's escape nodes lie in a pocket the lattice
     closes, a path off the lattice (5 um raster, exact clearances,
     straightened) leads from the pad through the corridor to a legal
     node beyond (`corridor_escapes`, on). The chooser counting the
     corridors was built and not taken (the pocket meets the corridor
     through a diagonal squeeze the other axis decides). The census of
     the night's 109 placements: 923 escape pads on 37 boards, one pocket
     the lattice closes with a way out (U10.5): one footprint, not a
     class; the tiers and fingerprints route identically, CPU within
     noise. OpenFC's ten seeds with the stubs: plain **10 of 10** strict,
     the race **7 of 10**, U10.5 connected on all 20; the race's three
     losses are pour islands (+5V on In3.Cu, GND on B.Cu, GND F.Cu and
     In1.Cu not joined). The short tier with the race keeps its 7 strict
     passes (unconnected 21 to 24 on three failing boards).
     **On by default since 2026-11-05 with a budget rule** (placer.md
     "The skeleton's budget (2026-11-05)"): OpenFC's loss is the budget's.
     At the task budget (600 s of placement work) OpenFC passes strictly
     on ten of ten seeds in both arms, its skeletons walking to levels 2
     to 4; at the short tier's 200 s the skeleton's 100 s make
     `skip_to_tight` jump to level 7 on a quarter of the seeds and the
     race keeps those. Of 21 probed board budgets the skip fires on two:
     OpenFC at 200 s (the loss) and protoesp at 600 s (its gain), so the
     rule is a budget, not "the skeleton walks": the race runs where a
     seed's placement work is at least `min_work_seconds` 500 (OpenFC
     walks from 474 s), else every seed places plain
     (`skeleton_race.budget_ok` false). Every stage-2 row ran at 600 s, so
     stage 2 stands; the short tier is the plain placement's.
     **The passives' first level (2026-11-06, branch skeleton-race-5,
     placer.md "The passives' first level")**: the first night's losses
     (vaio_re 34, jetson 17) were passives that started at the
     skeleton's full halos, found no room and skipped to the tight
     level. The fix starts them no looser than level 2
     (`PASSIVES_FIRST_LEVEL`) and keeps a skeleton's later level
     (protoesp unchanged). It brings vaio_re to 3 and jetson to 1 (both
     inside the plain spread over three seeds), SNSP-CPU-01 to 61
     (plain 97), and MokyaLora and zpn_devboard back to strict passes.
     Stage 1 against the race as it is: -0.5 %, not worse. Stage 2's
     composite against off: BETTER, -12.7 %, p 0.001, no dense strict
     pass lost against off. SNSP-CPU-01 is now a gainer (-13.5 %), but
     DSP-ADAU1452 loses the race's gain (0 to 7 open, all five seeds,
     and its one strict pass). Also fixed: an outline in pieces no
     longer turns the race off (zpn_devboard: its lowering's piece
     regions counted as user constraints). A variant (branch
     skeleton-race-5-skip) keeps the passives' start and only sends their
     skip to level 2 instead of the tight level. It keeps DSP-ADAU1452
     and its strict pass and stays BETTER (-12.3 %, p 0.002), and jetson
     recovers. But vaio_re only reaches 7 to 10 (plain 1 to 4: the
     level-0 attempt eats a third of the seed's work and half the race's
     seeds), and MokyaLora stays at 1. A second variant, a start level
     by area with halos, never says more than level 0: `fit_halos` keeps
     halos under 0.6 of the free area, and vaio_re's passives lack room
     where it is, not in total. **Decided 2026-11-06: the fix**. DSP-ADAU1452's
     loss is the known residual: its passives fitted at level 0. Taking it
     back needs a test of where the passives' room lies.
     **The passives' level per board (2026-11-07, placer.md "The
     passives' level per board (2026-11-07): no rule found")**: on one
     binary the flips between the 11-06 and 11-07 nightlies that are the
     level's are DSP-ADAU1452 (level 0: 0 open and a strict pass; level 2:
     7) against MokyaLora and jetson (level 2: a pass and 1 open; level 0:
     1 and 17). USB_Cable_Tester and zpn_devboard pass at both levels
     (their 11-06 losses were the router's and the region bug), and
     capybully and Quanta75 pass at neither (level 0 1 and 1 open, level
     2 2 and 3). Three per-board rules were measured and none is taken;
     `PASSIVES_FIRST_LEVEL` stays 2. (1) The placer's objective prefers
     level 2 on every seed of every board where level 0 routes better
     (the full halos double the decoupling relations' term), and the tight
     seat on jetson. (2) "Level 0 where every passive seats there" gets the
     seven right, but SNSP-CPU-01's passives seat at level 0 on all nine
     seeds and route worse there. Its proxy from skel5's rows: stage 1
     +0.4 % (p 0.64), stage 2 +1.5 % (p 0.56): DSP -60 %, SNSP +47.5 %.
     (3) The passives' twin (branch passives-twin, not for merge): the
     race probes the first-ranked placement's passives placed again at
     its skeleton's level. Stage 1 on the five dense boards it can change:
     +0.1 %, p 0.94, kept on 2 of 25 rows, +6.3 % CPU; on the seven boards
     DSP-ADAU1452 7 to 4, nothing else moves. Open: a judge that tells
     DSP-ADAU1452 from SNSP-CPU-01. Whole first routes of both levels
     would, on skel5's rows, at the cost of a second first route.
     Measured and not taken: `join_pins` 0 (fixes
     SNSP-CPU-01 and split72_left, loses the 4in1-mini: +3.0 % against
     the race).
   - **(b) The fine arrangement: the anneal's objective.** On the 4in1
     and Glasgow the anneal lowers our objective while it costs routes:
     without it our pipeline leaves 142 against 174 and 95 against 149
     first-route open terminals, at objectives 1.70 and 1.31 of ours;
     from the designer's poses the anneal takes the 4in1 from 61 to 149.
     The decoupling relations are 88 % of the 4in1's objective. Our
     legality costs 28 and 34 terminals there (the designer's poses
     seated at our levels), and Glasgow's refinement 40 more (its
     relation repair moves 153 parts more than 0.5 mm: a suspect, not
     established). **Stage 1 without the anneal fails** (2026-11-05,
     "The anneal skipped: stage 1"): weighted +1.4 %, p 0.653, seven of
     eleven boards worse (MegaDrive +29 %, split72_left +29 %, video
     +20 %); seed 1 was the only pair to improve, Glasgow's gain did not
     hold (-1.5 %). It holds on the 4in1 (-16.4 %) and DSP-ADAU1452
     (-29.7 %), where the anneal packs passives against the ICs under
     the decoupling relations (weight 50). Not a rebuild of the
     objective; the narrow lever left is the relations' weight
     (`constraint_weight`, stage 1 at 10, nothing to build).

   Judged by the protocol and on: crossings in the anneal (-10.8 %,
   2026-10-28), the race's ripped count (-5.6 % at nine pairs,
   2026-11-03). On by their own measurement: the room around dog-boned
   grid arrays (`bga_room`; Glasgow 343 to 262 unconnected over seeds 1,
   101 and 301, its first route better on all five protocol seeds; MIDAS's
   U106 and vd-rd's U200 left out; "Room around grid arrays
   (2026-11-04)"), the far side under a BGA's vias in pads
   (`via_in_pad_far_side`), length maxima (`length_maxima`), panels of
   copies kept off their frames and tabs (2026-11-03: the 0xCB pcb-panel
   62 to 0, a strict pass in v31), the courtless rule (stage 1 +1.7 %,
   null; it ends 292 shipped overlaps). Off and open: pad exits (open
   problem 17), pair rows (15), length minima (16), `decoupling_near`.
   Failed and removed (the list and the numbers in "Removed experiments
   (2026-11-04)"): the net weightings, rows of repeated parts,
   `legality: "kicad"`, `routing_demand`, `routing_room`, `shared_halos`,
   `anneal_far_side_gaps`, `ratsnest_crossings.global`; bus-group moves
   (the objective falls, routing does not); removed on 2026-10-16:
   `replicate_channels`, `supply_via_room`, `bus_rows`. More seeds do not
   close it (no seed holds the arrangement for the race to pick).
   Untested: seeded orientation starts in the race.
5. **Starved thermals as the only failure** on complete boards (rosco,
   Castor, mackerel-30, k30, mackerel-08, Hub, PixelWave, BAGEL; counts
   fell to 0-5 since the spoke model). Since 2026-10-21
   `solid_starved_thermals` is on by default (a starved pad, or one whose
   spokes reach only an island, gets a solid zone connection, kept when
   KiCad finds the board better): mackerel-30 and Castor pass; k30-SBC's
   solid pads cost an unconnected neighbour and are undone (docs/changelog.md,
   "net ties, same-number pads, starved thermals"). Left: pads where solid
   does not help, and the step's one round: on the designer tier a pad
   starved only after it fails k30-SBC and ghoul alone (router-ceiling.md,
   "Plain bugs").
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
     boards): the import said "no library X". Since 2026-11-04 (trial 6) a
     table entry naming another machine's folder is found by its last
     folders under the schematic's folder or its three parents and
     reported (`relocated_libraries`; eurorack-pmod 132 footprints to
     141), and a library found nowhere is named with the table's folder.
     Left: a `missing` line naming candidates further up (a folder
     `<library>.pretty`, or one holding the footprint's file, under the
     project's ancestors up to the repository root); the 22 boards are not
     measured again. Small.
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
12. **The near misses.** Of the 48 boards the sweep v31 fails, 18 are
    within five connections and five errors of clean (README.md's
    headline; the list changes with every night); v31 lost
    MIDAS-MK1.1-revA (0 to 2 open) and OpenAirScope (0 to 1). The last
    diagnosis, 2026-10-27 (12 boards, 34 open items; docs/router.md, "The
    last connections"): 10 the input's (the designer's board has them),
    5 a rule, 10 placement, 1 the lattice at the class's width, 8
    routing; no routing item was a budget that ran out (the trouble
    repair refused every exchange as a pure trade). Closed since: the
    panel tabs and frames (ULK 4 to 0, ULK_sl_PG1316s 5 to 0, jiran 1 to
    0, all strict passes; woagboard 4 to 1, its designer's own GND
    island), vias in BGA pads (MIDAS-MK1.1 5 to 0, a pass), two faults of
    the trouble repair. The ejection chains (`last_connections`) closed
    none and were removed on 2026-11-04. Left: the fine-pitch rows
    (problem 2), the keyboard's RESET (open on the designer's placement
    too), MIDAS-MK1's B2 (its via refused: our placement put an 0603
    under it on the back). The next diagnosis takes the night's list, not
    2026-10-27's.
13. **Deciding a placer default costs a day.** The placer-default
    protocol (benchmarks.md, "Measurement discipline: placer defaults")
    is 110 full layouts a lever, about 88 CPU hours and 18 hours of wall
    beside the nightly (the 2026-10-28 calibration); five seed pairs
    resolve a weighted change of about 8.5 %, nine about 6.5 %. The
    ladder (benchmarks.md, "The ladder of placer measurements"): stage 0
    the placement's own metrics (a sanity check, not a filter), stage 1
    the first route alone (`stop_after_first_route`; 0.55 to 0.57 of the
    full stage's cost in the calibration, 0.67 of the CPU beside full
    layouts of one binary; the full verdict on both calibrated
    experiments), stage 2 the full protocol. Since 2026-11-04 every
    placement lever takes stage 1 first (`experiments/placement-gap/screen.py`):
    the net weightings, rows of repeated parts, the courtless rule, pad
    exits and the skeleton race, whose stage 1 took 23.7 CPU hours for
    both arms on a shared machine (19:39 to 00:40). Rejected: the quarter
    budget (2026-11-03: two boards find no legal placement, the rows do
    not rank the full rows), condensed boards, coarse whole first routes
    as the race's probe. run.py `--reuse` saves the rows a lever leaves
    alone when both sides run on one binary with the lever as an option.
    Open: a stage that keeps the placement's budget and cuts only the
    routing (a layout option not built; placer.md, "The quarter budget
    (2026-11-03)").
14. **Blind, buried and micro vias** (opened 2026-11-05; `blind_vias`, off; docs/router.md,
    "Blind, buried and micro vias (2026-11-05)"). The router routes a
    via as the shortest span the board allows and KiCad accepts them,
    but no task board allows a span (every via of the 109 designers'
    boards is a through via, Glasgow's too), so the
    measure is on the expansion intake's HDI boards: route mode gains
    where they are hard (fpga_large 360 to 275 KiCad unconnected,
    OpenAIO-Whoop 135 to 117), one layout seed loses one (kvm-carrier_r9
    27 to 28). Open: stacked and staggered chains at one cell, spans for
    pour nets (a 1+6+1 board disallows through vias), the micro via's own
    size in the maps, the `.kicad_dru`'s via-type rules, the CPU of the
    via step.
15. **The differential pairs' cost in a layout** (opened 2026-11-05).
    Since 2026-11-05 a layout with the router's `diff_pairs` on plans the
    pairs in every routed stage (the first route, the race's probes, the
    moves; docs/router.md, "The layout's own routes (2026-11-04)"), so
    the pairs are coupled whichever stage keeps the board; and the
    planning costs connections where the board is dense: jetson 23 and
    44 unconnected at seeds 1 and 101 where it had 0 and 9 (5 to 8 pairs
    coupled where none were), route mode's price of the pairs, now in
    every stage. The pair rows (`pair_rows`, off: a pair's two series
    parts as a row of two at the pair's pitch, facing its ends) couple
    more (right_io 1 to 4 and 0 to 6 pairs) but, made to follow
    `diff_pairs`, raised unconnected beyond the seeds' spread on four of
    six runs (jetson +33, +20; docs/placer.md, "Following diff_pairs").
    `diff_pairs` stays off in a layout by default (route mode turns it on
    where the project's rules ask for pairs). In flight: a link that
    costs connections given up in the negotiation ("Working on it").
16. **Length minima without room** (opened 2026-11-04 by trial 6). A
    `length` rule's minimum is met by meanders where the route has room
    beside it (a bump adds 2.2 mm a millimetre of straight track); inside
    a dense bus it is not: the firestarter's DVP bus (25 to 27 mm on ten
    nets) has 5 and 4 of 10 nets within the rule in our layouts at seeds
    1 and 101, and route mode on the designer's placement 3. Two levers
    were built and left off (docs/placer.md, "Length minima
    (2026-11-04)"; docs/router.md, "Length minima: the placer's share and
    the reroute (2026-11-04)"): the placer's floor (`length_minima`: a
    two-part net's pins at least half the minimum apart; the lengths
    gained, `length_out_of_range` 17 to 9 summed over three layouts, oro's
    link's connections too, but the firestarter lost 22 connections over
    two seeds) and the length reroute (`length_reroute`: the bus's inner
    nets find no route with room; with the floor on it met one net more).
    Open: a floor that keeps the connections, and room for the meanders
    inside a bus on a fixed placement (the bus spread as a whole, or the
    inner layer the designer used and our pours keep). An agent keeps a
    designer's bus with `strip-kicad-tracks --keep-nets` (docs/agents.md).
    Of the task boards only oro's link carries a minimum (the sweep v31: 4
    `length_out_of_range`).
17. **Pad exits, a stage-1 survivor** (opened 2026-11-04). `pad_exits`
    (placer, off; docs/placer.md, "Pads without a way out (2026-11-04)")
    tests every pad's way out on the statics after each seed and moves or
    turns the parts that close one. Its first-route stage on the four
    boards with closed pads (the 4in1, the 4in1-mini, split72_left,
    Glasgow; five seed pairs): -3.0 % weighted, p 0.228, which passes
    stage 1 (the tool's verdict EXTEND); the eleven dense boards' full
    layouts at seed 1 964 to 958 unconnected, eight rows the same. But the
    routes do not connect most of the pads it opens (the 4in1: five of
    seven stay unconnected), and the race may keep another seed's
    placement with pads still closed (it does not rank by them). Left: the
    full protocol, after the current bet (one placement bet at a time).
18. **The router's ceiling** (opened 2026-11-06;
    [router-ceiling.md](router-ceiling.md)). The matched designer tier's
    41 failures, per open item and finding: 10 are the input's (the
    designer's own board leaves the same item), 5 the budget's (they
    pass at two or three times the tier's work); of ours congestion
    decides 13 (the stalled residue that single-net exchanges only
    trade), pad access 5, fine-pitch rows 4 (and the second class of 11
    more), and four plain bugs: the solid step runs once (k30-SBC and
    ghoul pass with one and three more rounds, checked on the night's
    boards), copper texts lowered from their render cache (both
    Quanta75), large enclosed SMD pads without a via (Sisu), a custom
    pad's anchor inside a neighbour's clearance (Hub). Designs: the
    ladder's work to the leader with the residue renegotiated in a
    window, and prepared fan-outs for fine-pitch rows (the BGA method).
    Problems 1, 2, 3, 5 and 6 above are the parts of it that were open
    before. Design 1 measured on 2026-11-07: the leader first on by
    default (+3 strict: video, BAGEL-MK1.1-Avocado, DSP-ADAU1452, none
    lost), the residue in a window in but off (`residue_window`: +4 -1
    alone, +1 -1 on top of the leader first; its windows stall at the
    price cap where the board-wide negotiation did). Design 2 (prepared
    fan-outs for fine-pitch rows) is next.

## Where things are

| What | Where |
| --- | --- |
| Current state, open problems | this file |
| What each round measured and changed | [changelog.md](changelog.md), newest first |
| Design notes | [router.md](router.md), [placer.md](placer.md), [constraints.md](constraints.md), [agents.md](agents.md), [congestion-model.md](congestion-model.md), [coupling.md](coupling.md), [global-route.md](global-route.md) |
| Results tables | [benchmarks.md](benchmarks.md), [benchmarks/agent-tasks/README.md](../benchmarks/agent-tasks/README.md) (tiers), [benchmarks/github/README.md](../benchmarks/github/README.md) (harvest, Freerouting) |
| Sweep logs | the nightly's `build/nightly/<date>/` (`sweep/`, `designer/`, `short/`, `expansion/`, `fingerprints/`, `summary.md`, `gap.txt`); the hand-run sweeps `build/github-layout-v16*` to `-v21*`; `build/github-freerouting-x738*.log` |
| Tier references | the newest nightly's `short.log`, `expansion.log` and `fingerprints.log`; not in the nightly: `build/quick30-x1152.log`, `build/sch-x1161.log`; the placer protocol's noise file `benchmarks/agent-tasks/dense-noise.json` |
| Snapshot binaries | `build/bin/pcb-maker-xNNNN` (four digits: main's, made by the gate; x1911 = main 1318b334), a package's own as `x<five digits>` from its range or by its name (`pcb-maker-fan1-a`); `scripts/snapshot.sh <n>` builds and copies one, never over an existing number; the nightly's `pcb-maker-nightly-<date>-<sha>` |
| Release | the rolling `snapshot` pre-release on GitHub, remade by `.github/workflows/release.yml` on every push to `main` that passes `scripts/check.sh --fast` (linux-x86_64, macos-aarch64 when its build succeeds, `VERSION`, `SHA256SUMS`); `scripts/release.sh` does it by hand (`--dry-run` packages into `dist/` only); `pcb-maker --version` and the verdict's `environment.pcb_maker_version` name the commit and build date |
| Tasks | committed in `benchmarks/agent-tasks/tasks/v7/` (every tier since 2026-10-21) and `tasks/v6/` (the quick and short tiers before; v7 is v6 plus uuids on six panels, none in those tiers), copies of `build/github-tasks-v6/` and `-v7/`, from `benchmarks/agent-tasks/from_pcbench.py --corpus benchmarks/github/boards.json`; tier lists in `benchmarks/agent-tasks/{quick,short,schematic}.json` |
| Harvested boards | `benchmarks/real/external/github/` (not in git; `benchmarks/github/manifest.json` has origin and licence); their schematics in `benchmarks/real/external/github-sch/` (`benchmarks/github/schematics.json`) |
| Code | `crates/pcb-router`, `pcb-placer`, `pcb-kicad`, `pcb-congestion`, `pcb-global` (the coarse global route), `pcb-topo`; the command line in `src/` (agent commands in `agent_cli.rs`, the rest in `main.rs`); `pcb-kicad/src/lib.rs` lists its modules (the file layer `kicad_sexpr`, `kicad_nets`, `copper_emit`; `geometry`, `copper_geometry`; `board_placer/`, `board_router/`, `board_layout`; `verification`, `native_report`, `drc_cache`, `verdict`; [README](../README.md#repository)) |
| The removed research stack | git history up to be4b012: the crates layout-trace-routing, pcb-routing, pcb-coordinator, pcb-placement, pcb-validate, pcb-viewer, pcb-benchmark and pcb-pipeline, 54 research commands (check, validate, place, route-grid, the semantic ladder, the route candidates, benchmark-route-*, ...), their tests and fixtures; list in [changelog.md](changelog.md#committed-on-2026-10-20-quality-research-stack-removed); pcb-kicad's research modules (the semantic ladder, the sequential and adaptive routers, the grid router and its inspections, via actions, route relaxation), the crates pcb-engine, pcb-grid-router, layout-trace-model and pcb-core and nine more commands in git history up to b306347 ([changelog.md](changelog.md#committed-on-2026-10-21-quality-pcb-kicad-research-removed-librs-split)) |
| Worktrees | one per package of work, `../pcb-maker-<name>`, made by `scripts/worktree.sh` |
| Agent memory | `~/.claude/projects/-home-flo-programming-pcb-maker/memory/` |
| Untracked in the main checkout | `pcb_pr_agent_brief.md`, `target-bisect/` (not from these sessions), `benchmarks/github/boards-guided.json`, `experiments/congestion/{audit_labels,reconstruct_finals,test_audit_labels}.py`: Florian decides |

## Working on it

**In flight on 2026-11-05** (two packages, each in its own worktree;
what each decides):

- **Pair links given up** (branch pair-links-give-up,
  `../pcb-maker-pairs4`; open problem 15). A pair's link that costs
  connections is given up in the negotiation, the pairs are checked at
  the first route, and the released links are tried again after the hard
  repair. Decides whether a layout with `diff_pairs` keeps its
  connections (jetson 23 and 44 unconnected where 0 and 9), the
  condition for coupling pairs in a layout by default.
- **This refresh** (branch handover-2026-11-05, `../pcb-maker-hand1`):
  the state, the open problems' rank, the switches and README.md's
  claims against the nightly of 2026-11-04 and the code after the switch
  removal. Decides nothing; the nightly of 2026-11-05 (x1909) is the
  first to measure the day's defaults, and the next refresh takes its
  numbers.

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
its doc comment; checked against the settings structs after the switch
removal of 2026-11-04, whose keys the settings no longer accept: "Removed
experiments (2026-11-04)" below):

- `ratsnest_crossings.anneal` (placer): **on** since 2026-10-28 (x15201;
  the placer-default protocol: -10.8 %, p 0.002).
- `race_count` (layout): **`"ripped"`** since 2026-11-03 (x1881; nine
  pairs: -5.6 %, p 0.014): the race ranks its probes by the open
  terminals after the rip-up of the conflicted branches, with a 2 %
  margin over the first-ranked placement; `"unfinished"` is the race
  before.
- `blind_vias` (router): **off** (2026-11-05; `PCB_ROUTER_BLIND_VIAS`;
  docs/router.md, "Blind, buried and micro vias"): where the board allows
  spans (`via_spans`, the designer's own blind/buried/micro vias, a KiCad
  6/7 project's switches), a via pierces only the shortest allowed span;
  route mode gains on the HDI boards, a layout seed lost one connection.
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
- `skeleton_race` (placer): **on** since 2026-11-05 where a seed's
  placement work is at least `min_work_seconds` (500 s; docs/placer.md,
  "A race over skeletons (2026-11-04)", "The fixes, and stage 1 again
  (2026-11-05)" and "The skeleton's budget (2026-11-05)"; defaults `room`
  1, `work_share` 0.5, `join_pins` 8, `plain_share` 0; `null` turns it
  off): each seed places every part but the free passives first, then
  the passives around the held skeleton; under it the global placement's
  stall stop and turns that fit the board. Stage 2 BETTER (-12.1 %, p
  0.002, no dense strict pass lost); OpenFC at the task budget 10 of 10
  strict in both arms. Below 500 s (the short tier's 200 s) every seed
  places plain, byte for byte the switch off's (`board-placer.json`'s
  `skeleton_race.budget_ok`). A level guard (placer.md "The level guard
  (2026-11-05)") was built, measured and removed. On branch
  skeleton-race-5 (2026-11-06, merged): the passives start no looser
  than level 2 (`PASSIVES_FIRST_LEVEL`; a per-board level measured on
  2026-11-07 and not found, placer.md "The passives' level per board"), and the regions the lowering
  gives an outline's pieces (`Constraints::automatic_regions`) do not
  keep passives in the skeleton (placer.md "The passives' first level";
  the known residual: DSP-ADAU1452's level-0 seat and its gain).
- `pad_exits` (placer): **off** (2026-11-04; `PCB_PLACER_PAD_EXITS`;
  docs/placer.md, "Pads without a way out (2026-11-04)"): after each
  seed the router's test of every pad's way out, and the parts that close
  one moved or turned. Stage 1 on its four boards -3.0 %, p 0.228 (open
  problem 17); the full protocol decides.
- `pair_rows` (placer): **off** unless set (2026-11-04, and on 2026-11-05
  not made to follow `diff_pairs`; docs/placer.md, "Pair rows
  (2026-11-04)", "Following diff_pairs"): a pair's two series parts as a
  row of two at the pair's pitch, facing its ends; more pairs coupled,
  unconnected beyond the seeds' spread on four of six runs (open problem
  15).
- `length_minima` (placer): **off** (2026-11-04; docs/placer.md, "Length
  minima (2026-11-04)"): a two-part net's pins at least half its rule's
  minimum apart; the lengths gained, the firestarter lost 22 connections
  over two seeds (open problem 16).
- `length_reroute` (router): **off** (2026-11-04;
  `PCB_ROUTER_LENGTH_REROUTE`; docs/router.md, "Length minima: the
  placer's share and the reroute (2026-11-04)"): a net still short of its
  minimum after the meanders routed again with room for them; no length
  finding went (open problem 16).
- `diff_pairs` (router): unset, **off in a layout**, on in route mode
  where the project's custom rules ask for pairs; since 2026-11-05 a
  layout with it on plans the pairs in every routed stage
  (docs/router.md, "The layout's own routes (2026-11-04)"), at a price in
  connections on dense boards (open problem 15).
- `via_in_pad_far_side` and `length_maxima` (placer): **on** (landed
  2026-11-03; docs/placer.md, "Length maxima and the far side of vias in
  pads (2026-10-28)"): the far side kept free under a BGA's vias in pads
  (MIDAS-MK1 and MK1.1: unconnected 5 -> 1 and 6 -> 3 over three seeds),
  a net's pins kept within 0.8 of its `length` maximum (oro's link:
  `length_out_of_range` 12 -> 5 and 6 -> 4).
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
- `panel_copies` (placer): **on**; since 2026-11-03 the copies keep off
  their frames and tabs as the sub-boards do, connectors and the
  designer's overhangs reaching over them (the 0xCB pcb-panel 62 -> 0, a
  strict pass in v31; docs/placer.md, "Frames on panels of copies, and
  what may reach over them (2026-10-28)").
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
- `PCB_ROUTER_NO_BEND_ROUND`: turns the router's bend round off (`bend_round`, on since 2026-11-06): after the via reduction, each complete signal net with a jog or a staircase rerouted alone at a raised bend cost and kept only if it stays complete with no via more, at most 1 % longer and fewer jogs and staircases (docs/router.md, "The bend round (2026-11-06)"; a diagnostic, for A/B).
- `PCB_ROUTER_BEND_ROUND_COST`: the router's `bend_round_cost`: the bend cost of the bend round's searches, in mm per 45 degrees (default 1.0; a diagnostic).
- `PCB_ROUTER_NO_STRAIGHTEN`: turns the router's straightening off (`straighten`, on since 2026-10-23): after the via reduction and the thermal spoke repairs, stair-steps and jogs taken out of the signal nets' paths where the clearance allows (docs/router.md, "Straightening"; for attributing rows).
- `PCB_ROUTER_TROUBLE`: overrides the router's `trouble_repair` (`0` off, anything else on): the trouble-makers' repair after the hard repair (an open net routed through others at a price, the nets it displaces routed again hard, kept when the open terminals drop; docs/experiments/human-level-routing.md).
- `PCB_ROUTER_EXACT_ESCAPES`: overrides the router's `exact_escape_stubs` (`0` off, anything else on; on by default since 2026-10-26): escape stubs stamp exactly (along a lattice line as lattice nodes, oblique ones by their measured footprint) and a pad leaves by straight stubs along a lattice line where it has any; off, the stubs stamp the lattice's discs grown by the snapping error (docs/router.md, "Fine-pitch escapes").
- `PCB_ROUTER_CORRIDOR_ESCAPES`: overrides the router's `corridor_escapes` (`0` off, anything else on; on by default since 2026-11-05): a pad without a legal node of its own whose escape nodes lie in a pocket the lattice closes leaves by a corridor stub off the lattice, a path through the region its track keeps its clearances in to a legal node beyond the pocket (OpenFC's U10.5; docs/router.md, "Corridors of pocketed pads (2026-11-05)").
- `PCB_ROUTER_ROW_FANOUT`: overrides the router's `row_fanout` (`0` off, anything else on; off by default): before routing, every signal pad of a row whose adjacent pads lie closer than 0.65 mm gets its exit at exact geometry, jointly with its neighbours (straight out beyond the row's tips, fanned where the pitch is narrower than two tracks, a clearance and the lattice's room), and the pads whose targets cross the row's order a via beyond the row whose landing on another layer is checked first; fixed copper of the pad's net that its route may end on, written only where the route uses it; a row the plan cannot serve whole is left to the search (docs/router.md, "Prepared fan-outs for fine-pitch rows (2026-11-07)").
- `PCB_ROUTER_ROW_FANOUT_DIVE`: with `row_fanout` on: `0` plans the rows on the surface alone (no diver's via; an experiment's switch).
- `PCB_ROUTER_BGA_FANOUT`: overrides the router's `bga_fanout` (`0` off, anything else on; on by default): before routing, the inner balls of every grid array (those no track between the outer balls serves) whose net's own via does not fit a diagonal gap get a stub and a through via in a diagonal gap, the largest of the class's and the project's via sizes that keeps every clearance, as fixed copper of the ball's net that its route may end on; written only where the route uses it; the placer keeps the far side free under those vias (docs/router.md, "Prepared fan-outs for BGAs (2026-11-04)").
- `PCB_ROUTER_BLIND_VIAS`: overrides the router's `blind_vias` (`0` off, anything else on; off by default): where the board allows via spans short of the stack (`via_spans`, the designer's blind, buried and micro vias, a KiCad 6 or 7 project's switches), a via takes the shortest allowed span holding the two layers it joins, copper and a hole only there, written as KiCad's blind, buried or micro via; a BGA's dog-bone takes the shortest span to a routable layer (docs/router.md, "Blind, buried and micro vias (2026-11-05)").
- `PCB_LAYOUT_MOVE_DESCENT`: overrides the layout's `move_descent` (`0` off, anything else on with layout.json's settings or the defaults): the move descent after the first route (docs/experiments/move-descent.md).
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
- `PCB_PLACER_PAD_EXITS`: overrides the placer's `pad_exits` (`1` on with the defaults where the config has none, `0` off; off by default): after each seed's placement the router's test of every pad's way out on the statics, the parts whose pads another part or the edge closes moved or turned, the pads left closed in board-placer.json's `pad_exits` and the verdict (docs/placer.md, "Pads without a way out (2026-11-04)").
- `PCB_ROUTER_DIFF_PAIRS`: turns differential pairs on (1) or off (0) where the settings name none (`diff_pairs`; unset, route mode turns them on where the project's custom rules ask for pairs, else off): route mode routes the pairs KiCad names (`_P`/`_N`, `+`/`-`, `P`/`N`) as coupled runs at the rules' width and gap, each link a net of the route negotiated with the others (docs/router.md, "Differential pairs").
- `PCB_ROUTER_DIFF_PAIRS_WORK`: differential pairs: the fallback router's work deadline in seconds, which routes what the coupled runs left open (default 0.05 of the ladder's budget, at least 1 s; for experiments).
- `PCB_ROUTER_DIFF_PAIRS_GIVE_UP`: differential pairs: `1` gives a link without a path, or whose conflicts stop falling, up in the negotiation: its pair's nets route uncoupled from then on, in the same attempt, and the hard repair tries it once more; where the settings name no `diff_pair_give_up` (default off: jetson-nano-baseboard's unconnected 23 and 44 down to 10 and 16, but ducktop2's left_io lost 3 of 6 coupled pairs on one seed; docs/router.md, "Giving up on a link").
- `PCB_ROUTER_GIVE_UP`: the give-up's rounds as `min,stall,pathless,max[,present]` (default `8,4,1,16,100`): rounds in conflict count once the price of sharing has reached `present`; no link goes before `min` such rounds, then once its conflicts have not fallen below their lowest for `stall` rounds, and after `max` rounds whatever; a link without any path goes after `pathless` rounds (for experiments; docs/router.md, "Giving up on a link").
- `PCB_ROUTER_DIFF_PAIRS_SKEW`: differential pairs: `0` turns off the meanders that match a coupled pair's two lengths to the rules' skew (else the pair's gap) or length (the skew is still measured and reported; docs/router.md, "Skew and length").
- `PCB_ROUTER_DIFF_PAIRS_RETURN_VIAS`: differential pairs: `0` turns off the return vias of the reference net (GND) beside a coupled run's vias, `1` turns them on (default on; the settings' `diff_pair_return_vias` wins; docs/router.md, "Return vias").
- `PCB_ROUTER_LENGTH_MATCHING`: turns the length and skew rules' meanders off (0) or on (1) where the settings name none (`length_matching`; unset, on): a net shorter than a custom rule's `length` minimum, or further than a `skew` maximum below the longest net of its rule, gets a meander on a straight track of its route, written as a KiCad tuning pattern (in route mode and on a layout's own routes), and the verdict's `lengths` reports each such net; a net over a `length` maximum is routed again the short way where the nets in its way can go round (`PCB_ROUTER_LENGTH_REPAIR_PRESENT`) (off: none of it; docs/router.md, "Length and skew rules").
- `PCB_ROUTER_LENGTH_REROUTE`: turns the length reroute on (1) or off (0) where the settings name none (`length_reroute`; unset, off): a net still short of its `length` minimum (or its `skew` group's) after the meanders is routed again alone, the other nets' tracks kept and kept off by half a meander's height more than the clearance beyond 1.5 mm of its pads, meandered again and kept where it ends nearer its window (docs/router.md, "Length minima: the placer's share and the reroute (2026-11-04)").
- `PCB_ROUTER_LENGTH_RULE_LAYERS`: where the settings name none, which side of a layer-limited length or skew rule's layers its nets prefer (`length_rule_layers`; unset, `designer`): `designer` the side the designer's board (`--reference`, else the input's own tracks) runs each on, `rule` the rule's layers for all of them, `off` none (docs/router.md, "Skew groups' layers (2026-11-06)").
- `PCB_ROUTER_LAYER_PREFERENCE_COST`: the router's `layer_preference_cost` where the settings name none: the factor on a step off a net's preferred layers (`length_rule_layers`; default 10, at least 1).
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
- `PCB_ROUTER_RESIDUE`: `0` or `1`: the residue renegotiated in windows after the trouble repair (router `residue_window`; wins over the router setting `residue_window`; default off).
- `PCB_LADDER_LEADER_FIRST`: `0` or `1`: route mode's ladder runs the leading rung's seed round and finer pitches before the rungs of other pour modes, and those only where their probe left at most twice the leader's unfinished nets (wins over the router setting `leader_first`; default on; `0` the order before 2026-11-07).
- `PCB_LADDER_NO_STEP_COST_GATE`: route mode's steps start with less left under the bound than a fifth of the ladder's first attempt's work (the rule before 2026-10-21, probe overhead; for attributing rows).
- `PCB_LADDER_SEARCH_LIMITS`: route mode's first attempt stops on its search alone, its pour analyses and lattice on top (the rule before 2026-10-21, probe overhead; for attributing rows).
- `PCB_LADDER_PROBES_COUNT_ALL`: route mode's rung probes stop on all their work (search, pour analyses, lattice and their part of the static maps) instead of on their search (an experiment: leaders changed; docs/changelog.md, probe overhead).
- `PCB_LADDER_CONTINUATION_LATTICE`: route mode's probed rung's continuation takes its share in search work by the routers' proportion of search with their lattices counted, as the fresh routers do (the rule before 2026-10-21, probe overhead; for attributing rows).
- `PCB_ROUTER_NO_SHARED_STATICS`: every router builds its own static maps and pour masks instead of sharing those of routers of the same board, lattice and classes (for attributing rows; the routing is the same).
- `PCB_ROUTER_LIMITS_BETWEEN_ITERATIONS`: a negotiation's work limits (its own and the work deadline) bind only after a whole iteration, not between its batches too (the rule before 2026-10-22, probe overshoot; for attributing rows).
- `PCB_PROBES_SIDE_BY_SIDE`: `N`: at most N probes of the probe ladder at once (1: one after the other).
- `PCB_PLACER_START`: `<board>`: a diagnostic start: the placer's stages from `PCB_PLACER_START_STAGE` on start from this board's poses, by reference (docs/placer.md, "Where the arrangement is lost (2026-11-04)").
- `PCB_PLACER_START_STAGE`: with `PCB_PLACER_START`: `global` (default: the global placement starts from the poses), `anneal` (no global placement), `legalize` (no global placement and no anneal: the legaliser seats the poses, the refinement follows).
- `PCB_PLACER_SKIP_ANNEAL`: a diagnostic: no anneal at any level; each level's legaliser seats the global placement's poses (or `PCB_PLACER_START`'s), the refinement follows (docs/placer.md, "Where the arrangement is lost (2026-11-04)").
- `PCB_PLACER_START_PARTS`: `<file>`: with `PCB_PLACER_START_STAGE=global`, a JSON list of `REF#n` (the n-th footprint of that reference): only these start from the board's poses, the others from the placer's own start.

Diagnostics (stderr unless said; the result does not change):

- `PCB_ROUTER_CORRIDOR_CENSUS`: set: the router prints one `corridor census:` line per escape pad and lattice on stderr (its escape nodes' pocket open or closed on the lattice, and whether a corridor stub leaves it); `experiments/corridors/census.py` reads them (docs/router.md, "Corridors of pocketed pads (2026-11-05)").
- `PCB_ROUTER_TROUBLE_WHY`: the trouble-makers' repair prints every exchange's outcome (kept, or why refused; a refusal names the nets left in conflict and what their conflicts name), and after the repair the crossing census: every open pad of a fine-pitch row, what closes its straight ways out, and whether a row neighbour's target crosses its own (docs/router.md, "Crossings decided in the repair (2026-11-03)").
- `PCB_ROUTER_ROW_FANOUT_WHY`: the prepared row fan-outs (`row_fanout`) print each fine-pitch row's plan once a process: its part, layer, pads and pitch, the exits planned, whether they fan, the divers, and why a row was left to the search.
- `PCB_ROUTER_BGA_FANOUT_WHY`: the prepared fan-outs (`bga_fanout`) print each grid array's plan once a process: pitch, rows and columns, the rings that leave on the surface, the inner balls, the dog-bones planned by via size, the balls kept (a via in the pad) and those no gap fits.
- `PCB_ROUTER_NET_WORK`: `<file>`: the search expansions per net over the process (every router, every phase), written to the file as JSON at each router's finish; the last write holds the whole run (docs/router.md, "Prepared fan-outs for BGAs (2026-11-04)").
- `PCB_PAD_EXITS_WHY`: the pad-exits test prints, per closed pad, every rim node of its pocket and the item that closes it, and per open pad where its pocket opened.
- `PCB_PAD_EXITS_COUNTS`: pad-exits-kicad-board prints the probe's counts beside the CPU time of the lowering and the probe (for calibrating the test's work).
- `PCB_ROUTER_LENGTH_DEBUG`: the length repair: prints per net over its maximum what the priced search found, the victims and why the exchange was kept or undone; the length reroute: its router's log and per net the route's status.
- `PCB_ROUTER_DIFF_PAIRS_DEBUG`: differential pairs: prints each launch, each link's path, the copper refused with the reason, each stub straightened or not, and the fallback's work.
- `PCB_ROUTER_BUS_WHY`: bus bundles: prints every bus found and every member broken out of its bundle.
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
- `PCB_LAYOUT_DESCENT_CLASSIC`: `<n>`: the move descent's first `n` router trials are also tried and timed as the coupled loop tries a move (a clone, `Router::update`, a reroute without the polish) on a copy; the result does not change.
- `PCB_PLACER_TRACE`: `<file>`: per seed (its number, whether it was kept) the weighted wirelength after each stage, the placer's objective at each stage split by term (at the placement's level and without halos; at the designer's poses too with `PCB_PLACER_DESIGNER`), and per part each stage's pose and how far each stage moved it.
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
`PCB_ROUTER_ISLAND_STUBS` when the island stubs went off on 2026-10-21,
and went on 2026-11-04 ("Removed experiments (2026-11-04)" below).

## Removed experiments (2026-11-04)

Switches of levers that lost or were null under their measurement and
that nothing in the agent flow used, removed on 2026-11-04 (Florian: they
cost agents and maintainers and buy nothing; branch prune-switches, one
commit each). The default path is byte for byte as before (the
fingerprints, the short tier, the place checks; changelog.md, "the
failed-experiment switches removed"). What each measured, and where the
details are:

- `crossing_dive` (router, `PCB_ROUTER_CROSSING_DIVE`; off): in the trouble
  repair, one net of a crossing pair at a fine-pitch row took a via
  beyond the row. Route mode on the five near-miss boards: the same KiCad
  counts (Quanta75's dives traded the crossing for a key-matrix row on
  the other layer); the expansion tier 558 to 556 unconnected, strict
  passes 24 to 25; layout mode on the five boards the same open items;
  the eleven dense boards 876 to 878 (SNSP-CPU-01 97 to 90, MegaDrive
  118 to 125). Changelog 2026-11-04, "the crossing dive in the trouble
  repair, off"; docs/router.md, "Crossings decided in the repair
  (2026-11-03)". The crossing census (`PCB_ROUTER_TROUBLE_WHY`) stays.
- `fanout` (router, `PCB_ROUTER_FANOUT`, with `fanout_cost`,
  `fanout_depth` and the experiment knobs `PCB_ROUTER_FANOUT_COST`,
  `_DEPTH`, `_FREE`, `_VIAS` and its report `PCB_ROUTER_FANOUT_WHY`;
  off): before the negotiation,
  every pad of a dense row got one way out chosen jointly with its
  neighbours, reserved against other nets by a cost. Route mode on the
  five near-miss boards 8 and 8 unconnected (Quanta75's three top-row
  pads connect, another pad and the ground pour lose; OpenAirScope trades
  CMD for DAT3); layout mode 8 and 8, a pass each way; the expansion tier
  558 to 510 unconnected but strict passes 24 to 23; the eleven dense
  boards 931 to 966. Changelog 2026-10-28, "the fan-out step before
  negotiation: diagnosed, built, off"; docs/router.md, "Fan-out before
  negotiation (2026-10-28)". The plan's dense pads and straight ways out
  stay for the census (`fanout_plan`, axes only, no costs or via sites).
- `last_connections` (router, `PCB_ROUTER_LAST_CONNECTIONS`; 0, off): once
  the trouble repair's rounds left at most that many open terminals, a
  last pass with ejection chains three deep, board-wide victims and pour
  nets taking part. At 8 it closed no connection on the 12 near-miss
  boards (layout mode, 34 to 34 KiCad unconnected); the expansion tier
  558 to 555 unconnected with one router-check violation more
  (nervous-ecg, by the ladder's timing). Changelog 2026-10-27, "the last
  connections: why the near-misses stay open"; docs/router.md, "The last
  connections (2026-10-27)". The two trouble-repair fixes of that package
  stay on; the ejection chain itself stays reachable through
  `PCB_ROUTER_TROUBLE_DEPTH`.
- `capacitated_corridors` (router, `PCB_ROUTER_CAPACITATED_CORRIDORS`,
  with `capacitated_retries`, `capacitated_stuck` /
  `PCB_ROUTER_CAPACITATED_STUCK` and `capacitated_growth` /
  `PCB_ROUTER_CAPACITATED_GROWTH`; off): the negotiation's corridors from
  pcb-global's route with hard capacities, a miss lowering the boundary
  it failed at and re-planning the nets through it. The expansion tier's
  best variant (`g6`) 616 unconnected against 588 off, one pass more; as
  designed 667, the misses' learning alone 689; the fingerprints 46
  against 43; the dense boards 1267 against 1010. The negotiation's
  corridors seldom miss (0.7 % off, 0.16 % capacitated), so the learning
  had little to learn from. Changelog 2026-10-24, "capacitated corridors,
  experiment"; docs/router.md, "Capacitated corridors (2026-10-24)". The
  search counters stay (the verdict's `corridors`, without its `band`
  and learning fields).
- `island_stubs` (router, `PCB_ROUTER_ISLAND_STUBS`; off since
  2026-10-21): a pad whose spokes reach only a fill island got its own
  straight track into it, which changes no connection and only silences
  KiCad's isolated-island finding (Florian, 2026-10-21). k30's tracks
  rung 14 starved thermals to 1, mackerel-30's 19 to 3 when it was on.
  Changelog 2026-10-21, "starved thermals at scale"; docs/router.md,
  "Thermal spokes as KiCad counts them" (the island vias stay on). The verdict's
  `repairs.island_stubs`, board-router.json's `island_stubs` and the
  environment echo go with it; compare.py prints the count only for rows
  of older binaries.
- `align_rows` (placer; off): rows of repeated parts, parts of one kind
  paying in the anneal for standing out of line with the nearest of
  their kind and lined up after the refinement where it cost at most
  0.5 mm of wire. Stage 1 of the placer protocol (five seed pairs on the
  dense boards, the first route's open terminals): **+4.2 %** (p on
  better 0.79 / 0.82), fails; Glasgow better (-8.6 %), the 4in1-mini
  (+12 %), protoesp (+23 %) and video worse; the rows bought with wire
  (up on 8 of 11) and crossings. Changelog 2026-11-04, "the designer as
  teacher"; docs/placer.md, "The designer as teacher (2026-11-04)".
  `legal::align_orientations` (on) stays. experiments/placement-gap's
  scripts that pass `{"align_rows": {}}` need a binary before the
  removal.
- `routing_demand` (placer, a factor; 0, off): every net's estimated wire
  area (half perimeter times the track pitch, over its signal layers)
  as charge over its box in the global placement. Measured when it was
  built (2026-10-07, commit 8e133a08): the ESC mini's wirelength 433 at
  1.0, 446 at 3.0, 446 without; no win within the seed noise there
  (docs/todo.md, "Crowded boards"), and it did not move the dense boards
  (docs/benchmarks.md, the placement gap). The global placement's
  `routing_layers` and `track_pitch`, read only by it, go too.
- `routing_room` (placer, with `routing_room_share`, `_floor`, `_most`,
  `_all_parts`; off): after a seed's placement each part's halo rescaled
  by the RUDY demand around it within the same halo area, then annealed
  again. Measured 2026-10-14: it never fired on video, the OpenESC 4in1
  and hackrf; on the five boards where it fired, passes over 15 runs off
  11, share 1.0 9, all parts 11, share 0.75 9, share 0.5 9; the quick
  tier 22 to 24 passes, unconnected 22 both, vias -3.7 %: within the
  seed noise. docs/placer.md, "Routing room from congestion (measured
  2026-10-14, off)"; changelog 2026-10-14, "the routing-room agent";
  scripts experiments/routing-room/. The race's `routing_room_failed`
  rank goes with it; the per-part halos (`Relaxation::halos`) stay, the
  skeleton race sets them.
- `shared_halos` (placer; off): neighbours kept the larger of their two
  halos between them, not the sum. Measured 2026-10-14: six gap boards'
  median final wirelength -6 %; the quick tier (with the annealer's
  gaps, x700) passes 19 to 20, unconnected 30 to 24, starved thermals 17
  to 9, vias -5 %, but zpn_devboard lost its pass (a mask bridge, vias
  299 to 348) and hackrf's vias went 429 to 673. docs/placer.md, "Where
  the wirelength goes (measured 2026-10-14)"; changelog 2026-10-14, "the
  legaliser agent, x701 / x711". `Problem::shared_halos` and its branches
  in the anneal, the global placement, the halo budget and the legality
  test go.
- `anneal_far_side_gaps` (placer, `AnnealConfig::legal_far_side`; off):
  the annealer kept the legality test's gaps across the board (holes the
  full spacing from bodies, far-side copper the clearance from pads).
  Measured 2026-10-14: it halved the parts legalisation moves on Castor
  but won no wirelength; layout flat on the quick tier (x700 against
  x651: 19 passes both, unconnected 29 to 30, starved thermals 12 to 17,
  vias -4 %), worse on video (240 to 315 unconnected) and the OpenESC
  4in1 (120 to 132), SNSP-CPU-01 239 to 224. docs/placer.md, "Where the
  wirelength goes (measured 2026-10-14)"; changelog 2026-10-14, "the
  legaliser agent, x701 / x711". `State::far_overlap` goes;
  `anneal::overlaps` takes no configuration.
- `ratsnest_crossings.global` (placer; off; the other parts of
  `ratsnest_crossings` stay, `anneal` on since 2026-10-28): after the
  global placement the parts of 16 pins or more traded places and
  turned where wire plus crossings fell (`crossings::reorder`). On
  MegaDrive it traded in two of three seeds' global placements, but the
  placed board came out the same (seed 1: 8135 crossings with and
  without); on Glasgow it found nothing; not measured in layouts.
  Changelog 2026-10-25, "ratsnest crossings in the anneal";
  docs/placer.md, "Where the order is decided: the anneal and the global
  placement (2026-10-25)". The anneal's large-part peers keep its pin
  thresholds (`REORDER_PINS`, `REORDER_PIN_SHARE`).
- `net_weight` (placer; `"steep"`, the default and now the only
  weighting, `board_placer::net_weight`): `sqrt`, `flat` (every net 1)
  and `rail` (1 for nets of nine pins or more and those named as a
  supply or ground). Stage 0 shortened the large nets (the 9+ pin nets'
  excess over the designer's 24 % steep, 18 % sqrt, 16 % flat, 14 %
  rail); stage 1 (five seed pairs on the dense boards, the first route's
  open terminals): `rail` **+5.6 %** (p on better 0.953), `flat`
  **+4.0 %** (0.883): both fail. Changelog 2026-11-04, "net weights: the
  rails and buses counted more, a switch; stage 1 fails"; docs/placer.md,
  "Net weights (2026-11-04)". The `net_weight` key goes from the placer
  settings; experiments/placement-gap's `place.py` variants that set it
  need a binary before the removal.
- `legality` (placer; `"model"`): `"kicad"` (KiCad's legality at every
  level whose spacing is the rules' least) and `"strict"` (the model
  before 2026-10-27, bodies the copper's inset apart at the last level
  too). `"kicad"` judged on 2026-10-26: the dense sum of medians 967 to
  934, but two of three seed pairs rose (seed 1 +37, seed 101 +89), so it
  failed the flip's conditions; the short tier 7 to 7 strict passes,
  unconnected 22 to 24. Changelog 2026-10-26, "the placer's legality
  against KiCad's"; docs/placer.md, "The legality model against KiCad
  (2026-10-26)". The last level keeps KiCad's legality
  (`core::Config::kicad_last_level`, always on from the KiCad layer);
  `legality_census.py --legality` and `legality-kicad-board --placer
  '{"legality": ...}'` go.
- `final_route_from_state` (layout, `PCB_LAYOUT_FINAL_FROM_STATE`; off):
  the final route started from the router's state after the first route
  and the moves, its legal nets kept as the board's own copper, the
  ladder on the rest within 0.3 of the work left, from scratch when its
  first attempt left more open than the first route. Measured (x1741):
  the eleven dense boards 973 to 979 KiCad unconnected with the descent
  off (better 0, worse 3), 988 to 998 with it on; the short tier the
  same (22 / 22, passes 6 / 6). Changelog 2026-10-23, "routes survive
  moves; the narrow step's bound"; docs/placer.md, "Routes survive moves
  (2026-10-23)". The ladder's `fresh_open` give-up, used only by it,
  goes too; route mode's kept routes of a designer (`kept.rs`) stay.

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
