# pcb-maker for agents

You have a KiCad schematic, or a board with footprints and nets from
"Update PCB from Schematic", SKiDL or a generator. pcb-maker places the
parts, routes the board, tells you precisely what it achieved, and writes
what a fab needs to make and assemble it.

An agent CLI that reads skill files (Claude Code and others) can load the
short version of this guide: [skills/pcb-maker/SKILL.md](../skills/pcb-maker/SKILL.md)
(install: README.md, "For agents"). A test keeps it in step with the
command table and the exit codes.

**It needs KiCad 10.** pcb-maker writes KiCad 10 boards and judges them
by KiCad's own DRC, so `kicad-cli` of KiCad 10 must be on the `PATH`, or
`PCB_KICAD_CLI` must name it (on macOS
`/Applications/KiCad/KiCad.app/Contents/MacOS/kicad-cli`).
`route-kicad-board`, `layout-kicad-board`, `drc-kicad-board`,
`fab-kicad-board` and the import of a schematic ask `kicad-cli version` before any work: a missing
or failing kicad-cli, or one older than KiCad 10, is an input error
(exit 2) whose `next_actions` says this. A route or layout with KiCad's
check off (`PCB_SKIP_NATIVE`, `skip_native_verification`), a placement
and a netlist's import go on without it, with a warning in the verdict
(the import: on stdout).

## 0. From a schematic

When the mechanical brief is ready, run import and ordinary layout together.

```sh
pcb-maker layout-kicad-schematic hardware/board.kicad_sch board run \
  --constraints constraints.json --mechanical-from hardware/board.kicad_pcb \
  --layout-config layout.json
```

Create a fresh output directory by passing its path as `run`. The
mechanical donor and the layout config are optional. The command places
the supplied constraints in `run/project` before import so the importer
can apply any `add` entries and explicit mechanical poses. Layout reads
that same brief. A supplied layout config can omit `placer.constraints`
or point it at `constraints.json`.

When a donor has a locked schematic footprint, give it an absolute
`place` entry with a matching `front` or `back` assignment. To let that
part move, explicitly name it in the layout config's `placer.free`.
The workflow stops at import if it finds an unapplied donor hold.

Read `run/workflow.json` for the current stage, exit code, next actions
and report paths. If import stops, `run/import.json` and `run/import.log`
record what it found and `run/project` retains the generated files.
The initial layout streams to `run/layout.log`. Read `outputs.result`
and `outputs.layout_verdict` in the workflow report for the selected
project and verdict.

The workflow's `schematic_parity` records the check against the staged
schematic that accompanies the returned PCB. If the importer assigned
different footprint IDs, `footprint_identity` points to `footprint_id_changes`
in `import.json` for each source and imported ID. Verify against the
original KiCad project when its exact assignments are required.

When native checks leave one to three opens, the workflow can spend
the remaining modeled budget on one advised recovery. It first
considers the result's prepared row-escape advice and can apply its
exact pad list to the current router settings. An explicit list takes
precedence. Otherwise it can use an alternate placement seed suggested
by the result. Both strategies use the same imported project and brief.
Explicit seeds, route reuse and reserved recovery are honored. The
candidate must satisfy placement constraints, close every native
open in both copper states and introduce no active errors or parity
findings. `placement_retry` records the strategy, budget, work,
decision and both results. On a tie or a failed check, the first
result stays selected.

If the board is already connected and has one supported body-to-pin
distance miss, the layout can check a neighboring position on the
selected grid with its existing tracks and vias. This keeps the applied
grid, halos and footprint faces. When `spacing_mm` is omitted, the trial
may use the placer's courtyard-clearance model. A candidate must meet
every placement constraint and preserve native quality in both stored
and refilled copper. `body_near_grid_retry` records the result and modeled
work. The verdict links to the final placement report and playback and
identifies the pre-retry placement report.

The schematic workflow can also retry at a new pose on a finer local
pitch with the two inferred routing halos varied. Explicit grid and halo
settings take precedence for that search. The retry reuses eligible
copper and keeps the original brief, rules and other poses. The
`placement_retry.body_near_reseat` receipt records the measured gap,
requested limit and trial pose. A skipped retry reports why, including
when the geometry was ineligible or the search ran out of work.

`stored_delivery` reports the returned copper's verification scope.
With the default `save_zone_fills`, a refill-only completion is
incomplete at the workflow level until stored copper is verified.
The first action shows how to save fills and check them in a separate
directory. With `save_zone_fills: false`, the workflow can complete
with the reported refill-only scope. An unapplied donor hold also
returns exit 1.

When an unplaced RF module has a native antenna keepout and no existing
location constraint, the placement verdict can suggest the same
`edge` entry as the constraints template. Try it only when the enclosure
allows the antenna to extend beyond the board. The command reports the
edit in `next_actions` and leaves the brief intact.

Use the separate import when you still need to inspect footprints or
draft the constraints.

```sh
pcb-maker import-kicad-netlist <project>/<name>.kicad_sch <project-dir> <board-id> [--layers 4]
```

does what KiCad's "Update PCB from Schematic" does for a new board, on
`--layers` copper layers. Without an explicit count, the importer uses the
`--stackup-from` board's count, the mechanical board's count, the
`--footprints-from` board's count, or two layers, in that order. The report records `copper_layers` and
`copper_layers_from`. The import includes the following schematic data.

- every symbol's footprint from the project's and the user's
  `fp-lib-table` (KiCad's own libraries without one), with its fields (an
  `LCSC` field ends up in the BOM) and its DNP and BOM flags. The user's
  table is KiCad 10's (else 9's) under `KICAD_CONFIG_HOME`,
  `~/.config/kicad` (Linux), `~/Library/Preferences/kicad` (macOS) or
  `%APPDATA%\kicad` (Windows). KiCad's own libraries are the folder
  `KICAD10_FOOTPRINT_DIR` (else `KICAD9_FOOTPRINT_DIR`,
  `KICAD8_FOOTPRINT_DIR`) names, else the first of
  `/usr/share/kicad/footprints`, `/usr/local/share/kicad/footprints`,
  `/Applications/KiCad/KiCad.app/Contents/SharedSupport/footprints` and
  `%ProgramFiles%\KiCad\<version>\share\kicad\footprints` that exists;
  set `KICAD10_FOOTPRINT_DIR` for an install elsewhere;
- pads on their nets, footprints linked to their symbols, so KiCad's
  schematic parity check passes. Pads in older libraries that lack an ID
  get a deterministic UUID per footprint instance and pad. Re-importing
  the same design preserves the pad identities reported in native DRC;
- the schematic, project file and the project's own libraries copied
  next to the board.

The import copies the input's companion `.kicad_dru` to
`<project-dir>/<board-id>.kicad_dru`. For a netlist without a companion
file, it also looks for the rules of the schematic named in the netlist.
An existing output rules file takes precedence and is retained on
re-import. To use revised rules from the source, update the output
rules file or import into a fresh directory.

A KiCad netlist (`.net`, from `kicad-cli sch export netlist`) works in
place of the schematic; its footprints come from the libraries of the
netlist's folder and of the project the netlist names as its source. A
same-stem `.kicad_pro` beside the `.net` supplies the project rules under
the chosen output board ID. An existing output `.kicad_pro` takes
precedence. A file that is not a KiCad netlist is an input error (exit 2). Footprints
from KiCad 5 and 6 libraries (`module`, `fp_text reference`, arcs given by
centre and angle) are brought to today's form, so KiCad loads the board
and reads the schematic's reference, not the library's. Libraries outside
the project folder (`${KIPRJMOD}/../lib`) are used but not copied. All
footprints sit at one point and there is no outline. Next to the board it writes, unless they exist:
- `constraints.json`: `"move_all": true`, `"outline": {}` (sized from the
  parts), and the plug-in connectors (USB, RF, audio and barrel jacks,
  D-sub, network, video, card edges, terminal blocks) on `"edge": "any"`,
  facing out. Edit it: it is where what the user wants goes. With
  `--near-decoupling` (off by default; since 2026-11-04) it also holds the
  decoupling capacitors' `near` entries that `describe-kicad-board
  --constraints-template` writes for the imported board (each at a supply
  pin of the IC the schematic draws it nearest to; the report's
  `near_entries` counts them). They are hard constraints a layout can
  miss: on the schematic tier they cost 10 of 12 passes at a third of the
  budget (2 to 29 missed per board) for no fewer unconnected items
  ([benchmarks.md](benchmarks.md));
  the placer's own pull (`decoupling_near`) is softer.
- `layout.json`, which uses it, so that
  `pcb-maker layout-kicad-board <project-dir> <board-id> <out-dir> auto <project-dir>/layout.json`
  lays the board out.

**A keyboard: `constraints.starter.json`.** When the netlist holds a key
matrix, the import also writes `constraints.starter.json` beside
`constraints.json` (rewritten on every import; `constraints.json` never
is). From the netlist alone it finds the switches (footprint or symbol
named `SW`, `KEY`, `MX`, `Choc`, `hotswap`, or a two-pin part on a row and
a column), each key's diode (a two-pin `D` part on a net with that switch
alone), the rows and columns (a switch's two nets, directly or through its
diode: rows by their names, `ROW0`, `r2`, `col_3`, else the side with
fewer nets) and the halves of a split keyboard (two matrices no switch
joins, or columns named for a hand, `col_L1` and `col_R1`). The file is
the import's starter constraints plus:
- a `row` per matrix row, its keys in column order at the key pitch
  (Choc 18 x 17 mm, else 19.05 mm, or the switch body where it is wider);
  rows over the same columns, one after the other, are one grid
  (`columns`, `row_pitch_mm`); keys larger than the pitch (1.5u, 2u, an
  ISO enter) stay out of the rows, `near` their neighbour, the keys after
  them `relative` right of the keys before; a row whose columns have a gap
  (thumb keys of both hands on one row) is a row per piece;
- each row `relative` below the one before (short rows, the thumb keys,
  up to 10 mm below);
- each diode `near` its key (3 mm);
- per half, a `region` in words (`"at": "left-half"`) with its rows and its
  controller (the part with the most pins on the half's lines);
- `comment`: what was inferred and what needs your decision (key pitch and
  stagger, thumb clusters, the controller's edge, the outline, the switches
  outside the matrix).

Copy it to `constraints.json` and edit it there. The report's `starter`
says what was found: `matrix` (row and column nets), `switches`,
`diodes`, `halves` (each with its rows, columns, switches, controller),
`unassigned` (switch-like parts outside the matrix: reset and power
switches), `wide`, `alternates` (a second footprint on the same key), or
`why` no file was written (keys wired each to its own pin: no matrix).
Urchin (two 4 x 5 halves) from the starter alone plus the walkthrough's
230 x 95 mm outline: 42 of 42 constraints kept, 68/68 routed, KiCad 0
unconnected, exit 0 (`"total_seconds": 30`).

The report lists what needs a look:
- `missing`: footprints not found in any library (the board leaves them
  out), or unreadable, one line per footprint and reason with the parts
  that share it (`8 parts: D27, D29, ... (LLP1006-2M:VISHAY-LLP1006-2M
  not found: ...)`, a single part's `R2 (...)`; since 2026-11-04,
  `missing_parts` counts the parts), each with where it was looked for: no library of
  that name (add it to the schematic project's `fp-lib-table`, or put its
  `.pretty` folder in the schematic's folder or in `<project-dir>`, the
  output), a table entry whose folder is not there (the message names the
  table's folder), or the library without that footprint; or give
  a board that has them (`--footprints-from`, below). Then
  import again into the same directory: the board is written anew,
  `constraints.json` and `layout.json` are kept (the report still names
  both, and lists them in `kept`);
- `substituted`: footprints found in another library, under KiCad 9's
  name (`DSUB-25_Female` is now `DSUB-25_Socket`), under KiCad 10's (a
  vendor prefix: where the library lacks the name, its one footprint
  whose name ends in `_<name>`, so
  `Package_SON:WSON-8-1EP_2x2mm_P0.5mm_EP0.9x1.6mm_ThermalVias` becomes
  `Package_SON:Texas_DSG0008A_WSON-8-1EP_2x2mm_P0.5mm_EP0.9x1.6mm_ThermalVias`,
  "(KiCad 10's name: a vendor prefix)") or in an add-on library under
  its Plugin and Content Manager nickname (`Espressif:` as
  `PCM_Espressif:`); the board and the copied schematic use the name
  found;
- `add_on_libraries`: footprints taken from KiCad's add-on libraries,
  which the import reads from the Plugin and Content Manager's folder
  (`KICAD10_3RD_PARTY`, else `~/.local/share/kicad/10.0/3rdparty` on
  Linux, `~/Documents/KiCad/10.0/3rdparty` on macOS and Windows; 9.0
  after 10.0) also without a table entry, under the folder's name and
  `PCM_<name>`. Used, not copied into the project. Add-on libraries
  schematics name that KiCad does not ship, and where `missing` says to
  get them: `Espressif` (the ESP32 modules: the Plugin and Content
  Manager's "Espressif KiCad Library", or
  github.com/espressif/kicad-libraries);
- `relocated_libraries` (since 2026-11-04): libraries a `fp-lib-table`
  names at a folder that is not there (a path on the designer's machine,
  `/home/seb/dev/eurorack-pmod/hardware/libs/custom/AudioJacksFixed.pretty`),
  found by their last folders under the schematic's folder or one of its
  three parents (`<checkout>/hardware/libs/custom/AudioJacksFixed.pretty`)
  and used from there; trial 6's eurorack-pmod found 2 of its 3 such
  libraries so (132 footprints to 141);
- `unmatched_pins` lists schematic pins missing from the selected footprint.
  `unmatched_pin_details` identifies each part, footprint, pin, and net.
  Its `connection` field distinguishes a named net, an explicit
  no-connect pin, and an unwired pin. For a named net, check the footprint
  variant and pin numbering before routing. Then re-import and check parity.
  With an own-reference `--footprints-from` match, the detail can include
  `donor_pin_numbering`. A `unique_candidate` is one donor pad number
  present in the selected footprint, absent from the schematic pin list,
  and on the same authored net. Repeated physical pads of that number are
  shown separately, with their source IDs and footprint-local positions.
  `ambiguous` and `no_candidate` give the reason mapping evidence was
  insufficient. Review any candidate against the source symbol and
  footprint before correcting the source. The importer keeps the original
  numbering. `layout-kicad-schematic` surfaces unique matches in
  `workflow.json.pin_numbering_review`.
- For KiCad's `footprint_filters_mismatch`, open the named symbol in the
  schematic and compare its assigned footprint with its footprint filters.
  Check the package and pad numbers first. If the assignment is wrong,
  correct the footprint. If the custom footprint is intentional, add a
  filter specific to its library and name to the corresponding symbol
  and run schematic parity again. On a copy of TelemetryOnboard, adding
  `Footprints:JL1` to the filters for `Symbols:JL1_JL3` cleared JL1's
  finding. JL3 still reported its own mismatch. The PCB was byte-identical
  and KiCad reported the same 131 physical findings and zero opens.
- `donor_connectivity`: when a footprint donor is supplied, compares
  electrical groups on shared, unambiguous reference and pad numbers.
  Renaming a net or relabeling an isolated terminal as a no-connect
  preserves connectivity. A `different` report gives counts and examples
  of split, merged or isolated groups to review before layout. The
  imported board uses the schematic or netlist's connections. The report
  counts pads found on only one side and excludes repeated references and
  conflicting duplicate pad numbers from comparison;
- For an authored footprint ID without a library nickname, an
  unambiguous same-reference donor with the same name supplies the model.
  The board keeps the schematic's footprint ID and the importer stages a
  local library copy of the model;
- `clearances` lists the footprint clearance overrides applied for gaps
  below the net class clearance that meet the board minimum;
- `clearance_conflicts` names the footprint, measured pad gap and
  required clearance when a board minimum or explicit rule requires more
  spacing, or when `--keep-footprint-clearances` retains an inherited
  net class requirement. Check the footprint and the named rule before
  laying out the board;
- Pass `--keep-footprint-clearances` to keep the clearance policy of the
  supplied footprints. `footprint_clearance_policy` records the choice.
  For each footprint with a supplied override or an eligible own-pad
  gap, `footprint_clearance_evidence` records the supplied value if
  present, the measured gap and rule, the proposed value, and whether
  import applied it. The same diagnostic check runs in either mode;
- `footprint_findings` reports KiCad errors on the pads or drills of one
  footprint from the importer's existing isolated DRC. Each group gives
  the reference, library footprint, finding type, count, and KiCad's
  measured and required values where reported. Read the evidence and
  check the part drawing and the project's fabrication rules before
  routing. This check covers built-in hole clearance, hole-to-hole,
  annular width, drill size and padstack errors. A zero-width annulus
  on an unnumbered plated hole with no net prompts a check of the
  part drawing's plating specification. A plated electrical pad needs
  the required copper annulus around its drill. Placement, board geometry
  and custom-rule findings remain for the final KiCad check;
- `added`: the parts of `constraints.json`'s `add` (below).

**A board to take footprints from: `--footprints-from <board.kicad_pcb>`.**
Designers keep footprints in personal libraries that do not travel with
the project, and libraries change after a board is made (a courtyard that
grew, a hotswap socket redrawn). When you have a board with these parts
(the designer's, an older revision, a colleague's), give it:

```sh
pcb-maker import-kicad-netlist <project>/<name>.kicad_sch <project-dir> <board-id> --footprints-from old/<name>.kicad_pcb
```

- The importer assigns each footprint on that board to at most one symbol.
  A path unique in both files follows the symbol even after renumbering.
  When a path repeats, the importer matches unique references within that
  path, then matches unique references among the remaining parts. If the
  symbol still needs a model and its library lacks it, the importer can
  copy an unambiguous donor model of the same name. It restores library
  form by bringing the footprint to the front at the origin, removing
  its rotation and dropping pad nets. An exact qualified ID from the
  symbol's own donor footprint can be kept when the current KiCad
  library lacks the model. The importer adds it to the staged project
  library or copies every existing model under that nickname into a
  project-local library before adding it. Equivalent models can share
  the authored ID when their hidden properties have different positions
  on individual footprints. The board instances keep those positions and
  the property values. With a schematic as input, several parts can also
  share a land pattern when each part's own donor agrees exactly with
  that symbol's hidden `Datasheet`, `Description`, `LCSC`, `MFG` and
  `MFGPN` fields. Each PCB footprint retains its own field forms.
  The shared library model has standard Reference and Value defaults.
  Distinct geometry, assembly attributes, visible field positions and
  unresolved property differences can still produce individual models in
  `from_board.pretty`. These choices appear in `footprints_from_board`.
  A symbol without a footprint takes the board's footprint of it too.
- A schematic's footprint table can point to a missing legacy `.mod`
  library. Import into a separate output project with `--footprints-from`
  to restore the nickname there for exact, unambiguous models from each
  symbol's own donor footprint. The importer stages a native `.pretty`
  library and preserves the original table. If the legacy file exists or
  its location cannot be resolved, the donor model gets a separate ID.
  To use the models from a legacy library itself, convert that library to
  KiCad's `.pretty` format first.
- To keep a schematic export portable, the importer copies regular
  footprint models from a native `.pretty` library referenced by a
  project-relative path outside the source folder. The copied project's
  table keeps the library nickname and points to the local copy. A
  different library already available at the output path keeps precedence
  in the output table.
- Where the board has the symbol's footprint, **the board's version is
  used over the library's**: the board is what was made and checked, the
  library is what changed since. The count is `board_versions`;
  `drifted_footprints` lists the ones whose library version differs from
  the board's by more than a fifth on a side or in pad count, with both
  sizes (`lib:SW (64 parts: SW1, ...): library 15.93 x 12.20 mm, 10 pads;
  board 12.85 x 11.60 mm, 6 pads`). Where the board's footprint has another
  name than the schematic's (the board was changed after the schematic, or
  the schematic after the board), the board's is used under its own name
  and listed in `substituted`. The copied schematic links the selected model
  for each symbol. `schematic_footprint_link_conflicts` lists links the
  importer could not update, such as a symbol in a modern shared sheet
  whose instances use different footprint IDs on the board. Import without
  `--footprints-from` to use the library versions.
  When the symbol's own donor instances share a footprint ID and differ
  only in User.Drawings shapes or supported Fab reference text
  presentation, the importer keeps that ID and preserves each instance's
  drawings and text on the board. For a Fab reference at the same anchor,
  the instance angle can differ. `footprints_from_board` records that choice.
  Nested footprint groups share a model when their names and member trees
  agree after resolving item UUIDs. Each instance keeps its own group links
  and child IDs. Different or unresolved group membership keeps models
  distinct.
  A missing model for a two-terminal rectangular SMD footprint can retain
  its authored ID when every donor belongs to its own schematic symbol and
  differences are limited to one nanometre per local pad-centre or solid
  silkscreen line coordinate, line direction, and the presentation of
  standard hidden Fab fields. The comparison preserves one exact donor
  model in the project library and each instance's own geometry on the PCB.
- A symbol's own footprint (and a `from_board` part's) **keeps the board's
  ids** (the footprint's, its pads' and drawings'), as KiCad's "Update PCB
  from Schematic" does: the project's DRC exclusions name items by them,
  so a finding the designer excluded (two courtyards that overlap by
  design, a key socket over the edge) stays excluded on the new board
  where the part stays where the designer put it. A copy of a footprint
  (another part of that name) gets ids of its own.
- `missing` then says when the board lacks a part as well.

**An authored fabrication stackup** can be copied with
`--stackup-from old.kicad_pcb`. This imports the board thickness and the
complete copper, dielectric and finish definitions. The donor's active
copper layers and authored stackup must agree with each other and with
any `--layers` value. The import report records the donor and its SHA-256.
The standalone from-schematic workflow takes the same option. Its
prepared package and saved-board scorer check the whole stackup. They use
KiCad's nanometre precision for thickness and the physical order of its
numbered dielectric layers. Material and electrical properties must match.

**A mechanical starter board** can supply its board-level Edge.Cuts, rule
areas and locked schematic-backed footprint poses and sides.

```sh
pcb-maker import-kicad-netlist hardware/board.kicad_sch work board \
  --mechanical-from hardware/starter.kicad_pcb
pcb-maker layout-kicad-board work board result auto work/layout.json
```

The imported project contains schematic-derived pads and nets. Parts
without a hold are movable. Add `--footprints-from` or `--stackup-from` explicitly
to use a PCB's footprint definitions or fabrication stackup. The
`mechanical_from` report gives the source and geometry hashes, counts of
edge items and rule areas, and each locked match's pose, side and applied
status. When a schematic footprint carries the supplied perimeter, the
import also holds that footprint at its authored pose and reports its
exact native local shape count as `locks[].perimeter_edge_cuts`. Its
existing schematic pad assignments stay in place. A movable part with a
slot inside the board keeps its usual placement behavior. The import
also transfers locked board-only footprints that provide
unnetted non-plated drills or footprint Edge.Cuts. These retain their
native UUIDs and poses even when display references repeat. See
`retained_board_objects` for their drill and edge counts. Footprint
edges can join the main contour. The importer checks closure with
those edges at their retained positions. For a held footprint with a
closed interior cutout drawn as connected lines and arcs, an absolute
`place` entry lets the placer establish the cutout at that pose before
seating the other parts. Duplicate UUIDs on hidden
footprint metadata fields are recorded under `reidentified_metadata`.
`--mechanical-footprint UUID` explicitly transfers a board-only physical
footprint at its native pose, even if it is unlocked on the source PCB.
Repeat the option for each UUID and repeat the selection when reimporting.
This accepts unnetted, circular plated drills with concentric standard
lands on all copper layers. Their pad UUIDs, local positions, drill and
land diameters appear in `retained_board_objects[].plated_drills`.
A bare UUID also accepts a board-only footprint tagged `fiducial`, with a
single unnumbered circular copper mark and explicitly sized solder mask
opening on the same face. `retained_board_objects[].optical_fiducial`
records the pad UUID, local center, copper and mask diameters, and any
pad clearance. The mark stays netless at its authored pose and side.
Use `--mechanical-footprint UUID=NET` for an explicitly board-only
footprint whose plated drills already use one named schematic net.
The import checks the existing assignments against the requested net
and an imported schematic pad. Bound drill receipts include `net`.
Unassigned drills keep their unassigned state. Reimport repeats the
selection and checks each plated pad UUID against the donor's binding.
The same receipt sets `explicit_selection`. The transferred footprints
are locked on the imported board.
`excluded_board_objects` identifies omitted locked or selected physical objects
with their reference, available UUID and reason. These make the import
incomplete. Reconcile an omitted drill or cutout with the mechanical source.
Other unmatched locks, including
artwork, appear in `unmatched_locks`. Mixed
copper and technical rule-area layers are preserved when the generated
stack supports their names. A locked footprint must have a unique
matching reference. When both boards have schematic paths for that
reference, the paths must agree. Ambiguous or conflicting identities
appear under `conflicting_locks` and cause an incomplete import exit.

An explicit `add.from_board` can also satisfy an authored board-only
lock when both donor options read the same PCB bytes. Keep the original
reference, footprint UUID, pose, side, pad nets and physical model.
`mechanical_from.matched_additions` lists the verified copies. A renamed,
moved or missing object still requires resolving its mechanical transfer.

Pass `--mechanical-from` again when reimporting. The importer keeps
existing constraints and layout files. When the output PCB already has
an outline, it keeps its local Edge.Cuts and rule areas. The kept
constraints are authoritative for footprint poses and sides. Use a
fresh output directory to start from a different mechanical source.
Applied holds are native PCB footprint locks, so KiCad also keeps them
in place. On reimport the importer uses the current layout's absolute
pose and explicit side for each matching hold. Removing the pose or
listing the reference in `placer.free` releases that native lock.
To release a component that carries the perimeter, first transfer its
outline geometry to board-level Edge.Cuts in the existing project.
Matching board-only mechanical UUIDs keep admissible local geometry
and poses. Reimporting from the donor restores an absent donor object.

If a held footprint carries the whole outline and represents rows of
perimeter contacts without a separate body shape, the placer can retry
failed placement using all of that footprint's pad and copper obstacles.
`board-placer.json.perimeter_contact_recovery` reports the attempt and
modeled cost. This retry keeps the held pose and accepts a placement
only when every part fits and all authored placement constraints pass.

When front-side crowding blocks placement and movable SMD passives can
be considered for the rear, `verdict.json.rear_placement_recovery`
lists exact references in `allow_back_additions`. If the assembly
permits rear mounting, append the approved references to
`placer.allow_back` and retry. `affected_movable` counts movable parts
without a legal position. `rear_headroom_mm2` estimates rear area at the
placer’s utilization target. The retry must still pass placement and
routing checks.
When placement uses the input PCB’s supplied Edge.Cuts,
`outline_from_input` is true and the verdict keeps that outline as the
basis for recovery.

Set `placer.signal_anchor_seed` to `true` to try one alternate starting
pose for a signal-dense movable footprint connected to held interfaces.
The placer estimates its orientation and position from the pad graph and
projects the estimate to a legal body position. That footprint remains
movable during full placement. Ordinary seeds still compete on the same
layout. When the placer proposes a pose, `board-placer.json` records the
reference, estimated and projected positions, angle, seed number and
estimated work in `signal_anchor_seed`. Use `seed_kept` to identify the
placement the solver selected and `footprints` for the final positions.

**Parts the schematic does not have** (mounting holes plated on GND, a
second footprint for another switch): an `add` entry in
`<project-dir>/constraints.json`, then import again into the same
directory. `{"part": "H1", "footprint": "MountingHole:MountingHole_3.2mm_M3_Pad",
"nets": {"1": "GND"}, "x": 3.5, "y": 3.5}` adds a library footprint there;
`{"part": "H1", "from_board": true}` takes the `--footprints-from` board's
H1 with its nets, where that board has it. The parts are `board_only` (the
parity check leaves them out) and are routed like the others;
[constraints.md](constraints.md#parts-the-schematic-does-not-have-add).

## 1. Look at the board

```sh
pcb-maker describe-kicad-board <project-dir> <board-id> > board.json
```

The file lists what constraints refer to:
- the outline's box and the copper layers (`outline` is null without
  one; `from_footprints: true` when the board draws no Edge.Cuts itself
  and the box is only footprints' slots and cut-outs, as on a fresh
  import: give the board its size with `outline` in `constraints.json`);
  `points` are the outline's outer corners (arcs as short chords) and
  `cutouts` the holes the board's own Edge.Cuts draws inside it (a
  panel's slots between its boards and rails, holes drawn on Edge.Cuts;
  since 2026-11-04): to keep the shape of another board (the designer's,
  an older revision), describe that board and copy its `points` into
  `outline.polygon`, its `minimum` into `outline.x`, `outline.y` (so
  that positions read off it, `place` with `"origin": "absolute"`, stay
  where they were) and each of its `cutouts` into `outline.cutouts` as
  `{"polygon": [...], "origin": "absolute"}` (without them a panel comes
  back as one plate, and no tab or frame is found);
- `sub_boards` where the outline falls into pieces at tabs, narrow necks
  or V-cuts (a panel; since 2026-11-04, absent for a board of one
  piece): `pieces`, `cut_by`, the `tabs`' boxes (a V-cut is one box per
  cut line, a strip about 2 mm wide along it: since 2026-11-04; trial 5's
  SmartSpin2k grid of V-cuts was one box over the whole panel), the `frames` (pieces
  no part with pins lies on, or where no part lies on any piece, the
  pieces that span the panel along a side with at most a fifth of its
  area: `frames_by` `parts` or `shape`), `parts_assigned` (parts with
  pins whose origin lies on a piece: 0 for a netlist import's stack) and
  `sub_boards`, the other pieces' boxes with the parts with pins on each
  (`parts`; no `movable`, which a description cannot tell: the place
  check's `sub_boards.sub_boards[].movable` in board-placer.json counts
  the parts it moves there). With
  `parts_assigned` 0 nothing tells which part goes on which board: give
  `--reference <the designer's project>` or a `region` per board (the
  boxes are the regions);
- every footprint with reference, value, library name, position, side
  (where its pads put it), `layer` (the footprint's own, `F.Cu` or `B.Cu`:
  what `back` and `front` set; a key socket under its switch is a front
  footprint with its pads on the back), size, whether it is through-hole
  or locked, its `uuid` (which footprint
  an entry for a repeated reference means, [constraints.md](constraints.md)),
  and its pads with their nets;
- every net with its pads (`REF:PAD`, the form `pin_of` takes), its
  `class` (its highest-priority net class, `Default` for a net in none)
  and its track width and clearance;
- `classes` (since 2026-11-06, trial 7: the class widths were only numbers
  per net, and an agent read the `.kicad_pro` to learn that 0.1422 mm was
  a class): one row per class of the project, Default first, with its
  `track_width_mm` and `clearance_mm` as the project states them, its
  `nets` (how many of the board's nets are in it), and where custom rules
  speak of its nets `layer_widths_mm` (the width a `track_width` rule with
  an `opt` or a `max` sets per layer, on some layers only: a controlled
  impedance per layer, such as Sisu_SSE-9's USB at 0.1 mm on In1.Cu and
  In4.Cu; a `min` alone is the process's floor on those layers, such as
  the 4in1-mini's "2 oz outer copper" 0.16 mm on F.Cu and B.Cu, and gives
  none: until trial 8 it did, and the verdict's `class_layers` told the
  agent to keep every net off the inner layers) and
  `disallowed_layers` (the layers a `disallow track` rule keeps them
  off). A class narrower than Default is likely a controlled impedance, a
  wider one a current to carry; nothing in KiCad ties a class without
  such rules to a layer (a route's `verdict.class_layers` says where it
  went);
- `class_assignment_suggestions`, where a net in `Default` matches one
  unused project class by name (ignoring a leading sheet slash and ASCII
  case). Each row gives the net's router name (`net`), its full KiCad
  name (`kicad_net`), its current class and resolved track width and
  clearance, and the matching class and its declared values. For
  example, the RP2040 motor controller's `VMOT` resolves to 0.2 mm in
  `Default` while its unused `VMOT` class declares 2 mm. If the class is
  intended for that net, add the row's `netclass_pattern` entry to
  `net_settings.netclass_patterns` in the `.kicad_pro`, or set the class
  with a schematic directive and update the board. Use the directive
  for names with special characters, where the row omits the pattern.
  Describe the board again to confirm the class. The field is absent
  when there are no suggestions;
- `custom_rules` (since 2026-11-06): every constraint of the project's
  `.kicad_dru` with its `status` as the router reads it (`applied`,
  `approximated`, `no_effect`, `not_routing`, `unmodelled`, `skipped`)
  and `detail`, the length and skew rules as the length matching keeps
  them (a route's `verdict.custom_rules` is the same list, with the pair
  rules `applied` where route mode routes the pairs coupled), and
  `unmodelled_rules` (the routing rules the router does not keep, which
  KiCad still checks);
- `swap_candidates`: parts whose general-purpose pins the layout could
  reassign (see pin swapping below), by their pad functions or, where
  the board names none, by their pins' electrical types.

**Starter constraints for a board** (2026-10-26): `pcb-maker
describe-kicad-board <project-dir> <board-id> --constraints-template
constraints.json` also writes a `constraints.json` skeleton to a new file
(an existing file is refused, exit 2), as `import-kicad-netlist` writes its
starter for a schematic:

- `edge`: the plug-in connectors (USB, jacks, coax, terminal blocks, by
  footprint name), `flush`, with `opening_outwards` where the footprint
  shows its mouth. On a board with a placement (most parts at positions of
  their own) each gets the side of the outline its body lies within 3 mm
  of (at a corner, the side its courtyard reaches farthest beyond its pads
  towards, else the one its mouth faces); on a board without one (a fresh
  import, the parts stacked) `any`.
- `near`: each decoupling capacitor (two pins, a supply rail and ground,
  the rail on an IC's supply pin) at the closest supply pin of its IC on a
  placed board; on a board without a placement at a supply pin of the IC
  the project's schematic draws it nearest to (an IC of the rail drawn
  without a capacitor takes the extra of one with more capacitors than
  supply pins on the rail; at an IC its supply pins in turn), else at the
  rail's supply pins in turn; `max_mm` 1.5 (3 for 4.7 uF and more), and on a
  placed board at least the board's own distance from the capacitor's
  courtyard to the pin, rounded up to 0.5 mm, where that is more (the
  `comment` lists them; since 2026-10-27: framework_mobo_lefthalf's C1 lies
  5.85 mm from U1:22, `max_mm` 6, where 1.5 missed on the designer's own
  placement; 26 of MokyaLora's 53 entries widen, to 2 to 6 mm). A board
  without a placement keeps 1.5 and 3. A
  supply pin is told by the pin's function where the board names it
  (`VDD`, `VCC`, `AVDD`, `VIN`, `VM`, `3V3`, ...; never an enable or
  control input tied to the rail: `EN`, `SHDN`, `CE`, `SLEEP`, `CS`), else
  by its electrical type (power in or out; an input, output or
  bidirectional pin tied to a rail is none), else by the net's name
  (VDD/VCC/AVDD/+xVx). The schematic trial's little-red-rover (stacked,
  2026-10-26): C7 and C8 (22 uF) at the motor drivers' VM pins U6:1 and
  U5:1, C9 and C10 at U6:8 and U5:8, the designer's pairs (before: C7 at
  the LDO's EN pin U3:3).
- RF modules (since 2026-11-04; KiCad's `RF_Module:` footprints and
  footprints named for ESP32, WROOM, WROVER or nRF modules): where the
  footprint has a keepout zone (KiCad's ESP32 and nRF modules draw the
  antenna's), an `edge` entry, `flush`, with `overhang` (the keepout
  beyond the edge, the module inside): on a placed board the edge the
  keepout faces, else `any` with `"overhang": {}` (the edge chosen). The
  comment names the rotations that turn the antenna end to each edge,
  for keeping the keepout on the board instead. Where the footprint marks
  its antenna only in text ("Antenna", "Keepout Area" on a drawing layer,
  as laptop power's ESP32-S2-MINI-1 does) no entry, a comment: `U5
  (RF_Module:ESP32-S2-MINI-1) is an RF module whose footprint marks its
  antenna in text only (no keepout zone, so no overhang): the antenna end
  faces the bottom edge at 180 degrees here. To put it at an edge, add an
  edge entry ("flush": true) and the rotation that turns that end to it:
  top 0, right 270, bottom 180, left 90.`
- a `bank:` comment (since 2026-11-08, trial 8) for six or more two-pad
  parts of one footprint on the same two nets that no `near` entry holds
  (bulk capacitors on a supply and ground), holding the `row` to paste
  that keeps them one block: `columns`, across their short side at their
  pads' pitch as they are turned (neighbours in a line 0.1 mm apart, their
  facing pads sharing a net; the lines the pads' span and the project's
  copper clearance apart, as every part of a row is turned alike and two
  lines face with opposite nets), about as wide as it is tall with the
  fewest empty places in the last line (the 4in1-mini: 6 columns at 1.9
  and 4.26 mm, which places with a `place` for its first part at the
  designer's spot and leaves 3 parts without a place without one). The
  placement starts each part where its nets pull it, which for parts on
  the same two nets is one spot, and spreads them wherever they fit: the
  mini's 22 1206 capacitors came back around the board's edge, where the
  designer packs them in one block. A comment, not an entry: as a row the
  mini's bank left 69 and 103 unconnected against 89 without (one layout
  each), no better, so it is the agent's call (an enclosure, a designer's
  look). stderr counts it (`1 bank of parts on two nets with its row in
  a comment`).
- `move_all`, and a `comment` list saying what was inferred and what to
  decide: connectors whose mouth the footprint does not show (give them a
  `rotation`; the comment names their angle on the board), the other
  connectors (headers), fixed parts, keepouts, regions.

On the trial's rp2350-dual-usb the designer's board gives J2 and J4 left,
J3 and J5 right, and 17 capacitor-pin pairs the designer's own (C13 at
U2:38, C18 at U2:44); lpc1311-target-v1 gives J7 left facing out and C4 at
U2:10, the constraints the trial wrote by hand.

## 2. Say what you want: `constraints.json`

```json
{"version": 1,
 "move_all": true,
 "outline": {"width": 60, "height": 40},
 "edge": [{"part": "J1", "edge": "left", "flush": true}],
 "near": [{"part": "C1", "pin_of": "U1:3", "max_mm": 2}]}
```

Full reference: [constraints.md](constraints.md).
- **`move_all`** places each movable part. It keeps native locks, the
  brief's `fixed` entries, and the placer's `fixed` and `fixed_patterns`
  holds. Use `placer.free` to release a default hold. Mounting holes no
  constraint names go to the corners.
- **`place`** says where a part goes in words (`"at": "top-left"`,
  `"center"`, `"front"` with `device_front`) or exactly (`x`, `y`,
  `angle`, for a part that must match an enclosure).
- **`outline`** sets the board size, or its shape: a `polygon`, round
  corners (`corner_radius`), holes (`cutouts`) or several `pieces`
  ([constraints.md](constraints.md#outlines-that-are-not-rectangles)).
  On a polygon, `edge` means the nearest side of the polygon facing that
  way.
- **`edge`, `region`, `keepout`, `rotation` and `fixed`** decide where parts may be;
  **`back`/`front`** on which side. Parts the placer keeps where they are
  (`fixed`, `fixed_patterns`, locked) that lie on top of each other (a
  glob that fixed parts still stacked at their import position) are
  warned of before the run, in the log and first in the verdict's
  next_actions: `fixed parts C1 and C2 overlap (courtyards and pads on
  F.Cu)`. These warnings use the contours and pads at the held poses.
  A `bounding boxes` warning marks a pair whose exact overlap remains
  unverified, for example when just one footprint has a reverse-side
  courtyard. Inspect the held poses and contours in KiCad.
- **`hollow`** lets parts sit inside a big part's outline (a shield, a
  module above parts): only its pads block. Parts with holes still stay
  out of its courtyard's shapes, as KiCad forbids holes inside a
  courtyard.
- **`near`, `group` and `relative`** keep parts together; **`apart`** keeps
  them away from each other; **`row`** puts parts in a line at a pitch (LED
  bars, key rows). A row moves as its first part: other constraints name
  that part (and then hold the whole row: `"below": "SW1"` is below the
  grid), except that `near`'s `part_of` and `pin_of` may name any key of
  it (a diode at its switch).
- **`overhang`** lets a module's antenna reach beyond an edge.

Unknown keys, parts or pads are errors, not silently ignored.

### Cookbook: from what people say to constraints

| They say | Write |
| --- | --- |
| "The buttons are at the front, the front is the left side" | `"device_front": "left"`, `"place": [{"part": "SW1", "at": "front"}]` |
| "Power jack at the back" | `"place": [{"part": "J2", "at": "rear"}]` (with `device_front`) |
| "USB-C on the bottom edge, plug from outside" | `"edge": [{"part": "J1", "edge": "bottom", "flush": true, "opening_outwards": true}]` |
| "The connectors on the edge, whichever suits" | `"edge": [{"part": "J1", "edge": "any", "flush": true, "opening_outwards": true}, ...]` |
| "Mounting holes 3.5 mm in from each corner" | `"place": [{"part": "H1", "x": 3.5, "y": 3.5}, ...]` |
| "Four M3 holes on GND" (the schematic has none) | `"add": [{"part": "H1", "footprint": "MountingHole:MountingHole_3.2mm_M3_Pad", "nets": {"1": "GND"}, "x": 3.5, "y": 3.5}, ...]`, then import again |
| "Use the footprints of the old board" | `import-kicad-netlist ... --footprints-from old.kicad_pcb` (missing footprints from it, its versions over the libraries'; `drifted_footprints` says what differs) |
| "Keep the old board's extra parts" | `"add": [{"part": "MX37_1", "from_board": true}, ...]` with `--footprints-from` |
| "The connector must match the enclosure cut-out" | `"place": [{"part": "J3", "x": 12, "y": 0, "angle": 90}]` |
| "Status LED in the top-right corner" | `"place": [{"part": "D1", "at": "top-right"}]` |
| "The bulk capacitors in one block, as on the old board" (a bank: many parts of one footprint on the same two nets) | `"row": [{"parts": ["CL*"], "pitch_mm": 2.0, "row_pitch_mm": 4.3, "columns": 6}]`, `"place": [{"part": "CL32", "x": 71.3, "y": 40.3, "origin": "absolute"}]` (`--constraints-template` writes this row with the pitches from the pads in a `bank:` comment; every part of a row takes the first part's angle, so two lines face with opposite nets and keep the copper clearance: a pitch too small is refused with the one needed and which setting it is, `pitch_mm` in a line or `row_pitch_mm` between lines) |
| "Eight LEDs in a row, 5 mm apart, along the top" | `"row": [{"parts": ["D1", "D2", ..., "D8"], "pitch_mm": 5}]`, `"place": [{"part": "D1", "at": "top"}]` |
| "The eight jacks down the left side, 13.79 mm apart" (vertical jacks on a narrow board) | `"row": [{"parts": ["J11", "J10", ..., "J4"], "pitch_mm": 13.79, "axis": "x"}]`, `"rotation": [{"part": "J11", "angle": 90}]`, `"place": [{"part": "J11", "x": 6.8, "y": 102.5}]`: a row's parts keep their angle within it, so a row of parts turned a quarter from its line is the other axis turned; it runs from its first part along the axis in that part's frame (turned 90 degrees: up the board, so the first part goes at the bottom; 270: down) ([constraints.md](constraints.md)) |
| "It's a keyboard" (a key matrix in the schematic) | copy `constraints.starter.json` to `constraints.json`, give it an `outline`, then decide what its `comment` asks (pitch, stagger, thumbs, the controller's edge) |
| "This half on the left of the board" | `"region": [{"parts": ["SW1", "U1"], "at": "left-half"}]` (`right-half`, `top-half`, `bottom-half`, `center`: on any outline) |
| "A 4 x 4 key matrix, 19.05 mm pitch" | `"row": [{"parts": ["SW*"], "pitch_mm": 19.05, "columns": 4}]` |
| "Each key's diode right at its switch" | `"near": [{"part": "D2", "part_of": "SW2", "max_mm": 3}, ...]` (one per key; the keys may be in a row) |
| "The thumb keys below the grid" | `"relative": [{"part": "SW16", "below": "SW1", "max_gap_mm": 10}]` (`SW1`, the row's first key, stands for the grid) |
| "WiFi antenna over the board edge" | `"edge": [{"part": "U1", "edge": "top", "flush": true, "overhang": {"edge": "top"}}]` (the footprint needs an antenna keepout zone; one without, such as the ESP32-S2-MINI-1 of trial 6's laptop power, is refused: then the edge entry alone and a `rotation` that turns the antenna end to that edge) |
| "Place the capacitor's supply pad as close as feasible to the IC supply pin" | `"near": [{"part": "C1", "pad": "1", "pin_of": "U1:3", "minimize": true}]`. Use `describe-kicad-board` to find the pad numbers on the supply net. Read the achieved centre-to-centre distance in millimetres from the `pad_near_objective` row. |
| "Keep those two pad centres within 2 mm" | `"near": [{"part": "C1", "pad": "1", "pin_of": "U1:3", "max_mm": 2}]`. The `pad_near` evidence gives the measured distance and any excess over 2 mm. |
| "Keep the capacitor body within 2 mm of the supply pin" | `"near": [{"part": "C1", "pin_of": "U1:3", "max_mm": 2}]`. A missed, unambiguous request reports the pin, measured gap and limit in `body_near_pin`. |
| "Keep the temperature sensor away from the regulator" | `"apart": [{"parts": ["U4"], "from": ["U2"], "min_mm": 10}]` |
| "Keep the power supply together, bottom right" | `"group": [{"parts": ["U2", "L1", "C1?"], "max_mm": 4}]`, `"place": [{"part": "U2", "at": "bottom-right"}]` |
| "Nothing under the display window" | `"keepout": [{"x": [10, 40], "y": [5, 25]}]` |
| "No copper under the antenna" | `"keepout": [{"x": [...], "y": [...], "copper": true}]` |
| "Battery holder on the back" | `"back": ["BT1"]` |
| "A HAT: the Pi's outline is fixed, parts go inside it" | `"fixed": ["J5"]`, `"hollow": ["J5"]` |
| "As small as possible" | `"outline": {"shrink": true}`: the smallest board a quick route completes (each size tried costs a placement and a route) |
| "The height can be anything, keep it under 100 mm" | in `layout.json`: `"free_dimension": {"edge": "bottom", "min": 34, "max": 60, "step": 2, "prefer_below": [100]}`: the smallest height whose layout comes out clean ([constraints.md](constraints.md#a-free-dimension-free_dimension)) |
| "100 x 60 mm with 3 mm round corners" | `"outline": {"width": 100, "height": 60, "corner_radius": 3}` |
| "It must fit the enclosure: an L, the right part 30 mm lower" | `"outline": {"polygon": [[0, 0], [115, 0], [115, 30], [230, 30], [230, 125], [0, 125]]}`; `"edge": "top"` for a part in the right part puts it against the step |
| "A window for the display, nothing in it" | `"outline": {"width": 80, "height": 50, "cutouts": [{"x": [20, 60], "y": [10, 35]}]}` (relative to the outline's corner; a `polygon` works too) |
| "I routed the USB pair and the crystal by hand, route the rest" | route the traces in KiCad (or through its API), save, then `route-kicad-board <dir> <id> <out> auto`: the board's own tracks and vias are kept and only what they leave open is routed ([below](#pre-routed-nets-keep-what-is-routed)) |
| "Throw away the old tracks" | `route-kicad-board ... --strip` |
| "Send it to JLCPCB" | `fab-kicad-board <out>/result <id> <release> --profile jlcpcb` ([5](#5-order-it)) |
| "The sensor board breaks off the main board" | `"outline": {"pieces": [{"width": 60, "height": 40}, {"width": 15, "height": 40, "x": 65}]}`; parts wired to a fixed part on a piece stay on it |

Router side (the file passed in place of `auto`):

| They say | Write |
| --- | --- |
| "Ground plane on the bottom" | `"add_pours": [{"net": "GND", "layers": ["B.Cu"]}]` |
| "In1 is a solid ground plane, no traces on it" | `"add_pours": [{"net": "GND", "layers": ["In1.Cu"], "plane": true}]` |
| "Keep the bottom ground solid under the MCU" | `"add_pours": [{"net": "GND", "layers": ["B.Cu"], "keep_whole": [{"parts": ["U1"]}]}]` |
| "Our fab fills and caps vias, vias in pads are fine" | `"via_in_pad": true` |
| "The BGA's inner balls need vias in their pads" (the verdict names them) | `"via_in_pad": ["U106"]` (only that part's pads no track leaves get a filled and capped via) |
| "Power traces 0.8 mm wide" | `"net_classes": [{"name": "Power", "nets": ["VBUS", "+5V", "GND"], "track_width_mm": 0.8}]` |

### Panels of copies

A panel of copies of one circuit (KiKit's tabbed copies, the verdict's
`panel_copies`) is placed as its designer would: the placer places the
first copy (`panel_copies.copies[0]`) in its region and copies that
placement to every other copy by its turn and offset; a copy whose
surroundings differ (an edge constraint, a board edge or a fixed part in
reach: `cell_classes`) is placed from its own copy only where many of the
copied parts are not legal in it (`by_cell_classes`), and what is left
(the copied parts not legal where they land, the parts in no copy) is
placed after it. What you can influence, by the code since 2026-11-04:

- **Which footprint an entry names.** The copies share their references
  (SmartSpin2k: nine `C9`). `fixed`, `back`, `front`, `hollow` and the
  patterns of `region`, `group`, `apart` and `row` name every footprint of
  the reference; an `edge`, `rotation` or `place` entry names one (its
  `uuid`, or in turn, the n-th entry the n-th footprint in board order:
  [constraints.md](constraints.md)); a `near` or `relative` entry names
  the reference's first footprint in board order (by reference, then
  uuid: any copy's), and its anchor (`pin_of`, `part_of`, `below`, ...)
  the anchor reference's footprint sharing the most nets with it (on a
  KiKit panel, whose copies' nets are apart, the same copy's; until
  2026-11-04 the first in board order, which could be another copy's).
- **A `near`, `relative` or `apart` between two parts of one copy holds
  in every copy**, whichever copy's parts it names: the first copy is
  placed with it carried to its own parts of the same circuit, and the
  copies copy it. One `{"part": "C9", "pin_of": "U1:5", "max_mm": 1}`
  is enough for all nine (until 2026-11-04 it held only where it named
  the first copy's parts, and else nowhere: the copying overrode it).
- **An `edge`, `region`, `place` or `rotation` entry holds in the copy
  whose footprint it names**: the copied part that does not keep it is
  placed again after the copying, in its copy's region (where its region
  lets it keep the constraint). So does a `near` or `relative` towards a
  part outside the copy (a shared connector, another copy's part).
- **A stranded edge connector on a single-piece panel** can use a nearby
  slot that straddles an inferred cell boundary. The search keeps its
  side, rotation, authored regions and edge limit. Its centre stays within
  the cell's depth and at least a quarter of its width remains within the
  cell along the edge. The board outline, cutouts and clearance still
  apply to every proposed pose.
- `describe-kicad-board` lists each footprint's `uuid` and position, and
  `panel_copies.copies[]` each copy's region and its references in the
  first copy's order: the r-th reference of every copy is the same part.
  A panel's frames, tabs and V-cuts take no parts ([placer.md](placer.md),
  "Frames on panels of copies"); connectors may reach over them.

## 3. Run

```sh
echo '{"placer": {"constraints": "constraints.json"}}' > layout.json
pcb-maker layout-kicad-board <project-dir> <board-id> <out-dir> auto layout.json
```

- **Class clearance warnings.** Route and layout verdicts expose active
  KiCad netclass warnings in `class_clearance_warnings`, with the
  configured and measured gaps, exact item IDs, available net names, and
  locations from `drc.json`. A report can show these warnings with a
  `complete` status under the project's configured severity. Inspect
  track and via examples for a legal route at the configured gap. Review
  pad-pair examples in the footprint and placement.

- **Panel assignments.** Add `--check-sub-boards` to the layout command
  to check the configured outline and sub-board assignments before the
  placement and routing search. The check prints JSON to stdout and
  leaves `<out-dir>` available for the layout. Exit 1 means that
  `next_actions` has an assignment to review. On panels of separate
  boards, supply the placed project with `--reference`, or group parts
  into `region` constraints with `"origin": "absolute"`. Use the
  `sub_boards` boxes as a starting point and verify any piece inferred
  to be a frame. Rerun the check with the same layout config and updated
  inputs. Exit 0 means the assignment check found no blocking issue or is
  disabled (`enabled: false`). Check `unassigned` for parts that still lack
  ownership. KiCad DRC and placement legality are checked in the layout.

  On a panel that retains source-sheet metadata, `sub_boards.sheet_ownership`
  lists movable parts by sheet file and instance path. A
  `suggested_sub_boards` entry proposes a physical piece for a witnessed
  sheet subtree, including children without their own fixed parts.
  Confirm the destination and copy its `constraint` into `sub_board`
  in the placement brief. Each entry identifies the piece by a point
  inside it, including where its bounding box overlaps a fabrication rail.

  ```sh
  jq '[.sub_boards.sheet_ownership.suggested_sub_boards[]? | .constraint]' panel-check.json
  ```

  A `suggested_region`
  needs at least two fixed electrical witnesses on one piece, agreement with
  existing assignments, and a separately witnessed group on another piece.
  Append the confirmed rows to `constraints.json` under `region` and rerun
  the check. To collect the proposed rows from a saved report:

  ```sh
  jq '[.sub_boards.sheet_ownership.groups[] | .suggested_region // empty]' panel-check.json
  ```

  For a group with no suggestion, use its `unresolved_parts` as the starting
  list for a region once the board's mechanical brief identifies its piece.
  Review `unclassified_parts` separately. The preview lists all outline
  boxes, including pieces without assigned parts. Those can be boards or
  panel rails. The preview leaves every proposed assignment for you to confirm.
  Once explicit groups locate two sheets on separate pieces, net names shared
  across sheets no longer place unassigned parts by association. `movable_held`
  counts movable parts actually held on their assigned pieces by the configured
  placement, including those with explicit regions.
- **`auto`** takes the design rules (net classes, clearances, widths,
  vias) from the project's `.kicad_pro`.
- **Try another placement.** When KiCad finds open connections and the
  movable placement and its constraints otherwise pass, `next_actions`
  can give a command to try a different placement in a sibling output
  directory. `--placement-seed N` sets the starting seed for the placer.
  Reuse the original project, router and layout files, and any `--reference`
  option. The retry uses the same constraints, design rules and work budget.
- **Only routing?** When the placement is already final, use
  `pcb-maker route-kicad-board <project-dir> <board-id> <out-dir> auto`.
  The board's own tracks and vias stay (pre-routed nets, below);
  `--strip` routes from nothing.
- **A designer's board as the reference.** When the board comes from a
  finished design, `--reference <its-project-dir>` makes its own DRC
  errors pre-existing (see 4). It also tells a panel of different boards
  which board each part belongs to where the nets cannot (`sub_boards`,
  in `place-kicad-board` too), and its vias in a footprint's small pads
  admit vias into that footprint's enclosed pads (`via_in_pad`, below).
- **Router options.** Pass a JSON file instead of `auto`, for example
  `{"seeds": 4}`. The rules still come from the project: they are the
  base, and a key the file sets replaces the project's value of that key
  only (`edge_clearance_mm`, `hole_clearance_mm`,
  `solder_mask_to_copper_mm`, ...). `connection_rules` and
  `default_rules` together are the classes: a file setting either routes
  by its own classes, still with the project's other rules.
  - `seeds`: route that many ways in parallel and keep the best: fewest
    opens, then vias, then copper. Worth it on idle cores; best of 4 took
    PIC from 2 vias to 0.
  - `pours`: `auto`, `connect` or `tracks`.
  - `add_pours`: planes to add before routing, such as
    `[{"net": "GND", "layers": ["B.Cu"]}]`. Each covers the board outline;
    pads connect to it, and islands are stitched with vias. A layer where
    that net already has a zone is skipped, also when the zone covers a
    layer set (`F&B.Cu`, `*.Cu`; until 2026-10-26 a second zone was
    written over it, a `zones_intersect` error). With `"plane": true` no track
    of any net goes on those layers, only vias through them: a rule area
    keeping tracks off each layer is written beside the zone (also where
    the zone exists), so every rung of the ladder and KiCad's DRC keep
    the plane whole. This replaces drawing the rule area by hand (a
    per-layer net restriction in KiCad's sense does not exist; the rule
    area is how KiCad says it).
  - `keep_whole` in an `add_pours` entry: regions where that pour stays
    whole on its layers, for a two-layer board's ground that `plane`
    would make unroutable. Each entry is `{"parts": ["U1", "U4"]}` (each
    part's courtyard, else its pads, grown by `margin_mm`, default 0.5)
    or a box `{"x": [0, 76.2], "y": [30, 33]}` (from the outline's
    top-left corner unless `"origin": "absolute"`; `margin_mm` default
    0). Other nets' tracks stay off the layer there and their vias out;
    the pour's own net may pass, so its fill there is one piece. Each
    region is written as a rule area `keep whole <net> <layer>` with
    everything allowed (KiCad has no rule by net; it only marks the
    region), which the router lowers by its name. The parts are taken
    where they are when the pours are added (in a layout, after
    placement). Another net's pad on that layer inside a region can
    only be left on the other side: keep regions off them. The input
    check warns of it before the run, naming the pads (in the log and
    first in the verdict's next_actions: `keep_whole of the GND pour on
    F.Cu covers 22 pads of other nets on that layer (U2:1, U2:2, ...)`):
    in route mode every pad inside a region, in a layout (which places
    first) a named part's own pads, which move with it. A region under a
    signal IC on its own layer is the case (the trial's board A,
    2026-10-26: 22 connections opened); under the IC on the other layer
    of a two-layer board it is what the cookbook's "keep the bottom
    ground solid under the MCU" means.
  - `via_in_pad` (**vias in BGA pads**, since 2026-10-28): by default no
    via goes inside an SMD pad but an exposed (thermal) pad; a pad's own
    net's vias sit beside it. A via in a reflowed pad takes its solder
    unless the fab fills and caps it (via-in-pad, IPC-4761 type VII), at
    extra cost. A fine-pitch BGA's inner balls have no other way out,
    though: at 0.5 mm pitch no track passes between two balls. Where the
    designer's board holds vias in a footprint's small pads (the input
    board's own vias, or the `--reference` board's), the designer has
    told the router the fab does it: each pad of that footprint (same
    reference, or same footprint name) that no track at the neck width
    can leave between its neighbours gets a filled and capped via of its
    net at its centre, of the designer's size where it fits, else of the
    largest of the project's via sizes that keeps every clearance (the
    other side's parts too: the via goes through; a pad 2 mm across or
    more gets it off its centre where the centre does not fit, and no via
    goes into a rule area without vias). A pad inside another net's pad
    of a small footprint (a dome switch's centre in its ring) counts as
    enclosed too, and a via in a large pad on the designer's board is
    evidence as well (since 2026-11-06, Sisu's keypad). Other pads and
    other footprints stay closed. For a board without such a reference (from a
    netlist), name the parts or footprints: `"via_in_pad": ["U106"]`
    (a reference or a footprint name, with or without its library), or
    `[{"part": "U106", "via_mm": [0.35, 0.25]}]` for a via size of your
    fab's. `true` lets vias into every SMD pad (all vias filled and
    capped) and puts one in every such enclosed pad; `false` none, the
    designer's neither. board-router.json's `vias_in_pads` says what was
    done: `placed`, per footprint the `rule` that admitted it (`the board
    has 10 vias in U106's pads B2, ...` or `via_in_pad: ["U106"]`), the
    `via_mm` and the `pads`, and under `closed` the footprints whose
    enclosed pads no rule admits, each with `why` (route.log's `pads no
    track leaves, no via allowed`); the verdict repeats them as
    `closed_pads`, with the nets among them that are open, and the open
    connections' action points there. The verdict groups equivalent land
    patterns under `via_in_pad_retries` when KiCad names those exact pads
    as open endpoints. The report lists the pad UUIDs, nets and references.
    `router_config_patch` gives explicit part entries to merge into the
    existing `via_in_pad` list when the fabrication process permits it.
    The through vias need clearance on the opposite side.
    `describe-kicad-board` lists enclosed pads before routing
    (`vias_in_pads`). The vias are written
    `(capping yes) (filling yes)`, so KiCad's fab outputs carry the
    process, and score-kicad-board's "vias in pads" does not count them
    (`filled_vias_in_pads` does; `vias_in_pads_at` lists every via in a
    small pad, filled or not, `vias_in_large_pads_at` those in large
    pads, which no count includes).
  - Open connections at a fine-pitch row: run with
    `PCB_ROUTER_TROUBLE_WHY=1` in the environment to see which open pads
    are boxed in by a neighbour whose route must cross theirs (the log's
    `crossing census` lines; docs/router.md "Crossings decided in the
    repair").
  - `bga_fanout` (default true; docs/router.md "Prepared fan-outs for
    BGAs"): a ball grid array's inner balls, which no track between the
    outer balls serves and whose net's via is too large for the diagonal
    gap between four balls, each get a short track and a via in that gap
    before routing (the largest of the project's via sizes that keeps its
    clearances, the canonical dog-bone); the route ends on the via on any
    layer, and a dog-bone the route does not use is not written. The
    placer keeps the board's other side free under those vias. Set it to
    false to route without; run with `PCB_ROUTER_BGA_FANOUT_WHY=1` to see
    each array's plan (the balls planned, kept, and refused where a part
    on the other side or a neighbour's copper takes the gap). The
    placer's `bga_room` (default true since 2026-11-04; layout.json
    `{"placer": {"bga_room": false}}` turns it off; docs/placer.md "Room
    around grid arrays") keeps room around an array whose inner balls
    take those dog-bones: no part within a band on its side (the outer
    rings' tracks and a row of the class's vias: 1.5 mm beyond the balls
    at Glasgow's 0.8 mm pitch), and on the other side only the array's
    own decoupling capacitors under it, off the fan-out's vias;
    board-placer.json's `array_rooms` says each band and which parts it
    let in (Glasgow: 343 -> 262 unconnected over three seeds). Arrays
    whose inner balls leave by vias in their pads or on the surface keep
    no room.
  - `pin_lines` (default unset: the router decides; docs/router.md
    "The rows decided at the probes (2026-11-08)" and "Lattice lines where
    the rows are open (2026-11-08)"): the router's lattice gets lines
    through the pads of fine-pitch rows it misses. Unset, route mode
    decides twice: at the probes, where the leader's probe leaves pads
    unfinished at such rows, its rung is probed once more with the lines
    and they stay on when that probe leaves a tenth fewer nets unfinished
    (the run log: `probe pours=... with lines at the pin rows: 18 nets
    unfinished ...: it leads`); else after the ladder's leader's run when
    KiCad's open pads are in such parts (`open pads at pin rows the
    regular lattice misses (U19, U15)`). A board that completes without
    them never pays for them. Set it true in route mode when the open
    connections are pads of a 0.4 to 0.5 mm row with nothing else in the
    way and neither log line shows (a board without probes: fewer than
    four layers or 120 connections, on a short budget; the census line
    `no straight way out` under `PCB_ROUTER_TROUBLE_WHY`, or `dead_pads`
    on the row with no wall named); on every attempt it moves other
    boards' routes (+3 -2 strict passes over the designer tier). False
    keeps the regular lattice. Whether the lines were on and why is in the
    report (since 2026-11-08, trial 8: only the run log said so):
    `verdict.pin_lines` and board-router.json's `pin_lines` (`decided at
    the probes: the probe pours=tracks left 74 nets unfinished with pads at
    pin rows the regular lattice misses, 59 with lines at those rows, so
    the lines stayed on ...`, `decided after the leader's run: open pads at
    pin rows ... (U19, U15) ...`, or `on: pin_lines true in the router
    config`), stdout's `pin lines: ...` line and each attempt's
    `attempts[].pin_lines`; absent where the run had none. A layout
    decides them itself too (since 2026-11-08, docs/router.md "A layout's
    rows decided by its probe's open pads"): where the race's probe of the
    placement it keeps leaves pads unfinished at rows the regular lattice
    misses, the first route starts over with the lines and every route
    after it (the moves, the final ladder) has them (`decided at the kept
    placement's probe: ...` in `verdict.pin_lines` and board-layout.json's
    `pin_lines`); otherwise the final ladder decides at its probes as
    route mode does. Set it true in a layout only where the verdict's
    action names it (a hot spot at a 0.5 mm row or finer that the layout
    routed without lines: its probes left no pad open at a missed row);
    false keeps every route of the layout on the regular lattice.
  - `blind_vias` (default false; off by the evidence of 2026-11-05; docs/router.md "Blind, buried and
    micro vias"): on a board of three or more layers that allows them,
    a via that joins two layers pierces only the shortest allowed span
    holding both, so the layers it skips stay free for other nets, and
    is written as KiCad's blind, buried or micro via (KiCad's DRC
    accepts them: copper and a hole on their span only). What allows a
    span: `via_spans` in the router file (`["F.Cu-In1.Cu:micro",
    "In1.Cu-In4.Cu"]`, or the keywords `"blind"`, `"buried"`,
    `"micro"`), else the designer's own vias of those spans on the board
    or the `--reference` board (board-router.json's
    `rules.via_spans_designer` lists them), else a KiCad 6 or 7
    project's "allow blind/buried vias" and "allow micro vias". KiCad 8
    and later have no such switch: on a fresh board say `via_spans`
    yourself, and only when the fab builds them (they cost extra
    laminations). A route of a designer's board built with such vias
    (the input before `--strip`, or `--reference`) with the setting off
    gets a next action naming the board's spans and `"blind_vias": true`
    (since 2026-11-08, trial 8: the OpenAIO-Whoop, an HDI 1+6+1 board with
    2479 micro and 465 buried vias, came back with through vias only and
    144 unconnected, 114 with the setting). Pour nets keep through vias; a BGA's dog-bone takes
    the shortest span to a layer it can route on (a micro via at the
    project's micro via size). `PCB_ROUTER_BLIND_VIAS=1` forces it on.
  - `automatic_planes` (default true): a board with four or more copper
    layers and no pours gets ground as a plane on the first inner layer
    and the supply rail with the most pads on the last inner layer, as a
    designer would lay it out. `add_pours` replaces this.
  - `exclusive_planes`: an inner-layer pour over the whole board is a
    plane. Left unset, the router first keeps other nets' tracks out of it
    (it stays whole) and, if connections stay open, then lets them cut it
    as designers do (`plane_cut_cost`, default 3, prices that). `true` or
    `false` picks one.
  - `net_classes`: track widths and clearances for named nets, such as
    `[{"name": "Power", "nets": ["VBUS", "+5V", "GND"], "track_width_mm": 0.6,
    "clearance_mm": 0.3}]`. The fields are `via_diameter_mm` and
    `via_drill_mm` besides those two. Nets take KiCad wildcards (`*`, `?`)
    and may leave out the sheet path's leading `/`. The first class naming
    a net wins. The classes go into the result's `.kicad_pro` ahead of the
    project's own, so KiCad and its DRC use the same rules. Without them,
    net classes come from the project.
  - **Below the project's minimum** (since 2026-10-23): a class's,
    `default_rules`' or a net's (`connection_rules`) track width below the
    board's `min_track_width`, a clearance below its `min_clearance` (and
    `default_rules`' or `connection_rules`' below the project's class
    clearance), `neck_width_mm` or `narrow_signal_mm` below
    `min_track_width`, `relaxed_clearance_mm` below `min_clearance`: the
    run is refused before any work (exit 2), one line naming each value,
    the project's minimum and where it comes from (`net_classes[Narrow]
    asks for track width 0.15 mm below the project's 0.2 mm
    (board.design_settings.rules.min_track_width in the .kicad_pro) on
    QSPI_SCLK`). KiCad's DRC holds the board to the project's rules, so
    narrower classes give width findings. To route with them anyway (a
    diagnostic control), add `"allow_below_project_minimum": true`: the
    run goes on, board-router.json's `rules.overrides` lists them and the
    verdict's first next actions warn. The verdict's `rule_overrides`
    (since 2026-10-25) says per value what became of it:
    `requested_mm`, `project_minimum_mm` with `project_source`,
    `routed_with_mm`, `emitted_track_widths_mm` (the widths written on its
    nets) and `emitted_below_minimum` (how many tracks are narrower than
    the minimum; both `null` for a clearance), `native_required_mm` (what
    KiCad's DRC holds the board to: the project's minimum, unchanged) and
    `native_findings` (KiCad's `track_width` or `clearance` findings on
    its nets; `null` when KiCad did not check). `describe-kicad-board` on the
    result shows both: `track_width_mm` (the project's, raised to the
    minimum) and `class_track_width_mm` (the class's own, which that run
    routed with); it does not report a changed minimum, since the
    project's is unchanged.
  - `existing_tracks` (default `keep`): route mode's handling of the
    board's own tracks and vias; `strip` is `--strip`.
  - `retained_pad_tips`: exact pad names for the detailed Keep router,
    for example `["U1:42", "U1:43"]`. See the retained-tip procedure below.
  - `prepared_row_escape_pads`: two to four exact consecutive pad names
    of one dense surface row on a fixed placement. Use on copper-free
    input or with `--strip`. See the prepared row escape procedure below.
  - `diff_pairs` (default by need; `PCB_ROUTER_DIFF_PAIRS=1` or `0`
    where the settings name none): route mode routes differential pairs
    (KiCad's names: `D+`/`D-`, `USB_DP`/`USB_DN`) as coupled runs at the
    project's pair width and gap, negotiated with the other nets, their
    nets kept to the layers the rules allow. It is on by default where the
    project's custom rules ask for pairs (a `diff_pair_gap` or
    `diff_pair_uncoupled` constraint holding for a paired net; a net
    class's diff pair width and gap do not), off otherwise; `true` or
    `false` always wins. The verdict's `diff_pairs` says whether it was on
    and why (`default`: `rules`, `off` or `explicit`) and what became of
    the pairs. Without such rules it can cost connections on dense boards
    (docs/router.md, "Differential pairs"): compare with a run without it.
    **In a layout** the default is off, whatever the rules ask: give
    `"diff_pairs": true` in the router config
    (`layout-kicad-board <dir> <id> <out> router.json layout.json`) to
    couple the pairs there too. A pair couples between two parts whose
    pads of the pair lie side by side (at most 3 mm apart, no other net
    between them). A pair with no such link (since 2026-11-05) also takes
    a part's two pads with a ground pin between them as such a part (an
    HDMI receptacle's or an HDMI ESD part's shielded pairs), except where
    the pin would be walled in by the stubs (a 0.4 mm pitch USB mux: the
    pair's `why` says "the pin between its pads (U17.4) would be walled in
    by its stubs"). A pair's AC-coupling or series capacitor the placer
    put apart from its partner leaves the pair uncoupled ("fewer than two
    parts ... side by side"; a run from the one part to a split end near
    such parts was built and measured, and left out, again with a gate on
    2026-11-06: docs/router.md, "A pair point across a pin" and "Split
    ends with a gate"): a `row`
    of the two (`{"parts": ["C150", "C151"], "pitch_mm":
    1.0, "axis": "y"}`, a pitch no shorter than the parts along the axis)
    keeps them side by side. The placer's `pair_rows` (layout.json
    `{"placer": {"pair_rows": true}}`, off by default; since 2026-11-04)
    makes those rows itself: two free parts of one footprint with two pads
    each, whose pads of one number lie on the two nets of a pair (the
    pair's AC-coupling capacitors or series resistors) become a row of two
    when they share a face and have no authored position or rotation
    constraints. The row runs across their pads at the
    pair's track width plus gap where the footprint allows it (else the
    courtyards' width), turned so that each side faces its own nets' far
    pins (the pair's source and destination; any quarter turn where they
    lie to one side); `constraints[]` lists each as `pair_row` with a
    `detail` (the pitch and why, and whether it faces the pair's ends)
    and stderr says `placer: pair rows: ...`. Measured on right_io,
    left_io and jetson, two seeds each (docs/placer.md, "The pair's pitch
    and the pair's ends"): where the pairs route coupled it couples more
    (1 -> 4, 0 -> 6, 3 -> 6 pairs), unconnected within the seeds' spread,
    but `diff_pair_gap_out_of_range` rises. **Every route of a layout
    plans the pairs** (since 2026-11-04: the first route, the placement
    race's probes, the moves, the final ladder), so the board kept is
    coupled where coupling is possible whichever stage made it, and
    `verdict.diff_pairs` is always filled when `diff_pairs` is on: no
    need to route the layout's `placed` board in route mode for the
    pairs (before, where the final ladder did not run or was not kept,
    the first route stood uncoupled with `verdict.diff_pairs` null). A
    run where only the first route ran and left pairs uncoupled says so
    in one next action (`diff_pairs: only the first route ran ...`) with
    the lever. The price: the links are nets of every route, and on a
    dense board with many pairs they cost connections (jetson-nano-
    baseboard: 23 and 44 unconnected with `diff_pairs`, 0 and 9 without,
    5 to 8 of 75 pairs coupled; docs/placer.md, "Following diff_pairs"):
    compare with a layout without it, and with `diff_pair_give_up` below. `pair_rows` does not follow
    `diff_pairs` (measured: no more pairs coupled on left_io's seed 101,
    unconnected up on four of six runs); set it where the rows help.
    Trial 5's ducktop2 right_io (21 pairs,
    `.kicad_dru` pair rules): the default layout coupled none (54
    `diff_pair_gap_out_of_range`, 12 unconnected); route mode on its
    placement coupled 1 and part of another (19 fell back, 13 of them for
    want of two parts side by side); `diff_pairs: true` with rows of the
    AC-coupling capacitors coupled 7 and part of an eighth (9 unconnected).
    A `diff_pair_*` finding's next action says which of these happened:
    the setting where the pairs were off, else the counts and, per pair,
    why the links fell back (board-router.json `diff_pairs.pairs[].why`).
    For a native-checked board with zero opens,
    `verdict.diff_pairs.fallback_lengths` gives the exact net names, pads and
    planar track lengths of simple pairs routed separately. Use those net
    names to scope a KiCad `skew` rule if the electrical brief specifies a
    timing limit. [Skew on separately routed pair members](router.md#skew-on-separately-routed-pair-members)
  - `diff_pair_give_up` (default off; `PCB_ROUTER_DIFF_PAIRS_GIVE_UP=1`
    where the settings name none): a link the negotiation finds no path
    for, or whose conflicts stop falling once the price of sharing is high,
    is given up there; its pair's two nets route uncoupled from then on,
    in the same attempt, and the hard repair tries the link once more
    (docs/router.md, "Giving up on a link"). The verdict's
    `diff_pairs.given_up` counts the links given up, `not_routed` names
    them "given up in the negotiation: no room" (or "no path"). Where a
    dense board with many pairs loses connections to the pairs, try it:
    on jetson-nano-baseboard's layout it left 10 and 16 unconnected where
    the default left 23 and 44, with as many pairs coupled (9 and 5); on
    ducktop2's left_io it cost seed 101 3 of its 6 coupled pairs and 16
    connections (seed 1: 15 fewer unconnected, one pair less), so it is
    not the default. Compare the two verdicts.
  - `diff_pair_return_vias` (default on with the pairs;
    `PCB_ROUTER_DIFF_PAIRS_RETURN_VIAS=0` where the settings name none)
    and `diff_pair_return_net` (default `GND`): a via of the reference
    net beside each via of a coupled run where one fits (docs/router.md,
    "Return vias"); the verdict's `diff_pairs.return_vias` counts them.
  - `length_matching` (default on; `PCB_ROUTER_LENGTH_MATCHING=0` or `1`
    where the settings name none): the project's custom `length` and
    `skew` rules over single nets and groups of nets. After the route a
    net shorter than a `length` minimum, or further than a `skew` maximum
    below the longest net of its rule, gets a meander on a straight track
    of its route (a KiCad tuning pattern), in route mode and on a
    layout's own routes alike. A net over a `length` maximum after the
    route is routed again the short way, pushing the nets in its way
    aside, where that opens nothing (no hard bound: a placement that puts
    the net's pads further apart than the maximum leaves it over). The
    verdict's `lengths` says per net what came of it. Only where the
    project has such a rule; elsewhere nothing changes. With
    `length_reroute` (off by default: on the firestarter it met no more
    lengths) a net still below its minimum after the meanders is routed
    again with room beside it for the bumps and meandered again, kept
    where it ends nearer the rule (docs/router.md, "Length minima: the
    placer's share and the reroute (2026-11-04)").
    In a layout the placer keeps every two pins of such a net within 0.8
    of the maximum (`length_maxima`, on); with `length_minima` (off by
    default: on trial 6's firestarter it met more lengths and lost
    connections) it also keeps a two-part net's pins at least half the
    minimum apart: closer, no route with meanders makes up the minimum.
    board-placer.json's `length_rules` says per net what the placer held
    and where the pins ended (with either switch), and the constraints
    template names the rule with its nets and parts. A minimum the layout
    leaves short: lay out again with `{"placer": {"length_minima": true}}`
    and compare, or hold the parts apart with an `apart` entry
    (docs/constraints.md). What still needs `strip-kicad-tracks
    --keep-nets` (Other commands) or tracks routed by hand: a dense bus on
    a fixed placement whose inner nets have a neighbour at the clearance
    on either side (route mode on the firestarter's designer placement:
    the reroute finds them no room, six of ten stay short), and a minimum
    that needs a layer the route keeps for a plane (the designer's DVP
    tracks run on In2.Cu, ours keep it for the GND pour). The
    `length_out_of_range` action says what the meanders added, what is
    missing and which lever is left.
- **Router options in layout mode.** The same file goes in the router
  config position:
  `pcb-maker layout-kicad-board <dir> <id> <out> router.json layout.json`.
  - `pin_swaps`: see below.
- **A side that may be any length.** `free_dimension` in the layout
  config names the edge that moves and the sizes to try (`"edge":
  "bottom", "min": 34, "max": 60, "step": 2`, optional `prefer_below`
  price thresholds and `band_mm`, what moves with the edge): the layout
  runs at a third of a third of `total_seconds` per size, several sizes
  side by side, from the smallest upward, then gives everything left to
  a smaller size that came close (results are not monotone) or to the
  first clean size again, and returns the smallest clean layout as its
  result, with `free-dimension.json` listing every size tried
  ([constraints.md](constraints.md#a-free-dimension-free_dimension)).
  `total_seconds` is then the whole search's budget: give it several
  layouts' worth (the fence board: 6000).
- **Pin swapping.** `"swappable": [{"part": "U1", "pins": ["GPIO*"],
  "except": ["GPIO0"]}]` in the layout config lets pcb-maker reassign
  interchangeable pins (pad functions or numbers, globs); `"pin_swaps":
  "pin-swaps.json"` takes the full format (resistor networks, groups
  across parts, restrictions). The schematic is rewritten to match
  ([pin-swap.md](pin-swap.md)). `describe-kicad-board` lists
  `swap_candidates`: parts (not connectors) with three or more connected
  general-purpose pins (`GPIO12`, `IO5`, `PA3`, `P0.13`) and the globs for
  them (`by: function`). A part whose pads name no function on the board
  (a generated board: the names are in the schematic only) is listed by
  its pins' electrical types instead (`by: electrical type`, with its
  symbol `unit`): bidirectional or passive pins of one symbol unit, on
  signal nets (not ground or a rail) that reach another part, three or
  more on three nets, the type from the pad's `pintype` or else from the
  schematic's symbol, and the pad numbers as the globs. Not diodes,
  transistors, switches, crystals and the like by their passive pins. A
  resistor network's elements are listed pin by pin; in `pin-swaps.json`
  they swap as pairs (units). On the breadboard fence (its CH32X035's
  pins are PA15, PA0/A0, ... in the schematic only) that is U1 (44
  bidirectional pins), U4 and U5 (the switches' 24 passive X and Y ports)
  and RN1-RN8. Ask whether the firmware can use any of them for any signal (a
  GPIO matrix, PIO, bit-banged buses) and which pins are special (boot
  straps, a UART the bootloader uses, ADC-only pins): those go in
  `except`.
- **Output.** `<out-dir>/result/` holds the finished project, copied, with
  the placement and the copper. `<out-dir>/placed/board-placer.json`
  describes the placement in the adjacent board. Each footprint row
  records its `at` coordinates, mounting `side`, and UUID when available.
  `placement.html` animates the placement or shows the selected layout
  poses when the layout changed them.
- **Iterate quickly.** Work in three steps:
  1. Check the constraints alone, without routing:
     `pcb-maker place-kicad-board <dir> <id> <out> <dir>/layout.json`
     (`--reference <project-dir>` on a panel of different boards, as for
     the layout) places as the layout of that layout config does (since 2026-10-27):
     its `placer` object, the placement's 40 % of `total_seconds` (600 s
     of work by default), the race's 16 seeds and the escape demand
     measured with the router config the layout will take (`--router
     <router.json>`, default `auto`: the project's rules); a file holding
     only the placer object works too. It pins the reference's work rates
     (`3.15e6,150e6`, `work_rate_source` `pinned reference`) unless
     `PCB_WORK_RATE` or `--measure-work-rate` says otherwise, so that its
     answer repeats on a loaded machine; at the same rates its placement
     is the layout's byte for byte (the OpenESC 4in1 panel,
     framework_mobo_lefthalf and MokyaLora at the reference's rates; the
     panel at the trial's measured rates gave the trial layout's 8 parts
     without a place). Until then it placed three seeds on the escape
     demand's estimate at measured rates: the panel's check placed every
     part where the layout left 8 without a place, and framework's C1
     missed by 1.60 mm on one run and 4.33 mm on another. A layout
     measures its rates: when its parts find no place, its verdict names
     the rates it placed at and the `PCB_WORK_RATE` that repeats its
     placement. The check takes what the layout's placement takes (the
     seeds' pool), about a minute to several on a large board. What cannot fit is said at once, with exit
     2: a `region` smaller than its part, a `row` longer than the outline
     (as it may turn), a `row` whose pitch puts two of its parts' pads on
     each other (since 2026-11-04; trial 5), parts held at one edge that need more length than
     the edge has. Parts that need more area than a side has even with
     their tightest bodies, or that cannot lie side by side in strips of
     the board, are a no in seconds (exit 1, `"overfull": true` in
     `board-placer.json`). Either way `verdict.json` has `room`: what is
     missing, and an outline to try. Constraints missed on a side fuller
     than the target (70 %) while the other holds under half of it add
     both sides' shares and the `back` (or `front`) lever to their action
     (since 2026-11-04; trial 6's eurorack-pmod: the front 121 % full, the
     back empty, eight `near` entries missed by up to 8.4 mm; the
     designer's board has 27 parts on the back). A board that is merely crowded is
     the slow case: the placer tries looser levels before it reports
     parts as unplaced, up to the layout's placement budget (600 s of
     work per seed by default; urchin's 80 parts on 80 x 40 mm without
     rows: about seven minutes); `PCB_PLACER_WORK_SECONDS=60` gives a
     quicker answer (placed plain: see the race over skeletons below).
     **The race over skeletons** (on by default since 2026-11-05;
     docs/placer.md, "A race over skeletons" and "The skeleton's budget"):
     each seed places the skeleton first (every part but the free
     two-pad resistors, capacitors, inductors, ferrites and fuses), then
     the passives around it, held. It runs where a seed's placement work
     is at least `min_work_seconds` (500 s): the layout's default budget
     gives 600, so a layout or a place check at the defaults places by
     skeleton; a layout with `total_seconds` under 1250 or a place check
     with `PCB_PLACER_WORK_SECONDS` under 500 places plain, as with the
     race off (a quick check then answers for the plain placement, not
     the layout's). `board-placer.json`'s `skeleton_race` says what ran,
     first in one line, `summary` (since 2026-11-06, trial 7; also on
     stderr as `placer: skeleton race: ...`): `not run: each seed's
     placement had 200 s of work, under min_work_seconds (500 s), so all
     3 seeds placed plain, ... a layout's total_seconds of 1250 or more
     ...`, or `ran: the kept seed placed its skeleton of 68 parts first
     (level 3) and its 129 passives around it (level 3); 12 of 12 seeds
     placed by skeleton (0 fell back to a plain placement); ...`
     (MIDAS-MK2 at 500 and 1500 s), or `ran without a skeleton: none of
     the N passives was free (a constraint the user wrote names each, or
     it is fixed), ...` where every passive is held by a constraint (a
     region the tool gives an outline's piece does not count); on a
     panel of copies or sub-boards
     the race does not run and the block is absent (the stderr line says
     so). The summary is also the verdict's `skeleton_race` (place check
     and layout), and a layout left with open connections whose placement
     ran plain for want of budget gets a next action naming the
     `total_seconds` that places by skeleton (MIDAS-MK2: 1 unconnected at
     500, none at the default 1500; one board, a lever to try, not a
     rule). Then `budget_ok` (false: every seed plain, and the level fields
     below 0 for that reason, not full halos), `work_seconds` (the seed's
     placement work) and `min_work_seconds`; for the kept seed
     `skeleton_parts`, `absent` (the passives left out), `held`,
     `fitting_level`, `skeleton_level` and `level` (the relaxation levels
     its skeleton and its passives seated at: 0 is full halos, 7 and later
     tight bodies), `fallback` and `fallback_kept` (the skeleton or the
     passives left parts without a place and the seed was placed plain);
     over the kept seeds `seeds_fallback` and each phase's work. Turn it
     off with `"skeleton_race": null` in the placer settings (layout:
     under `placer`), e.g. to compare with the plain placement.
     `<out>/board-placer.json` reports every constraint, `unplaced`,
     `hints`, `utilization` and `room`; `unplaced_parts` has each
     unplaced or illegal movable part with what holds it (`held_by`) and,
     for an illegal one, `reason` (since 2026-11-04): the first part it
     meets and the test, at the placement's level, such as `bodies
     overlap (no courtyard) with SW15` (two footprints without a
     courtyard whose pads, copper and fabrication outlines overlap: a
     key on a key), `a slot of one meets the other` (a footprint's own
     Edge.Cuts hole under another part, its copper or a board cutout),
     `bodies overlap`, `shaped or hollow boxes`, `pads in a cutout or
     pad keepout`, `far side`, or `the outline or a hard constraint`.
     `local_diagnosis` gives the side and relaxed body bounds, then the
     conflicting footprints or board features at the retained pose. For
     a part held to a region, `region.by_rotation` gives its body size
     and the permitted center rectangle at each allowed rotation. If one
     retained body blocks the entire region, `region.blocked_by` names it.
     Where a rectangular-body bound proves the pair cannot fit anywhere on
     their current sides, `region.packing_conflict` gives the available
     width left, above, right and below the region-constrained part and
     the other part's minimum projected width and height. The verdict
     leads with the applicable change to review.
     If the selected seed found room by searching farther across the
     board, its placement report includes `sparse_legalization.seated`
     and `sparse_legalization.work_seconds`. The search uses the
     supplied outline, placement grid and constraints. In a layout run,
     read `result/board-placer.json` for the seed used by the routed board.
     With the placer's `pad_exits` on (off by default;
     `{"placer": {"pad_exits": {}}}` or `PCB_PLACER_PAD_EXITS=1`; since
     2026-11-04) it also has `pad_exits`: `closed` lists the pads of the
     placement that no track of their class can leave (the router's
     lattice, on the fixed copper alone, has no path out of the pad's
     pocket at the class's width; a narrower rung of the router may still
     reach one), each with `reason` (`pad U1.21 has no way out (blocked by
     C29.1, U1.20, U1.22, U1.57)`), `blocked_by` (other parts' items
     first) and `footprint_only` (its own footprint walls it in: only a
     via in the pad or another footprint helps); `moves` the parts the
     placer moved or turned after each seed to open such pads. A place
     check with a pad another part or the edge closes is `incomplete`
     (`N pads without a way out (...)`) and its next actions name the
     parts to turn or move; a layout's next actions name a closed pad
     whose net is left open. `pcb-maker pad-exits-kicad-board <dir> <id>
     [router.json|auto] [--repair] [--out <report.json>]` gives the census
     of a board as it is placed (and with `--repair` what the placer's
     repair would move). Its first line counts parts:
     `placed 37 of 38 parts (1 kept where they are; 14 board marks and
     rule areas kept clear of), 29 nets` (lpc1311-target-v1; until
     2026-10-26 "placed 38 of 52 components", the board's own texts and
     graphics counted in; a row's other parts count as placed, with
     their row, since 2026-11-04: trial 5's right_io said 17 kept where
     11 were fixed). A directory holding only the
     verdict of such an input error is taken again by the retry.
  2. Make a draft: `{"moves": 0, "placer": {...}}` in `layout.json` places
     and routes once, skipping the router-driven part moves. Those moves
     take most of a layout's time.
  3. Make the final run with the defaults. It makes up to 40 moves, for
     at most 600 s while connections are open and 60 s
     (`polish_seconds`) once everything is routed.
  - **The budget.** `total_seconds` in `layout.json` (default 1500) is
    the layout's whole budget in seconds of work (search expansions and
    placement units at rates the binary measures on this machine at its
    start, so a second of work is about a second of its wall clock; the
    run is the same on a loaded machine for the same rates): the placement
    race stops at 40 % of it, the moves at 65 %, the final route gets
    the rest. A small board needs far less: `"total_seconds": 30` lays
    out a 68-net keyboard in about 15 s. The wall clock guards it at
    `wall_guard` times (default 2; exit 3, `cut`);
    `PCB_ROUTER_NO_WALL_GUARD=1` leaves it to the work alone.
  - **The work rates.** The first line on stderr is `work rate: ...`: the
    rates applied and what the start-up calibration measured (about a
    second; `work_rate`, `work_rate_source`, `work_rate_measured` in the
    verdict). Results depend on the measured rates. To repeat a run
    exactly, here or on another machine, pin the rates it recorded:
    `PCB_WORK_RATE=<work_rate>,<placer_work_rate>` (the reference's:
    `3.15e6,150e6`, which the benchmark tiers use), or the router
    settings `work_rate` and `placer_work_rate`. `place-kicad-board`
    pins the reference's by default (a check should not depend on the
    load; `--measure-work-rate` measures as a layout does).
  - **The threads.** `RAYON_NUM_THREADS=<n>` caps the router's search
    threads and the placer's seed pool (default: every core), for a
    machine shared with other work; the result does not depend on it (the
    budgets count work), only the wall clock does.

### Pre-routed nets: keep what is routed

Route the traces that matter by hand (a differential pair, a crystal, a
switching loop; in KiCad or through its API), save the board, and route
the rest:

```sh
pcb-maker route-kicad-board <project-dir> <board-id> <out-dir> auto
```

- The board's own track segments, arcs and vias are **kept as they are**:
  the same items at the same coordinates, widths, layers and nets in the
  result. The router never owns them, so it never rips one up or reroutes
  it; other nets keep their clearance from them.
- **Their connectivity counts.** Pads a piece of kept copper joins (copper
  of the net overlapping on a layer both have, as KiCad's connectivity
  joins it) are one terminal of the net: that connection is made. A net
  they join whole has nothing left to route; a net they join in part is
  routed from the joined group to the rest. A piece on no pad of its net
  is a terminal of its own (KiCad wants it joined), unless the net has a
  pour, whose fill joins it (a stitching via).
- `board-router.json` reports `kept_copper`: `segments`, `arcs`, `vias`,
  `nets`, `connections_made`, `nets_complete` and `floating` (pieces on no
  pad); stdout has a line `kept: ...`. `routable_connections` and the
  nets' `terminals` count what was left to route.
- The router joins a kept group at one pad, so a route can be longer
  than a connection to the nearest point on the existing track. KiCad
  checks the entire result, including any copper that was already there.
  By default `--reference` uses the input to identify pre-existing findings.
- When an authored short trace already escapes a crowded surface pad,
  select its exact `REF:PAD` in `retained_pad_tips` in the router JSON.
  The trace must be a single retained straight segment, at most 2.5 mm,
  with a stable UUID, one endpoint at the pad centre and a free tip
  outside the pad on the same layer. The source pad must have its own
  copper component. The router uses that tip as its terminal so it can
  extend the existing trace. Keep the original project and named areas
  with the board so KiCad checks the actual clearances. Use
  `kept_copper.retained_pad_tips` in `board-router.json` for selected
  tips and ignored pads. Check the returned native findings and opens
  to see whether each extension worked.
- If KiCad reports several open pads on one dense row, look for
  `prepared_row_escape_advice` in `inspect-routing-run`. Merge its
  `router_config_patch` into an ordinary route config and rerun on the
  same copper-free placement. To reroute the routed board itself, pass
  `--strip`. The planner sizes all of the short planar launches together
  using the project's item-specific clearance and minimum track widths.
  It leaves spacing for neighboring pad exits. Use the same router
  configuration with `layout-kicad-schematic --router-config` to plan
  launches on a new placement. The imported and placed boards remain
  copper-free, and the selected launches are part of the routed result.
  The project rules stay in place. Review `kept_copper.retained_pad_tips`,
  native opens and DRC before adopting the returned project.
- With an authored copper zone and existing tracks, `route-kicad-board`
  can use a fresh KiCad check to recognize connections through plane fill.
  It returns the original PCB byte for byte when all these connections
  already pass. On an eligible partial board, it can route the open nets
  while protecting the plane nets KiCad already verified. A protected trial
  must reduce native opens and preserve existing copper and native DRC.
  `input_connectivity` in `verdict.json` reports the decision, native open
  counts as stored and after refill, and any rejected trial. For a trial
  that introduced a reported error, `trial_rejection` includes KiCad's
  description and up to two affected items. Use `rejected_drc` for the
  complete trial report, including item UUIDs and positions. When KiCad
  caps a footprint-library warning category, `source_library_sample`
  points to a second check of the original project with its footprints
  listed in reverse order. Each additional trial warning must match an
  original warning, including its item UUIDs and multiplicity. If only the
  refilled result is closed, use the supplied `retry_argv` to save and verify
  the fills in a separate output directory.
- `--strip` (router config `"existing_tracks": "strip"`) removes the
  board's tracks, arcs and vias first: the board is routed from nothing.
  Copper zones stay either way. `layout-kicad-board` always strips (it
  moves the parts).

Before 2026-10-21 route mode lowered the board's tracks and vias as
obstacles of their nets, routed every connection anew and then deleted
them from the result: other nets avoided copper that was not in the
board they got, and arcs were refused.

### Reuse verified routing after a revision

Pass a previous KiCad PCB to `--reuse-routes` when the revised board's
unchanged parts are still at the same coordinates.

```sh
pcb-maker route-kicad-board revised-project board-id revised-routing auto \
  --reuse-routes previous-layout/result/board-id.kicad_pcb
```

For a fresh layout after a schematic edit, add the same flag to
`layout-kicad-board`. The placer tries compatible previous positions and
can make room for newly added parts. It checks the current mechanical
brief and placement legality before trying the previous routing.
`board-layout.json` records how many footprints moved back to their
previous positions and how many new footprints moved to make room under
`route_reuse_placement`.

The placer and router identify renumbered footprints by their unique KiCad
schematic instance paths. A legacy KiCad timestamp matches its
zero-padded UUID form at any level of the path. The router then matches pad numbers and checks pose,
shape and layers. KiCad versions may write a pad's mask and paste layers
in a different order. The router checks layer membership, so reordering
the same entries preserves eligibility. Adding or removing a layer makes
that contact ineligible for full reuse. A unique unchanged reference can match an older unlinked
board when there is no conflicting instance path. Recycled references and
ambiguous links leave affected copper for ordinary routing. Update any
reference-based mechanical brief to name the parts meant by the revised
schematic. If a numbered pin has several
physical lands, all must still belong to one net and every contact must match.
An eligible net can be renamed or gain terminals. The router uses the current project's
schematic, layout and rules, and keeps any copper already on the current
board. Reused tracks and vias use the current routing dimensions.
An older protected-escape segment can qualify at its historical neck
width when its generated UUID matches its exact geometry, net and
originating pad. This compatibility check requires matching pad
contacts, the same board setup and native project rule files, and
the project's Default width settings. An explicit class or custom width
rule keeps its own dimensions. The independent native check still
decides whether the trial can be kept.
An exact replay can also keep the previous board's copper item order
and UUIDs when all donor tracks qualify and their IDs are unique.
Nets with active length, skew or coupled-pair routing use ordinary routing.
A net can keep the connected portion of a proven segment tree when one
terminal moves or leaves the net. For a short compatible move, the trial
replaces the leaf's first segment with a connection to its original
neighbor. A terminal that leaves the net has its old leaf branch removed.
Where a branch joins the middle of a straight trunk at an exact tee, the
connectivity check can split that line internally and retain the whole
original trunk segment on the board.
KiCad checks the trial against the current board.
Before trying prior copper, the router checks straight-sided rule areas
that forbid tracks or vias on the relevant layers. A confirmed overlap
withholds that prior net for rerouting. The detailed report and verdict
name it in `keepout_skipped_nets`. KiCad checks the full copper geometry,
including curved rule areas and overlaps the preliminary check cannot resolve.

In route mode the trial gets 10% of the modeled budget, up to 30 seconds. KiCad
checks the trial against the current input with all track findings enabled.
An accepted trial has no new DRC findings, opens or schematic parity issues.
If the trial fails, routing resumes from the current board with the
remaining budget. `board-router.json` records provenance, reuse eligibility,
the decision, the work charged, and the native comparison time under
`route_reuse`.

`shared_final_drc: true` means one fresh all-track KiCad report served both
final board verification and comparison against the current input. The
input gets its own check. Sharing requires matching board, schematic,
rules, library inputs and report contents. The first model-complete
attempt can share an all-track report with zero opens and parity findings
and only optional footprint library warnings. It can have pours if the
regular KiCad check can refill them without repairing the board first.
Findings that need ordinary verification keep their separate report. The
fresh raw report can serve the reuse comparison while the checked inputs
still match.

Read the reuse outcome in `verdict.json` for either command. [Reuse outcome fields](#route-reuse-in-the-verdict)

### Saved zone copper on layouts

When a selected layout has fillable copper zones and enough work remains,
KiCad fills a private copy, then checks the returned PCB with its fill as
stored. The returned PCB keeps its original items, zone outlines and rules.
KiCad may round outline coordinates to its integer nanometre grid when it
saves the private copy. The handoff checks the vertices on that grid and
keeps the original outline coordinates in the returned file. It expands
copper layer aliases against each board's stack and checks that each
zone still covers the same layers.
`fill_publication` in the layout report and verdict records the outcome,
both PCB hashes, filled zone and polygon counts, and the native check cost.
The stored-board report is `result/drc-saved-fill.json`.
When the routing budget expires before the save and reopen checks,
`fill_publication.retry_argv` gives the standalone command to create a
verified filled project in a sibling of `result/`. The CLI puts this
handoff first in `next_actions` when the layout otherwise completes.
When a layout with copper pours already connects its modeled terminals,
optional footprint moves and polishing reserve work for the final native
checks and saved fill. A layout with open connections can use that work
to seek a route.

`route-kicad-board` also saves and verifies fills on newly routed projects
that complete native verification and have fillable zones. The two checks
use the routing budget that remains after search. The expected refill,
staged refill and reopened PCB must reproduce the same native findings.
The route verdict records `fill_publication` and links `drc-saved-fill.json`
when the handoff succeeds. A budget deferral leaves the route's completion
scoped to the refilled check and gives a `retry_argv` for a filled sibling
project. Boards without fillable zones incur no handoff checks. Existing
copper kept byte-for-byte still uses its separate `input_connectivity`
report and fill command.

On KiCad 10.0, a capped `lib_footprint_issues` warning set can accompany
published fill. A capped `silk_overlap` warning set can also accompany it
when every physical zone is on copper only. Publication still compares
every reported finding by identity and records the capped types under
`fill_publication.saved_as_stored.possibly_capped_types`. Other caps
block publication, including caps on opens, parity or electrical
findings. [KiCad silkscreen check](https://github.com/KiCad/kicad-source-mirror/blob/10.0/pcbnew/drc/drc_test_provider_silk_clearance.cpp) · [KiCad footprint library check](https://github.com/KiCad/kicad-source-mirror/blob/10.0/pcbnew/drc/drc_test_provider_library_parity.cpp)

Set `"save_zone_fills": false` in the layout config for a faster routing
iteration. The normal native check still refills in memory. The verdict
records that the stored fill was skipped. To save and check the fill later,
use `drc-kicad-board --save-filled` with a new output directory.

Near the work limit, the ordinary final layout check can save its refill
in scratch when KiCad's isolated settings are established. The receipt
binds the selected board, project inputs, native tool and exact DRC
report. Publication still reopens the stored board with an all-track
native check and compares the complete findings. A reused receipt is
reported as `fill_publication.refill_source: "fresh_layout_drc"` and
charges one additional check.

When an incremental layout keeps a route that already closes the board,
the fresh all-track reuse check can capture the fill on a private copy.
Publication then uses that fill and runs a fresh as-stored check.
`refill_source: "fresh_route_reuse_drc"` and `native_checks: 1` identify
this case. The project, board and native inputs must still match the
checked snapshot.

Equivalent inputs can reuse verified stored fills from the local
`verified-fills/` cache. The key includes the selected PCB and project
design files, library paths, and the KiCad executables, settings and
check policy. Caching requires a flat schematic and contained KiCad
libraries. Unsupported projects still run the fresh checks. A hit
reports `cache_status: "hit"` and `native_checks: 0`. It retains two
`native_checks_charged` so a hit and a miss use the same modeled budget.
`PCB_MAKER_NO_CACHE=1` runs a fresh check without using or updating the
cache. Choose a new `PCB_MAKER_CACHE_DIR` to start a fresh cache.

## 4. Read the verdict: exit code and `<out-dir>/verdict.json`

Every run of `route-kicad-board`, `layout-kicad-board`,
`place-kicad-board` and `fab-kicad-board` ends with an exit code and a
verdict that say the same. `pcb-maker --help` and `pcb-maker <command> --help` list them too.

| Exit | `verdict.status` | Meaning | What to do |
| --- | --- | --- | --- |
| 0 | `complete` | The run finished and the result passes. | Use it. |
| 1 | `incomplete` | The run finished and wrote its result, but connections are open, there are new DRC errors, constraints are missed or parts found no place. | Read `next_actions`. |
| 1 | `router_complete_unverified` | KiCad's check was skipped (`PCB_SKIP_NATIVE`, `skip_native_verification`; `kicad_checked: false`) and the router's own count finds nothing open. Not a pass: KiCad's connectivity and DRC decide, and they did not run. | Check the board (`drc-kicad-board`) before using it. |
| 2 | `usage_error` | Bad arguments or input: a missing file, invalid JSON, an unknown key or part, an output directory that exists. Nothing was routed. | Fix the input the message names. |
| 3 | `cut` | Incomplete, and the wall clock (not the work) cut the run short (`cut`): the result depends on the machine's load. | Rerun on a less loaded machine or with a larger budget. |
| 4 | `internal_error` | An internal error or a panic; also a KiCad check that ran but left no readable report (`drc.json`) for the verdict to rest on. | Report it with the input; where kicad-cli died of a signal (KiCad 10.0.7's refill crashed on trial 8's OpenAIO-Whoop, SIGSEGV, and did not repeat) the next action says it is KiCad's: run again into a new output directory. |

A process killed by a signal (a harness's timeout, out of memory) has no
verdict; its exit status is the signal's (128 + n in a shell). Its output
directory holds no report either: route mode routes into
`<output>.partial` and renames it to the output when it is done, the
report written last, and the verdict and the report are each written
whole (a temporary file renamed into place), so a killed run's best
attempt so far cannot pass for a result. Errors and incomplete results
print one line on stderr.

**Memory: the machine's ledger** (since 2026-10-22). `route-kicad-board`,
`layout-kicad-board`, `place-kicad-board` and `drc-kicad-board` reserve
their memory in one ledger file per machine (`PCB_MAKER_LEDGER`, default
`$XDG_RUNTIME_DIR/pcb-maker-ledger.json`, else
`/tmp/pcb-maker-ledger.json`; `off`: none) before they allocate: what the
process holds and its routers' estimated peak (their lattice maps and
600 MB, with a 1.25 margin; 1.2 times and 540 until 2026-10-23, 1.7 and
460 until 2026-10-22), 2.2 GB more while kicad-cli runs. Every kicad-cli
that loads a board runs inside a reservation (since 2026-10-27):
`drc-kicad-board`, `place-kicad-board` and `inspect-routing-run
--native-check` reserve KiCad's 2.2 GB up front, every other command
that runs kicad-cli (`export-kicad-fab`, `fab-kicad-board`,
`import-kicad-netlist`) reserves it at its first check, and `pcb-maker
ledger` marks a reservation's check `[KiCad's check: 2253 MB, reserved]`
or `running`. A run waits while the reservations of every run on the
machine and its request exceed the ledger's capacity: the machine's
memory less what runs outside the ledger (the median of the headroom
samples run.py records beside the ledger; without an hour of them 25 %
of the machine's memory), at most the machine's memory less 6 GB
(`PCB_MAKER_LEDGER_MARGIN_MB`); `pcb-maker ledger` says what it is made
of. A new request also keeps the harnesses' floor of available memory
beside it (the memory available less what the other runs still grow by,
less the request: 2 GB for a short command, the measured floor, about
9 GB, else 10 GB, for a route or a layout), unless no other run holds
memory. It says so on stderr (`memory ledger:
waiting to reserve ...`); routers that would run side by side run fewer
at a time instead, which does not change the result. Waiting is in
order (since 2026-10-24): a run that holds nothing yet does not pass the
request that has waited longest, only takes what still leaves it its room
(`behind an older request` on stderr; docs/benchmarks.md, "Memory ledger:
who runs next"), and once that request has waited 10 minutes, only what
leaves it the room it waits for (`the ledger drains for it`), as long as
the ledger could start it (otherwise `not drained for: ...`, and the
ledger drains for the next waiter it could start); after 2 hours of its
wait strictly (since 2026-10-28). A request larger than the ledger's
capacity runs only alone; one that has waited 2 hours for the ledger to
empty ends with exit 4 and a verdict saying so. A short command never waits behind that order
or a drain (since 2026-10-26): `drc-kicad-board`, `inspect-routing-run
--native-check` and `place-kicad-board` (which reserves only for KiCad's
check) hold their memory for seconds, and a request of theirs of at most
2 GB beside one KiCad check goes as soon as the machine's memory is there
(the ledger's capacity has its room and the memory available, less what
the other runs still grow by, covers it); `pcb-maker ledger` marks it
`[short]`. A route and a layout hold theirs for minutes and wait in
order. `PCB_MAKER_LEDGER_SHORT=1` makes a command's requests short, `0`
makes them wait in order. The reservation is
also a limit of the process's own: a run that grows past it is given more
while the ledger has room; otherwise (since 2026-10-27) it runs on and
waits in the ledger for the room (`memory ledger: the process grew to
... it runs on and waits for the room` on stderr; its request counts as
old as its admission, so newer requests leave it its room), and ends
with exit 4 and a verdict whose error names the reservation (rerun when
fewer runs share the machine), rather than being killed by the kernel,
only when the machine's memory available falls below 2 GB, or after
600 s without room when the memory available less what the other runs
still grow by does. A harness that reserved for the process and handed
it the reservation (run.py) sets the least of the process's own
(`granted_mb`). A run's reservation
goes with the process, also when it panics or is killed (the ledger drops
the reservations of dead pids). `pcb-maker ledger` (`--json`) lists the
live reservations: pid, megabytes, what a waiting run waits for, age and
the run's command, board and output directory.

**Boards the reader refuses** (exit 2, `usage_error`, one line naming
the place; since 2026-10-21). A board, footprint library, schematic or
netlist that does not parse is the input's error, as are: lists nested
more than 1000 levels deep (KiCad's own files nest a dozen); a
coordinate, size or angle that is not a finite number (`NaN`, `inf`,
`1e999`, which Rust reads as numbers), named with its item (`a gr_line's
start coordinate 1 is inf`, `the footprint J3's at coordinate 1 is
18e999`; a word such as `inf` where a name is read, a KiCad 5 net or a
tstamp, stays a name); a board outline (Edge.Cuts) that spans more than 2000 mm (a stray
Edge.Cuts item far from the board: the message gives the extent and
the corners); a lattice the memory cannot hold (the outline's extent at
the router's coarsest pitch, a floor on its maps, against
`PCB_MAKER_MEMORY_MB` or the router setting `memory_mb`, the cgroup,
the address space limit or the machine's available memory: the verdict's
first next action names the extent, the pitch and the nodes; fix the
outline and run again into the same output directory); a footprint form
without its name (`(footprint)`); a board id that is no file stem (`..`,
a path separator, a leading `~` or `$`); and a footprint taken from a
`--footprints-from` board whose library nickname or name is no file
name (`../../x:../../y`, which would be written outside the output
directory). A library footprint file that holds no footprint is reported
missing (exit 1), and a netlist's footprint name that climbs out of its
library (`lib:../../x`) is not found. A custom rule condition nested more
than 200 levels deep does not parse and is reported unmodelled, as other
conditions that do not parse are. What the reader leaves out without an
error is listed in `board-router.json`'s `warnings` and leads the
verdict's next actions: copper (segments, arcs, vias, zones) on a copper
layer the board's stack lacks (`In3.Cu` on a two-layer board; the router
ignores it there and the written board moves it to Cmts.User) and text
boxes of fewer than three corners.

**The pass rule**: KiCad's own DRC on the result decides, not the router's
count. A result is complete when KiCad reports no unconnected item and no
error that the reference board does not have at the same place
(`new_findings_strict` is empty). A finding is the reference's when it has
the same type and either the same items (every item's uuid, the same set)
still the same distances apart (a part's own finding, such as two holes of
one footprint too close, goes with the part wherever it is placed), or
items at the same positions within 0.05 mm (a fixed part's, an outline's);
each reference finding answers for one finding of the result. A finding
of one item has no distances to keep: it is the reference's only where
its item lies within 0.05 mm of the reference's, and never where KiCad
gives the item no place (a zone it reports at (0, 0), as for a
`connection_width` on a pour, unless the finding has a position of its
own): the same uuid alone no longer matches (since 2026-10-22; until then
a narrow neck of ours on a pour matched any of the designer's on that
pour). A finding between items of one footprint only (its pads and
graphics, whose geometry and nets no placement or route changes: a
mounting hole's ring of overlapping pads) is the reference's. **Without
a reference** (a layout from a schematic: there is no designer's board)
such a finding is the footprint's own and does not count either
(`footprint_findings`), except a short (`shorting_items`: the
schematic's nets on its pads, which stays judged): a library footprint
whose pads lie closer to its own holes than the project's rule (the
schematic trial's USB4105 receptacle: GND pads 0.19 mm from its locating
holes against a 0.25 mm hole clearance) failed every layout and refused
the fab release. `next_actions` names them and the exact custom rule
that makes KiCad's own DRC accept them, for the board's `.kicad_dru`, at
the smallest value KiCad measured rounded down to 0.01 mm: `(rule "J1 own
hole_clearance" (constraint hole_clearance (min 0.19mm)) (condition
"A.memberOfFootprint('J1') && B.memberOfFootprint('J1')"))` (a new file
starts with `(version 1)`), or a KiCad exclusion for a type no custom
rule sets. `fab-kicad-board` releases with them (`footprint_own` and
`footprint_note` in fab-manifest.json, the note last in the verdict's
next_actions, tagged, since 2026-11-04; first before), and `drc-kicad-board` prints them on its last `next:` lines
and does not fail on them. Since 2026-10-27 a finding on one pad alone
that is the pad's own geometry against a board-wide minimum counts as its
footprint's too (`pad_own`): its annular ring (`annular_width`), its hole
(`drill_out_of_range`) or its padstack (`padstack`: a hole that leaves no
copper, an SMD pad with copper on both sides); so does, since
2026-11-04, a footprint's courtyard that is no closed shape
(`malformed_courtyard` on "Footprint U5": trial 6's ESP32-S2-MINI-1, on
the designer's board too, failed the layout as a new error). The nightly of 2026-10-27
laid out ten boards complete and clean but for such findings (the RP2040's
thermal-via pads with a 0.10 mm ring under 0xCB-1337's 0.13 mm minimum,
the 7805's 0.0875 mm on the mackerel boards, hackrf's fiducials), each on
the designer's board too, and every one exited 1 without a reference.
KiCad checks a pad's ring and hole with no second item, so the rule for
them names `A` alone: `(rule "U3 own annular_width" (constraint
annular_width (min 0.1mm)) (condition "A.memberOfFootprint('U3')"))` (a
condition on `B` never holds there); a hole over a maximum, and a
padstack, get an exclusion. So does a finding KiCad reports against the
board minimum (`(board minimum hole clearance 0.2500 mm; ...)`, since
2026-10-27): KiCad says so where the rule that holds (a footprint's own
clearance) is below the board setup's minimum, and applies the minimum,
which no custom rule goes below (framework_mobo_lefthalf's J1, kicad-cli
10.0.7: a rule at 0.1 mm, also with `(severity ignore)`, left its 8
findings; `min_hole_clearance` at 0.1 mm took them away, for the whole
board). Until 2026-10-27 such findings were offered a rule that did not
take. A via's finding names no footprint and stays
judged.

**Pads walled by their own footprint** (since 2026-11-11;
`walled_pads`). Some footprints draw copper over their own pads: an
inverted-F antenna's polygon over its feed pad, a microphone's ring over
its ground pads. Unless the footprint is a net tie, KiCad keeps such a
graphic copper of no net: it joins nothing, and any track or via that
touches it is a finding (`shorting_items`, or `clearance` at 0 mm), so a
pad it walls in cannot be connected without one. The router wires each
such pad through a corridor it cuts in the graphic (`walled_pad_exits`,
on; `false` leaves them unwired, reported as dead pads), and the verdict
sets apart exactly the findings between that graphic (by uuid) and the
written tracks and vias of the pad's net in that corridor; any other
contact with the graphic (another net's track, a track of the pad's net
away from the corridor) stays the run's. stdout says, before the
verdict line, one line per pad: `walled pad: AE1.1 [ANT] wired through
AE1's own fp_poly on F.Cu: KiCad shows shorting_items 2
(verdict.walled_pads)`, and `next_actions` one line per footprint with the
two ways on: accept the findings as forced by the footprint (KiCad's own
DRC keeps reporting them; exclude them in its DRC dialog), or rework the
footprint. KiCad 10.0.7 accepts the connection cleanly (kicad-cli, routed
boards) when the footprint is a net tie of the pads its copper touches,
`Net tie pad groups "1,2"` in the footprint's properties
(`(net_tie_pad_groups "1,2")` in the file; also for pads of one net, a
microphone's ground pads), or, where the copper touches one pad only,
when that copper is part of the pad: a custom pad (`pad ... custom`)
whose `primitives` hold the graphic. A net tie group of one pad does not
help. The line names the one that fits: `walled pads of AE1: AE1.1 [ANT]
(KiCad shows shorting_items 2), AE1.2 [GND] (KiCad shows shorting_items
2), wired through AE1's own fp_poly on F.Cu: ... rework the footprint:
mark AE1 a net tie of the pads its copper touches: Net tie pad groups
"1,2" in the footprint's properties ((net_tie_pad_groups "1,2") in the
board file), ...`.

**The input's own findings** (since 2026-10-27, without a reference
only; `input_findings`). A finding is the input's, and does not count,
when it is of a type that is not electrical between two nets and every
item of it is the input's: a pad, graphic or courtyard of a part the run
kept where the input has it (its position, rotation and side unchanged:
the fixed parts, and in route mode every part), the board edge, the
input's own board graphics (`gr_*` on any layer: Edge.Cuts, Margin, a
copper ring) or the input's zones, unchanged but for their fill. One item
the run wrote (a track, a via, a pour it added, a part it placed, an
outline it changed) and the finding is the run's. The verdict tells them
by uuid: before it judges, it compares the input board with the result
(`input_items`). The types it may excuse (`INPUT_KINDS`):
`copper_edge_clearance` (a connector's pads over the outline, a card's
GND ring at its edge), `solder_mask_bridge` (a mask opening over copper
of another net: a fiducial's over the designer's zone, two stacked switch
footprints), `courtyards_overlap`, `pth_inside_courtyard`,
`npth_inside_courtyard` and `malformed_courtyard`; with the footprint's
own (`annular_width`, `drill_out_of_range`, `padstack` above) and the
cosmetic types (silkscreen) that do not count anyway. Never excused this
way, even with both items the input's: `shorting_items`, `clearance`,
`hole_clearance`, `hole_to_hole`, `items_not_allowed`,
`connection_width`, `track_width`, unconnected items and every other
type: two fixed parts of two nets too close still fail the run and
refuse the release (framework_mobo_lefthalf's R31 and R32 at J1). Their
next action says that the parts are kept where the input has them, and to
move one in the input (or free it from `fixed`) or accept the finding
knowingly, not "move the parts apart" (since 2026-10-27). Where only some
of a type's findings lie between such parts, those are said to be the
input's and the others the run's (`1 of them lie between parts kept where
the input has them (HC1: fixed) ...; the other 3: move those nets' parts
apart`, Batch_1's shorts; since 2026-11-04 they all read "move those
nets' parts apart"), and route mode, which keeps every part, says that
none is the route's to move: a finding with a track or via of the route
is the router's, one between the input's parts alone the input's. The
nightly of 2026-10-27 (109 layouts without a reference): 113
`copper_edge_clearance` findings, 96 of them fixed parts' pads, 16
Batch_1's own GND ring of `gr_line`/`gr_arc`, none a track, via or zone
of ours; 0xcb-static's 2 bridges (switch footprints stacked by the
designer) and Explorer's 3 (fiducials over the input's zones) made boards
with nothing open exit 1. The two the sweep's reference rows found ours
stay ours: osprey's antenna at the edge (a part the layout placed) and
vaio_re's 5 bridges (a moved part's pads). `next_actions` names them and
how KiCad's own DRC accepts them, for the board's `.kicad_dru` (checked
with kicad-cli 10.0.7 on the sweep's boards): an edge clearance of
footprints, `(rule "input edge_clearance" (constraint edge_clearance (min
0mm)) (severity ignore) (condition "A.memberOfFootprint('J2') ||
A.memberOfFootprint('M1')"))`; of the board's own copper graphics, by
their net, `(rule "input GND graphics edge_clearance" (constraint
edge_clearance (min 0mm)) (severity ignore) (condition "A.Type ==
'Graphic' && A.NetName == 'GND' && !A.memberOfFootprint('*')"))`;
courtyards, by their pairs (either order), `(rule "input courtyards"
(constraint courtyard_clearance (min 0mm)) (severity ignore) (condition
"(A.Reference == 'H4' && B.Reference == 'J2')"))`. A copper finding
crossing the outline is reported whatever the minimum, so the rule needs
`(severity ignore)` (`(min 0mm)` alone left J2's two on
framework_mobo_lefthalf).
No custom rule changes a solder mask bridge (`bridged_mask`,
`solder_mask_expansion` and `clearance` with `(severity ignore)` left
Explorer's 3 as they were): those, and a reference no rule can name (a
blank one, a `*`), get an exclusion in KiCad's DRC dialog. With a
reference nothing changes: the reference decides by place (run.py's
strict rule too). **`drc-kicad-board` and `fab-kicad-board`** judge a
board on its own: every part is kept and every zone and board graphic is
the board's, tracks and vias never are. On a layout's result that also
excuses a mask bridge or an edge clearance of a part the layout placed,
or of a pour it added (vaio_re's 5 bridges and osprey's antenna), which
the layout's verdict counts; `--input <project-dir>` (the project the
board was laid out or routed from) gives both the layout's rule. Neither
excuses a short or a clearance between two nets: the release gate still
refuses a board on the designer's own shorts. A
`copper_edge_clearance` is the reference's where its copper item is
(KiCad names whichever piece of the outline is nearest beside it). A type the
reference has 199 times or more (KiCad's report stops at a cap: 199 and
499 are the counts seen, so such a count is a floor) is listed in
`preexisting_unknown`: of it only the findings the reference has at the
same place are subtracted. Warnings, excluded findings and cosmetic
types (`silk_*`, `text_*`, library findings, `missing_courtyard`,
`isolated_copper`) do not count. A layout or placement must also place
every part and keep every constraint. Without KiCad's check
(`PCB_SKIP_NATIVE`, `skip_native_verification`) a route is never
`complete`: `router_complete_unverified` at best (exit 1). The router's
count is built to agree with KiCad's connectivity (docs/router.md,
"KiCad's connectivity": pads of one number on two sides are two
terminals, nets named by numbers and net codes are resolved, copper
graphics with a net are terminals, the pour model connects only what
KiCad's refilled fill connects, and the emitted tracks are checked as
KiCad joins them): on 65 intake boards that the router used to call
complete while KiCad found unconnected items it agrees with KiCad on 63
(2026-10-21). It still checks no other DRC rule, and the pour model is a
model: run KiCad's check before using the board. A check that ran
but whose report is missing or unreadable when the verdict reads it is an
internal error (exit 4), never a verdict on the router's count. The old
rule (errors beyond the reference's count of their type anywhere on the
board: a new error of a type the reference has was hidden) is kept as
`new_findings` for comparisons; the benchmarks report both
(`benchmarks/agent-tasks/run.py`: `pass` and `pass_strict`).

**The reference board** is where pre-existing findings come from (a
fiducial near a hole, a rule the designer tightened later): `--reference
<project-dir>` names it. Route mode defaults to the input project (its
board as given); layout mode to none, since its input is not a placed
board. `--reference none` counts every error. The reference is checked
(one more KiCad DRC, kept as `reference-drc.json`) only when the result has
errors that count. The reference's findings do not count in the run's
verdict, but `fab-kicad-board` judges the board on its own: where some of
them are between two nets (a clearance, a hole clearance, a short; not
within one footprint), a next action says that the release refuses them
(since 2026-10-27; framework_mobo_lefthalf with `--reference` said only
"2 unconnected", and the release then refused its fixed parts' 2
clearance and 4 hole clearance findings). A reference with its tracks stripped (a board as handed
to the router) says what the fixed geometry brings on its own; the
benchmarks' strict rule uses the designer's board stripped so
(`strip-kicad-tracks`: tracks, vias and small zones), and since its rule 2
(run.py, 2026-10-21) also leaves out a solder mask bridge between a track
and a designer's mask opening that bridges on that board or on the
designer's own (benchmarks/agent-tasks/README.md, "How to measure"); the
verdict here does not.

`<out-dir>/verdict.json` is written for every run that got as far as an
output directory, also for an error (with `error`) or when the layout
stopped because parts found no place (then there is no `board-layout.json`).
A finished run also carries it as `verdict` in its report:

| Field | Meaning |
| --- | --- |
| `status`, `exit_code`, `reason` | As above; `reason` is one line. |
| `route_reuse` | The reuse decision and retained copper counts after requesting `--reuse-routes`. [Reuse outcome fields](#route-reuse-in-the-verdict) |
| `kicad_checked` | Whether KiCad's DRC checked the result (`false` with `PCB_SKIP_NATIVE` or `skip_native_verification`, and for a placement or an error). |
| `open_connections`, `unconnected_terminals` | The router's own count of unfinished connections and unreached pads. They describe the board written: a net an accepted open repair after the router (route mode's `open_repair`, a layout's `local_open_repair`) closed by KiCad's count is routed (`nets[].open_repair` in board-router.json and `routed.nets[]`). |
| `open_repair` | Route mode, where KiCad found connections open after the ladder: the open repair's `status` (`accepted`: copper was added after the router; `no_improvement`, `skipped`, `failed` with `reason`), `nets` (the nets that got the copper), `added_copper` (objects), `opens_before` and `opens_after` (KiCad's unconnected items), `native_checks`, `search_work_seconds`. Also in `board-router.json`. A layout has `local_open_repair` instead. |
| `saved_copper` | The final PCB's `track_length_mm`, `segments`, `arcs` and `vias`, including retained copper, pair routing and accepted repairs. Length sums the centrelines of straight and circular track objects. Also in `board-router.json` and `board-layout.json.routed`. The router's legacy `length_mm` and `vias` still sum its per-net route-emission records. |
| `kicad_unconnected_items` | KiCad's count, which decides (`null`: KiCad did not check). On a panel of different boards the items between two sub-boards are not in it (`sub_boards`). |
| `new_findings_strict` | KiCad errors per type that the reference does not have at the same place: the run's own. They decide. |
| `new_findings` | The old rule: errors per type beyond the reference's count of that type anywhere on the board. For comparisons only. |
| `preexisting_findings`, `preexisting_from` | Errors per type the reference board has at the same place, and its project. |
| `footprint_findings` | Without a reference: errors per type that lie within one footprint each (no short), or are one pad's own ring, hole or padstack, which do not count; absent when there are none. For a different-net pad clearance finding, `next_actions` gives the pads and nets, measured and required gap, and shortfall in millimetres where the native evidence resolves them. It names the footprint library ID and its resolved project-library copy when available. Check the source land pattern and fabrication requirements before updating the PCB. For a plated through hole with zero annular width, it names the footprint and location and asks you to confirm the intended plating. |
| `footprint_clearances` | Up to four of the largest resolved different-net copper-clearance shortfalls within a footprint. Each entry has the `reference`, two `pads` with `number`, `net` and any native item `uuid`, and numeric `measured_mm`, `required_mm` and `shortfall_mm`. When available, `library_id` identifies the source footprint and `resolved_library_copy` locates the copy resolved through the current project. The field is omitted when no pair resolves. See `drc.json` for all native items. |
| `held_pad_shorts` | KiCad different-net pad shorts between two held footprints, with both references and footprint UUIDs, the native finding count, and up to three matching pad and net samples. Correct the footprint geometry or revise a held pose. For placement-only runs, a held body warning calls for review and names modeled copper overlap when the geometry supports it. |
| `invalid_padstacks` | Up to eight blocking KiCad custom-pad geometry findings resolved to a unique pad on the saved PCB. Each gives its `reference`, `pad`, `net`, reported `layer`, item `uuid`, source `footprint_id` and native `reason`. The errors remain in the strict count. Repair the source footprint and verify the resulting board in KiCad. |
| `input_findings` | Without a reference: errors per type that are the input's (edge clearance, solder mask bridges, courtyards; every item a part kept where the input has it, the board edge, the input's board graphics or zones; never a short or a clearance), which do not count; absent when there are none. next_actions has the custom rule or the exclusion that accepts them. |
| `walled_pads` | Route and layout (since 2026-11-11): the pads wired through their own footprint's copper ("Pads walled by their own footprint" above), each with `part`, `pad`, `net`, `layer`, `graphics` (`uuid`, `kind`), `touching` (the footprint's pads that copper touches), `crossings` (the uuids of the written tracks and vias in its corridors) and `findings` (KiCad's, `type: description`, set apart: they do not count); absent where there are none. |
| `retry_guidance` | A route whose only blockers are native DRC errors between pads or holes of footprints kept at their input poses can report `kind: "fixed_input_pads"` and `same_placement_and_rules_can_resolve: false`. It names the footprints, counts all blocking errors by KiCad type, and includes up to four examples with native item descriptions and reported anchors. Correct the input pad or hole geometry before rerouting, or move a footprint where the finding spans two footprints. The geometry action precedes parity advisories. This field is omitted when other blockers exist or the evidence is incomplete. |
| `preexisting_unknown` | Types the reference has at KiCad's cap (199 or more): only the findings at the same place are subtracted. |
| `clean_without_subtraction` | KiCad checked the result and finds no unconnected item and no error that counts, with nothing subtracted: a board with walled pads (`walled_pads`) is complete but not clean in this sense, since KiCad reports their crossings. |
| `diff_pairs` | Route mode, and a layout whose router config sets `"diff_pairs": true` (absent in a layout's verdict otherwise; docs/router.md "Differential pairs"): `on`, `default` (`rules`: a custom `diff_pair_gap` or `diff_pair_uncoupled` rule holds for a paired net, so the pairs routed coupled by default; `off`: nothing asks, so they did not; `explicit`: the settings' `diff_pairs` or `PCB_ROUTER_DIFF_PAIRS` decided), `reason` (the rule and the net, or the setting); off, the counts are 0. With the pairs on: `found` (pairs KiCad names by `_P`/`_N`, `+`/`-` or `P`/`N`, both nets with pads), `coupled` (every link between the pair's parts a coupled run), `partly_coupled`, `fallen_back` (routed as two ordinary nets), `links`, `links_coupled`, `crossed` (coupled links whose pads lie in the other order at one end: the stubs cross there, uncoupled), `crossovers` (of those, links whose crossed end has a crossover in its lead: one net dives under the other with a via each side, so the stubs do not cross; since 2026-11-05 on, where the link carries the stubs to it), `coupled_mm`, `uncoupled_mm` (per coupled pair the longer net's tracks less its coupled run, KiCad's measure), `skew_mm` (the largest skew of a pair with a coupled link: its two nets' lengths apart in the written board, tracks and vias at the stackup's height), `meanders` (pairs whose shorter net got a meander, written as a KiCad tuning pattern, to match the custom rules' `skew` maximum, else the pair's gap, or a `length` minimum), `skews` (per pair with a coupled link: `positive`, `negative`, `lengths_mm`, `uncoupled_mm` (since 2026-11-05: the longer net's length less the coupled run, KiCad's uncoupled length, per pair), `skew_mm`, `target_mm`, `meander_mm`, `pair_vias`, `return_vias`), `not_routed` (the links whose coupled run was not routed, by cause: `a launch has no legal node`, `no path in the negotiation`, `the hard repair found no room`, `no hard repair: the attempt was judged hopeless`, `the hard repair's budget ran out before it`, `lost after the repair`, and with `diff_pair_give_up` `given up in the negotiation: no room` or `...: no path`; absent when none), `given_up` (the links the negotiation gave up, `diff_pair_give_up`: their pairs' nets routed uncoupled in the same attempt), `stubs_routed` (since 2026-11-05: stubs from a pad to its end of a coupled run that the attempt left out, routed after it with everything else kept, so that the link is coupled instead of falling back for "a pad's stub does not reach the run"; absent when none), `pair_vias` and `return_vias` (the kept links' vias, and the vias of the reference net placed beside them, each within 0.5 mm ring to ring where a pour of that net reaches it on two layers and every clearance holds; `diff_pair_return_vias: false` turns them off), `return_net` (their net: `diff_pair_return_net`, else `GND`), `work_seconds` (the fallback router's, which routes what the coupled runs left open, charged to the ladder). `board-router.json` also lists each pair: its nets, width, gap, where they came from, why links fell back. |
| `lengths` | Where the project's custom rules have `length` or `skew` constraints (docs/router.md "Length and skew rules"; absent elsewhere, and with `length_matching` off): per rule and net `net`, `rule`, `constraint` (`length` or `skew`), `length_mm` (as KiCad 10 counts it for the rule: the tracks on the layers the rule holds on, a track's way inside a pad as the straight line from the pad's position, vias at the stackup's height), `target_mm` (what the meander aimed at: a length rule's opt, else the middle of its bounds; a skew rule's longest net less half the skew), `min_mm` and `max_mm` (the bounds KiCad holds the net to; a skew rule's: its longest net less the skew, and the longest), `meander_mm` (what the meanders added), `within` (KiCad then has no finding), `note` (why not: not routed, no straight track with room for all of it, longer than the maximum, a coupled pair's net). A net whose copper lies only off a rule's layers (a rule with a layer condition: Sisu_SSE-9's USB rule on In1.Cu and In4.Cu) has no length the rule counts and is not checked by it, nor of a skew rule's group: `within` true, `note` `routed off the rule's layers (...)` (since 2026-11-06, trial 7; before, "not routed", and the action called such nets open); a net of one pad has nothing to route. A net the route takes onto the rule's layers joins its skew group (Sisu's route mode: six USB nets on In4.Cu matched against a 43 mm pair). Since 2026-11-06 also `layers` (the copper layers its tracks went on), `in_group` (KiCad counts copper of it for the rule: of the skew group, checked by a length rule) and `preferred` (the layers the router asked it to keep to and why: `In1.Cu, In4.Cu (on the rule's layers, by the designer's board)` or `F.Cu, B.Cu (off the rule's layers, by the designer's board)`): with `length_rule_layers` (default `"designer"`) each net of a rule on some layers only prefers the side of those layers the designer's board (the `--reference`, else the input's own tracks, `--strip` or not) runs it on, a step elsewhere costing ten times as much (`layer_preference_cost`), and a coupled pair whose two nets prefer the same layers couples on them; nothing without the designer's tracks (a layout from a netlist import). `"rule"` puts every net of such a rule on its layers, `"off"` none (docs/router.md, "Skew groups' layers"). |
| `narrowed_classes` | Route and layout, where the narrow-signal step (the ladder's last resort: the signal nets of the Default class at the project's narrowest predefined width when connections stay open at the class widths) routed nets of the board written below their class's track width (since 2026-11-06, trial 7; absent otherwise): per class `class`, `class_width_mm`, `routed_width_mm` and its `nets`. Since 2026-11-06 the step narrows only the Default class (a net no other class gives a width, at Default's width): every class the designer named keeps its width, so the list holds `Default` alone. Until then it narrowed every class wider than the menu's width, and trial 7's Sisu_SSE-9 layout wrote its RF_ANT nets (0.1422 mm, the designer's 50 ohm microstrip), MODEM_ANT1 (RF_ANT_INT, 0.1102 mm stripline) and Power_VBATT_3V8's 0.5 mm charger nets at 0.1 mm with no finding (KiCad's DRC checks no class width); no board of the 109 tasks completed only by narrowing a named class (docs/router.md, "The narrow-signal step's classes (2026-11-06)"). Should a class other than Default appear all the same (a guard: the step's rule and the report attribute a net to its class alike), a next action lists it and gives the levers: a custom rule giving the class's width as `opt` (`(rule "RF_ANT width" (condition "A.hasNetclass('RF_ANT')") (constraint track_width (opt 0.1422mm)))`; the step keeps a net whose width a rule gives on every layer), or `"use_narrow_signals": false` in the router config. Also in board-router.json's `rules.narrowed`. |
| `class_layers` | Route and layout (since 2026-11-06, trial 7): per net class with tracks on the board written, `class`, `track_width_mm` (the class's own), `nets` and per copper layer its `layer`, `length_mm`, `nets` and `rule_width_mm` (where a custom `track_width` rule gives the class a width on that layer, as for one impedance on In1.Cu and In4.Cu), and `designer_layers` (the layers the designer's board runs the class on: the `--reference`'s, else the input's where it has tracks). KiCad ties no class to a layer unless a custom rule does, and its DRC checks no class width, so a class whose width is an impedance on one layer passes every check on another: trial 7's Sisu_SSE-9 route wrote MODEM_ANT1 (RF_ANT_INT, 0.1102 mm, the designer's stripline on In1.Cu) 55 mm on B.Cu. A `warning` (and one next action for all of them) says where a class went on a layer its width is not for: a class with widths per layer from the rules on a layer they give no width on (and the designer's board does not run it on); a class narrower than Default without layer rules (likely an impedance; a wider one is likely a current to carry) on a layer the designer's board does not run it on, or, with no designer's copper to compare, on both outer and inner layers (a microstrip's width outside is not a stripline's inside). What to do: keep the class off the layers it must not use with a custom rule in the board's `.kicad_dru`, `(rule "RF_ANT_INT layers" (condition "A.hasNetclass('RF_ANT_INT') && A.Layer == 'B.Cu'") (constraint disallow track))` per such layer (applied on every rung of the router's ladder; a new file starts with `(version 1)`), or give it a `track_width` rule per layer (a condition on `A.Layer`) for the layers it may use, each layer at its own width; then run again. |
| `class_track_widths` | Route and layout measure saved tracks by copper layer and width for each routed net whose saved class specifies a width above Default. The class width is the preferred width. `below_class_width_mm` measures narrower copper separately from `native_width_errors`, the active width findings KiCad names on the net when checked. The reference track distribution is included only for an exact match of electrical terminals. |
| `designer_widths` | Route and layout with a designer's board, the `--reference` or else the input with its tracks (since 2026-11-08, trial 8): per net with matching terminals that the board written runs at most half the designer's width, `net`, `class`, `designer_width_mm` and `routed_width_mm` (each the width at half the net's track length), the lengths, and `unassigned_class` with its width where the project defines a class of the net's name that holds no net. KiCad's DRC checks no width above the minimum, so nothing else says it; the next action gives the `net_classes` entry to paste and, for an unassigned class, that the .kicad_pro's `netclass_patterns` assign it for KiCad too (the rp2040 motor controller in route mode: VMOT 2 mm on the designer's board, 0.2 mm here; its project's VMOT class of 2 mm names no net). |
| `reference_keepout_overlaps` | Layout with an explicit `--reference`. Reports when at least half of a footprint antenna keepout overlapped the same retained board copper rule area in the reference and the saved board has no measurable overlap there. The check requires matching zone IDs, copper layer policies and shapes. The reference pose is an observation. A `trial_pose` appears only when the current placement model accepts it with the other parts held. A pose trial requires rerouting and native verification. |
| `pin_lines` | Whether the router's lattice had lines at the pin rows and why, in one line (the setting, route mode's or the final ladder's decision at the probes or after its leader's run, or a layout's decision at the kept placement's probe); absent where it had none. Also stdout's `pin lines: ...` and board-router.json's `pin_lines` and `attempts[].pin_lines` (since 2026-11-08, trial 8; a layout's own decision since the same day). |
| `repairs` | What the run added for KiCad's DRC that changes no connection: `neck_tracks` and `neck_track_mm` (the connection-width repair's tracks kept, docs/router.md "KiCad's connection width"), `pour_mode` (the kept board's `pours`: `connect`, `tracks`, `none`, ...) and `pours_as_tracks`. Routes and layouts; also at the top of `board-router.json` and `board-layout.json`. |
| `pours` | How whole the pours stayed, per net and layer, by the router's pour model on the board written (after the final route; not a decision): `pieces` (the fill's pieces holding copper of the net, slivers under 0.25 mm² left out; KiCad removes the others, `islands_removed`), `largest_share` (the largest piece over the pour's area inside the outline), `coverage` (the copper over that area), `largest_mm2`, `copper_mm2`, `area_mm2`, `cut_mm` (other nets' tracks anywhere on the layer, using chords for arcs). `bounded_regions` measures other nets' straight track centreline inside each generated rectangular region by zone UUID and bounds. Its total counts a track portion shared by regions once. A pour in more than 5 pieces, or whose largest piece holds less than half its copper, adds a next action: with `keep_whole_parts` (the parts of six pads or more under which a region on that layer would hold no other net's pad, at most four), the `add_pours` entry naming them, `{"net": "GND", "layers": ["B.Cu"], "keep_whole": [{"parts": ["U1"]}]}`; without, that a region under any such part would cut other nets' pads off on the layer, so `keep_whole` under parts does not fit it (a box clear of other nets' pads may; more room or layers). Until 2026-10-26 the action named `keep_whole` under parts in general, and an agent following it put one under the ICs on their own layer. Against KiCad's own fill (`benchmarks/fence/pour-pieces.py`) on the fence: copper within 2 %, pieces within about 10 %. Also `pour_quality` in `board-router.json`; score-kicad-board's `pours`. |
| `custom_rules` | Route and layout, where the project has a `.kicad_dru` (since 2026-10-27; before, in board-router.json's `rules.custom_rules` only): every constraint of its rules, `rule`, `constraint`, `status` and `detail`. `applied`: the router keeps it as KiCad does. `approximated`: kept more widely than KiCad asks (on every layer of the rule, for every item kind, for both nets of a pair, wherever a rule area may hold: MokyaLora's `TFT_ZONE_2W_Clearance`, 0.25 mm between tracks in its TFT_ZONE, kept as a pair rule on In3.Cu wherever the area lies), so it costs room but no finding. `no_effect`: the classes already keep as much, or nothing routed matches (MokyaLora's `Shrink_Clearance_In_MCU`, 0.085 mm below its classes' clearance: the router keeps the larger). `not_routing`: not about copper the router lays (silk, courtyards). `skipped`: its severity makes no error, or no layer has it. `unmodelled`: a routing rule the router cannot keep; KiCad checks it, and its findings name the rule (`rule 'NAME' ...` in drc.json's description): fix them by hand or move the parts; a rule that only allows less than the router keeps anyway (a `min` below the classes') costs nothing. A line on stdout before the verdict counts them by status: `custom rules (.kicad_dru): 2 constraints: approximated 1 (TFT_ZONE_2W_Clearance clearance), no_effect 1; ...`; a `length` or `skew` constraint the length matching keeps is said as what it became, with its bounds (since 2026-11-04; it was "approximated 1", explained as kept more widely than KiCad asks): `ULPI_exact_length length: applied as a length minimum of 31.40 mm and maximum of 31.60 mm (a meander after the route below the minimum, over the maximum the net routed again the short way and checked, no hard bound; verdict.lengths per net)`. |
| `environment` | The result-changing environment variables that were set, with their values: `PCB_ROUTER_SEED`, `PCB_ROUTER_SEEDS`, `PCB_ROUTER_NO_WALL_GUARD`, `PCB_SKIP_NATIVE`, `PCB_MIN_CONNECTION_FLOOR`, `PCB_KICAD_NO_NECK_REPAIR`, `PCB_KICAD_TIMEOUT`, `PCB_MAKER_KICAD_USER_LIBS`, `PCB_PLACER_WORK_SECONDS`, `PCB_NARROW_RESERVE`, `PCB_PROBES_SIDE_BY_SIDE`, `PCB_LADDER_STATS`, `PCB_WORK_RATE`, `PCB_MAKER_TEST_PANIC_ATTEMPT`; and `pcb_maker_version`, the build that ran (`pcb-maker --version` without the name: the commit and the build date). |
| `cosmetic_findings`, `warnings` | Counts of cosmetic findings and native warnings. |
| `silkscreen` | Layouts: the footprints' visible Reference and Value fields, placed after routing where KiCad's silkscreen checks find nothing (docs/placer.md, "The fields on the silkscreen"): `fields`, `moved`, `blocked` (fields no spot is clean for, kept at the spot with the fewest findings: the reference, the field and the findings predicted there; a next action lists them), `hidden` (with `"silkscreen": {"hide_when_blocked": true}` in layout.json, such fields are hidden instead) and `predicted` (KiCad's `silk_over_copper`, `silk_overlap` and `silk_edge_clearance` findings the fields will have, from our geometry). What `cosmetic_findings` has beyond them is the footprints' and the board's own graphics, which nothing moves. |
| `schematic_parity_checked`, `schematic_parity_issues`, `schematic_parity_by_type` | Whether a native check included schematic parity, plus the total and counts by KiCad type. A zero count with `schematic_parity_checked: false` has no native parity evidence, as in a `.net`-only project. A missing schematic footprint, a missing pad on a named net, or a confirmed pad-net mismatch makes the route or layout incomplete even when its copper has zero opens. Other parity findings remain visible for review. |
| `footprint_filter_mismatches` | Native schematic parity findings grouped by the actual PCB footprint ID and the symbol's filter text. Each group lists the affected references. Check the pin map and physical package against the intended part before correcting the footprint assignment or the symbol's filter in the source. Update the PCB from the schematic and rerun parity. |
| `open_pad_concentration` | When at least half of the nets with native opens name local singleton pads at up to three footprints, lists the count of distinct nets, the footprint references, and the pad UUID and net for each. Each reported footprint has at least four distinct nets. The saved-board copper check requires one pad in each reported group and omits nets with ordinary pours. Review the named pad rows and nearby copper together. |
| `missing_schematic_footprints` | Components KiCad found missing from the PCB. Use the reported reference and symbol value to check the footprint assignment and update the PCB from the current schematic. Set **Exclude from board** on a symbol intended to remain schematic-only. |
| `missing_schematic_pads`, `schematic_net_mismatches` | Pin-level evidence from the saved PCB's native parity check. When KiCad can export the checked schematic, a missing-pad entry's `schematic_netlist` gives the net, terminal count and pin type for an unambiguous reference and pin. It distinguishes a no-connect marker, a no-connect symbol pin type, an isolated single-pin net and a connected pin. The native parity checks still govern acceptance. For a net mismatch, compare the PCB and schematic nets at the reported reference and pin. `physical_pads` counts the affected lands when a pin uses several pads. |
| `rear_routing_candidates` | On a legally placed layout with native opens and a crowded front (that the crowding causes the opens is a hypothesis, not shown), lists exact passive SMD references that could use estimated rear headroom. If rear assembly is permitted, add approved names from `allow_back_additions` to `placer.allow_back` and rerun placement and routing. The area estimates include placement spacing. |
| `starved_thermals`, `internal_violations` | Pads short of thermal spokes; the router's own clearance check. |
| `closed_pads` | Route and layout: per footprint, the pads no track leaves and no via may enter (board-router.json's `vias_in_pads.closed`; route.log's `pads no track leaves, no via allowed`): `part`, `footprint`, `pads` (`AC9 (RESET)`), `nets`, `open_nets` (those KiCad reports unconnected, else the router) and `why`. KiCad reports such pads open until the class, the footprint or the via-in-pad rule changes; absent where there are none. |
| `board_outlines` | Route and layout, when the board file holds two separate board outlines or more (Edge.Cuts loops that do not touch: several boards in one file): `outlines`, their `boxes`, and `unconnected_across`, KiCad's unconnected items whose two ends lie on two different outlines (each end on the outline that holds its place, else the nearest), which no track or pour can join; `unconnected_across_items` names them, each by its two ends' uuids in KiCad's report. The open connections' actions say so first, and a pour's action says that `keep_whole` keeps a piece whole within one outline only. |
| `outline_waists` | Route and layout, when the outline has necks under 2 mm without mouse bites or a V-cut whose sides are no copies of one board: the board is checked and routed as one across them. Per neck `neck` (its box), `parts` (per side) and `repeated` (references the sides share); `unconnected_across` counts KiCad's unconnected items across the necks, and with any a next action says how to mark a tab if the file is a panel (a row of non-plated holes of at most 1 mm drill at most 1.25 mm apart, or a V-cut line with a V-CUT label). |
| `constraints_missed`, `unplaced` | Placement constraints not kept (with mm), parts without a legal place. board-layout.json's `constraints` lists every one kept or not, also those kept by construction (`back`, `front`, `rotation`, `opening_outwards`, and `place` with `x` and `y`, checked against the part's final pose). |
| `cut` | What the wall clock cut short (`deadline`, `negotiation guard`, `budget guard`, `ladder`, `placement`, `layout`). |
| `work_seconds`, `wall_seconds`, `work_wall_ratio` | The work done (what budgets count: seconds at the work rates), the wall clock, and the one over the other (about 1 where the rates fit the machine; lower where the wall clock ran ahead: load, KiCad's checks beyond their model, a pinned rate faster than the machine, the waits in the memory ledger (`ledger_wait_seconds`); above 1 where routers ran side by side, the rung probes or the rungs of a pitch, whose work adds up). |
| `ledger_wait_seconds` | The part of `wall_seconds` the run waited in the machine's memory ledger for its reservations (several threads waiting at once counted once; a run past its reservation that runs on while it waits is not waiting): neither work nor load, so `work_seconds / (wall_seconds - ledger_wait_seconds)` is the ratio of the run itself. Also in board-layout.json (`ledger_wait_seconds`) and on the layout's `layout: done` line. Since 2026-11-04 (trial 5: three runs waited 5 minutes each, said only on stderr). |
| `work_rate`, `placer_work_rate`, `work_rate_source` | The search expansions and placement units a second of work stood for, and where they came from: `measured` (the start-up calibration), `PCB_WORK_RATE`, `config` (the router settings `work_rate`, `placer_work_rate`) or `reference`. |
| `work_rate_measured`, `placer_work_rate_measured` | What the start-up calibration measured (also beside a pinned rate). |
| `kicad_check_work_seconds` | Layouts: the part of `work_seconds` that is KiCad's checks inside the layout (the board kept, the final ladder's attempts, the neck repair), each counted at a cost modelled from the board, the same whether the DRC cache answered it or not. |
| `pour_work_seconds`, `lattice_work_seconds` | `route-kicad-board`: the parts of `work_seconds` that are the pour analyses and fill probes (every phase's: the negotiation's pour refresh, the clean-up's and the via reduction's pour checks, the manual spokes) and the routers' lattice builds with the board's lowerings, each modelled from what it counted, the same on every machine; the rest is search. Since 2026-10-21 route mode's budget counts all of them (a 30 s budget used to buy 30 s of search and leave the pour work, up to 47 s on a panel, on top). |
| `skipped` | Steps of the router's attempt ladder that did not run while the board was incomplete, with why (`the narrow-signal step (signals at 0.15 mm) did not run: the ladder's budget was used (ladder_budget_seconds)`). |
| `outputs` | Absolute paths: `project`, `board`, `drc`, `report`, `verdict`, `placement`. |
| `next_actions` | What to do next, factual, in this order (since 2026-11-04; before, the input's own observations led the list on all three boards of trial 4): (1) on a panel whose parts' boards nothing tells (`sub_boards.unassigned`), the call for `--reference` (since 2026-10-27); (2) the input check's and the placement's warnings about what you gave (a keepout over a held part, a `keep_whole` region, a value below the project's minimum, a missing kicad-cli where the run went on without it, copper on a layer the stack lacks); (3) what this run left to act on: parts without a place and the outline to try, the wall clock's cut, a search at under half the reference's rate whose first route took the whole budget (below), open connections (where nets fought, which pads no track enters, the nets KiCad finds unconnected, what the evidence for density is, below) followed by the hot spot with the parts nearest it, a search at a loaded machine's rate, pads no via fits, starved thermals, the new DRC errors by type (the largest four each with KiCad's rule, the nets or items, and the setting or step for that type), constraints missed, the project's rules the router does not keep, parts on the board's texts, silkscreen fields without a clean spot, pours in many pieces, decoupling, the reference's capped or failed check, a skipped KiCad check, the steps that did not run; (4) last, the input's own observations, each starting with `nothing to do (the input's): ` (its tag: nothing this run did or can change; a complete run can still carry schematic parity review actions): fixed parts on the board's own texts and silkscreen graphics, fixed parts on top of each other, the findings within one footprint (`footprint_findings`) and the input's own (`input_findings`), and with `--reference` the reference's findings between two nets that the release still refuses. Observations of one kind that repeat per part fold into one line at the first one's place (since 2026-11-04; the Quanta75's four lines of fixed parts on its texts, Batch_1's three of overlapping fixed parts): `nothing to do (the input's): 4 fixed parts lie on 12 of the board's texts and silkscreen graphics (6.25u stabilizer on 8, J3 on 2, SW42 on 1, M2.5 on 1): all stay where the input put them (input_observations has each part's marks)`, `3 pairs of fixed parts overlap (C8 and U2 (courtyards), C16 and U2 (courtyards), M1 and U2): ...`, the first five named and the rest counted (`and 7 more`); a name that repeats (a panel's copies share their references) is named once with its count (`C2 and U1 (courtyards) 9 times`; since 2026-11-04, trial 5's SmartSpin2k named "C2 and U1 (courtyards)" five times). Only pads on each other cost a connection: an overlap of courtyards or bodies alone (parts under a socket or module on purpose: jetson-nano-baseboard's 36 parts under its SO-DIMM socket) says its pads are apart and that it costs none (since 2026-11-04, trial 5; it said their pads "cannot both be connected"). **Open connections in a layout**: the placement is the layout's, so `inspect-routing-run --reference`'s `reference.connected` (the designer's route on the designer's placement) is no evidence for it; the action says so, with KiCad's unconnected items on the designer's board where the reference was checked, and the layout's own routes of its placement (the footprint moves tried and kept, `moves`); in route mode (the input's placement) the designer's route is the evidence, as before. **The rate first** (since 2026-11-04): where a layout searched at under half the reference's rate and nothing ran after its first route (no move, no final ladder, nothing cut), the rate's action leads the open connections and says so: `this run searched at 0.39 of the reference's rate ...; the placement and the first route used 1502 of the 1500 s of work the layout's budget counts (total_seconds), so no move and no final ladder ran` (trial 5's SmartSpin2k: 475 unconnected so, 51 at the reference's rates; the action came third, after the hot spot). At a rate that is no reason the open connections' action says it instead (`connections are open (the placement and the first route used ...: a larger total_seconds in the layout config gives them their time)`). **The hot spot** (since 2026-11-04): the worst of `diagnostics.hot_spots` on an open net (else the worst), its layer, point and open nets, the tile's pads on them (else on the nets in conflict; up to four, `REF:NUMBER (x, y)` in mm, the rest in `diagnostics.hot_spots[i].pads`) and the other nets still in conflict there (since 2026-11-05; the remapper trial's hot spots had no object links), and the five parts nearest its point on its layer, each with its side and the distance from the point to what of it is on that layer (`over it` inside): its pads on the layer (a through-hole pad on every layer) and, on an outer layer, its body (courtyard, else the pads' box) when it lies on that side, so that a key switch on the front over a back-side MCU's hot spot counts by its pins only: `hot spot 2 (diagnostics.hot_spots[2]: B.Cu at 202.75, 149.46, on the open nets +1V1; their pads in its tile U1:23 (202.70, 149.50); in conflict there with +3V3): the parts nearest it on B.Cu are C16 (back, 0.7 mm), U1 (back, 0.8 mm), ...`, then what moves them (in a layout `place`, `apart` or a `keepout`; in route mode the input). Where KiCad names an unconnected item on one of the hot spot's open nets, that open link leads the action (the one of those nets' items nearest the hot spot), whose far end may lie elsewhere, with a `near` to paste (since 2026-11-04; trial 5's jetson: R8, 10.4 mm from J1:11, was not among J1's neighbours): `KiCad's open link on Display/TMDS_D0_C_N runs from J1:11 to R8:2, 10.6 mm apart (its unconnected item). To bring R8:2 closer to J1:11, add {"minimize":true,"pad":"2","part":"R8","pin_of":"J1:11"} to constraints.json's near list`: the free part of the two (of two free ones the one with fewer pads) to the other's pin; with both fixed it says so; in route mode the input to change. The hot spot follows with its distance to the link's nearer end (`; hot spot 0 (...) lies 0.4 mm from J1:11: the parts nearest it ...`); farther than 5 mm it is said to be where nets fought, not where this connection is missing, and its parts are not offered (the eval study of 2026-11-24: USB_Keypad's hot spots lay 16 mm from its open item and read as its cause). Without an open link (no hot spot on an open net, or KiCad's item is no pad pair) the hot spot gets its distance to the nearest end of KiCad's unconnected items (zones' items left out) the same way (SUMEC_MK_IV's worst hot spot, 20 mm from the nearest open item, led its open connections). Where the board file holds several separate outlines, a line before the open connections' says how many of KiCad's unconnected items join copper on two of them (`board_outlines`). **Starved thermals** name the pads (`U1.57 on F.Cu`) and say what `solid_starved_thermals` did for them on the board written: connected solid and undone by KiCad's check (its unconnected items and findings solid and with spokes; `solid_starved` in board-router.json), kept, or not tried on this board; then the first pad's advice (without the solid connection where it was undone). A layout with at least three measured capacitors and fewer than 60 % within 3 mm of a same-rail supply pin reports the median and the farthest measured pins. `decoupling_near_suggestions` lists capacitors beyond the near limits used by a fresh schematic template when the schematic assignment agrees with the placed board’s template on which IC they serve. Each entry gives the capacitor, the exact supply pin, the suggested `max_mm` and the measured body-to-pin distance. The example in `next_actions` uses the same pair. Fixed capacitors and capacitors with an existing `near` entry keep their established placement intent. The same-rail measurements remain available when there is no schematic pairing. |
| `input_observations` | The input's own observations of `next_actions` (4) each in full and untagged, before the folding (since 2026-11-04; absent when there are none): every fixed part on the board's texts with its marks, every overlapping pair, the footprints' and the input's own findings' notes, the reference's findings the release refuses. |
| `drc_cache` | `{calls, hits}`: KiCad DRC runs the process asked for (its attempts, the reference) and how many the DRC cache answered (see below). |
| `skeleton_race` | Placement and layout with the placer's race over skeletons on (the default; absent on a panel of copies or sub-boards): what it did in one line, board-placer.json's `skeleton_race.summary` (`not run: each seed's placement had 200 s of work, under min_work_seconds (500 s), ...` or `ran: the kept seed placed its skeleton of 68 parts first (level 3) ...`; since 2026-11-06). |
| `room` | Placement and layout (also an input error that a constraint needs more room than the outline has): the room as numbers, below. Absent for a route. |
| `panel_copies` | Placement and layout with the placer's `panel_copies` on (the default; `false` turns it off) where the board holds copies of one circuit (docs/placer.md, "Panel copies (2026-10-26)"): `count`, `parts_per_copy`, `movable_per_copy`, `detected_by` (`net islands`, `numbered references`, `repeated references`), `regions_from` (`fixed parts`, `outline pieces`, `slices`), `replicated` (false: the seed kept placed the board as a whole, the copies' surroundings differing), `shared_parts` (movable parts in no copy, placed after the copies), `moved_after_replication`, `cell_classes` (the copies grouped by their surroundings, the most constrained first: per class its `copies`, the first the one placed for it, and its counts of `constraints`, `outline_pieces` and `obstacles`; docs/placer.md "Cell classes (2026-10-27)"), `by_cell_classes` (each class was placed from its own copy), and per copy its `region`, `rotation` and `offset` from the first and its `references` in the first copy's order: the mapping (the r-th reference of every copy is the same part of the circuit; where references repeat, the region tells the instances apart). Since 2026-11-04 also where the outline falls into two pieces or more and no part lies on any (a netlist import's stack without `--reference`, every part off the pieces): `count` 0, `pieces`, `tabs`, `cut_by`, `frames` by their shape (pieces spanning nine tenths of the panel along a side with at most a fifth of its area: a frame or rails), `sub_boards` the other pieces' boxes with no parts, `parts_assigned` 0 and every movable part in `unassigned`; nothing is held or kept off (the placement is the one without it), and the first next action is "give --reference or a region per board: the outline falls into N pieces (cut by ...) and M of the M movable parts lie on no board ..." (where more than a tenth of the movable parts have no board and the parts tell fewer than two). `pieces`, `movable` and `parts_assigned` are in every report. Absent otherwise; also in `placed/board-placer.json`. |
| `sub_boards` | Placement and layout with the placer's `sub_boards` on (the default; `false` turns it off) where the outline falls into two pieces or more once its tabs are taken out (necks under 2 mm where the parts on the two sides are copies of one board: more than half of the smaller side's parts repeat a reference of the other, or, where a side holds no parts, most of the board's parts are copies; a neck with different parts on its sides is a waist of one board, `outline_waists`; since 2026-10-28 also rows of three or more non-plated holes of at most 1 mm, at most 1.25 mm apart, drilled as footprint pads across a wider bridge, and V-cut lines: a line on a user layer with a "V-CUT" or "V-SCORE" text at one end; pieces only tabs join) and parts lie on two of them (a panel of different boards), or a piece holds no part (a frame or rail) (docs/placer.md, "Sub-boards in a panel (2026-10-27)" and "Tabs and frames (2026-10-28)"). `cut_by` says what cut the outline (`narrow outline`, `mouse bites`, `V-cuts`); `frames` the boxes of the pieces no fixed part, reference place or placed input puts a part with pins on (a panel's frame or rails; at most half the area of the pieces with parts): no part is placed on a frame or within 1.5 mm of a tab, and `kept_clear_mm2` is the area so kept free (the room advice names it: room is made on the boards, not on the frame). `count`, `from` (what told each part's sub-board: `fixed parts`, `reference board` (`--reference`), `placed input`, `nets`), `split_nets` (nets of one name with parts on several sub-boards: one net per sub-board to the placer, and to the router where no copper of theirs runs over a tab), `unassigned` and `unassigned_parts` (movable parts nothing puts on a sub-board; with more than a tenth of them the placement holds none, the run is `incomplete` (exit 1) with the boards' parts mixed as the first problem of its `reason` (since 2026-10-27: the OpenESC panel's place check placed all 404 parts and exited 0), and the first next action asks for `--reference` or region constraints; no `suggested_outline_mm` and no room advice then, the parts being short of room only because two boards' parts share one), `conflicts`, `held` (the parts were held in their sub-board's region: two sub-boards or more and at most a tenth of the movable parts without one, since 2026-10-28 also where no net lies on two), `tabs` (the tabs' boxes, kept free of parts with a 1.5 mm margin where the parts are held or the board has a frame), and per sub-board its `region`, `parts` and `movable`. `overhang_connectors` and `overhang_designer` (since 2026-10-28) count the movable parts that may reach over the tabs and frames: on a panel (a frame or rails, or copies) a connector (by footprint name, the USB-C pads, or a J/P/CN/USB reference whose courtyard reaches past its pads on one side) whose pads and copper keep the edge margin off the tabs and frames themselves, and with `--reference` a part the designer's placement put into the kept area, as far as it reached there (docs/placer.md, "Frames on panels of copies, and what may reach over them"). Panels of copies are placed as `panel_copies` says; since 2026-10-28 they also report `sub_boards` (with `copies: true`, never "boards mixed") where the cuts find pieces: each copy keeps off every copy's frames and tabs, carried into its own frame, so that the copies stay alike (not along a board edge where a copy's part is held); `keepout_margin_mm` is 1.5, or null where the copies are too small for the margin (the grown keepouts would take more than 0.3 of a copy: OpenRX) and nothing is kept off. KiCad's unconnected items between two sub-boards of a net on several by design (split by the parts whose board is known, or holding two parts of one reference; one item per such net and sub-board beyond the first: what joining the boards would take, which KiCad reports on the designer's own board too) are moved to `unconnected_across_sub_boards` in drc.json and not counted; items across of other nets count (the placement put their parts on two boards). Absent otherwise; also in `placed/board-placer.json`. |
| `free_dimension` | A layout with `free_dimension`: `chosen` (the size in mm), `kind` (`probe`, `retry`, `confirm` or `closest`: which run the result is), `clean`, `probes` (layouts run), `reason` (the smaller sizes and how they failed, a price threshold exceeded, sizes left untried), `edge`, the search's `work_seconds` and `wall_seconds` (the verdict's own fields are the chosen layout's), `report` (`free-dimension.json`). |

`room` says how full the board is and how large it would need to be, so
that an agent enlarges it in one step. `room`, `utilization` and
`target_utilization` measure whether a legal placement fits, not whether
it routes:

| Field | Meaning |
| --- | --- |
| `outline_mm` | The outline's box, width and height. |
| `usable_mm2` | Per side (front, back): the outline's area (a polygon's own, less its cutouts) less the band along the edge bodies keep free and less the fixed parts' bodies. |
| `parts_mm2` | Per side: the movable parts' bodies (courtyards, or the body box) with the spacing around them; a row is one body. |
| `utilization` | `parts_mm2` over `usable_mm2`, per side: what the placement hints call full above `target_utilization`, and board-placer.json's `utilization`. |
| `target_utilization` | 0.7: above it a side is called full and placement gets hard. |
| `missing_mm2` | Per side: the usable area missing for the parts at the target. |
| `smallest_outline_mm` | The smallest outline (at the current aspect, unless large parts need another) that holds every side's parts at the target by area, its large parts side by side in strips across it (as full as the target), the largest part, every row and every edge's parts. An estimate from sizes: the placement may need more. |
| `suggested_outline_mm` | Only when parts found no place or the outline failed a check, and never on a panel whose parts' boards are unknown (`sub_boards`): the larger of the current outline and `smallest_outline_mm` per side, or, where area is not what is missing, a quarter more area. `next_actions` names it ("try an outline of 118.0 x 107.5 mm ..."). |
| `limits` | What bounds the outline besides area: rows, edges, parts larger than the board. |

Urchin (both keyboard halves, 80 parts) on 80 x 40 mm, its two 5 x 3 key
grids as rows: exit 2 in 0.04 s, with

```json
{"outline_mm": [80.0, 40.0], "usable_mm2": [3080.0, 3080.0], "parts_mm2": [1419.0, 3038.8],
 "utilization": [0.461, 0.987], "target_utilization": 0.7, "missing_mm2": [0.0, 1261.2],
 "smallest_outline_mm": [118.0, 107.5], "suggested_outline_mm": [118.0, 107.5],
 "limits": ["the row of SW1 (15 parts) is 88.2 x 43.7 mm", "the row of SW18 (15 parts) is 88.2 x 43.7 mm"]}
```

(118 x 107.5 mm then places every part; two `near`/`relative` entries
written for the halves side by side miss.) On the 230 x 95 mm board the
walkthrough uses, the layout's verdict has `utilization` [0.066, 0.141],
`missing_mm2` [0, 0] and `smallest_outline_mm` [213.5, 54.5]: the two
grids side by side.

If a layout's blocking solder-mask bridge names a round non-plated hole
and a round plated pad on two held footprints, the verdict checks whether
the drill reaches the pad's copper. With matching native items and exact
authored holds, `next_actions` names both footprints, the faces and pad
positions KiCad reported, and the `place` or `add` entries. Review the
footprint geometry or change an authorized pose before rerouting.

**KiCad's DRC is cached.** Every DRC (15-25 s and 2.2 GB on a large
board) is kept in `$XDG_CACHE_HOME/pcb-maker/drc/` (`~/.cache/pcb-maker/drc/`
without it) under a hash of everything it reads: the board's bytes and file
name, the KiCad files of the project (`.kicad_pro`, `.kicad_dru`,
schematic sheets, project libraries and library tables), the user's global
library tables, `KICAD*` environment variables, the `kicad-cli version` and
the DRC's arguments. A rerun on unchanged inputs (the reference board, an
attempt that ends with the same board, a second run of the same task)
takes the stored report: KiCad's exact bytes, validated as a fresh report
is; only its `date` is the first run's. `PCB_MAKER_CACHE_DIR=<dir>` keeps
the cache in `<dir>/drc/`, `PCB_MAKER_NO_CACHE=1` turns it off. Processes
side by side share it safely (entries are written whole, then renamed);
it keeps at most 2000 entries and 2 GB, dropping the least recently used.
KiCad's own libraries are not hashed (only the `lib_footprint_*`
warnings, which never count, read them); after changing them without a new
KiCad version, delete the directory. The stderr line `board check: ...`
and `drc_cache` in the verdict count the calls and hits.

**KiCad's DRC runs without the global libraries.** kicad-cli's DRC
otherwise loads every footprint library of the global `fp-lib-table`
first (about 2.7 s, 9 CPU s and 2 GB per call, for the `lib_footprint_*`
warnings alone), so pcb-maker runs it with `KICAD_CONFIG_HOME` set to
`<cache root>/kicad-config/` (the root as for the cache: `PCB_MAKER_CACHE_DIR`,
else `$XDG_CACHE_HOME/pcb-maker`, else `~/.cache/pcb-maker`), which holds
empty `fp-lib-table` and `sym-lib-table`. Every finding but the
`lib_footprint_*` warnings is the same. The user's configuration is kept
when the project sets a `lib_footprint_*` severity to `error`, and with
`PCB_MAKER_KICAD_USER_LIBS=1`. ERC runs the same way (its library rules
`lib_symbol_issues`, `lib_symbol_mismatch`, `footprint_link_issues`):
these three warnings depend on the libraries installed, so
`verification.json` leaves them out of `erc_violations` and counts them by
type in `erc_library_warnings`.

Read `verdict.json` for the routing or layout decision. The native
summary in `result/verification.json` sets `complete` only when
`erc_violations`, `drc_design_violations`, `schematic_parity_issues`
and `selected_net_unconnected_items` are all zero. An existing schematic
ERC finding can therefore leave that flag false on a fully routed layout.
Use `erc.json` to inspect the schematic finding. The summary's
`schematic_parity_checked` is true after a native parity check or an exact
verification cache match for such a check. Board-only verification leaves
it false and applies the existing completion policy.

The other commands exit 0 when done, 1 when their result is incomplete
(`repair-kicad-silkscreen` with findings left, `import-kicad-netlist` with
footprints missing, `fab-kicad-board` refused by the DRC), 2 on usage or
input errors and 4 on internal errors.

### Route reuse in the verdict

`route_reuse` is present after an opt-in routing decision, including when
an illegal initial placement stops a layout before routing. Ordinary
runs omit the object.

When exactly one terminal moves or leaves a net, reuse can keep its
straight-segment tree. The moved pad must be a leaf with a short direct
escape to its previous joint on the same layer. A removed terminal's
branch is trimmed back to the first surviving pad or joint. The tree
must connect every old terminal through exact track joins and plated
through-hole pads, with its tracks on layers separate from any copper
zone on the net. The trial uses current zones and rules and requires a
fresh native connectivity, schematic parity and all-severity DRC check.

| Field | Meaning |
| --- | --- |
| `status` | `accepted` means the returned board contains the verified prior copper. `skipped` means reuse ended before the trial. `fallback` means the warm candidate was set aside and ordinary routing continued. |
| `reason` | Why reuse was skipped or fell back. |
| `retained_nets`, `retained_segments`, `retained_arcs`, `retained_vias` | Counts of copper retained from the previous board. All four are zero after a skip or fallback. |
| `partial_nets` | Accepted branch reuse, keyed by current net name. Each entry gives the `pruned_terminal`, counts of retained and pending terminals, and counts of retained and pruned segments. `spliced_segments` counts new segments that reconnect a moved terminal. The detailed report also records branches offered in a rejected trial. |
| `pending_routed_connections`, `pending_routable_connections` | Accepted reuse only. The router's counts for work left after keeping copper. Both are zero when the retained copper already completes the board. |
| `saved_routable_nets` | Accepted reuse with zero KiCad opens and schematic parity findings. An inventory of named nets with multiple electrical contacts on the saved board. A retained net can also need pending routing. |
| `renamed_references` | Previous to current reference for unique matching schematic instances on an accepted trial. Detailed route reports also list proposed matches after fallback. |
| `restored_poses` | Layouts only. The number of footprints moved back to their previous positions on the returned board. |
| `keepout_skipped_nets` | Current net names withheld from reuse because their previous copper would encounter an explicit current copper keepout. Ordinary routing handles these nets. |
| `baseline_schematic_parity_by_type` | KiCad schematic parity findings on the current input that prevented the reuse trial from starting, counted by type. |
| `new_findings` | Counts by KiCad finding type when the reuse comparison found new violations on the rejected candidate. |
| `new_finding_examples` | Up to six of those findings, favoring different types. Each shows the native severity and description, a named rule when available, and up to three items with their native descriptions, net names, and anchors where available. |

The examples describe the rejected trial. An item's `anchor_mm` is
the position KiCad reported for it, such as a track endpoint. The
conflict can occur elsewhere along that item. Long descriptions may be
shortened. The top-level findings and verdict describe the returned board.
For trial eligibility, provenance and work, read `board-router.json`
in the route output or `result/board-router.json` in the layout output.

### The layout's report: `<out-dir>/board-layout.json`

| Field | Meaning |
| --- | --- |
| `routed.routed_connections` / `routable_connections` | The router's made and needed connection counts. Accepted reuse counts the remaining work after retaining copper. |
| `routed.unconnected_terminals` | Pads the router could not reach. |
| `routed.native` | KiCad's raw counts: `complete`, `drc_design_violations`, `selected_net_unconnected_items`, `schematic_parity_issues`. They count every finding of any severity, silkscreen warnings included (a layout places the fields itself, `verdict.silkscreen`; `repair-kicad-silkscreen` places them on any board with the layout's scorer and also clips front silkscreen strokes off pads, other silkscreen and the edge; it takes any outline, a `gr_rect` too, and keeps strokes within the bounding box of one that is not a rectangle, noted in `proposal.notes`), so `complete` is often false on a board that passes: `verdict` decides. |
| `routed.vias`, `routed.length_mm` | Cost of the routing. |
| `constraints[]` | Every constraint: `kind`, `part`, `other`, `satisfied`, `violation_mm`. The ones the placer keeps by construction are listed too: `back` and `front` (a part on its side), `rotation` and `opening_outwards` (a part at an allowed angle). |
| `routed.diagnostics.dead_pads[]` | Pads the router's lattice found no way out of (`pad`, `net`, `at`, the objects `near` it). `access` says how open the pad is, the most open of its layers: `no_legal_access` (no point of the pad keeps the clearances for the net's narrowest track: by the router's rules no track can end on it, a wall of the input or of the class), `off_lattice` (no lattice node within 2 mm: the pad lies off the routed area, a part beside the board outline), `no_lattice_node` (a track may end on the pad, but no lattice node within 2 mm admits the class's full width: the lattice's and the neck rule's limit, not the input's), `stubs_blocked` (nodes admit the class's track; items or the edge block the straight stubs to the nearest three), `start_blocked` (a stub from the pad's best point is free, but a plain pad's stubs start at its centre, which is not), `escape_not_kept` (free from the router's own start too: the router did not keep the escape), `unknown` (not measured: a pair's launch). Only the first two are walls by the router's rules, and KiCad is the judge: on the eval study's cases KiCad accepted a track at a `no_lattice_node` pad (SUMEC's IC2.11) and at a `start_blocked` one (WLED_Matrix's LDO1.3). `wall` (since 2026-11-07) says what walls it, per layer (by KiCad's name, `B.Cu: ...`): where no point of the pad keeps the clearances for the net's narrowest track, the item nearest to being cleared at the best point (its label, net and place; two footprints may share a reference) with how far it is and what is asked; where a track end fits, what blocks the stubs from there to the nearest nodes the class's track may use, and for a free stub whether the router's own start reaches the node. |
| `routed.diagnostics.conflicted_nets` | Nets still fighting for room when the router gave up. |
| `routed.nets[]` | Per router net (also board-router.json's `nets`): `connection`, `terminals`, `status` (`routed`, `partial`, `unreachable`, `trivial`), `unconnected_terminals`, `vias`, `length_mm`. A `partial` or `unreachable` net has `open`: its open terminals grouped by why the router left them open, each group `reason`, `terminals` (exact) and `pads` (the first 16 labels, `U1.3`). The reasons are what the router decided at that point: `no_access` (the pad has no lattice node: a dead pad), `net_not_routable` (the net was never searched: fewer than two of its pads have access), `no_path_static` (the negotiation's board-wide search from the net's tree found no path to the pad even through other nets' copper: what never changes during the negotiation is in the way. The group's `pads` are the pads the tree did not reach; the wall may be at the pads the tree started from. `walled` names the pads the wall is at where the router finds it at a pad, a pad of the net whose pocket is closed on the lattice within 1 mm of it: a pad of `pads`, or the tree's pads when every one of them is so closed; `wall` says whose copper bounds the pocket: `input` (the board's own pads, copper, rule areas, holes or edge), `router` (vias the router planned before it routed: a BGA's fan-out, vias in pads) or `input_and_router`, and `blocked_by` lists up to 8 of the obstacles; a group's `walled` and `blocked_by` are those of its pads with a located wall, so a pad of `pads` that is neither in `walled` nor reached by a tree in `walled` has none located; without `walled` the router located no wall at a pad: a region cut off farther away, a pad on several layers), `no_lattice_exit` (as `no_path_static` with `walled`, but a 5 µm raster search shows the rules leave a way out of the walled pad's pocket for the net's track: the wall is the routing lattice's pitch, not the board; a finer `tuning.pitches` may pass it. Decided only for nets that end open; it changes no route), `repair_skipped:<why>` (open when the negotiation stopped, and the hard repair did not search the net again: `work_deadline`, `budget` or `hopeless`), `no_path_hard:<scope>` (the hard repair searched with every other net's copper as a wall and found no path, within its `corridor`, `window` or the whole `board`), `lost:<phase>` (joined after the hard repair, then a later phase left it open: `hard_repair`, `trouble_repair`, `clean_up`, `via_reduction`, `bends`, `thermal_spokes`, `straighten`, `compaction`, `pour_stitching`, `manual_spokes`; `pour_stitching` only for a pad the stitching's first look found on the pour's main piece), `pour_apart_before_stitching` (the negotiation's pour model counted the pad joined, but the pour stitching's first look, before it changes anything, found it off the pour's main piece, and the stitching did not join it), `copper_check` (the router counted it joined, but the copper it writes leaves it apart), `unknown` (nothing recorded; also the rest where the router recorded fewer terminals than `unconnected_terminals`). An unreachable net lists all its terminals. `open_repair` (a net whose copper from the open repair after the router KiCad accepted, in route mode and in a layout): `status` `closed` (KiCad finds no unconnected item on the net: the net is `routed` with no `open`) or `improved` (KiCad still finds `kicad_unconnected_items`; the net's count and `open` are the router's from before the repair, because KiCad's items do not say which pads are apart afterwards: an item names the two nearest objects of two parts of the net, a track as often as a pad, and not the parts' other pads; `kicad_unconnected_pads` lists the pads the items do name, each at an end of a connection still missing), `unconnected_terminals_before` and `open_before` (the router's count and reasons before the repair), `added_vias` and `added_length_mm` (the repair's copper, counted in the net's `vias` and `length_mm`). `repair` (an open net the hard repair took up): `complete` and `in_conflict` when the negotiation ended, `skipped`, and where it was in conflict, `conflicted_with` (the nets whose copper covered its first conflicting nodes, up to four) with `conflict_at` and `conflict_layer` (`via` for a via). Where the router keeps several nets of one name (a panel's sub-boards or cells), each has `copy` (its index among them in `nets` order) and `pads` (all its terminals' labels). |
| `routed.diagnostics.hot_spots[]` | Where they fought longest (`layer`, `at`, `history`, the tile's `box`), worst first, with what was there when the router stopped: `pads` (`ref`, `number`, `net`, `at`; `pads_in_tile`), `nets` (copper crossing the tile on that layer) and `conflicted_nets`. Observations, not a cause. |
| `constraint_warnings` (in `placed/board-placer.json`) | Inputs accepted but not acted on; the verdict's `next_actions` repeat them. |
| `placement_digest` | A sha256 of everything the steps after the placement read from it (the placed board, the lowered placement problem with the placer settings it carries, the race's other placements, the pin swaps, the work the placement booked; since 2026-10-28), also printed on stderr as `layout: placement digest <sha256>` before the race: two layouts of one binary, task, settings and rates with the same digest route alike (benchmarks/agent-tasks/run.py `--reuse` keys on it). |
| `stopped_after_first_route` | With the layout option `stop_after_first_route` (a measurement mode, off by default): the layout ended with its first route's board, KiCad's check and the report, skipping the moves, the polish, the final ladder and the neck repair. |

When the layout selects another placement seed, keeps a footprint move,
or restores prior poses, the placement report and view in `placed/` and
`result/` show the selected poses. `selected_placement` records the source
(`layout` or `route_reuse`), the seed's pool index for layout, and the
number of kept layout moves. `initial_placement_report` links to the
original report beside its board and animation in `initial-placement/`
or `reuse/initial-placement/`. Final wirelength, constraints and room use
the selected placement. `seed_kept`, global metrics and the pre-move
wirelength stages describe the base solver seed, when one applies. Pool
work still covers the original placement run. Solver diagnostics tied to
the original placement remain in its archived report when they do not
apply to the selected placement.

The placer's own numbers do not predict whether the board routes: board-placer.json's
`wirelength_source_mm`, `wirelength_global_mm`, `wirelength_legal_mm`,
`wirelength_final_mm`, `global_iterations`, `global_overflow` and
`skeleton_race.skeleton_wirelength_mm`, board-layout.json's `congestion`
(RUDY `scores`, ratsnest `crossings`; the race's pre-ranking, also in
layout.log) and place-kicad-board's "wirelength source" line are the
placer's objective. The routed probes and KiCad's native checks decide.

## When it does not work out

- **`placement is not legal (unplaced [...]); hints`.** No position
  satisfies the hard constraints for these parts. The layout stops before
  routing (exit 1) and writes no `board-layout.json`: `verdict.json` has
  `unplaced`, `error` and the hints in `next_actions`. The hints after the
  semicolon say why: a side that is more than 70 % full, or parts held by an `edge`,
  `region` or overhang constraint. Give them
  more room: a larger `outline` (`room.suggested_outline_mm`), a wider
  `region`, a looser `edge` `max_mm`, parts on the other side, or unfix a
  neighbour.
  `placed/board-placer.json` has the same `hints` and the `utilization`
  per side. "Full" is the room's measure, `room.utilization` (since
  2026-11-04): the movable parts' bodies with spacing over the side's
  usable area, the outline less its edge band, its cutouts and the fixed
  parts' bodies (`the front side is 92 % full (room.utilization 0.923:
  the movable parts' bodies with spacing over its 3080 mm2 usable, ...)`).
  Before, the hint and board-placer.json's `utilization` counted every
  component, fixed parts, the board's marks and a panel's kept-clear
  frames included, over the outline's gross area: trial 4's ULK panel was
  "99 % full" beside `room.utilization` 0.135, and its parts lacked a
  place for the column stagger, not for area. The outline's advice after
  the hints says how full a side is only where no hint said it (since
  2026-11-04: `try an outline of 118.0 x 107.5 mm (...): the front side
  is 1188 mm2 short of 70 %`; it repeated the hint's "% full").
  Before giving up, the placer shrinks the halos and the spacing (down to
  the copper clearance) and retries legalization with the stuck parts
  first. As a last resort it lets courtyards overlap: bodies shrink to
  their fabrication outline and pads (`"tight_bodies": true` in
  `board-placer.json`). It does that only where the project's DRC does not
  treat a courtyard overlap as an error; `"tight_bodies": false` in the
  placer config forbids it. Parts held at an edge then need only their
  copper on the board (`"edge_copper": true`): a card edge's fingers fill
  a tab of the outline that their courtyard overhangs, key notch and all.
- **A `near` or `relative` constraint is not `satisfied`.** Geometry forbids
  it, or it conflicts with another constraint; `violation_mm` says by how
  much. Relax the distance, or fix the part yourself.
- **Connections stay open.** First ask which pads, under which rules:
  `pcb-maker inspect-routing-run <out-dir> --reference <designer's
  project>` (section "Which nets stayed open") names each open net's groups
  of pads, KiCad's pairs, the rules and their sources, and the designer's
  route of the net. `routed.diagnostics` adds `hot_spots` (where nets
  fought longest, with the pads and nets in each tile) and `dead_pads`
  (pads the router's lattice found no way out of; only `access`
  `no_legal_access` and `off_lattice` are walls by the router's rules, the others have a
  legal track end the lattice or the stub rules did not use). When KiCad
  also reports an open at the same pad on the same net, the verdict names
  up to three such pads with their `access` and shows the layer,
  clearance or obstacle from the router's access check. For example, a pad inside a track keepout points to a
  placement or rule-area change. Density is a hypothesis, not a
  finding: when the designer's board routes the same placement under the
  same rules (`reference.connected: true` for the open nets), density is
  not the reason, and a larger outline, more layers or narrower classes
  are not called for. The verdict's next action says the same. The discriminating query is the
  reference's route of the open nets beside the groups the run left;
  `benchmarks/corpus/run.py --shrink` shows how much a design can shrink
  when there is no reference.
- **Pads or copper too close to the edge.** How parts at an edge keep the
  copper-to-edge clearance is described once, under `edge` in
  [constraints.md](constraints.md); a `copper_edge_clearance` error left in
  drc.json is copper the placer did not hold (a fixed part, a board
  graphic) or a project rule changed after the placement.
  When KiCad identifies a held pad against the board outline, the verdict
  names that pad and gives the measured and required clearance when KiCad
  reports both. Check the source footprint's pad geometry and the held
  interface pose against the outline before rerouting.

## 5. Order it

```sh
pcb-maker fab-kicad-board <out-dir>/result <board-id> <release-dir> [--profile jlcpcb]
```

is a fab release gated by KiCad's DRC:
- **The checks.** KiCad's DRC as `drc-kicad-board` runs it (the DRC cache,
  zones refilled, the library-free KiCad configuration) and, with a
  schematic beside the board, its ERC (counted in the manifest, never
  blocking). While copper errors or unconnected items remain, the release
  is **refused**: exit 1, nothing exported, `verdict.json` with the counts
  in `reason` and the first findings in `next_actions` (`clearance: Track
  [GND] on F.Cu at (12.30, 4.10); Pad 2 [VBUS] of J1 ...`). With
  `--input <project-dir>` (since 2026-10-27) the clearances, hole
  clearances, holes too close and shorts between parts the layout kept
  where the input has them (framework_mobo_lefthalf's fixed R31 and R32
  under J1's pads) are named per type first, as the layout's verdict
  names them: the input's, not the layout's to move, with what to do;
  the rest are the layout's ("move what the layout put too close").
  Without `--input` the board is judged on its own and every part is its
  own, so the refusal's first action names `--input <the project it came
  from>` for a board pcb-maker laid out or routed (since 2026-11-04).
  The order of `next_actions` is the layout verdict's (since 2026-11-04;
  before, the notes below led it): the release's own actions first (on a
  refusal what to fix, with `--input` the kept parts' findings after that
  line, then the blocking findings; released, the findings it went ahead
  with, the parts without an LCSC number, the rotations to check), last
  the footprints' own and the board's own findings that do not block,
  tagged `nothing to do (the input's): ` or `review the input footprint: `; `verdict.input_observations`
  holds those two notes. The refusal's `reason` is unchanged.
  Warnings,
  excluded and cosmetic findings (silkscreen, text, library) do not block.
  `--allow-findings` releases anyway and lists the findings it allowed.
- **The files**, all through kicad-cli: `gerbers/` (every copper layer,
  both masks, `Edge.Cuts`, and the pastes and silkscreens something lies
  on; `--layers F.Cu,B.Cu,...` to choose; silkscreen clipped by the mask;
  zones refilled), the Excellon drill files (PTH and NPTH apart, mm,
  absolute origin) and their Gerber maps; `<board-id>-gerbers.zip` of
  that folder, which is what the fab's upload takes (written without
  compression and with a fixed date, so the same files give the same
  zip); `assembly/<board-id>-pos.csv` (kicad-cli's position file: both
  sides, mm, KiCad's rotations, DNP parts left out);
  `assembly/<board-id>-bom.csv` (kicad-cli's BOM of the schematic grouped
  by value, footprint and the symbols' LCSC field when they have one, or
  without a schematic the board's footprints in JLCPCB's columns);
  `netlist/<board-id>.d356` (IPC-D-356, the fab's electrical test) and,
  with a schematic, `netlist/<board-id>.net`; `<board-id>.step` (the 3D
  model with the parts' models KiCad finds; a failure is listed in
  `skipped`, `--no-step` skips it).
- **`fab-manifest.json`**: `released`, `profile`, `kicad_version`, `drc`
  (errors by type, unconnected items, warnings, cosmetic, `clean`,
  `findings_allowed`; `footprint_own`, errors within one footprint each,
  which do not block, and `footprint_note` with the custom rule that
  accepts them, as in the verdict's rule above; `input_own`, the board's
  own edge clearances, mask bridges and courtyards with no track or via in
  them, which do not block either, and `input_note`; section 4, "The
  input's own findings": the board judged on its own, or with `--input
  <project-dir>` against the board it was laid out from; a short or a
  clearance between two nets always blocks), `erc` (errors, warnings, library warnings),
  `layers`, `bom_source`, `lcsc_field`, `without_lcsc` (parts without an
  LCSC part number; `without_lcsc_lines`, their BOM lines), `zip`, `skipped`, `findings`, and `files`: every file
  with its `kind`, `bytes` and `sha256`. Since 2026-11-06 (trial 7) also
  `stackup` (Board Setup's Physical Stackup as the board file gives it,
  which the Gerber job file carries too: `thickness_mm`, `copper_finish`
  and per layer `name`, `kind` (`copper`, `prepreg`, `core`, the masks),
  `thickness_mm`, `material`, `epsilon_r`, `loss_tangent`; absent where the
  board states none), `controlled_impedance` (absent where nothing asks
  for it: the classes that look like a controlled impedance, a class with
  widths per layer from the custom rules or one narrower than Default, as
  rows of `describe-kicad-board`'s `classes`, the custom rules' track
  geometry on some layers only with an `opt` or a `max` (a `min` alone is
  the process's floor, such as 2 oz outer copper's: until 2026-11-08 the
  4in1-mini's release asked for impedance control on Default for it),
  `rules` (`track_width`, `diff_pair_gap`,
  `diff_pair_width` with their bounds, `layers` and `condition`), and the
  `action`, which the verdict's next actions repeat: order impedance
  control on this stackup for those classes; Sisu_SSE-9's release names
  RF_ANT, RF_ANT_INT and USB and its 3313 prepreg 0.0918 mm under the
  outer layers), and `polarised_parts` (the polarised parts on the
  placement file, by reference prefix `D`, `LED`, `LD`, `CR`, `ZD`, `Q`,
  `U`, `IC` or by footprint name: a diode's `D_`, an LED's `LED_`, a
  polarised capacitor's `CP_`, electrolytic or tantalum; the verdict's
  rotation line names them, the first 20 with the rest counted: MIDAS-MK2's
  40). A stackup with copper heavier than 1 oz gets a next action naming
  the layers and their weight (`F.Cu, B.Cu (0.07 mm, 2 oz)`: a fab makes
  1 oz unless the order says otherwise; since 2026-11-08, trial 8's
  4in1-mini). `verdict.json` adds
  `fab_release` (released, findings allowed, files, zip, manifest).
- **`--profile jlcpcb`**: the layers of JLCPCB's KiCad guide (both pastes
  and silkscreens always), and beside kicad-cli's files
  `assembly/<board-id>-bom-jlcpcb.csv` (`Comment,Designator,Footprint,JLCPCB
  Part #`, every designator listed; with a schematic: without one, a
  layout's result, `<board-id>-bom.csv` itself has these columns and no
  second BOM is written) and
  `assembly/<board-id>-cpl-jlcpcb.csv` (`Designator,Mid X,Mid Y,Layer,Rotation`).
  Parts without an LCSC part number get a next action with the BOM lines
  they make (the numbers to find: MIDAS-MK2's 207 parts are 78 lines) and
  where the number goes (since 2026-11-06, trial 7): with a schematic its
  field on the symbols and Update PCB from Schematic; without one a
  footprint property `LCSC` (or `LCSC Part`, `LCSC Part #`, `JLCPCB Part
  #`, `JLC`, `LCSC#`) on the board's footprints, which the board's BOM
  reads (`without_lcsc_lines` in fab-manifest.json).
  KiCad's Gerber names (Protel extensions) are the ones JLCPCB's guide
  uses. JLCPCB's rotation offsets per package are not applied: check
  the polarised parts the rotation line names in their placement preview.
  A board with `controlled_impedance` needs impedance control on the
  order, with the JLCPCB stackup whose dielectrics match the manifest's
  `stackup` (the files are the same: MIDAS-MK2's release is 23 files).

Exit 0 released, 1 refused, 2 usage or input errors (an output directory
that exists, a layer the board does not define, an unknown profile), 4 a
kicad-cli export that failed. On the walkthrough's keyboard (urchin, 80
parts, laid out at `"total_seconds": 30`) the release takes 8 s: 24
files, ERC 42 errors (the schematic's own, not blocking).

The older

```sh
pcb-maker export-kicad-fab <out-dir>/result <board-id> <fab-dir>
```

writes the fab files without the DRC gate: `<board-id>-gerbers.zip` (copper, mask, paste,
silkscreen, outline, drill files; zones filled as KiCad fills them),
`<board-id>-bom.csv` and `<board-id>-cpl.csv` in JLCPCB's assembly format,
and `fab-export.json` with the estimated price at common fabs. BOM lines
take the LCSC part number from a footprint property (`LCSC`, `LCSC Part`,
`JLCPCB Part #`); parts without one are listed in `without_lcsc`. Check the
rotations of polarised parts in the fab's preview: library orientations
differ between KiCad and assemblers.

## Check a board with KiCad

```sh
pcb-maker drc-kicad-board <project-dir> <board-id> <report.json> [--no-refill] [--input <project-dir>] [--stitch-zone-island <out-dir> | --save-filled <out-dir> | --localize-zone-widths]
```

runs KiCad's DRC on the board and its rules (`.kicad_pro`, `.kicad_dru`;
no schematic parity), zones refilled unless `--no-refill`, through the DRC
cache (section 4), and writes KiCad's report (every severity) to
`<report.json>`. One line on stdout counts errors by type, unconnected
items, warnings and cosmetic findings, and the cache's calls and hits.
Use `--no-refill` to check the copper actually saved in the PCB.
For layouts, `fill_publication` in the verdict records whether the
verified fill was saved. An unfilled board can show extra opens in a
stored-copper check. One schematic trial had 24 opens as stored and
zero after refill. To check schematic parity as well, run
`kicad-cli pcb drc --refill-zones --schematic-parity` on the PCB in
the directory containing its matching schematic and project settings.
Exit 0 when no error counts (warnings, excluded and cosmetic findings do
not, nor a footprint's own findings and the board's own edge clearances,
mask bridges and courtyards with no track or via in them: section 4, "The
input's own findings"; `--input <project-dir>`: the input is that
project's board, and only the parts still where it has them, and its zones
and graphics, are the input's) and no item is unconnected, 1 otherwise, 2 for a missing project or
board, 4 when KiCad fails. After the count, `next:` lines in a verdict's
order (since 2026-11-04): the project's rules the router does not keep
where the check fails, then, last, the footprints' own findings with the
custom rule that accepts them and the board's own with theirs, each
`next: nothing to do (the input's): ...`, or `next: review the input footprint: ...` for a plated hole with zero annular width (until then a `footprint:` and
an `input:` line ahead of the rest). The benchmark runners (`run.py`'s designer
check, `freerouting.py`) use it when the binary has it.

Two findings give specific guidance when reviewing an imported layout.

- A solder-mask bridge between a track or via marked `<no net>` and an
  unassigned pad identifies the affected footprint. Check its schematic pin
  and the nearby routing before assigning a net or removing copper.
- For a bridge at a footprint's mask graphic, inspect the listed opening
  and copper on the same board face. If KiCad names a zone on that face,
  or a foreign-net zone outline covers the marker, check whether the
  refilled copper participates before editing the zone.
- An isolated copper zone marked `<no net>` identifies the affected layer.
  Use separate report paths for a default check and a `--no-refill` check.
  The first measures regenerated copper. The second checks the fill saved
  in the board file. Consult the design brief before assigning the plane net.

**Locate a narrow connection within a zone.** When a `connection_width`
error names only a zone, KiCad gives the zone's anchor as its position.
Use `--localize-zone-widths` on its own with a report outside the project.
The ordinary refilled DRC report gains a `zone_width_localization` section
with the finding, zone UUID, net, layer and measured chord and midpoint in
millimetres where independently witnessed. The report identifies whether the
geometry came from the saved board or a private refill. A bounded geometry
search uses the KiCad Python bindings. KiCad must reproduce the complete
original width finding set with temporary witnesses in a private board.
The source project stays unchanged and the command retains the ordinary DRC
exit status. A diagnostic reason accompanies any unconfirmed coordinates.

**Stitch a zone island.** When KiCad reports one open between two islands
of the same zone, `--stitch-zone-island <out-dir>` tries a through via
where filled copper overlaps on two layers. It uses the net's resolved via
diameter and drill and checks up to six sites with KiCad after refilling.
A layout or route verdict with a verified zone identity provides its net,
zone UUID and a command as `zone_island_recovery.retry_argv`. The command
tests the returned board with fresh refill and DRC.
A repair is kept only if the open closes with no new noncosmetic findings.
The new directory must be outside the input project. It receives a copy
of the project including its schematic, child sheets and local libraries,
the board with one additional via, and fresh `drc.json` and
`zone-island-repair.json` reports. Other pcb-maker run reports are dropped.
Use a sibling directory when the project refers to external libraries by
relative path. The requested `<report.json>` still holds the input's DRC.
Stdout reports the attempts with the net, site, layers, via dimensions, remaining opens and new
findings, even when the repair is rejected and no new directory is written.
The flag is optional and requires zone refill.

For one open within a single-layer zone, the DRC, route and layout
reports name the zone and its layer. Inspect the filled polygons on
that layer for a gap. A higher-priority pour, a keepout, or clearance
around tracks can divide a continuous zone outline into separate pieces.
KiCad can list the same UUID and position at both ends of this open.
Refill and verify the stored board after editing its copper.

**Save the checked fill.** Use `--save-filled` to hand off a project with
KiCad's regenerated zone copper saved in its board file.

```sh
pcb-maker drc-kicad-board run/result board_id build/refilled.json --save-filled build/filled-project
```

Use a fresh output directory and a separate report path outside the source
project. Use a sibling directory if the project refers to external files by
relative path. The copy keeps the project rules, schematic hierarchy and
local libraries. KiCad checks the stored source and refills a private copy.
When zone identities match, pcb-maker keeps the source footprint, track and
zone outline forms and copies KiCad's generated fill. This can include
pad/via teardrops and derived layer connections. It then opens the new file
and repeats DRC without refill. If source-form preservation cannot pass
those same checks, the handoff tries KiCad's own saved board. If a matching
schematic exists, each check includes schematic parity. These checks use
fresh, raw native DRC reports.

KiCad can choose different ratsnest endpoints for the same disconnected
copper. If that is the only difference in the uncapped reports, pcb-maker
can verify the saved fill on the authored board by proving that every
named pad, track, arc and via belongs to the same connected component
with in-memory refill and after reopening the saved file. This additional
check requires the optional KiCad `pcbnew` Python bindings for the same
version as `kicad-cli`. It applies to layout publication only if the
remaining modeled budget covers the two additional native checks. If
footprint library warnings reach a report cap, install the project's
libraries and retry with `PCB_MAKER_KICAD_USER_LIBS=1`.

When changed open witnesses name zone or teardrop copper, standalone
`--save-filled` can also compare the exact native filled polygon shapes
and their attachments using the captured refill and reopened board. The
proof requires an unambiguously matched set of supported, anchored
islands and the same open counts by net. If the proof cannot verify the
copy, inspect the reported witnesses and their pad or via attachments
in a private KiCad copy before retrying `--save-filled`.

The requested report captures the refill. The output contains `drc.json`
for the saved board, `drc-before-fill.json` for the source as stored, and
`fill-handoff.json` with both board hashes, counts, inherited findings and
rule types that reached a native report limit. `preserved_source_forms`
records which board form was saved. `connectivity_checks`
records whether the additional native component checks ran. When present,
`connectivity_proof` records the number of copper objects, components and
nets checked. The manifest also records elapsed checking time in `wall_seconds`.
When saving changes which footprint-library warnings fit into a capped
report, the handoff reopens the saved PCB and refills it in memory.
`drc-refilled-as-published.json` records that extra check. Every raw
finding must match the check of the saved board.
For a measured connection-width or uncoupled-length finding whose reported
track changes on reopen, the standalone handoff can retry with KiCad's
track order. It moves complete original copper forms, preserves the same
native rule measurements and affected nets, and verifies the full native
connectivity of the saved and freshly refilled board against the original
refill. `reordered_copper_forms` counts the moved forms. The width and
uncoupled-length findings stay visible in the saved board's report.
Exit 0 means the copy has zero native errors, opens and parity findings.
Exit 1 also delivers the verified copy and reports the remaining issues.
Exit 2 means invalid input. Exit 4 means saving or verification failed.
Use `--save-filled` without the other DRC options.

## Compare explicit copper pour policies

Run `compare-kicad-pour-policies` to compare several requested pours on
one layout brief. The policy file is a JSON array. Omit `add_pours` from
an entry to keep the shared router settings. An entry with `add_pours`
replaces that list and uses the ordinary layout's format for the requests.

```sh
cat > policies.json <<'EOF'
[
  {"name":"as supplied"},
  {"name":"ground on both sides","add_pours":[{"net":"GND","layers":["F.Cu","B.Cu"]}]}
]
EOF
pcb-maker compare-kicad-pour-policies project board comparison policies.json auto project/layout.json
```

Every policy gets the layout file's full `total_seconds` budget, so two
policies request twice that work. The comparison holds the work rates,
project, design rules, placement constraints and starting seed fixed.
The ordinary layout can choose different movable placements as it routes
each policy. The report includes initial and final placement digests.

`comparison/pour-policies.json` gives per-policy and total planned work,
the work reported by each run, raw and counted opens, raw errors, counted
new errors and inherited findings. On a panel, the raw open count includes
links between separate sub-boards that the layout verdict sets aside. The
comparison includes incomplete runs and their logs.

For a pad-relative zone, `bounded_regions` reports the requested rectangles
and the other nets' track length through them. A `keep_whole` suggestion for
that zone requires a part's proposed region to fit inside one rectangle and
to leave other nets' pads clear on the named layer. Apply the suggestion to
the existing bounded request on that layer.

A policy qualifies when the ordinary native layout verdict passes, all
placement constraints hold, and a schematic, where supplied, was checked
for parity. If a policy keeps the shared router settings, its parity
findings are the baseline and a candidate adding any of them is ineligible.
The comparison prefers fewer parity findings, then shorter physical track
length, then fewer vias. Inherited DRC findings can remain. The chosen
project, board and verdict are in `selected`. Exit 1 means `selected` is
`null` because no policy qualified.

## Retry an open plane pad with protected escapes

When a dense footprint has a surface pad still open to a copper plane,
one local escape can take room needed by a nearby pad on another plane.
An opt-in layout can reserve part of its modeled budget to reroute the
same placement with both short plane escapes held:

```sh
pcb-maker layout-kicad-board project board layout auto layout.json --plane-escape-budget 150
```

For example, with `"total_seconds": 390` in `layout.json`, ordinary
layout uses its usual 240-second budget and the retry has up to 150
modeled seconds. The retry is eligible when the selected native result
has one open and clean schematic parity. It needs an identifiable
surface pad on a plane net and a nearby pad on a different plane net
with an exact short connection to a through via in the first route.
The staged search holds both escapes while routing the rest of the board.
A fresh all-track KiCad check must find zero opens and no new errors,
warnings or exclusions. The retry never moves footprints or edits rules.

`plane_escape` in `board-layout.json` and `verdict.json` records the
outcome, reserved and measured work, both terminal paths, native check
counts and raw errors. The physical statistics of the selected layout
include the held tracks and vias. If the retry is skipped or rejected,
`result/` retains the ordinary layout. A successful layout uses the
standard verified zone-fill publication if enabled and within budget.
Inherited footprint errors remain visible in the native reports.

To retry an existing layout without rerunning placement:

```sh
cat > retry.json <<'JSON'
{"ladder_budget_seconds": 150}
JSON
pcb-maker repair-kicad-opens layout/result board --plane-escape repaired layout/placed retry.json
pcb-maker drc-kicad-board repaired board repaired-filled-drc.json --save-filled repaired-filled
```

The standalone repair assumes matching project and original copper-free
placement. It writes `plane-escape.json` with native comparisons after
refill. The separate handoff saves and reopens the zone copper so KiCad
can verify stored connectivity.

## Verify an additive repair for open connections

After selecting a partial layout, `layout-kicad-board` uses any remaining
modeled work to try additive local repairs on up to four native open nets,
the shortest native witness first. The final ladder and the neck repair
leave work for it. The stage reserves work for its native checks and for
requested zone-fill publication. `route-kicad-board` runs the same repair
after its ladder (`open_repair` in `board-router.json` and `verdict.json`).
In both commands the result then describes the board written: a net on
which KiCad finds no unconnected item after the repair is `routed`, the
repair's vias and track length are in the net's and the result's totals,
and `nets[].open_repair` keeps what the router had left (so
`routed_connections`, `unconnected_terminals`, the verdict's
`open_connections` and a benchmark row agree with KiCad's count for those
nets). A net the repair improved without closing keeps the router's count
and `open` from before; its `open_repair.kicad_unconnected_pads` are the
pads KiCad's remaining items name. Library-only
findings (`lib_footprint_issues`, `lib_footprint_mismatch`, `text_height`,
`text_thickness`) do not count as a capped report that blocks the repair,
and the repair is not refused for adding them. It uses the layout's routing rules. On two-layer boards it
can try a pair inferred only from exact complementary net names when
coupling is off and neither member has an applicable coupling, length
or skew rule. The local report gives the companion in
`name_inferred_pair_partner`. The main router handles coupled pairs and
length-constrained nets. The native trial checks both nets and the rest
of the board.
Verified additions leave the selected placement and its
existing copper intact. `local_open_repair` in `verdict.json` gives the
outcome, the `nets` that got copper and the `added_copper` objects where
it is `accepted`, work, report path and a standalone retry command as an
argument array when standalone repair could do more. `same_board_search_complete`
means a completed search covered every native open on the selected board
and would repeat with the standalone rules. Check the native witnesses
or the suggested alternate placement in `next_actions` for a different
approach. The search metrics (`searches`, `expansions`, `iterations`,
`attempts`) are the router's from before the repair. The result's
`model_precedes_additive_repair` is set only where a repair's copper is
not in the nets and their totals: a terminal-launch retry's local trial,
the same-hatch width repair, or an open repair whose `reason` says
`nets not accounted` (use KiCad's check and `saved_copper` for the board
then).

On a routed board, search from the copper already connected to the endpoints
of KiCad's opens.

```sh
pcb-maker repair-kicad-opens routed board --local repaired
```

The search starts at a 0.025 mm pitch and can land on reachable tracks
and vias elsewhere on the same connected component. Larger windows use
the finest pitch up to 0.127 mm that fits 4.5 million copper-layer cells.
It holds existing copper and footprint poses fixed and keeps the original
project and rules. It tries up to four nets with identifiable native opens.
Repeat `--net "/NET_NAME"` after the output directory to restrict the
search to exact net names. Names copied from KiCad's DRC report work
when they identify one board net. `unmatched_requested_nets` lists
requests without an unambiguous native open, even when other requested
nets can be tried. `local_search` in `open-repair.json`
records the window, pitch, modeled components and search work. When a
component has no legal launch, `blocked_terminals` records the affected
pads, layers and nearby obstacles identified by the router.
When local search cannot reach a single native open, it can try one
straight bridge to a nearby same-net surface pad of the same footprint.
The pad centres must be within 1.5 mm on the same copper layer. The
bridge respects the project's track-width floor, and each end lands
inside its pad copper. Coupled and length-constrained nets stay with
their own routing. KiCad must prove the bridge joins the two components
behind the open without adding findings or changing other components.
With zones, the repair also checks stored copper. If KiCad changes
the remaining stored ratsnest witnesses, the repair verifies their
connectivity partitions. `local_search.same_footprint_pad_bridge`
records the pad UUIDs, geometry and stored verification.

For a poured net with multiple opens, KiCad's filled connectivity can
identify witness endpoints already in one electrical component.
`refilled_components` reports the number of distinct native groups when
local repair used them to select fewer targets. KiCad still checks each
proposed repair.
When KiCad names two zones without reporting a physical endpoint, a
named local repair can report `zone_open_pad` from its baseline native
copper partition. Use the pad UUID, position and layer to inspect the
open, especially when one footprint repeats its pad number.
When an explicitly named net's native open ends at a generated teardrop,
local repair can use a same-net track inside the uniquely matched refill
polygon. `teardrop_contacts` gives the layer, witness location, and kept
track UUID and contact point. An accepted repair must improve the stored
and refilled native checks. This search applies to single boards and
leaves coupled or length-constrained nets to their dedicated routing.
On a two-layer board, a named local repair that finds no route can also
try one smaller through-via size listed in Board Setup. It keeps the
board's diameter, drill and annular ring minimums and leaves the default
net-class size intact. `preset_via_mm` records the size tried. KiCad
checks the candidate as stored and after refill before it is accepted.
On stacks with more than two copper layers, the search excludes dedicated
foreign plane layers from new track paths. It reports them as
`reserved_plane_layers` and applies the board's via span and layer rules.
Standalone `--local` can find an isolated surface pad with a native open
near a same-net solid plane. Nets with a candidate that reaches actual
refilled copper get priority within the existing four-net limit. Use
`--net` to restrict the search to specific rails if needed. This recovery
is available through the layout verdict's `local_open_repair` retry. If
the ordinary search finds no route, the plane escape uses the active
track width and the normal through-via size or a listed smaller preset.
The ring stays
off the pad and the drill stays outside its mask opening. The report's
`plane_landing` entry gives the pad UUID, plane UUID and layer, route
geometry, and number of filled islands checked. KiCad's Python bindings
must prove the pad joins the named plane through the added copper,
preserves every other component and
original island's attachments, and adds no native findings. This is a
refilled result. Use the printed `--save-filled` command to deliver a
reopened, verified copy.
On a single board, standalone local repair can try one recognized pair
with two pads and a native open on each conductor, with neither
conductor already routed. If using `--net`, select both names. The two
conductors are routed and checked together with the original rules.
`paired_search` names both members and their pads. It records the
router's geometry measurements or the reason the attempt declined.
An unmatched end can include its spacing and the planner's ordinary span.
The repair requires both opens to close with no new findings in KiCad's
refilled and stored-board checks. On a panel, the command reports opens
per physical piece and checks that all copper
of an accepted trial stays on the piece whose open count decreased. If
KiCad changes the endpoint it reports across the gap, the repair can
accept a newly added track at the same copper contact with the original
net, layer and peer. `panel_witness_replacements` records the track and
original endpoint UUIDs. For a plane escape, the component and island
proof compares raw opens by net, including cross-piece items.
KiCad refills zones for the checks when the board contains zones. On a single
board, if the trial closes an open and the only new findings are narrow
joints between new track tips and existing straight tracks, the repair can
retry once with those tips on the existing centerlines. The
`contact_refinement` report lists both native checks and the adjusted
contact coordinates. The retry must pass the same native checks as the
original trial. In both local and alternate-board repair,
`drc_refilled_zones` records whether the
open counts include a refill. After adding copper in that mode, the CLI
reads the repaired project's PCB. With fillable copper zones, it prints
a `drc-kicad-board --save-filled` command to save those fills in a new
project and verify its connectivity after reopening it. If the board
has no fillable copper zones, the command uses `--no-refill` to check
the repaired PCB as stored. Rule areas alone require no fill handoff.
For a net named with `--net` that has more than eight disconnected
components, the local command tries up to four short branches from the
native ratsnest within one 20-second search window. It submits at most one
branch per net for native verification. The report lists the searched
endpoint pairs in `local_search.attempted_branches`.
If the project's native DRC disables copper-to-edge clearance,
`native_edge_clearance_ignored` records that gap in verification.

For a cold layout with a few tightly spaced open pads, reserve work for a
single retry on the chosen placement.

```sh
pcb-maker layout-kicad-board project board layout auto layout.json --terminal-launch-budget 190
```

The flag reserves 190 modeled seconds within the layout's existing
`total_seconds`. The ordinary layout uses the rest. The retry looks for
one or two pads that KiCad names in its open findings and the router
identifies as blocked, on one compact two-row surface-mount footprint.
KiCad checks the provisional outward tracks against the same placement
before rerouting. The reroute has up to eight native checks and can
finish with bounded local repair. The work reserve also covers the
comparison checks and saved-fill publication. It keeps the
first board unless KiCad confirms that all opens closed with no new
findings or schematic parity issues. The verdict's `terminal_launch_retry`
records the pads, geometry, modeled work and outcome. Its `local_trials`
list gives the net, trial open counts, decision and report path relative
to the layout directory. The retry's top-level counts describe the board
the layout selected. Wall time depends on the machine and its native checks.

To select the pads yourself, add `"terminal_launch_pads": ["U1:7", "U1:8"]`
to the layout JSON and use the same reservation flag. Choose one or two
native open witnesses on the same surface-mount footprint of the selected
placement. This retry starts from a placement without authored tracks or
vias. It proposes short outward traces on rectangular, rounded rectangular
or oval pads with a clear tip, then checks those traces under the original
KiCad project rules. The detailed router must use each tip and close every
remaining open. The retry replaces the first board only after the native
checks reproduce those results on the saved PCB without new rule findings
or schematic parity issues. `terminal_launch_retry.requested_pads` and its
decision explain which selection was tried.
On a suitable ordinary layout, `terminal_launch_advice` gives the JSON
patch, the reservation flag and candidate geometry. Its minimum reserve
is the modeled work needed to start the trial. A full reroute can need
more. KiCad verifies the proposed tracks in the actual retry.

For an open between two nearby surface pads on opposite faces, give the
inspector the original placed project with its `board-placer.json`.

```sh
pcb-maker inspect-routing-run routed --terminal TP1:1 --escape-placement placed
```

`placement_escape` checks the exact native pad pair against the original
placement. It screens short straight traces with one through via, using
KiCad's pad shapes and the project's default or resolved class sizes.
If a short escape already fits in the sample, it leaves the poses alone.
If one movable front footprint accounts for the obstruction, the probe
tests shifts of up to 1 mm on that face and reports a witness and a
`constraints_patch` when a shift clears the sampled geometry. The patch
holds every other pose. Nearby footprints without a native courtyard
are listed in the result. Keep the original face declarations and
mechanical constraints, run ordinary placement with the patch, then
reroute and compare the native findings on the saved result.

If one local open remains in a channel too narrow for its via, the retry
can detour one short generated track between its original endpoints.
It compares the native finding identities and copper component membership
before retrying the net. `track_detour` records the track and the proposed
coordinates when this step runs.

For an open surface pad fully covered by an authored track-and-via
keepout, give the routed project and the exact pad number.

```sh
pcb-maker repair-kicad-opens routed board --rule-area repaired --pad C90:2
```

The command confirms the pad's UUID in the native open report. It searches
within 3 mm of the original pose and can reseat up to two movable neighbors.
It routes only nets of the moved footprints and preserves all existing
tracks and vias. The staged repair must reduce stored and refilled native
opens, close the named pad, and preserve native DRC and schematic parity.
When KiCad reports a different witness for a remaining open, its native
connectivity partition must show that no existing component split.
Exit 0 saves the verified filled PCB. Exit 1 leaves the original board in
the output project. `rule-area-open-repair.json` lists the moves, work and
checks. The source project is read only.

When an authored rule area prohibits tracks and allows zone fill at an open
surface-mount pad, ask KiCad to try a local pour on the pad's copper layer.
`inspect-routing-run` lists these pads under `nets[].local_pour_retries`.

```sh
pcb-maker repair-kicad-opens routed board --local-pour repaired --pad PNT1:MP
```

Both native reports must identify the same physical open pad. The command
tries one rectangle around that pad's copper and the nearest retained
through via in a different component of the same net, with a 0.30 mm
margin. The trial keeps existing copper and rule-area geometry. It publishes
the filled board only if KiCad confirms the selected pad-to-via join and
one fewer open in both stored and refilled checks, with the same other
findings and all other electrical components preserved. Exit 1 leaves the
original PCB in the copied project. `local-pour-repair.json` records the
selected pad and via UUIDs, layer, bounds, and native checks.

When retained copper blocks a route, prepare a candidate PCB with the same
footprint placements and keep every original track and via UUID. Submit the
candidate with the native open's exact net name.

```sh
pcb-maker repair-kicad-opens routed board --relocate repaired candidate.kicad_pcb --net /SIGNAL
```

This checks one proposed edit with up to eight relocated unlocked straight
tracks or through vias, at most 0.25 mm from their original geometry, and
up to 128 added copper items. Existing track widths, layers, via sizes, drills
and net names must match. Preserve each outer segment endpoint. A bend
shared by two moved segments can move with both, and an endpoint attached
to a relocated via can follow that via. New segments on
other nets must stay within the same limit of a relocated original segment.
The command takes only the copper edit from the candidate and checks it under
the source project's board, footprint and design rules.

KiCad must confirm fewer opens on the requested net in the stored and
refilled board. Native findings and schematic parity must match, and every
other original electrical component must retain its membership. The native
checks have a 150-second wall limit and require matching KiCad Python
bindings. The report `relocated-open-repair.json` records the candidate
UUIDs, counts, the native connectivity proofs and the decision. Exit 0 saves
the verified fill. Exit 1 publishes a byte-identical copy of the original PCB.

For a two-layer board with a listed through-via preset, the same command can
search small bends in the router's generated tracks. Supply the original
placed project to identify which copper must be kept.

```sh
pcb-maker repair-kicad-opens routed board --relocate repaired --generated-from placed --net /SIGNAL
```

The search tries small shifts of neighboring three-track chains and a
short branch on the requested net. Tracks in the placed input stay fixed.
The same stored and refilled DRC and connectivity proof gate
publication. `automatic_search` records the variants and candidates.

On multilayer boards this command can also try moving one router-generated
filled and capped via up to 2 mm inside its original large land to open an
escape on the other face. The full ring stays on the land. The via
dimensions, fabrication flags, original tracks and footprint poses stay
fixed. A new trace reconnects the via to its original junction.
`automatic_search.filled_pad_via` records the pad, via, old and
proposed positions, and the retained junction. Both native checks must
verify the repair. Any resolved centering warnings at that via are recorded
in stored and refilled order under `resolved_via_centering_warnings`.
If KiCad selects different objects for an existing connection-width warning,
the tool can replay that exact warning on unchanged source copper.
`reproduced_width_warning` identifies which check needed this proof.
An ordinary layout verdict and its `inspect-routing-run` report can list
`generated_pad_via_retries` for an exact native open beside such a via.
Each entry gives both pad UUIDs, the via and junction geometry, and a
`retry_argv` command using that layout's original placed project.
A normal layout with verified stored fill and enough remaining work tries
one eligible generated pad-via repair before writing its verdict. The
layout charges the local routing work, native checks and connectivity
proofs to its budget. See `generated_pad_via_recovery` for the outcome and
`saved_copper` for the published board's geometry. The retry remains
available when the layout runs short of work.
If a native finding type that copper can change hits KiCad's report cap,
the repair leaves the board alone and identifies the evidence limit. The
library and text findings (`lib_footprint_issues`, `lib_footprint_mismatch`,
`text_height`, `text_thickness`) at their cap do not stop it, and they are
not compared between its checks.

Automatic layout repair tries locally short native opens first and
submits at most one ordinary trial to native DRC. On a single board it
can then try one branch of a larger open net. It routes against any
copper just accepted in the ordinary trial. The branch needs remaining
work for its search, native DRC and two native component checks. Layout
also reserves additional checks to verify the saved fill when needed.
When the reports differ only in which
endpoints KiCad chose to show an open, two further native connectivity
checks can certify the repair within the remaining budget. The selected
net may merge connected components. Every other net must keep the same
copper components. `trials[].connectivity_proof` records the merge and
the verified copper counts in `open-repair.json`. The native DRC and
parity findings must still agree for this check to run.

`verdict.json.via_cost_retry` can propose a controlled comparison on
a multilayer board with many verified opens and existing routed vias.
Apply `router_config_patch` to a copy of the current router JSON, keeping
its other keys, and use the same placement and project rules. An `auto`
run can use the patch as its router JSON, including any effective seed
count. This suggestion is omitted when the current router settings
specify a via cost. Compare saved and refilled opens, findings and via
count before choosing the resulting route.

Use `--compare-incumbent` on `route-kicad-board` for a fixed-placement
trial against a routed incumbent. Start with the layout's `placed/`
project and write the alternate route to a fresh directory. Supply a
router JSON with an explicit `ladder_budget_seconds` and the strategy
changes you want to test.

```sh
pcb-maker route-kicad-board run/placed board-id alternate alternate-router.json \
  --compare-incumbent run/result
```

The comparison checks the complete placement, board design, locked and
grouped copper, project rules and schematic inputs. Four private KiCad
checks use the work left in the budget to compare the two saved boards
and both native refills. `verdict.json.fixed_route_comparison` lists
new findings and each open net's before and after counts. A result can
improve the total while opening a previously connected net. Review each
tradeoff before choosing a route. The ordinary route exit code still
describes the candidate, and both PCB files stay available. With too
little work to finish the native comparison, its verdict is
`inconclusive` and reports the required work.

When only the saved pad/via teardrops differ, the refilled check also
saves a private copy and verifies that KiCad regenerates the exact
stored shapes. The verdict records the count for each verified board in
`verified_regenerated_teardrops`. Authored zones and locked or grouped
copper remain part of the fixed design comparison.

Local repair can verify a capped `silk_overlap` warning on KiCad 10.0.8
when the project uses implicit silkscreen rules and the reported pairs
resolve to unchanged artwork on the same silkscreen face. The native
reports retain every warning they emitted. `verified_invariant_native_caps`
records this proof in the accepted repair report. Other capped categories,
including `silk_over_copper`, still prevent this acceptance.

If a routed board has a few native opens, try a different router pitch or
seed while keeping the copper already on the board. For example:

```sh
cat > retry.json <<'EOF'
{"existing_tracks":"keep","seeds":1,"ladder_budget_seconds":40,"tuning":{"pitches":[0.075]}}
EOF
pcb-maker route-kicad-board routed board alternate retry.json
pcb-maker repair-kicad-opens routed board alternate/board.kicad_pcb repaired
```

You can use the alternate board if its reroute wrote the board and kept the
placement. The command selects up to four nets with native opens and
matching additions in the alternate file. To target a particular open,
append `--net '+3V3'` after the output directory. Repeat `--net` for
other names exactly as KiCad reports them. Both modes make a copy of
`routed` in a new directory outside the source project and accept up to 256
copper objects per net. The checks use the source schematic, project and
custom rules.
KiCad must report fewer opens and no new findings at any severity, including
schematic parity when available. Native reports with unstable item IDs or
truncated findings can prevent acceptance.

When missing footprint libraries cause KiCad to hit its reporting limit,
`open-repair.json` lists the library nicknames and the number of visible
warnings for each in `reported_missing_footprint_libraries`. Restore the
source project's `fp-lib-table` beside the PCB and copy the libraries to
the paths in that table. Set `PCB_MAKER_KICAD_USER_LIBS=1` if the
project depends on installed user libraries. Retry the same repair in a
fresh output directory.

`unstable_native_copper_ids` means the trial's unmatched findings differ
only in native IDs at legacy copper coordinates. The `native_identity`
field reports the trial's open count, the number of unmatched findings
and how many source tracks, arcs or vias lack a persistent ID. On a fresh
copy of the entire source project, use KiCad to persist the IDs before
retrying the same candidate:

```sh
cp -a routed upgraded-routed
kicad-cli pcb upgrade --force upgraded-routed/board.kicad_pcb
pcb-maker repair-kicad-opens upgraded-routed board alternate/board.kicad_pcb repaired-upgraded
```

The retry compares the upgraded copy with a fresh native report and
applies the same strict acceptance check.
At least 199 findings of most DRC types or 499 opens can indicate truncated
output and prevent acceptance. KiCad 10.0 also uses 499 for clearance
findings. A local trial on a capped report can collect evidence in the
output, and the published board retains its original copper.
`capped_schematic_parity_types` identifies
possibly truncated inherited parity. Repairs still compare all reported
parity identities and keep the source schematic and pad assignments.

Exit 0 means an improvement was verified. Exit 1 means that the copy retains
the original board. Exit 2 indicates invalid input and exit 4 indicates a
failed check or write. Read `open-repair.json` for identified nets, attempts
and their native reports, and `drc.json` for the result in the output. An
improvement can leave inherited findings or other opens.

## Shorten a routed connection between two named pads

Use this command when placement is settled and two pads on the same net
need a shorter copper path. For example, to shorten the connection from
a capacitor's supply pad to a module pin, run

```sh
pcb-maker shorten-kicad-pad-path layout/result board shortened --pads C3:1 U5:16
```

Use the board's file stem for `board` and a fresh directory for `shortened`.
The command works on a private copy. It can replace a leaf of a routed
net with a direct track while retaining the junction that serves the
rest of the net. It checks the original and proposed copper with KiCad
both as saved and after refilling zones.

Read `shortened/pad-path.json` for the outcome. Exit 0 publishes the
shortened path. Compare its estimated pad-to-pad span with the change in
total track length. Preserving a branch can shorten the selected path
while increasing total copper. Exit 1 keeps the original copper in the
copy and records why the command stopped. When KiCad rejects a trial,
inspect its entry in `rejected` for the layer, affected rule types, and
the path to the native report that identifies the items.

Native verification refusals name the stored or refilled check and link
to its report when the output retains one. If footprint
library warnings reach KiCad's report limit, restore missing footprint
libraries or retry with `PCB_MAKER_KICAD_USER_LIBS=1` when the correct
libraries are already configured in KiCad. If the project disables a
required copper check, the reason names the setting to enable before
retrying. These checks also apply to parallel feeds.

For a capacitor-to-supply detour through other branches or layers, add
`--decoupling-shortcut` after the two pad names. KiCad's current schematic
must identify the two-pin capacitor, its grounded return, and the
selected power-input pin on the same supply net. The search retains the
old copper and tries a direct local route with at most one via. It uses
the KiCad Python bindings to measure the planar trace span outside the
pad and via copper. Read `path-before.json` and `path-trial.json` in the
reported native checks directory. If a new via intersects stored fill,
the command can save regenerated fill after both native modes pass.

## Add a separate feed between connected pads

Use this command when a completed board needs an additional current path
between two known pads. Select the endpoints, copper layer and trace width
from the electrical brief.

```sh
pcb-maker add-kicad-parallel-feed routed board reinforced \
  --pads J1:1 U5:22 --layer B.Cu --width-mm 0.513593
```

Supply the current schematic and a fresh output directory. Both uniquely
named pads must belong to the same net and expose copper on the chosen
layer. The search adds straight tracks of the requested width on that
layer, with electrical contact to existing copper confined to the two
pads. It keeps the existing tracks, vias, footprints, rules and zones in
the copy. Selected nets with coupling or length requirements, enabled
teardrops on that layer, and selected layers with same-net or unassigned
copper zones are outside this command's scope.

Exit 0 publishes the feed after stored and refilled KiCad checks confirm
zero opens, unchanged schematic parity and no new DRC findings. Exit 1
publishes a copy with the original PCB. Read `parallel-feed.json` for the
selected net, exact width and layer, added track length, search outcome,
and native evidence. The router gets one bounded search with a 20-second
wall limit and at most 128 segments per candidate. The native checks have
a separate 60-second wall limit.

If the only new stored findings are clearance violations between the new
tracks and existing zone fill, the command tries the native refill. When
that check passes, it copies the verified fill payload to the private
board and reopens it for another stored check. `fill_publication` records
the outcome. The original zone outlines, allocation, and rules stay in
the board. If the refill creates an open, a starved thermal, or another
new finding, the output copy retains the original PCB.

Ask for the load and allowable drop when the electrical objective is
unspecified. Use the board's layer thickness and actual loads to measure
the voltage drop after adding a feed. The path length is a geometry
measurement.

## Widen a narrow join between repair tracks

After a neck repair that wrote `track_ids` in `necks.json`, run
`pcb-maker repair-kicad-necks routed board --widen-supplemental widened`.
The command checks the original again and tries widening pairs of its
recorded repair tracks that KiCad identifies in the same width finding.
It keeps all track centrelines, authored zone settings and design rules.
It publishes a separate project in the fresh output directory outside
the source. The copy keeps the original copper if widening fails to
reduce the width errors or KiCad reports any new finding. Read
`supplemental-neck-repair.json` for the counts and `drc.json` for the
result, including any remaining zone width findings.

### Repair width while preserving hatch settings

Ordinary layout tries the same-hatch repair after its usual neck repair
and fill publication when KiCad names a hatched zone in a connection
width error on a fully connected board. The trial can widen recorded
generated tracks and add local bridges or small solid copper patches.
It keeps the selected board unless the trial removes native width
errors, preserves the original hatch and other authored geometry, and
passes both refilled and stored-board verification without adding a
finding. `unchanged_hatch_width_repair` records the outcome, work and
supporting report.

If the repair did not fit in the layout's remaining work but the original
fill was verified, the report includes `retry_argv`. The standalone
command starts from the verified filled project and writes a new copy.

```sh
pcb-maker repair-kicad-necks routed board --preserve-hatch repaired
```

The standalone repair handles one unsmoothed hatch. Its solid patches can
change the copper area after refill, even though the hatch spacing,
orientation and hole-area fraction stay the same.

The repair publishes changes only when KiCad reports fewer width errors
and identical other findings, opens, and schematic parity in fresh
refilled and reopened checks. Use `unchanged-hatch-width-repair.json`
for the exact widened track IDs, added track and zone IDs, and native
verification. `drc.json` records the refilled result and
`drc-saved-fill.json` records reopened copper. Exit 1 leaves the source
copper intact in the output copy.

### Retain copper at clipped hatch holes

If the project's hatch policy permits a higher minimum hole area,
specify the new fraction for a verified repair on a separate copy.

```sh
pcb-maker repair-kicad-necks routed board --retain-clipped-hatch 0.6 repaired
```

A larger fraction leaves copper where KiCad clipped a hatch hole to
less than that fraction of its full area. The option applies to one
unsmoothed hatched zone named by a KiCad width error. The new fraction
must exceed the zone's current value and be below 1. The check keeps the
zone outline, hatch pitch and orientation, footprints, and design rules.
It may widen recorded repair tracks and add one short bridge at a
remaining narrow connection. A multilayer zone shares its minimum hole
area across its layers, so this option can change copper on each of them.

The repair keeps changes that reduce width errors without adding a native
finding, including opens and schematic parity. Exit 0 requires the saved
PCB to reproduce the repaired findings when reopened without a refill.
The saved PCB carries KiCad's zone fill and generated pad/via teardrops.
Read `hatched-neck-repair.json` for the selected zone, remaining width
errors, `as_refilled` and `as_stored` counts, and elapsed time. An
unimproved copy retains the original copper. If the repair improves
refilled DRC and fill publication fails, exit 1 and the
`fill_publication.retry_argv` describe how to verify a separate saved
fill. `drc.json` reports the PCB as stored and
`drc-refilled-after-hatch-repair.json` records the refill check.

When new width errors name an eligible hatched zone, the verdict reports
its UUID and current minimum hole-area fraction. The suggested repair
command is for designs that permit retaining copper at clipped holes.

## Repair a via-to-track short

```sh
pcb-maker repair-kicad-via-short layout/result board layout/placed repaired
```

Supply the matching pre-route `placed/` directory so the repair can exclude
existing copper. The repair can jog one freshly generated, straight track
away from a different-net through via. It keeps the track width, layer and
endpoints as well as the via and all other board items. Tracks belonging to
differential pairs, length rules or KiCad groups are outside this repair's
scope. Ordinary `layout-kicad-board` can try the same repair automatically
when a native short identifies these items and its remaining budget covers
the checks and any requested fill publication.

The command checks the original and trial both as stored and after refill.
It requires closed refilled connectivity, zero schematic parity findings
and no new native finding. If KiCad changes the witnesses for the same
stored opens, the repair requires identical native copper components after
substituting the jog for the old track. Exit 0 publishes the repair. Exit 1
keeps the original PCB in the new project. Read
`via-detour.json` in the new project for the outcome, native check count and
track geometry. A successful standalone repair moves the old routing and
verification reports to `via-detour-input-evidence/` and writes current
KiCad reports beside the PCB. A normal layout also records the outcome in
`board-layout.json` under `via_detour`.

## Remove a generated dangling track

Run `pcb-maker repair-kicad-dangling routed board repaired` on the output
of a completed route or a layout's `result/` directory. The router records the IDs of
short, generated pad stubs in `board-router.json`. The repair tries
removing at most one such track when a fresh KiCad `track_dangling`
error identifies it. It verifies the trial with a second native check,
including zone refill and schematic parity when applicable. It keeps
the change only when the trial removes a dangling-track error without
introducing a native finding or open. The source board, authored zone
settings, and all other copper are preserved.

The command publishes a separate project. Exit 0 means it verified a
repair. Exit 1 means the copy retains the original copper. Older route
reports without the track IDs yield no eligible repair. Read
`dangling-repair.json` for the track, removed length, native check count,
and elapsed time. The published project's `drc.json` includes any
remaining findings.

For a dangling branch in a layout's own generated copper, add
`--generated-from <layout-dir>/placed` to the same command. Use that
layout's `result/` as the routed project. The placed project must match
the routed design rules and footprint geometry, and any copper supplied
by placement must be intact. Both native modes must name the same sole
dangling track. The repair checks that every surviving copper object's
net and connected group are identical both as stored and after refill.
If KiCad changes which endpoints it shows for existing opens, the
repair also verifies that the open count for each net stays the same.
A changed width-warning witness on untouched copper requires a replay of
the complete finding on the original design.
The command keeps the original copper if these checks fail and gives
the reason in `dangling-repair.json`.
The layout verdict and `inspect-routing-run` publish `generated_dangling_retry`
when KiCad names a generated leaf. It includes the measured track, its UUID,
and `retry_argv` with the exact placed project.

## Which nets stayed open: `inspect-routing-run`

```sh
pcb-maker inspect-routing-run <run-dir> [--net <net>] [--terminal <REF:PAD>] [--reference <project-dir>] [--native-check] [--output <file.json>]
```

reads what a route or layout run saved (board-router.json, the board it
wrote, drc.json; a layout's `result/`) and answers, per net the router
left partial or unreachable (or the one `--net` or `--terminal` names),
which pads are apart and under which rules. Nothing is routed: the result
board is lowered as the router lowers a board (any outline the router
reads) and its copper joined as KiCad joins it without zone fill (about a
second; `--reference` lowers the reference too). `--native-check` runs
KiCad's DRC first (cached, into `native-check-drc.json` beside the board)
when the run has none. Exit 0 with the report written, 2 for a directory
without board-router.json or an unknown net or pad.

Use `--near-paths` to measure saved copper for a brief's near-pin pairs,
including on a completed route. It reads `constraints.json` beside the
PCB, or the file supplied by `--near-constraints`. Pass the saved PCB
directly after a fill or a repair that retains no router report.

```sh
pcb-maker inspect-routing-run layout/result --net /TARGET_VREF --near-paths
pcb-maker inspect-routing-run saved/MioLink.kicad_pcb --near-paths --near-constraints constraints.json
```

`near_paths` records the board and brief digests and lists the exact
pad endpoints, shortest planar trace span outside contact copper,
layers, transitions and intermediate pads. Intermediate pad contacts give
their physical UUIDs and positions, including for repeated pin numbers.
The requested endpoints must each identify one physical pad. When an intermediate pad
is farther from the direct line than the terminal spacing and the
saved project netlist identifies a supported decoupling pair,
`recovery.argv` gives a [pad path shortening trial](#shorten-a-routed-connection-between-two-named-pads).
The trial checks the live schematic and native rules again. A copper zone, ambiguous endpoint
or unsupported copper produces an unavailable reason. The optional
native measurement requires matching KiCad Python bindings and has a
512-item per-net limit. It does not add a DRC or change route acceptance.

For a selected later ladder attempt, `attempts.strategy_history` identifies
the selected attempt and sums the recorded work of the rows before it.
This sum uses ledger order and can include work inherited from a probe.
It excludes work outside those rows. Use it to budget a separate trial
on a related revision, with wall time and actual savings measured on
that revision.

When a checked plain or exclusive-plane connect rung closes all opens
at a new finer pitch and a matching coarser rung still had opens,
`strategy_history.next_trial` includes `router_config_patch` and the
comparable earlier attempt ID. Merge the provided fields into a copy
of the current router config, including the individual fields under
`tuning`. The patch selects one base rung and pitch with the recorded
plane and pin-line modes. It disables other pitch candidates, probes
and seed retries for this separate trial. Existing rules and any
configured finishing steps still apply. A smaller pitch can cost more
memory and search work. Run the usual router configuration as a control
and compare saved-board KiCad findings. First-attempt winners and runs
without a usable ledger omit `strategy_history`.

On a panel with a native report, `panel_opens.by_piece` groups retained
opens and reported DRC errors by physical board piece. Each row includes
its bounds, the open count, errors by type, up to eight net samples with
endpoint IDs and coordinates, and recorded pad access failures on its open
nets. Errors spanning pieces and errors without a resolved piece have
their own counts. Use a sample's `net` with `--net` to inspect the full
route and the rules on those terminals.
Review the named obstacle before rerouting a part with a recorded pad
access failure. Known retained across-piece pairs, pairs without a
unique physical location, and pairs stored as excluded across pieces
in the run's report are counted separately. The inspection keeps the
run's verdict. A zone requires a unique piece from its stored outline
and UUID because KiCad reports its outline origin in place of the
open island's position.

Per net:
- `components`: the groups of pads its copper joins, each pad with `ref`,
  `number`, `uuid` (the same on the source, result and reference board),
  `at` (mm) and copper `layers`, with the largest group marked.
  `pour_layers` lists ordinary copper zones. A net with a pour shows
  the groups KiCad's pairs name, and `components_omitted` counts the
  others. Supported polygons from saved teardrops are included in the
  copper connectivity model and reported in `teardrop_layers`.
- `counts`: the router's terminals and unconnected terminals (its
  search's state, pads it merged before routing as one), the groups and
  the links missing between them, KiCad's unconnected pairs, and an
  `explanation` of how they relate. They are different quantities: never
  add or compare them as one.
- `native_pairs`: KiCad's pairs with their items and the groups the
  items lie in: a pad by its uuid, a track, arc or via by the group of
  the copper it lies on (since 2026-10-25; `null` where it lies on copper
  that joins no pad).
- `open_pad_findings` links KiCad short and clearance findings to the
  same pad UUIDs and positions as retained native opens. Samples include
  the open pad, the counterpart's saved item UUID, owner and reported
  location, and KiCad's original finding. For two items on one footprint,
  review its source land pattern and pad nets. For different footprints,
  inspect their relative poses and the source geometry of the named
  copper. These findings locate an existing electrical problem at the pad.
- `zone_open_pad` is available with `--net NAME` when the native report
  names two zones for the net's one open and the matching KiCad bindings
  confirm exactly two refilled components, one with a single physical pad.
  The field gives that pad's UUID, position and copper layers, and says
  whether the footprint repeats its number. Inspect the pad and its local
  filled copper at that location. The reported zone origins identify the
  zone objects.
- `footprint_copper_bridges` appears when a native finding at a selected
  open pad leads to a verified same-footprint copper path. The probe uses
  the matching KiCad PCB bindings on the saved board and checks each
  consecutive contact at zero clearance. An active short on another pad
  of the same footprint can supply the native witness when the selected
  open pad touches the same copper path. For a witnessed plated pad the
  probe also checks that the artwork intersects its copper ring outside
  the hole. Each path names the native finding, the copper
  layer, physical pad UUIDs, pin numbers and nets, plus the ordered
  unassigned artwork UUIDs. The verified geometry describes the current
  footprint. Review the intended electrical behavior against the schematic
  before editing it. The probe covers supported planar copper graphics and
  pads, leaves declared net ties to KiCad, and records any probe limits or
  missing native bindings in `skipped`.
- `translated_routed_match` identifies a unique two-pad counterpart on
  the same board when KiCad reports a retained open between the two
  identified pads. The counterpart has matching footprint IDs, pad
  geometry, orientation and side at a common translation. Its copper
  joins both pads and KiCad reports no open on its net. The field gives
  the two pairs of endpoint poses and the translation in millimetres.
  Compare the two pad escapes and neighboring copper to investigate
  congestion around the open connection.
- `rules`: track width, clearance and vias as the run's project resolves
  them, each with its source (the class, the board's minimum with its own
  source, a custom rule), the class's own value where it is below
  (`class_track_width_mm`), the run's `overrides`, and the widths the
  board has on the net (`emitted_track_widths_mm`).
- `reference` (with `--reference`): the designer's route of the net:
  `placement` (whose placement it is on, below), `connected`,
  `components`, segments, vias, layers, widths, length.
  `connected` is `true` when its tracks, arcs, vias and stored
  teardrop copper join every pad. When separate groups remain, it is
  `false` if there is no ordinary pour and `null` if one may join them.
  `connected_reason` and `pour_layers` explain that case. KiCad DRC
  determines connectivity through ordinary pours.
- `open_at`: where KiCad finds the net open, each retained unconnected
  item's two ends (`[[x, y], [x, y]]`, KiCad's places; a zone's item is
  placed at the zone's first outline corner). Look here first.
- `dead_pads` (with `access`, below) and `hot_spots_near`: with KiCad's
  report, the hot spots within 3 mm of an end of the net's open items (a
  pad, a track or a via), the nearest first, each with
  `to_open_item_mm`, its index into the top-level `hot_spots`,
  `nearest_pad_mm` and the nets still in conflict there; without the
  report, those within 3 mm of a pad of the net (`to_open_item_mm`
  `null`). Empty where no hot spot lies near the open item: on a net with
  pads all over the board (GND, a rail) a hot spot near some pad says
  nothing of the open item (the eval study of 2026-11-24, USB_Keypad's
  +3.3V: every hot spot listed lay 16 mm away).
- `access` and `cause`: `unknown` (nothing here measures them; a hot spot
  near a pad is no cause).

With `--reference`, the top-level `reference` (since 2026-11-04) says
whose placement those routes are on, in the layout verdict's words:
`placement` is "this placement" where every part of the run's board lies
where the reference has it (route mode's: the designer's route there shows
that the net routes under these rules on this placement), else "the
designer's own placement" (a layout's: `connected` then shows that the
net routes there, not on this placement, and no route of the designer's
on this placement exists); `parts_in_place` of `parts` and
`connected_means` say it in numbers and a sentence. `reference_tracks`
counts the reference board's tracks and arcs: 0 (an unrouted board, such
as a benchmark task's stripped input; trial 5's ducktop2 right_io) makes
every net's `connected` false where no zone joins it, which says nothing
about routing, and `connected_means` says so and asks for the designer's
routed board. For a layout,
`evidence_for_this_placement` points at the layout's own routes of its
placement instead (board-layout.json and verdict.json): its first route
(`first_route_unconnected_terminals`, `first_route_vias`,
`first_route_length_mm`), its footprint moves (`moves`: `routed`, the moves
tried, and `kept`), the result's `unconnected_terminals` and
KiCad's `kicad_unconnected_items`. The Quanta75's nightly layout
(2026-11-04) with its input as `--reference`: "the designer's own
placement", 35 of 276 parts in place, first route 8 open terminals and
314 vias, 9 moves routed and 1 kept, 4 open at the end.

At the top: `checks` (`router`; `native`: `not_run`, `passed` or
`failed`, the pairs, `capped` when KiCad's report stopped at 499 items,
the pairs on nets not inspected; `components`) and `attempts`: the
ladder's ledger from board-router.json (since 2026-10-23; `null` before):
each attempt's name, pitch, pours, open count, vias, iterations, work,
routing seconds, `negotiation_stop` (`conflict_free`, `stalled`,
`work_deadline`, `negotiation_work_limit`, `probe_work_limit`,
`wall_guard`, `past_wall_deadline`, `hopeless`, `iteration_cap`) and the
`selected` one, with the run's `skipped` steps and `cut`. `hot_spots`
(since 2026-10-25): the router's hot spots, each with its tile (`box`,
16 x 16 lattice nodes), the pads in it (`ref`, `number`, `net`, `at`;
the twelve nearest the centre, `pads_in_tile` all), the nets whose copper
crosses it on its layer (`nets`) and those of both still in conflict
(`conflicted_nets`); `links_from` says whether the router recorded them
or, for a run before them, this command read them off the result board.
With a DRC report each has `nearest_open_item_mm` and
`nearest_open_item_net`: the distance to the nearest end of any of
KiCad's retained unconnected items (zones' items left out) and that
item's net. They say where nets fought, not why; a hot spot far from
every open item is not where a connection is missing.

`inspect-kicad-terminal-access`, `inspect-kicad-routing-cut` and
`inspect-kicad-route-obstructions` were removed on 2026-10-21 with the
research grid router (they read only an outline of one `gr_rect` or one
loop of `gr_line`s). Called, they exit 2 with one line naming this
command as the replacement (`--terminal REF:PAD`, `--net`); there is no
access probe or cut today. The commands that read an outline
(`describe-kicad-board`, `score-kicad-board`, `inspect-kicad-board`,
`inspect-kicad-connections`) share the router's lowering (lines, arcs,
rectangles, polygons, circles and curves, the remapper's round corners
included), and this one does not read the outline at all; the test
`the_diagnostic_commands_read_an_outline_with_arcs` runs them all on a
board with round corners.

Two nets of the remapper trial's board (jfedor2/hid-remapper, two RP2040s
at 0.4 mm pitch, 0.2 mm tracks and clearances; the trial's deep run, 300 s
of work, 19 nets open; KiCad's DRC of it as drc.json; the designer's board
as `--reference`). The run's counts, reconciled at the top: the router's
19 nets and 20 unreached terminals, 39 groups with 20 links missing, KiCad's
20 pairs, one starved thermal. QSPI_SCLK, the trial's question "which
pads, under which rules, and does the designer route it":

```json
{"net": "QSPI_SCLK", "status": "partial",
 "counts": {"router_terminals": 2, "router_unconnected_terminals": 1, "components": 2, "missing_links": 1, "native_pairs": 1,
            "explanation": "the result board's copper (pours not counted) leaves 2 groups of pads, 1 link missing; ..."},
 "components": [
   {"pads": [{"ref": "U3", "number": "52", "uuid": "eae0ab9f-65b2-44d3-aba7-873c3227fba7", "at": [138.9, 130.2625], "layers": ["F.Cu"]}], "largest": true},
   {"pads": [{"ref": "U5", "number": "6", "uuid": "7cd0194c-2aba-437b-a016-5ca4880c9dd2", "at": [136.25, 125.525], "layers": ["F.Cu"]}], "largest": false}],
 "native_pairs": [{"items": [{"description": "Pad 6 [QSPI_SCLK] of U5 on F.Cu", "at": [136.25, 125.525], "uuid": "7cd0194c-..."},
                             {"description": "Pad 52 [QSPI_SCLK] of U3 on F.Cu", "at": [138.9, 130.2625], "uuid": "eae0ab9f-..."}],
                   "components": [1, 0]}],
 "copper": {"segments": 0, "arcs": 0, "vias": 0, "layers": [], "track_widths_mm": [], "length_mm": 0.0},
 "rules": {"track_width_mm": 0.2, "clearance_mm": 0.2, "via_diameter_mm": 0.6, "via_drill_mm": 0.35, "classes": ["Default"],
           "track_width_source": "class Default (net_settings.classes in the .kicad_pro)",
           "project_minimums": {"track_width_mm": 0.2, "track_width_source": "board.design_settings.rules.min_track_width in the .kicad_pro", "...": "..."},
           "emitted_track_widths_mm": []},
 "reference": {"connected": true, "components": 1, "segments": 7, "arcs": 0, "vias": 0, "layers": ["F.Cu"], "track_widths_mm": [0.2], "length_mm": 7.6538},
 "hot_spots_near": [{"hot_spot": 0, "at": [139.375, 129.95], "layer": "F.Cu", "history": 47.09993, "nearest_pad_mm": 0.5686,
                     "conflicted_nets": ["+1V1", "QSPI_SCLK", "QSPI_SD3"]}, "..."],
 "access": {"status": "unknown", "reason": "not measured: this command runs no access probe"},
 "cause": {"status": "unknown", "reason": "not measured: this command reads the run's outputs; a cause needs a discriminating experiment"}}
```

+1V1, a supply with a zone: KiCad's pairs name a track, which lies in
the largest group; the designer's tracks alone leave three groups, and its
F.Cu zone may join them, so `connected` is `null`, not `false`:

```json
{"net": "+1V1", "status": "partial",
 "counts": {"router_terminals": 6, "router_unconnected_terminals": 2, "components": 3, "missing_links": 2, "native_pairs": 2, "explanation": "..."},
 "components": [
   {"pads": [{"ref": "C1", "number": "1", "...": "..."}, {"ref": "C2", "number": "1", "...": "..."}, {"ref": "C13", "number": "1", "...": "..."},
             {"ref": "U3", "number": "45", "uuid": "25e5aa8e-...", "at": [141.7, 130.2625], "layers": ["F.Cu"]}], "largest": true},
   {"pads": [{"ref": "U3", "number": "23", "uuid": "8c1605f9-...", "at": [140.5, 137.1375], "layers": ["F.Cu"]}], "largest": false},
   {"pads": [{"ref": "U3", "number": "50", "uuid": "7afa54c4-...", "at": [139.7, 130.2625], "layers": ["F.Cu"]}], "largest": false}],
 "native_pairs": [
   {"items": [{"description": "Pad 50 [+1V1] of U3 on F.Cu", "...": "..."}, {"description": "Track [+1V1] on F.Cu, length 0.4875 mm", "at": [141.7, 130.2625], "...": "..."}],
    "components": [2, 0]},
   {"items": [{"description": "Pad 23 [+1V1] of U3 on F.Cu", "...": "..."}, {"description": "Track [+1V1] on B.Cu, length 2.4750 mm", "at": [141.175, 135.65], "...": "..."}],
    "components": [1, 0]}],
 "copper": {"segments": 22, "arcs": 0, "vias": 4, "layers": ["B.Cu", "F.Cu"], "track_widths_mm": [0.2], "length_mm": 36.0545},
 "rules": {"track_width_mm": 0.2, "clearance_mm": 0.2, "...": "..."},
 "reference": {"connected": null,
               "connected_reason": "its tracks, arcs and vias leave 3 groups; its zone on F.Cu may join them (zone fill is not modelled here: KiCad's DRC of the reference decides)",
               "components": 3, "pour_layers": ["F.Cu"], "segments": 18, "vias": 4, "layers": ["B.Cu", "F.Cu"], "track_widths_mm": [0.2, 0.4], "...": "..."},
 "hot_spots_near": [{"hot_spot": 8, "at": [140.575, 137.15], "nearest_pad_mm": 0.076, "conflicted_nets": ["+1V1", "+3V3", "SWCLK"], "...": "..."}, "..."],
 "access": {"status": "unknown", "...": "..."}, "cause": {"status": "unknown", "...": "..."}}
```

and the hot spot both nets are near, from the report's `hot_spots`: the
1.2 mm tile above U3's pads 50 to 52, with the nets still in conflict
there:

```json
{"at": [139.375, 129.95], "box": [138.775, 129.35, 139.975, 130.55], "layer": "F.Cu", "history": 47.09993,
 "pads": [{"ref": "U3", "number": "51", "net": "QSPI_SD3", "at": [139.3, 130.2625]},
          {"ref": "U3", "number": "50", "net": "+1V1", "at": [139.7, 130.2625]},
          {"ref": "U3", "number": "52", "net": "QSPI_SCLK", "at": [138.9, 130.2625]}],
 "pads_in_tile": 3, "nets": ["QSPI_SD3"], "conflicted_nets": ["+1V1", "QSPI_SCLK", "QSPI_SD3"],
 "links_from": "the result board (board-router.json predates hot-spot links): the copper written, after the router's finishing steps"}
```

## Score a board

```sh
pcb-maker score-kicad-board <project-dir> <board-id>
```

reports objective measures ([quality.md](quality.md)); `layout-kicad-board`
puts the same report into `board-layout.json` as `quality`. It covers:
- `manufacturing.drc`: KiCad's findings by type from a `drc.json`
  beside the board; `null` with `drc_note` ("no DRC run") without one (a
  designer's project has none: `{}` read as clean on the schematic
  trial's designer board with 21 errors);
- decoupling capacitor distance to every IC supply pin, and each
  capacitor's distance to its nearest supply pin. Decoupling capacitors
  are those of at most 1 uF between a rail and ground; a regulator's
  pins (an IC of at most eight pads on two rails or more, an LDO's input
  and output) count its rails' bulk capacitors too, whatever their
  value (`worst` named an LDO's output `null` beside its 10 uF output
  capacitor);
- crystal path lengths; switching inductors (a regulator's) near
  crystals, antenna modules and analog parts (op-amps, ADCs, codecs,
  references);
- how much other copper cuts the ground pours (return paths), and every
  pour's pieces, largest piece and coverage by the router's pour model
  (`pours`, as in the verdict);
- hot parts (a regulator's tab, an exposed pad): the pad's area, the
  planes of its net over it, vias in it and track copper near it. A pour
  of that net (`add_pours`) is the cheapest heat sink;
- orientation consistency, parts near the edge, connectors facing out;
- vias in pads, tombstoning risk, KiCad DRC counts;
- the estimated price of 10 boards at common fabs, and the price
  thresholds the board crosses ([cost.md](cost.md)).

Score the designer's board and yours side by side.

## Other commands

- `pcb-maker swap-kicad-pins <project-dir> <board-id> <out-dir> <pin-swaps.json> [config.json]`
  reassigns interchangeable pins to shorten the ratsnest, board and
  schematic together, without routing ([pin-swap.md](pin-swap.md)); a
  shorter ratsnest is not shown to route better; the
  layout does the same with `swappable` or `pin_swaps`.
- `pcb-maker strip-kicad-tracks <source.kicad_pcb> <output.kicad_pcb> [--keep-nets <net>[,<net>...]]`
  removes tracks, vias and small zones (copper pours stay): a designer's
  board as the benchmarks hand it to the router. `--keep-nets` (KiCad
  wildcards, the sheet path's leading `/` optional; since 2026-11-04)
  keeps those nets' tracks, arcs and vias, which route mode then keeps
  as pre-routed nets: trial 6's firestarter, route mode on the designer's
  placement left 6 of its 10 DVP nets under the 25 mm minimum of
  `DVP_Bus_Length_Matching`; `--keep-nets 'DVP_*'` on the designer's
  board, then `route-kicad-board` on it: complete, all ten within.
- `pcb-maker strip-kicad-copper <source.kicad_pcb> <output.kicad_pcb> [report.json]`
  removes tracks, vias, copper zones and copper graphics: the bare
  placement.

- `pcb-maker ablate-kicad-placement <designer.kicad_pcb> <placed-dir> <board-id> <out-dir> [ablation.json] [router-config.json|auto]`
  (a diagnostic, [placer.md](placer.md) "Placement ablation (2026-10-24)"):
  from the designer's board and a placement of the same project (a
  layout's `placed` directory) it writes hybrids into `<out-dir>/<variant>/`,
  each a copy of the placed project with the footprints' poses changed:
  `designer` and `ours` (the two placements), `d-<group>` (the designer's
  placement with one group where ours has it), `o-<group>` (ours with the
  group where the designer has it) and `d-null-N`/`o-null-N` (one passive
  nudged 0.1 mm: the routes' noise). Groups: every footprint with more than
  20 pads alone (`large-U4`), the same with the passives of at most four
  pads that share a net with it and lie within 3 mm of its body in either
  placement (`cluster-U4`), the connectors, the board's quadrants (by the
  designer's positions), and groups of your own (`"extra": {"name":
  ["U4", "U8"]}`). Parts a moved group lands on (pads within 0.1 mm, or a
  body mostly on another) move to the nearest free spot; the report lists
  them. `<out-dir>/ablation.json` has the groups, per variant the
  displaced parts and the signals a placer can read before routing
  (wirelength, escape demand's channel overflow, exits facing a body,
  overlapping parts, ratsnest crossings, also weighted by net size and
  within buses, pins whose row faces away from their net, the global
  route's overflow and soft overflow at 0.5; these are the placer's own
  objective terms and do not predict the routed result: route the
  variants), and per moved part of both placements its escape (demand per
  side, the body it faces, the gap and the channel's capacity, the board
  edge, its connections' distance). Settings: `large_pads` (20),
  `passive_pads` (4), `passive_mm` (3), `repair` (true), `groups` (only
  these), `extra`, `threshold` (0.5), `global_route` (true), `nulls` (2),
  `null_nudge_mm` (0.1), `replicates` (0: every variant again with one
  more passive nudged, for means over the routes' noise). `benchmarks/agent-tasks/ablation.py` routes the
  variants in route mode (run.py, the memory ledger) and attributes the
  open connections per group (`prepare`, `route`, `report --markdown`).

- `pcb-maker legality-kicad-board <project-dir> <board-id> <report.json> [--poses <board.kicad_pcb>] [--drc <drc.json>] [--placer <placer.json>]`
  (a diagnostic, [placer.md](placer.md) "The legality model against
  KiCad (2026-10-26)"): judges a placement by the placer's own legality
  at every relaxation level it walks down (loosest first), without
  placing anything. The board is lowered as `place-kicad-board` lowers
  it (the directory's `constraints.json` unless `--placer` names a
  placer config; its project rules); `--poses` takes every footprint's
  pose from another board first (by uuid, else by reference: a
  designer's placement on a task's prepared board). One line per level
  on stdout (its halos, spacing, grid, edge and courtyard settings, the
  movable parts illegal there); the report has per level the illegal
  parts and their causes, and per part that is illegal at some level
  what it meets at the tightest such level: the other part or obstacle
  (`outline`, a keepout, the board's silkscreen marks), the placer's
  test, the cause measured on the footprints' own shapes (courtyards
  overlap, pads closer than their clearance, courtyards apart but closer
  than the spacing, bodies past their courtyards, the edge margin, the
  far side, ...), the courtyard, body, pad and edge gaps in mm. With
  `--drc` (KiCad's report of the board whose poses are judged, as
  `drc-kicad-board` writes it) each conflict lists KiCad's errors and
  warnings between the two footprints, each part and level counts the
  parts KiCad agrees on (`kicad_agrees`: an error against what it meets)
  and those where the placer is stricter than KiCad (`model_stricter`).
  `benchmarks/agent-tasks/legality_census.py` runs it on
  every task's designer placement.

- `pcb-maker probe-trouble-placement <placed.kicad_pcb> [router-config.json|auto] (--trouble-from <run-dir|board-router.json|probe.json> | --trouble <names.txt> | --auto)... [--nets N] [--background none|stubs|global|full] [--share 0.05] [--total-seconds 1500] [--work-seconds S] [--stub-mm 1.0] [--global-weight 2.0] [--out report.json]`
  (a research measure, [placer.md](placer.md) "The trouble-maker probe
  (2026-10-26)"): judges a placed board (a layout's `placed/<board>.kicad_pcb`;
  tracks stripped, planned pours added, the project's rules) by routing
  only its trouble-makers, first and alone, for `--share` of
  `--total-seconds` of work (or `--work-seconds`). The set: from a
  previous run (`--trouble-from` a layout or route directory or its
  board-router.json: the nets it left open, most open terminals first,
  then the nets in conflict when its negotiation ended; or a previous
  probe's report, its `new_trouble`), from a file of net names, or cold
  (`--auto`: the most pins times the largest box); `--nets` from each
  source (15 % of the board's nets of two or more pads, at least 8);
  several sources are joined; pour nets are left out. The other nets:
  `none`, `stubs` (a fixed straight stub of `--stub-mm` out of each of
  their pads towards the nearest pad of the net) or `global` (the global
  route's use of each tile by them, `--global-weight` times their tracks
  over its capacity, as a standing price); `full` is the race's
  whole-board probe for comparison. The report: the set's open terminals
  (plus one per complete net in conflict), nets unfinished and in
  conflict, conflict cells, the negotiation's work per connection and per
  mm of the set's rectilinear spanning trees, corridor misses and wider
  searches, the work (search, build, global route) and wall clock, and
  `new_trouble` (the set's nets still open or in conflict first, then by
  work per mm: the next set). Deterministic at pinned work rates. It
  reserves its memory in the ledger. `experiments/trouble-probe/study.py`
  runs it over the dense boards' seeds and ranks them.

- `pcb-maker probe-coarse-placement <placed.kicad_pcb> [router-config.json|auto] [--pitch-factor 1.0] [--iterations N] [--hard-repair] [--trouble-repair] [--negotiation-share 0.5] [--share 0.05] [--total-seconds 1500] [--work-seconds S] [--out report.json]`
  (a research measure, [placer.md](placer.md) "A coarse whole first
  route as the race probe (2026-10-27)"): routes a placed board (a
  layout's `placed/<board>.kicad_pcb`, lowered as the trouble probe
  lowers it) whole, with the race's and the first route's router
  settings at a reduced fidelity: the router's candidate lattice pitches
  (0.127, 0.125, 0.1 mm) times `--pitch-factor`, at most `--iterations`
  negotiation iterations (the router's 80 without it), all of it within
  `--share` of `--total-seconds` of search work (or `--work-seconds`).
  Without `--hard-repair` the negotiation gets the whole budget and the
  nets still in conflict then lose their conflicted branches (the hard
  repair's rip-up without its searches); with it the negotiation gets
  `--negotiation-share` of the budget and the first route's hard repair
  (and with `--trouble-repair` the trouble repair) the rest. The report:
  the pitch chosen and the lattice's nodes a layer, the iterations and
  the negotiation's stop, its standing (`negotiated_open_terminals`:
  open terminals plus one per complete net in conflict and per pour pad
  off its pour; `unfinished`; `conflicted`), the nets ripped and the
  open terminals after the rip-up, after the repair where it ran,
  `open_terminals` (the repaired count where the repair ran, else the
  ripped one), the work (negotiation, repair, build, pour) and the wall
  clock. Deterministic at pinned work rates. It reserves its memory in
  the ledger. `experiments/coarse-probe/study.py` runs it over the
  trouble study's placements and ranks them.

`pcb-maker <command> --help` gives every command's arguments and
defaults.

## Quality you can expect

Measured results are in [benchmarks.md](benchmarks.md) (corpus) and
[../benchmarks/pcbench/README.md](../benchmarks/pcbench/README.md)
(PCBench/D3, about 630 open-source boards).
