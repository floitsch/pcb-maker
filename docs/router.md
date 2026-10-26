# The whole-board router (`pcb-router`)

`pcb-router` is the project's routing core. It replaces the per-net,
file-to-file sequential router described in the
[trust audit](reviews/2026-09-21-trust-audit.md).

```sh
cargo run --release -- route-kicad-board \
  <source-directory> <board-id> <output-directory> [config.json|auto]
```

Every key of `config.json` with its type, default and doc comment is in
the generated [configuration reference](config-reference.md).

The command parses the board once, routes every connection in memory, checks
the result with an exact internal verifier, writes the project once, and runs
native KiCad verification once as the final gate. It exits non-zero when the
internal verifier or KiCad finds anything.

A config file takes its rules from the project too: the project's rules
are the base, and each key the file sets replaces the project's value of
that key (`edge_clearance_mm`, `hole_clearance_mm`,
`solder_mask_to_copper_mm`, `solder_mask_bridge_severity`, ...), so
`{"seeds": 4}` routes by exactly the rules of `auto`. `connection_rules`
and `default_rules` together are the classes: a file setting either
replaces the project's classes (the project gives every net its entry);
the custom rules' per-net floors and pair clearances stay
(`project_rules::router_config_over_project`; until 2026-10-21 a file
dropped the project's mask rules, and one with classes every other
project rule). `{"frame_directory": "<dir>"}` additionally
writes the board after every negotiation iteration as
`<dir>/attempt-NN/frame-NNNN.kicad_pcb`; `docs/images/animate.py routing`
turns those frames into a GIF (the README's routing animation is
Interf-U, cold, 81 iterations).

## Results (2026-09-21, one thread, cold boards, zero initial copper)

| Board | Nets | `pcb-router` | Old internal router | Freerouting 2.2.4 |
| --- | ---: | --- | --- | --- |
| ECC83 | 9 | 9/9, **0.2 s**, 1 via, 344 mm | 9/9, 104 s, 8 vias, 388 mm | complete |
| Complex hierarchy | 50 | 50/50, **1.2 s**, 0 vias, 1330 mm | 50/50, 487 s, 10 vias | 49/50 (one open), 8.5 s routing |
| PIC programmer | 34 | 34/34, **2.9 s**, 1 via, 1907 mm | 34/34, 308–1272 s, 39–53 vias | 33/34 (one open) |
| Interf-U | 110 | 110/110, **22.5 s**, 58 vias, 4789 mm | 94/110 at the 1800 s cap | 110/110, 49 s routing, 44 vias, 5051 mm |

Times are routing only; the single native KiCad gate adds about 6 s per
board. Every `pcb-router` result above passes native ERC/DRC/parity with zero
findings and zero internal violations. The Freerouting hierarchy and PIC
figures are the ones retained by the trust audit of 2026-09-21; ECC83 and
Interf-U were re-run during the audit.

![Interf-U routed by pcb-router](reviews/2026-09-21-trust-audit/interf-pcb-router.png)

## How it works

The code (`crates/pcb-router/src`): `router.rs` holds the shared state
(`Router`, `NetState`, `Scratch`, the lattice nodes and branches, the
work constants) and the small helpers; `router/` has one file per part of
`impl Router`, in the order of a run: `build` (the lowering into maps),
`escapes`, `guards` (thermal guards and the spoke repairs
`free_thermal_spokes`, `free_lost_spokes`), `prepare` (a net's search state),
`stamping` (occupancy, rip-up, conflicts), `route_net`, `planner` (tile
corridors), `capacitated` (the capacitated corridors, an experiment),
`search` (the lattice A*), `update`, `batching`, `global`, `driver`
(the phases, budgets and clocks), `fixed`, `negotiate`,
`pour_refresh`, `vias` (via reduction), `finish`, `pour_analysis`, `fill`
(the fill probe), `spokes` (KiCad's spoke model), `manual_spokes`,
`pour_dump`, `stitch`, `cleanup`, `resolve` (the hard repair), `output`;
beside them `config`, `result`, `heap` (the open list), `hash`, `wall`
(wall-clock cuts), `profile`, `congestion` (tile views for the congestion
model) and `tests`.


1. **Lowering (once).** The adapter (`crates/pcb-kicad/src/board_router/`)
   turns pads, holes, rule areas, existing copper, the outline and the resolved
   per-net rules into a format-independent `pcb_router::Board`.
2. **Lattice.** A pitch and phase are chosen so that as many pad centres as
   possible fall on nodes (through-hole boards live on an imperial lattice).
   Tight channels count ten times more than pad centres: a channel between two
   neighbouring pads that the narrowest track fits through with less than
   half a pitch to spare needs a node row within that slack of its centre, or
   the pad row is a wall. (ngdevkit's 0.75 mm BGA with 0.28 mm balls and
   0.15 mm rules leaves 0.01 mm of slack; 0.125 or 0.075 mm lattices reach
   its inner balls, 0.1 mm cannot.)
3. **Static maps (once per rule class).** Per layer and node: free, blocked, or
   owned by one net (its own pads). Nodes within one chord sagitta of an
   obstacle get exact per-edge checks, so there is no blanket safety margin
   eating narrow channels.
4. **Occupancy.** Routed copper is stamped into per-class occupancy maps as the
   zone where another net's centreline or via may not be. Stamps are reference
   counted per net, so rip-up is an exact decrement; a restamp builds the
   net's new list first and changes only the counts of the stamps that
   came or went (a rerouted branch, not the branches kept); a node one step
   from the node before on its layer and in its class marks only its
   footprints' leading edge (`Fresh`: the offsets the node before did not
   cover), which lists the same cells in the same order. They are exact: a node's
   disc covers the cells closer than the clearance (in whole nanometres), so
   two tracks at exactly their clearance are legal, as KiCad has it (0.4 mm
   pad pitch with 0.2 mm tracks and clearance). On a lattice no node lies
   closer to an axis step than to its ends; a diagonal step passes the nodes
   on its bisector closer, so it stamps those too, and every node stamps,
   into a second family of maps, the start cells of the diagonal steps that
   would pass it too closely (one lookup per diagonal move in the search).
   The distance between two segments of a lattice is always attained at a
   node of one of them, so these two stamps together are exact. What stays
   quantised is the spacing of parallel diagonals: multiples of pitch/√2,
   so on a 0.127 mm lattice three 0.25 mm tracks with 0.2 mm clearance fit
   an axis channel of 1.15 mm but not a diagonal one (5 steps are 0.449 mm).
5. **Negotiated congestion.** Every net is routed allowing overlaps at a cost.
   Contested nodes get more expensive from iteration to iteration (present
   factor and history). Only nets still in conflict are touched, and of those
   only the conflicting branches; the remaining tree components are
   reconnected. A net in conflict for eight iterations running is ripped up
   whole every eighth: the branches it kept may be what boxes the other net
   in (Interf-U's edge fingers, where a kept branch fenced the neighbour's
   finger on the only layer that reaches it). Branches a rip-up keeps stay
   stamped while the net is searched again; the search masks its own stamps
   (`mask_own`, an epoch that never repeats: the old Jacobi mask reused
   generation 1 for every net, so a previous net's stamps passed for the
   current one's). Fixed branches (plane skeletons, escape stubs) therefore
   hold their ground even in the hard rerouting at the end, where a bulk
   rip-up used to unstamp them and other nets crossed them.
   `escape_stub_mm` (off) fixes a straight stub outward for every pad of a
   fine-pitch row before negotiation; it did not gain on Tiny Tapeout or
   ColdFire yet. How the escape stubs stamp ("Fine-pitch escapes"
   below) decides whether a 0.4 mm row can leave at all. `fixed_plane_stubs` connects every surface pad of a pour
   net to an inner plane by a stub and via before the signals route (only
   inner planes count: on the empty board every pad touches its own
   layer's pour, which the signals then cut to pieces); the KiCad ladder
   runs it as a rung when the plain pour connection left pour-net pads
   open (Framework mainboard half: 13 open -> 3, then complete as tracks). Under `PCB_ROUTER_DEBUG` the
   conflict spots of stuck nets name the nets stamping them, and
   `PCB_ROUTER_DUMP=x0,y0,x1,y1` (mm) prints the lattice of that region per
   layer once few nets remain (statics, occupancy, the stuck nets' nodes).
6. **Search.** A* over (layer, node) with preferred layer axes, bend and via
   costs. Few-target searches use a layer-direction-aware bound; many-target
   searches (power nets, tree components) use an exact coarse octile distance
   transform.
7. **Resolution.** If negotiation stalls, the most contested nets are removed
   and reinserted with all other copper as hard obstacles; partial trees are
   kept and reported. Since 2026-10-15 a net in conflict loses only its
   conflicted branches (and those left dangling by them), not the whole
   tree, whatever the negotiation's every-eighth-iteration whole rip-up
   would say, and an incomplete net without conflicts keeps its tree; the
   hard search then joins the rest to what is left (the switch back,
   `PCB_ROUTER_OLD_RESOLVE`, was removed on 2026-10-16). Ripped up whole,
   a net searched again from one pad among copper that no longer gives
   way often found nothing: MegaDrive's cold route left 19 signal nets
   with no copper at all after the repair (ten VD, five VA, ZA0, ZA8,
   ZD4, PA6; 4 now), and vaio_re's narrow-signal attempt +3V3E with 3 of
   43 pads. Then the trouble-makers' repair (`trouble_repair`, on since
   2026-10-23; `PCB_ROUTER_TROUBLE=0` turns it off; not in a layout's
   move trials): each net still open, most open first, is routed through
   other copper at a price, the nets it overlaps lose the crossed
   branches and are routed again hard, and the exchange is kept when the
   open terminals of all of them drop (a ten-pad bus net may cost a
   short net a pad; docs/experiments/human-level-routing.md), or stay
   the same and fewer of the nets are open (a tie that gathers the open
   terminals into fewer nets; `PCB_ROUTER_TROUBLE_TIES=0` refuses ties).
   Refused: more nets displaced than the trouble-maker's limit (12, or
   twice its open terminals where that is more: video's GND with 283
   open terminals may displace 32 nets, a two-pad net 12;
   `PCB_ROUTER_TROUBLE_VICTIMS`, `_PER_OPEN`), a pour net in the way,
   fixed copper. Up to three rounds while a round keeps an exchange; a
   refused net is tried again only after an exchange was kept, all
   within half the attempt's negotiation in work. Most open terminals
   first; the surveyed order (`PCB_ROUTER_TROUBLE_ORDER=few`: each net's
   priced search alone first, the fewest victims per terminal closed
   first) was measured and not kept (docs/changelog.md, 2026-10-23).
   An exchange's victims are routed again on their corridors first,
   and a victim left with more open terminals than it had then in its
   window (not grown fourfold as the hard search's, never board-wide)
   while the exchange can still pass (`PCB_ROUTER_TROUBLE_LOCAL=0` the
   full hard search); an exchange is undone as soon as the victims
   routed so far leave it no way to pass (`PCB_ROUTER_TROUBLE_EARLY=0`
   routes them all first): a victim routed keeps its terminals, so the
   open terminals of the trouble-maker and of the victims routed so far
   bound the result from below. The victims' failing searches through
   the grown window and the board were most of the repair's work; an
   exchange costs 1.6 to 5 times less, the gap boards try about half
   as many again in the same bound, and the v19 placements' gap boards
   end at 653 unconnected instead of 702 (docs/changelog.md, "trouble
   repair: cheaper exchanges"). Corridors alone were cheaper still but
   lost MegaDrive's designer route, whose victims need the detour.
8. **Cleanup.** Each net is rerouted alone with pure geometric cost and a high
   via cost; the result is kept only if strictly cheaper.
   After the via reduction and the spoke repairs, the straightening
   (`straighten`, "Straightening" below) takes stair-steps and jogs out of the signal nets' paths.
9. **Verification.** `pcb_router::verify` measures true distances between all
   emitted copper, obstacles and the outline, independent of the lattice.

### Pads no path reaches (2026-10-15)

A net's tree grows by one multi-target search at a time and stops at the
first search that finds nothing. In negotiation only what never changes
(pads, rule areas, the outline) stops a search, so a
board-wide search that fails proves its open pads unreachable for the
whole negotiation. Since 2026-10-06 two such failures make the net keep to
its corridors (`hopeless_searches`, link's wasted board-wide searches),
and from then on a corridor that missed ended the net, open pads and all,
and left it blocked. SNSP-CPU-01's GND (199 pads, four walled in at two
connectors, J3 and P6): 195 pads in iterations 0 and 1, then 48, 30, 21
for the rest of the negotiation while the signals took the room; the hard
repair could not bring it back (178 unconnected in KiCad). The narrower
corridors of 3c1f81d (a tile more every sixth reroute, not every third)
made the miss happen there: 55 open before it, 178 since. The pads a
board-wide search did not reach are now left out of the net's later
searches in that negotiation (`NetState::unreachable`; the A/B switch
`PCB_ROUTER_OLD_HOPELESS` was removed on 2026-10-16). Route mode,
cold, 4 threads, with the repair's change above (x881 against x902, this
change on main before 670454c; KiCad's unconnected items): SNSP-CPU-01 178
to 5 (GND's 4 walled-in pads and one signal), MegaDrive 137 to 70, vaio_re
8 to 5, mackerel-30 0 to 0, DSP-ADAU1452 0 to 1 (the ladder had time for
the 0.05 mm rung and left the narrow-signal step 52 s of work instead of
142). Either change alone loses somewhere: the pad rule alone took vaio_re
to 41 (its tracks rung, 634 to 132 open, became the best and its narrow
attempt ripped +3V3E up whole: 3 of 43 pads).

**Unreachable pads join each other** (2026-10-21, `Config::apart_trees`,
`PCB_ROUTER_NO_APART_TREES`). The pads left out stayed apart from each
other too: jiran-ble-lite is a split keyboard's two halves in one file,
joined only by a frame whose mouse-bite holes no track crosses, and both
halves' pads are GND. The tree grew on the left half and the right half's
20 GND pads were each an unconnected item (KiCad 20; the designer's own
board has 1, the two halves' GND). Now, after the net's tree is done,
the terminals marked unreachable are joined among themselves: a tree of
their own from the first of them, by board-wide searches from that tree
alone (the main tree's region is not searched again), then the next one
left (`route_apart`). Their terminals stay unconnected by the net's
count, and the net stays partial: the router's open count is the same,
KiCad's falls to the one item between the trees (jiran on its v18
placement 20 to 1; on the designer's below). A walled-in pad alone costs
one search of its wall. Fingerprints of Castor, k30-SBC, jetson and Sisu
are unchanged (no such group there).

### KiCad's connectivity (2026-10-21)

In the 5 834-board intake, 192 boards (119 with nothing but unconnected
items beyond the designer's findings) were complete by the router's count
while KiCad's DRC, zones refilled, reported unconnected items: the
router's model of what KiCad connects was wrong. The causes, the fixes
and the census (`experiments/kicad-connectivity/census.py`) are in
docs/changelog.md, "KiCad connectivity agreement". What the model now
holds:

- **Terminals are KiCad's items.** Pads of one number are one terminal
  only where they overlap on a copper layer they share (an exposed pad's
  thermal vias, a pad with holes in it), and the overlap is transitive
  (2026-10-21): an exposed pad on F.Cu and its copy on B.Cu are one item
  through the thermal vias that overlap both (MSign's U1.21, Sisu's
  U2.11), a reversible diode's SMD pads on both sides one through its
  through-hole pad (urchin's and eternal-keypad's D_SMD_TH), so pads of
  one number are grouped by overlap and the largest of a group is the
  terminal. Pads of one number at one place on different layers with
  nothing between them (an edge connector's or a USB-C shell's top and
  bottom pads, an edge-mount SMA's) are two items to KiCad, which wants
  them joined by a via: two terminals. KiCad 9's footprint jumpers join
  pads without copper: `(duplicate_pad_numbers_are_jumpers yes)` the pads
  of one number, `(jumper_pad_groups ("1" "2") ...)` the pads of a
  group's numbers (kicad-cli 10.0.7: no unconnected item between them);
  such pads are one terminal too (ducktop2's bms, its WAGO terminals). A net named by a number ("14", "0") is
  a net like any other: a net form of one unquoted number ("(net 6)",
  KiCad 9 and before) is a net code, resolved through the board's net
  table (`net_codes`, `item_net`) for tracks, vias and graphics; the
  lowering used to take every name of digits for a code and left such
  nets unrouted. A copper graphic with a net that is only strokes (a line,
  an arc, a circle's ring) is an item of KiCad's connectivity like a pad:
  a terminal at the middle of its first piece, unless it lies on copper
  of a terminal of its net already.
- **The pour model connects only what KiCad's fill connects.** A solidly
  connected pad (`connect_pads yes`, `zone_connect 2`) joins a fill piece
  only where the fill reaches its copper: a free node of the piece within
  the brush's half width of the pad, or a straight run of fill (the fill
  probe, off the lattice) from a node within two pitches beyond it
  (`solid_fill_reaches`; a piece within 0.8 mm counted before). A pad set
  to no zone connection is cut out of the fill at the zone's clearance
  and half the minimum width (`isolated_knockouts`), as another net's
  pad, so only a track from it into the fill joins it; in a hatched zone
  at the pad's own clearance (its or its footprint's local clearance)
  where that is larger (2026-10-21: KiCad 10.0.7's
  buildCopperItemClearances takes the clearance between zone and pad; a
  solid zone's knockoutThermalReliefs takes the zone's own clearance or a
  `physical_clearance` rule, not the pad's). A thermal relief is the
  thermal gap alone in both (THERMAL_RELIEF_GAP_CONSTRAINT), whatever
  the pad's clearance. The pour masks
  keep a node only where the brush (half the zone's minimum width) fits
  inside the zone's outline and clear of the regions other zones take,
  at the clearance the fill probe keeps from them
  (`Board::excluded_clearance`): a column of nodes on the outline joined
  pieces KiCad's fill keeps apart. Another net's zone of the same
  priority is filled first when its uuid is the larger
  (ZONE::HigherPriority). A spoke is no wider than the pad, a custom
  pad's no wider than its anchor (KiCad caps it by the pad's size, which
  is the anchor's): a custom pad with an anchor narrower than the zone's
  minimum width gets no spoke and joins the pour only by a track.
- **The emitted copper is checked** (`verify::copper_components`): a net
  routed with tracks alone, its terminals joined by the copper of the
  net that overlaps on a shared layer (pads, the pads merged into a
  terminal, existing copper, graphics, the emitted tracks and vias; a
  custom pad through its contact only), as KiCad's connectivity without
  pours sees it. A terminal the copper leaves apart (a stub dropped for
  clearance after the status was taken) makes the net partial: a route
  complete by the router's count is one whose tracks KiCad finds
  connected. A pour net's connections stay the pour model's (fill only
  adds connections in KiCad, so the check cannot judge a pour net).

- **A via of the net in a hatched zone has a thermal relief**
  (2026-10-21). KiCad 10 treats such a via like a thermal pad where the
  zone's pads connect by thermals (knockoutThermalReliefs: a relief of
  the thermal gap, a ring, four spokes; with `connect_pads yes` or
  `thru_hole_only` it connects solidly, `Hatch::via_reliefs`). The fill
  probe keeps brush centres out of the reliefs of the net's vias, routed
  or on the board (`FillProbe::relieved_vias`), and the rings count as
  fill; the pour map clears the relief's nodes but for the via's four
  spokes along the axes (`cut_via_reliefs`). issue6284's U102.6: its
  spoke ended in the relief of a via of its own net, the model joined the
  pad to the via's piece through it and called the board complete;
  IotThermalCamera's Tgnd: a diagonal neck of the lattice past a via
  joined two pieces the relief keeps apart.

- **A net tie joins no nets** (2026-10-21). KiCad 7's
  `(net_tie_pad_groups "1, 2")` (and, on a board before version
  20220817, a footprint whose keywords start with "net tie": all its
  pads) lets the footprint's copper short the group's pads; KiCad's
  connectivity still counts each pad as an item of its own net
  (kicad-cli 10.0.7 on a NetTie-2_SMD_Pad0.5mm between nets A and B: a
  track of A to pad 1 alone leaves pad 2 and B's other pad unconnected;
  tracks of A and B to their pads, over the bridge, find nothing). So the
  pads stay ordinary terminals and the tie's nets are routed as before,
  each to its own pad. What changed is the bridge: copper of no net that
  overlapped both pads, which made every tie pad dead (neotron-pico's
  NT901 to NT912, all 24 of its open connections on both placements;
  ghoul's 6, protoesp's 10 to 13). KiCad lets the tie's own nets touch
  the bridge anywhere (an A track ending on its middle: clearance against
  pad 2 only) and keeps every other net clear of it. A lattice node has
  one owner, so the lowering keeps the bridge as copper of no net less a
  disc around each tie pad's centre of the pad's net's clearance, half
  its track width and 0.1 mm (`carve_net_tie`, cells of 0.02 mm): the
  pad's net ends its track there, other nets keep clear of the rest and
  of the pads. The cost is a sliver of bridge other nets could come
  within the clearance of (beside a tiny tie's middle, 0.12 mm on
  ghoul's 0.2 mm pads at most).

What still disagrees on the 65 boards with a licence (one): a dimension
drawn on a copper layer, which KiCad's fill keeps clear of and the
lowering does not lower (robotique's BioGenius; the lowering gaps 2
package's).

### The probe ladder (2026-10-03)

On boards with four or more layers and 120 or more connections the KiCad
adapter does not split the ladder's budget by a fixed rule (each rule tried
failed some board: docs/todo.md). Every rung (exclusive planes, connected
pours, plane stubs, pour nets as tracks) is lowered and negotiates for
`probe_seconds` (75) with `Router::probe`, which keeps its whole state; the
rung with the fewest unfinished nets (conflicted or incomplete) is resumed
with `Router::resume` for a third of what is left, then finished and
polished. A probe that finishes everything stops the probing, and only the
leading router stays in memory. The probes run side by side (as many as
fit the memory, judged by the first router's maps: "Memory limits" below), with
the outcome of probing one after the other: a probe's budget is work, the
fewest unfinished nets lead, a tie goes to the earlier rung, and only the
probes up to a rung that finishes everything count. The layout's placement
race probes its placements the same way. In route mode the probes
together take at most half of the ladder's budget (`PROBE_SHARE`: each
`probe_seconds` or half the budget over the rungs, 3.75 s on a 30 s
budget with four rungs; the intake's 4-layer boards probed 4 x 75 s on
30 s), and the continuation stops where the ladder's work bound ends ("The
work bound in route mode" below). Under a limit of the process's own that
two routers of the first one's size do not fit (the leader stays in
memory while the next probe runs), there are no probes: the rungs run one
after the other in the regular ladder. Finer pitches follow as before when
connections stay open.

### Hole clearance

Copper keeps two distances from a via: the copper clearance from its ring
and the board's hole clearance from its drill; with a thin ring the second
binds. Both enter the stamps (trace and via against via) and the static
map around pads, and the exact verifier checks copper-to-hole as well as
hole-to-hole. The same holds for a pad's plated hole: other nets' tracks
and vias keep the hole clearance from its drill, which reaches past a thin
ring (jetson's J2: 0.55 drill in 0.75 copper, clearance 0.125, hole
clearance 0.25: a via 0.769 from the centre kept the copper rule and was
0.244 from the hole). The pad's own net is exempt, as in KiCad, whose
tracks end on the drill; its vias keep the pad copper's rule
(`StaticMaps::via_owner`).

### Vias in pads (2026-10-21)

A via inside an SMD pad is a defect on a reflowed board: the open hole
takes the pad's solder (a 0.3 mm drill in a WS2812B-2020's 0.7 mm pad
leaves the LED unsoldered), and KiCad's DRC does not flag it. Until
2026-10-21 a surface pad of a net let that net's vias in wherever the
whole via fit inside its copper (`StaticMaps::via_owner`), so congested
boards put their ground vias in ground pads (the fence's LED row: 3 to
14 on four layers, 28 at 34 mm on two). Now the lowering lists every SMD
pad (`smd` and `connect` pads, their padstack shapes too) in
`Board::no_via_pads`, and every via keeps its annular ring out of their
copper: another net's vias their clearance beyond it as from any copper,
the pad's own net's may touch it from outside (no clearance between a
via and its own pad: a pad of a pour net gets its via beside the pad,
the stub of length zero). Through-hole pads hold no via either way (their
copper keeps every via at its clearance; a hole that reaches past a thin
ring lets its own net's vias within only the hole clearance, as before).

One SMD pad per footprint is exempt: an exposed pad (a QFN's, a DFN's,
an SOIC's thermal pad), the largest SMD pad, at least twice any other
SMD pad's area, its centre strictly inside the box of the centres of at
least three other pads. The pins enclose it, a via is the designer's way
out (KiCad's own footprints carry thermal vias there) and a large pad
keeps its solder. A tab beside a row of pins (SOT-223, D-PAK, SOT-89)
is not exposed: a track reaches it from outside. SMD pads of one number
that touch or lie within 0.5 mm of each other are one pad to the test,
of their boxes' box (`exposed_pad`, shared with score-kicad-board's
count; 2026-10-21): an exposed pad drawn on both sides (Sisu's U2.11,
MSign's U1.21, LC_PAMV4b's U3.25, ZSWatch's IC401.33: an F.Cu and a B.Cu
copy at one place, thermal vias between) was its own copy's "other pad
of its size", both copies were listed and the pad's pour-net vias had to
go beside a pad its pins enclose; a pad split into pieces is one too.
Two same-number pads apart (a connector's shield tabs) stay two.

The via-owner map does not depend on the pads' order: a node an earlier
pin of the net left to the net (`beside`) stays the net's where the
exposed pad holds a whole via (with the pins before the pad, KiCad's
usual order, the pad's arm found the node blocked and gave it to no
net, losing legal via sites along the pad's edge).

A touch electrode is exempt too (2026-10-23, `touch_electrode` in
board_router/lower.rs): a footprint whose only pad is of a custom shape,
copper art no part is soldered to. d20_pcb's J2 is a face of the die,
enclosed by the footprint's own copper frame on F.Cu and by a keepout on
B.Cu, and the whole die is the footprint's solder mask opening; the
designer reaches it with a via in the face's corner. A mask opening
keeps every via out (`solder mask opening` keepouts); a touch
electrode's own opening (labelled `... of the touch electrode J2`)
leaves its net the vias that lie inside its pad (`own_opening` and
`via_inside` in grid.rs: the via-owner arms of a via inside its net's
own copper, not the `beside` arm of a listed pad, where only the mask
web parts the via from the pad's solder). KiCad bridges only copper of
different nets in one opening. Other footprints' openings keep their
rule: letting an exposed pad's mask windows hold its vias moved Sisu's
fingerprint run from 23 to 29 unconnected, and letting vias in beside
pads under an opening routed OpenRX-Mono's U.FL from 12 to 23.

`via_in_pad: true` in the router config restores the rule of before for
boards whose vias are filled and capped. score-kicad-board's "vias in
pads" (SMD pads under 2 mm but an exposed pad, the drill over the pad's
copper box) stays the check.

A path no longer hops: a via within the hole-to-hole spacing (the drill
plus `hole_to_hole`) of the previous via of the path being searched is
not taken (`hops_from_a_via`; the net's routed branches kept theirs
apart already by `own_via_near`). With vias out of the pads Castor's
Net-(C33-Pad1) went up and down again through two vias 0.25 mm apart to
pass a back-side track, which KiCad reports as hole_to_hole whatever
the nets.

### Board edges and copper of no net (2026-10-21)

KiCad 10.0.7's copper_edge_clearance measures copper against more than
the outline (checked with kicad-cli on boards made for it): an unplated
slot (an NPTH pad whose drill sizes differ, a milled hole; "NPTH pad of
J1") is board edge, a round NPTH hole, a plated slot and an oval drill
of equal sizes are not (they keep the hole clearance); a graphic on the
Margin layer, on the board or in a footprint, is edge with its stroke
and its fill (Edge.Cuts strokes count as lines of no width), a text on
Margin is not. The lowering (`board_router/lower.rs`) gives an unplated
slot's hole obstacle the edge clearance as its clearance floor (the
hole kind keeps the larger of it and the hole clearance, tracks and
vias alike) and lowers Margin graphics as keepouts on every copper layer
at the edge clearance plus the arcs' chord error (`margin_obstacles`;
KiCad's zone filler knocks Margin out too, and the static maps' fill
keeps them). buganini's USB-C shell slots, facade60's switch slots and
michpro's Margin outline were the rules boards' edge findings.

Copper of no net keeps KiCad's Default class: other copper keeps the
larger of its own clearance and Default's (a board graphic of no net, a
footprint's copper graphic, a footprint's text on copper; a graphic
whose net is not routed keeps that net's class). Before, such copper
kept only the routed net's clearance. Footprint graphics on several
layers at once (KiCad 9's `(layers "B.Cu" "B.Mask")`: tuxkeychain's
GRAPHIC3 logo, 10 findings at 0.01 to 0.07 mm) are copper on their
copper layers and openings on their mask layers; only a single `(layer
...)` was read, so they were neither. A footprint's visible texts
(`fp_text`, KiCad 8's `property`) on a copper layer are copper of no
net, on a mask layer openings, as the board's texts are: the stroke
font's box (`footprint_text_box`), in the footprint's frame (KiCad 7 and
8 store the text's angle relative to the footprint's), a knockout text
its plate, a hidden one nothing (jnftech's test points with their Value
on F.Cu: clearance and shorting_items against "Value field" in the
intake).

Two shapes were smaller than KiCad's: an arc graphic is flattened to
chords up to 0.005 mm inside the arc, so its stroke now grows by that
tolerance (name-card's 10.7 mm antenna arc AE1: a track 0.1978 mm from
it under 0.2); a rounded-rectangle pad's corner named in `(chamfer ...)`
with a `chamfer_ratio` of 0 is sharp, not rounded (`roundrect_pad.rs`;
AQEYE's solder jumpers: tracks 0.126 mm from the sharp inner corners
under 0.2, checked with kicad-cli).

### Dimensions, text boxes, cached copper texts and padstacks (2026-10-21)

Checked with kicad-cli 10.0.7 on boards made for each (the unit tests in
`board_router/tests.rs`, lowering_gap_tests, use the same boards):

- **Dimensions** on a copper layer are copper of no net
  (`dimension_shapes`): an aligned or orthogonal dimension's two
  extension lines (from the extension offset to the extension height past
  the crossbar; an orthogonal one's second line runs from its point to
  the crossbar's end), the crossbar, the arrows (27.5 degrees, the arrow
  length; pointing out, or in on tails with `(arrow_direction inward)`)
  and the text's stroke-font box. KiCad measures from the lines' centres,
  not their drawn thickness: tracks 0.19, 0.15, 0.10, 0.064 and 0.05 mm
  from the crossbar, an extension line's start, an orthogonal second
  extension and an arrow, as zero-width lines predict; the lowering uses
  zero-width capsules. A leader, centre or radial mark is a line between
  its points; KiCad 5's stored lines (`feature1` ... `arrow2b`) are read
  as they are. shrimp-18's GameBit ("Dimension '10' on F.Cu": three
  shorting_items), robotique-udes' BioGenius (B.Cu).
- **Text boxes** (`gr_text_box`, a footprint's `fp_text_box`, in its
  frame) on copper are their box, filled, grown by half the stroke: KiCad
  measures from the outline grown so, `(border yes)` or not, and from the
  glyphs inside (a track 0.05 mm past the stroke of either). On a mask
  layer the box is an opening. A hidden one is nothing. A box of `start`
  and `end` keeps those corners whatever its `angle`: KiCad's parser
  turns the text, not the box (a box turned off the axes is written as
  `pts`). The intake has no text box on copper or
  mask (all 80 are on silk, fab or user layers).
- **A cached (TrueType) text on copper** is drawn by KiCad from the font
  installed, not from the file's render cache (TomKeddie's six-line SAO
  sign in Cantarell bold, 5.8 mm: KiCad's plotted glyphs up to 0.92 mm
  outside the cached ones, median 0.14; tracks 0.22 mm from the cache were
  0.02 from KiCad's). Each line of the cache is also copper as a box, in
  the text's frame, grown by a tenth of the text's height
  (`cached_text_line_boxes`); the gaps between lines stay open. The
  stroke-font box the mask openings use would cover a centred six-line
  text 1.7 times as wide as it is. 42 such texts on 22 intake boards.
- **Padstacks** (KiCad 9's `(padstack (mode front_inner_back) (layer
  "Inner" ...) (layer "B.Cu" ...))`, `(mode custom)` per copper layer): the
  pad's other shapes are copper of its net on their layers
  (`padstack_variants`), and the back's mask aperture is the back
  copper's. adrianmarple's Raspberry Pi header (1.2 mm on F.Cu, 1.778 on
  B.Cu) and dennisss's lens mounts (4 mm on F.Cu, 3 on B.Cu): B.Cu
  tracks shorted and bridged to the larger back copper. Since 2026-10-21
  the pad's own shape is copper only on F.Cu and the layers the stack
  does not name (it was on every layer: under a smaller back shape the
  model had copper KiCad has not, and `copper_components` called a B.Cu
  track that ended there connected). Each named layer's shape is merged
  into the pad's terminal (`MergedPad::padstack`): the terminal's nodes
  on that layer lie in that shape, a track ending on it ends on the
  terminal, its zone connection, thermal relief and spokes are the
  pad's, its spokes' frame its own size. A pad whose every layer the
  stack names keeps its own shape everywhere.
- **Blind, buried and micro vias** on the board (a blind via's layers
  "F.Cu" and "In1.Cu") are copper on the layers from their first to their
  last (`via_layers`), a through via on every layer; they were copper on
  every layer, so a track on another layer over one was kept off it and
  `copper_components` could join through it. The router's own vias are
  through vias.
- **A plated hole that lists no copper layer** (`(layers "*.Mask")`, a
  DevKitC's mounting pins) is copper on every layer, as KiCad shorts it on
  F.Cu, an inner layer and B.Cu; a pad listing some copper layers is
  copper only on those.
- **An arc ending where it starts** is a whole circle with `mid` across
  from `start` (`outline::arc_points`: cnlohr's ring pad, a custom pad's
  arc primitive; jetson's two small rings on In4.Cu).
- **Footprint graphics of a net** stay copper of no net: KiCad 10.0.7
  connects nothing to them and keeps every net's copper, their own net's
  and their own pad included, at the clearance (an antenna polygon of net
  A: clearance findings at 0 mm against its pad and an A track, the pad
  unconnected), which is what the lowering did (Default's clearance).

### Solder mask bridges (2026-10-15)

KiCad 10.0.7's `solder_mask_bridge` (DRC_TEST_PROVIDER_SOLDER_MASK): items
of two nets bridge when an item on a mask layer (a pad with that mask
layer, an untented via, a mask graphic or text) comes within the board's
`solder_mask_min_width` plus both mask expansions of another such item,
or when covered copper (a track, a tented via, zone fill) comes within
`solder_mask_to_copper_clearance` (the project's rules) plus the
opening's expansion. A pad's expansion is its own `solder_mask_margin`,
else its footprint's, else the board's `pad_to_mask_clearance`; a via's
the board's; a text has none. Pairs of one net never bridge; a plain
opening (a graphic, a mask-only pad) bridges once it exposes two nets.
Vias are tented unless the setup says otherwise (`(tenting ...)`, KiCad
9's `(tenting front back)`, the old plot setting `(viasonmask yes)`).

The lowering keeps it as keepouts of the opening's net: graphics, texts
and mask-only pads (see the island and bodies sections of
docs/changelog.md), and a pad's aperture, which other nets' tracks and
tented vias keep at the solder-mask-to-copper clearance plus the pad's
expansion, untented vias at the web plus both expansions (a pad or
footprint margin of its own also keeps its margin plus half the web, the
older rule). A cached TrueType text on a mask layer also opens the stroke
font's box: KiCad draws a face it does not have in a stand-in, not from
the cache. Only where the project rates solder_mask_bridge an error
(KiCad's default); at warning or ignore the router keeps the looser rules
of before (OpenESC 4in1-mini: the exact rule took its warnings 29 to 2 and
cost 9 unconnected). Not modelled: via-to-via mask spacing (no corpus
board has untented vias with a web or an expansion), pads placed under
another part's opening (placement's), zone fill under openings.

A pad whose layers open the mask on a side it has no copper on (c-lydia's
U3: `(layers "F.Mask" "B.Cu" "F.Paste")`) bridges other nets' copper under
that opening to itself ("Front solder mask aperture ... Track on F.Cu ||
Pad on B.Cu"): the aperture is lowered on that side too, of the pad's net,
whatever the copper clearance (2026-10-21).

Openings the designer drew along their own tracks (AntonSievering's
0.54 mm F.Mask strips along RF lines, a pad at one end, over the GND pour;
bostang's strips from U1's pins; eigenhari's B.Mask segments) expose the
pour's fill once the tracks are stripped: KiCad bridges the pad, the pour
and whatever is routed under them. Taking such an opening for no net's
where it reaches farther than the pour's clearance from its pads (the
pour shows) made AntonSievering 7 to 11 findings and eigenhari's PQ_PL2 6
to 9 (the pads' own stubs leave under the strips), so the opening stays
the pad's net's; only the designer's track along the whole strip avoids
the bridge. The same holds for an opening over pads of several nets
(Sisu's J_LCD1 rectangle, ringshout's J2): a track leaving a pad inside it
bridges. The strict rule subtracts the designer's own pairs (opening and
pad, opening and zone) but not the opening and our track; see
docs/changelog.md, "Committed on 2026-10-21 (lowering gaps 2)".

### Project rules (2026-10-21)

`project_rules.rs` resolves the router's rules from the project, as
KiCad 10.0.7's DRC reads it:

- **Board setup.** A minimum the project does not state takes KiCad's
  default (board_design_settings.cpp, confirmed by the intake's DRC
  reports): `min_track_width` 0.2 (the neck-down width then is 0.2, not
  the 0.127 of a project whose minimum is 0), `min_via_diameter` 0.5,
  `min_through_hole_diameter` 0.3, `min_via_annular_width` 0.1, edge
  clearance 0.5, hole clearance and hole to hole 0.25.
  `rules.kicad_defaults` in `board-router.json` lists the ones taken.
- **Net classes.** A pattern matches as a wildcard or as an anchored
  regular expression (KiCad's `EDA_COMBINED_MATCHER`: `.*(_TAP|V_POE).*`
  is the latter, `+3V3` only the former). A net takes every class
  assigned to it (explicitly, by a KiCad 6/7 class's net list, by every
  pattern it matches), and each value from the first of them by
  `priority` that has one, else from `Default` (KiCad 8's composite
  classes; a class without a width is Default's width, not 0.2). A pair
  of nets keeps the larger of the two clearances, as in KiCad. The rules
  are keyed by connection as the lowering looks them up: a board with a
  bare net beside a hierarchical one of that name (meshsat's
  `/VIN_CELL` and `VIN_CELL`) keeps the slash (since 2026-10-21; before,
  the hierarchical net fell back to Default's rules).
- **Custom rules** (`<project>.kicad_dru`, `project_rules/dru.rs`). The
  file is parsed (comments, units mm/mil/in/um, escapes) and each
  condition evaluated, three-valued, for what the router lays: tracks and
  vias against tracks, vias and pads (SMD, plated, NPTH), on each copper
  layer the rule's `(layer ...)` names, both ways round (KiCad's
  conditions are commutative). Evaluable: `Type`, `NetClass` (a
  constituent's or the composite's name, case-insensitive, wildcards),
  `NetName`, `Net` (also `A.Net == B.Net`), `Layer`, `Pad_Type`,
  `Via_Type`, `isPlated()`, `hasNetclass()`, `hasExactNetclass()`,
  `existsOnLayer()`, the blind/buried/micro via tests (false: through
  vias only). Anything else (areas, courtyards, references, sizes,
  component classes, diff pairs) is unknown; a rule whose result depends
  on it is not lowered. Nets are grouped by what the condition can tell
  apart, so a rule costs the number of groups squared, not of nets.

  KiCad applies the last matching rule (`EvalRules`); the router keeps,
  for each value, the largest of every rule that can match: never less
  than KiCad asks. Every lowered value is a floor: a custom rule never
  lowers a clearance below the net class's (`apply_custom`, and
  `Resolved::new` takes the larger of a pair rule and the classes), and
  two rules on one pair give the larger, not the later. KiCad's custom
  rules replace the implicit net-class rule even when smaller (a JLCPCB
  template's 0.127 mm "Min. Spacing" after a 0.2 mm class). The reason:
  the condition is three-valued and the router lays items whose final
  neighbours it does not know, so it keeps what every rule that can match
  asks, and a smaller value is honoured only where it is surely the last
  rule for every item pair it covers, which the lowering cannot prove for
  the class rule it would undercut. The router therefore never makes a
  KiCad clearance error from a rule, but it can refuse a board the
  designer made routable by lowering a pair's clearance (the rule is
  reported `applied` with the classes kept). Copper graphics and the
  pour's fill take no pair rule either (`rules::obstacle_kind` gives a
  graphic no item kind, the fill brush keeps its class): KiCad applies a
  net-conditioned rule to every copper item of the net. A fourth item
  kind (a graphic or a fill, matched only by rules without a type
  condition) would carry them; the table of kinds is 3 x 3 today. For clearance, hole clearance, edge clearance and hole
  to hole, a rule does not count where a later rule with a minimum of the
  same constraint surely matches (JLCPCB's template ends with "Min.
  Spacing" 0.127 mm on everything, after "Pad to Track" 0.2 mm: KiCad
  checks 0.127 between pads and tracks, AMLing2's drone carrier); the
  nets are grouped by the later conditions too. Lowered: `clearance` per net (a rule on one set of nets
  against every net raises that set; against every pair, every net),
  `track_width` min, max and opt per net (a rule's minimum also floors
  the neck-downs and the narrow-signal width of the nets it matches: the
  router's `width_floor`), and per layer where the rule holds on some
  layers only (since 2026-10-24, below), `connection_width` min as a
  track width minimum (since 2026-10-24, below), `via_diameter`, `hole_size` (vias) and
  `annular_width` per net, and `edge_clearance`, `hole_clearance` and
  `hole_to_hole` for the board (the router has one value each). A
  clearance that no per-net value keeps exactly is a pair rule of the
  router (`ClearanceRule`, "Rules beyond the classes" below): a rule
  between two sets of nets (ChengminLi's flyback, 2.5 mm primary to
  secondary), one from some item kinds only (JLCPCB's and PCBWay's "Pad
  to Track": tracks against pads), one on some layers (`(layer outer)`,
  `(layer inner)`). Per pair of the condition's net groups and both ways
  round, the lowering evaluates the condition for each pair of kinds (a
  track or a via against a track, a via, an SMD or a plated pad) on each
  layer and lists the layers it holds on; a rule true for every kind on
  every layer that raising a set of nets keeps goes to the per-net
  clearances as before. The router's pads are one kind: a rule on SMD
  pads holds for plated pads too (`approximated`). A rule whose project
  severity (or its own `(severity ...)`) is below an error is not
  lowered (hole_to_hole is a warning by KiCad's default, also under its
  KiCad 8 name `hole_near_hole`). Every constraint of every rule is in
  `board-router.json`'s `rules.custom_rules` with a status: `applied`,
  `approximated` (kept more widely: SMD and plated pads alike, one
  value for the board, a via rule on every layer), `no_effect` (the classes and board setup keep at least as much,
  or nothing routed matches), `not_routing` (silk, text, courtyards, pad
  holes, zone connections, assertions), `skipped` (severity, or a layer
  the board lacks) or `unmodelled` (creepage, length and skew without
  `length_matching` or in time units, diff pairs without `diff_pairs`,
  physical clearances, disallows other than blind, buried and micro vias,
  rules on what is unknown). `rules.unmodelled` lists the last one line
  each; the verdict's next_actions name them when a route is incomplete,
  and `drc-kicad-board` prints them on a `next:` line when KiCad finds
  errors.

  The placer has one copper clearance (`copper_clearance_mm`, the
  bodies' floor): the classes' largest and, since 2026-10-24, the
  largest a custom rule asks between the pads of every two nets on some
  layer (`rules.pad_clearance_mm`, evaluated for an SMD or plated pad
  against one, after the later rules). Until then a rule on some layers
  only reached the router alone, as a pair rule: on the OpenESC 4in1
  boards (0.16 mm on F.Cu and B.Cu, 0.09 inside) the placer packed pads
  0.09 mm apart and KiCad found 170 and 222 clearance errors between
  pads of two parts (changelog, "the OpenESC clearance regression"). A
  rule between two sets of nets or from pads to tracks only is no such
  floor (the placer would keep every part 2.5 mm from every other for
  ChengminLi's isolation).

  **Track widths per layer** (2026-10-24). A `track_width` rule that
  holds on some layers only (a `(layer ...)` clause, or a condition on
  `A.Layer`) is kept per layer: the lowering evaluates the condition for
  a track of each net group on each copper layer (`Lowered::layer_width_min`,
  `_max`, `_opt`, per layer and net), and the project gives the router,
  per net it holds for, the bounds on each layer
  (`NetRules::layer_widths`, the every-layer bounds and the board's
  minimum track width and connection floor folded in). The router splits
  those nets' classes by their widths per layer
  (`ClassRules::class_widths`, `class_floors`; the class's `trace_width`
  is the widest, for what reads one value) and routes each layer at its
  own width: the class's width held inside the layer's bounds (its opt
  where it has one). Layers whose widths differ are stamp variants of
  their own (`Resolved::layer_variant`, as layers of different pair
  clearances were): the stamps of a variant measure both classes' tracks
  at that variant's widths, the static maps keep pads and the outline at
  each layer's half width, a node stamps its layer's footprints, and the
  emitted tracks, the escape and pad-centre stubs and the neck-downs
  (per layer: the board's neck or the layer's minimum, never wider than
  the layer's track) take the width of their layer. Vias keep one size.
  The router's check of the result (`verify_with_rules`, in
  board-router.json's internal violations where a net has widths per
  layer) holds each track to its layer's minimum and the pair clearances
  per layer. OpenESC's "2 oz outer copper, track width" (0.16 mm on
  F.Cu and B.Cu over classes of 0.09) routes 0.16 outside and 0.09 inside
  and is reported `applied`; until then every track of every layer was
  0.16 (`approximated`). The layer clearance of the same file was exact
  before (a pair rule per layer). Boards without such a rule build the
  same classes, variants and stamps as before. What remains approximated:
  via rules on some layers (below: no benchmark board has one), SMD against
  plated pads, one value for the board's edge and hole clearances.

  **Connection width** (2026-10-24). A custom `connection_width` rule's
  minimum is a floor on the width of the tracks of the nets it holds for,
  lowered as a `track_width` minimum: per net, or per layer where it
  holds on some layers only (`Lowered::layer_width_min`, into
  `NetRules::layer_widths`), the neck-downs with it (a neck is never
  narrower than its layer's minimum), reported `applied` ("connection
  width at least 0.15 mm: a floor on the track width"); its max and opt
  mean nothing to KiCad and are not read. KiCad checks the union of a
  net's copper on each layer ("KiCad's connection width (necks)"
  below); the router's share of it is the width of what it lays. It
  floors where the project (or the rule's own `(severity ...)`) rates
  connection_width findings errors and, by default, warnings
  (`PCB_MIN_CONNECTION_FLOOR=errors`: errors only; `BoardFacts::
  connection_floor_warnings`); the board setup's `min_connection` alike,
  on every layer (the floor folded into each layer's bounds). No board of
  the corpus has a custom connection_width rule; the board setup's minimum
  is above the board's minimum track width on OpenESC's two 4in1 boards
  only (0.16 over 0.09, warning) of the 60 benchmark boards with a
  .kicad_dru or in the expansion tier, so the new default moves those two
  alone: their inner layers route at 0.16 again, as the designer's do
  (no track of the designer's narrower than 0.16 on any layer).

  **Via rules on some layers** (census 2026-10-24). KiCad 10.0.7, measured
  with kicad-cli on a four-layer board with one through via (0.4/0.2 mm)
  and one rule at a time: a `hole_size` rule holds whatever its
  `(layer ...)` (the drill is one); `via_diameter` and `annular_width`
  are evaluated on F.Cu alone for a via of one size (its padstack's every
  layer is F.Cu), so a rule on `outer` or `F.Cu` holds and one on `inner`
  or `B.Cu` only never fires; `disallow via` with any layer clause
  catches a through via, a condition `A.Layer == 'In1.Cu'` does not. The
  lowering takes a via rule where any of its layers holds (`approximated`):
  exact for hole sizes and for rules that include F.Cu, more than KiCad
  asks (never an error) for diameter and annulus rules on inner or back
  layers only. On the 14 boards with a .kicad_dru and the expansion tier's
  46 (36 via, hole and annulus rules on 9 boards) no via rule has a layer
  condition, and the only one with a layer clause that matches a via is
  minuet light's and FUEL4EP's "Annular ring width (via and PTH)" (`(layer
  outer)`, 0.075 mm, `no_effect`: the classes keep more; exact, outer
  includes F.Cu); their hole rules on `outer` and the power manifold's
  (NPTH and castellated pads) match no via. None has `via_count` or
  `disallow via`. Their other via
  rules (`via_diameter`, `hole_size`, `annular_width` on every layer:
  JLCPCB's and PCBWay's templates on the laptop boards, tuxkeychain,
  ErgoSNM, AMLing2's drone carrier, the power manifold) are `no_effect`
  (the classes and the board setup keep them) except the power manifold's
  two that depend on the via's own size (`unmodelled`). Nothing was
  changed for vias: no board asks.

  What the lowering cannot do yet (the edge clearance from NPTH slots
  and Margin graphics it does since 2026-10-21, "Board edges and copper
  of no net" above): a
  later rule that loosens an earlier one only inside an area or a
  courtyard (ChengminLi's "YPS jumper pins (intended bridge)", 0.5 mm
  between the Y capacitor jumper's primary and secondary pins under the
  2.5 mm isolation: the router keeps 2.5 mm there and cannot reach the
  jumper's pins, 2.54 mm apart); clearances of pours (the pour model
  keeps the classes', not the pair rules', from other nets' tracks).

### Rules beyond the classes (2026-10-21)

`Config::net_rules` (`crates/pcb-router/src/rules.rs`), filled by
`project_rules.rs` and copied by `core_config`, holds what the classes
cannot say: a track width floor per net, clearance rules between two
sets of nets per pair of item kinds and per layer, and the hole-to-hole
spacing a via keeps from a pad hole of its own net. `None` (a project
without them) routes as before, byte for byte.

- **Classes split by group.** `ClassRules::split` puts nets in groups
  (the nets no rule tells apart: the same side of every rule) and gives a
  net a class of its own group and width floor; a class keeps its index
  for the first nets that need no other, so rules that split nothing (the
  same-net hole spacing alone, a rule the classes keep) leave the board's
  classes, the lattice and every stamp as they were (Castor, k30 and
  jetson, whose projects rate hole to hole an error, route with identical
  fingerprints).
- **Width floors.** A net's class width rises to its floor, and its neck
  class is the board's neck width or its floor, the wider
  (`with_neck_classes`, `neck_width_of` for stubs and escapes): a
  `track_width (min ...)` rule on some nets holds in their neck-downs and
  in the narrow-signal step (the lowering sets the narrow width as the
  class width; the floor raises it again).
- **Widths per layer** (2026-10-24). A net of `NetRules::layer_widths`
  gets a class of its widths and floors per layer
  (`ClassRules::class_widths`, `class_floors`; `width_on`, `floor_on`,
  `neck_width_on`), and `Resolved::layer_variant` tells layers apart
  where any net's bounds differ, so that each variant's stamps and each
  layer's static maps measure the tracks at that layer's width
  (`variant_layer`: the layer a variant's widths are read on). A net
  whose bounds give the same width and floor on every layer keeps one
  class, as before.
- **Pair clearances.** `Resolved` tabulates, per pair of groups, pair of
  kinds (track, via, pad) and layer variant (layers the same rules hold
  on), the clearance the rules ask beyond the classes. The stamps are
  built per pair of classes and per layer variant (`stamps[class][variant
  * classes + querying]`; one variant without layer-qualified rules):
  track to track, track to via and via to track per variant, via to via
  the largest of every layer's (a through via meets the other via on
  every layer). A node stamps its layer's footprints; a step from a layer
  of another variant stamps whole footprints, not the leading edge. The
  static maps keep a pad, an existing track or an existing via (told
  apart by the lowering's labels; copper graphics are none of them) at
  the class's clearance or the rule's, per layer for tracks and the
  largest over the obstacle's layers for vias; the pad stubs
  (`stub_is_clear`) too. A pad also keeps what its net's tracks keep: the
  net leaves it by a stub off the lattice that only the pad's clearance
  protects (tuxkeychain's outer-layer 0.127 mm over classes of 0.1 left
  stubs 0.104 mm from other tracks without it). A pad-centre stub (the
  cosmetic stub from the node a route ends on inside a pad to the pad's
  centre, at the track's width: wider than a narrow pad, it reaches past
  the pad's copper) is checked against fixed copper when it is laid and
  against every net's routed copper when the routes are emitted, where a
  stub too close is dropped (`finish`): that check now holds the rules
  too (`verify::verify_with_rules`; tuxkeychain's 0.3 mm GND stubs along
  pad 2 of U4, a QFN's 0.2 mm pad, 0.104 mm from the track and the stub
  of HPL's pad 1 under the 0.127 mm rule, 3 findings). The internal verifier of the board router
  (`verify`, the attempt ladder's count) checks the classes' clearances
  only; KiCad checks the rest.
- **Same-net hole to hole.** Where the project rates hole_to_hole findings
  errors (KiCad 8's `hole_near_hole` too), a via keeps the board's hole to
  hole from the plated holes of its own net's pads (KiCad checks drilled
  holes whatever their nets: nuclone's VSS via 0.464 mm from J3's VSS pad
  hole at 0.5). Before, its own net was exempt from the hole and kept only
  the pad copper's rule.

### Memory (2026-10-03)

A thread's search scratch is the largest part of a big board's footprint
(cost, parent and generation marks per lattice state, one scratch per
worker thread), followed by the per-class occupancy and static maps. Marks
that only deduplicate within one operation are bitsets cleared from the
list of what was touched (stamp marks per map, the own-stamp mask per
thread); search generations are 16-bit and cleared on wrap. ATAT1800 (four
layers, 3952 x 1227 nodes): 8.65 GB -> 5.52 GB peak, identical result.
Next candidates: the static maps per class (41 bytes per node and class)
and fewer scratches on boards where parallel batches are small.

2026-10-09: verbose runs print the maps' sizes after the lattice line
(`memory: ...`). Per node and layer, for C rule classes of which S are
searched (net classes and their neck classes; the rest are pour brushes,
read only by the pour model): static maps 7C bytes (the trace map 4, the
edge directions 1, their owner 2), per node 1 (vias blocked) and for a
searched class 6 more (via owner, free layers), and the mask openings'
differences only where they are; occupancy 6C (a 16-bit count for the trace and two diagonal maps) plus
2C per node for the via maps; the search records `hot` 12S (were 16C);
history, guard and cover 12 plus 4 per node; a search scratch 12 per state
(were 20: the tree and target marks are hash maps of the few nodes they
hold), one for the router and one per worker thread that routes a batch.
Copies of a router (the via reduction's snapshot, the layout's trials)
share the static maps and leave the scratch, the stamp marks and `hot`
behind; the via reduction's whole copy had doubled every board's peak.
Route mode, one attempt, 4 cores, x155 -> x161: Castor 1391 -> 687 MB,
SNSP-CPU-01 2209 -> 1536, katia 6292 -> 4315, identical routes; layout
mode (x155 -> x160, 2 threads): CyberKeeb2040 6916 -> 3395 MB, Qfwfq 3330
-> 1414, identical boards. Threads hardly speed the search (k30-SBC: 34.2, 32.4, 31.3 s on 1,
2, 4 threads) and each holds a scratch: run boards side by side with few
threads each (`RAYON_NUM_THREADS`, the runners' `--threads`).

### Memory (2026-10-22)

What a router holds now, per node and layer, for C rule classes of which
S are searched (the 2026-10-09 sizes above, before):

- static maps per class: the trace map 2 bytes (16-bit owners, as the
  edge and via owners; was 4); the blocked unit steps a bit and, for the
  one node in a hundred that has any, their directions and exempt net
  apart (`EdgeMap`; were 3 bytes dense); per node the via block a bit
  (was a byte), and for a searched class the via owner 2 and the free
  layers 1 byte up to 8 layers, 2 up to 16, else 4 (`LayerBits`; was 4);
- the search records (`hot`) 8 bytes per searched class (were 12): the
  history, the static map's 16-bit value and an index into the table of
  (guard, cover) pairs of the board (`hot_pours`; few pairs: a pour net
  and the guard's), built in place when they change (a second set beside
  the first, while the lost-spokes step set and restored its guards, was
  katia's peak: 1.1 GB);
- a net's stamp list 4 bytes a stamp (the map above the cell's bits in
  one word where they fit, `Stamped`; were 8);
- one search scratch per router, allocated on its first search (a router
  that never searches, the pour measure's, has none), and the scratches
  the jobs of a parallel batch use pooled with the router
  (`ScratchPool`): until now every pool thread kept a scratch of its own
  in a thread local, beside the router's and after it was gone, so one
  thread held two;
- the via reduction's snapshot leaves out the occupancy and the tiles'
  claims (the sum of the nets' stamps): a revert takes the changed nets'
  stamps off and the snapshot's back (`restore_lean`);
- the allocator: glibc's malloc maps every block of a MiB or more and
  unmaps it when freed (`configure_allocator`, M_MMAP_THRESHOLD, set
  first thing in `main`). Its threshold otherwise grows to the largest
  block freed (up to 32 MB), the lattice maps (a few MB a layer) came
  from the heap after the first router and stayed resident: katia held
  1.75 GB with no router left after its first attempt, 0.24 GB now.

The pour measure of the board written (`measure_pours`) drops the
source board's parse and its own before it builds its router (jetson's
25 MB file parses to 240 MB; the measure was its peak).

Unchanged: the occupancy (2 bytes per map and node: per class L + 1
maps and 2L diagonal maps), the history (4 bytes per node and layer and
the via layer), the guard and the cover (4 bytes per node and layer
where there are pours), a scratch's 12 bytes per state. On katia
(2775 x 1077 nodes, 4 layers, 10 classes, 8 searched) the occupancy and
the search records are now 67 % of the maps (741 and 730 of 2185 MB). Every route and fingerprint
is the same (docs/changelog.md, "router memory").

### Memory (2026-10-23)

The second round, every route and fingerprint the same (docs/changelog.md,
"router memory, round 2"):

- the occupancy a byte a count (`Occupancy`; two until now): a count is
  how many nets' stamps cover a cell, a handful where nets overlap; a
  count of 255 or more keeps 255 in its byte and the rest in a side map,
  so every count is exact. Nothing keeps a
  second copy (the via reduction's snapshot leaves it out since
  2026-10-22; only the layout's trial copies of a whole router carry
  one). Not shared between classes: classes apart in their vias only have
  the same trace and diagonal maps (katia's three neck classes; a neck
  class and its net class the same via map), but sharing them changes
  the nets' stamp lists, which the fingerprint hashes. Not sparse: a
  search step reads a count per neighbour, and a tile-sparse map costs it
  an indirection;
- the search records (`hot`) a set per class of some net, and one set for
  the classes of one static trace map (classes apart in their vias only):
  of a record only the static map's value depends on the class (the
  history, the guard and the cover are the layer's). A neck class (no net
  of its own) has none: a step into a neck zone reads its net class's
  record with the neck's static map value, which is what the neck's
  record held. Katia's eight searched classes are four net classes and
  four necks: 730 MB of records, 365 now; jiran-ble-lite 422 to 211;
- the guard and the cover maps in 16 bits (`owner16`, as the records
  hold them; were 32), and the lost-spokes step's copy of the guard with
  them; the via cover (a word a node) none on a board without pours.

The maps on the seven boards (MB, one router, one thread; the largest
memory line): katia 2189 -> 1408 (occupancy 741 -> 371, records 730 ->
365, history/guard/cover 162 -> 117), jiran-ble-lite at 0.05 mm 1589 ->
1141, the laptop motherboard 636 -> 497, Sisu 491 -> 379, k30 247 -> 194,
jetson 223 -> 165, Castor 193 -> 134. Katia's largest now: occupancy 370,
the records 365, the static maps 347, a scratch 147, the pour analysis's
transient maps 137, the stamp lists 124.

### Memory (2026-10-26)

The third round (docs/profiling.md, "Memory"; docs/changelog.md, "memory
round 3"), every route and fingerprint the same: the stamp lists as
varint differences (1.5 bytes a stamp, were 4), the search arrays shared
by the process's routers (a net's search takes idle arrays of its lattice
and gives them back; a router held its own and a spare per batch thread
for its life), the occupancy in four bits a count (was a byte). The
profile (`PCB_ROUTER_MEM_PROFILE=1`) prints the resident size's highs,
the phases and every live router's parts. The router's `memory: N MB in
maps` keeps counting a scratch per router (`Scratch::bytes_for`), now an
upper bound.

### Memory limits (2026-10-21)

How many routers the ladder runs side by side (the rung probes, the rungs
after its first attempt, the seeds) follows the tightest of the process's
limits (`MemoryLimits` in board_router/memory.rs): its address space
(RLIMIT_AS, against its virtual size), its cgroup's memory.max (cgroup
v2, every level up to the root; v1's memory.limit_in_bytes; against the
memory in use less the reclaimable inactive_file), an explicit limit
(`PCB_MAKER_MEMORY_MB` or the router setting `memory_mb`, the smaller,
against the resident size) and half of MemAvailable (shared with the rest
of the machine). Until then only MemAvailable counted (and a router was
the process's peak after the first attempt, plus 2200 MB a rung for a
KiCad check that no longer runs inside a rung), so a harness's cap went
unseen: 456 runs of the 5 834-board intake died at a 3 GiB cap, their peak
2.4 to 5.2 times the router's maps with two to four routers at once.

A router's peak is estimated from its lattice maps (the router's
`memory: N MB in maps`, read from `Router::memory_report` as each
attempt builds it; `maps_mb` in board-router.json): the maps and 600 MB
beside them resident, the maps and 760 MB of address space, times a
margin of 1.25 (`router_peak_mb`; 1.2 times and 540 MB, 1.3 times and
780 MB until 2026-10-23, when the occupancy, the search records and the
pour maps shrank; 1.7 and 460, 2.0 and 550 until 2026-10-22, when the
maps shrank and freed blocks went back to the system). Measured with one
router at a time and one thread, a ladder of 60 s of work without probes
and seeds (the whole process's peak, x1622, 2026-10-23):

| board | maps MB | peak RSS | RSS / maps | VmPeak | VM / maps |
|---|---|---|---|---|---|
| katia (4 layers, 10 classes) | 1408 | 1763 | 1.25 | 2029 | 1.44 |
| jiran-ble-lite at 0.05 mm (2) | 1141 | 1212 | 1.06 | 1506 | 1.32 |
| byrantech laptop motherboard (6) | 497 | 774 | 1.56 | 979 | 1.97 |
| Sisu_SSE-9 (v16 placement) | 379 | 554 | 1.46 | 736 | 1.94 |
| k30-SBC (v16) | 194 | 357 | 1.84 | 598 | 3.08 |
| jetson (v16) | 165 | 744 | 4.51 | 903 | 5.47 |
| Castor (v16) | 134 | 328 | 2.45 | 565 | 4.22 |

What lies beside the maps hardly follows them (katia 355 MB, jetson 579):
a slope of 1, and the offsets pass jetson by 21 MB resident and 22 MB of
address space before the margin. The table of 2026-10-22 (x1600):

| board | maps MB | peak RSS | RSS / maps | VmPeak | VM / maps |
|---|---|---|---|---|---|
| katia (4 layers, 10 classes) | 2189 | 2534 | 1.16 | 2809 | 1.28 |
| jiran-ble-lite at 0.05 mm (2) | 1589 | 1608 | 1.01 | 1954 | 1.23 |
| byrantech laptop motherboard (6) | 636 | 897 | 1.41 | 1112 | 1.75 |
| Sisu_SSE-9 (v16 placement) | 491 | 721 | 1.47 | 912 | 1.86 |
| k30-SBC (v16) | 247 | 406 | 1.64 | 629 | 2.55 |
| jetson (v16) | 223 | 796 | 3.57 | 1052 | 4.72 |
| Castor (v16) | 193 | 392 | 2.03 | 571 | 2.96 |

jetson holds the most beside its maps (its 25 MB file parsed, 240 MB,
and lowered); the line passes it by 12 MB before the margin. The table
of 2026-10-21 (x1225):

| board | maps MB | peak RSS | RSS / maps | VmPeak | VM / maps |
|---|---|---|---|---|---|
| DevMubarak1 pidgin_translator (2 layers) | 244 | 255 | 1.05 | 343 | 1.41 |
| sssamuelll sam17-power-v2 (2) | 739 | 981 | 1.33 | 1273 | 1.72 |
| sikaxn nz (6) | 298 | 380 | 1.28 | 587 | 1.97 |
| zOnlyKroks WK-3 (4) | 763 | 1280 | 1.68 | 1509 | 1.98 |
| mark69163 PRODIGY-MOTHERBOARD (4) | 580 | 929 | 1.61 | 1137 | 1.96 |
| Castor (v16 placement) | 264 | 669 | 2.54 | 879 | 3.33 |
| k30-SBC (v16) | 319 | 792 | 2.49 | 1018 | 3.19 |
| jetson (v16) | 302 | 969 | 3.22 | 1149 | 3.80 |

The ratio falls with the maps' size: what a router holds beside its maps
(the board parsed and lowered, its pours, the router's records; 290 MB of
jetson's before any router) weighs most on the small ones, so a line
(slope and offset) fits all eight where one ratio would size the large
boards three times too big. The routers that run at once are as many as
fit what is free under each limit (MemAvailable: half of it); at least
one. The decisions that change what the ladder attempts go by the limit
itself, a fixed number, not by what is free: a finer pitch is not tried
when one router of it (its maps grow with the square of the pitch ratio)
does not fit a limit of the process's own, and the probes do not run when
two do not, and not when one router of it would take more than 0.4 of
MemTotal (until 2026-10-21 that rule took the process's peak resident
size after the ladder's first rung, which followed how many probes had
run side by side; now the same maps estimate, resident). No decision of
the ladder reads the process's peak any more. Without such
a limit nothing changes but how many run at once, which never changes a
result. The ladder logs the limits as it starts (`memory limits: ...`)
and every side-by-side count with its reason (`ladder: 1 of 3 rungs side
by side (about 2090 MB a router (561 MB of maps), 2428 MB usable under the
address space limit (RLIMIT_AS))`); board-router.json has them as
`memory_limits`. The layout's placement race sizes its probes from
`memory_mb`, which is MemAvailable or less under a limit of the
process's own (a count only: its outcome is the one of probing one
placement after the other).

## Two-level search

Every connection is first planned on a coarse graph of 16 x 16-node tiles
(per layer, with via edges between layers). A tile costs more the fuller it
is, with the fill maintained incrementally from the occupancy maps, and more
again where conflicts keep happening. The lattice A* then runs only inside the
planned corridor (one tile of margin); nets that keep losing negotiations get
wider corridors, and a failed corridor falls back to the plain window and the
whole board. This is the "graph layer tells the geometric layer where to go"
idea; it halved the time on the large boards and reduced vias and copper.

## Capacitated corridors (2026-10-24)

An experiment behind `capacitated_corridors` (router config, off;
`PCB_ROUTER_CAPACITATED_CORRIDORS` overrides it, `capacitated_corridors`
in the KiCad router settings). Florian's idea: route the board as a graph
first, with hard capacities, and when the geometric search cannot get
through a corridor, teach the graph instead of searching wider.

**Design** (`pcb_router::capacitated`, `router/capacitated.rs`,
`pcb_global::corridors`):

- **The plan.** When a negotiation starts, the router builds pcb-global's
  global route of its own board (`CorridorPlan`: the capacities per
  boundary and layer and the via sites of docs/global-route.md, the
  PathFinder negotiation, 1.5 mm tiles). pcb-global depends on
  pcb-router, so the plan reaches the router through a trait
  (`CapacitatedPlan`) and a factory pcb-kicad registers
  (`register_corridor_planner`, in `core_config`); without one the switch
  does nothing. An `update` (a moved part) drops the plan; the next
  negotiation builds it again.
- **The corridor.** A net the plan routes searches, in the negotiation
  (not the hard repair, not the clean-up), inside its plan's tiles and one
  tile around, mapped onto the router's 16-node tiles by overlap (the
  band). The tile planner (`plan_corridor`, its fill cost) picks the way
  inside the band first (one router tile of margin, the band its wall);
  if that search fails, the whole band is searched. No widening with the
  reroutes, no window, no board.
- **A miss** (no path in the band) is located on the plan: the plan tiles
  the search reached (its closed nodes, per layer) against those it did
  not; every boundary of the net's global route with one side reached and
  the other not, and every via tile of the route reached on one of its
  layers only, is where it failed. The net is not marked blocked: it
  stays in the negotiation.
- **The learning** (after the batch, `learn_from_misses`): each such
  boundary (and via site) loses one track (via), never below the tracks
  (vias) the routed copper puts across it now (its floor: every net's
  branches walked, each crossing at the net's demand). The nets through
  the lowered boundaries are planned again, then the nets that missed
  (pcb-global's incremental path: their routes out, routed again in that
  order against the others' usage at the negotiation's last prices), and
  the missed nets are routed again in their new corridors, up to
  `capacitated_retries` (2) rounds; nets still missing then wait for the
  next iteration (`deferred`). A miss nothing could be learned from (no
  boundary between reached and unreached tiles, or every floor held) sends
  that net's next search to the window and the board (`fallbacks_nothing_learned`).
- **No global path.** A net whose plan does not reach every terminal
  under the learned capacities searches the window and the board, as
  without the switch (`fallbacks_no_path`), within the same two-failure
  rule (`hopeless_searches`). A net the plan does not have (all its pads
  in one 1.5 mm tile) uses the tile planner as without the switch.
- **Conflicts teach too** (`capacitated_stuck`, 4;
  `PCB_ROUTER_CAPACITATED_STUCK`, 0 off): every second iteration, a net in
  conflict for four iterations running lowers the boundary of its global
  route with the most conflict cells in its two tiles, and is planned
  again with the nets through it (the reading of "the negotiation ejects
  the net from it repeatedly").
- **Width** (`capacitated_growth`, 0; `PCB_ROUTER_CAPACITATED_GROWTH`):
  with a value, the band grows a plan tile every that many reroutes of
  the net (at most ten), as the tile planner's corridors grow (added
  after the first tier: the variant `g6` below).
- **Work.** The plan's build and re-plans count in the router's work at
  pcb-global's model (5 units a capacity distance test, 30 a state popped,
  150e6 units a second), converted to search expansions at the reference
  rate (`UNIT_EXPANSIONS`) and added to the router's expansions, so every
  budget counts them; the floors' walk at a sixteenth of an expansion a
  step. A router with the switch off is unchanged: the expansion tier at
  x14200 with the switch off is identical to main's x1754 row for row.

**The instrumentation** (the verdict's `corridors`, every run, either
setting; summed over every router of the process, the ladder's attempts
and probes alike): per mode (`negotiation`, `hard`) and kind (`corridor`:
the tile planner's corridor, or with the switch the planner inside the
band; `band`; `window`; `board`) the searches, how many found a path and
their work in seconds; `plans_built`, `plan_work_seconds`, `misses`,
`reductions`, `conflict_reductions`, `floor_held`, `no_boundary`,
`replans`, `replanned_nets`, `replan_work_seconds`, `retries`,
`fallbacks_no_path`, `fallbacks_nothing_learned`, `deferred`,
`floor_work_seconds`, `hopeless_skips`.

**Measured** (2026-10-24, all at pinned work rates, `PCB_ROUTER_NO_WALL_GUARD=1`
so that every row is deterministic; logs `build/corr1-*`). Off: main's
x1754 (the expansion tier, the fingerprints) and x14202 with the switch
off (the dense boards: identical to x1754 on the three dense boards x1754
finished, and x14200 off identical to x1754 on all 46 tier rows). On:
x14200 (`s4`: as designed, one plan tile around the route), x14201
with `PCB_ROUTER_CAPACITATED_STUCK=0` (`s0`: the misses' learning alone)
and x14202 with `PCB_ROUTER_CAPACITATED_GROWTH=6` (`g6`: the band grows a
plan tile every six reroutes, as the tile planner's corridors grow; the
fingerprints and the dense boards ran this one, the best on the tier).

*Expansion tier* (46 boards, 8 x 1, 30 s of work; cells KiCad's
unconnected items, `*` a pass; the 25 boards not listed are the same in
all four):

| Board | off | s4 | s0 | g6 |
| --- | ---: | ---: | ---: | ---: |
| rpi_cm5_dronecarrier | 3 | 8 | 7 | 8 |
| ducktop2 bms | 0* | 0* | 2 | 0* |
| musicbrain-busboard | 0* | 0* | 1 | 0* |
| OpenRX-Mono | 12 | 16 | 11 | 12 |
| node_v6_0_0 | 32 | 34 | 31 | 28 |
| ZSWatch-Watch-DevKit | 2 | 3 | 2 | 3 |
| epdInky ED052TC4-Adapter | 2 | 8 | 7 | 6 |
| tuxkeychain | 11 | 6 | 7 | 9 |
| driver_cplex_rp2040 | 87 | 64 | 70 | 39 |
| snaphat main_board | 0* | 0* | 2 | 0* |
| power-manifold backplane | 1 | 3 | 2 | 1 |
| MSign mainboard | 19 | 13 | 35 | 14 |
| nervous-ecg | 4 | 3 | 2 | 3 |
| OpenDeck-r3.1.2 | 12 | 13 | 8 | 5 |
| facade60_daughterboard | 9 | 17 | 15 | 12 |
| Miniboard | 6 | 11 | 20 | 10 |
| magnetorquer_control_board | 1 | 0* | 0* | 0* |
| Zulassungskarte | 1 | 2 | 2 | 1 |
| CompassTag | 1 | 3 | 2 | 2 |
| anthracite-bottom | 0* | 0 | 0* | 0* |
| Wonkle | 364 | 442 | 442 | 442 |
| **sum** | **588** | **667** | **689** | **616** |
| passes (strict) | 20 | 20 | 18 | 21 |
| new errors | 3 | 2 | 2 | 2 |
| vias | 4308 | 4463 | 4427 | 4658 |
| length mm | 112520 | 110280 | 109603 | 109760 |
| work s | 1250.5 | 1280.8 | 1261.2 | 1247.9 |

Wonkle (271 nets, four negotiation iterations in its 30 s) is the same
442 in every variant: its first six reroutes run the same corridors in
all three. It ends its negotiation with fewer nets in conflict than off
(254 against 260), and the hard repair then leaves 793 open terminals
against 723: a board decided by the repair on an unconverged
negotiation. Without it, `g6` is 174 against 224.

*Fingerprints* (Castor, k30-SBC, jetson, Sisu; 1 x 4, a quarter of the
budget; x1754 against x14202 `g6`; cells unconnected / strict new errors
/ vias / length mm / work s):

| Board | off (x1754) | `g6` (x14202) |
| --- | --- | --- |
| Castor | 0 / 0 / 240 / 5287 / 174 | 0 / 0 / 306 / 5660 / 135 |
| k30-SBC | 0 / 1 / 343 / 14368 / 277 | 0 / 1 / 371 / 14733 / 275 |
| jetson | 20 / 0 / 584 / 10759 / 315 | 22 / 0 / 602 / 10831 / 315 |
| Sisu | 23 / 0 / 379 / 5553 / 315 | 24 / 0 / 407 / 5561 / 285 |
| sum | 43 / 1 / 1546 / 35967 / 1082 | 46 / 1 / 1686 / 36784 / 1011 |

Passes 2 / 2 (Castor and k30-SBC; strict 1 / 1). `g6` there: 669 misses, 522 reductions, 7372 conflict reductions, 606 re-plans, 450 retries, 211 deferred, 18 fallbacks after nothing learned, none for want of a global path; the plans' and re-plans' work 5.8 s.

*The eleven dense boards* (route mode on the v19 placements at the task
budgets, 3 x 2; x14202 off against `g6`; same cells):

| Board | off (x14202) | `g6` (x14202) |
| --- | --- | --- |
| video | 125 / 0 / 1198 / 39165 / 1107 | 216 / 0 / 1324 / 35956 / 1090 |
| OpenESC 4in1 | 108 / 0 / 150 / 2256 / 1248 | 109 / 0 / 143 / 2108 / 1120 |
| 4in1-mini | 72 / 0 / 118 / 1545 / 905 | 56 / 0 / 135 / 1508 / 960 |
| Glasgow | 144 / 0 / 329 / 4881 / 1262 | 162 / 0 / 347 / 4458 / 1258 |
| SmartSpin2k | 117 / 113 / 1201 / 24688 / 688 | 132 / 113 / 1260 / 24656 / 719 |
| MegaDrive | 157 / 0 / 1270 / 32617 / 1030 | 198 / 0 / 1685 / 32919 / 1199 |
| SNSP-CPU-01 | 119 / 0 / 1365 / 22495 / 882 | 182 / 0 / 1196 / 19637 / 847 |
| split72_left | 25 / 0 / 884 / 20941 / 1255 | 46 / 0 / 1049 / 19856 / 1260 |
| protoesp | 65 / 2 / 275 / 5148 / 733 | 71 / 2 / 265 / 4808 / 1003 |
| DSP-ADAU1452 | 25 / 0 / 206 / 7111 / 1056 | 42 / 0 / 204 / 6048 / 1033 |
| 0xCB panel | 53 / 0 / 277 / 4847 / 1140 | 53 / 1 / 327 / 5190 / 1195 |
| **sum** | **1010 / 115 / 7273 / 165694 / 11305** | **1267 / 116 / 7935 / 157145 / 11685** |

Passes 0 / 0. Better only on 4in1-mini (72 to 56); the 0xCB panel ties
at 53 (its plan reaches not every terminal of many nets: 16 905
searches went to the window for want of a global path).

The learning, summed (`g6`; tier / fingerprints / dense): misses 404 /
669 / 2212; boundaries lowered after a miss 376 / 522 / 2383; after
conflicts 15 646 / 7372 / 41 795; floors that held 9808 / 2401 /
38 539; misses that named no boundary 8 / 0 / 175; re-plans 2341 / 606
/ 2324 (21 989 / 10 010 / 62 784 nets); retries 307 / 450 / 1722;
deferred 97 / 211 / 490; fallbacks after nothing learned 46 / 18 / 343,
for want of a global path 0 / 0 / 16 908; plans built 242 / 16 / 69;
the plans' and re-plans' work 5.3 / 5.2 / 43.5 s.

**Where the time goes** (the instrumentation; the share of the verdicts'
work summed over the boards):

| | tier off | tier `g6` | dense off | dense `g6` |
| --- | ---: | ---: | ---: | ---: |
| negotiation corridor | 781 s (62.5 %), 207706 / 206206 | 669 s (53.6 %), 251942 / 248940 | 6255 s (55.3 %), 486905 / 452573 | 3887 s (33.3 %), 687811 / 633465 |
| negotiation band | - | 31 s (2.5 %), 3003 / 2599 | - | 2482 s (21.2 %), 54346 / 52134 |
| negotiation window | 4 s (0.3 %), 119 / 69 | 1 s (0.1 %), 50 / 5 | 1042 s (9.2 %), 11022 / 9949 | 355 s (3.0 %), 17251 / 16469 |
| negotiation board | 3 s (0.2 %), 26 / 0 | 2 s (0.2 %), 20 / 0 | 151 s (1.3 %), 1057 / 197 | 113 s (1.0 %), 748 / 0 |
| hard corridor | 33 s (2.6 %), 3241 / 1388 | 48 s (3.9 %), 4753 / 1811 | 472 s (4.2 %), 23289 / 7130 | 986 s (8.4 %), 37777 / 12207 |
| hard window | 339 s (27.2 %), 44669 / 43755 | 360 s (28.9 %), 46550 / 45130 | 1913 s (16.9 %), 58939 / 52939 | 2128 s (18.2 %), 68834 / 58964 |
| hard board | 27 s (2.2 %), 158 / 1 | 30 s (2.4 %), 205 / 14 | 362 s (3.2 %), 1648 / 337 | 528 s (4.5 %), 1922 / 428 |
| plans, re-plans, floors | - | 5.7 s (0.5 %) | - | 46.3 s (0.4 %) |
| the rest (pours, lattices, checks) | 63 s (5.0 %) | 100 s (8.0 %) | 1109 s (9.8 %) | 1159 s (9.9 %) |
| work in all (boards) | 1250 s (46) | 1248 s (46) | 11305 s (11) | 11685 s (11) |

(Cells: work, its share, searches / found.)

**Reading.**

- **The default stays off.** On the tier the best variant (`g6`) has
  more unconnected items (616 against 588; Wonkle +78, the other 45
  boards 174 against 224), one pass more (21), the same work (1247.9
  against 1250.5 s): it fails the unconnected condition. As designed
  (`s4`): 667, 20 passes, work +2.4 %; the misses' learning alone
  (`s0`): 689, 18 passes. Fingerprints 46 against 43 (vias +9 %, work
  -7 %); dense boards 1267 against 1010 (+25 %, work +3 %, vias +9 %).
- **Where the time goes** (off). The negotiation's corridor searches are
  55 to 63 % of the work, the hard repair's window searches 17 to 27 %.
  The negotiation's window and board searches, the wider searches this
  experiment was to replace, are 0.5 % of the tier's work and 10.5 % of
  the dense boards', almost all on two boards: protoesp (57 % of its
  work: its tile-planner corridors found a path in 10 209 of 22 774
  searches, the window then in 5348 of 5362) and SmartSpin2k (52 %:
  16 863 of 26 219, then 3288 of 3475); MegaDrive 14 %, the others 0 to
  5 %. Board-wide searches: in the negotiation 26 on the tier (none
  found), 1057 on the dense boards (197 found), 1.3 %; in the hard
  repair 158 (1 found) and 1648 (337 found), 2 to 3 %. No search was
  skipped for two board-wide failures (`hopeless_skips` 0 everywhere).
- **The premise holds on two boards, and there the learned corridor
  costs what the window did.** Elsewhere the negotiation's corridors
  seldom miss (0.7 % of the tier's corridor searches off; the
  capacitated corridors miss in 0.16 %), so the misses' learning has
  little to learn from; it is cheap (plans and re-plans 0.4 to 0.5 % of
  the work). On SmartSpin2k the band search takes the window's place at
  the window's price (63 % of its work, 12 222 of 12 232 found; 10
  misses) and ends worse (132 against 117); on protoesp too (63 %, 306
  misses, 270 reductions; 71 against 65, and 1003 s of work against the
  733 the off side needed).
- **The corridors' shape decides.** Inside the band the tile planner
  picks the way as before, but the band follows the plan, and nets that
  keep losing cannot take the room around it: widening the band with the
  reroutes recovers most of the tier's loss (`g6` 616 against `s4`'s
  667), and the conflicts' learning beats none (`s4` 667 against `s0`
  689). On the long negotiations of the dense boards the conflicts'
  learning lowers boundaries by the ten thousand (41 795; floors held
  38 539 times), the plans the corridors follow keep moving, and the
  repair has more to do (its corridor searches 986 s against 472, its
  window searches 2128 against 1913): video +91, SNSP-CPU-01 +63,
  MegaDrive +41, split72_left +21.
- **Wonkle** (271 nets, four iterations in its 30 s) gives 442 in every
  variant (its first six reroutes run the same corridors in all three).
  It ends the negotiation with fewer nets in conflict than off (254
  against 260); the hard repair then leaves 793 open terminals against
  723.

**What would change the verdict.** Capacities that see what stops the
lattice inside a tile (pin access: dense misses named no boundary 175
times, and the floors held 38 539 times), so that one miss teaches the
right boundary instead of a track at a time; a band search cheaper than
the window on the boards whose planner corridors fail (protoesp,
SmartSpin2k), which is where the idea applies; a conflict learning
tied to the negotiation's progress (none leaves the corridors stuck,
every second iteration moves them too much on long negotiations).
Outside the negotiation, the plan could serve as an oracle for the hard
repair's board-wide searches (1648 on the dense boards, 337 found).

## Copper pours

Zones covering at least a tenth of the board are pours. Pads inside a pour
are connected; other pads get a short stub and a via to the nearest free pour
node. After routing, the pour's solid pieces are labelled on the lattice;
islands holding pads are stitched to the main piece with vias, and pads that
cannot be stitched are routed to it. Around pads that connect to a pour, other
nets pay extra so the thermal spokes survive. If the router or KiCad still
sees open items, the pour nets are routed as tracks instead and the better
board is kept. The stub's via stays out of the pad (SMD pads hold no via,
"Vias in pads").

A pour that must stay whole on every rung (a ground plane on an inner
layer) is an `add_pours` entry with `"plane": true`: beside the zone the
router writes a rule area that keeps every net's tracks off that layer
(`plane <net> <layer>`, vias allowed), which the lowering keeps as a track
keepout on every rung and KiCad checks. `exclusive_planes` keeps other
nets off only on the rungs that use it (the plane-stub and tracks rungs
let them in).

A rule area that keeps tracks out but lets pours in (`(copperpour
allowed)`: such a `plane` area, a hand-drawn "no tracks on the plane"
area) does not stop the pour's fill in the model either
(`Board::fill_passes`: the static maps take it after the rest, as solder
mask openings, so the fill trace keeps what lies under it). Before
2026-10-21 it did: the fence's four-layer boards, whose In1.Cu GND zone
carries a `GND plane: no tracks` area, routed as if the plane had no
fill at all (GND by tracks and vias), and an `add_pours` plane hid its
own fill.

**A pour kept whole where it matters** (2026-10-21). On two layers a
`plane` is no answer (every crossing needs the back), and
`plane_cut_cost` did not stop the cutting on the fence (37 pieces at 46
mm; why, and `pour_integrity`, below). `keep_whole` in an `add_pours` entry names regions, parts or boxes,
where the pour stays one piece: each is written as a rule area `keep
whole <net> <layer>` with everything allowed (KiCad has no rule by net:
the area only marks the region), and the lowering makes it a keepout of
that net (`Obstacle::net` the pour's): other nets' tracks stay off the
layer there, their vias out (`StaticMaps::via_owner` names the pour's
net in it), the pour's own tracks and vias pass, and its fill fills it.
The router's verifier checks other nets' copper against it.

**How whole the pours stayed.** After the final route (route mode's
ladder; a layout's board kept) the board written is lowered with its
tracks and vias as copper, a router is built on it without routing, and
its pour analysis labels the pieces per pour and layer
(`Router::pour_report`): the pieces holding copper of the net (what
KiCad keeps), each one's area (its free lattice nodes times the pitch
squared, plus half the brush along its rim), slivers under 0.25 mm² left
out, and the pour's area inside the outline. The verdict's `pours` and
board-router.json's `pour_quality` carry it with other nets' track
length on the layer (`cut_mm`, as score-kicad-board's `cut_by_mm`), and a
pour in more than 5 pieces, or whose largest piece holds less than half
its copper, gets a next action. It counts as no work; with
`pour_integrity` set the ladder measures each two-layer attempt the same
way to rank it (below). Against KiCad's own fill (benchmarks/fence/pour-pieces.py) on
the fence's two-layer boards at 34, 40, 44, 46 and 48 mm: the copper
within 2 % (403.5 against 411 mm², 1080 against 1071), the largest
piece within 6 %, the pieces within about 10 % (16 and 16, 48 and 45,
37 and 41, 31 and 29, 41 and 42); four layers, In1.Cu: 1 piece of 2045
mm² against KiCad's 2016 mm² and two slivers of 2 mm².

**Why `plane_cut_cost` did nothing on the fence, and `pour_integrity`**
(2026-10-22). Three reasons. (1) The rung that wins on two layers is
pours as tracks (46 mm, x1404: connected 19 open, skeleton 10, tracks
0), and with its pours not connected a pour masked nothing: `covered`
was built from the nets' connected pour masks only, so `layer_cut` was 1
on every layer and the cut cost never entered its search, its vias or
its corridor plan. (2) Where it did enter (the connected rungs) it is
the same factor on every covered node, whatever the step cuts: a nick at
a pour's rim costs what a cut through the one corridor joining two
halves costs (and `layer_cut` is relative to the least covered layer, so
a board poured on both layers pays nothing). (3) Nothing ranked attempts
by their pours: the ladder kept the fewest open, then starved thermals,
then vias and copper.

`pour_integrity` (router and route config, default 0: off, main's code
path before it; two-layer boards only) addresses each when set (1 is
what was measured):
- a pour not connected covers what KiCad would fill (its masks computed
  for `covered` alone, counted in the build), so the tracks rung pays
  `plane_cut_cost` like the connected ones;
- what a cut severs (`Router::refresh_sever`, router/sever.rs): per
  pour and layer, the fill as the routes leave it (the pour's nodes free
  of other nets' copper) is split into wide cores (its opening by a disk
  of 0.6 mm) and corridors, the corridors cut into chunks of 1.2 mm; on
  the graph of cores and chunks a chunk that is an articulation point
  severs the parts its removal leaves outside the largest, and a step or
  via of another net on its nodes pays `layer_cut` times `1 + 20 *
  pour_integrity * severed` (the share of the pour). A core, a dead end's
  tip or one of two parallel corridors severs nothing. It is recomputed
  after a batch's rip-up (negotiation and clean-up) when copper changed,
  at most at a tenth of the search since the last refresh (`SEVER_SHARE`;
  30 ns a node, `SEVER_NODE_NANOS`, as pour work);
- the ladder ranks equally open attempts (open, then starved thermals)
  by their pours before their vias and copper: each judged attempt's
  board is measured as the verdict's `pours` line measures the final one
  (`measure_pours`, charged as a lowering and a build), its loss
  (`pour_loss`: per pour the share of the zone outside the largest
  piece plus 0.01 a piece beyond the first) in steps of 0.02
  (`pour_rank`); the attempt line shows `pours lost`. The check gate
  lets an attempt as open as the best with better pours be checked
  whatever its vias.

On four layers and more it changes nothing: there the inner planes stay
whole on the rungs that keep signals off them, and pricing them for the
tracks rung cost the expansion tier 197 unconnected items (x1570: 761 to
958; MSign 69 to 137, ZSWatch 3 to 63) and the fingerprints' jetson 20
to 85, Sisu 25 to 52.

Measured at 1 (x1571 against x1539, 2026-10-22), and so left off by
default: it does not keep the
fence's two-layer pour whole. Layouts of the fence (GND on B.Cu):

| board | x1539 | x1571 |
|---|---|---|
| 46 mm (`router-cut10.json`) | complete, 265 vias, 4604 mm; 9 pieces, largest 170 mm² (5 %), copper 13 %, 182 islands, cut 2043 mm | 1 unconnected, 338 vias, 4910 mm; 12 pieces, largest 621 mm² (18 %), copper 38 %, 126 islands, cut 1778 mm |
| 44 mm (free dimension's) | complete, 440 vias; 44 pieces, largest 6 %, copper 37 %, cut 1842 mm | complete, 423 vias; 50 pieces, largest 6 %, copper 42 %, cut 1737 mm |
| 40 mm (free dimension's) | complete, 404 vias; 45 pieces, largest 13 %, copper 39 %, cut 1674 mm | complete, 500 vias; 48 pieces, largest 6.5 %, copper 28 %, cut 1863 mm |

Four times the sever weight (`pour_integrity` 4) or `plane_cut_cost` 30
at 46 mm: 1 and 3 unconnected, 35 and 30 pieces, largest 9 %, cut 2068
and 1933 mm. The other nets' track on B.Cu does not shrink with its
price: the board needs the back for its crossings (1700-2100 mm of it
on every run), and those tracks mesh the fill into 130-180 islands
without GND copper. Pricing what a cut severs moves where the tracks
cross, not how much of the back they take. Route mode, the expansion
tier's 27 two-layer boards (x1571 against x1542): unconnected 431 to
422, vias 2128 to 2153, the pours mixed (better: pdlc 65 to 82 % for
the largest piece, dreamV1_0 85 to 92; worse: MakerTiles' B.Cu 72 to
37 %, HB-UNI-SEN-BATT 1 to 9 pieces). It does not apply to the tier's
19 boards of four layers and more (spliced from x1542); the
fingerprints (Castor, k30-SBC, jetson, Sisu, 1 x 4) are identical.

### Thermal spokes as KiCad counts them

A pad joins a pour through thermal spokes, and KiCad's DRC reports a
starved thermal where it counts fewer than two. The spoke model
(`kicad_spokes`, on; `analyze_pours_full`, `router/fill.rs`) follows
zone_filler.cpp and drc_test_provider_zone_connections.cpp of KiCad 10:

- a spoke runs from the pad's centre to its box grown by the gap, half the
  zone's minimum width and 0.005 mm, and is kept where that end lies in
  the fill (a brush centre within half the minimum width, and the end
  itself keeping every clearance: KiCad tests the point, so an end a few
  microns inside another pad's clearance drops the spoke where a brush
  centre beside it would still fit within the probe's tolerance;
  anthracite's R503.2, 2026-10-21) or in another pad's spoke whose end
  lies in this one (pads side by side);
- it is as wide as the zone's spoke width but no wider than the pad, and
  none is drawn when that is narrower than the minimum width; other nets'
  clearances then cut it, and it counts where a brush still fits from
  the ring half a gap around the pad to its end;
- the DRC ignores spokes into a fill piece that touches nothing but the
  pad ("isolated island"); a piece with any second item (another pad's
  spokes, a track or via of the net, the pad's own track, which touches
  its spokes outside the pad's drill hole: a through-hole pad's fill
  reaches the pad only up to its hole, so a track leaving between the
  spokes touches none of them, `track_meets_spokes`) is no island, and
  connects what it touches (KiCad joins a track to every fill piece its
  copper overlaps, not only at its ends);
- while a pad is short, the DRC counts each track segment connected to it
  that starts within half a gap of it and ends in fill (on any layer, in
  this layer's fill; a kept spoke is fill, as far as other nets'
  clearances leave it), and falls back to the island spokes only when
  nothing else counts.

Whether a point lies in the fill is asked off the lattice (`FillProbe`:
the obstacles and the other nets' routed copper at their exact
distances), since the lattice misses brush centres a fraction of a pitch
from a node. Measured with `experiments/pour-fidelity` (`census.py`,
`spokes.py`, `causes.py`) on 44 routed boards (4274 thermal pad layers,
171 starved findings): the old model flagged 77 of them and 866 pads KiCad
does not; this one flags 116 and 87 (19 more are pads it does not have:
pads sharing a number, custom pads). Thermal reliefs do not cut spokes
(KiCad adds them after the reliefs). Spokes of pads apart meet whatever
their directions (KiCad asks only that each end lies in the other spoke;
a pad doubled at one spot has one set of spokes).

What KiCad's filler and DRC do besides, modelled since (2026-10-14, the
fill2 agent; census: 129 to 146 findings predicted, 28 to 13 missed, 90
to 27 false alarms, pads not modelled 14 to 12):

- spokes in the pad's own frame (`Board::pad_spokes`, `PadSpokes`): turned
  with the pad (0xCB's parts at 45 degrees), at its spoke angle (a
  rectangular pad's `thermal_bridge_angle 45` gives diagonal spokes to its
  box grown by the gap); the routing guards keep the box's axes (in the
  pad's frame they cost Hub 6 connections);
- a pad's clearance override replaces the zone's clearance in the fill,
  also when smaller (`Board::fill_clearance`, in the probe and in the
  static fill map, where overridden pads are stamped last);
- hatched zones (`Plane::hatch`): the spoke's end half a web out, a ring a
  spoke wide around each thermal pad (in the fill the spoke ends are
  tested against and in the final fill), and the holes on KiCad's grid
  (turned about the origin, smoothed, a web left along the fill's edge,
  holes cut small by the edge or around a whole ring filled) for the
  DRC's track ends;
- a spoke cut off from its pad by clearances near the pad still crosses
  the ring, but brings no item to the piece it lands in (k30's U14.12);
- other pads' kept spokes across a pad's ring count as its spokes
  (`foreign_spokes`), the DRC counting every crossing of the fill;
- a track or via is an item of every piece its copper overlaps
  (`track_touch_fill`), each straight run one item; a pad is one item of
  an outline however many of its pieces it reaches; a pad's spokes that
  overlap (all of them without a hole) make the pieces they land in one
  outline, as does a neck between pieces the lattice cuts (`neck_joins`);
- tracks of a pad at the same spot count for both (USB-C A1 and B12);
  pads merged into a larger pad of their number are checked on the
  layers where they are copper of their own (`Board::merged_pads`);
- brush centres keep the clearances to 3 microns (KiCad's pruning
  epsilon and arc approximation), track ends test what clearances leave
  of a spoke anywhere across it, a spoke's end is searched for along the
  spoke (a neck exactly the minimum width), and an end in a sliver the
  lattice misses joins the piece a straight run of fill reaches.

Thermal vias inside an exposed pad (2026-10-14). KiCad's filler treats a pad
inside a larger pad of its net like any thermal pad: it cuts its relief
(inside the larger pad's, so nothing changes there), in a hatched zone adds
its ring (pad + gap to pad + gap + spoke width, clipped only by the zone's
outline, then by other nets' clearances), builds its four spokes from its
centre and keeps them where their ends land in that fill or meet another
spoke; it knocks the plated holes of all thermal pads out of the fill at
the end. The DRC checks every pad by its own ring (half a gap out): the
crossings of any fill outline with it, an outline over the larger pad
holding both pads (no island). So in a hatched zone (0xCB's QFN, U3.45 on
B.Cu) each via has its ring and four spokes into it, and a via at the pad's
corner, whose ring and outer spokes the neighbouring pins' clearances take,
keeps one spoke: starved. In a solid zone the larger pad's relief takes the
vias' spokes, and what crosses a via's ring is the larger pad's own spokes
from its centre: one running right through the ring counts twice, one
grazing it once, none none (no finding). The model: merged pads get
records on every layer, reliefs and rings (`thermal_ring_pads`); spokes of
pads of one terminal meet; a spoke of a pad inside its terminal's pad that
lands counts (the outline holds both pads); foreign spokes across such a
pad's ring are sampled from their centre and count twice when both ends
lie outside the ring; a merged pad's DRC tracks are those whose copper
overlaps its own (KiCad's connectivity: the exposed pad's tracks crossing a
via), and no fill lies in a thermal pad's plated hole (a track to a via's
centre ends in none). The DRC's track-end test keeps other nets'
clearances to half a micron (`TRACK_END_TOLERANCE`) rather than the
spokes' 3 microns: 0xCB's
corner via has a track from it ending 1.5 microns outside a sliver of fill
between two pins. Half a micron is KiCad's DRC epsilon (`GetDRCEpsilon`,
ADVANCED_CFG `m_DRCEpsilon` 0.0005 mm), which its clearance tests subtract
from every clearance; the starved-thermal check itself tests the point
against the fill with none, and the filler prunes with its own micron.
Exact gives two false alarms, a micron loses those four findings. `experiments/pour-fidelity/pads_in_pads.py`
lists the pads inside pads of a board.

A starved via inside an exposed pad gets a track of its own
(`add_inner_spokes`):
`manual_spokes` cannot help it (a merged pad has no lattice nodes), so a
straight track of the net's width runs from its centre into the fill over
the exposed pad, in a hatched zone its own ring, its copper wholly inside
the exposed pad's (every clearance the pad keeps, it keeps), the shortest
of sixteen directions; KiCad's DRC counts a track connected to the via that
starts in its ring and ends in fill as a spoke. It is kept only if the
model then finds the via no longer short, and emitted as exact copper
(`NetState::inner_spokes`).

The fill model reads every zone of a pour net as KiCad fills it, also in
the tracks rung, whose pours do not connect (2026-10-15): a hatched
zone's hatch, its
rings and its outline as in the connecting rungs, for the spoke verdicts,
manual spokes, island vias and stubs and inner spokes there. The DRC's
track rule tests the segments as the router emits them: a straight run
from the pad ends at the first junction, where another branch of the net
leaves it and `materialize` cuts the segment, not at its turn (0xCB's
MX3.S1: the run passed a junction in fill and ended 12 microns inside a
hatch hole, KiCad's segment ended at the junction; reading the hatch
without the cut lost its track spoke, its two island spokes stood in for
it, and no island via was added).

Zones of one net that meet on a layer are checked apart (2026-10-15). KiCad 10 fills each zone on
its own: the fill is built over the zone's outline and the outlines of the
net's zones it collides with (`ZONE::BuildSmoothedPoly`), trimmed to its
own outline, and the outlines of the net's zones of a strictly higher
priority are cut out of it last (`subtractHigherPriorityZones`); equal
priorities overlap. The fills meet at the higher zone's outline, the seam,
with no clearance between them. The DRC tests every zone and layer apart
(`testZoneLayer`): a pad's spokes are the crossings of its ring with that
zone's fill, the isolated islands are that zone's, the tracks those ending
in that zone's fill, and a zone whose fill does not cross the ring is no
connection. So a pad whose ring the seam crosses is checked against each
zone with the spokes on its side (Telemetry's USB shield pads: three
spokes in the whole-board zone of priority 1, one into the edge zone of
priority 11, starved there). The model keeps one fill per net and layer
and counts, per zone of the layer (`Plane::above`, the outlines cut from
it; `Router::in_zone_part`), the spokes whose crossing of the ring lies in
the zone's part (a spoke the seam splits lengthwise counts in both), the
island spokes, other pads' crossings and the tracks ending there, by the
DRC's rule; the worst zone's shortfall (`SpokeRecord::seam_short`,
`seam_zone`) adds to the pad's, and a manual spoke for it must end in that
zone's part. Not modelled: per-zone fill settings (the layer's fill uses
one zone's clearance and minimum width, Telemetry's band zones keep 0.25
mm, the whole-board zone the class's 0.2) and islands the seam alone
makes. Zone outlines drawn with arcs are followed (they were dropped,
`PCB_KICAD_OLD_ZONE_ARCS`): Telemetry's band zones were polygons through
their other corners.

A spoke's clearance test runs from the ring to where the fill's own brush
centres begin, the gap and half the minimum width past the pad, rather than
to its end 5 microns further: Telemetry's D12.1 has a spoke into a strip
exactly the minimum width between its relief and D12.2's clearance, whose
brush centres lie on one line 2 microns short of the spoke's end.

After a layout move's reroute the pours are refreshed (pads the rerouted
signals cut off are routed again), and a plane end whose fill the signals
took is trimmed back to its junction (`trim_dangling_plane_ends`;
USB_Cable_Tester's dangling /SHLD stub).

`render.py` draws KiCad's fill and our map around a pad.

A pad still short after the stitching gets straight tracks into its pour
(`manual_spokes`, on): a track of the net's own class from the pad into a
piece KiCad keeps, which the DRC counts as a spoke; a track keeps the net
class's clearance, often less than the zone's. A pad whose spokes reach
only islands (pieces nothing else of the net touches, typical of inner
planes the signals cut up in the tracks rung) gets, when no straight track
reaches kept fill, a stitching via from the island to the net's main piece
on another layer (`island_vias`), and failing that a straight track of its
own on that layer into the island (`island_stubs`): the island then holds
two items and the DRC counts its spokes. The stub changes no connection
(the island hung on the pad already); it is what KiCad's rule asks for.
k30's tracks rung: 14 starved thermals to 1; mackerel-30's: 19 to 3.
The stubs are off since 2026-10-21 (Florian: they only silence KiCad's
isolated-island finding; `PCB_ROUTER_ISLAND_STUBS=1` turns them on); the
island vias stay on. What that costs: changelog, "starved thermals at
scale".

### Starved thermals at scale (2026-10-21)

In the 5 834-board expansion intake starved_thermal was the second
largest finding type beyond the designer's board (120 boards, 103
blocked by it alone). A census of those 103 routed boards
(`experiments/pour-fidelity/census.py` on the intake's routes) found the
model short of KiCad there in a way the 44-board census was not: of 252
starved pad rows it predicted 69, missed 145 and had no row for 38. The
stripped (empty) boards, censused the same way, split the findings: on
about half the pads KiCad counts fewer than two spokes before any track
is routed (the designer's own tracks gave the second), on the rest the
signals took a spoke. What changed (route mode on the 40 boards of the
103 with a usable licence, 8 x 1 at the intake's 30 s, x1232 against
x1241: 69 new starved findings to 36, 1 board passing to 19, no other new
error, unconnected 4 to 2, vias 1542 to 1532):

- A zone's solid connection is the terminal pad's (`kicad_spokes`). The
  lowering gives zone connections per pad label, and the largest pad of a
  label is the terminal: an exposed pad set to solid made its thermal vias
  solid too, and they had no records (ray4477's U1.33, OpenAir's U5.17 and
  U6.17, starved in KiCad with one spoke each). The pads merged into the
  terminal keep the zone's relief and ring; inner spokes reach them inside
  any pad of their number on the layer (on B.Cu the 2.5 mm pad that is not
  the terminal). A pad is solid where the zones it lies in are: a small
  solid zone of the net elsewhere on the layer no longer makes every pad
  of the layer solid (Apex's GND on F.Cu: C1.2, C2.2 and C6.2 had no rows).
- Another net's zone of a higher priority keeps its clearance
  (`Board::excluded_clearance`): its fill reaches its outline, and this
  zone's brush centres keep the larger of the two zones' clearances and
  half the minimum width from it in the fill probe (the spoke ends, the
  track ends, manual spokes); a keepout's region is cut exactly, as before.
  At a track end of the net the margin is waived (the track keeps the
  other fill away itself: robotique's D3.1). USSTRocketry's LM2576.1
  (/Vin under GND's zone of priority 3) and smkent's LED1.1 had a spoke and
  a manual spoke the model counted into fill KiCad does not have. The pour
  masks, which the routing's pour connectivity reads, keep the old margin:
  with it OpenFC (short tier) lost two connections in its first route and
  robotique's ladder moved to the tracks rung.
- A manual spoke is kept only when the model then counts it, the shortest
  of the first four straight tracks into kept fill, and none passes the
  net's own copper on its way: `materialize` cuts the track at the
  junction and KiCad's DRC tests the segment from the pad, which then ended
  in the relief (1999AZZAR's U2.19: two manual spokes, neither counted).
- A pad the model sees without any spoke (none counted, none into an
  island, none of another pad's across its ring, no track into the fill)
  is no finding to the model; 22 of the 85 new findings on the 40 boards
  were such pads with one spoke in KiCad (USB-C shield and B1/B12 pads,
  buttons). It gets one
  straight track into kept fill (`spokeless_tracks`, on;
  `PCB_ROUTER_NO_SPOKELESS_TRACKS`), which KiCad's DRC counts only while
  the pad is short. Not for pads whose spokes KiCad does not draw at all
  (narrower than the minimum width).
- Spokes the signals cut are given back (`free_lost_spokes`, after
  `free_thermal_spokes`; `PCB_ROUTER_NO_LOST_SPOKES`): for each pad the
  model finds starved, the strips of its failed spokes from the pad's edge
  to the spoke's end plus the fill's clearance; every complete signal net
  with copper within the clearance and half the minimum width of one is
  routed again with the strips closed to it (the thermal guard at 10 000),
  and kept when it is complete and the pour nets' starved spokes go down
  (74th's ESP32-H2 C1.2: two spokes on the empty board, one after a loop
  of signals around it). `free_thermal_spokes` keeps a move only on the
  same condition now (it moved every net crossing a starved corridor of
  the pad's box: Squantor's 10x10 alubox went from 38 vias to 96 when its
  budget was left; with the condition 48).

Left: KiCad spokes the model misses in tight spots (spoke ends against a
track or a neighbour's relief the model finds a few hundredths of a
millimetre too close, spokes of adjacent pads meeting), thermal vias on a
layer where no pad of their number holds them (ray4477's U1.33 on In1.Cu:
merged pads have no lattice nodes for a manual spoke), island-only pads
(tag-designs' U5.6, U5.7) and per-label isolated connections in the
lowering (74th's J3.S1: its SMD pads set to no connection make its
through-hole pads isolated too; a per-pad list from pcb-kicad's lowering
would fix it).

**Starved pads solid by default** (2026-10-21). `solid_starved_thermals`
was opt-in; it is on unless false now. After the ladder, the pads KiCad's
DRC finds starved get `(zone_connect 2)` and the board is checked again
(one more KiCad check, charged as work); the pads whose spokes reach only
an isolated island of fill too (no spoke count in the finding: before
they were skipped, though solid joins the pad to the piece and the
finding goes: mackerel-30-proto's four, KiCad 10.0.7). The solid board is
kept only when KiCad finds no more unconnected items and fewer findings
than before (`solid_kept`): on k30-SBC's v18 route the four solid pads
took the starved thermals away and left J1.39, between J1.14 (solid) and
J1.15 on a 0.635 mm card connector, unconnected twice; the board of before
is restored. What it changes: mackerel-30-proto, Castor (fingerprint
route) and the boards in docs/changelog.md, "net ties, same-number pads,
starved thermals". What it costs: a KiCad check where starved pads are
left, and the designer's thermal relief on those pads (a soldering
choice; false keeps it).

### The pour model against KiCad's islands (2026-10-26)

Four starved thermals of the fine-pitch escapes' tier (BloopBox J1.1,
backplane J4.A1, anthracite-bottom J107.1 and J107.4) and LC_PAMV4b's
J9.1 on main were KiCad errors the router did not prevent. The census
(`experiments/pour-fidelity/census.py`: the model on the board as
written, `dump-pour-map`, against KiCad's own fill and DRC rule,
`spokes.py`) on the tier's routed boards (x14482, both switch settings)
split them: the model saw three of the five starved (J1.1, J107.4,
J9.1: spokes only into isolated islands, the pad's own track the one
spoke counted) and missed two (J4.A1, J107.1: no spoke at all in the
model), and on top it flagged 23 to 26 pads per run that KiCad does not.
What KiCad does that the model did not:

- **A higher zone of another net is knocked out by its fill, not its
  outline.** zone_filler.cpp's `buildDifferentNetZoneClearances` grows
  the higher zone's *filled* polygons by the zones' clearance
  (`ZONE::TransformShapeToPolygon` uses `m_FilledPolysList`), and its
  iterative refill (`ZoneFillIterativeRefill`, on by default in 10.0)
  hands back what the higher zone's island removal deletes. So the lower
  zone fills over its own copper inside the higher zone (anthracite's
  GNDPWR via inside the +5V zone on In2.Cu: a disc of GNDPWR fill that
  J107.1's spoke lands in, one counted spoke, starved) and the gaps the
  higher zone cannot fill (backplane's J4.A1 inside VIN's zone on F.Cu:
  one spoke into such a gap, an isolated island). The model now
  reclaims that fill (`Router::reclaimed_fill`, router/reclaim.rs): near
  the net's own pads, vias and tracks inside a region `Plane::excluded`
  leaves to another net's zone, a node is the net's brush centre where
  no brush centre of that zone (its outline less its own exclusions, its
  static fill map, the routed copper of other nets in its brush's
  occupancy) lies within the zones' clearance and both brushes' halves.
  The pour analysis adds those nodes to the net's fill, the fill probe
  takes them as inside (`FillProbe::reclaimed_at`).
  `PCB_ROUTER_NO_RECLAIM` turns it off.
- **The zone's island removal** (`island_removal_mode`, now
  `Plane::island_min_area`: always, never, or below an area). KiCad
  deletes only pieces with no connection at all; a piece holding a
  track or via of the net is kept even where no pad reaches it, and the
  DRC counts a pad's track that ends over it (anthracite's J107.4 with
  the switch off: three tracks ending, on other layers, over pieces of
  In2.Cu's +5V fill that hold a via; the model, which kept only pieces
  a pad reaches, counted one). A zone that never removes islands keeps
  every piece, one by area those of the area. In the reclaimed fill the
  higher zone's pieces it removes are not its fill (a bounded flood of
  its brush centres, diagonals included; one larger than 20 mm² at a
  0.1 mm lattice is taken as kept).
- **The board setup's minimum spoke count and the finding's severity.**
  The DRC's minimum is `min_resolved_spokes` (KiCad's default 2; 1 on
  17 of the tier's 46 boards, 0 on one), and a project that ignores
  `starved_thermal` gets no finding (5 boards). Every false alarm of the
  census was such a pad: one spoke on a board that asks for one, or any
  shortfall on a board that ignores the finding. The model now takes
  both from the project (`min_resolved_spokes` in the router settings,
  router `min_spokes`, 0 where ignored; `SpokeRecord::required`), and
  everything that counts spokes (the manual spokes, the lost-spoke
  repair, the island vias) asks for that many.

- **A spoke beside a plated hole.** KiCad's spoke runs from the pad's
  centre and the plated hole is knocked out of the fill at the end, so
  what is left of a diagonal spoke beside a large drill reaches nearer
  the centre than the hole's radius along the spoke; a track leaving
  along an axis touches it there and is an item of the piece (the
  fingerprint k30-SBC's C16.1 on In1.Cu: 0.8 mm drill, its stub straight
  down; the model began the spoke's strip at the hole's radius along the
  spoke and saw an isolated island). A track now meets a spoke where a
  point of its centreline outside the hole lies within its half width of
  the whole strip.

**What the router does with it.** The model's starved pads on the board
as written are measured with the pours' pieces after the final route
(`measured_board`, one lowering and one router; no work) and written to
board-router.json (`predicted_starved_thermals`: pad, net, layer,
spokes, island spokes, tracks). The repairs inside the route (manual
spokes into the main piece, island vias to another layer's pour, the
lost-spoke reroute) act on the same model, which now sees the reclaimed
fill and asks for the board's minimum. Where they cannot (J1.1: two
DFN escapes enclose its F.Cu fill; no other layer pours GND) and KiCad
does not check the board (`skip_native_verification`, the expansion
tier), the pads the model finds starved get KiCad's solid connection,
as `solidify_starved_thermals` gives the pads KiCad finds starved on a
checked board, when the project allows it (`solid_starved_thermals`):
the board is measured again and kept when the model then finds fewer
starved pads (`solid_by_model` in board-router.json). On a checked
board KiCad's own findings stay the last resort, as before.

**Measured.** The census on the tier's x14482 boards: with the switch
on, KiCad 5 starved pads, the model 5 of them (was 3) and no false
alarm (was 23); off, 1 and 1 (J9.1) and none (was 26); on the
fingerprints' boards (x14701's routes, the switch off) 1 and 1 (k30's
J1.39) and none (C16.1 before the plated-hole fix). The expansion tier
(x14703, 8 x 1, 30 s; the model's starved count on the final board
against KiCad's starved_thermal findings, board by board): equal on all
46 boards with the switch off and on (0 and 0 everywhere). Before its
solid step the model found 1 pad off (J9.1) and 5 on (J9.1, J1.1,
J4.A1, J107.1, J107.4), the same pads KiCad reported on x14482's
boards. Against x14482 (main's code):

| | x14482 off | x14703 off | x14482 on | x14703 on |
| --- | --- | --- | --- | --- |
| KiCad unconnected | 588 | 589 | 556 | 557 |
| passes (strict) | 21 | 21 | 22 | 24 |
| new errors | 1 | 0 | 5 | 0 |
| vias | 4312 | 4280 | 4295 | 4263 |

The rows that moved: LC_PAMV4b's error goes both ways, BloopBox's and
anthracite-bottom's (on) too, both now passing; backplane keeps 2
unconnected and loses its error; MSign (starved thermals ignored by its
project) 19 to 20 off, 18 to 19 on; every other board routes as before.
The fingerprints (x14703, both switch settings): the model's count equals
KiCad's on all four (k30's J1.39). Left then: on the dense boards the
model predicted 4 starved pads KiCad does not report (glasgow's U1.57,
video's U15.59, U21.7, U21.8); see the next section.

### Thermal vias, custom pads and necks (2026-10-26)

The four pads were taken for thermal vias of exposed pads that KiCad
connects through their vias to an inner plane. KiCad's DRC has no such
path: `testZoneLayer` (drc_test_provider_zone_connections.cpp, 10.0.7)
checks every pad that flashes on the zone's layer and whose zone
connection there is thermal, by the crossings of its ring with that
zone's fill, the isolated islands apart, and its connected tracks; a
via elsewhere on the pad's net is no spoke. What the four were
(kicad-cli's fill and `pcbnew` on the written boards, the model's
`dump-pour-map`):

- **Glasgow's U1.57** is a thermal via, but one set to solid: the
  exposed pad on F.Cu has no zone connection of its own (thermal), its
  25 vias `(zone_connect 2)`. KiCad resolves the connection per pad
  (`EvalZoneConnection`: the pad's own, else the footprint's, else the
  zone's), so the vias are no thermal pads and are not checked. The
  lowering already listed every pad's own setting
  (`Board::solid_pad_obstacles`, `isolated_pad_obstacles`), but the
  spoke model took a merged pad's connection from its label (the
  exposed pad's) and gave every via a relief and spokes. Now a pad
  merged into its terminal that is not the terminal's own copper has its
  own zone connection (`Router::merged_pad_thermal`): set to solid or to
  none it has no spokes and no relief; the label's solid connection
  still joins the terminal (the vias' own layers included) as before.
  The same holds after the solid step on a checked board, which writes
  `(zone_connect 2)` on every pad of a label: 0xCB's U3.45 vias on F.Cu
  were a false alarm on the board as left.
- **Video's U15.59** (a SIMM socket's pin, +5V on In2.Cu): KiCad's four
  spokes all land in islands, and two tracks of the pad (on In1.Cu) end
  inside the pad over its spokes, which KiCad's DRC counts (its fill
  test is a point in the zone's polygons on the layer, any layer's
  track). The model had two of the spokes blocked (another net's track
  cuts them beyond the ring) and so no fill over the pad there. KiCad
  keeps a spoke whose end lands (its filler tests the end alone) and
  cuts the clearances out of it afterwards, which reach no further in
  than the pad's edge: a blocked spoke's part over its pad is fill to
  the DRC's track rule now.
- **Video's U21.7 and U21.8** (a DIP's VPP and VCC pins): each has one
  spoke and one track that ends over a piece of fill KiCad keeps and the
  model took for an island: the lattice cuts it from the piece holding
  copper of the net. U21.7's piece (50 mm²) meets the one with a via
  through a strip 0.26 mm wide between the zone's edge and a /DQ1 via's
  clearance (the minimum width 0.25 mm: its brush centres lie on a line
  10 microns from the nearest node); U21.8's sliver (0.4 mm²) touches the
  pad's own track through a channel 0.08 mm wide between the two pins'
  reliefs. A piece holding nothing is now kept for the track rule when
  brush centres join it to a kept piece (`Router::neck_joined`): from
  its rim, for each other piece with a node within ten pitches, the four
  closest pairs of nodes are tried by a flood of brush centres
  (`FillProbe::fits`) on a grid of a twelfth of the pitch, 8-connected,
  in the pair's box grown by one and a half pitches, until a centre
  comes within the minimum width of the other node (KiCad's pruning
  deflates and inflates by half of it, so two sets of centres that close
  are one outline, and a centre that close to the net's track touches
  its copper); up to six pieces are chained.
- **SNSP-CPU-01's P7.1** (a JST custom pad, circle anchor, polygon
  primitive) came up on the 30 s routes: the lowering gave custom pads
  no spoke frame, so their spokes ran along the axes of the copper's box
  and one landed in an island, where KiCad's run at 45 degrees (the
  anchor's default angle) and land nowhere (no connection, no finding).
  A custom pad without spoke templates gets KiCad's frame now
  (zone_filler.cpp, buildThermalSpokes: the dummy pad's bounding box at
  orientation 0, from the box's centre, turned with the pad;
  `custom_pad_local_geometry`, `PadSpokes::custom`). Pads whose custom
  spoke templates KiCad follows are not modelled (none on the sets).

**Routing keeps its model.** With the first three rules on in routing
too (x14850), the in-route repairs (manual spokes, island vias, the pour
connectivity's spoke islands) changed 7 of 61 routes (the fingerprints'
four identical):
on the dense boards protoesp, DSP-ADAU1452 and split72_left (unconnected
unchanged; on DSP-ADAU1452 a reclaimed piece of GNDD fill inside VAA's
zone, which KiCad's VAA fills, now counted P307.2's second track, so no
island via was placed and KiCad's solid step took the pad instead), on
the expansion tier ZSWatch, AUSIGN, Miniboard (6 to 10 unconnected) and
brickpico (unconnected 557 to 561, vias 4263 to 4252, passes 24 and 24).
So they hold for the measurement only (router `measured_thermals`, on
in `measured_board` and dump-pour-map; `measure_pours`, which ranks the
ladder's attempts by their pours, keeps routing's model), and
`PCB_ROUTER_MEASURED_THERMALS=1` turns them on while routing for a
trial.

**Measured** (route mode, `PCB_WORK_RATE=3.15e6,150e6`, no wall guard;
x1838, main, against x14853; the model's predicted starved pads on the
final board against KiCad's starved_thermal findings, by pad label and
layer; "same route" is the written board byte for byte):

| set | boards | KiCad's starved | x1838 missed / false | x14853 missed / false | predictions changed | same route |
| --- | --- | --- | --- | --- | --- | --- |
| dense, 30 s, 3 x 2 | 11 | 1 (0xCB U3.23 B.Cu) | 1 / 3 (0xCB U3.45 F.Cu, glasgow U1.57 In1.Cu, SNSP-CPU-01 P7.1 F.Cu) | 1 / 0 | 3 | 11 of 11 |
| expansion tier, 8 x 1 | 46 | 0 | 0 / 0 | 0 / 0 | 0 | 46 of 46 |
| fingerprints, a quarter budget, 1 x 4 | 4 | 1 (k30 J1.39) | 0 / 0 | 0 / 0 | 0 | 4 of 4 |
| pour3's dense routes (x14703, both escape settings; the model alone) | 22 | 0 | 0 / 8 (U1.57, U15.59, U21.7, U21.8 twice) | 0 / 0 | 4 | - |
| pour3's tier and fingerprint routes (the model alone) | 100 | 2 (k30 J1.39 twice) | 0 / 0 | 0 / 0 | 0 | - |

The act step (the model's starved pads made solid on a board KiCad does
not check, `solid_by_model`) fires on the same four tier boards as
before (BloopBox's J1.1, backplane's J4.A1, anthracite-bottom's J107.1
and J107.4, LC_PAMV4b's J9.1: connector pins, no exposed pad). An
exposed pad whose vias are set to solid is no longer predicted at all
(glasgow, 0xCB on the dense boards), so on an unchecked board it would
not be made solid for nothing.

On the eleven boards of pour3's full-budget routes (x14703, both escape
settings; the model alone on the written boards, `dump-pour-map` and the
DRC's rule) the four false alarms go (4 to 0 on each). Left: 0xCB's
U3.23 on B.Cu (a QFN pin with a 0.15 mm gap at 45 degrees, in two
panel copies), which KiCad finds starved with one spoke and the model
not: the model blocks a spoke KiCad keeps and counts a track over the
pad KiCad does not (tight spots, "Starved thermals at scale"; on x1838
too; the next section). Not modelled: the zone's `thru_hole_only`
connection (SMD pads solid, the rest thermal; Castor and jetson have two
such zones, whose SMD pads the model takes for thermal) and `no`;
neither changes a thermal via.

### The last miss, and the exact model while routing (2026-10-26)

**0xCB's U3.23 on B.Cu.** The panel holds four copies of the board; the
QFN pin (1.075 x 0.25 mm at 45 degrees, GND, a hatched zone with a
0.15 mm gap, 0.3 mm clearance and a 0.2 mm web) is starved in KiCad on
Board_2 and Board_3, one spoke each and no track counted. The model had
two errors that cancelled on Board_2 and not on Board_3:

- **A spoke end in its own ring, by microns.** The spoke along the pin
  towards the exposed pad ends (gap, 5 microns and half a web past the
  pin) in the pin's own hatch ring, between U3.22's clearance and the
  exposed pad's corner. KiCad's fill ends 0.5 microns short of that
  point (pcbnew's fill, the board as written): the spoke is dropped. The
  model's ring test found a brush centre for it 1 to 2 microns inside
  U3.22's clearance (`in_ring` at the probe's `EPSILON` of 3 microns; at
  1 micron it finds none), kept the spoke as blocked and, by the rule
  for a blocked spoke's part over its pad, counted the pin's own track,
  which ends over the pin there: two, not starved (Board_3; Board_2 had
  the same track and so one, starved by accident). In the measurement a
  spoke's end now lies in a ring only where a brush keeps the clearances
  to a micron (`SPOKE_END_RING_TOLERANCE`, the filler's pruning micron).
- **A spoke cut short of its end that joins the fill beside it.** On
  Board_2 a VCC track runs 0.69 mm below the pin; its clearance cuts the
  diagonal spoke 0.06 mm before the fill's own brush centres begin, so
  `spoke_clear` (a brush across the spoke at every step to there) found
  it blocked. KiCad adds the spoke, cuts the clearances out and prunes:
  the remnant joins the fill to its side and crosses the ring, one
  spoke. In the measurement a spoke that fails `spoke_clear` still
  counts where a flood of brush centres (a quarter of the brush's
  radius apart, 8-connected) from the spoke at the ring, through the
  spoke and through the fill, reaches the fill (`spoke_joins_fill`).

Both hold for the measurement only (`measured_thermals`). With them the
model's U3.23 rows match KiCad's on all four copies (Board_0 no spoke,
Board_1 two, Board_2 and Board_3 one).

**The prediction check** (the model alone on the written boards,
`dump-pour-map` and the DRC's rule, x1840 against x14870; predicted
starved pads against KiCad's starved_thermal findings, by pad label and
layer; missed / false):

| routes | boards | KiCad's starved | x1840 | x14870 | predictions changed |
| --- | --- | --- | --- | --- | --- |
| pour4's dense, 30 s (x14853) | 11 | 1 (0xCB U3.23 B.Cu) | 1 / 0 | 0 / 0 | 1 |
| pour4's tier (x14853) | 46 | 0 | 0 / 0 | 0 / 0 | 0 |
| pour4's fingerprints (x14853) | 4 | 1 (k30 J1.39) | 0 / 0 | 0 / 0 | 0 |
| pour3's dense, both escape settings (x14703) | 22 | 0 | 0 / 0 | 0 / 0 | 0 |
| pour3's tier and fingerprints, both settings (x14703) | 100 | 2 (k30 J1.39 twice) | 0 / 0 | 0 / 0 | 0 |
| this A/B's tier, off and on | 92 | 0 | 0 / 0 | 0 / 0 | 0 |
| this A/B's fingerprints, off and on | 8 | 2 (k30 J1.39 twice) | 0 / 0 | 0 / 0 | 0 |
| this A/B's dense, task budgets, off | 11 | 0 | 0 / 0 | 0 / 0 | 0 |

**The exact model while routing** (`PCB_ROUTER_MEASURED_THERMALS=1`, all
of the measurement's rules: thermal vias by their own connection, a
blocked spoke's part over its pad, necks, custom pads' frame, and the
two above). Route mode, `PCB_WORK_RATE=3.15e6,150e6`, no wall guard; off
is x1840 (main's default), on is x14870 with the variable; one run a
side (a second run of the tier on each side gave the same 46 boards byte
for byte); errors are the strict new findings, starved thermals apart;
work in work seconds:

| set | KiCad unconnected | passes (strict) | errors | starved thermals | vias | work | same route |
| --- | --- | --- | --- | --- | --- | --- | --- |
| expansion tier, 8 x 1, 30 s, off / on | 557 / 564 | 24 / 22 | 0 / 0 | 0 / 0 | 4263 / 4244 | 1143.0 / 1147.1 | 39 of 46 |
| fingerprints, 1 x 4, a quarter budget | 42 / 42 | 1 / 1 | 20 / 20 | 1 / 1 (k30 J1.39) | 1562 / 1562 | 1074.5 / 1075.3 | 4 of 4 |
| dense, 3 x 2, task budgets | 1010 / 1010 | 0 / 0 | 115 / 115 | 0 / 0 | 7429 / 7431 | 11076.0 / 11084.1 | 8 of 11 |

What moved and why (the first spoke verdict that differs, from
`PCB_ROUTER_SPOKE_DEBUG` reruns where they reproduced the runs, else the
two models' verdicts on the written board):

- siderakb's Miniboard, 6 to 10 unconnected (73 to 63 vias): the first
  two rungs route alike; in the narrow-signal step the trouble round
  closed 9 open terminals off and none on (the exchanges' pour checks
  judged differently; the two models' verdicts on the written boards
  are equal, so the pad is one of an intermediate state). A second run
  of each side gave the same boards; the rerun with the spoke debug did
  not (the debug output changes this board's route: 10 off, 5 on), so
  the pad is not named.
- siderakb's facade60_daughterboard, 0 to 2 unconnected, pass lost
  (102 to 94 vias): C15.2's right spoke, cut by a track's clearance,
  joins the fill beside it in the exact model (`spoke_joins_fill`), so
  the routing no longer guards it as blocked.
- marchmare's snaphat main_board, 0 to 1 unconnected, pass lost: the
  mounting holes H26.1 to H31.1 (custom pads) get KiCad's frame, their
  spokes at 45 degrees instead of along the axes.
- AMLing2's rpi_cm5_dronecarrier (the same counts) and AUSIGN (86 to 85
  vias): J6.MP and J16.MP (custom pads, their spokes from the copper's
  centre: the one into the other pad's relief now lands); U8.41's
  thermal vias, `(zone_connect 2)` on the input board, lose their
  reliefs and spokes.
- brickpico (the same counts): H1.1's vias (a pad-and-via mounting
  hole, the footprint's `(zone_connect 2)`) lose their spokes. ZSWatch (the same counts): the written boards' verdicts are
  equal, an intermediate state differs.
- 0xCB (53 and 53 unconnected, 277 to 279 vias): off the routing placed
  manual spokes for C8.2, C9.2 and R6.1 and left no pad starved; on it
  placed U1.1's and KiCad found one of U3.45's thermal vias starved, so
  the solid step set all 60 of U3.45's pads (four copies of 15) to
  solid: the designer's relief is lost.
- video (the same counts): no manual spoke for U18.30 on the router's
  layer 2 (the exact model counts it fed). DSP-ADAU1452 (the same
  counts): none for P503.1 on layer 2.

**The default stays the inexact model while routing** (router
`measured_thermals` off; `PCB_ROUTER_MEASURED_THERMALS=1` for trials):
the tier is worse (7 more unconnected, 2 passes fewer: Miniboard,
facade, snaphat), the fingerprints are identical and the dense set
equal but for vias (+2) and 0xCB's relief. What the inexact model gets
right by accident is conservatism: it gives every thermal via a relief
and spokes and sees a spoke blocked where the clearances only nick it,
so it finds more pads short and spends manual spokes and inner spokes
on them, and its routing guards keep more spoke corridors open. The
exact verdicts take that margin away where the router's later repairs
(the narrow-signal step's exchanges, the manual spokes) relied on it.
The measurement keeps the exact model.

The spoke debug (`PCB_ROUTER_SPOKE_DEBUG`) is not neutral: with it,
Miniboard, ZSWatch and brickpico route differently on both sides
(Miniboard 10 off, 5 on), while the tier without it repeats byte for
byte.

### KiCad's connection width (necks)

KiCad's `connection_width` check (drc_test_provider_connection_width.cpp,
10.0.7) runs only when the board setup has a minimum connection width
(`min_connection`, 0 by default) or a custom `connection_width` rule; the
netclasses have none. Per net and copper layer it unites every zone fill
(teardrops are zones), track, flashed via and flashed pad
(`TransformShapeToPolygon`, `ERROR_OUTSIDE`, `ARC_HIGH_DEF`), fractures
the union and walks each outline (`POLYGON_TEST`, a `VERTEX_SET`): at every
concave vertex it looks for the nearest other vertex of the same outline
that is not its neighbour, nearer than the minimum less twice (DRC epsilon
0.0005 + the board's max error + the fill's extra margin 0.0005 mm), seen
from inside, with the outline bulging further than that in x and in y on
both sides of the cut (`isSubstantial`). Each pair is a finding; its width
is the two vertices' distance, its place their midpoint (not in the JSON
report). Nothing is ignored but rule areas; a pad's own copper is part of
the union. `experiments/pour-fidelity/necks.py` reproduces the check
vertex for vertex on kicad-cli's own fill (`--refill-zones --save-board`)
and names each neck; it agrees with KiCad's report on every board that has
findings (OpenESC's 199 are KiCad's error cap). KiCad's test is local:
it also pairs vertices across a narrow gap between two parts of one
outline.

KiCad's filler removes necks narrower than the zone's minimum thickness
(deflate and inflate, again after other nets' zones are knocked out), so
on a solid zone with `min_thickness` at least the minimum connection width
there are none from the fill. What it does not prune, and what the census
finds: in hatched zones the thermal rings (added after the pruning), the
border it restores around another net's teardrop in the iterative refill
(issue 24758, a band of the minimum thickness clipped to the unhatched
fill), and pad corners against hatch holes; and tracks narrower than the
minimum (a warning on OpenESC). kicad-cli's DRC with `--refill-zones`
also rebuilds the teardrops of every pad that has them enabled, which the
router's pour model knows nothing of.

So the router does not predict them; it repairs them with KiCad's fill
(`necks.rs`, on; `PCB_KICAD_NO_NECK_REPAIR`). When the final DRC reports
connection_width errors (errors only: the check's severity is the
project's, warning on most boards), kicad-cli refills and saves a copy of
the board, the net's copper on the layer (KiCad's fill outlines, the
net's pads, tracks and vias) is drawn on a 4 micron raster tile by tile,
and the passages narrower than the limit that lead to copper wider than
it on both sides are found from the distance map (a pixel whose wider
neighbours form two arcs, one each way along the passage). Over each a
track of the net as wide as the minimum runs along the passage into the
wide copper on both sides, where it keeps the net classes' clearances to
other copper and other nets' fills (their teardrops), the hole clearance
to drills and the edge clearance; shifted sideways or shortened when not.
The board is checked again; the tracks are kept while the necks go down
and no other error type or the unconnected count goes up, all together or,
failing that, by halves (at most six checks). The raster finds the same
necks KiCad reports on WLED_Matrix and USB_Keypad (7 of 7, 12 of 13, 3 of
3, 4 of 4). `pcb-maker repair-kicad-necks <dir> <id>` runs it on a routed
directory; the result's `board-router.json` has `neck_repair`.

The board setup's minimum connection width is also a floor on every track
width the router chooses (2026-10-15): when the project rates
connection_width findings as errors, `min_connection` raises the class
widths, the narrow-signal width and the neck width (`project_rules.rs`,
`KiCadBoardRouterConfig::min_connection_mm`, `apply_width_floor`), and the
lowering raises every class and the neck to it again, so a configuration
file's or a requested net class's widths keep it as well; stubs, escapes,
manual and inner spokes take their net's class or neck width, and the
neck repair's tracks are the minimum rounded up to a micron. Since
2026-10-24 warning severity (KiCad's default) floors as well, and so does
a custom `connection_width` rule (per layer where it has a layer clause:
"Project rules" above). Until then a warning set no floor (a warning
does not fail the pass rule, and on OpenESC, minimum 0.16 over classes of
0.09, the only benchmark boards where it matters, the floor cost routing
in route mode on 2026-10-15: 4in1-mini 52 to 89 unconnected, 4in1 104 to
121); once the widths were per layer the inner layers routed at 0.09 and
KiCad reported 225 connection_width warnings where the designer's boards
have 1 and 0 (changelog, "connection width floor"). `PCB_MIN_CONNECTION_FLOOR=errors`
floors errors only, `off` never; `ignore` never floors. `board-router.json`'s `rules` names the
floor, the severity, the narrowest class width and the neck width the
board was routed with.

### Plane skeleton (fallback)

When a pour ends up shredded by the signal routing, stitching after the fact
cannot repair it (ngdevkit's ground pour: 1536 pieces, 171 stranded pads,
274 stitching vias for six of them). The skeleton secures the pour's
continuity first: the pour net is routed as a plain tree, biased six-fold
onto the layer its pours cover most, against the still empty board, and
that tree is *fixed* for the rest of the run (never ripped up, never the
pour net's conflict). After routing, every part of it that lies in solid
pour copper is trimmed away again, so only the hops the pour cannot provide
remain, and the usual stitching adds what the trimmed pieces still need.
Where the pour alone would have done, the skeleton costs signals room on the
plane layer (dut-c3: 7 more vias, 8 % more copper), so it is the second rung
of the attempt ladder, not the default.

## The attempt ladder

`route-kicad-board ... auto` runs attempts until one is clean (no open
items, no internal violation, no starved thermal in KiCad's DRC) and keeps
the best otherwise: pours connected; pours connected with the plane
skeleton; pour nets as tracks; then the same on finer lattices (0.075 and
0.05 mm) as long as the time projected from the previous attempt stays under
`refine_budget_seconds`. StickHub is the board that needs the finer
rung: 8 open on the regular lattice, clean at 0.075 mm. The default
budget is 240 s, a fifth of `ladder_budget_seconds` where that is more
(route mode: 1200 s by default, so 240 s), and in a layout's ladder 0.4
of the work left before the layout's end where that is more (the share
the narrow-signal step's reserve may take; since 2026-10-21: the fence's
two layers at `total_seconds` 3000 started the ladder with 2241 s left
and stopped at 759 s of work, the 0.075 mm lattice projected at 367 s
against 240; its ladder's budget was also at most 1200 s, now the
layout's work left unless the router config gives `ladder_budget_seconds`).
A lattice not tried is in the verdict's next_actions
(`skipped`) with its projected seconds and the value of
`refine_budget_seconds` that tries it.

Attempts rank by their open count (KiCad's, or the router's where KiCad
did not check), then starved thermals, then, with `pour_integrity` set
on a two-layer board with pours, by how much of the pours they lost
(`pour_rank`, 2026-10-22:
"Why `plane_cut_cost` did nothing on the fence"), then vias and copper.

After the rungs, the finer pitches and the best rung's seed rounds, a
board still open gets one more attempt: the best attempt with signal nets
at the project's narrowest predefined width (its width menu, when that is
narrower than a class; poured nets, grounds and rails keep their class
width), and with the board's minimum clearance when the project rates
clearance findings as warnings; or, with no narrower width, only the
minimum clearance. In route mode the step has what is left of the
ladder's work bound (below; until 2026-10-21 the ladder's budget plus a
half). In the layout's ladder the work end is the ladder's budget, so the
rungs used to leave the step nothing (the A13 module: 126 open to 17 by
the step in one run, not run in the next). There, when the step applies,
the ladder keeps work back for it once its first attempt has run
(`narrow_reserve_share` of that attempt's work, default 0.75, at least
240 s, at most 0.4 of the work left at the start and 600 s, none when
that cap is under the step's 180 s minimum; `PCB_NARROW_RESERVE`
overrides the share). The reservation is a floor (2026-10-15): a further
rung, a finer pitch, the seed retries or a seed round starts only if its
projected work (the last attempt's, times the square of the pitch ratio
on a finer lattice; seeds count once each) leaves all of it, and their
work deadlines end where it begins (seeds side by side each their share).
A board the ladder completes releases it. The step then
runs if half the reservation and at least 180 s are still there, and
negotiates a third of what is left (60-900 s, never more than the
attempt it varies), but at least what the ladder's first attempt
negotiated, within the reservation and half of what is left, with its
router's work deadline at what is left, so it ends inside the layout's
end (`PCB_LADDER_OLD_RESERVE` restores the reservation before 2026-10-15:
at least 120 s, checked against the last attempt's work, the step's
negotiation from what is left only). Its work counts in the ladder's
(`work_seconds`), and when it does not run `skipped` says why. Boards
where the step does not apply, and route mode, are unchanged.

In a layout, a board far from complete after its first route where the
step applies gets the step alone instead of the ladder (2026-10-15): one
attempt with the first route's pour mode, signals narrow, refine pitches
off, negotiating three quarters of what is left after a clean-up capped
at a tenth of it (30-120 s; `direct_narrow_budget`; since 2026-10-21 the
caps grow with what is left, the negotiation at most 900 s or half of
it, the clean-up at most 120 s or a twentieth: unchanged up to 1800 s
left, where a layout of 3000 to 6000 s used to end near its first route
and twice 1020 s), its work deadline the
layout's end; the layout keeps it if KiCad finds it better than the first
route. Far from complete is the old "many open" (more than 5 % of the
terminals, four layers with pours: no moves, no resume, no polish; a
bounded round of moves there, tried on 2026-10-21, only lost time: see
the changelog's fence benchmark pass) or,
where the step applies, more than 2 % (at least 10): jetson's 31 open (its
limit 27) used to send it into moves and a resume (388 s of
work for 31 to 25 open) that left 218 s and no ladder. The ladder's
probes, its continued rung and its reserved step took 900-1000 s on such
boards, and none of its class-width attempts beat the first route
(module, DSP-ADAU1452, vaio_re on x833). `PCB_LAYOUT_OLD_FIRST_ROUTE_BUDGET`
restores the old split.

When that attempt leaves at least 240 s of the layout's work and
connections open, the attempt runs once more in the other pour mode (pour
nets as tracks after pours connected, and the other way round), with the
same split of what is left (`direct_narrow_budget`) and that work as its
deadline and its ladder's budget (until 2026-10-21 at most 1200 s); the layout keeps the better board as before (KiCad's open
items, then starved thermals). vaio_re: 410 s left, the tracks attempt
55 open against 76, KiCad 35 to 21 unconnected. A second seed round, the
class-width ladder and probing both pour modes for the attempt itself
were measured and lost (docs/changelog.md, "after the narrow attempt");
`PCB_LAYOUT_NARROW_CONTINUATION` (stop, tracks, seeds, ladder) and
`PCB_LAYOUT_NARROW_CONTINUATION_FLOOR` choose them for experiments, and
`skipped` says when the attempt did not run.

**The work bound in route mode** (2026-10-21). Without a layout's work
deadline (`deadline_work`), the ladder's whole work stays within
`ladder_budget_seconds` and 5 % (`ladder_work_bound`,
`LADDER_WORK_TOLERANCE` in board_router/budget.rs): the rung probes, the
probed rung's continuation, the rungs, the finer pitches, the seeds and
the narrow-signal and minimum-clearance steps, each router's resolve,
clean-up, via reduction and stitching included. Every router the ladder
starts has a work deadline (router `deadline_work`): the ladder's first
attempt all that is left under the bound; the rungs of a pitch after it an
even share of what is left when the first of them starts (`rung_share`,
decided before they run, so the rungs side by side stay within the bound
together and are still judged in order); seeds side by side a share each;
a step what is left. A negotiation gets at most half of what is left
(`bounded_negotiation`: the floor of 60 s gave a 30 s budget's rungs 60 s
each), the probes together half of the budget (`PROBE_SHARE`), a finer
pitch starts only if its projected work fits what is left, and nothing
starts with less than a twentieth of the budget left
(`bound_allows_attempt`). Where the narrow-signal step applies, the
first attempt's work (at most what is left) is kept for it once that
attempt leaves connections open (`route_step_reserve`), released when a
board completes; the step runs if half of it is still there. Beyond the
bound goes only what a router does past its deadline (a fresh router's
first negotiation iteration, the phases that finish a negotiation the
deadline stopped): on the expansion tier at 30 s (x1261) the ladder's
work was 9-34.2 s, median 31.3, against a median of 52.7 and up to 254 s
before, and no row was cut by the wall guard (13 before); at 300 s
289-329 s. A budget B means at most about 1.15 B of work. Until then a 30 s budget meant 60 s of
negotiation a rung and 75 s a probe, the narrow-signal step had the
budget plus a half, and only the regular ladder's next rung checked the
budget. The layout's ladder (its deadline_work path) is unchanged.

Since 2026-10-21 route mode's ladder also counts each attempt's manual
spokes (the pour analyses and fill probes that repair starved thermals
after the negotiation, `add_manual_spokes`; `manual_spokes_work_seconds`
in board-router.json): no work deadline ends them, and on Vortac's
panels they were 30 s of every attempt beside 5 s of search, uncounted,
so the rungs after the continuation ran and the 90 s wall guard cut
every run. And no step starts with less left under the bound than the
ladder's first attempt took to the end of its first negotiation
iteration (`first_iteration_fits`), which a fresh router does whatever
its deadline: megaduck's narrow-signal step started with 3.9 s left and
did 17.6 s in its one iteration (45 s of work for a bound of 32 s); the
step is then skipped, with why; nor with less than a fifth of the
first attempt's work (since the same day, `step_cost_fits`: "The probes'
overhead" below). `PCB_LADDER_SEARCH_ONLY` (until the ladder review
fixes `PCB_LADDER_UNCHARGED_SPOKES`) and
`PCB_LADDER_NO_FIRST_ITERATION_GATE` restore the count and the start rule
before (for attributing rows). The layout's ladder counts neither.
Since the same day route mode's ladder counts all of an attempt's work
(every pour analysis and fill probe, its lattice's build, its board's
lowering; "All the work charged" below) and gives each router its share
in search work by the routers' proportion of search so far; the first
iteration that a step needs room for includes the first attempt's
lattice and lowering.

**The ladder in code, and its faults** (2026-10-21). The ladder is
`Ladder` in board_router/ladder.rs, its steps in order (`Ladder::run`):
`probes` (the rung probes, their leader's continuation), `rungs` (every
pitch's rungs and the last rung's seed retry, `pitch_skipped` for the
finer pitches' gates), `seed_rounds`, `steps` (narrow-signal, then
minimum clearance). Its bookkeeping (the work, the reserves, the bound,
the best board, the check gate) is the struct's state; the routing,
KiCad's check and the memory measure are a `LadderRunner`, route.rs's
`KiCadRunner` for the board and a scripted one in ladder_tests.rs that
tests the transitions. Every attempt runs isolated (`isolate`,
`catch_unwind`; a thread's join error counts the same): a panic in a
probe, the continuation, a rung, a seed retry or round, or a step is
logged (`ladder: <attempt> failed (panic: ...); the ladder goes on with
its best board`), listed in `attempts_failed` of board-router.json and in
the verdict's `next_actions`, and the ladder goes on with the best board
it holds; only when no attempt gave a board does the run end with an
internal error (exit 4, "no attempt of the ladder produced a board: N
failed (...)"). A panicked probe counts no work (its router is gone); a
panicked continuation leaves the probed rung to the regular rungs.
`PCB_MAKER_TEST_PANIC_ATTEMPT=<kind>[:<n>|:all]` (probe, continuation,
rung, seeds, step) makes the n-th attempt of a kind panic, for tests. A
board beyond the router's maps (more than 65 533 nets, more than 32
copper layers; `pcb_router::grid::check_capacity`) is an input error of
the lowering (exit 2) naming the limit and the count, no longer a panic
in the router (whose nets were limited to 15 bits until then).

**KiCad's checks in the ladder** (2026-10-15). Every attempt routes
without KiCad's check (10 to 50 s of kicad-cli on a big or hatched board;
in a layout counted as modelled work, below); the ladder then checks only
the attempts that could still win (`CheckGate` in board_router/budget.rs). An attempt's open count is the
router's (unconnected terminals and internal violations) plus KiCad's
unconnected items, so the router's count is a lower bound with no slack:
an attempt whose router count is above the best checked board's open
count, or equal to it with no starved thermal on the best and no fewer
vias and copper, cannot win and is not checked. The ladder's first board
is always checked (it is the measure), and so is any board the router
finishes (KiCad's agreement decides whether the ladder stops). Route mode
uses this rule alone, which keeps every decision of the ladder. The
layout's final ladder (`kicad_check_top`, 2) checks only attempts among the
two best by the router's measures (open count, then vias, then copper),
the layout's own board counted first; KiCad decides between the attempts
checked. Past the wall-clock deadline no
further attempt starts, so an attempt is checked only if it could beat
both the ladder's best and the layout's board (`kicad_kept`); a ladder
board left unchecked is never kept by the layout.

**KiCad's checks as work** (2026-10-15, check_work.rs). In a layout every
KiCad check (the board kept after the moves and the polish, each checked
attempt of the final ladder, the neck repair's checks and its KiCad fill)
counts a modelled cost in the layout's work (the neck repair's within the
work left since 2026-10-21: as many checks as it pays for, at least a fill,
a trial and the board as chosen, else the repair does not run and its
note says so; it used to be charged after it, past `total_seconds` by up
to eight checks): `CHECK_SECONDS` (3.6 s) plus
`SECONDS_PER_THOUSAND_HATCH_HOLES` (1.7 s) per thousand holes of the
board's hatched copper zones (area on each copper layer over the square of
the hatch pitch), and since 2026-10-21 `COMPLEX_OUTLINE_SECONDS` (20 s)
for a board with more than `COMPLEX_OUTLINE_ITEMS` (200) graphic items on
Edge.Cuts (the slow plain boards were the complex outlines: katia,
ghoul, ULK, slab-pcb, SmartSpin2k's panel, 15 to 34 s against 3.6), at
most 50 s; the inputs come from the board file before
the layout starts (`layout: a KiCad check counts ...`). The cost does not
depend on whether the DRC cache answered the check, so a warm run ends
where a cold one does; a check the gate skips counts nothing. In
the ladder it is in its work tally (`spent`, the ladder's budget,
`deadline_work`) and in its arithmetic: the work kept for the
narrow-signal step is the step's reserve plus one check
(`reserve_with_check`), an attempt starts only if its projected work and
its check leave the reserve whole, and every router's work deadline (the
rungs', the seeds', the probed rung's continuation, the step's) ends a
check before the ladder's end. The ladder reports it apart
(`kicad_check_work_seconds`, `kicad_checks_charged` in board-router.json),
the layout too (`kicad_check_seconds`, `kicad_checks_charged`,
`kicad_check_work_seconds` in board-layout.json; the verdict's
`kicad_check_work_seconds`; `check work N x S s = T s` on the
`layout: done` line). Route mode (no layout) counts none: its ladder is
unchanged. The model, measured: docs/changelog.md, "Committed on
2026-10-15 (KiCad check work)".

## Via reduction

Once the board is complete, the nets that have vias are renegotiated from the
converged state with the via cost doubled per round, up to three rounds. A
round is kept only if nothing opens and the via count drops; otherwise the
state is restored. This is where most of the via savings come from: the
cleanup pass alone cannot remove a via, because a single net rerouted against
all other copper is boxed in, while renegotiation lets the neighbours move.

## Straightening (2026-10-23)

The search costs `bend_cost` (0.1 mm) per 45 degrees of direction change
against the direction a node was reached from: a direction change, not a
per-step zigzag. But its state is the (layer, node), not the arrival
direction too, and a node is closed at its first arrival, so among the
many shortest paths of an octilinear lattice it often keeps one that
steps sideways and back. The negotiation's detours leave jogs on top, and
the clean-up does not take them out: it keeps a reroute only when it is
strictly shorter (length plus vias), and a path with fewer bends is
mostly as long. score-kicad-board's `aesthetics` counts what that looks
like (docs/quality.md, "Aesthetics"; changelog 2026-10-23).

`straighten` (router/straighten.rs, `Config::straighten`, on by default;
`PCB_ROUTER_NO_STRAIGHTEN` turns it off) runs after the via reduction
and the thermal spoke repairs (which so see the routes as before), as
part of the clean-up's phase. For every complete
signal net (pour nets and fixed branches stay as they are) it lifts the
net's stamps and works on each branch one layer's run at a time: between
two of the run's corners (up to eight corners and 15 mm apart) it tries
the two shortest octilinear paths, the axis part first or the diagonal
part first, and takes one where the run has fewer bends, else fewer
stair-steps, else less turning in all (counted on the window and three
corners either side, by the measure's rule: turns of one sense within
1 mm and at most 90 degrees are one bend, two bends within 2 mm back to
the first direction a stair-step). Every step must be free as the hard
search judges one (the static map and its edge blocks, the other nets'
clearance zones at the node and beside a diagonal, exclusive pours), the
new nodes may touch none of the net's other nodes, branch ends and
junctions stay, and the new path may lie in no more of other nets'
thermal guards and pour pads' spoke corridors (`spoke_corridors`) than
the old one (without the guards the spoke repair after it moved such
paths out again, with vias: BAGEL-MK1 153 to 157 vias; with the guards
alone k30-SBC had a starved thermal more), and on no more of another
net's pour. Incomplete nets are left as they are (jetson: 130 nets
straightened); taking them too is open. A shortest path is never longer and no via moves,
so connections, vias and length cannot get worse. The net is stamped
again. With `pour_aware_polish` the pour nets' stranded pads are
counted before the first change and once after the pass (the pour nets
within a stamp's reach of what changed); where a count grew, the changed
nets go back to their old branches, the last changed first, until none
has grown (counted per net, the pour analyses' work moved the route
ladder's shares: flexypin's tracks rung 20.77 to 19.49 s). Eight step checks count as one
expansion of work. `PCB_ROUTER_FINGERPRINT` prints a fingerprint after
it.

## Fine-pitch escapes (2026-10-25)

A pad too narrow to hold a legal lattice node leaves by an escape stub
(router/escapes.rs, `escape_nodes`): a straight or bent segment from the
pad centre to a nearby legal node, kept as the first piece of copper of
the branch that starts there (the search starts and ends at the stub's
node). At 0.4 mm pitch with 0.2 mm pads, tracks and clearance (RP2040's
QFN-56) every pad of the row needs one even on a lattice aligned with
the pad centres: the centre line keeps exactly 0.2 mm from both
neighbours, and the static maps' margin (`SAFETY`, 2 nm) refuses an
exact fit, so the column inside the pad is blocked and the first legal
node is just beyond the pad's end. The stub along the column is legal to
the nanometre (`stub_is_clear`), as the designer's tracks are.

**What kept the rows open.** A stub stamped the lattice's discs grown by
the error of snapping an off-lattice sample to its node (`snap`, three
quarters of a pitch and a sagitta: 0.525 mm against 0.4 mm at a 0.1 mm
lattice with 0.2/0.2). Neighbouring stubs in a 0.4 or 0.5 mm row are
0.4 or 0.5 mm apart, so each stub covered its neighbours' and every pair
of adjacent escapes was in conflict whatever the negotiation did: the
conflict sits on the pads themselves, which nothing reroutes, and the
hard repair's test of a stub (`escape_is_free`: no other net within one
node of any of its cells) refused them too. Measured with
`PCB_ROUTER_DEBUG` on jfedor2/hid-remapper (x1823, the 30 s task budget,
0.1 mm lattice aligned with U3's and U103's rows, 27 nets open): at the
end of the negotiation every one of the 27 open nets had its conflict
spots on its own fine-pitch pads (the stub's cells), against the nets of
the neighbouring pads: the QFN rows of U3 and U103 (22 nets: QSPI,
USB_DP/DM, GPIO, JACK, UART, SWD, XOUT, SDA, +1V1, +1V2), the USONs U5
and U105 (0.3 mm pads at 0.5 mm, half a pitch off the lattice: oblique
stubs; QSPI_SD1, 1QSPI_SD3 and the far ends of the QSPI nets), the USB-C
connector's 0.5 mm row (1D-) and U3's ground pad 19 between XIN and
XOUT. Not the lattice phase (0.1 mm is aligned with the QFN rows), not
the neck-down (the neck is the 0.2 mm track), not the order.

**The change** (`Config::exact_escape_stubs`; KiCad config
`exact_escape_stubs`; `PCB_ROUTER_EXACT_ESCAPES` overrides either):

- A stub along a lattice line (one segment, parallel to an axis through
  its node: the pad's centre line on an aligned lattice) is lattice
  copper: its nodes, from the one at or beyond the pad centre to the
  escape node, stamp a branch node's footprints, and the hard repair
  tests those nodes alone, exactly as a branch's.
- A stub off the lattice's lines stamps its measured footprint
  (`escapes::stub_footprint`): the nodes closer than the radius to the
  stub, the nearer node of every axis step that passes closer (a step's
  nearest point to an off-lattice vertex may lie between its nodes), the
  via nodes closer than the via radius and the diagonal steps passing
  closer; exactly the radius is legal, as for the discs.
- A pad leaves by straight stubs along a lattice line where it has any;
  bent and oblique stubs only where none leaves. What a bent stub
  reaches, the lattice reaches from the straight stub's node, and the
  bent ones of a pad beside its own net's pad (U3's +3V3 pad 44 into the
  gap to pad 43) had covered the neighbour on the other side.

Stubs off the lattice's lines keep the hard repair's test of their
sampled nodes and the nodes beside them (`escape_is_free`): an exact
test of such a stub against other copper needs where that copper's
centrelines are, which the occupancy (zones, not centrelines) does not
say.

**Measured.** The steps one at a time on the remapper at 30 s (KiCad
unconnected): x1823 33; lattice-line stubs exact 24 (x14470); straight
stubs first 14 (x14471); oblique stubs measured, both ends of a close
step 6 (x14473); the nearer end only 0 (x14474, the change).

*jfedor2/hid-remapper in route mode* (the designer's placement, tracks
stripped, the intake's 30 s of work, 1 x 1; routed / KiCad unconnected /
new errors / vias / length / work):

| | x1823 (off) | x14475 (on) |
| --- | --- | --- |
| 30 s | 93/120 / 33 / 0 / 109 / 2057 mm / 31.9 s | **120/120 / 0 / 0 / 95 / 2548 mm / 31.9 s, pass** |
| 300 s | 102/120 / 19 / 0 / 63 / 2881 mm / 250.2 s | **120/120 / 0 / 0 / 92 / 2520 mm / 67.3 s, pass** |

The designer's board has 100 vias and 1052 segments. Off and on with
one binary (x14476, `PCB_ROUTER_EXACT_ESCAPES=0` and `1`) give x1823's
and x14475's rows; the default-off head (x14477) routes the remapper and
the 22 tier boards that moved exactly as x1823.

*The expansion tier* (46 boards, 8 x 1, 30 s; x1823 identical to
x1821 on every row): 24 boards route identically; KiCad unconnected 588
to 556, passes 20 to 21 (strict too), new errors 3 to 8, vias 4308 to
4291, work 1250 to 1155 s; better 10, same 33, worse 3. The rows that
moved (routed / unconnected / new errors):

| Board | off (x1823) | on (x14475) | Why |
| --- | --- | --- | --- |
| node_v6_0_0 | 55/77 / 32 / 0 | 67/77 / 11 / 0 | 64 SMD pads under 0.35 mm |
| facade60_daughterboard | 41/47 / 9 / 0 | 47/47 / 0 / 0, pass | no pad under 0.35 mm: wider pads that hold no node leave by stubs too |
| ch32v003_rbo12864h-wfi | 19/21 / 2 / 0 | 21/21 / 0 / 0, pass | 28 pads under 0.35 mm |
| 3117_AUSIGN | 69/70 / 1 / 0 | 70/70 / 0 / 0, pass | 68 pads under 0.35 mm |
| rpi_cm5_dronecarrier | 125/129 / 3 / 0 | 128/129 / 1 / 0 | 354 pads under 0.35 mm |
| tuxkeychain, nervous-ecg, OpenRX-Mono, MSign | 11, 4, 12, 19 | 9, 2, 11, 18 | fewer open |
| BloopBox_LED-Board-I2C | 7/8 / 1 / 0 | 8/8 / 0 / 1 | complete; a connector pad's thermal starved by the new route |
| pdlc | 23/33 / 10 / 0 | 23/33 / 10 / 1 | a via 0.16 mm from NT1's net-tie copper (KiCad's hole clearance): the router reads that copper as its net's, an older gap the new route met |
| anthracite-bottom | 139/139 / 0 / 0, pass | 139/139 / 0 / 2 | two starved thermals at J107's PTH pads (In2.Cu) |
| backplane | 77/78 / 1 / 0 | 76/78 / 2 / 1 | one connection and a starved thermal at J4 |
| ED052TC4-Adapter | 36/37 / 2 / 0 | 34/37 / 4 / 0 | its first attempt better (16 to 8 open), the kept result worse |
| bms | 99/99 / 0 / 0, pass | 91/99 / 7 / 0 | the probes rank the plane-stubs rung first (85 against 86 unfinished nets; tracks ranked first before, 81 against 88) and its continuation ends with 9 open |

The four starved thermals are at through-hole connector pads (no
escape stubs); they come with routes moved elsewhere, as do the ladder's
changed choices.

*The fingerprints* (route mode on the v16 placements, 1 x 4, a quarter of
the budget, `PCB_ROUTER_FINGERPRINT`, no wall guard; routed / KiCad
unconnected / new errors / vias / work): unconnected 43 to 42, new
errors 20 and 20, passes 2 and 2 (strict 1 and 1); Castor and k30-SBC
route identically.

| Board | off (x1823) | on (x14475) |
| --- | --- | --- |
| jetson | 320/340 / 20 / 0 / 584 / 315 s | 327/340 / 13 / 0 / 606 / 308 s |
| Sisu | 184/192 / 23 / 20 / 379 / 315 s | 180/192 / 29 / 20 / 373 / 315 s |

**Default: off.** By the rule (unconnected not worse in total, passes
not fewer, errors not more on the tier and the dense boards) the tier
loses on errors (3 to 8) while it wins on connections and passes; the
dense boards and the fingerprints gain a little (8 and 1 connections).
The switch closes the remapper at its task budget. Open: why the
starved thermals and the net-tie via appear (other routes, not the
stubs), and an exact hard-repair test for oblique stubs.


*The eleven dense boards* (route mode on the v19 placements at the task
budgets, 3 x 2, `PCB_ROUTER_NO_WALL_GUARD`; routed / KiCad unconnected /
vias / work): 9 of 11 route identically; unconnected 1020 to 1012, new
errors 115 and 115 (SmartSpin2k's 113 and protoesp's 2 as before),
passes 0 and 0, vias 7283 to 7367, work 11034 to 11092 s.

| Board | off (x1823) | on (x14475) |
| --- | --- | --- |
| poly_kybd_split72_left | 403/420 / 25 / 884 / 1255 s | 413/420 / 9 / 955 / 1191 s |
| DSP-ADAU1452 | 168/190 / 25 / 206 / 1056 s | 162/190 / 33 / 219 / 1178 s |

**Chasing the five new errors** (2026-10-26). The net tie: the lowering
carved a tie's bridge back where its own nets end their tracks, so a via
of a tie net could come within the hole clearance of the bridge's copper
(KiCad's hole_clearance holds against the bridge for every net). The
whole bridge now keeps every via out (an obstacle for vias only, beside
the carved one for tracks; on by default, not part of the switch):
pdlc's error goes, and FUEL4EP's two (a hole clearance and a starved
thermal, the designer's net tie) with it; the fingerprint and dense
boards with net ties (k30-SBC, split72, protoesp, MegaDrive) route as
before. The expansion tier with it
(x14482, the switch off and on; off identical to x1823 on every other
row):

| | off | on |
| --- | --- | --- |
| KiCad unconnected | 588 | 556 |
| passes (strict) | 21 | 22 |
| new errors | 1 (LC_PAMV4b's starved thermal, as before) | 5 |

The four starved thermals left with the switch on (BloopBox J1.1,
backplane J4.A1, anthracite J107.1 and J107.4) are at connector
through-hole pads, where routes moved for reasons unrelated to the
stubs. What the chase guessed at first (a B.Cu track counted as an F.Cu
spoke, spokes off the board's edge) was not it; see "The pour model
against KiCad's islands (2026-10-26)": the model on the written board
already saw J1.1, J107.4 and LC_PAMV4b's J9.1 starved (spokes into
islands, no repair possible: no straight track reaches the main piece,
no other layer holds a pour of the net) but nothing acted on it where
KiCad does not check the board; J4.A1 and J107.1 lie inside another
net's zone of a higher priority, whose fill KiCad knocks out of the
lower zone, not its outline, and the model saw no spoke at all. A
longer thermal guard (the corridor reaching as far as another net's
centreline starves a spoke: the gap, the zone's minimum width and
clearance, half a track; for every pad, then only for pads with two or
fewer spokes that land on the empty board) took the errors with the
switch on from 8 to 5 but moved starved thermals rather than removing
them (ZSWatch and anthracite got new ones) and cost connections with
the switch off (588 to 596, then 606: MSign 19 to 35, OpenDeck 12 to 14);
not kept.

**Default: on** (2026-10-26, with the pour model matching KiCad's
starved thermals, "The pour model against KiCad's islands"). The four
starved thermals the switch brought, and LC_PAMV4b's on main, are now
predicted and connected solid where KiCad does not check the board.
The rule (unconnected not worse, passes not fewer, errors not more on
the tier) holds: the expansion tier on x14703, one binary, the switch
off and on: KiCad unconnected 589 to 557, passes 21 to 24 (strict
too), new errors 0 and 0, vias 4280 to 4263; better 10 boards (node_v6
32 to 11 unconnected; facade60, AUSIGN, ch32v003 and BloopBox to
passes), worse 3 (bms 0 to 7: the probes rank another rung first, as
on x14475; ED052TC4 2 to 4; backplane 1 to 2). The fingerprints (1 x
4, a quarter of the budget): unconnected 43 to 42 (jetson 20 to 13,
Sisu 23 to 29), new errors 20 and 20, passes 2 and 2 (strict 1 and 1),
the model's starved count equal to KiCad's (k30's J1.39, 1 and 1).
The eleven dense boards (route mode on the v19 placements at the task
budgets, 3 x 2, no wall guard; x14703 off and on): 9 of 11 route
identically; unconnected 1018 to 1010 (poly_kybd_split72_left 25 to 9,
DSP-ADAU1452 25 to 33), new errors 115 and 115 (SmartSpin2k's 113 and
protoesp's 2), passes 0 and 0, vias 7359 to 7429, work 11031 to 11076
s. There the model predicts 4 starved pads KiCad does not report, both
ways (glasgow's U1.57 on In1.Cu, video's U15.59, U21.7 and U21.8 on
In2.Cu: thermal vias of exposed pads on inner layers, the limit "Starved
thermals at scale" names); KiCad checks those boards, so nothing acts on
them. The test fixture f1 closes its last connection with it (its
goldens updated). `exact_escape_stubs: false` in the router settings
or `PCB_ROUTER_EXACT_ESCAPES=0` turns it off. Open: an exact
hard-repair test for oblique stubs.

## Pin-row cost (2026-10-23)

score-kicad-board's pin-row crossings (docs/quality.md, "Aesthetics")
count another net's track between two adjacent SMD pads of a part: the
"snaking through SMD pins" reviewers see first. The straightening cannot
take them out (it keeps a run's corners and layer), so the search pays
for them.

**The gaps.** The lowering (`footprint_pin_row_gaps`,
crates/pcb-kicad/src/board_router/lower.rs) lists per footprint and
copper layer with three SMD pads or more the pairs of pads adjacent as
the measure pairs them (centres at most 1.25 times the closer pad's
nearest-neighbour distance apart) and at most 1.5 mm apart
(`PIN_ROW_PITCH_MM`: fine-pitch rows, SOIC and tighter; an SOT-23's
1.9 mm and a 2.54 mm row are ordinary routing), with the two pads'
nets: `Board::pin_row_gaps`. The router (`mark_pin_rows`, router/
build.rs) marks per layer the nodes between the two centres within
half of `pitch * (|nx| + |ny|)` of the line through them (n its normal):
a lattice path changes its distance from the line by at most that much
a step, so every path across the line has a node in the band, one where
the line runs along a node row (pads mostly lie on nodes). The marks are
a sorted list per layer (`Router::pin_row`), and a marked node's
`Hot::pour` index names the triple (guard, cover, gap) instead of the
pair (guard, cover): the search finds the gap where it already looks for
a guard, no map grows.

**The cost.** A step into a marked node by a net other than the gap's
two pads' costs `pin_row_cost` mm on top of its step cost (after the
congestion and layer factors, like a bend); the pads' own nets escape
through their gaps freely. The clean-up's comparison
(`geometric_cost_of`) counts the same per node, so it keeps a reroute
out of a gap when that is cheaper all told and none into one. The
straightening takes a gap node as it takes another net's thermal guard:
a new path may enter none where the old one was not. The ladder's
charging is unchanged (a cost, not work). `pin_row_cost` 0 builds no
marks and routes as before (fingerprints identical);
`pin_row_cost_mm` in the router settings sets it, else
`PCB_ROUTER_PIN_ROW_COST`.

**The default, 2 mm** (changelog 2026-10-23, "pin-row crossings"): on
the expansion tier (route mode, 30 s, 8 x 1, one binary at 0 and 2)
unconnected 625 to 592, passes 19 to 20, new errors 3 both, vias 4223 to
4308, length +0.3 %, crossings 211 to 197 (those within 1.5 mm 59 to
48). From 3 mm on (3, 4, 6 and 20 measured) two boards gain a starved
thermal (ZSWatch's X403 or X602, anthracite-bottom's J106: PTH pads of a
pour net, a pour-as-tracks GND track wrapped round them): a step in
another net's thermal guard costs `thermal_guard_cost` (10) times the
step, about 1.25 mm at 0.125 mm, so above 2 or 3 such steps a gap costs
more than a guard, and the spoke model passes what KiCad's refill
starves. Most crossings left are wider pairs, not charged: at 20 mm the
crossings within 1.5 mm fall 58 to 15 but those of 1.5 to 2.6 mm stay
116 to 121. Charging those too (2.6 mm at 2 mm cost) took 211 to 154
crossings but cost 46 unconnected (629 to 675) and a pass. At the task
budget on the designer's placement (the eleven boards furthest over the
designers' crossings) 2 mm took crossings 452 to 410 and 6 mm 437 to 365
(of the ten run at 6), but SNSP-CPU-01 went 6 to 11 unconnected (22 at
6 mm) and video 0 to 1.

## Differential pairs (2026-10-23, phases 2 to 4 2026-10-24, polarity and skew 2026-10-25, runs not routed and return vias 2026-10-26)

`diff_pairs` in the router settings (route mode). **Default by need**
(phase 3): on where the project's custom rules ask for pairs, that is
where a `diff_pair_gap` or `diff_pair_uncoupled` constraint surely holds
for a net KiCad pairs on some layer (`KiCadDiffPairRule::asked`,
`KiCadBoardRouterConfig::diff_pairs_choice`), else off. The settings'
`diff_pairs` (true or false) always wins, then `PCB_ROUTER_DIFF_PAIRS`
(`1` or `0`); route-kicad-board alone applies the default
(`diff_pairs_by_need`), a layout's routes keep it off. The net classes'
diff pair width and gap do not ask: KiCad's DRC holds the class's gap
only as the pair's minimum (no finding for an uncoupled pair), most
projects carry KiCad's defaults (0.2 and 0.25 mm), and 12 of the
expansion tier's 25 boards with pairs set other values in a class (a USB
class),
among them those where the switch cost connections in phase 2 (AMLing2's
drone carrier, ausign, snaphat, ducktop2's bms). The code: crates/pcb-kicad/src/board_router/diff_pairs.rs, the rules in
project_rules.rs (`diff_pair_of`, `diff_pair_rules`) and
project_rules/dru.rs (`track_constraint`, `inDiffPair()`). Phase 1
(2026-10-23) routed the coupled runs first, in a router of their own,
and kept them as fixed copper; phase 2 (2026-10-24) makes each link a
net of the route itself, negotiated with every other net, and offsets
it into the pair's two tracks when the attempt is written.

**Pairs.** KiCad's convention (`DRC_ENGINE::MatchDpSuffix`): past
trailing digits and underscores a net name ends in `+`/`-` or `P`/`N`,
and the net with the other suffix exists: `D+`/`D-`, `USB_DP`/`USB_DN`,
`TX_P_1`/`TX_N_1` (not `USB_DM`). Per copper layer the pair's width is
the custom rules' `track_width` for a track of the pair there (the last
rule whose condition surely holds, `inDiffPair()` evaluated: KiCad's
`EvalRules`), else the class's `diff_pair_width`, else the net's width;
its gap the rules' `diff_pair_gap`, else the class's `diff_pair_gap`,
else its clearance (a rule's opt, else the class value held inside the
rule's min and max), never below what KiCad's clearance check lets the
two nets come (the class's diff pair gap where narrower than its
clearance, its implicit `isCoupledDiffPair` rule, else the clearance);
`diff_pair_uncoupled`'s max is read. The custom rules come first, as in
KiCad (a custom rule replaces the class's implicit diff pair rule). A
pair uses the layers with its first layer's width and gap (ducktop2's
DIFF_85: 0.183 mm on the outer layers, 0.114 on In2.Cu, so not In2).
The coupled runs take the pair's width; the pair's nets elsewhere (the
stubs, a pair that falls back) take it only where a custom rule sets it
(KiCad checks that one on every track of the pair; the class's
`diff_pair_width` only KiCad's interactive router takes; phase 1 widened
every pair's nets to it, which alone moved iorodeo from 87 to 93
unconnected). With the switch on, the custom rules a pair takes
(diff_pair_gap, diff_pair_uncoupled, track_width on `inDiffPair()`) are
reported `applied`.

**Pair points and links.** A positive and a negative pad of one part
(the labels' reference), at most 3 mm apart, nearest first, with no
other net's copper on the line between their centres (a GND pin between
D+ and D-, or an ESD part's pins across its body, are no pair point),
are a pair point. So is (phase 4) a pad of each of two two-pad parts
whose other pads lie as close (at most 3 mm: the pair's AC-coupling or
series capacitors, resistors, ESD diodes side by side), taken after the
pair points of one part: the pair couples from its connector up to the
capacitors, and the nets past them (another pair by name) from the
capacitors on (`series_parts`). A minimum spanning tree links the pair
points; pair points of one part (a USB-C receptacle's two rows) are
joined first, at no cost, and by ordinary routing: only links between
parts are coupled (phase 1 coupled the two rows of one receptacle, whose
launches face each other). A pair with no link falls back.

**Launches.** Per link end the search goes out from between the two pads
along the pads' normal, turned away from the part's middle (the middle
of its pads' copper; toward the link's other end where the part does
not say), and turned from it by up to a right angle in 15° steps, in
0.05 mm steps up to 4 mm; per direction the first *run start* where the
stubs' ends beside it (an offset to either side, each clear of every
other net's copper and of both pads by the pair's gap: at a crossed end
the nets swap sides), the run's straight lead (at least 2.5 offsets,
long enough that the link's copper keeps its clearance from the stubs'
ends) and the envelope at the lead's end (with 0.15 mm for lattice
nodes) are clear of every obstacle and the edge. Of the directions the
one closest to the pads wins, a right angle costing 1 mm. The lead and
the launch are on one layer. On the expansion tier no link fails for
want of a launch any more (phase 1: 30 of 60 pairs). **The link's own
stubs (phase 4).** A launch where straight stubs of the pair's width
from the pads to the stubs' ends keep every clearance (another net's
copper, keepouts, holes, the edge; the pair's other net its gap; the two
stubs at least `w + min(g, c)` apart; `stubs_clear`) and where the
envelope is clear along the whole lead wins over a closer one without:
the link carries those stubs, the router does not route them. A stub
starts where the line from the pad's anchor to its end leaves the pad,
less half its width (a card edge's 2.4 mm finger reaches under the
edge's mask opening; a mask opening over the pad is left by the stub, as
the router's `stub_is_clear` leaves it). At a *crossed* end (the pads'
order there against the run's: the positive pad on the side the run
gives the negative net) the stubs must cross, which only the router
can (with a via): that end's stubs stay the router's, its launch the
closest; the second end, unless only the second end's stubs are
straight.

**The links in the negotiation** (`plan_pairs`, after each lowering of
a rung's board). Each link is a net of the route: its track the pair's
envelope (`2 w + g`), its via a disc holding two vias side by side as
far apart as at a right-angle turn (`2 dv / 0.7 + V`), a class per set
of pair layers (the widest envelope, clearance and via of its pairs;
`RuleClass::layers` keeps its tracks on the pair's layers: the static
maps block the others), its terminals the two launches (a contact of
0.15 mm), its copper the lead past the stubs' ends (other nets keep
clear of it). Each of the pair's nets gets a terminal at each stub end
(its copper the first track width of the run), the two ends of a link
merged into one terminal (`Board::terminal_copper`: a route may end on
either), so the router routes the stubs from the pads to the run's ends
as ordinary connections, with its pad exits, necks and vias (a crossed
end's stubs cross with a via). Where the link carries the stubs (phase
4) they are copper of the pair's nets (other nets keep clear), the pads
are of the run's terminal (no stub to route), and the link's copper is
the lead from the run's start (no other net between the two tracks'
beginnings: m2_card's ground vias sat in the lead's first half and
refused its runs). The router does not neck a link down near its
launches (`Router::prepare_net` skips "pair launch" pads): the lead, the
launch terminal's pad, is exactly as wide as the link's track, narrower
than track and clearance, so the link routed at the neck width (0.127
mm) for 1.5 mm from each launch, and the pair's offset tracks there
came within a ground via's clearance (m2_card: 0 of 9 links coupled,
then 9 of 9). The plan rides along in `Board::attachment`; the router
does not read it.

**Writing an attempt** (`finish_pairs`, in `write_attempt`, after
tightening). Each routed link's path (its route from launch to launch,
the route's own stubs to the launches dropped) gets the run start in
front of each end and half the lead straight along the launch's
direction, so that the tracks begin beside the stubs' ends; the
centreline is offset by `(w + g) / 2` to each side per layer run (mitred
inside a bend, bevelled outside), the positive net on its pad's side of
the first end; at a layer change each net gets a via `dv` (up to
`dv / 0.7`) out along the bisector; short jogs join the tracks to the
stubs' ends where those lie further out (the ends keep the router's
clearance, wider than a gap below it); where the link carries the stubs,
they are part of its copper. The links' copper is checked
with `verify` against the board and every route near it (a finding
between a pair's own two nets counts only below their gap); a link with
another finding is refused. What holds is written as tracks and vias of
the pair's nets and is kept copper of the board the report is made on;
then each kept link's routed stubs are replaced by straight tracks from
the pads where those hold (the router's stubs leave the pads parallel at
the pads' pitch, which KiCad takes for a coupled piece out of the gap's
range). The links' nets and the stubs' end terminals go; the router's
diagnostics name the pair's nets for them. A link not routed, refused
or crossed leaves connections open: its stubs go (and after the
fallback what is left of them dangling, followed from the stub's end) and
the *fallback* routes the pair's nets with every other route kept (the board's
classes; 0.05 of the ladder's budget, at least 1 s,
`PCB_ROUTER_DIFF_PAIRS_WORK`; its work charged to the ladder as search).

**Layers (phase 3).** With the switch on, a pair's nets route in a
class kept to their layers (`RuleClass::layers`, set when the board is
lowered): those where no custom rule disallows their tracks (`disallow
track`) and where the rules' track width bounds hold the width they
route at (a class has one width: ducktop2's DIFF_90 is 0.1796 mm outside
and at most 0.111 on In2.Cu, so its nets keep to F.Cu and B.Cu). Phase 2
kept only the links' class to the pair's layers; the pairs' nets kept the
class they were lowered with, so their stubs and the fallback routed
them anywhere, and the fallback, which drops every other net's plane,
also dropped the exclusive ones: on ducktop2 the pairs' nets crossed its
solid ground layers (In1, In3, In6) and its impedance rule's In4 and In5,
items_not_allowed +89. The fallback now keeps other nets' exclusive
planes as keepouts for tracks (vias pass). Keeping the nets to the layers
their rules allow first moved them onto In2.Cu at the outer width (left_io:
37 track_width errors); hence the width bounds.

**A clean fallback (phase 3).** A link is kept only when each of its
pads' stubs reaches its end of the run (`stub_joined`: the router merges
a link's two ends into one terminal, so it may route a pad to the other
end, or leave it open at the budget, and phase 2 then wrote the run as an
island: left_io's J22_TX1 pair was "coupled" with neither pad joined);
otherwise it falls back as a refused link ("a pad's stub does not reach
the run"; 30 of the rule boards' 97 links with x1797). A kept link's stub that ends
elsewhere on its end's copper gets a track to that end, where the run's
jog starts (`join_stub_ends`; else KiCad finds the jog dangling). After
the fallback, a failed link's stub is pruned from its end past a via
(`prune_stub`), then every fallen-back pair's nets lose what joins
nothing (`prune_dangling`: a track with an end that overlaps no copper of
the net, a via on no copper, a piece no longer than its width whose end
lies in no other copper where removing it splits nothing). A via with
copper on one layer stays: KiCad's teardrops may join it (m2_card's
CLOCK+ lost a connection when such a via went).

**No second route (phase 3).** Keeping the better of the pairs-on board
and a pairs-off board would need a second full route of the board (the
fallback routes only the fallen-back pairs' nets, with everything else
kept); it costs as much as the route itself, so it is not done. A refused
or failed link falls back as above instead.

**The verdict.** `diff_pairs` in `board-router.json` and the verdict:
`on`, `default` (`rules`: the project's rules ask; `off`: nothing asks;
`explicit`: the setting or `PCB_ROUTER_DIFF_PAIRS`), `reason` (the rule
and the net it holds for, or why nothing asks; route mode reports it on
every board), and with the switch on `found`, `coupled` (every link), `partly_coupled`, `fallen_back`,
`links`, `links_coupled`, `crossed`, `crossovers` (2026-10-25), `coupled_mm`, `uncoupled_mm`
(after the route, per coupled pair the longer net's tracks less the
coupled run: KiCad's measure), `skew_mm`, `meanders`, `skews` (2026-10-25,
"Skew and length" below), `not_routed` (2026-10-26: the links whose run
was not routed, by cause; "Runs not routed at the task budget" below),
`pair_vias`, `return_vias`, `return_net` (2026-10-26, "Return vias"
below), `work_seconds` (the fallback's), and in
board-router.json per pair its nets, width, gap, source and why links
fell back (`fewer than two parts with the pair's pads side by side`,
`the launches are too close for a coupled run` (under 1 mm),
`the coupled run was not routed (<cause>)`, `a pad's stub does not reach the run`,
`its copper is ... from ...`).

**Measured** (route mode, 30 s, 8 x 1; off x1753, main at 4d9965e; on
x1766): docs/changelog.md, "Committed on 2026-10-24 (differential pairs,
phase 2)", and phase 1's in "Committed on 2026-10-23 (differential pairs,
phase 1)". The expansion tier's 21 boards without pairs route as with the
switch off (x1762, identical rows); on the 25 with pairs unconnected 556
-> 576, passes 9 -> 7, new errors 1 -> 3; the rule boards' gap findings
113 -> 83: the switch stays off. The
launches work (no "no launch" fallback; 55 links, 16 coupled); what
costs is the negotiation itself: on MSign's mainboard six
link nets of the envelope's width in the dense USB corner leave its
power nets (VDD, +5V, VBUS) tens of terminals short at 30 s although
only two links couple (19 -> 75); on AMLing2's drone carrier (3 -> 9) the CSI
bundle at the CM5 connector: six of twelve link runs not routed, three
refused; on ducktop2's bms (0 -> 5) the one current-sense link is
refused and the board's other nets lose what the negotiation spent on
it. A link that fails costs more than its pair: the board's negotiation
converged around it.

**Measured, phase 3** (route mode, 30 s of work, one thread, the
board's own tracks kept, KiCad's DRC with `--refill-zones`, as phases 1
and 2; off x1791, main; on x1797, the default by rules): on the 7 boards
with diff_pair_gap rules KiCad's errors 601 -> 186, gap findings 114 ->
74, unconnected 1 283 -> 1 262, items_not_allowed 20 -> 0 (no board
worse; off routes ducktop2's pair nets onto its disallowed layers too),
vias 1 096 -> 966; 154 pairs, 3 of 97 links coupled. Most of the gain is
the pairs' nets at the rules' width (oculink: track_width 454 -> 105) and
on the layers the rules allow, not coupling. The expansion tier's 46
boards (no board's rules ask) are identical to x1791, rows and routed
boards byte for byte: the default flips by rule. Tables: docs/changelog.md,
"Committed on 2026-10-24 (differential pairs: on where the rules ask; the
disallowed layers)".

**The coupled share (phase 4, 2026-10-24).** On the 7 rule boards
phase 3 coupled 3 of 97 links. Per fallback reason, in the order of the
brief (route mode, 30 s of work, one thread, the board's own tracks
kept, KiCad's DRC with `--refill-zones`; no setting, so the default by
rules fires on all 7; x1798 is main):

| step | snapshot | links coupled | KiCad errors | unconnected | gap findings | dangling | vias |
|---|---|---|---|---|---|---|---|
| main | x1798 | 3/97 | 186 | 1 262 | 74 | 40 | 966 |
| 1 the link's own stubs | x14240 | 11/97 | 151 | 1 258 | 84 | 39 | 970 |
| 2 refusals: no neck at a launch | x14241 | 19/97 | 98 | 1 232 | 73 | 39 | 965 |
| 3 runs not routed: links first (not kept) | dev | 19/97 | 103 | 1 249 | 74 | 37 | 958 |
| 4 series parts as pair points | x14243 | 22/150 | 96 | 1 227 | 69 | 40 | 981 |

(1) *A pad's stub does not reach the run* (30): the router merges a
link's two run ends into one terminal, so a pad's route may end at the
other end, or the pad is dead (m2_card's J1, 5 mm by 0.2 mm pads at 0.4
mm pitch: at the pair's width no node in the pad, and the run's end
0.3 mm before it). The link now carries straight stubs where they fit
(above, "The link's own stubs"; the pads join the run's terminal) and
reserves its lead from the run's start. Left: 19 (26 with step 4), at
ends whose stubs stay the router's: in a debug run identical to x14243,
97 of the 276 pad pairs at a link end had no launch with straight stubs
(95 of them on the ducktop2 boards), and 35 were planned again as a
crossed end (or for launches too close), whose stubs the router must
cross, on fine-pitch pads too (oculink_port's RX pairs, J2 at 0.4 mm). The `crossed` count was inverted until now (it
counted the ends whose order matches): phase 2's and 3's crossed links
were the uncrossed ones. (2) *Copper refused* (14 after step 1): the
offset tracks came within a ground via's clearance beside the launches;
the link's route there was 0.127 mm wide, not the envelope's 0.54 (the
router necked it down near its launch terminal, whose pad, the lead, is
narrower than track and clearance). The router no longer necks at pair
launches: refused 14 -> 2, m2_card 3 -> 9 of 9. (3) *Run not routed*
(48): the link nets fail even in the hard searches (oculink_m2's ten 96
mm links from F.Cu to B.Cu), not for want of priority: putting them
first in the routing order (`routing_order`) left 19 coupled and cost 17
connections; a smaller via disc (`2 dv + V`, no turn allowance) coupled
one fewer and refused 5 more. Neither kept. (4) *Fewer than two pair
points* (65, ducktop2's pairs through AC capacitors): two two-pad parts
side by side are a pair point (above, "Pair points and links"): 65 ->
22, 97 -> 150 links. Requiring the parts parallel as well (x14242: 138
links, 24 coupled, errors 88) cost 18 connections on the ducktop2
boards: not kept.

| board | gap/uncoupled/length x1798 -> x14243 | errors | unconnected | vias | dangling | links coupled/total | uncoupled mm |
|---|---|---|---|---|---|---|---|
| ducktop2-center | (20, 0, 0) -> (14, 0, 0) | 20 -> 14 | 499 -> 499 | 479 -> 465 | 0 -> 1 | 2/28 -> 5/54 | 5 -> 12 |
| ducktop2 left_io | (16, 0, 0) -> (15, 0, 0) | 16 -> 15 | 351 -> 356 | 235 -> 239 | 1 -> 0 | 0/27 -> 0/41 | 0 -> 0 |
| ducktop2 right_io | (25, 0, 0) -> (38, 0, 0) | 27 -> 40 | 185 -> 166 | 124 -> 154 | 1 -> 3 | 0/12 -> 1/23 | 0 -> 1 |
| OpenAIO-Whoop | (1, 0, 1) -> (1, 0, 1) | 0 -> 0 | 187 -> 183 | 35 -> 36 | 0 -> 0 | 0/2 -> 0/4 | 0 -> 0 |
| oculink m2_card | (0, 0, 0) -> (1, 0, 0) | 71 -> 2 | 10 -> 2 | 21 -> 22 | 3 -> 0 | 0/9 -> 9/9 | 0 -> 10 |
| oculink_m2 | (12, 0, 0) -> (0, 0, 0) | 23 -> 4 | 14 -> 9 | 58 -> 47 | 1 -> 2 | 1/10 -> 4/10 | 2 -> 8 |
| oculink_port | (0, 0, 0) -> (0, 0, 0) | 29 -> 21 | 16 -> 12 | 14 -> 18 | 34 -> 34 | 0/9 -> 3/9 | 0 -> 8 |

Totals: links coupled 3/97 -> 22/150, errors 186 -> 96, unconnected
1 262 -> 1 227, gap findings 74 -> 69, dangling 40 -> 40, vias 966 ->
981. The fallbacks left with x14243: run not routed 80, a pad's stub
does not reach the run 26, fewer than two pair points 22, launches under
1 mm apart 11, no launch 7, copper refused 4. The ducktop2 boards stay
far from complete at 30 s (499, 356, 166 open) and move by ten
connections either way with any change to the plan; their links mostly
fail with the board (49 of 54, 41 of 41, 22 of 23 not coupled). The
expansion tier's 46 boards (no board's rules ask) are identical to x1798,
rows and routed boards byte for byte. Tables: docs/changelog.md,
"Committed on 2026-10-24 (differential pairs: the coupled share)".

**Polarity (2026-10-25; off by default).** A link is *crossed* where the
pads' order at one end is against the run's (the positive pad on the side
the run gives the negative net). Two remedies exist. *A swap at the part*
(P and N exchanged on the part's pins where the schematic allows it) needs
swap data: KiCad has none, and the one hook here (pin-swaps.json,
docs/pin-swap.md, layout mode) is set on none of the rule boards, so no
swap is made. *A crossover in the run* is built: at the crossed end's
launch (`crossover`, `Launch::crossover`) the stubs' ends keep the pads'
order (the stubs do not cross, so the link can carry them straight), one
net (the *straight* one) runs on from its stub's end, the other *dives*:
out by what its via needs (a via's radius plus the clearance, or the
drill's radius plus the hole clearance, plus half the width, from the
straight track), a via, a track across under the straight net on the
nearest other layer of the pair's, a via on the straight net's far side,
back on the launch layer two offsets beside it. The run starts past it,
its centreline beside the straight track (the run's lead and envelope
checked there as at any launch). On the launch layer every track of the
diving net lies two offsets (a gap) from the straight one, so KiCad's gap
check (which pairs any P and N segments whose projections overlap with
nothing between, KiCad 10's drc_test_provider_diff_pair_coupling) sees the
gap; where the link carries the stubs their ends lie an offset from the
centreline (not the router's wider end offset), so the tracks before the
crossover are a gap apart as well. Either net may run straight, the first
that fits; the crossover's copper (tracks, both vias on every layer with
their holes; hole to hole and hole clearance from other holes and copper)
keeps every clearance, and is copper of the pair's nets in the plan (other
nets keep clear of it). The second end gets it, else the first; an end
whose crossover leaves both ends' stubs to the link first. With it come
two changes the crossed links needed once they reached the copper check:
the link's path loses its hairpins (`despiked`: a point where the path
turns back by more than a right angle goes, two turns the same way with
less than 2.5 offsets between them meet at the lines' crossing; the raw
path is the alternative where that copper is refused), and a kept link's
vias keep their holes' clearance in the fallback's board (the fallback
routed beside them at the copper clearance: hole_clearance errors on
oculink_m2). Verdict: `crossovers` (coupled links through a crossover).
`PCB_ROUTER_DIFF_PAIRS_CROSSOVER=1` turns all three on.

**Skew and length (2026-10-25; on with the pairs).** After the fallback,
per pair with a kept link (`match_skew`), the two nets' lengths from their
copper: the routes' and the kept links' tracks, and each via at the
stackup's height between the outermost layers of the tracks that end on
it (`copper_heights`: the board's `stackup`, each copper layer at its
middle; else the board's thickness spread over its copper layers), as
KiCad measures with the stackup's height. The offset tracks are measured,
not the centreline: at a bend of θ the outer track is longer by about
`2 d tan(θ/2)`, and a crossover's diving net carries two vias. The target
is the custom rules' `skew` maximum for the pair's nets (the least over
the layers; `project_rules`, `KiCadDiffPairRule::max_skew`), else the
pair's gap; a `length` rule's minimum (or its opt) is a target length for
both nets. Where the nets lie further apart than the target (or a net is
shorter than the length), the shorter net gets a meander on a straight
track of its coupled run: rectangular bumps with 45° corners away from
its partner, `max(w + c, 2 w)` wide and apart, at most `max(3 w, 2 g)`
high, as few as hold the length and their height bisected so that they
add exactly what it lacks; the longest straight tracks first, per track
the meander near its start, in its middle, near its end, the first whose
copper keeps every clearance (`refused_groups`); none where none fits.
The meander's tracks are written as a KiCad tuning pattern (`generated`,
`tuning_pattern`, `diff_pair_skew`): KiCad 10's gap check skips a tuning
pattern's tracks (their bumps would otherwise be P and N pieces out of the
gap's range). KiCad's own `skew` rule compares every net the rule matches
(without `within_diff_pairs`), so a rule over a class compares more than
one pair; the pass matches each pair. KiCad has no skew check without a
rule; the gap as the default target is this router's. Verdict and
board-router.json: `skew_mm` (the largest skew of a pair with a coupled
link, from the written board), `meanders`, and `skews` (per such pair its
nets, both lengths, the skew, the target and what the meander added; the
verdict keeps it, unlike `pairs`); per pair in board-router.json
`skew_mm`, `skew_target_mm`, `meander_mm`. `PCB_ROUTER_DIFF_PAIRS_SKEW=0`
turns the meanders off (the skew is still measured).

**Measured (2026-10-25).** The 7 rule boards, route mode, 30 s of work,
one thread, the board's own tracks kept, KiCad's DRC with
`--refill-zones`, no setting (the default by rules fires on all 7); unlike
the phases before, the wall guard raised to 1200 s (`deadline_seconds`)
on both sides: at 90 s the ducktop2 boards were cut by the wall clock on a
loaded machine (x1820's ducktop2-center 96 s of work against 419 s uncut)
and their rows depended on the load. Off side x1820 (main); the crossovers
with x14400 (the code before the default flipped: crossovers on, meanders
off with `PCB_ROUTER_DIFF_PAIRS_SKEW=0`); the default with x14401 (the
branch: crossovers off, meanders on). Gap, uncoupled and length are
KiCad's diff_pair_gap_out_of_range, diff_pair_uncoupled_length_too_long and
length_out_of_range plus skew_out_of_range; errors are KiCad's errors;
dangling are track_dangling and via_dangling.

*Polarity* (x1820 -> x14400 with crossovers; links coupled with the
crossovers among them in brackets):

| board | gap/uncoupled/length | errors | unconnected | vias | dangling | links coupled/total | uncoupled mm |
|---|---|---|---|---|---|---|---|
| ducktop2-center | (12, 0, 0) -> (8, 0, 0) | 15 -> 11 | 499 -> 499 | 700 -> 684 | 0 -> 0 | 3/54 -> 4/54 (3) | 6 -> 12 |
| ducktop2 left_io | (15, 0, 0) -> (30, 0, 0) | 15 -> 30 | 356 -> 360 | 239 -> 238 | 0 -> 2 | 0/41 -> 3/41 (1) | 0 -> 8 |
| ducktop2 right_io | (38, 0, 0) -> (54, 0, 0) | 40 -> 54 | 166 -> 179 | 154 -> 146 | 3 -> 1 | 1/23 -> 1/23 (0) | 1 -> 1 |
| OpenAIO-Whoop | (1, 0, 1) -> (1, 0, 1) | 0 -> 0 | 183 -> 183 | 36 -> 36 | 0 -> 0 | 0/4 -> 0/4 | 0 -> 0 |
| oculink m2_card | (1, 0, 0) -> (1, 0, 0) | 2 -> 2 | 2 -> 2 | 22 -> 22 | 0 -> 0 | 9/9 -> 9/9 (0) | 10 -> 10 |
| oculink_m2 | (0, 0, 0) -> (11, 0, 0) | 4 -> 14 | 9 -> 1 | 47 -> 38 | 2 -> 0 | 4/10 -> 9/10 (5) | 8 -> 32 |
| oculink_port | (0, 0, 0) -> (1, 0, 0) | 21 -> 29 | 12 -> 14 | 18 -> 12 | 34 -> 34 | 3/9 -> 3/9 (3) | 8 -> 12 |

Totals: links coupled 20/150 -> 29/150 (12 through a crossover), errors
97 -> 140, unconnected 1 227 -> 1 238, gap findings 67 -> 106, vias 1 216
-> 1 176. Of the 106 gap findings 2 are on pairs with a coupled link: the
rise is on the pairs that fall back, whose two nets the fallback routes
apart (KiCad pairs any of their segments with overlapping projections and
nothing between, at any distance). The fallbacks: run not routed 83 -> 78,
a pad's stub does not reach the run 25 -> 18, fewer than two pair points
22 -> 22, launches under 1 mm apart 11 -> 12, no launch 7 -> 7, copper
refused 4 -> 6. Perturbed (the router's `first_seed` 2; on the ducktop2
boards, where the seed changes nothing, the ladder's budget at 28 and 32
s): seed 2 coupled 17 -> 25, errors 110 -> 296 (oculink_port 24 -> 183:
its ladder kept a narrow rung, track_width 22 -> 147), unconnected 1 228
-> 1 231, gap 73 -> 137; left_io is the same at every budget (gap 15 ->
30, unconnected 356 -> 360), right_io's unconnected is worse at each
(166 -> 186, 178, 179; gap 39 -> 27, 38 -> 52). ducktop2-center with
crossovers at 28 and 32 s was refused memory by the ledger and has no
row. Where it works it works: oculink_m2 couples 9 of 10 links (5 through
a crossover) and leaves 1 connection open instead of 9. But coupled links
rise while errors, gap findings and unconnected rise too, on both
perturbations: **not the default** (`PCB_ROUTER_DIFF_PAIRS_CROSSOVER=1`).
What stops it: the crossovers crowd a dense connector's neighbours
(oculink_port's TX stubs at J1 fail beside the RX crossovers, which couple
instead), and the links they let through reach the copper check and are
refused there (a run turning back at a via, which swaps the pair's sides
in the plane: the vias of a U-turn on a layer change collide), and each
refused link is a pair that falls back.

*Skew and length* (x1820 -> x14401, the default): every row identical
(the routes are; the meanders add copper only): links coupled 20/150,
errors 97, unconnected 1 227, gap findings 67, skew/length findings 1
(OpenAIO-Whoop's `/USB_D_P` is not routed: KiCad's skew from a net of
length 0; it has no coupled link) both ways, vias 1 216. Eight pairs got a
meander (oculink_port TX0 0.61 mm, TX1 0.32, TX2 2.05; oculink_m2 TX0 to
TX3 0.21 to 0.49; ducktop2-center's WIFI_PCIE_TX_RAW 0.27), their skew now
0.000 mm; the other coupled pairs lay within their gap already (m2_card's
nine at most 0.002 mm, two of center's 0.007), but for right_io's
GBE_REFCLK at 0.562 mm, whose run has no straight track with room for the
bumps. Largest skew of a coupled pair: 0.562 mm on right_io, 0.007 mm on
the other boards. The meanders' tracks are tuning patterns in the written
boards; KiCad's gap findings are unchanged on every board. With the
crossovers on as well (x14400, seed 0, meanders on against off): the same
rows, the meanders longer (a crossover's diving net carries two vias from
F.Cu to B.Cu: oculink_port's RX0, RX1 and CLOCK got 5.32, 3.23 and 4.71
mm). The expansion tier's 46 boards (no board's rules ask) are identical
to x1820 with x14401, rows and routed boards byte for byte. Tables:
docs/changelog.md, "Committed on 2026-10-25 (differential pairs: polarity
and skew)".

**Runs not routed at the task budget (2026-10-26).** At 30 s, 83 of the
rule boards' links fell back as "run not routed", most of them on the
ducktop2 boards, which are far from complete at that budget. The
diagnosis ran at the task budget instead: the intake's deep run, 300 s
of work (`--budget-scale 10` on the intake's 30 s task, wall guard 1200
s), route mode, one thread per board. The verdict now says why each such
link was not routed (`not_routed` in `diff_pairs`, and the pair's `why`
in board-router.json), from a record the router keeps of its last hard
repair (`Diagnostics::repair`: per net it took up, whether the
negotiation had joined it, whether it was in conflict, and why the repair
skipped it, if it did; `not_routed_cause`):

- *a launch has no legal node*: a launch terminal is a dead pad
  (`dead_pads`): the envelope does not fit there, between pads or in
  another net's pour;
- *no path in the negotiation*: incomplete without a conflict when the
  negotiation ended: even through other nets' copper the link's envelope
  or via disc found no way past the fixed copper, or its searches gave up;
- *the hard repair found no room*: in conflict when the negotiation
  ended, and the hard repair, which takes every other net's copper as
  fixed, found no path (the negotiation ejected it);
- *no hard repair: the attempt was judged hopeless*: the attempt the
  ladder kept had been judged hopeless (a quarter of its nets without any
  path), and such an attempt runs no hard repair: its nets in conflict are
  ripped up and stay open;
- *the hard repair's budget ran out before it* (its own budget or the
  attempt's work deadline).

On the four small rule boards at 300 s (x14810, which routes as main's
x1835: rows identical) 9 links were not routed: 7 on oculink_port (6
with no hard repair: the attempt the ladder kept, pours connect at 0.05
mm, 6 open, had stopped as hopeless; 1 no room) and 2 on OpenAIO-Whoop
(1 no path, 1 no room); oculink_m2 and m2_card had none. The envelope
passing between pads, a launch in a pour and the repair's budget did
not come up. At 1200 s (the three oculink boards, x14810) oculink_port
still had 4 not routed (3 with no hard repair, 1 no room) and its ladder
stopped after 399 s of work with no rung left: structure, not budget.
The ducktop2 boards were not run at 300 s: each needs about 16 GB, which
the shared machine did not have that afternoon (a first request waited
2 h in the memory ledger); their 30 s rows are below, the 300 s rows
pending.

**The fix: links first in the hard repair, also on a hopeless attempt.**
The hard repair (`resolve_remaining`) takes the pairs' links (a net whose
terminals are pcb-kicad's "pair launch" ones) before every other net, and
searches for them even on an attempt judged hopeless, within the budget a
repair has otherwise (the negotiation's seconds, at least 60): a link left
open costs its pair a fallback, which costs more than the link. The other
nets keep the routing order and the hopeless attempt's no-repair rule;
on a board without links nothing changes (the expansion tier below).

| 300 s, 4 small rule boards | snapshot | links coupled | KiCad errors | unconnected | gap findings | dangling | vias | not routed |
|---|---|---|---|---|---|---|---|---|
| main (with the diagnosis) | x14810 | 14/32 | 106 | 163 | 55 | 37 | 165 | 9 |
| links first, hopeless repaired (return vias off) | x14812, `PCB_ROUTER_DIFF_PAIRS_RETURN_VIAS=0` | 17/32 | 62 | 158 | 39 | 5 | 160 | 2 |
| the branch (return vias on) | x14814 | 17/32 | 61 | 158 | 38 | 5 | 170 | 2 |

Per board, x14810 -> x14814 (errors are KiCad's errors, gap its
diff_pair_gap_out_of_range, dangling track_dangling and via_dangling):

| board | links coupled | errors | unconnected | gap | dangling | vias | not routed |
|---|---|---|---|---|---|---|---|
| OpenAIO-Whoop | 0/4 -> 0/4 | 0 -> 0 | 148 -> 148 | 1 -> 1 | 0 -> 0 | 65 -> 65 | 2 -> 2 |
| oculink m2_card | 9/9 -> 9/9 | 1 -> 1 | 1 -> 1 | 0 -> 0 | 2 -> 2 | 17 -> 17 | 0 -> 0 |
| oculink_m2 | 4/10 -> 4/10 | 36 -> 43 | 3 -> 2 | 31 -> 37 | 1 -> 2 | 60 -> 70 | 0 -> 0 |
| oculink_port | 1/9 -> 4/9 | 69 -> 17 | 11 -> 7 | 23 -> 0 | 34 -> 1 | 23 -> 18 | 7 -> 0 |

On oculink_port the six links the hopeless attempt left ripped up are
searched again and route; three more couple, its 23 gap findings and the
34 dangling GND tracks on In1.Cu go (the attempt the ladder keeps is now a
different one), track_width 44 -> 15. oculink_m2 has no link not routed,
but the repair's order moves its other nets: 7 more gap findings (31 ->
38 without the return vias), one connection fewer open. The remaining
fallbacks on the four boards: a pad's stub does not reach the run 8,
copper refused 3, no launch 2, fewer than two pair points 2, run not
routed 2 (Whoop: no path 1, no room 1).

**Return vias (2026-10-26; on with the pairs).** Beside each via of a
kept link (both nets' at each layer change, a crossover's too) a via of
the *reference net* where one fits (`place_return_vias`): the settings'
`diff_pair_return_net` (a net by name), else the net named `GND` (past a
sheet path), else of the poured nets whose name ends in `GND` the one with
the most pads; a net with no pour gets none (`return_net`). The return
via is the pair's via size. It is searched outward from the pair's via,
first straight away from the via's partner (the other net's via of that
layer change), then turned by 30° steps up to 120° either way, its ring
from touching the pair via's ring to at most **0.5 mm** from it
(`RETURN_GAP`, in 0.05 mm steps), the nearest first. A candidate is kept
where (`joins_pour`) the reference net's pour reaches it on two layers
(KiCad calls a via joined on one layer dangling): inside the pour's
outline and outside the pours above it, with six of sixteen points
around its ring, a fill's half minimum width out, clear of every other
net's copper by the larger of the pour's and that net's class clearance
(holes by the hole clearance) and of the reference net's own pads that
join by a thermal relief by the pour's thermal gap (oculink_m2's 6 mm GND
pad left a return via in its gap: KiCad's fill did not reach it); and
where (`return_via_clear`) it keeps every clearance the router checks
(`verify` with the project's rules: other nets' copper and holes, the
edge), hole to hole and hole clearance from the kept links' vias and every
hole (the reference net's too), and no overlap with the reference net's
pads or vias or the other return vias. They join the reference net's
route (written as its vias). Verdict and board-router.json: `pair_vias`,
`return_vias`, `return_net`; per pair in board-router.json and in the
verdict's `skews` `pair_vias` and `return_vias`.
`diff_pair_return_vias: false` (or `PCB_ROUTER_DIFF_PAIRS_RETURN_VIAS=0`)
turns them off. Measured (300 s, x14812 with them off against x14814):
10 return vias beside 28 pair vias (oculink_m2 9 of 20, oculink_port 1 of
8; m2_card's coupled runs change no layer, OpenAIO-Whoop couples none),
errors 62 -> 61, unconnected 158 -> 158, gap findings 39 -> 38, dangling 5
-> 5: not worse, so on. The checks came from the versions before: x14811
took the pour by its outline alone and checked holes against routed vias
only (six hole_to_hole findings at the rule's 0.5 mm against the pairs'
vias, which the board holds as obstacles, and one via KiCad called
dangling); x14812 and x14813 left one via in the GND pad's thermal gap
(dangling and an unconnected item).

**Measured at 30 s (2026-10-26, continuity).** The 7 rule boards as the
tables before (route mode, 30 s of work, one thread, KiCad's DRC with
`--refill-zones`, no setting, wall guard 1200 s; the ducktop2 boards one
at a time at their measured 30 s peaks: center 14.2 GB, left_io 2.3,
right_io 2.7), main's x1835 against the branch's x14814:

| board | links coupled | errors | unconnected | gap | dangling | vias | return vias / pair vias | not routed |
|---|---|---|---|---|---|---|---|---|
| ducktop2-center | 2/54 -> pending | 18 -> | 499 -> | 16 -> | 0 -> | 686 -> | | 42 -> |
| ducktop2 left_io | 0/41 -> 0/41 | 17 -> 17 | 337 -> 337 | 17 -> 17 | 1 -> 1 | 240 -> 240 | 0 / 0 | 26 -> 26 |
| ducktop2 right_io | 1/23 -> 1/23 | 38 -> 38 | 159 -> 159 | 38 -> 38 | 2 -> 2 | 145 -> 145 | 0 / 0 | 11 -> 11 |
| OpenAIO-Whoop | 0/4 -> 0/4 | 0 -> 0 | 185 -> 185 | 1 -> 1 | 0 -> 0 | 39 -> 39 | 0 / 0 | 2 -> 2 |
| oculink m2_card | 9/9 -> 9/9 | 2 -> 2 | 2 -> 2 | 1 -> 1 | 0 -> 0 | 22 -> 22 | 0 / 0 | 0 -> 0 |
| oculink_m2 | 4/10 -> 4/10 | 4 -> 4 | 9 -> 9 | 0 -> 0 | 2 -> 2 | 47 -> 54 | 7 / 12 | 1 -> 1 |
| oculink_port | 3/9 -> 3/9 | 21 -> 21 | 12 -> 12 | 0 -> 0 | 34 -> 34 | 18 -> 20 | 2 / 6 | 0 -> 0 |

Over the six boards with both sides: every row as main's but the vias
(511 -> 520: 9 return vias beside 18 pair vias, on oculink_m2 and
oculink_port): links coupled 17/96, errors 82, unconnected 704, gap
findings 57, dangling 39 both ways. ducktop2-center's branch row is
pending: at its 14.2 GB the ledger did not start it in over an hour on the
shared machine (main's row is x1835's). At 30 s the links not routed are
a matter of budget: on left_io (26), right_io (11) and OpenAIO-Whoop (2)
the hard repair's budget ran out before them (the attempt's work deadline
had passed), oculink_m2's one found no room; the links-first rule changes
nothing where the repair does not run.

The expansion tier's 46 boards (route mode, 8 x 1, 30 s; no board's rules
ask for pairs, so no link and no return via) are identical with main's
x1842 and the branch's x14815, rows and routed boards byte for byte
(unconnected 557, passes 24 both ways). Tables: docs/changelog.md,
"Committed on 2026-10-26 (differential pairs: runs not routed; return
vias)".

**Phase 2's remaining items, and what was learnt.**
(1) A link must not cost the board: the gate failed on boards where the
link nets lose or are refused. Candidates: a link net that stays in
conflict after a few iterations dropped from the negotiation (and its
pair's connections handed back to the pair's nets in the same router,
which needs the core to drop and add connections mid-run); or the ladder
running a rung without the pairs and keeping the better board (it costs
budget on every board with a link). (2) Refusals: of 55 links 20 are
refused, mostly a pair's own or a neighbour's ordinary copper inside the
run's offset tracks where the negotiation ended with conflicts. (3)
Polarity: a crossed end (14 of the coupled or partly coupled links on the
tier) now crosses in the stubs with a via, which the router places; KiCad
then counts the crossed stubs as uncoupled and, where they run parallel,
out of the gap's range (the fixture's LVDS pair). 2026-10-25: a crossover
in the lead is built and measured, not the default (above, "Polarity");
left: a crossover where the run changes layers anyway (its two vias
staggered, no via added: a U-turn at a layer change needs one), a
crossover that leaves a dense connector's neighbours their stubs, and the
swap at a part, which needs swap data no rule board has. (4) Length and
skew (KiCad's `length`, `skew`): the coupled run's offset makes the outer
track of each bend longer by `2 d tan(θ/2)`. Done 2026-10-25 (above, "Skew
and length"): measured from the written copper, matched by a meander on
the shorter net, a KiCad tuning pattern; left: a meander where the run has
no straight track with room (right_io's GBE_REFCLK, 0.562 mm), and length
matching across pairs (a bus's `length` rule over several pairs, which
KiCad's `skew` without `within_diff_pairs` compares too). (5) GND return
vias beside each pair via: done 2026-10-26 (above, "Return vias"), placed
after the route where room is left, not reserved in the link's via disc
(`2 dv / 0.7 + 2 V + 2 c`), which would cost the links room in the
negotiation. (6) Per-layer widths in one run (ducktop2's DIFF_85,
0.183 mm outside, 0.114 on In2): the router has widths per layer for a
net since 2026-10-24 (`NetRules::layer_widths`, "Project rules" above:
classes split by their widths per layer, a stamp variant per layer
width), so a link class may now carry the pair's width per layer; what
is left is to give the links' classes those widths (the pairs' rules are
`inDiffPair()` conditions, which the custom rules' lowering does not
turn into widths per layer), the offset per layer at the write, and then
the class's layers all the pair's (and the pairs' nets no longer kept off
the layers where one width breaks the bounds, phase 3's "Layers").
(7) Done in phase 4: pairs through series parts (a capacitor pair as a
pair point of two parts; "fewer than two pair points" 65 -> 22 on the
rule boards). (8) Done in phase 3: the pairs' nets keep to the layers their rules
allow (above, "Layers").
(9) Coupled branches (a T at an ESD part) instead of one run per link.

## Length and skew rules (2026-10-26)

`length_matching` in the router settings (default on;
`PCB_ROUTER_LENGTH_MATCHING=0` or `1` where the settings name none).
KiCad's custom `length` and `skew` constraints over single nets and
groups of nets (a memory bus, a camera bus, a USB group) were lowered as
`unmodelled` until now, and KiCad reported `length_out_of_range` and
`skew_out_of_range` on every routed board with such a rule. The pairs'
pass ("Skew and length" above) matched only the two nets of a coupled
pair. The code: crates/pcb-kicad/src/project_rules.rs (`length_rules`),
board_router/lengths.rs (the pass), board_router/meander.rs (the bumps,
the heights, the tuning pattern, shared with diff_pairs.rs).

**Census.** Of the 109 boards of tasks v7 and the 46 of the expansion
tier, two have such a rule in their `.kicad_dru`, both in v7, none in the
expansion tier. The board setup has no length rule of its own: 106 of the
155 projects carry `tuning_pattern_settings` (the interactive tuner's
default amplitudes and spacings, no DRC check; 6 differ from KiCad's
defaults), none has a KiCad 10 tuning profile (`tuning_profiles`, a net
class's `tuning_profile`), and every project rates `length_out_of_range`
and `skew_out_of_range` errors. The designer's own boards meet their
rules (kicad-cli 10.0.7, zones refilled: no finding); the routes do not
(the designer tier x1832 at 1143 s of work, the 30 s route below):

| board | rule | nets | bounds | designer | x1832 designer tier | 30 s route, x1840 |
|---|---|---|---|---|---|---|
| oro-os link | `ULPI_exact_length`: `length` on class ULPI | 12 ULPI nets | 31.4 to 31.6 mm | 0 | 12 length | 12 length |
| Sisu_SSE-9 | `USB 90 ohm differential geometry on L2 and L5`: `skew` on class USB on In1.Cu and In4.Cu (with track_width, diff_pair_gap, diff_pair_uncoupled) | 11 USB nets (5 pairs and a lone net) | 0.5 mm | 0 | 3 skew | 4 skew |

Beyond the two tiers, the intake (benchmarks/expansion/data/boards) has
seven more boards with such rules: hstarmans firestarter's compute board
(`DVP_Bus_Length_Matching`: `length` 25 to 27 mm on 10 nets of class
DVP_BUS), OpenAIO-Whoop (two USB `skew` rules, rated warnings by the
project: not lowered), jorisplusplus pcdock's base board (`length` with an
opt only: KiCad checks min and max, so no finding), KiCad's QA boards
issue24772 (`skew` in picoseconds: unmodelled) and four length or skew QA
boards (length_calculations twice, skew_group_matched_drc,
skew_within_diff_pairs_drc). firestarter, OpenAIO and
skew_group_matched_drc are measured with the two below.

**Lowering** (`project_rules::length_rules`). Per net and copper layer,
the rule KiCad applies to a track of the net there (`dru::track_rule`:
the last whose condition surely holds, `inDiffPair()` known from the
names), for `length` and `skew` separately; the nets are grouped by the
rule (its index: two rules of one name are two groups), as KiCad's
matched-length check groups items by the rule that matched them. Per
member the layers the rule holds on and whether its vias count (KiCad
evaluates a via's rules on its first layer, F.Cu for a through via:
Sisu's rule on In1.Cu and In4.Cu counts those layers' tracks and no via).
A rule rated below an error (its own `(severity ...)` or the project's
`length_out_of_range` or `skew_out_of_range`) is passed over, as is one
with no bound in lengths (KiCad 10's `ps`). `(within_diff_pairs)` makes
a skew compare the two nets of each pair only. With the switch on
(`with_lengths`) a `skew` and a `length` with only a minimum are reported
`applied`, a `length` with a maximum `approximated` (the minimum is met,
the maximum only checked), and leave `rules.unmodelled`.

**Measuring as KiCad does** (`kicad_length`). A net's length for a rule
is its tracks on the rule's layers, its vias at the stackup's height, and
inside a pad that a net's tracks leave once, the straight line from the
pad's position to where they leave instead of the tracks' way there. A
via's height is between the outermost layers of what it joins: the
tracks ending on it and the pads it lies in (a via in an SMD pad to a
track on B.Cu counts the whole stack). The heights (`copper_heights`,
now shared): the outer copper layers at the board's faces, an inner one
at its middle, a dielectric's sublayers (`addsublayer`) summed. Fitted
on KiCad's QA board length_calculations, whose cases KiCad holds within
1 µm (the unit test reproduces four of them exactly and the pad case
within 5 µm: KiCad's pad is rounded); until then the pairs' heights
took each layer's middle and the first sublayer only, and oro's four
routed ULPI nets that met 31.5 mm by that measure were 31.69 to 31.97 to
KiCad (its F.Cu to In5.Cu via 1.215 mm, not 0.98). On the 30 s routes
below, of the nine routed nets KiCad reports a length for, eight agree
with the verdict's to 0.1 µm (firestarter's DVP_PCLK and Y2 to Y8, Sisu's
MODEM_USB_PHY_N); firestarter's DVP_Y9 is 0.024 mm longer by the
verdict's measure (30.856 against 30.832 mm; not meandered, it is over
its maximum either way).

**Meanders** (`match_lengths`, when each attempt is written, after the
differential pairs, before the routes are emitted). The `length` rules
first, then the `skew` rules on the lengths they made. A net below a
`length` minimum is lengthened to the rule's opt (held inside the
bounds), else to the middle of its bounds, else to its minimum; a net
further than a `skew` maximum below the longest net of its group to the
longest less half the maximum. Only a completely routed net that is not
the net of a coupled pair (the pairs' pass matches those; a meander on
one track of a coupled run would part the pair) gets one. A meander is
`meander::bumps` on a straight track of the net on a layer the rule
counts: bumps `max(w + c, 2 w)` wide and apart, at most `max(1 mm, 3 w)`
high (KiCad's single-track default amplitude), then half and a quarter
of that, their height bisected to add exactly what is missing, as many
as hold it or fewer (the rest on other tracks), at the track's start,
end, middle and up to twelve places between, either side; the first
whose copper keeps every clearance (`core::verify` against the board's
obstacles near the net and every route near the meander) and that parts
no branch, via or pad of the net from the track. The longest tracks
first; what is left of a split track is tried again. The meander's
tracks are the route's (the internal checks see a connected net) and are
grouped as a KiCad tuning pattern (`tuning_mode "single"`, the rule's
target and bounds). A `length` maximum is not kept: the router's search
has no per-net length bound (the A* cost is the path's; a bound would
need the net's whole tree, its pads' order, in the search), so a net
longer than its maximum is reported (`note`), and a group's longest net
sets its skew target whatever it is. Verdict and board-router.json:
`lengths` (per rule and net `length_mm`, `target_mm`, `min_mm`,
`max_mm`, `meander_mm`, `within`, `note`; docs/agents.md).

**Measured** (route mode, the designer's placement with its tracks
stripped, one thread, KiCad's DRC with zones refilled; off x1840 (main),
on x14861 (this branch). Length and skew are KiCad's length_out_of_range
plus skew_out_of_range, errors all of KiCad's errors).

*30 s of work* (the wall guard 1200 s):

| board | length/skew | errors | unconnected | vias | meanders |
|---|---|---|---|---|---|
| oro-os link | 12 -> 5 | 22 -> 15 | 183 -> 183 | 146 -> 146 | 7 of 12 nets, 37.2 mm, 19 patterns |
| Sisu_SSE-9 | 4 -> 1 | 10 -> 7 | 111 -> 111 | 160 -> 160 | 4 of 11, 34.4 mm |
| firestarter compute | 10 -> 8 | 10 -> 8 | 3 -> 3 | 164 -> 164 | 9 of 10, 47.3 mm, 27 patterns |
| OpenAIO-Whoop | 0 -> 0 | 0 -> 0 | 185 -> 185 | 39 -> 39 | none (its skew rules are warnings): board identical |
| QA skew_group_matched_drc | 5 -> 5 | 5 -> 5 | 0 -> 0 | 0 -> 0 | none (the undefined class, below) |

Totals: length/skew findings 31 -> 19, errors 47 -> 35, unconnected 482
-> 482, vias 509 -> 509 (the routes are the same: the meanders add
copper only). oro: the seven routed ULPI nets at 31.500 mm, 1.9 to 12.8
mm of meander each; its five findings left are the five ULPI nets the
route left open (KiCad: length 0). Sisu: MODEM_USB_N, RP_USB_P and
RP_USB_N (fallen-back pairs) matched to MODEM_USB_P (39.29 mm), its
finding left MODEM_USB_PHY_N (8.7 mm of In1.Cu, 36 mm short: 5.8 mm
placed); its other findings: diff_pair_gap 3 -> 4 (one more on the RP_USB pair,
both of whose nets got a meander), uncoupled 3 -> 2. firestarter: XCLK and Y8 to
26.000 mm; PCLK, Y3, Y4 and Y6 lengthened by 6.7 to 11.1 mm and still
short; Y2, Y5 and Y7 (inside the bus) got one 0.35 mm bump each; Y9 is
over its maximum (30.8 mm). No KiCad clearance finding was added on any
board (every type but length, skew and the pairs' is unchanged).

*The expansion tier* (46 boards, 8 x 1, 30 s; no board has a length or
skew rule): x1840 (build/pour5-exp-off-x1840) against x14861, 46 of 46
routed boards byte-identical, rows identical (routed, vias, KiCad's
findings, work).

*At the task budget* (the designer tier, `--tier designer`, 1500 s of
work, 2 x 1; the two census boards, both in tasks v7):

| board | length/skew | errors | unconnected | vias | meanders |
|---|---|---|---|---|---|
| oro-os link | 12 -> 1 | 12 -> 1 | 1 -> 1 | 370 -> 370 | 11 of 12 nets at 31.500 mm (0.35 to 11.2 mm each); ULPI_D4 32.94 mm, over the maximum |
| Sisu_SSE-9 | 4 -> 1 | 14 -> 11 | 17 -> 17 | 281 -> 281 | RP_USB_N, RP_USB_P (33.0 and 37.2 mm) and MODEM_USB_P to MODEM_USB_N's 58.31 mm; RP_USB_PHY_P 9.6 of 53.8 mm placed (its finding) |

Sisu's other findings are unchanged at this budget (diff_pair_gap 7,
uncoupled 3). After main took the pairs' return vias (x1843), the
branch rebased (x14863) routes Sisu at 30 s and at the task budget, oro
and the QA board at 30 s byte-identically to x14861 (Sisu's pairs fall
back, so no return via is placed): the tables stand. Default: **on**
(the findings fall on the three boards whose rules are lowered;
unconnected and vias do not change; errors fall).

**What is left.** (1) Room: on a dense bus the router lays the nets side
by side at the clearance, and no bump fits beside a track with a
neighbour at its clearance on both sides (firestarter's Y2, Y5 and Y7
get one small bump each); the outermost nets take the length. Room needs
the route to leave it (a bus's lanes spread, or the meander as a routing
cost), not a pass after it. (2) A maximum: a net longer than its rule's
maximum stays so (firestarter's DVP_Y9, 30.8 mm against 27). (3) Nets
the route leaves open have no length to match (oro's five unrouted ULPI
nets are its remaining findings). (4) A coupled pair in a skew group of
several pairs: its nets are the pairs' pass's, which matches the pair to
itself, not to the group (Sisu's pairs fell back, so the pass here
matched them as single nets; on a pair net KiCad's gap check then finds
the meander out of the pair's gap: Sisu's diff_pair_gap 3 to 4).
(5) A net class named in `netclass_assignments` that the project does
not define (KiCad's QA board skew_group_matched_drc: `DIFF_PAIR`) is not
the net's class to the lowering, so the rule holds for no net there;
KiCad's `hasNetclass()` sees it.

## CPU profile (2026-10-25)

Where a route's CPU goes and how to profile it: [profiling.md](profiling.md).
The search is 76 to 90 % of a route's CPU (its heuristic a further 4 to
7 %); inside it the heap's sift (15 to 18 %), the neighbour steps' search
nodes and occupancy bytes (18 to 26 %) and the hop test (3 to 9 %). The
profile round made the hop test walk only where a via can lie within its
reach (`SearchNode::since_via`), the heap's root wait for the next push
(`KeyHeap`), and the expansion prefetch its neighbourhood: identical
routes, 9 to 12 % less CPU.

## Determinism and the wall clock (2026-10-14)

Every routing decision is taken in work (search expansions at
`expansions_per_second`, placement work at the placer's
`work_per_second`), so the same input, thread count and work rates give
the same board however busy the machine is (`PCB_ROUTER_FINGERPRINT`
prints phase fingerprints to compare runs). The rates are measured at
the start of every route, layout and placement unless pinned ("The
machine's rates" below): results depend on the measured rates unless
`PCB_WORK_RATE` pins them, as the benchmark tiers do.
The wall clock only guards: in route mode at `GUARD` (4) times a budget
(the attempt ladder's own budget only once the wall clock over 4 is ahead
of its work by more than a twentieth of that budget: in route mode the
work counts only search expansions, not KiCad's checks), in a layout at
its deadline, `wall_guard` (default 2) times its budget. A layout counts each
KiCad check at a cost modelled from the board, the same cached or not
("KiCad's checks as work" above), so its end in work no longer leaves
10 to 50 s of uncounted wall clock per check. Where
the wall clock, not the work, stops something, the run says so once per
kind in its log (`wall guard: the wall clock cut the run short (...)`)
and in its report (`cut` in `board-router.json` and `board-layout.json`,
from `pcb_router::router::wall_cuts`); `run.py` copies it into the row
(`cut`, also `["timeout"]` for a run the harness killed) and `compare.py`
marks such rows CUT and leaves them out of better / same / worse. The
command line's exit code is 3 for an incomplete result with a cut
(docs/agents.md, section 4).

A layout's routers stop at the layout's end in work (router
`deadline_work`, armed before every phase from what is left of
`total_seconds`), not at the wall-clock deadline. The placement's seeds run
in a pool (`race_seeds` in board_placer/seed_race.rs), as many at once as the
process has search threads (`RAYON_NUM_THREADS`), a seed starting as a
thread frees; they are judged in seed order as if one ran after the
other, and the batches they start in are sized from the kept seeds' work,
so the seeds run, the seeds kept and the placement do not depend on the
threads. The clock counts the kept seeds as the reference machine runs
them (a pool of eight threads, `REFERENCE_THREADS`: `pool_work_seconds`); the
part beyond the longest seed (`work_seconds`) counts toward that end, not
toward the budgets. Until 2026-10-14 the seeds ran in waves as wide as the
machine's cores, each as long as its slowest seed, and the waves' work
counted, so the end followed the cores (video: 392 s of waves beyond its
longest seed on four cores, 281 s on eight). The first
route's negotiation gets at most half of what is left before it, so that
the hard repair after it has as much (a deadline inside the negotiation
leaves every net still in conflict open). `PCB_ROUTER_NO_WALL_GUARD` drops
the layout's wall-clock deadline too: the work alone decides, for checking
that a run is deterministic on a machine too busy for its budget.

**A debug flag only prints** (2026-10-26). A flag that prints
(`PCB_ROUTER_SPOKE_DEBUG`, `PCB_ROUTER_FINGERPRINT`, `PCB_ROUTER_DEBUG`
and the like) must not change what the router counts or caches, since
the work decides the budgets: a question the debug output asks that is
counted as work, or an answer it leaves in a cache that a later
question then finds for free, moves the route. Until 2026-10-26 the
spoke debug asked the fill probe questions of its own
(`blocker`, `fill_at`, `in_ring`, `in_hatch_hole`, the necks' pieces)
whose items, queries and vertices were charged as pour work and whose
hatch holes and neck verdicts stayed cached: on F1 it raised the pour
work from 0.20 to 0.36 s, and on the expansion tier (x1844, route mode,
30 s) Miniboard routed to 6 unconnected without the flag and 10 with it
(10 and 5 with the exact model on), and ZSWatch's and brickpico's boards
moved too.
Now those questions go through `FillProbe::quietly`: their counts are
put back, the probe keeps no hole they decide (`hole_kept`) and the
spokes no neck verdict (`is_quiet`), an index they build is charged
where the probe first needs it outside (`unpaid_index`), and the
router's probe shapes are not built for them (a polygon is measured
without its index there; the answers are printed, never used).
`PCB_ROUTER_FINGERPRINT`'s pour analysis before stitching was charged
too (F1: 0.200 to 0.205 s of pour work); it runs under
`Router::uncharged` now, which puts the router's pour and probe counters
back (and prints the fingerprint alone before the probe shapes exist).
`tests/fixtures.rs` (`debug_flags_change_neither_the_route_nor_its_work`)
routes F1 without flags and with each of the two and compares the
written board byte for byte and the work counted; the router's unit test
`a_fill_probe_counts_nothing_for_the_debug_output` the probe's counts.
A new debug output that asks the probe or the router anything goes
through the same two scopes.

Memory decides only how many routers run side by side (the placement
race's probes, the probe ladder's rungs, the regular rungs, the seeds),
never which count (a limit of the process's own, a fixed number, decides
whether the probes and a finer pitch run at all: "Memory limits" above): they are judged in order as if one ran after the other,
a probe the work left would not have started counting for nothing (the
probe ladder checked the work left only between waves, so under the
layout's `deadline_work` the number of rungs judged followed the memory
free at the start; it now checks before each probe of a wave).

A router given no negotiation budget (`negotiation_seconds` unset) has
its default 900 s in work too; it used to be 900 s of wall clock and the
only limit of the attempts the ladder repeats from a probed rung (the best
rung's seeds, the narrow-signal and relaxed-clearance steps: video's
narrow-signal attempt stopped wherever the load had brought it), and the
pour nets' pre-negotiation in route mode has its minute in work.

The phase budgets after negotiation (resolve, clean-up, via reduction,
thermal spokes: `clock` and `spent`) count the pour analyses as work beside
the search (`budget_work`): a clean-up on a pour board spends much of its
time checking that a batch strands no pour pad (Sisu: 21-47 s of pour
checks in a 61-66 s budget, uncounted until 2026-10-14, so its budget ran
on in wall clock with the load and the route-mode guard once stopped it).
An analysis costs `POUR_NODE_NANOS` (23) per lattice node inside the net's
pour, `POUR_STAMP_NANOS` (6) per stamp of the net, and for its fill probe
(`fill::FillProbe`, the spoke tests) `PROBE_ITEM_NANOS` (9) per item it
indexes or looks at, `PROBE_QUERY_NANOS` (850) per query (a point tested
against its index: the bucket lookups and the polygon test, whatever the
items) and `PROBE_VERTEX_NANOS` (9) per polygon vertex its shape tests go
through. Recalibrated 2026-10-15 (x1013, route mode, one thread, v16
placements) on twelve boards, six of them hatched (Batch_1, WLED_Matrix,
USB_Keypad, Qfwfq, 1337-v4.0, the A13 module; Castor, Sisu, reCamera,
k30-SBC, USB_Cable_Tester, spell_tome solid): per board 0.89-1.12 of the
analyses' wall time (spell_tome, 1.1 s of analyses, 1.41). The items-only
model (18 / 26 / 42 ns, calibrated on seven boards with few or no hatched
zones) gave the hatched boards 0.34 (Batch_1) to 1.82 (A13): a hatched
zone's probe asks many more points (hatch holes, rings, each brush centre
a query of about a microsecond whatever its items), and Batch_1's board
cutouts (polygons of up to 134 vertices) are measured in every query near
them, so its probes ran 220 ns per item instead of 20-80.
The same constants on one board vary 0.8-1.2 from run to run with the
load. `PCB_ROUTER_POUR_TRACE` prints each analysis's counts,
modelled work and wall time for recalibrating; the verbose `profile work:`
line splits wall time, search work and pour work by phase, and the
clean-up's last line says how much of its budget it used and how much of
that was pour analyses. Not counted: the negotiation's stamping and
conflict bookkeeping (its limit is search expansions; at one thread its
wall time runs 10-40 % ahead of search and pour work), the pour analyses
in the layout's work tallies and in `deadline_work` (those still count
search expansions). Route mode's ladder counts the manual spokes' since
2026-10-21 ("The work bound in route mode") and every pour analysis,
lattice build and lowering since the same day ("All the work charged"
below).

**The manual spokes** (2026-10-21). `add_manual_spokes` runs after
every finish: per starved pad a full pour analysis, then one per
candidate track it tries (Vortac's panel: 34 tracks, 27 s of analyses
in 28 s of wall clock per attempt, and its fill probes were not counted
at all). Now an analysis of routes it did not change is reused (the
state after a kept track is the one its trial analysed; after a track
not kept, the one before: an analysis is a function of the routes), its
fill probes count as pour work, it is a phase of its own in the
profile (`manual spokes`), and it has a budget in work (the
negotiation's work, at least `MANUAL_SPOKES_SECONDS`, 60 s: a bound no
ordinary board reaches) and stops at the wall-clock deadline, but not at
the work deadline (the negotiation before it has usually reached that,
and starved thermals are repaired however the budget went). On six
boards with manual spokes (CompassTag, RS485M, Zulassungskarte,
ch32v003, backplane, nervous-ecg) the boards and the fingerprints are
those of x1366; Vortac's phase went from 28 s to 13 s.

**Where the wall clock went** (2026-10-21). Every route ends with a
`wall accounting:` line and board-router.json's `wall_accounting`: the
ladder's wall clock and work, the machine's measured share of the work
rate and the guards' stretch (below), then thread seconds (routers side
by side add up) of the lattices built, the lowerings, the routers'
preparation, each phase with the search and pour work it counted, and
kicad-cli's runs. What the phases' wall clock has beyond their work is
what the budgets do not see. Since "All the work charged" the line
starts with the ladder's work by kind (search, pour, lattices,
lowerings, KiCad's checks; `wall_accounting.charged`) and gives the
lattices' and lowerings' modelled work beside their thread seconds.

**All the work charged** (2026-10-21). Route mode's ladder counted an
attempt's search expansions and its manual spokes; the rest of what a
router does cost wall clock and no work: the negotiation's pour refresh
and the finishing phases' pour checks (on the expansion tier 0 to 15 s
a board beside a budget of 30, brickpico 15.3 s), the lattice builds
(ChengminLi's 16 classes: 15 builds, 24 to 40 thread seconds), the
stamping of the static maps, the lowerings. Now an attempt's work is
all of it, each priced from what it counted, the same on every machine
and at every load (a measured duration would make the ladder's
decisions follow the load):

| charge | where it happens | unit (reference nanoseconds) | calibration |
|---|---|---|---|
| search | every phase's searches | `expansions_per_second` (3.15e6 a second) | 2026-10-16, unchanged |
| pour analyses and fill probes | the negotiation's pour refresh, `unfinished` (the probes' ranking), resolve, clean-up, via reduction, thermal spokes, stitching, manual spokes (`RoutingResult::pour_work`, `Router::pour_work`) | `POUR_NODE_NANOS` 23 per node inside the pour, `POUR_STAMP_NANOS` 6 per stamp, `PROBE_ITEM_NANOS` 9, `PROBE_QUERY_NANOS` 850, `PROBE_VERTEX_NANOS` 9 | 2026-10-15, unchanged (0.89 to 1.12 of the analyses' wall clock per board) |
| lattice builds | `Router::new`: the static maps of every class (the outline, every obstacle's nodes, the tight steps), the nets' terminals and pour masks, the occupancy and search records (`BuildCounts`, `RoutingResult::build_work`) | `BUILD_MAP_NANOS` 22 per node of every class's maps (cells x layers x classes), `BUILD_TEST_NANOS` 11 per node tested, `BUILD_EDGE_NANOS` 2.75 per shape edge measured, `BUILD_STEP_NANOS` 22 per edge a tight node's eight steps measure | 2026-10-21, below |
| lowerings | `lower`, per attempt and per rung probe's board | `LOWERING_EXPRESSION_NANOS` 26 per expression of the board file | 2026-10-21, below |
| KiCad's checks | layouts only | `CHECK_SECONDS` 3.6 s and the hatch and outline terms | 2026-10-15 and 2026-10-21, unchanged |

All are seconds of the reference at one thread, scaled to the process's
rate as the pour model is (`reference_time_scale`). The build's
constants were fitted on 83 distinct builds of the expansion tier's
boards and the named boards (route mode at 4 s, one thread,
`PCB_ROUTER_BUILD_TRACE`, the fastest of each build's repeats scaled by
the machine's measured share at load 20 to 30): the model is 1.0 of the
sum, 0.6 to 1.6 per board (10 % to 90 %), and two builds of one board
in one process differ by 1.34 times (median, at most 2) at that load,
so the spread is mostly the load's. A first model that counted only
the obstacles' nodes and the polygons' corners came out 0.28 to 2.8 a
board (the outline, the tight steps, the pour masks and the fields of
large polygons were missing).
ChengminLi's builds alone at one thread are 0.74 of the model's (1.99 s
for 1.48 s at 0.1 mm, 3.67 for 2.61 at 0.075 mm); three side by side
at load 16 to 30 took 2.7 times the model, as its search did (the
lattices of 16 classes are memory bound). On the expansion tier the
lattices' model is 0.87 of their thread seconds times the measured
share, the search's and pour model's 1.28. A lowering is 0.5 to 22 ms
(the median 19 and 26 ns an expression in two runs), under 0.1 % of
any budget, and charged for completeness.

The ladder charges each kind (`Ladder::charge`, `charged`): an
attempt's result carries its router's pour and build work and its
lowering (`pour_work_seconds`, `build_work_seconds`,
`lowering_work_seconds` in its report); a probe counts its search, its
pour work and its lattice (`LadderRunner::work`), the rungs' lowerings
for the probes count when they happen, the probed rung's continuation
its own search and pour work beyond its probe's. The routers stop as
before on their search alone (`deadline_work`, the negotiation's
limit): a router given the same share searches byte for byte as before.
What changes is the share: the ladder turns a share of what is left
under its bound into a router's limits in search work by the share of
search in the routers' work so far (`search_fraction`, at least a
quarter, 1 before any search; the lowerings apart): the continuation's
deadline and negotiation, the rungs' deadlines and negotiation and
their `run_work`, the seeds' deadlines, the steps' deadline and
negotiation (`router_work`). The ladder's first attempt, or its rung
probes, have no proportion to go by: they went beyond the bound by their
own pour work and lattices (the five rows of the expansion tier above
36 s of work: musicbrain 42.6 s, a first attempt of 34 s of search and
8.4 s of pour work), and VortacDesign's probes alone were 54 s of work
on a budget of 30 (34.6 s of it the three connect probes' pour
refresh), so its continuation got nothing and the board went from 125
to 202 open (since "The probes' overhead" below: the first attempt
stops on all its work, the probes' pour refresh and lattices cost a
fraction, Vortac 125 open again). The verdict has `pour_work_seconds` and
`lattice_work_seconds` (with the lowerings) beside `work_seconds`.

`PCB_LADDER_UNCHARGED_POUR` restores the count before (search and manual
spokes, search shares), `PCB_LADDER_SEARCH_SHARES` charges all but
gives the routers their shares as search work; `PCB_ROUTER_BUILD_TRACE`
prints every build's and lowering's counts, modelled work and wall
clock (for recalibrating). A layout's ladder and its work tallies are
unchanged: they count search expansions and KiCad's checks.

**The probes' overhead** (2026-10-21). Charging all the work showed what
the probe ladder spent beside its search; most of it is gone now, not
hidden, each part still charged:

- *The fill probes' polygon tests.* The negotiation's pour refresh (from
  the third iteration on, per pour net, `refresh_pour`) runs a full pour
  analysis, twice when pads are off the plane (with and without their
  stubs), and every analysis checks every pad's spokes with the fill
  probe (`kicad_spokes`). On Vortac's panel 90 % of an analysis of GND
  was the probe measuring its points against the board cutouts and a
  copper logo, polygons of 71 to 1589 corners, every corner for every
  query near them (29.5M corners an analysis, 307 ms of modelled work).
  The probe now measures polygons of more than 48 corners
  (`INDEXED_POLYGON_VERTICES`) by their edges near the point
  (`geometry::PolygonIndex`: edges in cells of 0.25 mm or more and in
  rows for the crossing test, built once a router, `Router::probe_shape`):
  the same answers (`distance_below` is the distance's comparison;
  `distance_within` the distance where it is below a cap, which `slack`
  and `in_ring` ask only there), 2.5M corners and cells an analysis, 65
  ms. The counts are the work: `PROBE_VERTEX_NANOS` per edge and cell a
  query goes through, `PROBE_ITEM_NANOS` per edge put in a cell. The
  fingerprints of Vortac's attempt are those of before.
- *The static maps.* Every router built every class's static maps (the
  outline, the obstacles' nodes, the tight steps: about half a build),
  and the four rung probes, the rungs of a pitch, the seeds and the steps
  of one board built the same maps again and again. They are a function
  of the board's outline, obstacles, classes and the nets' classes, the
  lattice, which classes are searched and the rules beyond the classes,
  so routers of the same inputs share them now (`shared_statics`, a
  process-wide cache by a 128-bit key of those inputs' exact debug
  forms, while a router or a hold holds them; `Config::share_static_maps`).
  A router that shares them builds its own maps only
  (`BUILD_SHARED_MAP_NANOS`, 12 ns a node; fitted on the shared builds of
  eleven probed boards at 4 s, the fastest of each board's repeats scaled
  by the measured share, 11.7 over their sum, 7.0 to 20.6 a board) and
  its nets' preparation. The ladder holds the maps of the routers it
  starts (`hold_static_maps`, `LadderRunner::hold`), so that which router
  starts first does not decide who builds them. The connected pours'
  masks (`compute_pour_masks`) are held with them by the board's pours
  (`POUR_MASKS`): the three connected probes lower one board. The routing
  is the same (`PCB_ROUTER_NO_SHARED_STATICS` builds them all anew). How
  the shared build is charged: "The canonical charge" below (until the
  ladder review fixes a router that found its maps counted its own maps
  only, so the charge followed what the process happened to hold).
- *The continuation's share.* The leader's continuation builds no
  lattice: its share of the bound becomes search work by its own probe's
  proportion of search and pour work (`built_router_work`), not by all
  the routers' with their lattices (`PCB_LADDER_CONTINUATION_LATTICE`).
  A leader routing its pours as tracks did no pour work where the
  connected probes beside it did a third of theirs (Vortac).
- *The first attempt's limits.* With no proportion of search to go by,
  the ladder's first attempt had all that is left of the bound as search
  and its pour analyses and lattice on top (musicbrain: 33.3 s of search,
  8.4 of pour work, 42.6 s for a bound of 31.5). It stops on all its
  work now (`Config::limits_count_all`: its work deadline counts its
  search, pour work and build, its negotiation's limit its search and
  pour work; `PCB_LADDER_SEARCH_LIMITS` restores the search limits). The
  rung probes keep their search limits: with their pour work and
  lattices out of their shares (`PCB_LADDER_PROBES_COUNT_ALL`) they
  stopped an iteration earlier and two leaders of the expansion tier
  changed (ducktop2's from tracks to plane stubs, 1 to 7 open); equal
  search is what the ranking compares.
- *A step's realistic cost.* A step starts only with a fifth of the
  ladder's first attempt's work left under the bound
  (`step_cost_fits`, `STEP_LEAST_SHARE`; `PCB_LADDER_NO_STEP_COST_GATE`
  for the rule before): its first iteration fitting what was left let
  steps start with a second (iorodeo, ducktop2, tuxkeychain) and leave
  hundreds of nets in conflict; on the expansion tier's logs every step
  started with less than a tenth of that work lost, and facade60 won
  with a third.

What is left: the probes' search overshoot (a probe stopped after the
iteration that crossed its share: MSign's four probes searched 5.1 to
5.2 s on shares of 3.75; since 2026-10-22 it stops between the batches,
"The probes' overshoot" below), each router's own maps (MSign 0.19 s a probe),
the connected probes' pour refresh at the analyses' remaining cost (the
nodes inside the pour and the spoke queries: Vortac 1.9 to 4.5 s a
probe), the key of the static maps (the obstacles' debug form, in the
fitted constant).

**The probes' overshoot** (2026-10-22). A negotiation checked its work
limit (`negotiation_expansions`) and the work deadline only after a
whole iteration, and an iteration of a big board is seconds of work: a
rung probe went past its share by up to an iteration (MSign's four 5.06
to 5.26 s of search on shares of 3.75, iorodeo's 3.84 to 4.99), work the
ladder charged and the continuation lacked. Now a probe's limit binds
between the batches of an iteration too (`negotiate_from`,
`Router::in_probe`), from its second iteration on (its first is whole
whatever its limit: `first_iteration_expansions`), so it stops within a
batch of its share (MSign 3.75 to 3.97 s). The nets of the batches after
the cut keep their routes of the iteration before and the history stays
as it was; the probe's count of unfinished nets (the ranking) is taken
there. The cut's rest is kept (`CutIteration`: its batches, the
iteration's index, the windows' growth and the price of sharing), and
`resume` routes it first and ends that iteration as it would have ended
(`finish_cut_iteration`: `end_iteration`'s history and pour refresh, the
replan, the price grown), so the leading probe and its continuation
search what they searched before, the continuation with the other
probes' overshoot as more work (a test checks that a cut probe resumed
is the probe that finished its iteration, expansion for expansion).
That rest is outside the continuation's negotiation limit, as the
overshoot was. Only the probes are cut inside an iteration. Two
variants lost on the expansion tier (docs/changelog.md, probe
overshoot): cutting every negotiation (the continuation's and the via
reduction's rounds too: ZSWatch 3 to 9 unconnected after a continuation
cut with seven nets in conflict) and cutting the probes with the
continuation starting afresh from the cut board; both lost ducktop2's
pass (its leader cut at 84 unfinished nets, its continuation 2 open).
`PCB_ROUTER_LIMITS_BETWEEN_ITERATIONS` restores the rule before.

**The canonical charge** (2026-10-21, ladder review fixes). With the
static maps shared, who found its maps and who built them depended on
the order routers were built in, and that on how many rungs, probes and
seeds ran side by side, which follows the memory available
(MemAvailable, a cgroup's use): a router that found the maps of an
earlier one (the last built stayed in the process) counted its own
maps, one that did not the whole build, and the difference (0.3 to 6 s
of work a board) went into every share after it. So the charged work,
and the routing, depended on the machine's free memory
(docs/reviews/2026-10-21-review-ladder-work.md, finding 1). Now:

- *A router's price is canonical.* `Router::build_work` is the build as
  if the router built everything itself (`build_counts`: every class's
  nodes at `BUILD_MAP_NANOS`, the static maps' tests, the nets'
  preparation and their pours' masks), whether it built its maps or
  found them, and `Router::shared_build` names the parts routers of the
  same inputs share, each with its key: the static maps beyond the
  router's own (nodes at `BUILD_MAP_NANOS` less `BUILD_SHARED_MAP_NANOS`,
  their tests, edges and tight steps; `statics_key`) and the connected
  pours' masks (`pour_masks_key`). A router's own counts do not depend on
  the cache any more (a first attempt that stops on all its work,
  `limits_count_all`, stops where it did when it built them).
- *The ladder charges each shared part once a phase.* A phase is the
  probes, a pitch's rungs (with the last rung's seed retry), a seed
  round, a step (`Ladder::new_phase`). In the rungs' order, the first
  router of a key charges the shared parts, the others their own maps
  (`Ladder::lattice_charge`); seeds charge their own maps once a seed
  and the shared parts once. Charging happens in the order the ladder
  judges the attempts, which does not depend on how many ran at once,
  and a rung that ran in a wave and that the order never reached
  charges nothing, as before. The total is what a single-threaded run
  of the ladder charges: one router after the other, each phase building
  each key's maps once and every later router of the phase sharing them
  (with the canonical price, a single-threaded run and a run with three
  side by side charge the same; charging a found map as a whole build
  instead would charge four lattices for four probes that build one,
  and seeds `retry` builds for one).
- *What is charged once is built once.* The ladder holds the maps of
  every rung it starts for the rest of the pitch, a lone rung's as a
  wave's (`rung_wave`): the pitch's later rungs and the last rung's seeds
  find them. The probes hold theirs once the first probe's router
  passed the memory gate, and the regular pitch after them goes on with
  the probes' phase (the same boards: its rungs find the maps, `carried`;
  without that a probed board's regular rungs rebuilt them, which x1404
  charged as found when the last held maps were still in the process); a
  seed round and a step hold their own (`route_seeds`, the router). Before the seed retry the
  pitch releases every rung's maps but the retried rung's. Nothing else
  keeps maps: the cache's entries are weak (until now the last maps
  built stayed for the process's life, `cache.1`: a router's worth after
  a 0.05 mm rung, and one set across a layout's move routers), and the
  pours' masks are held by the holds alone (they were the last four
  boards' for the process's life). `wall_accounting.lattice_work_seconds`
  counts what the holds built beside the routers' own builds
  (`physical_build_work`), so it is the charged lattice work where the
  ladder ran every router it charged (it still counts builds outside the
  ladder's charge: rungs of a wave the order never reached, routers built
  before the ladder).
- *One lowering an attempt.* A rung's board is lowered once, for its
  hold, and routed as lowered (`LadderRunner::route_lowered`); a wave's
  hold lowered its rungs a second time before (and only a wave of two or
  more: the lowerings charged followed the side-by-side count too). Seeds
  share one lowering and charge it once (it was charged `retry` times).
  The probes' boards are lowered for the probes and the regular pitch's
  rungs lower theirs again: two lowerings done, each charged once.
- *The memory.* The probes' memory gate counts the other rungs' static
  maps held beside the two routers (`too_small_for_beside`: the distinct
  keys beyond the first router's, at its `statics` size), and nothing is
  held or charged before it passes. `MemoryLimits` names the static maps
  and pour masks the process holds (`held: static maps N MB, pour masks M
  MB` in the `memory limits:` line; `held_maps_bytes`); they are in the
  resident size every limit's free memory is measured from.

The determinism check (VortacDesign, MSign, ChengminLi alone, with
`PCB_MAKER_MEMORY_MB` small enough for one router at a time and large)
and the tiers are in docs/changelog.md (ladder review fixes). Smaller
fixes of the same review: a negotiation bound of nothing is one search
expansion (`negotiation_expansions`; 0 means no limit, and the bound
held only because the work deadline was reached too), the router's
`run_work` is search work against search work (the via reduction's extra
allowance compared search work left with half of all the work), and
`PCB_LADDER_SEARCH_ONLY` names what `PCB_LADDER_UNCHARGED_SPOKES` did
(the search alone, not only the spokes).

**The search's calibration** (2026-10-16). `EXPANSIONS_PER_SECOND` is
the reference's rate: search expansions per second of a router's phases
(negotiation, resolve, clean-up, via reduction, thermal spokes,
stitching: their wall clock without the pour analyses, which count by
their own model) on the 5950X nearly idle (load under 10 at the start),
one search thread (`RAYON_NUM_THREADS=1`), route mode on the v16
placements. Measured on x1150 (main 8e8b4ba plus the `profile rate:`
line), each board pinned to eight physical cores (the ladder's probes and
rungs still run side by side as std threads), five routers summed on Sisu
and jetson:

| board | expansions | phase seconds | per second | inside `search` | pour work / wall |
|---|---|---|---|---|---|
| Castor | 611.7M | 183.4 | 3.34M | 3.81M | 29.1 / 19.4 s (1.50) |
| jetson | 3555.5M | 1101.2 | 3.23M | 3.57M | 16.4 / 16.1 s (1.02) |
| k30-SBC | 638.5M | 195.2 | 3.27M | 3.58M | none |
| Sisu | 2458.5M | 801.4 | 3.07M | 3.49M | 33.6 / 30.6 s (1.10) |
| urchin | 56.2M | 20.3 | 2.78M | 3.17M | none |
| spell_tome | 2.3M | 1.0 | 2.27M | 3.27M | 1.6 / 1.7 s (0.93) |

The median, 3.15M, is the constant since 2026-10-16 (it was 2.0e6, an
idle eight-core machine's negotiation before four speed rounds): a
second of work is now a second of the phases' wall clock on the
reference at one thread, where it was 0.6-0.7 s. Every budget in
seconds buys about 1.57 times the expansions it bought. Inside `search`
alone the rate is 10-15 % higher (the phases' rest: the corridor planner,
rip-up, stamping, conflict bookkeeping). `PCB_ROUTER_SEARCH_RATE=1`
times every search for that column; the verbose `profile rate:` line
prints the phases' expansions and seconds, the pour analyses' work and
wall, and the constant. The pour model (`POUR_*`) was checked, not
changed: work over wall 0.93-1.10 on spell_tome, Sisu and jetson, 1.50 on
Castor (its analyses ran faster idle than in their loaded calibration).
The reference is the fastest the machine runs a thread: under a tier's
load (all cores busy, SMT siblings, the all-core clock) the same work
takes longer (see benchmarks/agent-tasks/README.md, "How to measure",
for the tiers' parallelism). Since 2026-10-21 the constant is the
reference's rate, which the start-up calibration scales to the machine
(below).

**After the guard** (2026-10-15): once the layout's wall-clock deadline
has passed, the layout goes straight to finishing (`WallStop`,
`LayoutStep` in board_layout.rs): the board kept so far, its labels,
KiCad's check and the report run; the placement race, the move trials
(checked before each trial, not only each move), the resume, the polish,
the final ladder and the attempt after the narrow-signal attempt do not
(each skip noted as a `layout` cut). Inside a router, a negotiation that
resumes from an earlier one (a resume, a move's trial, the via reduction)
does not start an iteration past the deadline, and the pour stitching
leaves stranded pads stranded instead of their hard reroute; a fresh
router still negotiates once (without it there is no board). In the
ladder only an attempt that could beat the kept boards is checked
(above). v16's poly_kybd_split72_left ran three move trials of 60-200 s
after the deadline and was killed at 1800 s with no result.

**The machine's rates** (2026-10-21, `work_rate.rs` in pcb-kicad). The
rates that turn seconds into work were the reference's constants, so a
second of work was a second of a 5950X core nearly idle, and any machine
slower than that by more than the layout's 10 % margin (a laptop, a CI
runner, the macOS snapshot's machine, this one above load 45) had every
non-trivial layout cut (exit 3): 0xcb-static at 40 s pinned to one
logical CPU ended at 20 s of work after 50 s of wall clock. Now every
`route-kicad-board`, `layout-kicad-board` and `place-kicad-board`
measures the machine first (about a second, once per process, before
the run's clock starts): a fixed search on a canned lattice (40 x 30 mm,
two layers at 0.1 mm, 24 crossing nets and keepouts, 240 k nodes and
about 50 MB of maps, beyond the caches as a board's are) on one thread,
five rounds; and a fixed placement (40 parts, 20 global iterations, 15 M
units) as three copies at once on the process's threads, the placement
race's first batch, three rounds. Each workload's work is the same on
every machine (a unit test checks), so the fastest round's rate against
the same workload's on the reference
(`REFERENCE_CANNED_EXPANSIONS_PER_SECOND` 6.9 M,
`REFERENCE_CANNED_PLACER_PER_SECOND` 155 M, measured here at load 12)
scales the reference's rates (`REFERENCE_EXPANSIONS_PER_SECOND` 3.15e6,
`REFERENCE_WORK_PER_SECOND` 150e6) to this machine's
(`set_expansions_per_second`, `set_work_per_second`). The pour analyses'
nanoseconds and KiCad's check model, both measured on the reference,
scale with the search's rate (`reference_time_scale`). The placer's rate
is measured with three copies at once because the placement race's
clock counts its seeds as the reference's eight threads run them: with
fewer threads than three (one pinned CPU) a seed gets that share, and
the placer's rate says so; the search's work is counted as one thread
does it, so its rate is one thread's. The fastest round counts, not the
median: another job's burst on the core or its sibling halves a round
for a moment (pinned at load 16: 5.4, 5.0, 5.8 and 2.96 M a second, one
process after another), and a layout's minutes average such bursts.

The work counters are unchanged, so a run is byte-identical for given
rates, and results now depend on the measured rates unless they are
pinned: `PCB_WORK_RATE=<expansions/s>[,<placer units/s>]` (a search rate
alone scales the placer's alike; `3.15e6,150e6` is the reference's), or
the router settings `work_rate` and `placer_work_rate`
(`PCB_WORK_RATE` wins). A bad value is an input error (exit 2). The
first line of a run is `work rate: ...` (applied, source, measured, the
canned rates and their share of the reference's); every verdict has
`work_rate`, `placer_work_rate`, `work_rate_source` (`measured`,
`PCB_WORK_RATE`, `config`, `reference`), `work_rate_measured`,
`placer_work_rate_measured` and `work_wall_ratio` (work over wall), and
the `layout: done` line prints the ratio and the rates. To repeat a run,
pin the rates its verdict recorded. The tiers pin the reference's (run.py
`--work-rate`, default `3.15e6,150e6`; `measured` lets each binary
measure), so rows compare across machines and loads; the rows carry the
verdict's rate fields, summary.json the setting and the medians of the
measured rates. `pcb-maker calibrate-work-rate [rounds]` prints the
measurement alone.

The layout's wall guard is a config default now (`wall_guard`, 2, was the
const 1.1): with the rates measured, a second of work is about a second
of the machine's wall clock, so the guard is only a margin for load and
for what the work does not count (KiCad's checks beyond their model, a
board whose placement units run faster or slower than the canned ones,
the load changing after the measurement), not a budget; a run that hangs
still ends. run.py's timeout follows it: the budget times the guard plus
300 s, at least 1800 s. `PCB_ROUTER_NO_WALL_GUARD` still drops it.

**The guards stretch for pinned rates** (2026-10-21). With the rates
pinned (`PCB_WORK_RATE`, the router settings `work_rate`), a second of
work is a second of the pinned machine's, not of this one: the tiers pin
the reference's, and at a load of 25 this machine measures 0.4 to 0.6 of
it, at 60 0.3 to 0.4. Every wall-clock guard was then a budget in
disguise: route mode's 90 s guard of a 30 s budget held 32 s of work
only above 0.36 of the reference, so the boards that used their budget
(OpenDeck, brickpico, Wonkle, ChengminLi, megaduck) were cut whenever
the tier ran beside other work, and a layout at a work-over-wall ratio
of 0.32 to 0.42 (v18b's four cut rows: katia, Batch_1, Quanta75, the
0xCB panel, measured at 0.41 to 0.74 of the reference's search rate and
0.20 to 0.40 of its placer rate) reached its 2 x budget. Now the guards
stretch by the pinned rate over the measured one
(`KiCadWorkRate::wall_stretch`: the search's in route mode, the larger
of the search's and the placer's in a layout; at least 1, at most
`MAX_WALL_STRETCH`, 4, or `PCB_WALL_STRETCH_MAX`; 1 with the rates
measured): route mode's `deadline_seconds`, the layout's `wall_guard`
and the router's `GUARD` (`guard()`). The `work rate:` line and the
ladder's line say so; the verdict has `wall_stretch`, and run.py copies
it into the rows. run.py's timeouts allow for the cap when it pins the
rates (`WALL_STRETCH_MAX`: the intake's 180 s for a 30 s budget becomes
420 s, a layout's 3300 s 12 300 s); they are hang guards, the binary's
own guard ends a run first. A machine slower than a quarter of the
pinned rate is still cut, and says so.

What remains load-dependent: the measured rates themselves (pin them for
comparisons), a run that reaches the wall clock first (a machine that
gets much busier after the measurement), and the harness's timeout. Both
are reported as above.

**The order of the file** (2026-10-16): the lowering takes pads and
obstacles in the order the board holds them, and that order reached the
board (homebrew_486's race kept another seed from a file that differed
from another in pcbnew's order of each footprint's graphics). Every
command that places or routes sorts the board as it reads it
(`canonical_order::parse_board`): footprints by reference (natural
order), then uuid; in each footprint the pads by number, then position,
the graphics by layer and content, the zones by content, each kind in
the places it held. So two saves of one board (KiCad's, the import's,
the schematic flow's) route alike, and the boards written are in that
order (the placed and the routed board are byte-identical from any input
order). A save may also spell one angle two ways (a footprint at -180 or
180, a pad at -90 or 270 relative to it): pads, rotations and rectangles
take their sine and cosine exact for quarter turns
(`geometry::degrees_sin_cos`), so the two lower alike. Top-level graphics, zones, tracks and the net list are not
sorted (KiCad's save sorts those too, by uuid; nothing measured here
followed them).

## Known limits / next steps

- Through vias only: inner layers are routed (and take the automatic
  planes on four or more layers), but there are no blind, buried or
  micro vias.
- Existing tracks and vias (route mode keeps them, `existing_tracks`) are
  fixed copper of their net; the pads a piece of them joins are merged
  into one terminal (`board_router/kept.rs`), so the connection counts as
  made, but new routes join the group at its first pad, not along a kept
  track (docs/agents.md, "Pre-routed nets").
- Only pours covering at least 10 % of the board are connected through;
  smaller zones are treated as routing. Existing arc tracks are lowered as
  their chords, grown by the flattening tolerance (refused before 2026-10-21).
- 45° lattice output, no any-angle/arc post-processing yet; the
  straightening only takes bends out on the lattice.
- Rules come from the `.kicad_pro` (net classes, patterns, minimums) and
  the `.kicad_dru` ("Project rules" above); clearances are per net and,
  where a rule names two sets of nets, some item kinds or some layers,
  per pair ("Rules beyond the classes"); rules on areas, creepage and
  lengths are reported, not kept.
- Nets with disjoint search windows are routed in parallel; on large boards
  with long nets almost nothing is disjoint and the router is effectively
  single threaded. Jacobi-style parallel routing of overlapping nets (each
  against the current copper, then all re-stamped) was tried and converges
  far worse (Interf-U: 448 s and 44 vias against 81 s and 28 vias); the
  option `jacobi_batch` was removed on 2026-10-16. Speculative routing
  (the next batches of an iteration routed side by side against the board
  as it was, a route kept where nothing it read had changed since, the
  same result as routing in turn; also for the clean-up's single-net
  trials, and a variant keeping a route unless its own path met changed
  copper) kept too few routes to pay (Castor 16 %, k30-SBC 8 %: the
  negotiation reroutes the nets of one congested area one after another,
  each reading what the one before changed). It was off and was removed
  on 2026-10-16 with its switches (`PCB_ROUTER_NO_SPECULATION`,
  `_SPECULATE_PATHS`, `_SPECULATION_CHECK`, `_SPECULATION_NO_TILES`,
  `_CLEANUP_WINDOW`); the code (`route_batches`, `ReadSet`,
  `clean_up_window`) is in commit e256657, which added it, and on main up
  to be4b012. Hard thermal guards (a pad's last two spoke corridors closed
  to other nets) and stitching vias during the negotiation, both measured
  worse and off, went with it.
- No gate swapping, no differential pairs, no length tuning. Pin swapping
  is a separate step before routing ([pin-swap.md](pin-swap.md)).
