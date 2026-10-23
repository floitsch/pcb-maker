# Bus bundles (2026-10-23)

Florian's idea 4 (docs/experiments/human-level-routing.md): "try to
group traces where the nets are similar (a bus shouldn't need to layout
all traces separately, unless there are issues). Break them up at the
end or when it's not possible to route them this way." The failure data
behind it: buses hold 80 % of video's open terminals on our placement
and 64 to 75 % of MegaDrive's; open bus nets are open whole, routed
last and starved. This is a bounded phase 1 behind a switch
(`bus_bundles`, `PCB_ROUTER_BUS_BUNDLES`, off), with a go or no-go.
Branch `bus-bundles` on main 81356b8 (the trouble repair on by default);
snapshots x1670 (the measured version), x1671 (with two ablations),
x1672 (the patience as a variable).

## Method

`crates/pcb-router/src/router/bundles.rs`, hooked into the driver (once
per router, before the routing order), the routing order, `route_net`
and the lattice search.

1. **Detection.** Signal nets (not pour nets) of one rule class with 2
   to 12 pads that join exactly the same set of at least two footprints
   (the footprint is the terminal label's reference), four or more of
   them, form a bus; its members are ordered by their pad on the first
   footprint along that footprint's longer extent. Video's designer
   board: 16 buses, 195 nets (the PCI connector to the bridge, 51; the
   TV colour bus, 24; four TVRAM byte lanes of 8 over nine footprints;
   MXA, DQ); MegaDrive 13 buses, 89 nets (VA, VD, ZA, the 68000's
   control lines). SNSP-CPU-01's D and PA nets join varying footprint
   sets and mostly do not group (as the failure data found).
2. **One corridor per bus** (the tile planner's equivalent of a fat
   pseudo-net): from the footprint with most member pads, a tree on the
   16 x 16-node tiles that reaches each other footprint's member pad
   tiles in turn (Dijkstra from the tree so far); a tile costs
   `1 + 12 f^3` per its size, `f` the bus's width (members x (track +
   clearance) across one tile) over the room left in the tile over all
   layers, so the tree avoids tiles the bus does not fit through; grown
   by half the bus's width plus four tiles (`PCB_ROUTER_BUS_MARGIN`):
   thinner corridors pass where the lattice does not (video's designer
   placement at 150 s: 188 of 195 members found no path in their
   corridor at 0 extra tiles, 68 at 2, 52 at 3, none at 4).
3. **Reservation by order.** The members route first, right after the
   pour nets, bus by bus (the widest first), each in its pad order: in
   the negotiation's first iteration the bus's copper is down before any
   other signal net searches, which then pays the present price to take
   that room (the tile planner's fill already counts it). No separate
   capacity bookkeeping.
4. **Side by side.** A member's searches run inside its bus's corridor
   with a lane: the cells (per layer) within track pitch plus 1.5
   lattice pitches of the copper of its two neighbours on either side in
   the order. A trace step off the lane costs 1.5 times
   (`bus_lane_cost`, `PCB_ROUTER_BUS_LANE`); on it the usual cost. The
   factor is never below 1, so the A* bound stays admissible; bends are
   shared because the cheapest path follows the neighbour's.
5. **Break-up.** A member whose search finds no path inside the
   corridor is broken out for the rest of the negotiation (ordinary
   corridors, no lane); so is a member rerouted 8 times
   (`PCB_ROUTER_BUS_PATIENCE`); the hard repair, the cleanup, the via
   reduction and the straightening always route as before. (The trouble
   repair's own search of an open net is not hard: a member not broken
   out tries its corridor and lane there first, falling back as above.)

With the switch off nothing changes: `bus_of` is empty, every net ranks
`(1, 0, 0)` in the order, `bundled` returns none, the lane stays
inactive (one predicted branch per search step).

Ablations (x1671, x1672; each with the switch on): `PCB_ROUTER_BUS_LANE=1`
(no lane: corridor and order), `PCB_ROUTER_BUS_NO_CORRIDOR` (the lane in
each member's own corridors, and the order), `PCB_ROUTER_BUS_NO_ORDER`
(corridor and lane, the usual order), `PCB_ROUTER_BUS_PATIENCE=2` (the
bundle for the first two iterations only).

## Measurements

Route mode, one search thread a board, the pinned work rates, the
ledger's admission beside four other agents' runs, `--quality` for the
bends. Before: the switch off (x1660, main; x1673 off where x1660 had
been pruned, the two byte-identical below); after: the switch on (x1670;
x1673 for video's designer route, which reproduces x1670 row for row on
the expansion tier). Each cell: KiCad's unconnected items (open
terminals in the router's report) / new strict DRC errors / vias /
length / bends per connection / work seconds. Logs
`build/bus1-{designer2,designer3,v19t,expansion,fp,why}-*.log`.

**The designer's placement** (`--tier designer`, 1500 s of work):

| Board | off | on |
| --- | --- | --- |
| video | VIDEO_OFF | 0 (0) / 0 / 503 / 35.02 m / 2.00 / 757 |
| SNSP-CPU-01 | 4 (4) / 0 / 1339 / 24.90 m / 3.71 / 1108 | 3 (3) / 0 / 1407 / 24.76 m / 3.71 / 1101 |
| MegaDrive | 59 (25) / 0 / 1351 / 33.47 m / 4.62 / 1433 | 70 (41) / 0 / 1204 / 31.65 m / 4.15 / 1001 |

**The v19 placements** (`--route-from build/github-layout-v19`, the
route-mode budget):

| Board | off | on |
| --- | --- | --- |
| video | 157 / 0 / 1172 / 38.92 m / 5.96 / 1261 | 150 / 0 / 1102 / 38.11 m / 5.31 / 1120 |
| SNSP-CPU-01 | 128 / 0 / 1314 / 22.40 m / 4.04 / 958 | 135 / 0 / 1421 / 23.00 m / 4.32 / 920 |
| MegaDrive | 184 / 0 / 1367 / 32.52 m / 5.64 / 958 | 199 / 0 / 1284 / 32.73 m / 5.34 / 1031 |
| sum | 469 | 484 (+3 %) |

**Where the open terminals are** (the router's report per net; bus
nets by the same footprint-set rule, from the board file): open
terminals in all / on bus nets / bus nets with any open.

| Board, placement | off | on |
| --- | --- | --- |
| video, v19 | 221 / 155 / 48 | 181 / 73 / 34 |
| SNSP-CPU-01, v19 | 134 / 24 / 13 | 150 / 17 / 11 |
| MegaDrive, v19 | 225 / 131 / 38 | 237 / 45 / 26 |
| SNSP-CPU-01, designer | 7 / 0 / 0 | 8 / 0 / 0 |
| MegaDrive, designer | 79 / 66 / 18 | 81 / 40 / 17 |

The bundles do what they are for: the buses' open terminals fall by half
to two thirds (video 155 to 73, MegaDrive 131 to 45 and 66 to 40) and
the bends per connection drop (video 5.96 to 5.31, MegaDrive 5.64 to
5.34 and 4.62 to 4.15), vias fall on video and MegaDrive (-6 to -11 %).
But the rest of the board pays more than the buses gain: off the buses
the open terminals rise from 66 to 108 (video), 94 to 192 (MegaDrive
v19) and 13 to 41 (MegaDrive designer). Only video's v19 placement nets
a gain (-7 KiCad items).

**Expansion tier** (46 boards, 8 x 1, 30 s, the designer's placement;
before x1660, the switch off):

| Variant | strict passes | KiCad unconnected | new errors | vias | length | work s |
| --- | --- | --- | --- | --- | --- | --- |
| off (x1660; x1670 and x1673 off identical) | 19 | 625 | 3 | 4223 | 112.3 m | 1249 |
| on (x1670 = x1673) | 21 | 721 | 3 | 4166 | 110.2 m | 1222 |
| on, no lane (`BUS_LANE=1`) | 18 | 804 | 4 | 4019 | 108.8 m | 1238 |
| on, own corridors (`BUS_NO_CORRIDOR`) | 18 | 660 | 2 | 4219 | 111.8 m | 1241 |
| on, usual order (`BUS_NO_ORDER`) | 18 | 794 | 2 | 4093 | 107.5 m | 1260 |
| on, patience 2 (`BUS_PATIENCE=2`) | 19 | 641 | 6 | 4224 | 111.4 m | 1259 |

The switch on gains two strict passes (musicbrain-busboard, CompassTag)
and loses none, but leaves 96 more unconnected: OpenDeck 9 to 68 (12
buses of 8 LED and button lines), MSign mainboard 19 to 62, rpi_cm5 3 to
13; better OpenRX-Mono 34 to 10, facade60 24 to 22. Every ablation is
worse than the full version or than off: the lane is worth 83 items
(804 against 721), the order 73; the shared corridor costs on OpenDeck
(68 against 13 in the members' own corridors) and gains on OpenRX. On
OpenDeck the bundled searches take twice the expansions of an
iteration (iteration 6 at 53M against 27M) for the same conflicts (130
against 128), so at 30 s the negotiation stops four iterations earlier.

**Fingerprints, the switch off:** Castor, k30-SBC, jetson and Sisu
(route mode on the v16 placements at a quarter of the budget,
`PCB_ROUTER_FINGERPRINT=1`): every fingerprint line and the routed
board byte-identical, x1660 against x1670. The expansion tier with the
switch off: all 46 rows identical (x1660 against x1670).

**What broke the bundles.** Few members leave their corridor: with the
four extra tiles, none of video's 195 members at a quarter of the
budget on the v19 placement, 11 of MegaDrive's 89 (the 68000 to Z80
control lines, U4 to U6: their ordinary paths run 40 % outside the
corridor); the rest stay bundled until the patience
(8 reroutes) frees them. The bundles do not break; they win. What
breaks is the rest of the board: routed first, side by side, inside a
corridor planned on an empty board, the buses take the room the short
nets used (the same trade the trouble repair makes after the fact, but
without its test that the trade closes more than it opens); our
reading, not measured apart, is that the lane also makes the members'
own detours dearer, so the negotiation's prices move the short nets
rather than the bus. The work rises too: the lane factor makes the A*
bound weaker inside the corridor (MegaDrive's designer route spent 1052
s on its first attempt against 729 and ran two attempts instead of
three). Whether a placement that lines the buses up changes the trade was not
measured: on the designer's placements, where they are lined up, the
trade is no better (MegaDrive 59 to 70; video complete either way).

## Verdict

**No-go** for phase 1 as built; the switch stays, off, with its
ablations, for the next attempt.

- The bus boards do not improve: KiCad's unconnected items on the v19
  placements 469 to 484 (video 157 to 150, SNSP-CPU-01 128 to 135,
  MegaDrive 184 to 199), on the designer's MegaDrive 59 to 70,
  SNSP-CPU-01 4 to 3, video complete on both sides. No new DRC errors.
- The expansion tier gets worse: 625 to 721 unconnected, although two
  strict passes are gained and none lost.
- What works: the bundles themselves. Bus nets' open terminals fall
  by half to two thirds, bends per connection and vias drop where buses
  are, the corridors hold (few members break out), the switch off is
  byte-identical.
- What broke it: not the escapes and not vias. The buses take the room
  first, and the short nets lose more than the buses win; the bundled
  search costs up to twice the work per iteration, which at short
  budgets ends the negotiation early (OpenDeck) and on MegaDrive's
  designer route costs the ladder its third attempt. The placement
  lining the bus up is not the limit here: the designer's placement of
  MegaDrive, which lines its buses up, loses too.

What a phase 2 would need, in the order the data asks: (1) a trade
test for the bundle, as the trouble repair has: keep a bus's
bundled routing only where the board's open terminals fall, else
negotiate its members as before (the patience-2 variant, which lets go
after two iterations, is closest to off: 641 against 625); (2) the
corridor's capacity reserved against the bus's own width only where
the global route says the region has spare capacity, not wherever the
bus fits on an empty board; (3) the lane bias without weakening the A*
bound (a discount on the lane with a matching heuristic scale, or the
lane as the first corridor and the ordinary search as the fallback).
Each of these is days of work; none is promising enough from this data
to take before the placement half of the gap (idea 3).
