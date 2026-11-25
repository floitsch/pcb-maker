# Window census: is the router's residue a search failure?

The census of Proposal A in the research note of 2026-11-23
(`build/scratch/fundamentals/fundamental-approaches.md`, section 2, "The
cheapest decisive experiment"): when a routed board ends with a few KiCad
unconnected items, does a legal solution exist in a small window on the
router's own lattice? Every endgame repair today is a sequential
single-net move (the hard repair, `trouble.rs`, the open repairs); nobody
had checked whether solutions exist. Code: `census-kicad-window`
(`crates/pcb-kicad-lab/src/window_census.rs`, on
`Router::window_census` in `crates/pcb-router/src/router/window_census.rs`
and `Router::footprint_of` in `router/stamping/stamp.rs`) and
`experiments/window-census/` (the SAT model, the driver, the board
application, the tables).

## Verdict

**No go, and no kill by the note's rule: do not build the in-tree CDCL
endgame now.**

| Criterion (note, section 2) | Threshold | Measured |
| --- | --- | --- |
| Go: open items SAT | at least 30 % | 14 of 117 (12 %) on the lattice, 7 (6 %) that KiCad accepts; 18 % of the 77 items the census could pose |
| Go: boards strict under KiCad | at least 5 | 4 (2 of them on main's binary) |
| Kill: SAT at 4 mm | under 15 % | 12 % (h = 4 adds none) |
| Kill: cores | more than about 8 nets | 4 of 5 cores name 2 to 5 nets; one names 11 |

The SAT half of the kill rule holds, the core half does not: where the
solver proves a window infeasible, the cause is local. But the result that
decides the bet is a different one: **13 of the 14 SAT answers needed no
other net moved.** The item's net alone, routed on the lattice against all
copper as the router left it, closes the item; only one (Sisu's +3.3V)
needed two other nets rerouted. Every item the router left open behind a
wall of other nets' copper (19 items: 5 UNSAT, 14 timeouts) stayed open
for the solver too. So the residue is not sequential best-response cycling
that a joint solve would untangle: it is single-net misses the endgame
never tried (mostly pour nets the router believed joined by their fill),
model classes (dead pads, pockets, pour islands, long items), and genuine
joint conflicts that CaDiCaL cannot decide at window size within 10
minutes.

What would pay instead (section "What to build next"): a single-net retry
of each KiCad-open item against the routed copper, pours ignored and
refilled, KiCad as the judge. On these boards that alone closes 6 of the
7 items KiCad accepted and makes the same 4 boards strict, at a cost of
seconds a window.

## Results

117 KiCad unconnected items on 42 boards. Classes (the better of the two
half-sizes; `experiments/window-census/summarize.py`):

| Class | Sweep 11-18 | Designer 11-24 | All | What it means |
| --- | ---: | ---: | ---: | --- |
| sat-kicad | 5 | 2 | 7 | a solution in the window; KiCad finds the item closed and no new error |
| sat-lattice | 6 | 1 | 7 | a solution on the lattice that KiCad does not accept |
| unsat | 4 | 1 | 5 | no solution in the window (1 certified without a budget, 4 within the detour budget) |
| walled-dead | 8 | 4 | 12 | a side of the item is a pad with no lattice node at all |
| walled-pads | 6 | 3 | 9 | a side no lattice path reaches inside the window, even with every track and via of the board removed |
| walled-cu | 1 | 1 | 2 | walled by copper kept as routed (pairs, nets the lattice cannot rejoin) |
| timeout | 7 | 7 | 14 | no answer in 600 s |
| too-big | 12 | 9 | 21 | over 150,000 nodes or 12 million clauses |
| zone | 9 | 3 | 12 | between pieces of a pour: fill connectivity, not the lattice's |
| long | 12 | 10 | 22 | the window would be wider than 20 mm (items of 9 to 149 mm) |
| error | 2 | 4 | 6 | pads off the board (SUMEC's R12 and C19), the item's net not found (Hub), out of the census's 8 GB |

The half-size of 4 mm changed no class to SAT: of the 43 items h = 2 left
undecided, 7 stayed timeouts, 14 became too big, the walls stayed walls.

**Strict passes.** `apply.py` applied the SAT answers board by board (each
window censused again on the board the windows before left):

| Board | Tier (binary) | KiCad unconnected | Strict |
| --- | --- | --- | --- |
| poly_kybd_split72_left | sweep 11-18 (3b27f2d0eb93) | 1 -> 0 | yes |
| capybully | sweep 11-18 (3b27f2d0eb93) | 3 -> 0 | yes (tonight's sweep passes it already, on its own placement) |
| poly_kybd_split72_right | designer 11-24 (96e891b27162) | 1 -> 0 | yes |
| DSP-ADAU1452 | designer 11-24 (96e891b27162) | 1 -> 0 | yes |
| Sisu_SSE-9 | sweep 11-18 | 4 -> 3 | no |
| osprey, WLED_Matrix, reCamera_S101, RP2040_base | | unchanged (the answers add KiCad errors) | no |

Strict here means no unconnected item, no error type more often than on
the routed board (both by `drc-kicad-board`, zones refilled), and no other
new strict finding in the nightly's row; run.py's rule 5 was not applied.

**The SAT answers, one by one.**

- *Pour nets the router thought joined* (5 items, 3 boards): GND on
  split72_left (a mounting hole's pad and a capacitor 1.6 mm apart), GND
  three times on capybully (the two copper sides of the hot-swap sockets'
  pad 3), +3.3V on Sisu. The attempt kept connects pours; the router counted
  the connection made by the fill (split72_left's verdict has
  `open_connections` 0, and its layout log has a KiCad check "skipped by
  the router's count"), KiCad's fill did not make it. A track was there to be had: four
  segments on split72_left, one via and a few segments on capybully.
  Sisu's was the one answer that freed other nets (2).
- *Signals with a path against the routed copper*: VDD on split72_right
  (the attempt "pours=connect (plane stubs)": exclusive planes the census's
  lowering does not have; KiCad refills GND around the new track), DVDD on
  DSP-ADAU1452 (the narrow-signal step's board; why its repairs missed a
  path the regular lattice holds is not established).
- *KiCad rejects* (7): pads a footprint's own copper graphic walls
  (osprey's and WLED's antenna feeds AE1, reCamera's MIC1 ring, WLED's GND):
  the answers cross the graphic and KiCad reports `shorting_items` and
  `clearance` against it, the input's finding that run.py's rule 5 sets
  apart when the designer pairs it the same way; and RP2040_base's
  QSPI_SS_2, where the answer leaves a `track_dangling`.

**The UNSAT cores** name few nets: keyboard VSS_PA {GND, VSS_PA} (twice, 2
and 4 mm, and the designer tier's at 2 mm {U2-ANT, U2-DEC3, U2-DEC5,
VDD_NRF, VSS_PA}, infeasible without a budget too: a certificate), Sisu
+3.3V {+1V1, +3.3V}, Quanta75 +3V3 {two GPIOs, Row5}, and one large one,
corne_split42_left K_D/CS (11 nets).

**The timeouts** are all items whose net is walled by other nets' copper
with everything else kept (19 of 19 timeouts and UNSATs; 1 of 14 SATs).
The stages show how many must move: Quanta75's +3V3 is UNSAT with its 5
nearest nets free and undecided with 10; vd-rd's a14, RP2040's +3.3VA
and vaio's USB_DM stay walled with the 10 nearest free. Windows of 11
(median) to 31 nets on 2 to 8 layers, 0.15 to 3 million variables.

**The walls.** 12 items have a dead pad: no lattice node admits the
class's track (SUMEC's IC2.5, IC2.11, IC3.11: the router's own
`dead_pads` diagnosis, a class wider than the pad), katia's USB-C pads
J8.A6 and J8.B6, WLED's LDO1 pads (its stubs blocked). 9 have a pad in a
pocket the lattice closes within the window even on the board stripped of
every track and via, checked per terminal pair (oro link's U6.33,
corne_right's U5.4, MIDAS's BGA balls U106.H5 and K5, vd-rd's BGA U200
K8 and N9, USB_Keypad's C2.1, protoesp's U1.6, eurorack's RN11.5); 4 of
them still walled at 4 mm. The census's lowering turns BGA fan-outs and
vias in pads off (`lower_for_local`), so for the BGA balls this may be
the census's model, not the router's.

**Cost.** A census of a window takes 5 to 25 s (the whole board's lattice
is built); SAT answers came in 0.1 to 37 s, all within the first stage
that had them. The two passes and the board application took about 3
hours on 4 to 6 cores.

## The residue, classified

| Class | Items | |
| --- | ---: | --- |
| search failure: a solution in the window that KiCad accepts | 7 | 6 of them single-net misses (the item's net alone); 1 joint (Sisu) |
| lattice model or the input's | 12 + 9 + 7 + 12 = 40 | dead pads; pockets of pads and graphics; answers KiCad rejects at walled pads; pour islands |
| needs a change outside the window or many nets | 5 + 2 | UNSAT cores (4 of 5 local, 2 to 5 nets); walled by kept copper |
| undecided | 14 + 21 | timeouts; windows too big |
| not local | 22 + 6 | long items; census errors |

## Method

**Boards.** The strict failures with 1 to 7 KiCad unconnected items of the
sweep of the night of 2026-11-18 (22 boards, binary
`pcb-maker-nightly-2026-11-18-3b27f2d0eb93`) and of the designer tier of
the night of 2026-11-24 (20 boards, binary
`pcb-maker-nightly-2026-11-24-96e891b27162`, the code of this branch's
base). 117 KiCad unconnected items (`experiments/window-census/items.py`).
The 2026-11-24 sweep was still running.

**Window.** Per item, the box of its two ends (KiCad's positions) grown by
the half-size h (2 mm, then 4 mm for the items h = 2 left unsolved). An
item whose window would be wider than 20 mm is *long* and not censused.

**Cut and lowering.** `census-kicad-window` takes every net with a track
or via in the window, cuts its tracks at the window's edge (the parts
outside stay, their ends on the edge) and removes its vias inside, writes
that board, and lowers it as the open repairs lower a routed board
(`lower_for_local`: the copper outside kept as fixed copper, the pads it
joins one terminal, a piece on no pad a terminal of its own; pours not
connected). The router is built on it with the lattice of the attempt the
nightly kept (its pitch and pin lines, from `board-router.json`). The
lattice of the window grown by 0.4 mm (the cut ends' nodes) is dumped per
net, with nothing re-derived:

- the nodes `trace_allowed` allows for the class at the node (the neck
  class in the net's neck zones), the layer steps neither end's blocked
  edges forbid, the through vias `via_allowed` lets stand;
- each terminal's nodes in the window (`terminal_nodes`, escape nodes with
  their stubs);
- per element (a node, a diagonal step, a via, an escape stub) the
  occupancy cells it would stamp, by the router's own marking
  (`mark_stamps` on a one- or two-node branch; a step's or via's stamps
  less its nodes'), and the cells the hard search reads to take it (the
  node's trace map, the diagonal map at the step's start, the via map, the
  stub's cells). Two elements of different nets conflict exactly when one
  stamps a cell the other reads: the hard search's test, with
  `mask_own`'s exemption of a net's own stamps;
- a net's own vias keep the class's via spacing (`mark_own_vias`).

**Requirements.** Per net, the terminals that must end joined are those
the cut copper joined (the original board's terminals, after
`keep_existing_copper`, holding each cut piece); for the item's net, also
the two pieces holding the item's ends. Per requirement, a spanning tree
of its terminals by lattice distance gives the connections. Each
connection may use the nodes on paths at most max(20 steps, 0.5 d) longer
than its shortest d for the item's net, max(10 steps, 0.25 d) for the
others (the detour budget); an UNSAT answer is then confirmed on the whole
window without a budget.

**What stays as it is.** Nets the census does not model are kept as
routed (their copper in the window stays fixed): differential pairs (by
KiCad's +/- and P/N suffixes; a pair's gap is narrower than the lattice's
clearance, and the router routes pairs with its own machinery), except the
item's own net and its partner; and nets the lattice cannot rejoin
(*unreproducible*: a requirement no static path joins once the copper is
cut, mostly copper an open repair or a pair laid off the lattice).

**SAT model** (`experiments/window-census/model.py`). Per connection a
path in the degree encoding: each terminal's super node takes exactly one
way in, a used node has exactly two used incident steps (lattice steps,
via steps between any two of its layers, ways in), an unused node none;
stray cycles are allowed and dropped when the copper is written. The
conflicts go through one auxiliary variable per (net, key): element ->
stamp, stamp and another net's reader -> false. Each net's requirement
holds under a selector literal, so a core is computed by deletion over
the nets (CaDiCaL 2.1.2 has no assumption interface on the command line).
The cut copper snapped to the lattice gives the initial phases (written
with flipped polarity, solved with `--phase=0`).

**Stages.** The full window is often too hard for CDCL. A SAT answer with
fewer nets free is still a solution in the window, so each item is first
solved with only the item's net free (every other net's copper kept as
routed), then with the 2, 5 and 10 nets nearest the item free (by the
distance of their cut copper to the item), 120 s each, then with every net
free (600 s). UNSAT is a certificate only at the last stage.

**KiCad.** A SAT answer's copper (a segment per used lattice step at the
class's width there, a through via of the class's size, the escape stubs)
is written into the cut board, and `drc-kicad-board` (zones refilled)
checks it. The item counts as closed when KiCad finds fewer unconnected
items than on the routed board (checked by the same command) and no error
type more often. `apply.py` then applies the answers board by board, one
window after the other on the board the windows before left, and counts a
board a strict pass when KiCad finds no unconnected item, no error type
more often than on the routed board, and the nightly's row had no other
new strict finding.

## Caveats

- **Pairs and unreproducible nets are kept as routed.** The census does not
  model differential pairs, so a pair's copper in the window stays (except
  the item's own pair); 38 of the 64 windows censused at 2 mm had none,
  the others up to 18 kept nets (pairs and unreproducible together). Nets
  whose copper the lattice cannot rejoin once cut (copper laid off the
  lattice, by a pair or a repair) stay too: 13 of the 64 windows had one
  or more, up to 14. A window counted UNSAT or walled may be solvable with
  them moved; a SAT answer is unaffected.
- **The census's lowering is the open repairs', not the routed attempt's.**
  Pours are not connected (another net's pour is no obstacle; KiCad
  refills around the new copper), BGA fan-outs and vias in pads are off,
  and the classes are the project's even where the kept attempt was the
  narrow-signal or relaxed-clearance step. Pitch and pin lines are the kept
  attempt's.
- **Lattice vs KiCad.** The lattice's clearance is the router's exact
  stamping, so every answer is as legal as a route of the router; KiCad
  still rejected 7 of 14 (the walled pads' graphics, one dangling track).
  A layer step is allowed only when neither end's blocked edges forbid it;
  allowing either end changed none of the walled items. Same-net vias keep
  the class's via spacing; hole-to-hole against the net's own kept vias is
  not modelled.
- **UNSAT within a budget is not a certificate.** Each connection may only
  use nodes within a detour budget (20 steps or half its length for the
  item's net, 10 steps or a quarter for the others); the confirmation
  without a budget was too big for 3 of the 5 UNSAT windows and confirmed 1
  (keyboard, designer tier).
- **Timeouts are windows too big for SAT at 10 minutes**, not evidence of
  infeasibility: in the note's toy calibration the wall was a dozen nets;
  here the median window holds 11 nets on 4 layers and the hard ones 14 to
  31. The cut copper as initial phases and the staged freeing helped the
  SAT side only.
- **Binaries.** The sweep's boards are the 11-18 nightly's (3b27f2d0eb93,
  before the open repairs of 2026-11-20 existed); tonight's sweep passes
  capybully and fails split72_left on a new placement. The designer tier's
  boards are main's binary (96e891b27162).
- **Strict** is judged here by error types against the routed board, not by
  run.py's rule set; the four boards counted have no KiCad error at all
  before or after.

## What to build next

1. **A single-net KiCad-judged retry of each open item** (no SAT engine):
   after the layout's last KiCad check, for each unconnected item, lower the
   board as `lower_for_local` does (pours not connected), route the item's
   net alone with every other net's copper kept, write it and let KiCad
   judge. It closes the 6 single-net items of this census and makes 4
   boards strict (two of them failing on main tonight). Two questions
   first: why `open_connections` is 0 where KiCad's fill leaves a pour pad
   islanded (split72_left), and why the open repairs (`repair_kicad_opens_locally`, which
   lowers the board this way) did not take DSP-ADAU1452's and
   split72_right's items on main's binary.
2. **The walls before any joint solver**: 21 items have a dead pad or a
   pocket the lattice closes even on an empty board, and 12 are pour
   islands. They are the largest local classes, and no search reaches
   them; their fixes are the lattice's (escape and corridor stubs at these
   pads, BGA balls with the fan-out on) and the pour model's.
3. **The joint conflicts** (19 items walled by other nets' copper) remain
   the only place for an exact joint solve, and plain CDCL on the cell
   lattice does not decide them at window size. If they are taken on, the
   path-based formulation (the note's option 2: columns from the router's
   own priced searches, real branching) with the incumbent as its pool is
   the next thing to try; `census-kicad-window` gives it its instances.

## Per item

| tier | board | net | ends mm | h = 2 | h = 4 | note |
| --- | --- | --- | ---: | --- | --- | --- |
| sweep | osprey_rev_a | USB-Di- | 9.26 | timeout | timeout | h2 stages (nets free: answer) 0: separated, 2: separated, 5: separated; h4 stages (nets free: answer) 0: separated, 2: separated, 5: timeout, 10: timeout |
| sweep | osprey_rev_a | Net-(AE1-Pad1) | 2.06 | sat-lattice |  | h2: 0 other nets free |
| sweep | jetson-nano-baseboard | HDMI_SHIELD | 1.2 | walled-cu | walled-cu |  |
| sweep | katia | +1V1 | 141.26 | long |  |  |
| sweep | katia | /Right Board/USB/DATA+ | 3.92 | walled-dead | walled-dead | dead pad J8.A6 |
| sweep | katia | /Right Board/USB/DATA+ | 1.0 | walled-dead | walled-dead | dead pad J8.A6, J8.B6 |
| sweep | Quanta75_BareRP2040_JLCPCBAoptimized | Column5 | 66.27 | long |  |  |
| sweep | Quanta75_BareRP2040_JLCPCBAoptimized | Column10 | 43.42 | long |  |  |
| sweep | Quanta75_BareRP2040_JLCPCBAoptimized | +3V3 | 3.6 | timeout | timeout | h2 stages (nets free: answer) 0: separated, 2: separated, 5: unsat, 10: timeout; h4 stages (nets free: answer) 0: separated, 2: separated, 5: unsat, 10: timeout |
| sweep | Quanta75_BareRP2040_JLCPCBAoptimized | +3V3 | 4.05 | unsat | timeout | h2 core (3): GPIO24_SPI1RX_UART1TX_I2C0SDA_PWM4A_CLOCKGPOUT2, GPIO25_SPI1CSn_UART1RX_I2C0SCL_PWM4B_CLOCKGPOUT3, Row5; h2 without budget: too_big; h2 stages (nets free: answer) 0: separated, 2: separated, 5: timeout, 10: unsat; h4 stages (nets free: answer) 0: separated, 2: separated, 5: timeout, 10: unsat |
| sweep | Quanta75_BareRP2040_JLCPCBAoptimized | +3V3 | 1.6 | timeout | timeout | h2 stages (nets free: answer) 0: separated, 2: timeout, 5: timeout; h4 stages (nets free: answer) 0: separated, 2: timeout, 5: timeout, 10: unsat |
| sweep | Quanta75_BareRP2040_JLCPCBAoptimized | Row4 | 27.9 | long |  |  |
| sweep | link | GND | 5.35 | walled-pads | too-big |  |
| sweep | link | /ULPI_CK | 11.94 | too-big |  |  |
| sweep | link | /ULPI_D1 | 10.05 | too-big |  |  |
| sweep | link | /ULPI_D2 | 9.45 | too-big |  |  |
| sweep | link | /USB/HS_D+ | 15.64 | too-big |  |  |
| sweep | SUMEC_MK_IV | GND | 2.86 | error |  |  |
| sweep | SUMEC_MK_IV | GND | 87.08 | zone | zone |  |
| sweep | SUMEC_MK_IV | +3V3 | 2.03 | walled-dead | walled-dead | dead pad IC2.5 |
| sweep | SUMEC_MK_IV | /MCU/SHLD | 2.86 | error |  |  |
| sweep | SUMEC_MK_IV | /MCU/SHLD | 11.27 | walled-dead | walled-dead | dead pad C19.2 |
| sweep | SUMEC_MK_IV | +12V | 1.27 | walled-dead | walled-dead | dead pad IC3.11 |
| sweep | SUMEC_MK_IV | +12V | 1.46 | walled-dead | walled-dead | dead pad IC2.11 |
| sweep | keyboard | VSS_PA | 6.95 | unsat | unsat | h2 core (2): GND, VSS_PA; h2 stages (nets free: answer) 0: separated, 2: separated, 5: unsat, 10: unsat; h4 core (2): GND, VSS_PA; h4 without budget: too_big; h4 stages (nets free: answer) 0: separated, 2: separated, 5: unsat, 10: unsat |
| sweep | keyboard | RESET | 81.31 | long |  |  |
| sweep | motherboard | GND | 0.0 | zone | zone |  |
| sweep | motherboard | /GPIO0_D3/CC_INT_L | 43.35 | long |  |  |
| sweep | motherboard | /GPIO0_D5 | 36.63 | long |  |  |
| sweep | poly_kybd_split72_left | GND | 1.64 | sat-kicad | sat-kicad (implied) | h2: 0 other nets free |
| sweep | poly_corne_split42_right | /K_R/CS | 3.8 | walled-pads | walled-pads |  |
| sweep | poly_corne_split42_right | /K_T/CS | 21.44 | too-big |  |  |
| sweep | poly_corne_split42_right | /ShiftRegisters/Carry1 | 20.73 | long |  |  |
| sweep | poly_corne_split42_right | GND | 0.0 | zone | zone |  |
| sweep | poly_corne_split42_right | RESET | 12.1 | too-big |  |  |
| sweep | capybully | GND | 0.0 | sat-kicad | sat-kicad (implied) | h2: 0 other nets free |
| sweep | capybully | GND | 0.0 | sat-kicad | sat-kicad (implied) | h2: 0 other nets free |
| sweep | capybully | GND | 0.0 | sat-kicad | sat-kicad (implied) | h2: 0 other nets free |
| sweep | SNSP-CPU-01 | GND | 0.0 | zone | zone |  |
| sweep | SNSP-CPU-01 | Net-(P2-JPIO6) | 30.59 | long |  |  |
| sweep | Sisu_SSE-9 | GND | 0.0 | zone | zone |  |
| sweep | Sisu_SSE-9 | +3.3V | 1.66 | sat-kicad | sat-kicad (implied) | h2: 2 other nets free |
| sweep | Sisu_SSE-9 | +3.3V | 0.12 | unsat | too-big | h2 core (2): +1V1, +3.3V; h2 without budget: too_big; h2 stages (nets free: answer) 0: separated, 2: separated, 5: separated, 10: unsat |
| sweep | Sisu_SSE-9 | PMIC_38_EN | 8.11 | too-big |  |  |
| sweep | Hub | GND | 23.76 | zone | zone |  |
| sweep | Hub | GND | 31.83 | zone | zone |  |
| sweep | poly_corne_split42_left | /K_D/CS | 4.4 | unsat | too-big | h2 core (11): K_A/CS, K_D/CS, K_F/CS, K_G/CS, K_S/CS, GND, RESET, Row3, SR_CLOCK, SR_LATCH, VDD; h2 stages (nets free: answer) 0: separated, 2: unsat, 5: unsat |
| sweep | poly_corne_split42_left | /K_B/CS | 8.39 | too-big |  |  |
| sweep | poly_corne_split42_left | /RpPico/QSPI_SCLK | 11.44 | too-big |  |  |
| sweep | vaio_re | XOUT_E | 6.51 | too-big |  |  |
| sweep | WLED_Matrix | GND | 2.07 | sat-lattice |  | h2: 0 other nets free |
| sweep | WLED_Matrix | GND | 20.48 | zone | zone |  |
| sweep | WLED_Matrix | +3.3V | 4.16 | walled-dead | walled-dead | dead pad LDO1.3 |
| sweep | WLED_Matrix | Net-(LDO1-Vin) | 5.09 | walled-dead | walled-dead | dead pad LDO1.2 |
| sweep | WLED_Matrix | Net-(AE1-A) | 5.68 | sat-lattice |  | h2: 0 other nets free |
| sweep | protoesp | CH_SDA | 58.68 | long |  |  |
| sweep | protoesp | /5D5 | 21.78 | long |  |  |
| sweep | protoesp | /5D3 | 22.82 | long |  |  |
| sweep | MIDAS-MK1.1-revA | Net-(U106-DS) | 0.37 | walled-pads | walled-pads |  |
| sweep | MIDAS-MK1.1-revA | /MEM_RST_N | 7.64 | walled-pads | too-big |  |
| sweep | MIDAS-MK1.1-revA | /LIS3MDL_INT | 2.57 | timeout | timeout | h2 stages (nets free: answer) 0: separated, 2: separated, 5: timeout, 10: timeout; h4 stages (nets free: answer) 0: separated, 2: timeout, 5: timeout, 10: separated |
| sweep | module | +1V5 | 2.53 | walled-pads | too-big |  |
| sweep | module | +1V5 | 1.6 | timeout | too-big | h2 stages (nets free: answer) 0: separated, 2: separated, 5: timeout, 10: timeout |
| sweep | module | +1V5 | 3.6 | too-big |  |  |
| sweep | module | /DDR3 Memory/ddr-a11 | 8.5 | too-big |  |  |
| sweep | module | /DDR3 Memory/a14 | 4.3 | timeout | too-big | h2 stages (nets free: answer) 0: separated, 2: separated, 5: separated, 10: separated |
| sweep | USB_Keypad | GND | 12.93 | zone | zone |  |
| sweep | USB_Keypad | +3.3V | 2.57 | walled-pads | walled-pads |  |
| sweep | reCamera_S101_v1.1 | GND | 1.25 | timeout | timeout | h2 stages (nets free: answer) 0: separated, 2: separated; h4 stages (nets free: answer) 0: separated, 2: separated, 5: timeout |
| sweep | reCamera_S101_v1.1 | GND | 1.81 | sat-lattice |  | h2: 0 other nets free |
| sweep | reCamera_S101_v1.1 | GND | 1.25 | sat-lattice |  | h2: 0 other nets free |
| sweep | reCamera_S101_v1.1 | GND | 1.89 | sat-lattice |  | h2: 0 other nets free |
| designer | osprey_rev_a | c2 | 38.56 | long |  |  |
| designer | poly_kybd_split72_right | VDD | 0.8 | sat-kicad | sat-kicad (implied) | h2: 0 other nets free |
| designer | link | GND | 3.75 | walled-pads | too-big |  |
| designer | Batch_1 | GND | 28.65 | long |  |  |
| designer | Batch_1 | Net-(R28-Pad2) | 34.06 | long |  |  |
| designer | Quanta75_BareRP2040_JLCPCBAoptimized | +1V1 | 2.0 | timeout | timeout | h2 stages (nets free: answer) 0: separated, 2: timeout, 5: timeout; h4 stages (nets free: answer) 0: separated, 2: timeout, 5: separated, 10: separated |
| designer | Quanta75_BareRP2040_JLCPCBAoptimized | Row1 | 22.9 | long |  |  |
| designer | Quanta75_BareRP2040_JLCPCBAoptimized | /~{RESET} | 3.2 | timeout | too-big | h2 stages (nets free: answer) 0: separated, 2: timeout, 5: timeout, 10: timeout |
| designer | jetson-nano-baseboard | /Supply/5V_EN | 3.94 | timeout | too-big | h2 stages (nets free: answer) 0: separated, 2: separated, 5: timeout, 10: timeout |
| designer | jetson-nano-baseboard | Net-(IC7-CURRENT_MODE) | 5.89 | too-big |  |  |
| designer | poly_corne_split42_right | D-C | 22.19 | long |  |  |
| designer | Sisu_SSE-9 | /AUDIO/I_MIC_P | 73.72 | long |  |  |
| designer | Sisu_SSE-9 | /4G LTE/RESET | 5.71 | walled-cu | too-big |  |
| designer | motherboard | /GPIO0_D3/CC_INT_L | 72.08 | long |  |  |
| designer | motherboard | /GPIO0_D5 | 24.1 | long |  |  |
| designer | Hub | GND | 23.76 | zone | zone |  |
| designer | Hub | GND | 31.83 | zone | zone |  |
| designer | Hub | /Zigbee-Thread Module/SPI_ESP32/EFR32MG24_CS | 3.74 | error | error |  |
| designer | SUMEC_MK_IV | GND | 2.86 | error |  |  |
| designer | SUMEC_MK_IV | GND | 87.08 | zone | zone |  |
| designer | SUMEC_MK_IV | /MCU/SHLD | 2.86 | error |  |  |
| designer | SUMEC_MK_IV | /MCU/SHLD | 11.27 | error |  |  |
| designer | SUMEC_MK_IV | +12V | 0.86 | walled-dead | error | dead pad IC2.11 |
| designer | SUMEC_MK_IV | +12V | 0.89 | walled-dead | error | dead pad IC3.11 |
| designer | WLED_Matrix | +3.3V | 2.81 | walled-dead | walled-dead | dead pad LDO1.3 |
| designer | WLED_Matrix | Net-(LDO1-Vin) | 1.41 | walled-dead | walled-dead | dead pad LDO1.2 |
| designer | protoesp | RXD | 3.23 | walled-pads | walled-pads |  |
| designer | power | /HIDRV1 | 11.13 | too-big |  |  |
| designer | keyboard | VSS_PA | 2.51 | unsat | unsat | h2 core (5): Net-(U2-ANT), Net-(U2-DEC3), Net-(U2-DEC5), VDD_NRF, VSS_PA; h2 without budget: unsat; h2 stages (nets free: answer) 0: separated, 2: unsat, 5: unsat, 10: unsat; h4 core (2): GND, VSS_PA; h4 without budget: too_big; h4 stages (nets free: answer) 0: separated, 2: unsat, 5: unsat, 10: unsat |
| designer | keyboard | RESET | 149.4 | long |  |  |
| designer | DSP-ADAU1452 | /ADAU1452/DVDD | 1.76 | sat-kicad | sat-kicad (implied) | h2: 0 other nets free |
| designer | vaio_re | Net-(U6-USB_DM) | 3.68 | timeout | timeout | h2 stages (nets free: answer) 0: separated, 2: timeout, 5: timeout, 10: timeout; h4 stages (nets free: answer) 0: separated, 2: separated, 5: separated, 10: separated |
| designer | vaio_re | LED_PWM | 15.19 | too-big |  |  |
| designer | vaio_re | GPE_P3 | 9.91 | too-big |  |  |
| designer | module | /DDR3 Memory/ddr-dqs0- | 13.91 | too-big |  |  |
| designer | module | /DDR3 Memory/ddr-we | 12.13 | too-big |  |  |
| designer | module | /DDR3 Memory/ddr-a0 | 12.08 | too-big |  |  |
| designer | RP2040_base | Net-(U19-XIN) | 9.16 | too-big |  |  |
| designer | RP2040_base | /SWCLK_2 | 53.62 | long |  |  |
| designer | RP2040_base | /AUX3_CSn | 2.81 | timeout | too-big | h2 stages (nets free: answer) 0: separated, 2: separated, 5: timeout, 10: timeout |
| designer | RP2040_base | /QSPI_SS_2 | 5.91 | sat-lattice |  | h2: 0 other nets free |
| designer | RP2040_base | /QSPI_SCLK | 9.05 | timeout | too-big | h2 stages (nets free: answer) 0: separated, 2: separated, 5: separated, 10: timeout |
| designer | RP2040_base | M1A_PWM_L\ | 16.34 | too-big |  |  |
| designer | RP2040_base | +3.3VA | 3.32 | timeout | too-big | h2 stages (nets free: answer) 0: separated, 2: separated, 5: separated, 10: separated |
| designer | eurorack-pmod-pcb | Net-(D18-A) | 15.08 | walled-pads | long |  |

Classes per half-size: `h = 4` is empty where h = 2 settled the item
(SAT, zone, long, too big, error) and "(implied)" where a solution at 2 mm
is one at 4 mm too. "Nets free" counts the other nets rerouted beside the
item's at the stage that answered. Stages: the target alone, then the 2,
5 and 10 nearest nets free, then all (separated: a requirement no static
path joins, typically the item's net walled by the nets kept).

## Reproducing

```
cargo build --release   # census-kicad-window is a research command
python3 experiments/window-census/items.py items.json
python3 experiments/window-census/run.py items.json census --half 2 --jobs 5
python3 experiments/window-census/run.py items.json census4 --half 4 --unsolved-in census/results.jsonl
python3 experiments/window-census/apply.py items.json census/results.jsonl apply --half 2
python3 experiments/window-census/summarize.py items.json census census4 --markdown table.md
```

`PCB_MAKER` names the binary (default the copy under
`build/scratch/fund-window/bin`). Every subprocess is capped at 8 GB
(a census of a long window once took 17 GB before the size cap) and a job
waits for 16 GB available and outside 02:00 to 07:00.
