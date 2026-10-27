# Changelog

What each round of work measured and changed, newest first: the dated
sections of the old hand-over (docs/handover.md up to 2026-10-16), moved
here verbatim. "Above" and "below" in them refer to the old hand-over's
order, oldest first; read "above" as further down this file. The current
state, the benchmarks that count and the open problems are in
[handover.md](handover.md); git log has the commits themselves.

Sections are ordered by the date in their heading, newest first, and
within one date by when they were written. At the end: the sweeps before
v16 and the hand-over's summary of the week up to 2026-10-09 (binary x155).

## Committed on 2026-10-27 (vias in small SMD pads: the regression and the fix)

Branch vias-in-pads, snapshot x1858 (6dbe3508, the chamfer and trapezoid
fix) and build/bin/pcb-maker-viapad1-b (65231466, with the custom pads)
against main's x1857 (docs/router.md, "Vias in pads": which vias the
rule covers).

**What the nightly saw.** The nightly of 2026-10-26 (x1842) counted 3
of our vias in small SMD pads, where v21 (x1832) and every sweep since
the fence fix had 0: koeg 2, bumwings_v001_core 1. None is in a pad.
Measured with KiCad's own pad polygons (pcbnew 10.0.7,
`PAD::TransformShapeToPolygon`, the via's centre to the copper):

| Board | Via | Pad | Net, layer | Centre to copper | Ring | Placed by |
| --- | --- | --- | --- | --- | --- | --- |
| koeg | (232.92, 133.25) | D23.4, chamfer 0.5 bottom_left | GND, B.Cu | 0.3465 mm | 0.3 | the search: the pad's escape stub, three diagonal steps on B.Cu, the via step (on to In2.Cu) |
| koeg | (117.92, 76.55) | D83.4, the same footprint | GND, B.Cu | 0.3005 mm | 0.3 | the search: the escape stub, three steps on B.Cu, the via step |
| bumwings_v001_core | (257.937, 89.789) | SW32.1, trapezoid, rect_delta 0.8 | col_Ri2 | 0.4337 mm | 0.4 | the search's via step |

The router lowers both shapes exactly (`roundrect_pad`'s chamfers,
`lower_pad`'s trapezoid) and let each pad's own net's via touch the pad
from outside, as the rule says. score-kicad-board measured the drill
against the pad's nominal box with its corners rounded: the chamfer's
cut and the trapezoid's slanted side were copper to it (0.124, 0.140
and 0.195 mm from the drill's centre, under the drill's radius). The
koeg board of x1849 has 19 such GND vias beside the LEDs' chamfered
pads (0.3005 to 0.38 mm); two happened to lie within the box.

**The bisect** (layout mode, task budget, seed 1, both boards; vias in
pads by the old score / routed / KiCad unconnected): koeg x1832 0 /
164/175 / 11 (500 vias), x1833, x1840, x1842, x1845, x1847, x1849 2 /
160/175 / 15 (419 vias, the same board as the nightly's, byte for
byte); bumwings x1832 0, x1833 0, x1840 0, x1841 0, x1842 1 (the nightly's
board), x1845, x1847, x1849 1, 98/98 throughout. Koeg's came with the
exact escape stubs and the pour model (x1833; the placement differs
too, the placer's probes route); bumwings' with the placer's
fixed_share 0.6 (x1842, 29508083; x1841 routes as x1840): other routes put a via beside
a cut corner, no via placer changed.

**The fix** (quality.rs): the vias-in-pads count measures the drill
against the pad's copper as the router lowers it (`lower_pad`, then
`ObstacleGeometry::router_shape`, the conversion the lowering uses, now
beside the geometry): chamfered corners, trapezoids and custom shapes;
the rounded box remains where the lowering fails. A custom pad is small
by its copper's extent, not its anchor (with the exact shape alone
d20_pcb's touch electrode, a die's face on a 1.5 mm anchor, counted the
via the router puts there by design; x1858 had it, viapad1-b not).
Unit tests: the four vias of the nightly's two shapes (cut corner 0,
uncut corner 1, beside the slant 0, in the wide end outside the box 1)
and a custom pad (quality.rs); every node near D23.4 and SW32.1 where
the pad's net may hold a via keeps the ring out of the pad's copper,
and the box would have counted some of them (board_router/tests.rs).

**Measured** (routes unchanged by construction; checked):

| | x1857 | fix | moved |
| --- | --- | --- | --- |
| nightly 2026-10-26's result boards, our vias in pads (104 boards) | 3 | **0** | koeg 2 to 0, bumwings_v001_core 1 to 0 |
| expansion tier (46 x 8 x 1), routed boards | | | all 46 byte-identical, rows equal (557 unconnected, 0 new errors, 4263 vias, work 1143 s) |
| expansion tier, vias in pads, ours and the designer's (92) | 133 | 133 | none (x1858 without the custom pads: backplane's designer 0 to 1, MSign's 7 to 19) |
| fingerprints (1 x 4, a quarter of the budget, v16 placements) | 42 unconnected, 20 new errors, 1562 vias | the same | all 4 boards byte-identical |
| short tier (3 x 2, viapad1-b) against the nightly of 2026-10-27 (e7a12a80) | 20 unconnected, 8 passes, 1245 vias | the same | every row equal; our vias in pads 0 on all 12 |

## Committed on 2026-10-27 (every ladder rung keeps the disallowed layers closed; the narrow step keeps rule widths)

Branch ladder-rules; snapshots x15090 (both fixes) and x15091 (x15090
with a `disallow track` rule honoured only where its findings are errors:
the measured on side but for the dense layouts). Off side x1856 (main;
x1855 and x1856 route as x1854: the small rule boards, right_io and
left_io give x1854's rows of build/dp7-rules; ducktop2-center's off row
is x1854's, build/dp7-rules/x1854-30-center, not rerun: 15 GB on a full
machine). docs/router.md, "Project
rules", "Disallowed layers and rule widths on every rung", has the design.

- **Every rung keeps the disallowed layers closed.** The project gives the
  router, per net, the layers where a `disallow track` rule surely holds
  for its tracks (`track_disallowed`; KiCad's `inDiffPair()` known; only
  rules whose findings are errors), and every lowering keeps the net's
  class off them (`RuleClass::layers`, with a pair's layers). Before, only
  the pairs' nets were kept off, and the exclusive planes kept the rest
  off on the rungs that have them: on right_io with the crossovers (x15074)
  the probes ranked pours-as-tracks first, a rung without planes, and 58
  items_not_allowed followed. The probes, seed retries, the continuation
  in the other pour mode and the steps all lower through the same path.
  Such a rule is now reported `applied`. Unit tests
  `every_rung_keeps_a_nets_tracks_off_the_layers_a_rule_disallows` (two
  nets, GND poured on In1.Cu, In1.Cu the only way across: without the rule
  the pours-as-tracks rung routes /A over it, with it no rung does) and
  `a_disallow_track_rule_closes_its_layers_for_the_nets_it_holds_for`.
- **The narrow step keeps rule widths** (`ruled_widths`): a net whose
  width a rule gives (a per-net `track_width` opt; a pair's `inDiffPair()`
  width, set by `with_pairs`) is not narrowed, and the step runs only
  where a net at its class's width is wider than the narrow width. Unit
  test `the_narrow_step_keeps_the_widths_a_rule_gives`.
- **Measured** (route mode, 30 s, one thread, the task list of
  dp7-rules30 (`rung1-rules30-tasks.json`), the ducktop2 boards one at a
  time; KiCad's errors by type, unconnected, links coupled/links):

  | 30 s | errors by type | unconnected | links coupled | vias | rung kept |
  |---|---|---|---|---|---|
  | ducktop2 right_io | gap 38 -> gap 49 | 159 -> 158 | 1/23 -> 4/23 | 145 -> 77 | plane stubs -> pours as tracks |
  | ducktop2 left_io | gap 17 -> 17 | 337 -> 337 | 0/41 -> 0/41 | 240 -> 240 | identical board |
  | ducktop2-center (off: x1854) | gap 16, clearance 2 -> same | 499 -> 499 | 2/54 -> 2/54 | 689 -> 689 | identical board |
  | OpenAIO-Whoop | none -> none | 185 -> 185 | 0/4 -> 0/4 | 39 -> 39 | identical board |
  | oculink m2_card | gap 1 -> 1 | 2 -> 2 | 9/9 -> 9/9 | 22 -> 22 | identical board |
  | oculink_m2 | hole_clearance 4 -> 4 | 9 -> 9 | 4/10 -> 4/10 | 54 -> 54 | identical board |
  | oculink_port | track_width 19 -> 19 | 12 -> 12 | 3/9 -> 3/9 | 20 -> 20 | identical board |

  right_io's probes on the on side: plane stubs 126 nets unfinished
  (123 before: other nets no longer cross In1, In3, In6 there), pours as
  tracks 125, so the pours-as-tracks rung continues, now without a track
  on the ground layers (items_not_allowed 0 either way). Where the
  defects showed (the crossovers' candidate, `PCB_ROUTER_DIFF_PAIRS_CROSSOVER=1`,
  and the narrow step forced from the start, `use_narrow_signals` true in
  the task's router settings; same binaries):

  | 30 s | errors by type | unconnected | links coupled | vias |
  |---|---|---|---|---|
  | right_io, crossovers on | gap 36, items_not_allowed 58, starved 1 -> gap 30 | 151 -> 162 | 4/23 -> 2/23 | 94 -> 148 |
  | oculink_port, crossovers on | gap 1 -> 1 | 12 -> 12 | 7/9 -> 7/9 | 18 -> 18 |
  | oculink_port, narrow signals | gap 23, track_width 105 -> track_width 24 | 9 -> 8 | 3/9 -> 3/9 | 21 -> 22 |

  With the crossovers right_io's probes tie at 122 (plane stubs, pours as
  tracks), the earlier rung, plane stubs, continues: the 58 tracks of
  other nets (ETH_CT, MCU_3V3, ...) on the solid ground layers go. On
  oculink_port with narrow signals the pairs' tracks at 0.15 mm (72: 66
  outer, 6 inner) are gone: every pair track is at the rule's 0.1712 mm
  but the neck-downs at 0.127 (24, as off's 19 track_width at 30 s); the
  crossover run at 30 s does not reach the narrow step either way.

- **Identity.** The expansion tier (46 boards, route mode, 8 x 1, 30 s):
  every row and every routed board byte-identical (unconnected 557, 24
  passes both ways; the tier has no error-severity `disallow track` rule;
  the power manifold's opt widths matter only where the narrow step runs,
  and it does not). The fingerprints (Castor, k30-SBC, jetson, Sisu; route
  mode from github-layout-v16, budget scale 0.25, 1 x 4,
  PCB_ROUTER_FINGERPRINT=1): identical rows and boards. k30 has no
  .kicad_dru; Sisu's "Keep USB off L3 and L4" does not reach the
  fingerprint: github-layout-v16's placed projects carry no .kicad_dru
  (its route reports no custom rule, pairs off), so nothing changes there.
- **Dense layouts** (the eleven dense boards, layout mode, tasks v7,
  placer seed 1, 3 x 2; off x1856, on x15090: the dense boards have no
  `disallow track` rule, so x15090 and x15091 lower them alike): all 11
  rows and result boards byte-identical (unconnected 809, vias 9148). The
  narrow step ran on DSP-ADAU1452 and Glasgow on both sides; no net there
  has a rule width.
- Logs: build/rung1 (`rules30-*`, `expansion-*`, `fingerprints-*`,
  `dense-s1-*`).

## Committed on 2026-10-27 (panels of different boards)

Branch multi-board-panels, snapshots x15020-x15029 (docs/placer.md "Sub-
boards in a panel (2026-10-27)", docs/router.md "Nets split by sub-board
(2026-10-27)", docs/agents.md `sub_boards`). New module
`crates/pcb-kicad/src/sub_boards.rs`.

- **Pieces without the tabs**: the outline rasterised and opened by 1 mm
  (necks under 2 mm, rows of mouse bites, fall away); only pieces tabs
  join count. On the 109 sweep boards: the OpenESC 4in1 panel (2 pieces,
  2 tabs, 130 nets on both by its pads); no other board, the four panels
  of copies, zpn_devboard and the Modular Hub (separate loops) included.
  `pcb-maker sub-boards-kicad-board <board>` (research) prints them.
- **The router's net split** (`sub_board_nets`, router, on): a net with
  pads on two pieces, none on a tab and no copper of its own on a tab or
  over two pieces, is one net per piece (pours cut to each piece's box).
- **KiCad's check per sub-board**: KiCad still sees one net name and
  reports one item between the boards per such net (130 on the
  designer's own panel); as many items across as the net's pieces less
  one, of nets on several boards by design (split by parts whose board
  is known, or holding two parts of one reference), go to
  `unconnected_across_sub_boards` in drc.json and are not counted; at
  KiCad's cap of 499 the rest is a lower bound
  (`unconnected_items_capped`).
- **The placement** (`sub_boards`, placer, on): which board each part is
  on, from fixed parts, `--reference <project-dir>` (now on
  place-kicad-board too; the layout's reference also gives the boards)
  and the nets; with at most a tenth of the movable parts unknown, every
  part is held in its board, each board placed alone in its region, its
  nets split, the tabs kept clear; else nothing changes and a next action
  asks for `--reference` or region constraints (the task's 4in1 panel:
  392 of 398 parts unknown: the unplace step stacks them; the net graph
  cannot tell the boards apart). run.py `--reference-designer` passes the
  designer's project.
- **The molecules' cells**: copies told apart by repeated references
  whose nets share names: their regions are cells, those nets split per
  cell and judged per cell.
- **Fixes on the way**: a padstack layer's `offset` is its drill offset
  (OpenESC's motor pads' back copper sat 0.1 mm off: 16 clearance errors
  at x15024); a layout move into another sub-board is no trial (the
  router refused the update: the 5x4 wave's internal error at x15028).
- **Measured** (layout, task budget, seed 1 unless said; off x1853 =
  x1856 on the panel): the 4in1 panel 499 (KiCad's cap) to 353 counted
  (394 in KiCad, 41 across), no error, as the task is; with the
  designer's places 236 counted (366 in KiCad, 130 across) at seed 1 and
  240 (370) at seed 101, where both sides' whole-panel placement leaves 4
  parts without a place. Molecules: 4x4 499 to at least 244, 4x5 499 to
  at least 239, 5x2_shifted 427 to 219, 5x4 wave 499 to at least 204
  (x15029); the first route's open terminals 3 to 50 times fewer; 1
  starved thermal on the 4x4 (0 off), 1 on the 4x5 (8 off). The short
  tier (x1856 against x15028), the single 4in1 and 4in1-mini and the copy
  panels' placements byte for byte.

## Measured on 2026-10-27 (nightly: layout sweep v22, matched designer tier)

Written by the nightly from `build/nightly/2026-10-27/` (main e7a12a8;
[benchmarks.md](benchmarks.md#layout-sweep-v22-2026-10-27)). The guard
held: every stage ran, no row was cut or killed, the sweep has at least
100 rows with a result.

- **Layout sweep v22**: **57 of 109 pass the strict rule** (57 per
  type), 2140 unconnected in all, strict errors 53, 6 without a result.
  Against the nightly of 2026-10-26: better 4, same 99, worse 0, strict
  passes 57 -> 57 (gained 0xCB-1337 panel; lost
  Quanta75_RP2040Stamp_JLCPCBAoptimized); against v21: better 21, same
  76, worse 6, strict passes 56 -> 57 (gained 0xCB-1337 panel, Explorer;
  lost Quanta75_RP2040Stamp_JLCPCBAoptimized).
- **Medians against the designer's**: vias x0.73, length x1.07, open
  0.000 (mean 0.110), bends x1.15, stair-steps +60, crossings +1.
- **Matched designer tier** (budget scale 0.69): 63 strict passes (67
  per type); over the 99 target boards placement 52 %, routing 48 % of
  the open connections (placement 69 %, routing 31 % without Batch_1).
- **Placement costs most** on video (+189: 190 / 1), OpenESC-30x30 4in1
  (+91: 106 / 15), glasgow revC3 (+88: 143 / 55), protoesp (+72: 73 /
  1), 4in1-mini (+44: 81 / 37).
- **Worse beyond noise**: none.

## Committed on 2026-10-27 (the nightly writes its sweep section, README and hand-over itself)

Branch nightly-docs. The write-ups of v19, v20 and v21 were done by hand
from the logs while the nightly's `summary.md` had the same numbers, so
README.md and docs/benchmarks.md went stale between them. The nightly
now publishes its night ([benchmarks.md](benchmarks.md#nightly), "How
the nightly publishes"):

- **`nightly-report.py --docs <night>`** writes a "Layout sweep vN
  (<date>)" section into docs/benchmarks.md in the hand-written
  sections' shape (the totals table against the previous night and the
  previous numbered sweep; better / same / worse and the strict passes
  gained and lost with their finding types; INPUT boards named and left
  out; the rows without a result; the strict errors; the matched
  designer tier's gap split, the five costliest placements; the losers
  beyond the seed spreads of `dense-noise.json`), the night's "Nightly"
  row, README.md's headline and two newest rows, the hand-over's title,
  state line and night note, and a changelog section "Measured on
  <date> (nightly: layout sweep vN, matched designer tier)". vN is one
  more than the newest numbered sweep; a night written again keeps its
  number and replaces its own parts. `--dry-run` prints the section.
  (`--docs` named the checkout before; that is `--checkout` now.)
- **The guard**: every stage ran, no row cut or killed, at least 100
  sweep rows with a result; otherwise the night is summarised only and
  the summary's last line in nightly.log gives the reason.
- **Committed by default** (`--commit-report` is the default;
  `--no-commit-report` or `PCB_NIGHTLY_COMMIT_REPORT=0` turns it off):
  the four files alone on main, "nightly <date>: layout sweep vN, ...",
  pushed. Only when the main checkout is on main with no change to a
  tracked file and `git pull --ff-only origin main` succeeds; else
  nothing is written, nightly.log says so, and the next night publishes
  every guarded night after the newest published one (at most three).
- compare.py's INPUT test is one function now (`on_other_input`), used
  by its table and by the write-up.

Run on the night of 2026-10-26 into a scratch copy of the docs (not
committed; the first published section is tonight's): v22 = 57 of 109
strict (v21 56, the nightly of 2026-10-25 53), against the nightly
better 20 / same 74 / worse 6 with neotron-pico, Explorer, homebrew_486
and ghoul gaining the strict pass; the matched tier 64 strict, the gap
over the 100 target boards 57 % placement / 43 % routing (73 / 27
without Batch_1); losers beyond noise ATAT1800 5 -> 19,
poly_corne_split42_right 20 -> 25, ULK 2 -> 4, koeg 13 -> 15, vaio_re
9 -> 10 (the 0xCB panel's 50 -> 60 lies within its seed spread of 11).
Tests: scripts/test_nightly.py (the guard, the numbering, a night
published by nightly.sh into a git checkout and pushed, publishing
again changes nothing, the commit rule against a dirty tree, a
diverged origin and another branch, the losers, the wrapping).
## Measured on 2026-10-27 (a coarse first route as a placement ranker)

Branch coarse-probe, snapshot x14982 (on main 947ee550); docs/placer.md,
"A coarse whole first route as the race probe (2026-10-27)". Nothing in
the placer or the layout changed.

- **`pcb-maker probe-coarse-placement`** (a research command,
  docs/agents.md; `crates/pcb-kicad/src/coarse_probe.rs`,
  `Router::rip_up_standing`, `Router::finish_probe`,
  `Router::negotiation_progress` in `crates/pcb-router/src/router/coarse.rs`):
  a placed board routed whole with the race's settings at a reduced
  fidelity (the pitches times `--pitch-factor`, `--iterations`,
  `--hard-repair`, `--trouble-repair`) within a share of the task budget;
  it reports the standing when the negotiation ended, the open terminals
  once the conflicted branches are ripped (the hard repair's rip-up
  without its searches) and, with the repair, after it.
  `experiments/coarse-probe/study.py` runs and ranks it.
- **The study** (the trouble study's 75 placements of the eleven dense
  boards, x1828's seeds 1 to 601; 608 probes): pitch 2x, 1.5x and 1x at
  5 % of the budget rank the seeds at +0.44, +0.49 and +0.42 mean
  Spearman (5, 5 and 6 of 11 best picks, regret 110, 92, 94), 10 % no
  better (+0.40 to +0.42), 2 % worse (+0.23, +0.25); the hard repair at
  10 % +0.58 and +0.56 (regret 106, 132). The race's recorded 75 s probe
  +0.15 (5/11, 90), the first route +0.70 (7/11, 13). In a race of three
  (summed mean finals): the 75 s probe 989, the first route 871, the
  coarse settings 907 to 956.
- **The verdict: no.** No setting reaches the first route (about +0.6
  and a regret under 50), and the 2x lattice saves only 20 % of the
  race probe's work (549 against 686 s summed over the boards' medians),
  so six to nine probes fit the race's probe work only at 2 %, where the
  ranking falls. The budget loses the ranking first; the pitch changes
  which placements look hard on four of eleven boards but not the
  summed quality; the iteration cap shows no effect; the hard repair
  adds correlation, not picks.
- **The finding: the count.** On the same negotiation the open terminals
  after the rip-up rank at +0.42 to +0.49 where the race's standing
  ranks at +0.11 to +0.41; the race's own probe (1x, 75 s, the cap at
  25) ranked by it: +0.42, 6/11, regret 94, a race of three 910 against
  the standing's 1060 and the recorded probe's 989. Proposed for the
  next package: `race_probe.count = "ripped"` (off by default; the
  kept placement's count without ripping its router, which goes on as
  the first route), judged by the dense pairs; a second arm, six 2x
  probes at 2 % by the same count (902 in the simulation).

## Committed on 2026-10-27 (router profile round 3: the open list with a decrease-key; the restamp decodes each stamp list once)

Branch router-profile-3, snapshot x15185 (d975ffae) against main's x1856
(edc1d286); the candidates x15180 to x15184 against x1855 (630799ab)
(docs/profiling.md, "The third router round").

- **The open list with a decrease-key** (`SearchHeap`, heap.rs;
  search.rs): the 4-ary heap of `KeyHeap` (the empty root, the
  prefetches) keeping each queued node's slot in its `SearchNode`; a
  cheaper path to a queued node moves its key up instead of queueing a
  second one. No pop is stale (39 to 41 % were; 1.63 to 1.70 pops an
  expansion). The nodes expand in the same order: a node's heuristic is a
  function of its state, so its keys only fall, and the heap holds the
  least, which the lazy heap popped first. The node stays 12 bytes: the
  slot took the closed and target marks' bytes (`SLOT_CLOSED`,
  `SLOT_UNQUEUED` for a target not reached; the target mark is
  `since_via`'s high bit, its step count stopping at 127, a lower bound
  as before). `KeyHeap` stays for the tile planner and the bundles.
- **The restamp decodes each stamp list once** (`Stamped::iter_ahead`):
  each of its two passes decoded its list twice, in order and 32 entries
  ahead for the occupancy prefetch (decoding was 3.0 % of Castor's route,
  2.1 % of Sisu's); one decode now feeds both through a ring of 32 pairs.
- **Measured**, x1856 against x15185, two pairs a board on SMT siblings in
  both orientations, each pair inside one reservation of the memory
  ledger, pinned rates (the router's CPU seconds): Castor -3.0 %, jetson
  -1.3 %, k30-SBC -2.7 %, Sisu -3.9 %, SmartSpin2k -5.6 %, protoesp
  -6.8 % (geometric mean -3.9 %; instructions -5 to -9 %); every routed
  board, fingerprint line, expansion and search count identical, and the
  verdict apart from its work, wall, rate, memory and path fields. The
  expansion tier (46 boards, 8 x 1): 46 of 46 routed boards byte-identical
  to x1856's, the rows identical apart from their timing fields and the
  speed class of four boards, 557 unconnected, 0 new errors, 4263 vias
  both. Peak resident size (memory profile): Castor 602 / 606 MB against
  602 / 617 MB, k30-SBC 904 / 892 MB against 885 / 910 MB.
- **Tried and not taken** (k30-SBC at 60 s, both orientations): an 8-ary
  heap on whole cache lines (+1.9 and +1.4 % CPU; without its
  grandchildren's prefetch +1.8 and +5.0 %; branch
  router-profile-3-arity8-trial); the decrease-key with the slots in an
  array of their own (-2.8 and +1.8 %, 4 bytes a state more). Each kept
  change alone: the decrease-key -4.7 and -3.2 % (k30-SBC), the single
  decode -1.6 and -2.9 % (Castor at 60 s, on the decrease-key).
- **The work clock**: the canned search of an unpinned rate got about 2 %
  faster (six paired calibrations: medians 2.226 M and 2.273 M expansions
  a second): an unpinned run measures a rate that much higher.
- **Measuring under a full ledger**: a pair whose one side waits in the
  memory ledger while the other runs alone is void; the pairs now reserve
  for both binaries at once and start both inside that reservation
  (`PCB_MAKER_LEDGER_INSIDE`).
- Tests (pcb-router): `search_heap_pops_as_a_lazy_heap`,
  `search_heap_hints_the_next_pop`,
  `stamps_ahead_pair_each_stamp_with_the_one_ahead`;
  `key_heap_hints_the_next_pop` went with `KeyHeap::next_hint`.

## Committed on 2026-10-27 (differential pairs: the crossover kept only when its link couples)

Branch differential-pairs-7 (on main e7a12a80); snapshots x15070 (the
room beside the neighbours, crossovers still off), x15071 (on by
default), x15072 and x15073 (the room counting the lead past the
crossover, then as the neighbours' launches check it), x15074 (the
vias spread; on by default: the measured on side) and x15075 (the
branch: x15074 with the crossovers off by default). Off side x1854
(main). docs/router.md, "Differential pairs", "The safe crossover", has
the design and the tables.

- **A fallen-back pair leaves no crossover copper**: the crossover is
  part of its link's copper, written only for a link that couples end to
  end (both stubs reach the run, the run routed, the copper check
  passed); the plan's obstacles go with the board as lowered, the
  router's stubs of a failed link as before. The code did this already;
  unit test `a_crossed_pair_that_falls_back_leaves_no_crossover_copper`
  (run not routed, copper refused). The gap findings of 2026-10-25 on
  fallen-back pairs are the fallback's own routes of the two nets (the
  off side has them too: 54 of left_io's and right_io's 55 at 30 s).
- **Room beside the neighbours' launches** (`Neighbour`,
  `neighbour_room`, `ROOM`, `ROOM_COST`): each link end's launch on the
  board as lowered (its stubs, their ends' leads, the run's lead with the
  launch's margin); a crossover (tracks, vias, the run's lead past it)
  that reaches into another pair's is not taken, one with less than the
  gap plus clearance to spare costs, the search goes on along the
  direction. oculink_port's four TX pairs carry their stubs at J2 again
  (none did with the 2026-10-25 crossovers); unit test
  `a_crossover_keeps_room_beside_a_neighbours_launch`.
- **A link's vias spread** where one comes within the gap of the other
  net's copper or its hole within the hole clearance (`own_vias_clear`,
  `VIA_SPREADS` 1.5, 2, 3; with the crossovers).
- **Measured** (route mode, one thread, KiCad's DRC with
  `--refill-zones`, no setting, wall guard 1200 s; off x1854, on x15074;
  errors KiCad's errors, gap diff_pair_gap_out_of_range, dangling
  track_dangling and via_dangling):

  | 30 s | links coupled | errors | unconnected | gap | dangling | vias | return vias / pair vias |
  |---|---|---|---|---|---|---|---|
  | ducktop2-center | 2/54 -> 2/54 | 18 -> 14 | 499 -> 499 | 16 -> 12 | 0 -> 0 | 689 -> 690 | 3/4 -> 7/10 |
  | ducktop2 left_io | 0/41 -> 1/41 | 17 -> 24 | 337 -> 346 | 17 -> 24 | 1 -> 0 | 240 -> 248 | 0/0 -> 0/2 |
  | ducktop2 right_io | 1/23 -> 4/23 | 38 -> 95 | 159 -> 151 | 38 -> 36 | 2 -> 3 | 145 -> 94 | 0/0 -> 3/6 |
  | OpenAIO-Whoop | 0/4 -> 0/4 | 0 -> 0 | 185 -> 185 | 1 -> 1 | 0 -> 0 | 39 -> 39 | 0/0 -> 0/0 |
  | oculink m2_card | 9/9 -> 9/9 | 2 -> 2 | 2 -> 2 | 1 -> 1 | 0 -> 0 | 22 -> 22 | 0/0 -> 0/0 |
  | oculink_m2 | 4/10 -> 6/10 | 4 -> 3 | 9 -> 6 | 0 -> 0 | 2 -> 0 | 54 -> 57 | 7/12 -> 23/36 |
  | oculink_port | 3/9 -> 7/9 | 21 -> 3 | 12 -> 12 | 0 -> 1 | 34 -> 34 | 20 -> 18 | 2/6 -> 13/26 |

  Totals at 30 s: coupled 19/150 -> 29/150 (11 through a crossover),
  errors 100 -> 141, unconnected 1 203 -> 1 201, gap 73 -> 75, dangling
  39 -> 37, vias 1 209 -> 1 168. At 300 s on the four small boards
  (oculink_m2 4/10 -> 10/10, errors 43 -> 2, gap 37 -> 0; oculink_port
  4/9 -> 5/9, errors 17 -> 36, dangling 1 -> 34): coupled 17/32 -> 24/32,
  errors 61 -> 39, unconnected 158 -> 155, gap 38 -> 10, dangling 5 ->
  36. **Not the default**: at 30 s errors and gap findings rise in total;
  ducktop2 right_io blocks it (errors 38 -> 95: 58 items_not_allowed, the
  ladder's probe ranking pours=tracks, a rung without exclusive planes,
  one net ahead of the plane stubs) and left_io (gap 17 -> 24,
  unconnected 337 -> 346). The switch stays:
  `PCB_ROUTER_DIFF_PAIRS_CROSSOVER=1` (`CROSSOVERS_BY_DEFAULT` false).
  The branch with it off (x15075) routes the four small rule boards at
  30 s as x1854, rows and routed boards identical.
- **The expansion tier** (46 boards, route mode, 8 x 1, 30 s; x1854
  against x15071 and x15075, no setting): every row and every routed board identical
  to x1854 with both (unconnected 557, passes 24).
- Logs: `build/dp7-rules/x1854-30-{small,lr,center}`,
  `x15074-30-{small,lr,center}`, `x1854-300-small`, `x15074-300-small`,
  `x15075-30-small` (the branch: as x1854), the steps `x15070c-30-*`,
  `x15071-30-*`, `x15073-30-*`, `x15073-300-small`, the crossovers of
  2026-10-25 on x1853 `x1853c-30-*`, debug runs `dbg-*`; the tier
  `build/dp7-exp-x1854`, `dp7-exp-x15071`, `dp7-exp-x15075`.

## Committed on 2026-10-27 (the ledger's capacity from the machine; kicad-cli under a reservation)

Branch ledger-capacity (docs/benchmarks.md, "Memory ledger", "The
capacity from the machine" and "kicad-cli under a reservation"). The
ledger and the harness's admission only; routing and placement are
unchanged.

- **The capacity**: MemTotal less the memory outside the ledger, the
  median of the fresh headroom samples (the floor's freshness: 360
  samples, a day), else 25 % of MemTotal; at most MemTotal less the 6 GB
  margin (the static 56.7 GB). Here 44.6 GB from the night's samples,
  47.0 GB without samples; another machine without samples: 10 GB of 16,
  18 of 24, 24 of 32, 96 of 128. `machine_capacity_mb` in ledger.rs and
  harness_common.py; `pcb-maker ledger` and run.py's start line say what
  it is made of.
- **Binaries keep the floor**: a binary's new request keeps the measured
  floor (8.8 GB here, 10 GB without samples) beside it, a short one the
  2 GB to keep, unless no other run holds memory; it waits in the ledger
  with `floor_mb` and `waits_on_floor`. Growth of a run that holds memory
  keeps none.
- **The replays** (both languages, with the capacity and the other runs'
  reservations per knot): the evening and tonight's first hour start the
  same boards at the same times as with the floor alone (10 GB and
  measured), the least MemAvailable unchanged (28.9 and 17.5 GB with the
  measured floor): no added wait, no admission below 2 GB. The bound
  behind it: a board the floor admits fits the capacity whenever the
  outside memory is at least its median less the floor (9.3 GB; the
  night's least was above 16 GB). A fresh 39-minute sample (237
  samples, replay.py `--capacity`): 21 boards, the same starts and waits
  with the capacity of 45.4 GB; 18 of 97 direct binaries (one agent's
  probes) began below the floor they would now keep.
- **kicad-cli under a reservation**: every kicad-cli that loads a board
  runs inside its process's reservation (`ledger::kicad_check`): the
  check-only commands reserve the 2253 MB up front, every other command
  (fab exports, netlist import, research commands) arms at its first
  check; a route's and a layout's checks raise their reservation while
  they run, as before. `pcb-maker ledger` shows `[KiCad's check: 2253 MB,
  reserved|running]`. run.py's binaries of a board run inside the board's
  reservation (`PCB_MAKER_LEDGER_INSIDE`; after the board's binary, a
  short reservation of 4301 MB for the strict baseline and the route's
  check). Smoke on a private ledger and a read of the live ledger during
  a check: as documented.

## Committed on 2026-10-27 (the ledger's floor from measured headroom)

Branch ledger-floor (docs/benchmarks.md, "Memory ledger", "The floor from
measured headroom"). The harness's admission only; routing and placement
are unchanged.

- **The measurement**: a read-only sampler every 10 s for three hours of
  a busy night (1091 samples; experiments/ledger-floor). Outside the
  ledger 16.7-27.5 GB (p50 18.1: the browser 8 GB, the agents' builds up
  to 7.5 GB, kicad-cli and the nightly's binary outside any holder); its
  rise within 10 minutes p50 2.5, p95 6.7, max 8.9 GB (builds, 26
  episodes). No binary above its reservation at any sample; the holders
  resident at 13.4 GB beside 33.7 reserved (medians); MemAvailable never
  below 17.3 GB. The overshoot of the estimates (a time-ordered replay,
  788 rows): p95 at or below 0 but for layout (+52 MB); two watchdog
  ends in the week; two global OOMs (2026-10-24, -25) with the old 16 GB
  swap full killed browser processes, no pcb-maker run. The 10 GB floor
  alone held 3095 of 3893 waits in the logs since 2026-10-22 (2.89 of 3.27
  million board-seconds), with a median slack of 8.6 GB.
- **The floor**: `FLOOR_KEEP_MB` (2 GB) + the 0.95 quantile of the
  outside memory's 10-minute rise (samples of the last six hours beside
  the ledger, `<ledger>.headroom.json`, written by run.py's gate every
  10 s and by `benchmarks/ledger_headroom.py --sample`) + the board's
  kind's overshoot (estimates.json `overshoot`, from
  `tier_estimates.py --refresh`); 10 GB without an hour of samples;
  `--min-free-gb` overrides. `measured_floor_mb` in ledger.rs and
  harness_common.py; `pcb-maker ledger` prints it. Measured: 8.8 GB
  (layout 8990 MB). scripts/nightly.sh drops its `--min-free-gb 10`.
- **The replays** (both languages): the evening of 2026-10-26, four
  boards: 14 220 s waited, the last end at 6095 s with 10 GB (the
  evening's starts), 9840 s and 5877 s with the measured floor, the least
  MemAvailable 28.9 GB. Tonight's first hour, 14 boards: 9 started and
  20 766 s waited with 10 GB, 10 and 18 376 s measured, 17.5 GB least.
  Three hours, 123 boards (replay.py): 10.1 to 8.5 board-hours waited,
  the least MemAvailable 9.2 GB; with every board resident at its
  expected peak 4.1 GB (10 GB: 7.5; 6 GB: 1.4, below the 2 GB to keep).
- **Not changed**: the capacity (56.7 GB) is 11-21 GB more than the
  machine has beside the memory outside the ledger, and binaries admit by
  it alone.

## Committed on 2026-10-27 (router profile round 2: the expansion's remaining maps, the next pop's node and its blocked-step bits prefetched)

Branch router-profile-2, snapshot x14967 against main's x1851
(docs/profiling.md, "The second router round").

- **The profile again** (main a4e5de2, the six boards of the first
  round): the search 70 to 89 % of a route, with its heuristic 77 to
  94 %. Through the inline chain (a new attribution: each sample at its
  outermost search line): the neighbour steps 17-22 %, the heap 16-21 %,
  the occupancy lookups 11-18 %, the heuristic 5-8 %, the via steps 5-8 %.
  The costliest lines: a step's push and the pop (7-10 % each), a
  diagonal step's clearance count (6-10 %, not prefetched), the popped
  node's own record and blocked-step bits (2-4 % each).
- **Three prefetches** (search.rs; `KeyHeap::next_hint`,
  `StaticMaps::prefetch_edges`, `NodeBits::prefetch`,
  `LayerBits::prefetch`): the diagonal steps' clearance counts and the via
  step's maps of the cell at the expansion's start; the search node of
  the key the next pop most likely takes (the least child of the empty
  root) and its blocked-step bits, an expansion ahead. Hints only: every
  route is the same by construction.
- **Measured**, x1851 against x14967, two pairs a board on SMT siblings
  in both orientations, pinned rates (the router's CPU seconds): Castor
  -5.4 %, jetson -9.0 %, k30-SBC -4.4 %, Sisu -10.5 %, SmartSpin2k
  -6.0 %, protoesp unchanged (+0.0 %); every routed board, fingerprint
  line, expansion and search count identical, and the verdict apart from
  its work, wall, rate, memory and path fields. The expansion tier (46
  boards, 8 x 1): 46 of 46 routed boards byte-identical to x1850's (main
  ccb9504f, which routes as x1851), the rows identical apart from their timing
  fields and the intake's speed class (`outcome`, from the route's
  seconds: five clean_slow to clean_medium, one incomplete_at_budget to
  incomplete_early), 557 unconnected, 0 new errors, 4263 vias both.
- **Tried and not taken** (measured, k30-SBC at 60 s, both orientations):
  a radix heap for the open list (+4.9 and +5.0 % CPU; branch
  router-profile-2-radix-trial), the heuristic inlined as a struct (+1.9
  and +0.9 %; router-profile-2-heuristic-trial), a summary bit per 16
  cells of the occupancy (-3.3 and -1.0 % on k30-SBC, -0.9 and +3.8 % on
  SmartSpin2k; router-profile-2-summary-trial).
- **The work clock**: the canned search of an unpinned rate fits the
  caches and did not get faster (six paired calibrations: medians 2.074 M
  and 2.065 M expansions a second): an unpinned run measures the rate it
  did and spends the saving as wall time.
- Tests: `key_heap_hints_the_next_pop` (pcb-router): without a push in
  between, the hint is the key the next pop takes.

## Measured on 2026-10-26 (placement evaluation: seed noise and the protocol)

Branch placement-eval. Nothing in the placer or the router changed. The
weekend's placement levers lowered the objective and the crossings, yet
the old ship rule (three seed pairs on the eleven dense boards: the sum
of medians falls and every pair's sum falls) rejected each of them on
sums that moved by less than the boards' seed spreads. This round
measured that noise and set a rule of known resolution
([benchmarks.md](benchmarks.md#measurement-discipline-placer-defaults-2026-10-26)).

- **Noise**: the eleven dense boards on x1828 at nine placer seeds (1,
  101, 201 from the legality round; 301 to 801 new, `build/eval1-dense-s*-off`;
  seeds 2, 3, 5 ... would share 15 of seed 1's 16 race candidates). KiCad
  unconnected per board: sd 3 (the 0xCB panel) to 61 (video), spread 11 to
  193; the tier's sum 843 to 1159 (sd 97). The first route's open
  terminals follow the final count (pooled within-board r 0.73) with as
  much spread: not a cheaper proxy. The objective (wirelength plus 3 mm
  per crossing) spreads 2 to 16 % and explains about a fifth of the
  unconnected count's spread within a board (r 0.47). Noise file
  `benchmarks/agent-tasks/dense-noise.json`.
- **Resolution** (`placer_ab.py resolution`, power 0.8 at 5 % false
  positives): the sum of medians 18.5 / 15 / 12 % at 3 / 5 / 9 pairs; the
  old rule 15 % at three pairs with 12 % false positives (it finds a
  true 5 % gain a third of the time); the weighted relative change
  (each board's mean change over its noise mean, weight 1 / max(cv,
  0.10)^2) 9.5 / 7 / 5.5 %.
- **The rule** (benchmarks.md; `placer_ab.py compare` prints the
  verdict): five pairs (seeds 1 to 401); better when the weighted
  change's p (the larger of a paired sign-flip test and the noise
  file's null) is at most 0.03; at 0.03 to 0.25 four more pairs and the
  same test over nine. Simulated: 3.5 % false positives, power 0.67 at a
  5 % effect, 0.90 at 6.5 %, 0.97 at 8 %; 5.8 pairs on average under no
  effect.
- **Re-judged** (existing rows only): crossings in the anneal -8.1 %
  weighted (p 0.047 / 0.013; 8 of 11 boards better, video +26 %): extend
  to five pairs, which at that size find it with power 0.8; bus groups
  -0.5 % (p 0.45), while its objective falls 9.4 % with confidence:
  not better, no seed count would confirm it; `legality: "kicad"` +2.8 %
  (+5.3 % on the four boards it moves): not better; `decoupling_near`
  +4.5 % over two pairs: not better. For reference, the shipped defaults:
  escape demand -1.5 % (p 0.36), ratsnest crossings -4.1 % (p 0.03 /
  0.13: extend; 18 pairs to confirm its size).
- **Tool**: `benchmarks/agent-tasks/placer_ab.py` (compare, resolution,
  protocol, noise, report) and its tests; the agent-tasks README points
  to the rule.
- **Open**: the noise file is x1828's; main's later layouts of protoesp
  and the 0xCB panel (the escape guard of x1842, the panel copies of
  x1849) need those boards' nine seeds again before the next lever is
  judged on them.

## Measured on 2026-10-26 (the trouble-maker probe as a placement ranker)

Branch trouble-probe; snapshot x14900 (main 99b6259 and the branch's
code). docs/placer.md, "The trouble-maker probe (2026-10-26)", has the
design, the tables and the verdict. Nothing in the placer or the layout
changed.

- **`pcb-maker probe-trouble-placement`** (a research command,
  docs/agents.md): a placed board's trouble-makers (from a previous
  run's board-router.json, a previous probe's `new_trouble`, a list of
  names, or cold: pins times box; 15 % of the nets, pour nets out)
  negotiated alone with a work budget (`Router::probe_subset`), the other
  nets as no background, fixed stubs out of their pads
  (`stamp_background_stubs`) or the global route's tile usage as a cost
  (`add_background_cost`); reports the set's standing, work per
  connection and per mm, corridor misses and the next set. A per-net
  expansion counter (`NetState::expansions`) is the only change to the
  router's state; routes are unchanged.
- **The study** (`experiments/trouble-probe/study.py`; 75 placements of
  the eleven dense boards, x1828's seeds 1, 101, 201, 301, 401, 501 and
  601; 614 probes and 22 whole-board 75 s probes): with seed 1's set and
  no background the probe orders the other seeds at a mean Spearman of
  +0.24 (the race's 75 s probe +0.08, the first route's open terminals
  +0.63), picks the best on 4 of 11 boards (6, 6), regret 69 (62, 40; a
  random pick 275). 2 % of the budget is the same as 5 % (most probes
  stall first); `stubs` and `global` are worse than none with every set;
  seed 101's set gives the same mean (+0.19) but other boards. The cold
  set does not rank (regret 223; in a race of six no better than random).
  The iteration (seed 1's set and the picked placement's trouble-makers)
  orders the rest better (+0.07 to +0.22, best picks 1 to 4 of 11) at
  twice the work.
- **Cost**: 3.6 to 4.5 times less search work than the 75 s probe (190
  against 688 s, the per-board medians summed; 153 against 686 on seed
  301's boards), 2 to 3 times less wall clock; 3 to 9 s on half the
  boards, 24 to 39 s on protoesp, split72_left, video and MegaDrive.
- **Verdict**: representative at the 75 s probe's level where an earlier
  run of the board names the set, not cold (the race's usual case);
  proposed for the next package only as an option with an earlier run
  (nine placer seeds pre-ranked by it, the best three raced as now).

## Committed on 2026-10-26 (the schematic path: a footprint's own findings, the decoupling action, KiCad 10 names, the template)

Branch schematic-path-items, snapshot x14950: the schematic trial's
frictions 2, 3, 4, 7 to 10 ([the review](reviews/2026-10-26-schematic-trial.md),
"What remains"; docs/agents.md).

- **A footprint's own findings** (verdict.rs, fab_release.rs): without a
  reference board, a DRC error whose every item lies within one footprint
  (no short) does not count (`footprint_findings`; the fab manifest's
  `footprint_own`, `footprint_note`; drc-kicad-board's `footprint:` line),
  and the next action gives the custom rule for the board's `.kicad_dru`
  at the smallest value KiCad measured, rounded down (J1's `hole_clearance
  (min 0.19mm)`, `memberOfFootprint('J1')` on both sides), or a KiCad
  exclusion. Unit tests `a_footprints_own_findings_do_not_fail_a_layout_without_a_reference`,
  `a_footprints_own_findings_do_not_block_the_release`.
- **The decoupling action** (quality.rs, verdict.rs): a far supply pin is
  named only where a capacitor that counts for it is free of a near
  constraint (the example is that capacitor at that pin); decoupling_near
  is offered only when off and a free capacitor exists; otherwise it says
  that a capacitor at those pins is a schematic change. Unit test
  `a_layout_with_its_decoupling_far_from_the_pins_gets_a_next_action`.
- **KiCad 10 names and add-on libraries** (netlist_import.rs): a library's
  one footprint whose name ends in `_<name>` (`Texas_DSG0008A_WSON-8-...`),
  KiCad's add-on folder (`KICAD10_3RD_PARTY`, `~/.local/share/kicad/10.0/3rdparty`)
  and PCM's nickname (`PCM_Espressif` for `Espressif`); `substituted` says
  why, `add_on_libraries` lists add-on footprints, a missing Espressif
  library says where to get it. The trial's import without
  `--footprints-from`: U5 and U6 substituted, U10 missing (no add-on on
  this machine) with the hint; with the trial's ESP32 footprint in a
  stand-in add-on folder, complete (65 footprints, exit 0). With
  `--footprints-from` the import is byte-identical to the trial's. Unit
  test `kicad_10_names_and_add_on_nicknames_are_found`.
- **The template** (constraints_template.rs): supply pins by function
  (never EN, SHDN, CE, SLEEP, CS), then electrical type, then net name;
  without a placement, the IC the schematic draws a capacitor nearest to.
  The trial: C7 at U6:1 (was the LDO's EN, U3:3), C8 at U5:1, C9 at U6:8,
  C10 at U5:8, the designer's pairs. Unit tests
  `a_decoupling_capacitor_goes_to_a_supply_pin_of_the_ic_the_schematic_draws_it_at`,
  `supply_pins_are_told_by_function_type_and_net`.
- **Small items**: score-kicad-board's `manufacturing.drc` is `null` with
  `drc_note` ("no DRC run") without a drc.json; `place` with x/y is listed
  in board-layout.json's constraints (kept, checked against the final
  pose); a second import names constraints.json and layout.json and lists
  them in `kept`. Unit tests in quality.rs, placement_constraints.rs
  (`place_takes_words_and_exact_poses`), netlist_import.rs.
- **The trial's layout rerun from the schematic** (x14950, the trial's
  import with `--footprints-from`, constraints.json and layout.json):
  the board byte-identical to the trial's, 64/64, KiCad 0 unconnected, 65
  vias, 2468 mm, 19 of 19 constraints kept (the trial's 16 and MH1 to MH3's
  `place`), exit 0 (was 1: hole_clearance 4); `fab-kicad-board` released
  it (exit 0, 21 files) without a `.kicad_dru`; the offered rule in a
  `.kicad_dru` makes KiCad's DRC find 0 errors. build/ux3-trial2.
- **The expansion tier** (46 boards, route mode, 8 x 1, x1848 against
  x14950): 46 of 46 routed boards byte-identical, every row the same
  (unconnected 557, passes 24 both ways). build/ux3-expansion-x1848,
  build/ux3-expansion-x14950.

## Committed on 2026-10-27 (panel copies: cell classes; legality against KiCad on the molecules)

Branch molecule-panels, snapshots x14980 (first version), x14981 (pads
keep the project's copper-to-edge clearance at KiCad's legality),
x14985 (held instances without a fitting copy; free re-placement where
a copy's region cannot hold a part). placer.md "Cell classes
(2026-10-27)" and "The molecules against KiCad: the last level is
KiCad's legality (2026-10-27)" have the method and the tables.

- **Cell classes** (`panel.rs`): where references repeat, an instance
  an edge, overhang or region constraint holds goes to a copy whose
  region lets it keep the constraint (a matching; the nearest copy where
  none can); the copies are grouped by their surroundings (constraints
  on their parts, outline pieces and obstacles within reach, each in its
  own frame), and where more than a tenth of the replicated parts are
  not legal each class's representative is placed in its region (the
  most constrained first) and replicated within its class. A replicated
  part its copy's region cannot hold is placed free afterwards.
  `panel_copies` reports `cell_classes` and `by_cell_classes`.
- **The last level is KiCad's legality** (`legality: "model"`, the
  default; `"strict"` the model before): courtyards touch where pads of
  two nets keep the copper clearance (pads of one net may come closer),
  pads keep the project's copper-to-edge clearance (floor 0.01 mm, at
  every level, was 0.02), the board's silkscreen marks the project does
  not rate an error over a part (`silk_over_parts_error`, read from the
  project) are no obstacle, and leads keep the copper clearance from the
  far side's pads where holes in courtyards are no error. Each cause was
  checked against KiCad's DRC of the designer's molecules; the designer
  placements' illegal parts at the last level 128/156/101/87/148 ->
  0/0/1/6/7 (12 of the 14 left are where KiCad finds an error too: the
  wave boards' J5 by a non-plated hole).
- **Measured** (layout, task budgets, x1849 against x14985 / x14981):
  the molecules seed 1 and 101: 4x4 31/27 parts without a place -> 499
  (KiCad's cap) unconnected, no error; 4x5 42/43 -> 499, 8 starved
  thermals at seed 1; 5x2_shifted 22/10 -> 427/369 (placed by cell
  class), no error; 5x4_wave 5/5 -> 499, 1 starved thermal at seed 101;
  7x5_wave_left 6/6 -> 1/3 parts without a place (still refused: its
  J3/J4/J6 are in no copy). The short tier byte for byte; the dense
  boards byte for byte except SmartSpin2k (21/21 unconnected, vias 745
  -> 531) and the 4in1-mini's pool work (365 -> 290 s, the same board);
  OpenRX 191 -> 197 (seed 1), 211 -> 200 (seed 101).

## Committed on 2026-10-26 (the free-dimension fixture under load)

Branch free-dimension-fixture (docs/benchmarks.md, "Measurement
discipline in the tests"). Tests only; the router and the placer are
unchanged.

- **The cause**: `f2s_free_dimension_stops_at_the_first_clean_size`
  removed `PCB_WORK_RATE` so that the parent measured its rates and pinned
  them for the children. Under a load of 30 to 60 the measured search
  rate was 0.4e6 to 0.97e6 (the reference 3.15e6), and F2's 6 s probes
  counted 14 to 31 s of work (4.2 and 6.4 s at the reference: their work
  does not stop at so small a budget), so the retry of 20 mm left less
  than a probe of the search's 60 s and 22 mm was not confirmed (the
  tables of 15 failed runs left in /tmp; the wall guard was off already).
  Pinned at 0.8e6 it fails every time, at the reference it passes.
- **The change**: the clean path runs at the fixtures' pinned rates (20
  and 22 probed, 20 retried, 22 confirmed, as before); a new test,
  `f2s_free_dimension_children_run_at_the_parents_rates`, runs a 3 s
  search without `PCB_WORK_RATE` and KiCad's check and asserts only that
  the parent measured (not the fixtures' rates) and that every child ran
  at its rates (`work_rate_source` PCB_WORK_RATE). The walkthrough's two
  60 s wall bounds became checks of what ran (no `placer:` line before the
  input error) and of work (under 30 s; 8.6 s, 16 s of wall under a load
  of 50); the hostile inputs' routes turn the wall guard off.
- **Audit** (`env_remove("PCB_WORK_RATE")`, `Instant`, `timeout`,
  `probe_seconds`): cli_exit_codes.rs measures its rates in every test
  but routes only the three-pad board (8 ms of work, no outcome a rate
  changes) and pins them for its free-dimension test; its 600 s ledger
  wait is a hang guard. router_fingerprint.rs and the fixtures pin rates
  and the guard; the crates' tests count work, not wall.
- **Proof**: the whole fixture suite (11 tests) 10 times beside
  `stress-ng --cpu 16`, load 36 to 62: 10 passed; 10 times without it
  (the other agents' load, 39 to 54; the machine was not idle): 10
  passed.

## Committed on 2026-10-26 (short requests never wait behind a drain)

Branch ledger-short (docs/benchmarks.md, "Memory ledger: who runs next",
"Short requests never wait behind a drain").

- **The case**: the schematic trial's `drc-kicad-board` (1.5 s, 2253 MB)
  waited 657 s behind run.py pid 3598969 (`--min-free-gb 10`), whose four
  boards of 0.6-4.3 GB waited 2210-4371 s: 9.4-13.1 GB available beside
  the reservations' growth against 2.9 + 10 GB needed, with 25 GB of the
  ledger unreserved and about 19 GB of the machine outside the ledger.
- **The rule**: a new request of a short command (every command that
  reserves but a route and a layout; `PCB_MAKER_LEDGER_SHORT` 0/1
  overrides) of at most 2 GB of its own beside one KiCad check goes when
  the capacity has its room and the memory available less growth covers
  it, whatever the order and the drain say. Its reservation is marked
  `short` and counts as gone in the order's and the drain's sums for the
  others. ledger.rs (`short_request`, `short_allows`, `short_class`) and
  harness_common.py (`try_admit(short=True)`), the same rule.
- **Simulated** (ledger.rs and test_run.py, the same numbers): the
  check's wait 600 s before, 0 s now; other agents' checks up to 592 s,
  now 0; the waiting board starts at 608 s in both; a route arriving at
  30 s starts at 606 s before, 600 s now.
- **Smoke** (a private ledger of 8000 MB, a 4000 MB holder, a harness
  waiting for 4500 MB that the ledger drains for): x1846's
  `drc-kicad-board` waited until its 40 s timeout; the branch's held its
  2253 MB within 0.8 s and ended in 1.7 s (`PCB_MAKER_LEDGER_SHORT=0`:
  waited as x1846).
- **The floor**: `--min-free-gb` does not stack over harnesses (one
  MemAvailable, each harness's own floor), so sharing or scaling it by
  the number of harnesses changes nothing or only lowers it; a 6 GB
  floor would have started the board at once, with 6.5 GB left for the
  memory outside the ledger. Not changed: how far that memory moves was
  not measured.
- Routing is untouched.
## Committed on 2026-10-26 (a neutral spoke debug flag; the solid step per pad)

Branch spoke-debug-solidify (docs/router.md, "Determinism and the wall
clock" and "The solid step per pad"); snapshots x14940 to x14942.

- **`PCB_ROUTER_SPOKE_DEBUG` only prints.** Its lines asked the fill
  probe questions of their own (`blocker`, `fill_at`, `in_ring`,
  `in_hatch_hole`, the necks' pieces), whose items, queries and vertices
  were charged as pour work, and whose hatch holes and neck verdicts
  stayed cached for the probe's own later questions: the work decides
  budgets, so the route moved (F1's pour work 0.20 to 0.36 s; on x1844,
  route mode, 30 s: Miniboard 6 unconnected without the flag and 10 with
  it, 10 and 5 with `PCB_ROUTER_MEASURED_THERMALS=1`, ZSWatch's and
  brickpico's boards moved too). They run inside
  `FillProbe::quietly` now (counts put back, nothing cached, an index
  they build charged where the probe first needs it, the router's probe
  shapes not built for them). With x14940 and x14941 the three boards
  are byte for byte the same with and without the flag, with the exact
  model off and on, and the same as x1844 without it.
- **`PCB_ROUTER_FINGERPRINT`'s pour analysis before stitching is not
  charged either** (`Router::uncharged`; F1's pour work 0.200 to 0.205 s
  with the flag). Its fingerprint lines are unchanged; the nightly's
  fingerprints stage, which sets the flag, now routes as a run without
  it does.
- **The solid step per pad.** `set_pads_solid` sets solid only the pads
  of a label at KiCad's findings' points or the pour model's (the
  starved records' anchors, now in `predicted_starved_thermals` as
  `at`), the whole label only as the fallback when a point matches no
  pad. 0xCB's panel (the exact model on, task budget): 60 pads solid to
  1, KiCad's starved thermals 0 and 0, unconnected 53 and 53, findings
  327 and 327, the board otherwise byte for byte. On the other boards
  where the step fired (BloopBox, backplane, anthracite-bottom,
  LC_PAMV4b on the expansion tier; protoesp, video, DSP-ADAU1452,
  SNSP-CPU-01 dense; framework_mobo_lefthalf on the short tier; jetson
  and k30-SBC) every starved label is one pad: the same pads, boards byte
  for byte.
- **The expansion tier** (8 x 1, 30 s): x1844 against x14941 46 of 46
  boards byte for byte (557 unconnected, 0 new errors, 4263 vias both);
  x1845 against x14942 (this branch rebased onto the length rules)
  46 of 46 byte for byte too (557, 0, 4263).
- Tests: `a_fill_probe_counts_nothing_for_the_debug_output` (pcb-router),
  `debug_flags_change_neither_the_route_nor_its_work` (tests/fixtures.rs:
  F1 routed without flags and with each, board bytes and work compared),
  `the_solid_step_connects_only_the_starved_pads_of_a_label` and
  `kicads_findings_name_their_pads_by_label_and_point` (pcb-kicad).

## Reviewed on 2026-10-26 (a fresh agent's trial from the schematic)

Branch schematic-trial, x1844 (main 56ce9a4).
[reviews/2026-10-26-schematic-trial.md](reviews/2026-10-26-schematic-trial.md):
an agent new to pcb-maker took little-red-rover (2 layers, 65 parts, 328
pads, one sheet) from its `.kicad_sch` to a fab release with the user
documentation alone, and routed the designer's placement for comparison.

- **Result.** The layout from the schematic: 64/64, KiCad 0 unconnected,
  65 vias, 2468 mm in 160 s; 4 hole_clearance errors, J1's (USB4105) own
  pads against its own holes, which the designer's board has too, cleared
  by a custom rule for J1 and then released (21 files). The designer's
  placement routed: complete, 163 vias, 4404 mm (designer 170, 4011; with
  their GND pour, which the layout lacks). 18 commands in 25 minutes, 11
  of them one wait in the memory ledger.
- **Frictions that remain** (proposals in the review): the ledger still
  drains for a run.py waiter that does not start (657 s for a 1.5 s DRC);
  a footprint's own hole_clearance blocks layout and release without a
  reference; the decoupling action repeats after `decoupling_near` is
  turned on and names pins without a capacitor to give them; KiCad 10's
  renamed WSON-8 is reported missing, not substituted; the template put a
  bulk capacitor at an LDO's EN pin.
- **Fixed** (each its own commit): the decoupling action's example names
  the board's farthest pin (it was `U2:39` on every board); a DRC type
  whose findings lie within one footprint each says so instead of "move
  the parts apart"; docs on taking another board's outline from
  `describe-kicad-board`'s `points` and on `--refill-zones` for a direct
  kicad-cli check (24 unconnected without it on a complete board).

## Committed on 2026-10-26 (length and skew rules for single nets)

Branch length-rules (docs/router.md, "Length and skew rules
(2026-10-26)").

- **Census**: of the 109 boards of tasks v7 and the 46 of the expansion
  tier two have KiCad `length` or `skew` rules (oro-os link's
  `ULPI_exact_length`, 12 nets at 31.4 to 31.6 mm; Sisu_SSE-9's USB
  `skew` 0.5 mm on In1.Cu and In4.Cu over 11 nets); no project has a
  KiCad 10 tuning profile, the board setup's `tuning_pattern_settings`
  (106 projects) are the tuner's defaults, no rule. The designers' boards
  meet them; the designer tier x1832 had 12 length and 3 skew findings
  on them. The intake adds firestarter's compute board (10 nets, 25 to
  27 mm) and KiCad's QA boards.
- **Lowering** (`project_rules::length_rules`, `dru::track_rule`): per
  net and layer the rule KiCad applies to its tracks there, grouped by
  the rule; a via counts where the rule holds on F.Cu; rules rated below
  an error and bounds in `ps` are passed over. Reported `applied`
  (`skew`, a `length` minimum) or `approximated` (a `length` with a
  maximum: checked, not kept by the search) with `length_matching` on.
- **Meanders** (`lengths::match_lengths`, when each attempt is written,
  after the pairs): a completely routed net below a `length` minimum, or
  further than a `skew` maximum below its group's longest, gets bumps on
  its straight tracks (the bisected height, up to `max(1 mm, 3 w)`, at
  most 200 tracks, twelve places each, both sides), checked against
  every obstacle and route near them; their tracks are the route's,
  grouped as a KiCad tuning pattern (`tuning_mode "single"`). The bumps,
  the heights and the pattern are in board_router/meander.rs, shared
  with the pairs. Verdict and board-router.json: `lengths`.
- **Lengths as KiCad 10 counts them** (`kicad_length`, fitted on KiCad's
  QA board length_calculations): a via spans what it joins, its pads
  included; the outer layers sit at the board's faces, an inner one at
  its middle, a dielectric's sublayers summed (`copper_heights`, now
  shared: the pairs' skew changes on boards with such stackups); a
  track's way inside a pad it leaves once is the straight line from the
  pad's position. oro's routes that met 31.5 mm by the old heights were
  31.69 to 31.97 mm to KiCad (its F.Cu to In5.Cu via 1.215 mm, not
  0.98); now eight of the nine lengths KiCad reports on the 30 s routes
  agree to 0.1 µm (firestarter's Y9 by 0.024 mm).
- **Measured**, route mode, off x1840 against on x14861. 30 s of work,
  the five rule boards (the two of the census, firestarter, OpenAIO,
  the QA skew group): length/skew findings 31 -> 19 (oro 12 -> 5, the
  five left the unrouted ULPI nets; Sisu 4 -> 1; firestarter 10 -> 8),
  errors 47 -> 35, unconnected 482 -> 482, vias 509 -> 509; Sisu's
  diff_pair_gap 3 -> 4 (a meandered fallen-back pair). At the task budget
  (the designer tier, 1500 s, the two census boards): oro 12 -> 1 (11 of
  12 nets at 31.500 mm; ULPI_D4 32.94 mm, over the maximum), errors 12
  -> 1, unconnected 1 -> 1, vias 370 -> 370; Sisu skew 4 -> 1, errors 14
  -> 11, unconnected 17 -> 17, vias 281 -> 281. Rebased onto the
  pairs' return vias (x14863): Sisu (30 s and task budget), oro and the
  QA board byte-identical to x14861.
  The expansion tier (46 boards, 8 x 1): 46 of 46 routed boards
  byte-identical to x1840's, rows identical. **On by default**: the
  length and skew findings fall on the three boards whose rules are
  lowered (OpenAIO's are warnings, the QA board's class is undefined:
  both unchanged), unconnected and vias are unchanged, errors fall. Known costs: a meandered net of a
  fallen-back pair can leave its partner's gap (Sisu's diff_pair_gap +1
  at 30 s); the nightly's fingerprint stage routes Sisu, whose route
  gains its meanders (not a regression of the routes: the rest of the
  board is unchanged).

## Committed on 2026-10-26 (the exact thermal model while routing)

Branch exact-thermals-routing (docs/router.md, "The last miss, and the
exact model while routing").

- **The last miss, 0xCB's U3.23 on B.Cu**, two errors of the model that
  cancelled on one panel copy and not on the other. A spoke's end in a
  hatched zone's ring now keeps the clearances to a micron in the
  measurement (`SPOKE_END_RING_TOLERANCE`; the end lay 0.5 microns
  outside KiCad's fill, the model reached it at its 3 micron
  tolerance, kept the spoke as blocked and counted the pin's track over
  it). A spoke the clearances cut short of the fill's brush centres
  still counts where its remnant joins the fill beside it
  (`spoke_joins_fill`, a flood of brush centres; Board_2's diagonal
  spoke, nicked by a VCC track's clearance). Measurement only
  (`measured_thermals`). The model now matches KiCad on all four copies;
  the prediction check (the model alone on 294 written boards of pour3,
  pour4 and this A/B) changes no other prediction: 0 missed, 0 false.
- **The exact model while routing, measured** (x1840 off, x14870 with
  `PCB_ROUTER_MEASURED_THERMALS=1` on; route mode, fixed work rate):
  the expansion tier 557 to 564 unconnected, passes 24 to 22, errors 0
  and 0, vias 4263 to 4244 (Miniboard 6 to 10, facade 0 to 2, snaphat 0
  to 1; a second run of each side repeated byte for byte); the
  fingerprints identical; the dense set (task budgets) 1010 and 1010,
  errors 115 and 115, vias 7429 to 7431, and 0xCB's U3.45 made solid by
  KiCad's finding on one of its vias. **The routing default stays off**:
  the inexact model's conservatism (reliefs and spokes on every thermal
  via, spokes blocked where clearances only nick them) buys manual and
  inner spokes and open corridors the later repairs rely on.

## Committed on 2026-10-26 (differential pairs: runs not routed; return vias)

Branch differential-pairs-6 (rebased onto 5255222); snapshots x14810
(the diagnosis only: routes as main's x1835), x14811 to x14813 (steps),
x14814 (the branch as measured on the rule boards) and x14815 (x14814
rebased: the expansion tier). docs/router.md, "Differential pairs", "Runs
not routed at the task budget" and "Return vias", has the design and the
tables.

- **Why a run was not routed** (`not_routed` in the verdict's
  `diff_pairs`, the pair's `why` in board-router.json): the router keeps a
  record of its last hard repair (`Diagnostics::repair`: per net it took
  up, joined or not, in conflict or not, and why it skipped the net:
  hopeless attempt, work deadline, budget), from which each link not
  routed gets a cause: a launch with no legal node, no path in the
  negotiation, the hard repair found no room, no hard repair (the attempt
  judged hopeless), the repair's budget ran out.
- **The diagnosis at the task budget** (the intake's deep run: 300 s of
  work, wall guard 1200 s, route mode, one thread a board; the four small
  rule boards): 9 links not routed, 7 on oculink_port (6 because the
  attempt the ladder kept was judged hopeless and so had no hard repair,
  1 no room), 2 on OpenAIO-Whoop (1 no path, 1 no room). At 1200 s
  oculink_port still had 4 (its ladder ran out of rungs at 399 s of
  work): structure, not budget. The ducktop2 boards were not run at 300 s
  (16 GB each; the shared machine did not have it: pending, commands
  below).
- **The fix** (`resolve_remaining`): the hard repair takes the pairs'
  links (nets with "pair launch" terminals) first, and searches for them
  even on an attempt judged hopeless, within a repair's usual budget; the
  other nets as before, nothing changes on a board without links. At
  300 s on the four boards (x14810 -> x14814): links coupled 14/32 ->
  17/32, KiCad errors 106 -> 61, unconnected 163 -> 158, gap findings 55
  -> 38, dangling 37 -> 5, not routed 9 -> 2; oculink_port 1/9 -> 4/9
  coupled, errors 69 -> 17, unconnected 11 -> 7; worse: oculink_m2's gap
  findings 31 -> 37 (the repair's order moves its other nets; one
  connection fewer open).
- **Return vias** (`place_return_vias`; on with the pairs,
  `diff_pair_return_vias`, `diff_pair_return_net`,
  `PCB_ROUTER_DIFF_PAIRS_RETURN_VIAS=0`): beside each via of a kept link a
  via of the reference net (GND), its ring within 0.5 mm of the pair
  via's, where that net's pour reaches it on two layers (the fill's
  clearances and the thermal gap around its relieved pads) and every
  clearance holds (verify with the project's rules, hole to hole and hole
  clearance from the pairs' vias). Verdict: `pair_vias`, `return_vias`,
  `return_net`, per pair in `skews`. At 300 s (off -> on, both with the
  fix): 10 return vias beside 28 pair vias, errors 62 -> 61, unconnected
  158 -> 158, gap 39 -> 38, dangling 5 -> 5. **Kept, on.**
- **30 s, the 7 rule boards** (continuity with the tables before; x1835
  -> x14814): on the six boards with both sides (ducktop2-center's
  branch row pending: its 14.2 GB did not start in over an hour) every
  row as main's but the vias, 511 -> 520 (9 return vias beside 18 pair
  vias): links coupled 17/96, errors 82, unconnected 704, gap 57,
  dangling 39 both ways. At 30 s the links not routed are a matter of
  budget (left_io 26, right_io 11, Whoop 2: the attempt's work deadline
  had passed before the hard repair; oculink_m2 1: no room).
- **The expansion tier** (46 boards, route mode, 8 x 1, 30 s; main's
  x1842 against x14815, neither with a setting): every row identical and
  every routed board identical byte for byte (unconnected 557, passes 24
  both ways).
- Unit test `a_pair_via_gets_a_return_via_of_ground_beside_it` (a
  crossover's two vias, GND poured on F.Cu and B.Cu: return vias within
  the gap, clear of everything; without the pour none).
- Logs: `build/dp6-rules/x14810-300-small` (the diagnosis),
  `x1835-300-small`, `x14810-1200-oculink`, `x14811-300-small`,
  `x14812-300-small`, `x14812r0-300-small` (return vias off),
  `x14813-300-small`, `x14814-300-small`; 30 s: `x1835-30-{small,duck}`,
  `x14810-30-{small,duck}` (the causes), `x14814-30-{small,duck}`; the
  tier `build/dp6-exp-x1842`, `build/dp6-exp-x14815`. Pending, the
  ducktop2 boards at 300 s (one at a time, about 16 GB each):
  `benchmarks/agent-tasks/run.py build/dp6-rules/<snapshot>-300-duck
  --tasks build/dp6-rules/dp6-rules-tasks.json --only EwoudVV__ducktop2__ducktop2-center
  EwoudVV__ducktop2__left_io EwoudVV__ducktop2__right_io --binary
  build/bin/pcb-maker-<snapshot> --jobs 1 --threads 1 --budget-scale 10
  --timeout 600 --min-free-gb 10 --expected-gb 16`, with x14810 and
  x14814 (that task list: the seven boards' intake directories as
  route-mode tasks of 30 s, `designer_budget` false, router `seeds` 1,
  `skip_native_verification`, `deadline_seconds` 1200).

## Committed on 2026-10-26 (panel copies: place one, replicate)

Branch panel-copies, snapshots x14890 (the replication), x14891
(without fixed parts only net islands are a panel), x14892 (more than a
tenth of the replicated parts not legal: the board as a whole), x14893
(bodies grown to holes' clearance rings: dropped), x14894 (pads keep the
copper clearance from those rings). `panel_copies`, a placer switch,
**on by default**; placer.md "Panel copies (2026-10-26)" has the method
and the tables.

- **Detection**: copies by net islands (KiKit), numbered references
  (U201/U301; variants that share nine in ten of their parts) or
  repeated references (a fixed anchor per copy), the nets mapping one to
  one; each copy's transform (translation, quarter turn) from the fixed
  parts both hold, its region from its outline piece or the cuts between
  the copies' fixed parts. Found on the 0xCB pcb-panel (4 x 115, two
  turned half a turn), the 0xCB panel (4 x 53), SmartSpin2k (9 x 79),
  OpenRX (2 x 46 of its four sheets) and the five molecules; on none of
  the other 100 sweep boards, the OpenESC 4in1 panel included.
- **Placement**: the first copy with the ordinary placer in its region,
  replicated, the replicated parts that are not legal and the parts in
  no copy placed after it; more than a tenth not legal (the molecules),
  the board as a whole. `panel_copies` in board-placer.json and the
  verdict (count, parts per copy, how found, regions, the mapping).
- **Hole clearance** (`Component::hole_rings`): a non-plated hole's
  clearance ring is tested against other parts' pads at every level.
  OpenRX's C208 and C308 (one capacitor, both copies) sat 0.19 mm from a
  mouse bite's hole where the project asks 0.2: the ring reached past
  the bite's courtyard, and the spacing tests keep bodies apart. Growing
  the bodies instead changed four short-tier boards and lost two strict
  passes; the ring test changes none.
- **Measured** (layout, task budgets, x1842 against x14890/x14894): the
  four panels with copies 399 to 255 unconnected at seed 1 and 411 to
  267 at seed 101; the 0xCB panel 60 and 54 to 0, strict passes; the
  pcb-panel 93/78 to 43/28; SmartSpin2k 33/53 to 21/28 (vias 1646 to
  744); OpenRX 213/226 to 191/211, no new error. The 4in1 panel and the
  short tier (12 boards, x14894 with the switch on) byte for byte the
  same; the molecules still refused with the same parts without a
  place.

## Committed on 2026-10-26 (escape demand's fixed-share guard: the 45 boards)

Branch escape-guard-sweep. The 45 sweep boards `escape_demand.fixed_share`
0.5 fires on, layout mode at the task budget, seed 1, the defaults
against the guard (x1832; three boards x14720 = x1831), and
`fixed_share` 0.6 made the default; placer.md "The 45 fixed-heavy boards
(2026-10-26)" has the table.

- **katia** 162 to 38 at seed 1 (11 to 6 at seed 101): the loser the
  guard was for recovers.
- **Sums** over the 40 rows with a result on both sides (the five
  molecules are refused on both): at 0.5 unconnected 413 to 318, strict
  passes 28 to 27 (ghoul: one starved thermal; over three seeds both
  sides pass twice); at 0.6 413 to 321 with 28 to 28; at 0.7 413 to 420
  (katia, protoesp and ATAT1800 fall below it).
- **Losers**: ATAT1800 +22 at seed 1, -6 at seed 101; the 0xCB panel
  +10; the rest within 4.
- **`fixed_share` 0.6 by default** (35 of the 109 sweep boards): it
  meets the rule at the measured seed without counting ghoul's flip as
  noise; the dense and short tiers' fixed-heavy boards are all above 0.6,
  so their 0.5 measurement stands. The 64 boards below 0.5 are identical
  (MIDAS-MK1.1 and OpenFC placed byte for byte the same).

## Committed on 2026-10-26 (thermal vias of exposed pads in the pour model)

Branch thermal-vias (docs/router.md, "Thermal vias, custom pads and
necks"). The four starved pads the model predicted on the dense boards
and KiCad did not report were no via path: KiCad's DRC checks each
thermal pad by its own ring and tracks only.

- **Thermal vias by their own zone connection** (`merged_pad_thermal`):
  glasgow's U1.57 vias are `(zone_connect 2)` in a thermal exposed pad,
  no thermal pads to KiCad; such a merged pad has no spokes and no
  relief now (0xCB's U3.45 vias after the solid step too).
- **The DRC's track rule**: a blocked spoke's part over its pad is fill
  (video's U15.59, two tracks ending there), and a piece the lattice
  cuts from a kept one through a neck of brush centres between the
  nodes is kept (`neck_joined`: video's U21.7, a 0.26 mm strip; U21.8, a
  channel to its own track).
- **Custom pads' spokes in KiCad's frame** (the copper's box at
  orientation 0, the anchor's spoke angle): SNSP-CPU-01's P7.1.
- **For the measurement only** (router `measured_thermals`, on in
  `measured_board` and dump-pour-map, `PCB_ROUTER_MEASURED_THERMALS`):
  in routing the first three moved 7 of 61 routes (x14850; Miniboard 6
  to 10 unconnected), so routing keeps its model; `measure_pours` (the
  ladder's ranking) too.
- **Measured** (x1838 against x14853, route mode at a fixed work rate):
  the eleven dense boards at 30 s (3 x 2) 4 mismatches to 1 (0xCB's
  U3.23 on B.Cu, a tight spot on both), 11 of 11 routes byte-identical;
  the expansion tier (8 x 1) 0 and 0, 46 of 46 identical, the act step
  as before (BloopBox, backplane, anthracite-bottom, LC_PAMV4b; no
  exposed pad among them); the fingerprints (1 x 4) 0 and 0 (k30's J1.39 predicted), 4 of 4
  identical; pour3's full-budget
  dense routes (the model alone) 4 false alarms to 0 on each escape
  setting, its tier and fingerprint routes (100 boards) unchanged at 0.

## Committed on 2026-10-26 (the placer's legality against KiCad's)

Branch placer-legality (docs/placer.md, "The legality model against
KiCad (2026-10-26)").

- **`legality-kicad-board`** (a research command, agents.md): a
  placement judged by the placer's legality at every relaxation level,
  each illegal part's conflicts with their causes measured on the
  footprints' courtyards and pad copper, beside KiCad's DRC report of
  the same board. `place-kicad-board`'s preparation is now one function
  (`prepare_board`) both use, the levels one (`relaxation_levels`,
  `relax_level`, `pcb_placer::level_problems`); placements are
  byte-identical. `benchmarks/agent-tasks/legality_census.py` runs it
  on every task's designer placement.
- **The census** (109 boards, 16 470 movable parts at the designer's
  poses): 8 233 illegal at the least-spacing level without halos (the
  ablation's "tightest model"), 2 766 at the tightest; KiCad reports an
  error against what they meet for 220 (on 30 boards, 45 of whose
  designer boards have placement errors at all). The rest is the model
  being stricter: courtyards kept the copper clearance apart (5 622),
  courtyard overlap the project allows (1 044, kept: fabrication
  outlines), the board's silkscreen marks (698, kept), bodies past their
  courtyards (649 + 385), the edge margin by the copper's least inset
  (511), the far side, hollow boxes, parts without courtyard, obstacles.
  Eight of the eleven dense boards place at halo levels, which this does
  not touch (SNSP-CPU-01's wider spacing is its full halos).
- **`legality: "kicad"`** (placer setting, default `"model"`): at the
  levels of the rules' least spacing, courtyards may touch where the
  pads keep the copper clearance (or the mask's web) pad to pad, and an
  unheld part's body may come to the edge where its pads keep the
  margin. The census at the tightest level: 2 766 -> 1 861.

- **Measured** (eleven dense boards, seeds 1, 101, 201, x1828 against
  x14502): KiCad unconnected, sum of medians 967 -> 934, but seed 1 917
  -> 954 and seed 101 1059 -> 1148 (seed 201 1065 -> 983); seven boards
  unchanged (halo levels), the 4in1 125 -> 85, the 4in1-mini 82 -> 89,
  Glasgow 149 -> 142, video 190 -> 197 (medians); no new KiCad error, no
  cut row. Short tier: unconnected 22 -> 24 (MIDAS-MK1.1 6 -> 8),
  strict passes 7 -> 7. The default stays `"model"`: two of three pairs
  rise.

## Committed on 2026-10-26 (per-board memory estimates for every tier)

Branch tier-estimates. The layout sweep's task list carried no memory
estimates, so run.py declared one `--expected-gb` for every board (v21
ran in three parts to keep each declaration above the board's measured
peak) and `--order peak-desc` did nothing there; the designer tier's came
from older rows. Routing is untouched: only what run.py declares to the
memory ledger and the order the boards start in change
([benchmarks.md](benchmarks.md#per-board-estimates-for-every-tier-2026-10-26)).

- **The sidecar** `benchmarks/agent-tasks/estimates.json`: per board of
  tasks/v7 and the expansion list, its pads and copper layers and per
  kind (layout, short, schematic, designer, expansion) the expected peak
  (the largest `peak_rss_mb` of the rows of its newest three sources,
  times 1.25), seconds and the rows with their sources and times; a size
  model per family (pads x layers, at the 90th percentile of the measured
  peaks) for boards without a row. `tier_estimates.py --refresh` writes
  it from run.py logs; its older form (a tier file's own estimates)
  stays.
- **run.py** reads it by default (`--estimates`); a board falls back to
  its layout rows, the tier file's entry, the size model, then 3 GB;
  `--expected-gb` given overrides every board's; `--dry-run` prints the
  start order with each declaration and its source; every run prints the
  sources counted.
- **Coverage of the first file**: all 155 boards have a measured row of
  their kind (layout 109, designer 109, short 12, schematic 12, expansion
  46); the size model covers none of them. Dry runs: the sweep declares
  ULK 8982 MB (876 s) and spell_tome_bottom 644 MB (118 s), 109 boards
  from layout rows; the designer tier 109 from designer rows, the
  largest link's 7564 MB.
- **The nightly** declares every stage from its own copy
  (`build/nightly/estimates.json`, the committed file on the first
  night), refreshes it at the end of each night from the night's rows
  (`estimates.log`), and starts the sweep's largest boards first.
- **Tests**: test_tier_estimates.py (refresh, size model, fallbacks, the
  committed file covers every tier's boards), test_run.py `Estimates`
  (dry runs: loading, the size model on a board's file, the default,
  `--expected-gb`), test_nightly.py (the second night reads the first
  night's refresh). The ledger's simulations are unchanged.

## Committed on 2026-10-26 (bus-group moves in placement)

Branch bus-groups, snapshot x14560 (main 991ad37 and the moves). The
ablations found that the placer's own objective (wirelength plus 3 mm
per crossing) already prefers the designer's placement on video, the
4in1 and protoesp, and that single-part moves do not reach the orders
it prefers. `bus_groups` (placer option, off; `{}` turns it on; code
`crates/pcb-placer/src/groups.rs`, the anneal's `group_batch`,
`legal::refine_groups` and `legal::place_unit`; docs/placer.md
"Bus-group moves (2026-10-26)") moves units: banks (parts of one
footprint sharing 4 bus nets: video's eight memory modules, MegaDrive's
U2 U3 and U9 U10) and clusters (a part with its relation-bound and
nearby passives). After the global placement each bank takes its best
order and arrangement; the anneal shifts, turns, swaps and reorders the
groups each stage (a bank's members only with their bank); the
refinement does so legally, each unit placed whole; all judged by
wirelength plus their nets' crossings. The groups are reported in
board-placer.json and the verdict (`bus_groups`). With the switch off,
placements are byte-identical to x1828's (video, MegaDrive).

**Measured** (the eleven dense boards in layout mode at the task
budgets, seeds 1, 101, 201, off x1828 = the placer-legality round's
runs `build/legal1-dense-*-off`, on x14560 `build/group1-dense-*-on`;
the short tier one pair): the placed boards' objective falls on every
seed's sum (-8.5, -8.8, -12.0 %) and on 22 of 33 pairs, the crossings
10 to 21 %, video's objective 20 to 27 %. KiCad unconnected: summed
medians 967 to 960, pairs 917 to 986, 1059 to 988, 1065 to 871; video's
median 190 to 170, MegaDrive's 110 to 106; by the median 5 boards
better, 6 worse (Glasgow 149 to 165, SmartSpin2k 52 to 61). Short tier:
unconnected 22 to 25, strict passes 7 and 7. Placement work +10, -3,
-3 %.

**Judgement: off.** Seed 1's pair rises, so the rule (the dense sum of
medians and each pair's sum fall, no short-tier pass lost, video's and
MegaDrive's medians better) is not met. The finding: the objective
falls and routing does not follow it closely (14 of the 22 pairs whose
objective falls route better; Glasgow routes worse on two seeds with
an objective 9 to 16 % lower). Video's modules still do not stack in
one column and MegaDrive's QFP order is unchanged.
## Measured on 2026-10-26 (layout sweep v21, matched designer tier)

The full measurement on x1832 (main 2087046) and the README's numbers
moved to it: tasks v7, `--quality`, pinned work rates, 10 GB kept free,
`--ledger-priority nightly`, through the memory ledger without the lock
([benchmarks.md](benchmarks.md#layout-sweep-v21-2026-10-26)). Since v20
(x1725): x1753, x1754, ratsnest crossings by default, the race gate fix
(5 % pick margin), thermals solidified first, ordered admission, the
pad-clearance floor, per-layer widths and the connection-width floor,
diff pairs by rule, the panels fixes, the net tie's vias, the
default-losers switches and memory round 3. 14 boards gained
`.kicad_dru` files on 2026-10-24 (INPUT against v20).

- **Layout sweep v21** (`build/github-layout-v21.log`, 10 x 2; the task
  list has no memory estimates, so it ran in three parts by measured peak
  with an `--expected-gb` above each board's: 5 boards at 4 GB and 7 at
  8 GB side by side, merged into `build/github-layout-v21-ab.log`, then
  the other 97 at 3.5 GB spliced from it; 07:31 to 11:13, largest peak
  4.6 GB): **56 of 109 pass the strict rule** (56 per type), 2444
  unconnected in all, strict errors 44, no row cut or killed, five
  without a result (molecule panels). Against the nightly of 2026-10-25
  (x1817, same inputs): better 6, same 91, worse 3, 86 rows identical;
  on the 100 boards with a result in both 1788 -> 1672 open; strict pass
  gained by homebrew_486, neotron-pico and ghoul, none lost. Against v20
  on the 95 boards of the same input: better 20, same 59, worse 9;
  1651 -> 1561 open on the 88 with a result in both; strict 49 -> 53
  (gained hackrf-one, eternal-keypad, ghoul, neotron-pico; MokyaLora,
  INPUT, lost by clearance 3).
- **Results for four boards the nightly had none for**: the OpenESC
  4in1 panel (499 open), katia (162), the 0xCB pcb-panel (93; its DRC
  finishes now) and laptop power (18); the five molecule panels still
  refuse with fewer unplaced parts (3 to 45).
- **Medians against the designer's**: vias x0.72, length x1.04, open
  mean 0.124 (the new results' open counts), bends x1.06, stair-steps
  +63.5, crossings +0.5; no via in a pad on any of the 104 boards with
  a result.
- **Matched designer tier** (`build/designer-x1832-matched.log`, budget
  scale 0.762 = v21's median final-ladder share, 8 x 1, `--order
  peak-desc`, all 109 rows in one part, 11958 s, none cut or killed):
  62 strict passes (66 per type). **Gap over the 100 target boards**:
  layout open 1740, designer route 742, placement 57 %, routing 43 %
  (72 / 28 without Batch_1); over all 104 boards with both results
  51 / 49.
- **Placement costs most** on video (+189: 190 / 1), katia (+152: 162 /
  10), the 0xCB pcb-panel (+93: 93 / 0), the OpenESC 4in1 (+91: 106 /
  15) and Glasgow (+88: 143 / 55).
- **Worse than the nightly** (the next triage): RP2040_base 47 -> 50,
  the Hub 21 -> 22, woagboard 4 -> 5; none loses a pass. Larger: katia
  (162 open; v20 37) and laptop power (18; v20 1), which the nightly had
  no result for.
- **Since v21**: x1833 (exact escape stubs on by default, the pour model
  matching KiCad) moves routing, so v22 is expected to improve.

## Committed on 2026-10-26 (the ledger drains only for a waiter it could start)

Branch ledger-startable. Since the evening of 2026-10-25 the
2026-10-25 nightly's designer stage (pid 4134524, a harness copy of
356c8db that predates every ledger change) waited for 24.7 GB with its
own `--min-free-gb 10`. As the oldest waiter every newer request drained
for it: with the feasibility cap only requests of 4 GB or less went
(katia 7 GB, video 8 GB, ducktop2 16 GB waited for hours), and the
fresh-agent trial waited 3-7 minutes per run. Its floor and the other
runs' growth kept it from starting; the drain blocked everyone for a
request the ledger could not start. The process is left alone (only
Florian ends it); the rule is now immune to it. Routing is untouched:
only who is admitted when changes.

**The rule** (`drain_state` in ledger.rs and harness_common.py, the same
rule; docs/benchmarks.md, "Memory ledger: who runs next"). A waiter
counts for the drain only while, with every reservation that began
holding since its wait began removed, it would fit under its own floor
(`floor_mb`; 10 GB assumed for an older writer's entry without one) and
the reservations' growth; while its wait is within `drain_max` (run.py
`--ledger-drain-max`, default 7200 s); and while its entry is refreshed
(`refreshed`, new in the entries of harnesses and binaries; stale after
120 s; absent: assumed live). Otherwise newer requests pass it by
ordered admission alone and it keeps its place. `pcb-maker ledger`
names the first waiter and whether the ledger drains for it.

**Tests**: ledger.rs and test_run.py
`the_ledger_drains_only_for_a_waiter_it_could_start` (a 24.7 GB waiter
without a floor, 26 GB reserved, the smallest run 3 GB for another hour:
before, a 7 GB request was held; now it starts, at once when the waiter
cannot start, after 2 hours of drain when it could), the try_admit test
(stale, past the bound, an older holder), the earlier simulations
unchanged. **Smoke** on a private ledger with the 356c8db harness as the
waiter: x1834's route waited behind it ("the ledger drains for it"), the
branch's reserved at once. **Live** (read only): the branch's `pcb-maker
ledger` classifies pid 4134524's ULK_sl_PG1316s (waiting 97 967 s for
24 724 MB) as `not drained for: it has waited past the drain's bound of
7200 s`. Harnesses and binaries from before this change still drain for
it.

## Committed on 2026-10-26 (pours: the second zone and the keep_whole check; decoupling by default; starter constraints)

Branch pours-decoupling, snapshots x14740 to x14743 (x14741: the switch
on, measured against x1835; x14743: the head, rebased on d7159a3, its
default placements byte-identical to x1838's, the switch's to x14741's).
From the trial's frictions 2, 3, 4 and 7
([reviews/2026-10-26-fresh-agent-trial.md](reviews/2026-10-26-fresh-agent-trial.md)).

- **add_pours over a layer set.** A zone on `F&B.Cu`, `*.Cu` or a layer
  list counts as existing on each of its layers (board A wrote a second
  F.Cu GND zone over its F&B.Cu one: `zones_intersect`); test on a
  multi-layer zone.
- **The keep_whole check.** A `keep_whole` region over other nets' pads
  on its layer is warned of before the run, naming the pads (route mode:
  every pad inside; a layout: the named parts' own pads), and leads the
  verdict's next_actions. The pour advice names only parts whose region
  on the layer holds no other net's pad (`keep_whole_parts` in the pour
  report), with the `add_pours` entry; where none does it says keep_whole
  under parts does not fit the layer. Board A's trial step again: the
  warning (41 pads), no zones_intersect, the advice "does not fit F.Cu".
- **Decoupling.** `decoupling_near` (placer, off by default): each
  decoupling capacitor near the nearest of every supply pin of its rail at
  1 mm, and near relations followed through the legaliser (a follower
  legalised right after its anchor, moved as it moved, paying for the
  relation at each spot). Board B 14 % to 50 % of its capacitors within
  3 mm, board A 40 % to 80 %, every connection made; on the eleven dense
  boards (seeds 1, 101) the capacitor median falls on nine (summed
  medians 56.8 to 50.6 mm) and rises on the two OpenESC boards, KiCad's
  unconnected 974 to 987 summed medians (seed 1 +116, seed 101 -90: the
  noise); short tier 8 strict passes both, unconnected 20 to 18, the
  median rising on two of its six scored boards. Not every board's
  median falls, so by the agreed rule a switch, off (docs/placer.md,
  "Decoupling at the pins (2026-10-26)", the tables). A layout under 60 % of its capacitors
  within 3 mm of a pin gets a next action naming the farthest pins, the
  near constraint and the switch.
- **Starter constraints.** `describe-kicad-board --constraints-template
  <file>`: the plug-in connectors as `edge` entries (the side they lie at
  or face on a placed board, else `any`; `opening_outwards` where the
  footprint shows its mouth), each decoupling capacitor's `near` to the
  closest supply pin of its IC, `move_all`, and comments on what to
  decide. From the designer's boards it gives the trial's hand-written
  constraints (A: J7 left facing out, C4 at U2:10).
- **The placement line** counts parts: "placed 37 of 38 parts (1 kept
  where they are; 14 board marks and rule areas kept clear of)".

## Reviewed on 2026-10-26 (a fresh agent's trial)

Branch usability-trial, x1833 (main e935324).
[reviews/2026-10-26-fresh-agent-trial.md](reviews/2026-10-26-fresh-agent-trial.md):
an agent new to pcb-maker laid out two boards from the expansion intake
(lpc1311-target-v1, 2 layers, 194 pads; rp2350-dual-usb, 4 layers, 247
pads) from the user documentation alone, with a connector on an edge and
a capacitor at its IC, and without constraints.

- **Result.** Every layout and the designer's placements routed complete
  under KiCad (0 unconnected, 0 errors, kicad-cli agrees); A 15 vias and
  464 mm against the designer's 42 and 943, B 70 vias and 863 mm against
  55 and 575 for the designer's placement routed. About 40 commands, 24
  minutes, 7 of them in the memory ledger.
- **Frictions that remain** (proposals in the review): the ledger drained
  for a 26-hour-old request that its own room would admit and held every
  run 3 to 7 minutes; the pour advice's `keep_whole` under the ICs opened 22
  connections, and `add_pours` over a net's `F&B.Cu` zone wrote a second
  zone (`zones_intersect`); B's layout is complete with its decoupling
  capacitors a median 6.9 mm from their pins (designer 1.8 mm) and no next
  action says so; no starter constraints for a board without a schematic.
- **Fixed** (each its own commit): the skill's exit-1 paragraph (inspect
  first, room is a hypothesis), the layout's decoupling share printed as a
  percentage, the pour advice's warning about regions over other nets'
  pads, constraints.md on `opening_outwards` without a clear mouth and on
  `move_all` for a placed board (the start does not matter: the same poses
  from the designer's positions and from one stack).

## Committed on 2026-10-26 (the thermal-pad model matches KiCad's islands)

Branch thermal-islands (snapshots x14700 to x14703; x14703 measured). The fine-pitch escapes
left four starved thermals with `exact_escape_stubs` on (BloopBox J1.1,
backplane J4.A1, anthracite-bottom J107.1 and J107.4) and LC_PAMV4b's
J9.1 on main, all KiCad errors the router did not prevent.
docs/router.md, "The pour model against KiCad's islands (2026-10-26)".

- **Why.** The census of the tier's routed boards (x14482, both switch
  settings; the model on the board as written against KiCad's fill)
  found three of the five in the model already (spokes only into
  isolated islands; no manual spoke or island via could help, and the
  tier's route mode does not run KiCad, so nothing solidified them) and
  two missed: J4.A1 and J107.1 lie inside another net's zone of a
  higher priority, which KiCad 10 knocks out of the lower zone by its
  *fill* grown by the clearance, not by its outline, so the lower zone
  fills over its own via (J107.1) and into gaps the higher zone cannot
  fill (J4.A1). The chase's first guesses (a B.Cu track counted as an
  F.Cu spoke, spokes off the board's edge) were not it. The census also
  showed 23 and 26 false alarms a run: all pads on boards whose setup
  asks for one spoke (`min_resolved_spokes` 1) or ignores the finding.
- **The model** (pcb-router): reclaimed fill inside other nets' higher
  zones (router/reclaim.rs, `PCB_ROUTER_NO_RECLAIM`), the zones' island
  removal (`Plane::island_min_area` from `island_removal_mode`: KiCad
  keeps a piece holding a track or via of the net, and the DRC counts a
  pad's track ending over it), and the board's spoke minimum and the
  finding's severity (`min_resolved_spokes` in the router settings,
  router `min_spokes`, `SpokeRecord::required`).
- **The act** (pcb-kicad): the model's starved pads on the final board
  in board-router.json (`predicted_starved_thermals`, measured with the
  pours, no work); where KiCad does not check the board, those pads get
  the solid connection when the project allows it and the model then
  finds fewer (`solid_by_model`).
- **A spoke beside a plated hole**: a pad's track meets the part of a
  spoke the hole leaves (k30-SBC's C16.1, a fingerprint's false alarm).
- **Measured.** Census: on 5 of 5 (was 3, false alarms 23 to 0), off 1
  of 1 (false alarms 26 to 0), the fingerprints 1 of 1. Expansion tier
  (x14703 against x14482): the model's final starved count equals
  KiCad's on all 46 boards both ways; off 588 to 589 unconnected, 21
  passes, errors 1 to 0; on 556 to 557, passes 22 to 24, errors 5 to 0.
  Fingerprints identical to x1823 (off) and x14475 (on), the model's
  count equal to KiCad's on all four.
- **`exact_escape_stubs` on by default**: with the model fixed the tier
  meets the rule (off 589 / 21 / 0, on 557 / 24 / 0); the fingerprints
  43 to 42 unconnected, errors 20 and 20. Dense boards (3 x 2): 1018 to 1010
  unconnected, errors 115 and 115; the model predicts 4 starved thermal
  vias of exposed pads on inner layers there that KiCad does not report
  (glasgow, video; KiCad checks those boards, nothing acts on them).

## Committed on 2026-10-26 (memory round 3: the stamp lists, the search arrays, the occupancy)

Branch memory-round-3, snapshot x14553 against main's x1830 (a905905);
docs/profiling.md, "Memory", has the tables. Every route the same: the
routed boards' SHA-256, every fingerprint line and the verdicts apart from
work, wall and memory fields, on the four fingerprint boards, the
0xCB-1337 panel and SmartSpin2k_Panelized.

- **The profile** (`PCB_ROUTER_MEM_PROFILE=1`, mem_profile.rs): a
  sampler of the resident size every 10 ms, the live heap from a
  counting global allocator (the system's; one relaxed load an allocation
  while off), every live router's parts (`Router::memory_parts`) and the
  ladder's phases, as `mem-profile:` lines in the log, the peak at the
  end. Route mode, a quarter of the task budget, main's code: the peak of
  video (3396 MB), katia (5933), the 0xCB-1337 panel (1452) came in the
  rung probes, four routers side by side, SmartSpin2k_Panelized's (734)
  in a rung's via reduction (the router and its snapshot). The largest
  parts: the search scratches (21 to 34 %: each router's own and a spare
  per batch thread), the occupancy (7 to 25 %), the search records (8 to
  25 %), the stamp lists (5 to 25 %; video 843 MB). The KiCad parse is
  10 to 44 MB, the lowered boards 2 to 6 MB a router.
- **The stamp lists as differences** (`Stamped`): zigzag LEB128 varints
  of each stamp's difference from the one before, 1.5 bytes a stamp
  instead of 4, trimmed after a restamp; read in order only.
- **The search arrays shared by the process** (`SearchArrays`,
  `IDLE_ARRAYS`, `LatticeUser`): a net's search takes idle arrays of its
  lattice and gives them back after the net; until now a router held its
  own and a spare per batch thread for its life.
- **The occupancy in four bits a count** (`Occupancy`): two cells a
  byte, counts of 15 and more keep the rest in the side map (as 255 did);
  the router's maps shrink by half the occupancy.
- **Peaks** (route process, x1830 to x14553): the 0xCB-1337 panel 1452
  to 1120 MB (-22.9 %), SmartSpin2k_Panelized 745 to 650 (-12.8 %),
  Castor 1073 to 661 (-38.4 %), k30-SBC 1630 to 1000 (-38.7 %), jetson
  1500 to 1017 (-32.2 %), Sisu 2114 to 1508 (-28.7 %); every pair
  identical. katia and video are not measured after: the ledger held
  every request above 4 GB behind a stale 24 GB waiter (the commands are
  in profiling.md).
- **CPU** (indicative, k30-SBC at 60 s, six sibling-CPU pairs of x1829
  and x14551): +2.5 % (from -3.7 to +10.4 %), the stamping's share beside
  the search 3.7-4.0 % to 5.2-5.4 %.
- **Tier estimates** unchanged: the rows here are a quarter budget on two
  threads, the designer tier's estimates the nightly's budget on one
  (`tier_estimates.py` takes the largest row and its seconds as they
  are); the next nightly's designer.log with this binary gives them.
## Committed on 2026-10-26 (the defaults' per-board losers)

Branch default-losers. The triage of v20 (x1725, both placement defaults
off) against the nightly of 2026-10-25 (x1817, escape demand and ratsnest
crossings on), the four-way attribution of the eight largest losers on
x1825, the losers' traits, and one guard per default, both off by
default; placer.md "Per-board losers of the defaults (2026-10-25)" has
the tables.

- **Triage.** Leaving out the 14 boards with a new input
  (`.kicad_dru`), four boards are worse beyond their noise: katia
  (refused), OpenRX-panel-rev2 (200 to 248), protoesp (76 to 105) and
  homebrew_486 (its strict pass); video, poly_corne_split42_left, koeg
  and split72_right lose within it.
- **Attribution** (x1825, seed 1, defaults / escape off / crossings off /
  both off): katia 155 / 38 / 152 / 37 is escape demand's; OpenRX 231 /
  208 / 190 / 199 the pair's; video, split42_left and koeg lose to either
  default alone about as much as to both (perturbations of a good v20
  draw); protoesp (68) and homebrew_486 (strict pass) did not lose on
  x1825. Both off reproduces v20's placement.
- **Trait.** Escape demand does not help the boards whose footprints are
  mostly fixed (katia 0.63, the 0xCB panel 0.77, protoesp 0.63) and helps
  eight of the nine dense boards below half; channel overflow, layers,
  size, pad density, fine pitch and panels do not separate. For crossings
  no trait known before placement separates its losers.
- **`escape_demand.fixed_share`** (off; measured at 0.5: no escape
  demand where at least that share of the footprints is fixed), x14600
  against x1829: protoesp 110 / 76 / 90 to 73 / 45 / 58 over seeds 1,
  101, 201, the 0xCB panel 50 / 50 / 51 to 60 / 54 / 56, the dense summed
  medians -26, koeg 11 to 12, homebrew_486 passes on both, the short
  tier's four fixed-heavy boards pass on both. katia's row is pending (its
  layouts were held behind the nightly designer stage's 24 GB waiter);
  the guard fires on 45 of the 109 sweep boards. Not the default yet.
- **`ratsnest_crossings.per_layer`** (off: the weight times two over the
  routing layers), x14601 against x1829 at seed 1: loses
  framework_mobo_lefthalf's strict pass, OpenRX 200 to 212, 4in1 106 to
  129, video 190 to 162, DSP 17 to 7; not kept.
- Config reference regenerated for the two keys.

## Committed on 2026-10-26 (the drain's feasibility cap; the nightly's order)

Branch ledger-feasibility. On 2026-10-25 from about 22:30 to past 00:30
the nightly's designer stage had three boards left (neotron-pico 27.9 GB
expected, ULK_sl_PG1316s 24.7, ULK 23.2, `--min-free-gb 10`), five agents
were held back and the nightly did not progress. Routing is untouched:
only who is admitted when, and the designer tier's estimates and order,
change.

**What happened.** The nightly ran main 356c8db, built before the drain
landed (21:00): its waiting entries had no `floor_mb` and no `priority`.
Other agents' harnesses (with the drain) drained for it with a floor of
0 and the end exception, holding each other to 23-28 GB of room while
the waiter's own harness needed 33-38 GB available (about 42 GB on an
idle machine) or an empty ledger. The estimates were wrong too: the
designer tier's `peak_rss_mb` came from its boards' v17 layout rows
(x1170), not from routes on the designer's placement. Measured, the
largest of five designer runs (the nightlies of 2026-10-24 and
2026-10-25, x1725, x1539 and x1404 matched): neotron-pico 5497 MB, ULK
1512, ULK_sl_PG1316s 1389 (674-704 in the nightlies), katia 6550,
0xCB-1337 panel 1534, jiran-ble-lite 4917; the median estimate was 1.8
times the measured peak, the ULK boards' 34 times.

**The feasibility cap** (`blocked_by_large`, `SMALL_REQUEST_MB` 4096 in
ledger.rs and harness_common.py, the same rule). While the drained-for
waiter cannot fit before a run larger than 4096 MB ends (with every
smaller run gone its room, the capacity and the memory available less
its floor, still falls short), the drain lets requests of at most
4096 MB go and holds back the larger ones. Once no large run blocks it,
the drain holds everything as before. With tonight's (x1824) nightly the
three boards would each have needed the machine nearly to themselves,
and each next board's drain would have held every other request while
the one before it ran.

**Simulation** (ledger.rs `small_requests_go_while_the_waiter_waits_for_a_large_run_to_end`,
test_run.py `MemoryLedger.tonight`, the same numbers): a 58 066 MB
ledger, 42 000 MB available on an idle machine, an 8.8 GB run until
900 s, the three nightly boards waiting from 0 s (floor 10 240 MB, each
running 1500 s), three agents' chains of 2 GB binaries of 120 s. Before:
the boards start at 900, 2400 and 3900 s; a binary waits up to 3900 s;
264 binaries run in four hours. After: 960, 2460 and 3960 s; 120 s at
most; 348. A drain deadline (giving up after the waiter's run time) was
simulated as well and not taken: the second and third boards never
started in eight hours. A scratch simulation with five run.py agents of
1.2-6 GB boards beside it showed the same for binaries (longest wait
4036 s before, 180 s after); the agents' own boards still wait while a
28 GB board runs, by their own 10 GB floor.

**Smoke** (two processes and a phantom 6000 MB holder until 60 s, a
private ledger, `PCB_MAKER_LEDGER_TOTAL_MB` 12000): a nightly waiter of
8000 MB and a stream of 1000 MB requests every 2 s, each held 20 s. With
main's harness_common.py the stream got none in while the waiter waited
("the ledger drains for it"); with the branch's about ten went in between
3 and 60 s. The waiter started at 60.1 s in both (57 s of waiting).

**The estimates.** designer.json's `peak_rss_mb` and `seconds` are now
per board the largest of those five runs' designer-placement rows
(`benchmarks/agent-tasks/tier_estimates.py <tier> --placement designer
<logs>`, test test_tier_estimates.py). With them the largest designer
board expects 6.5 GB.

**The nightly's order.** scripts/nightly.sh runs the designer tier
before the sweep, with `run.py --order peak-desc` (new: the largest
expected peak first, then the longest; the default `seconds-desc` is the
tier's order as before). The designer tier is the stage with the large
boards and the shorter one (46 000 s of boards on 2026-10-25 against the
sweep's 91 000), and after the sweep it began at 06:27-10:29, in the
agents' day. Its budget scale now comes from the newest earlier night's
sweep (the matched share of the binary of the night before; 0.695
without one).
## Committed on 2026-10-25 (the race counts every probe it ran)

A fix of the race's accounting, with a margin for its pick. Branch
race-gate, snapshots x14370 (the gate removed) and x14371 (the margin
and shortened probes), off side x14343 (main c7b5552's code, the
default layouts of x1820); docs/placer.md "The race counts every probe
it ran (2026-10-25)" has the tables.

- **The gate is gone**: the race's probes ran side by side and a gate
  dropped, uncharged, every one judged past 40 % of the budget, so on
  the boards whose placement takes its share (Glasgow, 4in1, video's
  seed 201) the race kept the placer's pick. Now every probe that runs
  counts and is charged to the work clock. Where the probes would leave
  the final route less than 45 % of the budget (`RACE_MIN_FINAL_SHARE`)
  they are shortened alike, to a third at most, then fewer placements
  are probed (never on the dense boards; the short tier's MIDAS-MK1.1
  at 500 s probes three placements for 37 s each).
- **The pick needs a margin**: the probes' best routes on only where it
  leaves at least 5 % fewer nets unfinished than the first-ranked
  placement (`RACE_CONFIDENT_MARGIN`), else the first-ranked does.
  Calibrated on the 31 dense races: reverts video's seed 201 (504
  against 529, 159 to 270 with the probes' pick), keeps 4in1's two
  (125 to 82, 93 to 79).
- **Reported**: `race` in board-layout.json and the verdict whenever
  the race runs: each probe's standing, work and `gated` (the old rule
  would have dropped it), `kept`, `best`, `confident`, `reason`,
  `kept_gated`, `probe_seconds`, `dropped`.
- **Measured** (eleven dense boards, seeds 1, 101, 201): the gate fired
  in 7 races; the 23 seed-boards neither gated nor reverted (21 races,
  video's two seeds without one) are byte-identical to off. Pairs 933 to 891, 898 to 888, 1014 to 983; summed medians 920
  to 892 (-3.0 %). Without the margin (x14370): 1014 to 1111 on seed
  201, medians 920 to 932. Short tier 24 / 24 unconnected, 7 / 7 strict
  passes.

## Committed on 2026-10-25 (placement ablation: video, SNSP-CPU-01, 4in1, protoesp)

A diagnostic; nothing in the placer or the router changed. Branch
placement-ablation-2. Hybrids from x1820's tool, signals from x14392
(the branch), routes by x1820 (main 23f629e) in route mode. Our
placements are main's at the task budget, seed 1, crossings on. Tables
are in docs/placer.md, "Four more boards".

- **Tool**: `ablate-kicad-placement` adds weighted crossings (a line of
  an n-pin net weighs 1/(n-1)), crossings within a bus (two lines
  joining the same two parts), and pins whose row faces away from their
  net (count and mm). ablation.py adds the placer's pair (wirelength +
  3 mm per crossing) and describes the groups: faced gaps, turned
  parts, and the crossings and facing-away distance a group adds.
- **Gaps left** (route mode, designer's / ours, open connections):
  video 0.5 / 122, SNSP-CPU-01 32 / 85.5, the 4in1 14.3 / 158,
  protoesp 0.7 / 72.7. The costly groups:
  - the 4in1: every ESC copy, 34 to 78 per quadrant; the U9 cluster is
    48 on its own (passives 0.05 to 0.1 mm off its pin rows);
  - protoesp: the connectors (41.5) and the north-west (32);
  - video: the designer's memory bank in ours saves 40.
  On video and SNSP-CPU-01 the quadrant costs are global: the d side is
  large and the o side negative.
- **Signals**: crossings follow the costs best on the d side on all
  four boards (Spearman 1.00, 0.95, 0.89, 0.97) and order the
  placements right on three. On SNSP-CPU-01 they order them wrong (ours
  4286 against 5076, yet 85.5 open against 32). There wirelength and
  the new facing-away distance order it right, but the distance's
  Spearman over our own SNSP layouts is 0.03. The new candidates do not
  beat crossings.
- **Judgement**: no new signal. By wirelength + 3 mm per crossing the
  designer's placement is already 44 %, 26 % and 51 % cheaper on video,
  the 4in1 and protoesp, and cheaper than all 29 of our layouts there.
  The gap is the search reaching those orders (the anneal's crossing
  term, group moves of parts sharing a bus), not a missing signal.
  Partial: video's southern quadrants and SNSP's connectors and U13
  cluster were not routed.

## Committed on 2026-10-25 (layout profile round: the placer's turns, the near-any relations' anchors, the neighbour lookups)

Branch layout-profile, snapshot x14490 against main's x1824 (09299b6). A
CPU profile of `layout-kicad-board` beyond the router's search on
MegaDrive and Glasgow (task budgets) and the short tier's 1337-v4.0 and
OpenFC (a third), two threads, pinned rates: docs/profiling.md, "The
layout round". The placer is 30 to 61 % of a layout's CPU, the router's
search 35 to 59 %, the router's pour analyses 0.3 to 12.7 %, kicad-cli 0.3
to 3.8 % (1 to 5 DRCs a layout, none on a board checked before), the
lowering, the file's parse and the encoding below 0.1 %.

Three changes in the placer, each identical by construction (the
placement, the routed board, the fingerprints and the verdict unchanged on
all four boards, both orientations of each pair):

- **The turns without libm**: `sin_cos_degrees` gives the bits of
  `angle.to_radians().sin_cos()` with the quarter and eighth turns from a
  table filled once by the same expression; `rotation` answers the exact
  quarter turns without its division and `round`, `direction_of` without
  `fmod`; boxes and turned rectangles turn once a pose instead of once a
  box. (sincos was 2.2 to 7.9 % of a layout, `round` 1.3 to 3.2 %.)
- **A near-any relation's anchors** (`nearest_violation`): the least
  violation over a decoupling capacitor's supply pins stops at the first
  anchor within the tolerance and skips the hypot of an anchor whose gap
  along one axis already reaches the least (hypot was 6.4 % of Glasgow's
  layout, 12 % of its placement).
- **The neighbour lookups**: the bucket grid's query without a branch on
  seen items (the same items in the same order); the linked pairs looked
  up in each part's partner list instead of a `BTreeSet`.

Measured (two pairs a board on SMT siblings, x1824 against x14490, the
layout process's CPU seconds, geometric mean of the pairs): MegaDrive
2338 / 2321 to 2177 / 2183 (-6.4 %), Glasgow 2689 / 2522 to 2287 / 2167
(-14.5 %), 1337-v4.0 275 / 185 to 251 / 166 (-9.6 %), OpenFC 709 / 598 to
670 / 578 (-4.3 %). Wall: Glasgow -19.7 %, MegaDrive -2.8 %, the short
boards within the ledger's waits (+1.0, +0.4 %). The kicad-cli calls are
the same (no call removed or merged); KiCad's findings are the same by type
and count and in every violation but the unconnected items, whose named
track differs between kicad-cli runs on one board (three runs of x1824's
MegaDrive result gave three lists of the same 301 findings).

## Committed on 2026-10-25 (the seven sweep boards without a result)

Branch panel-refusals, snapshot x14421 (x14420 the same but for the
zones' order and the hints' wording, never measured); docs/placer.md "The
sweep boards without a result (2026-10-25)" has the diagnosis. The
nightly of 2026-10-25 (356c8db) had nine rows without a result, not the
seven of v20: katia and the laptop's power board were refused there
too, and still on x1820.

- **KiCad check repairs** (`check_repairs`, before every DRC with zones
  refilled): copper zones of two nets that overlap at one priority get
  distinct priorities in KiCad's own tie order (by uuid), Edge.Cuts lines
  of no length go. The 0xCB pcb-panel's check was a hang in KiCad
  10.0.7's filler (no CPU used after 15 s), not a slow check: two of its
  overlapping pours suffice; the designer's board hangs too. Its result
  board's check now takes about 110 s. The timeout rule is unchanged.
- **The legaliser tests legality before escape demand**: the same spots,
  less work (katia's passes 134 s to about 25 s).
- **The levels go on to the tight bodies (or, where courtyards may not
  overlap, the fine grid without spacing) when many parts lack room and
  the work left cannot pay for the levels before them** (`skip_to_tight`).
- **Refusals name movable parts once, a repeated reference with its
  uuid** (`unplaced_parts` in board-placer.json, with `held_by`), not the
  fixed parts and marks a part was left on.

Each board in layout mode at its task budget (tasks/v7, `--quality`, 2 x
2 through the ledger), x1820 (`build/panel1-before-x1820.log`) against
x14421 (`build/panel1-after-x14421.log`); unconnected and new errors are
KiCad's, work and wall the row's seconds (a refused row has no work):

| Board | x1820 | x14421 | work / wall s |
| --- | --- | --- | --- |
| OpenESC 4in1-panel | 34 parts without a place | 499 unconnected, 0 new errors (designer's own board: 130) | 1479 / 1368 |
| katia | 10 parts without a place | 155 unconnected, 0 new errors (v20: 37) | 1756 / 2415 |
| laptop power | 27 parts without a place | 16 unconnected, 0 new errors (v20: 1) | 1381 / 2049 |
| 0xCB pcb-panel | kicad-cli timed out (900 s) | 93 unconnected, 0 new errors | 1316 / 3101 |
| molecule 4x4 | 48 without a place (fixed parts and marks among them) | 29, by uuid | - / 995 |
| molecule 4x5 | 60 | 45 | - / 1728 |
| molecule 5x4_wave | 22 | 5 | - / 798 |
| molecule 5x2_shifted | 42 | 3 | - / 506 |
| molecule 7x5_wave_left | 48 | 9 | - / 1188 |

No board passes (strict or per type) on either side; four rows with a
result instead of none. Not solved: the 4in1 panel's two boards share
their net names (so its copies cannot be kept apart and KiCad wants the
boards joined across the tabs; 43 of 184 connections routed on tight
bodies), katia routes worse than before its refusals (348 open after the
first route against v20's 169: its placement with escape demand), and
the molecules stay refused (their designer's placement is illegal in the
placer's loosest model: 128 parts of the 4x4).

Short tier (3 x 2), x1820 against x14421: 11 of 12 boards give
byte-identical placed and result boards (only their placement work is
less, 0.3-8 s); MIDAS-MK1.1, whose placement runs into its budget, places
differently (wirelength 2495 to 2448 mm) and ends 8 to 9 open, 253 to
245 vias. Totals: unconnected 24 to 25, strict passes 7 to 7, new errors
0 to 0, CPU 4964 to 4220 s (unpaired, loaded machine).

## Committed on 2026-10-25 (the ledger drains for a starved waiter)

Branch ledger-drain. Ordered admission (2026-10-24) let a new request pass
the first waiter whenever the waiter would still fit beside every
reservation made since it began waiting; a waiter held back by its
harness's floor (`waits_on_floor`) was passed by the capacity alone. On
2026-10-25 five agents' stream of 1-8 GB routes kept the running set from
ever draining, and the nightly's last designer boards (katia 19 GB
expected, the others 6-27 GB, `--min-free-gb 10`) waited from about 11:00
to past 20:00 (build/nightly/2026-10-25/designer.err: "katia started after
waiting 33034 s"; five rows still waiting at 20:00 with 20-33 GB
available). Routing is untouched: only who is admitted when changes.

**The drain** (`drained_for`, `drain_allows` in
crates/pcb-kicad/src/ledger.rs and benchmarks/harness_common.py, the same
rule). Once the first waiter has waited longer than its bound
(`drain_after` in its entry; run.py `--ledger-drain-after`, default
600 s), a new request goes only if after it the waiter's room still
covers what it waits for (the capacity less every other reservation, and
the memory available less what the reservations still grow by and less
the waiter's floor `floor_mb`), or if it is expected to end no later than
every run that holds memory (`expected_end`, which run.py's tier boards
carry from the tier's seconds; binaries have none and go by the room
alone). Runs that hold memory and grow, and binaries a harness admitted,
are not held back. `--lock` works as before.

**The nightly's priority.** run.py `--ledger-priority nightly` marks its
requests `priority: "nightly"`: first among the waiters, drained for at
once and without the end exception, so never passed once waiting.
scripts/nightly.sh sets it on every stage; `pcb-maker ledger` marks such
entries `[nightly]`.

**Compatibility.** The new fields are optional; older binaries and
harness copies ignore them (an older binary that rewrites the ledger
drops them until the harness's next poll rewrites its waiting entry). A
binary of this change keeps unknown fields when it rewrites the ledger
(`Reservation::other`) and carries a harness reservation's
`expected_end` and `priority` into its own entry.

**Tests.** A second-by-second simulation of that day in both languages
(ledger.rs `a_stream_of_small_requests_starves_a_waiter_until_the_ledger_drains`,
test_run.py `MemoryLedger` likewise): a 56 GB ledger, 30 GB available, a
19 GB waiter with a 10 GB floor arriving at 300 s and looking every 15 s,
a 2 GB request every 5 s that runs 60 s. Under the rule of 2026-10-24 the
waiter does not start in four hours (2939 small runs); with the drain it
starts 660 s after it began waiting (600 s bound + the 60 s the last
admitted runs take), 179 small runs before it. Unit tests of the room and
end clauses, the nightly's order and drain, the fields surviving a
rewrite, and a real-ledger test that a request the order admits waits in
the drain ("the ledger drains for it") and goes under the bound.

**Smoke** (two processes, a private ledger, `PCB_MAKER_LEDGER_TOTAL_MB`
200000 so that only the floor binds, phantom reservations): a stream
keeping as many 3 GB holders as admitted (a request every 2 s, each held
30 s, no floor) and from 20 s a 10 GB waiter with a 2 GB floor. With
main's harness_common.py the waiter gave up after 180 s without starting
(78 holders admitted beside it); with the branch's and
`drain_after` 60 s it started after waiting 75 s (8 holders left, 15
newer requests waiting).

## Committed on 2026-10-25 (fine-pitch escapes)

The remapper trial's routing hypothesis (tight escapes at U3/U103),
tested and fixed generally. Branch fine-pitch-escapes, snapshots x14470
to x14477 (x14475 the change with the switch on, x14477 the head, off by
default); off side main's x1823. docs/router.md, "Fine-pitch escapes
(2026-10-25)", has the mechanism and the tables.

- **Diagnosis** (`PCB_ROUTER_DEBUG`, jfedor2/hid-remapper, 30 s, 0.1 mm
  lattice aligned with the QFN rows): all 27 open nets end the
  negotiation with their conflict spots on their own fine-pitch pads
  (U3, U103, U5, U105, USB101) against the neighbouring pads' nets. Every
  pad of a 0.4 mm row with 0.2/0.2 leaves by an escape stub (the centre
  line fits only exactly, which the static maps refuse), and the stubs
  stamped the lattice's discs grown by the snapping error (0.525 mm), so
  neighbouring stubs 0.4 or 0.5 mm apart were in conflict for good; the
  hard repair's test refused them too. Neither the lattice's phase nor
  the neck-down nor the order.
- **`exact_escape_stubs`** (router and KiCad config,
  `PCB_ROUTER_EXACT_ESCAPES`; off by default): a stub along a lattice
  line stamps and is tested as lattice nodes; an oblique or bent one
  stamps its measured footprint (`escapes::stub_footprint`); a pad leaves
  by straight stubs along a lattice line where it has any. Off is
  identical to x1823 (the remapper, the 22 tier boards that move on).
- **Measured** (on against off): the remapper at 30 s 33 unconnected to
  0, 120/120, no new error, a pass (at 300 s 19 to 0 in 67 s of work).
  Expansion tier 588 to 556 unconnected, passes 20 to 21, new errors 3 to
  8 (four starved thermals at connector PTH pads, a via at pdlc's net-tie
  copper, all with routes moved elsewhere), better 10 / worse 3 (bms
  loses its pass to the probe ranking, epdInky 2 to 4, backplane 1 to
  2). Fingerprints 43 to 42 (jetson 20 to 13, Sisu 23 to 29), errors and
  passes the same. Dense boards 1020 to 1012 (split72 25 to 9, DSP 25 to
  33, nine identical), errors 115 and 115. **Default off**: the tier has
  more errors.
- **The new errors chased** (2026-10-26): a net tie's whole bridge keeps
  every via out (lowering, on by default: pdlc's error and FUEL4EP's two
  go; the tier with the switch off 588 unconnected, 21 passes, 1 error;
  on 556, 22, 5). The four starved thermals left are pour-model misses
  at connector through-hole pads (router.md, "Fine-pitch escapes"). A
  longer thermal guard was measured and not kept: it moved starved
  thermals and cost connections (off 588 to 606).

## Committed on 2026-10-25 (the remapper trial's items: run-aware diagnosis, override preflight, arc outlines, hot-spot links)

The four items of the remapper diagnostic trial
([reviews/2026-10-18-remapper-agent-diagnostic-trial.md](reviews/2026-10-18-remapper-agent-diagnostic-trial.md),
its new "Taken" section) that 2026-10-23 left open or had not checked on
the trial's own board. Branch remapper-items, binary x14440 (x1821 the
main it is measured against). Nothing changes in routing: the router's
hot spots gain a field, the rest reads outputs.

**1. Run-aware diagnosis on the remapper board** (run_diagnosis.rs).
inspect-routing-run on the trial's deep run (its board, KiCad's
`native-deep.json` as drc.json, the designer's board as reference)
answers the trial's acceptance check: QSPI_SCLK's two groups are U3:52 at
(138.9, 130.2625) and U5:6 at (136.25, 125.525), both F.Cu with the
trial's uuids, 0.2 mm track and clearance from the Default class and the
board's 0.2 mm minimum with its source, KiCad's one pair joined to both
groups, the designer's route 7 segments of 0.2 mm on F.Cu, no vias; the
counts side by side: the router's 19 nets and 20 unreached terminals, 39
groups and 20 missing links, KiCad's 20 pairs and one starved thermal.
Two gaps closed: a KiCad item that is a track, an arc or a via is joined
to the group of the copper it lies on (a probe terminal on that copper,
`LoweredBoard::item_components`; on the deep run 5 such items (tracks), all
joined, none `null`), and a reference net whose tracks leave several
groups but which has a zone is `connected: null` with
`connected_reason` and `pour_layers` (the deep run's +1V1, +1V2 and +3V3
had read `false` while the designer's board is clean). The JSON of
QSPI_SCLK and +1V1 is in agents.md.

**2. Override preflight, the verdict's side** (verdict.rs,
run_diagnosis.rs `override_outcomes`). The narrow-four config is refused
before any work (exit 2, 0.01 s) with the four nets and the
min_track_width source (since 2026-10-23, checked on the trial's config).
New: with `allow_below_project_minimum` the verdict's `rule_overrides`
lists per value `requested_mm`, `project_minimum_mm` and its source,
`routed_with_mm`, `emitted_track_widths_mm` and `emitted_below_minimum`
(the result board's tracks on its nets), `native_required_mm` (the
project's minimum: describe-kicad-board keeps it) and `native_findings`
(KiCad's track_width or clearance findings on its nets, `null` unchecked).
On the trial's recorded narrow-four output inspect-routing-run shows
`class_track_width_mm` 0.15 beside the project's 0.2 and 0.15 emitted.

**3. Outlines with arcs.** The two commands the trial ran were removed
on 2026-10-21; they (and inspect-kicad-route-obstructions) now exit 2
with one line naming `inspect-routing-run --terminal REF:PAD` and the
outlines it reads, not "unknown command" (main.rs, agent_cli.rs
`removed`). describe-kicad-board, score-kicad-board, inspect-kicad-board
and inspect-kicad-connections run on the remapper's eight lines and eight
arcs.

**4. Hot spots with object links** (pcb-router HotSpot `half_size`;
board_router/result.rs `hot_spot_links`). Each hot spot in
board-router.json carries its tile's `box`, the pads in it (ref, number,
net, at; twelve nearest the centre, `pads_in_tile` all), the nets whose
copper crosses it on its layer and `conflicted_nets`, as the board stood
when the router stopped. inspect-routing-run lists them at the top
(`hot_spots`, per net `hot_spots_near` by index) and reads them off the
result board for older runs (`links_from`). The deep run's hottest tile
is the 1.2 mm above U3:50-52 with +1V1, QSPI_SCLK and QSPI_SD3 in
conflict; a fresh 60 s route of the board with x14440 records the tile
over U103:52-56 with the 1QSPI nets. "Too dense for its layer count" is
gone from agents.md and SKILL.md; the verdict's open-connections action
no longer says "make room there (move parts apart, enlarge the outline,
add layers)": it asks for the inspection first and calls room, layers
and narrower classes hypotheses where the designer's route does not rule
density out.

Tests: hot-spot links on a two-part board (pads, nets on each layer,
conflicts, an empty tile), override outcomes (widths, below-minimum
count, findings by kind, unchecked), KiCad item layers; the CLI tests
join the via of the inspection test to its group, check the verdict's
`rule_overrides` (with --strip, so the class routes) and the removed
commands' message.

**Measured.** The expansion tier (46 boards, route mode, 30 s of work,
8 jobs x 1 thread, `--min-free-gb 10`; build/remap1-exp-x1821,
build/remap1-exp-x14440): every row identical on every field but the
timings and `outcome` (a wall-clock class: 7 rows moved between
fast/medium/slow or early/at budget under the load), no row cut; all 92
boards (46 stripped, 46 routed) byte for byte; 20 / 46 pass on both,
work 1250.5 s on both, KiCad's unconnected items 588 on both. The router
fingerprint test passes unchanged. Not measured: CPU (the diagnostics run
once per routed board after routing, ten tiles).

## Committed on 2026-10-25 (router profile round: the hop test, the heap's root, the neighbourhood prefetch)

A speed package with identical results. Branch router-profile, snapshots
x14380 (the first version) and x14381 (the measured one) against main's
x1820 (23f629e); [profiling.md](profiling.md) has how to profile, the
tables and the measurements.

- **The profile** (perf 7.2 run from its Arch package as the user; route
  mode, two pinned threads; the fingerprints at a quarter budget and
  SmartSpin2k and protoesp at the task budget): the search is 76 to 90 %
  of the CPU, its heuristic 4 to 7 % more. Inside it the heap's sift 15
  to 18 % (one load, the children of a level, 8 to 11 %), the neighbour
  steps and their occupancy bytes 18 to 26 %, the hop test 3 to 9 %, the
  via steps 3 to 6 %. The tile planner 1 to 6 %, stamping 1 to 5 %,
  hashing under 1 %, allocation at most 0.11 %, the corridor
  instrumentation under 0.05 %.
- **The hop test** walks back only where a via can lie within its reach
  (`SearchNode::since_via`, the struct's padding byte: steps since the
  last via or source on the node's path), by coordinates and with its
  reach once a search.
- **The heap** leaves the root a pop empties for the next push to fill
  (the neighbours' keys stop near the root, where the last key sank to the
  bottom), and its sift prefetches the next level.
- **The expansion** prefetches the rows below and above (search nodes,
  packed maps, occupancy) and its cell on the other layers.
- **Measured**: x1820 against x14381 side by side on SMT siblings, two
  pairs a board with the CPUs swapped, pinned rates: CPU -10.7 % Castor,
  -11.1 % k30-SBC, -9.4 % jetson, -9.0 % Sisu (300 s of work), -12.4 %
  SmartSpin2k, -10.6 % protoesp (1200 s); expansions per CPU second
  0.67-2.43 M before, 0.74-2.82 M after. Identical in every pair: the
  routed board's SHA-256, the fingerprints, expansions, searches and the
  verdict apart from its seconds; the fingerprints equal the nightly's of
  2026-10-25 at two and four threads.
- **The work clock**: at pinned rates (the tiers) nothing changes but
  the CPU. The measured rate (the default outside the tiers) follows the
  canned search, which is the router's: x14381's canned search is 1.12 to
  1.16 times x1820's, so an unpinned run spends about 15 % more
  expansions per budget second and its result changes. Not recalibrated
  (`REFERENCE_CANNED_EXPANSIONS_PER_SECOND` stays 6.9e6).

## Committed on 2026-10-25 (differential pairs: polarity and skew)

Branch differential-pairs-5 (rebased onto 3786a9e); snapshots x14400
(crossovers on by default, the code measured for polarity) and x14401
(the branch: crossovers off, meanders on); main is x1820. docs/router.md,
"Differential pairs", "Polarity", "Skew and length" and "Measured
(2026-10-25)" have the design and the tables.

- **Polarity: a crossover in the lead** (`crossover`; off by default,
  `PCB_ROUTER_DIFF_PAIRS_CROSSOVER=1`): at a crossed end the stubs' ends
  keep the pads' order, one net runs straight on, the other dives under
  it with a via on each side (on the nearest other layer of the pair's)
  and comes up two offsets beside it, the run starting past it; every
  track on the launch layer a gap from the other net (KiCad's gap check).
  With it the links' paths lose their hairpins (`despiked`) and the kept
  links' vias keep their holes' clearance in the fallback. No swap at a
  part: KiCad has no swap data and no rule board has a pin-swaps.json.
- **Measured** (the 7 rule boards, route mode, 30 s of work, one thread,
  KiCad's DRC with `--refill-zones`, no setting; the wall guard raised to
  1200 s on both sides, since at 90 s it cut the ducktop2 rows on a loaded
  machine): x1820 -> x14400, links coupled 20/150 -> 29/150 (12 through a
  crossover), oculink_m2 9/10 coupled and 1 unconnected instead of 4/10
  and 9, but errors 97 -> 140, unconnected 1 227 -> 1 238, gap findings 67
  -> 106 (104 of them on pairs that fall back); with the router's seed 2
  errors 110 -> 296, unconnected 1 228 -> 1 231; right_io's unconnected
  worse at the ladder's budgets 28, 30 and 32 s. Fallbacks: run not
  routed 83 -> 78, stub does not reach the run 25 -> 18, fewer than two
  pair points 22 -> 22, launches too close 11 -> 12, no launch 7 -> 7,
  copper refused 4 -> 6. **Not the default.**
- **Skew and length** (`match_skew`, on with the pairs;
  `PCB_ROUTER_DIFF_PAIRS_SKEW=0` turns the meanders off): per pair with a
  kept link the two nets' lengths from their copper (tracks, vias at the
  stackup's height, `copper_heights`); further apart than the custom
  rules' `skew` maximum (else the pair's gap), or shorter than a `length`
  minimum, the shorter net gets a meander on a straight track of its
  coupled run (bumps away from its partner, the height bisected to add
  exactly what it lacks, checked against everything routed), written as a
  KiCad tuning pattern, whose tracks KiCad 10's gap check skips. The
  rules' `skew` and `length` are read (`KiCadDiffPairRule::max_skew`,
  `length`) and reported `applied`. Verdict: `skew_mm`, `meanders`,
  `skews`; board-router.json per pair `skew_mm`, `skew_target_mm`,
  `meander_mm`. x1820 -> x14401: every row identical (errors 97,
  unconnected 1 227, gap 67, skew/length 1, coupled 20/150); eight pairs
  matched by a meander (0.21 to 2.05 mm) to 0.000 mm; the largest skew of
  a coupled pair 0.562 mm (right_io's GBE_REFCLK, no straight track with
  room), 0.007 mm elsewhere. **Kept.**
- **The expansion tier** (46 boards, route mode, 8 x 1, 30 s, x1820
  against x14401, neither with a setting): every row identical (routed,
  vias, length, KiCad's findings, pass, open, strict findings) and every
  routed board identical byte for byte, the 25 boards with pairs among
  them; the default fires on none. Unconnected 588, passes 20 both ways.
- Unit tests `a_crossed_end_crosses_over_in_its_lead` (off by default; on:
  a crossover of two vias and a B.Cu track, the stubs uncrossed, the link
  coupled, the meander) and
  `the_skew_counts_tracks_and_vias_at_the_stackups_height`.
- Logs: `build/dp5-rules/a-x1820-{s0,s1,s2,b28,b32}`,
  `a-x14400x-{s0,s2,b28,b32}` (crossovers, no meanders), `a-x14400-s0`,
  `a-x14400k-s0` (crossovers off), `f-x14401-s0`; the tier
  `build/dp5-exp-x1820`, `build/dp5-exp-x14401`.

## Committed on 2026-10-25 (the race picks its seed by a route probe)

An experiment, off by default. Branch race-probe, snapshots x14350
(the probe) and x14351 (its `repair` variant), off side x14343 (code
of main c7b5552, x1819); docs/placer.md "The race's route probe
(2026-10-25)" has the tables.

- **The ceiling** (existing runs, x1818, eleven dense boards, three
  layout seeds): summed medians 926, the best of three seeds 812
  (-12 %). The race already probed (75 s, fewest unfinished nets) its
  three best-ranked placements, but where the placement takes its 40 %
  of the budget (Glasgow, 4in1, video) the probes after the first ran
  and were dropped. Across runs the kept placement's probe picks the
  best run on 6 of 11 boards, the first route's open terminals on 7
  (regret 79 against 18 connections).
- **`race_probe`** (layout option, `{}` for `{"seeds": 3, "share":
  0.08}`): the race probes its `seeds` best-ranked placements for
  `share` of `total_seconds` each, every probe charged to the work
  clock (no 40 % gate), ranks them by the probe's open terminals
  (`Router::probe_standing`: unconnected terminals plus complete nets
  in conflict plus stranded pour pads; then unfinished nets; then
  conflicts) and routes on with the best. board-layout.json and the
  verdict carry `race` (each probed placement's standing and work, the
  kept one). `repair: true` makes each probe a whole first route within
  its share.
- **Measured**: seeds 1 and 101, summed medians 915.5 to 872 (pairs 933
  to 872, 898 to 872), about one standard deviation of the seed noise;
  seed 201 1014 to 1151 (video 159 to 302, Glasgow 157 to 182), over the
  three seeds 920 to 924. Short tier unchanged (24 / 24 unconnected,
  7 / 7 strict passes). `repair` (seed 101): 898 to 966. Two probes at
  10 % on 4in1 and protoesp: no better than three. The probe kept the
  best known placement on 10 of 16 seed-boards where two finals are
  known. **Default: off.**

## Committed on 2026-10-25 (the crossings term's work booking)

A correctness fix of the work clock. Branch crossings-booking, snapshots
x14340-x14343 (x14343 the measured one), off side x1818 (main ab86b54);
docs/placer.md "The crossings' work booking (2026-10-25)" has the tables.

- **Measured** (`PCB_PLACER_CROSSINGS_PROFILE`, new, with
  `PCB_PLACER_DEBUG`: per phase the count's wall time against its booked
  work and the rest of the placement's; `ops` times each operation):
  the ratsnest-crossings count took 1.5 to 3.7 times the CPU per booked
  unit of the rest of the placement (legaliser: MegaDrive 3.0-3.1,
  Glasgow 2.2-2.6, video 3.4-3.7, split72_left 1.7-1.8; refinement 2.5-2.6,
  1.5-1.8, 3.0-3.6, 1.5-1.7). The grid's upkeep (inserting and removing
  lines) was not booked at all, and a line met cost about 2.5 units.
- **The count books per operation**: 5/2 a line met, 7/2 a cell walked
  to insert or remove a line, 1/4 a list entry scanned (the rest as
  before). After: 0.6 to 1.2 by board and phase. The count itself is
  unchanged (exact). The anneal's term (off) books 8/5 of the new
  booking instead of 13/5 of the old (1.60 and 1.57 measured).
- **Layouts** (eleven dense boards, seeds 1, 101, 201): 31 of the 33
  placements byte-identical; KiCad unconnected, summed medians 926 to
  920, the pairs 947 to 933, 898 to 898, 1032 to 1014. The placement's
  booked work +2.4 %; its wall clock per second of work 0.438 to 0.393
  on the pair run side by side (seed 201). Short tier: unconnected 26 to
  24, strict passes 7 and 7. No weight adjustment needed.

## Committed on 2026-10-25 (ratsnest crossings in the anneal)

Where the order of parts is decided. Branch crossings-anneal, snapshot
x14312 (the sub-switches off by default), off side x1817 (main
356c8db); docs/placer.md "Where the order is decided: the anneal and
the global placement (2026-10-25)" has the tables.

- **`ratsnest_crossings.anneal`** (off): the anneal's moves pay the
  crossings change of the moved parts' nets at the same 3 mm, signed,
  counting the nets of up to 16 pins, until the term's work reaches
  half the anneal's own. The count works in three steps for the
  anneal's 1 to 3 million moves (the kept lines first, a bound that
  skips the new trees when the move is rejected anyway, then the new
  trees, then keep or drop); half the swaps trade a part with a peer
  (same footprint, or both with 16 pins or more and at least half the
  other's pins). The term's work is booked at 13/5 of what the count
  books, which brings the anneal's booked work in line with its CPU.
  The anneal's work a move: MegaDrive 7101-7367 off, 10309-10560 on;
  Glasgow 3818-3861, 4767-4971; video 2779-2857, 4104-4316;
  split72_left 3619-3622, 4368-4431. Every net to the end cost split72
  14 times the anneal's work and two unplaced parts; every stage cost
  video its placement (crossings 19728 to 21361); hence the cap and the
  share.
- **`ratsnest_crossings.global`** (off): after the global placement the
  parts with 16 pins or more trade places and turn where wirelength
  plus crossings fall. Cheap, but the placed boards came out unchanged
  on MegaDrive and Glasgow; not measured in layouts.
- **Measured** (the eleven dense boards in layout mode, seeds 1, 101,
  201, off / on): KiCad unconnected, summed medians 926 to 927; the
  pairs 947 to 898, 898 to 782, 1032 to 1071; MegaDrive's median 110 to
  105, Glasgow 163 to 127, video 159 to 262 (the widest swing). The
  placed boards' crossings fall on 27 of 33 pairs (Glasgow below the
  designer's on every seed, MegaDrive on two). Short tier: unconnected
  26 to 36, strict passes 7 to 6. Placement work not higher.
- **Default stays off** by the agreed rule (flat medians, one pair up,
  a strict pass lost). MegaDrive's QFP order is not what the anneal
  changes: in our placements U4 and U8 lie one above the other.

## Committed on 2026-10-24 (ratsnest crossings as a placement cost)

The placement ablation's lever, on by default. Branch ratsnest-crossings,
snapshot x14270 (the switch off by default then), off side x1813;
docs/placer.md "Ratsnest crossings (2026-10-24)" has the tables.

- **`ratsnest_crossings`** (placer option, on; `null` turns it off;
  `weight` 3 mm per crossing, `legalise`, `refine`, `rank_seeds`): per
  counted net the minimum spanning tree of its pins, the pairs of tree
  lines of different nets that cross. `crates/pcb-placer/src/crossings.rs`
  keeps the count incrementally (the moved parts' nets' trees again,
  their lines walked through a bucket grid against the others; exact,
  tested against the all-pairs count over 300 random moves); the
  ablation tool now shares its tree and crossing test.
- **Where it acts**: the legaliser (a spot pays for the crossings it
  adds, within 2 mm of the closest legal spot: unbounded, the term
  widened the search and cost 7 times the legaliser's work), the
  refinement (moves, turns, swaps pay for their change), and the
  layout race's seed ranking (rank by RUDY plus rank by crossings).
  Not in the global placement or the anneal yet.
- **The weight** from the ablation data
  (`experiments/crossings/calibrate.py`): wirelength plus w crossings
  orders the variants and our layouts best from w = 2 mm; 3 mm is the
  smallest that makes the designer's placement the cheapest on
  MegaDrive, Glasgow and the 4in1.
- **Measured** (the eleven dense boards in layout mode, seeds 1, 101,
  201, off / on): KiCad unconnected, summed medians 1027 to 966; each
  pair falls (1013 to 946, 995 to 904, 1099 to 1069); 8 boards better
  by the median, 3 worse; MegaDrive 126 to 110, Glasgow 164 to 163. The
  placed boards' crossings fall on 32 of 33 pairs (sums -11 %, -19 %,
  -22 %; Glasgow at the designer's count). Short tier: unconnected 18
  to 26, strict passes 6 to 7, none lost. Placement work +17 to +23 %.
- **Default on** by the agreed rule; `{"ratsnest_crossings": null}`
  gives the placement before it (byte-identical boards on MegaDrive and
  Glasgow against main x1816).

## Committed on 2026-10-24 (connection width floor; via rules per layer)

Branch connection-width-and-vias, snapshot x14300 (every table below:
x1815, main before the branch, against x14300).

- **The gap.** Once the widths were per layer (x14291), OpenESC's inner
  layers routed at the classes' 0.09 mm, and KiCad reported 225
  connection_width warnings (2 before): the board setup's minimum
  connection is 0.16 mm at warning severity, and only error severity
  floored the router's widths (`FloorMode::Errors`, 2026-10-15). A custom
  `connection_width` rule was `unmodelled`.
- **Custom rules** (`dru.rs`). A `connection_width` rule's minimum lowers
  as a `track_width` minimum: per net, per layer where it holds on some
  layers only (`Lowered::layer_width_min`, then `NetRules::layer_widths`
  as the width rules), the neck-downs with it, reported `applied`
  ("connection width at least 0.15 mm: a floor on the track width"); max
  and opt are not read (KiCad's check has a minimum only). It floors
  where the project's connection_width severity (or the rule's own
  `(severity ...)`) is an error and, with the new default, a warning
  (`BoardFacts::connection_floor_warnings`).
- **The default.** The board setup's `min_connection` (and the custom
  rules) floor at warning severity too: `FloorMode::ErrorsAndWarnings`;
  `PCB_MIN_CONNECTION_FLOOR=errors` gives the old default, `off` none.
  Of the 60 boards measured (the 14 with a .kicad_dru, the expansion
  tier's 46), 17 state a minimum connection; it exceeds the board's
  minimum track width (so the floor can move a width) on OpenESC's two
  4in1 boards only. Their inner layers route at 0.16 mm again, as the
  designer's do (no designer track narrower than 0.16 on any layer;
  KiCad finds 1 and 0 connection_width warnings on the designer's
  boards). With the floor the rows are x1814's exactly (every layer 0.16:
  the per-layer widths collapse to one).
- **Via rules per layer: the census.** KiCad 10.0.7 on a four-layer test
  board with one through via (0.4/0.2 mm), one rule at a time: a
  `hole_size` rule holds whatever its layer clause; `via_diameter` and
  `annular_width` are evaluated on F.Cu only for a via of one size (a
  rule on `inner` or `B.Cu` never fires, `outer` and `F.Cu` do); `disallow
  via` with any layer clause catches the via, `A.Layer == 'In1.Cu'` does
  not. The lowering takes a via rule where any of its layers holds: exact
  for holes and for rules that include F.Cu, more than KiCad asks for
  diameter and annulus rules off F.Cu (no error). What the 60 boards ask:
  36 via, hole and annulus rules on 9 boards (the laptop motherboard and
  power board, tuxkeychain: JLCPCB's template; ErgoSNM; AMLing2's drone
  carrier; minuet light and FUEL4EP: one template; the power manifold),
  every one on every layer but four with `(layer outer)`: minuet light's
  and FUEL4EP's "Annular ring width (via and PTH)" (0.075 mm, outer
  includes F.Cu: exact) and three hole rules on NPTH and castellated pads
  (no via matches). All are `no_effect` (the classes and the board setup
  keep them; or no via matches) but two of the power manifold's that
  depend on the via's own size (`unmodelled`). No board has `via_count`,
  `disallow via` or a via rule with a layer condition. Nothing was
  implemented for vias: no board asks.
- **Tests.** `a_connection_width_rule_floors_the_track_width` (dru: on
  the inner layers per layer, a class per net, skipped at warning
  severity unless warnings floor, a rule's own warning, a rule the
  classes and necks keep); `a_connection_width_floors_the_widths_per_layer`
  (project: OpenESC's outer width rule with an inner connection rule
  gives the layers 0.16, 0.12, 0.12, 0.16 and `applied`; the board's 0.16
  at warning severity floors every layer and the neck);
  `the_minimum_connection_floors_the_project_widths` now expects the
  floor at warning severity.

**The 14 boards with a .kicad_dru**, route mode on the designer's
placement (`--tier designer --budget-scale 0.02`: 30 s of work, one
thread, 4 jobs, `--min-free-gb 10`), x1815 against x14300; KiCad's
errors (every type, the same types and counts on both sides on every
board), its connection_width warnings, unconnected items, vias and
length:

| Board | errors | connection_width warnings | unconnected | vias | length mm | routed |
| --- | --- | --- | --- | --- | --- | --- |
| power | 11 -> 11 | 0 -> 0 | 9 -> 9 | 118 -> 118 | 2189 -> 2189 | 106/115 -> 106/115 |
| Sisu_SSE-9 | 11 -> 11 | 1 -> 1 | 108 -> 108 | 168 -> 168 | 3477 -> 3477 | 144/192 -> 144/192 |
| MokyaLora | 6 -> 6 | 1 -> 1 | 61 -> 61 | 165 -> 165 | 4502 -> 4502 | 238/262 -> 238/262 |
| module | 22 -> 22 | 0 -> 0 | 226 -> 226 | 68 -> 68 | 1027 -> 1027 | 56/223 -> 56/223 |
| 4in1 | 0 -> 0 | 138 -> 0 | 30 -> 25 | 154 -> 146 | 2149 -> 2106 | 130/152 -> 133/152 |
| 4in1-mini | 0 -> 0 | 85 -> 0 | 39 -> 53 | 119 -> 113 | 1452 -> 1483 | 127/152 -> 122/152 |
| WLED_Matrix | 93 -> 93 | 0 -> 0 | 2 -> 2 | 82 -> 82 | 1838 -> 1838 | 100/104 -> 100/104 |
| OpenFC | 4 -> 4 | 0 -> 0 | 4 -> 4 | 132 -> 132 | 1590 -> 1590 | 78/82 -> 78/82 |
| ErgoSNM_keyboard | 3 -> 3 | 0 -> 0 | 0 -> 0 | 29 -> 29 | 4321 -> 4321 | 58/58 -> 58/58 |
| PCB | 3 -> 3 | 0 -> 0 | 0 -> 0 | 120 -> 120 | 1956 -> 1956 | 54/54 -> 54/54 |
| USB_Keypad | 80 -> 80 | 0 -> 0 | 0 -> 0 | 60 -> 60 | 1619 -> 1619 | 39/39 -> 39/39 |
| link | 30 -> 30 | 0 -> 0 | 191 -> 191 | 145 -> 145 | 5544 -> 5544 | 164/251 -> 164/251 |
| motherboard | 270 -> 270 | 0 -> 0 | 161 -> 161 | 126 -> 126 | 4552 -> 4552 | 138/236 -> 138/236 |
| SUMEC_MK_IV | 12 -> 12 | 0 -> 0 | 6 -> 6 | 90 -> 90 | 1631 -> 1631 | 66/69 -> 66/69 |
| total | 545 -> 545 | 225 -> 2 | 837 -> 846 | 1576 -> 1562 | 37846 -> 37834 | |

No new error; 223 warnings gone; the unconnected items move on the two
OpenESC boards alone, both ways (+14, -5), to x1814's rows exactly (the
same widths: 0.16 on every layer). The three CDFER boards (connection
width an error) floored before and route the same; the other boards
with a minimum connection at warning severity (Sisu, MokyaLora, OpenFC,
Sumec) have it at or below the board's minimum track width.

**Expansion tier** (46 boards, 8 x 1, x1815 against x14300): 46 rows
identical (routed, vias, length, work seconds, open connections, KiCad's
findings, pass and outcome class); 20/46 pass in both. Eight of its boards
state a minimum connection at warning severity (DIYSynth, nixie_board,
OpenRX-Mono, node_v6, LC-MARE, minuet light, facade60, anthracite), all at
or below their minimum track width: the floor is set (board-router.json's
`rules.width_floor_mm`) and changes no width.

**Layout mode** (4in1-mini, task budget, placer seed 1, 1 job x 2
threads, tasks v7, x1815 against x14300, one run each): 0 -> 0 errors,
connection_width warnings 92 -> 1, unconnected 91 -> 95, vias 122 ->
118, length 1341 -> 1478 mm, routed 89/152 -> 88/152 (x1815's row is
x14291's, x14300's x1814's: both deterministic at the seed). One run
each is no evidence of a routing gain or loss.

The call: a warning does not fail the pass rule, and the floor costs
routing on 4in1-mini (+14 unconnected in route mode, +4 in the layout)
and gains on 4in1 (-5); `PCB_MIN_CONNECTION_FLOOR=errors` restores
x1815's behaviour if the routing matters more than the designer's 0.16 mm
minimum.

## Committed on 2026-10-24 (per-layer track width and clearance)

Branch per-layer-rules, snapshots x14290 (the router, measured on the
rule boards and the expansion tier) and x14291 (also: a layer rule that
changes nothing gives no widths; every table below is x14291's unless it
says otherwise; x14291 repeats x14290's rows on all of them).

- **The gap.** The router had one track width and one clearance per net
  (its `RuleClass`). A custom `track_width` rule on some layers only was
  kept on every layer (`approximated`): OpenESC's "2 oz outer copper,
  track width" (0.16 mm on F.Cu and B.Cu over the board's 0.09) made
  every track 0.16 wide, the inner ones too, and a rule with a layer
  condition (Sisu's USB geometry, `A.Layer == 'In1.Cu' || ...`) was
  applied on every layer and reported `applied`. The layer clearances
  were already exact: a rule on some layers is a pair rule of the router
  with a stamp variant per layer (`ClearanceRule::layers`, 2026-10-21).
- **The design.** Widths per layer ride on the layer variants the pair
  clearances already have, not on more classes: `NetRules::layer_widths`
  (per net, per layer: min, max, opt) splits a net's class by its widths
  and floors per layer (`ClassRules::class_widths`, `class_floors`; the
  `RuleClass`'s `trace_width` is the widest, for what reads one value),
  and `Resolved::layer_variant` tells layers apart where any net's bounds
  differ. The stamps of a variant measure both classes' tracks at that
  variant's widths (track to track, track to via, via to track, the
  diagonal and stub stamps; via to via needs no width), the static maps
  keep pads, holes, keepouts and the outline at each layer's half width,
  and a node stamps its layer's footprints, so the occupancy maps (per
  class and layer already) stay exact and the search reads them as
  before: no new class per layer, no new map, one comparison more per
  stub. The emitted runs, escape stubs, pad-centre stubs, pad copper
  joins, manual spokes and neck-downs take their layer's width; a neck
  is per layer the board's neck or the layer's minimum, never wider than
  the layer's track (`neck_width_on`). Vias keep one size. A net whose
  bounds give one width and one floor on every layer keeps one class:
  boards without a layer rule build the same classes, variants, stamps
  and lattice as before.
- **Lowering** (`dru.rs`). A `track_width` rule is evaluated for a track
  of each net group on each copper layer; where the nets it holds for
  differ by layer, its bounds go per layer and net
  (`Lowered::layer_width_min`, `_max`, `_opt`) instead of the net's
  every-layer values, and the report says `applied` ("width at least 0.16
  mm on every net on F.Cu, B.Cu (per layer)"). A rule the classes keep
  stays `no_effect` and gives no widths (tuxkeychain's 0.127 mm outside,
  Sisu's 0.1 mm USB geometry over a 0.1 mm class). `project_rules.rs`
  folds the every-layer bounds and the board's minimum track width and
  connection floor into each layer's bounds. The placer keeps the
  largest pad clearance (x1814's `pad_clearance_mm`); route mode never
  reads it.
- **Check and write-back.** The tracks carry their layer's width into the
  board. The router's check (`check_copper`: board-router.json's
  `internal_violations`) uses `verify_with_rules` on the split classes
  where some net has widths per layer: each track at least its layer's
  minimum ("track width") and the pair clearances per layer; elsewhere
  `verify` as before. On both OpenESC boards it finds 0, as KiCad does
  (0 track_width, 0 clearance).
- **Tests.** Router: `a_net_routes_each_layer_at_its_own_width` (two
  layers, 0.3 mm front minimum, 0.15 back opt, a wall that forces a hop:
  every front track 0.3, every back track 0.15, two stamp variants, the
  check clean and a 0.25 front track found). Lowering:
  `track_widths_on_some_layers_lower_per_layer` (the OpenESC rule, a
  layer condition with opt and max, a rule the classes keep, a layer
  clause naming every layer). Project:
  `a_clearance_on_the_outer_layers_spaces_the_parts` with the width rule
  (the class keeps 0.09, the layers' minimums 0.16, 0.09, 0.09, 0.16;
  `applied`).

**The 14 boards with a .kicad_dru**, route mode on the designer's
placement (`--tier designer --budget-scale 0.02`: 30 s of work, one
thread, 4 jobs, `--min-free-gb 10`), x1814 against x14291; KiCad's
errors of the two types, unconnected items, vias and length:

| Board | track_width errors | clearance errors | other errors | unconnected | vias | length mm | routed |
| --- | --- | --- | --- | --- | --- | --- | --- |
| PCB | 0 -> 0 | 0 -> 0 | 3 -> 3 | 0 -> 0 | 120 -> 120 | 1956 -> 1956 | 54/54 -> 54/54 |
| USB_Keypad | 0 -> 0 | 58 -> 58 | 22 -> 22 | 0 -> 0 | 60 -> 60 | 1619 -> 1619 | 39/39 -> 39/39 |
| WLED_Matrix | 0 -> 0 | 62 -> 62 | 31 -> 31 | 2 -> 2 | 82 -> 82 | 1838 -> 1838 | 100/104 -> 100/104 |
| 4in1-mini | 0 -> 0 | 0 -> 0 | 0 -> 0 | 53 -> 39 | 113 -> 119 | 1483 -> 1452 | 122/152 -> 127/152 |
| 4in1 | 0 -> 0 | 0 -> 0 | 0 -> 0 | 25 -> 30 | 146 -> 154 | 2106 -> 2149 | 133/152 -> 130/152 |
| OpenFC | 0 -> 0 | 1 -> 1 | 3 -> 3 | 4 -> 4 | 132 -> 132 | 1590 -> 1590 | 78/82 -> 78/82 |
| SUMEC_MK_IV | 0 -> 0 | 8 -> 8 | 4 -> 4 | 6 -> 6 | 90 -> 90 | 1631 -> 1631 | 66/69 -> 66/69 |
| Sisu_SSE-9 | 0 -> 0 | 0 -> 0 | 11 -> 11 | 108 -> 108 | 168 -> 168 | 3477 -> 3477 | 144/192 -> 144/192 |
| motherboard | 0 -> 0 | 24 -> 24 | 246 -> 246 | 162 -> 161 | 123 -> 126 | 4804 -> 4552 | 139/236 -> 138/236 |
| power | 0 -> 0 | 0 -> 0 | 11 -> 11 | 9 -> 9 | 118 -> 118 | 2189 -> 2189 | 106/115 -> 106/115 |
| link | 0 -> 0 | 0 -> 0 | 30 -> 30 | 191 -> 191 | 145 -> 145 | 5543 -> 5543 | 164/251 -> 164/251 |
| ErgoSNM_keyboard | 0 -> 0 | 0 -> 0 | 3 -> 3 | 0 -> 0 | 29 -> 29 | 4321 -> 4321 | 58/58 -> 58/58 |
| MokyaLora | 0 -> 0 | 0 -> 0 | 6 -> 6 | 61 -> 61 | 165 -> 165 | 4502 -> 4502 | 238/262 -> 238/262 |
| module | 0 -> 0 | 0 -> 0 | 22 -> 22 | 226 -> 226 | 68 -> 68 | 1027 -> 1027 | 56/223 -> 56/223 |
| total | 0 -> 0 | 153 -> 153 | 392 -> 392 | 847 -> 837 | 1559 -> 1576 | 38087 -> 37846 | |

No track_width error on either side, and the clearance errors are the
same on every board and none from a layer rule: USB_Keypad's 58 and
WLED_Matrix's 62 are PTH pads of one footprint, the laptop
motherboard's 24 the designer's pads under its "pad to pad clearance"
rule, Sumec's 8 pads against pads and a polygon. Three boards route
differently: the two OpenESC boards and the laptop motherboard (six
layers: "Minimum Trace Width (outer layer)", 0.127 mm, is per layer now,
its inner neck-downs back at the board's 0.11 mm: 162 -> 161
unconnected). The laptop power board has the same rule on four layers
and the same route. KiCad's connection_width warnings
(OpenESC's 0.16 mm minimum connection, a warning the router does not
floor: `FloorMode::Errors`) go from 2 to 225: the inner tracks are 0.09
mm, as the project's classes say. The two OpenESC rows move by the
lattice as much as by the widths: the narrower inner tracks change the
lattice's tight channels, and 4in1 takes 0.127 mm instead of 0.125. On
the same placement at a fixed pitch (`grid_pitches_mm`, one run each,
other phases):

| 4in1, 30 s | pitch 0.125 | pitch 0.127 |
| --- | --- | --- |
| x1814 | 25 unconnected | 18 unconnected |
| x14291 | 21 unconnected | 30 unconnected |

so at 30 s of work the boards' rows say what the rules are (exact, no
error), not whether the widths route better.

**Expansion tier** (46 boards, 8 x 1, x1814 against x14291): 46 boards
in both, better 0, same 46, worse 0; every row's routed count, vias,
length, work seconds, open connections and KiCad findings the same.
Three of its boards carry a track width rule with a layer clause:
FUEL4EP's and minuet light's name inner layers the board lacks
(`skipped`), tuxkeychain's (0.127 mm outside, 0.0889 inside) are kept by
the classes (`no_effect`; with x14290 they gave widths per layer, the
same as the classes', and the same route). One outcome class differs
(nixie_board clean_medium against clean_slow: the same route, 45 s of
wall at load 22 against 29 s at 13). x14290 gave the same 46 rows. No
switch: nothing moves where no layer rule holds.

**Layout mode** (task budget, placer seed 1, 2 jobs x 2 threads, tasks
v7, x1814 against x14291, one run each):

| Board | track_width | clearance | other errors | unconnected | vias | length mm | routed |
| --- | --- | --- | --- | --- | --- | --- | --- |
| 4in1-mini | 0 -> 0 | 0 -> 0 | 0 -> 0 | 95 -> 91 | 118 -> 122 | 1478 -> 1341 | 88/152 -> 89/152 |
| 4in1 | 0 -> 0 | 0 -> 0 | 0 -> 0 | 107 -> 112 | 133 -> 145 | 2095 -> 2125 | 73/152 -> 67/152 |
| total | 0 -> 0 | 0 -> 0 | 0 -> 0 | 202 -> 203 | 251 -> 267 | 3573 -> 3465 | |

KiCad connection_width warnings 1 -> 244.

No copper error on either side. The placement differs from the first
route on (the layout's first route feeds its moves). One run each at a
seed is no evidence of a routing gain; the rules are kept exactly.

## Committed on 2026-10-24 (the OpenESC clearance regression)

Branch openesc-regression, snapshots x14280 (uncommitted, unused) and
x14281 (the fix). The report: in layout mode at the task budget, placer
seed 1, the OpenESC boards went from x1780's 57 unconnected and no
copper error (4in1-mini, this morning) to 102 unconnected and 170
clearance errors (4in1: 125 and 222) on x1813 and x14270.

- **No snapshot regressed: the input changed.** Every snapshot of the
  day gives the same bad rows on today's input (4in1-mini, layout mode,
  task budget, seed 1, 2 threads, one run each through the ledger):

  | Snapshot | Commit | Routed | Unconnected | Clearance errors | Vias | Length |
  | --- | --- | --- | --- | --- | --- | --- |
  | x1752 | (before 3bfc315) | 90/152 | 106 | 178 | 113 | 1296 mm |
  | x1780 | 3bfc315 | 89/152 | 102 | 170 | 127 | 1395 mm |
  | x1790 | ee778f5 | 89/152 | 102 | 170 | 127 | 1395 mm |
  | x1791 | 7f2b3ea | 89/152 | 102 | 170 | 127 | 1395 mm |
  | x1792 | dbfd1b5 | 89/152 | 102 | 170 | 127 | 1395 mm |
  | x1798 | c038a34 | 89/152 | 102 | 170 | 127 | 1395 mm |
  | x1799 | 76c0485 | 89/152 | 102 | 170 | 127 | 1395 mm |
  | x1813 | 2e41ea3 | 89/152 | 102 | 170 | 127 | 1395 mm |
  | x1813, the board without its .kicad_dru | 2e41ea3 | 122/152 | 57 | 0 | 139 | 1680 mm |

  4in1, x1780: 68/152, 125 unconnected, 222 clearance errors, the same
  row as x1813's. x1813 on the board without its `.kicad_dru` repeats
  this morning's x1780 row (fix3-dense2-x1780) to the millimetre.
- **What changed.** After the 10:42 incident (the worktree links that
  replaced `benchmarks/real/external`) the harvested boards were fetched
  again at 12:38, this time with their `.kicad_dru` where the repository
  has one: 14 boards, none before (harvest.py takes the `.kicad_pcb`
  and `.kicad_pro`). `board_sha256` and `task_sha256` stayed, so
  compare.py saw one input. The OpenESC boards' file holds "2 oz outer
  copper": clearance 0.16 mm and track width 0.16 mm on F.Cu and B.Cu,
  over the board's 0.09. Of the others, the short tier's (USB_Keypad,
  USB_Cable_Tester, WLED_Matrix, OpenFC) hold no copper rule; the laptop
  boards, ErgoSNM and MokyaLora do.
- **Why it broke the layout.** The 0.16 mm holds on two of four layers,
  so the custom rules' lowering makes it a pair rule of the router
  (`ClearanceRule`, layers F.Cu and B.Cu), not a net's clearance. The
  placer's copper clearance (`copper_clearance_mm`,
  `largest_clearance`) read only the classes: 0.09 mm. Crowded, the
  placer relaxed its spacing down to 0.09 (`spacing_mm` 0.09 in
  board-placer.json), and KiCad's DRC, now with the file, found the
  pads 0.09 to 0.15 mm apart: 167 pad-to-pad and 3 pad-to-hole findings
  between parts on 4in1-mini, every one by the outer layers' rule. The
  router kept the rule (and its 0.16 mm width): 102 open against 57
  without the file.
- **Fix** (pcb-kicad, placement only). The lowering also evaluates each
  clearance rule for a pad against a pad (SMD or plated, after the later
  rules) and keeps the largest that holds between the pads of every two
  nets on some layer (`Lowered::pad_clearance_all`), reported as
  `rules.pad_clearance_mm`; `largest_clearance` takes it with the
  classes'. A rule between two sets of nets (an isolation) or from pads
  to tracks only is no such floor. Tests: the dru lowering
  (`a_pad_clearance_on_some_layers_floors_the_placer`: layer rule,
  every-layer rule, pad-to-track, one net, a later pad rule) and the
  project (`a_clearance_on_the_outer_layers_spaces_the_parts`: four
  layers, the OpenESC file, `largest_clearance` 0.16, 0.09 without it).
- **Measured** (x14281 against x1813, the same input, seed 1):

  | Board | x1813 | x14281 |
  | --- | --- | --- |
  | 4in1-mini | 89/152, 102 unconnected, 170 clearance | 88/152, 95 unconnected, no error |
  | 4in1 | 68/152, 125 unconnected, 222 clearance | 73/152, 107 unconnected, no error |

  The placer's spacing is 0.16 (board-placer.json). Not x1780's 57: that
  was the board without its rules. Routing the same board under them
  (0.16 mm from tracks on the outer layers, and every track 0.16 wide on
  every layer, `approximated`: the router has one width a net, where the
  rule leaves the inner layers at 0.09) is harder; per-layer widths are
  the next lever there. Short tier (3 x 2) against x1813's
  cross1-short-off: all 12 rows the same (unconnected 18, new errors 5,
  strict 15, vias 1134, 6 passes). The route mode never calls
  `largest_clearance`; the expansion tier was not run.
- **Harness.** Rows record `project_sha256` (the `.kicad_pro` and
  `.kicad_dru`), and compare.py marks a board INPUT when it differs,
  from the prepared `source` for older rows: fix3-dense2-x1780 against
  cross1-dense-s1-off now marks 4in1-mini INPUT. The tiers' README
  ("How to measure") says so.
- **Rows before 12:38 and after** on the 14 boards are other inputs:
  the dense boards' and the short tier's comparisons across that time
  compare the OpenESC boards (and the four short-tier boards, whose rule
  files hold no copper rule) on other projects.

## Committed on 2026-10-24 (placement ablation: which decisions cost on MegaDrive and Glasgow)

A diagnostic, no placer or router change. Branch placement-ablation,
snapshots x1810 to x1812 (the tool; x1812 with the ratsnest crossings),
every route x1754 (main 3036703), docs/placer.md "Placement ablation
(2026-10-24)" has the tables.

- **`pcb-maker ablate-kicad-placement`** (docs/agents.md) writes hybrids
  of the designer's placement and a placed board: one group of parts
  moved to the other placement's poses (each footprint of more than 20
  pads, the same with its passives within 3 mm, the connectors, the
  quadrants, groups of one's own), the parts it lands on moved to the
  nearest free spot, null variants with one passive nudged 0.1 mm,
  replicates; per variant the signals a placer can read before routing
  (wirelength, escape demand's channel overflow, exits facing a body,
  overlaps, the global route's overflow and soft overflow, and the new
  ratsnest crossings), per moved part both placements' escape.
  `benchmarks/agent-tasks/ablation.py` routes the variants with run.py in
  route mode and attributes the open connections per group (unit tests
  in Rust and `test_ablation.py`).
- **Noise first.** One passive nudged 0.1 mm moves a MegaDrive route by
  a standard deviation of 12.7 open connections (ours 143, 158, 177);
  Glasgow's by 2.4. Single-route attributions on MegaDrive are good to
  about ±15.
- **MegaDrive** (designer 61.7, ours 159.3 open, means of three routes):
  our north-west quadrant costs 94 in the designer's board (31 the other
  way), U4's cluster 74 / 30, U8's 48 / 19; U4 (QFP-160) and U8
  (QFP-128) traded places, U4 ending far from the parts it shares most
  nets with (U1, J3, J2), and our passives sit 0.5 mm in front of all
  four of U4's pin rows. The other large parts alone cost nothing
  measurable.
- **Glasgow** (designer 52.0, ours 129.7): the eastern quadrants (the
  small parts around the 14-pin arrays and the TSSOP-16s) cost 62 / 22
  and 33 / 19; the BGA alone 19 from the designer's side. Escape room
  does not explain them (ours has less channel overflow there).
- **Signals.** Wirelength is wrong between the placements on both boards
  (ours shorter). Escape demand's channel overflow orders them right but
  follows the groups' costs weakly (Spearman 0.36, -0.54), and escape
  demand already drives it to zero on MegaDrive without closing the gap.
  The global route's soft overflow follows the costs (0.59, 0.82) but
  calls the designer's MegaDrive placement the worse one. **Ratsnest
  crossings** (each net's minimum spanning tree, crossing pairs counted)
  follow the costs best (0.60, 0.94) and give the designer's placement
  fewer crossings than every one of 18 of our layouts on MegaDrive,
  Glasgow and the OpenESC 4in1. Proposed next lever: the crossings as a
  placement cost (legaliser, refinement, the race's ranking); predicted
  from the attribution, an upper bound: about 60 of MegaDrive's 98 and up
  to 65 of Glasgow's 78.
- Not done (time): the 4in1's hybrids (prepared in `build/abl1/4in1`, not
  routed: the 4in1 has its signals only), Glasgow's 12 to 20 pad parts,
  MegaDrive's U4-U8 pair as one group, replicates of the top groups
  (prepared in `build/abl1/{glasgow-extra,megadrive-extra,megadrive-rep}`).
- Incident (10:42): main's `benchmarks/real/external` and `artifacts`
  became links to themselves (a bad merge); the coordinator re-fetched
  the boards. The ablation's routes read the designer projects from
  `build/abl1/designer-src` (copies of the matched designer tier's
  stripped projects) and were paused while the boards were missing.

## Committed on 2026-10-24 (differential pairs: the coupled share)

Phase 4 of differential pairs. Branch differential-pairs-4 on main
c038a34; snapshots x14240 to x14243 (x14243 the commit measured); main
is x1798. docs/router.md, "Differential pairs", has the design and "The
coupled share" the per-step table. The default stays by rules (on for
the 7 boards whose custom rules ask for pairs, off elsewhere).

**What changed, per fallback reason.** (1) *A pad's stub does not reach
the run*: the link carries straight stubs from the pads where they keep
every clearance, starting where the line to the stub's end leaves the
pad (a mask opening over the pad is left by the stub), its copper
reserved from the run's start, the pads of the run's terminal; a crossed
end keeps the router's stubs. (2) *Copper refused*: the router necked a
link down to 0.127 mm within 1.5 mm of its launches (the launch's pad,
the lead, is as wide as the link's track, narrower than track and
clearance), so the offset tracks met ground vias there; pcb-router's
`prepare_net` no longer makes neck zones at "pair launch" pads. (3) *Run
not routed*: links first in the routing order, and a via disc without
the turn allowance, were measured and not kept. (4) *Fewer than two pair
points*: a pad of each of two two-pad parts side by side (their other
pads within 3 mm) is a pair point. Also: `crossed` counted the ends
whose pads' order matches the run's (inverted since phase 2); unit tests
`a_link_is_a_net_with_launches_and_the_pairs_nets_end_beside_its_run`
(the link carries both ends' stubs, four pads in three terminals) and
`series_parts_side_by_side_are_a_pair_point`.

**The 7 rule boards** (route mode, 30 s of work, one thread, the board's
own tracks kept, KiCad's DRC with `--refill-zones`, no setting: the
default fires on all 7; x1798 against each step's snapshot):

| step | snapshot | links coupled | errors | unconnected | gap findings | dangling | vias | fallbacks (not routed / stub / fewer than two points / refused / too close / no launch) |
|---|---|---|---|---|---|---|---|---|
| main | x1798 | 3/97 | 186 | 1 262 | 74 | 40 | 966 | 52 / 30 / 65 / 4 / 4 / 4 |
| 1 the link's own stubs | x14240 | 11/97 | 151 | 1 258 | 84 | 39 | 970 | 47 / 19 / 65 / 14 / 2 / 4 |
| 2 no neck at a launch | x14241 | 19/97 | 98 | 1 232 | 73 | 39 | 965 | 48 / 22 / 65 / 2 / 2 / 4 |
| 3 links first (not kept) | dev on x14241 | 19/97 | 103 | 1 249 | 74 | 37 | 958 | 49 / 20 / 65 / 3 / 2 / 4 |
| 4 series parts, parallel (not kept) | x14242 | 24/138 | 88 | 1 245 | 63 | 38 | 951 | 71 / 28 / 29 / 3 / 6 / 6 |
| 4 series parts | x14243 | 22/150 | 96 | 1 227 | 69 | 40 | 981 | 80 / 26 / 22 / 4 / 11 / 7 |

A smaller link via disc (`2 dv + V`) on step 4's code: 22 -> 21 links
coupled, errors 96 -> 119, unconnected 1 227 -> 1 223, refused 4 -> 9:
not kept.

| board | gap/uncoupled/length x1798 -> x14243 | errors | unconnected | vias | items_not_allowed | dangling | links coupled/total | uncoupled mm | work s |
|---|---|---|---|---|---|---|---|---|---|
| EwoudVV__ducktop2__ducktop2-center | (20, 0, 0) -> (14, 0, 0) | 20 -> 14 | 499 -> 499 | 479 -> 465 | 0 -> 0 | 0 -> 1 | 2/28 -> 5/54 | 5 -> 12 | 136.4 -> 159.4 |
| EwoudVV__ducktop2__left_io | (16, 0, 0) -> (15, 0, 0) | 16 -> 15 | 351 -> 356 | 235 -> 239 | 0 -> 0 | 1 -> 0 | 0/27 -> 0/41 | 0 -> 0 | 96.9 -> 84.3 |
| EwoudVV__ducktop2__right_io | (25, 0, 0) -> (38, 0, 0) | 27 -> 40 | 185 -> 166 | 124 -> 154 | 0 -> 0 | 1 -> 3 | 0/12 -> 1/23 | 0 -> 1 | 70.5 -> 85.9 |
| OpenDrone-hw__OpenAIO-Whoop__OpenAIO-Whoop | (1, 0, 1) -> (1, 0, 1) | 0 -> 0 | 187 -> 183 | 35 -> 36 | 0 -> 0 | 0 -> 0 | 0/2 -> 0/4 | 0 -> 0 | 39.1 -> 39.2 |
| williamyang98__oculink_m2_adapter__m2_card | (0, 0, 0) -> (1, 0, 0) | 71 -> 2 | 10 -> 2 | 21 -> 22 | 0 -> 0 | 3 -> 0 | 0/9 -> 9/9 | 0 -> 10 | 24.2 -> 28.9 |
| williamyang98__oculink_m2_adapter__oculink_m2 | (12, 0, 0) -> (0, 0, 0) | 23 -> 4 | 14 -> 9 | 58 -> 47 | 0 -> 0 | 1 -> 2 | 1/10 -> 4/10 | 2 -> 8 | 32.1 -> 36.5 |
| williamyang98__oculink_m2_adapter__oculink_port | (0, 0, 0) -> (0, 0, 0) | 29 -> 21 | 16 -> 12 | 14 -> 18 | 0 -> 0 | 34 -> 34 | 0/9 -> 3/9 | 0 -> 8 | 34.4 -> 32.4 |

Totals: links coupled 3/97 -> 22/150, errors 186 -> 96 (the oculink
boards' track_width and gap findings: m2_card 71 -> 2, oculink_m2 23 ->
4), unconnected 1 262 -> 1 227, gap findings 74 -> 69, dangling 40 -> 40
(oculink_port's 34 GND on In1.Cu both ways), vias 966 -> 981. Worse:
right_io's errors 27 -> 40 (gap 25 -> 38: its pairs' nets routed side by
side uncoupled), left_io's unconnected 351 -> 356. The ducktop2 boards
move by about ten connections either way with any change to the plan
(far from complete at 30 s); their links mostly fail with the board.

**The expansion tier** (46 boards, route mode, 8 x 1, 30 s; x1798
against x14243, neither with a setting): every row identical (routed,
vias, length, KiCad's findings, pass, open, strict findings) and every
routed board identical byte for byte, the 25 boards with pairs among
them, no row cut by the wall guard; the default fires on none (no
board's rules ask). x1798's own run is identical to phase 3's x1797.

**Decision.** Kept: on the rule boards links coupled 3 -> 22, errors
186 -> 96 and unconnected 1 262 -> 1 227; where no rule asks nothing
changes. Left for later: crossed ends on fine-pitch pads (a crossover
inside the run, with staggered vias at a layer change), the links that
fail even in the hard searches (oculink_m2's 96 mm links from F.Cu to
B.Cu), and the ducktop2 boards, whose links fail with the board.

## Committed on 2026-10-24 (differential pairs: on where the rules ask; the disallowed layers)

Phase 3 of differential pairs. Branch differential-pairs-3 on main
1600943, rebased onto dbfd1b5 (capacitated corridors, off: routes
identical to x1791); snapshots x1793 to x1797 (x1797 the commit
measured); off is main's x1791. docs/router.md, "Differential pairs",
has the design. **The default flips by rule**: route mode turns
`diff_pairs` on where the project's custom rules ask for pairs, and only
there.

**The disallowed layers.** Phase 2 kept only the links' class to the
pair's layers. The pairs' nets kept the class they were lowered with, so
their stubs and the fallback routed them on any layer, and the fallback,
which drops every other net's plane, dropped the exclusive ones too:
ducktop2's pair nets crossed its solid ground layers (In1, In3, In6; its
rule disallows tracks there for every net but GND) and its impedance
rule's In4 and In5 (items_not_allowed +89 with x1766). Now a pair's nets
route in a class kept to the layers where no custom rule disallows their
tracks and where the rules' track width bounds hold their width (without
the bounds the nets moved onto In2.Cu at the outer width: left_io 37
track_width errors with x1793), and the fallback keeps other nets'
exclusive planes as track keepouts. Unit test
`a_pairs_nets_keep_off_the_layers_a_rule_disallows` (In1.Cu disallowed,
In2.Cu's width below the nets': class, link and routed tracks on F.Cu
and B.Cu only).

**The default by need.** On (`"default": "rules"`) where a custom
`diff_pair_gap` or `diff_pair_uncoupled` constraint surely holds for a
net KiCad pairs on some layer; else off (`"off"`); the settings'
`diff_pairs` or `PCB_ROUTER_DIFF_PAIRS` decides when set
(`"explicit"`). The verdict's `diff_pairs` (now in every route-mode
verdict) has `on`, `default` and `reason` (the rule and the net). A
net class's diff pair width or gap does not turn it on, unlike the
brief's wording: KiCad checks the class's gap only as a minimum (no
finding for an uncoupled pair), most projects carry KiCad's defaults, and
12 of the expansion tier's 25 boards with pairs set other class values,
among them the boards where the switch cost connections in phase 2
(AMLing2's drone carrier 3 -> 9, ausign, snaphat, ducktop2's bms); with
the class rule the tier would not have stayed identical.

**A clean fallback, no second route.** A ladder that keeps the better of
a pairs-on and a pairs-off board needs a second full route (the fallback
routes only the fallen-back pairs' nets): not added. Instead a link is
kept only when each pad's stub reaches its end of the run (phase 2 wrote
such a link as an island, its pads open: left_io's J22_TX1 counted
"coupled" with neither pad joined, x1793; with x1797 30 of the rule
boards' 97 links fall back for it); a kept link's stub is
joined to its end where the router ended it elsewhere on the end's
copper; a failed link's stub is pruned past a via; and every fallen-back
pair's nets lose what joins nothing (tracks to nowhere, vias on no
copper, KiCad-dangling nubs where removing them splits nothing). Phase
2's dangling pieces on the pairs' nets (left_io 11, oculink_m2 6, center
4 with x1793) are gone but for three vias with copper on one layer, kept
on purpose (KiCad's teardrop joined m2_card's CLOCK+ through one).

**The boards with diff_pair_gap rules** (route mode, 30 s of work, one
thread, the board's own tracks kept, KiCad's DRC with `--refill-zones`,
as phases 1 and 2; off x1791 with no setting, on x1797 with no setting:
the default fires on all 7). Errors and items_not_allowed are KiCad's
errors; dangling are track_dangling and via_dangling warnings (any net;
oculink_port's 34 are GND on In1.Cu both ways); the last column the
work.

| board | gap/uncoupled/length off -> on | errors | unconnected | vias | items_not_allowed | dangling | pairs found/coupled/partly/fallen, crossed, uncoupled mm | work s |
|---|---|---|---|---|---|---|---|---|
| EwoudVV__ducktop2__ducktop2-center | (5, 0, 0) -> (20, 0, 0) | 27 -> 20 | 499 -> 499 | 615 -> 479 | 13 -> 0 | 0 -> 0 | 61/2/0/59 x2 u5 | 164.8 -> 136.4 |
| EwoudVV__ducktop2__left_io | (11, 0, 0) -> (16, 0, 0) | 18 -> 16 | 372 -> 351 | 206 -> 235 | 7 -> 0 | 0 -> 1 | 39/0/0/39 x0 u0 | 50.3 -> 96.9 |
| EwoudVV__ducktop2__right_io | (18, 0, 0) -> (25, 0, 0) | 18 -> 27 | 193 -> 185 | 114 -> 124 | 0 -> 0 | 0 -> 1 | 21/0/0/21 x0 u0 | 42.1 -> 70.5 |
| OpenDrone-hw__OpenAIO-Whoop__OpenAIO-Whoop | (1, 0, 1) -> (1, 0, 1) | 0 -> 0 | 187 -> 187 | 35 -> 35 | 0 -> 0 | 0 -> 0 | 3/0/0/3 x0 u0 | 38.5 -> 39.1 |
| williamyang98__oculink_m2_adapter__m2_card | (0, 0, 0) -> (0, 0, 0) | 57 -> 71 | 12 -> 10 | 23 -> 21 | 0 -> 0 | 0 -> 3 | 10/0/0/10 x0 u0 | 31.8 -> 24.2 |
| williamyang98__oculink_m2_adapter__oculink_m2 | (30, 0, 0) -> (12, 0, 0) | 231 -> 23 | 10 -> 14 | 76 -> 58 | 0 -> 0 | 0 -> 1 | 10/1/0/9 x1 u2 | 35.4 -> 32.1 |
| williamyang98__oculink_m2_adapter__oculink_port | (49, 0, 0) -> (0, 0, 0) | 250 -> 29 | 10 -> 16 | 27 -> 14 | 0 -> 0 | 34 -> 34 | 10/0/0/10 x0 u0 | 30.7 -> 34.4 |

Totals: errors 601 -> 186, gap findings 114 -> 74, unconnected 1 283 ->
1 262, items_not_allowed 20 -> 0 (no board worse; off puts ducktop2's
pair nets on disallowed layers too), vias 1 096 -> 966, dangling 34 ->
40 (GND 3, one-layer vias 3). 154 pairs, 3 of 97 links coupled (phase
2's x1766: 10, islands among them); the fallbacks: fewer than two pair points
65, run not routed 52, a pad's stub does not reach the run 30, no launch
4, too short 4, copper refused 4. What helps is mostly the pairs' nets at
the rules' width (the oculink boards' track_width 454 -> 105) and on their layers,
not coupling. Worse: right_io's errors 18 -> 27 (gap 18 -> 25, two
starved thermals), m2_card's 57 -> 71 (track_width 56 -> 70), the
ducktop2 boards' gap findings (5 -> 20, 11 -> 16, 18 -> 25: their pair
nets now run side by side on F.Cu and B.Cu, uncoupled), oculink_m2 and
oculink_port's unconnected (10 -> 14, 10 -> 16).

**The expansion tier** (46 boards, route mode, 8 x 1, 30 s; x1791
against x1797, neither with a setting): every row identical (routed,
vias, length, KiCad's findings, pass, open, strict findings) and every
routed board identical byte for byte, the 25 boards with pairs among
them; the default fires on none (`"default": "off"` in each verdict:
no board's rules ask, ducktop2's bms has DIFF classes but no pair rule).
Unconnected 588, new errors 3, vias 4 308 both ways. x1796 (before the
prune's last change, which only a run with pairs on reaches) was
identical too.

**Decision.** The default flips by rule: on the rule boards KiCad's
errors and gap findings fall, unconnected is not worse in total and
items_not_allowed is worse on no board; where no rule asks nothing
changes.

## Committed on 2026-10-24 (capacitated corridors, experiment)

Florian's idea (route the board as a graph first with hard capacities;
when the lattice cannot get through a corridor, teach the graph instead
of searching wider), built behind a router switch and measured. Branch
capacitated-corridors, snapshots x14200 (as designed), x14201
(`PCB_ROUTER_CAPACITATED_STUCK`), x14202 (`PCB_ROUTER_CAPACITATED_GROWTH`).
docs/router.md, "Capacitated corridors (2026-10-24)" has the design,
the tables and the reading.

- **The switch** `capacitated_corridors` (router config and the KiCad
  router settings, off; `PCB_ROUTER_CAPACITATED_CORRIDORS`): the
  negotiation's corridors from pcb-global's global route
  (`pcb_global::CorridorPlan` behind `pcb_router::capacitated`'s trait
  and factory: pcb-global depends on the router), a net's route tiles and
  one more, the tile planner choosing inside; a miss located on the
  plan's boundaries (tiles reached against tiles not), each such
  boundary one track lower (never below the tracks routed across it),
  the nets through it and the net planned again and the net retried (two
  rounds a batch); nets in conflict for four iterations lower their
  route's most conflicted boundary every second iteration
  (`capacitated_stuck`); only a net without a global path falls back to
  the window and the board. The plan's work counts in the budgets.
  `capacitated_retries`, `capacitated_stuck`, `capacitated_growth` are
  the knobs.
- **The instrumentation**, every run, either setting: the verdict's
  `corridors` (`KiCadCorridors`): per mode (negotiation, hard) and kind
  (corridor, band, window, board) the searches, those that found a path
  and their work; the learning's misses, reductions, floors held,
  re-plans, retries, fallbacks, deferred nets and their work. Summed over
  every router of the run.
- **Measured** (pinned rates, no wall guard, so every row is
  deterministic): the switch off is identical to main's x1754 on all 46
  rows of the expansion tier. On the tier: off 588 unconnected, 20
  passes, 1250.5 s of work; as designed 667 / 20 / 1280.8; without the
  conflicts' learning 689 / 18 / 1261.2; with the band growing with the
  reroutes (`g6`) 616 / 21 / 1247.9 (Wonkle +78 in every variant).
  Fingerprints (`g6`) 46 against 43, vias +9 %, work -7 %. The eleven
  dense boards at the task budgets (`g6`) 1267 against 1010, work +3 %,
  better only on 4in1-mini (72 to 56). **Not a default.**
- **Where the time goes** (off): the negotiation's corridor searches
  55 to 63 % of the work, the hard repair's window searches 17 to 27 %,
  the negotiation's window and board searches 0.5 % of the tier's and
  10.5 % of the dense boards' work, almost all on protoesp (57 % of its
  work) and SmartSpin2k (52 %), where the tile planner's corridors miss
  in 55 and 36 % of the searches and the window then finds a path. The
  capacitated corridors miss in 0.16 % of the searches; where the
  planner's do miss, their band search costs what the window did.
- Logs: `build/corr1-exp-{x1754,x14200-off,x14200-on,x14201-on-s0,x14202-on-g6}`,
  `build/corr1-fp-{x1754,x14202-g6}`, `build/corr1-dense-{x14202-off,x14202-g6}`
  (and `corr1-dense-x1754`, three boards, the identity check).
## Committed on 2026-10-24 (differential pairs, phase 2)

Phase 2 of differential pairs, scoped to what gates the default: the
launches and the pairs in the negotiation. Branch differential-pairs-2
on main 4d9965e (rebased onto 3036703, which changes layout mode
only); snapshots x1760 to x1766 (x1766 the commit measured, x1762 the
full tier); off is main's x1753. docs/router.md, "Differential
pairs", has the design. **The switch stays off**: on the expansion tier
the boards with pairs lose 20 connections and two passes.

**Launches.** Phase 1's commonest fallback, "no launch clear of the
pads" (30 of 60 pairs), is gone (0 of 60): the launch search turns from
the pads' normal by up to a right angle, orients the normal away from
the part's middle, and asks for room for the stubs' ends beside the run
and a straight lead, not for a disc between the pads. Pair points of one
part (a USB-C receptacle's two rows, whose launches faced each other)
are joined by ordinary routing, not coupled; pads with another net's pin
between them are no pair point.

**Pairs in the negotiation.** Each link is a net of the route itself
(the envelope's width, a via disc for two vias, a class per pair layer
set kept to the pair's layers by the new `RuleClass::layers`), with its
launches as terminals; the pair's nets get a terminal at each end of the
run (merged per link), so their stubs are ordinary routing. When an
attempt is written the link's route is offset into the two nets' tracks
and vias, checked against everything routed (refused links fall back),
the kept links' stubs straightened where that holds (KiCad takes the
router's parallel stubs at the pads' pitch for a coupled piece out of
the gap's range), and what the links left open is routed by a fallback
router with everything else kept (0.05 of the budget). The plan rides
along in the new `Board::attachment`. Also: a pair's nets take the
pair's width only where a custom rule sets it (phase 1 widened every
pair to the class's diff pair width, which alone cost iorodeo 6
connections with no link at all), and kept vias' holes are lowered as
with the switch off.

**The expansion tier** (46 boards, route mode, 8 x 1, 30 s; off x1753,
on x1762 for all 46 and x1766 for the 25 with pairs): the 21 boards
without pairs route identically to off (x1762: routed, vias, length,
KiCad's findings, pass; what changed after x1762 touches only boards
with pairs). On the 25 with pairs (x1766): better 3, same 17, worse 5;
unconnected 556 -> 576, passes 9 -> 7, new errors 1 -> 3, vias 2 813 ->
2 930; 60 pairs found, 5 coupled, 10 in part, 45 fell back
(55 links, 16 coupled, 14 crossed); the fallbacks: fewer than two pair
points 25, copper refused 20, run not routed 13, launches under 1 mm
apart 6, no launch 0. Fallback work 10.9 s over the 25 boards. KiCad
finds no diff pair finding on the tier either way.

| board | gap/uncoupled/length off -> on | errors | unconnected | vias | pairs found/coupled/partly/fallen, crossed, uncoupled mm |
|---|---|---|---|---|---|
| cnlohr__rv003usb__ch32v003_rbo12864h-wfi | (0, 0, 0) -> (0, 0, 0) | 0 -> 0 | 2 -> 2 | 6 -> 5 | 1/0/0/1 x0 u0 |
| u1f992__atmega32u4-prototyping-board__atmega32u4-prototyping-board | (0, 0, 0) -> (0, 0, 0) | 0 -> 0 | 0 -> 0 | 11 -> 11 | 1/0/0/1 x0 u0 |
| u1f992__wanbeiyu__proto | (0, 0, 0) -> (0, 0, 0) | 0 -> 0 | 0 -> 0 | 16 -> 16 | 1/0/0/1 x0 u0 |
| ZSWatch__Watch-DevKit-HW__ZSWatch-Watch-DevKit | (0, 0, 0) -> (0, 0, 0) | 0 -> 0 | 2 -> 2 | 146 -> 146 | 2/0/0/2 x0 u0 |
| wonkleio__wonkle__Wonkle | (0, 0, 0) -> (0, 0, 0) | 0 -> 0 | 364 -> 326 | 651 -> 747 | 1/0/0/1 x0 u0 |
| DIYSynthMNL__Eurorack-Multi-Saturation-Module__Multi_saturation_module | (0, 0, 0) -> (0, 0, 0) | 0 -> 0 | 0 -> 0 | 10 -> 10 | 2/0/0/2 x0 u0 |
| EwoudVV__ducktop2__bms | (0, 0, 0) -> (0, 0, 0) | 0 -> 0 | 0 -> 5 | 237 -> 232 | 3/0/0/3 x0 u0 |
| siderakb__facade__facade60_daughterboard | (0, 0, 0) -> (0, 0, 0) | 0 -> 0 | 9 -> 9 | 78 -> 94 | 1/0/0/1 x0 u0 |
| marchmare__snaphat-pcb__main_board | (0, 0, 0) -> (0, 0, 0) | 0 -> 0 | 0 -> 1 | 64 -> 62 | 1/0/1/0 x1 u16 |
| mikesmitty__power-manifold__backplane | (0, 0, 0) -> (0, 0, 0) | 0 -> 0 | 1 -> 1 | 54 -> 54 | 0/0/0/0 x0 u0 |
| r4hulrr__ausign__3117_AUSIGN | (0, 0, 0) -> (0, 0, 0) | 0 -> 0 | 1 -> 2 | 74 -> 79 | 1/0/0/1 x0 u0 |
| VTC-WISDOM__node_hardware__node_v6_0_0 | (0, 0, 0) -> (0, 0, 0) | 0 -> 0 | 32 -> 26 | 93 -> 112 | 4/0/0/4 x0 u0 |
| sensors-inl__Nervous-ECG__nervous-ecg | (0, 0, 0) -> (0, 0, 0) | 0 -> 2 | 4 -> 4 | 54 -> 54 | 2/0/0/2 x0 u0 |
| iorodeo__panels_g4_single_pcb__driver_cplex_rp2040 | (0, 0, 0) -> (0, 0, 0) | 0 -> 0 | 87 -> 87 | 292 -> 292 | 1/0/0/1 x0 u0 |
| siderakb__miniboard__Miniboard | (0, 0, 0) -> (0, 0, 0) | 0 -> 0 | 6 -> 6 | 79 -> 79 | 1/0/0/1 x0 u0 |
| AMLing2__MAS500_Drone_Arena__rpi_cm5_dronecarrier | (0, 0, 0) -> (0, 0, 0) | 0 -> 0 | 3 -> 9 | 174 -> 195 | 14/3/0/11 x2 u43 |
| MAHESH-THILAK-K__ADDITIONAL-PROJECT__TokayEdge_Lite_Platform | (0, 0, 0) -> (0, 0, 0) | 0 -> 0 | 0 -> 0 | 82 -> 88 | 1/0/0/1 x0 u0 |
| WMXZ-EU__LC-MARE__LC_PAMV4b | (0, 0, 0) -> (0, 0, 0) | 1 -> 1 | 1 -> 1 | 28 -> 28 | 1/0/0/1 x0 u0 |
| smkent__pcb__flexypin-esp32c3-wroom-jig | (0, 0, 0) -> (0, 0, 0) | 0 -> 0 | 0 -> 0 | 33 -> 33 | 1/0/0/1 x0 u0 |
| zOnlyKroks__pcb-consortium__ICE40HX1K-TQ144 | (0, 0, 0) -> (0, 0, 0) | 0 -> 0 | 0 -> 0 | 193 -> 193 | 1/0/0/1 x0 u0 |
| mincrmatt12__MSign__mainboard | (0, 0, 0) -> (0, 0, 0) | 0 -> 0 | 19 -> 75 | 252 -> 184 | 8/0/2/6 x1 u37 |
| OpenDrone-hw__OpenRX-Mono__OpenRX-Mono | (0, 0, 0) -> (0, 0, 0) | 0 -> 0 | 12 -> 12 | 40 -> 40 | 1/0/0/1 x0 u0 |
| dragonlock2__kicadboards__nrf24_remote | (0, 0, 0) -> (0, 0, 0) | 0 -> 0 | 2 -> 2 | 42 -> 41 | 1/1/0/0 x1 u5 |
| tjko__brickpico__brickpico | (0, 0, 0) -> (0, 0, 0) | 0 -> 0 | 0 -> 0 | 31 -> 67 | 8/0/7/1 x8 u189 |
| electron271__tuxkeychain__tuxkeychain | (0, 0, 0) -> (0, 0, 0) | 0 -> 0 | 11 -> 6 | 73 -> 68 | 2/1/0/1 x1 u3 |

Worse, and why: MSign's mainboard (19 -> 75): six link nets in the
dense USB corner, two coupled; its power nets lose tens of terminals
(VDD 4 -> 60, +5V 0 -> 26, VBUS 0 -> 18 in the router's count) at 30 s:
the negotiation with the link nets converges worse. AMLing2's drone
carrier (3 -> 9): the CSI bundle at the CM5 connector, 3 of 12 links
coupled, 6 runs not routed and 3 refused; the fallback leaves some of
the pairs' connections and GND's open. ducktop2's bms (0 -> 5): its one
current-sense link (BMS_SRP/SRN) refused, PACK_NEG_RAW and two others
open. ausign (1 -> 2): both USB links refused, the fallback leaves one
USB_DN connection. snaphat (0 -> 1): one link coupled, one too short;
Net-(U3-SEL) open. Nervous-ECG (errors 0 -> 2): its LOD link refused,
two pieces of its stubs left dangling (`track_dangling`, an error in
that project; the prune follows a stub only from its end). Better:
wonkle (364 -> 326), node (32 -> 26), tuxkeychain (11 -> 6, its USB
pair coupled), none by coupling more than one link: the negotiation
moved. brickpico couples 8 of its 24 links and stays complete.

**The boards with diff_pair_gap rules** (route mode, 30 s of work, one
thread, skip native then KiCad's DRC with `--refill-zones`, the board's
own tracks kept, as phase 1; off x1753, on x1766): gap findings
113 -> 83, errors 601 -> 405 (track_width -257 on the oculink boards,
whose pair nets now route at the rules' width; items_not_allowed +89 on
the ducktop2 boards: pair nets on the layers their rules disallow),
unconnected 1 283 -> 1 262; 154 pairs, 8 coupled, 2 in part (97 links,
10 coupled). The fallbacks: fewer than two pair points 65 (ducktop2's
pairs through AC capacitors), run not routed 47, refused 32, no launch
4, too short 4.

| board | gap/uncoupled/length off -> on | errors | unconnected | vias | pairs found/coupled/partly/fallen, crossed, uncoupled mm |
|---|---|---|---|---|---|
| EwoudVV__ducktop2__ducktop2-center | (5, 0, 0) -> (33, 0, 0) | 27 -> 113 | 499 -> 499 | 615 -> 470 | 61/2/0/59 x1 u8 |
| EwoudVV__ducktop2__left_io | (11, 0, 0) -> (14, 0, 0) | 18 -> 94 | 372 -> 355 | 206 -> 246 | 39/3/1/35 x3 u70 |
| EwoudVV__ducktop2__right_io | (18, 0, 0) -> (23, 0, 0) | 18 -> 44 | 193 -> 182 | 114 -> 117 | 21/0/1/20 x1 u62 |
| OpenDrone-hw__OpenAIO-Whoop__OpenAIO-Whoop | (0, 0, 0) -> (0, 0, 0) | 0 -> 0 | 187 -> 187 | 35 -> 35 | 3/0/0/3 x0 u0 |
| williamyang98__oculink_m2_adapter__m2_card | (0, 0, 0) -> (3, 0, 0) | 57 -> 76 | 12 -> 10 | 23 -> 22 | 10/1/0/9 x1 u8 |
| williamyang98__oculink_m2_adapter__oculink_m2 | (30, 0, 0) -> (10, 0, 0) | 231 -> 49 | 10 -> 13 | 76 -> 60 | 10/2/0/8 x2 u4 |
| williamyang98__oculink_m2_adapter__oculink_port | (49, 0, 0) -> (0, 0, 0) | 250 -> 29 | 10 -> 16 | 27 -> 14 | 10/0/0/10 x0 u0 |

**Decision.** Off: on the tier unconnected is worse (556 -> 576),
passes fewer (9 -> 7) and errors more (1 -> 3); only the rule boards'
gap findings improve (113 -> 83). The launches are solved; what fails is the negotiation's
cost on dense boards, where link nets that lose or are refused cost the
board more than their pair (docs/router.md, "Phase 2's remaining
items"). The verdict's `diff_pairs.work_seconds` is now the fallback's.
## Committed on 2026-10-24 (thermals solidify first; the ledger test; ordered admission)

Three fixes, branch three-fixes on main 3036703; snapshot x1780 (the
first fix; the ledger changes do not change results).

**Thermals solidify first** (board_layout.rs; docs/placer.md, "Only
thermals starved: the pads solid first"). Where KiCad and the router find
every connection and only thermals are starved, the starved pads go solid
before the pours-as-tracks ladder, which runs only if that leaves anything
starved or open. x1754 against x1780 (3 x 2, `--min-free-gb 10`, tasks
v7 at the task budgets; the short tier at its third):

| Set | KiCad unconnected | new DRC errors | work s | took the new path | byte-identical boards |
| --- | ---: | ---: | ---: | ---: | ---: |
| eleven dense boards | 916 / 916 | 4 / 4 | 14952 / 14952 | 0 of 11 | 11 of 11 |
| short tier | 18 / 18 | 5 / 5 | 3329 / 2630 | 3 of 12 | 10 of 12 (the 9 off the new path, and eternal-keypad) |
| 0xCB 1337-v4.0 alone (the case) | 0 / 0 | 0 / 0 | 498 / 109 | 1 | - |

The three boards on the new path (0xCB 1337-v4.0, framework_mobo_lefthalf,
eternal-keypad) end with nothing starved or open as before, passes equal,
at 389, 270 and 40 s less work; the ladder's kept boards had 8 and 20
fewer vias on the first two (64 against 72, 76 against 96); eternal-keypad's
board is the same. No board needed the ladder after the solid pads.

**The ledger test** (`cli_exit_codes::routes_side_by_side_serialise_through_the_memory_ledger`,
seen failing once in check.sh under load by two agents). It already had a
private ledger (`<scratch>/ledger.json`, `PCB_MAKER_LEDGER`), so the
machine's ledger was not the cause. Two timing assumptions were:

- **A 25 ms hold sampled every 5 ms.** A debug route of the test board
  spends about 11 s (its work-rate calibration) before it reserves and
  then holds its 790 MB for about 25 ms (probe: ledger states every 2 ms).
  The test sampled the ledger every 5 ms through the ledger's lock and
  wanted to see one route holding; a test thread descheduled for longer
  than a hold misses it, and missing both gives `most` 0, failing with
  "both routes held memory at once".
- **A holder of 60 s and 60 s to start.** The third process was
  `sleep 60` and the routes had 60 s to be seen waiting. Under load the
  routes took 15 to 30 s to reach the ledger (probe at load 46 to 56), and
  in one of ten whole-suite runs beside a cargo build the test ran past
  60 s; a route reaching the ledger after the holder ended never waits.

Now the routes write their stderr into one pipe, which keeps the order of
the writes, and the test asserts the order of the markers in it: a
route's admission (`memory ledger: reserved`), then that route's last line
(`...; see <out>/verdict.json`, written before it releases its reservation
at exit), then the other's admission and last line. The literal parts of a
line are written whole (one write each, atomic in a pipe), so the markers
survive interleaving. The holder sleeps an hour (killed by the test) and
the routes have 600 s to wait. Failures of the test, before / after:

| Runs | before (main) | after |
| --- | --- | --- |
| the test alone, 20 times | 0 / 20 | 0 / 20 |
| alone beside a looping `cargo build --release -j 32` (load 28-39), 10 times | 0 / 10 | 0 / 10 |
| the whole cli_exit_codes binary beside the looping build (load 24-45), 10 times | 0 / 10 | 0 / 10 |

The failure did not reproduce in 30 loaded runs before the change; the
causes above are what the probes measured, not an observed failure.

**Ordered admission** (crates/pcb-kicad/src/ledger.rs `order_allows`,
benchmarks/harness_common.py the same rule; docs/benchmarks.md, "Memory
ledger: who runs next"). A waiting request is in the ledger (megabytes 0,
`waiting_mb`, `waiting_since`; run.py's waiting boards too now), and a new
request may pass the one that has waited longest only while both fit (the
oldest fits now) or, while the oldest cannot fit, only with room for it
beside every reservation that began holding after it began waiting
(`held_since`), so it fits at the latest when the runs it waits on end.
Holders that grow and binaries admitted by a harness are not held back;
a waiter held back only by its harness's floor (`waits_on_floor`) is
passed as when it cannot fit. `--lock` is unchanged. Tests: the unit test
`the_request_that_waited_longest_is_not_passed` (Rust) and test_run.py's
`test_ordered_admission_does_not_pass_the_request_that_waited_longest`,
`test_a_waiting_request_is_in_the_ledger_and_newer_ones_let_it_go_first`
and `test_two_processes_are_admitted_in_the_order_they_began_waiting`
(two harness processes; both fail on main's harness_common). Smoke with
the binaries (4000 MB in all, a holder of 3000, a harness process waiting
for 3500, then `route-kicad-board` of a 773 MB reservation that fits
beside the holder but would leave the harness short): x1754's route went
at once and ended at 1.3 s, 5 s before the harness got its memory; the
branch's route waited ("behind an older request, harness: large (pid
..., waiting 3 s for 3500 MB)") and reserved 8 s later, after the
harness's run released. No known-issue note about the ledger was in
docs/benchmarks.md to remove.

## Committed on 2026-10-24 (escape demand on by default)

The seed confirmation the escape-demand experiment proposed (docs/placer.md,
"Seed confirmation and the default (2026-10-24)"). Branch
escape-demand-default.

- **The default**: `escape_demand` is on (`KiCadBoardPlacerConfig`'s
  default and the field's serde default); `{"escape_demand": null}` turns
  it off and places as before (4in1-mini byte for byte). The measured
  version runs only where the layout hands the placer its router
  configuration; `place-kicad-board` (no rules) runs on the estimate
  without trying the lowering (it printed a failed lowering per
  refinement before).
- **The confirmation**: two more seed sets (`seed` 101 and 201, the race's
  16 seeds start there) off and on on the eleven dense boards (x1752),
  with the first pair (x1751, seed 1) three pairs a board:

| Board | off: seeds 1, 101, 201 | off median (range) | on: seeds 1, 101, 201 | on median (range) | first route open, median off / on |
| --- | --- | ---: | --- | ---: | ---: |
| OpenESC 4in1 | 79, 87, 91 | 87 (79-91) | 73, 68, 90 | 73 (68-90) | 159 / 142 |
| OpenESC 4in1-mini | 37, 36, 39 | 37 (36-39) | 57, 28, 63 | 57 (28-63) | 63 / 87 |
| split72_left | 38, 36, 19 | 36 (19-38) | 20, 27, 29 | 27 (20-29) | 39 / 46 |
| video | 165, 128, 347 | 165 (128-347) | 188, 190, 197 | 190 (188-197) | 228 / 241 |
| DSP-ADAU1452 | 15, 40, 37 | 37 (15-40) | 9, 11, 35 | 11 (9-35) | 88 / 63 |
| Glasgow | 148, 162, 192 | 162 (148-192) | 150, 163, 170 | 163 (150-170) | 169 / 182 |
| SmartSpin2k | 80, 53, 50 | 53 (50-80) | 43, 61, 46 | 46 (43-61) | 55 / 49 |
| 0xCB panel | 53, 48, 49 | 49 (48-53) | 53, 57, 56 | 56 (53-57) | 77 / 243 |
| SNSP-CPU-01 | 91, 109, 132 | 109 (91-132) | 107, 132, 85 | 107 (85-132) | 124 / 119 |
| MegaDrive | 191, 157, 116 | 157 (116-191) | 126, 99, 174 | 126 (99-174) | 174 / 162 |
| protoesp | 76, 95, 60 | 76 (60-95) | 77, 83, 74 | 77 (74-83) | 124 / 103 |
| sum of medians | | 968 | | 933 | 1300 / 1437 |

  Summed medians 968 to 933 (-3.6 %); each pair's sum falls (973 to 903,
  951 to 919, 1132 to 1019); better 6, worse 5 by the median. Better on
  all three pairs: 4in1 and DSP-ADAU1452; worse on none of the three
  but the 0xCB panel on two (one equal); the other seven split 2 to 1
  either way (video worse on two: 188 and 190 against 165 and 128, its
  off side spreading 128-347). The short tier's pair (x1751) lost nothing: 22 to 18
  unconnected, passes 6 to 6.
- **0xCB 1337-v4.0's 725 s** (short tier, on): not a disagreement between
  our connectivity and KiCad's. KiCad found nothing unconnected on the
  first route's board (selected-net unconnected 0, router 0 open); it
  found a starved thermal, and the layout's rule runs the pours-as-tracks
  ladder for starved thermals alone (`current.1 > 0` in
  `board_layout.rs`): 6 attempts, 596 s of search, none kept (each left 1
  or 2 open). The board kept was the first route's, its starved pad made
  solid afterwards by `solidify_starved_thermals` in one KiCad check; the
  final DRC has no starved thermal and the same findings as the off side
  (annular_width 13, footprint_type_mismatch 10, both the designer's).
  The rule is the cost: where only thermals are starved, solidifying
  first (one check) would avoid the ladder; left to the final-route flow's
  owner.

## Measured on 2026-10-23 (layout sweep v20, matched designer tier)

The second full measurement on one binary, and the README's numbers
moved to it. Binary x1725 (main 38edddd), tasks v7, `--quality`, pinned
work rates, both runs through the memory ledger without the lock with 10
GB kept free ([benchmarks.md](benchmarks.md#layout-sweep-v20-2026-10-23)).
Since x1539: trouble repair on by default with its two follow-ups
(victims by the trouble-maker's open terminals; cheaper exchanges), the
pin-row cost at 2 mm, straightening after the spoke repairs, the placer
refusals, touch electrodes, the silkscreen fields, router memory rounds 1
and 2; in the harness the task labels (77 tasks drop labels at run time,
so their placements differ from v19's for that reason too) and strict
rule 4.

- **Layout sweep v20** (`build/github-layout-v20.log`, 10 x 2, 26143 s
  wall, much of it waiting for memory): **53 of 109 pass the strict
  rule** (rule 4; 53 the per-type rule), 1848 unconnected, strict errors
  26, no row cut or killed, seven without a result. v19 re-judged under
  rule 4 (`rejudge.py`, `build/github-layout-v19-rule4.log`): 3 rows gain
  the strict pass with nothing open (krishveercard, Qfwfq,
  interchange-pcb-right), 49 -> 52. Against it: better 23, same 59, worse
  17; on the 99 boards with a result in both, unconnected 1827 -> 1568,
  first-route open terminals 3997 -> 3395. Medians against the
  designer's on those 99 (v19 scored again by x1725's score-kicad-board):
  vias x0.74 -> x0.76, length x1.08 -> x1.05, open mean 0.090 -> 0.077,
  bends per connection x1.20 -> x1.11, stair-steps +88 -> +59, pin-row
  crossings +1 -> +1.
- **Gained the strict pass**, 4: OM-FlexGrid-Flex (places again: the
  placer refusals' eviction), homebrew_486 (its GND zone in one piece),
  capybully (SW20's GND pad joined across its sides), d20_pcb (J2's pad
  routed). **Lost**, 3, by open connections: BAGEL-MK1.1-Avocado (1),
  Explorer (3) and hackrf-one (1).
- **No vias in SMD pads**: 0 on all 102 boards with a result.
- **Fewer refusals**: the OpenRX panel and SmartSpin2k_Panelized have a
  result (200 and 80 open, which is why the all-rows unconnected total
  rises by 21); the six remaining refusals are panels with fewer parts
  without a legal place than in v19.
- **Matched designer tier, partial** (budget scale 0.721 = v20's median
  final-ladder share over 49 rows, not v19's 0.640; a start at 0.64 was
  stopped). Its slots first waited four hours on the largest boards (the
  ledger's admission order, now in benchmarks.md's known issues), then
  the machine stayed full, so it was cut to the sweep's 48 strict-failing
  target boards and run in parts (`build/designer-x1725-matched-a`, `-b`,
  `build/designer-x1725-matched`); 27 rows when written up, none cut or
  killed. **Gap over the 25 target boards with both results**: layout
  open 986, designer route 564, placement 43 %, routing 57 %; Batch_1
  alone is 342 of the designer route's open (ours 268), and without it
  the split is 69 / 31. Not comparable with v19's 97-board split.
- The largest placement gaps measured (ours / designer route open):
  MegaDrive 191 / 56, Glasgow 148 / 55, the OpenESC 4in1 79 / 0, protoesp
  76 / 1, 4in1-mini 37 / 2; video (165 open) and SNSP-CPU-01 (91) are
  among the boards not yet routed on x1725.
- **Since v20**: x1753 (the narrow step's bound; it affects DSP-ADAU1452
  and Glasgow in layout mode) and x1754 (escape demand on by default), so
  v21 is expected to move.

## Committed on 2026-10-23 (routes survive moves; the narrow step's bound)

Two things, one measurement. Branch keep-routes-across-moves, on main
38edddd (x1725, the cheaper exchanges) and rebased onto 85cafb7 (escape
demand, off) before the merge; snapshots x1741 (the measurement), x1742
and x1743 (the narrow step's continuation floor, below).

**The narrow-signal step's overrun.** DSP-ADAU1452 on the move descent
(x1691): 742 s of work left for the narrow-signal attempt run alone,
its ladder charged 1470 s. Not the negotiation cap and not the
first-iteration gate: the attempt's first route took 192 s, then its
best rung's seed round ran three seeds with no work deadline at all. A
layout's ladder gives its seeds a deadline only out of the narrow-step
reserve (`deadline_before_reserve`) or route mode's bound; the attempt
run alone keeps no reserve (its step does not apply to itself), so each
seed negotiated to its own budget and the ladder charged three times
the longest (425 s). Now in a layout's ladder each seed (the seed rounds
and a rung's seed retry) stops at its share of the work left beyond the
reserve, also when nothing is kept. And the attempt run alone keeps
the attempt after it (the other pour mode) its floor: with at least
twice `NARROW_CONTINUATION_FLOOR` left it is sized from what is left
without it (`narrow_with_continuation`) and its ladder keeps 1.1 times
the floor plus a check from the start (`keep_for_continuation`; the
first route stops before it too: Glasgow's took 664 of 701 s; a tenth
more because seeds stop only after the batch under way: DSP after the
descent had 239 s left of 244 s). Ladder tests:
`a_layouts_narrow_attempt_alone_ends_within_its_work` (the DSP case:
two unbounded seeds would charge 192 + 2 x 425 s of 742 s);
`the_narrow_attempt_alone_leaves_the_continuation_its_floor`.

The continuation runs again (single layouts at the task budgets):

| Board, binary | descent | narrow attempt's work left | continuation | KiCad unconnected |
| --- | --- | ---: | --- | ---: |
| DSP-ADAU1452, x1691 (before) | on | 742 s | did not run (-729 s left) | 35 |
| DSP-ADAU1452, x1743 | off | 1060 s | 371 s, 69 to 30 open | 15 |
| DSP-ADAU1452, x1743 | on | 759 s | 279 s, 83 to 55 open | 27 |
| Glasgow revC3, x1741 | off | 701 s | did not run (33 s left) | 148 |
| Glasgow revC3, x1743 | off | 701 s | 265 s, 317 to 347 open (not kept) | 153 |
| Glasgow revC3, x1743 | on | 637 s | 298 s, 311 to 315 open (not kept) | 151 |

Only DSP and Glasgow take the narrow path among the eleven dense boards
at the task budgets. On Glasgow the continuation runs but never wins,
and the attempt itself, sized without the floor (negotiation 311 s
instead of 473 s), ends at 153 instead of 148.

**Routes survive moves** (Florian's idea 2; docs/placer.md, "Routes
survive moves"): `final_route_from_state` (layout.json, off;
`PCB_LAYOUT_FINAL_FROM_STATE`) starts the final route (the ladder or
the narrow-signal attempt) from the router's state: its legal copper
kept as the board's own (route mode's kept copper), the nets within 1 mm
of a terminal left open freed, the pours as the state connected them;
the ladder on the rest with at most 0.3 of the work left; from scratch
after it, at once when its first attempt leaves more open by the
router's count than the first route did (`fresh_open`, the ladder stops:
`KEPT_GAVE_UP`), else with the work left while connections stay open.
Off is main's behaviour exactly (the kept route does not run; the
narrow step's changes above are in both sides). Tests:
`a_ladder_on_kept_routes_gives_up_after_a_worse_first_attempt`.

Measured on x1741, single layouts at the task budgets (tasks v7, 1500 s
of work, pinned rates), `--min-free-gb 10`, no lock, beside one other
agent; dense boards 3 x 2 per side, the short tier 3 x 2. Output
`build/keep1-dense-d{0,1}-{off,on}` (d1: `move_descent` on),
`build/keep1-short-{off,on}`.

The eleven dense boards, the descent off (`final_route_from_state`
false / true):

| Board | KiCad unconnected off / on | strict pass | vias off / on | work s off / on |
| --- | ---: | --- | ---: | ---: |
| panel | 53 / 53 | no / no | 244 / 244 | 1226 / 1229 |
| protoesp | 76 / 76 | no / no | 388 / 388 | 1208 / 1208 |
| revC3 | 148 / 151 | no / no | 279 / 279 | 1467 / 1504 |
| video | 165 / 165 | no / no | 1482 / 1482 | 1388 / 1388 |
| 4in1-mini | 37 / 38 | no / no | 134 / 138 | 1271 / 1315 |
| 4in1 | 79 / 81 | no / no | 159 / 156 | 1376 / 1456 |
| SmartSpin2k_Panelized | 80 / 80 | no / no | 1793 / 1793 | 1388 / 1388 |
| DSP-ADAU1452 | 15 / 15 | no / no | 267 / 270 | 1391 / 1337 |
| MegaDrive | 191 / 191 | no / no | 2120 / 2120 | 1574 / 1574 |
| SNSP-CPU-01 | 91 / 91 | no / no | 1763 / 1763 | 1334 / 1334 |
| poly_kybd_split72_left | 38 / 38 | no / no | 1114 / 1114 | 1263 / 1263 |
| sum | 973 / 979 | 0 / 0 | 9743 / 9747 | 14886 / 14996 |

The same with the descent on:

| Board | KiCad unconnected off / on | strict pass | vias off / on | work s off / on |
| --- | ---: | --- | ---: | ---: |
| panel | 54 / 54 | no / no | 255 / 255 | 1105 / 1151 |
| protoesp | 72 / 72 | no / no | 382 / 382 | 1413 / 1413 |
| revC3 | 151 / 151 | no / no | 282 / 282 | 1500 / 1394 |
| video | 165 / 165 | no / no | 1482 / 1482 | 1390 / 1390 |
| 4in1-mini | 38 / 43 | no / no | 150 / 114 | 1295 / 1441 |
| 4in1 | 72 / 94 | no / no | 174 / 176 | 1428 / 1430 |
| SmartSpin2k_Panelized | 80 / 80 | no / no | 1793 / 1793 | 1395 / 1395 |
| DSP-ADAU1452 | 44 / 27 | no / no | 292 / 241 | 1198 / 1402 |
| MegaDrive | 176 / 176 | no / no | 1962 / 1962 | 1402 / 1402 |
| SNSP-CPU-01 | 98 / 98 | no / no | 1762 / 1762 | 1438 / 1438 |
| poly_kybd_split72_left | 38 / 38 | no / no | 1114 / 1114 | 1298 / 1298 |
| sum | 988 / 998 | 0 / 0 | 9648 / 9563 | 14862 / 15154 |

The descent itself (both from-state off: d0 against d1), for the
question whether it pays now:

| Board | KiCad unconnected off / on | strict pass | vias off / on | work s off / on |
| --- | ---: | --- | ---: | ---: |
| panel | 53 / 54 | no / no | 244 / 255 | 1226 / 1105 |
| protoesp | 76 / 72 | no / no | 388 / 382 | 1208 / 1413 |
| revC3 | 148 / 151 | no / no | 279 / 282 | 1467 / 1500 |
| video | 165 / 165 | no / no | 1482 / 1482 | 1388 / 1390 |
| 4in1-mini | 37 / 38 | no / no | 134 / 150 | 1271 / 1295 |
| 4in1 | 79 / 72 | no / no | 159 / 174 | 1376 / 1428 |
| SmartSpin2k_Panelized | 80 / 80 | no / no | 1793 / 1793 | 1388 / 1395 |
| DSP-ADAU1452 | 15 / 44 | no / no | 267 / 292 | 1391 / 1198 |
| MegaDrive | 191 / 176 | no / no | 2120 / 1962 | 1574 / 1402 |
| SNSP-CPU-01 | 91 / 98 | no / no | 1763 / 1762 | 1334 / 1438 |
| poly_kybd_split72_left | 38 / 38 | no / no | 1114 / 1114 | 1263 / 1298 |
| sum | 973 / 988 | 0 / 0 | 9743 / 9648 | 14886 / 14862 |

The short tier, off against on:

| Board | KiCad unconnected off / on | strict pass | vias off / on | work s off / on |
| --- | ---: | --- | ---: | ---: |
| 1337-v4.0 | 0 / 0 | yes / yes | 73 / 73 | 101 / 101 |
| PCB | 0 / 0 | yes / yes | 106 / 106 | 134 / 134 |
| USB_Keypad | 2 / 2 | no / no | 181 / 181 | 472 / 464 |
| WLED_Matrix | 8 / 8 | no / no | 194 / 194 | 405 / 475 |
| framework_mobo_lefthalf | 0 / 0 | yes / yes | 101 / 101 | 498 / 498 |
| MIDAS-MK1.1-revA | 7 / 7 | no / no | 234 / 234 | 265 / 265 |
| OpenFC | 1 / 1 | no / no | 163 / 163 | 445 / 366 |
| reCamera_S101_v1.1 | 4 / 4 | no / no | 83 / 83 | 313 / 325 |
| spell_tome_bottom | 0 / 0 | yes / yes | 9 / 9 | 27 / 27 |
| eternal-keypad | 0 / 0 | no / no | 34 / 34 | 102 / 102 |
| main | 0 / 0 | yes / yes | 39 / 39 | 55 / 55 |
| Blue_Line | 0 / 0 | yes / yes | 26 / 26 | 5 / 5 |
| sum | 22 / 22 | 6 / 6 | 1243 / 1243 | 2822 / 2817 |

Reading:

- **Routes surviving moves do not pay as built.** Dense sum 973 to 979
  with the descent off (better 0, worse 3: Glasgow +3, 4in1 +2,
  4in1-mini +1), 988 to 998 with it on (DSP 44 to 27, 4in1 72 to 94,
  4in1-mini 38 to 43); the short tier identical (22 / 22, passes 6 / 6),
  work within noise. The kept route's board was kept once: 4in1-mini
  after the descent (94 open against the route from scratch's 113, which
  had 236 s less; KiCad 43 against 38 off). DSP's 44 to 27 is not the
  kept route: its continuation ran on the on side (278 s left) and not
  on the off side (239 s of 240: x1741 lacks x1743's margin).
- **Why:** kept copper is fixed. With every complete net kept, the open
  nets had no way out (4in1-mini: 20 of 28 open nets without any path,
  DSP 30 of 41); freeing the nets around the open terminals frees most
  of a dense board (1.5 mm: 4in1-mini kept 59 of 152 nets; 1 mm: 71-72),
  and what is freed is then routed between walls the router may not
  move. On the nearly complete short-tier boards the kept route left
  more open than the state (OpenFC 4 against 2, reCamera 10 against 8):
  the freed nets lose the room the state had found for them. The route
  from scratch (another pour mode, narrow widths, all nets negotiable)
  decides the board, and the kept route costs it 45-236 s.
- **The descent still does not pay**: 973 to 988 on this base (DSP 15 to
  44 on x1741, 27 on x1743 where its continuation runs: the descent's
  300 s still cost it 12; protoesp 76 to 72, 4in1 79 to 72 and MegaDrive
  191 to 176 gain; Glasgow, SNSP, 4in1-mini and the panel lose). With
  x1743's DSP the sum would be 971 against 973: even, not a gain one
  run per side can show. With the kept route on as well, 998: not below
  978.

Proposed: neither as a default; `final_route_from_state` stays as an
override. What would make it pay is a warm start in which the router
owns the state's routes (its negotiation may rip them up), on a new
lattice and with the narrow widths, rather than kept copper.

## Experiment 2026-10-23 (escape demand in placement)

Routing structure where the placement is made, one bounded experiment
from general signals (the plan agreed with Florian). Write-up:
docs/placer.md, "Escape demand (2026-10-23)". Branch
placement-escape-demand; snapshots x1750 (uncommitted), x1751 (9d01bdb,
measured). **Go by the rule, proposed as the default after a
seed-repeated confirmation**; the switch `escape_demand` stays, off.

- **The fact** (`pcb_placer::escape`, `Problem::escape`, lowered in
  `board_placer/config.rs::escape_fact`): per footprint and side, the
  nets leaving its body there (the cheapest way to each net's nearest pad
  on another part, twice under the body), recomputed as parts move; per
  pair of facing parts the channel (gap over the track pitch times the
  routable layers) and its overflow (both facing stretches' exits less
  the nets between the two), attributed to both; the board edge and a
  part without demand towards the side cost nothing. Unit tests in
  `escape.rs`.
- **Where it acts**: the global placement inflates each part's charge on
  each side by its demand (every 15 iterations); the rotation choice
  takes, within 5 % of the shortest HPWL, the angle whose dense sides
  face the least body area (`orient::escape_blocking`); the legaliser
  adds 2 mm per track of overflow to a spot's distance; the refinement
  adds it to a move's or swap's cost; the refinement's term is scaled per
  part by the global route's overflow attribution
  (`board_placer::guidance::measured_factory`: a `pcb_global::Incremental`
  carried along the kept moves, read every 20 of them).
- `move_descent.rs`: `PartCopper`, `part_copper` and `apply` are
  `pub(crate)` for the measured version; `board_layout.rs`: the one hook,
  the router configuration handed to the placer when `escape_demand` is
  set (as for `global_guidance`).
- **Off reproduces main**: x1725 and x1751 off place byte-identical
  boards on all 23 boards (11 dense, 12 short tier).
- **Measured** (single layouts at the task budgets, x1751 off / on):
  dense KiCad unconnected 973 / 903 (better 5, worse 5, equal 1: SmartSpin2k
  80 / 43, MegaDrive 191 / 126, split72_left 38 / 20, DSP 15 / 9, 4in1
  79 / 73; 4in1-mini 37 / 57, video 165 / 188, SNSP-CPU-01 91 / 107,
  Glasgow and protoesp +1 to 2), first route open 1349 / 1273, HPWL
  +1.6 %, vias +0.9 %, work equal; short tier unconnected 22 / 18,
  strict passes 6 / 6, first route open 47 / 23, vias 1243 / 1106,
  work 2930 / 3732 s (0xCB 1337's final ladder after a KiCad-counted
  item, same result). The per-board spread is the size seed changes
  give on these boards (phase 2, the move descent): the dense sum rests on
  MegaDrive and SmartSpin2k; the short tier, no row lost, is the cleaner
  signal.

## Committed on 2026-10-23 (trouble repair: cheaper exchanges)

The trouble-makers' repair (router.md, item 7) ran out of its work
bound in round 0 or 1 on most gap boards, and the victims' hard routes
were most of that work (the base below: 710 of the repair's 804 s on
the v19 placements' gap boards). They are cheaper now. Branch trouble-cheaper;
snapshots x1720 (with `PCB_ROUTER_TROUBLE_LOCAL=0
PCB_ROUTER_TROUBLE_EARLY=0` the base: its expansion tier is identical
to main's x1647 on all 46 rows), x1721-x1723 (the variants below),
x1724 (the defaults; x1723 with its variant 2 gives the same rows on
the four boards both ran).

- **The victims on their corridors first** (`PCB_ROUTER_TROUBLE_LOCAL`,
  on): an exchange's victims are routed again hard on their corridors
  only; a victim left with more open terminals than it had is then
  routed again in its window (the terminals' box and its margin, not
  grown fourfold as the hard search's window is) and never board-wide,
  while the exchange can still pass. The full hard search tried the
  corridors, then the window grown fourfold, then the board: a victim
  that cannot be reconnected failed all three, and that was most of
  the work.
- **Undone early** (`PCB_ROUTER_TROUBLE_EARLY`, on): a victim routed
  keeps its terminals (the later ones route around it), so the open
  terminals of the trouble-maker and of the victims routed so far, the
  victims still to route counted as closing all of theirs, bound the
  exchange's result from below; once that bound is above the open
  terminals before (with ties; one less without), the exchange is
  undone. Exact: it undoes only exchanges the acceptance would refuse.
- **The summary line** says where the work went: `trouble: ...; priced
  searches s, victims s (n routed; again n, s, n of them closing
  terminals, s), kept exchanges s, n undone early`.

**The variants** (x1721, x1723; route mode, Glasgow and MegaDrive on
the designer's placement, video and MegaDrive on v19; KiCad
unconnected):

| Victims routed again | Glasgow des. | MegaDrive des. | video v19 | MegaDrive v19 |
| --- | --- | --- | --- | --- |
| hard search (base) | 57 | 58 | 150 | 176 |
| corridors, then the full fallbacks | 57 | not run | not run | not run |
| corridors only (x1722) | 55 | 74 | 119 | 176 |
| corridors, then the window (x1724) | 55 | 54 | 125 | 157 |

The full fallbacks again (the window grown fourfold, then the board)
closed terminals for 7 of 291 victims on Glasgow (1.3 s of their 86 s)
and saved little. Corridors only was the cheapest (Glasgow's repair
out of work in none of its six runs) but lost MegaDrive's designer
route: its first attempt's repair kept 0 of 37 exchanges against the
base's 12 of 37, the victims there need the detour. The window keeps
those (on MegaDrive's designer route 142 of 275 victims routed again
close terminals so; on Glasgow 0 of 376, at 29.5 s).

**Route mode, the v19 placements** (`--route-from
build/github-layout-v19`, the route-mode budget, 1 thread a board, 5
jobs beside other agents). Cells: KiCad unconnected / new strict errors
/ vias / work seconds. Logs `build/trouble4-v19-{x1720-base,x1724}.log`.

| Board | base | x1724 |
| --- | --- | --- |
| video | 150 / 0 / 1291 / 1262.2 | 125 / 0 / 1198 / 1107.3 |
| SNSP-CPU-01 | 122 / 0 / 1401 / 979.5 | 119 / 0 / 1365 / 881.8 |
| OpenESC 4in1 | 108 / 0 / 150 / 1262.5 | 108 / 0 / 150 / 1247.7 |
| Glasgow | 146 / 0 / 328 / 1261.5 | 144 / 0 / 329 / 1261.7 |
| MegaDrive | 176 / 0 / 1316 / 937.7 | 157 / 0 / 1270 / 1029.6 |
| sum | 702 | 653 |
| vias, work in all | 4486, 5703 s | 4312, 5528 s |

The repair's exchanges, summed over the ladder's attempts (each
attempt's repair has its own bound): tried / kept / open terminals
closed / the repair's work in seconds (of it the victims') / work per
exchange / repairs out of work / undone early.

| Board | base | x1724 |
| --- | --- | --- |
| video | 25 / 13 / 98 / 168.0 (163.0) / 6.72 / 2 / - | 129 / 33 / 160 / 166.8 (149.4) / 1.29 / 1 / 30 |
| SNSP-CPU-01 | 443 / 50 / 155 / 205.8 (168.9) / 0.46 / 2 / - | 417 / 48 / 117 / 122.7 (87.8) / 0.29 / 1 / 38 |
| OpenESC 4in1 | 223 / 12 / 31 / 102.8 (81.8) / 0.46 / 2 / - | 298 / 13 / 31 / 82.0 (57.2) / 0.28 / 2 / 41 |
| Glasgow | 140 / 23 / 73 / 166.5 (149.6) / 1.19 / 2 / - | 251 / 27 / 74 / 166.4 (138.9) / 0.66 / 2 / 42 |
| MegaDrive | 80 / 22 / 131 / 160.8 (146.8) / 2.01 / 2 / - | 256 / 52 / 183 / 162.1 (121.8) / 0.63 / 2 / 38 |
| sum | 911 / 120 / 488 / 804 | 1351 / 173 / 565 / 700 |

Victims routed again in their window and those closing terminals so
(x1724, v19): video 335 (115.9 s), 97 of them (55.2 s); MegaDrive 481,
233; SNSP-CPU-01 457, 114; Glasgow 500, 37; OpenESC 310, 5. The work
per exchange falls 1.6 to 5.2 times; video's first attempt (one now, two
before: the ladder kept its first) tries 129 exchanges instead of 25.
The bound still binds on most boards; the corridors-only variant
(x1722) tried 1987 exchanges in 441 s on the same boards (0.1-0.4 s
each) but closed fewer where the victims need detours.

**Route mode, the designer's placement** (`--tier designer`, 1500 s of
work; logs `build/trouble4-des-{x1720-base,x1724}.log`; exchanges as
above):

| Board | base | x1724 | exchanges, base | exchanges, x1724 |
| --- | --- | --- | --- | --- |
| video | 1 / 0 / 634 / 1567.5 | 1 / 0 / 634 / 1570.5 | 97 / 32 / 149 / 138.5 / 1.43 | 207 / 52 / 156 / 134.9 / 0.65 |
| SNSP-CPU-01 | 30 / 0 / 1438 / 908.0 | 11 / 0 / 1404 / 1081.9 | 206 / 14 / 180 / 127.7 / 0.62 | 218 / 16 / 105 / 67.3 / 0.31 |
| OpenESC 4in1 | 0 / 0 / 118 / 260.0 | 0 / 0 / 118 / 260.0 | none | none |
| Glasgow | 57 / 0 / 213 / 1261.6 | 55 / 0 / 207 / 1268.4 | 289 / 11 / 15 / 143.1 / 0.50 | 369 / 17 / 30 / 96.0 / 0.26 |
| MegaDrive | 58 / 0 / 1311 / 1432.3 | 54 / 0 / 1225 / 905.3 | 291 / 44 / 83 / 204.9 / 0.70 | 184 / 36 / 73 / 103.4 / 0.56 |
| sum | 146 | 121 | 883 / 101 / 427 / 614 | 978 / 121 / 364 / 402 |

Cells of the exchanges: tried / kept / open terminals closed / work
seconds / work per exchange. Video keeps its first attempt (2 open in
the router's count, the same board both); the four attempts after it
differ in their repairs only. MegaDrive's designer route ends after two attempts instead of three
(905 s of work against 1432: the ladder's skeleton rung is not tried;
the router's open counts 150 and 126 against 140, 240 and 149).

**Expansion tier** (46 boards, route mode, 8 x 1, 30 s; logs
`build/trouble4-exp-{x1720-base,x1724}.log`): unconnected 592 to 588,
passes 20 and 20, new errors 3 and 3, vias 4308 and 4308, length
112576 to 112520 mm; 42 rows identical, none worse in unconnected,
passes or errors: facade60 daughterboard 13 to 9 (+2 vias), MAS500 -2
vias, OpenRX-Mono and ausign a few mm. The repair on the tier: 320
exchanges tried against 316, 21 kept against 22, its work 24.3 s
against 17.9 s.

**Fingerprints** (Castor, k30-SBC, jetson, Sisu; route mode from v16, 1
x 4, budget scale 0.25; logs `build/trouble4-fp-{x1720-base,x1724}.log`):
Castor and k30 byte-identical (no exchange tried on either); Sisu the
same counts (23 unconnected, 379 vias; its first attempt
byte-identical, its second differs: 18 exchanges in 7.0 s against
19.3 s); jetson 20 unconnected both, 589 to 584 vias (61 exchanges in
14.0 s against 49 in 30.7 s, 3 kept both).

The fixture goldens do not move (check.sh --fast).

## Experiment 2026-10-23 (move descent)

Florian's idea: move single parts a little where it helps, with trials
cheap enough to run on the dense boards the coupled loop skips. Write-up:
docs/experiments/move-descent.md. Branch move-descent; snapshots x1690
(dev), x1691 (main a49136c, the dense and the first short-tier
measurement), x1692 (main 5ef2fbc, the short tier again). **No-go as a
default**; `move_descent` (layout.json, off) and `PCB_LAYOUT_MOVE_DESCENT`
stay.

- **Trial cost today** (`PCB_LAYOUT_DESCENT_CLASSIC`, a diagnostic): a
  coupled-loop trial (clone, `Router::update`, `reroute` without the
  polish) costs 56-61 s of search work on 4in1-mini (44-58 s wall) and
  125-327 s on DSP-ADAU1452 (172-440 s wall); the update's map rebuild
  0.18 s and 1.15 s of modelled work.
- **Two tiers.** Tier one: `pcb_global::Incremental` (new): the global
  route kept up to date per move, the capacities around the part
  measured again (`Capacity::rebuild_region`, equal to a full build
  there) and only its nets routed again against the rest's usage, history
  and price; 1-8 ms a move (the 0xCB panel 0.9 s). Tier two: the best
  four moves on a clone of the router: `Router::update_nets` (new; keeps
  of a net whose pads moved the branches not ending on them, and the
  state of copper-less nets whose pads stayed) and
  `Router::local_reroute` (new: the ripped nets routed hard against the
  rest, at most 2 s of search); 0.6-4.5 s of work a trial. Kept when the
  board's open terminals fall; a trial of no move first calibrates (it
  left the count unchanged on every board). `Router::finish_as_is`
  (new) gives the layout the descent's routes.
- **Candidates and moves**: parts with own open terminals, then open
  nets, then the overflow attribution; nudges of 0.5-3 mm in eight
  directions, quarter turns, swaps of same-size parts within 10 mm, legal
  by the placer's checks; re-ranked after a kept move. Budget: half of
  the moves' budget; it runs wherever the first route left terminals
  open, also where the 5 % rule skips the coupled loop.
- **Eleven dense boards** (x1691, off against on): KiCad unconnected 978
  to 1026; better 4in1 79 to 75; worse DSP-ADAU1452 15 to 35 (its
  continuation in the other pour mode no longer fits the work), SNSP-CPU-01
  83 to 108 (no final route on either side; the descent's board is
  worse in KiCad than its router count says), 4in1-mini 35 to 38, Glasgow
  148 to 151, the 0xCB panel 53 to 54; video and MegaDrive have no moves
  budget after their first route. The router's own open count after the
  descent: DSP 93 to 66, 4in1 154 to 122, protoesp 126 to 103.
  Placements (board-placer.json) identical off and on.
- **Short tier** (x1692, off and on from one binary): unconnected 22 and
  22, strict passes 6 and 6; six boards complete after the first route
  are identical; on the other six only work, vias and HPWL move
  (WLED_Matrix 194 to 271 vias: the connected-pour board kept, equal in
  KiCad's count). Off equals x1652's placements on all twelve.
- **Tier one is no ranking** for millimetre moves: kept 4 % of the moves
  it predicted better, 6 % of those it predicted equal.
- **Next, if taken up**: start the final route from the descent's router
  state, take the descent's work from the first route rather than the
  last, judge the descent's board by KiCad's connectivity, rebuild only
  the changed tiles in `Router::update`.
- `bash scripts/check.sh --fast` and the clippy gate pass; fixture
  goldens unchanged; fingerprints are route mode and do not run the
  descent.
## Committed on 2026-10-23 (nightly fingerprints as sets)

Branch nightly-fingerprints, no snapshot (scripts only). The worry: the
nightly compares a board's fingerprint lines in the order the log prints
them, and rungs side by side print theirs as they finish (the 15
expansion boards of the section below), so a pure reordering would flag
a board. **It does not:** `scripts/nightly-report.py` already sorts each
board's lines (`fingerprints_of`) and compares them as a multiset
(`fingerprint_changes`, a `Counter` difference), naming the phases whose
hashes differ; a reordering reads "identical" in the board table, the
night's Fingerprints cell and the docs' table. Nothing else in scripts
compares fingerprints: compare.py and the nightly's diff-<stage>.txt
compare result rows, and the fixture golden (tests/fixtures.rs,
`f1-fingerprints.txt`) and tests/router_fingerprint.rs sort the lines
before comparing. A line names its phase only (`fingerprint after
clean-up: <hash>`, crates/pcb-router/src/router/finish.rs), not its rung,
so a change can name the phase, not the attempt.

Changed: the tests pin it. `test_fingerprints_compare_as_a_multiset`
(two logs in another order with a repeated line: identical; one copy of
a repeated line changed, one copy more: "fingerprints changed (after
clean-up)"; the stitching line with its trailing pieces), and the dry
run's second night prints Alpha's lines reversed and still reads
identical. `fingerprint_changes`' docstring says why.

At the source: the hashes come from the router's state, not from the
printing, so the binary could print in a deterministic order without
changing a hash: buffer each router's lines and let the ladder print
them after its `thread::scope` join in rung order, with the rung's name
in each line (crates/pcb-kicad/src/board_router/ladder.rs). Then a change
could name its attempt too. Not done here.

## Committed on 2026-10-23 (the boards other routers beat us on)

Branch beaten-boards, snapshot x1714 against main's x1647 (the pin-row
cost; x1710 to x1713 were steps on the way, x1712 the measured one for
the placer and the strict rule, whose router change x1714 narrows). The
boards of the two head-to-heads (docs/benchmarks.md, "Freerouting
against x1367" and "KiCadRoutingTools against x1367") where the other
router left fewer KiCad-unconnected items, each routed on the v17
placement at the task budget (route mode, `--threads 2`, the memory
ledger, no lock; `build/beat1-*`):

| board | x1367 (the comparison) | main x1647 | x1712 / x1714 | the other router |
| --- | --- | --- | --- | --- |
| katia | CUT at 2700 s, 8.8 GB | 155/162, KiCad 9, no error; 1299 s for 1133 s of work, 6.1 GB | the same board (1099 s) | Freerouting 90 and 126 hole_clearance |
| d20_pcb | 18/19, KiCad 1 | 18/19, KiCad 1 (93 s of work) | 19/19, KiCad 0, strict pass, 12.7 s of work | KiCadRoutingTools 0 with 199 mask bridges |
| WLED_Matrix | 99/104, KiCad 11, 2 copper_edge_clearance | 99/104, KiCad 6, the same 2 | the same board | KiCadRoutingTools 10 with 93 errors |
| Batch_1 | 106/204, KiCad 441; strict: clearance 261, copper_edge 11 | 76/204, KiCad 455, the same strict | the same board; strict (rule 4): copper_edge 2 | Freerouting 307 with 158 errors |

**katia: the wall clock went to routers run one after the other.** x1371
estimated a router at 7339 MB (3183 MB of maps): under half of
MemAvailable one fitted at a time, so the 4 probes, the 3 rungs and the
3 seeds ran one after the other (`ladder: 1 of 4 probes side by side`),
each charged as work but taking its own wall clock: the first attempt
alone took 711 s, and the harness's 2700 s came before the seeds ended.
No phase was uncharged and KiCad's check was not reached. The memory
rounds of 2026-10-22 and 2026-10-23 (the maps 3183 -> 1405 MB, the
estimate 2506 MB) run all 4 probes and 3 rungs side by side: main ends in
1299 s of wall clock for 1133 s of work (the guard at 6.3 times the
budget), 9 unconnected and no error, where Freerouting leaves 90.
Nothing to fix on main.

**d20_pcb: a touch electrode no via could reach.** The open connection
is net PAD's J2.1. J2 (d20_touchpad) is one custom pad, the die's face
"20", enclosed on F.Cu by the footprint's own copper frame (no net) and
on B.Cu by the designer's keepout (no tracks, no vias); the designer
reaches it with a via in the face's corner, where the keepout has a
notch. The router allowed no via there twice over: the face is an SMD
pad, so no via inside it (`no_via_pads`, the via-in-pad rule of
2026-10-21), and the whole die is the footprint's solder mask opening
(its F.Mask polygon), a keepout for every via. Every rung left it open,
so the ladder ran out of rungs at 93 s of work. Now a touch electrode
(`touch_electrode` in board_router/lower.rs: a footprint whose only pad
is of a custom shape, copper art no part is soldered to) is not listed in
`no_via_pads`, and its own mask opening (labelled `... of the touch
electrode J2`) leaves its net the vias inside its pad (`own_opening`,
`via_inside` in grid.rs: only where a via lies inside its net's own
copper, not beside a listed pad, where only the mask web parts the via
from the pad's solder). KiCad bridges only copper of different nets in
one opening. d20_pcb routes complete with no error in 12.7 s of work,
also on the designer's own placement (1 open there before too). The
first version let every mask opening of one net keep that net's vias
(x1711, x1712): beside listed pads that routed OpenRX-Mono's U.FL from
12 to 23 unconnected on the expansion tier (x1711), and inside exposed
pads under their mask windows Sisu's fingerprint run from 23 to 29
(x1712); x1714 keeps the rule to touch electrodes.

**WLED_Matrix's two copper_edge_clearance: ours, the placer's.** Both
are AE1 (a TI CC2430DB inverted-F antenna: three small pads, no
courtyard, 18 mm of copper graphics beside them) against the board's
outline. The placer's body was its pads' box, so the layout hung the
antenna over the edge (Batch_1's two are the same footprint). Every
router routes the placement it is given: KiCadRoutingTools has the same
two. Now a footprint's copper graphics are part of its body, as its mask
graphics are (`board_placer/bodies.rs`). Layout mode (tasks v7, 2 x 2,
x1647 against x1712): WLED_Matrix 101/104, KiCad 4, copper_edge_clearance
3 -> 101/104, KiCad 6, no counting error (strict: connection_width 2;
the antenna inside the outline takes room); Batch_1 KiCad 267 ->
268, strict copper_edge 2 and a starved thermal -> none.

**Batch_1's 261 clearance findings: the panel's mounting holes.** 244 of
them are between items of one footprint: 238 pairs of a mounting hole's
own overlapping plated pads (the M parts, MountingHole_3.2mm_M3, at
0 mm), 5 of the heater coil HC1's arcs against its own pads, 1 of AE1's
copper against its feed pad. The other 17 are R62 and HC1, at the
designer's places, against the heater's copper drawn on the board: the
designer's board has them too. The stripped designer board has 284
clearance findings, past KiCad's cap of 199 (its threads overshoot it;
the designer's own board has 499, also with `--all-track-errors`), so
the strict rule took the type as unknown and counted all of ours as
new. Strict rule 4 (run.py and verdict.rs):
- a finding between items of one footprint only (each item's
  description ends in "of <ref>", the same ref) is the reference's: the
  footprint's geometry and its pads' nets, which no placement or route
  changes;
- a finding at the same place is subtracted also of a type at KiCad's
  cap; only the others of that type stay new;
- a copper_edge_clearance is at the same place by its copper item:
  KiCad names whichever outline piece is nearest beside the copper
  (Batch_1's GND ring along a card's edge: 9 of the 11 "new" ones were
  the designer's ring beside another outline piece).
Batch_1's strict findings are now copper_edge_clearance 2 (AE1, the
placement's). `rejudge.py` judges x1647's three rows as x1712's.

**Batch_1's unconnected items: not fixed.** The designer's own board
has 132 (nets across six cards joined by mouse-bite tabs); ours 455,
Freerouting 307. The router counts 1360 open: it takes the cards for one
board through the tabs and spends its budget on connections between
them. Left open.

**Measured.** The expansion tier (46 boards, route mode, 30 s, 8 x 1),
x1647 against x1714: every row the same (unconnected 592, passes 20,
vias 4308, length 112 576 mm) and every board's fingerprints the same as
a set. In the order the log prints them 15 boards differ, on x1647 run
twice as well: rungs side by side print their lines as they finish.
(x1712: better 1, nervous-ecg 4 -> 2, same 45; x1711: OpenRX-Mono 12 ->
23.) The fingerprint boards (Castor, k30, jetson, Sisu; route mode from
v16, budget scale 0.25, 1 x 4): x1714's fingerprints identical to
x1647's on all four (x1712 moved Sisu's, above). check.sh --fast, the
clippy gate.

## Committed on 2026-10-23 (differential pairs, phase 1)

Differential pairs, the first feature of the Allegro stretch goal, behind
a switch: `diff_pairs` in the router settings, `PCB_ROUTER_DIFF_PAIRS=1`
(route mode; default off). docs/router.md, "Differential pairs
(2026-10-23)", has the design; docs/agents.md the option and the
verdict's `diff_pairs`.

**What it does.** Pairs by KiCad's names (`D+`/`D-`, `USB_DP`/`USB_DN`,
`TX_P_1`/`TX_N_1`: `diff_pair_of`, KiCad's `MatchDpSuffix`); per layer
the width and gap from the custom rules (`inDiffPair()` is evaluated
now, the last rule that surely holds, as KiCad's `EvalRules`), else the
class's `diff_pair_width` and `diff_pair_gap`, else its track width and
clearance; layers where a rule disallows the pair's tracks are none of
the pair's. A minimum spanning tree links the parts where the pair's
pads sit side by side; each link is routed first, in a router of its
own, as one track the width of the pair's envelope (`2 w + g`) with a
via the size of two vias side by side, from a launch just clear of one
part's pads to one of the other's; the centreline is offset into the
two nets' tracks and vias, straight stubs join the pads, everything is
checked against the board (`verify`) and written into the project as
the board's own copper, which the route keeps (`kept.rs`); what is
refused or not found falls back to ordinary routing. With the switch a
terminal reaches its whole kept piece (`Board::terminal_copper`) and a
kept via's drill is a hole. The project-rules report says `applied` for
the diff pair rules the pairs take.

**Pairs in the census.** The intake (5 834 boards): 1 894 boards have
nets KiCad pairs by name (mostly a USB `D+`/`D-`), 8 a custom
`diff_pair_gap` rule (ducktop2's center, left_io and right_io, three
oculink_m2_adapter boards, OpenAIO-Whoop, mikesmitty's backplane, whose
nets pair by no name). In the expansion tier 24 of the 46 boards have
pairs (60 pairs; listed below), the backplane the rule only.

**The boards with diff_pair_gap rules** (route mode, 30 s of work, one
thread; before x1703 (main), after x1733 with the switch on; KiCad's
errors; "gap/uncoupled/length" are diff_pair_gap_out_of_range,
diff_pair_uncoupled_length_too_long and length or skew findings;
pairs: found/coupled/partly/fallen back, crossed links, uncoupled mm;
the oculink boards are NOASSERTION-licensed and were routed from the
intake's local copies, not fetched):

| board | gap/uncoupled/length off -> on | errors | unconnected | vias | pairs found/coupled/partly/fallen, crossed, uncoupled mm |
|---|---|---|---|---|---|
| EwoudVV__ducktop2__ducktop2-center | (5, 0, 0) -> (5, 0, 0) | 27 -> 26 | 499 -> 499 | 615 -> 645 | 61/3/1/57 x1 u18 |
| EwoudVV__ducktop2__left_io | (11, 0, 0) -> (18, 0, 0) | 18 -> 36 | 372 -> 358 | 206 -> 233 | 39/1/2/36 x1 u19 |
| EwoudVV__ducktop2__right_io | (18, 0, 0) -> (19, 0, 0) | 18 -> 19 | 193 -> 194 | 114 -> 113 | 21/0/4/17 x0 u19 |
| OpenDrone-hw__OpenAIO-Whoop__OpenAIO-Whoop | (0, 0, 0) -> (0, 0, 0) | 0 -> 0 | 187 -> 186 | 35 -> 36 | 3/0/0/3 x0 u0 |
| williamyang98__oculink_m2_adapter__m2_card | (0, 0, 0) -> (0, 0, 0) | 67 -> 37 | 11 -> 10 | 23 -> 17 | 10/9/0/1 x0 u33 |
| williamyang98__oculink_m2_adapter__oculink_m2 | (43, 0, 0) -> (17, 0, 0) | 245 -> 61 | 5 -> 9 | 71 -> 61 | 10/4/0/6 x1 u16 |
| williamyang98__oculink_m2_adapter__oculink_port | (0, 0, 0) -> (7, 0, 0) | 79 -> 56 | 12 -> 9 | 16 -> 16 | 10/3/0/7 x0 u27 |

Totals: errors 454 -> 235 (the oculink boards' track_width: the pairs
route at the rules' 0.1549 or 0.1712 mm, not the class's 0.2),
unconnected 1 279 -> 1 265, KiCad's gap findings 77 -> 66, no uncoupled
or length findings either way (no board has a diff_pair_uncoupled rule).
m2_card couples 9 of its 10 pairs (96 mm) and has no gap finding;
oculink_m2's 43 gap findings go to 17 with 4 pairs coupled, and 4 more
connections open: the fixed runs in front of J1 (0.4 mm pitch) leave
its GND pins between the pairs no way out. The ducktop2 boards couple
few pairs (most "fewer than two parts with the pair's pads side by
side": their pairs pass through AC capacitors, and "no launch clear of
the pads"); left_io gains 7 gap findings from the router's stubs at
pairs coupled only in part.

**The expansion tier** (46 boards, 8 x 1, 30 s): with the switch off,
x1730 is identical to x1647 (main's routing) on every row (unconnected
592, new errors 3, vias 4 308, length 112 576 mm, passes 20); the
fingerprints (Castor, k30-SBC, jetson, Sisu; 1 x 4) are identical to
x1647's. With the switch on (x1733, the 25 boards with pairs or a
diff pair rule; the 21 others find no pair and route as before):
better 4, same 15, worse 6; unconnected 560 -> 623, passes 9 -> 8
(ducktop2 bms 99/99 -> 98/99: its current sense pair BMS_SRP/SRN
coupled), errors 20 -> 20, vias 2 813 -> 2 750; 60 pairs found, 9
coupled, 12 in part, 39 fell back (74 links, 21 coupled, 3 crossed); the
fallbacks: "no launch clear of the pads" 30 (USB connectors: shield and
neighbour pads in front of D+ and D-), refused copper 17, fewer than two
pair points 17, no path 6. KiCad finds no diff pair error on the tier
either way (no tier board has a gap maximum: KiCad's implicit pair rule
has a minimum only). Worse: MSign mainboard (19 -> 50: eight pairs, its
USB pairs' runs fixed through the dense part), wonkle (364 -> 391),
AMLing2's drone carrier (3 -> 9, 4 of its 14 CSI pairs coupled),
iorodeo (87 -> 93), node (32 -> 33), bms (0 -> 1); better: tuxkeychain
(11 -> 9), facade60 (13 -> 10), Nervous-ECG (4 -> 2), miniboard (6 -> 4).

| board | gap/uncoupled/length off -> on | errors | unconnected | vias | pairs found/coupled/partly/fallen, crossed, uncoupled mm |
|---|---|---|---|---|---|
| cnlohr__rv003usb__ch32v003_rbo12864h-wfi | (0, 0, 0) -> (0, 0, 0) | 0 -> 0 | 2 -> 2 | 6 -> 6 | 1/0/0/1 x0 u0 |
| u1f992__atmega32u4-prototyping-board__atmega32u4-prototyping-board | (0, 0, 0) -> (0, 0, 0) | 0 -> 0 | 0 -> 0 | 11 -> 11 | 1/0/0/1 x0 u0 |
| u1f992__wanbeiyu__proto | (0, 0, 0) -> (0, 0, 0) | 0 -> 0 | 0 -> 0 | 16 -> 16 | 1/0/0/1 x0 u0 |
| ZSWatch__Watch-DevKit-HW__ZSWatch-Watch-DevKit | (0, 0, 0) -> (0, 0, 0) | 0 -> 0 | 2 -> 2 | 146 -> 139 | 2/1/1/0 x1 u26 |
| wonkleio__wonkle__Wonkle | (0, 0, 0) -> (0, 0, 0) | 0 -> 0 | 364 -> 391 | 651 -> 607 | 1/0/1/0 x0 u5 |
| DIYSynthMNL__Eurorack-Multi-Saturation-Module__Multi_saturation_module | (0, 0, 0) -> (0, 0, 0) | 0 -> 0 | 0 -> 0 | 10 -> 10 | 2/0/0/2 x0 u0 |
| EwoudVV__ducktop2__bms | (0, 0, 0) -> (0, 0, 0) | 0 -> 0 | 0 -> 1 | 237 -> 231 | 3/1/0/2 x0 u18 |
| siderakb__facade__facade60_daughterboard | (0, 0, 0) -> (0, 0, 0) | 0 -> 0 | 13 -> 10 | 76 -> 87 | 1/0/0/1 x0 u0 |
| marchmare__snaphat-pcb__main_board | (0, 0, 0) -> (0, 0, 0) | 0 -> 0 | 0 -> 0 | 64 -> 65 | 1/0/1/0 x1 u14 |
| mikesmitty__power-manifold__backplane | (0, 0, 0) -> (0, 0, 0) | 7 -> 7 | 1 -> 1 | 54 -> 54 | 0/0/0/0 x0 u0 |
| r4hulrr__ausign__3117_AUSIGN | (0, 0, 0) -> (0, 0, 0) | 0 -> 0 | 1 -> 1 | 74 -> 79 | 1/0/1/0 x0 u17 |
| VTC-WISDOM__node_hardware__node_v6_0_0 | (0, 0, 0) -> (0, 0, 0) | 5 -> 5 | 32 -> 33 | 93 -> 91 | 4/0/0/4 x0 u0 |
| sensors-inl__Nervous-ECG__nervous-ecg | (0, 0, 0) -> (0, 0, 0) | 0 -> 0 | 4 -> 2 | 54 -> 59 | 2/1/0/1 x0 u1 |
| iorodeo__panels_g4_single_pcb__driver_cplex_rp2040 | (0, 0, 0) -> (0, 0, 0) | 0 -> 0 | 87 -> 93 | 292 -> 283 | 1/0/0/1 x0 u0 |
| siderakb__miniboard__Miniboard | (0, 0, 0) -> (0, 0, 0) | 1 -> 1 | 6 -> 4 | 79 -> 76 | 1/0/0/1 x0 u0 |
| AMLing2__MAS500_Drone_Arena__rpi_cm5_dronecarrier | (0, 0, 0) -> (0, 0, 0) | 0 -> 0 | 3 -> 9 | 176 -> 182 | 14/4/0/10 x0 u39 |
| MAHESH-THILAK-K__ADDITIONAL-PROJECT__TokayEdge_Lite_Platform | (0, 0, 0) -> (0, 0, 0) | 0 -> 0 | 0 -> 0 | 82 -> 81 | 1/0/0/1 x0 u0 |
| WMXZ-EU__LC-MARE__LC_PAMV4b | (0, 0, 0) -> (0, 0, 0) | 1 -> 1 | 1 -> 1 | 28 -> 28 | 1/0/0/1 x0 u0 |
| smkent__pcb__flexypin-esp32c3-wroom-jig | (0, 0, 0) -> (0, 0, 0) | 4 -> 4 | 0 -> 0 | 33 -> 33 | 1/0/0/1 x0 u0 |
| zOnlyKroks__pcb-consortium__ICE40HX1K-TQ144 | (0, 0, 0) -> (0, 0, 0) | 0 -> 0 | 0 -> 0 | 193 -> 217 | 1/0/0/1 x0 u0 |
| mincrmatt12__MSign__mainboard | (0, 0, 0) -> (0, 0, 0) | 0 -> 0 | 19 -> 50 | 252 -> 219 | 8/0/1/7 x1 u8 |
| OpenDrone-hw__OpenRX-Mono__OpenRX-Mono | (0, 0, 0) -> (0, 0, 0) | 0 -> 0 | 12 -> 12 | 40 -> 40 | 1/0/0/1 x0 u0 |
| dragonlock2__kicadboards__nrf24_remote | (0, 0, 0) -> (0, 0, 0) | 2 -> 2 | 2 -> 2 | 42 -> 42 | 1/1/0/0 x0 u5 |
| tjko__brickpico__brickpico | (0, 0, 0) -> (0, 0, 0) | 0 -> 0 | 0 -> 0 | 31 -> 32 | 8/0/7/1 x0 u177 |
| electron271__tuxkeychain__tuxkeychain | (0, 0, 0) -> (0, 0, 0) | 0 -> 0 | 11 -> 9 | 73 -> 62 | 2/1/0/1 x0 u4 |

**Verdict and default.** The switch stays off: on the tier six rows lose connections
and one its pass, KiCad's errors do not move, and on the boards whose
rules bound the gap coupling helps KiCad's gap and width findings but
not completion. Phase 1 is the mechanism (detection, rules, the coupled
search, offsetting, the stubs, the kept copper, the report); turning it
on by default needs the launches and the negotiation of phase 2.

**Phase 2.** (1) Launches: the commonest refusal is "no launch clear of
the pads" (a USB connector's shield pads and its neighbours in front of
D+ and D-): try lateral and angled launches, or launch from the stubs'
ends routed first. (2) Polarity: swap where the part allows, or cross in
the run with staggered vias, instead of crossed uncoupled stubs.
(3) Length and skew matching (KiCad's `length` and `skew` rules: a
meander pass on the coupled run, within_diff_pairs skew first).
(4) GND return vias beside each pair via. (5) Per-layer widths in one run
(ducktop2's DIFF_85: 0.183 mm outside, 0.114 on In2.Cu). (6) Pairs in the
negotiation rather than fixed first: fixed runs in front of a fine-pitch
connector starve the GND pins between the pairs (oculink_m2's J1), and
two-wire nets that are no signal pairs (ducktop2 bms's current sense
`BMS_SRP`/`BMS_SRN`) lose a connection. (7) Coupled branches (a T at an
ESD part) instead of one run per link.

Binaries: x1703 (main) before; x1730 the first measured (the tier with
the switch off, the fingerprints), x1731 to x1733 the steps after
(one class and one run for the pairs: ducktop2-center's pairs router 41
-> 11 s of work; disallowed layers; custom pair clearances), x1733 the
commit measured with the switch on.

## Committed on 2026-10-23 (run diagnosis and override preflight)

Three of the P0 items of the remapper diagnostic trial
([reviews/2026-10-18-remapper-agent-diagnostic-trial.md](reviews/2026-10-18-remapper-agent-diagnostic-trial.md)):
an agent that wanted to know why nets stayed open had to join
board-router.json, a separate KiCad DRC and raw geometry by hand; a
documented `net_classes` override routed 0.15 mm tracks under a 0.2 mm
board minimum without a word; and the access and cut diagnostics refused
an outline the router reads. Branch run-diagnosis, binaries x1700 (the
router as committed) and x1702 (the branch's head).

**inspect-routing-run** (src/agent_cli.rs, crates/pcb-kicad/src/run_diagnosis.rs;
docs/agents.md, "Which nets stayed open"): `<run-dir> [--net N]
[--terminal REF:PAD] [--reference <dir>] [--native-check] [--output f]`
reads a route or layout run's saved outputs and writes, per open net (or
the one named): the groups of pads the result board's copper joins (the
board lowered as the router lowers it, joined by
`pcb_router::verify::copper_components` without pours; each pad's ref,
number, uuid, position in mm and copper layers), KiCad's unconnected
pairs joined to the groups, the counts side by side with what each counts
(the router's terminals, the groups and missing links, KiCad's pairs;
KiCad's 499-item cap flagged), the net's rules with their sources (class,
board minimum with its own source, custom rule; the class's own value
below the minimum; the run's overrides; the widths emitted), the
reference's route (connected, groups, segments, vias, layers, widths),
dead pads and hot spots within 3 mm, and `access` and `cause`: unknown.
At the top: the router's, KiCad's (not_run, passed, failed) and the
groups' checks, KiCad's pairs on nets not inspected, and the ladder's
attempts. No search: on MegaDrive's 30 s run below, all 191 open nets
with the reference in 0.8 s. A net with a zone lists only the groups
KiCad's pairs name (its fill joins the others or not; not modelled).

**The attempt ledger** (board_router/ladder.rs, result.rs): route mode's
ladder records every attempt it judges (`attempts` in board-router.json:
name, pitch, pours, open count as the ladder ranks it, counted by kicad
or router, vias, iterations, work as charged, routing seconds,
`negotiation_stop`, `selected`). The router records why its last
negotiation ended (pcb-router `RoutingResult::negotiation_stop`:
conflict_free, stalled, work_deadline, negotiation_work_limit,
probe_work_limit, wall_guard, past_wall_deadline, hopeless,
iteration_cap); board-router.json has the selected board's. A record:
nothing reads it to route.

**Override preflight** (project_rules.rs `preflight_overrides`,
board_router/config.rs `allow_below_project_minimum`): a router file's
`net_classes` width or clearance, `default_rules`, `connection_rules`,
`neck_width_mm`, `narrow_signal_mm` or `relaxed_clearance_mm` below the
project's minimum (`min_track_width`, `min_clearance`; for
`default_rules` and `connection_rules` clearances the project's class
clearance) refuses the run before any work (exit 2), each value with the
project's minimum and its source. With `"allow_below_project_minimum":
true` the run goes on, `rules.overrides` in board-router.json lists them
and the verdict's next actions lead with a warning. The project rules
report carries the board's minimums with their sources (`rules.minimums`;
F1's golden gains it). describe-kicad-board shows a class's own width or
clearance (`class_track_width_mm`, `class_clearance_mm`) where it is below
the project-resolved value it already printed. Tested with the trial's
case (0.15 mm classes under a 0.2 mm minimum) as a unit test and through
the CLI.

**Outlines** (copper_geometry.rs): inspect-kicad-terminal-access and
inspect-kicad-routing-cut, the two commands the trial ran, were removed
with the research grid router on 2026-10-21 (47883ff). The outline
reader they shared with the board statistics (one `gr_rect` or one loop
of `gr_line`s) is still `board_outline`: inspect-kicad-board's
`board_outline_area_mm2` and the verification preview read it. It now
falls back to the lowering's (`outline::board_loops`: lines, arcs,
rectangles, polygons, circles, curves); the preview names the lowering's
reason when there is none. Tested with a rounded rectangle of lines and
arcs (area within the arcs' chord tolerance).

**Docs**: "Connections stay open" in agents.md no longer says the board
is too dense: that is a hypothesis, and the discriminating query is the
reference's route of the open nets beside the groups the run left
(inspect-routing-run --reference). The verdict's open-connections next
action names the command and says "not below the board's minimum" for
narrower classes. SKILL.md has the command; the skill test admits
`inspect-` commands beside `<verb>-kicad-<noun>`.

**Measured.** MegaDrive on its v19 placement
(build/bus1-r2-v19-x1674-off), route mode, `ladder_budget_seconds` 30,
x1700 (build/diag1-megadrive-x1700): 107 of 298 connections, 856
unconnected terminals, KiCad's report at its cap of 499. One attempt
(0.127 mm, pours connect, negotiation_work_limit, 42.4 s of work against
the bound's 31.5: a fresh router's first iteration). For /VD1 the
command names three groups (J2:a5, U1:4, U2:12, U8:56; J3:a23; U4:139),
KiCad's two pairs (J3:a23 to U4:139, J3:a23 to a track of the first
group), the Default class's 0.25 mm and the board's 0.1 mm minimum with
their sources, 0.1 mm necks emitted, and the designer's route: one group,
79 segments, 6 vias on F.Cu and B.Cu. The router counts 6 of 6 terminals
unconnected: its search's state, not the copper written, which the
explanation says. Fingerprints on Castor and k30-SBC (route mode from
v16, budget scale 0.25, 1 x 4) against x1652: identical, the
fingerprint lines and the boards byte for byte (work 174.4 and 277.0 s
both; build/diag1-fp-x1652, build/diag1-fp-x1700); each ledger holds the
one attempt, the probed rung's continuation, conflict_free.

**Left out**: an access probe and the cut (`access`, `cause` stay
unknown); joining a pair's track or via to a group (`null`); zone fill in
the groups; hot spots' object links; a post-run native check with input
hashes and the reference's comparison beyond the pairs; the ledger's
config hash, grid origin and seed.

## Experiment 2026-10-23 (bus bundles)

Florian's idea 4 (route a bus as a bundle, break it up where it cannot
follow), one bounded phase behind a switch. Write-up:
docs/experiments/bus-bundles.md. Branch bus-bundles; snapshots x1670
(first base), x1671 (ablations), x1672 (patience variable), x1674 (head,
measured on both sides).
**No-go**; the switch stays, off.

- **`bus_bundles`** (router config, `PCB_ROUTER_BUS_BUNDLES`, off;
  `router/bundles.rs`): four or more signal nets of 2-12 pads joining the
  same footprints form a bus, ordered along the first footprint's row;
  one tile corridor per bus (a tree over the footprints' pad tiles,
  priced by the room left for the bus's width, grown by half the width
  plus four tiles); the members route first after the pour nets, in
  order, inside it, a trace step off the lane beside their neighbours'
  copper costing 1.5 times (`bus_lane_cost`); a member with no path in
  the corridor, or rerouted 8 times, negotiates as before.
  `PCB_ROUTER_BUS_{LANE,MARGIN,PATIENCE,NO_CORRIDOR,NO_ORDER,WHY}`.
- **Bus boards in route mode** at the task budgets, x1674 (on the
  trouble repair's victims merge) off against on: v19 placements 433 to
  457 KiCad unconnected (video 130 to 141, SNSP-CPU-01 127 to 135,
  MegaDrive 176 to 181); designer placements MegaDrive 58 to 70,
  SNSP-CPU-01 4 to 3, video complete both (vias 603 to 503). Bus nets'
  open terminals fall by a third to a half (video 153 to 93, MegaDrive
  145 to 76), bends per connection and vias drop, but the other nets
  lose more. (First base, x1660 against x1670: v19 469 to 484.)
- **Expansion tier** (46 boards, 30 s): strict passes 19 to 21, KiCad
  unconnected 625 to 721 (OpenDeck 9 to 68, MSign mainboard 19 to 62);
  ablations 641 to 804, none better than off.
- **Switch off identical:** fingerprints of Castor, k30-SBC, jetson and
  Sisu (x1660 against x1670, x1683 against x1674) and every
  expansion-tier row (x1660 against x1670 and x1674).
- **Reading:** the bundles hold and the buses go in; the room they take
  costs the short nets more than the buses gain, and the bundled search
  doubles an iteration's work. A phase 2 would want the trouble
  repair's trade test for whole bundles and a lane that keeps the A*
  bound tight.
## Committed on 2026-10-23 (pin-row crossings)

Reviewers' first complaint after the bends: traces "snaking through SMD
pins". score-kicad-board counts them as pin-row crossings (another net's
track across the line between two adjacent SMD pads of a part; ours
median 0.013 per connection against the designers' 0.005); the
straightening cannot fix them (it keeps corners and layers). Branch
pin-row-crossings; snapshots x1640 (the cost, off by default; on the
trace-aesthetics base), x1641 (with a temporary switch for the pitch
cap), x1642 (rebased on 81356b8, off by default), x1643 (2 mm by
default; x1640 to x1642 have since been removed from build/bin), x1644
(the commit, rebased on the trouble-victims merge).

**The cost** (docs/router.md, "Pin-row cost"): the lowering lists the
gaps between adjacent SMD pads of a footprint's rows (as the measure
pairs them, at most 1.5 mm apart: `Board::pin_row_gaps`); the router
marks the lattice nodes in a band across the line between each pair's
centres (one node per crossing where the line runs on a node row), the
marked node's `Hot::pour` index names a (guard, cover, gap) triple, and
a step there by a net other than the two pads' costs `pin_row_cost` mm
(`pin_row_cost_mm`, `PCB_ROUTER_PIN_ROW_COST`; 0 off). The clean-up's
comparison counts it too, the straightening enters no new gap node.
Fingerprints at 0 identical to before on Castor, k30, jetson and Sisu
(x1640 against x1618; jetson x1642 against x1660).

**Expansion tier** (46 boards, route mode, 30 s, 8 x 1, one binary
x1642 at 0 and at 2): unconnected 625 -> 592, passes 19 -> 20, new
errors 3 -> 3, vias 4223 -> 4308, length 112254 -> 112576 mm, work 1249
-> 1253 s, crossings 211 -> 197 (fewer on 10 boards, more on 3; those of
pads within 1.5 mm 59 -> 48), stair-steps 2979 -> 2803, bends per
connection median 1.433 -> 1.465. Other values (x1640/x1641 against
x1618's log; x1640 at 0 routes as x1618, its fingerprints identical): 3, 4, 6 and 20 mm each add
two starved thermals (ZSWatch's X403 or X602, anthracite-bottom's J106,
PTH pads of a pour net with a pour-as-tracks GND track wrapped round
them: a step in another net's thermal guard costs 10 steps, about 1.25
mm, so the gap's price outbids a few guard steps and KiCad's refill
starves what the spoke model passes); crossings 211 -> 205 (3 mm), 211
-> 195 (4), 190 (6), 171 (20); within 1.5 mm 58 -> 37 (6), 15 (20).
Charging pairs up to 2.6 mm too (the SOT-23's 1.9 and 2.1 mm, 2.54 mm
rows: most crossings left, 116 of 193) at 2 mm: crossings 211 -> 154
but unconnected 629 -> 675 and passes 18 -> 17; not taken.

**Worst boards** (route mode on the designer's placement, 1500 s of
work, 4 x 2, x1640 at 0 / 2 / 6 mm; the ten largest crossing excesses of
build/github-layout-v19 over the designers' and MokyaLora, MIDAS-MK2.1;
neotron-pico (27 GB), ULK (25 GB) and 0xCB's pcb-panel (KiCad timeouts)
left out):

| board | crossings 0 / 2 / 6 (designer) | unconnected | new errors | vias | length (mm) | bends per connection |
|---|---|---|---|---|---|---|
| glasgow revC3 | 66 / 57 / 50 (33) | 55 / 55 / 58 | 0 / 0 / 0 | 234 / 239 / 255 | 4314 / 3906 / 3838 | 1.259 / 1.256 / 1.226 |
| sbc_allwinner_a13 module | 33 / 25 / 21 (19) | 1 / 1 / 3 | 1 / 2 / 2 | 307 / 303 / 287 | 5191 / 5480 / 5455 | 2.752 / 2.100 / 2.115 |
| MokyaLora | 39 / 46 / 34 (24) | 0 / 0 / 0 | 0 / 0 / 0 | 225 / 239 / 234 | 5351 / 5794 / 5253 | 1.375 / 1.173 / 1.316 |
| MIDAS-MK1.1-revA | 20 / 13 / 15 (4) | 5 / 5 / 5 | 0 / 0 / 0 | 131 / 134 / 135 | 3034 / 3069 / 3000 | 0.900 / 0.903 / 0.938 |
| MIDAS-MK2.1-revA | 21 / 16 / 12 (5) | 0 / 0 / 0 | 0 / 0 / 0 | 132 / 177 / 189 | 4088 / 3075 / 3106 | 1.248 / 1.148 / 1.144 |
| MIDAS-MK2 | 23 / 24 / 10 (5) | 0 / 0 / 0 | 0 / 0 / 0 | 140 / 170 / 146 | 4051 / 3420 / 3900 | 1.247 / 1.319 / 1.162 |
| eternal-keypad | 100 / 100 / 100 (36) | 0 / 0 / 0 | 200 / 200 / 200 | 62 / 62 / 62 | 3784 / 3784 / 3784 | 0.939 / 0.939 / 0.939 |
| framework_mobo_lefthalf | 9 / 9 / 9 (0) | 0 / 0 / 0 | 0 / 0 / 0 | 71 / 71 / 71 | 1278 / 1278 / 1278 | 1.302 / 1.302 / 1.302 |
| laptop keyboard | 94 / 94 / 94 (14) | 3 / 3 / 3 | 0 / 0 / 0 | 161 / 161 / 161 | 7605 / 7605 / 7605 | 2.135 / 2.135 / 2.135 |
| SNSP-CPU-01 | 32 / 19 / 20 (2) | 6 / 11 / 22 | 0 / 0 / 0 | 1367 / 1418 / 1214 | 24801 / 24720 / 23702 | 3.944 / 4.013 / 3.520 |

Over the ten: crossings 437 -> 403 -> 365 (designer 142; median 32.5
-> 24.5 -> 20.5), unconnected 70 -> 75 -> 91 (SNSP-CPU-01, which no
budget completes: 6 -> 11 -> 22), new errors 201 -> 202 -> 202, passes
4 at each, vias 2830 -> 2974 -> 2754, length 63495 -> 62130 -> 60920
mm, bends per connection median 1.281 -> 1.279 -> 1.264.
video (at 0 and 2 only, on x1643: the x1640 snapshot was removed from
build/bin while its run waited for memory; x1643 at 0 routes video as
x1618): crossings 15 -> 7 (designer 4), unconnected 0 -> 1 (a pass
lost: the attempt at 2 used the whole budget, 681 -> 1569 s of work),
vias 603 -> 634, length 35157 -> 35357 mm, bends per connection 2.198
-> 2.243. So at the task budget the cost takes a tenth of the
crossings and costs completeness on two of eleven boards (SNSP-CPU-01,
video), where the 30 s tier gained.

The ladder's rung differs on several (MIDAS-MK2 and MK2.1: pours as
tracks at 0, connected at 2: the vias and length move with the rung,
not the gaps). Unchanged at every cost: eternal-keypad, keyboard and
framework_mobo_lefthalf, whose crossings are all of wider pairs
(switch-diode and socket rows; eternal-keypad's 200 new errors are the
same at 0). module's second items_not_allowed is a KiCad teardrop of a
GND track in AE1's keepout (the first, at 0, the same kind).

**Fingerprints** (Castor, k30, jetson, Sisu; route mode from v16, 1 x 4,
budget scale 0.25, x1642 at 0 and 2): Castor (2 crossings), k30 (1) and
Sisu (49) identical: their crossings are of wider pairs but k30's one,
which the cost does not move; jetson changes from before resolve on:
crossings 9 -> 4, vias 607 -> 589, length 10889 -> 10809 mm, bends per
connection 2.215 -> 2.125, unconnected 17 -> 20 (an incomplete board at
a quarter budget).

**Default 2 mm**: the largest cost whose tier loss is nil (3 mm loses
two thermals); the worst boards' completeness (SNSP-CPU-01, video) is
the case against it, and `pin_row_cost_mm` 0 or `PCB_ROUTER_PIN_ROW_COST=0`
routes as before. It takes a fifth of the charged crossings; most of the
excess against the designers is in wider pairs and on boards the cost
does not touch. Open: the wider pairs at a cost that leaves the
thermal guards their priority (a guard cost per pin-row cost, or
exempting pour nets' tracks), and pin swaps.
**Tried and not taken: the cost only while the attempts are complete**
(branch pin-row-conditional, x1645): once the probe leader or an
attempt leaves connections open, the ladder's later attempts and the
continuation route at 0. Expansion tier (x1645 at 0 against it):
unconnected 625 -> 609 (plain 2: 592), passes 19 -> 20, new errors 3
-> 6 (ZSWatch: 3 starved thermals), crossings 211 -> 206 (plain 2:
197). SNSP-CPU-01 (no probes; its first rung at 2, the rest at 0): 4
unconnected (6 at 0, 11 at plain 2), crossings 33 (32 at 0). video: 1
unconnected and 7 crossings, as at plain 2: its probes run at 2 and
the continuation goes on from the probe's routing, so turning the cost
off there does not give back the connection. Fingerprints at 0
identical. It keeps half the tier's gain and adds errors, so plain 2
stays, SNSP-CPU-01 and video noted.
`bash scripts/check.sh --fast` and the clippy gate; the fixture goldens
did not move (no fixture route crosses a charged gap).

## Measured on 2026-10-23 (Freerouting and KiCadRoutingTools, complete)

The two head-to-heads of 2026-10-21 finished: every one of the 109 v7
placements routed and judged on both sides, and the speed studies at 1
and 16 threads. Our side stays x1367's route-mode rows
(`build/fr2-route-final.log`) for comparability with the walls the other
routers were given; main has moved on since (x1573 and later: the memory
ledger, pour integrity, silkscreen placement, router memory, trace
aesthetics, the trouble repair default; the sections above). Details and
per-board tables: [benchmarks.md](benchmarks.md#freerouting-against-x1367-2026-10-21)
and [benchmarks.md](benchmarks.md#kicadroutingtools-against-x1367-2026-10-21).
Measurement and docs only, branch autorouter-comparisons; no code changed.

- **Freerouting 2.2.4**, given our wall per board: ours 107 results, 51
  pass, 44 strict; Freerouting 75 results, 3 pass and 3 strict
  (0xcb-static, OM-60-Flex, Blue_Line, all three strict passes of ours
  too). Better: ours 101, Freerouting 4 (Business-Cards Batch_1, katia
  where ours times out, jiran-ble-lite, SNSP-CPU-01), 4 ties. On the 74
  boards both finish: KiCad unconnected 2 700 against 9 619, counting
  errors 141 against 3 016, at a median 1.31 times our CPU per board.
  Freerouting has no result on 34: 25 it did not stop within its job
  timeout plus 300 s, 5 sessions KiCad cannot import, 3 boards it cannot
  read, the pcb-panel whose KiCad check times out.
- **KiCadRoutingTools cddf4cd**, capped at one to three times our wall: 66 results (36
  finished, 30 cut after writing its board), 43 cut before writing one;
  7 pass, 5 strict, all seven passed by ours too. Better: ours 93, it 6
  (d20_pcb with 199 solder mask bridges, WLED_Matrix, jiran-ble-lite, the
  0xCB panel, Neotron-Pico, glasgow revC3), 10 ties. On the 66 boards
  both have: unconnected 1 778 against 3 296, copper errors 69 against
  3 661, vias 10 985 against 25 200.
- **Speed.** Freerouting on ten boards with a 1500 s job timeout stops
  early (its wall a median 0.53 times ours at 1 thread) but leaves
  connections open on eight of them and has no result on coco2, at 1.20
  times our CPU; `-mt 16` changes neither its results nor its time
  (x1.05 CPU), and 16 threads did not speed ours up either (x1.05).
  KiCadRoutingTools on its ten, run to completion: over the seven it
  finished a median 4.6 times our CPU at 1 thread and 3.6 times at 16;
  the other three were cut at 3600 s.
- **Running it.** Freerouting's last 69 boards ran three at a time under
  the tier lock overnight; KiCadRoutingTools' 50 waiting boards had been
  judged under it by the evening of 2026-10-22. The Freerouting speed runs and our 16-thread runs on
  KiCadRoutingTools' ten ran without the lock, admitted by the memory
  ledger (`--min-free-gb 10`); Freerouting's JVM was reserved in the
  ledger by a wrapper around `freerouting.freeroute` (4.5 GB while it
  runs), and the time a run waited for the ledger is left out of its
  wall.
## Committed on 2026-10-23 (task labels)

The tasks stack their movable parts at one point, and parts sharing an
origin claim no mark (docs/placer.md, "The board's own texts and
silkscreen graphics"), so the designers' labels of those parts (a
connector's pin names, a chip's function) stayed at the designers' spots
as obstacles for parts the task had taken away from there ("fence report:
placer side": OpenFC's 17 connectors and headers seated on their stale
pin names). Branch task-labels; harness only, binary x1652.

**What changed** (run.py's `unplace`, `labels_of_movable`): before
stacking, the board texts and text boxes on F.SilkS, B.SilkS, F.Cu and
B.Cu and the silkscreen graphic clusters (within 0.5 mm; not one spanning
0.8 of the outline) whose box overlaps a movable footprint's
courtyard-and-pads box at the designer's position on its side, or its
holes and through pads from the other side, are dropped from the task
board (the binary's attachment rule; text boxes estimated from the font
size, pcbnew's widths 0.54 to 1.2 of the size per glyph, taken as 0.95).
Moving them with the part instead (translating by the stack offset)
attaches nothing: the stack's parts share an origin, so the labels would
block the stack's spot. Texts over fixed parts (MIDAS-MK1.1's 14 terminal
labels of the locked J101 and J102, which the binary still attaches) and
over free space stay. Rows record `labels: "dropped"` and
`labels_dropped`, summary.json their total. The committed task lists
(`tasks/v7`) did not change and nor do `task_sha256` and `board_sha256`:
the strip happens at run time. Over v7's 109 tasks 77 drop labels, 2824
in all.

**Placement alone** (`place-kicad-board` on the prepared boards, x1652,
the reference work rate, two threads; parts on marks counts entries of
`parts_on_marks`, a part once per mark):

| board | labels dropped | placed | parts on marks | HPWL mm | level (halo, grid) | work s |
| --- | --- | --- | --- | --- | --- | --- |
| OpenFC | 84 | complete | 17 -> 1 | 829 -> 1053 | (0, 0.1) -> (0.5, 0.635) | 332 -> 25 |
| MIDAS-MK1.1 | 10 | complete | 0 -> 0 | 2312 -> 2440 | (0, 0.1) both | 281 -> 219 |
| SmartSpin2k panel | 153 | complete | 9 -> 16 | 16681 -> 15741 | (0.5, 0.635) both | 189 -> 144 |
| OpenRX panel | 21 | complete | 0 -> 11 | 1076 -> 1727 | (0, 0.1) -> (0, 0.635) | 600 -> 509 |
| molecule 5x4_wave | 306 | J4 refused -> J3 and J4 refused | 56 -> 11 | 395 -> 390 | (0, 0.1) both | 581 -> 514 |

OpenFC now places at the halo level its marks-off placement takes (HPWL
1000 there): the 17 parts are off the marks, the larger HPWL is the halos'
room. OpenRX's panel finishes a level earlier (grid 0.635 instead of 0.1)
with 11 parts seated on the remaining marks (`seat_on_marks`), which costs
it 60 % HPWL: the placer's ladder, not the labels. SmartSpin2k's 16
entries lie on marks that stay (420 of its 573: over its fixed parts, such
as U1's pin names in every copy, or over free space). The 5x4_wave still
refuses: the marks left at J3's spot lie on the back (J3 has no through
pads) or beside its courtyard (the front pin graphics 0.4 mm off it), so
by the rule they are not J3's and stay; J3 as well as J4 now goes without
a place.

**Short tier** (3 x 2, x1652; build/labels1-short-before-x1652 with
main's run.py, build/labels1-short-after-x1652): passes 6 -> 6, strict 6
-> 6, KiCad unconnected 20 -> 17, vias 1184 -> 1180, length 21199 ->
21638 mm, first-route open terminals 58 -> 40. The five boards without
dropped labels give identical rows (routed, vias, HPWL); every changed
row has labels dropped:

| board | labels dropped | routed | KiCad unconnected | vias | HPWL mm | parts on marks |
| --- | --- | --- | --- | --- | --- | --- |
| OpenFC | 84 | 80/82 -> 81/82 | 2 -> 1 | 192 -> 159 | 829 -> 866 | 17 -> 1 (halo 0 -> 0.5) |
| MIDAS-MK1.1 | 10 | 149/156 -> 151/156 | 8 -> 6 | 225 -> 226 | 2317 -> 2439 | 0 |
| framework_mobo | 2 | 71/71, passes | 0 | 106 -> 109 | 916 -> 910 | 1 -> 0 (halo 0.125 -> 0.25) |
| eternal-keypad | 2 | 65/65 | 0 (its shorting_items stays) | 29 -> 34 | 522 -> 526 | 0 |
| 1337-v4.0 | 1 | 47/47, passes | 0 | 58 -> 73 | 997 -> 999 | 0 |
| reCamera | 1 | 55/56 | 4 | 71 -> 74 | 212 -> 210 | 0 |
| spell_tome | 1 | 18/18, passes | 0 | 7 -> 9 | 166 -> 166 | 0 |

compare.py: better 2 (OpenFC, MIDAS-MK1.1), same 10, worse 0. The single
labels moved the placement a little (HPWL within 1 %) and the routes with
it.

## Committed on 2026-10-23 (placer refusals)

The v19 sweep (x1539) refused four boards v18 (x1315) placed:
OM-FlexGrid-Flex, SmartSpin2k's panel, OpenRX's panel and the molecule
5x4_wave. Branch placer-refusal; snapshots x1650 (the share gate alone)
and x1651. Details: docs/placer.md,
"Placer refusals since v18".

**Causes** (bisected with placement alone, `place-kicad-board` on the
tasks' prepared boards, snapshots x1315 to x1535 and builds of the
commits between x1534 and x1535). Neither the held-part unification nor
the free-dimension fixes caused these refusals:
- OM-FlexGrid-Flex: 6ebd4e4 (a held part's pads keep the copper-to-edge
  clearance on every side at the edge-copper levels). J1's window in its
  tab shrank to 0.25 mm, between the eviction scan's 0.5 mm steps.
- SmartSpin2k, OpenRX, 5x4_wave: 057b3a4 (the board's marks). The tasks
  stack their parts, so the designers' labels attach to nothing. On the
  panels the marks made the first levels too slow for the budget:
  SmartSpin2k's first level took all 600 s of work, 200 to 280 s of it
  in `seat_on_marks`' passes over every part. With `keep_board_marks:
  false`, main places all three.

**What changed** (pcb-placer):
- `evict_for` scans a part held at one edge again on the level's grid
  along that edge, flush against it, when the coarse scan found no spot
  (`scan_steps`, `constraints::flush_center`). Unit test
  `eviction_finds_a_held_part_s_narrow_window_in_a_tab`.
- The legaliser's ring search and the eviction's scans skip positions a
  part's regions exclude (`region_window`). Exact: only the work counted
  changes, only where parts have regions (panels whose copies have their
  own nets).
- `seat_on_marks` skips legalizing every part again only where both an
  eighth of the movable parts or more failed (OpenRX: 55 of 216 at full
  halos) and the four passes would take more than an eighth of the
  placement's budget. Either condition alone moved MIDAS-MK1.1's or
  reCamera's placement. A debug line says how many each step seated.

**Measured** (x1651):
- Placement alone at the reference work rate, against main:
  - OM-FlexGrid-Flex: J1 without a place -> complete.
  - SmartSpin2k: 12 parts -> complete (U2 twice and U5 seven times on
    marks).
  - OpenRX: 81 parts -> complete.
  - 5x4_wave: still refused (J4, or J3 and J4 by seed). Its connectors
    fill 70 of 76 mm of each side edge under their own pin marks.
    Seating them with every mark yielding, by eviction or by legalizing
    every part again, found no slot either, so that code is not in.
- Layouts at the task budget (build/refuse1-recovered-x1651), against
  v18:
  - OM-FlexGrid-Flex: 19/19, passes strict, 93 vias (v18 100).
  - OpenRX: 50/127 routed (v18 47/127), 217 unconnected (226).
  - SmartSpin2k: 538/585 (v18 553/585), 54 unconnected (39), 1757 vias
    (1139). It now has 630 parts around the panel's marks.
- All 109 v7 tasks placed alone: 62 on a first build (the finer eviction
  only) and 49 on x1651 (the rest, the panels and the molecules). The
  only refusals left among boards v18 placed are 5x4_wave. OpenESC's 4in1
  panel and the four other molecules were refused by v18 too (4in1 now
  12 parts, v19 42, v18 216).
- Short tier (3 x 2, build/refuse1-short-x1651) against
  build/gplace1-short-off (x1630) and build/silk1-short-x1552: every
  placed board is byte-identical to x1630's. The rows are the same
  except 1337-v4.0's vias, 61 -> 58 on the identical placement, which is
  main's routing since x1630 (the trouble repair). 6 of 12 pass, strict
  6, unconnected 21 (x1630: 21).
- x1650, which skipped the passes on the share of failures alone, moved
  reCamera's placement in the tier (55/56 -> 54/56). The rule now also
  needs costly passes.

## Committed on 2026-10-23 (trouble repair: victims and order)

The trouble-makers' repair (router.md, item 7) refuses less where a
large net is in trouble. Branch trouble-victims; snapshots x1680
(before: the repair's default merge, routing identical to main's x1660:
pcb-router unchanged, the expansion tier identical row for row), x1681
(the knobs below, all off; since gone from build/bin), x1682 (the new
defaults).

- **The victims' limit by the trouble-maker's size**
  (`PCB_ROUTER_TROUBLE_PER_OPEN`, 2): an exchange may displace 12 nets
  or twice the trouble-maker's open terminals, whichever is more; the
  acceptance stays "the open terminals of all of them drop". What it
  lets through are supply nets (on the v19 placements; the open
  terminals of all the nets involved, before and after, and the nets
  displaced): video's GND 283 to 46 (32) and +5V 36 to 24 (21),
  OpenESC's GND 72 to 63 (17), MegaDrive's VCC 57 to 35 (26) and VDDF
  9 to 4 (15), SNSP-CPU-01's /RESOUT1 9 to 5 (13).
- **Ties** (`PCB_ROUTER_TROUBLE_TIES`, on): a trade that closes as
  many terminals as it opens is kept when fewer of the nets involved
  stay open afterwards (open terminals, then open nets, fall with every
  kept exchange: no cycle). Rare: on A's nine gap-board runs 8 ties
  kept, 269 refused (241 leave as many nets open, 28 more).
- **Rounds**: already there (three rounds while a round keeps an
  exchange; a refused net is tried again only after an exchange was
  kept). A tie kept now counts as a change of the board. The work bound
  (half the attempt's negotiation) is unchanged and binds on most gap
  boards ("out of work" in round 0 or 1).
- **The order** (`PCB_ROUTER_TROUBLE_ORDER`): most open terminals first
  (`open`, kept) against `few`, a survey first (each trouble-maker's
  priced search alone, undone; refused there, it is not tried in the
  round) and the fewest victims per terminal closed first. `few` loses
  on the v19 placements (710 against 702; better only on SNSP-CPU-01)
  and ties on the designer's; it wins a little on the expansion tier
  (625 to 623, Miniboard 6 to 4) and on jetson's fingerprint (17 to
  15). Kept as an experiment, off: the gap boards are the measurement.

**Route mode, the v19 placements** (`--route-from
build/github-layout-v19`, the route-mode budget, 1 thread a board, 5
jobs beside four other agents). Cells: KiCad unconnected / new strict
errors / vias / work seconds. A = x1681 with `PER_OPEN=2 TIES=1` (the
new default), B = A with `ORDER=few`. Logs
`build/trouble3-v19route-{x1680,x1681-A,x1681-B}.log`.

| Board | x1680 (before) | A (x1681; x1682 by default) | B (A, `ORDER=few`) |
| --- | --- | --- | --- |
| video | 157 / 0 / 1172 / 1261.0 | 130 / 0 / 1202 / 1262.6 | 139 / 0 / 1131 / 1260.9 |
| SNSP-CPU-01 | 128 / 0 / 1314 / 957.7 | 127 / 0 / 1363 / 939.0 | 120 / 0 / 1381 / 976.8 |
| OpenESC 4in1 | 108 / 0 / 156 / 1265.0 | 102 / 0 / 155 / 1264.1 | 106 / 0 / 151 / 1264.0 |
| Glasgow | 168 / 0 / 294 / 1230.1 | 167 / 0 / 296 / 1230.4 | 168 / 0 / 291 / 1228.3 |
| MegaDrive | 184 / 0 / 1367 / 958.2 | 176 / 0 / 1316 / 937.7 | 177 / 0 / 1249 / 973.6 |
| sum | 745 | 702 | 710 |
| vias, work in all | 4303, 5672 s | 4332, 5634 s | 4203, 5704 s |

**Route mode, the designer's placement** (`--tier designer`, 1500 s of
work). Logs `build/trouble3-designer-*.log`.

| Board | x1680 (before) | A | B |
| --- | --- | --- | --- |
| video | 0 / 0 / 603 / 681.2 | 0 / 0 / 603 / 681.2 (*) | not run |
| SNSP-CPU-01 | 4 / 0 / 1339 / 1108.2 | 4 / 0 / 1339 / 1109.5 | 4 / 0 / 1339 / 1113.2 |
| OpenESC 4in1 | 0 / 0 / 111 / 281.2 | 0 / 0 / 111 / 281.2 | 0 / 0 / 111 / 281.2 |
| Glasgow | 49 / 0 / 240 / 1575.1 | 49 / 0 / 240 / 1575.1 | 49 / 0 / 240 / 1575.6 |
| MegaDrive | 59 / 0 / 1351 / 1432.8 | 58 / 0 / 1311 / 1432.3 | 58 / 0 / 1236 / 1388.2 |
| sum | 112 | 111 | 111 |

(*) By construction, not measured: video's designer route is complete
before the repair (x1680 tried no exchange there), so the repair's
rules change nothing; A's run lost its binary and x1682's waited hours
for its 9.9 GB in the memory ledger.

**Expansion tier** (46 boards, route mode, 8 x 1, 30 s): x1682 (and
x1681 with A) identical to x1680 on all 46 rows (unconnected 625,
passes 19, new errors 3, vias 4223, length 112254 mm); against the
default-on reference build/t2-expansion-x1589 627 to 625 (the
difference is main's straightening since, x1680 alike). B: 623
(Miniboard 6 to 4, facade60 daughterboard +16 vias, MAS500 +1.5 mm),
passes and errors the same.

**Fingerprints** (Castor, k30-SBC, jetson, Sisu, route mode from v16, 1
x 4): A byte-identical to x1680 on all four (the larger limit and the
ties change no exchange there). B: jetson 17 to 15
unconnected (607 to 604 vias), Sisu's second ladder attempt different
(the survey spends the first attempt's repair work differently: 11 s
against 19 s) and its kept board the same (23, 379 vias), Castor and
k30 identical.

x1682 (the defaults) reproduces x1681 with A exactly: the expansion
tier row for row, OpenESC and SNSP-CPU-01 on v19 (counts, vias and
work to the tenth of a second). The fixture goldens do not move
(check.sh --fast). Vias: A +0.7 % on v19 (video +30, SNSP-CPU-01 +49,
MegaDrive -51), designer MegaDrive -40.

What is left: the work bound binds (most gap-board attempts run out of
it in round 0 or 1), and Glasgow's kept v19 attempt connects through
its pours (the other four route them as tracks): nearly every exchange
there meets a pour net (437 of B's 456 survey refusals). Next: a cheaper exchange (the victims' hard
routes are most of the work) rather than more refusals lifted.

## Committed on 2026-10-23 (trouble repair default)

The trouble-makers' repair of the experiment of 2026-10-22
(docs/experiments/human-level-routing.md) is on by default: router
`Config::trouble_repair` (true), `trouble_repair` in the router
settings (router.json; unset, the router's default), and
`PCB_ROUTER_TROUBLE` the override either way (`0` off, anything else
on). Snapshots x1586-x1589 (x1589 the default, on main 93d6f6e, the
memory round 2). Branch trouble-default.

- **Where it runs.** After the hard repair of every finish, but not in a
  layout's move trial (`Router::reroute` without the polish,
  `in_trial`). With it in every trial (x1585, the experiment's rule)
  OpenFC's layout on the short tier fit 5 move trials into the moves'
  work instead of 11 and ended at 12 unconnected instead of 2; run only
  where the board is polished (x1587) it lost video's layout gain (199
  both: a board far from complete gets no polish and no moves, only its
  unpolished first route). The first route and the race's routes keep
  it.
- **Expansion tier** (46 boards, route mode, 8 x 1, 30 s, x1585 with the
  switch against build/expansion-mem1-x1601, the same routing; x1587
  and x1589 by default give the same 46 rows): unconnected 631 to 627,
  passes 18 to 19, new errors 3 to 3, vias 4204 to 4216, starved
  thermals unchanged on every row; better 4, worse 1, the rest
  identical in result (12 rows differ at all):

  | Board | routed | KiCad unconnected | vias | pass |
  | --- | --- | --- | --- | --- |
  | bread-modular noise | 20/21 -> 21/21 | 1 -> 0 | 18 -> 12 | no -> **yes** |
  | ddB0515 ED052TC4-Adapter | 34/37 -> 36/37 | 4 -> 2 | 57 -> 59 | no |
  | OpenDeck r3.1.2 | 295/304 -> 296/304 | 10 -> 9 | 277 -> 282 | no |
  | facade60 daughterboard | 27/47 | 25 -> 24 | 64 -> 63 | no |
  | siderakb Miniboard | 35/39 -> 34/39 | 5 -> 6 | 79 -> 73 | no |
  | seven more (ZSWatch, MusicBrain, tuxkeychain, flexypin, ausign, MAS500, rv003usb) | same | same | +1 to +6 | same |

  CPU 1664 to 2277 s, but the identical rows rose alike (+37 %: load 9
  against 19 when they ran); work 1221 to 1242 s.
- **Short tier** (12 layouts, 3 x 2; on and off from one binary): x1589
  by default against x1585 with `PCB_ROUTER_TROUBLE` unset (off then):
  all 12 boards identical (routed, vias, length, KiCad's counts), passes
  6 and 6, unconnected 21 and 21, new errors 3 and 3. (x1585 with the
  repair in the move trials too: OpenFC 2 to 12, the rest identical;
  the reason for the trial rule above.)
- **The five gap boards' layouts** (x1592 off against x1589 on, the
  placements byte-identical): video 199 to 185, SNSP-CPU-01 147 to 134,
  OpenESC 4in1 127 to 120, Glasgow 191 to 190, MegaDrive 215 to 154;
  879 to 783 in all, no new errors, vias +2 to +11 %. Route mode on them is as measured on 2026-10-22 (the
  ladder's attempts are polished: x1589 by default reproduces x1582
  with the switch on).
- **Fingerprints**: with `PCB_ROUTER_TROUBLE=0` Castor, k30-SBC, jetson
  and Sisu byte-identical to main (x1588 against x1603 and x1624); by
  default jetson 20 to 17 unconnected (597 to 607 vias), Sisu 25 to 23
  (375 to 379), Castor and k30 identical.
- **Pour nets in the repair** (`PCB_ROUTER_TROUBLE_POURS=1`, off): pour
  nets as trouble-makers and as victims, weighed by their pads off the
  pour's main piece (`open_measure`, the pour analysed after the
  exchange). OpenESC and Glasgow (route mode, both placements):
  identical to the repair without it. The boards' kept attempts route
  the pours as tracks, where the supplies are signal nets already; what
  refuses their exchanges is the victims' limit (+3V3 with 13-25
  victims, GND 21-23) and trades that close as many as they open.
- **Twelve victims or 24** (`PCB_ROUTER_TROUBLE_VICTIMS=24`, route mode
  on the v18 placements): OpenESC 104 to 94, Glasgow 148 to 139,
  SNSP-CPU-01 140 to 127, but video 214 to 218 and MegaDrive 135 to 145
  (on the designer's placement 59 to 62, SNSP-CPU-01 4 both); the
  expansion tier identical. Mixed: 12 stays. A limit by the
  trouble-maker's size (more for a supply net of many pads) is the next
  thing to try.
## Measured on 2026-10-22 (layout sweep v19, matched designer tier)

The first full measurement on one binary after the packages of 2026-10-21
and 22, and the README's numbers moved to it. Binary x1539 (main
af9d32a), tasks v7, `--quality`, pinned work rates, both runs through the
tier lock with 14 GB kept free
([benchmarks.md](benchmarks.md#layout-sweep-v19-2026-10-22)):

- **Layout sweep v19** (`build/github-layout-v19.log`, 10 x 2, 18399 s
  wall): **49 of 109 pass the strict rule** (52 the per-type rule), 1827
  unconnected, strict errors 1326, no row cut (katia and Batch_1 finish:
  38 and 234 open) and none killed, ten without a result. Against v18
  (x1315, two binaries; v18 re-judged under strict rule 3 with
  rejudge.py, no row changed: 42 / 46): better 28, same 53, worse 16; on
  the 99 boards with a result in both, unconnected 1948 -> 1827. Medians
  against the designer's: vias x0.71 -> x0.74, length x1.03 -> x1.08, open
  0.009 -> 0.000 (mean 0.284 -> 0.090).
- **Gained the strict pass**, 9: mackerel-30-proto, rosco's `kicad`, coco2
  and k30-SBC (starved thermals gone), silkscreen_pcb (copper to the edge
  gone), zpn_devboard, BAGEL-MK1.1-Avocado, Explorer and slab-pcb-left
  (their last connections routed). neotron-pico routes every net-tie pad
  now (24 open to 0; hole_clearance 2 keeps it failing). **Lost**, 2:
  OM-FlexGrid-Flex refuses ("1 parts without a legal place (J1)", J1 held
  by an edge, region or overhang constraint) and homebrew_486's GND zone
  is in two islands (one open).
- **No vias in SMD pads**: score-kicad-board counts 0 on all 99 boards
  with a result (v18: 1299 on 74 boards).
- **New refusals**: the OpenRX panel (81 parts without a legal place; 226
  open in v18), SmartSpin2k_Panelized (12; 39 open), the molecule 5x4_wave
  panel ("J3, J4, silkscreen mark"; 499 open) and OM-FlexGrid-Flex; 764 of
  the 885 fewer unconnected in all rows are those boards.
- **The ground pour cut is not comparable** with v18: x1539's
  score-kicad-board gives the designer's own boards no cut on 79 of 94
  (x1315: 19 of 96; hackrf-one's source 76.95 mm under x1315, 0 under
  x1539), so the median "+2142 mm against the designer's" is a scorer
  change; to look into.
- **Matched designer tier** (`build/designer-x1539-matched.log`, budget
  scale 0.695 = 1042.5 s of work, 8 x 1; 11 rows from a first part on the
  same binary spliced in): 54 of 109 strict (61 per type), no row cut or
  killed, the 0xCB pcb-panel without a result (kicad-cli). Against
  x1404's matched run: better 17, same 90, worse 1, strict 51 -> 54.
- **Gap on one binary** (`compare.py --gap build/github-layout-v19.log
  build/designer-x1539-matched.log --targets`): over 97 target boards,
  layout open 1823, designer route 747: **placement 59 %, routing 41 %**
  (x1404 against v18: 49 / 51). v19's final ladder had a median share of
  0.640 (58 rows), below v18's 0.695, so the designer route had some 8 %
  more routing work than the layout's final ladder.
- The largest placement gaps (ours / designer route open): video 199 / 0,
  Glasgow 191 / 50, the OpenESC 4in1 127 / 0, MegaDrive 222 / 109,
  protoesp 106 / 1.
## Experiment 2026-10-23 (global route inside placement)

The placement half of the gap, one bounded experiment: the global route
(`pcb-global`) as a gradient inside the placer's search instead of a
seed ranker. Write-up: docs/global-route.md, "Phase 2". Branch
global-route-placement; snapshot x1630. **No-go as a default**; the
option stays, off.

- **`global_guidance`** (placer config, off): the anneal re-evaluates the
  global route of its current placement (`board_placer/guidance.rs`: the
  poses written into the board, lowered as the layout routes it) every
  200 accepted moves or more, as long as the evaluations stay within 10 %
  of the anneal's work; between evaluations each net's wirelength is
  multiplied by one plus the mean excess (usage above half the
  boundaries' capacity) over the tiles of its box, and half of the moves
  draw their part by its attribution (the excess on the boundaries its
  nets cross). The coupled loop's moves rank the parts by the same
  attribution on the kept board. `pcb_placer::guidance` (the field, the
  draws), `AnnealConfig::guidance`; `Capacity::work` and
  `GlobalRoute::pops` count the global route's work deterministically.
  The plain overflow is no gradient (0 to 5 tracks on the anneal's
  placements), hence the threshold 0.5, the soft overflow phase 1
  validated.
- **Eleven dense boards** at the task budgets, x1630 off against on
  (`build/gplace1-dense-*`): KiCad unconnected 1179 to 1132 over the ten
  with a result (SmartSpin2k refused on both sides: main's placer
  state); better protoesp 106 to 56, Glasgow 191 to 154, DSP-ADAU1452 30
  to 9, the 0xCB panel 53 to 49, SNSP-CPU-01 147 to 145; worse video 199
  to 224, 4in1-mini 81 to 106, MegaDrive 215 to 225, split72_left 30 to
  37; 4in1 127 both. First-route open terminals 1712 to 1565 (8 of 10
  better), HPWL +1.9 %, vias +5.7 %, work -7 %. Strict passes 0 both.
- **Short tier** (3 x 2, `build/gplace1-short-*`): unconnected 21 both,
  strict passes 6 to 5 (framework_mobo_lefthalf 0 to 1 unconnected),
  first-route open 58 to 36, HPWL +10 % (MIDAS +49 %), work +10 %.
- **Reading:** the guided anneal lowers the excess while it runs and the
  first route opens fewer terminals, but the legaliser and refinement
  (wirelength only) and the race decide the legal placement, whose soft
  overflow is lower on four dense boards and higher on five; per-board
  shifts are of the size any perturbation of these boards gives (phase
  1: 17-19 from probing other seeds). Left untried: the term in the
  legaliser and refinement, halos from the field, the global placement's
  density.
- **Off reproduces main:** x1624's placer output (board-placer.json,
  timing fields aside) equals the off side's on all eleven dense boards
  and all twelve short-tier boards. Unit tests: the field's box means
  and weighted draws (`pcb_placer::guidance`), and an anneal whose
  guidance has no field is pose for pose the plain anneal while a hot
  field is evaluated within its work share (tests/placement.rs).

## Committed on 2026-10-23 (router memory, round 2)

The second round of the router's memory, the candidates round one named:
katia's maps 2189 -> 1408 MB, its peak 2536 -> 1763 MB, every route and
fingerprint the same. Branch router-memory-2; snapshots x1620 (the maps),
x1621 (the estimate and the lattice floor re-fitted), x1622 (no via
cover without pours), x1623 (rebased on the trouble-repair merge). Details:
docs/router.md, "Memory (2026-10-23)" and "Memory limits".

**What changed** (pcb-router unless named):
- The occupancy a byte a count (`Occupancy`, was 16 bits): a count is
  how many nets' stamps cover a cell; 255 or more keep 255 in the byte
  and the rest in a side map (exact).
  Nothing keeps a second copy. Not shared between classes of one trace
  geometry (katia's three neck classes apart in their vias only): their
  maps are the same, but sharing them changes the stamp lists, which the
  fingerprint hashes. Not sparse: a search step reads a count per
  neighbour.
- The search records (`hot`) a set per class of some net, shared by the
  classes of one static trace map; a neck class has none, and a step into
  a neck zone reads its net class's record with the neck's static map
  value (all the neck's record differed in). Katia's eight searched
  classes are four net classes and four necks: 730 -> 365 MB.
- The guard and cover maps in 16 bits (`owner16`, as the records hold
  them), with the lost-spokes step's copy of the guard; no via cover (a
  word a node) on a board without pours (jiran-ble-lite 55 MB).
- The estimate (pcb-kicad): the maps and 600 MB resident (was 1.2 x +
  540: jetson came 6 MB over before the margin), the maps and 760 MB of
  address space (was 1.3 x + 780), the 1.25 margin kept: what lies beside
  the maps hardly follows them (katia 355 MB, jetson 579). The lattice
  floor 24 + 4 a class bytes a node and layer (was 26 + 6, which came
  within 1.25 of 28 of the 67 lattices measured, the least 1.08): under
  all 68 by 1.28 or more (the least node_v6_0_0, 6 layers and 2 classes,
  41.0 bytes a node and layer; ICE40HX1K-TQ144 1.32).

**The consumers** (the scratch heap profiler of round one, not
committed: the live blocks of 64 KiB or more at the live peak, MB; before
is round one's after, x1600 to x1602, the same code as x1603; after is
x1620's code, before x1622 dropped the via cover on boards without
pours: jiran-ble-lite's guard/cover 66 holds its 55 MB):

| board | before | after |
|---|---|---|
| katia | occupancy 741, search records 730, static maps 347, scratch 147, stamp lists 128, guard/cover 106 | occupancy 370, search records 365, static maps 347, scratch 147, pour analysis (transient) 137, stamp lists 124, guard/cover 60 |
| jiran-ble-lite 0.05 mm | search records 422, occupancy 369, scratch 343, static maps 205, history 158, guard/cover 66 | scratch 343, search records 211, static maps 205, occupancy 184, history 158, guard/cover 66 |
| laptop motherboard | occupancy 222, search records 112, blocks under 64 KiB 94, static maps 90, scratch 88, guard/cover 62 | search records 112, occupancy 111, blocks under 64 KiB 105, static maps 90, scratch 88, guard/cover 34 |
| Castor | blocks under 64 KiB 86, occupancy 73, stamp lists 37, via snapshot 31, search records 30 | blocks under 64 KiB 90, stamp lists 37, occupancy 36, static maps 27, scratch 23, search records 15 |

**The seven boards** (one router at a time, one thread, a ladder of 60 s
of work without probes and seeds, x1603 against x1622; MB; the routes'
fingerprints the same on all seven):

| board | maps | peak RSS | VmPeak |
|---|---|---|---|
| katia | 2189 -> 1408 | 2536 -> 1763 | 2810 -> 2029 |
| jiran-ble-lite 0.05 mm | 1589 -> 1141 | 1594 -> 1212 | 1954 -> 1506 |
| laptop motherboard | 636 -> 497 | 894 -> 774 | 1114 -> 979 |
| Sisu_SSE-9 | 491 -> 379 | 725 -> 554 | 912 -> 736 |
| k30-SBC | 247 -> 194 | 411 -> 357 | 694 -> 598 |
| jetson | 223 -> 165 | 796 -> 744 | 1052 -> 903 |
| Castor | 193 -> 134 | 388 -> 328 | 579 -> 565 |

**Fingerprints** (Castor, k30, jetson, Sisu; route mode from v16, 1 x
4, budget scale 0.25, `PCB_ROUTER_FINGERPRINT=1 PCB_ROUTER_NO_WALL_GUARD=1`,
build/fp-mem2-x1623 against build/fp-mem1-x1601; x1620 the same): every
fingerprint and every board identical; peak RSS (KiCad's check included)
Castor 1219 -> 1073, k30 1833 -> 1637, jetson 1721 -> 1466, Sisu 2545 ->
2143.

**Expansion tier** (46 boards, route mode, 8 x 1, 30 s, `--min-free-gb
10 --no-lock`, build/expansion-mem2-x1623 against
build/expansion-mem1-x1601; x1620 and x1622 the same): every row's
routed, vias, length, KiCad counts, verdict and work identical and all
46 result boards byte-identical; unconnected 631 both, work 1221 s both.
Peak RSS per row median 235 -> 192.5 MB, max 1297 -> 823 (MSign), sum
16.5 -> 13.0 GB; per row 0.63 to 1.02 of before (median 0.81; the small
boards' peak is KiCad's check): Wonkle 850 -> 701, brickpico 835 -> 621.

**Speed** (Castor, `bench-search` 10 iterations, one thread, pinned to
one core, x1603 and x1623 alternating (ABBA), ten runs each; the machine
loaded): user CPU median 16.56 against 16.74 s, sum 684.9 against 668.6,
least 14.51 against 14.44: no difference beyond the noise; 36 963 190
expansions both.

`bash scripts/check.sh --fast` and the clippy gate pass.
## Committed on 2026-10-23 (trace aesthetics)

The first complaint about autorouters (Hacker News, ours included by
implication) is traces that "snake through SMD pins" and bend "in many
small steps". pcb-maker had bend costs and a clean-up but no measurement
of either. Branch trace-aesthetics; snapshots x1610-x1618 (x1618 the
commit's routing).

**The measure** (score-kicad-board's `aesthetics`, crates/pcb-kicad/src/
aesthetics.rs; docs/quality.md, "Aesthetics"): per board the track paths
(chains of a net's segments on one layer between pads, vias, junctions
and ends), bends (direction changes of 5 degrees or more; turns of one
sense within 1 mm and at most 90 degrees one bend, so a corner cut at 45
degrees is one), bends per connection (over the netlist's connections,
the same for any routing), stair-steps (two bends within 2 mm back to the
first direction) and pin-row crossings (another net's track crossing the
line between two adjacent SMD pads of one part). quality.py lists them,
designer-numbers.json has the designer's, compare.py's normalised line
gives ours over (bends) or less (stair-steps, crossings) the designer's.
On the way: score-kicad-board read no net on a track of a KiCad 9 board
since 2026-10-21 (`(net 5)` is a code; now resolved by the board's net
table), so such boards had no paths and their pours no cut;
designer-numbers.json, written before that rule, had counted the pour
net's own tracks as cuts (59 of 109 designer ground cuts lower now, jetson
6792 to 6397 mm; vias and lengths the same).

**Baseline** (build/github-layout-v19, x1539, against the designers'
boards; 97 of the 109 boards, the rest unscored: 0xCB's panels and a few
others take minutes in score-kicad-board's existing measures, cut at 240
s): bends per connection ours median 2.32 (deciles 1.31 / 4.88), the
designers' 2.05 (0.91 / 3.94); ours over the designer's median x1.21, no
worse on 27 of 97; stair-steps per connection 0.47 against 0.18, no worse
on 10 of 97 (sum 44389 against 18642); pin-row crossings per connection
0.013 against 0.005, no worse on 44 of 97 (sum 1434 against 1517). The ten
worst by bends per connection over the designer's: DSP-ADAU1452 (x3.91),
MIDAS-MK2 (x3.62), mackerel-30-proto (x3.46), ecad-viewer video (x3.14),
MIDAS-MK2.1-revA (x3.12), BAGEL-MK1 (x2.97), mackerel-10-v1 (x2.93),
TelemetryOnboard (x2.72), MokyaLora (x2.70), neotron-pico (x2.56).

**Why the router makes them:** the search costs `bend_cost` per 45
degrees against the arrival direction (a direction change, not a per-step
cost) but closes a node at its first arrival whatever the direction, so
among the octilinear lattice's many shortest paths it keeps stepped ones;
the negotiation's detours leave jogs; the clean-up keeps a reroute only
when strictly shorter, and fewer bends are mostly as long.

**The straightening** (router/straighten.rs, `Config::straighten`, on;
`PCB_ROUTER_NO_STRAIGHTEN` turns it off; docs/router.md,
"Straightening"): after the via reduction and the thermal spoke repairs,
per complete signal net and layer run, the shortest octilinear path
between two corners (axis part first or diagonal first) where the run has
fewer bends, else fewer stair-steps, else less turning, every step free as
the hard search judges it; never longer, no via moves, branch ends and
junctions stay, no new node in another pad's thermal guard or spoke
corridor, no more pour cut; the pour nets' stranded pads and the spoke
model's starved pads counted before and after the pass, changed nets
undone last first while either grew. Eight step checks count as one
expansion. Tried on the way and dropped: the pour check per net (its pour
analyses moved the route ladder's shares: flexypin 30 to 33 vias),
straightening before the spoke repairs and without the guard rule
(BAGEL-MK1 153 to 157 vias: the repair moved 37 nets out of spoke
corridors where it had moved 16).

**Ten worst, route mode on the designer's placement at the task budget**
(1500 s of work, 4 x 2; before x1613 with the straightening off, which
routes as main; after x1618; neotron-pico not run, its 27 GB estimate was
never admitted beside the other agents' work):

| board | unconnected | new errors | vias | length (mm) | bends per connection (designer) | stair-steps (designer) | pin-row crossings (designer) |
|---|---|---|---|---|---|---|---|
| DSP-ADAU1452 | 4 -> 4 | 0 -> 0 | 79 -> 79 | 4735 -> 4732 | 1.017 -> 0.878 (0.965) | 138 -> 89 (63) | 2 -> 2 (0) |
| mackerel-10-v1 | 0 -> 0 | 0 -> 0 | 201 -> 201 | 22670 -> 22669 | 3.019 -> 2.730 (2.123) | 415 -> 269 (221) | 0 -> 0 (0) |
| MokyaLora | 0 -> 0 | 0 -> 0 | 225 -> 225 | 5363 -> 5351 | 1.541 -> 1.375 (1.046) | 265 -> 199 (116) | 39 -> 39 (24) |
| mackerel-30-proto | 0 -> 0 | 0 -> 0 | 301 -> 301 | 22137 -> 22125 | 4.829 -> 4.120 (2.202) | 938 -> 638 (240) | 0 -> 0 (0) |
| MIDAS-MK2.1-revA | 0 -> 0 | 0 -> 0 | 132 -> 132 | 4092 -> 4088 | 1.569 -> 1.248 (0.847) | 184 -> 63 (37) | 21 -> 21 (5) |
| BAGEL-MK1 | 0 -> 0 | 0 -> 0 | 153 -> 153 | 2680 -> 2673 | 1.609 -> 1.333 (0.829) | 151 -> 75 (45) | 0 -> 0 (0) |
| MIDAS-MK2 | 0 -> 0 | 0 -> 0 | 140 -> 140 | 4056 -> 4051 | 1.546 -> 1.247 (0.846) | 171 -> 63 (35) | 23 -> 23 (5) |
| TelemetryOnboard | 0 -> 0 | 0 -> 0 | 76 -> 76 | 3793 -> 3791 | 1.549 -> 1.277 (0.789) | 92 -> 39 (4) | 0 -> 0 (0) |
| video | 0 -> 0 | 0 -> 0 | 603 -> 603 | 35187 -> 35156 | 2.705 -> 2.198 (3.383) | 820 -> 409 (1238) | 15 -> 15 (4) |

Medians: bends per connection 1.569 -> 1.333 (designer 0.965),
stair-steps 184 -> 89 (63); unconnected, errors, vias and passes the same,
length 104713 -> 104637 mm. Pin-row crossings do not move: the
straightening keeps a path's layer runs between their corners.

**Expansion tier** (46 boards, route mode, 8 x 1, 30 s, x1618 against
build/expansion-probe2-x1542.log): unconnected 631 -> 629, new errors 3
-> 3 (strict 3), passes 18 -> 18 (strict 18), vias 4204 -> 4208, length
112361 -> 112146 mm, work 1221 -> 1228 s. Rows changed: MSign 21 -> 19
unconnected, 251 -> 252 vias; Miniboard 79 -> 82 vias, unconnected 5
both (the straightening's work in the first attempt moved the
narrow-signal step's deadline 10.51 -> 10.35 s: 14 -> 16 router-open
there). Over the 45 scored boards: bends per connection median 1.592 ->
1.419 (designer 1.628), ours over the designer's x1.026 -> x0.881;
stair-steps median 47 -> 30 (designer 27), sum 4257 -> 2695 (2496); bends
per board 0.74 to 0.98 of before (median 0.86); pin-row crossings 183 ->
183.

**Fingerprints** (Castor, k30, jetson, Sisu; route mode from v16, 1 x 4,
budget scale 0.25, against build/fp-probe2-x1542): every router's
fingerprints before resolve, after the clean-up, after the via reduction
and after the thermal spokes identical; the straightening changes them
after. Castor 241 -> 240 vias, length 5297 -> 5287; k30 344 -> 343 vias,
length 14379 -> 14369, KiCad's starved thermal on J1 moved from pad 14
(the designer's, subtracted) to pad 39 beside it (counted new; the spoke
model finds no more starved pads, the change comes from KiCad's refill in
tracks mode); jetson 597 vias both, length 10782 -> 10773; Sisu 375 vias,
25 unconnected both, length 5493 -> 5491. Straightened per board: 50,
116, 130 and 89 + 79 nets; bends 921 -> 705, 2046 -> 1685, 1746 -> 1386.

**Open:** incomplete nets are not straightened (jetson: 130 nets);
pin-row crossings need the search (a cost for passing between a part's
adjacent pads); any-angle or arc output is still out of scope.
`bash scripts/check.sh --fast` and the clippy gate; f1's golden
fingerprints gain the straightening's line, the others unchanged.

## Experiment 2026-10-22 (routes and placements closer to humans)

Florian's brief: what would get the routes and placements closer to a
designer's, with five ideas to assess (trouble-makers first, moves that
keep traces, zero-width capacity first, buses as bundles, pin swaps).
Write-up: docs/experiments/human-level-routing.md. Branch
human-level-routing; snapshots x1580-x1584.

- **The failure data.** On the five gap boards the open nets are open
  whole (a ten-pad bus net with nine pads open): the hard repair rips
  the conflicted nets and routes them again in the routing order, which
  puts the multi-pad buses after every short net. Bus nets (four or
  more nets joining the same footprints) hold 80 % of video's open
  terminals on our placement and 64-75 % of MegaDrive's on either; OpenESC
  and Glasgow are open on supply and motor nets.
- **The assessment.** Moves that keep traces exist (`Router::update`, the
  coupled loop) and do not run on boards far from complete, where a
  bounded round measured worse on 2026-10-21; zero-width capacity exists
  twice (the tile planner, `pcb-global`) and is a placement lever, not a
  routing one; pin swaps exist and do not apply to the gap boards'
  netlists; bus bundles are the largest share and weeks of work. Chosen:
  the trouble-makers, as a trade the router did not make.
- **The trouble-makers' repair** (`crates/pcb-router/src/router/trouble.rs`,
  `PCB_ROUTER_TROUBLE`, off; its knobs `_PRESENT`, `_SHARE`, `_ROUNDS`,
  `_VICTIMS`, `_DEPTH`, `_WHY` in the environment index). After the hard
  repair each open net, most open first, is routed through other copper
  at a price; the nets it overlaps lose the crossed branches and are
  routed again hard; the exchange is kept when the open terminals of all
  of them drop, else restored exactly. Up to half of the attempt's
  negotiation in work. `force_connect` kept an exchange only when every
  net ended complete; this one trades.
- **Measured** (route mode, run.py; x1543 against x1582 with the
  switch): designer placements 136 to 112 KiCad unconnected (SNSP-CPU-01
  6 to 4, Glasgow 55 to 49, MegaDrive 75 to 59; video and OpenESC
  complete and unchanged), v18 placements 841 to 741 (video 235 to 214,
  SNSP-CPU-01 158 to 140, OpenESC 112 to 104, Glasgow 159 to 148,
  MegaDrive 177 to 135); x1560 against x1580 the same picture (134 to
  112, 825 to 738). Layouts of the five (x1592 against x1582, identical
  placements): 879 to 777 (video 199 to 185, SNSP-CPU-01 147 to 128,
  OpenESC 127 to 120, Glasgow 191 to 190, MegaDrive 215 to 154). No new
  errors, no row cut; vias +11 % on MegaDrive, +29 % on video's v18
  route. Fingerprints with the switch off byte-identical on Castor,
  k30-SBC, jetson and Sisu (x1543/x1581, x1543/x1582, x1573/x1583, x1603/x1584);
  with it on jetson 20 to 17, Sisu 25 to 23.
- **One long negotiation** (configuration only, `negotiation_seconds`
  900 on the designer placements): MegaDrive's conflicts plateau at
  118-150 at the price cap for 34 iterations, 85 unconnected against 75;
  SNSP-CPU-01 7 against 6, Glasgow 53 against 55. No-go.
- **Recommendation**: go as the next default candidate, after the short
  and expansion tiers with the switch (complete boards unchanged so far;
  the risk is starved thermals and vias on nearly complete ones), then
  the sweep. Then: supply nets as trouble-makers (the refusals: more than
  twelve victims, a pour in the way), the global route in the placer's
  search for the placement half, bus bundles after a placement that
  lines the bus up.

## Committed on 2026-10-22 (router memory)

A router on a big board took 4 to 7 GB (katia 4.8 GB with one router),
which held how many boards, agents and rung probes run side by side.
Now katia peaks at 2.5 GB, the laptop motherboard at 0.9 instead of 1.7,
every route and fingerprint the same. Branch router-memory; snapshots
x1600 (the shrink and the estimate), x1601 (the own-stamp mask kept when
a pooled scratch first allocates), x1602 (rebased on the pour-integrity
package; the lattice floor). Details: docs/router.md, "Memory
(2026-10-22)" and "Memory limits".

**Measured first** (route mode, one thread, one router at a time: a
ladder of 60 s of work without probes and seeds, `PCB_WORK_RATE`
pinned; a scratch heap profiler, not committed, grouped the live blocks
of 64 KiB or more at the live peak by their source lines). Before (x1592,
the same routing as x1543), MB at the live peak:

| board (maps MB, peak RSS) | 1 | 2 | 3 | 4 | 5 |
|---|---|---|---|---|---|
| katia (3184, 4841) | search records 2018 (two sets: the lost-spokes step rebuilt them beside the first) | static maps 978 (trace 456, edge owner 228, edge block 114, free layers 91) | occupancy 741 | scratches 285 (the router's and the pool thread's) | stamp lists 233 |
| jiran-ble-lite 0.05 mm (2156, 2634) | scratches 685 | search records 633 | static maps 560 | occupancy 369 | history 158 |
| laptop motherboard (869, 1748) | allocator retention 546 (peak RSS 1748 for 1343 live: freed maps stayed resident; the pour measure on top) | static maps 270 | occupancy 222 | scratches 175 | search records 169 |
| Castor (264, 690) | via reduction snapshot 166 | allocator retention 119 | static maps 87 | stamp lists 79 | occupancy 73 |

After (x1600 to x1602):

| board (maps MB, peak RSS) | 1 | 2 | 3 | 4 | 5 |
|---|---|---|---|---|---|
| katia (2189, 2534) | occupancy 741 | search records 730 | static maps 347 | scratch 147 | stamp lists 128 |
| jiran-ble-lite 0.05 mm (1589, 1608) | search records 422 | occupancy 369 | scratch 343 | static maps 205 | history 158 |
| laptop motherboard (636, 897) | occupancy 222 | search records 112 | blocks under 64 KiB 94 | static maps 90 | scratch 88 |
| Castor (193, 392) | blocks under 64 KiB 86 | occupancy 73 | stamp lists 37 | via snapshot 31 | search records 30 |

**What changed** (pcb-router unless named):
- The search records (`Hot`) are 8 bytes, not 12: the static map's
  value in 16 bits and an index into a table of the board's (guard,
  cover) pairs (`hot_pours`; a pair beyond 65 534 reads the maps).
  `build_hot` writes them in place: the lost-spokes step rebuilt a full
  second set beside the first twice (katia's peak, 1.1 GB).
- The static trace map in 16 bits (`trace_at` widens it); the blocked
  unit steps sparse (`EdgeMap`: a bit a node, the 1 % of nodes with such
  a step in a hash map; were 3 bytes a node); `via_blocked` a bit a
  node; `free_layers` in the bytes the layers need (`LayerBits`).
- A net's stamps 4 bytes each (`Stamped`: map and cell in one word where
  the lattice's maps and cells fit, two words otherwise; same order).
- The search scratch: allocated on a router's first search (a router
  that never searches has none), and a parallel batch's jobs take the
  router's own and its spares (`ScratchPool`) instead of a thread-local
  scratch per pool thread, which stayed after the router; with one thread
  one scratch, not two. `Scratch::ensure` keeps the own-stamp mask set
  before the first allocation (x1600 lost it for a pooled scratch the pool
  had just created: caught by a router test before any run counted).
- The via reduction's snapshot leaves out the occupancy and the tiles'
  claims; a revert takes the changed nets' stamps off and the
  snapshot's back on (the counts are the sum of the nets' stamps).
- pcb-kicad: glibc's mmap threshold fixed at 1 MiB (`configure_allocator`,
  first in `main`): freed maps go back to the system (katia held 1.75
  GB after its first attempt with no router, 0.24 GB now; its peak 3.9
  to 3.0 GB from this alone, the search's time the same). The pour
  measure drops the source board's parse (jetson: 240 MB) and its own
  before it builds its router (jetson's peak was the measure: 1392 to
  797 MB).
- The estimate: 1.2 x maps + 540 MB resident (was 1.7 x + 460), 1.3 x
  + 780 MB of address space (was 2.0 x + 550), the 1.25 margin kept; the
  lattice floor 26 + 6 a class bytes a node and layer (was 32 + 6),
  under all 197 lattices measured by 1.25 or more (the least
  ICE40HX1K-TQ144 at 1.33). The ledger's reservations and the ladder's
  side-by-side counts read these.

Not done (the next candidates, by bytes): the occupancy (741 MB on
katia, 16-bit counts, 3L + 1 maps a class) and the search records (730:
one set per searched class, duplicated for classes of one geometry);
the guard and cover maps in 16 bits (92 MB on katia).

**The eight boards** (one router at a time, one thread, x1592 against
x1600; every fingerprint and board identical; jiran at 0.125 mm ran
four seeds side by side, identical as sets):

| board | maps MB | peak RSS MB | VmPeak MB |
|---|---|---|---|
| katia | 3184 -> 2189 | 4841 -> 2534 | 5128 -> 2809 |
| jiran-ble-lite 0.05 mm | 2156 -> 1589 | 2634 -> 1608 | 2994 -> 1954 |
| laptop motherboard | 869 -> 636 | 1748 -> 897 | 1944 -> 1112 |
| Sisu_SSE-9 | 694 -> 491 | 1424 -> 721 | 1605 -> 912 |
| k30-SBC | 319 -> 247 | 770 -> 406 | 977 -> 629 |
| jetson | 302 -> 223 | 1392 -> 796 | 1536 -> 1052 |
| Castor | 264 -> 193 | 690 -> 392 | 851 -> 571 |
| jiran-ble-lite 0.125 mm (4 seeds) | 473 -> 339 | 1295 -> 997 | 1904 -> 1409 |

The old estimate missed jetson (973 MB for a peak of 1392, before the
margin): the pour measure's second parse; the new line passes it by 12
MB. After x1602's rebase, katia and the laptop motherboard against
x1573: identical, 4804 -> 2544 and 1744 -> 897 MB.

**Fingerprints** (Castor, k30, jetson, Sisu; route mode from v16, 1 x
4, budget scale 0.25, `PCB_ROUTER_FINGERPRINT=1 PCB_ROUTER_NO_WALL_GUARD=1`,
build/fp-mem1-x1601 against build/fp-probe2-x1542): every fingerprint
and every board identical; peak RSS (KiCad's check included) Castor 1482
-> 1219, jetson 1770 -> 1721, Sisu 2870 -> 2545, k30 1829 -> 1833 (its
peak is KiCad's).

**Expansion tier** (46 boards, route mode, 8 x 1, 30 s, `--min-free-gb
10 --no-lock`, build/expansion-mem1-x1601 against
build/expansion-probe2-x1542): every row's routed, vias, length, KiCad
counts and verdict identical and all 46 result boards byte-identical;
18 passes both, unconnected 631 both, work 1221 s both. Peak RSS per
row (`peak_rss_mb`, `layout_rss_mb` the same here) median 368.5 -> 235
MB, max 1724 -> 1297 (MSign), sum 23.9 -> 16.4 GB; per row 0.47 to 0.90
of before (median 0.72): Multi_saturation_module 1356 -> 745, yuiop29re
888 -> 421, Wonkle 1283 -> 850.

**Speed** (Castor, `bench-search` 10 iterations, one thread, pinned to
one core, x1573 and x1602 alternating, four pairs; the machine loaded):
13.7 to 21.7 s against 13.3 to 17.8 s, x1602 the faster in every pair,
36 963 190 expansions both. The tier's CPU seconds 2383 -> 1664 (under
another load; work the same).

`bash scripts/check.sh --fast` and the clippy gate pass.

## Committed on 2026-10-22 (pour integrity)

Fence issue 17: keep a two-layer pour whole when the router can. The
diagnosis and the mechanism are kept, the default is not:
`pour_integrity` defaults to 0, main's exact code path (routing
byte-identical: F1's golden fingerprints unchanged, the expansion tier's
27 two-layer boards identical to x1542 with x1572). At 1 (measured
below) the 46 mm fence lost its completeness, the tier's pours were
mixed and F1's golden moved; the fence's back-layer crossings, not their
price, keep its pour in pieces. Branch pour-integrity; snapshots x1570
(at 1 on every layer count, before the rebase on ec60853), x1571 (at 1,
two layers only), x1572 (the commit, default 0). Before: x1539 (fence
layouts), x1542 (expansion tier, fingerprints).

**Why `plane_cut_cost` had no effect.** The rung that wins on the
fence's two layers is pours as tracks (46 mm, x1404: connected 19 open,
skeleton 10, tracks 0), and a pour not connected masked nothing:
`covered` came from the nets' connected pour masks, so `layer_cut` was 1
and the cut cost never entered that rung's steps, vias or corridors.
Where it did enter, it was one factor for every covered node, a nick at
the rim as dear as a cut through the corridor that joins two halves,
relative to the least covered layer (nothing on a board poured on both
sides). And the ladder ranked attempts by open, starved, vias, copper:
the pours never decided anything.

**`pour_integrity`** (router and route config, default 0, measured at 1; two
layers only), docs/router.md "Why `plane_cut_cost` did nothing on the
fence":
- pours not connected cover what KiCad would fill (masks computed for
  `covered`, counted in the build): the tracks rung pays the cut cost;
- what a cut severs (router/sever.rs, `refresh_sever`): the fill as the
  routes leave it, split into wide cores (opening by 0.6 mm) and corridor
  chunks of 1.2 mm; a chunk that is an articulation point of the graph of
  cores and chunks severs the parts outside the largest after its
  removal, and another net's step or via there pays `layer_cut` times
  `1 + 20 * pour_integrity * severed share`; refreshed after a batch's
  rip-up in the negotiation and the clean-up when copper changed, at most
  a tenth of the search since the last refresh, 30 ns a node as pour
  work; unit tests (a dumbbell's corridor severs half, a spur's root its
  spur, two corridors in parallel nothing);
- the ladder ranks equally open attempts (open, starved) by their pours
  before vias and copper: each judged attempt measured as the verdict's
  `pours` (`measure_pours`, charged as its lowering and build), its
  `pour_loss` (share outside the largest piece plus 0.01 a further
  piece, summed over the pours) in steps of 0.02 (`pour_rank`); the check
  gate checks an attempt as open as the best with better pours; the
  attempt line shows `pours lost`.

On every layer count first (x1570) the expansion tier lost 197
unconnected (761 to 958: MSign 69 to 137, ZSWatch 3 to 63, AMLing2 45 to
70, anthracite 1 to 22, the tracks rung now paying for its inner planes)
and the fingerprints jetson 20 to 85, Sisu 25 to 52, k30-SBC 344 to 912
vias: hence two layers only (`sever::integrity`, `ranks_pours`).

**Measured.** Fence layouts, GND on B.Cu (before x1539, after x1571;
x1570 the same on all three):

| board | complete | pieces | largest | copper | vias | length | cut by |
|---|---|---|---|---|---|---|---|
| 46 mm, before | yes | 9 | 170 mm², 5 % | 13 % (182 islands) | 265 | 4604 mm | 2043 mm |
| 46 mm, after | 1 unconnected | 12 | 621 mm², 18 % | 38 % (126 islands) | 338 | 4910 mm | 1778 mm |
| 44 mm, before | yes | 44 | 203 mm², 6 % | 37 % | 440 | 4764 mm | 1842 mm |
| 44 mm, after | yes | 50 | 202 mm², 6 % | 42 % | 423 | 4723 mm | 1737 mm |
| 40 mm, before | yes | 45 | 384 mm², 13 % | 39 % | 404 | 4415 mm | 1674 mm |
| 40 mm, after | yes | 48 | 191 mm², 6.5 % | 28 % | 500 | 4725 mm | 1863 mm |

46 mm with `pour_integrity` 4: 1 unconnected, 35 pieces, largest 9 %,
cut 2068 mm; with `plane_cut_cost` 30: 3 unconnected, 30 pieces, 9 %,
1933 mm. The goal (46 mm complete, well under 37 pieces, the largest
over half the zone) is not reached: the other nets' track on B.Cu stays
at 1700-2100 mm whatever its price (the board needs the back for its
crossings) and meshes the fill into 130-180 islands; the sever cost moves
where tracks cross, not how much of the back they take.

Expansion tier, route mode 30 s, 8 x 1, its 27 two-layer boards rerun
(x1571) and the rest spliced from x1542: unconnected 631 to 622 (two
layers 431 to 422: facade60 25 to 10, miniboard 5 to 3, MusicBrain 1 to
0, CompassTag 1 to 0; OpenDeck 10 to 12, Wonkle 364 to 370, snaphat 0
to 1, flexypin 0 to 1), vias 2128 to 2153; pours mixed (the largest
piece of the main pour: pdlc 65 to 82 %, dreamV1_0 85 to 92, mini48 F.Cu
41 to 53; MakerTiles B.Cu 72 to 37, HB-UNI-SEN-BATT B.Cu 1 to 9 pieces,
76 to 64 %, facade60 F.Cu 44 to 9); 4 boards changed pour mode (the
ranking). Fingerprints 1 x 4 (Castor, k30-SBC, jetson, Sisu, all of four
layers and more): identical to x1542. At 1, F1's golden fingerprints
moved for its connected rung (after clean-up, before resolve; the kept
board's unchanged); at the default 0 the golden is main's, unchanged.

## Committed on 2026-10-22 (silkscreen placement)

The fence's open item C and issue 8: every result had silkscreen
findings, and `repair-kicad-silkscreen` was a separate step. Branch
silkscreen-placement on 313b920; snapshots x1552 (the commit) against
main's x1592, and x1550 (the same code on af9d32a) against x1539.

**Measured first** (KiCad's findings by cause, `drc.json`): the fence's
v5 four-layer result of x1539 has 15 (8 `silk_over_copper`, 3
`silk_overlap`, 4 `silk_edge_clearance`): 4 fields over their own pads
(C26, C27, which the old polish left at their default offset, "no room"),
4 over a neighbour's pads, 2 over a neighbour's outline, 1 field on
another, and 4 connector outlines (J1 back, J4 front) over the edge. The
short tier (x1521, 12 boards) has 649 (`silk_*` and `text_*`): 8 from
fields (5 over a neighbour's outline, 1 over the edge, and R6's
`text_height` and `text_thickness`: the designer's 0.2 mm text), the rest the
footprints' graphics over each other and over pads (a logo footprint over
the LEDs of USB_Cable_Tester: 197), the board's own marks under parts
(Blue_Line's artwork: 180) and outlines over the edge; most of its boards
hide their references.

**The fields** (silk_fields.rs; docs/placer.md, "The fields on the
silkscreen"): the polish after routing places every visible Reference and
Value field (KiCad 8 properties, KiCad 6 `fp_text`) by a scorer that
computes KiCad's three rules from our geometry: the ink of KiCad's stroke
font (measured with pcbnew per character: exact to 0.001 mm), mask
openings (not copper; overlap only), every silkscreen item and the
outline at the project's `min_silk_clearance`. Candidates: where it is,
the four sides slid from corner to corner at six distances, a quarter
turn, inside a large part, shrunk to 0.8 last; fewest findings first,
then the shortest move on the side it was on. A field without a clean
spot stays at its best and is reported (`labels.silkscreen` in
board-layout.json, the verdict's `silkscreen` fact and a next action), or
is hidden with `"silkscreen": {"hide_when_blocked": true}` (default
false). Board texts and graphics never move (main's rule).

**repair-kicad-silkscreen** shares the scorer: its pcbnew worker clips
the front strokes and moves crosses (it no longer moves references), then
the fields are placed as in the layout (no shrinking, no hiding), also
where the worker fails (KiCad 6 boards without uuids). On the x1539 fence
result: 15 findings before, 2 after (J1's back outline).

**Tests**: kicad-cli on a board of fifteen fields in every predicament at
`min_silk_clearance` 0 and 0.2: the predicted findings per field equal
KiCad's, and after placing KiCad finds none; the ink against pcbnew's
numbers; a boxed-in field kept and reported, or hidden. The labels test's
reference now lies on the board text (the old box overlapped it, the true
ink does not).

**Measured.** The fence's v5 four-layer board (`layout.json`, single
layouts outside the tiers): x1539 15 findings, x1550 4; x1592 15, x1552
4 (all `silk_edge_clearance` of J1's and J4's outlines, which overhang
the edge); 43 fields, 35 moved, none blocked; the boards byte-identical
outside the fields on both bases (segments, vias, zones, parts), the
verdict complete as before. The short tier, 3 x 2 with the ledger
(`--min-free-gb 10`), x1592 against x1552: cosmetic findings 1951 ->
1950 (framework_mobo_lefthalf 65 -> 64: a reference over the edge; every
other board the same), KiCad silkscreen findings 566 -> 565, every
board's result byte-identical outside the fields, the objectives
identical (unconnected 21, new errors 3, vias 1183, 21211 mm, work
2469 s). The short tier's boards hide most references (0xCB, the CDFER
cards, urchin, Blue_Line show none; reCamera 87, all clean before and
after): their 576 remaining findings are the footprints' graphics over
each other and over pads (179 + 142 + 45), the board's marks under parts
(159) and outlines over the edge (36), and R6's designer text below the
minimum height (2). Router fingerprints are route mode, where no polish
runs: not rerun.

## Committed on 2026-10-22 (memory ledger)

Progress had slowed to the queue for build/quick-tier.lock, and without
the lock two rounds of runs were OOM-killed: each run (and each harness)
judged by the memory available when it started, runs admitted together
could not see each other's reservations, and an admission floor does not
protect against growth after admission. Now a machine-wide **memory
ledger** lets runs go concurrently without the lock. Snapshots x1590
(measured below) and x1591 (x1590 and a race fix: a process's own limit
never lowers when the watchdog raised it while the main thread waited).
Branch memory-ledger.

**The ledger** (crates/pcb-kicad/src/ledger.rs; the same file and rules
in benchmarks/harness_common.py): one JSON file
(`$XDG_RUNTIME_DIR/pcb-maker-ledger.json`, else `/tmp/...`;
`PCB_MAKER_LEDGER` names another, `off` none) of reservations: pid,
started, the process's start ticks (a reused pid is another process),
megabytes, label and, while a process waits, what it waits for. Every
update takes an exclusive flock on `<ledger>.lock` and writes the file
whole (renamed into place); reservations of dead pids are dropped on
every read, so a run that panics or is killed releases its memory.
`pcb-maker ledger` (`--json`) lists who holds what.

**The binary** (route, layout, place and drc-kicad-board arm it):
reserves before it allocates: at the lattice check (before any map) what
it holds and a router at the finest pitch it starts at (the maps' floor,
1.7 times plus 460 MB, the 1.25 margin); where the ladder or the layout's
placement race decides how many routers run side by side, as many as the
ledger has room for (at least one, waiting for its room), the maps
estimate of each and the fixed 460 MB once; 2.2 GB more while kicad-cli
runs (`run_bounded`). It waits while the ledger's total and its request
exceed MemTotal less a margin (`PCB_MAKER_LEDGER_MARGIN_MB`, default
6144; `PCB_MAKER_LEDGER_TOTAL_MB` replaces it, a test knob) and logs the
wait every 60 s. A request larger than the capacity runs when nothing
else holds memory; two runs that each hold memory and wait to grow would
wait for ever, so the oldest of them goes on (into the margin);
reservations of the process's ancestors (a harness, a free dimension's
parent) do not stop it. The reservation is also a limit of its own: a
watchdog thread reads the resident size every 200 ms, raises the
reservation by a fifth while the ledger has room and otherwise writes the
verdict (exit 4, internal_error, the error naming the reservation and the
ledger) and exits, instead of the OOM killer. Not as
`PCB_MAKER_MEMORY_MB`: that one is one of the ladder's own limits and
decides which finer lattices it attempts (`too_small_for`), so a
reservation fed into it would change results; the ledger changes only how
many routers run at once, which never decides which are judged. The
library's callers and the tests do nothing until the executable arms it;
the CLI tests use a ledger of their own.

**The harnesses**: run.py's MemoryGate admits against the ledger
(`harness_common.admit`: the live reservations and the board's expected
peak against the capacity, and `--min-free-gb` kept as a floor on
MemAvailable less what the reserving runs are still expected to grow
by), reserves the expected peak at once under the ledger's lock and hands
it to the board's binary when it starts (a binary since x1590 replaces it
with its own); the tier lock is opt-in (`--lock`; `--no-lock` accepted
and does nothing). triage.py's shared gate is the machine ledger
(`--memory-gate` names another; one entry per runner, its jobs times
`--job-memory-gib`), import_pending.py reserves each conversion through
it: the three memory gates the harness review found are one rule
(`harness_common.memory_room`). The harness tests use a ledger of their
own.

**Judged.** Unit tests: stale and reused pids dropped on read, 8 threads
x 25 updates under the flock losing none, totals, the wait and deadlock
rules; a CLI test (`routes_side_by_side_serialise_through_the_memory_ledger`):
two routes started together under `PCB_MAKER_LEDGER_TOTAL_MB=1000` both
wait for a third process's 900 MB, then go one after the other, never
both reserved at once, and both finish with their verdicts, the ledger
empty after; run.py tests (waiting while the reservations fill the
machine and going when one ends, a board larger than the machine alone,
the hand-over to the binary, the floor's growth, the format triage and
the binary share). Three short route-mode boards at once (ducktop2 bms,
brickpico, musicbrain-busboard, `--jobs 3 --expected-gb 1.2`, a ledger
of 3000 MB: two fit): the third waited 45 s in run.py's admission while
the two ran (their binaries reserved 1126 and 1826 MB, then fewer probes
side by side: "3 of 4 probes side by side ... 3 fit the memory ledger"),
none was killed, all three rows have results identical to x1542's
(routed, vias, length, open, work; peaks 811-1049 MB). The route-mode
expansion tier (8 x 1, no lock, 30 s, `--min-free-gb 6`, the machine's
ledger beside other agents' work) x1590 against
build/expansion-probe2-x1542.log: all 46 rows identical (routed, vias,
length, open, work seconds, pass and strict pass, KiCad's findings, exit,
verdict; 631 unconnected, 18 passes, strict 18) and all 46 result boards
byte-identical; no row killed or cut; wall 272 s (x1542: 616 s at
`--min-free-gb 14` on a busier machine; not a like-for-like timing). 11
admissions waited 15-30 s on the floor: the binaries' reservations are
upper bounds (2 to 7 times the measured peaks on small boards), so the
floor's "still to grow" is conservative. `bash scripts/check.sh --fast`
and the clippy gate.

## Committed on 2026-10-22 (probe overshoot)

A negotiation checked its work limit and the work deadline only after a
whole iteration, so a rung probe searched up to an iteration past its
share (probe overhead, below: MSign's four probes 5.06 to 5.26 s on
shares of 3.75; on the expansion tier 25.1 s of probe search past the
shares over 61 probes, at most 1.51 s one). Now a probe's limit binds
between the batches of its iterations from its second iteration on, the
rest of the cut iteration is kept, and `resume` routes it first and ends
that iteration as it would have ended, so the leading probe and its
continuation search what they did before, with more of the bound left.
Branch probe-overshoot; snapshots x1540 (every negotiation cut inside
an iteration), x1541 (the probes alone, the continuation starting from
the cut board), x1542 (the commit: the probes, the cut finished on
resume). Details: docs/router.md, "The probes' overshoot".

**What changed** (pcb-router, negotiate.rs): the batch loop checks
`negotiation_worked_out` (the old end-of-iteration test, now a method)
and `past_work_deadline` before each batch after the first while
`Router::in_probe` (set by `probe`) and the router has an iteration
behind it; a cut stores `CutIteration` (the batches left, the
iteration's index, growth, price of sharing) and sets `last_conflicted`
from the board as it is; `resume_polished` calls
`finish_cut_iteration` before it collects the pending nets (the batches
left, then `end_iteration`, the conflict, history and pour-refresh part
of the loop moved into a method, the replan on odd iterations, the price
grown when something is still in conflict); any other negotiation drops
a stale cut. `PCB_ROUTER_LIMITS_BETWEEN_ITERATIONS` restores the rule
before (config reference and env index updated). Tests: a probe whose
limit lies just past its first iteration stops after the first of its
second iteration's batches, reports the nets left in conflict, and,
resumed, gives the same expansions, fingerprint, iterations, conflicts
and statuses as a probe that finished that iteration
(`a_negotiation_limit_binds_between_the_batches_of_an_iteration`).
tests/fixtures.rs `f2s_free_dimension_stops_at_the_first_clean_size`
compares the retry's and confirmation's budgets to the microsecond: it
compared floats summed in another order than the product's
(25.050905193714215 against ...208 in one run of `check.sh --fast`; the
fixture's work follows the measured rate, so the values change run to
run).

**MSign and iorodeo** (30 s, in the expansion tier runs below, 8 x 1):

| board | x1539: probes' search (share 3.75) | continuation's deadline / negotiation | KiCad unconnected, router open, vias | x1542: probes' search | continuation | result |
|---|---|---|---|---|---|---|
| MSign | 5.06, 5.06, 5.15, 5.26 | 14.04 / 4.39 s | 69, 341, 192 | 3.75, 3.75, 3.97, 3.75 | 18.23 / 7.24 s | 21, 201, 251 |
| iorodeo | 3.84, 4.31, 4.99, 3.90 | 17.47 / 6.78 s | 133, 246, 265 | 3.77, 3.75, 4.79, 3.78 | 18.34 / 7.28 s | 93, 247, 283 |

The leaders are the same (tracks); iorodeo's plane-stubs probe still
goes 1.04 s past its share in its first iteration, which is whole
whatever the limit. Over the tier's 61 probes the search past the
shares is 3.1 s (25.1 before), 21.9 s less probe search in all.

**Expansion tier** (46 boards, 8 x 1, `--no-lock`, no row cut; x1539
run first as the reference, build/expansion-probe2-x1539, as no x1539
log existed): KiCad unconnected 761 to 631, passes 18 to 18 (strict 18
to 18), new errors 3 to 3, vias 4174 to 4204, length 109890 to 112361
mm, work 1222 to 1221 s. 12 rows changed, 34 the same (routed,
unconnected, vias, length):

| board | x1539 (routed, unconnected, vias) | x1542 | why |
|---|---|---|---|
| MAS500 dronecarrier | 103/129, 45, 203 | 125/129, 3, 165 | the leader changed: the plane-stubs probe led with 95 unfinished after 4.23 s of search; stopped at 3.80 s it has 102, and tracks (97 both) leads: 130 to 5 open |
| MSign | 166/188, 69, 192 | 178/188, 21, 251 | as above |
| iorodeo | 158/178, 133, 265 | 159/178, 93, 283 | as above |
| node_v6 | 53/77, 35, 95 | 55/77, 33, 99 | more work |
| tuxkeychain | 36/42, 6, 66 | 37/42, 5, 63 | more work |
| OpenRX-Mono | 34/43, 31, 26 | 31/43, 34, 28 | the same leader (tracks, 32 unfinished both), its negotiation limit 8.28 to 8.60 s ends elsewhere: 43 to 46 open |
| ICE40HX1K | pass, 210 vias | pass, 202 vias | more work for the via reduction |
| ducktop2 bms, ZSWatch, power-manifold, magnetorquer, anthracite | the same open, errors and passes | 0 to 2 vias fewer, the length within 0.7 % | more work for the finishing phases |

The variants (same reference): x1540 (every negotiation cut inside an
iteration: the continuations and the via reduction's rounds too) 666
unconnected, 17 passes, new errors 5 (ducktop2 lost its pass, ZSWatch 3
to 9 unconnected and an error after a continuation cut at seven nets in
conflict, anthracite an error); x1541 (only the probes cut, the
continuation from the cut board) 664, 16 passes (ducktop2 and
ICE40HX1K lost theirs: ducktop2's leader cut at 84 unfinished where it
had 80, its continuation 2 open). Neither reached 18 passes.

**Fingerprints** (Castor, k30, jetson, Sisu; route mode from v16, 1 x
4, budget scale 0.25, `PCB_ROUTER_FINGERPRINT=1
PCB_ROUTER_NO_WALL_GUARD=1`, outside the lock; build/fp-probe2-x1539 and
build/fp-probe2-x1542):
- Castor: no probe reached its limit (the connect probe finishes at
  15.99 s of 37.5): every fingerprint and the board identical.
- k30: the leader (tracks) finishes everything at 30.26 s, under its
  limit; the three other probes were cut (37.5 s of search where they
  had 37.8 to 43.4): the leader's every fingerprint and the board
  identical, the work 284.6 to 277.4 s.
- jetson: every probe cut, the leader (tracks) at 37.57 s where it had
  39.05; the continuation's negotiation limit is the same 60 s, its
  deadline 188.0 to 199.5 s: the fingerprint before resolve identical,
  after clean-up changed (the clean-up's budget follows the work left;
  jetson's clean-up also differs between runs of one binary, fp-fence
  above); 20 unconnected and 597 vias both, length 10784.0 to 10781.9.
- Sisu: every probe cut (37.5 to 38.1 s where they had 37.6 to 42.3),
  the leader's search 37.55 both; changed from the fingerprint before
  resolve, which Sisu does between runs of one binary too (finding 5,
  lowfix1 above); 25 unconnected and 375 vias both; its 0.1 mm attempt
  72 to 60 open.

**Not done.** The short tier (a layout tier: it waits for the lock, and
the layouts do not use route mode's shares; a layout's probes get the
same in-iteration limit, the layout ladder's `probe_seconds`). The
probes' unfinished count is taken where the cut leaves the board, so a
probe cut early in an iteration counts its rest as conflicted; on the
tier's 16 probed boards one leader changed (MAS500, for the better).
`bash scripts/check.sh --fast` and the clippy gate pass.
## Committed on 2026-10-22 (the snapshot release)

The `snapshot` release had been stale since 2026-10-21 (`21194a9`): the
macOS build failed (pcb-kicad's `libc` was a Linux-only dependency, the
memory limits' `getrlimit` needs it on every Unix) and the release job
needed every platform. `.github/workflows/snapshot.yml` became
`release.yml`: linux-x86_64 runs `scripts/check.sh --fast` and must
build, macos-aarch64 may fail without holding the release; the release
and its tag are deleted and made again with the archives (binary, README,
LICENSE, VERSION, docs/agents.md, skills/pcb-maker/SKILL.md), `VERSION`
and `SHA256SUMS`. `scripts/release.sh` does the same by hand.
`pcb-maker --version` prints the commit and the build date (build.rs,
"unknown" without git); the verdict's `environment` records them as
`pcb_maker_version`. Branch release-snapshot.

## Committed on 2026-10-21 (free dimension and layout budget fixes)

The review of 2026-10-21 (docs/reviews/2026-10-21-review-placer.md),
findings 3, 4, 5, 6, 8, 9, 10 and 11: the free dimension's geometry,
budget, children and reports, one notion of a held part, the layout's
budget caps and footprint ties. Branch free-dimension-fixes; snapshots
x1480 (the measurements), x1482 (the commit, on main ad6d9cd).

**Shrinking past the band (3).** A size used to move points within
`band_mm` of the edge past points that stay, and only fixed parts, rule
areas and constraint boxes were checked. Now (free_dimension.rs, `build`):
- a zone whose points in the band would pass points beyond it (a pour's
  notch just past the band) is cut at the new edge, its outline and its
  fills (Sutherland-Hodgman, `clip_polygon`; KiCad clips a pour to the
  board anyway); a drawn shape that would fold (a silkscreen frame) makes
  the size infeasible;
- board texts, circles, images, dimensions and vias move whole by the
  side of their box nearest the edge (a text's box from its font,
  length, justification and angle), and any drawn item, text or inner
  cutout that lay on the board and would end beyond the outline makes the
  size infeasible (copper zones are cut instead, tracks go with the
  layout);
- arcs move by their ends: whole with both in the band, not at all with
  neither, and with one the arc keeps its shape between its new ends (the
  similarity that maps the old ends to the new takes the middle), so
  what is joined to either end stays joined;
- `region` and `keepout` boxes move side by side: a box reaching from the
  edge across the board stretches (it used to move whole, infeasible at
  every shrink and short of its far end on growth); one whose near side
  would pass its far side is infeasible;
- a held part whose body lies at the edge (within 0.5 mm, or beyond it)
  moves with the edge though its origin lies outside the band (a
  centre-origin connector, a module over the edge); a tall one that only
  reaches into the band stays (the anchor by the body's side in the band
  carried the fixture F2's header J1, 20 mm tall, off the top at 20 mm);
  other footprints still go by their origin.
Unit tests: a pour's notch cut, a frame folding, a cutout and a label
past the band, a label by its box, an arc keeping its joint, a box
stretching and vanishing, a held part by its body at the edge and a tall
one staying. The fixture test F2 (main's, through the binary) now
retries 20 mm (one unconnected item, close) with what is left before
confirming 22 mm.

**The budget (4).** The full runs get everything left of
`total_seconds` (they got `confirm_seconds` at most: a search with a clean
first probe spent under half its budget). After a clean probe the
largest smaller size that came close (every part placed, KiCad's
unconnected items and errors at most 4 together, `CLOSE_FINDINGS`) is
laid out again with all that is left (`retry`; results are not
monotone), else the smallest clean size (`confirm`); without a clean
probe the closest (`closest`). The smallest clean run is the result. The
docs' example (6000 s, neither key set) affords six probes, not nine.

**Children (5).** Each probe gets threads / `parallel` threads and
`PCB_MAKER_MEMORY_MB` = (an explicit `PCB_MAKER_MEMORY_MB`, else the
memory available when the search starts) / `parallel`, the last batch's
too (it used to get all the threads, and every child planned its routers
side by side for the whole machine's free memory). A size's probe can
differ at another `parallel` through the wall guard, and on a board whose
finer lattice's single router does not fit a probe's share (the ladder
skips it, as under any explicit limit).

**One notion of a held part (6).** `held_parts` (placement_constraints.rs)
lowers the board with the placer config and reads each footprint's
`fixed`: the placer's and constraints.json's `fixed`, parts placed with
`x` and `y`, by default locked and netless parts and parts at the edge,
less what `free`, a constraint or `move_all` frees. The keepout
constraints (footprints allowed or not, the input-time warning) and the
free dimension take it from there; `HeldParts::of` (the placer's lists,
locked, netless, less `free`) and the free dimension's own variant are
gone. A keepout over a part constraints.json fixes now allows footprints
(issue 15 for that input). Under `move_all` the placer's own `fixed`
goes too, as the lowering has always had it. Unit test
`one_notion_of_a_held_part_covers_every_case`.

**Layout budget caps (8).** `direct_narrow_budget`: the clean-up at most
120 s or a twentieth of the work left, the negotiation at most 900 s or
half of it (unchanged up to 1800 s left); the attempt in the other pour
mode gets the work left as its ladder budget (it was at most 1200 s). The
neck repair runs within the layout's work left: as many KiCad checks as
it pays for (at least a fill, a trial and the board as chosen, else it
does not run and its note says so; `neck_check_bound`, `NECK_MIN_CHECKS`).

**Paths (9), the reference (10), ties (11).** The chosen run's paths are
rewritten by whole components (`from` itself, `from/...`, inside
sentences; not `from.json`, `from.log`) in every JSON report it holds.
An explicit `--reference` is moved with each size (its own rectangle's
edge, the same band and distance; `free-dimension/<size>mm-<kind>/reference/`),
so a finding shared with it stays at the same place. Footprints with one
reference and the same or no uuid keep the file's order in the canonical
order (their text, which holds the pose, used to decide, so a placed
board read back could swap two of them for the layout's index-based
poses).

**The fence boards** (benchmarks/fence, frozen; the free dimension
`{"edge": "bottom", "min": 34, "max": 60, "step": 2, "prefer_below":
[100], "band_mm": 10, "parallel": 2}` added to the board's own layout
config, router `auto`, `PCB_WORK_RATE=3.15e6,150e6`, 16 threads, through
the tier lock; build/dim2). x1404 (main's free-dimension code, the
"before") against x1480 (this branch before its rebase on main, so the
difference is this work alone).

Two layers (v5-2layer, JLCPCB's 0.127 mm rules, layout-long.json:
`total_seconds` 3000, defaults: probes 333 s, 1000 s kept):

| mm | run | budget s x1404 | budget s x1480 | KiCad unconnected | errors | work s | vias | length mm | chosen |
| --- | --- | --- | --- | --- | --- | --- | --- | --- | --- |
| 34 | probe | 333 | 333 | 18 | starved_thermal 1 | 225 | 402 | 3844 | |
| 36 | probe | 333 | 333 | 10 | starved_thermal 1 | 194 | 458 | 4342 | |
| 38 | probe | 333 | 333 | 5 | starved_thermal 1 | 106 | 497 | 4547 | |
| 40 | probe | 333 | 333 | 0 | 0 | 210 | 320 | 4244 | |
| 40 | confirm | 1000 | 2266 | 0 | 0 | 244 | 320 | 4244 | yes |

40 mm chosen both, 977 s of work both (424 and 352 s wall); every probe
and the result board byte for byte the same. The confirmation now has
the 2266 s left instead of 1000 s, and ends at 244 s all the same: the
layout completes long before either budget, so on this board the budget
the search leaves unused is the layouts' own early end, not the search's
split. No smaller size came close (38 mm: 5 unconnected and a starved
thermal, over `CLOSE_FINDINGS`), so nothing was retried. (The fence
report's run at 6000 s with probes of 500 s chose 44 mm; at 333 s and
x1404 40 mm is clean.)

Four layers (v5-4layer, its layout.json with three keepouts,
`total_seconds` 1500: probes 167 s, 500 s kept):

| mm | run | budget s x1404 | budget s x1480 | KiCad unconnected | errors | work s | vias | length mm | chosen |
| --- | --- | --- | --- | --- | --- | --- | --- | --- | --- |
| 34 | probe | 167 | 167 | 0 | 0 | 113 | 197 | 3752 | |
| 36 | probe | 167 | 167 | 0 | 0 | 115 | 181 | 3785 | |
| 34 | confirm | 500 | 1271 | 0 | 0 | 222 | 181 | 3698 | yes |

34 mm both, 451 s of work both, every board byte for byte the same; the
confirmation's budget 500 to 1271 s, its work 222 s either way. On both
boards the boards at each size are byte-identical inputs too (the held
set from the lowering and the new move rules move the same parts and
items there; the keepouts are written footprints not allowed both ways).

**Probes the same at parallel 1 and 2** (the two-layer board, sizes 34
and 36 mm, probes of 200 s, `total_seconds` 600, x1480, build/dim2/par1-
and par2-x1480): parallel 1 gave each probe 16 threads and
`PCB_MAKER_MEMORY_MB` 30529, parallel 2 8 threads and 15287 each. Both
probes (34 mm 20 unconnected, 36 mm 9 and two errors), their work
(148.9 and 161.5 s) and their boards are byte for byte the same, and so
is the closest run (36 mm with the 290 s left: 7 unconnected) and the
result; wall 285 s against 213 s.

**Short tier** (3 x 2, x1480 against build/short-fence-x1403.log, in
build/short-dim2-x1480; outside the lock under the coordinator's relaxed
rule, 18 GB available): 11 of 12
rows and their result boards byte for byte identical. Every short-tier
board's budget is 500 s (a third of 1500), under the 1800 s where the
narrow-signal caps start to grow and the 1200 s the continuation was
capped at, so those changes cannot show there. USB_Keypad changed by the
neck repair's work bound: its layout had 28 s of work left before the
repair (a check counts 14.9 s), the repair needs three checks, so it did
not run (x1403 ran it, 4 connection_width necks to 1, and ended at 524.0 s
of work on a 500 s budget); now 479.3 s, the same routes (37/39, 95 vias,
2 KiCad unconnected, no error that counts either way).

`bash scripts/check.sh` passes (after the rebase on ad6d9cd, x1482) and the clippy gate. Docs: constraints.md
("A free dimension": what moves, sizes that cannot work, the search,
threads and memory, the budget, a reference, what comes back; the
keepout row), placer.md (one notion of a held part, the order's ties),
router.md (the narrow-signal caps, the neck repair's bound), agents.md,
config reference.

## Committed on 2026-10-21 (fence benchmark pass)

Every breadboard-fence benchmark run with x1404, the issues of the fence
report that are not the placer's, and one measurement of each. Branch
fence-benchmark; snapshots x1460 (the first build), x1461 (slivers out of
the piece count; the runs on x1461), x1462 (rebased on 2094b6b; the
keep_whole runs and the Sisu and DSP layouts), x1463 (the commit: the bounded round removed). Results:
benchmarks/fence/README.md ("benchmark pass" rows, issue log), docs
benchmarks.md "Fence (2026-10-21)". Machine rule: one run at a time,
layouts outside the lock at 16 GB available, the free-dimension search
under it.

**The pours' verdict** (issue 17; docs/router.md "Copper pours",
docs/agents.md). After the final route (route mode's ladder, not a
layout's own ladders; a layout's board kept) the written board is
lowered with its tracks and vias as copper and a router built on it
without routing (`measure_pours`, `Router::pour_report`): per pour and
layer, the pieces holding copper of the net (KiCad removes the others),
their areas (free nodes times the pitch squared plus half the brush
along the rim; slivers under 0.25 mm² left out), the pour's area inside
the outline, and other nets' track length on the layer. The verdict's
`pours`, board-router.json's `pour_quality` and score-kicad-board's
`pours` carry it; more than 5 pieces, or a largest piece under half the
copper, adds a next action naming `keep_whole`. It counts as no work
and decides nothing. Against pour-pieces.py (KiCad's fill), mm² of
copper and pieces: fence 2 layers x1403 403.5/411, 16/16; the
free-dimension probes of x1420 at 40/44/46/48 mm 1108/1099, 48/45;
1163/1160, 37/41; 1080/1071, 31/29; 1305/1291, 41/42; this pass at 34 mm
195/230, 23/19 (the worst); 46 mm 440.5/441, 9/10; four layers In1.Cu
1 piece 2048 against 3 pieces (2019 and two of 2 mm²).

**A rule area that lets pours in no longer hides the fill.** Writing
the verdict showed the four-layer fence's In1.Cu GND with no fill at all
in the model: its `GND plane: no tracks` rule area (tracks out,
copperpour allowed) was stamped into the trace map the fill reads, as
was the `plane <net> <layer>` area of an `add_pours` plane (ded766d),
which hid its own plane. Such areas (`Board::fill_passes`, in the
statics key) now come after the rest like mask openings, so the fill
trace keeps what lies under them, and the fill probe skips them. Unit
test: the plane request's In1.Cu report equals the board's without the
area. The fence four layers x1404 to x1461: complete and clean both,
181 to 179 vias, 3698 to 3930 mm, work 233 to 312 s. No fingerprint
board and no short-tier board but USB_Cable_Tester (3 such areas, its
row unchanged) has one.

**keep_whole** (issue 17). `add_pours[].keep_whole`: parts (globs, each
courtyard, else its pads, plus `margin_mm` 0.5) or boxes (from the
outline's corner, `origin`, `margin_mm` 0). Each is written as a rule
area `keep whole <net> <layer>` with everything allowed (KiCad has no
rule by net), lowered by its name as a keepout whose net is the pour's:
other nets' tracks off the layer, their vias out (`via_owner` names the
pour's net), the pour's net passes and fills. An earlier run's regions
give way; an unknown part is an input error. Unit test (maps inside and
outside a region, the areas written twice, the error). The fence at 46
mm (router-cut10, layout-long), before x1404: 134/134, one starved
thermal, 231 vias, pour 10 pieces, largest 293, 441 mm² (13 %). With
x1462: under U1, U4, U5 6 unconnected, pour 18 pieces, largest 408, 994
mm²; those and a strip x 2-74.2 y 27-28.5 9 unconnected, largest 719,
1163 mm²; the strip alone **complete and clean**, 287 vias, pour 15
pieces, largest 373, 657 mm² (19 %). Under the chips the escapes lose
their vias; across the middle band the vertical rows go over on the top
for 1.5 mm and keep their order. The free dimension's
size (44 mm: the search itself waited behind 47 runs on the tier lock
and was not rerun; its 44 mm input laid out singly): before x1404
complete and clean, pour 41 pieces, largest 389, 1425 mm² (42 %); the
strip at the same place in the band (y 28.25-29.75) complete and clean,
495 vias against 395, 39 pieces, largest 630, 1284 mm² (38 %).

**Fixed parts on top of each other** (issue 5).
`held_parts_overlapping` (placement_constraints.rs): pairs of held
parts with a net (`fixed`, `fixed_patterns`, locked) whose courtyards
on one side share half the smaller one, or whose pads meet on a copper
layer; `placement_input_warnings` (keepouts, then overlaps) is what
layout-kicad-board and place-kicad-board check before the run, printed
and first in next_actions. Unit test: C1 and C2 at one spot warned, C3
touching and a movable R1 and a back-side U1 not. The placer's own
"fixed parts X and Y overlap" (its constraint warnings, board-placer.json)
stays.

**swap_candidates by electrical type** (issue 3, open item F).
describe-kicad-board read only the pads' `pinfunction`; the fence's
board has none (its CH32X035's names, PA15, PA0/A0, ..., are in the
schematic). A part whose pads name no function is now listed by its
pins' types (`by: electrical type`, its symbol `unit`): bidirectional or
passive pins of one unit (the pad's `pintype`, else the schematic's
symbol, sheets followed), on signal nets reaching another part, at
least three on three nets, not diodes, transistors, switches, crystals
and the like by passive pins; pad numbers are the globs. The fence: U1
(44 pins), U4 and U5 (24 X and Y ports each), RN1-RN8. Parts with pad
functions keep the name rule: the short tier's lists (0xCB, USB_Keypad,
WLED_Matrix, urchin, MIDAS, OpenFC: GPIO*, PA*, IO*, P0.*) unchanged.
Unit test.

**The 5 % rule** (open item B; the placer review's finding 7). The
rule's reason is time (Sisu: moves took the final ladder's time, 108
open in the layout against 20 in route mode), and since 2026-10-07
(5f6dc2f) it holds only on four layers with pours: the two-layer fence
gets its moves (34 mm: 6 trials, none kept; README row B was out of
date). A bounded round there (at most 4 parts in a tenth of the
budget) was built and measured, then removed: the short tier never
reaches it (12 rows and result boards identical), Sisu at a third of
its budget is the narrow step's (37 open, more than 2 % but not 5 %:
unchanged), and DSP-ADAU1452 at a third, 100 open after the first
route, tried R237 three ways: 224 to 285 open each (32 nets rerouted
in that congestion do not settle), none kept, the same board with 89 s
more work (368 to 457 s). The comment at the rule says so.

**Measurements.** Fingerprints (Castor, k30, jetson, Sisu; route mode
from v16, 1 x 4, budget scale 0.25, `PCB_ROUTER_FINGERPRINT=1
PCB_ROUTER_NO_WALL_GUARD=1`) x1461 against x1404 (build/fp-fence3-*):
every fingerprint and every routed board identical (against
build/fp-fence-x1402, jetson after clean-up and Sisu differ as they do
for x1404 itself). Short tier (3 x 2) x1461 against
build/short-fence-x1403.log: all 12 rows identical and the 12 result
boards byte for byte. `bash scripts/check.sh` and the clippy gate pass.

**Not done.** Issue 14 (silkscreen text, the placer's). Fewer pour
pieces at 46 mm (keep_whole grows the main piece, not the count); a
keep_whole region over another net's pad on the layer leaves that pad
only the other side. The model's copper on the 34 mm board is 15 %
under KiCad's.

## Committed on 2026-10-21 (lowering review fixes)

The findings of docs/reviews/2026-10-21-review-lowering.md in its order.
Branch lowering-review-fixes; snapshots x1510 (finding 1 alone), x1511
(1, 5, 2, 7, 3, 8: the measurement), x1512 (1 and 5 alone, for
attribution), x1513 (the commit, rebased on the fab-release package, the
placer's nets as before). Measured against x1404.

**1. The exposed pad and its copy** (docs/router.md, "Vias in pads").
`exposed_pad` took the largest SMD pad and asked it to be twice any
other's area; an exposed pad drawn on both sides (an F.Cu and a B.Cu copy
at one place, thermal vias between) is its own copy's equal, so neither
copy was exempt from `no_via_pads`. SMD pads of one number within 0.5 mm
of each other are now one candidate of their boxes' box (also a pad split
into pieces), its members out of the "others"; score-kicad-board's count
shares the function (`PadBox`). Footprints whose verdict changes (pcbnew
emulation of both rules): Sisu U2.11, MSign U1.21, U5.32, U7.29, U9.81,
LC_PAMV4b U3.25, node_v6 U1.57; none on ZSWatch, Castor, k30 or jetson.
The review expected this to recover Sisu (6 to 23 unconnected in ded766d)
and ZSWatch (0 to 4): it does neither. Sisu's U2 (WSON-10, a 1.2 x 2 mm
pad with six 0.3 mm thermal vias in it) has no room for a router via
between its own holes: x1510 routes Sisu (v18 placement, quarter budget,
1 x 4) to the byte the board of x1404 (172/192, 357 vias, 37 KiCad
unconnected). ZSWatch's loss is its module's LGA pads (22 vias in them
before ded766d), which no exposed-pad rule exempts; its row is identical.
MSign gains: 71 to 42 unconnected at 30 s (x1510 and x1511 alike).
Unit tests: a_two_sided_or_split_exposed_pad_is_one_exposed_pad (Sisu's
U2 pattern, a four-piece pad with paste-only apertures, a connector's
two shield tabs that stay listed), the score's vias-in-pads test with a
B.Cu copy of its QFN's pad.

**5. The via-owner map and pad order** (grid.rs). The `own_vias` arm (a
whole via inside single-layer copper of its net that may hold vias: the
exposed pad, a net's copper graphic) now also takes a node an earlier
obstacle left to the net (`via_owner == owner16(net)`, the `beside` arm
of a listed pin). With the pins first, KiCad's order, such nodes went to
no net. Unit test an_exposed_pads_own_vias_do_not_depend_on_the_order_of_the_pads
(fails without the guard). This is what changes Sisu: x1512 (1 and 5)
gives x1511's board, 175/192 routed, 373 vias, 37 unconnected (x1404
357 vias, 37).

**2. Padstack phantom copper** (docs/router.md, "Dimensions, text boxes,
cached copper texts and padstacks"). A padstack pad's own (front) shape
was copper on every layer of the pad, the stack's shapes added beside
it: under a smaller back shape (dennisss's 4 mm over 3 mm) the model had
copper KiCad has not. The own shape is now on F.Cu and the layers the
stack does not name; each named layer's shape is merged into the pad's
terminal (`MergedPad::padstack`, after the terminal merge, by the
terminal that holds the pad): `copper_components` joins it to the
terminal, `prepare_net` takes the terminal's nodes on that layer inside
that shape, a kept or adopted track ending on it ends on the terminal,
its zone connection (none, solid) and thermal relief are the pad's, its
spokes' frame is its own size, and the terminal's own spoke record keeps
to the layers its shape has. (kept.rs: a padstack shape whose pad another
terminal takes is another pad of it.) No board of the tiers has a
padstack on a layer its pad has copper on (OpenESC's and MokyaLora's
stacks name layers their SMD pads lack), so nothing measured changes.
Unit tests: a_padstacks_front_shape_is_copper_only_where_the_stack_names_no_other
(the obstacle per layer, the merged shape, a B.Cu track 1.8 mm from the
centre not connected, on F.Cu and at 1.4 mm connected),
a_padstack_terminal_is_reached_inside_each_layers_own_shape.

**7. Knockouts of unconnected pads** (docs/router.md, "KiCad's
connectivity"). The review's fix (the larger of the zone's and the pad's
clearance) holds for hatched zones only. KiCad 10.0.7's
knockoutThermalReliefs knocks a same-net `zone_connect 0` pad out of a
solid zone at the zone's local clearance or a `physical_clearance` rule,
not the pad's; a hatched zone sends it to buildCopperItemClearances,
which takes the clearance constraint between zone and pad (the pad's
local override included). `isolated_knockouts` now does that. Thermal
reliefs are the thermal gap alone in both (THERMAL_RELIEF_GAP_CONSTRAINT):
unchanged. Unit test a_hatched_zone_keeps_an_unconnected_pads_own_larger_clearance.
No tier board has a hatched zone with such a pad.

**3. Net names without thread state.** The two thread-locals (bare
names, jumper nets) are one `NetNames` value (`NetNames::of(pcb)`,
`normalize`, `routable`, `written`). score-kicad-board, the statistics,
describe, the pour plan, the pours, the neck repair, the pin swap, the
net classes and the placer's decoupling read it from the board they
have; `lower_board`, the placer's lowering and the project rules use
theirs (`note_net_names` returns it and notes it on the thread). Only the
copper writer (`canonicalize_copper_net_names`, `written_net_name`)
still reads the noted value, which its entry `unambiguous_pad_net_names`
notes; the free functions assert (debug builds) that one is noted. The
placer counts no `unconnected-(...)` net as joining parts (`placer_net`):
it read the router's jumper nets from whatever board its thread had
lowered (none in a layout's placement), and noting them moved MIDAS's,
USB_Cable_Tester's and eternal-keypad's placements in the short tier
(x1511: MIDAS 7 to 11 unconnected); a jumper net's pads are one pin's on
one part. Unit tests: the_score_reads_net_names_by_the_board_it_scores
(pads on GND and /GND, a /GND track cuts the GND pour after an empty
board was noted), a_hierarchical_and_a_bare_net_of_one_name_stay_two_nets
(describe-kicad-board: /A15 and A15).

**4. Rules as floors** (docs/router.md, "Project rules": the reason).
Documented, not changed: every lowered value is a floor, a custom rule
never lowers a class clearance, two rules on a pair give the larger.
Giving copper graphics and the fill the pair clearance of their net is
not contained (a fourth item kind in the 3 x 3 kind table, the dru
lowering marking rules without a type condition): documented as a limit.

**8. Parser consistency.** A blind, buried or micro via on the board is
copper on the layers from its first to its last (`via_layers`), a
through via on all (it was all for every via). A hidden text box draws
nothing, and a footprint text box with no shape is skipped (its mask
opening indexed an empty list). A text box of `start` and `end` with an
`angle` was right already: KiCad's parser turns the text, not the box (a
box off the axes is written as `pts`), documented. Unit test
a_blind_via_is_copper_on_its_own_layers_and_a_hidden_text_box_is_none.

**Route mode on the four boards** (vias in SMD pads by pcbnew's
`HitTest` of the via's centre, exposed pads apart; the router's own rule
keeps the same):

| board | x1404 | x1511 |
|---|---|---|
| Sisu (v18, quarter budget, 1 x 4) | 172/192, 37 unconnected, 357 vias, 0 in pads (2 in exposed pads) | 175/192, 37, 373 vias, 0 (2) |
| ZSWatch (expansion, 30 s) | 3 unconnected, 140 vias, 0 in pads | identical board |
| MSign (expansion, 30 s) | 71 unconnected, 183 vias, 0 (3 in exposed pads) | 42, 254 vias, 0 (1) |
| LC_PAMV4b (expansion, 30 s) | 1 unconnected, starved_thermal 1, 28 vias, 0 | identical board |

**Fingerprints** (Castor, k30, jetson, Sisu; route mode from v16, 1 x
4, budget scale 0.25, `PCB_ROUTER_FINGERPRINT=1
PCB_ROUTER_NO_WALL_GUARD=1`, outside the lock by the relaxed rule;
build/lowfix1-fp-x1511) against build/fp-fence-x1402: Castor and k30
every fingerprint and the board identical; jetson identical to x1404's
(build/fp-robust-base-x1404: every fingerprint and the board; x1402
predates the probe-overhead package); Sisu changes from the first
negotiation (finding 5: x1512 reproduces it): 23 to 25 unconnected, 387
to 375 vias (x1404's 24 and 386).

**Expansion tier** (8 x 1, `--no-lock`, `--min-free-gb 14`) x1511
against build/expansion-fence-x1404.log: 46 rows valid (no OOM kill), 39
result boards identical; passes 18 to 18 (strict 18 to 18), unconnected
750 to 737, new errors 2 to 2, vias 4161 to 4231, work 1222 to 1223 s.
Changed rows, all reproduced by x1512 (findings 1 and 5 alone): MSign 71
to 42 (finding 1, its four exposed pads); node_v6 28 to 34 (finding 1,
U1.57's two copies; the vias the pad now takes changed the negotiation,
102 to 90 vias); OpenRX-Mono 21 to 31 (finding 5: vias along its QFN's
pad, 1 to 5 in the exposed pad, 19 to 25 vias); pdlc, nrf24_remote,
backplane (51 to 55 vias), CompassTag (33 to 34) other boards with the
same counts. ZSWatch stays at 3 (the review's "0 to 4 should come back":
it had come back to 3 with x1404 already, and its pads are an LGA
module's).
## Committed on 2026-10-21 (fixture tests)

The fixture tests the review of the tests proposed
(docs/reviews/2026-10-21-review-tests.md, "Proposed fixture tests"), in
its order, on two committed fixtures; check.sh's fast gate and full run;
CI's nightly job with KiCad 10. Branch fixture-tests; no binary change
that needs a snapshot (two clippy rewrites in src/, no behaviour).

**F1** (`tests/fixtures/f1`): 30 x 25 mm, two layers, 13 parts, KiCad
10's format written by hand: a QFN-16 with an exposed pad and four
thermal vias, a SOT-223 (tab and pin 2 one number), a WS2812B-2020 with
0.7 mm pads, a 1.2 mm GND test pad alone on F.Cu (TP2), a net tie
(`net_tie_pad_groups`, a copper bridge), a stacked F.Cu and B.Cu pad of
one number without a hole, a solder jumper (`jumper_pad_groups`), a pad
with `zone_connect 0`, an arc track of +3V3 from C1.1 to U2.2 that route
mode keeps (2094b6b), a Margin rectangle near the right edge, an
unplated slot, a plated hole listing only `*.Mask`, a text box and a
dimension on F.Cu, a KiCad 9 padstack (1.2 mm front, 1.8 mm back), a net
named "14", GND and +5V zones of equal priority on B.Cu, a hatched GND
zone on F.Cu, a .kicad_pro with a Power class by a wildcard (`+5V`) and
a regular expression (`\+3V[0-9]`), and a JLCPCB-style .kicad_dru ("Pad
to Track clearance" 0.2 mm, then "Min. Spacing" 0.127 mm, and a 0.4 mm
pair rule between PA and PB). kicad-cli 10.0.7 loads it and its DRC
runs. **F2** (`tests/fixtures/f2/layout.json`): F1's layout with its
bottom edge free (18 to 24 mm, band 7 mm, two probes side by side, 60
s).

**The tests** (tests/fixtures.rs through the binary, every route at
`PCB_WORK_RATE=3.15e6,150e6`, `PCB_MAKER_MEMORY_MB=4096`, no wall guard;
crates/pcb-kicad/src/tests/fixtures.rs for what needs the crate's
insides; golden files in `tests/fixtures/golden`, rewritten under
`PCB_UPDATE_GOLDEN=1`, a difference failing with its lines and the
command that accepts it):

| # | Test | On main (c127abb) |
| --- | --- | --- |
| 1 | no via in an SMD pad (the exposed pad excepted) and no two vias of one net closer than their drills plus hole_to_hole, on F1 routed in route mode; with `via_in_pad: true` TP2's via inside its pad | passes; fails with the lowering's no-via list emptied (TP2's GND via 0.62 mm into the pad) |
| 2 | the static maps' key (`static_maps_key`) of F1 differs for F1 with via_in_pad, one no-via pad fewer, without the .kicad_dru's net rules, with a keepout, another pitch, a class or hole_to_hole, and is the same for the same inputs (it compared build work until main's charging change made shared and unshared builds charge alike) | passes: the key is complete for these; fails with `no_via_pads` out of `statics_key` (ded766d's rebase) |
| 3 | board-router.json's rules list against the golden (the pair rule applied, "Pad to Track" no_effect as a later rule takes its pairs, "Min. Spacing" no_effect), PA's routed copper 0.4 mm from PB's, and the pair rule at 3 mm leaving PA or PB open (F1's pair keeps 0.4 mm without the rule too) | passes; fails with `core_config` dropping `net_rules` |
| 3b | the kept arc track: written back as it was, `kept_copper` one arc on +3V3 making one connection | passes |
| 4 | F1's fingerprints at 1 and 4 threads equal, and equal to the golden ones | passes |
| 5 | with kicad-cli: the router's open connections and KiCad's unconnected items agree on the routed F1 (none and none, or some and some), every dead pad among KiCad's unconnected items, their counts by net against the golden | passes: the router 1 open, KiCad 1 unconnected (+3V3; before the net-tie fix 2 and 2, the net tie's pads); which two items KiCad names for an unconnected pair changes from run to run where several are as near (J2's stacked pads; a via or a track beside NT1.1), so the golden counts them by net |
| 6 | with kicad-cli: the unrouted and the routed F1's DRC findings against the golden, the verdict's starved thermals KiCad's | passes: the unrouted F1 has 23 unconnected items, as the lowering's terminals have them (the stacked pair two items, the jumper group one, U2's pin 2 and tab two), no error (an isolated_copper warning for the hatched zone, library warnings); the routed F1 one +3V3 item, no error, no starved thermal |
| 7 | the lowering of F1 against a golden table: obstacles by kind, every obstacle that is not a pad, the terminals of every net, the merged pads, the pads closed to vias, the planes | caught the net tie (below; fixed on main since) |
| 8 | F2 through the binary with a fake kicad-cli clean from 22 mm: 18 mm infeasible (J1 would leave the outline), probes at 20 and 22, the confirmation at 22 mm with a third of the budget, 24 untried; the outline 22 mm, the band's footprints and the hatched zone moved by 3 mm, J1 not; the children at the parent's measured rates, pinned | passes |
| 9 | F1 and F1 reversed placed one after the other, their boards compared as `parse_board` reads them back, with the wirelengths (ignored until fence-report-placer's fix of `SOURCE_ORDERS` lands) | passes when run; the placer's own order test alone fails 3 times in 3 (the review's prediction) |
| 10 | a keepout over the fixed J1 written with footprints allowed and warned of first in next_actions; `add_pours` GND plane on B.Cu writes one rule area `plane GND B.Cu`, no track on B.Cu | passes with the placer config's `fixed`; with constraints.json's `fixed` not (below; an ignored test) |

**Caught on main.**

- **Net ties were not lowered as KiCad has them** (at 20956b3): the net
  tie's copper bridge was copper of no net, so both NT1 pads were dead
  and F1 routed with 2 connections open whatever the budget (KiCad
  10.0.7: the same two items unconnected, no finding about the bridge).
  Fixed on main by the net-ties package; after the rebase on c127abb the
  goldens changed as expected: the lowering's bridge is NT1's own (box
  15.9..16.1 instead of 15.5..16.5), the dead pads are gone, the routed
  F1 at 2 s leaves one +3V3 connection open (router 1, KiCad 1), and the
  fingerprints changed (GND before stitching 5 pieces where it had 4
  and 11).
- **A keepout over a part fixed by constraints.json.** The keepout check
  (`HeldParts`) counts the placer config's `fixed` and `fixed_patterns`,
  locked and netless parts, not the constraints' own `fixed`: with
  `{"constraints": {"fixed": ["*"], "keepout": [...]}}` J1 stays where it
  is but the rule area is written with footprints not allowed (KiCad
  then reports J1) and nothing is warned of.
  `a_keepout_over_a_part_fixed_by_the_constraints_file_is_warned_of`
  (ignored) holds the expected behaviour.
- **Workspace clippy.** `cargo clippy --workspace --all-targets` failed on
  two collapsible ifs (src/agent_cli.rs, src/main.rs), a complex type and
  a `Some(..).filter` in pcb-kicad's ladder_tests.rs: rewritten, and the
  gate is now the whole workspace with every target.

An unplated slot listing `*.Cu` is lowered as a hole and as a copper pad
of no net of the same size (harmless: the hole's 0.3 mm edge clearance
is the larger); the golden table shows it.

**check.sh.** `bash scripts/check.sh` (and `--full`) runs what it ran
before plus the 23 tests that never ran (pcb-congestion 5, pcb-topo 8,
tools/config-reference 4, the root binary's unit tests 6), the fixture
tests and workspace clippy (`--all-targets`, warnings as errors): what
runs by default grows by these. `--fast` is the gate: the same without
the agent walkthrough and the placer's placement.rs, 28 s of tests and 5
s of pytest once built (cargo's link steps on top). `--no-walkthrough`
stays. The script says first whether kicad-cli is there and which tests
skip without it; empty arrays expand under `set -u` on bash before 4.4.
`bash scripts/check.sh` passes with kicad-cli and without it (a PATH
without it). Warm, `--fast` took 32 s and the default 41 s without
kicad-cli, 97 s with it (the walkthrough's layouts). The placer's
`a_placement_does_not_depend_on_the_order_of_the_file` failed one
default run of four (alone it fails every time: its race).

**CI.** Push and pull request: `check.sh --fast` (clippy now in it) and
rustfmt. Nightly at 02:17 UTC and on demand: `check.sh --full` on
ubuntu-24.04 with KiCad 10 from the kicad-10.0-releases PPA (not yet run
on GitHub: the PPA's name is the KiCad project's pattern), which runs
the walkthrough, the KiCad fixture tests and placement.rs. The
walkthrough job of its own, which always skipped, is gone.

**Also.** The free-dimension CLI test
(`a_free_dimension_searches_the_sizes_and_keeps_one_layout`) pins the
rates and turns the wall guard off (two 2 s probes side by side were cut
by the wall clock under load). The ignored `census_of_custom_rules`
(an environment-gated test that asserted nothing) is a tool now:
`cargo run --release --manifest-path
experiments/project-rules/rules-census/Cargo.toml --
benchmarks/expansion/data/manifest.json`.

**Left out.** `verify_with_rules` on the output (test 3 measures the
pair's copper itself), the pour model's predicted pieces against KiCad's
fill (the fingerprint lines carry the router's pieces; KiCad's count
needs the refilled board), and order independence across two
place-kicad-board processes.

## Committed on 2026-10-21 (fence report: placer side)

The breadboard fence report (`~/tmp/fence-report.md`, finding 2): the
board's own silkscreen labels (header pin names, the test pads' names)
were no placement obstacle, the placer put SW1 under `3V3  VDD  5V`, and
the polish after routing moved seven labels off the parts' silkscreen,
one 7.5 mm, the test pads' names shuffled. Branch fence-report-placer;
snapshots x1410 (first build), x1411 (graphics clustered by their
strokes), x1412 (a level of their own where the marks give way, last),
x1413 (that level also past the budget), x1414 (halo-free marks; its
tier stopped unrun), x1415 (parts seated on marks per level), x1416 (the
seating retries with those parts first), x1417 (the commit: x1414's
halo rule had reached linked pairs against hollow parts in the anneal;
now only marks).

**What changed** (docs/placer.md, "The board's own texts and silkscreen
graphics"; docs/constraints.md):

- The board's visible texts and text boxes on F.SilkS, B.SilkS, F.Cu
  and B.Cu (by the lowering's stroke-font box) and its silkscreen
  graphics in clusters of strokes (not a cluster spanning 0.8 of the
  board: frames, Blue_Line's background image) are placement obstacles
  on their side with a keepout's semantics (`board_placer::board_marks`,
  `mark_obstacle`); parts meet silkscreen marks without their routing
  halos. A fixed part on a mark gets a warning in the placement's
  `constraint_warnings`.
- A part that finds no room off the marks at a level lies on them
  (`seat_on_marks`: the parts the level left without a place are
  legalized with the marks out of their way, the others kept, else all
  again with those first), before the next level takes halos or spacing
  from every part; also after the levels when the budget ended them.
  `parts_on_marks` in `board-placer.json`; a placement with fewer ranks
  first among the seeds and in the race.
- The polish (`labels`) moves only the parts' reference and value
  labels; board texts, silkscreen graphics and copper texts stay where
  the input put them, and the layout routes around copper texts (it took
  them off the board while routing and moved them afterwards). Movable
  parts whose silkscreen, pads or stuck label lie on a mark are reported
  in `board-layout.json` (`labels.overlaps`) and the verdict's
  next_actions; fixed parts are not (the placement warned).
- A mark on a movable part at its input position belongs to that part
  (`attach_marks`): no obstacle, left in place, listed as
  `attached_marks` in `board-placer.json` and `labels.attached` in
  `board-layout.json`. Parts stacked at one origin claim nothing. (The
  coordinator's refinement after the first measurement; x1419.)
- `keep_board_marks` (placer config, on) switches it all; off restores
  the behaviour before.
- Fence issue 12 (benchmarks/fence/README.md, D): at the placer's
  `edge_copper` levels a part held at an edge was judged by its pads,
  which kept the project's copper-to-edge clearance on the held side but
  only min(copper clearance, edge margin) on the others, the side an
  overhang reaches past among them: the fence ESP32's top pads 0.2 mm
  from the edge against the project's 0.3 mm (15 copper_edge_clearance).
  Every side now keeps at least the copper-to-edge clearance
  (`legal::on_board`; unit test in tests/placement.rs);
  docs/constraints.md said the router enforced it, which it cannot.
- The writer's footprint graphic order comes from the content again:
  each courtyard's graphics in KiCad's save order (`FOOTPRINT::
  cmp_drawings`: segments before rectangles, arcs, circles, polygons, then
  by start and end point; `canonical_order::source_order`), the rest in
  the canonical order. Since 3d28004 the writer restored each
  footprint's graphics in the order of the file read, from a process-wide
  map keyed by reference and graphics, so the result was not
  byte-identical from any input order (placer.md promises it):
  `a_placement_does_not_depend_on_the_order_of_the_file` failed every
  time alone (five of five on main ded766d, also on 8608a51 and bbdb205)
  and passed or failed in the full suite by when the lowering test beside
  it read the same footprints in the other order; the last board read
  won, and a turned footprint's `fp_text` changed the key. KiCad-saved
  files are in KiCad's order, so the malformed courtyards the map was
  built for stay the designer's: re-written with kicad-cli 10.0.7's DRC,
  AccordeurDeGuitare 0 (designer 0), dacha legacy_pi_mocap 6 (6), pi_rack
  18 (18), compute_r1 14 (14); with the canonical order alone 14, 22 and
  16 (and MK1 malformed). The order test passes five of five, as written
  (byte comparison), and the full suite three of three.

**Designers' texts lie on their parts.** On the 107 boards of tasks v7
with an outline, 3144 of 6088 board texts on the silkscreen (by a rough
box from size and character count) overlap a
designer part's courtyard-and-pads box on their side (header pin names
inside a connector's courtyard, a chip's function on its DIP outline,
KiCad 6 boards' reference designators drawn as board texts). In a task
whose parts are unplaced those texts mark where the designer's parts
were; kept as obstacles they keep our parts from those spots, which is
where the tier's changes come from.

**The fence** (`layout-kicad-board` on copies of
`breadboard-enhancer/hw/build/v5`, the reference's rates pinned, two
threads; "without": layout.json's three workaround keepouts removed).
Board texts at the input's position against the result, and what lies on
the input's spot (pcbnew's boxes):

| text | x1385 without | x1417 without |
| --- | --- | --- |
| `GND` (81.50, 58.90) | moved 0.50 mm | stays |
| `VDD` (84.04, 58.90) | moved 2.00 mm (to 84.43, 60.86) | stays |
| `DIO` (86.58, 58.90) | moved 2.00 mm (to 84.62, 59.29, under the VDD pad's name) | stays |
| `CLK` (89.12, 58.90) | moved 3.00 mm | stays |
| `3V3  VDD  5V` (103.37, 55.34) | moved 7.50 mm (to 110.87); SW1's courtyard, silkscreen and pads on its spot | stays, nothing on it |
| `P2 P1` (111.02, 55.34) | moved 1.50 mm | stays |
| `GND 3V3 5V` (118.67, 55.34) | moved 1.50 mm | stays |
| `B7  ANALOG BUS  B0`, `STACK`, `breadboard fence v5` | stay | stay |

The test pads' names lie on the fixed TP1-TP4's and C1/C2's courtyards
(1.3 mm around a 1.5 mm pad) as the generator placed them: eleven
placement warnings, nothing moved, nothing reported (fixed parts).
pcbnew's generous text box also touches R23's reference label at
`P2 P1`; KiCad's DRC finds no silkscreen finding on any board text.
Verdicts, without / with the keepouts: x1385 complete, 186 / 201 vias,
3757 / 3887 mm, 17 / 15 cosmetic findings, HPWL 2838 / 2914 mm; x1417
complete, 187 / 202 vias, 3771 / 3837 mm, 15 / 17 cosmetic findings (two
of R22's label over J3's pins without the keepouts before, two with them
after), HPWL 2837 / 2913 mm, no next action. With the keepouts x1385
still moved the four test pad names; x1417 moves none. The keepouts are
no longer needed.

**The short tier** (`--tier short`, 3 x 2, `--no-lock`; x1381's
`build/short-charge-x1381.log`, which main's x1385, x1396 and x1422
repeat, against x1417 in `build/short-fence-x1417.log`). Every one of
the 12 boards has marks (3 to 104 obstacles: `components` in
`board-placer.json` before and after), so every placement changes; the
polish moved 2 to 148 of the designer's board items per board before
and moves none now:

| board | marks | HPWL mm | front, back utilization | parts seated on marks; marks reported under parts | routed, KiCad unconnected, pass |
| --- | ---: | --- | --- | ---: | --- |
| 1337-v4.0 | 5 | 984.6 -> 997.1 | 0.14, 0.32 -> 0.15, 0.32 | 0; 0 | 47/47 pass -> 47/47 pass |
| USB_Cable_Tester | 3 | 1274.4 -> 1269.7 | 0.62, 0.20 -> 0.62, 0.21 | 0; 0 | 54/54 pass -> 54/54 pass (95 -> 138 vias) |
| USB_Keypad | 43 (artwork) | 804.5 -> 1460.4 | 0.57, 0 -> 0.79, 0.33 | 6 (J1, J2, SW2, SW3, SW8, SW9); 13 | 37/39, 2 -> 37/39, 2 |
| WLED_Matrix | 40 (artwork) | 1057.0 -> 1078.1 | 0.50, 0 -> 0.59, 0.27 | 0; 0 | 100/104, 5 -> 101/104, 4 |
| framework_mobo_lefthalf | 9 | 946.0 -> 915.9 | 0.94, 0.08 -> 1.03, 0.24 | 1 (R29); 1 | 71/71 pass -> 70/71, 1 and a new error |
| MIDAS-MK1.1 | 102 | 2543.5 -> 2971.4 | 0.45, 0.42 -> 0.57, 0.51 | 6 (J102, J104, J107, TP112-TP114); 23 | 151/156, 7 -> 151/156, 6 |
| OpenFC | 104 | 842.8 -> 828.6 | 0.36, 0.43 -> 1.11, 0.98 | 17 (CN1, P1, 15 headers); 21 | 82/82 pass -> 80/82, 2 |
| reCamera_S101 | 6 | 188.6 -> 212.2 | 0.37, 0.27 -> 0.39, 0.33 | 0; 3 (silkscreen or labels) | 55/56, 4 -> 54/56, 5 |
| spell_tome_bottom | 95 (artwork) | 146.6 -> 165.9 | 0.07, 0.07 -> 0.10, 0.27 | 0; 0 | 18/18 pass -> 18/18 pass |
| eternal-keypad | 8 | 531.8 -> 522.2 | 0.35, 0 -> 0.37, 0.02 | 0; 0 | 65/65 pass -> 65/65, a new shorting_items (a VCC track over ProMicro1's netless pad: the router's) |
| urchin | 24 | 1678.7 -> 1698.6 | 0.08, 0.41 -> 0.09, 0.42 | 0; 1 | 68/68 pass -> 68/68 pass |
| Blue_Line | 4 (its 8793-polygon background left out) | 104.3 -> 101.9 | 0.005, 0.002 -> 0.005, 0.015 | 0; 0 | 26/26 pass -> 26/26 pass |

Objectives: passes 8 -> 5 (strict 7 -> 5: OpenFC, framework_mobo and
eternal-keypad lose theirs), KiCad unconnected 18 -> 20, new errors 2 ->
3, vias 1068 -> 1254, length 20596 -> 21804 mm, work 2749 -> 2829 s,
first-route open terminals 28 -> 44, cut 0; against the designer
(medians) vias x0.84 -> x0.96, length x0.98 -> x1.02, open 0.000 ->
0.007 of the connections. HPWL rises on 6 boards (USB_Keypad +82 %,
MIDAS +17 %, spell_tome +13 %, reCamera +13 %) and falls on 6 (at most
-3 %). The cost is the benchmark's: its texts mark where the designer's
parts were (above), and a task re-places those parts. With
`--placer '{"keep_board_marks": false}'` the tier repeats x1381
(`build/short-fence-x1416-off.log`: 10 rows identical, 1337-v4.0 and
OpenFC changed by an anneal slip in x1416 that x1417 fixes; x1417 off on
those two, `build/short-fence-x1417-off.log`, gives x1381's rows).

**After the refinement** (x14190, rebased on main 20956b3, which routes
with no vias in SMD pads; marks on movable parts attached, the
courtyard write order, the edge-pad rule). The fence: no text moves,
nothing movable on a label, no mark attached (four of its five movable
parts are stacked under the back title), HPWL 2837 / 2913 mm as x1417,
complete, 182 / 194 vias, 3659 / 3812 mm (main's router). The short
tier against x1381 (`build/short-fence-x14190.log`): passes 8 -> 6
(strict 7 -> 6), KiCad unconnected 18 -> 28, new errors 2 -> 3, vias
1068 -> 1190, length 20596 -> 21494 mm; designer-relative medians vias
x0.84 -> x0.87, length x0.98 -> x0.99, open 0.000 -> 0.000 (mean 0.018
-> 0.027). Only MIDAS-MK1.1 attaches marks (14 `+` and `-` texts to its
two 36 mm headers J101 and J102, which its task does not stack): HPWL 2543.5 -> 2317.4 mm (x1417 2971.4), 149/156, 9
unconnected (x1417 151/156, 6). Every other task stacks its movable
parts at one point before the binary sees the board, so the input says
nothing about which part a text labels and the rule cannot fire there;
the HPWL of the other 11 boards is x1417's. The residue: OpenFC 82/82
pass -> 78/82, 9 unconnected (its 17 connectors and headers seated on
their stale pin names, 104 marks, the front 111 % full) and
eternal-keypad's one shorting_items; framework_mobo passes again (71/71,
the same placement as x1417, main's router). Against the same binary with
`--placer '{"keep_board_marks": false}'` (`build/short-fence-x14190-off.log`:
its placements, HPWL to the decimal, are x1381's; its routes main's),
which separates the marks from main's router: passes 6 -> 6 (strict 5 ->
6), KiCad unconnected 20 -> 28 (OpenFC 2 -> 9, MIDAS-MK1.1 7 -> 9,
WLED_Matrix 5 -> 4), new errors 2 -> 3, vias 1088 -> 1190, length 20673
-> 21494 mm, medians vias x0.85 -> x0.87, length x0.93 -> x0.99. So of
x1381's 8 passes main's router (no vias in SMD pads) takes two
(OpenFC 80/82, eternal-keypad's shorting_items, both also with the marks
off), and the marks take none; framework_mobo's strict pass comes back.
The marks' cost is OpenFC's 7 more unconnected items, MIDAS's 2 and the
designer-relative length (+6 %).

Steps on the way (all on this tier): x1411, the marks hard at every
level, left MIDAS-MK1.1, USB_Keypad and OpenFC with parts without a
place (connectors at the edge under their pin names, beside artwork);
x1412's level of their own after the courtyard level placed MIDAS and
USB_Keypad only after every part gave up its halo and spacing (USB_Keypad
35/39 routed) and OpenFC's levels spent the budget before it; x1414 let
parts meet silkscreen marks without their halos (WLED_Matrix HPWL 1229
-> 1078 mm, 97/104 -> 101/104); x1415 seated the parts without room on
the marks at their level (OpenFC placed); x1416 retried the seating with
those parts first (MIDAS-MK1.1 placed: its two 36 mm headers at the
bottom edge).

**The schematic tier** has no mark at all: its 12 imported boards (the
x1250 run's projects, the latest schematic log) carry no board text,
text box or graphic on a silkscreen or outer copper layer, so the
placement problem, the routed board and the polish are the old ones.
Checked by placing all 12 with x1385 and x1417 (`place-kicad-board`, the
tier's placer config): byte-identical placed boards, the same
components and wirelength. The tier itself ran one board under the
machine's memory rule (MokyaLora 261/262, KiCad 0 unconnected, pass, as
in x1250) and was stopped there.

## Measured on 2026-10-21 (Freerouting refresh)

The README's comparison with Freerouting rested on x738 (2026-10-15, the
designer's placement with all copper removed). Refreshed on the current
binary in route mode: the v17 layout placements, pcb-maker-x1367 at the task
budgets (`run.py --route-from build/github-layout-v17`), Freerouting 2.2.4
on the same placed boards with its own job timeout at the wall our row took,
both judged by run.py's pass and strict rules on KiCad's DRC. Partial: 40 of
109 boards measured (the longest walls first), the rest and the ten-board
speed study (1 and 16 threads on both sides) still running, a batch of three
boards per hold of the tier lock. Details, the per-board table and the
commands to continue: [benchmarks.md](benchmarks.md#freerouting-against-x1367-2026-10-21).

On the 40: ours 38 with a result, 2 pass, 1 strict pass (MokyaLora);
Freerouting 28 with a result, none passes; ours better on 35, Freerouting on
4 (Business-Cards Batch_1, katia where ours times out, jiran-ble-lite, which
main at x1434 now routes to the designer's 1 open, SNSP-CPU-01), one tie (the
0xCB pcb-panel, whose KiCad check times out on both sides). On the 27 both
finish: KiCad unconnected 2 379 against 6 161, counting errors 128 against
1 371, at a median 1.29 times our CPU per board for Freerouting.

Tooling: `benchmarks/github/freerouting.py`'s `freeroute` takes the thread
count (`-mt`), Freerouting's job timeout (`--router.job_timeout`: it stops
and writes the session of what it has) and a refill for boards that keep
their pours, and records the passes and whether the timeout stopped it;
`freerouting_route_mode.py` runs it on a route-mode run's placements and
judges the results with run.py's functions (`--lock` takes the tier lock per
batch of boards); `freerouting_route_compare.py` prints the table and the
summary. Measurement notes: two of our rows were killed by the OOM killer
(exit -9 with a result board from the ladder's last best attempt: not the
binary's answer) and rerun alone; the rows cut by the wall clock or the
harness timeout were rerun on x1371 (which stretches the wall guard under
load and differs from x1367 only in work accounting).

## Measured on 2026-10-21 (KiCadRoutingTools head-to-head)

What was measured: KiCadRoutingTools, at commit cddf4cd (MIT; Python
around a Rust A* core), routed the 109 v7 placements of the Freerouting
refresh. Our side is x1367's route-mode rows of the same placements
(`build/fr2-route-final.log`). Both sides are judged by KiCad's DRC with
the pass rule and the strict rule. A measurement and tooling change on
branch kicadroutingtools-benchmark; no Rust.

New tooling:
- `benchmarks/github/kicadroutingtools.py` runs its CLI router headless.
  It passes `--no-fix-drc-settings` and `--escalation board`, so the board's
  own rules stay the judge. Each run has a cap in multiples of our row's
  wall; a cut run that already wrote its board is still judged.
  `--lock` holds the tier lock around KiCad's checks only, and
  `--krt-jobs` limits how many routers run at once.
- `benchmarks/github/kicadroutingtools_compare.py` writes the per-board
  table and the summary: within our wall, boards it wins, speed ratios.
  For a cut run it reads how far the log got.
- The setup (its own clone and venv in build/krt, the Rust module built
  from source) is in benchmarks/github/README.md.

Results ([docs/benchmarks.md](benchmarks.md#kicadroutingtools-against-x1367-2026-10-21)):
- KiCadRoutingTools passes no board.
- On the 59 boards judged so far, ours passes 19 (17 strict).
  KiCadRoutingTools finished 6, was cut after writing its board on 10, and
  was cut before writing one on 43. It is better than ours on 3 boards
  (Neotron-Pico 0 unconnected items against our 24, but with 5 hole and
  annular errors; glasgow; the 0xCB panel) and worse on 53.
- Where both have a result: KiCad unconnected items 899 ours against 1505;
  copper errors 25 against 890; vias 4005 against 8435.
- On the ten timing boards it takes a median 3.5 times our CPU seconds;
  it has no thread setting, its search is one thread.
- Not done: 50 boards it routed are still waiting for the tier lock to be
  judged, and our 1- and 16-thread timing runs did not get the lock
  overnight. Both stay queued.

Its features we lack are summarised there for the roadmap, with how each
works and how much code it is: diff pairs with polarity, length and time
matching, impedance widths, BGA and QFN fanout, placement optimisation,
its own checkers with KiCad as the oracle, and the AI tab and skills.

## Committed on 2026-10-21 (robustness against hostile boards)

The findings of the review of the readers against hostile or odd input
(docs/reviews/2026-10-21-review-robustness.md), in its order. Branch
robustness-fixes; snapshots x1490 (a first version that refused
non-finite numbers in the parser) x1491 (the commit before the rebase on 2094b6b, measured) and x1492 (rebased). Before, two
inputs ended the process without a verdict (a stack overflow, an OOM
kill), a downloaded board could make the import write outside its
output directory, and seven panics were reachable from a file.

**The lattice bounded before it is allocated** (3.2). The outline's
extent is at most 2000 mm (`outline::MAX_BOARD_EXTENT_MM`; KiCad holds
32-bit nanometres, about 2.1 m either way): beyond, `board_loops`
returns the input's error with the extent and the corners (a stray
Edge.Cuts line to 10 m made a lattice of 10^10 nodes a map). Then the
lowering, before any map, sizes the lattice at the router's coarsest
pitch from the outline alone (`pcb_router::grid::LatticeSize`: nodes,
and a floor on the maps of 32 + 6 bytes a node, layer and rule class,
under every one of 1 644 lattices in the logs of 2026-10 by 1.25 or
more) and refuses it (exit 2) when a router of 1.7 times those maps and
460 MB (the ladder's estimate without its margin) exceeds
`PCB_MAKER_MEMORY_MB` or `memory_mb`, the cgroup, the address space
limit or MemAvailable (read once a run, `check_lattice_fits`); the
verdict's first next action names the extent, the pitch and the nodes,
and the retry takes the same output directory (an input error that
notes an action writes its verdict, `verdict::note_input_action`). In
the router, `Grid::choose` saturates instead of overflowing on an
infinite outline and `Grid::check_size` (at most 2^32 nodes a layer)
runs before `build_statics`; `Router::try_new` returns it.

**No recursion to overflow** (3.1). The S-expression parser keeps its
open lists on a stack and refuses nesting past 1000 levels
(`kicad_sexpr::MAX_NESTING`; KiCad's files nest a dozen) as the input's
error with its line; the writer (`encode`), `collect_named_nets`,
`expression_count` and `renew_uuids` walk with stacks; the recursive
walks left (the flip's mirror, the zone point maps) and the drop of a
tree stay within a 2 MiB thread at the limit (unit test, debug build).
A custom rule condition nested past 200 levels does not parse (reported
unmodelled, as a condition that does not parse always was), chains of
`||` and `&&` are built balanced (2 000 alternatives nest a dozen levels; the
truth is the same, both are associative), and a rule file nested past
1000 levels is the rules file's error. Parse errors of every KiCad file
are the input's (exit 2, not 4), as are the lowering's.

**Paths** (1.1 to 1.4). A footprint taken from a `--footprints-from`
board whose nickname or name is no file name (`../../x:../../y`) is the
input's error before anything is written (`fs_util::file_name_error`:
empty, `.`, `..`, a separator or NUL, a leading `~` or `$`); a
netlist's footprint name that climbs out of its library is not found;
every command checks its board id the same way (`check_board_id`). The
import's library copy (`copy_library_tree`, for the schematic flow)
copies files only: links are left out, folders at most 8 deep, a library
outside the project or holding the output directory (`${KIPRJMOD}/.`)
is not copied, with a note.

**Panics** (2.1 to 2.7). A footprint form without its name is refused
where it is read (`footprint_form_error`: the --footprints-from board,
the routed board; a library file of one is reported missing). A text
box of fewer than three corners on a mask layer is left out (it panicked
every route and layout of the board). Every coordinate, size and angle
read from a form goes through `finite_number` (`form_f64`, `form_at`,
`expression_coordinate`, a rule area's points, a drill, an arc's points):
NaN, inf and 1e999 are the input's error naming the item (`a gr_line's
start coordinate 1 is inf`). Refusing them in the parser (x1490) was
wrong: 56 of the intake's boards carry `(tstamp inf)`, and a KiCad 5
board writes a net named inf unquoted. A `(property "Footprint")`
without value is passed over, and footprint-link normalisation writes
the sheets only once all were read. `(net 1 "GND" x)` names GND (the
third field, as the net's name is read), not x. 32 copper layers are a
mask of 32 bits, not 0; the custom rules' layer shifts are checked.

**Misreads** (4.1 to 4.3). A net name meets the project's JSON
(`netclass_assignments`, `netclass_patterns`, KiCad 6's class lists) and
the custom rules unescaped (`kicad_sexpr::unescape`: `A\"B` in the board
is `A"B` there); a net-class pattern that is no expression by itself
(`x)|(?:.*`, which escaped the anchoring and matched every net) is no
expression anchored either. Copper (segments, arcs, vias, zones) on a
copper layer the stack lacks and text boxes without a box are listed in
board-router.json's `warnings` (empty, and not written, on an ordinary
board) and lead the route's next actions.

**Left out.** The review's 3.3 (quadratic outline chaining) and 3.4
(custom rules polynomial in the groups); they cost time, not the
process. A depth limit of 1000 rather than the review's 10 000: the
recursive walks of the readers stay within a test thread's 2 MiB at
1000 in a debug build, not at 10 000.

**Unchanged on ordinary boards.** Short tier (3 x 2) x1491 against
build/short-fence-x1403: all 12 rows identical and the 12 result boards
byte for byte (6 passes, strict 5, unconnected 20, work 2961 s both).
Fingerprints (Castor, k30, jetson, Sisu; route mode from v16, 1 x 4,
budget scale 0.25, `PCB_ROUTER_FINGERPRINT=1 PCB_ROUTER_NO_WALL_GUARD=1`,
build/fp-robust-x1491): Castor and k30 identical to build/fp-fence-x1402,
fingerprints and boards; jetson and Sisu differ from x1402, which predates
bbdb205 (probe overhead), and are identical, fingerprints and boards, to
x1404 (main's commit of the fence package, build/fp-robust-base-x1404);
the routed boards of all four are byte for byte those of build/fp-base-x1404
and build/fp-fab-x1450. No parsing change changed routing.

**Timing.** The largest intake board (HDMI2RGB raw, 39 MB): the parser
takes 6 % fewer instructions (callgrind of inspect-kicad-board: parse
5.30e9 to 4.97e9, the whole command 7.99e9 to 7.40e9; collect_named_nets
1.86e9 to 0.14e9); the lowering (`PCB_ROUTER_BUILD_TRACE`'s seconds,
three runs each interleaved on a loaded machine) 0.078-0.105 s to
0.070-0.092 s; the writer 4 to 15 % faster (indentation without an
allocation a line). An A/B in one binary of the parser with the
non-finite check in it (x1490's) was 1 to 4 % slower.
## Committed on 2026-10-21 (harness review fixes)

The findings of the independent harness review
(docs/reviews/2026-10-21-review-harness.md: H1-H3, M2-M5, L1-L3, L5 and
its list of duplicated code), on branch harness-fixes; snapshot x1500
(main 20956b3 with the route-mode publication, the one-item strict rule
in verdict.rs and the layout's final-ladder budget).

**A killed run is no result (H1).** Route mode moved each new best
attempt into its output directory while it ran, so a route the OOM
killer ended left `board-router.json` and `drc.json` of its best attempt
so far, and run.py judged them: build/fr2-route-x1367.log
(bumwings_v001_core, ghoul) and build/fr2-route-x1371-r2.log (katia, 93
open) have rows with `exit: -9`, `routed`, `native` and no `error`.
Now, in the binary, `route-kicad-board` routes into `<output>.partial`
(the ladder publishes there) and renames it to the output when it is
done, `board-router.json` written last; the verdict and the report's
rewrite with it are written whole (`write_atomically`, a temporary file
renamed into place). Checked on WLED_Matrix: killed with -9 after its
first attempt, it leaves `out.partial` and no `out`; run to the end, `out`
holds the report and the verdict and no `out.partial` is left. In
run.py, any exit outside the verdict's 0 to 4 (a signal, a shell's
128 + n, a panic past the verdict) gives a row with `error` ("killed by
signal 9 (SIGKILL)"), `killed` and `cut: ["killed"]`; nothing it left is
judged, and route-mode tasks run no drc-kicad-board on it (outcome
`process_signal`, triage's class). compare.py's `killed()` also flags
older rows by their exit code: the three rows above are KILLED (a
failure, out of better / same / worse, a KILLED line naming them).

**Process groups (M4).** run_measured starts each binary in a process
group of its own; a timeout kills the group (kicad-cli with it), and when
the binary ends whatever it left running is killed and waited for until
the group is gone, before the memory gate releases the board. The
harness's short calls (strip-kicad-tracks, the designer's and the
baseline's DRC, `--help`, score-kicad-board, the schematic tier's steps)
go through `harness_common.run_group`, which does the same on its
timeout. The orphans' CPU is still not counted.

**One-item findings (H2, strict rule 3).** `same_place` had nothing to
compare for a finding of one item, so the same uuid matched: a
`connection_width` of ours on a pour matched any of the designer's on
that pour, all reported at (0, 0). Now a finding of one item is the
designer's where its item lies within 0.05 mm of theirs, never where
KiCad gives the item no place (a zone at (0, 0); the finding's own
position stands in where a report has one, KiCad 10's has none), or
where it is the same item, not a zone, with the same description word for
word, KiCad's measured values included. That last clause keeps a moved
part's own findings with the part: with position alone, v18's strict
passes fell from 42 to 38 on the designer's own footprints moved by the
layout (mackerel-10-v1 and -08-v1, annular_width 5 each; Quanta75
RP2040Stamp, annular_width 6; CyberKeeb2040, malformed_courtyard 1).
verdict.rs keeps rule 1 with the same one-item matching (DrcItem `zone`,
`unlocated`, `located()`; DrcFinding keeps KiCad's description); tests in
both for the USB_Keypad case (our one connection_width on zone 8951654c,
the stripped designer board 45 there). Re-judged without rerouting
(`rejudge.py`, the stored drc.json and baseline-drc.json, the designer's
bridges from the DRC cache; rule 2 against rule 3, both with the
openings):

| run | pass_strict rule 2 -> 3 | strict new errors | boards changed |
| --- | --- | --- | --- |
| designer tier, build/designer-x1315b.log | 50 -> 49 | 1239 -> 1240 | USB_Keypad: connection_width 1 (it passed) |
| layout sweep, build/github-layout-v18c.log | 42 -> 42 | 1536 -> 1543 | USB_Keypad +6 connection_width, WLED_Matrix +1 (both failed already) |

So the designer tier is 49 of 109 under the strict rule, and the
"placement" group of the v18 comparison (the designer's placement passes,
ours does not) is 12 boards, not 13: USB_Keypad fails on both
placements.

**Provenance (M2, M3).** Every row records `binary`, `binary_sha256` and
`strict_rule` (also rows without a result); a spliced row records
`spliced_from` (the original log where the row was spliced there already
and says so). A spliced run's summary.json lists its `parts` (per log:
rows, jobs, threads, wall), takes the measured rates over every row
(`this_run_rates` beside), and says `strict_rules` when the rows mix
rules; its pass line then says A MIX OF RULES. compare.py names each
run's rules ("strict: before rule 2; after rules 1 (105 rows), 3 (4
rows)") and marks a strict count over several rules MIXED RULES.
`rejudge.py <log>[:<dir>] ...` judges a stored run again under the
current rules from its stored files (spliced rows from their source log,
the chain given or `spliced_from`), so a rule's re-judging is a logged
run rather than a scratch script.

**Smaller ones.** M5: the timeout's stretch allowance is
`PCB_WALL_STRETCH_MAX` as the binary reads it (a finite number of at
least 1, else 4). L1: rows without a result fail the strict count
(compare.py: v17 against v18 now says "strict - -> 42", 42 of 109 rows,
not "42 of 103 rows"). L2: rows of `compare.py --splice` are SPL and
counted. L3: the pass rules include the schematic tier's missing
footprints (compare.passes, passes_strict), and the schematic tier uses
them instead of deriving the condition again. L5: the README's x738 row
says why its 62 is not comparable. `compare.py --gap --targets` leaves
out the nine boards that are not targets (the v18 split's 97 boards) and
the gap prints both sides' routing work and warns when the binaries
differ.

**Dedupe, behaviour identical.** benchmarks/harness_common.py holds the
DRC counting filter, file_sha256, atomic_json, read_json, safe_name, the
machine's MemAvailable and the process-group runner; run.py, compare.py,
quality.py, schematic.py and benchmarks/expansion (harvest, triage,
import_pending) use it. run.py has one copytree ignore list,
strip-kicad-tracks through strip_command, board-router.json read once per
route row, intake_outcome calling triage.classify, and sets `pass` and
`pass_strict` with compare.passes and passes_strict. Proof: rejudge.py
over designer-x1315b and github-layout-v18c and compare.py over v17/v18,
--gap v18/designer-x1315b and short-fence x1403/x1416 gave byte-identical
output with the harness of 20956b3 and the deduplicated one (rows,
summary and text). The memory gates share the reading of MemAvailable;
their policies stay two (run.py's reservation per board in one process,
triage's file shared between processes). test_pipeline.py loads its
run.py by path: a pytest run over benchmarks/agent-tasks and
benchmarks/expansion together imported the other run.py (2 failures,
also on 20956b3).

**Matched budgets (H3).** Layout rows carry `ladder_budget_seconds` and
`ladder_budget_share` (the final ladder's work, or the narrow-signal
attempt's that stands in for it, and its share of `total_seconds`; null
where none ran), from board-layout.json's new
`final_ladder_budget_seconds` (x1500) or derived from layout.log for
older binaries (the two agree within 0.3 s on the short tier's rows).
Over v18's 109 rows the final ladder ran on 62, with a median share of
0.695 (0.684 over the 46 of them left open): the plain designer tier
gave the router 1500 s, the layout's last router about 1040. The tasks
README's designer tier says how to route the designer's placement at that
share (`--budget-scale 0.695`) and that the split stays an accounting of
open connections, not a measured cause. Matched designer tier (x1404, budget scale 0.695, 8 x 1, strict rule 3; build/designer-x1404-matched.log): 59 of 109 pass, 51 strict. Against the v18 layout (x1315) over the 99 target boards with both results: layout open 1973, designer-route open 1016; placement gap 957 (worse on 37 boards, same 54, better 8), routing gap 1016 on 39 boards; share of the layout's open connections placement 49 %, routing 51 % (the unmatched run at the full 1500 s gave 71/29). Binaries differ (x1315 layout, x1404 designer route), so the matched split is an upper bound on the placement share.

**Judged.** `python3 -m pytest benchmarks/agent-tasks benchmarks/expansion`
(119 tests, together and apart), `bash scripts/check.sh`, rustfmt and the
clippy gate. The short tier (3 x 2) with x1500 against
build/short-fence-x1403.log: all 12 result boards byte-identical, every
row's routed, vias, length, KiCad findings, work, first-route open
terminals and pass the same; 6/12 pass, strict 5/5; strict new errors
208 -> 210 (rule 3: USB_Keypad's and WLED_Matrix's connection_width on a
pour).

Left out: M1 (rule 2's unbounded subtraction and the designer-routed
openings), M6 (check.sh and CI, another package's), L4 (the greedy
position fallback), L6 (kicad-cli timeouts inside the binary not marked
cut), counting the CPU of a killed board's orphans, and one memory gate
policy for run.py and triage.
## Committed on 2026-10-21 (global route, phase 1)

A coarse global router as a routability measure (the v18 section's
recommendation and the 2026-10-21 review's direction call), validated
against final results; [global-route.md](global-route.md) has the model
and the tables. Snapshots x1440-x1442 (x1440 and x1441 had the race
score unreachable: the layout rejected it, then skipped the pre-ranking
for it).

**Built.** `crates/pcb-global` (new crate, in check.sh and the clippy
gate): tiles of 1.5 mm over the lowered `pcb_router::Board`; per layer
and tile boundary the tracks of the reference class that fit between the
pads, keepouts, holes and fixed copper (sampled every 0.05 mm on the
boundary, a pad the boundary cuts passing its own net), via sites per
tile, pour ownership per layer and tile; nets as trees along their
spanning tree (pour nets: each pad to its pour), A* in a window,
PathFinder negotiation (present price from 0.5 growing 1.8 a round,
history 1 on overflow, 16 rounds, the least overflow kept), vias at the
router's cost, the router's preferred directions and plane cut cost.
Output: overflow per boundary and layer, the score (total, largest and
soft overflow, ACE, overflowed, unrouted and detoured nets) and per-net
corridors. `pcb-maker global-route-kicad-board` (the layout's lowering;
RUDY and, with `--probe`, the race's probe beside it; `--corridors`),
`congestion_score: "global"` in layout.json (the race's pre-ranking by
the global route's usage above half the capacity; opt-in), run.py
`--layout` (layout settings for every board), `PCB_GLOBAL_DEBUG`.
Time: median 0.26 s a placement over 218, katia 2.6 s, Batch_1 3.2 s, the
laptop motherboard 0.24 s.

**Validated** (`experiments/global-route/validate.py`). Our v18
placement against the designer's on every board (209 placements with a
final result, 43 boards whose finals differ): the soft overflow at 0.5
orders 0.791 of the pairs as their finals (RUDY 0.523, the global
route's wirelength 0.663; on the 23 pairs with probes on both sides the soft overflow 0.783, the 75 s probe 0.739, RUDY 0.543), right on 12 of the 18 boards that
pass the strict rule on one placement only (8 of the 11 decided by open
connections; 7 differ only in starved thermals and other findings).
Robust: 0.70-0.79 at tiles of 1.0, 1.5 and 2.0 mm, with 0.7 of the
capacity, or measured on three lines. The placer's seeds of one board
(the historical cohort of 82 final layouts where the probe's 0.55 and
RUDY's 0.30 were measured): within-board Spearman 0.327, pair accuracy
0.642, about RUDY (0.300, 0.630), below one probe (0.526, 0.735), and
unstable across the variants (0.0 to 0.33). Reading: it tells a
structurally less routable placement from a routable one, not one seed
from another; the phase-1 rule holds on its paired clause only.

**Prototype** (the paired clause held): the race's pre-ranking by the
global route (`congestion_score: "global"`, x1442) against RUDY (x1385)
on the eleven dense boards at the task budgets: KiCad unconnected 1096 ->
1140, strict passes 0 -> 0; seven boards identical (four keep only three
seeds, nothing to rank; three keep the same seed), protoesp the same seed
and one more open (its continuation's work moved by the other probes'),
4in1-mini 57 -> 74 and DSP-ADAU1452 49 -> 56 (seeds with better probes
that finished worse), the 0xCB panel 65 -> 84 (RUDY's best seed ranked
out). Not a default. Next: use the measure where placements differ in
structure (an anneal term or moves), not between seeds; the corridors
for the detailed router.

**Unchanged by default.** Short tier (6 x 2, `--no-lock`) x1441 against
build/short-charge-x1381: 12 rows and the 12 result boards identical.
Fingerprints (Castor, k30-SBC, jetson, Sisu; route mode from v16, 2 x 4,
budget scale 0.25) x1441 against build/fp-fence-x1385: every fingerprint
and every board identical. (x1442 differs from x1441 only where the
score is `global`. All three snapshots are this package on 043c247, as
x1385 is; the branch was rebased onto later main afterwards, with no
conflict outside this changelog.)
and every board identical.
## Committed on 2026-10-21 (agent contract review fixes)

The sure findings H1-H3 and M1-M4 of
[the agent contract review](reviews/2026-10-21-review-agent-contract.md)
(main at 20956b3). Branch agent-contract-fixes; snapshots x1470 (the
measurement) and x1471 (the commit, rebased on the fab-release package).

**H1, a router config file keeps the project's rules.** A file's rules
are now the project's with the file's keys over them, key by key
(`project_rules::router_config_over_project`): before, a file without
classes dropped `solder_mask_to_copper_mm` and
`solder_mask_bridge_severity` (a project's mask clearance was lost, a
project rating bridges as warnings got the strict mask rule), and a file
with `default_rules` or `connection_rules` dropped every project rule
(edge clearance 0, hole clearances 0.25, no custom rules). The classes
(`connection_rules` and `default_rules`) count as one key, since the
project gives every net with pads its entry; the custom rules' per-net
floors and pair clearances always stay. A file naming
`edge_clearance_mm` or a hole clearance is now taken at its word (before,
a file without classes got the project's). `PCB_SKIP_NATIVE` works with
a file too. Tests: `{"seeds": 4}` lowers to the same board and rules as
`auto` (the Debug of the lowered board and of every rule field), and a
file with `default_rules` keeps the project's edge, hole and mask rules
unless it names them.

**H2, kicad-cli before any work.** `route-kicad-board`,
`layout-kicad-board`, `drc-kicad-board` and a schematic's
`import-kicad-netlist` ask `kicad-cli version` first (the cached probe):
missing, failing or below KiCad 10 is an input error, exit 2, its
verdict (the output directory holding only it, which a retry may use)
and its line naming `KICAD_REQUIREMENT` (KiCad 10's kicad-cli on the
PATH or in `PCB_KICAD_CLI`, new: the program every kicad-cli call runs).
A route or layout with KiCad's check off, a placement and a netlist's
import go on with the problem as a warning in the verdict's next actions
(on stdout). The import's netlist export failing on the schematic, or
kicad-cli not starting, is the input's error (exit 2, as its help said).
A kicad-cli timeout names `PCB_KICAD_TIMEOUT` in its message and next
action. agents.md states the requirement at the top.

**H3, KiCad's libraries off Linux.** The import looks for KiCad's own
footprints in `KICAD10_FOOTPRINT_DIR`, `KICAD9_FOOTPRINT_DIR`,
`KICAD8_FOOTPRINT_DIR`, then the first existing of
/usr/share/kicad/footprints, /usr/local/share/kicad/footprints, the macOS
bundle's SharedSupport/footprints and %ProgramFiles%'s KiCad, and for
the user's table under `KICAD_CONFIG_HOME`, `$XDG_CONFIG_HOME/kicad`,
~/.config/kicad, ~/Library/Preferences/kicad and %APPDATA%\kicad (10.0
before 9.0). There is no symbol lookup to extend: kicad-cli resolves a
schematic's symbols itself. Documented in agents.md and the environment
index.

**M1, the keepout warning with an outline.** The input check now
applies the constraints' outline first, as the placement does (the
import always writes `"outline": {}`), so a keepout over a held part is
warned of before the run in the usual flow too; the placement's
`constraint_warnings` (board-placer.json) follow the input check's in
the verdict's next actions (place and layout, without repeats).

**M2, the free dimension.** With `accept: "pass"` a size clean by that
rule is the search's `complete` verdict (exit 0), the layout's own
status kept as `free_dimension.layout_status`
(`free_dimension::accept_verdict`); the not-clean next action names
`free_dimension.probe_seconds`; with no feasible size `verdict.json`
(exit 2, a next action naming the ranges and `band_mm`) and
`free-dimension.json` (every size and its reasons) are written.

**M3, next actions name a step.** "new DRC errors" is followed by one
action per type, the four largest: KiCad's rule (the description's
parenthesis), the nets (from the items' `[net]`) or the first items, and
the setting or step (clearance and shorts: the nets' parts or an
unmodelled rule; edge: the edge constraint's `max_mm` or `flush`;
`items_not_allowed`: the rule area; `solder_mask_bridge`: the project's
mask rules; `courtyards_overlap`: `placer.tight_bodies`;
`starved_thermal`: `solid_starved_thermals`, also when the router counted
none). Open connections name the nets KiCad finds unconnected. The
silkscreen repair's incomplete line names silkscreen-repair.json and the
command to run again. KiCad's DRC has no via-in-pad finding, so no
action names `via_in_pad`.

**M4, docs.** `move_seconds` and `total_seconds` are seconds of work;
`automatic_planes` puts the rail on the last inner layer (router.md's
line on existing tracks is the fab-release package's); placer.md: `row`
exists, the layout moves labels and texts; copper at the edge is
described once, under `edge` in constraints.md.

**Measurements** (x1470, before the rebase on the fab-release package;
`PCB_WORK_RATE=3.15e6,150e6`). No task of the tiers has a router config
with `default_rules` or `connection_rules`, so H1's only change on them is
the mask keys, which route mode's budget file (`ladder_budget_seconds`)
dropped: on the fingerprint boards and the short tier's they are 0.0 and
`error` or absent, which lower as before (OpenFC rates bridges as
warnings, but the short tier is layout mode with `auto`). Short tier (3 x
2, layout mode) build/short-contract-x1470 against
build/short-fence-x1403: every row and every result board identical (6/12
pass, strict 5; unconnected 20, new errors 2, vias 1088, length 20673 mm,
work 2961 s); only the verdicts' next actions are longer (WLED_Matrix: the
nets KiCad finds unconnected, GND (2) and three more, and clearance,
hole_to_hole, hole_clearance and connection_width each with KiCad's rule
and nets). Fingerprints (route mode from v16, 1 x 4, budget scale 0.25,
`PCB_ROUTER_FINGERPRINT=1 PCB_ROUTER_NO_WALL_GUARD=1`): at a memory limit
of one router (`PCB_MAKER_MEMORY_MB=2500`, the probes one at a time)
build/fp-contract-m-x1470 against main's x1404 run beside it
(build/fp-contract-m-x1404): every fingerprint line and all four result
boards identical (jetson 324/340, 20 unconnected; Sisu 171/192, 44; k30
and Castor complete). With free memory (build/fp-contract-x1470-free,
stopped after two boards) jetson and Sisu are fp-base-x1404's rows; the
task's reference build/fp-fence-x1402 predates bbdb205, and the probes
side by side follow free memory (Sisu: 3 of 4 here, 1 of 4 in x1402;
the ladder review's finding), so it differs for both.
## Committed on 2026-10-21 (ladder review fixes)

The fixes of docs/reviews/2026-10-21-review-ladder-work.md. Branch
ladder-review-fixes; snapshots x1520 (the first version: the probes'
maps released when the probes end), x1521 (carried into the regular
pitch; the measurements) and x1522 (x1521 rebased on 2094b6b, the
pre-routed nets: a rung the ladder lowered has its kept copper from the
runner's lowering; Vortac's determinism run identical to x1521's).
Details: docs/router.md, "The canonical charge".

**Finding 1, the charged work by the free memory.** A router's build is
now priced as if it built everything itself (`Router::build_work`,
`build_counts`), whether it built its static maps and pour masks or
found them, and names the parts routers of the same inputs share, keyed
(`Router::shared_build`, `SharedBuild`: the static maps beyond its own
maps, the connected pours' masks). The ladder charges each shared part
once a phase (the probes with the regular pitch after them, each finer
pitch, each seed round, each step; `Ladder::new_phase`,
`lattice_charge`), the first time a router of its key is charged in the
rungs' order, and the routers their own maps. Of the review's two
options this is the second (the hold-once one, made independent of
waves by keying the charge on the ladder's order, not on which hold ran
first): its total is what the ladder charges running one router at a
time, each phase building each key's maps once and the later routers
sharing them, and what is charged once is built once (the ladder holds a
phase's maps: every rung's, a lone rung's as a wave's). The first option
(a found map at a whole build's price) would charge four lattices for
four probes that build one, and `retry` builds for seeds that share one:
work no run does. The scripted-runner test
`route_mode_charges_the_same_one_at_a_time_and_side_by_side` (probed
four-layer, unprobed four-layer and two-layer ladders with the memory of
one router at a time and of 32 GB) asserts the same charges, work and
kept board; before, a wave's hold lowered its rungs a second time (and
only a wave), so it fails on the lowerings alone.

**Finding 2, the probes' hold.** Held and charged only once the first
probe's router passed the memory gate (before, built and charged before
it, also when no probe then ran); the gate counts the other rungs'
static maps beside the two routers (`too_small_for_beside`, the
distinct keys beyond the first router's at its `statics` size), and the
probes charge their maps as they are judged (a probe the order never
reaches charges nothing, as its search).

**Finding 3, retention.** The strong `cache.1` is gone: the static maps'
cache and `POUR_MASKS` are weak entries, held by routers and holds
alone. The ladder's phase holds are its explicit retention: released
when the pitch ends, before the seed retry (all but the retried rung's
maps), per seed round and per step; the probes' holds go on into the
regular pitch (its rungs lower the same boards) and end with it.
`MemoryLimits` names what is held (`held: static maps N MB, pour masks M
MB` in the `memory limits:` line, `held_maps_bytes`); it is in the
resident size every limit's free memory counts. The pour masks need no
byte bound now: nothing outlives the holds. A layout's move routers no
longer keep the last router's maps.

**Finding 5, lowerings.** A rung's board is lowered once, for its hold,
and routed as lowered (`LadderRunner::route_lowered`); seeds charge
their one lowering once (it was `retry` times). The probes' lowerings and
the regular rungs' are both done and each charged once.

**Finding 6, `run_work`.** Search work against search work: the rungs'
and the continuation's total is the budget at the routers' search
fraction (`PitchPlan::total`, `built_router_work`), as what is left is.

**Finding 7, a negotiation of nothing.** `negotiation_expansions(seconds)`
is at least one expansion (0 means no limit): a bound of nothing stops
the negotiation after its first iteration by its own limit (probe,
resume, `core_config`).

**Finding 8, the panic hook.** main.rs keeps the last panic's message
and place; the uncaught panic is reported by its own payload (with the
hook's place when that panic is the last one), and `is_closed_stdout`
tests the uncaught panic's message (unit test
`the_uncaught_panic_is_reported_with_its_own_message`).

**Finding 9, the build race.** A router that loses the race of two
builds of one key did the build: it counts as built (`shared_statics`).

**Finding 10, the wall accounting.** The holds' builds count in
`BUILD_WORK_NANOS` and a router counts what it built
(`physical_build_work`), so `wall_accounting.lattice_work_seconds` is
the lattice work done (the charged lattice work where the ladder charged
every router built: Vortac 1.5 to 1.6 s done for 1.3 charged, the
difference small routers built before the ladder and a wave's rung the
order never reached; x1404 had 0.5 done for 1.3 charged, the holds
missing).

**Nit.** `PCB_LADDER_UNCHARGED_SPOKES` is `PCB_LADDER_SEARCH_ONLY` (it
turned off all charging beyond the search and the shares' conversion,
not only the spokes); config reference regenerated.

**The determinism check** (each board alone, route mode at 30 s, one
thread, the reference's rates pinned, `PCB_ROUTER_NO_WALL_GUARD=1`,
`PCB_MAKER_MEMORY_MB` small enough for one router at a time where the
probes' gate still passes, then 64000; Vortac's panel, MSign, ChengminLi's
flyback; work and the charges from `wall_accounting`):

| board | side by side, small / large | x1404 small | x1404 large | x1521 small and large (identical) |
|---|---|---|---|---|
| Vortac_Panel_REV1 (1900 MB) | probes 2, 1, 1 then rungs 1 / probes 4, rungs 3 | work 34.9726, lowerings 0.0331 | work 34.9792, lowerings 0.0397 | work 34.9726, lattices 1.2990, lowerings 0.0331 |
| MSign (3200 MB) | probes 1 / 4 | 32.346 | 32.346 | 32.346, lattices 1.3466 |
| ChengminLi flyback (4000 MB) | rungs 1, seeds 1 / rungs 2 | 24.1777, lattices 3.460, lowerings 0.0091 | 24.1800, lowerings 0.0113 | 23.9813, lattices 3.2667, lowerings 0.0057 |

Rows and boards are identical in all twelve runs (Vortac 50/61, 125
unconnected, 54 vias; MSign 155/188, 346, 183; ChengminLi 35/38, 4,
11; the result boards byte for byte per board). On x1404 the charged
work already differed with the memory (the wave's second lowering;
none of these three had a router find another's maps in a way that
moved the lattices) without moving a board; on x1521 it does not.
x1520 released the probes' maps when the probes ended, so Vortac's
regular rungs rebuilt them and charged 2.265 s of lattices (x1404 had
charged them as found, the last held maps still in the process); x1521
carries them into the regular pitch: 1.299, x1404's.

**Expansion tier** (8 x 1, `--no-lock --min-free-gb 14`, no OOM kill)
x1521 against build/expansion-fence-x1404.log: all 46 rows the same
result and the 46 result boards byte for byte; work moved on four rows
by 0.1 s (Miniboard 28.9 to 28.8, LC_PAMV4b 27.5 to 27.6, Barklind
25.2 to 25.3, pdlc 31.5 to 31.6: their seeds' one lowering, a found map
now charged at its canonical price), not enough to move a share past a
decision. Against build/expansion-probe-x1394.log it is x1404's
comparison exactly: unconnected 786 to 750, passes 16 to 18, new
errors 4 to 2, better 13, worse 10.

**Short tier** (3 x 2, `--no-lock --min-free-gb 14`) x1521 against
build/short-fence-x1403.log: all 12 rows identical and the 12 result
boards byte for byte (a layout's ladder charges search; the maps are
the same maps).

**Fingerprints** (Castor, k30-SBC, jetson, Sisu; route mode from v16,
1 x 4, budget scale 0.25, `PCB_ROUTER_FINGERPRINT=1
PCB_ROUTER_NO_WALL_GUARD=1`) x1521 against build/fp-fence-x1402: Castor
and k30 every fingerprint and the board identical (work 170.5 to 174.2
and 285.6 to 284.6 s: x1404's as well); jetson and Sisu changed, but
their boards, rows and work are build/fp-base-x1404's exactly (jetson
20 unconnected, 597 vias, 10784.0 mm; Sisu 24, 386, 5634.7 mm): the
change is x1404's (its routing after x1402), none of this package's
(Castor's work 174.1 to 174.2 against x1404, its probes' lattices at
the canonical price).

Unit tests: route_mode_charges_the_same_one_at_a_time_and_side_by_side,
route_mode_charges_all_the_work (one lowering a seed round),
routers_of_one_board_and_lattice_share_their_static_maps (the canonical
counts, the shared parts and their keys, the hold's work, nothing kept
after the holders), a_negotiation_bound_of_nothing_is_one_expansion,
the_uncaught_panic_is_reported_with_its_own_message. `bash
scripts/check.sh` and the clippy gate pass. Docs: router.md (The
canonical charge, the static maps, the work bound's switches), config
reference (`PCB_LADDER_SEARCH_ONLY`, `share_static_maps`).

Logs: build/expansion-ladfix-x1521, build/short-ladfix-x1521,
build/fp-ladfix-x1521; the determinism runs' boards and logs were in
the agent's scratchpad (ladfix1-det).
## Committed on 2026-10-21 (net ties, same-number pads, starved thermals)

The three cheap fixes of the v18 gap analysis (docs/benchmarks.md,
"Layout sweep v18"): net ties, pads of one number, and the starved-thermal
default. Branch net-ties; snapshots x1430 (net ties), x1431 (and the
solid default), x1432 (and joined unreachable pads), x1433 (and island
pads solid), x1434 (and the solid board kept only when better; the
measured binary), x1435 (rebased on the probe and fence packages), x1436
(and the layout's solid step on its kept board only; the commit, rebased
since on the fab-release package, whose route mode keeps a board's own
tracks: the tiers strip first). The before-binary of the other
pairs is x1385 (main 043c247, its fingerprints those of x1381); the
after-binary x1434 has neither the probe-overhead nor the fence package
(both landed while these ran), so its pairs isolate this package; x1436
is measured against main's short-tier reference (x1403).

**Net ties** (KiCad 7's `(net_tie_pad_groups "1, 2")`; on a board before
version 20220817 a footprint whose keywords start with "net tie", all its
pads). The brief's model (a tie's pads are one terminal of every net in
the group, connections through the tie satisfied) is not KiCad's: kicad-cli
10.0.7 on a NetTie-2_SMD_Pad0.5mm between nets A and B with a pad of each
elsewhere: a track of A to the tie's pad 1 alone leaves pad 2 and B's pad
unconnected; tracks of A and B to their own pads find nothing; a track of
A over the bridge onto pad 2 is a short. A tie joins no nets for the
connectivity; each pad is an item of its own net. What left neotron-pico's
24, ghoul's 6 and protoesp's NT pads open was the bridge: the footprint's
copper polygon, lowered as copper of no net, overlaps both pads, so every
tie pad was dead (`diagnostics.dead_pads`: "near NT901 copper fp_poly").
KiCad lets the tie's own nets touch the bridge anywhere (an A track ending
on its middle: clearance against pad 2 only, none against the polygon) and
keeps other nets clear of it (without `net_tie_pad_groups`: clearance and
shorting_items against the polygon from both tracks and pads). A lattice
node has one owner, so the lowering keeps the bridge as copper of no net
less a disc around each tie pad's centre of its net's clearance, half its
track width and 0.1 mm (`carve_net_tie`, cells of 0.02 mm; a graphic no
disc touches stays exact). The legacy rule, by kicad-cli on the same board
at versions 20211014 to 20220817: a tie by `(tags "net tie")` and "net tie
short" before 20220817, not at it, not by "Net Tie", "foo net tie" or a
description (ghoul is a KiCad 6 board with tags only). The router's
connection list and the verifier needed nothing: the nets were always
routed apart, as KiCad counts them. Routed with x1434, the tie board of
the test: 2/2, KiCad 0 unconnected, no finding (x1385: 0/2, both pads
unconnected).

**Pads of one number** (jiran-ble-lite, interchange-pcb-right,
capybully). interchange's and capybully's open items in v18 and the
designer tier were stacked F.Cu and B.Cu pads of one number on x1315: one
terminal then, two since the connectivity agreement (x1345). On x1385
both route complete with KiCad 0 unconnected (capybully passes strict,
interchange fails strict on its pads over the edge only). jiran's are not
pads of one number: its file holds a split keyboard's two halves, joined
by a frame whose tabs carry mouse-bite holes no track crosses, and both
halves' pads are GND. The net's tree grew on one half; the other half's
20 GND pads (J4's four S1 shield pads among them, each a terminal of its
own) were left out as unreachable, and apart from each other too: KiCad 20
unconnected, the designer's own board 1 (the halves' GND). Now the
terminals a board-wide search proved unreachable are joined among
themselves after the tree (`Config::apart_trees`,
`PCB_ROUTER_NO_APART_TREES`; `route_apart` in route_net.rs, a tree from the
first such terminal grown by searches from it alone): jiran 20 to 1 on its
v18 placement. They stay unconnected by the router's count and the net
partial (KiCad finds one item between the trees, not one per pad).

**Starved thermals.** The census (`experiments/pour-fidelity/census.py`,
the 32 boards of the 44 still on disk and the 42 of route-t8, 218 starved
pad rows) on x1385: predicted 165, missed 49, false alarms 29, no model row
4, row for row x1363's (nothing in the pour model changed since). The
switch does not depend on the model: it acts on KiCad's findings after the
ladder. On the v18 placements (x1385) two of the four boards no longer fail
on starved thermals (rosco and coco2 pass strict: the spoke end test, the
via reliefs and the transitive merge of the pour package); mackerel-30-proto
(4 strict) and k30-SBC (3) do. mackerel's four were "spokes connected to
isolated island" findings, which the solid step skipped (`spokes` none);
solid, the pad joins the island and the finding goes (kicad-cli on its
route: R14.1, U2.42, U2.43, U11.61, four to none, nothing else). k30's four
solid took the starved findings away and left J1.39, between J1.14 (made
solid) and J1.15 on a 0.635 mm card connector, unconnected twice (KiCad 0
to 2): a solid pad changes the fill. So the step now keeps the solid board
only when KiCad finds no more unconnected items and fewer findings
(`solid_kept`), else restores the board and its reports. In a layout the
step ran inside each `route_kicad_board` of the ladder and the
continuation, and the layout compares those by (open, starved, vias):
eternal-keypad's continuation with its starved thermals gone (and a short
of a GND track with a ProMicro hole the comparison does not weigh) beat
the board kept before (short tier x1434: per-type pass to shorting_items
1). The layout now turns the step off for its attempts and runs it once on
the board it keeps (`solidify_starved_thermals`, x1436).

Decision: **`solid_starved_thermals` is on unless false.** It flips
mackerel-30-proto (v18 placement) and Castor (the fingerprint route) to
strict passes, takes k30's fingerprint route from 4 strict starved
thermals to 1, and costs nothing on the tiers (below); where it would
cost, the board is restored. Its costs: one KiCad check on a board with
starved pads left (charged as work), and the designer's thermal reliefs on
those pads (a soldering choice; false keeps them). k30-SBC's v18 route keeps
its 3 strict starved thermals (solid not kept).

**Route mode on the v18 placements** (tasks v7, the task budgets: 1200 s
of work, 3 x 2 and 2 x 2 at --min-free-gb 14; x1385 against x1434; jiran,
ghoul, protoesp, mackerel, coco2, capybully and interchange from x1433,
which differs from x1434 only where the solid board is not kept: k30 run
again on x1434, ghoul too, its x1433 row):

| board | x1385 | x1434 | cause |
| --- | --- | --- | --- |
| neotron-pico | 176/190, KiCad 24 | 190/190, KiCad 0, strict pass | net ties (NT901-NT912) |
| ghoul | 142/148, KiCad 6, clearance 1 | 148/148, KiCad 0, clearance 1, strict +starved 1 | net ties (NT1-NT3, KiCad 6 tags); 5 starved pads solid, 1 left |
| protoesp | 67/104, KiCad 58 | 75/104, KiCad 56 | net ties (NT items 12 to 5; the rest is placement) |
| jiran-ble-lite | 104/105, KiCad 20 | 104/105, KiCad 1 | unreachable pads joined |
| mackerel-30-proto | 217/217, starved 2 (strict 4) | 217/217, strict pass | island pads solid |
| rosco | 148/148, strict pass | the same row | - |
| coco2 | 173/173, strict pass | the same row | - |
| k30-SBC | 173/173, pass, strict starved 3 | the same row | solid not kept (KiCad 0 to 2) |
| capybully | 32/32, strict pass | the same row | - |
| interchange-pcb-right | 83/83, KiCad 0, strict copper_edge_clearance 8 | the same row | - |

**On the designer's placement** (the designer tier's boards,
build/designer-x1315/<task>/placed, route-kicad-board alone at the tier's
1500 s of work, `--reference none`, outside the lock under the
coordinator's rule for single routes; build/ties-designer-single):
neotron-pico 176/190, KiCad 24 to 190/190, KiCad 0 and no finding
(complete); ghoul 142/148, KiCad 6 to 148/148, KiCad 0 (hole_clearance 40
both, the designer's; 1 starved pad solid). As the v18 analysis expected,
both designer routes are complete now. protoesp and jiran were not run
there (the lock's queue; their v18 rows above).

**Fingerprints** (route mode from the v16 layouts, quarter budget,
`PCB_ROUTER_FINGERPRINT=1 PCB_ROUTER_NO_WALL_GUARD=1`, build/fp-ties-x1434
against build/fp-ties-x1385, 1 x 4 for memory; x1385's are x1381's): every
fingerprint identical on Castor, k30-SBC, jetson and Sisu (none has a net
tie, none a group of unreachable pads); the boards of jetson and Sisu
identical; Castor's and k30's differ only by the solid step: Castor 4
starved pads solid, a strict pass (was strict starved_thermal 4); k30 4
solid, 1 starved left (strict 4 to 1).

**Expansion tier** (46 boards, 8 x 1, x1434 against
build/expansion-charge-x1381.log; no row cut or killed): 41 rows identical
(the solid step does not run: the tier skips KiCad's check in the router);
passes 16 to 17 (strict 16 to 17), unconnected 846 to 864, new errors 2 to
2, vias 4006 to 3966. Changed rows: FUEL4EP 58/60, KiCad 2 (both NT1's
pads) to 60/60, a pass; brown-studios pdlc 21/33, KiCad 12 to 22/33, 11
(NT1's pads joined); brown-studios minuet 252.1 to 252.0 mm (its tie);
mincrmatt12 MSign 162/188, KiCad 75 to 145/188, 96: its four tie items
(NT1, NT2: VSEN to GND and 5V_REG) joined, the rest the 30 s budget on a
board that does not fit it (its probes 163 to 172 nets unfinished from the
changed lowering; its row has moved between 15 and 75 unconnected over the
last packages);
CASE-Association Julkort 32.9 to 33.0 s of work, the same board (two
nets' unreachable pads joined apart in an attempt the ladder did not keep;
with `PCB_ROUTER_NO_APART_TREES` 32.9 s, x1381's row).

**The connectivity list** (connectivity-tasks.json, 65 boards, 30 s, 8 x 1,
`census.py --run build/conn-ties-x1434`): verdict agrees with KiCad on 65
of 65, complete by our count with KiCad unconnected items on 0 boards
(build/conn-x1365: 1, BioGenius, fixed since by the lowering gaps 2
package), KiCad's unconnected items 9 (x1365 11), no row killed. The tier
skips KiCad's check in the router, so the solid step does not run there.

**Short tier** (tasks v7, 3 x 2, `--no-lock` under the coordinator's
memory rule). x1434 against build/short-charge-x1381.log: 11 rows
identical (MIDAS's too), eternal-keypad per-type pass to shorting_items 1
(the solid step inside the layout's attempts, above). x1436 (rebased, the layout's
step on its kept board) against build/short-fence-x1403.log (main's
reference): 10 rows identical; framework_mobo_lefthalf strict
starved_thermal 1 to a strict pass (1 pad solid, 3.6 s of work for the
check); eternal-keypad per-type starved_thermal 1 (strict 4) to a
per-type pass (10 pads solid: its ProMicro through-hole pads; strict now
only the designer's capped hole_clearance). Passes 6 to 7, strict 5 to 6,
unconnected 20 both, vias and length identical.

Unit tests: pcb-kicad's `a_net_ties_pads_are_reached_by_their_own_nets`
(the lowering's terminals and bridge, and a route of the tie board, with
the kicad-cli findings above), `a_kicad_6_footprint_tagged_net_tie_is_a_tie`,
`starved_pads_stay_solid_only_when_kicad_finds_the_board_better`;
pcb-router's `terminals_no_path_reaches_are_joined_among_themselves`.

**Gates**: `bash scripts/check.sh` and the clippy gate pass. Other
packages' code touched: route_net.rs (`route_apart`, the router's core
search, owned by no package), board_layout.rs (the layout's solid step,
two config fields and one call), agent_cli.rs (the help's default).

## Committed on 2026-10-21 (fab release, pre-routed nets, agent skill)

Three agent-facing features from the Hacker News evaluation (commenters
asked for a DRC-gated headless fab release, for pinning key traces by
hand and autorouting the rest, and for agent skill files).

**fab-kicad-board** (`crates/pcb-kicad/src/fab_release.rs`,
`src/agent_cli.rs`): `fab-kicad-board <dir> <id> <out> [--allow-findings]
[--layers ...] [--profile kicad|jlcpcb] [--no-step]`. KiCad's DRC as
drc-kicad-board runs it (the DRC cache, zones refilled, the library-free
configuration) and, with a schematic, the ERC (counted, never blocking).
Copper errors or unconnected items refuse the release: exit 1, nothing
exported, the first findings in `next_actions`; cosmetic findings do not
block, `--allow-findings` overrides. Released (exit 0), through kicad-cli:
Gerbers of the layers the board uses (every copper layer, both masks,
Edge.Cuts, the pastes and silkscreens something lies on), Excellon drill
files (PTH and NPTH, mm) with Gerber maps, `<id>-gerbers.zip` (written by
a small stored-zip writer: the same files give the same zip byte for
byte), the position file (CSV, both sides, KiCad's rotations, DNP out),
the BOM (kicad-cli's `sch export bom` grouped by value, footprint and the
symbols' LCSC field when there is a schematic, else the board's
footprints), IPC-D-356 and the schematic's netlist, the STEP model (a
failure goes to `skipped`), and `fab-manifest.json` (every file with kind,
bytes and sha256; the DRC and ERC counts; KiCad's version; layers;
`without_lcsc`). `verdict.json` follows the verdict contract and adds
`fab_release`. `--profile jlcpcb`: every layer of JLCPCB's KiCad guide and
`assembly/<id>-bom-jlcpcb.csv` (Comment, Designator, Footprint, JLCPCB
Part #; designators listed, no ranges) and `assembly/<id>-cpl-jlcpcb.csv`
(Designator, Mid X, Mid Y, Layer, Rotation). JLCPCB's Gerber names are
KiCad's Protel extensions, so the profile renames nothing; their rotation
offsets per package are not applied (documented). The board BOM of
export-kicad-fab became `fab_export::board_bom`, shared.

Measured (x1450, kicad-cli 10.0.7): the walkthrough's keyboard (urchin
from its schematic, laid out at `"total_seconds": 30`, `--profile
jlcpcb`): released in 8 s, 24 files, `main-gerbers.zip` 14 files (693
kB), STEP 500 kB, DRC 0 errors, 0 unconnected, 82 cosmetic; ERC 42 errors
(the schematic's own); 80 parts without an LCSC number. Short tier
results (x1403's layouts): spell_tome_bottom released in 5 s (19
files; no B.Paste, nothing lies on it; 49 cosmetic findings); Blue Line (`--profile jlcpcb`) released in
4 s (21 files, 206 cosmetic); USB Keypad refused (exit 1: clearance 58,
connection_width 1, hole_clearance 5, hole_to_hole 12, 2 unconnected).

**Pre-routed nets** (`crates/pcb-kicad/src/board_router/kept.rs`). Found
before the change: route mode lowered the board's own segments and vias as
obstacles of their nets (other nets kept clear of them), did not count
their connectivity (every connection was routed anew), and `emit_routes`
then deleted every segment, arc and via of the input: the result had only
the router's copper, other nets having avoided copper that was no longer
there; existing arc tracks were refused ("the board router does not yet
lower existing arc tracks", an internal error). Now route-kicad-board
keeps them by default (router config `existing_tracks`: `keep`, or
`strip` and the command's `--strip`, which removes them before anything
reads the board): the lowering's existing tracks and vias stay fixed
copper of their nets; the pads a piece of them joins (copper of the net
overlapping on a shared layer, custom pads through their contact, as
`verify::copper_components` joins items) are merged into one terminal
(`MergedPad`, as pads of one number that overlap are), so the connection
counts as made and the router routes only what is open; a piece on no pad
is a terminal of its own unless its net has a pour. The router never owns
kept copper, so nothing rips it up; `emit_routes(keep_existing)` writes it
back (layouts still replace). Arc tracks are lowered as chords grown by
the flattening tolerance (a fix in the lowering's handling of existing
tracks; no board of the tiers has them after their strip).
`board-router.json` reports `kept_copper` (segments, arcs, vias, nets,
connections_made, nets_complete, floating); absent without kept copper,
so reports of track-free boards are unchanged. Limits (docs/agents.md):
new routes join a kept group at its first pad, not along a kept track; a
pour net's kept track ending in the pour does not mark its pad connected.

Measured on spell_tome_bottom (the designer's board stripped as run.py
strips it, then the designer's 3V3 and SDA tracks put back: 33 segments, 2
vias; `ladder_budget_seconds` 400, x1450): kept, 11 connections made, both
nets complete with their tracks alone, the other 16 nets routed (146
items), KiCad complete, exit 0, 21 s; 3V3's 28 and SDA's 7 items in the
result are the input's token for token (uuids included; the writer
re-indents the whole file, tabs to spaces). With `--strip`: 18 nets
routed, 0 of the 35 input items left, 3V3 45 items and SDA 9, 16 vias and
601 mm against 12 vias and 447 mm kept, also complete.

**Agent skill** (`skills/pcb-maker/SKILL.md`): frontmatter (name,
description) and the instructions: workflow, commands, the verdict fields
and exit codes, what to do on exit 1, constraints.json and layout.json
keys, the free dimension, pre-routed nets, the fab release. README, "For
agents", says how to install it. `tests/cli_exit_codes.rs` keeps it from
going stale: every `*-kicad-*` command it names is in `--help`'s command
table and in docs/agents.md, every agent command is in it, and its exit
table matches `--help` and docs/agents.md.

Tests: unit (kept copper: merging, a loose track, a net complete with its
tracks, the router leaving it alone, a board without tracks unchanged,
arcs; the zip and its CRC, read back by Python's zipfile; JLCPCB's
placement columns; the layers a release plots) and CLI (route keeps two
pre-routed nets and routes the third, `--strip` routes all three; the fab
release refused, allowed with `--allow-findings`, every manifest file's
size; usage errors; the skill against the command table).

No behaviour change without kept copper: the short tier (3 x 2) x1450
against x1403 is identical, 12 rows and the 12 result boards byte for
byte (20 unconnected, 6 passes, work 2961 s). Fingerprints (route mode
at a quarter of the budget, 2 x 4, the v16 placements): against
build/fp-fence-x1402 Castor and k30 give the same boards, jetson (10804.2
to 10784.0 mm) and Sisu (23 to 24 unconnected) differ, but x1402 predates
the probe-overhead commit (bbdb205: it lacks `PCB_LADDER_NO_STEP_COST_GATE`);
against main's own snapshot x1404 (ded766d), run beside it
(build/fp-base-x1404), all four rows and all four result boards are
identical (jetson 324/340, 20 unconnected; Sisu 184/192, 24; k30 and
Castor complete). Snapshots
x1450-x1451.

**Short tier** (3 x 2, `--no-lock`, `--min-free-gb 14`) x1513 against
build/short-fence-x1403.log: 12 rows valid, every count the same (passes
6, strict 5, unconnected 20, vias 1088, length 20673 mm, work 2961 to
2955 s); 10 result boards byte for byte, the placements all identical.
MIDAS and OpenFC move one via each by one pitch (0.1 mm; finding 5: a
node beside a pin of the via's net inside single-layer copper of that
net is the net's now in either pad order), the same counts. x1511
(before the placer's `placer_net`) had moved MIDAS's, USB_Cable_Tester's
and eternal-keypad's placements through the jumper nets (MIDAS 7 to 11
unconnected), hence the fix above.

**Checks.** `bash scripts/check.sh` passed at the commit; the clippy
gate. `a_placement_does_not_depend_on_the_order_of_the_file` failed in
some runs before the rebase, on the branch's first commits as on main
(load-dependent), and passed in check.sh.

**On the net-ties merge** (rebased on 9b1e1ff, x1514 against main's
x1530, 30 s, one thread): MSign 69 and 69 unconnected (192 vias both,
boards differ; the first probe 208 to 173 nets unfinished, the ladder
ends level: the 71 to 42 above was against x1404); Sisu (v18, budget
scale 0.025) 170 to 158 unconnected, 206 to 243 vias. check.sh passed.

**Fixture goldens** (tests/fixtures, rebased on b0791ae): F1's lowering
gains one line, TP1.1's back padstack shape merged into its terminal
(finding 2); F1's routing fingerprints change from the first
negotiation: TP1.1 is 1.2 mm on F.Cu and 1.8 on B.Cu, and the terminal's
B.Cu nodes now lie in the 1.8 mm shape (before in the 1.2 mm front shape
on every layer). Both rewritten with `PCB_UPDATE_GOLDEN=1`.

**Not done.** Findings 6 (the redundant terminal scan in
`copper_components`) and 9 (the per-opening obstacle scans,
`net_ids_lookup`'s linear search) are left. The terminal merge's overlap
test still uses the pad's own shape's layers, so a padstack pad and a
same-number SMD pad on a layer only its stack names stay two terminals
(no tier board has one). The spokes of a padstack terminal on its own
layers use its front frame, as before; a pad's local `thermal_gap` is
not read. The free normalisation's debug assertion fires in debug
builds only; the tests and the tiers run release builds.

## Committed on 2026-10-21 (fence report: router side)

The router-side findings of a report from an agent that laid out a real
board with main at 36f24ba: the breadboard fence v5 (76.2 x 34 mm, 91
footprints, 41 WS2812B-2020 LEDs with 0.7 mm pads in a 2.54 mm row; four
layers, and two at 34 mm with 0.127 mm rules). Branch fence-report-router;
snapshots x1400 (vias out of SMD pads, keepouts, the silkscreen outline,
the refinement budget, decoupling, plane pours), x1401 (with the layout's
ladder budget), x1402 (with the hop rule), x1403 (the measurement: x1402
with the score's drill-box check), x1404 (the commit, rebased on the
probe-overhead package: the score's check against the pad's shape, the
shared static maps keyed by `no_via_pads` too). The measurements below
are against x1385 (main before the free-dimension and probe-overhead
packages, which left layouts and fingerprints as x1385); the expansion
tier also against x1394 with x1404. The placer side
(finding 2, silkscreen text as a placement obstacle) is another branch's.

**Vias in SMD pads** (finding 1; docs/router.md, "Vias in pads"). A
surface pad of a net let that net's vias in wherever the whole via fit in
its copper, so congested boards put ground vias in ground pads (the
fence: 3 in the report's run, 14 at the pinned rate on four layers, 28
at 0.2 mm rules on two): an open hole in a reflowed pad takes its
solder, and KiCad's DRC does not flag it. The lowering now lists every
SMD pad (smd and connect pads with copper, their padstack shapes) but an
exposed pad in `Board::no_via_pads`; every via keeps its ring out of
their copper, another net's its clearance beyond, the pad's own net's
may touch the pad from outside (`StaticMaps::via_owner`; before, a via
of the pad's net outside it kept the full clearance): a pad of a pour
net gets its via beside the pad. An exposed pad (the largest SMD pad,
at least twice any other's area, its centre strictly inside the box of
the centres of three other pads or more) keeps its thermal vias; a tab
beside a pin row (SOT-223, D-PAK) is not one. Through-hole pads hold no
via either way, as before. `via_in_pad: true` (router config, default
false) restores the rule of before for filled and capped vias. Unit
tests: the maps around a listed and an unlisted pad, a pour-net pad's
via beside its pad, a through-hole pad, the lowering's list (QFN, 0805,
header, SOT-223) with and without `via_in_pad`.

**Hops.** With vias out of the pads Castor's Net-(C33-Pad1) went up and
down again through two vias 0.25 mm apart (x1400: KiCad hole_to_hole,
which holds for one net too). The search no longer takes a via within
the drill plus `hole_to_hole` of the previous via of the path it
searches (`hops_from_a_via`: a walk back of a few steps per via
expansion; a net's routed branches kept theirs apart already). Unit
test: a path over a back-side wall (vias 0.4 mm apart without the rule).

**The score's check** (docs/quality.md). score-kicad-board's "vias in
pads" stays the check, now measured as the router keeps it: the drill's
disc against the pad's copper in its own frame (rotated, corners and
circles rounded), paste-only apertures and a part's exposed pad left
out. The drill's box against the pad's rotated box counted vias beside a
pad's corner (Sisu 11 after the change, 0 by the pad shapes), at a
QFN turned 45 degrees (0xCB's U3) and beside round pads (framework's
M1).

**Keepouts over fixed parts** (finding 3; docs/constraints.md). A
`keepout` box that meets the courtyard of a part the placer keeps where
it is (`fixed`, `fixed_patterns`, locked, without a net; unless `free`)
is written as a rule area with footprints allowed, so KiCad does not
report the part the input put there; the placer keeps movable parts out
of every `constraint keepout` rule area by its name whatever it allows
(board_placer/lower.rs, one condition), and with `"copper": true`
tracks, vias and pours stay out for KiCad too. The input check
(`keepout_warnings`, layout-kicad-board and place-kicad-board) warns of
such a box before the run, in the log and first in the verdict's
next_actions (`KiCadRunFacts::input_warnings`). The fence with boxes
like the report's first ones (y 3.5..6 over JP1, J2, J5; y 8.5..9.25
over TP1-TP4), place-kicad-board x1400: two warnings, those two rule
areas with footprints allowed, the third box not.

**repair-kicad-silkscreen and the outline** (finding 4). The worker took
only Edge.Cuts segments; the fence's outline is one gr_rect. It takes a
gr_rect, a polygon or segments tiling the sides (zero-length segments
and arcs left out); any other outline (curves, notches, cutouts) is its
bounding box, with a note in `proposal.notes` (`proposal.outline`:
`rectangle` or `bounding box`). The fence's four-layer result: x1385
worker exit 1 ("Rectangular board required"), 15 findings left; x1400
worker exit 0, 32 labels moved, 10 left (silk_edge_clearance of J1 and
J4 at the edge, 3 labels without a spot: C27, RN2, RN7). Unit test of
the outline function (with a stand-in for pcbnew).

**The refinement budget and the layout's ladder** (docs/router.md, "The
attempt ladder"). `refine_budget_seconds` defaults to 240 s, a fifth of
the ladder's budget where that is more (route mode: 1200 s by default,
so 240 s; the tiers' 30 s budgets unchanged), and in a layout's ladder
0.4 of the work left where that is more (`default_refine_budget`). A
layout's ladder budget is the work left unless the router config gives
one (it was at most 1200 s: at the pinned rate the fence's two-layer
layout of 3000 s stopped at 1927 s of work, "its budget of 1200 s of
work was used"). A lattice not tried names the value that tries it in
the verdict's next_actions (`"refine_budget_seconds": 367 or more in
the router config tries it`).

**Decoupling** (finding 5; docs/agents.md, docs/quality.md). U2:2, the
fence's LDO output (XC6206), reported null although C4 sat beside it on
+3V3: the pin was recognised (by its rail's name, +3V3), but C4 is 10 uF
and only capacitors up to 1 uF counted. A regulator's pins (an IC of at
most eight pads with supply pins on two rails or more) now count its
rails' bulk capacitors too, and those count as decoupling capacitors
against the regulator's pins. The fence's four-layer result: U2:2 null
to 2.16 mm, U2:3 (VBUS) 7.24 to 3.65 mm, capacitors 10 to 12, within 3
mm of a supply pin 0.6 to 0.8 of the pins. Unit test.

**A `plane` pour** (the report's smaller points). Judged cheap: an
`add_pours` entry with `"plane": true` writes, beside the zone (also
where the board has the zone), a rule area keeping every net's tracks
off the layer (`plane <net> <layer>`, vias and pours allowed), which the
router lowers as a track keepout on every rung and KiCad checks;
`exclusive_planes` holds only on some rungs. docs/agents.md has it in
the router table. Unit test (the fence's In1.Cu GND zone: the rule area
once, the lowering's track keepout).

**The fence** (`layout-kicad-board`, `PCB_WORK_RATE=3.15e6,150e6`,
build/fence; vias in pads by the report's pcbnew script, `HitTest` of
the via's centre, and by the ring against SMD pad copper):

| run | x1385 | x1403 |
|---|---|---|
| four layers, v5/layout.json (1500 s) | complete, 0 unconnected, 201 vias, 3887 mm, 14 vias in pads (LED4, 9, 10, 16, 17, 19, 20, 25, 26, 27, 29, 30, 36 pad 2, TP4.1), work 226 s | complete, 0 unconnected, 185 vias, 3715 mm, 0 in pads, work 217 s |
| two layers 34 mm, layout-long.json (3000 s) | 10 unconnected, starved_thermal 1, 281 vias, 2 in pads (LED15.4, LED41.1), work 1769 s, ladder stopped at its 1200 s | 6 unconnected, no error, 301 vias, 0 in pads, work 2430 s, every rung run (the finer lattices did not beat the regular one) |
| the same at the report's measured rate (1.1797e6) | the report's board exactly: 10 unconnected, 281 vias, 0.075 mm "not tried: projected 367 s exceeds 240 s", work 1523 s | 7 unconnected, starved_thermal 2, 319 vias, 0 in pads; the 0.075 mm lattice tried (16 open, the regular one's 15 kept), the ladder to the layout's end (work 2917 s) |

**Fingerprints** (Castor, k30, jetson, Sisu; route mode from v16, 2 x 4,
budget scale 0.25, `PCB_ROUTER_FINGERPRINT=1 PCB_ROUTER_NO_WALL_GUARD=1`;
build/fp-fence-x1385, -x1400, -x1402). Every board had vias in SMD pads
before (Castor 6, k30 1, jetson 22, Sisu 34; exposed pads 0, 0, 2, 3),
so every fingerprint changes from the first negotiation on; after, 0 in
SMD pads (Sisu's 3 in exposed pads stay). Castor: the probes chose the
connect rung over tracks, 187 to 241 vias, 4 starved thermals to none:
passes (x1400 had a hop's hole_to_hole there); k30: 335 to 344 vias, 4
starved to none: passes; jetson 19 to 20 unconnected, 620 to 597 vias;
Sisu 6 to 23 unconnected (its tracks probe 17 to 53 nets unfinished:
the cost of its 34 vias in pads).

**Short tier** (3 x 2 under the memory rule, `--no-lock`; layout mode,
work-based) x1403 against build/short-charge-x1381.log (6 x 2; layouts
as x1385): passes 8 to 6 (strict 7 to 5), unconnected 18 to 20, vias
1068 to 1088, work 2749 to 2961 s; vias in SMD pads 91 to 0 (exposed
pads 8 to 10). Changed rows: OpenFC 0 to 2 unconnected (32 vias in pads
before), WLED_Matrix (11 in pads: 147 to 168 vias, its errors moved),
MIDAS (8: 212 to 236 vias, 7 unconnected both), framework_mobo (2: a
starved thermal, strict pass lost), eternal-keypad (none in pads: the
own net's vias beside pads and the hop rule moved its routes; starved
thermals 2 to 4, above the designer's 3), smaller via counts on 0xCB
(5 in pads: 66 to 62), USB_Cable_Tester (6: 95 to 92), reCamera (6: 74
to 65), USB_Keypad (93 to 95); spell_tome, urchin, Blue_Line (15 in
pads before, now beside them: 26 vias both) complete as before.

**Expansion tier** (8 x 1 under the memory rule, `--no-lock`; route
mode, 30 s) x1403 against build/expansion-charge-x1381.log (16 x 1):
46 rows valid (no OOM kill), 2 identical; passes 16 to 18 (strict 16 to
18), KiCad unconnected 846 to 788, new errors 2 to 2, vias 4006 to
4141, work 1296 to 1250 s; vias in SMD pads 136 to 0 (exposed pads 15
to 19). Better 15, worse 9. Better where a pad's own via may now sit
beside it, with or without vias in pads before: Wonkle 427 to 361,
OpenRX-Mono 31 to 21, atmega32u4 9 to 0 and wanbeiyu 7 to 0 (pass),
node_v6 35 to 28, tuxkeychain 12 to 7, facade60 17 to 13, snaphat 3 to
0, magnetorquer 2 to 0, Julkort 1 to 0, ducktop2 1 to 0 (pass each),
backplane, ausign, OpenDeck, Zulassungskarte one fewer. Worse: the
boards with many fine pads that held vias (ZSWatch 0 to 4: 22 vias in
its module's pads; Nervous-ECG 2 to 4: 3; MSign 75 to 81, iorodeo 159
to 167: probed, 7 each), one each on anthracite, MusicBrain, Miniboard,
LC_PAMV4b (a starved thermal), FUEL4EP (a new starved thermal, 10 in
pads before); AMLing2's dronecarrier 15 to 52 (1 in pads: its probes
tied, 95 against 97 unfinished, and the plane-stub rung was continued
instead of tracks). How much harder fine pads route: the 11 boards that
had 5 vias in SMD pads or more went 279 to 283 unconnected (passes 3 to
5), the 15 with 1 to 4 468 to 437 (5 to 4), the 20 with none 99 to 68
(8 to 9): what the boards lose to the rule, the own net's vias beside
their pads give back; ZSWatch (22, a module's LGA pads) and Sisu (34,
fingerprints) are the real losses. The same tier with x1404 (rebased)
against x1394: 46 rows valid, passes 16 to 18 (strict 16 to 18),
unconnected 786 to 750, new errors 4 to 2 (strict 2 to 2), vias 4071 to
4161, better 13, worse 10.

**Not done.** Vias in BGA or LGA pads where no dogbone fits (ZSWatch's
module) need `via_in_pad: true`; the router does not decide that from
the designer's board. Same-net vias of different branches searched in
one batch are kept apart by `own_via_near` only against routed branches
(the hop rule covers one path). The router's own verification
(`verify`) does not check same-net hole spacing or vias in pads.
`check.sh`'s placer test `a_placement_does_not_depend_on_the_order_of_the_file`
fails on main as well (not this branch's).

## Committed on 2026-10-21 (probe overhead)

Charging all the work (the section below) exposed what the probe ladder
spent beside its search: on VortacDesign's panel the four rung probes
alone were 54 s of work on a 30 s budget (34.6 s of it the connected
probes' pour refresh), on MSign the probes' four lattices (2.7 s) moved
its continuation past a cliff, and the expansion tier went from 724
KiCad unconnected (x1371) to 846 (x1381). This removes most of that
work rather than uncharging it: everything stays charged
(`PCB_LADDER_UNCHARGED_POUR` remains the comparison switch only).
Branch probe-overhead; snapshots x1390 (polygon index, shared static
maps, step gate), x1391 (and the pour masks shared, the continuation's
fraction without lattices, the first attempt and the probes stopping on
all their work), x1392 (the probes back on search limits), x1393 (x1392
with a layout fix; its route mode is x1392's), x1394 (the commit: the
continuation's fraction by its own probe, the shared build at 12 ns a
node). Details: docs/router.md, "The probes' overhead".

**What changed** (each kind still charged at its counts):

| what | before | after | where |
|---|---|---|---|
| the fill probe's polygon tests | every corner of every polygon near a query (Vortac: board cutouts and a logo of 71 to 1589 corners, 29.5M corners an analysis of GND, 307 ms modelled) | polygons over 48 corners by their edges near the point (`geometry::PolygonIndex`, cells of at least 0.25 mm, rows for the crossing test; exact: `distance_below`, `distance_within`), 2.5M an analysis, 65 ms | every pour analysis: the negotiation's refresh, the probes' ranking, resolve, clean-up, via reduction, manual spokes |
| static maps (outline, obstacles' nodes, tight steps of every class) | built by every router | shared by routers of the same board, lattice, classes and rules (`shared_statics`, process-wide, by a 128-bit key); held by the ladder before routers start side by side (`hold_static_maps`) so that the count does not depend on who starts first | probes, rung waves, seeds; a shared build counts its own maps at `BUILD_SHARED_MAP_NANOS` (12 ns a node) |
| the connected pours' masks | computed by every router | computed with the held static maps, by the board's pours (`POUR_MASKS`) | the three connected probes |
| the continuation's share | turned into search work by all the routers' proportion of search, their four lattices included | by its own probe's proportion of search and pour work (it builds no lattice; a tracks leader did no pour work) | `built_router_work` |
| the first attempt's limits | its share of the bound in search, its pour work and lattice on top (musicbrain 42.6 s of work for a bound of 31.5) | its work deadline counts search, pour work and build, its negotiation search and pour work (`limits_count_all`) | not the probes: see below |
| a step's start | its first iteration fits what is left | and a fifth of the first attempt's work is left (`step_cost_fits`) | iorodeo, ducktop2, tuxkeychain no longer start a step with a second left |

Not kept: the rung probes stopping on all their work
(`PCB_LADDER_PROBES_COUNT_ALL`, x1391). With their pour work and
lattices out of their shares they stopped an iteration earlier and two
leaders changed: ducktop2 from tracks (1 open) to plane stubs (7 open),
brickpico from exclusive planes to tracks (35 to 30 vias); the ranking
compares equal search, and the probes' search limits stay.

**The named boards alone** (one thread, the reference's rates pinned,
x1385 then x1394, load 20 to 37; wall clock and the machine's share
follow the load):

| board | x1385: wall / work (search, pour, lattices), share | x1394 | probes' pour and lattice work (x1394, each) | continuation's deadline (fraction) | board |
|---|---|---|---|---|---|
| VortacDesign | 47.1 / 68.0 (16.6, 47.5, 4.0), 0.74 | 38.5 / 31.7 (19.2, 11.3, 1.2), 0.43 | connect 1.92, 1.92, 4.54 (stubs) and 0 (tracks); lattices 0.31 | 3.36 s (0.286) to 9.40 s (1.000) | 202 to 125 open (x1371's), 66 to 65 vias |
| FoCTow | 60.2 / 68.0 (16.6, 47.5, 4.0), 0.45 | 40.0 / 31.7 (19.2, 11.3, 1.2), 0.41 | as Vortac | as Vortac | as Vortac |
| MSign | 27.1 / 31.1 (26.9, 1.5, 2.7), 0.69 | 38.7 / 32.5 (29.6, 1.6, 1.3), 0.43 | 0.27, 0.27, 0.29, 0; lattices 0.34 | 11.64 s (0.854) to 14.05 s (1.000) | router count 351 to 234 open (x1371: 24) |
| iorodeo | 26.6 / 31.8 (30.1, 0.5, 1.3), 0.42 | 25.4 / 31.8 (30.8, 0.4, 0.4), 0.44 | 0.10, 0.10, 0.23, 0; lattices 0.11 | 15.98 s (0.922) to 17.58 s (1.000) | 335 to 273 open (x1371: 261); no step with 1 s |
| ChengminLi | 70.5 / 33.9 (14.2, 7.9, 11.7), 0.62 | 100.3 / 30.5 (20.0, 4.5, 6.1), 0.44 | not probed | | 4 open, 11 vias both (the 4 seeds of its second rung now fit) |
| brickpico | 58.3 / 28.2 (18.3, 9.4, 0.5), 0.37 | 65.8 / 30.7 (20.2, 9.9, 0.5), 0.36 | 1.58 and 0.23 (its first probe finishes) | 17.92 s (0.569) to 19.78 s (0.639) | complete, 35 vias both |

Vortac's per-analysis cost (x1385 against x1394, the pour trace): GND
inside 508874 nodes, 12.5 ms of node work both; the probe 295 to 56 ms
(29.5M to 2.9M corners and cells, 27k queries both). Its fingerprints in
a probe-less run at 4 s are identical before resolve and after every
phase (the answers are the same; only the count moved). Where MSign and
iorodeo still lose to x1371: the probes' search overshoots its share by
up to an iteration (MSign's probes searched 5.1 to 5.2 s on shares of
3.75 s, iorodeo's 3.8 to 5.0), which x1371 had too, and what is left of
their pour and lattice work (MSign 1.6 + 1.3 s) keeps the continuation
an iteration short (iorodeo's negotiation limit of 6.85 s stops 0.1 s
before the fifth iteration x1371 reached).

**Expansion tier** (16 x 1, `--no-lock`; every row checked for an OOM
kill: none). x1394 against x1381: 15 rows changed, unconnected 846 to
786, passes 16 to 16, new errors 2 to 4 (strict 1 to 2), work 1296 to
1263 s, the longest row 42.6 to 34.5 s; charged over the tier: lattices
41.1 to 21.3 s, pour 103.6 to 99.3 s, search 1150 to 1142 s. Against
x1371: unconnected 724 to 786, passes 16 to 16, new errors 2 to 4. The
goal (at or below 724, new errors at or below 2) is not reached. Work
over wall median 1.00 (10 % to 90 % 0.61-1.60) in x1381, 0.74
(0.46-0.97) in x1394; at the loads of the day the measure moves more
with the load than with the change (x1392 0.66, x1393's attribution run
1.18, the same binary).

The changed rows against x1381, by mechanism:
- The probes' lattices shared and the continuation's fraction by its own
  probe (a larger continuation): MSign 75 to 38, iorodeo 159 to 132,
  dronecarrier 15 to 14, ducktop2 1 to 0 (passes again), anthracite
  159 to 150 vias (back to x1371's 2 starved thermals), backplane,
  ausign, magnetorquer, ICE40 (vias and length).
- The rungs' shares larger (smaller lattices charged): Miniboard 3 to
  2 (x1371's), flexypin 39 to 51 vias (x1371's board: the 0.075 mm
  connect rung's share completes it again), ZSWatch.
- The step gate: iorodeo, ducktop2, tuxkeychain no longer start a step
  with a second left (tuxkeychain's board is the same).
- The first attempt's limits counting all its work (attribution run
  x1393 with `PCB_LADDER_SEARCH_LIMITS`, 781 unconnected, five rows
  apart): musicbrain 42.6 to 32.3 s of work (0 open both, 233 to 241
  vias), OpenDeck 12 to 11, Wonkle 427 to 435 (its one attempt searched
  34.0 s, now 31.3), epdInky and Miniboard (the same counts).
- New errors: RS485M's 2 starved thermals (as in x1381) and
  anthracite's 2 (as in x1371).

Where the tier stands against x1371 (62 unconnected more): MSign +21,
iorodeo +23, Wonkle +8 (the first attempt's limit), facade60 +6 (its
first attempt's pour and lattice charged leave its step 7.9 s, was 9.0),
dronecarrier +4, tuxkeychain +1, Zulassungskarte +1, RS485M -1, OpenDeck
-1.

**Short tier** (3 x 2, `--no-lock`, `--min-free-gb 14`) x1394 against
build/short-charge-x1381: all 12 rows identical and the 12 result
boards byte for byte (a layout's ladder counts search; the polygon index
answers as before; the shared maps are the same maps). CPU 5204 to
3988 s, at other loads.

**Fingerprints** (Castor, k30-SBC, jetson, Sisu; route mode from v16,
1 x 4, budget scale 0.25, `PCB_ROUTER_FINGERPRINT=1
PCB_ROUTER_NO_WALL_GUARD=1`) x1394 against x1385 (build/fp-probe-x1385,
run for this): every board's continuation got a larger share (its own
probe's fraction, 0.889-0.977 to 1.000; Castor's deadline 230.3 to
258.9 s). Castor and k30: every fingerprint and the board identical
(neither reaches its limits; work 159.6 to 158.4 and 297.0 to 296.1 s,
the lattices). Sisu: the continuation's fingerprints identical, its
board identical (13 open, 393 vias); its narrow-signal step changed with
its share (46.8 to 51.7 s: 23 to 26 open by the router's count, not
kept either time). jetson: identical before resolve, the clean-up on
changed with the share (negotiation 58.6 to 60.0 s, deadline 182.3 to
187.6 s): 10803.0 to 10791.4 mm (x1371's length), 620 vias and 19
unconnected both.

Unit tests: an_indexed_polygon_answers_as_the_plain_tests (geometry: the
index against the plain tests on a star of 300 corners and a long slot,
20 000 points, five thresholds), routers_of_one_board_and_lattice_share_their_static_maps
(the shared build's counts, the held work, the same maps and fingerprint),
a_build_is_priced_by_its_counts (the shared nodes), route_mode_steps_need_a_fifth_of_the_first_attempt,
route_mode_continuation_goes_by_its_own_probe, route_mode_probes_charge_their_pour_work_and_lattices
(the continuation's fraction), the step's cost in the work bound tests.
`bash scripts/check.sh` and the clippy gate pass. Docs: router.md ("The
probes' overhead", the work bound in route mode), config reference
(`share_static_maps`, `limits_count_all`, `PCB_ROUTER_NO_SHARED_STATICS`,
`PCB_LADDER_SEARCH_LIMITS`, `PCB_LADDER_PROBES_COUNT_ALL`,
`PCB_LADDER_CONTINUATION_LATTICE`, `PCB_LADDER_NO_STEP_COST_GATE`).

Logs: build/expansion-probe-x1390, -x1391, -x1392,
-x1393-searchlimits, -x1394; build/short-probe-x1394;
build/fp-probe-x1385, -x1394.

## Committed on 2026-10-21 (free dimension)

An agent that may choose one side of the board ("the height can be
anything; JLCPCB charges more past 100 mm") ran the layout at several
sizes by hand, and the results are not monotone (the fence report's
two-layer board at 34, 40 and 46 mm: 32, 1 and 2 KiCad unconnected).
Now the layout config says it: `"free_dimension": {"edge": "bottom",
"min": 34, "max": 60, "step": 2, "prefer_below": [100], "band_mm": 10}`
and `layout-kicad-board` searches the sizes itself
(docs/constraints.md, "A free dimension"; docs/agents.md). Branch
free-dimension; snapshots x1420 (the fence runs), x1421 (the commit:
x1420 formatted, doc comments; the short tier).

**What moves.** The edge of a rectangular outline (the constraints'
`outline` with width and height, or the board's own gr_rect or four
lines; polygons, pieces, cutouts, an automatic size, arcs on the
board's outline are refused, exit 2) and everything within `band_mm` of
it (default 0.5: what lies on it): footprints by their origin, board
texts and other rigid items by their anchor, lines, polygons, tracks and
zones point by point (a pour over the board grows with it), `region`
and `keepout` boxes by their side nearest the edge and `place`/`add`
points, re-expressed relative to the corner when that moves (`top`,
`left`). The rest keeps its board coordinates. A size where a fixed part
or a rule area that lay inside the outline would leave it, or a fixed
part carried by the edge would land on one that stays, is listed as
infeasible without a run. On the fence board `band_mm` 10 moves exactly
what the generator moves for its 40 and 46 mm variants (J1, the LED
row, its resistor networks and capacitors; nothing 12 mm from the edge
or farther).

**The search.** Each size is a layout of its own (this binary as a
child process, the parent's work rates pinned, threads shared among the
probes side by side), in `<out-dir>/free-dimension/<size>mm-<kind>/`.
From the smallest size upward, `parallel` sizes at a time at
`probe_seconds` (default a ninth of `total_seconds`, which is the whole
search's budget) while the work spent leaves `confirm_seconds` (default
a third) for the full run; the first batch with a clean size ends the
probes and the smallest clean size is laid out again at the full
budget; without a clean probe the closest size (unplaced, KiCad
unconnected, errors, missed constraints, size) gets it. The result is
the smallest clean run (the confirmation, else its clean probe), else
the closest; its layout is moved to `<out-dir>` with its paths
rewritten, `free-dimension.json` lists every run, the verdict gains
`free_dimension` {chosen, kind, clean, probes, reason, the search's
work and wall}, and when nothing is clean the first next action names
the closest size and what blocked it. Clean is the verdict `complete`
(`accept` `pass_strict`, default) or KiCad 0 unconnected and no new
error by the count rule with every part placed (`pass`).

**The fence boards** (breadboard-enhancer hw/build v5 and v5-2l-x0, 34
mm inputs, the report's layout.json with its 83 fixed parts and three
keepouts, router `auto`; `PCB_WORK_RATE=3.15e6,150e6`, load 40 to 80).

Two layers, 0.127 mm rules (v5-2l-x0), bottom edge 34 to 60 mm step 2,
prefer_below 100, band 10, total_seconds 6000, probes 500 s, full run
1500 s, 4 side by side on 16 threads:

| mm | run | budget s | KiCad unconnected | errors | work s | wall s | vias | length mm | chosen |
| --- | --- | --- | --- | --- | --- | --- | --- | --- | --- |
| 34 | probe | 500 | 19 | starved_thermal 1 | 265 | 488 | 450 | 4094 | |
| 36 | probe | 500 | 8 | starved_thermal 1 | 423 | 613 | 299 | 3780 | |
| 38 | probe | 500 | 3 | 0 | 428 | 634 | 319 | 4180 | |
| 40 | probe | 500 | 0 | starved_thermal 1 | 523 | 591 | 466 | 4509 | |
| 42 | probe | 500 | 0 | starved_thermal 1 | 675 | 549 | 306 | 4368 | |
| 44 | probe | 500 | 0 | 0 | 328 | 540 | 477 | 4856 | |
| 46 | probe | 500 | 0 | 0 | 229 | 363 | 461 | 5140 | |
| 48 | probe | 500 | 0 | 0 | 205 | 312 | 502 | 5208 | |
| 44 | confirm | 1500 | 0 | 0 | 329 | 523 | 477 | 4856 | yes |

44 mm chosen, exit 0, 9 runs, 3406 s of work, 1716 s wall; 50 to 60 mm
untried. The confirmation repeats the probe's board (the layout ended
complete well inside both budgets). 40 and 42 mm route every connection
and fail on one starved thermal each (the router's
`solid_starved_thermals` would take them; not tried: the search keeps
the given router config); 42 mm's probe took 675 s of work on a 500 s
budget (its final ladder 511 s after 164 s of placement and moves: the
layout's budget, not the search's).

Four layers (v5, routes clean at 34 mm), the same ladder with the
defaults (total_seconds 1500: probes 167 s, full run 500 s, 3 side by
side on 12 threads):

| mm | run | budget s | KiCad unconnected | errors | work s | wall s | vias | length mm | chosen |
| --- | --- | --- | --- | --- | --- | --- | --- | --- | --- |
| 34 | probe | 167 | 0 | 0 | 116 | 160 | 210 | 3849 | |
| 36 | probe | 167 | 0 | 0 | 106 | 170 | 192 | 3859 | |
| 38 | probe | 167 | 0 | 0 | 118 | 162 | 186 | 3924 | |
| 34 | confirm | 500 | 0 | 0 | 226 | 369 | 201 | 3887 | yes |

34 mm (its own size) chosen, exit 0, 566 s of work, 546 s wall.

**Unchanged without the key.** The layout config gains one optional
field (not serialised when unset); a single layout given one refuses it
(the command's search). Short tier (6 x 2, `--no-lock`) x1421 against
build/short-charge-x1381: all 12 rows identical and the 12 result boards byte for byte (8 passes, strict 7, unconnected 18, work 2749 s both). Fingerprints (Castor, k30,
jetson, Sisu, route mode from v16, 2 x 4, budget scale 0.25,
`PCB_ROUTER_FINGERPRINT=1 PCB_ROUTER_NO_WALL_GUARD=1`) x1421 against
x1385 (Sisu and k30 rerun one job at a time after the memory rule, build/fp-fd-x1385-part and -x1385b): every fingerprint and every routed board identical on all four, and identical to build/fp-charge-x1381 too.

**Left out.** Rounded or polygonal outlines on the board itself (give a
rectangle with `corner_radius` in constraints.json), cutouts, two free
edges at once; probes beyond a clean one in the same batch still run
(the fence: 46 and 48 mm, 434 s of work) and the confirmation runs even
when the probe ended well inside its budget (it repeated the probe's
board here); a smaller size at the full budget is not retried when a
larger one is clean at the probe budget (the results are not monotone:
40 mm might pass at 1500 s).

Unit tests (crates/pcb-kicad/src/free_dimension.rs): the ladder with
its thresholds, the bottom edge's outline, band, label, rule area and
pour, the default band, the top edge's corner and relative boxes, a
constraints outline resized, outlines refused, infeasible sizes (a fixed
part outside, a rule area, a carried part on a fixed one, a keepout
box), the smallest clean size over a non-monotone sequence, a failed
confirmation keeping its clean probe, the closest size given the full
budget, a price threshold exceeded only when nothing below is clean,
budgets and verdicts read. CLI test
(a_free_dimension_searches_the_sizes_and_keeps_one_layout): input
errors exit 2 in one line, a search without KiCad's check keeps the
closest size's layout as the result with its table. `bash
scripts/check.sh` and the clippy gate pass. Docs: constraints.md ("A
free dimension"), agents.md (the run, the cookbook, the verdict's
field), config reference (the `free_dimension` table).

Logs: build/dim1/v5-2l-free-x1420(.log), build/dim1/v5-free-x1420(.log),
build/short-fd-x1421, build/fp-fd-x1421, build/fp-fd-x1385-part, build/fp-fd-x1385b.

## Measured on 2026-10-21 (layout sweep v18, designer tier)

The deciding measurement for where the next work goes: the full layout
sweep and the designer tier on one binary, x1315 (main 35456a1; 6f10989
differs only in doc comments), tasks v7, the strict pass rule, the work
bound and the pinned work rates (3.15e6 and 150e6 on every row of both
runs). No code changed.

**Layout sweep v18** (`build/github-layout-v18.log`, three parts spliced:
41 boards at 6 x 2 with 20 GB kept free, stopped because memory admission
held it to two or three boards; the other 68 at 10 x 2 with 6 GB; the
four rows cut there again at 2 x 2): **42 of 109 pass the strict rule**,
46 the per-type rule (v17: 48); unconnected 2712 (2708), new errors 50
(38), vias 31044 (30601); medians against the designer's board vias
x0.71 (x0.70), length x1.03 (x1.03), open 0.009 (0.011) of the
connections, ground pour cut +0 mm (+0). Better 5, same 87, worse 7;
mackerel-30-proto and rosco lose their pass to starved thermals, MIDAS-MK1
places again. Cut at 10 jobs and again at 2: katia (1155 of 1500 s of
work at 3040 s of wall) and Batch_1 (1280 s at 3296 s), with the same
rows both times; 0xCB panel and Quanta75 Bare, cut at 10 jobs, finished
at 2 with the same rows.

**Designer tier** (`build/designer-x1315b.log`, through the lock at
8 x 2, its two timeouts rerun at 2 x 2): 50 strict passes, 58 per type;
katia and Batch_1 time out at the harness's 3300 s on both runs (katia's
ladder runs one 7.3 GB router at a time).

**The gap** (`compare.py --gap`, the 97 target boards with both
results): layout open 1483, designer route 430, so placement 71 % and
routing 29 %. By the strict pass: 13 boards pass on the designer's
placement and not on ours (video 199 open, OpenESC 4in1 111, 0xCB panel
65, and ten within six), 5 the other way (OpenFC, framework_mobo_lefthalf,
hackrf-one, keyboar_, Quanta75 RP2040Stamp), 44 on neither: 14 where the
designer route leaves as many open (routing), 24 where ours leaves more
(both), 4 complete on both but failing on findings, 2 without a designer
result. Eleven dense boards hold 82 % of the placement gap. Two router
modelling gaps: net-tie footprints are not modelled (all 24 of
neotron-pico's open connections and all 6 of ghoul's, on both
placements), and pads of one footprint with one number (jiran's USB
shield, 15 items in all). Four boards fail the strict rule on starved
thermals alone (mackerel-30-proto, rosco, coco2, k30-SBC).

Recommendation in [benchmarks.md](benchmarks.md#layout-sweep-v18-2026-10-21):
routing-aware placement, with a coarse global route as its fast
routability measure (RUDY and the probe were too weak or too slow), after
the cheap fixes (net ties, starved thermals). README.md's headline, the
hand-over and the tasks README carry the new numbers.
## Committed on 2026-10-21 (all pour work charged)

Route mode's ladder counted an attempt's search expansions and, since
the wall-guard work just below, its manual spokes; the negotiation's
pour refresh, the finishing phases' pour checks, the lattice builds
(with the stamping of the static maps) and the lowerings cost wall
clock and no work, so a budget of 30 s of work bought 30 s of search
and left 0 to 15 s of pour work a board (expansion tier; Vortac's
panels 47 s) and 1 to 40 thread seconds of lattices on top. Now every
kind is charged at a unit priced from what it counted, and the ladder's
shares are shares of all of it. Branch charge-pour-work; snapshots
x1380 (the measurement), x1381 (with the verdict's fields, the same
routing), x1382 and x1383 (attribution experiments, not committed),
x1384 (the commit: x1381 with the shares' log lines; the expansion
tier's 46 rows repeat x1381's).

**The pricing** (docs/router.md, "All the work charged"):

| charge | unit (reference nanoseconds) | calibration |
|---|---|---|
| search | expansions at 3.15e6 a second | unchanged |
| pour analyses and fill probes, every phase (the negotiation's pour refresh, the probes' ranking, resolve, clean-up, via reduction, thermal spokes, stitching, manual spokes) | 23 per node inside the pour, 6 per stamp, 9 per probe item, 850 per query, 9 per polygon vertex | unchanged (2026-10-15) |
| lattice builds (`Router::new`: every class's static maps, the nets' terminals and pour masks, the search records) | 22 per map node (cells x layers x classes), 11 per node tested, 2.75 per shape edge measured, 22 per edge a tight node's eight steps measure (`BuildCounts`) | 83 distinct builds, model 1.0 of the sum, 0.6 to 1.6 a board (10 % to 90 %; repeats of one build differ 1.34 times at the load) |
| lowerings | 26 per expression of the board file | 162 lowerings, median 19 and 26 ns in two runs; 0.5 to 22 ms each |

The counts are the router's own (`StaticMaps::tests`, `vertex_tests`,
`tight_tests`; `NetState::prepare_counts`; `Lowered::expressions`), so
a build or a lowering costs the same work on every machine and at every
load. A router stops as before on its search alone (its work deadline,
its negotiation's limit): given the same share it searches byte for byte
as before. The ladder charges each kind (`charged` in
`wall_accounting`; the verdict's `pour_work_seconds` and
`lattice_work_seconds`) and turns a share of what is left under its
bound into a router's limits in search work by the share of search in
the routers' work so far (`search_fraction`, at least a quarter; the
continuation, the rungs, the seeds, the steps; a `ladder: <attempt>:
work deadline ...` line says what each got). Its first attempt and its
rung probes have no proportion to go by and go beyond the bound by
their own pour work and lattices.

**The named boards alone** (one thread, the reference's rates pinned,
x1371 then x1380 back to back per board, load 12 to 30; "honest" is the
work over the routers' thread seconds times the machine's measured
share, which side-by-side routers and the load do not move as much as
work over wall):

| board | x1371: wall / work, work over wall, honest | x1380: wall / work (search, pour, lattices), work over wall, honest | board |
|---|---|---|---|
| VortacDesign | 39.2 / 31.8, 0.81, 0.51 | 35.5 / 68.0 (16.6, 47.5, 4.0), 1.92, 1.15 | 125 open, 65 vias to 202 open, 66 vias |
| FoCTow | 45.2 / 31.8, 0.70, 0.51 | 43.3 / 68.0 (16.6, 47.5, 4.0), 1.57, 1.11 | as Vortac |
| megaduck | 31.6 / 27.8, 0.88, 0.98 | 28.3 / 28.2 (27.5, 0.5, 0.3), 1.00, 1.08 | the same |
| ChengminLi | 61.8 / 29.2, 0.47, 0.33 | 61.6 / 33.9 (14.2, 7.9, 11.7), 0.55, 0.40 | 4 open, 11 vias both |
| brickpico | 53.9 / 32.4, 0.60, 0.55 | 34.0 / 28.2 (18.3, 9.4, 0.5), 0.83, 0.64 | complete both, 32 to 35 vias |
| Wonkle | 46.2 / 34.2, 0.74, 0.81 | 41.2 / 37.3 (34.0, 2.3, 1.0), 0.90, 1.07 | the same |

Where the wall clock went, x1371's `wall accounting:` (thread seconds):
Vortac 9 lattices 4.0, negotiation 47.6 for 18.3 s of search and 34.6
of pour work (the three connect probes' pour refresh), manual spokes
12.1; ChengminLi 15 lattices 23.6, clean-up 35.0 for 23.2 + 3.1;
brickpico via reduction 37.1 for 24.9 + 8.5. In x1380 Vortac's probes
alone are 54 s of work (15 of search, 34.6 of pour, 4.0 of lattices):
its continuation finds the bound reached, does no resolve and no
clean-up, and leaves 202 open; brickpico's continuation stops at its
share by the probes' proportion of search (via reduction 15.7 thread
seconds for 11.1 + 4.5); ChengminLi's 0.1 mm seed retry (four seeds of
the first attempt's charged work) no longer fits. ChengminLi is slow
all through, not only its lattices: three rungs side by side at load
16 to 30 ran its lattices at 2.7 times the model and its search at 2.7
times too (16 classes, memory bound); alone at one thread a build is
0.74 of the model (1.99 s for 1.48 at 0.1 mm).

**Expansion tier** (16 x 1, `--no-lock`, load 16 to 31) x1381 against
build/expansion-cuts-x1371: 27 rows the same board (work changed), 19
changed, none cut. Passes 16 to 16 (ducktop2's bms lost, anthracite
gained), KiCad unconnected 724 to 846, new errors 2 to 2, work 1272 to
1296 s; the longest row's work 34.2 to 42.6 s (five rows above 36 s:
first attempts whose pour work comes on top of a search deadline of
the whole bound, musicbrain 34 s of search and 8.4 of pour). Work over
wall on the uncut rows median 0.98 to 1.00, 10 % to 90 % 0.54-1.51 to
0.61-1.57; on the boards with the most uncharged pour and lattice work
before it rose (DIYSynth 0.39 to 0.62, brickpico 0.47 to 0.54,
musicbrain 0.49 to 0.61, koron 0.62 to 0.69, marchmare 0.54 to 0.58)
but the measure follows the load (x1384, the same rows at a higher
load: median 0.82) and routers side by side (above 1). Over the tier
the lattices' model is 0.87 of their thread seconds times the measured
share, the search and pour models' 1.28.

The loss is the lattices' charge, its unit right: attribution runs
(x1382, `PCB_LADDER_CHARGE_ONLY`, not committed) charging only the
lattices and lowerings gave 825 unconnected, only the pour work 750,
all of it without the shares' conversion (`PCB_LADDER_SEARCH_SHARES`)
832 (15 passes, 3 new errors); with `PCB_LADDER_UNCHARGED_POUR` x1381
repeats x1371's 46 rows. Most of it is two probed four-layer boards:
MSign 17 to 75 (lattices alone the same 75; its four probes' lattices,
2.7 s, shortened the continuation's negotiation past a cliff: router
count 24 to 351 open) and iorodeo's driver 109 to 159 (lattices alone
144, pour alone 120). Netting each probe's lattice out of its share
(x1383) did worse (870 unconnected, 14 passes) and is not kept. The
changed rows by mechanism:
- The probes' lattices and pour refresh charged, so the leader's
  continuation has less (its deadline and negotiation by the probes'
  share of search): AMLing2's dronecarrier 10 to 15, ducktop2 0 to 1
  (pass lost), tuxkeychain 11 to 12, iorodeo 109 to 159, MSign 17 to 75,
  backplane, magnetorquer, anthracite (149 to 159 vias, its 2 starved
  thermals gone: passes), ICE40 (209 to 217 vias), ZSWatch (length).
  Where that leaves a second, the narrow-signal
  step runs on it and loses (iorodeo, ducktop2, tuxkeychain).
- A first attempt charged more, so the rungs, seeds and steps after it
  have less: RS485M (the 0.05 mm seed retry no longer fits; the step
  then has more: 2 to 1 open, 2 starved thermals), pdlc (the step no
  longer fits; the 0.075 mm tracks rung's 30 vias kept instead of the
  step's 29), epdInky (the step's share 10.6 to 9.8 s of search: the
  same counts, another board), facade60 (the step's share 9.0 to 7.5 s:
  11 to 17), Miniboard (11.8 s kept for the step, so the rungs' shares
  are smaller and the step's board has 9 open, was 7: 2 to 3), ausign
  (the rungs after its probed continuation: 62 to 74 vias),
  Zulassungskarte (the 0.075 mm pitch projected
  past what is left: 1 to 2), flexypin (the 0.075 mm connect rung's
  share: 2 open, so the tracks rung runs and wins with 39 vias instead
  of 51), brickpico (its continuation's share: 32 to 35 vias).

**Short tier** (6 x 2, `--no-lock`) x1381 against build/short-cuts-x1371:
all 12 rows identical and the 12 result boards byte for byte (layouts
are unchanged: their ladder and tallies still count search and KiCad's
checks).

**Fingerprints** (Castor, k30-SBC, jetson, Sisu; route mode from v16,
2 x 4, budget scale 0.25, `PCB_ROUTER_FINGERPRINT=1
PCB_ROUTER_NO_WALL_GUARD=1`) x1381 against build/fp-cuts-x1371:
Castor's and k30's every fingerprint and board identical (one attempt
each; work 152.0 to 159.6 and 291.4 to 297.0 s, pour 6.9 and 6.5 s,
lattices 2.2 and 1.9 s charged). Sisu: the probes' and the
continuation's fingerprints identical (the same share: the bound), its
narrow-signal step's changed with its share (61.1 s kept before, 49.0
now: the probes' lattices, 8.2 s, and pour work; 15 to 23 open by the
router's count, not kept either time), the kept board identical. jetson:
the continuation's negotiation identical (before resolve), its clean-up
on changed with its share (its work deadline by the probes' share of
search), 10791.4 to 10803.0 mm, 19 unconnected both; the narrow-signal
step no longer runs (3.2 s left, its first iteration does not fit).

**Not changed, and what is left uncounted.** A layout's ladder and its
work (search expansions and KiCad's checks): the probes, the moves'
routers and the resumes there still leave their pour work, lattices and
lowerings uncounted. In route mode: the rung probes the order never
reaches (side by side with a probe that finishes everything: brickpico's
three), the stamping and conflict bookkeeping inside a phase (in the
search's calibration), the pour masks the manual spokes build per pad.

Unit tests: a_build_is_priced_by_its_counts, a_build_counts_its_maps_tests_and_pour_masks
(pcb-router), a_share_becomes_search_work_by_the_routers_search_share,
a_lowering_is_priced_by_the_board_files_expressions (work_bound_tests),
route_mode_charges_all_the_work,
route_mode_probes_charge_their_pour_work_and_lattices and the manual
spokes' test with the conversion (ladder_tests). `bash scripts/check.sh`
and the clippy gate pass. Docs: router.md ("All the work charged", the
work bound in route mode, where the wall clock went), agents.md (the
verdict's fields), config reference (`PCB_LADDER_UNCHARGED_POUR`,
`PCB_LADDER_SEARCH_SHARES`, `PCB_ROUTER_BUILD_TRACE`).

Logs: build/expansion-charge-x1381, -x1381-old, -x1381-searchshares,
-x1382-pour, -x1382-build, -x1383, -x1384; build/short-charge-x1381;
build/fp-charge-x1381.
## Committed on 2026-10-21 (strict rule: pre-bridged mask openings)

The remaining solder mask bridges of lowering gaps 2 (56 on 8 intake
boards; Sisu's 14 to 17 strict findings in the fingerprint, layout and
designer rows) are openings the designer drew along their own tracks over
a pour, or apertures over pads of several nets: once the tracks are
stripped the pour shows through, and any track of ours leaving a pad
inside bridges to it. The strict rule subtracted only the stripped
designer board's own pairs at the same place, never the opening and our
track. A harness change (branch strict-mask-bridges), no Rust.

**Rule 2** (run.py's `strict_findings`, `prebridged`; README.md "How to
measure", "The strict rule"): a solder_mask_bridge finding that is not
the designer's at the same place is still not counted when one item is a
mask opening (a graphic on F.Mask or B.Mask, or a pad's aperture, by
KiCad's item description) that bridges on the stripped designer board
(`baseline-drc.json`) or on the designer's own board with its tracks
(they shipped it bridged), matched by the opening's uuid or by position
within 0.05 mm, and the other item is a track or a zone. Our vias under
an opening, a pad of another part under an aperture, two items of ours
and an opening that bridges on neither designer board still count. The
designer's own board's bridges are kept by `designer_findings` (the DRC
it already ran) as `designer-mask-bridges.json` in the task's work
directory. Rows gain `strict_rule` (2; rows without it are rule 1) and
`strict_mask_openings` (the findings this rule left out, in neither
`new_findings_strict` nor `preexisting_strict`); summary.json gains
`strict_rule`. compare.py sums `strict_mask_openings` in the objectives
("N mask-opening bridges not counted") and prints STRICT RULES DIFFER
before the table when the two runs' rows were judged under different
rules. The binary's verdict (verdict.rs) keeps rule 1 (docs/agents.md
says so). Unit tests: test_run.py's StrictMaskOpenings on synthetic KiCad
report pairs (an opening bridged on the stripped board, one only on the
designer's own, position matching, an aperture over pads of several nets,
our via and another part's pad under a bridged aperture, two of our own
items, an opening bridged nowhere, no baseline, the row fields, the kept
designer bridges through a stand-in binary); test_compare.py's
StrictRules.

**Re-judged without rerouting** (a scratch script over the stored task
directories: the result's drc.json, the stored baseline-drc.json, the
designer's own board through drc-kicad-board's cache; rule 1 re-judged
first reproduced every stored row's `new_findings_strict`):

| run | rows | pass_strict rule 1 -> 2 | strict new errors | bridges not counted | boards changed |
| --- | --- | --- | --- | --- | --- |
| short tier, build/short-cuts-x1371 | 12 | 7 -> 7 | 205 -> 205 | 0 | none (no strict bridge) |
| expansion tier, build/expansion-cuts-x1371 | 46 | 16 -> 16 | 1 -> 1 | 0 | none (no strict bridge) |
| designer tier, build/designer-x1315 | 109 | 50 -> 50 | 1256 -> 1239 | 17 | Sisu |
| layout sweep, build/github-layout-v18b (41 rows of v18a) | 109 | 42 -> 42 | 1555 -> 1541 | 14 | Sisu |
| layout sweep, build/github-layout-v18c (105 rows of v18b) | 109 | 42 -> 42 | 1550 -> 1536 | 14 | Sisu |
| fingerprints, build/fp-cuts-x1371 | 4 | 0 -> 0 | 22 -> 8 | 14 | Sisu |

No strict verdict changes: Sisu's strict solder_mask_bridge findings
(17 in the designer tier, 14 in the layout sweep and the fingerprint row)
all go, but it stays failed on KiCad's unconnected items (18 and 15; the
designer row keeps one starved_thermal). Every one is J_LCD1's "Rectangle
of J_LCD1 on F.Mask" with one of our LCD tracks; the stripped designer
board has no bridge there, the designer's own board 22: the rectangle
with their LCD_DC track and with the GND zone. The bridges still counted
are ours or unknown: PolyKybd 5x4 (an NPTH pad against a kibuzzard logo
footprint), Quanta75 RP2040Stamp (our I2C1-SDA track under the
'0bsilab' F.Mask text, which bridges on neither designer board), vaio_re
in v18a (six pads of our C98, C84 and PD4 placed under J1's NPTH
aperture), protoesp and byrantech's motherboard (199, KiCad's cap on the
stripped board: unknown).

## Committed on 2026-10-21 (wall guard cuts at small budgets)

A cut row is not the binary's deterministic answer, so cuts must be rare
and explained. In route mode at 30 s, VortacDesign's and FoCTow's
toolchanger panels (intake boards) were cut in every run, idle or not;
ChengminLi's flyback (rules boards), megaduck, and OpenDeck, brickpico
and Wonkle (expansion tier) when the load was 45 to 70; the layout sweep
v18 (10 x 2, load 30 to 40) had four cut rows of 76. Branch
deadline-cuts; snapshots x1370 (on 642e28c), x1371 (rebased on 401ab2f,
with the check model's outline term); x1366 is the before-binary.

**Where the wall clock went.** Each board alone, one thread, the
reference's rates pinned, x1366 then x1371 back to back (load 15 to 25;
"share" is the machine's search rate measured at the start over the
pinned 3.15e6). x1366's split is from its `profile:` lines, x1371's from
the new `wall accounting:` line (thread seconds; routers side by side
add up).

| board | x1366: wall s / work s (share) | cut | where the wall went beyond the work | x1371: wall s / work s (share) | cut |
| --- | --- | --- | --- | --- | --- |
| VortacDesign | 92 / 19.1 (0.77); 166 / 20.4 (0.64) earlier | yes, every run | manual spokes 46 s in the continuation's finish, 58-69 s in each rung after it (uncharged); probes' pour refresh (34.6 s of pour work beside 18.3 of search, all probes) | 91 / 31.8 (0.38) | no |
| FoCTow (the same panel) | 118 / 19.1 (0.37) | yes | as Vortac: manual spokes 59 s in the verbose router alone | 77 / 31.8 (0.42) | no |
| megaduck | 93 / 46.2 (0.42); 104 / 45.2 (0.45) earlier, cut | at load 25+ | the narrow-signal step's one iteration: 17.6 s of work with 3.9 s left; the rest is the load (search at the machine's rate) | 57 / 27.8 (0.38) | no |
| ChengminLi | 74 / 28.7 (0.44); 95 / 25.0 (0.39) earlier, cut | at load 28+ | 15 lattice builds of 16 classes (40 thread s), stamping 14 maps, the load | 63 / 29.2 (0.60) | no |
| OpenDeck | 35 / 32.3 (0.79) | under load only | the load; resolve 23 s for 16 s of search | 47 / 33.6 (0.84) | no |
| brickpico | 67 / 32.1 (0.44) | under load only | the via reduction's pour checks (8.5 s of pour work), the load | 53 / 32.4 (0.75) | no |
| Wonkle | 41 / 34.0 (0.80) | under load only | its one attempt's resolve past its deadline (34 s of work for a bound of 32), the load | 51 / 34.2 (0.78) | no |

What was wrong, by cause:

- *The manual spokes* (Vortac, FoCTow). `add_manual_spokes` (straight
  tracks from starved pads into their pour, after every finish) runs a
  full pour analysis of GND per pad and one per candidate track: on the
  panels (642 x 647 nodes, four layers, 122 GND pads) 27 s of analyses
  in 28 s of wall clock for 34 tracks, in every attempt, beside 5 s of
  search. No budget counted it, no work deadline ended it, its fill
  probes were not even counted as pour work, and the rungs that ran on
  past the deadline did it too (x1366's log: 48 s in the continuation,
  58, 63 and 69 s in the three rungs side by side). The pour model was
  right (its modelled work 30.0 s against 27.2 s of wall): the work was
  real and uncharged.
- *A step past the bound* (megaduck). The ladder's first attempt took
  27.6 s; the narrow-signal step started with the 3.9 s kept for it,
  and a fresh router's first negotiation iteration, which no deadline
  stops, did 17.6 s of work (33 searches of half a second): 45 s of
  work against a bound of 32 s.
- *The pinned rate* (everything cut only under load). The tiers pin
  the reference's rate; the machine measures 0.4 to 0.6 of it at load
  25 and 0.3 to 0.4 at 60. A 90 s guard over a 30 s budget then holds
  only up to 0.36 of the reference, so every board that used its budget
  was cut whenever the tier ran beside other work. In the rows: the
  cut rows' `work_wall_ratio` was 0.27 to 0.45 where the measured share
  was 0.39 to 0.45 (megaduck 0.45 at 0.45, ChengminLi 0.28 at 0.39,
  brickpico 0.33 at 0.39). The guards did not follow the measured rate:
  `work_wall_ratio` was recorded, nothing acted on it.
- *Wall clock beyond the work, everywhere* (the rest of the ratio):
  the negotiation's pour refresh (counted as pour work, but neither in
  the negotiation's limit nor in the ladder's tally: Vortac's probes
  33 s of pour work beside 18 s of search, brickpico's via reduction
  8.5 s), the lattices (ChengminLi's 16 classes: 15 builds, 22 thread
  seconds), stamping. Measured, not charged here (below).
- *KiCad's check model* (layouts only; route mode with
  `skip_native_verification` runs no check). The constant 3.6 s fits
  ordinary boards (twelve measured at 2.8 to 5.4 s) but missed the
  complex outlines by four to nine times; the 0xCB panel's checks took
  275 s in v18b against 69 s charged.
- *The search itself* ran at the machine's measured rate on every board
  (Vortac's phases 2.99e6 a second at a measured share of 0.64, ChengminLi 0.78-1.09e6 at
  0.39, megaduck 1.2-1.7e6 at 0.45): no memory-bound lattice ran below
  the calibration; lowering is under 0.2 s on all of them.

The fixes:

1. **Manual spokes reuse their analyses** (exact): an analysis is a
   function of the routes, so after a kept track the next pass takes the
   trial's analysis, after a track not kept the one before, and the
   inner spokes start from the manual spokes' last. Vortac's phase 28 s
   to 13 s; on CompassTag, RS485M, Zulassungskarte, ch32v003, backplane
   and nervous-ecg (`PCB_ROUTER_NO_WALL_GUARD`, fingerprints on) every
   board and fingerprint is x1366's. The phase is a profile phase of its
   own (`manual spokes`), its fill probes count as pour work, it has a
   budget in work (the negotiation's, at least 60 s:
   `MANUAL_SPOKES_SECONDS`, a bound no ordinary board reaches; not the
   clean-up's cap, which a layout's polish shrinks) and stops at the
   wall-clock deadline, not at the work deadline.
2. **Route mode's ladder counts the manual spokes** beside the search
   (`result_work`; the router's `RoutingResult::manual_spokes_work`,
   board-router.json's `manual_spokes_work_seconds`). Vortac: the
   continuation's 6 s of search and 13 s of spokes reach the bound, no
   rung runs after it (they never won).
3. **No step without room for its first iteration**: route mode's
   narrow-signal and minimum-clearance steps start only if what is left
   under the bound covers the ladder's first attempt's work up to the
   end of its first negotiation iteration (`first_iteration_fits`,
   `RoutingResult::first_iteration_expansions`); else they are skipped
   with why. megaduck: 45.2 s of work to 27.9, the same board (the step
   lost anyway).
4. **The wall-clock guards stretch for pinned rates**
   (`KiCadWorkRate::wall_stretch`): by the pinned rate over the one
   measured at the start, at least 1, at most 4 (`MAX_WALL_STRETCH`,
   `PCB_WALL_STRETCH_MAX`), the search's in route mode, the larger of
   the search's and the placer's in a layout: route mode's
   `deadline_seconds`, the layout's `wall_guard`, the router's `GUARD`.
   With the rates measured nothing changes. The verdict and the rows
   have `wall_stretch`; run.py's timeouts allow for the cap when it
   pins the rates (`WALL_STRETCH_MAX`: 420 s for the intake's 30 s
   budget, 12 300 s for a layout of 1500 s; hang guards).
5. **KiCad's check model: complex outlines.** 20 s more per check
   (`COMPLEX_OUTLINE_SECONDS`) for a board with more than 200 graphic
   items on Edge.Cuts (`COMPLEX_OUTLINE_ITEMS`), measured on 20 of
   v18b's routed boards checked again uncached one at a time (load 22):
   katia (404 items) 15.5 s, ULK (411) 33.1, ghoul (496) 28.8,
   slab-pcb-left (1030) 15.1, interchange (1162) 33.8, SmartSpin2k's
   panel (1367) 28.8, the 0xCB panel (6369, and 8100 hatch holes) 41.7
   against its 17.4: model over measured 0.64 to 1.43 (was 0.11 to
   0.42); the twelve boards of at most 100 items (2.8 to 5.4 s) and
   every short-tier board (at most 140) unchanged. Linear terms in the
   items or in polygon vertices fit worse (beyond about 400 items the
   time stops growing; the vertex count also marks fast boards: urchin,
   0xcb-static, Blue_Line's 359 k). Not explained: d20_pcb (194 k
   vertices, 17.4 s), Quanta75 (14.4 s), PixelWave (12.1 s).
6. **Where the wall clock went, in every run**: the `wall accounting:`
   line and board-router.json's `wall_accounting` (the ladder's wall and
   work, the machine's share, the stretch, thread seconds of lattices,
   lowerings, preparation, each router phase with its search and pour
   work, kicad-cli's runs); the verbose `manual spokes:` line gives its
   wall clock and pour work.

**Measured.** The named boards: the table above; none cut by x1371.
x1366's wall over x1371's is not the comparison (the load moved
between runs); the ratio of work to wall clock against the machine's
share is: Vortac 0.21 at a share of 0.77 before (the uncounted spokes),
0.36 at 0.38 after (the work is the wall clock's at the machine's
rate); megaduck 0.51 to 0.52 at the same share, 46.2 to 27.8 s of work.
megaduck's row also changed with main's lowering gaps 2 (x1367: 72
unconnected, 16 vias, the same as x1371).

**Expansion tier** (16 x 1, `--no-lock`; x1365's at 8 x 1, load 37 to
69; x1370 and x1371 at load 14 to 27): x1370 and x1371 give the same 46
rows. Against x1365, with `PCB_LADDER_UNCHARGED_SPOKES=1
PCB_LADDER_NO_FIRST_ITERATION_GATE=1` (build/expansion-cuts-x1370-old):
45 rows identical, brickpico (cut in x1365) uncut at 32.1 s of work; so
the manual spokes' reuse, their budget, the stretch and the accounting
change no row. With the defaults: routed, vias, length, KiCad's findings
and verdict identical on 44 rows, `work_seconds` +0.1 to +1.3 s on 30
(their manual spokes, now counted); no row cut (x1365: brickpico). Where
the count changed where the ladder stops:
- FUEL4EP: the skeleton rung (111 open, 2.3 s) no longer starts (the
  bound is reached with the spokes counted); the same board, 30.8 to
  29.9 s of work.
- Julkort: the 0.075 mm rungs' shares are smaller by the regular
  pitch's spokes; the kept connect attempt negotiated 6 iterations, not
  4: 23/24 both, vias 20 to 23, length 257.2 to 255.1 mm, KiCad the
  same.
- RS485M: the 0.05 mm rungs' shares are smaller; their seeds and the
  tracks rung end at 3 open (were 2), the narrow-signal step varies
  another best attempt and loses (3 open; was 1): 19/20 to 18/20,
  KiCad 1 unconnected and 2 starved thermals to 2 unconnected and no
  error.
The tier's work 1264 to 1272 s, KiCad unconnected 723 to 724, new
errors 4 to 2, passes 16 to 16. Wall 2077 to 1519 s summed over the
rows, the longest row 111 to 72 s (the load differed).

**Short tier** (6 x 2, `--no-lock`) x1370 and x1371 against
build/short-x1365: all 12 rows identical (routed, vias, length, work,
KiCad's findings, verdict), none cut; no short-tier board has more than
140 Edge.Cuts items (1337-v4.0), so the check model is unchanged there.

**Fingerprints** (Castor, k30-SBC, jetson, Sisu; route mode from v16,
2 x 4, budget scale 0.25, `PCB_ROUTER_FINGERPRINT=1
PCB_ROUTER_NO_WALL_GUARD=1`) x1370 against build/fp-lower2-x1366 and
x1371 against -x1367 (main's, jetson's rings): every fingerprint and
kept board identical;
`work_seconds` +0.2 to +2.8 s (the spokes counted; k30 2.8, Castor
1.6), and their wall clock fell (k30 270 to 181 s, Castor 187 to 115 s
at a lower load: the reuse).

**The layout sweep's cut rows** (v18b, 10 x 2, load 25 to 69): katia
(work over wall 0.35; measured 0.41 of the reference's search rate and
0.20 of the placer's), Batch_1 (0.32; 0.41 and 0.24), Quanta75 (0.39;
0.74 and 0.34), the 0xCB panel (0.42; 0.65 and 0.40). Their placement
ran at the placer's share (katia: 548 s of wall for 123 s of work) and
their routing near the search's: the pinned rate, not a phase. The
stretch (the larger share's inverse, at most 4: 4.0, 4.0, 3.0, 2.5)
puts their guards at 12 000, 12 000, 8 800 and 7 500 s, past their
3000-3200 s. The 0xCB panel's checks also took 275 s for 69 s charged;
with the outline term (6369 Edge.Cuts items) each charges 37.4 s. Not run again
here (a full layout each).

**What is still not counted** (measured, left for a decision): the
negotiation's pour refresh and the finishing phases' pour checks in the
ladder's tally (Vortac's probes 34.6 s of pour work beside 18.3 s of
search; brickpico 12.8 s beside 31; on the expansion tier 0 to 12.8 s a
board, median about 1.5): counting them is honest but moves most pour
boards' rows (every rung share after the first); the lattice builds
(ChengminLi 40 thread seconds for 15 builds); the stamping (the
negotiation's limit is search expansions). With the stretch these cost
wall clock, not cuts.

**The guard, recommended.** Keep `wall_guard` 2 and route mode's 3 x
budget as margins over the work at the machine's measured rate, and let
them stretch with a pinned rate (done here, cap 4): with the rates
measured a second of work is a second of this machine, and the guard is
only the margin for what the work does not count; with the rates pinned
the guard was a load-dependent budget, which is what cut the loaded rows
(their `work_wall_ratio` sat at the machine's share). run.py's
timeouts scale with the cap, not with a measurement of their own (the
binary measures; the harness only needs a hang guard above the
binary's guard). A tier on a loaded machine now takes longer instead of
cutting: up to four times a quiet run's wall clock.

Unit tests: ladder_tests (route mode counts the manual spokes; a step
needs room for a first iteration), work_bound_tests
(`first_iteration_fits`), work_rate (`wall_stretch`), check_work (the
outline term, footprint graphics on Edge.Cuts), test_run.py (the
stretched timeouts). `bash scripts/check.sh` and the clippy gate pass.
Docs: router.md (the work bound in route mode, the manual spokes, where
the wall clock went, the guards' stretch, KiCad's checks as work),
agent-tasks/README.md, config-reference (three variables).

Logs: build/expansion-cuts-x1370, -x1370-old, -x1371;
build/short-cuts-x1370, -x1371; build/fp-cuts-x1370, -x1370-old,
-x1371.

## Committed on 2026-10-21 (lowering gaps 2)

The finding types the previous lowering round left ("Lowering gaps in the
intake's findings" below): solder mask bridges, dimensions and texts on
copper, footprint graphics of a net and SeanBP's numbered nets. Each
checked with kicad-cli 10.0.7 on a board made for it; the changes are in
the lowering (`board_router/lower.rs`, `geometry.rs`) and `outline.rs`.
docs/router.md, "Dimensions, text boxes, cached copper texts and
padstacks" and "Solder mask bridges".

Binaries: x1345 is main before (3223282, the connectivity agreement);
x1350 and x1351 steps (x1351 without plated holes of no listed layer),
x1352 the commit's code.

**The measurement.** From the intake's KiCad reports (the 2026-10-18
quick run, an older binary): the boards whose result has more findings of
a type than the designer's board, the router-output types left out
(track_dangling, track_not_centered_on_via, unconnected_items,
starved_thermal): 184 boards. The intake's own count over them (its
binary, before the project rules and the first lowering round):
track_width 7120 on 89 boards, clearance 1362 on 41, hole_to_hole 123 on
8, copper_edge_clearance 100 on 17, solder_mask_bridge 78 on 15,
drill_out_of_range 51 on 1, shorting_items 32 on 13, length_out_of_range
30 on 4, hole_clearance 29 on 9, annular_width 22 on 1,
malformed_courtyard 13 on 4, creepage 3 on 3, skew_out_of_range 1. With them the intake boards that have a
dimension on copper (40), a padstack (16) or a cached text on copper (22):
252 boards, routed again from the intake's stripped boards and settings
(route mode, 30 s of work, one thread, `PCB_WORK_RATE=3.15e6,150e6`),
KiCad's DRC with zones refilled, against the intake's DRC of the
designer's board (scratchpad, not a tier). Excess findings by type (sum
over boards of ours less the designer's, where more), x1345 against
x1352:

| type | x1345 findings / boards | x1352 findings / boards |
| --- | --- | --- |
| solder_mask_bridge | 89 / 18 | 56 / 8 |
| shorting_items | 36 / 11 | 0 / 0 |
| clearance | 32 / 10 | 2 / 2 |
| length_out_of_range | 30 / 4 | 30 / 4 |
| track_width | 24 / 1 | 24 / 1 |
| hole_to_hole | 7 / 1 | 7 / 1 |
| hole_clearance | 6 / 4 | 3 / 1 |
| creepage | 3 / 3 | 3 / 3 |
| skew_out_of_range | 1 / 1 | 1 / 1 |

KiCad's unconnected items over the 252: 980 to 973. 223 rows identical,
29 changed, each with an item of these gaps but VortacDesign's panel (cut
by the wall clock's deadline in both runs, as FoCTow's) and megaduck
(cut at x1352 only: the machine's load was 60 to 70 during that run): padstacks (dennisss's seven
camera boards: excess 53 to 0, unconnected 133 to 125; adrianmarple 10 to
0), dimensions on copper (shrimp-18 3 to 0, robotique-udes 2 to 0,
liquid-rocketry 18 to 0 with a cached text, and eight boards whose
dimensions moved routes only: megaduck, PCBench BalthazarPSU3, Soraxem,
kapibaratomo, koron, sbeddes, ub1qbm), cached copper texts (TomKeddie 3 to
0, FoCTow 1 to 0, Pips801, eutectic6337, mothenjoyer69 routes only), the
bare-side aperture (c-lydia 2 to 0), a whole-circle arc (cnlohr 3 to
0) and plated holes listing no copper layer (aaronds's two ESP32 DevKitC
boards, clearance 6 and 2 to 0; shoonie's two c10 keyboards,
solder_mask_bridge 3 fewer each). Four rows have more unconnected items (dennisss ar0234_r1 2 to 5,
mira220 29 to 30, megaduck 64 to 65, PCBench 163 to 167): more copper to
route around in 30 s. FoCTow's route needs 150 to 180 s of wall clock
under load (the scratch harness's timeout was raised from 200 to 400 s
for it).

**What changed.**

- **Dimensions on copper** are copper of no net (`dimension_shapes`):
  KiCad 10.0.7 measures from the lines' centres (zero width: tracks at
  0.19, 0.15, 0.10, 0.064 and 0.05 mm from the crossbar, an extension
  line's start, an orthogonal second extension and an arrow, exactly as
  computed) and from the text's glyphs. Aligned and orthogonal geometry
  as PCB_DIM_ALIGNED and PCB_DIM_ORTHOGONAL build it; KiCad 5's stored
  lines read as they are; leader, centre and radial a line between their
  points.
- **Text boxes** (`gr_text_box`, `fp_text_box`) on copper: the box filled,
  grown by half its stroke (KiCad measures from that outline, border or
  not); on a mask layer an opening. None in the intake is on copper or
  mask; checked on a made board.
- **Cached (TrueType) copper texts**: KiCad draws them from the installed
  font (TomKeddie's sign: plotted glyphs up to 0.92 mm outside the cache);
  each cached line is also a box grown by a tenth of the text height
  (`cached_text_line_boxes`), the gaps between lines open.
- **Padstacks** (KiCad 9's front_inner_back and custom): the inner and
  back shapes are copper of the pad's net on their layers
  (`padstack_variants`); the back's mask aperture follows the back shape.
- **A pad's mask opening on a side without its copper** (c-lydia's U3:
  `(layers "F.Mask" "B.Cu" "F.Paste")`) is an aperture of its net there.
- **A plated hole that lists no copper layer** (`(layers "*.Mask")`) is
  copper on every layer: kicad-cli shorted tracks on F.Cu, an inner layer
  and B.Cu to such a pad of no net; a pad listing some copper layers has
  copper only there. It was a mask-only opening (aaronds's DevKitC pins:
  tracks 0.005 mm from them).
- **An arc ending where it starts** is a whole circle (`arc_points`; MK1's
  ring pad in cnlohr's tadpoleswadge, a custom pad primitive; jetson's
  two In4.Cu rings).

**Decided, not changed.**

- **Footprint graphics of a net** (item 4 of the list): KiCad 10.0.7
  treats them as copper of no net. kicad-cli on an antenna footprint whose
  polygon is on net A, KiCad 9 `(net 1)` and KiCad 10 `(net "A")` alike:
  clearance findings at 0 mm against its own pad and an A track, the pad
  unconnected although the track touches the polygon. The lowering
  already kept them as copper of no net at Default's clearance; a comment
  says why. The intake's four boards with such graphics (the TI antennas
  of two student boards, keytartouch's 24 touch pads, a minibadge
  holder) have these findings on the designer's board too.
- **SeanBP's numbered nets** (item 5): the connectivity agreement (main,
  x1345) made digit names routable and reads `(net N)` as a code; the same
  fix was here and was dropped in the rebase. SeanBP's Edge-Card-CAEN
  ("the board has no routable connections" in the intake): x1345 routes
  69/70, KiCad 1 unconnected, no error. Its 70 nets each join a test
  point on F.Cu and one on B.Cu at one place, both "REF**.1": the
  pads-of-one-number merge took them for one terminal until x1345's
  shared-layer condition.
- **Solder mask bridges over pours** (item 1): openings the designer drew
  along their own tracks (AntonSievering's F.Mask strips along RF lines
  over the GND pour, bostang's strips from U1's pins, eigenhari's B.Mask
  segments) show the pour once the tracks are stripped, and a pad inside
  bridges whatever is routed there. Treating such an opening as no net's
  where it reaches farther than the pour's clearance from its pads (tried
  as x1350's predecessor on 19 boards) took AntonSievering 7 to 11 and
  eigenhari's PQ_PL2 6 to 9 (56/59 routed, was 59/59) and fixed two
  single findings: not kept. These, and the openings over pads of
  several nets (Sisu's J_LCD1, ringshout's J2: a track leaving a pad
  inside bridges), are the designer's board: the strict rule subtracts
  the stripped designer board's own pairs (opening and pad, opening and
  zone) but not the opening and our track. A strict rule that subtracts
  every bridge of an opening that already bridges in the baseline would
  remove them (Sisu's 15 strict solder_mask_bridge in the fingerprint
  rows are of this kind); a harness change, not made.

**What remains** of the 252 at x1352: solder_mask_bridge 56 on 8 boards
(the openings above: bostang 33, AntonSievering 7, eigenhari's PQ_PL2 7
and ringshout 1; shoonie's c8 5 and c8v3 1, B.Mask graphics and NPTH
pads beside the switch pads; McbeEringi's two boards 1
each, a B.Mask text and an F.Mask polygon over the pour),
length_out_of_range and skew_out_of_range (length-tuning rules, 5
boards), track_width 24 on one board, hole_to_hole 7 on one, creepage 3
on three (pad pairs the router cannot change), clearance 2 (KiCad's QA
board issue11814 in two copies: 0.1875 against its PadToTrack rule),
hole_clearance 3 on jrsteensen's OpenHornet (a custom rule's 2.65 mm
from an NPTH hole "for GND copper"). Lathe-Mariel's In4.Cu short against
a PTH pad (intake) is not reproduced at x1345.

**Rules boards** (rules-tasks.json, 8 x 1, `--no-lock`), x1345 against
x1351 and x1352: all 20 rows identical (passes 18, strict 18, unconnected
4, vias 494). ChengminLi's flyback is CUT by the deadline in all three
runs (the machine's load at 45 to 60 while the sweep and these runs
shared it; 35/38 each time).

**Expansion tier** (8 x 1 for the reference, 10 x 1 and 8 x 1 here,
`--no-lock`), build/expansion-x1345 against x1351: all 46 rows identical,
no row cut; against x1352: 43 identical, three cut at a load of 60 to 70
(OpenDeck, brickpico, Wonkle) and run again alone: identical.

**Short tier** (4 x 2 for the reference, 5 x 2 and 6 x 2 here,
`--no-lock`), build/short-x1345 against x1351 and x1352: all 12 rows
identical, no row cut.

**Fingerprints** (Castor, k30-SBC, jetson, Sisu; route mode, 2 x 4,
budget scale 0.25, `PCB_ROUTER_FINGERPRINT=1 PCB_ROUTER_NO_WALL_GUARD=1`),
build/fp-conn-x1345 against build/fp-lower2-x1351 and -x1352 (the same
fingerprints): Castor, k30 and Sisu every fingerprint and board
identical; jetson's kept board (md5) and row identical, the fingerprints
of one attempt's clean-up changed: its two whole-circle arcs on In4.Cu
(radius 0.1 mm, `(start)` equal to `(end)`) are rings now, not points.
Castor's two dimensions are on Dwgs.User; none of the four has a
padstack, a cached copper text or a plated hole without copper layers.

Unit tests: board_router lowering_gap_tests (dimensions and text boxes
against kicad-cli's distances, a padstack's back copper and a bare-side
aperture, cached copper text lines and a footprint graphic of a net),
outline (a whole-circle arc), the plated hole without copper layers. `bash scripts/check.sh` and the clippy gate
pass.

**Rebased on 642e28c** (the pour model, x1366) as x1367: rules boards
x1366 against x1367 identical on all 20 rows (ChengminLi's flyback the
same row, cut by the deadline at x1367 only); against x1352 nuclone (48/48
to 18/18 connections) and issue25101 (length) moved with the pour
package, as x1366 shows. Short tier build/short-x1365 against x1367: all
12 rows identical, none cut. Fingerprints x1366 against x1367: Castor,
k30 and Sisu identical; jetson's board identical, the one clean-up's
fingerprints changed by its rings as above.

Logs: build/rules-lower2-x1345, -x1351, -x1352, -x1366, -x1367;
build/short-lower2-x1367; build/fp-lower2-x1366, -x1367;
build/expansion-lower2-x1351, -x1352, -x1352-recut; build/short-lower2-x1351,
-x1352; build/fp-lower2-x1351, -x1352.

## Committed on 2026-10-21 (pour model after connectivity)

The connectivity agreement (below) made the router's "complete" what
KiCad finds connected on 63 of the 65 boards, but cost rows: MSign (15
to 60 unconnected), stellarcorp's magnetorquer (a pass to 84/86),
LC_PAMV4b (a starved thermal), urchin (42 to 65 vias) and
eternal-keypad in the short tier, and left issue6284 (a spoke joining
U102.4 and U102.6 that KiCad keeps apart), the older starved thermals
of ZSWatch and anthracite and KiCad 9's footprint jumpers. Branch
pour-regressions; snapshots x1360 (the transitive terminals), x1361
(with the spoke end test and jumpers), x1362 (with the vias' reliefs),
x1363 and x1365 (the commit, x1362 without its switches; every row of
the 65, the expansion and the short tier and the fingerprints repeat
x1362's); x1364 an experiment not kept (below). Attribution with a
binary whose changes could be switched off one at a time (x1361, x1362:
`PCB_X_OLD_END`, `PCB_X_NO_VIA_RELIEFS`, `PCB_X_NO_JUMPERS`, not
committed) and the per-net terminal counts of board-router.json.

**What was wrong, and the fixes.**

- *Pads of one number joined through a third were two terminals.* The
  connectivity commit merged pads of one number only into the largest
  that overlaps them on a shared layer. An exposed pad on F.Cu and its
  copy on B.Cu share no layer, but both overlap the thermal vias between
  them: one item to KiCad (kicad-cli on a board with a track from the
  F.Cu pad: no unconnected item; without the vias the B.Cu pad is
  unconnected). The lowering made the B.Cu pad a terminal of its own and
  routed it: MSign's U1.21, LC_PAMV4b's U3.25, Sisu's U2.11, ZSWatch's
  IC401.33, IotThermalCamera's TU2.9; urchin's and eternal-keypad's
  reversible diodes (D_SMD_TH: SMD pads on both sides and a through-hole
  pad that both overlap), mounting holes with copper on
  both sides (magnetorquer's MH1 to MH4, VTC-WISDOM's H2). Pads of one
  number are now grouped by overlap (union-find over the pairs that
  share a layer and hold each other's anchor), the largest of a group
  its terminal; stacked pads with nothing between them (Knox Key's
  P1.A1, VTC-WISDOM's edge-mount SMA J3) stay two terminals, which
  KiCad wants joined by a via. The urchin and MSign costs were these
  terminals, not a choice of side: no pad pair without a hole needs the
  router to pick one.
- *Footprint jumpers.* KiCad 9's `(duplicate_pad_numbers_are_jumpers
  yes)` joins the pads of one number of a footprint,
  `(jumper_pad_groups ("1" "2") ...)` the pads of a group's numbers,
  without copper (kicad-cli 10.0.7: a track from one pad 1 leaves the
  other pad 1 and pad 2 unconnected without either, pad 2 only with the
  first, nothing with the groups). Such pads of one net are one terminal
  now (37 boards of the intake, mostly KiCad QA copies; ducktop2's bms:
  its WAGO terminals, PACK_NEG_RAW and three more nets one terminal
  fewer; Sisu's I-PEX J2).
- *A spoke whose end lies in another item's clearance.* KiCad keeps a
  spoke where its end point lies in the fill (zone_filler.cpp tests
  `spoke.CPoint(3)` against the fill less the clearance holes, opened by
  half the minimum width); the model asked for a brush centre within
  half the minimum width of the end that keeps the clearances less
  `EPSILON` (3 microns). anthracite's R503.2 and R703.2: diagonal spokes
  (`thermal_bridge_angle 45`) whose ends lie 0.3981 mm from R503.1's
  rounded corner under a 0.4 mm clearance; a brush centre beside the end
  fit by 0.3 microns of the tolerance, KiCad drops the spoke and finds
  the pad starved. The end itself must now keep every clearance
  (`spoke_end_lands`, `END_TOLERANCE` 0; pcbnew 10.0.7 on a test board:
  an end 0.202 mm from another net's pad under a 0.2 mm clearance keeps
  its spoke, 0.198 mm loses it).
- *Vias of the net in a hatched zone.* KiCad 10 gives every via of the
  zone's net a thermal relief in a hatched zone whose pads connect by
  thermals (knockoutThermalReliefs: the via's radius and the thermal gap
  knocked out, a ring a spoke wide, four spokes; with `connect_pads yes`
  or `thru_hole_only` the via is solid). The model filled up to the via.
  issue6284's U102.6: its +x spoke ends 0.67 mm from a GND via (relief
  0.8 mm); KiCad drops the spoke, the pad's other piece holds nothing
  else, and KiCad finds two zone pieces unconnected where the model
  joined the pad to the via's piece and called the board complete (the
  "spoke join" of the connectivity commit). `Hatch::via_reliefs` (the
  lowering: a zone's `connect_pads` with no keyword), the fill probe
  keeps brush centres out of the reliefs of the net's vias, routed or
  already on the board (`FillProbe::relieved_vias`, `Item::ViaRelief`),
  their rings count as fill (`in_ring`), and the pour map clears the
  relief's nodes but for the via's four spokes along the axes
  (`cut_via_reliefs`): IotThermalCamera's Tgnd, routed again after the
  transitive terminals, joined two pieces through a diagonal neck of
  the lattice past a via's relief and was complete by our count with
  KiCad 1 unconnected. pcbnew on a test board: a pad's spoke end in a
  GND via's relief loses the spoke, with `connect_pads yes` it keeps it.

Not kept (x1364): the starved-thermal model by each pad's own zone
connection (`solid_pad_obstacles`, `isolated_pad_obstacles`) rather than
its label's. It gave eternal-keypad's ProMicro1.4 and .23 and 74th's
J3.S1 model rows (by label their SMD pads set to no connection made the
through-hole pads isolated), but those rows are still missed (the model
counts two spokes where KiCad counts one), and ray4477's U1.33 thermal
vias inside a solid exposed pad went from predicted to missed (by label
the B.Cu exposed pad was thermal, which happened to give the vias
KiCad's one spoke). Census below: predicted 165 to 163, false alarms 29
to 31, no model row 4 to 0.

Unit tests, one per cause, with the kicad-cli or pcbnew checks in their
comments: pcb-kicad's
`pads_of_one_number_joined_through_a_third_are_one_terminal`,
`pads_of_a_jumper_group_are_one_terminal`; pcb-router's
`a_spoke_end_inside_a_clearance_is_dropped`,
`a_via_of_the_net_has_a_thermal_relief_in_a_hatched_zone` (the spoke
and the pour map's nodes).

**Starved-thermal census** (`experiments/pour-fidelity/census.py`, the
model on the final copper against KiCad's refilled DRC; the 32 boards of
the 44-board census still on disk and the 42 of the intake's
route-t8, 218 starved pad rows): predicted 162 to 165, missed 52 to 49,
false alarms 29 both, no model row 4 both (x1360's model, which has the
transitive terminals only, against x1363). The three: ptSolar's PV2.2 on
three layers (spoke ends inside a clearance). The hatched boards' rows
(0xCB, Qfwfq) change pieces, not verdicts.

**Route mode on the 65** (connectivity-tasks.json, 30 s, 8 x 1,
`--no-lock`; `census.py --run`), x1345 against x1365: 57 rows
identical; verdict agrees with KiCad 63 to 64; complete by our count
with KiCad unconnected 2 to 1 (robotique's BioGenius, the dimension on
copper of the lowering gaps 2 package); KiCad's unconnected items 12 to
11; passes 56 to 57 (strict 53 to 54); vias 1811 to 1900. The changed
rows:

| board | x1345 | x1365 | cause |
| --- | --- | --- | --- |
| baxter-barlow issue6284 | 127/127, KiCad 2 (complete by our count) | 126/127, KiCad 2 (U102.6 open: agrees) | via reliefs |
| IotThermalCamera | 88/88, KiCad 0, hole_to_hole 1 | 88/88, pass | transitive terminals (TU2.9, two nets one fewer) and via reliefs (with the terminals alone 88/88 and KiCad 1 unconnected: TU8.9 and TU8.14 apart) |
| BleJoystick | 27/27, 37 vias, pass | 27/27, 40 vias, pass | via reliefs |
| Cuprum77 BananaBoard Passive | 3/3, 48.9 mm, pass | 3/3, 42.0 mm, pass | transitive terminals |
| OpenAir rf-amplifier | 32/32, 250.5 mm, pass | 32/32, 251.8 mm, pass | transitive terminals |
| smkent dip-adapters | 17/17, 3 vias, pass | 17/17, 5 vias, pass | transitive terminals |
| 7-seg-clock | 66/66, 2208.5 mm, pass | 66/66, 2158.8 mm, pass | transitive terminals |
| mikesmitty controller | 73/78, KiCad 8 | 74/78, KiCad 7 | transitive terminals |

(with the end test and the vias' reliefs switched off, x1362, every row
but issue6284, IotThermalCamera and BleJoystick is x1365's).

**Expansion tier** (46 boards, 8 x 1, `--no-lock`), x1345 against
x1365 (x1361 and x1362 the same rows; x1361 cut brickpico by the wall
clock, the same row, x1365 cut none): 36 rows identical; passes 15 to
16 (strict 15 to 16), unconnected 764 to 723, new errors 7 to 4 (strict
3 to 2), vias 4029 to 4090.

| board | x1345 | x1365 | cause |
| --- | --- | --- | --- |
| mincrmatt12 MSign | 165/188, KiCad 60, 200 vias | 176/188, KiCad 17, 289 vias | transitive (U1.21; x1332 177/188, 15) |
| WMXZ LC_PAMV4b | 51/52, starved_thermal 1 | 51/52, pass, 26 vias (x1332's) | transitive (U3.25) |
| VTC-WISDOM node | 56/77, KiCad 33, 98 vias | 56/77, KiCad 35, 69 vias (x1332's) | transitive (H2 mounting holes, GND 113 to 108 terminals; J3's SMA pads stay two) |
| ZSWatch | 107/107, pass, 134 vias | 107/107, pass, 142 vias | transitive (IC401.33) |
| TokayEdge | 84/84, pass, 87 vias | 84/84, pass, 82 vias (x1332's) | transitive |
| ducktop2 bms | 99/99, pass, 234 vias | 99/99, pass, 240 vias, 3141.6 mm | jumpers (the WAGO terminals) and transitive (U2218's thermal vias; alone 239 vias, x1332's row) |
| brown-studios pdlc | 21/33, KiCad 12, 34 vias | 21/33, KiCad 12, 29 vias | transitive |
| Wonkle | 135/271, 496 vias | 135/271, 497 vias | transitive |
| stellarcorp magnetorquer | 84/86, KiCad 2, 139 vias | 84/86, KiCad 2, 137 vias | transitive (MH1 to MH4, GND 93 to 89, x1332's terminals) |
| trope-oshw anthracite | starved_thermal 4 | starved_thermal 2 | the spoke end test (R503.2, R703.2 now predicted and repaired) |

The magnetorquer stays at 84/86 (a pass at x1332): its terminals are
x1332's again, but the ladder keeps the plane-stubs rung, whose probe
leaves 41 nets unfinished against the tracks probe's 42 (x1332: 44
against 42); the other pour changes of the connectivity commit made the
connect probes better, and the connect rung leaves I2C3.SDA and
Net-(U8-LX1) open, which KiCad confirms (2 unconnected): honest, a
ladder choice by probe counts, not a model error.

**Short tier** (tasks v7, 4 x 2, `--no-lock`), x1345 against x1365
(x1362 at 6 x 2 the same rows): 8 rows identical; passes 7 to 8
(strict 6 to 7).

| board | x1345 | x1365 | cause |
| --- | --- | --- | --- |
| 0xCB-1337 | 47/47, 50 vias, strict starved_thermal 1 | 47/47, 66 vias, strict pass | via reliefs (hatched GND; off, x1345's row) |
| USB_Keypad | 37/39, 113 vias | 37/39, 93 vias | via reliefs (hatched; off, x1345's row) |
| eternal-keypad | 69/69, 66 vias, shorting_items 1 | 65/65, 45 vias, pass (strict: starved_thermal 2) | transitive (its D_SMD_TH diodes: 4 connections fewer) |
| urchin | 68/68, 65 vias | 68/68, 42 vias (x1332's row) | transitive (its D_SMD_TH diodes) |

eternal-keypad's two strict starved thermals are ProMicro1.4 and .23 on
B.Cu (through-hole pads of ProMicro_facedown, whose SMD pads of the same
numbers are set to no connection): the model, by label, takes them for
isolated and has no row (the per-pad experiment above gave rows that
still count two spokes against KiCad's one).

**Fingerprints** (route mode from the v16 layouts, quarter budget, 2 x
4, `PCB_ROUTER_FINGERPRINT=1 PCB_ROUTER_NO_WALL_GUARD=1`,
build/fp-pour3-x1365 against build/fp-conn-x1345; build/fp-pour3-x1363
the same): Castor, k30-SBC and
jetson identical (every fingerprint and the board); Sisu changes from
the start: U2.11 is one terminal again and J2's jumper pads one each
(GND 265 to 263 terminals, Net-(J2-RF) 5 to 4): 182/192 to 188/192,
KiCad 13 to 6 unconnected, solder_mask_bridge 14 both (with the end
test off the same row and fingerprints).

**Gates**: `bash scripts/check.sh` and the clippy gate pass.

**Left**: anthracite's J106.20 (F.Cu: a spoke the zone's outline cuts
from the pad that still crosses its ring; the lattice joins the piece it
lands in to nothing, KiCad's outline holds more than a dozen items) and J107.1 (In2.Cu:
a sliver of fill around a via the model calls outside the zone),
RS485M's JP1.3 and R3.2 (one spoke the model counts as two, one it does
not see), ZSWatch's older one is gone with the transitive terminals;
eternal-keypad's ProMicro pads (per-label zone connections, above);
robotique's BioGenius (a dimension on copper, the lowering gaps 2
package's).

## Committed on 2026-10-21 (KiCad connectivity agreement)

In the expansion intake (be4b012, 5 834 boards) 192 boards were complete
by the router's count (every connection routed, no unconnected
terminal) while KiCad's DRC with zones refilled, which loaded every one
of them, reported unconnected items; on 119 of them that was the only
new finding (docs/benchmarks.md, "Expansion intake"). The router's
"complete" and `router_complete_unverified` were statements about a
model of KiCad's connectivity that was wrong in five ways. Branch
kicad-connectivity; snapshots x1340 to x1344 on ffa49a5, x1345 (the
commit, rebased on the lowering gaps, 837b5f6). On the rebased binary
every measurement below repeats row for row: the 65 (x1333 against
x1345: the x1322 and x1344 rows), the expansion tier against
build/expansion-x1332 (the same 24 changed rows; tuxkeychain is the
lowering gaps' 35/42 in both), the short tier against build/short-x1332
(x1344's rows) and the fingerprints against build/fp-lower-x1332
(x1344's).

**The census** (`experiments/kicad-connectivity/census.py`; nothing in the
boards is executed): the intake's rows (the last per board) with the
router complete and KiCad's unconnected items, each item classified from
the routed board (items by uuid, net codes resolved) and KiCad's report;
`--run <dir>` does the same for a route-mode run of run.py.
`experiments/kicad-connectivity/tasks.py` writes
`benchmarks/agent-tasks/connectivity-tasks.json`: the 65 of the 192 whose
repository has a licence run.py fetches (the other 127 are NOASSERTION).
By the cause of the first unconnected item (boards; items of that cause
on those boards):

| cause | boards (65 with a licence) | items |
| --- | ---: | ---: |
| pour, a zone among the items: the refilled fill does not reach what the pour model connected | 58 (20) | 105 |
| pour, two items of a pour net, no zone among them | 57 (19) | 191 |
| copper graphic with a net (a line, an arc, a circle, a filled rectangle) | 30 (8) | 99 |
| pads of one number stacked on two sides | 28 (10) | 570 |
| net named by a number | 13 (5) | 176 |
| track ending on a pad without copper on its layer | 2 (1) | 4 |
| other | 4 (2) | 70 |

**What was wrong, and the fixes**, in order of boards:

- *The pour model connected what KiCad's fill does not.* A solidly
  connected pad (`connect_pads yes`, `zone_connect 2`) joined every fill
  piece within 0.8 mm of its nodes (ORBIT's U2.14: a piece 0.3 mm past
  its edge, beyond the clearance of the pads beside it, joined U2's
  island of ground pads to the rest). Now a piece joins it only where its
  fill reaches the pad's copper: a free node within the brush's half
  width, or a straight run of fill (the fill probe, off the lattice) from
  a node at most two pitches beyond (`solid_fill_reaches`,
  `PourMap::pieces_near`). A pad set to no zone connection (`zone_connect
  0`) sat in the fill like any pad of the net (1999AZZAR's J2.RN: a
  branch of no length "connected" it); it is cut out at the zone's
  clearance and half the minimum width (`isolated_knockouts`), so only a
  track into the fill joins it. The pour masks held nodes up to the
  zone's outline and up to other zones' regions; KiCad's brush keeps half
  the minimum width inside the outline and the larger clearance from a
  higher zone of another net (as the fill probe already did): a column of
  nodes on the outline joined pieces KiCad keeps apart
  (mothenjoyer69's GND_B). And of two zones of other nets with one
  priority KiCad fills the one with the larger uuid first
  (ZONE::HigherPriority; the USB-C 3-way splitter's +5V and GND, both
  priority 1): the lowering cut neither (`Pour::uuid`). KiCad caps a
  spoke's width by the pad's size, a custom pad's by its anchor's: a
  custom pad whose anchor is narrower than the zone's minimum width gets
  no spoke (Cuprum77's paw pads, a 0.1 mm anchor in large primitives,
  checked on a small board with kicad-cli: unconnected with a 0.1 mm
  anchor, connected with 1 mm); the model drew them as wide as the
  primitives' box.
- *Pads of one number on two sides were one terminal.* The lowering
  merged pads of one number that overlap (an exposed pad's thermal vias)
  without asking for a shared layer, so a USB-C receptacle's or an edge
  connector's top and bottom pads at one place were one terminal: a
  track on B.Cu stood for both, KiCad wanted them joined (Knox Key's
  P1.A1). They are merged now only when they overlap on a layer both
  have (KiCad: a pad of one number apart, or on two sides without a
  hole, is an item of its own; `duplicate_pad_numbers_are_jumpers` and
  `jumper_pad_groups` do join pads, which the lowering does not model:
  the router routes them, more than KiCad needs).
- *Copper graphics with a net.* KiCad 8 and 9 name a graphic's net by
  code (`(gr_arc ... (net 2))`); the lowering read "2", took it for no
  net and the graphic for an obstacle of none; KiCad wanted it joined to
  Net-(D1-K) (justbuchanan's ledboard). Net codes are resolved through
  the board's net table (`net_codes`, `item_net`, also for old tracks and
  vias), a filled graphic with a net was a terminal already, and a
  stroke-only one is now too: at the middle of its first piece, unless it
  lies on a terminal's copper of its net (BleJoystick's arcs, the
  chessboards' rings).
- *Nets named by numbers.* `routable_net` left out every name of digits
  as "a placeholder for a single pin"; Soraxem's usb-dmx names nets "14"
  and "0", and the router called the board complete without them. A net
  form of one unquoted number is a code, every other number a name
  (`net_name_from_form`); board_stats counts such nets too.
- *The emitted copper is checked.* The finish drops stubs that come too
  close to other nets' copper after the statuses are taken, so a net
  could be "routed" with a pad left apart. `verify::copper_components`
  joins a net's copper (pads, merged pads, existing copper, graphics,
  the emitted tracks and vias) where it overlaps on a shared layer, as
  KiCad's connectivity without pours does (a custom pad through its
  anchor only); a net routed by tracks alone whose copper leaves a
  terminal apart is partial (`copper check:` under verbose or
  `PCB_ROUTER_DEBUG`). Switched off, the expansion tier's changed rows
  and jetson's fingerprints stay as they are: it caught nothing there;
  it is the guard that keeps the router's "complete" a statement about
  the copper it writes.

Unit tests, one per cause: pcb-kicad's
`stacked_pads_of_one_number_on_two_sides_are_two_terminals`,
`a_net_named_by_a_number_is_routed`, `a_net_form_names_a_net_unless_it_is_a_code`,
`an_item_names_its_net_by_code_in_an_older_board`,
`a_stroke_with_a_net_is_a_terminal`,
`of_two_zones_of_one_priority_the_larger_uuid_is_filled_first`;
pcb-router's `a_solid_pad_joins_only_the_fill_that_reaches_it`,
`a_pad_without_zone_connection_is_kept_out_of_the_fill`,
`the_fill_keeps_its_brush_inside_the_zone_outline`,
`the_copper_check_joins_what_overlaps_on_a_shared_layer`,
`a_custom_pad_spoke_is_no_wider_than_its_anchor`.

**Route mode on the 65** (connectivity-tasks.json, the intake's settings,
30 s, 8 x 1, `--no-lock`; `census.py --run`):

| | x1322 (main) | x1340 | x1341 | x1343 | x1344 |
| --- | ---: | ---: | ---: | ---: | ---: |
| our verdict agrees with KiCad | 15 | 32 | 52 | 62 | 63 |
| complete by our count, KiCad unconnected | 50 | 33 | 13 | 3 | 2 |
| KiCad's unconnected items | 346 | 77 | 29 | 15 | 12 |
| passes (designer-relative, strict) | 14 (14) | 28 (26) | 46 (45) | 55 (52) | 56 (53) |

x1340: stacked pads and numeric names; x1341: the solid reach, the
isolated pads' knockout, net codes and stroke graphics; x1343: the
brush in the outline and clear of other zones, the uuid order and the
copper check; x1344: the custom pads' spokes. By cause (first item,
boards that disagreed at x1322, then at x1344): stacked pads 13 (10, and
3 whose first item was a zone's two sides) to 0; pour 23 to 2 (12 fixed
by the solid reach and the knockout, 8 by the brush and the zones'
order, 1 by the custom pads' spokes); copper graphics 8 to 0;
numeric names 5 to 0; other 1 to 0. No board went the other way (our
count incomplete while KiCad finds it connected): the one board our
count calls incomplete, mikesmitty's controller (5 open), KiCad finds 8
unconnected, as at x1322. Per board (our count, KiCad's unconnected
items; (x) marks a disagreement):

| board | x1322 | x1344 | x1322 cause |
| --- | --- | --- | --- |
| 1999AZZAR_pcb-skill_project_pcb_tht_opamp_eq_tht_opamp_eq | complete, 2 (x) | complete, 0 | pour net |
| 74th_74th-oshw-projects_MISC-grove_expander_MIAC-grove_expander | complete, 0 | complete, 0 | - |
| Aightech_CleverHand-hardware_pcb_hmi_interface_DRY_ELEC_6CH_THIN | complete, 1 (x) | complete, 0 | pour zone |
| Apex-Ottawa-Team_hardware_Archive_RF_Module_RF_Module | complete, 1 (x) | complete, 0 | pour net |
| Apoll011_avi-hw-mesh-node_Hardware_ORBIT | complete, 1 (x) | complete, 0 | pour zone |
| atopile_packages_packages_rj45-connectors_layouts_usage_usage | complete, 18 (x) | complete, 0 | numeric net name |
| atopile_packages_packages_saleae-header_layouts_usage_usage | complete, 4 (x) | complete, 0 | numeric net name |
| azerimaker_olmBoard-RP605x_hardware_olmBoard-RP605x-v1_olmBoard- | complete, 2 (x) | complete, 0 | pour zone |
| AzonInc_Doorman_pcb_slot_doorman_slot | complete, 1 (x) | complete, 0 | pour net |
| baxter-barlow_kicad-assistant_qa_data_pcbnew_issue11814 | complete, 2 (x) | complete, 0 | same number stacked |
| baxter-barlow_kicad-assistant_qa_data_pcbnew_issue6284 | complete, 2 (x) | complete, 2 (x) | pour zone |
| BerkeleyHCI_PolymorphicBlocks_examples_BleJoystick_BleJoystick_b | complete, 11 (x) | complete, 0 | copper graphic |
| BerkeleyHCI_PolymorphicBlocks_examples_IotThermalCamera_IotTherm | complete, 1 (x) | complete, 0 | pour zone |
| bluepylons_USB-C-3-way-splitter_V5__autosave-USB-C-3-way-splitte | complete, 1 (x) | complete, 0 | pour zone |
| bluepylons_USB-C-3-way-splitter_V5_USB-C-3-way-splitter-V5 | complete, 1 (x) | complete, 0 | pour zone |
| Cuprum77_BananaBoard_Passive | complete, 3 (x) | complete, 0 | pour net |
| dennisss_dacha_pkg_media_camera_boards_camera_vb5943_r1_board-la | complete, 1 (x) | complete, 0 | pour net |
| dennisss_dacha_pkg_vision_mocap_boards_compute_r10_board-latest | complete, 0 | complete, 0 | - |
| dennisss_dacha_pkg_vision_mocap_boards_compute_r1_board-latest | complete, 0 | complete, 0 | - |
| dennisss_dacha_pkg_vision_mocap_boards_compute_r2_board-latest | complete, 0 | complete, 0 | - |
| dennisss_dacha_pkg_vision_mocap_boards_compute_r3_board-latest | complete, 0 | complete, 0 | - |
| dennisss_dacha_pkg_vision_mocap_boards_compute_r6_board-latest | complete, 0 | complete, 0 | - |
| dennisss_dacha_pkg_vision_mocap_boards_compute_r7_board-latest | complete, 0 | complete, 0 | - |
| dennisss_dacha_pkg_vision_mocap_boards_compute_r8_board-latest | complete, 0 | complete, 0 | - |
| dennisss_dacha_pkg_vision_mocap_boards_compute_r9_board-latest | complete, 0 | complete, 0 | - |
| dragonlock2_kicadboards_projects_BSWelder_BSWelder | complete, 1 (x) | complete, 0 | pour zone |
| embeddedos-org_eCAD-Hardware-Products_boards_cad_m5stack_m5_cad_ | complete, 27 (x) | complete, 0 | same number stacked |
| enzocoralc_Tiny20_tiny20_choc_tiny20 | complete, 1 (x) | complete, 0 | same number stacked |
| enzocoralc_Tiny20_tiny20_xiao_tiny20_xiao | complete, 2 (x) | complete, 0 | same number stacked |
| Franticware_usb-to-ps1-mouse-pro_kicad_old_rev_PCB_1_usb2ps1mous | complete, 1 (x) | complete, 0 | copper graphic |
| freerouting_freerouting_scripts_benchmark_fixtures_PCBench_kitsp | complete, 0 | complete, 0 | - |
| freerouting_freerouting_scripts_benchmark_fixtures_PCBench_kitsp | complete, 2 (x) | complete, 0 | same number stacked |
| Heorenmaru_Hardware_Stm32Board_Small_oneSide_Stm32Board | complete, 1 (x) | complete, 0 | pour zone |
| Huaqiu-Electronics_kicad_qa_data_pcbnew_issue11814 | complete, 2 (x) | complete, 0 | same number stacked |
| jo5huajohn_BNO085-PCB_BNO085 | complete, 1 (x) | complete, 0 | pour net |
| justbuchanan_oasis_pcb_fabricated-boards_ledboard-v0.3_kicad_led | complete, 5 (x) | complete, 0 | copper graphic |
| justbuchanan_oasis_pcb_ledboard_ledboard | complete, 5 (x) | complete, 0 | copper graphic |
| MAHESH-THILAK-K_ADDITIONAL-PROJECT_VoltForge_Nexus_OTHER_FILES_V | complete, 1 (x) | complete, 0 | other |
| MarkWestbroek_MusicBrain_hardware_schematics_musicbrain-adc8_mus | complete, 3 (x) | complete, 0 | pour net |
| MarkWestbroek_MusicBrain_hardware_schematics_musicbrain-adc8_rel | complete, 1 (x) | complete, 0 | pour net |
| mikesmitty_power-manifold_hardware_controller_controller | open 5, 8 | open 5, 8 | other |
| mothenjoyer69_pcb-design-files_usb-uart-eqmod_usb-uart-eqmod_usb | complete, 1 (x) | complete, 0 | pour zone |
| OpenAir-Lab_capstone-project_Hardware_KiCAD_rf-amplifier-modulin | complete, 0 | complete, 0 | - |
| PantsForBirds_adsbee_kicad_prototyping_panel_proto_lna_board_pan | complete, 48 (x) | complete, 0 | same number stacked |
| PantsForBirds_adsbee_kicad_prototyping_panel_saw_eval_board_pane | complete, 48 (x) | complete, 0 | same number stacked |
| Qeteshpony_ESPFanController_pcb_ESPFanController | complete, 2 (x) | complete, 0 | copper graphic |
| robotique-udes_PCB_exo_ACE_2023_PCB_BioGenius | complete, 2 (x) | complete, 2 (x) | pour net |
| ryanhaus_rowan-baja-daq_accelerometer-board_pcb_accelerometer-bo | complete, 0 | complete, 0 | - |
| samienr_kryptonite_kryptonite | complete, 2 (x) | complete, 0 | same number stacked |
| smkent_pcb_pcb-archived_jigs_dip-adapters_dip-adapters | complete, 20 (x) | complete, 0 | numeric net name |
| smkent_pcb_pcb-archived_power_eta-buck_eta-buck | complete, 1 (x) | complete, 0 | pour zone |
| smkent_pcb_pcb-archived_stackable2_esp-cwww_esp-cwww | complete, 1 (x) | complete, 0 | pour zone |
| smkent_pcb_pcb_power_microbuck_microbuck | complete, 2 (x) | complete, 0 | pour zone |
| sommerper_framework16-keyboard-v1_kicad_framework16v1_framework1 | complete, 68 (x) | complete, 0 | same number stacked |
| Soraxem_trance_devboard_pcb_devboard | complete, 11 (x) | complete, 0 | numeric net name |
| Soraxem_trance_usb-dmx_pcb_usb-dmx | complete, 10 (x) | complete, 0 | numeric net name |
| Squantor_various_PCB_electronics_DIM_powergate_SOIC-8_DIM_powerg | complete, 0 | complete, 0 | - |
| Squantor_various_PCB_electronics_DIM_SN6505_PushPullConv_DIM_SN6 | complete, 1 (x) | complete, 0 | pour zone |
| swayerloren_KICAD_ENGINE_32_OPEN_KICAD_SAMPLE_INTAKE_normalized_ | complete, 1 (x) | complete, 0 | pour net |
| tdunning_low-pass_low-pass-2-sides_low-pass | complete, 2 (x) | complete, 0 | pour zone |
| tdunning_low-pass_low-pass_low-pass | complete, 2 (x) | complete, 0 | pour zone |
| urichess_chessboard_kicad_projects_others_CHESS_PIECES_DETECTION | complete, 4 (x) | complete, 0 | copper graphic |
| urichess_chessboard_kicad_projects_others_CHESS_PIECES_DETECTION | complete, 4 (x) | complete, 0 | copper graphic |
| urichess_chessboard_kicad_projects_others_READER_READER | complete, 3 (x) | complete, 0 | copper graphic |
| zOnlyKroks_pcb-consortium_Misc_7-seg-clock_7-seg-clock | complete, 0 | complete, 0 | - |

The two that still disagree: robotique's BioGenius (a dimension drawn
on B.Cu, which KiCad's fill keeps clear of and the lowering does not
lower: a band across the board our pour fills; the no-net copper of the
lowering-gaps package) and KiCad's own QA board issue6284 (a spoke joins
U102.4 and U102.6, which KiCad keeps apart). The 127 boards without a
licence were not routed again (run.py does not fetch them); by cause
they are the same mix as the 65 (table above).

**Expansion tier** (46 boards, 8 x 1, `--no-lock`; build/expansion-x1343
against build/expansion-x1322; build/expansion-x1344 is x1343's row for
row): 22 rows identical; passes 14 to 15
(strict 14 to 15), unconnected 787 to 758, new errors 18 to 20 (strict
15 to 16), vias 4022 to 4032; compare.py: better 7, same 37, worse 2.
Every changed row comes from one of the changes, found with a binary
whose changes could be switched off one at a time (build/xtog-*, not
committed) and from the boards' features:

- the new terminals: pads of one number on two sides on VTC-WISDOM's
  node (14 pads; 35 to 33 unconnected, 69 to 98 vias), ZSWatch (10; its
  starved thermal gone: a pass), ducktop2's bms (2), Wonkle (2),
  stellarcorp's magnetorquer (8; 86/86, a pass, to 84/86: the ladder kept
  its connect rung, I2C3.SDA and Net-(U8-LX1) open, KiCad 2 unconnected,
  so honest), mincrmatt12's MSign (2, with the brush below: 177/188 and
  15 unconnected to 165/188 and 60), TokayEdge (8; vias), LC_PAMV4b (2;
  a pass to one starved thermal, 51/52 either way) and pdlc (2); filled
  graphics named by code now terminals on bread-modular's noise (vias,
  length), minuet's light (22/23 to 23/23, a pass), pdlc (16/33 to
  21/33, 21 to 12 unconnected) and CleverHand's EMG_DAQ (21/22 to 22/22,
  a pass);
- the pour masks' brush (in the outline, clear of other zones): iorodeo's
  driver (170 to 109 unconnected), RS485M (2 to 1 unconnected, two
  starved thermals), Zulassungskarte (2 to 1), and in vias, length or
  work only Nervous-ECG, PCBMints, brickpico, OpenRX-Mono, AMLing2,
  FUEL4EP, anthracite, mikesmitty's backplane and nrf24_remote.

**Short tier** (tasks v7, 4 x 2, `--no-lock` while the designer tier held
the lock; build/short-x1344 against build/short-x1322, no row cut): 3 rows
identical (USB_Keypad, MIDAS, CharlieBoard), passes 7 to 7 (strict 7 to
6), unconnected 19 to 18, vias 1125 to 1116, work 2517 to 3080 s.
compare.py: better 1 (WLED_Matrix 101/104 to 100/104, 6 to 5
unconnected), same 11. The 9 changed rows, with both brush bands
switched off (build/stog-NOBANDS): back to x1322's on reCamera (vias,
length, first-route open 5 to 7), 0xCB-1337 (a starved thermal: no
longer a strict pass), USB_Cable_Tester (128 to 95 vias),
spell_tome_bottom (work 30.1 to 30.0 s) and framework_mobo_lefthalf
(vias, length); still x1344's on eternal-keypad (200 pads of one number
on two sides, its reversible switch footprints: 65 to 66 vias, a starved
thermal) and urchin (136 such pads: 42 to 65 vias); WLED_Matrix (two
filled rectangles named by code, now terminals) and OpenFC (12 custom
pads and solid zones: the custom pads' spokes and the solid reach, which
the switches do not cover) neither.

**Fingerprints** (route mode from the v16 layouts, quarter budget, 2 x 4,
`PCB_ROUTER_FINGERPRINT=1 PCB_ROUTER_NO_WALL_GUARD=1`, build/fp-conn-x1343
against build/fp-lower-x1322; build/fp-conn-x1344 has x1343's
fingerprints on all four): Castor and k30-SBC identical at every
phase; jetson identical up to the clean-up of its tracks attempt, which
the pour masks' brush changes (with both bands switched off every
fingerprint is x1322's), the board identical (324/340, 620 vias, 10791.4
mm, KiCad 19 unconnected); Sisu changes from the start: U2's pad 11 (GND)
on F.Cu and again on B.Cu at one place is two terminals now (183/192 to
182/192, KiCad 13 unconnected both, solder_mask_bridge 15 to 14).

**Gates**: `bash scripts/check.sh` and the clippy gate pass.

## Committed on 2026-10-21 (lowering gaps)

The two rules agents named KiCad findings the router could not see: the
edge clearance from NPTH holes and Margin graphics, copper graphics of no
net, and stubs of two nets leaving neighbouring pads. This closes them
and four more gaps the remaining findings showed, all in the lowering
(`board_router/lower.rs`, `geometry.rs`, `roundrect_pad.rs`,
`project_rules.rs`) except the stub check (`pcb-router`: `verify.rs`,
`router/finish.rs`); docs/router.md, "Board edges and copper of no net".

Binaries: x1322 is main before (ffa49a5); x1330 and x1331 steps on the
way (x1331: the edge clearance, copper of no net, footprint graphics and
texts, the stub check); x1332 all of it, measured; x1333 the commit (a
tuple of the lowering made a struct for clippy: the 20 rules boards
identical to x1332, rows and boards).

**What changed.**

- **Edge clearance from unplated slots.** KiCad 10.0.7's
  copper_edge_clearance takes an NPTH pad whose drill sizes differ (a
  milled slot) for board edge; a round NPTH hole, an oval drill of equal
  sizes and a plated slot keep only the hole clearance (kicad-cli on a
  board made for it: "NPTH pad of H2" at 0.2 mm, nothing for the others).
  The slot's hole obstacle takes the edge clearance as its floor (tracks
  and vias). buganini's USB-C shell slots (4 findings) and facade60's
  switch slots (3).
- **Margin graphics.** A graphic on Margin, on the board or in a
  footprint, is board edge to KiCad with its stroke and fill (a text on
  Margin is not): a keepout on every copper layer at the edge clearance
  (`margin_obstacles`; the zone filler knocks Margin out too). michpro's
  "Segment on Margin" (1).
- **Copper of no net keeps the Default class.** Board graphics, footprint
  copper graphics and footprint texts on copper of no net carry Default's
  clearance as their floor (a graphic whose net is not routed, that net's
  class): KiCad keeps the larger of both items' clearances, the router
  kept only the routed net's.
- **Footprint graphics on several layers.** KiCad 9 draws a footprint
  graphic on `(layers "B.Cu" "B.Mask")`; only `(layer ...)` was read, so
  tuxkeychain's GRAPHIC3 logo was neither copper nor a mask opening (10
  findings, tracks and vias 0.01 to 0.07 mm from it). Footprint graphics
  now lower on every copper layer they name and open every mask layer.
- **Footprint texts on copper and mask layers.** A visible `fp_text` or
  `property` on a copper layer is copper of no net, on a mask layer an
  opening (`footprint_text_box`: the stroke font's box in the
  footprint's frame; KiCad 7 and 8 store the angle relative to the
  footprint, checked with kicad-cli; knockout texts their plate, hidden
  ones nothing). The intake's jnftech test points (Value fields on F.Cu:
  clearance and shorting_items) and Tert2's Reference on a mask layer.
- **Pad-centre stubs keep the rules.** The cosmetic stub from a route's
  node inside a pad to the pad's centre is drawn at the track's width; on
  a pad narrower than the track it reaches past the pad. It is checked
  against fixed copper when laid and against routed copper when the
  routes are emitted, where `finish` drops a stub too close; that check
  (`verify::verify_with_rules`, the classes plus `ClassRules`' pair
  clearances per item kind and layer; `verify` itself unchanged) now
  holds the rules. tuxkeychain's three "Minimum Trace Spacing (outer
  layer)" findings: GND's 0.3 mm stubs along U4's 0.2 mm QFN pad 2,
  0.104 mm from HPL's track and stub under 0.127. Measured alone (x1322
  with only this change, tuxkeychain): the same routes less three stub
  segments, clearance 13 to 10 (the GRAPHIC3 ten), 5 unconnected both.
- **Arcs.** A copper (or mask, or Margin) arc graphic is flattened to
  chords up to 0.005 mm inside the arc; its stroke now grows by that
  tolerance. name-card's antenna AE1 (a 10.7 mm arc): a track 0.1978 mm
  from it under 0.2 (2 findings).
- **Zero chamfers are sharp corners.** A rounded-rectangle pad's corner
  named in `(chamfer ...)` with `chamfer_ratio 0` is sharp in KiCad (not
  rounded; kicad-cli: 0.1546 mm from the corner, 0.32 from a rounded
  one); the lowering rounded it. AQEYE's solder jumpers JP1 and JP2 (2
  findings, tracks 0.126 and 0.112 mm from the inner corners).
- **Net keys of the project rules.** `resolve_project_rules` keyed the
  connections before noting the board's bare net names: on a board with
  both `/VIN_CELL` and `VIN_CELL` (meshsat) the rules were keyed
  `VIN_CELL` while the lowering asked for `/VIN_CELL` and fell back to
  Default's (0.25 mm tracks, 0.7 mm vias, 0.14 clearance against CELL's
  0.8, 1.0 and 0.15: 6 findings).

**Rules boards** (rules-tasks.json, route mode, 30 s, 8 x 1, `--no-lock`),
x1322 against x1332: 14 rows identical; passes 12 to 18 (strict 12 to
18), new errors 18 to 0 (strict 19 to 1), unconnected 6 to 4, vias 493
to 464, work 233 to 230 s.

| board | gap | x1322 | x1332 |
| --- | --- | --- | --- |
| buganini Kikakuka gerber | NPTH slots as edge | 5/5, copper_edge_clearance 4 | 5/5, pass |
| facade60 main | NPTH slots as edge | 92/92, 98 vias, copper_edge_clearance 3 | 92/92, 69 vias, pass |
| michpro PY32F002 | Margin graphic | 12/12, copper_edge_clearance 1 | 12/12, pass |
| name-card | arc chord error | 13/13, clearance 2 | 13/13, pass (one track a node over) |
| AQEYE-SB-D1 | zero chamfer | 28/28, 23 vias, clearance 2 | 28/28, 25 vias, pass |
| meshsat pcb-d-aprs | net keys | 60/61, 2 unconnected, clearance 6 | 61/61, pass |

x1331 (without the last three) changed the first three rows only. The two
rows left failing are ChengminLi (the courtyard rule's dead jumper pads)
and kisurf (8 mm between pins of one part; strict starved_thermal 1),
both as on x1322.

**Expansion tier** (8 x 1, `--no-lock`), x1322 against x1332 (x1331 the
same): 45 rows identical; tuxkeychain 38/42 to 35/42, clearance 13 to 0,
unconnected 5 to 11, vias 69 to 66. Its GRAPHIC3 logo is B.Cu copper the
routes crossed within 0.01 mm before; routing around it at 30 s leaves
SDC0_D3, SDC0_D0, SPI1_MISO and SPI_MISO open where SPI1_CLK was (no dead
pads). Totals: unconnected 787 to 793, new errors 18 (strict 15) to 5
(strict 2), vias 4022 to 4019, 14 passes both. The new errors left are
starved_thermal on ZSWatch (1) and anthracite (4, strict 1): the pour
model, not the lowering.

**Fingerprints**: Castor, k30-SBC, jetson and Sisu, route mode from the
v16 layouts (`PCB_ROUTER_FINGERPRINT=1 PCB_ROUTER_NO_WALL_GUARD=1`, 2 x
4, budget scale 0.25), x1322 against x1332: every phase fingerprint, the
board (md5) and the row identical. All four have copper graphics
(footprint polygons of no net on Castor, k30 and jetson, now at
Default's clearance, which no class there exceeds; Sisu's GND
rectangles and polygons), jetson two copper arcs, none near enough a
route to move it; none has a Margin graphic, an NPTH slot, a zero
chamfer, a footprint graphic on several layers or a visible footprint
text on copper or mask.

**Short tier** (tasks v7, 4 x 2, `--no-lock` while the designer tier
held the lock; results count work), x1322 against x1332: all 12 rows
identical (routed, vias, length, KiCad's findings, work), all 12 result
boards byte-identical, no row cut; 7 passes, 19 unconnected, 1125 vias
both. framework_mobo_lefthalf's one NPTH slot is the only item of these
gaps in the tier (Default is the smallest class on every short-tier
project, so copper of no net keeps what it kept); it moved nothing.

**Four intake boards with the text findings** (the intake's stripped
boards and router settings, route mode, 30 s of work, kicad-cli's DRC;
in the scratchpad, not a tier), x1322 against x1332: jnftech's SOIC
board clearance 1 and shorting_items 1 (Value fields) to none, 4
unconnected both; its UQFN board clearance 5 and shorting_items 2 to
none, 3 unconnected both (annular_width 6 both: the designer's); Tert2's
jogi solder_mask_bridge 11 (Reference fields on F.Mask) to 0,
unconnected 43 to 22 (the edge, hole and clearance findings left are
the designer's); stonedDiscord's ZLKalt shorting_items 1 to 0 but 0 to
4 unconnected: the router's pour-connect attempt now counted 0 open and
was kept without KiCad's check (the intake's settings), and KiCad's fill
of the B.Cu GND zone leaves four pieces apart (the pour model, not the
texts, which are on F.Cu).

**Lowering gaps in the intake's findings** (the 2026-10-18 intake's
result DRC on 1 725 boards, an older binary; findings of routed copper
against a fixed item, on boards with more of the type than the
designer's): closed here: copper_edge_clearance against NPTH slots (28
on 5 boards) and Margin graphics (31 on 5), footprint fields on copper
(clearance 8 and shorting_items 4 on jnftech's boards, shorting_items 1
on stonedDiscord's ZLKalt) and on a mask layer (solder_mask_bridge 16, Tert2).
Not lowering, closed before: clearance against pads and track_width
(the project rules), copper_edge_clearance against Edge.Cuts and
hole_to_hole against NPTH pads (rules' values, such as conorcurley5's
"Clearance: hole to hole (perimeter), different nets"). Left: dimensions on
copper (shorting_items 3 and clearance 1, two boards: KiCad counts a
dimension on a copper layer as copper), a multi-line copper text
(TomKeddie's supercon SAO, 3, not analysed), solder_mask_bridge against
pads and mask graphics on 11 boards (38 findings, after the 2026-10-15
mask model; not analysed here), track_dangling and
track_not_centered_on_via (the router's output, not the lowering).

Unit tests: board_router lowering_gap_tests (an NPTH slot, a round hole,
a plated slot, Margin on the board and in a footprint and a Margin text
against tracks through `verify`; a footprint polygon on B.Cu and B.Mask,
a board line of no net at Default's clearance against a smaller class,
a footprint Value on F.Cu at its turned position, a hidden field, a
footprint text on F.Mask), roundrect_pad (a zero chamfer is sharp),
project_rules (a hierarchical net beside a bare one keeps its class;
fails without the fix), pcb-router rule_tests (`verify_with_rules` holds
a track-to-track rule `verify` does not, not against a via). `bash
scripts/check.sh` and the clippy gate pass.

Logs: build/rules-x1331, -x1332, -x1333; build/expansion-x1331, -x1332,
build/expansion-lower-stubonly (the stub check alone); build/fp-lower-x1322,
-x1332; build/short-x1332.

## Committed on 2026-10-21 (ladder fault isolation)

Review finding 8: `route_kicad_board` (board_router/route.rs) was about
1 500 lines of closures over `Cell`s with no test of its control flow; its
worker threads were joined with `.expect`, and two asserts reachable from
the input (nets in 15 bits, layers at most 32) panicked, so a panic in one
probe or rung lost every board the ladder held (exit 4). And the finer
pitch's machine-memory gate read the process's peak resident size, which
follows how many probes ran side by side. Branch ladder-fault-isolation;
snapshots x1310 (main at b306347, the baseline), x1311, x1312 (this on
b306347), x1313 (on 535edb5), x1314 (on 5a99d89).

**The ladder as steps** (board_router/ladder.rs, a move). `Ladder` holds
the ladder's state (its work, the reserve of a layout's ladder and route
mode's, the work bound, the best board and attempt, the check gate, the
skipped steps, the failures) and runs its steps in order (`Ladder::run`):
`probes` (the rung probes side by side, their leader's continuation,
`continue_probe`), `rungs` (per pitch: `pitch_skipped` for the finer
pitches' four gates, `pitch` with `rungs_side_by_side`, `rung_wave`, the
last rung's `seed_retry`), `seed_rounds`, `steps` (narrow-signal, then
minimum clearance, each `step`). The closures became methods
(`spent`, `bound_left`, `rung_left`, `judge`, `step_has_time`, `blocked`,
...); `ladder_rungs` and `ladder_pitches` are the rung and lattice lists.
What routes, checks and measures is a `LadderRunner`: route.rs's
`KiCadRunner` (lowering, `Router::new`, `probe`, `resume`,
`write_attempt`, `route_kicad_board_once`, `check_attempt`, the memory
limits) and the scripted `Fake` of ladder_tests.rs. route.rs went from 2 146
to 800 lines (the preparation, the runner, the finish, `route_seeds`,
`core_config`, `finish_routed_board`); every log line and json field
stays.

**Fault isolation.** Every probe (its router's build too), the
continuation, every rung (in its thread), the rung's seed retry, every
seed round and both steps run under `catch_unwind` (`isolate`; a thread
join's error counts the same, `route_seeds` passes a seed thread's panic
on with its own message), and so does KiCad's judgement of each. A panic
is logged (`ladder: the rung pours=tracks pitch=regular failed (panic:
...); the ladder goes on with its best board`), listed in
`attempts_failed` of board-router.json (absent when empty) and in the
verdict's `next_actions` ("N attempts of the router's ladder failed ...:
the board is the best of the other attempts; report the panic"), and the
ladder goes on: a failed probe counts no work and leaves the lead to the
next best; a failed continuation leaves the probed rung to the regular
rungs; a failed rung or seed round leaves the best board. Only when no
attempt produced a board does the run end in an error, an internal one
(exit 4: "no attempt of the ladder produced a board: N failed (...)"),
where `expect("at least one routing attempt")` panicked before. The test
hook `PCB_MAKER_TEST_PANIC_ATTEMPT=<kind>[:<n>|:all]` (probe,
continuation, rung, seeds, step) makes the n-th attempt of a kind panic;
it is in `RESULT_ENVIRONMENT`. Not covered: a panic outside the attempts
(the preparation, the finish, the neck repair) still ends the run, and
main.rs's panic hook keeps the first panic's message, which may then be
an isolated attempt's.

**Capacity limits** (pcb-router grid.rs, build.rs). `check_capacity`
returns an error naming the limit and the board's count: more than
`MAX_NETS` (65 533) nets ("the board has N nets: the router names nets in
16 bits, at most 65533") or `MAX_LAYERS` (32) copper layers. The lowering
calls it and returns it as an input error (exit 2); `Router::try_new`
returns it; `Router::new` and the static maps keep it as an assert. The
net limit went from 15 bits (32 766 nets) to the 16 of the static maps:
the search records (`Hot`) hold `owner(net)` in a u16 and the bit kept
for a hard guard flag is unused since the router split (changelog,
"quality: pcb-router").

**Memory decisions.** The finer-pitch gate against 0.4 of MemTotal now
takes the router's estimate from the first attempt's maps
(`router_peak_mb`, resident) instead of the process's peak resident size
after the first rung (which included the probes side by side and, in a
layout, the first route); the probes' first router no longer falls back
to the growth of the resident size when its memory report has no maps.
No ladder decision reads the peak any more, so no `cut` kind is needed;
how many routers run side by side still follows what is free now
(MemAvailable, the cgroup's use), which by design changes no result. The
layout's placement race (board_layout.rs, not this package) sizes its
side-by-side count from `memory_mb()`'s MemAvailable the same way.

**Folded in from the thermals package.** The kept attempt's
`island_stubs` (pcb-router's `RoutingResult::island_stubs`) is in
board-router.json and the verdict's `repairs.island_stubs` (was `null`);
`PCB_ROUTER_ISLAND_STUBS` is in `RESULT_ENVIRONMENT` in place of
`PCB_ROUTER_NO_ISLAND_STUBS`. The lowering records each pad's zone
connection by obstacle index beside the label lists
(`Board::solid_pad_obstacles`, `isolated_pad_obstacles`: the pad's own
`zone_connect`, else its footprint's), so that 74th's J3.S1 (SMD shield
pads without a zone connection, through-hole ones with the zone's) can be
told apart; the router does not read them yet (`kicad_spokes`,
`thermal_ring_pads`, `prepare_net` are the follow-up).

**Tests.** ladder_tests.rs (11, the scripted runner): the rung and
lattice lists; the hook's parsing and `isolate`; four probes sharing half
the budget, the leader continuing with at most half of what is left and
its deadline at the bound, a complete continuation ending the ladder; a
panic in one probe (the hook and a router panic) recorded and the next
best leading; a panicked continuation falling back to all rungs; route
mode's first rung with all that is left, its work kept for the
narrow-signal step, the next rung's share, the seed retry, the step last
with what was kept, every router with a work deadline and the work within
the bound; a panic in a rung that runs beside others keeping the best
board and no rung directory behind; every attempt failing leaving no
board; a panic in the seeds keeping the rung's board; a layout's ladder
keeping 300 s for the step with the next rung's deadline where the
reserve begins; a finer pitch skipped by its maps against a small
machine. grid.rs: the capacity limits. tests.rs: each pad's zone
connection. cli_exit_codes.rs: `PCB_MAKER_TEST_PANIC_ATTEMPT=rung` on
the three-pad board (the regular rung panics, the 0.075 mm rung's board
is kept, exit 1 as without KiCad's check, `attempts_failed` and the
next action), `rung:all` (exit 4, "no attempt of the ladder produced a
board: 3 failed"). The probe hook through the binary needs a board of four
layers and 120 connections; the scripted runner tests it.

**Identity.** Route mode on the v16 placements of Castor, k30-SBC,
jetson and Sisu (`PCB_ROUTER_FINGERPRINT=1 PCB_ROUTER_NO_WALL_GUARD=1`,
4 threads), x1310 against x1312: the same board (md5), the same
fingerprints (4, 6, 29, 26), vias (159, 560, 615, 410) and work (310.7,
448.1, 1219.0, 989.5 s) on every board; the rows differ only in
`repairs.island_stubs` (null to 0). Short tier (tasks v6, under the lock)
x1310 against x1312: every row identical apart from timing and
`repairs.island_stubs`, all 12 result boards byte-identical (19
unconnected, 1125 vias, 2517 s of work, 7 passes; against short-x1170
main itself moved: 1134 to 1125 vias, 2501 to 2517 s, the thermals
package). Expansion tier (30 s, 8 x 1) x1310 against x1312: all 46 rows
identical apart from `routing_seconds` and `repairs` (807 unconnected,
4038 vias, 1279 s of work, 14 passes, 0 cut). Rebased on 5a99d89 (the
runtime calibration, project rules), x1277 (main) against x1314 with
`PCB_WORK_RATE=3.15e6,150e6`: the same four fingerprint runs identical
(boards, fingerprints, vias, work; rows apart from `repairs` and the
measured rates and work/wall ratio), the short tier identical (all 12
boards byte-identical, 2517 s of work, 7 passes) and the expansion tier
identical on all 46 rows (787 unconnected, 4022 vias, 1275 s, 14 passes,
0 cut) apart from timing and `repairs`. Logs in build/ladder3 (qb-, qb2-,
short-, short2-, expansion-, expansion2-).

## Committed on 2026-10-21 (router rule features)

The project rules work (below) lowered the project's rules but left
three kinds the router could not keep: width floors per net, clearances
between two sets of nets, per item type and per layer, and the hole to
hole spacing within a net. This adds them to the router
(`crates/pcb-router/src/rules.rs`, `Config::net_rules`) and to the
lowering (`project_rules.rs`, `project_rules/dru.rs`); docs/router.md,
"Project rules" and "Rules beyond the classes".

Binaries: x1277 is main before (5a99d89; main's x1315 after the ladder
refactor routes identically), x1320 this work on 98efa4a, x1322 the
commit (rebased on 6f10989, with the two changes the expansion tier
asked for, below). x1321 was not measured.

**What changed.**

- `pcb_router::NetRules` (`Config::net_rules`, `None` by default):
  `width_floor` per net name, `clearances` (`ClearanceRule`: two sides
  as net names or every item, the pairs of item kinds as bits of track
  or via against track, via or pad, the layers, the clearance) and
  `same_net_hole_to_hole`. `KiCadBoardRouterConfig::net_rules` (a skipped
  field, one line in board_router/config.rs) carries it from
  `resolve_project_rules` to `core_config` (one line in
  board_router/route.rs); main.rs's file merge copies it.
- The router splits its rule classes by group (the nets no rule tells
  apart) and width floor, keeping a class's index for the first nets
  that need no other: rules that split nothing route byte for byte as
  without them. Stamps are built per pair of classes and per layer
  variant (one without layer-qualified rules); static maps keep pads,
  existing tracks and vias at the rules' clearance per layer; pad stubs
  (`stub_is_clear`) too. A net's neck class is its floor where that is
  wider than the board's neck; stubs and escapes take it.
- Same-net hole to hole: where the project rates hole_to_hole an error,
  a via keeps the board's hole to hole from its own net's pad holes.
- The lowering evaluates a clearance rule per pair of net groups, pair
  of item kinds and layer; a rule true everywhere for a set that raising
  covers stays a per-net clearance, anything else becomes a pair rule
  (`applied`; `approximated` where it tells SMD from plated pads). A
  per-net `track_width` minimum floors that net's neck-downs and narrow
  width (`applied` where it was `approximated`).
- KiCad applies the last matching rule: a clearance, hole clearance,
  edge clearance or hole-to-hole rule no longer counts where a later rule
  with a minimum of the same constraint surely matches the pair (the
  groups tell apart what the later conditions do). JLCPCB's template
  ends with "Min. Spacing" 0.127 mm on everything, after its "Pad to
  Track" 0.2 mm: KiCad checks 0.127 between pads and tracks (AMLing2's
  drone carrier, where KiCad found none of the 0.2), and so does the
  router now (`no_effect`, "a later rule takes every pair it matches").
- A pad keeps what tracks of its net keep from other nets' copper too:
  its net leaves it by a stub off the lattice that only the pad's
  clearance protects (x1320's tuxkeychain: 0.127 mm between outer-layer
  tracks over classes of 0.1 left stubs 0.104 mm from other tracks).

**Route mode on the 20 rules boards** (rules-tasks.json, 30 s, 8 x 1),
x1277 against x1322 (x1320 the same rows): 17 rows identical (routed,
vias, length, KiCad's findings, work); passes 11 to 12, new errors
(strict) 38 to 19, unconnected 2 to 6, vias 494 to 493.

| board | rule | x1277 | x1322 |
| --- | --- | --- | --- |
| nuclone (Squantor) | hole to hole 0.5 (error), same net | 18/18, hole_to_hole 2 | 18/18, pass |
| ChengminLi flyback | 2.5 mm primary to secondary, pair rule | 38/38, clearance 1 | 35/38, 2 unconnected, no error |
| kisurf issue24211 | 8 mm feedback to switch node, pair rule | 34/34, clearance 11 (strict 17) | 32/34, 2 unconnected, no clearance (strict: starved_thermal 1) |
| the other 17 | | | identical |

ChengminLi's two open connections are the pins of the Y capacitor's
jumper YPS (primary PSY1, PSY2 and secondary SGND, 2.54 mm apart): its
file loosens the isolation there by a later rule, "YPS jumper pins
(intended bridge)", 0.5 mm between items inside YPS's courtyard, which
the router cannot evaluate (`intersectsCourtyard`): it keeps 2.5 mm and
no track reaches those pads (diagnostics.dead_pads names them). kisurf's
rule asks 8 mm between the feedback and the switch node, whose pins sit
on one part (U4.2 and U4.8, 3 mm apart): no track can leave either pin
and keep it; the designer's own board breaks it (10 findings). Both are
dead pads of the rule, not of the router; the earlier per-set raise
(x1270) had left 19 and 8 open there.

**Expansion tier** (x1277 against x1322, 8 x 1, no row cut): 45 rows
identical, unconnected 787 to 787, new errors (strict) 17 to 15, vias
4022 to 4022, 14 passes both. tuxkeychain: 37/42 to 38/42 routed, 5
unconnected both, clearance 15 to 13: the 9 "Pad to Track clearance"
findings (vias 0.187 mm from tracks under 0.2, the rule matches plated
items: vias here) are gone; 3 tracks of GND and HPL 0.104 mm apart
where both leave neighbouring pads by stubs under the outer-layer 0.127
rule, and 10 (was 6) tracks and vias within 0.1 of a copper graphic
polygon without a net (GRAPHIC3: the lowering's graphics, not a rule)
remain. AMLing2 (x1320 without the later-rule change: 10 to 30
unconnected, the "Pad to Track" 0.2 kept) is identical to x1277.

**Short tier** (x1277 against x1320 and x1322, tasks v7, 4 x 2): all
12 rows identical on both; no short-tier project has a .kicad_dru rule.

**Fingerprints**: Castor, k30-SBC and jetson (projects rating hole to
hole an error, so their rules hold the same-net spacing and split
nothing) and Sisu, route mode from the v16 layouts with
`PCB_ROUTER_FINGERPRINT`, 30 s: every phase fingerprint of x1320 and
x1322 equals x1277's.

**Anthracite** (the coordinator's question on the project rules
change): GNDPWR is in `power` (explicitly, priority 4) and `GND` (by
pattern, priority 0); KiCad's composite class takes GND's width 0.2 and
via 0.6, as the router does since x1275 (the clearance is 0.15 in
both). A class width is no minimum in KiCad's DRC, so the narrow width
need not keep 0.5. The thermal spokes follow the zone, not the class:
the spoke model draws them at the zone's thermal_bridge_width
(`thermal_reach - thermal_gap`) and drops those narrower than its
min_thickness (`kicad_spokes`). starved_thermal 1 to 4 came with 2 to
0 unconnected: different pads (J107, J106, R503, R703 against J104),
other routing.

**What stays unmodelled**: rules on areas, courtyards, footprints,
component classes (their conditions are unknown), also where a later
such rule loosens an earlier one (ChengminLi's jumpers); creepage,
lengths, skew, diff pairs, physical clearances, disallows other than
blind, buried and micro vias; the edge clearance from NPTH holes and
Margin graphics; pair rules in the pour model (fills keep the classes'
clearance from other nets' tracks) and in the internal verifier (KiCad
checks them); stubs of two nets leaving neighbouring pads are not
checked against each other (tuxkeychain's 3).

Unit tests: pcb-router `router::rule_tests` (a pad-to-track rule keeps
tracks from pads only, a rule between two sets splits their classes with
stamps per layer variant, a width floor holds for the neck, the same-net
hole spacing blocks a via the copper rule allows, rules that split
nothing route identically); project_rules::dru (pair rules between two
classes, item types and layers as pair rules on two and four layers,
SMD pads approximated, the last matching rule); project_rules (a per-net
width floor, a project with an isolation rule, JLCPCB's pad to track
and an outer-layer via rule, hole_near_hole as an error). `bash
scripts/check.sh` and the clippy gate pass.

Logs: build/rules-x1277, -x1320, -x1322; build/expansion-x1277, -x1320,
-x1322; build/short-x1277, -x1320, -x1322. x1322's tiers ran with
`--no-lock` while the designer tier (x1315) held the lock for hours
(results count work, not wall time).

## Committed on 2026-10-21 (config reference and stale docs)

Review finding 10: the agent-facing surface is about 130 config keys and
about 50 environment variables, most keys named in no document, and the
documents carried stale statements.

**The reference is generated.** `tools/config-reference` (a workspace
crate; `cargo run -p config-reference`) writes
[config-reference.md](config-reference.md) from the config structs: the
field names, types, doc comments and serde attributes from their
sources, the defaults from the structs at run time (a settings file of
`{}` read through serde and serialized again; the core configs'
`Default` printed with `{:#?}`; a default read from the environment, as
the router's `island_stubs`, shown with and without its variable). One
table each for the router settings (with `topological`, `pin_swap`,
`add_pours`, `net_classes` and the connection rules), the router core
(`pcb_router::Config`, with the settings `core_config` copies into each
field, read from that function), the placer settings, the layout
settings (with `swappable`), the placer core (`Config`, `global`,
`anneal`, with what `place_kicad_board` copies), and, as an output,
board-router.json's `rules` (the project-rules report). Each key is
marked when docs/router.md or docs/placer.md names it in code, and
internal (`#[serde(skip)]`) fields are marked as such. The same tool
writes the hand-over's environment index ("Switches", one line per
variable, grouped user-facing, switches, diagnostics, tests): every
`PCB_*` string literal of the crates' sources outside tests needs its
line in the tool's `ENVIRONMENT`, a line whose variable the code no
longer names is an error, and every user-facing variable must be in
`pcb-maker --help` (`ENVIRONMENT_HELP` in src/main.rs).
`tests/config_reference.rs` (in scripts/check.sh) fails while either
file differs from what the tool makes of the tree, or while one of those
checks fails; it names the first differing line.

Counts (13 structs, 249 fields; the three report structs aside): 155
file keys, 12 internal (serde-skipped: 10 router settings, 2 placer
settings), 82 fields of the core configs. With a doc comment: 194 of 249
before, 233 now; the other 16 are in crates/pcb-kicad/src/board_router/
config.rs (14 router settings, 2 of `topological`), which the ladder
fault isolation package has open: their comments are written and wait
as a patch for after it lands. Named in router.md or placer.md: 35 of
the 155 file keys (unchanged; the reference now documents every key).
No field is unused: each is read somewhere outside its definition.

**Doc comments written** for every field that had none (from the use
sites): router core `present_factor`, `present_growth`,
`history_increment`, `max_iterations`, `via_reduction_factor`,
`verbose`; placer settings `keep_rotation`, `pins_per_halo_track`,
`maximum_halo_mm`, `seed`; layout `placer`, `pin_swap`,
`congestion_candidates`, `congestion_probes`, `congestion_score` and
`swappable[].part`; `net_classes[]` `name` and its four sizes; the
connection rules' four sizes; `pin_swap.seed`; placer core `global`,
`anneal`, `refine_passes`, `global.bins`, `stop_overflow`,
`max_iterations`, `frame_interval`, `seed`, `routing_layers`,
`track_pitch`, `anneal.stages`, `moves_per_part`, `seed`.

**Stale statements fixed** (no behaviour change except the verdict's
environment echo and the KiCad version warning):

- router `stall_drop`: the doc called 0 pathological while 0 is the
  default. It now says what the code does: 0 turns the threshold off
  (any new low counts, as below the cap), the final ladder keeps it, and
  the layout's in-place router (first route and moves) and placement race
  set 0.1 unless the router settings give `stall_drop`.
- router `seed` carried `verbose`'s first line; each has its own now.
- `pcb-maker --help` listed five variables; it lists the fourteen
  user-facing ones (added `PCB_WORK_RATE`, `PCB_KICAD_TIMEOUT`,
  `PCB_MIN_CONNECTION_FLOOR`, `PCB_KICAD_NO_NECK_REPAIR`,
  `PCB_ROUTER_ISLAND_STUBS`, `PCB_MAKER_CACHE_DIR`, `PCB_MAKER_NO_CACHE`,
  `PCB_MAKER_KICAD_USER_LIBS`, `PCB_MAKER_MEMORY_MB`).
- The verdict's `environment` echoed `PCB_ROUTER_NO_ISLAND_STUBS`, which
  nothing reads since the island stubs went off; it echoes
  `PCB_ROUTER_ISLAND_STUBS` (verdict.rs, docs/agents.md). The hand-over's
  switch list said the same and named two test fixtures the code no
  longer has.
- docs/router.md's limit "Two copper layers, through vias only": inner
  layers are routed; the limit is through vias only.
- docs/handover.md: the leftover "Fill in when it lands: v17" template
  and its fragment, replaced by the v17 result it was waiting for; the
  open problem listing `bus_rows`, `supply_via_room` and
  `replicate_channels` as built and off (removed on 2026-10-16).
- docs/constraints.md documented `bus_rows`, which the placer config
  rejects since 2026-10-16; the paragraph also called `keep_on_pieces`
  opt-in while it is on by default.
- docs/changelog.md: seven sections out of date order (2026-10-16 and
  2026-10-20 among the 2026-10-21 ones) moved, sections verbatim (two
  got the blank line before the next heading they lacked).
- README and `--help`: "KiCad 9 or newer", while the import writes KiCad
  10 boards and nothing ran on KiCad 9: KiCad 10 (checked with
  `kicad-cli version` 10.0.7). native_report.rs asks `kicad-cli version`
  (cached, as the DRC cache does) before the first KiCad check of a
  process and warns once on stderr when it is older than 10.

**The flaky expansion test.**
benchmarks/expansion/test_triage.py's
`test_timeout_kills_descendants_and_keeps_cpu_measurement` failed about
one run in three under load (6 of 15 runs at load 18 here). The
traceback caught: `ProcessLookupError` from reading `/proc/<pid>/stat` of the descendant
after `exists()`: the kill worked, and the process was reaped between
the look and the read (ESRCH). A second race waited behind it: the
0.4 s wall timeout could pass before the parent, under `nice -n 19`
and `prlimit`, had started the descendant and written its pid. The test
now runs `run_process` on a clock of its own that passes the timeout
only once the pid file (renamed into place whole) exists, counts a
descendant gone on either error, and bounds its waits at 300 s where
both processes would sleep 600 s: 20 of 20 runs passed at load about 20.
benchmarks/expansion's tests are now in scripts/check.sh (41 pass).

## Committed on 2026-10-21 (project rules at scale)

The intake (docs/benchmarks.md, "Expansion intake") showed the router
ignoring rules KiCad enforces: track_width on 89 boards (84 on inferred
rules: KiCad's 0.2 mm minimum where the project states none, against our
0.127 neck), clearance from `.kicad_dru` rules (1 014 findings) and from
net classes (465, all `HIGH_V` on dennisss's dacha boards), edge
clearance from the board setup (203) and custom rules (40), hole to hole.
`project_rules.rs` now reads the project as KiCad 10.0.7's DRC does and
`project_rules/dru.rs` reads the custom rules (docs/router.md, "Project
rules").

Binaries: x1274 is main at b306347, x1275 this work on it, x1276 the
commit (rebased on the research removal, 0dff44e). Snapshots x1270-x1273
are the steps on the way (x1270 and x1271 on 0d13e80).

**Census** (`experiments/project-rules/census.py` over the 5 834 intake
boards; `census_of_custom_rules`, an ignored test, runs the lowering over
the 316 with a `.kicad_dru`). Projects: 390 boards have no `.kicad_pro`,
30 one without design rules, 33 one without `min_track_width` (all take
KiCad's 0.2); 842 boards have a pattern that is not a plain wildcard, 133
(399 patterns) a real regular expression (`.*(_TAP|V_POE|POE_RAW).*`);
105 a class without a clearance or width (KiCad 8, Default's). Custom
rules: 316 files, 76 with no rule, 4 040 rules on 240 boards.
Constraints by boards: clearance 193, hole_clearance 142, track_width
140, hole_size 137 (+96 with a max), annular_width 126, edge_clearance
125, hole_to_hole 98, silk 97, text 92, via_diameter 84,
physical_hole_clearance 21, courtyard 10, diff_pair_gap 8, creepage 6,
disallow 5+, length 3, skew 2. Condition atoms by boards: `Type` 128,
`isPlated()` 97, `A.Net != B.Net` 97, `Fabrication_Property` 91,
`Pad_Type` 70, `Layer` 49+15, `NetClass` 36, `Hole_Size_X` 24,
`memberOfFootprint()` 15, `hasNetclass()` 12, `NetName` 10, `Reference`
9, `insideArea()` 8 (1 371 uses), courtyards 6+6, `inDiffPair()` 4;
`(layer outer)` 96, `inner` 92, a named layer 114. Most files are fab
templates (JLCPCB, PCBWay).

What the lowering made of them (rules / boards): clearance applied 4/3,
approximated 3/2, no effect 387/156 (looser than the classes), skipped
106/62 (severity, or an inner layer on two layers), unmodelled 95/44
(pair rules, pad-to-track rules, areas);
track_width applied 4/4, approximated 12/7, no effect 185/139,
unmodelled 1 370/7 (one generated file of `insideArea` necks);
edge_clearance applied or approximated 20/20, no effect 85/84;
hole_clearance approximated 168/78 (JLCPCB's 0.33 PTH to track over the
0.25 setup), no effect 111/80; hole_to_hole approximated 105/51 (0.5
between nets; most projects keep KiCad 8's `hole_near_hole: error`),
skipped 103/40 (warning); hole_size, via_diameter and annular_width
nearly all no effect (the classes' vias are larger). Reported only:
creepage 8/6, diff_pair_gap 163/8, length 30/5, skew 7/5,
physical_clearance 6/3, physical_hole_clearance 40/21, disallow 22/7
(tracks or vias in areas), rules on areas, courtyards, references or
component classes. Silk, text, courtyard, thermal and assertion rules
are `not_routing`.

**What changed.**

- Board setup minimums the project does not state take KiCad's defaults
  (min_track_width 0.2, via diameter 0.5, through hole 0.3, annular ring
  0.1; edge 0.5 and holes 0.25 as before), so the neck-down is 0.2
  there. `rules.kicad_defaults` lists them.
- Net class patterns match as a wildcard or an anchored regular
  expression (regex-lite, a new dependency of pcb-kicad); a net takes
  every class it matches, each value from the first by priority that has
  it, else Default's; a class's missing value is Default's (was KiCad's
  built-in 0.2/0.6/0.3).
- `.kicad_dru` rules lower to per-net clearances, widths (min, max, opt)
  and vias and to the board's edge, hole and hole-to-hole clearances,
  the largest of every rule that can match (KiCad takes the last match;
  the largest is never less). Pair rules between two sets of nets,
  clearances from pads or vias to tracks that match no two tracks,
  rules on what the router cannot evaluate and creepage, length, skew,
  diff pair, physical clearance and disallow rules are reported, not
  kept. Every rule read and what became of it is in `board-router.json`'s
  `rules` (`custom_rules_file`, `custom_rules[]` with rule, constraint,
  status, detail; `unmodelled[]`); the route and layout verdicts' next
  actions name the unmodelled rules when the result is incomplete, and
  `drc-kicad-board` prints them on a `next:` line when KiCad finds errors.
- Plumbing outside the rules: `KiCadBoardRouterConfig::project_rules`
  (a skipped field, board_router/config.rs), `KiCadRulesSummary` flattens
  it (board_router/result.rs, one field and one line in `of`), main.rs's
  file merge copies it, `KiCadRunFacts::unmodelled_rules`.

Two over-approximations were measured and dropped. x1270 lowered a rule
between two sets of nets by raising one set against every net:
ChengminLi's flyback (2.5 mm primary to secondary) went from 1 finding
to 19 unconnected, kisurf's issue24211 (8 mm feedback to switch node)
from 11 findings to 8 unconnected (route mode, 30 s, against x1225).
x1273 still raised every net for JLCPCB's "Pad to Track" (0.2 mm,
`A.Type == 'Pad' && B.Type == 'Track'`): AMLing2's drone carrier went
from 10 to 35 unconnected and tuxkeychain from 23 to 41 (expansion tier
against x1261); routed with that rule removed from the file, AMLing2
had 10 unconnected and no error. Both kinds are reported now; a pad
clearance floor in the lowering would keep the second exactly.

**Route mode on 20 intake boards** (`benchmarks/agent-tasks/rules-tasks.json`,
from `experiments/project-rules/tasks.py`: boards of
drc_failure_boards.csv whose only new findings are clearance,
track_width, copper_edge_clearance or hole_to_hole, with a licence;
the expansion tier's machinery, 30 s, `--jobs 8 --threads 1`, no row
cut). x1274 is main at b306347 (identical to x1244 on the expansion
tier), x1275 this branch rebased on it. New findings (KiCad's errors
beyond the designer's; the strict column matches them by place):

| board | rule behind the findings | x1274 new / strict | unconn | vias | x1275 new / strict | unconn | vias | pass |
| --- | --- | --- | ---: | ---: | --- | ---: | ---: | --- |
| dacha compute r4 | regex class pattern (HIGH_V) | clearance 35 / 37 | 0 | 66 | 0 / 0 | 0 | 73 | no -> yes |
| dacha compute r5 | regex class pattern (HIGH_V) | clearance 35 / 37 | 0 | 68 | 0 / 0 | 0 | 62 | no -> yes |
| flatmaxxing valid_board | KiCad's 0.2 minimum | track_width 125 | 0 | 5 | 0 | 0 | 5 | no -> yes |
| kicad issue25101 | KiCad's 0.2 minimum | track_width 199 | 0 | 20 | 0 | 0 | 20 | no -> yes |
| EEGFrontier | KiCad's 0.2 minimum | track_width 166 | 0 | 21 | 0 | 0 | 20 | no -> yes |
| emonWiFi | .kicad_dru edge 0.3 | edge 10 | 0 | 5 | 0 | 0 | 10 | no -> yes |
| kisurf issue24211 | .kicad_dru on component classes, pair rule | clearance 11 / 17 | 0 | 2 | same | 0 | 2 | unmodelled, named |
| ChengminLi flyback | .kicad_dru on courtyards | clearance 1 | 0 | 10 | same | 0 | 10 | unmodelled, named |
| AQEYE, name-card, meshsat | net class (KiCad's fast path) | clearance 2, 2, 6 | 0, 0, 2 | | same | | | geometry |
| michpro, buganini, facade60 main | board setup edge | edge 1, 4, 3 | 0 | | same | | | Margin graphic, NPTH pads |
| nuclone (Squantor) | board setup hole to hole 0.5 | hole_to_hole 2 | 0 | 21 | same | 0 | 21 | same-net via to pad hole |
| 10VBlocker, HP303B, sonde, pic_programmer, TFRPM01 | (passing both) | 0 | 0 | | 0 | 0 | | yes, identical |

5/20 pass to 11/20 (strict 5 to 11), new errors 602 to 32 (strict 612
to 38), unconnected 2 to 2 (meshsat's both times), vias 489 to 494. The
rows left are not rules the router reads wrongly: kisurf's and
ChengminLi's rules are unmodelled and named; AQEYE, name-card and
meshsat are net class clearances KiCad finds closer than the router's
check (KiCad's fast path names no rule); the edge findings pair tracks
with "Segment on Margin" or "NPTH pad of J1" (KiCad keeps the edge
clearance from Margin graphics and NPTH pads, the router does not:
lowering, for the ladder agent); nuclone's is a via 0.464 mm from a pad
hole of its own net (hole to hole holds within a net in KiCad).

**Expansion tier** (x1275 against build/expansion-x1244.log, 8 x 1, no
row cut): 14/46 pass both (strict 14 both), unconnected 807 to 787, new
errors 291 to 20 (strict 17), vias 4038 to 4022. Five rows changed, all
on projects whose rules changed; the 41 others are identical (routed,
vias, length, KiCad's findings, work):

| board | what changed in its rules | x1244 | x1275 |
| --- | --- | --- | --- |
| tuxkeychain | Power class without clearance takes Default's 0.1 (was 0.2); .kicad_dru trace spacing 0.127 on every net, hole clearance 0.33; "Pad to Track" 0.2 unmodelled | 29/42, 23 unconnected, clearance 43 | 37/42, 5 unconnected, clearance 15 (9 from the unmodelled rule) |
| anthracite-bottom | GNDPWR matches both power and GND: GND's 0.2 by priority (was power's 0.5) | 137/139, 2 unconnected, starved_thermal 1 | 139/139, 0 unconnected, starved_thermal 4 |
| power-manifold backplane | .kicad_dru track width 0.15 floors the neck (was 0.127) | track_width 199 | 0 |
| AMLing2 drone carrier | .kicad_dru via to via 0.5 between nets | hole_to_hole 8 | 0 (10 unconnected both) |
| FUEL4EP CO2 | .kicad_dru PTH to track 0.33, NPTH 0.254, edge 0.3 | hole_clearance 38, starved_thermal 1 | 0 |

`experiments/project-rules/changes.py` predicts these from the files
alone (tuxkeychain, anthracite and the .kicad_dru boards).

**Short tier** (x1275 against x1274, 6 x 4, tasks v6): all 12 rows
identical (routed, vias, length, KiCad's findings, work); no short-tier
project has a .kicad_dru rule, a regular-expression pattern, a class
without a value or a minimum left to KiCad's default. Against
build/short-x1170.log the rows differ only as x1274's do (main's
thermals: vias 1134 to 1125, unconnected 19 both, 7 passes both).

Unit tests (project_rules and project_rules::dru): a file with comments,
units and escapes; a class clearance (case-insensitive); a rule on every
pair; a rule between two classes (unmodelled) and a class against every
other (applied); net names with wildcards and net identity; item types
and layers (pad to track unmodelled, track to track kept, an inner layer
on two layers skipped); conditions on courtyards, creepage and lengths
reported; widths, vias, holes, edge and hole-to-hole (with KiCad's
warning default and `hole_near_hole`); rules the classes keep; not
routing rules; three-valued, commutative evaluation; KiCad's defaults
for a project without minimums; regular-expression patterns and
composite classes; a project end to end (report, statuses, unmodelled).
`bash scripts/check.sh` and the clippy gate pass.

Logs: build/rules-x1225.log, -x1270, -x1271, -x1292, -x1273, -x1274,
-x1275, -x1276 (the commit: all 20 rows as x1275); build/expansion-x1271, -x1273,
-x1274, -x1275; build/short-x1274,
-x1275.

## Committed on 2026-10-21 (runtime calibration)

The review's finding: work budgets were seconds of one machine.
`EXPANSIONS_PER_SECOND` (3.15e6) and the placer's `WORK_PER_SECOND`
(150e6) were constants, the layout's wall guard the const 1.1, so any
machine slower than a 5950X core by more than a tenth (the macOS
snapshot's, CI runners, laptops, this one above load 45) had every
non-trivial layout cut (exit 3). Snapshots x1300 (b306347 plus this
change: the judges) and x1301 (rebased on the research removal). Logs and scripts in `build/calib2` (main tree).

**The rates are measured at the start** (`work_rate.rs`; docs/router.md,
"The machine's rates"). `route-kicad-board`, `layout-kicad-board` and
`place-kicad-board` first run a fixed search on a canned lattice (40 x 30
mm, two layers at 0.1 mm, 24 crossing nets, keepouts: 240 k nodes, about
50 MB of maps, about 600 k expansions a round) on one thread, five
rounds, and a fixed placement (40 parts, 20 global iterations, 15 M
units) as three copies at once on the process's threads (the placement
race's first batch), three rounds: 1 to 2 s in all, once per process,
before the run's clock. The fastest round's rate over the same workload's
on the reference (6.9 M expansions and 155 M units a second, measured
here at load 12 with `calibrate-work-rate`) scales the reference's
rates, now `REFERENCE_EXPANSIONS_PER_SECOND` and
`REFERENCE_WORK_PER_SECOND`; `expansions_per_second()` and
`work_per_second()` (set once, an atomic each) replace the constants at
every use. The pour analyses' nanoseconds and KiCad's check model scale
with the search's rate (`reference_time_scale`, exactly 1 at the
reference's rate). The placer's rate takes three copies at once because
the race's clock counts its seeds as the reference's eight threads run
them: on one CPU 0xcb-static's three seeds took 44 s of wall for 16 s of
counted work. The work counters are unchanged: a run is byte-identical
for given rates. Pinning: `PCB_WORK_RATE=<expansions/s>[,<placer
units/s>]` (a search rate alone scales the placer's alike; a bad value is
exit 2), else the router settings `work_rate`, `placer_work_rate`. The
first line of a run is `work rate: ...`; the verdict has `work_rate`,
`placer_work_rate`, `work_rate_source` (`measured`, `PCB_WORK_RATE`,
`config`, `reference`), `work_rate_measured`,
`placer_work_rate_measured` and `work_wall_ratio` (work over wall; with
several search threads the summed expansions can exceed the wall clock);
`PCB_WORK_RATE` joins the echoed environment; `layout: done` prints the
ratio and the rates. `pcb-maker calibrate-work-rate [rounds]` prints the
measurement alone.

**The wall guard** is the layout config's `wall_guard` (default 2, was the
const 1.1; below 1 is an input error): with measured rates it is a load
margin, not a budget. run.py's timeout without `--timeout`: the budget
times the guard plus 300 s, at least 1800 s (the short tier 800 to 1800 s,
the full 1800 to 3300 s). run.py pins the reference's rates for every
board (`--work-rate`, default `3.15e6,150e6`; `measured` lets each binary
measure), copies the verdict's rate fields into the rows, and
summary.json has `work_rate` and the medians of the measured rates and
of work over wall. The fingerprint and walkthrough tests pin the rates.

**Spread of the measurement.** The canned lattice is beyond the caches, so
the rate follows other jobs' memory traffic and bursts on the core or its
sibling: unpinned at load 12 to 13, 0.976, 0.988 and 0.998 of the
reference's search rate and 1.01 to 1.06 of the placer's; pinned to one
core at load 16 to 18, 3.3 to 5.7 M canned expansions a second on eight
cores (0.48 to 0.83), and on one core one process after another 5.4, 5.0,
5.8 and 2.96 M; during the short tier's 6 x 4 (load 7 to 14 plus the
tier) the binaries measured 0.78 to 0.96 of the reference (median 2.77 M,
placer 129 M). The fastest of the rounds counts, not the median, for
that reason; a run that must be repeated pins its rates.

**Judge 1, the review's experiment** (0xcb-static, total_seconds 40,
move 16, polish 2, `taskset -c 0`, the quick tier's prepared board).
x1292: exit 3, cut (deadline, placement, layout), 20 s of work at 50 s
of wall: the placement took 44 s of wall for 16 s of work, the race, the
moves and the neck repair were skipped, KiCad 32 unconnected. x1300, two
runs at load 12 to 13: not cut, exit 1 (KiCad 0 unconnected, the
designer's solder mask bridges), measured 2.88 M (0.91 of the
reference; placer 0.35 on one CPU) work 32 s at 32 s of wall (1.00), and
measured 1.60 M (0.51: a burst during the measurement) work 47 s at 31 s
(1.52; the check charge scales too, 7 s instead of 3.6). Two runs of the
development binary at load 20: work 48 s at 40 and 34 s of wall, not cut.

**Judge 2, the rate pinned at the reference** (short tier, `--jobs 6
--threads 4` under the lock, x1300 against x1244, the coordinator's
before side `build/short-x1244q.log`): every row identical apart from
timing and the new fields (same 12, unconnected 19, passes 7, strict 7,
vias 1125, 21142 mm, work 2517 s), every result board byte-identical
(md5). Against `build/short-x1170.log`: same 12, unconnected 19, passes
7; vias 1134 to 1125 and 21228 to 21142 mm from the repairs since. Wall
356 s, no row cut.

**Judge 3, three short-tier boards at the measured rate** (`--work-rate
measured --jobs 3 --threads 4` under the lock, x1300, load 8 to 9): the
binaries measured 3.12, 3.06 and 2.94 M expansions a second (0.989,
0.970, 0.934 of the reference) and 151, 147, 138 M placer units (1.006,
0.981, 0.923). urchin, spell_tome and USB_Cable_Tester routed as with the
pinned rate (68/68, 18/18, 54/54; the same vias and length, all pass and
pass strict), work 40.9 / 31.0 / 132.8 s against 40.5 / 30.1 / 124.0 s
pinned, wall 31 / 41 / 87 s. At load 20 the measurement is far lower:
unpinned 2.84 to 2.86 M canned (0.41 of the reference; 4.55 M, 0.66, a
moment later), pinned to CPU 0 0.47 to 0.52, other cores 0.48 to 0.83.
The canned search's best rounds below load 10 (the median binary,
`build/calib2/ref/idle.out`): 7.1 to 7.2 M pinned (1.03 to 1.04 of the
reference constant), unpinned 5.6 to 7.2 M.

Unit tests: the rate's parsing and its precedence (variable, config,
measurement, reference), a search rate alone scaling the placer's, the
canned workloads' work fixed, the log line; the guard's arithmetic and
its config default; CLI tests of a measured, a pinned and a configured
rate and a bad one (exit 2); run.py's pinning, timeout and row fields.
`bash scripts/check.sh` passes.

Outside this package's files (coordination): the rename of the constant
at its uses in pcb-router (driver.rs, fixed.rs, negotiate.rs, profile.rs,
vias.rs, tests.rs: `expansions_per_second()`, and the pour work's scale in
driver.rs's `pour_work_seconds`), two lines in pcb-kicad's lib.rs (the
module and its exports), two router settings in board_router/config.rs,
route.rs's uses, and run.py (the pinning, the timeout, `verdict_extras`
and the summary).

## Committed on 2026-10-21 (quality: pcb-kicad research removed, lib.rs split)

The last package of the quality pass: pcb-kicad's research code removed
and what is left of lib.rs split into modules, with placements and routes
unchanged. Branch `quality-kicad-research`, five commits on b306347
(written on 0d13e80, rebased four times); snapshots x1250 (the four steps
on 0d13e80), x1251 (on 1819890), x1252 (on 9da1213), x1253 (the five on
6868f9a), x1254 (on b306347). **The last commit holding the removed code
is b306347**; check it out to run or read any of it. Rust goes from
120 258 to 82 959 lines (on b306347).

**How the reachability was found.** pcb-kicad built as a binary rooted at
lib.rs, with src/main.rs, agent_cli.rs and freerouting_backend.rs as its
modules and every `pub` made `pub(crate)`: rustc's dead-code pass then
reports every item no command reaches (board_layout, board_router,
board_placer, verdict and the agent commands included, since main reaches
them). The removal was driven by that report and the compiler's errors
(impls of removed types, use lists), every step checked by the build.

| Step | What | Rust lines |
| --- | --- | ---: |
| 1 | nine research commands of src/main.rs and the island-bridge stage of route-kicad-board-freerouting; eight experiment scripts that only drove them | -469 (Python -1 012) |
| 2 | pcb-kicad's research modules and the research half of lib.rs (17 367 to 4 652 lines), their tests | -29 270 |
| 3 | the crates layout-trace-model, pcb-core, pcb-engine, pcb-grid-router; four benchmarks/imported fixtures | -7 731 |
| 4 | lib.rs split into eleven modules as moves (lib.rs 145 lines) | +165 (headers, rustfmt reflows) |
| 5 | pcb-kicad clippy-clean for the CI gate | +6 |

**Commands removed** (9): route-kicad-board-adaptive,
route-kicad-board-sequential, resume-kicad-board-sequential,
discover-kicad-vias, refine-kicad-vias (only experiments/whole-board drove
them), inspect-kicad-routing-cut, inspect-kicad-terminal-access,
inspect-kicad-route-obstructions (they inspect the research grid router)
and bridge-kicad-open-connections with route-kicad-board-freerouting's
optional `island_bridges` stage: no benchmark configuration sets it
(benchmarks/corpus/freerouting.json does not, and
benchmarks/github/freerouting.py runs the Freerouting jar itself, never
this command), and keeping it would have kept the research grid router it
routed with. The scripts removed with them: completion_stop_controls,
lazy_forecast_controls, route_placement_case, route_one_sweep (and its
test), routing_interference_order, run_via_discovery,
inspect_terminal_access and, in step 2 with island_bridge.rs that embedded
it, inspect_native_islands (and its test). area_probe.py still names
route-kicad-board-adaptive but already depended on `place`, removed on
2026-10-20; inspect_routing_cut.py stays because audit_partial_ripup.py
(a helper the Freerouting backend bundles) imports its renderer.

**Commands kept**: the agent commands; verify-kicad-rung (the
verification path board_router calls, `verify_materialized_rung`),
normalize-kicad-footprint-links (benchmarks/real/*/prepare.sh),
shrink-kicad-board (benchmarks/corpus/run.py), inspect-kicad-board
(pcbench/manifest.py, tscircuit, the Freerouting helpers) and
inspect-kicad-connections (small: board_stats), render-kicad-layers
(small: layer_render), score-congestion, score-congestion-sample,
export-congestion-sample (`congestion_samples::race_core_config` is the
race's), dump-pour-map, repair-kicad-necks, bench-search and
route-kicad-board-freerouting (benchmarks/corpus/run.py) with its external
stage and final-selection.json (a finished, preserved import with only
native opens is still the selection).

**Removed in pcb-kicad** (step 2): the modules connection_insertion,
semantic_template, sequential_router, joint_relaxation, adaptive_routing,
compound_recovery, sequential_resume, sequential_order_search,
connection_order, restoration_recovery, partial_ripup, island_bridge,
pad_pair, via_discovery, coupled_vias, routing_cut, terminal_access,
route_obstructions, route_failure, routing_demand (with its dense
reference), routing_grid, routing_cost, grid_alignment,
pad_layer_contacts and the route_union and rule_area test modules; in
lib.rs the research routing model (KiCadGridRouteConfig,
KiCadRoutingModel, the obstacle grid, grid-path materialisation), via
actions, route relaxation and shortening, reference fields, route
candidates and their import, ground zones, yielding diagnosis, the
semantic ladder's materialisation and the route-only verification. Kept:
`KiCadConnectionRoutingRules` (its resolve, validate and apply went),
`pad_clearance`, roundrect_pad, electrical_terminals' `plated_pad` and
`contact_layers` (layer_render), shrink.rs, `verify_materialized_rung`.
Beyond deletions: the route-graph normalisation no longer counts contacts
and inserted points (only research read them) and `LoweredPad` loses its
unread `layers`. pcb-kicad loses 157 tests (367 and one ignored to 210
on 0d13e80; 225 remain on 9da1213), all of removed code. route_obstructions' roundrect tests
went with it: they test its obstruction report, and roundrect_pad.rs's
own tests cover the corner gap and the rejected shapes. The pad and outline
geometry tests stay, on a test-only `geometry_queries` module (the
`contains` and `intersects_segment` queries the research model used, as
their oracle).

**lib.rs split** (step 4): kicad_sexpr (383 lines: Expr, parser, writer,
forms), kicad_nets (292), copper_emit (241), geometry (67),
copper_geometry (808), route_graph (289), board_stats (1 014),
verification (678), fs_util (90), layer_render (472), footprint_links
(336); lib.rs keeps the module list and re-exports (145, a crate doc). A
line multiset check of old against new finds only the module headers,
`use` lines and two rustfmt reflows. Each module takes the crate's names
with `use super::*` and lib.rs each module's with a glob, so paths inside
the crate stay; items, fields and methods another module uses are
`pub(crate)`, the rest private (all made private, then the ones the
compiler asked for restored). Two line changes:
`kicad_verifier_implementation_sha256` also hashes kicad_sexpr.rs,
kicad_nets.rs, verification.rs and fs_util.rs (lib.rs held them, and the
verification cache must still invalidate when they change: its keys change
once), and native_report's tests name `crate::verification::`.

**Clippy** (step 5): what the removal left of the CI gate's lints
(`cargo clippy --release -p pcb-router -p pcb-kicad -p pcb-placer -- -D
warnings`, 20 lints at 6868f9a, 9 of them in removed code) fixed in the
production code: let chains for two collapsible ifs (native_report),
`as_chunks` (congestion_rank), a function for a closure (labels), `if let`
for a single match and `split_whitespace` without `trim_start`
(netlist_import), type aliases `PinNet`, `NetChange` and `Choice`
(netlist_import, pin_swap), the positive-size check by `partial_cmp` (NaN
still rejected, net_classes), an iterator in shrink; with `--all-targets`
also two test asserts. The gate passes, pcb-kicad with `--all-targets`
too.

Not built any more: experiments/engine-playback (a standalone crate on
pcb-core and pcb-engine), like experiments/placement-exploration since
2026-10-20.

**Gates.** After every step `scripts/check.sh` (the library tests, the CLI
exit-code, walkthrough and native-verification suites, the Python tests)
and `cargo clippy --release --workspace`: no new warnings (22 at 0d13e80,
13 after step 2, the 9 gone in removed code). place-kicad-board on the
short tier's prepared x1170 sources (each board's layout.json, 4 threads,
`PCB_PLACER_WORK_SECONDS=120`): spell_tome_bottom, USB_Cable_Tester,
urchin, eternal-keypad, OpenFC and Blue_Line byte-identical between x1225
(main's) and x1250 (`build/qe-place`, `build/qe-place2`).
Short tier (tasks v6, 6 jobs x 4 threads, under the lock): x1250 (the
four steps on 0d13e80) and x1251 (rebased on 1819890) against
build/short-x1170.log: 11 of 12 rows identical but CPU, wall, load and
memory, and MIDAS-MK1.1's work 285.1 to 287.5 s, which is main's MIDAS
change (x1223 has it); against main's own x1223 and x1261 every row is
identical, and every routed board byte-identical to x1230 (the writer's
footprint-graphics order of 3d28004 makes x1170's and x1223's files
differ from all later ones) and to x1261: 7/12, unconnected 19, new errors
2, vias 1134, length 21228 mm, work 2504 s, cut 0, first-route open
terminals 26. Schematic tier (x1250) against build/sch-x1161.log: the rows
differ in `work_seconds` and `kicad_check_work_seconds` only (x1161
predates the check work accounting), and TelemetryOnboard 108/108 to
107/108, as the pcb-kicad quality pass found (x1170 gives it too); against
x1194, that pass's last schematic run, every row identical but the DRC
line's report path and cache hit, every board the same lines in another
order (3d28004's footprint graphics); 9/12, unconnected 5, vias 3190.
Expansion tier (8 jobs x 1 thread): x1250 against
build/expansion-x1231.log: all 28 rows uncut in both identical (13 cut in
x1231, 18 in x1250 at that load). x1252 against build/expansion-x1261.log
(main's reference since the bounded route-mode budgets): no row cut in
either; routed, vias, length, KiCad's counts, work, open, outcome and pass
identical on all 46 rows and every routed board byte-identical; the rows
differ only in what main's 9da1213 and 6868f9a add (`verdict`
router_complete_unverified and exit 1 for the 13 complete rows, as the
tier skips KiCad in route mode; the strict and repair fields). x1253
(the five steps on 6868f9a), short tier: every routed board byte-identical
to x1251's, the same objectives. **On the final base** (b306347, main's
thermals package; x1254): the short tier against a run of main's x1244
(`build/short-x1244q.log`) and the expansion tier against
build/expansion-x1244.log, every row identical but CPU, wall, load and
memory and every routed board byte-identical (short: 7/12, unconnected 19,
new errors 2, vias 1125, length 21142 mm, work 2517 s; expansion: 15/46,
unconnected 807, vias 4038, work 1279 s, cut 0); the outcomes as in
build/short-x1170.log (7/12, unconnected 19), whose copper the thermals
package changed. scripts/check.sh, the clippy gate and rustfmt pass on
b306347.

## Committed on 2026-10-21 (starved thermals at scale)

starved_thermal was the second largest finding type the router adds
beyond the designer's board in the 5 834-board intake: 120 boards, 103
blocked by it alone (benchmarks/expansion/data/analysis/drc_failure_boards.csv).
Snapshots x1240 (first candidate), x1241 (the thermal changes, on
0d13e80), x1242 (island stubs off) and x1243 (both commits rebased on
9da1213).
Working files, scripts and logs: `build/thermal-scale` (`tools/`:
`static_census.py`, `rows2.py`, `findings.py`, `table.py`, `cmp.py`,
`mktasks.py`; `starved-tasks.json`).

**Census.** `census.py` on the intake's routed boards of all 103 (the
x1185 model on their final copper, KiCad's side refilled): 252 starved pad
rows, 69 predicted, 145 missed, 38 with no model row, 9 false alarms;
very unlike the 44-board census (162 / 7 / 2 / 26). The same census on
the stripped boards (no tracks) split the findings into pads KiCad gives
fewer than two spokes before anything is routed (the designer's own
tracks gave the second) and pads the signals took a spoke from. Per new
finding (by pad and layer, the designer's own findings at the same pad
left out), route mode on the 40 of the 103 with a usable licence, x1185:

| Cause | Findings | Empty board < 2 spokes | Fix |
| --- | ---: | ---: | --- |
| model sees no spoke at all (no finding to it), KiCad one | 22 | 13 | spokeless pads get a track |
| predicted, not repaired: the signals took the spoke (a pocket around the pad, a track across the strip) | 12 | 0 | `free_lost_spokes` |
| predicted, not repaired: manual spoke cut at a junction of the net's own track, or none straight | 7 | 7 | manual spokes verified, no own copper on the way |
| model optimistic: another net's zone of a higher priority (USSTRocketry, smkent), meeting spokes, ends against tracks | 14 | 7 | zone clearance in the fill probe (part) |
| model counts a track KiCad does not (segment cut at a junction, end in a margin) | 8 | 3 | as above (part) |
| no model row: an exposed pad set to solid made its thermal vias solid | 5 | 5 | solid applies to the terminal's pad |
| no model row: a small solid zone of the net made the layer solid | 2 | 0 | solid per containing zone |
| no model row: per-label isolated connection (74th's J3.S1) | 2 | 0 | lowering (not done, below) |
| census could not match the finding to a pad (pads of one number in several zones) | 13 | | |

85 findings (x1185; x1232 identical but for rows the wall guard cut).
After, on x1241's routes: 44, of them 11 predicted and not repaired, 11
model-zero (tag-designs' island-only U5.6/U5.7, robotique's buttons),
8 optimistic, 5 track miscounts, 2 J3.S1, 7 unmatched. The census on
the intake's routes with x1241's model: 99 predicted, 151 missed, 2 no
row, 18 false alarms (the newly modelled vias inside solid exposed pads on
the near side, OpenAir's U6.17 F.Cu: KiCad gives them four spokes, the
model one; a false alarm only costs a repair attempt).

**Changes** (crates/pcb-router; docs/router.md, "Starved thermals at
scale"):

1. `kicad_spokes`: a label's solid connection is its terminal pad's; pads
   merged into it keep thermal records and rings (`thermal_ring_pads`),
   and `add_inner_spokes` reaches them inside any pad of their terminal on
   the layer. A pad is solid where the zones containing it are, not where
   any zone of the net on the layer is.
2. `Board::excluded_clearance`, `FillProbe`: an excluded region that is
   another net's zone keeps the zones' clearance plus half the minimum
   width (waived at the net's track ends). Tried in the pour masks too and
   left out there: OpenFC (short tier, layout) lost two connections in its
   first route, robotique's ladder moved to the tracks rung.
3. `add_manual_spokes_of`: candidates sorted by length, each kept only
   when the model then counts it (the first four tried), none passing a
   node of the net's own copper.
4. `Config::spokeless_tracks` (on; `PCB_ROUTER_NO_SPOKELESS_TRACKS`):
   one track into kept fill for a pad the model sees without a spoke
   (`Router::spokeless`, `SpokeRecord::drawn`).
5. `free_lost_spokes` (`Config::lost_spoke_repair`, on;
   `PCB_ROUTER_NO_LOST_SPOKES`) after `free_thermal_spokes`; and
   `free_thermal_spokes` keeps a move only when the pour nets' starved
   spokes go down (Squantor's alubox: 96 vias at x1232 when its budget
   was left, 48).

Unit tests, one per cause:
`a_solid_pads_merged_vias_keep_their_thermal_spokes`,
`another_nets_zone_of_a_higher_priority_keeps_its_clearance`,
`a_pad_without_a_spoke_in_the_model_gets_a_track_into_the_fill`,
`a_net_crossing_a_starved_pads_spoke_is_routed_around_it`.

**Gates.** Both sides of every route-mode pair below (x1232 and x1241,
x1225 and x1242) predate main's work bound for route mode (9a2cf41).

- Census, 44 boards (fill3 KiCad cache): 162 / 7 / 26 / 2 for x1241 as
  for x1184, every verdict line the same (`build/thermal-scale/census/c44-x1241.txt`).
- Route mode, the 40 licensed boards of the 103 (`starved-tasks.json`,
  expansion task format, fetched by run.py, 8 x 1, 30 s), x1232 against
  x1241: new starved findings 69 to 36, boards with one 39 to 19, passing
  1 to 19, other new errors 0 to 0, KiCad unconnected 4 to 2 (Apex 3 to 1
  and OpenAir 1 to 0, mdiph 0 to 1), vias 1542 to 1532, length 47742 to
  47343 mm, work 958.6 to 985.4 s. The first 20 boards: starved 19 to 8,
  passing 1 to 12, unconnected 0 to 1 (mdiph), vias 692 to 653.
  Worse rows: embeddedos 3 to 4 and tag-designs fc8db 1 to 2 (a pad the
  model sees without a spoke and no straight track reaches, on a changed
  route); unchanged: ray4477's vias on In1.Cu (no pad holds them there).
- Expansion tier (8 x 1) x1241 against `build/expansion-x1211.log`:
  every one of the 31 rows neither run cut has KiCad's findings and
  routed count unchanged; 21 of them changed copper (manual and spokeless
  tracks, moved nets): vias 1141 to 1139, length 36424 to 36267 mm, work
  1366.1 to 1369.6 s. Passes 14 and 14; 15 rows cut (13 before; load
  about 23), their unconnected 456 to 508 is theirs.
- Short tier (tasks v6, 6 x 4 under the lock) x1241 against
  `build/short-x1170.log`: 12 same, none better or worse; unconnected 19
  and 19, new errors 2 and 2, passes 7 and 7, first-route open terminals
  26 and 26, vias 1134 to 1125, length 21228 to 21142 mm, work 2501 to
  2517 s, none cut. Four rows identical (USB_Cable_Tester, eternal-keypad,
  urchin, Blue_Line); MIDAS the same copper with 2.4 s more work (the
  spoke analyses); the other seven changed copper by the repairs, summed
  over each layout's routes: nets moved off lost spokes on 0xCB (4),
  USB_Keypad (5), WLED_Matrix (17), reCamera (20) and spell_tome (14);
  manual spokes kept after the check on CRImier (5 lines to 9), WLED (48
  to 31), reCamera (53 to 181); OpenFC's `free_thermal_spokes` kept 5
  moves where it made 49 (159 vias to 146). The candidate x1240 (the zone
  clearance in the pour masks too) lost OpenFC (82/82 to 80/82, its first
  route 1 open terminal to 2); x1241 leaves the masks alone.

**Island stubs off** (Florian's decision; snapshot x1242, x1241 with
`Config::island_stubs` false; `PCB_ROUTER_ISLAND_STUBS=1` turns them on,
`PCB_ROUTER_NO_ISLAND_STUBS` is gone; the island vias stay on). A stub
changes no connection, it only silences KiCad's isolated-island finding.
What it costs:

- Short tier: nothing. No layout of x1241's short tier added a stub in any
  attempt, so the switch reads the same code path.
- Full corpus: v17 (x1170) has 168 island-stub lines on 29 boards, 9 of
  them passing. Route mode on those 9 placements
  (`--route-from build/github-layout-v17`, tasks v7, 8 x 1,
  `PCB_ROUTER_NO_WALL_GUARD=1`): x1225 passes 9, x1242 8, x1242 with
  stubs on 9. The one lost: mackerel-30-proto (2 starved thermals beyond the
  designer's, four in KiCad's report, all "spokes connected to isolated
  island" of through-hole pads on In1.Cu and In2.Cu; x1225 added 3 stubs
  there). Island vias and the thermal fixes recover none of
  its two; they do carry the other eight, whose x1225 routes had stubs in
  some attempt (homebrew_486 3, Castor, k30, SNSP 1 each) and pass without.
  The 20 failing boards cannot lose a pass.
- Expansion tier: x1241 added a stub on 9 of its 46 rows (in some
  attempt). Those 9 again with x1242 (8 x 1 under the lock): the 7 rows
  neither run cut have KiCad's findings unchanged, LC_PAMV4b and flexypin
  still pass; ZSWatch (cut by the deadline in both runs, the same rung)
  loses its pass: its winning attempt's one stub, on X602.1, leaves "3
  spokes connected to isolated island" on F.Cu. Passes 14 to 13.
- Total: one pass lost in the short and expansion tiers (ZSWatch), one of
  the nine v17 passes (mackerel-30-proto); neither island vias nor this
  package's repairs recover them.
- The count reaches the result: `RoutingResult::island_stubs` (stubs
  this router's finishes added; pcb-topo fills 0), with a unit test. For
  pcb-kicad (verdict.rs, the board router's result): carry it from the
  winning attempt's `RoutingResult` into `KiCadBoardRouterResult` and
  board-router.json, read it in `KiCadRepairs::of` as
  `Some(result.island_stubs)`, and list `PCB_ROUTER_ISLAND_STUBS` in
  `RESULT_ENVIRONMENT` in place of `PCB_ROUTER_NO_ISLAND_STUBS`.

**Not done (pcb-kicad's lowering, board_router/).** The lowering lists
zone connections per pad label (`solid_pads`, `isolated_pads`); a label
whose SMD pads have `zone_connect 0` and whose through-hole pads inherit
the zone's (74th's J3.S1, a USB-C shield) makes all its pads isolated, so
the model has no row for the thermal ones. The fix: record the
connection per pad, as obstacle indices beside the labels (for example
`Board::isolated_pad_obstacles` / `solid_pad_obstacles`, filled where
`isolated_pads.push(label)` is now, from the pad's own `zone_connect`
else its footprint's), and let `kicad_spokes`, `thermal_ring_pads` and
`prepare_net` test the obstacle (the router side is a few lines).

## Committed on 2026-10-21 (harness validity)

An independent review found the benchmark harness reproducible but weak on
validity. Seven fixes on branch `harness-validity`, one commit each; no
Rust change but one test file.

1. **The lock is run.py's.** A tier run, or a run of a whole task list,
   takes `build/quick-tier.lock` itself (an exclusive flock, a blocking
   wait like flock(1)'s, so that it waits its turn beside the older
   `flock ... run.py` habit, which still works: a caller that holds the
   lock lets the run through) and prints who holds it (pid and command
   line from /proc/locks, again when the holder changes). `--no-lock`
   skips it. A first version polled with LOCK_NB and lost every release
   to flock(1)'s blocking waiters.
2. **The task lists are committed**: `benchmarks/agent-tasks/tasks/v6` and
   `v7` (170 KB each); every tier defaults to `tasks/v7` (v6 and v7 differ
   on six panels, none in the quick or short tier). Finding: tasks.json is
   the same file in github-tasks-v2 to v7 (sha256 c8f95caf5089), so
   `tasks_sha256` never told the lists apart; rows now also record
   `task_sha256` (the entry with its constraints' digest) and
   `board_sha256`, and compare.py marks a board INPUT when either differs.
3. **Objectives against the designer**: compare.py and run.py's last line
   give per-board medians (vias and length over the designer's, KiCad's
   unconnected items over the connections, the ground pours' cut by other
   tracks where rows carry scores) beside the raw sums;
   `designer-numbers.json` holds the designer's numbers (score-kicad-board,
   15 s for the 109 boards); `run.py --quality` writes quality.py's
   measures of both boards into each row (0.1 s a board), `compare.py
   --score` scores older runs. v16 against v17 (103 boards with the same
   input): vias x0.70 -> x0.72 of the designer's, length x1.03 -> x1.03,
   open 0.009 -> 0.009 of the connections (mean 0.164 -> 0.113).
4. **The designer tier** (`--tier designer`, `designer.json`): every
   corpus board routed on the designer's placement, stripped as the layout
   tier strips it, its own rules, 1500 s of work; `compare.py --gap`
   gives per board the placement gap (layout open less designer-route
   open) and the routing gap (designer-route open), with corpus totals:
   the deciding experiment for placement versus routing work
   (benchmarks/agent-tasks/README.md, "The designer tier"). Six boards
   proved the path (below); the full tier (about 5 hours) is not run yet.
5. **Licences** (benchmarks/github/licences.py, README "Licences"):
   harvest.py gates `fetch` as harvest_schematics.py does
   (`--allow-unlicensed` for private benchmarking), every board carries a
   licence class, and training data (`fetch --training`, the expansion
   harvest's `nn_license_eligible`) is permissive only. The corpus: 109
   boards, 42 without an SPDX licence (24 of them with a licence file
   granting one), 32 permissive, 12 weak-copyleft, 27 copyleft, 16
   unclassified, 4 restricted, 18 unlicensed (private benchmarking only;
   seven in the quick tier, MIDAS-MK1.1 in the short tier). The congestion
   model's training manifest (700 boards) is 205 permissive by SPDX.
6. **CI** (`.github/workflows/check.yml`): check.sh without the
   walkthrough, a lint job (`cargo fmt --all --check`, clean; `cargo clippy
   --release -p pcb-router -p pcb-kicad -p pcb-placer -- -D warnings`, red:
   20 lints left in pcb-kicad, outside the modules made clippy-clean on
   2026-10-20), the walkthrough where kicad-cli is installed (else a
   notice). `tests/router_fingerprint.rs`: route mode on the fence's
   2-layer board, 2 s of ladder work and 6 iterations, identical
   `PCB_ROUTER_FINGERPRINT` lines at 1 and 4 threads (about 15 s a run,
   no kicad-cli). act is not installed: the workflow was dry-parsed.
7. **`scripts/prune.sh`** lists (and with `--yes` removes) merged, idle,
   clean worktrees and snapshots no committed file names. Listed on
   2026-10-21, nothing removed: no worktree (every merged one was active
   within the week; the first listing ran `git status` in each and so
   refreshed their indexes), 102 snapshots (4 GB of 18).

**Checks.** `bash scripts/check.sh` passes (the fingerprint test included);
the Python tests of benchmarks/agent-tasks, benchmarks/github and
benchmarks/expansion pass (the expansion's
`test_timeout_kills_descendants_and_keeps_cpu_measurement` fails about
one run in three under load, on main as well). The short tier through the
new in-script lock and the committed v7 list
(`build/short-x1224-harness.log`, x1224, 4 x 2) against
`build/short-x1170.log` (v6): "12 boards in both: better 0, same 12,
worse 0", every row's routed count, KiCad findings, vias, length,
first-route open terminals and prepared constraints identical (work
2501 -> 2504 s, MIDAS-MK1.1's +2.4 s as on x1223). It waited 3037 s for the
lock behind the designer run and another agent's run, naming each.

**The designer tier on six boards** (`build/designer6-x1225.log`, x1225,
4 x 2, 2816 s wall), joined with v17 (x1170) by `compare.py --gap`:

| Board | Connections | Layout open (v17) | Designer-route open | Placement gap | Routing gap |
| --- | ---: | ---: | ---: | ---: | ---: |
| ecad-viewer video | 371 | 201 | 0 | 201 | 0 |
| OpenESC-30x30 4in1 | 152 | 111 | 0 | 111 | 0 |
| nonSNES SNSP-CPU-01 | 363 | 136 | 27 | 109 | 27 |
| DSP-ADAU1452 | 190 | 68 | 3 | 65 | 3 |
| OnBoard woagboard | 91 | 8 | 1 | 7 | 1 |
| Business-Cards Batch_1 | 204 | 415 | 453 | -38 | 453 |

Totals: layout open 939, designer-route open 484; placement gap +455
(worse on 5 boards, better on 1), routing gap 484 (on 4 boards): 48 %
placement, 52 % routing, and the routing share is Batch_1 alone (453 of
484; KiCad counts 453 unconnected items on a panel of 204 connections, the
router 102 open). Without Batch_1 the five boards are 94 % placement gap
(493 of 524 open), so on these boards placement is the work; whether the
corpus agrees is the full tier's question (about 5 hours). No row was cut;
x1225 predates the ladder's work bound (4in1 used 2427 s of work against
the 1500 s budget), so the full tier should run on a binary with it.
Both comparisons are across binaries (x1170 layout, x1225 route): for
a decision, run the layout and the designer tier on one binary.

## Committed on 2026-10-21 (verdict integrity)

Three defects an independent review found in the verdict
(docs/agents.md, "Read the verdict"), fixed, and one measurement. Binary
x1290 (on 0d13e80).

**Exit 0 without KiCad's judgement.** `native_report.rs` deletes a
report before each check; a kicad-cli failure or timeout inside the
neck repair (`necks::repair_with`, its trial checks and its restore of
the unrepaired board) was swallowed into `necks.json`'s note and the
repair returned `kept: false`, so the router did not check again, the
result directory held no `drc.json`, and `route_facts` read that as
"KiCad did not check": the router's count decided, `complete`, exit 0
on a board the router calls complete, with the next action blaming
`PCB_SKIP_NATIVE`. Reproduced on WLED_Matrix's v17 placement in route
mode with a kicad-cli wrapper that fails every DRC after the repair's
`--save-board` (and `PCB_MAKER_NO_CACHE=1`: with the DRC cache on, the
restore's check of the unrepaired board is a hit and hides it): no
`drc.json`, `board-router.json` with `native` set, the verdict
`kicad_unconnected_items: null`, "(router's count; KiCad did not
check)" (exit 1 there only because the router left 5 connections open).
Fixed three ways:

- The neck repair keeps the result directory's board, `drc.json`,
  `erc.json`, `verification.json` and preview before it starts and puts
  them back when a check fails (`necks.json`'s note says so; `kept`
  false, `tracks` 0): the directory always holds the report of the board
  it holds. Same reproduction: `drc.json` there, the verdict KiCad's
  (12 unconnected items, `kicad_checked: true`).
- `route_facts` returns an error when KiCad checked the board (`native`
  set) but `drc.json` is missing or unreadable: an internal error, exit
  4, with the reason, never a verdict on the router's count.
- Without KiCad's check (`PCB_SKIP_NATIVE`, `skip_native_verification`)
  a route is never `complete`: the new status
  `router_complete_unverified` (exit 1) when the router's count finds
  nothing open, and `kicad_checked: false` in the verdict.

**The pass rule's baseline.** The verdict (`split_findings`) and run.py
(`native`) subtracted the reference's error count per type anywhere on
the board, so a new error of a type the designer also has was hidden,
and nine v17 rows had designer counts at KiCad's cap (199, 499 for
clearance), which are floors. New, the strict rule
(`split_findings_strict`, run.py's `strict_findings`): a finding is the
reference's when it has the same type and either the same items (every
uuid, the same set) still the same distances apart (a part's own
finding goes with the part: TelemetryOnboard's 72 matches are U1, U2,
J1 and J2's own pad clearances and mask bridges, placed elsewhere), or
items at the same positions within 0.05 mm; each reference finding
answers for one. A type the reference has 199 times or more is
`preexisting_unknown` and none of it is subtracted. The verdict decides
on `new_findings_strict` and keeps the old count as `new_findings`; it
gains `clean_without_subtraction`. run.py's baseline is KiCad's DRC of
the designer's board stripped as the task strips it
(`strip-kicad-tracks`: tracks, vias and small zones; the fixed geometry
on its own), through drc-kicad-board's cache and only for results with
errors that count; rows gain `new_findings_strict`,
`preexisting_strict`, `strict_unknown`, `clean_without_subtraction` and
`pass_strict` beside the unchanged `pass`; compare.py prints a strict
column beside each run's cell, the strict pass count beside the pass
count and the strict new errors in the objectives.

The v17 sweep re-judged (build/github-layout-v17, its result boards'
own `drc.json`, one baseline DRC per board): **46 of the 48 passes stay
passes**; 21 are clean without any subtraction, 25 pass with findings
the stripped designer board has at the same place. The two that do not:

| Board | Strict new findings | Why |
| --- | --- | --- |
| hackclub OnBoard krishveercard | clearance 199 (unknown) | The stripped designer board has clearance at the cap (199; 215 on the full board). Every one of our 199 is at a designer finding's place (J4's own pads at 0.17 mm against 0.2), so without the cap rule it would pass; but a capped count says nothing about the findings past the cap. |
| ASH-ART Qfwfq | starved_thermal 1, hole_clearance 199 (unknown) | C4 pad 2 (GND, B.Cu) has one spoke of two: the designer's three starved thermals are elsewhere, so it is our own. hole_clearance is capped on both boards (the switches' PTH against their NPTH pads, K33...); without the cap rule 155 of our 199 match and 44 do not, the two capped lists holding different subsets. |

**Repairs and environment.** The verdict and board-router.json
(board-layout.json) gain `repairs`: `neck_tracks` and `neck_track_mm`
(the connection-width repair's kept tracks; `NeckRepair` gains
`track_mm`), `pour_mode` and `pours_as_tracks` (the kept board's
`pours`), and `island_stubs`, `null`: the stubs are added in pcb-router
(`manual_spokes.rs`), which reports no count yet; counting them needs a
counter there (another package's). The verdict gains `environment`: the
result-changing variables that were set (`RESULT_ENVIRONMENT`:
PCB_ROUTER_SEED, PCB_ROUTER_SEEDS, PCB_ROUTER_NO_ISLAND_STUBS,
PCB_ROUTER_NO_WALL_GUARD, PCB_SKIP_NATIVE, PCB_MIN_CONNECTION_FLOOR,
PCB_KICAD_NO_NECK_REPAIR, PCB_KICAD_TIMEOUT, PCB_MAKER_KICAD_USER_LIBS,
PCB_PLACER_WORK_SECONDS, PCB_NARROW_RESERVE, PCB_PROBES_SIDE_BY_SIDE,
PCB_LADDER_STATS). run.py rows carry `repairs` (and `environment` when
set); compare.py sums the neck tracks, their length, the island stubs
where counted and the rows kept with pours as tracks. The router's
behaviour and the island stubs' default are unchanged.

Tests: verdict unit tests for the strict rule (the hidden same-type
finding, items moved together, the cap) and the unverified status;
run.py's strict rule in test_run.py; CLI tests for the three exit paths:
`a_route_kicad_did_not_check_is_never_complete` (exit 1, the
environment and repairs echoed), `a_kicad_failure_in_the_neck_repair_leaves_the_last_report`
(a fake kicad-cli that crashes after the repair's save; fails on the old
necks.rs) and `a_missing_report_after_kicads_check_is_an_internal_error`
(exit 4; the hook `PCB_MAKER_TEST_DROP_DRC` removes the report).

Short tier under the lock (x1290, 6 x 4, load 3-6) against
build/short-x1170.log (build/short-x1290.log): every routing row the same
(routed, KiCad's unconnected and errors, vias, length, first-route open
terminals; 299 s wall, cut 0); MIDAS-MK1.1's work 285.1 to 287.5 s, as
x1223 to x1261 have it since the placer commit 0d13e80. Passes 7 and 7,
strict 7: new errors 2 by the old rule, 202 strict (eternal-keypad's
hole_clearance at the cap, 199 unknown, and its shorting_items 1;
WLED_Matrix's connection_width 1, hidden before by the designer's
connection_width elsewhere, and its copper_edge_clearance 1). Repairs: 7
neck tracks (1.31 mm: WLED_Matrix 4, USB_Keypad 3), one board kept with
pours as tracks (WLED_Matrix). Binary of the rebased commit: x1291.

## GPU grid A* prototype (2026-10-21)

A bounded go/no-go experiment on branch `gpu-astar-prototype` (not for
main): the router's lattice A* as a wgpu compute kernel, against a CPU
reference that reproduces the router's search, on dumped negotiation
searches of real boards. Details, tables and how to run it:
[crates/pcb-gpu-proto/README.md](../crates/pcb-gpu-proto/README.md).
**No-go.**

- The dump (`PCB_GPU_DUMP`, `PCB_GPU_STATS`; a patch to pcb-router kept on
  the branch as its own commit): every negotiation search of iteration 1
  of `bench-search ... 3 auto` on the v16 placements of Castor, k30-SBC and
  jetson-nano, 1416 searches, with what each reads resolved for its net
  over the region it can touch, and the router's path and expansions.
- Determinism: the CPU reference equals the router on all 1416 searches
  (path node for node, expansions), and the GPU kernel (one search per
  workgroup, invocation 0 pops a four-way heap of the router's keys, eight
  plus layer-count invocations evaluate the steps at once) equals the
  reference on all of them at 1, 8, 64 and 256 searches a launch and in the
  negotiation's own batches, with and without guards against fused
  multiply-adds. Device: GTX 1650, driver 615.78.08, Vulkan, wgpu 30.0.1.
- Speed: one GPU search 0.18-0.21 M expansions a second (4.5-5.5
  microseconds an expansion: a chain of 17-22 dependent global reads at
  150-255 ns, measured by a pointer chase), 256 a launch 2.4-3.0 M (a launch
  lasts as long as its longest search), the negotiation's own batches
  0.18-0.22 M; the CPU reference 3.4-4.8 M on one thread, the router 3.0-3.6
  M (route mode, `profile rate`, x1184). Launch overhead about 0.05-0.07 ms.
  Windows are the whole board (corridors confine the search), so dense node
  arrays need 16-32 MB a search; the searches touch 0.4-1.8 MB (median).
- The share a GPU could take at identical results: the parallel batches,
  1-4 % of the wall clock in route mode (Castor 1.6 %, k30-SBC 0.8 %,
  jetson 3.9 %), about two nets each (2.1-2.3). A single search split with
  the router's order (speculative rounds of the next K pops, an ordered
  commit, cut where a pushed key undercuts) commits 10-17 expansions a round
  at K of 64-128 (`lookahead`): CPU parity on this card at best, behind an
  exact batched GPU heap.
- What would have to change first: the negotiation offering hundreds of
  independent searches per synchronisation at identical results (Jacobi and
  speculative batches were measured worse and removed on 2026-10-16; the
  parked corridor graph is the remaining candidate), or an exact batched
  device heap with a much lower dependent-read latency; and paged node
  storage. The README lists what a production backend would need besides.

## Committed on 2026-10-21 (ladder memory and work bound)

Two defects of the route-mode ladder that the 5 834-board intake exposed
(docs/benchmarks.md, "Expansion intake"): it sized its routers side by
side from /proc/meminfo alone, so a harness's cap went unseen (456 runs
killed at 3 GiB), and `ladder_budget_seconds` did not bound its work (279
runs cut by the 90 s wall guard on overrun work). Binaries x1260 (the
two fixes), x1261 (with the narrow-signal step's share, the commit).
Code in board_router/memory.rs, budget.rs, route.rs, attempt.rs;
docs/router.md, "Memory limits" and "The work bound in route mode".

**Memory limits.** The ladder reads the process's limits as it starts and
runs as many routers side by side (rung probes, rungs, seeds) as fit the
tightest: RLIMIT_AS (against the virtual size), the cgroup's memory.max
(v2, every level up to the root; v1's memory.limit_in_bytes), an
explicit limit (`PCB_MAKER_MEMORY_MB` or the new router setting
`memory_mb`, the smaller, against the resident size), and half of
MemAvailable as before. A router's peak is estimated from its lattice
maps (each attempt now reads its router's `memory: N MB in maps` and
reports it as `maps_mb`): 1.7 x maps + 460 MB resident, 2.0 x maps + 550
MB of address space, times 1.25. Before, a rung counted the process's
peak after the first attempt plus 2200 MB for a KiCad check that no
longer runs inside a rung, and a probe 1.5 times what building the first
router added to the resident size.

The ratio, measured with one router at a time (pours connect, no finer
pitch, no seeds), one thread, x1225: peak RSS over maps 1.05 (DevMubarak1
pidgin_translator, 244 MB of maps), 1.33 (sam17-power-v2, 739), 1.28 (nz,
298), 1.68 (WK-3, 763), 1.61 (PRODIGY-MOTHERBOARD, 580) on five intake
resource_limit boards; 2.54 (Castor, 264), 2.49 (k30-SBC, 319), 3.22
(jetson, 302) on the v16 placements. VmPeak over maps 1.41-1.98 and
3.19-3.80. The ratio falls with the maps (a router holds 200-600 MB
beside them: the board parsed and lowered, its pours; jetson's process
holds 285 MB before any router), so the estimate is a line through all
eight, not one ratio (which would size a 1 GB-maps router three times
too large). Decisions that change what the ladder attempts go by the
limit itself, a fixed number: no finer pitch whose one router does not
fit a limit of the process's own, no rung probes where two routers (the
leader stays while the next probe runs) do not. The log names the limits
(`memory limits: the address space limit (RLIMIT_AS) 3072 MB, 3033 MB
free; ...`) and every side-by-side count with the router's estimate and
the limit that held it; board-router.json has `memory_limits`.

The judge: ten resource_limit boards of the intake
(resource_and_cut_boards.csv), stripped as the intake did, routed at its settings (route mode, 30 s,
`seeds` 1, `skip_native_verification`, `deadline_seconds` 90, one
thread) under `prlimit --as=3221225472`, 3 at a time
(scratch driver: polls VmHWM and VmPeak, wait4's maxrss):

| board | x1225 RSS / VmPeak MB | x1225 | x1261 RSS / VmPeak MB | x1261 |
|---|---|---|---|---|
| embeddedalpha IMU_V100 | 2643 / 3037 | killed (allocation failure) | 1190 / 1549 | 7/19, 34 open |
| DevMubarak1 pidgin_translator | 2570 / 3034 | 6/7, 1 open | 763 / 960 | 6/7, 1 open |
| sssamuelll sam17-power-v2 | 2473 / 3068 | killed | 969 / 1268 | 31/33, 2 open |
| knewter kvm-carrier-fanout | 2802 / 3066 | killed | 1091 / 1191 | 0/51, 193 open |
| DownUnderCTF sirkit | 2724 / 3072 | killed | 1001 / 1192 | complete 104/104 |
| sundevilmotorsports CANSniffer | 2295 / 2626 | 11/57, 145 open | 1058 / 1331 | 11/57, 145 open |
| zOnlyKroks WK-3 | 2618 / 3062 | killed | 1317 / 1567 | complete 165/165 |
| ENACRobotique useless_box_2025 | 2794 / 3064 | killed | 720 / 894 | 0/9, 191 open |
| mark69163 PRODIGY-MOTHERBOARD | 2750 / 3072 | killed | 944 / 1268 | 162/257, 476 open |
| sikaxn nz | 2608 / 3072 | 29/65, 101 open, wall cut (64 s of work) | 825 / 999 | 28/65, 87 open |

Before, seven die and the three that finish come within 450 MB of the
cap (VmPeak 2626 to 3072 MB; DevMubarak1 ran its 0.05 mm rungs, 975 MB
of maps each, two side by side); after, all ten finish at 894-1567 MB of
address space, two of them complete. Four boards (sirkit, PRODIGY,
kvm-carrier, WK-3) run their rungs one after the other instead of
probing them (two routers do not fit 3 GiB), three skip a finer pitch
whose router does not fit (nz and useless_box at 0.05 mm, kvm-carrier at
0.075), each logged. The intake's own cap was a resident-size kill at 3
GiB (and RLIMIT_AS at 12 GiB): with
`PCB_MAKER_MEMORY_MB=3072` and no address-space limit the ten peak at
800-1393 MB resident. knewter's and ENACRobotique's boards route nothing
in either binary (no negotiation: their nets cannot be lowered to
anything the router reaches).

**The work bound.** In route mode (no layout `deadline_work`) the
ladder's whole work stays within `ladder_budget_seconds` and 5 %
(`LADDER_WORK_TOLERANCE`): every router has a work deadline (the first
attempt all that is left, the rungs of a pitch after it an even share of
what is left when they start, seeds a share each, the steps what is left);
negotiations at most half of what is left (the 60 s floor gave a 30 s
budget's rungs 60 s each); probes together at most half the budget
(3.75 s each on 30 s with four rungs, were 75 s); a finer pitch only if
its projected work fits; nothing starts with less than a twentieth of
the budget left. Where the narrow-signal step applies, the first
attempt's work is kept for it once that attempt leaves connections open
(x1261: x1260 left facade60's step no work, 20 open; with the share 7).
The layout's ladder is unchanged (every new rule is off with a work
deadline).

Expansion tier, 30 s, 8 jobs x 1 thread under the lock, x1231 (the
reference log) against x1261:

| | x1231 | x1261 |
|---|---|---|
| pass | 14/46 | 15/46 |
| rows cut by the wall guard | 13 | 0 |
| ladder work: median / mean / max | 52.7 / 65.4 / 253.8 s | 31.3 / 27.7 / 34.2 s |
| work, CPU, wall of the run | 3007 s, 4902 s, 337 s | 1276 s, 1553 s, 136 s |
| router's open connections (sum) | 216 | 293 |
| KiCad unconnected (sum) | 508 | 808 |
| outcomes | clean_slow 11, clean_medium 1, cut 9, at_budget 21, early 4 | clean_slow 4, clean_medium 9, at_budget 18, early 15 |

The ladder's work never exceeds 34.2 s (tuxkeychain; Wonkle 34.0: what
routers do past their deadlines, below). Every outcome change:

- Complete in both (12), now in less work and wall: bms (87 to 33 s of
  work), nixie_board, TokayEdge, ZSWatch (112 to 32), flexypin,
  magnetorquer, ICE40HX1K, musicbrain, mini48, yuiop29re, brickpico,
  Multi_saturation (10.5 s in both, identical): all pass; seven move from
  clean_slow to clean_medium (the class is wall time) and the four that
  were cut are not.
- Better: MakerTiles Screen 2 open to complete (its narrow-signal step,
  14 s), epdInky 13 to 3, Miniboard 5 to 2, nervous-ecg 2 to 1.
- Same open count (21): EMG_DAQ, Julkort, FUEL4EP, LC_PAMV4b and
  Barklind (both still pass), RS485M (its narrow-signal step, 0.6 s, now
  has its share: x1260 left it 2 open), BloopBox, noise, minuet, pdlc,
  rbo12864h, nrf24_remote, snaphat, dreamV1_0, 3117_AUSIGN,
  Zulassungskarte, CompassTag, atmega32u4, wanbeiyu, and the two that
  were cut (backplane, node_v6). Nine of them (and nervous-ecg and
  Miniboard above) move from incomplete_at_budget to incomplete_early:
  the class means less than 95 % of the budget in work, and they stop at
  18-28 s because the next attempt does not fit what is left (wanbeiyu:
  "skipping pitch [0.075]: projected 35 s exceeds the 13 s left").
- Worse, all boards that used to overrun (their work then in brackets):
  rpi_cm5_dronecarrier 3 to 8 open (129 s, cut), OpenRX-Mono 12 to 13
  (74 s), tuxkeychain 11 to 13 (201 s, cut), driver_cplex_rp2040 3 to 20
  (254 s, cut; KiCad 7 to 170 unconnected: its probes at 3.75 s rank
  the tracks rung first, at 75 s the connected pours), MSign 4 to 11 (138
  s, cut), OpenDeck 1 to 11 (46 s, cut), facade60 3 to 7 (58 s: its
  narrow-signal step had 19 s then, 9 s now), anthracite 1 to 2 (86 s,
  cut), Wonkle 90 to 136 (61 s, cut). In all 93 more open connections
  (router's count) and 317 more KiCad unconnected items, 263 of them
  driver_cplex's and Wonkle's.

At 300 s (`--budget-scale 10`, x1261, the nine boards that got worse
and node_v6, 8 jobs x 1 thread under the lock; work 289-329 s), against
x1231's 30 s rows with their overrun:

| board | x1231 (30 s, its work) | x1261 30 s | x1261 300 s | intake 300 s |
|---|---|---|---|---|
| rpi_cm5_dronecarrier | 3 open (129 s, cut) | 8 | 1 | 1 |
| OpenRX-Mono | 12 (74 s) | 13 | 12 | 12 |
| node_v6 | 21 (221 s, cut) | 21 | 20 | 21 |
| tuxkeychain | 11 (201 s, cut) | 13 | 8 | 10 |
| driver_cplex_rp2040 | 3 (254 s, cut) | 20 | 3 | 2 |
| MSign | 4 (138 s, cut) | 11 | 6 | 6 |
| OpenDeck | 1 (46 s, cut) | 11 | 1 | 1 |
| facade60 | 3 (58 s) | 7 | 5 | 5 |
| anthracite | 1 (86 s, cut) | 2 | 1 | 1 |
| Wonkle | 90 (61 s, cut) | 136 | complete, pass | complete |

The 300 s run recovers eight of the ten to x1231's count or better
(Wonkle complete, tuxkeychain 8, rpi_cm5 1); MSign and facade60 stay two
above it, at the intake's own 300 s result. Their x1231 rows had the
work the budget did not allow (MSign 138 s before the wall guard cut
it, facade60 a narrow-signal step of 19 s).

**The tolerance, measured.** The ladder plans within the budget and 5 %;
what a router does past its work deadline comes on top (a fresh router's
first negotiation iteration, the phases that finish a negotiation the
deadline stopped): at 30 s the ladder's work ends at 9-34.2 s
(tuxkeychain: four probes' first iterations and the continuation's
finish, 2.7 s past the bound), at 300 s at 289-329 s (MSign: its
narrow-signal step took 82 s with 72 left). So a budget B means at most
about 1.15 B of work on these boards, and the wall guard (3 B, at least
90 s) fires only where the machine runs the work at less than about 40 %
of the calibrated rate.

Short tier under the lock (6 jobs x 4 threads, x1261 against
build/short-x1170.log): every row identical apart from timing and the
tasks file's spelling, except MIDAS-MK1.1's work seconds (285.1 to 287.5,
also in x1223, x1240 and x1250: the MIDAS-MK1 commit); against
short-x1223 and short-x1250 every row identical, and all 12 result boards
byte-identical to x1230's and x1250's (against x1223's only the footprint
graphics' order, 3d28004's writer). CPU 3354 to 2930 s, 940 to 320 s of
wall, 0 cut.

Unit tests (board_router/tests.rs): the limit resolution (machine only;
the address space against the virtual size; every limit with the
tightest deciding, the cgroup level with the least free, the explicit
limit the smaller of setting and environment), cgroup paths of both
versions, the maps from a real router's memory report, and the bound's
rules (the bound, the probes' share, the negotiation, the rung shares,
the step's share and its time). `bash scripts/check.sh` passes.

## Committed on 2026-10-21 (parser and writer robustness)

The reader and writer defects that the 5 834-board intake
(benchmarks/expansion/data/analysis: model_error_boards.csv,
verification_unknown_boards.csv, source_issue_boards.csv and the four
boards with new malformed_courtyard findings in drc_failure_boards.csv)
put on us. Binaries x1230 (the code on f720ab7) and x1231 (rebased on
8877428, the pcb-router quality package). Each defect has a unit test
from the board's own geometry where its licence allows (MIT, BSD,
Apache), a synthetic equivalent otherwise. Each board was run as the
intake ran it (strip-kicad-tracks, route-kicad-board on the designer's
placement with the intake's router-config.json: 30 s of work, one seed,
90 s wall guard; then drc-kicad-board), with x1211 before and x1230
after (scratch runs, not kept).

| Defect | Boards | Cause | Fix | x1211 | x1230 |
| --- | --- | --- | --- | --- | --- |
| rule area "fewer than three points" | Boardoza LSM303, McbeEringi 30led, tobycm skibidi disc | a zone outline that is one arc ending where it starts: KiCad reads it as a full circle (its arc centre is halfway between start and mid; 30led's own B.Mask fill of that outline is a 26 mm circle), we as two points | `outline::pts_points` lowers a closed arc as the circle (`full_circle`, shared with the Edge.Cuts arc); so the areas are kept as KiCad keeps them, not skipped | error | routed 17/17, 26/26, 14/14; KiCad: no rule-area finding; Boardoza 80 track_width (0.127 mm tracks, see below), 30led hole_clearance 4 as the designer's, tobycm 5 unconnected (below) |
| custom-pad primitive gr_bbox | 2 kicad-assistant QA boards | the pad editor's number box, which KiCad parses as a proxy shape (`SetIsProxyItem`) and leaves out of the pad's copper | lowered to nothing | error | 86/87 and 62/63 at the budget, KiCad loads and checks them (2 and 3 unconnected) |
| degenerate copper polygon | BratwurstPower | an fp_poly of three points, the last the first, 2 nm wide, negative stroke: `pts_points` dropped the repeated point, two left | the closing point is dropped only while three stay: a zero-area shape, as KiCad keeps it (Edge.Cuts skips polygons with fewer than three distinct points as before) | error | 67/67, KiCad: 1 starved_thermal |
| roundrect pad without roundrect_rratio | jayis1 lumen-tap | required | KiCad's default 0.25 when absent; a ratio that is there must still be a number | error | 14/14, KiCad: 1 shorting_items + 1 solder_mask_bridge (the designer's: 2 + 2, 39 unconnected) |
| ambiguous net identity "/A15" vs "A15" | gametank exp_port_mod_board | a KiCad 10 board has no net table, and `note_net_names` read the bare names from the table alone: none, so "/A15" folded into "A15" | `note_net_names` also takes the pads' names; the writer spells a connection that is a bare net as it is (`written_net_name`; "/A15" then is connection "/A15", written "//A15", and `canonicalize_copper_net_names` finds each pad's spelling) | error | 41/41, KiCad: the designer's copper_edge_clearance 22 and invalid_outline 2, no shorts |
| image data on one 24 MB line | stonedDiscord EMP | the encoder writes a list's atoms on one line; KiCad reads lines of at most 1 MB | a `data` list one string a line, as KiCad writes it | KiCad cannot load the result | KiCad loads it: courtyards_overlap 9 (the designer's 9), 0 unconnected |
| footprint graphics reordered on write | AccordeurDeGuitare, 3 dacha boards | the canonical order sorts a footprint's graphics, and KiCad chains a courtyard in file order: MK1's courtyard (two zero-length lines, two arcs) closes in the designer's order only | `canonical_order` notes each footprint's source order of graphics (by a digest of its reference and graphics); `encode` writes them back in that order while they are unchanged (a flipped footprint or a moved label is written as held); keys and digests use `encode_canonical` | malformed_courtyard 3, 10, 22, 16 | 0, 6, 18, 14: the designer's counts |
| outline "do not form closed loops" although KiCad accepts it | 8 hermod keyboard halves, 4 panels (Pips801 minibadge, SEM-karifa rudolf, prrpls406 and programmer panels) | outlines drawn from 10-20 um segments: the chaining snaps each end to the nearest one seen before and drops pieces shorter than its 0.02 mm tolerance, which tears such outlines apart | when that chaining fails, KiCad's own (`chain_as_kicad`: integer nanometres, an unused exact hit first, else the nearest end within 0.02 mm, closed when the end is back within 0.02 mm of the start); outlines that chained before are untouched | strip-kicad-tracks fails | all 12 load and route: hermod 119/119 to 133/133 (one 123/124, cut), Pips801 99/99, rudolf 162/162 clean, programmer 16/16, prrpls406 27/162 at the budget (41 unconnected); KiCad's findings the designer's plus those below |
| "trailing input" | RoyalBlue54L-Feather | "(curved_edges no)filter_ratio 0.9)" in each teardrops form: KiCad's teardrop reader takes the "(" as optional, ours closed the pad and then the board | in a `teardrops` list a bare key starts a list that ends at its ")" | strip fails | loads; 43-46/68 at the budget (cut), 30 unconnected, the designer's other findings |

**Not fixed here** (in files other packages own):

- **SeanBP Edge-Card-CAEN, "no routable connections"**: its 70 nets are
  named "1", "3", ..., "139", and `board_router::routable_net` rejects
  names of digits only (meant for single-pin placeholders). Counting
  digit names shared by two pads as `note_jumper_nets` counts
  `unconnected-(` names (a two-line change, tried on a scratch build)
  routes 70/70, but KiCad then finds 70 unconnected: every net joins a
  test point on F.Cu and one on B.Cu 0.3 mm apart, which the router takes
  as one terminal; KiCad needs a via. Both for the board_router package.
- tobycm skibidi disc: copper rings drawn as `gr_circle` with a net on
  `(layers "F.Cu" "F.Mask")` are not modelled; KiCad: 5 unconnected.
- Boardoza LSM303: 80 GND tracks of 0.127 mm on a project of 0.2 mm
  minimum width (the board's own narrow width).
- hermod halves: hole_to_hole 62 and 14, hole_clearance 5 and 10 from
  vias near switch holes (a custom rule of 0.5 mm the router does not
  read); prrpls406: hole_to_hole 24.

**Tiers.** Short tier (x1230, `--jobs 6 --threads 4`, under the lock):
`build/short-x1230.log` equals `build/short-x1170.log` row for row
(routed, vias, length, work, KiCad findings): 7/12, unconnected 19, new
errors 2, work 2501 s. Expansion tier (x1231, `--jobs 8 --threads 1`):
`build/expansion-x1231.log` against `build/expansion-x1211.log`: 14/46
pass and the same outcome classes (clean_medium 1, clean_slow 11,
cut_wall_clock 9, incomplete_at_budget 21, incomplete_early 4); the 33
rows the 90 s wall guard cut in neither run are identical (routed, vias,
length, work seconds, KiCad findings, outcome); the 13 cut in both differ
as cut rows do with the load. Four stripped boards (EMG_DAQ_ADS1292,
Barklind, power-manifold backplane, anthracite-bottom) have another
SHA-256: their embedded files' `(data |...|)` is back on its lines, the
same tokens in the same order. None of the 46 boards is among the fixed
ones. x1231 and x1230 give byte-identical routed boards (four of the
boards above checked).

Tests: `outline::tests` (closing as KiCad, the closed-arc circle, the
degenerate polygon), `canonical_order::tests::footprint_graphics_are_written_in_the_files_order`,
`roundrect_pad::tests::roundrect_without_a_ratio_takes_kicads_quarter`,
`tests::a_custom_pads_number_box_is_no_copper`,
`tests::a_teardrop_setting_without_its_opening_parenthesis_reads_as_kicad_reads_it`,
`tests::embedded_data_is_written_one_string_a_line`,
`tests::colliding_normalized_pad_names_stay_two_nets_when_the_board_declares_both`
(now the KiCad 10 case); `route_obstructions`' malformed-roundrect test
no longer counts a missing ratio as malformed.

## Committed on 2026-10-21 (expansion tier)

The intake's proposed 46-board tier (benchmarks/expansion/analysis) as a
run.py tier: `--tier expansion` (benchmarks/agent-tasks/README.md, "The
expansion tier"). No Rust change; binary x1211.

- **Route-mode tasks** (`"mode": "route"` in a task): the designer's
  placement, fetched into `build/github-expansion/<task>/source` when not
  there (the harvest's copy when its SHA-256 matches, else GitHub at the
  pinned commit with harvest.py's fetch; every file verified, staged, and
  nothing kept on a mismatch), stripped with the intake's exact
  `strip-kicad-tracks <original> <copy>`, routed with `route-kicad-board`
  at the task's `budget_seconds` times the budget scale with the intake's
  router settings (one seed, no KiCad check inside), wall guard (max(90,
  3 x budget)) and process timeout (max(180, 3 x budget + 60)), then
  checked with `drc-kicad-board`. Rows add `license`, `fetched`,
  `fetched_from`, `source_sha256`, `stripped_sha256`, `budget_seconds`,
  `routing_seconds`, `open`, `drc_seconds` and `outcome` (the intake's
  class); `peak_rss_mb` covers the DRC too and the summary counts the
  outcomes. `constraints` is optional in every task.
- **Licences**: every task carries its repository's licence and every row
  prints it; a board without one (missing, empty, `NOASSERTION`) is not
  fetched.
- `expansion.json` (budget_scale 1: 30 s; `--budget-scale 10`: the deep
  run's 300 s) and `expansion-tasks.json`, written by `from_expansion.py`
  from the proposal, the CSV, the harvest's manifest and the quick run's
  results; `--harvest-data`; `--timeout` defaults to the intake's in route
  mode. compare.py reads the rows unchanged. Tests:
  `test_expansion.py` (task list, router settings, strip arguments against
  triage.py's, the fetch with a fake GitHub, the outcome classes).

**First run, x1211** (8 jobs x 1 thread, load 13-23, 471 s): 14/46 pass;
clean_slow 11, clean_medium 1, incomplete_at_budget 21, cut_wall_clock 9,
incomplete_early 4. 45 of 46 boards have the intake's outcome class (the
other is the same route in fewer wall seconds: B1's line is wall time).
All 46 stripped boards match the intake's, and all 32 routes that neither
run's 90 s wall guard cut are identical, work seconds included: the
1.57x calibration was already in the intake's binary (015676c precedes
be4b012) and the router has not changed since. 13 rows are cut by the
wall guard and depend on the load. Three B4 boards at 300 s pass with the
intake's deep work seconds.

## Committed on 2026-10-20 (MIDAS-MK1 refusal)

In layout sweep v17 (x1170, tasks v7) MIDAS-MK1 ended "incomplete: 6
parts without a legal place (J105, J101, J102, R204, R207, R208)" after
206 s; v16 (x862) had placed and routed it (151/159, 16 unconnected).
Snapshots x1221-x1223 are this work on 0425fea; x1224 is the commit on
3d28004 (MIDAS's poses as x1223's; the file differs only by 3d28004's
writer, which keeps footprint graphics in the file's order).

**Which rule.** None of the rules is wrong. Only J105 is unplaced; the
five others are fixed parts it was left on: J105 (a 7.7 x 35.4 mm Molex
terminal block held at the bottom edge, turned 90) stood over the back's
fixed R204/R207/R208 (its holes, "far side") and against the front's
fixed J101/J102. `PCB_PLACER_DESIGNER` calls the designer's J105, J103
and J108 legal (its 70 flagged parts are spacing at the default level).
The switches change nothing: `mask_openings`, `solid_pin_rows`,
`keep_copies` off give x1170's placement byte for byte,
`orientation_search` off another placement with the same refusal.
Bisecting the snapshots on `place-kicad-board` (the v17 source, three
seeds, 600 s of work, four threads): x862, x1112, x1130 and x1152 place
all; x1161 (order independence) refuses. The canonical order sent all
seeds' anneals elsewhere: J108 (a 24.7 x 19.1 mm SD slot held at the
left edge) into the bottom-left corner, where the designer has it at the
top left. Left of the fixed parts the bottom edge then holds one of the
two terminal blocks, not both.

**A search gap.** Largest first, the legaliser seats J108 in the corner,
J103 beside the fixed parts, and J105 finds nothing. With J105 first
(the retry) J105 takes J103's spot and J103 finds nothing; eviction moves
the parts on one spot but cannot reseat J103, which needs J108 moved.
J108 has a spot only once bodies may reach past the edge (the inset
levels: x862 put it 4.6 mm over the left edge, copper inside), and those
levels only keep the placement and try the stuck part among the kept
ones. With J105 and J103 both first, both stand at the bottom and J108
moves left over the edge by 1.9 mm.

**The fix** (`legal::legalize_stuck_first`, `lib.rs`'s kept-placement
path): at a level with no halo, spacing or grid left to give (the edge
and courtyard levels), when the kept placement leaves a few parts
without room after keeping and eviction (as many as an eviction takes
on), the level legalizes the anneal that gave the kept placement again
with those parts first; the parts that pass leaves without room join
them, up to four passes (`STUCK_FIRST_PASSES`), while new parts fail and
the work lasts; the pass with fewest parts left wins if it beats the
kept placement. MIDAS: J105 first leaves J103, J105 and J103 first leave
none. Two broader variants were measured and not kept: the passes also
in the fresh anneal's retry and at every kept level (x1221), or at every
kept level (x1222), seat stuck parts of other seeds at stricter levels
and change the seed kept on framework_mobo and OpenFC (16 seeds); x1222's
short tier lost framework_mobo's pass (70/71). A whole-board search on
the fine grid for parts placed first was tried too and not needed.
Unit test `parts_without_room_go_first_until_none_takes_anothers_place`
(tests/placement.rs): MIDAS's bottom edge simplified (the two blocks,
the slot in the corner, a fixed block on the right): `legalize` leaves
J105, `legalize_first` with J105 leaves J103, the passes leave none and
every part is legal.

**MIDAS-MK1** (layout, tasks v7, four threads, build/midas-x1223.log):
complete placement (3 seeds, wirelength 2295 mm against x1170's 2431 with
J105 unplaced and x862's 2628), 153/159, **6 unconnected** (v16 16), 0
new errors, 205 vias (v16 207), 4225 mm (v16 4565), work 1404 s, cut 0,
first-route open terminals 13 (v16 19). x1221's and x1222's routed boards
are the same file.

**Identity.** `place-kicad-board` on short-x1170's prepared sources
(each board's layout.json, four threads): spell_tome_bottom,
USB_Cable_Tester, urchin, eternal-keypad, OpenFC and Blue_Line
byte-identical with x1170; framework_mobo and OpenFC with 16 seeds too.
Short tier (jobs 6, threads 4, under the lock) x1223 against
build/short-x1170.log: 7/12 both, unconnected 19, new errors 2, vias
1134, length 21228 mm, cut 0, first-route 26 on both; every row
identical and every placed board byte-identical but MIDAS-MK1.1's work,
285.1 to 287.5 s: it is MIDAS-class (two terminal blocks held at the
bottom, J101 and J102), a seed's stuck parts took the passes at a
loosest level without a better result (build/short-x1223.log).

**The other v17 refusals** (`place-kicad-board`, v17 sources and
layout.json, 600 s of work, four threads; unplaced x1170 / x1223): the
molecule panels are the designer's own geometry (neighbouring tiles' J5
and J6 0.03 mm apart on the back, J9 against the outline; panel fixes,
2026-10-15), not this mechanism, and the passes seat some of them:
7x5_wave_left 1 / 1 and 5x2_shifted 2 / 2 byte-identical, 4x4 3 / 2,
4x5 7 / 3. OpenESC's 4in1-panel is crowding (111 / 111 unplaced,
byte-identical: too many stuck parts for the passes). SmartSpin2k
finished in v17 and was not rerun.

## Committed on 2026-10-20 (quality: research stack removed)

The research stack of August and September 2026 removed, as Florian
approved (branch quality-research: 346c9ba, 8ac4653, d988456, 9df9ee5 and
this section, on be4b012). **The last
commit holding all of it is be4b012** (main on 2026-10-16); check it out to
run or read any of the removed commands, crates, tests or fixtures. Rust
goes from 190 752 to 118 802 lines; 152 444 lines deleted in all.

| Step | What | Lines deleted |
| --- | --- | ---: |
| 1 | `crates/pcb-pipeline` (no `pcb_pipeline::` use anywhere) | 763 |
| 2 | 54 research command arms of `src/main.rs`, their helpers (the semantic ladder's placement and order portfolios, the resistor field, the config loaders), their unit tests and usage lines: `main.rs` 6610 to 1008 lines | 5 602 net |
| 3 | the crates `layout-trace-routing` (24 707), `pcb-routing` (12 862), `pcb-coordinator` (9 083), `pcb-placement` (8 750), `pcb-benchmark` (3 667), `pcb-validate` (2 637), `pcb-viewer` (532), with the root tests that only tested them (`exact_validation_smoke`, `occupied_passage`) | 62 567 |
| 4 | the root tests `kicad_ladder` (2 038), `kicad_pad_layer_contacts`, `board_continuation`, `conflict_actions`, `kicad_grid_alignment`, `kicad_via_keepout`, `kicad_pad_clearance`, `imported_benchmarks`; the fixtures `benchmarks/esp32-c3-ladder` (29 814), `dual-esp32-ladder` (12 060), `analysis-lab`, `small`, `esp32-pad-gaps`, `competitive`, `smoke` and `imported` but four files (23 790); the experiments `placement-locality`, `field-routing`, `occupied-passage` | 82 976 |

**Commands removed** (54): check, validate, insert-connection,
trace-placement, place, route-grid, route-grid-negotiated,
explore-conflict-actions, search-conflict-actions, continue-board,
place-route-grid, place-route-grid-width-continuation,
place-route-grid-negotiated, analyze-corridors, analyze-route-families,
materialize-route-family-portfolio, repair-route-families,
repair-route-grid, optimize-route-junctions, repair-route-pressure,
repair-route-order, repair-route-ripup, view-route, render-route-layers,
view-continuous-repair, relax-continuous, resistor-field,
resistor-ablation, generate-semantic-kicad-ladder,
solve-semantic-kicad-prefix, solve-semantic-kicad-order-portfolio,
solve-semantic-kicad-placement-portfolio,
solve-semantic-kicad-prefix-feedback, materialize-kicad-rung,
insert-kicad-connection, progress-kicad-connections,
relax-kicad-joint-routes, search-kicad-board-sequential-orders,
propose-kicad-pad-pair-route, route-kicad-connection,
diagnose-kicad-yielding-connections,
repair-kicad-single-connection-ripup, import-kicad-route-candidate,
apply-kicad-route-candidate, relax-kicad-reference-fields,
search-kicad-reference-fields, search-kicad-via-actions,
replace-kicad-connection-with-zones, measure-kicad-route-candidate,
shorten-kicad-route-candidate, relax-kicad-route-candidate,
benchmark-route-freerouting, benchmark-route-compare,
benchmark-route-corpus.

**Commands kept**: the agent commands (route-, layout-, place- and
drc-kicad-board; repair-kicad-silkscreen, strip-kicad-tracks and -copper,
import-kicad-netlist, export-kicad-fab, score-kicad-board,
describe-kicad-board, swap-kicad-pins), verify-kicad-rung,
normalize-kicad-footprint-links, shrink-kicad-board, inspect-kicad-board,
-connections, -routing-cut, -terminal-access and -route-obstructions,
score-congestion, score-congestion-sample, export-congestion-sample,
dump-pour-map, repair-kicad-necks, bench-search,
route-kicad-board-freerouting (benchmarks/corpus/run.py),
render-kicad-layers, and for the next package to decide
route-kicad-board-adaptive, route- and resume-kicad-board-sequential,
refine- and discover-kicad-vias, and **bridge-kicad-open-connections**:
route-kicad-board-freerouting's `island_bridges` option
([native-external-routing.md](native-external-routing.md)) runs it as a
subprocess.

**Kept for the pcb-kicad package** (they go with pcb-kicad's research
modules): the crates `layout-trace-model` (pcb-kicad's
`semantic_template.rs`), `pcb-core` and `pcb-engine` (pcb-kicad's
`lib.rs`), `pcb-grid-router` (`lib.rs` and `routing_cut.rs`, behind
inspect-kicad-routing-cut); no longer root dependencies. Four files of
`benchmarks/imported/layout-trace/` (dual-esp32-benchmark,
forced-crossing-two-layer, decoupling-capacitor,
esp32-slices/esp-pad-fanout-clearance) stay because the tests of
`semantic_template.rs` and of `layout-trace-model` include them;
`benchmarks/manifest.sha256` lists only those. `experiments/whole-board`
stays: `src/freerouting_backend.rs` and pcb-kicad's `island_bridge.rs`
include its scripts. `experiments/placement-exploration` (a standalone
crate on `pcb-placement`) no longer builds; it is history. Historical
documents (docs/experiments, docs/reviews, experiments/*.md) that name
removed commands or fixtures are left as written.

`benchmarks/README.md` is now an index of the benchmark directories;
`benchmarks/real/README.md` notes that its benchmark-route commands are
gone. `main.rs` (1 008 lines) needed no split.

**Gate**: `scripts/check.sh` passes after every step (455 library
tests, the three CLI suites, the Python tests); `cargo clippy --release
--workspace` has the same warnings in the kept crates as before (the
root's 2, pcb-kicad's 46, pcb-router's 26, pcb-topo's 11, pcb-placer's 6,
pcb-congestion's 2; pcb-placement's 12 left with it).

**Short tier** (x1210 = the branch's code at 9df9ee5, `--jobs 6 --threads
4` under the lock): `build/short-x1210.log` equals `build/short-x1170.log`
in every row but the CPU, wall and memory fields: 7 of 12 pass,
unconnected 19, new errors 2, vias 1134, length 21228 mm, work 2501 s, cut
0, first-route open terminals 26.

## Committed on 2026-10-16 (quality: pcb-kicad)

A code-quality pass over the layout, router and placer adapters of
`pcb-kicad` with no behaviour change. Binaries x1190 (step 1), x1191
(step 2), x1192 (step 3), x1193 (rebased on the research removal), x1194
(rebased on the pcb-router quality package, for the final gates), x1195
(this commit on 3d28004).

**Removed switches and options** (each restored code from before a change
the measurements settled):

- `PCB_LADDER_OLD_RESERVE`: the ladder's narrow-step reserve is a floor
  ("narrow reserve floor" below); `narrow_step_reserve` is the floored one.
- `PCB_LADDER_FORCE_RUNG`, `PCB_LADDER_SHORT_RESUME`: the ladder judge's
  experiment hooks (`experiments/congestion/ladder_judge.py` used them; the
  predictor is a closed negative). `PCB_LADDER_STATS` stays.
- `PCB_LAYOUT_OLD_FIRST_ROUTE_BUDGET` ("first-route budget").
- `PCB_LAYOUT_NARROW_CONTINUATION` and `_FLOOR`: after the narrow-signal
  attempt run alone only the other pour mode runs ("after the narrow
  attempt"); the stop, seeds and ladder variants and their seed helpers
  are gone.
- `PCB_KICAD_OLD_ZONE_ARCS` ("zone seams").
- Placer options `bus_rows` (with `bus_row_gap_scale` and the row
  constraint's internal `turns`), `supply_via_room` (with
  `supply_via_room_mm`) and `replicate_channels` (with
  `PCB_PLACER_CHANNEL_SELF`, the channel lowering and `lower_placement`'s
  channels argument): all off, none won. A placer config that still sets
  one is now rejected (the config denies unknown fields);
  docs/constraints.md still documents `bus_rows`.

- `jacobi_batch` in the router config and `PCB_JACOBI_BATCH` in
  src/main.rs: the router no longer reads it (pcb-router quality).

Kept: the user-facing kill switches (island stubs,
solid starved thermals, `PCB_KICAD_NO_NECK_REPAIR`) and every diagnostic
switch.

**Clippy** is clean on the files of this pass: dead code (the wall
guard's `Labels`, `Check` and `Report` steps, never asked: asking would
only fire the guard later; `courtyard_shapes`; `default_edge_rule`, a
serde default on a Serialize-only result; `turned_box`), the move trial's
restore before returning an error (the error ends the layout), type
aliases, allows with reasons for long argument lists and the negated
comparisons that reject NaN, `clamp` where the bounds are constants and
the value never NaN.

**Splits** (moves): `board_router.rs` (6750 lines) into
`board_router/{config, result, lower, tools, geometry, memory, budget,
route, pour_plan, attempt, tests}.rs`, `board_placer.rs` (5300 lines) into
`board_placer/{config, result, rules, bodies, decoupling, courtyards,
lower, edges, trace, seed_race, place, tests}.rs`; `mod.rs` re-exports, so
paths outside are unchanged. A line multiset check of old against new
finds only rustfmt reflows, the module lines and `lower_board`'s own copy
of `polygon_area` (the same products in the same order), which gives way
to the module's.

Lines: step 1 -1236 (148 added, 1384 removed), step 2 -39, step 3 +159
(module headers and rustfmt reflows of the moved code).

**Gates**, after each step: the tests of pcb-kicad, pcb-placer and
pcb-router and the CLI exit-code and walkthrough tests; place-kicad-board
on the short tier's prepared x1170 sources (spell_tome_bottom,
USB_Cable_Tester, urchin, eternal-keypad, OpenFC, Blue_Line; 4 threads,
120 s of placement work), boards byte-identical to x1170; layout mode, one
run a side, 4 threads, on TelemetryOnboard and MokyaLora (the ladder keeps
244 s for the step; Telemetry releases it complete) and vaio_re (the
narrow-signal attempt alone, then the other pour mode): rows identical
but CPU, wall and memory, boards byte-identical to x1170
(`build/qc-lay-x1170`, `-x1190`, `-x1191`, `-x1192`).
At the end, on x1194: the short tier under the lock (6 jobs, 4 threads)
against build/short-x1170.log, all 12 rows identical but CPU, wall,
load and memory, every board byte-identical (7/12, unconnected 19, new
errors 2, 1134 vias, work 2501 s); the schematic tier against x1193 (the
same code on the previous base) all 12 rows and boards identical. Against
build/sch-x1161.log the rows differ in `work_seconds` and
`kicad_check_work_seconds` only (x1161 predates the KiCad check work
accounting, d490aad), boards identical, but TelemetryOnboard (108/108 to
107/108): x1170, main's code before this pass, gives the x1193 row and
board exactly (build/sch-x1170-telemetry), so the change is the check
work's, not this pass's. On 3d28004 (x1195): scripts/check.sh, and
place-kicad-board on the six boards byte-identical to main's x1232.

Not taken (behaviour changes, for a later agent): `routable_net` rejects
net names made only of digits (SeanBP Edge-Card-CAEN); the writer's
footprint file order in a process-global registry rather than on the
board (this pass did not touch `encode`).

## Committed on 2026-10-16 (quality: pcb-router)

A code-quality package for `crates/pcb-router`, no algorithm change:
every step routes byte-identically to x1170 (main 54eff39). Branch
`quality-router`, four commits, snapshots x1180 to x1183 (one per step)
and x1184 (the four rebased on main, the gates' last binary). Finished on
2026-10-21 after a pause; the date is the package's start.

**1. Settled switches removed** (x1180; the router crate -350 lines).
Each restored the code before a change for A/B runs, and the changelog
section that introduced it records the measurement that settled it. Gone,
with their old code paths, the current path the only one:
`PCB_ROUTER_OLD_SPOKES`, `NO_MANUAL_SPOKES` (the fields `kicad_spokes`
and `manual_spokes` stay switchable: router.json sets them),
`NO_ISLAND_VIAS` (`island_vias`), `OLD_TRACKED` (`track_meets_spokes`),
`NO_FOREIGN_SPOKES`, `OLD_TRACK_PIECES` (`track_touch_fill`),
`NO_NECK_JOINS`, `OLD_SPOKE_FRAMES`, `NO_INNER_PADS`, `OLD_TRACK_ENDS`
(`track_end_tolerance`), `NO_INNER_SPOKES`, `OLD_TRACKS_FILL`
(`tracks_rung_fill` and `Router::hatch_any_plane`), `OLD_TRACK_RUNS`
(`track_end_junctions`), `NO_ZONE_SEAMS`, `OLD_SPOKE_CLEAR`
(`spoke_clear_to_fill`), `OLD_HOPELESS` (`unreachable_terminals`),
`OLD_RESOLVE` (`resolve_keeps_partial`), `OLD_FILL_CLEARANCE` (board.rs),
`OLD_POLYGON_DISTANCES` (grid.rs), `REROUTE_FRACTION`, and the
`jacobi_batch` option with `route_jacobi` (`PCB_JACOBI_BATCH` is still
read by src/main.rs into the pcb-kicad field, which no longer reaches the
router). `PCB_ROUTER_NO_ISLAND_STUBS` stays (a default for Florian). The
tests' "old" halves went; their current halves stay.

**2. Parked speculation removed** (x1181; -791 lines): `speculate`,
`speculate_cleanup`, `speculate_paths`, `route_batches` (now a loop over
`route_batch`), `ReadSet` and its notes in the search, the planner and the
per-search escapes loop (which only noted reads), `changed_cells`,
`mark_dirty_blocks`, `clean_up_window`, `clean_up_batch`, the speculative
part of `clean_up`, the profile line's speculation counts, and
`PCB_ROUTER_NO_SPECULATION`, `SPECULATE_PATHS`, `SPECULATION_CHECK`,
`SPECULATION_NO_TILES`, `CLEANUP_WINDOW`. The code is in e256657, which
added it (router.md, "Known limits"). Also `hard_thermal_guards` and
`stitch_in_negotiation` (off, measured worse in the 2026-10-09 sections,
unset by pcb-kicad). `pour_whole_rip_up` stays: it is on by default.

**3. Clippy** (x1182; +33 lines): pcb-router is clippy-clean
(`cargo clippy -p pcb-router --all-targets`): iterators for the indexed
loops, `is_multiple_of`, `as_chunks`, a slice for `stitching_vias`'
parent, type aliases (`FrameHook`, `SpokeCorridors`, `CellWindow`,
`SpokePad`, `RoutedItem`), `too_many_arguments` allowed on `plan_corridor`
with a reason, `neg_cmp_op_on_partial_ord` allowed where a NaN crossing
must count as not beyond.

**4. router.rs split** (x1183; pure moves, +324 lines of headers and
`use`): router.rs (15 395 lines at 54eff39, 14 291 before the split) keeps
the shared state and helpers in 920 lines; `router/` has config, result,
heap, wall, hash, profile, and `impl Router` in build, escapes, guards,
prepare, stamping, route_net, planner, search, update, batching, global,
driver, fixed, negotiate, pour_refresh, vias, finish, pour_analysis,
spokes, manual_spokes, pour_dump, stitch, cleanup, resolve, output; the
tests in router/tests.rs. Methods are `pub(super)`; the paths pcb-kicad
uses are re-exported (`router::{Config, Router, EXPANSIONS_PER_SECOND,
GUARD, wall_cuts, wall_guard, note_wall_cut, WALL_CUTS, build_totals,
congestion::tile_frame}`). router.md, "How it works", lists the files.

**Gates**, after every step: `cargo test --release -p pcb-router -p
pcb-kicad` (58 router tests, the dropped halves aside, and pcb-kicad's
365). Route mode on the v16 placements (`--route-from
build/github-layout-v16 --tasks build/github-tasks-v6/tasks.json --jobs 2
--threads 4`, `PCB_ROUTER_FINGERPRINT=1 PCB_ROUTER_NO_WALL_GUARD=1`) of
Castor, k30-SBC, jetson, Sisu, urchin and spell_tome, x1180, x1181, x1182
and x1184 (the four commits rebased on fb4b7e6) each against x1170: the
same board md5, vias, length, work seconds (1649.8, 1032.0, 418.5, 310.7,
16.8, 1.0), verdict and fingerprints on every board (`build/qb-x1170*`,
`build/qb-x118*`). `PCB_ROUTER_FINGERPRINT_ALL` at a quarter budget, two
threads, on Castor and k30-SBC, x1184 against x1170: identical
(`build/qb-fpall-*`). The starved-thermal census (census.py, fill3 KiCad
cache, 44 boards) 162 predicted / 7 missed / 26 false alarms / 2 not
modelled for x1170 and every step, the whole output identical
(`build/qb-census`). Short tier (tasks v6, 6 x 4 under the lock) x1184
against `build/short-x1170.log`: all 12 rows identical apart from CPU,
wall, load and memory (unconnected 19, new errors 2, 1134 vias, 21228 mm,
work 2501 s, passes 7, first-route open terminals 26).

Not pure clean-ups, for the reviewer: pcb-kicad's `jacobi_batch` field
(board_router.rs) and the `PCB_JACOBI_BATCH` line of src/main.rs stay but
do nothing now (the three lines that passed the field on were removed so
that pcb-kicad compiles); the verbose `profile:` line lost its speculation
counts and the `PCB_ROUTER_DEBUG` batch sizes their `j` marks; the
net-count assertion of `build` keeps its 15-bit limit, which the hard
guard's bit no longer needs.

## Committed on 2026-10-16 (quality: placer, topo, congestion)

A code-quality pass over `pcb-placer`, `pcb-topo` and `pcb-congestion`
with no behaviour change. Binary x1200 (this code; main had no Rust change
since 54eff39).

**Clippy.** `cargo clippy --release -p pcb-placer -p pcb-topo -p
pcb-congestion` reported 19 warnings in the three crates (6 + 11 + 2),
now none, also with `--all-targets` (the 26 left in that output are
`pcb-router`'s, a dependency):

- `needless_range_loop` (8): iterators with `enumerate` in
  `global.rs` (filler sides), `legal.rs` (`refine`'s bucket fill),
  `features.rs` (layer slots) and `layers.rs` (the layer DP, the
  stretches, the clusters, the stretch costs); same order, same values;
- `type_complexity` (7): aliases `Boxes` (anneal.rs), `NetPin`
  (global.rs), `Level` (lib.rs, the relaxation levels' tuple),
  `WireRoom` (layers.rs), `Ring` (realize.rs);
- `too_many_arguments` (2, `realize::pull`, `cached_crossing_cost`):
  allowed, as elsewhere in the crates;
- `chunks_exact_to_as_chunks`, `unnecessary_sort_by` (a stable
  `sort_by_key` on the same key), `manual_is_multiple_of`: as suggested;
- with `--all-targets`, `excessive_precision` in the tiny model's test:
  allowed, the expected values are PyTorch's as printed.

No NaN-sensitive comparison was among them.

**File sizes.** No `pcb-placer` file is over 2 500 lines (the largest:
legal.rs 1 713, constraints.rs 1 277, lib.rs 1 119), so nothing was split.

**Kept on purpose** (documented, off or parked), untouched:

- `shared_halos` (placer option, off): the field `Problem::shared_halos`
  (problem.rs:147), its branches in lib.rs:386, global.rs:297,
  anneal.rs:225, legal.rs:482, room.rs:186; the KiCad option in
  board_placer.rs:143;
- `anneal_far_side_gaps` (off): board_placer.rs:146 sets
  `AnnealConfig::legal_far_side`; the annealer's `far_overlap` and
  `far_courtyards` (anneal.rs:374-479, about 106 lines) and `build_state`;
- `routing_room` (off): crates/pcb-placer/src/room.rs (428 lines),
  `Config::routing_room` in lib.rs (about lines 967-1063),
  crates/pcb-kicad/src/placement_room.rs (676 lines);
- the topological engine: crates/pcb-topo (8 files, 5 730 lines), called
  from board_router.rs (around line 5159), docs/topological.md;
- the congestion network: crates/pcb-congestion/src/model.rs (190 lines),
  crates/pcb-kicad/src/congestion_rank.rs (225 lines);
  crates/pcb-congestion/src/features.rs (790 lines) also holds
  `rudy_overflow`, which the race uses.

**Identity.** Tests (`-p pcb-placer -p pcb-topo -p pcb-congestion -p
pcb-kicad`, release) pass. `place-kicad-board` on the short tier's prepared
sources (build/short-x1170, with each board's layout.json, 4 threads):
spell_tome_bottom, USB_Cable_Tester, urchin, eternal-keypad, OpenFC and
Blue_Line give byte-identical boards with x1170 and x1200.
`score-congestion` (q-ensemble.onnx) on urchin's and OpenFC's placements:
the same `open`, `overflow` and tiles. Route mode on Castor (v16
placement, quarter budget, 4 threads, `PCB_ROUTER_FINGERPRINT=1
PCB_ROUTER_FINGERPRINT_ALL=1`): the same four fingerprints in the same
order, the same routed board (md5) and row (126/126, 187 vias, 5490 mm,
work 150.4 s).

## Committed on 2026-10-16 (stamp)

`Router::stamp` made cheaper with routes and occupancy maps byte-identical
(the next target the search-core round named). Binaries: x1140 (the change
on aacdd28), x1141 (the same without a no-op, see below), x1142 (on
015676c, for the last pairs), x1143 (the commit's code on 28b9bf8, for
the short tier). Scripts and logs in the worktree
`pcb-maker-stamp` (`build/prof`, `build/bench`, `build/check`,
`build/fpall`, `build/pairs`).

**Line profile, before** (x1123's code with line tables, gperftools at
200 Hz, route mode on the v16 placements, one search thread, share of the
process's samples; kicad-cli did not run under the profiler). `stamp` is
10.5 % of Castor, 6.0 % of k30-SBC and 9.8 % of Sisu; `unstamp`,
`mask_own` and `OwnStamps::new` together 0.1-0.3 % (`route_jacobi`'s
unstamp is rare: few parallel batches). Its two phases, Castor / k30 /
Sisu:

| what | Castor | k30 | Sisu |
|---|---|---|---|
| list building: per node and querying class, every offset of four footprints (trace, via, two diagonal-block maps) tested against the mark bitset | 4.2 | 2.5 | 3.5 |
| of it: the mark test / bounds tests / loop / push | 2.5 / 0.7 / 0.4 / 0.4 | 1.6 / 0.4 / 0.3 / 0.3 | 2.1 / 0.5 / 0.4 / 0.3 |
| diff (the two rows below) | 6.2 | 3.4 | 6.2 |
| diff: old list (mark test, count down, tile claim) | 0.6 / 2.0 / 0.2 | 0.4 / 1.0 / 0.1 | 0.7 / 2.0 / 0.2 |
| diff: new list (mark test, count up, tile claim) | 0.6 / 2.2 / 0.3 | 0.4 / 1.2 / 0.2 | 0.6 / 2.2 / 0.2 |

What the lists hold (Castor, a counting build, one route): 18 000 calls,
775 M old stamps, 818 M new, 565 M of them added and about 522 M removed:
two thirds of every list changes, because the negotiation calls `stamp`
twice per reroute (`rip_up_conflicted` takes the conflicted branches'
stamps away, the stamp after the search puts the new ones in), so the
restamp diff saves little there and the count changes themselves are the
cost. The count lines are memory: Castor's 42 occupancy maps are 1 MB
each (568 x 865 cells of u16), beyond the core's 512 KB L2.

**Changed** (crates/pcb-router/src/router.rs, `Fresh`, `mark_footprint`,
`prefetch`, `Router::stamp`). Measured with bench-search as two processes
on the two hyperthreads of one core, sides swapped each repetition
(build/bench/sib.sh: user seconds, the bench lines must agree), Castor,
12 iterations, 6 repetitions; null test of x1123 against itself 1.000
(0.998-1.001):

- Leading edges (0.964 against x1123): a node one step from the node
  before (8-neighbour or the same cell), in the same class and, for the
  trace and diagonal-block maps, on the same layer, marks only the
  offsets of its footprint that the node before did not cover: `Fresh`,
  per footprint and step, the offsets `o` with `o + step` not in the
  footprint, in the footprint's order. Every skipped cell was marked by
  the node before, which applied the same footprint to the same map, so
  the list gets the same cells in the same order. A disc of radius r
  goes from about 3r^2 offsets to 2r+1 on a straight run. With it: no
  bounds test when the node is at least the footprints' reach from the
  edges, the node classes in one flat list (was a vector per branch), and
  a via through several layers at one cell stamps its via footprints
  once. (The diagonal bisectors cannot use it: the bisectors of a
  diagonal run are parallel lines that share no cell; x1140 carried that
  case, x1141 dropped it, 1.003 against x1140, identical lists.)
- Prefetch in the diff (0.982 against the step before, 16 entries
  ahead; 32 ahead against 16: 0.998): the count of the stamp 32 entries
  on is asked for early (`_mm_prefetch`; nothing off x86_64).
- One flat mark bitset for all maps instead of a vector per map: 0.998.
- Tried and dropped: tile claims without a branch and without a division
  (multiply-high by a reciprocal): 1.005; division-free with the branch:
  1.002; prefetching the mark word too: 1.004.

All three against x1123 (bench-search, the whole process including the
lattice build): Castor 0.947 (12 iterations), Sisu 0.958 (4), k30-SBC
0.975 (4), jetson 0.985 (2). bench-search spends more of its time
stamping than route mode (15.8 % of the Castor bench before, 7.6 % after).

**Line profile, after** (the change on x1123's code, same settings; a
quieter machine: load about 10 against 15-25 before, so the seconds are
not a claim): `stamp` 3.6 % of Castor (list 1.4, diff 2.2), 2.3 % of k30
(0.9, 1.4), 3.0 % of Sisu (1.6, 1.4); fingerprints identical to the
before-runs. Remaining lines: the mark test of the leading edge (0.6 on
Castor), the count tests (0.4 each way), the push (0.2).

**Byte-identical.** A check build ran the original list building beside
the new one on every call and asserted equal lists (content and order):
route mode, two threads, urchin, spell_tome, Castor, k30-SBC, Sisu and
jetson, about 185 000 calls, all equal. `PCB_ROUTER_FINGERPRINT_ALL`
(every occupancy map hashed into each fingerprint) at a quarter of the
budget, two threads, x1132 against x1141: urchin, spell_tome, Castor,
k30-SBC, Sisu and jetson identical (fingerprints and board md5,
`build/fpall/fpall.out`). Unit test `fresh_footprints_mark_as_whole_ones`
(random walks near the edges: the same list and marks with leading
edges; bisectors share no cell).

**CPU, pinned simultaneous pairs** (route mode, `--jobs 1 --threads 4`,
`PCB_MAKER_NO_CACHE=1`, `cpu_seconds`; route mode on fixed placements is
untouched by the placer and calibration commits in between, so both
bases hold for the ratio). r1, r3: the base on 0-7,16-23; r2, r4
swapped:

| board | r1 x1132 / x1141 | r2 swapped | r3 x1152 / x1142 | r4 swapped | all four |
|---|---|---|---|---|---|
| Castor | 0.946 (257.9 / 244.0) | 0.966 (277.2 / 267.9) | 0.923 (336.3 / 310.5) | 1.011 (319.6 / 323.1) | 0.961 |
| k30-SBC | 0.934 (401.8 / 375.2) | 1.007 (306.0 / 308.1) | 0.950 (536.1 / 509.5) | 1.023 (489.3 / 500.7) | 0.978 |
| Sisu | 0.969 (1185.0 / 1148.4) | 1.004 (876.7 / 880.3) | 0.925 (1151.3 / 1064.5) | 1.016 (1092.7 / 1109.8) | 0.978 |
| jetson | 0.944 (1451.5 / 1369.7) | 1.050 (1095.1 / 1150.2) | 0.911 (1452.7 / 1323.4) | 1.024 (1382.9 / 1415.8) | 0.981 |
| geomean | 0.948 | 1.006 | 0.927 | 1.018 | 0.974 |

Ratios new / old. The halves differed by about 8 points this time (the
new binary on 8-15 gained 5-7 %, on 0-7 it lost 1-2 %), more than the 3-5
of earlier rounds, so only the four together say anything: about 2.5 %
of route mode's CPU, the stamping's share falling by two thirds. Every
pair: the same board file (md5), vias, length, work seconds, verdict and
fingerprints (`PCB_ROUTER_FINGERPRINT=1`).

Short tier (tasks v6, tier default 8 x 2, under the lock) x1143 against
`build/short-x1161.log`: all 12 rows identical apart from CPU, wall,
load, memory and the designer DRC's seconds (routed, KiCad's findings,
vias, length, work seconds, first-route open terminals; the tasks path
spelled relative, the same sha256); compare.py same 12, unconnected 19
to 19, passes 7 to 7, cut 0. Its CPU (2882 to 2784 s) is not a claim:
unpaired.

`cargo test --release -p pcb-router -p pcb-kicad` passes.

Next on this path: the counts. The negotiation takes a rerouted branch's
stamps out (`rip_up_conflicted`) and puts the new route's in after the
search; two thirds of the stamps change on every call, and a branch that
comes back on the same cells pays both. Leaving the ripped stamps in
place and masking them in the search like the kept branches (`mask_own`
and `own_claimed` already do this for the kept ones) would let one diff
cancel them, but every reader of the occupancy during the search (the
tile planner, the conflict and history bookkeeping, pour checks) must see
the same values, a semantic change not attempted here.

## Committed on 2026-10-16 (order independence)

The defect: the placement, and through it the layout, depended on the
order of the footprints in the `.kicad_pcb` and on the order of each
footprint's own items, which pcbnew's save, the import and the schematic
flow all permute; two saves of one design laid out apart. Binaries x1160
(on aacdd28, the order alone), x1161 (on 015676c, the order and the exact
quarter turns: the measured one). Runs in `build/order-independence/`,
tools in `experiments/order-independence/` (`permute.py`: a project
with its footprints, and each footprint's pads and graphics, as is,
reversed or shuffled with seed 7; `compare.py`: poses by reference and
uuid, placed and routed board and copper identical or not; `run.sh`).

**Change 1, a canonical order as the board is read**
(`canonical_order::parse_board` in pcb-kicad, used by `place-kicad-board`,
the layout's outline sizing and reads of the placed board, every router
attempt (`route_kicad_board_once`, the poured copy), the congestion score
and the designer diagnostics): footprints by reference in natural order,
then uuid, then their text; in each footprint the pads by number
(natural), then position, the `fp_*` graphics by layer and content
without uuid, the footprint's zones by content; each kind keeps the
places it held. Nothing in `pcb-placer` changed: with one input order,
global placement, the annealer's draws, the legaliser, refinement, rows,
channels, net numbering, the pool's seeds and the race's probes see one
order. The boards written are in that order, so the result file is
byte-identical from any input order (a side effect: the placed and routed
boards no longer keep the input's footprint order; KiCad does not mind,
its own save sorts by uuid).

**Change 2, exact quarter turns** (`pcb_router::geometry::degrees_sin_cos`,
in the placer's pad boxes and the router's pad shapes, rotations and
rectangles): found on the orientation search's pcbnew-saved pair
(homebrew_486 from 0 degrees and from the designer's angles): with the
order alone (x1160) it placed 102 of 130 parts elsewhere, since pcbnew
writes a pad's absolute angle wrapped (relative -90 in one file, 270 in
the other; footprints at -180 or 180) and `to_radians().sin_cos()`
rounds them apart (1e-16 in a pad box); x1132 had been lucky in pcbnew's
order. With both changes the pair gives the same poses and the same
copper (971 vias, 0 unconnected).

**Three orders** (schematic tier's prepared sources; placement alone with
200 s of work and three seeds; layouts at a third, four threads; parts
elsewhere against the file as is; vias, length mm, KiCad unconnected):

| Board | x1132 placement (reversed, shuffled) | x1132 layouts (as is / reversed / shuffled) | x1161 placement | x1161 layouts, all three |
| --- | --- | --- | --- | --- |
| urchin | 20, 22 of 122 | 41, 2683, 0 / 41, 2692, 0 / 40, 2603, 0 | identical | 43, 2636, 0, byte-identical |
| coco2 | 148, 148 of 156 | 1144, 26,093, 0 / 593, 25,126, 0 / 852, 24,226, 3 | identical | 932, 27,715, 0, byte-identical |
| homebrew_486 | 104, 104 of 130 | 1008, 27,273, 0 / 1033, 27,657, 0 / 1039, 27,226, 0 | identical | 971, 26,869, 0, byte-identical |

x1160 (before the calibration) was byte-identical across the orders too
(41, 1001, 959 vias). coco2's 593-1144 vias is the order alone; the
schematic tier's coco2 rows were draws from that spread.

**Short tier** x1161 against `build/short-x1151.log` (6 x 4, the lock):
passes 6 to 7, unconnected 30 to 19, new errors 3 to 2, vias 1178 to
1134, length 21,528 to 21,228 mm, work 2327 to 2103 s, CPU 4143 to 2882
s, cut 0; better 3 (OpenFC 2 to 0 unconnected, pass; reCamera 9 to 4;
MIDAS-MK1.1 11 to 7, 246 to 212 vias), same 9, worse 0. Every placement
changed (the stack holds the designer's order): vias per row
eternal-keypad 56 to 65, USB_Keypad 106 to 111, OpenFC 154 to 159,
reCamera 68 to 76, spell_tome 8 to 9, USB_Cable_Tester 90 to 128,
Blue_Line 25 to 26, MIDAS 246 to 212, 0xCB-1337 78 to 57, urchin 40 to
42, WLED_Matrix 216 to 157 (errors 2 to 1), framework_mobo 91 to 92; a
reshuffle like the orientation change's, this time a good draw.
**Schematic tier** x1161 against a fresh x1152 base on the same tasks
(`build/sch-x1152-order-base.log`; `build/sch-x1130.log` predates the
calibration): passes 8 to 10 (OpenFC, TelemetryOnboard), unconnected 6
to 4, vias 3027 to 3184, length 91,198 to 92,388 mm, work 3164 to 2931 s.
Identical rows and placements: urchin, spell_tome, USB_Keypad,
0xcb-static, bumwings (their import was already in reference order, so
the exact quarter turns changed nothing there either). Reshuffled
placements: coco2 717 to 932 vias, homebrew_486 960 to 971, OpenFC 165
to 148 (2 to 0 unconnected), stm32_chiffre 62 to 112, MokyaLora 420 to
400. Same placer seeds, other routing: slab-pcb-left (the race's probes
9 / 185 / 4 against 8 / 9 / 12 unfinished keep another seed: 405 to 326
vias, 1 to 2 unconnected) and TelemetryOnboard (one part moved by the
layout's moves, 1 to 0 unconnected).

**Threads:** Blue_Line, spell_tome and 0xCB-1337 (short tier) at 1, 2 and
4 threads: identical rows and byte-identical boards (the calibration
agent saw the same on urchin, USB_Cable_Tester, reCamera).

Tests: `canonical_order` (natural order; footprints by reference then
uuid, other items in place, idempotent; pads and graphics from three
orders), `board_placer::order_tests` (the lowering and a whole placement
identical from a file and its reverse; both fail without the sort),
`geometry::one_angle_spelled_two_ways_has_one_sine_and_cosine`.
`cargo test --release -p pcb-kicad -p pcb-placer -p pcb-router`,
`cli_exit_codes`, `agent_walkthrough` pass.

Left: the board's top-level graphics, zones, tracks and net list stay in
the file's order (nothing measured followed them); a save that rounds
coordinates differently is another input.

## Committed on 2026-10-16 (calibration and tier parallelism)

Two measurements, one constant and one runner default. Binaries: x1150
(main 8e8b4ba plus the `profile rate:` line, for the rates), x1151 (main
aacdd28 plus the new constant: the commit's code). Scripts and logs in the
worktree `pcb-maker-calib` (`build/rate`, `build/det`, `build/par`); tier
logs `build/short-x1151*.log` in the main tree.

**`EXPANSIONS_PER_SECOND` 2.0e6 to 3.15e6.** Measured at one search
thread (`RAYON_NUM_THREADS=1`) in route mode on the v16 placements, two
boards at a time, each pinned to eight physical cores (`taskset -c 0-7`
and `8-15`), started at a load under 10 (10-17 while they ran, other
agents included). New verbose line `profile rate:` per router: the
phases' expansions over their wall seconds without the pour analyses
(what a work second stands for in the phase budgets), the pour analyses'
work and wall, and with `PCB_ROUTER_SEARCH_RATE=1` the rate inside
`search` alone (it times every search; off, a branch per search):

| board | routers | expansions | phase s | per second | inside `search` | pour work / wall |
|---|---|---|---|---|---|---|
| Castor | 1 | 611.7M | 183.4 | 3.34M | 3.81M | 29.1 / 19.4 (1.50) |
| jetson | 5 | 3555.5M | 1101.2 | 3.23M | 3.57M | 16.4 / 16.1 (1.02) |
| k30-SBC | 1 | 638.5M | 195.2 | 3.27M | 3.58M | - |
| Sisu | 5 | 2458.5M | 801.4 | 3.07M | 3.49M | 33.6 / 30.6 (1.10) |
| urchin | 1 | 56.2M | 20.3 | 2.78M | 3.17M | - |
| spell_tome | 1 | 2.3M | 1.0 | 2.27M | 3.27M | 1.6 / 1.7 (0.93) |

Median 3.15M (Sisu 3.07, jetson 3.23): the constant is 3.15e6, 1.57 times
the old one, not the 5-10 % the search-core note guessed (that was one
speed round; the 2.0e6 was an eight-core negotiation's rate from before
all four rounds, and the one-thread rates the core agent saw were bench
runs under load 14-30 with lattice builds included). A budget in seconds
now buys 1.57 times the expansions, and on the reference a work second
is a second of the phases' wall clock (it was 0.6-0.7 s). The pour model
is left: work / wall 0.93-1.10 but Castor's 1.50 (idle, its analyses ran
faster than in their loaded calibration of x1013). Unit test
`a_budget_in_seconds_buys_expansions_at_the_calibration` (a probe's limit
is its seconds at the constant; work, the profile's phase expansions and
the `profile rate:` line count the same expansions). The placement
pool's `REFERENCE_THREADS` (8) is the placement's own convention, not the
router's calibration; its comment and docs/router.md say so now.

Short tier, `--jobs 6 --threads 4` under the lock, x1151 against
`build/short-x1130.log` (x1132's tree, the coordinator's reference):
compare.py better 1, same 11, worse 0; unconnected 32 to 30, passes 6 to
6, new errors 3 to 3, vias 1130 to 1178, length 20777 to 21528 mm, work
2840 to 2327 s, CPU 3116 to 4143 s, cut 0 to 0, first-route open
terminals 59 to 52; wall 554 to 482 s (load differed). Eleven rows
identical in routed, KiCad's findings, vias, length and first-route open
terminals; their work seconds fall (eternal-keypad 177 to 118, OpenFC 584
to 394, USB_Keypad 451 to 288: the same expansions at the higher rate)
and their CPU rises where a budget ran out (reCamera 397 to 662 s, WLED
614 to 778: more search for the same budget, the same board). One row
moved, by more work per budget: MIDAS-MK1.1 149/156 to 150/156, KiCad
unconnected 13 to 11, vias 198 to 246, 3421 to 4173 mm, first-route open
22 to 15 (its first route negotiated with 1.57 times the expansions).
Wall clock: a layout now spends about its budget in wall time on an idle
core, so the wall guard (1.1 times) has much less room than before under
load; no row was cut in any run below (load 9-25), but a loaded machine
(v16's load 16-35 at five jobs of four threads) would cut rows that the
old constant's 35 % slack hid. Keep tiers at the default parallelism and
under the lock.

**Thread independence, checked.** urchin, USB_Cable_Tester and reCamera
laid out with x1151 at 1, 2 and 4 threads (`--tier short --only ...
--jobs 3`, `PCB_ROUTER_FINGERPRINT=1 PCB_ROUTER_NO_WALL_GUARD=1`):
prepared source boards, placed boards and result boards byte-identical
(md5), rows identical (work 60.7 / 119.1 / 385.2 s at every count). The
fingerprints match on urchin and USB_Cable_Tester; on reCamera at 4
threads the ladder's last 0.05 mm wave ran two rungs side by side where
1 and 2 threads ran one, and the second (fingerprints `46eced3e...`,
`27ace45f...`) was dropped unjudged at the deadline: CPU spent, nothing
judged or counted (the memory free decides how many rungs run side by
side, not which are judged). And the whole short
tier at 6 x 4, 16 x 1, 8 x 2 and 4 x 4 gave the same 12 rows.

**Tier parallelism** (x1151, short tier, one run each under the lock,
started at load 9-12, mem 21-29 GB):

| jobs x threads | wall s | CPU s | board s summed | WLED_Matrix s | cut |
|---|---|---|---|---|---|
| 16 x 1 | 448 | 3562 | 2732 | 433 | 0 |
| 8 x 2 | 401 | 3263 | 2006 | 339 | 0 |
| 4 x 4 | 570 | 3305 | 1877 | 327 | 0 |
| 6 x 4 (above) | 482 | 4143 | 2258 | 406 | 0 |

The tier is as long as its longest board, WLED_Matrix (the runner starts
it first), and a board gains from a second thread (the placement pool
runs its seeds on the search threads; the negotiation's few parallel
batches) but little from more, while 16 x 1 shares the cores so that
every board runs slower (2732 board seconds against 2006). Chosen: 8 x 2,
run.py `TIER_THREADS = 2` with physical cores / 2 jobs as the default of
every tier (quick, short, schematic) when neither `--jobs` nor
`--threads` is given; either given keeps the old defaults for the other.
`summary.json` records `parallelism` ("tier default" or "command
line") beside `jobs`, `threads`, `physical_cores`; README "How to measure"
says rows compare across thread counts while the check above holds.
`benchmarks/agent-tasks/test_run.py` (4 tests). One run per
configuration: 16 x 1 against 8 x 2 is 47 s, within what the load can
move; the quick tier (30 boards, more throughput than critical path) was
not measured and may prefer 16 x 1.

`cargo test --release -p pcb-router -p pcb-kicad` passes.

## Committed on 2026-10-15 (KiCad check work)

Item 4 of the v16 triage's ranked list, second half: KiCad's checks inside
a layout (the board kept after the moves and the polish, the final
ladder's checked attempts, the neck repair's checks and its KiCad fill)
cost 3 to 60 s of wall each uncached and under a second from the DRC
cache, and none of it was work, so the layout's end in work came at wall
times that followed the cache and the board. Binaries: x1080 (the change
on 84fcc3b, first calibration), x1081 (rebased on aacdd28, recalibrated,
budget-guard floor), x1082 (rebased on 015676c), x1083 (on 28b9bf8,
the short tier), x1084 (on 1a5e0f6, the commit's code).

**The charge** (check_work.rs; docs/router.md "KiCad's checks as work").
Every check counts a cost modelled from the board file alone, read before
the layout starts: `CHECK_SECONDS` (3.6 s) plus
`SECONDS_PER_THOUSAND_HATCH_HOLES` (1.7 s) per thousand holes of the
board's hatched copper zones (zone area on each copper layer over the
square of hatch thickness plus gap), at most 50 s. It does not depend on
whether the cache answered, so a warm run makes every decision a cold run
makes. It goes into the ladder's single work tally (`ladder_work_now`: the
ladder's budget, `spent`, `deadline_work`), into the layout's `work`
(the kept board's check, before the final ladder's deadline is taken) and
into `routed.work_seconds` (the neck repair after the ladder). The
reservation arithmetic keeps the step's own check: the narrow-signal
reserve is the step's routing reserve plus one check (`reserve_with_check`),
a rung or seed round starts only if its projected work and its check leave
the reserve whole (`fits_before_reserve(work_left - check, ...)`, the seed
rounds' `affordable`), and every router's work deadline ends one check
before the ladder's end (rungs, seeds, the probed rung's continuation, the
step: `routing_left`). The continuation's seeds multiply their routing,
not their checks (`seeds_work`). Route mode (no layout) charges nothing
and is unchanged. Reported: `layout: a KiCad check counts S s of work (N
hatched zones, H hatch holes)` at the start, `check work N x S s = T s`
on `layout: done`, `kicad_check_seconds`, `kicad_checks_charged`,
`kicad_check_work_seconds` in board-layout.json (and the last two per
ladder in board-router.json), `kicad_check_work_seconds` in the verdict
and run.py's rows. Not reserved: the first route, moves and polish are
still armed at the layout's end, so a layout that spends all of it before
the ladder ends one check past its budget in work (the check is counted,
the wall guard has its 10 %).

**Measured check times.** v16 sweep logs (x862, uncached `board check`
lines over 2 s): 191 checks on 59 boards, pooled median 9.3 s (mean
15.6); per board, the 51 boards without hatched zones 8.0 s (quartiles
6.2 / 14.2), the 8 hatched 29.8 s (4.8 to 49.4). The short tier's cold run
(build/drccache/short-cold, x801): 53 checks, pooled median 9.4 s, plain
boards 5.6 s, hatched 23.8 s. Every routed v16 board and short-tier board
checked again one at a time, uncached (`drc-kicad-board`,
PCB_MAKER_NO_CACHE=1): x978 at load 20-48, plain median 5.3 s (58 boards);
x1093 (library-free kicad-cli config) at load 14-50, plain median 3.6 s
(quartiles 2.8 / 7.2, slowest 28 s), 0.69 of x978 per board; a spot check
of 23 boards on x1132 found no change beyond the load (the same board 32 s
at load 10, 55 s at load 21). The slow plain boards are ULK (27 s), koeg
(26-33 s), ghoul (18-22 s): complex outlines, which the model does not
see. Hatch holes explain the hatched boards: least squares 1.73 s per
thousand on x1093 (1.57 on the v16 logs), correlation 0.90 between model
and measured on the nine hatched boards (Batch_1 30500 holes 60 s at load
18-36, qfwfq 14500 22 s, capybully 10600 25 s, the 0xCB panel 8100 in
four zones 29 s, WLED_Matrix 7200 14-34 s with the load, USB_Keypad 6600
9 s, stm32 chiffre 6800 on one layer 4 s, 0xCB-1337 2000 2 s). Over all
boards the model's median relative error is 0.48 (as a constant's: the
plain boards' spread), its summed absolute error 308 s against 384 s for
a constant. A first fit read only the first points of each zone polygon
(half of every rectangle's area) and gave 2.7 s per thousand; x1080 has
that, with 7 s a check (the pre-x1093 median).

**Judge, eight boards, layout mode, 4 threads, one run per side, both
sides side by side** (x978 against x1080 on 84fcc3b; rows in
build/checkwork/judge-{x978,x1080}-{cold,warm}.log). Cold is an empty cache
directory of the run's own (`PCB_MAKER_CACHE_DIR`), so every check runs
KiCad and fills it (0 hits in every cold row); warm reuses it. unconnected /
errors / vias / work / wall / cut / KiCad checks (hits, kicad-cli seconds) /
charged check work:

| board | x978 cold | x978 warm | x1080 cold | x1080 warm |
|---|---|---|---|---|
| ghoul | 6 / 0 / 385 / 1152 / 962 / no / 3 (0, 86 s) | same board, 1152 / 775 / no / 3 (3, 2 s) | 6 / 0 / 385 / 1043 / 869 / no / 3 (0, 74 s) / 21 | same board, 1043 / 640 / no / 3 (3, 2 s) / 21 |
| 0xCB panel | 118 / 0 / 248 / 1191 / 1113 / no / 4 (0, 176 s) | same, 1191 / 758 / no / 4 (4, 3 s) | 118 / 0 / 248 / 1108 / 929 / no / 4 (0, 154 s) / 116 | same, 1108 / 702 / no / 4 (4, 2 s) / 116 |
| Batch_1 | 491 / 0 / 384 / 1471 / 1158 / no / 2 (0, 114 s) | same, 1471 / 1376 / no / 2 (2, 3 s) | 491 / 0 / 384 / 1359 / 1110 / no / 2 (0, 108 s) / 100 | same, 1359 / 1380 / no / 2 (2, 3 s) / 100 |
| ULK | 4 / 0 / 130 / 1478 / 1616 / no / 7 (0, 196 s) | same, 1478 / 1285 / no / 7 (7, 5 s) | 4 / 0 / 130 / 1527 / 1496 / no / 7 (0, 218 s) / 49 | same, 1527 / 1293 / no / 7 (7, 6 s) / 49 |
| woagboard | 5 / 1 / 185 / 1540 / 1395 / no / 8 (0, 60 s) | same, 1540 / 1406 / no / 8 (8, 5 s) | 5 / 1 / 185 / 1596 / 1547 / no / 8 (0, 64 s) / 56 | same, 1596 / 1658 / deadline,layout / 8 (8, 5 s) / 56 |
| MokyaLora | 0 / 0 / 333 / 1206 / 634 / no / 3 (0, 21 s) | same, 1206 / 660 / no | 0 / 0 / 333 / 1227 / 616 / no / 3 (0, 22 s) / 21 | same, 1227 / 666 / no / 21 |
| zpn_devboard | 0 / 0 / 417 / 522 / 512 / no / 1 (0, 7 s) | same, 522 / 503 / no | 0 / 0 / 417 / 529 / 521 / no / 1 (0, 6 s) / 7 | same, 529 / 499 / no / 7 |
| Castor | 0 / 0 / 167 / 924 / 650 / no / 2 (0, 9 s) | same, 924 / 754 / no | 0 / 0 / 167 / 938 / 686 / no / 2 (0, 9 s) / 14 | same, 938 / 773 / no / 14 |

The kept board (the result's .kicad_pcb, byte for byte) is the same on all
eight boards across the four runs, and each binary's work is the same warm
and cold. Where x1080's work is lower the charge ended the ladder earlier,
dropping only attempts that were not checked and could not be kept: ghoul
one 0.075 mm skeleton rung (router count 14, cannot win), the panel the
0.075 mm skeleton and tracks rungs (unchecked), Batch_1 the seed retry and
the tracks rung after its skeleton rung (Batch_1's 50 s checks); ULK's and
woagboard's ladders ran the same attempts, the finer pitches with a little
less negotiation (woagboard's unchecked 0.05 mm rungs 79 / 90 open by the
router against 75 / 86). Cut: none, but woagboard warm on x1080: the wall
guard (1650 s) fired in its last rung and skipped the neck repair, which
had nothing to keep on this board (the same board as the cold run); that
run's search was slower than its work under the load (1658 s of wall for
1596 s of work; its cold twin 1547 s), not its checks (5 s of kicad-cli).
The checks' uncounted wall on the cold runs, summed: x978 669 s of
kicad-cli, none counted; x1080 655 s, 384 s counted (ghoul 74 / 21, panel
154 / 116, Batch_1 108 / 100, ULK 218 / 49, woagboard 64 / 56). Wall minus
work over the eight cold rows went from -1443 to -1552 s; per board it
shrank where the uncounted checks were largest (the panel -78 to -179,
ULK +138 to -31) and did not on ghoul, Batch_1 and woagboard (-190 to
-174, -314 to -249, -145 to -48), whose ladders changed and whose wall
follows the load (work above wall: seeds and probes side by side count
each router's work).

**Judge again on the 3.15e6 calibration** (x1152, main 015676c, against
x1082, the change on it; cold as above, same conditions; rows in
build/checkwork/judge-{x1152,x1082}-cold.log). A work second is now a wall
second of an idle core, so the uncounted checks are a larger share of the
slack:

| board | x1152 cold | x1082 cold |
|---|---|---|
| ghoul | 6 / 0 / 338 / 1144 / 1189 / no / 6 (0, 125 s) | same board, 1166 / 1188 / no / 6 (0, 129 s) / 22 |
| 0xCB panel | 97 / 0 / 242 / 1145 / 1075 / no / 5 (0, 133 s) | same board, 1117 / 1080 / no / 5 (0, 137 s) / 87 |
| Batch_1 | 471 / 0 / 388 / 1492 / 1752 / deadline,layout / 4 (0, 132 s) | 438 / 0 / 508 / 1461 / 1495 / no / 3 (0, 103 s) / 150 |
| ULK | 2 / 0 / 121 / 1066 / 1371 / no / 7 (0, 162 s) | same board, 1091 / 1322 / no / 7 (0, 163 s) / 25 |
| woagboard | 16 / 0 / 172 / 1196 / 1673 / deadline,layout / 7 (0, 41 s) | same board, 1113 / 1455 / no / 6 (0, 36 s) / 22 |
| MokyaLora | 0 / 0 / 341 / 1241 / 936 / no / 3 (0, 14 s) | same board, 1252 / 936 / no / 3 (0, 14 s) / 11 |
| zpn_devboard | 0 / 0 / 367 / 453 / 495 / no / 1 (0, 4 s) | same board, 457 / 498 / no / 1 (0, 4 s) / 4 |
| Castor | 0 / 0 / 207 / 610 / 559 / no / 2 (0, 4 s) | same board, 617 / 562 / no / 2 (0, 4 s) / 7 |

Cut 2 to 0. On x1152 the wall guard cut Batch_1 in its 0.05 mm rung (181
s of that router's work, three checks of 30-36 s before it uncounted) and
skipped the neck repair; x1082 counts 50 s a check there, ends the ladder
after the 0.075 mm pitch by work and keeps another board, 438 unconnected
against 471 (508 vias against 388; the cut run's board is the one that
depends on the load). woagboard: x1152's guard cut its last 0.05 mm
tracks rung and skipped the neck repair; x1082 skips one check and ends
by work, the same board. The six other boards keep their board; the work
rises by the charge (ghoul, ULK, MokyaLora, zpn, Castor); on the panel
the 0.075 mm seed retry no longer fits with its check before the reserve
and a 0.05 mm rung runs instead (unchecked, 110 open by the router), work
1145 to 1117 s. Wall minus work over the eight rows
703 to 263 s (Batch_1 261 to 34, woagboard 478 to 342, ULK 305 to 231,
ghoul 45 to 22). The warm runs of this pair were stopped for time; warm
against cold was checked on x1080 above (same boards, same work).

**Short tier** (flock, tasks v6, `--jobs 2 --threads 4`), x1081 against
build/short-x1130.log (main aacdd28, same thread count): all 12 rows
identical (unconnected 32, new errors 3, 1130 vias, 20777 mm, passes 6,
cut 0). Work 2840 to 3139 s: each row rises by its charge (3.6 s a check;
7.0 s on 0xCB-1337, 2000 hatch holes; 15.9 s on WLED_Matrix, 14.9 s on
USB_Keypad) except two whose ladders ended earlier: WLED_Matrix +72 s for
159 s charged (8 checks: the ladder stopped before the 0.075 mm pitch,
whose two rungs were not checked, 7 and 8 open by the router, and so
could not be kept), USB_Keypad +161 s for 342 s charged (21 checks,
15 of them the neck repair's after the ladder; the ladder stopped before
the 0.05 mm seed retry and tracks rung, both unchecked). KiCad checks the
same on every row (41). build/short-x1081.log.

Again on main 28b9bf8 (x1083, the change on it) against
build/short-x1161.log (x1161, same thread count): all 12 rows identical
(unconnected 19, new errors 2, 1134 vias, 21228 mm, passes 7, cut 0).
Work 2103 to 2501 s, every row by exactly its charge (3.6 s a check; 7.0
on 0xCB-1337, 15.9 on WLED_Matrix's 9 checks = 159 s, 14.9 on
USB_Keypad's 10 = 179 s); no ladder ended earlier on these placements;
KiCad checks the same on every row (35). build/short-x1083.log.

**Budget guard floor** (router.rs, `BUDGET_GUARD_FLOOR` 5 s,
`budget_guard_fires`): in route mode a phase budget is cut by the wall
clock at four times the budget, so a budget under about half a second was
cut before its first pour analysis (a second of wall on Batch_1: "budget
guard: 0 s of wall clock for a budget of 0 s of work"); the guard now
waits at least 5 s. Unit test
`a_tiny_budget_has_the_guard_floor_before_its_wall_guard`. Layouts are
unaffected (their routers have a deadline, which replaces this guard).

Unit tests: check_work.rs (the constant on a board without hatch, hatch
holes from area, layers and pitch, the cap, `checks_count_the_same_work_cached_or_not`),
`the_reserve_covers_the_steps_own_check` (board_router.rs),
`seeds_multiply_their_routing_not_their_checks` (board_layout.rs);
`cargo test --release -p pcb-kicad -p pcb-router` passes.

Not done: the model does not see what makes ULK, koeg and ghoul slow
(complex outlines, 18-33 s a check at 3.6 s charged); a term from the
outline's corner count would be the next input to try. The first route,
moves and polish do not reserve the kept board's check.

## Committed on 2026-10-15 (orientation search)

The premise was open problem 9 item 8: the import puts every part at 0
degrees, the layout tier's stack keeps the designer's angles, and "the
placement follows the orientations it starts from" (coco2: 549 vias from
the designer's angles, 1144 from 0). Measured, it does not follow them:
the placer turns parts at every stage already (global placement's
`choose_rotations`, the annealer's quarter turns, the legaliser and
refinement try every angle), and from the designer's angles x1112 left
only 25-40 % of the moved parts at them (22-35 % from 0 degrees, a quarter
by chance; ten short-tier boards, four seeds). What the start decided was
the trajectory: every part started at its angle and its options were
listed from it, so the annealer drew other turns and ties broke the other
way. coco2's gap was that chaos: on x1112 the same imported board (only
the angles changed, `experiments/orientation-search/set_angles.py`) gave
663 vias from 0 degrees and 1060 from the designer's angles, the reverse
of x1100's 1144 and 549.

**Change** (`orientation_search`, placer option, on; `core::orient`):
angle options in one order (0, 90, 180, 270; an odd-angled part keeps its
extra ones), every movable part that may turn starts at the first, turned
about its body centre; fixed parts, parts with one option (`rotation`,
`opening_outwards`, overhangs) and row leaders keep theirs. The lowering
sorts the options too, so the layout's moves walk them in that order. No
mirror: the placer never changes a part's side (no constraint lets it).
Tests `crates/pcb-placer/tests/orientation.rs`: the order, the canonical
start (movable parts only), the placement identical from two sets of
starting angles and from the same start twice (and different with the
option off), a part between two connectors ends turned by a half from
each of four starting angles.

**Stronger searches measured and dropped** (`seeds.py`, placement only,
one seed a run, four seeds, eight boards, against the canonical start):
the annealer's turn proposing the cheapest other angle by wire,
constraints and overlap (coco2 -4 %, homebrew_486 +3 %, OpenFC +4 %, the
rest within 2 %; 15 % more placement work), the global rotation choice
every 5 iterations instead of 15 (geometric mean -0.5 %), a random start
angle per part (+1 %; spell_tome +27 %), rotations from the first
iteration (no change). HPWL from the designer's angles against the
canonical start over 12 boards: geometric mean about 1 %, per board -19 %
(spell_tome) to +16 % (OpenFC, the one board whose designer angles beat
chance: its global placement is 20 % shorter from them); the seeds of
one board spread 10-20 %. The order of the footprints in the file
decides as much as the angles (homebrew_486 sorted by pcbnew's save: 103
of 130 parts elsewhere), and the router follows the order of a
footprint's graphics.

**Per board** (layouts at a third, four threads, one run a side; vias,
KiCad unconnected, weighted HPWL after placement; x1112 / x1130):

| Board | from 0 degrees | from the designer's angles | Placements |
| --- | --- | --- | --- |
| coco2 | 663, 0, 12,414 / the same | 1060, 0, 13,556 / 663, 0, 12,414 | x1130 identical from either start |
| urchin | 41, 0, 1669 / the same | 40, 0, 1692 / 41, 0, 1669 | x1130 identical |
| homebrew_486 | 981, 0, 11,959 / the same | 1196, 0, 13,494 / 1040, 1, 11,959 | x1130 placer poses identical; the race's router probes differ on the input files' graphics order (213 against 183 unfinished for seed 2), so another seed is kept |

**Short tier** x1130 against `build/short-x976.log` (`--jobs 6 --threads
4`, under the lock): every placement changed (the stack's angles no
longer count), passes 6 to 6, unconnected 35 to 32, vias 1129 to 1130,
length 22,253 to 20,777 mm, work 3236 to 2840 s, cut 0. Vias (HPWL from
x1111's placements, whose rows are x976's): USB_Cable_Tester 131 to 90
(1270 to 1279), OpenFC 178 to 154 (756 to 937), framework_mobo 108 to 91
(921 to 850), MIDAS-MK1.1 207 to 198 (2523 to 2137; unconnected 20 to
13), Blue_Line 26 to 25, 0xCB-1337 76 to 78, reCamera 56 to 68 (199 to
190; unconnected 5 to 9), USB_Keypad 78 to 106 (822 to 804), WLED_Matrix
165 to 216 (1079 to 1193); urchin, spell_tome, eternal-keypad the same
vias, other placements. A reshuffle, no cost to the layout tier.
**Schematic tier** x1130 against `build/sch-x1100.log`: 8/12 both, vias
3577 to 3578, ten rows identical; OpenFC (166 to 165) and 0xcb-static (2
to 4) move one part each in the layout's moves (x1112 places them
identically; the moves walk the options in the canonical order).

**Recommendation:** keep it on: one placement whatever the stack holds,
no cost on either tier. It does not make the import's placements better;
nothing measured here beats the seeds' noise, and OpenFC shows designer
angles can carry information the HPWL search does not find. Next, if
anyone takes it up: seed the race's starts with orientations of its own
(half the seeds canonical, half a seeded per-part choice) and judge on
seed-repeated layouts, not single runs; make the placement independent
of the footprints' order too.

Binaries x1130 (the branch on 8216bc9, measured), x1131 (rebased on
8e8b4ba). `cargo test --release -p pcb-kicad -p pcb-placer`,
`agent_walkthrough` and `cli_exit_codes` pass. Logs build/short-x1130.log,
build/sch-x1130.log; runs in build/orient-search/ (`out-*` layouts,
`seeds1`-`seeds3` placements and their tables `seeds*.txt`).

## Committed on 2026-10-15 (starter constraints for key matrices)

Open problem 8, item 4: the import recognises a key matrix from the
netlist alone and writes starter constraints, so that an agent does not
write a keyboard's hundred entries by hand.

**Inference** (`crates/pcb-kicad/src/key_matrix.rs`, `infer_key_matrix`):
- switches: footprint or symbol named `SW`, `KEY`, `MX`, `Choc`,
  `hotswap`, not of a reference class that is never a key (`D`, `LED`, `J`,
  `U`, ...: a USB receptacle `G-SWITCH`, slab's `LED_MX` key LEDs); then
  two-pin parts of any name on a row and a column (capacitive pads);
- a switch's terminals: each of its nets that is no supply (`GND`, `+5V`,
  a `power_in` pin), followed through a diode when the net between holds
  only that switch and one two-pin `D` part (not an LED): the key's own
  diode. A diode net several switches share is followed only when their
  other lines are the same (alternate footprints of one key: le_chiffre's
  1.25u and 2u on one diode), not for a column diode five keys share
  (CyberKeeb2040);
- lines: terminals two switches or more reach; a key alone on its column
  counts when the controller (a part with pins on two lines) has its other
  net (bumwings' mod key);
- rows and columns: the lines two-coloured by the switches between them,
  per connected component; rows by name (`row`, `r2` against `col`,
  `c3`), else the side with fewer nets (jiran-ble-lite's `/P0.07` pins);
  order by the number in the name where every line is named as a row or
  column and has its own number, else by the first switch's reference
  (bumwings' `col_Lp2`, `col_Lp1`, ...; a controller's pin numbers are no
  order);
- halves: two components (urchin, ULK, jiran: separate controllers), or
  one matrix whose column names say the hand (`col_L*`, `col_R*`:
  bumwings); right by name (`R_`, `_r`, `right`); a controller per half
  (most pins on its lines; one shared by both halves is neither's).

**The file** (`starter_constraints`, `constraints.starter.json`, rewritten
on every import, never `constraints.json`): the import's own starter plus
a `row` per matrix row in column order at the key pitch (Choc/PG1350 18 x
17, else 19.05, raised to a unit key's body), consecutive whole rows over
the same columns merged into a grid (`columns`, `row_pitch_mm`: the first
cut with one `row` per matrix row and a `relative` chain missed `relative
SW6 below SW1` by 16 mm on urchin, the rows stacked out of order; the grid
keeps every one), keys larger than the pitch out of the rows and `near`
their neighbour (the keys after them `relative` right of the keys
before), a row with a gap in its columns in pieces (le_chiffre's thumbs
on both hands), each row block `relative` below the one before (thumb
rows, half the longest row or less, up to 10 mm), each diode `near` its
key at 3 mm, and per half a `region` with its rows' first keys and its
controller. `comment` says what was inferred and what needs a decision
(pitch and stagger, thumbs, the controller's edge, the outline, the
switches outside). The import report's `starter`: `matrix` (row and
column nets), `switches`, `diodes`, `halves`, `halves_by`, `rows_by`,
`pitch_mm`, `unassigned`, `wide`, `alternates`, or `why` no file.

Format additions (docs/constraints.md): `region` takes `at` (`left-half`,
`right-half`, `top-half`, `bottom-half`, `center`) in place of `x`/`y`,
cut from the outline's box whatever its size (the starter's outline is
`{}`); a top-level `comment` is accepted and not read.

**Judge** (`experiments/key-matrix/judge.py`, x1121, imports as the
schematic tier does with `--footprints-from` the designer's board, then
checks the starter against the designer's board: a row's switch centres
within one key pitch of their best-fit line, in the row's order, each
`relative` below holding, each diode within its 3 mm):

| board | matrix | switches | diodes | halves | unassigned | rows on a line | in order | below | diodes within 3 mm |
| --- | ---: | ---: | ---: | --- | ---: | ---: | ---: | ---: | ---: |
| urchin | 8 x 10 | 34 | 34 | 2 (separate) | 4 | 8/8 | 8/8 | 2/2 | 33/34 |
| le_chiffre 36keys | 5 x 10 | 41 | 35 | 1 | 2 | 5/5 | 5/5 | 2/3 | 35/35 |
| le_chiffre hotswap | 5 x 10 | 36 | 34 | 1 | 1 | 3/3 | 3/3 | 1/2 | 34/34 |
| bumwings v001 | 7 x 14 | 54 | 52 | 2 (column names) | 0 | 11/11 | 11/11 | 6/7 | 52/52 |
| slab-pcb-left | - | - | - | - | 31 | | | | |
| interchange-pcb-right | - | - | - | - | 31 | | | | |
| keyboard (byrantech) | 6 x 14 | 78 | 78 | 1 | 1 | 6/6 | 6/6 | 3/3 | 78/78 |
| ErgoSNM | 5 x 7 | 35 | 35 | 1 | 6 | 2/5 | 4/5 | 0/0 | 11/35 |
| CyberKeeb2040 | 5 x 14 | 67 | 67 | 1 | 2 | 6/8 | 7/8 | 3/3 | 0/67 |
| eternal-keypad | 5 x 8 | 36 | 36 | 1 | 2 | 5/5 | 5/5 | 2/2 | 35/36 |
| ULK | 8 x 12 | 42 | 42 | 2 (separate) | 4 | 8/8 | 8/8 | 2/2 | 42/42 |
| jiran-ble-lite | 10 x 12 | 56 | 56 | 2 (separate) | 4 | 10/10 | 10/10 | 2/2 | 56/56 |
| capybully (capacitive, no diodes) | 4 x 12 | 48 | 0 | 1 | 1 | 5/6 | 6/6 | 1/1 | - |
| Quanta75 | 6 x 15 | 82 | 82 | 1 | 3 | 7/7 | 7/7 | 5/5 | 82/82 |
| poly_corne left | 4 x 6 | 21 | 21 | 1 | 2 | 4/4 | 4/4 | 1/1 | 21/21 |
| osprey | 7 x 6 | 42 | 42 | 1 | 2 | 7/7 | 7/7 | - | 42/42 |
| ghoul, USB_Keypad | - | - | - | - | 43, 9 | | | | |

The matrices are right wherever the board has one; what misses is
geometry the netlist cannot say. slab-pcb (both halves), ghoul and
USB_Keypad wire every key to its own pin (slab and ghoul through an IO
expander or shift register): no matrix, `why` says so, no file. ErgoSNM's
rows curve (up to 29 mm off a line) and its diodes sit 3.3-4.2 mm from
the key; CyberKeeb's diodes are gathered away from the keys (62 mm median)
and two of its rows are bent; capybully's thumb row is bent. Over all
boards but CyberKeeb, 521 of 547 diodes are within 3 mm of their key and
546 within 5 (3 kept, as the walkthrough had). In all, 14 of the 18
keyboards have a matrix; 87 of 93 row lines lie within a key pitch of a
line and 91 come in the row's order. urchin's left matrix is
the designer's right half (`left_is_left` false: the halves are mirrored
on the panel); the regions are a suggestion. Unassigned are the reset,
boot and power switches, an encoder, the TRRS jacks ErgoSNM names `SW`.

**Layout** (urchin, the walkthrough's fixture, the starter alone plus the
walkthrough's 230 x 95 mm outline, `"total_seconds": 30`, four threads,
load 22-31; x1121): exit 0, 42 of 42 constraints kept (2 grids, 2 thumb
rows, 34 `near`, 2 `relative`, 2 `region`), 68/68 routed, KiCad 0
unconnected and no new error, 38 vias, 1960 mm, 19.7 s of work. The
walkthrough's 100 hand-written constraints on the same binary: exit 0,
100/100, 68/68, 53 vias, 1903 mm, 7.2 s of work. The starter as written
(automatic outline): exit 0, 186 x 124 mm, 36 vias, 1902 mm.

**Schematic tier** (`--jobs 6 --threads 4`, under the lock, x1120 = the
branch on cc9c4df; the tier does not read the starter file):
`build/sch-x1120.log` against `build/sch-x1100.log`: **8/12 pass both**,
every row's fields identical (routed, vias, length, constraints, parity,
DRC errors and unconnected, `work_seconds` to the last digit) but
timing and memory (651 s wall against 684; CPU 5702 s against 5208) and
the cosmetic count in `drc_line`: KiCad's `lib_footprint_issues` and
`lib_footprint_mismatch` (spell_tome 9 + 2 against 25 + 0; never judged),
which a tier of another branch at the same time (`build/sch-x1095.log`)
shows alike: the library check's environment, not this change. No tier
with the starter applied: the tier's constraints are the designer's
placement moved onto the import (fixed parts at the designer's spot),
which the starter's rows would contradict rather than add to.

Tests: `key_matrix` (8: rows and columns in order with each key's diode,
rows as the fewer lines with pin-named nets, two matrices as halves named
right, one matrix split by column names, keys wired to their own pins,
alternate footprints and a column diode, a two-pin pad and a ground pin,
the starter's grids, thumbs, wide key, regions and its text parsing as
constraints), `placement_constraints`
`a_region_in_words_is_that_part_of_the_outline_and_a_comment_is_read_past`,
`agent_walkthrough` `the_starter_constraints_of_the_key_matrix_lay_out_the_keyboard`
(import report, file, layout from it at 230 x 95: exit 0, 15 s).
`cargo test --release -p pcb-kicad` (351), `agent_walkthrough` and
`cli_exit_codes` pass. Snapshots `build/bin/pcb-maker-x1120` (on cc9c4df,
the tier), `x1121` (rebased on 8216bc9, the judge and the urchin numbers), `x1122`
(rebased on c9fe4de: key_matrix and the starter walkthrough test pass).

Not done: thumb clusters are a straight `row` below the grid (urchin's
are turned 15 degrees on the designer's board); column stagger is not
inferred (a `row` is a straight line); the controller's edge and the USB
connector's are left to the agent; keys wired to their own pins get no
rows (their positions are not in the netlist); a key's LED (slab's
SK6812 per key) is not tied to its key.

## Committed on 2026-10-15 (search core, x1060 / x1062)

Two measurements and one change to the search proper, routes
byte-identical (x1060: the change on 8a1fd2c, what was measured; x1062:
the commit on c9fe4de). Scripts and logs in the worktree `pcb-maker-core`
(`build/scale`, `build/prof`, `build/bench`, `build/pairs`, `build/fpall`).

**Thread scaling (route mode, x1042, v16 placements, tasks v6,
`RAYON_NUM_THREADS` 1 to 16, one run each, `PCB_MAKER_NO_CACHE=1`).**
Every run of a board gave the same board file (md5) and the same
fingerprints (as a multiset: side-by-side probes print in the order they
finish), so the results are thread-independent as designed. Wall seconds
(CPU seconds):

| board | 1 | 2 | 4 | 8 | 16 |
|---|---|---|---|---|---|
| Castor | 302 (369) | 305 (381) | 223 (295) | 225 (296) | 239 (296) |
| k30-SBC | 226 (388) | 234 (396) | 229 (395) | 229 (380) | 226 (401) |
| jetson | 1114 (1744) | 1075 (1781) | 976 (1665) | 851 (1510) | 786 (1370) |

There is no speed-up to speak of: what differs follows the machine's load
(1 and 2 threads ran at load 14 to 30, jetson at 16 threads at 5 to 12),
not the threads. The proof is in the routers' own profile lines: the
negotiation runs its nets in batches, and almost all are single-net
batches, one thread by construction, whose seconds fell as much as the
wall did (Castor 3084 single batches, 132 s at 1 thread, 96 s at 16; the
561 parallel batches with 1214 nets took 6.5 s and 3.2 s of wall; jetson's
first attempt 3617 single batches 209 s to 148 s, 520 parallel 18 s to
9 s). Resolve, clean-up and the via reduction (its negotiations: single
batches again) do not scale either; KiCad's DRC is its own process. What
runs side by side are the probes (four) and the ladder's rungs (three),
std threads sized by free memory, not by `RAYON_NUM_THREADS`: so a
1-thread run still uses up to four cores while it probes (jetson: 1744 s
of CPU in 1114 s of wall). For development jobs: `--threads 1` (or 2) per
board and more jobs gives the same rows for fewer cores; 8 or 16 threads
buy nothing.

**Profile at one thread** (x1042's code with line tables, gperftools
`libprofiler.so`, 200 Hz, route mode, share of the process's samples,
Castor / jetson): the search proper 76 / 87 (`Router::search` and its
heuristic closure). Its lines:

| what | Castor | jetson |
|---|---|---|
| `KeyHeap::pop` (the sift-down's least-of-four loop and compare) | 11.2 | 14.9 |
| `occupancy_seen`: the u16 occupancy read and the own-stamp test | 8.6 | 10.3 |
| search node reads: neighbour closed, via target closed, cost compare, popped node closed | 9.5 | 12.2 |
| the neighbour's `Hot` record (trace owner, history, guard, cover) | 4.7 | 6.0 |
| heuristic closure (`points`: up to 12 target points a push) | 5.5 | 5.1 |
| bounds checks, unattributed lines | 6.8 | 7.2 |
| edge block and owner of the expanded node | 2.4 | 3.4 |
| the via step's per-layer pour cover loop | 2.4 | 0.5 |
| corridor and window tests | 1.7 | 2.5 |
| via allowed, free layers, own via ring | 2.4 | 2.0 |
| heap pushes | 1.3 | 1.4 |
| neck zone test (`zone_class`) | 0.8 | 0.3 |

Outside the search: `Router::stamp` 9.3 / 2.4 (Castor stamps for six
classes), `route_net` with the planner inlined 2.2 / 4.9, the planner's
entry cost 0.9 / 2.0, pour building 2.7 / 0.1. The layout: a
`SearchNode` is 12 bytes (cost f32, three 16-bit generation marks seen,
closed, target, parent u8) per state, states layer by layer (`layer *
cells + cell`, which is also the key's tie-break); occupancy is a dense
`u16` count per map (`Vec<Vec<u16>>`: per class a trace map per layer
and a via map, two diagonal maps per class and layer), the routed net's
own stamps subtracted through a bitset (`own_mark`) only where the count
is not zero; `Hot` is a 12-byte record per (class, layer, cell); the heap
is a four-way heap of one-word keys (estimate bits above the state).
cachegrind of a 3-iteration bench on Castor puts the search's cost in
branch mispredictions (the search's lines: 142 M of 198 M; the neighbour
closed test 21 M, the seen-or-cheaper test 11 M, the heap's sift 25 M, what
the branchless sift removed) and in L1 misses (the node records, the per-layer cover maps).

**Changed** (crates/pcb-router/src/router.rs; the expansions, their
order, every key and cost unchanged). Measured with bench-search
(`PCB_ROUTER_NO_WALL_GUARD=1`, one thread) as two processes on the two
hyperthreads of one core, sides swapped each repetition, user seconds,
median of the ratios (a null test of x1042 against itself: 1.002, spread
0.999-1.003; the machine's load otherwise made one-thread timings swing
by 15 %). Ratios against x1042, Castor (4 iterations) / jetson
(2 iterations), the whole process including the lattice build:

- The key heap keeps its four children per node as an aligned group (half
  a cache line, root at slot 3, slots past the end hold `u64::MAX`) and
  takes the least of the four without branches: 0.996 / 0.990.
- Per expansion, not per step: whether all eight neighbours lie inside the
  window, the corridor's answer when they all lie in the node's tile (77 %
  of nodes; a node outside the corridor there skips its steps),
  the neck zones' bounding box (a step asks the zones only near it); per
  cell, the via step reads one summary word of the pour cover (one net on
  a mask of layers, multiplied in the same layer order; several nets fall
  back to the per-layer maps) instead of a map a layer: together with the
  heap 0.980 / 0.979.
- The via step's target layers as a bit mask (the test against the own
  layer mispredicted) and the unrolled eight steps (one copy a direction:
  the diagonal test and the step's constants are compile-time): all
  together 0.963 / 0.957.

cachegrind of the 3-iteration Castor bench, x1042 to x1060: instructions
31.3 G to 28.1 G, mispredictions 203 M to 147 M, L1 data misses 197 M to
177 M. Tried and dropped (measured as above): a radix heap (slower: keys
are unique, so every pop split a bucket, 438 M key moves for 47 M
expansions), an 8-byte search node (cost and one tag word: 1 % slower on
both boards despite 3 % fewer L1 misses), the node array cell by cell
(a via step's layers on one line; no gain in cachegrind), the expanded
layer's occupancy maps looked up once (0.7 % slower), the heuristic over
four points at a time (more instructions: most searches have one to three
points), touching the next heap top's node early (1.000), a front slot
for the least key pushed since the last pop (1.000).

**CPU, pinned simultaneous pairs** (route mode, `--jobs 1 --threads 4`,
`PCB_MAKER_NO_CACHE=1`, x1042 against x1060, which is this change on
8a1fd2c; `cpu_seconds` of the rows; r1 x1042 on 0-7,16-23, r2 swapped):

| board | r1 x1042 / x1060 | r2 x1060 / x1042 (swapped) | both |
|---|---|---|---|
| Castor | 0.904 (299.7 / 270.8) | 0.951 (356.2 / 338.7) | 0.927 |
| k30-SBC | 0.924 (402.3 / 371.7) | 0.958 (392.3 / 375.8) | 0.941 |
| Sisu | 0.932 (1369.4 / 1276.5) | 0.929 (1352.9 / 1256.2) | 0.930 |
| jetson | 0.908 (1432.9 / 1301.5) | 0.961 (1649.8 / 1585.5) | 0.934 |
| spell_tome | 1.012 (17.2 / 17.4) | 1.079 (20.3 / 21.9) | 1.045 |
| geomean | 0.935 | 0.974 | 0.954 (big four 0.933) |

Ratios are new / old; the half a side ran on moved them by 3 to 5 points,
so the swapped pair matters. spell_tome is 1.1 s of work in 17-22 s of
CPU (KiCad's DRC). Every pair: the same board file (md5), vias, work
seconds, verdict and fingerprints (`PCB_ROUTER_FINGERPRINT=1`, as a
multiset). With `PCB_ROUTER_FINGERPRINT_ALL` (every occupancy map and pour
mask hashed into each fingerprint) at a quarter of the budget, two
threads: urchin, spell_tome, Castor, k30-SBC, Sisu and jetson identical to
x1042 (`build/fpall/fpall.out`).

**The work counter** keeps its meaning: the same expansions (work seconds
identical in every row). An expansion now costs about 5-7 % less CPU on
the big boards, so `EXPANSIONS_PER_SECOND` (2.0e6, the idle eight-core
calibration) is stale by about that much. One-thread rates on the loaded
machine (bench-search, 47 M expansions on Castor, simultaneous runs on two
cores) swung between 1.4 M and 1.9 M expansions a second for both
binaries, too loaded to recalibrate from. A recalibration to about
2.1e6 would give every budget 5-7 % more expansions for the same seconds:
every result would move (more search per budget, the same wall per
budget). Not done here.

Short tier (tasks v6, 2 jobs x 4 threads, under the lock) on x1060
against `build/short-x1041.log`: all 12 rows identical but for CPU, wall,
load and memory (routed, vias, length, work seconds, KiCad's findings,
first-route open terminals); compare.py: same 12, unconnected 35 to 35,
passes 6 to 6, cut 0. Its CPU (2756 to 3510 s) is not a claim: unpaired,
and it ran beside the fingerprint runs and other agents (load about 20).
`cargo test --release -p pcb-router -p pcb-kicad` passes (two heap tests
new: pops as a binary heap's, across clears and with keys below the last
popped).

Next on this path: `Router::stamp` (9 % of Castor at one thread, 15 % of
the bench; the footprint loop tests bounds and the mark per offset; most
offsets of consecutive nodes are already marked), and the mispredicted
data-dependent tests that remain (closed neighbour, seen-or-cheaper,
occupancy zero), which no layout change removes.

## Committed on 2026-10-15 (mask openings block placement)

The open item of the mask-bridge section: the placer let parts sit under
a fixed part's mask opening. Cause: the body's box holds a footprint's
mask graphics (`body_on_side`), but a part whose courtyard covers under
three quarters of that box blocks with its courtyard's shape and pads
(`hollow`), without them. SNSP-CPU-01's J2 (RFModulator: 12 x 5 mm
courtyard, 22 x 32 mm F.Mask polygon) and SNSP-CPU-1CHIP's RF1 are such
parts; `PCB_PLACER_DESIGNER` on the v16 layouts judged T1 and C61 (and
1CHIP's T1) legal under the opening on x1055, illegal ("shaped or hollow
boxes", J2) on x1110. Fix (crates/pcb-kicad/src/board_placer.rs,
`mask_openings`, on): the shape includes the part's mask graphics on its
side, held to the body's box (a stroke's half width past it would raise
`far_reach` on every search), so the legality test and the annealer keep
bodies and pads off it, fixed and movable parts alike. Where KiCad's mask
margin, max(web + `pad_to_mask_clearance`, `solder_mask_to_copper_clearance`
(project_rules, new placer key `solder_mask_to_copper_mm`)), exceeds the
copper clearance, the openings grow by the difference (eternal-keypad,
protoesp, USB_Cable_Tester, WLED_Matrix, USB_Keypad: none has such a
part). Same-net pads are kept out too. The designers' placements of both
SNSP boards: the same parts illegal before and after (51 and 47, none
for an opening).

Census (`experiments/mask-openings/pads_under_openings.py`: other
footprints' pads whose box meets a footprint's mask graphics, per side):
0 on all 109 designers' boards; on the 109 v16 layouts only SNSP-CPU-01
(J2: T1 4 pads, C61 2) and SNSP-CPU-1CHIP (RF1: T1 4 pads). Parts with a
courtyard and mask graphics past it: 7 on 3 boards (SNSP-CPU-01 4,
1CHIP 2, USB_Cable_Tester's battery holder 1).

Layouts, tasks v6, one run a side, jobs 1, threads 4 (x1055 = 8136361,
x1110 = that + this; build/openings-snsp-x1055, -x1110):

| board | pads under openings | solder_mask_bridge | unconnected | errors | vias | first route |
| --- | --- | --- | --- | --- | --- | --- |
| SNSP-CPU-01 | 6 / 0 | 15 / 0 | 102 / 147 | 16 / 2 (copper_edge_clearance 1 / 2) | 1812 / 1615 | 114 / 215 |
| SNSP-CPU-1CHIP | 4 / 0 | 16 / 0 | 2 / 0, passes | 16 / 0 | 757 / 678 | 9 / 6 |

SNSP-CPU-01's unconnected rose: front utilization 0.732 to 0.757 (J2's
opening), the race kept seed 3 instead of 12 (probe 308 unfinished nets
against 317, wirelength 14235 to 14556 mm), its first route left 215
open terminals against 114. One run cannot tell a reshuffle of this
crowded board from a cost of the rule (v16, x862, with the parts under
J2: 152); a few placer seeds per side would.

Identity: place-kicad-board (short-x1054's prepared sources and
layout.json, 120 s of placement work, four threads), x1055 against
x1110: byte-identical boards on spell_tome_bottom, USB_Cable_Tester,
urchin, eternal-keypad, OpenFC and Blue_Line (board-placer.json differs
only in seconds and paths). Short tier on x1111 (x1110 rebased on
2ab5ef5; tasks v6, jobs 6, threads 4, under the lock,
build/short-x1111.log) against build/short-x976.log: compare.py same
12, unconnected 35, new errors 3, vias 1129, length 22253 mm, work 3236
s, first-route 61 on both; no row field differs but `tasks` (the path's
spelling) and timing; every placed and result board byte-identical to
build/short-x1054's. Unit tests
`a_fixed_parts_mask_opening_keeps_other_parts_pads_out` (J2's geometry
at its angle, J2 fixed and movable, legality and the annealer's overlap,
the switch off) and
`a_mask_opening_keeps_kicads_mask_margin_beyond_the_copper_clearance`.
Not done: a pad's own mask expansion beyond the spacing, and the grown
margin for whole-body parts (their box holds the openings at the
spacing only).

## Committed on 2026-10-15 (schematic tier failures)

Triage of the seven failing rows of the schematic tier's x1071
(`build/sch-x1071.log`). Binaries: x1100 (the branch on f1698bf, measured),
x1101 (rebased on 8136361, main's x1055), x1102 (rebased on 2ab5ef5,
main's x1093); no tier on either. Runs, one per
side: the boards at the full budget (`--budget-scale 1`) on x1072 (main's
code: MokyaLora, slab-pcb-left) and x1100 (slab-pcb-left), the layout tier
at a third on x1100 (tasks v7: coco2, OpenFC, TelemetryOnboard;
`build/schfail-layout-third-x1100.log`), the layout tier's v16 and x903
rows.

| Board (x1071) | Mechanism | Specific to the schematic flow? | Done |
| --- | --- | --- | --- |
| urchin, copper_edge_clearance 3 | The designer's `.kicad_pro` excludes 5 pads over the edge (`copper_edge_clearance\|x\|y\|<edge id>\|<pad id>`); the tier drew the outline with ids of its own and the import gave the board's footprints new pad ids, so 3 of the 5 came back as new errors (the designer's run counts 2, the other 3 being excluded). Our edge floor and the outline's position were not it: rules (0 mm) and geometry identical. | Yes (ids) | Fixed: passes |
| le_chiffre, copper_edge_clearance 1 | The same: one excluded pad, its KiCad 7 pad `tstamp` kept by accident (the renewal only knew `uuid`), the outline's id new. | Yes | Fixed: passes |
| MokyaLora, KiCad 1 unconnected | Mtr2's shield pad (a custom pad with primitives, GND, on the back): the router never reaches it (261/262 in the layout tier too), KiCad joins it through the designer's GND pour on six layers, which the tier did not copy (it got the router's automatic In1/In4 planes). At the full budget without it (x1072): still open. | Yes (zones) | Fixed: passes |
| MokyaLora, courtyards_overlap 1 | R34 (0 ohm) and R35 (DNP) at the same spot by design, excluded by footprint ids in the project. | Yes (ids) | Fixed |
| slab-pcb-left, KiCad 2 | The budget: at the full budget it passes without the pours (x1072, 76/76, 718 s of work) and with them (x1100, 76/76, 332 vias; the layout tier v16 315). | No | Budget (x1100 at a third: 75/76, KiCad 1) |
| USB_Keypad, KiCad 2 | The layout tier's own loss at every budget: a decoupling capacitor at -1.9, 12.7 (C11 in v16, C15 in x903 and x1100), its GND pad off the hatched pour, its 3.3 V pad short of its track. | No | - |
| OpenFC, KiCad 2 | U10 (SOT-23-5 on the back): v16 81/82 at the full budget (U10 pad 5), short-x976 81/82 KiCad 2, the layout tier at a third on x1100 81/82 KiCad 2; with the designer's pours a GND island on B.Cu (zone to zone) and U10's pad 5. | No | - |
| TelemetryOnboard, KiCad 2 | The routing at the budget: the USB-C pair and U1's pins; the layout tier at a third 107/108 KiCad 2, v16 at the full budget 107/108 KiCad 1 (J1's D+ pair). | No | - |

Candidates ruled out: the generated `fixed`/`place`, `edge`, `back`/`front`
and `add` entries (the failing parts were at the designer's spots and
sides, footprints identical to the designer's; the drawn outline identical
shape for shape on urchin), drift (none on these boards but OpenFC's four,
which cost nothing since x1071).

**Fixes:**
- *The designer's DRC exclusions apply* (`netlist_import.rs`,
  `kept_uuid`): with `--footprints-from`, a symbol's own footprint (and a
  `from_board` `add`) keeps the board footprint's uuid and its pads' and
  drawings' ids, the first time that board footprint is used (a copy of
  it as a template for another part gets new ones), as KiCad's "Update
  PCB from Schematic" keeps them. Library footprints renew `tstamp` ids
  too (KiCad 6 and 7 files): every instance of one had the same ids.
  `schematic.edge_cuts` keeps each Edge.Cuts shape's id (`uuid` or
  `tstamp`). docs/agents.md says so. Test
  `footprints_from_the_board_keep_their_ids_for_the_drc_exclusions`
  (the board's ids kept for the own footprint and the add, new ones for a
  copy, the KiCad 6 library's tstamps renewed, no id twice; fails on the
  old code twice over).
- *The designer's zones* (`schematic.zones`, run.py): the designer's board
  through `strip-kicad-tracks` (what the layout tier keeps: rule areas and
  the pours covering a good part of the board, unfilled), its zones in
  KiCad 10's syntax (`(net "GND")`, `uuid`) on the imported board's names
  of their nets (`designer_matches`, shared with `additions`). Rows have
  `zones` (1 to 43 per board). A tier-generation gap: every one of the 12
  boards has pours (2-layer boards were routed without any, 4- and
  6-layer ones with the router's automatic planes) and 4 have rule areas
  (TelemetryOnboard 36, MokyaLora 10). All 12 generated boards load in
  KiCad (kicad-cli DRC exit 0, every pour's net on the board).
- *`stopped` follows the pass rule*: `routing` only on KiCad's unconnected
  items (MokyaLora passed at 261/262 with KiCad at 0 but said `routing`).
- Tests `benchmarks/agent-tasks/test_schematic.py` (outline ids, zones in
  KiCad 10's syntax on the imported nets, rule areas without a net, a
  net no pad shows keeps its name, KiCad 10 designer zones).

**Schematic tier** x1100 against `build/sch-x1071.log`, 6 jobs x 4 threads,
under the lock, load 12-32: **5/12 to 8/12** (684 s wall against 1168),
unconnected 9 to 8, new errors 5 to 0, vias 2877 to 3577, work 2247 to
3710 s, cut 0. Passes: urchin and le_chiffre (exclusions), MokyaLora (the
pour and the exclusion). Every row but spell_tome's changed, each
explained in the tier README ("x1100"): the pours add vias (coco2 661 to
1144, homebrew_486 907 to 1008, USB_Keypad 94 to 110) and work (the router
analyses pours; slab-pcb-left 74/76 to 75/76, TelemetryOnboard KiCad 2 to
3, OpenFC 80/82 to 81/82).

coco2's vias against the layout tier at a third (573 vias, 22,373 mm
against 1144 and 26,093): not the pours (GNDS vias 177 against 140) but
the orientations. The same imported coco2 board laid out again by hand
(x1100, a third, 4 threads) with every movable part stacked at the
layout tier's point gives the tier's row exactly (1144, 26,093); with the
designer's angles set on the parts (pads turned with them) 549 vias and
22,505 mm. The layout tier's stack keeps the designer's orientations, the
import puts every part at 0 degrees, and the placement follows the
orientations it starts from. Open problem 9, item 8.

**Short tier** (tasks v6, `--threads 4 --jobs 2`, under the lock) x1100
against `build/short-x976.log`: 12 of 12 rows identical (routed, vias,
length, work, KiCad; unconnected 35, new errors 3, 1129 vias, passes 6,
work 3236 s, cut 0). The import and the
schematic tier's scripts are not on the layout tier's path.

`cargo test --release -p pcb-kicad` (342 on x1102), `agent_walkthrough` and
`cli_exit_codes` pass. Logs: build/sch-x1100.log, build/schfail-full-x1072.log,
build/schfail-full-x1100.log, build/schfail-four-x1100.log (the first
four boards), build/schfail-layout-third-x1100.log, build/short-x1100.log.

## Committed on 2026-10-15 (solder mask bridges, x1050-x1054)

The premise (our vias bridge pads of other nets) does not hold: no via-pad
or via-via solder_mask_bridge in any of our outputs. Census
(`experiments/pour-fidelity/mask_bridges_census.py`: findings of a run's
drc.json by item pair against the designer's own board through
`drc-kicad-board`, with the values KiCad's rule reads from pcbnew) over
build/github-layout-v16, short-x1020, quick30-x903 and seams-x910: boards
with more bridges than the designer, or a pair kind the designer has
fewer of (build/mask-census-before.json):

| run | board | ours | designer | beyond the designer, by pair | expansion | web | mask-cu | tented |
| --- | --- | --- | --- | --- | --- | --- | --- | --- |
| v16 | OpenESC 4in1-mini | 52 | 33 | opening-track 13, opening-via 5, opening-zone 7, track-pad 2 | 0 | 0 | 0.005 | both |
| v16 | OpenESC 4in1 | 8 | 0 | track-pad 7, pad-pad 1 | 0 | 0 | 0.005 | both |
| v16 | eurorack-pmod | 1 | 2 | opening-track 1 | 0 | 0 | 0 | both |
| v16 | SNSP-CPU-01 | 20 | 5 | opening-pad 16, opening-zone 4 | 0 | 0 | 0 | both |
| v16 | SNSP-CPU-1CHIP | 16 | 0 | opening-pad 8, opening-track 4, opening-zone 4 | 0 | 0 | 0 | both |
| v16 | protoesp, laptop motherboard | 199 | 199 | pad-pad same part 2, 5 (DRC limit) | | | | |
| x910 route | OpenESC 4in1-mini | 114 | 33 | opening-pad 45, opening-track 12, opening-via 15, opening-zone 6, track-pad 6 | | | | |
| x910 route | SNSP-CPU-01 | 18 | 5 | opening-pad 15, opening-zone 3 | | | | |

No bridge beyond the designer's on the short and quick tiers. Every board
of the corpus tents its vias but SUMEC (`viasonmask yes`, with expansion,
web and mask-to-copper all 0: its vias would need to touch), so a
via-to-via mask spacing binds nowhere and was not built.

KiCad's rule (pcbnew/drc/drc_test_provider_solder_mask.cpp, 10.0.7): an
item on a mask layer (pad with that layer, untented via, track on a mask
layer, mask graphic or text) against another: shapes closer than
`solder_mask_min_width` plus both expansions; covered copper (track,
tented via, a pad without the mask layer) against an opening: closer than
`solder_mask_to_copper_clearance` plus the opening's expansion; zone fill
likewise. Same-net pairs never; a plain opening (graphic, mask-only pad)
once it exposes two nets. Expansion: pad, footprint, else the board's
`pad_to_mask_clearance`; vias the board's; texts none. KiCad draws an
outline-font text from the font, a missing face in a stand-in, not from
the render cache.

Causes, by pair: opening-pad is placement (SNSP's C63, C64, C66 or T1
and C61 placed under the fixed J2's 22 x 32 mm F.Mask polygon; 1CHIP's T1
under RF1's; the placer's report puts them there, so its mask-as-body
rule does not keep movable parts out of a fixed part's opening, for a
placement agent); opening-zone follows from it (the GND fill the designer
also has under J2 meets a second net only because the pads are there);
1CHIP's opening-track is T1's own stubs. Router causes: (1) a pad's
aperture kept the margin alone, without the project's
solder_mask_to_copper_clearance (OpenESC 4in1's J1: margin 0.102, plus
0.005; our tracks kept 0.105); the latent `solder_mask_margin` read meant
the board's expansion never counted. (2) Mask texts: 4in1-mini's "ESC"
in Tokyo, drawn by KiCad in a stand-in 2 mm wider than the cache we
routed around; eurorack's "[apf.audio]" (stroke font, bottom-anchored:
the "p" hangs 0.25 mm past the anchor, our box stopped at it), and after
that fix its "⇒⊕⊥∃⊢≢" (symbols 0.73 mm across the line of a 1 mm text).

Fix (crates/pcb-kicad/src/board_router.rs): pad apertures per mask side
keep covered copper at max(old margin rule, mask-to-copper + expansion)
and untented vias at web + pad expansion + via expansion; the board's
expansion is `pad_to_mask_clearance`; only pads with the mask layer open
it; a reach equal to the copper clearance adds no obstacle (it rerouted
USB_Cable_Tester for nothing). `vias_tented` reads KiCad 10's, KiCad 9's
and the old plot form. A cached text on a mask layer also opens the
stroke box (`mask_graphic_shapes`; the box holds all nine cached mask
texts of the corpus within 0.06 mm of KiCad's drawing); the stroke model
hangs descenders below a bottom anchor and counts non-ASCII as tall.
All of it only where solder_mask_bridge is an error (KiCad's default; 95
of 109 projects): on OpenESC (warning) the exact rule took 4in1-mini's
bridges 29 to 2 and cost 52 to 61 unconnected (4in1 104 to 106), the
min-connection precedent (`solder_mask_bridge_severity`, project_rules).

Route mode (tasks v6, jobs 1, threads 4), x978 (main) against x1054:

| board (placement) | x978 unconnected, bridges | x1054 |
| --- | --- | --- |
| SNSP-CPU-01 (v15) | 191, 18 (13 new) | identical board |
| SNSP-CPU-01 (v16) | 152, 20 | identical board |
| SNSP-CPU-1CHIP (v16) | 2, 16 | identical board |
| OpenESC 4in1-mini (v16, warning) | 52, 29 | identical board |
| OpenESC 4in1 (v16, warning) | 104, 4 | identical board |
| eurorack-pmod (v16) | 18, 1 | 13, 0 (136 vias against 132, 3131 mm against 3075) |

SNSP's 13 cannot go by routing: all 18 findings pair J2's F.Mask polygon
with a pad of C63, C64 or C66 or with the GND fill under it (and the fill
is the designer's too). x1051 (exact rule everywhere) gave the table in
the severity paragraph. Starved-thermal census (fill3 KiCad cache, 44
boards) with x1050 and x1054: 162 / 7 / 26 / 2, unchanged. Short tier
(flock, tasks v6, jobs 2, threads 4) x1054 against build/short-x1020.log:
all 12 rows identical (35 unconnected, 3 new errors, 1129 vias, 22253 mm,
first-route 61); work 2671 to 3236 s is main's accounting (x976's rows
exactly). x1053 (before the equal-reach rule) changed only
USB_Cable_Tester (131 to 105 vias, first route 0 to 1, pass both).
Unit tests: `a_pads_mask_aperture_keeps_the_mask_to_copper_clearance_and_an_untented_via_the_web`,
`at_snsps_values_a_via_by_another_nets_pad_bridges_no_mask`,
`below_an_error_the_pad_apertures_keep_the_rule_of_before`,
`via_tenting_is_read_as_kicad_10_reads_it`,
`a_cached_text_on_a_mask_layer_also_opens_the_stroke_fonts_box`,
`a_bottom_anchored_texts_descenders_hang_below_its_anchor`,
`symbols_outside_ascii_reach_as_far_as_descenders`. Logs
build/mask{,2,3}-*.log, build/short-x1053.log, build/short-x1054.log,
build/mask-census-*.json, build/mask-thermal-census-x1054.txt.

## KiCad check cost (2026-10-15)

The question: how much of an uncached KiCad check (`kicad-cli pcb drc
--format json --severity-all [--schematic-parity] --refill-zones`, 5-35 s,
about 2.2 GB) is fixed cost that a persistent checker would remove, and
can we build one? Measured, not integrated; scripts and logs in
`experiments/kicad-daemon/` (KiCad 10.0.7, four pinned CPUs, load 8-33
from other agents: walls are +-2 s, CPU seconds are steadier).

**Where the fixed cost is.** `kicad-cli version` takes 0.17 s. A DRC of an
empty board takes 3.0 s wall, 9.1 s CPU and 2.1 GB, and the same DRC with
`KICAD_CONFIG_HOME` pointing at empty `fp-lib-table`/`sym-lib-table` takes
0.4 s, 0.4 s CPU and 110 MB. The 2 GB is KiCad 10 loading every footprint
library of the global table (155 libraries, 181 MB of `.kicad_mod`):
`PCBNEW_JOBS_HANDLER::JobExportDrc` calls `FootprintLibAdapter(...)->
AsyncLoad()` and `BlockUntilLoaded()` before anything else, for the library
parity test, whose `lib_footprint_*` warnings pcb-maker never counts
(`is_library_metadata_warning`, `is_cosmetic`). The board size plays no
part. The empty `sym-lib-table` makes no difference to the DRC.

v16 layout results (`measure_cli.py`, board-only projects, so no parity;
best of two runs). The process+init column is the empty-library empty-board
run. The library column is the default call minus the empty-library call.
Refill is the empty-library call minus a run on the board KiCad filled and
saved, without `--refill-zones`. The DRC column is what is left, board load
included (0.07-0.5 s through pcbnew, see below). Writing the report takes
1-100 ms (the time between kicad-cli's `Found` and `Saved` lines):

| board | wall s | CPU s | RSS MB | process+init | library preload | refill | DRC + load | wall / RSS without libraries |
|---|---|---|---|---|---|---|---|---|
| spell_tome | 5.8 | 10.5 | 2155 | 0.4 (7%) | 1.2 (21%) | 0.8 (13%) | 3.4 (59%) | 4.6 s / 245 MB |
| Castor | 5.4 | 13.1 | 2187 | 0.4 (7%) | 3.5 (66%) | 0.7 (12%) | 0.8 (15%) | 1.8 s / 356 MB |
| WLED_Matrix | 18.3 | 29.6 | 2294 | 0.4 (2%) | 2.5 (13%) | 9.8 (54%) | 5.6 (31%) | 15.8 s / 560 MB |
| ghoul | 21.4 | 27.7 | 2228 | 0.4 (2%) | 1.4 (7%) | 3.6 (17%) | 16.0 (74%) | 20.0 s / 413 MB |
| 0xCB panel | 31.9 | 75.0 | 3003 | 0.4 (1%) | -0.5 (noise) | 28.4 (89%) | 3.6 (11%) | 32.4 s / 1695 MB |
| jetson | 8.6 | 15.3 | 2256 | 0.4 (5%) | 3.5 (41%) | 1.3 (15%) | 3.4 (40%) | 5.1 s / 510 MB |
| sum | 91.4 | 171.2 | | 2.4 (3%) | 11.7 (13%) | 44.6 (49%) | 32.8 (36%) | 79.7 s, 134.6 s CPU |

The fixed cost (process, init, library preload) is 15% of the wall over the
six boards and 21% of the CPU (36.6 of 171 s). It is above a third only on
quick checks (Castor 73%, jetson 46%). The rest is per board: the refill
(the panel's 28 s, WLED's 10 s with its teardrops) and the DRC proper
(ghoul's 16 s). A persistent process cannot cache either, because every
check is on a new board. Peak memory falls from 2.2-3.0 GB to 0.25-1.7 GB.
With the empty tables the findings are the same per type and severity on
all six boards, except `lib_footprint_issues` (more: every footprint's
library is now "not in the table") and `lib_footprint_mismatch` (gone),
both warnings. Castor's designer schematic with `--schematic-parity`: 4.5 s
/ 2.3 GB to 2.1-3.5 s / 0.46 GB, the 205 parity entries identical; parity
itself costs 0.5-0.9 s. ERC (`sch erc`, Castor) is 2.7-3.4 s / 1.3 GB
against 1.0 s / 0.3 GB, but its library warnings change
(`lib_symbol_issues` 47 to 248, `lib_symbol_mismatch` 134 to 0,
`footprint_link_issues` 124 to 158); check how ERC warnings are used before
applying the change there.

**Persistent checker options.**
- KiCad IPC API (kipy 0.8.0, API 10.0.6, read from its protobufs): only a
  running GUI KiCad with the API server enabled serves it, and Xvfb is not
  installed. Its board commands have no way to open a board file (only
  Save/Revert of the open document), `RefillZones`, `InjectDrcError` and
  `RunBoardJobExport*` (no DRC job). It cannot run a DRC or return
  findings. Not feasible.
- A pool of warm kicad-cli processes: each invocation checks one board and
  exits; there is no server mode (10.0.7: fp, jobset, pcb, sch, sym,
  version). Not feasible.
- The pcbnew Python module (SWIG): `LoadBoard`, `ZONE_FILLER(board).Fill`,
  `WriteDRCReport`. Standalone, `WriteDRCReport` segfaults in 10.0.7
  (`assert "process" failed in Pgm()`): the library parity test reaches
  `Pgm().GetLibraryManager()`, and standalone Python has no PGM.
  `ClearProject()` gets past that and crashes later. It works once
  `daemon_drc.py` builds what kicad-cli has, through ctypes on exported
  symbols: `wxInitialize` (a console wxApp), the `PGM_BASE` constructor,
  `SetPgm`, `PGM_BASE::InitPgm(headless)`. That is mangled C++ names in a
  1 MB buffer, tied to libkicommon 10.0.7.

**Prototype numbers** (`daemon_drc.py`, all six boards in one process,
`results/daemon-2026-10-15.log`). Imports and bootstrap take 0.3 s. Board
load is 0.06-0.47 s. A second DRC on the loaded board costs the same as the
first (Castor 0.91 s against 0.97 s, ghoul 18.4 s against 18.3 s), so the
engine keeps nothing worth having across runs. Per type and severity the
findings equal kicad-cli's (empty libraries) on five of six boards. WLED
differs (`connection_width` 4 against 57, `copper_sliver` 0 against 24):
kicad-cli's refill is `ZONE_FILLER_TOOL::FillAllZones`, which first
rebuilds the teardrops (`TEARDROP_MANAGER`, not in the SWIG API). On WLED's
board as KiCad saved it, teardrops included, refill and DRC match exactly.
Paired with kicad-cli under empty tables on the same CPUs (`pair.sh`,
`results/pair-2026-10-15.log`, two alternations each), a warm daemon call
against a cold process is: spell 4.5-5.0 against 6.1-7.0 s, Castor
2.2-2.3 against 2.6-2.8, jetson 5.8-8.7 against 5.5-7.9, ghoul 17.0-19.2
against 18.0-21.1, panel 27.0-27.1 against 27.2-27.4. Summed: 59.4 against
62.7 s (5%). Also:
- WriteDRCReport has no schematic parity (it passes `false`).
- Its output is a text report without item UUIDs (pcb-maker reads `uuid`
  for silk_overlap labels and pad positions from `unconnected_items`); the
  DRC items are not exposed and it adds no markers.
- RSS grows from 0.3 to 3.2 GB over the six boards and is not returned.
- The SWIG bindings have been deprecated in favour of the IPC API since KiCad 9.

**Recommendation: no daemon.** After the library fix it would save about
0.4 s per call (5%), lose parity and teardrops, and need a JSON writer that
SWIG cannot feed. Its only robust form is a C++ host built against KiCad's
own source, which is out of proportion. What a persistent process would
have removed is the library preload, and an environment variable removes
it today: run kicad-cli with `KICAD_CONFIG_HOME` set to a pcb-maker-owned
directory (for example `<cache root>/kicad-config/10.0/` with empty
`fp-lib-table` and `sym-lib-table`, created on first use), in
`run_report_command_cached` and `run_kicad_save_board`. This saves 13% of
the wall and 21% of the CPU per uncached check (8.7 CPU s on an empty board),
and 1.5-2 GB of peak memory, which lets more checks and jobs run side by
side. Size: about 60-100 lines in native_report.rs plus a test with the
fake kicad-cli.
- The DRC cache key already hashes `$KICAD_CONFIG_HOME`'s tables. It must
  take the override's, so every entry misses once.
- A project whose `.kicad_pro` sets `lib_footprint_issues` or
  `lib_footprint_mismatch` to `error` would then count one error per
  footprint. None of the 109 v16 projects does (75 warning, 6 ignore for
  issues; 69 and 12 for mismatch). Fall back to the user's configuration
  for such a project.
- A project-local `fp-lib-table` still loads its own libraries. That is the
  designer's choice and usually small.
- `kicad_common.json` is created fresh in the override directory. kicad-cli
  rewrites it anyway, and the cache leaves it out.
- ERC can take the same directory once its changed library warnings are
  shown not to count.

**Built (x1091 on 0121d54; x1092 rebased on 8136361).** `kicad_config.rs`:
kicad-cli's DRC (`run_kicad_report` for `pcb drc`, `run_kicad_save_board`)
runs with `KICAD_CONFIG_HOME=<cache root>/kicad-config`. That directory
holds `10.0/fp-lib-table` and `10.0/sym-lib-table`, both empty. They are
created on first use and rewritten if changed. The root is the DRC cache's
(`PCB_MAKER_CACHE_DIR`, `XDG_CACHE_HOME`, `HOME`) and is used even with
`PCB_MAKER_NO_CACHE=1`. The user's configuration is kept:
- with `PCB_MAKER_KICAD_USER_LIBS=1`;
- when the board's `.kicad_pro` sets a `lib_footprint_*` rule severity to
  `error`, or is not JSON;
- when the directory cannot be prepared;
- for ERC, which stays on the user's configuration until its changed
  library warnings (`lib_symbol_issues`, `lib_symbol_mismatch`,
  `footprint_link_issues`) are shown not to count.

The cache key hashes the tables of the configuration kicad-cli runs with
and the `KICAD_CONFIG_HOME` it sees, so every entry missed once after the
switch. `netlist_import.rs`'s own DRC call is unchanged. Tests:
`kicad_config` (version series, tables written and repaired, a project with
library errors keeps the user's libraries) and a fake-kicad-cli test (the
DRC sees the empty tables through `KICAD_CONFIG_HOME`; the same
configuration hits; the user's configuration is a second entry and a second
run). `cargo test --release -p pcb-kicad` (341) and `-p pcb-maker --test
cli_exit_codes` (7) pass.

Six boards, `pcb-maker drc-kicad-board` with the cache off, user against
override, best of two alternated runs on four pinned CPUs
(`judge_override.py`, `results/judge-x1091-2026-10-15.log`). Findings per
type and severity are identical on all six, except the `lib_footprint_*`
warnings:

| board | wall s | CPU s | peak RSS MB |
|---|---|---|---|
| spell_tome | 6.7 / 4.1 | 12.9 / 3.9 | 2140 / 245 |
| Castor | 4.7 / 2.2 | 11.7 / 2.9 | 2182 / 349 |
| WLED_Matrix | 18.2 / 14.8 | 30.4 / 21.1 | 2300 / 585 |
| ghoul | 20.3 / 17.8 | 26.0 / 18.9 | 2196 / 386 |
| 0xCB panel | 29.2 / 28.4 | 72.8 / 67.4 | 2951 / 1654 |
| jetson | 7.9 / 5.5 | 14.4 / 7.3 | 2275 / 508 |
| sum | 87.0 / 72.8 (-16%) | 168.2 / 121.5 (-28%) | |

x1092 on Castor: 4.6 / 1.9 s, 11.4 / 2.5 CPU s, 2180 / 356 MB, same
findings.

Short tier under the lock (`--jobs 6 --threads 4 --min-free-gb 20`, tasks
v6, load 5-25), x1091 against build/short-x976.log: all 12 rows identical
except timing and memory (unconnected 35, new errors 3, 1129 vias,
22253 mm, 6 passes, work 3236 s, cut 0). Peak RSS per row is now the
layout's own wherever KiCad set it:

| board | peak RSS MB |
|---|---|
| WLED_Matrix | 2326 to 792 |
| USB_Keypad | 2244 to 1202 |
| Blue_Line | 2325 to 2092 (its layout) |
| largest of the tier | 2.3 to 2.0 GB |

CPU is 3258 to 3327 s, but the runs differ: x976 ran on a warm DRC cache
(0.2-0.3 s designer checks) and x1091 on a cold one, since the new keys
missed everywhere (designer checks 1.5-14.8 s). The per-check CPU saving
is the six-board table above. Tier wall 1161 s.
build/short-x1091.log.

**Follow-up: ERC checked, import DRC configured (x1095 on cc9c4df, x1096
rebased on 8216bc9).**

The DRC that `netlist_import.rs` runs for the pad check now takes the
library-free configuration (`kicad_config::configure_drc`, which
`run_kicad_save_board` uses too). It reads only clearance findings.

ERC stays on the user's configuration. I ran `sch erc --format json
--severity-all` on the designer schematics of the twelve schematic-tier
boards and Castor's, user configuration against empty tables
(`judge_erc.py`, `results/erc-2026-10-15.log`):
- Every type is identical except three warnings: `lib_symbol_issues`,
  `lib_symbol_mismatch` and `footprint_link_issues`. They change on all
  thirteen.
- The ERC drops from 2.0-6.3 s / 6-13 CPU s / 1.2-1.5 GB to 0.4-2.7 s /
  0.4-2.7 CPU s / 0.26-0.37 GB.
- The condition "only `lib_*` types differ" fails on
  `footprint_link_issues`.
- More important, `erc_violations` in verification.json counts every ERC
  finding, warnings included, and `complete` needs it to be 0. With empty
  tables the totals rise: spell_tome 0 to 38, TelemetryOnboard 3 to 351,
  homebrew_486 19 to 436. spell_tome's verification would no longer be
  complete.

So moving ERC means first deciding that these three warnings are library
metadata, like `is_library_metadata_warning` for the DRC, and leaving them
out of `erc_violations`. That changes what verification.json counts (it
already depends on the libraries installed: `lib_symbol_mismatch` 240 on
coco2 under the user's configuration). It is not done.

Schematic tier under the lock (`--jobs 6 --threads 4 --min-free-gb 20`,
tasks v7, load 18-42), x1095 against build/sch-x1100.log:
- All 12 rows are identical except time, memory and the DRC cache line.
  8 pass. Objectives are the same: unconnected 8, new errors 0, 3577 vias,
  92070 mm, work 3710 s.
- Parity is identical per row (40 issues: OpenFC 5, TelemetryOnboard 16,
  slab-pcb-left 6, urchin 6, bumwings 7).
- Errors, unconnected items and warnings in every `drc_line` are the same.
  Only the cosmetic counts differ: x1100's reports had the user's
  `lib_footprint_*` warnings.
- Peak RSS: OpenFC 2246 to 1270 MB, USB_Keypad 2292 to 1280, spell_tome
  2185 to 1263, 0xcb-static 2273 to 1360, TelemetryOnboard 2263 to 1975.
  The others are their layout's.
- CPU 5208 to 5024 s.
- x1100 shows the user's library warnings, so most of this saving is the
  DRC override of 2ab5ef5 plus the pad check.

Tests: `cargo test --release -p pcb-kicad` (344) and `-p pcb-maker --test
cli_exit_codes --test agent_walkthrough` (7 and 2) pass.

**ERC without the global libraries (x1098 on c9fe4de).** Decision: ERC's
`lib_symbol_issues`, `lib_symbol_mismatch` and `footprint_link_issues`
warnings are library metadata. They depend on the libraries installed where
KiCad runs: coco2 had 240 `lib_symbol_mismatch` under the user's
configuration, which is wrong for a verdict.
- `is_erc_library_warning` and `erc_design_issues` (native_report.rs,
  exported) keep them out of verification.json's `erc_violations`. They are
  counted by type in the new `erc_library_warnings`, which defaults empty
  when reading older files.
- The same filter applies to the introduced-ERC comparisons in
  sequential_router.rs and pcb-benchmark.
- A library rule that a project rates `error` still counts, and such a
  project keeps the user's libraries (`erc.rule_severities`, as for the
  DRC).
- ERC (`sch erc` through `run_kicad_report`) now runs with the same
  `KICAD_CONFIG_HOME`. `PCB_MAKER_KICAD_USER_LIBS=1` restores the user's.
- The verification cache's policy is now
  `verify-materialized-rung-v2:erc-without-library-warnings:...`, so cached
  verifications with the old count are not reused.
- Before this change, `erc_violations` included these warnings: coco2
  counted 295 where it now counts 0.

Judge (`judge_verification.py`, `results/verification-x1098-2026-10-15.log`):
`verify-kicad-rung` with every cache off on the twelve schematic-tier
results of x1095 and on Castor's designer schematic beside its v16 board,
user configuration against empty tables:
- `erc_violations` is identical on all 13, and so is `complete` (false on
  all 13, for DRC or parity reasons).
- The library warnings appear only in `erc_library_warnings`, for example
  OpenFC {lib_symbol_issues 2, lib_symbol_mismatch 23} against
  {footprint_link_issues 86, lib_symbol_issues 303, lib_symbol_mismatch 17}.
- The only other field that differs is the DRC's `library_metadata_warnings`
  (its `lib_footprint_*` warnings).
- The whole check (ERC plus DRC) drops from 8.7-29.2 s / 20-50 CPU s /
  2.2-2.5 GB to 3.7-21.5 s / 3.4-27 CPU s / 0.3-0.8 GB.
- The walkthrough test's parity assertions pass. New unit test:
  `erc_library_warnings_are_not_design_issues`.
- `cargo test --release -p pcb-kicad` (345) and `-p pcb-maker --test
  cli_exit_codes --test agent_walkthrough` (7 and 2) pass.

Still on the user's configuration, unmeasured: the import's `sch export
netlist` (its netlist's `libraries` section may name the user's library
URIs), the preview SVG export (`verification_preview.rs`), the fab exports
(`fab_export.rs`), and the other `kicad-cli` calls in lib.rs (lines near
2881, 4407 and 13627 at c9fe4de).

## Committed on 2026-10-15 (import footprints from the board)

Open problem 9's items 1, 2 and 4: personal footprint libraries that are
not in the repository, library drift, board-only parts with nets.

**`import-kicad-netlist --footprints-from <board.kicad_pcb>`**
(`crates/pcb-kicad/src/board_footprints.rs`, `netlist_import.rs`):
- A footprint no library has is taken from that board: the symbol's own
  footprint (by symbol path, else by a reference one footprint has whose
  path is no other symbol's), else any footprint of that name. It is
  brought to library form as KiCad's "Export Footprints to Library" does:
  a back footprint flipped (`flip.rs`), pad and text angles made relative
  (a board keeps them absolute), zones moved into the footprint's frame,
  links, nets, `locked` (also KiCad 7's `fp_text reference locked`) dropped.
  It is written to `<out>/<library>.pretty` with an `fp-lib-table` entry
  (an entry of that name that resolves nowhere, a designer's own folder, is
  replaced), or to `from_board.pretty` when a library of that name exists
  but lacks it (renamed in the copied schematic). Report:
  `footprints_from_board`. A symbol without a footprint takes the board's.
- Where the board has the symbol's footprint, the board's version is used
  over the library's (`board_versions`): the board was made and checked
  with it, the library changed since (KiCad 10's ESP32 modules with the
  antenna clearance in the courtyard, redrawn hotswap sockets). Where the
  board's footprint has another name than the schematic's, the board's is
  used under its name (`substituted`, "(the board's SW0, which has another
  footprint than the schematic's)"); the copied schematic's rename takes
  the most common new name per old id (it names a footprint once per id:
  bumwings' one 1.5u key on a 1u footprint stays a parity finding).
  `drifted_footprints`: per footprint id, the library versions whose body
  (courtyard, else pads) differs from the board's by more than a fifth on a
  side, or whose pad count differs, with both sizes. Without the option
  the import is unchanged.
- Check of the inverse transform: the designer's boards with our written
  libraries in their table, KiCad's `lib_footprint_mismatch`: 29 of 2598
  footprints on 46 boards differ, each a footprint the designer edited on
  the board apart from its other copies (the library is written from the
  first one).

**`add` in constraints.json** (`KiCadAddConstraint`, docs/constraints.md
"Parts the schematic does not have"): `part`, `footprint` (a library's) or
`from_board` (the `--footprints-from` board's `part` or `board_reference`;
a repeated reference is chosen by `x`, `y` absolute), `nets` per pad,
`x`/`y`/`angle`/`origin`, `side`. The import reads the existing
`constraints.json` of the output directory and puts the parts on the board
`board_only` (report `added`); a `from_board` part without `x`/`y` stays
where that board has it, locked. The layout (`expand_additions` in
`place_kicad_board`) turns an `add` with `x`/`y` into a `place` and refuses
(exit 2) an `add` part the board lacks: "import again". Chosen over adding
in the layout because the layout reads the source board in several places
(outline sizing, net classes, the designer comparison); the import writes
the board anew anyway and keeps `constraints.json`.

**The schematic tier** imports with `--footprints-from` the designer's
board and turns the designer's footprints with nets but without a symbol
into `add` entries (`schematic.additions`: from the board, at the
designer's spot, nets renamed to the imported board's by the matched pads,
a repeated reference renamed `REF**_1`), then imports again. Rows have
`from_board`, `board_versions`, `drifted`, `parts.added`.

**Import of all 79 harvested schematics**
(`experiments/schematic-import/import_all.py`, the tier's import step and
library step without the layout; `build/schimport/x10{33,71,72}`):

| | x1033 (before) | x1071 / x1072 (`--footprints-from`, `add`) |
| --- | ---: | ---: |
| imported | 79 | 79 |
| complete (no footprint missing) | 34 | 77 |
| missing footprints (boards) | 1740 (45) | 10 (2) |
| taken from the board (`footprints_from_board`) | - | 2120 |
| board versions used | - | 11155 |
| drifted footprint ids (parts, boards) | - | 67 (453, 31) |
| designer's parts with nets the import lacks (boards) | 2970 (52) | 1253 (16) |
| added by `add` | - | 1253 |

Before, the designer's parts with nets the import lacks are mostly the
missing footprints' parts; after, 1113 of the 1253 are the two panels'
(Batch_1 402, SmartSpin2k 711: their schematic is not the panel's, it
imports no footprint, the panel's parts are all added) and 140 on 14
boards are board-only parts proper (OpenESC 4in1 47 and 4in1-mini 21
copies of parts, Quanta75's two boards 23, 0xcb-static 11, interchange 8,
Sisu 7, bumwings core 6, USB_Keypad and WLED_Matrix 4 holes each,
TelemetryOnboard 3, ...).
The 10 still missing: video's 8 (U25-U27 and others, `kicad_api_lib`, not
on the designer's board either) and bumwings core's 2 (`dao-choc-ble`, not
on its board). `unmatched_pins` 32 to 47: the board's versions lack pads of
no-connect pins (KiCad 10's footprints have them) and, on Quanta75's USB-C
receptacles, pads numbered otherwise (the import says so per part). Drift,
largest: bumwings' hotswap/solder switches (54-58 parts on six boards),
Glasgow's SOT-363 (17), OpenESC's QFNs (pad count), vaio_re's crystals and
SOT-23s, protoesp's horizontal sockets (12.3 x 13.7 against 1.7 x 11.9).

**Schematic tier** x1071 (branch on 8a1fd2c) against `build/sch-x1033.log`,
6 jobs x 4 threads both, under the lock: **5/12 pass both** (x1033 1568 s
wall, x1071 1168 s). bumwings passes (81/81, the board's 12.85 x 11.6 mm
solder footprints instead of the library's 15.9 x 12.2 mm hotswap
sockets); slab-pcb-left fails (74/76, KiCad 2: same body sizes and pads,
an older library drawing; the placement moved 53 of 129 parts); OpenFC
loses its 7 new errors (clearance 3, annular_width 4) with the board's
WSON, TF socket and USB-C, still 80/82; USB_Keypad gets its four GND
holes (37/39 as before, 94 vias against 77); 0xcb-static keeps passing with
its 11 alternative switch footprints added; TelemetryOnboard 3 to 2 KiCad
unconnected with 3 test holes added. Every row in the tier README.
Parity 31 to 40 (OpenFC 5 no-connect pins, bumwings 7 footprint names).

Tests: `board_footprints` (a turned back footprint to library form and
back, drift), `netlist_import`
`footprints_come_from_the_board_where_no_library_has_them_and_the_board_version_wins`
(missing from the board, library written and named in the table, the
board's version and drift, `add` from the board and from a library, the
layout's `expand_additions`, the input errors), `cli_exit_codes` (a
missing `--footprints-from` board exits 2). `cargo test --release -p
pcb-kicad`, `agent_walkthrough` and `cli_exit_codes` pass. Snapshots
x1070 (first cut), x1071 (the measured branch on 8a1fd2c), x1072 (rebased
on 9073633, the 79-board import identical to x1071; no tier on it).

## Committed on 2026-10-15 (hatched pour work)

Question: on main's constants (18 / 26 / 42 ns), do pour analyses on
hatched boards still run far from their modelled work? Route mode on the
v16 placements, tasks v6, one thread, `PCB_ROUTER_POUR_TRACE`, x952 (main
6099e13), all eight boards at once (load 12-24); "model / wall" is the
traced analyses' modelled work over their wall time, the profile column
sums the `profile work:` lines' pour wall and work:

| board | zones | analyses | wall s | model s | model / wall | profile pour wall / work s |
|---|---|---|---|---|---|---|
| Batch_1 | hatched | 442 | 282.5 | 105.4 | 0.37 | 167.3 / 71.0 |
| WLED_Matrix | hatched | 723 | 44.3 | 61.5 | 1.39 | 38.7 / 53.5 |
| USB_Keypad | hatched | 571 | 38.2 | 53.5 | 1.40 | 33.7 / 46.8 |
| Qfwfq | hatched | 309 | 20.9 | 19.2 | 0.92 | 20.5 / 19.0 |
| 1337-v4.0 | hatched | 452 | 14.4 | 10.2 | 0.71 | 14.1 / 10.0 |
| A13 module | hatched | 2250 | 31.2 | 41.1 | 1.32 | 26.1 / 34.8 |
| Castor | solid | 499 | 34.6 | 33.7 | 0.97 | 29.1 / 28.1 |
| Sisu | solid | 1154 | 50.2 | 51.1 | 1.02 | 40.3 / 43.3 |

The controls sit inside 0.8-1.2, the hatched boards outside on both sides
(a repeat with the trace build, x1011, load 13-32: Batch_1 0.34, WLED
1.60, Keypad 1.83, A13 1.76, 1337 0.81, Castor 0.99, Sisu 1.05). Batch_1
is still about 3 times under on main (the v16 triage's 755 / 200 s was a
four-thread layout on x862's constants). What the items-only model
misses, from trace counters per analysis (thread-local: route mode runs
two ladder rungs side by side even at one search thread, so process-wide
counters mixed them, as x1010's did):

- The probe's queries (a point tested against its index, `blocker` and
  `slack`). A query costs about a microsecond whatever its items (the 9-25
  bucket lookups, the `edges` vector, `inside`'s nearest-node and polygon
  test); a hatched probe asks many more of them, with few items each in
  open fill: a hatch hole judged (`hole_kept`, memoised per probe; 1.8-2.0
  k on WLED and Keypad) samples 64 points with `fill_at` and `near_edge`,
  up to some 2000 queries, and the ring test (`in_ring`) up to 33 per pad
  of the net (WLED 24 M queries, 400 k ring tests). Per
  item the probe's wall time was 21-34 ns on WLED and Keypad (many items
  per query) and 51-81 ns on 1337, Qfwfq and Castor; with a query term
  it is one model.
- Polygon vertices. Batch_1's queries cost 5.5 microseconds and its items
  220 ns: its board cutouts are `Hole` obstacles of up to 134 vertices
  (x1012 dump), and every shape test near them walks the polygon twice;
  15.9 G vertices in 442 analyses. Not the hatch itself, but the hatch
  multiplies the queries near them. The ring test's own pads are 0.8 G of
  those (x1014), `inside`'s polygon fallback 0.1 G: not it.
- So the hole grid and the rings cost through their queries, not as a
  term of their own; the per-zone seam counting (`in_zone_part` on
  4-point outlines) is small. The
  lattice part (fill mask, labelling, stamps) was over-counted instead:
  A13's analyses spend 17.6 s outside `kicad_spokes` for 41.1 s of node
  and stamp work (486 k stamps at 26 ns), Sisu 38.2 for 51.1, which the
  items-only fit balanced against `kicad_spokes`' uncounted rest.

Change (`fill::FillProbe` counts `queries` and the `vertices` its shape
tests go through, `Router::probe_nanos`): `PROBE_QUERY_NANOS` 850,
`PROBE_VERTEX_NANOS` 9, `PROBE_ITEM_NANOS` 42 to 9, `POUR_NODE_NANOS` 18
to 23, `POUR_STAMP_NANOS` 26 to 6. Fitted (least squares of the log
ratio, every board one weight) on the eight boards and the four other
boards of the x816 calibration (reCamera, k30-SBC, USB_Cable_Tester,
spell_tome; x1013, route mode at budget scale 0.3, load 10-17): per board
model / wall Batch_1 1.03, WLED 0.97, Keypad 0.97, Qfwfq 1.01, 1337 0.91,
A13 1.01, Castor 1.02, Sisu 0.89, reCamera 1.12, k30 1.05,
USB_Cable_Tester 1.00, spell_tome 1.41 (1.1 s of analyses; it was 1.11
with the old constants). The pour trace now prints `probe queries`,
`probe vertices`, `probe work` and whether the net has a hatched zone.
Test: `a_fill_probe_counts_its_queries_and_polygon_vertices`;
`pour_analyses_count_as_budget_work` takes the probe's whole work.

Judge, route mode, the same eight boards one run per side, x952 against
x1016 (6099e13 with this commit), one thread; the after side at load
22-24:

| board | result (both sides) | model / wall before, after | clean-ups: work (pour) of budget in wall s, before; after |
|---|---|---|---|
| Batch_1 | cut (budget guard), 499 open, 345 vias, 5180.1 mm | 0.37, 0.87 | 57.2 (19.5) of 293 in 71.5; 73.3 (35.6) of 293 in 58.7 |
| WLED_Matrix | 6 open, 143 vias, 2139.8 mm | 1.39, 0.86 | 55.1 (27.3) of 240 in 37.5; 47.9 (20.1) of 240 in 46.0 |
| USB_Keypad | 2 open, 79 vias, 1555.7 mm | 1.40, 0.86 | 93.7 (30.6) of 360 in 61.9; 83.1 (19.9) of 360 in 66.7 |
| Qfwfq | 0 open, 62 vias, 3787.2 mm | 0.92, 0.81 | 83.9 (10.8) of 240 in 72.3; 85.1 (11.8) of 240 in 87.7 |
| 1337-v4.0 | 0 open, 66 vias, 1578.7 mm | 0.71, 0.83 | 41.9 (5.4) of 240 in 35.7; 43.9 (7.5) of 240 in 43.9 |
| A13 module | 14 open, 411 vias, 6015.1 mm | 1.32, 0.96 | 99.0 (27.9) of 196 in 68.9; 86.1 (15.3) of 196 in 50.3 |
| Castor | 0 open, 262 vias, 5593.9 mm | 0.97, 0.86 | 156.8 (21.0) of 209 in 132.5; 157.8 (21.9) of 209 in 146.1 |
| Sisu | 6 open, 388 vias, 5897.0 mm | 1.02, 0.90 | 170.1 (36.1) of 189 in 131.3; 165.9 (27.8) of 189 in 107.4 |

Every board's result is identical; the analyses' counts too but Sisu's
(1154 to 1156, its work 1455.3 to 1459.5 s): the clean-ups here end on
their passes, not on their budgets, so the counted work moves (Batch_1's
pour share 19.5 to 35.6 s, the hatched 1.3-1.4 boards' down by a quarter to a half)
without moving a decision. The after side's ratios sit 0.81-0.96 at the
higher load (the spread, 0.15, is what matters: before 0.37-1.40).
Batch_1's cut is the same on both sides and not the pour work: "budget
guard: 0 s of wall clock for a budget of 0 s of work", a phase with a
budget under half a second whose first analysis runs past four times it
before the work notices (one Batch_1 analysis is about a second of wall).
Its route-mode result (107/204 routed, 499 open) is that board's v16
placement, not this change.

Short tier (tasks v6, `--jobs 2 --threads 4`, under the lock, load 15-17)
x1015 (8c20715 with this commit) against build/short-x1041.log (main's
code): 12 of 12 rows identical (vias, length, work, KiCad; unconnected 35
to 35, errors 3 to 3, passes 6 to 6, work 2671 s on both). No clean-up
budget on the tier's hatched boards (1337-v4.0, USB_Keypad, WLED_Matrix)
ends by work. Against build/short-x941.log the rows differ by main's own
changes since (neck repair, placement work), not this one.
`cargo test --release -p pcb-router -p pcb-kicad` passes. Logs:
build/hatchwork-* (x952), build/hwtrace-*, build/tl-*, build/poly-*,
build/vx-*, build/val-*, build/ring-batch1 (traces), build/after-*
(x1016), build/short-x1015.log.

## Committed on 2026-10-15 (wall guard stop, fewer layout checks)

Three items of the v16 triage ("Ranked": 2, 3, 4) and the work of an
unkept ladder (smaller hole 4). Binaries: x972 (the change on 9ce95b1, for
the layout pairs against x934), x976 (rebased on bd0babc: neck repair,
reserve floor; short tier), x977 (rebased on 8a1fd2c, the commit's code).

**Stop after the wall guard** (`WallStop`, `LayoutStep` in board_layout.rs).
Once the layout's wall-clock deadline passes, only the steps that make the
result run: labels, KiCad's check of the board kept, the report. Skipped,
each noted as a `layout` cut and logged once (`layout: past the deadline,
... skipped`): the placement race, move trials (checked before every
trial, not only every move: v16's poly_kybd_split72_left ran three trials
of 60-200 s past the guard and was killed), the resume, the polish, the
final ladder, the attempt after the narrow-signal attempt and the neck
repair (up to six KiCad checks; also skipped in the ladder past its
deadline, and its checks are counted apart in the log). In the router, a
negotiation that resumes an earlier one (resume, trial, via reduction)
starts no iteration past the deadline, and the pour stitching leaves
stranded pads instead of their hard reroute; a fresh router still
negotiates once. Without a cut nothing changes (every skip needs the wall
clock past the deadline). Checked on poly_kybd_split72_left, tasks v6,
4 threads pinned to two cores (`taskset`, beside other agents, load 14-27):
full budget x970 ended at 1719 s (harness 1800 s) with a result, 29
unconnected, 981 vias, cut deadline+layout: the guard fired in the
narrow-signal attempt's forced repair, the attempt finished and was
checked (it could win and won), the continuation was skipped, `layout:
done` at 1688 s. At `--budget-scale 0.3` (timeout 750 s, deadline 495 s)
x970 and x934 both ended at 520-525 s (cut in the first route, 368 and
483 open after it, so no moves or ladder to skip). Unit tests: a fake step
list (`past_the_wall_guard_only_the_finishing_steps_run`,
`once_fired_the_guard_stays_fired`).

**Fewer KiCad checks in the ladder** (`CheckGate` in board_router.rs;
docs/router.md "The attempt ladder"). Attempts route without KiCad's check
and are checked only when they could still win. The ladder's open count is
the router's (unconnected terminals and internal violations) plus KiCad's
unconnected items, so the router's count is a lower bound with no slack:
an attempt whose router count is above the best checked open count (or
equal, with no starved thermal on the best and no fewer vias and copper)
cannot win. The ladder's first board and any board the router finishes
are always checked; a seed retry against an unchecked rung prefers the
checked board (exact otherwise). Route mode uses this rule alone (no
decision of the ladder changes). The layout's final ladder (and the
narrow-signal attempt and its continuation) also checks only attempts
among the two best by the router's measures (open count, vias, copper),
the layout's own board counted first (`KICAD_CHECK_TOP`); a first version
ranked by the open count alone and lost MokyaLora's 333-via narrow step
(339 kept), fixed by the vias tie-break. Past the deadline an attempt is
checked only if it could beat both the ladder's best and the layout's
board (`kicad_kept`), and an unchecked ladder board is never kept. Log:
`board check: KiCad skipped, ...` per attempt, `layout: done (...); KiCad
checks N (from the DRC cache, of the neck repair), M skipped`. Unit tests
`check_gate_tests` (5).

**Unkept ladder work counts**: the layout's `routed.work_seconds` is the
ladder's work (and the continuation's) whether its board is kept or not,
as the narrow path did since 974fa2b (koeg in v16 reported 410 s with
1100 s more run). Rows' `work_seconds` rise where a ladder ran and lost;
results do not change.

**run.py threads**: the default with several jobs is the physical cores
(from /sys, else lscpu) over the jobs, at most 4, at least 1 (was the
logical CPUs: 5 jobs got 4 threads on 16 cores, now 3; 6 jobs got 4, now
2). An explicit `--threads` is kept; jobs x threads above the physical
cores is warned about. `summary.json` records `jobs`, `threads`,
`physical_cores`. A layout is deterministic per thread count: compare at
the same `--threads` (give `--threads 4` against older short and quick
tier logs).

Layout pairs, tasks v6, 4 threads, one run per side, PCB_MAKER_NO_CACHE=1,
x934 against x972 side by side, load 9-39 (rows in build/guard-ab-x934.log
and build/guard-ab-x972.log; the poly_kybd runs in build/guard-poly-*.log). KiCad checks are the DRC calls
of the layout (seconds of kicad-cli in brackets):

| board | unconnected | errors | vias | work s | wall s | cut | KiCad checks |
|---|---|---|---|---|---|---|---|
| ghoul | 6 / 6 | 0 / 0 | 385 / 385 | 1208 / 1208 | 1088 / 1079 | no / no | 7 (188 s) / 4 (88 s) |
| 0xCB panel | 109 / 109 | 0 / 0 | 231 / 231 | 362 / 362 | 1298 / 1052 | no / no | 10 (331 s) / 4 (163 s) |
| Batch_1 | 491 / 491 | 0 / 0 | 384 / 384 | 1156 / 1156 | 1262 / 1377 | no / no | 5 (203 s) / 2 (108 s) |
| ULK | 4 / 4 | 0 / 0 | 130 / 130 | 98 / 98 | 1679 / 1572 | deadline / no | 13 (370 s) / 7 (205 s) |
| woagboard | 5 / 5 | 1 / 1 | 185 / 185 | 1131 / 1540 | 1665 / 1641 | deadline / no | 12 (127 s) / 8 (70 s) |
| MokyaLora (pass) | 0 / 0 | 0 / 0 | 333 / 333 | 1365 / 1365 | 944 / 812 | no / no | 7 (61 s) / 3 (22 s) |
| zpn_devboard (pass) | 0 / 0 | 0 / 0 | 417 / 417 | 522 / 522 | 595 / 466 | no / no | 1 / 1 |
| Castor mainboard (pass) | 0 / 0 | 0 / 0 | 167 / 167 | 924 / 924 | 829 / 721 | no / no | 2 / 2 |

All eight boards identical (unconnected 615, 1 new error, 2232 vias,
47510 mm both); KiCad checks 57 to 31, kicad-cli 1297 to 676 s; CPU 14495
to 12643 s. Cut 2 to 0: on x934 the wall clock cut woagboard's and ULK's
last rung (both boards unchanged; woagboard's counted work 1131 against
1540 s is where the cut stopped that rung, ULK's 98 s leaves out its
unkept ladder, the accounting fixed above came later). Wall clock follows
the load (Batch_1 ran beside load 22-26 on x972, 9-23 on x934); the
checks saved are 100-170 s a board on the big ones.

Short tier (flock, tasks v6, `--threads 4 --jobs 1`, DRC cache warm), x976
against build/short-x1020.log (x1020 = x950's rows; x1022 changes none):
all 12 rows identical (unconnected 35, new errors 3, 1129 vias, 22253 mm,
passes 6, cut 0). Work 2671 to 3236 s: framework_mobo 220 to 292 and OpenFC
140 to 634, their unkept ladders now counted; every other row's work the
same. KiCad checks 62 to 41 (rung checks 53 to 32; the neck repair's 7 on
USB_Keypad and 2 on WLED_Matrix both sides): USB_Keypad 20 to 14,
reCamera 14 to 4, WLED_Matrix 11 to 9, OpenFC 6 to 3, the rest unchanged.
Wall 1702 to 1946 s summed over the rows (the cache made the checks
0.5 s each, so the saving is nil there; jobs 1 beside other agents' tiers
against x1020's run). build/short-x976.log.

Not done: the uncached KiCad check's time is still not work (the gate cuts
the number of checks, not their charge; done since: "KiCad check work"); the ladder's first board is always
checked (it is the ladder's measure), even where it cannot beat the
layout's board.

## Committed on 2026-10-15 (schematic tier)

The agent's real use case as a benchmark (open problem 8, item 7): the
harvested boards' schematics, and a tier that starts from them.

**Harvest.** `benchmarks/github/harvest_schematics.py` fetches, at each
board's origin commit, the root sheet and every sheet it names, the
project file and `.kicad_dru`, the library tables, the symbol libraries the
tables keep in the repository and the footprints the symbols name from the
repository's footprint libraries (Git submodules followed: 19 boards keep
their libraries in one). Libraries a table names by a designer's absolute
path (`/home/seb/dev/...`), or that lie outside the project folder, are
found in the repository by path suffix, folder or file name and recorded as
`library_fixes` (22 boards). Files go to
`benchmarks/real/external/github-sch/<board>/repo/` (gitignored), the
per-board record to `schematic.json` there and to the committed
`benchmarks/github/schematics.json`. Licences: GitHub's SPDX id where it
has one; `NOASSERTION` and none are decided by reading the licence files
at the root and along the board's folder (CERN OHL, AGPL and Hippocratic
texts GitHub does not name were allowed; no licence file at all means all
rights reserved and the board is skipped). **79 of the 109 boards have a
usable schematic** (241 sheets, 1171 files, 95 MB). Skipped: 17 without a
licence file (ISS-PCB's six, seven OnBoard boards, MegaDrive, Neotron-Pico,
SUMEC, ATAT1800, vd-rd's module), 10 without a schematic of the board's
name (PolyKybd's five molecules, the OpenESC, OpenRX and two 0xCB panels,
krishveercard, whose sheet is misspelt), 2 with a sheet missing at the
commit (the Hub, poly_corne_split42_right), 1 KiCad 5 schematic (Qfwfq).
50 of the 79 have every named footprint in KiCad's libraries or the
repository.

**Imports of all 79** (x1002, import and constraints only): 76 imported,
3 failed (osprey, hackrf, rosco_m68k: KiCad 5 arcs, below); 33 complete
after the library step (2 of them only through it), 43 with footprints
missing: personal libraries that are not in the repository (Castor's
`winterbloom` 160, DSP-ADAU1452 371, jetson's `antmicro-footprints` 121,
reCamera's `Capacitor`/`Resistor` 92, OpenMuscle's `FlexSensorFootprint`
60, ligra's `otter` 33, ...), hackrf's KiCad 5 `.mod` library (395),
footprints a KiCad library no longer has (OpenESC's `C_0402_WIDE`,
Blue_Line's `CONN3_S3B-PH_JST`), symbols without a footprint (framework
mobo's RN1). Two panels (Batch_1, SmartSpin2k) import empty: their
schematic is not the panel's.

**The tier** (`run.py --tier schematic`, `benchmarks/agent-tasks/schematic.py`,
`schematic.json`; README.md, "The schematic tier"): per board, import the
copied project with the designer's layer count, add the found libraries and
import again when footprints are missing, move tasks v7's constraints onto
the new footprints (matched by symbol path, else by reference; `fixed`,
netless and locked parts as `place` at the designer's spot; every part on
the designer's footprint layer with `back`/`front`), draw the designer's
Edge.Cuts, copy the netless board-only footprints (fixed, `board_only`),
`layout-kicad-board` at a third of the budget, `drc-kicad-board`. Pass:
the layout rule and no footprint missing; parity counted. Rows have
`stopped` (import, constraints, placement, routing, drc) and `why`.

First run on x1002 (4 jobs x 2 threads, load 9-21, `build/sch-x1002.log`):
4/12 pass in 1034 s wall; stopped at routing 6, drc 2. Passed:
spell_tome, 0xcb-static (after the library step), slab-pcb-left,
homebrew_486, each as in the layout tier. The other rows are in the README.

**Fixed in the branch** (x1030 = this branch's binary):
- *KiCad 5 library footprints*: `(module ...)` files were imported under
  that head; pcb-maker reads only `footprint`, so the parts stayed at the
  import's point, unplaced and unrouted, without a word (coco2's 16
  footprints: 87 unconnected and 199 shorts; OpenFC's SD socket and a
  sensor: 17 unconnected). The import writes `footprint`. Arcs given by
  centre and angle (`fp_arc (start c) (end s) (angle a)`) became a board
  KiCad 10 cannot load (kicad-cli exit 3, no message; the import's pad check
  failed with exit 4 on osprey, hackrf and rosco_m68k); they are converted
  to start, mid and end. Tests `kicad_5_arcs_become_three_points`,
  `kicad_5_library_footprints_become_footprints_of_today`.
- *Libraries outside the project were copied outside the output*: a
  `${KIPRJMOD}/../lib` entry was copied to `<out>/../lib` (the survey
  found four library folders written beside its output directories). Only
  libraries inside the project are copied now.
- *`back` reported missed on a board that kept it*: urchin's key sockets
  are front footprints with their pads on B.Cu; `back` flips them to B.Cu
  as asked, but the kept check compared the part's side by its pads
  (front) with "back": all 34 were reported missed (exit 1). The side to
  keep is now the one the flipped footprint's pads give, as lowered.
  `describe-kicad-board` lists `layer` (the footprint's own) beside `side`
  (where its pads are), so an agent sees which one `back` sets. Test in
  `constraints_kept_by_construction_are_reported`.

Rerun on x1030 of the boards the fixes touch (coco2, OpenFC, 0xcb-static,
spliced into the x1002 log, `build/sch-x1030-three.log`, after a
harvest refresh): **5/12 pass**. coco2 173/173 routed, KiCad 0 unconnected,
no new error, 614 vias, passed (x1002: 87 unconnected, 199 shorts; the
layout tier's v16 row 173/173 pass); OpenFC 80/82, KiCad 2 unconnected,
clearance 3 and annular_width 4 (x1002: 17 unconnected; layout tier 81/82,
1); 0xcb-static passed again, now with its library found in the project
folder on the first import (the refreshed harvest fetched the copy nearest
the project), 43 footprints substituted by name, which showed one more
gap: parity 44 (`footprint_symbol_mismatch` 43).
- *A substituted footprint kept its old name in KiCad 6 schematics*: the
  import rewrote the symbol's `Footprint` property in the copied
  schematic, but a KiCad 6 file repeats it per instance
  (`symbol_instances`, `(footprint "MX_Only:...")`), and that one wins.
  Both are rewritten now (x1031): the imported 0xcb-static's parity check
  (kicad-cli `--schematic-parity`) goes from 44 findings to the 1
  no-connect pin.

Unit tests (pcb-kicad netlist_import, placement_constraints, describe) and
`tests/agent_walkthrough.rs` pass. Snapshots `build/bin/pcb-maker-x1030`
(the rerun) and `x1031` (with the schematic rewrite); `x1032` is the
branch rebased on 8c20715 (main's x1042).
No layout tier was run on x1030: the import fixes change only boards with
KiCad 5 footprints, which no layout task has (their boards come from the
designer, written by KiCad); the side check changes the report (and so the
exit code and the pass rule) only where `back`/`front` name a footprint
whose pads are on the other side, which no task does.

## Committed on 2026-10-15 (outline polygons)

Gap 5 of the walkthrough (open problem 8): `outline` in constraints.json
took only a rectangle. Snapshots: x1040 = main at 851a473 (the base),
x1041 = this.

**What an agent can write** (docs/constraints.md, "Outlines that are not
rectangles"; agents.md's cookbook):
- `"polygon": [[x, y], ...]` (mm, closed implicitly, its points' box
  moved to `x`, `y`), checked: three corners, an area, no side crossing
  another (input errors, exit 2);
- `"corner_radius"` on the rectangle, the polygon, a piece or a cutout:
  every corner (inner ones too) an arc, the radius cut where a side is
  too short (a 30 x 4 mm outline with radius 5 is a slot with round ends);
- `"cutouts"`: boxes or polygons relative to the outline's `x`, `y`
  (`"origin": "absolute"`), inside their outline and apart from each
  other;
- `"pieces": [outline, ...]`, each with its own offset, radius and
  cutouts; they may not overlap or touch. No tabs are drawn.
A rectangle without radius or cutouts takes the old path (one gr_rect);
the automatic size and `shrink` stay rectangles (`area_factor`, `aspect`,
`shrink` with a polygon or pieces are errors; cutouts need a size).

**Writing** (`apply_shaped_outline`, placement_constraints.rs): every
contour as `gr_line` and `gr_arc` (start, mid, end; nanometre-rounded so
the pieces chain), stroke 0.05 mm, replacing the board's Edge.Cuts
graphics as before. `outline::board_loops` reads them back (the router's
and the placer's boundary, unchanged), and now also keeps the pieces'
own loops (`piece_loops`) and which cutouts the board's own graphics draw
(`board_cutouts`, as opposed to a footprint's slot).

**The placer.**
- *Edges on a polygon* (`Constraints::sides`, set by the lowering when
  the constraint's outline or one of its pieces is a polygon): each
  segment of the pieces' loops faces the side its outward normal points
  along most (`outline_sides`; 45 degrees counts as top or bottom). An
  edge constraint holds the part at the nearest segment facing its edge,
  by box-to-segment distance (`side_lines`): the line is the segment's
  innermost coordinate over the body's stretch (a slanted side: where the
  body's corner meets it), and a body not alongside the segment misses by
  the gap (`hard_violation` adds it). `edge_frame` takes the body's half
  extent for this; every caller had it. `any` measures the same way
  (`side_distance` in `choose_any_edges`). Rectangles, rounded ones, ones
  with cutouts and pieces that are all rectangles keep the box (or the
  piece boxes): nothing changes for them. On urchin's L (below)
  SW_POWER1 "top" goes to the step at y = 30; against the box it would
  have had no place in the right half.
- *Cutouts keep bodies off* (`Constraints::solid_cutouts`): cutouts the
  board's own graphics draw when the constraints give an outline (the
  outline constraint replaced all others) are tested against bodies by
  their polygon in `clear_of` (`body_meets_solid_cutout`; touching is
  allowed) and penalised by their box in the anneal; a footprint's slot
  still keeps only copper off (a connector's pegs).
- *Room*: the usable area subtracts the solid cutouts' area (the polygon's
  own area was already used); `outline_mm` and the suggestions stay boxes.
- With an outline from the constraints, a part whose centre lies off a
  polygon (in an L's notch) counts as off the board (placed, not kept).
- Pieces feed `keep_on_pieces` and the per-piece edge frames as a board
  drawn with several pieces did.

**Test boards** (urchin, the walkthrough's fixture and constraints, 30 s
of work, four threads, `PCB_ROUTER_NO_WALL_GUARD=1`): spell_tome's
schematic is not in the repo (its licence is NOASSERTION, and its
artwork footprints are 4 MB), so the test uses urchin (MIT).
- L-shaped polygon, 230 x 125 mm box less a 115 x 30 mm notch (the right
  half 30 mm lower), each half's region in its part: exit 0 in 15 s
  (18 s of work), 68/68, KiCad 0 unconnected, 0 new errors, every
  constraint kept; SW_POWER1 (edge top) at y = 30.02, the step.
- 230 x 95 mm with 5 mm round corners, a 10 x 55 mm slot between the
  halves and a triangular window: exit 0 in 23 s (30 s of work), 68/68,
  0 unconnected, 0 new errors, no body in the slot; four gr_arc.
- drc-kicad-board on both: exit 0, no `invalid_outline`, no
  `copper_edge_clearance` (only the fixture's library findings, and two
  silk_edge_clearance warnings on the rounded board).
- `"edge": "any"` on the L: chosen by the nearest facing side, kept.

**Tests**: `outline_shapes_parse_and_mistakes_are_errors`,
`outline_shapes_are_written_as_edge_cuts_lines_and_arcs`
(placement_constraints), `a_shaped_outline_gives_the_placer_its_sides_pieces_and_solid_cutouts`
(board_placer), `a_part_held_at_the_top_of_an_l_keeps_to_its_step`,
`a_cutout_the_user_cut_keeps_bodies_off` (pcb-placer tests), and
`the_keyboard_lays_out_on_an_l_and_on_a_board_with_round_corners_and_cutouts`
in tests/agent_walkthrough.rs (both layouts above, about 35 s).

**Identity** (rectangles: the tasks give no `outline`, so no new path
runs). place-kicad-board with the short tier's prepared sources and
layout.json (build/short-x1000, tasks v6, 120 s of placement work, four
threads), x1040 against x1041: byte-identical boards on all 12 short-tier
boards. Short tier on x1041 (tasks v6, 2 jobs x 4 threads, under the
lock, `build/short-x1041.log`) against build/short-x1000.log: compare.py
same 12, unconnected 35 to 35, passes 6 to 6, vias 1129, length 22253 mm,
work 2671 s, first-route open terminals 61, every row field identical
but WLED_Matrix's `native` (connection_width 9 to 0: new errors 12 to 3
in all); every layout's placed board byte-identical, every result board
too except USB_Keypad's and WLED_Matrix's, which carry the neck repair's
added tracks (2 and 12, their layout logs' "connection width" lines).
That is main's neck repair (70279ba, after x1000's base 9ce95b1), not
this change: x1000 predates it, as x941 to x950 measured (12 to 3).

## Committed on 2026-10-15 (minimum connection width)

KiCad's board setup has `min_connection` (minimum connection width) and
the project a severity for connection_width findings; the router chose
widths from the classes, the narrow-signal step's menu width and the neck
width without regard to it. Now `project_rules.rs` reads both
(`min_connection_mm`, `connection_width_severity` in the router config;
KiCad's default severity is a warning, board_design_settings.cpp) and,
where it applies, floors every track width: the class widths, the
narrow-signal width (the menu's smallest width raised to the floor), the
neck width; `route_kicad_board` and the lowering raise them again
(`apply_width_floor`, `width_floor`), so a configuration file's rules and
requested net classes keep it; stubs, escapes, manual and inner spokes
take their net's class or neck width; the neck repair's tracks already
were the minimum rounded up to a micron. main.rs's file merge copies the
floor from the project. `board-router.json` has a new `rules` object:
`width_floor_mm`, `connection_width_severity`, `narrowest_track_mm`,
`neck_width_mm` (WLED: 0.127 / error / 0.127 / 0.127).

The choice: the floor applies at error severity only, by default
(`PCB_MIN_CONNECTION_FLOOR=warning` adds warnings, `off` none). Measured
with warnings floored too (x1020), it cost routing on the only boards
where it matters (below). Census of the 109 v6 tasks
(`experiments/min-connection/census.py`, mirroring project_rules as of
x952; `emitted.py` counts the routed tracks below the minimum in a run):
18 have a `min_connection`, all with an explicit severity (3 error, 15
warning); a router width below the minimum only on two, both warnings:

| board | min_connection | severity | narrowest class | narrow step | neck | below the minimum |
| --- | ---: | --- | ---: | ---: | ---: | --- |
| OpenESC-20x20 4in1-mini | 0.16 | warning | 0.09 | - | 0.1 | Default 0.09 (every track); neck 0.1 |
| OpenESC-30x30 4in1 | 0.16 | warning | 0.09 | - | 0.1 | Default 0.09 (every track); neck 0.1 |
| OpenESC-30x30 4in1-panel | 0.16 | warning | 0.16 | - | 0.16 | - (no v16 result) |
| USB_Keypad, WLED_Matrix | 0.127 | error | 0.127 | - | 0.127 | - |
| USB_Cable_Tester | 0.1 | error | 0.1 | - | 0.1 | - |
| MokyaLora | 0.1 | warning | 0.1199 | 0.1199 | 0.1 | - |
| Sisu_SSE-9 | 0.1 | warning | 0.1 | 0.1 | 0.1 | - |
| reCamera | 0.1016 | warning | 0.1016 | - | 0.1016 | - |
| OpenFC, OpenRX panel | 0.09 | warning | 0.09 | - | 0.1 | - |
| 0xCB-1337 pcb, pcb-panel | 0.09 | warning | 0.1 | 0.2 | 0.1 | - |
| BAGEL-MK1, MK1.1-Avocado | 0.1 | warning | 0.127 | - | 0.1 | - |
| OpenAirScope | 0.1 | warning | 0.2 | - | 0.14 | - |
| MegaDrive | 0.1 | warning | 0.25 | - | 0.1 | - |
| SUMEC_MK_IV | 0.01 | warning | 0.25 | 0.25 | 0.1 | - |

On the v16 layouts only the two OpenESC boards have tracks below the
minimum (all 2530 and 2483 of them at 0.09; the designer's are 0.16 and
up, mostly 0.2). Route mode on the v16 placements, tasks v6, one run per
side, 2 jobs x 2 threads (build/minconn-route-x952, -x1020, -x1021;
connection_width counted apart, KiCad's report caps it at 199, necks.py
counts all; no other errors on any side):

| board | side | unconnected | connection_width | vias | length | work |
| --- | --- | ---: | --- | ---: | ---: | ---: |
| 4in1-mini | x952 (no floor) | 52 | 199 warnings (1007 by necks.py) | 157 | 1770 | 835 |
| 4in1-mini | x1020 (floor 0.16) | 89 | 0 | 126 | 1508 | 1172 |
| 4in1 | x952 (no floor) | 104 | 199 warnings (709) | 123 | 2073 | 903 |
| 4in1 | x1020 (floor 0.16) | 121 | 0 | 124 | 2003 | 1111 |

0.16 tracks at the class's 0.09 clearance need a lattice pitch of 0.25
against 0.18: 37 and 17 more open items for the warnings gone, so the
floor is not shipped there. x1021 (the commit, rebased on 851a473, error
severity only) gives x952's rows exactly on both. The error-severity
boards (WLED, Keypad, Cable_Tester) already route at or above their
minimum, so the floor changes nothing in their routing today; it guards
other class or narrow-step widths (a requested net class, the narrow step,
a neck) from going below.

Short tier (flock, tasks v6) x1020 (on 6099e13) against
build/short-x941.log: unconnected 35 to 35, new errors 12 to 3, vias 1129
to 1129, length 22253 mm both, passes 6 to 6, work 2671 s both, cut 0.
The changed rows are x950's neck repair, not this: WLED_Matrix
connection_width 13 to 1 (+11 to +2 new errors), USB_Keypad 4 to 2;
against build/short-x950.log all 12 rows are identical (routed, vias,
length, KiCad's counts, work, first-route open terminals). No short-tier
board has a width below its minimum (reCamera's and OpenFC's classes are
at it), so the warning choice does not move them either. Logs
build/short-x1020.log, build/minconn-route-x{952,1020,1021}.log.

Unit tests: `the_connection_width_severity_decides_the_floor`,
`the_minimum_connection_floors_the_project_widths` (project_rules),
`the_minimum_connection_floors_classes_the_narrow_width_and_the_neck`,
`a_routed_board_has_no_track_narrower_than_the_minimum_connection`
(board_router; lowers and routes a board with a forced neck-down).

Open: OpenESC's warnings (700-1000 necks of our own 0.09 tracks) remain;
a middle way would be the floor where it costs nothing (a last pass
widening tracks to 0.16 where the clearances allow, as the neck repair
does), not built.

## Committed on 2026-10-15 (panel fixes)

The two panel defects of the v16 triage (part 4). x1000 = this on
9ce95b1; x1001 = the same after the rebase on 6099e13 (necks only).

**1. Entries for a repeated reference name their footprint by uuid.**
The task generator writes an edge and rotation entry only for the copies
the designer put at an edge, in board order; x792's in-turn rule bound the
n-th entry to the n-th footprint of that reference, so the entries landed
on other copies. Now an `edge`, `rotation` or `place` entry may carry
`uuid` (the footprint's `uuid`, `tstamp` in KiCad 5-7 files;
`describe-kicad-board` lists it): it binds that footprint, a uuid no
footprint of the reference has is an error, and entries without one keep
the in-turn rule (hand-written constraints; docs/constraints.md).
`from_pcbench.py` writes `uuid` for every entry of a repeated reference.
**Tasks v7** (`build/github-tasks-v7`, the existing generator with
`--corpus` on the absolute-path copy of boards.json, x1000's
describe-kicad-board): tasks.json byte-identical to v6, 103 constraint
files identical, six differ only by uuids (the five molecules and
OpenESC's 4in1-panel, one entry). Binaries before x1000 reject v7's six
files (unknown field `uuid`); the other 103 are v6's.
Judge, every part fixed at the designer's spots (`move_all` off, the
task's constraints, place-kicad-board): missed constraints x990 on v6 /
x1000 on v7: 4x4 29 / 0 (17 of them on J1, J3, J4, the triage's count;
the rest on J5, J6), 4x5 31 / 0, 5x2_shifted 14 / 0, 5x4_wave 10 / 0,
7x5_wave_left 19 / 0. With J5, J6 and J9 free (the rest fixed): 5x4_wave
and 7x5_wave_left complete; 4x4 (J5, J6), 4x5 (J5, J6) and 5x2 (J5)
still find no legal place for some back connectors. That is geometry, not
binding: at the designer's poses (`PCB_PLACER_DESIGNER`) neighbouring
tiles' J5 and J6 lie 0.03 mm apart on the back (centres 2.03 mm, pad
bodies 2 mm wide) and the edge tiles' J5 0.016 mm from the board edge
(the copper-to-edge floor is 0.02 mm).

**2. keep_copies regions follow the copies' layout, and copies start in
their own region.** SmartSpin2k's 57 unplaced parts were the rule's, not
only the start's: its fixed parts sit at the top of each copy (2.3 mm
from the board's top edge, 40.5 mm from its bottom), so the cut halfway
between two copies' fixed parts ran through a copy's own parts: 162 of
the designer's 630 movable parts lay outside their copy's region
(the 3 x 3 copies got rows 50.7, 65.3 and 88.9 mm high). The cut now
moves from halfway by half the difference of the board's margins beyond
all copies' fixed parts, clamped between the two copies (each copy's
region lies around its fixed parts as the board lies around all of
them): 0 of 630 outside, rows of 67.6, 65.3 and 72 mm. The 0xCB pcb-panel's
regions do not change (0.1 mm display), the 0xCB panel's cut moves from
82.7 to 82.0 (the board's middle). In addition each confined part starts
global placement at its region's centre (`Constraints::starts`, read by
`global_place`; parts without one start as before). place-kicad-board on
v16's prepared SmartSpin2k (4 threads): x990 57 parts without a legal place; the shifted
regions alone complete (final wirelength 14217 mm); shifted regions and
starts complete (12219 mm). Only boards where keep_copies fires change:
by a replica of its test over the 109 tasks of v7, SmartSpin2k and the
two 0xCB panels.

**Layouts, one run each** (four threads, jobs of two, load 12-30; tasks
v6 for x990, v7 for x1000; placed = movable parts with a legal place):

| board | before | x1000 |
| --- | --- | --- |
| SmartSpin2k_Panelized | x990: 57 parts without a legal place (exit 1, 185 s) | 630/630 placed, 568/585 routed, KiCad 19 unconnected, 0 new errors (the designer's 199 invalid_outline subtracted), 1036 vias, 1218 s of work, not cut, 924 s |
| poly_kb_molecule_4x4 | v16: 4 unplaced, 8 illegal (J5, J6, SW1) | 157/160 (J5, J6 twice), no constraint missed |
| poly_kb_molecule_4x5 | v16: 6 unplaced, 17 illegal | 190/196 (J3, J5 three times, J6 twice, SW1) |
| poly_kb_molecule_5x2_shifted | v16: 2 unplaced (J5, J6) | 129/130 (J5) |
| poly_kb_molecule_5x4_wave | v16: placed, 2/28 routed, 499 unconnected, 5 starved | placed, 3/28 routed, 499 unconnected, 6 starved_thermal, 501 vias, 937 s |
| poly_kb_molecule_7x5_wave_left | v16: 1 unplaced (J9) | 234/235 (J9) |
| OpenESC 4in1-panel (information) | v16: 207 parts (load 54) | 199 parts without a legal place: crowding (one net group, no copies) |

The molecules now place the designer's fixed parts' edges right, but
from the stack the back connectors still need the designer's 0.03 mm
gaps (above); the next step there is pad-level spacing for same-net
pads of neighbouring parts on one side, not constraints. 5x4_wave's 499
unconnected are its routing (25 of 28 connections open in both).

**Identity.** place-kicad-board with the short tier's prepared sources
and layout.json (build/short-x941, tasks v6), x990 against x1000:
byte-identical boards on spell_tome_bottom, USB_Cable_Tester, urchin,
eternal-keypad, OpenFC and Blue_Line. Short tier on x1000 (tasks v6, 2
jobs x 4 threads, under the lock, `build/short-x1000.log`) against
build/short-x941.log: compare.py same 12, unconnected 35 to 35, passes 6
to 6, new errors 12, vias 1129, length 22253 mm, work 2671 s, first-route
open terminals 61; every layout's placed board and result board
byte-identical. The only row field that differs is `constraints` on
MIDAS-MK1.1 (3 to 6), OpenFC (25 to 50) and reCamera (1 to 2): x941
(ad4362f) predates the placement room commit, which lists the tasks'
`rotation` entries (as x948 against x921).

Unit tests: `a_repeated_reference_entry_with_a_uuid_names_that_footprint`
(placement_constraints), the shifted cut and the starts in
`copies_of_a_tabbed_panel_keep_their_parts` (board_placer),
`global_placement_starts_parts_where_a_start_says` (pcb-placer tests).
Logs: build/panels-x1000.log, build/panels-ss-x990.log,
build/panels-ss-x1000.log. Snapshots x1000, x1001.

## Committed on 2026-10-15 (narrow reserve floor)

The layout ladder's reservation for the narrow-signal step
(`narrow_step_reserve`, "ladder reservation" above) was not a floor: a
rung, a finer pitch or a seed round started when the work left beyond
the reserve covered the last attempt's work, a finer pitch costs about
the square of the pitch ratio more, a rung's work deadline was the
pitch's whole work left (the reserve included), and the step then needed
only half of the reserve. Worse, the reserve's own floor (120 s) was
below the step's minimum (`narrow_step_minimum`, 180 s), so a board with
a short first attempt kept a reserve the step could never use:
TelemetryOnboard (v16 triage) kept 120 s, its three 0.05 mm rungs ran
(none better than the 0.075 mm tracks rung), 105 s were left, the step did not run and the
board ended 1 unconnected (v15 passed with the step).

Changed (crates/pcb-kicad/src/board_router.rs), only where the ladder
keeps a reserve (a work deadline, i.e. a layout, and a project where the
step applies):

- The reserve is at least `NARROW_RESERVE_FLOOR` (240 s: the step's
  180 s minimum and its clean-up at the negotiation's 60 s floor), at
  most the cap as before (0.4 of the work left, 600 s), and none when
  the cap is under the step's minimum (`narrow_step_reserve_floored`).
- A further rung starts only if its projected work leaves the whole
  reserve (`fits_before_reserve`; the last attempt's work, times the
  square of the pitch ratio on a finer lattice, `projected_attempt_work`,
  at least 120 s). The seed retries and seed rounds were already checked
  against the whole reserve.
- Work deadlines end where the reserve begins (`deadline_before_reserve`):
  a rung's at the pitch's work left less the reserve, each seed's at its
  share (seeds side by side count once each) of the work left less the
  reserve. A rung that stops at the deadline only does so where it would
  otherwise have eaten into the reserve.
- The step negotiates at least what the ladder's first attempt negotiated
  (its budget: the probe and its continuation, or the first rung's),
  bounded by the reserve and half of what is left
  (`narrow_step_negotiation_floored`).
- A board the ladder completes (a clean skeleton rung that goes on to
  the next rung) releases the reserve.
- The step still runs with half the reserve and at least 180 s left (a
  rung can still overrun its projection inside its deadline).

`PCB_LADDER_OLD_RESERVE` restores the old rules. Route mode keeps no
reserve and is unchanged, as are projects without the step and the
direct narrow-signal attempt of far-open boards (its configuration is
already narrow, so the step does not apply to it).

Layout runs, tasks v6, four threads, one run per side, unpinned; before
x941 (main at ad4362f), after x960 (this on 1223878; the polygon field
of 6aa79de changes no map; x961 is the commit rebased onto 6099e13,
whose neck repair acts only on boards with connection_width errors, none
of these). KiCad's unconnected and new errors, vias,
the layout's work (s); "same board" means the routed .kicad_pcb is byte
for byte the same:

| board | path | before | after |
| --- | --- | --- | --- |
| TelemetryOnboard | ladder, step applies | 1 / 0 / 102 / 1379, step skipped (105 s left of 120 kept) | **0 / 0 / 99 / 1196, passes**: 240 s kept, the 0.05 mm rungs did not start, the step ran (139 s of work, router 0 open) |
| MokyaLora | ladder, step applies | 0 / 0 / 333 / 1365, passes | same board, 1206 s: 240 s kept, the one-seed round of the best rung (1 open, about 160 s of work) did not start |
| A13 module | direct narrow attempt | 17 / 3 / 488 / 1653 | same board |
| vaio_re | direct narrow attempt | 15 / 0 / 716 / 1263 | same board |
| DSP-ADAU1452 | direct narrow attempt | 65 / 0 / 220 / 1276 | same board |
| Castor | no step | 0 / 0 / 167 / 924 | same board |
| keyboar_ | no step | 0 / 0 / 39 / 275 | same board |

No row was cut. module, vaio_re and DSP take the direct narrow-signal
attempt since x855 and never reach a ladder with a reserve, so this change
cannot touch them (their x941 numbers differ from x892's in the "after
the narrow attempt" table through the commits between, not this one).

DSP-ADAU1452 cold route (designer placement, no copper, route mode,
1200 s ladder, 4 threads): x940 1 unconnected, x960 the same board
(same attempts, 60 vias). It was not a reserve problem: route mode keeps
no reserve, and the step was not short of work. Its log (x903, x940,
x960): the best attempt before the step is the 0.075 mm tracks rung (20
open, 115 vias, ahead of the connect rung's 20 open and 135 vias on
vias); the step varies it, its negotiation reaches 1 net in conflict at
iteration 10, sits there to the price cap and stops on the stall at the
cap (`stall_at_cap` 12) after 25 iterations, with 16 s of negotiation
work of a budget of at least 60 s; the repair leaves 2 open. x817's step varied
the 0.075 mm connect rung (22 open against tracks' 24), completed in its
negotiation, and 96 of its 142 s of work were the via reduction of a
complete board. The 0.05 mm rung came after the best attempt was chosen
and only spent ladder work. What would help there is a step that tries
the other pour mode or the runner-up attempt when it stalls, not more
work.

Short tier x960 against build/short-x941.log (tasks v6): all 12 rows
identical (vias, length, work, KiCad's findings; 35 unconnected, 12 new
errors, 1129 vias, 2671 s of work; CPU 3913 to 3349 s is load). No
short-tier board keeps a reserve (no "kept for the narrow-signal step"
line in any layout log).

Unit tests: `the_floored_reserve_is_enough_for_the_step_to_run`,
`no_attempt_starts_into_the_reserve`,
`attempts_stop_where_the_reserve_begins`,
`the_step_negotiates_at_least_what_the_first_attempt_did_within_the_reserve`.
Logs: worktree pcb-maker-reserve2, build/runs (rows.py prints the rows
with the ladder's reserve, rungs and step lines; cold-x960 the cold
route), build/short-x960.log.

## Placement seeds after honest work (2026-10-15, x934 / x980, no code changed)

Question from the v16 triage (its item 9): since the placer's work is
counted honestly (93f40bd) and the pool runs only the seeds the budget keeps
(8435fff), the race runs fewer seeds (hackrf 16 to 10, WLED_Matrix 14 to
11, 4in1-mini 9 to 5, MIDAS-MK1.1 12 to 5 kept). Is the placement budget too
small? Answer: **no; noise or honest cost, nothing shipped.**

No setting exists for this: the layout sets the placer's `work_seconds` to
0.4 of `total_seconds` itself (`--placer` cannot change it) and the race's
first batch is `min(placement_seeds, 3)`. x980 = 1223878 plus two fields used
only for this measurement and not kept: the layout's `placement_share`
(0.4) and the placer's `minimum_seeds` (3), and a `--layout` JSON option in
run.py. Layout mode, tasks v6, four threads, three jobs, load 8-31, one run
per configuration (the layout is deterministic, no row cut):
(a) x934 as is; (b) `placement_share` 1.6 (2400 s of seed work, room for
16 seeds on all six); (c) `minimum_seeds` 16 (all 16 seeds in one batch,
whatever the budget); (d) as (c) with `congestion_probes` 6 (the race
probes six placements instead of three); and the designer's placement
(tracks stripped) in route mode on x934 as the ceiling. Logs and outputs
`build/seeds/{a-x934,b-x980,c-x980,d-x980,designer-x934}`.

KiCad unconnected (+ new errors); v15 and v16 are the sweeps' rows:

| board | v15 | v16 | (a) | (b) | (c) | (d) | designer, route mode |
| --- | --- | --- | --- | --- | --- | --- | --- |
| hackrf | 0 +1e | 2 | 2 | 2 | 2 | 0 +1e | 0 +1e |
| reCamera | 4 | 9 | 5 | 5 | 5 | 5 | 4 |
| WLED_Matrix | 4 +13e | 6 +1e | 6 +1e | 6 +1e | 6 +1e | 6 +1e | 1 +4e |
| 4in1-mini | 38 | 71 | 54 | 48 | 48 | 56 | 5 |
| Castor | 0 | 0 | 0 | 0 | 0 | 0 | 0 |
| MIDAS-MK1.1 | 11 | 9 | 8 | 8 | 8 | 8 | 8 |
| sum | 57 | 97 | 75 | 69 | 69 | 75 | 18 |

Vias 1416 / 1422 / 1422 / 1310 (a, b, c, d); CPU 7139 / 9811 / 8546 / 8251 s.

The placement per configuration (seeds kept of run, the placer's kept
seed; the clock's charge = the kept seeds' work as the reference pool of
eight threads runs them per batch; routing work left = 1500 s less the
charge; placement wall on four threads):

| board | (a) seeds, charge, left, wall | (c) | (b) | sum of seed work (a / 16 seeds) |
| --- | --- | --- | --- | --- |
| hackrf | 10/10 #4, 146, 1354, 214 | 16/16 #4, 153, 1347, 191 | 16/16, 196, 1304, 225 | 646 / 1048 |
| reCamera | 16/16 #1, 78, 1422, 60 | 54, 1446, 64 | 78, 1422, 82 | 402 |
| WLED_Matrix | 11/11 #2, 121, 1379, 168 | 122, 1378, 176 | 171, 1329, 308 | 603 / 877 |
| 4in1-mini | 5/5 #3, 297, 1203, 162 | 16/16 #16, 311, 1189, 400 | 443, 1057, 413 | 691 / 2100 |
| Castor | 16/16 #1, 28, 1472, 27 | 19, 1481, 39 | 28, 1472, 38 | 145 |
| MIDAS-MK1.1 | 5/7 #4, 309, 1191, 199 | 16/16 #11, 307, 1193, 341 | 406, 1094, 515 | 630 / 2082 |

What it says, per board:

- **The clock barely charges the extra seeds.** The charge is the
  reference pool's makespan per batch; (a) runs a batch of three and then
  one more batch, two rounds of the eight-thread pool, and (c)'s single
  batch of 16 is also two rounds. So 16 seeds cost the routing 0-14 s of
  its budget (less on reCamera and Castor, whose 3 + 13 batches become one),
  but 20 % more CPU over the six boards (+1400 s, 4in1-mini and MIDAS
  +1050 and +500) and twice the placement wall on four threads where the
  seeds are long (4in1-mini 162 to 400 s, MIDAS 199 to 341 s). (b) runs
  the same 16 seeds in 3 + 13 and charges up to 130 s more (4in1-mini 443,
  MIDAS 406) for the same boards as (c): the routing did not use it.
- **hackrf: not the seeds, the probe.** v15's final placement (RUDY 12.51,
  the best score) is one of (a)'s ten seeds too, and the race probes it
  first in every configuration (scores [12.51, 19.06, 19.6] in v15, v16,
  (a) and (c): the same three placements). v15's probes left [16, 16, 23]
  nets unfinished and it kept that placement (0 +1e); x862's and x934's
  probes leave [36, 72, 10] and keep the third (2). The probe is a 75 s
  route by today's router, so the router change since x723 moved the race,
  not the placer. With six probes (d) the fourth placement (RUDY 20.15,
  probe 0 unfinished) wins and ends 0 +1e, 603 vias against 701.
- **MIDAS-MK1.1:** 16 seeds bring v15's best-scored placement back (RUDY
  1.57, absent with five seeds); its probe leaves 24 unfinished against the
  kept one's 17, so the race keeps the same placement and the board is the
  same (8, 197 vias). v15's final placement is the placer's kept seed then
  and now (5 parts differ).
- **WLED_Matrix:** the extra seeds only change the third probe (4.29
  instead of 4.89, 119 unfinished); the same placement wins, identical
  board. v16's poses equal (a)'s; v15's (138 parts differ) are not among
  today's seeds.
- **reCamera, Castor:** 16 seeds already in (a); identical results in all
  configurations. reCamera's v16 row (9) is 5 on main: the router, not
  the placement (v16's triage counted it as placement).
- **4in1-mini:** seed 16 (beyond (a)'s five) scores best by RUDY and is
  kept: 54 to 48. With six probes (d) a placement whose probe left fewer
  nets unfinished (101 against 109) wins the race and ends worse (56):
  the probe's order does not hold for the final route when the counts are
  this close. v15's 38 came from placements today's placer no longer makes
  (every configuration differs from v15 in 188 of 191 parts; x792's far-side
  courtyards changed the seeds, Castor's sixteen seeds differ from v15's in
  162 parts too). The designer's placement routes to 5: the gap on this
  board is the placement itself, not the number of seeds.

Over the six boards: (b) and (c) 75 to 69 unconnected, all of it one board
(4in1-mini 54 to 48), with (d) giving 6 back on the same board; the
legaliser agent measured a placement change to flip crowded boards like
this one by far more, so one seed's difference on one board is not a
signal. More placement budget (b) costs routing budget (up to 130 s) and
CPU for nothing measurable; a floor of 16 seeds (c) is nearly free in the
clock but costs 20 % CPU and placement wall. Neither is clear enough for a
default, and the v16 "worse" rows were not seed counts: hackrf is the
probe race under the new router, reCamera the router, WLED_Matrix and
MIDAS unchanged with 16 seeds, 4in1-mini a placement gap that seeds do not
close (designer 5, ours 48-56). Short tier not run (no default changed).

Worth knowing for later: the race's probes (RUDY's best three, 75 s each)
pick the final placement by a margin smaller than their own noise on
hackrf and 4in1-mini (probe 36 for a placement that routes to 0-1; probe
101 for the one that ends 8 worse than the probe-109 one). The designer's
ceiling (18 unconnected over the six against 69-75) is in 4in1-mini (5 vs
48-56) and hackrf's vias (277 vs 603-701).

## Committed on 2026-10-15 (connection width)

KiCad's `connection_width` (drc_test_provider_connection_width.cpp, 10.0.7,
fetched from the 10.0.7 tag): runs only with a board-setup
`min_connection` (KiCad's default 0) or a custom rule, never from a
netclass. Per net and copper layer it unites zone fills (teardrops
included), tracks, flashed vias and flashed pads, fractures the union and
pairs each concave vertex with the nearest non-neighbouring vertex of the
same outline closer than the minimum less 2 x (DRC epsilon 0.0005 + max
error + extra fill margin 0.0005) = 0.115 mm for 0.127, seen from inside
at that vertex, with the outline bulging past the limit in x and y on both
sides. Vertex based and local: it also flags narrow gaps between two
parts of one outline ("gap" below). Position is not in the JSON.
`experiments/pour-fidelity/necks.py` reproduces it vertex for vertex on
kicad-cli's own fill (`--refill-zones --save-board`) and agrees with
KiCad on every board with findings (OpenESC's 199 are KiCad's cap).
Severity is the project's: error only on three corpus boards (CDFER
WLED_Matrix 0.127, USB_Keypad 0.127, USB_Cable_Tester 0.1); 15 others
have it as a warning.

Why our pour model misses them: kicad-cli's DRC with `--refill-zones`
first rebuilds teardrops for every pad with teardrops enabled (WLED: 3
zones become 636), and the necks are KiCad filler leftovers its
min-thickness pruning does not reach, all in hatched zones: the thermal
rings (added after pruning), the minimum-width border the iterative
refill restores around other nets' teardrop knockouts, pad corners
against hatch holes. Census (necks.py; build/necks/designer.txt,
results.txt, route.json, gaps.json):

| board | KiCad | kinds |
|---|---|---|
| designer WLED_Matrix | 4 | fill between pads / by a track (2 near other nets' teardrops), 2 gaps |
| designer USB_Keypad | 9 | fill near teardrops 5, thermal ring 2, by a track 2; 2 gaps |
| our WLED, v16 / short-x903 / short-x921 / short-x941 | 4 / 13 / 7 / 13 | GND F.Cu at the RGB LEDs' pad 3: pad copper or relief against hatch holes and the LED's own pad 1, a ring, fill near teardrops |
| our USB_Keypad, v16 / x903 / x921 / route x941 | 3 / 4 / 3 / 7 | GND F.Cu fill near teardrops, 3 of route's 7 are gaps |
| warnings: 0xCB pcb (ours) | 14 | thermal rings against other nets' copper |
| warnings: OpenESC 4in1, 4in1-mini (ours) | 199 cap (855, 1090) | our own tracks narrower than 0.16 |
| warnings: reCamera, MegaDrive, BAGEL, OpenESC designer | 1-30 | own tracks, a via, fill between pads |

Not modelled in the pour: it would need KiCad's teardrop generator and
its hatched iterative refill (polygon booleans the router does not have).
Built instead: a repair with KiCad's fill (`necks.rs`, on,
`PCB_KICAD_NO_NECK_REPAIR`; docs/router.md "KiCad's connection width").
When the final DRC has connection_width errors, kicad-cli refills and
saves a copy, the net's copper (fill outlines, pads, tracks, vias) is
rasterised at 4 microns, and saddles of the distance map (a pixel
narrower than the limit whose wider neighbours form two arcs, leading to
copper wider than the limit both ways within 0.5 mm) are the necks; on
the boards above it finds exactly KiCad's passages (7/7, 12/13 with one
duplicate pair, 3/3, 4/4). A track of the net, minimum wide, runs along
each into wide copper where it keeps class, hole, edge and other-fill
clearances (shifted or shortened if not); the board is checked again and
the tracks kept while connection_width drops and nothing else (error
types, unconnected) rises, by halves if all together fail (at most six
DRCs). Gaps are not repaired: the space's saddles were mostly gaps KiCad
does not report (Keypad: 20 candidates, 7 to 6 instead of 7 to 3). A
track that removes a Keypad neck sometimes adds a zone-zone unconnected
item in KiCad (not understood; the halving drops it). Cost: two to six
kicad-cli runs, 45-90 s wall, only on boards with such errors; a hook in
route_kicad_board's end and in the layout's end (a `necks.json` marker
stops a second try); `pcb-maker repair-kicad-necks <dir> <id>` runs it on
a routed directory; `board-router.json` gets `neck_repair`.

Numbers (x950; the commit's binary x951 is the same code plus comments):

- Route mode on the v16 placements, tasks v6, x941 against x950
  (build/necks-route-x9{41,50}.log): WLED_Matrix connection_width 4 to 0
  (5 tracks), USB_Keypad 7 to 3 (4 tracks; the 3 left are gaps),
  USB_Cable_Tester none; unconnected, vias, other errors identical.
- Short tier (flock, tasks v6) x941 against x950 (build/short-x941.log,
  build/short-x950.log): new errors 12 to 3, unconnected 35 to 35, vias
  1129 to 1129, passes 6 to 6, work 2671 to 2671 s, cut 0. Changed rows:
  WLED_Matrix connection_width 13 to 1 (12 tracks; new errors +11 to +2,
  the copper_edge_clearance it had), USB_Keypad 4 to 2 (2 tracks kept,
  under the designer's 9 either way). Every other row identical; the
  rows' length is the router's (the repair tracks, under a millimetre a
  board, are not in it). CPU WLED 609 to 637 s, Keypad 517 to 591 s (not
  a pinned pair).
- Starved-thermal census (census.py, fill3 KiCad cache, 44 boards) with
  x950: 162 predicted / 7 missed / 26 false alarms / 2 not modelled, as
  x912 (the router is untouched).
- Unit tests `a_thin_bridge_between_two_wide_parts_is_a_neck_a_wide_one_and_a_spike_are_not`,
  `findings_are_read_from_kicads_report`, `a_repair_track_keeps_its_clearances`.

Open: gaps (fill them along the slot, with KiCad's "same outline"
condition); the Keypad unconnected item; the WLED neck the repair leaves
(0.070 at a repair track, x950 short tier).

## Committed on 2026-10-15 (placement room)

The first three of the walkthrough's automation gaps (open problem 8):

- **A fast no.** Before placement (after the constraints are applied),
  a user's `row` that fits the outline's box in no orientation it may take
  and the parts held at one edge on one side that need more length than it
  has (single file: only parts too deep to stand two behind each other
  within their reach, each by its shortest tight body, the two that may
  overhang the ends by their copper) are input errors (exit 2), said
  together with the region errors (`placement_room::outline_holds`; not
  for an automatic outline, which the layout grows). In the placer,
  `hopeless` (computed on the last, loosest level before the levels run):
  the movable tight bodies need more area than a side's outline, or they
  cannot lie side by side in strips (`strips_hold`: bodies at least `t`
  across meet a line at most `floor(height / t)` times, so their least
  extents along it sum to at most that times the width). Both are exact,
  so a board that can be placed never takes this way; a hopeless one skips
  every level: one hurried legalisation from the global placement,
  `overfull` in board-placer.json and a hint. The work budget is checked
  inside the passes (`out_of_work`, a thread-local stop the running
  `place` sets): past it a part without a spot in `legalize` searches 31
  rings (15 mm of a 0.5 mm grid) instead of the board, `evict_for`,
  `refine`, `repair_relations` and `align_orientations` stop, the
  stuck-first retry is skipped.
  Urchin (walkthrough fixture) on 80 x 40 mm, four threads, load 20-35:
  with the walkthrough's constraints (rows, regions) x921 exit 1 after
  742 s (the "over 12 minutes"), now exit 2 in 0.04 s; move_all only on
  60 x 40 mm exit 1 in 3 s. Move_all only on 80 x 40 mm is not hopeless
  by either test (the KiCad 6 switches have no courtyard: their pads alone
  block, and pads could interleave), so it still takes the levels: x921
  398 s, x948 353 s (loads differ; the seeds' work 756 s against the
  600 s budget before, 604 s now: the passes stop at the budget).
- **Room as numbers** (`placement_room::room`, `room` in board-placer.json
  and in the verdict of place, layout and of an input error a constraint
  of the outline raised): per side the usable area (outline less the edge
  band and fixed bodies), the movable bodies with spacing, utilization,
  the area missing at a target of 0.7, the smallest outline at the target
  (area at the current aspect, then large parts side by side in strips as
  full as the target, choosing among sizes within a tenth of the least
  area the one nearest the current aspect; every part and row; edge sets)
  and, when parts did not fit, `suggested_outline_mm` with a next action.
  The urchin example (agents.md) suggests 118 x 107.5 mm; placed there,
  every part finds room (two near/relative entries written for halves
  side by side miss). An error verdict with room is written into a fresh
  output directory, and a directory holding only such a verdict is taken
  again by the retry.
- **What is kept.** `back`, `front`, `rotation` and `opening_outwards` are
  in `constraints[]` (`Constraints::kept`, `report_kept`; not in the
  placer's ranking, which sees only `report`). The verdict has
  `schematic_parity_by_type`, and a next action that says there is
  nothing to do when every finding is "No pad found for pin" (urchin's
  6 net_conflict, the controller's no-connect pins).

Identity (x948 = this on 140d0bf, against x921): place-kicad-board with
the short tier's layout.json and constraints (tasks v6) writes
byte-identical boards on spell_tome_bottom, USB_Cable_Tester, urchin,
eternal-keypad, OpenFC and Blue_Line; the short tier (tasks v6, 2 jobs x 2
threads, `build/short-x948.log`) against build/short-x921.log: every
layout's placement (placed/board-placer.json poses) and result board
byte-identical, all rows identical (compare.py same 12, unconnected 42
to 42, passes 6 to 6, vias, length, work, first-route open terminals)
except the `constraints` count on MIDAS-MK1.1 (3 to 6), OpenFC (25 to
50) and reCamera (1 to 2): their tasks' `rotation` entries are listed
now; nothing missed changed.
Snapshot `build/bin/pcb-maker-x948` (x940 was taken by another agent);
after the rebase on ad4362f, x949.

## Committed on 2026-10-15 (cold-route regression, x900-x903)

The two regressions of the Freerouting refresh, cold route (designer
placement, no copper), route mode, 4 threads, one run each (deterministic;
`build/cold/run.sh` and `table.py` in the worktree pcb-maker-cold).

Bisect, KiCad unconnected items (b399ec85 and b3c1f81d built from 399ec85
and 3c1f81d in a scratch worktree):

| binary | x155 | 399ec85 | 3c1f81d | x325 | x510 | x644 | x738 | x881 | x902 |
| --- | --- | --- | --- | --- | --- | --- | --- | --- | --- |
| SNSP-CPU-01 | 55 | 55 | 178 | 178 | 178 | 178 | 178 | 178 | 5 |
| MegaDrive | (95) | 95 | 137 | | | | 137 | 137 | 70 |

Both flip at 3c1f81d (2026-10-10, a rerouted net's corridor grows a tile
every sixth reroute, not every third). Mechanism (`PCB_ROUTER_DEBUG`, new
`route_net ...: search failed` and `resolve ...` lines):

- SNSP: GND (199 pads, four walled in at J3 and P6) failed a board-wide
  search in iterations 0 and 1 with those four open, so it kept to its
  corridors (`hopeless_searches` = 2, b72e61a); in iteration 2 a corridor
  missed and route_net stopped the whole net at 48 pads, frozen as
  blocked (21-48 pads for the rest), while the signals took the room.
  The repair ripped it up whole and its hard reroute from one pad reached
  21. Router and KiCad agreed all along (GND partial, 178 terminals): no
  counting bug, the 362/363 is per net. The narrower corridors only made
  the miss happen. Fix: pads a board-wide search did not reach are left
  out of the net's later searches in that negotiation
  (`unreachable_terminals`, `PCB_ROUTER_OLD_HOPELESS`).
- MegaDrive: narrower corridors miss more, the windowed fallbacks cost
  (open searches 228M expansions against 14M), the work budget ends
  negotiation at iteration 15 with 145 nets in conflict instead of 19
  with 106, and the repair ripped 77 nets up whole; 19 ended with no
  copper at all. Not a bug in 3c1f81d (it measured well on the quick
  tier), but the repair's whole rip-up is: it now removes only the
  conflicted branches (ignoring the negotiation's every-eighth whole
  rip-up) and keeps an incomplete net's tree (`resolve_keeps_partial`,
  `PCB_ROUTER_OLD_RESOLVE`).

Cold route, KiCad unconnected (x881 = main before; x900 = the pad rule
plus keeping incomplete trees; x901 + conflicted branches kept; x902 +
the stuck reset, = x903 before the rebase onto 670454c):

| board | x881 | x900 | x901 | x902 |
| --- | --- | --- | --- | --- |
| SNSP-CPU-01 | 178 | 5 | | 5 |
| MegaDrive | 137 | 137 | 147 | 70 |
| vaio_re | 8 | 41 | 5 | 5 |
| mackerel-30 | 0 | 0 | | 0 |
| DSP-ADAU1452 | 0 | | 0 | 1 |

vaio on x900: the tracks rung went 634 to 132 open and became the best,
and its narrow attempt ripped +3V3E up whole (3 of 43 pads): the reason
the repair change is part of this. DSP on x902: every rung as good or
better, but the ladder had time for the 0.05 mm rung (50 open) and left
the narrow-signal step 52 s of work instead of 142 (2 open against 0;
82 to 60 vias). SNSP CPU 816 to 283 s (one attempt, no seed rounds).

Short tier x903 against build/short-x817.log (tasks v6, current main):
unconnected 42 to 35 (MIDAS 26 to 20, WLED 8 to 6, reCamera 4 to 5),
passes 6 to 6, vias 1176 to 1129, new errors 6 to 12 (WLED_Matrix
connection_width 3 to 9: KiCad's GND fill necks at the RGB LEDs' pad 3,
on both sides, more of them on the new layout), work 2390 to 2677 s
(reCamera 65 to 366: its layout took another path; WLED 358 to 469),
CPU 4437 to 4865 s (not a pinned pair). Unit tests:
`a_walled_in_pad_does_not_stop_the_rest_of_its_net` (shows the old net
ending at the corridor miss), `the_hard_repair_keeps_the_branches_out_of_conflict`.

Quick tier (30 boards, tasks v6, 6 jobs), x817 (main at 670454c)
against x903 (this change on it): build/quick30-x817.log,
build/quick30-x903.log.

- x817: 24/30 pass, unconnected 27, new errors 2, vias 2913, length
  98215 mm, CPU 13555 s, work 9313 s, cut 0, first-route open 52; load
  median 17.9 (8.3-41.7).
- x903: 25/30 pass, unconnected 21, new errors 2, vias 2867, length
  99235 mm, CPU 13190 s, work 11453 s, cut 0, first-route open 47; load
  median 10.9 (1.7-34.7).

Better 3, same 27, worse 0; no row cut on either side; 23 rows
identical. The changed rows:

- OpenFC: 1 to 0 open, passes. Its first route 2 to 1 open; the ladder's
  regular rungs ran quicker, so the 0.075 mm pitch fit the refinement
  budget (x817: projected 296 s, over 240) and its re-seeded tracks rung
  finished (0 open, 169 vias against 159). Work 563 to 2143 s, CPU 464 to
  1802 s: the finer rungs really ran.
- MIDAS-MK1.1: 9 to 8 open, vias 240 to 197, length 3391 to 4464 mm (GND
  as tracks: the ladder's tracks rung now beat the first route, which
  x817 kept). Work 396 to 1322 s is accounting: the layout's work counts
  the ladder's only when its board is kept (src/agent_cli.rs), x817's
  ladder ran as long and was dropped; CPU 1572 to 1425.
- reCamera: 9 to 5 open, first route 10 to 8, work 891 to 610.
- WLED_Matrix: 6 to 6 open, vias 138 to 143, first route 10 to 8; errors
  the same (copper_edge_clearance 1; its connection_width necks are in
  the short tier's layout, not this one).
- USB_Keypad: 2 to 2, vias 99 to 78, work 513 to 445.
- urchin (work 97.3 to 93.8) and framework_mobo_lefthalf (391.8 to 392.0):
  same boards, the searches differ by a little.

New errors 2 on both (WLED copper_edge_clearance, eternal-keypad's), so
none came or went. Work +2140 s is OpenFC +1580 and MIDAS +926 (the
accounting above), less reCamera, Keypad, WLED.

The five cold boards on one binary pair, x817 against x903, KiCad
unconnected / vias / CPU s:

| board | x817 | x903 |
| --- | --- | --- |
| SNSP-CPU-01 | 178 / 1220 / 775 | 5 / 1456 / 302 |
| MegaDrive | 137 / 1148 / 615 | 70 / 1324 / 506 |
| vaio_re | 8 / 421 / 1142 | 5 / 369 / 961 |
| mackerel-30 | 0 / 46 / 574 | 0 / 46 / 555 |
| DSP-ADAU1452 | 0 / 81 / 1587 | 1 / 60 / 1450 |

(x903 identical to x902 on all five; x817 the same counts as x881 and
x738. x940, this change rebased onto f7addd8's zone seams: the same
counts, vias and lengths as x903 on all five.)

WLED_Matrix's connection_width findings (short tier): KiCad's own fill,
not ours (the routed board has no filled_polygon; the tracks rung leaves
GND's zone to KiCad's refill), necks of the GND zone on F.Cu at the
LEDs' pad 3 where other nets' tracks pass close. Not more GND as tracks:
GND track length 531 to 496 mm (F.Cu 409 to 423, B.Cu 122 to 73). The
x903 result is the tracks rung at the 0.075 mm pitch (10 open, the
regular pitch 14), x817's at the regular pitch (15; its 0.075 rung 23):
on the finer lattice other nets' tracks hug the LED pads, 25 to 31 of
the 64 LED GND pads with a foreign F.Cu track within 0.35 mm.

Not done: the walled-in pads could still be joined among themselves (a
forest of trees in the hard search: J3.5-J3.6, P6.5-P6.6 would save 2 of
SNSP's 5); a hard search that fails from the first pad's tree still ends
the net; the corridor schedule itself (3c1f81d) was not revisited.

## Committed on 2026-10-15 (zone seams)

The census's five TelemetryOnboard misses (J1/J2 SH3/SH4, D12.1, all on
F.Cu): its GND has four zones, a whole-board one of priority 0 (F, B), one
of priority 1 (F, B, In1, also the whole board) and two 4 mm bands along
the round edge, priority 10 (left, top) and 11 (right, bottom), drawn with
arcs. What KiCad 10.0.7 does (zone_filler.cpp, zone.cpp
BuildSmoothedPoly, drc_test_provider_zone_connections.cpp; the 10.0.7 tag,
not the master checkout in benchmarks/real/external/_downloads): each zone
is filled on its own over its outline and those of the same-net zones it
collides with (so reliefs and spokes are the same up to the seam), trimmed
to its own outline, and the outlines of the net's zones of a strictly
higher priority are cut out of it last (`subtractHigherPriorityZones`;
equal priorities overlap). The fills meet at the higher zone's outline
with no clearance between them. The DRC runs per zone and layer: a pad's
spokes are the crossings of its ring with that zone's fill, islands are
that zone's, tracks count where they end in that zone's fill, and a zone
whose fill does not cross the ring is no connection. The USB shield pads
(1.55 x 1.5 mm, 1 mm from the band's inner edge at y = 45.5) have their
ring 0.015 mm inside the band: three spokes in the priority-1 zone, one
(+y) in the band, "actual 1" for zone 11 (10 for J2.SH3). D12.1 sits in
the left band with its +x spoke crossing into the priority-1 zone: one.

Modelled (`zone_seams`, on, `PCB_ROUTER_NO_ZONE_SEAMS`): `Plane::above`
holds the outlines of the net's higher-priority zones on the layer; where
a net has two or more zones on a layer, `kicad_spokes` counts per zone the
pad's spokes whose ring crossing lies in the zone's part (sampled across
the spoke: a spoke the seam splits counts in both), its island spokes,
other pads' crossings and its tracks ending there, by the DRC's rule; the
worst zone's shortfall (`SpokeRecord::seam_short`, `seam_zone`) joins
`spokes_short`, the dump (`seam_short`) and spokes.py; a manual spoke for
it must end in that zone's part. Two fixes it needed: zone outlines drawn
with arcs are followed (`rule_area_polygon_points` dropped `arc`, so the
bands were polygons through their other corners; `PCB_KICAD_OLD_ZONE_ARCS`;
in the corpus only Telemetry's zones and a keepout of oro-os link have
arcs), and D12.1's +x spoke, which the model blocked: it ends in a strip
exactly the minimum width (0.2 mm) between its relief and D12.2's 0.2 mm
clearance, whose brush centres lie on one line 2 microns short of the
spoke's end (KiCad's end test is 5 microns past gap and half width); the
clearance test now runs from the ring to gap plus half width
(`spoke_clear_to_fill`, on, `PCB_ROUTER_OLD_SPOKE_CLEAR`). Not modelled:
per-zone fill settings (the layer's fill takes the last zone's clearance
and minimum width: Telemetry's bands keep 0.25, the priority-1 zone the
class's 0.2) and islands the seam alone makes (island verdicts stay the
net's one fill's).

Census (44 boards, KiCad side from the fill3 cache), x817 against x912
(x910 before the rebase, identical), and each change switched off:

| | x817 | x912 | no zone_seams | old spoke clear | old zone arcs |
|---|---|---|---|---|---|
| DRC starved predicted | 157 | 162 | 157 | 161 | 157 |
| missed | 12 | 7 | 12 | 8 | 12 |
| false alarms | 27 | 26 | 26 | 27 | 26 |
| starved pads not modelled | 2 | 2 | 2 | 2 | 2 |

Telemetry 7 found / 5 missed to 12 / 0 (D12.1 now as an island sliver in
zone 1, the strip the lattice misses: right verdict, KiCad counts it as a
plain spoke). The false alarm gone is Quanta75's FB2.1 (B.Cu, the same
end-of-spoke case); capybully's U1.8 changes one spoke from blocked to
none, verdict unchanged. Hub (several equal-priority GND zones per layer)
gets `seam` rows only on pads already short. No new false alarm.
Overlapping same-net zones of different priority (`experiments/
pour-fidelity/seams.py`, every census board and every v15 source and
result): only Telemetry in the census; in the corpus also OpenESC-20x20
4in1-mini (GND In2, priority 0 under 30) and SNSP-CPU-01 (GND B.Cu, 2
under 4), neither with a thermal pad near the seam.

Route mode on the v15 placements (tasks v6, `--jobs 1 --threads 4`, one
run per side), x817 against x910, `build/seams-x8{17,10}.log`:
Telemetry 108/108, 0 unconnected, 88 vias both, KiCad starved_thermal 5
(J1.SH3, J2.SH3/SH4, J1.SH4, D12.1, each "actual 1" in a band or the
priority-1 zone) to 0, length 3549.8 to 3559.4 mm (the manual spokes into
the short zones), other findings identical (the designer's 50 clearance,
22 solder_mask_bridge); 4in1-mini and SNSP-CPU-01 identical (53 and 259
unconnected, SNSP's 13 solder_mask_bridge and 2 starved on both).

Short tier x912 (rebased on 140d0bf) against build/short-x880.log (tasks
v6): all 12 boards byte-identical (no short-tier board has two zones of a
net on a layer, so only `spoke_clear_to_fill` could move them), 42
unconnected, 6 errors, 1176 vias, 22032 mm both; work 2390 to 2384 s
(framework_mobo 225.5 to 220.3 s, the same board: the shorter clearance
test asks the fill probe less). Tests:
`each_zone_counts_the_spokes_in_its_own_part` (router: a pad with three
spokes in the lower zone and one in the upper is short 1 in the upper, by
priority or overlapping at equal priority, 0 as one fill; the manual spoke
ends in the upper part) and
`a_zone_of_a_higher_priority_is_cut_from_the_nets_lower_zone_and_arcs_are_followed`
(lowering). Logs: build/census-seams (census x817.txt, x912.txt, the switched-off
runs try2-*.txt, seams.py's JSON), build/seams-x817.log,
build/seams-x910.log, build/short-x912.log.

## Committed on 2026-10-15 (agent walkthrough)

An agent's whole flow on two real boards, using only README.md,
docs/agents.md, docs/constraints.md and `--help`, from the KiCad project
with its schematic to a board `drc-kicad-board` passes. The harvested
GitHub boards carry no schematic (`benchmarks/real/external/github` holds
the board and project only), so the schematics and project libraries were
fetched at the boards' commits (origin.json): spell_tome_bottom (25 parts,
18 nets, 2 layers, its libraries in the project) and urchin (a split
keyboard, both halves on one board: 80 parts, 68 nets, KiCad 6 schematic
and libraries, MIT).

The flow (RAYON_NUM_THREADS=4):

```sh
pcb-maker import-kicad-netlist dl/spell_tome/spell_tome_bottom.kicad_sch tome spell_tome_bottom
pcb-maker describe-kicad-board tome spell_tome_bottom > board.json
# constraints.json: outline 70 x 100, U1 on the back, SW2 flush on the right
# edge opening outwards, J1 at the top, the four buttons in a region,
# L1 below U2 (relative), C2 near U2 pin 6
pcb-maker place-kicad-board tome spell_tome_bottom place1 tome/layout.json
pcb-maker layout-kicad-board tome spell_tome_bottom lay1 auto tome/layout.json
pcb-maker drc-kicad-board lay1/result spell_tome_bottom drc.json
pcb-maker export-kicad-fab lay1/result spell_tome_bottom fab
```

spell_tome: exit 0 in 41 s (73 s of work), 18/18, 9 vias, 586 mm, 8 of 8
constraints; drc-kicad-board 0 errors, 0 unconnected. urchin with 100
constraints (two 5 x 3 key grids as `row`s, a region per half, each of the
34 diodes `near` its key, thumb keys `relative` below the grid, the power
switches on the top edge, display and reset near the controller): exit 0
in 29 s (20 s of work), 68/68, 39 vias, 1897 mm, 100 of 100. With
`"total_seconds": 20` both finish in 12-13 s, still complete. Failure
paths tried: a 22 x 24 mm spell_tome (exit 1, `unplaced` U1, J1, J2 and
the hints in `next_actions`; 30 x 30 mm already routes clean), footprint
libraries removed (exit 1, `missing`), a netlist exported outside the
project, a file that is no netlist.

Friction found, and what was done:

- **Bug: a turned row was re-lowered the wrong way.** Layout mode lowers
  its own placement again for the report and the moves; `apply_rows` built
  the row from the leader's angle alone, so a row the placer had turned
  half way ran the other way: urchin's right half reported `region SW18`
  missed by 61 mm and `relative SW33 SW18` by 95 mm (exit 1 on a board
  that was fine), and the members would have been pulled there by the
  followers' sync. The row's frame is now the leader's angle or that angle
  turned by a half (a quarter with `turns`) turn, whichever matches where
  the members lie; a fresh stack matches all alike and keeps the leader's.
- **`near` could not name a key of a row** ("SW2 is carried by its row"):
  a keyboard's diode at its switch could not be said. `part_of` and
  `pin_of` may now name any row member: a new placer anchor
  `Anchor::Area(leader, member, centre, half)` is the member's body (or
  pad, half size 0) in the leader's frame, and the report names the member.
- **Bug: KiCad 6 library footprints kept `fp_text reference`.** KiCad
  reads it over the Reference property the import sets, so every copy of a
  footprint carried the library's reference (`Bat+0`): 181 schematic
  parity issues on urchin (73 duplicate, 73 missing footprints, 35 net
  conflicts). The import converts `fp_text reference` and `fp_text value`
  to properties (place, layer, font, hidden kept): 6 left, the controller's
  no-connect pins below.
- **276 false warnings "fixed parts X and Y overlap"** on every placement
  of a fresh import: the check ran before the constraints, when the stack
  was fixed by the default rules (parts off the board are on the edge);
  `move_all` frees them. It runs after the constraints now.
- **The layout's last line said `native complete=false`** above a verdict
  `complete` (KiCad's raw count includes the cosmetic silkscreen
  warnings). It now prints KiCad's unconnected count and its findings of
  any severity, "the verdict judges them"; agents.md says `routed.native`
  is raw and the verdict decides.
- **`describe-kicad-board` gave a fresh import a 3.8 x 0.8 mm outline**:
  SW2's slots on Edge.Cuts. `outline.from_footprints` is true when the
  board draws no Edge.Cuts of its own.
- **`place-kicad-board` needed a placer.json** that agents.md told the
  agent to cut out of layout.json; it now takes the layout config (its
  `placer` object) as well.
- **Missing footprints said only "not found"**: now where it was looked
  for (no library of that name, or the library's folder without that
  file), and the stderr line says to add the library and import again
  into the same directory (constraints.json and layout.json are kept; the
  help said "existing files kept", which read as "the board too").
- **A netlist exported outside the project** missed the project's
  libraries (9 of 25 footprints on spell_tome): the import also looks in
  the folder of the schematic the netlist names as its source.
- **A file that is no netlist imported as an empty board, exit 0**: now an
  input error (exit 2) that says how to export one.
- **`place-kicad-board` on a board without room did not end** ("checks
  constraints in seconds"): urchin on 80 x 40 mm ran over 12 minutes on
  two threads before it was killed; one part in a region too small for it
  took 55 s. The placer's relaxation levels anneal and legalize again and
  again, and the command had no work budget unless
  `PCB_PLACER_WORK_SECONDS` was set (with 10 s it still took 60 s of work:
  the budget is checked between levels only, and the first level's
  legalize, legalize-first and repair passes took 11, 10 and 8 s). The
  command now takes the layout's placement share by default (600 s of
  work, the same budget the layout would give it), and a `region` smaller
  than its part in every orientation it may take is an input error at
  once (exit 2, "region for Display1 cannot hold it: the box is 3.00 x
  3.00 mm, the part ...").
- **Help and docs**: layout's help said the unplaced case leaves a
  board-layout.json with only the verdict (there is none; verdict.json has
  it); `total_seconds` was only in `--help` (now in agents.md with what it
  does to the phases); the row rules (constraints on the first part hold
  the whole row) and two keyboard lines in the cookbook; import's help says
  what comes next and its exit codes; `unmatched_pins` on KiCad's
  `unconnected-(...)` nets are harmless but show as parity `net_conflict`.

Left as they are, small: every command's help repeats the agent exit-code
table, also for `describe-kicad-board` and `import-kicad-netlist` whose
codes differ; the import report's paths are relative where the verdict's
are absolute; `back`, `front` and `opening_outwards` are kept but not
listed in `constraints[]`; `drc-kicad-board` counts library findings
(`lib_footprint_issues`) as cosmetic that the layout's DRC does not run.

`tests/agent_walkthrough.rs` runs the documented flow on urchin
(`crates/pcb-kicad/tests/fixtures/urchin`: the schematic, project, library
table and the seven footprints used, with the MIT licence): import,
describe, a placement with a region too small for its part (exit 2 at
once), a placement that keeps all 100 constraints (a diode
reported against its key), the layout at `"total_seconds": 30` with
`PCB_ROUTER_NO_WALL_GUARD=1` (exit 0 and the verdict's fields), and
`drc-kicad-board` (exit 0). It is in the root package:
`cargo test --release -p pcb-maker --test agent_walkthrough` (14-28 s at a load of 20-25).

The row fix changes layouts whose row the placer turns (keyboards, LED
bars written as `row`); boards without rows place as before. The region
check and the placement default touch only `place-kicad-board` and
regions too small to hold their part. Snapshot: `build/bin/pcb-maker-x920`.
No benchmark tier was run.

## Triage of v16 (2026-10-15, x931 / x932)

The v16 sweep (x862, tasks v6, five jobs of four threads beside other
agents, load 16-35) against v15 (x723). The fix is on branch
v16-triage: x931 = x862's tree (068de8b) + fix, so that x931 against the
v16 rows changes only the fix; x932 = main f7addd8 + fix (tests, short
tier). Outputs were in the triage agent's scratchpad (not kept).

**1. The 17 cut rows.** Per row from the layout logs (seconds; "search"
and "pour" sum every phase's `profile work:` line, probes side by side
included, so they are thread sums, not wall; the KiCad column sums the
`board check` lines; lattice builds are the last `so far N lattices` count,
also summed over routers built side by side; the placement charge is its
work plus the pool's excess, `layout: placed`):

| board | wall | work | placement wall / charge | KiCad DRC (calls, hits, s) | lattice builds (n, s) | search wall / work | pour wall / work | cut |
|---|---|---|---|---|---|---|---|---|
| motherboard | 1702.5 | 909.5 | 262 / 122 | 2, 1, 16 | 7, 9.1 | 4159 / 2662 | 258 / 262 | deadline |
| link | 1709.0 | 1108.7 | 389 / 368 | 1, 1, 3 | 3, 22.3 | 3545 / 2353 | 280 / 304 | deadline |
| poly_kybd_split72_right | 1722.9 | 1232.9 | 131 / 192 | 1, 1, 2 | 3, 4.0 | 4120 / 2686 | 178 / 92 | deadline |
| poly_kybd_split72_left | 1800.7 | - | 194 / 210 | 0, 0, 0 | -, - | 3298 / 2084 | 146 / 80 | timeout |
| poly_corne_split42_right | 1784.9 | 1149.7 | 179 / 208 | 1, 0, 28 | 3, 5.9 | 3539 / 2193 | 191 / 103 | deadline,layout |
| poly_corne_split42_left | 1669.6 | 1168.4 | 96 / 125 | 1, 1, 1 | 3, 3.1 | 4182 / 2840 | 182 / 102 | deadline,layout |
| koeg-board-pcb | 1699.0 | 409.7 | 139 / 183 | 6, 5, 37 | 11, 40.1 | 1964 / 1050 | 42 / 54 | deadline |
| katia | 1671.9 | 585.6 | 200 / 323 | 1, 0, 17 | 3, 704.6 | 507 / 221 | 62 / 31 | layout |
| capybully | 1696.1 | 710.9 | 85 / 64 | 2, 0, 73 | 7, 179.1 | 1210 / 918 | 172 / 131 | deadline |
| Batch_1 | 1680.2 | 1325.7 | 370 / 387 | 4, 0, 225 | 6, 74.1 | 3432 / 3901 | 755 / 200 | deadline |
| Quanta75_BareRP2040_JLCPCBAo | 1721.3 | 1045.2 | 194 / 124 | 1, 0, 28 | 3, 42.8 | 2918 / 2175 | 243 / 243 | deadline,layout |
| woagboard | 1759.1 | 1201.7 | 89 / 76 | 11, 0, 115 | 19, 39.0 | 1833 / 1363 | 579 / 566 | deadline |
| panel | 1710.7 | 507.8 | 219 / 167 | 7, 0, 306 | 12, 567.3 | 1127 / 1765 | 640 / 293 | deadline |
| MegaDrive | 1800.7 | - | 130 / 253 | 0, 0, 0 | -, - | 4690 / 3537 | 89 / 85 | timeout |
| ghoul | 1700.7 | 1162.9 | 260 / 410 | 8, 0, 321 | 13, 44.9 | 1048 / 937 | 511 / 524 | deadline |
| ULK_sl_PG1316s | 1758.4 | 92.1 | 3 / 4 | 7, 0, 196 | 12, 1099.6 | 310 / 390 | 276 / 66 | deadline |
| ULK | 1800.6 | ~95 (at routed) | 1 / 2 | 8, 0, 201 | 13, 1201.4 | 335 / 406 | 313 / 63 | timeout |

No single phase explains them; four holes, by size:

- **Lattice builds of boards with a large cutout** (ULK, ULK_sl, katia,
  the 0xCB panel; 570-1200 s of the run). Not work, and not
  interruptible. Profiled (gdb samples, a per-obstacle trace): all of it in
  `StaticMaps::build`, `Shape::distance_to_point` of one board cutout
  measured for every node of its box edge by edge: ULK's 454-corner
  cutout over 16867 mm2, 15 s a class, four classes, 60-190 s a lattice
  (0.075 mm rungs the most); katia's 228-corner cutout 15 s a class; the
  panel's slots 0.7-2.7 s each. **Fixed** (`PolygonField` in grid.rs, a
  polygon of 16 corners or more): each edge updates only the nodes within
  reach of its box, the inside comes from each row's sorted crossings with
  `point_in_polygon`'s test; the distances are bit for bit
  `distance_to_point`'s below the reach (unit test against it on concave
  polygons with lattice-aligned corners), so the maps and the routes do
  not change. ULK's lattice 72 s to 1.2 s; route mode on v16's ULK
  placement, both measures in one binary (`PCB_ROUTER_OLD_POLYGON_DISTANCES`
  for the old): the 32 fingerprints the old side reached are all in the new
  side's (rungs side by side print in another order).
  Layouts on x931, one run each, four threads, load 6-21 (v16: 13-27):

  | board | v16 (x862) | x931 |
  | --- | --- | --- |
  | ULK | timeout at 1800 s, no row (v15: 66/70, 4 unconnected, 1 starved) | 1480 s, no cut, 66/70, 4 unconnected, no new error, 64 vias |
  | katia | 1672 s, cut (no final ladder), 139/162, 104 | 1382 s, no cut, the same board (139/162, 104, 401 vias); the final ladder ran and lost |
  | 0xCB panel | 1711 s, cut (deadline), 152/188, 86, +5 starved | 957 s, no cut, the same board (283 vias, 4594 mm); one more ladder rung ran and lost |
  | ULK_sl_PG1316s | 1758 s, cut, 68/70, 2 | 1710 s, still cut, the same board: its ladder now ran nine rungs (1200 s budget used) and seven uncached KiCad checks (22 s each), the last rung's seed retry was cut |

  The DRC cache hit the first route's and the early rungs' checks on
  ULK, ULK_sl and the panel: the boards were byte-identical to v16's.

- **KiCad DRC inside the layout is not work** (ghoul 8 calls 321 s, the
  panel 7 / 306 s, Batch_1 4 / 225 s, ULK 200 s, woagboard 11 / 115 s,
  capybully 73 s; an uncached check 10-50 s). A deterministic charge needs
  a model, and the obvious ones fail: over 57 v16 boards the median
  uncached check time does not follow the board's pads, tracks and vias
  or its file size (correlation 0.02 each); hatched zones (the panel,
  Batch_1, WLED_Matrix, capybully) and the load dominate. Counting the
  measured wall would make the layout follow the load. Not changed;
  options below. Done since: a charge per check from a constant and the
  hatch holes ("Committed on 2026-10-15 (KiCad check work)").
- **The search's work rate under the sweep's load.** On the boards that
  spend the whole budget (the four PolyKybd halves, link, motherboard,
  MegaDrive, Quanta75 Bare) negotiation and repair run 1.3-1.6 times
  their counted work in wall time: `EXPANSIONS_PER_SECOND` is an idle
  machine's, and five jobs of four threads already exceed the 5950X's 16
  physical cores, so with other agents (load 16-35 on 32 logical CPUs)
  most search threads share a core. The 10 % margin cannot hold there;
  this is the sweep's configuration as much as the accounting.
- **Pour analyses on hatched and big pour boards** run 2-4 times their
  modelled work (Batch_1 755 s of wall for 200 s of work, the panel 640 /
  293, PolyKybd 180 / 95; x862 still has the 41 ns constants, main
  recalibrated to 18 ns after the overhead round, so recheck on main).
- Smaller: the placement pool's charge (kept seeds at the reference eight
  threads) below the four-thread wall on motherboard (262 s of wall, 122
  charged) and Quanta75 Bare (194 / 124); above it on most boards. A
  layout's `work_seconds` in the row leaves out a final ladder it did not
  keep (koeg: 410 s reported, its unkept ladder ran 1100 s more of wall),
  so the rows understate the work that ran.

After the wall guard fires, the layout still runs what it does not
interrupt (lattice builds, repairs and stitching of the current attempt,
KiCad checks, labels and the final DRC), and the harness kills at 1.2x
(1800 s): that is how three of the cut rows became timeouts (ULK,
poly_kybd_split72_left, MegaDrive).

**2. New timeouts.** ULK: the lattice builds above (13 builds, 1201 s;
the first route needed 95 s of work); v15 finished in 1688 s because its
deadline was then the wall clock, which stopped the ladder; fixed by the
polygon field (row above). poly_kybd_split72_left: the first route's
negotiation ran 1046 s of wall for 677 s of search work (the load), the
wall guard cut it at 862 s of its work, and the two repair passes after
the cut (each about 60 s of negotiation, 300 s of hard repair and
stitching in all) ran past 1800 s; v15 (x723, wall-clock deadline) ended
at 396/420. Same class as the cut rows, no lattice or DRC share. 0xCB
pcb-panel: not a layout problem; the placement (306 s) and the first route
ran, then kicad-cli's zone refill timed out at 900 s (open problem 7,
`internal error: kicad-cli timed out`); v15 timed out at 1800 s before
getting there.

**3. The 13 worse rows** (compare.py counts unconnected only; PixelWave's
lost pass, one new error, is not among them). Placement against v15's
(poses of every footprint compared; v15's rows have no
`first_unconnected_terminals`), then the mechanism:

| board | v15 -> v16 (KiCad unconnected) | placement | route mode on v16's placement, x723 / x862 | x862 on v15's placement | verdict |
| --- | --- | --- | --- | --- | --- |
| Telemetry | 0 pass -> 1 | identical | 0 / 0 (88 vias both) | - | layout flow: the ladder kept 120 s for the narrow step, the step needs 180 s (`narrow_step_minimum`), 153 s were left, so it did not run (v15 won with it: 0 open; route mode's step: 0 open). Constants of 18a9a77: the reserve's floor (120 s) is below the step's minimum (180 s). Regression, on main too (not a far-open board) |
| ATAT1800 | 53 -> 54 | identical | - | - | noise: first route 67 -> 68 open, one attempt each, new errors 7 -> 4 |
| Explorer | 1 -> 2 | 2 parts moved | 3 / 3 | - | noise: first route 9 -> 3 open, the narrow attempt 8 open both |
| eurorack-pmod | 20 -> 21 | 3 parts moved | 22 / 17 | - | noise: router not worse; first route 19 -> 26, the polish kept the unpolished routes |
| protoesp | 83 -> 92 | 1 part moved, identical first route (88 open, 319 vias) | - | - | layout work: the first route and its resume booked 1246 s of work in 881 s of wall (parallel batches count each thread's expansions), so no ladder ran (v15, wall-clock deadline: four attempts). 30b14fb (x747) |
| module | 81 -> 82 | different | - | - | narrow-step reserve too small (154 s left of 319 kept); fixed on main by 1c8f17f / 974fa2b (x892: 17) |
| DSP-ADAU1452 | 105 -> 121 (+59e -> 0) | different | - | - | same; main x892: 82 |
| vaio_re | 101 -> 138 | different | - | - | same; main x892: 21 |
| BAGEL-MK1.1 | 0 pass -> 1 | different | 2 / 2 (identical) | 2 | noise in the layout path: router and placement give the same count in route mode |
| hackrf | 0 +1e -> 2 | different (205 of 421 moved, 0.635 mm steps) | 13 / 13 | 1 | placement |
| reCamera | 4 -> 9 | different | 9 / 9 | 4 | placement |
| WLED_Matrix | 4 +13e -> 6 +1e | different | 6 / 6 | 4 | placement (and errors 13 -> 1) |
| 4in1-mini | 38 -> 71 | different | 74 / 79 | 53 | placement (router 74 / 79 is noise) |

The router is not the cause on any of the eight boards routed in pairs
(x723 and x862 on v16's placement, route mode, four threads, one run
each: identical counts and vias but for eurorack and 4in1-mini, which
differ by 5 either way). Where the placement is the cause, the race ran
fewer seeds: hackrf 16 to 10, WLED_Matrix 14 to 11, 4in1-mini 9 to 5,
BAGEL-MK1.1 9 to 6. The placer's work now counts what the legaliser and
annealer do (93f40bd, x792) and the pool runs only the seeds the budget
keeps (8435fff), so the same placement budget holds fewer seeds; the
kept seed's wirelength is the same on these boards, the poses are not.
Suspects: 93f40bd and 8435fff; seed-repeated pairs would settle it (one
seed's result is not a placement's quality). Not regressions of the
code paths, but the budget now buys fewer seeds.

**4. Panels.** The rows changed wording more than behaviour: v15's
"placement is not legal" and v16's "incomplete: N parts without a legal
place" are the CLI exit-code commit's verdicts for the same failure.

- PolyKybd molecules (4x4, 4x5, 5x2_shifted, 7x5_wave_left; 5x4_wave
  places since x792's in-turn naming and routes 2/28): the parts named are the back connectors
  J5 and J6 (J9 on 7x5). The designer stacks them on the front J3 and J4
  (same spot, other side, SMD pads only, no courtyard) and KiCad finds
  no courtyard or other overlap there (DRC of the designer's 4x4: only
  starved thermals and cosmetic findings). What refuses them is the edge
  constraints of repeated references, named in turn since x792 (the k-th
  entry for J5 binds the k-th J5 in board order). The task's entries are
  not in board order, and a corner part's two edges are two entries that
  now bind two parts: placing the 4x4 with every part except J5 and J6
  fixed where the designer put them (`move_all` off) misses 17 edge
  constraints on the designer's own fixed J1, J3 and J4 (18.5, 37.6 and
  56.6 mm off: whole tile pitches) and leaves J5, J6, J2 and SW1
  without a legal place. So the rule is wrong for these tasks, not the
  geometry: a repeated reference's edge entries should be matched to the
  instances that can keep them (per entry, any instance; or the task
  generator writes them in board order with a corner's edges in one
  entry).
- SmartSpin2k_Panelized: `keep_copies` confines each KiKit copy's
  movable parts to its region (no edge constraints in the task, so the
  "region" of the hint is the copy's). The rule agrees with the designer:
  placing from the designer's poses (`move_all` off, x862) ends complete,
  every part placed and every constraint kept. From the prepared stack
  (every movable part at one point, inside one copy's region) 57 parts
  find no legal place: the search, not the rule. Start each confined part
  inside its own copy's region (its copy's fixed parts' centre) instead
  of at the common stack.
- OpenESC 4in1-panel: one net group (no copies), the front side 101 %
  full by the placer's bodies: crowding, as in v15 ("not legal" then).

**Ranked, what to assign next:**

1. Merge the polygon field (this branch; byte-identical maps, ULK no
   longer times out, katia and the panel no longer cut). Short tier x932
   against build/short-x912.log (main f7addd8 without the fix): all 12
   rows identical (42 unconnected, 6 errors, 1176 vias, 22032 mm, 2384 s
   of work); build/short-x932.log. Tests of pcb-router and pcb-kicad pass.
2. The sweep's configuration against the 1.1 margin: at most 16 search
   threads in all on this machine (four jobs of four, or three while
   other agents run), or a harness timeout and wall guard that scale
   with the measured load; otherwise every board that spends its budget
   is cut (8 of the 17).
3. Stop after the wall guard: once it fires, skip the remaining repairs,
   stitching passes and ladder rungs and go to the final check (three
   timeouts came from work after the guard).
4. KiCad checks in the layout: judge ladder rungs by the router's count
   and check only the candidates that could win, or charge a check from
   a per-board model of zone fill (hatched zones dominate; pads and tracks
   do not predict it), or store the check's seconds in the DRC cache and
   charge them on a hit as on a miss once known. Done: the gate
   ("wall guard stop, fewer layout checks") and the modelled charge
   ("KiCad check work").
5. Repeated-reference edge constraints (molecules): match entries to
   instances; then the four molecule panels can place.
6. Keep_copies start positions (SmartSpin2k).
7. Recheck the pour work model on hatched boards on main (2-4x under).
   Done: "Committed on 2026-10-15 (hatched pour work)".
8. The narrow-step reserve's floor (18a9a77): `narrow_step_reserve`
   keeps at least 120 s, `narrow_step_minimum` wants at least 180 s, so
   a board whose first attempt is short keeps a reserve the step can
   never use (Telemetry lost its pass to this; route mode's step
   completes it). Raise the floor to 180 s, or let the step run on what
   was kept. Done: "Committed on 2026-10-15 (narrow reserve floor)".
9. Placement seeds per budget (hackrf, reCamera, WLED_Matrix,
   4in1-mini): seed-repeated pairs of x723 and main's placer to see
   whether fewer seeds is the loss; if so, the placement budget in the
   new work units needs raising. Measured: not the loss ("Placement seeds
   after honest work").

## Layout sweep v16 (2026-10-15, x862)

docs/benchmarks.md "Layout sweep v16": 44 of 109 pass (v15 45), unconnected
3868 to 3527, new errors 289 to 82, better 13 / same 58 / worse 13, 17 rows
cut by the wall clock at load 16 to 35, which says a phase's wall runs well
ahead of its counted work on a quiet machine (KiCad DRC inside the layout
is not counted; the pool's charge is a reference 8-thread pool; candidates).
New timeouts ULK and poly_kybd_split72_left, the 0xCB pcb-panel ends on
kicad-cli's 900 s zone refill, SmartSpin2k and OpenESC panels stop with
parts the placer refuses. A triage agent (x930+) has the cut rows, the
timeouts, the worse rows and the panels; its section "Triage of v16"
follows. x862 predates the first-route budget (x855), hole clearance
(x881), router overhead (x872), pour recalibration (x817) and the second
narrow attempt (x893); the next sweep carries them.

## Committed on 2026-10-15 (after the narrow attempt)

The narrow-signal attempt run alone (first-route budget, above) did not
use the layout's work on every board: Hub ended at 577 of 1500 s, vaio_re
at 1005. What was left after it, in work (1500 less the layout's work and
the placement pool's excess), and what to run with it:

| board | left after the attempt (x855) |
| --- | --- |
| jetson (x881) | 114 s |
| A13 module | 40 s |
| DSP-ADAU1452 | 228 s |
| vaio_re | 410 s |
| Hub | 916 s |
| koeg | 53 s |

Changed (crates/pcb-kicad/src/board_layout.rs): after that attempt, with
at least 240 s left (`NARROW_CONTINUATION_FLOOR`), connections open and
the wall clock inside the layout's guard (`narrow_continuation`), the
attempt runs once more in the other pour mode (as tracks after connected,
and the other way round), negotiation and clean-up split from what is left
as for the attempt (`direct_narrow_budget`), that work its deadline; the
layout keeps the better board as before (open items, then starved
thermals). The work of the attempt and of this one now counts in the
layout's `work_seconds` whether kept or not (it used to count only a kept
board's; on the ordinary ladder fallback it still does). `skipped` says
when it did not run (with too little work: jetson, module, DSP, koeg).
`PCB_LAYOUT_NARROW_CONTINUATION` (stop, tracks, seeds, ladder) and
`PCB_LAYOUT_NARROW_CONTINUATION_FLOOR` are experiment switches; stop is
x855's behaviour. Unit tests for the decision and the seed count.

Candidates, one layout run each, four threads, unpinned, load 13-34, no
row cut. Unconnected and errors are KiCad's (new errors against the
designer's), then vias and the layout's work; in brackets the router's
open count of the extra attempt, kept or not. Before: x855 (jetson x881,
the hole-clearance fix; module, DSP and vaio_re rows from the first-route
agent's x855 runs, deterministic, same tasks file); candidates on x890
(this code on 1c8f17f, switches by environment); after: x892 (this code
with the default on 2b3b787, routes identical to x855 there).

| board | before (stop) | P: probe both pour modes for the attempt | T: other pour mode | S: fresh seeds | L: class-width ladder |
| --- | --- | --- | --- | --- | --- |
| jetson | 17 / 1 starved / 670 / 1307 | - | not run (114 s left); x892 identical | - | - |
| A13 module | 17 / 1 track_dangling / 490 / 1330 | 21 / 1 items_not_allowed / 409 / 1127 | not run (40 s); x892 identical | - | - |
| DSP-ADAU1452 | 82 / 0 / 225 / 1210 | 126 / 0 / 189 / 864 | not run (228 s); x892 identical; floor 180: 82 / 0 / 225 / 1441 (223, not kept) | floor 180: 82 / 0 / 225 / 1347 (one seed, 229) | floor 180: 82 / 0 / 225 / 1380 (339) |
| vaio_re | 35 / 0 / 686 / 1005 | not run | **21** / 0 / 637 / 1338 (55, kept); x892 the same | 35 / 0 / 686 / 1210 (one seed, 121) | 35 / 0 / 686 / 1310 (551) |
| Hub | 30 / 0 / 170 / 577 | 39 / 11 starved / 179 / 875 | 30 / 0 / 170 / 818 (244, not kept); x892 the same | 28 / 0 / 172 / 891 (4 seeds, 111, kept) | 30 / 0 / 187 / 1404 (114, kept: KiCad the same, 17 more vias) |
| koeg | 7 / 20 (15 courtyards, 5 shorting) / 427 / 1335 | not run | not run (53 s); x892 identical | - | - |

Steps (x892): jetson the attempt alone; module the attempt and a round of
3 seeds; DSP the attempt and 4 seeds; vaio_re the attempt, one seed, then
the tracks attempt; Hub the attempt with its 4 seed retries, then the
tracks attempt with its 4; koeg the attempt and 3 seeds.

What was learned:

- The other pour mode is the only continuation that gained: vaio_re 35 to
  21 unconnected with 49 fewer vias. On Hub it spent 240 s for a board it
  did not keep (Hub's leftover would otherwise go unused: CPU, not
  quality), on DSP (floor lowered to 180) 230 s for nothing.
- Fresh seeds: Hub 30 to 28 (two vias more, 314 s); on vaio_re one seed
  fit (255 s an attempt) and lost by 45 open; on DSP one seed lost too.
  One perturbed seed with less negotiation is a worse attempt, not a
  variation of the same one.
- The class-width ladder with 230 to 900 s: never better in KiCad's
  count (vaio_re: four probes and the continued rung in 306 s of the 410,
  551 open; Hub: 826 s for the same 30 and 17 more vias). The ladder needs the budget the first-route
  agent took from it, which is why the attempt beats it.
- Probing both pour modes (75 s each side by side, the better continued
  with the attempt's negotiation less the probes): tracks led every probe
  (module 60 against 123 unfinished nets, DSP 104 against 247, Hub 17
  against 78) and the continued tracks attempt then lost on all three
  (module 21 against 17 unconnected, DSP 126 against 82, Hub 39 and 11
  starved thermals against 30). A probe's unfinished count does not
  compare the modes: in connect mode it counts pour pads the pour does not
  reach yet, in tracks mode the pour nets are ordinary nets that finish
  late. The probe code was removed again; tracks as the second attempt,
  judged by KiCad, is the safe form of the mode choice.

Short tier x893 (this on 670454c) against build/short-x855.log: all 12
rows identical (42 unconnected, 6 errors, 1176 vias, 22032 mm, 2390 s of
work; CPU 5064 to 3924 s is main's router-overhead round). No short-tier
board has the narrow-signal step, so none takes this path.
Logs in the worktree's build/ (pcb-maker-narrow2: <board>-x855, -P,
-T, -S, -L, -T180, -S180, -L180, -x892, jetson-x881; rows2.py prints the
rows with the steps), short tier build/short-x893.log.

Left: on boards with little left (module 40 s, koeg 53 s, jetson 114 s,
DSP 228 s) nothing runs; a tracks attempt first instead of connect where
tracks wins (vaio_re) needs a predictor the probe is not. The ordinary
ladder fallback's work is still counted only when kept.

## Committed on 2026-10-15 (router overhead, x870 / x871 / x872)

Speed round four, unlike the three before it, changed no search: it looked
for CPU that buys no search and removed it with routes byte-identical.
Profiled route mode on the v15 placements of Castor, jetson-nano-baseboard
and k30-SBC (x862's code with debug symbols, four threads as the pairs run
them; `perf` is not installed and samply needs `perf_event_paranoid` 1, so
gperftools' `libprofiler.so` by `LD_PRELOAD` with `CPUPROFILE_USE_PID`,
200 Hz, libunwind stacks; tools and profiles in the worktree
`pcb-maker-speed`, `build/prof`). Share of the run's CPU (kicad-cli's DRC
included), Castor / jetson / k30:

| group | Castor | jetson | k30 |
|---|---|---|---|
| A* search proper (expansions, key heap, occupancy reads, heuristic) | 69.8 | 80.5 | 77.6 |
| the search's neck class lookup (`class_at` per step) | 2.2 | 1.2 | 0.0 |
| tile planner (`plan_corridor`: Dijkstra, `BinaryHeap`) | 3.0 | 5.9 | 8.8 |
| stamping and conflict bookkeeping | 6.8 | 3.0 | 4.1 |
| pour refresh and fill probes, negotiation | 3.3 | 0.5 | 3.5 |
| pour checks in clean-up and via reduction | 5.9 | 1.5 | 0.0 |
| lowering and lattices | 0.6 | 0.5 | 0.1 |
| KiCad (kicad-cli DRC) | 3.1 | 3.9 | 2.8 |
| allocation, memset, memcpy | 2.2 | 1.4 | 1.4 |
| SipHash | 1.8 | 0.4 | 0.8 |
| other | 1.3 | 1.1 | 0.8 |

Top symbols by self time (inlined callees included), Castor / jetson /
k30: `Router::search` 65.8 / 77.2 / 72.7 (its own lines: `KeyHeap::pop`
9.7 / 12.5 / 10.6, `occupancy_seen` 8.1 / 10.7 / 9.9), the search's
heuristic closure 4.6 / 3.6 / 5.6, `stamp`'s apply closure 4.9 / 1.8 /
2.9, `plan_corridor`'s entry-cost closure 1.4 / 2.8 / 3.9, `route_net`
(the planner inlined) 1.0 / 1.9 / 2.8, `PourMap::build` 2.6 / 0.6 / 1.0,
the planner's `BinaryHeap::pop` 0.7 / 1.4 / 2.2, `class_at` 2.6 / 1.4 /
0.0, `RandomState::hash_one` 1.9 / 0.4 / 0.8, `unstamp` 1.5 / 0.8 / 0.8,
`OwnStamps::contains` 1.8 / 0.4 / 0.9, SipHash `write` 1.1 / 0.3 / 0.4,
`Shape::distance_to_point` 1.1 / 0.0 / 0.2, `FillProbe::blocker` 0.6 /
0.0 / 0.1. The search proper was left alone (cheaper expansions were
exhausted in round two); the rest, with the share of the code each change
touched (Castor / jetson / k30):

- The pour analysis's own-stamp bitsets (`OwnStamps`) are found by a
  table indexed by map, not a SipHash map, asked once per node of the
  pour (4.0 / 0.9 / 1.7, most of it hashing).
- The search's neck class comes from the step's coordinates
  (`zone_class`), not `class_at` with a division per neighbour (1.4 /
  0.7 / 0).
- The tile planner computes each (layer, tile)'s entry cost once per plan
  (it was computed for each of up to eleven relaxations) and pops from
  the four-way `KeyHeap` with (cost bits, state) as one word, the order
  of the `(u32, u32)` pairs it popped before (3.0 / 5.9 / 8.7).
- A restamp builds the new list first (marks only) and then changes the
  counts only of stamps that went or came; the kept branches' stamps were
  decremented and incremented again on every reroute. The list, its
  order and the final counts and tile claims are as before (6.7 / 2.8 /
  4.0; `PCB_ROUTER_FINGERPRINT_ALL`, which hashes every occupancy map,
  identical). The diagonal footprint is mirrored in place (`turned`
  allocated a vector per diagonal node).
- `Board::fill_clearance` read `PCB_ROUTER_OLD_FILL_CLEARANCE` from the
  environment for every item a fill probe tests; now once (0.6 on
  Castor).
- The fill probe's index and hatch-hole memo hash with `FastHasher`
  (rustc's FxHash; iteration order unused) (0.4 / 0.1 / 0.2).
- `PourMap::build` looks its layer's maps up once, not per neighbour
  (2.1 / 0.5 / 0.8 touched).

After (a profile of all changes but the restamp, scaled to the same
search samples): Castor's run 11 % less CPU (own stamps and SipHash 3.5
points, planner 1.8, `class_at` 1.4, pour labelling and probes 1.5), k30's
9 % (planner 5.5, pour 1.8). Nothing in
the work accounting changed: the negotiation's limit is expansions and the
pour analyses count by their model (41 ns a node, 38 ns a stamp, 47 ns a
probe item). The model now overstates the analyses' wall time (Castor's
clean-up: 9.8 s of pour work in 6.7 s of wall, was 10.9 s wall), so the
budgets buy less pour checking than they could; recalibrating would move
results and wants its own measurement.

CPU, pinned simultaneous pairs in route mode (`--jobs 1 --threads 4`,
`PCB_MAKER_NO_CACHE=1` both sides, `cpu_seconds` of the rows; the base is
always main's code without this change; scripts `build/pairs/pair.sh`,
`run-pairs.sh`, `rows.py` in the worktree, rows under `build/pairs/r1`
to `r4`). The first side named ran on 0-7,16-23 except in r2:

| board | r1 x862 / x870 | r2 x870 / x862 (swapped) | r3 x855 / x871 | r4 x881 / x872 |
|---|---|---|---|---|
| Castor | 0.864 (511.0 / 441.3) | 0.915 (507.7 / 464.6) | 0.858 (561.8 / 482.2) | 0.862 (552.6 / 476.3) |
| k30-SBC | 0.888 (612.1 / 543.5) | 0.940 (524.8 / 493.4) | 0.939 (494.8 / 464.6) | 0.894 (644.0 / 576.0) |
| jetson | 0.895 (2462.6 / 2205.0) | 0.919 (1938.6 / 1780.7) | 0.878 (2236.4 / 1963.6) | 0.840 (2151.0 / 1807.3) |
| spell_tome | 0.960 (22.4 / 21.5) | | 0.995 (18.8 / 18.7) | 0.967 (18.0 / 17.4) |
| urchin | 0.958 (43.1 / 41.3) | | 0.891 (43.2 / 38.5) | 0.941 (37.6 / 35.4) |
| geomean | 0.912 | 0.925 (three boards) | 0.911 | 0.900 |

Ratios are new / old CPU. Over the four rounds (geomeans): Castor 0.87, k30 0.92,
jetson 0.88, the small boards 0.95 (KiCad's DRC more than routing:
spell_tome is 1.2 s of work in 18-22 s of CPU); the spread between rounds
is the machine's load (19-37 beside other agents). Every pair: the same
board file (md5), vias, length, work seconds and verdict; with
`PCB_ROUTER_FINGERPRINT=1` (r2, r4) the same fingerprints on every board
(jetson in r2 the same multiset: its side-by-side probes print in the
order they finish). During the work, `PCB_ROUTER_FINGERPRINT_ALL` (every
occupancy map hashed) at a fifth of the budget, two threads: Castor and
k30 identical.

Short tier (tasks v6) x871 against `build/short-x855.log`: all 12 rows
identical apart from CPU, wall, load and memory (vias, length, work,
unconnected, errors, first-route open terminals; compare.py: same 12,
unconnected 42 to 42, passes 6 to 6, cut 0 on both). The tier's CPU (5064
to 3185 s) is not a claim: unpaired, and the two runs saw different
loads. Rebased on the hole-clearance fix, x872 against
`build/short-x880.log`: again all 12 rows identical (compare.py same 12;
CPU 5216 to 4982 s, unpaired).

Next: the search proper is 70-80 % of route mode; outside it the largest
items left are the restamp's list building (the marks pass over every
branch on every reroute) and the clean-up's pour analyses (pour labelling
and the fill probe's distance tests).

## Freerouting refresh on x738 (2026-10-15, no code changed)

docs/benchmarks.md "Freerouting against our cold route, x738". Same picture
as the 2026-10-09 round (47 / 6 fewer unconnected, 51 / 8 clean where both
finish, 0.65x Freerouting's CPU), no board flipped sides. Our cold route
regressed on 11 boards since x136/x155: SNSP-CPU-01 55 to 178 open (all
GND; 362 of 363 connections "routed" while KiCad finds 178 unconnected
terminals: our connectivity and KiCad's disagree) and MegaDrive 95 to 137
repeat on a calm machine; katia hits the 1800 s wall at 14.9 GB. Assigned
to a cold-route regression agent (x900+). Fairness: Freerouting timeouts
above load 45 rerun, eight boards rerun Freerouting-only, 4 GB heap shown
harmless by A/B, keyboar_ is a parser hang (backtick in a part number), not
a timeout.

## Committed on 2026-10-15 (hole clearance)

Jetson's 8 hole_clearance findings (x855 layout) are one 3V3_SYS via at
(133.219, 122.186), 0.769 from the centre of J2's PTH pad 1 (GND; 0.75
copper, 0.55 drill). KiCad measures the via's copper (0.25 radius) to the
pad's hole (0.275 radius): 0.769 - 0.25 - 0.275 = 0.2438 where the
project's `min_hole_clearance` is 0.25; it lists the pair 8 times. To the
pad's copper the via kept 0.144 (0.125 needed), hole to hole 0.419 (0.25).
Not lattice rounding (the via sits on a 0.127 node, the pad off the
lattice; 0.775 needed, 0.75 kept) and not the narrow width: since b6e3530
a routed pad's plated hole was a hole obstacle of its net that blocked
other nets' tracks but not their vias ("vias stay with the pad copper
rule"), so vias kept only clearance plus copper, which with a 0.1 ring
and 0.125 clearance is 0.025 short of the hole clearance. The verifier
skipped it too (`blocks_vias` false), hence 0 internal violations.

Fix (crates/pcb-kicad/src/board_router.rs, crates/pcb-router/src/grid.rs):
the plated hole blocks vias; the static via map exempts the pad's own net
through `via_owner` where only the hole reaches (its copper still blocks
the own net as before). KiCad does not test a net's own copper against its
holes (every track to a THT pad ends on its drill; none of the 735
hole_clearance findings with a via or track in recent drc.json files pairs
items of one net). The verifier now checks other nets' vias against these
holes. Unit test `other_nets_vias_keep_the_hole_clearance_from_a_plated_hole`
(J2's geometry: lowering, static map for both nets, verifier 0.2439
against 0.25); it fails on x855 at the lowering and without the grid
change at the own-net exemption.

How common: the only finding of this kind in our routers' output. Of 3269
drc.json files of the last six days under build/ and the worktrees
(references and Freerouting left out), hole_clearance findings with a via,
track or zone are this via alone (jetson's x850, x851, x852 and x855
layouts). build/github-layout-v15 (100 boards) and v16 (39 so far): 20 and
9 boards with hole_clearance, every one footprint-intrinsic (pad to NPTH
of the same part, pad holes within J2, MX switch pins), none new against
the designer (v15's rows: no new hole_clearance).

Jetson in route mode, the a-x855 placement, the layout's narrow attempt
(`use_narrow_signals`, pours connect, no refine pitches, no retry seeds,
four threads; clean-up at its default 180 s, the 60 s cap is not a file
setting), three negotiation budgets per side since the result is chaotic
(identical up to iteration 48 at 405 s, where the via first differs):

| negotiation | x855 unconnected | x855 via findings | x880 unconnected | x880 via findings |
| --- | --- | --- | --- | --- |
| 380 s | 18 | 8 | 17 | 0 |
| 405 s | 11 | 8 | 16 | 0 |
| 430 s | 19 | 0 | 15 | 0 |

Sum 48 against 48; the other findings equal on all six (clearance 2,
courtyards_overlap 1, starved_thermal 4, the designer's 36
hole_clearance). Short tier x880 against build/short-x855.log (tasks v6):
all 12 rows identical (42 unconnected, 6 errors, 1176 vias, 22032 mm,
work 2390 s); none of its boards had a via in that ring.
Logs: build/repro in the hole-clearance worktree, build/short-x880.log.

## Committed on 2026-10-15 (first-route budget)

Jetson ended layout mode at 25 unconnected with no ladder at all. Where
its work went, x833 (four threads, `profile work:` lines, work seconds):

| phase | work |
| --- | --- |
| placement (longest seed; the pool's 79 s count toward the end only) | 52 |
| race: the two other placements' probes | 150 |
| first route: kept probe 75 + negotiation 478 (cap 556) | 553 |
| first route: hard repair 40, stitching and pour analyses 6 | 46 |
| move C33 (rejected, 39 open): negotiation 84, repair 37, stitching 3 | 124 |
| move C33 turned (kept, 30 open): negotiation 120, repair 31, stitching 2 | 152 |
| resume (82 s, 30 to 25 open): negotiation 83, repair 27, stitching 2 | 112 |
| clean-up, via reduction, spokes | 0 (polish needs 240 s, 218 left) |
| ladder | none (needs 240 s, 218 left) |

1203 s of work plus 79 s of pool: 1282 of 1500 (x832 before the pool:
1222 plus 46 of waves, wall-cut in the polish). The first route's
negotiation was not the problem (it stalled at 478 of 556 s); the moves
and the resume were: jetson's 31 open sat under the "many open" limit (5 %
of its terminals), so it was treated as nearly complete. On the boards
over that limit (module, DSP-ADAU1452, vaio_re) the ladder ran but its
probes and continued tracks rung took 900-1000 s, the reserved
narrow-signal step got 135-156 s or nothing, and no class-width attempt
beat the first route (x833: module 82, DSP 121, vaio_re 138).

Changed (crates/pcb-kicad/src/board_layout.rs): where the narrow-signal
step applies (`narrow_step_applies`), a board on four layers with pours
counts as far from complete above 2 % of its terminals (at least 10;
`far_open_limit`) as well as above the old 5 %; such a board skips the
moves, the resume and the polish, and instead of the ladder gets one
attempt of the step alone (`narrow_step_config`: signals at the narrowest
menu width, the minimum clearance where clearance findings are warnings;
the first route's pour mode, no finer pitches) with the work left: the
clean-up capped at a tenth of it (30-120 s, new `cleanup_seconds` in the
router config, not read from files), the negotiation three quarters of
the rest (60-900 s; `direct_narrow_budget`), its work deadline the
layout's end; the layout keeps it when KiCad finds it better.
`PCB_LAYOUT_OLD_FIRST_ROUTE_BUDGET` restores the old split. Unit tests
for the limits and the split. Boards without the step, and boards near
completion, take the old paths.

Layout runs, one per side, four threads each, unpinned, load 22-41;
before x833, after x855 (x855 is this on x862; x833 and x862 have the same
budget code, x862 changes hatched zones; the jetson, module, DSP and
vaio_re rows of x855 equal those of the x851/x854 runs built on x833).
Unconnected and errors are KiCad's (new errors against the designer's):

| board | unconnected | errors | vias | before: steps | after: steps |
| --- | --- | --- | --- | --- | --- |
| jetson | 25 -> 18 | 1 clearance -> 8 hole_clearance | 772 -> 710 | moves, resume, no polish, no ladder | narrow attempt (408 s negotiation, router 36 open) |
| A13 module | 82 -> 17 | 0 -> 1 track_dangling | 424 -> 490 | ladder, step skipped (154 s left of 319 kept) | narrow attempt (stalled at 103 s), a seed round |
| DSP-ADAU1452 | 121 -> 82 | 5 starved -> 0 | 186 -> 225 | ladder, step at 156 s: 258 | narrow attempt 169, four seeds 207 |
| vaio_re | 138 -> 35 | 0 / 0 | 551 -> 686 | ladder, step at 135 s: 530 | narrow attempt 76, one seed |
| Castor | 0 / 0 | 0 / 0 | 249 -> 167 | first route, moves, tracks rung | same; 167 is x862's (hatched zones; x862 alone gives 167) |
| keyboar_ | 0 / 0 | 0 / 0 | 39 / 39 | first route, moves | same |

No row was cut. Jetson's 8 hole_clearance findings are one via of
3V3_SYS 0.2438 mm from J2's PTH pad 1 hole (0.25 required; KiCad lists
it 8 times): a router hole-clearance rounding to look at, not the narrow
width. Two more boards with the step, on x833 against x854 with the same
rule (`PCB_LAYOUT_DIRECT_NARROW=always`, an experiment switch not kept):
Hub 30 -> 30, 187 -> 170 vias, 1378 -> 577 s of work; koeg 12 -> 7
unconnected, shorting_items 2 -> 5 (15 courtyard overlaps on both, both
runs wall-cut under load).

Variants measured on the way (jetson unless said): the narrow attempt with
half of what is left and the default 180 s clean-up (x850): 23, its
clean-up took 186 of 604 s; three quarters after a capped clean-up
(x851): 18, the negotiation still falling (21 nets in conflict); 0.85
(x852): 21, a plateau at 21 and another end; the step alone only when the
ladder could not afford probes plus two attempts of the first route's cost
(x851): module took it (17), DSP and vaio_re took the ladder and stayed at
121 and 138, which is why the rule now has no such threshold. A cap on the
first route itself was not needed on jetson (its negotiation stalled
under its cap) and would risk boards that complete in it.

Short tier x855 against x862 (same tasks, v6): all 12 rows identical, 42
unconnected, 6 errors, 1176 vias; against build/bench-short/short-x783.log
only 1337-v4.0's vias differ (64 -> 76, the recorded router change). No
short-tier board has the step, so none takes the new path. Logs in the
worktree's build/ (pcb-maker-jetson: six-x833, six-x851, a-x855, b-x855,
always-x854, extra-x833, extra-x854-always, jetson-x832, jetson-x852,
castor-x862), short tier build/short-x855.log.

Left: the narrow attempt does not use all the work on every board (Hub
ended at 577 of 1500 s): a class-width ladder after it with what is left;
a mode choice for the attempt (the first route's pour mode now; tracks
probed better on module, koeg and DSP); the via hole-clearance rounding
(not rounding: "hole clearance" below).

## Committed on 2026-10-15 (hatched zones in the tracks rung)

The tracks rung lowers its pours with `connect` false, and the fill probe
read a zone's hatch (and the outline edges) only from connecting pours, so
there hatched zones were modelled solid: the spoke verdicts, manual
spokes, island vias and stubs (only `add_inner_spokes` read the hatch,
through `hatch_any_plane`). The probe now reads every zone of the net as
KiCad fills it (`FillProbe::filled_planes`; `tracks_rung_fill`, on,
`PCB_ROUTER_OLD_TRACKS_FILL`): hatch, rings, hole grid and outline in
every rung. In the tracks rung the probe is used only after the routing
(the manual-spoke pass and its island repairs; the via reduction's starved
count is the corridor model), so the routing itself never saw the hatch.

Why reading the hatch alone hurt 0xCB (x860, route mode on v15: tracks
rung 46 vias, KiCad "2 spokes connected to isolated island" at MX3.S1 on
B.Cu): not the island rule and not the routing. MX3.S1 (a 2 mm PTH pad)
has two F.Cu tracks; KiCad's DRC counts a track connected to the pad that
starts in its ring and ends in this layer's fill, and one of them,
112.07 to 118.37 mm, ends in B.Cu fill. Our model tests a pad's straight
lattice run to its turn, which ran past that end (a junction: another GND
branch leaves it there, and `materialize` cuts the segment) to 98.77 mm,
outside the fill; the other run ends at 131.07 mm, 12 microns inside a
hatch hole (pcbnew agrees: not in fill). With the solid model the second
end was "in fill", so the model counted one track, found the pad short
with two island spokes, and added an island via; with the hatch it counted
no track, and with no track KiCad's rule lets the island spokes stand in
(spokes_short 0: no repair). KiCad counted the first track (1 counted, the
islands then ignored): starved. Fix: the DRC track rule cuts the run at
the first junction as the emitter does (`junction_nodes`;
`track_end_junctions`, on, `PCB_ROUTER_OLD_TRACK_RUNS`). The census
cannot see either change (it lowers with the pours connected and adopts
the board's exact segments). Router tests:
`the_tracks_rung_reads_a_hatched_zone_as_hatched`,
`a_track_ends_for_the_drc_at_a_junction`.

Hatched boards (`(fill ... (mode hatch`): 0xCB-1337 1337-v4.0, pcb, panel,
pcb-panel; Business-Cards Batch_1, USB_Keypad, WLED_Matrix; capybully;
le_chiffre stm32_chiffre_36keys and stm32_hotswap_chiffre; Qfwfq; the A13
module. Route mode on the v15 placements (tasks v6, `--jobs 1 --threads
4`), x832 (= main's router) against x861, `build/hatch/route-x8{32,61}.log`:
pcb-panel has no v15 placement and panel times out at 1800 s on both
sides; on the other 10 the same rung wins everywhere, unconnected 518 to
518, errors 14 to 14, starved thermals the same per board (Batch_1 7,
the rest 0), vias 1428 to 1427 (USB_Keypad 100 to 99), length 37025 to
37022 mm. 0xCB 1337-v4.0: tracks rung, 0 open, 0 starved, 47 vias on both,
MX3.S1's island via kept, and two manual spokes (B.Cu, four segments) the
hatched model no longer needs are gone (1953.2 to 1949.2 mm); capybully
-1.1 mm (two manual spokes fewer), Batch_1 +2.3 mm. Census of 44 boards:
x832, x860 and x862 identical (157 predicted, 12 missed, 27 false alarms,
2 not modelled), `build/hatch/census-x8*.txt`.

Short tier (tasks v6) x862 (this, rebased on main 9276fba) against
x833h (main's x833, run now as the base: short-x829 predates the work
accounting and the placement pool, which move 0xCB 1337-v4.0 to 76 vias
and 1611.6 mm on main itself), `build/short-x833h.log`,
`build/short-x862.log`: 10 of 12 boards byte-identical (placement and
result); USB_Keypad 100 to 99 vias, WLED_Matrix 2457.8 to 2459.2 mm (its
tracks rung adds 11 manual spokes, not 8; the model on x833h's board had
2 false alarms there), both 0 starved in KiCad on both sides; unconnected
42 to 42, errors 6 to 6, passes 6 to 6, vias 1177 to 1176, length 22030
to 22032 mm; OpenFC cut by the wall clock on both sides.

## Committed on 2026-10-15 (clean-up work accounting, x814)

The determinism agent left the clean-up's pour checks out of the work:
the pour analyses that keep a batch from stranding pour pads ran on wall
clock (Sisu: 21-47 s of pour checks in a 61-66 s budget; search work alone
covered 52-80 % of the clean-up's wall time), so a busy machine
stretched the clean-up and the route-mode guard (4 x the budget) could stop
it. Now every pour analysis counts as work in the phase budgets
(`budget_work` behind `clock` and `spent`: resolve, clean-up, via
reduction, thermal spokes): 41 ns per lattice node inside the net's pour,
38 ns per stamp of the net, 47 ns per item the fill probe indexes or looks
at (the spoke tests; `FillProbe` counts them). Calibration (route mode, one
thread, `PCB_ROUTER_POUR_TRACE`, seven boards): per board the model gives
0.89-1.07 of the analyses' wall time at load 17-22, 0.70-1.10 at 21-25
(x814, Castor and reCamera lowest); a lattice-only model spread 0.5-1.65
(the spoke tests are a third to two thirds of the time on reCamera and
Castor). At one thread the clean-up's counted work is now 0.98 of its wall
time on Castor (0.64 before). The verbose log has a `profile work:` line
(wall, search work, pour wall and work per phase) and the clean-up's last
line gives its work, its budget and the pour share. What stays uncounted
is in docs/router.md, "Determinism and the wall clock" (the negotiation's
stamping, the ladder's and the layout's tallies, `deadline_work`).

Route mode on v15 placements, x777 (main) against x814 side by side, two
jobs, load 29-35, one run each:

| | before | after |
|---|---|---|
| Sisu result | 7 unconnected, 408 vias, 5924.8 mm | identical (the winning attempt, signals at 0.1 mm on pours=tracks, has no pour checks) |
| Sisu clean-ups on the pour rungs | 404 / 308 / 234 analyses | 404 / 290 / 220; work 60.1 of 63, 69.3 of 66, 60.9 of 61 s in 80.9 / 80.5 / 61.8 s wall (work/wall 0.74-0.99) |
| Castor result | 0 unconnected, 3 starved (allowed 3, pass), 225 vias, 5186.2 mm | 0 unconnected, 4 starved (fail by one), 209 vias, 5156.1 mm |
| Castor clean-ups | first one finished its passes | each ends by work (60.1 of 60 s in 82.7 s wall, 60.3 of 60 in 59.6, 68.8 of 71 in 60.9) |

No wall guard fired on either side. Castor's change is the budget meaning
less work than before: the first clean-up stops earlier, the via
reduction starts elsewhere (16 vias fewer) and one pad more is starved;
a flip of the kind the short tier holds, not a fault of the accounting.

Short tier x814 (load 9 at the start, 7 at the end) against
build/bench-short/short-x783.log (both tasks v6): 12 of 12 the same by
compare.py (better 0, same 12, worse 0; unconnected 42 to 42, passes 6 to
6), 11 rows identical in vias, length and work. The one that moved,
1337-v4.0 (passes on both): vias 64 to 76, length 1594 to 1611 mm, work
169 to 130 s. Its first polish's via reduction (strict, 60 s) now counts
the pour analyses of its renegotiations and clean-ups, so after two rounds
it stops ("round 2 would take about 24s of the 15s left of 60s") where it
ran three (71 to 69 vias); the second polish starts from that board. Tests:
`cargo test --release -p pcb-router -p pcb-kicad` pass (new:
`pour_analyses_count_as_budget_work`). Logs: build/cleanup-x777,
build/cleanup-x814 (route mode), build/short-x814.log, calibration
build/cleanup-cal3-x813 and build/cleanup-val-x814.

Recalibrated 2026-10-15 (x817) after the router-overhead round (x872)
made the pour analysis about twice as fast (the old constants booked
Castor's clean-up 9.7 s of pour work for 5.2 s of pour checks, Sisu's
25.6 for 16.3): `POUR_NODE_NANOS` 41 to 18, `POUR_STAMP_NANOS` 38 to 26,
`PROBE_ITEM_NANOS` 47 to 42, fitted over three calibration runs at one
thread (load 15-31; build/cleanup-cal-x816, -x816b; the trace now also
prints the pour layers' nodes, which did not help the fit). Per board
0.71-1.08 of the analyses' wall time by the median, the big boards
(Castor, Sisu, reCamera, k30) 0.85-1.08; one board's ratio moves 0.6-1.2
between runs with the load, so 0.9-1.1 per board is beyond what one
calibration run can show. Route mode, x872 against x817 side by side (two
jobs, load 15-18): Sisu identical (7 unconnected, 408 vias, 5924.8 mm);
Castor back to its pre-accounting board (0 unconnected, 3 starved, pass,
225 vias, 5185.8 mm against x872's 209 vias, 5157.5 mm, also a pass now).
The clean-ups' pour work against their pour checks' wall time (four
threads): Castor 5.8 / 5.5 / 5.9 s for 5.1 / 4.3 / 4.9 s (x872: 9.7 / 9.0 /
9.6), Sisu 12.5 / 10.4 / 10.8 for 16.4 / 12.1 / 8.8 (x872: 25.6 / 20.1 /
19.0); each clean-up ends by work, no wall guard. Short tier x817 (load 17
to 10) against build/short-x880.log (tasks v6): all 12 rows identical
(vias, length, work, KiCad), 1337-v4.0 included.

Next: the ladder's and the layout's work tallies still count search
expansions only; counting the pour work there too (a `work()` in
`RoutingResult` instead of `expansions`) would make `deadline_work` and the
ladder's budget consistent with the phase budgets, at the price of shifting
more rows (every pour board's attempts), so it wants its own measurement.

## Committed on 2026-10-14 (placement pool)

The placement race ran its seeds in waves as wide as the machine's cores
(`available_parallelism`, 32 here whatever `RAYON_NUM_THREADS` said), each
wave as long as its slowest seed, and the waves' work counted toward the
layout's work end (`waves_work_seconds`). Now (`race_seeds` in
board_placer.rs) the seeds run in a pool of the process's search threads
(`RAYON_NUM_THREADS`), a seed starting as a thread frees; they are judged
in seed order as one after the other (seed k kept while the seeds before it
used less than the budget, three at least; ties to the earlier seed), as
the probe ladder judges its probes. The batches they start in are sized
from the kept seeds' work alone (the first holds the three, each later one
as many as the budget left holds at their mean work), so the seeds run and
the seed kept do not depend on the threads. A seed the budget would not
keep no longer runs: the waves ran one more seed after the budget was
spent and counted it (glasgow 260 s, the 0xCB pcb-panel 171 s, MIDAS-MK1.1
167 s of work for nothing). The clock counts the kept seeds as the
reference machine (eight threads, the calibration of
`EXPANSIONS_PER_SECOND`) runs them per batch (`pool_work_seconds`,
replacing `waves_work_seconds`); `board-placer.json` adds `seeds_run`,
`seed_kept` and `seeds_work_seconds` (their sum), and the layout's
`layout: placed` line prints them.

Six boards, one layout a side (x783 = main before, x830 = x783's tree plus
this; four threads, tasks v6, `PCB_ROUTER_NO_WALL_GUARD=1` on both so the
work alone decides, load 18-48; `build/pool/six-x7xx`). Wall and work in
seconds; work is the longest seed; the clock is what the layout counted:

| Board | placement wall | work | clock (waves / pool) | sum of kept seeds | kept / run | seed kept | final board |
| --- | --- | --- | --- | --- | --- | --- | --- |
| video | 421 / 457 | 439 | 567 / 567 | 1026 | 10 / 11 | 2 | identical, 263/371, 1238 vias |
| 0xCB pcb-panel | 471 / 314 | 170 | 491 / 320 | 652 | 4 / 4 | 4 | identical; both end on kicad-cli's 900 s zone refill (exit 4) |
| glasgow | 501 / 305 | 334 | 593 / 334 | 816 | 3 / 3 | 1 | identical, 112/226, 201 vias |
| Castor | 29 / 54 | 9.8 | 19 / 28 | 145 | 16 / 16 | 1 | identical, pass |
| USB_Keypad | 11 / 23 | 5.1 | 9.8 / 13.5 | 71 | 16 / 16 | 14 | identical |
| urchin | 8.8 / 13.8 | 4.7 | 8.4 / 10.7 | 55 | 16 / 16 | 6 | identical, pass |

The placements (poses) are identical on all six, so the kept seed is
x783's too; the result boards are byte-identical. glasgow's first route
negotiated with 467 s of work allowed instead of 337 s and took another
path (its fingerprints differ until the final ladder), but ended on the
same board. The placement's wall time follows the threads now: x783 ran up
to 13 seeds at once per board whatever the search threads, x830 four in
the tier, so boards of many short seeds place 1.5-3 times longer in wall
time (Castor 29 to 54 s, the same CPU); boards that ran a wasted seed are
faster. Placement alone with `RAYON_NUM_THREADS` 4, 8 and 16 on x830
(`build/pool/rayon`): seeds run, kept seed, clock and poses identical on
all six boards (walls 413/316/402 s video, 43/25/16 Castor, 308/286/274
the pcb-panel, 272/291/294 glasgow, 16/12/9 USB_Keypad, 10/7.5/6.4
urchin; the machine was loaded).

Short tier (x831 = main 6b3ced6 plus this, `build/short-x831.log`, against
`build/bench-short/short-x783.log`, both tasks v6): all 12 boards same,
every placement and every result board byte-identical; unconnected 42,
errors 6, vias 1165, length 22013 mm on both; work 2339 to 2428 s, CPU
4920 to 5127 s. The clock moved on every board (16-seed boards 1-5 s
more: the waves were 32 wide, the reference pool 8; less where a wave
waited on one slow seed or ran a wasted one: WLED_Matrix 177 to 113 s,
MIDAS-MK1.1 300 to 133 s, reCamera 75 to 52 s), and the work
left went into routing that did not win: WLED_Matrix's ladder ran its
two fine-pitch rungs (23 and 83 open against the kept 15; row work 282 to
358 s), MIDAS-MK1.1's first route negotiated 137 s instead of 53 s (334 to
347 s), reCamera's probe ladder ran where x783 had "deadline too near"
(its rungs lost). CPU: the pool runs the same seeds or fewer, so the
extra CPU is that routing; one pinned pair on reCamera
(`PCB_MAKER_NO_CACHE=1`, `build/pool/pair-x7xx`, load 40-58): 937 to 1061
s, the ladder's 4 probes and 3 rungs, identical row (55/56, 72 vias).
Tests: `placement_seeds_are_judged_in_seed_order_whatever_the_threads`
(threads 1-16, seeds finishing in reverse order, the sequential reference).

## Committed on 2026-10-14 (inner-pad spokes)

The exposed-pad model (above) found 0xCB-1337's two starved corner vias
but the router could not repair them: `manual_spokes` starts its tracks
from a pad's lattice nodes, and a via merged into the exposed pad has none.
`add_inner_spokes` (`inner_spokes`, on, `PCB_ROUTER_NO_INNER_SPOKES`; after
the manual spokes) gives such a via, where the spoke model finds it short
on a layer inside the exposed pad, a straight track of the net's width from
its centre into fill over the pad (in a hatched zone its own ring), its
copper wholly inside the exposed pad's, the shortest of sixteen directions,
kept only if the model then counts the via whole. KiCad's DRC counts a
track connected to the pad that starts in its ring and ends in fill as a
spoke; inside the exposed pad's copper the track keeps every clearance the
pad keeps. The tracks are exact copper (`NetState::inner_spokes`), counted
by the DRC track rule in `kicad_spokes`. Checked with kicad-cli on the
x825 board plus the two tracks before building it in: starved 2 to 0, no
other finding.

One trap: the tracks rung lowers pours with `connect` false, and the fill
probe read a zone's hatch only from connecting planes, so in that rung the
vias' rings did not exist and the model saw them unconnected (no finding,
no repair), while the ladder picked that rung's board. While
`add_inner_spokes` runs the probe now reads the hatch of any plane
(`Router::hatch_any_plane`); reading it in the tracks rung everywhere
changed 0xCB's tracks-rung routing (46 vias, MX3.S1 left an isolated
island), so it stays scoped. The tracks rung's fill model ignoring hatched
zones is a gap of its own.

Route mode on 0xCB's v15 placement (tasks v6), x825 against x828: the
same board plus two segments of 0.42 mm (length 1952.3 to 1953.2 mm),
starved_thermal 2 to 0, no new finding (annular_width 13 is the
footprint's vias), CPU 160 to 138 s. Census of 44 boards: x828 identical
to x825 (157 predicted, 12 missed, 27 false alarms, 2 not modelled). Short
tier (tasks v6) x829 against build/bench-short/short-x783.log: 11 boards
byte-identical, 0xCB-1337 the same board plus the two segments, its
starved_thermal 2 to 0 (it passed before on the designer's 3); passes 6 to
6, unconnected 42 to 42, vias 1165 to 1165, length 22013 to 22014 mm, CPU
4920 to 4951 s. An earlier tier against short-x791 was tasks v5 (the
list mix-up) and is void; WLED_Matrix on it was identical between x825
and x828. Router test:
`a_starved_thermal_via_inside_an_exposed_pad_gets_a_track_into_its_ring`.
Logs `build/fill3b-x8{25,27,28}.log`, `build/short-x829.log`.

## Committed on 2026-10-14 (ladder reservation)

The layout's ladder now keeps work for its last step, the best attempt
again with signals at the project's narrowest predefined width (plus the
minimum clearance when the project rates clearance findings as warnings),
on projects where that step applies (`narrow_step_applies`: 43 of the 109
v6 tasks have a narrower menu width or a relaxed clearance). In layout
mode the ladder's work end is about its budget, so the regular rungs used
to spend everything and the step never ran (module, DSP, Hub, vaio_re in
v15). Once the ladder's first attempt has run with connections open,
`narrow_reserve_share` (0.75) of that attempt's work, at least 120 s and
at most 0.4 of the ladder's work and 600 s, is held back: every later
rung, the seed retries and the seed rounds check the work left without it.
The step runs when half the reservation and at least 180 s are left (the
floor: on jetson, with 70 s left, a 60 s negotiation took 232 s and was
useless), negotiates a third of what is left with its router's work
deadline at what is left (it had no work budget when it varied a probed
rung, so it ran to the wall clock: DSP 619 open in v15), its work counts
in `work_seconds`, and `skipped` says why it did not run. Route mode and
projects without the step are unchanged (no reservation: the code paths
are gated on it). A limit on how many rungs run side by side was built
and removed: the rungs are judged in order either way, so it only
serialised waves and cost wall clock (Telemetry).

Layout runs, one per side, pinned (A 0-7,16-23, B 8-15,24-31, 4 threads
each); unconnected is KiCad's count, errors KiCad's; "step" = whether the
narrow step ran (router's open count after it):

| board | base / ours | unconnected | errors | vias | CPU ours/base | step base / ours |
| --- | --- | --- | --- | --- | --- | --- |
| A13 module | x747 / x772 | 81 -> 18 | 1 / 1 (pre-existing) | 414 / 404 | 0.86 | no / yes (37) |
| vaio_re | x769 / x773 | 163 -> 73 | 0 / 0 | 604 / 572 | 1.02 | no / yes (165) |
| DSP-ADAU1452 | x791 / x774 | 129 -> 108 | 0 / 5 starved thermals | 188 / 185 | 1.07 | no / yes (230) |
| Hub | x791 / x774 | 30 / 30 | 0 / 0 | 187 / 187 | 0.90 | no / yes (176, not kept) |
| MokyaLora (passes) | x791 / x774 | 0 / 0 | 0 / 0 | 334 / 334 | 0.96 | yes / yes (bounded: 157 s) |
| TelemetryOnboard | x791 / x774 | 1 -> 0 (pass) | 0 / 0 | 102 / 108 | 0.93 | no / yes (0); both wall-cut |
| jetson | x769 / x773 | 25 / 25, identical | 1 / 1 | 782 / 782 | 0.96 | no ladder (first route 1283 of 1500 s) |
| ghoul | x791 / x775 | 9 / 9, identical | 1 / 1 | 604 / 604 | 1.00 | step blocked: wall cut in placement |
| OpenFC (no step) | x791 / x775 | 1 / 1, identical fingerprints | 0 / 0 | 159 / 159 | 1.07 | - |
| protoesp (no step) | x791 / x775 | 92 / 92, identical | 2 / 2 | 295 / 295 | 1.00 | - |

The bases moved during the work (determinism, fill round two, exit codes);
each pair has both sides on the same base. Earlier pairs on x738 (before
the determinism change): module 81 -> 13, vaio_re 101 -> 21, Hub 35 -> 29,
DSP and koeg unchanged. Jetson cannot be helped by the ladder: its first
route takes the layout's budget (in route mode the step takes it from 48
to 6 open); a narrow first route when the project has the step is the
lever there, untested. Narrow is not a better default for the ladder's
first attempt: on designer placements (route-v12) the step beat the whole
class-width ladder on 9 of 16 boards (DSP 114 to 12, jetson 29 to 8,
vaio_re 36 to 16, Telemetry 8 to 0) and lost on 5 by 2-28 open, and in
layout v15 it lost on koeg (164 against 134) and Explorer (8 against 4).
The narrow step's DRC risk: the widths are the project's own menu (at or
above its minimum width), so KiCad raises no width finding; DSP's 5 new
errors are starved thermals of the narrow attempt's pours. Quick tier
x776 (this change on b2f5fbf, run on github-tasks-v6): no tier board
reserved or ran the step (none prints the reservation line). Its first
comparison, against build/quick30-x768.log, was not one: x768 ran on the
tier's default list then, github-tasks-v5, and v5 and v6 differ in the
constraints of 12 of the 30 boards (v6 fixes more parts: Castor J2 at the
designer's place instead of an edge constraint, BAGEL U102, USB_Keypad
and WLED_Matrix M1-M4, ...). Exactly those 12 rows changed (BAGEL 319 to
360 vias, Castor 230 to 249, USB_Keypad 81 to 100, WLED_Matrix's
connection_width 2 to 11; framework_mobo_lefthalf passes on v6); the 18
boards with the same constraints gave identical rows (1738 vias, 56520 mm
both). Castor swapped: x776 on v5 gives x768's row (230 vias, 4746.5 mm),
x768 on v6 gives x776's (249, 5601.1), same ladder and via-reduction lines
(`build/via-bisect/`). No commit between x768 and b2f5fbf changed routing:
the `spent` change of the exit codes only acts without a deadline (route
mode), the DRC cache only adds counters. The reservation's tier
comparison is x776 against a v6 base, build/quick30-x783.log (pending:
not finished when this was written). Logs `build/ladder-reserve/`
(summary.py there), `build/quick30-x776.log`.

## Committed on 2026-10-14 (placer fixes, x792)

Three placer defects from the v15 triage (`build/triage*`).

**1. The work clock under-counted the placer** (`placer phase` lines now
print the thread's CPU time; the ratio is work over CPU). The legaliser
booked 3-15 % of its CPU time on the 0xCB panels, the annealer 25-50 %
on boards with many relations, against 100 % on keyboar_, so the
layout's wall guard fired and results followed the load. Profiled with
callgrind (instructions per counted unit; about 42 on keyboar_'s anneal,
the reference) and fixed by counting what is done (details and the table
in docs/placer.md): cutout polygons per edge visited (the panels' slots
have thousands of points), box lists and pairs as the annealer counts
them, the other part's box and the linked lookup per same-side test,
relation anchors (glasgow's capacitors, 23 supply pins each), pin
positions (3 units), held parts' pad offsets, edge and region scans.
Exact speed-ups too: one pass over a cutout with a box prefilter (the
pcb-panel's legalisation 224 s to 28 s of CPU a pass), pads' edge offsets
only for parts an edge holds. Ratios, pairs on the two threads of a core
(before / after, whole placement): 0xCB pcb-panel 0.11 / 0.95, 0xCB panel
0.11 / 0.77, glasgow 0.40 / 1.08, vaio_re 0.72 / 0.98, MIDAS-MK2 0.52 /
1.07, Castor 0.68 / 0.98, urchin 0.86 / 1.08, keyboar_ 1.02 / 1.02;
phases now 0.63-1.6 (refine and repair on the panel up to 2.2, global
1.5-2.3 as before) where they were 0.03-1.5. A seed does exactly what it
did: only where budgets stop moves. The layout's placement step on the
tier's 30 prepared tasks (16 seeds, x738 against x780): 29 identical;
MIDAS-MK1.1 keeps 5 seeds instead of 12 (its seeds booked 0.45 of their
CPU time, now 1.05: twelve were 2.4 times the budget), wirelength 2119 to
2255 mm, the same 11 unconnected in the tier. Rebased on the determinism
agent's work (x781 = x747 + this): the waves' work now counts each wave's
slowest seed at its real cost too. Layouts, one run a side (x791 =
main against x792 = main + this, four threads each, load 16-49): 0xCB
pcb-panel, x791 never finished placing (1800 s timeout, cut); x792 placed
in 329 s (170 s of work, 321 s more in waves), routed (177 open after the
first route) and ended on kicad-cli's 900 s zone refill (open problem 7,
no KiCad count). 0xCB panel, x791 placed in 1248 s for 34 s of work and
was cut twice (router deadline, no final ladder), 136/188 routed, 213
unconnected; x792 placed in 222 s, cut once (a router deadline, 14 s of
its work done, load 30), 152/188, 86 unconnected. glasgow, both
identical (112/226, 263 unconnected, no cut), placement 755 to 544 s of
wall. vaio_re, no cut either side, 163 to 138 unconnected (8 seeds
kept instead of 11).

**2. Tabbed panels** (`keep_copies`, on): copies told apart by their
nets (KiKit's `Board_N-` names) and repeated references, each with fixed
parts apart from the others', keep their movable parts in their copy's
region (cut halfway between the copies' fixed parts). The 0xCB
pcb-panel and panel (four copies each) qualify; no tier board does. A
repeated reference in a constraint now names its parts in turn (the
molecules' J3 had a left, a top and a bottom edge at once).
Parts whose nearest fixed part belongs to another copy, the 0xCB panel:
13-20 in v14, v15 and on x791, 0 on x792; pcb-panel on x792: 2 (C47 and
C1, next to a cut line). The PolyKybd molecules share their net names across
copies (one net group), so the copy rule cannot see them; with the
reference fix 5x4_wave places legally, the other four still fail: the
designer stacked alternative connectors (J3 on J5, J4 on J6, pad on pad)
on the one spot per tile the fixed switches leave, which our rules
cannot reproduce. OpenESC's 4in1 panel is one net group too (its error
is crowding); the 20x20 4in1-mini is a 4-in-1 board, not a panel.

**3. Far-side courtyards of parts on one side** (`far_courtyard`, always
on): the legality test and the annealer keep one part's courtyard on the
other side from the other part's far side (holes, pads, courtyard).
woagboard's back-mounted choc switches: 28 courtyard overlaps in the
placement to 0 (KiCad DRC on the placed board). Not the cause elsewhere:
koeg-board's 15 are the designer's own (fixed switches against netless
mounting holes, both where the designer put them; the designer's board
does not load in kicad-cli, exit 3, so the runner cannot subtract them);
Quanta75's 45-66 were the runner's: its mounting holes are all "M2.5",
some on GND, so the GND ones were stacked and then held at the stack by
the netless ones' "M2.5" in `fixed`. run.py now leaves a netted
footprint in place when a netless one shares its reference (Quanta75
both 0 overlaps placed; it also changes WLED_Matrix's LDO1 and Hub's
REF**). Layouts on x792 (one run each): woagboard 26 courtyard
overlaps and 25 unconnected on x791 to 0 and 8 (one items_not_allowed;
both runs cut by the wall clock at load 20-50); Quanta75 Stamp passes
(0 overlaps, 162/162; v15 45 overlaps), Quanta75 Bare 0 overlaps (v15
66), 126 unconnected, 8 starved thermals (cut). The layout's verdict
could name the parts a repeated reference or a copy region held when a
placement fails (`unplaced`, `next_actions`); not done.

Quick tier x782 (x769 + this) against x768, both on the tier's own task
list (github-tasks-v5): 29 of 30 rows identical (routed, vias, length,
KiCad findings); WLED_Matrix 4 to 5 unconnected, from run.py's change
(its netted LDO1 now stays where the designer put it); 23 passes both,
unconnected 25 to 26, vias 2814 to 2824, CPU geomean 0.94, no cut rows.
The same binary's placements on github-tasks-v6 (the 30 tier boards
prepared once, x738 against x780): 29 identical, MIDAS-MK1.1 as above.
Logs `build/quick30-x782.log` (v5), `build/quick30-x781.log` (v6, 23 of
30; framework_mobo passes on v6's constraints, which fix M1).

## Committed on 2026-10-14 (exposed-pad thermal vias)

The largest class left by fill round two: 0xCB-1337's U3.45, ten
starved-thermal findings on six census boards the model had no row for
(thermal vias of the QFN's exposed pad, pads of its number merged into it,
which the model checked only where they are copper of their own). What
KiCad 10 does (zone_filler.cpp, drc_test_provider_zone_connections.cpp;
KiCad's filled polygons drawn per outline): every thermal pad
gets its relief, a hatched zone's ring (pad + gap to pad + gap + spoke
width, clipped by the zone outline only, then by other nets' clearances)
and four spokes from its centre, kept where their ends land in that fill
or meet another spoke, a pad inside a larger same-net pad included; the
thermal pads' plated holes are cut from the fill at the end. The DRC checks
each pad by its own ring half a gap out (crossings with fill outlines; an
outline over the larger pad holds both pads, no island) and its own
connected tracks (KiCad's connectivity: any track whose copper overlaps the
via, the exposed pad's tracks crossing it too). 0xCB's GND zone is hatched:
each via has a ring 0.4 to 0.7 mm and four spokes into it; the vias at the
exposed pad's corners lose their ring and outer spokes to the QFN pins'
clearances and keep the inward spoke: one, starved. In a solid zone the
larger pad's relief takes the vias' spokes and the DRC sees the larger
pad's own spokes crossing a via's ring (mackerel's mounting holes on In1:
2 on the spoke axes, 0 off them; solid there, so not in the census).

Modelled (`inner_pads`, on, `PCB_ROUTER_NO_INNER_PADS`): merged pads get
spoke records on every layer, reliefs and rings (`thermal_ring_pads`);
spokes of pads of one terminal meet; a landed spoke of a pad inside its
terminal's pad counts; foreign spokes across such a pad's ring are sampled
from their centre and count twice when they run through it; a merged pad's
DRC tracks are those overlapping its copper; no fill in a thermal pad's
plated hole. The earlier attempt's 15 false alarms came from the missing
rings (the vias' spokes found no fill), terminal-wide track ownership and
the track to the centre via's hole. Separately (`track_end_tolerance`, on,
`PCB_ROUTER_OLD_TRACK_ENDS`): the DRC's track-end test keeps other nets'
clearances to 0.5 microns instead of the spokes' 3 (`TRACK_END_TOLERANCE`);
four of the ten have a track from the corner via ending 1.5 microns outside
a sliver of fill between pins U3.23 and U3.24. Half a micron is KiCad's
DRC epsilon (`BOARD_DESIGN_SETTINGS::GetDRCEpsilon`, ADVANCED_CFG
`m_DRCEpsilon` 0.0005 mm, 500 nm read back in pcbnew), which its clearance
providers subtract from every clearance (`sub_e`); the zone-connection
check's point test uses none and the filler prunes with 1 micron, so the
number is KiCad's clearance tolerance, not that check's. Exact 12 missed /
29 false alarms, 0.5 microns 12 / 27, 1 micron 16 / 27, 3 microns 17 / 27.

Census, same tooling and KiCad cache (x801 = x820 = x768 before; x824
after, x823 before the rebase onto the DRC cache, identical):

| | x801 | x824 | x823 without inner_pads | x823 without track_end_tolerance |
|---|---|---|---|---|
| DRC starved predicted | 146 | 157 | 147 | 152 |
| missed | 13 | 12 | 12 | 17 |
| false alarms | 27 | 27 | 27 | 27 |
| starved pads not modelled | 12 | 2 | 12 | 2 |

By cause (`causes.py`, missed incl. not modelled): missing 12 to 2, tracks
4 to 3 (PixelWave's D45.4, the tolerance), the rest unchanged; false alarms
unchanged by cause. Every thermal pad inside a pad in the census (0xCB's 78
on six boards, Hub's U16.9_1/_2, Quanta75's U1.31; `pads_in_pads.py`,
mackerel's and PixelWave's are solid): 85 rows, 10 starved found, 75 agree,
none missed or false. Three router tests (hatched rings, the spoke through
a via's ring, merged pads on all layers).

Route mode on 0xCB's v15 placement, x801 against x824: identical boards
(47/47, 47 vias, 1952 mm, KiCad unconnected 0), starved_thermal 2 both (the
two corner vias, which the model now predicts), annular_width 13 (the
footprint's vias) both. The router does not repair them: `manual_spokes`
skips merged pads (no nodes of their own). A repair would be a short track
from the corner via inward into its own ring, which the DRC counts; not
done. Logs `build/census/x801.txt`, `x824*.txt` in the fill3 worktree;
`build/fill3-x801.log`, `build/fill3-x824.log`.

## Committed on 2026-10-14 (DRC cache)

KiCad's DRC (`kicad-cli pcb drc`, 6-40 s of wall clock, about 20 s of CPU
and 2.2 GB per call) is now cached on disk (`crates/pcb-kicad/src/drc_cache.rs`,
hooked into `native_report::run_kicad_report`, so every DRC of a route,
layout, verify-kicad-rung and the reference check goes through it; ERC is
about 1 s and is not cached). Key: SHA-256 (sha2 is in the tree) of the
kicad-cli version, the DRC arguments, the board's file name (the report's
`source`), every KiCad design file under the board's directory by relative
path and bytes (`.kicad_pcb`, `.kicad_pro`, `.kicad_dru`, `.kicad_sch`,
`.kicad_sym`, `.kicad_mod`, library tables; hidden entries skipped), the
global `fp-lib-table`/`sym-lib-table` and the `KICAD*` environment.
`kicad_common.json` is left out on purpose: kicad-cli rewrites it on every
run. KiCad's installed libraries are not hashed (they feed only the
`lib_footprint_*` warnings, which never count). Value: KiCad's report
bytes; a hit is validated like a fresh report and published with a rename.
Entries are `<key>.json` in `$PCB_MAKER_CACHE_DIR/drc`, else
`$XDG_CACHE_HOME/pcb-maker/drc`, else `~/.cache/pcb-maker/drc`;
`PCB_MAKER_NO_CACHE=1` turns it off; temporary file plus rename; least
recently used out beyond 2000 entries or 2 GB (a hit touches its entry);
any cache failure falls back to running KiCad. Visible as `drc_cache:
{calls, hits}` in verdict.json (and the reports' `verdict`) and on the
`board check:` stderr line, which also gives the lowering and lattice
totals so far. Snapshot x801.

Correctness (verify-kicad-rung, fresh with `PCB_MAKER_NO_CACHE=1`, then a
miss, then a hit, each in its own copy of the project): on Castor's routed
board, spell_tome, fence-esp32 (with schematic parity), jetson and Hub the
hit's drc.json is byte-identical to the miss's and verification.json is
identical across all three. Fresh against miss (two fresh runs) differs on
jetson and Hub beyond the date: KiCad's DRC is not byte-deterministic (the
order of findings, and which item pair names a missing connection); the
counts per type agree. One byte in the board (a footprint coordinate) and
one byte in the `.kicad_pro` each missed and added an entry on all five.
Unit tests: key derivation (every input changes it, unrelated and hidden
files do not, length-prefixed fields), atomic writes under eight
concurrent writers, eviction; an integration test with a fake kicad-cli
(two DRCs of one board: one KiCad run, one entry, identical bytes; a
changed byte or argument misses; a corrupt entry is dropped).

Savings, x801. Per call (verify-kicad-rung): Castor 5.8 s wall / 21.8 s CPU
to 1.1 s / 1.1 s (the rest is the preview SVG export, uncached), jetson
17-23 s to 6 s. Castor route mode on its v15 placement: 2 DRC calls (the
result and the reference), cold 557 s wall / 598 s CPU, warm 489 s / 578 s,
same board and drc.json; the board check's KiCad 38.3 s to 0.7 s. Short
tier (4 jobs, 2 threads, `build/drccache/short-{cold,warm}.log`): 54 DRC
calls, warm 53 hits (WLED_Matrix's cold run was cut by the deadline, the
warm one not, so its final board differs); 11 of 12 results identical;
tier wall 873 to 683 s, CPU 7008 to 5167 s (load-sensitive), and the
layout's peak memory drops from 2.1-2.3 GB to its own (0.4-1.4 GB) where
every DRC hits. run.py's designer DRC calls kicad-cli itself and is not
cached.

Within one run (cold cache) almost no DRC is on a byte-identical board:
spell_tome layout 1 call, Castor layout 2, Castor route 2, jetson route 5
(4 attempts and the reference), 0 hits each; the short tier 54 calls, 1
hit (reCamera) and 2 identical boards checked side by side (51 entries).
v15 layouts make 0-19 DRC calls (median 2). The cache pays across runs:
reruns, A/B pairs on the same placement, the reference board.

Follow-up (x802): `pcb-maker drc-kicad-board <project-dir> <board-id>
<report.json> [--no-refill]` runs the cached DRC on the board and its
rules as the reference check does (`pcb_kicad::kicad_drc_report`; exit 0
no error that counts and nothing unconnected, 1 otherwise, 2 input, 4
KiCad failed; `tests/cli_exit_codes.rs` with a fake kicad-cli). run.py's
designer check and freerouting.py's designer and Freerouting-result checks
use it when `<binary> --help` lists it (probed once per binary; older
snapshots keep calling kicad-cli). Its report has every severity; the
designer counts (errors only, not excluded, not cosmetic) equal the old
`--severity-error` counts on qfwfq (546, at KiCad's 199 cap), WLED_Matrix,
USB_Keypad, Castor, 1337-v4.0 and MIDAS-MK1.1. run.py rows gain
`designer_drc_seconds`. Short-tier splice of spell_tome and Blue_Line
twice: the designer DRC 7.2 s to 0.4 s and 4.6 s to 0.4 s, the layouts'
own DRC hits too (CPU 100.5 to 91.9 s and 53.8 to 44.5 s), same results.

Lowering and lattice building (the second target): measured, not cached.
jetson route 7 lowerings 1.3 s and 7 router builds 15.4 s (summed over
threads) of 1558 s wall; Castor route 4 builds 4.3 s of 557 s; Castor
layout 22 lowerings 0.1 s, 4 builds 2.6 s of 904 s. Under 1.1 % everywhere,
below the 5 % bar.

## Committed on 2026-10-14 (benchmark tooling: short tier, route mode, splicing)

Florian's feedback: we spent most of the day waiting for CPU, because every
agent ended with a serialised full-budget 30-board tier and paired every
measurement. `benchmarks/agent-tasks/README.md` "How to measure" now states
the discipline: the short tier (`--tier short`, 12 flip-prone boards from
`flippers.py` over x644..x782, a third of the budget via `--budget-scale`,
about 600 s at 6 jobs; WLED_Matrix gets wall-cut above load 45 at that
budget) for iteration; the 30-board tier at full budget only as the merge
gate; quality judged by one run per side (the layout is deterministic unless
`cut`); pinned simultaneous pairs only for CPU claims; router questions in
route mode on fixed placements (`--route-from <sweep dir>`, verified
identical to manual routes on two boards); incremental sweeps with
`--splice-from <log>` (rows marked `spliced`, compare.py shows SPL); an
objectives line (unconnected, new errors, vias, length, CPU, work, cuts,
first-route open terminals) in the summary and summary.json.

## Committed on 2026-10-14 (CLI exit codes)

A benchmark agent found `route-kicad-board` exiting 1 on most harvested
boards with 0 open connections: the CLI failed on any error-severity DRC
finding, the designer's own included (USB_Cable_Tester on its v15
placement, x782: 54/54 routed, KiCad 0 unconnected, one `courtyards_overlap`
H4/J2 that the designer's board has too; Castor: 126/126, KiCad 0
unconnected, 4 `solder_mask_bridge` and 3 `starved_thermal`, exactly the
designer's board's, plus a cosmetic `footprint_type_mismatch`). The
layout's exit code used the router's count instead (pads joined through a
pour's fill counted open), and every error, a missing file and a panic
alike, exited 1 (a panic 101).

The contract now (`pcb-maker --help`, docs/agents.md section 4):

| Exit | Status | When |
| --- | --- | --- |
| 0 | complete | KiCad: no unconnected item, no error beyond the reference board's per type (cosmetic types and warnings never count); layouts and placements also place every part and keep every constraint |
| 1 | incomplete | finished, result written, but not complete |
| 2 | usage_error | arguments, missing or invalid inputs, unknown keys or parts (`pcb_kicad::input_error` marks the library's input-stage errors), output directory exists (now checked before the run, not after an hour of routing) |
| 3 | cut | incomplete and `cut` non-empty (a complete result with a cut is 0) |
| 4 | internal_error | other errors and panics (one stderr line; the panic hook keeps the default report only with `RUST_BACKTRACE`) |

The rule is run.py's: KiCad decides, not the router's count; the
reference's errors are subtracted per type (`pcb_kicad::verdict`, unit
tests there). The reference is `--reference <project-dir>`: route mode
defaults to its input as given (for a harvested board, the designer's
routed board, which is run.py's `designer_findings`), layout mode to none
(its input is stacked parts); it is checked only when the result has errors
that count (one more KiCad DRC, `reference-drc.json`). Schematic parity is
reported, not judged (run.py does not judge it either; the old route check
did).

Every route, layout and placement writes `<out>/verdict.json` (also on an
error once the output directory exists, and for a layout whose parts found
no place, which writes no board-layout.json, so that scripts' "report
missing" test still means "no layout") and the same as `verdict` in its
report: status, exit code, one-line reason, router and KiCad open counts,
new and pre-existing findings by type, cosmetic and warning counts, cut,
work and wall seconds, absolute output paths, skipped ladder steps and
next actions. board-router.json gained `work_seconds` (the ladder's work)
and `skipped` (the narrow-signal and minimum-clearance steps, finer
lattices and further rungs that did not run while connections were open,
with why). run.py records `exit` and `verdict` per row and takes a missing
report's error from verdict.json; its pass rule is unchanged. freerouting.py
and the corpus and PCBench runners read only the reports; they are
unchanged (their route runs now pay the reference DRC on boards whose
result has errors: about 10-50 s of wall clock, not in `routing_seconds`).

Found on the way: every route-mode run that skipped the probe ladder
reported `cut: ["ladder"]` (x746 to x782): the ladder's `spent` noted a
cut whenever the wall clock over 4 was ahead of the work, which at the
ladder's start (0 s of work) is always. The guard now takes over only when
the wall clock is ahead by more than a twentieth of the ladder's budget
(60 s of work by default); below that the shares are the work's alone.
USB_Cable_Tester routes the same board (86 vias, 2292.4 mm).

Verified: USB_Cable_Tester route on its v15 placement, exit 1 (x782) to 0
(x790), the courtyard overlap pre-existing; Castor (wntrblm mainboard, v15 placement) exit 1
to 0 with the same board as x782 (126/126, 225 vias, 5186.2 mm), its 4
solder_mask_bridge and 3 starved_thermal pre-existing in the input as in
the designer's board, 419 s wall for 399 s of work, the reference DRC
included; the tiny
three-pad fixture in route, place and layout mode (complete, unplaced parts,
a missed `near`), usage errors and a forced panic
(`tests/cli_exit_codes.rs`, `PCB_MAKER_TEST_PANIC`).

Left for agents to work around: a process killed by a signal (a harness's
timeout, the OOM killer) has no verdict; `import-kicad-netlist` now exits 1
when footprints are missing; research commands keep 0/1/2 (+4 on a panic)
without the incomplete/internal distinction. Tests: pcb-kicad, pcb-router,
pcb-placer and tests/cli_exit_codes.rs pass. Three tests of the root
crate's tests/kicad_ladder.rs fail, before and after this change (the
research ladder, code this change does not touch; a 2026-09-30 test build
fails the first one too): native_one_connection_transaction_commits_or_exactly_rolls_back,
native_shared_tree_insertion_reuses_copper_without_a_via_cluster and
trace_tension_can_move_a_real_rung_one_footprint_without_breaking_kicad.
A closed stdout (`pcb-maker --help | head`) ends quietly with exit 0 (or
the verdict's code when a run already had one).

## Committed on 2026-10-14 (fill round two, x768 / x769)

The census of 44 boards (`experiments/pour-fidelity/census.py`, KiCad's
starved-thermal findings against our pour model) before and after, same
tooling (the tooling now reads standard layer names; rosco's In1.Cu is
called "Route2"):

| | x760 (= x738 model) | x768 |
|---|---|---|
| DRC starved findings predicted | 129 | 146 |
| missed | 28 | 13 |
| false alarms | 90 | 27 |
| starved pads not modelled | 14 | 12 |

What changed, all on by default, each with the census step it bought (env
switches for A/B): track ends found anywhere across a trimmed spoke; spokes
meet at any angle; pad clearance overrides replace the zone clearance in the
fill, also when smaller (`Board::fill_clearance`, `PCB_ROUTER_OLD_FILL_CLEARANCE`);
sliver ends; spokes in the pad's own frame (`Board::pad_spokes`,
`PCB_ROUTER_OLD_SPOKE_FRAMES`; the routing guards keep the box axes, frames
there cost Hub 29 to 35 unconnected); repeated pad numbers checked on layers
where they are copper of their own (`Board::merged_pads`); no relief cuts a
spoke in the DRC track-end test; cut-off spokes bring no item; hatched zones
(`Plane::hatch`: spoke end half a web out, a ring around thermal pads,
KiCad's hole grid); coincident pads share tracks; foreign spokes across a
pad's ring count (`PCB_ROUTER_NO_FOREIGN_SPOKES`); tracks and vias are items
of every piece their copper overlaps (`PCB_ROUTER_OLD_TRACK_PIECES`); 3 micron
tolerance on brush clearances; joined outlines for a pad's overlapping spokes
and lattice-cut necks (`PCB_ROUTER_NO_NECK_JOINS`). Router fixes: `reroute()`
refreshes the pours after negotiation; `trim_dangling_plane_ends` cuts a
plane tail whose fill was taken back (USB_Cable_Tester /SHLD: a layout move
rerouted signals over the plane end, not fill optimism).

Route-mode pairs against x738 (pinned, same time): Castor starved 4 to 3,
k30 1 to 0, mackerel-30 3 to 2, Hub 4 to 4 with 29 unconnected (x761 and
x765 had 35 from spoke frames in the guards, fixed in x767); CPU 0.94 to
1.05. Quick tier `build/quick30-x768.log` against x746: pass 22 to 23
(USB_Cable_Tester), starved 7 to 6, vias 2847 to 2814, unconnected 22 to 25
(reCamera 4 to 9, its ladder went another way; MIDAS 11 to 9). Eleven router
tests, one per class. `experiments/pour-fidelity/render.py` draws KiCad's
fill against our map around a pad.

Fragmentation measured on 7 boards: our extra pieces are lattice sampling of
necks; PixelWave's 1212 single-node blobs hold no spoke and no item; of 23
island spokes in outlines our map split, 4 pads had a wrong verdict and the
neck and spoke-overlap joins fixed 3. Remaining disagreements by cause:
0xCB U3.45 thermal vias inside the exposed pad (10 misses, modelled since in
"exposed-pad thermal vias"; modelling them
made 15 false alarms); seams between overlapping same-net zones of
different priority (Telemetry, 5; modelled since in "zone seams"); marginal fine-pitch clearance within a
few microns; hatched fill lying over thermal pads without relief (not
understood); per-track DRC subtleties with islands.

## Committed on 2026-10-14 (the determinism agent, x746 / x747)

Video's 262-392 unconnected for one placement was the layout's wall-clock
deadline (1.1 x 1500 s), not a race: in the four room-pair runs the
negotiation is identical to its last iteration and the runs part where the
deadline cut the hard repair (48, 70, 72, 98 nets rerouted). Video's work
lagged its wall clock (placement: 193 s of work, 630-1385 s of wall clock;
its seeds run in waves, each as long as its longest seed), so the deadline,
not the work, ended its layout under any load. Other wall-clock decisions
found: the via reduction's allowance in a layout (a third of the wall clock
left: Castor's final ladder 196 s on x738, 260 s of work now), the default
900 s negotiation of the attempts the ladder repeats from a probed rung
(wall clock only: video's narrow-signal step, 364 against 339 unconnected
on two runs), the pour nets' one-minute pre-negotiation in route mode, and
the probe ladder's check of the work left only between waves (the number
of rungs judged followed free memory). Route mode (Castor, Sisu: three runs
each) and the small boards' layouts were already deterministic.

Changed: the layout's routers stop at its end in work (router
`deadline_work`, armed before every phase; the end is `total_seconds`, the
wall-clock deadline 10 % later is the load margin); the waves of placement
seeds count toward that end (`waves_work_seconds`); the first route's
negotiation gets at most half of what is left; the via-reduction allowance,
the default negotiation budget and the pour pre-negotiation are in work;
probes are judged one after the other whatever ran side by side; every
wall-clock cut is logged (`wall guard: ...`) and reported (`cut` in the
reports and in run.py's rows, CUT in compare.py); `PCB_ROUTER_NO_WALL_GUARD`
drops the layout's deadline too. Video (seed 1, four threads): three runs
on eight logical CPUs 300/300/300 unconnected, 1362 vias, one fingerprint,
1355-1475 s; on four pinned CPUs it still meets the deadline (placement
alone 980-1385 s) and says so; without the guard there four runs of x744
and x746 give 318 and one fingerprint. Quick tier x746 (load 7-30): all 30
rows identical to x722, 22 pass, CPU geomean 0.97. The waves' count followed
the cores the process may use, so a run pinned to fewer cores could end
elsewhere than an unpinned one (fixed by the placement pool, below). Left: the clean-up's pour checks are not
work (a route-mode budget guard fired once under load 56, on a losing
rung). See docs/router.md, "Determinism and the wall clock".

## Triage of the v15 regressions (2026-10-14, no code changed)

A read-only triage of the 16 boards v15 (x723) lost against v14 (x510), by
pinned route-mode pairs on the same placement, layout repeats, and layout
with `solid_pin_rows` off. Outputs in `build/triage/` and `build/triage-*/`.

- The router is not the cause. On the same placement x510 and x723 give
  identical tracks-mode results (counts, vias, length; x723 reports fewer
  starved thermals), connect mode differs by a few connections either way.
  CPU x723/x510 0.98-1.06. Every x723 layout repeat reproduced its v15 row
  exactly unless the wall deadline fired.
- Ladder budget cliff (module 8 to 81 open, jetson 9 to 25, vaio_re): the
  last ladder step, signals at the project's narrowest predefined width plus
  the relaxed clearance, is what nearly completes these boards (module 126
  to 17 open, jetson 48 to 6 in route mode). In layout mode the first route
  eats the budget and the step's guards (`no_time_for_another`,
  `spent < 1.5 * ladder_budget`) refuse it. Assigned: reserve the step's
  cost or run it as an early rung.
- Placement perturbation (RP2040_base 100 to 173, Hub, SNSP, glasgow): a
  rule change shifts the legaliser's path and the race picks another seed;
  v14's RP2040 placement is legal under `solid_pin_rows`, the rule only
  perturbs (off: 100 open again). Seed-repeated pairs are the only sound
  judge of such changes.
- Wall clock not booked as work (0xCB panel, glasgow, vaio_re): the
  legalise phases book 0.10-0.15 of their wall time as work (panel 504-966 s
  wall for 34 s of work), so the layout deadline fires and the result
  depends on machine load (vaio_re 63 or 151 open for one placement).
  Assigned.
- Tabbed panels (0xCB panel, OpenESC 4in1, molecule panels): one outline
  piece, so `keep_on_pieces` never fires and 15-22 parts of one copy land
  on another copy; the router then reports 49 of 188 nets without a path.
  Assigned: confine each net group to its own fixed parts' region.
- Far-side courtyards (woagboard 26 courtyard overlaps; koeg-board and
  Quanta75 may share the cause): `legal.rs` `clear_of` checks far-side
  rects only between parts on opposite sides; KiCad checks two back-mounted
  parts' front courtyards against each other. Assigned.
- protoesp and vaio_re lost because v14's placements put parts inside
  module and connector bodies that `solid_pin_rows` now blocks (honest).
- None of v15's 45 passes relies on a bug: `inside_bodies.py` finds only
  fixed designer parts inside solid boxes, the designer-budgeted errors are
  internal to footprints. Caveat: qfwfq's hole_clearance sits at KiCad's
  199 cap, like the designer's.

## Committed on 2026-10-14 (the routing-room agent, x737 / x738): measured, built, off

Halo granted against halo used on 35 routed boards
(`experiments/routing-room/halo_use.py`, `analyse.py`; a part's band is
its body grown 1 mm; a 25 s probe's conflict and history maps from
`export-congestion-sample`): pin count does not predict where routing goes
(Spearman about 0 per board), RUDY in the band does (0.4-0.84, median 0.6)
and the probe's history likewise; 7-34 % of halo area is used and 0-74 %
idle (zpn 33 %, keyboar_ 48 %, 0xcb-static 74 %); keeping the half of the
halo area with the highest RUDY keeps 58-79 % of the routing inside halos
on crowded boards, the half by pin count about 50 %. `routing_room` (off;
`routing_room_share` 1.0, `_floor` 0.5, `_most` 2.0, `_all_parts` on;
`crates/pcb-placer/src/room.rs`): per-part demand from RUDY over each
net's spanning tree, halo = pin-count halo times min(2, k demand/median),
k by bisection so the total stays at share times the level's area; after
the normal placement a re-anneal with small moves, legalise, refine; a seed
whose pass fails keeps pin-count halos and sorts after the others. Seed
pairs (seeds 1, 101, 201, pinned): the option never fires on video,
OpenESC and hackrf (their kept level has no halos; hackrf walks down to
halo 0 because one part finds no room); on BAGEL, MK2.1, MK2, Castor and
zpn passes over 15 runs off 11, on 11 (BAGEL and MK2.1 win, where the level
had stripped small parts' halos; MK2, Castor and zpn lose 5-9 % HPWL as
busy clusters spread; zpn gains 1-3 mask bridges); CPU geomean 1.15x (the
second anneal on every seed). Share 0.5 gives HPWL -8 % on all five without
the routing collapse of the uniform x507 scale, but two passes lost to
single errors (within seed noise). Quick tier x722 to x737 with the option:
22 to 24 pass, both single-error flips (USB_Cable_Tester's track_dangling,
eternal-keypad's shorting_items), vias -3.7 %, 8 boards identical. Verdict:
implementation sound, idea as built a wash; left off. Found on the way:
video's routing is not deterministic for an identical placement (392, 314,
325, 262 unconnected for seed 1). Logs `build/room-*`,
`build/quick30-x737.log`.

## Committed on 2026-10-14 (the island agent, x722 / x723)

The islands are real in KiCad's fill, not our fragmentation
(`experiments/pour-fidelity/islands.py`): signal tracks and vias on the
plane layers cut them off in the tracks rung. KiCad experiments (kicad-cli
on mackerel-30 C50.2): a pad's own track ending inside the island cures
the finding, as does a track crossing it; a plated pad's fill stops at the
drill, so a track leaving a round through-hole pad between its diagonal
spokes touches none of them (the fill agent's "own track touches every
spoke piece" was wrong there). Changes, all on: `track_meets_spokes` (a
pad's own track joins a spoke's piece only where its first run overlaps the
spoke's strip outside the drill; census found 116 to 124, missed 36 to 28,
false alarms 87 to 90; `PCB_ROUTER_OLD_TRACKED`), `island_vias` (a short
pad whose manual spoke reaches no kept fill gets a stitching via from its
island to the net's main piece on another layer, also in tracks mode;
`PCB_ROUTER_NO_ISLAND_VIAS`), `island_stubs` (failing a via, a straight
track of the pad's own into its island, slivers included;
`PCB_ROUTER_NO_ISLAND_STUBS`). The stub changes no connection: the island
already hangs off the pad; it only satisfies KiCad's two-items rule. Its
broader form (stub every island-only pad) was rejected: 18 pads KiCad
passes for every 1 fixed. USB_Cable_Tester's bridges were not a pour
problem: KiCad exposes copper within `solder_mask_to_copper_clearance`
plus the aperture's expansion (0.1 mm there; our model had 0.075); now read
from the project (`solder_mask_to_copper_mm`) and used as the opening
clearance, always on. A pour keepout under openings exposing another net
was measured and not built: 1 real case in 128 results against 475 false
firings (`mask_bridges.py`). Pairs x710 to x722 (route mode, v14
placements): k30 starved 14 to 1 (+10 island vias), mackerel-30 19 to 3,
Castor 5 to 4, Hub 4 to 4, USB_Cable_Tester bridges 3 to 0; CPU within
±4 % paired. Quick tier x693 to x722: 21 to 22 pass (mackerel-08), 29 of
30 same, unconnected 22 to 22; USB_Cable_Tester's race picked another seed
whose /SHLD connect branch ends where KiCad leaves no fill (pour optimism,
one track_dangling). Remaining starved pads explained pad by pad in the
agent's report: spoke landing and piece fidelity (28 missed, 90 false
alarms), repeated pad numbers, a fine-pitch spoke-clearance case (k30
U14.12), slivers with no manual path. Reference tier log now
`build/quick30-x722.log`.

## Committed on 2026-10-14 (the legaliser agent, x701 / x711): the premise was wrong

The legaliser does not lose 30-90 % of the global placement's wirelength.
Measured with `PCB_PLACER_TRACE` (per seed the weighted HPWL after global
placement, annealing, legalisation and refinement, plus the designer's for
the same movable set) on the 30 tier boards and nine gap boards: the
global placement is 30-90 % longer than the final one and the annealer
removes that (annealed/global median 0.83); legalisation costs 4 % (median;
Castor 25 %, MIDAS-MK2 28 % before refinement, where the nearest-spot
legaliser moves 100-120 parts because larger parts placed first take
their spots or a fixed through-hole part's far side is met); refinement
wins most of it back (final/annealed median 1.00). Final/designer median
0.85; above the designer only BAGEL 1.77, zpn 1.39, MIDAS-MK2 1.34, MK2.1
1.28, hackrf 1.19, Castor 1.14. Video (0.96), OpenESC (0.61-0.73) and SNSP
(0.85-0.93) are below the designers: their failures are not wirelength.
The gap to the designers on crowded boards is the routing halos: no halos
gives -22 % HPWL, and designers' own placements are illegal at our halo
level for nearly every part (they put courtyards against each other). Four
legalise-or-place-better variants (reflow only conflicted parts, push apart
a step at a time, filler whitespace, four times the annealer's moves) do
nothing beyond seed noise (about ±10 % per board); removed, numbers in
docs/placer.md. Two options built, both off: `anneal_far_side_gaps` (the
annealer sees the legality test's hole and far-side gaps; Castor's invisible
conflicts 33 to 0, parts moved by legalisation 119 to 66; tier flat, video
and OpenESC worse, SNSP better) and `shared_halos` (neighbours keep the
larger halo, not the sum; six gap boards HPWL -6 %; tier 19 to 20 passes,
unconnected 29 to 24, vias -8.8 %, but zpn loses its pass and hackrf +57 %
vias). Placement with defaults is byte-identical to x652. Lesson: a
placement change flips crowded boards by ±75 connections at random, so
seed-repeated pairs are needed before any placement default changes. Next
lever: routing room from congestion (a halo only where RUDY or the first
route says a part's escapes need it) instead of the pin-count rule; less
halo everywhere has now lost routing twice (x507 halo scale, shared halos).
Scripts `experiments/placement-loss/`; logs `build/legal-*`,
`build/quick30-x700*.log`.

## Committed on 2026-10-14 (the fill agent, x693 / x710)

Starved thermals were not optimistic fill misses: in a census of 44 routed
boards (4274 thermal pad-layers, 171 DRC starved findings;
`experiments/pour-fidelity/{spokes,census,classify,causes}.py`, `spokes.py`
counts spokes in pcbnew exactly as the DRC does, `dump-pour-map` writes
`<net>.spokes`) the old model predicted 77 of the findings but raised 866
false alarms, so the router could not tell real starved pads from false
ones. `kicad_spokes` (on; `PCB_ROUTER_OLD_SPOKES` for A/B) copies KiCad
10.0.7's zone_filler and zone-connection DRC: spoke end at gap + half the
minimum width, spoke width min(spoke, pad minor), landing test off the
lattice with exact distances (`router/fill.rs` FillProbe), clearance from
the DRC ring, facing spokes of neighbouring pads meet, the island rule
("single" = exactly one item touches the piece), and the DRC's rule that a
track leaving the pad into kept fill counts as a spoke. New census: 116
predicted, 36 missed, 87 false alarms. `manual_spokes` (on;
`PCB_ROUTER_NO_MANUAL_SPOKES`): after stitching, a pad our model finds
short of two spokes gets straight tracks of its own class into a piece
KiCad keeps (the DRC counts them as spokes); also for pour nets routed as
tracks. Quick tier x651 to x693: 19 to 21 pass (Castor, spell_tome,
rp2040zero), starved 12 to 2, unconnected 29 to 22, vias 2859 to 2838, CPU
ratio geomean 0.87 under different load (no slowdown). USB_Cable_Tester
lost its pass: GND fill under U1's F.Mask polygon (two solder-mask
bridges), a routing change from the spoke model, not the repair; the pour
under a mask opening of another net is the gap the bodies agent noted.
Starved-only boards in route mode: Castor 20 to 5, PixelWave 30 to 1, Hub
15 to 4, rosco 9 to 3, k30 21 to 14, mackerel-30 21 to 19. The rest:
inner-layer through-hole pads whose spokes reach only island pieces (k30,
mackerel-30; needs stitching of single pieces in tracks mode), island-rule
edge cases, pads with repeated numbers and custom spoke templates (19 in
the census, unmodelled). Cell-level fill agrees to 1-3 % but our map is in
far more pieces than KiCad keeps (Castor F.Cu 128 against 17); untouched.
Logs `build/quick30-x693.log`, `build/fill-starved`, `build/fill-census`.

## Committed on 2026-10-13 evening (the bodies agent, x652)

Census of the 109 boards: 3800 footprints with pads and no courtyard
(2591 movable) on 92 boards; they blocked only with their pads, so hackrf's
LQFP-144 held 32 parts and homebrew_486's ISA slots interleaved. A Fab-layer
body rule fails (hackrf's library and the ISA16 draw none; where a Fab box
exists, 42 of ULK's switches hold a diode on purpose). The pad-pattern rule
`solid_pin_rows` (on): a footprint with 4 or more pads, three quarters of
them in rows or columns of 3 or more at most 2.6 mm apart, blocks with its
whole pad box plus copper and mask graphics; 625 footprints are solid, and
only 17 hold a designer's part (16 fixed-on-fixed). Keyboard switches,
hot-swap sockets and stabilisers stay hollow. `mask_only_pad_openings`
(router, on): a pad with a mask layer and no copper is a mask opening of
the net under it (hackrf's test points: its two solder-mask bridges on the
designer's placement become a pass). Copper-edge floor of 0.02 mm in the
placer (KiCad flags contact at zero clearance). Quick tier x651 against
x644: 19/30 both, unconnected 29 both, 21 boards byte-identical; OpenFC,
USB_Keypad and WLED change because parts no longer sit inside a QFN-80, a
USB-C receptacle or a LED pad box. hackrf: 0 open, one starved thermal
left (P5 VCC on the inner pour layer), but 1.6x the routed length now that
its parts are legal (crowded placement, problem 4); homebrew_486 passes
with its slots apart. Latent: the copper-pad aperture rule reads
`solder_mask_margin`, a key KiCad never writes (`pad_to_mask_clearance`);
noted in a comment. Logs `build/quick30-x644.log` (the reference),
`build/quick30-x651.log`, `build/bodies-*`; scripts `experiments/bodies/`.

## Planted boards (2026-10-13)

Boards routable by construction, three ways (`experiments/planted/`,
`docs/experiments/planted-boards.md`): maze (unrealistic), grow (realistic
at the easy level, cheap, but every one of 48 boards is easy for us: ours
shorter with fewer vias on 45), derived (blockers in a routed real board;
the only set that exposed failures, all starved thermals on complete
routes). Planted tile occupancy is a weak congestion label (Spearman
0.05-0.23 against our routing). No network; grow needs real difficulty
(pours on 2 layers, BGAs, buses) before its results are worth training on.

## Committed on 2026-10-13 (the placement-gap agent, x644)

Five boards that pass with the designer's placement and not with ours
(zpn_devboard, video, MokyaLora, hackrf-one, homebrew_486; logs
`build/gap-*`), each run pinned with ours against the designer's placement.
Wirelength is not the gap: our HPWL is within 0.8-1.4x of the designer's on
all five, and on four the loss is a modelling or task bug.

- **MokyaLora** passes now: a movable footprint's own rule areas (a U.FL's
  keepout) now move and turn with it, in the writer and in `run.py`'s
  stacking (`move_zones`). About 15 of the 109 tasks change through
  `run.py`, so baselines made before this differ on those boards (three
  tier boards remeasured: `build/quick30-x510-zonetasks.log`).
- **zpn_devboard** passes now (two runs, 0 open, 291-299 vias against the
  designer's 136): `keep_on_pieces`, on by default, keeps the parts a net
  joins to fixed parts on one piece on that piece; before, most of the
  board had moved onto an empty breakaway (129 open after the first route).
  Nothing changes on the tier.
- **hackrf-one** loses its 6-9 `items_not_allowed`: a movable part's keepout
  that forbids pads keeps the far side's pads out; a board rule area on
  inner layers only that forbids pads keeps pads out on both sides (a
  through-hole pad crosses them). It still fails on two solder-mask
  bridges from mask-only test points the router does not treat as
  obstacles (the designer's placement routed by us shows the same two).
- **homebrew_486** is not a gap (plus or minus one unconnected run to run);
  its v14 pass is flattered: a courtyard-less footprint blocks only with
  its pads, so the second ISA slot interleaved the first 2.54 mm apart,
  and on hackrf a header and an SOIC sat inside a turned LQFP's pad ring.
- **video** is the one real objective gap: eight SIMM sockets in one column
  at the designer's 8.89 mm pitch line the bus up; we scatter them.
  `bus_rows` (identical footprints sharing a bus pin for pin placed as a
  row; through-hole only; off) gets video from 315 to 259 open at the
  designer's pitch and 499 with the automatic one, made mackerel-08 worse
  (+6 starved, +21 vias, +1100 CPU s) and homebrew_486's SRAM rows lost two
  connections; left off. Video needs more than aligned sockets.
- Rows whose members block with shaped boxes now block with every member's
  boxes (before only the first part's; likely the J9/U16 overlap on 486).

Quick tier x642 against x510: 19/30 both, unconnected 30 to 29, other
copper errors 8 to 3, starved 12 to 18 and +21 vias all from the bus rows
on mackerel-08 (now off); 25 boards byte-identical. Next on this thread:
courtyard-less ICs and connectors should block the inside of their pad
box (not a blanket rule: ULK puts 42 diodes inside its switches' pad
boxes).

## Committed on 2026-10-10 (the memory and congestion agents)

- Rungs side by side with fixed work budgets, honest via-reduction budgets,
  a 4-ary key heap (9-13 % less search CPU), polish-again after a trial's
  polish, and the layout deciding by work everywhere: a board's layout now
  reproduces exactly run to run (CyberKeeb 3 of 3, Sisu byte-identical).
- The placement race pre-ranks 16 placer seeds by RUDY (routing demand over
  tile capacity) and probes the best three: quick tier KiCad unconnected
  98 to 64 (6 boards better, 2 worse), laptop power finds a legal seed.
  Default from the next commit. The neural predictor ranks race candidates
  about as well as RUDY (its label, a single 75 s probe, agrees with
  itself only at Spearman 0.6 between router seeds) and stays an
  experiment behind `congestion_model`; see docs/congestion-model.md.
- Placer correctness (the placement agent, in 3b890fa and 93b83f5): a
  fixed part's copper and mask graphics on its other side are obstacles
  there (Castor's logo: the short is gone); parts at odd angles are tested
  as turned rectangles and may turn square (ErgoSNM places: 58/58 clean);
  a footprint with a courtyard on each side gets a body per side (urchin
  places: 66/68); courtyards may touch where the pads inside keep the
  copper clearance, as a last resort (laptop power places on every seed,
  but RUDY then prefers a touching seed: 9 to 18 unconnected, a ranking
  fix is under way); rule areas inside footprints and pads-only rule
  areas are lowered (PixelWave 41 keepout findings to 0, 343/343 routed;
  d20 27 to 0); eviction 7x faster. framework_mobo's M1 has no legal spot
  at the designer's outline (its pads overhang the polygon): the task
  generator should test overhang against the polygon. SmartSpin2k: 16
  seeds cost 1218 s of placement on 4 cores (a placement budget for the
  seed count is under way). Measured: the legaliser's wirelength loss is
  mostly won back by refinement (final within 10 % of the global
  placement on most tier boards); the 1.4-1.9x against the designers is
  the routing halos, whether less halo still routes is the next experiment.
- Placer round 2 (a609a75): a touching-courtyard placement ranks last;
  the race's seed count follows the placer's 40 % share of the work (the
  placer's work clock was undercounting legalisation 20-fold; fixed);
  courtyards that fill under 75 % of their box block with their shape
  (ghoul places, urchin passes). Quick tier x617 against x500 (everything
  since x500 together): passes 16 to 20, KiCad unconnected 67 to 29,
  placement failures 4 to 1 (framework M1, the board's). The task
  generator now tests overhang against the outline polygon: 35 of 109
  tasks gain fixed parts (mounting holes in rounded corners, parts in
  notches); tasks v6 for v14. Open: `halo_scale` experiment (does less
  routing halo still route), the placer's model being stricter than
  KiCad at mounting-hole rings and pad boxes (45 designer poses illegal on
  ghoul), SmartSpin2k (a 585-connection panel whose first route alone
  needs 1000 s; not a target).
- Halo scale measured on the 30-board tier (x507): 0.75 and 0.5 cut the
  placer's wirelength only 1.3 and 4 % (halos are already shrunk where
  boards are crowded) and win nothing clearly (passes 19 / 19 / 17);
  1.0 stays. The 1.4-1.9x gap to the designers' wirelength is not the
  halos. The race's judge: the 75 s probe is the best pre-routing
  predictor of a layout's final result (Spearman 0.55 on 92 finals; two
  probes averaged no better; RUDY 0.30, wirelength 0.23, the network
  0.13), so the network is a closed negative as a ranker and
  `probe_seeds: 2` is not built.
- Speed is structural now: cheaper expansions are exhausted (five
  candidates under 5 %). The waste measurement (`PCB_ROUTER_WASTE=1`,
  bench-search) found chronically rerouted nets searching corridors up to
  10 tiles wide for narrow paths, and on Sisu and jetson half the
  expansions after iteration 20 gaining nothing. Committed (3c1f81d): the
  corridor margin grows every sixth reroute instead of every third: quick
  tier 10 % less CPU, unconnected 32 to 30, starved 20 to 15, vias -6 %,
  no pass lost; SNSP route mode 171 to 98 unconnected at +6 % vias. A
  stall stop (N=12 iterations without a conflict improvement; 3-11 % less
  CPU, boards nearly unchanged) is queued for a tier pair. Capping the
  margin fails (SNSP needs the wide corridors).
- Committed aa4e9e8: the negotiation ends twelve iterations after the
  price cap without a conflict improvement (tier -8 % CPU, -18 % board
  time, no pass lost). The 30-board tier's clean baseline on x508: 19 of
  30 pass, 23 unconnected, 12 starved thermals, 31.5 min of wall at 8
  jobs (`build/quick30-x508.log`).
- Speed, round three (2026-10-13; experiments in the worktree
  `pcb-maker-speed2`, diff `build/speed2/speed2-experiments.diff`, logs
  `build/speed2`, tiers `build/quick30-x333-L12.2.2.log`,
  `build/quick30-x334-viastop.log` against `build/quick30-x510.log`), all
  negative:
  - Chronic nets searched near their last route (`PCB_ROUTER_LOCAL=12,2,2`:
    a net rerouted 12 times or more searches its corridor cut to two tiles
    around its previous route, tree and targets first, every other
    reroute). Measured first: nets with 18+ reroutes find their path within
    one tile of the previous route in 97-98 % of searches yet expand 72-78 %
    of their nodes outside it. bench-search reaches lower conflict counts at
    equal work (Sisu 17 at 350M expansions against 28; jetson 9 at 550M
    against 25) and stops at 58 % / 82 % of the expansions. But the tier is
    neutral (19/30 both, unconnected 30 to 24, CPU -5.5 % against x510 and
    +9 % against x508: noise), Sisu in route mode -10 % CPU (pinned pair,
    swapped: 0.894, 0.915) ends 34 open against 27, jetson identical (2
    open) at -3 %. Route mode rarely reaches the chronic regime; where it
    does, the full searches are what finds the detours.
  - Not re-searching a conflict-free net: already so (a net's branches are
    tested for conflicts at its turn, after the nets before it moved; a
    speculative search that read a changed block is redone).
  - A cheaper first iteration: iteration 0 is 1-2 % of the negotiation's
    expansions on Sisu and jetson, the first three 3-7 %; not built.
  - On the tier the negotiation is not the cost: the via reduction is (its
    lines sum to 66 % of the v14 tier layouts' routing time, the clean-up's
    to 27 %; 80 % of its rounds reverted, 72 %
    because the round found no fewer vias, and after such a round the next
    is kept 9-11 % of the time). Stopping at such a round
    (`PCB_ROUTER_VIA_STOP`) cut the tier's via-reduction time 13 % and its
    CPU 1.7 % (noise), +18 vias: a layout spends its work budget anyway
    (more move trials), so a cheaper phase shows up as more trials, not as
    less CPU. Speed on the tier means quality per work, and a phase that
    ends by its budget gains nothing from being cheaper.
- The quick tier is 30 boards now (`benchmarks/agent-tasks/quick.json`,
  README): real layouts in about 1100 s and 3.7 GB; about 35 min at 8
  jobs.

## Committed on 2026-10-09 evening (b67da0a), measured by the pour agent

- Spoke clearance (a spoke counts only when its strip clears other
  nets' copper), pour-aware clean-up and via reduction (a change that
  strands a pour pad is rejected), stale plane stubs rerouted,
  deterministic stitching, the pour-refresh panic fixed at its root, circle
  rule areas lowered. k30 forced to connect: 41 open (before) to 0 open
  with 14 starved thermals, beating the tracks rung's 16. Castor's connect
  rung in route mode: 126/126 with 18 starved; Hub 144/159 with 20.
- Measured and left off: stitching vias during negotiation (Sisu 63 to 68
  open: the stitches crowd the signals), main-piece hysteresis (the main
  piece does not flip, it collapses when a stuck pour net is ripped up
  whole), not ripping pour nets up whole (worse), hard thermal guards.
- Cost: the pour-aware check's own-stamp lookup is a bitset now (k30
  forced-connect checks 179 s to 78 s, Hub 207 s to 130 s, identical
  results); skipping far pour nets never fires on boards whose pours cover
  the board.
- Same-copper comparison, route mode on v12's placed boards, x165 against
  HEAD: k30 identical (tracks wins both), Castor 184 to 176 vias at 18
  starved, Hub 18 to 20 unconnected, Sisu 20 to 13 unconnected. The
  probe ladder picks the tracks rung by its 75 s probe even where the
  connected rung's final result is better (k30 forced connect: 0 open,
  14 starved against tracks' 16): the ladder's judge has the race's
  noisy-probe problem.
- Real-layout rows on x204 (fixed run.py): k30 173/173 with 3 starved
  (365 vias to 287), Qfwfq 64/64 clean, Telemetry 107/108, Castor 125/126
  with one unconnected, one short and one bridge (the short is under
  investigation), Hub 132/159 with 40 (placed for the first time), Sisu
  183/192 with 12 (unchanged: it was always placed).

## Sweeps before v16 (2026-10-09 to 2026-10-14)

**Layout sweep v11** (`benchmarks/agent-tasks/run.py` on
`build/github-tasks-v5/tasks.json`, binary x136 with killed or panicked
boards rerun on x137-x146; 109 boards harvested from GitHub, each reduced to
stacked parts plus a `constraints.json` derived from the designer's board,
1800 s each): **38 of 109 pass** (KiCad: 0 unconnected, no copper error,
constraints met). Against the x112 baseline on the 29 boards both have:
better on 14, same on 13, worse on 2 (the two OpenESC/OpenRX panels), 1598
to 1231 unconnected, passes 5 to 5. Reproduce the table with

    benchmarks/agent-tasks/compare.py build/github-layout-v8-x112.log build/github-layout-v11.log \
      --splice build/github-layout-v11-x138.log build/github-layout-v11-x139.log \
               build/github-layout-v11-x140.log build/github-layout-v11-x141.log \
               build/github-layout-v11-x146-jiran.log --markdown

The full table is in `docs/benchmarks.md`.

**A caveat on every layout sweep up to v12 (found 2026-10-09 by the
congestion-model agent):** `run.py`'s unplace only read KiCad 10 net
syntax, so on the 89 pre-KiCad-10 tasks every footprint counted as netless
and stayed where the designer put it; only 22 of the 109 tasks were really
placed (the v12 logs' "placed in" times show which). Fixed in run.py;
v13 is the first sweep that places all of them, and its numbers will be
lower. Route-mode conclusions (pour model, connect rungs, Freerouting
comparison) are unaffected.

**Layout sweep v15** (x723, tasks v6, 6 jobs, 17910 s, 2026-10-14): **45 of
109 pass**, 3829 open; against v14 better 20 / same 63 / worse 16; the
worse rows are the crowded boards (see docs/benchmarks.md).
**Layout sweep v14** (x510, tasks v6, 10 jobs, 12668 s): **34 of 109
pass**, 3065 open in all; against v13 better 19 / same 51 / worse 24. The
baseline for what comes next (`build/github-layout-v14.log`; tasks
`build/github-tasks-v6`). Timeouts: link, katia, MegaDrive, the 0xCB
panel; no legal placement: the molecule panels and the OpenESC 4in1 panel.

**Layout sweep v13** (x503, 12 jobs, load 25-40, 14525 s): **31 of 109
pass** with every board placed from stacked parts; 3586 open in all. The
honest layout baseline from now on (v12's 41 was mostly routing). 7
placement-illegal rows (5 molecule panels, framework M1, ghoul), 6
timeouts (katia, PixelWave, SmartSpin2k, MegaDrive, two 0xCB panels).
v14 should run on the 16-seed race (x504+) once the placement budget for
the seed count is in. Table in `docs/benchmarks.md`.

**The first real-layout baseline, quick tier** (`build/quick-v13-base.log`,
x165 with the fixed run.py, 37 boards, 12 jobs, 3617 s): **18 of 37 pass**
(25 when the designer's placement was kept). New failure classes now
visible: four boards fail before routing because the placer finds no legal
spot for a part (framework_mobo_lefthalf M1; laptop power C8/R49/C22;
ErgoSNM KEY31/KEY33; urchin SW31/SW23); the SmartSpin2k panel at 270/585;
WLED_Matrix 26 and PixelWave 30 unconnected with shorts, bridges and 50
keepout violations; MIDAS-MK1 15, BAGEL 7. Starved thermals only:
mackerel-08 8, Castor 9, 1337-v4.0 3, bumwings R64 1. The tier's stored
times and peaks (quick.json) are from designer-placement runs and must be
redone (one board peaked at 5.7 GB against 3.7 assumed).

**Layout sweep v12** (x165 = 1c7a557, 14 jobs, 10375 s in all): **41 of
109 pass**; against v11 better 12 / same 77 / worse 12, unconnected 4722 to
4771 (SNSP +90 and Quanta75 +25 are the known costs of the mask-body and
plate rules; A13 +42: x141, x146 and x155 all give 162/223 with 103 under the
sweep's load and the same race as v11, x165 picks another seed (279
against 288 unfinished after the probe) and ends worse again; the board
is sensitive to the race's pick and to load, not to one code step). Seven
error rows: Telemetry's degenerate fill-keepout rule area (new lowering
bug), the `refresh_pour` -> `find` stale-index panic on SUMEC, coco2,
bumwings xiao_sd and Qfwfq, PolyKybd right at the 1800 s limit under
load, the 0xCB panel's kicad-cli timeout. Table in `docs/benchmarks.md`.
Compare: `compare.py build/github-layout-v11-spliced.log build/github-layout-v12.log`.

**Freerouting against our cold route** (`benchmarks/github/freerouting.py`,
headless, 1500 s each, both on the designer's placement with all copper
removed; `freerouting_compare.py`): all 109 boards. Freerouting finishes 70,
times out on 34 and fails on 5; where it finishes we have fewer unconnected
on 49 boards, it on 6; unconnected in all 913 against 206; clean boards 8
against 50. Our rows mix x136 (40 boards) and x155 (69); rerun the 40 on a
current binary when the comparison is quoted as final. Logs:
`build/github-freerouting{,-2,-3}.log`, `build/github-ours-cold{,-2,-3}.log`
(`-all.log` is their concatenation).

**Route sweep v16** (`benchmarks/corpus/run.py --skip-layout`, x136, 2700 s)
has 2 rows (jetson 338/340 with 2 unconnected, link 246/251 with 8) and was
stopped; restart with `build/chain-v11.sh`'s last line.

Rerun lists kept: `build/github-layout-v11*.log`, `build/github-freerouting*.log`,
`build/github-ours-cold.log`; binaries `build/bin/pcb-maker-x112` ... `x155`.

## What was learned this week, condensed

- **A knockout text is its plate** (the glyphs' box grown by the stroke),
  not its glyphs; a copper text with a twin on its mask layer is
  exposed-copper artwork and stays put, for the mover and for the layout's
  in-place route alike. Quanta75: shorts 0, the bridges left are the
  designer's J8 inside the logo. Routing less (146/172 against 169) is the
  price of the plate as an obstacle.
- **A footprint's own mask graphics are part of its body** for the placer
  (SNSP-CPU-01's J2: a 22 x 32 mm opening, 63 pads of other parts under it,
  each a bridge). The cost is real on that crowded board (333/363 with 116
  unconnected to 309/363 with 290; the race ends at 418 unfinished nets
  instead of 314), measured by bisecting (x143 reproduces x136 exactly).
  Kept: pads under a mask opening are bridges, and the back side stays free.
- **The ladder's memory** (x146): a finer pitch grows as the square of the
  pitch ratio and the seeds run side by side; jiran-ble-lite (3299 x 1048
  nodes at 0.1 mm, 13.8M at 0.05 mm, 7 GB a router) took 31 GB. The ladder
  now projects memory from the process's peak after the first attempt
  (`memory_mb`, `router_mb`): a finer pitch must fit 80 % of what is
  available, seeds half of it. jiran: 104/105, KiCad 20 unconnected (its GND
  has no pour; half its 40 GND pads are not reached on 2 layers; open).
- **KiCad's island rule** (x147-x155): a fill piece joined to the net by
  thermal spokes alone is an island; KiCad removes it and does not count the
  spoke ("spokes connected to isolated island"). Measured in pcbnew on
  k30-SBC: a via inside the piece cures it. Changes, all default:
  - `analyze_pours_full`: a piece gets copper of the net first (a track or
    via node, a solid pad); a thermal link joins a pad to a piece with such
    an item, never the piece to the pad;
  - every item-less piece a spoke reaches, for a pad short of two good
    spokes on that layer, gets a stitching via when one fits;
  - the net's own tracks are fill wherever fill may lie, inside a pad's
    thermal ring too (a plane stub ending in the ring read as "stranded, 1
    branch, 0 pieces" and was rerouted and stitched for nothing);
  - `canonicalize_copper_net_names` falls back to the bare name: copper is
    written as `(net "/GND")`, and since the net-name registry (x140) kept
    the slash when a bare GND exists, KiCad made a net "/GND" of our items
    (Hub: 636 items, 36 clearance errors);
  - `junction_hosts` skips a one-node branch between two junctions (the Hub
    panicked).
  Results: Castor 126/126 clean, islands 4 to 2 (x155; one run gave 5
  starved thermals in all, another 18: the winning rung decides); Sisu
  186/192 with 13 (x136: 185 with 26); Hub 146/159 with 17 (as x136); k30
  173/173 with 10 (as x136, its 8 one-spoke pads untouched).
- **Hard thermal guards do not work and cost routes.** Closing a pad's last
  two spoke corridors to other nets (`hard_thermal_guards`, off) left k30's
  one-spoke pads as they were and cost Castor six nets and Sisu fifty
  connections.
- **The finding under all of it: our fill is 10-40x more fragmented than
  KiCad's.** k30's GND fill is 63 + 33 + 29 outlines in KiCad (F, B, In2)
  and 1100-1500 pieces in our pour map; +5V on In1 24 against 338. The
  brush-centre model on a 0.1 mm lattice loses every neck near the minimum
  width (0.254 mm is 2.5 cells), and clearance rings around other nets' vias
  on a plane layer meet where KiCad's do not. The two-way pad links of the
  old model hid this; the one-way links show it as stranded pads, reroutes
  and stitching vias. k30's +5V: "38 stranded terminals" while KiCad finds
  every +5V pad connected.
