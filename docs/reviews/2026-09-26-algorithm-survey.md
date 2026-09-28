# Algorithm survey: what else could help

2026-09-26. A literature and ideas survey, not an experiment. Nothing in the
router or placer changed. The question was which algorithms, beyond the ones
already in `pcb-router` and `pcb-placer`, could matter, including ones from
unrelated fields.

## How it was done

Five fresh agents, none of them with this project's history, received a
neutral description of the problem and the corpus pain points (both-layer
pours stalling at 85–90 %, BGA escape with 0.01 mm slack, dense two-sided
boards without a legal placement, wirelength ≠ routability, 10–20 minute
four-layer runs, no diff pairs, length matching or pin swap, no way to tell
"infeasible" from "gave up"). Each got a different lens:

| Report | Lens | Knew our current approach? |
| --- | --- | --- |
| [1-blind](2026-09-26-algorithm-survey/1-blind.md) | any field; problem statement only | no |
| [2-theory-or](2026-09-26-algorithm-survey/2-theory-or.md) | LP/IP, flows, certificates, approximation, CP/SAT | yes |
| [3-cross-domain](2026-09-26-algorithm-survey/3-cross-domain.md) | structurally similar problems in other fields | yes, and what was already tried |
| [4-geometry-topology](2026-09-26-algorithm-survey/4-geometry-topology.md) | computational geometry and topology | yes |
| [5-search-stochastic-learning](2026-09-26-algorithm-survey/5-search-stochastic-learning.md) | randomized search, inference, learning | yes |

They produced about 130 ideas. The raw reports are kept in full; this page
de-duplicates them, checks them against the code, and orders them. References
marked as verified in the reports were checked by web search; the reports
flag the ones that were not. One new external resource was spot-checked here:
[PCBWorld](https://github.com/LGAI-Research/PCBWorld) (LG AI Research, KDD
2026) exists and ships a benchmark set of open-source KiCad boards ("D3") plus
FreeRouting/OrthoRoute baselines. The board count (679) comes from a search
summary and was not confirmed.

## What is already covered (filtered out)

Several suggestions describe what the code already does, which is at least a
sign the current design is not idiosyncratic:

- PathFinder negotiation, coarse tile corridors, rip-up/reinsert
  ([router.md](../router.md)); via reduction by renegotiation.
- "Route the pour net first as a backbone" is the plane skeleton
  (`route_skeletons`, `router.rs`), which is the second rung of the ladder.
- ePlace-style electrostatic placement, SA legalization, router-driven nudges.
- Disjoint-batch parallelism exists. Jacobi batches (all nets against one
  snapshot) were tried and converged worse (Interf-U 448 s / 44 vias vs 81 s /
  28 vias). Two reports recommend Jacobi; our own measurement says no. The
  *deterministic reservation* variant (below) is different and still open.
- Pressure fields and PBD rubber bands were prototyped in
  [2026-09-07](2026-09-07-algorithm-exploration.md).

Facts checked in the code that several ideas depend on: the router has no
randomness at all (`routing_order` is a fixed sort by pour/pin-count/span,
ties by net id); the A* heuristic is octile (a coarse octile distance
transform for many targets); multi-pin trees are grown Takahashi–Matsuyama
style (successive A* to the partial tree); the placer models bodies as
rectangles/discs, never changes a part's side, and the annealer uses a fixed
seed.

## Where the agents converged

Ideas that three or more of the five reports arrived at independently. The
blind report reached most of them without knowing anything about our code.

| Idea | Reports | Pain point |
| --- | --- | --- |
| Exact, incremental pour-island accounting (planar duality / Euler characteristic / union-find) as a routing cost | 1, 2, 3, 4 | pours |
| Fractional multicommodity flow on the tile graph (multiplicative weights, BonnRoute-style resource sharing): λ\*, dual prices, bottleneck cuts | 1, 2, 3, 4, 5 | infeasible vs gave up, which net yields, corridors, placement feedback |
| Geometric cut certificates on a Delaunay triangulation of obstacles ("27 nets must cross this 3.1 mm gap, 22 fit") | 1, 2, 4, 5 | infeasible vs gave up, placement |
| Planar max-cut layer assignment (Hadlock; Chen–Kajitani–Chan; Pinter) for two-layer boards, with a pour term | all five | vias, pours |
| Large-neighbourhood search endgame (MAPF-LNS2, ALNS, regret insertion) around rip-up | 1, 3, 5 | the 85–90 % stall |
| Local exact window solving (CBS with corridor reasoning, CP-SAT/MaxSAT with UNSAT cores) | 1, 2, 3, 5 | stall, which net yields, local proofs |
| Irregular nesting for legalization (sparrow / jagua-rs, Rust) or CP-SAT NoOverlap2D | 1, 2, 3, 4, 5 | no legal placement |
| BGA escape as a flow on exact channel capacities (Yan–Wong) | 1, 2, 4, 5 | BGA escape |
| ALT landmark heuristic (admissible for every negotiation iteration because prices never drop below base cost) | 1, 2, 3, 5 | four-layer time |
| Measure routability proxies (RUDY, ratsnest crossings, cut overflow, λ\*) against routed outcome before using any | all five | placement |
| Pin/gate swap as assignment, or VPR's shared super-sink | all five | missing feature |

## The ideas, by pain point, mapped onto the code

Value/effort are our estimates after reading the code, not the reports'.

### 1. Pours shredded by signals (ngdevkit, starved thermals)

- **Island-aware step cost (reports 1, 2, 3, 4).** Per layer, the pour is
  the complement of foreign copper inflated by zone clearance plus half the
  minimum width. A new track segment creates an island exactly when it touches
  two obstacles that are already in the same connected cluster (it closes a
  loop around a face). With union-find over obstacle clusters this is O(α)
  per contact. The penalty applies only when the enclosed face has no
  pour-net pad and no room for a stitching via. Today `plane_cut_cost` prices
  every cut the same; this separates harmless cuts from fatal ones. Report 3
  frames it through Hex duality, report 2 through tree–cotree duality: the
  skeleton reserves a primal spanning tree, the island cost forbids dual
  cycles, but softly. First step, no router change: log which nets close
  islands on ngdevkit and dut-s2. **Value high, effort low–medium.**
- **Pour healing as a Steiner / prize-collecting Steiner problem (1, 2).**
  Nodes are islands per layer; edges are via sites where islands overlap and
  short bridges; terminals are islands with pour-net pads. Hundreds of nodes,
  so it can be solved exactly. When no tree exists, the minimum cut names the
  signal segments that separate the islands. Raise their price and reroute (a
  Benders loop between signals and pour). This would replace the greedy
  per-island via choice in `stitch_pours`. **High, low–medium.**
- **Stitch sites and a pour spine from the medial axis (4).** The medial axis
  of free space with inscribed radius ρ gives a "via fits here on both
  layers" test and a widest-neck spanning tree for the skeleton. That is a
  geometric version of `route_skeletons`, which today routes a lattice tree.
  **Medium, medium.**
- **Loopy skeleton (3, from leaf venation, Katifori et al. 2010).** Networks
  that must survive damage grow loops. A skeleton with deliberate loops
  around dense signal regions tolerates cuts that would island a tree
  skeleton. It is cheap to try as a variant of the skeleton rung.
  **Medium, low.**
- **Max-cut layer assignment with a pour term (all five).** Fix the 2-D
  projection of a finished two-layer board, then relabel segments top/bottom:
  crossings must differ, a label change along a net costs a via, and a unary
  term keeps each pour's critical necks clear. Instances have a few thousand
  segments; an ILP or QPBO solves them. Offline first, on finished boards:
  count the change in vias and islands. It is a go/no-go test for a 2.5-D
  router (route with crossing costs, assign layers afterwards).
  **Medium–high, medium.**

### 2. The 85–90 % stall and "which net yields"

`negotiate` stops after 25 non-improving iterations (`router.rs`), then
`resolve_remaining` reinserts the most contested nets against hard obstacles.
That is a depth-1 repair. Candidates, cheapest first:

- **Cycle detection plus hand-off (2).** Hash the (net → route) state; a
  repeat means PathFinder is cycling (history plus a growing present factor
  breaks Rosenthal's potential, so convergence is not guaranteed). Stop
  negotiating then, instead of after 25 iterations. **Medium, low.**
- **Opportunity-cost arbitration (1, 2, 3).** At a persistent bottleneck,
  compute each occupant's regret: its best route with the passage masked
  minus its current route (one A* each). Give the slots to the nets with the
  largest regret and evict the one with the smallest regret per conflict
  (Chaitin's spill metric, from register allocation). History cost records who
  lost often, not who loses least by moving. **Medium–high, low.**
- **LNS endgame (1, 3, 5).** Destroy a neighbourhood (conflict component,
  spatial window, pour-island region, random), reinsert with regret-2
  ordering, accept by lexicographic score (open, then vias, then length),
  and pick operators by a bandit. Disjoint windows can run in parallel. This
  is MAPF-LNS2 with nets as agents. **High, medium.**
- **Exact window solving (1, 2, 3, 5).** For windows with 5–25 nets: CBS with
  corridor reasoning (branch once on "A takes the channel, B detours"), or
  CP-SAT/MaxSAT with the outside frozen. It yields either a routing or a
  minimal conflicting net set; MaxSAT weighted by detour cost says who
  yields. UNSAT is only relative to the frozen boundary and the lattice. Keep
  it for the hard kernels after LNS. **High, medium–high.**
- **Ejection chains (2).** A net takes another net's resources, which takes a
  third's, up to depth 3, accepting the best prefix (Lin–Kernighan style).
  The natural generalization of the depth-1 reinsert. **Medium, low–medium.**

### 3. Infeasible or gave up?

Two levels must be kept apart (report 2): the lattice is a *restriction*, so
lattice failure proves nothing about the board. A proof needs a *relaxation*
whose capacities are upper bounds.

- **Lagrangian bound from our own prices (2).** With y = history costs,
  Φ(y) = Σ_nets min-tree-cost(y) − Σ_nodes y·capacity. Φ > 0 proves the
  lattice problem infeasible. The per-net minima are almost free, but only
  from an unrestricted A* (not the corridor search) and a valid Steiner lower
  bound for multi-pin nets. It is weak (half-half fractional splits pass),
  but costs nothing to log. **Medium, low.**
- **Fractional multicommodity flow on the tile graph (all five).**
  Multiplicative weights (Garg–Könemann; Müller–Radke–Vygen, BonnRoute) with
  per-net Steiner oracles over the existing tile graph. It gives λ\* (λ\* > 1 on
  conservative capacities proves infeasibility at the tile level), named
  saturated cuts, dual prices to seed history, and a *fractional* corridor
  (several tile paths per net) in place of the single one from
  `plan_corridor`. Oracle calls within a phase use stale prices, which the
  theory tolerates, so they parallelize. Tile capacity estimation near
  fine-pitch parts is the weak point. **High, medium.**
- **Geometric cut certificates (1, 2, 4, 5).** Delaunay-triangulate pad and
  courtyard corners. Each edge's track capacity is exact in millimetres,
  independent of the lattice pitch. A cut is a boundary-to-boundary chain;
  demand is the number of nets with pins on both sides. The minimum cut per
  region is a shortest path in the dual. The output is readable: "27 nets
  must cross between U4 and J2; 22 fit on two layers". The cut condition is
  necessary, and on a single layer with fixed topology also sufficient
  (Leiserson–Maley). The same number is a placement objective (below).
  **High, medium.**

### 4. BGA escape and lattice resolution

- **Escape as flow on exact channel capacities (1, 2, 4, 5).** Orthogonal
  channel capacity ⌊(p − d − c)/(w + c)⌋, diagonal from p√2 − d, with the
  Yan–Wong correction for shared diagonal capacity. Max-flow says how many
  balls can escape per layer, exactly, and its min-cut says why not. Kong–Yan–
  Wong add pin assignment; Bayless–Hoos–Hu do multi-layer escape with SAT
  modulo graphs. Positions inside each channel then come from clearance sums
  or a small LP, so the 0.01 mm slack becomes arithmetic instead of a lattice
  phase problem. **High, medium.**
- **Topology on the lattice, coordinates by LP (2, 4).** Keep a coarser
  lattice for topology and legalize the geometry with a 1-D spacing LP per
  channel. When the LP is infeasible, the negative cycle is the explanation
  ("pad–track–track–pad exceeds the gap by 7 µm"). **Medium–high, medium.**
- **Fanout templates (1, 3).** Dogbone quadrants for BGAs; offline-enumerated
  exact tiles for periodic ball fields, assigned by constraint propagation
  (report 3 calls it wave function collapse; Karth & Smith 2017). Ordered
  assignment of perimeter nets to gaps per ring is a monotone DP, the same
  shape as Knuth–Plass line breaking. **Medium, low–medium.**
- **Non-uniform grid (1, 4).** Add the escape lines (obstacle edge offset by
  clearance + half width per class, 45° lines through inflated corners)
  instead of refining the whole lattice. It competes with the 0.075/0.05 mm
  ladder rungs. **Medium, medium.**

### 5. Placement: no legal solution on dense two-sided boards

stickhub, openair-max and tiny_tapeout fail here. `legalize` in
`legal.rs` is greedy largest-first; bodies are rectangles.

- **Nesting engine (1, 3, 4).** sparrow (Gardeyn, Vanden Berghe, Wauters
  2025) on jagua-rs (Rust) solves "polygons into a polygon with holes" as a
  sequence of feasibility problems driven by guided local search on
  penetration depth. Per-pair overlap weights grow when a pair keeps
  overlapping, which is PathFinder history applied to placement. Each side is
  a sheet; through-hole parts sit on both. Real courtyard polygons recover
  the L- and T-shaped slack that rectangles waste. First experiment: export
  a failing board as a jagua-rs instance. **High, medium.**
- **CP-SAT NoOverlap2D (1, 2, 5).** Side literal, orientation class, integer
  positions on 0.05 mm, one NoOverlap2D per side, objective = displacement
  from the global placement. The result is a legal placement or, at that
  discretization, a proof that none exists. Cheap necessary conditions
  (per-side area, forced-side subsets) first. **High, low–medium.**
- **Inflate until jammed (3, from granular physics, Lubachevsky–Stillinger).**
  Start with all courtyards at half size, grow them while resolving overlap,
  and record the jamming scale s\*. s\* < 1 is strong evidence of
  infeasibility, and the contact network names the culprits. It also works as
  a homotopy that avoids legalization's local minima. **Medium, low.**
- **Parallel tempering (1, 5).** Replicas of the existing annealer at
  different temperatures with state swaps. It uses idle cores with almost no
  code. **Medium, low.**

### 6. Placement: wirelength is not routability

- **Measure first (all five).** Over the corpus plus perturbed placements,
  rank-correlate HPWL, peak RUDY, ratsnest crossing count, maximum cut
  overflow and λ\* with routed completion and vias. Nothing below should be
  adopted without this table. On two-layer pour boards crossings are a
  plausible proxy: each forced crossing is a layer change that cuts a pour.
- **Cut penalties and Benders cuts (1, 2, 4).** When routing fails, turn the
  saturated cut into a placement constraint anchored to the parts bounding
  it (the gap between U3 and U7 must grow by δ, or they must not share a side)
  and re-place. Cuts accumulate. The router becomes a cut generator rather
  than a score. This is the principled version of step 3 in
  [coupling.md](../coupling.md). **Medium–high, medium.**
- **Placement gradient from routing duals (3, from transportation network
  design).** At equilibrium, the derivative of total congestion cost with
  respect to a pin position is the gradient of that net's price-weighted
  distance field at the pin (envelope theorem). With a softmin
  (recursive-logit) distance on the tile graph, it is smooth and comes out of
  the flow solve above, so ePlace gets a routability force with no router
  call. **High if the flow solve exists, medium.**
- **Seam insertion (3, from seam carving).** A monotone minimum-energy seam
  across the board, found by DP, orthogonal to a saturated cut; shift
  everything on one side by the missing width, absorbing it in slack
  elsewhere. One coherent move that opens a channel where single-part nudges
  do not. **Medium, low.**
- **Multi-fidelity racing and a diverse archive (1, 3, 5).** Generate
  placements from several seeds and parameterizations, screen them with the
  proxies, coarse-route the best half, fully route the best few (successive
  halving). Keep structurally different placements (MAP-Elites niches by
  bottom-side fraction, congestion spread) so that a failing placement is
  replaced by a different one, not nudged. **High, low–medium once the
  proxies are measured.**

### 7. Four-layer run time

- **ALT landmarks (1, 2, 3, 5).** Distances from 8–16 landmarks per layer,
  computed once on base costs, stay admissible for every negotiation
  iteration because present and history factors never lower a cost. A tighter
  bound than octile around connectors, slots and BGA fields.
  **Medium, low.**
- **Deterministic reservations (3, from Blelloch et al. 2012).** All pending
  nets route in parallel against a snapshot; each writes its priority to the
  cells it uses; only nets that win all their cells commit, and the rest retry.
  Unlike the Jacobi batches that failed, losers never stamp conflicting
  copper, and the result is reproducible. Fall back to sequential below a few
  pending nets. **Medium–high, low–medium.**
- **Separator decomposition (2).** Fix where long nets cross a separator from
  the global plan, route the halves in parallel, renegotiate near the
  separator. **Medium, medium.**
- **Seeded portfolio (1, 5).** Independent runs with randomized order and
  tie-breaking on idle cores (see the next section). **Medium–high, low.**

### 8. Missing features

- **Pin swap as a shared super-sink (2, VPR).** An equivalence group becomes
  one virtual target; A* reaches any member and negotiation settles the
  assignment. Gate swap and decoupling-capacitor assignment are linear
  assignments (Hungarian), re-solved per outer iteration. Needs swap-group
  metadata from the user or symbols. **Medium–high, low.**
- **Diff pairs as one fat agent (1, 3, 4).** Route the centerline with
  obstacles inflated by (2w + s)/2 + clearance and heading in the state;
  offset to two tracks; uncoupled breakout stubs at pads. Report 4 gives the
  exact skew identity (skew = (s + w) · signed turning for arc corners), so
  skew is a property of the centerline. **Medium, medium.**
- **Length matching as a resource-constrained shortest path (2, 3), with meander
  area as a transportation problem (2).** Infeasibility becomes a Hall
  violation: "these 3 nets need 14 mm more, adjacent free area supports
  9 mm". **Low–medium for the current corpus, medium.**
- **Any-angle and arc output by shortest homotopic paths (4).** Funnel
  algorithm on obstacles inflated by clearance + w/2: tangents plus arcs, exact
  by construction (Hershberger–Snoeyink cover the fixed-orientation case for
  45°). The trust audit already proposed this over PBD. It can start as a
  post-processor on the lattice output. **Medium, medium.**

## Your two hints: Monte Carlo and graph algorithms

The briefs did not mention either hint; only report 5's lens list named MCTS
among other search methods. Both came back anyway:

- **Graph algorithms** dominate every report without prompting: flows and cuts
  (escape, pour healing, certificates), planar duality and union-find (pour
  islands), max-cut (layers), Steiner trees (exact Dijkstra–Steiner for 3–6
  pins, Hougardy–Silvanus–Vygen 2017, instead of the order-dependent
  successive A*), matching (pin swap, identical-part permutation), landmarks
  and contraction hierarchies (speed), maximum-weight independent sets over
  candidate positions (placement, from cartographic label placement),
  metro-line crossing minimization (track order in corridors, from transit
  maps).
- **Monte Carlo.** The blind report did not propose tree search; it
  proposed randomized restarts. Report 5 covers the sampling family: nested
  rollout policy adaptation over the order of the ~30 most contested nets with
  coarse rollouts, sequential Monte Carlo/beam over partial routings (for BGA
  escape order), the cross-entropy method over per-net weights,
  perturb-and-MAP noise on history costs as a sampler, and counterfactual
  rip-up rollouts as training labels. Our own read: rollouts are only
  affordable on the tile graph, and PathFinder is less order-sensitive than
  sequential routing, so MCTS over orders is a medium bet. The cheapest and
  most informative Monte Carlo step is a precondition for all of it: **add
  seeds to the router and measure the outcome distribution.** The router is
  fully deterministic today. If 32 seeds on ngdevkit or stickhub spread
  widely (heavy-tailed completion), a seeded portfolio with Luby restarts
  that keeps history across restarts pays immediately on idle cores. If they
  do not spread, most of the stochastic family can be deprioritized.

## Unusual transfers worth remembering

Lower priority, but distinct enough to keep on file:

- Dead-end elimination from protein side-chain packing: provable pruning of
  candidate routes or poses before an exact search (report 3).
- Pigouvian (marginal-cost) tolls: change the present-cost increment from
  p(x) to p(x) + x·p′(x) to push negotiation from user equilibrium toward
  system optimum; a one-line A/B (report 3).
- Graph cuts with connectivity priors for splitting a plane layer between
  several power nets (report 3).
- Persistent homology over the clearance radius: one barcode that says which
  channels close for which net class, and which pour necks die below the
  minimum width (report 4).
- Inverse optimization (maximum margin planning) of our cost weights from
  human-routed boards, a measurable "looks human" (report 1).
- Metamorphic testing: route each board rotated and mirrored; completion
  that changes with orientation is a bug, not a hard board (report 1).
- LLM-driven program search (FunSearch/EoH style) over small pluggable
  heuristics such as the net-order key; only with a held-out board set
  (report 5).

All five reports rank reinforcement learning lowest: sample-hungry, and in
the chip placement debate tuned classical search matched it.

## Suggested order

Measurement first; each item is a day or two and decides whether a larger
family is worth building.

1. **Router seeds**: add randomized net order, tie-breaking and history
   noise; 32 seeds on ngdevkit, stickhub, coldfire; plot the completion
   distribution.
2. **Island logging**: union-find over obstacle clusters on ngdevkit and
   dut-s2. Do a few nets cause most islands?
3. **Offline max-cut relabel** of finished two-layer boards: change in vias
   and pour islands.
4. **Proxy table**: HPWL, RUDY, crossings, cut overflow against routed
   outcome over the corpus plus perturbed placements.
5. **Legality check**: stickhub / openair-max / tiny_tapeout through CP-SAT
   NoOverlap2D and through sparrow with real courtyards.
6. **Metamorphic run** of the corpus (cheap, may find bugs that look like
   hard boards).

Then, depending on those results: island-aware step cost and Steiner pour
healing; regret arbitration and an LNS endgame in place of
`resolve_remaining`; ALT landmarks; the multiplicative-weights tile router
with λ\* and duals (which also feeds placement); exact-capacity BGA escape.
The structural option, a Delaunay capacity graph as the topological router
with funnel realization and the lattice only inside its sleeves, is where
several reports point in the long run. It extends the 2026-09-07 topology
prototype and should only be started once the certificates above show that
the tile model is what limits us.
