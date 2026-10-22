# The placer (`pcb-placer`) and the coupled layout loop

`pcb-placer` replaces the spring-plus-pairwise-pushing placer described in the
[trust audit](reviews/2026-09-21-trust-audit.md). Together with
[`pcb-router`](router.md) it forms the automatic layout flow:

```sh
# Placement only (writes the project plus placement.html, an animated playback)
cargo run --release -- place-kicad-board <source-dir> <board-id> <out-dir> [placer.json]

# Coupled placement and routing
cargo run --release -- layout-kicad-board <source-dir> <board-id> <out-dir> <router.json> [layout.json]
```

Every key of `placer.json` and `layout.json` with its type, default and
doc comment is in the generated [configuration reference](config-reference.md).

## Results (2026-09-21, cold: all movable footprints re-placed, zero copper)

Every board below is placed and routed automatically, is electrically complete
in native KiCad (zero unconnected items, zero clearance/courtyard findings) and
has zero internal violations. The only remaining native findings are
silkscreen label cosmetics (2–19 per board), which the existing
`repair-kicad-silkscreen` tool reduces further.

| Board | Movable | Auto placement → routed | Human placement → routed |
| --- | ---: | --- | --- |
| ECC83 | 6 / 15 | 9/9, 0 vias, 368 mm | 9/9, 1 via, 344 mm |
| Complex hierarchy | 66 / 68 | 50/50, 0 vias, 1318 mm | 50/50, 0 vias, 1330 mm |
| PIC programmer | 55 / 63 | 34/34, 1 via, 1722 mm | 34/34, 1 via, 1907 mm |
| Interf-U | 22 / 25 | 110/110, 71 vias, 5103 mm | 110/110, 58 vias, 4789 mm |

Placement takes 0.6–3 s, routing 0.2–25 s. Both routed columns use the same
router and rules. On Interf-U the human placement is still better (fewer vias,
less copper); on PIC and the hierarchy board the automatic one is.

| Global placement (hierarchy board) | Placed and routed (Interf-U) |
| --- | --- |
| ![global placement](images/hierarchy-global-placement.png) | ![auto layout](images/interf-auto-layout.png) |

## How it works

1. **Lowering.** Footprints become bodies (courtyard plus pad bounding box,
   rectangular or round) with pins at their true offsets. Locked footprints,
   footprints without nets, and footprints on the board edge (connectors) stay
   fixed; everything else may move and turn in 90° steps, from a
   canonical start (`orientation_search`, below). Large nets get a
   smaller weight so power does not collapse the layout. Pad boxes cover
   the drill. A through-hole part blocks the other side only with its
   holes and pads, and a part with a courtyard on both sides with that
   one too, against parts on either side. A hollow part (a shield outline) blocks only with its
   pads, and with its courtyard against parts with holes, since KiCad
   forbids holes in a courtyard unless the courtyard is malformed and
   KiCad skips it. A board cutout keeps only copper away: a connector body
   may reach over its peg holes. A footprint's own mask graphics (an RF
   modulator's exposed 22 x 32 mm area) are part of its body, also where
   it blocks with its courtyard's shape (`mask_openings`, below).
2. **Routing halos.** A PCB routes in the same plane it places in. Every body
   gets a halo proportional to its pin count; halos count as body area and as
   spacing. If the board cannot afford them they shrink uniformly.
3. **Global placement** (ePlace/RePlAce formulation). Body area is electric
   charge; the potential solves Poisson's equation on the board with a
   spectral method, so the spreading force acts at a distance and points to
   free space. Fixed parts and everything outside the outline are fixed
   charge; filler charge decides how much whitespace stays between the parts.
   Wirelength is the weighted-average model on true pin positions. Nesterov's
   method with a Barzilai–Borwein step minimizes
   `wirelength + lambda * density` while `lambda` grows until overlap is gone.
   Rotations are chosen greedily every few iterations.
4. **Annealing legalization.** Moves, quarter turns and swaps are judged on
   wirelength plus an overlap penalty that grows until no overlap survives.
   It starts cold so the global result is refined, not scrambled. An exact
   nearest-legal-position pass is the safety net. When it fails, it runs
   again with the stuck parts first. At the edge and courtyard levels
   (no halo, spacing or grid left to give), a level that keeps the
   placement of the level before, and whose few stuck parts find no room
   among the kept ones, legalizes its anneal again with them first, the
   parts that pass leaves without room joining them, up to four passes (a
   part placed first can take another's only spot: MIDAS-MK1, changelog
   "MIDAS-MK1 refusal"). Crowded boards then walk down relaxation levels:
   - smaller halos, then none;
   - spacing down to the copper clearance;
   - a 0.1 mm grid;
   - bodies closer to the edge, as far as their copper allows;
   - where the project lets courtyards overlap, bodies shrunk to their
     fabrication outline and pads.

   The level used is reported, and the router-driven moves keep to it.
   Several seeds run in parallel (`placement_seeds`, 3; the layout asks for
   16, see the race below); the best placement by the placer's own key
   (fewest illegal parts, least missed constraints, least wirelength; a
   tie to the earlier seed) is kept, the others are the race's
   alternatives. The seeds share the work budget: seed k is kept while the
   seeds before it used less (three at least). They run in a pool of the
   process's search threads, judged in seed order whatever finishes first,
   in batches sized from the kept seeds' work (`race_seeds`), so the seeds
   run and kept do not depend on the threads; `board-placer.json` reports
   `seeds` (kept), `seeds_run`, `seed_kept`, `seeds_work_seconds` and
   `pool_work_seconds` (what the layout's clock counts: the kept seeds on
   the reference machine's eight threads).
5. **Refinement.** Greedy legal moves, rotations and swaps that strictly
   reduce wirelength.
6. **The placement race.** Wirelength is not routability. The layout ranks
   the seeds' legal placements (those that keep the constraints as well as
   the kept one) by RUDY: each net's spanning-tree length times its track
   pitch spread over its bounding box, summed per 16 x 16-node router tile,
   above a quarter of the tile's free capacity (every layer less obstacles
   and pad copper; `pcb_congestion::rudy_overflow`). The best three by that
   score are probed by the router for 75 s of work each, side by side, and
   the one with the fewest unfinished nets routes on (ties to the better
   ranked). Ranking takes 10-100 ms a placement (lowering, rasterizing);
   the 16 seeds cost placement wall time (about 30 s a board on the quick
   tier) but no extra placement work. Quick tier, 2026-10-10, one binary
   (x402), the plain race of three seeds against the pre-ranked race of 16:
   best race probe 204 -> 189 unfinished nets, open after the first route
   149 -> 112, KiCad unconnected 98 -> 64 (6 boards better, 2 worse),
   passes 16 -> 18. `congestion_score: "none"` in layout.json asks for the
   plain race; `congestion_model` ranks with the learned predictor
   instead (an experiment, [congestion-model.md](congestion-model.md)).
7. **Coupling.** `layout-kicad-board` routes the placement once, then the
   router and placer take turns on one in-memory state: congested footprints
   are nudged, the router reroutes only what the nudge touched, and moves are
   kept when the board improves. See [coupling.md](coupling.md).

The written board is checked: every pad must land exactly where the placer's
model said it would, including rotated footprints (KiCad stores pad and text
angles as absolute values).

## A fast no, and the budget inside the passes (2026-10-15)

A board without room used to cost every relaxation level an anneal and a
legalisation in vain, each part without a spot searching the whole board
(urchin's 80 parts on 80 x 40 mm with its key grids as rows: 742 s of
wall on four threads; the work budget was checked only between levels,
so a seed overran its 600 s by a quarter). Now:

- **Before placement** (`placement_room::outline_holds`, after the
  constraints are applied; input errors, exit 2): a user's `row` that fits
  the outline's box in no orientation it may take, and the parts an edge
  constraint holds at one edge on one side that need more length than the
  edge has. Only parts too deep to stand two behind each other within
  their `max_mm` count, each by its shortest tight body; the two that could
  overhang the edge's ends at the edge-copper level count their copper
  only. Regions were checked this way already (`region_holds`); all such
  errors are now said together. Not for an automatic outline (the layout
  grows it). Urchin's case: exit 2 in 0.04 s.
- **At once, before the levels** (`place`, `hopeless`): when even the
  last (loosest) level's bodies cannot all be placed, by area (the movable
  tight bodies with the least spacing need more than a side's outline
  area) or by strips (`strips_hold`: bodies at least `t` across meet any
  line at most `floor(height / t)` times, so their least extents along it
  sum to at most that times the width), no level runs: the parts are
  legalized once from the global placement with the hurried search below,
  the rest are unplaced, and `Placement::overfull` (`"overfull": true` in
  board-placer.json) says why. Both tests are exact (a necessary
  condition for any packing): a board that can be placed never takes
  this way, so its placement is unchanged. Hollow parts (those without a
  courtyard block with their pads alone), parts held at an edge or
  reaching over it, and cutouts are left out of the strips. Urchin
  without its rows on 60 x 40 mm: exit 1 in 3 s.
- **Inside the passes** (`out_of_work`, a thread-local stop the running
  `place` sets from `work_limit`): past the budget, a part without a spot
  in `legalize` searches only as many rings as 15 mm of a 0.5 mm grid (3
  mm of the fine grid) instead of the board; `evict_for`, `refine`,
  `repair_relations` and `align_orientations` stop; the retry with the
  stuck parts first is skipped. A placement within its budget is
  unchanged. Urchin without rows on 80 x 40 mm is not hopeless by either
  test (its switches have no courtyard: their pads alone block, which
  could interleave), so it still takes the levels up to the budget.

The placement's `room` (`placement_room::room`, in board-placer.json and
the verdict, see [agents.md](agents.md)) puts the room in numbers: per
side the usable area (outline less the edge band and the fixed bodies),
the movable bodies with spacing, the utilization, the area missing at a
target of 0.7, the smallest outline at that target (area, strips with the
target's slack, the largest part, rows and edge sets) and, when parts
found no place, a suggested outline.

## Where the wirelength goes (measured 2026-10-14)

`PCB_PLACER_TRACE=<file>` writes, per seed, the weighted wirelength after
global placement, annealing, legalization and refinement (and at the
designer's poses with `PCB_PLACER_DESIGNER`), the relaxation level, and
per movable part how far each stage moved it and what it met where the
annealer left it. `experiments/placement-loss/` prepares tasks as the
runner does, places them (`measure.py`, 16 seeds and the layout's 600 s of
work via `PCB_PLACER_WORK_SECONDS`) and tabulates (`analyse.py`,
`compare.py`). On the 30 quick-tier boards plus hackrf, video, the three
OpenESC boards, both SNSPs and zpn (the kept seed, x652's placer):

| Stage ratio | Median | Range |
| --- | ---: | --- |
| annealed / global | 0.83 | 0.36-1.54 |
| legal / annealed | 1.04 | 1.00-1.45 |
| final / legal (refinement) | 0.97 | 0.74-1.01 |
| final / annealed | 1.00 | 0.88-1.29 |
| final / designer | 0.85 | 0.16-1.77 |

- The legalizer is not where the wire goes in sum: the global placement is
  30-90 % longer than the final one, and the annealer, not the
  legalizer, removes that. Legalization adds 8 % over all boards and
  refinement wins 5 % back; a few crowded boards lose more (Castor 25 %,
  MIDAS-MK2 28 % before refinement, 21 and 12 % after), where the
  nearest-spot legalizer moves 100-120 parts, some 20-30 mm.
- Four ways to legalize or place better found nothing within the seeds'
  noise (median final wirelength on the six gap boards): keeping the
  annealer's legal parts and reflowing only the conflicted ones (-0.5 %
  on the tier), pushing overlapping parts apart a grid step at a time
  before that (+2 %), filler whitespace in the global placement (+2 %,
  +1 %), four times the annealer's moves (+4 %). The annealer is
  converged under the current rules.
- What the gap to the designers is: the routing halos. Without halos the
  six gap boards' final wirelength drops 22 % (median 23 %), to 0.87-1.27x
  the designers'; a spacing of 0.2 instead of 0.5 mm drops it 4 %. The
  designers' own placements are illegal at our full level for nearly
  every part (Castor 156 of 163, MIDAS-MK2 192 of 197): designers put
  parts courtyard to courtyard.
- The annealer does not see all the legality test forbids across the
  board: a through-hole part's holes meet bodies at half the spacing, and
  its copper-only far-side boxes not at all (Castor: 33 of 69 parts the
  legalizer had to move were invisible to the annealer).
  `anneal_far_side_gaps` (placer option, off) gives the annealer the
  legality test's gaps; it halves the parts legalization moves on Castor
  but wins no wirelength, and on the layout it is flat on the quick tier
  (x700 against x651: 19 passes both, unconnected 29 to 30, starved
  thermals 12 to 17, vias -4 %) and worse on two crowded boards (video
  240 to 315 unconnected, OpenESC 4in1 120 to 132; SNSP-CPU-01 239 to
  224 and mask bridges 22 to 5).
- `shared_halos` (placer option, off): neighbours keep the larger of their
  two halos between them instead of the sum; the halo budget and the
  global placement's charge count one halo per gap. Six gap boards' median
  final wirelength -6 %. Quick tier (with the annealer's gaps, x700):
  passes 19 to 20, unconnected 30 to 24, starved thermals 17 to 9, vias
  -5 %; but on the crowded boards zpn_devboard loses its pass (one mask
  bridge, vias 299 to 348) and hackrf's vias go 429 to 673. Not a default.

## Routing room from congestion (measured 2026-10-14, off)

`routing_room` (placer option, off): after a seed is placed with the
pin-count halos, each movable part's halo becomes its pin-count halo
times `factor * demand / median demand` (RUDY around the part's body
grown by 1 mm, nets a pour carries left out; none below half the median,
at most twice the pin-count halo), with `factor` chosen so the halos take
`routing_room_share` (1.0) of the area the level's halos took; the parts
anneal again from the legal placement (small moves), legalize and refine.
Parts the new halos leave without room fall back to the level's halo,
then to none; a seed whose pass still fails keeps the pin-count halos
and ranks after the others. `routing_room_all_parts` (on) lets parts the
level left without halo get one. The moves after routing keep the
per-part halos (`Relaxation::halos`). Code `crates/pcb-placer/src/room.rs`,
scripts `experiments/routing-room/`.

What the routed boards say (x693 tier and x652 gap runs, 35 boards;
`halo_use.py`, `analyse.py`): the routing density in a part's 1 mm band
does not follow its pin count (Spearman about 0 per board, -0.3 to 0.4)
but follows the RUDY demand there (median 0.6, 0.1-0.8) and the 25 s
probe's history (similar). On boards with full halos 15-76 % of the halo
area carries under 5 % of its routing capacity; keeping the half of the
halo area with the highest RUDY keeps 58-79 % of the routing inside
halos on the crowded boards, the half by pin count about 50 %.

What the layouts say (seed-repeated pairs, seeds 1, 101, 201, logs
`build/room-pair-*`): the option never fires on video, OpenESC 4in1 and
hackrf (their kept level has no halos; hackrf drops to it because one
part finds no room at the first level). On the five boards where it
fires, passes over the 15 runs: off 11, share 1.0 within the level 9,
share 1.0 all parts 11, share 0.75 9, share 0.5 9. Share 1.0 moves HPWL
by -15 % (BAGEL), -6 % (MK2.1) and +5 to +9 % (MK2, Castor, zpn); share
0.5 shortens HPWL on all five (median -8 %) and routed length on four,
with unconnected medians still 0 but single errors (a mask bridge, one
open connection, a starved thermal) costing two passes. Quick tier
(x737 against x722, one seed): 22 to 24 passes, unconnected 22 both,
vias -3.7 %. Within the seed noise; not a default. Video's routing
differs by up to 130 unconnected between runs of the same placement
(x732 and x736 with identical placements: 392, 314, 325, 262 for seed 1).

## Placement work against the CPU (calibrated 2026-10-14)

Budgets count placement work (`pcb_placer::work`, 150 M units a second
of one thread on the reference, `REFERENCE_WORK_PER_SECOND`; since
2026-10-21 scaled to the machine at the start of a run unless pinned,
`work_per_second`: docs/router.md, "The machine's rates"); the layout's
wall clock only guards. `PCB_PLACER_DEBUG`
prints per phase `placer phase <name>: <wall> s, cpu <thread cpu> s, work
<counted> s (<counted / cpu>)`; the CPU time comes from
`/proc/thread-self/schedstat`, so the ratio is the calibration whatever
the load (absolute values still drift with it: keyboar_'s anneal measured
0.89 to 1.45 at loads 25 to 50; compare boards run side by side).

Before, the legaliser booked 3-15 % of its CPU time on the 0xCB panels
(and the wall guard fired, so results followed the machine's load:
vaio_re 63 or 151 open, glasgow "deadline reached"), the annealer 25-50 %
on boards with many relations (glasgow, MIDAS, Castor) against 100 % on
keyboar_. Profiled with callgrind (instructions per counted unit, about
42 on the calibrated anneal), what was not counted:

- a cutout's outline (the panels' slots between the copies, thousands of
  points) was walked nine times per pad, counted once per part: now one
  pass with a box prefilter decides unless an edge comes near the pad
  (exact: the full test runs then), counted per edge visited (a quarter
  unit); the 0xCB pcb-panel's legalisation went from 224 s to 28 s of CPU
  a pass;
- box lists (hollow parts, far sides, pads) and their pair tests: counted
  as the annealer does (8 + 3 per box, 1 per pair);
- the other part's box (two turns) and the linked-pair lookup per
  same-side test in the legality test (4 units, and log2(linked)/2);
- relations: each anchor a relation measures from costs about 4 units
  (glasgow's decoupling capacitors are each near any of 23 supply pins),
  and finding a part's relations scans every anchor; counted in the
  annealer's move work and the legaliser's penalty;
- pin positions in wirelength sums: 3 units a pin;
- parts held at an edge: their pads' offsets and the board's box on every
  test (eviction of a held connector, five times what it counted); the
  edge and region lists, scanned per test (half a unit an entry);
- exact speed-ups: a part not held at an edge no longer computes its pads'
  edge offsets in every snap, overlap and hard-constraint test.

Work ratio by phase (work / thread CPU; each pair placed at once on the
two threads of a core, one seed, 720 s of work; before = x738's counting):

| Board | Before (all; anneal, legalise, evict, refine, repair) | After |
| --- | --- | --- |
| 0xCB pcb-panel | 0.11; 0.52, 0.03, -, -, - | 0.95; 0.87, 0.93, 0.90, 1.50, 2.15 |
| 0xCB panel | 0.11; 0.45, 0.07, 0.07, 0.22, 0.08 | 0.77; 0.66, 0.77, 0.63, 0.65, 0.75 |
| glasgow | 0.40; 0.31, 0.40, 0.23, 0.48, 0.14 | 1.08; 0.98, 1.01, 1.05, 1.16, 1.12 |
| vaio_re | 0.72; 0.72, 0.49, -, 0.78, 0.61 | 0.98; 0.93, 0.80, -, 1.24, 1.08 |
| MIDAS-MK2 | 0.52; 0.49, 0.50, -, 0.62, 0.09 | 1.07; 0.98, 1.45, -, 1.63, 1.29 |
| Castor | 0.68; 0.59, -, -, 1.52, 0.23 | 0.98; 0.88, 1.14, -, 1.57, 0.84 |
| urchin | 0.86; 1.11, 0.23, -, -, - | 1.08; 1.09, 1.02, -, 1.00, - |
| keyboar_ (reference) | 1.02; 1.02, 0.61, -, 0.55, - | 1.02; 1.01, 1.17, -, 1.04, - |

(the pcb-panel's base placement was stopped after its first two
legalisations, 209 s of CPU; keyboar_ and the 0xCB panel from separate
pairs.) Global placement counts about twice its CPU time (1.5-2.3),
unchanged: it is 1-3 % of the work. A single seed's placement is
byte-identical before and after on glasgow, vaio_re, MIDAS-MK2, Castor,
urchin, spell_tome and keyboar_: the work changes where budgets stop
(the race's seed count, a level's or the annealer's work limit), not
what a seed does.

## Panels and courtyards on the far side (2026-10-14)

- **Copies of a tabbed panel** (`keep_copies`, on): the copies are one
  outline piece, so `keep_on_pieces` cannot tell them apart; their nets
  can (KiKit names a copy's nets `Board_N-...`). Where two or more net
  groups hold the same references (with pin counts), each with fixed
  parts whose boxes do not overlap, every movable part of a copy gets a
  region: the board's box cut between its copy's fixed parts and each
  other copy's, on the axis they are farther apart. Boards whose
  nets form one group (all others, and panels whose copies share net
  names: the PolyKybd molecules, OpenESC's 4in1 panel) do not change.
  Since 2026-10-15 the cut is halfway moved by half the difference of the
  board's margins beyond all copies' fixed parts (kept between the two
  copies): the copies repeat one layout, so each copy's region lies around
  its fixed parts as the board lies around all of them. Halfway alone cut
  SmartSpin2k's copies (fixed parts at each copy's top, 2.3 mm from the
  board's top edge and 40.5 mm from its bottom) through their own parts:
  162 of the designer's 630 movable parts outside their copy's region,
  none now; the 0xCB panels' cuts move by at most 0.75 mm. Each copy's
  parts also start global placement at their region's centre
  (`Constraints::starts`) instead of the common start, which clamped the
  other copies' parts against their regions' near borders.
- **A repeated reference** in a constraint (panels repeat every
  reference): an edge, rotation or place entry with a `uuid` is about the
  footprint with that uuid (`describe-kicad-board` lists them; the task
  generator writes one for every entry of a repeated reference). Without
  one, the entries name the parts in turn: the n-th edge, rotation or
  place entry for J3 is about the n-th J3 in board order. All of them
  used to land on the first J3 (one connector asked to touch the left,
  top and bottom edges at once); in turn, a task with entries only for
  the copies at an edge bound them to other copies (the molecules' fixed
  J1, J3 and J4 missed 17 edges by whole tile pitches).
- **Courtyards on the far side of parts on one side**
  (`Component::far_courtyard`): a back-mounted hot-swap switch has its
  14 mm square on the front; KiCad checks it against the front
  courtyards and holes of every part, the switches on the back included.
  The legality test and the annealer now keep one part's far courtyard
  from the other's far side (holes, pads, courtyard) at the bodies' gap;
  neighbour searches reach `far_reach` farther where such parts exist.
  woagboard's 26-28 courtyard overlaps go to 0.

## Mask openings block placement (2026-10-15)

A footprint's own mask graphics (filled F.Mask or B.Mask polygons,
rectangles, circles, strokes) bare the copper under them: another
footprint's pad there is exposed next to the opening, and KiCad reports
solder_mask_bridge for every such pad of another net. The body's box has
held them since the bodies agent (`body_on_side`), but a part whose
courtyard covers less than three quarters of that box blocks with its
courtyard's shape and its pads (`hollow`), and the openings were lost
there. SNSP-CPU-01's J2 (an RF modulator: a 12 x 5 mm courtyard around
four pins, a 22 x 32 mm F.Mask polygon beside it) held T1 and C61 in the
layout, SNSP-CPU-1CHIP's RF1 held T1; J2 is fixed, but a movable part
lost them alike. Now that shape includes the part's mask graphics on its
side (`mask_openings`, on), held to the body's box, so the legality test
and the annealer (both test the shape) keep other parts' bodies and pads
the bodies' spacing away, fixed and movable parts alike. Where KiCad's
mask margin, max(minimum web + `pad_to_mask_clearance`,
`solder_mask_to_copper_clearance`), exceeds the copper clearance the
openings grow by the difference (eternal-keypad and protoesp 0.10 mm,
USB_Cable_Tester 0.08, WLED_Matrix and USB_Keypad 0.05; none of them has
such a part). Pads of the opening's own net are kept out too (KiCad
exempts them; a whole pad of one net is rare). Courtyard-less parts
already blocked with their mask graphics, whole-body parts with their
box, and mask graphics on a part's far side are far-side copper.

Only three boards of the 109 have a part with a courtyard and mask
graphics past it (SNSP-CPU-01 4, SNSP-CPU-1CHIP 2, USB_Cable_Tester 1).
`experiments/mask-openings/pads_under_openings.py` counts other
footprints' pads under a footprint's mask graphics: 0 on all 109
designers' boards; on the 109 v16 layouts only SNSP-CPU-01 (6 pads of T1
and C61 under J2) and SNSP-CPU-1CHIP (4 pads of T1 under RF1). Layouts
(tasks v6, four threads, one run a side), x1055 against x1110:

| board | pads under openings | solder_mask_bridge | unconnected | errors | vias |
| --- | --- | --- | --- | --- | --- |
| SNSP-CPU-01 | 6 / 0 | 15 / 0 | 102 / 147 | 16 / 2 | 1812 / 1615 |
| SNSP-CPU-1CHIP | 4 / 0 | 16 / 0 | 2 / 0 (passes) | 16 / 0 | 757 / 678 |

SNSP-CPU-01's front is 73 % full (76 % with J2's opening). Another
seed won the race (seed 3 instead of 12; its race probe left 308
unfinished nets against 317), its first route left 215 open terminals
against 114, and it ended with 45 more unconnected items. One run a side
cannot tell a reshuffle of a crowded board from a cost of the rule; the
v16 layout (x862, the parts under J2) had 152.

## Orientation search from a canonical start (2026-10-15)

The placer turns parts at every stage: global placement picks each part's
angle by the HPWL of its nets (nets up to 24 pins) every 15 iterations
once the overflow is under 0.75 (`choose_rotations`), the annealer
proposes quarter turns (15 % of its moves, judged on wire, constraints
and overlap like any move), the legaliser tries every angle at each spot,
and refinement every angle at each target. What made the result depend on
the angles the board held: every part started at its angle, and its
options were listed from that angle on (`at + 90 k`), so the annealer's
random turn drew another absolute angle and ties broke the other way. An
import from a schematic holds every part at 0 degrees, the layout tier's
stack the designer's angles; the two placed differently.

Now (`orientation_search`, on; `core::orient`): every part's options are
sorted (0, 90, 180, 270; a part at an odd angle keeps its extra ones, so
it alone still differs), and every movable part that may turn starts at
the first, turned about its body centre. Fixed parts, parts with one
option (`rotation`, `opening_outwards`, an overhang) and a row's leader
(the row's body is built in the leader's frame at its angle) keep their
poses. The placement no longer depends on the angles it starts from:
coco2, urchin and homebrew_486 imported from their schematics and placed
from 0 degrees and from the designer's angles (the same file but for the
angles, `experiments/orientation-search/set_angles.py`) give the same
poses. Since an import holds 0 degrees, the schematic flow places as
before; the layout tier now places as if from an import.

What the measurements say (`experiments/orientation-search/`):

- **The layout tier does not keep the designer's angles.** From the
  designer's angles x1112 left 25-40 % of the moved parts at them, from 0
  degrees 22-35 % (four seeds, ten short-tier boards; a quarter by
  chance). coco2's 549 against 1144 vias (docs/changelog.md, schematic tier
  failures) was the placement's chaos, not inheritance: on x1112 the same
  board from 0 degrees gave 663 vias, from the designer's angles 1060.
- **HPWL over seeds** (`seeds.py`: `place-kicad-board`, one seed a run,
  seeds 1-4; median weighted HPWL in mm), x1112 from the designer's angles
  against the canonical start: OpenFC 897 / 1043, spell_tome 191 / 155,
  homebrew_486 14831 / 13408, WLED_Matrix 1289 / 1345, 0xCB-1337 1047 /
  1067, urchin 1732 / 1760, coco2 14393 / 14527, USB_Keypad 885 / 886,
  USB_Cable_Tester 1296 / 1304, eternal-keypad 550 / 543, reCamera 201 /
  199, Blue_Line 108 / 110. Geometric mean about 1 %; the seeds of one
  board spread 10-20 %. OpenFC is the one board where the designer's
  angles carry more than chance: its global placement is already 20 %
  shorter from them (786-897 against 950-1066).
- **The order of the footprints decides as much**: pcbnew's save sorts
  them, and homebrew_486 with the same angles in another order placed
  103 of 130 parts elsewhere (both orders are deterministic).
- **Stronger searches, measured and not kept** (eight boards, four
  seeds, against the canonical start): the annealer's turn proposing the
  cheapest other angle (wire, constraints and overlap) instead of a random
  one: coco2 -4 %, homebrew_486 +3 %, OpenFC +4 %, the rest within 2 %,
  15 % more placement work; the global rotation choice every 5 iterations
  instead of 15: geometric mean -0.5 % (OpenFC -7 %, WLED_Matrix +3 %);
  a random start angle per part instead of the first: +1 % (spell_tome
  +27 %, OpenFC -6 to -8 %); the rotation choice from the first iteration:
  no change.

Layouts (a third of the budget, four threads, one run a side; before
x1112, after x1130), each board imported from its schematic and laid out
from 0 degrees and from the designer's angles (vias, length mm, KiCad
unconnected, weighted HPWL after placement):

| Board | x1112 from 0 | x1112 from the designer's | x1130 from 0 | x1130 from the designer's | Placements |
| --- | --- | --- | --- | --- | --- |
| coco2 | 663, 23,233, 0, 12,414 | 1060, 26,155, 0, 13,556 | 663, 23,233, 0, 12,414 | the same | x1112: 148 of 156 elsewhere; x1130: identical, and identical to x1112 from 0 |
| urchin | 41, 2724, 0, 1669 | 40, 2544, 0, 1692 | 41, 2724, 0, 1669 | the same | x1112: 22 of 122 elsewhere; x1130: identical |
| homebrew_486 | 981, 26,013, 0, 11,959 | 1196, 29,169, 0, 13,494 | 981, 26,013, 0, 11,959 | 1040, 25,260, 1, 11,959 | x1112: 104 of 130 elsewhere; x1130: the placer's poses identical |

homebrew_486 from the designer's angles: the placer's 11 seeds are
identical, but the race's router probes differ (seed 2 left 213
unfinished nets from 0 degrees, 183 from the designer's), so the race
keeps another seed: the two input files differ in more than the angles
(pcbnew's save turns a footprint's graphics into another order and writes
-180 for 180), and the router follows the order of the graphics.

Short tier (`--jobs 6 --threads 4`, under the lock) x1130 against
`build/short-x976.log` (x1111's rows are x976's; HPWL from x1111's
placements): every
placement changed (the stack's designer angles no longer count); passes
6 to 6, unconnected 35 to 32, vias 1129 to 1130, length 22,253 to 20,777
mm, work 3236 to 2840 s, cut 0. Per row (vias, HPWL, KiCad unconnected):
USB_Cable_Tester 131 to 90 (HPWL 1270 to 1279, passes), OpenFC 178 to
154 (756 to 937, 2 both), framework_mobo 108 to 91 (921 to 850, passes),
MIDAS-MK1.1 207 to 198 (2523 to 2137, 20 to 13), Blue_Line 26 to 25 (105
to 101), urchin 40 both (1686 to 1675), spell_tome 8 both (149 to 143),
eternal-keypad 56 both (541 to 525, its one error both), 0xCB-1337 76 to
78 (994 to 992), reCamera 56 to 68 (199 to 190, 5 to 9), USB_Keypad 78 to
106 (822 to 804, 2 both), WLED_Matrix 165 to 216 (1079 to 1193, 6 both).
A reshuffle in the noise of a placement change (OpenFC's HPWL +24 %
with 24 fewer vias), not a cost.

Schematic tier (the same) x1130 against `build/sch-x1100.log`: 8/12 both,
unconnected 8, new errors 0, vias 3577 to 3578; ten rows identical
(placements and results); OpenFC (166 to 165 vias) and 0xcb-static (2 to
4) differ in one part each, after the layout's moves: the placements are
x1112's, the moves after routing walk the options in the canonical order
now (C3 at 90 rather than 270, D4 one key pitch over).

## Order independence (2026-10-16)

The placement depended on the order of the footprints in the file and on
the order of each footprint's own items, as chaotically as it did on the
start angles: part indices decide the annealer's move draws and every
tie in the legaliser and refinement, nets are numbered by the first pad
seen, bus rows and channels are found in footprint order, and the
router's probes in the race took obstacles in the graphics' order.
pcbnew's save, the netlist import and the schematic flow each write
another order, so two saves of one design placed and routed apart
(homebrew_486, same angles, pcbnew's order: 103 of 130 parts elsewhere).

Now every command that places or routes sorts the board as it reads it
(`canonical_order::parse_board` in pcb-kicad: `place-kicad-board`, the
layout's outline sizing, placement and reads of the placed board, the
router's every attempt, the congestion score): footprints by reference
in natural order (R2 before R10), then uuid, then the file's order
(until 2026-10-21 their text, which holds the pose: two footprints with
one reference and no uuid could come back from the placed board in the
other order, and the layout, which writes poses by index, swap them;
only such ties depend on the file's order now); inside a
footprint the pads by number (natural), then position, the `fp_*`
graphics by layer and content without their uuid, the footprint's zones
by content; each kind keeps the places it held among the items, the rest
stays. With the input in one order, everything downstream (global
placement, the annealer's moves and seeds, legaliser, refinement, rows,
the pool's seeds, the race's ranking and probes) iterates one order; no
change inside `pcb-placer` was needed. The boards written (placed,
routed) are in that order too, so the result file is byte-identical
from any input order; KiCad does not care about the order (its own save
sorts footprints by uuid), but for a footprint's courtyard graphics,
which it chains in file order: those are written in KiCad's own save
order (`canonical_order::source_order`, since 2026-10-21; before, the
writer restored the input file's order and broke this promise).

A second difference between two saves was the angle's spelling: KiCad
stores pad angles absolute and wrapped, so one pad's angle relative to
its footprint is -90 in one file and 270 in another, and a footprint is
at -180 or 180. `to_radians().sin_cos()` rounds the two differently
(1e-16 in a pad box, the sign of sin(pi)); the placer's lowering and the
router's pad, rotation and rectangle code now take the sine and cosine
exact for quarter turns (`pcb_router::geometry::degrees_sin_cos`).

Measured (`experiments/order-independence/`: `permute.py` writes a
project with its board's footprints, and each footprint's pads and
graphics, as is, reversed or shuffled with seed 7; `compare.py` compares
the outputs; `run.sh` runs them, four threads). coco2, urchin and
homebrew_486 from the schematic tier's prepared sources
(`build/sch-x1130/*/project`), placement alone (`place-kicad-board`, 200 s
of work, three seeds) and layouts at a third of the budget; parts at
another pose against the file as is, then vias, length mm, KiCad
unconnected for the layouts:

| Board | x1132 placement (reversed, shuffled) | x1132 layouts (as is / reversed / shuffled) | x1161 placement | x1161 layouts, all three orders |
| --- | --- | --- | --- | --- |
| urchin | 20, 22 of 122 | 41, 2683, 0 / 41, 2692, 0 / 40, 2603, 0 | identical | 43, 2636, 0, byte-identical |
| coco2 | 148, 148 of 156 | 1144, 26,093, 0 / 593, 25,126, 0 / 852, 24,226, 3 | identical | 932, 27,715, 0, byte-identical |
| homebrew_486 | 104, 104 of 130 | 1008, 27,273, 0 / 1033, 27,657, 0 / 1039, 27,226, 0 | identical | 971, 26,869, 0, byte-identical |

coco2's 593 to 1144 vias is the order alone: the schematic tier's coco2
row had been one draw of that. "Byte-identical" is the placed board, the
routed board and so the copper. The orientation search's pair
(homebrew_486 saved by pcbnew from 0 degrees and from the designer's
angles, `build/orient-search/homebrew_486-*`), which differ in the
angles, the graphics' order and the spelling of angles: x1132 placed
both alike (the race then kept other seeds), x1160 (the order alone)
placed 102 of 130 parts elsewhere (layouts 959 against 1056 vias), x1161
(with the exact quarter turns) gives the same poses and the same copper
(971 vias, 0 unconnected; the files written differ only where the inputs
spell an angle differently).

Tiers (`--jobs 6 --threads 4`, under the lock). Short tier x1161
against `build/short-x1151.log` (x1152's calibration, the order's
baseline): every placement reshuffled (the stack's file holds the
designer's order); passes 6 to 7 (OpenFC), unconnected 30 to 19, new
errors 3 to 2, vias 1178 to 1134, length 21,528 to 21,228 mm, work 2327
to 2103 s, cut 0; no row worse by compare.py's rule. Schematic tier x1161
against x1152 on the same tasks (`build/sch-x1152-order-base.log`):
passes 8 to 10 (OpenFC, TelemetryOnboard), unconnected 6 to 4, vias 3027
to 3184; the five boards whose import was already in reference order
(urchin, spell_tome, USB_Keypad, 0xcb-static, bumwings) give identical
rows and placements, the five whose order changed reshuffle (coco2 717 to
932 vias, homebrew_486 960 to 971, OpenFC 165 to 148 and 2 to 0
unconnected, the chiffre 62 to 112, MokyaLora 420 to 400), and on two
the placer's seeds are the same but the router follows its new order:
slab-pcb-left's race probes give other counts (9, 185, 4 against 8, 9, 12
unfinished) and keep another seed (405 to 326 vias, 1 to 2 unconnected),
TelemetryOnboard's layout moves one part elsewhere (1 to 0 unconnected).
Three short-tier boards (Blue_Line, spell_tome, 0xCB-1337) at 1, 2 and 4
threads: the same rows and byte-identical boards.

## The board's own texts and silkscreen graphics (2026-10-21)

The breadboard fence (an external report, `~/tmp/fence-report.md`,
finding 2) labels its headers and test pads with board texts on F.SilkS
(`3V3  VDD  5V`, `GND`, `VDD`, `DIO`, `CLK`). The placer put SW1 under
`3V3  VDD  5V` (free text was no obstacle), and the polish after routing
then moved the text off the part's silkscreen, 7.5 mm to the right and
under a socket; the test pads' names were shuffled so that `DIO` sat
under the `VDD` pad. A text the input placed is an intent: it stays, and
the parts keep off it.

**What is a mark** (`board_placer::board_marks`, on with
`keep_board_marks`): every visible board text (`gr_text`, KiCad 6's
`gr_text locked`) and text box (`gr_text_box`) on F.SilkS, B.SilkS, F.Cu
and B.Cu, by the box the lowering computes for copper texts (KiCad's
stroke font: size, thickness, justification, lines; a knockout text's
plate; a cached TrueType text's glyphs), and the board's silkscreen
graphics (`gr_line`, `gr_rect`, `gr_arc`, `gr_circle`, `gr_poly`,
`gr_curve`) in clusters of shapes within 0.5 mm of each other, each
cluster blocking with its shapes' boxes (a rectangle drawn around a group
blocks with its four sides, not its inside; a dense logo is coarsened to
at most 48 boxes, runs of a grid that doubles until they are few
enough). Left out: hidden texts, marks off the outline's box, and a
cluster that spans more than 0.8 of the board's width or height (a frame
around the board, an image behind every part: Blue_Line's 8793
polygons), which the polish never moved either. Inner-layer copper texts
are not marks (the placer models no inner layer) but stay where they are
too, and the router routes around them.

**A mark on a movable part belongs to that part** (`attach_marks`): one
whose boxes overlap a movable part's body (courtyard and pads) on its
side, or the part's holes and pads there from the other side, at the
part's input position labels that part (a connector's pin names inside
its courtyard, a chip's name on its outline). The placement moves the
part, so the mark is no obstacle; it stays where the input put it and is
listed in `board-placer.json` (`attached_marks`) and `board-layout.json`
(`labels.attached`), not among the overlaps. Of several parts, the one
overlapping most claims it. Parts stacked at one origin claim nothing:
their input position says nothing (four of the fence's five movable
parts lie at one spot under its back title; a benchmark task stacks every movable
part at one point, so in the tiers' tasks almost nothing attaches, see
below). Marks over fixed parts, over free space and at the edge stay
obstacles. The attachments are taken from the input once; the layout's
lowerings of the placed board keep them.

**What a mark does.** It is a fixed obstacle on its side with a
keepout's semantics: a part's body (courtyard and pads) on that side
stays the spacing away, and a part on the other side keeps its holes and
pads there off it (a pin through a back label would cut it). A fixed
part already on a mark stays, with a warning in the placement's
`constraint_warnings` ("fixed part TP1 lies on the board's text "GND"
(F.SilkS at 81.50, 58.90): both stay where the input put them"); fixed
parts never block each other. The room model counts the marks as fixed
area. A part meets a silkscreen mark without its routing halo (tracks
run under silkscreen; a copper text keeps the halo, as a copper graphic
does). **A part without room lies on a mark**: where a level leaves
parts without a place, they are seated again with the marks out of
their way (`seat_on_marks`; `Constraints::yielding`, `on_marks`,
`Relaxation::on_marks`; `parts_on_marks` in `board-placer.json`), every
other part keeping its place, or, where that seats fewer, every part
legalized again with those first (MIDAS-MK1.1's two 36 mm headers at
the bottom edge, whose spots the others had taken), before the next
level takes halos or spacing from every part (OpenFC's and
MIDAS-MK1.1's connectors at the edge under their own pin names,
USB_Keypad's J1, J2 and four switches beside its artwork). The same happens after the levels when the work budget ended
them with parts still unplaced. A placement with fewer parts on marks
ranks first among the seeds and in the race. A first try made the marks
give way only at a level of their own after the courtyard level: the
three boards then gave up halos and spacing first (USB_Keypad placed
with courtyards touching, 35/39 against 37/39 routed), and OpenFC's
levels spent the placement budget before that level (CN1 and USB1
without a place).

**The polish** (`labels`, after routing) used to move, and now moves:

| item | before 2026-10-21 | now (`keep_board_marks`) |
| --- | --- | --- |
| footprints' visible Reference and Value fields on the silkscreen | moved to the nearest free spot around the part, turned, shrunk to 0.8 | the same; the board's marks are taken spots they keep off (since 2026-10-22 by KiCad's own rules: "The fields on the silkscreen" below) |
| board texts on F.SilkS and B.SilkS | moved to the nearest spot off the parts' silkscreen and pads (up to 25 mm) | stay |
| board silkscreen graphics in clusters (not those spanning the board) | moved as one piece (up to 30 mm) | stay |
| board texts on copper layers (no mask twin) | taken off the board while routing, then moved off the copper and the parts (turned when that fitted) | stay; the router routes around them (as route mode always did) |
| copper texts with a mask twin, footprints' own texts, texts on other layers, text boxes, dimensions | stay | stay |

`repair-kicad-silkscreen` moves footprints' fields and front silkscreen
strokes only; it treats the board's drawings as fixed.

**What is reported.** A movable part whose silkscreen, pads or label
(one without a free spot) lies on a silkscreen mark after the layout, or
whose pads or body lie on a copper text, is listed in `board-layout.json`
(`labels.overlaps`: the mark, its layer and anchor, and each part with
what of it lies there) and in the verdict's next_actions (keep the parts
off with a keepout constraint, a region or fixed positions, or move the
text in the input). Fixed parts are not listed (the placement warned).

**Measured** (changelog, "Committed on 2026-10-21 (fence report: placer
side)"; the numbers here are before marks on movable parts were
attached, which in the tiers' tasks fires on MIDAS-MK1.1 alone, since
the tasks stack their movable parts). The fence without its workaround keepouts: no board text moves
(seven did, up to 7.5 mm), no movable part lies on a label's spot (SW1
did), the verdict complete as before (187 vias against 186). The short
tier, whose tasks keep the designer's texts while re-placing the
designer's parts: every board has marks (3 to 104), passes 8 -> 5,
KiCad unconnected 18 -> 20, vias 1068 -> 1254, HPWL up to +82 %
(USB_Keypad, whose artwork takes a fifth of the front). About half the
designers' board texts (3144 of 6088 on the 107 boards of tasks v7, by
a rough box) lie on their own parts: pin names inside a connector's
courtyard, a chip's name on its outline. For such tasks
`"keep_board_marks": false` gives the rows before (checked on the
tier). The schematic tier's boards have no board texts or silkscreen
graphics (the import writes none): their placements with x1385
and x1417 are byte-identical.

## The fields on the silkscreen (2026-10-22)

Every layout result had KiCad silkscreen findings (`silk_over_copper`,
`silk_overlap`, `silk_edge_clearance`; the fence's issue 8), and an agent
had to run `repair-kicad-silkscreen` afterwards. On the fence's v5
four-layer result (x1539) 11 of its 15 came from footprint fields: four
labels the polish found "no room" for stayed at their default offset, on
their own pads (C26, C27) or a neighbour's pad and outline (J2, JP1). The
old polish judged spots by a rough box (30 % too wide) against pads with
a 0.1 mm margin and kept a label where it was when nothing was free.

**The scorer** (`silk_fields.rs`) computes KiCad's own rules from our
geometry, measured against kicad-cli 10.0.7 on test boards:
- `silk_over_copper`: the field's ink overlaps a mask opening (a pad on
  `F.Mask`, `B.Mask` or `*.Mask`, grown by its `solder_mask_margin`, its
  footprint's or the board's `pad_to_mask_clearance`; a graphic on a mask
  layer). Touching is no finding and `min_silk_clearance` does not apply;
  copper without a mask opening is none.
- `silk_overlap`: within `min_silk_clearance` of another silkscreen item:
  every footprint's graphics and texts, its own included, the other
  fields (its own Value too), the board's texts and graphics. One finding
  per item, as KiCad reports them.
- `silk_edge_clearance`: within `min_silk_clearance` of the outline or a
  cutout, or off the board.
- The ink: KiCad's stroke font per printable ASCII character (advance and
  ink, measured with pcbnew's `GetEffectiveTextShape`), the pen (default
  an eighth of the size, at most a quarter), justification (a justified
  text starts 0.658 pens past its anchor), mirroring and KiCad's upright
  rule (a text at 90 to 270 degrees reads turned half way). Its box agrees
  with pcbnew's to 0.001 mm on the strings tested; KiCad checks the glyph
  strokes, so the box is on the safe side (a pad in the notch of a "V" is
  no finding).

**The candidates.** A field stays where it is if KiCad will find nothing
there. Else: the four sides of its part's body (courtyard and pads), each
slid from corner to corner (up to 16 spots) at six distances (0.1 to
2.2 mm), at the field's angle and a quarter turn, and a grid inside the
body of a part larger than the text; shrunk to 0.8 (not below the
project's `min_text_height`, 0.8 mm at least, nor `min_text_thickness`)
only when no full-size spot is clean. The fewest findings win, then the
smallest cost: the move in mm, +1.5 off the side the field was on, +0.8
turned, +3 shrunk, +2.5 on another part's body, +1 on its own from
outside. Big parts first; a placed field is an obstacle for the rest.
Fields are the visible Reference and Value (KiCad 8's `property`, KiCad
6's `fp_text reference` and `value`, written back with `unlocked` kept);
the footprints' own texts and graphics and the board's (`keep_board_marks`)
never move.

**No clean spot.** The field stays at its best spot and is reported:
`labels.silkscreen` in `board-layout.json` and the verdict's
`silkscreen` fact (`fields`, `moved`, `blocked` with the findings
predicted there, `hidden`, `predicted` by KiCad's type), and a next
action. With `"silkscreen": {"hide_when_blocked": true}` in layout.json
such a field is hidden instead.

**repair-kicad-silkscreen** keeps its pcbnew worker for the front
strokes (clipped off pads, other silkscreen and the edge) and the
isolated crosses, which no longer moves the reference fields; then the
same placement runs on its result (both sides, no shrinking and no
hiding, so the guards on text, size and visibility hold), also where the
worker fails (a KiCad 6 board, whose records carry `tstamp`s, not uuids).
On the fence's v5 four-layer result of x1539: 15 findings before, 2 after
(J1's back outline over the edge, which the front-only worker leaves).

**Tests.** `the_scorer_predicts_kicads_silkscreen_findings_and_placing_clears_them`
(kicad-cli on the PATH): a board of fifteen fields in each predicament
(own pads, a neighbour's pad, its own outline, another label, its Value,
the edge, a board text, a back THT part, a turned part, copper without a
mask, 0.05 mm from a pad, 0.15 mm from an outline), at
`min_silk_clearance` 0 and 0.2: the predictions per field equal KiCad's,
and after placing KiCad finds nothing. The ink against pcbnew's numbers,
and a boxed-in field kept or hidden, without KiCad.

**Measured** (changelog, "Committed on 2026-10-22 (silkscreen
placement)"): the fence's v5 four-layer board 15 -> 4 KiCad silkscreen
findings (x1592 -> x1552; the 4 are connector outlines over the edge),
the copper byte-identical; the short tier's cosmetic findings 1951 ->
1950, its boards byte-identical outside the fields: they hide most of
their references, and what KiCad reports there is the parts' and the
board's own graphics.

## One notion of a held part (2026-10-21)

Which parts the placer leaves where the input has them used to be
decided three times: by the lowering (the placer's `fixed` and
`fixed_patterns`, constraints.json's `fixed`, parts placed with `x` and
`y`, by default locked parts, parts without a net and parts at the edge,
and the parts a constraint or `move_all` frees), by the keepout
constraints (`HeldParts`: the placer's lists, locked and netless parts,
less `free`) and by the free dimension (its own variant, without `free`
and the edge). A keepout over a part fixed by constraints.json or over a
connector at the edge was written footprints not allowed, and KiCad
reported the part (the fence's issue 15 for that input); a part a
constraint frees got footprints allowed and a warning.

Now one function decides it from the lowering (`held_parts` in
placement_constraints.rs: the board with the outline and sides the
constraints give, lowered with the placer config, each footprint's
`fixed`), and the keepout constraints (which rule areas allow
footprints, the input-time warning) and the free dimension (which parts
must stay inside the outline, which move with the edge by their body)
take it from there. Under `move_all` the placer's own `fixed` list goes
too, as the lowering has always had it: every part is placed but locked
ones and those constraints.json fixes or places. The unit test
`one_notion_of_a_held_part_covers_every_case` enumerates the cases.

## What went wrong on the way (kept as lessons)

- A density target below 1 can never be met when every part is larger than a
  bin; overflow plateaued at 0.24–0.40 and the layout froze. Bodies are now
  exclusive at density 1 and whitespace is controlled by halos and fillers.
- Greedy largest-first legalization doubled the wirelength by exiling small
  passives. The global placement was fine; the annealing legalizer fixed it.
- Minimum wirelength is not routability: the most compact Interf-U placement
  has the best wirelength and does not route. Only routing feedback tells.

## Known limits / next steps

- Bodies are rectangles or discs; rotations are quarter turns; the placer
  never changes a footprint's side by itself (`back`/`front` constraints
  do, see [constraints.md](constraints.md)), so the orientation search
  has no mirror to try.
- The board's top-level items (graphics, zones, tracks, the net list)
  are taken in the file's order; only the footprints and their own items
  are sorted (see "Order independence" above). Nothing measured follows
  them yet. Of the numbers a save may spell another way only the
  quarter-turn angles (-180 or 180, -90 or 270) lower alike; a save
  that rounds coordinates is another input.
- Constraints: see [constraints.md](constraints.md). `row` lines parts up
  (and fills a grid with `columns`); there is no other alignment
  constraint yet.
- Congestion drives nudges of individual parts, but does not yet feed the
  global placer's density term (coupling step 3).
- Reference and Value fields are placed after routing by the polish
  (`labels`, `silk_fields`) where KiCad's silkscreen checks find nothing
  ("The fields on the silkscreen"); what KiCad still reports on the
  silkscreen is the footprints' and the board's own graphics (a
  connector's outline over the edge, a logo footprint over parts), which
  nothing moves; `repair-kicad-silkscreen` clips front strokes. A board text that labels a
  part the placer moves does not move with it (the benchmark tasks' pin
  names of their connectors): it stays, and the part keeps off it until
  the last level.
- Pin swapping runs once after placement when the layout config names a
  `pin_swaps` file ([pin-swap.md](pin-swap.md)); it is not yet part of the
  move loop. No gate swapping.
- The spectral solve is a direct O(n^3) cosine transform (64 bins); an FFT or
  a GPU kernel becomes relevant for boards with thousands of parts.
