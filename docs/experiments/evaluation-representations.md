# What should the tool show an agent that evaluates a layout? (2026-11-24)

An experiment, not a feature: candidate representations of a routing or
placement problem were built as lab scripts and measured with blind test
subjects (Opus subagents, one case and one representation each, a fixed
tool budget, no answer). Code and data: `experiments/eval-representations/`
(`whyopen.py`, `placeview.py`, the cases, every subject's prompt under
`subjects/`, every answer under `results/`). Outputs under
`build/scratch/fund-eval/` (not in git).

## Verdict

**For "why is this connection open": a per-item card, a short dossier of
evidence plus one annotated picture of the window, with KiCad as the judge
of any path the tool claims.** 12 of 12 subjects answered right from it in
about 30 s and 3 tool calls. Today's raw outputs also gave 9 of 9, but at
7 tool calls, 120 s and a hand-written pcbnew script per case: the subjects
rebuilt the analysis each time. A dossier alone (6 of 9), a plain picture
(7 of 9) and a text map (7 of 9) each failed in their own way.

**For "which placement routes better": nothing tried works.** Raw boards,
pre-routing measures and pictures gave 5 of 12, a coin flip, and the
measures misled with confidence (1 of 4, wrong at 65 to 85 %). The
ratsnest's length and crossings rank the designer's placement below ours on
three of four boards, as the cut census found for the placer's objective.
No representation fixes a measure that points the wrong way; what the tool
can show honestly is a routed probe.

**The dossier must be built on evidence the reader can check.** The
tool's own conclusion was wrong on 2 of the 12 cases in round 1, and so was
the census's class on 3: the truths below were corrected by KiCad's DRC and
pcbnew. The subjects who found them had the picture or the raw board, not
the verdict.

## Results

### Why is this connection open? (48 subject runs)

Twelve KiCad unconnected items of the window census's boards
(`cases.json`), five answer classes: A a single-net miss (a legal path
exists with all other copper kept), B walled by other nets' routed copper,
C walled by the input (pads, footprint copper, holes, the edge, the class's
width), D a pour island, E not local. Round 1: four representations, each
case in three of them. Round 2: the card, every case.

| representation | runs | correct | mean confidence right / wrong | tokens | tool uses | seconds |
| --- | ---: | ---: | --- | ---: | ---: | ---: |
| raw: the run's directory, `inspect-routing-run`, Python with pcbnew | 9 | 9 | 79 / - | 61k | 7.2 | 120 |
| dossier (JSON, round 1's tool) | 9 | 6 | 77 / 59 | 40k | 2.0 | 27 |
| plain picture (PNG, no analysis) | 9 | 7 | 65 / 68 | 43k | 2.1 | 46 |
| text map (ASCII, no analysis) | 9 | 7 | 71 / 75 | 50k | 2.1 | 50 |
| **card: annotated picture + dossier (iterated tool)** | 12 | 12 | 79 / - | 44k | 3.2 | 31 |

Tokens include about 38k that every subject spends before reading
anything; tool uses include reading the prompt. Per class (A / B / C after
the corrections): raw 3/3, 2/2, 4/4; dossier 2/3, 1/2, 3/4; plain 3/3, 3/3,
1/3; text map 3/3, 2/2, 2/4; card 4/4, 3/3, 5/5.

| case | truth | dossier | text map | plain | raw | card |
| --- | --- | --- | --- | --- | --- | --- |
| a1 DSP-ADAU1452 DVDD | A | A (85) | A (80) | A (65) | | A (85) |
| a2 capybully GND | A | | A (85) | A (70) | A (85) | A (90) |
| a3 WLED +3.3V | A | A (92) | A (85) | | A (75) | A (90) |
| b1 keyboard VSS_PA | B | C (55) x | | B (65) | B (80) | B (78) |
| b2 jetson HDMI_SHIELD | B | B (70) | B (65) | B (55) | | B (85) |
| b3 corne_left K_D/CS | B | | B (65) | B (60) | B (85) | B (85) |
| c1 osprey antenna feed | C | C (75) | A (80) x | | C (80) | C (65) |
| c2 SUMEC +12V | A (census: C) | C (72) x | | A (70) | A (72) | A (75) |
| c3 katia USB-C | C | C (80) | B (70) x | B (75) x | | C (88) |
| c4 USB_Keypad +3.3V | C | | C (70) | C (70) | C (75) | C (75) |
| d1 Hub GND | C (census: D) | E (50) x | C (50) | | C (85) | C (55) |
| e1 keyboard RESET | C (census: E) | C (60) | | E (60) x | C (75) | C (80) |

Answer and stated confidence in per cent; x: wrong.

**Three truths were corrected after round 1**, each by a direct check, not
by a subject's say-so:

- c2: KiCad accepts a 0.4 mm track of 2 mm from IC2.11 to the +12V pad
  C15.2 beside it (unconnected 6 to 5, no new error). The router's neck
  rule wants a node that admits the class's 2.0 mm within 2 mm and never
  tries the same net's copper 0.55 mm away. The census (walled-dead) and
  round 1's dossier (`class_width_does_not_fit`) were both wrong; the
  subjects with the plain picture and with the raw board found it.
- d1: the board file holds three board outlines with a GND zone each
  (pcbnew). No copper can join them. The census's "zone" names the item's
  kind only.
- e1: `route.log` says "3 pads no track leaves, no via allowed (AC9, AC11,
  AC13 (RESET))". The 149 mm are not the cause.

### How each representation failed

- **Dossier alone**: the reader has nothing to check the tool against.
  b1: the tool counted a net tie's copper as an obstacle, its two searches
  contradicted each other, and the subject overrode the verdict with the
  wrong one. c2: the tool searched only between the two points KiCad
  names; the legal joint went to another pad of the net. d1: the search
  found no copper at KiCad's zone positions and reported nulls. Two
  subjects asked for a picture.
- **Plain picture**: clearances are not visible. c3: the class's 0.4 mm
  clearance against 0.2 mm pad gaps makes the pads unreachable; the subject
  saw a track under the pad and blamed it. Every subject estimated gaps
  from pixels and said so; confidence was the lowest (65 %) and did not
  separate right from wrong. Nets beyond the legend's eight ("11 more
  nets") could not be named, twice the blocking one.
- **Text map**: the same, plus a 0.1 mm character grid that cannot show a
  0.13 mm gap. c1: the antenna's copper graphic looked like it touched the
  pad on one side only; wrong at 80 %. It costs the most tokens of the
  compact forms and is the slowest.
- **Raw**: right every time, by writing a pcbnew script: two subjects
  wrote their own grid search with tracks removed (b1, b3), two a
  clearance check of a candidate track (a3, c2), the others dumped the
  footprint's or the board's geometry. That is `whyopen.py` again, per
  case. What they reported as misleading in today's outputs:
  `inspect-routing-run`'s hot spots were elsewhere than the open item (a2,
  c4; right place on b3) and it has no cause or access check; the dead-pad wording ("pocket closed", "no track of their class
  can enter") reads like an input wall where a legal track exists (a3,
  c2); `next_actions` advises `keep_whole` for a pour on a board that is
  three boards (d1); the first lines of `route.log` hold the answer for e1
  and nothing in the JSON reports points there; `board-placer.json` did not
  match the written board on c4 (69 of 98 parts).
- **Card**: no miss. The clearance band settled c3, the other island's
  colour c2, the judged path the A cases. Lowest confidence where the tool
  had no path to hand to KiCad (c1, 65 %) and where the question's classes
  did not fit (d1, 55 %).

### The tool's own classification, on all 117 census items

`whyopen.py --verify` over the window census's 117 items on 42 boards
(about 2 minutes on 4 cores without the judge; with it, two DRC runs more
per board that has a claimed path):

- 11 single-net paths that KiCad accepts (the item closed, no new error
  type), against the census's 7; 6 are the same, 5 are new (Sisu +3.3V with
  a 0.3 mm neck, WLED +3.3V and LDO1-Vin, SUMEC +12V twice with a 0.1 mm
  neck on a 2 mm class, which is legal for KiCad and the designer's call).
  One of the census's 7 it does not find (Sisu's, which moved two other
  nets). 3 raster paths KiCad rejects and 2 too marginal to ask about are
  reported as such.
- 37 walled by routed copper with the nets the cheapest path crosses (the
  census's 19 UNSAT and timeouts are among them, 15 of 19 with the same
  class), 24 walled by fixed objects with their names, 4 where the class's
  width does not fit, 4 on separate board outlines, 5 pour islands.
- 16 items whose ends are far apart (12 of the 22 "long", 3 "zone", 1 "too
  big") have an end that cannot leave its pad on a board stripped of other
  tracks (`end_walled_by_fixed_objects`).
  Only e1 of these was checked against the router's log; the rest are
  candidates, not findings.

An earlier version without the judge claimed more than a dozen single-net
paths on the items the census left undecided (10 of its "too big" alone); every one was the raster's error (a via
overlapping a track, a track inside the net's own pad too close to a
neighbour). The judge is what makes the class A answer worth reading.

### Which placement routes better? (12 subject runs)

Four boards with a large placement gap in the nightly of 2026-11-24
(Glasgow 85 against 17 open, OpenESC 4in1 78 against 12, eurorack-pmod 18
against 1, Batch_1 137 against 2), the designer's placement and ours as P
and Q, unrouted. Chance is 2 of 4.

| representation | correct | confidence right / wrong | tokens | tool uses | seconds |
| --- | ---: | --- | ---: | ---: | ---: |
| raw: the two boards, Python with pcbnew | 2 of 4 | 85, 72 / 88, 70 | 62k | 9.8 | 152 |
| measures (`placeview.py`: ratsnest, demand against room, ring closure) | 1 of 4 | 70 / 72, 85, 65 | 41k | 2.0 | 29 |
| picture (parts, pads, ratsnest, full cells) | 2 of 4 | 80, 70 / 68, 57 | 41k | 3.0 | 22 |

- The measures point at ours: on the 4in1 the designer's ratsnest has 237
  crossings against our 102, on eurorack 738 against 282 and 49 % more
  length, on Batch_1 4003 against 2550. The subjects followed
  them, with pcbnew too (p3 raw: wrong at 88 %).
- The demand map said nothing on three boards (no cell over half full).
- What worked where anything did: "this one looks like a designer's
  layout" (the two right picture answers), pads crowding the BGA and the
  QFN (Glasgow, where our placement walls U1: ring closed 0.44 against
  0.10), and on Batch_1 finding with pcbnew that the board is a panel of
  sub-boards on mouse-bite tabs and measuring per sub-board.
- The picture is legible (a subject names parts and regions from it) and
  cheap; it is not evidence of routability.

## Recommendation

### For agents using the tool

1. **A card per open item in the output directory, named in the verdict.**
   `open-items/<n>.json` and `open-items/<n>.png`, and in `verdict.json` an
   `open_items` list with, per KiCad unconnected item: the net, the ends,
   `cause`, one sentence `because`, the objects in the way, and the two
   paths. The `next:` line for an open connection names the card and what
   the cause calls for, in place of today's hot-spot line. Causes and what
   an agent does with each:
   - `single_net_path_exists` (KiCad accepted it): nothing in the design;
     the tool should have taken it (see developers, 1).
   - `walled_by_routed_copper` with the nets crossed: more room at that
     place, or move the named parts; the nets are named so a constraint
     can.
   - `walled_by_fixed_objects`, `class_width_does_not_fit`: the input.
     Name the pad, the neighbour, the gap and what the class asks
     (`a track needs 1.0 mm between coppers, the pads leave 0.2`): the
     agent changes a rule, a class or a footprint.
   - `separate_board_outlines`: the file holds several boards.
   - `pour_island` with whether a track would join the pieces.
2. **The picture's contract** (what round 2 changed, each from a subject's
   miss): one panel per copper layer, all from the top; the open net
   highlighted, its other island in a second colour; the band where this
   net's track would be too close; the cheapest path with its vias; pad
   labels `ref.pin`; every net near the ends named in the legend, nearest
   first; a scale bar and the window's coordinates. About 1900 x 1700
   pixels for four layers reads well.
3. **Say what was searched and what was not judged.** "Between the island
   of end A and all the net's other copper in the window"; "KiCad was not
   asked: no path to write". The subjects' confidence followed these
   lines.
4. **No placement score in the contract.** Do not print ratsnest length or
   crossings as a quality of a placement: agents believe them and they
   point the wrong way. If a placement is to be judged before a full
   route, the tool should route a short probe and report open connections
   and where; that is a measurement to make, not one made here.

### For tool developers

1. **The single-net retry with KiCad as judge is a repair, not only a
   diagnosis.** 11 of 117 items close with one net's copper added and no
   other net touched; the window census proposed the same from 7. The lab
   script's order of work is the specification: island against the rest of
   the net (not KiCad's two points), the class's width then the neck's, a
   margin on the clearance, write, DRC, keep only what KiCad accepts.
2. **Three model defects the cards exposed, each a small fix with boards
   attached**: the neck rule that wants a full-width node within 2 mm and
   ignores same-net copper beside the pad (SUMEC +12V, both tiers); the
   escape "found and not kept" at a pad that a duplicate or overlapping pad
   crowds (WLED LDO1); pads of one number on two layers taken as one
   terminal (capybully's sockets).
3. **`whyopen` as a lab command over a night**: one table of causes per
   tier replaces reading reports board by board. Today's split of the
   11-24 designer tier and the 11-18 sweep is in
   `build/scratch/fund-eval/all2/causes.json`.
4. **Fix what misled the raw subjects**: hot spots that are not at the open
   item should not lead `next_actions`; the dead-pad text should say
   "KiCad accepts a track here" or "no legal track" once that is known;
   the log's "no track leaves, no via allowed" belongs in the JSON report;
   a board with several outlines should be said to be one.
5. **For placement, measure before showing.** The cheapest decisive check
   is the one the cut census named: which measure ranks the designer's
   placement above ours on the gap boards. Until one does, a placement
   picture is for orientation and a designer-versus-ours overlay is the
   only comparison worth drawing.

### Build first

The card in the layout's output (users 1 to 3) on top of the single-net
retry (developers 1): the retry computes the path and the judgement the
card shows, so one piece of work serves both. In Rust the search belongs on
the router's own stamping, not on a raster; the raster was the cheapest way
to find out what to show.

## Method

**Representations.** `whyopen.py` reads a routed board with KiCad 10's
pcbnew and KiCad's `drc.json`. Per unconnected item it rasterises the
copper in a window (the ends' box plus 2.5 mm, 0.02 mm a pixel), builds the
free space for the net's track (the class's width and clearance, holes, the
edge, keepouts; another net's pour is no obstacle; inside the net's own
copper a track is free only where it lies wholly within it), and runs one
shortest-path search in which crossing another net's routed copper costs
1000 mm a pixel: a path without such a cost is a single-net path, one with
it names the nets crossed, none means fixed objects wall an end, and the
objects along each end's reach are listed. A second search runs at the
router's neck width. `--verify` searches again with the clearance grown by
0.03 mm, writes the path as tracks and through vias into a copy of the
project and compares KiCad's DRC before and after. From the same raster it
writes the dossier, the picture (plain or annotated) and the text map.
`placeview.py` reads a placed board and writes the ratsnest (a spanning
tree per net, pour nets left out) with length and crossings, a 2 mm map of
the ratsnest's demand against room for tracks, the share of the ring around
each part of ten pads or more that other parts' pads close, and a picture.

**Iteration.** After round 1 the tool was changed where subjects' answers
showed a defect: the search goes from the island of one end (KiCad's
connectivity) to all the net's other copper; a net tie's copper belongs to
its nets; ends on different board outlines are a cause; a far end that
cannot leave its pad is one; the legend names the nets nearest the ends;
the other island has its own colour.

**Subjects.** Opus subagents, one per run, given the prompt file and its
material directory only. Budgets: 3 tool calls for a dossier, a picture or
a map, 4 for the card, 20 for raw. Each answered a fixed JSON (cause, what
is in the way, fault, fix, confidence, notes on what decided it and what
misled). 48 runs for the open items, 12 for the placements. Tokens, tool
uses and seconds are the harness's figures.

**Ground truth.** The window census's class per item, and for class A
KiCad's DRC accepting the path. Three keys were corrected as described;
against the census's first keys the card scores 9 of 12 and raw 6 of 9,
which measures the keys, not the representations. For placements, the
nightly's open connections on the two placements.

## Caveats

- **Round 2 is not a held-out test.** The tool was fixed on the cases the
  subjects then saw in the card. The card's 12 of 12 says the fixed tool
  and its picture agree with the truth on these cases and that subjects
  read them correctly; the 117-item run is the only check beyond them, and
  it has the census's classes, not verified truths, to compare with.
- **Small numbers.** Nine to twelve runs per representation, one subject
  per cell, one model. The differences between 6, 7 and 9 of 9 are not
  significant by count; the kinds of failure are the finding.
- **The classes were too coarse.** D and E describe the item, not a cause;
  three subjects looked past them and were right. A placement fault (c4:
  the placer put a capacitor on the panel's breakaway tab) has no class and
  was scored as the input's.
- **The raster is not the router.** No custom `.kicad_dru` rules, no
  per-pad clearances, one clearance for all pairs of nets, through vias
  only, no via in pad, windows of a few millimetres. A path KiCad accepts
  is legal; "walled" is the raster's word, and 16 end-wall candidates are
  unchecked. The dossier's stated path and the judged path can differ (the
  judged one is searched with a margin).
- **Placement ground truth is one night's routes** of one placement each,
  and the question was binary. Whether a routed probe ranks placements was
  not measured.
- **Raw subjects had pcbnew and a 20-call budget**; an agent without KiCad's
  Python has less.

## Reproducing

```
export TMPDIR=/home/flo/programming/pcb-maker/build/scratch/tmp-fund-eval
cd experiments/eval-representations
python3 whyopen.py BOARD.kicad_pcb --drc drc.json --router board-router.json --out DIR --verify
python3 run_all.py ITEMS.json SUMMARY.json OUT --jobs 4 --verify   # the census's items and classes
python3 build_cases.py CASES                                       # the 12 cases, every representation
python3 build_subjects.py CASES SUBJECTS r1                        # or r2: prompts and material
python3 build_placement.py SUBJECTS p1
python3 score.py results/r1.jsonl results/r2.jsonl
```

`ITEMS.json` and `SUMMARY.json` are the window census's
(`build/scratch/fund-window/`). The prompts as the subjects got them are
under `subjects/`; their material paths point into
`build/scratch/fund-eval/subjects/`.
