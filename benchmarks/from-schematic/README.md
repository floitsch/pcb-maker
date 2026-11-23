# Schematic to board

The whole way an agent (or a person) takes: a KiCad schematic in, a board
out, with nothing but the starter files `import-kicad-netlist` writes.

```sh
benchmarks/from-schematic/run.py build/from-schematic --jobs 2
```

For each KiCad demo whose footprints the libraries have:

1. `import-kicad-netlist` makes the board from the schematic, on as many
   copper layers as the designer used, and writes `constraints.json` (every
   part placed, the outline sized from the parts, plug-in connectors on an
   edge) and `layout.json`.
2. `layout-kicad-board` lays it out with that `layout.json`, unchanged.
3. The designer's own board is scored for comparison.

`layout.log` receives placement and routing progress during the run. Its
partial output is available if the layout fails or times out.

A board passes when every connection is routed and KiCad finds no
unconnected item, no DRC error and no schematic parity issue. DRC warnings
(a library footprint's silkscreen on its own pads) and the schematic's own
ERC findings are reported apart: they are the library's and the
schematic's, not the layout's.

Left out: the simulation demos (no footprints), jetson, One-Air-Max,
RoyalBlue54L and Q17ng (footprints their projects do not ship), and
vme-wren (1507 parts).

Results are in [docs/benchmarks.md](../../docs/benchmarks.md).

The [Open Air Max single-schematic replay](#open-air-max) uses the source PCB to supply the missing footprints.

## Supplied schematic and mechanical constraints

| Task | Reproduction guide |
| --- | --- |
| Place and route from a schematic, enclosure outline and held connector poses | [Run a supplied brief](#running-a-supplied-brief) |
| Score a layout against the original schematic, rules and mechanical constraints | [Verify a returned PCB](#verify-a-returned-pcb) |
| Compare against commercial layouts with the same input and rules | [Powder doser](#july-14-powder-doser-comparison) and [GigaShield](#gigashield-archived-two-layer-comparison) |
| Reuse a previous layout after adding a capacitor or changing a pin | [Incremental schematic edits](#incremental-schematic-edits) |
| Follow an agent's recovery after routing leaves connections open | [ClearBell amplifier and speaker nets](#clearbell-indoor-unit) |
| Diagnose an unplaced part on a breakaway board | [ProtoESP section assignments and fixed blockers](#protoesp-breakaway-panel) |
| Check a placed capacitor against its supply pin | [Ligra placement distances](#ligra-back-carrier) and [MioLink routed copper paths](#miolink-debug-probe) |
| Diagnose a routed board with a missing connector shield pin | [Explorer shield numbering](#explorer-shield-numbering) |
| Separate panel ownership from switch-socket clearance | [Katia split keyboard](#katia-split-keyboard) |

### ClearBell indoor unit

This four-layer replay starts with 56 schematic footprints and zero tracks.
The benchmark brief holds the three connectors at the source board's poses
and adds the speaker and two mounting holes at their original locations.
These six holds are assumptions of the replay. The source PCB has no
locked footprints. [Pinned KiCad project](https://github.com/akoerner-dev/ClearBell/tree/58a8d1e659f9c77e26863ec7a098708dd02772f7/hardware/Inneneinheit)

```sh
source_stem=/path/to/ClearBell/hardware/Inneneinheit/ClearBell_V0.2_Innen
board="$source_stem.kicad_pcb"
mkdir -p build/clearbell
echo '{"placer":{"constraints":"constraints.json"},"total_seconds":240,"tuning":{"move_seconds":96},"polish_seconds":10}' > build/clearbell/layout.json
PCB_MAKER_KICAD_USER_LIBS=1 RAYON_NUM_THREADS=2 PCB_WORK_RATE=3.15e6,150e6 \
  target/release/pcb-maker layout-kicad-schematic \
  "$source_stem.kicad_sch" ClearBell build/clearbell/run \
  --constraints benchmarks/from-schematic/clearbell-mechanical.json \
  --layout-config build/clearbell/layout.json --mechanical-from "$board" \
  --footprints-from "$board" --stackup-from "$board" --keep-footprint-clearances
```

With the `edge` entry omitted, the cold placement left U2 and S_RST1 without
a legal position. U2's footprint includes an antenna keepout. Adding the
exact U2 edge entry suggested by `describe-kicad-board --constraints-template`
placed all 59 parts and met all eight constraints with the same binary and
budget. The saved board retained all six held poses and all 13 outline
graphics. Independent KiCad checks on stored and refilled copper both
found three opens (the two speaker nets and GND), zero DRC errors and
zero schematic parity findings.

The one-command workflow recovered the three opens within its original
240-second allowance. After the initial 100.68 modeled seconds it used the
two amplifier pads named by the layout verdict and gave the retry 139
whole seconds. The retry used 98.00 seconds. The selected PCB had 589.21 mm
of track and 51 vias. Independent checks found zero active errors, opens
or parity findings on the saved PCB and after refill.

To reproduce the earlier standalone experiment that gave the suggested
router settings a fresh 240-second allowance, use the first layout's
verdict.

```sh
python3 -c 'import json,sys; print(json.dumps(json.load(sys.stdin)["prepared_row_escape_advice"]["router_config_patch"]))' \
  < build/clearbell/run/layout/verdict.json > build/clearbell/speaker-router.json
PCB_MAKER_KICAD_USER_LIBS=1 RAYON_NUM_THREADS=2 PCB_WORK_RATE=3.15e6,150e6 \
  target/release/pcb-maker layout-kicad-schematic \
  "$source_stem.kicad_sch" ClearBell build/clearbell/retry \
  --constraints benchmarks/from-schematic/clearbell-mechanical.json \
  --layout-config build/clearbell/layout.json --router-config build/clearbell/speaker-router.json \
  --mechanical-from "$board" --footprints-from "$board" --stackup-from "$board" --keep-footprint-clearances
```

With installed user libraries enabled for both arms, the control had three
opens. The retry joined both speaker pads and added one 1.435 mm track
from the amplifier's open ground pad to its connected exposed pad.
It completed in 118.99 modeled seconds with 572.29 mm of track and
59 vias. All 59 footprint poses matched the control. Independent stored
and refilled KiCad checks found zero opens, errors or parity findings.

Score the board selected by the one-command workflow against the
original project and the complete mechanical brief.

```sh
project=$(jq -er '.outputs.result' build/clearbell/run/workflow.json)
benchmarks/from-schematic/run.py build/clearbell/scored \
  --schematic "$source_stem.kicad_sch" --outline-board "$board" \
  --constraints benchmarks/from-schematic/clearbell-mechanical.json \
  --footprints-from "$board" --mechanical-donor "$board" --stackup-from "$board" \
  --keep-footprint-clearances --candidate-board "build/clearbell/run/$project/ClearBell.kicad_pcb"
```

The independent checker passed all ten measurements. It found U2's
antenna keepout 0.5 mm beyond the right board edge, with no overlap
or pad copper outside the board. Moving the module 3 mm inward caused
103.707 mm² of overlap. Moving H1 by 2 mm and removing LS1 also failed
their respective mechanical checks.

### MioLink debug probe

The four-layer revision B design has 111 schematic footprints and six
locked board-only footprints. Its mechanical brief copies the four
plated mounting holes and two logos from the original board and holds
three headers and a switch at their original poses. The designer left
the USB connector J1 movable. [Pinned KiCad project](https://github.com/Misaka0x2730/MioLink/tree/358438abc3601673212aa1cbc1790c4ed2e6d343/hardware/revB)

```sh
source_stem=/path/to/MioLink/hardware/revB/MioLink
board="$source_stem.kicad_pcb"
mkdir -p build/miolink
echo '{"placer":{"constraints":"constraints.json"},"total_seconds":300,"tuning":{"move_seconds":120},"polish_seconds":12}' > build/miolink/layout.json
RAYON_NUM_THREADS=2 PCB_WORK_RATE=3.15e6,150e6 target/release/pcb-maker layout-kicad-schematic \
  "$source_stem.kicad_sch" MioLink build/miolink/run \
  --constraints benchmarks/from-schematic/miolink-mechanical.json \
  --layout-config build/miolink/layout.json --mechanical-from "$board" \
  --footprints-from "$board" --stackup-from "$board" --keep-footprint-clearances
```

The import reports four schematic locks and six `matched_additions`,
eight outline items and 16 rule areas. The 117-part project starts with
zero tracks. With the 300-second allowance, the initial layout met the
placement constraints and had zero parity findings. KiCad reported one
remaining connection on +3.3VA between C6 pad 1 and U6 pad 43.

A second brief models an enclosure with a fixed USB cutout. It adds
`J1` to `front` and `{"part":"J1","origin":"absolute","x":136.9,"y":91.3,"angle":-90}`
to `place`. This additional hold is an assumption of the enclosure test.
The initial run left three opens and suggested trying placement seed 101.
Running the same schematic command with the second brief, a fresh output
directory and `--placement-seed 101` finished in 52.22 modeled seconds.
The result had 1,396.46 mm of track, 117 vias, zero opens, zero active
DRC errors and zero schematic parity findings both as saved and after
refill. Holding J1 at its original position also preserves the project's
location-specific exclusion for its mixed surface-mount and through-hole
USB footprint. On the original ten-hold brief, seed 101 left two opens.

The designer's board uses 867.77 mm of track, compared with 1,396.46 mm
on the completed cutout run. Ground accounts for 266 of the original
362 vias and 36 of the generated board's 117. On the two-terminal USB
nets from U2 to R16 and R17, the designer used 28.27 mm on each net.
The cutout run used 6.44 mm on `/USB_DP` and 5.26 mm on `/USB_DN`.
These measurements come from the saved PCB copper.

The one-command placement retry also completed the cutout brief with
the original 300-second allowance. The first placement used 65.22
modeled seconds and left three opens. The command gave seed 101 the
remaining 234 whole seconds. That placement used 42.84 seconds and the
independent returned-board check accepted its saved PCB with zero opens,
active DRC errors or parity findings. The 16 mechanical checks passed.

On the explicit seed-101 run, enabling `diff_pairs` coupled 10.19 mm
of the U2 USB pair. Its measured skew grew from 1.18 to 3.19 mm.
The other USB pair fell back to individual routing. KiCad found 62
opens in the delivered file and zero after refilling. The independent
scorer rejected that file and accepted the separate project produced by
`drc-kicad-board --save-filled`. These two results score different PCB
files under the same original rules and enclosure brief.

The completed seed-101 board also suggested moving decoupling capacitor
C33 closer to U10 pin 8. Adding its suggested
`{"part":"C33","pin_of":"U10:8","max_mm":1.5}` to `near` and starting
again from the schematic left five opens. Running ordinary layout on a
copy of the completed PCB with that same constraint, seed and allowance
closed every connection. The supply-pad centres moved from 5.87 to
1.82 mm apart. The 1.5 mm request applies to the capacitor body.
The original enclosure checks still passed on the delivered PCB, with
zero active errors, opens or parity findings as stored and after refill.
The resulting board used 1,334.52 mm of track and 118 vias. This trial
allowed general placement changes. Although the previous routes were
supplied to the command, it retained zero nets and routed the board again.

An edit on the same completed PCB held the other 116 footprints and
placed C33 on the back. KiCad verified all connections. The independent
mechanical check measured 1.315 mm from C33's body to U10 pin 8 against
the 1.5 mm limit. The actual supply connection took a 59.077 mm path
through J3 pin 2, C12, C31 and C30. A separate geometry experiment
added three 0.20 mm tracks and one through via between C33 pad 2 and
U10 pin 8. All original pads and tracks remained in place. The independent
path measurement fell to 0.329 mm of planar trace outside pad and via
copper. The three new track centrelines totalled 1.887 mm. The metric
excludes the via barrel. Both delivered PCBs passed the same mechanical
and electrical checks. Their complete native finding lists matched,
including 130 active warnings and 19 exclusions.

The public path-shortening command independently routed the same pair.

```sh
pcb-maker shorten-kicad-pad-path back-side-layout/result MioLink shortened \
  --pads C33:2 U10:8 --decoupling-shortcut
```

It added four tracks and one through via. The independent returned-board
scorer measured 0.640 mm of planar trace outside pad and via copper,
with zero errors, opens or parity findings in both copper checks.
Running the command on the already shortened 0.329 mm control kept the
original PCB byte for byte. Selecting C33's ground pad and U10's supply
pad also kept the board unchanged and reported the net mismatch.

### Caribou six-layer controller

Use a checkout of the pinned Caribou project with its schematic sheets,
library tables and footprint libraries present. Set `caribou` to the
project's KiCad directory. The brief holds the nine original locked
footprints at their authored positions and on their original faces.
The other 308 schematic footprints are movable. [Pinned KiCad project](https://github.com/Arrow-air/project-caribou/tree/05598365c661f43418147e053c528f38b9c95fb2/engineering/electronics/pcbs/CBC_PCB/kicad)

```sh
caribou=/path/to/project-caribou/engineering/electronics/pcbs/CBC_PCB/kicad
board="$caribou/CBC_PCB.kicad_pcb"
PCB_MAKER_KICAD_USER_LIBS=1 /usr/bin/python3 benchmarks/from-schematic/run.py build/caribou-cold \
  --binary target/release/pcb-maker --schematic "$caribou/CBC_PCB.kicad_sch" \
  --constraints benchmarks/from-schematic/caribou-mechanical.json --layers 6 \
  --outline-board "$board" --rule-areas-from-outline --footprints-from "$board" \
  --stackup-from "$board" --total-seconds 240
```

The donor supplies the 26 outline primitives, four board-level rule areas,
footprint geometry and 1.6 mm stackup. Routing starts without its copper.
To independently score a delivered board, use a fresh output directory
and replace `--total-seconds 240` with `--candidate-board path/to/CBC_PCB.kicad_pcb`.
The scorer checks the saved file and a fresh refill under the schematic
project's rules. The two footprint cutouts belong to HC201 and HC202.

To reproduce the cold run with the nine native footprint locks present
from import, use the production commands directly. The runner above
applies the corresponding poses during placement. These two starting
conditions can produce different placements of the movable parts.

```sh
pcb=target/release/pcb-maker
project=build/caribou-import
"$pcb" import-kicad-netlist "$caribou/CBC_PCB.kicad_sch" "$project" CBC_PCB \
  --mechanical-from "$board" --footprints-from "$board" --stackup-from "$board"
jq '.total_seconds = 240' "$project/layout.json" > "$project/layout-240.json"
PCB_MAKER_KICAD_USER_LIBS=1 "$pcb" layout-kicad-board "$project" CBC_PCB \
  build/caribou-native-locks auto "$project/layout-240.json"
```

At 240 seconds of requested work, both variants routed 157/159
connections, met the mechanical constraints and saved the zone fills.
The native-lock run used 176.66 seconds of modeled work and the explicit
constraint run used 223.64. Although the two runs produced different
placements, KiCad identified the same two open pads in both layouts,
U201.7 and U201.8. Both had zero schematic parity findings and eight
edge-clearance errors within HC201 and HC202.

`--terminal-launch-budget SECONDS` reserves part of the declared
`--total-seconds` for a second routing attempt when the first run
identifies blocked pins on a compact surface-mount footprint. The
standalone runner records the reservation in `terminal_launch_reserved_seconds`.
Read `terminal_launch_retry` for the affected pads, attempted tracks,
decision, and work consumed. On the already connected July board, this
option skipped recovery with zero additional checks and zero retry work.
When using `layout-kicad-board` directly, set `terminal_launch_pads` in the
layout JSON to select specific native-open endpoints and route from their
verified short track tips. The named-pad retry uses the same reservation.

In the Caribou replay the JSON listed `"terminal_launch_pads": ["U201:7", "U201:8"]`
and reserved 190 of 430 modeled seconds. All three arms kept the same
317 footprint poses and project rules. Each had the same eight native
footprint-edge errors and zero schematic parity findings.

| Layout | Requested work | Actual work | Native opens saved and refilled |
| --- | ---: | ---: | ---: |
| Ordinary layout | 240 s | 162.87 s | 2 and 2 |
| Ordinary layout | 430 s | 227.04 s | 2 and 2 |
| Layout with two selected pad tips | 430 s | 307.52 s | 0 and 0 |

To run the controller with an explicit copper requirement, use the
[prefuse router settings](caribou-prefuse.json) with the same schematic,
mechanical brief and six-layer stackup. The settings specify GND on In1.Cu,
+3V3 on In4.Cu and a bounded prefuse zone on In2.Cu. In the first
standalone command above, select a fresh output directory, use
`--total-seconds 430 --terminal-launch-budget 190`, and add
`--router-config benchmarks/from-schematic/caribou-prefuse.json`.
The measured run used `RAYON_NUM_THREADS=2` and
`PCB_WORK_RATE=3.15e6,150e6`.

The layout used 312.80 modeled seconds and preserved every
held pose. Its prefuse rectangle extended 2 mm beyond all 118 physical
pad centres of the seven named terminals and filled 1,789.736 mm².
Deleting just that zone from a diagnostic copy produced six opens after
refill. Keeping the zone and deleting the net's only routed copper item
left zero opens. Both checks used the original project rules.
Independent scoring found zero opens as saved and after refill, zero
schematic parity findings, and eight inherited footprint-edge errors.
This is a geometric requirement. The replay does not measure current
capacity or temperature rise.

### Running a supplied brief

Use single-schematic mode with a project schematic, a `constraints.json`
and the intended copper layer count. Put the outline in the constraints or
give an outline source with `--outline-board`. By default the runner copies
the board-level Edge.Cuts shapes from the outline source. It imports the
schematic and its libraries, applies the supplied constraints, and lays
out the new board with the schematic project's rules.

Add `--rule-areas-from-outline` to carry over board-level KiCad keepouts
from that same outline board. This option supports straight-edged
polygons, with active copper layers and declared technical layers.
It preserves area names and restrictions. The schematic project still
supplies the electrical rules. The saved-board check accepts KiCad's
expansion of layer aliases such as `F&B.Cu` and checks that technical
layers remain present.
The rule area count and geometry hash appear in `outline_rule_areas`.
Standalone layout, `--prepare-only` and `--candidate-board` scoring check
the output against the source areas. A missing or altered restriction,
polygon or layer fails the check.

The layout and returned-board checks also reject separate board-level
zones that share a UUID. See `duplicate_zone_uuids` for the IDs and
their counts. KiCad can report zero DRC errors and zero opens on such a
board even though a tool referring to the UUID cannot distinguish the zones.

Pass `--footprints-from BOARD.kicad_pcb` when the project's schematic
names footprints whose geometry comes from an existing board. KiCad's
schematic netlist supplies the part identities, pin metadata and net
assignments. The existing board supplies footprint shapes and pad
geometry, independent of its placement and routing. To use its outline
too, pass it separately as `--outline-board`. The raw footprint donor's
SHA-256 is recorded in `inputs.footprints_from` and contributes to the
brief digest. The same option works in standalone layout, `--prepare-only`
and `--candidate-board` scoring. Use `--mechanical-donor` separately when
scoring whole board parts explicitly listed in `add.from_board`.

For a comparison under the supplied footprint clearance policy, pass
`--keep-footprint-clearances` to both the layout and the `--candidate-board`
runner. The option is recorded in the brief digest and used for the
independent reference import. The result checks explicit footprint
clearance fields against that import. `import.json` identifies any
net class gaps that would have received an override under the default
policy. Board minima and custom rules still apply.

Pass `--router-config FILE` to try a routing choice such as
`{"add_pours":[{"net":"GND","layers":["F.Cu","B.Cu"]}]}`.
The result records the options file's path and SHA-256 separately from
the mechanical brief, so layouts using different routing options can be
checked against the same inputs.

Pass `--placer-config FILE` to supply a JSON object of placer options,
such as `{"decoupling_near":true}`. Omit `constraints` from this object or
set it to `"constraints.json"` to use the mechanical brief supplied by
`--constraints`. The original options file and the generated
`project/layout.json` each have a path and SHA-256 in `results.json`.
The options are recorded separately from the mechanical brief digest.

After editing a schematic, pass `--reuse-routes path/to/previous.kicad_pcb`
to try earlier poses and retain copper at unchanged pad contacts.
Placement still has to satisfy the current brief, including any held poses.
The router can keep copper for a renamed net or a net with added terminals
at new pads when the copper matches the current routing dimensions. KiCad
checks all tracks against the current project and requires no new findings,
opens, or schematic parity issues. If the trial fails, ordinary placement
and routing continue with the remaining work. `results.json` records the
previous board's digest and the reuse decision.

For accepted reuse, read `routing_coverage` for the saved board's
routable net inventory and KiCad's open-item count. It also gives the
number of retained nets and the router's pending connection counts.
The existing `routed` value still shows the router's counts. A fully
retained board can have `routed: "0/0"` because it needed no new routing.

The single-schematic workflow checks the delivered PCB's connectivity
as stored before marking the layout complete. `stored_copper` names its
verification source and reports the PCB digest, opens, errors and parity.
The runner reuses verified fill publication when its saved digest matches.
It can also use the existing native report when the board has no fillable
zones. An otherwise passing board without either proof gets a fresh DRC
of the saved file. `unconnected` remains the layout's native count after
refill and `routed` remains the model's count. If refill closed opens
that remain in the saved PCB, `stored_copper.next_action.argv` gives a
command that saves and checks the filled PCB in a new directory.

Pass `--defer-zone-fill` for a standalone layout whose verified fills
will be saved later. The option writes `"save_zone_fills": false` to
`project/layout.json`, whose SHA-256 is recorded in `results.json`.
KiCad still checks the refilled board during layout. A passing board
with fillable zones is reported at `stage: "routing_iteration"` with
`stored_copper.source: "deferred"`. When the layout
reports a fill publication outcome, `results.json` includes it under
`fill_publication`.

Use the same `PCB_MAKER_CACHE_DIR` with fresh output directories to
reuse a verified fill for the exact selected PCB and native project
inputs. `fill_publication.cache_status` records a hit, miss, or bypass.
`native_checks` counts the checks run for publication and
`native_checks_charged` records their modeled budget. Set
`PCB_MAKER_NO_CACHE=1` to run fresh checks.

The command exits 0 when every connection is routed, KiCad DRC ran with
zero errors and open items, schematic parity is clean, and all reported
placement constraints are satisfied. A failed run exits 1. The new output
directory holds `project/`, the import and layout logs, `layout/`, and
`results.json` with the gate counts, constraint results, input digests
and work rates. If a stage fails, `results.json` lists the stage, the
command's exit code, and its report or verdict when one was written.
When schematic parity fails, `parity_report` points to KiCad's findings.
If both KiCad and the source import identify the same missing footprint
pad, `unmatched_pin_details` includes the reference, pin, source net
and connection type. `next_actions` groups the affected pins by part.

`wall_time_seconds.total` measures monotonic elapsed time from the start
of the board workflow to the completed result row. The non-overlapping
values in `wall_time_seconds.stages` cover input preparation, schematic
import, layout preparation, layout and result readback. Failures include
the time through the stage reached. The layout subprocess includes the
native checks it runs. Result readback reads those reports and scores
the layout. With `--candidate-board` the `candidate_verification` stage
runs the separate verifier. With `--prepare-only` the runner reports
`package_preparation`. The older `seconds` and modeled `work_seconds`
retain their original scope.
When layout exits unsuccessfully, `layout_reason` and `next_actions`
carry the explanation and recovery advice from its verdict when available.
These fields can identify open nets or an exhausted budget even when every
placement constraint passed.
If every PCB connection is routed but the verdict identifies a missing
schematic terminal, inspect `import.json` under `unmatched_pin_details`.
It identifies the reference, footprint, pin and expected net. Correct the
symbol-to-footprint mapping, then rerun the revised schematic with the same
mechanical brief. For a pad on the wrong net, the layout verdict records
the PCB and schematic assignments under `schematic_net_mismatches`.
If the verdict reports `missing_schematic_footprints`, check the listed
symbols' footprint assignments and regenerate the board from the current
schematic. Use **Exclude from board** for intentionally schematic-only
symbols.
If the placer stops the layout, the summary includes the room estimate,
next actions, and any `placement_constraints` observed on the attempted
placement. Final layout constraint results appear after routing.
For an explicit pad-to-pad `near`, a current layout reports the strict
distance and limit in `constraints[].pad_near`. A miss appears in
`constraints_missed` even when the legacy 0.05 mm placement tolerance
covers it.

For example, run the harvested Spell Tome schematic with a two-layer
stack, five outline shapes and an absolute switch pose. The harvested
board supplies the outline geometry. The schematic and installed project
libraries supply the footprints.

```sh
case_id=bitshiftcrazy__spell_tome__spell_tome_bottom
corpus=benchmarks/real/external
mkdir -p build
cat > build/spell-tome-brief.json <<'JSON'
{
  "version": 1,
  "move_all": true,
  "place": [
    {"part": "SW2", "origin": "absolute", "x": 140.285384, "y": 103.374883, "angle": 90}
  ]
}
JSON
PCB_WORK_RATE=3.15e6,150e6 benchmarks/from-schematic/run.py build/spell-tome-layout \
  --schematic "$corpus/github-sch/$case_id/repo/spell_tome_bottom.kicad_sch" \
  --outline-board "$corpus/github/$case_id/spell_tome_bottom.kicad_pcb" \
  --constraints build/spell-tome-brief.json --layers 2 --total-seconds 500
```

The example expects the [harvested corpus](../../docs/benchmarks.md) and
a release build of `pcb-maker`. Use `--binary` to choose the same
executable on subsequent runs.

### Verify a returned PCB

Use the same schematic, brief, outline and layer count for every
submission, including a result from the standalone layout command.

```sh
benchmarks/from-schematic/run.py build/spell-tome-returned \
  --schematic "$corpus/github-sch/$case_id/repo/spell_tome_bottom.kicad_sch" \
  --outline-board "$corpus/github/$case_id/spell_tome_bottom.kicad_pcb" \
  --constraints build/spell-tome-brief.json --layers 2 \
  --candidate-board path/to/returned.kicad_pcb
```

The checker copies the PCB beside the original schematic, reachable
child sheets, project libraries, and the original `.kicad_pro` and
`.kicad_dru` when present. It uses KiCad's defaults if the schematic has
no project settings file. Candidate sidecar files are ignored. A
separate schematic import resolves the expected library cutouts. It
never places or routes the candidate. KiCad checks all track errors and
schematic parity on the staged board. The score counts active findings.
The report counts KiCad exclusions in `drc_exclusions`,
`unconnected_exclusions` and `parity_exclusions`, with the excluded
findings in the native reports. For boards with zones, it checks
the submitted copper first and then refills the zones for a second
check. Both must pass. `as_saved` gives the submitted board's counts
and report path, and `drc.json` contains the refilled results. The
checker verifies that the staged submission stayed byte-identical.

An accepted board has zero active KiCad DRC errors, open items and parity
findings, plus the expected copper layers and board-level Edge.Cuts.
The report checks footprint-local Edge.Cuts against the imported
project libraries, accounting for back-side flips. The outline donor
supplies only board-level geometry. Shape order, direction, UUIDs and
stroke widths are ignored. Rectangles and polygons can be represented
by their component lines. An arc or curve must retain its defining
points. Cutouts count as part of the outline.

With `--footprints-from`, the checker also reads the independently
imported footprints with KiCad's `pcbnew` bindings. The `footprint_geometry`
result compares each reference's local padstacks and geometric shapes on
Fab, CrtYd, copper, mask and paste layers. It ignores footprint pose and
schematic-derived pin metadata. Pad centres and graphic coordinates
are rounded to 10 nm to account for KiCad rotation rounding. Pad layer
membership is compared on the board's active copper stack. Native schematic parity still
checks connectivity. This geometry check covers pads and shapes. It
excludes text, embedded zones and 3D models.
Repeated references are compared by the full collection of local footprint
geometries, including multiplicity. The report includes physical footprint
counts and identifies unmatched repeated occurrences by UUID and pose.

The checker accepts exact `add` entries with `part` and `from_board: true`
when supplied `--mechanical-donor`. The report records the donor's
SHA-256 and compares each addition's reference, UUID, locked pose,
side, footprint attributes, graphic shapes and native pad UUIDs,
geometry and net names. Footprint zones or conductive text in added
parts require separate verification. The `edge` form with an exact `part`,
`edge: any`, `flush: true` and `overhang: {}` is supported for a single
rectangular footprint antenna keepout with full copper and footprint
restrictions. The checker compares that area against the schematic's
imported footprint, measures the gap to a straight facing edge with
up to 0.5 mm allowed, and checks for any overlap with board material
and any pad copper outside it.

Supported briefs use an outline donor or a fixed rectangle or polygon
at explicit `x` and `y`. Supply one outline or ensure they agree. For
parts, the checker measures `place` entries with `x`, `y` and `angle`,
allowed `rotation`, `front` and `back` footprint layers, and rectangular
`region` entries with x and y ranges. `origin` defaults to the outline's
top-left corner. Regions contain the whole body measured by
`describe-kicad-board`. Absolute and board-relative coordinates are both
supported. The `move_all` field controls a placement search and carries
no additional requirement on the finished board.

Use `part`, `pin_of` (for example `U1:3`) and `max_mm` to score a
body-to-pin `near` constraint. The checker measures the gap from the
placed footprint's nominal body box to the target pad centre. It
projects the local courtyard and pad extent using the returned PCB's
position, side and rotation. Mask and copper outline graphics also
contribute where the placer includes them. For a footprint without a
courtyard, the pad extent supplies the body. The report includes the
measured box and target. Body measurements with custom pad primitives
require separate verification.

For an explicit pad-to-pad `near` constraint, add `pad` and supply either
`max_mm` or `"minimize": true`. The checker reads both pad centres
from the returned KiCad board, including back-side and rotated
footprints, and records the measured distance in millimetres.
`minimize: true` records a distance objective with no acceptance
threshold. Omit `max_mm` for this mode. The independent result uses
`measured: true` for a resolved objective. For a numerical limit,
`excess_mm` shows how far the distance exceeds the request, and
`within_placer_tolerance` shows whether the placer's 0.05 mm grid
allowance covers that excess. The returned-board pass checks the
requested limit with a 1 nm numerical tolerance. This check requires
KiCad's Python bindings. A missing or ambiguous footprint or pin
location produces an `unverified` result.

Each authored `near` check can also report a `routed_path`. An explicit
`pad` must share the target's named net. For a body-to-pin check, the
scorer resolves the source when exactly one physical pad on the source
footprint shares that net. The measurement is the shortest supported
planar trace span outside pad and via copper. The report lists layers,
via or plated-pad transitions, intermediate pad centres and their
distance from the direct pad-to-pad line. It gives a reason when the
pair is ambiguous, no supported trace path connects it, or a copper
zone can carry the net. This is a quality measurement. The geometric
limit and KiCad's connectivity, parity and DRC still decide acceptance.

The CLI exits 0 for `accepted` and 1 for `rejected` or `unverified`.
`rejected` lists measured differences or native findings. `unverified`
identifies missing or unsupported evidence, such as an unresolved sheet,
missing original footprint, unhandled constraint kind, or unreadable
KiCad report. See `results.json` for the gate results, `drc.json` for the
native findings, and the step logs for command failures. The brief SHA-256
includes the schematic, its reachable local files, original rules,
constraints, normalized board-level outline geometry and requested layer
count. The original donor file, candidate and scorer binary have separate
provenance digests. Use the same KiCad version and
installed libraries across submissions. The result follows KiCad's
configured severity and exclusions. KiCad does not attest that every
custom rule compiled, so use a validated original rule file for the
comparison.

### Prepare the inputs for a comparison

Run `--prepare-only` to get a local bundle for a tool that accepts KiCad
projects. This mode requires KiCad 10's `pcbnew` Python bindings in the
runner's interpreter, in addition to `kicad-cli` and `pcb-maker`.

```sh
benchmarks/from-schematic/run.py build/spell-tome-exchange \
  --schematic "$corpus/github-sch/$case_id/repo/spell_tome_bottom.kicad_sch" \
  --outline-board "$corpus/github/$case_id/spell_tome_bottom.kicad_pcb" \
  --constraints build/spell-tome-brief.json --layers 2 --prepare-only
```

The `package/` directory contains the project, a standalone
`outline.kicad_pcb`, and `exchange.json`. Exact `place` poses with an
angle are applied and locked. `front` and `back` are applied to the
named footprints. All other footprints sit unlocked on the right of
the outline in rows, listed as `unplaced` in the manifest. The authored
constraints remain in `project/constraints.json`.
With `--placer-config FILE`, the bundle also carries `project/layout.json`
for a later pcb-maker layout, and `exchange.json` records its SHA-256.
For a placement-only replay of a bundle without that file, save
`{"constraints":"constraints.json"}` as a placer config and pass its path
as the fourth argument to `place-kicad-board`. This selects the packaged
brief, including objectives that the prepared KiCad board cannot express.

Rectangular placement regions also appear as named KiCad rule areas with
all keepout options disabled. `manual_constraints` lists the designators
and eligible area names for manual assignment in the layout tool. When
either side is allowed, the two areas are alternatives for the same
region. Enter the remaining manual constraints in the layout tool.
[Quilter's KiCad placement-region instructions](https://docs.quilter.ai/design-parameters/placement-regions.md)

For pad-to-pin proximity objectives (`minimize: true`), preparation checks
that each named pad has one unambiguous centre. The manifest asks the layout
operator to minimize each distance and report the achieved value in millimetres.

The PCB uses schematic connectivity, library footprints and any
explicitly requested footprint geometry. The outline donor contributes
board-level Edge.Cuts coordinates. The prepared project uses newly
generated footprint and outline instances, with only its authored
placement information. Local source project files are copied byte for
byte. Original DRC exclusions tied to item IDs can require review
because regenerated board items receive their own IDs. If a donor's
closed perimeter depends on footprint graphics, supply a complete
closed outline drawn with board-level Edge.Cuts. An open or unsupported
outline leaves the preparation unverified. The package manifest lists
any unresolved local project files. Keep the top-level `results.json`
and logs for local provenance.

To score the return, use the `project/` schematic and `constraints.json`
from the package, the same `--layers`, and the bundle's
`--outline-board package/outline.kicad_pcb`. Pass the returned PCB as
`--candidate-board`. Where `exchange.json` records `footprints_from`,
also pass the original footprint geometry donor. The manifest gives its
checksum. The package carries the generated footprint geometry and the
raw donor stays with the source project. Check that the brief digest
matches `exchange.json`.

### Replay the published Quilter run

The June 10 test-module project includes its unrouted starter board with
the proxy footprints already linked to the schematic. All 14 parts start
outside the board outline. Keep the three original KiCad files together
in a source directory and download the six candidate ZIPs for the second
iteration. The input revision, download URLs, and SHA-256 digests are in
the [comparison manifest](published-layout.json).

```sh
python3 benchmarks/from-schematic/published-layout.py build/quilter-replay \
  --source-dir build/quilter-input --candidates build/quilter-downloads/*.zip \
  --binary target/release/pcb-maker
```

The benchmark lays out the starter twice, once with its original design
rules and once with the settings common to all six returned projects.
It runs KiCad on each candidate under each set of rules. The starter
provides the baseline for schematic parity. Execution times for the
published layouts are unavailable.
[Measured results](../../docs/benchmarks.md#published-quilter-placement-and-routing-comparison).

The six returned boards all use ground pours on the two outer faces.
To compare that design choice on pcb-maker, run a fresh layout of the
same starter using the published runner's project and layout budget.

```sh
printf '%s\n' '{"add_pours":[{"net":"GND","layers":["F.Cu","B.Cu"]}]}' > build/two-ground-pours.json
for rules in original returned; do
  project="build/quilter-replay/$rules/layout-input"
  PCB_WORK_RATE=3.15e6,150e6 RAYON_NUM_THREADS=2 target/release/pcb-maker \
    layout-kicad-board "$project" test_module_unplaced "build/quilter-ground-$rules" \
    build/two-ground-pours.json "$project/layout.json"
done
```

### July 14 powder-doser comparison

The [July manifest](published-july-layout.json) pins the original unplaced
project, the five archived returns (1, 2, 3, 5 and 6), and the author's
review. Candidate 4 has no published board in this archive. Download
the pinned dataset and run an all-movable schematic-to-layout replay
with the following commands from the repository root. The runner
checks each archive file against the manifest before using it.

```sh
dataset="$PWD/build/july-input"
manifest=benchmarks/from-schematic/published-july-layout.json
jq -r '(["review.md", .review.url],
  (.source_files | to_entries[] | ["original/" + .key, .value.url]),
  (.candidates | to_entries[] as $candidate | $candidate.value | to_entries[] |
   ["candidates/" + $candidate.key + "/" + .key, .value.url])) | @tsv' "$manifest" |
while IFS="$(printf '\t')" read -r file url; do
  mkdir -p "$(dirname "$dataset/$file")"
  curl --fail --silent --show-error --location "$url" --output "$dataset/$file" || exit 1
done
cargo build --release --bin pcb-maker
PCB_MAKER_LEDGER="$PWD/build/july-ledger.json" \
  /usr/bin/python3 benchmarks/from-schematic/published-july-layout.py build/july-replay \
  --dataset "$dataset" --binary target/release/pcb-maker --from-schematic --near-objective
```

The source is a two-layer 110 × 110 mm board with 15 unlocked footprints
outside the outline and no routing. The runner adds the archived returns'
common four-layer stackup to that starter and uses their common project
settings for every four-layer native check. The source has its own native
baseline. With `--from-schematic`, the runner imports the pinned schematic
and verifies the footprints and routed terminal names against the
archive. It adds the original outline and common four-layer stack
before placement. The fresh layout makes all footprints front-side, plans GND on
In1.Cu and sets +12V and GND traces to the 0.513593 mm width measured in
all five returns. KiCad's common project minimum is 0.254 mm. The result
records the widths actually emitted on each net. The replay reserves
memory on the shared pcb-maker ledger and uses 1,200 modeled layout seconds
with four threads by default.

To test a shorter routed connection from C3's supply pad to its target
pin on the completed board, run the following command with the same release binary.

```sh
target/release/pcb-maker shorten-kicad-pad-path build/july-replay/layout/result \
  test_module_unplaced build/july-c3-shortened --pads C3:1 U5:16
```

`build/july-c3-shortened/pad-path.json` records the proposed span,
the change in total track length, and whether KiCad accepted the change.

Native KiCad 10 checks each board with stored fills and with a fresh refill.
The five archived returns are checked in a single background worker while
the fresh layout runs. Every replay verifies the input pins and reruns
these checks in its own KiCad configuration. Use `--serial-archive-scoring`
to run the checks after layout. A matched 1,200-modeled-second
`--near-objective` run with KiCad 10.0.8 took 40.225 s serially and 27.627 s
with overlap. Both runs returned the exact same new board and native
score. A body-to-pin run returned a different board and scored it afresh.
All three runs agreed on archive findings, placement and routing metrics
after accounting for KiCad-generated UUIDs on items with no stored ID.
See `build/july-archive-score-20261120/audit.json`.

Add `--rail-drop-simulator /path/to/simulate_power_ngspice_run2.py` to
score the five archived returns and the new layout with the July author's
ngspice model. Supply the script locally with Git blob
`76ef67462ea28a0a908b0ba3adf9206ec84ebbf7`. The Python interpreter needs
`pcbnew` and `kiutils`, and `ngspice` must be on `PATH`.

The `rail_drop` report names the source, load currents and per-load
operating-point drop on +12V, +5V and +3V3. It includes the +12V
solenoid transient and lists modeled links through modules. The author
model assumes 35 µm trace copper, ideal through vias and ignores zone
copper and ground return. Unsupported geometry, unexpected copper nets,
unreached loads and missing simulation dependencies are reported as
`unavailable` for the rail score. Placement and native board scores
use the same checks with or without this option.

To score an existing routed July PCB without laying it out again, run
`/usr/bin/python3 benchmarks/from-schematic/july_rail_drop.py PCB --rail-drop-simulator SCRIPT`.
The JSON includes the scored PCB's SHA-256 and the simulator provenance.

The all-front powder-doser layout and the author's preferred archived
return had the following worst-load drops. Both used the same three
sources, seven loads and ideal via assumptions from the pinned model.

| Board | +12 V | +5 V | +3.3 V | Solenoid droop | Total track | Vias |
| --- | ---: | ---: | ---: | ---: | ---: | ---: |
| Schematic layout, three capacitor objectives | 81.53 mV | 36.79 mV | 13.45 mV | 39.50 mV | 1,116.58 mm | 0 |
| Same layout with a B.Cu feed from J1:1 to U5:22 | 39.53 mV | 36.79 mV | 13.45 mV | 39.50 mV | 1,188.00 mm | 0 |
| Archived candidate 6 | 71.19 mV | 124.42 mV | 16.75 mV | 71.20 mV | 1,363.09 mm | 25 |

To reproduce the second row, point `routed` to the first row's routed
project and `output` to a fresh directory. The command added seven
segments totaling 71.43 mm on the back. Under the same load currents,
the largest remaining +12 V drop was at the solenoid driver U4. KiCad
reported zero errors, opens and schematic mismatches on the saved
and refilled boards.

```bash
pcb-maker add-kicad-parallel-feed "$routed" test_module_unplaced "$output" \
  --pads J1:1 U5:22 --layer B.Cu --width-mm 0.513593
/usr/bin/python3 benchmarks/from-schematic/july_rail_drop.py \
  "$output/test_module_unplaced.kicad_pcb" --rail-drop-simulator "$simulator"
```

On a second generated board, Spell Tome, the battery net shares the
front layer with a GND pour. A 0.20 mm feed from connector J2:1 to
inductor L1:1 added three tracks totaling 7.032 mm. The command rebuilt
the ground fill and checked it after reopening. Independent KiCad checks
found zero errors, opens or schematic mismatches both as saved and after
refill. All 145 original tracks and vias were preserved. A second trial
from J2:1 to regulator U2:6 left two open connections and one starved
thermal after refill. The command returned the original PCB byte-for-byte.

U5, the 1 A stepper controller, accounted for the baseline's largest
12 V drop. Asking the placer to minimize the distance from J1:1 to
U5:22 reduced the rail's worst drop to 38.24 mV and increased the +5 V
drop to 72.14 mV. The board used 1,412.99 mm of track. Choosing the
other +12 V pin, U5:16, also reduced the drop at U5, but moved the
worst drop to the solenoid driver at 104.66 mV. All three layouts
routed 22/22 nets, used zero vias and passed native error, open and
schematic parity checks in both fill modes.

The simulator script and the review are archived alongside the
returned boards. The review records 178 mV as candidate 3's worst
12 V drop. Replaying the pinned script gave 170.52 mV for candidate 3
with both its KiCad file parser and the native-coordinate adapter.
[Author's simulator](https://github.com/vertical-cloud-lab/powder-doser/blob/a829d3d335f143ae6ca536c6bd7ee7740afe7ad9/paper/background/starter_board/quilter_candidates_run2/simulate_power_ngspice_run2.py)

The JSON includes raw errors by type, open connections, schematic parity
against the starter, copper length, vias and mechanical geometry. It
also compares exact outline, footprint geometry and pad/net assignments.
The archive-based replay tracks the 77 inherited parity findings by
type and description. The schematic-based replay measures parity
against its own newly imported starter. The
five returns each report 154 errors with the stored fills (77 clearance
and 77 hole-clearance, all against an unassigned zone); they have zero
errors after refill and zero opens in either mode.

Capacitor distances are straight-line distances between native pad centres.
C1:1 to U1:2 reproduces the published SHDN distance, while C1:1 to U1:3
measures the physical VIN pin requested for placement. Both sets use
C2:1 to U2:22 and C3:1 to U5:16. There is no authored acceptance threshold
for these distances. Leave `--pad-to-pad` off for the matched body-to-pin
placement control, which uses the same three target pins.
Use `--near-objective` for an all-movable pad-to-pad placement objective
with no numerical gate. `--pad-to-pad` replays the historical input,
which sets a numerical limit of zero. The zero-limit run measures
distances and cannot satisfy the constraints on these separate pads.
To test a finite limit with all 15 parts movable, run
`--pad-near-max-mm 6`. That option applies a diagnostic limit chosen for
this benchmark. The replay records the native pad distances in
`near_measurements` and the finite limit result in `near_limits_met`.
Use distinct output directories for each arm. Run the archival controls
without `--from-schematic`.

To refine the three capacitors while holding the other 12 poses from
that control, replay from the same copper-free starter with the same binary.

```sh
python3 benchmarks/from-schematic/published-july-layout.py build/july-body \
  --dataset build/july-input --binary target/release/pcb-maker
python3 benchmarks/from-schematic/published-july-layout.py build/july-refined \
  --dataset build/july-input --binary target/release/pcb-maker \
  --refine-body-to-pin build/july-body
```

The November 18 replay measured these pad distances to the physical pins.
All three pcb-maker boards were entirely front-side, routed 22 of 22
connections without vias, and had zero KiCad errors and opens in both
fill modes. The refinement verified that only C1, C2 and C3 changed pose.
Archived candidates 2 and 5 placed components on the back.

| Placement | C1 to VIN (mm) | C2 to VSYS (mm) | C3 to VIN (mm) | Track length (mm) |
| --- | ---: | ---: | ---: | ---: |
| Body to pin | 9.819 | 6.674 | 6.009 | 1231.406 |
| Pad to pad, all parts movable | 5.860 | 5.870 | 5.870 | 1569.433 |
| Pad to pad, only capacitors movable | 5.860 | 5.870 | 6.176 | 1234.771 |
| Archived candidate 6 | 5.153 | 5.740 | 5.206 | 1363.087 |

Each physical pad pair also reports its signed pad displacement, assembly
faces, relative rotation, the native copper polygon gap on shared copper
layers and the package gaps. The Fab gap uses the nominal
body outlines when both consist of four axis-aligned lines. The courtyard
gap uses KiCad's native polygons, with their bounding boxes measured
separately. On opposite faces these gaps describe the board-plane projection.

| Placement | C1–U1 Fab / courtyard (mm) | C2–U2 Fab / courtyard (mm) | C3–U5 Fab / courtyard (mm) |
| --- | ---: | ---: | ---: |
| Pad to pad, only capacitors movable | 2.090 / 1.100 | 1.520 / 0.530 | 2.100 / 1.110 |
| Archived candidate 6 | 1.383 / 0.393 | 1.383 / 0.393 | 1.431 / 0.441 |

Candidate 6 and the refinement place all three pairs on the front at the
same relative rotations of 180°, 0° and 0°. Each pair clears its native
courtyard and has the same gap from the polygon and its bounding box.
Candidate 6 packs the packages closer. Its C3 pad also sits 0.220 mm off
the target pad's row, compared with 1.920 mm in the refinement.

A November 20 cold replay imported the pinned schematic, allowed all
15 footprints to move and used `--near-objective`. It routed 22/22 in
34.56 modeled seconds with 1,116.579 mm of track and zero vias. Against
the schematic, KiCad found zero opens, errors and parity findings both
as saved and after refill, with 15 warnings. The three physical pad
distances were 5.271, 5.851 and 5.271 mm. Archived candidate 6 measured
5.153, 5.740 and 5.206 mm on those same pairs. The five archived boards
used 18–25 vias and 874–1,603 mm of track. The complete native reports
are in `build/published-july-schematic-replay-20261120-gap/results.json`.

`routed_supply_paths` measures the shortest supported path between each
pad pair along the routed net's planar trace centrelines. Copper contacts
through pads or vias contribute zero length. The JSON records the traversed
layers, track spans and layer changes, or a reason when the geometry is
unsupported. All six paths below stay on F.Cu.

| Placement | C1 to VIN trace (mm) | C2 to VSYS trace (mm) | C3 to VIN trace (mm) |
| --- | ---: | ---: | ---: |
| Pad to pad, only capacitors movable | 4.133 | 4.260 | 4.596 |
| Archived candidate 6 | 3.392 | 4.087 | 3.441 |

#### All-movable schematic replay

The November 19 replay used the same binary, stack, board outline,
0.513593 mm power track widths, 1,200 modeled-second budget and four
threads for all three schematic arms. Every board preserved the
footprints and routed all 22 connections without vias. Independent
KiCad checks found zero errors, opens and parity findings in both
stored and refilled modes.

| Input for the three pairs | C1 / C2 / C3 pad centres (mm) | C1 / C2 / C3 supply traces (mm) | Total track length |
| --- | --- | --- | ---: |
| Historical zero limit | 5.860 / 5.870 / 5.870 | 4.133 / 4.260 / 5.324 | 1569.433 mm |
| Diagnostic 6 mm limit | 5.895 / 6.006 / 5.906 | 4.133 / 4.392 / 4.144 | 1121.305 mm |
| Minimize distance | 5.271 / 5.851 / 5.271 | 3.544 / 4.241 / 4.667 | 1567.292 mm |
| Archived candidate 6 | 5.153 / 5.740 / 5.206 | 3.392 / 4.087 / 3.441 | 1363.087 mm |

C2 exceeds the 6 mm diagnostic limit by 0.005814 mm. The native
pad copper gaps in the objective arm are 3.511, 4.205 and 3.511 mm.
C3's supply trace runs on B.Cu between its plated pads, adding
1.156 mm to that pair's measured copper gap. Candidate 6 uses F.Cu
and its trace follows the 3.441 mm pad copper gap. The objective arm
still uses 204.205 mm more total trace than candidate 6.

The current `--from-schematic --near-objective` replay also searches for
a compact placement with the same three pad distances. It kept compact
draft seed 1, transferring C1, C2 and C3 independently from seed 16.
All 15 footprints remained movable under the authored brief. The board
measured 5.271000, 5.851224 and 5.271245 mm between pad centres and
3.544346, 4.240831 and 3.545585 mm along their F.Cu supply traces.
Total trace was 1,116.579 mm, down 450.713 mm from the objective replay
above. The independent score verified the same outline, layer stack,
footprint geometry, pad nets and 0.513593 mm power track widths. Both
KiCad readbacks had zero errors, opens and parity findings, with 15
footprint library warnings. The routed board had zero vias and all 22
connections. The 1,200-second work budget and four threads stayed the
same. The layout recorded 34.559 modeled seconds, including 1.831 for
the extra placement search, versus 116.975 in the previous objective arm.

After applying the strict pad limits to placement seed selection, racing
and routing moves, the same all-movable 6 mm brief passed at 5.993930,
5.904246 and 5.905878 mm, with 1,113.416 mm of track and one via.
With `refine_pad_near` enabled it passed at 5.895390, 5.987019 and
5.905878 mm, with 1,121.241 mm of track and zero vias. A separate
tighter diagnostic brief capped the distances at 5.272000, 5.852224
and 5.272245 mm. The completed board measured 5.271574, 5.851653
and 5.271262 mm with 1,390.711 mm of track and zero vias. An earlier
run of that brief passed the limits during placement, then moved U2
during routing and exceeded the C2 limit. The three updated boards
routed 22 of 22 connections with zero native errors, opens and parity
findings in both fill modes.

#### Standalone replay with a 6 mm diagnostic brief

The [6 mm brief](july-six-mm.json) sets a diagnostic limit chosen for
this benchmark. It holds the 12 non-capacitor poses from the body-to-pin
run, places all parts on the front, and gives the three capacitors the
same pad pairs as the comparison above. Use the original schematic,
project rules, outline and footprint geometry. The router options come
from the July replay above.

```sh
july_source=build/july-input/original
july_board=test_module_unplaced
july_brief=benchmarks/from-schematic/july-six-mm.json
july_router=build/july-refined/layout-input/router.json
july_run=build/july-six-mm-layout
july_binary=target/release/pcb-maker
printf '%s\n' '{"tuning":{"refine_pad_near":true}}' > build/july-six-mm-placer.json

PCB_WORK_RATE=3.15e6,150e6 RAYON_NUM_THREADS=4 \
  /usr/bin/python3 benchmarks/from-schematic/run.py "$july_run" \
  --schematic "$july_source/$july_board.kicad_sch" --layers 4 \
  --outline-board "$july_source/$july_board.kicad_pcb" \
  --footprints-from "$july_source/$july_board.kicad_pcb" \
  --constraints "$july_brief" --total-seconds 1200 --binary "$july_binary" \
  --router-config "$july_router" --placer-config build/july-six-mm-placer.json

/usr/bin/python3 benchmarks/from-schematic/run.py build/july-six-mm-scored \
  --schematic "$july_source/$july_board.kicad_sch" --layers 4 \
  --outline-board "$july_source/$july_board.kicad_pcb" \
  --footprints-from "$july_source/$july_board.kicad_pcb" \
  --constraints "$july_brief" --binary "$july_binary" \
  --candidate-board "$july_run/layout/result/$july_board.kicad_pcb"
```

For the ordinary-placement control, omit `--placer-config` and use a
fresh output directory. Both runs used 1,200 modeled seconds and four
threads. The scorer checks the 6 mm limit against native pad centres.

| Placement | C1 to VIN (mm) | C2 to VSYS (mm) | C3 to VIN (mm) | Independent verdict |
| --- | ---: | ---: | ---: | --- |
| Ordinary | 5.860009 | 6.005814 | 6.176026 | Rejected |
| With `refine_pad_near` | 5.860009 | 5.987019 | 5.546004 | Accepted |

Only C2 and C3 changed pose. Both routed all 22 connections with zero
vias, KiCad DRC errors, opens and schematic parity findings. Both
independent scores recorded 15 footprint library warnings.
The ordinary run used 17.25 modeled work seconds and 1255.858 mm of
trace. The refinement used 27.71 modeled work seconds and 1254.426 mm
of trace. Both schematic ERC reports list 77 unconnected pins in the
source schematic.

#### Standalone workflow latency

Repeat the ordinary 6 mm layout above in two new output directories,
with both runs using the same initially empty `PCB_MAKER_CACHE_DIR`.
The table also includes a run using the accepted prior through
`--reuse-routes` with a separate empty cache, and an ordinary repeat
with `PCB_MAKER_NO_CACHE=1`. All four runs used the same source and
brief, four threads, a 1,200-modeled-second budget, the standard KiCad
CLI, and the default native checks and saved fills.

| Run | Import (s) | Layout subprocess (s) | Workflow total (s) | Fill publication within layout (s) |
| --- | ---: | ---: | ---: | ---: |
| First ordinary layout | 0.969 | 30.759 | 31.759 | 5.124 |
| Exact ordinary repeat | 2.489 | 27.803 | 30.351 | 0.083 |
| Reuse the routed PCB | 0.980 | 11.311 | 12.321 | 3.584 |
| Ordinary repeat with cache bypass | 1.691 | 20.814 | 22.545 | 2.141 |

The exact repeat published a verified cache hit with zero actual
publication checks, down from two. It still performed 42,436,658 A*
expansions and used 34.91 modeled seconds, the same nominal work as the
first run and the cache-bypass control. Route reuse kept all 309
segments and needed zero A* expansions and 15.29 modeled seconds.
Even then, layout took 11.311 seconds, including fresh native checks
and verified fill publication. Result readback took at most 1.1 ms.
The four saved boards were byte-identical and passed independent
mechanical, stored-fill and refilled all-track checks. Elapsed times
are single observations on the benchmark host.

#### Incremental schematic edits

Use the saved PCB from the accepted 6 mm run above as the previous revision.
The [GPIO edit](july-edits/move-gpio.patch) moves the servo signal from U2
pad 20 to pad 12 and renames a separate net. The [capacitor
edit](july-edits/add-capacitor.patch) adds C4 between +5V and GND. Each
patch applies independently to the original July schematic.

```sh
edit=add-capacitor       # or move-gpio
edited="build/july-$edit-source"
mkdir -p "$edited"
cp "$july_source/$july_board.kicad_pro" "$edited/"
patch -o "$edited/$july_board.kicad_sch" "$july_source/$july_board.kicad_sch" \
  "benchmarks/from-schematic/july-edits/$edit.patch"
previous="$july_run/layout/result/$july_board.kicad_pcb"
PCB_WORK_RATE=3.15e6,150e6 RAYON_NUM_THREADS=4 \
  /usr/bin/python3 benchmarks/from-schematic/run.py "build/july-$edit-incremental" \
  --schematic "$edited/$july_board.kicad_sch" --layers 4 \
  --outline-board "$july_source/$july_board.kicad_pcb" \
  --footprints-from "$july_source/$july_board.kicad_pcb" \
  --constraints "$july_brief" --total-seconds 1200 --binary "$july_binary" \
  --router-config "$july_router" --placer-config build/july-six-mm-placer.json \
  --reuse-routes "$previous"
```

For a cold comparison, omit `--reuse-routes` and use a fresh output
directory. Score both returned boards against the edited schematic and the
original 6 mm brief using the independent scoring command above.

The November 19 runs used the same binary within each pair, four threads
and a budget of 1,200 modeled seconds. All four layouts completed with
zero vias, native DRC errors, open connections and schematic parity
findings. Elapsed times are single observations on the benchmark host.

| Change | Cold / incremental elapsed (s) | Cold / incremental modeled work (s) | Cold / incremental A* expansions | Cold / incremental trace (mm) |
| --- | ---: | ---: | ---: | ---: |
| Add C4 | 24 / 8 | 17.42 / 11.80 | 42,948,037 / 216,642 | 1267.67 / 1318.72 |
| Move servo GPIO and rename a net | 24 / 8 | 27.72 / 11.73 | 42,487,908 / 43,178 | 1260.21 / 1259.16 |

A later C4 replay included publication of the filled board and used a
fresh native-report cache for each run. In cold, reuse, reuse, cold
order, elapsed times were 33.19, 15.82, 15.18 and 25.53 seconds on a
shared host. Each cold run used 24.62 modeled seconds and 1267.67 mm
of trace. Each reuse run used 15.40 modeled seconds and 1310.15 mm of
trace, retained 309 segments and routed both new connections. All four
outputs passed independent KiCad checks for opens, errors and schematic
parity both as saved and after refill.

The capacitor revision kept 309 existing segments, placed the new
capacitor, and connected its two pads. The pin revision kept 304 segments
and routed the changed signal. Those runs used two native comparison
checks each. A separate control added a front copper keepout across
the old `STP_RX` trace. The router withheld that net before the trial and
kept 304 segments on 21 nets. It routed `STP_RX` and both new capacitor
connections in 292,291 A* expansions. The cold run took 42,938,523
expansions. Independent KiCad and mechanical checks passed both boards,
including the copper keepout. The incremental board used 1318.77 mm of
copper and the cold board used 1267.67 mm. Both used zero vias.

The C4 revision also measures the cost of checking the same routed
candidate twice. Run the incremental command above three times with each
binary in separate output directories and set `PCB_MAKER_NO_CACHE=1` in
both arms. The ordinary and shared-check runs used the same four threads
and 1,200 modeled seconds. A fresh validated all-track report served both
the final-board check and the comparison against the separately checked
input. This took two native DRC calls per layout instead of three. All
six saved PCBs were byte-identical, with identical stable verdict fields
and native findings. All runs retained 309 segments, routed both new
connections in 216,642 A* expansions, and reported zero errors, opens,
parity findings or vias and nine footprint library warnings. Modeled
work fell from 11.80 to 8.20 seconds. Median elapsed time was 14.23 vs
13.11 seconds over three runs each, with overlapping ranges of
13.82–14.44 and 13.04–16.61 seconds.

A route-only control added a current rule that warns on `+12V` tracks
wider than 0.2 mm. Both runs rejected the reuse trial on the same 39
native findings. Their fallback PCBs were byte-identical, with the
same final KiCad findings and verdict. Both ran four native DRC checks.

#### Add C4, remove it, and reuse the result

Start with the accepted 6 mm PCB and run the C4 addition above. Use the
added board as the previous PCB for a layout from the original schematic.
The original schematic is byte-identical to the result of reversing the
C4 patch. Run it once more using the removal result as the previous PCB.

```sh
previous="build/july-add-capacitor-incremental/layout/result/$july_board.kicad_pcb"
for stage in remove-c4 reuse-after-removal
do
  out="build/july-$stage"
  PCB_WORK_RATE=3.15e6,150e6 RAYON_NUM_THREADS=4 \
    /usr/bin/python3 benchmarks/from-schematic/run.py "$out" \
    --schematic "$july_source/$july_board.kicad_sch" --layers 4 \
    --outline-board "$july_source/$july_board.kicad_pcb" \
    --footprints-from "$july_source/$july_board.kicad_pcb" \
    --constraints "$july_brief" --total-seconds 1200 --binary "$july_binary" \
    --router-config "$july_router" --placer-config build/july-six-mm-placer.json \
    --reuse-routes "$previous"
  previous="$out/layout/result/$july_board.kicad_pcb"
done
```

Score each saved board against that revision's schematic and the 6 mm
brief. Independent checks preserved all common footprint poses, pad
contacts, and constraints. All three boards had zero vias, all-track DRC
errors, opens, and schematic parity findings. The addition had 16
footprint-library warnings. Removal and subsequent reuse each had 15.

| Stage | Prior segments kept | Pending connections routed | A* expansions | Modeled work (s) |
| --- | ---: | ---: | ---: | ---: |
| Add C4 | 309 | 2 | 216,642 | 8.20 |
| Remove C4 | 309 | 0 | 0 | 8.09 |
| Reuse after removal | 309 | 0 | 0 | 8.09 |

The addition suppressed two redundant +5V track items totaling 8.567 mm,
reducing saved trace from 1318.721 to 1310.154 mm. Exact copper coverage and
the 318 remaining segment UUIDs matched the earlier build. Removal pruned
five +5V segments and four GND segments. It preserved all 309 original
segment geometries, including the unsplit +5V trunk at the branch tee.
All 309 segment UUIDs survived removal, and the next reuse produced
the same PCB bytes. The earlier build rerouted +5V after removal in
159,745 expansions and used 8.16 modeled seconds.

A verification control charged three KiCad checks and 11.69 modeled
seconds on both zero-pending stages. The final runs used the same
validated all-track report for saved-board verification and reuse
comparison. The two checks per stage reduced modeled work to 8.09
seconds and produced byte-identical PCBs.

#### Reannotation with unchanged symbol paths

The [July reannotation fixture](july-reannotation.py) swaps C2 and C3
while keeping their original schematic UUIDs. It rewrites the two
reference-based near constraints so each follows its original symbol.
The second fixture moves C2 to C7 and U2 to U7. KiCad reassigns 20
automatically generated unconnected net names for U2. All 99 electrical
groups stay the same when compared by symbol path and pad number.

Use the accepted 6 mm layout as the prior routed PCB. Set
`baseline_binary` to the binary before schematic-path matching, and
`july_binary` to the current binary.

```sh
variant=swap             # or rename
work="build/july-reannotation-$variant"
prior="$july_run/layout/result/$july_board.kicad_pcb"
mkdir -p "$work"
/usr/bin/python3 benchmarks/from-schematic/july-reannotation.py prepare \
  "$july_source" "$work/input" --variant "$variant"

run_reannotated() {
  arm=$1
  binary=$2
  PCB_MAKER_NO_CACHE=1 PCB_WORK_RATE=3.15e6,150e6 RAYON_NUM_THREADS=4 \
    /usr/bin/python3 benchmarks/from-schematic/run.py "$work/$arm" \
    --binary "$binary" --schematic "$work/input/$july_board.kicad_sch" --layers 4 \
    --outline-board "$july_source/$july_board.kicad_pcb" \
    --footprints-from "$july_source/$july_board.kicad_pcb" \
    --constraints "$work/input/constraints.json" --total-seconds 1200 \
    --router-config "$july_router" --placer-config "$work/input/placer.json" \
    --reuse-routes "$prior"
}
run_reannotated baseline "$baseline_binary"
run_reannotated current "$july_binary"
```

Score each returned board with the 6 mm `--candidate-board` command above
using the edited schematic and constraints. For independent all-track
checks, run `july-reannotation.py native <returned.kicad_pcb> <check-dir>`.
Its `compare` command takes the original donor, the variant and each layout,
score and native-check directory.

Both versions kept all 15 poses and footprint geometries by symbol path.
The current version retained all 309 donor track UUIDs and geometries
in both edits, including the run where the unused net names changed.
Both versions passed the current brief and independent stored and
refilled KiCad all-track checks with zero errors, opens or parity
findings. All four boards had zero vias.

| Edit | Kept segments, baseline / current | A* expansions, baseline / current | Modeled work, baseline / current (s) | Elapsed, baseline / current (s) |
| --- | ---: | ---: | ---: | ---: |
| Swap C2 and C3 | 112 / 309 | 36,274,344 / 0 | 26.90 / 15.29 | 22.77 / 30.51 |
| Rename C2 and U2 | 85 / 309 | 24,025,090 / 0 | 23.32 / 15.29 | 30.82 / 14.05 |

Elapsed times are single serial observations on the benchmark host.
The slower swap observation included an 8.3-second native board check.
The saved swap trace length was 1254.426 mm in both runs. The rename
layout measured 1265.919 mm with the baseline and reused 1254.426 mm
with the current binary.

The `recycled` fixture replaces C3's symbol UUID while leaving its
reference, connections and geometry intact. Reuse withheld both
+12V and GND, reused 124 other segments, and rerouted both affected nets.
The `legacy-donor` control removes prior board paths and keeps all 309
segments for the unchanged schematic. Combining the swap with
`--move-gpio` retained 304 segments, carried the HAPT_ENABLE net rename
and routed the changed SERVO_SIG terminal. A swap using the old
mechanical brief moved C2 and C3 by symbol path. The original poses
would have produced 60.451 and 49.880 mm gaps under that brief. The
saved gaps were 5.987 and 5.546 mm against the 6 mm limit. All controls
passed independent native and current mechanical checks.

A separate swap used the [moved-C3 layout](#move-a-capacitor-on-a-routed-net)
as the donor. The retry restored one pose by symbol path. Current C2
moved from its initial pose at `(99.096196, 16.070861)` mm to the donor
C3 pose at `(99.096196, 14.95)` mm. It kept all 309 donor segment UUIDs
and passed the current brief and native checks.

#### Move a capacitor on a routed net

The [branch reuse check](july-branch-reuse.py) uses the original July
schematic and the accepted 6 mm board above. It fixes all 15 footprint
poses and moves C3 from `(99.096196, 16.070861)` to
`(99.096196, 14.95)` mm. The C3:1 to U5:16 distance is 5.989953 mm
after the move. The unchanged original board passes the 6 mm brief and
misses the revised brief at C3's exact pose.

Generate a revised brief with the previous board and stage it with
`run.py --prepare-only`. Route the same placed PCB with each binary
and share the same previous board and router settings.

```sh
work=build/july-moved-c3
prior="$july_run/layout/result/$july_board.kicad_pcb"
mkdir -p "$work"
/usr/bin/python3 benchmarks/from-schematic/july-branch-reuse.py brief \
  "$prior" "$work/revised.json"
/usr/bin/python3 benchmarks/from-schematic/run.py "$work/prepared" \
  --schematic "$july_source/$july_board.kicad_sch" --layers 4 \
  --outline-board "$july_source/$july_board.kicad_pcb" \
  --footprints-from "$july_source/$july_board.kicad_pcb" \
  --constraints "$work/revised.json" --binary "$july_binary" --prepare-only
printf '%s\n' '{"constraints":"constraints.json"}' > "$work/placer.json"
export PCB_WORK_RATE=3.15e6,150e6 RAYON_NUM_THREADS=4
"$july_binary" place-kicad-board "$work/prepared/package/project" "$july_board" \
  "$work/current" "$work/placer.json" --router "$july_router"
# Set baseline_binary and branch_binary to the two executables.
PCB_MAKER_CACHE_DIR="$work/baseline-cache" "$baseline_binary" route-kicad-board \
  "$work/current" "$july_board" "$work/baseline" "$july_router" --reuse-routes "$prior" \
  > "$work/baseline.log" 2>&1
PCB_MAKER_CACHE_DIR="$work/branch-cache" "$branch_binary" route-kicad-board \
  "$work/current" "$july_board" "$work/branch" "$july_router" --reuse-routes "$prior" \
  > "$work/branch.log" 2>&1
```

Use the independent `--candidate-board` command from the 6 mm example
to score the previous board under the original brief and both routed
boards under `"$work/revised.json"`. Write the scores to
`"$work/prior-score"`, `"$work/baseline-score"`, and
`"$work/branch-score"`.

```sh
/usr/bin/python3 benchmarks/from-schematic/july-branch-reuse.py compare \
  "$prior" "$work/current/$july_board.kicad_pcb" \
  "$work/prior-score" "$work/revised.json" \
  --arm baseline "$work/baseline" "$work/baseline-score" "$work/baseline.log" \
  --arm branch "$work/branch" "$work/branch-score" "$work/branch.log" --expect-branch branch \
  > "$work/comparison.json"
```

The two route commands used four threads, a fresh cache per arm, and
the same 1,200-second modeled budget. Both saved boards satisfied the
revised brief and had zero vias, DRC errors, opens and schematic parity
findings. Each had 15 footprint-library warnings and one silkscreen-edge
warning.

| Route mode | Prior segments kept | A* expansions | Modeled work (s) | Board trace (mm) | C3:1 to U5:16 trace (mm) |
| --- | ---: | ---: | ---: | ---: | ---: |
| Whole-net reuse | 124 | 35,816,502 | 12.17 | 1255.345 | 4.293 |
| Branch reuse | 307 | 0 | 0.72 | 1256.294 | 5.049 |

Two serial runs per arm took 18.47–31.07 seconds for whole-net reuse and
8.79–8.87 seconds for branch reuse. These elapsed times cover the route
command, including its checks. Each arm returned byte-identical PCBs on
its repeats.

Branch reuse preserved the +12V trunk to all 11 stationary contacts and
the GND trunk to all 34 stationary contacts. It replaced one segment per
net with a short front copper connection from C3's moved pad to the
original neighbor. The saved GND zone is on In1.Cu, separate from the
donor ground segments. The independent measurement uses exact segment
coordinates, layers, widths, and physical pad contacts. It measures the
+12V track-centreline path through copper outside the pads. An earlier
trial let the router reconnect C3:1 after removing its entire leaf
branch. It passed the DRC but reached U5:16 by a 75.587 mm trace path.

Standalone routing on the July schematic replay saved the GND inner-layer
fill after two KiCad checks. Independent readback matched all 290 track
and via forms and all 15 footprint poses with the earlier route. Stored
and refilled checks both had zero opens, errors and parity findings.
Enabling the installed footprint libraries generated the same PCB bytes.

The archived placement input has legacy footprints without persistent
instance UUIDs. KiCad assigned different IDs to the same library warnings
when it reopened this input. The fill check reported the mismatch and
retained the routed PCB byte-for-byte, with or without the installed
footprint libraries. See
`build/direct-route-powder-doser-handoff-20261120/schematic-comparison.json`
and `build/direct-route-powder-doser-handoff-20261120/candidate/verdict.json`.

### Spell Tome net rename

This two-layer board exercises a footprint with two physical lands on each
numbered pin. Start with a routed Spell Tome board from the
[mechanical workflow above](#supplied-schematic-and-mechanical-constraints), using
[the enclosure brief](spell-tome-keepout.json). Copy the schematic project,
including its libraries, and rename both `SELECT` global labels to
`SELECT_KEY`. Run the revised schematic once cold and once with
`--reuse-routes` pointing to the original layout. Keep the outline,
brief, two layers, four threads and 500-second work budget the same.

On November 19 both runs passed the mechanical brief and KiCad's DRC,
connectivity and schematic parity error checks. Reuse kept 200 segments
and one via across all 18 routed nets, including the renamed net. Both
boards had 441.75 mm of copper and preserved all 37 footprint polygons on
`User.1`. A* expansions fell from 7,552,940 to zero. Sharing the final
all-track DRC report eliminated one KiCad check on the fully connected
board. Two runs per arm with a fresh cache each took 11.73–11.82 seconds
cold and 8.14–8.45 seconds with reuse, including verification. Modeled
work fell from 11.99 to 9.16 seconds. The saved board had zero KiCad
opens across its 18 routable nets and zero connections pending routing.

### SLAB left keyboard

The [SLAB brief](slab-left-mechanical.json) holds all 30 key switches,
their 30 matching LED footprints, the LED31 indicator, nine mounting
holes, five connectors, the edge-mounted Xiao controller and the two
user controls. The other 48 schematic parts can move, including the
J6 header added in the schematic. The original board and schematic
are from headblockhead/slab-pcb at
`c71d1f6a1f07661351185173c98aee3c5ce4b81d`. The schematic project
supplies the design rules. The original board supplies the outline
and the footprint geometry. The generated board starts with no
tracks, vias or copper zones.

```sh
case_id=headblockhead__slab-pcb__slab-pcb-left
corpus=benchmarks/real/external
donor="$corpus/github/$case_id/slab-pcb-left.kicad_pcb"
source="$corpus/github-sch/$case_id/repo/slab-pcb-left/slab-pcb-left.kicad_sch"
brief=benchmarks/from-schematic/slab-left-mechanical.json
LC_ALL=C PCB_WORK_RATE=3.15e6,150e6 RAYON_NUM_THREADS=4 \
  benchmarks/from-schematic/run.py build/slab-left-layout \
  --schematic "$source" --constraints "$brief" --layers 2 --total-seconds 548 \
  --outline-board "$donor" --footprints-from "$donor"
benchmarks/from-schematic/run.py build/slab-left-score \
  --schematic "$source" --constraints "$brief" --layers 2 \
  --outline-board "$donor" --footprints-from "$donor" \
  --candidate-board build/slab-left-layout/layout/result/slab-pcb-left.kicad_pcb
```

The six parity findings in the earlier benchmark are present on the
original board and on the new copper-free placement. U1's schematic
marks pins 17–22 no-connect. Its original custom footprint in the
library and on both boards has two physical lands for each of pins
1–14. The importer reports the six unmatched pins and preserves the
schematic and footprint. The older PCB also lacks J6 and has
different assignments at U1 pins 7 and 14 and U3 pin 13. The new
layout uses the supplied schematic assignments.

The archived board uses GND on F.Cu and +3V3 on B.Cu. Repeat the
cold invocation in a new directory with
`--router-config benchmarks/from-schematic/slab-left-pours-050.json`
to generate the same supply layers with 0.50 mm thermal spokes.
`slab-left-pours.json` runs the 0.30 mm default-spoke control. Both
generate zones with 0.50 mm clearance and relief gap, 0.25 mm minimum
copper width, and the archived thermal connection and island-removal
settings. The generated zones follow the curved board perimeter.
Clipped to the original board material, they differ from the archived
zone polygons by 1.02 mm² per layer out of 13517.89 mm².

| Run | Routed | Trace length | Vias | Stored fill polygons | KiCad opens, stored/refilled | Modeled seconds |
| --- | ---: | ---: | ---: | ---: | ---: | ---: |
| Archived designer PCB | — | 5945.58 mm | 239 | 76 | 0 / 1 | — |
| Cold without pours | 76/76 | 7994.41 mm | 203 | 0 | 0 / 0 | 370.30 |
| Cold with 0.30 mm spokes | 75/76 | 7598.05 mm | 312 | 0 | 7 / 1 | 508.84 |
| Cold with 0.50 mm spokes | 76/76 | 7269.47 mm | 316 | 121 | 0 / 0 | 405.54 |

All three cold boards preserved the 78 holds and 1030 outline and
cutout primitives. The independent checks used the original project
rules in both stored and refilled modes. Every row has zero DRC
errors and the three cold rows have the same six U1 parity findings.
The 0.30 mm control missed `/ENCODER_B` after refill and had
insufficient remaining work budget to publish verified fills. The
0.50 mm run saved both verified fills. Strict scoring still fails on
the six source parity findings.

### Open Air Max

Set `case_dir` to the KiCad `demos/openair-max` directory containing the revision 3.2 schematic and PCB. The [mechanical brief](openair-max-mechanical.json) fixes the six footprints locked on the designer PCB. The schematic determines connectivity. Importing from the installed libraries alone found 7 of the 204 parts. The board supplied the other 197 footprints, the 58 outline primitives, three rule areas and the 1.2 mm four-layer stack.

```sh
pcb="$case_dir/One-Air-Max.kicad_pcb"
PCB_WORK_RATE=3.15e6,150e6 RAYON_NUM_THREADS=4 \
  /usr/bin/python3 benchmarks/from-schematic/run.py build/openair-max-cold \
  --binary target/release/pcb-maker --schematic "$case_dir/One-Air-Max.kicad_sch" \
  --outline-board "$pcb" --rule-areas-from-outline --footprints-from "$pcb" --stackup-from "$pcb" \
  --constraints benchmarks/from-schematic/openair-max-mechanical.json --total-seconds 240
```

The November 19 cold run placed all 198 movable parts and routed 120/120 nets with 154 vias and 3,196.107 mm of track. Independent checks matched the six held poses, outline, rule areas, stackup and footprint geometry. KiCad found zero schematic parity findings and 24 DRC errors. Twenty clearance and solder-mask findings involved geometry inside Q1, Q2 and F2. U6 pads 3, 4, 7 and 9 each had a custom pad shape that failed KiCad's single-polygon check. Those four errors also persisted on isolated copies of U6 from both the designer and cold boards.

The initial saved PCB had 76 native opens. They all closed on zone refill. The layout had deferred saving the fills because the native report reached its per-type cap for two kinds of warning, silkscreen overlap and footprint-library issues. Saving and reopening a separately refilled PCB gave zero opens, the same 24 errors and zero parity findings in both native checks. The independent scorer rejected both boards.

With verified fill publication for these static warnings, the same cold run saved both filled zones. Fresh KiCad checks found zero opens in the delivered file and after refill. All 2,096 track segments, 154 vias and 204 footprints matched the prior run exactly, including their IDs. The extra verification brought the modeled work from 197.69 to 201.29 seconds. Independent scoring still reports 24 errors. The verdict identifies U6's four malformed custom pads and their source library footprint.

A separate 180-second cold run using `placer.signal_anchor_seed` reached one native open on `/Solar_module/ILIM_HIZ`. The normal local repair closed it with eight copper additions in 7.65 modeled seconds, and kept all 2,464 existing tracks and vias (2,460 distinct forms) and the same 204 footprint poses. The saved board and its refill each had zero opens, zero parity findings and the same 24 native errors. The check retained all reported warnings, including 199 capped silkscreen overlaps. A copy with a custom `silk_clearance` rule kept the open and the original PCB bytes because that rule can also compare silkscreen to copper. The unchanged footprint errors still prevent strict acceptance.

### CM5 Minima on the original board outline

Point `case_dir` at `demos/cm5_minima` from the [pinned KiCad source tree](https://github.com/KiCad/kicad-source-mirror/tree/f35419ed35e74ed6b6f82e1f9a9d29e8eb2df59f/demos/cm5_minima).
The benchmark imports the eight-sheet design and takes 30 outline primitives and one six-layer keepout from the original PCB. Placement and routing start from the schematic. The original PCB also contains 28 board-only footprints outside the 84 schematic-backed components.

The comparison brief fixes 16 poses taken from the designer board, including external connectors, module sockets, switches and mounting points, and assigns 15 components to the rear face. The remaining 68 poses are free. We chose these holds for the benchmark. An earlier 15-hold trial left the HDMI connector J501 free and placement failed to find a legal pose for it. The internal headers J502, J503, J601 and J802 remain movable.

```sh
PCB_WORK_RATE=3.15e6,150e6 RAYON_NUM_THREADS=4 PCB_MAKER_LEDGER="$PWD/build/cm5-ledger.json" \
  /usr/bin/python3 benchmarks/from-schematic/run.py build/cm5-cold \
  --binary target/release/pcb-maker --schematic "$case_dir/CM5_MINIMA_3.kicad_sch" \
  --outline-board "$case_dir/CM5_MINIMA_3.kicad_pcb" --layers 6 --rule-areas-from-outline \
  --constraints benchmarks/from-schematic/cm5-minima-mechanical.json --total-seconds 240
```

At a 240-second modeled budget, the cold run placed 84/84 components and routed 95/95 nets with 143 vias and 2121.86 mm of tracks. KiCad found zero opens, three geometry errors and four schematic parity findings. Both J1 pad-to-edge errors and the H102/J1 courtyard overlap also occur on the original board. The four parity findings identify symbol pins M1–M4 missing from the J701 footprint, including M1 on GND. The original board has one additional courtyard error and one additional parity finding. The benchmark returns an incomplete verdict.

An independent returned-board check reproduced the three geometry errors, zero opens and four parity findings with both stored and refilled zones. It verified all 31 face and pose constraints and the source keepout.

The eight KiCad-recognized differential pairs with exactly two pads on each net and copper on both members give a second comparison. On the designer PCB the largest difference between the two members' summed in-plane track lengths was 0.010 mm. On the cold result it was 11.423 mm on DPHY0 D2. These lengths exclude via barrels. The two boards also have different physical stackups. KiCad's Gerber job export reports a 1.5296 mm board with 0.0994 mm between F.Cu and In1.Cu for the original. The six-layer import in the command above exports 1.600 mm and 0.274 mm respectively. Impedance and propagation delay require separate evaluation.

Add `--stackup-from "$case_dir/CM5_MINIMA_3.kicad_pcb"` to carry the original 1.5296 mm fabrication stackup. The option works with `--prepare-only` and `--candidate-board` as well. The saved-board check includes the full ordered layer definitions, so a changed dielectric property is reported even when the overall thickness and copper layer count match.

A cold run with the original stackup and coupled routing enabled connected 95/95 nets at the same 240-second modeled budget. KiCad found zero opens, three geometry errors and four parity findings. KiCad's Gerber job export reproduced all 17 material entries from the designer board on the final routed board.

Under the same work rates and budget, add `--router-config` with `{"diff_pairs":true}` to exercise coupled routing. An earlier build routed all 95 nets with 184 vias and 2606.72 mm of track. It reported five pairs coupled across all planned links, six partly coupled and 21 on ordinary routing. On PCIe TX the negative member also carried an ordinary-width path around the paired track.

| Coupled-routing run | Board track (mm) | Vias | PCIe TX negative / positive track (mm) | PCIe TX negative / positive shortest pad-boundary path (mm) |
| --- | ---: | ---: | ---: | ---: |
| Before branch removal | 2606.72 | 184 | 99.901 / 51.759 | 43.901 / 49.198 |
| With branch removal | 2559.69 | 182 | 53.872 / 50.749 | 52.260 / 49.198 |

Branch removal retained all 11 coupled links and all 95 routed nets. Both shortest paths remained traceable on the saved board. The in-plane track-length difference on PCIe TX fell from 48.142 mm to 3.124 mm. Both runs had the same three native DRC errors, 122 warnings, four schematic parity findings and zero opens. On a separate M.2 adapter with nine routed differential pairs and 28/28 connections, all nine measured pair lengths stayed identical.

On the original material stack, a matched 240-second coupled-routing replay
tested a 0.25 mm native `skew` rule scoped to
`/CM5/DPHY0_P.D2_P` and `/CM5/DPHY0_P.D2_N`. D2 still routed as separate
nets; the rule added a 0.135 mm meander to D2_P on In1.Cu. KiCad's
via-inclusive skew fell from 0.2784 to 0.1436 mm under the same measurement
rule. The layout kept every footprint pose, all 182 vias, zero opens,
the same three geometry errors and four schematic parity findings.
Both arms used 172.987 modeled seconds. Use the electrical design's
limit when applying the rule to a board.

A development control removed only PCIe TX's two conductors from the paired output and rerouted with a 30-second budget. The router kept 2,406 segments and 178 vias on the other nets. KiCad found zero opens, but the pair fell back to ordinary routing because a pad stub could not reach the coupled run. Use the full cold replay above when testing whether a change fixes the duplicate path.

Two further comparisons tested the placement of the D401 and D402
HDMI protection arrays. The schematic connects each of the four lanes
to J501 and to the module, with two pads per conductor on the array.
The first comparison added eight 5 mm bounds from the connector-facing
array pads to their matching J501 pads. It retained the 16 mechanical
holds and met all eight new bounds. The other 68 poses changed. Native
opens fell from three to two while coupled HDMI links fell from three
to zero. A separate KiCad measurement found all four uncoupled lengths
increased, including the clock from 21.4294 to 33.3766 mm.

The second comparison froze all 82 unrelated poses and routed both
boards again from the schematic. It moved only D401 and D402 from the
first comparison's control poses to the author's lane-aligned poses.
The native clock skew fell from 8.4950 to 0.6458 mm. D1 skew grew from
0.0330 to 20.7031 mm, and the candidate introduced two shorts and one
clearance error. Neither candidate passed the whole-board comparison.
The measurements used private copies with native skew and uncoupled
length checks to read the values. Both routing arms kept the authored
project rules. Runs, constraints and independent stored/refill checks
are in `build/cm5-hdmi-placement-20261120/`.

Planning the remaining connector endpoint across its ground pad gave a
different result. The router can now plan paired runs on both sides of a
component with opposing rows of the same pair. In the matched cold
replay with 68 movable footprints, routed connections rose from 93/95
to 95/95, and native opens fell from three to zero. Three of four planned
HDMI links coupled before the change, and five of eight coupled after it.
The routed output changed just one footprint pose, R704, and used 178
vias instead of 180. Both versions kept the original project rules and
reported the same three hard geometry findings and four parity findings.

| HDMI lane | Native skew, before / after (mm) | Native uncoupled length, before / after (mm) |
| --- | ---: | ---: |
| CK | 12.3931 / 7.6623 | 21.4294 / 11.0671 |
| D0 | 5.2962 / 5.8297 | 18.3178 / 21.1376 |
| D1 | 1.2211 / 1.2404 | 11.9925 / 14.5598 |
| D2 | 2.1009 / 0.0360 | 7.9716 / 16.4846 |

With all 84 poses fixed, three of four HDMI lanes had shorter native
uncoupled lengths. Native opens, geometry errors and parity remained at
one, three and four, respectively; the router coupled five of eight HDMI
links, up from two of four. The M.2 holdout stayed byte-identical.

### CM5Carrier V1 cold on the authored six-layer stack

Use the `hardware` directory at the [pinned CM5Carrier revision](https://github.com/AsadImranRafique/CM5-Carrier-Board/tree/188c42e7768c7373f4dbbd421967b5722bab9112/hardware).
The designer PCB locks all 54 footprints. For this comparison we hold
14 interface poses: the module frame and its four mounting holes, its
two mating connectors, J1–J8, RJ45 U4 and front indicators D2 and D3.
All 40 other poses are movable. The brief preserves the designer's
assembly sides for all 54 parts. The original project, eight outline
primitives, multilayer rule area, footprint geometry and 1.6 mm
fabrication stack are inputs. Import starts with zero routed copper and
all 40 movable footprints at the origin. Both layouts gave each of the
40 footprints a different pose from the designer PCB.

```sh
w="$PWD/build/cm5-carrier-cold"
pcb="$case_dir/CM5Carrier.kicad_pcb"
sch="$case_dir/CM5Carrier.kicad_sch"
bin="$PWD/target/release/pcb-maker"
mkdir -p "$w"
export LC_ALL=C PCB_WORK_RATE=3.15e6,150e6 RAYON_NUM_THREADS=2
/usr/bin/python3 benchmarks/from-schematic/cm5-carrier-mechanical.py \
  "$pcb" "$w/mechanical.kicad_pcb" "$w/brief.json"
"$bin" import-kicad-netlist "$sch" "$w/project" CM5Carrier \
  --mechanical-from "$w/mechanical.kicad_pcb" --footprints-from "$pcb" --stackup-from "$pcb"
cp "$w/brief.json" "$w/project/mechanical-brief.json"
printf '%s\n' '{"total_seconds":240,"placer":{"constraints":"mechanical-brief.json"}}' > "$w/layout.json"
printf '%s\n' '{"tuning":{"via_cost":0.5}}' > "$w/cheap-vias.json"
PCB_MAKER_CACHE_DIR="$w/ordinary-cache" PCB_MAKER_LEDGER="$w/ordinary-ledger.json" \
  "$bin" layout-kicad-board "$w/project" CM5Carrier "$w/ordinary" auto "$w/layout.json"
PCB_MAKER_CACHE_DIR="$w/cheap-vias-cache" PCB_MAKER_LEDGER="$w/cheap-vias-ledger.json" \
  "$bin" layout-kicad-board "$w/project" CM5Carrier "$w/cheap-vias" "$w/cheap-vias.json" "$w/layout.json"
for arm in ordinary cheap-vias; do
  /usr/bin/python3 benchmarks/from-schematic/run.py "$w/score-$arm" --binary "$bin" \
    --schematic "$sch" --constraints "$w/brief.json" --outline-board "$pcb" \
    --rule-areas-from-outline --footprints-from "$pcb" --stackup-from "$pcb" --layers 6 \
    --candidate-board "$w/$arm/result/CM5Carrier.kicad_pcb"
done
```

Both November 20 cold runs used 240 modeled seconds of available work and
the same selected placement. Both routed 65/65 connections and saved
their filled In1.Cu GND plane. Fresh KiCad 10.0.8 all-track checks found
zero errors, warnings, opens or schematic parity findings on the saved
and refilled boards. The independent scorer verified all 68 face and pose
constraints, 54 footprint geometries, eight outline primitives, one
rule area and the full stackup. Native readback counted 486 pads and ten
non-plated drills on each board.

| Saved board | Track centreline (mm) | Vias | Modeled layout work (s) | KiCad errors / opens / parity |
| --- | ---: | ---: | ---: | ---: |
| Designer | 2624.528 | 265 | — | 0 / 0 / 5 |
| Default via cost | 2872.132 | 109 | 107.240 | 0 / 0 / 0 |
| Via cost 0.5 | 2935.625 | 120 | 102.755 | 0 / 0 / 0 |

Designer parity comprises four DNP settings and one description field.
The designer's ten copper zones follow its original placement. The cold
brief leaves electrical copper generation to the router, so the zone
counts and track lengths describe different routing implementations.

### d20 battery side and touch-electrode rule area

The [d20 brief](d20-mechanical.json) holds D6, J2, J1, C3 and SW1 at the
designer board's poses and sides. The archive at
`220807c99caede139dd4e102ad59b6b185d05d6a` supplies the outline, three
custom footprints and a 12-vertex back-copper rule area that blocks
tracks, vias, pads and zone fills. The original schematic supplies all
37 components and 22 named terminal groups. Explicit permission lets
the placer try BT1 on the back, as on the designer board.

```sh
case_id=bitshiftcrazy__d20_pcb__d20_pcb
corpus=benchmarks/real/external
donor="$corpus/github/$case_id/d20_pcb.kicad_pcb"
source="$corpus/github-sch/$case_id/repo/hardware/d20_pcb.kicad_sch"
brief=benchmarks/from-schematic/d20-mechanical.json
mkdir -p build
printf '%s\n' '{"allow_back":["BT1"]}' > build/d20-allow-back.json
LC_ALL=C PCB_WORK_RATE=3.15e6,150e6 RAYON_NUM_THREADS=4 \
  benchmarks/from-schematic/run.py build/d20-layout \
  --schematic "$source" --constraints "$brief" --layers 2 \
  --outline-board "$donor" --rule-areas-from-outline --footprints-from "$donor" \
  --placer-config build/d20-allow-back.json --total-seconds 240
benchmarks/from-schematic/run.py build/d20-score \
  --schematic "$source" --constraints "$brief" --layers 2 \
  --outline-board "$donor" --rule-areas-from-outline --footprints-from "$donor" \
  --candidate-board build/d20-layout/layout/result/d20_pcb.kicad_pcb
```

In the November 19 run the supplied permission placed BT1 on the back.
The matching arm without the permission stopped with BT1 unplaced.
The completed board routed 19/19 connections in 92.49 modeled seconds
with 472.06 mm of copper and 23 vias. Independent scoring accepted all
five poses and sides, the outline and the rule area. Fresh native KiCad
all-track checks on the stored and refilled board each found zero
errors, opens and schematic parity findings, with 37 warnings.
Deleting the area from a candidate or changing it to allow pads failed
independent scoring on an otherwise accepted board.

### OpenFC Lite

The [OpenFC brief](openfc-mechanical.json) specifies one back-side USB
connector pose and an absolute region for the MCU on the original
outline, which spans 37.94 × 37.94 mm. The schematic and project
libraries supply all 164 footprints.

```sh
case_id=OpenDrone-hw__OpenFC-Lite__OpenFC
corpus=benchmarks/real/external
PCB_WORK_RATE=3.15e6,150e6 RAYON_NUM_THREADS=4 benchmarks/from-schematic/run.py build/openfc-layout \
  --schematic "$corpus/github-sch/$case_id/repo/hardware/OpenFC.kicad_sch" \
  --outline-board "$corpus/github/$case_id/OpenFC.kicad_pcb" \
  --constraints benchmarks/from-schematic/openfc-mechanical.json --layers 6 --total-seconds 240
```

In the 2026-11-18 run the attempted placement satisfied the USB pose,
USB side and MCU region. The front-side utilization estimate was
105.3%, and the verdict counted 89 parts without a legal place. The
run stopped before routing or KiCad DRC.

The [optional side permissions](openfc-allow-back.json) let the placer
move approved SMD footprints to the back. Add
`--placer-config benchmarks/from-schematic/openfc-allow-back.json`
to the command above and use a fresh output directory. The permission
patterns identify 94 footprints on the imported board. The same outline,
USB pose and MCU region are checked throughout the run.

At 240 modeled seconds on November 19, the current baseline stopped at
91 parts without a legal place. With the permissions, the placer moved
82 footprints to the back and legally placed all 164. Front/back
utilization was 71.6%/34.1%. U2 stayed in its region and Card1 stayed on
the front. The saved board routed 81 of 82 connections. The independent
schematic and mechanical check found all three constraints kept, zero
parity issues, one +3.3 V open at U10:5, and eight DRC errors within
Card1, U10 and USB1. The run reports the selected references under
`back_side_assignments` in `board-placer.json`.

The imported pad geometry matches the supplied schematic libraries.
USB1 and Card1 each have two unnumbered plated holes with a copper
diameter equal to the drill. The saved designer PCB uses unplated holes
at those same locations and edits U10's custom copper on pads 1, 3 and 6.
The project rules and custom-rule file match across both sources. A
KiCad check of the three imported footprints in isolation reproduced
all eight errors. The same isolated check with `--footprints-from` the
designer PCB found one clearance between U10 pads 4 and 5, measured at
0.0860 mm against the 0.0900 mm project minimum. The full donor import
also reported five unmatched schematic pins, each on an unconnected net.

For this placement, add a protected plane-escape retry and leave the
default verified zone-fill publication enabled.

```sh
case_id=OpenDrone-hw__OpenFC-Lite__OpenFC
corpus=benchmarks/real/external
LC_ALL=C PCB_WORK_RATE=3.15e6,150e6 RAYON_NUM_THREADS=4 \
  benchmarks/from-schematic/run.py build/openfc-plane-escape \
  --schematic "$corpus/github-sch/$case_id/repo/hardware/OpenFC.kicad_sch" \
  --outline-board "$corpus/github/$case_id/OpenFC.kicad_pcb" \
  --constraints benchmarks/from-schematic/openfc-mechanical.json \
  --placer-config benchmarks/from-schematic/openfc-allow-back.json \
  --layers 6 --total-seconds 390 --plane-escape-budget 150
```

The 390-second modeled budget reserves 150 seconds for the retry and
keeps the ordinary layout's 240 seconds. The retry protects nearby plane
connections and reroutes the selected placement. KiCad checks all native
opens, parity and findings before the run adopts the new route. Read the
retry report at `layout/result/plane-escape.json` and stored-fill evidence
at `layout/result/drc-saved-fill.json`. To recheck the returned board
against the original schematic, outline and constraints, run the same brief
in a new output directory with
`--candidate-board build/openfc-plane-escape/layout/result/OpenFC.kicad_pcb`
and omit the placer and work-budget options.

On November 19, the retry closed the +3.3 V open at U10:5 in 116.02
modeled seconds. The full layout used 243.02 modeled seconds and routed
82/82 connections. It kept all 164 footprint poses, sides, pad geometry
and nets, all three mechanical constraints and the original outline and
project rules. The saved PCB contains both verified plane fills and keeps
both authored zone polygons byte-for-byte. Independent KiCad 10.0.8 checks
found zero opens and zero parity issues with both stored and fresh fills.
The whole PCB has 126 vias and 1269.67 mm of traces, compared with 150
vias and 1190.62 mm in the ordinary layout.

The layout verdict reports `complete` and records the accepted attempt
in `plane_escape` and the stored-fill check in `fill_publication`. The
strict schematic benchmark still fails on the same eight raw errors.
These comprise four U10 copper clearance findings and four zero-annulus
plated holes on Card1 and USB1.

### Eurorack PMOD audio interface

The [mechanical brief](eurorack-pmod-mechanical.json) holds eleven
connectors and two mounting holes at their original positions, angles and
sides. The six-layer layout places the other 138 schematic footprints.
The donor supplies the outline, footprint geometry, clearance policy and
stackup. The input is revision `ddb9aa92fab7f74783f7ed3bf248eec56a6ceb00`.
[Eurorack PMOD r3.5 source](https://github.com/apfaudio/eurorack-pmod/tree/ddb9aa92fab7f74783f7ed3bf248eec56a6ceb00/hardware/eurorack-pmod-r3.5)

```sh
case_name=apfaudio__eurorack-pmod__eurorack-pmod-pcb
archive=benchmarks/real/external
schematic="$archive/github-sch/$case_name/repo/hardware/eurorack-pmod-r3.5/eurorack-pmod-pcb.kicad_sch"
board="$archive/github/$case_name/eurorack-pmod-pcb.kicad_pcb"
PCB_WORK_RATE=3.15e6,150e6 RAYON_NUM_THREADS=2 \
  /usr/bin/python3 benchmarks/from-schematic/run.py build/eurorack-interface \
  --schematic "$schematic" --constraints benchmarks/from-schematic/eurorack-pmod-mechanical.json \
  --layers 6 --outline-board "$board" --footprints-from "$board" --stackup-from "$board" \
  --keep-footprint-clearances --total-seconds 600 --binary target/release/pcb-maker
```

Eight audio jacks on repeated schematic sheets share the footprint
`AudioJacksFixed:pj324m`. Their hidden supplier fields occupy different
positions on the donor board. Treating those positions as instance data
reduced schematic parity findings from eight to zero and preserved each
jack's field values, positions and visibility. At 547 modeled seconds,
all 13 held poses matched. The saved and refilled PCB still had 31 opens
and one footprint-type error. Ten opens belonged to `/-VCOM`, whose
eleven pads had no connecting track.

With the original side permissions, applying the verdict's suggested
`{"tuning":{"via_cost":0.5}}` reduced opens from 31 to 18 at the same
600-second budget. Both stored and refilled checks reported 18. The
returned PCB used 225 vias, up from 188, and consumed 590 modeled seconds.

A matched run gave explicit assembly permission to move `R*`, `C*` and
`L*` to the back. To reproduce it, supply a placer JSON file containing
`{"allow_back":["R*","C*","L*"]}` via `--placer-config` and use a fresh
output directory. All 13 interface poses remained exact. The placer
moved 94 components to the back, including 13 resistor networks and
eight RV-series protection devices. The run connected **133/133 nets**
in 303 modeled seconds, using 186 vias and 3,041 mm of track. Independent
KiCad checks on saved and refilled copper found zero opens and zero
parity findings against the packaged schematic. Scoring the same PCB
against the untouched original schematic reported two footprint ID
mismatches. H1 and H2 have empty Footprint fields in that schematic,
and the import assigns their supplied mounting-hole footprints in its
project copy. The returned-board check matched all 151 footprint
geometries, the outline, stackup and mechanical constraints. U1 retained
the footprint-type error present on the original board, so the raw
zero-error gate still failed.

A narrower permission named 88 capacitors, resistors, resistor networks
and inductors. The placer moved 86 to the back and left the eight RV
protection devices on the front. This also connected 133/133 nets, in 323 modeled seconds
with 192 vias and 3,328 mm of track.

Restricting back-side permission to just the 13 resistor networks left
three native opens. Applying the 96 individual permissions proposed by
the all-front run's `rear_placement_recovery.allow_back_additions`
closed every net in 267 modeled seconds. Independent KiCad checks of
the saved and refilled board agreed, and all 13 mechanical poses held.

Two decoupling constraints illustrate the difference between pad distance
and routed length. With a 1.5 mm pad-centre limit, pad refinement and
both capacitors on the front, C31:1 to U7:6 measured 1.894 mm and C14:1
to U8:2 measured 2.194 mm. Requiring C31 and C14 on the back brought the
distances to 0.686 mm and 0.183 mm while keeping all nets connected.
The measured C31 supply trace grew from 1.050 mm on one layer to
1.559 mm of planar copper and one via. The path measurement for C14
was unavailable because that supply net contains a custom copper pad.

A separate trial used 1.5 mm **body-to-pin** limits for the same two
capacitors. A 0.1 mm placement grid and zero routing halos on C14, C31,
U7 and U8 produced gaps of 1.507501 mm for C14 and 1.232500 mm for C31.
In a private copy of the routed board, moving only C14 by 0.1 mm towards
U8 reduced its gap to 1.407501 mm. All track and via geometry stayed in
place. Independent readback passed all 28 pose, face and distance checks.
KiCad found zero opens in both copper states. The original-schematic
check still found the two mounting-hole parity issues and U1's inherited
footprint-type error. As a negative control, moving C14 by 2 mm from
its initial pose disconnected two items and introduced shorts. Both
trials require native verification after moving the footprint.

### RP2040 motor-controller interface

The [motor-controller brief](rp2040-motor-interface-mechanical.json)
fixes the 21 connectors, four switches and two mounting holes. The two
plug-in modules can move while their declared outlines stay clear of each
other on the same face. The four-layer board has two RP2040s that
exercise fine-pitch escape routing. The archived PCB supplies the outline,
footprints and layer stack. Its original copper provides an independent
reference after routing. [Source revision](https://github.com/Twisted-Fields/rp2040-motor-controller/tree/d51dc5441ad7371df24abd6f7f8783ccf11461ff/RP2040_base)

```sh
case_name=Twisted-Fields__rp2040-motor-controller__RP2040_base
archive=benchmarks/real/external
schematic="$archive/github-sch/$case_name/repo/RP2040_base/RP2040_base.kicad_sch"
board="$archive/github/$case_name/RP2040_base.kicad_pcb"
PCB_WORK_RATE=3.15e6,150e6 RAYON_NUM_THREADS=2 \
  /usr/bin/python3 benchmarks/from-schematic/run.py build/motor-interface \
  --schematic "$schematic" --constraints benchmarks/from-schematic/rp2040-motor-interface-mechanical.json \
  --layers 4 --outline-board "$board" --footprints-from "$board" --stackup-from "$board" \
  --keep-footprint-clearances --total-seconds 600 --binary target/release/pcb-maker
```

In the original 600-second run with only the 27 fixed poses and sides,
29 parts had no legal seat. Giving the
placer permission to move the 154 passive components named in the
recovery trial cleared the placer's unseated count. Thirteen capacitors
and three inductors moved to the back, and all 27 held poses matched.
KiCad still reported 40 opens on 31 nets. Every one of those nets had
an isolated pad on U15 or U19. The designer's board connected all 36
such pads with tracks on the front. Check both device pin access and
native schematic parity when comparing future runs of this case.

That result fails a separate assembly check. U1 and U2 are plug-in driver
boards. Each footprint's back courtyard covers its two connector rows.
Its closed drawing on `Eco2.User` matches the four straight edges and
four rounded corners of the actual daughterboard. Applying the returned
poses to the daughterboard perimeter gives **1,460 mm² of overlap**.
The author's mounting poses leave a 1.3 mm gap. Using three pin-distance
constraints for the crystal and flash lowered opens from 40 to 25, but
still left 1,458 mm² of overlap. Both layouts fail the assembly comparison.
[Driver PCB at the pinned revision](https://github.com/Twisted-Fields/rp2040-motor-controller/blob/d51dc5441ad7371df24abd6f7f8783ccf11461ff/driver_module/driver_module.kicad_pcb)

With the original 27 fixed positions and rear-passive permissions, the
placer can use one seed to find a pair of module positions first, then
arrange the other footprints around them. It seated all 321 footprints,
placing U1 at (138.0, 78.3) mm and U2 at (134.1, 114.3) mm, both at 0°.
Independent KiCad readback verified the 27 fixed positions, footprint
geometry, outline and stackup. Applying the poses to the daughterboard
Edge.Cuts gave zero overlap and a **0 mm edge gap**. The two perimeters
touch, as permitted by the supplied non-overlap constraint.
Rescoring the same saved board with an explicit `min_gap_mm: 0.5`
rejected the touching pair. The same check accepted the author's
1.3 mm gap on the board with both module positions held.

A cold import and layout with a 0.5 mm minimum gap and 27 held positions
placed U1 at (139.1, 79.6) mm and U2 at (137.3, 116.2) mm. The physical
daughterboard perimeters were 0.6 mm apart on the returned PCB. With
via cost 0.5 the run charged 539 modeled seconds, used 575 vias and
5,907 mm of track, and saved its zone fills within the layout budget.
Independent checks against the original schematic measured 27 opens,
eight hole-clearance errors and zero parity findings both as saved
and after refill. All 27 holds and all footprint geometries matched.

Following the verdict's alternate-placement command with
`--placement-seed 101` reduced refilled opens from 27 to 17 and the
via count from 575 to 532. Track length increased from 5,907 to
6,039 mm. Both layouts requested 600 modeled seconds. The retry charged
645 and deferred publishing its fills. Its separate fill handoff
reduced stored opens from 131 to 17. The independent check of the
filled PCB confirmed 17 stored and refilled opens, eight raw errors,
zero original schematic parity findings and the same 0.6 mm module gap.

On this seed, U19:47 to R70:2 (`/USB_DP_2`) remained open. Repeating
the layout with `diff_pairs: true` connected all four MCU USB conductors
and published its fills. Both native checks still counted 17 opens
elsewhere, eight raw errors and zero original schematic parity findings.
KiCad measured 0.0825 mm skew and 18.6584 mm of uncoupled length on the
second USB pair in a separate copy with diagnostic measurement rules.
The author board measured 6.1322 mm uncoupled on this pair.
Enabling only the placer's `pair_rows` also connected the four USB
conductors, with 21 board opens. Enabling both options gave 25 opens
and two additional dangling-via findings. All four arms met the
0.5 mm module gap and kept the 27 fixed positions. See
`build/rp2040-motor-interface-20261120/paired-layout-101/comparison.json`.

In both 17-open layouts, every missing connection in the refilled KiCad
report touched U15 or U19. Only U19 pad 55 appeared in all four arms.
Its flash connection to U13 pad 2 (`/QSPI_SD1_2`) had no routed copper
in any arm. The corresponding U15-to-U12 flash connection routed in
all four. In the two 17-open layouts both channels had identical relative
footprint poses and the same 9.563 mm pad-to-pad distance. These two
channels provide a placement-matched case for studying escape congestion.
On either 17-open result, `inspect-routing-run --terminal U19:55`
reports U15:55 to U12:2 as the unique translated routed match, at an
offset of 8.8 mm in X and 18.7 mm in Y from the routed to the open pair.
The 21-open and 25-open arms have different relative poses and omit
the match. An independent KiCad connectivity check confirmed both nets'
states and the endpoint poses in all four layouts. See
`build/rp2040-motor-interface-20261120/channel-diagnosis/audit.json`.

A fixed-placement comparison used the same schematic import, 1,081 pads,
two inner planes, router settings and 145-second routing budget. The
automatic placement left 41 native opens. Transferring the archived
designer's 321 poses and starting again with zero tracks left 40.
Specifying rear access for the 62 test points and 15 solder jumpers,
and letting the placer choose their positions, left 36 opens and one
new dangling-track error. All 27 mechanical holds and pad assignments
survived. These scores are from fresh KiCad refills.

A separate control on the archived poses measured how much connectivity
the designer placed in the zones. All 264 named nets had the same pad
contacts in the fresh import. Transferring the 4,536 tracks, eight arcs
and 553 vias onto the two default planes left 80 opens. Adding just the
designer's 19 × 46 mm `GND_ISO` zone on the two outer layers closed 32.
With all the original zone definitions KiCad found zero opens, one
item-not-allowed error and zero schematic-parity findings. The authored
copper served as a diagnostic reference in this comparison. Starting
again from the archived poses with just the authored zone outlines, no
tracks or vias, and no saved fills left 49 opens at the same routing
budget. The reference zone shapes alone did not reproduce the designer's
result.

A separate early-routing trial selected two-terminal groups by footprint
pair. It disabled other searches in memory, keeping their actual pads,
net assignments and obstacles. The temporary project received only the
selected groups' tracks and vias. Exact KiCad connectivity partitions
confirmed every selected pad pair closed and preserved the original
copper components. Native refills also checked for new errors. The normal
`Keep` continuation deducted the prefix work, including four native checks.

| Fixed placement | Requested work (s) | Actual work, ordinary → early (s) | Refilled opens, ordinary → early | Raw errors, ordinary → early |
| --- | ---: | ---: | ---: | ---: |
| RP2040 motor, both MCU-to-flash links | 160 | 170.575 → 169.957 | 33 → 25 | 8 → 8 |
| ICEpi Zero, FPGA-to-microSD link | 120 | 123.617 → 125.657 | 32 → 62 | 2 → 2 |

All ten flash signals and seven microSD signals closed in the respective
early runs. The ICEpi result shows why a trial that closes a group still
needs a whole-board comparison. See `build/link-group-native-prefix-20261120/`.

Translating the first channel's routed SD1 copper onto the completed
second channel closed its open but introduced shorts against existing
copper in both 17-open layouts. The additive repair rejected both trials.
A separate routing-only comparison started from the same placed PCB
with a 160-modeled-second budget. Reserving that translated route (15
segments and two vias) before routing the rest of the board gave 22
refilled opens, versus 33 in the unseeded control. It also left a new
dangling `/CAN0_N` segment on B.Cu. Both arms retained the eight inherited
hole errors and had zero parity findings. See
`build/rp2040-motor-interface-20261120/paired-layout-101/flash-first/comparison.json`.
A replay with the fallback-track cleanup removed the dangling CAN
segment. An independent fill and reopen confirmed 22 opens, the eight
inherited errors and zero parity findings. The stored and refilled
copper partitions agreed on 286 components across 264 nets. See
`flash-first/independent-overlap-filled/fill-handoff.json` in the same run.
At a 260-modeled-second budget the same seed still connected the flash
net, but this time the control finished with 24 refilled opens and the
seeded run with 26. The latter also had a new dangling GND track. Both
had zero parity findings. This budget reversal limits the case for
reserving the route by default.

A cold routing experiment took the five two-terminal signals between
each MCU and its flash chip. The private first pass retained all pad
obstacles and routed just those ten connections together. On the
original, copper-free placement, KiCad accepted all ten generated routes
without an additional error. Keeping those routes during the same
160-second whole-board run left 25 opens, versus 33 for the control.
The first pass cost another 2.7 modeled router seconds. After publishing
fills, the original-project check found all flash connections closed,
eight inherited DRC errors and zero parity findings both on disk and
after refill. See `paired-layout-101/flash-joint/verified.json` under
`build/rp2040-motor-interface-20261120/`.

Routing only the first channel and translating it to the second cost
0.9 modeled router seconds. One translated via collided with a back-side
capacitor at C13, so the trial discarded that entire net before the
whole-board run. Reserving the other nine connections resulted in 24
opens and a new dangling USB via. These runs use generated copper from
the placed input and require no prior layout.

With 260 modeled seconds for the whole-board pass, routing both flash
groups early tied the unseeded run at 24 refilled opens, eight inherited
errors and zero parity findings. The early group's flash signals remained
connected. The 160-second advantage did not carry over to this budget.

A routing-only comparison started both routers from the same copper-free
placement of the first 0.5 mm layout. pcb-maker with via cost 0.5 and a
260-modeled-second router budget took 223 wall seconds and left 26
native opens. Freerouting 2.2.4 used its 260-wall-second job limit
and left 240 after refill. Both were configured for two routing threads.
We checked the external router's 1,308 tracks and 181 vias on an exact
copy of the input so all 321 footprint geometries and the original project
rules matched. Both refilled PCBs had eight raw errors and zero original
schematic parity findings. The saved boards had 26 and 431 opens
respectively. The external run reached its time limit; this comparison
measures bounded routing on one placement. The reports are in
`build/rp2040-motor-interface-20261120/freerouting-gap-half/`.

The ordinary-cost layout on this placement had 41 refilled opens, 362
vias and 5,946 mm of track. Reimporting the original schematic with the
shared driver footprint ID and lowering the via cost to 0.5 produced
21 refilled opens, 537 vias and 5,811 mm of track on the 27-hold brief.
The low-cost result retained all 27 fixed positions and all footprint
geometries. It had eight USB footprint errors and zero original schematic
parity findings. Both 600-second layouts deferred saving fills. The
separate fill handoff reduced stored opens from 146 to 41 on the ordinary
board and from 158 to 21 on the low-cost board.

A separate control fixed U1 and U2 at the author's mounting positions
with the corrected thermistor pin assignments. It preserved all 29 fixed
poses, placed all 321 parts and cleared the daughterboard collision.
The stored and refilled PCB had 46 open connections and eight
hole-clearance errors inside the two USB footprints. Native parity
passed against the packaged schematic. Against the original schematic
there were two footprint library ID mismatches on the modules and zero
pad-net mismatches. This control
uses two additional mechanical inputs beyond the movable-module brief.

On the same 29-mount brief, applying the verdict's `via_cost_retry`
patch (`{"tuning":{"via_cost":0.5}}`) reduced native opens from 46 to
9 in both saved and refilled checks. The layout race selected a different
placement from the same seed pool. Modeled work went from 522 to 548
seconds, via count from 373 to 585, and track length from 5,941 to
6,649 mm. All mounts and module outlines still passed the returned-board
check. Four signal nets and two supply nets remained open.

A fresh import retaining the driver's original shared footprint library
ID reproduced the nine opens, 585 vias and 6,649 mm of track. Against
the original schematic, KiCad now found zero parity findings, both as
saved and after refill. All 29 holds and the physical footprints passed
the independent comparison. The eight USB footprint errors remained.

To isolate the router setting, we kept each of the two selected
placements fixed and routed it twice with 260 modeled seconds requested
per run. At via costs 8 and 0.5, KiCad found 37 and 14 refilled opens
on the ordinary placement, and 36 and 15 on the alternate placement.
The two lower-cost arms used 456 and 441 vias, up from 383 and 379.

Two more trials kept the same 29 mounts and ordinary via cost. Moving
12 rear-permitted capacitors near U15 and U19 to the back reduced
refilled opens from 46 to 37, with the same eight hole-clearance errors.
KiCad readback confirmed all 12 face changes. Applying the project's
0.65 mm via with a 0.25 mm drill to 18 two-terminal signal nets instead
gave 42 opens and one additional dangling-track error. Both layout runs
deferred saving their zone fills. Before the separate fill handoff the
saved boards had 134 and 140 opens respectively. After the handoff KiCad
reopened them with 37 and 42 opens, matching the refilled checks.

To replay the passive-only permission, extract the proposed references
from the first verdict and repeat the command in a fresh output directory
with `--placer-config build/motor-interface-passives.json`.

```sh
jq '{allow_back: [.rear_placement_recovery.allow_back_additions[] | select(test("^(C|R|RN|L)[0-9]+$"))]}' \
  build/motor-interface/layout/verdict.json > build/motor-interface-passives.json
```

### Sisu handset mechanical transfer and keypad escape

The six-layer Sisu handset has 283 schematic footprints and 13 board-only
assembly features. For a copper-free import, supply the archived PCB with
`--mechanical-from`, `--footprints-from` and `--stackup-from`. Select the
seven plated mounting holes H1–H7 using a separate
`--mechanical-footprint <footprint-uuid>=GND` for each. Select the six
fiducials FID1–FID6 using their bare footprint UUIDs. The resulting board
has all 296 footprints, preserves the original pad UUIDs, mounting-hole
net assignments, positions and faces, and starts with zero routed copper.
The six optical marks retain their 0.75 mm copper lands, 1.5 mm mask
openings and 0.475 mm local clearance. [Pinned handset project](https://github.com/briskspirit/Sisu_SSE-9/tree/ca6867498d6f45a20581c37e4d33b79bcfcff7cb)

An enclosure brief held the original locked parts, switches, connectors,
mounting holes and fiducials. The remaining parts could move on their
original faces. The 240-modeled-second layout left 19 native opens.
Sixteen were on the enclosed centre contacts of SW2–SW17. On the archived
PCB, each of those 16 contacts has a same-net 0.4 mm via with a 0.3 mm
drill entirely inside its copper land. To test this as an explicit
fabrication choice, the comparison gave those 16 switches individual
`{"part":"SW2","via_mm":[0.4,0.3]}` entries in the router's `via_in_pad`
array, substituting each reference. LS1 remained outside this policy.

On the same copper-free placement and 145-modeled-second routing budget,
the ordinary router left 22 opens. The switch policy reduced that to six.
Independent KiCad checks under the original project rules found five
errors on the scoped route, six on the ordinary route, and zero schematic
parity findings in both. The scoped route also produced 11
`track_not_centered_on_via` warnings. In a fresh placement, the default
opposite-face exclusion for the switch vias prevented the large rear
modem U11 from fitting. In the archived design seven of the centre lands
lie within that modem's rear courtyard, so opposite-face clearance is a
separate mechanical decision.

In a [second mechanical brief](sisu-mechanical.json) U11 also held its archived pose. The ordinary layout
left 18 opens after refill and 178 as saved. With filled and capped vias
allowed on SW2–SW17 and `placer.tuning.via_in_pad_far_side` set to
`false`, the layout retained all 53 held poses and all 296 faces, saved
its pours and left two opens in both native checks. One connects to the
speaker, the other to the modem reset. KiCad also reported one differential
pair uncoupled-length error and one plated hole inside a courtyard.

The archived speaker also has a centred 0.4 mm via inside LS1 pad 1.
Adding LS1 as a seventeenth explicit part connected the speaker. The
saved and refilled result retained the same 53 poses, had one open on
the U11 reset, two active differential pair uncoupled-length errors,
one excluded courtyard finding and zero schematic parity findings. The additional
speaker admission changed the routing elsewhere as well.

The ordinary layout now repairs the reset connection automatically under
the same 17-part policy. A cold run took 216.54 modeled seconds of the
240-second budget. Independent KiCad checks found zero opens and zero
schematic parity findings both on the delivered file and after refilling
it. Readback matched all 296 requested faces and all 53 fixed poses.
The two differential pair uncoupled-length errors keep the layout incomplete.
KiCad also lists the excluded courtyard finding.

To replay the 17-part policy from a fresh checkout of the pinned handset
project, set `source` to its project directory and `work` to a fresh
absolute path. The commands use Bash, jq and KiCad's Python bindings.
Run them from the pcb-maker repository root.

```bash
name=Sisu_SSE-9
board="$source/$name.kicad_pcb"
mkdir -p "$work/project"
cp benchmarks/from-schematic/sisu-mechanical.json "$work/project/constraints.json"
export RAYON_NUM_THREADS=2 PCB_WORK_RATE=3.15e6,150e6 PCB_MAKER_KICAD_USER_LIBS=1
export PCB_MAKER_CACHE_DIR="$work/cache" PCB_MAKER_LEDGER="$work/ledger.json"
/usr/bin/python3 - "$board" > "$work/mechanical-features" <<'PY'
import pcbnew, sys
board = pcbnew.LoadBoard(sys.argv[1])
by_ref = {fp.GetReference(): fp for fp in board.GetFootprints()}
for prefix, count, binding in [("FID", 6, ""), ("H", 7, "=GND")]:
    for i in range(1, count + 1):
        print(by_ref[f"{prefix}{i}"].m_Uuid.AsString() + binding)
PY
args=()
while read -r feature; do args+=(--mechanical-footprint "$feature"); done < "$work/mechanical-features"
pcb-maker import-kicad-netlist "$source/$name.kicad_sch" "$work/project" "$name" \
  --mechanical-from "$board" --footprints-from "$board" --stackup-from "$board" "${args[@]}"
jq -n '{via_in_pad:[("LS1",(range(2;18)|"SW"+tostring))|{part:.,via_mm:[0.4,0.3]}]}' > "$work/router.json"
jq -n '{total_seconds:240,placer:{constraints:"constraints.json",tuning:{via_in_pad_far_side:false}}}' > "$work/layout.json"
pcb-maker layout-kicad-board "$work/project" "$name" "$work/layout" "$work/router.json" "$work/layout.json"
```

The reset pad also exposed an error in how the router assigned solder
mask openings to nets. A diagonal WCI_RX track's bounding box crossed
the opening at U11:R13, although the track itself missed it by more
than 1 mm. Checking the actual copper shapes recovered a legal launch
from R13. The complete route still left RESET open. A placement trial
that minimized the Q4:3 to U11:R13 distance shortened it from
15.99 mm to 10.42 mm and left two additional regulator connections
open. The one-open result remains the stronger layout under this brief.

On that one-open layout, a generated-copper repair closed RESET in
43.49 s on a fresh end-to-end replay. It moved the existing 0.4 mm
filled and capped via 1.90 mm within SW9 pad 1 and reconnected its
inner-layer junction. All 296 footprint poses stayed fixed. The command
found the via and its new position using the generated layout and
the copper-free placement.

```bash
pcb-maker repair-kicad-opens "$work/layout/result" "$name" --relocate "$work/recovered" \
  --generated-from "$work/layout/placed" --net '/4G LTE/RESET'
```

Independent checks with the archived project rules found zero opens
and zero schematic parity findings both as saved and after refill.
Two active errors remain. The modem USB pair has 36.5706 mm of uncoupled
length and the RP2354B USB pair has 4.2980 mm, against a 2 mm limit.
The plated ground hole H7 lies inside the J7 courtyard. KiCad reports
this third finding as an excluded error. The archived board has that
same finding, and the mechanical brief holds both H7
and J7 at their archived poses. The fabrication
assumption for this comparison includes filled and capped switch and
speaker vias under the specified opposite-face components.

On the completed board, C75 pad 1 is 1.868 mm from U2 pin 10 in the
board plane. Its supply connection passed through C23 pad 1 and measured
8.978 mm along the traces. With the project libraries available,
`shorten-kicad-pad-path --pads C75:1 U2:10 --decoupling-shortcut`
reduced the measured span to 1.282 mm by adding nine tracks and one
through via. The comparison retained all 5,528 original non-zone board
forms, including 296 footprints, and all 38 checked project and library
files. Independent stored and refilled checks found zero opens, zero
parity findings, the same two active USB errors and 27 active warnings.
KiCad named a different pre-existing keypad track as one witness for the
same 0.042 mm connection-width warning.

### OpenMuscle FlexGrid sensor arrays

These two-layer flex boards each have 60 sensor footprints with four
groups of artwork per sensor. For the mating-pose comparison, hold every
sensor and connector at its original position, angle and face. The PCB
supplies the outline and land patterns. The imported board starts with
zero routed tracks or vias. [Pinned OpenMuscle project](https://github.com/Open-Muscle/OpenMuscle-FlexGrid/tree/912e6e615c1f123141933a465c620f1a636692f0/KiCad)

Set `board` to either `OM-60-Flex.kicad_pcb` or `OM-FlexGrid-Flex.kicad_pcb`
in the pinned project and set `work` to a new output directory.

```sh
mkdir -p "$work"
/usr/bin/python3 - "$board" > "$work/brief.json" <<'PY'
import json
import sys
import pcbnew

board = pcbnew.LoadBoard(sys.argv[1])
brief = {"version": 1, "move_all": True, "place": []}
for part in sorted(board.GetFootprints(), key=lambda f: f.GetReference()):
    ref = part.GetReference()
    pos = part.GetPosition()
    brief.setdefault("back" if part.IsFlipped() else "front", []).append(ref)
    brief["place"].append({"part": ref, "origin": "absolute", "x": pos.x / 1e6,
                           "y": pos.y / 1e6, "angle": part.GetOrientationDegrees()})
brief["place"].sort(key=lambda pose: not pose["part"].startswith("JP"))
print(json.dumps(brief))
PY
PCB_WORK_RATE=3.15e6,150e6 RAYON_NUM_THREADS=2 PCB_MAKER_KICAD_USER_LIBS=1 \
  /usr/bin/python3 benchmarks/from-schematic/run.py "$work/layout" \
  --binary target/release/pcb-maker --schematic "${board%.kicad_pcb}.kicad_sch" \
  --constraints "$work/brief.json" --layers 2 --outline-board "$board" \
  --footprints-from "$board" --keep-footprint-clearances --total-seconds 240
```

| Exact mating poses | Routed nets | Native opens | Native errors, returned / archived | Vias, returned / archived | Track length, returned / archived |
| --- | ---: | ---: | ---: | ---: | ---: |
| OM-60-Flex | 21/21 | 0 | 0 / 0 | 104 / 120 | 3,769 / 4,070 mm |
| OM-FlexGrid-Flex | 19/19 | 0 | 19 / 19 | 148 / 120 | 3,639 / 5,177 mm |

KiCad reopened and refilled each returned board under the archived project
rules. Each retained all 60 sensor poses, their pad geometry, 240 group
memberships and the exact connector poses, with zero schematic parity
findings. The 19 edge-clearance errors on OM-FlexGrid-Flex match the
archived connector pad descriptions and locations in both checks.
Allowing J1 to move within a 3 mm margin around its
original courtyard produced zero native errors and 72 vias, but turned
the connector 180° and moved its mating interface. The original project
rules and exact connector pose govern the table. On OM-60-Flex the
archive uses 21 short sections of 0.5 mm track. The returned board
uses 0.2 mm throughout.

An exact spatial index speeds up the pad-escape check on the detailed
sensor copper polygons. In a matched cold OM-60-Flex run the layout
subprocess took 78.8 s, down from 360.3 s. The complete workflow took
151.5 s, down from 387.6 s. Elapsed times are single observations on
a shared host. Both arms performed 10,225,354 A* expansions and wrote
byte-identical PCBs. In two isolated measurements on each sensor board,
geometry preparation fell from 92–98 s to 9–10 s. The Sisu handset and
Neotron-Pico BGA controls also produced identical routing geometry.

With the 60 sensor poses fixed and both OM-60-Flex connectors movable
inside regions, the layout evaluated ten distinct connector placements
and prepared routing geometry 14 times. Layout wall time fell from
1,606 s to 320 s. Both runs wrote the same PCB, with all 21 nets
connected, 101 vias and 3,723 mm of track. These two binaries also
differed in other changes. The fixed-connector comparison above used
matched source snapshots differing only in the index.

### OpenESC outlines inside electrical footprints

Both OpenESC boards store the finished perimeter inside a footprint with
38 electrical pads. The schematic supplies the connections. The mechanical
donor supplies the owner's position, angle and side as well as the outline.
Other footprints with ordinary mounting slots can still move.
[OpenESC 20x20 source](https://github.com/OpenDrone-hw/OpenESC-20x20/tree/104ea51edfd88c2c381f30bb108f886acb7d6bab/hardware)
and [OpenESC 30x30 source](https://github.com/OpenDrone-hw/OpenESC-30x30/tree/683f175987b964f1cab83f3b54ac2a938042afcc/hardware).

Set `source_dir` to the upstream `hardware` directory and `board` to
`4in1-mini` or `4in1`. Run the import in a fresh directory.

```sh
pcb="$source_dir/$board.kicad_pcb"
pcb-maker import-kicad-netlist "$source_dir/$board.kicad_sch" "build/openesc-$board" "$board" \
  --mechanical-from "$pcb" --footprints-from "$pcb" --stackup-from "$pcb"
```

| Board | Held perimeter owner | Native edge shapes | Interior cutouts | Schematic footprints | Physical pads |
| --- | --- | ---: | ---: | ---: | ---: |
| 4in1-mini | U4 | 107 | 0 | 170 | 1,051 |
| 4in1 | U3 | 141 | 2 | 173 | 953 |

KiCad readback matched every donor outline line, arc, Bézier and circle
at the original position, including the two interior circles on 4in1.
Each import kept one board rule area, and all pads retained the same
net assignments as an independent schematic-only import. Both imported
boards started with zero tracks and vias.

KiCad's Edge.Cuts Gerber exports also matched the donors after placement
and after routing. Comparing the plotted strokes by endpoints, arc center
and direction at 1 nm output precision matched all 1,666 strokes on Mini
and all 1,531 on 30x30.

For a 30x30 layout trial, we held J1 at its connector position, assigned
157 schematic footprints to the rear according to the donor PCB
and left all other component positions free apart from U3. At a budget of
180 modeled seconds the placer seated all 173 parts. The routed board
retained all 141 perimeter shapes and both holes. KiCad reported 91
opens as saved and 40 after refill, with zero DRC errors and 16
schematic parity findings in both checks. The import report identifies
all 16 as symbol pins marked no-connect that have no matching pad on
the selected footprint. KiCad reports the same missing pins on the donor.

On 4in1-mini, treating U4's rows of contacts as one solid body left
26 parts unplaced. When placement failed, the placer retried U4 using
the areas occupied by its contacts. The retry cost 38.84 modeled
seconds and seated all 170 parts while keeping U4 and J1 at their
specified poses. At the same requested 180-second budget, the full
run used 187.59 modeled seconds. KiCad found 136 opens on the saved
board and 93 after refill. Both checks reported zero DRC errors and
eight missing no-connect pins.

On the 30x30 result, `drc-kicad-board --save-filled` produced a board
with 40 opens both as saved and after refill. This run used installed
KiCad libraries (`PCB_MAKER_KICAD_USER_LIBS=1`). The native comparison
matched 3,320 pads, tracks and vias across 234 connected components
on 194 nets. Both checks kept the same 16 schematic parity findings.

Routing the same 30x30 placement twice at a requested 90 modeled seconds
and 0.1 mm pitch gave 44 native opens with the ordinary via cost and 27
with `{"tuning":{"via_cost":0.5}}`. The two runs kept the same plane
policy, rules and footprint poses. Track length changed from 1983.95 to
1876.89 mm and via count from 189 to 274. An independent KiCad check found
zero errors, 16 schematic parity findings, one connection-width warning,
and four library mismatches in each run. On a full matched 180-second
layout, the same settings edit reduced refilled opens from 40 to 33
and as-saved opens from 91 to 79. Actual modeled work was 165.63 and
188.52 seconds. The higher-work run skipped publishing filled zones.

In an ordinary 180-second layout replay, automatic local repair used
three copper additions on `/ESC4/FBA`. Native connectivity merged two
of that net's three components while preserving all 3,318 existing
named copper objects and every other net's component membership. The
repair used 14.98 modeled seconds, including four native checks.
Independent KiCad checks measured 90 saved and 39 refilled opens, with
zero DRC errors and the same 16 parity findings. The earlier routed
artifact had two additional duplicate copper objects and produced the
same 40-to-39 improvement under standalone local repair. A separate
`+10V` trial was rejected because it produced a new connection-width
warning.

### WLED Matrix business card

The [WLED mechanical brief](wled-matrix-mechanical.json) fixes the 64
addressable LEDs, the antenna, two connectors, six switches and four mount
locations. The schematic supplies 138 footprints. The board supplies the
outline, stackup, footprint geometry, two mouse-bite footprints and four
board-only grounded mounts. The remaining schematic components can move.
The source is CDFER/Business-Cards revision
`1f6432d7b65871354a7358242df49a5025cb8e3c`.

```sh
case_id=CDFER__Business-Cards__WLED_Matrix
name="WLED Matrix"
corpus=benchmarks/real/external
pcb="$corpus/github/$case_id/$name.kicad_pcb"
sch="$corpus/github-sch/$case_id/repo/WLED_Matrix/PCB/$name.kicad_sch"
work="$PWD/build/wled-matrix-replay"
brief="$PWD/benchmarks/from-schematic/wled-matrix-mechanical.json"
pcb-maker import-kicad-netlist "$sch" "$work/project" "$name" \
  --mechanical-from "$pcb" --footprints-from "$pcb" --stackup-from "$pcb" \
  --keep-footprint-clearances \
  --mechanical-footprint 9c4387e0-ae55-48ed-8294-ebef1fa0c47e \
  --mechanical-footprint c918ca17-16f2-4c1e-a295-0b4ff5cb9576 \
  --mechanical-footprint bb17d66d-4a72-4a99-aa35-67a7604cee7b=GND \
  --mechanical-footprint c6ddd723-8dab-4e50-8bbf-d67321ebb905=GND \
  --mechanical-footprint d75ba32d-56c7-4933-b8b4-adfcc6feb859=GND \
  --mechanical-footprint e99bfb5c-739e-47f8-ada5-94dd9c2c0e67=GND
jq --arg brief "$brief" '.total_seconds=240 | .placer.constraints=$brief' \
  "$work/project/layout.json" > "$work/layout.json"
PCB_WORK_RATE=3.15e6,150e6 RAYON_NUM_THREADS=2 PCB_MAKER_KICAD_USER_LIBS=1 \
  pcb-maker layout-kicad-board "$work/project" "$name" "$work/layout" auto "$work/layout.json"
```

Seven of the mounts' 36 plated pads carry GND. The other 29 keep their
empty net assignments. An independent KiCad drill export reproduced all
36 hole centers and diameters. On the schematic import, preserving each
instance's fabrication text and schematic-backed component metadata
reduced original-schematic footprint ID mismatches from 129 to zero.
KiCad readback matched all component fields, pad geometry and assignments,
footprint graphics and poses between the two identity-handling versions.
The 240-second layout placed 144 footprints and retained every pose in
the brief. Independent checks against the untouched schematic reported
seven opens and zero parity findings in both modes, with 138 raw DRC
errors in the saved board and 136 after refill. See
`build/wled-matrix-schematic-20261120/independent-import/combined/verified-layout.json`.

With the same original project rules, KiCad found 364 open connections
on the placed board before routing. All 136 refilled DRC errors on the
returned board were already present at placement, matching by rule type,
item UUIDs, description and occurrence count. The three short findings
pair AE1's copper graphics with its pads. Those same three item pairs
also short on the designer's board. Trials with a full-board GND pour
and with the designer's three unfilled GND, +5V and VBUS pour outlines
each finished with eight opens after refill and the same 136 error count.

As a placement control, holding the remaining schematic components at
the designer's positions and routing from zero tracks produced three
opens, all at AE1. It also introduced five courtyard overlaps relative
to the 77-pose run. Both positions of AE1 pad 2 and the feed pad remain
open in this control. The native routing feedback identifies the antenna's
own unassigned copper at the feed pad as a clearance obstacle.

KiCad's footprint geometry explains why these connections need design
review. On both the designer and returned boards, AE1's three unassigned
front-copper shapes touch one another. One end touches the feed pad and
the other touches both GND pads. The same connected copper therefore
meets two named nets. The source footprint has no declared net tie.
See `build/wled-matrix-schematic-20261120/antenna-contacts/result.json`.

Reusing the 77-pose run's **placed** project supports a faster routing
comparison. The following retry used 118.1 modeled seconds and reduced
the seven opens to four. Both saved and refilled KiCad checks against
the original schematic reported 136 raw errors and zero parity findings.
The antenna feed, its two ground contacts and the MCU's `/Sea` connection
remain open. All 77 mechanical poses held.

```sh
echo '{"ladder_budget_seconds":145,"deadline_seconds":130,"seeds":1,"tuning":{"first_seed":5,"via_cost":0.8}}' > "$work/route-retry.json"
PCB_WORK_RATE=3.15e6,150e6 RAYON_NUM_THREADS=2 PCB_MAKER_KICAD_USER_LIBS=1 \
  pcb-maker route-kicad-board "$work/layout/placed" "$name" "$work/route-retry" \
  "$work/route-retry.json" --compare-incumbent "$work/layout/result"
```

The comparison reserves 14.4 modeled seconds for four KiCad checks. It
keeps both boards and lists each net that closed, gained an open or still
needs work in `verdict.json`. Here `/Sea` gained an open while GND
improved and three signal nets closed.

Using `"$work/route-retry.json"` in place of `auto` in the original
240-second layout command also preserved all 77 specified poses. That
run selected different poses for all 65 movable parts and finished with
six opens, 136 refilled errors and zero parity findings. Doubling the
original layout budget to 480 seconds with default routing gave seven
opens and 137 refilled errors. Moving R1 and R6 closer to their open MCU
pins closed those two nets, but opened other MCU connections and left
eight opens overall.

A local repair experiment closed the remaining MCU connections with all
144 footprints fixed. It started from the via-cost-only route, shifted
two internal bends of the generated `/Sea` trace by 0.050 mm, then
reconnected C52 using three GND tracks and a 0.50 mm via with a
0.30 mm drill, a size explicitly listed in the project. After
`--save-filled`, independent checks against the original project gave
three opens, 136 errors and zero schematic parity findings in both
saved and refilled modes. The three opens touch the AE1 antenna pads.
The 136 error identities match the four-open candidate. This experiment
supplied the bend change separately. The public `repair-kicad-opens
--relocate` replay checked all 2,436 original copper item IDs, verified
the GND connection and retained all three project footprint libraries
in the published output. See
`build/wled-matrix-schematic-20261120/independent-import/combined/verified-layout-collective-release.json`.

The automatic replay finds the bend adjustment from the placed input and
the routed board. Start from the same placement with the default seed
and the 0.8 via cost. The repair checks which copper the router created
since placement, tries bounded adjustments near the remaining GND open,
and reports the accepted copper changes in `relocated-open-repair.json`.

```sh
jq 'del(.tuning.first_seed)' "$work/route-retry.json" > "$work/route-low-via.json"
PCB_WORK_RATE=3.15e6,150e6 RAYON_NUM_THREADS=2 PCB_MAKER_KICAD_USER_LIBS=1 \
  pcb-maker route-kicad-board "$work/layout/placed" "$name" "$work/low-via" "$work/route-low-via.json"
pcb-maker repair-kicad-opens "$work/low-via" "$name" --relocate "$work/repaired" \
  --generated-from "$work/layout/placed" --net GND
pcb-maker drc-kicad-board "$work/repaired" "$name" "$work/refilled.json" --save-filled "$work/filled"
```

The independent replay tried six adjustments and completed the repair
in 30.9 seconds. The saved and refilled board again had three antenna
opens, 136 errors and zero parity findings. See
`build/wled-matrix-schematic-20261120/independent-import/combined/verified-layout-independent-auto.json`.

The same routing settings were compared with a single default seed on
two other placed boards, with the same 145-second budget per arm. KiCad
checked each result before and after refilling the zones.

| Fixed placement | Default opens, saved / refilled | Alternate opens, saved / refilled | Raw errors in either route |
| --- | ---: | ---: | ---: |
| RP2040 motor interface, 321 footprints | 42 / 40 | 16 / 16 | 8 |
| Eurorack PMOD, 151 footprints | 0 / 0 | 75 / 0 | 1 |

After refill, the motor alternate had 32 fewer gaps across the nets
that improved and one more gap on each of eight other nets. Varying one
setting at a time isolated the effect on this placement. Changing only the
seed reproduced the default PCB byte-for-byte. Changing only the via cost
to 0.8 reproduced the alternate PCB byte-for-byte. The alternate used
620 vias against the default's 395 and reduced track length from 6,822
to 6,677 mm. The two motor routes used 152.3 and 153.5 modeled seconds
respectively with the configured 145-second budget. On Eurorack
the alternate's saved gaps were all on GND
(54) and VDD (21). Running `drc-kicad-board --save-filled` on that
result and reopening the published board in KiCad gave zero saved
opens and the original footprint-type error. All Eurorack arms matched
the emitted schematic. Against the untouched archived schematic, KiCad
reported the same two footprint ID mismatches in every arm, including
the published result. The motor arms matched the untouched schematic.
Both comparisons kept all footprint poses, project rules and schematic
sheets. Within each board, the raw error identities matched in both
check modes. See `build/fixed-route-strategy-holdouts-20261120/comparison.json`.

### Osprey wireless keyboard

The [Osprey mechanical brief](osprey-mechanical.json) fixes 61 poses,
including the key switches, connectors, antenna, indicators and mounts.
It preserves the authored board face of all 160 footprints and leaves 99
positions for the placer. The schematic and two-layer board come from
`ebastler/osprey` revision `0765a0c27566816c6b0becd4da4c2e720eed36f9`.

```sh
case_id=ebastler__osprey__osprey_rev_a
name=osprey_rev_a
archive=benchmarks/real/external
sch="$archive/github-sch/$case_id/repo/$name/$name.kicad_sch"
board="$archive/github/$case_id/$name.kicad_pcb"
brief="$PWD/benchmarks/from-schematic/osprey-mechanical.json"
work="$PWD/build/osprey-replay"
mkdir -p "$work"
jq '{allow_back: .back}' "$brief" > "$work/placer.json"
PCB_WORK_RATE=3.15e6,150e6 RAYON_NUM_THREADS=2 PCB_MAKER_KICAD_USER_LIBS=1 \
  /usr/bin/python3 benchmarks/from-schematic/run.py "$work/run" \
  --binary target/release/pcb-maker --schematic "$sch" --layers 2 \
  --outline-board "$board" --rule-areas-from-outline --footprints-from "$board" \
  --keep-footprint-clearances --constraints "$brief" \
  --placer-config "$work/placer.json" --total-seconds 360
```

The import started with zero tracks. The result retained all 61 specified
poses, every board face and all six board and footprint rule areas.
KiCad reported two opens and zero parity findings against the untouched
schematic in both saved and refilled modes. The open connections were at
the antenna feed and PNT1's front GND pad. Refill found 51 raw errors,
including 45 footprint-type mismatches. The archived routed board had
zero opens after refill, 61 raw errors and 249 reported parity findings
under the same project, including 199 net conflicts at KiCad's reporting
limit. See
`build/osprey-schematic-workflow-20261120/verified.json`.

On the same imported project, `place-kicad-board` reduced 63 fixed-part
overlap warnings to three unverified bounding-box hits. Both placed PCBs
were byte-identical. KiCad measured separated courtyards on both faces
for all 60 omitted pairs. Each of the three remaining pairs lacks one
reverse-side courtyard. A control brief also held C17 at its designer
pose beside J5. KiCad measured 0.108 mm² of intersecting back courtyards,
and the placement verdict retained that specific overlap. See
`build/osprey-schematic-workflow-20261120/held-warning-fix/verification-v2.json`.

PNT1's front rule area prohibits tracks and allows zone fill. Its open
GND pad is at (6.100, −0.835) mm. On a copy of the returned board, a
front GND zone from (4.0, −2.5) to (9.0, 6.3) mm connected that pad to
existing GND copper. No tracks or vias were moved or added. After
`drc-kicad-board --save-filled`, both checks under the original project
rules reported one open at AE1, zero parity findings and the same 51 raw
error identities. Extending GND over the entire front also connected the
pad, but added a starved thermal relief at a different PNT1 pad. See
`build/osprey-schematic-workflow-20261120/verified-tight.json`.

A second trial derived the region directly from the open pad and the
nearest GND via on its layer. Expanding their combined copper bounds
by 0.30 mm gave a 3.025 × 10.465 mm rectangle. After refill it produced
the same single antenna open and 51 errors. The design-rule inspection
also traced the antenna feed's open pad through its back-copper polygon
to the GND pad named in KiCad's short. The designer board had no open
at the feed, so the same inspection did not attribute an open to its
antenna. See `build/osprey-schematic-workflow-20261120/artwork-verification.json`.

The local-pour repair accepts the returned PCB and the selected pad.

```sh
pcb-maker repair-kicad-opens "$work/run/layout/result" "$name" \
  --local-pour "$work/pour-repair" --pad PNT1:MP
```

The command selected the one open land among three pads labelled
`PNT1:MP`, joined its GND component to a retained via, and published
the checked fill. Independent KiCad readback preserved all 3,809 tracks
and vias, 55 board drawings, 160 footprint models and three board-level
rule areas. The single added zone reduced two opens to one in both
saved and refilled checks, with unchanged error and warning identities.
Repeating the command on the repaired project produced an identical PCB.
See `build/osprey-schematic-workflow-20261120/verified-independent-pour.json`.

### USB Cable Tester MK2

The [mechanical brief](usb-cable-tester-mechanical.json) holds the five
edge USB receptacles, the CR2032 holder, both product mounting holes,
and the 17 visible indicator LEDs at their archived positions. The
remaining 55 schematic parts can move, including the tooling holes.
The archive is CDFER/Business-Cards at
`1f6432d7b65871354a7358242df49a5025cb8e3c`. The original board has a
74 × 48 mm outline made from 14 board graphics and 11 connector
Edge.Cuts graphics. The connector graphics complete the outline when
their absolute holds are applied. The schematic project supplies the
rules and both copper layers.

```sh
case_id=CDFER__Business-Cards__USB_Cable_Tester__PCB
corpus=benchmarks/real/external
donor="$corpus/github/$case_id/USB Cable Tester.kicad_pcb"
source="$corpus/github-sch/$case_id/repo/USB_Cable_Tester_MK2/PCB/USB Cable Tester.kicad_sch"
brief=benchmarks/from-schematic/usb-cable-tester-mechanical.json
LC_ALL=C PCB_WORK_RATE=3.15e6,150e6 RAYON_NUM_THREADS=4 \
  benchmarks/from-schematic/run.py build/usb-cable-tester \
  --schematic "$source" --constraints "$brief" --layers 2 --total-seconds 240 \
  --outline-board "$donor" --footprints-from "$donor"
benchmarks/from-schematic/run.py build/usb-cable-tester-score \
  --schematic "$source" --constraints "$brief" --layers 2 \
  --outline-board "$donor" --footprints-from "$donor" \
  --candidate-board "build/usb-cable-tester/layout/result/USB Cable Tester.kicad_pcb"
```

The flat import recovered all 80 footprints and matched the archive's
218 shared named terminals and 54 connected terminal groups. The archive
omits tooling hole H6 and includes extra artwork. Direct KiCad checks
found ten DRC errors, two opens and 17 parity findings on the archived
designer board in both stored and refilled mode.

The November 19 run placed all 80 footprints and routed all 54
connections in 114.98 modeled seconds, with 47 vias and 1565.70 mm of
copper. Independent checks confirmed all held poses and faces, the
original rules, footprint geometry, and all 25 perimeter graphics.
Both stored and refilled KiCad checks gave zero opens and zero
schematic parity findings. The result failed strict scoring on one
courtyard overlap between held footprints H4 and J2. The overlap also
appears in the copper-free placement and the original board. The
verdict directs review to those footprint courtyards and held poses.

### ICEpi Zero v1.4

This four-layer FPGA board has 163 schematic footprints. The benchmark
holds the six connectors, four mounting holes and two switches at their
mechanical interface positions. It keeps the FPGA on the front and allows
the placer to choose the side of other movable SMDs. The original board
supplies eight edge primitives, 30 rule areas, the stackup and missing
footprint definitions. The reference run used the installed KiCad libraries
for 140 footprints and the board for the remaining 23.
[Pinned upstream design](https://github.com/cheyao/icepi-zero/tree/8cc3895e2a8be2fe4eb071726c62972d6032354e/hardware/v1.4)

Set `case_dir` to the upstream `hardware/v1.4` directory, then run from
the pcb-maker checkout.

```sh
work=build/icepi-zero-cold
mkdir -p "$work/source"
cp "$case_dir"/*.kicad_sch "$case_dir/icepi-zero.kicad_pro" "$case_dir/icepi-zero.kicad_dru" "$work/source/"
donor="$case_dir/icepi-zero.kicad_pcb"
pcb-maker import-kicad-netlist "$work/source/icepi-zero.kicad_sch" "$work/project" icepi-zero \
  --footprints-from "$donor" --mechanical-from "$donor" --stackup-from "$donor"
cp benchmarks/from-schematic/icepi-zero-mechanical.json "$work/project/constraints.json"
jq '.total_seconds = 180 | .placer.allow_back = ["*"]' "$work/project/layout.json" > "$work/layout-180.json"
PCB_WORK_RATE=3.15e6,150e6 RAYON_NUM_THREADS=2 pcb-maker layout-kicad-board \
  "$work/project" icepi-zero "$work/layout" auto "$work/layout-180.json"
```

In the November 19 control, automatic side selection put the memory chip U11
on the rear. The placer reported a conflict with the FPGA and stopped with
U1 and U11 unplaced. With automatic side recovery, the placer retried U11
on the front and placed all 163 parts. The retry cost 30.13 modeled seconds
within the 180-second run. After refill, KiCad reported 104 opens. Readback
matched all 12 held poses, eight edge primitives and 30 rule area IDs.

Adding U11 to the brief's `front` list before starting also placed all 163
parts and left 79 opens at the same budget. Both routed boards had zero
schematic parity findings and two copper-edge errors at J6 pad 11. These
errors also occur on the donor, where the pad touches the edge against a
0.25 mm clearance requirement.

#### Comparing FPGA starting positions

The `placer.signal_anchor_seed` option estimates a starting position and
angle from signal connections to held parts. The FPGA can move during
placement. To isolate this option on ICEpi, put U11 on the front in both
runs. Use the same copper-free import and run from the checkout that
created `$work`.

```sh
jq '.front += ["U11"]' "$work/project/constraints.json" > "$work/project/with-memory-front.json"
jq '.placer.constraints = "with-memory-front.json"' "$work/layout-180.json" > "$work/plain.json"
jq '.placer.signal_anchor_seed = true' "$work/plain.json" > "$work/anchor.json"
for variant in plain anchor; do
  PCB_WORK_RATE=3.15e6,150e6 RAYON_NUM_THREADS=2 pcb-maker layout-kicad-board \
    "$work/project" icepi-zero "$work/$variant" auto "$work/$variant.json" || [ "$?" -eq 1 ] || exit 1
done
```

At a requested budget of 180 modeled seconds, independent KiCad refill
checks gave the following counts. The footprints remained movable except
for the mechanical holds specified by each input.

| Board | Held poses | Ordinary opens | Signal-seed opens | Raw DRC errors in either run |
| --- | ---: | ---: | ---: | ---: |
| ICEpi Zero | 12 | 79 | 61 | 2 |
| Glasgow revC3 | 14 | 70 | 68 | 4 |
| One-Air-Max | 6 | 0 | 1 | 24 |

All held positions, angles and sides matched, and KiCad found zero
schematic parity issues on every board. The option remains off by default.

With `--placement-seed 101` and the same requested budget, Glasgow went
from 62 ordinary opens to 44 with the signal seed. Its measured work
rose from 183.77 to 197.14 modeled seconds. On ICEpi, both variants
left 58 opens and used 145.70 and 141.78 modeled seconds respectively.
Raw errors and parity counts matched the first seed.

A subsequent local repair closed the One-Air-Max signal seed's last
open on `/Solar_module/ILIM_HIZ`. It added eight copper items and
preserved all 2,464 existing tracks and vias, 204 footprints and 655
pads. Independent KiCad checks found zero opens and parity findings
both as stored and after refill. The 24 raw errors persisted, including
four invalid source padstacks, and the verdict remained incomplete.

On the two FPGA boards, the verdict identified open pads on the second
ring of the ball grid array. Running the same layout with
`{"bga_fanout_second_ring":true}` as the router configuration kept each
placed board byte-identical. ICEpi went from 61 to 59 opens and from
261 to 248 vias. Glasgow went from 68 to 60 opens and from 440 to
473 vias. Raw error and parity counts matched the ordinary fanout
on both boards after an independent refill.

The three saved ICEpi boards (ordinary placement, signal seed, signal
seed plus second-ring fanout) had 90, 72 and 65 opens before refill.
On Glasgow the respective saved counts were 158, 164 and 163. For these
incomplete boards, the state of the plane fills affects the comparison.

On the ICEpi signal-seed and second-ring placement, an early-routing
experiment selected pairs of footprints sharing five to twelve
two-terminal nets. The FPGA and microSD connector qualified with seven
signals. Routing these first and keeping their 68 generated copper
objects closed all seven, compared with three microSD opens in the
ordinary control. With 120 modeled seconds for each whole-board pass,
board-wide opens increased from 32 to 47. After publishing fills, both
stored and refilled checks agreed on those counts, two inherited errors
and zero parity findings. The reports are in
`build/icepi-link-groups-20261120/comparison.json`.

Repeating the ICEpi second-ring run with the router's duplicate-segment
fix preserved all 163 footprint poses and all 4,686 distinct copper forms.
The older output had 13 repeated segments. The new layout delivered 33
filled polygons. Independent KiCad checks found 59 opens, two
copper-to-edge clearance errors and zero parity findings both as saved
and after refill. A separate `drc-kicad-board --save-filled` readback
matched the connected components of all 5,463 numbered copper objects.

On that same input, turning on `placer.decoupling_near` alone brought
the median measured capacitor-to-supply-pin distance from 5.04 mm to
3.03 mm. The share within 3 mm rose from 13.2% to 50.0%. At the same
180-second requested budget, KiCad found 70 opens after refill, up from
59, with the same two edge-clearance errors and zero parity findings.

### Trying a lower via cost on incomplete multilayer layouts

These cold runs changed one routing setting, `tuning.via_cost`, from its
default of 8 to 0.5. The inputs, placement seed, mechanical constraints,
plane settings and requested work budget stayed the same within each pair.
The FPGA runs used the signal seed and second-ring fanout described above.
KiCad readback matched every footprint position, angle, face and lock.
The first four boards requested 180 modeled seconds. The July schematic
replay used its normal 1,200-second budget and finished in about 35.

| Board | KiCad opens after refill, default → 0.5 | Vias, default → 0.5 | Track length in mm, default → 0.5 |
| --- | ---: | ---: | ---: |
| OpenESC Mini | 93 → 61 | 177 → 250 | 1,602 → 2,225 |
| OpenESC 30×30 | 40 → 33 | 200 → 220 | 1,903 → 1,780 |
| ICEpi Zero | 59 → 41 | 248 → 309 | 3,665 → 3,752 |
| Glasgow | 60 → 41 | 473 → 577 | 5,339 → 5,225 |
| July powder doser, schematic replay | 0 → 0 | 0 → 0 | 1,117 → 1,117 |

Raw DRC error counts and schematic parity counts matched within each
pair. ICEpi changed from one dangling-track warning to two dangling-via
warnings. Modeled work varied by board, increasing from 165.63 to 188.52
seconds on OpenESC 30×30 and decreasing from 175.86 to 154.70 on ICEpi.
Compare a separate output with the same constraints before adopting the
lower cost, particularly when via count matters.

Four further cold schematic replays started from boards that routed
with zero opens at the ordinary cost. Lowering the cost to 0.5 kept
OpenFC, TelemetryOnboard and MokyaLora at zero. On M8SBC-486 it
introduced two opens on both the saved and refilled board and increased
the via count from 776 to 1,392. Each pair used the same executable,
target board, budget scale, mechanical constraints and project settings.
The per-board comparison is in
`build/schematic-multilayer-via-cost-20261120/comparison.json`.

On the alternate ICEpi placement with `placer.decoupling_near`, the
same cost change cut opens from 70 to 41 while retaining the 3.03 mm
median capacitor distance. This layout used 313 vias and 3,803 mm of
track. KiCad reported the same two edge-clearance errors and zero parity
findings. On OpenESC 30×30, a separate `repair-kicad-opens --local`
run on the low-cost result joined the two remaining copper components
of `/ESC1/FBA` with five additions. KiCad confirmed 32 refilled opens
with zero DRC errors and the same 16 schematic parity findings.

On Glasgow, matched 1,500-second cold layouts used the same schematic,
constraints, placement seed pool and two routing threads. Both started
from the same placement digest, then selected different candidates.
At the ordinary via cost, KiCad found 42 opens after refill, compared
with 17 at a via cost of 0.5. Both had four footprint-type errors and
zero schematic parity findings. Via count increased from 493 to 547,
track length fell from 5,966.53 to 5,815.90 mm, and the verdicts
charged 1,476.53 and 1,370.98 modeled seconds respectively. The
ordinary run deferred its fill handoff and had 93 opens as saved.
The low-cost run published its fills and had 17 opens as saved.
Repeating the ordinary board's standalone fill handoff with the
installed KiCad footprint libraries saved a board with 42 opens,
matching its refill. Its four footprint-type errors remained.

Splitting the same requested 1,500-second allowance into two independent
cold runs of 750 seconds, one at each via cost, left 57 and 46 refilled
opens. Together they charged 1,340.82 modeled seconds. A single longer
run at the lower cost reached 17 opens in 1,370.98 modeled seconds on
this board. All four results had the same four footprint-type errors.
The smaller runs both deferred saving their fills.

On a separate six-layer TAPRX-888 fixed-placement replay, the routing
budget was 90 modeled seconds for each policy. Automatic pour selection
charged 281.72 seconds and left 139 native opens and six errors. Forcing
`pours: connect` charged 107.52 seconds and left 131 opens and five
errors. Forcing `pours: tracks` charged 101.28 seconds and left 133 opens
and seven errors. The automatic run spent 274.73 seconds on search.
Each preliminary routing strategy received a full first pass before its
work limit could stop it. Independent KiCad refill checks supplied all
three counts. The old and updated paired-transition binaries produced
byte-identical automatic routes on this board.

After automatic selection began measuring the first pass before starting
another policy, the same standalone route charged 106.68 seconds. KiCad
found 119 opens and five errors after refill. The 90-second request
allows 94.5 seconds with tolerance. Continuing and finishing the selected
route still exceeded that bound. The [matched controls](../../docs/benchmarks.md#standalone-route-probe-budget-on-taprx-888-2026-11-20)
include three smaller boards.

In the 17-open Glasgow result, both pads of C90 were in the outer-layer
perimeter rule area, which bars tracks and vias. The `pad_exits` probe
now identifies the blocked VBUS pad even though it has no legal routing
node. The same census on the ICEpi holdout still reported zero blocked
pads out of 383 sampled.

On the saved Glasgow placement, a local trial moved C90 from (128.905,
99.06) at 90° to (127.0, 100.95) at 0° and routed its two connections.
This trial preserved every existing track and via and added ten track
segments. Independent KiCad checks of the saved and refilled board each
found 15 opens, exactly the same DRC findings as the old refill (including
four inherited footprint-type errors), and zero schematic parity findings.

Enabling `placer.pad_exits` with a 2 mm reach in a cold 1,500-second
Glasgow layout changed the selected placement. C90 was inside the
routable opening, and all 14 held poses matched. Independent checks of
the saved and refilled result found 19 opens, four footprint-type errors
and zero parity findings. The original cold placement followed by the
targeted C90 repair retained the lower open count of 15.

Four remaining opens came from the +1V2 pads of U30. A separate routing
trial on the exact same placement compared an In2.Cu region around those
pads and two decouplers with the ordinary GND and +3V3 pours. At the
450-second setting, the no-region control left 29 native opens. The
51.5 mm² region left 37, and the router estimated that only 1.2 mm² filled.
Keeping tracks out of the region while allowing vias raised the estimated
fill to 16.1 mm² and left 36 native opens. Both region variants left U30's
supply connections unresolved.

To reproduce the change from an existing router JSON, preserve the rest
of that file and pass the generated file as the router argument to the
same `layout-kicad-board` invocation in a fresh output directory.

```sh
jq '.tuning.via_cost = 0.5' router.json > router-low-via.json
```

We also ran a separate **fixed-placement** comparison on 11 boards from
the [routing holdout](../expansion/holdout/manifest.json). We selected all
locally staged boards below 2 MB with at least four copper layers whose
prior 30-second routing trial had at least 50 routed connections, eight
vias and 12 remaining KiCad opens. Each arm started from the same stripped
board and used the same placement, rules, via-in-pad permissions and
30-second routing budget. The counts below are independent KiCad checks
after refill. These inputs have no accompanying schematic for parity checks.

Ten boards had fewer opens with the lower via cost. CM5Carrier went from
14 to zero and DMXion from 61 to zero, with zero raw DRC errors on both
boards. RetroCastX stayed at 16 opens and increased from 86 to 105
clearance errors. On the Sega CDX board both DRC listings reached the
499-open reporting cap. KiCad's native connectivity count after refill
measured 904 and 730 respectively. The selection targets incomplete
routes and does not estimate how often a random board benefits.

Rerouting RetroCastX after implementing its U9 footprint clearance
rule and the GbE class clearance rule with footprint exceptions removed
all clearance errors at both via costs. On both saved boards and after
refill, KiCad still reported 16 opens and two differential-pair skew
errors. All opens were on conductors between U9 and J11/J12.

The router now applies the footprint-local pad rule at U9 where it
overrides the broader net-class clearance. At the lower via cost
all 196 connections routed, with zero native opens or clearance errors.
KiCad then reported seven uncoupled-length errors and two skew errors
on the connected Ethernet routes. At the ordinary cost one open
remained. These are diagnostic runs; the connected low-cost board
still violates the project's electrical rules.

The pinned designer PCB provides a feasible reference under these exact
rules. All three file hashes (PCB, project and rules) matched the holdout
manifest. A fresh KiCad refill found zero opens and zero errors. Each of
the eight conductors in the second Ethernet port uses B.Cu for most of
its length. Seven reach their first via 1.60 mm along the trace from U9,
and the eighth at 2.475 mm. Each uses at most two vias. The reference is
used for measurement only. The routing experiments start without this
copper.

An existing-copper control exposed an independent issue in the
`route-kicad-board` keep mode. The unchanged designer PCB passed both
stored and refilled native checks with zero opens and zero errors. A
30-second routing run preserved every original copper object but added
532 segments and 98 vias across six already-connected power nets,
adding 364.05 mm of track. KiCad reported 262 clearance and 151
hole-clearance errors when it reopened the result as saved. Refilling
cleared all 413 errors. The router's complete verdict reflected the
refilled state. Removing only the two conductors of
one Ethernet pair produced exactly two native opens on input. In the
pad-precedence experiment, rerouting that input added 631 copper
objects to those same six power nets in addition to the Ethernet work.

This partially routed board also exercises the saved-copper report. In
an ordinary keep-mode run, KiCad independently counted 2,616 segments,
68 arcs and 580 vias in the output and measured 7,049.211013 mm of
planar track. The router report and verdict each recorded 7,049.210983
mm in `saved_copper.track_length_mm` and matched all three object
counts. The existing search-length field was 456.416063 mm for this
run. The difference includes the previously routed board copper.

The same keep-mode issue reproduced on the tool's own completed
CM5Carrier output. Rerouting the unchanged board added 25 vias and
446.39 mm of track despite zero native opens. Both stored and refilled
boards still passed KiCad on this fixture.

With the native input connectivity check enabled in ordinary keep
mode, the completed CM5Carrier board was returned byte for byte with
zero routing searches. Two timed reruns took 9.4 and 21.7 seconds of
wall time, including native checks, versus 46.1 and 48.0 seconds
before the change. Runs alternated versions (new, old, old, new) with
fresh caches and two routing threads on a shared host. The unchanged
RetroCastX control also kept its
original copper. On a deliberately shorted RetroCastX variant with zero
opens and four native errors, the unchanged-board path declined and
the final verdict remained incomplete. A separate input
with the stored zone fills removed measured 307 opens as stored and
zero after refill. Executing the exact `input_connectivity.retry_argv`
from the verdict published a separate board with zero stored opens
and zero errors.

For a positive partial-route control, removing just three CM5Carrier
LED segments left one open and zero other native findings. Before the
native plane check, rerouting closed the open but added 360 unrelated
GND objects, including 25 vias. With the check, the router added only
three segments (3.077 mm) on the LED net, preserved every original
copper object and charged 0.65 modeled seconds versus 30.92. Independent
checks on both the saved and refilled board found zero opens, errors,
warnings or schematic mismatches.

On a fresh route from the same CM5Carrier placement, a separate power
region experiment used a 90-second modeled budget in each arm. Each
region was the convex hull of its rail's pads with 1 mm margins.

| Added regions | Total track (mm) | Vias | CM5_3V3 track (mm) | M2_3v3 track (mm) |
| --- | ---: | ---: | ---: | ---: |
| None | 2,983.09 | 103 | 210.8 | 113.2 |
| CM5_3V3 on In2.Cu | 2,826.54 | 106 | 14.5 | 126.2 |
| Both, with M2_3v3 on In3.Cu | 2,792.67 | 102 | 15.9 | 38.6 |
| Board-wide CM5_3V3 on In2.Cu | 2,916.05 | 91 | 14.6 | 116.9 |
| Both board-wide, with M2_3v3 on In3.Cu | 3,086.71 | 76 | 24.6 | 58.4 |

The last two arms used the existing `add_pours` setting. Both board-wide
planes saved 26 vias compared with the two local regions and used
294.04 mm more routed track. Independent KiCad refill checks found zero
opens, violations and parity findings in all five arms. The normal fill
handoff also published independently reopened clean boards for the
baseline and the two local regions. This comparison measures routing
geometry on one placement.

The same 90-second experiment on the four-layer DMXion placement added
a region for the 30-pad +LDO rail. The baseline already had GND and
+3V3 inner planes. Giving +LDO priority over +3V3 on In2.Cu produced
993 mm² of filled +LDO copper, reduced the existing +3V3 fill by
1,024 mm² and left routed track length unchanged at 6,062.1 mm. Using
B.Cu instead saved 2.7 mm. All three arms used 265 vias and had zero
opens, errors and parity findings after refill, with 61 warnings. The
router's probe preferred routing with tracks in the inner-region arm.
Adding rail regions by pad count alone yielded little on this board.

### DMXion 4-port node with board-only mounting hardware

The pinned DMXion schematic supplies 269 footprints. Three further
footprints on the PCB share the reference `REF**` and the value `NLR-LB`.
Each has two unnetted 1.2 mm plated drills in 2 mm lands. Select the
three by UUID to preserve their exact holes and copper while moving the
schematic components. Three board-only logos were outside the brief.
[Pinned KiCad project](https://github.com/NEOgHacking/DMXion_4Port_Node/tree/6742f672f3ba41dd4f72a0fd0b9b68d4165ffbfc/PCB/DMXion%204port%20node).

Set `source_dir` to the pinned project's `PCB/DMXion 4port node`
directory and run from the pcb-maker repository root. The
[mechanical brief](dmxion-mechanical.json) holds eight mounting holes,
13 connectors and two switches at the donor poses. It keeps 62 schematic
parts on the rear face. Four pairs of alternative XLR footprints share
their respective panel openings on the donor. The other 246 schematic
footprints have free positions. The board supplies the outline, two rule
areas, fabrication stack and footprint geometry.

```sh
board="DMXion 4port node"
pcb="$source_dir/$board.kicad_pcb"
work="$PWD/build/dmxion-replay"
mkdir -p "$work"
pcb-maker import-kicad-netlist "$source_dir/$board.kicad_sch" "$work/project" "$board" \
  --mechanical-from "$pcb" --footprints-from "$pcb" --stackup-from "$pcb" \
  --mechanical-footprint 8986ad2b-5588-4a00-96e1-59e019977527 \
  --mechanical-footprint c42990fc-4f29-481a-9e7c-f6db2c16b18f \
  --mechanical-footprint dd70b240-12c9-4796-9448-54fb94616ef9
jq -n --arg constraints "$PWD/benchmarks/from-schematic/dmxion-mechanical.json" \
  '{total_seconds:240,placer:{constraints:$constraints}}' > "$work/layout.json"
PCB_WORK_RATE=3.15e6,150e6 RAYON_NUM_THREADS=2 \
  pcb-maker layout-kicad-board "$work/project" "$board" "$work/layout" auto "$work/layout.json"
```

The ordinary run routed all 160 connections with 279 vias and 5,575.75 mm
of track. KiCad confirmed all 874 schematic pad assignments, all 23 held
poses, 20 outline primitives and two rule areas. Its plated Excellon
export contained all six bracket holes at the donor coordinates.

KiCad found zero opens, zero errors and zero parity findings after refill.
The budget ended before saving the fills. As written, the PCB had 100
opens and 76 dangling-via errors. The standalone fill handoff refused
this particular result because KiCad changed the reported objects for
a 0.0165 mm connection-width warning on `/POWER/CC2` between refill
and reopen. The project's minimum connection width is 0.1 mm.

A later diagnostic isolated the effect of track order. A copy with the
original track forms ordered as KiCad writes them passed the same fill
handoff. The delivered file had zero opens, errors and parity findings,
and retained the 0.0165 mm width warning. All 4,172 pads and copper
objects had the same connectivity on the original refill, on reopening
the delivered file, and after a fresh refill. The standalone
`drc-kicad-board --save-filled` command now tries this ordering when
the native width or differential-pair reports meet its witness checks.
On a USB hub, the retry reordered 2,738 of 2,752 track and via forms.
The delivered file reproduced all seven differential-pair length
errors and 29 opens from the original refill, and preserved all 3,415
pads and copper objects in the same 195 connected groups.

The standalone `repair-kicad-necks` command now repairs this contact
with a 0.0915 mm long, 0.1 mm wide segment on B.Cu. KiCad verified all
302 original connectivity components and removed the width warning.
The ordinary `drc-kicad-board --save-filled` command then saved the
board. Reopening and refilling each found zero opens, errors or parity
findings and the same 61 remaining warnings. A control with a B.Cu
copper keepout across the contact declined the repair.
Both commands used `PCB_MAKER_KICAD_USER_LIBS=1` with this project's
footprint libraries installed. In the isolated library configuration
the KiCad library warnings reached the reporting cap, and the repair
declined because it could not verify the full set of findings.

With the same placement inputs and 240-second allowance, setting
`tuning.via_reduction_rounds` to zero in the router configuration saved
the filled board in 198.79 modeled seconds (ordinary run 241.93).
The saved board had 279 vias and 5,572.75 mm of track. Independent KiCad
checks on reopening it and after refilling it each reported zero opens,
zero errors, zero schematic parity findings and 61 warnings. Neither
check reported a connection-width warning. This comparison covers one
board and uses a cold start in both runs.

With placement seed 101 and ordinary via reduction, the supplied
`fill_publication.retry_argv` took the board from 115 opens as stored
to zero on disk. The saved board had zero errors and parity findings,
56 warnings and 288 vias. The same six drill hits, 23 held poses and
874 pad-net assignments were verified against the inputs.
Disabling via reduction at this seed also saved the filled board within
the allowance. Both arms selected the same 272 footprint poses. Work
fell from 238.86 to 196.03 modeled seconds, with 288 vias in both and
0.14 mm less track. Reopening and refilling each found zero opens,
errors or parity findings and 56 warnings.

The ordinary layout now reserves time for the required native checks
before admitting an optional via-reduction pass. Repeating seed 101
with default router settings produced the same filled PCB byte for byte
as the explicit no-via-reduction run. It retained all 23 held poses
and six bracket drill hits. On the clean CM5Carrier control, the via
pass still ran and the saved PCB was byte-identical to its baseline.
The default placement seed also published its verified fills under the
ordinary router settings, in 198.79 modeled seconds. Independent saved
and refilled checks each found zero opens, errors or parity findings
and 61 warnings.

Native connectivity alone misses a signal-routing difference on this
fixture. On the five KiCad-recognized differential pairs with two pads
per net, the largest difference in planar track length between paired
conductors was 0.022 mm on the donor and 11.133 mm on the fully
connected ordinary-seed run with via reduction disabled. Both maxima
were on the Ethernet TX+/TX− pair. These lengths exclude via barrels.

Turning on `diff_pairs` with the same inputs and via reduction disabled
coupled eight of 13 detected pairs. Independent KiCad checks on the saved
board and after refill both found zero opens, errors or parity findings
and 56 warnings. KiCad measured 5,330.00 mm of planar track and 317
vias on the coupled run, versus 5,572.75 mm and 279 vias on the ordinary
run. The layout runs selected
different placement candidates (seed 2 versus seed 1), so these figures
compare end-to-end outcomes. Individual pairs responded differently.

| Paired conductors | Ordinary routing, difference (mm) | Coupled routing enabled, difference (mm) |
| --- | ---: | ---: |
| Ethernet TX+ / TX− | 11.133 | 0.287 |
| Ethernet RX+ / RX− | 3.503 | 6.529 |
| USB D+ / D− | 5.336 | 2.251 |
| Ethernet TXP / TXN | 4.969 | 7.782 |
| Ethernet RXP / RXN | 1.523 | 5.139 |

Only TX+ / TX− was coupled among these five. The route report identifies
the other four as having fewer than two parts with side-by-side pads.

A separate `route-kicad-board` comparison used the identical 272 poses
from the ordinary-seed board in both arms, one seed, a 120-second
routing allowance and no via-reduction rounds. Each routed all 160
connections, and independent refill checks found zero opens, errors
or parity findings and 61 warnings. Enabling coupled routing reduced
total saved track length from 6,291.42 to 6,290.17 mm and increased
the saved via count from 247 to 270. Ethernet TX+ / TX− improved from 5.652 to 0.287 mm of
planar length difference. Ethernet RX+ / RX− remained at 6.748 mm.
The USB D+ / D− difference increased from 3.500 to 4.460 mm.

On this placement, KiCad measured 5.64 mm between the USB-facing
pads of the two series resistors R5 and R4. A diagnostic arranged
those resistors and the two Ethernet series-resistor pairs side by side,
moving six footprints and keeping all others fixed. With identical
routing settings, USB then coupled and its planar length difference
fell below 0.001 mm. The router found both Ethernet endpoints but
reported a blocked launch on TXP/TXN and a copper conflict on RXP/RXN.
All 160 connections remained routed. Across the board, coupled pairs
fell from six to five and KiCad reported three additional warnings
(two copper-edge clearances and one connection width). This diagnostic
isolates the effect of resistor placement on the USB pair.

In the cold layout with explicit `placer.pair_rows`, the initial
placement reported zero pair rows. The final report marked ten as
satisfied. A KiCad readback of both same-number pad separations on each
reported resistor or capacitor pair found that all ten differed from
their reported pitch by more than 0.5 mm on at least one pad. The
largest discrepancy was 25.23 mm on R27/R31. The saved footprint
positions are the acceptance check for subsequent changes to this flow.

After fixing cold row placement and carrying the selected pitches into
the final report, all ten pairs met their 0.95 or 1.47 mm pitch in the
saved PCB. Both native DRC modes found zero opens, errors or parity
findings. The courtyard warning count stayed at five. The USB pair's
planar length difference fell from 2.251 to 0.287 mm and the Ethernet
TXP/TXN difference fell from 7.782 to below 0.001 mm. Ethernet TX+/TX−
went from 0.287 to 5.670 mm. The saved board had 300 vias, 5,491.74 mm
of track and two additional warnings. Both control runs used identical
inputs, budget and explicit pair-row settings.

Two further starts (101 and 201) still met all ten physical pitches. Both
routed all 160 connections but failed native DRC. Seed 101 left a
0.2 mm dangling Ethernet TXN track. Seed 201 shorted an Ethernet
TX− via against a 5.9 mm ordinary front-layer track on Net-(C65-Pad1).
The via and track centres were 0.357 mm apart, with copper radii of
0.3 and 0.1 mm respectively.

In an isolated geometry trial on seed 201, moving a short section of
the ordinary track 0.20 mm away from the via, with two 45-degree
transitions, removed the short and one hole-clearance warning. Every
other native finding was identical. The change replaced one segment
with five, added 0.166 mm of track, and kept all 367 vias. The standard
fill handoff saved the board with zero errors, opens or parity findings
and 61 warnings. All 23 held poses, all 874 pad-net assignments and
all six bracket drill hits still matched the inputs. This was a local
diagnostic edit after routing.

The standalone `repair-kicad-via-short` command reproduced the repair
from KiCad's short report and the original placement. It selected a
0.25 mm offset, adding 0.207 mm of track. Independent stored and
refilled checks each lost exactly the short and the hole-clearance
warning. All other findings matched. When given a placement that
already contained the offending track, the command declined and
returned a byte-identical PCB.

The ordinary seed 201 layout run applied the detour automatically and
completed all 160 connections in 239.988 modeled seconds of its 240-second
budget. Independent KiCad checks of saved and refilled copper each found
zero opens, errors or schematic parity findings, with 61 warnings. All
23 held poses, 62 rear placements, 874 pad-net assignments and six
bracket drill hits matched the inputs.

In seed 101, removing only the KiCad-identified dangling segment also
allowed the standard fill handoff to finish. The saved board had zero
opens, errors or parity findings and retained the same 62 warnings.
The Ethernet TXP/TXN planar copper-length difference increased from
below 0.001 mm to 0.200 mm when the unused track was removed. This is
an example where summing copper length counts a dangling branch. The
existing pad-stub repair did not recognize this generated signal track.

The source-level repair now discards this generated launch tip before
skew tuning. A cold `layout-kicad-board` replay starting at seed 101
used the same pair-row placement, coupled routing and disabled via
reduction as the failing run. It saved the filled PCB in 146.275
modeled seconds of the 240-second budget. Independent KiCad checks
of the saved board and after refill each found zero opens, errors or
schematic parity findings and the same 62 warnings as the old refill.
Readback again matched all 23 held poses, 62 rear placements, 874 pad-net
assignments and six bracket drill hits.

The replay kept eight of 14 coupled links and all 357 vias. The
TXP/TXN planar length difference stayed below 0.001 mm. The old TXN
tip accounted for 0.200 mm of the 0.400 mm reduction in total track
length. The existing skew tuner shortened the TXP meander by the other
0.200 mm. The remaining track geometry matched the failing run.

On the completed seed 101 layout, the verdict suggested enabling
`placer.decoupling_near`. Replaying from the schematic with that single
change raised the share of capacitors within 3 mm of a supply pin from
24% to 39%. Median distance fell from 3.8 to 3.3 mm. The router kept
14 of 14 planned differential links coupled, up from 8. Track length
increased from 5,515.34 to 5,854.56 mm and vias from 357 to 394.
Both saved and refilled KiCad checks found zero opens, errors and parity
findings, with 55 warnings. The fixed geometry and pad assignments passed
the same mechanical readback. These distances measure placement geometry.
The coupled-link count includes the name-inferred /Ethernet/V+ and
/Ethernet/V- pair. Those nets connect the PoE rectifier diodes, C64
and the power controller. Their reported mismatch of 28.10 mm accounts
for the largest skew in this run.

A diagnostic routing build omitted only that rectified-supply pair from
coupled search. Placement, native rules and routing budget stayed the
same. Seed 101 retained zero errors and opens, used 342 vias instead
of 357 and added 30.15 mm of track. On seed 301, the original dangling
power track disappeared, vias fell from 343 to 314 and track length fell
by 142.21 mm. One connection remained open, between IC1:5 and L5:2
on the switch node. Saved and refilled KiCad checks agreed in both arms.

The router setting `"diff_pair_exclusions": [["/Ethernet/V+", "/Ethernet/V-"]]`
now provides that selection. In a 150-second `route-kicad-board`
comparison with identical footprint poses, native DRC confirmed 160/160
connections and zero errors or parity findings on the excluded run,
both as stored and after refill. It coupled Ethernet TX+/TX− and used
276 vias and 6,371.6 mm of track. The matching control used 283 vias
and 6,546.3 mm of track and left a 0.141 mm dangling V− track.

A separate rectifier board with no coupled links kept identical PCB
bytes when its `V+`/`V-` pair was excluded. The CLI rejected a nonexistent
name, a mismatched pair and a duplicate exclusion. On the USB/LVDS rule
fixture, excluding the USB pair left its native gap and uncoupled-length
errors visible. The verdict named the applicable rule for the excluded
pair, and the output retained the original rule file byte for byte.

With every component's face specified and the PoE supply pair excluded,
the updated placer met the requested spacing at both pads of all eight
series-component pairs. The previous placer met none. KiCad readback of
the repaired, filled board confirmed all eight rows, all 272 footprint
faces and all 23 explicitly placed poses. Three other footprints shared the
front-side selector `REF**`. Despite their KiCad locks, all three brackets
moved in both placements. A UUID-based check found none of their six holes
at the input coordinates. The layout gained coupled USB and Ethernet
RX links, while the total coupled links fell from seven to six. Vias
increased from 314 to 342 and copper length from 5,505.04 to 5,524.77 mm.
The original saved output had 125 opens because the layout exhausted its
fill budget. The suggested fill command produced a board with zero opens,
errors and schematic parity findings on disk.

The placer now keeps a locked footprint at its input origin when the
only placement instruction for it selects a face. Repeating placement
with the same DMXion brief retained all six bracket holes at their design
coordinates. KiCad's plated Excellon export from the ordinary routed PCB
contained all six 1.2 mm drill hits, up from zero with the same brief
before the fix. A comparison with explicit bracket holds reached zero opens
and one dangling 0.80 mm +3V3 track on In2.Cu. Removing that
single generated segment in a diagnostic copy left all 4,612 remaining
pads and copper objects in the same 202 connected groups. KiCad confirmed
the partition both on reopening and after refill. The resulting PCB
retained all six drill positions and had zero opens, errors and parity
findings, with 58 warnings.

With finish time reserved ahead of optional routing improvements, the
ordinary DMXion run published its filled PCB in 234.59 of 240 modeled
seconds. The saved board had zero opens, zero parity findings and one
dangling-track error. Running `repair-kicad-dangling` with
`--generated-from` pointing to that layout's `placed/` directory removed
the 0.80 mm +3V3 track. KiCad verified all 4,626 surviving copper objects
in the same 202 groups on reopening and after refill. The repaired board
had zero opens, errors and parity findings in both checks.

On a separate RP2040 motor-interface board, the same command removed a
0.80 mm GND track on In1.Cu. The eight footprint errors and 42 existing
opens remained. KiCad chose different examples for several of those
opens, but both checks confirmed that all 6,856 surviving copper objects
were still in their original 306 groups.

With the exclusion applied during cold seed 101 placement, the two
inferred PoE rows went away. The first route had three USB via-to-track
clearance errors. The copper check now identifies both shapes even when
the verifier names the ordinary track first. Repeating that run caught
all three clearances before emission and left a refill-clean board. The
ordinary fill handoff declined because KiCad named different objects for
a connection-width warning when reopening the fill.

With `placer.decoupling_near` also enabled, the ordinary 240-second
layout saved verified fills directly in 211.44 modeled seconds. It
routed all 160 connections with 302 vias and 5,723.08 mm of track. Nine
of 11 links coupled, including both Ethernet TX pairs and the main USB
data pair. Independent checks of saved and refilled copper each found
zero opens, errors or parity findings and 53 warnings. The pad nets,
held poses, rear assignments and bracket drill hits matched the input.

Placement seed 301 provided a further cold control. With the original
placement settings, the launch cleanup left both runs at 343 vias and
5,478.60 mm of track. Each had one crystal-net open between U1:31 and
R6:2 plus a dangling track on /Ethernet/V+.
Enabling the recommended `decoupling_near` setting on this seed closed
the crystal connection and cleared the dangling-track error. The share
within 3 mm rose from 26% to 47% and median distance fell from 3.9 to
3.1 mm. Track length fell to 5,437.50 mm and vias to 298. The verdict
charged 213.00 modeled seconds. Both native checks reported zero opens,
errors and parity findings, with 59 warnings. The same held poses, rear
placements, pad nets and drill hits matched on readback.

### CyberKeeb mechanical keyboard

The two-layer MainBoard schematic contains 166 footprints. Its mechanical
board fixes every one, including 76 on the rear. This case exercises routing
from a copper-free import with a fully specified placement. The schematic
project supplies its own footprint libraries.

```sh
case_id=zli117__CyberKeeb2040__MainBoard
schematic="benchmarks/real/external/github-sch/$case_id/repo/PCB/MainBoard/MainBoard.kicad_sch"
donor="benchmarks/real/external/github/$case_id/MainBoard.kicad_pcb"
work=build/cyberkeeb-mechanical
pcb-maker import-kicad-netlist "$schematic" "$work/project" MainBoard --mechanical-from "$donor"
jq '.total_seconds = 600' "$work/project/layout.json" > "$work/layout-600.json"
PCB_WORK_RATE=3.15e6,150e6 RAYON_NUM_THREADS=2 pcb-maker layout-kicad-board \
  "$work/project" MainBoard "$work/layout" auto "$work/layout-600.json"
```

The November 19 cold run routed all 118 connections in 211.11 modeled
seconds. KiCad readback matched all 166 held schematic footprints, the outline
and its cutout. It also matched the native IDs, positions, sides, angles and
locks of ten mounting holes and five switch stabilizers that share the
reference `REF**`. The remaining unmatched lock is a logo. An earlier import
lost 30 holes despite passing connectivity checks. Separate Excellon exports
from the donor and the routed board now match all 368 non-plated drill hits
by location and diameter.

KiCad reported zero opens and zero schematic parity findings both as saved
and after refilling. The three raw errors also occur on the donor: J2 has a
malformed courtyard, and the stabilizer courtyards overlap SW24 and SW25.

A partial mechanical brief held the 69 switches and five connectors,
retained all assembly faces and unlocked the 84 through-hole diodes, two
capacitors, five resistors and U1. Both runs started from the same
copper-free import. At the default 0.635 mm placement grid the layout
stopped before routing. Five diodes had no assigned legal position, and
the verdict named eight movable parts after including their conflicts.
A 2.54 mm grid placed all 92 movable parts and satisfied all 74 pose and
166 face constraints. Each of those 92 parts moved from its source pose.
The selected placement seed used 126.58 modeled seconds at 2.54 mm and
257.48 at 0.635 mm. These are placement times for the selected seeds.
The larger grid was chosen for this trial. It routed 118/118 connections
with 71 vias and 10,937.22 mm of track. Independent KiCad checks found
zero opens and zero schematic parity findings both as saved and after
refill. The same three source courtyard errors remained.

The revised placer also completed this brief on its original 0.635 mm
grid. When the local search could not seat all the parts, it sampled
more widely spaced points on that same grid. All 92 movable parts
moved, all 74 held poses and 166 specified faces matched, and the board
routed 118/118 in 338.14 modeled seconds with 96 vias and 10,361.74 mm
of track. KiCad independently found the same three courtyard errors,
zero opens and zero parity findings both as saved and after refill.
On a separate BumWings keyboard with its controller and switches held
and 64 diodes movable, both placer versions returned the same PCB bytes
in 49.36 modeled seconds. That board routed 81/81 with zero native errors.

### USB Keypad, finished board

This brief fixes the nine switches and three connectors on the front at the
harvested USB Keypad's mechanical interface positions. The supplied drawing
specifies the 85 × 25 mm finished body with rounded corners. It closes the two
breakaway tab locations at the finished edges. The panel drawing in the
harvest includes handling rails and uses two board-only mouse-bite footprints
to complete its perimeter.

```sh
case_id=CDFER__Business-Cards__USB_Keypad
case_dir=benchmarks/from-schematic/usb-keypad
source_dir="benchmarks/real/external/github-sch/$case_id/repo/USB_Keypad/PCB"
PCB_WORK_RATE=3.15e6,150e6 RAYON_NUM_THREADS=2 benchmarks/from-schematic/run.py build/usb-keypad-finished \
  --schematic "$source_dir/USB Keypad.kicad_sch" --layers 2 --total-seconds 240 \
  --constraints "$case_dir/constraints.json" --outline-board "$case_dir/finished-outline.kicad_pcb"
```

The November 18 run imported 81 footprints, held all 24 pose and side
constraints and routed all 39 connections. KiCad reported zero open
connections and zero schematic parity findings. Five hole clearances within
the supplied J2 footprint failed the raw DRC gate.

### USB Keypad, panel drawing

The panel supplies two locked breakaway footprints, MB2 and MB3. Each has
four Edge.Cuts curves and seven non-plated holes. The curves join the 24
board-level edges to close the perimeter. This run leaves all 81 schematic
footprints movable. The donor supplies the footprint definitions and mechanical
geometry.

```sh
case_id=CDFER__Business-Cards__USB_Keypad
source_dir="benchmarks/real/external/github-sch/$case_id/repo/USB_Keypad/PCB"
donor="benchmarks/real/external/github/$case_id/USB Keypad.kicad_pcb"
work=build/usb-keypad-panel
pcb-maker import-kicad-netlist "$source_dir/USB Keypad.kicad_sch" "$work/project" "USB Keypad" \
  --footprints-from "$donor" --mechanical-from "$donor"
jq '.total_seconds = 240' "$work/project/layout.json" > "$work/layout-240.json"
PCB_WORK_RATE=3.15e6,150e6 RAYON_NUM_THREADS=2 pcb-maker layout-kicad-board \
  "$work/project" "USB Keypad" "$work/layout" auto "$work/layout-240.json"
```

The November 19 run routed 39/39 connections in 129.51 modeled seconds.
Independent KiCad readback matched all 32 perimeter primitives and both
breakaway footprints' poses, locks and drills. KiCad found zero opens and zero
schematic parity findings on the stored and refilled output. Its 17 raw errors
were five hole-clearance findings within J2 and six hole-to-hole findings
within each breakaway footprint.

### GigaShield archived two-layer comparison

The [GigaShield manifest](gigashield-layout.json) pins Andrew Jokela's
parked starter and published Quilter Candidate 1. These files use two
copper layers and a 0.20 mm minimum clearance. The run retains the 15
component poses wholly inside the source outline, moves the other 49,
and holds all 64 footprints on the front. The source includes both GND
zones and no tracks or vias. The author's separate four-layer run is
described in the [routing account](https://tinycomputers.io/posts/what-routing-314-nets-taught-me-about-ai-assisted-pcb-design.html).

```sh
git clone https://github.com/ajokela/giga-shield.git build/gigashield-input
git -C build/gigashield-input checkout ad408d17cba5fa4d87c53d4f0478f9313a3bce84
/usr/bin/python3 benchmarks/from-schematic/gigashield-layout.py build/gigashield-replay \
  --dataset build/gigashield-input --binary target/release/pcb-maker
```

Use a Python with the matching KiCad 10 `pcbnew` module. The runner
checks the archived hashes, outline, 64 footprints, 507 pads, 161 nets,
and all 15 held poses, then independently scores the starter, candidate
and local result under both project files. It stores native KiCad reports
with saved and refilled copper. Pass `--existing-layout` with a previous
run directory to rescore that run after validating the brief, settings,
and recorded work rates. The default layout budget is 1,500 work seconds.

The November 18 run used KiCad 10.0.8. The original project's raw results
were as follows. Both refilled columns had the same counts under the
returned settings. All 79 reported placement constraints were met.

| Board | Saved opens | Refilled opens | Refilled DRC errors | Track length (mm) | Vias |
| --- | ---: | ---: | ---: | ---: | ---: |
| Quilter Candidate 1 | 1 | 1 | 0 | 7381.073 | 520 |
| pcb-maker | 19 | 0 | 0 | 6171.044 | 239 |

The archived starter, candidate and local result each reported 228
schematic parity findings, including 199 net conflicts at the report's
per-type cap. The full parity count could be higher. A separate import
from the schematic reproduced only 12 of 161 archived multi-pad net
groups exactly by reference and pad number. Thirteen pins on named
archived nets became schematic no-connects, including J10 pad 1 on
`+5V`. This comparison scores the archived board netlist. The local result
preserved the footprint and pad geometry and added a solid zone
connection override on C28 pad 2 (GND). Restoring the source's inherited
thermal on a test copy reproduced one starved-thermal error; connectivity
still passed after refill. All 19 saved local opens were GND and cleared
on refill. A separately saved filled copy also scored zero opens and
zero DRC errors with the stored copper. Candidate 1's remaining open was
`PA1_C` between J2 pad 8 and U4 pad 4. The local layout used 262.72 work
seconds, or 182.64 seconds of elapsed time on that machine.

A November 20 cold replay with the revised finish budget retained all 79
placement constraints. Under the original project rules, the local board
used 6,168.572 mm of track and 239 vias. KiCad found 19 opens as saved
and zero after refill. The archived candidate had one open in both
checks, 7,381.073 mm of track and 520 vias. Both had zero refilled DRC
errors. The local verdict reported 193 pad numbers whose net assignments
differed from the schematic. Its fill check stopped at KiCad's reporting
cap for schematic net conflicts with work time still available.

In a separate fixed-placement route of the schematic-derived GigaShield
project, routing consumed 449.6 of 450 requested modeled seconds. The
new standalone fill handoff deferred its extra checks and recorded the
budget reason. Independent saved and refilled checks still found zero
opens, errors and parity findings. The previous and current binaries
produced identical track and via forms on all 2,489 items and kept all
64 footprint poses. See `build/direct-route-gigashield-handoff-20261120/comparison.json`.

To lay out the schematic's own connections, reuse the mechanical brief
from that replay. The board supplies the outline and footprint geometry.
The two ground pours are explicit routing instructions. The importer
reports donor connectivity differences by reference and pad number.

```sh
source_dir=build/gigashield-input/benchmark/placement
printf '%s\n' '{"add_pours":[{"net":"GND","layers":["F.Cu","B.Cu"]}]}' > build/gigashield-ground.json
PCB_WORK_RATE=3.15e6,150e6 RAYON_NUM_THREADS=4 \
  /usr/bin/python3 benchmarks/from-schematic/run.py build/gigashield-from-schematic \
  --schematic "$source_dir/giga_shield_v04.kicad_sch" --layers 2 \
  --outline-board "$source_dir/giga_shield_v04.kicad_pcb" \
  --footprints-from "$source_dir/giga_shield_v04.kicad_pcb" \
  --constraints build/gigashield-replay/input/constraints.json \
  --router-config build/gigashield-ground.json --total-seconds 1500 \
  --binary target/release/pcb-maker
```

This run routed all 155 multi-pad schematic nets, kept the same 79
placement constraints, and passed KiCad with zero DRC errors, opens and
schematic parity findings after refill. The routed PCB before fill
publication had 36 stored GND opens. No published commercial run on
this electrical input was available for comparison.

Running the same schematic and mechanical brief with two routing
policies shows how the requested ground pours affect track length and
via count. To reproduce the comparison, set `RAYON_NUM_THREADS=2` in
the command above. Omit `--router-config` for the default row. Both
runs used the same work rate and the same 1,500-second budget.

| Schematic routing policy | Track length (mm) | Vias | Modeled seconds |
| --- | ---: | ---: | ---: |
| Default, ground connected with tracks | 7,299.975 | 246 | 351.99 |
| GND pours on both copper layers | 6,981.195 | 314 | 360.14 |

Both boards satisfied all 79 placement constraints. Independent KiCad
checks found zero opens, errors and parity findings and 53 warnings on
each board, both as saved and after refill. The explicit-pour result
was byte-identical to the earlier schematic replay. These measurements
are under `build/giga-commercial-main-20261120/`.

With enough budget, normal layout saves the verified fills in
`layout/result/`. A matched replay published 66 polygons in the two
GND zones. Fresh KiCad checks with all track errors and schematic parity
enabled found zero errors, opens and parity findings both as stored and
after refill. Each check reported 53 warnings. The selected PCB retained
all 2,425 segment and 314 via identities and geometries, all 64
footprints and their pad geometry, the outline, project rules and
schematic. Only the fill forms changed. Layout charged two additional
KiCad checks, for 360.14 modeled seconds versus 352.94 before saving
fills. The `fill_publication` report gives the board digests and stored
verification counts when saved, or the reason publication was skipped.

The November 20 finish-budget replay from the actual schematic also met
all 79 constraints and connected all 155 multi-pad nets. It used 314
vias and 6,981.195 mm of track. Independent KiCad checks of the saved
file and a fresh refill both found zero opens, errors and parity findings.

To save the refill of an existing board in a separate project directory:

```sh
run=build/gigashield-from-schematic
target/release/pcb-maker drc-kicad-board "$run/layout/result" giga_shield_v04 \
  "$run/handoff-check.json" --save-filled "$run/handoff"
```

In a separate July C4 incremental comparison, the same release retained
309 prior segments and routed both pending connections in each arm.
Default fill publication saved one In1.Cu polygon. It charged four
KiCad checks for the full run and used 15.40 modeled seconds. With
`--defer-zone-fill`, the run charged two checks and used 8.20 modeled
seconds. The individual runs took 12.94 and 8.02 elapsed seconds,
respectively. Copper identities and geometry outside the fill, the
mechanical brief, and the project inputs matched. Both versions had
zero errors, opens and parity findings in independent stored and
refilled checks.

Repeating the same July C4 invocation in fresh output directories reused
the verified fill when the runs shared `PCB_MAKER_CACHE_DIR`. With two
separate cache directories, the first publication in each took 2.11–3.53
seconds and ran two native checks. Cache hits took 0.08–0.09 seconds and
ran no publication checks. Full layout elapsed ranges overlapped at
10.43–13.56 seconds for the first runs and 8.47–13.41 seconds for the
hits. Both charged the same 15.40 modeled seconds. Independent checks
matched the saved PCB bytes, KiCad findings, and all brief constraints.

For this check, keep the C4 schematic and original 6 mm donor fixed.
Change the output directory for each invocation. A new cache directory
or `PCB_MAKER_NO_CACHE=1` gives fresh verification. A custom rule
scoped to an absent net produced the same PCB bytes and still required
fresh verification. An authored 0.127 mm move of C4 also required fresh
verification. Repeating either edited case then hit its own cached fill.

For a consecutive reopen, point `--reuse-routes` at the just-saved filled
PCB and repeat with the latest result. The C4 replay preserved all 318
track identities and their order in both reopens, with zero connections
pending. Each selected board matched the exact bytes eligible for
cached fill, so both reopens hit on their first lookup. Removing C4
then required fresh fill verification and pruned the five +5V and four
GND segments that belonged to C4.

### QPlayer standalone board comparison

Quilter published a parked KiCad PCB and a ZIP containing its routed PCB
and project settings. The runner pins both downloads by SHA-256. The
archives contain no schematic, so schematic parity is unavailable. [QPlayer](https://www.quilter.ai/blog/qplayer)

```sh
mkdir -p build/qplayer-input
curl --fail --location https://storage.googleapis.com/quilter-public/qplayer_input.kicad_pcb \
  --output build/qplayer-input/qplayer-input.kicad_pcb
curl --fail --location https://storage.googleapis.com/quilter-public/qplayer_fabbed.zip \
  --output build/qplayer-input/qplayer-fabbed.zip
/usr/bin/python3 benchmarks/from-schematic/qplayer-layout.py build/qplayer-replay \
  --starter build/qplayer-input/qplayer-input.kicad_pcb \
  --archive build/qplayer-input/qplayer-fabbed.zip --binary target/release/pcb-maker
```

Use the Python `pcbnew` module for the installed KiCad 10 CLI. The runner
adds two inner layers to the starter and uses the archived project rules
for every board. The replay holds all 84 component faces and the nine
poses whose origins start inside the outline. It moves the other 75
components and requests board-wide GND on In1.Cu and 3.3V on In2.Cu.
Each score requires the archived outline, pad geometry, reference and
pad-net assignments. KiCad's pad coordinates and physical footprint
graphics are compared at 10 nm resolution. Routed boards must have the
two specified zone outlines. The replay also checks the track keepouts
pcb-maker generates on the inner layers. `--existing-layout build/qplayer-replay/layout` can
replace `--binary` to verify an existing run and rerun native DRC.

The November 18 boards independently rescored as follows with KiCad
10.0.8, all severities and `--all-track-errors`. Track lengths include
segments and arcs. The replay's `results.json` links each matched board
to its saved and refilled native reports and includes the normal
pcb-maker verdict.

| Board | Saved opens | Saved DRC errors | Refilled opens | Refilled DRC errors | Track length (mm) | Vias |
| --- | ---: | ---: | ---: | ---: | ---: | ---: |
| Adapted starter | 207 | 43 | 207 | 43 | 0 | 0 |
| Quilter archive | 4 | 481 | 0 | 109 | 7457.487 | 326 |
| pcb-maker | 0 | 68 | 0 | 68 | 2245.541 | 60 |

The refilled archive had 95 solder-mask bridges and 14 clearance errors.
The pcb-maker board had 54 bridges and 14 clearance errors in both fill
modes. Its normal verdict was incomplete with 19 new solder-mask bridge
errors, and separately recorded 47 findings within individual footprints.

Removing both inner pours from a diagnostic copy left all 54 mask
bridges. Removing the routed tracks and vias left 23. Native shape
checks found a mask graphic overlapping pads of two named nets on
each of nine solder jumpers. Isolated copies of SJ1 and SJ8 reproduced
the polygon-to-pad finding on opposite faces with no routing. Removing
either numbered pad from each diagnostic copy cleared that finding.
SJ8 still had a separate finding between its remaining numbered pad
and its unassigned WIRE pad.

The 19 new records in the normal verdict name nine pad-entry tracks,
one on each of SJ1 and SJ5 through SJ12. The contacted pads lie wholly
inside their existing explicit mask openings, with a minimum inset of
0.0508 mm or 0.0635 mm. Moving an outer-layer track's landing point
anywhere on the pad still exposes that contact through the same opening.

Three controlled trials changed only the +5V routing into SJ1 pad 2.
They kept the board's footprint, pad, mask and zone geometry and used
the same project rules. The standalone native rescore reported 54
baseline bridges across the board.

| SJ1 routing | KiCad opens | Raw DRC errors | Mask bridges | SJ1 mask findings |
| --- | ---: | ---: | ---: | ---: |
| Original | 0 | 68 | 54 | 4 |
| Remove the reported track | 1 | 66 | 52 | 2 |
| Shorter track entering the far edge of pad 2 | 0 | 68 | 54 | 4 |
| Two through vias and a back-layer escape | 0 | 68 | 54 | 4 |

The deleted track disconnected +5V from pad 2. The shorter track
retained both of its bridge records. The back-layer escape replaced
the two track records with via records at the same opening. Both
connected trials still reported 54 bridges. The other eight jumpers
share the same pad-enclosure evidence. The alternate-routing check
was limited to SJ1. The geometry witnesses, exact route deltas and
native reports are in `build/qplayer-router-mask-pilot-20261119/`.

### Little Red Rover

The two-layer rover controller exercises placement around seven connectors,
three mounting holes and four switches. The benchmark holds their 14 poses
and all 65 assembly faces from the designer PCB. The other 51 parts can
move. The donor provides the footprint geometry, 58 outline primitives,
one keepout and the stackup. The import starts with zero tracks and vias.
The donor and schematic agree on all 64 multi-pad net groups.
[Mechanical brief](little-red-rover-mechanical.json) · [Upstream revision](https://github.com/little-red-rover/little-red-rover/tree/1509a7a94df51ac4be854c543c8866e2a85803f4/HARDWARE/little_red_rover)

```sh
case_id=little-red-rover__little-red-rover__little_red_rover
schematic="benchmarks/real/external/github-sch/$case_id/repo/HARDWARE/little_red_rover/little_red_rover.kicad_sch"
donor="benchmarks/real/external/github/$case_id/little_red_rover.kicad_pcb"
w=build/little-red-rover
mkdir -p "$w"
printf '%s\n' '{"add_pours":[{"net":"GND","layers":["F.Cu","B.Cu"]}]}' > "$w/ground.json"
PCB_WORK_RATE=3.15e6,150e6 RAYON_NUM_THREADS=2 python3 benchmarks/from-schematic/run.py "$w/layout" \
  --schematic "$schematic" --footprints-from "$donor" --outline-board "$donor" \
  --rule-areas-from-outline --stackup-from "$donor" --constraints benchmarks/from-schematic/little-red-rover-mechanical.json \
  --router-config "$w/ground.json" --total-seconds 240
```

On November 20 both the unpoured control and the requested two-face GND
layout routed 64/64 with zero opens. Adding GND reduced trace length from
5,519.22 to 5,071.15 mm and increased vias from 174 to 192. The GND run
took 128.70 modeled seconds. The independent scorer matched all 65
footprint geometries, the poses, faces, stack and keepout. It rejected
both boards with four USB connector hole-clearance errors and two
footprint ID mismatches against the original schematic at U5 and U6.
The import's copied schematic used the generated library nickname and
therefore had zero parity findings against the same layouts.

The historical footprint import now keeps the authored IDs at U5 and U6.
The project library gained the one missing WSON model and carries all 113
installed `Package_SON` models byte for byte. The copied schematic also
matches the original bytes. The fresh unpoured run reproduced the same
physical board in 138.65 modeled seconds, with 174 vias and 5,519.22 mm
of track. Saved and refilled checks against the original schematic
reported zero parity findings and zero opens. The independent scorer
still rejected the board for the four source USB hole-clearance errors.
A repeated Rover import kept all project overlay models. An Urchin holdout
added missing `SW_RESET` directly to its staged `lib` folder and preserved
all six existing models and the original schematic. A second Urchin
donor had a 0.20 mm pad-width difference between its two same-ID reset
switches. Their fallback models kept separate names and geometry. The
logs and audits are under `build/rover-cold-20261120/historical-footprint-*` and
`build/rover-cold-20261120/historical-local-urchin-*`.

The import report also records a footprint-specific clearance of 0.15 mm
at U9, where adjacent pads sit closer than the default class's 0.20 mm.
Restoring the donor's inherited clearance on the unpoured output added
13 pad-to-pad errors inside U9. The four hole-clearance errors occurred
on the original designer board as well.

Add `--keep-footprint-clearances` to both layout and `--candidate-board`
when measuring against the inherited 0.20 mm rule. In matched cold runs
the two imported PCBs differed by one U9 clearance field. The final
boards preserved all 65 footprint poses, 2,055 segments, and 257 vias.
Both routed 64/64 with zero opens and zero parity findings against the
copied schematic. The saved PCB without the override had exactly 13
additional U9 clearance errors, at 0.150 or 0.175 mm against the
0.200 mm requirement. The four USB connector hole-clearance findings
and all 59 warnings matched. The independent comparison flagged the
generated override on the old board and verified the preserved policy
on the board without it. See `build/rover-cold-20261120/policy-cold-audit.json`.

The antenna placement trial adds a region and rotation to the same
14 holds and 65 assembly faces.

```sh
jq '.rotation = [{"part":"U10","angle":-90}] |
    .region = [{"parts":["U10"],"origin":"absolute","x":[192,213.5],"y":[59,141]}]' \
    benchmarks/from-schematic/little-red-rover-mechanical.json > "$w/antenna-region.json"
```

Run the two-face GND command above twice with fresh output directories
and `--keep-footprint-clearances` on both. Supply the original mechanical
brief to one and `--constraints "$w/antenna-region.json"` to the other.
Both layouts routed 64/64 nets. The changed brief reduced track length
from 4,992.25 to 4,221.04 mm and vias from 201 to 142. KiCad polygon
intersection measured 0 and 74.690 mm² of overlap between U10's antenna
keepout and the separate board keepout. Both returned boards retained
17 footprint DRC errors under the source rules.

The verdict reads class assignments from the saved project. With the
`BatteryFeed` class on `/bat_+`, it reports 63 routed nets in Default
and one in BatteryFeed. The assigned 1.0 mm width is a preference.
The router used 22.686 mm of 1.0 mm copper and 2.317 mm of 0.127 mm
copper, with no native width error. The designer's track between the
same J6:1 and Q2:3 terminals is 39.193 mm, all at 1.0 mm. A separate
rule explicitly requiring at least 1.0 mm produced a 25.004 mm route
entirely at that width. All three checks had zero opens and zero parity
findings. A two-class +3.3V and +5V experiment reported 67.772 and
28.346 mm respectively below their 1.0 mm preferred width, with zero
native width errors. KiCad readback agreed with the reported length in
each layer and width group. See `build/rover-cold-20261120/class-audit.json`.

With an explicit reference, the verdict measures the overlap between
U10's named antenna keepout and the retained copper keepout on both
layers. The designer placed 77.448 of the antenna's 78.540 mm² over that
strip. The unheld GND layout had zero overlap. An alternate pose kept
73.544 mm² in the strip. The earlier placement model added a 0.5 mm
board-edge margin to the pad rule area and rejected the original pose
against the other new poses. For a pad rule area covering the complete
copper stack, the placer now tests its polygon against the modeled pad
envelopes. The original pose passes with the other poses held.
Translating only U10 by 1.0 mm into the strip gives 17 native pad keepout
errors, at the same pads whose measured copper shapes cross the polygon.
A separate layout with the original pose held restored 77.448 mm²,
reduced track length from 5,071.15 to 4,405.00 mm and reduced vias from
192 to 145. Both layouts routed 64/64 with zero opens and the same
four inherited USB connector hole-clearance findings. The overlap
report describes the reference placement and the kept area geometry.

### Ligra back carrier

The two-layer carrier from `Jana-Marie/ligra` revision
`8a98f3938c20fe3ed07bfb34e7d43f243e21f519` has 59 schematic components.
The [mechanical brief](ligra-back-mechanical.json) takes 15 poses and faces
from the four mounts, eight connectors, status LED, programming interface
and switch. Layout chooses the other 44 poses. The donor supplies 16 board
edge items and the exact footprint models, including the USB connector's
local cutout. The imported project starts with zero tracks, vias and
copper zones. The source's off-panel decorative lock appears in
`import.json.mechanical_from.unmatched_locks`.

Run against the pinned checkout or the matching schematic and PCB archives.
Use KiCad 10 with matching installed footprint libraries. The [layout
configuration](ligra-back-layout.json) gives each arm 220 modeled seconds.

```sh
src=benchmarks/real/external/github-sch/Jana-Marie__ligra__ligra_back/repo/ligra_back
pcb=benchmarks/real/external/github/Jana-Marie__ligra__ligra_back/ligra_back.kicad_pcb
bench=benchmarks/from-schematic
bin=target/release/pcb-maker
out=build/ligra-replay
mkdir -p "$out"
export PCB_MAKER_KICAD_USER_LIBS=1 PCB_WORK_RATE=3.15e6,150e6 RAYON_NUM_THREADS=2

for arm in default strict; do
  policy=()
  if [ "$arm" = strict ]; then policy=(--keep-footprint-clearances); fi
  "$bin" layout-kicad-schematic "$src/ligra_back.kicad_sch" ligra_back "$out/$arm" \
    --constraints "$bench/ligra-back-mechanical.json" \
    --layout-config "$bench/ligra-back-layout.json" \
    --mechanical-from "$pcb" --footprints-from "$pcb" --layers 2 "${policy[@]}"
done

returned="$out/strict/layout/result/ligra_back.kicad_pcb"
cmp "$src/ligra_back.kicad_sch" "${returned%.kicad_pcb}.kicad_sch"
cmp "$src/ligra_back.kicad_pro" "${returned%.kicad_pcb}.kicad_pro"
kicad-cli pcb drc --format json --severity-all --schematic-parity \
  --output "$out/strict-stored.json" "$returned"
kicad-cli pcb drc --format json --severity-all --schematic-parity --refill-zones \
  --output "$out/strict-refilled.json" "$returned"

python3 "$bench/run.py" "$out/strict-score" --binary "$bin" \
  --schematic "$src/ligra_back.kicad_sch" --layers 2 --outline-board "$pcb" \
  --footprints-from "$pcb" --constraints "$bench/ligra-back-mechanical.json" \
  --keep-footprint-clearances --candidate-board "$returned"
```

Both arms kept all 30 pose and face constraints and routed 33/33 connections
with 1,372.044 mm of track, 11 vias and 38.193 modeled seconds. Saved and
refilled native checks recorded zero opens. The original-schematic scorer
verified zero parity findings and all outline, layer, footprint geometry and
placement checks. Its raw-error gate rejects both arms. Default import adapts
JP1's clearance to 0.10 mm and leaves four active findings in J10's cutout and
J11's hole geometry. The strict arm preserves the authored project rules and
reports those four plus two JP1 pad-clearance findings. The designer PCB has
13 active findings, zero opens, 1,062.862 mm of track and 251 vias, including
ground stitching on its filled zones.

The prior importer assigned aliases to 28 capacitor and resistor instances
of four missing `otter` models. Its copied schematic passed native parity,
and the original schematic reported 28 footprint identity mismatches. The
current import restores their authored IDs and one representative model per
ID. The 28 IDs are the only changed fields across the 59 footprint forms.
Copper forms are identical, and the returned schematic matches the
source bytes. Reimport retains all 59 IDs and an existing project library
keeps its exact model files. A 0.020 mm edit to C1's pad width in a private
donor control still stages the nine divergent `otter:C_0603` instances under
separate IDs, preserving the edited model.

An alternate strict brief exposed a missing placement obstacle. Add
`near` entries for C1 to U7:5 at 1.5 mm, C5 to U4:3 at 3 mm, and C2 to
U4:2 at 3 mm, using each capacitor as `part` and each IC pin as `pin_of`.
J10's four lines and four arcs form an interior cutout at its held rear
pose. The initial import stacks the footprint away from that location.
Before the fix, the placer seated Q2 with its pad 3 touching one of the
cutout arcs. Establishing the cutout at J10's absolute hold before placement
moved Q2 clear. Native KiCad found this difference before routing and after
routing, both as saved and after refill. The new layout has six inherited
errors, down from seven total, and retains zero opens and parity findings.
It still misses C1's requested near distance at 3.672 mm, so the workflow
reports it as incomplete. Both arms retained all 15 original holds and the
perimeter. They produced 1,492.331 and 1,471.551 mm of track respectively,
with 19 vias in each and 30.864 and 31.598 modeled seconds.

The following four runs use that same strict three-capacitor brief, cutout
handling and 220-second budget. Each distance is the native measurement
from the C1 body to the centre of U7 pin 5 on the returned board.

| Placement grid | Routing halo around C1 and U7 | Distance (mm) |
| --- | --- | ---: |
| 0.635 mm | Default | 3.6725 |
| 0.635 mm | 0 mm | 1.7675 |
| 0.1 mm | Default | 3.4875 |
| 0.1 mm | 0 mm | 1.4875 |

To reproduce the last row, add `"tuning":{"grid_mm":0.1,
"halo_overrides_mm":{"C1":0,"U7":0}}` inside `placer` in the layout
configuration. This changes the space reserved for pin escapes at two
components. Board clearances and the original project rules still apply.
The public workflow routed 33/33 nets with 1,337.622 mm of track and 22
vias in 74.153 modeled seconds. Stored and refilled KiCad checks both
found zero opens, zero parity findings and six inherited errors.

C5 measured 2.658 mm from U4 pin 3. U4 has two physical pads numbered
2 on the same net. C2 measured 2.941 mm from the lead and 8.865 mm from
the tab. The independent returned-board scorer reports this target as
ambiguous when asked to verify the entire three-capacitor brief.

On the default grid, the schematic workflow's local retry moved only C1
and retained copper on 31 nets from its first routing attempt. The saved
board has 21 vias and 1,463.288 mm of track. Both attempts together used
48.21 modeled seconds. An independent check of this board against the
15 held poses, 15 face requests and C1's 1.5 mm limit passed all 31
checks. C1's body was 1.4875 mm from U7:5 and the direct supply trace
measured 0.750 mm. The original schematic had zero parity findings and
KiCad found zero opens. The six inherited errors still fail the raw DRC
gate.

On the same saved boards, `inspect-routing-run --near-paths` measures
the F.Cu trace from C1:2 to U7:5 at 2.972899 mm with the default grid and
halo, and 0.785708 mm with the 0.1 mm grid and the two halo overrides.
Both paths have zero intermediate pads and vias. A separate +3V3 path
from C14:2 to RV1:3 on the default-grid board visits both physical U5:2
pads and reports their separate UUIDs and centres. A request that names
U5:2 as an endpoint is ambiguous.

Rerunning the 0.1 mm grid case with its own result as `--reuse-routes`
retained all 33 nets, 578 segments and 22 vias. Search expansions fell
from 75,276,218 to zero. Modeled work fell from 74.15 to 65.20 seconds,
including placement and verification. Supplying the board from before
the three constraints were added caused the pose-reuse check to identify
all three misses. The cold fallback and the unchanged-input reuse run
both returned PCB files byte-identical to the 0.1 mm grid result.

### ProtoESP breakaway panel

The two-layer ProtoESP project tests placement across ten inferred breakaway
sections. The [mechanical brief](protoesp-mechanical.json) holds 15 authored
poses, including the prototyping grid and four fiducials. Six named additions
preserve four board-only tooling footprints and two graphics. Select the 77
mouse-bite footprints by donor UUID because they share the reference `REF**`.
The schematic, donor and KiCad Python bindings used by the local corpus replay
are required. From the repository root, in Bash:

```bash
work=build/protoesp-public-replay
case=CRImier__MyKiCad__protoesp
donor="benchmarks/real/external/github/$case/protoesp.kicad_pcb"
sch="benchmarks/real/external/github-sch/$case/repo/ESP8266/protoesp/protoesp.kicad_sch"
mkdir -p "$work"
/usr/bin/python3 - "$donor" > "$work/mouse-bites.txt" <<'PY'
import sys
import pcbnew

board = pcbnew.LoadBoard(sys.argv[1])
holes = sorted(part.m_Uuid.AsString() for part in board.GetFootprints()
               if part.GetFPIDAsString().startswith("protoesp:mouse_"))
assert len(holes) == 77
print(*holes, sep="\n")
PY
args=()
while read -r id; do args+=(--mechanical-footprint "$id"); done < "$work/mouse-bites.txt"
for modules in true false; do
  jq -n --argjson modules "$modules" \
    '{total_seconds:300, placer:{constraints:"constraints.json",keep_in_modules:$modules},
      tuning:{move_seconds:120}, polish_seconds:12}' > "$work/config-$modules.json"
  PCB_WORK_RATE=3.15e6,150e6 RAYON_NUM_THREADS=2 pcb-maker layout-kicad-schematic \
    "$sch" protoesp "$work/modules-$modules" --layers 2 \
    --mechanical-from "$donor" --footprints-from "$donor" --keep-footprint-clearances \
    --constraints benchmarks/from-schematic/protoesp-mechanical.json \
    --layout-config "$work/config-$modules.json" "${args[@]}" || cat "$work/modules-$modules/workflow.json"
done
```

The November 20 grouped baseline stopped with ten unplaced parts. With module
grouping disabled, prioritizing the last unplaced IC (U6) seated all 228
footprints. Native readback of the returned board matched all 98 held poses
and the geometry of 263 mechanical pads. U6 moved to a different breakaway
section. KiCad reported 102 unconnected items across 52 nets and six schematic
pins marked no-connect whose pad numbers are absent from the donor's U6 footprint.
The ungrouped result measures placement recovery on the panel as a whole.
Use the grouped run to judge placement for a design that must work after separation.

The archived layout distributes the 130 movable footprints across all ten
sections. Automatic grouping assigned them to four sections, matching the
archived section for 62 footprints. Supplying section boxes and the archived
membership for all 130 parts left 12 unplaced. Also specifying the archived
rear face for 11 components reduced that to eight. Both runs required U6 in
its authored section and left it unplaced. At its authored pose, U6's body
covers the held FID2 optical opening. In the inferred section, the fixed
FID2, SW1 and SW3 together exclude
every front-side center for U6 at all four allowed rotations under the placer's
body and clearance model.

Two alternate mechanical briefs tested the resulting advice under the same
section assignments and work budget. Moving only FID2 from (129.7, 69.6)
to (124.3, 59.5) mm seated U6 on the front at (132.0, 72.5) mm. Keeping
FID2 in place and requiring U6 on the rear seated it at (121.8, 74.3) mm.
KiCad reported no active placement-board error involving U6 in either test.
Both alternatives left five other parts unplaced. The first assumes the
assembly process can use the moved fiducial. The second assumes adequate
rear assembly and antenna clearance.

The [section and assembly constraints](protoesp-sections.json) specify the
archived section of each movable component, the eleven rear placements and
the pad-only nature of P1. P1 is the PCB's prototyping grid with 948 pads.
It has no separate housing, courtyard, graphics or 3D model. With an explicit
`hollow` declaration, its pads and drills still obstruct other pads and holes.
The body of the grid previously blocked through-hole parts anywhere inside its
bounding rectangle, including parts on three separate breakaway sections.

The following uses the variables and mouse-bite selectors from the first
command. The first run retains all archived fiducial poses. The second tests
the alternate FID2 position. Both start with an unrouted board.

```bash
jq -s '.[0] * .[1]' benchmarks/from-schematic/protoesp-mechanical.json \
  benchmarks/from-schematic/protoesp-sections.json > "$work/sections.json"
jq '(.place[] | select(.part == "FID2")) |= (.x=124.3 | .y=59.5)' \
  "$work/sections.json" > "$work/sections-alternate.json"
for brief in sections sections-alternate; do
  PCB_WORK_RATE=3.15e6,150e6 RAYON_NUM_THREADS=2 pcb-maker layout-kicad-schematic \
    "$sch" protoesp "$work/$brief" --layers 2 --mechanical-from "$donor" \
    --footprints-from "$donor" --keep-footprint-clearances \
    --constraints "$work/$brief.json" --layout-config "$work/config-true.json" "${args[@]}" \
    || cat "$work/$brief/workflow.json"
done
```

With the archived fiducials U6 was the only unplaced component, down from
five on the same brief. Moving FID2 seated all 228 footprints. The measured
section for each of the 130 movable components matched the archive. The
returned board met every placement constraint and had four native open
connections, six missing-pad parity findings on U6, and seven strict DRC
findings. Placement, routing and verification used 233.88 modeled seconds
of the 300-second budget.

With the alternate fiducial brief, a separate run requested a fresh GND
pour on each copper layer via `--router-config` and enabled the configured
KiCad libraries with `PCB_MAKER_KICAD_USER_LIBS=1`. The donor also uses
GND on both layers. The public workflow repaired RXD, saved the zone fills
and returned a board with two native opens in 225.07 modeled seconds. The
stored and refilled checks both found a break in /5D3 and a disconnect
between the front and back GND zones. Both retained six parity findings
and ten raw active errors.

### Explorer shield numbering

The two-layer Explorer exercises a hierarchical schematic with 259 electronic
footprints and seven board-only markings. The [mechanical brief](explorer-mechanical.json)
fixes 23 connector, switch, fiducial and mounting-hole poses. It selects the
seven markings by donor reference and position because they all share the
reference `G***`. The donor supplies eight outline primitives and two rule
areas. [Source revision](https://github.com/anyshake/explorer/tree/0404bea927f25ca75e0c832fd0a33bcd1ed13b22/hardware)

```sh
case_id=anyshake__explorer__Explorer
archive=benchmarks/real/external
sch="$archive/github-sch/$case_id/repo/hardware/Explorer.kicad_sch"
donor="$archive/github/$case_id/Explorer.kicad_pcb"
out=build/explorer-replay
mkdir -p "$out"
echo '{"total_seconds":420,"placer":{"constraints":"constraints.json","allow_back":["R*","C*","L*"]}}' > "$out/layout.json"
PCB_WORK_RATE=3.15e6,150e6 RAYON_NUM_THREADS=2 PCB_MAKER_KICAD_USER_LIBS=1 \
  pcb-maker layout-kicad-schematic "$sch" Explorer "$out/original" \
  --constraints benchmarks/from-schematic/explorer-mechanical.json \
  --layout-config "$out/layout.json" --layers 2 --mechanical-from "$donor" \
  --footprints-from "$donor" --stackup-from "$donor" --keep-footprint-clearances
```

All 266 footprints were placed and KiCad found zero opens. Two missing
schematic terminals still made the workflow incomplete. J7 and J8 each
have a schematic shield pin numbered `SH` on GNDD, while the donor calls
both physical shield pads on each connector `0`. In a private footprint
donor, renumbering just those four pads to `SH` gave zero opens and zero
parity findings in the routed board. The schematic and project settings
retained their original bytes.

The original project excludes a footprint-type finding at U1's native pose.
It becomes active when U1 moves. A separate brief held U1 at its authored
position, (69.7225, 112.8) mm, with its original face and angle. The run
used the corrected footprint donor and 191.75 modeled seconds. Independent
readback matched all 24 holds, all seven marking poses, the two rule areas,
the outline and the stackup. Both stored and refilled KiCad checks found
zero active errors, zero opens and zero parity findings. The existing U1
exclusion still applied.

### Katia split keyboard

Katia has two keyboard halves and two panel rails. The schematic has
212 electrical footprints. Its 98 held electrical parts include 49
on each half. The remaining 114 movable footprints occur under two
distinct top-level sheet instances. Both halves reuse the flash, LED
and USB sheet files. Matching the instance paths to the held parts
places 57 movable parts on each half. All 114 assignments agree with
the archived positions. [Project revision](https://github.com/ikajdan/katia/tree/bb9ef0334de3d4ef5a090dc711b4e3d06c8da1b7/hardware)

The archived footprints for 46 back-mounted hot-swap sockets draw the
front-side switch courtyard and omit a rear socket courtyard. To
isolate the effect, these trials keep the same sheet ownership using
the archived placement as the piece reference, the original footprint
faces and the same 400-second layout budget.

| Switch model used in placement | Unplaced parts | Socket-to-resistor/capacitor solid intersections |
| --- | ---: | ---: |
| Original footprint geometry | 22 | — |
| Only switch pads and holes block placement | 0 | 7 |
| Private donor with a rear courtyard measured from the socket model | 0 | 0 |
| Author's assembled placement | — | 0 |

The third trial exported the author's STEP model for a single back-side
socket using KiCad with the footprint rotation set to zero. The
projection of its three solids, expanded by 0.25 mm and rounded
outward, gives a local rear courtyard from (−6.835, 0.600) to
(8.196, 6.990) mm. The trial added that rectangle on `B.CrtYd` to each
of the 46 socket footprints in a private donor. The front courtyards,
pads and holes retained their original shapes. For each assembled
placement, KiCad exported 138 socket solids and 72 resistor and
capacitor solids. OpenCascade counted pairs with more than
0.00001 mm³ of common volume. The table describes the placement stage.
[Socket STEP model](https://github.com/ikajdan/katia/blob/bb9ef0334de3d4ef5a090dc711b4e3d06c8da1b7/hardware/models/Cherry-MX_Socket_Hotswap.step)

Repeating the third trial with the two confirmed `sub_board` groups
from the schematic precheck produced a byte-identical placed PCB.
This run used the 98 held electrical parts and sheet membership to
assign pieces. It needed no reference-board placement. Every part
had a legal seat and all reported placement constraints passed.
