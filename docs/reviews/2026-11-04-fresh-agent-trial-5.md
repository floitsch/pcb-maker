# A fresh agent's fifth trial — 2026-11-04

One agent that had never used pcb-maker took three boards new to it
through the commands an agent would use, reading only README.md,
skills/pcb-maker/SKILL.md and docs/agents.md (and the pages agents.md
sends the reader to: constraints.md, router.md's "Differential pairs",
placer.md's panel sections), and wrote down every point of friction.
Source code was read only after a board's first runs, to fix what was found.
Binaries: the worktree's `target/release/pcb-maker` at main 55597d14
for the first board's description and place check, then (rebased as
asked) at main d37c24d4 (x1895, the verdict's wording) for its layouts
and route, at e8759889 and e7e7de8c (this branch's first five and
seven fixes) for the other two boards, and at 4d624ed6 for the panel's
second layout. KiCad 10.0.7, a shared machine (load 27 to 42 on 32
cores, 26 other runs in the memory ledger; three runs waited 5 minutes
each for their reservation; the measured search rate 0.38 to 0.78 of
the reference's).

## The boards

| | ducktop2 right_io | antmicro jetson-nano-baseboard | SmartSpin2k_Panelized |
| --- | --- | --- | --- |
| Layers, outline | 8, 58 x 117 mm, the designer's outline with notches | 8 (not 4, as the brief said), 100.2 x 55 mm | 2, a 189.5 x 204.9 mm panel: 9 copies in 21 pieces cut by V-cuts, 12 frames |
| Parts, nets | 174, 128; 21 differential pairs | 391, 340 | 749 (9 copies of 79 parts, 20 `REF**` and 18 `G***` marks), 1017 |
| What it tests | a `.kicad_dru` with pair gaps, per-layer pair widths and disallow rules | a dense board's open connection, hot spot and levers | copies, frames and keepouts |
| Started from | the expansion tier's input (the designer's placement, tracks stripped; not under benchmarks/real/external: the harvest has ducktop2's bms only), constraints from `--constraints-template` | the nightly's task input (parts stacked, 106 fixed, three edge connectors) | the nightly's task input (parts stacked, 81 fixed: U1, C2, C10, C11, R4, R5, R6, R10, R11 of every copy) |

## Log

Times are wall clock. **F** marks friction: a guess, a retry, or reading
something other than the user documentation.

### ducktop2 right_io (8 layers, `.kicad_dru` pair, width and disallow rules)

The board is not under `benchmarks/real/external` (the brief's place):
the harvest has ducktop2's `bms` only; right_io is an expansion-tier
task, and its copies in `build/diff1/*/EwoudVV__ducktop2__right_io/source`
are the designer's placement with every track stripped (0 segments, 7
vias). Started from that copy. 174 parts, 128 nets, 21 differential
pairs (HDMI through AC-coupling capacitors, USB, PCIe, the GbE MDI
pairs), 32 custom-rule constraints.

1. `describe-kicad-board ducktop right_io --constraints-template
   ducktop/constraints.json`: exit 0, "4 edge, 42 near entries; read its
   comments". **F** (5 min): the edge list holds D2090, a B340A clamp
   diode in `Diode_SMD:D_SMA`, as a plug-in connector (`SMA` read as the
   coax connector). The description says nothing of the project's custom
   rules or its pairs (a newcomer learns of them from the verdict). The
   comment's `near` list keeps C2409 at U42:38 at 29.8 mm and C1770 at
   11.85 mm (the designer's distances: right, but no hint that such a
   capacitor is likely not that pin's). Removed D2090's entry, fixed the
   mounting holes, the FPC to the centre board, the power looms and the
   RJ45 (11 parts), kept the 42 `near` entries.
2. `place-kicad-board ducktop right_io d-place1 ducktop/layout.json`
   (binary 55597d14): exit 1, 39 s wall, 8 `near` missed by 0.09 to
   0.79 mm (C505 to C512 around U500's QFN-32). Next action: "constraints
   missed (constraints[] has violation_mm for each)". **F** (2 min): which
   file's `constraints[]`? (`board-placer.json`.) Accepted the misses.
3. Rebased onto d37c24d4 (x1895) as the coordinator asked, rebuilt;
   `layout-kicad-board ducktop right_io d-lay1 auto ducktop/layout.json`:
   exit 1 after 15 min wall (0.38 of the reference's rate). 12
   unconnected, **54 `diff_pair_gap_out_of_range`**, the 8 near misses.
   `verdict.diff_pairs` is null. The custom-rules line: "applied 10 ...
   unmodelled 17", the six `diff_pair_gap` constraints among them with
   the detail "differential pairs route coupled only with `diff_pairs`
   (off)". The next actions: "diff_pair_gap_out_of_range 54 (right
   DIFF_100 on outer maximum gap 0.2540 mm; actual 0.6257 mm; nets ...):
   see drc.json", and "the project has rules the router does not keep
   ... their findings are for the designer to resolve by hand or by
   moving parts", listing eleven intra-footprint clearance rules that
   only lower the clearance and the six pair gaps. **F** (25 min, the
   board's largest): the verdict calls the pair findings the designer's
   to fix by hand; README's "differential pairs are routed coupled by
   default only in route mode" and agents.md's `diff_pairs` paragraph
   ("route mode routes differential pairs ...") left open whether a
   layout can couple them at all; router.md's "a layout's routes keep it
   off" and the verdict table's "absent in a layout's verdict unless
   switched on" suggested a switch, without saying where. Also on stdout:
   "ESD protector C1771: null mm from its pins" (a capacitor whose value
   names TPD1S514).
4. `inspect-routing-run d-lay1 --reference ducktop`: 11 nets, every one
   `reference.connected: false`. **F** (10 min): the reference is the
   stripped copy (0 tracks); nothing in the report says the reference
   has no copper, and `connected_means` reads "shows that a net routes
   under these rules there". Without the harvest's routed right_io there
   is no designer route to compare.
5. The documented lever for pairs: route mode on the layout's
   placement, `route-kicad-board d-lay1/result right_io d-route1 auto
   --strip --reference none`: exit 1, 12 min. Pairs on by the rules
   (`default: rules`): **1 of 21 coupled, 1 partly, 19 fell back**; 39
   unconnected (the layout had 12), 69 pair-gap findings. The verdict's
   next actions do not mention the pairs; `board-router.json`'s
   `diff_pairs.pairs[].why` does: 13 pairs "fewer than two parts with
   the pair's pads side by side", the rest links the hard repair found
   no room for, or a stub that does not reach the run. **F** (15 min):
   reading router.md "Pair points and links" explains it: a pair couples
   only between two parts whose pads of the pair lie within 3 mm, and
   the layout had put each HDMI AC-coupling capacitor away from its
   partner (C150 and C151 on TCP2_TX0 P and N).
6. The improvement: `row` constraints for the six AC-coupling pairs
   (`{"parts": ["C150", "C151"], "pitch_mm": 1.0, "axis": "y"}`, ...)
   and router.json `{"diff_pairs": true}`, `layout-kicad-board ducktop2
   right_io d-lay2 ducktop2/router.json ducktop2/layout.json`: exit 1,
   22 min (5 of them waiting in the memory ledger). **7 of 21 pairs
   coupled, 1 partly** (307 mm coupled; all four TCP2 pairs from the
   capacitor rows to the mux, PD2_CONN, GBE_HOST_TX, GBE_REFCLK), 9
   unconnected (12 before), 72 pair-gap findings (the uncoupled stubs
   and the HDMI side, whose receptacle has a shield pin between each
   pair's pads: "fewer than two parts side by side" stays), and a new
   **short**: C502 and C503 (0402, turned 90 degrees, 1.82 mm along y)
   in a row at 1 mm. The row was accepted, the placement check said
   nothing, and KiCad found `shorting_items` and `courtyards_overlap`.
   **F** (10 min): my pitch, but no check said so. The place check also
   counted the six row followers as "kept where they are" (17 where 11
   are fixed).

Agent's time on the board: about 2 hours (55 minutes of it waiting on
runs).

### jetson-nano-baseboard, the nightly's task

1. `describe-kicad-board jetson jetson-nano-baseboard`: 391 footprints,
   340 nets, 8 copper layers, no custom rules, no `sub_boards`, no swap
   candidates. Nothing to change in the task's constraints.
2. `place-kicad-board jetson jetson-nano-baseboard j-place1
   jetson/layout.json --router jetson/router.json` (binary e8759889,
   this branch's first five fixes):
   exit 0 in 76 s, "complete: every part placed, every constraint
   kept", "placed 285 of 391 parts (106 kept where they are; 17 board
   marks and rule areas kept clear of)". Then two tagged observations:
   "36 pairs of fixed parts overlap (C10 and J15 (courtyards), ...)" and
   "12 fixed parts lie on 14 of the board's texts". **F** (10 min, on
   main's wording): the line said the pairs' "pads cannot both be
   connected; give one of each pair its own place" under the tag
   "nothing to do". J15 is the Jetson's SO-DIMM socket (77.9 x 19 mm);
   the 36 capacitors and resistors lie inside its courtyard on purpose,
   their pads apart. Fixed before the layout (below): the line now says
   the pads are apart and it costs no connection.
3. `layout-kicad-board jetson jetson-nano-baseboard j-lay1
   jetson/router.json jetson/layout.json` (the documented default
   budget, 1500 s of work): exit 1 after 17.6 min wall (5 min of it in
   the memory ledger: "waiting to reserve 2118 MB for a router of 152 MB
   of maps ... behind an older request, run.py: ...video ... the ledger
   drains for it"). First route 3 open, 14 moves (7 kept), final **1
   unconnected** (Display/TMDS_D0_C_N, J1:11 to R8:2) and **2 new
   `clearance`**. The verdict:
   - "connections are open ... This placement is the layout's, not the
     designer's: reference.connected is the designer's route on the
     designer's placement ..." then "hot spot 0 (F.Cu at 108.45,
     122.58, on the open nets Display/TMDS_D0_C_N): the parts nearest
     it on F.Cu are J1 (front, over it), D11 (2.8 mm), D4 (3.4 mm), R37
     (4.3 mm), R2 (5.5 mm) ... move or turn one of them". Clear, but
     **F** (10 min): the open link's other end, R8, is not among them;
     `inspect-routing-run j-lay1 --reference <the harvest's board>`
     shows why the link is open: R8:2 lies 10.4 mm from J1:11, where
     the designer's R8 sits beside it (a 3.6 mm route on F.Cu,
     `reference.connected: true` "on the designer's own placement").
     The lever an agent wants (a `near` for R8 at J1:11) is in no next
     action.
   - "clearance 2 (clearance 0.1250 mm; actual 0.0000 mm; nets GND (2)):
     move those nets' parts apart, or check whether the rule is one the
     router does not keep". **F** (10 min): one net, no part: drc.json's
     items are an "Arc [<no net>] on In4_GND.Cu" (two copper circles
     the designer drew on In4.Cu) and the input's GND zone. Fixed (the
     action names the board's own graphics and zones).
   - The decoupling line (36 % within 3 mm, "turn on the placer's
     decoupling_near"), the GND pour's six pieces on In4.Cu with a
     `keep_whole` entry to paste, the silkscreen fields: actionable.
   - The memory and the work rate: `work_seconds` 1299 of the 1500,
     `wall_seconds` 1055 (the 5 minutes in the ledger included and not
     named in the verdict), `work_wall_ratio` 1.23, rate 0.78 of the
     reference's (no rate action below half). The ledger's stderr line
     is long but says who is ahead and why; that a router of 152 MB of
     maps reserves 2.1 GB (maps and 600 MB with a 1.25 margin, plus
     KiCad's 2.2 GB while it checks) needs agents.md's ledger paragraph,
     a 60-line block a newcomer does not need otherwise. Agents.md says
     "the first line on stderr is `work rate: ...`"; here it was line
     37, after the 36 overlap warnings.

Agent's time on the board: about 45 minutes, 18 of them waiting.

### SmartSpin2k_Panelized, the nightly's task

1. `describe-kicad-board smartspin SmartSpin2k_Panelized`: `sub_boards`
   with 21 pieces cut by V-cuts, 12 frames (`frames_by: parts`), and 9
   boards of 9 parts each, `movable: 0` on every one. **F** (10 min):
   `tabs` is one box, `[15.0, 15.0, 204.464, 219.9]`, the whole panel
   (the V-cut strips form one connected grid, and its box is reported as
   the tab); `movable: 0` per board while 630 parts are to be placed
   (the stack lies off every piece; agents.md says `parts_assigned`
   counts parts on a piece, not what `movable` means here).
   `swap_candidates` lists U1's IO pins nine times over (one per copy,
   the reference shared).
2. `place-kicad-board smartspin SmartSpin2k_Panelized s-place1
   smartspin/layout.json`: exit 0 in 53 s, "complete: every part placed,
   every constraint kept". The copies are named on stderr only: "panel
   copies: 9 copies of 79 parts (net islands, regions from fixed
   parts)", "sub-boards: a panel of 9 copies, 12 frames, cut by V-cuts,
   490 keepouts over the tabs and frames (10129 mm2, each copy's in every
   copy; margin 1.5 mm ...), 36 connectors and 0 designer's overhangs
   may reach over them". stdout and next_actions say nothing of copies,
   frames or keepouts. The verdict's `panel_copies` (`replicated: true`,
   9 cell classes of one copy each, `by_cell_classes: false`) and
   `sub_boards` (`copies: true`, `keepout_margin_mm` 1.5,
   `kept_clear_mm2` 10129) answer once read against agents.md's table
   rows, two cells of 1500 and 2600 characters. **F** (15 min): what an
   agent can still influence on a panel of copies is not said anywhere:
   constraints.md says a repeated reference "names all of them in `fixed`
   and the other lists" and that an `edge`, `rotation` or `place` entry
   is about one footprint, but not whether a `near` on the first copy's
   C9 holds in every copy, or what a constraint on the fifth copy's part
   does to the replication. The folded observations named "C2 and U1
   (courtyards)" and "U1 on 2" five times each (fixed: named once with a
   count). `room.utilization` 0.775 on the front, `smallest_outline_mm`
   193.5 x 209.5: a panel's outline is no lever, and no action offered
   it (right).
3. `layout-kicad-board smartspin SmartSpin2k_Panelized s-lay1 auto
   smartspin/layout.json` (binary e7e7de8c, measured rates): exit 1
   after 8.7 min wall, **475 unconnected**, no new DRC error. The first
   route left 1092 open terminals and took the budget (`work_seconds`
   1388 of 1500, `work_wall_ratio` 2.67); no move and no final ladder
   ran, and the verdict says neither (`skipped` and `cut` empty). The
   next actions in order: connections are open (the nets by copy,
   "Board_7-GND (23), Board_1-GND (22), ..."), the hot spot on copy 0
   (U2, C9, J1, C1, C6), then "this run searched at 0.39 of the
   reference's rate ... rerun on an idle machine, or pin the reference's
   rates (PCB_WORK_RATE=3.1500e6,1.5000e8: the same work, about 2.6
   times the wall here) before changing the design".
4. Followed the third action: `PCB_WORK_RATE=3.1500e6,1.5000e8
   layout-kicad-board ... s-lay2` (binary 4d624ed6): exit 1 after 21 min,
   **51 unconnected**, no new DRC error, first route 51 open, 3 moves
   (none kept). The pinned placer rate changed the placement too (11
   seeds, seed 7 kept, against 5 and seed 5). **F** (the board's cost: a
   first layout an agent would have taken for the tool's best): on this
   board the rate was the cause of nine tenths of the open connections,
   and its action came third, after the advice to move parts at a hot
   spot.

Agent's time on the board: about 1 hour, 30 minutes of it waiting.

## The result

| | right_io | jetson-nano-baseboard | SmartSpin2k |
| --- | --- | --- | --- |
| Runs | 2 place checks, 2 layouts, 1 route | 1 place check, 1 layout | 1 place check, 2 layouts |
| Best verdict | 9 unconnected, 72 `diff_pair_gap_out_of_range` and a short of my row (7 of 21 pairs coupled; the default layout: 12 unconnected, 54 pair-gap findings, none coupled) | 1 unconnected (an HDMI TMDS line, its series resistor 10.4 mm from the connector pin), 2 clearances between the input's drawn arcs and its GND zone | 51 unconnected at the reference's rates (475 at the measured 0.39), no new DRC error |
| Agent's time | about 2 hours | about 45 minutes | about 1 hour |

## The verdict's advice, ranked by usefulness

1. The work-rate action (SmartSpin2k, 0.39 of the reference): "pin the
   reference's rates (PCB_WORK_RATE=...) before changing the design"
   took the panel from 475 unconnected to 51. Third in its list, though.
2. The hot spot with the parts nearest it, by side and distance
   (jetson): a concrete set of parts and three levers; it missed only
   the open link's own far end.
3. `custom_rules` per constraint with its `detail` (right_io): every
   one of 32 constraints accounted for; "differential pairs route
   coupled only with `diff_pairs` (off)" was the only place that named
   the switch, in the JSON, not in an action.
4. `board-router.json`'s `diff_pairs.pairs[].why` (route mode): per
   pair why it fell back ("fewer than two parts with the pair's pads
   side by side"), which led to the row lever. Reached only by reading
   the report; the next actions now carry it.
5. The `keep_whole` entry for a fragmented pour, ready to paste (all
   three boards), and the decoupling line with the setting to try.
6. `inspect-routing-run --reference`: the open net's two groups, the
   designer's route and `reference.placement` ("the designer's own
   placement"), precise (jetson: R8 10.4 mm from J1:11 against the
   designer's 3.6 mm route), but silent that a stripped reference has no
   route to compare (right_io).
7. Last: "the project has rules the router does not keep ... for the
   designer to resolve by hand or by moving parts", listing eleven rules
   that only lower a clearance and the six pair gaps the router can keep
   with one setting; and "move those nets' parts apart" for a GND-only
   clearance between a drawn arc and a zone.

## Frictions, ranked by cost

1. **A layout routes the pairs uncoupled and says nothing about it**
   (right_io, about 40 minutes and two extra runs): 54
   `diff_pair_gap_out_of_range` with "see drc.json", `diff_pairs`
   null, the pair gaps among "rules ... for the designer to resolve by
   hand". README and agents.md say route mode couples pairs; whether a
   layout can was left to router.md ("a layout's routes keep it off")
   and the verdict table ("unless switched on"). Route mode on the
   layout's placement then coupled 1 of 21 and left 39 unconnected (12
   before) without a word of the pairs in its actions. Fixed: the
   `diff_pair_*` action gives the setting where pairs were off, else
   the counts and the reasons per pair; docs and the SKILL say
   `"diff_pairs": true` works in a layout and what a pair needs.
2. **The work-rate action comes third where it is the cause, and
   nothing says the first route took the whole budget** (SmartSpin2k: a
   9-minute layout taken at face value is 475 open instead of 51; no
   move and no final ladder ran, `skipped` and `cut` empty). Open
   (below).
3. **A pair couples only between parts side by side; nothing places
   them so** (right_io, 15 minutes): 13 of the 19 fallen-back pairs had
   their AC-coupling capacitors apart. A `row` of each pair's two
   capacitors coupled 7 (and part of an eighth) with `diff_pairs: true`.
   Documented (agents.md, constraints.md, the SKILL; the action names
   the row); the placer still knows no pairs (below).
4. **What an agent can influence on a panel of copies** is not
   documented (SmartSpin2k, 15 minutes): which copy a constraint holds
   in, whether a `near` on one copy's part holds in all. Open.
5. **A row whose pitch shorts its parts is accepted** (right_io, 10
   minutes and a layout with a `shorting_items`): two 0402 capacitors
   1.82 mm long along the row at a 1 mm pitch. Fixed: refused before the
   run, exit 2, with the length needed.
6. **The reference's `connected: false` from a reference without
   copper** (right_io, 10 minutes). Fixed: `reference_tracks` and a
   `connected_means` that says so.
7. **Courtyard overlaps called lost connections** (jetson, 36 pairs;
   SmartSpin2k, 72): "their pads cannot both be connected" for parts
   under a socket. Fixed.
8. **"move those nets' parts apart" for a drawn arc against a zone**
   (jetson, 10 minutes). Fixed.
9. **The open link's far end is not among the hot spot's parts**
   (jetson, 10 minutes). Open.
10. **The panel's V-cut grid reported as one tab over the whole panel**,
    and `movable: 0` per board in `describe-kicad-board` (SmartSpin2k,
    10 minutes). Open.
11. **Clearance rules that only lower a clearance listed as rules the
    router does not keep** (right_io, 5 minutes of reading eleven
    rules). Fixed (`no_effect` with the reason).
12. **A diode's SMA package in the starter's edge list** (right_io, 5
    minutes). Fixed.
13. **The copies' repeated references in folded lines** (SmartSpin2k:
    "C2 and U1 (courtyards)" and "U1 on 2" five times each). Fixed
    (named once with a count).
14. "constraints missed (constraints[] has violation_mm ...)" names no
    file (2 minutes). Fixed. "ESD protector C1771: null mm from its
    pins" for a capacitor named after its protector (2 minutes). Fixed.
15. The place check counts a row's followers as "kept where they are"
    (17 where 11 are fixed; 2 minutes). Open.
16. The brief's board was not where it said (ducktop2 right_io is an
    expansion task, stripped); a newcomer would want the designer's
    routed board as a reference first. Not the tool's.

## Fixed on the branch (each its own commit, with a test)

- `describe-kicad-board --constraints-template` (and the netlist
  import's starter): a diode's package (`Diode_SMD:D_SMA`, a footprint
  name starting `D_`) is no plug-in connector
  (`plug_in_connectors_are_told_by_their_footprint_names`).
- `inspect-routing-run`: `reference.reference_tracks`; with none,
  `connected_means` says `connected` shows nothing and asks for the
  designer's routed board
  (`the_reference_block_says_whose_placement_it_routes`).
- Custom rules: a clearance minimum at or below every net's clearance,
  or a hole or edge clearance at or below the board setup's, is
  `no_effect` whatever its condition (right_io: unmodelled 17 to 11)
  (`a_minimum_below_what_the_router_keeps_has_no_effect_whatever_its_condition`).
- The verdict's `diff_pair_*` action: off, the router setting that
  couples the pairs; on, "diff_pairs: C of F pairs coupled, P partly, B
  routed as two nets; why, per pair: ..." and the row lever where pads
  were not side by side; the unmodelled `diff_pair_gap` rule's detail
  names the setting (`diff_pair_findings_say_what_became_of_the_pairs`).
  Docs: agents.md (pairs in a layout, the row), constraints.md ("not
  supported" said what is), the verdict table, the SKILL.
- A `row` whose pitch puts two members' pads on each other is refused
  before the run (`a_row_whose_pitch_puts_pads_on_pads_is_refused`).
- score and the layout's stdout: a capacitor, resistor or inductor whose
  value names a protector is no ESD protector; an unmeasured distance is
  said, not printed as `null mm`
  (`a_capacitor_named_after_its_protector_is_no_protector`).
- Fixed parts whose courtyards or bodies overlap but whose pads are apart
  cost no connection, and the folded line counts the pairs with pads on
  each other (`repeated_input_observations_fold_into_one_line`).
- The constraints-missed action names `board-layout.json` or
  `board-placer.json` (`placements_and_layouts_answer_for_their_constraints`).
- A clearance whose every item is the board's own copper graphic of no
  net or a zone names them and no part to move
  (`a_clearance_between_board_graphics_and_a_zone_names_no_part`).
- A folded observation names a repeated name once with its count
  (`repeated_input_observations_fold_into_one_line`).

## Left open, with a suggested design

- **The rate first where it decides.** When the measured search rate is
  under half the reference's and the first route left more open than
  the layout's moves can take on (no move ran, or the first route used
  more than 80 % of the budget), lead next_actions with the rate action
  and say what the first route took ("the first route used 1145 of the
  1500 s of work: no moves and no final ladder ran"); SmartSpin2k: 475
  unconnected measured, 51 pinned. verdict.rs `search_rate_action` and
  the order of section (3).
- **V-cut tabs**: report the V-cut strips as one box per straight strip
  (or `v_cut_strips` beside `tabs`), and in `describe-kicad-board` a
  board's `movable` as the parts nothing holds that will be placed
  there, or drop it where nothing is assigned yet. sub_boards.rs; the
  panels' agent's area.
- **Panels of copies in agents.md**: one paragraph, what the placer does
  (places one copy per cell class and replicates it), which constraints
  an agent writes for it (on the first copy's parts, by uuid where the
  copies differ), what `cell_classes` and `by_cell_classes` say, and
  what reaches over frames (connectors, `--reference`). Needs the panel
  code's owner to state the behaviour.

- **Pairs in the placement.** The placer knows no pair; the router
  couples only between two parts whose pads of the pair lie side by
  side. Design: where `diff_pairs` is on (or the project's rules ask),
  lower each pair's two-pad series parts (same footprint, pads on the P
  and N nets of one pair) as an implicit row of two at their pitch
  (body length along the pads' normal), as `row` does, and report it in
  `constraints[]` as `pair_row`; measure on right_io and the expansion
  tier's 25 boards with pairs.
- **Pairs on by default in a layout where the rules ask.** Right_io's
  default layout: 12 unconnected and 0 coupled; with `diff_pairs: true`
  and the rows, 9 and 7 coupled. One board; route mode's experience
  (connections lost on dense boards in phase 2) says to measure before
  changing the default.
- **The open link's far end in the hot-spot action.** When an open net
  has two groups, name the part of the smaller group and its distance to
  the other group's nearest pad ("R8:2 lies 10.4 mm from J1:11"), with a
  `near` entry to paste (`{"part": "R8", "pin_of": "J1:11", "max_mm":
  2}`), where neither part is fixed. verdict.rs `hot_spot_action`.
- **The ledger's wait in the verdict.** `wall_seconds` includes the
  minutes a run waited for its reservation; a `ledger_wait_seconds`
  beside it (and in `work_wall_ratio`'s note) would let an agent read
  the ratio. The ledger is another agent's area; not touched.
- **A row's followers in the placed count**: count them as placed (they
  move with their row) in `placed N of M parts (K kept where they are)`.
- **The placer's ESD pull** (board_placer/decoupling.rs) still takes a
  capacitor whose value names a protector as a protector and pulls it to
  a connector; the fix above is the report's only, as the placer is
  another agent's area: apply the same `C`/`R`/`L`/`FB` test there, and
  measure.
