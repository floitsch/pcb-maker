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
   fixed; everything else may move and turn in 90° steps. Large nets get a
   smaller weight so power does not collapse the layout. Pad boxes cover
   the drill. A through-hole part blocks the other side only with its
   holes and pads. A hollow part (a shield outline) blocks only with its
   pads, and with its courtyard against parts with holes, since KiCad
   forbids holes in a courtyard unless the courtyard is malformed and
   KiCad skips it. A board cutout keeps only copper away: a connector body
   may reach over its peg holes.
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
   again with the stuck parts first. Crowded boards then walk down
   relaxation levels:
   - smaller halos, then none;
   - spacing down to the copper clearance;
   - a 0.1 mm grid;
   - bodies closer to the edge, as far as their copper allows;
   - where the project lets courtyards overlap, bodies shrunk to their
     fabrication outline and pads.

   The level used is reported, and the router-driven moves keep to it.
   Several seeds run in parallel (`placement_seeds`, 3; the layout asks for
   16, see the race below); the best placement by the placer's own key
   (fewest illegal parts, least missed constraints, least wirelength) is
   kept, the others are the race's alternatives.
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
  do, see [constraints.md](constraints.md)).
- Constraints: see [constraints.md](constraints.md). No alignment
  constraints yet.
- Congestion drives nudges of individual parts, but does not yet feed the
  global placer's density term (coupling step 3).
- Silkscreen labels are not placed; the separate repair tool handles most.
- Pin swapping runs once after placement when the layout config names a
  `pin_swaps` file ([pin-swap.md](pin-swap.md)); it is not yet part of the
  move loop. No gate swapping.
- The spectral solve is a direct O(n^3) cosine transform (64 bins); an FFT or
  a GPU kernel becomes relevant for boards with thousands of parts.
