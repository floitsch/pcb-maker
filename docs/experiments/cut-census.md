# Cut census and the skeleton's objective gap (2026-11-24)

Two stage-0 censuses that gate two proposals of the research note
`build/scratch/fundamentals/fundamental-approaches.md` (2026-11-23):

- **Census 1, Proposal B** (an exact cut-capacity model): is the routing
  room on the dense boards a discrete-cut problem the placer gets wrong?
- **Census 2, Proposal C's gate** (an arrangement search on the skeleton):
  does the skeleton's optimiser leave objective on the table?

No routing was run for either. Scripts: `experiments/cut-census/`
(`cuts.py` and `report.py` for census 1, `skeleton_gap.py` for census 2,
`test_cuts.py`). Outputs under `build/scratch/fund-cuts/` (not in git).

## Verdicts

- **Census 1 (Proposal B), gaps: kill.** The designer's pad-to-pad gaps
  are not clustered just above the track quanta (pooled phase flat, 10 to
  16 % per eighth of a pitch; the share within 0.1 mm below a quantum at
  its base rate on nine of eleven boards), and ours are not more wasted
  either, except a few points on the two OpenESC boards.
- **Census 1, escape flow: a weak separation, not decisive.** Without
  vias our deficits exceed the designer's on the 4in1 (5 pins against 0),
  DSP-ADAU1452 (5 against 2), Glasgow (47 against 28) and the keyboard;
  with vias every difference vanishes but SmartSpin2k's panel copies (54
  against 0). A few pins against gaps of 67 and 68 open connections. As a
  stage-0 seed metric the planar deficit ranks the first route at a mean
  Spearman of +0.30 (seven boards where it varies), as the placer's
  objective (+0.28) and crossings (+0.29) do; the gap measures carry
  nothing (-0.09 to +0.14).
- What separates the placements instead is packing: ours closes about
  twice the designer's share of cuts to below one track on the over-full
  boards (4in1-mini 25 against 10 %, DSP 24 against 11 %, Glasgow 22
  against 13 %). It does not rank our seeds (+0.07).
- **Census 2 (Proposal C's gate): Kill 1, and more.** On no board does the
  designer's skeleton, legalised by our legaliser, beat our best seed by
  more than the seeds' spread. On ten boards with a placement gap and a
  skeleton it is worse than every one of our seeds on four (Glasgow, the
  4in1-mini, Batch_1, capybully), worse than all but one or two on three
  (link, Sisu, WLED), in the middle on two (the 4in1, poly_corne), and
  better than all only
  on MIDAS-MK1.1 (7 %, inside the spread of 17 seeds). Unlegalised, the
  designer's skeleton scores worse still on most. The objective does not
  prefer the designer's arrangement, so a stronger search on it would not
  move towards it: the work belongs in the objective, not in the search.

## Census 1: room between parts in track quanta

### Method

`cuts.py` reads a placed board with KiCad 10's pcbnew (pad copper per
outer layer, holes, the outline, zone nets, the board's own default net
class) and measures two things.

**Gaps.** A *cut* is a pair of pads of two different footprints on one
outer copper layer whose copper lies within `Q_3 + w + s` (at most 2 mm)
and whose shortest connecting segment crosses no third pad. Its gap `g`
holds `k = floor((g - s) / (w + s))` tracks of the board's default class
(width `w`, clearance `s`); the quanta are `Q_k = k w + (k + 1) s` (at
0.2 / 0.2: 0.6, 1.0, 1.4 mm; a 0.55 mm gap holds none, 0.60 mm one; the
note's `floor((gap + s) / (w + s))` counts one track too many for a gap
between two pieces of copper). The
*phase* of a gap is `(g - s) / (w + s) - k`: 0 just at a quantum, near 1
just below the next. Measures over the gaps from `s` to `Q_4`:

- `wasted 0.1 mm`: the brief's measure, the share of gaps within 0.1 mm
  below a quantum `Q_k`, k >= 1, beside the share a uniform spread would
  put there (the base, which depends on the rules: 0.1 mm is more than
  half a pitch at the OpenESC boards' 0.09 / 0.09);
- `top quarter`: phase at least 0.75 (room just short of one more track,
  comparable across rules); `bottom quarter`: phase below 0.25 (a gap
  just at a quantum, the designer's predicted signature);
- `no track`: the share of cuts below `Q_1` (closed up: no track passes).

The gap tables count the cuts with at least one *moved* part (a footprint
whose pose differs between the two placements); on these boards that is
nearly every cut.

**Escape flow.** Per footprint of ten copper pads or more (not a frame
part whose pads span a quarter of the board): the pins that must escape
are its pads on signal nets (a net with a pad on another footprint, not
carried by a zone). The free copper space in its pads' box grown by
`ring` + 1 mm is triangulated (Delaunay on every copper outline, the
holes and the board edge, sampled every 0.05 mm; triangles inside copper
or off the board removed). Adjacent free triangles are joined by an arc of
the tracks their shared edge holds between the two pieces of copper it
spans (the formula above, the copper-to-edge clearance at the edge); the
sink is every free triangle outside the box grown by `ring`. `planar` is
pins minus the max-flow on the part's own layers (both outer layers for a
through-hole part); `vias` also lets each free triangle inside the ring
whose empty circle holds a via and its clearance, clear on the other outer
layer, absorb one unit. Triangle edges overstate what a triangle passes
(the diagonal capacity of Yan and Wong), so the flow is an upper bound and
the deficit a lower bound. `test_cuts.py` checks the capacities on made-up
walls (0.55 mm passes nothing, a 0.62 mm opening one track, 1.02 mm two,
a via fits in 1.2 mm and not in 0.55 mm).

**Placements.** The eleven dense boards of `dense-noise.json`. The
designer's: the nightly of 2026-11-18, `designer/<board>/placed/`. Ours:
the same night's sweep, `layout/result/` with the layout's kept moves
undone (the race winner's poses at the first route; the placer's
`layout/placed/` gives the same deficits and every gap share within a
point). For the metric check, the 99 placements of
`dense-noise.json` (nine seeds per board, binaries x1828 and x1851, each
matched to its stored layout by its first-route open terminals).

### Gaps

| board | rules w / s | cuts D / O | wasted 0.1 mm D / O (base) | top quarter D / O | bottom quarter D / O | mean phase D / O | no track D / O |
| --- | --- | --- | --- | --- | --- | --- | --- |
| 0xCB panel | 0.20 / 0.13 | 866 / 478 | 18 / 7 (30) | 17 / 4 | 21 / 33 | 0.43 / 0.35 | 44 / 70 |
| protoesp | 0.25 / 0.20 | 598 / 452 | 27 / 25 (22) | 29 / 29 | 28 / 19 | 0.50 / 0.54 | 18 / 18 |
| Glasgow revC3 | 0.15 / 0.15 | 803 / 945 | 39 / 36 (33) | 25 / 28 | 24 / 22 | 0.52 / 0.51 | 13 / 22 |
| video | 0.20 / 0.20 | 141 / 219 | 27 / 32 (25) | 27 / 32 | 18 / 28 | 0.57 / 0.51 | 2 / 4 |
| 4in1-mini | 0.09 / 0.09 | 883 / 908 | 52 / 65 (56) | 28 / 30 | 27 / 21 | 0.49 / 0.55 | 10 / 25 |
| 4in1 | 0.09 / 0.09 | 1063 / 1077 | 55 / 71 (56) | 23 / 28 | 21 / 13 | 0.50 / 0.57 | 24 / 35 |
| SmartSpin2k panel | 0.25 / 0.20 | 2160 / 4815 | 27 / 25 (22) | 29 / 29 | 22 / 24 | 0.54 / 0.53 | 14 / 24 |
| DSP-ADAU1452 | 0.50 / 0.20 | 916 / 931 | 13 / 10 (14) | 19 / 19 | 26 / 28 | 0.45 / 0.47 | 11 / 24 |
| MegaDrive | 0.25 / 0.20 | 219 / 347 | 24 / 24 (22) | 24 / 28 | 28 / 25 | 0.51 / 0.51 | 0 / 0 |
| SNSP-CPU-01 | 0.25 / 0.20 | 708 / 417 | 24 / 25 (22) | 25 / 27 | 32 / 29 | 0.47 / 0.48 | 2 / 0 |
| poly_kybd_split72_left | 0.25 / 0.15 | 1669 / 1723 | 35 / 28 (25) | 35 / 28 | 14 / 24 | 0.57 / 0.52 | 13 / 15 |

Shares in per cent; D the designer's, O ours.

The phases pooled over the eleven boards (per cent of the cuts per eighth
of a track pitch above the quantum below):

| phase | 0-.125 | .125-.25 | .25-.375 | .375-.5 | .5-.625 | .625-.75 | .75-.875 | .875-1 |
| --- | ---: | ---: | ---: | ---: | ---: | ---: | ---: | ---: |
| designer (10014 cuts) | 10.3 | 12.3 | 16.2 | 12.0 | 12.2 | 10.3 | 12.3 | 14.3 |
| ours (12296 cuts) | 10.5 | 12.8 | 12.4 | 11.8 | 13.2 | 12.1 | 13.7 | 13.6 |

- **The designer's gaps do not cluster just above the quanta.** The
  designer's phase is flat (10 to 16 % per eighth, mean 0.50), and the
  share within 0.1 mm below a quantum is at its base rate on nine of
  eleven boards. Designers do not place to the track quantum.
- **Ours do not differ in phase either**, with one pair of exceptions in
  the predicted direction: on the two OpenESC boards our gaps sit more
  often just below a quantum (4in1: 71 against 55 % within 0.1 mm, top
  quarter 28 against 23 %, bottom quarter 13 against 21 %; the mini
  65 against 52 %). Those boards' 0.1 mm grid against a 0.18 mm pitch can
  make such a pattern by itself. On the 0xCB panel and the keyboard ours
  waste less than the designer.
- **What differs is the count of closed cuts.** Ours closes about twice
  the designer's share of cuts below one track on the over-full boards
  (4in1-mini 25 against 10 %, DSP-ADAU1452 24 against 11 %, Glasgow 22
  against 13 %, the 4in1 35 against 24 %, SmartSpin2k 24 against 14 %, the
  0xCB panel 70 against 44 %), and about the same on the boards with room
  (protoesp, MegaDrive, SNSP-CPU-01, the keyboard). That
  is packing, not phase: our parts sit closer, and the room between them
  is gone rather than wasted.

### Escape deficits

Pins short of a simultaneous escape, summed over the parts of ten pads or
more, at three ring radii:

| board | pins | planar r1.5 D / O | vias r1.5 D / O | planar r2.5 D / O | vias r2.5 D / O | planar r4 D / O | vias r4 D / O |
| --- | ---: | --- | --- | --- | --- | --- | --- |
| 0xCB panel | 160 | 8 / 8 | 8 / 8 | 8 / 8 | 8 / 8 | 8 / 8 | 8 / 8 |
| protoesp | 119 | 8 / 8 | 7 / 7 | 8 / 8 | 7 / 7 | 9 / 9 | 7 / 7 |
| Glasgow revC3 | 287 | 28 / 30 | 28 / 30 | 28 / 47 | 28 / 30 | 28 / 61 | 28 / 30 |
| video | 1172 | 0 / 0 | 0 / 0 | 0 / 0 | 0 / 0 | 0 / 7 | 0 / 0 |
| 4in1-mini | 149 | 0 / 0 | 0 / 0 | 0 / 0 | 0 / 0 | 0 / 0 | 0 / 0 |
| 4in1 | 149 | 0 / 3 | 0 / 0 | 0 / 5 | 0 / 0 | 0 / 7 | 0 / 0 |
| SmartSpin2k panel | 648 | 0 / 99 | 0 / 45 | 0 / 126 | 0 / 54 | 0 / 144 | 0 / 54 |
| DSP-ADAU1452 | 126 | 1 / 0 | 0 / 0 | 2 / 5 | 0 / 0 | 7 / 15 | 0 / 0 |
| MegaDrive | 695 | 0 / 0 | 0 / 0 | 0 / 0 | 0 / 0 | 0 / 0 | 0 / 0 |
| SNSP-CPU-01 | 710 | 6 / 6 | 0 / 0 | 6 / 6 | 0 / 0 | 6 / 6 | 0 / 0 |
| poly_kybd_split72_left | 1011 | 0 / 2 | 0 / 0 | 1 / 3 | 0 / 0 | 1 / 5 | 0 / 0 |

- The deficits the two placements share are the footprints' own (the 0xCB
  panel's and protoesp's connectors with pads closer than a track, Glasgow's
  FPGA ball grid U30, whose inner balls need vias between balls too tight
  for one).
- Without vias ours is short where the designer is not on the 4in1 (5
  pins of 149 at 2.5 mm, U10), DSP-ADAU1452 (5 against 2, U201), Glasgow
  (47 against 28: U30's balls reach the ring 17 pins worse, and U1), the
  keyboard (3 against 1), more so as the ring grows. The ratios are
  "several times" on the 4in1 and DSP, but the counts are a few pins out
  of 126 to 287, against first-route gaps of 67 (4in1) and 68 (Glasgow)
  open connections.
- With a via allowed wherever one fits, every difference but SmartSpin2k's
  vanishes (Glasgow 30 against 28). The 4in1-mini, the third over-full
  board, has no deficit on either placement.
- **SmartSpin2k is the exception:** each of the nine panel copies of our
  placement walls in U2, U3 and U5 (126 pins planar, 54 with vias; 0 on the
  designer's). Its placement gap is the panel's sub-board assignment
  (docs/benchmarks.md); this is the same defect seen from the pins.

### As stage-0 metrics

Spearman correlation with the first route's open terminals across the
nine seeds of each board (`dense-noise.json`), its mean over the boards
where the measure varies (count in brackets), beside the placer's
objective and ratsnest crossings from the same file:

| measure | mean rho | boards |
| --- | ---: | ---: |
| deficit, planar (r 2.5) | +0.30 | 7 |
| deficit, with vias | +0.12 | 4 |
| parts short, planar | +0.14 | 6 |
| no-track share | +0.07 | 11 |
| wasted 0.1 mm | -0.08 | 11 |
| top quarter | -0.09 | 11 |
| room above the quanta, mm (sum of phases) | +0.14 | 11 |
| placer objective | +0.28 | 11 |
| ratsnest crossings | +0.29 | 11 |

The planar deficit ranks seeds as well as the objective and the crossings
do, on the boards where it varies at all, carried by SmartSpin2k (+0.89), the
4in1 (+0.58), the keyboard (+0.58) and Glasgow (+0.45). The gap measures
carry nothing (-0.09 to +0.14); the closed share, which tells ours from
the designer's, does not tell our seeds apart (-0.65 and -0.42 on the two
OpenESC boards).

### Verdict on Proposal B's census

- **Gaps: kill.** No separation: the designer's gaps are not clustered at
  the quanta, and ours are not systematically wasted; the one difference
  (the OpenESC boards) is a few points and grid-shaped. The step "gaps in
  track quanta" (snapping after the legaliser) has nothing to snap
  towards.
- **Escape flow: weak separation, not decisive.** Without vias our
  deficits exceed the designer's on four of the eleven boards and are
  several times larger on the 4in1 and DSP, as predicted, but they are a
  few pins against gaps of tens of connections, and with vias they vanish
  except on the SmartSpin2k panel. As a seed metric the planar deficit is
  as good as the objective (0.30 against 0.28), no better.
- What the census found instead: our placements close about twice the
  designer's share of the cuts between parts to less than one track.
  That is a packing difference (the over-full boards' legaliser seating
  bodies tight) that neither quantisation nor a per-part escape flow sees;
  and as a seed metric it does not rank (+0.07).

So B's steps 1 (snapping, the escape flow as an objective term) are not
supported by this census. Its step 0 at the stall sites (the binding cut
on the trouble-maker's path, from the router) is a different measurement
and is not answered here.

## Census 2: the skeleton's objective, ours against the designer's

### Method

The skeleton is what Proposal C would search: the parts but the free
passives, placed alone with bodies grown by their passives' room and nets
joined through series passives (`pcb_placer::skeleton`). A lab hook in the
placer's trace (`board_placer/trace.rs`, only with `PCB_PLACER_TRACE`)
rebuilds that problem (`skeleton_problem`, halos off) and writes
`skeleton_objective`: the anneal's objective by term at every `skeleton_*`
stage's poses and at the designer's (`PCB_PLACER_DESIGNER`). The total is
the anneal's prices without the overlap term: weighted wire + relations +
1000 x hard + 3 mm per ratsnest crossing (the overlap, against bodies
grown at the full spacing, is reported apart; the legal placements of
both carry similar amounts).

`skeleton_gap.py` runs per board the layout's place check
(`place-kicad-board` with the 2026-11-18 sweep's layout.json and
router.json, two threads, pinned work rates, the worktree's binary at main
96e891b2 plus the hook):

- **ours**: `tuning.seed` 1 and 101, every kept seed (3 to 16 per run, 5
  to 30 per board), compared at `skeleton_final` (after the skeleton's
  refinement);
- **designer**: `PCB_PLACER_START=<designer's board>`,
  `PCB_PLACER_START_STAGE=legalize`: no global placement and no anneal,
  each level's legaliser seats the designer's poses and the refinement
  follows, in the skeleton's phase as in the passives'. Its
  `skeleton_final` is the designer's arrangement legalised and refined by
  our own stages (deterministic: every seed gives the same).

Seeds that fell back to a plain placement (the skeleton or the passives
around it left parts without a place) have no skeleton and are left out.
Boards: the twelve gap boards of `gap.txt` with a skeleton race and a
placement gap of at least 3 (SmartSpin2k places as panel copies, without
skeletons).

### Results

Skeleton objective at `skeleton_final` (lower is better):

| board | gap | our seeds | ours best / median / worst | spread | designer legalised | (designer - best) / spread | designer's rank among ours | raw designer |
| --- | ---: | ---: | --- | ---: | ---: | ---: | --- | ---: |
| Batch_1 | 140 | 6 | 22443 / 23639 / 25076 | 2633 | 39270 | +6.39 | worse than all | 28749 |
| Glasgow revC3 | 68 | 6 | 2768 / 2844 / 2972 | 205 | 3419 | +3.18 | worse than all | 3860 |
| 4in1 | 67 | 6 | 686 / 961 / 1397 | 710 | 1055 | +0.52 | 5th of 7 | 886 |
| 4in1-mini | 51 | 5 | 654 / 680 / 687 | 33 | 741 | +2.64 | worse than all | 970 |
| eurorack-pmod | 17 | 6 | 1218 / 1360 / 1839 | 621 | (fell back) | | | 2957 |
| laptop power | 9 | 0 | (all seeds fell back) | | | | | |
| link | 4 | 10 | 6820 / 7236 / 9232 | 2412 | 8422 | +0.66 | 10th of 11 | 12310 |
| WLED_Matrix | 3 | 26 | 1063 / 1298 / 1655 | 592 | 1514 | +0.76 | 25th of 27 | 1169 |
| MIDAS-MK1.1 | 3 | 17 | 3092 / 3939 / 5552 | 2459 | 2865 | -0.09 | better than all | 3022 |
| Sisu_SSE-9 | 3 | 15 | 4061 / 5268 / 7059 | 2998 | 5641 | +0.53 | 14th of 16 | 9853 |
| capybully | 3 | 30 | 863 / 916 / 993 | 131 | 1221 | +2.74 | worse than all | 1867 |
| poly_corne_split42_left | 3 | 24 | 4543 / 4999 / 5459 | 916 | 4974 | +0.47 | 12th of 25 | 5221 |

`gap`: the board's placement gap in connections (2026-11-18). `raw
designer`: the designer's poses on the skeleton's problem without
legalisation (grown bodies overlap there; the wire and crossing terms are
what count).

Where the designer loses, it loses on the terms the objective is made of,
wire and crossings, not on the overlap: Glasgow 2246 mm and 391 crossings
against our best seed's 1823 mm and 315; the 4in1 698 mm and 119 crossings
against 488 and 66; capybully 999 and 74 against 740 and 41. MIDAS is the
one board where the designer's skeleton is shorter (2061 mm, 268
crossings, against 2156 and 312).

The whole placement (the passives placed around each skeleton, the
trace's `final`, all terms) tells the same: the designer's legalised
placement is inside our seeds' range on Glasgow, the 4in1, link, Sisu and
poly_corne, above it on the 4in1-mini, Batch_1, capybully and WLED, below
it only on MIDAS (3082 against 3233).

### Verdict on Proposal C's gate

- **Not open.** The test was "the designer's below our best seed by more
  than the seeds' spread"; no board meets it, MIDAS comes closest at
  -0.09 spreads.
- **Kill 1 holds, more strongly than stated.** On the boards that carry
  the placement gap (Glasgow, the two OpenESC boards, Batch_1), the
  designer's skeleton is inside our spread (the 4in1) or worse than every
  seed (the rest), even though the designer's placement routes with 51 to
  140 fewer open connections. The objective does not merely fail to tell
  them apart: on most gap boards it ranks the designer's arrangement
  below ours.
- **Kill 2's check is moot** (the search would have no reason to reach the
  designer's skeleton), and the earlier evidence answers its question:
  pinning the designer's active parts recovers a median 0.92 of the
  designer's first-route advantage (docs/placer.md, "Which placement
  decisions matter"). The designer's skeleton routes better; our
  objective scores it worse. A replica-exchange or sequence-pair search
  on today's objective would find lower objective, further from it.
- This revises the placer.md line "the objective is not what is wrong"
  (the designer at 0.59 to 1.02 of our objective, measured on the whole
  placement of four boards with an earlier binary, `stages.py`): on the
  skeleton, with today's objective, it is 0.93 to 1.75 of our best seed.

### Caveats

- The designer's poses are legalised against the skeleton's grown bodies
  (`room` 1); the legaliser moves the parts it must, and the refinement
  then lowers the objective from there. The raw designer's poses, not
  moved at all, are still worse than our best seed on ten of eleven
  boards (all but MIDAS, 0.98 of it; the 4in1's 886 lies inside our
  spread).
- Two runs per board (6 to 30 seeds); the second batch of bases (201 to
  501) was stopped, since more seeds can only lower our best and widen the
  spread, which cannot turn the verdict towards the designer.
- eurorack-pmod's and laptop power's skeletons fell back on the
  designer's start too; they are not judged. Batch_1 is a panel of three
  different boards whose parts our placement mixes (its verdict), at level
  0, where the relations term dominates; its row is less telling than the
  others.

## Caveats

- Rules: the default net class only (power classes are wider), no custom
  `.kicad_dru` rules (the OpenESC boards and Glasgow have some), no
  per-pad clearances, outer layers only (inner signal layers of the
  four- and six-layer boards are not counted for through-hole pads).
- Cuts are pad to pad between footprints; bodies, silkscreen and the
  courtyards that make the placer's spacing are not copper and do not
  count. Zones and existing tracks are ignored (the placed boards have
  none; the designer's are stripped).
- The flow is per part, without the competition of other parts' escapes
  through the same cuts, and with the diagonal-capacity overestimate. A
  joint flow of all parts would raise every deficit; whether it would
  separate the placements more is not measured.
- Our placement is one night's (one race winner per board); the seed
  check uses October binaries (x1828, x1851), not today's main.

## What I would build first

Neither proposal as written. In order:

1. **Measure the objective against the designer before searching it
   harder** (the finding of census 2). Per term of the skeleton's
   objective, which term ranks the designer's skeleton below ours on the
   gap boards, and what term would rank it above while keeping our seeds'
   order among themselves (the first route's rank, the
   `dense-noise.json` seeds). Candidates from census 1 and earlier work:
   the wire term's weighting of the large nets, the crossings against bus
   order (the designer's Glasgow skeleton has more ratsnest crossings and
   routes better), and room (the closed cuts below). The skeleton hook in
   the trace makes this a Python loop over stored traces, no placement.
2. **The closed-cut share as a stage-0 check of packing**, not of seeds:
   it tells ours from the designer's on every over-full board (twice the
   designer's share) and costs seconds per board with `cuts.py`. Whether
   relaxing the packing (fewer cuts below one track around the many-pin
   parts) helps the first route is the stage-1 question; it is the
   placer's halo and level policy, not a quantum snap.
3. **The planar escape deficit** only as a diagnostic where it is large
   (SmartSpin2k's copies, Glasgow's FPGA): it ranks seeds no better than
   the objective, and with vias it says nothing on ten of eleven boards.

Proposal B's step 0 at the router's stall sites (the binding cut on the
trouble-maker's path) is a different measurement and stays open; this
census says only that the placement's cuts, as placed, are not where the
dense boards' gap is decided.
