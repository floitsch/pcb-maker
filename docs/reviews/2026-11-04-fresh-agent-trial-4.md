# A fresh agent's fourth trial — 2026-11-04

One agent that had never used pcb-maker took three boards new to it
through the commands an agent would use, reading only README.md,
skills/pcb-maker/SKILL.md and docs/agents.md (and docs/constraints.md
where agents.md sends the reader), and wrote down every point of
friction. Source code was read only after a board's runs, to fix what
was found. Binaries: the worktree's `target/release/pcb-maker` at main
70b5cab0 for the first place checks of the keyboard, at 72faf9dd (x1883,
panels of copies off frames and tabs) for its later checks and its
layout, and at 9d6844aa (x1884, the placer's length maxima and the far
side of vias in pads) plus this branch's fixes for the other two boards.
KiCad 10.0.7, a shared machine (load 25 to 35 on 32 cores, 16 to 26
other runs in the memory ledger; one layout waited minutes for its
reservation).

## The boards

All three are boards of `benchmarks/real/external`:

| | Goga64 ULK | obsilab Quanta75 (BareRP2040) | oro-os link |
| --- | --- | --- | --- |
| Layers, outline | 2, a 275.5 x 105.6 mm panel: two keyboard halves, a top and a bottom rail, joined by eight mouse-bite rows | 2 (not 4, as the brief said), 320.7 x 158.3 mm | 8, the designer's outline |
| Parts, nets | 92 from the schematic (42 keys, 42 diodes, 2 Pro Micros, 2 reset switches, 2 power switches, 2 headers), 76 nets; the designer's board adds 8 mouse bites, logos and art | 276, 172 | see below |
| What it tests | the netlist path to a panel: sub-boards, frames, tabs | closing the last open connections with the documented levers | a `.kicad_dru` `length` rule (ULPI, 31.4 to 31.6 mm) |
| Started from | the schematic (`github-sch/Goga64__ULK__ULK`), `import-kicad-netlist` | the nightly's task input (tracks stripped, parts stacked, the task's constraints: six fixed keys and J3, three connectors on the top edge) | the nightly's task input |

## Log

Times are wall clock. **F** marks friction: a guess, a retry, or reading
something other than the user documentation.

### Goga64 ULK, from the schematic

1. `import-kicad-netlist .../ULK.kicad_sch ulk ULK` exits 1: 90 lines of
   `missing`, one per part (42 times the same `kbd:D3_SMD_v2` sentence,
   42 times `PG1316:...`). The stderr line names `--footprints-from`; the
   designer's board is at hand. **F** (2 min): the list is not grouped
   by footprint, and `starter.why` says "4 switch-like parts, but no two
   share a row and a column net: no key matrix" while 42 switches are
   simply missing from the board.
2. The same with `--footprints-from <designer's ULK.kicad_pcb>`: exit 0,
   92 footprints, 90 in `footprints_from_board`, and a
   `constraints.starter.json` that found the split: 8 rows x 12 columns,
   42 switches and diodes, two halves (separate matrices) with U1 and U2
   as controllers, the thumb rows `relative` below the grids. Good.
3. The panel. agents.md section 1: copy the designer's `outline.points`
   into `outline.polygon` with `minimum` as `x`, `y`. The designer's
   board also has eight mouse-bite footprints (all named
   `mouse-bite-2mm-slot`), which the schematic lacks:
   `"add": [{"part": "MB1", "from_board": true, "board_reference":
   "mouse-bite-2mm-slot", "x": 19.9075, "y": 83.38, "origin":
   "absolute"}, ...]` (constraints.md's table covers the repeated
   reference exactly), import again: `added` lists the eight.
4. `place-kicad-board ulk ULK place1 ulk/layout.json`: exit 0,
   "complete: every part placed, every constraint kept", 22 s.
   `sub_boards` null. **F** (25 min, the trial's largest): rendered with
   `kicad-cli pcb export svg`, the result is one plate: the 2 mm slots
   between the halves and the rails are gone, U2 and the thumb keys sit
   on what was the bottom rail. `describe-kicad-board`'s `points` is the
   outer loop alone (174 corners); the designer's Edge.Cuts has 327
   items in many loops, and nothing in the description says there are
   holes. Workaround: a script that copies the designer's top-level
   Edge.Cuts graphics into the imported board, and `outline` dropped
   from constraints.json. Not an agent's job.
5. With the real Edge.Cuts, still `sub_boards` null and exit 0. Adding
   `"fixed": ["MB*"]` changes nothing (same wirelength to the
   millimetre). **F** (10 min): nothing says the outline falls into
   pieces at the mouse bites; the SKILL's sentence about panels
   ("whose nets do not tell the boards apart needs `--reference`") is
   what made me try it. `--reference <designer's project>`: `placer:
   sub-boards: 2 (reference board) ... 2 frames, cut by mouse bites, 40
   keepouts over the tabs and frames (6177 mm2)`. With the reference the
   tool knows the frames and tabs; without it, silence.
6. The same check now exits 1: "constraints missed: relative SW19 SW1
   (12.73 mm)". **F** (15 min): the render shows the left thumb row lying
   on the grid's last row (SW19 at y 74.93, SW15 at 73.66, both 14 mm
   tall), yet `illegal []` and no word of an overlap. KiCad's DRC of the
   placement has no `courtyards_overlap` either (the PG1316 footprint has
   no courtyard). The next action "relax them or give the parts room"
   misleads: `max_gap_mm` 15 gives the same 12.73 mm (the violation is
   the row not being below at all), and `board-placer.json`'s entry
   (`{"kind":"relative","violation_mm":12.73}`) does not say which way.
7. The designer's thumbs are fanned (0, -15, 60 degrees): I dropped the
   thumb rows and placed the six keys at the designer's positions
   (`place` with `x`, `y`, `angle`, `"origin": "absolute"`). Exit 1 with
   **15 warnings "fixed parts SW19 and SW20 overlap (pads on F.Cu)"**
   first in next_actions (every pair of the six), then "2 parts without
   a legal place (U1, SW1)", "SW1 is held by region x -6.30..139.95",
   "the front side is 97 % full", and "try an outline of 327.5 x 135.5
   mm". **F** (20 min): the overlap warnings are about the import's stack
   at the origin, where the six keys are before the `place` moves them;
   the region's left end, -6.30, is the stacked switches' own Edge.Cuts
   slots at the origin, which became a second piece of the outline (the
   same in step 4: board-placer.json's `outline_mm` 292.5 x 121.0 for a
   275.5 x 105.6 mm panel); and "97 % full" contradicts the verdict's own
   `room.utilization` of 0.105 (missing 0 mm2). Moving the stacked parts
   into the board by hand fixed the box (275.5 x 105.6) and left the
   rest.
8. Back to the starter's rows, the stack moved inside, `--reference`:
   the check misses the two thumb rows' `relative` (12.73, 12.10 mm), the
   rows overlapping as in 6. I laid it out anyway:
   `layout-kicad-board ulk3 ULK lay1 auto ulk3/layout.json --reference
   <designer's>`, 117 s wall: 70/70 routed, KiCad 0 unconnected, exit 1
   with `copper_edge_clearance` 15 and `invalid_outline` 1. **F** (10
   min): the advice for the 15 was "for a part at an edge, its edge
   constraint's max_mm or flush; otherwise the project's
   min_copper_edge_clearance": no part is at an edge; drc.json shows each
   is a key's or diode's pad in another key's centre hole (`Circle of
   SW20 on Edge.Cuts` against `Pad 2 of SW13`), the overlap of step 6
   seen by KiCad. `invalid_outline` is SW10's centre hole crossing one of
   the designer's mounting holes (a board cutout the placer let a key's
   slot meet).
9. `fab-kicad-board lay1/result ULK rel1`: refused on the
   `invalid_outline`, correctly. But its note calls the 15 edge findings
   "the input's: every item of each is a part kept where the input has
   it", which they are not (no input was given; the layout placed those
   keys); had the outline been valid, the release would have shipped
   keys on keys.
10. After the rebase onto x1883 and with this branch's fixes (the
    outline box without the stacked slots, `outline.cutouts` from
    `describe-kicad-board`), the documented path alone gives the panel:
    polygon plus 15 cutouts plus the mouse bites, and the place check
    with `--reference` reports `sub_boards: 2 (reference board) ... 2
    frames, cut by mouse bites, 47 keepouts (6422 mm2)` on the real
    outline box (275.5 x 105.6). The straight 6 x 3 grid then has no
    legal place in either half (SW1, SW22): the halves are cut to the
    designer's column stagger, which the starter's `comment` says is the
    agent's to state ("a column-staggered board needs a place or
    relative per column"). Fair, but the "front side is 99 % full"
    beside `room.utilization` 0.135 sends the reader the wrong way.

Does the tool tell about sub-boards, frames and tabs? Only with
`--reference`, and only once the outline the agent gives it has the
panel's slots, which until this branch it could not get from
`describe-kicad-board`. Without a reference a panel whose parts are all
stacked off its pieces (every netlist import) is reported as nothing at
all. Does the advice lead somewhere useful? Twice it pointed at the
wrong lever (relax the relative; the edge constraint's max_mm), once it
buried the real problem under 15 false warnings.

### obsilab Quanta75 (BareRP2040), the nightly's task

1. `describe-kicad-board`: 2 copper layers (the brief said 4), 276
   footprints, 172 nets. The task's constraints: `move_all`, six keys,
   two diodes and J3 fixed, J1, J7, J9 on the top edge at angle 0. A
   `layout.json` naming them, then
   `layout-kicad-board quanta Quanta75_... qlay1 auto quanta/layout.json`:
   743 s wall (minutes of it waiting in the memory ledger behind 26 other
   runs), exit 1, "3 unconnected items in KiCad; new DRC errors:
   clearance 1". 170/172 routed, the first route 71 open, one kept move
   (C25) to 8, the final route 3.
2. The next actions, in order: four warnings that fixed parts lie on the
   board's silkscreen texts (the stabilizer on "Quanta75" and the URL, J3
   on "GND"/"3V3", SW42 on "ISO ENTER", M2.5 on the hotswap note: the
   designer's own art, "all stay where the input put them"), then the
   open connections, then the one clearance, then 18 footprint-own
   findings with their custom rules, two GND pour actions, decoupling.
   **F** (3 min): the useful action is the fifth; the four text warnings
   say nothing can be done and still come first.
3. The clearance: `J3:1 [+3V3]` against `Arc [<no net>] on F.Cu`, the
   designer's copper art beside a fixed header. Advice: "move those
   nets' parts apart, or check whether the rule is one the router does
   not keep". **F** (5 min, drc.json read by hand): both items are the
   input's; J3 is fixed. With `--reference` (run 2) it became
   pre-existing and the verdict then said that fab-kicad-board would
   still refuse it: the right sentence, which run 1's advice lacked.
4. As the advice says, `inspect-routing-run qlay1 --reference <designer's
   project>`: +1V1 (U1:23, U1:50 apart from the rest) and Row1 (U1:27),
   all three at the RP2040's right side on B.Cu, at hot spot 0 where
   +1V1, +3V3, XOUT and ~RESET fought; `reference.connected: true` for
   both, `access` and `cause` `unknown`. **F** (10 min): the verdict's
   sentence "where the designer's board routes those nets on this
   placement ... density is not shown to be the reason" reads as if the
   designer's route applied, but in a layout the designer's placement is
   not this one (U1 and its capacitors moved); the evidence it asks for
   cannot exist for a layout, and nothing says so. The documented lever
   left is the placement around U1.
5. The decoupling action ("12 % of the capacitors within 3 mm ... turn
   on the placer's decoupling_near") was the only placement lever named,
   so run 2: `{"placer": {"constraints": "constraints.json",
   "decoupling_near": true}}` and `--reference` (which also brings the
   designer's via in R21's pad). 683 s wall, exit 1: **8** unconnected
   (+1V1 3, /SWD, Column12, GND, D79-A, J7-CC1) and `starved_thermal` 2;
   capacitors 53 % within 3 mm (from 12 %). Three moves of J8, each worse
   (14, 16, 16 open), none kept. The capacitors packed around U1 closed
   its escape: the lever the verdict offers made the routing worse here,
   as placer.md's own measurement warns ("routing within the noise,
   farther on a few boards"), which the next action does not.
6. `starved_thermal` 2 (U1's pad 57, its exposed pad): "solid_starved_
   thermals (on unless false in the router config) connects them solid"
   while two remain starved. **F** (5 min): if it is on, why are they
   left? board-layout.json's advice ("keep other copper off the pad's
   sides, or give the pad a solid zone connection (its zone_connect 2)")
   is the real one; the verdict line should carry it.
7. Both runs measured 0.37 and 0.38 of the reference's work rates on
   the loaded machine (`work rate: 1.1644e6 expansions/s ... 0.370 and
   0.326 of the reference` on stderr). Each layout's 1500 s of work was
   therefore about a third of the search the nightly's run makes (the
   tiers pin 3.15e6). **F** (15 min, read agents.md's "The work rates"
   twice): nothing in the verdict says that the open connections may be
   the load's; the documented knob (`PCB_WORK_RATE`) repeats the
   reference's work at about 2.7 times the wall. Fixed on this branch:
   such a run now says so in next_actions.

Levers the tool gives for the last connections: `near` constraints and
`decoupling_near` (measured here: worse), `--reference` (pre-existing
findings, the designer's vias in pads), `total_seconds` and
`PCB_WORK_RATE` (more search), router `seeds`. Levers it does not give:
which part to move or keep away from U1's right side (the hot spot names
nets and a point, not the parts whose bodies or escapes crowd it), a way
to keep U1's escape corridor free (an `apart` or a keepout can, but
nothing proposes one), and a way to ask "would the designer's placement
of U1's neighbourhood route?" short of hand-copying positions into
`place` entries.


### oro-os link (8 layers, a length rule), the nightly's task

1. `describe-kicad-board`: 8 copper layers, 381 footprints, 251 nets;
   `link.kicad_dru` holds one rule, `ULPI_exact_length`, length 31.4 to
   31.6 mm on net class ULPI (twelve nets, `ULPI_CK`, `ULPI_D0` to `D7`,
   `DIR`, `NXT`, `STP`, each from the fixed MCU U1 to the movable PHY
   U6). The task fixes U1 and 14 other parts.
2. `layout-kicad-board link link llay1 auto link/layout.json` on x1884
   plus this branch (the placer's `length_maxima` on): 1345 s wall at
   0.56 of the reference's rates, exit 1: "38 unconnected items in KiCad;
   new DRC errors: length_out_of_range 10". U6 sits 17.8 mm from U1, well
   within reach of 31.5 mm.
3. stdout said what the rule became before the verdict: "custom rules
   (.kicad_dru): 1 constraints: approximated 1 (ULPI_exact_length
   length)". **F** (3 min): "approximated: kept more widely than KiCad
   asks" fits clearances; for a length rule, `custom_rules[].detail` in
   the verdict says what it means (a meander after the route, a short
   way for a net over the maximum, no hard bound), which is the sentence
   stdout should carry.
4. The next action for the ten findings: "length_out_of_range 10 (rule
   'ULPI_exact_length' min length 31.4000 mm; actual 0.0000 mm; nets
   /ULPI_CK (1), /ULPI_D0 (1), /ULPI_D2 (1), /ULPI_D3 (1), 6 more): see
   drc.json". **F** (10 min): "actual 0.0000 mm" reads like a measuring
   bug. `verdict.lengths` has the answer per net, and it is good: nine
   nets "no copper the rule counts (not routed)", ULPI_NXT 42.89 mm
   "longer than the maximum: no shorter way was found around the other
   nets (the length repair)" (the route's fault, not the placement's:
   the note says "the placement decides" only where the pads lie
   further apart than the maximum), ULPI_D1 and ULPI_STP met at 31.50 mm
   with 9.14 and 2.27 mm of meander. Over/under and why are all there;
   the next action did not point at them. Fixed on this branch: the
   action now summarises `lengths` (9 not routed, 1 over by the route,
   with the nets and lengths).
5. The open connections' action lists KiCad's unconnected nets by count,
   the eight largest (GND 8, I2C3_SDA 4, ...), and "20 more": the nine
   ULPI nets, the reason for nine of the ten length findings, are in the
   "20 more". `inspect-routing-run --reference`: every ULPI net
   `reference.connected: true`, hot spots near U6 shared with +3V3, the
   PHY's capacitors and the USB pair. As on the Quanta75, the verdict's
   own logic ends at "a hypothesis to test".
6. Again the first next actions were the input's: "fixed parts C17 and
   U1 overlap (courtyards)", "fixed parts H3 and J9 overlap", J6 on the
   text "SD". The task fixed them where the designer has them.

What can the agent do for the lengths? The placer rule for maxima keeps
U6 within reach (it is). Beyond that: nothing ULPI-specific; route the
open nets (more search, room near U6), and the meanders follow. The tool
does report it honestly once one reads `lengths`.

## The result

| | ULK (netlist path) | Quanta75 | oro link |
| --- | --- | --- | --- |
| Runs | 12 place checks, 1 layout, 1 release | 3 layouts (measured rates; `decoupling_near`; pinned reference rates) | 1 layout |
| Best verdict | 70/70 routed, KiCad 0 unconnected, exit 1: 15 edge clearances and an invalid outline from overlapping keys; release refused | 3 unconnected and 1 clearance (the designer's, pre-existing with `--reference`); 8 with `decoupling_near`; 4 at the reference's rates | 38 unconnected, 10 length findings (9 nets unrouted, 1 over by the route) |
| Agent's time | about 2 hours, most of it on the panel's shape | about 1 hour | about 30 minutes |

Quanta75 run 3, to test whether the load was the reason (step 7):
`PCB_WORK_RATE=3.15e6,150e6`, `--reference`, the task's layout.json,
1670 s wall: first route 8 open, ten moves around U1 (C16 kept, 4 open;
U1 itself moved 0.6 mm: 17 open), final **4 unconnected**, no new DRC
error, 1065 s of the 1500 s of work used. The reference's rates gave the
nightly's 4, the loaded runs 3 and 8: on this board the open
connections are the placement around U1's right side, not the search,
and the next action added on this branch says only that the search was
short, which here was not the cause (it says "before changing the
design", which is what the third run did).

## The verdict's advice, ranked by usefulness

1. `--footprints-from` in the import's error line: one retry and every
   footprint was there.
2. `verdict.lengths`' notes (oro link): per net the length, the bounds,
   the meander and why it is out, placement or route. Only reached by
   reading the JSON; the next action now points at them.
3. The reference's pre-existing finding between two nets with the
   release's refusal named (Quanta75 run 2): the one sentence that tells
   an agent what the release will do.
4. `inspect-routing-run --reference`: precise (pads, uuids, hot spots,
   the designer's route), but on a layout it cannot give the evidence
   the verdict asks for (the designer's placement is not this one).
5. The room advice with the frames' area named (ULK with
   `--reference`): right in kind, but next to "front side 99 % full"
   and `room.utilization` 0.135 it cannot be acted on.
6. The decoupling action: correct as a fact (12 % within 3 mm), and the
   lever it offers (`decoupling_near`) made the routing worse here (3 to
   8 open).
7. "relax them or give the parts room" for the thumb rows' `relative`:
   relaxing changes nothing, the rows overlap the grid.
8. Last: the fixed parts' warnings that lead every next_actions list on
   all three boards (silkscreen texts under fixed parts, the designer's
   own overlapping courtyards, and on ULK 15 false overlaps from the
   stack), and the edge constraint's `max_mm` offered for keys in other
   keys' holes.

## Frictions, ranked by cost

1. **A panel from the netlist path loses its shape, silently** (ULK,
   about 45 minutes). `describe-kicad-board` gave the outer loop only;
   the place check then said "complete" on one plate. Fixed:
   `outline.cutouts`, documented. Still open: without `--reference`
   nothing says the outline has pieces at all (below).
2. **Rows placed on rows, reported as a missed `relative`** (ULK, 15
   minutes, and a layout that came back with 15 edge-clearance findings
   and an invalid outline): the legaliser lets the thumb row lie on the
   grid's last row (`illegal []`), and KiCad has no courtyard to object
   with. Open (the placer's).
3. **The stacked footprints' Edge.Cuts slots joined the outline** (ULK,
   20 minutes): regions' halves, `origin: board` and the room were
   measured from the origin. Fixed.
4. **Load decides the search, and nothing says so** (Quanta75, 15
   minutes and a 28-minute rerun): 0.37 of the reference's rate, a third
   of the nightly's search, behind "connections are open". Fixed (a next
   action); on this board the rerun at the reference's rates left 4
   open, so the load was not the cause here.
5. **Fifteen false overlap warnings first** (ULK, 10 minutes): parts a
   `place` puts at `x`, `y` judged at the import's stack. Fixed.
6. **Length findings at "actual 0.0000 mm, see drc.json"** (oro, 10
   minutes). Fixed (the summary of `lengths`).
7. **"on this placement" in the open-connections advice** (Quanta75 and
   oro, 10 minutes): the reference's route is evidence for route mode,
   not for a layout. Open.
8. **"% full" against `room.utilization`** (ULK, 5 minutes and a wrong
   direction): 97 to 99 % against 0.105 to 0.135. Open (the placer's
   hint counts every component, frames' keepouts and board marks
   included, over the outline's gross area).
9. **The input's warnings lead every next_actions list** (all three, a
   few minutes each). Open.
10. **Edge clearance advice for a part's own hole** (ULK, 10 minutes).
    Fixed (names the pairs, says the parts are too close).
11. **The release calls a layout's placed parts "the input's"** (ULK,
    5 minutes; with a valid outline it would have shipped keys on
    keys). Fixed in wording; the release still judges the board on its
    own without `--input`, as documented.
12. `missing` not grouped by footprint and `starter.why` saying "no
    keyboard" when the switches are missing (2 minutes). The second
    fixed.
13. `starved_thermal` advice saying `solid_starved_thermals` is on while
    two pads stay starved (5 minutes). Open.

## Fixed on the branch (each its own commit, with a test)

- `outline`: a footprint's closed Edge.Cuts loop outside every piece the
  board draws is no piece (`a_footprints_slot_outside_the_drawn_board_is_no_piece`).
  ULK's outline box 292.5 x 121.0 mm became 275.5 x 105.6, the regions'
  halves split at 148.45 mm instead of 139.95.
- The input check's fixed-parts overlap leaves out parts a `place` puts
  at `x`, `y` (`fixed_parts_at_one_spot_are_warned_of`, extended); the
  lowering still checks them where the entry puts them.
- The verdict's `copper_edge_clearance` action for copper at another
  part's own Edge.Cuts names the pairs (`SW13 at SW20's cutout`) and says
  the parts are too close (`new_drc_errors_name_their_rule_nets_and_step`,
  extended).
- `describe-kicad-board`'s `outline.cutouts`
  (`the_outline_lists_the_boards_own_cutouts_not_a_footprints_slot`),
  documented in agents.md section 1 and constraints.md. ULK's 15 cutouts
  copied into `outline.cutouts` give the place check (with
  `--reference`) the panel's 2 boards, 2 frames and 47 keepouts.
- `fab-kicad-board` and `drc-kicad-board` without `--input`: the board's
  own findings are said to be judged with the board alone, and
  `--input` is named (`release_gate` test).
- `import-kicad-netlist`'s `starter.why` says parts are missing
  (`no_matrix_with_parts_missing_says_the_parts_are_missing`).
- A next action for open connections at measured rates under half the
  reference's (`open_connections_at_a_third_of_the_reference_rate_say_so`).
- The length findings' action summarises `verdict.lengths`
  (`length_findings_say_which_nets_are_unrouted_and_which_over_by_what`).

## Left open, with a suggested design

- **A panel whose parts are all stacked off its pieces says nothing.**
  sub_boards.rs reports only when parts lie on two pieces or a piece
  holds none of the placed parts; a netlist import has every part at
  the stack. Design: when the outline falls into two or more pieces at
  tabs (the cut is found without parts) and no part is on any piece,
  report `sub_boards` with `count`, `cut_by`, `tabs`, every movable part
  in `unassigned_parts`, and lead next_actions with "the outline falls
  into N pieces (cut by mouse bites) and nothing tells which parts go on
  which: give --reference <the designer's project> or a region per
  board". The other agent's area (panels); not touched.
- **Rows overlapping rows** (`illegal []`): the legaliser should treat a
  row as one rigid body against the other rows and parts, and a part
  without a courtyard should be checked by its body. Placer area.
- **A key's slot over a board cutout** (`invalid_outline` on SW10): the
  placer should keep a footprint's own Edge.Cuts loops off the board's
  cutouts and edges as it keeps bodies. Placer area.
- **The "% full" hint**: use the room's numbers (movable parts over the
  usable area, frames out) or say what it counts.
- **"on this placement"**: in a layout, say that `reference.connected`
  is the designer's route on the designer's placement, and offer the
  comparison that tests it: `route-kicad-board` on the designer's board
  (or the designer tier) against this placement's opens.
- **The input's warnings first**: sort the fixed parts' warnings that
  have no action ("both stay where the input put them") after the
  run's own findings, and fold repeats (15 pairs of one stack into one
  line).
- **A lever for the escape of a crowded IC** (Quanta75): the hot spot
  names nets and a point; naming the parts whose bodies lie within a
  few millimetres of it, with an `apart` or `keepout` to try, would give
  the agent a move that is not "more room".
- **stdout's custom-rules line** for a `length` rule: carry the
  `detail` (meander after the route, the short way over a maximum, no
  hard bound) instead of "kept more widely than KiCad asks".
- **`starved_thermal`**: carry board-layout.json's per-pad advice into
  the verdict line.
