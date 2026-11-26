# Benchmarks

What is measured, how to run it, how a row is judged, and what main does
tonight. The runner and its tiers are `benchmarks/agent-tasks/run.py`
([benchmarks/agent-tasks/README.md](../benchmarks/agent-tasks/README.md)
has every tier's details); `compare.py` diffs two runs; `placer_ab.py`
judges a placer change; `scripts/nightly.sh` measures main every night
and `scripts/nightly-report.py` writes the results into this file.
Older measurements and studies are in git history.

## Published Quilter placement and routing comparison

On November 18, 2026 we replayed a public June 10 placement-and-routing
experiment on a two-layer 82.28 × 112.95 mm module board. The published
starter has 14 proxy footprints parked outside the outline, 20 named nets,
and no routed copper. pcb-maker received the original starter, schematic
and project settings, with all parts movable and a 240 s work budget.
We scored all six Quilter candidates that the experimenter posted for
that starter. [Original input revision](https://github.com/vertical-cloud-lab/powder-doser/tree/cfea9ddb597c084dd4d8666bafe55d0c60d6df26/paper/background/starter_board)
and [published candidate files](https://github.com/vertical-cloud-lab/powder-doser/issues/95#issuecomment-4723991102).

The project file bundled with every Quilter candidate specifies 0.1524 mm
minimum track width and clearance. The source project specifies 0.25 mm
width and 0.20 mm clearance. The Power class also changes from a 0.60 mm
preferred width to 0.1524 mm. We ran KiCad 10.0.8 with zone refill and
schematic parity under both sets of rules. Each row for pcb-maker uses
a fresh layout made under the named rules. The six published boards are
the same in both checks.

| Project rules | Layout | KiCad opens | Reported DRC errors | Routed length | Added vias |
| --- | --- | ---: | ---: | ---: | ---: |
| Original | pcb-maker | 0 | 0 | 883.2 mm | 0 |
| Original | Quilter, six candidates | 0 each | 240–269 | 815.0–1084.3 mm | 81–91 |
| Returned | pcb-maker | 0 | 0 | 857.8 mm | 0 |
| Returned | Quilter, same six | 0 each | 23–24 | 815.0–1084.3 mm | 81–91 |

Under the original rules KiCad reported 199 track-width errors per
Quilter candidate. Counting the actual segments found 238–268 below
0.25 mm. Under the returned rules all remaining errors were solder-mask
bridges involving items without an assigned net at U2 or U5. The
candidate boards contain 60.8–105.1 mm of track without a net. Counting
all track, two candidates are shorter than pcb-maker. Counting only
track assigned to a net, five are shorter (745.3–1023.5 mm across the
six). Ground accounts for 215.9 mm of routed track on pcb-maker's board.
All six Quilter candidates use front and back ground pours and zero
ground track length. Track lengths exclude the copper in those pours.
In a separate full placement and route, we asked pcb-maker for ground
pours on both faces using its existing `add_pours` option. With the
original rules it used 587.1 mm of track, down from 883.2 mm. Under the
returned rules it used 589.1 mm, down from 857.8 mm. Both boards had zero
vias, opens and DRC errors. The default runs in the table used the
starter's unpoured board.

All results retained the starter's 21 schematic parity warnings, so
none passes the standalone benchmark's zero-parity gate.
In a diagnostic on separate copies of the six Quilter boards, removing
only unassigned track segments (36–46 per board) and unassigned vias
(one each on two boards) cleared all solder-mask errors under the
returned settings. KiCad still found zero opens and the same 21 parity
warnings. The table scores the original published files.
This is one through-hole module board. The experimenter supplied no
matched Quilter runtime or application-side settings, and the hardware
uses proxy footprints.

## The tiers

The 109 harvested tasks use a board with its tracks stripped, its movable
parts stacked at one point, and a `constraints.json` (tasks v7,
`benchmarks/agent-tasks/tasks/v7`). A layout task runs `layout-kicad-board`.
A route task keeps the designer's placement and runs `route-kicad-board`.
The standalone schematic benchmarks below import the footprints from
libraries and can run from a schematic and a mechanical brief alone.

| Tier | Boards | Mode and budget | What it is for | Who runs it |
| --- | --- | --- | --- | --- |
| Fingerprints | Castor_and_Pollux, k30-SBC, jetson-nano, Sisu_SSE-9 | route mode from the v16 placements (`build/github-layout-v16`), budget scale 0.25, 1 x 4, `PCB_ROUTER_FINGERPRINT=1 PCB_ROUTER_NO_WALL_GUARD=1` | identity: a router change that should change nothing shows identical fingerprints and routed boards | a router package; the nightly |
| Short (`--tier short`) | 12 boards of the quick tier, chosen by how often their result flipped between rounds | layout, a third of the budget | iterating on a placer or layout change; about ten minutes | a placer or layout package; the nightly (3 x 2) |
| Quick (`--tier quick`) | 30 boards that lay out in about 1100 s or less | layout, the full budget | a wider check of a layout change, diffed against its last run | by hand |
| Expansion (`--tier expansion`) | 46 boards of the expansion intake, many designers and tools | route on the designer's placement, 30 s of work (`--budget-scale 10` for 300 s) | a router tier over a wider spread of boards than the harvested ones | a router package; the nightly (8 x 1) |
| Schematic (`--tier schematic`) | 12 harvested boards with a usable schematic | import from the schematic (`import-kicad-netlist`), the task's constraints moved onto the new parts, layout at a third of the budget | the flow an agent takes from a KiCad project: import, libraries, constraints and layout together | by hand |
| Layout sweep | all 109 boards of tasks v7 | layout, 1500 s of work each, `--quality` | the headline: how many boards pcb-maker places and routes to a clean KiCad verdict | the nightly only (10 x 2, `--order peak-desc`) |
| Matched designer tier (`--tier designer`) | the same 109 boards | route on the designer's own placement, 1500 s of work scaled by the median final-ladder share of the previous night's sweep (`nightly-report.py --ladder-share`; 0.695 without one) | splits what the layout leaves open into a placement gap and a routing gap (`compare.py --gap`) | the nightly only (8 x 1, `--order peak-desc`) |
| Endgame (`--tier endgame`) | 38 routed results a nightly left 1 to 7 KiCad-unconnected items short (the sweep of 2026-11-18 and the designer tier of 2026-11-24), frozen under `build/endgame-tier/v1` | the routed board resumed with its copper kept (`route-kicad-board`, 30 s of ladder work, the run's settings and pour mode), then `repair-kicad-opens --local` | an endgame change: does it close more of the last connections; minutes instead of layouts ([README](../benchmarks/agent-tasks/README.md#the-endgame-tier---tier-endgame)) | a router package (3 x 2) |
| From schematic (`benchmarks/from-schematic/run.py`) | the KiCad demos whose footprints the libraries have | import, then `layout-kicad-board` with the starter `layout.json` unchanged | the whole way from a schematic with nothing but the starter files | by hand |
| [Schematic with a mechanical brief](../benchmarks/from-schematic/README.md) | one supplied design | project libraries, explicit layer count and placement constraints, optional board outline | verifies the requested poses, KiCad connectivity, DRC and schematic parity together | by hand |
| KiCad demo corpus (`benchmarks/corpus/run.py`) | the KiCad demo projects (`benchmarks/corpus/fetch.sh`) | route and place + route | a small, quick sanity check on boards everyone knows | by hand |

The eleven **dense boards** (the boards of
`benchmarks/agent-tasks/dense-noise.json`: the 0xCB 1337 panel, protoesp,
glasgow revC3, video, OpenESC 4in1-mini and 4in1, SmartSpin2k_Panelized,
DSP-ADAU1452, MegaDrive, SNSP-CPU-01, split72_left) are the placer's
measurement set: layout mode at the task budgets, 3 x 2, `--only` with
their names.

On November 18 the 12-board schematic tier went from 8 to 10 passes under
strict rule 4 at the same layout budget. Correctly matching copied symbol
instances on Bumwings produced the ninth pass. It held all 129 requested
poses and routed all 81 connections. After the import and plated-hole
placement changes, MokyaLora passed on the ordinary layout run with no
follow-up repair. At that stage USB Keypad still had two KiCad opens and
slab-left one. Keeping parts off narrow breakaway rails moved USB Keypad's
C15 onto the finished board. A full matched 12-board rerun kept all ten
passes, reduced native opens from three to one, and reduced new strict DRC
errors from eight to six. All six are track-width-at-connection findings
on USB Keypad. On slab-left two separate same-budget runs with placement
seeds 7 and 101 each routed all 76 connections with zero new errors. These
retries are outside the 10-of-12 default result.
The tier records schematic parity separately from its historical pass count
(40 findings across the 12 boards, including seven on Bumwings).

The November 19 footprint audit found that SW28 and SW29 share a symbol
path in the supplied BumWings PCB. The importer now assigns each donor
footprint once. On both 64-key revisions this restored the 1.5u model
for SW29 and six soldered switch footprints that had been imported as
ten-pad hotswap models. On the 55-key Xiao revision, linking each
copied schematic symbol to its own footprint resolved the remaining
SW24 mismatch. KiCad export of all 12 tier projects preserved all
6,197 net-to-pin assignments and 1,729 symbol references. Every
imported footprint ID matched its copied schematic. A fresh layout of
the 64-key RP2040-Zero revision held all 129 poses and routed 81/81
nets in 49.05 modeled work seconds. KiCad reopened the saved, filled
board with zero DRC errors, opens or schematic parity findings.
The matched 12-board layout rerun retained 10 strict passes, one open,
and six new width errors. Its schematic parity total fell from 40 to
33. The other eleven boards retained their previous per-board counts.

Replaying that November 19 archive accepts six of 12 boards under the
schematic acceptance gate. OpenFC, Urchin, and TelemetryOnboard were historical
strict passes with KiCad parity findings. OpenFC had five `net_conflict`
findings, Urchin six, and TelemetryOnboard 16 footprint filter mismatches.
All five OpenFC findings also occur on the pinned designer board. A
fresh KiCad netlist export places USB1:A8, USB1:B8, U7:2, U15:2 and
Card1:8 on five separate single-pin nets. KiCad gives U7:2 the
`no_connect` pin type and marks the other four `unspecified+no_connect`.
On Urchin, all six returned findings also occur on the pinned designer
board checked against the original schematic. Each names an unwired
symbol pin (31, 32 or 33) missing from the supplied footprint of U1 or
U2. KiCad reported 196 parity findings on the original pair of files.
Matched layout replays now include the fresh schematic evidence in
`missing_schematic_pads`. OpenFC routed 82/82 with zero native opens
and retained all five parity findings. Its verdict identifies four
no-connect markers and U7's no-connect symbol pin type. Urchin reports
six isolated single-pin input nets. A control with Urchin's U1:17 pad
removed still identifies COL0 and its four schematic terminals, even
with an adjacent stale `.net` asserting a no-connect. The verdict keeps the
native parity acceptance rules. See
`build/schematic-pin-guidance-20261120/`.

On TelemetryOnboard, the verdict groups the 16 footprint-filter findings
by assigned footprint ID and filter text, giving 11 groups to review.
In a diagnostic copy, allowing the assigned footprints in the corresponding
symbol filters cleared all 16 findings. With JL1 withheld, KiCad reported
exactly one mismatch, on JL1. All three runs used byte-identical PCB and
project files, kept the same 539 terminal assignments and 140 footprint
assignments, and had zero opens and 26 raw DRC errors. See
`build/telemetry-footprint-filter-replay-20261120/audit.json`.

On AnyShake Explorer, the schematic assigns both D-sub shields J7:SH
and J8:SH to GNDD. Public import identifies the donor's pad number 0 as
the sole candidate for each pin. Each number occurs at two separately
identified plated pads, all four assigned to GNDD on the source board.
The import with this diagnostic wrote the same PCB bytes as its
predecessor. Changing those four numbers to SH in a private donor
control cleared both import findings and all six native net conflicts.
KiCad still reported the same nine unrelated footprint field differences
on the original board and its numbering control. In the matched public
layout runs, independent stored and refilled checks found zero opens
in both arms. Staged schematic parity fell from two findings to zero.
Both boards retain KiCad's footprint type finding on U1. On CM5 Minima, J701:M1
has no candidate among its donor pads, so the import still directs the
reader to review the footprint assignment. On SLAB left, the six missing
no-connect pads produce no donor numbering advice. The evidence is in
`build/schematic-corpus-screen-20261120/explorer/numbering-feedback/`.

On the Katia keyboard panel, the schematic import and the supplied holds
left 114 movable parts without a piece in `--check-sub-boards`. Its two
top-level sheet instances each have 49 fixed electrical witnesses, some
in descendants. Their window boxes overlap the fabrication rails. The
precheck now proposes two `sub_board` entries of 57 parts each using the
actual outline piece at a confirmed point. All 114 memberships agree
with independent KiCad readback of the archived footprint positions in
the two functional pieces. Applying the entries made the precheck
complete with zero unassigned parts and the same four split names as the
separate designer-placement reference control. The ordinary 400-second
layout with those entries left 22 footprints unplaced, down from 38 in
the absence of piece assignments. The original footprint geometry's
rear-body clearance remains a separate limit. Moving one fixed witness
to the other piece suppressed the automatic proposal. Removing C27's
sheet metadata left that part unclassified, while the other 113 still
received suggestions. See `build/schematic-corpus-screen-20261120/katia/`.

Katia's 46 rear-mounted switch footprints include a front courtyard and
lack a rear socket courtyard. In a completed placement using the sockets'
pad geometry alone, STEP solids showed seven intersections between
rear passives and socket bodies. A `placement_courtyard` declaration used
the measured socket solid bounds with a 0.25 mm margin. The ordinary schematic
workflow used the same original donor and confirmed piece assignments,
seated every footprint and proceeded to routing. All 46 switches retained
their authored pose, pad geometry, net assignments and graphics in
independent KiCad readback. The declared courtyard lives in the placement
model. KiCad's STEP export and an independent solid intersection check
found zero overlaps between the 138 socket solids and 72 resistor and
capacitor solids, and zero against the 14 IC and diode solids. Those
measurements used the archived model files for these component groups.
The 400-second layout used 414.239 modeled seconds. All 114 explicit
piece assignments passed and all 230 PCB footprints kept their placed
pose in the routed board. Independent KiCad checks reported 56 stored
and 29 refilled unconnected items, zero active rule errors, and zero
schematic parity findings. The layout counted 25 refilled opens after
identifying four expected cross-piece findings. It had insufficient
budget to publish the fill. See `courtyard-final/` for the outcome and
`solid-study/declared-final-placed-assembly/` for the STEP measurements
under `build/schematic-corpus-screen-20261120/katia/`.

On that same panel, a named local repair (`repair-kicad-opens --local`
with `--net +5V`) found an escape from LED27:4 into the existing In2.Cu
plane. It added two 0.600 mm back traces totaling 1.288 mm and used the
project's 0.50/0.30 mm tented via preset, with the entire ring clear of
the pad. The repair preserved all 230 poses and existing copper. KiCad
proved the +5V pad joined its intended island, with the original ordinary
copper and 45 filled-island attachments preserved. Panel-local opens fell
from 25 to 24 (29 to 28 raw). Independent stored and refilled checks of
the public `--save-filled` copy found 28 raw opens, the same 48 warnings,
zero errors and zero schematic parity findings. The +3V3 control declined
without changing the board. Its U2 pad launches remain blocked under the
current clearance and track rules. The layout verdict's unqualified
`local_open_repair.retry_argv` previously spent four net slots and found
no repair. It now finds the same +5V branch without the agent naming a
net. The repaired PCB matches the named run byte-for-byte. The unqualified
run took 39.9 wall seconds, compared with 36.2 for the named run, and kept
the same four-slot limit. A second +5V retry found no legal path for a
remaining pad. See `power-escape-local/` in the Katia benchmark tree.

The unqualified selector also kept the existing GND bridge on ClearBell
(three to two opens) and matched the Sisu control byte-for-byte after
reconnecting a deliberately removed +3.3V feed at J_LCD1:1 (one to zero
opens). On the RP2040 motor panel neither tested plane had a legal short
escape. The PCB stayed byte-identical with nine opens and the ordinary
four-net ordering. See `power-escape-local/agent-actual-*` for the results.

On the returned board, all 138 socket solids also cleared the 230
LED solids and 48 solids from the four connectors that have STEP
models. The archived placement passed the same checks. The exports
and intersection counts are under `solid-study/all-components-{returned,author}/`.

The earlier 34-open placement tested whether turning on differential
pair routing would help. All 230 placement poses matched between trials.
The planner found both USB data links and declined both because their
stubs would block the middle pin on U5 or U9. The paired run also ended
with 34 opens and 454 vias. A reduced board containing J7, U5 and R20
retained the connector notch and the original net classes. Coupling
closed its last USB open, with 39 native clearance warnings compared
with 26 when routed independently. Six pad-to-pad warnings already
existed on the bare board. When clearance was set to error, both trials
left five native opens and retained the six input clearance findings.
The full and reduced trials are under `pair-comparison/` and
`usb-launch-coupon/`.

The coupon's paired verdict now reports the 33 copper-related warnings
separately from those six pad-pair warnings. KiCad measured a minimum
USB_D track or via gap of 0.1026 mm against a class clearance of
0.4000 mm. Each group's examples carry native item IDs, nets and
locations. The warning-severity and error-severity replays retained
the same respective PCB bytes and verdict outcomes as before the
feedback change. See `usb-launch-coupon/clearance-feedback-verification.json`.

On the separate Katia trial with the measured courtyard drawn into
a private copy of the socket footprints, the stored and refilled
native partitions agree for all 5,893
ordinary copper objects in 204 groups. Some changed open witnesses name
zone or teardrop copper, including +3V3 teardrops on F.Cu and B.Cu.
The original handoff refused because the ordinary partition could only
verify pads, tracks and vias. The current `--save-filled` checked all 45
native island shapes and their exact attachments against the captured
refill. It published a source-preserving copy with 32 raw opens, four
errors and zero parity findings, the same as the refilled DRC. A control
with identical ordinary components and raw open counts but one altered
island attachment fails the proof. July's independent stored fill control
delivered with zero opens, errors and parity findings. See
`solid-study/{partition-study,zone-witness-original-public}` and
`power-escape-local/{ship-legacy-witness,island-swap-control}` in the
Katia benchmark tree.

0xCB-Static had clean parity and a historical strict pass. Its layout
exited 1 with two solder-mask bridge errors. KiCad locates both on the
same pair, the non-plated hole in MX41_1 and pad 1 of MX41 on COL4. The
mechanical brief holds those footprint origins at (183.0400, 137.1650)
and (187.8100, 137.1600) mm. At those poses the drilled hole reaches
0.399 mm inside the plated pad's copper outline, even with zero mask
expansion. The same front and rear findings were present before routing.
Removing that unnumbered hole from a diagnostic copy removed both
findings while KiCad still reported zero opens and parity issues.
The production verdict identifies the two held footprints and their
constraints. A November 20 layout readback regression had lost the
`add.from_board` hold after initial placement. Keeping that hold through
the layout restores the exact geometry advice. The cold rerun produced
the same saved PCB in 83.118 modeled seconds. KiCad found the same two
strict bridges and zero opens or parity findings. See
`build/0xcb-static-mask-20261119/readback-holds-audit.json`. The historical
scores remain 11 of 12 for `pass` and 10 of 12 for `pass_strict`.
[Schematic acceptance gate](../benchmarks/agent-tasks/README.md#the-schematic-tier---tier-schematic)

The November 20 cold rerun completed all 12 layouts in 959 wall seconds
at the same work rates, task inputs, placement constraints and budgets.
Each board matched its November 19 schematic acceptance, open count,
schematic parity count and new strict errors. Six passed schematic
acceptance and ten passed strict rule 4. The remaining native open
was on slab-left, and all six new width errors were on USB Keypad.
The tier reported 33 schematic parity findings.

The subsequent 12-board cold run included the bounded placement fallback
and two-layer open repair. At matched inputs and budgets, all 12 boards
retained their individual acceptance, strict results, open counts,
parity counts and new strict findings. The run consumed 2,841 modeled
seconds. One board lacked native verification of its saved copper.

The next matched cold run (`schematic-regression-20261120-verified-recovery`)
completed in 867 wall seconds. Ten boards still passed strict rule 4.
Schematic acceptance fell from six boards to five. On MokyaLora, the
router connected all 262 modeled links, but the remaining layout budget
could no longer cover saving and checking the zone fills. KiCad found
158 open connections on the delivered PCB and zero after refill. The
other eleven boards retained their schematic acceptance results.
Independent KiCad readback across the twelve delivered boards matched all
469 absolute `place` constraints, 18 absolute positions for added parts
and 549 requested footprint faces. For repeated references the audit used
the UUID from the placement constraint. Both input footprints carrying a
KiCad lock retained their poses.

Reserving finish time before optional placement and routing improvements
restored schematic acceptance to six of twelve on the same inputs,
work rates and budgets. MokyaLora's ordinary run produced the previously
recovered PCB byte for byte in 454.04 modeled seconds. Independent
stored and refilled checks both found zero opens, five inherited errors
and zero parity findings. Strict rule 4 still passed ten boards. Nine of
the twelve delivered PCBs were byte-identical to the preceding run.
The initial finish reservation cost 58 additional vias across le_chiffre
and coco2. A second full comparison let optional polishing use the time
above the native-check reservation. It retained all six schematic and
ten strict routing passes, reproduced ten original PCBs byte for byte
and reduced the additional vias to six. Le_chiffre returned to its
original 60 vias and 4,481.5 mm. Coco2 used 361 vias and 19,488.5 mm,
compared with its original 355 vias and 19,439.3 mm. MokyaLora retained
the same verified saved fills.

A separate repair on the original slab-left placement tested whether its
last I²C clock connection could close with all 129 poses fixed. Rerouting
five nets and joining two pairs of copper regions yielded a saved board
with zero opens and zero raw DRC errors. All 82 placement constraints and
the six inherited schematic parity findings were preserved. Track length
increased from 7041.531 to 7388.383 mm and vias from 318 to 350. KiCad
also reported five additional dangling-copper warnings. This result took
several guided retries and extra native checks outside the tier budget.

The ordinary Slab-left layout now reports a completed same-board search
for its remaining I²C open and omits the standalone retry command. The
standalone replay ran the same seven searches and 1,257,140 expansions,
and its saved PCB matched the layout byte for byte. The layout used
304.573 modeled seconds, including 0.018 seconds for the equivalence
check. The alternate placement command still offered seed 101. That
run routed 76/76 connections in 384.592 modeled seconds, with zero
native opens or errors after saving and reopening the fills. Its six
schematic parity findings were inherited. Evidence is under
`build/slab-retry-feedback-20261120/`.

In a matched 153-second Slab-left seed-101 retry, the original layout
left 10 opens in its stored board despite zero after refill. The layout
with a final-check fill receipt produced the same prefill PCB and spent
one additional 23.6-second native check to publish its fill. Independent
stored and refilled checks found zero opens, zero active errors, six
unchanged schematic parity findings, and identical violation multisets
to the control's refill. All 3,992 non-zone board forms matched.
Reported work was 133.624 seconds on the retry. At 148 seconds, the
placement-pool work left less than one native check. At 159 seconds,
an additional placement probe also exhausted that margin. Both
controls kept the standalone fill action. See
`build/placement-remainder-20261120/slab-receipt-native-audit/`.

The standalone mechanical-brief run on Spell Tome held the specified
switch pose, routed 18 of 18 connections, and finished with zero KiCad
DRC errors, opens, or schematic parity findings. The example in the
[runner guide](../benchmarks/from-schematic/README.md) reproduces its
inputs, including the outline and two-layer stack.

The [one-command schematic layout](agents.md#0-from-a-schematic) also
completed Spell Tome with the supplied enclosure keepout and exact SW2
pose, plus an explicit J2 hold matching the mechanical donor. Separate
ordinary import and layout commands produced byte-identical imported and
routed boards. Both used 21.87 modeled seconds and routed 18 of 18
connections with all three constraints met. Fresh KiCad checks on the
stored and refilled board found zero errors, opens, or parity findings.
The returned project kept all 12 schematic, project, and library files.
The original HackRF One checkout stopped at import and reported 395
missing footprint instances. A separate Spell Tome run with an omitted
J2 hold stopped at import and reported the exact donor pose. Explicitly
releasing J2 in `placer.free` completed layout and recorded the release.
See `build/schematic-layout-command-20261120/audit.json`.

ClearBell's cold layout stopped with two movable parts without a legal
placement, including its ESP32 module U2. U2's footprint contains the
antenna keepout. Its courtyard extends 15 mm past the designer board's
top edge at the antenna end. Adding the template's single U2
`edge: any`, `flush: true`, `overhang: {}` entry let all parts seat in
ordinary layout, with all eight constraints met. The three connector
poses, speaker and two mounting holes stayed in place. The returned
board still had three opens on the speaker outputs and ground, with
zero native errors and parity findings as saved and after refill. This
was a placement recovery. See `build/clearbell-schematic-20261120/`.

On that same generated ClearBell placement, the ordinary local search
could not reach U3's ground pad 11. Its same-net exposed pad 17 already
connected to the ground copper. The verified local bridge added one
1.435 mm track on F.Cu at the project's 0.15 mm width floor. At the same
240-second budget, the cold schematic run went from three to two saved
and refilled opens with zero native errors and parity findings. Modeled
work rose from 93.48 to 125.88 seconds, including seven additional
repair checks and two additional fill connectivity checks. The two
speaker nets remained open.

The same 240-second [cold schematic workflow](../benchmarks/from-schematic/README.md#clearbell-indoor-unit)
closed all three opens with the verdict's `prepared_row_escape_pads`
retry. It put down two 1.0 mm F.Cu speaker launches before routing, then
the local ground bridge. An independent KiCad run found zero opens,
errors, and parity findings as saved and after refill. The matched
control and retry had identical poses, pad mappings, outline,
schematic, and rules. The saved track length went from 556.54 to
572.29 mm with 53 to 59 vias, and modeled work went from 100.68 to
118.99 seconds. See `build/joint-pad-escape-20261120/integrated-audit.json`.

The independent returned-board checker accepted the same ClearBell
board under the original project and all ten mechanical checks. U2's
antenna keepout sits 0.5 mm outside the right edge, clear of board
material. Moving U2 inward produced 103.707 mm² of overlap. Moving H1
or removing LS1 failed the separate donor-instance checks. On the USB
cutout MioLink brief, the returned board passed both native modes with
19 authored DRC exclusions. Moving J1 by 0.2 mm invalidated the
location-specific footprint-type exclusion and failed its held pose.
See `build/clearbell-schematic-20261120/scored-final-integrated/` and
`build/miolink-schematic-20261120/scored-final-{positive,moved}/`.

On ZX Multisound, the ordinary search still reduced 32 opens to 29
without a bridge.
On MioLink, the selected USB net stayed open and the repair declined a
single-net trial under its coupling guard. See
`build/same-footprint-holdouts-20261120/`.

The [schematic workflow](agents.md#0-from-a-schematic) used unspent work
to try one alternate placement on the fixed-USB-cutout MioLink brief.
The first layout used 65.220 of 300 modeled seconds with three opens.
Seed 101 got a 234-second allowance and used another 42.835 seconds
to close all three. The returned board passed stored and refilled
native checks with zero active errors or parity changes and all holds
intact. The ClearBell control used 191.008 of 240 seconds across both
attempts and kept its original result when both placements had three
opens. See `build/placement-remainder-20261120/{mio-auto,clearbell-auto}/workflow.json`.

The workflow now takes ClearBell's exact prepared-speaker-escape
advice when it has unspent work. The first attempt used 100.682
modeled seconds and the retry used 98.000, for 198.682 of 240.
The selected result closes all three opens, passes all ten
independent mechanical checks and has zero active errors or parity
findings in both copper states. MioLink still selected seed 101
with the same winning PCB and 108.055 combined seconds. See
`build/placement-remainder-20261120/{clearbell-row-auto,mio-row-control}/workflow.json`.

The Slab left control's alternate had zero refilled opens and ten
stored opens. The workflow's acceptance gate rejected the archived
alternate because it had no verified stored fill. The incumbent had
one open. Both candidates retained six schematic parity findings. See
`build/placement-remainder-20261120/slab-proof-from-actual-reports.json`.

On MioLink's localized capacitor trial, the placement config holds
116 of 117 footprints with `move_all` enabled. The corrected ordinary
placer exposes only C33 as movable, down from 106 movable footprints.
Native readback confirms all 116 held poses match the input both when
named individually and when the four connector holds use `J?`.
C33's near check remains 19.130 mm against the 1.5 mm limit. See
`build/miolink-schematic-20261120/config-fixed-native-audit.json`.

The independent returned-board scorer also measured the same maximum
as a body-to-pin constraint against the original MioLink project.
The clean cutout board missed it at 5.292344 mm. The reflowed board
passed at 1.33 mm and had zero active DRC errors, opens or parity
findings in both copper states. An earlier routed placement at the
same 1.33 mm failed the native gate with five opens. See
`build/miolink-schematic-20261120/scored-near-{original,reflow,exact-prior}/`.

Ligra back's 59-footprint, two-layer schematic layout left one body-to-pin
request unmet. The ordinary workflow spent 48.213 modeled seconds of 220
across the initial layout and its fixed-neighbour retry. The retry moved
only C1, reducing its gap to U7:5 from 3.6725 to 1.4875 mm against a
1.5 mm limit. It retained 31 complete net routes. Saved track length went
from 1471.551 to 1463.288 mm and vias from 19 to 21. Independent stored
and refilled KiCad checks reported zero opens and parity findings, the
same six inherited errors, and 65 warnings in both modes. C5 remained
2.676907 mm from U4:3. A later replay with literal distance checks kept
a different seed, where C5 was 3.033299 mm from U4:3 against 3.0 mm.
The one-miss retry declined that two-miss input. U4:2 has two physical
centres, so the independent check leaves C2's target ambiguous. An
explicit 0.635 mm grid suppressed the earlier local retry. See
`build/unseen-mechanical-20261120/ligra-back/near-local/` under
`production-with-reuse/independent-native/audit.json` and
`production-explicit-grid/workflow.json`.

The six-layer Eurorack replay found a literal body-to-pin miss at
C14–U8:2, 1.507501 mm against 1.5 mm. The ordinary schematic workflow
moved only C14 by one 0.1 mm grid step and returned a 1.407501 mm gap.
C31–U7:6 stayed at 1.232500 mm. The retry used the courtyard-clearance
model because the brief omitted `spacing_mm`. It charged two native
checks and 7.200 modeled seconds, totaling 392.771 of 600. All 2,091
segments and 181 vias stayed in place. Both native modes had zero opens
and parity findings, one inherited error, and 126 warnings. The verdict
links to the updated placement report and playback. Setting
`spacing_mm` explicitly to 0.5 reproduced the original PCB byte for
byte and skipped the move. A 1.2 mm displacement that also met the
distance limit created one open and twelve additional native errors.
See `build/eurorack-interface-layout-20261120/near-holdout/` under
`fine-near-copper-rigid`, `fine-near-copper-explicit`, and
`near-copper-displaced-exact-rules/audit.json`.

A separate MioLink comparison measured the returned `/TARGET_VREF`
path from backside C33:2 to U10:8. Both boards met the 1.5 mm
body-to-pin limit at 1.315 mm and passed both native copper checks.
The shortest planar trace span went from 59.077 mm through the plated
connector pad J3:2 to 0.329 mm through one via. J3:2 is 26.213 mm
from the direct pad-to-pad line. The measurement excludes spans
inside contact copper and vertical barrel length. See
`build/miolink-local-power-path-20261120/scored-path-{before,after}/`.

The inspector also emitted a guarded trial for the original detour.
Executing its `recovery.argv` retained the original copper, added four
segments and one via, and published verified stored copper. The
inspector measured the returned span at 0.640 mm. See
`build/miolink-local-power-path-20261120/cli-recovery-{before,selected}.json`.

ProtoESP's 228-part replay retains all ten seating failures with
identical placed PCB bytes. Every affected diagnostic now identifies
its automatic breakaway module region, with one grouped action. The
brief contains no authored regions. The modules-disabled control leaves
one unplaced U6 and no module hint. See
`build/protoesp-schematic-mechanics-20261120/module-advice-{on,off}/`.

The fine-grid reseat on the same modules-disabled input placed all 228
footprints and preserved 98 fixed poses. The selected placement spent
3.12 modeled seconds on the reseat and moved U6 plus 128 other parts in
that pass. KiCad independently found the same 375 connections awaiting
routing as on the earlier partial placement and removed two U6-to-CN6
clearance errors. The returned PCB had 102 opens, 10 active errors and
six schematic parity items in both stored and freshly refilled checks.
U6 moved to a different breakaway section, so the modules-disabled
result serves as a placement control. See
`build/protoesp-schematic-mechanics-20261120/u6-space/production/independent-native/`.

With the breakaway regions active, a fresh schematic replay reproduced
the prior placed PCB byte for byte. The verdict names the three fixed
footprints that obstruct U6 on the front of the intended top-right
section. FID2 and the SW1/SW3 courtyards together cover its entire
19.0 × 19.97 mm body's centre domain at all four rotations. U6's
model body at the designer pose also covers FID2. The native footprint
has no courtyard. A scratch placement of just U6 at (120.4, 61.8) mm on
the back passed the placement body's checks in the same section and
removed ten U6 clearance and mask findings while preserving all 227
other poses and six parity findings. That trial establishes a possible
rear seat. It requires a separate decision on mounting, assembly and
antenna location. See `build/protoesp-schematic-mechanics-20261120/`
under `public-fixed-region-cover/workflow/layout/` and
`u6-space/rear-witness-native/audit.json`.

With U6 kept on the front in its original section, the separate
reference brief authorizes an alternate FID2 hold, 11 rear faces, and
the archived section membership. It also declares P1's pad field
hollow. The footprint contains 948 pads and has no physical graphics,
courtyard or 3D model. Honouring the brief's pad-only geometry when
testing through-hole parts seated CN30–CN32 and R17 inside their three
assigned breakaway sections. Both runs preserved the same 98 fixed
poses, and the revised placement kept all 130 region assignments.
Native KiCad found the same ten hard findings and six parity items on
the two placements. After routing, stored and refilled checks both
reported four opens with those same findings. See
`build/protoesp-schematic-mechanics-20261120/public-protogrid-hollow/`
and `u6-space/protogrid-audit/audit.json` in the same benchmark tree.

On that fixed placement, an additive RXD repair reduced four opens to
three. The generated track-relocation command found no candidate for the
three residual opens. Making the other routes transparent in a private
probe found nominal-width `/5D4` and `/5D7` paths through the same lower
breakaway corridor. Each touched copper on four other nets. Removing
either `/5D1` or `/5D6` alone freed a different `/5D4` path, but routing the
displaced net left one open on it. The joint four-net trial from the
three-open board closed `/5D7` and opened `/5D6`, leaving three opens and
the same native hard findings. Four alternate contacts in GND's native
component yielded no path to U1:12 in the bounded search. The experiments
kept the incumbent copper and produced no change to routing policy. See
`build/protoesp-schematic-mechanics-20261120/u6-space/residual-opens/`.

MioLink's separate ten-hold cold schematic run left one +3.3VA
connection between C6:1 and U6:43. The named local retry added 20 copper
objects on +3.3VA and kept all 1,205 original tracks and vias, 117
footprint forms, and the authored outline. The stored-fill handoff
reproduced KiCad's alternate Q1-G width witness on unchanged source
copper by swapping the order of its two nearby original segments in
scratch. Independent stored and refilled checks found zero opens and
parity findings, the same single J1 footprint-type error, and the
original 0.0790 mm width warning. Some installed-library runs produced a
different set of representatives. The handoff still requires an exact
source replay and declines a witness it cannot reproduce. See
`build/miolink-schematic-20261120/width-local-replays/`.

The 116-hold MioLink local capacitor edit put C33:2 just 1.795 mm from
U10:8 in XY on the opposite face. The routed supply path still ran
59.077 mm through an intervening connector and other pads. The verified
`shorten-kicad-pad-path --decoupling-shortcut` added four segments and
one through via. The measured planar span outside pad and via copper
fell to 0.640 mm, with no intermediate pads. Total track length increased
by 2.229 mm. Independent KiCad stored and refilled checks kept zero
opens, active errors, or parity findings, all 19 exclusions, and the
existing warnings. All 117 footprint forms and the project inputs match
the local edit. A request from C33:2 to a resistor on the same supply net
returned the original PCB. See
`build/miolink-local-power-path-20261120/production-native-audit/`.

### Two-face ground pours

The November 18 [pour policy comparison](agents.md#compare-explicit-copper-pour-policies)
ran each policy with the same mechanical brief and work rates. Both Spell
Tome policies held its switch pose and routed all 18 connections with no
KiCad error or parity finding. At 500 work seconds per policy, adding GND
on both faces cut physical track length from 522.87 to 268.58 mm, with zero
vias in either result. The comparison selected the shorter layout.

On the portable USB Keypad brief, both policies kept all 24 constraints
and KiCad reported zero opens or parity findings at 240 work seconds per
policy. Adding the pours reduced physical track length from 1444.99 to
1273.89 mm and increased vias from 41 to 80. It also created one dangling
track error, so the as-supplied policy was selected. The five raw
hole-clearance findings within J2 were inherited in both layouts. These
are selected results for two tested briefs, with the inherited-finding
policy applied, and do not change the schematic tier's default result.

The optional [dangling-pad repair](agents.md#remove-a-generated-dangling-track)
verified removal of that same error in a matched two-face Keypad replay.
It removed one 0.25 mm GND track in two native checks. The 80 vias, five
inherited hole-clearance errors, and zero opens and parity findings were
preserved. The ordinary Keypad control copied through byte-for-byte.

The schematic tier uses a separate USB Keypad brief with a hatched GND
zone. Its fully routed board had six new minimum-connection-width errors
at 0.127 mm. The opt-in [hatched-zone repair](agents.md#retain-copper-at-clipped-hatch-holes)
cleared all six after raising the minimum hole area fraction from 0.30
to 0.60, widening four generated repair tracks and adding one 0.11 mm
bridge. KiCad still reported zero opens and zero parity findings and
retained all 17 inherited footprint errors. The ordinary schematic tier
keeps the supplied hatch policy.

Saving KiCad's 288 generated teardrops removed 66 stored-board opens.
With the same footprint libraries packaged in cacheable `.pretty`
directories, a repeated fill took 3.79 seconds versus 16.23 seconds on
a cache miss and returned the same board bytes. The
[same-hatch repair](#usb-keypad-hatch-repair-and-saved-fill)
then cleared all six width errors in ordinary layout.

The later cold run with the USB Keypad hatch repair passed **7 of 12**
schematic cases and **11 of 12** under strict rule 4. All twelve used
binary `f4195076d7e3` with the same v7 tasks, work rates and per-board
budgets as the preceding integrated run. USB Keypad retained its authored
hatch and cleared its six new width errors in 458.13 of 500 modeled
seconds. The other eleven delivered PCBs were byte-identical to their
preceding results. The per-board comparison is in
`build/schematic-regression-20261120-unchanged-hatch-other-eleven/integrated-comparison.json`.

A fresh twelve-board run with the ClearBell pad escape and mechanical
import changes (binary `bda807e0cf5f`) reproduced all twelve delivered
PCBs byte for byte at the same budgets and work rates. The five remaining
schematic failures were TelemetryOnboard (16 footprint-filter mismatches),
OpenFC (five net conflicts), Slab left (one open and six net conflicts),
0xCB Static (two solder-mask bridge errors in its layout verdict), and
Urchin (six net conflicts). The matched report is in
`build/schematic-regression-20261120-combined-clearbell/integrated-comparison.json`.

OpenFC Lite tests a tighter six-layer outline with a back-side USB
connector fixed in place and a region for the MCU. The first run
estimated front utilization at 105.3% and stopped with 89 of 164 parts
without a legal pose. In a second brief, assigning six more named connectors to the
back gave a legal placement with all nine constraints satisfied. The
router connected 80 of 82, and KiCad reported two opens, ten DRC errors
and zero schematic parity findings.

A matched rerun with the project's original custom rules held all nine
constraints and again routed 80 of 82. Accounting for plated-drill
clearance in placement removed both cross-footprint errors. KiCad still
reported the four clearance errors inside U10 and four zero-width
annular rings on the USB and card connector footprints.

The November 20 panel-placement build (`acfc9bac62b4`) passed strict
routing on all 12 boards and full schematic acceptance on seven.
The v7 task definitions and per-board modeled budgets matched the
previous run. Eleven returned PCBs were byte-identical. SLAB left
connected 76 of 76 nets, up from 75, using 326 vias and 7,690.7 mm
of track. A separate cold rerun produced the same PCB bytes.
Independent KiCad checks of that board, both as saved and after refill,
reported zero opens, zero active rule errors, 70 warnings, and six
schematic net conflicts. The strict routing gain leaves its full
schematic acceptance open. See
`build/schematic-regression-20261120-sheet-courtyard/independent-comparison.json`
and `build/slab-schematic-native-audit-20261120/comparison.json`.

The schematic tier's `--import-near` imports with `import-kicad-netlist
--near-decoupling` (the decoupling capacitors' `near` entries in the new
`constraints.json`). Measured on the tier, it keeps the unconnected count
(2 against 2) but the placer misses most of the entries, so the tier's
passes fall from 10 to 2: the import's flag stays off by default.

**What a package runs.** A router package runs the fingerprints and the
expansion tier against main's numbers; a placer or layout package adds
the short tier and, for identity, the dense boards at seed 1. A placer
default goes through the protocol below. No package runs the layout
sweep or the designer tier: the nightly runs them on main, and a
package's claim about the sweep is checked by the first nightly after its
merge.

### How to run each

Benchmark a copied binary (`bash scripts/snapshot.sh <n>` builds and
copies it to `build/bin/pcb-maker-x<n>`), never `target/release`. Every
run goes through the memory ledger (below) and may run beside others.

```sh
B=build/bin/pcb-maker-x15210
R=benchmarks/agent-tasks/run.py
# fingerprints (compare the sorted fingerprint lines of each route.log and the routed boards)
PCB_ROUTER_FINGERPRINT=1 PCB_ROUTER_NO_WALL_GUARD=1 $R build/fp-x15210 \
  --tasks benchmarks/agent-tasks/tasks/v7/tasks.json --route-from build/github-layout-v16 \
  --budget-scale 0.25 --jobs 1 --threads 4 --binary $B \
  --only wntrblm__Castor_and_Pollux__mainboard transistorfet__computie__k30-SBC \
         antmicro__jetson-nano-baseboard__jetson-nano-baseboard briskspirit__Sisu_SSE-9__Sisu_SSE-9
$R build/short-x15210 --tier short --jobs 3 --threads 2 --binary $B > build/short-x15210.log
$R build/expansion-x15210 --tier expansion --jobs 8 --threads 1 --binary $B > build/expansion-x15210.log
$R build/quick-x15210 --tier quick --binary $B > build/quick-x15210.log
$R build/sch-x15210 --tier schematic --binary $B > build/sch-x15210.log
$R build/endgame-x15210 --tier endgame --jobs 3 --threads 2 --binary $B > build/endgame-x15210.log
# the layout sweep and the matched designer tier, as the nightly runs them
$R build/sweep-x15210 --tasks benchmarks/agent-tasks/tasks/v7/tasks.json --jobs 10 --threads 2 \
  --quality --order peak-desc --binary $B > build/sweep-x15210.log
$R build/designer-x15210 --tier designer --budget-scale 0.613 --quality --jobs 8 --threads 1 \
  --order peak-desc --binary $B > build/designer-x15210.log
# against a reference run
benchmarks/agent-tasks/compare.py build/short-main.log build/short-x15210.log
benchmarks/agent-tasks/compare.py --gap build/sweep-x15210.log build/designer-x15210.log --targets
# from a schematic; the KiCad demo corpus
benchmarks/from-schematic/run.py build/from-schematic --jobs 2
benchmarks/corpus/fetch.sh && python3 benchmarks/corpus/run.py build/corpus
```

Useful options of run.py: `--only NAME...` (a subset), `--route-from DIR`
(route the placements of an earlier layout run: router experiments
without the placement), `--splice-from LOG` (run only `--only`'s boards
and copy the other rows from a log, marked spliced), `--reuse DIR` (a
layout whose placement digest matches a stored one skips its routing),
`--placer '{"tuning": {"seed": N}}'` and `--layout '{...}'` (settings; the keys of a `tuning` group merge one by one with the task's), `--budget-scale`,
`--dry-run` (the start order and each board's memory declaration).

`compare.py` prints, over the boards in both runs, better / same / worse
by KiCad's unconnected count, the passes gained and lost, the summed
objectives and the medians against the designer's board. It marks rows
cut (CUT), killed (KILLED), spliced (SPL) and boards whose input changed
(INPUT: task, constraints, board or project files), which it leaves out
of better and worse.

## The pass rules

A row passes when every connection is routed and KiCad's check finds no
copper error beyond the designer's, and every constraint holds but those
the task lists as impossible. Silkscreen and library cosmetics never
count. Two rules decide "beyond the designer's":

- **The per-type rule** (`pass`): per finding type, KiCad's count on our
  board less the count on the designer's own routed board. Lenient: a
  finding of ours hides behind one of the designer's anywhere on the
  board.
- **The strict rule** (`pass_strict`, `new_findings_strict`, rule 4,
  `strict_rule` in every row; the headline counts this one): a finding of
  ours counts unless the designer's board with its tracks, vias and small
  zones stripped (`baseline-drc.json`) has it at the same place: the same
  type and the same items the same distances apart, or items at the same
  positions within 0.05 mm, each designer finding answering for one. A
  finding of one item is the designer's only where that item lies within
  0.05 mm of theirs. A finding between items of one footprint is always
  the designer's (its pads and graphics, which no placement or route
  changes). A solder mask bridge of a mask opening that is bridged on
  the designer's boards already is not counted (`strict_mask_openings`).
  `rejudge.py` judges a stored run again under the current rule.

A row without a result (no legal placement, an error, a timeout) fails
under both rules. A row the machine ended (a signal, or the memory ledger
twice) is `killed`: no result, and compare.py leaves it out of better and
worse. A row the wall clock cut is marked `cut`. The schematic tier also
needs every footprint found (`missing`).

## Measurement discipline

A layout is deterministic for its binary, task, rates and settings,
whatever its thread count, unless the wall clock cuts it (such rows carry
`cut`). One run per side answers a quality question; a repeat gives the
same rows.

- **Pinned work rates.** Budgets are in seconds of work, not wall time.
  run.py pins `PCB_WORK_RATE=3.15e6,150e6` (the reference) for every
  board, so rows compare across machines and loads; `--work-rate
  measured` lets each binary measure its machine instead, which ties the
  rows to the machine and its load. The wall-clock guards stretch by the
  pinned rate over the measured one (at most four times,
  `PCB_WALL_STRETCH_MAX`), so a loaded machine takes longer instead of
  cutting rows; a row is cut only below a quarter of the reference rate.
- **Tests pin the rates too.** Every test run of the fixtures, the
  walkthrough, the fingerprints and the hostile inputs sets
  `PCB_WORK_RATE=3.15e6,150e6` and `PCB_ROUTER_NO_WALL_GUARD=1` (and the
  fixtures `PCB_MAKER_MEMORY_MB`). A test that checks the measured rates
  asserts the rates only, nothing a rate could change. No wall-clock bound
  decides a test; a wall bound is only a hang guard far above any load.
- **Judge by continuous objectives from one run**, not by pass counts,
  which move by single-error flips: KiCad's unconnected items, new copper
  errors, vias, length, work seconds, the first route's open terminals,
  and the medians against the designer. Totals compare only over the same
  rows with a result.
- **One task list per comparison.** Pass the same `--tasks` on both sides
  or rely on the tier's default on both. compare.py says TASK LISTS DIFFER
  and marks changed boards INPUT.
- **Threads.** Tiers default to 2 search threads a board and physical
  cores / 2 boards at once (`TIER_THREADS`). Rows are comparable across
  thread counts as long as no row is cut; CPU and wall seconds are not.
- **Paired CPU runs only for CPU claims.** Both binaries at the same time
  on symmetric halves of the cores (`taskset -c 0-7,16-23` against
  `8-15,24-31`: logical CPUs n and n + 16 share a physical core, never
  give the two sides siblings), CPU seconds from rusage (the rows'
  `cpu_seconds`), never wall seconds. Reserve the memory for both sides
  at once and start both inside it (`PCB_MAKER_LEDGER_INSIDE`): a pair
  whose one side waited is void. KiCad's DRC reports are cached
  (`~/.cache/pcb-maker/drc`); the side that checks a board first pays and
  the other hits, so CPU pairs run with `PCB_MAKER_NO_CACHE=1` on both
  sides, or both warm. Quality comparisons are unaffected by the cache.
- **Rows say what made them** (`binary_sha256`, the task, board,
  constraints and project hashes, `strict_rule`, `spliced_from`, the rate
  fields). A spliced row is the old binary's: say so when quoting it.

## Measurement discipline: placer defaults

A placer change moves every board to another draw of its seed noise: on
the dense boards one board's unconnected count spreads by up to 193
(video) over placer seeds. The rule for a placer default, on the eleven dense boards in
layout mode at the task budgets (tasks v7, pinned rates, 3 x 2, through
the ledger):

1. Run **five seed pairs**, placer seeds 1, 101, 201, 301, 401 on both
   sides (`--placer '{"tuning": {"seed": N}, ...}'`). Seeds lie at least 16 apart: the
   race places seeds N to N + 15 (`config.tuning.seed + offset` in
   `board_placer/place/seeds.rs`), so nearer seeds share candidates and are not
   independent draws.
2. Judge with `placer_ab.py compare --off DIR... --on DIR...` (each
   directory's seed from its `-s<N>`, or `DIR@N`). The statistic is the
   **weighted relative change**: each board's mean change over its seeds
   divided by its noise mean, weighted by 1 / max(cv, 0.10)^2 from the
   noise file. Its one-sided p is the larger of a paired sign-flip
   randomisation test (this comparison's own spread) and the noise file's
   null.
3. **Better** when p <= 0.03. When 0.03 < p <= 0.25, run seeds 501, 601,
   701, 801 too and judge the nine pairs again at p <= 0.03. Otherwise not
   better. Worse when the smaller p is at least 0.97.
4. The other conditions stay: the short tier loses no strict pass and
   gains no DRC error. A strict pass the short tier loses at its seed is
   checked on that board alone at the five seed pairs; the condition
   holds when the on side passes on at least as many of the five as the
   off side.

Simulated from the noise file (`placer_ab.py protocol`), the rule calls a
lever with no effect better about 4 % of the time and finds a 5 % effect
about 55 % of the time, 8 % 90 %, 10 % 98 %. The weighted change answers
"does the typical board route better", not "does the tier's count fall":
it listens to the steady boards, and a lever that helps only video (the
noisiest) does not pass it, nor would it pass any statistic on this
noise. The summed medians cannot resolve a 3 to 5 % effect at any number
of seeds that is affordable.

The off side's rows can be reused while main's layouts of the boards stay
the same; when main changes a dense board's layouts, its off rows and its
rows in the noise file need running again.

```sh
P=benchmarks/agent-tasks/placer_ab.py
$P compare --off build/X-dense-s{1,101,201,301,401}-off --on build/X-dense-s{1,101,201,301,401}-on
$P compare ... --metric first_open|objective|crossings --tool build/bin/pcb-maker-xNNNN   # a proxy
$P resolution --k 3 5 9 --at 0.03 0.05 0.10     # the smallest effect each statistic resolves
$P protocol                                     # the rule's false positive rate and power
$P noise build/Y-dense-s{1,...,801}-off --out benchmarks/agent-tasks/dense-noise.json \
  --tool build/bin/pcb-maker-xNNNN [--boards NAME... --update]
$P report                                       # the noise table and the proxies' correlations
```

### The noise file

`benchmarks/agent-tasks/dense-noise.json` holds, per dense board and for
nine seeds (1, 101, ..., 801) of one binary, KiCad's unconnected count,
the first route's open terminals, and with `--tool` the placed board's
wirelength and ratsnest crossings (the placement objective is the
wirelength plus 3 mm per crossing). Each board's entry names its binary;
`--update` replaces the named boards' rows. The nightly's losers use it
too: a loss on a dense board counts only beyond the board's seed spread.

Within a board across the seeds, the first route's open terminals follow
the final count (pooled r about 0.7) with as much relative spread; the
objective has little noise but explains a fifth of the final count's
spread. A lever that lowers the objective is not yet a routing gain.

### The ladder of placer measurements

The full protocol costs 110 layouts a lever (about 88 CPU hours). A lever
climbs cheaper stages first and stops where it fails:

| Stage | What runs | Cost | Calibration |
| --- | --- | --- | --- |
| 0: placement | the placement alone; `ablate-kicad-placement` metrics (crossings, objective, wirelength, blocked exits, overflows) | about a quarter of a layout | a check that the lever moved what it meant to move, not a filter: weakly tied to the final count |
| 1: first route | the five seed pairs with `--layout '{"stop_after_first_route": true}'`; `placer_ab.py compare --metric first_open` | 0.55 to 0.57 of the full stage | agreed with the full verdict on both calibration experiments (crossings in the anneal, the race's ripped count) |
| 2: full | the protocol above | 110 layouts | the ground truth |

- **A lever passes stage 1** when the first route's weighted change is
  negative at the noise file's p <= 0.25; then stage 2 on the same seeds.
  A full layout cannot start from a stopped one, and `--reuse` does not
  cross the option (it is in the key).
- **What it saves** is half: the first route is most of a dense
  layout's routing.
- **Reuse** (`run.py --reuse build/layout-reuse`): run both sides on one
  binary with the lever as an option; boards the lever leaves alone then
  cost their placement only. The key holds the placement digest
  (`layout: placement digest` on stderr, `placement_digest` in
  board-layout.json), the binary, the task, the settings, the rates, the
  threads and the `PCB_*` environment; a switch that lands in the lowered
  placement problem, or two binaries, never hit.
- **Not stages**: a quarter budget changes the placement itself (two
  dense boards find no legal placement) and does not rank the full rows;
  condensed boards (a window around a crowded cluster) were assessed and
  rejected ([placer.md](placer.md)).

## Memory ledger

Benchmarks and agents share the machine through the memory ledger
(`$XDG_RUNTIME_DIR/pcb-maker-ledger.json`, `PCB_MAKER_LEDGER`; how a run
reserves and waits: [agents.md](agents.md)). Every route, layout and KiCad
check of a binary reserves its expected memory before it allocates, and
run.py admits each board by the same ledger. `pcb-maker ledger` lists
who holds and who waits; `--lock` still takes the exclusive tier lock
(`build/quick-tier.lock`) for a wall or CPU measurement that must not
share the machine. The rules are the same in
`crates/pcb-kicad-check/src/ledger.rs` and `benchmarks/harness_common.py`.

- **Capacity**: MemTotal less the memory outside the ledger (the median
  of the headroom samples of the last six hours when they are fresh: at
  least 360, the newest at most a day old; else 25 % of MemTotal), at
  most MemTotal less 6 GB (`PCB_MAKER_LEDGER_MARGIN_MB`).
  `PCB_MAKER_LEDGER_TOTAL_MB` replaces it.
- **The floor**: run.py keeps the memory available, less what the
  reservations still grow by, above a floor per board: 2 GB to keep
  (`FLOOR_KEEP_MB`), plus the 0.95 quantile of how far the memory outside
  the ledger rose within 10 minutes over the last six hours of samples,
  plus the tier's overshoot (measured peak less declared, from
  estimates.json). Without an hour of fresh samples it is 10 GB.
  `--min-free-gb` overrides it. A binary's new request keeps the same
  floor (without the overshoot); a short request keeps 2 GB. The samples
  live in `<ledger>.headroom.json`; run.py samples while it runs, and
  `python3 benchmarks/ledger_headroom.py [--sample --for S]` samples or
  prints the floor per kind.
- **Ordered admission**: a waiting request is in the ledger, and a newer
  request may not pass the one that has waited longest: it goes only if
  the oldest would still fit beside it and every reservation that began
  after the oldest began waiting. A request larger than the capacity runs
  alone; after `--ledger-drain-max` (7200 s) of waiting for the ledger to
  empty it ends with an error.
- **The drain**: once the first waiter has waited `--ledger-drain-after`
  (600 s), a new request goes only if it leaves the waiter its room, or
  ends no later than every run that holds memory. The ledger drains only
  for a waiter it could start once the runs since its wait began end, and
  whose entry is fresh (rewritten within 120 s); past 7200 s it drains
  strictly (no early-end exception). The drain is for the first waiter
  that qualifies; the others keep their place by ordered admission.
  While the waiter is blocked by a run larger than 4 GB, requests of at
  most 4 GB still go (the feasibility cap).
- **The nightly's priority**: `--ledger-priority nightly` puts the
  nightly's requests first among the waiters; they are never passed, and
  the ledger drains for them at once.
- **Short requests never wait behind a drain**: a command that holds its
  memory for seconds (`drc-kicad-board`, `place-kicad-board`,
  `inspect-routing-run --native-check`, a command arming on demand for
  kicad-cli; `PCB_MAKER_LEDGER_SHORT` overrides) with at most 2 GB of its
  own beside one KiCad check (2253 MB) goes whenever the memory is there.
- **kicad-cli runs under a reservation**: a check command reserves one
  check up front; other commands that run kicad-cli arm on demand; a
  route or layout raises its reservation by the check while it runs.
- **A board's reservation is its binary's**: run.py hands the board's
  reservation to its binary, whose own limit is then at least that
  (`granted_mb`); the board's other binaries run inside it
  (`PCB_MAKER_LEDGER_INSIDE`), and those after it inside a short
  reservation of 4301 MB.
- **A run past its reservation waits** for room instead of ending. It
  ends with exit 4 only when the memory available falls below 2 GB, or
  after 600 s when the memory available less the other runs' growth is
  below 2 GB. run.py runs a row the ledger ended once more
  (`ledger_retry`); ended twice, it is killed.

### Per-board estimates

run.py declares each board's expected peak from
`benchmarks/agent-tasks/estimates.json` (`--estimates` another;
`--expected-gb` overrides every board's: give it only from a measured
peak, never a round placeholder). Per board and kind (layout, short,
schematic, designer, expansion) it holds the largest `peak_rss_mb` of the
newest three sources that measured it, times 1.25, and their largest
seconds. A board without a row of its kind falls back to its layout rows,
the tier file's entry, a size model over pads times copper layers, then
3 GB. `tier_estimates.py --refresh --layout <log>` (`--designer`,
`--short`, `--schematic`, `--expansion`, `--night build/nightly/<date>`)
writes it; commit the file after a refresh. The nightly keeps its own
copy, `build/nightly/estimates.json`, refreshed from each night's rows,
so its estimates follow the binary.

## Nightly

`scripts/nightly.sh` measures main every night at 02:00 (a systemd user
timer; `--install` sets it up, `--quick` leaves out the sweep and the
designer tier, `--no-commit-report` publishes nothing). It builds main's
head in its own checkout (`../pcb-maker-nightly`), snapshots the binary
as `build/bin/pcb-maker-nightly-<date>-<sha12>`, and runs, through the
memory ledger with `--ledger-priority nightly`: the fingerprints, the
expansion tier, the short tier, the matched designer tier and the layout
sweep. It writes `build/nightly/<date>/`: each stage's log and output,
`summary.md` (per stage and board), `nightly.json`, `gap.txt`,
`diff-<stage>.txt` against the previous night that ran the stage, and
`causes.json`: KiCad's unconnected items per cause the run's own outputs
name, also summary.md's "Open causes" (benchmarks/agent-tasks/README.md,
"Nightly").

**Publishing.** `scripts/nightly-report.py --commit` (the last step)
writes the night into the main checkout and commits it on main as
"nightly <date>: layout sweep vN, <strict> strict (<change> against the
previous night), matched designer tier <strict> strict, gap placement
<p> / routing <r>", then pushes. It writes

- this file: the night's row of the table below, and a section "Layout
  sweep vN (<date>)" after the newest numbered sweep; only the newest two
  such sections stay (the next night compares with the previous one);
- README.md: the headline's measured part and the two newest table rows;
- docs/handover.md: the title's state, the nightly state line and the
  night note.

**The guard.** A night is published only when every stage ran, no row of
any stage was cut or killed, and the sweep has at least 100 rows with a
result; otherwise it is only summarised and `nightly.log` says why. The
report commits only when the main checkout is on main, has no change to
a tracked file, and `git pull --ff-only origin main` succeeds; otherwise
the next night publishes every guarded night after the newest one this
file has (at most three). By hand: `scripts/nightly-report.py --docs
build/nightly/<night>` (`--dry-run` prints the section and writes
nothing; `--checkout` writes another checkout's docs).

The table: one row a night, newest first, the last 14 nights. Cells:
passes / rows (strict passes), KiCad's unconnected items, rows cut and
killed; the designer tier's budget scale; the gap split over the target
boards (`compare.py --gap --targets`); against the previous night,
better / same / worse per stage and the passes gained and lost.
What keeps the matched designer tier's failing boards from the strict
pass is analysed in [router.md](router.md).

<!-- nightly:begin -->
| Night | main | Fingerprints | Expansion | Short | Sweep | Designer | Gap (placement / routing) | Against the previous night (better/same/worse) |
| --- | --- | --- | --- | --- | --- | --- | --- | --- |
| 2026-11-26 | `cb7743eb2bc6` | changed: Sisu_SSE-9 | 27/46 (strict 28), 569 open | 9/12 (strict 10), 2 open | 72/109 (strict 74), 1209 open | 77/109 (strict 78), 2662 open at 0.524 | 61% / 39% | fingerprints 0/4/0; expansion 1/45/0; short 0/12/0; sweep 9/97/2; designer 4/102/0 (pass +1 -0) |
| 2026-11-24 | `96e891b27162` | changed: Sisu_SSE-9 | 27/46 (strict 28), 571 open | 8/12 (strict 9), 5 open | 71/109 (strict 73), 1836 open | 74/109 (strict 75), 2668 open at 0.595 | 62% / 38% | fingerprints 0/4/0; expansion 1/45/0; short 2/9/1 (pass +0 -1); sweep 12/86/7 (pass +3 -2); designer 3/103/3 (pass +0 -1) |
| 2026-11-18 | `3b27f2d0eb93` | identical | 26/46 (strict 26), 556 open | 9/12 (strict 9), 16 open | 70/109 (strict 71), 1451 open | 75/109 (strict 76), 2677 open at 0.595 | 61% / 39% | fingerprints 0/4/0; expansion 0/46/0; short 0/12/0; sweep 0/106/0; designer 0/109/0 |
| 2026-11-17 | `77eb39eb6e52` | identical | 26/46 (strict 26), 556 open | 9/12 (strict 9), 16 open | 70/109 (strict 71), 1451 open | 75/109 (strict 76), 2677 open at 0.595 | 61% / 39% | fingerprints 0/4/0; expansion 0/46/0; short 0/12/0; sweep 0/106/0; designer 0/109/0 |
| 2026-11-16 | `f8f6ec663a81` | identical | 26/46 (strict 26), 556 open | 9/12 (strict 9), 16 open | 70/109 (strict 71), 1451 open | 75/109 (strict 76), 2677 open at 0.595 | 61% / 39% | fingerprints 0/4/0; expansion 0/46/0; short 0/12/0; sweep 0/106/0; designer 0/109/0 |
| 2026-11-15 | `ae56024c8820` | identical | 26/46 (strict 26), 556 open | 9/12 (strict 9), 16 open | 70/109 (strict 71), 1451 open | 75/109 (strict 76), 2677 open at 0.595 | 61% / 39% | fingerprints 0/4/0; expansion 0/46/0; short 0/12/0; sweep 0/106/0; designer 0/109/0 |
| 2026-11-14 | `fed05b94abc9` | identical | 26/46 (strict 26), 556 open | 9/12 (strict 9), 16 open | 70/109 (strict 71), 1451 open | 75/109 (strict 76), 2677 open at 0.595 | 61% / 39% | fingerprints 0/4/0; expansion 0/46/0; short 0/12/0; sweep 0/106/0; designer 0/109/0 |
| 2026-11-13 | `86554c6e447f` | identical | 26/46 (strict 26), 556 open | 9/12 (strict 9), 16 open | 70/109 (strict 71), 1451 open | 75/109 (strict 76), 2677 open at 0.595 | 61% / 39% | fingerprints 0/4/0; expansion 0/46/0; short 0/12/0; sweep 0/106/0; designer 1/106/2 |
| 2026-11-12 | `bc69e68de947` | identical | 26/46 (strict 26), 556 open | 9/12 (strict 9), 16 open | 70/109 (strict 71), 1451 open | 75/109 (strict 76), 2671 open at 0.613 | 62% / 38% | fingerprints 0/4/0; expansion 0/46/0; short 0/12/0; sweep 3/103/0 (pass +1 -0); designer 1/108/0 |
| 2026-11-11 | `c0e40c30d391` | identical | 26/46 (strict 26), 556 open | 9/12 (strict 9), 16 open | 69/109 (strict 70), 1552 open | 75/109 (strict 76), 2672 open at 0.613 | 67% / 33% | fingerprints 0/4/0; expansion 0/46/0; short 0/12/0; sweep 0/106/0; designer 0/109/0 |
| 2026-11-10 | `8bfeb313afbf` | identical | 26/46 (strict 26), 556 open | 9/12 (strict 9), 16 open | 69/109 (strict 70), 1552 open | 75/109 (strict 76), 2672 open at 0.613 | 67% / 33% | fingerprints 0/4/0; expansion 0/46/0; short 0/12/0; sweep 0/106/0; designer 2/102/5 (pass +1 -1) |
| 2026-11-09 | `80c050719bf8` | changed: jetson-nano-baseboard | 26/46 (strict 26), 556 open | 9/12 (strict 9), 16 open | 69/109 (strict 70), 1552 open | 75/109 (strict 76), 2634 open at 0.676 | 68% / 32% | fingerprints 0/4/0; expansion 1/44/1 (pass +1 -0); short 0/12/0; sweep 6/99/1 (pass +2 -0); designer 5/103/1 (pass +2 -0) |
| 2026-11-08 | `b090e40d64d3` | changed: Sisu_SSE-9, computie__k30-SBC | 25/46 (strict 25), 557 open | 9/12 (strict 9), 16 open | 67/109 (strict 68), 1697 open | 73/109 (strict 74), 2685 open at 0.663 | 71% / 29% | fingerprints 1/3/0; expansion 1/44/1 (pass +1 -0); short 3/9/0 (pass +2 -0); sweep 22/72/12 (pass +4 -0); designer 14/91/4 (pass +4 -1) |
| 2026-11-07 | `406dcb94a0f6` | changed: jetson-nano-baseboard, Sisu_SSE-9, computie__k30-SBC, Castor_and_Pollux__mainboard | 24/46 (strict 24), 558 open | 7/12 (strict 7), 21 open | 63/109 (strict 63), 1957 open | 70/109 (strict 67), 2756 open at 0.662 | 70% / 30% | fingerprints 0/4/0; expansion 0/46/0; short 0/12/0; sweep 11/87/8 (pass +3 -3); designer 1/107/1 (pass +0 -1) |
<!-- nightly:end -->

## Four-layer recovery on the Capybully keyboard (2026-11-19)

The November 18 nightly placed and routed Capybully but left three native GND opens. A follow-up on its saved output closed all three by adding 18 tracks and two vias. All 2,218 original footprints, copper items, zones, board graphics and board setup forms compared equal. The search used a 0.075 mm pitch and expanded 140,787 nodes.

```sh
case_dir=build/nightly/2026-11-18/sweep/sporkus__capybully_keyboard__capybully
pcb-maker repair-kicad-opens "$case_dir/layout/result" capybully --local build/capy-repaired --net GND
pcb-maker drc-kicad-board build/capy-repaired capybully build/capy-drc.json --save-filled build/capy-filled
```

KiCad 10.0.8 reopened the filled board with zero errors and zero opens. The source and final independent checks each reported 94 warnings. This board-only replay checks connectivity against the input PCB's net assignments. The original nightly sweep results are recorded below.

Two fresh `layout-kicad-board` runs from the nightly source reproduced
the same placement and initial routing. The control finished with three
opens at 1,204.82 modeled seconds. Automatic local repair closed all three
at 1,249.59 modeled seconds and produced the exact PCB bytes of the
standalone repair above. A full run with verified fill publication enabled
finished at 1,271.16 modeled seconds. Its verdict records the accepted
three-open repair and 85 saved polygons across three zones, including the
hatched ground pour on both outer layers. The delivered PCB was byte-identical
to the separately published board that passed both stored and refilled KiCad
checks with zero opens and zero errors.

## Partial mechanical brief on CyberKeeb2040 (2026-11-20)

A stripped schematic-linked project kept 74 switch and connector poses
and prescribed the mounting faces of all 166 parts. The remaining 92
were movable. At the default 0.635 mm grid and 600 modeled seconds for
layout, the earlier placer used 257.48 modeled seconds in its selected
seed. Five diodes had no placement and their tentative positions
conflicted with three more movable parts. Routing could not start.

With the same project and settings, the bounded sparse pass completed
all three placements. It seated 10–12 parts per seed in 2.68–3.91
modeled seconds within the placement allocation. The normal layout ran
in 338.14 modeled seconds and routed 118/118 connections, with 10,361.74
mm of saved copper and 96 vias. The explicit 2.54 mm control also routed
118/118, in 355.99 modeled seconds with 10,937.22 mm of copper and 71
vias. Independent KiCad 10.0.8 stored and refilled checks on both found
zero opens or parity findings and the three inherited courtyard errors.
All 92 unlocked parts moved in each arm. The default grid result kept
the outline, 74 held poses, and all mounting faces. Its
`sparse_legalization` report records 11 parts seated and 3.91 modeled
seconds of search on the selected seed.

The fully held CyberKeeb control kept all 166 poses and routed 118/118
with the same inherited findings in both checks. A 1 mm × 1 mm authored
region for one 9.72 mm × 2.50 mm diode was rejected. On the BumWings
holdout, 64 movable diodes with the controller and switches held yielded
a byte-identical PCB before and after this change. Both layouts completed
81/81 connections in 49.36 modeled seconds with zero native errors,
opens, and parity findings.

The CyberKeeb inputs, logs and independent native comparison are in
`build/cyberkeeb-partial-mechanical-20261120/`. The BumWings comparison
is in `build/bumwings-diode-placement-20261120/`.

## Layout sweep v46 (2026-11-24)

<!-- nightly-sweep night=2026-11-24 sweep=build/nightly/2026-11-24/sweep.log main=96e891b2716257cb110546c713145c9c4e914bcb -->

Written by the nightly (`scripts/nightly-report.py --docs`) from
`build/nightly/2026-11-24/`: main 96e891b (57 commits after v45's main
3b27f2d; 57 commits after the nightly of 2026-11-18's main 3b27f2d),
binary `pcb-maker-nightly-2026-11-24-96e891b27162` (sha256
fd23b893b009), tasks v7 (sha256 c8f95caf5089), `--quality`, at the
pinned work rates (the measured router rate had a median of 1.6e6, work
over wall 0.61), 10 jobs x 2 threads, `--order peak-desc`, through the
memory ledger with `--ledger-priority nightly`: 16459 s wall, largest
peak 3.2 GB, no row cut or killed in any stage. Per board:
`build/nightly/2026-11-24/summary.md`, `diff-sweep.txt` and `gap.txt`.

**Inputs.** Every board in both ran on the same input as in v45 (nightly
2026-11-18).

| | v45 (nightly 2026-11-18) | v46 (nightly 2026-11-24, 96e891b) |
|---|---|---|
| pass, per-type rule | 70 of 109 | **71 of 109** |
| pass, strict rule (rule 5) | 71 of 109 | **73 of 109** |
| unconnected (all rows) | 1451 | 1836 |
| unconnected where both have a result on the same input (that run / this) | 1451 / **1356** (105 boards) | - |
| new DRC errors (per-type rule) | 27 | 37 |
| new DRC errors (strict rule) | 15 | 18 |
| vias (rows with a result) | 29113 (106) | 29737 (107) |
| length (rows with a result) | 752 m (106) | 750 m (107) |
| work s | 71596 (106) | 73544 (107) |
| CPU s (unpaired, a shared machine) | 84250 | 123001 |
| first-route open terminals | 1926 (106) | 1909 (107) |
| rows cut by the wall clock / killed | 0 / 0 | **0 / 0** |
| rows without a result | 3 | 2 |
| our vias in small SMD pads (score-kicad-board) | 0 on all 106 | 0 on all 107 |
| vias against the designer's (median) | x0.65 (106) | x0.64 (107) |
| length against the designer's (median) | x0.98 (106) | x0.98 (107) |
| open over connections (median, mean) | 0.000, 0.055 | 0.000, 0.061 |
| bends per connection against the designer's (median) | x0.99 | x1.00 |
| stair-steps, ours less the designer's (median) | +31 | +28 |
| pin-row crossings, ours less the designer's (median) | +0.5 | +0 |
| ground pour cut, ours less the designer's (median) | -56.6 mm (99; 1 lost) | -88.1 mm (100; 1 lost) |
| final ladder's share of the budget (median) | 0.595 (34 rows) | **0.524** (27 rows) |

**Against the nightly of 2026-11-18 (v45)** (compare.py, 109 boards in
both on the same input): better 12, same 86, worse 7; 19 of the 105 with
a result in both are identical in routed count, unconnected, vias,
length and strict findings; unconnected there 1451 -> 1356. Better, the
largest: poly_kb_molecule_4x5 304 -> 218, RP2040_base 11 -> 5, Batch_1
142 -> 137, WLED_Matrix 5 -> 0, reCamera_S101_v1.1 4 -> 0, capybully 3
-> 0, MegaDrive 57 -> 54, USB_Keypad 2 -> 0, MIDAS-MK1.1-revA 3 -> 1,
SUMEC_MK_IV 7 -> 5 and 2 more. Worse: SNSP-CPU-01 2 -> 9, vaio_re 1 ->
5, laptop power 9 -> 13, 0xCB-1337 pcb-panel 0 -> 3, 4in1-mini 88 -> 91,
sbc_allwinner_a13 module 5 -> 8, poly_kybd_split72_left 1 -> 3. Strict
passes 71 -> 73: gained 4: WLED_Matrix (5 open -> none),
reCamera_S101_v1.1 (4 open -> none), osprey_rev_a (2 open -> none),
capybully (3 open -> none); lost 2: 0xCB-1337 pcb-panel (3 open),
CyberKeeb2040 MainBoard (usage or input error: fixed pattern "locked"
matches no part).

**Rows without a result, 2** (the nightly of 2026-11-18: 3):
CyberKeeb2040 MainBoard (usage or input error: fixed pattern "locked"
matches no part; the previous night none), OpenESC-30x30 4in1-panel (the
panel's 2 boards' parts are mixed: 392 movable parts belong to no
sub-board; the previous night the panel's 2 boards' parts are mixed: 392
movable parts belong to no sub-board).

**Strict errors, 18 on 5 boards**: Sisu_SSE-9
(diff_pair_gap_out_of_range 4, diff_pair_uncoupled_length_too_long 3),
link (length_out_of_range 6), USB_Keypad (connection_width 3),
eurorack-pmod-pcb (solder_mask_bridge 1), sbc_allwinner_a13 module
(items_not_allowed 1). Failing the strict rule with nothing open, 1:
USB_Keypad (connection_width 3). Of the 36 that fail, 15 are within five
connections and five errors of clean, 12 leave more than 20 open, 2 have
no result.

### Placement or routing on one binary (matched designer tier, nightly 2026-11-24)

`build/nightly/2026-11-24/designer.log`: 109 boards routed by the same
binary on the designer's placement at `--budget-scale 0.595` (the median
final-ladder share of the previous night's sweep, `nightly-report.py
--ladder-share`; 892.5 s of work), `--quality`, 8 jobs x 1 thread,
`--order peak-desc`: 6587 s wall, largest peak 2.8 GB, no row cut or
killed, 109 with a result. **74 of 109 pass the per-type rule, 75 the
strict rule**; 2668 unconnected in all; strict errors 8; medians against
the designer's own board: vias x0.47, length x1.00, bends x0.82,
stair-steps +0.

`compare.py --gap build/nightly/2026-11-24/sweep.log
build/nightly/2026-11-24/designer.log --targets` over the **99 target
boards with both results** (of 100): **layout open 598, designer-route
open 229: placement gap +369 (62 %), routing gap 229 (38 %)**; ours is
worse on 19, the same on 70, better on 10. MegaDrive dominates the
routing side (the designer route 70 open, ours 54); without it 544
against 159: placement 71 %, routing 29 %. Over all 107 boards with both
results (the panels included): 1836 against 2634, placement -43 %,
routing 143 %. The previous night's split (2026-11-18, 100 target
boards): 61 % / 39 %.

**The five boards whose placement costs most** (ours / designer route, open connections):

| Board | Connections | Layout open | Designer route open | Placement gap |
| --- | --- | --- | --- | --- |
| Batch_1 | 254 | 137 | 2 | +135 |
| glasgow revC3 | 226 | 85 | 17 | +68 |
| OpenESC-30x30 4in1 | 152 | 78 | 12 | +66 |
| 4in1-mini | 152 | 91 | 37 | +54 |
| SmartSpin2k_Panelized | 585 | 51 | 0 | +51 |

Next: eurorack-pmod-pcb +17 (18 / 1), laptop power +12 (13 / 1),
sbc_allwinner_a13 module +5 (8 / 3), link +4 (5 / 1),
poly_corne_split42_right +4 (5 / 1). Routing first (the designer route
leaves as much or more): MegaDrive 70 / ours 54, SNSP-CPU-01 40 / ours
9, katia 14 / ours 3, RP2040_base 7 / ours 5,
PCB-Modular-Multi-Protocol-Hub Hub 3 / ours 2, WLED_Matrix 2 / ours 0
and 5 more.

**Worse than the nightly of 2026-11-18, beyond noise** (the next triage;
a layout is deterministic for its binary, so every change is the
binary's; on the dense boards of `dense-noise.json` a loss counts only
when it exceeds the spread of the board's unconnected count over the
placer seeds there), 4: vaio_re 1 -> 5, laptop power 9 -> 13, 0xCB-1337
pcb-panel 0 -> 3, sbc_allwinner_a13 module 5 -> 8. Losing the strict
pass: 0xCB-1337 pcb-panel (3 open), CyberKeeb2040 MainBoard (usage or
input error: fixed pattern "locked" matches no part). Within the seed
spread: SNSP-CPU-01 2 -> 9 (spread 68), 4in1-mini 88 -> 91 (spread 29),
poly_kybd_split72_left 1 -> 3 (spread 17).

## Layout sweep v47 (2026-11-26)

<!-- nightly-sweep night=2026-11-26 sweep=build/nightly/2026-11-26/sweep.log main=cb7743eb2bc6f2c1e9b7e635ff5830545982065d -->

Written by the nightly (`scripts/nightly-report.py --docs`) from
`build/nightly/2026-11-26/`: main cb7743e (109 commits after v46's main
96e891b; 88 commits after the nightly of 2026-11-25's main cdf003c),
binary `pcb-maker-nightly-2026-11-26-cb7743eb2bc6` (sha256
488906c34e81), tasks v7 (sha256 c8f95caf5089), `--quality`, at the
pinned work rates (the measured router rate had a median of 2.7e6, work
over wall 1.09), 10 jobs x 2 threads, `--order peak-desc`, through the
memory ledger with `--ledger-priority nightly`: 6214 s wall, largest
peak 3.3 GB, no row cut or killed in any stage. Per board:
`build/nightly/2026-11-26/summary.md`, `diff-sweep.txt` and `gap.txt`.

**Inputs.** Every board in both ran on the same input as in nightly
2026-11-25; 1 board ran on other inputs than in v46 (nightly 2026-11-24)
(INPUT: project files, task, board or constraints changed; left out of
that comparison): CyberKeeb2040 MainBoard.

| | v46 (nightly 2026-11-24) | nightly 2026-11-25 (cdf003c) | v47 (nightly 2026-11-26, cb7743e) |
|---|---|---|---|
| pass, per-type rule | 71 of 109 | 72 of 109 | **72 of 109** |
| pass, strict rule (rule 5) | 73 of 109 | 74 of 109 | **74 of 109** |
| strict passes on the 108 boards of the same input as v46 (nightly 2026-11-24) | 73 | - | 73 |
| unconnected (all rows) | 1836 | 1834 | 1209 |
| unconnected where both have a result on the same input (that run / this) | 1836 / **1209** (107 boards) | 1834 / **1209** (108 boards) | - |
| new DRC errors (per-type rule) | 37 | 33 | 54 |
| new DRC errors (strict rule) | 18 | 14 | 35 |
| vias (rows with a result) | 29737 (107) | 30076 (108) | 30181 (108) |
| length (rows with a result) | 750 m (107) | 762 m (108) | 763 m (108) |
| work s | 73544 (107) | 73883 (108) | 73120 (108) |
| CPU s (unpaired, a shared machine) | 123001 | 95684 | 76839 |
| first-route open terminals | 1909 (107) | 1928 (108) | 1928 (108) |
| rows cut by the wall clock / killed | 0 / 0 | 0 / 0 | **0 / 0** |
| rows without a result | 2 | 1 | 1 |
| our vias in small SMD pads (score-kicad-board) | 0 on all 107 | 0 on all 108 | 0 on all 108 |
| vias against the designer's (median) | x0.64 (107) | x0.64 (108) | x0.64 (108) |
| length against the designer's (median) | x0.98 (107) | x0.96 (108) | x0.96 (108) |
| open over connections (median, mean) | 0.000, 0.061 | 0.000, 0.060 | 0.000, 0.046 |
| bends per connection against the designer's (median) | x1.00 | x0.99 | x0.99 |
| stair-steps, ours less the designer's (median) | +28 | +30.5 | +29 |
| pin-row crossings, ours less the designer's (median) | +0 | +0 | +0 |
| ground pour cut, ours less the designer's (median) | -88.1 mm (100; 1 lost) | -60.2 mm (101; 1 lost) | -116.1 mm (101; 1 lost) |
| final ladder's share of the budget (median) | 0.524 (27 rows) | 0.524 (27 rows) | **0.528** (26 rows) |

**Against the nightly of 2026-11-25** (compare.py, 109 boards in both on
the same input): better 9, same 97, worse 2; 95 of the 108 with a result
in both are identical in routed count, unconnected, vias, length and
strict findings; unconnected there 1834 -> 1209. Better, the largest:
poly_kb_molecule_7x5_wave_left 256 -> 28, poly_kb_molecule_5x4_wave 224
-> 14, poly_kb_molecule_4x4 259 -> 153, poly_kb_molecule_5x2_shifted 201
-> 158, poly_kb_molecule_4x5 218 -> 180, glasgow revC3 85 -> 81,
OpenRX-panel-rev2 75 -> 71, Sisu_SSE-9 4 -> 3, SNSP-CPU-01 9 -> 8.
Worse: 4in1-mini 91 -> 98, OpenESC-30x30 4in1 78 -> 81. Strict passes 74
-> 74: gained 0; lost 0.

**Against v46** (compare.py, 108 boards in both on the same input, 1
INPUT left out): better 12, same 92, worse 3; 86 of the 107 with a
result in both are identical in routed count, unconnected, vias, length
and strict findings; unconnected there 1836 -> 1209. Better, the
largest: poly_kb_molecule_7x5_wave_left 256 -> 28,
poly_kb_molecule_5x4_wave 224 -> 14, poly_kb_molecule_4x4 259 -> 153,
poly_kb_molecule_5x2_shifted 201 -> 158, poly_kb_molecule_4x5 218 ->
180, glasgow revC3 85 -> 81, OpenRX-panel-rev2 75 -> 71, laptop power 13
-> 9, sbc_allwinner_a13 module 8 -> 5, SUMEC_MK_IV 5 -> 4 and 2 more.
Worse: 4in1-mini 91 -> 98, RP2040_base 5 -> 11, OpenESC-30x30 4in1 78 -> 81. Strict passes 73 -> 73: gained 0; lost 0.

**Rows without a result, 1** (the nightly of 2026-11-25: 1):
OpenESC-30x30 4in1-panel (the panel's 2 boards' parts are mixed: 392
movable parts belong to no sub-board; the previous night the panel's 2
boards' parts are mixed: 392 movable parts belong to no sub-board).

**Strict errors, 35 on 6 boards**: poly_kb_molecule_5x4_wave
(starved_thermal 21), link (length_out_of_range 6), USB_Keypad
(connection_width 3), sbc_allwinner_a13 module (items_not_allowed 2),
Sisu_SSE-9 (diff_pair_uncoupled_length_too_long 2), eurorack-pmod-pcb
(solder_mask_bridge 1). Failing the strict rule with nothing open, 1:
USB_Keypad (connection_width 3). Of the 35 that fail, 16 are within five
connections and five errors of clean, 11 leave more than 20 open, 1 have
no result.

### Placement or routing on one binary (matched designer tier, nightly 2026-11-26)

`build/nightly/2026-11-26/designer.log`: 109 boards routed by the same
binary on the designer's placement at `--budget-scale 0.524` (the median
final-ladder share of the previous night's sweep, `nightly-report.py
--ladder-share`; 786 s of work), `--quality`, 8 jobs x 1 thread,
`--order peak-desc`: 4618 s wall, largest peak 2.8 GB, no row cut or
killed, 109 with a result. **77 of 109 pass the per-type rule, 78 the
strict rule**; 2662 unconnected in all; strict errors 8; medians against
the designer's own board: vias x0.48, length x0.99, bends x0.83,
stair-steps +2.

`compare.py --gap build/nightly/2026-11-26/sweep.log
build/nightly/2026-11-26/designer.log --targets` over the **100 target
boards with both results** (of 100): **layout open 601, designer-route
open 235: placement gap +366 (61 %), routing gap 235 (39 %)**; ours is
worse on 20, the same on 73, better on 7. MegaDrive dominates the
routing side (the designer route 75 open, ours 54); without it 547
against 160: placement 71 %, routing 29 %. Over all 108 boards with both
results (the panels included): 1209 against 2625, placement -117 %,
routing 217 %. The previous night's split (2026-11-25, 100 target
boards): 60 % / 40 %.

**The five boards whose placement costs most** (ours / designer route, open connections):

| Board | Connections | Layout open | Designer route open | Placement gap |
| --- | --- | --- | --- | --- |
| Batch_1 | 254 | 137 | 2 | +135 |
| OpenESC-30x30 4in1 | 152 | 81 | 15 | +66 |
| glasgow revC3 | 226 | 81 | 18 | +63 |
| 4in1-mini | 152 | 98 | 37 | +61 |
| SmartSpin2k_Panelized | 585 | 51 | 0 | +51 |

Next: eurorack-pmod-pcb +17 (18 / 1), laptop power +9 (9 / 0),
RP2040_base +5 (11 / 6), link +4 (5 / 1), 0xCB-1337 pcb-panel +3 (3 /
0). Routing first (the designer route leaves as much or more): MegaDrive
75 / ours 54, SNSP-CPU-01 39 / ours 8, katia 14 / ours 3,
PCB-Modular-Multi-Protocol-Hub Hub 3 / ours 2, WLED_Matrix 2 / ours 0,
jetson-nano-baseboard 2 / ours 1 and 2 more.

**Worse than the nightly of 2026-11-25, beyond noise** (the next triage;
a layout is deterministic for its binary, so every change is the
binary's; on the dense boards of `dense-noise.json` a loss counts only
when it exceeds the spread of the board's unconnected count over the
placer seeds there), 0: none. Within the seed spread: 4in1-mini 91 -> 98
(spread 29), OpenESC-30x30 4in1 78 -> 81 (spread 49).

## Standalone route probe budget on TAPRX-888 (2026-11-20)

All four arms started from the same stripped placement and used one seed,
two router threads, a 90 s modeled routing budget and the same pinned work
rate. KiCad independently refilled each saved board before measuring opens
and errors.

| Pour selection | Modeled work (s) | Observed wall time (s) | Native opens | Native errors |
| --- | ---: | ---: | ---: | ---: |
| Previous automatic ladder | 281.722 | 320.98 | 139 | 6 |
| Fixed `pours: connect` | 107.523 | 108.52 | 131 | 5 |
| Fixed `pours: tracks` | 101.279 | 85.16 | 133 | 7 |
| Automatic ladder with measured probe admission | 106.683 | 112.06 | 119 | 5 |

Wall times come from each route verdict on this host. Machine load varied
across the runs. The pinned work rate supplies the comparable budget.

The exclusive-plane probe spent 44.46 s of search on its first pass, against
an 11.25 s per-probe target and a 45 s probe search share. The ladder
continued that router and skipped the three remaining probes. Work fell by
62% against the previous automatic ladder. At less than one modeled second
from the fixed `connect` control, the automatic result had 12 fewer native
opens and the same number of errors. The new result is incomplete and
finishing work still took it past the 94.5 s bound.

CAN485-dongle, planewatcher and the two-layer rectifier control had
byte-identical saved boards before and after the change. Their independently
refilled native results were respectively 0 opens and 0 errors, 0 opens
and 3 errors, and 0 opens and 0 errors. The planewatcher run kept all four
probes, including the tracks probe that completed the route. The underlying
logs and native reports are in
`build/retrocast-clearance-20261120/pair-launch-study/` under `qa-tapr-*`
and `qa-probes-*-final`.

Two further fixed-placement holdouts used their archived 30-second route
configs and one thread per arm. The LoRa sensor node produced an identical
PCB at 31.21 modeled seconds, with eight opens and zero native errors in
both runs. On the Caribou control board, modeled work fell from 62.59 to
40.04 seconds and refilled opens fell from 53 to two. Raw DRC errors rose
from nine to ten. Both boards had eight edge-clearance errors. The earlier
board had one starved thermal and the newer board had two at U303 pad 13.
Both remaining opens involved U201. The native reports and verdicts are
under `qa-probe-extra-*` in the same directory.

The archived timing configs defer native verification until the independent
rescore. A follow-up Caribou run enabled KiCad during routing at the same
30-second budget. The normal solid-connection repair cleared both thermal
errors. Independent refill found two opens and eight edge-clearance errors.
The stripped input also has eight edge-clearance errors. The route charged
40.04 modeled seconds, including zero modeled seconds for its native checks
under standalone route accounting.

## Placement seeds from repeated schematic sheets (2026-11-20)

A separate cold-start trial matched source-symbol UUIDs across sheet
instances, checked local pad connectivity, and used held connector
positions to transform the first placement into a seed for the other
instance. It screened out transfers that disagreed with mechanical
constraints. On Katia, translating one USB connector to the other put
only two of 16 movable origins inside the receiving piece. The fixed
LED positions also disagreed with a single rigid transform.

| Board and placement seed | KiCad opens | Vias | Track length (mm) |
| --- | ---: | ---: | ---: |
| Eurorack, ordinary alternate | 13 | 178 | 3,180.4 |
| Eurorack, start from the same parts' previous poses | 12 | 137 | 3,141.9 |
| Eurorack, transfer between repeated instances | 11 | 150 | 3,199.7 |
| Glasgow, ordinary alternate | 65 | 842 | 7,167.8 |
| Glasgow, transfer between repeated instances | 73 | 709 | 7,822.9 |

Saved and refilled counts agreed in every row. Routing took 152–154
modeled seconds on Eurorack and 242 on Glasgow. Both Glasgow placements
had the same requested budget, but the transfer used 17.4 seconds
more placement work. The transfer improved one board's open count and
worsened the other's. The ordinary placer uses its existing seeds.
The survey, transformations and native readbacks are in
`build/repeated-sheet-sample-20261120/`.

## Glasgow placement beside an authored rule area (2026-11-20)

On the 17-open Glasgow placement, the F.Cu rule area enclosed both C90
pads and barred tracks and vias. The 0.50 mm placement spacing ruled
out moving C90 alone. The scoped pad-exit repair kept the original spacing
and moved C90 and TP11 1.27 mm toward the opening. Both stayed on F.Cu
at their original angles. The primary placement kept all other poses,
14 mechanical holds, 1,142 pads and the authored outline and rule area.
The search took 0.067 modeled seconds for the selected placement.

On the previously routed board, the same two moves temporarily opened
TP11's `/SCL`. The ordinary local open repair added ten copper items to
reconnect `/SCL` and C90's GND and VBUS, preserving all 5,572 existing
tracks and vias. The saved board has 15 opens in both stored and refilled
native KiCad DRC, zero schematic parity findings and the same four errors
and 245 warnings by finding and item as the 17-open incumbent. This was
a separate retained-copper trial.

On the independent ICEpi Zero holdout, the 180 s cold A/B wrote
byte-identical boards with 79 opens, two errors and zero parity findings.

The 1,500-second cold trial enabled this search on all kept placements
before the routing race. It improved the first route from 68 to 20
unconnected terminals and changed the race winner. After the final ladder,
refilled native opens increased from 17 to 19 and vias from 547 to 592.
Both arms had the same four inherited errors and zero parity findings.
The top-level layout verdict charged 1,382.514 and 1,452.054 modeled
seconds respectively. The early placement change was removed.

The routed-board recovery reproduced the two-pose move from the original
17-open board using `repair-kicad-opens --rule-area --pad C90:2`. It added
eleven copper items and saved the filled board with 15 stored and refilled
opens, the same four errors and 245 warnings by identity, and zero parity
findings. It preserved all 5,572 existing tracks and vias, the 14 held
footprints, the outline, rule area and project inputs. Measured work for
placement and local search was 0.055 and 0.015 modeled seconds. The full
command took 66.3 wall seconds for nine primary KiCad checks and four native
connectivity scans. The connectivity scans verified that a changed `/CLKIF`
open witness represented the same electrical components.

The ICEpi routed-board holdout named an open pad without an applicable
keepout. The Glasgow control named an already-connected TP11 pad. Both
returned the original PCB byte for byte without trying a copper change.
The cold recipes, implemented CLI outputs, pose and copper audits, and
independent KiCad reports are in `build/cold-keepout-placement-20261120/`.

### Glasgow FPGA escape with retained copper

Glasgow revC3 had 15 native opens on the retained-copper result. The U30
BGA has 0.35 mm pads on 0.8 mm pitch. The two best landing sites for
`/IO_Banks/QB2` and `/IO_Banks/QB4` were only 0.18 mm apart. A 0.025 mm
jog in an In1.Cu segment made room for both 0.4/0.2 mm vias at 0.716 mm
spacing. Those launches and four additional ordinary local searches left
all 15 opens.

For `/IO_Banks/U5`, adding a legal via at each terminal still left the
BGA exit trapped among retained vias. Routing a 0.15 mm trace between two
0.4 mm vias at 0.1499 mm clearance takes about 0.85 mm center spacing.
Ten single-via placements and four two-via placements were tried on copies.
The finer 0.025 mm search found a path past two relocated vias if one
In1.Cu `/SCL` segment made room. Its parallel neighbor was 0.30 mm away.
The accepted trial moved the two adjacent segments by 0.175 mm within
their original endpoints and placed a new U5 transition clear of that jog.

The public `repair-kicad-opens --relocate` command accepted the staged
edit. It saved the board with 14 opens in both stored and refilled KiCad
10.0.8 DRC. All four native errors and 246 warnings matched by finding
identity, with zero schematic parity findings. Independent connectivity
checks showed exactly one merge of the original U5 components. The repair
kept all 257 poses and 5,575 of 5,582 original tracks and vias exactly.
Seven original geometries changed and 95 copper items were added, including
four U5 vias and 20.749 mm of U5 track. The CLI completed in 37.469 wall
seconds. A one-via relocation with both U5 launches left 15 opens and
returned the original PCB byte for byte. A locked-via input was rejected.
The route was constructed in an exploratory copy. The command verifies and
publishes supplied copper edits.

The candidate, native checks, independent audits and rejected trials are
in `build/glasgow-bga-local-escapes-20261120/`.

### Rear assembly permissions on Eurorack PMOD

The six-layer cold brief kept 13 interface poses and used the supplied
outline. The donor's eight identical jack models now keep their common
`AudioJacksFixed:pj324m` ID, including each instance's hidden field
positions and values. The reimport and placement had zero schematic
parity findings, compared with eight with the former model comparison.

With the same schematic, outline, stackup and brief, the all-front
layout emitted 96 named rear-mount candidates and left 30 native opens.
The exact-name trial moved those 96 footprints to the rear and completed
all 133 connections with zero native opens and schematic parity issues.
KiCad found the same inherited footprint-type error in both arms. The
13 held poses and outline agreed on readback. Modeled work fell from
557.93 to 266.79 seconds. A normal Urchin control with zero native opens
omitted the rear-mount advice. Results are in
`build/eurorack-interface-layout-20261120/`.

### Large native-open group on Eurorack PMOD

On the six-layer Eurorack PMOD saved layout, `/-VCOM` had 11 open
components and no routed copper. The targeted local command reached its
eight-component limit without starting a search. The command now tries
short branches of the native ratsnest when an explicit `--net` exceeds
that limit. Each call searches at most four branches and has 20 seconds
for routing the named net. The first three accepted calls closed the
RN5 pad pair, the RN6 pad pair and the connection from RN7 to C1.
They required 415, 1,465 and 153,986 route expansions. With eight
components left, the existing full-group search closed three further
opens. A fifth call kept the previous PCB byte for byte.

The verified filled result reduced total opens from 31 to 25 and
`/-VCOM` opens from ten to four. It added 53 copper items on that net,
retained all 3,076 original copper objects and preserved all 151 footprint
poses. Stored and refilled checks matched the original one error and
117 warnings by identity, with zero schematic parity findings.
The targeted Glasgow `/A0` holdout kept the input PCB byte for byte
and repeated its previous 895-expansion search.
Results are in `build/eurorack-automatic-open-20261120/`.

With the same 600-second single-side brief, ordinary automatic layout
kept its existing GND repair and then closed the RN5 branch. The two
fresh layouts used the same placement and core route. Total modeled
work was 546.956 seconds for the 31-open control and 557.933 seconds
for the 30-open trial. The added branch used 415 route expansions and
three extra native checks. It added six front copper segments and left
all 3,076 prior copper items and 151 poses intact. Stored and refilled
DRC had the same one error and 117 warnings by identity, with zero
schematic parity findings. A retained-board run with only 20 modeled
seconds for repair kept the GND improvement and skipped the branch.
The independent 0xCB-Static layouts both used 83.118 modeled seconds and
saved byte-identical boards with zero opens and zero parity findings.
The matched runs and native reports are in
`build/eurorack-layout-branches-20261120/`.

## Track attachment in Caribou local repair (2026-11-20)

### Caribou copper distribution

On the cold-placement diagnostic with launches at two reported pins,
KiCad found zero opens and zero schematic parity findings. The original
designer's board filled 33 zones on 25 nets. The diagnostic filled two
zones, on GND and +3V3. For 24 nets with identical sets of schematic
terminals, the reference contained filled zone copper and the diagnostic
contained none. The numbers below use native KiCad track lengths and
refilled polygon areas summed over the copper layers.

| Net | Terminals on both boards | Designer filled area (mm² across layers) | Designer track (mm) | Diagnostic track (mm) | Diagnostic track width (mm) |
| --- | ---: | ---: | ---: | ---: | ---: |
| H_PWR-IN- | 114 | 36,709 | 5.2 | 907.9 | 0.15 |
| H_PWR-OUT- | 19 | 22,892 | 22.3 | 493.3 | 0.15 |
| H_PWR-IN--PREFUSE | 7 | 20,817 | 0.0 | 45.7 | 0.15 |

All three designer pours span six layers. The project assigns these nets
the default 0.15 mm track width. A cold mechanical import carries the
outline and keepouts. Supply distribution requires its own design input.
These measurements expose the difference in copper distribution. Current
capacity, voltage drop and temperature rise remain unverified.

In a separate diagnostic, removing all 17 `H_PWR-IN--PREFUSE` track
segments from the connected candidate created six native opens. A second
copy received the same removal and one rectangular zone on In2.Cu.
Its rectangle enclosed the current pad centres with a 2 mm margin. KiCad
filled 1,778.7 mm² and connected all seven terminals through the zone.
Refill and independent reopen both found zero opens, zero parity findings
and the same eight footprint-edge errors. The diagnostic used the new
placement and the original project rules. Its zone geometry was supplied
in the test. The cold router did not generate it.

A negative control crossed the same rectangle with a 7 mm wide,
higher-priority GND pour on In2.Cu. Refill left one open between two
pieces of the prefuse zone. KiCad reported the same zone UUID at both
ends and one isolated-copper warning on the GND pour.
Extending the prefuse rectangle onto In3.Cu as well closed that open
with the competing GND corridor still present. The additional fill
introduced no new DRC finding. The isolated GND warning remained.

### Native repair results

On a routed Caribou board with its original schematic and libraries,
the first local repair of `Net-(U201-1Y)` closed the last open with
seven additions. KiCad rejected the trial with two new connection-width
warnings. The new 0.15 mm tracks connected through the caps of the kept
tracks. Their tips were 0.100 and 0.112 mm from the respective
centerlines. KiCad measured contact widths of 0.1121
and 0.1009 mm against a 0.127 mm minimum.

The exact centerline retry closed the last open with nine additions and
preserved all 2,473 existing tracks and vias and all 319 footprint poses.
On the prepared two-open board, one ordinary `repair-kicad-opens --local`
invocation closed both opens with ten additions, preserving all 2,472
existing tracks and vias. Both results reopened with zero opens in
independent stored and refilled KiCad DRC, zero schematic parity findings,
eight inherited edge-clearance errors and eight inherited warnings. The
repair removed the original dangling-track warnings and introduced no
new native finding.

On the Glasgow positive holdout, the three requested nets still closed
with ten additions and without a contact retry. Stored and refilled
checks both reported 15 opens, the same four errors and 245 warnings,
and zero parity findings. The ICEpi Zero negative holdout returned the
79-open input byte for byte when no legal local route was available.

The ordinary 240-second cold Caribou route still has two opens, at U201
pads 7 and 8. The local command reported blocked terminal access for both
pads before it reached any track joint, returning the board byte for
byte. Recipes, repair reports, copper and pose audits, and native stored
and refill reports are in `build/caribou-track-contacts-20261120/`.

The six-layer [Caribou schematic replay](../benchmarks/from-schematic/README.md#caribou-six-layer-controller)
also checks the physical brief independently. The two-open cold result
preserved all 317 schematic footprint geometries, 26 outline primitives,
four rule areas, both footprint cutouts, the stackup and nine held poses.
KiCad 10.0.8 measured four 0.3749 mm and four 0.4978 mm edge clearances
inside HC201 and HC202 against the project's 0.5000 mm minimum. The
designer's pinned routed board reproduced the eight findings and had zero
opens, zero schematic parity findings and five additional starved thermals
after refill. Both boards were checked with the original rules and
`--all-track-errors`. The returned-board benchmark rejects the cold result.

On that selected layout, the as-written PCB had 138 opens and the native
refill had two. Copying the filled polygons alone left stale flash-layer
records on eight plated pads, adding clearance findings where the +3V3
fill passed those pads. The layout now also copies KiCad's derived
`zone_layer_connections` for matching plated pads and vias that remove
unused rings. Publication still requires the reopened board to reproduce
the native refill findings.

The matched Caribou handoff changed exactly eight pad layer records and
added four polygons across two zones. An audit matched every other board
form and the project, rule and schematic files. A normal layout replay
from the imported schematic with the same 240-second budget selected the
same placement and published the identical fill at 176.66 modeled
seconds. Independent stored and refill checks both reported two opens,
the same eight footprint edge-clearance findings, zero other clearance
findings and zero schematic parity findings. Artifacts are in
`build/caribou-layer-connections-20261120/`.

The two-layer GigaShield schematic holdout routed 155/155 connections
and published 66 polygons across two zones. Its independent stored and
refill checks both found zero opens, DRC errors and parity findings, and
the same 53 warnings.

The standalone cold-placement route had moved 1,030 items from the
project's implicit `User.1` and `User.3` layers to `Cmts.User`.
This changed the library art and gave KiCad 199 reported
`lib_footprint_mismatch` warnings, at its cap. The router now preserves
the canonical `User.1` through `User.45` layers that KiCad 10 can read
without table entries. A two-layer rectifier holdout wrote byte-identical
complete boards before and after the change.

An eight-launch cold-placement diagnostic repeated the 90-second route
with temporary outward traces on the U201 pad rows. With the original
footprint artwork preserved, local repair reached both opens and closed
them with thirteen additions and two exact joint retries. Independent
stored and refilled verification found zero opens, zero parity findings,
eight inherited edge-clearance errors and five dangling-track warnings
on temporary launches. Removing all five warned tracks reopened a net.
Adding only the two blocked pads' temporary launches after the normal
240-second cold route allowed both searches to run. Native checks
rejected the results for a narrow joint on one net and four clearance
errors on the other.

In the matched standalone reroutes from the genuine cold placement,
the unmodified, eight-launch and two-launch arms took 94.788, 94.766 and
94.921 modeled seconds respectively. Each initial reroute had two native
opens. After local repair, the unmodified arm still had two opens. The
eight-launch arm closed both with thirteen added pieces and five dangling
warnings. The two launches selected from the ordinary layout's two
blocked-pad diagnostics closed both with nine added pieces. The selected
two-launch board reopened with zero opens, no schematic parity findings,
no warnings, and the eight inherited edge-clearance errors. All three
matched the 317 original footprint expressions and kept all 1,030 items
on implicit user layers.

The public layout retry was verified at a declared 430 seconds with
190 seconds reserved. The native-locked input and its explicit-constraint
representation used the same physical pose for all 317 footprints.
Both ordinary 430-second controls spent 231.718 modeled seconds and
saved two opens. Both retry arms spent 312.545 modeled seconds in all,
including 135.890 for the trial. Each ran ten comparison and local checks
and one reroute check. Full elapsed times were 559.6 and 553.9 seconds
under shared load. Independent KiCad checks of the saved board and a
fresh refill found zero opens, zero parity findings, no warnings, and the
same eight footprint edge-clearance errors. The retry preserved every
selected pose and kept the project, schematic and rules files byte for byte.

The closed two-layer rectifier control skipped the trial with zero retry
work and zero retry checks. The layout recipes, native evidence, budget
tests and pose audits are under `build/caribou-track-contacts-20261120/`.

A 620-second arm reserved the same 190 seconds after the ordinary
430-second Caribou placement. This placement put U201 at
(101.60, 196.85) mm and R201 at (99.695, 192.405) mm, both at −90°.
KiCad accepted both provisional launches. Local repair closed the +3V3
open, while the signal between U201 pads 5 and 7 had no local path.
The layout kept the original two-open board, preserving all 317 poses
and all 3,919 tracks and vias. Independent verification of saved and
refilled copper found the same eight inherited errors and two
silkscreen warnings, with zero parity findings. The run charged
353.166 modeled seconds, including 121.449 for the retry, and took
641.0 wall seconds under shared load. The retry remains opt-in.

On a copy of that provisional one-open candidate, the signal's pad 5
sat in a 0.855 mm channel between two 0.15 mm tracks. A through via
requires a 0.900 mm channel at the project's 0.45 mm diameter and
0.15 mm clearance. The local trial moved the short generated GND
return 0.120 mm south with 45° joints and kept its original pad and via
contacts. KiCad accepted the same ground connection with no new
findings. Ordinary targeted local repair then closed the signal with two
0.45/0.25 mm vias and seven tracks on F.Cu and In2.Cu. Independent stored
and refilled checks found zero opens and parity findings, with the same
eight inherited errors and two silkscreen warnings. This experiment is
saved separately as `holdout-detour-filled/`.

The bounded layout implementation reproduced the result on the moved
placement in 386.214 modeled seconds under the same 620-second total.
It spent 154.496 modeled seconds on recovery, with 15 comparison,
connectivity and local checks plus one reroute check. Whole-layout
elapsed time was 594.9 seconds under shared load. It changed one
generated blocking track, kept its endpoints and added two track
sections to make the jog. Native component membership and every finding
identity matched before and after that change. The signal then closed
with two vias and seven tracks. The native stored and refilled reports
showed zero opens, zero parity findings, and the same eight inherited
edge errors and two silkscreen warnings. The primary placement with no
detour needed and the unrelated closed rectifier control both produced
byte-identical saved boards before and after this change.

### Reference comparison and bounded pours

The explicit reference comparison found 24 same-terminal nets with all
zones absent on the cold Caribou result. For `H_PWR-IN-`, the original
had 36,709.411 mm² of stored fill across six layers and the cold result
had 929.884 mm of 0.15 mm track across three layers. On CM5 Minima the
same check found `M2_3V3`, `/PCIe-M2/M2_LX` and `+3V3_PI`. An independent
KiCad inspection reproduced the net sets, terminal counts and layers
on both boards and matched the measurements within 0.0005 mm or mm².
Comparing a board with itself returned no lost pours.

A separate Caribou control removed the 17 tracks on `H_PWR-IN--PREFUSE`
and had six native opens on that net. With the same route input, one
`add_pours` region on In2.Cu enclosed its seven named terminals with a
2 mm margin. The generated box covered all 118 physical pad centres,
including the repeated pad number on HC202. All 3,724 retained tracks
and vias matched by identity and geometry. The zone alone closed all
six opens in native stored and refill checks, with 1,779.364 mm² filled
and the same eight inherited edge-clearance findings. Adding a competing
GND zone to the control split the bounded zone. Native checks reported
one remaining open with 1,519.822 mm² of target fill.

A July schematic replay used a `+12V` region on In2.Cu around C1:1 and
C3:1. The four saved vertices matched the final pad bounds plus 2 mm.
The layout routed 22/22 connections with zero native errors, opens and
parity findings. The new zone filled 649.242 mm². The ordinary replay
used 1,254.43 mm of track, and the bounded replay used 1,208.75 mm.
Caribou commands and native readbacks are in
`build/caribou-bounded-pours-20261120/`. The July replay is in
`build/retrocast-clearance-20261120/pair-launch-study/qa-july-regional-layout/`.

Separate July requests at C1:1 and C3:1 produced two In2.Cu zones with
distinct UUIDs. Both had copper after refill and the saved board had
zero native errors, opens and parity findings. A second invocation
restored one of those zones onto the verified placement, leaving the
other zone's UUID intact and passing native checks again. The scorer's
layer total includes 241.8 mm of other nets' track on In2.Cu. Clipping
straight track centrelines to the two outlines gives 0.000 and 0.996 mm
inside the C1 and C3 regions, respectively.

On the combined Caribou layout, the corresponding layer total was
1,096.5 mm on In2.Cu. None of that straight track centreline lay inside
the `H_PWR-IN--PREFUSE` region. The scorer matched the independent
KiCad clipping measurements on both boards.

A second July run requested two separate 4 × 4 mm zones on that layer,
one around C1:1 and one around C3:1. An earlier output reused a UUID for
both zones. Its saved and refilled DRC had zero errors and zero opens,
but the independent scorer rejected the duplicate identity. The fixed
output had distinct UUIDs and exact bounds at both anchors. KiCad
measured 12.068 and 10.958 mm² of filled copper respectively and the
returned board passed the original brief.

### Time to score stored copper

Caribou and July were scored both ways against their reference projects.
Each result is the median of two runs with alternating mode order and two
search threads. Shared quality fields and reference findings were identical.
All project file hashes stayed the same.

| Saved board | Default wall seconds | Stored wall seconds | Default peak MiB | Stored peak MiB | Native stored area, mm² |
| --- | ---: | ---: | ---: | ---: | ---: |
| Caribou | 19.33 | 0.338 | 2,482 | 120 | 70,796.816 |
| July | 1.45 | 0.021 | 227 | 22 | 10,983.635 |

KiCad's read of the saved polygons agreed per net and layer within
0.000001 mm². A July copy with no zone definitions reported zero rows.
A copy with the definitions and no fill polygons reported two rows,
both with zero polygons and zero area.

On the pinned CM5 Minima designer PCB, `score-kicad-board --stored`
read the saved fills in 0.18–0.22 wall seconds across three warm runs
with 48–49 MiB peak RSS. The default score, which also models pours on
the routing grid, took 2.11–2.47 seconds and 193–194 MiB. All 14 shared
quality fields matched. An independent KiCad read of the same PCB
matched the stored area and polygon count on each of 13 net/layer
combinations, totaling 31 polygons and 13,866.704 mm². These timings
cover scoring an existing board. They exclude placement, routing and DRC.

### Following placement and width advice on Little Red Rover

The cold, two-layer [Rover replay](../benchmarks/from-schematic/README.md#little-red-rover)
provides a control with 14 held poses, 51 movable parts and GND on both
faces. Supplying the designer PCB with `--reference` produced a
byte-identical routed board. The verdict identified 12 nets whose
length-weighted median trace width was at most half the designer's.

The verdict's suggested 1 mm classes for +3.3V and +5V routed all
64 connections. KiCad still found four connector hole-clearance errors
and zero opens. The physical traces on +3.3V measured 273.513 mm at
1 mm wide and 67.772 mm at 0.127 mm wide. On +5V the lengths were
47.758 and 28.346 mm respectively. All narrow segment midpoints were
within 1.36 mm of a same-net pad's bounding box. A class width sets
the preferred width and the router can narrow tracks near pads. These
measurements describe geometry rather than electrical capacity.

On the separate battery feed from J6:1 to Q2:3, a 1 mm preferred
class produced 22.686 mm at 1 mm wide and 2.317 mm at 0.127 mm.
Adding an explicit KiCad `track_width` minimum of 1 mm on `/bat_+`
produced 25.004 mm entirely at 1 mm. All other nets kept the same
track and via geometry. The stored and refilled boards had zero opens
and the same four inherited connector errors.

Turning on `placer.decoupling_near` alone increased the share of
capacitors within 3 mm of a supply pin from 20% to 40%. The capacitor
median fell from 4.35 to 3.76 mm. Both boards had zero opens. Trace
length rose from 5,071.15 to 5,499.37 mm and vias rose from 192 to 280.
Separate placement-only trials tested the advice about C3. Constraining
it near U10:3 put its supply pad 63.368 mm from U6:8. The template
generated from the placed PCB picks U6:8 by proximity. Using that
template's C3 constraint placed those same pads 2.250 mm apart. The
template generated from the unplaced schematic input pairs C3 with
U3:5. These results establish that the suggested placements differ.
The designer's intended assignment for C3 remains unverified.

The revised verdict offers a named capacitor target when the schematic
assignment and placed-board template agree on the IC. The exact pin comes
from the placed-board template. All suggestions matched that template in
the cold Rover run (three pairs), an authored C3-to-U10 constraint control
(two pairs), and the USB Keypad holdout (12 pairs). C3 remained a case for
review on the ordinary Rover. The authored control held its C3 constraint.
All three runs produced byte-identical boards to the earlier guidance
controls. The two Rover boards had zero native opens and parity findings.
USB Keypad had zero opens and parity findings with its six existing
connection-width errors. See
`build/rover-cold-20261120/decoupling-guidance-v2-audit.json`.

A placement-only trial used the existing pad-to-pad `near` objective with
`minimize: true` for C3:1 to U6:8. It produced a 1.732 mm gap and kept all
14 supplied poses. This was one explicitly selected target.

The seed-101 follow-up for the C2 advice left one `/left_motor_-` open.
The matching names `/left_motor_-` and `/left_motor_+` had previously
blocked the bounded local search. The cold two-layer rerun recovered the
open in 79.883 modeled seconds. The repair itself used 7.680 seconds,
including two native checks, and added 12 tracks and three vias on the
open net. All 1,786 existing copper objects and all 65 poses matched.
Independent as-stored and refilled checks reported zero opens and
parity findings. Each matched its corresponding baseline's four hole
errors and all other findings. A requested coupled route and an authored
uncoupled-length bound on the companion each kept the open and searched
no geometry. A bound scoped to an unrelated net still allowed repair.
A companion that differed in its exact native name also kept the open.
See `build/rover-cold-20261120/independent-pair-repair-audit.json` and
`independent-pair-controls-final.log` in the same directory.

Following the Rover verdict's first suggestion, a 3 mm
limit from C2's body to U2 pin 10, reduced the measured gap from
21.232 to 2.830 mm. On that matched run with `decoupling_near` enabled
and no added GND pours, track length fell from 5,863.03 to 5,122.14 mm
and vias from 187 to 131. Both boards routed 64/64 with four connector
errors and zero opens or parity findings against the imported project.

The original Rover layout also provides evidence for a mechanical
constraint that the initial benchmark brief leaves open. On the
designer PCB, 77.448 mm² of the module U10's 78.540 mm² antenna keepout
lies inside the separate board-level copper keepout. The initial layout
preserved both keepouts but placed them entirely apart. Adding the
designer's U10 pose as one more hold restored the measured overlap.
With the same GND policy and 240-second budget, the layout routed 64/64.
Total track length fell from 5,071.15 to 4,405.00 mm and vias from 192
to 145. KiCad reported zero stored opens and the same four connector
errors. This measures placement and copper geometry. RF performance
was not measured.

A second brief kept U10's orientation and allowed it to move along
the right side of the board. The benchmark chose a body region in
absolute board coordinates, x = 192–213.5 mm and y = 59–141 mm,
with U10 at −90°. The selected module origin was (200.025, 92.710) mm.
Its antenna kept 74.690 mm² of overlap. This arm used 3,841.83 mm of
track and 149 vias, routed 64/64, and met every specified mechanical
constraint. KiCad found zero stored opens and the same four connector
errors under the import's default footprint clearance policy.

In a later replay the explicit-reference verdict identified both keepouts
and returned U10's prior pose as a placement trial. Applying that pose
from the verdict to a new run restored 77.448 mm² of overlap, with all
64 connections routed, 4,124.12 mm of track and 152 vias. The new verdict
reported no further overlap loss. KiCad found zero stored opens and the
same four connector errors under the default footprint clearance policy.

A matched pair of fresh runs preserved the original footprint clearances
and used the same GND pours. Without the U10 region and rotation, the
board used 4,992.25 mm of track and 201 vias, with zero antenna overlap.
With the region and rotation it used 4,221.04 mm and 142 vias, with
74.690 mm² of overlap. Both routed 64/64. Scoring the region layout against
the original schematic confirmed all 65 footprint geometries, all 58
outline primitives, the board keepout, the stackup, and every supplied
mechanical constraint. KiCad found zero opens and zero schematic parity
errors both as saved and after refill. The board failed the raw DRC
gate with 13 U9 pad-clearance errors and four connector hole-clearance
errors.

### USB Keypad hatch repair and saved fill

KiCad reported six minimum connection-width errors against the project's
0.127 mm rule. Four named the unsmoothed GND zone on F.Cu and B.Cu. The
verdict identified the zone and the authored 0.3 minimum hole-area
fraction. With explicit permission to test 0.6, the repair changed that
fraction, widened four supplementary segments, and added one 0.110 mm
segment. It saved the authored zone's fill and 288 native pad/via
teardrops. Both reopened and refilled checks had zero width errors,
opens, and parity errors. The same 12 hole-to-hole and five hole-clearance
errors remained. Both authored zone IDs and outlines, all 98 footprint
poses and 315 pad definitions, and the schematic and rule files were
preserved. A 0.3 request returned exit 1 with an identical copy of the
original PCB and separate stored and refilled counts.

The standalone `repair-kicad-necks --preserve-hatch` command kept the original 0.30 fraction,
0.20 mm hatch lines, 0.60 mm gaps and 45° angle. The four zone-only DRC
findings all named the zone origin. In a private copy, KiCad identified
the actual narrow sections when tiny reference tracks were placed at
candidate locations in the existing filled copper.

The delivered repair widened four earlier supplementary segments, added
two short GND segments and placed two bounded solid GND zones on F.Cu.
The two sites that needed zones were on isolated pieces of the original
fill. Using tracks there had introduced two opens and one dangling
track. The supplementary zones filled 0.0485 and 0.0553 mm². Refilling the
entire front GND changed which hatch islands were retained, adding
5.359 mm² of copper and removing 0.213 mm². The back copper was
identical. Independent stored and refilled checks removed exactly the
six width findings and added none at any severity. The 1,306 original
named pads and routing objects retained the same 71 connectivity groups
in both checks.

The next cold schematic-to-board run adopted the same repair in ordinary
layout. USB Keypad passed the v7 schematic gate with 39 of 39 connections
routed and all 21 constraints satisfied. The repair consumed 89.33 of
the remaining 117.54 modeled seconds. Total work was 458.13 seconds
of the declared 500, with 88 vias and 1,325 mm of track. KiCad reported
zero opens, zero parity issues and the same 17 inherited errors.

### Multiple schematic pin definitions on the RP2040 motor interface

U1 and U2 each describe two functions as pin 2 in the exported schematic
netlist. The old import assigned both physical thermistor pads to GND.
The revised import used each donor pad's `Thermistor_2` function and
assigned `/Bridge_TH_1` and `/Bridge_TH_2`, respectively. Native
schematic parity findings fell from two to zero. The audit checked
1,006 unambiguous numbered pad keys and recorded the two distinct
schematic candidates and actual physical pad for each ambiguous key.
The footprints' pad IDs, local positions, shapes, drills and sizes
matched. The assignments survived writing placement and routing.
The 90-second electrical diagnostic replay left 49 opens and used
older overlapping module poses. See
`build/rp2040-motor-interface-20261120/duplicate-pins-audit.json`.

### Mounted daughterboards on the RP2040 motor interface

The connector courtyards cover both rows of each driver module. The
module's own Edge.Cuts matches the chamfered outline on its footprint's
Eco2.User layer. Independent native polygon readback gave 0 mm²
intersection at the author's two held poses and 1,460.281 mm² on the
earlier overlapping layout. With the explicit module group, the
ordinary placement command reports the overlapping pair as requiring
22.70 mm of separation. The report accepts the authored separated pair.
Those placement-only checks used 16 modeled seconds and left unrelated
parts unseated.

The 27-hold cold layout has 36 mm tall modules and a narrow horizontal
span near the fixed connectors. The ordinary courtyard model left less
than 29 mm between its extreme legal origins at zero degrees. At the
placer's existing final copper legality level, the span grew to 37.8 mm.
Joint seat generation finds candidates near both ends of the board
in this level. A trial that placed the modules after the rest affected
47 footprints and left three unseated. The placement race can instead
arrange the board around the generated seats in one seed.

In the matched 600-second cold layout, the constructive seed placed all
321 footprints, kept the 27 specified holds and started routing after
339.877 modeled seconds of placement. U1 and U2 remained movable in
the saved board and sat at (138.0, 78.3) and (134.1, 114.3) mm. Native
outline intersection was 0 mm². The earlier constraint-only run stopped
before routing with a 24.57 mm module violation. The independent
0xCB-Static placement without a module group produced byte-identical
boards and the same 28.934405 modeled work seconds. See
`build/rp2040-motor-interface-20261120/module-seats-v4-layout/` and
`module-seat-inspection/` in the same directory.

The zero-gap layout ended at 596.718 modeled seconds with no missed
placement constraints. Native checks on the board as stored found
146 opens, 97 raw errors, 241 warnings and zero packaged schematic parity
findings. Saving the verified refill in a separate project gave
41 opens, 8 raw errors, the same 241 warnings and zero parity findings.
The refilled and reopened connection partitions agreed, and both
saved projects have zero module intersection and zero contour gap.
The same two poses held with a specified 0.5 mm minimum cause the
placer to report a 0.50 mm shortfall.

A separate 600-second brief set `min_gap_mm: 0.5` with the same 27
holds and the same cold PCB. The selected placement kept every hold
exactly and seated all 321 parts in 303.642 modeled seconds. KiCad's
placed-contour readback measured 0.600 mm with zero intersection.
The readback measured 1.300 mm in the original author's placement.
This comparison preserves the earlier zero-gap arm. See
`build/rp2040-motor-interface-20261120/module-gap-050-layout/` and
`module-seat-inspection/independent-050-placement.json` in the same
directory.

The 0.5 mm arm ended at 604.256 modeled seconds with no missed placement
constraints. As stored, KiCad measured 177 opens, 115 raw errors, 118
warnings and no packaged schematic parity findings. The separately saved and
verified refill has 42 opens, 9 raw errors, the same 118 warnings and
zero packaged parity findings. Both copies retain the 0.600 mm module gap.
The independent original-schematic check found the two driver footprint
ID mismatches in this earlier import. A fresh schematic import with the
shared driver ID and via cost 0.5 completed its fill handoff within the
layout budget. Against the original schematic that PCB had 27 opens,
eight raw errors and zero parity findings both as stored and after refill.

### RP2040 rail escapes beside a movable capacitor

On the original 0.5 mm arm, native connectivity found U15:48 and
U15:49 together on a two-pad +3V3 island without a via. The existing
`pad-exits-kicad-board` census found zero closed track exits. Screening
0.7/0.3 mm through vias and 0.2 mm track shapes against the bare
placement found 157 and 142 inward via sites within 3 mm of the
two pads. C41 blocked the outward sites. Moving only C41 by 1 mm
without turning it opened 44 outward sites per pad and passed the
placer's final KiCad legality level. A native fill confirmed that a
1 mm inward branch at the original placement and a 1 mm outward
branch after the move each connected both pads to four existing
plated rail pads.

Each fixed-placement route requested 200 modeled seconds with
`pours: connect`. The independently refilled and reopened boards
kept all 27 supplied holds, the 0.600 mm gap, the same eight hole
errors, 118 warnings, and zero packaged schematic parity findings.
Both seeded branches stayed on the native board with their copper intact.

| Change to bare placement | Router work (s) | Reopened opens | +3V3 opens | Vias |
| --- | ---: | ---: | ---: | ---: |
| Baseline | 210.786 | 43 | 4 | 317 |
| C41 move | 206.097 | 48 | 0 | 318 |
| Inward branch | 201.517 | 50 | 1 | 315 |
| C41 move and outward branch | 189.414 | 44 | 0 | 330 |

The C41-only arm joined the target through an inward path to a via
about 3.8 mm away. The outward branch carried the target through its
seeded via. A separate native partition of the lower-via benchmark
isolated seven MCU rail pads, including U15:48 and U15:49. Each had at
least 75 screened inward via sites on that bare board. These trials
supported keeping the ordinary placement behavior.
See `build/rp2040-motor-interface-20261120/module-gap-neighbor-escape/audit.json`.

### Clearance for rear test points on the RP2040 motor interface

The rear-access brief placed 62 test points and 15 jumpers on the back.
KiCad still reported `U19:7` open to `TP1:1` on `/GPIO5_2`. Their pad
centers were only 0.037 mm apart in XY. On the bare placement the
inspector's short-via screen found no path. A 0.791 mm same-face move
of the front SOT-23-6 footprint `U26` opened two sites while holding all
other 320 poses. The identical two traces and 0.70/0.30 mm via caused a
short, a solder-mask bridge, and two hole-clearance errors on the
original placement. On the moved placement they connected both pads
with zero additional native findings. `TP3:1` already had 166 sampled
sites, so the inspector offered no move for that open.

At the same 145-second routing request, the ordinary rear-access arm
left 36 refilled opens and nine raw errors. Applying the emitted patch
and rerouting closed the named pair and returned 34 saved and refilled
opens, eight inherited hole-clearance errors, and zero schematic parity
findings. The 0.50 mm mounted-module gap stayed exact. Routing used
155.15 modeled seconds, up from 154.52. Vias increased from 383 to 391
and track length from 6,132.18 to 6,338.93 mm. A separate `Keep` trial
with the three independently checked copper objects reached 33 saved
and refilled opens, eight inherited errors, 378 vias and 6,342.21 mm.
See `build/motor-pose-control-20261120/branch-access/`.

### Legacy KiCad instance paths in route reuse

A moved-capacitor replay on July also tested a KiCad pad-layer spelling
difference. Reordering the same layer names on 172 pads of the prior
board made the old router skip reuse and spend 22.46 modeled seconds.
Comparing layer membership let it keep 307 original segments on all 22
routable nets and replace the two tips at C3. The same run used 7.92
modeled seconds including native checks. Independent KiCad checks found
zero errors, opens or parity findings on both boards as saved and after
refill. The corresponding KiCad 8-to-10 ordering difference occurs on
745 pads of the archived RP2040 motor controller.

A July controller test replaced one U6 instance path with its legacy
timestamp spelling and removed the four `SCALE_RX` tracks from the
current board. pcbnew read the two path spellings as the same identity.
The old router skipped reuse and rerouted the net. The revised router
retained all four donor segments with no pending routes, no native opens,
no raw errors, and zero schematic parity findings. On a mixed nested
DMXion path the revised router reached the native comparison, which
refused the trial's new connection-width finding. STRF's `.net`-only
input still requires native schematic parity before retaining prior copper.
See `build/strf-commercial-workflow-20261120/legacy-path-fixture/`.

### Freerouting session attachment

On the 321-footprint RP2040 motor placement, a plain KiCad load and save
changed the stored copper layer sets of six footprints' non-plated pads.
The benchmark now transfers native session copper onto the original PCB
file and reopens it in KiCad before publication. The original copper, if
present, must survive session import unchanged. The transfer retains the
original footprint coordinates even if the session rounds origins to the
DSN grid. In a 120 s session on this board, KiCad rounded three origins by
at most 38 nm on the 100 nm grid.

The full adapter run with a 75 s Freerouting limit retained all 321
footprint poses and geometries, the original project files, and the
0.600 mm module gap. It returned 463 segments and 45 vias. Independent
native refill found 499 opens, nine raw errors, and zero packaged
schematic parity findings. In a matched benchmark on the older KiCad 7
E-Fidget-Lite project, both routers reached zero native opens and added
no counted DRC errors. The external adapter retained all 29 original
footprint poses and geometries and returned 229 segments and 10 vias.
See `build/rp2040-motor-interface-20261120/freerouting-gap-half/native-adapter-audit.json`
and `build/rp2040-motor-interface-20261120/freerouting-gap-half/small-matched/results.json`.

### Native findings at an open WLED pad

On the archived WLED Matrix result, `inspect-routing-run --net
'Net-(RGB53-DO)'` links the open RGB53:1 pad to KiCad's short against an
AE1 front-copper polygon. It also links the other endpoint, RGB54:4, to
three clearance findings against AE1 copper. A fresh KiCad 10.0.8
refill reproduced the same RGB53:1 pad and polygon UUIDs in the short.
On the separate schematic-based layout, the AE1:1 pad shorts to copper
inside its own footprint, so the report directs attention to the source
land pattern and pad net. A closed WLED net and the open motor QSPI
connection with no matching pad finding both omit this field.
See `build/wled-open-pad-diagnosis-20261120/audit.json`.

In the schematic-based WLED project, the native artwork probe found the
same three front-copper graphics on both paths from AE1:1
`Net-(AE1-A)` to the two physical AE1:2 `GND` pads. The pad and graphic
UUIDs agree with a separate KiCad shape-intersection audit. The public
inspection omits a path when the graphics are disconnected, when their
pads share a net, or when the footprint declares a net tie. Osprey has a
similar geometric path on B.Cu. Its native check reports no open pad
there, so the open-pad inspection leaves that path unreported. See
`build/wled-matrix-schematic-20261120/antenna-contacts/public-verification.json`
and `build/footprint-conductor-survey-20261120/osprey-native.json`.

Osprey's separate cold layout has an open AE1:1 feed. Its native short
names the GND sibling AE1:2 and the polygon. A fresh public inspection
proved the B.Cu polygon touches both the open feed and the plated pad's
copper ring. Moving the feed off the polygon in a control left the open
and native GND short present and removed the verified path. The original
donor still has no open to attribute. See
`build/osprey-schematic-workflow-20261120/artwork-verification.json`.

### WLED Matrix donor footprint IDs and instance fields

The previous import's routed WLED board had 129 footprint ID mismatches
against the original schematic. A comparison limited to the display angle
of supported Fab reference text recovered all 64 RGB footprint IDs and
left 58 mismatches. The metadata comparison then recovered the other IDs
using the original schematic's per-part fields. The placed board and a
reimport from the emitted project both had zero schematic parity findings
against the original schematic. The two new placed arms had the same 140
footprint poses and 20 raw KiCad errors each.

The audit matched 415 field values on 138 schematic parts to both the
original schematic export and the donor PCB. All 140 imported footprint
forms agreed with the previous importer after omitting their footprint
IDs, including styles, pad geometry, and text caches. The 64 held RGB
footprints kept their native poses and field presentation from the author
board. The shared 0402 library model has its own Reference and Value
defaults, and the board instances carry their individual sourcing fields.
Two actual donor variants that widened one RGB2 or C1 pad kept separate
models. A donor-only C1 part number also kept the 0402 model split.
See `build/wled-donor-text-fidelity-20261120/audit.json`.

An independent full WLED layout kept all 77 held poses and had zero
original-schematic parity findings in the saved and refilled checks.
Both checks counted seven opens. The saved board had 138 raw errors and
the refill had 136. On USB Cable Tester the metadata comparison reduced
original-schematic footprint ID mismatches from 37 to 20, alongside one
missing footprint in both imports. See
`build/wled-matrix-schematic-20261120/independent-import/combined/verified-layout.json`
and `build/footprint-model-survey-metadata-20261120/native-cable-tester.json`.

### Importing paired public schematics and boards

Across 79 archived designs with both a schematic and a matching board,
76 imports resolved every requested footprint. Three reported missing
models: one on USB Cable Tester MK2, eight on the Huaqiu video board and
two on the Bumwings keyboard. All 79 produced import receipts. The previous
run aborted on HackRF One because a project library table referenced an
unavailable legacy library. The recovered HackRF import placed 418
footprints in the project, of which 271 received donor-derived library IDs
that differ from the original schematic.

The untouched HackRF One archive and explicit donor restored 34 readable
models under `hackrf` for 124 footprint instances. KiCad 10.0.8 reported
zero parity findings against the emitted schematics on the 418-footprint
import and its reimport after moving the project. The importer packaged
all six `gsg-modules` library models byte-for-byte, preserving the six
LEDs' model IDs and the other 412 IDs on reimport with or without the
board as a donor. The original tables and sheets stayed byte-identical.
The same-source retry preserved every staged model and the output table,
including a separately added local entry. An independently readable
external `.mod` control retained the original legacy nickname, entry and
file bytes and still imported all 418 footprints using separate donor IDs.
See `build/legacy-library-donor-20261120/portable-v2/new-verification.json`
and `build/legacy-library-donor-20261120/portable-v2/new-external-verification.json`.
The independent SLAB Left import copied all ten shared footprint models
byte-for-byte. Moving that project and reimporting without a donor kept
all 126 footprint IDs and the same six native `net_conflict` findings.
See `build/legacy-library-donor-20261120/portable-v2/slab-holdout/result.json`.

The 271 original-schematic ID differences are 258 `GSG-0402`, eleven
`GSG-0805` and two `GSG-XTAL3.2x2.5mm` instances. The audited pad geometry
agrees within each family. The donor models differ in other fields,
including legacy `Description`, `Part Number` and `Sheetfile` fields on
the 0402s. See `build/legacy-library-donor-20261120/portable-v2/new-model-variants.json`.

On both OpenMuscle FlexGrid boards, all 60 sensor pads now share their
original library ID. Independent KiCad checks of each unrouted import
found zero schematic parity issues. Reimport preserved the original UUIDs
and all 240 nested group membership links on each board. Across the 78
designs with import receipts in both survey runs, the list of references
requiring a separate footprint model changed only on these two designs.
See `build/footprint-model-survey-group-and-legacy-20261120/summary.json`
and `build/footprint-model-survey-20261120/group-fix/`.

### Comparing alternate routes on an unchanged placement

On the 144-footprint WLED Matrix, a 145-second modeled routing budget
with one initial seed, `first_seed: 5` and `via_cost: 0.8` used 118.122
modeled seconds for routing and 14.4 for the four native checks. The
alternate reduced seven opens to four in both the saved and refilled
comparisons and introduced no native DRC or schematic findings. It
closed `/Neopixel Bus`, `/Noise` and `Net-(U3-XTAL_P)`, reduced GND
opens from three to two, and opened one `/Sea` connection. The incumbent
PCB kept its original SHA-256. See
`build/fixed-route-comparison-20261120/wled/verdict.json`.

On the fixed 321-footprint RP2040 Motor Interface, the alternate reduced
42 saved opens and 40 refilled opens to 16 in both checks, with eight
newly open nets and no new native findings. An alternate on Eurorack
PMOD kept zero refilled opens but had 75 saved opens and 51 new saved
`via_dangling` warnings. Both comparisons retained the original boards.
See `build/fixed-route-comparison-20261120/motor.json` and
`build/fixed-route-comparison-20261120/eurorack.json`.

A matched WLED follow-up with a verified fill had 666 saved pad/via
teardrops. The comparator reproduced all 666 on a private native refill
with the same four checks. The follow-up reduced four opens to three in
both checks and introduced no new findings. Editing one stored teardrop
point made the native reproduction fail. Giving a teardrop-shaped zone a
persistent UUID made it fail the fixed-design comparison before native
work. See
`build/fixed-route-strategy-holdouts-20261120/collective-final-control/teardrop-verification.json`.

### Saving a completed standalone route's zone copper

The ordinary Eurorack PMOD route reproduced the exact alternate PCB that
had 75 saved opens and zero refilled opens. With two verification checks
the same command published two zones and three fill polygons. Independent
reads reported zero opens and zero schematic parity findings both saved
and refilled, with the same one raw error. The final PCB changed only
where KiCad stores fill. Routing used 63.399 modeled seconds and the
handoff used 7.2 modeled seconds in a 145-second budget. See
`build/direct-route-saved-fill-20261120/verification.json`.

A no-zone route produced a byte-identical PCB with the same single native
check before and after the change. A previously routed RetroCastX PCB
also stayed byte-identical. Its kept-input verdict still reports 307
stored opens and the existing command to save a filled copy. See
`build/direct-route-saved-fill-20261120/retained/verdict.json`.
