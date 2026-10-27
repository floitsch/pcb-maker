# A fresh agent's third trial — 2026-10-27

One agent that had never used pcb-maker took three boards new to it
through the commands an agent would use, reading only README.md and
docs/agents.md (and, for the panel, docs/placer.md "Sub-boards in a
panel", which the task named), and wrote down every point of friction.
Source code was read only after each board's runs, to fix what was found.
Binaries: `build/bin/pcb-maker-ux4-base` (main 7adb78e1)
for the first board, `pcb-maker-ux4-x1867` (main ff120a4c, the area rules
as `approximated`) for the other two, `pcb-maker-ux4-fix2`/`-fix3` (this
branch) for the reruns that check the fixes. KiCad 10.0.7, a shared
machine (load 20 to 35 on 32 cores, 38 to 48 other runs in the memory
ledger).

## The boards

All three are boards of `benchmarks/real/external/github`, inputs pinned:

| | framework_mobo_lefthalf | MokyaLora | OpenESC 4in1 panel |
| --- | --- | --- | --- |
| Source | CRImier/MyKiCad | tengigabytes/MokyaLora | OpenDrone-hw/OpenESC-30x30 |
| Layers, outline | 6, 28 x 50 mm polygon (arcs) | 6, 66 x 105 mm | 6, 41.6 x 80.4 mm, two boards joined by tabs |
| Parts, nets | 45, 73 | 443, 262 | 404, 184 |
| What it tests | the designer's own findings between fixed parts (R31 and R32 under J1's pads: clearance 2, hole clearance 4; 8 edge clearances of fixed parts) | a `.kicad_dru` with area rules (`TFT_ZONE`, `MCU_ZONE`) | the parts' sub-boards unknown on a stacked panel |
| Started from | the designer's placed board, `describe-kicad-board --constraints-template`, and the task's fixed parts (J1, J2, J5, M1, U3, R31, R32; J1 hollow) | the designer's placed board and the template | the task's board (tracks stripped, parts stacked by run.py's `unplace`) and its constraints |

## Log

Times are wall clock. **F** marks friction: a guess, a retry, or reading
something other than the user documentation.

### framework_mobo_lefthalf

| # | Command | Expected | Happened | Wall |
| --- | --- | --- | --- | --- |
| 1 | read README.md, docs/agents.md (1477 lines) | the workflow | Clear: describe, constraints, place check, draft, final, release. Section 4 names this very board (R31 and R32 at J1), so the expected outcome was known before the first command. | 15 min |
| 2 | `describe-kicad-board fw framework_mobo_lefthalf --constraints-template fw/constraints.json` | a starter | 0.26 s: J1 on the left facing out, C1 near U1:22 at 1.5 mm, J2 and J5 named as other connectors. Added the task's fixed parts and `hollow: ["J1"]`, dropped J1's edge entry (fixed). | 1 cmd |
| 3 | `place-kicad-board fw framework_mobo_lefthalf fw-place fw/layout.json` | constraints checked in seconds | 60 s, 54 s of it in the ledger behind a run.py request. Exit 1: `near C1 U1 (1.60 mm)` at 1.5. **F** three warnings `fixed parts J1 and R31 overlap (pads on *.Cu): ... their pads cannot both be connected` (and J1/R32, J1/M1), plus a bare repeat `fixed parts J1 and M1 overlap`. The designer's board connects all of them (0 unconnected). 5 minutes to check the designer's DRC: R31 and R32 are 0.0867 mm from J1's B6/B7, not on them. After the trial, in the code: the boxes of J1's round pads overlap R31's pad, and R31's pad 1 sits on J1's B5 of the same net (a 0R soldered onto the connector's pad). | 1 min + 5 |
| 4 | `drc-kicad-board fw framework_mobo_lefthalf designer-drc.json` | the designer's findings | exit 1: clearance 18, hole_clearance 93, starved_thermal 3 counted; `footprint:` J1's own hole clearance 8 with a rule; `input:` 8 edge clearances (J2, M1, U3) with a rule. Clear. **F** C1 is 7.96 mm from U1:22 on the designer's board: the template's 1.5 mm asks more than the designer did. Relaxed to 2 mm. | 0.4 s |
| 5 | `layout-kicad-board fw framework_mobo_lefthalf fw-layout auto fw/layout.json` (no reference) | a board, the designer's findings set apart where the rule allows | 4:50 (1200 s of work, 4.2 work over wall), exit 1: 2 unconnected (/TC_D-), new clearance 2 and hole_clearance 4, `footprint_findings` J1 8, `input_findings` edge 8, each with a `.kicad_dru` rule. **F** the clearance and hole clearance actions said "move those nets' parts apart" and "move the parts apart": every one of them lies between parts I had fixed. | 5 min |
| 6 | `inspect-routing-run fw-layout --reference fw` | why /TC_D- is open | 0.5 s: three groups (J1:A7, J1:B7, J2:5), all fixed parts' pads; the designer routes it on F.Cu and In3.Cu (`connected: true`). B7 is the pad R32 crowds. Right and fast. | 1 cmd |
| 7 | the offered rules in a copy of the result's `.kicad_dru`, then `drc-kicad-board` | the 16 findings gone from KiCad's own DRC | **F** the edge rule took the 8 edge clearances away; the J1 rule `(constraint hole_clearance (min 0.1mm))` took none of its 8: KiCad reports them as `board minimum hole clearance 0.2500 mm`. With `(severity ignore)` added: the same. Lowering `min_hole_clearance` to 0.1 mm in the `.kicad_pro` took them away (and R31's and R32's with them: board-wide). And adding the tool's own rule made `drc-kicad-board` warn "the project has rules the router does not keep" about it. 15 minutes and six DRC runs. | 15 min |
| 8 | the same layout with `--reference fw`, the first run's work rates pinned (`PCB_WORK_RATE`) | the designer's findings pre-existing | 12:26 (1.6 work over wall; the first ran routers side by side), the placed and the result board byte for byte the first run's. Exit 1, "2 unconnected items" only; `preexisting_findings` clearance 2, edge 8, hole 12. **F** nothing on stdout says that the release will refuse 6 of those pre-existing findings. | 12 min |
| 9 | `fab-kicad-board fw-layout-ref/result framework_mobo_lefthalf fw-release` | refused, with reasons | 1 s, exit 1: "refused: KiCad's DRC: 6 errors (clearance 2, hole_clearance 4), 2 unconnected items, ..., 8 within one footprint each (not blocking), 8 the board's own, not electrical (not blocking); nothing was exported", then each finding with its items and coordinates. The refusal explains itself; it still offered the J1 rule that does not take. | 1 cmd |

### MokyaLora (x1867)

| # | Command | Expected | Happened | Wall |
| --- | --- | --- | --- | --- |
| 1 | `describe-kicad-board mokya MokyaLora --constraints-template ...` | a starter | J1 at the bottom facing out, 53 near entries. | 1 cmd |
| 2 | `place-kicad-board` | constraints checked; a word on the `.kicad_dru` | 46 s, exit 1: 11 near entries missed by 0.05 to 1.29 mm at 1.5 mm. **F** (the template again) relaxed all by 1.5 mm. Nothing about the custom rules (a placement need not say). | 1 min |
| 3 | `layout-kicad-board mokya MokyaLora mokya-layout auto mokya/layout.json` | which rules were applied, approximated or not modelled | 13:41 (1327 s of work), exit 1: KiCad 0 unconnected, 55 of 55 constraints, one new `solder_mask_bridge` (a GND track of ours under J6's B.Mask rectangle). **F** stdout, verdict.json and next_actions said nothing of the two custom rules. docs/agents.md documents no field for them. Found by listing board-router.json's keys: `rules.custom_rules`, `Shrink_Clearance_In_MCU` `no_effect` ("the net classes keep at least as much"), `TFT_ZONE_2W_Clearance` `approximated` ("clearance 0.25 mm as a pair rule: every net (track to track; In3.Cu) (where a rule area may hold ...)"). The statuses are documented only in the code. | 14 min + 5 |
| 4 | the mask bridge's advice | what to do | "the project's solder_mask_to_copper_clearance and the solder_mask_bridge severity (Board Setup) decide, and the router keeps them": the router did not keep it here (a footprint's mask graphic). Nothing to act on. Not pursued (router side). | - |

What an agent does about an `unmodelled` rule could not be tried on this
board after x1867 (both rules are now modelled). On framework, the J1 rule
added in step 7 above was reported unmodelled by `drc-kicad-board`: "the
project has rules the router does not keep, which KiCad checks:
hole_clearance ('J1 own hole_clearance', min 0.1 mm): the condition
depends on A.memberoffootprint()". It does not say whether that matters:
a rule that only allows less than the router keeps costs nothing.

### OpenESC 4in1 panel (x1867)

| # | Command | Expected | Happened | Wall |
| --- | --- | --- | --- | --- |
| 1 | prepare the task's board: run.py's `strip` and `unplace` from a script (as the first trial did) | the stacked panel | 10 labels dropped, every movable part at (75.6, 67.5). | 2 min |
| 2 | `place-kicad-board panel 4in1-panel panel-place panel/layout.json` | the boards' parts unknown, said | 3:35, exit 0 (`complete`). **F** 21 next actions: 19 "fixed part U3 (U4) lies on the board's text ..." for the two hollow frames, and only then "the board is a panel of 2 different boards sharing net names, and 392 movable parts belong to no sub-board ...: give the designer's placed board with --reference <project-dir> ... or hold each part in its board with region constraints". The advice is exact and actionable; my first look (the last lines of stdout, filtered) missed it. And a placement that mixed the two boards' parts exits 0. | 4 min |
| 3 | `layout-kicad-board panel ... auto panel/layout.json` (no reference) | the baseline | 6:17, exit 1: **F** 8 parts without a legal place (Q17, CL35, CL36, C107, CL40, CL42, CL47, Q20), no route, although the place check had placed all 404. The actions after the panel line: "give the unplaced parts room ... try an outline of 43.0 x 83.0 mm": an outline suggestion for a panel whose frame is fixed, where the parts are short of room only because two boards' parts share one. | 6 min |
| 4 | the same with `--reference <the designer's project>` | each part on its board | 9:45, exit 1: every part placed (`sub_boards.from`: fixed parts, reference board; 0 unassigned, held), 158 of 300 connections routed, 229 vias, KiCad 396 unconnected of which 130 between the two boards by design (`unconnected_across_sub_boards`), 266 counted. Better than the baseline (no placement at all) and as docs/placer.md measured (236 to 240 at seeds 1 and 101). | 10 min |

## The result

| Board | Run | Verdict | KiCad unconnected | Counted errors | Work s | Wall s |
| --- | --- | --- | --- | --- | --- | --- |
| framework | layout, no reference | incomplete | 2 | clearance 2, hole_clearance 4 (fixed parts') | 1200 | 288 |
| framework | layout, `--reference` (same board) | incomplete | 2 | none | 1200 | 744 |
| framework | fab gate on that board | refused | 2 | clearance 2, hole_clearance 4 | - | 1 |
| MokyaLora | layout | incomplete | 0 | solder_mask_bridge 1 | 1327 | 818 |
| panel | layout, no reference | incomplete: 8 parts unplaced | - | - | 660 | 376 |
| panel | layout, `--reference` | incomplete | 266 (+130 across) | none | 1359 | 583 |

## The verdict's advice, ranked by usefulness

1. **The panel's sub-board action** (`--reference <project-dir>` or region
   constraints): exact, one step, and the second run placed every part.
   Its place in the list was the problem (fixed below).
2. **`inspect-routing-run --reference`** for framework's open net: the
   groups, the designer's route, in half a second.
3. **The fab gate's refusal**: counts, what does not block and why, every
   finding with its items and coordinates.
4. **The input's own edge clearances** with a `.kicad_dru` rule: the rule
   took KiCad's 8 findings away as written.
5. **The template's near entries**: a fair start, but at 1.5 mm they
   missed on both placed boards (framework's designer has C1 7.96 mm from
   its pin; 11 of MokyaLora's 53 missed), a retry each.
6. **The pour, decoupling and silkscreen actions**: correct, not tried.
7. Misleading before the fixes: "move the parts apart" for fixed parts'
   findings; the J1 rule that does not take; "their pads cannot both be
   connected" for pads the designer connected; the outline suggestion on
   the panel; nothing on the custom rules.

## Frictions, ranked by cost

1. **The footprint rule that does not take** (15 min, six DRC runs):
   `(rule "J1 own hole_clearance" (constraint hole_clearance (min 0.1mm))
   ...)` left all 8 findings, as did `(severity ignore)`: KiCad reports
   `board minimum hole clearance` where the rule that holds (J1's own
   0.1 mm clearance) is below the board setup's minimum, and applies the
   minimum. The schematic trial's USB4105 findings read `board setup
   constraints hole clearance`, which a custom rule does lower. Fixed:
   such findings get an exclusion or the board setup minimum to lower.
2. **The custom rules invisible** (about 10 min, a key listing of
   board-router.json): neither stdout, verdict.json, next_actions nor
   docs/agents.md said which `.kicad_dru` rules were applied,
   approximated or left to KiCad. Fixed: `custom_rules` in the verdict, a
   stdout line by status, docs/agents.md on each status and what to do.
3. **"Move the parts apart" for fixed parts** (5 min to establish that the
   parts are the designer's and fixed): fixed: the action says the parts
   are kept where the input has them, that the finding counts and the
   release refuses it, and what the choices are.
4. **The overlap warnings** (5 min with the designer's DRC; the cause
   found only in the code): pad boxes, not copper, and pads of one net
   counted as a conflict. Fixed: copper shapes, and pads of one net on
   each other are joined. With `hollow` the placer's own body warning is
   left out, and it is not repeated after the input check's.
5. **The panel's action buried and the outline advice** (3 min, and a
   wrong turn avoided only because the docs had prepared me): fixed: the
   sub-board action leads and says to act on it first; the marks of one
   part are one line.
6. **With `--reference` the verdict hides what the release will refuse**
   (a fab run to find out): fixed: a next action names the reference's
   electrical findings the release refuses.
7. **The place check and the layout disagree on the panel** (exit 0 with
   every part placed; the layout then left 8 unplaced): open.
8. **The place check differs from run to run**: framework's C1 missed
   by 1.60 mm (at 1.5) on one run and by 4.33 mm (at 2) on a later one,
   the measured work rates differing under load (2.23e6 and 1.36e6
   expansions/s). Open.
9. **The template's near distance** below what the designer reached
   (a retry per board): open.
10. **The ledger**: 54 s of a 60 s place check behind a run.py request
    (`place-kicad-board` is meant to be short); the layouts' waits were
    not counted separately. Not pursued.
11. **A stale pointer**: the clearance action named
    `rules.project.unmodelled`, which no report has (the field is
    `rules.unmodelled`): fixed.

## Fixed on the branch (each its own commit, with a test)

- crates/pcb-kicad/src/placement_constraints.rs: fixed parts overlap by
  their pads' copper (`lower_pad`'s shapes), not their boxes; meeting pads
  of one net are joined, not a conflict (tests:
  `fixed_pads_whose_boxes_meet_but_not_their_copper_are_not_warned_of`,
  `fixed_parts_at_one_spot_are_warned_of`). framework's three warnings
  are gone.
- crates/pcb-kicad/src/board_placer/lower.rs, src/agent_cli.rs: one
  warning per fixed part on the board's marks; the placer's body warning
  leaves hollow parts out and is not repeated after the input check's
  (`a_fixed_part_on_many_marks_is_one_warning`,
  `the_placers_overlap_warning_is_not_repeated_after_the_input_checks`).
- crates/pcb-kicad/src/verdict.rs: the panel's sub-board action first
  (`a_panel_without_its_parts_boards_says_so_first`).
- verdict.rs: a footprint's findings under the board minimum get an
  exclusion or the minimum to lower, not a custom rule
  (`a_footprints_own_findings_under_the_board_minimum_get_no_custom_rule`).
- verdict.rs: a clearance or hole clearance between parts kept where the
  input has them says so instead of "move the parts apart"
  (`a_clearance_between_fixed_parts_is_not_the_runs_to_move`); the
  clearance action names `rules.unmodelled` and `rules.custom_rules`.
- verdict.rs, src/agent_cli.rs: `custom_rules` in the verdict and a
  stdout line (`the_custom_rules_are_said_in_one_line`); the unmodelled
  action says how to find KiCad's findings under such a rule.
- verdict.rs: with a reference, a next action names the reference's
  electrical findings that the release refuses
  (`the_references_electrical_findings_are_said_to_refuse_the_release`).
- docs/agents.md: each of the above.

Checked on the boards with the branch's binary: framework's place check
(no overlap warnings; M1's two marks one line), framework's layout at the
first run's pinned rates (the same result board byte for byte; the new
clearance and hole clearance actions; the J1 note with the exclusion),
framework's fab gate (the J1 note), MokyaLora in route mode on the
designer's board (`custom rules (.kicad_dru): 2 constraints: approximated
1 (TFT_ZONE_2W_Clearance clearance), no_effect 1; ...` and `custom_rules`
in verdict.json).

## Left open, with a suggested design

- **The place check and the layout disagree** (the panel: 404 of 404
  placed and exit 0 by `place-kicad-board`, 8 unplaced by the layout).
  The layout's placement race runs other seeds and budgets than the place
  check. Suggest: the layout's verdict, when parts find no place, says
  what the place check would have said differently (its seed and level),
  or the place check runs the race's first seed at the layout's budget
  share, so that its yes means the layout's.
- **A placement whose panel parts are mixed exits 0.** Suggest: with
  `sub_boards.unassigned` above the tenth that holds placement, the
  placement is `incomplete` (exit 1), the sub-board action its reason.
- **Room advice on a panel with mixed boards**: the outline suggestion
  still follows; now after the sub-board action that says to act first.
  Suggest: no `suggested_outline_mm` while the parts' boards are unknown.
- **The template's near distance on a placed board**: suggest
  `max_mm` at least the designer's distance (rounded up to 0.5 mm) where
  that is above 1.5 mm, with the designer's distance in the comment.
- **The place check's reproducibility**: suggest that `place-kicad-board`
  pins the reference work rates by default (it is a check, not a
  layout), or prints the line to pin them.
- **The mask bridge under a footprint's mask graphic** (MokyaLora's J6
  B.Mask rectangle over a GND track of ours): the router does not see a
  footprint's mask graphics as openings; the action's "the router keeps
  them" is not true for them. Router side.
- **The fab gate's refusal on fixed parts' findings** still says "move
  what is too close"; with `--input` the gate could say which findings
  are between parts the layout kept, as the layout's verdict now does.

Inputs and outputs: build/trial3-ux4/ in the main checkout's build
directory (fw*, mokya*, panel*).
