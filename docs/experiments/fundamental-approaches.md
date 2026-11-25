# Fundamental approaches for pcb-maker (research note, 2026-11-23)

Question: instead of refining what every tool does (lattice A*, PathFinder,
annealing), which algorithms could change the quality or the speed of the
tool fundamentally? This note reads the code and the nightly of 2026-11-18,
ranks three proposals in depth, lists secondary ideas, says what was
considered and rejected, and treats evaluation separately. Nothing in the
repository was changed; no benchmark or layout was run. Two toy scripts were
run to calibrate one claim (section 7).

## 0. The short version

1. **A: an exact endgame.** Replace the sequential single-net repairs of the
   last connections by a joint, complete solve of every net in a small
   window, with an infeasibility certificate when there is none. The
   headline is decided there: 22 of the 38 failing sweep boards have at most
   7 connections open.
2. **B: an exact cut-capacity model shared by the placer and the router.**
   Room on a PCB is a step function of a gap (0, 1, 2 tracks); the placer
   models it with halos by pin count and a 0.635 mm grid. Simultaneous
   escape by max-flow, gaps rounded to track quanta, and part moves computed
   from overloaded cuts by an LP instead of tried one at a time.
3. **C: objective-guided arrangement search on the skeleton.** The skeleton
   (a median 54 parts) costs a median 22 s of a 600 s budget and is searched
   by up to 16 independent cold descents. Replica exchange over arrangements uses
   the same work to search, not to sample. Gated by one cheap census.
4. **Evaluation (section 5):** an endgame tier on stored stalled boards
   (minutes instead of layouts), coupled A/B runs through counter-based
   random numbers, a variance decomposition of placer against router noise,
   seed-free certificates as metrics.

The censuses of A and B and the evaluation items cost days and decide the
rest. Each proposal names the result that kills it.

## 1. What the evidence says, read once more

The brief says 61 % of what stays open is placement. That is true of the
count of open connections and misleading for the headline. From
`build/nightly/2026-11-18/sweep.log` and `designer.log` (my tabulation):

| | boards |
| --- | ---: |
| pass strict in both tiers | 69 |
| fail only in the sweep (placement costs the pass) | 7 |
| fail only on the designer's placement | 2 |
| fail in both | 31 |

Of the 38 sweep failures: 22 have 1 to 7 connections open (20 have at most
5), 3 have 9 to 18, 10 have 51 or more, 3 have no result. About a third of
the 22 are the input's by the ceiling census (katia, reCamera, SUMEC, Hub's
zone pieces, the keyboard's RESET, osprey's feed), so about 15 boards are
within a handful of connections of a pass and ours to close.

The placement gap in connections (+367 over 100 target boards) is five
boards (docs/benchmarks.md, v45): Batch_1 +140 and SmartSpin2k +51 (panels,
sub-board assignment), Glasgow +68, OpenESC 4in1 +67, 4in1-mini +51. The
last three are over-full: `board-placer.json` reports a front-side
utilisation of 1.19 (4in1), 1.27 (mini), 0.73 at level 7 (Glasgow), and
eurorack 1.65. On these the placement problem is packing with routing
channels, at the last relaxation levels, not the coarse arrangement.

So there are two regimes and they want different bets:

- **The headline (strict passes)** is an endgame problem: small residues,
  local, on boards whose placement is fine. `board-router.json` of the
  near-miss boards shows 2 to 7 conflicted nets on most and hot spots of 1
  to 7 nets in tiles of 16 lattice nodes, 1.6 to 2 mm (the designer tier's
  jetson: IC7's CC1 and CC2 pads, 0.4 mm apart, with 5V0_SYS and IC7's
  PORT net).
- **The open count** is a dense-packing problem on a few boards, where
  sub-grid seating decides (the designer's poses seated at our levels cost
  28 and 34 open terminals on the 4in1 and Glasgow; a two-part move of
  1.27 mm took Glasgow's first route from 68 to 20 open terminals).

Three more facts shape the proposals:

- Every endgame tried so far is the same algorithm class: one net routed at
  a price, its victims rerouted one after the other (`trouble.rs`; ejection
  chains three deep; the residue renegotiated in a window). The exchange
  "closes as many as it opens" on 134 of 217 items; the designer joins 196
  of the 217 on the same placement.
- Tile-level congestion as placement feedback failed three times (wire area
  as charge, the global route in the anneal, the global route as
  pre-ranker), and RUDY ranks seeds at 0.30. The global route tells ours
  from the designer's on 0.79 of pairs but not our seeds from each other
  (0.33). Two probes of one placement agree at 0.44: among our own seeds
  the routed outcome is mostly noise.
- The skeleton placement is cheap and under-searched: over the 81 sweep
  boards with a skeleton, a median 54 parts, 22 s of work for the skeleton
  and 16 s for the passives, against a budget of 600 s a seed.

## 2. Proposal A: an exact endgame (joint window solve with certificates)

### The idea

When a negotiation stops with a small residue, solve each hot spot as one
problem over all nets with copper in a window, by a complete method, instead
of exchanging one net at a time. Either it returns routes for all of them or
it returns a certificate: the set of nets and fixed items that cannot
coexist in the window.

Mechanics:

- *Trigger.* `negotiate_from` (router/negotiate.rs) stops `stalled` with at
  most about 12 nets in `last_conflicted`, or the hard repair
  (`resolve_remaining`, router/resolve.rs) leaves terminals open. Today
  follow `repair_trouble` (router/trouble.rs), the ladder's seed rounds,
  finer pitches and steps, and outside the router the growing family of
  special repairs (`crates/pcb-kicad-route/src/open_repair/`: local,
  plane_escape, pad_bridge, terminal_launch, zone_open, and
  `board_layout/additive_repair.rs`, `via_detour.rs`).
- *Window.* A hot spot's tile (`diagnostics.hot_spots`, 1.6 mm) grown by 1
  to 2 mm, all layers. Nets wholly outside stay fixed. A net crossing the
  edge keeps its outside copper; its crossing nodes become terminals for
  the solve. A supply net routed as tracks is a net like any other inside
  the window (this is where the census found our supply copper across the
  designer's path on 114 of 199 open nets).
- *Model.* The router's own lattice, static maps and stamps
  (`StaticMaps`, `occupancy.rs`), so a solution is as legal as any route of
  the router and goes through `verify` and KiCad as today. Three
  formulations, in the order my toy runs support them (section 7):
  1. cell-level SAT with a detour budget per net (a net may only use nodes
     on paths at most `d` longer than its shortest), the incumbent routes as
     phase hints, and one cardinality constraint per narrow cut;
  2. path-based: columns are routes (the incumbent, the trouble-maker's
     priced routes, routes generated against prices), an exact set-packing
     master, real branching;
  3. conflict-based search with the existing hard search as the low level.
- *Certificate.* On infeasibility the core over assumption literals (one
  per net kept fixed at the edge, one per movable part's obstacles) names
  what to release: grow the window towards the nets in the core, hand the
  parts in the core to the placement repair (proposal B, step 2), or print
  it in the verdict ("U3 and C7 leave one track's room for three nets").
- *Crossing pairs at fine-pitch rows* (handover problem 4: "the crossing
  has to be decided inside the negotiation") are the same solve over the
  row's window: which pad leaves on which layer, and where the vias land,
  is decided jointly.

### Why it could change the basin

PathFinder is priced best response; it has no completeness. Every repair
after it has been a deeper or wider version of the same sequential move, and
each was measured and removed or left even. A complete method on a small
neighbourhood is a different class: in the terms of the OR literature the
project has done large-neighbourhood search with heuristic repair operators
and has not tried an exact one. The residue is exactly the size where exact
methods work (2 to 7 nets a hot spot) and where the heuristics cycle ("one
or two nets cycling at the price cap"; Sisu spent 54 % of its expansions
after stalling).

It would also replace a class of code rather than add to it: the special
cases in `open_repair/` are each one shape of the same question.

### Expected impact, confidence, cost

- If a third of the roughly 15 near-miss boards close: +5 strict passes
  (71 to 76); the ceiling is about +15. My confidence: 35 % for at least
  +5, 60 % for at least +2.
- The prior is lowered by two measured negatives (ejection chains closed
  nothing on 12 near misses; the window renegotiation ended 20 to 20). They
  do not say whether a solution exists in the window, only that best
  response does not find one. The census below separates the two.
- Even a negative census pays: it classifies the residue into "search
  failure", "lattice model" and "needs a change outside the window", which
  today is unknown and is what the next router bet depends on.
- Cost: the census 3 to 5 days. The engine in the router (an in-tree CDCL
  with cardinality constraints, about 1500 lines; pcb-router depends on
  rayon only) and its integration 2 to 4 weeks.

### Determinism, KiCad as judge, work units

A single-threaded CDCL is deterministic. Work is its propagation count at a
calibrated rate, like `SEVER_NODE_NANOS`. Windows are solved in a fixed
order; disjoint windows may run in parallel and merge in order, as the
batches do. KiCad stays the judge; certificates are statements about the
router's model and are worded so.

### The cheapest decisive experiment

A window feasibility census on stored boards, no routing:

1. Take the nightly's near-miss results (22 sweep boards, the designer
   tier's too). Lower each routed board with its copper kept (the path
   `keep_existing_copper` and `open_repair::lower_for_local` already use)
   and dump, per unconnected item, the lattice of a window: static maps and
   which net holds each node (`PCB_ROUTER_DUMP` prints this for a region; a
   research command makes it machine-readable).
2. A Python script writes the CNF and runs CaDiCaL (installed at
   `/usr/bin/cadical`). Window half-sizes 2 and 4 mm, 10 minutes each.
3. Per item: SAT, UNSAT with its core, or timeout. Write each SAT answer
   into the board and check with `drc-kicad-board`.

Go: at least 30 % of the open items of the near-miss boards are SAT, or at
least 5 boards become strict passes under KiCad. Kill: under 15 % SAT at
4 mm and cores that name more than about 8 nets (the cause is not local):
drop the solver, keep the classification, and read B's census for the next
bet.

### References

- Nam, Sakallah, Rutenbar, "A new FPGA detailed routing approach via
  search-based Boolean satisfiability", TCAD 2002 (FPGA 1999): all nets at
  once, and proofs of unroutability.
- A UC Berkeley ERL report (99-16) routes the nets of a cluster at once
  with SAT and treats finished clusters as blockages: the windowed scheme.
  (Found by a web search; I read its summary, not the report.)
- Ryzhenko, Burns, "Standard cell routing via Boolean satisfiability",
  DAC 2012.
- Wu, Davoodi, Linderoth, "GRIP: global routing via integer programming",
  DAC 2009 and TCAD 2011: column generation with shortest paths as pricing.
- Sharon, Stern, Felner, Sturtevant, "Conflict-based search for optimal
  multi-agent pathfinding", Artificial Intelligence 2015; Li, Chen,
  Harabor, Stuckey, Koenig, "MAPF-LNS2", AAAI 2022 (repairing an infeasible
  plan by replanning subsets: what the trouble repair is).
- Shaw 1998 (large-neighbourhood search); Fischetti, Lodi, "Local
  branching", 2003 (exact solves of neighbourhoods).
- Luo, Wong, "Ordered escape routing based on Boolean satisfiability",
  ASP-DAC 2008.

## 3. Proposal B: an exact cut-capacity model shared by placer and router

### The idea

A *cut* is the free width between two fixed features (two pads, a pad and
the edge, a pad and a hole) on a layer. Its capacity in tracks is exact
arithmetic, `floor((gap + s) / (w + s))`, and independent of any lattice.
Classical results (below) say that on one layer a set of routes is
realisable exactly when no cut is over capacity. The tool has this quantity
in three approximate forms (the tile boundaries of `pcb-global`
`capacity.rs`, the facing-side channels of `pcb-placer` `escape.rs`, the
edge cuts of the parked `pcb-topo` `Topology::capacity`) and uses none of
them where the decisions are made. The proposal is to make it the shared
quantity, in four steps that each stand alone.

**Step 0, a census (no product code).** At every stall site of the
near-miss and dense boards, find the binding cut on the path the open net
wants (the trouble-maker's priced route) and compare the wires across it
with its exact capacity, per layer. Each open item is then one of:
capacity is there but wasted by where the tracks lie or by the lattice's
phase; or the cut is full on every layer, and the cut names the two parts
and the distance that would open it.

**Step 1, the placer's room model.** Two changes where the docs say the
objective is blind:

- *Simultaneous escape by max-flow.* `escape.rs` estimates a part's demand
  per side and a channel per pair of facing bodies. Replace the estimate by
  a max-flow per part on the real geometry: sources are the pins whose nets
  leave the part, the sink is a ring at a few millimetres, capacities are
  the cuts between neighbouring pads and bodies on each layer plus via
  sites. The deficit (pins less flow) is exact for the model, noise-free
  and local. `pad_exits` asks whether each pad can leave alone; the flow
  asks whether all can leave together, which is what a QFN crowded by its
  own decoupling capacitors fails.
- *Gaps in track quanta.* The legaliser (`legal.rs`) and the refinement
  seat parts on `grid_mm` 0.635 with `spacing_mm` 0.5, and room comes from
  halos of `ceil(pins / 8)` tracks scaled 1, 0.5, 0 by level
  (`board_placer/lower/footprint.rs`, `halo`). Neither knows that a pad gap
  of 0.55 mm holds no track at 0.2 / 0.2 and 0.60 mm holds one. Where the
  measured demand (`board_placer/measured.rs`) or the flow says k tracks
  must pass between two parts, round their pad gap up to
  `k * w + (k + 1) * s`; where it says none, let them close up.

**Step 2, the layout's moves.** `try_moves` (board_layout/moves.rs) tries
single parts at up to three poses, each a reroute: 494 trials on 98 boards
in the sweep of 2026-11-18, 158 kept (the two trials I read took 34 and
70 s). With cuts, the
move is computed: an LP over small displacements of the parts that bound
the overloaded cuts (from step 0 or from A's certificates), each such cut
at least its required width, bodies apart in their current order,
constraints kept, least total displacement. One solve moves several parts
at once; `Router::update` and `reroute` then route only what moved. This is
the concrete form of "push-and-shove with placement participating".

**Step 3, later and only if 0 to 2 pay.** The cut graph as the two-level
corridor graph Florian parked, with exact capacities instead of tiles:
negotiation prices on cuts, lanes assigned in a cut, the lattice search
confined to its lane. And for over-full boards, relax-then-compact: route
on an enlarged outline where it completes, then shrink with the routes'
topology kept, the cut constraints deciding how far.

### Why it could change the basin

- The placer's room is continuous and isotropic; the router's is discrete
  and specific. The evidence that the mismatch costs connections: sub-grid
  reseating of the designer's poses costs about 30 open terminals on each
  of the two densest boards; "the anneal lowers our objective while the
  routes get worse" there, with the decoupling relations 88 % of the
  objective; the passives' level per board has "no rule" because "the
  objective is blind to it".
- Area congestion has been fed back three times without effect. That is
  what one expects if the binding constraint is a handful of discrete cuts
  at pads and not the fill of tiles.
- Exact-fit geometry is the router's second failure class and has needed
  surgery on the lattice each time (pin lines, corridor stubs on a 5 um
  raster, F23's 4 um miss). A cut does not have a phase.
- The earlier negatives are different mechanisms: capacitated corridors and
  the global plan worked on 16 x 16 node tiles, whose capacities are sums
  over a boundary and carry no order; halos rescaled by RUDY used an area
  estimate. None used the exact cut at the pad.

### Expected impact, confidence, cost

Aimed at the open count of the dense boards (Glasgow and the two OpenESC
boards hold 252 of the 605 open connections of the target boards) and at
the boards placement costs one to three connections. Confidence: 30 % that
step 1 passes stage 1 at -5 % or better; 50 % that step 2 beats the trial
moves per unit of work. Step 0 is worth doing whatever follows. Cost: step
0 three to four days, step 1 about two weeks, step 2 about two weeks, step
3 months.

### Determinism, KiCad as judge, work units

Max-flow with a fixed augmentation order and an in-tree simplex are
deterministic; work is augmentations and pivots. The placer's legality and
KiCad's check stay as they are.

### The cheapest decisive experiment

A gap census on existing placements (stage 0, Python on the placed boards
the nightly keeps):

1. For the designer's and our placements of the eleven dense boards: the
   distribution of pad-to-pad gaps between neighbouring parts relative to
   the track quanta of the board's rules, and the share of gaps within
   0.1 mm *below* a quantum (room paid for and unusable).
2. Per part of ten pins or more, the escape max-flow deficit on both
   placements.

Prediction if the thesis holds: the designer's gaps cluster just above the
quanta and ours do not; our deficits on the 4in1, Glasgow and DSP-ADAU1452
are several times the designer's. Kill: no separation on either measure.
If they separate, the first intervention is a snapping pass after the
legaliser, screened at stage 1 on two seed pairs before the full five.

### References

- Cole, Siegel, "River routing every which way, but loose", FOCS 1984;
  Leiserson, Maley, "Algorithms for routing and testing routability of
  planar VLSI layouts", STOC 1985; Maley, *Single-Layer Wire Routing and
  Compaction*, MIT Press 1990 (the cut condition, and compaction with
  wires attached).
- Dai, Kong, Sato, "Routability of a rubber-band sketch", DAC 1991; UCSC
  report CRL-97-07, "Fast and incremental routability check of a
  topological routing using a cut-based encoding".
- Yan, Wong, "Correctly modeling the diagonal capacity in escape routing",
  TCAD 2012 (DAC 2009): escape as an exact network flow.
- Cheng, Ho, Holtz, "Net separation-oriented printed circuit board
  placement via margin maximization" (NS-Place), ASP-DAC 2022: a
  routability-aware MILP legalisation for PCBs.
- Zhang et al., "Cypress: VLSI-inspired PCB placement with GPU
  acceleration", ISPD 2025: an ePlace-style PCB placer with a net-crossing
  term, the nearest published relative of the current pipeline; its gains
  are from the objective, which this project already has.

## 4. Proposal C: objective-guided arrangement search on the skeleton

### The idea

`pcb_placer::skeleton::place_skeleton` places the active parts alone, then
the passives around them. The skeleton is placed by the ordinary pipeline:
`initial::quadratic_start`, the electrostatic descent
(`global/descent.rs`), an anneal that starts cold on purpose (temperature
0.3, times 0.975 a stage, `anneal/run.rs`), the legaliser. Diversity comes
from seeds: `race_seeds` runs up to sixteen independent copies.

Replace the independent copies by one search over arrangements:

- *Replica exchange.* The K seeds become K replicas of the skeleton's
  anneal on a ladder of temperatures. Replicas trade states at fixed work
  checkpoints, so a hot replica explores arrangements and a cold one
  refines the best found. The moves are those of `anneal/moves.rs` plus
  arrangement moves: trade any two parts, turn or mirror a cluster with its
  neighbours, reverse a bank. The objective is today's (weighted wire,
  ratsnest crossings, escape).
- *What goes to the race.* The best few arrangements that differ (by the
  `rigid_fit` displacement `stages.py` already computes), not sixteen draws
  of one distribution.
- *A stricter variant* if the anneal's overlap penalty gets in the way: a
  floorplan representation (a sequence pair fixes for each pair of parts
  "left of" or "below"), and for a fixed sequence pair the positions that
  minimise the half-perimeter wirelength are a linear program. Discrete
  search outside, exact convex solve inside, every state legal.

### Why it could change the basin

The diagnosis in docs/placer.md is that the objective accepts the
designer's arrangement (0.59 to 1.02 of our own placement's objective,
lower where they differ) and the search does not reach it; what paid were
changes of start, not of cost terms; more seeds did not help. A descent
with a density penalty cannot carry one part across another, and
independent restarts sample local optima without using what the others
found. Replica exchange and floorplan moves do both. The work is already
there: the skeleton takes a median 22 s of 600.

### Expected impact, confidence, cost

Uncertain, about 25 % for a stage-1 win of 5 %. The reason is in section 1:
the coarse-arrangement gap that this attacks may be mostly closed (the
glued banks took video from 92 to 0 unconnected by the median), and what is
left of the placement
gap sits on over-full and panel boards, which B addresses. The gate below
is cheap enough to find out. Cost after the gate: one to two weeks for
replica exchange on the existing anneal, more for the sequence-pair variant.

### Determinism, KiCad as judge, work units

Each replica has its own random stream and counts its work; exchanges
happen when every replica has reached the same work checkpoint and are
decided in replica order, so the result does not depend on threads. The
race and the routed judge stay.

### The cheapest decisive experiment

An objective-gap census on today's main, with switches that exist
(`PCB_PLACER_TRACE`, `PCB_PLACER_DESIGNER`): per board with a placement
gap, the skeleton objective of the designer's arrangement, legalised by our
legaliser, against the sixteen seeds' skeletons.

- If the designer's lies below our best seed by more than the seeds'
  spread: the optimiser leaves objective on the table. Build a rough hot
  schedule for the skeleton on a branch and run stage 1 on the boards with
  a gap.
- Kill 1: the designer's objective lies inside our seeds' spread on the gap
  boards. Then the objective cannot tell them apart any more and a stronger
  search has nothing to follow.
- Kill 2: the search reaches the designer's objective and the first route
  does not improve. Then "accepts" was not "prefers for the right reason",
  and the work belongs in the objective (B).

### References

- Swendsen, Wang 1986 and Hukushima, Nemoto 1996 (replica exchange);
  Lourenco, Martin, Stutzle (iterated local search).
- Murata, Fujiyoshi, Nakatake, Kajitani, "VLSI module placement based on
  rectangle-packing by the sequence-pair", TCAD 1996; Adya, Markov,
  "Fixed-outline floorplanning", TVLSI 2003; Chang et al., B*-trees, DAC
  2000.

## 5. Evaluation: cheaper and more trustworthy

Iteration speed limits everything else, so these are ranked on their own.

**E1. An endgame tier on stored stalled boards.** The regime that decides
the headline is the last connections of about 22 boards. Every experiment
on it today costs whole layouts. The final boards are on disk and the
router can take a routed board with its copper kept
(`keep_existing_copper`, `repair-kicad-opens`). A tier that starts from the
nightly's stalled results and runs only the endgame costs minutes and is
exact for the question "does this repair close more". It is the first thing
to build, with or without proposal A.

**E2. Coupled A/B runs.** The anneal draws from one sequential stream
(`struct Random(u64)`, a xorshift, `anneal.rs` line 64). One more or one
fewer draw desynchronises everything after it, so "a placer change moves
every board to another draw of its seed noise" and pairing by seed removes
no variance. With counter-based random numbers, a draw is a hash of (seed,
part, stage, sweep, draw index): two runs that differ in one place make the
same proposals everywhere else, and at the anneal's low temperature the
difference tends to stay local. A lever that acts locally then gives a
local difference and the paired variance falls. First test, a placebo: move
one passive's start by 0.1 mm and count the parts whose final pose differs,
before and after the change. Honest limits: a lever that changes the global
placement decorrelates anyway, and the router's own chaos remains.

**E3. Where the noise comes from.** Three dense boards, three placer seeds,
three router seeds each (`PCB_ROUTER_SEED`), first-route open terminals.
If the router's share of the variance is the larger one, the race is
picking among placements by noise, and replicating placer seeds is the
wrong replication. This costs about 27 first routes. The docs hold one
number that points this way (two probes agree at 0.44) and no
decomposition.

**E4. Certificates as metrics.** The escape deficit and the overloaded cuts
of B, and the counts of infeasible windows of A, are exact statements of
the model, have no seed noise and cost milliseconds. They are necessary
conditions, not the routed outcome; their use is as stage-0 measures that
are trusted once B's census shows they separate the designer's placements
from ours.

**E5. A sequential, board-adaptive protocol.** The rule is five pairs, then
nine. Boards differ tenfold in cost and noise; a design that gives the next
seed to the board with the most information per CPU hour and stops early in
both directions (always-valid p-values) would cut the average cost by a
third to a half at the same error rates. Statistics, not an algorithm, but
it compounds with everything.

**E6. A topological routability oracle, as a test only.** `pcb-topo` can
route every connection in planar mode in a second or so (`Topology::route`,
then `layers::assign`, `crossing_pairs`, `overflowing`). As a router it is
parked; as a judge of placements it has never been measured. The bar is
known: within-board Spearman against the first route of 0.30 for RUDY, 0.33
for the global route, 0.55 for a 75 s probe, 0.44 between two probes. At
0.45 or better it would rank hundreds of arrangements for C; below RUDY it
is dead. One day on the placements of `dense-noise.json`.

Not worth it: a cheap proxy as a control variate. With a correlation of 0.7
at 0.55 of the cost (the first route) the optimal multi-fidelity estimator
has a variance ratio of (sqrt(1 - 0.49) + sqrt(0.55 * 0.49))^2 = 1.5, worse
than the full stage alone; it needs a correlation near 0.9 at a tenth of
the cost.

## 6. Secondary ideas, briefly

- **Pour mode by region, not by rung.** The census's largest signal (our
  copper of a net the designer pours crosses the designer's path on 114 of
  199 open nets; the kept attempt routes pours as tracks on most near-miss
  boards) has no live design. A is the local answer. The global one is to
  decide per pad or region whether the fill or a track carries the supply,
  and to take out supply tracks the fill provably makes redundant before
  the open nets retry.
- **Prices from duality.** PathFinder's history is additive and local to
  where a conflict happened. Resource-sharing algorithms raise the price of
  every resource in proportion to its use and converge provably on the
  fractional problem. On lattice cells the fractional problem means little
  (section 7 shows the gap); on cuts with capacities above one it does.
  Only after B's step 3.
- **A non-uniform lattice from cuts.** `Grid::warp` already holds per-axis
  coordinate tables. Lines at the lanes of every binding cut would
  generalise the pin lines and remove the phase problem where it matters.
- **Repeated channels solved once.** The transfers tried copy an
  arrangement between instances and were mixed. What a designer copies is
  the routed channel. A module placed and routed as one small problem, then
  stamped with its copper rippable, is untested.
- **A cooperative portfolio.** Seed rounds restart from nothing. Seeds that
  share what they proved (certificates of A, cut prices) at work
  checkpoints stay deterministic and use the idle cores of a single run.
- **Local priors from the designers.** Not placements, but how a two-pin
  passive sits against the pin it serves. Small, CPU-only, and it must be
  cross-validated by repository, because the corpus is also the benchmark.

## 7. A calibration of the exact methods (toy runs)

To check that proposal A is not wishful, two scripts on synthetic grids
with a planted solution (two layers, random blocks, two-terminal nets, unit
clearance; `scratchpad/sat/win.py`, `win2.py`, `cg.py`). They are not
boards; they bound what a naive formulation does.

| Formulation | Instance | Result |
| --- | --- | --- |
| Cell-level CNF, CaDiCaL | 16 x 16 x 2, 5 nets | solved, 0.1 s |
| | 24 x 24 x 2, 7 nets | solved, 0.4 s |
| | 30 x 30 x 2, 9 nets | no answer in 40 s |
| | 40 x 40 x 2, 12 nets | no answer in 120 s |
| With a detour budget of 10 cells a net | 30 x 30 x 2, 9 nets | solved, 0.1 s |
| | 40 x 40 x 2, 12 nets | solved, 12.7 s |
| | 60 x 60 x 2, 25 nets | no answer in 40 s |
| Column generation, then an ILP over the pool | 30 x 30 x 2, 9 nets | LP bound 9; ILP routes 7 |
| | 40 x 40 x 2, 12 nets | LP bound 12; ILP routes 10 |
| | 60 x 60 x 2, 25 nets | LP bound 25; ILP routes 19 |

What this means:

- Exact cell-level solving has a sharp wall. It fits a hot spot (a window
  of about 4 mm at 0.1 mm pitch, a dozen nets), not an area. That happens
  to be the size of the residue, with little headroom.
- Modelling decides by orders of magnitude (the detour budget alone moved
  one instance from a timeout to 0.1 s). The incumbent routes as phase
  hints and cut cardinalities are the next such steps and were not tested.
- Pricing converges fast and the fractional bound is useless by itself at
  cell level: an ILP over the generated routes stays 20 to 30 % short of a
  solution that exists. A path-based method needs real branching, or the
  incumbent as its starting pool.

So A is feasible at the scale it is needed and not a free lunch; the census
must run on real residues before anything is built.

## 8. Considered and rejected

- **A full topological router again.** `pcb-topo` was written on one day
  (its engine commits are all of 2026-09-29, the verdict of 2026-09-30) and
  lacks Steiner joins, pours and free via positions, so its loss (video 145
  of 371) does not refute the principle. A real one is months of work
  against a lattice router with two months of fixes from real boards. What transfers is the cut
  model as a census, a judge and a certificate (B, E6), not the engine.
- **Anything on the GPU.** Measured; the GTX 1650 and the search's memory
  latency settle it.
- **End-to-end learning** (RL or diffusion placement, learned routers). The
  corpus is 109 boards and is also the benchmark; licences cut it further;
  floating-point inference and determinism across machines conflict; the
  published PCB results are judged by another router's completion rate, not
  by a clean DRC.
- **Tile-level congestion feedback in new clothes** (a differentiable
  global router, cell inflation from RUDY or from a global route). Three
  measured negatives here, and the mechanism in B explains them.
- **Whole-board exact place and route** (ILP, SAT, SMT). Section 7's wall
  is at tens of cells; the SMT place-and-route literature is about single
  standard cells.
- **Per-board knob search** (Bayesian optimisation, CMA-ES). Each sample is
  a route and two probes agree at 0.44.
- **Tree search over net orders or rip-up choices.** Each rollout is a
  negotiation.
- **Faster single searches** (jump points, bidirectional or incremental
  A*). The costs are not uniform, the profile rounds have taken the
  constant factors, and at twice the work 17 of 27 failing boards end
  unchanged to the item: speed is not what the router lacks.
- **More seeds, more budget, kept copper as a warm start, condensed
  boards.** All measured.
- **Incremental reroute as the judge of a placement move on dense boards.**
  "A reroute in that congestion does not settle"; B's step 2 uses it only
  after a computed move, not as a trial.

## 9. An order of work

Week 1, in parallel, none of it product code: E1 (the endgame tier), A's
window census, B's gap census, E3 (the noise's source), C's objective-gap
census. About five agent packages and well under 100 CPU hours.

Then one bet at a time, chosen by the censuses: A if windows are solvable;
B's step 1 if gaps and deficits separate the designer's placements from
ours; C only if its gate opens. E2 before the next placer protocol runs,
since it makes every later protocol cheaper.

## 10. Uncertainties worth stating

- The split of near-miss boards into "ours" and "the input's" uses the
  ceiling census of 2026-11-06 and its notes; the list moves nightly.
- The designer-against-ours objective ratios (0.59 to 1.02) predate the
  netlist start and the skeleton race. C's gate re-measures them.
- I did not run the router, so the sizes of real windows (nodes, nets,
  stamps) are inferred from `board-router.json`, not measured.
- The 0.44 agreement between two probes is quoted from docs/router.md; E3
  would measure the same thing directly.

## 11. Web sources consulted

Checked by web search on 2026-11-23; the other references are cited from
memory and worth a second look before anyone builds on a detail.

- [Cypress, ISPD 2025 (paper)](https://www.csl.cornell.edu/~zhiruz/pdfs/cypress-ispd2025.pdf),
  [NVlabs/Cypress](https://github.com/NVlabs/Cypress)
- [NS-Place, arXiv 2210.14259](https://arxiv.org/abs/2210.14259)
- [GRIP, DAC 2009](https://jlinderoth.github.io/papers/Wu-Davoodi-Linderoth-09.pdf)
- [Nam, Sakallah, Rutenbar: SAT-based detailed FPGA routing](https://research.ibm.com/publications/a-new-fpga-detailed-routing-approach-via-search-based-boolean-satisfiability)
- [UC Berkeley ERL 99-16](https://www2.eecs.berkeley.edu/Pubs/TechRpts/1999/ERL-99-16.pdf)
- [MAPF-LNS2, AAAI 2022](https://ojs.aaai.org/index.php/AAAI/article/view/21266)
- [Dai, Kong, Sato, DAC 1991 (record)](https://bibtex.github.io/DAC-1991-DaiKS.html),
  [UCSC-CRL-97-07](https://tr.soe.ucsc.edu/research/technical-reports/UCSC-CRL-97-07)
- [Yan, Wong: diagonal capacity in escape routing](https://scholars.hkbu.edu.hk/en/publications/correctly-modeling-the-diagonal-capacity-in-escape-routing/)
- [SP&R, SMT-based simultaneous place-and-route](https://cseweb.ucsd.edu/classes/fa23/cse248-a/papers/cells/SMT-Based.pdf)
