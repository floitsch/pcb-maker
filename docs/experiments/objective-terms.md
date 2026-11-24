# The objective against the designer, term by term (2026-11-24)

The cut census (docs/experiments/cut-census.md, census 2) found that the
skeleton's objective scores the designer's arrangement of the active parts,
legalised and refined by our stages, worse than every one of our seeds on
Glasgow, the 4in1-mini, Batch_1 and capybully, although the designer's
placement routes with 51 to 140 fewer open connections. docs/placer.md
says the opposite ("The objective is not what is wrong", from 2026-11-04).
This note reconciles the two, splits the objective by term, looks for a
routing quantity the objective lacks, and proposes changes.

Stage 0 (rescoring stored placements) plus 30 pinned first routes of the
placements it scores. Scripts in `experiments/objective-terms/`:
`terms.py` (the trace's objective by term), `measures.py` (routing proxies
from the pads of a placed board), `labelled.py` (place checks and pinned
first routes), `report.py` (these tables). Data under
`build/scratch/fund-objective/` (not in git): `recon/runs/` (place checks
with the trace: the census's runs of Glasgow, the 4in1 and the 4in1-mini,
and video, SNSP-CPU-01 and the plain-pipeline runs made here; binary
`pcb-maker-fobj0`, main 96e891b2 plus the census's trace hook, whose code
equals main 262adf59), `recon/firstroute/` (the first routes),
`traces.json` and `measures.json` (the pad measures), `report.md`
(`report.py`'s output). The October seeds are the 99 placements of
`dense-noise.json` (nine per dense board, binaries x1828 and x1851, each
with its first route's open terminals), as the cut census extracted them.

## Verdict

- **The 2026-11-04 claim does not hold today, and was weaker than its
  summary then.** It scored the *whole* objective (all parts, relations,
  overlap), one seed against one seed, on the plain pipeline, with the
  designer's poses seated by the plain legaliser. Re-run on today's code
  in that framing, the designer's legalised placement scores exactly what
  it scored then (the 4in1: relations 7544, 51 crossings; first route 56
  open terminals, as then), but our own plain placements improved on the
  objective (the 4in1's relations 10 317 to 6582, crossings 190 to 92;
  first route 174 to 116): the ratio is now 1.10 on the 4in1 and 1.09 on
  Glasgow (then 0.72 and 1.02). With today's skeleton pipeline the
  designer is 1.17 to 1.22 of our median, worse than five of six seeds on
  both. The 2026-11-04 study itself had found that on the 4in1 and
  Glasgow the anneal lowers our objective while routing gets worse ("our
  objective is wrong after all"); the summary line dropped that.
- **Wire and crossings are not where the designer is better.** At the
  designer's own poses the wire and crossing terms are inside or above our
  seeds' range on every gap board (the 4in1-mini: 465 against 284 mm of
  two-pin wire, 251 against 82 crossings). The relations (the automatic
  decoupling distances at `constraint_weight` 50 mm per mm) cost the
  designer most on Glasgow (3268 against 119 to 932) and video. The old
  "large nets 1.22 times longer in ours" is gone on today's seeds
  (Glasgow 1.02, the two OpenESC boards 1.00). **Among our own seeds the
  objective's terms rank the first route well**: crossings at Spearman
  +0.94, +1.00 and +0.57 (Glasgow, the 4in1, the 4in1-mini; six seeds
  each), wire +0.77, +0.83, +0.49. The objective orders our arrangements
  right and misjudges the designer's.
- **What the designer has that our placements lack is room at the pins of
  the active parts.** Pairs of pads of different parts and nets closer
  than one track's quantum (`w + 2 s`) with at least one active part:
  the designer 32, 1 and 14 on Glasgow, the 4in1 and the 4in1-mini; our
  seeds 134 to 180, 212 to 275 and 141 to 183, every seed. Between two
  passives the designer packs as tight as we do (the 4in1: 160 against
  95 to 123). On the boards without a placement gap the designer has as
  many such pairs as ours or more (protoesp 52 against 1, the 0xCB panel
  24 against 16), or both have almost none (MegaDrive, SNSP-CPU-01);
  DSP-ADAU1452 and the split72 keyboard are exceptions (the designer far
  fewer, yet no or a small gap). The objective has no term for it, and
  our stages close it: the designer's arrangement seated and refined by
  our pipeline has as many as ours (152, 204, 146).
- **The routes do not show that lost pin room is the cause.** Pinned first
  routes (open terminals): the designer's own placement 19, 33 and 105 on
  Glasgow, the 4in1 and the 4in1-mini; our seeds 88 to 137, 93 to 187,
  126 to 166. The designer's arrangement seated and refined by today's
  pipeline routes like ours or worse (114, 175, 117), with the pin room
  closed (152, 204, 146 pairs). The plain pipeline keeps half the 4in1's
  room (107) and most of its advantage (56), closes Glasgow's (140) and
  loses most of it (93). That is consistent with the room mattering, but
  in every placement that lost the room the active parts also moved (a
  mean 3.4 to 5.4 mm on Glasgow, 3.6 on the 4in1 through the skeleton,
  0.5 through the plain pipeline): room and arrangement fall together,
  and these routes cannot tell them apart. Within our seeds the pin room
  does not rank the first route (Spearman -0.14, +0.43, +0.17).
- **Candidates** (stage 0): (1) a pin-room term, about 5 to 15 mm of wire
  per pair of foreign pads within one track quantum of an active part's
  pad, in the legaliser, the refinement and the skeleton's legalisation;
  (2) the relations at a fifth (`constraint_weight` 10, a knob, recommended
  for stage 1 on 2026-11-04 and never run). Together they rank the
  designer's own placement above every seed on the three gap boards; the
  relations alone do not, and the pin-room term alone needs 39 mm a pair on
  Glasgow. Neither survives the second stage-0 test: added to the
  objective, the pin-room term ranks our October seeds by their first route
  worse than the objective alone (mean Spearman +0.28 to +0.25 at 3 mm,
  +0.18 at 5 mm; the 4in1-mini turns from +0.72 to -0.25).
- **Recommendation: build neither yet; no stage 1 for C2.** C2 alone fails
  stage 0 (it moves no gap board's designer above our seeds; on the plain
  pipeline the 4in1's designer carries more relations than ours, 7544
  against 6582), and its only role here was to let C1 work on Glasgow. C1
  waits for one direct test, queued: the designer's active parts pinned,
  our passives placed around them (partialcopy.py's `nonpassive`, which
  recovered 0.92 of the designer's first-route advantage on 2026-11-04).
  If our passives close the pin room there (pairs at our seeds' level)
  and the route still recovers most of the advantage, the room is not the
  cause and C1 is dropped: the gap is the active parts' arrangement, as
  placer.md says, and our objective does not prefer it. If the route
  recovers the advantage only where the room stays open, build C1 behind
  a switch (without C2) and take it to stage 1. Either way correct
  placer.md (wording below).

## 1. Reconciliation

**What 2026-11-04 measured** (docs/placer.md before 66346adc, "Where the
arrangement is lost (2026-11-04)", "The objective at the designer's
arrangement and ours"): binary `pcb-maker-diag1-a` (main x1904 plus that
package), before the race over skeletons (on from 2026-11-05); four boards
(video, the 4in1, Glasgow, SNSP-CPU-01); place checks at layout seeds 1
and 101, the kept seed; the placer's whole objective
(`anneal::objective` on the full problem at the placement's level: wire by
net size, overlap x 400, relations, hard x 1000, crossings x 3 mm); the
designer's poses started at `PCB_PLACER_START_STAGE=legalize` (each
level's legaliser seats them, the refinement follows; the plain legaliser
moved the active parts 0.01 to 0.05 of their spread); the ratio of that
total to our kept seed's at the same layout seed. Result 0.59 (video),
0.72 (the 4in1), 1.02 (Glasgow), 0.99 (SNSP). The same section's routed
table then showed the anneal from the designer's poses lowering the
objective to 0.60 of ours on the 4in1 while the first route went from 56
to 149 open terminals, and concluded "our objective is wrong after all"
for the 4in1 and Glasgow; the line in today's placer.md keeps only the
ratios.

**What the cut census measured**: the skeleton's own problem (the passives
absent, the skeleton parts' bodies grown by their passives' area, two-pin
passives' nets joined, halos off, no overlap term) at `skeleton_final`,
every kept seed of two place checks against the designer's poses
legalised by today's pipeline, which seats the skeleton against the grown
bodies first.

**Both on today's code** (whole: `objective` at `final`, overlap
included, as on 2026-11-04; skeleton: the census's; "below" counts our
seeds that score lower than the designer):

| board | 2026-11-04: designer / ours (seeds 1, 101) | today, same: designer / ours (kept seeds 1, 101) | today, whole: designer / our median (seeds below it) | today, skeleton: designer / our median (seeds below) | today, plain pipeline: designer / ours (kept seed 1; without overlap) |
| --- | --- | --- | --- | --- | --- |
| Glasgow revC3 | 1.02, 1.05 | 1.20, 1.26 | 1.22 (5/6) | 1.20 (6/6) | 1.09 (1.09) |
| OpenESC 4in1 | 0.72, 0.74 | 1.40, 1.27 | 1.17 (5/6) | 1.10 (4/6) | 1.11 (1.10) |
| OpenESC 4in1-mini | - | 0.57, 0.57 | 0.78 (1/6) | 1.09 (5/5) | 7.11 (1.31) |
| video | 0.59, 0.55 | 0.94, 0.92 | 0.79 (0/8) | 0.87 (0/6) | 0.62 (0.61) |
| SNSP-CPU-01 | 0.99, 1.04 | 1.01, 1.16 | 1.03 (16/20) | 1.04 (15/20) | 0.92 (0.94) |

(The 4in1-mini's kept seeds carry an overlap of 5 to 13 mm² priced at 400
a mm²; without it the designer is 1.08 of our median, worse than all six.
Its plain-pipeline designer keeps 14 mm² of overlap, hence 7.11. SNSP-CPU-01
has no placement gap today: our layouts leave 31 fewer open connections
than the designer's placement routed.)

Why they differ:

1. **Whole against skeleton matters less than the date.** On today's code
   the whole objective also ranks the designer's legalised placement below
   five of six seeds on Glasgow and the 4in1; the skeleton's is harsher on
   the 4in1-mini (5/5 against 1/6) because its wire and crossings join
   the series passives' nets.
2. **One seed against one seed.** The 2026-11-04 ratio was against the
   kept seed at the same layout seed; our seeds spread 1.3 to 1.6 times
   from best to worst on these boards, so one ratio near 1 says little.
3. **The designer's legalisation changed.** With the race over skeletons
   the designer's start is seated against the skeleton's grown bodies
   first: the active parts move a mean 5.2 mm on Glasgow (61 of 84 more
   than 0.5 mm), 3.6 mm on the 4in1, 1.4 mm on the 4in1-mini (the plain
   legaliser on 2026-11-04: a fixed point). On the 4in1 the designer's
   legalised objective went from 8525 (relations 7544, 51 crossings) to
   16 152 (relations 14 777, 163 crossings) while our seeds' stayed in
   11 500 to 16 700.
4. **Our own placements improved; the designer's legalised one did not
   change.** The plain pipeline re-run today seats the 4in1's designer
   exactly as on 2026-11-04 (relations 7544, 51 crossings, 56 open
   terminals at the first route, all three as then), but our plain seed
   now carries relations 6582 and 92 crossings (then 10 317 and 190) and
   routes 116 (then 174). The ratio rose from 0.72 to 1.10 because ours got
   better on the objective and on the route, while the designer still
   routes better (56 against 116).
5. **The designer's own poses** (no legalisation) on the whole objective
   without overlap: worse than every seed on Glasgow (8765 against 4590
   to 5960) and the 4in1-mini (1480 against 690 to 895), in the middle on
   the 4in1 (14 040 against 11 484 to 16 660); on video better than three
   of eight. Every gap board but the 4in1 ranks the designer's own
   arrangement below ours, so the verdict does not rest on our
   legalisation.

So the claim that holds today is the census's: on the boards with a
placement gap our objective, whole or skeleton, does not prefer the
designer's arrangement. Video, the board where the 2026-11-04 ratio was
lowest, still prefers it (0.79 of our median) and today has no placement
gap.

## 2. Per term

The whole objective (`objective_bare` at `final`: all parts, no halos;
overlap in mm², not in the total). Ours: the median and range of six to
eight seeds; the designer's rows give the value and how many of our seeds
are below it:

| board | placement | total | wire 2 pins | 3-8 | 9+ | half perimeter 9+ | crossings | relations | overlap mm² |
| --- | --- | --- | --- | --- | --- | --- | --- | --- | --- |
| Glasgow | ours | 4813 [4590, 5960] | 1752 [1629, 1903] | 1015 [938, 1147] | 82 [77, 84] | 658 [630, 683] | 578 [509, 695] | 275 [119, 932] | 0 |
| Glasgow | designer, own poses | 8765 (6/6) | 1935 (6/6) | 913 (0/6) | 82 (3/6) | 643 (1/6) | 689 (5/6) | 3268 (6/6) | 49 |
| Glasgow | designer, legalised and refined | 5655 (5/6) | 1896 (5/6) | 974 (2/6) | 84 (6/6) | 656 (3/6) | 576 (3/6) | 973 (6/6) | 1 |
| 4in1 | ours | 13721 [11484, 16660] | 446 [349, 624] | 224 [210, 252] | 97 [92, 115] | 549 [527, 629] | 102 [62, 215] | 12715 [10646, 15045] | 0 |
| 4in1 | designer, own poses | 14040 (3/6) | 410 (3/6) | 251 (5/6) | 101 (4/6) | 548 (3/6) | 189 (5/6) | 12711 (3/6) | 58 |
| 4in1 | designer, legalised and refined | 16152 (5/6) | 527 (4/6) | 260 (6/6) | 99 (4/6) | 550 (3/6) | 163 (4/6) | 14777 (5/6) | 0 |
| 4in1-mini | ours | 868 [690, 895] | 284 [260, 333] | 221 [181, 241] | 98 [96, 100] | 397 [387, 406] | 82 [50, 94] | 0 | 9 [5, 13] |
| 4in1-mini | designer, own poses | 1480 (6/6) | 465 (6/6) | 159 (0/6) | 103 (6/6) | 397 (3/6) | 251 (6/6) | 0 | 15 |
| 4in1-mini | designer, legalised and refined | 936 (6/6) | 379 (6/6) | 196 (1/6) | 89 (0/6) | 367 (0/6) | 91 (5/6) | 0 | 6 |
| video | ours | 61434 [51180, 79964] | 8052 [6752, 12627] | 13171 [12281, 15369] | 3412 [2972, 4290] | 10867 [9581, 13198] | 12044 [9358, 16195] | 196 [187, 2987] | 0 |
| video | designer, own poses | 64079 (5/8) | 6551 (0/8) | 13086 (3/8) | 3060 (1/8) | 9693 (1/8) | 8651 (0/8) | 15429 (8/8) | 191 |
| video | designer, legalised and refined | 48252 (0/8) | 6672 (0/8) | 12869 (1/8) | 3118 (2/8) | 9881 (2/8) | 8125 (0/8) | 1218 (7/8) | 0 |

The skeleton's objective tells the same with more weight on wire and
crossings (Glasgow's designer: 1630 against 1317 to 1415 mm of two-pin
wire, 391 against 313 to 349 crossings; the census's table has the rest).

Which terms rank the designer below us, relative to our seeds' spread:

- **Two-pin wire**, on Glasgow and the 4in1-mini (the designer 1.1 and 1.6
  times our median, above every seed): the designer's two-pin nets are
  longer. The three-to-eight-pin nets are the designer's shorter on both
  (0 of 6 seeds below). The nine-pin-plus nets are equal everywhere.
- **Crossings**, on the 4in1-mini (three times ours) and Glasgow (top of
  our range); the 4in1's designer has more too (189 against 102). By
  crossings the designer's placement is the worse one on every gap board.
- **Relations**, on Glasgow (12 times our median, still 3.5 times after our
  refinement repaired them) and video: the automatic decoupling relations
  (a capacitor within 2.5 mm of a supply pin of its IC) priced at 50 mm of
  wire a millimetre; the designer's capacitors sit further from the pins
  than we ask. On the 4in1 the relations are 90 % of every placement's
  objective and do not separate.
- Overlap: only the designer's own poses carry it (our spacing is stricter
  than the designer's courtyards); it is not in the totals above.

**Is HPWL or crossings simply the wrong proxy here?** On these boards,
yes, in the sense that the designer's placement routes far better while
being longer in two-pin wire and more crossed: on the 4in1-mini by a
factor of three in crossings. Both terms reward pulling parts together,
and nothing in the objective rewards what the designer buys with that
wire.

### What the objective lacks: room at the pins

`measures.py` computes from the pads of placed boards (the October seeds
with their first routes, today's seeds from the traces' final poses):
half perimeters by net size, minimum spanning tree crossings of the nets
no zone carries, RUDY overflow on 0.5 mm bins against a capacity from the
layers, the track pitch and the pad copper in each bin (at the nominal
capacity, half and a quarter of it), crossings weighted by the local
demand, pad density, spread, and the pairs of pads of different parts and
nets on a shared outer layer closer than one track quantum (`w + 2 s`,
`closed`) or two (`2 w + 3 s`, `narrow`), all pairs or only those with at
least one part that is not a free passive (`_active`).

Today's placements:

| board | placement | closed | closed_active | narrow_active | RUDY overflow at half capacity | crossings | half perimeter 2 / 3-8 / 9+ |
| --- | --- | --- | --- | --- | --- | --- | --- |
| Glasgow | ours | 198 [183, 221] | 147 [134, 180] | 321 [291, 334] | 452 [413, 561] | 727 [638, 853] | 1752 / 1300 / 658 |
| Glasgow | designer, own poses | 79 (0/6) | 32 (0/6) | 175 (0/6) | 288 (0/6) | 814 (5/6) | 1935 / 1063 / 643 |
| Glasgow | designer, legalised and refined | 239 (6/6) | 152 (5/6) | 322 (3/6) | 388 (0/6) | 725 (3/6) | 1896 / 1124 / 656 |
| 4in1 | ours | 336 [312, 398] | 235 [212, 275] | 356 [328, 406] | 31 [26, 48] | 136 [84, 273] | 446 / 216 / 546 |
| 4in1 | designer, own poses | 161 (0/6) | 1 (0/6) | 134 (0/6) | 22 (0/6) | 242 (5/6) | 410 / 231 / 547 |
| 4in1 | designer, legalised and refined | 312 (0/6) | 204 (0/6) | 313 (0/6) | 35 (5/6) | 192 (4/6) | 527 / 247 / 550 |
| 4in1-mini | ours | 268 [235, 285] | 152 [141, 183] | 272 [248, 307] | 58 [45, 66] | 114 [79, 138] | 284 / 211 / 394 |
| 4in1-mini | designer, own poses | 24 (0/6) | 14 (0/6) | 98 (0/6) | 36 (0/6) | 266 (6/6) | 465 / 151 / 396 |
| 4in1-mini | designer, legalised and refined | 254 (1/6) | 146 (2/6) | 279 (4/6) | 51 (2/6) | 114 (3/6) | 379 / 178 / 367 |
| video | ours | 10 [6, 20] | 10 [6, 20] | 44 [33, 76] | 1970 [1528, 3724] | 12188 [9557, 16295] | 7793 / 14671 / 10867 |
| video | designer, own poses | 1 (0/8) | 1 (0/8) | 17 (0/8) | 1705 (3/8) | 8763 (0/8) | 6531 / 14688 / 9693 |
| video | designer, legalised and refined | 2 (0/8) | 2 (0/8) | 31 (0/8) | 1795 (3/8) | 8219 (0/8) | 6638 / 14527 / 9881 |

By the class of the two parts (Glasgow / the 4in1 / the 4in1-mini; A an
active part of fewer than ten pads, M one of ten or more, P a free
passive; the designer's own poses, the designer legalised and refined, our
seeds' range):

| pair | Glasgow | 4in1 | 4in1-mini |
| --- | --- | --- | --- |
| active with passive (AP + MP) | 12 / 84 / 95 to 123 | 1 / 125 / 147 to 221 | 14 / 94 / 88 to 122 |
| active with active (AA + AM + MM) | 20 / 99 / 17 to 57 | 0 / 70 / 45 to 65 | 0 / 27 / 47 to 62 |
| passive with passive (PP) | 47 / 47 / 40 to 68 | 160 / 160 / 95 to 123 | 10 / 10 / 92 to 130 |

- The designer keeps passives a track away from the pads of its active
  parts (1 to 14 such pairs against our 88 to 221), keeps its active
  parts apart on the OpenESC boards (none against 45 to 65; on Glasgow 20,
  inside our 17 to 57), and packs passives against passives as we do or
  tighter. That is a rule about
  escapes, not about density: the designer's pad density, spread and
  RUDY at the nominal capacity are not consistently lower.
- RUDY overflow at half capacity also ranks the designer first on all
  three gap boards (0 of 6 seeds below), but not at the nominal capacity
  (the 4in1-mini 1.45 of ours) or a quarter (video): the result depends
  on a capacity we cannot calibrate.
- **Our pipeline closes the room.** The designer's arrangement seated and
  refined by our stages has as many closed pairs at active parts as our
  seeds; the passive pairs keep the designer's count exactly (47, 160,
  10): what moves is the active parts (the skeleton's legalisation against
  grown bodies) and the passives next to them. Per stage of the
  designer's start (Glasgow, the 4in1, the 4in1-mini): the passives'
  legaliser 182, 281, 176 closed pairs, the refinement 239, 312, 254,
  while the refinement takes the crossings from 1013 to 725, 643 to 192,
  319 to 114. The refinement trades room for wire and crossings because
  the objective prices only those. On video our pipeline keeps the
  designer's room (2) and today video has no placement gap.
- On the boards without a placement gap the separation goes or turns:
  protoesp 52 against 1 to 3 (the designer more), the 0xCB panel 24
  against 8 to 24, SNSP-CPU-01 4 against none, MegaDrive none on either
  (October seeds). Two exceptions: the split72 keyboard (a placement gap
  of 3) has the gap boards' pattern (62 against 200 to 275), and so has
  DSP-ADAU1452 (6 against 44 to 82), which our layouts route as well as
  the designer's placement. The October seeds show the gap boards'
  separation as today's do (Glasgow 32 against 194 to 259, the 4in1 1
  against 200 to 271, the 4in1-mini 14 against 152 to 188; SmartSpin2k 162
  against 600 to 1115).

**As a seed ranker** (Spearman with the first route's open terminals over
the nine October seeds of each board, mean over the boards where it
varies): the objective +0.28, crossings +0.27, wire +0.21, RUDY overflow
-0.03 to +0.13, crossings weighted by demand +0.13, `closed` +0.13,
`narrow` +0.13, two-pin wire +0.19. Nothing pad-based ranks our seeds
better than the objective; the pin room, which separates the designer
from every seed by a factor of 4 to over 200, does not order our seeds
(their counts lie within a third of each other).

## 3. Candidates

Rescoring stored placements: the weight `w` (mm of wire per pair) at
which the designer's own placement scores at or below every one of our
seeds on the whole objective (trace's `objective_bare`; relations at full,
a fifth, or none):

| board | relations x 1 | x 0.2 | x 0 |
| --- | --- | --- | --- |
| Glasgow, `closed_active` | 39.1 | 14.9 | 9.6 |
| 4in1, `closed_active` | 11.0 | 3.9 | 2.1 |
| 4in1-mini, `closed_active` | 5.8 | 5.8 | 5.8 |
| video, `closed_active` | 2459 | 102 | 0 |
| Glasgow, `narrow_active` | 30.1 | 11.2 | 7.2 |

Added to the October seeds' objective, the mean Spearman with their first
route at `w` = 0, 2, 3, 5, 10, 20 mm: `closed_active` +0.28, +0.25, +0.25,
+0.18, +0.15, +0.15; `narrow_active` +0.28, +0.22, +0.18, +0.18, +0.17,
+0.19; the loss is the 4in1-mini (+0.72 to -0.25 at 5 mm) and the 4in1
(+0.45 to +0.12 at 10 mm).

**Candidate 1: a pin-room term.** For every pair of pads of two parts, on
a shared outer layer, of different nets, at least one of the parts not a
free passive, whose copper lies closer than `w + 2 s` of the board's
default class: `w_room` mm of wire (5 to 15). Priced where the room is
lost: the skeleton's and the passives' legaliser spot cost and the
refinement's move cost (beside escape demand and crossings), and the
anneal's if the work allows. The placer's core knows pad boxes
(`Component::pads`) but not their nets; the lowering would have to carry
each pad's net and the part's class. It is the same family as the levers
that failed (halos, `routing_room`, `shared_halos`, escape demand's
channels) and the closest to `pad_exits` (a pad closed entirely; -3.0 % at
stage 1 on its four boards, p 0.228). What is different is the evidence
and the grain: the halos act on bodies and shrink to nothing on exactly
the over-full boards where the room is lost (`fit_halos`), escape demand
counts tracks through channels between facing bodies, which our
placements keep while the pads at their edges touch; this term charges
the copper pairs the designer avoids and charges nothing between passives,
where the designer packs as tight as we do. What is not different: it is
one more term in a sum whose seeds it does not rank. At stage 0 it passes
the designer test only with candidate 2 (Glasgow) and fails the seed test.

**Candidate 2: the relations at a fifth** (`constraint_weight` 10 instead
of 50; it prices the user's near and relative constraints too). The
designer misses our decoupling distances by a little everywhere (Glasgow:
3268 at 50 mm/mm is 65 mm of misses over its capacitors), and the
refinement's relation repair cost the designer's legalised placement 40
open terminals on Glasgow on 2026-11-04. Alone it does not rank the
designer above our seeds (Glasgow 6 of 6 seeds still below, the 4in1 4,
the 4in1-mini 6), so it does not pass stage 0 alone; it is what lets
candidate 1 work on Glasgow at a moderate weight. It is a knob: stage 1
costs no code (`--placer '{"constraint_weight": 10}'`).

Neither survives stage 0 as the ladder asks (the designer above our seeds
*and* the boards that pass not broken, whose only offline proxy is the
seeds' ranking), so no stage 1 was run.

## 4. Labelled first routes: is lost pin room the cause?

Thirty first routes (layout seed 1, `stop_after_first_route`, every
movable part pinned at the placement named; `labelled.py route`, finished
2026-11-24 20:38), each beside the placement's pin room (`closed_active`),
its pad crossings and its weighted wire. `designer@1`: the designer's
poses seated and refined by today's pipeline (the skeleton's grown bodies
first); `designer-plain@1`: the same with `"skeleton_race": null`;
`ours-plain-s1`: our plain placement, kept seed; `ours-s*@n`: every traced
seed of the two place checks.

| board | placement | first-route open | closed_active | crossings | wire | active parts moved, mean mm |
| --- | --- | ---: | ---: | ---: | ---: | ---: |
| Glasgow | designer, own poses | 19 | 32 | 814 | 2916 | 0 |
| Glasgow | designer, plain pipeline | 93 | 140 | 640 | 2795 | 3.4 |
| Glasgow | designer, today's pipeline | 114 | 152 | 725 | 2940 | 5.4 |
| Glasgow | ours, plain | 103 | 210 | 548 | 2693 | |
| Glasgow | ours, six seeds | 88, 90, 100, 103, 127, 137 | 134 to 180 | 638 to 853 | 2778 to 2933 | |
| 4in1 | designer, own poses | 33 | 1 | 242 | 742 | 0 |
| 4in1 | designer, plain pipeline | 56 | 107 | 64 | 630 | 0.5 |
| 4in1 | designer, today's pipeline | 175 | 204 | 192 | 873 | 3.6 |
| 4in1 | ours, plain | 116 | 165 | 121 | 728 | |
| 4in1 | ours, six seeds | 93, 105, 122, 164, 172, 187 | 212 to 275 | 84 to 273 | 638 to 961 | |
| 4in1-mini | designer, own poses | 105 | 14 | 266 | 718 | 0 |
| 4in1-mini | designer, today's pipeline | 117 | 146 | 114 | 645 | 1.4 |
| 4in1-mini | designer, plain pipeline | 147 | 182 | 148 | 648 | 2.2 |
| 4in1-mini | ours, plain | 143 | 152 | 103 | 555 | |
| 4in1-mini | ours, six seeds | 126, 127, 139, 159, 166, 166 | 141 to 183 | 79 to 138 | 534 to 646 | |

Spearman with the first route, over the ten placements of each board (and
over our six seeds alone):

| measure | Glasgow | 4in1 | 4in1-mini |
| --- | --- | --- | --- |
| closed_active | +0.32 (-0.14) | +0.61 (+0.43) | +0.52 (+0.17) |
| RUDY overflow at half capacity | +0.56 (+0.60) | +0.70 (+0.31) | +0.23 (+0.23) |
| crossings | +0.36 (+0.94) | +0.54 (+1.00) | +0.03 (+0.57) |
| wire | +0.33 (+0.77) | +0.68 (+0.83) | -0.12 (+0.49) |

What the routes say:

- **The designer's arrangement through our pipeline loses its advantage.**
  Today's pipeline: Glasgow 19 to 114, the 4in1 33 to 175 (worse than our
  median), the 4in1-mini 105 to 117. The plain pipeline keeps the 4in1's
  (56) but not Glasgow's (93, inside our seeds); these two numbers equal
  the 2026-11-04 study's. Our legality and refinement, not the search,
  are what the designer's arrangement does not survive.
- **The pin room goes with it**, to our seeds' level in every legalised
  variant but the 4in1's plain one, which keeps half the room (107
  against our 212 to 275) and most of the advantage. Across all ten
  placements the room ranks the route at +0.32 to +0.61; the objective's
  wire and crossings at -0.12 to +0.68, because they rank the designer's
  own placement among the worst.
- **But the room is not separated from the arrangement.** Every
  placement that lost the room also moved the active parts (Glasgow 3.4
  and 5.4 mm on the mean; the 4in1 3.6 mm through the skeleton against
  0.5 mm through the plain pipeline). The 4in1's two legalisations differ
  in both at once. Within our seeds, all crowded, the room does not rank
  the route; the crossings do (+0.57 to +1.00).
- **The 4in1-mini's first-route gap is small**: the designer's own
  placement leaves 105 open, our seeds 126 to 166 (its 54-connection gap
  in the nightly is in the full layout, 37 against 91). It cannot carry a
  verdict.

So: lost pin room is consistent with the open connections, not shown to
cause them.

### Queued: the direct test

`build/scratch/fund-objective/recon/nonpassive.sh` (PID 1895169, started
2026-11-24 20:43; status `recon/nonpassive.status`, log
`recon/nonpassive.log`; gated on load at most 20 and 16 GB free, never
02:00 to 07:00, three two-thread jobs): six first routes, the designer's
active parts pinned (partialcopy.py's `nonpassive` rule) and the passives
placed by our layout, on Glasgow, the 4in1 and the 4in1-mini at layout
seeds 1 and 101 (`labelled.py route ... BOARD:nonpassive@1`). The active
parts do not move, so the arrangement is the designer's; whether the room
stays open is up to our passives. Read with `labelled.py results` (units
`<board>-nonpassive-s1`, `-s101`), and measure the pin room of each
layout's placed board
(`recon/firstroute/runs/<unit>/<board>/layout/placed/*.kicad_pcb`) with
`measures.py`'s `measure`.

- Room closed (pairs near our seeds' 134 to 275) and the route near the
  designer's (2026-11-04: 0.92 of the advantage recovered with these
  parts pinned): the room is not the cause; drop C1.
- Room open and the route near the designer's, or room closed and the
  route near ours: the room is implicated; build C1 behind a switch and
  take it to stage 1.

## Proposed wording for docs/placer.md

In "Where the arrangement is lost", replace the bullet "**The objective is
not what is wrong.** ..." with:

> - **The objective does not prefer the designer's arrangement.** On the
>   boards with a placement gap (Glasgow, the two OpenESC boards) the
>   whole objective and the skeleton's rank the designer's placement,
>   legalised and refined by our stages or at its own poses, below most
>   or all of our seeds (docs/experiments/objective-terms.md): it is
>   longer in two-pin wire, more crossed and misses the decoupling
>   relations, and keeps foreign copper a track away from its active
>   parts' pads, which no term prices (1 to 32 such closed pairs against
>   our 134 to 275). Seated and refined by today's pipeline the
>   designer's arrangement loses that room and its advantage (first
>   route: Glasgow 19 to 114, the 4in1 33 to 175 open terminals, our
>   seeds 88 to 187); whether the room or the moved active parts cost
>   the routes is not yet separated. Among our own seeds the objective's
>   crossings rank the first route well (Spearman +0.57 to +1.00). (On
>   2026-11-04 the designer scored 0.59 to 1.02 of one seed's whole
>   objective on the plain pipeline; our plain placements have since
>   improved on the objective, the designer's legalised one has not
>   changed, and the same study found the anneal lowering the objective
>   while the 4in1's and Glasgow's routes got worse.)

and in "Which placement decisions matter" end the paragraph with "So the
gap is the active parts' arrangement as a whole (banks of repeated parts
its sharpest case), which our search does not reach and our objective does
not prefer; ..." in place of "which our objective accepts and our search
does not reach".
