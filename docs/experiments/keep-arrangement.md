# Keeping the arrangement: who moves the active parts after the anneal (2026-11-26)

[fundamentals.md](fundamentals.md) (finding 2, follow-up 1) and
[objective-terms.md](objective-terms.md) (section 4) found that the
designer's arrangement of the active parts does not survive our stages: the
legaliser and the refinement move it 3 to 5 mm and the route loses with it.
The bet that followed: our own placements get worse for routing because the
stages after the search move the active parts away from the arrangement the
search chose, chasing the relations and the passives. This note measures
that on our own placements, tries what follows from the measurement, and
judges it by routed layouts.

Scripts in `experiments/keep-arrangement/`: `stages.py` (place checks with
the trace, the table per stage), `judge.sh` and `judge.py` (layouts at
several placer seeds, two binaries side by side), `sweep-subset.json` (the
sweep boards judged, fixed before any result). Data under
`build/scratch/keep-arrangement/` (not in git). Main is cb7743eb.

**Active part**: a movable part the race over skeletons' class rule does
not call a passive (`skeleton_passive` in `board_placer/skeleton.rs`: by
reference prefix, pad count and footprint name). The trace now says it per
part (`passive`), and holds the poses after the first refinement (`refined`)
and after the relation repair (`repaired`) beside `legal` and `final`.

## Verdict

- **On our own placements the active parts do move after the anneal, and
  the objective says which stage is which.** The legaliser moves them 2.8 mm
  on the mean on plain seeds and 4.7 mm in the skeleton and raises the
  objective on 163 of 179 seeds (outside the objective: it seats by
  distance); the refinement moves them a further 2.7 and 8.0 mm and lowers
  the objective on every seed (inside it); the objective ends where the
  anneal left it. The relation repair moves them 0.03 to 0.3 mm, and once a
  skeleton is set its parts are held already. So two of the study's three
  candidates had nothing to act on (anchoring after the skeleton, capping
  the relations' pull), and the third is what the legaliser does (largest
  first).
- **Holding the active parts through the refinement loses.** Short tier:
  unconnected items 9 to 21 over three seeds, worse at each, strict passes
  28 to 27. Sweep subset: 2173 to 2449 (+13 %), worse at each seed, strict passes 33 to 33. The change is removed.
- **Taking the legaliser's blind work away does not win either.** With the
  anneal reaching as far as the board for a part in overlap, the overlap it
  leaves falls on every board (23 to 99 %) and the legaliser moves the
  parts less, and the layouts are no better: short tier 9 to 9 unconnected,
  28 to 27 strict; sweep subset 2171 to 2281 over three seeds, 33 to 33
  strict, and one board (protoesp at one seed) left without a layout. It
  stays on its branch, not in this one.
- **So the bet does not hold**: our layouts do not lose their connections
  because late stages wear an arrangement off. Nothing here changes how
  the placer places. The branch brings the trace's new fields, the
  measurement scripts, this note and the corrections to docs/placer.md.

## 1. Which stage moves the active parts, and does the objective follow

Place checks of main with `PCB_PLACER_TRACE` (`stages.py place`, placer
seed 1, the 2026-11-25 nightly's sources and settings, pinned work rates,
two threads): the twelve short-tier boards at the short tier's budget (200 s
of placement work a seed, under the skeleton's 500 s: every seed places
plain) and twelve sweep boards at the sweep's (600 s: by skeleton), all of
them failing rows of that night: Glasgow, the two OpenESC boards, Batch_1,
MegaDrive, SmartSpin2k, eurorack-pmod, RP2040_base, SNSP-CPU-01, laptop
power, the OpenRX panel and the PolyKybd 4x4. Every kept seed of each place
check counts (3 to 16 a board). The objective is the trace's `objective_bare`
without its overlap term (wire, relations, hard constraints, 3 mm a
crossing over every counted net); for the skeleton's own stages it is
`skeleton_objective` (the passives absent, the bodies grown). Panels of
copies and sub-boards place by their own stages and have no rows
(SmartSpin2k, the OpenRX panel, the PolyKybd 4x4, urchin); Batch_1's place
check ends `incomplete` (a panel without `--reference`) but places and
traces.

Pooled over the seeds:

| budget | stage | seeds | active parts moved, mean mm | share moved > 0.5 mm | > 2 mm | turned | objective after / before, median | seeds where it rose |
| --- | --- | ---: | ---: | ---: | ---: | ---: | ---: | ---: |
| short (plain) | legaliser | 115 | 2.80 | 0.33 | 0.24 | 0.16 | 1.111 | 102 |
| short (plain) | refinement | 115 | 2.68 | 0.34 | 0.25 | 0.26 | 0.869 | 0 |
| short (plain) | relation repair | 115 | 0.03 | 0.01 | 0.00 | 0.01 | 1.000 | 0 |
| short (plain) | alignment | 115 | 0.00 | 0.00 | 0.00 | 0.02 | 1.006 | 92 |
| sweep, skeleton | legaliser | 27 | 4.67 | 0.35 | 0.29 | 0.17 | 1.087 | 25 |
| sweep, skeleton | refinement | 27 | 7.98 | 0.52 | 0.47 | 0.46 | 0.889 | 0 |
| sweep, skeleton | relation repair | 27 | 0.24 | 0.01 | 0.01 | 0.01 | 1.000 | 1 |
| sweep, skeleton | alignment | 27 | 0.00 | 0.00 | 0.00 | 0.05 | 1.002 | 18 |
| sweep, passives around the held skeleton | every stage | 27 | 0.00 | 0.00 | 0.00 | 0.00 | | |
| sweep, plain seeds (no skeleton, or its fallback) | legaliser | 10 | 7.28 | 0.43 | 0.36 | 0.27 | 2.439 | 9 |
| sweep, plain seeds | refinement | 10 | 6.36 | 0.39 | 0.34 | 0.38 | 0.752 | 0 |
| sweep, the last legaliser of every seed (the passives' or a plain seed's) | legaliser | 37 | | | | | 1.071 | 36 |
| sweep, the last refinement of every seed | refinement | 37 | | | | | 0.861 | 0 |

(The last two rows: RP2040_base, laptop power, eurorack-pmod's and
MegaDrive's plain seeds; the per-board table has them.)

From the anneal's placement to the final one the objective ends where it
began: final over annealed is 1.03 at the median on the short tier (above 1
on 72 of 115 seeds), 1.01 in the skeleton (15 of 27) and 1.00 over the
sweep's whole placements (20 of 37). The annealed placement overlaps, so
part of the legaliser's rise is the price of a legal placement, not a loss.

Per board (the mean over its kept seeds):

| board | budget | seeds (by skeleton) | active parts | stage | moved, mean mm | > 2 mm | turned | objective before -> after |
| --- | --- | --- | ---: | --- | ---: | ---: | ---: | --- |
| 0xCB 1337 v4.0 | short | 16 (0) | 10 | legaliser | 9.50 | 5.6 | 4.1 | 1470 -> 3724 |
| | | | | refinement | 0.62 | 0.6 | 1.9 | 3724 -> 2722 |
| USB_Cable_Tester | short | 16 (0) | 15 | legaliser | 0.00 | 0.0 | 0.0 | 2097 -> 2106 |
| | | | | refinement | 7.57 | 9.9 | 5.4 | 2106 -> 1901 |
| USB_Keypad | short | 8 (0) | 23 | legaliser | 7.09 | 13.0 | 9.6 | 2531 -> 12208 |
| | | | | refinement | 5.38 | 6.5 | 11.6 | 12208 -> 7671 |
| WLED_Matrix | short | 5 (0) | 84 | legaliser | 10.40 | 66.2 | 44.6 | 2227 -> 8787 |
| | | | | refinement | 3.73 | 36.2 | 44.4 | 8787 -> 4901 |
| framework_mobo_lefthalf | short | 16 (0) | 9 | legaliser | 1.55 | 2.1 | 1.7 | 1765 -> 2426 |
| | | | | refinement | 3.36 | 3.6 | 3.8 | 2426 -> 1848 |
| MIDAS-MK1.1 | short | 3 (0) | 36 | legaliser | 4.56 | 10.7 | 5.7 | 4220 -> 11043 |
| | | | | refinement | 5.27 | 11.0 | 11.3 | 11043 -> 3904 |
| OpenFC | short | 6 (0) | 82 | legaliser | 1.88 | 23.5 | 11.7 | 2052 -> 4532 |
| | | | | refinement | 3.07 | 26.7 | 19.0 | 4532 -> 2931 |
| reCamera | short | 3 (0) | 26 | legaliser | 0.11 | 0.3 | 2.0 | 388 -> 410 |
| | | | | refinement | 0.41 | 2.0 | 3.0 | 410 -> 378 |
| spell_tome_bottom | short | 16 (0) | 9 | legaliser | 0.40 | 0.5 | 0.3 | 174 -> 185 |
| | | | | refinement | 0.21 | 0.2 | 0.6 | 185 -> 177 |
| eternal-keypad | short | 10 (0) | 86 | legaliser | 0.15 | 1.7 | 1.1 | 917 -> 920 |
| | | | | refinement | 1.72 | 13.0 | 12.4 | 920 -> 815 |
| Blue_Line | short | 16 (0) | 25 | legaliser | 0.19 | 0.5 | 0.8 | 123 -> 128 |
| | | | | refinement | 0.34 | 0.9 | 3.4 | 128 -> 122 |
| Batch_1 | sweep | 3 (3) | 192 | skeleton legaliser | 22.03 | 166.7 | 93.0 | 8796 -> 29189 |
| | | | | skeleton refinement | 13.85 | 90.3 | 89.7 | 29189 -> 25420 |
| Glasgow revC3 | sweep | 3 (3) | 75 | skeleton legaliser | 1.67 | 19.0 | 18.7 | 2836 -> 3069 |
| | | | | skeleton refinement | 2.46 | 20.0 | 23.7 | 3069 -> 2840 |
| OpenESC 4in1-mini | sweep | 3 (3) | 44 | skeleton legaliser | 0.31 | 2.0 | 2.3 | 653 -> 686 |
| | | | | skeleton refinement | 0.61 | 5.0 | 7.0 | 686 -> 660 |
| OpenESC 4in1 | sweep | 3 (3) | 45 | skeleton legaliser | 2.01 | 11.3 | 6.0 | 772 -> 1390 |
| | | | | skeleton refinement | 2.62 | 13.3 | 17.3 | 1390 -> 996 |
| eurorack-pmod | sweep | 6 (3) | 37 | skeleton legaliser | 5.03 | 12.7 | 9.3 | 1309 -> 1671 |
| | | | | skeleton refinement | 2.85 | 7.0 | 10.7 | 1671 -> 1502 |
| MegaDrive | sweep | 3 (2) | 51 | skeleton legaliser | 8.74 | 25.0 | 15.0 | 55507 -> 72527 |
| | | | | skeleton refinement | 13.02 | 27.0 | 27.0 | 72527 -> 63597 |
| SNSP-CPU-01 | sweep | 10 (10) | 167 | skeleton legaliser | 1.53 | 16.8 | 8.3 | 25633 -> 26729 |
| | | | | skeleton refinement | 12.23 | 93.1 | 85.9 | 26729 -> 23787 |
| RP2040_base | sweep | 3 (0) | 55 | legaliser | 17.62 | 30.0 | 21.0 | 9571 -> 31099 |
| | | | | refinement | 13.26 | 27.0 | 28.7 | 31099 -> 24049 |
| laptop power | sweep | 3 (0) | 46 | legaliser | 2.89 | 14.0 | 12.7 | 2373 -> 9792 |
| | | | | refinement | 2.65 | 11.0 | 15.7 | 9792 -> 3359 |

What it says:

- **Once the skeleton is set, its parts do not move.** The passives are
  placed around a held skeleton (`place_skeleton` fixes it), so "anchor the
  active parts once the skeleton is set" is what the sweep's budget does
  already. The moves are inside the skeleton's own placement, between its
  anneal and its end, and on plain seeds.
- **The relations do not move the active parts.** The relation repair moves
  them 0.03 to 0.3 mm on the mean (1 % of them more than 0.5 mm); it moves
  the capacitors it names. Capping the relations' pull has nothing to cap.
- **The legaliser is the stage outside the objective.** It raises the
  objective on 102 of 115, 25 of 27 and 36 of 37 seeds, by 7 to 11 % at the
  median and by a factor of two to five on crowded boards (USB_Keypad, WLED
  Matrix, RP2040_base, laptop power, Batch_1's skeleton), and it moves the
  active parts 2.8 mm (plain) and 4.7 mm (skeleton) on the mean, a quarter
  of them more than 2 mm. It seats the parts largest first, each at the
  nearest free spot, so the large parts do stay; the anneal ends with
  overlaps it sees and cannot leave (its moves are short and cold by the
  time its penalty is high: USB_Keypad 53 of 77 parts, 60 mm2), and where a
  group is too crowded for its level the later parts find room only at the
  group's border (0402 resistors 45 to 70 mm away on USB_Keypad and WLED
  Matrix). Some conflicts the anneal does not see at all (the far side of
  through-hole parts: 1337's connector J2 on the key switches' pins moves
  41 mm).
- **The refinement follows its objective and is the larger mover in the
  skeleton.** It never raises the objective (0 of 179 seeds), lowers it by
  11 to 14 % at the median, about what the legaliser added, and moves the
  active parts 8.0 mm on the mean in the skeleton (47 % of them more than
  2 mm, 46 % turned; SNSP-CPU-01 12 mm, 86 of 100 turned), 2.7 mm on plain
  seeds. It does not put the parts back: it takes them where its own sum
  is lower (all crossings, where the anneal counts nets of up to 16 pins,
  and escape demand, which the anneal does not have).
- So on our own placements the active parts are moved after the anneal by
  the legaliser (outside the objective) and then, further, by the
  refinement (inside it), and the objective ends where the anneal left it.
  Whether those moves cost routed connections is not something the
  objective can say; the routed comparison below does.

### Why the legaliser has that much to move

The anneal ends with overlaps it sees itself. Of the parts the last anneal
of a seed moves (every part on a plain seed, the passives around a held
skeleton), those it leaves in overlap by its own sum
(`anneal_overlap`), and how far the legaliser then moves them, against the
anneal's own reach (a shift is at most the stage's radius on each axis: 6 %
of the board's larger side at the first stage, here of the parts' extent,
1.5 grid steps at the last):

| board | budget | parts the last anneal moves, a seed | in overlap at its end | the legaliser moves those, median mm | beyond the last stage's radius | beyond the first stage's |
| --- | --- | ---: | ---: | ---: | ---: | ---: |
| 0xCB 1337 v4.0 | short | 27 | 9.9 | 6.38 | 0.70 | 0.61 |
| USB_Cable_Tester | short | 46 | 2.1 | 2.03 | 0.50 | 0.50 |
| USB_Keypad | short | 77 | 53.5 | 17.45 | 0.87 | 0.76 |
| WLED_Matrix | short | 137 | 82.8 | 9.88 | 0.87 | 0.66 |
| framework_mobo_lefthalf | short | 38 | 16.2 | 4.58 | 0.71 | 0.67 |
| MIDAS-MK1.1 | short | 160 | 7.3 | 2.58 | 0.77 | 0.27 |
| OpenFC | short | 161 | 26.0 | 4.35 | 0.72 | 0.62 |
| reCamera | short | 82 | 3.3 | 0.10 | 0.30 | 0.10 |
| eternal-keypad | short | 86 | 0.8 | 0.91 | 0.50 | 0.00 |
| Batch_1 | sweep | 195 | 140.7 | 8.89 | 0.85 | 0.43 |
| Glasgow revC3 | sweep | 162 | 20.3 | 1.43 | 0.79 | 0.10 |
| OpenESC 4in1-mini | sweep | 145 | 33.0 | 0.30 | 0.52 | 0.31 |
| OpenESC 4in1 | sweep | 176 | 65.3 | 1.93 | 0.74 | 0.45 |
| RP2040_base | sweep | 97 | 28.3 | 22.76 | 0.93 | 0.80 |
| eurorack-pmod | sweep | 108 | 18.7 | 0.76 | 0.60 | 0.14 |
| laptop power | sweep | 174 | 74.0 | 3.89 | 0.75 | 0.48 |
| MegaDrive | sweep | 214 | 13.3 | 0.64 | 0.45 | 0.12 |
| SNSP-CPU-01 | sweep | 205 | 28.4 | 0.90 | 0.48 | 0.03 |

Over all: 3010 parts in overlap, 0.75 of them seated beyond the last stage's radius, 0.50 beyond the first stage's.

The room these parts need lies farther than the anneal can move them: by
the time its penalty makes an overlap dear (0.5 mm of wire a mm2 at the
first stage, 400 at the last) its shifts are a grid step or two, and a part
inside a crowded group has only crowded spots within reach. The legaliser
then finds the room, by distance alone.

After the first route the coupled loop keeps 68 moves of active parts over
the 108 layouts of the 2026-11-25 sweep (0.6 a board, a median 0.64 mm,
each kept only when its reroute leaves no more open): not a stage that
loses an arrangement.

## 2. Mechanisms

### Tried at placement alone and dropped

Two changes to the legaliser, each from a stated property, each checked by
place checks against main at the same placer seed (the objective at
`final`, the median over the seeds both sides kept):

- **A part legal where it stands keeps its spot** (in the legaliser's
  order, a part whose pose is legal among the fixed parts and those standing
  so far is entered before the search starts, so that a larger part that
  must move does not take its spot). Final objective against main: 0.90,
  1.00, 1.12, 0.94, 1.08, 1.18, 0.83, 1.00 on eight short boards, 1.00, 0.91,
  0.84, 0.98 and 1.40 on Glasgow, the 4in1-mini, the 4in1, eurorack-pmod and
  MegaDrive (geometric mean 1.00). The skeleton's own objective got worse
  where it matters: after its legaliser 1390 to 1738 (the 4in1), 1671 to
  2297 (eurorack-pmod), 72 527 to 97 192 (MegaDrive), at its end 1003 to
  1107, 1502 to 1785, 58 790 to 71 350: with the small parts standing, the
  large ones that must move go farther.
- **A crowded group pushed apart as a whole before the legaliser** (pairs in
  conflict move apart along the axis of least overlap, the two sharing the
  way in inverse proportion to their areas, 64 rounds). Final objective
  against main 1.00, 1.01, 1.00, 1.01 on USB_Keypad, WLED Matrix,
  eurorack-pmod and MegaDrive; the legaliser's own rise unchanged on
  USB_Keypad (12 208 against 12 233). A jam bounded by fixed parts and the
  outline does not open by pushing.

A third change, to the anneal, from the reading that its moves are too
short to leave a jam by the time its penalty is high:

- **A part the anneal sees in overlap keeps the first stage's reach**
  (the others the stage's shrinking one). Final objective against main at
  thirteen boards: a geometric mean of 1.00 (0.85 to 1.20); the legaliser
  moves the parts as far as before (USB_Keypad 22.4 against 21.7 mm on
  the mean over all parts, its objective after the legaliser 12 208
  against 12 274). The first stage's reach is too short as well: half the
  parts in overlap are seated beyond it (the table above).

None of the three moved what it was meant to move, so none went to
routing.

### Judged by routing, 1: the active parts anchored through the refinement

The bet in its plainest form, on what the measurement leaves of it: the
global placement and the anneal choose the active parts' arrangement
together; the legaliser has to move them (legality), the refinement does
not have to. So **the refinement and the alignment leave the active parts
where the legaliser seated them** (`Constraints::anchored`, set by the KiCad
layer from the class rule; `legal::refine_with` skips an anchored part as a
mover and as a partner of a swap, `legal::align_orientations` does not turn
it). The passives are refined as before; a relation's repair still moves
the part it names, which is a requirement, not the sum (and moves active
parts 0.03 to 0.3 mm). Under the race over skeletons this is the skeleton's
own refinement gone (its parts are all active but the passives a user's
constraint names); on plain seeds, and in a panel's copies and sub-boards,
the active parts stand while the passives are refined around them. It has
no setting and no threshold, and "active" is the class rule the skeleton
already places by.

It does what it means to at placement alone (place checks at seed 1,
against main, the seeds both kept):

| board (budget) | seeds both kept | final objective main -> anchored (median ratio) | crossings | active parts moved after the legaliser, mm (turned) | skeleton: after its legaliser, mm (turned) | skeleton objective at its end | unplaced or illegal |
| --- | ---: | --- | --- | --- | --- | --- | --- |
| 0xCB 1337 v4.0 (short) | 16 | 2607 -> 2658 (1.005) | 121 -> 127 | 0.70 -> 0.06 (2.4 -> 0.8) | - -> - (- -> -) | - -> - | 0 -> 0 |
| USB_Keypad (short) | 8 | 7084 -> 7628 (1.087) | 176 -> 199 | 5.65 -> 0.02 (11.5 -> 0.9) | - -> - (- -> -) | - -> - | 0 -> 0 |
| WLED_Matrix (short) | 5 | 4672 -> 5894 (1.206) | 246 -> 342 | 3.74 -> 0.00 (46.0 -> 0.4) | - -> - (- -> -) | - -> - | 0 -> 0 |
| framework_mobo_lefthalf (short) | 16 | 1861 -> 1942 (1.045) | 255 -> 276 | 3.36 -> 0.00 (3.4 -> 0.0) | - -> - (- -> -) | - -> - | 0 -> 0 |
| MIDAS-MK1.1 (short) | 3 | 3902 -> 4565 (1.060) | 427 -> 515 | 5.27 -> 0.05 (11.3 -> 0.3) | - -> - (- -> -) | - -> - | 0 -> 0 |
| OpenFC (short) | 6 | 2562 -> 2912 (1.108) | 314 -> 325 | 3.13 -> 0.05 (20.8 -> 0.5) | - -> - (- -> -) | - -> - | 0 -> 0 |
| eternal-keypad (short) | 10 | 823 -> 920 (1.108) | 85 -> 120 | 1.72 -> 0.00 (12.2 -> 0.0) | - -> - (- -> -) | - -> - | 0 -> 0 |
| Glasgow revC3 (sweep) | 3 | 5272 -> 5405 (1.034) | 716 -> 765 | 0.00 -> 0.00 (0.0 -> 0.0) | 2.46 -> 0.00 (23.0 -> 0.0) | 2861 -> 3069 | 0 -> 0 |
| OpenESC 4in1-mini (sweep) | 3 | 963 -> 861 (0.917) | 119 -> 93 | 0.00 -> 0.00 (0.0 -> 0.0) | 0.61 -> 0.00 (7.0 -> 0.0) | 667 -> 676 | 0 -> 0 |
| OpenESC 4in1 (sweep) | 3 | 14798 -> 14927 (1.017) | 160 -> 169 | 0.00 -> 0.00 (0.0 -> 0.0) | 2.62 -> 0.00 (17.7 -> 0.0) | 1003 -> 1390 | 0 -> 0 |
| eurorack-pmod (sweep) | 6 | 3256 -> 4088 (1.261) | 398 -> 562 | 0.39 -> 0.00 (3.7 -> 0.0) | 2.85 -> 0.00 (12.0 -> 0.0) | 1502 -> 1633 | 0 -> 0 |
| MegaDrive (sweep) | 3 | 59431 -> 63652 (1.053) | 7532 -> 8066 | 4.61 -> 0.91 (7.0 -> 3.0) | 15.57 -> 5.05 (26.5 -> 7.5) | 58790 -> 63833 | 0 -> 0 |
| SNSP-CPU-01 (sweep) | 9 | 26150 -> 29263 (1.115) | 3684 -> 4581 | 0.00 -> 0.00 (0.0 -> 0.0) | 12.53 -> 0.00 (87.1 -> 0.1) | 23895 -> 26833 | 0 -> 0 |

(The objective's ratio over the boards: a geometric mean of 1.075. On MegaDrive
the relation repair still moves the active parts its relations name.)

The objective at the end is higher, as it must be: the refinement's gain on
the active parts is given up. The question is whether routing prefers the
anneal's arrangement to the refinement's.

### Judged by routing, 2: a part in overlap reaches as far as the board

The legaliser's side of the bet, from "Why the legaliser has that much to
move": the room a part in overlap needs lies beyond the anneal's reach, so
the anneal leaves the overlap and the legaliser resolves it by distance,
blind to the objective. So **a part the anneal sees in overlap draws its
shift's reach between the stage's radius and the board's larger side,
evenly in the logarithm** (`Run::propose` in `anneal/moves.rs`; the other
parts keep the stage's radius; a move is taken by the rule it always was,
so early, when an overlap is cheap, nothing jumps). The anneal then finds
the room itself, and which part gives way and where it goes is judged by
wire, crossings and relations. No setting, no threshold, no class of
parts. On a strip where sixty parts want a spot that holds forty and the
other room is 47 mm off behind a fixed block, the anneal ended with every
part in overlap; now twenty cross the block and none is in overlap (the
unit test). On branch `keep-arrangement-n2` (7a5d5960, main plus the trace),
binary `pcb-maker-keep-arrangement-n2`.

At placement alone (place checks at seed 1 against main, the seeds both
kept):

| board (budget) | seeds both kept | overlap at the last anneal's end, mm2 | parts in overlap | the legaliser moves a part, mean mm | objective after the legaliser | final objective (median ratio) |
| --- | ---: | --- | --- | --- | --- | --- |
| 0xCB 1337 v4.0 (short) | 16 | 14.9 -> 8.1 | 9.9 -> 3.9 | 5.69 -> 3.77 | 3724 -> 3904 | 2607 -> 2536 (0.954) |
| USB_Keypad (short) | 8 | 60.0 -> 38.9 | 53.5 -> 39.4 | 22.41 -> 15.57 | 12208 -> 8707 | 7084 -> 5876 (0.835) |
| WLED_Matrix (short) | 5 | 44.5 -> 21.6 | 82.8 -> 38.4 | 12.08 -> 8.73 | 8787 -> 8801 | 4672 -> 4865 (1.014) |
| framework_mobo_lefthalf (short) | 16 | 10.7 -> 4.0 | 16.2 -> 12.7 | 4.70 -> 4.73 | 2426 -> 2454 | 1861 -> 1872 (0.979) |
| MIDAS-MK1.1 (short) | 3 | 58.9 -> 45.6 | 7.3 -> 3.3 | 4.27 -> 8.52 | 11043 -> 19531 | 3902 -> 4869 (1.365) |
| OpenFC (short) | 6 | 9.0 -> 2.1 | 26.0 -> 7.3 | 3.27 -> 2.20 | 4532 -> 3646 | 2562 -> 2305 (0.949) |
| eternal-keypad (short) | 10 | 0.7 -> 0.0 | 0.8 -> 0.3 | 0.15 -> 0.13 | 920 -> 908 | 823 -> 828 (1.011) |
| Glasgow revC3 (sweep) | 3 | 11.6 -> 1.6 | 33.0 -> 11.7 | 0.27 -> 0.24 | 5858 -> 5761 | 5272 -> 5171 (0.982) |
| OpenESC 4in1-mini (sweep) | 3 | 6.1 -> 2.5 | 48.7 -> 32.3 | 0.63 -> 0.31 | 1099 -> 1029 | 963 -> 925 (1.002) |
| OpenESC 4in1 (sweep) | 3 | 52.5 -> 7.9 | 86.3 -> 53.0 | 1.46 -> 1.50 | 16206 -> 15332 | 14798 -> 14284 (0.815) |
| eurorack-pmod (sweep) | 6 | 4.0 -> 1.1 | 27.8 -> 15.0 | 0.68 -> 0.56 | 3768 -> 3966 | 3256 -> 3345 (1.105) |
| MegaDrive (sweep) | 3 | 59.6 -> 0.5 | 16.0 -> 5.3 | 1.32 -> 0.11 | 68004 -> 62473 | 59431 -> 62528 (1.025) |
| SNSP-CPU-01 (sweep) | 10 | 9.4 -> 0.4 | 42.3 -> 12.1 | 0.37 -> 0.13 | 26697 -> 26250 | 26210 -> 26275 (0.981) |

(The final objective's ratio over the boards: a geometric mean of 0.994, a median of 0.982.)

It does what it means to: the overlap the anneal leaves falls on every
board (by 23 to 99 %), and the legaliser moves the parts less on ten of
thirteen. The final objective stays where it was: the refinement was
getting that back already. (MIDAS's three seeds took another walk through
the levels; its numbers are that, not the reach.)

## 3. Routed comparison

`judge.sh`: full layouts through `benchmarks/agent-tasks/run.py` as the
nightly runs them (the short tier: `--tier short`; the sweep: tasks v7,
`--quality`; pinned work rates, 3 boards x 2 threads a side, both sides at
once, through the ledger), main cb7743eb (`pcb-maker-main-cb7743eb`)
against the branch (`pcb-maker-keep-arrangement`, bb3a2068), at placer seeds
1, 101 and 201 on both sides (`--placer '{"tuning": {"seed": N}}'`; the
race places seeds N to N + 15). Judged by the totals of KiCad's unconnected
items and of strict passes with the seeds pooled, per set.

- **Short tier**: its 12 boards. At its budget every seed places plain, so
  this set judges the anchor on plain seeds.
- **Sweep subset**: 25 boards of the 2026-11-25 nightly's sweep, written
  down before any result (`sweep-subset.json`): 13 failing rows (Glasgow,
  the OpenESC 4in1 and 4in1-mini, Batch_1, MegaDrive, SmartSpin2k,
  eurorack-pmod, RP2040_base, SNSP-CPU-01, laptop power, the OpenRX panel,
  the PolyKybd 4x4, protoesp: the step-1 boards and protoesp) and 12 of its
  74 strict passes, every sixth by name. At the sweep's budget the seeds
  place by skeleton where the board is no panel, so this set judges the
  skeleton's refinement gone.
- **Held-out set** (`benchmarks/expansion/holdout`, 56 boards): route mode
  on the designer's placement. A placer change cannot move it; it is run
  once as an identity check only if the two sets above are a win.

Cost: 333 layouts, 64 hours of work in all (111 a side); a side's layouts
took 75 500 s of work on main, 76 800 with the anchor and 76 800 with the
anneal's reach, so neither change costs work.

Main's rows at seed 1 are the 2026-11-26 nightly's rows of the same commit
on all 37 boards (unconnected items and strict passes: the harness
repeats). Main's own seeds give the noise: on the sweep subset 775, 682 and
716 unconnected items and 12, 10 and 11 strict passes; on the short tier 2,
3 and 4, and 10, 10 and 8. The machine was loaded to 40 to 130 from other work during the later runs
(the anneal variant's sweep at seeds 101 and 201 and its short tier, the
anchor's sweep at seed 101, main's at seed 201), and the wall-clock guards
of 53 rows stretched to their fourfold cap. Every row of every run was
checked: none was cut by the wall clock or killed, no guard fired, no
check timed out. One row has no layout (protoesp under the anneal variant
at seed 201: a part without a legal place); it was rerun alone on both
sides at a lower load and came out the same on both, main's to the digit
of its work, so it is the binary's result and counts as one.

### The anchor

Short tier (12 boards, every seed plain):

| seed | boards with both | unconnected off -> on | strict off -> on | pass off -> on | first open off -> on | work s off -> on | without a result off, on |
| --- | ---: | --- | --- | --- | --- | --- | --- |
| 1 | 12 | 2 -> 5 | 10 -> 9 | 9 -> 9 | 25 -> 23 | 2306 -> 2459 | 0, 0 |
| 101 | 12 | 3 -> 7 | 10 -> 10 | 9 -> 10 | 19 -> 19 | 2048 -> 2382 | 0, 0 |
| 201 | 12 | 4 -> 9 | 8 -> 8 | 7 -> 8 | 24 -> 55 | 2374 -> 1992 | 0, 0 |
| pooled | | 9 -> 21 | 28 -> 27 | 25 -> 27 | 68 -> 97 | 6728 -> 6833 | |

Boards by unconnected items over their seeds: 2 better, 8 same, 2 worse; strict passes gained on 2 board-seeds, lost on 3.

| board | s1 off -> on | s101 off -> on | s201 off -> on | strict passes off -> on |
| --- | --- | --- | --- | --- |
| 0xCB-1337__1337-v4.0 | 0 -> 0 | 0 -> 0 | 0 -> 0 | 3 -> 3 |
| Business-Cards__USB_Cable_Tester__PCB | 0 -> 0 | 0 -> 0 | 0 -> 0! | 3 -> 2 |
| Business-Cards__USB_Keypad | 0! -> 0! | 0 -> 0! | 0! -> 0! | 1 -> 0 |
| Business-Cards__WLED_Matrix | 2! -> 4! | 2! -> 7! | 1! -> 8! | 0 -> 0 |
| MyKiCad__framework_mobo_lefthalf | 0 -> 0 | 0 -> 0 | 2! -> 0 | 2 -> 3 |
| ISS-PCB__MIDAS-MK1.1__MIDAS-MK1.1-revA | 0 -> 0 | 1! -> 0 | 1! -> 1! | 1 -> 2 |
| OpenFC-Lite__OpenFC | 0 -> 1! | 0 -> 0 | 0 -> 0 | 3 -> 2 |
| OSHW-reCamera-Series__reCamera_S101_v1.1 | 0 -> 0 | 0 -> 0 | 0 -> 0 | 3 -> 3 |
| spell_tome__spell_tome_bottom | 0 -> 0 | 0 -> 0 | 0 -> 0 | 3 -> 3 |
| eternal-keypad__eternal-keypad | 0 -> 0 | 0 -> 0 | 0 -> 0 | 3 -> 3 |
| urchin__main | 0 -> 0 | 0 -> 0 | 0 -> 0 | 3 -> 3 |
| CharlieBoard__Blue_Line | 0 -> 0 | 0 -> 0 | 0 -> 0 | 3 -> 3 |
(`!`: not a strict pass.)

Sweep subset (25 boards):

| seed | boards with both | unconnected off -> on | strict off -> on | pass off -> on | first open off -> on | work s off -> on | without a result off, on |
| --- | ---: | --- | --- | --- | --- | --- | --- |
| 1 | 25 | 775 -> 845 | 12 -> 11 | 11 -> 10 | 993 -> 1132 | 22129 -> 24267 | 0, 0 |
| 101 | 25 | 682 -> 826 | 10 -> 11 | 9 -> 10 | 960 -> 1073 | 23378 -> 23380 | 0, 0 |
| 201 | 25 | 716 -> 778 | 11 -> 11 | 10 -> 12 | 894 -> 1026 | 23309 -> 22325 | 0, 0 |
| pooled | | 2173 -> 2449 | 33 -> 33 | 30 -> 32 | 2847 -> 3231 | 68815 -> 69972 | |

Boards by unconnected items over their seeds: 5 better, 12 same, 8 worse; strict passes gained on 1 board-seeds, lost on 1.

| board | s1 off -> on | s101 off -> on | s201 off -> on | strict passes off -> on |
| --- | --- | --- | --- | --- |
| 0xCB-1337__1337-v4.0 | 0 -> 0 | 0 -> 0 | 0 -> 0 | 3 -> 3 |
| Business-Cards__Batch_1 | 137! -> 171! | 114! -> 127! | 107! -> 139! | 0 -> 0 |
| Business-Cards__WLED_Matrix | 0 -> 1! | 1! -> 1! | 1! -> 0! | 1 -> 0 |
| MyKiCad__protoesp | 3! -> 5! | 4! -> 1! | 2! -> 0! | 0 -> 0 |
| glasgow__glasgow__revC3 | 81! -> 64! | 79! -> 64! | 72! -> 82! | 0 -> 0 |
| ISS-PCB__BAGEL-MK1 | 0 -> 0 | 0 -> 0 | 0 -> 0 | 3 -> 3 |
| jiran-ble-lite__jiran-ble-lite | 0 -> 0 | 0! -> 0 | 0 -> 0 | 2 -> 3 |
| OpenESC-20x20__4in1-mini | 98! -> 66! | 72! -> 94! | 119! -> 81! | 0 -> 0 |
| OpenESC-30x30__4in1 | 81! -> 91! | 54! -> 65! | 77! -> 75! | 0 -> 0 |
| OpenRX__OpenRX-panel-rev2 | 71! -> 65! | 79! -> 67! | 98! -> 101! | 0 -> 0 |
| Telemetry__TelemetryOnboard | 0 -> 0 | 0 -> 0 | 0 -> 0 | 3 -> 3 |
| rp2040-motor-controller__RP2040_base | 11! -> 14! | 5! -> 11! | 2! -> 12! | 0 -> 0 |
| eurorack-pmod__eurorack-pmod-pcb | 18! -> 16! | 16! -> 20! | 6! -> 20! | 0 -> 0 |
| laptop__power | 9! -> 7! | 8! -> 7! | 13! -> 17! | 0 -> 0 |
| mackerel-68k__mackerel-10-v1 | 0 -> 0 | 0 -> 0 | 0 -> 0 | 3 -> 3 |
| SmartSpin2k__SmartSpin2k_Panelized | 51! -> 76! | 40! -> 37! | 24! -> 29! | 0 -> 0 |
| OpenAirScope__OpenAirScope | 0 -> 0 | 0 -> 0 | 0 -> 0 | 3 -> 3 |
| OnBoard__koeg-board-pcb | 0 -> 0 | 0 -> 0 | 0 -> 0 | 3 -> 3 |
| little-red-rover__little_red_rover | 0 -> 0 | 0 -> 0 | 0 -> 0 | 3 -> 3 |
| capybully_keyboard__capybully | 0 -> 0 | 0 -> 0 | 0 -> 0 | 3 -> 3 |
| MegaDrive__MegaDrive | 54! -> 68! | 18! -> 108! | 35! -> 54! | 0 -> 0 |
| nonSNES__SNSP-CPU-01 | 8! -> 62! | 8! -> 34! | 8! -> 26! | 0 -> 0 |
| PolyKybd__poly_kb_molecule_4x4 | 153! -> 139! | 184! -> 190! | 152! -> 142! | 0 -> 0 |
| CharlieBoard__Blue_Line | 0 -> 0 | 0 -> 0 | 0 -> 0 | 3 -> 3 |
| bumwings-kbd__bumwings_v001R64_rp2040zero_sd | 0 -> 0 | 0 -> 0 | 0 -> 0 | 3 -> 3 |
(`!`: not a strict pass.)

- **The short tier loses**: unconnected items 9 to 21, worse at each of the
  three seeds (2 to 5, 3 to 7, 4 to 9), 28 against 27 strict passes (main's
  own seeds: 10, 10 and 8), 25 against 27 passes; the first routes leave 68
  against 97 open terminals. The loss is WLED Matrix (5 to 19 over the
  seeds), the board whose legaliser moves 66 of 84 active parts more than
  2 mm and whose refinement brings them back: anchored, they stay where the
  legaliser threw them.
- **The sweep subset loses**: unconnected items 2173 to 2449 (+13 %), worse
  at each seed (775 to 845, 682 to 826, 716 to 778), strict passes 33 on
  both sides, passes 30 against 32; the first routes leave 2847 against
  3231 open terminals. The skeleton's refinement gone costs most where it
  moved most: SNSP-CPU-01 (12 mm on the mean, 86 of 100 parts turned) goes
  from 8 to 62, 34 and 26, MegaDrive from 54, 18 and 35 to 68, 108 and 54,
  Batch_1 from 137, 114 and 107 to 171, 127 and 139. Glasgow, the board the
  bet was drawn from, is better at two seeds of three (81 to 64, 79 to 64,
  72 to 82); the two OpenESC boards go both ways.
- So the refinement's moves of the active parts are worth connections on
  our own placements. The bet's plain form is refuted.

### The anneal's reach

Main's rows are those of the anchor's comparison (the same binary, seeds
and settings); only the branch's side ran (`pcb-maker-keep-arrangement-n2`).

Sweep subset (25 boards):

| seed | boards with both | unconnected off -> on | strict off -> on | pass off -> on | first open off -> on | work s off -> on | without a result off, on |
| --- | ---: | --- | --- | --- | --- | --- | --- |
| 1 | 25 | 775 -> 864 | 12 -> 11 | 11 -> 10 | 993 -> 1136 | 22129 -> 24384 | 0, 0 |
| 101 | 25 | 682 -> 702 | 10 -> 11 | 9 -> 10 | 960 -> 939 | 23378 -> 23986 | 0, 0 |
| 201 | 24 | 714 -> 715 | 11 -> 11 | 10 -> 10 | 887 -> 1024 | 21876 -> 21732 | 0, 1 |
| pooled | | 2171 -> 2281 | 33 -> 33 | 30 -> 30 | 2840 -> 3099 | 67383 -> 70102 | |

Boards by unconnected items over their seeds: 9 better, 11 same, 5 worse; strict passes gained on 2 board-seeds, lost on 2.

| board | s1 off -> on | s101 off -> on | s201 off -> on | strict passes off -> on |
| --- | --- | --- | --- | --- |
| 0xCB-1337__1337-v4.0 | 0 -> 0 | 0 -> 0 | 0 -> 0 | 3 -> 3 |
| Business-Cards__Batch_1 | 137! -> 140! | 114! -> 116! | 107! -> 95! | 0 -> 0 |
| Business-Cards__WLED_Matrix | 0 -> 0 | 1! -> 0 | 1! -> 1! | 1 -> 2 |
| MyKiCad__protoesp | 3! -> 4! | 4! -> 1! | 2! -> - | 0 -> 0 |
| glasgow__glasgow__revC3 | 81! -> 68! | 79! -> 63! | 72! -> 56! | 0 -> 0 |
| ISS-PCB__BAGEL-MK1 | 0 -> 0 | 0 -> 0 | 0 -> 0 | 3 -> 3 |
| jiran-ble-lite__jiran-ble-lite | 0 -> 0! | 0! -> 0 | 0 -> 0 | 2 -> 2 |
| OpenESC-20x20__4in1-mini | 98! -> 118! | 72! -> 65! | 119! -> 82! | 0 -> 0 |
| OpenESC-30x30__4in1 | 81! -> 96! | 54! -> 90! | 77! -> 81! | 0 -> 0 |
| OpenRX__OpenRX-panel-rev2 | 71! -> 72! | 79! -> 75! | 98! -> 76! | 0 -> 0 |
| Telemetry__TelemetryOnboard | 0 -> 0 | 0 -> 0 | 0 -> 0 | 3 -> 3 |
| rp2040-motor-controller__RP2040_base | 11! -> 1! | 5! -> 32! | 2! -> 13! | 0 -> 0 |
| eurorack-pmod__eurorack-pmod-pcb | 18! -> 25! | 16! -> 12! | 6! -> 11! | 0 -> 0 |
| laptop__power | 9! -> 9! | 8! -> 12! | 13! -> 15! | 0 -> 0 |
| mackerel-68k__mackerel-10-v1 | 0 -> 0 | 0 -> 0 | 0 -> 0 | 3 -> 3 |
| SmartSpin2k__SmartSpin2k_Panelized | 51! -> 34! | 40! -> 21! | 24! -> 54! | 0 -> 0 |
| OpenAirScope__OpenAirScope | 0 -> 0 | 0 -> 0 | 0 -> 0 | 3 -> 3 |
| OnBoard__koeg-board-pcb | 0 -> 0 | 0 -> 0 | 0 -> 0 | 3 -> 3 |
| little-red-rover__little_red_rover | 0 -> 0 | 0 -> 0 | 0 -> 0 | 3 -> 3 |
| capybully_keyboard__capybully | 0 -> 0 | 0 -> 0! | 0 -> 0 | 3 -> 2 |
| MegaDrive__MegaDrive | 54! -> 126! | 18! -> 54! | 35! -> 79! | 0 -> 0 |
| nonSNES__SNSP-CPU-01 | 8! -> 3! | 8! -> 4! | 8! -> 4! | 0 -> 0 |
| PolyKybd__poly_kb_molecule_4x4 | 153! -> 168! | 184! -> 157! | 152! -> 148! | 0 -> 0 |
| CharlieBoard__Blue_Line | 0 -> 0 | 0 -> 0 | 0 -> 0 | 3 -> 3 |
| bumwings-kbd__bumwings_v001R64_rp2040zero_sd | 0 -> 0 | 0 -> 0 | 0 -> 0 | 3 -> 3 |
(`!`: not a strict pass.)

Short tier (12 boards):

| seed | boards with both | unconnected off -> on | strict off -> on | pass off -> on | first open off -> on | work s off -> on | without a result off, on |
| --- | ---: | --- | --- | --- | --- | --- | --- |
| 1 | 12 | 2 -> 3 | 10 -> 9 | 9 -> 9 | 25 -> 22 | 2306 -> 2025 | 0, 0 |
| 101 | 12 | 3 -> 4 | 10 -> 9 | 9 -> 10 | 19 -> 14 | 2048 -> 2380 | 0, 0 |
| 201 | 12 | 4 -> 2 | 8 -> 9 | 7 -> 8 | 24 -> 17 | 2374 -> 2271 | 0, 0 |
| pooled | | 9 -> 9 | 28 -> 27 | 25 -> 27 | 68 -> 53 | 6728 -> 6676 | |

Boards by unconnected items over their seeds: 1 better, 9 same, 2 worse; strict passes gained on 2 board-seeds, lost on 3.

| board | s1 off -> on | s101 off -> on | s201 off -> on | strict passes off -> on |
| --- | --- | --- | --- | --- |
| 0xCB-1337__1337-v4.0 | 0 -> 0 | 0 -> 0 | 0 -> 0 | 3 -> 3 |
| Business-Cards__USB_Cable_Tester__PCB | 0 -> 0 | 0 -> 0 | 0 -> 0 | 3 -> 3 |
| Business-Cards__USB_Keypad | 0! -> 0! | 0 -> 0! | 0! -> 0! | 1 -> 0 |
| Business-Cards__WLED_Matrix | 2! -> 2! | 2! -> 3! | 1! -> 1! | 0 -> 0 |
| MyKiCad__framework_mobo_lefthalf | 0 -> 0 | 0 -> 0 | 2! -> 0 | 2 -> 3 |
| ISS-PCB__MIDAS-MK1.1__MIDAS-MK1.1-revA | 0 -> 1! | 1! -> 0 | 1! -> 1! | 1 -> 1 |
| OpenFC-Lite__OpenFC | 0 -> 0 | 0 -> 0 | 0 -> 0 | 3 -> 3 |
| OSHW-reCamera-Series__reCamera_S101_v1.1 | 0 -> 0 | 0 -> 1! | 0 -> 0 | 3 -> 2 |
| spell_tome__spell_tome_bottom | 0 -> 0 | 0 -> 0 | 0 -> 0 | 3 -> 3 |
| eternal-keypad__eternal-keypad | 0 -> 0 | 0 -> 0 | 0 -> 0 | 3 -> 3 |
| urchin__main | 0 -> 0 | 0 -> 0 | 0 -> 0 | 3 -> 3 |
| CharlieBoard__Blue_Line | 0 -> 0 | 0 -> 0 | 0 -> 0 | 3 -> 3 |
(`!`: not a strict pass.)

- **The short tier does not move**: 9 unconnected items on both sides, 28
  against 27 strict passes (main's own seeds: 10, 10 and 8), 25 against 27
  passes.
- **The sweep subset is not a win.** Over the 74 board-seeds with a layout on both sides the
  unconnected items go from 2171 to 2281 (+5 %; 775 to 864, 682 to 702, 714
  to 715 by seed) and the strict passes stay at 33; the first routes leave
  2840 against 3099 open terminals.
- **One board loses its layout**: protoesp at seed 201 ends `incomplete`, one
  part (REF**) without a legal place, where main leaves 2
  unconnected. Rerun alone on both sides at a lower load (the first run's
  guards were stretched to their cap): the same on both (main 2 unconnected,
  7 open after the first route, 1432.2 s of work, to the digit; the branch
  `incomplete` again). So it is the change, not the load.
- **MegaDrive carries the loss**: 54 to 126, 18 to 54 and 35 to 79, worse at
  every seed, though its anneal now ends without overlap and its legaliser
  moves a part 0.11 mm on the mean. Without it the other 71 board-seeds
  go from 2064 to 2022 (-2 %). Glasgow (81 to 68, 79 to 63, 72 to 56) and
  SNSP-CPU-01 (8 to 3, 4 and 4) are better at every seed; the 4in1 is
  worse at every seed (81 to 96, 54 to 90, 77 to 81); the others move both
  ways.
- So taking most of the legaliser's blind work away changes which boards
  route better, not how many connections stay open.

## What is left

- **The bet as stated is spent.** On our own placements the active parts
  do move after the anneal, by the legaliser and then further by the
  refinement, but holding them still does not route better, and taking
  most of the legaliser's work away does not either. What the designer's
  arrangement has, our search does not reach; it is not something our
  later stages wear off an arrangement we had.
- **The refinement earns its place.** With it gone for the active parts
  the layouts are worse at every seed that ran. Its sum differs from the
  anneal's (every counted net's crossings, escape demand), and it turns
  half the skeleton's parts: whether the anneal should count what the
  refinement counts was not tested here.
- **The anneal still ends in overlap where an overlap is cheaper than the
  wire to the room** (USB_Keypad 39 mm2 with the board's reach): the
  penalty tops out at 400 mm of wire a mm2, and a sliver of overlap costs
  less than a part's exile. The legaliser then pays the exile anyway. A
  penalty that ends hard, or the legaliser's cost judged with the nets,
  would close that; the reach's routed result says not to expect
  connections from it.
- **MegaDrive under the anneal's reach** is worse at all three seeds
  though its anneal ends without overlap and its legaliser barely moves a
  part. Not diagnosed. It is the one board that carries the reach's loss.
- **Conflicts the anneal does not see** (the far side of through-hole
  parts: 1337's connector under the key switches' pins, 41 mm) are moved by
  the legaliser alone. docs/placer.md records that far-side gaps in the
  anneal were tried in October and lost on video and the 4in1.
- **A looser level keeps what a stricter one seated**: MIDAS's seeds that
  walk levels 0 to 4 keep the exile of a level whose halos the board
  cannot afford, where the seeds that jump to level 5 anneal afresh. The
  objective ends alike; the routed difference was not measured.
