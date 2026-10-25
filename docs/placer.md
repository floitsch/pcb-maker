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
   it blocks with its courtyard's shape (`mask_openings`, below), and so
   are its copper graphics (since 2026-10-23: a printed antenna's copper
   beside its three small pads, which the pads' box let hang over the
   board edge).
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
   - spacing down to the copper clearance (the classes' largest, or a
     custom rule's between the pads of every two nets on some layer:
     docs/router.md, "Project rules");
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
   Where the placement takes its whole 40 %, the race's other probes
   run but count for nothing (the probes past 40 % of the budget are
   dropped); `race_probe` (below, "The race's route probe") charges
   every probe to the clock and gives each a share of the budget.
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
below; since 2026-10-23 the tasks drop the labels of the parts they
stack, benchmarks/agent-tasks/README.md "Tasks"). Marks over fixed parts, over free space and at the edge stay
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
other part keeping its place, or, where that seats fewer (and not both
many parts failed and a pass is costly: "Placer refusals since v18"
below), every part legalized again with those first (MIDAS-MK1.1's two 36 mm headers at
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

## Placer refusals since v18 (2026-10-23)

The v19 sweep (x1539) refused four boards v18 (x1315) placed:
OM-FlexGrid-Flex (J1), SmartSpin2k's panel (12 parts), OpenRX's panel
(81 parts) and the molecule 5x4_wave (J3, J4). Bisected by building the
commits between x1534 and x1535 (placement only, `place-kicad-board` on
the tasks' prepared boards): none of it is the held-part or the
free-dimension change.

- **OM-FlexGrid-Flex** (6ebd4e4, the edge pads' copper-to-edge rule):
  at the edge-copper levels a held part's pads keep the project's
  copper-to-edge clearance (0.5 mm here) on every side, not only the
  0.2 mm copper clearance beside the edge. J1, a 19-pin flex connector
  (pads 11.15 mm wide) in its 12.4 mm tab, then has a window of 0.25 mm
  along the edge (x 63.525 to 63.775), and the eviction's whole-board
  scan, the only search that reaches the tab from where the anneal left
  J1 (the fine grid's ring search looks 15 mm around), steps 0.5 mm
  from the board's left end: 63.5 and 64.0, both outside. Now a part
  held at one edge (or two parallel ones) that the coarse scan finds no
  spot for is scanned again on the level's grid along that edge, pulled
  flush against it (`legal::scan_steps`, `constraints::flush_center`):
  J1 at 63.6, its pads 0.575 and 0.675 mm from the tab's sides. Only
  where the coarse scan found nothing, so every placement that had a
  spot is unchanged.
- **SmartSpin2k and OpenRX** (057b3a4, the board's marks): the tasks
  stack their parts, so none of the designers' labels attaches (the
  panels' pin labels lie where the designers' connectors were), and the
  marks made the placement too slow for its budget rather than the board
  too full. SmartSpin2k's first level took the whole 600 s of work (a
  legalization 125 to 147 s against 82 to 104 without marks, the
  eviction 33 to 55 s against 3 to 8, and `seat_on_marks`' passes over
  every part 200 to 280 s), so the looser levels that place it never
  ran. Two changes:
  - The searches skip positions a part's regions exclude
    (`legal::region_window`): the ring search of the legaliser runs only
    the rings and steps whose body centre the regions admit, and the
    eviction's scans skip such positions before testing them. A panel's
    copies put a region on every movable part, and a part without room
    searched the whole board, nine copies' worth on SmartSpin2k, at every
    pass. Exact: those positions were illegal; only the work counted
    changes, and only on boards with regions (in the tiers, the panels
    whose copies have their own nets).
  - `seat_on_marks` no longer legalizes every part again (with the
    failed ones first) where both an eighth of the movable parts or more
    found no room and a pass over every part (the level's legalization)
    costs more than a 32nd of the placement's budget, so that its four
    passes would take more than an eighth. Many failures mean a level
    too tight, which the next level eases. OpenRX's panel had 55 of 216
    parts without room at full halos, a pass of 35 s, and spent 100 s of
    its 600 s on those passes at each of the first two levels, so the
    spacing level that places it (as x1315 did, at 105 % of the front's
    area) never came. MIDAS-MK1.1, whose headers the passes are for,
    fails 1 to 10 of 160 parts at a time, and reCamera fails more of fewer
    parts with cheap passes: both unchanged. A gate on either condition
    alone moved them.
  SmartSpin2k: 12 parts without a place -> none (U2 twice and U5 seven
  times on marks); OpenRX: 81 -> none.
- **The molecule 5x4_wave** (the marks): not recovered. Its four
  DisplayPort connectors a side (J3 left, J4 right, 17.5 mm each, with
  J5 and J6 on the same edges) fill 70 of the edge's 76 mm, and the
  designer's pin marks lie at each connector's spot, 19.05 mm apart. The
  connectors placed off the marks leave the last one no slot; seating it
  on the marks (also with every part allowed on them, by eviction and by
  legalizing every part again) found none either. Without marks
  (`keep_board_marks: false`) it places. The other four molecules were
  refused by v18 already.

## The sweep boards without a result (2026-10-25)

The nightly sweep of 2026-10-25 (main 356c8db) left nine of the 109
boards without a result: six panels and boards of repeated cells refused
by the placer (the OpenESC 4in1 panel and the five PolyKybd molecules),
two boards refused since the binaries of 2026-10-24 (katia, the laptop's
power board), and the 0xCB pcb-panel, whose KiCad check never finished.
Diagnosed on the tasks' prepared boards with `place-kicad-board` and
`PCB_PLACER_DEBUG` (the levels below are its lines), on x1820 (main
23f629e). The layout rows before and after are in the changelog
("Committed on 2026-10-25 (the seven sweep boards without a result)").

- **The 0xCB pcb-panel: KiCad waits forever, not slowly.** `kicad-cli
  pcb drc --refill-zones` used 15 s of CPU and then slept (main thread in
  a polling sleep, every worker idle) until the 900 s limit, on our
  result, on our placed board without tracks and on the designer's own
  board alike. Bisected by removing zones: two of the panel's pours alone
  (Board_0-GND and Board_1-GND, F&B.Cu, priority 1: each copy's pours
  reach 23 mm into its neighbour) hang it; any one pour, or two on
  different layers, does not. With distinct priorities it finishes. It is
  not every such pair: BAGEL-MK1 has twelve and its checks finish, and a
  small test board with two overlapping pours and pads of both nets in
  the overlap finishes too. The panel's outline also has 36 Edge.Cuts
  lines from a point to itself (SmartSpin2k's panel 115): KiCad then
  finds no outline (`invalid_outline`, "segment has null or very small
  length") and fills to the board's box; with them removed the placed
  board's check took 210 s instead of 504 s (load about 20). The repair
  (`check_repairs`, before every DRC with zones refilled, so on the
  boards a layout writes and in drc-kicad-board's copy of a reference):
  the lines of no length go, and the overlapping copper zones of two nets
  at one priority get distinct priorities in KiCad's own order: KiCad
  breaks the tie by uuid (`ZONE::HigherPriority`, the greater uuid fills
  first), so the repair writes `p * k + r` (r the rank by uuid among the
  paired zones of priority p, 0 for the others): the fixture F1 (a +5V
  pour inside a GND pour at one priority) gives the same report, where an
  order by file position changed it. Boards without either defect are not
  touched; in the tiers only the two 0xCB panels, the OpenESC 4in1 panel
  (+BATT and GND on F.Cu), BAGEL-MK1 and BAGEL-MK1.1-Avocado have such
  pairs, and SmartSpin2k's and the 0xCB pcb-panel's outlines lines of no
  length. Our result's check: about 110 s. The timeout stays at 900 s.
- **The OpenESC 4in1 panel: the budget, not the room.** Two different
  boards (the 4in1 and the 4in1-mini) joined by tabs, sharing net names
  (so `keep_copies` does not apply). Its front side is 107 % full at the
  full spacing; the single 4in1 board places at the tight-body level,
  which every level before it skips as overfull. The panel's levels are
  not overfull by area (the tabs' outline holds more), so each ran: the
  spacing level (27-37 parts without room), the fine grid (a kept
  placement and a fresh anneal, 250-330 s), the edge inset; the 600 s of
  work ended before the tight bodies. With 3000 s it places at the tight
  level (0 parts left). Now the level loop goes on to the first
  tight-body level at once when more parts lack room than a level's
  passes seat (a fiftieth of the parts, three at least) and the work left
  is less than twice the last level's work for each level before it
  (`skip_to_tight`): the panel then places within its 600 s (seeds with
  17 and 23 parts left jump with 174 and 100 s left; one seats every
  part). Where the rules keep courtyards apart (no tight level) the jump
  goes to the fine grid without spacing: the laptop's power board (98 %
  of its front full, its pad clearance of 0.127 mm floors the spacing
  since 2026-10-24) went from 35 parts left after the halo levels to 4-5
  on the fine grid and none at the edge inset (refused at 600 s before,
  placed at 4000 s).
- **katia: the legaliser's escape demand.** On since 2026-10-24, the
  escape term was computed at every spot the legaliser tried, before the
  legality test: a pass took 134 s against 21 s without it, and the
  deadline came before the level that places katia (2 parts left at every
  halo level). The legaliser now tests legality first and prices the
  escape overflow of legal spots only. The spot chosen is the same (the
  least distance among the legal spots); the work counted is less, so
  placements that ran into their budget change.
- **The molecules: the designer's own placement is illegal.** Boards of
  repeated key cells (4 x 4 to 7 x 5) joined by AtomConnect edge
  connectors, one net group (not a panel of copies), courtyard overlaps
  an error. `PCB_PLACER_DESIGNER` on the designer's board, at the
  placer's loosest level: 128 movable parts of the 4x4 (87 of the
  5x4_wave) are illegal at the designer's poses: the mating connectors of
  neighbouring cells (J3/J4, J5/J6) lie on each other, J1 sits closer to
  C2 than the copper clearance, and the edge connectors' pads reach over
  the outline. With 4000 s of work the 4x4 still leaves 28 parts, the
  5x4_wave 1-2. They stay refused, now with a reason an agent can act
  on (below).
- **Measured** (layout rows at the task budget, x1820 against x14421,
  in the changelog): the 4in1 panel, katia, the power board and the
  0xCB pcb-panel have a result now (499, 155, 16 and 93 unconnected, no
  new error); the molecules' parts without a place fall from 48, 60, 22,
  42, 48 to 29, 45, 5, 3, 9. The short tier keeps 11 of 12 boards
  byte-identical; MIDAS-MK1.1, whose placement ran into its budget,
  changed (8 to 9 open).
- **Refusals name movable parts, a repeated reference by uuid.** The
  verdict's "N parts without a legal place (...)" took the unplaced
  parts' names and added those of every illegal part not yet named,
  fixed ones and the board's marks included: the 4x4 molecule's list
  ended in "D1, J1, J5, R1, SW1, silkscreen mark, cutout" (fixed parts
  and obstacles a part was left on) and named J3 four times without
  saying which. board-placer.json now has `unplaced_parts`: each movable
  part without a legal place once, its `label` (the reference, with
  `(uuid ...)` where the reference repeats on the board, as
  constraints.json takes it), `status` (unplaced or illegal), `at` and
  `held_by` (the edge, region and overhang constraints that hold it,
  "edge left within 0.52 mm"); the verdict's list is the labels, and the
  hint for a held part names its label and constraints.

## Escape demand (2026-10-23)

`escape_demand` (placer option, on since 2026-10-24, `null` turns it off; `{"weight",
"inflate", "turn_tolerance", "measured", "measure_every",
"threshold"}`, defaults 2 mm, 1, 0.05, on, 20, 0.5). The plan agreed
with Florian: put the routing structure where the placement is made,
from general signals, and keep it to the end (phase 2 of
[global-route.md](global-route.md) put a routing term into the anneal
only, and the legaliser and refinement that follow, optimising
wirelength alone, undid it). Code `crates/pcb-placer/src/escape.rs`, the
lowering in `board_placer/config.rs` (`escape_fact`) and
`board_placer/guidance.rs` (`measured_factory`).

**The fact** (`Problem::escape`, lowered per board: the track pitch
`track_pitch_mm`, the routable layers, and the nets that count: two pins
or more, not carried by a pour). Per footprint and side (-x, +x, -y, +y
of its body box at its pose), the **escape demand**: each of its pads'
nets leaves through the side on the cheapest way to the net's nearest pad
on another part (out to the side, twice the way under the body, then
straight to that pad), one exit per net and side, recomputed from the
parts' current positions as they move. Between two parts on one board
side whose bodies face each other across a gap (overlapping along the
side, at most 5 mm apart) the **channel**: the exits of both facing
stretches (less those going to the other part, which cross the gap)
must run along it, and it holds the gap over the pitch times the
routable layers. Demand over capacity is the channel's overflow,
attributed to both parts. A dense side facing the board edge or a part
without demand towards it costs nothing. Unit tests in `escape.rs`
(sides chosen, poured nets, overflow at four gaps and its symmetry, the
facing stretch, the zero cases, a move into a full channel).

**Where it acts.**

- Global placement (`global.rs`): every rotation interval (15
  iterations) each part's charge grows on each side by its demand there
  times the pitch over the layers (`inflate`): a dense side needs room in
  front of it. The added area pushes (density and the field's force);
  the overflow that stops the placement still counts the bodies.
- Rotation choice (`orient::escape_blocking`, from `choose_rotations`):
  among the angles within `turn_tolerance` (5 %) of the shortest HPWL,
  the one whose dense sides face the least body area (a band in front of
  each side as deep as its demand's tracks, weighted by the demand; the
  board edge is free) wins; ties by HPWL, then the angle held.
- Legaliser (`legal::legalize_keeping`): a spot's distance grows by the
  overflow of the channels the part forms there with the parts already
  placed, `weight` mm per track (the spots' search order and its early
  stop are unchanged: the term only adds).
- Refinement (`legal::refine_with`): a move or swap pays `weight` mm per
  track of overflow of the channels the moved part forms (its exits
  fresh, the neighbours' as last refreshed: the moved part and the parts
  sharing a net of up to 24 pins with it are refreshed after a kept
  move).
- The measured version (refinement): the global route's overflow
  attribution per part (`board_placer::attribution` at `threshold` 0.5 of
  the capacity, phase 2's measure), from a `pcb_global::Incremental` of
  the placement at the refinement's start, carried along (the moved
  parts' copper and terminals rigidly, their nets routed again) and read
  every `measure_every` kept moves. It scales each part's term by
  `2 a / (a + mean a)` (0 where the real route has no excess on the part's
  nets, 1 at the mean, towards 2 above): the estimate is corrected by a
  route late in placement. The layout lowers it with its router
  configuration (the one hook in `board_layout.rs`); `place-kicad-board`
  has no router rules and runs without it. Its work counts as placement
  work, as phase 2's evaluations do.

With the switch off none of this runs: `Problem::escape` is `None`, the
global placement, the rotation choice, the legaliser and the refinement
take their old paths.

### Judgement

**Verdict: the rule's conditions hold; proposed as the default, with the
spread below as the caveat.** The eleven dense boards' summed KiCad
unconnected falls 973 to 903 (-7 %; better on 5, worse on 5, equal on
1), and the short tier loses nothing (unconnected 22 to 18, strict passes
6 to 6). The open terminals after the first route fall on both (dense
1349 to 1273, short 47 to 23).

Single layouts at the task budgets (tasks v7, 1500 s of work, pinned
rates 3.15e6 and 150e6), both sides x1751 (branch
placement-escape-demand 9d01bdb), 5 x 2 for the dense boards and 3 x 2
for the short tier, `--min-free-gb 10`, no lock, beside a full sweep and
another agent's runs; the on side `--placer '{"escape_demand": {}}'`.
Output `build/escape1-dense-{off,on}` and `build/escape1-dense2-{off,on}`
(the memory ledger starved one side's later boards; the rest ran again
with `--expected-gb 2` and the first rows spliced in: 4 off, 1 on),
`build/escape1-short-{off,on}`. HPWL is the half-perimeter over the
nets of the final board (unweighted); the ceiling is the designer's
placement routed by our router (`build/designer-x1539-matched.log`; the
x1725 run had no dense rows yet).

**Off reproduces main.** `place-kicad-board` with x1725 (main 38edddd)
and x1751 off (60 s of placement work at the pinned rates) writes
byte-identical boards on all eleven dense and all twelve short-tier
boards; the off layouts' rows equal the move-descent experiment's off
side (x1691) on the boards where nothing downstream changed since
(4in1 79, Glasgow 148, SmartSpin2k 80, panel 53, split72_left 38, MIDAS
7, USB_Keypad 2). The fixture goldens do not move (check.sh --fast).

| Board | KiCad unconnected off / on | first route open off / on | strict pass | HPWL mm off / on | vias off / on | work s off / on | designer's placement routed (x1539): unconnected, strict |
| --- | ---: | ---: | --- | ---: | ---: | ---: | --- |
| OpenESC 4in1-mini | 37 / 57 | 61 / 87 | no / no | 740 / 775 | 134 / 139 | 1271 / 1427 | 2, no |
| Glasgow | 148 / 150 | 164 / 194 | no / no | 3094 / 3104 | 279 / 266 | 1467 / 1370 | 50, no |
| SmartSpin2k | 80 / 43 | 82 / 47 | no / no | 18861 / 17727 | 1793 / 1690 | 1388 / 1302 | 0, no |
| 0xCB panel | 53 / 53 | 82 / 84 | no / no | 5601 / 5699 | 244 / 315 | 1226 / 969 | 0, yes |
| OpenESC 4in1 | 79 / 73 | 154 / 142 | no / no | 1237 / 1225 | 159 / 169 | 1376 / 1393 | 0, yes |
| split72_left | 38 / 20 | 38 / 46 | no / no | 12730 / 12923 | 1114 / 863 | 1263 / 1398 | 1, no |
| protoesp | 76 / 77 | 124 / 103 | no / no | 5676 / 6050 | 388 / 431 | 1208 / 1207 | 1, no |
| video | 165 / 188 | 228 / 238 | no / no | 32694 / 34300 | 1482 / 1533 | 1388 / 1509 | 0, yes |
| MegaDrive | 191 / 126 | 214 / 162 | no / no | 28995 / 30214 | 2120 / 2273 | 1574 / 1523 | 109, no |
| SNSP-CPU-01 | 91 / 107 | 109 / 119 | no / no | 19117 / 18559 | 1763 / 1835 | 1334 / 1431 | 46, no |
| DSP-ADAU1452 | 15 / 9 | 93 / 51 | no / no | 4049 / 4278 | 270 / 319 | 1321 / 1291 | 4, no |
| sum | 973 / 903 | 1349 / 1273 | 0 / 0 | 132794 / 134853 | 9746 / 9833 | 14816 / 14821 | |

| Board | KiCad unconnected off / on | first route open off / on | strict pass | HPWL mm off / on | vias off / on | work s off / on |
| --- | ---: | ---: | --- | ---: | ---: | ---: |
| eternal-keypad | 0 / 0 | 0 / 0 | no / no | 2233 / 2368 | 34 / 34 | 102 / 113 | |
| MIDAS-MK1.1 | 7 / 7 | 17 / 7 | no / no | 3069 / 3062 | 234 / 234 | 265 / 330 | |
| USB_Keypad | 2 / 2 | 2 / 2 | no / no | 1625 / 1372 | 181 / 122 | 472 / 560 | |
| WLED_Matrix | 8 / 4 | 12 / 7 | no / no | 1844 / 1755 | 194 / 176 | 405 / 519 | |
| reCamera | 4 / 4 | 10 / 5 | no / no | 355 / 330 | 83 / 63 | 313 / 289 | |
| OpenFC | 1 / 1 | 2 / 1 | no / no | 1267 / 1286 | 163 / 163 | 445 / 403 | |
| 0xCB 1337-v4.0 | 0 / 0 | 0 / 0 | yes / yes | 1293 / 1272 | 73 / 64 | 101 / 725 | |
| USB_Cable_Tester | 0 / 0 | 0 / 0 | yes / yes | 1711 / 1670 | 106 / 101 | 134 / 136 | |
| framework_mobo_lefthalf | 0 / 0 | 4 / 1 | yes / yes | 1090 / 1105 | 101 / 76 | 605 / 566 | |
| Blue_Line | 0 / 0 | 0 / 0 | yes / yes | 239 / 252 | 26 / 26 | 5 / 8 | |
| spell_tome_bottom | 0 / 0 | 0 / 0 | yes / yes | 264 / 265 | 9 / 8 | 27 / 21 | |
| urchin | 0 / 0 | 0 / 0 | yes / yes | 2571 / 2576 | 39 / 39 | 55 / 63 | |
| sum | 22 / 18 | 47 / 23 | 6 / 6 | 17561 / 17311 | 1243 / 1106 | 2930 / 3732 |

Every short-tier placement differs (the inflated charge changes the
global placement from the first rotation interval on, so every seed and
the race's choice move). The rows: WLED_Matrix 8 to 4 (first route 12
to 7, vias 194 to 176, HPWL -5 %); USB_Keypad equal at 2 with HPWL -15 %
and vias 181 to 122 (the placer's own wirelength 1460 to 939 mm);
reCamera equal at 4, first route 10 to 5, vias 83 to 63; MIDAS equal at
7, first route 17 to 7; OpenFC equal at 1 (first route 2 to 1, the
placer's wirelength +23 %); framework_mobo passes both (first route 4
to 1, vias 101 to 76); 0xCB 1337-v4.0 passes both, its 725 s of work
against 101 s are a final ladder (596 s of search) the on board's first
route triggered with an item KiCad counted where the router did not (5
checks, none cached), ending at the same board quality; eternal-keypad,
USB_Cable_Tester, Blue_Line, spell_tome and urchin within a few percent.
The summed work (+802 s) is that ladder and the placement's own extra
work.

The dense rows: the gains are SmartSpin2k (80 to 43, first route 82 to
47), MegaDrive (191 to 126, 214 to 162), split72_left (38 to 20, vias
1114 to 863), DSP-ADAU1452 (15 to 9, first route 93 to 51) and 4in1 (79
to 73); the losses 4in1-mini (37 to 57, first route 61 to 87), video
(165 to 188: the placement took 609 s of work against 525 s, its
routing the less), SNSP-CPU-01 (91 to 107) and Glasgow and protoesp by
one or two. The measured version ran 11 measurements and 217 kept
moves per refinement on average over the dense boards' 75 refinements;
the placement's work grows (video +16 %, Glasgow +19 %) within its
budget.

### Reading

- **The first route moves, and here the final count follows on the
  sum.** Unlike phase 2 (the anneal only, lost again in the legaliser
  and refinement), the term is kept to the end: the open terminals after
  the first route fall on the sum of both tiers (dense 1349 to 1273,
  though on only 5 of the 11 boards; short tier lower or equal on every
  board with any), and the KiCad count falls on the sum of both tiers.
- **The spread is the size any perturbation of these boards gives.**
  Phase 2 and the move descent moved single dense boards by 17 to 25
  from seed changes alone; 5 better against 5 worse is no direction per
  board, and the dense drop rests on MegaDrive (-65) and SmartSpin2k
  (-37). The short tier is the cleaner signal: no row lost, three rows'
  first routes halved or better, vias -11 %.
- **What it does not do yet**: the channel term ignores the neighbours'
  exits changing when a part moves (refreshed after a kept move only);
  the legaliser has the estimate only (a measurement per legalisation
  would cost a full global route at every level of every seed); the race
  still ranks by RUDY and the placer's key by wirelength.

Proposed: `escape_demand` on by default, after a seed-repeated pair on
the dense boards (the same rule) confirms the sum; the switch stays as
the override (`{"escape_demand": null}` turns it off once the default
is `Some`).

### Seed confirmation and the default (2026-10-24)

Two more pairs on the eleven dense boards, the placer's seed set moved
whole (`--placer '{"seed": 101}'` and `201`: the race's 16 seeds start
there; seed 1 is the default, the pair above, and seed 2 would share 15
of its 16), x1752 (main 85cafb7, the switch off by default) both sides,
5 x 2 capped at six layouts in flight, `build/escape2-dense-s{101,201}-{off,on}`.
Seed 1's pair is x1751's above (the same placer and router code). KiCad
unconnected per board:

{table}

The summed medians fall 968 to 933 (-3.6 %), and every one of the three
pairs falls on its own sum (seed 1: 973 to 903, 101: 951 to 919, 201:
1132 to 1019); by the median 6 boards are better and 5 worse. Better
on all three pairs: 4in1 and DSP-ADAU1452 (on 9, 11, 35 against 15, 40,
37); the 0xCB panel is worse on two and equal on one (53-57 against
48-53); the other seven split 2 to 1 either way: SmartSpin2k, MegaDrive
and split72_left better on two, video (on 188-197, off 128-347: off
spreads widely, on does not), 4in1-mini, Glasgow, SNSP-CPU-01 and
protoesp worse on two. The first route's open terminals rise in the summed medians
(1300 to 1437), all of it the 0xCB panel (77 to 243); without it they
fall 1223 to 1194. The short tier's single pair lost nothing (above).
By the rule (dense sum down, short tier not worse) the default is on
since 2026-10-24: `escape_demand` absent means the defaults above;
`{"escape_demand": null}` gives the placement before it (checked: the
same board byte for byte as main on 4in1-mini). `place-kicad-board`
runs on the estimate alone (no router rules, so no measured version).

## Only thermals starved: the pads solid first (2026-10-24)

The layout's final route used to start route mode's ladder with the pour
nets as tracks whenever KiCad found a starved thermal, also when KiCad and
the router both found every connection (`current.1 > 0` in
`board_layout.rs`). On the short tier's 0xCB 1337-v4.0 that ran 6
attempts and 596 s of search with x1751 and 3 attempts with x1754 (498 s
of work against 109 without them), and then `solidify_starved_thermals`
fixed the pad in one KiCad check anyway. Now, when only thermals are
starved (0 unconnected by KiCad and by the router, no internal
violation, pours connected, `pours` auto), the starved pads go solid
first (one KiCad check, charged as work; kept only when KiCad finds the
board better, `solid_kept`), and the ladder runs only when that leaves a
starved thermal or anything open (`layout: only thermals starved: the
starved pads solid first: ...` on stderr). The try after the ladder runs
only on a board the ladder kept. Every other path is unchanged.

Measured with x1754 (main 3036703) against x1780 (the change), 3 jobs x
2 threads, `--min-free-gb 10`, beside other agents' runs, tasks v7 at the
task budgets (the eleven dense boards) and the short tier at its third:

| Board | KiCad unconnected | work s | vias | path (x1754 / x1780) |
| --- | ---: | ---: | ---: | --- |
| 0xCB 1337-v4.0 (short tier) | 0 / 0 | 498 / 109 | 64 / 72 | ladder, its last attempt kept / solid first |
| framework_mobo_lefthalf | 0 / 0 | 392 / 122 | 76 / 96 | ladder, its first attempt kept then solid / solid first |
| eternal-keypad | 0 / 0 | 113 / 73 | 34 / 34 | ladder, none kept, then solid / solid first (the same board) |

Three of 23 boards took the new path (all on the short tier: every dense
board has connections open after its first route), and on all three the
solid pads left nothing starved or open, so no ladder ran after them.
Passes and DRC errors are equal on every board (eternal-keypad's one
`shorting_items` on both). The ladder's boards it replaces had fewer vias
on two boards (8 and 20 more without it); the work falls 699 s over the
three. On the other 20 boards (11 dense, 9 short) the result boards are
byte-identical. Logs `build/fix3-{dense,dense2,short,cb}-x{1754,1780}`.

## Routes survive moves (2026-10-23)

Florian's idea 2: "make it easier to move components without losing all
existing traces: moving something to make space shouldn't require a
full reroute". Inside the coupled loop that already holds (a move trial
is `Router::update` and a reroute of what the move touched, and the move
descent's `update_nets` keeps even the untouched branches of a moved
part's nets), but the layout's last step did not: the final ladder, and
the narrow-signal attempt standing in for it on boards far from
complete, routed the final placement from scratch, and the board the
moves and the descent had left only competed with it.

`final_route_from_state` in layout.json (`PCB_LAYOUT_FINAL_FROM_STATE`
overrides; false, as before, by default: see the measurement below)
starts the final route from the router's state instead:

- **What is kept.** Every net's legal copper of the state (complete
  nets and the connected pieces of open ones, pour nets' stubs and
  stitching vias with them), written into the placed board as its own
  copper, which route mode keeps as it keeps a designer's pre-routed
  nets (`kept.rs`: the pads it joins are one terminal, the router routes
  what is open). Not kept: a net in a violation of the router's own
  check, and every net with copper within 1 mm of a terminal the state
  left open (`KEPT_FREE_RADIUS_MM`). Kept as walls, the open nets found
  no way out (4in1-mini: 20 of 28 open nets without any path, DSP 30 of
  41); 1.5 mm freed nearly the whole board (4in1-mini kept 59 of 152
  nets). The pours connect as the state connected them (connected, or as
  tracks; no rung of the other mode).
- **The ladder on the rest**, with at most 0.3 of the work left
  (`KEPT_SHARE`; half took DSP's continuation): the finer pitches, seeds
  and the narrow-signal step work on the nets not kept.
- **From scratch as the fallback**: at once when the ladder's first
  attempt on the kept routes leaves more open by the router's count than
  the layout's first route (a fresh first attempt) did (`fresh_open`; the
  ladder stops there, `KEPT_GAVE_UP`), and otherwise with the work the
  kept route left while connections stay open. KiCad's check decides
  between the boards as before; the attempt after the narrow-signal
  attempt (the other pour mode) varies the one from scratch.

What it cannot do: the kept copper is fixed. The router never rips up
a kept net, so an open terminal whose way out runs through kept copper
more than 1 mm away stays open; only the route from scratch can move
those nets. A warm start in which the router owns the state's routes
(and may rip them up in its negotiation) on a new lattice or with the
narrow widths is the step this leaves.

The narrow-signal attempt run alone (boards far from complete) now ends
within its share: its seeds stop at their share of the work left (they
had no deadline: DSP-ADAU1452 after the descent charged 1470 s of 742
s), and with twice the floor left it keeps the attempt after it (the
other pour mode, 240 s) its floor from the start.

**Measured (x1741, changelog "routes survive moves"): no default.**
Over the eleven dense boards at the task budgets KiCad's unconnected
items go from 973 to 979 with the descent off (better 0, worse 3) and
from 988 to 998 with it on; the short tier is identical (22 / 22,
passes 6 / 6). The kept route's board was kept once (4in1-mini after
the descent, still 5 worse than off); on the nearly complete short-tier
boards it left more open than the state (OpenFC 4 against 2): the nets
freed around an open terminal lose the room the state had found for
them and cannot take back any other. The move descent does not pay
with it either (998 against main's 973 on this base).

## Placement ablation (2026-10-24)

On the 25 strict-failing target boards with both results, v20's layouts
leave 986 open against 564 for our router on the designer's placement
(benchmarks.md, "Layout sweep v20"). Placement levers chosen by
intuition (seed scoring, in-anneal global-route guidance, bus bundles,
the move descent) did not pay; escape demand did. This experiment asks
which placement decisions cost, as general signals, before the next
lever is built. Nothing in the placer or the router changed.

**The tool.** `pcb-maker ablate-kicad-placement <designer.kicad_pcb>
<placed-dir> <board-id> <out-dir> [ablation.json]` (docs/agents.md, code
`crates/pcb-kicad/src/placement_ablation.rs`) writes hybrids of the
designer's placement and ours: `d-<group>` is the designer's placement
with one group of parts where we put them, `o-<group>` ours with the
group where the designer put it, beside `designer` and `ours`. Every
variant is our placed board (the task's preparation: tracks stripped,
pours and rule areas, labels) with only footprint poses changed, so the
variants differ in poses alone (the `designer` variant's global route
equals that of the designer's stripped board exactly). Groups: (a) every
movable footprint with more than 20 pads alone, (b) the same with the
passives (at most four pads, not connectors) that share a net with it
and lie within 3 mm of its body in either placement, (c) the connectors,
(d) the board's quadrants by the designer's positions. A moved group
lands on parts of the other placement: those it collides with (pads
within 0.1 mm, or a body more than a quarter on another; the placer's
own legality, even at its tightest, calls 63 of MegaDrive's and 183 of
Glasgow's designer parts illegal, so it cannot be the test) move to the
nearest free spot, everything else stays; the report lists them
(`displaced`). Null variants nudge one passive by 0.1 mm: a route's
noise. `benchmarks/agent-tasks/ablation.py` lays the variants out as a
`--route-from` tree with one task per variant and routes them with run.py
in route mode at the task budget (1200 s of work), every variant with
x1754 (main 3036703) and pinned rates, 3 at a time with 2 threads,
through the memory ledger with 10 GB kept free; the tool's boards are
x1811/x1812's (byte-identical between the two). The judge is KiCad's
unconnected count, as run.py judges. Our placements are v20's (x1725,
`build/github-layout-v20/<task>/layout/placed`), the ones the gap above
was measured on. Output `build/abl1/{megadrive,glasgow}` (hybrids,
routes, `routes.log`), signals measured again by x1812 in
`build/abl1/*-sig`.

**Attribution.** For a group, `cost d` = open(d-group) - open(designer)
(what our placement of it costs the designer's board) and `cost o` =
open(ours) - open(o-group) (what the designer's placement of it would
save ours); the cost is their mean. The baselines are means over the
placement and its two nulls. Two things limit the reading:

- **Noise.** A 0.1 mm nudge of one passive moves MegaDrive's routes by a
  standard deviation of 12.7 (ours 143, 158, 177; the designer's 56, 62,
  67), so a single route's cost there carries about ±15; Glasgow's
  routes are steady (2.4). On MegaDrive only costs above about 30 are
  distinguishable from noise.
- **Displacement.** A group lands on the other placement's parts, and
  those move. In ours, packed tighter, the designer's group displaces
  many (Glasgow's U30 23 parts, its south-west quadrant 44): every
  `o` variant of Glasgow that displaces ten or more parts leaves more open
  than ours, so its negative `cost o` is the disruption, not the
  designer's choice. The `d` side displaces fewer and reads cleaner.
  Groups are also not additive (MegaDrive's four quadrants sum to 125
  against a gap of 98).

Clusters of MegaDrive's DIPs and the cartridge connector (J3, U2, U3,
U7, U10: two to four parts each, nearly their large part alone) were not
routed; Glasgow's parts of 12 to 20 pads and MegaDrive's U4-U8 pair as
one group were prepared (`build/abl1/{glasgow,megadrive}-extra`) and not
routed, nor was the OpenESC 4in1 (time).

**MegaDrive**: routed open, the designer's placement [56, 62, 67] (mean 61.7), ours [143, 158, 177] (mean 159.3); the nudges' standard deviation 12.7.

| group | parts | open: designer's + our group / ours + the designer's group | cost d / o | cost | displaced d / o |
| --- | ---: | ---: | ---: | ---: | ---: |
| quadrant-nw | 92 | 156 / 128 | 94 / 31 | 62.8 | 5 / 14 |
| cluster-U4 | 26 | 136 / 129 | 74 / 30 | 52.3 | 3 / 10 |
| cluster-U8 | 21 | 110 / 140 | 48 / 19 | 33.8 | 2 / 11 |
| large-U8 | 1 | 94 / 127 | 32 / 32 | 32.3 | 2 / 7 |
| quadrant-sw | 82 | 123 / 159 | 61 / 0 | 30.8 | 20 / 5 |
| large-U4 | 1 | 119 / 159 | 57 / 0 | 28.8 | 1 / 7 |
| quadrant-se | 46 | 96 / 143 | 34 / 16 | 25.3 | 6 / 5 |
| large-U7 | 1 | 64 / 141 | 2 / 18 | 10.3 | 1 / 4 |
| quadrant-ne | 28 | 111 / 196 | 49 / -37 | 6.3 | 3 / 8 |
| large-U3 | 1 | 73 / 169 | 11 / -10 | 0.8 | 5 / 4 |
| large-J3 | 1 | 64 / 163 | 2 / -4 | -0.7 | 0 / 0 |
| large-U2 | 1 | 66 / 165 | 4 / -6 | -0.7 | 3 / 1 |
| large-U9 | 1 | 58 / 157 | -4 / 2 | -0.7 | 0 / 2 |
| large-J2 | 1 | 51 / 152 | -11 / 7 | -1.7 | 0 / 0 |
| cluster-U9 | 7 | 53 / 154 | -9 / 5 | -1.7 | 2 / 3 |
| large-U10 | 1 | 52 / 154 | -10 / 5 | -2.2 | 0 / 3 |
| large-U6 | 1 | 51 / 166 | -11 / -7 | -8.7 | 3 / 3 |
| connectors | 11 | 55 / 195 | -7 / -36 | -21.2 | 1 / 2 |
| cluster-U6 | 8 | 52 / 193 | -10 / -34 | -21.7 | 3 / 3 |

| signal | designer's | ours | orders the two right | Spearman with the variants' costs | sign agrees (costs beyond 2 se) |
| --- | ---: | ---: | --- | ---: | ---: |
| wirelength (mm) | 29390.3 | 28995.0 | no | 0.31 | 11 of 14 |
| escape channel overflow (tracks) | 0.1 | 24.4 | yes | 0.36 | 8 of 14 |
| exits facing a body within 5 mm | 362 | 669 | yes | 0.52 | 12 of 14 |
| ratsnest crossings | 7931 | 11710 | yes | 0.60 | 12 of 14 |
| ratsnest crossings between large parts | 4849 | 5200 | yes | 0.39 | 11 of 14 |
| global route overflow | 8.0 | 1.0 | no | 0.29 | 2 of 14 |
| global route soft overflow at 0.5 | 5426.5 | 5398.0 | no | 0.59 | 13 of 14 |
| global route overflowed nets | 10 | 1 | no | 0.03 | 3 of 14 |

**Glasgow**: routed open, the designer's placement [53, 52, 51] (mean 52.0), ours [126, 132, 131] (mean 129.7); the nudges' standard deviation 2.4.

| group | parts | open: designer's + our group / ours + the designer's group | cost d / o | cost | displaced d / o |
| --- | ---: | ---: | ---: | ---: | ---: |
| quadrant-ne | 72 | 114 / 108 | 62 / 22 | 41.8 | 40 / 19 |
| quadrant-se | 75 | 85 / 111 | 33 / 19 | 25.8 | 24 / 15 |
| quadrant-sw | 62 | 91 / 143 | 39 / -13 | 12.8 | 27 / 44 |
| connectors | 7 | 84 / 142 | 32 / -12 | 9.8 | 21 / 10 |
| cluster-U30 | 52 | 70 / 143 | 18 / -13 | 2.3 | 5 / 33 |
| large-J5 | 1 | 50 / 130 | -2 / -0 | -1.2 | 0 / 2 |
| cluster-J5 | 6 | 51 / 132 | -1 / -2 | -1.7 | 1 / 1 |
| large-U30 | 1 | 71 / 173 | 19 / -43 | -12.2 | 4 / 23 |
| quadrant-nw | 28 | 53 / 187 | 1 / -57 | -28.2 | 2 / 12 |

| signal | designer's | ours | orders the two right | Spearman with the variants' costs | sign agrees (costs beyond 2 se) |
| --- | ---: | ---: | --- | ---: | ---: |
| wirelength (mm) | 3640.9 | 3093.5 | no | 0.81 | 9 of 13 |
| escape channel overflow (tracks) | 31.0 | 50.3 | yes | -0.54 | 5 of 13 |
| exits facing a body within 5 mm | 609 | 571 | no | -0.12 | 6 of 13 |
| ratsnest crossings | 815 | 1153 | yes | 0.94 | 12 of 13 |
| ratsnest crossings between large parts | 91 | 96 | yes | 0.73 | 6 of 13 |
| global route overflow | 57.0 | 147.0 | yes | 0.11 | 7 of 13 |
| global route soft overflow at 0.5 | 775.7 | 882.1 | yes | 0.82 | 11 of 13 |
| global route overflowed nets | 82 | 141 | yes | 0.13 | 8 of 13 |


**What differs in the top groups** (the tool's per-part escape of both
placements, `build/abl1/*-describe/ablation.json`; "demand" is escape
demand's exits per side, "within 1 mm" the demand on sides that face
another body closer than 1 mm):

| group | placement | demand | on sides facing a body within 5 mm | within 1 mm | channel overflow | nearest-pin distance mm | board edge mm | global route attribution |
| --- | --- | ---: | ---: | ---: | ---: | ---: | ---: | ---: |
| MegaDrive quadrant-nw | designer's | 282 | 120 | 35 | 0.2 | 16.1 | 35.8 | 13712 |
| MegaDrive quadrant-nw | ours | 281 | 201 | 89 | 10.0 | 16.6 | 54.9 | 17280 |
| MegaDrive cluster-U4 | designer's | 177 | 8 | 3 | 0.0 | 19.0 | 47.8 | 13420 |
| MegaDrive cluster-U4 | ours | 177 | 162 | 152 | 22.0 | 29.7 | 77.6 | 11686 |
| MegaDrive cluster-U8 | designer's | 129 | 34 | 29 | 0.0 | 12.9 | 67.0 | 9293 |
| MegaDrive cluster-U8 | ours | 129 | 91 | 54 | 8.6 | 12.2 | 78.7 | 10366 |
| Glasgow quadrant-ne | designer's | 203 | 185 | 120 | 31.4 | 2.8 | 12.4 | 19728 |
| Glasgow quadrant-ne | ours | 202 | 178 | 99 | 17.1 | 2.5 | 14.7 | 28300 |
| Glasgow quadrant-se | designer's | 198 | 183 | 110 | 30.2 | 2.8 | 12.3 | 20595 |
| Glasgow quadrant-se | ours | 197 | 153 | 64 | 32.9 | 3.5 | 15.5 | 29758 |
| Glasgow quadrant-sw | designer's | 173 | 140 | 119 | 0.0 | 2.8 | 12.1 | 15947 |
| Glasgow quadrant-sw | ours | 173 | 133 | 101 | 44.1 | 2.4 | 15.4 | 21748 |

- **MegaDrive: the two QFPs traded places.** U4 (QFP-160, 160 pads) and
  U8 (QFP-128) share 63 nets with each other and 52 and 49 with the
  cartridge connector J3, 53 and 49 with the fixed DIP-64 U1 and 43 and
  36 with the expansion connector J2, which sit to the right (x 172 to
  275). The designer puts U4, the part with the most nets to U1 and J2,
  on their side (x 202) and U8 left of it (x 132); ours has them the other
  way round (U4 at x 141, U8 at x 192). Orientation and board side are
  the same (U4 at 0 degrees, U8 at 270, both on the front); the edge
  distance is larger in ours. The wirelength does not see it: ours is
  1.3 % shorter, U4's nearest-pin distance 44.6 mm against 54.4. The
  ratsnest does: U4 alone moved to our spot adds 934 crossings to the
  designer's board, U8 alone 1027, our north-west quadrant (U8 and 91
  parts) 2025. The second difference is escape room: our U4 has four
  small parts (EM4, EM11, EM13, EM33) 0.5 to 0.6 mm in front of all four
  pin rows (all 143 exits face a body within 1 mm, channel overflow 14.4;
  the designer's U4 has nothing within 5 mm), and U8 has C23 0.8 mm off
  its +x row (overflow 8.6). The clusters (the QFPs with their passives)
  cost more than the parts alone (52 and 34 against 29 and 32), but at
  ±15 a route that is no measured difference.
- **MegaDrive: the south-west and south-east quadrants** (the DIP-40
  U6, the DIP-28s U7, U2, U3 and their passives) cost 61 and 34 from the
  designer's side (means 31 and 25); the parts alone cost nothing measurable (U6 -9, U2
  -1, U3 1, U7 10): what costs there is where the many small parts go
  relative to them, again with more crossings (2387 and 1618 added).
- **Glasgow: the small parts in the east.** The north-east quadrant (72
  parts: the 14-pin arrays D22 and D23, the TSSOP-16 U19, the header J2
  and many resistor networks and passives) costs 62 / 22, the south-east
  (D20, D21, the TSSOP-16 U5 and theirs) 33 / 19: both sides positive,
  the clearest result of the experiment. Ours pulls these parts towards
  the FPGA (U19 from (113, 92) to (89, 105); the 12-pad U8 and U36 from
  (66, 115) and (66, 110) to the middle): shorter (the wirelength 15 %
  less) but crossing: the designer's board with our north-east quadrant
  has 1680 more crossings. Escape room does
  not explain it: our north-east quadrant has less demand within 1 mm
  of a body (99 against 120) and less channel overflow (17 against 31)
  than the designer's. The BGA U30 alone costs 19 from the designer's
  side (its decoupling packed tighter, nearest-pin distance 8.4 mm
  against 12.9, channel overflow 40.8 against 0) and its cluster 18; the
  `o` side of both is displacement (23 and 33 parts moved).

**The signal check.** For each signal its value on both placements and,
over the variants, its change against the variant's cost (the same pairs;
Spearman's rank correlation, and how often the sign agrees where the
cost is beyond twice its standard error). Then, as an independent
check, the same signals on six of our own placements per board (v20 and
the escape-demand pairs' six layouts, x1725 to x1752, seeds 1, 101,
201, off and on) against their layout results, and the designer's
placement (its x1725 matched-tier route):

| board | placement | open | wirelength mm | channel overflow | exits facing a body | ratsnest crossings | between large parts | global overflow | soft overflow 0.5 |
| --- | --- | ---: | ---: | ---: | ---: | ---: | ---: | ---: | ---: |
| MegaDrive | designer's (x1725 matched route) | 56 | 29390 | 0.1 | 362 | 7931 | 4849 | 8 | 5426 |
| MegaDrive | github-layout-v20 | 191 | 28995 | 24.4 | 669 | 11710 | 5200 | 1 | 5398 |
| MegaDrive | escape1-dense2-on | 126 | 30214 | 0.4 | 628 | 9453 | 4122 | 1 | 5173 |
| MegaDrive | escape2-dense-s101-off | 157 | 33183 | 23.4 | 722 | 13522 | 6497 | 1 | 6644 |
| MegaDrive | escape2-dense-s101-on | 99 | 31260 | 0.0 | 639 | 10382 | 4941 | 0 | 5994 |
| MegaDrive | escape2-dense-s201-off | 116 | 30826 | 17.3 | 702 | 10151 | 4721 | 0 | 5556 |
| MegaDrive | escape2-dense-s201-on | 174 | 33296 | 0.0 | 608 | 14325 | 7205 | 0 | 6115 |
| glasgow | designer's (x1725 matched route) | 55 | 3641 | 31.0 | 609 | 815 | 91 | 57 | 776 |
| glasgow | github-layout-v20 | 148 | 3094 | 50.3 | 571 | 1153 | 96 | 147 | 882 |
| glasgow | escape1-dense2-on | 150 | 3104 | 82.5 | 604 | 1121 | 115 | 117 | 907 |
| glasgow | escape2-dense-s101-off | 162 | 3090 | 39.8 | 616 | 1008 | 81 | 166 | 1186 |
| glasgow | escape2-dense-s101-on | 163 | 3207 | 46.2 | 610 | 1215 | 63 | 167 | 1048 |
| glasgow | escape2-dense-s201-off | 192 | 3020 | 27.1 | 628 | 1266 | 92 | 220 | 1328 |
| glasgow | escape2-dense-s201-on | 170 | 3362 | 64.6 | 602 | 1308 | 56 | 137 | 1138 |
| 4in1 | designer's (x1725 matched route) | 0 | 1190 | 8.6 | 396 | 242 | 0 | 0 | 94 |
| 4in1 | github-layout-v20 | 79 | 1237 | 32.6 | 427 | 467 | 0 | 0 | 173 |
| 4in1 | escape1-dense2-on | 73 | 1225 | 17.3 | 421 | 417 | 0 | 3 | 155 |
| 4in1 | escape2-dense-s101-off | 87 | 1235 | 21.0 | 423 | 417 | 0 | 2 | 158 |
| 4in1 | escape2-dense-s101-on | 68 | 1153 | 17.2 | 425 | 370 | 0 | 0 | 150 |
| 4in1 | escape2-dense-s201-off | 91 | 1379 | 30.6 | 427 | 505 | 0 | 0 | 177 |
| 4in1 | escape2-dense-s201-on | 90 | 1208 | 20.7 | 428 | 308 | 0 | 0 | 128 |

- **Escape demand's channel overflow** (the signal we have) orders the
  two placements right on both boards, but follows the variants' costs
  weakly (Spearman 0.36 on MegaDrive, -0.54 on Glasgow, where our
  costly north-east quadrant has less of it). On MegaDrive the placer
  already drives it to zero (escape demand on: 0.4, 0.0, 0.0) and those
  placements still leave 99 to 174 open against the designer's 56: room
  in front of the pin rows was one cost, and it is paid; the rest is
  not that.
- **The global route's soft overflow at 0.5** (we have) follows the
  variants' costs nearly as well as the best (0.59, 0.82) and ranks our
  own placements on Glasgow (0.83) but not on MegaDrive (0.03), and it
  calls the designer's MegaDrive placement worse than ours (5426
  against 5398) and than two of our six. Its total overflow and
  overflowed nets (channel capacity in the global route) follow nothing
  (0.29 and 0.03 on MegaDrive, 0.11 and 0.13 on Glasgow).
- **Wirelength** is wrong between the placements on both boards (ours
  shorter) and ranks our six placements at -0.09, -0.03 and 0.60.
- **Ratsnest crossings** (a signal we lack) order the two placements
  right on both boards, follow the variants' costs best (Spearman 0.60
  and 0.94, sign agreement 12 of 14 and 12 of 13), give the designer's
  placement the fewest on all three boards against every one of our 18
  placements (MegaDrive 7931 against 9453 to 14325, Glasgow 815 against
  1008 to 1308, the 4in1 242 against 308 to 505), rank our placements at
  0.60, 0.71 and 0.32, and get 7 of the 9 off/on pairs right (soft
  overflow 8, channel overflow 7). Restricted to lines between two large
  parts they are weaker (0.39, 0.73): the crossings that count are not
  only the buses between the big parts.

**How the missing measure is computed** (`ratsnest_crossings` in the
tool's signals): for every net the escape fact counts (two pins or
more, not carried by a pour), the minimum spanning tree of its pins'
positions (Prim's, straight distance); then the pairs of tree lines of
different nets that properly intersect, counted (bounding boxes first,
then the orientation test; under a second for MegaDrive's 1300 pins).
It is what HPWL cannot see: two orders of the same parts along a bus, or
a port's chain folded back on itself, have the same bounding boxes and
different crossings, and on a two- or four-layer board every crossing is
a via pair or a detour.

**Recommendation: one lever, the ratsnest crossings as a placement
cost.** Counted incrementally for the moved part's nets (as escape
demand's channel term is), in the legaliser and the refinement where
the term survives to the end, and in the race's ranking of the seeds.
Predicted gain from the attribution, an upper bound and not measured:
on MegaDrive the groups the crossings explain are the U4/U8 swap and the
two southern quadrants, about 60 of the 98 open connections between the
two placements (the north-west quadrant alone 63, the clusters of U4
and U8 52 and 34, overlapping); on Glasgow the two eastern quadrants, up
to about 65 of 78 (42 and 26, not additive). The cross-placement table sets the scale differently: among our own
six MegaDrive layouts the three with the fewest crossings left 99, 116
and 126 open, the three with the most 157, 174 and 191.

## Ratsnest crossings (2026-10-24)

`ratsnest_crossings` (placer option, on since 2026-10-24, `null` turns
it off; `{"weight", "legalise", "refine", "rank_seeds", "anneal",
"global"}`, defaults 3 mm, on, on, on, off, off; the last two below,
"Where the order is decided"). The
placement ablation's lever (above): on MegaDrive and Glasgow the costly
decisions are part orders along buses, which wirelength cannot see and
ratsnest crossings can. Code `crates/pcb-placer/src/crossings.rs`, the
lowering in `board_placer/config.rs` (`crossing_nets`) and `place.rs`,
the race in `board_layout.rs`.

**The measure** is the ablation's, one implementation: for every net
escape demand counts (two pins or more, not carried by a pour), the
minimum spanning tree of its pins (Prim's from the net's first pin,
straight distance, a tie to the earlier pin: `crossings::spanning_tree`),
and the pairs of tree lines of different nets that properly cross
(bounding boxes, then the orientation test; touching and collinear lines
do not: `crossings::lines_cross`). `ablate-kicad-placement` now calls
both, so the placer and the tool count the same thing.

**The incremental count** (`CrossingState`). Every counted net's tree
lines lie in a uniform grid (about two lines a cell, cells 1 to 20 mm,
at most 2^16), each line in every cell it passes through (walked column
by column with 1e-6 mm of slack; points beyond the grid fall into its
border cells, so two lines that cross share the cell of their crossing).
A move or turn of parts changes only the trees of the nets on their
pins: `involving(poses, moved)` builds those trees at the new poses,
walks each new line through the grid and tests the lines it meets,
skipping its own net, the old lines of the moved nets, and pairs of two
new lines from the second side (each pair once). The result is the
crossings of the moved nets with everything at the new poses; the same
call at the old poses gives the old ones, so their difference is the
move's change exactly. `kept` replaces the moved nets' lines and keeps
the board's total. The work counted is the pins' positions, Prim's k²
per tree, the cells walked and the lines met. A unit test moves, turns
and swaps random parts 300 times (some beyond the board) and checks the
total against the direct all-pairs count after every kept move and the
`involving` difference before it; nothing depends on the order lines
were added in, so the count is deterministic.

**Where it acts.**

- Legaliser (`legal::legalize_keeping`): a legal spot is farther by
  `weight` mm per crossing it adds to those of the part's nets where the
  part wants to be (the parts not placed yet where they want to be;
  fewer crossings earn nothing, so the term only adds and the search's
  early stop holds), and the search looks at most 2 mm beyond the
  closest legal spot (`CROSSING_REACH`): without that bound a spot's
  crossings widened the ring search by their weight, and the legaliser
  took 7 times its work on MegaDrive (3.2 s to 24 s a legalisation).
- Refinement (`legal::refine_with`): a move, turn or swap pays `weight`
  mm per crossing of the moved parts' nets at the new poses against
  those at the old; the term is computed only for legal candidates that
  still beat the best (it is never negative), and one state is carried
  through the refinements and relation repairs of `finish`.
- The race's seed ranking (`board_layout.rs`): each placement of the
  pool gets its crossings (`crossings::total`, milliseconds), and the
  placements are ordered by the sum of their ranks by the congestion
  score (RUDY) and by the crossings, a tie to the better score; the
  courtyard-touching last resort sorts after as before. The counts are
  in board-layout.json's `congestion.crossings`.
- The anneal and the global placement: behind the sub-switches
  `anneal` and `global`, off by default (below, "Where the order is
  decided").

With the switch off none of this runs (`Problem::crossings` is `None`):
`place-kicad-board` with x14270 off and x1813 (main 2e41ea3) writes
byte-identical boards on MegaDrive and Glasgow, and since the default
flipped, `{"ratsnest_crossings": null}` against x1816 (main c2d82c4)
the same.

**The weight** (`experiments/crossings/calibrate.py` on build/abl1):
wirelength (unweighted HPWL, mm) plus w times the crossings, against the
routed open connections, for w from 0 to infinity:

| w (mm per crossing) | 0 | 0.5 | 1 | 2 | 3 | 5 | crossings alone |
| --- | ---: | ---: | ---: | ---: | ---: | ---: | ---: |
| MegaDrive variants: Spearman (44), sign agrees (19 beyond the noise) | -0.08, 16 | 0.75, 16 | 0.84, 17 | 0.86, 16 | 0.87, 14 | 0.86, 14 | 0.86, 13 |
| Glasgow variants (24, 14) | -0.12, 10 | 0.18, 11 | 0.29, 11 | 0.74, 12 | 0.78, 12 | 0.79, 12 | 0.85, 13 |
| our six MegaDrive layouts: Spearman | -0.09 | -0.09 | 0.09 | 0.60 | 0.60 | 0.60 | 0.60 |
| our six Glasgow layouts | -0.03 | 0.09 | 0.60 | 0.60 | 0.60 | 0.71 | 0.71 |
| our six 4in1 layouts | 0.60 | 0.60 | 0.37 | 0.37 | 0.37 | 0.37 | 0.37 |
| the designer's placement the cheapest of 7 (MegaDrive, Glasgow, 4in1) | no, no, no | yes, no, yes | yes, no, yes | yes, no, yes | yes, yes, yes | yes, yes, yes | yes, yes, yes |

Wirelength alone orders nothing; the orderings saturate from 2 mm, and
3 mm is the smallest weight in the scan at which the designer's
placement is the cheapest on all three boards (on Glasgow the designer's is 547
mm longer with 338 fewer crossings than v20's, and 551 mm longer with
193 fewer than the escape-demand pair's seed-101 off layout, which takes
a weight above 2.85). Default 3 mm. The placer's
own wirelength weights large nets down, so in its units 3 mm is a little
more than in the scan's.

**Cost.** On MegaDrive and Glasgow (`place-kicad-board`, 3 seeds, the
default budget) the placement's work rises 26 % and 31 %, nearly all of
it the refinement (Glasgow 572 to 947 s of work over the three seeds,
the relation repairs' refinements 4 to 100 s); the capped legaliser is
within 10 % of off. In the dense layouts below the placement's work
(board-placer.json, summed over the eleven boards) rose 3078 to 3604,
3177 to 3684 and 2914 to 3590 s (+17 to +23 %), out of the same layout
budget. These are the count's own units of then, which booked
1.5 to 3.7 times too little of its CPU; since 2026-10-25 it books per
operation ("The crossings' work booking" below).

### Measurement

The escape-demand confirmation's protocol: the eleven dense boards in
layout mode at the task budgets (tasks v7, pinned rates), three seed
pairs (`--placer '{"seed": N}'`, N = 1, 101, 201, and on the on side
`"ratsnest_crossings": {}`), off x1813 (main 2e41ea3), on x14270
(branch ratsnest-crossings 1e58ff3: main 2e41ea3 and the crossings,
the switch off by default then), 3 x 2 with at most six layouts in
flight, through the memory ledger with 10 GB kept free, beside other
agents' runs; no row cut by the wall clock. Output
`build/cross1-dense-s{1,101,201}-{off,on}`, `build/cross1-short-{off,on}`.
Both sides carry the OpenESC boards' clearance errors of that day
(the re-fetched `.kicad_dru`, fixed on main as x1814: placement only;
like with like here, about 400 clearance errors a side, 4in1-mini and
4in1 above their usual open counts).

KiCad unconnected per board, off / on:

| Board | seed 1 | seed 101 | seed 201 | median |
| --- | ---: | ---: | ---: | ---: |
| OpenESC 4in1-mini | 102 / 91 | 74 / 79 | 101 / 105 | 101 / 91 |
| OpenESC 4in1 | 125 / 128 | 101 / 101 | 132 / 124 | 125 / 124 |
| DSP-ADAU1452 | 8 / 17 | 10 / 7 | 35 / 25 | 10 / 17 |
| split72_left | 20 / 23 | 25 / 21 | 29 / 33 | 25 / 23 |
| video | 188 / 190 | 190 / 145 | 197 / 159 | 190 / 159 |
| Glasgow | 164 / 145 | 163 / 163 | 170 / 175 | 164 / 163 |
| SmartSpin2k | 43 / 34 | 61 / 59 | 46 / 52 | 46 / 52 |
| 0xCB panel | 53 / 50 | 57 / 50 | 56 / 51 | 56 / 50 |
| protoesp | 77 / 105 | 83 / 76 | 74 / 90 | 77 / 90 |
| MegaDrive | 126 / 110 | 99 / 108 | 174 / 168 | 126 / 110 |
| SNSP-CPU-01 | 107 / 53 | 132 / 95 | 85 / 87 | 107 / 87 |
| sum | 1013 / 946 | 995 / 904 | 1099 / 1069 | 1027 / 966 |

The summed medians fall 1027 to 966 (-5.9 %) and each pair falls on its
own sum (seed 1: 1013 to 946, 101: 995 to 904, 201: 1099 to 1069); by
the median 8 boards are better and 3 worse (DSP-ADAU1452 10 to 17,
SmartSpin2k 46 to 52, protoesp 77 to 90). MegaDrive's median 126 to
110, Glasgow's 164 to 163 (its seed 1 164 to 145, the others within
5). The largest gains are video (190 to 159, better on two seeds by 45
and 38), SNSP-CPU-01 (107 to 87; seed 1 107 to 53) and MegaDrive. The
router's open terminals after the first route fall on every pair (1417
to 1271, 1582 to 1305, 1571 to 1480); vias 9671 to 10174, 10316 to
9414, 9805 to 9484.

**The lever moved what it was meant to move.** Ratsnest crossings of
the placed boards (`layout/placed`, counted by `ablate-kicad-placement`,
x1813), the designer's placement beside them, off / on:

| Board | designer's | seed 1 | seed 101 | seed 201 |
| --- | ---: | ---: | ---: | ---: |
| OpenESC 4in1-mini | 279 | 214 / 69 | 168 / 58 | 165 / 79 |
| OpenESC 4in1 | 242 | 447 / 206 | 370 / 150 | 305 / 201 |
| DSP-ADAU1452 | 170 | 931 / 744 | 1025 / 710 | 1142 / 720 |
| split72_left | 3201 | 4559 / 3639 | 4983 / 3745 | 4527 / 3853 |
| video | 8765 | 19233 / 19726 | 20345 / 16987 | 16180 / 14018 |
| Glasgow | 815 | 1121 / 829 | 1215 / 843 | 1308 / 953 |
| SmartSpin2k | 1410 | 3863 / 3149 | 4201 / 3348 | 4281 / 3301 |
| 0xCB panel | 319 | 580 / 460 | 557 / 493 | 655 / 480 |
| protoesp | 373 | 1228 / 1102 | 1262 / 875 | 1301 / 1046 |
| MegaDrive | 7931 | 9453 / 8136 | 10382 / 9042 | 14325 / 9952 |
| SNSP-CPU-01 | 5076 | 5919 / 4286 | 5956 / 4765 | 5774 / 4336 |
| sum | 28581 | 47548 / 42346 | 50464 / 41016 | 49963 / 38939 |

Fewer crossings on 32 of the 33 pairs (video's seed 1 the exception,
19233 to 19726), the sums -11 %, -19 % and -22 %. Glasgow comes to the
designer's count (829, 843, 953 against 815); MegaDrive closes about
half its gap to the designer's (9453, 10382, 14325 to 8136, 9042,
9952 against 7931); 4in1-mini and 4in1 go well below it. Where the
count fell most the open connections did not always follow (4in1:
crossings halved, open equal), so the crossings are one cost among
several, as the ablation's Spearman of 0.32 on the 4in1 said.

**Short tier** (one pair, seed 1, 3 x 2, the same binaries): KiCad
unconnected 18 to 26, strict passes 6 to 7 (eternal-keypad gains its
pass, none lost), first-route open terminals 23 to 48. The losses are
WLED_Matrix 4 to 8, MIDAS-MK1.1 7 to 10 and OpenFC 1 to 2; the
crossings fall on all 12 boards (sum 2823 to 2105).

| Board | KiCad unconnected off / on | strict pass | first route open | crossings (designer's) off / on |
| --- | ---: | --- | ---: | ---: |
| eternal-keypad | 0 / 0 | no / yes | 0 / 0 | 129 / 79 (181) |
| MIDAS-MK1.1-revA | 7 / 10 | no / no | 7 / 20 | 661 / 533 (376) |
| WLED_Matrix | 4 / 8 | no / no | 7 / 15 | 334 / 236 (215) |
| USB_Keypad | 2 / 2 | no / no | 2 / 2 | 238 / 176 (150) |
| reCamera_S101_v1.1 | 4 / 4 | no / no | 5 / 6 | 90 / 56 (137) |
| 1337-v4.0 | 0 / 0 | yes / yes | 0 / 0 | 156 / 120 (79) |
| spell_tome_bottom | 0 / 0 | yes / yes | 0 / 0 | 5 / 3 (22) |
| OpenFC | 1 / 2 | no / no | 1 / 2 | 534 / 350 (266) |
| urchin | 0 / 0 | yes / yes | 0 / 0 | 114 / 106 (123) |
| Blue_Line | 0 / 0 | yes / yes | 0 / 0 | 5 / 0 (3) |
| USB_Cable_Tester | 0 / 0 | yes / yes | 0 / 0 | 243 / 184 (345) |
| framework_mobo_lefthalf | 0 / 0 | yes / yes | 1 / 3 | 314 / 262 (308) |
| sum | 18 / 26 | 6 / 7 | 23 / 48 | 2823 / 2105 (2205) |

### Judgement

By the rule agreed for this lever (the dense sum of medians falls, each
pair's sum falls, the short tier loses no strict pass, and the medians
are not worse on MegaDrive and Glasgow) the default is on since
2026-10-24: `ratsnest_crossings` absent means the defaults above,
`{"ratsnest_crossings": null}` the placement before it. Caveats: the
short tier's unconnected sum rose (18 to 26, three boards by one to
four), the dense spread per board is as large as any perturbation gives
(three boards worse by the median), and the term costs about a fifth
more placement work. Next: the global placement and the anneal do not
see the crossings yet (they make the orders the legaliser and the
refinement can only repair locally; MegaDrive's U4/U8 order is such a
case; tried 2026-10-25, below: the anneal's term stays off), and a
weight per board (by layers: a crossing costs less with more of them)
is untried.

### Where the order is decided: the anneal and the global placement (2026-10-25)

The legaliser and the refinement repair orders locally; the anneal and
the global placement make them. Two sub-switches of
`ratsnest_crossings`, off by default (the judgement below):

- **`anneal`**: every anneal move (shift, quarter turn, swap) pays
  `weight` mm per crossing it adds to the moved parts' nets and earns
  as much per crossing it removes (the anneal takes uphill moves, so
  unlike the legaliser's the term is signed). Only nets of up to 16
  pins count there (`anneal::ANNEAL_NET_PINS`; the legaliser and the
  refinement count all). The anneal judges 1.5 to 2.9 million moves a
  placement, so the count works in three steps
  (`CrossingState::before`, `after`, `settle`): `before` counts the
  moved nets' kept lines (in the grid already, no tree built); since a
  move can at best remove all of those, it bounds what the move can
  save, and when even that leaves the move rejected by the random
  number the plain anneal would draw, no new tree is built (exact: the
  same number is drawn and the same decision falls). Otherwise `after`
  builds the new trees and counts them (up to 48 new lines against each
  other directly, outside the grid, so that a rejected move never
  touches the grid's lists), and `settle` keeps or drops them. Half of
  a part's swaps trade it with a peer: a part on the same side of the
  same footprint (pin count and body) or, for parts of 16 pins or more,
  one with at least half its pins (QFP-160 and QFP-128); a random
  partner of a QFP is nearly always a passive, which the overlap
  rejects at once. Tested: the three steps give `involving`'s change on
  300 random moves of two random boards (one whose moves have more than
  48 lines), and the anneal uncrosses a two-net bus.
- **`global`**: after the global placement (`crossings::reorder`) the
  movable parts of 16 pins or more trade places (body centre to body
  centre, angles kept) and turn where that lowers their nets' weighted
  half-perimeter wirelength plus `weight` times their crossings, in at
  most four first-improvement passes; a trade that breaks a part's
  constraint or relation is not taken. Cheap (under 0.1 s for
  MegaDrive's 10 such parts) and without effect where it counts: on
  MegaDrive it traded or turned parts in two of three seeds' global
  placements (crossings 18023 to 15795, 15441 to 14823), but the seed
  the placement keeps had nothing to trade, and the placed board came
  out the same (`place-kicad-board`, seed 1: 8135 crossings without
  either switch, 8135 with `global`, 6831 with the uncapped `anneal`,
  6831 with both); on Glasgow (3 such parts) it found nothing. Not
  measured in layouts; it stays off.

**The anneal's work per move** (`PCB_PLACER_DEBUG`'s `anneal:` line,
`place-kicad-board` at the default budget, three seeds; placer units,
the reference's 150e6 a second; the main anneal of each placement):

| board | moves | off (x1817) | every net, to the end (x14310) | nets of up to 16 pins, to the end (x14311) | up to 16 pins, share 0.5 (x14312) | of it the term (x14312) | judged with the term | rejected by the bound | taken | crossings when the term ends (x14312) |
| --- | ---: | ---: | ---: | ---: | ---: | ---: | ---: | ---: | ---: | --- |
| MegaDrive | 1587200 | 7101-7367 | 14982-15831 | 10935-11626 | 10309-10560 | 3453-3545 | 1.13-1.25 M | 0.54-0.63 M | 49-68 k | 16883 -> 7010, 14358 -> 7720, 15456 -> 5965 |
| Glasgow (nine anneals) | 1516800 | 3818-3861 | 6308-6754 | 4767-4971 | 4767-4971 | 962-1200 | 1.42-1.44 M | 0.65-0.93 M | 147-309 k | 2422 -> 656, 2629 -> 976, 2397 -> 475 |
| video | 1068800 | 2779-2857 | | 15372-17482 | 4104-4316 | 1420-1489 | 84-90 k | 37-41 k | 5 k | 36206 -> 8823, 23891 -> 6948, 30651 -> 9889 |
| split72_left | 2892800 | 3619-3622 | 49751-50491, cut at 1.8 M moves | 4368-4431 | as x14311 | 755-813 | 2.74 M | 1.77-1.81 M | 199-205 k | 2605 -> 1391, 3552 -> 1289, 2864 -> 1829 |

Two things set the cost. **The lines are long and meet many**:
MegaDrive's 826 tree lines average 51 mm, and the CPU goes into
gathering and testing the lines a moved line meets, not into the trees
(MegaDrive, one seed alone, every net: of 57 s in the term 4.4 s built
trees). A finer grid makes it worse (cells of 4.4, 2.2 and 1.1 mm
against `new`'s 10.5 mm: the anneal's CPU 98, 115 and 159 s against
96 s), reusing the buffers changed nothing. **Large nets dominate**:
with every net counted, a move of any part on split72_left's VDD (165
pins), VSUP (110) or a matrix row (36 to 41) builds that whole tree
again and walks its lines: 46000 units a move in the term against the
anneal's own 3600; the anneal used the placement's whole budget (600 s
of work a seed) in 1.8 of its 2.9 million moves, two parts found no
place, and the layout stopped there (the first on-run of the
measurement, x14310, set aside in `build/cross2-dense-s1-on-x14310-aborted`).
Hence the cap at 16 pins: the orders the term is for are buses of few
pins, while power nets and matrix rows spread over the board whatever
the order (above the cap besides GND: MegaDrive's VCCQ, 54 pins, and
VCC, 38; Glasgow's +3V3, VIOA, VIOB and +5V; each counts only where no
pour carries it).

The work booked is honest against the CPU: the count books a unit per
cell walked and per line met, and against the anneal's own work its
queries took 2.5 to 2.7 times the CPU time per unit booked (MegaDrive,
every net: the anneal's CPU 47 s off, 121 s on, its booked work 75 to
106 s), so the anneal books the term's work at 13/5 of the count's
(`anneal::CROSSING_WORK`; since the count's own booking was corrected,
8/5 of its new booking: "The crossings' work booking" below). With the cap the anneal's booked work per CPU
second is that of the plain anneal (1.35 to 1.58 on the three boards,
off 1.43 to 1.60).

**The term's share of the work.** Where lines are dense the term
costs several times the anneal even with the cap (video: 12700 to 14700
units a move in the term against the anneal's 2800), and that anneal
placed worse than without the term: the board after the legaliser had
21361 crossings against 19728 off. An anneal that pays for crossings to
its end resists the moves that resolve its last overlaps, and the
legaliser resolves them by moving parts far (video's annealed
wirelength 26592 mm became 30531 legalised, against 22843 to 25005 off).
So the term ends once its work reaches half the anneal's own, projected
over all stages (`anneal::TERM_SHARE`, 0.5); the anneal goes on without
it. The variants on the placed boards (`place-kicad-board`, seed 1, the
placement the three seeds keep; crossings and unweighted HPWL by
`ablate-kicad-placement`; the stages are where the term ended):

| board | off | the term to the end (x14311) | the first half of the stages | half the stages, share 0.5 | a quarter, share 0.5 | share 1 | share 0.5 (x14312) | share 0.25 |
| --- | ---: | ---: | ---: | ---: | ---: | ---: | ---: | ---: |
| MegaDrive crossings | 8135 | 6627 | | 7460 | 7123 | | 7018 (119-132 of 160) | 7493 (57-62) |
| Glasgow | 816 | 631 | | 915 | 816 | | 631 (all 160) | |
| video | 19728 | 21361 | 18414 | 14280 | | 17828 (26-28) | 14280 (13-14) | 15992 (6-7) |
| MegaDrive wirelength mm | 30116 | 28560 | | 28926 | 27728 | | 27972 | 28198 |
| Glasgow | 3148 | 3655 | | 3416 | 3343 | | 3655 | |
| video | 42530 | 43922 | 38665 | 38936 | | 44314 | 38936 | 42735 |

The designer's: MegaDrive 7931 crossings and 29390 mm, Glasgow 815 and
3641 mm, video 8765 crossings. One seed of each, so a few hundred
crossings either way is the seed's luck; what the table shows is that
on video the term must end early and that ending it early elsewhere
gives some of the gain back. On split72_left (term to the end) 3787
crossings off, 3317 on (the designer's 3201).

MegaDrive's QFP order did not change: in x1817's placement U4 sits
above U8 ((171, 91) and (184, 171)), not beside it as in v20, and so it
does with the term ((177, 107) and (184, 161)); neither the peers'
swaps nor `reorder` find the trade worth it there. The crossings fall
through the small parts.

#### Measurement

The protocol of the crossings' own measurement: the eleven dense boards
in layout mode at the task budgets (tasks v7, pinned rates), three seed
pairs (`--placer '{"seed": N}'`, N = 1, 101, 201, and on the on side
`"ratsnest_crossings": {"anneal": true}`), off x1817 (main 356c8db), on
x14312 (branch crossings-anneal 3707bf0: the sub-switches off by
default), 3 x 2 with at most six layouts in flight, through the memory
ledger with 10 GB kept free, beside other agents' runs and the nightly
of 2026-10-25; no row cut by the wall clock. Output
`build/cross2-dense-s{1,101,201}-{off,on}`, `build/cross2-short-{off,on}`.
`global` stays off on both sides (above). With the switches off,
x14312 places MegaDrive and Glasgow byte-identically to x1817
(`place-kicad-board`).

KiCad unconnected per board, off / on:

| Board | seed 1 | seed 101 | seed 201 | median |
| --- | ---: | ---: | ---: | ---: |
| OpenESC 4in1-mini | 81 / 91 | 82 / 70 | 99 / 89 | 82 / 89 |
| OpenESC 4in1 | 139 / 68 | 92 / 60 | 93 / 122 | 93 / 68 |
| DSP-ADAU1452 | 17 / 6 | 7 / 4 | 25 / 10 | 17 / 6 |
| split72_left | 23 / 5 | 21 / 26 | 33 / 36 | 23 / 26 |
| video | 190 / 262 | 145 / 83 | 159 / 338 | 159 / 262 |
| Glasgow | 145 / 127 | 163 / 122 | 175 / 147 | 163 / 127 |
| SmartSpin2k | 34 / 49 | 59 / 59 | 52 / 80 | 52 / 59 |
| 0xCB panel | 50 / 54 | 50 / 53 | 51 / 47 | 50 / 53 |
| protoesp | 105 / 76 | 76 / 88 | 90 / 77 | 90 / 77 |
| MegaDrive | 110 / 105 | 108 / 114 | 168 / 88 | 110 / 105 |
| SNSP-CPU-01 | 53 / 55 | 95 / 103 | 87 / 37 | 87 / 55 |
| sum | 947 / 898 | 898 / 782 | 1032 / 1071 | 926 / 927 |

The summed medians stay where they were (926 to 927). Two pairs fall
(seed 1: 947 to 898, seed 101: 898 to 782), the third rises (1032 to
1071). By the median 6 boards are better (4in1 93 to 68, DSP 17 to 6,
Glasgow 163 to 127, protoesp 90 to 77, MegaDrive 110 to 105, SNSP-CPU-01
87 to 55) and 5 worse, video by most (159 to 262: 190 to 262, 145 to 83
and 159 to 338, the widest swing of any board). The router's open
terminals after the first route: 1416 to 1253, 1280 to 1161, 1388 to
1472; vias 10189 to 9155, 9512 to 9736, 9570 to 9732. The placement's
work (board-placer.json's `work_seconds`, summed over the boards) 3265
to 2974, 3334 to 3084, 3174 to 3152 s: not more, as the anneal's term
ends on the dense boards where it would cost most, and on video the
placement no longer runs into its budget (608 s off, 265 and 358 s on
two seeds).

Ratsnest crossings of the placed boards (`layout/placed`, counted by
`ablate-kicad-placement`, x1817), off / on:

| Board | designer's | seed 1 | seed 101 | seed 201 |
| --- | ---: | ---: | ---: | ---: |
| OpenESC 4in1-mini | 279 | 80 / 79 | 84 / 75 | 91 / 55 |
| OpenESC 4in1 | 242 | 376 / 474 | 543 / 408 | 488 / 293 |
| DSP-ADAU1452 | 170 | 744 / 660 | 710 / 664 | 720 / 628 |
| split72_left | 3201 | 3639 / 3471 | 3745 / 3388 | 3853 / 3726 |
| video | 8765 | 19726 / 15160 | 16987 / 11126 | 14018 / 26030 |
| Glasgow | 815 | 829 / 631 | 843 / 702 | 953 / 773 |
| SmartSpin2k | 1410 | 3149 / 3017 | 3348 / 3101 | 3301 / 3181 |
| 0xCB panel | 319 | 460 / 523 | 493 / 491 | 480 / 530 |
| protoesp | 373 | 1102 / 844 | 875 / 1273 | 1046 / 1003 |
| MegaDrive | 7931 | 8136 / 7021 | 9042 / 9257 | 9952 / 7837 |
| SNSP-CPU-01 | 5076 | 4286 / 4219 | 4765 / 4343 | 4336 / 4196 |
| sum | 28581 | 42527 / 36099 | 41435 / 34828 | 39238 / 48252 |

Fewer crossings on 27 of the 33 pairs; the sums fall 15 % and 16 % on
two seeds and rise 23 % on the third, all of it video's seed 201
(14018 to 26030; without video the three sums fall 3 to 12 %). Glasgow
goes below the designer's count on every seed (631, 702, 773 against
815), MegaDrive on two (7021 and 7837 against 7931; 9257 on seed 101).
Where the crossings fell the open connections mostly followed, but not
always (protoesp seed 101: 875 to 1273 crossings, 76 to 88 open;
SmartSpin2k: fewer crossings on all seeds, more open on two).

**Short tier** (one pair, the tier's seed, 3 x 2, the same binaries):
KiCad unconnected 26 to 36, strict passes 7 to 6 (framework_mobo_lefthalf
loses its pass with one open connection), first-route open terminals
48 to 58. The losses are WLED_Matrix 8 to 14, MIDAS-MK1.1 10 to 13 and
framework_mobo_lefthalf 0 to 1; no board gains.

The intermediate x14311 (every stage, nets of up to 16 pins) ran seed 1
once: 947 to 1010, video 190 to 319 with 21361 crossings
(`build/cross2-dense-s1-on-x14311`); that is what the share fixed.

#### Judgement

By the rule agreed for this step (the dense sum of medians falls, each
pair's sum falls, the short tier loses no strict pass and its
unconnected does not rise, MegaDrive's median improves) the anneal's
term stays off: the summed medians are flat (926 to 927), seed 201's
sum rises, the short tier loses a pass and 10 connections, and
MegaDrive's 110 to 105 is within one route's noise there (sd 12.7).
`global` stays off (no effect on the placed boards). What the step
showed: the anneal can take the crossings below the designer's counts
on MegaDrive and Glasgow at a bounded cost (the term's work at most
half the anneal's), but fewer crossings alone do not route better on
this set, and video's placement swings by a factor of two in crossings
and open connections from seed to seed with the term. MegaDrive's QFP
order, the finding this was for, is not what the anneal changes: in
our placements U4 and U8 lie one above the other, and no trade of the
two pays.

### The crossings' work booking (2026-10-25)

Every placement decision is taken in work, so the count has to book
what it costs: a unit of its work should take the CPU of a unit of the
rest of the placement. Until 2026-10-25 it booked a unit per cell a
query walked and per line it met, a tree's pins and Prim's k² steps,
and nothing for keeping the grid (inserting a new tree's lines,
removing them again, the search in a cell's list for the line to
remove). Under-booked, the term made the placement run longer than its
budget said (split72_left spends two fifths of its placement in the
count), the layout's wall guard came nearer, and work-based comparisons
between binaries were skewed.

**The measure** (`PCB_PLACER_CROSSINGS_PROFILE` with
`PCB_PLACER_DEBUG`; `ops` times each operation too): per placer phase a
line `placer crossings <phase>` with the wall time inside the count's
calls, the work they booked and what they did (trees, pins, Prim's
steps, cells walked, lines met and tested, cells walked to insert and
remove, list entries scanned), beside the phase's wall time and work.
The factor is the count's wall time per booked unit against the rest of
the placement's (all phases of the run, the count's time and work taken
out): the same thread, the same load, so it is the CPU ratio. Runs:
`place-kicad-board` on the dense tier's task inputs (3 seeds, the
default 600 s of work, 2 threads, `PCB_WORK_RATE=3.15e6,150e6`), the
legaliser = legalize, legalize first, legalize keeping, evict and
legalize on marks, the refinement = refine and repair and align;
`build/book1-prof/p1` (x14340), `p2` (x14341, every operation timed),
`p3` (x14342, the new booking), `p3a` (the same with
`ratsnest_crossings.anneal`).

CPU per booked unit against the rest of the placement:

| Board | legaliser, before (two runs) | refinement, before | legaliser, after (two runs) | refinement, after | count's share of the placement's wall |
| --- | ---: | ---: | ---: | ---: | ---: |
| MegaDrive | 3.04, 3.14 | 2.51, 2.64 | 1.01, 1.14 | 0.96, 0.95 | 7 % |
| Glasgow | 2.18, 2.56 | 1.48, 1.79 | 0.84, 0.85 | 0.61, 0.77 | 10 % |
| video | 3.65, 3.40 | 3.62, 2.99 | 1.17 | 1.22 | 5 % |
| split72_left | 1.78, 1.73 | 1.49, 1.69 | 1.12 | 0.92 | 39 % |

(before: x14340 and x14341; after: x14342 default and with the anneal's
term; the share from x14340.) One factor would not have done: the old
booking was 1.5 times short on split72_left, whose cost is mostly
Prim's trees (its matrix nets) and booked about right, and 3.6 times on
video, whose cost is the queries and the grid's lists (long tree lines
through crowded cells). Timing each operation (x14341) gave, in units
of ordinary placement work per counted operation: a tree's pins and
Prim's steps 0.55 to 0.96 of what they book (kept), a query 1.5 to 3.0
per line met (cells walked are a few percent of it), an insert 3.1 to
5.0 per cell walked, a remove 4.6 to 6.6 per cell walked or 0.4 to 0.9
per list entry scanned (the search goes 7 to 16 entries deep on average). The count now
books (`crossings::MET_WORK`, `GRID_CELL_WORK`, `SCAN_WORK`) 5/2 a line
met, 7/2 a cell walked to insert or remove a line, 1/4 a list entry
scanned, and as before 1 a cell a query walks, `PIN_WORK` a pin and 1 a
Prim step. After it the factor is 0.6 to 1.2 by board and phase
(Glasgow's operations cost less than the others'; why is not measured); the counts are exact as before (the unit tests
unchanged, and the boards `place-kicad-board` writes at its default
budget are byte-identical before and after on all four boards; two of
Glasgow's other seeds end elsewhere, 850 to 848 and 679 to 672
crossings: what changes is where a budget stops, not what a step does).

The anneal's term (off) books 8/5 of the count's new booking
(`anneal::CROSSING_WORK`, 13/5 of the old): with the new booking its
queries took 1.60 (MegaDrive) and 1.57 (Glasgow) times the CPU per unit
against the anneal's own work (the kept lines counted again, the new
lines among themselves directly, most never entering the grid).

**`place-kicad-board` side by side** (`build/book1-prof/pair1`, x1818
against x14343, all eight at once, 2 threads each, the shell's CPU time
of the whole process against the seeds' summed work): CPU seconds per
second of work MegaDrive 1.027 to 0.924, Glasgow 0.973 to 0.900, video
0.862 to 0.813, split72_left 1.107 to 1.080; all four write the same
board. Glasgow's seeds stop at the budget (1801 s of work either way),
so the honest booking is 130 s less CPU there; on the others the same
steps book more (MegaDrive 920 to 956 s, split72_left 654 to 736 s).

**Layouts.** The crossings default's protocol: the eleven dense boards
in layout mode at the task budgets (tasks v7, pinned rates), seeds 1,
101 and 201 (`--placer '{"seed": N}'`), off x1818 (main ab86b54), on
x14343 (branch crossings-booking: ab86b54 and the booking), 3 x 2 with
at most six layouts in flight, through the memory ledger with 10 GB
kept free, beside the nightly; no row cut by the wall clock. Output
`build/book1-dense-s{1,101,201}-{off,on}`, `build/book1-short-{off,on}`.
The off side reproduces x1817's pairs of the anneal measurement
exactly (947, 898, 1032: the same placements, pinned rates).

KiCad unconnected per board, off / on:

| Board | seed 1 | seed 101 | seed 201 | median |
| --- | ---: | ---: | ---: | ---: |
| OpenESC 4in1-mini | 81 / 81 | 82 / 82 | 99 / 99 | 82 / 82 |
| OpenESC 4in1 | 139 / 125 | 92 / 92 | 93 / 93 | 93 / 93 |
| DSP-ADAU1452 | 17 / 17 | 7 / 7 | 25 / 25 | 17 / 17 |
| split72_left | 23 / 23 | 21 / 21 | 33 / 33 | 23 / 23 |
| video | 190 / 190 | 145 / 145 | 159 / 159 | 159 / 159 |
| Glasgow | 145 / 145 | 163 / 163 | 175 / 157 | 163 / 157 |
| SmartSpin2k | 34 / 34 | 59 / 59 | 52 / 52 | 52 / 52 |
| 0xCB panel | 50 / 50 | 50 / 50 | 51 / 51 | 50 / 50 |
| protoesp | 105 / 105 | 76 / 76 | 90 / 90 | 90 / 90 |
| MegaDrive | 110 / 110 | 108 / 108 | 168 / 168 | 110 / 110 |
| SNSP-CPU-01 | 53 / 53 | 95 / 95 | 87 / 87 | 87 / 87 |
| sum | 947 / 933 | 898 / 898 | 1032 / 1014 | 926 / 920 |

Of the 33 placements 31 are byte-identical; 4in1's seed 1 and Glasgow's
seed 201 differ (the race keeps another seed: where its budget ends
moved), and both route better (139 to 125, 175 to 157). The summed
medians fall 926 to 920, the pairs 947 to 933, 898 to 898 and 1032 to
1014. The first route's open terminals 1416 to 1421, 1280 to 1280, 1388
to 1390.

Placement (board-placer.json of `layout/placed`, the three seeds
summed): the race's seeds' work and the placement's wall clock, off /
on:

| Board | placement work off / on (s, 3 seeds summed) | placement wall off / on (s) | wall per work off / on |
| --- | ---: | ---: | ---: |
| OpenESC 4in1-mini | 2018 / 2054 | 1498 / 874 | 0.742 / 0.425 |
| OpenESC 4in1 | 5449 / 5463 | 2735 / 2230 | 0.502 / 0.408 |
| DSP-ADAU1452 | 1889 / 1871 | 759 / 613 | 0.402 / 0.327 |
| split72_left | 2030 / 2271 | 1381 / 1045 | 0.680 / 0.460 |
| video | 4109 / 4148 | 1563 / 1113 | 0.380 / 0.268 |
| Glasgow | 5106 / 5196 | 2198 / 2099 | 0.431 / 0.404 |
| SmartSpin2k | 2910 / 3136 | 1369 / 1347 | 0.470 / 0.430 |
| 0xCB panel | 1871 / 1896 | 720 / 735 | 0.385 / 0.388 |
| protoesp | 1857 / 1865 | 687 / 670 | 0.370 / 0.359 |
| MegaDrive | 2585 / 2690 | 907 / 873 | 0.351 / 0.325 |
| SNSP-CPU-01 | 1924 / 1911 | 785 / 714 | 0.408 / 0.374 |
| sum | 31748 / 32502 | 14601 / 12314 | 0.460 / 0.379 |

The booked work rises 2.4 % (split72_left 12 %, SmartSpin2k 8 %,
MegaDrive 4 %: the boards whose count is large), and the placement's
wall clock per second of work falls 0.460 to 0.379. The load is not the
same on both sides there: seed 201's pair ran side by side (both at a
mean load of 9.6) and fell 0.438 to 0.393 (-10 %); seeds 1 and 101 fell
0.472 to 0.366 and 0.469 to 0.378, but their off sides shared the
machine with the profiling runs above. Where the crossings count is a
large share the fall is largest (split72_left 0.680 to 0.460 over the
three seeds, 4in1-mini 0.742 to 0.425, the latter partly the load).

**Short tier** (one pair, the tier's seed, 3 x 2, the same binaries):
KiCad unconnected 26 to 24, strict passes 7 and 7, first-route open
terminals 48 to 38; the one board that changes is MIDAS-MK1.1 (10 to 8
unconnected, 20 to 10 open after the first route), the other eleven
are equal.

**Judgement.** The booking is a correctness fix of the work clock and
ships: a unit of the count's work now costs 0.6 to 1.2 units of the
rest of the placement's CPU (1.5 to 3.7 before). The default placements
barely move (two of 33 dense placements, both better; the dense sum of
medians 926 to 920), so no weight adjustment is needed: no phase lost
reach that the layouts show.

## The race's route probe (2026-10-25)

`race_probe` (layout option, off by default; `{}` for the defaults
`{"seeds": 3, "share": 0.08}`; code `board_layout.rs`, the standing
`Router::probe_standing`). Layout results swing with the placer's seed
far more than with any lever of the week; the question was whether the
race picks well among the seeds it has.

**The ceiling, from the data at hand** (no runs): the eleven dense
boards at the task budgets, KiCad unconnected per layout seed (each
layout seed is its own set of placer seeds), main's x1818
(`build/book1-dense-s{1,101,201}-off`; x1817's
`cross2-dense-*-off` are the same rows). The pick columns ask which
run a selector would have chosen among the three, from what each run
recorded, and by how much it missed the best (`python3 -I
experiments/race-probe/ceiling.py`: board-layout.json's
`placement_race`, results.json's first-route open terminals,
cross2-crossings.json; "75 s probe" is the kept placement's race
probe, unfinished nets):

| Board | seed 1 | seed 101 | seed 201 | median | min | max | spread | 75 s probe picks | first route picks | crossings pick |
| --- | ---: | ---: | ---: | ---: | ---: | ---: | ---: | --- | --- | --- |
| OpenESC 4in1-mini | 81 | 82 | 99 | 82 | 81 | 99 | 18 | yes (s1, +0) | no (s101, +1) | yes (s1, +0) |
| OpenESC 4in1 | 139 | 92 | 93 | 93 | 92 | 139 | 47 | yes (s101, +0) | no (s201, +1) | no (s1, +47) |
| DSP-ADAU1452 | 17 | 7 | 25 | 17 | 7 | 25 | 18 | no (s1, +10) | yes (s101, +0) | yes (s101, +0) |
| split72_left | 23 | 21 | 33 | 23 | 21 | 33 | 12 | yes (s101, +0) | no (s1, +2) | no (s1, +2) |
| video | 190 | 145 | 159 | 159 | 145 | 190 | 45 | - | no (s201, +14) | no (s201, +14) |
| Glasgow | 145 | 163 | 175 | 163 | 145 | 175 | 30 | yes (s1, +0) | yes (s1, +0) | yes (s1, +0) |
| SmartSpin2k | 34 | 59 | 52 | 52 | 34 | 59 | 25 | no (s101, +25) | yes (s1, +0) | yes (s1, +0) |
| 0xCB panel | 50 | 50 | 51 | 50 | 50 | 51 | 1 | yes (s1, +0) | yes (s101, +0) | yes (s1, +0) |
| protoesp | 105 | 76 | 90 | 90 | 76 | 105 | 29 | yes (s101, +0) | yes (s101, +0) | yes (s101, +0) |
| MegaDrive | 110 | 108 | 168 | 110 | 108 | 168 | 60 | no (s1, +2) | yes (s101, +0) | no (s1, +2) |
| SNSP-CPU-01 | 53 | 95 | 87 | 87 | 53 | 95 | 42 | no (s101, +42) | yes (s1, +0) | yes (s1, +0) |
| sum | 947 | 898 | 1032 | 926 | 812 | 1139 | | probe 6/11 regret 79 | first 7/11 regret 18 | cross 7/11 regret 65 |

- The oracle (the best of three layout seeds) is 812 against the
  summed medians' 926 (-12 %; x1819, which differs from x1818 on two
  rows, 4in1 seed 1 125 and Glasgow seed 201 157: 920 and 812); over all nine runs of the three variants
  (main, the booking x14343, the anneal's term x14312) 626.
- **The race already probes.** Since 2026-10-10 the race probes its
  three best-ranked placements with the router for `probe_seconds`
  (75 s of work) each and routes on with the fewest unfinished nets;
  RUDY and the crossings only choose which three, and only where the
  placer kept more than three seeds (board-layout.json's
  `congestion.probed`: 0xCB panel, protoesp, DSP-ADAU1452 and
  SNSP-CPU-01 on every seed, 4in1-mini on two, split72_left on one).
  On the boards whose placement uses its whole 40 % (Glasgow, 4in1,
  video's seed 201) the work gate (`work > 0.4 * total_seconds`)
  discards the other probes after they ran: those boards route the
  placer's own pick (video's seeds 1 and 101 have no other placement
  to race).
- **Across runs, the first route ranks better than the race's probe.**
  The kept placement's race probe (at most 75 s) picks the best run on 6 of 11 boards (regret 79
  connections summed), the full first route's open terminals on 7 of
  11 (regret 18), the ratsnest crossings on 7 (regret 65). On the
  anneal's runs: 7, 9 and 4 (regrets 58, 29, 136). The first route is
  several hundred seconds of work, its repair included; the probe only
  negotiates.

**The design.** With `race_probe` the race probes the `seeds` best
placements of its ranking (the pre-ranking by RUDY and crossings as
before where there are more; the first `seeds` in the placer's order
where there are not), each for `share` times `total_seconds` of work
(120 s at the task budget of 1500 s), all of them charged to the
layout's work clock: the 40 % gate is gone, and the final route's
budget is what the clock has left. The kept probe's router routes on
(its probe is the first part of the first route, as before). The
probes are ranked by their standing, compared in order: the open
terminals (`board_open_terminals`, plus one for every complete net
still in conflict and every pour pad its pour does not reach), the
unfinished nets (the race's measure until now), the nets in conflict;
a tie to the better-ranked placement. Deterministic: each probe is
work-bounded and judged in the ranking's order whatever finishes first.
board-layout.json and the verdict carry `race`: every probed
placement (its index among the ranked ones, 0 the placer's kept one),
its standing and work, the one kept and the probe's work.

### Measurement

The dense pairs' protocol: the eleven dense boards in layout mode at
the task budgets (tasks v7, pinned rates), `--placer '{"seed": N}'`,
3 x 2 with at most six layouts in flight, through the memory ledger
with 10 GB kept free, beside the nightly and other agents' runs; no row
cut by the wall clock. Off: x14343's rows (`build/book1-dense-s*-on`:
branch crossings-booking 5a12f57, whose code is main c7b5552's, x1819;
the later commits are docs). On: x14350 (branch race-probe ed36a79),
`--layout '{"race_probe": {}}'`, `build/race1-dense-s{1,101,201}-on`.
With the switch off x14350 writes byte-identical boards to x14343 on
the 0xCB panel and DSP-ADAU1452 (seed 1, `build/race1-ident-x14350`).
Seeds 1 and 101 were the plan; seed 201 was added when the first two
fell by about one seed-noise standard deviation.

KiCad unconnected per board, off / on:

| Board | seed 1 | seed 101 | seed 201 | median (1, 101) | median (all three) |
| --- | ---: | ---: | ---: | ---: | ---: |
| OpenESC 4in1-mini | 81 / 80 | 82 / 82 | 99 / 99 | 81.5 / 81 | 82 / 82 |
| OpenESC 4in1 | 125 / 82 | 92 / 96 | 93 / 79 | 108.5 / 89 | 93 / 82 |
| DSP-ADAU1452 | 17 / 17 | 7 / 19 | 25 / 25 | 12 / 18 | 17 / 19 |
| split72_left | 23 / 22 | 21 / 14 | 33 / 22 | 22 / 18 | 23 / 22 |
| video | 190 / 190 | 145 / 145 | 159 / 302 | 167.5 / 167.5 | 159 / 190 |
| Glasgow | 145 / 146 | 163 / 164 | 157 / 182 | 154 / 155 | 157 / 164 |
| SmartSpin2k | 34 / 37 | 59 / 60 | 52 / 57 | 46.5 / 48.5 | 52 / 57 |
| 0xCB panel | 50 / 50 | 50 / 50 | 51 / 51 | 50 / 50 | 50 / 50 |
| protoesp | 105 / 92 | 76 / 51 | 90 / 77 | 90.5 / 71.5 | 90 / 77 |
| MegaDrive | 110 / 102 | 108 / 110 | 168 / 186 | 109 / 106 | 110 / 110 |
| SNSP-CPU-01 | 53 / 54 | 95 / 81 | 87 / 71 | 74 / 67.5 | 87 / 71 |
| sum | 933 / 872 | 898 / 872 | 1014 / 1151 | 915.5 / 872 | 920 / 924 |

- Seeds 1 and 101: the summed medians fall 915.5 to 872 (-4.8 %), both
  pairs fall (933 to 872, 898 to 872). The seed noise of a two-seed
  median sum (x1819's per-board spread over its three seeds, the
  boards independent) is about 40; the fall is about one standard
  deviation.
- Seed 201 rises 1014 to 1151 (+137): video 159 to 302 (the probe kept
  placement 1, 456 against 488 open terminals at 120 s; its first route
  left 432 open where placement 0's left 189), Glasgow 157 to 182,
  MegaDrive 168 to 186. Over the three seeds the summed medians are
  920 / 924: flat.
- By board the gains are where the 40 % gate had discarded the better
  probe or where 120 s of probing separates the seeds: 4in1 (125 to
  82, 93 to 79), protoesp (105 to 92, 76 to 51, 90 to 77),
  split72_left (21 to 14, 33 to 22), SNSP-CPU-01 (95 to 81, 87 to 71).
  The losses are the boards whose placement takes its 40 % and whose
  probes end deep in conflict (Glasgow 169-182 nets in conflict, video
  291 and 316 after 120 s): there the probes do not tell the seeds apart,
  and the other probes' work (150 to 200 s on Glasgow, 120 s on video)
  comes out of the final route (Glasgow is worse on all three seeds,
  by 1, 1 and 25).
- **Pick accuracy** (the verdict's `race.probe`): of the 31 races, 15
  kept another placement than off; on the 16 seed-boards where the
  finals of two placements or more are known (off's, on's and the
  repair variant's runs) the probe kept the best known on 10. The open
  terminals and the unfinished nets pick differently in 4 of the 31
  races: twice the unfinished nets' pick was the better one
  (DSP-ADAU1452 seed 101: placement 2 ends at 7 open, the open
  terminals' placement 1 at 19; Glasgow seed 201: 157 against 182),
  once the open terminals' (protoesp seed 101, a tie on both broken by
  the conflicts: 51 against 76), once alike (4in1-mini seed 1, 80
  against 81).
- **Half the boards' probes stop early.** With the first route's fast settings
  (stall patience 8, 10 at the price cap) the negotiation ends when the
  conflicts stop falling: on 4in1-mini, 4in1, DSP-ADAU1452, the 0xCB
  panel and Glasgow after 9 to 108 s, so the 8 % share is a ceiling
  there (those races cost 37 to 294 s of work in all); on
  split72_left, SmartSpin2k, protoesp, MegaDrive, SNSP-CPU-01 and video
  every probe ran its 120 s. Work and
  CPU per pair (s): seed 1 14792 / 15000 work, 18208 / 19066 CPU;
  seed 101 14766 / 14598, 17684 / 18881; seed 201 15218 / 15606,
  19976 / 18893. First-route open terminals 1421 / 1148, 1280 / 1274,
  1390 / 1693.

**Short tier** (seed 1, 3 x 2, x14343 against x14350 with the probe):
KiCad unconnected 24 / 24, strict passes 7 / 7, first-route open 38 /
38: every row the same (the boards' probes end in seconds and keep the
same placements).

**Variant: a whole first route as the probe** (`race_probe.repair`,
x14351, seed 101 only, `build/race1-dense-s101-rep`): each probe
negotiates for half its share and then runs the first route's repair
within the rest, ranked by the open terminals it leaves; the kept one's
is the layout's first route. 898 to 966 (MegaDrive 108 to 150,
SNSP-CPU-01 95 to 136, Glasgow 163 to 155): 120 s is too little for a
first route on the large boards (first-route open terminals 1280 to
2737), and the shortened first route decides too much afterwards (no
moves where many stay open). Rejected; the option stays for an
experiment with a larger share.

**Two probes at 10 % each** (`{"seeds": 2, "share": 0.1}`, x14350,
on the two boards with the largest gains, seeds 1 and 101,
`build/race1-k2-s{1,101}`), KiCad unconnected, off / three probes / two:

| Board | seed 1 | seed 101 |
| --- | ---: | ---: |
| OpenESC 4in1 | 125 / 82 / 82 | 92 / 96 / 96 |
| protoesp | 105 / 92 / 105 | 76 / 51 / 69 |
| sum | 230 / 174 / 187 | 168 / 147 / 165 |

On 4in1 only two placements race anyway (the same picks); on protoesp
the pre-ranking's best two leave out the placement the third probe
found on both seeds. Two probes are no better than three here.

### Judgement

By the rule agreed for this step (the dense sum of medians falls by
more than the seed noise, each pair's sum falls, the short tier loses
no strict pass) `race_probe` stays off: on the two planned pairs the
fall (915.5 to 872) is about one standard deviation of the noise, and
the third pair rises by 137 (video's 302 alone undoes it), so over
three seeds the medians are flat (920 / 924). The short tier is
unchanged. What the step showed:

- The 40 % gate costs something real on the boards where placement
  takes its share: 4in1's seed 1 lost 43 connections to it (125 against
  82 with the other probe counted). A gate that charges the probes but
  keeps them would be a smaller change than this one.
- The probe's pick is right about two times in three where it can be
  checked, as docs/global-route.md's validation found for the 75 s
  probe (pair accuracy 0.68-0.74); 120 s does not make it better on the
  large boards, where the probes end with 170-320 nets in conflict.
- The oracle's 12 % is not within reach of a pick among one layout's
  seeds by a short probe: much of the spread is between the layout
  seeds' placer seeds (a seed set of three), not inside one race.

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
