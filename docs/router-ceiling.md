# The router's ceiling: the designer tier's 41 failures (2026-11-06)

The matched designer tier routes every board of tasks v7 on the
designer's own placement, at the layout's final-ladder share of the
budget. It has passed 68 of 109 boards under the strict rule for ten
nights while the layout sweep went from 57 to 63: 41 boards fail on a
placement a human routed completely. This census says, per board, what
keeps it from the pass, groups the causes into classes and ranks them,
separates our router's defects from the budget and from the input, and
designs a structural fix for the two largest classes of ours.

**In one paragraph.** Of the 41 boards, 10 cannot pass for anyone: the
designer's own board leaves the same unconnected item (the five
PolyKybd molecule panels, OpenRX's panel, katia's and woagboard's GND
pieces, SUMEC's swapped R12/C19 and reCamera's MIC1). 4 fail on a
finding alone, and a plain bug is behind each (the solid step runs once:
k30-SBC, ghoul; a copper text lowered from its render cache while KiCad
draws a stand-in font: both Quanta75 boards). 5 are the budget's: they
pass with more work (video and OpenAirScope at twice the tier's work,
koeg-board, the corne and BAGEL at three times). The other 22 fail on
open connections of ours: **congestion** decides 13 (the kept attempt's
negotiation stalls with a few nets that every single-net exchange only
trades for another), **pad access** 5 (a pad with no legal start: a
custom pad's anchor, a large pad enclosed by a ring, an aQFN's inner pad
narrower than the class, an antenna's copper), **fine-pitch rows** 4 (a
pad of a 0.4 to 0.5 mm row whose ways out are closed, crossed or exactly
as wide as the rules; the rows are also the second class of 11 boards,
three of them the budget's). The kept attempt stops by itself (stalled or
conflict-free) on 24 of the 27 boards with open items of ours and uses a
median 15 % of the ladder's work, while 28 % of the work goes to rungs
that end with far more open; every pass the extra work bought came from
another try of the best rung's kind. The designs: **the ladder's work to
the leader, and the residue renegotiated in a window** (congestion and
the budget; the second part built, measured and removed on 2026-11-07) and **prepared fan-outs for fine-pitch rows, the BGA method**
(rows; pad access is five different footprints, two of them plain bugs
with their own fixes). Estimated: design 1 up to 7 passes,
realistically 3; design 2 up to 5, realistically 2; the plain bugs 4
(2 checked by hand on the night's boards).

## What was measured

- **The rows.** The nightly of 2026-11-06 (main 66115ba, binary
  `pcb-maker-nightly-2026-11-06-66115bad368e`), its matched designer tier
  (`build/nightly/2026-11-06/designer.log`: route mode on the designer's
  own placement at `--budget-scale 0.647`, 970.5 s of work, 8 x 1, rates
  pinned): 68 of 109 pass the strict rule, 71 the per-type rule; the 41
  that fail are this census.
- **Per board, read from the night** (no routing): KiCad's unconnected
  items of our result (`route/drc.json`); the strict rule's new findings
  (the row); the ladder's ledger and the router's diagnostics
  (`board-router.json`: attempts, `skipped`, dead pads, conflicted nets,
  hot spots, starved thermals); `inspect-routing-run <run>/route
  --reference <the designer's project>` (whether the designer's route
  joins each open net on this very placement, `reference.connected`);
  KiCad's DRC of the designer's own board (`drc-kicad-board` on the
  designer's project: its unconnected items and findings); pcbnew on both
  boards (each open pad's footprint, its part's copper pads, the pitch
  to the nearest pad of another net, and whether the designer put a via
  of the pad's net in it).
- **The WHY route.** The 31 boards with open items not all the input's
  routed again at the tier's budget (scale 0.647, 3 x 1, the night's
  binary) with `PCB_ROUTER_TROUBLE_WHY=1 PCB_ROUTER_PAIRS_WHY=1
  PCB_ROUTER_CORRIDOR_CENSUS=1` (`build/ceil1-why.log`): all 31 rows equal
  the night's in unconnected items, vias, length and strict findings, so
  the diagnostics describe the night's boards. From each route.log: the
  crossing census of the kept attempt (its block of the log: the lines up
  to that attempt's own `attempt ...` line, in the ledger's order), the
  trouble repair's refusals per open net, the corridor census on the kept
  attempt's lattice.
- **The budget test.** The 27 boards whose open items are ours, routed
  again at `--budget-scale 1.3` (1950 s of work, twice the tier's;
  `build/ceil1-budget13.log`, link in `build/ceil1-budget13-link.log`),
  and the ten the ledger calls budget-bound (rungs or steps not run for
  want of work, or the kept attempt stopped at its work limit: jetson,
  OpenAirScope, video, link, koeg-board, the laptop motherboard, the
  corne, BAGEL, SNSP-CPU-01, MegaDrive; video and OpenAirScope passed at
  1.3 and were stopped) again at 2.0 (3000 s, 3.1 times;
  `build/ceil1-budget20.log`, `build/ceil1-budget20-link.log`). Same
  binary, 3 x 1 (2 x 1 and 1 x 1 at 2.0), measured `--expected-gb`
  (the night's peaks).
- **The census** (`experiments/router-ceiling/census.py`, its tests in
  `test_census.py`, the hand notes in `notes.json`). Reproduce:

```sh
N=build/nightly/2026-11-06
python3 experiments/router-ceiling/census.py pads $N/designer.log $N/designer --output pads.json
# per failing board: pcb-maker inspect-routing-run $N/designer/<task>/route \
#   --reference <task's directory> --output inspect/<task>.json
# and pcb-maker drc-kicad-board <task's directory> <board id> designer-drc/<task>.json
python3 experiments/router-ceiling/census.py tier $N/designer.log $N/designer --pads pads.json \
  --inspect inspect --designer-drc designer-drc --why build/ceil1-why \
  --notes experiments/router-ceiling/notes.json \
  --budget 1.3=build/ceil1-budget13.log --budget 1.3=build/ceil1-budget13-link.log --budget 2.0=build/ceil1-budget20.log --budget 2.0=build/ceil1-budget20-link.log --table
```

## One cause per open item

Each of KiCad's unconnected items on our result gets the first cause
that holds (census.py's docstring has the rules in full):

1. **input's**: the designer's own board has the same item (same net and
   pads; for a pour's pieces one zone at the same place);
2. **pour island**: an item is a zone (KiCad's fill joins no copper
   between a pad and the pour, or two pieces of it);
3. **via in pad**: a pad of the item holds a via of its net on the
   designer's board and the router may put none there (the part is not
   admitted, or no via size the project's rules allow fits);
4. **walled pad**: the pad has no legal start: among the router's dead
   pads, its escape pocket closed on the kept attempt's lattice with no
   corridor stub out (the corridor census), or its net unreachable;
5. **fine-pitch row**: a pad of a part of four copper pads or more with
   another net's pad closer than 0.65 mm (the fan-out's `FINE_PITCH`),
   and the crossing census of the kept attempt says the row closes it:
   *a crossing pair* (its neighbour's target crosses its own), *every
   straight way closed*, or *no straight way out*;
6. **congestion**: the rest: ordinary pads, or a fine-pitch pad whose
   straight way out the census finds open; the trouble repair's refusal
   for the net is the detail.

A board's class is the input's where any of its items is (nobody passes
it), else the cause of most of its items, else (nothing open) its strict
findings' type; the second class is the next. A board of ours that
passes at a tested budget is **the budget's** (its census class becomes
the second). `notes.json` overrides a class where the rule is too coarse
(marked * in the table): the four finding boards' bugs, and the
keyboard's aQFN pads as pad access. "Pad access" below is walled pad and
via in pad together.

## The 41 boards

| Board | Open (ours / designer) | Open by cause | Strict findings | Class | Second | Input's or ours | Ladder | Budget scale 1.3 | Budget scale 2.0 | Note |
| --- | --- | --- | --- | --- | --- | --- | --- | --- | --- | --- |
| Quanta75_BareRP2040_JLCPCBAoptimized | 0 / 1 | - | clearance 2, solder_mask_bridge 5 | copper text extent* | - | ours | the narrow-signal step: stalled, 229 s; short of work: finer lattice | - | - | every new finding is our I2C1-SDA track inside the '0bsilab' knockout plate as KiCad draws it (stand-in font, 4 mm past the cached glyphs) |
| Quanta75_RP2040Stamp_JLCPCBAoptimized | 0 / 2 | - | clearance 2, shorting_items 2, solder_mask_bridge 1 | copper text extent* | - | ours | pours=connect: conflict_free, 287 s | - | - | the same plate (shorts, bridge) and 'Quanta75' in Urbanist SemiBold 0.105 mm from Row6 |
| k30-SBC | 0 / 0 | - | starved_thermal 1 | starved thermal | - | ours | continued pours=tracks: conflict_free, 303 s | - | - | J1.39 starved only after the solid step's one round; one more round: clean (checked by hand) |
| ghoul | 0 / 1 | - | starved_thermal 1 | starved thermal | - | ours | pours=connect: conflict_free, 249 s | - | - | RN3.3 starved only after the solid step's one round; three more rounds (RN3.1, RN3.2): clean (checked by hand) |
| eurorack-pmod-pcb | 1 / 0 | fine-pitch row 1 | - | fine-pitch row | - | ours | the best rung's seed round from seed 1 (4 seeds): conflict_free, 213 s; short of work: finer lattice | 1 open | - |  |
| WLED_Matrix | 1 / 3 | walled pad 1 | - | walled pad | - | ours | pours=tracks pitch=[0.075]: conflict_free, 14 s | 1 open | - | LDO1.2 (SOT-23) has no node; what walls it is not shown by the diagnostics |
| protoesp | 1 / 0 | congestion 1 | - | congestion | - | ours | pours=tracks: stalled, 28 s | 1 open | - |  |
| OpenAirScope | 1 / 0 | congestion 1 | - | the budget | congestion | the budget | pours=connect pitch=[0.075]: stalled, 90 s; short of work: rungs | **pass** | - |  |
| woagboard | 1 / 1 | input's 1 | - | input's | - | input's | pours=connect: stalled, 141 s; short of work: finer lattice | - | - | an F.Cu GND zone piece the designer's board leaves apart too (two GND zones on F.Cu intersect) |
| video | 1 / 0 | fine-pitch row 1 | - | the budget | fine-pitch row | the budget | continued pours=tracks: stalled, 158 s; short of work: rungs | **pass** | - |  |
| DSP-ADAU1452 | 1 / 0 | fine-pitch row 1 | - | fine-pitch row | - | ours | the narrow-signal step: stalled, 31 s; short of work: finer lattice | 1 open | - |  |
| link | 1 / 6 | congestion 1 | - | congestion | - | ours | continued pours=tracks: conflict_free, 485 s; short of work: rungs | 1 open | 1 open |  |
| poly_kybd_split72_right | 1 / 0 | congestion 1 | - | congestion | - | ours | continued pours=connect (plane stubs): stalled, 263 s; short of work: finer lattice | 1 open | - |  |
| module | 1 / 0 | congestion 1 | items_not_allowed 1, track_dangling 1 | congestion | dangling track | ours | the narrow-signal step: stalled, 78 s; short of work: finer lattice | 1 open, 2 f. | - | ddr-dm1 under the SoC; findings: a GND track in a keepout and its dangling end |
| jetson-nano-baseboard | 2 / 1 | congestion 2 | - | congestion | - | ours | continued pours=tracks: stalled, 139 s; short of work: rungs | 2 open | 2 open |  |
| keyboard | 2 / 0 | congestion 2 | - | walled pad* | - | ours | the narrow-signal step: stalled, 129 s; short of work: finer lattice | 2 open | - | U2 aQFN-73 inner pads: VSS_PA (F23) and RESET (AC9), whose way out is narrower than the class (the designer necks down to 0.127 mm) |
| osprey_rev_a | 2 / 0 | congestion 1, walled pad 1 | - | walled pad | congestion | ours | pours=tracks: stalled, 34 s; short of work: finer lattice | 2 open | - | AE1.1, the antenna's feed, inside the footprint's own copper graphic; c2 at the aQFN |
| koeg-board-pcb | 2 / - | congestion 2 | - | the budget | congestion | the budget | pours=tracks: stalled, 104 s; short of work: rungs | 1 open | **pass** |  |
| BAGEL-MK1.1-Avocado | 2 / 0 | fine-pitch row 2 | - | the budget | fine-pitch row | the budget | pours=tracks pitch=[0.075]: negotiation_work_limit, 127 s; short of work: finer lattice | 1 open | **pass** |  |
| poly_corne_split42_right | 2 / 0 | fine-pitch row 2 | - | the budget | fine-pitch row | the budget | continued pours=connect (plane stubs): stalled, 540 s; short of work: rungs | 1 open | **pass** |  |
| Batch_1 | 3 / 12 | congestion 3 | - | congestion | - | ours | pours=tracks pitch=[0.075]: stalled, 83 s; short of work: finer lattice | 3 open | - |  |
| reCamera_S101_v1.1 | 4 / 3 | walled pad 2, pour island 1, input's 1 | - | input's | walled pad | input's | pours=tracks pitch=[0.05]: conflict_free, 10 s | - | - | MIC1's GND pads inside its own copper ring; the designer's board leaves 3 of them |
| SUMEC_MK_IV | 6 / 10 | input's 4, walled pad 2 | - | input's | walled pad | input's | the best rung's seed round from seed 5 (4 seeds): conflict_free, 114 s | - | - | input's: R12/C19 swapped GND/SHLD as on the designer's board; IC2.11/IC3.11: a 0.45 mm pad no track of the High Power class (2.0 mm) enters |
| vaio_re | 11 / 0 | fine-pitch row 9, congestion 2 | - | fine-pitch row | congestion | ours | the narrow-signal step: stalled, 116 s; short of work: finer lattice | 11 open | - |  |
| Hub | 12 / 2 | walled pad 9, pour island 1, congestion 1, fine-pitch row 1 | - | walled pad | pour island | ours | the narrow-signal step: stalled, 135 s; short of work: finer lattice | 12 open | - | 9 of 12: the X2SON-4 regulators' corner pads (U2, U3, U6), custom pads joined at a 0.148 mm anchor that lies within the class clearance of the centre pad: no legal node in the contact |
| motherboard | 15 / 0 | congestion 6, walled pad 5, fine-pitch row 2, via in pad 2 | - | congestion | walled pad | ours | continued pours=tracks: stalled, 244 s; short of work: rungs | 15 open | 11 open | WLCSP-9 U7/U13: the middle balls need a via in the pad and the designer's 0.4/0.3 via breaks the board's annular minimum (the designer's board has annular_width 199); the edge balls of power nets have no node |
| katia | 15 / 1 | walled pad 10, fine-pitch row 4, input's 1 | - | input's | walled pad | input's | continued pours=tracks: stalled, 186 s; short of work: finer lattice, steps | - | - | input's: the halves' GND (the designer's too). Ours: USB_D's 0.4 mm clearance walls J7/J8's 0.5 mm row, a clearance the project rates a warning; the minimum-clearance step did not run (work) |
| 4in1 | 15 / 0 | congestion 14, fine-pitch row 1 | - | congestion | fine-pitch row | ours | continued pours=tracks: stalled, 213 s; short of work: finer lattice | 11 open | - |  |
| Sisu_SSE-9 | 18 / 0 | via in pad 17, fine-pitch row 1 | skew_out_of_range 5 | via in pad | fine-pitch row | ours | continued pours=tracks: stalled, 145 s; short of work: finer lattice | 18 open, 5 f. | - | 16 of 18: dome-switch centre pads (4 mm) inside a 6.2 mm ring of another net, the designer's via in each; route mode admits only small pads. Also skew 5 (a USB skew rule over 5 nets) |
| revC3 | 18 / 0 | congestion 14, fine-pitch row 4 | - | congestion | fine-pitch row | ours | continued pours=tracks: stalled, 266 s; short of work: finer lattice | 18 open | - |  |
| 4in1-panel | 34 / 0 | congestion 23, fine-pitch row 9, pour island 2 | - | congestion | fine-pitch row | ours | continued pours=tracks: stalled, 345 s; short of work: finer lattice | 34 open | - |  |
| RP2040_base | 36 / 0 | fine-pitch row 27, walled pad 8, congestion 1 | - | fine-pitch row | walled pad | ours | continued pours=tracks: stalled, 250 s; short of work: finer lattice | 35 open | - | two RP2040 QFN-56 at 0.2/0.2 in a 0.4 mm row (an exact fit): 8 pads' escape pockets closed on the 0.1 mm lattice (no margin of zero), the rest crossing or closed ways |
| 4in1-mini | 37 / 0 | congestion 26, fine-pitch row 8, pour island 2, via in pad 1 | - | congestion | fine-pitch row | ours | continued pours=tracks: stalled, 282 s; short of work: finer lattice | 31 open | - |  |
| SNSP-CPU-01 | 44 / 2 | congestion 44 | - | congestion | - | ours | pours=tracks: negotiation_work_limit, 228 s; short of work: finer lattice | 3 open | 3 open |  |
| MegaDrive | 71 / 3 | congestion 71 | - | congestion | - | ours | pours=tracks: negotiation_work_limit, 267 s; short of work: finer lattice | 53 open | 51 open |  |
| OpenRX-panel-rev2 | 78 / 2 | fine-pitch row 48, congestion 17, pour island 11, input's 2 | - | input's | fine-pitch row | input's | continued pours=tracks: stalled, 333 s; short of work: finer lattice | - | - |  |
| poly_kb_molecule_5x2_shifted | 346 / 118 | congestion 163, fine-pitch row 121, input's 62 | - | input's | congestion | input's | the best rung's seed round from seed 1 (1 seeds): stalled, 167 s; short of work: finer lattice | - | - |  |
| poly_kb_molecule_5x4_wave | 474 / 472 | congestion 219, fine-pitch row 198, input's 57 | hole_clearance 1, solder_mask_bridge 1 | input's | congestion | input's | pours=tracks: negotiation_work_limit, 299 s; short of work: finer lattice | - | - |  |
| poly_kb_molecule_4x4 | 499 / 208 | congestion 245, fine-pitch row 217, input's 37 | - | input's | congestion | input's | pours=tracks: negotiation_work_limit, 271 s; short of work: finer lattice | - | - |  |
| poly_kb_molecule_4x5 | 499 / 266 | fine-pitch row 264, congestion 149, input's 83, pour island 3 | - | input's | fine-pitch row | input's | pours=tracks: negotiation_work_limit, 252 s; short of work: finer lattice | - | - |  |
| poly_kb_molecule_7x5_wave_left | 499 / 499 | fine-pitch row 240, congestion 228, input's 31 | - | input's | fine-pitch row | input's | pours=tracks: negotiation_work_limit, 279 s; short of work: rungs | - | - |  |

Ladder: the kept attempt, how its negotiation stopped and its work;
"short of work" lists what the budget left out (rungs or steps not run
for want of work, the finer lattice over `refine_budget_seconds`).
Budget: the board at that scale (`**pass**`, or KiCad's open items and
strict findings).

**Worse than the night before** (four of the 41 between the nightlies
of 2026-11-05 and 2026-11-06, different binaries and budget scales,
0.673 and 0.647): poly_corne_split42_right 1 -> 2 and glasgow revC3 15
-> 18 kept the narrow-signal step on 2026-11-05 and not on 2026-11-06:
since x1923 the step narrows only the Default class (the bisect of
2026-11-06, router.md: the old step wrote 1 A power nets and 0.25 mm
impedance classes at the menu's width), so their loss is a correctness
win, not a router defect, and their items here are what the rules allow.
SNSP-CPU-01 35 -> 44 and poly_kb_molecule_5x2_shifted 319 -> 346 kept
the same rung both nights, which stopped at its work limit: the smaller
budget scale (SNSP-CPU-01 is a budget board, below). Sisu improved, 21
-> 18.

## The classes, ranked

| Class | Decides (first) | Its only class | Second on | Open items (boards of ours and the budget's) | Whose |
| --- | ---: | ---: | ---: | ---: | --- |
| congestion | 13 | 7 | 7 | 217 on 20 boards | ours |
| the input's | 10 | 1 | 0 | 279 on 10 boards (with the molecules' own) | the input's |
| the budget | 5 | 0 | 0 | (their items are counted in their second class) | the budget |
| pad access (walled pad, via in pad) | 5 | 2 | 5 | 44 on 7 boards | ours |
| fine-pitch row | 4 | 2 | 11 | 69 on 14 boards | ours |
| starved thermal | 2 | 2 | 0 | - | ours (bug 1) |
| copper text extent | 2 | 2 | 0 | - | ours (bug 2) |
| pour island | 0 | - | 1 (Hub) | 5 on 3 boards | ours |
| dangling track, keepout | 0 | - | 1 (module) | - | ours |

"Decides": the boards whose class it is; "its only class": of those,
the boards where nothing else is open or found (its fix alone passes
them); "second on": the boards where it is the second class. The
budget's five have their census class as their second: video, the
corne and BAGEL fine-pitch rows, koeg-board and OpenAirScope congestion.

**The input's (10 boards).** The five PolyKybd molecule panels
(duplicated references across tiles; the designer's own board leaves
118 to 499 items: "Not targets" in handover.md), OpenRX's panel (two
GND fill pieces the designer leaves apart; our other 76 items are ours
and second), katia (the halves' GND: the designer's item; our other 14:
USB-C pads walled by a class clearance, below), woagboard (an F.Cu GND
zone piece left apart on both boards), SUMEC (R12 and C19 on swapped GND
and SHLD nets, as on the designer's board; IC2.11 and IC3.11, 0.45 mm
pads the 2.0 mm High Power class does not enter: the board's rule) and
reCamera (MIC1's GND pads inside the footprint's own copper ring; the
designer's board leaves three). Nothing the router does passes them;
under a rule that discounted the designer's own items, katia, OpenRX
and SUMEC would fall to the second classes.

**Congestion (13 boards; ours).** 217 items on 20 of the 27 boards of ours.
The repair's last word on them: refused, the exchange closes as many as
it opens ("1 open to 1, 1 nets open to 1"), 134; undone early (the
victims could no longer let it pass) 30; more victims than the limit
(12, or twice the open terminals) 30, all on MegaDrive; the priced
search closes nothing 7; kept 4. The designer joins 196 of the 217 on
this placement. The fine-pitch pads among them have a straight way out
(51). Their designer paths: of the 199 open nets of the congestion and
row classes on the 25 boards with at most 80 items, 114 have the
designer's track crossed by our copper of a net the designer pours (the
kept attempt routes the pours as tracks on 16 of these 25 boards).
The kept attempt stopped by itself (stalled or conflict-free) on 24 of
the 27 boards and used a median 15 % of the ladder's work; MegaDrive,
SNSP-CPU-01 and BAGEL stopped at the negotiation's work limit.

**Fine-pitch rows (4 boards; ours).** 69 items on 14 of the 27 boards: no
straight way out 32 (the 0.4 mm rows at 0.2/0.2, an exact fit: RP2040's
QFN-56 on RP2040_base, the corne's FH34 connector, BAGEL's VQFN-HR and
UQFN, the DSP's LFCSP-72), a crossing pair 19 (video's TQFP-100, the
corne's RP2040, vaio_re's QFN-56, katia's), every straight way closed 18
(vaio_re, eurorack's resistor array, the 4in1's QFN-24). RP2040_base
adds 8 pads whose escape pocket the 0.1 mm lattice closes with no way
out off it (the corridor census; "a last margin of zero ... not built",
router.md "Corridors of pocketed pads"). The rows were found "not short
of exits" on the layout's placements (router.md, "Prepared fan-outs for
BGAs"); on the designer's placements of these seven they are what is
left.

**Pad access (5 boards; ours).** Hub: 9 of 12 items are the corner pads
of three X2SON-4 regulators, custom pads joined at a 0.148 mm anchor
that lies inside the class clearance of the centre pad, so no node of
the contact is legal (the designer's tracks touch the anchor's far
side). Sisu: 16 of 18 are dome-switch centre pads, a 4 mm pad inside a
6.2 mm ring pad of another net; the designer leaves each by a via in it,
and route mode admits only parts whose small SMD pads hold one (also 5
skew findings). The laptop keyboard: two aQFN-73 inner pads whose way out
is narrower than the class (the designer necks to 0.127 mm). osprey:
the antenna's feed pad inside the footprint's copper graphic. WLED_Matrix:
LDO1.2 (SOT-23) has no node, and the diagnostics do not say what walls
it. Pad access is also the second class of katia (USB_D's 0.4 mm
clearance walls its own 0.5 mm USB-C row, a clearance the project rates
a warning; the minimum-clearance step that exists for this did not run
for want of work), the laptop motherboard (WLCSP-9 balls: the designer's
0.4/0.3 via in the middle ball breaks the board's annular minimum, its
own board has annular_width 199) and RP2040_base.

**Status of pad access (2026-11-07, branch pad-access; router.md "What
walls a pad").** The eight boards were diagnosed again on main 69c35f67,
and the census's reading changes:
- Hub's and Sisu's pads connect since the fixes of 2026-11-06. Hub's
  last two items are GND zone pieces that the designer's board leaves
  open too (the input's); Sisu's last item is congestion.
- The keyboard's RESET pads have no legal exit at all; the designer's
  via breaks the 0.127 mm clearance by 0.04 to 0.05 mm (the input's).
  F23 is not walled: two necked tracks share its channel, and the 0.125
  mm lattice misses the band they need by 4 um. That is the lattice's
  problem, not the neck's.
- osprey's feed pad touches the antenna's no-net polygon everywhere (the
  input's: any track shorts it, as the designer's does).
- WLED's LDO1 pads lie under a second footprint of the same reference
  without nets (ours).
- katia's USB-C pads are walled by USB_D's warning-rated 0.4 mm
  clearance. The board is the input's by its GND.
- The motherboard's WLCSP middle balls have no legal via (the input's).
  Its edge balls were walled by a dru rule on plated pads that the router
  held for SMD pads too (ours: fixed).
- RP2040_base's eight pads are the rows' exact fit (design 2).

What the branch takes:
- **Dead pads say what walls them** (`dead_pads[].wall`).
- **Plated pads are apart from SMD pads in the rules.** On the designer
  tier: **75 -> 76 strict** (the laptop power board), 2723 -> 2709
  unconnected, the motherboard 15 -> 2. No other board of the tiers
  changes.

What it does not take:
- **A plain pad leaving from the free part of its copper** (branch
  pad-access-starts). WLED 2 -> 0 open but strict-failed by a pour neck,
  75 -> 75 strict on the tier: measured, not taken.
- **The minimum clearance where a warning-rated clearance walls pads.**
  Only katia has such a pad, and katia is the input's.

**Starved thermals and copper texts (2 + 2 boards; ours, bugs).** Below.

## The budget test

At twice the tier's work (scale 1.3, 1950 s) the 27 boards with open
items of ours: **2 pass** (video, OpenAirScope), 8 improve
(SNSP-CPU-01 44 -> 3, MegaDrive 71 -> 53, the 4in1-mini 37 -> 31, the
4in1 15 -> 11, the corne, koeg-board and BAGEL 2 -> 1, RP2040_base 36 ->
35), 17 are unchanged to the item (vaio_re, split72_right, Glasgow,
Sisu, DSP-ADAU1452, Hub, the laptop motherboard and keyboard, Batch_1,
the module, protoesp, jetson, osprey, eurorack, the 4in1-panel,
WLED_Matrix, link). At three times (scale 2.0, 3000 s) the ten the ledger
calls budget-bound: **3 more pass** (koeg-board, the corne, BAGEL),
SNSP-CPU-01 stays at 3, MegaDrive 51, the motherboard 15 -> 11, link 1,
jetson 2 (video and OpenAirScope, passing at 1.3 already, were not run
again).

What won: video a seed round of its leading rung (466 s), after three
connect rungs of 390 s each ended at 131, 19 and 18 open; OpenAirScope
its 0.075 mm tracks rung with 81 s of work instead of 72; koeg-board and
BAGEL the 0.05 mm tracks rung, the last of 13 and 14 attempts; the
corne a second regular rung (exclusive planes, 652 s) after its 540 s
continuation; SNSP-CPU-01 a seed round of its tracks rung, which at the
tier's scale stopped at its work limit with 94 open. So the budget
decides 5 boards and moves 3 more (SNSP-CPU-01, MegaDrive and the
OpenESC pair) a long way, and 17 of 27 do not move at all: on them the
kept attempt stalls, and more work buys more rungs that end where the
first did. The ladder spends its work in a fixed order (every pour mode
at the regular pitch, then the finer pitches, then seeds and steps), so
the passes that more work buys come late; design 1's first part
reorders it.

## Plain bugs found on the way (no code changed)

**Status (2026-11-06, branch ceiling-bugs).** All four fixed; router.md
has each in its section ("The solid step in rounds", "A cached
(TrueType) text on copper", "Large pads inside a ring", "A custom pad's
contact in a neighbour's clearance").

Route mode on the designer's placement at the tier's budget (scale
0.647, 970.5 s of work, rates pinned), main 406dcb94 against the branch
with all four fixes, one run each (`build/bugs1/designer-base.log`,
`designer-fin.log`):

| Bug | Board | main: open / strict findings | fixed: open / strict findings |
| --- | --- | --- | --- |
| 1, solid step in rounds | k30-SBC | 0 / starved_thermal 1 | 0 / none: **strict pass** (2 rounds) |
| 1 | ghoul | 0 / starved_thermal 1 | 0 / none: **strict pass** (3 rounds) |
| 2, copper texts as drawn | Quanta75_BareRP2040 | 0 / clearance 2, solder_mask_bridge 5 | 0 / none: **strict pass** |
| 2 | Quanta75_RP2040Stamp | 0 / clearance 2, shorting_items 2, solder_mask_bridge 1 | 0 / none: **strict pass** |
| 3, vias in large enclosed pads | Sisu_SSE-9 | 18 / skew_out_of_range 5 | 1 / skew_out_of_range 5 |
| 4, a custom pad's contact | Hub | 12 / - | 4 / - |

The whole tier: 68 -> **72** strict passes (71 -> 72 per type), 2756 ->
2731 unconnected, 99 of 109 rows identical, no board worse; the
expansion tier (46 boards, 30 s) identical, the short tier's passes and
unconnected identical, the fingerprints' k30-SBC passes strict and
Sisu's v16 placement 29 -> 7. Two corrections to the census on the way:
ghoul's chain needs a round that is no better (RN3.3 solid starves RN3.1
and RN3.2: 68 findings to 69), so the step goes on while only starved
thermals move and keeps the best board KiCad saw; and Sisu's switches
were not refused for being "large" alone: `enclosed_pads` looked at pad
fields of nine pads or more only, and the designer's vias sit 0.9 to 1.8
mm off the pads' centres (LEDs on the back, the modem's keepouts), so the
via goes off the centre where needed.


1. **The solid step runs once** (board_router/route.rs,
   `solidify_starved_thermals`, called once after the ladder). It sets
   the pads KiCad finds starved to a solid connection, checks once and
   keeps the board when better; a pad starved only on the board after
   that try is not tried (the verdict says so: "J1.39 starved only on the
   board after that try (not tried)"). k30-SBC (J1.39) and ghoul (RN3.3)
   fail the strict rule on that pad alone. Checked by hand on copies of
   the night's result boards (pcbnew sets the pad's zone connection,
   `drc-kicad-board` refills and checks): k30-SBC after one more round
   has no starved thermal and nothing new (hole_clearance 8, the
   designer's); ghoul after three more (RN3.3, then RN3.1 and RN3.2 of
   the same array) has none, nothing open and nothing new. A loop to a
   fixpoint (one KiCad check a round, kept while better, bounded) passes
   both.
2. **A copper text is lowered as its render cache; KiCad draws a
   stand-in font** (board_router/geometry.rs, `copper_graphic_shapes`: a
   knockout text is the box of its cached glyphs grown by the stroke).
   `mask_graphic_shapes` adds the stroke font's box to a cached text on a
   mask layer for this very reason; the copper path does not. Quanta75's
   '0bsilab' knockout plate on F.Cu ("Blrrpix Tweak", not installed):
   the cache's box x 84.8 to 112.8, y 84.2 to 91.0 mm; pcbnew's box of
   the text as KiCad draws it here x 80.7 to 116.8, y 80.7 to 95.6 mm.
   Our I2C1-SDA track at x 80.5 to 81.2 runs inside KiCad's plate:
   clearance 2 and solder_mask_bridge 5, all of Quanta75_BareRP2040's
   strict findings. The Stamp board: the same plate (shorting_items 2,
   clearance 1, bridge 1) and Row6 0.105 mm from the plain 'Quanta75' in
   "Urbanist SemiBold", the same stand-in.
3. **A large enclosed SMD pad gets no via** (board_router/via_in_pad.rs
   and run.py's `with_designer_vias`): Sisu's 16 dome switches above.
   The rule's enclosed-pad test exists (`enclosed_of`); what keeps the 4
   mm pad out is the admission's "small SMD pads". 16 of Sisu's 18 items
   (KEYPAD_BACKLIGHT COL0 to COL2 without any copper).
4. **A custom pad's contact is its anchor, and the anchor may lie in a
   neighbour's clearance** (board_router/lower.rs: `contact` is the
   anchor shape, `parts[0]`): Hub's X2SON-4 corner pads above. KiCad joins
   a custom pad through its anchor too, but a track touching the anchor's
   far side keeps the clearance; the router's terminal nodes and escape
   stubs start inside the anchor's illegal part. 9 of Hub's 12 items.

## Design 1: the ladder's work to the leader, and the residue renegotiated (congestion)

**What the census says.** On the 13 boards congestion decides, the kept
attempt's negotiation stops by itself with a few nets in conflict (the
log's last iterations reroute the same one or two nets at the present
factor's cap: poly_kybd_split72_right's VDD from iteration 37 to 50),
the hard resolve leaves them open, and the trouble repair tries each
open net alone: routed through the others at a price, its victims routed
again hard one after the other. On 164 of the 217 items that exchange
closes as many terminals as it opens or is undone; on 30 it would
displace more nets than allowed. Deeper versions of the same sequential
move were built and measured: ejection chains three deep (the last pass:
closed nothing on the 12 near misses, removed on 2026-11-04), pour nets
as trouble-makers and victims (`PCB_ROUTER_TROUBLE_POURS`: identical on
OpenESC and Glasgow), 24 victims instead of 12 (mixed). The designer
joins 196 of the 217 on this very placement: the room exists.

Meanwhile the ladder spends 28 % of its work on these boards on rungs
that end with far more open than the kept attempt, which used a median
15 %. The budget test shows where the passes come from when the work
is there (scale 1.3): video passes by a seed round of its leading rung
(the probed tracks rung, 2 open, run again from another seed: 0 open, 466
s of work) after three connect rungs of 390 s each ended at 131, 19 and
18 open; OpenAirScope passes by its 0.075 mm tracks rung, the last
rung, which ends at 4 open with 72 s of work at the tier's scale and at
none with 81 s; koeg-board and BAGEL drop to 1 by that finer tracks
rung, SNSP-CPU-01 from 44 to 3 and the two OpenESC boards by 4 and 6 by
seed rounds of their best rung, and the corne to 1 by its narrow-signal
step. Every gain comes from another try of the best rung's kind or more
work for the rung that was already best; none from another pour mode's
rung.

**What changes, part 1: the leader first** (board_router/ladder.rs).
Once the probes have ranked a leader and its continuation leaves
connections open, the ladder's next work goes to the leader's own
variants, in this order: its seed rounds (`seed_rounds`, today run after
all rungs), the leader's pour mode on the finer pitches (today after the
other modes at each pitch), the narrow-signal and minimum-clearance
steps on the best of those. A rung of another pour mode runs only when
its probe left at most twice the leader's unfinished nets (video's
connect probes left 272 to 305 against tracks' 113, so none would run,
and its passing seed round fits in the 658 s they took; on OpenAirScope
all four probes are within one net, so its rungs still run; on Hub,
DSP-ADAU1452, koeg-board
and zpn_devboard, where the 75 s probe misjudged connect against tracks
(open problem 3), the probes are close and the rung still runs). Boards
without probes (two-layer boards with three rungs side by side:
SNSP-CPU-01, MegaDrive) keep their rungs, but a rung that stopped at its
work limit while the others ended worse gets their remaining share
before seeds start.

**What changes, part 2: the residue renegotiated in a window**
(router/residue.rs, new). Each of the leader's variants, after its
repair, renegotiates its residue (the open nets, at most 12) jointly in
a window instead of net by net:

1. *The window*: per open connection the box of its pads and the hot
   spots within 3 mm of them (`diagnostics.hot_spots`), grown by 2 mm
   (doubled on a failed round, at most 8 mm); overlapping boxes merge.
2. *The rip-up*: every net with copper in the window loses its branches
   inside it, signal and pour nets alike (in a pours-as-tracks attempt
   the supplies are tracks, and on 114 of 199 open nets our supply
   tracks cross the designer's path); a branch that crosses the window's
   edge leaves a free end there, a terminal of its net for the round,
   the outside copper fixed. A pour net in a `connect` attempt loses
   only tracks and stubs, never its plane.
3. *The negotiation*: `negotiate_from` over the window's nets, the
   search clipped to the window (the corridor planner's box), history
   reset inside and kept outside, the present factor from its start
   value (not the cap the board-wide negotiation ended at), the open nets
   first, vias anywhere legal. Three or four nets give way at once, each
   by a little: the move a single exchange cannot make.
4. *The judge*: kept when the window's open terminals drop and nothing
   in it is in conflict (the router's count), otherwise the window's
   copper is restored and it grows once. The attempt then goes to KiCad
   as every attempt does (the check gate).

**Code paths.** board_router/ladder.rs (`Ladder::run`'s order:
`probes`, then the leader's `seed_rounds` and finer pitch, then the
gated `rungs`, then `steps`; `rung_share` and `bound_allows_attempt`
over the new order), router/residue.rs called from router/driver.rs
after `repair_trouble`, `negotiate_from` with an optional window (search
clip, history reset). router/resolve.rs and router/trouble.rs stay.

**Work.** The ladder's bound is unchanged (`ladder_budget_seconds` and 5
%); what changes is who spends it: the leader's variants take the
shares the gated rungs had (`rung_share` over the attempts that run),
the reserve for the narrow-signal step (`route_step_reserve`) is taken
first as today, and a gated rung's skip is a line in `skipped` with the
probe counts that gated it. The residue step's searches are search work
of the attempt it changes (same router, lattice and maps), within a
limit of its own (a third of what the attempt has left, at least its
resolve and repair work), shared by its windows in the residue's order.
The ledger gets the residue's windows (nets, box, rounds, open before
and after, work).

**Stage 1** (the router's own protocol). Route mode on the designer tier,
all 109 boards at the tier's scale on one binary, on and off: strict
passes and unconnected items, vias and length; the congestion and
budget boards read one by one (which variant won, which windows closed
what). Guards: the expansion tier (46 boards at 30 s: identical or
better, work within the bound), the short tier, the fingerprints
(changed where a gated rung no longer runs: Castor and k30-SBC must stay
complete). Part 1 first (it moves only work), part 2 on top. Kill
criterion: no strict pass gained over the 13 congestion and 5 budget
boards, or a pass lost elsewhere.

**Status (2026-11-07, branch leader-first).** Stage 1 run (router.md,
"The leader first (2026-11-07)" and "The residue in a window", with the
table per arm). Part 1, the leader first, is on by default: the designer
tier 68 -> **71** strict (video, BAGEL-MK1.1-Avocado, DSP-ADAU1452),
2756 -> 2750 unconnected, no pass lost, 5 % less ladder work on the 54
boards that ran (base main 406dcb94, before the four bug fixes; the two
were not measured together, their gaining boards differ). Part 2, the
residue in a window, is in but off (`residue_window`,
`PCB_ROUTER_RESIDUE=1`): alone it also reaches 71 (video, the corne's
right half, BAGEL, DSP-ADAU1452; split72_left lost), on top of part 1
it adds the corne and loses split72_left, so 71 either way (removed
since: below). Against the
estimate: part 1 gave 3 where 1 was expected (BAGEL and DSP-ADAU1452 by
the leader's seed round and the admitted rungs' larger shares, not at
3000 s); part 2's five congestion boards with at most three items
(jetson, link, split72_right, Batch_1, protoesp) stay open (Batch_1
3 -> 2 unconnected): their
windows' joint negotiations stall at the price cap like the board-wide
one (protoesp's log: 6 nets in conflict from iteration 20 on), and are
restored.

**Part 2 closed (2026-11-07, branch residue-kicad-judged).** The one
follow-up: a board the residue completes is judged as the ladder judges
an attempt (polished only when KiCad confirms it complete; the attempts
before the narrow-signal step's reserve leave that work), and on the five
congestion boards one change to the windows' negotiation (the price
started at a tenth: same open counts, fewer windows kept, the same stall
at the cap; not taken). On the 54 multi-attempt boards, residue off
against on with the leader first in both: 20 -> 20 strict, 2723 -> 2721
unconnected; split72_left no longer lost (the completion had 7 open
after the stitching: not polished), the corne gained, Quanta75's
BareRP2040 lost (its first attempt's windows closed nothing in 314 s and
the tracks rung never ran); 19 % of the ladder's work. Kill criterion
met: the residue code is removed (router.md, "The residue in a window
(2026-11-07: built, measured, removed)"). Design 1 is part 1 alone.

## Design 2: prepared fan-outs for fine-pitch rows (the BGA method)

**What the census says.** 69 items on 14 boards are pads of a 0.4 to
0.5 mm row whose way out the crossing census finds closed in the kept
attempt: no straight way out (32: rows whose rules are an exact fit,
0.2 mm track and clearance at 0.4 mm), a crossing pair (19: the pad's
neighbour's target lies across its own, so one of the two needs a layer
change beyond the row), every straight way closed by a neighbour's via
or track (18). On RP2040_base the 0.1 mm lattice also closes 8 pads'
escape pockets outright. Four boards are decided by it, two by it
alone, and it is the second class of 11 more: of the budget's video,
the corne and BAGEL, which pass only with three times the work (the
0.05 mm lattice on BAGEL, the last of 14 attempts), of the three
OpenESC boards, Glasgow and Sisu, and of three of the input's boards. Two earlier attempts were removed on 2026-11-04: the fan-out step
before the negotiation reserved each pad's way out by a *cost*, so the
negotiation still traded which pad of a crossing pair stays open; the
crossing dive decided the layer change inside the repair, and its via
landed on the other layer's routes. The BGA fan-out (`bga_fanout`, on
since 2026-11-04) is the method that worked: geometry decided before
routing and fixed as copper.

**What changes.** `row_fanout` (board_router/row_fanout.rs, beside
bga_fanout.rs), in the lowering, for every row of a part (the fan-out
plan's rows: four copper pads or more, another net closer than 0.65 mm)
on which the plan finds a pad with at most one straight way or a
crossing pair:

1. *Exits at exact geometry*: each pad's exit is a stub from its contact
   to the first point beyond the neighbours' tips where the net's track
   is legal, computed off the lattice with a margin of zero (as
   `stub_is_clear` fits a stub to the nanometre), and its end snapped to
   a legal node. This is the corridor stub's computation made for every
   pad of the row, so an exact-fit row no longer depends on the
   lattice's phase.
2. *The order*: the row's pads in order along the row against their
   targets' order (the angle of each net's ratsnest target seen from the
   part's centre, the census's `targets_cross` for every pair); the
   longest set of pads whose order matches stays on the surface, the
   others change layer. That is the planar escape problem of a row: a
   minimum number of layer changes makes the rest non-crossing.
3. *The via sites*: each pad that changes layer gets a via at the end of
   its exit, staggered in two rings (the stub of every second pad one
   via pitch longer) so neighbouring vias keep their clearance; a site
   is taken only where the via is legal on every layer it spans on the
   statics and its landing on the target layer has a free way out of at
   least one straight run (the dive's failure, checked before, not
   after). A pad with no legal site stays on the surface and is listed.
4. *Fixed copper*: exits and vias become the nets' first branches, as
   bga_fanout's dog-bones do (fixed, stamped before the negotiation; the
   search starts at their ends). Pour nets' pads keep their own
   handling.

**Code paths.** board_router/row_fanout.rs (the plan, from the router's
`fanout_plan` geometry, which already finds the rows and their straight
ways; the order and the sites new), board_router/lower.rs (the fixed
branches, as for bga_fanout), router/escapes.rs (the exact stub, shared
with corridors.rs), a `row_fanout` router setting (on/off) and
`PCB_ROUTER_ROW_FANOUT_WHY` printing each row's plan. The crossing census
stays as the measure.

**Work.** The plan is part of the lowering: a few hundred geometric
checks per row, modelled as lowering work (`lowering_work_seconds`, with
`PCB_ROUTER_BUILD_TRACE` calibrating the constant), so it counts against
the ladder's bound as the BGA plan does. The negotiation has less to do
at the rows; the ladder's arithmetic is unchanged.

**Stage 1.** Route mode on the designer tier, all 109 boards, on and
off, one binary: strict passes and unconnected items, the four row
boards and the three budget boards whose items are rows read one by one (each row's plan against the crossing census of
the run); the expansion and short tiers and the fingerprints as guards
(Castor, k30-SBC and jetson have fine-pitch rows: their routes may
change; they must not lose). Kill criterion: the census's closed-row
items do not fall, or a board loses a strict pass.

**Status (2026-11-07, branch row-fanout): measured, out.** Built as
designed (router.md, "Prepared fan-outs for fine-pitch rows
(2026-11-07)": exits at exact geometry with the lattice's room, fanned
rows, divers of adjacent crossing pairs through a via whose landing is
checked, fixed copper the terminal reaches). The kill criterion's
second clause holds: boards lose strict passes. The census's closed-row
items do fall (113 -> 61 on the
fifteen boards; vaio_re 11 -> 7, RP2040_base 36 -> 31, Glasgow 18 -> 15)
and video and DSP-ADAU1452 pass, but x1930 (design 1's leader first)
passes those two already, and the boards lose elsewhere: against x1930
on the sixteen planned boards routed both ways, 10 -> 5 strict passes
(hackrf-one, little_red_rover, ATAT1800, split72_left, Quanta75_BareRP2040
lost) and 23 -> 44 unconnected; the whole tier with the plan before its
last writer fixes 64 strict. The fixed exits and the divers' vias take
room the negotiation used (jetson's tracks rung 4 -> 24 open). Taken
out; the census refresh (the rows, the designer's own exits) is in
router.md's section.

## Design 3: lattice lines at pin rows (the geometry, not a plan)

**What the evidence says.** Design 2's lesson was that fixed exits take
room the negotiation needs; the rows' remaining failures are geometric:
RP2040_base's two QFN-56 (0.4 mm rows at 0.2/0.2, an exact fit) lie at
phases the one regular lattice cannot both hold (U19 0.021/0.024 mm off
the 0.1 mm lattice, U15 0.035 mm), and the keyboard's F23 needs two
necked tracks 0.254 to 0.369 mm apart that a 0.125 mm lattice misses by
4 um. Moving the two QFNs onto the lattice in the board file (the oracle)
takes RP2040_base from 36 to 4 open.

**What changes** (docs/router.md, "Lattice lines at pin rows
(2026-11-07)"). The lattice's lines become per-axis coordinate tables
(`Grid::warp`), indices and step costs unchanged; a part whose row exits
share a phase gets the lattice at that phase over its rows and 1 mm
around (a band, taken where the band's narrow pads vote for it), other
exits single lines; footprints are measured on the lines where they are
not a pitch apart (exact as before; on the regular lattice the measured
footprints are the offsets cell for cell). The off path is main's code.

**Status (2026-11-07, branch row-lattice): measured; off by default, a
router setting for agents** (`pin_lines`). On the rows: RP2040_base 36 ->
5, vaio_re 11 -> 3, the census's no-straight-way pads 78 -> 44 on the
fourteen row boards. On the designer tier (scale 0.647): 76 -> 77 strict
(OpenAirScope, koeg-board, split72_right gained; Quanta75_BareRP2040 and
split72_left lost), 2709 -> 2679 unconnected; the expansion tier 25 -> 28
strict with none lost; the short tier unchanged; the fingerprints keep
their passes, 20 -> 29 unconnected. The rule's "none lost"
fails, so it is not the default; the channels' lines (F23) measured on
two boards and left out.

**Status (2026-11-08, branch pin-lines-default): on by default where the
rows are open** (router.md, "Lattice lines where the rows are open
(2026-11-08)"). The losses were not the lines' geometry but routes moved
on boards near their limit (taking away the bands a board did not need
lost three passes more); so the ladder turns the lines on only after its
leader's run, when KiCad's open pads lie in parts whose rows the regular
lattice misses. Designer tier against the knockout fix alone: 75 -> 77
strict (OpenAirScope, koeg-board), none lost, 2707 -> 2682 unconnected;
RP2040_base 36 -> 20, vaio_re 11 -> 3. Bug 2 had a second half: the
knockout plate's margin is a ninth of the height and a face's lines sit
higher than the stroke font's (Quanta75's via at the plate's top); fixed
for knockout plates and faces, it costs Quanta75_BareRP2040's pass (1
open at the RP2040's row, its routes moved).

**Status (2026-11-08, branch pin-lines-probes): the rows decided at the
probes** (router.md, "The rows decided at the probes (2026-11-08)").
Where the leader's probe leaves pads unfinished at rows the regular
lattice misses, its rung is probed once more with the lines (the same
work, at most a third of what is left under the bound) and leads at a
tenth fewer unfinished nets; the trigger after the leader's run stays
for the rest. Designer tier: 77 strict, none gained or lost, 2683 ->
2668 unconnected (RP2040_base 20 -> 5, ATAT1800 and OpenAirScope pass
with less work and fewer vias); the expansion tier at 30 s 25 -> 26
(backplane); fingerprints and short tier unchanged.

## The estimate

Per design, by the boards of its class (a board counts where the
design's class holds all its open items and nothing else fails it):

| Design | Boards it could pass (upper bound) | Realistic | Why not more |
| --- | --- | --- | --- |
| 1, part 1: the leader first | video (its passing seed round, 466 s, fits in the 658 s the gated connect rungs took); OpenAirScope only if its last rung gets more work, which the order alone does not give | 1 | koeg-board, the corne and BAGEL pass only at 3000 s, on attempts that do not fit 970 s in any order |
| 1, part 2: the residue in a window | the congestion boards with at most three items, all congestion: jetson (2), link (1), split72_right (1), Batch_1 (3), protoesp (1) | 2 | the same trades may stand jointly (the earlier deeper repairs closed nothing); the module also has its keepout finding |
| 2: prepared row fan-outs | DSP-ADAU1452 and eurorack (row items only), and at the tier's budget video, the corne and BAGEL (their items are rows) | 2 | vaio_re and RP2040_base have congestion and walled pads beside their rows; the earlier fan-out step moved which pad stays open |
| plain bugs 1 and 2 | k30-SBC, ghoul (checked by hand), both Quanta75 (every finding is the plate) | 4 | Sisu (bug 3) also has 5 skew findings and 2 other items; Hub (bug 4) 3 other items |

So the designer tier's 68 strict passes could reach about 71 with design
1, 73 with both and 77 with the four plain bugs fixed too; short of the
input's 10 its ceiling is 99. The numbers are boards whose last items the design
addresses, not measurements: stage 1 of each bet is the judge.
