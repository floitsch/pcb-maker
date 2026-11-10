# The router

The router turns a placed KiCad board into a routed one. It is two crates:

- `crates/pcb-router` is the format-free core: the lattice, the negotiated
  search, the repairs, the pour model and the exact verifier. Its input is
  a `pcb_router::Board` (`board.rs`), its output a `RoutingResult` of
  branches per net. `router/` has one file per part of `impl Router`.
- `crates/pcb-kicad/src/board_router/` is the KiCad side: it lowers a
  `.kicad_pcb` and its project rules into a `Board`, runs the attempt
  ladder, writes the copper into the board, runs KiCad's checks and writes
  `board-router.json`.

```sh
pcb-maker route-kicad-board <source-directory> <board-id> <output-directory> [config.json|auto]
```

`route-kicad-board` is *route mode*: it routes the placement it is given.
`layout-kicad-board` places the board and routes it with the same ladder
inside a layout's work budget ([placer.md](placer.md)); where the two
differ, this document says so.

Every key of the router config, with type, default and doc comment, is in
the generated [config-reference.md](config-reference.md); environment
variables are indexed in [handover.md](handover.md). The project's rules
are the base of every run: a config file's key replaces the project's
value of that key, and a file setting `connection_rules` or
`default_rules` replaces the project's classes
(`project_rules::router_config_over_project`). A file asking for widths or
clearances below the project's is refused (exit 2) unless it sets
`allow_below_project_minimum`. `frame_directory` writes the board after
every negotiation iteration (`docs/images/animate.py routing` makes a GIF).

## A run in order

1. **Preparation** (`route.rs`, `route_kicad_board`). Requested net
   classes (`net_classes`), the width floor, pin swaps (`pin_swaps`,
   [pin-swap.md](pin-swap.md)), added pours (`add_pours`) and
   `existing_tracks: strip` go into a copy of the project. Differential
   pairs (`with_pairs`) and length rules (`with_lengths`) adjust the
   config. The via-in-pad plan is made once.
2. **The ladder** (`ladder.rs`, `Ladder::run`). Each attempt lowers the
   board (`lower.rs`), builds a `Router`, negotiates, repairs and polishes
   (`pcb-router`, `router/finish.rs`), writes the attempt (`attempt.rs`,
   `write_attempt`) and, where it could win, checks it with KiCad. The
   ladder keeps the best board.
3. **Finishing the kept board** (`route.rs`). Pads KiCad finds starved are
   set solid in rounds, connection-width findings get neck tracks, the
   pours are measured and `board-router.json` is written. The board lives
   in `<out>.partial` until then; one rename makes it the output, so a
   killed run leaves no result.

## Lowering a KiCad board

`lower()` in `board_router/lower.rs` turns the parsed board, the attempt's
config and whether pours connect into a `Board`: outline, clearances, rule
classes, the neck width, obstacles, nets with terminals, planes, the
via-in-pad lists, pin-row gaps and terminal copper. A board beyond the maps
(65,533 nets or 32 copper layers, `grid::check_capacity`) or a lattice that
does not fit memory is an input error. The board is read in canonical order
(`canonical_order::parse_board`), so two saves of one board route alike.

### Project rules

`crates/pcb-kicad/src/project_rules.rs` and `project_rules/dru.rs` read the
`.kicad_pro` and `.kicad_dru`.

- Board-setup minimums the project omits take KiCad's defaults
  (`KICAD_MINIMUMS`), reported in `rules.kicad_defaults`. Class vias are
  floored by growing the diameter, never by shrinking the drill.
- A net takes every class assigned to it; each value comes from the
  highest-priority class that has one, else Default. Two nets keep the
  larger of their clearances.
- Custom rules are evaluated three-valued (a condition holds, fails or may
  hold). Per net the router keeps the largest value of every rule that may
  match, never below the class (`apply_custom`, `Resolved::new`), so it
  never makes a KiCad error from a rule it cannot decide; the price is
  that it may refuse room a designer opened by loosening a clearance in an
  area. Lowered: clearances, `track_width` (per layer where
  layer-qualified), `connection_width` as a width floor, via and hole
  sizes, annulus, edge (`edge_clearance_mm`), hole clearance
  (`hole_clearance_mm`), hole to hole, and a `disallow
  track` that surely holds. Rules below error severity are not lowered.
  Every rule is reported in `rules.custom_rules` with a status.
- The minimum connection width floors class widths, the narrow-signal
  width and the neck width at error or warning severity
  (`min_connection_mm`; `PCB_MIN_CONNECTION_FLOOR=errors|off`).
- Where the project rates `solder_mask_bridge` an error (KiCad's default;
  `solder_mask_bridge_severity`), mask openings become keepouts of their
  net and other nets keep `solder_mask_to_copper_mm` from apertures.

### Rules beyond the classes

`pcb-router/src/rules.rs` (`NetRules`, `ClassRules`) carries what a class
cannot: per-net width floors, per-layer widths, clearances per pair of net
groups, item kind (track, via, pad, plated pad) and layer, and same-net
hole to hole. `Router::with_neck_classes` splits the classes by these rules
and adds a neck class for every class wider than the board's neck width.
`Config::net_rules` of `None` routes as if the module did not exist. A rule
on `isPlated()` items does not wall SMD pads (`rules::PLATED`).

### Board edges, texts and copper of no net

Unplated slots count as board edge; Margin-layer graphics are keepouts at
the edge clearance. Copper of no net keeps the larger of its neighbour's
and the Default class's clearance (`no_net_clearance`). Dimensions, text
boxes, cached TrueType texts and knockout plates are lowered by their drawn
shapes (`board_router/geometry.rs`). Padstacks give per-layer copper; a
plated through-hole pad is copper on every layer. Net-tie pads stay
terminals of their own nets and the bridge is copper of no net carved
around them (`carve_net_tie`).

### KiCad's connectivity

Terminals are the items KiCad counts, so the router's unconnected count
agrees with KiCad's: same-number pads that overlap on a shared layer are one
terminal, footprint jumpers are one terminal, a net-carrying copper graphic
is a terminal unless it lies on a terminal of its net. The board's own
tracks and vias (`existing_tracks`, default `keep`) are fixed copper of
their net, and the pads a piece of them joins merge into one terminal
(`board_router/kept.rs`); new routes join such a group at its first pad.

### Hole clearance

Other nets' tracks and vias keep `hole_clearance` from every via drill and
plated pad drill, which binds where rings are thin; where the project rates
hole to hole an error, a via also keeps it from its own net's pad holes.
The stamp radii (`router/build.rs`) take the larger of copper and hole
rule.

### Vias in pads

No via goes into an SMD pad except the exposed pad (the largest SMD pad, at
least twice any other, inside the box of three other pads): an open hole
wicks the solder and KiCad does not flag it. The lowering lists the SMD
pads in `Board::no_via_pads` and every via asks `via_allowed`, so the rule
holds in every phase. A touch electrode may hold its net's vias inside its
mask opening. `via_in_pad: true` drops the list (for filled, capped vias).

### Vias in BGA pads

`board_router/via_in_pad.rs` (`plan`) puts vias into enclosed pads of
admitted footprints. A footprint is admitted when the designer's board (or
the `--reference` board) has vias in its SMD pads (`designer_entries`),
when the config lists it (`via_in_pad: ["U106"]`), or with `true`. A pad
gets a via when a fine flood finds no track or class via that leaves it;
the via takes the designer's size in that footprint, else the largest that
keeps every clearance, and is written capped and filled. Reported in
`vias_in_pads`. The designer's vias are read as the designer's statement
about the fab: MIDAS-MK1.1 went 5 to 0 unconnected and nothing else moved.

### Nets split by sub-board

On a panel, same-named nets of different boards share no copper.
`sub_boards.rs` (`Pieces`) finds the outline's pieces (necks under 2 mm,
mouse-bite rows and V-cuts cut it), and `split_router_nets` gives each piece
its own net unless a terminal or pour joins two pieces; `excuse_across`
moves KiCad's cross-piece items to `unconnected_across_sub_boards`.
Setting `sub_board_nets` (on).

## The lattice

`Grid::choose` (`pcb-router/src/grid.rs`) picks a pitch and phase per axis
from `pitches` (0.127, 0.125, 0.1 mm; KiCad key `grid_pitches_mm`). Pad
centres vote (only pads narrower than 0.35 mm, when there are any), and so
do tight channels: gaps between fenced-in pads that the narrowest track
fits with less than half a pitch to spare. All channels together weigh
twice all pads, because one missed channel walls in a BGA's inner balls
(ngdevkit's 0.75 mm BGA leaves 0.01 mm of slack: 0.125 mm reaches its
balls, 0.1 mm does not). Moves are the eight directions on a layer and a
via between any two layers.

**Static maps** (`StaticMaps::build_with_rules`, one set per rule class,
shared by routers of the same inputs: `share_static_maps`). Per layer and
node: free, blocked, or the one net allowed. Nodes near an obstacle get
exact per-edge checks instead of a blanket margin.

**Occupancy.** A routed net stamps, for each class that queries it, the
zone where another net's centreline or via may not be. The radii are exact
and a disc covers the cells strictly closer, so two tracks at exactly their
clearance are legal, as in KiCad (0.4 mm pitch, 0.2 mm tracks and
clearance). Every node also stamps the start cells of the diagonal steps
that would pass it too closely; the two stamps together are exact, and only
parallel diagonals stay quantised (to pitch/√2). Stamps are
reference-counted per net, so rip-up is an exact decrement. A net's search
ignores its own stamps (`mask_own`).

### Lattice lines at pin rows

An exact-fit 0.4 mm row (0.2 mm tracks and clearance) can only be left
along a line through the pad centres. `crates/pcb-router/src/pin_lines.rs`
lists the exit lines a straight track of each row pad's class can take and
those the regular lattice misses (`missed_exit_pads`). `warp` places them
per axis: a part whose exits share a phase gets that phase over its rows
plus 1 mm (a band, which the narrow pads in it must vote for), other exits
get a line added or a regular line moved onto them. The lattice then holds
per-axis coordinate tables (`Grid::warp`); a step costs the regular step
whatever its length, so the heuristics stay admissible. Bands beat single
lines because a single line makes the whole column irregular (RP2040_base:
21 open with single lines, 5 with bands).

The router's `pin_lines` is off by default; the KiCad key `pin_lines`
(true: every attempt, false: never, unset: the ladder decides) and
`PCB_ROUTER_PIN_LINES` set it. Lines for every attempt are not the default
because any lattice change moves routes by chance on boards near their
limit (+3 −2 strict on the designer tier); the ladder takes them where open
pads sit at missed rows ("The rows decided at the probes").
`PCB_ROUTER_PIN_LINES_PROBE` prints what the pins ask for.

### Fine-pitch escapes

A pad gets the legal nodes well inside its contact (`router/prepare.rs`). A
pad too narrow for any gets escape nodes (`escape_nodes`,
`router/escapes.rs`): up to 12 legal nodes within 2 mm, each reached by a
straight or bent stub from the pad centre, checked exactly
(`stub_is_clear`), at the track width else the neck width. With
`exact_escape_stubs` (on) a stub along a lattice
line stamps as lattice nodes and others stamp their measured footprint;
straight stubs are preferred. Discs grown by the snapping error made
adjacent 0.4 mm stubs conflict for ever: exact stubs took hid-remapper 33
to 0 unconnected and the expansion tier 589 to 557. `escape_stub_mm` (0,
off) fixes stubs before negotiation; it gained nothing.

### Corridors of pocketed pads

When a pad's escape nodes lie in a pocket the lattice closes,
`Router::corridor_escape` (`router/corridors.rs`) searches a 5 µm raster for
a path the track keeps its clearances on (margins 10 down to 0.5 µm) to a
legal node outside the pocket, and keeps it as a polyline escape stub.
`corridor_escapes` (on; `PCB_ROUTER_CORRIDOR_ESCAPES`). A two-dimensional
pocket (OpenFC's U10.5) cannot be opened by a phase per axis; on 15 OpenFC
placements the stubs took 9 to 13 strict passes, every tier identical.
`PCB_ROUTER_CORRIDOR_CENSUS` prints one line per escape pad. A custom pad
whose anchor centre is not clear starts its stubs from up to four clear
points of its contact (`contact_starts`; Hub 12 to 4 open).

**What walls a pad.** A pad no path reaches is a *dead pad*.
`Router::dead_pad_wall` names, per layer, the nearest item at the contact's
best point, or what blocks stubs from there; it changes no route and is
written as `diagnostics.dead_pads[].wall`.

## The negotiated search

### Costs

`Router::search` (`router/search.rs`) is an A* over (layer, node). A trace
step costs the pitch times the layer's direction cost (even layers prefer
horizontal, odd vertical; across costs `against_direction`, 1.6) times (1 +
history) and (1 + present price × occupancy). On top come
`thermal_guard_cost` (10) on another net's spoke corridors, `plane_cut_cost`
(3) on another net's pour (impassable on an exclusive plane) and
`layer_preference_cost` (10) off a net's preferred layers; plus
`pin_row_cost` (2 mm) in another net's pin-row gap and `bend_cost` (0.1 mm
per 45°; KiCad key `bend_cost_mm`). A via costs `via_cost` (8) with the same factors, `plane_via_cost`
(2) toward a pour. The heuristic is an octile bound aware of layer
directions for up to 12 targets and an exact distance transform for larger
trees. In *hard* mode (the repairs) any occupied cell is a wall.

### Pin-row cost

`footprint_pin_row_gaps` (lower.rs) marks the gaps between adjacent SMD
pads of a part. A step into a gap by another net costs `pin_row_cost`
(2 mm; `pin_row_cost_mm`; 0 builds no marks).
Gaps up to 1.5 mm are charged everywhere, gaps from 1.5 to 2.6 mm only in
the clean-up and the bend round. At 2 mm the expansion tier went 625 to 592
unconnected; at 3 mm a gap costs more than a thermal guard and starved
thermals appear.

### Two-level search

Every search in the negotiation and the hard repair first plans on a graph
of (layer, tile) nodes, each tile 16×16 lattice nodes
(`Router::plan_corridor`, `router/planner.rs`): a tile costs more as it
fills, with its history and under another net's pour. The A* then searches
the planned tiles grown by 1 + reroutes/6 tiles (at most 10, retried once
three wider), then the window (the terminals' box grown with the
iteration), then the whole board (`route_net.rs`). Setting `corridors`
(router, on). Widening every six reroutes rather than three saved 10 % CPU
and took SNSP-CPU-01 171 to 98 unconnected; a fixed cap left it
unconverged. Window and board fallbacks are 0.5 to 10 % of the work, so
wider searches are not the bottleneck. The verdict's `corridors` counts the
searches per mode (`pcb_router::search_counts`).

`global_routing` (off) instead negotiates every net's plan over
tile-boundary capacities first (`Router::plan_globally`, `router/global.rs`)
and confines the negotiation's searches to it. Nothing sets it, and no
measurement on record shows a gain over the tile corridors.

### Pads no path reaches

A board-wide search that fails proves its open terminals unreachable for
the rest of the negotiation (only fixed things stop it), and they are left
out of the net's later searches. `apart_trees` (on) joins them among
themselves, so KiCad finds
one unconnected item instead of one per pad. Without the rule one miss
froze a net: SNSP-CPU-01 went 178 to 5 KiCad unconnected with it.

### The negotiation

`negotiate_from` (`router/negotiate.rs`). Pour nets route first, then nets
of up to eight terminals, then by window span; a seed jitters the order and
the history. Before negotiating, the driver fixes escape stubs and, where
the rung asks, the plane skeleton and plane stubs. Each iteration splits
the pending nets (all, then the conflicted ones) into batches of up to 64
whose windows do not overlap, rips up their conflicted branches, routes
them in parallel and stamps them; batches depend only on windows and merge
in order, so routes do not depend on the thread count. A net in conflict
for eight iterations running is ripped up whole, because its kept branches
may box the other net in. Fixed branches are never ripped up.

Each conflict adds `history_increment` (0.3) to its node and tile. The price
of sharing starts at `present_factor` 0.5 and grows by 1.5 an iteration to
`present_cap` 1e4. From iteration 2, pour pads the signals cut off are
routed to their pour's main piece (`pour_islands`, on). The negotiation
stops conflict-free; stalled (25 iterations without a new low of
conflicted nets, or 12 at the price cap: Sisu spent 54 % of its expansions
after stalling); at its work limit, deadline or wall guard; hopeless
(`abandon_hopeless`: over a quarter of the nets blocked with no path); or
at 80 iterations. `Router::probe(seconds)` negotiates under a work limit
and keeps its state; `resume` continues at the probe's history and price.

## Repairs

`Router::finish` (`router/finish.rs`) runs, in order: the hard repair, the
trouble-makers' repair, the clean-up, the length repair, the via
reduction, the bend round, the thermal spoke repairs, straightening,
compaction, stitching per pour net and manual spokes. A layout's move
trials skip the trouble repair and the polishing.

### The hard repair

`resolve_remaining` (`router/resolve.rs`) runs after every negotiation. A
net in conflict loses only its conflicted branches; an incomplete net
without conflicts keeps its tree. Pair links go first. Each net is then
routed hard within max(negotiation work, 60 s), and `force_connect` routes
a net through what is in its way and reroutes the nets it hit, kept only if
all end complete. Ripping nets up whole was worse: searched again from one
pad among copper that no longer gives way, MegaDrive left 19 signal nets
with no copper.

### The trouble-makers' repair

`Router::repair_trouble` (`router/trouble.rs`; `trouble_repair`, on).
Each net still open, most open terminals first, is
routed through other copper at a price of 20; the nets it overlaps lose the
crossed branches and are routed again hard, their corridors first. The
exchange is kept when the open terminals of all of them drop, or tie with
fewer nets open; refused when it displaces more than max(12, 2 × open
terminals) nets or meets a pour net or fixed copper; undone as soon as it
can no longer pass. Up to three rounds within max(half the negotiation's
work, 30 s). `PCB_ROUTER_TROUBLE_*` tunes it. It is skipped in move trials
because OpenFC then fit 5 trials instead of 11 and ended at 12 unconnected
instead of 2.

### The crossing census

`PCB_ROUTER_TROUBLE_WHY` prints every exchange's verdict and, when signal
nets stay open, the crossing census (`router/dive.rs`, no work charged):
every open pad of a fine-pitch row (`Router::fanout_plan` in
`router/fanout.rs` lists dense pads and their straight ways out), what
closes each way, and whether a row neighbour's target crosses its own. Both
modules are diagnostics; the steps they fed lost ("Tried and rejected").

### Clean-up and via reduction

`clean_up` (`router/cleanup.rs`) reroutes each complete net alone at a
geometric cost (no axis preference, no history, `cleanup_via_cost` 25, an
exact search), kept only if strictly cheaper and no pour pad is stranded
(`pour_aware_polish`); `cleanup_passes` 4.

`reduce_vias` (`router/vias.rs`) runs on a complete board: up to three
rounds renegotiate the nets with vias at a via cost doubled each round, kept
only if nothing opens, vias drop and no pour pad strands. A via comes out
only here, because renegotiation lets neighbours move.

### The bend round

`reduce_bends` (`router/bends.rs`; `bend_round`, on). On a complete
board each signal net with a jog
(two bends returning within 2 mm) or staircase (three or more) is rerouted
alone by the clean-up's search at `bend_round_cost` (1.0 mm per 45°), kept
if still complete with no extra via, at most 1 % longer, no extra staircase
or pin-row crossing, and fewer jogs plus staircases; growth of pour islands
or starved pads is traced by halving and undone. Budget 5 % of the route's
work. Renegotiating at a raised bend cost always failed (CyberKeeb vias 134
to 239); alone, the round took the designer tier's jogs from 2.59 to 2.27
times the designer's.

### Straightening

`straighten` (`router/straighten.rs`; on). The
search keeps no arrival direction and closes a node at first arrival, so
among equal shortest paths it often keeps zigzags, and the clean-up keeps
only strictly shorter reroutes. Straightening tries the two shortest
octilinear paths over spans of up to 8 corners and 15 mm of each one-layer
run and keeps the one with fewer bends, then fewer stair-steps. Every step
must be free by the hard search's rules and may not enter more guards,
spoke corridors, pin-row gaps or pour than before.

### Pin-row gaps and compaction in the clean-up

Gaps from 1.5 to 2.6 mm are charged in the clean-up and the bend round
only: in the negotiation they cost 46 unconnected on the expansion tier, in
the via reduction's clean-up 14 vias on CyberKeeb; in the clean-up alone
they took crossings 64 to 57 on 23 complete boards. `compact`
(`router/compact.rs`) runs inside the straightening's branch (so
`straighten` off turns it off too): the middle segment of a Z
slides toward other copper with length, bends and ends unchanged, kept
where it hugs other copper more. It raised bundling on 18 of 26 boards.

## Copper pours

A zone covering at least a tenth of the board is a *pour* (`POUR_SHARE`,
`board_router/geometry.rs`). The KiCad key `pours` (`auto`, `connect`,
`tracks`) and the ladder's rungs decide whether pour nets connect through
their pours or route as tracks. Connected, a pad inside its pour joins it;
a pour-net pad off its pour gets a stub and a via to the nearest free pour
node. `PourMap::build` (`pcb-router/src/pour.rs`) labels the pieces where
the pour's brush fits; `router/pour_analysis.rs` names the main piece.
Other nets pay `plane_cut_cost` on a pour, scaled by the layer's coverage
relative to the least covered layer. An *exclusive* rung (`exclusive_planes`) keeps one
inner plane (two from six layers) free of other nets' tracks. `add_pours` adds
zones (`pour_plan.rs`; entries name `net` and `layers`); `plane: true` keeps every net's tracks off the
layer, `keep_whole` keeps other nets out of named parts or boxes. After the
repairs, `stitch_pours` (`router/stitch.rs`) joins island pieces holding
pads to the main piece by vias.

### Thermal spokes as KiCad counts them

`Router::kicad_spokes` (`router/spokes.rs`; `kicad_spokes`, on) reproduces
KiCad 10's zone filler and starved-thermal check per pad, layer and zone. A
spoke runs from the pad centre in the pad's frame at the zone's spoke width
and counts where its end lies in fill (tested off the lattice by
`FillProbe`, `router/fill.rs`) and a brush fits from the pad's ring to it.
Spokes into an island touching nothing but this pad do not count; hatched
zones, merged pads, zone priority, reclaimed fill and island removal are
modelled. Required spokes come from the project's `min_resolved_spokes`.
The model's starved count equals KiCad's on the expansion tier, the
fingerprints and the dense boards. Repairs, all on:

- Thermal guards (`router/guards.rs`): other nets pay `thermal_guard_cost`
  on the four spoke corridors of every pad joining a pour by spokes.
- `free_thermal_spokes` and `free_lost_spokes` reroute nets crossing a
  starved pad's corridors or spokes, kept only if complete with fewer
  starved spokes.
- `add_manual_spokes` (`router/manual_spokes.rs`; `manual_spokes`) gives a
  pad the model finds short straight tracks into the main piece, kept when
  the model then counts them; a pad reaching only islands gets a via to
  the main piece. It ignores the work deadline, because starved thermals
  must be repaired however the budget went.

### Thermal vias, custom pads and necks

The *measured* model (`measured_thermals`, off while routing) adds what
the measurement of a written
board needs: thermal vias in an exposed pad use their own zone connection,
custom pads KiCad's spoke frame, and pieces joined through a neck the
lattice misses count as kept. It stays off while routing because the
routing model's caution is what the repairs rely on (exact in routing cost
the tier 7 unconnected and 2 passes). `measured_board`
(`board_router/pour_quality.rs`) measures the kept board and reports
`pour_quality` and `predicted_starved_thermals`.

### The solid step in rounds

On a checked board `solidify_starved_thermals` (`route.rs`;
`solid_starved_thermals`, on unless false) sets the pads KiCad finds starved
to a solid connection (matched by position) and checks again, up to
`SOLID_ROUNDS` (4) rounds while only starved thermals change; the best
board KiCad saw is kept. Rounds, because a solid pad changes its
neighbour's fill: k30-SBC's J1.39 and ghoul's RN3.3, then RN3.1 and RN3.2,
starve only after a round, and both boards pass with the loop. On an
unchecked board the model's starved pads are set solid. The step trades the
designer's thermal relief for a clean check.

### KiCad's connection width (necks)

KiCad's refill makes necks the model does not know (hatched thermal rings,
borders around teardrops). When the final check reports `connection_width`
errors, `repair_connection_width` (`crates/pcb-kicad/src/necks.rs`, via
`repair_necks`) has kicad-cli refill a copy, finds narrow passages on a
4 µm raster and lays minimum-width tracks across them, kept while necks
drop and nothing else gets worse; skipped past the deadline or without
KiCad. `pcb-maker repair-kicad-necks` runs it alone. The router itself
routes at the neck width within `neck_reach` (1.5 mm) of a net's narrow
pads; `neck_width_mm` defaults to the project's minimum track width.

### Plane skeleton

`plane_skeleton` routes each pour net first as a fixed tree on its main
pour layer (`skeleton_bias` 6; `route_skeletons`, `router/fixed.rs`), later
trimmed where solid pour covers it. It helps two-layer boards whose pours
the signals shred and costs room elsewhere, so it runs by default only as a
rung of a two-layer ladder after a connect attempt left pour pads open.
`pour_integrity` (0, off; two layers, `router/sever.rs`) prices steps whose
cut would split a pour; it did not keep the fence's pour whole.

## The attempt ladder

`board_router/ladder.rs` holds the ladder as explicit steps on a `Ladder`;
what routes, checks and measures memory is a `LadderRunner` (`route.rs`
`KiCadRunner`, or a scripted runner in `ladder_tests.rs`). `Ladder::run`:
`probes`, then `leader_ladder` in route mode or `rungs` otherwise, then
`seed_rounds`, then `steps`.

**Rungs** (`ladder_rungs`) are (connect pours, plane skeleton, exclusive
planes, plane stubs). No pours, or `pours: tracks`: one tracks rung.
`auto` on two layers: connect, connect with skeleton, tracks. On four or
more: connect with exclusive planes, connect, connect with plane stubs
(`fixed_plane_stubs`), tracks. Skeleton and stubs rungs run only when the
plain connect attempt left pour pads open. Exclusive planes come first
because ColdFire needs them whole (13 open against 43); video needs the
room (9 against 110).

**Pitches** (`ladder_pitches`): the regular lattice, then each of
`refine_pitches_mm` (0.075, 0.05 mm). A finer pitch is skipped when the
slowest attempt's work times the squared pitch ratio exceeds
`refine_budget_seconds` or the work left, or when one router would not fit
the process's own memory limit or 0.4 of the machine's memory.

**Seeds.** A seed perturbs the net order and the history; seed 0 is the
deterministic order. A rung that leaves connections open gets an immediate
retry of `retry_seeds` (4) seeds when it fits, and up to three seed rounds
of the best rung follow. `seeds` routes every attempt that many ways, from `first_seed`.

**Ranking and KiCad's checks.** An attempt's open count is the router's
unconnected terminals plus internal violations plus KiCad's unconnected
items; the rank is (open, starved thermals), vias, length. Attempts route
without KiCad; `judge` checks the first board, every board the router
finishes and any other whose router count could beat the best checked one
(`could_win`). An unchecked board never becomes the best. The ladder stops
at the first clean board, or after `ladder_budget_seconds` (1200 s; a
layout's work left).

**The ladder in code, and its faults.** Every attempt runs isolated
(`isolate`): a panic is logged and listed in `attempts_failed`, and the
ladder goes on with its best board; the run fails only when no attempt
produced a board (exit 4). `PCB_MAKER_TEST_PANIC_ATTEMPT` makes an attempt
panic for tests. Every attempt is recorded in `attempts`; `skipped` lists
what did not run.

### The probe ladder

No fixed split of the budget suits every large multilayer board: one needs
600 s of negotiation in its first rung, another the last rungs. On a board
of at least three rungs, four layers and 120 connections (`probe_ladder`,
on; no `negotiation_seconds` given), every rung is probed for `probe_seconds` (75 s of search; in route
mode all probes together at most half the budget). The fewest unfinished
nets lead, ties to the earlier rung; probes run side by side as memory
allows but are judged in order. The leader is continued (`resume`) with a
third of what is left, 60 to 900 s of negotiation.

### The leader first

In route mode on two or more rungs (`leader_first`, on) the ladder
spends its work on the leader's kind
first: a seed round of the leader, the leader on the finer pitches, then
the other rungs on the regular and finer pitches, each only if its probe
left at most twice the leader's unfinished nets (`LEADER_GATE_FACTOR`). A
layout's ladder keeps every pour mode per pitch. The kept attempt used a
median 15 % of the ladder's work while 28 % went to rungs that ended far
worse, and every extra pass came from another try of the best rung's kind:
on the designer tier 68 to 71 strict, none lost, 5 % less work.

### The rows decided at the probes

When the leader's probe leaves pads unfinished at rows the regular lattice
misses, `Ladder::lines_probe` probes its rung again with pin lines (at most
a third of what is left). The lines lead with at least 10 % fewer
unfinished nets (`LINES_PROBE_MARGIN`) and the rest of the ladder takes
them. Without the margin jetson took lines at 241 against 240 and ended at
60 open instead of 27. `PCB_LADDER_LINES_PROBE=0` turns it off.

**Lattice lines where the rows are open.** Without probes (two layers or
under 120 connections), `Ladder::pin_lines_from_here` decides after the
leader's run: if KiCad's open pads lie in parts with missed rows, the rest
of the ladder uses pin lines. RP2040_base went 36 to 5 open.

### A layout's rows decided by its probe's open pads

A layout decides at the race's kept placement (`race_lines_probe`,
`board_layout.rs`): when its probe left pads at missed rows, it probes again
with lines and every later route takes them. It decides by the pads, not
the count (RP2040_base counted 82 with lines against 75, yet its first
route ended at 11 against 38). On the 28 sweep boards the race can change:
13 to 14 strict, none lost. `PCB_LAYOUT_LINES_PROBE` selects the triggers;
the verdict's `pin_lines` says why.

### The narrow-signal and minimum-clearance steps

When the project's narrowest predefined width is narrower than some
Default-class net, the last step reroutes the best attempt with the
Default class's signal nets at that width (`narrows_class_of`); named
classes, custom-width nets, pour nets, grounds and rails keep theirs. Where
the project rates clearance findings as warnings the step also routes at
the relaxed clearance; otherwise a minimum-clearance step may follow.
`use_narrow_signals: false` turns it off. Work is kept back for it once
the first attempt leaves connections open (a layout:
`narrow_reserve_share` 0.75 of the first attempt's work, 240 to 600 s).
Narrowing named classes narrowed impedance classes KiCad does not check
and completed no board, so it was dropped.

## Determinism and the wall clock

Every budget is *work*, not wall time, so a busy machine routes the same
way. A second of work is `expansions_per_second()` search expansions
(`REFERENCE_EXPANSIONS_PER_SECOND` 3.15e6, a 5950X core) or the rate set at
start-up; placement has its own rate (150e6). Results depend on the board,
config, seeds and work rates. The thread count only changes how many
routers run side by side: probes, rungs and seeds are judged and charged in
order as if run one after another, and a rung the order never reached
charges nothing. Free memory decides only how many run at once; fixed
limits gate probes and finer pitches. A debug flag only prints
(`debug_flags_change_neither_the_route_nor_its_work`).

### The machine's rates

`crates/pcb-kicad/src/work_rate.rs`: route and layout runs measure the
machine at start-up (a canned search, five rounds, the fastest counted
because other jobs' bursts halve a round) and scale the reference rates.
`PCB_WORK_RATE=<exp/s>[,<placer/s>]` pins them over the settings
`work_rate` and `placer_work_rate`; the tiers pin `3.15e6,150e6`.
`place-kicad-board` uses the reference rates unless pinned or given
`--measure-work-rate`. Verdicts carry the rates, so a run can be repeated.

### All the work charged

Route mode's ladder stays within 1.05 × its budget. It charges search,
pour analyses and fill probes, lattice builds and lowerings, each by a
fitted model in reference nanoseconds (`BUILD_*_NANOS`, `POUR_*_NANOS` in
`router.rs`), plus the pair fallback and the length reroute; it charges no
KiCad checks. A router's limits are given in search work (a share times the
search's fraction of all work); the first attempt, with no fraction known,
stops on all its work (`limits_count_all`). A layout's ladder charges
search and KiCad's checks (`check_work.rs`).

**The probes' overhead.** Routers of the same inputs share static maps, and
the ladder charges the shared part once per phase in judging order
(`hold_static_maps`), so the charge does not follow free memory. Probes
keep search-only limits: with all work counted ducktop2's leader went 1 to
7 open. From its second iteration a probe's limit binds between batches, so
a probe cannot overshoot by an iteration.

### The guards stretch for pinned rates

Wall-clock guards catch a run the work model misses: in route mode a
negotiation stops at 4 × its limit in wall time, a phase at 4 × its budget,
and a layout at `wall_guard` (2) × `total_seconds`. With a pinned rate
faster than the machine they stretch by the pinned over the measured rate,
at most 4 (`wall_stretch`), so they do not become budgets in disguise.
`deadline_seconds` sets a hard wall deadline; `PCB_ROUTER_NO_WALL_GUARD`
drops the guards. Wall-clock stops are reported in `cut` (exit 3).

### Memory

A router's maps grow with nodes × layers × classes; `memory: N MB in maps`
is an upper bound and `PCB_ROUTER_MEM_PROFILE=1` prints the profile.
`MemoryLimits` (`board_router/memory.rs`) takes the tightest of RLIMIT_AS,
the cgroup, `PCB_MAKER_MEMORY_MB` or `memory_mb`, and half of MemAvailable.
A router's peak is estimated as (maps + 600 MB) × 1.25: what sits beside
the maps is roughly constant, and a ratio sized large boards three times
too big.

## Differential pairs

`board_router/diff_pairs.rs`. `diff_pairs` (or `PCB_ROUTER_DIFF_PAIRS`)
turns pairs on or off; unset, route mode turns them on where a custom rule
with `diff_pair_gap` or `diff_pair_uncoupled` surely holds for a pair
(`diff_pairs_choice`), and a layout keeps them off. The class's pair width
and gap never turn them on: KiCad checks the class gap only as a minimum and
most projects carry its defaults. Always on cost the expansion tier 556 to
576 unconnected; on the seven rule boards pairs took errors from 601 to 186,
mostly from the rule width and allowed layers. Without pairs nothing changes.

A pair is two nets KiCad's name rule pairs (`_P`/`_N`, `+`/`-`, `P`/`N`).
Its width and gap per layer come from the custom rule, else the class's
pair values, else the net's width and clearance; it uses only the layers
sharing its first layer's width and gap (`with_pairs`).

### Pair points and links

A *pair point* is where P and N pads sit side by side: on one part within
3 mm with nothing between, or on two two-pad parts side by side (series
capacitors, ESD parts). A spanning tree joins the points; each edge between
parts is a *link*, a net of the route whose track is the envelope 2w + g,
whose via is a disc for two vias, and whose terminals are two *launches*: the
first run starts from between the pads (turned up to ±90°, out to 4 mm)
where stubs, a straight lead and the envelope keep every clearance. A
launch whose straight stubs keep the clearances wins, and the link carries
them. `plan_pairs` adds the links after each lowering; the router routes the
other stubs as ordinary connections. Links are negotiated with everything
else, because pre-routed coupled runs took room the board needed; the hard
repair routes them first, even on a hopeless attempt, because a link left
open costs its pair a whole fallback.

**A pair point across a pin.** P and N of one part with another net's pin
between them make a pin point, used only for a pair with no other link and
refused when the pin could not leave past the stubs (`walled_pin`); without
that check MSign went 73 to 107 unconnected.

### Polarity

A crossed end has its pads against the run's order. Where the link carries
the stubs, a crossover goes into the lead: one net dives through a via,
crosses on the nearest other pair layer and comes back two offsets beside,
keeping room from neighbouring launches (`neighbour_room`). On with the
pairs (`CROSSOVERS_BY_DEFAULT`). Where the router routes the stubs it
crosses them itself, and the launch keeps `CROSSING_ROOM` (via + 2c + w)
beyond the stubs. A failed link leaves no crossover copper.

**A pair's stubs in the plan.** A crossover goes only where the link
carries the stubs: one whose stubs the router routed walled in a pad
(ch32v003's D+). With that rule and the crossing room, jetson's designer
placement went 21 to 27 coupled links.

### Stubs and copper

`finish_pairs` (in `write_attempt`) turns each routed link into copper: the
path offset by (w + g)/2 either side with a via of each net at a layer
change, trying simpler candidates in turn (hairpins removed, vias off turns,
jogs straightened), each checked by `verify`. A link is kept only when both
stubs reach its run (`stub_joined`); missing stubs of a kept link are routed
afterwards (`repair_stubs`). Failed links lose their stubs and the fallback
(`route_fallback`) routes the pair's nets as ordinary nets within 5 % of the
ladder's budget.

**Skew and length.** After the fallback `match_skew` meanders the shorter
net of each kept pair, measured with vias at stackup height
(`copper_heights`), to the rule's `skew` (else the pair's gap), or both
nets to a `length` minimum, written as a KiCad tuning pattern, which
KiCad's gap check skips (`PCB_ROUTER_DIFF_PAIRS_SKEW=0` turns it off).

**Return vias.** Beside each via of a kept link goes a via of the reference
net (`diff_pair_return_net`, else `GND`, else the poured `*GND` net with
most pads) that joins its pour on two layers (`place_return_vias`;
`diff_pair_return_vias`, on). Placed after the route, because reserving
room costs the links room.

### Giving up on a link

With `diff_pair_give_up` (off; `PCB_ROUTER_DIFF_PAIRS_GIVE_UP`), a link in
conflict at a high price whose conflicts stop falling, or with no path, is
given up during the negotiation (`router/give_up.rs`;
`PCB_ROUTER_GIVE_UP=min,stall,pathless,max[,present]`, default
8,4,1,16,100), and its pair routes uncoupled in the same attempt. Off
because ducktop2's left_io lost 3 of 6 coupled pairs while jetson stayed
above its unplanned count. The hook is `Attachment::give_up` (`board.rs`).

### The layout's own routes

A layout plans pairs only with `diff_pairs` set: every routing stage then
lowers with the links, and the written board runs `finish_pairs` and
`finish_lengths`. On jetson the planned first route left 60 open where the
unplanned one left 2, which is why it is not the default. The verdict's
`diff_pairs` reports found, coupled and fallen-back pairs, links,
crossovers, skews and each pair's reason (`pairs` in `board-router.json`);
`PCB_ROUTER_PAIRS_WHY` and `PCB_ROUTER_LINK_TRACE` print more.

## Length and skew rules

`board_router/lengths.rs`, `meander.rs`. `project_rules::length_rules`
lowers per net and layer the custom rule KiCad would apply for `length` and
`skew`. `kicad_length` measures as KiCad does (fitted to KiCad's QA board).
With `length_matching` (on; `PCB_ROUTER_LENGTH_MATCHING`), `match_lengths`
meanders complete nets outside coupled pairs, lengths first and then skew,
each meander checked by `verify` and written as a tuning pattern; findings
fell from 31 to 19 with unconnected unchanged. A maximum is not a hard bound
in the search (the A* cost is per path, a bound needs the whole tree):
`Router::repair_lengths` reroutes a net over its maximum through others
(`length_repair_present` 2), kept if at least 0.5 mm shorter with nothing
more open. The verdict's `lengths` reports each net against its rule. The
placer's `length_maxima` keeps a bounded net's pins within 0.8 of its
maximum.

### Length minima: the placer's share and the reroute

A net short of its minimum after meandering lacks room for bumps.
`length_reroute` (off) reroutes such nets with
other tracks kept further off (`reroute_shorts`), kept if all connect and
the total miss falls; the placer's partner is `length_minima` (off).

### Skew groups' layers

A rule limited to some layers (`length_rule_layers`: `designer` by default,
`rule`, `off`) gives its nets preferred layers (`admit_length_rule_layers`,
`board_router/layer_preference.rs`): the side the designer's tracks take,
or the rule's layers. A step off them costs `layer_preference_cost` (10)
times more, a soft disallow. Without designer tracks nothing is admitted.
Cost 10 took Sisu's skew findings from 7 to 3.

## Prepared fan-outs for BGAs

`board_router/bga_fanout.rs` (`plan`, in the lowering; `bga_fanout`, unset
means on). A grid array is 16 or more squat SMD
pads in four or more rows and columns at a pitch up to 1.27 mm. A ball's
*ring* is its distance to the nearest empty place (`rings_of`); rings up to
the number of tracks that fit between two balls leave on the surface. A
deeper ball whose class via does not fit a diagonal gap gets a dog-bone: a
stub and a via in the gap toward the nearer way out, at the largest size
that keeps every clearance, fixed copper written only where the route uses
it (`fanout_used`). A board with no plan routes exactly as with the switch
off; an array whose class via fits is left to the search. Glasgow's inner
balls had taken 47 % of its search work; the fan-outs took it from 55 to 15
open on the designer placement, and no other tier board moved. The placer
keeps the far side free under the vias (`via_in_pad_far_side`).
`PCB_ROUTER_BGA_FANOUT_WHY` prints the plans; `PCB_ROUTER_NET_WORK=<file>`
writes search work per net.

## Blind, buried and micro vias

`board_router/via_spans.rs` (`blind_vias`, off; `PCB_ROUTER_BLIND_VIAS`).
Spans come from `via_spans` (`F.Cu-In2.Cu`, `F.Cu-In1.Cu:micro`, `blind`,
`buried`, `micro`), else the designer's vias, else a KiCad 6 or 7 project's
switches. A via takes the shortest allowed span holding its two layers;
pour nets keep through vias; clearances are checked only against items on
the via's layers. With no span every map is as before. Off because no task
board allows a span and the search's CPU rises 6 to 31 %; where a board
allows spans the gain is real (fpga_large −85 KiCad items). No stacked or
staggered chains.

## The emitted copper

`router/output.rs` (`materialize_with_escapes`) turns each branch into runs
per layer, cut at bends, junctions and width changes, a layer change into a
via of the class size, and a pad end into its escape stub or a short stub
to the pad centre where that keeps clearance. `verify::copper_components`
rechecks each non-pour net, and `pcb_router::verify` measures true
distances between all emitted copper, obstacles and the outline,
independent of the lattice. `write_attempt` writes the copper with the vias
in pads, used fan-outs, pair copper and length patterns. KiCad's DRC is the
gate: its unconnected items count in every open figure.
`skip_native_verification` skips it (machines without kicad-cli).

## The router inside a layout

A router lives across placement changes (`router/update.rs`).
`Router::update(board)` takes a board with the same nets and classes, keeps
every net whose pads did not move and whose copper is still legal (with
occupancy and history), and `reroute(polish)` negotiates the rest.
`update_nets` and `local_reroute` keep a moved net's legal branches and join
them against the rest as walls (the move descent). See
[placer.md](placer.md) for the moves.

## The topological engine (off)

`crates/pcb-topo` routes TopoR's way: wires as sequences of crossings of
one triangulation shared by all layers, pulled taut into arcs.
`{"engine": "topological"}` in a route-mode config makes one attempt with it
(no ladder, pour nets as tracks, no pairs; knobs under `topological`, a
wall-clock budget). It matches the lattice router on small two-layer boards
but falls far behind on dense and four-layer ones (video 145 of 371 against
362), so it is not developed further. `{"tighten": true}` pulls the lattice
router's copper taut where the verifier accepts it (`pcb_topo::tighten`):
2 to 4 % shorter, nothing else changed.

## What the routes look like

`crates/pcb-kicad/src/aesthetics.rs` (`score-kicad-board`'s `aesthetics`)
counts bends, stair-steps and pin-row crossings;
`experiments/aesthetics/census.py` adds bundling, jogs, detour and spacing.
On complete designer-tier boards the router has fewer vias (×0.58), less
detour and fewer bends than the designers, but bundles less (×0.38) and has
more jogs (×1.53), staircases (×2.32) and pin-row crossings (×3.13), by the
medians. The step cost has no attraction to neighbours, history pushes
tracks apart, and a 45° sidestep costs about 0.3 mm, so congestion makes
jogs; the bend round, straightening, compaction and the clean-up's pin-row
charge take part of it out.

## What limits the router

The matched designer tier routes the designers' own placements of 109
boards at a layout's routing budget; it passes 76 strict on the nightly of
2026-11-10 ([handover.md](handover.md) has current numbers). What the rest
fail on:

- **The input's (about 10 boards).** The designer's own board leaves the
  same item: duplicated references on tile panels, swapped nets, a class
  wider than its pads, pads a footprint walls itself. They set the ceiling
  at about 99 of 109. Not ours.
- **Congestion stalls (the largest class: 13 boards decided, about 200
  items on 20).** The kept attempt's negotiation stalls with one or two
  nets cycling at the price cap. The trouble repair's exchange closes as
  much as it opens. The designer joins almost all of these on the same placement,
  and on most open nets the router's supply tracks cross the designer's
  path. **No live design.**
- **Fine-pitch rows (second class on 11 boards).** Pin lines fixed the
  phase of exact-fit rows. Crossing pairs (a row neighbour's target on the
  other side) and ways closed by neighbours' vias need a layer change the
  negotiation does not find.
  The corridor search keeps a margin of at least 0.5 µm, so an exact fit on
  the 0.05 mm rung has no way out. Handover open problem 4. **No live
  design.**
- **The budget (about 5 boards).** At twice the work 17 of 27 failing
  boards end unchanged to the item; the leader first took the cheap gains.
- **Probe misjudgment.** A probe's count can rank a rung wrongly (bms's
  plane stubs ranked first by 85 to 86 and it ended 0 to 7 open). Boards
  without probes get pin lines only after the leader's run. Handover open
  problem 3.
- **Odd pads.** Pads under a second footprint of the same reference (WLED);
  pads walled by a warning-rated clearance.
- **Two-layer pours** are cut up whatever the price: crossings need the
  back layer, and neither `plane_cut_cost` nor `pour_integrity` shortens
  what crosses it; `keep_whole` regions are the tool.
- **The spoke model's gaps:** spoke ends a few hundredths from a track,
  thermal vias on a layer with no pad of their number, `thru_hole_only`
  zones, per-zone fill settings, teardrops.
- **Differential pairs on dense boards:** links stay in conflict through
  the whole negotiation (jetson, left_io); open stubs at straight ends need
  a stub reserved in the negotiation; groups of several pairs, coupled
  branches and per-layer widths within a run are not done.
- **Lengths:** no room for bumps in a dense bus; a net over its maximum may
  stay over.
- **Parallelism:** batches need disjoint windows, so on large boards with
  long nets the negotiation is nearly single-threaded.
- **Geometry:** a 45° lattice, parallel diagonals quantised to pitch/√2, no
  arc output outside `tighten`.
- **Rules not modelled:** a clearance loosened only inside an area or
  courtyard (ChengminLi's YPS jumper pins stay unreachable), creepage,
  rules in time units, one edge and hole clearance per board, pair rules
  for pours, `.kicad_dru` via-type rules.

## Measuring a router change

The protocol is in [handover.md](handover.md) ("Working on it"),
[benchmarks.md](benchmarks.md) and the tasks
[README](../benchmarks/agent-tasks/README.md). In short:

- **Fingerprints** (Castor, k30-SBC, jetson, Sisu; route mode, a quarter
  budget): a change that should not move routes leaves them identical.
- **Expansion tier** (46 boards, route mode, 8 × 1): the router's
  regression tier.
- **Matched designer tier** (109 boards, the designers' placements at the
  layout's routing budget): what the router loses on placements known to
  be routable; judged by the strict rule.
- **Short tier** (12 layouts) when the change touches a layout's routing.
- Route mode on fixed placements (`--route-from`) for router questions,
  with the work rate pinned (`3.15e6,150e6`); CPU claims by pinned
  simultaneous pairs ([profiling.md](profiling.md)).
- A new default needs the tiers not worse and no strict pass lost.

## Tried and rejected

One line each, so they are not tried again without a new reason. Removed
code is in git history.

- Jacobi-style parallel routing of overlapping nets: converges far worse.
  Speculative routing of later batches: too few routes survive, since a
  congested area's nets reroute one after another.
- Capacitated corridors (a global route with hard capacities, learning from
  misses, guiding the search): tier 616 against 588 unconnected; misses are
  too rare to learn from.
- Island stubs (a track into an island so its spokes count): only silenced
  KiCad's island finding. Hard thermal guards and stitching vias during
  the negotiation: worse.
- Thermal guards in the pad's own frame: Hub −6 connections.
  `via_reduction_keeps_spokes`: k30-SBC lost its via round, +5 starved.
- `fixed_plane_stubs` or `plane_skeleton` as defaults: take room;
  conditional rungs only. `pour_integrity` on the fence: no whole pour.
- Renegotiating the residue in a window around the open nets: stalls at
  the price cap like the board; 20 to 20 strict on 54 boards.
- The last connections: a last pass with ejection chains three deep, pour
  nets and board-wide victims closed nothing on the 12 near misses. 24
  victims instead of 12: mixed. Victims on their corridors only: lost MegaDrive.
- Fan-out before negotiation (one way out per dense pad, reserved by a
  cost): moved which pad of a crossing pair stays open; −3 passes.
- Crossings decided in the repair (one net of a crossing pair dives beyond
  the neighbours' tips): on Quanta75 every via landed on another layer's
  routes.
- Prepared fan-outs for fine-pitch rows (fixed exits with dive vias):
  closed-row items 113 to 61, strict 10 to 5 on 16 boards.
- Pin lines in the channels too (RP2040_base 5 to 12), or only for parts
  with a missed exit (−3 passes).
- A lattice chooser counting pocket corridors: one axis cannot open a 2-D
  pocket. Contact starts for plain pads: 75 to 75 strict.
- Cutting every negotiation between batches (not only probes): ZSWatch 3
  to 9.
- A global `bend_cost` of 0.3: the round's gain at +18 to 25 % CPU, −3 passes.
  Three bend rounds: 2.6× the work for 3 % fewer jogs.
- A hug discount in the search: length +1.6 %, more staircases, +4
  unconnected. Compaction inside the straightening: the pour check undid
  whole nets. A stair-step search after straightening: no gain, +5 to 8 %
  CPU.
- Pairs: pre-routed coupled runs (took room); links first in the routing
  order (−19 coupled, −17 connections); crossovers where the router routes
  the stubs (walled pads); split ends between far P and N pads (never
  coupled, cost full links); launches turned for their stubs or preferring
  clear approaches (lost pairs and connections); a probe after the race
  switching the plan off (a probe does not predict the first route).
- A hard layer mask for skew groups (cost connections); `length_rule_layers:
  rule` (no gain); a congestion share for length-bounded nets (more open).
- The lattice A* on a GPU (wgpu prototype): projected gain under the 20 %
  bar.
- Freerouting as an in-tree backend: removed; `benchmarks/github/freerouting.py`
  runs the jar headless, and the head-to-heads stand.
- A learned congestion predictor (a CNN on rasterised placements, trained
  on 75 s probes) as the race's pre-ranker: no better than RUDY among seeds
  of one board, because the probe it learns from is itself noisy (two
  probes agree at about 0.44).
