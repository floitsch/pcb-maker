# A fresh agent's eighth trial — 2026-11-08

One agent that had never used pcb-maker took three boards of classes no
earlier trial covered through the commands an agent would use, reading
only README.md, skills/pcb-maker/SKILL.md and docs/agents.md (and, where
agents.md sends the reader, constraints.md's `row`), and wrote down every
point of friction. Source code was read only after a board's first runs,
to fix what was found. Binaries: `build/bin/pcb-maker-x1938` (main
18f01076, the nightly's of 2026-11-08) for every first run;
`build/ux8/pcb-maker-b` (the branch's first eight fixes) for the
RP2040 route's second run, the row refusals and the bank's place checks;
`-c` (all fixes) for the bank's layout, the pinned pin-lines pair and the
reruns named below. KiCad 10.0.7, a shared machine (load 9 to 24 on 32
cores, another agent's pool beside; at most 12 of our threads,
`RAYON_NUM_THREADS=4` a run; no run waited in the memory ledger).

## The boards

| | OpenDrone OpenESC-20x20 4in1-mini | Twisted-Fields rp2040-motor-controller RP2040_base | OpenDrone OpenAIO-Whoop |
| --- | --- | --- | --- |
| Class | the netlist's start as an agent sees it: a bank of 22 bulk capacitors, steered with constraints.json | exact-fit 0.4 mm QFN rows (two RP2040s): the lattice lines at the pin rows, in route mode and in a layout | HDI 1+6+1, where blind and micro vias are the designer's way out (the `blind_vias` lever's documented use) |
| Why this board | placer.md names it as the board the netlist's start loses on: its 22 1206 capacitors (CL32 to CL55, +BATT and GND) start at one spot and the designer packs them in one block; nightly layout 88 unconnected | the nightly's designer tier decided the lines at the probes on it (U15, U19), and router.md measured it (20 -> 5 open) | the expansion intake's board with the most real spans (2479 micro F.Cu-In1.Cu and In6.Cu-B.Cu, 465 buried In1.Cu-In6.Cu; router.md "Blind, buried and micro vias") |
| Layers, outline | 6, the designer's 31.2 x 33 mm frame (a hollow fixed footprint U4 draws it), 2 oz outer copper | 4, 94.4 x 72.8 mm | 8, 30.3 x 30.3 mm |
| Parts, nets | 191 (189 movable, stacked), 152 | 325, 186 | 249, 178 |
| What it tests | `--constraints-template`, the place check, a `row` with `columns` for the bank, the layout, a fab release with `--allow-findings` | `route-kicad-board --strip` on the designer's placement, `inspect-routing-run --reference`, layouts with and without `"pin_lines": true` | route mode with `--strip`, then `"blind_vias": true`; `dead_pads[].wall` |
| Started from | the nightly's task input (parts stacked, U4 and J1 fixed, U4 hollow) | the designer's board (route mode) and the nightly's task input (layouts: 230 fixed parts) | the designer's board (benchmarks/expansion's copy in build/via1/boards) |

The classes, against trials 4 to 7: no earlier trial steered the
netlist's start, met a decided pin-lines rung, routed an HDI board or a
board whose designer drew supplies wider than any class (the rp2040's
VMOT 2 mm, its class VMOT naming no net).

## Log

Times are wall clock. **F** marks friction: a guess, a retry, or reading
something other than the user documentation.

### 4in1-mini, the bank

1. `describe-kicad-board mini 4in1-mini` (0.08 s): six layers, 191
   footprints of which 189 sit at one point, `outline.from_footprints`
   true (U4's frame), one class Default 0.09 mm with `layer_widths_mm`
   F.Cu 0.16, B.Cu 0.16, the custom rule "2 oz outer copper, track width"
   `applied`. `--constraints-template`: 4 `near` entries (C86, C42, C1,
   C43), nothing of the 22 1206 capacitors.
2. `place-kicad-board ... mini/layout.json` (the task's constraints):
   exit 0 in 46 s, every part placed, skeleton race ran (skeleton level
   7, passives level 9), `room.utilization` front 1.27, `tight_bodies`
   true. **F** (10 min): nothing says why the placement looks as it does.
   Rendered both boards with `kicad-cli pcb export svg` to see it: the
   designer's 22 capacitors stand in one block of four lines in the
   middle with the MOSFETs around it at the motor pads; ours stand singly
   around the edge, the MOSFETs in the middle. agents.md says nothing of
   the netlist's start (placer.md, not a user document, says the start
   puts all 22 at one spot).
3. `layout-kicad-board ... auto mini/layout.json`: exit 1 after 9 min 47
   s, **89 unconnected** (the nightly 88). **F** (10 min, and harmful if
   followed): the action "classes went on layers their width is not for
   ... Default has widths per layer from the custom rules (B.Cu 0.16 mm,
   F.Cu 0.16 mm) but went on layers they give it no width on (In1.Cu
   173.2 mm, ...): a width for one impedance differs by layer. Keep such a
   class on the layers its width is for with ... (constraint disallow
   track)": the rule is the 2 oz outer copper's minimum (its comment says
   "Inner layers stay at 1 oz and keep the standard"); following it keeps
   every net off the four inner layers. Fixed (below). The decoupling
   action named no pin (the bulk capacitors are on no IC's supply pin);
   28 `hot part` lines on stdout, one per MOSFET.
4. Steering the bank as constraints.md's `row` with `columns` says,
   `"row": [{"parts": ["CL*"], "pitch_mm": 1.9, "row_pitch_mm": 4.03,
   "columns": 6}]` (the designer's pitches): **F** (20 min, six
   refusals): "at pitch_mm 1.9 the pads of CL32 and CL39 lie on each other
   (their bodies are 4.60 mm along x): give a pitch_mm of at least that".
   CL39 is in the second line, so the setting was row_pitch_mm, and 4.60
   mm is the parts' length in their own frame, not along the board's x.
   Following the message (axis y and a rotation, then pitch 2.3, 2.35,
   2.4, 3, 4) every pitch was refused with "bodies are 2.30 mm along y",
   until 5, which the region refused. Fixed (below).
5. A `region` the size of the designer's block for `CL*` instead: exit 1
   after 3 min 49 s, 10 parts without a place, and "constraints missed:
   region CL36 (0.10 mm), ..." for a hard constraint. **F** (5 min).
6. Read the refusal again and guessed the lines: axis x, pitch 1.9,
   row_pitch 4.2, columns 6, the first part placed at the designer's
   spot (`place` with `origin` absolute): placed, exit 0 in 2 min 34 s.
7. Its layout: exit 1 after 11 min, **69 unconnected** (89 without the
   row) and **16 new clearance errors** (GND and +BATT 0.10 mm apart
   under the 0.16 mm outer rule). **F** (5 min): every part of a row takes
   the first part's angle, so two lines face with opposite nets, and the
   row check took only pads on pads; the action said "move those nets'
   parts apart" for parts of one row the agent wrote. Fixed (below).
8. `fab-kicad-board m-lay1/result ... --profile jlcpcb --input mini`:
   refused (89 unconnected), the first action naming `--allow-findings`.
   With it: released, 25 files. **F** (5 min): "order impedance control
   on this stackup ... for the classes Default" (the 2 oz floor again, an
   order option that costs money), and nothing of the 2 oz outer copper
   the stackup holds (F.Cu and B.Cu 0.07 mm), which the order must say.
   Fixed (below).
9. After the fixes (`-c`): the template writes the bank's row itself
   (`1 row (a bank of parts on two nets)`, 6 columns at 1.9 and 4.26 mm,
   axis x); the old pitches give "the pads of CL32 and CL39 lie on each
   other (lines 1 and 2 of the grid): turned as the row's first part is
   (90 degrees), they need 4.26 mm along the board's y (the pads' span and
   0.16 mm between pads of two nets), so row_pitch_mm must be at least
   that (now 4.03)". The template's row with the first part at the
   designer's spot places (2 min 30 s); without a place the placer leaves
   3 parts without a place (Rsense2, CL32, Q23). Its layout (measured
   rates): **103 unconnected**, no clearance error. With 69 at 4.2 mm and
   89 without a row (one layout each, the placements differing), the row
   is no measured gain on this board: the template writes it as a `bank:`
   comment to paste, not as an entry (`-d`).

### RP2040_base, the pin rows

1. `describe-kicad-board rp-designer RP2040_base`: four layers, 325
   parts, 186 nets, classes CAN, GND (0.6 mm), USB_DIFF_90, VMOT (2.0
   mm), low_power (0.6 mm), each with **0 nets**. **F** (5 min): read the
   `.kicad_pro` to learn that `netclass_assignments` is null and
   `netclass_patterns` empty, so every net is Default (0.2 mm); the
   designer drew VMOT at 2 mm and +5V at 1 mm by hand.
2. `route-kicad-board rp-designer RP2040_base r-route1 auto --strip`:
   exit 1 after 11 min, **24 unconnected**, 273 vias. **F** (10 min): the
   lines at the pin rows were decided at the probes ("probe pours=tracks
   with lines at the pin rows: 59 nets unfinished ...: it leads (74
   without)"), which only stderr said: not the verdict, not
   board-router.json's `attempts`. agents.md promised the run log, and an
   agent reads the verdict. Fixed (below). **F** (15 min and a script):
   VMOT routed at 0.2 mm on 94 segments, +5V at 0.2 (the designer's 2 and
   1 mm); KiCad's DRC checks no width above the minimum, the verdict said
   nothing. A motor supply at 0.2 mm burns. Fixed (below).
3. `inspect-routing-run r-route1 --reference rp-designer`: 23 nets,
   every one but two the designer routes (`reference.connected` true),
   most open at U15's pads. Clear and quick (1 s); nothing to add.
4. `layout-kicad-board rp RP2040_base r-lay1 auto rp/layout.json`: exit 1
   after 10 min, **36 unconnected** (the nightly 38), hot spot 0 at U15:38
   and U15:40. No line of the pin rows in the log. **F** (5 min):
   agents.md says route mode decides the lines; for a layout it does not
   say, so the agent guessed `"pin_lines": true`.
5. The same with `{"pin_lines": true}` as the router config: exit 1
   after 9 min, **6 unconnected** (first route 7 open against 38).
   Pinned at the reference's rates (`PCB_WORK_RATE=3.15e6,150e6`, `-c`),
   both with one placement digest: **38 unconnected without, 4 with**
   (first route 40 open against 4). The 4in1-mini pinned the same way: 88
   either way, the same board. The arm without the lines now ends with
   "the hot spot lies at the pin rows of U19 (0.4 mm pitch): a layout
   routes them on the regular lattice unless the router config says
   \"pin_lines\": true ...".
6. After the fixes (`-b`): stdout says `pin lines: decided at the probes:
   the probe pours=tracks left 79 nets unfinished with pads at pin rows
   the regular lattice misses, 65 with lines at those rows, so the lines
   stayed on ...`, the same in `verdict.pin_lines`, and each attempt has
   `pin_lines: true`; 26 unconnected. `verdict.designer_widths` was still
   empty: the designer's board (KiCad 8) names its tracks' nets by code,
   which the class view did not read (trial 7's `designer_layers` had the
   same blind spot on such boards). Fixed; on `-c` (20 unconnected):
   `verdict.designer_widths` lists 21 nets, 12V_switched, VMOT and SW 2
   mm on the designer's board, +5V and +12V 1 mm, +3V3 0.8 mm, ... all at
   0.2 mm here, and the action: `"net_classes": [{"name": "12V_switched
   width", "nets": ["12V_switched"], "track_width_mm": 2}, {"name":
   "VMOT", "nets": ["VMOT"], "track_width_mm": 2}]; ... the project
   defines the classes VMOT (2 mm), GND (0.6 mm) but assigns them no net
   ...`. The route with those classes was not run (the machine was
   another pool's by then).

### OpenAIO-Whoop, HDI

1. `describe-kicad-board whoop-designer OpenAIO-Whoop`: eight layers, 71
   custom constraints, among them "HDI 1+6+1: microvias F.Cu-In1.Cu and
   In6.Cu-B.Cu only ... micro_via: the router lays through vias only".
   **F** (3 min): no word of `blind_vias`, which agents.md documents for
   exactly this board.
2. `route-kicad-board ... w-route1 auto --strip`: exit 1 after 6 min,
   **144 unconnected**, 69 through vias on a board whose rules say "no
   through vias". **F** (10 min): board-router.json's
   `rules.via_spans_designer` lists the designer's 2944 vias by span, and
   nothing in the verdict says the router left them out by default and
   that `"blind_vias": true` takes them. Fixed (below).
3. With `{"blind_vias": true}`: exit 1 after 5 min 30 s, **114
   unconnected**, 70 micro and 15 buried vias, 55 through. `dead_pads`:
   AE2.3 (GND, the antenna's pad at the board's edge), `wall` "layer 7: a
   0.100 mm track may end at (100.140, 114.830); its stubs to the nearest
   nodes the class's 0.200 mm track may use: ... by the board's edge".
   **F** (5 min): "layer 7" is the router's index (B.Cu); the wall says
   what blocks (the edge) but no lever; the designer's board has vias in
   AE2's pads 1 and 3. The layer name fixed; the lever open (below).
4. After the fixes, the default route again: on `-c` kicad-cli 10.0.7
   crashed (SIGSEGV) refilling the routed board's zones, exit 4 with
   "report the error with the input" (**F**, 10 min: KiCad's crash, as
   once in router.md's measurement of this board; fixed, the action now
   says so); on `-e` the run completed its check: 144 unconnected and the
   action "the board has 2944 vias that pierce only some layers (micro
   In6.Cu-B.Cu 1240, micro F.Cu-In1.Cu 1239, buried In1.Cu-In6.Cu 465:
   rules.via_spans_designer in the report), and this run laid through
   vias only (blind_vias is off by default): add \"blind_vias\": true to
   the router config ..."

## The result

| | 4in1-mini | RP2040_base | OpenAIO-Whoop |
| --- | --- | --- | --- |
| Runs | 2 descriptions, 4 templates, 14 place checks, 5 layouts, 2 releases | 2 descriptions, 3 routes, 1 inspection, 4 layouts | 2 descriptions, 4 routes (one ended by kicad-cli's crash) |
| Best verdict | layouts: 89 unconnected without a row, 69 and 103 with the bank as a row (the 69 with 16 clearance errors), 88 pinned with and without `pin_lines` | layout with `pin_lines`: 6 unconnected (36 without); route mode 24 | route with `blind_vias`: 114 unconnected (144 without) |
| Agent's time | about 1 h 30 min | about 1 h | about 30 min |

## The verdict's advice, ranked by usefulness

1. `inspect-routing-run --reference` on the route: per open net the
   designer's route, in a second.
2. The release's refusal with `--allow-findings` and the findings by
   item.
3. The hot spot's action with its pads (U15:38, U15:40), which pointed at
   the fine-pitch row the lines then served.
4. Last: the `class_layers` warning on the mini (an impedance read into
   a process floor, advice that keeps every net off the inner layers) and
   the release's impedance line built on it.

## Frictions, ranked by cost

1. **A supply drawn wide by hand comes back at 0.2 mm, unsaid** (the
   rp2040, 15 minutes and a script; the cost of missing it is a burnt
   board): VMOT 2 mm and +5V 1 mm on the designer's board, its classes
   VMOT and GND naming no net. Fixed: `verdict.designer_widths` and an
   action with the `net_classes` to paste and the unassigned class.
2. **The bank's row refused at every pitch tried, then clearance
   findings** (the mini, 25 minutes, six refusals and an 11-minute
   layout): the refusal named pitch_mm for a pair of two lines and the
   parts' size in their own frame; the check let two lines of opposite
   nets 0.1 mm apart. Fixed: the setting and the pitch needed along the
   board's axis, the copper clearance between pads of two nets.
3. **The 2 oz floor read as an impedance** (the mini, 15 minutes; the
   advice, followed, empties the inner layers, and the release asks for
   a paid option): fixed, a `min` alone is no width per layer nor an
   impedance.
4. **Why the placement looks so, and the bank** (the mini, 10 minutes and
   a render): fixed in part: the template's `bank:` comment says why parts
   on two nets end up spread and holds the row to paste at pitches from
   the pads; not an entry, since the row measured no better.
5. **An HDI board routed with through vias, the setting unnamed**
   (the Whoop, 13 minutes): fixed, an action naming the spans and
   `"blind_vias": true`; the rule's note names it.
6. **The pin lines decided where only stderr says so** (the rp2040, 10
   minutes): fixed, `verdict.pin_lines`, stdout, `attempts[].pin_lines`.
7. **A layout's pin rows** (the rp2040, 5 minutes, a guess that paid
   off): fixed by an action where a layout's hot spot lies at a 0.5 mm
   row or finer without the lines; the default is open (below).
8. **Heavy copper unsaid on the release** (the mini, 3 minutes): fixed.
9. **A dead pad's layer by index** (the Whoop, 5 minutes): fixed; its
   lever open.
10. **No word of the threads** on a shared machine (every board, 3
    minutes): `RAYON_NUM_THREADS` documented.
11. **kicad-cli's crash read as ours** (the Whoop's rerun, exit 4 "report
    the error with the input"): kicad-cli 10.0.7 died of SIGSEGV refilling
    the routed zones, as once in router.md's blind-vias measurement. Fixed:
    the action says it is KiCad's, to run again, and where the routed board
    is.

## Fixed on the branch (each its own commit, with a test)

- Per-layer widths only from a `track_width` rule with an `opt` or a
  `max`; a `min` alone is a process floor
  (`a_minimum_width_on_some_layers_is_no_width_per_layer`), nor an
  impedance to order on the release (the same test).
- The row refusal names `pitch_mm` or `row_pitch_mm`, the pitch needed
  along the board's axis as the first part is turned, and keeps the
  copper clearance between pads of two nets
  (`a_row_whose_pitch_puts_pads_on_pads_is_refused`).
- `verdict.designer_widths` and its action, with the project's class of
  the net's name that holds no net; the class view reads KiCad 9 net codes
  (`nets_run_narrower_than_the_designers_are_named`).
- The blind-vias action and `rules.blind_vias`; the via-kind rule's note
  names the setting (`a_designers_vias_of_some_layers_name_blind_vias`).
- `verdict.pin_lines`, board-router.json's `pin_lines`,
  `attempts[].pin_lines`, stdout's `pin lines: ...` (the ladder tests'
  lines-probe and after-the-leader cases); the layout's action for a hot
  spot at a fine row (`a_layouts_hot_spot_at_a_fine_row_names_pin_lines`).
- Dead pads' walls by KiCad's layer names
  (`a_dead_pads_wall_names_kicads_layers`).
- The template's `bank:` comment with the row to paste
  (`a_bank_of_parts_on_two_nets_becomes_a_row`).
- The release's heavy-copper action (`heavy_copper_is_named_for_the_order`).
- kicad-cli's crash told apart (`a_kicad_crash_is_told_apart_from_ours`).
- agents.md (the threads, the cookbook's bank, `classes`, the template,
  `pin_lines`, `blind_vias`, the verdict's `designer_widths` and
  `pin_lines`, dead pads, the release), constraints.md's `row`, SKILL.md,
  the changelog, handover open problem 8.

## Guards

No router or placer behaviour changed: the fixes are reports, verdict
actions, the template's comment, a release action and the row refusal.
The row refusal is the one input-side change (a row whose members' pads
of two nets come closer than the copper clearance is now refused); no
task of the tiers (tasks/v7: the sweep, the short, quick and designer
tiers) and no committed fixture has a `row` in its constraints, and the
schematic tier's key-matrix starters set rows at key pitches of 17 to
19.05 mm, far from it, so no tier's input changes and the short tier was
not rerun. The router's fingerprints (f1s at 1 and 4 threads, the route
fingerprints) are the golden ones in `scripts/check.sh --fast`, which
passes; rustfmt clean. The one measured behaviour, pin lines in a layout,
stays a setting the action names: making it the layout's default is left
open for the short and designer tiers to decide.

## Left open, with a suggested design

- **Pin lines in a layout by default.** RP2040_base's layout, one
  placement at the reference's rates: 38 unconnected without the lines,
  4 with; the 4in1-mini 88 either way. A layout's first route and moves
  have no probes to decide at. Design: the layout's first route probes
  the lines as route mode's ladder does where the placement leaves pads
  at rows the regular lattice misses (`pin_lines::missing` on the placed
  board), or turns them on there outright; a router default, so the short
  tier and the designer tier (route mode, scale 0.647) decide, with
  RP2040_base and the boards router.md's census names.
- **A row whose lines turn by half turns.** A row turns every part alike,
  so a bank's lines face with two nets and keep the copper clearance
  (4.26 mm on the mini); the designer turns every other line, so facing
  pads share a net (4.03 mm). Design: `"alternate": true` on a row (each
  line's follower angle 180 on odd lines), the refusal's pitch then the
  same-net one.
- **The bank's place.** Without a `place` the template's row leaves 3 of
  the mini's parts without a place; with the designer's spot it places.
  The comment asks for a place; a suggestion (the centroid of the fixed
  pins on the bank's nets) is open, as is whether a bank helps routing at
  all (two layouts, 69 and 103 against 89).
- **A dead pad at the board's edge** (the Whoop's AE2.3, GND): the wall
  names the edge; the designer's board has vias in AE2's pads 1 and 3,
  and `via_in_pad` admits only enclosed pads. Design: where a dead pad's
  footprint is in `rules.via_in_pad_designer`, the wall's action names
  `"via_in_pad": ["AE2"]` once that admits such a pad (measure first).
- **A class that names no net, in the description.** The rp2040's VMOT
  and GND classes hold no net while nets of their names exist; the route's
  `designer_widths` says it, `describe-kicad-board` does not. Design: a
  `classes` row's `unassigned_nets` (nets of its name in Default).
- **The wider classes' cost.** The rp2040 route with the action's
  `net_classes` (VMOT and 12V_switched 2 mm, ...) was not run: wider
  tracks take room on a board left with 20 open connections.
