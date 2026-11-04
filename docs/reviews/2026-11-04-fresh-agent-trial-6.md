# A fresh agent's sixth trial — 2026-11-04

One agent that had never used pcb-maker took three boards of classes no
earlier trial covered through the commands an agent would use, reading
only README.md, skills/pcb-maker/SKILL.md and docs/agents.md (and the
pages agents.md sends the reader to: constraints.md, config-reference.md
for `length_maxima`, router.md's length rules), and wrote down every point
of friction. Source code was read only after a board's first runs, to fix
what was found. Binaries: `build/bin/pcb-maker-ux11-a` (main ac8cf07d) for
the first two imports, `-b` (main b2518a21, x1899) for eurorack-pmod's
place checks and first two layouts and the firestarter's layout and
first route, `-c` (this branch's first five fixes) for the six-layer
import and the firestarter's second route, `-d` (nine fixes, rebased
on b6b4e6a5, x1900) for the laptop's first layout and the aborted
six-layer one, `-f` (eleven) for the last two layouts. KiCad 10.0.7, a shared machine
(load 20 to 30 on 32 cores, 7 other layouts in the memory ledger; the
measured search rate 0.37 to 0.93 of the reference's).

## The boards

| | apfaudio eurorack-pmod r3.5 | hstarmans firestarter `compute` | byrantech laptop `power` |
| --- | --- | --- | --- |
| Class | schematic-only start | length rules | from scratch with an outline |
| Layers, outline | the designer's: 6, 31.7 x 110 mm (a eurorack module); ours 4, then 6, on a 32 x 110 mm outline with 2 mm round corners | 4, the designer's 88.4 x 63.6 mm | 4; ours: 70 x 50 mm, round corners of 3 mm and an 8 x 4 mm slot, then 78 x 56 mm as the tool advised (the designer's 64 x 45.4) |
| Parts, nets | 151 from the schematic (6 sheets; the designer's board 155), 147 nets | 125, 117 | 179, 118 |
| What it tests | the import from a hierarchical schematic whose fp-lib-table names another machine's paths, eight vertical jacks in a column, a board too full for one side | `.kicad_dru` `DVP_Bus_Length_Matching`: length 25 to 27 mm on ten nets (DVP_Y2 to Y9, PCLK, XCLK) from the camera FPC J2 to the ESP32-S3 module U2 | an authored outline, starter constraints from `--constraints-template`, a USB-C receptacle and a PicoBlade on edges, an ESP32-S2-MINI antenna at an edge |
| Started from | `benchmarks/real/external/github-sch` (the schematic), the designer's board only for footprints (`--footprints-from`) | the expansion tier's quick run (`benchmarks/expansion/data/runs/quick/boards/hstarmans_firestarter_base_compute*`: the designer's routed board, and its stripped copy for route mode) | the layout sweep's task input (`build/github-layout-v21/byrantech__laptop__power/source`: parts stacked), its constraints discarded |

## Log

Times are wall clock. **F** marks friction: a guess, a retry, or reading
something other than the user documentation.

### eurorack-pmod, from the schematic

1. `import-kicad-netlist .../eurorack-pmod-pcb.kicad_sch pmod
   eurorack-pmod-pcb`: exit 1 in 1 s, 132 footprints, 19 `missing`, one
   line per part: "D27 (LLP1006-2M:VISHAY-LLP1006-2M not found: no
   library "LLP1006-2M" in the fp-lib-table files, the project's .pretty
   folders, KiCad's own or its add-on libraries)", the same for eight
   jacks (`AudioJacksFixed:pj324m`) and the FFC socket J3. **F** (10 min):
   the project's fp-lib-table does name all three libraries, at
   `/home/seb/dev/eurorack-pmod/hardware/libs/...`, the designer's
   machine; two of them are in the checkout at `../libs/custom` and
   `../libs`. The message said the table had no such library.
2. Copied the two `.pretty` folders into `pmod/` (the message: "put the
   .pretty folders in the project") and imported again: the same 19.
   **F** (5 min): "the project" is the schematic's folder, which an agent
   working on a read-only checkout cannot write; the output directory was
   not searched.
3. `--footprints-from` the designer's board: exit 0, 151 footprints (the
   designer's 155 less 4 board-only marks), `footprints_from_board` the two
   holes the schematic gives no footprint, the eight LLP1006 diodes, the
   eight jacks and J3 (with the branch's library fix: the first ten
   only), `drifted_footprints` U1 (the QFN's thermal vias: 47 pads in
   the library, 59 on the board). `edge_connectors` empty (right: the
   PJ-324M jacks stand on the board's face, the panel's side). Then
   `describe-kicad-board`: `copper_layers` F.Cu and B.Cu. **F** (5 min):
   the designer's board has six; the import says nothing of it. Imported
   again with `--layers 4` (a cheaper board; the designer's six came
   later).
4. The import's `constraints.json`: `move_all` and `"outline": {}` only;
   `describe-kicad-board --constraints-template` on the imported board:
   20 `near` entries for the decoupling. **F** (3 min): two starters for
   one board, and the import's has none of the decoupling.
5. Constraints as the user would state them: 32 x 110 mm with 2 mm
   corners, the eight jacks in a column on the left at the panel's
   13.79 mm (`row` of J4 to J11, `"axis": "y"`, J4 `place`d with
   `"angle": 90`), the PMOD headers J1 and J2 `edge` right, opening
   outwards, the 20 `near` entries. `place-kicad-board`: exit 2 in 1 s,
   "the row of J4 (8 parts) cannot fit the outline: the row is 12.00 x
   114.73 mm (turned as it may be), the outline 32.00 x 110.00 mm".
   `"rotation": [{"part": "J4", "angle": 90}]` added: the same. **F**
   (10 min): the row's parts keep the angle they have in the stack (the
   jacks' 18.2 mm side along the row) whatever the first part's angle;
   nothing said how to turn them. Gave up the row for eight `place`
   entries at x, y, 90 degrees. (Found later by trying: `"axis": "x"`
   with the rotation is the column, the axis being the first part's
   frame; and that layout then failed, step 10.)
6. Place check: exit 1 in 77 s, 8 `near` missed by up to 8.43 mm (C17 at
   the EEPROM U5). `room.utilization` front 1.208, back 0.0. Next action:
   "constraints missed (...): relax them or give the parts room". **F**
   (10 min): nothing said the front was 121 % full with the back empty;
   the designer's board has 27 parts on the back (the jacks' ESD diodes
   and varistors, four transistors and their resistors, the FFC socket).
   Put those on the `back`.
7. Place check: exit 1 in 210 s (0.38 of the reference's rate), 7 `near`
   missed by 0.35 to 2.58 mm, front 105 %. Accepted.
8. `layout-kicad-board ... p-lay1 auto pmod/layout.json`: exit 1 after
   296 s wall (the measured rate 0.37): **26 unconnected**, first route
   34 open, 9 of 69 trouble exchanges kept. Next actions in order: open
   connections, the hot spot (U7, RN2, U3, R10, RN13, its open link
   "RN12:3 to U7:16, 9.8 mm apart ... a near constraint ... to paste"),
   the rate (third: "about 2.7 times the wall"), constraints missed, the
   GND pour in 11 pieces on In1.Cu with a `keep_whole` entry, decoupling
   50 % within 3 mm.
9. Followed the rate action: `PCB_WORK_RATE=3.1500e6,1.5000e8`, 983 s
   wall: **22 unconnected** (first route 26 open). The rate was not this
   board's cause.
10. The designer's six layers, on the fixes' binary: the import of `-c`
    takes the `--footprints-from` board's count by itself
    (`copper_layers` 6, `copper_layers_from` "the --footprints-from
    board's"), and the row as found in step 5 (`"axis": "x"`, J4's
    rotation 90; its place check: 3 `near` missed by 0.16 to 0.38 mm,
    the jacks in a column at 13.79 mm). The layout warned 21 times "fixed
    parts J5 and J6 overlap ... their pads cannot both be connected" (the
    row's members in the import's stack, not fixed), placed in 59 s and
    then ended **exit 4**: "the row of J4 (8 parts) cannot fit the
    outline: the row is 12.00 x 114.73 mm". **F** (15 min): the layout
    lowers its own placement again, and that lowering found the row along
    the board's x axis, as if it had never turned. Both fixed (below).
11. Again on `-d` (both fixed), pinned rates: placed in 42 s, then "71
    open after the first route: no moves". **F** (20 min): the placed
    board had J4 where its `place` put it (the outline's top, turned 90
    degrees) and J5 to J11 running up from it, 96 mm beyond the outline:
    a row runs along its axis in its first part's frame, and turned 90
    degrees that is towards smaller y. The place check of step 10 had
    accepted it (3 `near` missed, nothing else). Fixed: such a `place` is
    refused, with where the row reaches; the cookbook's jack column puts
    the row's first part at the end it leaves from (J11 first, at the
    bottom). Killed the run.
12. On `-f`, J11 first at the bottom, six layers, pinned rates: exit 1
    after 592 s, the jacks in their column, **24 unconnected** (first
    route 40 open: no moves ran), 9 `near` missed by 0.08 to 5.16 mm. The
    constraints-missed action now said "the front side is 122 % full and
    the back 8 % ...: put the parts the design allows there on the back
    ... or give the board the room (room.smallest_outline_mm 36.5 x 125.5
    mm)": on a eurorack module's fixed panel width, the back. Six layers
    did not help (24 against 22 on four); the board's limit here is the
    front's area, which the designer met with a denser placement and six
    layers. Stopped there.

Agent's time on the board: about 2 h 30 min, 50 min of it waiting on
runs.

### firestarter `compute`, a length rule

1. `describe-kicad-board fs compute`: 4 layers, 125 footprints, 117
   nets; the ten DVP nets each from J2 (Hirose FH12 24-pin FPC) to U2
   (ESP32-S3-WROOM-2, locked). `--constraints-template`: 2 `edge` (J1
   USB-C and J4 barrel jack, bottom), 47 `near`. **F** (5 min): neither
   the description nor the template's comment mentions the length rule;
   agents.md's `length_matching` paragraph says what the router does
   with it, nothing of the placer. config-reference.md has
   `length_maxima` (on): the placer keeps such a net's pins within 0.8 of
   the maximum. The place check (exit 1, 4 `near` missed) says nothing
   of it either.
2. `layout-kicad-board fs compute f-lay1 auto fs/layout.json`: exit 1
   after 897 s (0.41 of the reference's rate measured): **5 unconnected**
   (+3.3V, GND, PHOTODIODE, SPI_SI, Y_DIR), **5 `length_out_of_range`**.
   stdout's custom-rules line says what the rule became ("a meander after
   the route below the minimum, over the maximum the net routed again the
   short way and checked, no hard bound"). The length action: "1 over the
   maximum by the route (DVP_Y9 28.83 mm, max 27.00) ...; 4 under the
   minimum (DVP_Y3 17.46 mm, DVP_Y4 11.24, DVP_Y5 8.83, DVP_Y8 21.72):
   no straight track of the route had room for the whole meander". **F**
   (15 min): what to do about a net 16 mm short? The placer kept the
   pins well within 0.8 of the 27 mm maximum and put J2 so near U2
   that the route is a third of the minimum; it knows no minimum, which
   no page says, and the action names no lever. The hot spot's open
   link (SPI_SI, U2:4 to U5:17, 40.8 mm) is between two locked parts:
   said.
3. Route mode on the designer's placement (`route-kicad-board fs-src
   compute f-route1 auto --reference fs`, the stripped copy): exit 1 in
   151 s, **0 unconnected**, **7 `length_out_of_range`**: 6 under the
   minimum (DVP_Y2 12.93 mm, Y5 11.48, Y6 13.97 with 0 to 10 mm of
   meander each: "no straight track with room for all of it"), DVP_Y9
   27.74 mm over. The designer's own board meets all ten with KiCad
   tuning patterns. **F** (20 min): the documented lever is to route the
   critical nets by hand and let route mode keep them; an agent has no
   hands in KiCad, but has the designer's tracks, and no command keeps
   some nets' tracks and strips the rest. A 20-line script did it (249
   segments, arcs and vias of the DVP nets kept, 1442 others stripped).
4. `route-kicad-board fs-keep compute f-route2 auto --reference fs`:
   **exit 0 in 256 s, complete**: "kept: the board's 169 track segments,
   66 arcs and 14 vias on 12 nets", 105/105 connections, "lengths: 10 of
   10 nets within their bounds". `strip-kicad-tracks --keep-nets` does it
   now (below).

What can an agent do about the lengths? In route mode: keep a routed
version of the bus (the designer's, or one routed by hand) and route the
rest; the router's own meanders fall short where the straight runs are
short. In a layout: nothing that reliably works; an `apart` between the
bus's parts or fixing J2 where the designer has it would give the route
length to start from, but the designer's own placement in route mode
shows the meanders still fall short. Agent's time on the board: about 1 h
15 min, 25 min of it waiting.

### laptop `power`, from scratch with an outline

1. The task input's parts are stacked; its `constraints.json` (5 fixed
   parts, J5 at the right edge) discarded. `describe-kicad-board
   --constraints-template`: 1 `edge` (J1, the USB-C receptacle, `any`),
   29 `near`; "Other connectors (headers, sockets: no edge entry): J3,
   J4, J2, J5". **F** (3 min): U5 is an ESP32-S2-MINI-1 module whose
   antenna wants the board edge; the template's comment names "keepouts
   (antennas, ...)" only in general.
2. Authored: `"outline": {"width": 70, "height": 50, "corner_radius":
   3, "cutouts": [{"x": [31, 39], "y": [21, 25]}]}`, J1 `edge` right and
   J3 (the battery PicoBlade) left, both flush and opening outwards, U5
   `edge` bottom with `"overhang": {"edge": "bottom"}` (the cookbook's
   "WiFi antenna over the board edge"), the template's 29 `near`. Place
   check: exit 2 at once, "edge constraint for U5: overhang needs a
   keepout zone in the footprint (the part that sticks out)". **F**
   (5 min): the footprint has none; the refusal said what, not what to do
   instead. U5 `edge` bottom flush with `"rotation": [{"part": "U5",
   "angle": 180}]` (the antenna end down).
3. Place check: exit 1 in 114 s, 8 `near` missed (the module's five
   capacitors by 3 to 5.8 mm), front 84 % full, back 0 %, smallest
   outline 76.5 x 54.5 mm; the action said only "relax them or give the
   parts room" (on `-b`; on `-d` it names both sides' shares, the `back`
   lever and the outline). Took the tool's outline, rounded up: 78 x 56
   mm, the slot moved with it.
4. `layout-kicad-board lp power l-lay1 auto lp/layout.json` (`-d`):
   exit 1 after 693 s (0.70 of the reference's rate): **7 unconnected**
   (BATDRV_N, CELL_BATPRES, LODRV1, SCL, SRN, SRP, VSYS: all at U4, the
   BQ25713 charger's QFN-32, the hot spot "U4 (front, over it), C45
   ..."), 18 of 35 constraints missed by 0.05 to 2.76 mm (the eleven
   capacitors the template put at U4's pins among them), and **a new
   `malformed_courtyard`** on U5. **F** (10 min): the ESP32-S2-MINI-1's
   courtyard is open as its library draws it, and the designer's board
   has the same finding (`baseline-drc.json`); without a reference the
   layout counts it. Fixed: a footprint's own malformed courtyard is
   `footprint_findings`, with the exclusion.
5. `inspect-routing-run l-lay1`: the seven nets, each one link from U4 to
   a neighbour (Q8:4 to U4:21 for BATDRV_N), none routed. The levers
   offered: the hot spot's parts. Loosened U4's two tightest capacitors
   to 3 mm and laid out again (`-f`): exit 1 after 560 s, **1
   unconnected** (LODRV2 at U4, the hot spot again), first route 9 open,
   the `malformed_courtyard` (`-f` predates its fix; `drc-kicad-board
   l-lay2/result power ... --input lp` on the branch's head sets it apart
   as U5's own, with the exclusion), 25 of 35 constraints kept. One
   connection from a legal board; the levers left are the hot spot's
   (move one of C47, C40, C43 off U4) or more seeds.

Agent's time on the board: about 1 h 10 min, 25 min of it waiting.
The tool's advice got the board to one open connection without reading
code: the refusals named what was wrong, `room` gave an outline that
fits, the hot spot named U4's neighbours; what was missing was the
overhang's alternative and the full front in the action (both fixed).

## The result

| | eurorack-pmod | firestarter | laptop power |
| --- | --- | --- | --- |
| Runs | 5 imports, 5 place checks (2 refused), 5 layouts (1 failed with exit 4, 1 killed) | 1 place check, 1 layout, 2 routes | 3 place checks (1 refused), 2 layouts, 1 inspection |
| Best verdict | 22 unconnected (4 layers, pinned rates; 26 measured at 0.37; 24 on six layers), 7 `near` missed by up to 2.6 mm, no new DRC error | route mode with the designer's DVP tracks kept: **complete**, 10 of 10 nets within 25 to 27 mm; the layout 5 unconnected and 5 `length_out_of_range`, route mode without the kept bus 0 unconnected and 7 | 1 unconnected (LODRV2), no new DRC error on the branch's head (the open courtyard is U5's own), 10 `near` missed by up to 1.5 mm, on 78 x 56 mm with round corners and a slot |
| Agent's time | about 2 h 30 min | about 1 h 15 min | about 1 h 10 min |

## The verdict's advice, ranked by usefulness

1. `verdict.lengths` per net with its note (firestarter): over by the
   route or by the placement, under for want of a straight run, how much
   of the meander was added. Exact; what was missing was the lever.
2. The room numbers (`room.utilization` per side,
   `smallest_outline_mm`): the 121 % front of eurorack-pmod and the
   76.5 x 54.5 mm for the laptop power were right and decided both
   boards; they were only in the JSON, not in the action.
3. The custom-rules line on stdout, which now says what a length rule
   becomes (trial 4's complaint), and the hot spot's open link with its
   `near` to paste (eurorack-pmod: RN12:3 to U7:16) or "both fixed"
   (firestarter: U2 and U5 locked).
4. The import's `footprints_from_board` and `drifted_footprints`: what
   came from where, and the one footprint that differs.
5. The rate action (eurorack-pmod: 0.37 of the reference): followed, 26
   to 22 unconnected; right to come third here.
6. Last: the length minimum's "no straight track of the route had room
   for the whole meander" without a lever, and the import's "no library
   in the fp-lib-table files" for libraries the table names.

## Frictions, ranked by cost

1. **A length minimum the meanders cannot reach, and no lever**
   (firestarter, about 35 minutes and a script): the layout's placer
   knows only maxima and pulled the FPC within 9 mm of the module; route
   mode on the designer's placement left 6 of 10 short; nothing kept the
   designer's routed bus. Fixed: `strip-kicad-tracks --keep-nets`, the
   lever in the length action by mode, `length_maxima` in agents.md. The
   placer's and the meander's share open (below).
2. **A row of parts turned a quarter** (eurorack-pmod, about 45 minutes,
   two refusals, a layout that failed with exit 4 after its placement and
   one with seven jacks off the board): the members keep their stack
   angle; the first part's rotation turns the whole row, which then runs
   up the board from its first part; the layout's second lowering lost a
   quarter-turned row; a `place` that sent the row off the board passed
   the place check. Fixed: the refusal says how, the lowering keeps the
   row, a row sent off the board is refused, constraints.md, the
   cookbook and the SKILL say which way a turned row runs.
3. **The import's libraries** (eurorack-pmod, 15 minutes and two
   imports): a fp-lib-table naming another machine's paths read as "no
   library in the fp-lib-table files"; the `.pretty` folders put in the
   output directory were not searched. Fixed: the table's folder found by
   its last folders under the schematic's folder or its parents
   (`relocated_libraries`, 132 footprints to 141), the message names the
   table's folder, the output directory is searched.
4. **A side over full beside an empty one, unsaid** (eurorack-pmod 10
   minutes; laptop power): `room.utilization` 1.208 and 0.0, the action
   "relax them or give the parts room". Fixed: the action gives both
   shares, the `back` lever and the smallest outline.
5. **The import's layer count** (eurorack-pmod, 5 minutes and an import):
   two layers from a six-layer designer's board, unsaid. Fixed: the
   `--footprints-from` board's count without `--layers`, and
   `copper_layers_from`.
6. **The overhang refusal without a way on** (laptop power, 5 minutes).
   Fixed.
7. **Row members warned of as fixed parts on each other** (eurorack-pmod,
   21 lines ahead of the work rate). Fixed.
8. **A footprint's own open courtyard failed the layout** (laptop power,
   10 minutes): `malformed_courtyard` on the ESP32-S2-MINI-1, on the
   designer's board too. Fixed.
9. **The length rule's placement share is silent** (firestarter, 5
   minutes): the description, the template and the place check say
   nothing of `length_maxima`. Documented; a report open (below).
10. **Two starters** (eurorack-pmod, 3 minutes): the import's
    `constraints.json` has no decoupling `near`, `describe-kicad-board
    --constraints-template` has 20. Open.
11. **An RF module's antenna in the starter** (laptop power, 3 minutes).
    Open.

## Fixed on the branch (each its own commit, with a test)

- `import-kicad-netlist`: a fp-lib-table entry whose folder is not there
  is found by its last folders under the schematic's folder or one of
  its three parents (`relocated_libraries`); a library found nowhere is
  named with the table's folder; the output directory's `.pretty`
  folders are searched (`a_table_entry_at_another_machines_path_is_found_by_its_last_folders`).
- `import-kicad-netlist`: without `--layers` the `--footprints-from`
  board's copper layer count, else 2; `copper_layers` and
  `copper_layers_from` in the report
  (`the_footprints_board_gives_the_copper_layers_unless_layers_does`).
- The constraints-missed action names a side over the target beside one
  under half of it, the `back`/`front` lever and the smallest outline
  (`missed_constraints_on_a_full_side_beside_an_empty_one_name_the_other_side`).
- A row the outline cannot hold whose parts turned a quarter would fit:
  the refusal says the parts keep their angle and gives the other axis
  with the first part's rotation (`a_row_the_outline_cannot_hold_is_an_error_at_once`);
  constraints.md, the cookbook.
- The edge overhang refusal for a footprint without a keepout says how
  to keep the antenna at the edge (`an_overhang_without_a_keepout_says_how_to_keep_the_antenna_at_the_edge`).
- `strip-kicad-tracks --keep-nets <net>[,<net>...]` keeps those nets'
  tracks, arcs and vias for route mode to keep
  (`strip_keeps_the_tracks_of_the_nets_it_is_told_to_keep`); agents.md,
  SKILL.md.
- The length action for nets under the minimum gives the lever by mode
  (route mode: by hand or `--keep-nets` from the designer's board; a
  layout: the placer knows only maxima)
  (`length_findings_say_which_nets_are_unrouted_and_which_over_by_what`);
  agents.md on `length_maxima`.
- A row its first part's rotation turned a quarter is lowered as it lies
  when the layout lowers its placement again (it failed with exit 4)
  (`a_row_turned_a_quarter_keeps_its_members`).
- The input check does not hold a row's other parts where the input
  stacks them when a `place` entry puts its first part at x, y
  (`fixed_parts_at_one_spot_are_warned_of`).
- A `place` entry that sends a row (its body's centre) off the board is
  refused, with where it reaches and the two ways out
  (`a_placed_row_that_runs_off_the_board_is_refused`); constraints.md,
  the cookbook, the SKILL.
- A footprint's malformed courtyard (its one item the footprint) is the
  footprint's own without a reference
  (`a_footprints_malformed_courtyard_does_not_fail_a_layout_without_a_reference`).

## Left open, with a suggested design

- **Length minima in the placement.** `length_maxima` keeps a bounded
  net's pins within 0.8 of the maximum and knows no minimum: the
  firestarter's layout put J2 9 mm from U2 for a 25 mm minimum. Design:
  where a `length` rule has a minimum, a soft penalty when two of the
  net's pins lie closer (octilinearly) than a share of the minimum (half
  of it: the rest the meanders find on straight runs), in the same terms
  as the maximum's penalty in board_placer/rules.rs; measure on the
  firestarter and the expansion tier's length boards (pcdock base board's
  nine `opt` rules, OpenAIO-Whoop's skews).
- **Meanders where the straight runs are short.** Route mode on the
  designer's placement left 6 of 10 nets short with 0 to 10 mm of the
  meander added ("no straight track with room"): the meander is placed
  on one straight track. Design: spread the needed length over several
  straight tracks of the route (largest first), and where none has room,
  reroute the net with a detour cost (a target length in the search's
  cost, as a length-aware A*) before giving up; router.md "Length
  matching"; the router's area.
- **The length rule in the place check and the template.** board-placer.json
  `length_maxima`: per bounded net its pins' octilinear distance after
  the placement against 0.8 of the maximum (and the minimum), and a
  comment line in `--constraints-template` naming the rule, its nets and
  their parts. verdict.rs / board_placer/rules.rs.
- **One starter.** The import's `constraints.json` could carry the
  decoupling `near` entries the template writes for a stacked board (the
  same code, `constraints_template.rs`); the schematic tier's boards
  then start with them, which changes the tier's inputs: measure first.
- **An RF module's antenna in the starter.** A footprint of
  `RF_Module:` (or named ESP32/WROOM/MINI with an antenna drawing) gets
  an `edge` entry with `overhang` where it has a keepout, else a comment
  saying which end the antenna is and the rotation that turns it to an
  edge.
- **The missing list by footprint.** Trial 4 asked for it too:
  eurorack-pmod's 19 lines are 3 footprints; fold them as
  `LLP1006-2M:VISHAY-LLP1006-2M (8 parts: D27, ...)`.
