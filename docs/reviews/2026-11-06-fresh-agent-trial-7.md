# A fresh agent's seventh trial — 2026-11-06

One agent that had never used pcb-maker took three boards of classes no
earlier trial covered through the commands an agent would use, reading
only README.md, skills/pcb-maker/SKILL.md and docs/agents.md (and, for
the custom rules, router.md's "Rules beyond the classes", which
agents.md's custom-rules row points at), and wrote down every point of
friction. Source code was read only after a board's first runs, to fix
what was found. Binaries: `build/bin/pcb-maker-ux14-a` (main a1ed18ad,
x1920: the skeleton race on by default) for every first run, `-b` (the
branch's first two fixes) for MIDAS-MK2's short layout, Sisu's route
mode and both fab releases, `-c` (five fixes) for Sisu's last two
layouts. KiCad 10.0.7, a shared machine (load 9 to 14 on 32 cores, the
nightly not yet running; no run waited in the memory ledger; the measured
search rate 0.98 of the reference's).

## The boards

| | briskspirit Sisu_SSE-9 | ISSUIUC MIDAS-MK2 (released) | ISSUIUC MIDAS-MK2 |
| --- | --- | --- | --- |
| Class | six layers, impedance-controlled widths | fab release end to end | the new placer default as an agent sees it |
| Layers, outline | 6, the designer's 43.9 x 118.2 mm polygon (arcs); GND pours on F.Cu, In1.Cu, In2.Cu, In4.Cu, B.Cu, +3.3V on In3.Cu | 4, 45 x 100 mm | the same |
| Parts, nets | 296 (250 in the BOM), 192 | 217, 156 | 217 (197 movable, 131 passives), 156; room 38 % front, 48 % back |
| What it tests | `.kicad_dru` "USB 90 ohm differential geometry on L2 and L5": `track_width` min = opt = max 0.10 mm and `diff_pair_gap` 0.11 to 0.13 mm on In1.Cu and In4.Cu only, `diff_pair_uncoupled` 2 mm, `skew` 0.5 mm; "Keep USB off L3 and L4" (`disallow track` on In2.Cu, In3.Cu); classes RF_ANT 0.1422 mm and RF_ANT_INT 0.1102 mm (the designer's 50 ohm microstrip and stripline, stated as class widths only) | the layout's result through `fab-kicad-board --profile jlcpcb`, the Gerbers against kicad-cli's own export | the skeleton race at the default budget (600 s of placement work a seed) and at `total_seconds` 500 (200 s, under the race's 500 s threshold) |
| Started from | the nightly's task input (parts stacked, U1, LS1, SW1, J8 fixed, J5 on the left edge), then route mode on the designer's board with `--strip` | the layout below | the nightly's task input (17 fixed parts, five connectors on edges) |

## Log

Times are wall clock. **F** marks friction: a guess, a retry, or reading
something other than the user documentation.

### Sisu_SSE-9, six layers with per-layer widths

1. `describe-kicad-board sisu Sisu_SSE-9`: 0.6 s, six copper layers, 296
   footprints, 192 nets each with `track_width_mm` (USB nets 0.1, RF nets
   0.1422, MODEM_ANT1 0.1102). **F** (5 min): nothing of the project's
   `.kicad_dru` (two rules on the USB class by layer), of the pairs, or
   of which class a net is in; the class widths are only numbers per net.
   Read the `.kicad_dru` and the `.kicad_pro` by hand to learn that 0.1422
   and 0.1102 are the designer's impedance widths (the rule file's
   comment names the stackup JLC06101H-3313).
2. `--constraints-template`: 71 `near` entries and a comment on the skew
   rule ("on 11 nets ... the router matches the group's nets by
   meanders; the placer holds nothing of it"). Not said: the rule holds
   on In1.Cu and In4.Cu only. Kept the task's constraints.
3. `layout-kicad-board sisu Sisu_SSE-9 s-lay1 auto sisu/layout.json`:
   exit 1 after 8 min 45 s (1422 s of work), **18 unconnected**, 1
   `diff_pair_uncoupled_length_too_long`. The custom-rules line: `7
   constraints: applied 1, no_effect 1, skipped 2, unmodelled 2
   (diff_pair_gap, diff_pair_uncoupled)` and the skew applied by
   meanders; `verdict.custom_rules` gives the per-layer `track_width` as
   `no_effect` ("routed at 0.1 mm on 11 of 330 nets on In1.Cu, In4.Cu (per
   layer): the net classes and the board setup keep it") and the
   `disallow` as applied. Checked the written board with a script: every
   USB track 0.1 mm, none on In2.Cu or In3.Cu (both honoured). **F** (20
   min, the trial's largest): every RF net, RF_ANT's 0.1422 mm and
   MODEM_ANT1's 0.1102 mm, is written at **0.1 mm** (Net-(J2-RF) 79 mm of
   it, MODEM_RFCTRL2 47 mm). The log says "the narrow-signal step
   applies"; the verdict, board-layout.json and the next actions say
   nothing, and KiCad's DRC checks no class width, so no finding exists.
   An agent that trusts the verdict ships a 50 ohm antenna feed at about
   60 ohms. Found only by measuring the tracks. Fixed (below).
4. The pair findings' action said how to couple the pairs in a layout
   (`"diff_pairs": true`); trial 5's fix, clear. Not followed: the trial
   is about the widths.
5. `route-kicad-board sisu-designer Sisu_SSE-9 s-route1 auto --strip`
   (the designer's placement): exit 1 after 8 min 30 s, 18 unconnected
   (KEYPAD_BACKLIGHT's three column nets again, GND, C66), pairs on by the
   rules: 9 found, 3 coupled; new errors `diff_pair_gap_out_of_range` 2,
   `diff_pair_uncoupled_length_too_long` 1, **`skew_out_of_range` 7**. The
   class widths stayed (the narrow step did not run): RF_ANT 0.1422,
   MODEM_ANT1 0.1102. **F** (5 min): MODEM_ANT1, 0.1102 mm because the
   designer runs it as a stripline on In1.Cu, went 55 mm on **B.Cu**, an
   outer layer, at the stripline's width; nothing in the rules ties a
   class to a layer, so no report can know. Open (below).
6. The skew action: "target net length 43.4413 mm (from MODEM_USB_N) ...
   verdict.lengths: 3 not routed (MCU_USB_N, MCU_USB_P,
   unconnected-(U11-USB_ID-PadA14)): they are among the open connections
   ...; 7 under the minimum (MODEM_USB_PHY_N 8.96 mm, min 42.94, ...)".
   **F** (15 min): MCU_USB_N and _P were routed (1.8 mm each on F.Cu), not
   open; the third is a net of one pad. `drc-kicad-board` on the
   designer's board: 0 skew findings with MODEM_USB at 72 mm and the
   other USB nets at 10 to 30 mm. The rule's condition holds on In1.Cu
   and In4.Cu only, and KiCad counts only the copper there: the designer
   kept every USB net but MODEM_USB off those layers, so the group is one
   pair. The route put six more USB nets on In4.Cu and they joined the
   group of a 43 mm pair, which the meanders then tried to reach. Fixed
   the report (below); the route's share open.
7. `drc-kicad-board` on the designer's board: 0 errors, the 199 cosmetic
   findings (a cache hit).
8. After the fixes (`-c`, the reference's rates pinned so that both
   runs place alike): the same layout, exit 1 in 8 min 45 s, 24
   unconnected, `clearance` 2 (+3.3V and +1V1 0.10 mm apart under
   Power_3V3's 0.15 mm, on F.Cu: the router's, open), 6 pair findings.
   The new action: `the narrow-signal step routed 175 nets below their
   class's track width ...: RF_ANT 0.1422 mm -> 0.1 mm (8 nets ...);
   Power_3V3 0.3 mm -> 0.1 mm (5 nets ...); Power_VBATT_3V8 0.5 mm -> 0.1
   mm (2 nets: POWER/CHARGE_FUSED_IN_P, CHARGE_IN_P); RF_ANT_INT 0.1102 mm
   -> 0.1 mm (1 net: MODEM_ANT1); Default 0.15 mm -> 0.1 mm (159 nets
   ...)` with the rule to paste. **F** (5 min, a second finding): the
   battery and charger input nets of a 0.5 mm class written at 0.1 mm
   (their names are no rail's, so the step narrowed them as signals).
9. The lever the action names, two rules added to a copy of the
   project's `.kicad_dru` (`(rule "RF_ANT width" (condition
   "A.hasNetclass('RF_ANT')") (constraint track_width (opt 0.1422mm)))`,
   the same for RF_ANT_INT at 0.1102): exit 1 in 9 min 21 s, 20
   unconnected; the RF nets at 0.1422 mm and MODEM_ANT1 at 0.1102 mm (0.1
   only in the neck-downs at IC1's pads), the action now listing the two
   power classes and Default only, but its example rule was Power_3V3's
   under the words "controlled impedance". Fixed: the action tells a
   class no wider than Default's (likely an impedance) from a wider one
   (likely a current to carry) and takes its example from the first.
   MODEM_ANT1 went 14 mm on In2.Cu and 2 mm on B.Cu here: the layer
   question of step 5 again.

### MIDAS-MK2, the new placer default

1. `place-kicad-board midas MIDAS-MK2 m-place1 midas/layout.json`: exit 0
   in 56 s, every part placed, every constraint kept. **F** (10 min):
   stdout says nothing of the skeleton race; board-placer.json's
   `skeleton_race` holds 26 fields, of which agents.md names about half:
   `budget_ok` true, `work_seconds` 600 (the budget a seed, while
   board-placer.json's own `work_seconds` is 199 and `seeds_work_seconds`
   612), `skeleton_parts` 68, `absent` 129, `held` 68, `passives` 131,
   `skeleton_level` 3, `level` 3, `kept_plain` false, `seeds_plain` 0,
   `grown` 40, `joined_nets` 29, `room_mm2` 646, `unowned_mm2` 34. What a
   level 3 is, agents.md says only as "0 is full halos, 7 and later tight
   bodies". Readable with effort; nothing a reader of the verdict sees.
2. The same at `"total_seconds": 500`: exit 0 in 26 s. **F** (10 min):
   `skeleton_race` with `budget_ok` false, `work_seconds` 200,
   `kept_plain` true, `seeds_plain` 3, and every level 0, which
   agents.md's own key reads as "full halos". Nothing on stdout or in the
   verdict says that the race did not run, why, or that a
   `total_seconds` of 1250 would run it; the answer is in agents.md's
   paragraph (the 40 % share and the 500 s threshold), arithmetic left to
   the reader. Fixed (below).
3. `layout-kicad-board ... m-lay1 auto midas/layout.json` (1500): exit 0
   in 5 min 50 s, **complete** (156 of 156 connections), the skeleton race
   ran (68 parts at level 3, 129 passives at level 3, no fallback);
   `layout: placed (39 s, work 97 s, 10 seeds kept of 11 run, seed 4
   kept ...)` on stderr, no word of the skeleton; the verdict has `race`
   (the placement race's probes), nothing of the skeleton.
4. On `-b` (the summary fixed), `total_seconds` 500: exit 1 in 4 min 6 s,
   **1 unconnected** (SENSE_D at U206). The new stderr line said `placer:
   skeleton race: not run: each seed's placement had 200 s of work, under
   min_work_seconds (500 s), so all 3 seeds placed plain ... a layout's
   total_seconds of 1250 or more ...`. The verdict's actions named the
   hot spot, but not that the default budget places another way and
   completed. Fixed (below): one board's evidence, said as a lever.

### MIDAS-MK2, the release

1. `fab-kicad-board m-lay1/result MIDAS-MK2 m-fab1 --profile jlcpcb`:
   exit 0 in 8 s, 23 files: 11 Gerbers (four copper layers, both masks,
   pastes and silkscreens, Edge.Cuts), the job file, PTH and NPTH drill
   files with their maps, the zip (16 files), the position file, JLCPCB's
   CPL, the BOM, IPC-D-356, the STEP model; KiCad's DRC 0 errors, 4
   within J101's own footprint (not blocking, with the rule that accepts
   them); ERC and the schematic netlist skipped (no schematic beside the
   board: said on stdout).
2. Against `kicad-cli pcb export gerbers --check-zones` and `pcb export
   drill` of the same board: the four copper layers, both masks, both
   pastes and Edge.Cuts identical line for line but the dates; the
   silkscreens differ by the mask subtraction the release documents; the
   drill files hold the same 391 holes in 12 tools (ours split PTH and
   NPTH, as fabs want); kicad-cli's default adds 20 layers a fab does not
   take (courtyards, fab, user layers, adhesive). What it releases is what
   a fab wants.
3. **F** (5 min): "207 parts have no LCSC part number ... (an LCSC field
   on the symbol, then Update PCB from Schematic)": there is no schematic
   beside the result; the board's BOM reads footprint properties (`LCSC`,
   `LCSC Part`, ...), which the action did not say, nor that 207 parts are
   78 BOM lines (78 numbers to look up, not 207). Fixed (below).
4. **F** (3 min): agents.md says `--profile jlcpcb` writes
   `<id>-bom-jlcpcb.csv` beside kicad-cli's BOM; without a schematic it
   writes none (the `-bom.csv` already has JLCPCB's columns). Documented.
5. "check the rotations of polarised parts in the fab's placement
   preview": the same line on every release, no part named. Open.
6. Sisu's result (18 unconnected): refused, exit 1, the 18 items and the
   pair finding in `next_actions`, the first action naming `--input` and
   `--allow-findings`. Right. With `--allow-findings`: released, and the
   job file carries the designer's stackup (3313 prepreg 0.0918 mm under
   the outer layers, 1.022 mm, ENIG). **F** (5 min): nothing in the
   release says the board has controlled-impedance nets and needs the
   fab's impedance control on that stackup; the agent learns it from the
   `.kicad_dru`'s comment. Open (below).

## The result

| | Sisu_SSE-9 | MIDAS-MK2 release | MIDAS-MK2 placement |
| --- | --- | --- | --- |
| Runs | 2 descriptions, 3 layouts, 1 route, 1 DRC, 2 releases (in the next column) | 1 release, kicad-cli's export beside it | 2 place checks, 2 layouts |
| Best verdict | layout: 18 unconnected, 1 pair finding (20 with the RF classes kept by `opt` rules, pinned rates); route mode on the designer's placement: 18 unconnected, 10 pair and skew findings; per-layer USB width and disallowed layers honoured everywhere, the impedance and power classes narrowed in every layout until the `opt` rules | released: 23 files, the copper and drill identical to kicad-cli's | at 1500: complete; at 500: 1 unconnected |
| Agent's time | about 2 h 30 min | about 30 min | about 45 min |

## The verdict's advice, ranked by usefulness

1. `verdict.custom_rules` per constraint: the per-layer width as
   `no_effect` with the layers and the nets, the `disallow` as applied,
   the pair rules as unmodelled with the lever. Exact; the trial checked
   both against the board written.
2. The release's refusal and its first action (`--input`,
   `--allow-findings`), and the footprint's own findings with the rule
   that accepts them: right, nothing to add.
3. The pair findings' action (`"diff_pairs": true` in a layout; route
   mode on by the rules; why each pair fell back).
4. The hot spot's action with its pads, nets and parts (MIDAS's short
   layout: U206:7 on SENSE_D in conflict with SENSE_C).
5. Last: the skew action, which called two routed nets open and offered
   `--keep-nets` for nets the rule should not have grouped, and the
   release's LCSC line, whose lever needs a schematic the release did not
   have.

## Frictions, ranked by cost

1. **Impedance and power class widths narrowed in silence** (Sisu, about
   25 minutes and a script; the cost of missing it is a board that does
   not work): the narrow-signal step routed RF_ANT (0.1422 mm) and
   RF_ANT_INT (0.1102 mm) at 0.1 mm, the same as Default's signals, and
   Power_VBATT_3V8's 0.5 mm charger nets too; KiCad's DRC does not check a
   class's width, the verdict did not say it. Fixed:
   `verdict.narrowed_classes` and an action with the custom rule that
   keeps a class's width (measured: with it the RF classes keep theirs).
2. **A layer-limited skew rule read as an open connection** (Sisu route
   mode, about 15 minutes): nets routed off the rule's layers were "not
   routed ... among the open connections", a one-pad net too; the group
   KiCad checks is only the nets with copper on the rule's layers. Fixed
   in the report; the router's share open.
3. **What the skeleton race did, and why not** (MIDAS, about 20 minutes
   over two runs): no stdout line, no verdict field, zero levels that
   read as full halos, the threshold's arithmetic left to the reader.
   Fixed: `skeleton_race.summary`, the stderr line, `verdict.skeleton_race`
   and an action for a short layout left open.
4. **Nothing ties an impedance class to its layer** (Sisu, 5 minutes in
   the trial, by hand on every such board): MODEM_ANT1 (stripline width)
   routed on B.Cu. Open (below).
5. **The release's LCSC lever needs a schematic** (MIDAS, 5 minutes).
   Fixed.
6. **The description names no custom rule or class** (Sisu, 5 minutes):
   the class of a net and the project's rules are learned from the files.
   Open (below).
7. **No word of controlled impedance in the release** (Sisu, 5 minutes).
   Open.
8. **`--profile jlcpcb` without a schematic** writes no second BOM, which
   agents.md said it did (3 minutes). Documented.
9. **The rotation line names no part** (every release). Open.

## Fixed on the branch (each its own commit, with a test)

- The skeleton race says in one line what ran: board-placer.json's
  `skeleton_race.summary` and stderr's `placer: skeleton race: ...`
  ("not run: each seed's placement had 200 s of work, under
  min_work_seconds (500 s), so all 3 seeds placed plain, ... a layout's
  total_seconds of 1250 or more ..." or "ran: the kept seed placed its
  skeleton of 68 parts first (level 3) and its 129 passives around it
  (level 3); 12 of 12 seeds placed by skeleton (0 fell back ...)"); on a
  panel's copies or sub-boards stderr says it did not run; the layout's
  40 % is one constant (`board_layout::PLACEMENT_SHARE`)
  (`the_race_says_in_one_line_what_ran_and_why_not`).
- `verdict.narrowed_classes` (board-router.json `rules.narrowed`): per
  class the nets the narrow-signal step routed below the class's width,
  and where a class other than Default is among them an action telling
  a class no wider than Default's (likely an impedance) from a wider one
  (likely a current) and naming
  `(rule "RF_ANT width" (condition "A.hasNetclass('RF_ANT')") (constraint
  track_width (opt 0.1422mm)))` and `"use_narrow_signals": false`
  (`nets_the_narrow_signal_step_narrowed_are_named_by_class`).
- The release's LCSC action counts the BOM lines (`without_lcsc_lines` in
  fab-manifest.json) and says where the number goes by the BOM's source:
  a footprint property `LCSC` (or the other names the board's BOM reads)
  without a schematic, the symbols' field with one; agents.md on
  `--profile jlcpcb` without a schematic
  (`parts_without_an_lcsc_number_are_told_where_it_goes_by_the_boms_source`).
- `verdict.skeleton_race` (the summary) in the place check's and the
  layout's verdict, and for a layout left with open connections whose
  placement ran plain for want of budget an action naming the
  `total_seconds` that places by skeleton
  (`a_layout_left_open_without_the_skeleton_race_names_the_budget_that_runs_it`).
- A net whose copper lies only off a length or skew rule's layers is not
  checked by it nor of its skew group (`within`, note `routed off the
  rule's layers (...)`), a one-pad net has nothing to route, and the
  length action names such nets and says that a net routed onto the
  rule's layers joins the group
  (`a_net_routed_off_a_rules_layers_is_not_of_its_group`,
  `nets_routed_off_a_skew_rules_layers_are_not_called_open`).

## Left open, with a suggested design

- **Keep a class's width where the designer chose it.** The narrow-signal
  step narrows every signal net whose class width lies above the
  project's narrowest predefined width, ground, rails (by name), pours
  and nets with a rule's `opt` width excepted. It exists for a wide
  Default (ohdsp's 0.5 mm, its signals drawn at 0.135); on Sisu it also
  narrowed two impedance classes (0.1422 and 0.1102 below Default's 0.15)
  and two power classes (0.3 and 0.5 mm) whose nets' names are no rails'.
  Design: narrow only the nets of the Default class (or of classes of
  Default's width), every other class keeping its width as `ruled_widths`
  keeps a rule's (project_rules.rs, lower.rs's `narrow`); measure on the
  short tier and the boards whose layouts run the narrow step (Sisu, Hub,
  ohdsp's DSP board) that no connection is lost, since it is a router
  default. Until then the action above gives the rule per class.
- **The clearance the narrow attempt left.** Sisu's pinned layouts left
  `clearance` 2: +3.3V and +1V1 (both Power_3V3, 0.15 mm) 0.10 mm apart
  on F.Cu, the project rating clearance an error (no relaxed step). Not
  diagnosed; the router's area (the narrow attempt's and its
  continuation's classes).
- **Which layers a controlled class went on.** Nothing in KiCad's rules
  ties a class to a layer unless the designer writes a layer rule, and
  Sisu's designer did not (MODEM_ANT1, a stripline on In1.Cu, came back
  on B.Cu at the stripline's width). Design: `verdict.class_layers`, per
  class other than Default whose nets have no layer rule, the length its
  nets were written on each layer, with an observation where a class
  (or a net whose class width differs from Default's) runs on both an
  outer and an inner layer: "widths for one impedance differ by layer;
  check RF_ANT_INT on B.Cu (55 mm), or give it a `disallow track` rule
  for the layers it must not use". route_facts reads the written board
  already for the hot spots.
- **Keep a layer-limited skew group's other nets off its layers.** Route
  mode put six USB nets on In4.Cu, where the rule counts them, and they
  then could not be matched to the 43 mm MODEM_USB pair. Design: for a
  skew rule whose layers are a strict subset of the stack, raise the cost
  of those layers for the rule's nets that the designer's board (the
  input or `--reference`) routes without copper there, or let
  `length_matching` reroute a net that joined a group it cannot reach;
  the router's area (lengths.rs, the class layers in lower.rs).
- **Controlled impedance in the release.** fab-manifest.json could carry
  `stackup` (from the job file: layers, dielectric thicknesses and
  materials, finish) and, where the project's custom rules or classes set
  a per-layer width or a pair geometry (any `track_width` or
  `diff_pair_gap` rule with a layer condition, or a class other than
  Default narrower than Default), a `controlled_impedance` entry naming
  them, with the action "order impedance control on this stackup".
- **The description's rules.** `describe-kicad-board` could list each
  net's class name beside its width, and the `.kicad_dru` constraints as
  `custom_rules` already reports them after a run (`rules::lower` is
  pure; the description has the board and the project).
- **Rotations to check, by part.** The release's rotation line could name
  the polarised parts (diodes, LEDs, ICs, polarised capacitors by
  footprint) on the CPL, so that the preview check has a list.
