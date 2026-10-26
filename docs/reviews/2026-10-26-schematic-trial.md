# A fresh agent's trial from the schematic — 2026-10-26

An agent that had never used pcb-maker took one design from its
schematic to a fab release, reading only README.md,
skills/pcb-maker/SKILL.md, docs/agents.md, docs/constraints.md and
`--help`, and wrote down every point of friction. The earlier trial of
the day ([2026-10-26-fresh-agent-trial.md](2026-10-26-fresh-agent-trial.md))
started from boards with their parts; this one starts from the
`.kicad_sch`. Binary x1844 throughout (main 56ce9a4; x1845 appeared after
the first route had started), KiCad 10.0.7, a shared machine (46 to 57
runs in the memory ledger, a nightly and four other agents).

## The design

little-red-rover/little-red-rover, `HARDWARE/little_red_rover` (CERN OHL,
commit 1509a7a), from benchmarks/real/external/github-sch: one sheet, a
small robot's main board. 2 layers, 65 footprints, 328 pads, 64 routable
connections on 77 nets; an ESP32-S3-MINI-1 module, a BQ24072 charger
(VQFN-16), a boost converter and an LDO, two DRV8212P motor drivers
(WSON-8), an IMU (LGA-14 at 0.5 mm), USB-C (GCT USB4105), two 18650 clips,
a 2 x 12 header on the back, three M3 inserts. The designer's outline is
a half-disc joined to a rectangle with notches on both sides (9 arcs, 49
lines); the designer's board has a GND pour on F&B.Cu and a copper keepout
along the right edge. No `.kicad_dru` (none of the candidates outside the
short and schematic tiers with 150 to 500 pads on 2 or 4 layers has one).
Not in the short tier, the schematic tier or the dense set.

Picked by a script over the 79 boards: designer's board on 2 or 4
layers, 150 to 500 pads, not in short.json or schematic.json. The others
left were ligra_back, OpenMuscle-FlexGrid, ULK (17 to 24 GB a run) and
three bumwings keyboards.

## Log

**F** marks friction: a guess, a retry, or reading something other than
the user documentation. Wall times from a wrapper around each command.

| # | Command | Expected | Happened | Wall |
| --- | --- | --- | --- | --- |
| 1 | read README, SKILL.md, agents.md (1228 lines), constraints.md | the schematic path | README's "From a schematic to fab files" and SKILL.md's six steps are the plan. Clear. | - |
| 2 | `import-kicad-netlist repo/.../little_red_rover.kicad_sch board little_red_rover` | a board | **F** exit 1, 3 footprints missing: U10 `Espressif:ESP32-S3-MINI-1` (no such library: Espressif's add-on) and U5, U6 `Package_SON:WSON-8-1EP_2x2mm_P0.5mm_EP0.9x1.6mm_ThermalVias` ("Package_SON.pretty has no ..."). KiCad 10 has the latter as `Texas_DSG0008A_WSON-8-1EP_2x2mm_...`; nothing says so, `substituted` is empty. Found by `ls` of the library. | 1 s |
| 3 | the same with `--footprints-from designer/little_red_rover.kicad_pcb` | complete | exit 0, 65 footprints, the three from the board, 62 `board_versions`, no drift. The message of step 2 named this option. **F** (small) `constraints` and `layout` are `null` in the second report (kept, not written); the paths are what one wants next. | 2.7 s |
| 4 | `describe-kicad-board` of the import and of the designer's board | parts, nets, outline | 0.0 s and 1.4 s. The import has no outline (`null`), as documented. The designer's `outline` has `points`: the 128 corners of the shape, arcs as chords. **F** nothing says these can be the `polygon` of `constraints.json` (they can: fixed in the docs). | 2 cmds |
| 5 | `describe-kicad-board board ... --constraints-template board/constraints.template.json` | starter constraints | 1 edge (J1, `any`), 11 near entries, a comment list. **F** C7 (22 uF on VDD, the second motor driver's bulk capacitor) was sent to `U3:3`, the LDO's EN pin tied to VDD; the designer has it at U6. The import's own `constraints.json` has `outline: {}` and the edge only; the template has no outline key. | 1 cmd |
| 6 | write constraints.json: the template, the designer's outline (polygon and corner), J1 bottom facing out, U10 on the right edge at 270 degrees (antenna out), the three inserts at the designer's positions (`origin: absolute`), J8 on the back, C7 at U6:1 | | a Python edit of the JSON. | - |
| 7 | `place-kicad-board board little_red_rover place1 board/layout.json` | constraints checked | exit 1 in 7.4 s: every part placed, `near C3 U3 (0.16 mm)` missed, "relax them or give the parts room". Correct; left as is (the layout kept it). | 7 s |
| 8 | `layout-kicad-board board little_red_rover layout1 auto board/layout.json` (default budget) | a board | exit 1 in 160 s (672 s of work): 64/64 routed, KiCad 0 unconnected, 65 vias, 2468 mm, every constraint kept, but `hole_clearance 4`: J1's own GND pads 0.19 mm from its own locating holes (rule 0.25 mm). **F** next action "move the parts apart" (no placement changes a footprint's own geometry; fixed in the message). A decoupling action too (below). | 160 s |
| 9 | `kicad-cli pcb drc --schematic-parity --severity-all` on the result | the verdict | 0 unconnected, 0 parity issues, the same 4 hole_clearance errors; 51 `lib_footprint_mismatch` warnings (the board's footprints over the library's). | 15 s |
| 10 | `inspect-routing-run layout1 --reference designer` | nothing open | "inspected 0 nets", native `failed` (the 4 errors). | 1.5 s |
| 11 | `fab-kicad-board layout1/result little_red_rover release1` | Gerbers | **F** exit 1, refused on the 4 hole_clearance errors; "fix the findings (... move what is too close)" or `--allow-findings`. The designer's board has the same 4 (and 17 clearance errors); a layout has no reference by default and the fab gate takes none. | 3.3 s |
| 12 | the verdict's decoupling suggestion: `{"placer": {..., "decoupling_near": true}}`, layout again | capacitors nearer | exit 1 in 167 s, **the same board**: every footprint at the same pose, 65 vias, 2468 mm, 40 % within 3 mm, and the same action, still offering `decoupling_near` (now on) and the example `C5` at `U2:39` (U2 has 17 pads; fixed). | 167 s |
| 13 | a custom rule for J1 in `little_red_rover.kicad_dru` (hole clearance 0.15 mm where both items are J1's, `memberOfFootprint`), then `drc-kicad-board layout1/result ...` | clean | exit 0: 0 errors, 0 unconnected. **F** 657 s of wall for a 1.5 s check: `memory ledger: waiting to reserve 2253 MB ... behind an older request, run.py: ... (waiting 4312 s for 3004 MB; the ledger drains for it)`, with 33 of 58 GB reserved. | 657 s |
| 14 | `fab-kicad-board layout1/result little_red_rover release2` | released | exit 0 in 7.3 s: 21 files, the zip, STEP, BOM from the schematic (`JLC` field; SW1 to SW3 without one), ERC 5 errors (the schematic's own: power pins without a PWR_FLAG, an unconnected LED output; not blocking). | 7 s |
| 15 | the designer's placement: `strip-kicad-tracks`, `route-kicad-board designer-stripped little_red_rover droute auto` | the comparison | exit 0 in 180 s (267 s of work): complete, 163 vias, 4404 mm; the designer's 13 clearance and 4 hole_clearance findings at the same places are pre-existing. | 181 s |
| 16 | `kicad-cli pcb drc --severity-all` on the routed designer placement | 0 unconnected | **F** 24 unconnected. With `--refill-zones`: 0. The written board's zones carry no fill (0 `filled_polygon`); the docs never said to refill (fixed). | 2 cmds |
| 17 | `score-kicad-board` on the three boards | quality | vias, length, decoupling. **F** the designer's `manufacturing.drc` is `{}` (it reads a `drc.json` beside the board, which a designer's project has not), easily read as clean: the board has 21 errors. | 3 cmds |

From the first import to the release: 25 minutes, 18 pcb-maker commands
and 4 kicad-cli runs (plus 4 for the result table), of which 11 minutes
were one ledger wait; the two layouts took under 3 minutes each.

## The result

KiCad 10.0.7 (`kicad-cli pcb drc --refill-zones`) on each board; vias and
track length by `score-kicad-board`.

| Board | KiCad unconnected | Errors | Vias | Track mm | Wall s |
| --- | ---: | --- | ---: | ---: | ---: |
| designer's own board | 0 | 21 (clearance 17, hole_clearance 4) | 170 | 4011 | - |
| designer's placement, our route (default budget) | 0 | 17 (clearance 13, hole_clearance 4; all the designer's, at the same places) | 163 | 4404 | 180 |
| our layout from the schematic | 0 | 4 (J1's own hole_clearance) | 65 | 2468 | 160 |
| the same with the J1 rule (`.kicad_dru`) | 0 | 0 | 65 | 2468 | - |

Not like for like: the designer's board and its route have a GND pour on
both layers (stitching vias among the 170 and 163; the copper keepout
along the right edge too), our layout has none (the import brings no
zones and I added none); and our placement ignores what the schematic
does not say: the two 18650 clips, 73 mm apart on the designer's board
so that a cell fits between them, land where the wirelength puts them
(J5 at the middle, J6 turned). The electrical result is clean; the
mechanical one is not the designer's.

## The verdict's advice

- Place check, `near C3 U3` missed by 0.16 mm: "relax them or give the
  parts room": correct, one step.
- Import, footprints missing: names `--footprints-from` and the libraries:
  correct and actionable. Silent on KiCad 10's rename of the WSON-8.
- Layout, `hole_clearance 4`: "move the parts apart": wrong. Every
  finding lies between one footprint's pad and its own hole. Fixed in the
  message (names the footprint, `--reference`, a custom rule, an
  exclusion, `--allow-findings`).
- Layout, decoupling (40 % within 3 mm; U10:3 47.5 mm, U5:1 39.9 mm):
  not actionable here. Every capacitor of the template is under a kept
  `near` constraint; U5:1 has its 22 uF beside it (C8, within 3 mm by
  constraint), which the score does not count at a pin of a non-regulator,
  and neither pin has a capacitor of 1 uF or less in the schematic (the
  designer's board: U10:3 77 mm). Following the second half
  (`decoupling_near`) changed nothing, and the action came back the same,
  offering the switch already on. The example `C5` at `U2:39` named a pad
  U2 does not have (fixed: the example names the farthest pin).
- Fab gate: "fix the findings (route the open connections, move what is
  too close)": not for a footprint's own finding; `--allow-findings` is
  offered.

## Frictions, ranked by cost

1. **The ledger's drain** (657 s for a 1.5 s DRC; the earlier trial's
   friction 1, "fixed on main (x1835)"): the first waiter, a run.py with
   `--min-free-gb 10`, had waited 4300 s for 3 GB with 25 GB of the
   ledger free and 31 GB available, and every new request queued behind
   it. Whatever holds that run.py (its floor or its own job slots), the
   drain still holds others for a waiter that does not start. Not
   doc-sized.
2. **A footprint's own DRC findings block a schematic-only release**
   (step 11, a .kicad_dru written by hand). The layout has no reference
   and the fab gate none at all, so a library footprint whose pads are
   closer to its own holes than the project's rule fails every layout.
   The import already handles the copper case (`clearances`: a
   footprint's pads closer than the clearance get their gap as their own
   clearance); proposal: the same for hole clearance (a custom rule for
   that footprint written by the import, or such findings judged as the
   footprint's own, as with a reference). Message fixed.
3. **The decoupling action repeats after it is followed** (167 s for an
   identical board). Proposal: no `decoupling_near` advice when it is on;
   no pin named whose rail has no capacitor of the counted kind left
   free; count a constrained bulk capacitor beside a driver's supply pin.
4. **Three footprints missing on import**: the Espressif add-on (no
   remedy but a library or `--footprints-from`) and KiCad 10's renamed
   WSON-8. Proposal: where a library lacks the name, a unique footprint
   of that library whose name ends in `_<name>` (KiCad 10 put a vendor
   prefix on some) as a `substituted` entry.
5. **kicad-cli without `--refill-zones`** reports 24 unconnected on a
   complete board (the written zones are unfilled): doc fixed (agents.md
   and SKILL.md).
6. **Getting the designer's outline**: the polygon is in
   `describe-kicad-board`'s `outline.points`, undocumented: doc fixed.
7. **The template's capacitor at an EN pin** (C7 at U3:3): the template
   counts an enable pin tied to the rail as a supply pin. Proposal:
   leave out pins named EN/SHDN/CE, or prefer an IC whose supply pin is
   named for power.
8. **score-kicad-board's `drc: {}`** without a drc.json; proposal:
   `null` with a note.
9. **`place` with `x`, `y` is not in `board-layout.json`'s
   constraints** (MH1 to MH3 held where asked, but not listed), against
   "every constraint". Proposal: list it as kept by construction.
10. The import report's `constraints`/`layout` `null` on a second
    import: proposal, the path with `kept: true`.

## Fixed on the branch (each its own commit)

- crates/pcb-kicad/src/verdict.rs: the decoupling action's example names
  the board's farthest supply pin, not `U2:39` on every board.
- crates/pcb-kicad/src/verdict.rs: a DRC type whose findings each lie
  within one footprint says so (the footprint, `--reference`, a custom
  rule, an exclusion, `--allow-findings`) instead of "move the parts
  apart" (unit test).
- docs/agents.md, docs/constraints.md: another board's outline goes into
  `outline.polygon` from `describe-kicad-board`'s `points`.
- docs/agents.md, skills/pcb-maker/SKILL.md: a result checked with
  `kicad-cli pcb drc` directly needs `--refill-zones`.

## Against the previous trial

- **Gone**: the starter constraints for a board (`--constraints-template`
  ran in 0 s; one wrong anchor, friction 7); the placement count ("placed
  62 of 65 parts (3 kept where they are)"); the keep_whole advice (no pour
  here, not exercised); "complete" with far decoupling (now said, but see
  friction 3); stacking parts (an import is stacked).
- **Remains**: the ledger's drain (friction 1, as long a wait as then);
  decoupling as good as a designer's (40 % against the designer's 64 %
  here); the `opening_outwards` angle not named (not met: J1's mouth was
  clear); `access` and `cause` unknown (nothing open here).
- **New to the schematic path**: footprints missing from KiCad 10's
  libraries (friction 4; `--footprints-from` solves it when a board
  exists); the outline (friction 6); a footprint's own findings with no
  reference to subtract them (friction 2, which a board-based trial with
  `--reference` never meets); the zones unfilled for a direct kicad-cli
  check (friction 5, met here on the designer's route; the earlier trial
  passed `--refill-zones` from its own knowledge); intent the schematic
  does not carry (the battery clips' distance, the GND pour, the edge
  keepout), which no starter can infer.

Inputs and outputs: build/trial2/ in the main checkout's build
directory (commands.log has every command with its exit code and wall
time).
