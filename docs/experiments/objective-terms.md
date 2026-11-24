# The objective against the designer, term by term (2026-11-24)

The cut census (docs/experiments/cut-census.md, census 2) found that the
skeleton's objective scores the designer's arrangement of the active parts,
legalised and refined by our stages, worse than every one of our seeds on
Glasgow, the 4in1-mini, Batch_1 and capybully, although the designer's
placement routes with 51 to 140 fewer open connections. docs/placer.md
says the opposite ("The objective is not what is wrong", from 2026-11-04).
This note reconciles the two, splits the objective by term, looks for a
routing quantity the objective lacks, and proposes changes.

Stage 0 (rescoring stored placements) plus 36 pinned first routes (the
placements it scores, and the designer's active parts with our passives)
and three place checks with the designer's active parts pinned. Scripts in `experiments/objective-terms/`:
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
  seeds 134 to 180, 186 to 247 and 141 to 183, every seed. Between two
  passives the designer packs as tight as we do (the 4in1: 160 against
  119 to 152). On the boards without a placement gap the designer has as
  many such pairs as ours or more (protoesp 52 against 1, the 0xCB panel
  24 against 16), or both have almost none (MegaDrive, SNSP-CPU-01);
  DSP-ADAU1452 and the split72 keyboard are exceptions (the designer far
  fewer, yet no or a small gap). The objective has no term for it, and
  our stages close it: the designer's arrangement seated and refined by
  our pipeline has as many as ours (152, 183, 146).
- **Lost pin room is not the cause of most of the gap.** Pinned first
  routes (open terminals): the designer's own placement 19, 33 and 105 on
  Glasgow, the 4in1 and the 4in1-mini; our seeds 88 to 137, 93 to 187,
  126 to 166. Seated and refined by today's pipeline the designer's
  arrangement routes like ours or worse (114, 175, 117) with the pin room
  closed (152, 183, 146 pairs), but its active parts move too (a mean 1.4
  to 5.4 mm). The direct test pins the designer's active parts exactly
  and lets our passives fill in: our passives close the room (Glasgow 145
  and 107 pairs, at our seeds' level) and the route still keeps 0.54 and
  0.65 of the designer's advantage there (57 and 48 open); over the three
  boards the median kept is 0.49 (2026-11-04: 0.92 with an older binary).
  The active arrangement carries about half the advantage by itself; the
  room may carry part of the rest (on each board the seed with less room
  closed routed better, 3 of 3). Within our seeds the room does not rank
  the route (Spearman -0.14, +0.43, +0.17).
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
  +0.20 at 5 mm; the 4in1-mini turns from +0.72 to -0.25).
- **The objective accepts the designer's active arrangement on Glasgow
  and the 4in1, and the search does not reach it.** With the designer's
  active parts pinned and our passives placed around them, the whole
  objective scores below every one of our seeds (Glasgow 4420 to 5237,
  median 4540, against 4590 to 5960; the 4in1 9224 to 10 430 against
  11 484 to 16 660), by crossings and relations, and those placements
  route better than our seeds. What the objective rejects in the
  designer's own placement is its passives and their relations. On the
  4in1-mini it does not hold (971 to 1022 against 690 to 895), nor does the
  route improve there.
- **Where the arrangement is lost: the refinement.** On Glasgow's plain
  pipeline it costs 40 of the 74 lost terminals (19, 53 after the
  legaliser, 93 after the refinement), moves the active parts from 1.1 to
  3.4 mm off the designer's poses and closes the pin room (74 to 140);
  87 % of its objective gain is the decoupling relations. On today's
  pipeline the skeleton's legaliser and refinement against grown bodies
  move the active parts 5 mm before any passive is placed (the
  refinement the larger move); its 114 cannot be split from stored
  traces.
- **Recommendation: drop C1, no stage 1 for C2.** C2 alone fails stage
  0, C1 fails the seed test and, by the direct test, would aim at the
  smaller part of the gap. The work is in the search and in the
  refinement's moves on an arrangement it is given: the objective prefers
  the designer's active arrangement where the gap is largest, and our
  refinement walks away from it while lowering the objective by the
  relations. Correct placer.md (wording below).

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
| 4in1 | ours | 338 [314, 399] | 210 [186, 247] | 328 [299, 381] | 31 [26, 48] | 136 [84, 273] | 446 / 216 / 546 |
| 4in1 | designer, own poses | 161 (0/6) | 1 (0/6) | 134 (0/6) | 22 (0/6) | 242 (5/6) | 410 / 231 / 547 |
| 4in1 | designer, legalised and refined | 315 (1/6) | 183 (0/6) | 290 (0/6) | 35 (5/6) | 192 (4/6) | 527 / 247 / 550 |
| 4in1-mini | ours | 268 [235, 285] | 152 [141, 183] | 272 [248, 307] | 58 [45, 66] | 114 [79, 138] | 284 / 211 / 394 |
| 4in1-mini | designer, own poses | 24 (0/6) | 14 (0/6) | 98 (0/6) | 36 (0/6) | 266 (6/6) | 465 / 151 / 396 |
| 4in1-mini | designer, legalised and refined | 254 (1/6) | 146 (2/6) | 279 (4/6) | 51 (2/6) | 114 (3/6) | 379 / 178 / 367 |
| video | ours | 10 [6, 20] | 10 [6, 20] | 44 [33, 76] | 1970 [1528, 3724] | 12188 [9557, 16295] | 7793 / 14671 / 10867 |
| video | designer, own poses | 1 (0/8) | 1 (0/8) | 17 (0/8) | 1705 (3/8) | 8763 (0/8) | 6531 / 14688 / 9693 |
| video | designer, legalised and refined | 2 (0/8) | 2 (0/8) | 31 (0/8) | 1795 (3/8) | 8219 (0/8) | 6638 / 14527 / 9881 |

By the class of the two parts (A a part that is not a free passive, P a
free passive; the designer's own poses / the designer legalised and
refined / our seeds' range):

| pair | Glasgow | 4in1 | 4in1-mini |
| --- | --- | --- | --- |
| active with passive (AP) | 12 / 107 / 95 to 123 | 1 / 179 / 168 to 238 | 14 / 93 / 88 to 122 |
| active with active (AA) | 20 / 45 / 17 to 57 | 0 / 4 / 8 to 18 | 0 / 53 / 47 to 62 |
| passive with passive (PP) | 47 / 87 / 40 to 68 | 160 / 132 / 119 to 152 | 10 / 108 / 92 to 130 |

- The designer keeps passives a track away from the pads of its active
  parts (1 to 14 such pairs against our 88 to 238), keeps its active
  parts apart on the OpenESC boards (none against 8 to 18 and 47 to 62;
  on Glasgow 20, inside our 17 to 57), and packs passives against
  passives as we do or tighter (except on the 4in1-mini). That is a rule
  about escapes, not about density: the designer's pad density, spread
  and RUDY at the nominal capacity are not consistently lower.
- RUDY overflow at half capacity also ranks the designer first on all
  three gap boards (0 of 6 seeds below), but not at the nominal capacity
  (the 4in1-mini 1.45 of ours) or a quarter (video): the result depends
  on a capacity we cannot calibrate.
- **Our pipeline closes the room.** The designer's arrangement seated and
  refined by our stages has as many closed pairs at active parts as our
  seeds: the active parts move (the skeleton's legalisation against grown
  bodies) and the passives with them. Per stage of the
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
  against 178 to 232, the 4in1-mini 14 against 152 to 188; SmartSpin2k 45
  against 351 to 819).

**As a seed ranker** (Spearman with the first route's open terminals over
the nine October seeds of each board, mean over the boards where it
varies): the objective +0.28, crossings +0.27, wire +0.21, RUDY overflow
-0.03 to +0.13, crossings weighted by demand +0.13, `closed` +0.08,
`closed_active` +0.01, `narrow` +0.09, two-pin wire +0.19. Nothing pad-based ranks our seeds
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
| 4in1, `closed_active` | 12.2 | 4.3 | 2.4 |
| 4in1-mini, `closed_active` | 5.8 | 5.8 | 5.8 |
| video, `closed_active` | 2459 | 102 | 0 |
| Glasgow, `narrow_active` | 30.1 | 11.2 | 7.2 |

Added to the October seeds' objective, the mean Spearman with their first
route at `w` = 0, 2, 3, 5, 10, 20 mm: `closed_active` +0.28, +0.25, +0.25,
+0.20, +0.17, +0.17; `narrow_active` +0.28, +0.22, +0.18, +0.17, +0.17,
+0.19; the loss is the 4in1-mini (+0.72 to -0.25 at 5 mm) and the 4in1
(+0.45 to +0.27 at 10 mm).

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
| 4in1 | designer, plain pipeline | 56 | 86 | 64 | 630 | 0.5 |
| 4in1 | designer, today's pipeline | 175 | 183 | 192 | 873 | 3.6 |
| 4in1 | ours, plain | 116 | 142 | 121 | 728 | |
| 4in1 | ours, six seeds | 93, 105, 122, 164, 172, 187 | 186 to 247 | 84 to 273 | 638 to 961 | |
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
  variant but the 4in1's plain one, which keeps more than half the room (86
  against our 186 to 247) and most of the advantage. Across all ten
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

### The direct test: the designer's active parts pinned, our passives

Six first routes (`nonpassive.sh`, finished 2026-11-24 21:11): every part
partialcopy.py's `nonpassive` rule counts as active pinned at the
designer's pose (checked: they did not move), the passives placed by our
layout at layout seeds 1 and 101, `stop_after_first_route`. The
arrangement of the active parts is the designer's; whether the room at
their pins stays open is up to our passives. Pin room and pad proxies
measured on each layout's placed board (`measures.py`; the parts the
rule leaves free, such as resistor arrays and some connectors, count as
active in `closed_active` as everywhere in this note). Recovery: the
share of the designer's first-route advantage over our seeds' median
that the placement keeps, (median - open) / (median - designer).

| board | layout seed | first-route open | recovery | closed_active | crossings | wire |
| --- | --- | ---: | ---: | ---: | ---: | ---: |
| Glasgow | 1 | 57 | 0.54 | 145 | 589 | 2843 |
| Glasgow | 101 | 48 | 0.65 | 107 | 621 | 2866 |
| 4in1 | 1 | 98 | 0.41 | 107 | 75 | 708 |
| 4in1 | 101 | 66 | 0.70 | 97 | 83 | 711 |
| 4in1-mini | 1 | 137 | 0.27 | 112 | 130 | 670 |
| 4in1-mini | 101 | 130 | 0.43 | 98 | 121 | 665 |

For comparison: the designer's own placement 19 / 33 / 105 open with 32 /
1 / 14 pairs; our seeds 88 to 137 / 93 to 187 / 126 to 166 open with 134
to 180 / 186 to 247 / 141 to 183 pairs, crossings 638 to 853 / 84 to 273 /
79 to 138, wire 2778 to 2933 / 638 to 961 / 534 to 646.

Applying the rule of the previous version of this note:

- **Glasgow, the clean case: room closed, route nearer the designer's.**
  Our passives close the room around the designer's exact active parts to
  our seeds' level (145 and 107 against 134 to 180; the designer 32), and
  the route keeps 0.54 and 0.65 of the advantage (57 and 48 open, against
  19 and our median 101.5). By the rule: the room is not the cause; drop
  C1.
- **The 4in1: room half closed, route in between** (97 and 107 pairs
  against 186 to 247 and the designer's 1; recovery 0.41 and 0.70). Not
  decisive either way.
- **The 4in1-mini: room mostly closed, route near ours** (98 and 112
  against 141 to 183; recovery 0.27 and 0.43), but its first-route gap is
  44 terminals, too small to carry a verdict.
- The median recovery is 0.49, against 0.92 measured on 2026-11-04 with
  x1904 (whose seeds left 174 and 149 open on the 4in1 and Glasgow; ours
  today median 143 and 101.5). With our passives the designer's active
  arrangement keeps about half its advantage, not nearly all.
- Of each board's two seeds, the one with less room closed routed better
  (3 of 3 boards; by chance one time in eight). The room may account for
  part of the half that is lost, besides the passives' own arrangement;
  three pairs cannot separate the two.
- With our passives around them the designer's active parts give fewer
  pad crossings than every one of our seeds on Glasgow (589 and 621
  against 638 to 853) and the 4in1 (75 and 83 against 84 to 273), at wire
  inside our range; on the 4in1-mini more wire and crossings than ours.
  The objective's dislike of the designer's own placement on Glasgow and
  the 4in1 is then largely about the designer's passives (and relations),
  not its active arrangement. These are pad proxies, not the placer's
  objective.

**Verdict on C1: drop it as the next bet.** Lost pin room is not the
cause of most of the gap: the designer's active arrangement with the room
closed by our passives still routes far better than our seeds on Glasgow
and keeps about half the advantage on the three boards. The other half
may owe something to the room, but that is a weak signal (three pairs)
for a term that already fails the seed test and belongs to a family of
levers that failed.

### The objective at the designer's active arrangement with our passives

Three place checks with the trace (`labelled.py place BOARD:pinned-s1`,
2026-11-24 21:18 to 21:23): the sweep's constraints plus the designer's
active parts pinned at their poses (the `nonpassive` place entries), the
passives placed by our placer, every kept seed. The whole objective
(`objective_bare` at `final`), against our place checks' seeds:

| board | term | ours: median [min, max] (six seeds) | active parts pinned (kept seeds) | our seeds below the pinned median |
| --- | --- | --- | --- | --- |
| Glasgow | total | 4813 [4590, 5960] | 4420, 4540, 5237 | 0 of 6 |
| Glasgow | wire | 2865 [2790, 2944] | 2837, 2852, 2872 | 1 of 6 |
| Glasgow | wire 2 pins / 3-8 / 9+ | 1752 / 1015 / 82 | 1844 / 926 / 83 | 4 / 0 / 3 of 6 |
| Glasgow | crossings | 578 [509, 695] | 474, 483, 535 | 0 of 6 |
| Glasgow | relations | 275 [119, 932] | 99, 99, 963 | 0 of 6 |
| 4in1 | total | 13721 [11484, 16660] | 9224, 9733, 10430 | 0 of 6 |
| 4in1 | wire | 774 [652, 976] | 716, 721, 730 | 2 of 6 |
| 4in1 | crossings | 102 [62, 215] | 54, 60, 61 | 0 of 6 |
| 4in1 | relations | 12715 [10646, 15045] | 8311, 8838, 9547 | 0 of 6 |
| 4in1-mini | total | 868 [690, 895] | 971, 972, 990, 993, 1022 | 6 of 6 |
| 4in1-mini | wire 2 pins | 284 [260, 333] | 369 to 396 | 6 of 6 |
| 4in1-mini | crossings | 82 [50, 94] | 94 to 115 | 6 of 6 |

No pinned placement left a part without a place or illegal, and none
carries overlap.

- **On Glasgow and the 4in1 the objective accepts the designer's active
  arrangement, and the search does not reach it.** With our passives
  around it, it scores below every one of our seeds (Glasgow 4540 at the
  median against 4590 to 5960; the 4in1 9733 against 11 484 to 16 660),
  by fewer crossings (Glasgow 483 against 509 to 695, the 4in1 60 against
  62 to 215), shorter three-to-eight-pin wire (on the 4in1 also
  nine-pin-plus) and fewer missed relations; only the two-pin wire is longer on Glasgow. Our
  seeds, free to take that arrangement, end higher. These are also the
  two boards where the designer's active parts with our passives route
  better than our seeds (Glasgow 57 and 48 open against 88 to 137, the
  4in1 98 and 66 against 93 to 187): there the objective and the route
  agree.
- **On the 4in1-mini it does not**: the designer's active arrangement
  with our passives scores above every seed (971 to 1022 against 690 to
  895), by two-pin wire and crossings, and routes no better than our
  seeds (137 and 130 against 126 to 166). The mini's first-route gap is
  small; its 54-connection gap in the nightly is in the full layout.
- What the objective rejects in the designer's own placement on Glasgow
  and the 4in1 is therefore its passives and their relations, not its
  active arrangement.

### Where the designer's Glasgow arrangement is lost (stored traces)

The designer's poses on Glasgow route with 19 open terminals, after our
plain pipeline with 93, after today's with 114. From the stored traces
of `designer-plain` and `designer` (displacement of the active parts
from the designer's poses, pin room, the whole objective) and the routes
the 2026-11-04 study stored for the same stages of the plain pipeline
(its final stage, 93, is reproduced exactly today, as are the 4in1's
numbers):

| pipeline | stage | active parts moved, mean mm (> 0.5 mm, > 2 mm) | passives moved, mean mm | pin room | objective (relations) | first-route open |
| --- | --- | --- | --- | ---: | --- | ---: |
| both | the designer's poses | 0 | 0 | 32 | 8765 (3268) | 19 |
| plain | legaliser | 1.11 (33, 11) | 0.45 | 74 | 9347 (4293) | 53 (2026-11-04) |
| plain | refinement and relation repair | 3.40 (64, 39) | 4.75 | 140 | 4547 (99) | 93 |
| today | skeleton legaliser (grown bodies) | +2.62 between the stages (43, 29) | absent | | | |
| today | skeleton refinement | +3.09 (34, 31) | absent | | | |
| today | after the skeleton, passives at the designer's poses | 5.00 (61, 53) | 0 | 183 | 10 395 (4510) | |
| today | passives' legaliser | 5.27 (68, 58) | 0.55 | 97 | 11 500 (5652) | |
| today | passives' refinement and relation repair | 5.40 (71, 61) | 6.15 | 152 | 5655 (973) | 114 |

(The skeleton's own stages place grown bodies, whose poses differ from
the footprints' for some parts; their moves are given between stages.)

- **On the plain pipeline the refinement does the most damage**: 40 of
  the 74 terminals lost (53 to 93), against the legaliser's 34. It moves
  the active parts from 1.1 to 3.4 mm off the designer's poses (39 of 87
  more than 2 mm) and the passives 4.75 mm, closes the pin room from 74
  to 140, and of its objective gain (9347 to 4547) 87 % is the relations
  (4293 to 99): the refinement and its relation repair pull the
  decoupling capacitors to the pins our automatic relations ask for, at
  50 mm of wire per mm missed.
- **On today's pipeline the skeleton's phase does it before any passive
  is placed**: its legaliser against the grown bodies and its refinement
  move the active parts 2.6 and 3.1 mm on the mean (to 5.0 mm off the
  designer's poses, 53 of 87 more than 2 mm), the larger move of the two
  being the refinement's; the passives' phase adds 0.4 mm to the active
  parts and again trades the relations (5652 to 973) for pin room (97
  to 152). No route of the intermediate stages is stored for today's
  pipeline, so its 114 cannot be split between the two phases.
- So the step that does the most damage is **the refinement**: on the
  plain pipeline by the stored routes, on today's by the displacement of
  the active parts (the skeleton's refinement) and the pin room (the
  passives' refinement). It lowers the objective at each move; it is
  also where the relations at 50 mm per mm decide where passives go.

## Proposed wording for docs/placer.md

In "Where the arrangement is lost", replace the bullet "**The objective is
not what is wrong.** ..." with:

> - **The objective accepts the designer's active arrangement, not its
>   passives.** With the designer's active parts pinned and our passives
>   placed around them, the whole objective scores below every one of our
>   seeds on Glasgow and the 4in1 (by crossings and relations), and those
>   placements route better than ours; on the 4in1-mini it scores above
>   them (docs/experiments/objective-terms.md). The designer's own
>   placement scores worse than our seeds: longer two-pin wire, more
>   crossings, missed decoupling relations. Our search does not reach the
>   active arrangement, and our refinement walks away from it when given
>   it: on Glasgow's plain pipeline the legaliser costs 34 open terminals
>   (19 to 53) and the refinement 40 more (to 93), most of its objective
>   gain being the decoupling relations; today's pipeline moves the
>   active parts 5 mm in the skeleton's legaliser and refinement (114).
>   (On 2026-11-04 the designer's legalised placement scored 0.59 to 1.02
>   of one seed's whole objective; our plain placements have since
>   improved, the designer's has not changed.)

In "Which placement decisions matter" keep "which our objective accepts
and our search does not reach" but qualify it: "which our objective
accepts on Glasgow and the 4in1 (with our passives around it; not on the
4in1-mini) and our search does not reach".
