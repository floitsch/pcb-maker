# Why boards fail (analysis 2026-09-25)

This document goes through every corpus board that does not finish clean and
works out *why*. Fixing is left to later work. Each finding gives the
evidence, how certain it is, and what a fix has to achieve. The work list at
the end is ordered by how many boards a fix unblocks per unit of work.

| Board | Result (route mode unless noted) | Cause |
| --- | --- | --- |
| tinytapeout-4l | 78/108, 42 unconnected | 55 of the RP2040's 56 pads are unreachable because clearance overrides are resolved differently from KiCad ([F2](#f2-local-clearance-overrides-are-resolved-more-strictly-than-kicad-does)) |
| video-4l | 362/371, 45 unconnected | signals on the plane layers break GND and +5V apart ([F1](#f1-plane-layers-are-used-as-signal-layers)); 8 signal nets never converge ([F4](#f4-negotiation-stalls-in-saturated-escape-regions)) |
| coldfire-4l | 208/209, 11 GND pads | GND pour on B.Cu and In1 breaks into 200-330 pieces ([F5](#f5-pour-continuity-is-repaired-afterwards-not-planned)) |
| fence-v4-2l, fence-esp32-v4-2l | 91/101, 82/108 | a crossing structure the netlist forces, a saturated LQFP-48 escape ring ([F4](#f4-negotiation-stalls-in-saturated-escape-regions)) and a GND pour on the layer the signals need ([F5](#f5-pour-continuity-is-repaired-afterwards-not-planned)) |
| fence-keepout | 96/107, 11 `items_not_allowed` | zones inside footprints are ignored ([F3](#f3-keepouts-inside-footprints-are-ignored)); otherwise as the 2-layer fence |
| fence v4 4-layer without its workaround | tracks on the GND plane | layer type `power` ignored ([F1](#f1-plane-layers-are-used-as-signal-layers)) |
| ngdevkit | 153-160/180 | BGA escape region saturated, big pours broken apart ([F4](#f4-negotiation-stalls-in-saturated-escape-regions), [F5](#f5-pour-continuity-is-repaired-afterwards-not-planned)); diagnosed 2026-09-22/23, see [benchmarks.md](benchmarks.md) |
| stickhub | 44/45 (one GND pad) | dense two-sided board; the cold board completes on the 0.075 mm rung, which the time-based ladder may or may not reach ([F6](#f6-the-attempt-ladder-depends-on-machine-load)) |
| stickhub, openair, tinytapeout (place + route) | placement not legal | the placer's body model rules out the designer's own placement ([F7](#f7-the-placers-body-model-rejects-real-placements)) |

Measured today with the current `main` binary. The corpus runs are in
`build/diag-*` (one board per run). Scratch experiments (a rule area added
to a copy of the stripped source, debug runs with `PCB_ROUTER_DEBUG=1`)
are described inline; their files were not kept.
The layout column of the fence boards was not rerun.

## F1. Plane layers are used as signal layers

**Evidence.** Track segments per copper layer, the designer's board
against ours (route mode, from `build/corpus-pitch-4`):

| Board | Inner planes (pours) | Designer In1 / In2 | pcb-maker In1 / In2 |
| --- | --- | --- | --- |
| video | GND on In1, +5V on In2 | 69 / 539 | 2366 / 3080 |
| tinytapeout | GND on In1 and In2 | 1 / 3 | 546 / 326 |
| coldfire | GND on In1 (and B.Cu), +3.3V on In2 | 470 / 132 | 387 / 350 |
| openair | PGND on F, B, In2 | 185 / 46 | 112 / 39 |

On video and tinytapeout the router puts 6 to several hundred times more
copper on the plane layers than the designer did. The pours then fall apart: video's GND
into 495 pieces with 120 pads stranded, +5V into 601 pieces.

**Experiment.** Video routed with a board-level rule area that forbids
tracks (not vias) on In1:

- GND connects completely with no stitching vias. Before: 22 unconnected GND
  pads. After: none.
- +5V on In2 is still broken into 723 pieces, and the same 8 signal nets
  stay open (43 unconnected items against 45).

Forbidding In2 as well makes video unroutable: 236 nets are still in
conflict after 900 s. The designer does route on In2 (539 segments), so it
is a mixed layer, not a pure plane.

On ColdFire, forbidding In2 made things slightly worse (207/209). ColdFire's
problem is its GND pour on B.Cu and In1, layers the designer also routes on;
that is F5.

**Mechanism.**
- `LayerTable::from_pcb` (`crates/pcb-kicad/src/board_router.rs:228`) reads
  only the names of the copper layers. KiCad's layer type (`signal`,
  `power`, `mixed`, `jumper`) is ignored. This is fence issue 9.
- The only discouragement is `plane_cut_cost` (`router.rs:113`, default 3),
  a fixed multiplier on steps across covered tiles
  (`router.rs:787`, `1622`, `1969`). Congestion costs grow without bound:
  the present factor is capped at 10⁴, and history keeps growing. As soon
  as the signal layers are contested, a detour over the plane is the
  cheapest move by orders of magnitude.

**Certainty.** High for video and tinytapeout: the GND result follows
directly. Some boards type their layers carelessly (hierarchy's F.Cu is
`power` on a two-layer board), so the layer type alone cannot be the rule.

**What a fix has to do.**
- A `power`-typed layer that carries a pour gets no signal tracks. Vias are
  still allowed, with a config override.
- On other layers mostly covered by one net's pour, signal copper must be
  priced so that congestion cannot outbid it. Either multiply the plane
  cost into the present and history terms instead of leaving it beside
  them, or give each such layer a track budget.
- Better still, allow a cut only where the plane stays connected. This
  overlaps with F5.
- Pass criterion: video's GND needs no stitching, and the fence v4 4-layer
  board routes on 4 layers without its `GND plane: no tracks` rule area.

## F2. Local clearance overrides are resolved more strictly than KiCad does

**Evidence.** Tinytapeout routed with `PCB_ROUTER_DEBUG=1`:
- 56 pads are reported `dead`: 55 of the 56 pads of U6 (the RP2040,
  QFN-56 with 0.4 mm pitch) and one USB-C pad.
- Every net that touches the RP2040 is permanently open, among them +3V3,
  DVDD, the QSPI bus, the USB pair and all `in*`, `out*` and `uio*` nets.
  That accounts for most of the 42 unconnected items.
- The negotiation then sits at 34-35 conflicted nets for 40 iterations. Its
  result does not change when the planes are made track-free.

**Mechanism.**
- U6's footprint has `(clearance 0.18)`. Its pads are 0.2 mm wide with a
  0.2 mm gap. The GND pad next to them is in net class `pwr` (0.23 mm).
- `Board::copper_clearance` (`crates/pcb-router/src/board.rs:111-116`) takes
  `max(track class, pad override, pad's net class)` = 0.23 mm. Even the
  0.16 mm neck-down leaves only 0.22 mm to the neighbouring pad, so no
  escape stub fits.
- KiCad lets a local override take precedence over the net class; only the
  board minimum still applies. The designer's own board has 0.2 mm tracks
  between exactly these pads. KiCad's DRC reports none of its 8 clearance
  errors there.
- The adapter's `CopperObstacle::clearance`
  (`crates/pcb-kicad/src/local_clearance.rs:28`) has the same `max`.

**Certainty.** High that this is why the pads are dead. Medium on KiCad's
exact rule when both items have overrides. Confirm it against
`pcbnew/drc/drc_engine.cpp` (clearance constraint, local overrides) before
changing the core.

**What a fix has to do.**
- Resolve a pad's clearance as its local override when it has one, else its
  net class. The pair clearance is then the maximum of the two items'
  clearances and the board minimum.
- Pass criterion: no dead pads on tinytapeout. Its remaining result then
  shows the board's actual congestion.

## F3. Keepouts inside footprints are ignored

**Evidence.** `fence-keepout` (ESP32-C3-MINI-1 without the board-level copy
of its antenna keepout): 11 `items_not_allowed` in KiCad's DRC. pcb-maker's
own verifier reports nothing.

**Mechanism.**
- In the router adapter, the footprint branch (`board_router.rs:375-513`)
  lowers only pads. Rule areas are read only at the top level
  (`board_router.rs:581`).
- The placer does the same: `board_placer.rs:363-368` scans only top-level
  zones for footprint keepouts.
- In the board file, footprint zones carry absolute board coordinates.

**Certainty.** Certain (fence issue 1).

**What a fix has to do.**
- Lower footprint zones with `keepout` like board rule areas: tracks, vias,
  pads and pours per their flags, on their layers.
- In the placer, a footprint's own keepout is part of that footprint. It
  moves with the footprint, so it is not a fixed obstacle, but other parts
  may not be placed inside it.
- The internal verifier should check keepouts, so that the mismatch shows
  up before KiCad runs.

## F4. Negotiation stalls in saturated escape regions

This is the failure behind the fence 2-layer boards, ngdevkit, and the
signal nets that remain open on video.

**The pattern.** On every one of these boards, the conflicted count falls
until the present factor reaches its cap at iteration 25. After that it
stays flat for the remaining 50 iterations:

| Board | Conflicted nets after the cap |
| --- | --- |
| fence-v4-2l | 40-57 |
| fence-esp32-v4-2l | 63-83 |
| tinytapeout | 34 (dead pads, F2) |
| ngdevkit | 50-90 |
| video | 8-20 |

The router then keeps the partial result. It never notices that nothing
changes any more.

**Where it happens (fence-v4-2l).**
- With `PCB_ROUTER_DEBUG=1` and a tracks-only config, the hottest tiles are
  all on F.Cu in the pad ring of U1 (CH32X035, LQFP-48 with 0.5 mm pitch):
  (88, 64), (88, 66), (98, 66), (98, 68), (92, 71) and (92, 61), with
  history 40-95.
- The conflicted nets are almost all `MCU_ROW*` and `ROW*`.
- U1 escapes 39 GPIOs, and C1/C2 sit about a millimetre from its top row of
  pads.
- ngdevkit's hot spot has the same shape: the front side of its 0.75 mm
  PSRAM BGA.

**Why the fence is hard: a crossing structure the netlist forces.**
- Every row k has two nets. `ROWk` runs from J1 through a resistor-network
  element to an X port of U4 or U5. `MCU_ROWk` runs from the other side of
  that element to a GPIO of U1.
- On the ratsnest (minimum spanning tree edges between pad centres):
  - `MCU_ROW` edges cross each other 0 times; the hand-made order is already
    crossing-free.
  - `ROW` edges cross each other 12 times.
  - `MCU_ROW` edges cross `ROW` edges **210** times.
- 210 is exactly 2 × 15·14/2. In each half, the two families leave the
  network row interleaved and head for different chips. No pin assignment
  changes that, so on two layers the families must be split by layer.
- J1 is through-hole, so a `ROW` track can start on B.Cu at no cost. One
  plan is all `ROW` tracks on B.Cu and all `MCU_ROW` tracks on F.Cu, with one
  via per row at the switch. That is a global decision per net family.
  Negotiated congestion, which routes one net at a time against local
  costs, does not find it.
- Using B.Cu that way also leaves nothing of the GND pour, which is why
  `pours=connect` does worst on this board (F5).

**Is the board routable at all?** A rough cut analysis says capacity
suffices:
- Vertical cuts need at most 41 ratsnest crossings over 27 mm of height,
  about 67 tracks per layer.
- The tightest channel is the 2.5 mm between U4 and the network row: about
  6 tracks per layer.

So the limit is where the connections go and in what order, not the total
room.

**Freerouting gets further.** Freerouting 2.2.4 on the same cold board
(tracks, vias and pours stripped; `route-kicad-board-freerouting`, Java 27,
optimizer off):

| Router | Unconnected items | Vias | Copper | Time |
| --- | --- | --- | --- | --- |
| Freerouting | 8 | 170 | 3256 mm | 940 s |
| pcb-maker, same cold board | 40 | 213 | 2886 mm | 169 s |
| pcb-maker, GND pour kept (normal route mode) | 14 | 218 | 3066 mm | 103 s |

Freerouting's passes hover between 8 and 13 unrouted connections from pass
27 on. The board may be just barely routable on two layers. Either way, on
this board a mature rip-up and push router beats our negotiation clearly.
F4 is a router weakness, not only a hard board.

**What a fix has to do.** Four pieces, in order of expected value:
1. **Escape (fanout) planning for fine-pitch parts** (QFP/QFN ≤ 0.5 mm,
   BGA):
   - Before negotiation, give each pin an escape direction, and a via site
     where it needs one.
   - Order the escapes around the part to match where the nets go, as the
     tangential order of their destinations.
   - Reserve the escapes like the plane skeleton reserves its tree.
   - This is the step commercial routers run before anything else.
2. **A layer prior from the crossing graph:**
   - Two ratsnest edges that cross want different layers. A max-cut of the
     crossing graph (greedy or spectral is enough) gives each connection a
     preferred layer.
   - The corridor planner already has per-layer tiles and can take the
     prior as a bias.
   - On the fence this would put the `ROW` family on one layer and
     `MCU_ROW` on the other.
3. **Stall handling:**
   - Detect the plateau (no progress for about 5 iterations at the cap)
     instead of running to 80.
   - Then act on the region instead of on nets: rip up everything in the
     hot tiles, conflicted or not, and reroute the region with a different
     net order or with the tiles' capacity lowered for the global plan.
4. **Congestion estimate for the layout loop:**
   - The cut and escape numbers above can be computed from the ratsnest and
     the tile graph in milliseconds.
   - The coupled loop could then move U1's neighbours away, or tell the user
     "this placement cannot route on two layers", before spending ten
     minutes negotiating.

## F5. Pour continuity is repaired afterwards, not planned

**Evidence.**
- fence-v4-2l, per ladder rung: pours connected 92 open, with plane
  skeleton 60, pour nets as tracks 28 (then 14 unconnected items).
- fence-esp32-v4-2l: 153, 114 and 205 open.
- ColdFire's GND (B.Cu + In1) is in 203 pieces in route mode and 326 in the
  experiment above. 22-38 GND pads stay stranded after stitching.
- ngdevkit's GND is in 1500 pieces.
- On video, the only thing that kept GND whole was keeping signals off its
  layer entirely (F1).

**Mechanism.**
- Signals are routed first. The pour is then labelled on the lattice
  (`analyze_pours`), and islands are stitched or their pads routed as
  tracks.
- Once a layer is cut into hundreds of islands, stitching cannot repair it.
- The skeleton rung reserves a tree early, but only as a fallback and only
  with a fixed bias.

**What a fix has to do.**
- Treat plane connectivity as a constraint during negotiation, not as a
  repair.
- One concrete form: in the tile graph, keep a spanning structure of each
  pour's tiles. A signal may cross a pour tile only if the pour stays
  connected around it, the same test as F1's "allow a cut only where the
  plane stays connected".
- A cheaper first step: count the pour's pieces per iteration and charge
  the nets that cut it through history.

## F6. The attempt ladder depends on machine load

**Evidence.** fence-v4-2l, same binary, same input, same result (91/101,
218 vias, deterministic):
- Alone, the tracks attempt routes in 75.6 s.
- Next to four other routing processes, it takes 377.6 s.
- The ladder skips finer rungs when their projected time exceeds 240 s,
  and stops after any attempt over 480 s (`board_router.rs:993-1085`). So
  under load the board never gets its 0.075 mm rung. Alone it would have
  (projected 134 s).
- ngdevkit's completion varies between 153 and 160 for the same reason.
- The corpus runner runs boards in parallel, so the published numbers
  depend on which boards happen to run at the same time.

**What a fix has to do.** Budget attempts in deterministic units, such as
search expansions, which `board-router.json` already reports. Or at least
measure thread CPU time, not wall time.

## F7. The placer's body model rejects real placements

On all three boards, the placer's own model says the designer's placement
is illegal. On openair and tinytapeout it also leaves no legal position at
all for the part that ends up unplaced. With three small model changes,
tried in a scratch copy, all three boards placed with no unplaced or
illegal parts in 13-27 s. The legalizer is not the problem.

The five causes, from most to least important:
1. **Through-hole parts block their whole body on both sides**
   (`board_placer.rs:300,312`; `Side::Both`).
   - openair's 18650 holder BAT1 is an SMD part with one locating hole.
     Without a courtyard, its pad box (87.5 × 49 mm, 64 % of the board)
     blocks the back side, where the designer placed 154 parts including
     the ESP32 module.
   - Fix: on the far side, occupy only the holes and through-hole pads,
     plus clearance.
2. **Footprints with no pads and no courtyard become fixed 1 × 1 mm walls**
   (`board_placer.rs:197` with the no-net rule at `:556`).
   - The logos on stickhub block every 14.6 mm gap that U1 would need.
   - tinytapeout's net-less placeholder pads (J16, J18) wall off J4.
   - Fix: skip graphics-only footprints. Let net-less placeholder pads move
     with their host.
3. **The body is one box around all courtyard pieces**
   (`local_body`, `board_placer.rs:116-205`).
   - tinytapeout's J4 has two courtyard strips 24 mm apart. The designer
     uses the gap.
   - Fix: one body per courtyard piece, or courtyard polygons.
4. **Rotated parts become axis-aligned boxes** (`legal.rs:17-49`,
   `problem.rs:99`).
   - stickhub's back side sits at ±45°, and U1 counts as 14.6 mm instead of
     10.3 mm.
   - Fix: oriented rectangles (a separating-axis test).
5. **stickhub's designer overlaps courtyards on purpose.** Its project sets
   `courtyards_overlap: ignore`.
   - Fix: honour that setting by using pad boxes plus clearance as bodies.

Side issues:
- `fit_halos` adds front and back area together.
- tinytapeout's 130 stitching vias are movable "parts".
- `lower_placement` pushes keepout pseudo-components without matching
  `references`, `source_at` and `locked` entries (`board_placer.rs:~407-437`).
  The report names are then shifted, and `names()` can panic on a board
  with footprint keepouts.

## Also found

- **No failure report.** Dead pads, hot tiles, conflicted nets and pour
  pieces appear only on stderr under `PCB_ROUTER_DEBUG`. Every diagnosis
  above needed a rerun. `board-router.json` should carry them: dead pads
  with the reason, the ten hottest tiles, the final conflicted set, pour
  pieces per net.
- **The Freerouting setup broke with a Java upgrade.**
  `benchmarks/corpus/freerouting.json` named `java-26-openjdk`, which no
  longer exists. The jar needs Java 25 or later (class file version 69),
  while `/usr/bin/java` is Java 21.
  - Fixed today by pointing the config at `java-27-openjdk`.
  - The adapter should rather find a suitable `java` itself.
- **fence issue 5** (overlapping fixed parts are not reported) and the
  constraint language (issues 2 and 11) are still open and unchanged. They
  are not failures of the kind analysed here.

## Work list

Ordered by expected gain per effort. Each item names its pass criterion.

1. **Clearance overrides like KiCad (F2).** A small change in
   `copper_clearance` and `local_clearance.rs`; confirm the rule first.
   Pass: tinytapeout has no dead pads.
2. **Footprint keepouts (F3).** Router, placer and verifier. Pass:
   `fence-keepout` has no `items_not_allowed`.
3. **Placer body model (F7).** Items 1-3 of F7 made all three boards
   placeable in the experiment. Then oriented rectangles. Pass: stickhub,
   openair and tinytapeout place legally, and their layouts route.
4. **Plane layers (F1).** Honour `power`; price plane cuts so congestion
   cannot outbid them. Pass: video's GND unstitched; fence v4 4-layer clean
   without its rule area; tinytapeout's inner layers nearly track-free.
5. **Deterministic ladder budgets (F6).** Pass: the same board gives the
   same attempts with and without parallel load.
6. **Failure report in `board-router.json`** (Also found). Makes every
   later diagnosis a file read.
7. **Escape planning for fine-pitch parts (F4.1).** Pass: fence-v4-2l's U1
   ring no longer the hot spot; ngdevkit's BGA region converges.
8. **Layer prior from the crossing graph (F4.2)** and **stall handling
   (F4.3).** Pass: fence-v4-2l completes, or a Freerouting reference shows
   it cannot.
9. **Pour continuity as a constraint (F5).** Pass: ColdFire complete; the
   2-layer fence keeps a GND pour in one piece, or reports that it
   cannot.
10. **Congestion estimate in the layout loop (F4.4).**

Pin swapping (fence issue 3) is being built alongside this analysis. Its
first results are in [Pin swapping](#pin-swapping-first-results).

## Open questions

- **Is the 2-layer fence routable at all?** Freerouting stops at 8
  unconnected items (see F4), so probably only just, if at all.
- **Does fixing F2 leave tinytapeout converging?** 34 conflicted nets is
  what the dead pads produce. Whether a congestion core remains is unknown.

## Pin swapping (first results)

Implemented: `swap-kicad-pins`, and `pin_swaps` in the route and layout
configs. See [pin-swap.md](pin-swap.md). Board and schematic stay in
agreement: KiCad reports no parity issue and no ERC finding on either fence
board after the swap.

On the 2-layer fence, the swapped assignments cut ratsnest crossings by
15-30 % and mostly save vias or copper. They finish 81-88 nets against 91
for the generator's hand-made assignment, which was already crossing-free
where it matters. Whatever the pin assignment, the router's completion on
this board is set by the F4 stall. Two things would change that:
- a router-based judge of assignments, for example the tile graph's
  congestion;
- fixing F4.
