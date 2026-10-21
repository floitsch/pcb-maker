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

A config file that only tunes the router (no `connection_rules`) still takes
its rules from the project. `{"frame_directory": "<dir>"}` additionally
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
corridors), `search` (the lattice A*), `update`, `batching`, `global`,
`driver` (the phases, budgets and clocks), `fixed`, `negotiate`,
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
   ColdFire yet. `fixed_plane_stubs` connects every surface pad of a pour
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
   43 pads.
8. **Cleanup.** Each net is rerouted alone with pure geometric cost and a high
   via cost; the result is kept only if strictly cheaper.
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
  pad, so only a track from it into the fill joins it. The pour masks
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
is not exposed: a track reaches it from outside.

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
  layer the box is an opening. The intake has no text box on copper or
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
  pad's other shapes are copper of its net on their layers beside it
  (`padstack_variants`), and the back's mask aperture is the back
  copper's. adrianmarple's Raspberry Pi header (1.2 mm on F.Cu, 1.778 on
  B.Cu) and dennisss's lens mounts (3 and 4 mm): B.Cu tracks shorted and
  bridged to the larger back copper.
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
  than KiCad asks. For clearance, hole clearance, edge clearance and hole
  to hole, a rule does not count where a later rule with a minimum of the
  same constraint surely matches (JLCPCB's template ends with "Min.
  Spacing" 0.127 mm on everything, after "Pad to Track" 0.2 mm: KiCad
  checks 0.127 between pads and tracks, AMLing2's drone carrier); the
  nets are grouped by the later conditions too. Lowered: `clearance` per net (a rule on one set of nets
  against every net raises that set; against every pair, every net),
  `track_width` min, max and opt per net (a rule's minimum also floors
  the neck-downs and the narrow-signal width of the nets it matches: the
  router's `width_floor`), `via_diameter`, `hole_size` (vias) and
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
  `approximated` (kept more widely: every layer, SMD and plated pads
  alike), `no_effect` (the classes and board setup keep at least as much,
  or nothing routed matches), `not_routing` (silk, text, courtyards, pad
  holes, zone connections, assertions), `skipped` (severity, or a layer
  the board lacks) or `unmodelled` (creepage, length, skew, diff pairs,
  physical clearances, disallows other than blind, buried and micro vias,
  rules on what is unknown). `rules.unmodelled` lists the last one line
  each; the verdict's next_actions name them when a route is incomplete,
  and `drc-kicad-board` prints them on a `next:` line when KiCad finds
  errors.

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
attempt builds it; `maps_mb` in board-router.json): 1.7 times the maps
and 460 MB beside them resident, 2.0 times and 550 MB of address space,
times a margin of 1.25 (`router_peak_mb`). Measured with one router at a
time and one thread (the whole process's peak, x1225, 2026-10-21):

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
neck repair's tracks are the minimum rounded up to a micron. At warning
severity (KiCad's default) there is no floor by default: a warning does
not fail the pass rule, and on OpenESC (minimum 0.16, classes 0.09, the
only corpus boards where it matters) the floor cost routing in route mode
(4in1-mini 52 to 89 unconnected, 4in1 104 to 121) for 199+ warnings
gone. `PCB_MIN_CONNECTION_FLOOR=warning` floors warnings too, `off`
never; `ignore` never floors. `board-router.json`'s `rules` names the
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
at a tenth of it (30-120 s; `direct_narrow_budget`), its work deadline the
layout's end; the layout keeps it if KiCad finds it better than the first
route. Far from complete is the old "many open" (more than 5 % of the
terminals, four layers with pours: no moves, no resume, no polish) or,
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
deadline; the layout keeps the better board as before (KiCad's open
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
overhead" below). `PCB_LADDER_UNCHARGED_SPOKES` and
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
counts a modelled cost in the layout's work: `CHECK_SECONDS` (3.6 s) plus
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
  forms, the last built kept and the others while a router holds them;
  `Config::share_static_maps`). A router that shares them counts its own
  maps only: `BuildCounts::shared_map_nodes` at
  `BUILD_SHARED_MAP_NANOS` (12 ns a node; fitted on the shared builds of
  eleven probed boards at 4 s, the fastest of each board's repeats scaled
  by the measured share, 11.7 over their sum, 7.0 to 20.6 a board) and
  its nets' preparation. Before routers start side by side (the probes,
  a wave of rungs, the seeds) the ladder holds their maps in rung order
  (`hold_static_maps`, `LadderRunner::hold`), so that which router starts
  first does not decide who counts the build: the probes' build is
  counted once with the probes, a wave's with the rung that is first of
  its maps, the seeds' spread over them. The connected pours' masks
  (`compute_pour_masks`) are held with them by the board's pours
  (`POUR_MASKS`): the three connected probes lower one board. The routing
  is the same (`PCB_ROUTER_NO_SHARED_STATICS` builds them all anew).
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

What is left: the probes' search overshoot (a probe stops after the
iteration that crosses its share: MSign's four probes searched 5.1 to
5.2 s on shares of 3.75), each router's own maps (MSign 0.19 s a probe),
the connected probes' pour refresh at the analyses' remaining cost (the
nodes inside the pour and the spoke queries: Vortac 1.9 to 4.5 s a
probe), the key of the static maps (the obstacles' debug form, in the
fitted constant).

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
- 45° lattice output, no any-angle/arc post-processing yet.
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
