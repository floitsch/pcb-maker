# A fresh agent's trial — 2026-10-26

One agent that had never used pcb-maker laid out two boards new to the
tuning, reading only README.md, skills/pcb-maker/SKILL.md, docs/agents.md,
docs/constraints.md and `--help`, and wrote down every point of friction.
Binary: x1833 (main e935324; the first minutes ran x1832, rerun on x1833
on the supervisor's word), KiCad 10.0.7, a shared machine (load 19 to 23
on 32 cores, up to 35 runs in the memory ledger).

## The boards

Every board of `benchmarks/real/external/github` is a task of
`benchmarks/agent-tasks/tasks/v7` (all 109 names match), so both boards
come from the expansion intake (`benchmarks/expansion/data/boards`), outside
the expansion tier, with a `.kicad_pro` and a `.kicad_dru`:

| | A: lpc1311-target-v1 | B: rp2350-dual-usb |
| --- | --- | --- |
| Source | christiangroothuis/fault-injection-library (GPL-3.0) | lichen-community-systems/lichen-eurorack-hardware |
| Layers, outline | 2, 57 x 25.5 mm, round corners | 4, 51 x 32 mm |
| Parts, pads, nets | 38, 194, 29 | 53, 247, 40 |
| Hard parts | QFN-32 at 0.65 mm, USB-C, SMA | RP2350 QFN-60 at 0.4 mm, two overhanging USB-C, two jacks, 18 0402 capacitors |
| Pours | GND on F&B.Cu | GND on In1/In2, +3V3 on B.Cu, 70 small teardrop zones |
| `.kicad_dru` | empty | JLCPCB rules (0.127 mm outer tracks) |

Each was prepared as the layout tier prepares a task: tracks stripped
(`strip-kicad-tracks`), the parts stacked at the board's centre (run.py's
`unplace`), the outline kept. Two constraints an engineer would want: the
USB-C receptacle on the left edge opening outwards (A: J7; B: J2) and a
decoupling capacitor at its IC's supply pin (A: C4 within 2 mm of U2:10,
the CH340X's VCC; B: C13 within 3 mm of U2:44, the RP2350's IOVDD).

## Log

Times are wall clock; "work" is the verdict's `work_seconds`.
**F** marks friction: a guess, a retry, or reading something other than
the user documentation.

| # | Command | Expected | Happened | Wall |
| --- | --- | --- | --- | --- |
| 1 | read README, SKILL.md, agents.md (1150 lines), constraints.md | the workflow | Clear workflow: describe, constraints, place check, draft, final, release. agents.md is long but its section 3 "Iterate quickly" is the plan to follow. | 10 min |
| 2 | `describe-kicad-board designer lpc1311-target-v1 > board.json` | parts, pads, nets | 0.05 s; every footprint with `at`, `body`, its pads and nets; every net's `REF:PAD`. Found the CH340X's VCC (U2:10) in `nets`. | 1 cmd |
| 3 | **F** stack the parts | a way to place a placed board anew | Nothing in the docs says whether `move_all` depends on where the parts start, and no command stacks them. Used `benchmarks/agent-tasks/run.py`'s `unplace` from a script. Later measured: A placed from the designer's positions and from the stack gives the same poses, part for part (fixed in constraints.md). | 3 cmds, 5 min |
| 4 | `place-kicad-board task <id> place-c task/layout.json` (A) | the constraints checked in seconds | 3.5 s, exit 0, 3 of 3 kept. **F** "placed 38 of 52 components": the board has 38 footprints; the other 14 (69 from the unstacked board, whose 17 labels stay) are, it seems, the board's own marks. Not documented. | 1 cmd |
| 5 | layout draft (`moves: 0`), A | a first board | x1832: 38 s, complete, 33 vias, 512 mm. x1833: complete, 16 vias, 433 mm, 38 s of work in 495 s of wall: **F** 7 minutes in `memory ledger: waiting ... behind an older request ... the ledger drains for it`. | 1 cmd |
| 6 | final layouts A (with and without constraints) and the designer's placement routed, side by side | results | All three exit 0 (table below). Each waited 6 to 7 minutes in the ledger (work over wall 0.20). | 3 cmds |
| 7 | `place-kicad-board` (B) | constraints checked | **F** exit 2: `edge constraint for J2: cannot tell which way it opens (no side of its courtyard reaches clearly beyond its pads); give a rotation instead`. The footprint is an "Overhanging" USB-C. Which angle faces left? Took the designer's -90 from describe (a fresh import has every part at 0; then it is a guess). Fixed in constraints.md. | 2 s |
| 8 | the same with `rotation: -90` | exit 0 | **F** exit 1: `near C13 U2 (1.09 mm)` missed at 2 mm; next action "relax them or give the parts room". Relaxed to 3 mm: exit 0. Correct and actionable. | 2 cmds, 8 s |
| 9 | draft, final with and without constraints, designer route (B) | results | All four exit 0, after 3 to 6 minutes each in the ledger. | 4 cmds |
| 10 | `kicad-cli pcb drc --severity-all --refill-zones` on both final boards and both designer boards | the tool's verdict | 0 unconnected, 0 errors on all four, as the verdicts say; warnings only (text, library, silkscreen). | 4 cmds |
| 11 | `drc-kicad-board final-c/result <id> drc.json` | the same | 0 errors, 0 unconnected, 62 cosmetic, from the DRC cache in 0.2 s. | 1 cmd |
| 12 | `inspect-routing-run final-c --reference designer` | nothing open | "inspected 0 nets", native `passed`. | 1 cmd |
| 13 | the verdict's one suggestion on A: "the GND pour is in many pieces (GND on F.Cu: 20 pieces ...): keep_whole in its add_pours entry ... under named parts" | a whole F.Cu GND under the ICs | `route-kicad-board final-c/result <id> fix-keepwhole keepwhole.json --strip` with `{"add_pours": [{"net": "GND", "layers": ["F.Cu"], "keep_whole": [{"parts": ["U3", "U2"]}]}]}`: **F** exit 1, 22 unconnected, a new `zones_intersect` error. The pour got whole (8 pieces, the largest 48 %), every signal pad of U2 and U3 lost its F.Cu access. | 70 s |
| 14 | `inspect-routing-run fix-keepwhole --reference final-c/result` | why | One command, 0.05 s: 18 nets, every dead pad lists `keep whole GND F.Cu` among the objects near it, the reference `connected: true` for every net. The diagnosis is right and fast; the advice that led there was the problem (fixed in the message). The zones_intersect: add_pours wrote a second F.Cu GND zone over the existing F&B.Cu zone, although docs/agents.md says a layer where the net already has a zone is skipped (not fixed: code). | 1 cmd |
| 15 | `fab-kicad-board final-c/result <id> release-a --profile jlcpcb` and B without a profile | Gerbers | exit 0, 21 files each, 3.6 s and 4.8 s; next actions on LCSC numbers and rotations. | 2 cmds |
| 16 | `score-kicad-board` on all eight boards | quality | B's layout: **F** decoupling capacitors a median 6.9 mm from a supply pin, 7 % within 3 mm, against the designer's 1.8 mm and 86 %; U2:39 (+1V1) has its nearest capacitor 14.3 mm away. The verdict says complete with no word of it. | 8 cmds |

About 40 commands in 24 minutes from the first command to the second
release (plus 10 minutes of reading), of which about 7 minutes were the
ledger's waits; without them a layout of either board takes 1.5 to 4
minutes.

## The result

KiCad (kicad-cli 10.0.7, zones refilled) on every result: 0 unconnected,
0 errors. Vias and track length by `score-kicad-board`.

| Board | Run | Verdict | Vias | Track mm | Work s | Wall s |
| --- | --- | --- | --- | --- | --- | --- |
| A | designer's own board | - | 42 | 943 | - | - |
| A | designer's placement, our route | complete | 35 | 811 | 7 | 506 |
| A | layout, constraints (final) | complete, 3/3 kept | 15 | 464 | 97 | 476 |
| A | layout, constraints (draft, `moves: 0`) | complete | 16 | 433 | 38 | 495 |
| A | layout, `move_all` only | complete | 16 | 503 | 94 | 539 |
| B | designer's own board | - | 154 | 493 | - | - |
| B | designer's placement, our route | complete | 55 | 575 | 552 | 525 |
| B | layout, constraints (final) | complete, 7/7 kept | 70 | 863 | 236 | 412 |
| B | layout, constraints (draft) | complete | 72 | 911 | 167 | 389 |
| B | layout, `move_all` only | complete | 59 | 791 | 187 | 405 |

The wall column includes 3 to 7 minutes of waiting in the memory ledger.
On B the designer's placement needed the 0.05 mm rung (eleven attempts,
the first ten 13 to 31 connections open); our placement routed at the
first rung. On A the layout uses fewer vias and half the copper of the
designer; on B more copper (1.75 times) and the decoupling is far worse
(above). Without the edge constraint A's USB-C does not face out
(`connectors.facing_out` 0 of 1); B's other connectors, unconstrained,
land in the middle of the board, with their overhanging bodies inside it.

## The verdict's advice

- Place check (B, `near` missed): "relax them or give the parts room":
  correct, one step.
- Place check (B, `opening_outwards`): "give a rotation instead": correct,
  but which angle is left to the reader.
- Layout and route next actions on A: one, the pour in pieces. Following
  it as written (`keep_whole` under the named ICs) broke the board: 22
  connections open and a new error. The advice is not wrong about
  `keep_whole` (docs/agents.md warns that another net's pad inside a region
  is unreachable on that layer), but the next action invited naming the
  ICs and did not carry the warning; it also repeated itself on the broken
  run. Cost: 70 s and one failed board.
- On B: one, two blocked silkscreen fields (cosmetic). Correct; not tried.
- Missing: nothing flagged B's decoupling.

## Frictions, ranked by cost

1. **The ledger's drain held every run for 3 to 7 minutes** (all 8
   layouts and routes; work over wall 0.2 to 0.6). The request the
   ledger drained for (a nightly run.py, pid 4134524, waiting 26.6 hours
   for 24.7 GB) would fit the ledger's room as the waiting line counts it
   (26 GB reserved of 58 GB); presumably run.py's own MemAvailable floor
   (`--min-free-gb`) keeps it waiting, so the drain holds new runs for a
   request the ledger alone cannot start (not checked in the code). Proposal: drain
   only for a request that the ledger's room would admit now and that
   then starts, bound the drain (or exclude requests gated by a floor
   outside the ledger), and say in the waiting line how long a drain has
   held this run. Not doc-sized.
2. **The pour advice broke the board** (70 s, a failed run): message
   fixed (below). Remaining, code: add_pours over a layer where the net
   has a multi-layer zone (`F&B.Cu`) writes a second zone
   (`zones_intersect`), against the docs; an input check could warn when a
   `keep_whole` region covers other nets' pads on its layer, as keepouts
   over held parts are warned of.
3. **"Complete" with decoupling far from the IC** (B): the automatic
   decoupling pull (soft, 2.5 mm) did not hold around a QFN-60 with 18
   capacitors; 7 % within 3 mm against the designer's 86 %, a +1V1 pin
   14.3 mm from its nearest capacitor. Proposal: a next action when a
   supply pin's nearest capacitor is beyond 5 mm (the score has it), or
   the automatic pull as a reported constraint.
4. **No starter constraints for a board without a schematic** (B's
   connectors in the middle; minutes of reading to find which parts are
   connectors): the import's starter (plug-in connectors on `edge: any`,
   facing out) exists only for `import-kicad-netlist`. Proposal:
   `describe-kicad-board` writes the same starter for a board.
5. **opening_outwards on an overhanging receptacle** (a retry, a guessed
   angle): doc fixed; a better message would name the angle that faces
   each edge.
6. **Stacking the parts** (5 minutes, harness code read): unnecessary, as
   measured; doc fixed.
7. **"placed 38 of 52 components"** (a puzzle): the count includes the
   board's marks. Proposal: say "38 parts placed; 52 placer objects with
   the board's marks", once the meaning is confirmed in the code.
8. **"0.4 within 3 mm"** reads as a count: fixed.
9. **SKILL.md's exit-1 paragraph** said open connections "mean too little
   room", which docs/agents.md and the verdict no longer say: fixed.
10. The supervisor's `--min-free-gb 10` is run.py's flag; the binary has
    none and the docs say so implicitly (the ledger is automatic). No
    change.

## Fixed on the branch (each its own commit)

- skills/pcb-maker/SKILL.md: open connections call for
  `inspect-routing-run` first; room is a hypothesis.
- src/agent_cli.rs: the layout's decoupling line prints the share as a
  percentage ("40 % of them within 3 mm").
- crates/pcb-kicad/src/verdict.rs: the pour advice says a `keep_whole`
  region over another net's pad leaves that pad unreachable on the layer.
- docs/constraints.md: what to do when `opening_outwards` cannot tell
  which way a connector opens.
- docs/constraints.md: `move_all` lays out a placed board anew without
  stacking its parts (measured on board A).

## Against the earlier trials

docs/reviews/2026-10-18-remapper-agent-diagnostic-trial.md:

- Gone: the run-aware inspection (P0) answered "which pads, which rules,
  does the reference route it" in one command (step 14); the outline with
  arcs (P0) was accepted by every command used, describe, place, layout,
  route, inspect, score; native reconciliation (P1: `checks.native`
  passed/failed with the pairs); the attempt ledger (P1: B's eleven
  attempts with pitch, opens, vias, work and stop reason); "too dense" in
  the docs and the verdict (P1; the skill kept one sentence of it, fixed
  here); `--help` with defaults and units (P2).
- Not exercised: the below-minimum override preflight.
- Remains: `access` and `cause` are `unknown`; no region overlay.

~/tmp/fence-report.md (2026-10-21):

- Gone: vias in small SMD pads (`via_in_pad` false by default; 0 in pads
  on every result here); a pour in pieces is now said in the verdict (but
  see friction 2); the decoupling summary and the verdict carry
  `silkscreen` (fields placed, blocked ones named).
- Not exercised: board texts moved by polishing, keepouts over fixed
  parts, repair-kicad-silkscreen on a `gr_rect`, the ladder stopping
  early (no board here was incomplete).
- Remains: a rail the decoupling score does not see: U2:46 (VREG_AVDD)
  is `null` on B's designer board and on ours although C10 sits on that
  rail beside the pin, as the fence's U2:2 was.

Inputs and outputs: build/trial1/ (A) and build/trial1/b/ (B) in the main
checkout's build directory.
